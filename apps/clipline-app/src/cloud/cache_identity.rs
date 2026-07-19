pub(super) fn validate_cloud_cache_component<'a>(
    value: &'a str,
    label: &str,
) -> Result<&'a str, String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(format!("{label} contains unsupported characters"));
    }
    Ok(trimmed)
}

pub(super) fn cloud_cache_namespace(base: &str, account: &str) -> String {
    let key = format!("{}|{}", base.trim_end_matches('/'), account.trim());
    clipline_cloud_api::sha256_hex(key.as_bytes())[..16].to_string()
}

pub(super) fn cloud_cache_file_name(
    remote_clip_id: &str,
    asset: &str,
    extension: &str,
    version: u64,
) -> Result<String, String> {
    let remote_clip_id = validate_cloud_cache_component(remote_clip_id, "remote clip id")?;
    let asset = validate_cloud_cache_component(asset, "cloud asset")?;
    let extension = validate_cloud_cache_component(extension, "cloud asset extension")?;
    Ok(format!("{remote_clip_id}-{asset}-{version}.{extension}"))
}
