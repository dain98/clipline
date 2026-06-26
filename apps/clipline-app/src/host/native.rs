pub fn validate_external_url(url: &str) -> Result<String, String> {
    let parsed =
        reqwest::Url::parse(url.trim()).map_err(|e| format!("external URL is invalid: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed.to_string()),
        _ => Err("only http and https URLs can be opened".into()),
    }
}

pub fn open_external_url(url: &str, context: &str) -> Result<(), String> {
    let url = validate_external_url(url)?;
    crate::cloud::open_cloud_url_for_host(&url, context)
}

pub fn open_folder(path: &std::path::Path) -> Result<(), String> {
    crate::library::open_folder_path(path)
}

pub fn copy_file_to_clipboard(path: &std::path::Path) -> Result<(), String> {
    crate::library::copy_file_to_clipboard(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_url_validator_accepts_only_http_urls() {
        assert_eq!(
            validate_external_url(" https://clipline.test/user ").as_deref(),
            Ok("https://clipline.test/user")
        );
        assert_eq!(
            validate_external_url("http://127.0.0.1:3000").as_deref(),
            Ok("http://127.0.0.1:3000/")
        );
        assert!(validate_external_url("file:///C:/secret.txt").is_err());
    }
}
