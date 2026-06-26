#[derive(Debug, serde::Serialize)]
pub struct FallbackCommandResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl FallbackCommandResult {
    pub fn ok(value: serde_json::Value) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_result_serializes_success_and_error_for_fallback_bridge() {
        let ok = FallbackCommandResult::ok(serde_json::json!({"recording": true}));
        let err = FallbackCommandResult::err("failed");

        assert_eq!(serde_json::to_value(ok).unwrap()["ok"], true);
        assert_eq!(serde_json::to_value(err).unwrap()["error"], "failed");
    }
}
