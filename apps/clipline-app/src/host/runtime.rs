use std::sync::{Arc, Mutex};

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

#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
pub struct FallbackHostContext {
    settings: Mutex<crate::settings::AppSettings>,
    events: Arc<crate::host::events::ClientEventHub>,
}

#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
impl FallbackHostContext {
    pub fn new(
        settings: crate::settings::AppSettings,
        events: Arc<crate::host::events::ClientEventHub>,
    ) -> Self {
        Self {
            settings: Mutex::new(settings),
            events,
        }
    }

    pub fn settings(&self) -> crate::settings::AppSettings {
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    pub fn events(&self) -> Arc<crate::host::events::ClientEventHub> {
        self.events.clone()
    }

    #[cfg(test)]
    fn for_tests(settings: crate::settings::AppSettings) -> Self {
        Self::new(
            settings,
            Arc::new(crate::host::events::ClientEventHub::default()),
        )
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

    #[test]
    fn fallback_context_exposes_initial_settings() {
        let context = FallbackHostContext::for_tests(crate::settings::AppSettings::default());

        assert_eq!(context.settings().hotkey, "Alt+F10");
    }
}
