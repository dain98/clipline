#[allow(
    dead_code,
    reason = "used by fallback runtime tasks after the startup decision contract is introduced"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebviewPreflight {
    Available,
    Missing,
}

#[allow(
    dead_code,
    reason = "used by fallback runtime tasks after the startup decision contract is introduced"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackLaunchPreference {
    UseTauri,
    StartFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebviewHealthSignal {
    GetterFailedToReceiveMessage,
    FrontendReadyTimeout,
    FallbackLaunchFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebviewFailureAction {
    StartFallback,
    ShowNativeDiagnostic,
    Ignore,
}

#[allow(
    dead_code,
    reason = "used by fallback runtime tasks after the startup decision contract is introduced"
)]
pub fn fallback_launch_preference(
    args: &[String],
    preflight: WebviewPreflight,
) -> FallbackLaunchPreference {
    if args.iter().any(|arg| arg == "--force-fallback-client") {
        return FallbackLaunchPreference::StartFallback;
    }
    match preflight {
        WebviewPreflight::Available => FallbackLaunchPreference::UseTauri,
        WebviewPreflight::Missing => FallbackLaunchPreference::StartFallback,
    }
}

#[allow(
    dead_code,
    reason = "used by fallback runtime tasks after the startup decision contract is introduced"
)]
pub fn requested_fallback_port(args: &[String]) -> Option<u16> {
    args.windows(2)
        .find(|window| window[0] == "--fallback-port")
        .and_then(|window| window[1].parse::<u16>().ok())
}

pub fn launch_decision_after_webview_health(
    signal: WebviewHealthSignal,
    fallback_already_started: bool,
) -> WebviewFailureAction {
    if fallback_already_started {
        return WebviewFailureAction::Ignore;
    }
    match signal {
        WebviewHealthSignal::GetterFailedToReceiveMessage
        | WebviewHealthSignal::FrontendReadyTimeout => WebviewFailureAction::StartFallback,
        WebviewHealthSignal::FallbackLaunchFailed => WebviewFailureAction::ShowNativeDiagnostic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_flag_selects_fallback() {
        let args = vec![
            "clipline-app.exe".to_string(),
            "--force-fallback-client".to_string(),
        ];
        assert_eq!(
            fallback_launch_preference(&args, WebviewPreflight::Available),
            FallbackLaunchPreference::StartFallback
        );
    }

    #[test]
    fn missing_webview_selects_fallback() {
        let args = vec!["clipline-app.exe".to_string()];
        assert_eq!(
            fallback_launch_preference(&args, WebviewPreflight::Missing),
            FallbackLaunchPreference::StartFallback
        );
    }

    #[test]
    fn available_webview_uses_tauri() {
        let args = vec!["clipline-app.exe".to_string()];
        assert_eq!(
            fallback_launch_preference(&args, WebviewPreflight::Available),
            FallbackLaunchPreference::UseTauri
        );
    }

    #[test]
    fn requested_fallback_port_reads_valid_value() {
        let args = vec![
            "clipline-app.exe".to_string(),
            "--fallback-port".to_string(),
            "49152".to_string(),
        ];

        assert_eq!(requested_fallback_port(&args), Some(49152));
    }

    #[test]
    fn requested_fallback_port_returns_none_when_flag_absent() {
        let args = vec!["clipline-app.exe".to_string()];

        assert_eq!(requested_fallback_port(&args), None);
    }

    #[test]
    fn requested_fallback_port_returns_none_for_invalid_value() {
        let args = vec![
            "clipline-app.exe".to_string(),
            "--fallback-port".to_string(),
            "not-a-port".to_string(),
        ];

        assert_eq!(requested_fallback_port(&args), None);
    }

    #[test]
    fn requested_fallback_port_returns_none_for_trailing_flag() {
        let args = vec![
            "clipline-app.exe".to_string(),
            "--fallback-port".to_string(),
        ];

        assert_eq!(requested_fallback_port(&args), None);
    }

    #[test]
    fn requested_fallback_port_uses_first_flag_even_when_later_flag_is_valid() {
        let args = vec![
            "clipline-app.exe".to_string(),
            "--fallback-port".to_string(),
            "not-a-port".to_string(),
            "--fallback-port".to_string(),
            "49152".to_string(),
        ];

        assert_eq!(requested_fallback_port(&args), None);
    }

    #[test]
    fn dead_webview_health_signal_selects_fallback() {
        assert_eq!(
            launch_decision_after_webview_health(
                WebviewHealthSignal::GetterFailedToReceiveMessage,
                false
            ),
            WebviewFailureAction::StartFallback
        );
        assert_eq!(
            launch_decision_after_webview_health(WebviewHealthSignal::FrontendReadyTimeout, false),
            WebviewFailureAction::StartFallback
        );
    }

    #[test]
    fn fallback_failure_shows_repair_notice() {
        assert_eq!(
            launch_decision_after_webview_health(WebviewHealthSignal::FallbackLaunchFailed, false),
            WebviewFailureAction::ShowNativeDiagnostic
        );
    }

    #[test]
    fn already_started_health_signal_is_ignored() {
        assert_eq!(
            launch_decision_after_webview_health(
                WebviewHealthSignal::GetterFailedToReceiveMessage,
                true
            ),
            WebviewFailureAction::Ignore
        );
        assert_eq!(
            launch_decision_after_webview_health(WebviewHealthSignal::FrontendReadyTimeout, true),
            WebviewFailureAction::Ignore
        );
    }
}
