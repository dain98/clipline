//! Upload transport selection for Clipline Cloud.
//!
//! The server-proxy upload path remains the baseline. Direct-to-S3 multipart
//! uploads are used only when discovery and the create-upload response both
//! advertise the required capability.

use std::{collections::BTreeSet, path::Path};

use bytes::Bytes;
use clipline_cloud_api::{
    sha256_hex,
    types::{DirectPartUploadAckRequest, DirectPartUploadUrlResponse},
    CloudApiError, CloudApiResult, CloudClient, CreateUploadRequest, CreateUploadResponse,
    DiscoveryResponse, PartUploadResponse, UploadProgressResponse,
};
use reqwest::{header, StatusCode};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

const DIRECT_PUT_MAX_ATTEMPTS: usize = 3;
const MAX_UPLOAD_PART_BYTES: u64 = 64 * 1024 * 1024;

pub async fn upload_mp4_file_with_progress<F>(
    client: &CloudClient,
    device_token: &str,
    request: &CreateUploadRequest,
    description: Option<&str>,
    path: &Path,
    mut on_progress: F,
) -> CloudApiResult<UploadProgressResponse>
where
    F: FnMut(&UploadProgressResponse),
{
    validate_upload_request_matches_file(request, path).await?;
    let authenticated_control =
        crate::bounded_http::control_client().map_err(CloudApiError::InvalidUpload)?;
    let authenticated_stream =
        crate::bounded_http::authenticated_stream_client().map_err(CloudApiError::InvalidUpload)?;
    let object_http =
        crate::bounded_http::object_stream_client().map_err(CloudApiError::InvalidUpload)?;
    let direct_s3_available = discover_direct_s3(client, authenticated_control)
        .await
        .unwrap_or(false);
    let transport = UploadTransport {
        client,
        authenticated_control,
        authenticated_stream,
        object_http,
        device_token,
    };

    let upload = create_upload(
        client,
        authenticated_control,
        device_token,
        request,
        description,
    )
    .await?;
    match upload_existing(
        transport,
        &upload,
        path,
        direct_s3_available,
        &mut on_progress,
    )
    .await
    {
        Ok(progress) => Ok(progress),
        Err(DirectUploadError::Fallback(_reason)) => {
            let upload = create_upload(
                client,
                authenticated_control,
                device_token,
                request,
                description,
            )
            .await?;
            upload_existing(transport, &upload, path, false, &mut on_progress)
                .await
                .map_err(DirectUploadError::into_cloud_error)
        }
        Err(error) => Err(error.into_cloud_error()),
    }
}

#[cfg(test)]
async fn upload_mp4_bytes_with_progress<F>(
    client: &CloudClient,
    device_token: &str,
    request: &CreateUploadRequest,
    description: Option<&str>,
    bytes: &[u8],
    mut on_progress: F,
) -> CloudApiResult<UploadProgressResponse>
where
    F: FnMut(&UploadProgressResponse),
{
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);
    let suffix = TEST_UPLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "clipline-upload-test-{}-{suffix}.mp4",
        std::process::id()
    ));
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|error| upload_file_error("write test upload", &path, error))?;
    let result = upload_mp4_file_with_progress(
        client,
        device_token,
        request,
        description,
        &path,
        |progress| on_progress(progress),
    )
    .await;
    let _ = tokio::fs::remove_file(path).await;
    result
}

async fn create_upload(
    client: &CloudClient,
    http: &reqwest::Client,
    device_token: &str,
    request: &CreateUploadRequest,
    description: Option<&str>,
) -> CloudApiResult<CreateUploadResponse> {
    let body = create_upload_body(request, description)?;

    let url = client.base_url().join("api/v1/uploads")?;
    let response = http
        .post(url)
        .bearer_auth(device_token)
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        let message = response
            .json::<ErrorResponse>()
            .await
            .map(|body| body.error)
            .unwrap_or_else(|_| status.to_string());
        return Err(CloudApiError::Api { status, message });
    }
    Ok(response.json::<CreateUploadResponse>().await?)
}

fn create_upload_body(
    request: &CreateUploadRequest,
    description: Option<&str>,
) -> CloudApiResult<Value> {
    let mut body = serde_json::to_value(request)
        .map_err(|e| CloudApiError::InvalidUpload(format!("serialize upload request: {e}")))?;
    let Value::Object(ref mut map) = body else {
        return Err(CloudApiError::InvalidUpload(
            "upload request did not serialize to an object".to_string(),
        ));
    };
    map.remove("markers");
    map.remove("description");
    if let Some(description) = normalized_description(description) {
        map.insert(
            "description".to_string(),
            Value::String(description.to_string()),
        );
    }
    Ok(body)
}

fn normalized_description(description: Option<&str>) -> Option<&str> {
    description.map(str::trim).filter(|value| !value.is_empty())
}

async fn upload_existing<F>(
    transport: UploadTransport<'_>,
    upload: &CreateUploadResponse,
    path: &Path,
    direct_s3_available: bool,
    on_progress: &mut F,
) -> Result<UploadProgressResponse, DirectUploadError>
where
    F: FnMut(&UploadProgressResponse),
{
    match upload.mode.as_str() {
        "single_put" => upload_single(
            transport.client,
            transport.authenticated_control,
            transport.authenticated_stream,
            transport.device_token,
            upload,
            path,
            on_progress,
        )
        .await
        .map_err(DirectUploadError::Cloud),
        "chunked" => {
            let progress = get_upload_progress(
                transport.client,
                transport.authenticated_control,
                transport.device_token,
                &upload.upload_id,
            )
            .await
            .map_err(DirectUploadError::Cloud)?;
            on_progress(&progress);

            let Some(presign_template) = upload.direct_part_presign_url_template.as_deref() else {
                return upload_chunked_proxy(
                    transport.client,
                    transport.authenticated_control,
                    transport.device_token,
                    upload,
                    path,
                    progress,
                    on_progress,
                )
                .await
                .map_err(DirectUploadError::Cloud);
            };
            let Some(ack_template) = upload.direct_part_ack_url_template.as_deref() else {
                return upload_chunked_proxy(
                    transport.client,
                    transport.authenticated_control,
                    transport.device_token,
                    upload,
                    path,
                    progress,
                    on_progress,
                )
                .await
                .map_err(DirectUploadError::Cloud);
            };

            if !direct_s3_available {
                return upload_chunked_proxy(
                    transport.client,
                    transport.authenticated_control,
                    transport.device_token,
                    upload,
                    path,
                    progress,
                    on_progress,
                )
                .await
                .map_err(DirectUploadError::Cloud);
            }

            let templates = DirectPartTemplates {
                presign: presign_template,
                ack: ack_template,
            };
            upload_chunked_direct(transport, upload, path, progress, templates, on_progress).await
        }
        other => Err(DirectUploadError::Cloud(CloudApiError::InvalidUpload(
            format!("server returned unsupported upload mode {other:?}"),
        ))),
    }
}

async fn upload_single<F>(
    client: &CloudClient,
    control_http: &reqwest::Client,
    stream_http: &reqwest::Client,
    device_token: &str,
    upload: &CreateUploadResponse,
    path: &Path,
    on_progress: &mut F,
) -> CloudApiResult<UploadProgressResponse>
where
    F: FnMut(&UploadProgressResponse),
{
    let progress =
        get_upload_progress(client, control_http, device_token, &upload.upload_id).await?;
    if progress.status == "completed" {
        on_progress(&progress);
        return Ok(progress);
    }
    let template = upload.single_put_url.as_deref().ok_or_else(|| {
        CloudApiError::InvalidUpload("single_put upload omitted its content URL".to_string())
    })?;
    let url = upload_url(client, template, 0)?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| upload_file_error("open upload", path, error))?;
    let file_size = file
        .metadata()
        .await
        .map_err(|error| upload_file_error("read upload metadata", path, error))?
        .len();
    let response = stream_http
        .put(url)
        .bearer_auth(device_token)
        .header(header::CONTENT_LENGTH, file_size)
        .body(reqwest::Body::wrap_stream(ReaderStream::new(file)))
        .timeout(crate::bounded_http::upload_timeout(file_size))
        .send()
        .await?;
    let progress = parse_json_response(response).await?;
    on_progress(&progress);
    Ok(progress)
}

async fn upload_chunked_proxy<F>(
    client: &CloudClient,
    http: &reqwest::Client,
    device_token: &str,
    upload: &CreateUploadResponse,
    path: &Path,
    progress: UploadProgressResponse,
    on_progress: &mut F,
) -> CloudApiResult<UploadProgressResponse>
where
    F: FnMut(&UploadProgressResponse),
{
    let file_size = tokio::fs::metadata(path)
        .await
        .map_err(|error| upload_file_error("read upload metadata", path, error))?
        .len();
    validate_missing_parts(&progress.missing_parts, file_size, upload.part_size_bytes)?;
    for part_number in progress.missing_parts {
        let chunk =
            read_chunk_for_part(path, file_size, upload.part_size_bytes, part_number).await?;
        put_proxy_part(
            client,
            http,
            device_token,
            &upload.upload_id,
            part_number,
            chunk,
        )
        .await?;
        let progress = get_upload_progress(client, http, device_token, &upload.upload_id).await?;
        on_progress(&progress);
    }
    let progress = complete_upload(client, http, device_token, &upload.upload_id).await?;
    on_progress(&progress);
    Ok(progress)
}

async fn upload_chunked_direct<F>(
    transport: UploadTransport<'_>,
    upload: &CreateUploadResponse,
    path: &Path,
    progress: UploadProgressResponse,
    templates: DirectPartTemplates<'_>,
    on_progress: &mut F,
) -> Result<UploadProgressResponse, DirectUploadError>
where
    F: FnMut(&UploadProgressResponse),
{
    let file_size = tokio::fs::metadata(path)
        .await
        .map_err(|error| upload_file_error("read upload metadata", path, error))
        .map_err(DirectUploadError::Cloud)?
        .len();
    validate_missing_parts(&progress.missing_parts, file_size, upload.part_size_bytes)
        .map_err(DirectUploadError::Cloud)?;
    for part_number in progress.missing_parts {
        let chunk = read_chunk_for_part(path, file_size, upload.part_size_bytes, part_number)
            .await
            .map_err(DirectUploadError::Cloud)?;
        upload_direct_part(transport, upload, part_number, &chunk, templates).await?;
        let progress = get_upload_progress(
            transport.client,
            transport.authenticated_control,
            transport.device_token,
            &upload.upload_id,
        )
        .await
        .map_err(DirectUploadError::Cloud)?;
        on_progress(&progress);
    }
    let progress = complete_upload(
        transport.client,
        transport.authenticated_control,
        transport.device_token,
        &upload.upload_id,
    )
    .await
    .map_err(DirectUploadError::Cloud)?;
    on_progress(&progress);
    Ok(progress)
}

async fn upload_direct_part(
    transport: UploadTransport<'_>,
    upload: &CreateUploadResponse,
    part_number: u16,
    chunk: &Bytes,
    templates: DirectPartTemplates<'_>,
) -> Result<PartUploadResponse, DirectUploadError> {
    let mut last_retryable_error = None;
    for _ in 0..DIRECT_PUT_MAX_ATTEMPTS {
        let presign = request_direct_presign(
            transport.client,
            transport.authenticated_control,
            transport.device_token,
            templates.presign,
            part_number,
        )
        .await?;
        validate_presign(upload, part_number, chunk, &presign)?;

        match put_presigned_part(transport.object_http, &presign, chunk.clone()).await {
            Ok(etag) => {
                let checksum_sha256 = sha256_hex(chunk);
                let ack = DirectPartUploadAckRequest {
                    size_bytes: chunk.len() as u64,
                    checksum_sha256,
                    etag,
                };
                return ack_direct_part(
                    transport.client,
                    transport.authenticated_control,
                    transport.device_token,
                    templates.ack,
                    part_number,
                    &ack,
                )
                .await;
            }
            Err(DirectPutError::Retryable(message)) => {
                last_retryable_error = Some(message);
            }
            Err(DirectPutError::Fallback(message)) => {
                return Err(DirectUploadError::Fallback(message));
            }
            Err(DirectPutError::Terminal(error)) => {
                return Err(DirectUploadError::Cloud(error));
            }
        }
    }

    Err(DirectUploadError::Cloud(CloudApiError::InvalidUpload(
        format!(
            "direct S3 PUT for part {part_number} failed after refreshing presign: {}",
            last_retryable_error.unwrap_or_else(|| "unknown error".to_string())
        ),
    )))
}

async fn request_direct_presign(
    client: &CloudClient,
    http: &reqwest::Client,
    device_token: &str,
    template: &str,
    part_number: u16,
) -> Result<DirectPartUploadUrlResponse, DirectUploadError> {
    let url = upload_url(client, template, part_number).map_err(DirectUploadError::Cloud)?;
    post_empty_with_auth(http, url, device_token)
        .await
        .map_err(classify_direct_control_error)
}

async fn ack_direct_part(
    client: &CloudClient,
    http: &reqwest::Client,
    device_token: &str,
    template: &str,
    part_number: u16,
    ack: &DirectPartUploadAckRequest,
) -> Result<PartUploadResponse, DirectUploadError> {
    let url = upload_url(client, template, part_number).map_err(DirectUploadError::Cloud)?;
    post_json_with_auth(http, url, device_token, ack)
        .await
        .map_err(classify_direct_control_error)
}

fn validate_presign(
    upload: &CreateUploadResponse,
    part_number: u16,
    chunk: &[u8],
    presign: &DirectPartUploadUrlResponse,
) -> Result<(), DirectUploadError> {
    if presign.upload_id != upload.upload_id || presign.part_number != part_number {
        return Err(DirectUploadError::Fallback(
            "direct S3 presign response did not match the requested part".to_string(),
        ));
    }
    if !presign.method.eq_ignore_ascii_case("PUT") {
        return Err(DirectUploadError::Fallback(format!(
            "direct S3 presign returned unsupported method {:?}",
            presign.method
        )));
    }
    if presign.expected_size_bytes != chunk.len() as u64 {
        return Err(DirectUploadError::Fallback(format!(
            "direct S3 presign expected {} bytes for part {part_number}, but the client has {}",
            presign.expected_size_bytes,
            chunk.len()
        )));
    }
    Ok(())
}

async fn put_presigned_part(
    http: &reqwest::Client,
    presign: &DirectPartUploadUrlResponse,
    chunk: Bytes,
) -> Result<String, DirectPutError> {
    let chunk_len = chunk.len() as u64;
    let mut request = http.put(&presign.url).body(chunk);
    for header in &presign.headers {
        let name = header::HeaderName::from_bytes(header.name.as_bytes()).map_err(|e| {
            DirectPutError::Fallback(format!(
                "direct S3 presign returned invalid header name {:?}: {e}",
                header.name
            ))
        })?;
        let value = header::HeaderValue::from_str(&header.value).map_err(|e| {
            DirectPutError::Fallback(format!(
                "direct S3 presign returned invalid header value for {:?}: {e}",
                header.name
            ))
        })?;
        request = request.header(name, value);
    }

    let response = request
        .timeout(crate::bounded_http::upload_timeout(chunk_len))
        .send()
        .await
        .map_err(|e| DirectPutError::Retryable(format!("direct S3 PUT request failed: {e}")))?;
    let status = response.status();
    if !status.is_success() {
        let message = format!("direct S3 PUT failed with {status}");
        if is_retryable_direct_put_status(status) {
            return Err(DirectPutError::Retryable(message));
        }
        return Err(DirectPutError::Fallback(message));
    }

    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            DirectPutError::Terminal(CloudApiError::InvalidUpload(
                "direct S3 upload did not return an ETag for the uploaded part".to_string(),
            ))
        })?;
    Ok(etag)
}

fn upload_url(client: &CloudClient, template: &str, part_number: u16) -> CloudApiResult<String> {
    let path = template.replace("{part_number}", &part_number.to_string());
    let url = reqwest::Url::parse(&path).or_else(|_| client.base_url().join(&path))?;
    if url.origin() != client.base_url().origin() {
        return Err(CloudApiError::InvalidUpload(format!(
            "authenticated upload URL must use the configured cloud origin: {url}"
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CloudApiError::InvalidUpload(
            "authenticated upload URL must not contain user credentials".to_string(),
        ));
    }
    Ok(url.to_string())
}

async fn discover_direct_s3(client: &CloudClient, http: &reqwest::Client) -> CloudApiResult<bool> {
    let url = client.base_url().join(".well-known/clipline-cloud")?;
    let response = http.get(url).send().await?;
    let discovery: DiscoveryResponse = parse_json_response(response).await?;
    clipline_cloud_api::ensure_compatible_discovery(&discovery)?;
    Ok(discovery.features.direct_s3_upload)
}

fn upload_control_url(
    client: &CloudClient,
    upload_id: &str,
    suffix: Option<&str>,
) -> CloudApiResult<reqwest::Url> {
    let mut url = client.base_url().join("api/v1/uploads/")?;
    {
        let mut segments = url.path_segments_mut().map_err(|_| {
            CloudApiError::InvalidUpload("build cloud upload control URL".to_string())
        })?;
        segments.pop_if_empty().push(upload_id);
        if let Some(suffix) = suffix {
            segments.push(suffix);
        }
    }
    Ok(url)
}

async fn get_upload_progress(
    client: &CloudClient,
    http: &reqwest::Client,
    device_token: &str,
    upload_id: &str,
) -> CloudApiResult<UploadProgressResponse> {
    let response = http
        .get(upload_control_url(client, upload_id, None)?)
        .bearer_auth(device_token)
        .send()
        .await?;
    parse_json_response(response).await
}

async fn complete_upload(
    client: &CloudClient,
    http: &reqwest::Client,
    device_token: &str,
    upload_id: &str,
) -> CloudApiResult<UploadProgressResponse> {
    let response = http
        .post(upload_control_url(client, upload_id, Some("complete"))?)
        .bearer_auth(device_token)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    parse_json_response(response).await
}

async fn put_proxy_part(
    client: &CloudClient,
    http: &reqwest::Client,
    device_token: &str,
    upload_id: &str,
    part_number: u16,
    chunk: Bytes,
) -> CloudApiResult<PartUploadResponse> {
    let checksum = sha256_hex(&chunk);
    let size = chunk.len() as u64;
    let mut url = upload_control_url(client, upload_id, Some("parts"))?;
    url.path_segments_mut()
        .map_err(|_| CloudApiError::InvalidUpload("build cloud upload part URL".to_string()))?
        .push(&part_number.to_string());
    let response = http
        .put(url)
        .bearer_auth(device_token)
        .header(header::CONTENT_TYPE, "video/mp4")
        .header("x-clipline-part-sha256", checksum)
        .body(chunk)
        .timeout(crate::bounded_http::upload_timeout(size))
        .send()
        .await?;
    parse_json_response(response).await
}

async fn post_json_with_auth<T, B>(
    http: &reqwest::Client,
    url: String,
    device_token: &str,
    body: &B,
) -> CloudApiResult<T>
where
    T: serde::de::DeserializeOwned,
    B: serde::Serialize + ?Sized,
{
    let response = http
        .post(url)
        .bearer_auth(device_token)
        .json(body)
        .send()
        .await?;
    parse_json_response(response).await
}

async fn post_empty_with_auth<T>(
    http: &reqwest::Client,
    url: String,
    device_token: &str,
) -> CloudApiResult<T>
where
    T: serde::de::DeserializeOwned,
{
    let response = http.post(url).bearer_auth(device_token).send().await?;
    parse_json_response(response).await
}

async fn parse_json_response<T>(response: reqwest::Response) -> CloudApiResult<T>
where
    T: serde::de::DeserializeOwned,
{
    let status = response.status();
    let bytes = crate::bounded_http::response_bytes_limited(
        response,
        if status.is_success() {
            crate::bounded_http::CONTROL_JSON_MAX_BYTES
        } else {
            crate::bounded_http::ERROR_BODY_MAX_BYTES
        },
        "cloud upload control",
    )
    .await
    .map_err(CloudApiError::InvalidUpload)?;
    if !status.is_success() {
        let message = serde_json::from_slice::<ErrorResponse>(&bytes)
            .map(|body| body.error)
            .unwrap_or_else(|_| status.to_string());
        return Err(CloudApiError::Api { status, message });
    }
    serde_json::from_slice::<T>(&bytes).map_err(|error| CloudApiError::Api {
        status,
        message: format!("parse upload response: {error}"),
    })
}

fn classify_direct_control_error(error: CloudApiError) -> DirectUploadError {
    match error {
        CloudApiError::Api { status, message } if status == StatusCode::CONFLICT => {
            DirectUploadError::Cloud(CloudApiError::Api {
                status,
                message: format!(
                    "direct S3 part acknowledgement conflicted with existing metadata: {message}. Retry the upload from the beginning."
                ),
            })
        }
        CloudApiError::Api { status, message } if is_direct_control_fallback_status(status) => {
            DirectUploadError::Fallback(format!(
                "direct S3 control endpoint is unavailable ({status}): {message}"
            ))
        }
        other => DirectUploadError::Cloud(other),
    }
}

fn is_direct_control_fallback_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 404 | 405 | 410 | 501 | 503)
}

fn is_retryable_direct_put_status(status: StatusCode) -> bool {
    status == StatusCode::FORBIDDEN
        || status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

async fn validate_upload_request_matches_file(
    request: &CreateUploadRequest,
    path: &Path,
) -> CloudApiResult<()> {
    let file_size = tokio::fs::metadata(path)
        .await
        .map_err(|error| upload_file_error("read upload metadata", path, error))?
        .len();
    if request.file_size_bytes != file_size {
        return Err(CloudApiError::InvalidUpload(format!(
            "file_size_bytes is {}, but file has {} bytes",
            request.file_size_bytes, file_size
        )));
    }
    let checksum = sha256_file(path).await?;
    if request.checksum_sha256.to_ascii_lowercase() != checksum {
        return Err(CloudApiError::InvalidUpload(
            "checksum_sha256 does not match the upload file".to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn sha256_file(path: &Path) -> CloudApiResult<String> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| upload_file_error("open upload for hashing", path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|error| upload_file_error("hash upload", path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

async fn read_chunk_for_part(
    path: &Path,
    file_size: u64,
    part_size_bytes: u64,
    part_number: u16,
) -> CloudApiResult<Bytes> {
    if part_size_bytes == 0 {
        return Err(CloudApiError::InvalidUpload(
            "part size must be positive".to_string(),
        ));
    }
    if part_size_bytes > MAX_UPLOAD_PART_BYTES {
        return Err(CloudApiError::InvalidUpload(format!(
            "server part size {part_size_bytes} exceeds the {} byte client limit",
            MAX_UPLOAD_PART_BYTES
        )));
    }
    if part_number == 0 {
        return Err(CloudApiError::InvalidUpload(
            "part numbers start at 1".to_string(),
        ));
    }
    let index = u64::from(part_number - 1);
    let start = index
        .checked_mul(part_size_bytes)
        .ok_or_else(|| CloudApiError::InvalidUpload("part offset overflowed".to_string()))?;
    if start >= file_size {
        return Err(CloudApiError::InvalidUpload(format!(
            "part {part_number} starts beyond the upload file"
        )));
    }
    let length = part_size_bytes.min(file_size - start);
    let length = usize::try_from(length)
        .map_err(|_| CloudApiError::InvalidUpload("part size does not fit usize".to_string()))?;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| upload_file_error("open upload part", path, error))?;
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|error| upload_file_error("seek upload part", path, error))?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)
        .await
        .map_err(|error| upload_file_error("read upload part", path, error))?;
    Ok(Bytes::from(bytes))
}

fn validate_missing_parts(
    missing_parts: &[u16],
    file_size: u64,
    part_size_bytes: u64,
) -> CloudApiResult<()> {
    if part_size_bytes == 0 {
        return Err(CloudApiError::InvalidUpload(
            "part size must be positive".to_string(),
        ));
    }
    if part_size_bytes > MAX_UPLOAD_PART_BYTES {
        return Err(CloudApiError::InvalidUpload(format!(
            "server part size {part_size_bytes} exceeds the {} byte client limit",
            MAX_UPLOAD_PART_BYTES
        )));
    }

    let total_parts = file_size.div_ceil(part_size_bytes);
    if total_parts > u64::from(u16::MAX) {
        return Err(CloudApiError::InvalidUpload(format!(
            "upload requires {total_parts} parts, exceeding the protocol limit"
        )));
    }

    let mut seen = BTreeSet::new();
    for &part_number in missing_parts {
        if part_number == 0 {
            return Err(CloudApiError::InvalidUpload(
                "part numbers start at 1".to_string(),
            ));
        }
        if u64::from(part_number) > total_parts {
            return Err(CloudApiError::InvalidUpload(format!(
                "part {part_number} starts beyond the upload file"
            )));
        }
        if !seen.insert(part_number) {
            return Err(CloudApiError::InvalidUpload(format!(
                "server returned duplicate part {part_number}"
            )));
        }
    }
    Ok(())
}

fn upload_file_error(action: &str, path: &Path, error: std::io::Error) -> CloudApiError {
    CloudApiError::InvalidUpload(format!("{action} {path:?}: {error}"))
}

#[derive(Clone, Copy)]
struct UploadTransport<'a> {
    client: &'a CloudClient,
    authenticated_control: &'a reqwest::Client,
    authenticated_stream: &'a reqwest::Client,
    object_http: &'a reqwest::Client,
    device_token: &'a str,
}

#[derive(Clone, Copy)]
struct DirectPartTemplates<'a> {
    presign: &'a str,
    ack: &'a str,
}

#[derive(Debug)]
enum DirectUploadError {
    Fallback(String),
    Cloud(CloudApiError),
}

impl DirectUploadError {
    fn into_cloud_error(self) -> CloudApiError {
        match self {
            Self::Fallback(message) => CloudApiError::InvalidUpload(message),
            Self::Cloud(error) => error,
        }
    }
}

#[derive(Debug)]
enum DirectPutError {
    Retryable(String),
    Fallback(String),
    Terminal(CloudApiError),
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use httpmock::Mock;
    use serde_json::json;

    #[tokio::test]
    async fn file_hash_and_part_reads_are_seekable_and_bounded() {
        let path =
            std::env::temp_dir().join(format!("clipline-upload-source-{}.bin", std::process::id()));
        tokio::fs::write(&path, b"abcdef").await.unwrap();

        assert_eq!(sha256_file(&path).await.unwrap(), sha256_hex(b"abcdef"));
        assert_eq!(
            read_chunk_for_part(&path, 6, 3, 2).await.unwrap(),
            Bytes::from_static(b"def")
        );
        let _ = tokio::fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn hostile_part_size_is_rejected_before_file_allocation() {
        let missing = Path::new("this-file-must-not-be-opened.mp4");
        let error = read_chunk_for_part(missing, u64::MAX, MAX_UPLOAD_PART_BYTES + 1, 1)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("exceeds"), "{error}");
    }

    #[test]
    fn multipart_work_list_rejects_zero_duplicates_and_out_of_range_parts() {
        let zero = validate_missing_parts(&[0], 6, 3).unwrap_err();
        assert!(zero.to_string().contains("start at 1"), "{zero}");

        let duplicate = validate_missing_parts(&[1, 2, 1], 6, 3).unwrap_err();
        assert!(
            duplicate.to_string().contains("duplicate part 1"),
            "{duplicate}"
        );

        let out_of_range = validate_missing_parts(&[3], 6, 3).unwrap_err();
        assert!(
            out_of_range.to_string().contains("beyond"),
            "{out_of_range}"
        );
    }

    #[test]
    fn multipart_work_list_preserves_valid_resumable_subset() {
        validate_missing_parts(&[3, 1], 7, 3).unwrap();
        validate_missing_parts(&[], 7, 3).unwrap();
    }

    const TOKEN: &str = "device-token";

    #[test]
    fn create_upload_body_includes_description_and_omits_markers() {
        let bytes = b"abc";
        let mut request = upload_request(bytes);
        request.markers = Some(vec![clipline_cloud_api::types::CreateMarkerRequest {
            kind: "ChampionKill".to_string(),
            label: Some("kill".to_string()),
            timestamp_ms: 1200,
            metadata: Some(json!({ "deprecated": true })),
        }]);

        let body = create_upload_body(&request, Some("  Useful context  ")).unwrap();

        assert_eq!(body["description"], "Useful context");
        assert_eq!(body["checksum_sha256"], sha256_hex(bytes));
        assert!(body.get("markers").is_none());
    }

    #[test]
    fn create_upload_body_omits_blank_description() {
        let body = create_upload_body(&upload_request(b"abc"), Some(" \t\n ")).unwrap();

        assert!(body.get("description").is_none());
        assert!(body.get("markers").is_none());
    }

    #[test]
    fn authenticated_upload_urls_stay_on_the_configured_cloud_origin() {
        let cloud = MockServer::start();
        let other = MockServer::start();
        let client = test_client(&cloud);

        assert!(upload_url(&client, "/api/v1/uploads/u1/content", 0).is_ok());
        assert!(upload_url(
            &client,
            &format!("{}/api/v1/uploads/u1/content", cloud.base_url()),
            0,
        )
        .is_ok());
        assert!(upload_url(
            &client,
            &format!("{}/api/v1/uploads/u1/content", other.base_url()),
            0,
        )
        .is_err());
    }

    #[test]
    fn authenticated_upload_url_rejects_scheme_downgrade_and_port_change() {
        let client =
            CloudClient::with_device_token("https://cloud.example:8443/".parse().unwrap(), TOKEN);

        assert!(upload_url(
            &client,
            "http://cloud.example:8443/api/v1/uploads/u1/content",
            0,
        )
        .is_err());
        assert!(upload_url(
            &client,
            "https://cloud.example:9443/api/v1/uploads/u1/content",
            0,
        )
        .is_err());
    }

    #[tokio::test]
    async fn authenticated_create_upload_does_not_follow_redirects() {
        let cloud = MockServer::start();
        let target = MockServer::start();
        let redirected = target.mock(|when, then| {
            when.method(GET).path("/stolen");
            then.status(400)
                .json_body(json!({ "error": "reached target" }));
        });
        cloud.mock(|when, then| {
            when.method(POST).path("/api/v1/uploads");
            then.status(302)
                .header("Location", format!("{}/stolen", target.base_url()));
        });
        let client = test_client(&cloud);
        let http = crate::bounded_http::control_client().unwrap();

        let error = create_upload(&client, http, TOKEN, &upload_request(b"abc"), None)
            .await
            .expect_err("redirect must not be followed");

        assert!(error.to_string().contains("302"), "{error}");
        redirected.assert_hits(0);
    }

    #[tokio::test]
    async fn discovery_without_direct_s3_uses_proxy_chunked_path() {
        let bytes = b"abcdef";
        let cloud = MockServer::start();
        mount_discovery(&cloud, false);
        mount_chunked_create(
            &cloud,
            "u1",
            "c1",
            Some("/api/v1/uploads/u1/parts/{part_number}/presign"),
            Some("/api/v1/uploads/u1/parts/{part_number}/ack"),
        );
        mount_progress(
            &cloud,
            "u1",
            "c1",
            "uploading",
            bytes.len() as u64,
            vec![1, 2],
        );
        let part1 = mount_proxy_part(&cloud, "u1", 1, "abc");
        let part2 = mount_proxy_part(&cloud, "u1", 2, "def");
        let complete = mount_complete(&cloud, "u1", "c1", bytes.len() as u64);

        let client = test_client(&cloud);
        let progress = upload_mp4_bytes_with_progress(
            &client,
            TOKEN,
            &upload_request(bytes),
            None,
            bytes,
            |_| {},
        )
        .await
        .expect("upload");

        assert_eq!(progress.status, "completed");
        part1.assert();
        part2.assert();
        complete.assert();
    }

    #[tokio::test]
    async fn direct_s3_does_not_change_single_put_uploads() {
        let bytes = b"single body";
        let cloud = MockServer::start();
        mount_discovery(&cloud, true);
        cloud.mock(|when, then| {
            when.method(POST).path("/api/v1/uploads");
            then.status(200).json_body(json!({
                "clip_id": "c1",
                "upload_id": "u1",
                "mode": "single_put",
                "part_size_bytes": bytes.len(),
                "single_put_url": "/api/v1/uploads/u1/content",
                "parts_url_template": null
            }));
        });
        mount_progress(&cloud, "u1", "c1", "uploading", bytes.len() as u64, vec![]);
        let single_put = cloud.mock(|when, then| {
            when.method(PUT)
                .path("/api/v1/uploads/u1/content")
                .body("single body");
            then.status(200).json_body(progress_json(
                "u1",
                "c1",
                "single_put",
                "completed",
                bytes.len() as u64,
                bytes.len() as u64,
                vec![],
            ));
        });

        let client = test_client(&cloud);
        let progress = upload_mp4_bytes_with_progress(
            &client,
            TOKEN,
            &upload_request(bytes),
            None,
            bytes,
            |_| {},
        )
        .await
        .expect("upload");

        assert_eq!(progress.status, "completed");
        single_put.assert();
    }

    #[tokio::test]
    async fn direct_s3_chunked_upload_presigns_puts_acks_and_completes() {
        let bytes = b"abcdef";
        let cloud = MockServer::start();
        let s3 = MockServer::start();
        mount_discovery(&cloud, true);
        mount_chunked_create(
            &cloud,
            "u1",
            "c1",
            Some("/api/v1/uploads/u1/parts/{part_number}/direct-presign"),
            Some("/api/v1/uploads/u1/parts/{part_number}/direct-ack"),
        );
        mount_progress(
            &cloud,
            "u1",
            "c1",
            "uploading",
            bytes.len() as u64,
            vec![1, 2],
        );
        let presign1 = mount_presign(&cloud, &s3, "u1", 1, 3, "/s3-part-1", "abc");
        let presign2 = mount_presign(&cloud, &s3, "u1", 2, 3, "/s3-part-2", "def");
        let put1 = mount_s3_put(&s3, "/s3-part-1", "abc", "\"etag-1\"", 200);
        let put2 = mount_s3_put(&s3, "/s3-part-2", "def", "\"etag-2\"", 200);
        let ack1 = mount_ack(&cloud, "u1", 1, "\"etag-1\"", "abc", 200);
        let ack2 = mount_ack(&cloud, "u1", 2, "\"etag-2\"", "def", 200);
        let complete = mount_complete(&cloud, "u1", "c1", bytes.len() as u64);

        let client = test_client(&cloud);
        let progress = upload_mp4_bytes_with_progress(
            &client,
            TOKEN,
            &upload_request(bytes),
            None,
            bytes,
            |_| {},
        )
        .await
        .expect("upload");

        assert_eq!(progress.status, "completed");
        for mock in [presign1, presign2, put1, put2, ack1, ack2, complete] {
            mock.assert();
        }
    }

    #[tokio::test]
    async fn direct_s3_put_expiry_requests_fresh_presign_for_same_part() {
        let bytes = b"abc";
        let cloud = MockServer::start();
        let s3 = MockServer::start();
        mount_discovery(&cloud, true);
        mount_chunked_create(
            &cloud,
            "u1",
            "c1",
            Some("/api/v1/uploads/u1/parts/{part_number}/direct-presign"),
            Some("/api/v1/uploads/u1/parts/{part_number}/direct-ack"),
        );
        mount_progress(&cloud, "u1", "c1", "uploading", bytes.len() as u64, vec![1]);
        let presign = mount_presign(&cloud, &s3, "u1", 1, 3, "/expired-part-1", "abc");
        let expired_put = mount_s3_put(&s3, "/expired-part-1", "abc", "\"expired\"", 403);

        let client = test_client(&cloud);
        let error = upload_mp4_bytes_with_progress(
            &client,
            TOKEN,
            &upload_request(bytes),
            None,
            bytes,
            |_| {},
        )
        .await
        .expect_err("expired presign");

        assert!(
            error
                .to_string()
                .contains("failed after refreshing presign"),
            "{error}"
        );
        presign.assert_hits(DIRECT_PUT_MAX_ATTEMPTS);
        expired_put.assert_hits(DIRECT_PUT_MAX_ATTEMPTS);
    }

    #[tokio::test]
    async fn missing_direct_s3_etag_fails_with_retryable_upload_guidance() {
        let bytes = b"abc";
        let cloud = MockServer::start();
        let s3 = MockServer::start();
        mount_discovery(&cloud, true);
        mount_chunked_create(
            &cloud,
            "u1",
            "c1",
            Some("/api/v1/uploads/u1/parts/{part_number}/direct-presign"),
            Some("/api/v1/uploads/u1/parts/{part_number}/direct-ack"),
        );
        mount_progress(&cloud, "u1", "c1", "uploading", bytes.len() as u64, vec![1]);
        mount_presign(&cloud, &s3, "u1", 1, 3, "/s3-part-1", "abc");
        s3.mock(|when, then| {
            when.method(PUT).path("/s3-part-1").body("abc");
            then.status(200);
        });

        let client = test_client(&cloud);
        let error = upload_mp4_bytes_with_progress(
            &client,
            TOKEN,
            &upload_request(bytes),
            None,
            bytes,
            |_| {},
        )
        .await
        .expect_err("etag missing");

        assert!(
            error
                .to_string()
                .contains("direct S3 upload did not return an ETag"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn direct_s3_ack_conflict_surfaces_clear_retry_guidance() {
        let bytes = b"abc";
        let cloud = MockServer::start();
        let s3 = MockServer::start();
        mount_discovery(&cloud, true);
        mount_chunked_create(
            &cloud,
            "u1",
            "c1",
            Some("/api/v1/uploads/u1/parts/{part_number}/direct-presign"),
            Some("/api/v1/uploads/u1/parts/{part_number}/direct-ack"),
        );
        mount_progress(&cloud, "u1", "c1", "uploading", bytes.len() as u64, vec![1]);
        mount_presign(&cloud, &s3, "u1", 1, 3, "/s3-part-1", "abc");
        mount_s3_put(&s3, "/s3-part-1", "abc", "\"etag-1\"", 200);
        mount_ack(&cloud, "u1", 1, "\"etag-1\"", "abc", 409);

        let client = test_client(&cloud);
        let error = upload_mp4_bytes_with_progress(
            &client,
            TOKEN,
            &upload_request(bytes),
            None,
            bytes,
            |_| {},
        )
        .await
        .expect_err("ack conflict");

        assert!(error.to_string().contains("Retry the upload"), "{error}");
    }

    #[tokio::test]
    async fn direct_s3_template_missing_falls_back_to_proxy_parts() {
        let bytes = b"abcdef";
        let cloud = MockServer::start();
        mount_discovery(&cloud, true);
        mount_chunked_create(&cloud, "u1", "c1", None, None);
        mount_progress(
            &cloud,
            "u1",
            "c1",
            "uploading",
            bytes.len() as u64,
            vec![1, 2],
        );
        let part1 = mount_proxy_part(&cloud, "u1", 1, "abc");
        let part2 = mount_proxy_part(&cloud, "u1", 2, "def");
        mount_complete(&cloud, "u1", "c1", bytes.len() as u64);

        let client = test_client(&cloud);
        let progress = upload_mp4_bytes_with_progress(
            &client,
            TOKEN,
            &upload_request(bytes),
            None,
            bytes,
            |_| {},
        )
        .await
        .expect("upload");

        assert_eq!(progress.status, "completed");
        part1.assert();
        part2.assert();
    }

    #[tokio::test]
    async fn direct_s3_provider_failure_restarts_with_proxy_upload() {
        let bytes = b"abcdef";
        let cloud = MockServer::start();
        let s3 = MockServer::start();
        mount_discovery(&cloud, true);
        let create = cloud.mock(|when, then| {
            when.method(POST)
                .path("/api/v1/uploads")
                .json_body_partial(r#"{"description":"Retry context"}"#);
            then.status(200).json_body(json!({
                "clip_id": "c1",
                "upload_id": "u1",
                "mode": "chunked",
                "part_size_bytes": 3,
                "single_put_url": null,
                "parts_url_template": "/api/v1/uploads/u1/parts/{part_number}",
                "direct_part_presign_url_template": "/api/v1/uploads/u1/parts/{part_number}/direct-presign",
                "direct_part_ack_url_template": "/api/v1/uploads/u1/parts/{part_number}/direct-ack"
            }));
        });
        mount_progress(
            &cloud,
            "u1",
            "c1",
            "uploading",
            bytes.len() as u64,
            vec![1, 2],
        );
        mount_presign(&cloud, &s3, "u1", 1, 3, "/provider-fail", "abc");
        let failed_put = mount_s3_put(&s3, "/provider-fail", "abc", "\"bad\"", 400);
        let proxy_part1 = mount_proxy_part(&cloud, "u1", 1, "abc");
        let proxy_part2 = mount_proxy_part(&cloud, "u1", 2, "def");
        let complete = mount_complete(&cloud, "u1", "c1", bytes.len() as u64);

        let client = test_client(&cloud);
        let progress = upload_mp4_bytes_with_progress(
            &client,
            TOKEN,
            &upload_request(bytes),
            Some("  Retry context  "),
            bytes,
            |_| {},
        )
        .await
        .expect("fallback upload");

        assert_eq!(progress.clip_id, "c1");
        create.assert_hits(2);
        failed_put.assert();
        proxy_part1.assert();
        proxy_part2.assert();
        complete.assert();
    }

    fn test_client(server: &MockServer) -> CloudClient {
        let base_url =
            clipline_cloud_api::validate_cloud_host(&server.base_url(), true).expect("cloud URL");
        CloudClient::with_device_token(base_url, TOKEN)
    }

    fn upload_request(bytes: &[u8]) -> CreateUploadRequest {
        CreateUploadRequest {
            client_clip_id: Some("local-1".to_string()),
            title: "clip".to_string(),
            description: None,
            game_name: None,
            game_id: None,
            game_executable: None,
            source_type: Some("replay".to_string()),
            recorded_at: None,
            duration_ms: None,
            file_size_bytes: bytes.len() as u64,
            checksum_sha256: sha256_hex(bytes),
            container: "mp4".to_string(),
            video_codec: Some("h264".to_string()),
            audio_codec: None,
            width: None,
            height: None,
            fps: None,
            visibility: Some("private".to_string()),
            markers: None,
        }
    }

    fn mount_discovery(server: &MockServer, direct_s3_upload: bool) -> Mock<'_> {
        server.mock(|when, then| {
            when.method(GET).path("/.well-known/clipline-cloud");
            then.status(200).json_body(json!({
                "name": "Clipline Cloud",
                "api_version": "v1",
                "server_version": "1.0.0",
                "min_client_version": "0.1.0",
                "public_url": server.base_url(),
                "features": {
                    "single_put_upload": true,
                    "chunked_upload": true,
                    "direct_s3_upload": direct_s3_upload,
                    "public_sharing": true,
                    "clip_markers": true,
                    "max_upload_size_bytes": 1000000
                }
            }));
        })
    }

    fn mount_chunked_create<'a>(
        server: &'a MockServer,
        upload_id: &str,
        clip_id: &str,
        presign_template: Option<&str>,
        ack_template: Option<&str>,
    ) -> Mock<'a> {
        server.mock(|when, then| {
            when.method(POST).path("/api/v1/uploads");
            then.status(200).json_body(json!({
                "clip_id": clip_id,
                "upload_id": upload_id,
                "mode": "chunked",
                "part_size_bytes": 3,
                "single_put_url": null,
                "parts_url_template": format!("/api/v1/uploads/{upload_id}/parts/{{part_number}}"),
                "direct_part_presign_url_template": presign_template,
                "direct_part_ack_url_template": ack_template
            }));
        })
    }

    fn mount_progress<'a>(
        server: &'a MockServer,
        upload_id: &str,
        clip_id: &str,
        status: &str,
        file_size: u64,
        missing_parts: Vec<u16>,
    ) -> Mock<'a> {
        server.mock(|when, then| {
            when.method(GET)
                .path(format!("/api/v1/uploads/{upload_id}"));
            then.status(200).json_body(progress_json(
                upload_id,
                clip_id,
                "chunked",
                status,
                file_size,
                0,
                missing_parts,
            ));
        })
    }

    fn mount_proxy_part<'a>(
        server: &'a MockServer,
        upload_id: &str,
        part_number: u16,
        body: &str,
    ) -> Mock<'a> {
        server.mock(|when, then| {
            when.method(PUT)
                .path(format!("/api/v1/uploads/{upload_id}/parts/{part_number}"))
                .body(body);
            then.status(200).json_body(json!({
                "upload_id": upload_id,
                "part_number": part_number,
                "size_bytes": body.len(),
                "checksum_sha256": sha256_hex(body.as_bytes()),
                "etag": null,
                "idempotent": false
            }));
        })
    }

    fn mount_presign<'a>(
        cloud: &'a MockServer,
        s3: &MockServer,
        upload_id: &str,
        part_number: u16,
        expected_size_bytes: u64,
        s3_path: &str,
        expected_body: &str,
    ) -> Mock<'a> {
        cloud.mock(|when, then| {
            when.method(POST).path(format!(
                "/api/v1/uploads/{upload_id}/parts/{part_number}/direct-presign"
            ));
            then.status(200).json_body(json!({
                "upload_id": upload_id,
                "part_number": part_number,
                "method": "PUT",
                "url": format!("{}{}", s3.base_url(), s3_path),
                "expires_at": "2030-01-01T00:00:00Z",
                "expected_size_bytes": expected_size_bytes,
                "headers": [
                    { "name": "x-amz-meta-clipline-test", "value": expected_body }
                ]
            }));
        })
    }

    fn mount_s3_put<'a>(
        s3: &'a MockServer,
        path: &str,
        body: &str,
        etag: &str,
        status: u16,
    ) -> Mock<'a> {
        s3.mock(|when, then| {
            when.method(PUT)
                .path(path)
                .header("x-amz-meta-clipline-test", body)
                .body(body);
            then.status(status).header("ETag", etag);
        })
    }

    fn mount_ack<'a>(
        server: &'a MockServer,
        upload_id: &str,
        part_number: u16,
        etag: &str,
        body: &str,
        status: u16,
    ) -> Mock<'a> {
        server.mock(|when, then| {
            when.method(POST)
                .path(format!(
                    "/api/v1/uploads/{upload_id}/parts/{part_number}/direct-ack"
                ))
                .json_body(json!({
                    "size_bytes": body.len(),
                    "checksum_sha256": sha256_hex(body.as_bytes()),
                    "etag": etag
                }));
            if status == 409 {
                then.status(409)
                    .json_body(json!({ "error": "part metadata conflict" }));
            } else {
                then.status(status).json_body(json!({
                    "upload_id": upload_id,
                    "part_number": part_number,
                    "size_bytes": body.len(),
                    "checksum_sha256": sha256_hex(body.as_bytes()),
                    "etag": etag,
                    "idempotent": false
                }));
            }
        })
    }

    fn mount_complete<'a>(
        server: &'a MockServer,
        upload_id: &str,
        clip_id: &str,
        file_size: u64,
    ) -> Mock<'a> {
        server.mock(|when, then| {
            when.method(POST)
                .path(format!("/api/v1/uploads/{upload_id}/complete"));
            then.status(200).json_body(progress_json(
                upload_id,
                clip_id,
                "chunked",
                "completed",
                file_size,
                file_size,
                vec![],
            ));
        })
    }

    fn progress_json(
        upload_id: &str,
        clip_id: &str,
        mode: &str,
        status: &str,
        file_size: u64,
        received_size: u64,
        missing_parts: Vec<u16>,
    ) -> serde_json::Value {
        let missing_part_count = missing_parts.len() as u16;
        let progress_basis_points = received_size
            .saturating_mul(10000)
            .checked_div(file_size)
            .unwrap_or(0) as u16;
        json!({
            "upload_id": upload_id,
            "clip_id": clip_id,
            "mode": mode,
            "status": status,
            "file_size_bytes": file_size,
            "part_size_bytes": 3,
            "received_size_bytes": received_size,
            "total_parts": 2,
            "received_part_count": 2_u16.saturating_sub(missing_part_count),
            "missing_part_count": missing_part_count,
            "next_part_number": missing_parts.first().copied(),
            "progress_basis_points": progress_basis_points,
            "failure_reason": null,
            "recovery_action": null,
            "expires_at": "2030-01-01T00:00:00Z",
            "received_parts": [],
            "missing_parts": missing_parts
        })
    }
}
