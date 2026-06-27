//! Structural contract for the review player DOM: Clipline owns the controls,
//! the browser owns nothing, and the UI stays split into testable assets.

use std::fs;
use std::path::Path;

fn index_html() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/index.html");
    fs::read_to_string(path).expect("read ui/index.html")
}

fn main_js() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/main.js");
    fs::read_to_string(path).expect("read ui/main.js")
}

fn client_bridge_js() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/client-bridge.js");
    fs::read_to_string(path).expect("read ui/client-bridge.js")
}

fn fallback_manifest_rs() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fallback/manifest.rs");
    fs::read_to_string(path).expect("read src/fallback/manifest.rs")
}

fn quoted_calls(source: &str, function_name: &str) -> Vec<String> {
    let needle = format!("{function_name}(\"");
    let mut values = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find(&needle) {
        let value_start = start + needle.len();
        let tail = &rest[value_start..];
        let end = tail.find('"').expect("quoted call closes");
        values.push(tail[..end].to_string());
        rest = &tail[end + 1..];
    }
    values.sort();
    values.dedup();
    values
}

fn quoted_manifest_array(source: &str, array_name: &str) -> Vec<String> {
    let marker = format!("pub const {array_name}: &[&str] = &[");
    let start = source.find(&marker).expect("manifest array exists") + marker.len();
    let tail = &source[start..];
    let end = tail.find("];").expect("manifest array closes");
    tail[..end]
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim().trim_end_matches(',');
            trimmed
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .map(str::to_string)
        })
        .collect()
}

fn styles_css() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/styles.css");
    fs::read_to_string(path).expect("read ui/styles.css")
}

fn app_rs() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs");
    fs::read_to_string(path).expect("read src/app.rs")
}

fn compact_source(source: &str) -> String {
    source.chars().filter(|c| !c.is_whitespace()).collect()
}

fn fallback_runtime_rs() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host/runtime.rs");
    fs::read_to_string(path).expect("read src/host/runtime.rs")
}

fn fallback_server_rs() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fallback/server.rs");
    fs::read_to_string(path).expect("read src/fallback/server.rs")
}

fn tauri_config() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    fs::read_to_string(path).expect("read tauri.conf.json")
}

fn repo_file(path: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("app crate has workspace root");
    fs::read_to_string(root.join(path)).unwrap_or_else(|_| panic!("read {path}"))
}

fn fallback_validation_script() -> String {
    repo_file("scripts/validate-fallback-client.ps1")
}

fn source_after<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing marker {marker}"));
    &source[start..]
}

fn source_between<'a>(source: &'a str, start_marker: &str, end_marker: &str) -> &'a str {
    let tail = source_after(source, start_marker);
    let end = tail
        .find(end_marker)
        .unwrap_or_else(|| panic!("missing marker {end_marker} after {start_marker}"));
    &tail[..end]
}

fn quoted_strings(source: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find('"') {
        let tail = &rest[start + 1..];
        let Some(end) = tail.find('"') else {
            break;
        };
        values.push(tail[..end].to_string());
        rest = &tail[end + 1..];
    }
    values.sort();
    values.dedup();
    values
}

fn quoted_match_arm_values(match_source: &str) -> Vec<String> {
    let mut values = Vec::new();
    for line in match_source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("_ =>") {
            break;
        }
        let pattern = trimmed.split("=>").next().unwrap_or(trimmed).trim();
        if pattern.starts_with('"') || pattern.starts_with("| \"") {
            values.extend(quoted_strings(pattern));
        }
    }
    values.sort();
    values.dedup();
    values
}

#[test]
fn default_capability_only_targets_main_window() {
    let capability =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json"))
            .expect("read default capability");

    assert!(
        capability.contains("\"windows\": [\"main\"]"),
        "frontend commands should only target Clipline's main window"
    );
    assert!(
        !capability.contains("main-recovery"),
        "recovery windows are intentionally not created or granted frontend command permissions"
    );
}

#[test]
fn windows_installer_repairs_webview2_with_bootstrapper() {
    let config = tauri_config();

    assert!(
        config.contains("\"minimumWebview2Version\": \"120.0.2210.55\""),
        "Windows 10 installs must repair/update stale WebView2 runtimes before Clipline starts"
    );
    assert!(
        config.contains("\"webviewInstallMode\"")
            && config.contains("\"type\": \"embedBootstrapper\""),
        "the default NSIS installer should embed the small Evergreen bootstrapper instead of bundling the offline WebView2 installer"
    );
}

#[test]
fn main_window_is_not_created_before_setup() {
    let config: serde_json::Value =
        serde_json::from_str(&tauri_config()).expect("tauri.conf.json is valid JSON");
    let windows = config["app"]["windows"]
        .as_array()
        .expect("Tauri app windows are configured");
    let main_window = windows
        .first()
        .expect("Clipline config has a main app window");

    assert_eq!(
        main_window["create"].as_bool(),
        Some(false),
        "fallback startup must be selected during setup before any WebView is created"
    );
}

#[test]
fn frontend_reports_webview_readiness_to_native_shell() {
    let app = app_rs();
    let js = main_js();

    assert!(
        app.contains("fn frontend_ready()") && app.contains("frontend_ready,"),
        "Rust shell must expose a lightweight frontend_ready command"
    );
    assert!(
        js.contains("invoke(\"frontend_ready\")"),
        "main.js must report readiness once the frontend JavaScript boots"
    );
}

#[test]
fn frontend_uses_host_bridge_instead_of_tauri_directly() {
    let html = index_html();
    let js = main_js();
    let bridge = client_bridge_js();

    assert!(
        html.find("client-bridge.js").is_some_and(|bridge_pos| {
            html.find("main.js")
                .is_some_and(|main_pos| bridge_pos < main_pos)
        }),
        "client-bridge.js must load before main.js"
    );
    assert!(
        bridge.contains("window.cliplineHost"),
        "bridge must expose window.cliplineHost"
    );
    assert!(
        bridge.contains("close: () => appWindow.close()"),
        "Tauri bridge close must call the real window close API"
    );
    assert!(
        !bridge.contains("close: () => null"),
        "Tauri bridge close must not be a no-op placeholder"
    );
    assert!(
        bridge.contains("invoke: (command, args) => tauri.core.invoke(command, args)")
            && bridge.contains("listen: (event, handler) => tauri.event.listen(event, handler)")
            && bridge.contains("convertFileSrc: (path) => tauri.core.convertFileSrc(path)"),
        "Tauri bridge methods must wrap Tauri exports instead of relying on method binding"
    );
    assert!(
        bridge.contains("function showBridgeError(message)")
            && bridge.contains("function parseJsonText(text, context)"),
        "fallback bridge must centralize error display and JSON parsing"
    );
    assert!(
        bridge.contains("const text = await response.text()")
            && bridge.contains("parseJsonText(text, `fallback invoke ${command}`)")
            && !bridge.contains("response.json().catch"),
        "fallback invoke must parse response text explicitly and reject malformed JSON"
    );
    assert!(
        bridge.contains("parseJsonText(message.data, `fallback event ${event}`)")
            && bridge.contains("showBridgeError(error.message)"),
        "fallback event listener must surface malformed event JSON without throwing"
    );
    assert!(
        bridge.contains("window action failed: ${action}") && bridge.contains("if (!response.ok)"),
        "fallback window actions must reject failed HTTP responses"
    );
    assert!(
        !js.contains("window.__TAURI__"),
        "main.js must use window.cliplineHost instead of direct Tauri globals"
    );
}

#[test]
fn fallback_bridge_uses_one_shared_event_stream() {
    let bridge = client_bridge_js();

    assert!(
        bridge.contains("let fallbackEventSource")
            && bridge.contains("const fallbackEventHandlers = new Map()"),
        "fallback bridge must share one EventSource across all frontend listeners"
    );
    assert!(
        !bridge.contains("/events?name="),
        "fallback bridge must not open one filtered SSE connection per listener"
    );
}

#[test]
fn fallback_event_hub_receives_app_level_game_and_error_events() {
    let app = app_rs();
    let compact_app = compact_source(&app);

    assert!(
        app.contains("fn emit_client_event"),
        "app-level events must have a helper that mirrors Tauri emits into ClientEventHub"
    );
    assert!(
        compact_app
            .matches("emit_client_event(&app,\"game-detection\"")
            .count()
            >= 2,
        "game detection lifecycle events must reach fallback SSE clients"
    );
    assert!(
        compact_app
            .matches("emit_client_event(&app,\"error\"")
            .count()
            + compact_app
                .matches("emit_client_event(app.handle(),\"error\"")
                .count()
            >= 3,
        "app-level errors must reach fallback SSE clients"
    );
}

#[test]
fn fallback_browser_mode_uses_browser_chrome_instead_of_fake_titlebar() {
    let bridge = client_bridge_js();
    let styles = styles_css();

    assert!(
        bridge.contains(r#"document.documentElement.dataset.hostMode = "tauri""#),
        "Tauri bridge mode must mark the DOM as native-window hosted"
    );
    assert!(
        bridge.contains(r#"document.documentElement.dataset.hostMode = "fallback""#),
        "fallback bridge mode must mark the DOM as browser hosted"
    );
    assert!(
        styles.contains(r#"html[data-host-mode="fallback"]"#)
            && styles.contains("--titlebar-h: 0px")
            && styles.contains(r#"html[data-host-mode="fallback"] .titlebar"#)
            && styles.contains("display: none"),
        "fallback browser mode must hide the fake frameless titlebar and let browser chrome own window controls"
    );
}

#[test]
fn fallback_manifest_covers_every_frontend_command() {
    let js = main_js();
    let manifest = fallback_manifest_rs();
    let commands = quoted_calls(&js, "invoke");

    assert_eq!(commands.len(), 41, "main.js command inventory changed; update this assertion and the fallback manifest together");
    for command in commands {
        assert!(
            manifest.contains(&format!("\"{command}\"")),
            "fallback manifest must register frontend command {command}"
        );
    }
}

#[test]
fn fallback_manifest_covers_every_frontend_event_listener() {
    let js = main_js();
    let manifest = fallback_manifest_rs();
    let events = quoted_calls(&js, "listen");

    assert_eq!(
        events.len(),
        8,
        "main.js event inventory changed; update this assertion and the fallback manifest together"
    );
    for event in events {
        assert!(
            manifest.contains(&format!("\"{event}\"")),
            "fallback manifest must register frontend event {event}"
        );
    }
}

#[test]
fn every_fallback_manifest_command_has_dispatch_branch() {
    let manifest = fallback_manifest_rs();
    let server =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fallback/server.rs"))
            .expect("read fallback server");

    let commands = quoted_manifest_array(&manifest, "FALLBACK_COMMANDS");
    assert_eq!(
        commands.len(),
        41,
        "fallback command manifest inventory changed"
    );

    let predicate = source_between(
        &server,
        "pub fn fallback_dispatches_command(command: &str) -> bool {",
        "async fn invoke(",
    );
    let predicate_commands = quoted_strings(predicate);
    let invoke = source_between(&server, "async fn invoke(", "async fn events(");
    let match_arms = source_between(invoke, "match command.as_str() {", "        _ => {}");
    let match_commands = quoted_match_arm_values(match_arms);

    for command in commands {
        assert!(
            predicate_commands.contains(&command),
            "fallback dispatch predicate must mention manifest command {command}"
        );
        assert!(
            match_commands.contains(&command),
            "fallback invoke match must branch manifest command {command}"
        );
    }
}

#[test]
fn app_exposes_force_fallback_client_flag() {
    let startup =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fallback/startup.rs"))
            .expect("read fallback startup");

    assert!(
        startup.contains("--force-fallback-client"),
        "fallback implementation must expose a debug flag for forced fallback runtime testing"
    );
}

#[test]
fn fallback_external_validation_script_captures_webview2_removed_gate() {
    let script = fallback_validation_script();

    for required in [
        "param(",
        "$CliplineExe",
        "$EvidencePath",
        "$UseDebugMissingPreflight",
        "$IncludeGlobalHotkeyProbe",
        "--fallback-port",
        "--debug-webview2-preflight",
        "missing",
        "Clipline fallback client:",
        "startup fallback server started",
        "native save hotkey ready",
        "fallback native save hotkey available",
        "fallback unfocused native save hotkey",
        "fallback frontend_ready received",
        "fallback browser frontend_ready",
        "setup start launched_by_autostart=",
        "webviews=[]",
        "normal launch opening main window",
        "open_main_window start",
        "Assert-TextBefore",
        "Assert-TextNotContains",
        "__CLIPLINE_FALLBACK__",
        "client-bridge.js",
        "/invoke/get_settings",
        "/invoke/list_clips",
        "/invoke/storage_status",
        "/invoke/list_game_plugins",
        "/invoke/memory_status",
        "/media-path?path=",
        "MaximumRedirection",
        "Range = \"bytes=0-0\"",
        "fallback media playback smoke",
        "skipped: no clips available",
        "/events",
        "text/event-stream",
        ": heartbeat",
        "fallback event stream smoke",
        "Convert-HotkeyToProbeInput",
        "Send-GlobalHotkeyProbe -Hotkey",
        "clipline-hotkey-probe",
        "Send-GlobalHotkeyProbe",
        "SetForegroundWindow",
        "Wait-ForegroundWindow",
        "GetForegroundWindow",
        "foreground_window_handle",
        "Stop-Process -Id $focusedProcess.Id",
        "notepad.exe",
        "native save hotkey triggered",
        "Invoke-RestMethod",
        "FileShare]::ReadWrite",
        "Remove-Item -LiteralPath $diagnosticLogPath",
        "ConvertTo-Json",
        "Write-Error -ErrorAction Continue",
        "Stop-Process",
    ] {
        assert!(
            script.contains(required),
            "fallback validation script must include {required}"
        );
    }
}

#[test]
fn native_save_hook_reports_real_keyboard_hook_readiness() {
    let app = app_rs();
    let hotkeys = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hotkeys.rs"))
        .expect("read hotkeys");
    let install_hook = source_between(
        &hotkeys,
        "pub fn install_save_hook",
        "pub fn set_save_hotkey",
    );
    let keyboard_hook = source_between(&hotkeys, "fn run_keyboard_hook", "fn run_mouse_hook");

    assert!(
        install_hook.contains("ready_rx")
            && install_hook.contains("recv_timeout")
            && install_hook.contains("run_keyboard_hook(ready_tx)"),
        "install_save_hook must wait until the low-level keyboard hook reports OS readiness"
    );
    let publish_index = install_hook
        .find("publish_save_hook(state.clone())")
        .expect("install_save_hook publishes hook state before installing OS hook");
    let keyboard_start_index = install_hook
        .find("run_keyboard_hook(ready_tx)")
        .expect("install_save_hook starts keyboard hook thread");
    assert!(
        publish_index < keyboard_start_index && install_hook.contains("clear_save_hook(&state)"),
        "hook state must be visible before OS callbacks can fire and rolled back on install failure"
    );
    assert!(
        keyboard_hook.contains("SetWindowsHookExW")
            && keyboard_hook.contains("ready_tx.send(Ok(")
            && keyboard_hook.contains("ready_tx.send(Err("),
        "keyboard hook thread must report SetWindowsHookExW success or failure"
    );
    assert!(
        app.contains("native save hotkey ready hotkey={}"),
        "app setup must only log native save hotkey readiness after the OS hook is installed"
    );
}

#[test]
fn native_save_hotkey_trigger_is_diagnostic_logged() {
    let app = app_rs();

    assert!(
        app.contains("let accepted = app.state::<RuntimeState>().request_save();")
            && app.contains("native save hotkey triggered accepted={accepted}"),
        "native hotkey callback must log when an unfocused/global keypress reaches the recorder"
    );
}

#[test]
fn fallback_frontend_ready_records_browser_boot_diagnostic() {
    let app = app_rs();
    let server =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fallback/server.rs"))
            .expect("read fallback server");
    let invoke = source_between(&server, "async fn invoke(", "async fn events(");
    let frontend_ready_branch = source_between(
        invoke,
        "\"frontend_ready\" =>",
        "        \"save_replay\" =>",
    );

    assert!(
        app.contains("pub(crate) fn log_diagnostic("),
        "fallback server must be able to write browser readiness into the shared diagnostic log"
    );
    assert!(
        frontend_ready_branch.contains("crate::app::log_diagnostic(\"fallback frontend_ready received\")"),
        "fallback frontend_ready must prove that the auto-opened browser ran the shared UI JavaScript"
    );
}

#[test]
fn app_setup_selects_fallback_before_opening_webview() {
    let app = app_rs();
    let run_start = app
        .find("pub fn run()")
        .expect("app.rs exposes run entrypoint");
    let run = &app[run_start..];
    let preflight = run
        .find("webview_preflight = webview2_runtime_preflight")
        .expect("run computes WebView2 runtime preflight before fallback selection");
    let diagnostic = run
        .find("log_diagnostic(webview2_runtime_diagnostic(&webview2_runtime_versions))")
        .expect("run logs real WebView2 registry diagnostics before applying debug overrides");
    let preflight_override = run
        .find("debug_webview2_preflight_override(&args)")
        .expect("run applies debug WebView2 preflight override before fallback selection");
    let preference_call = source_between(
        run,
        "let startup_fallback_requested =",
        "== crate::fallback::startup::FallbackLaunchPreference::StartFallback",
    );
    let preference = run
        .find("let startup_fallback_requested =")
        .expect("run computes startup fallback selection");
    let setup_start = app
        .find(".setup(move |app|")
        .expect("app.rs configures setup");
    let setup = &app[setup_start..];
    let fallback_launch = setup
        .find("start_fallback_from_setup")
        .expect("setup starts forced fallback when requested");
    let normal_launch = setup
        .find("if !launched_by_autostart")
        .expect("setup handles normal non-autostart launch");
    let open_main = setup
        .find("open_main_window(app.handle())")
        .expect("setup opens the main window for normal launches");

    assert!(
        preference_call.contains("fallback_launch_preference(&args, webview_preflight)")
            && !preference_call.contains("WebviewPreflight::Available"),
        "run must pass the computed WebView2 preflight to fallback selection"
    );
    assert!(
        preflight < preference,
        "startup fallback selection must use registry preflight instead of hard-coding WebView2 availability"
    );
    assert!(
        preflight < diagnostic && diagnostic < preflight_override && preflight_override < preference,
        "debug WebView2 preflight override must apply after real registry logging and before startup fallback selection"
    );
    assert!(
        fallback_launch < normal_launch && normal_launch < open_main,
        "setup must choose startup fallback before opening the WebView for normal launches"
    );
}

#[test]
fn fallback_attached_host_delegates_recorder_and_settings_to_tauri_state() {
    let runtime = fallback_runtime_rs();

    let settings_method = source_between(
        &runtime,
        "pub fn settings(&self) -> crate::settings::AppSettings",
        "pub fn events(&self)",
    );
    assert!(
        settings_method.contains("app.state::<crate::app::RuntimeState>()")
            && settings_method.contains("crate::app::host_get_settings"),
        "attached fallback settings reads must use Tauri RuntimeState to avoid update-channel drift"
    );

    let save_settings_method = source_between(
        &runtime,
        "pub fn save_settings(",
        "pub fn list_displays(&self)",
    );
    assert!(
        save_settings_method.contains("crate::app::host_save_settings(&app, settings)")
            && save_settings_method.contains("*guard = committed.clone()"),
        "attached fallback save_settings must delegate to the shared Tauri settings save path and sync its cache"
    );

    let report_method = source_between(
        &runtime,
        "pub fn report_decode_support(&self, codecs: &[String])",
        "pub fn start_microphone_test(",
    );
    assert!(
        report_method.contains("crate::app::host_report_decode_support")
            && report_method.contains("app.state::<crate::app::RuntimeState>()"),
        "attached fallback decode support must update the Tauri RuntimeState recorder configuration"
    );

    let save_replay_method = source_between(
        &runtime,
        "pub fn save_replay(&self) -> bool",
        "pub fn recording_active(&self)",
    );
    assert!(
        save_replay_method.contains("crate::app::host_save_replay")
            && save_replay_method.contains("app.state::<crate::app::RuntimeState>()"),
        "attached fallback save_replay must request save on the Tauri recorder"
    );

    let set_recording_method = source_between(
        &runtime,
        "pub fn set_recording(&self, recording: bool)",
        "#[cfg(test)]",
    );
    assert!(
        set_recording_method.contains("crate::app::host_set_recording")
            && set_recording_method.contains("app.state::<crate::app::RuntimeState>()"),
        "attached fallback set_recording must control the Tauri recorder instead of starting a local service"
    );
}

#[test]
fn fallback_install_update_uses_tauri_helper_without_local_pre_stop() {
    let runtime = fallback_runtime_rs();
    let install_method = source_between(
        &runtime,
        "pub async fn install_update(&self) -> Result<(), String>",
        "pub fn save_settings(",
    );

    assert!(
        install_method.contains("crate::app::host_update_for_install(&app, &state).await"),
        "fallback install_update must prove an update exists through the shared Tauri helper"
    );
    assert!(
        !install_method.contains("set_recording(false)"),
        "fallback install_update must not stop fallback-local recording before an update is found"
    );
    assert!(
        install_method.find("host_update_for_install")
            < install_method.find("self.stop_microphone_test()")
            && install_method.find("self.stop_microphone_test()")
                < install_method.find("host_install_available_update"),
        "fallback install_update must stop fallback-local mic tests after update availability is proven and before installer launch"
    );
}

#[test]
fn fallback_autostart_status_uses_tauri_plugin_when_attached() {
    let app = app_rs();
    let runtime = fallback_runtime_rs();
    let server = fallback_server_rs();
    let status_method = source_between(
        &runtime,
        "pub fn get_autostart_status(&self) -> Result<bool, String>",
        "pub fn list_displays(&self)",
    );
    let branch = source_between(
        &server,
        "\"get_autostart_status\" => {",
        "\"check_for_updates\" => {",
    );

    assert!(
        app.contains("pub(crate) fn host_get_autostart_status"),
        "WebView autostart status helper must be reusable by fallback"
    );
    assert!(
        status_method.contains("crate::app::host_get_autostart_status(&app)"),
        "attached fallback autostart status must query the Tauri autolaunch plugin"
    );
    assert!(
        branch.contains("state.host.get_autostart_status()"),
        "fallback get_autostart_status dispatch must use host parity logic"
    );
}

#[test]
fn fallback_cloud_links_use_shared_cloud_url_validation() {
    let cloud = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cloud.rs"))
        .expect("read src/cloud.rs");
    let server = fallback_server_rs();
    let branch = source_between(
        &server,
        "\"open_cloud_clip_url\" => {",
        "\"report_decode_support\" => {",
    );

    assert!(
        cloud.contains("pub(crate) fn validate_cloud_link_url"),
        "cloud clip URL parser must be reusable by fallback and WebView commands"
    );
    assert!(
        branch.contains("crate::cloud::host_open_cloud_clip_url(args.url)"),
        "fallback open_cloud_clip_url dispatch must route through the shared host helper"
    );

    let host_helper = source_between(
        &cloud,
        "pub(crate) fn host_open_cloud_clip_url(url: String) -> Result<(), String> {",
        "fn open_cloud_clip_url_for_host(url: String) -> Result<(), String> {",
    );
    assert!(
        host_helper.contains("open_cloud_clip_url_for_host(url)"),
        "fallback host helper must delegate to the shared cloud clip opener"
    );

    let opener = source_between(
        &cloud,
        "fn open_cloud_clip_url_for_host(url: String) -> Result<(), String> {",
        "pub(crate) fn open_cloud_url_for_host(url: &str, context: &str) -> Result<(), String> {",
    );
    let validate = opener
        .find("let url = validate_cloud_link_url(&url)?")
        .expect("cloud clip opener validates URL first");
    let open = opener
        .find("crate::host::native::open_external_url(&url, \"cloud clip URL\")")
        .expect("cloud clip opener uses shared native URL opener");
    assert!(
        validate < open,
        "cloud clip opener must validate with the shared parser before opening"
    );
}

#[test]
fn fallback_setup_keeps_server_alive_when_browser_launch_fails() {
    let app = app_rs();
    let setup_fallback = source_between(
        &app,
        "fn start_fallback_from_setup(",
        "fn launch_fallback_client(",
    );

    assert!(
        setup_fallback
            .contains(r#"launch_fallback_client(host, port, "startup fallback", false)?"#),
        "default-browser launch failure should be logged without tearing down the fallback server"
    );
    assert!(
        app.contains("fn show_fallback_browser_launch_notice")
            && app.contains("fallback_browser_launch_notice_message")
            && app.contains(r#"show_fallback_browser_launch_notice(&info.base_url, &e)"#),
        "startup fallback browser launch failure must show the user the fallback URL instead of only logging it"
    );

    let launch = source_between(
        &app,
        "fn launch_fallback_client(",
        "fn open_url_in_default_browser(",
    );
    let show_notice = launch
        .find(r#"show_fallback_browser_launch_notice(&info.base_url, &e)"#)
        .expect("fallback launch failure shows URL notice");
    let error_return = launch
        .find("if open_failure_is_error")
        .expect("fallback launch can treat URL open failure as error");
    assert!(
        show_notice < error_return,
        "dead-WebView fallback launch failures should show the fallback URL before returning an error"
    );
}

#[test]
fn fallback_server_serves_shared_ui_assets() {
    let server =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fallback/server.rs"))
            .expect("read fallback server");

    assert!(
        server.contains("index.html"),
        "fallback server must serve the shared index.html"
    );
    assert!(
        server.contains(".route(\"/{token}\", get(index))"),
        "fallback server base URL must serve the shared index without a trailing slash"
    );
    assert!(
        server.contains("client-bridge.js"),
        "fallback server must serve the shared client bridge"
    );
    assert!(
        server.contains("__CLIPLINE_FALLBACK__"),
        "fallback index must inject fallback bridge config"
    );
    assert!(
        server.contains("/{token}/ui/{*asset}"),
        "fallback server must serve nested UI assets"
    );
    assert!(
        server.contains("std::path::Path::new(asset)")
            && server.contains(".components()")
            && server.contains("Component::Prefix(_)")
            && server.contains("Component::RootDir")
            && server.contains("Component::ParentDir"),
        "fallback UI asset paths must reject Windows drive prefixes, absolute roots, and parent traversal"
    );
}

fn library_rs() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/library.rs");
    fs::read_to_string(path).expect("read src/library.rs")
}

fn tag_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=\"");
    let start = tag.find(&prefix)? + prefix.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

#[test]
fn audio_preview_command_scopes_generated_preview_files() {
    let library = library_rs();

    assert!(
        library.contains("AppHandle")
            && library.contains("allow_audio_preview_asset")
            && library.contains("asset_protocol_scope")
            && library.contains("allow_file(preview"),
        "selected-audio preview MP4s are generated under AppData and must be exact-scoped before the player loads them"
    );
}

#[test]
fn review_player_owns_all_controls() {
    let html = index_html();

    let video_start = html.find("<video").expect("video element exists");
    let video_end = html[video_start..]
        .find('>')
        .map(|offset| video_start + offset)
        .expect("video element closes");
    assert!(
        !html[video_start..=video_end].contains("controls"),
        "the review player must not expose native browser video controls"
    );

    for required in [
        "id=\"play-toggle\"",
        "id=\"seek-back\"",
        "id=\"seek-forward\"",
        "id=\"prev-marker\"",
        "id=\"next-marker\"",
        "id=\"marker-count\"",
        "id=\"timeline\"",
        "id=\"trim-band\"",
        "id=\"handle-in\"",
        "id=\"handle-out\"",
        "id=\"time-readout\"",
        "id=\"rate-select\"",
        "id=\"mute-toggle\"",
        "id=\"volume-slider\"",
        "id=\"export-clip\"",
        "id=\"trim-summary\"",
        "id=\"keys-dialog\"",
        "id=\"keys-close\"",
        "id=\"delete-clip\"",
        "id=\"ruler\"",
        "id=\"overview\"",
        "id=\"overview-trim\"",
        "id=\"overview-markers\"",
        "id=\"overview-playhead\"",
        "id=\"overview-window\"",
        "id=\"overview-window-l\"",
        "id=\"overview-window-r\"",
        "id=\"zoom-out\"",
        "id=\"zoom-fit\"",
        "id=\"zoom-in\"",
        "id=\"snap-toggle\"",
        "id=\"audio-track-panel\"",
        "id=\"audio-track-summary\"",
        "id=\"audio-track-list\"",
        "id=\"open-folder\"",
        "id=\"stage-frame\"",
        "id=\"copy-clip\"",
        "id=\"stage-overlay\"",
        "id=\"memory-usage\"",
        "id=\"rail-hotkey\"",
        "id=\"rail-dot\"",
        "id=\"rail-status-text\"",
        "id=\"rail-status\"",
        "id=\"rail-status\" title=\"Stop recording\" aria-pressed=\"true\"",
        "id=\"rail-save\"",
        "id=\"rail-library-status\"",
        "id=\"rail-clips-count\"",
        "id=\"rail-profile\"",
        "id=\"rail-profile-avatar\"",
        "id=\"rail-profile-name\"",
        "id=\"rail-settings\"",
        "id=\"confirm-dialog\"",
        "id=\"confirm-accept\"",
        "id=\"confirm-cancel\"",
        "id=\"quit-dialog\"",
        "id=\"quit-accept\"",
        "id=\"quit-cancel\"",
        "id=\"update-dialog\"",
        "id=\"update-install\"",
        "id=\"update-cancel\"",
        "id=\"update-dialog-title\"",
        "id=\"update-dialog-body\"",
        "id=\"settings-page\"",
        "id=\"settings-tabs\"",
        "id=\"set-open-on-startup\"",
        "id=\"set-close-to-tray\"",
        "id=\"set-minimize-to-tray\"",
        "id=\"set-update-channel\"",
        "id=\"check-updates\"",
        "id=\"update-status\"",
        "id=\"set-capture\"",
        "id=\"set-backend\"",
        "id=\"backend-summary\"",
        "id=\"set-output-enabled\"",
        "id=\"set-audio-split-output\"",
        "id=\"set-output-device\"",
        "id=\"set-output-volume\"",
        "id=\"output-volume-summary\"",
        "id=\"set-mic-enabled\"",
        "id=\"set-mic-device\"",
        "id=\"set-mic-volume\"",
        "id=\"mic-volume-summary\"",
        "id=\"set-mic-mono\"",
        "id=\"test-mic\"",
        "id=\"mic-meter-fill\"",
        "id=\"mic-test-status\"",
        "id=\"capture-region-editor\"",
        "id=\"display-map\"",
        "id=\"display-map-inner\"",
        "id=\"region-box\"",
        "id=\"region-display-label\"",
        "id=\"set-region-width\"",
        "id=\"set-region-height\"",
        "id=\"set-region-x\"",
        "id=\"set-region-y\"",
        "id=\"capture-region-menu\"",
        "id=\"region-align-menu\"",
        "id=\"region-display-menu\"",
        "id=\"clip-context-menu\"",
        "id=\"clip-menu-play\"",
        "id=\"clip-menu-open-cloud-page\"",
        "id=\"clip-menu-copy-cloud-link\"",
        "id=\"clip-menu-upload\"",
        "id=\"clip-menu-rename\"",
        "id=\"clip-menu-delete\"",
        "id=\"clip-title-display\"",
        "id=\"rename-clip\"",
        "id=\"clip-title-edit\"",
        "id=\"rename-input\"",
        "id=\"rename-save\"",
        "id=\"rename-cancel\"",
        "id=\"upload-dialog\"",
        "id=\"upload-title\"",
        "id=\"upload-description\"",
        "id=\"upload-visibility\"",
        "id=\"upload-audio-section\"",
        "id=\"upload-audio-list\"",
        "id=\"upload-confirm\"",
        "id=\"upload-cancel\"",
        "id=\"upload-dialog-status\"",
        "id=\"set-buffer\"",
        "id=\"set-encoder\"",
        "id=\"encoder-summary\"",
        "id=\"set-output-resolution\"",
        "id=\"output-resolution-summary\"",
        "id=\"set-replay\"",
        "id=\"replay-summary\"",
        "id=\"replay-scale\"",
        "id=\"set-bitrate\"",
        "id=\"quality-summary\"",
        "id=\"quality-scale\"",
        "id=\"set-fps\"",
        "id=\"fps-summary\"",
        "id=\"fps-scale\"",
        "id=\"set-media-dir\"",
        "id=\"choose-media-folder\"",
        "id=\"set-quota\"",
        "id=\"set-replay-disk-enabled\"",
        "id=\"replay-disk-fields\"",
        "id=\"set-replay-disk-dir\"",
        "id=\"choose-replay-cache-folder\"",
        "id=\"set-replay-disk-quota\"",
        "id=\"replay-disk-estimate\"",
        "id=\"set-replay-disk-ack\"",
        "data-tab=\"cloud\"",
        "data-section=\"cloud\"",
        "id=\"cloud-connect-fields\"",
        "id=\"cloud-host-url\"",
        "id=\"cloud-username\"",
        "id=\"cloud-password\"",
        "id=\"cloud-http-warning\"",
        "id=\"cloud-connect\"",
        "id=\"cloud-disconnect\"",
        "id=\"cloud-connect-status\"",
        "id=\"cloud-connection-status\"",
        "id=\"cloud-default-visibility\"",
        "id=\"cloud-delete-local-after-upload\"",
        "id=\"cloud-auto-upload-rules\"",
        "data-tab=\"games\"",
        "data-section=\"games\"",
        "id=\"set-games-auto-detect\"",
        "id=\"supported-games\"",
        "id=\"custom-games\"",
        "id=\"add-custom-game\"",
        "id=\"game-window-picker\"",
        "id=\"refresh-game-windows\"",
        "id=\"game-window-list\"",
        "id=\"game-detection-status\"",
        "id=\"set-hotkey\"",
        "id=\"settings-save\"",
        "id=\"settings-close\"",
    ] {
        assert!(
            html.contains(required),
            "review player is missing required control {required}"
        );
    }

    assert!(
        html.contains("value=\"display_region\""),
        "capture target must expose the display_region mode"
    );
    assert!(
        html.contains("Experimental")
            && html.contains("set-audio-split-output")
            && main_js().contains("split_output_by_process")
            && main_js().contains("split_output_by_process: false"),
        "capture settings must expose and persist the experimental audio-splitting toggle"
    );
    assert!(
        html.contains("Close to Tray")
            && html.contains("Minimize to Tray")
            && html.contains("Updates")
            && html.contains("value=\"stable\" disabled")
            && main_js().contains("close_to_tray")
            && main_js().contains("minimize_to_tray")
            && main_js().contains("update_channel")
            && main_js().contains("check_for_updates")
            && main_js().contains("install_update")
            && main_js().contains("function updateUpToDateStatus(update)")
            && main_js().contains("update.current_version")
            && main_js().contains("update.status || updateUpToDateStatus(update)")
            && main_js().contains("checkForUpdates({ manual: false })")
            && app_rs().contains("tauri_plugin_updater::Builder::new().build()")
            && main_js().contains("minimize_main_window"),
        "general settings must expose and persist tray close/minimize/preview/update behavior"
    );
    assert!(
        main_js().contains("requestWindowClose")
            && main_js().contains("confirmQuit")
            && main_js().contains("close_to_tray === false")
            && styles_css().contains("#quit-dialog"),
        "the window close button must confirm before quitting when Close to Tray is disabled"
    );
    assert!(
        !html.contains(">primary monitor<")
            && main_js().contains("renderCaptureTargetSelect")
            && main_js().contains("displayCaptureValue")
            && main_js().contains("display:")
            && html.contains(">SET REGION<")
            && main_js().find("displayCaptureValue").unwrap()
                < main_js().find("region.value = \"display_region\"").unwrap(),
        "capture target must list available displays before the display-region option"
    );
    assert!(
        !html.contains("value=\"window_title\"") && !html.contains("id=\"set-window\""),
        "manual window-title capture was replaced by custom game detection"
    );
    assert!(
        html.contains("data-replay-preset=\"30\"")
            && html.contains("data-replay-preset=\"60\"")
            && html.contains("data-replay-preset=\"120\""),
        "recording tab must expose quick save-length presets up to two minutes"
    );
    assert!(
        !html.contains("data-replay-preset=\"300\""),
        "save length must not expose presets beyond two minutes"
    );
    let replay_start = html
        .find("id=\"set-replay\"")
        .expect("replay control exists");
    let replay_tag_end = html[replay_start..]
        .find('>')
        .map(|offset| replay_start + offset)
        .expect("replay control tag closes");
    assert!(
        html[replay_start..=replay_tag_end].contains("max=\"120\""),
        "save length slider must stop at two minutes"
    );
    let fps_start = html.find("id=\"set-fps\"").expect("fps control exists");
    let fps_tag_end = html[fps_start..]
        .find('>')
        .map(|offset| fps_start + offset)
        .expect("fps control tag closes");
    assert!(
        html[fps_start..=fps_tag_end].contains("type=\"range\""),
        "smoothness must be a slider, not a dropdown"
    );
    assert!(
        html.contains("id=\"hotkey-status\""),
        "hotkeys page must expose recorder status text"
    );
    let hotkey_start = html.find("id=\"set-hotkey\"").expect("hotkey input exists");
    let hotkey_tag_end = html[hotkey_start..]
        .find('>')
        .map(|offset| hotkey_start + offset)
        .expect("hotkey input tag closes");
    assert!(
        html[hotkey_start..=hotkey_tag_end].contains("readonly"),
        "hotkey input must record shortcuts instead of accepting free text"
    );
    let media_dir_start = html
        .find("id=\"set-media-dir\"")
        .expect("media folder input exists");
    let media_dir_tag_end = html[media_dir_start..]
        .find('>')
        .map(|offset| media_dir_start + offset)
        .expect("media folder input tag closes");
    assert!(
        html[media_dir_start..=media_dir_tag_end].contains("readonly"),
        "media folder should be chosen with the native folder picker"
    );
    assert!(
        html.contains("Choose Folder"),
        "storage settings must expose a native-folder-picker action"
    );
    assert!(
        html.contains("Disk replay buffer (advanced)")
            && html.contains("Only turn this on if you know what you're doing")
            && html.contains("can add significant SSD wear")
            && html.contains(
                "I understand this continuously writes to disk and can shorten SSD life."
            ),
        "disk replay buffer settings must carry explicit advanced SSD-wear warnings"
    );
    assert!(
        html.contains(">Cloud<")
            && html.contains("This host uses HTTP. Your password will be sent without TLS.")
            && !html.contains("id=\"cloud-http-confirm\"")
            && main_js().contains("cloud_connect")
            && main_js().contains("cloud_disconnect")
            && main_js().contains("function cloudHostUsesInsecureHttp()")
            && main_js().contains("plain_http_confirmed: cloudHostUsesInsecureHttp()")
            && main_js().contains(
                "$(\"cloud-host-url\").addEventListener(\"input\", syncCloudHttpWarning)"
            )
            && main_js().contains(
                "$(\"cloud-host-url\").value = connected ? \"\" : cloud.host_url || \"\""
            )
            && main_js().contains(
                "$(\"cloud-username\").value = connected ? \"\" : cloud.connected_username || \"\""
            )
            && main_js().contains("$(\"cloud-connect-fields\").hidden = connected")
            && main_js().contains("$(\"cloud-connect\").hidden = connected")
            && main_js().contains("$(\"cloud-disconnect\").hidden = !connected")
            && main_js().contains("upload_clip_to_cloud")
            && main_js().contains("function openUploadDialog(clip)")
            && main_js().contains("title: request.title || clipUploadDefaultTitle(clip)")
            && main_js().contains("visibility: request.visibility || cloudSettings().default_visibility || \"private\"")
            && html.contains("id=\"upload-dialog\"")
            && html.contains("id=\"upload-title\"")
            && html.contains("id=\"upload-description\"")
            && html.contains("maxlength=\"5000\"")
            && !html.contains("Not supported by Clipline Cloud yet")
            && !main_js().contains("Descriptions are not supported by Clipline Cloud yet.")
            && main_js().contains("description: request.description || null")
            && html.contains("id=\"upload-visibility\"")
            && html.contains("id=\"upload-audio-section\"")
            && html.contains("id=\"upload-audio-list\"")
            && main_js().contains("function clipAudioTracks(clip = currentClip)")
            && main_js().contains("function renderAudioTrackPanel()")
            && main_js().contains("function applySelectedAudioTracksToPlayback({ forceResume = false } = {})")
            && main_js().contains("preview_clip_audio_tracks")
            && main_js().contains("function renderUploadAudioTracks(clip = uploadDialogClip)")
            && main_js().contains("audioTrackIds: request.audioTrackIds || null")
            && !main_js().contains("video.audioTracks")
            && !main_js().contains("applyNativeAudioTrackSelection")
            && main_js().contains("audio-track-label")
            && styles_css().contains(".audio-track-panel")
            && styles_css().contains(".audio-track-row")
            && styles_css().contains(".audio-track-label")
            && styles_css().contains(".upload-audio-section[hidden] { display: none; }")
            && main_js().contains("listen(\"cloud-upload-progress\"")
            && main_js().contains("navigator.clipboard.writeText(record.remote_url)")
            && main_js().contains("syncUploadClipButton();")
            && main_js().contains("Connect Clipline Cloud before uploading.")
            && main_js().contains("function clipCloudVisibility(record)")
            && main_js().contains("CLOUD_VISIBILITY_ICONS")
            && main_js().contains("clip-cloud-visibility")
            && !main_js().contains(" · cloud:")
            && app_rs().contains("crate::cloud::cloud_connect")
            && app_rs().contains("crate::cloud::upload_clip_to_cloud")
            && app_rs().contains("crate::cloud::sync_cloud_clip_status")
            && app_rs().contains("crate::library::preview_clip_audio_tracks")
            && main_js().contains("sync_cloud_clip_status")
            && styles_css().contains(".cloud-connect-grid")
            && styles_css().contains(".cloud-connect-fields")
            && styles_css().contains(".cloud-connect-fields[hidden] { display: none; }")
            && styles_css().contains(".cloud-http-warning")
            && styles_css().contains(".cloud-http-warning[hidden] { display: none; }")
            && styles_css().contains(".clip-cloud-visibility.public")
            && styles_css().contains(".clip-cloud-visibility.unlisted")
            && styles_css().contains(".clip-cloud-visibility.private")
            && styles_css().contains(".clip .clip-title")
            && styles_css().contains(".review-head .clip-title")
            && styles_css().contains("#upload-dialog")
            && html.contains("id=\"upload-clip\"")
            && styles_css().contains(".review-actions .icon-button.uploaded"),
        "cloud settings, upload controls, and per-clip visibility icons must stay wired"
    );
    assert!(
        html.contains(">Games<") && html.contains("Add Custom Game"),
        "settings must expose the Games tab and custom game action"
    );
    assert!(
        main_js().contains("gameRecordingModeControl")
            && main_js().contains("custom-game-recording-mode")
            && main_js().contains("recording_mode")
            && main_js().contains("replays_only")
            && main_js().contains("full_session")
            && styles_css().contains(".custom-game-mode"),
        "custom games must expose and persist per-game recording mode choices"
    );
    assert!(
        main_js().contains("await invoke(\"list_game_plugins\")")
            && main_js().contains("renderGamePlugins")
            && main_js().contains("gamePluginSettings")
            && main_js().contains("games.plugins")
            && main_js().contains("dataset.gamePluginEnabled")
            && main_js().contains("game-plugin-mode-")
            && main_js().contains("normalizeGamePluginId")
            && main_js().contains("Takes priority over matching custom games.")
            && styles_css().contains(".game-profile-mode"),
        "supported games must render from backend game plugins, not hardcoded rows"
    );
    assert!(
        main_js().contains("const leagueMeta = playerSummaryLabel")
            && main_js().contains(
                "const leagueSessionTitle = isLeagueFullSessionClip(c, kind) && leagueMeta"
            )
            && main_js().contains("? leagueMeta")
            && main_js().contains("function clipLibraryTitle(clip, fallbackTitle)")
            && main_js().contains("if (isLeagueClip(clip)) return fallbackTitle")
            && main_js().contains("const clipName = clip && String(clip.name || \"\").trim()")
            && main_js().contains("return clipName || fallbackTitle")
            && main_js().contains("detail.className = \"league-meta\"")
            && main_js().contains("if (leagueMeta && !leagueSessionTitle)")
            && main_js().contains("const infoParts = []")
            && main_js().contains(
                "if (Number.isFinite(c.duration_s)) infoParts.push(fmtDur(c.duration_s))"
            )
            && main_js().contains("if (!leagueMeta && digest) infoParts.push(digest)")
            && styles_css().contains(".clip .league-meta"),
        "League rows must keep their title rules while non-League rows use actual clip names"
    );
    assert!(
        !index_html().contains("game-profile planned"),
        "planned game cards should not sit in static HTML where renderGamePlugins wipes them"
    );

    // Settings is a page in the main pane now, not a sidebar fold.
    assert!(
        !html.contains("settings-fold"),
        "the sidebar settings fold was replaced by #settings-page"
    );
    // Reversed (2026-06-12, PR #5): the footer now carries an explicit Close
    // button after Save, replacing the earlier "close only from the rail" rule.
    let settings_save = html
        .find("id=\"settings-save\"")
        .expect("settings save button");
    let settings_close = html
        .find("id=\"settings-close\"")
        .expect("settings close button");
    assert!(
        settings_save < settings_close,
        "Close must come after Save in the footer markup"
    );

    // Removed on purpose (2026-06-12): clicking the active library row again
    // closes the clip; the new copy affordance must not revive the old path-only id.
    for gone in [
        "id=\"copy-path\"",
        "id=\"close-review\"",
        "id=\"focus-toggle\"",
    ] {
        assert!(
            !html.contains(gone),
            "{gone} was removed from the header — do not reintroduce it"
        );
    }
    let upload_clip = html.find("id=\"upload-clip\"").expect("upload clip button");
    let open_folder = html.find("id=\"open-folder\"").expect("open folder button");
    let copy_clip = html.find("id=\"copy-clip\"").expect("copy clip button");
    let delete_clip = html.find("id=\"delete-clip\"").expect("delete clip button");
    assert!(
        upload_clip < open_folder,
        "upload button must sit immediately left of Open Folder in the review header"
    );
    assert!(
        open_folder < copy_clip && copy_clip < delete_clip,
        "copy clip must sit beside Open Folder before the destructive action"
    );

    // Conventional ordering: transport glued to the stage, timeline below it.
    let transport = html.find("id=\"play-toggle\"").expect("play toggle");
    let timeline = html.find("id=\"timeline\"").expect("timeline");
    assert!(
        transport < timeline,
        "transport row must precede the timeline in the deck"
    );
    assert!(
        styles_css().contains(".stage-frame")
            && styles_css().contains("object-fit: contain")
            && main_js().contains("updateStageFrame"),
        "the review stage must size an aspect-locked frame around the video"
    );
    // Icon buttons carry SVG icons; text labels are a regression.
    for id in [
        "id=\"play-toggle\"",
        "id=\"seek-back\"",
        "id=\"seek-forward\"",
        "id=\"prev-marker\"",
        "id=\"next-marker\"",
        "id=\"mute-toggle\"",
        "id=\"upload-clip\"",
        "id=\"open-folder\"",
        "id=\"copy-clip\"",
        "id=\"rail-save\"",
        "id=\"rail-settings\"",
        "id=\"delete-clip\"",
        "id=\"export-clip\"",
        "id=\"zoom-out\"",
        "id=\"zoom-fit\"",
        "id=\"zoom-in\"",
        "id=\"snap-toggle\"",
    ] {
        let start = html.find(id).expect("transport button exists");
        let body_end = html[start..]
            .find("</button>")
            .map(|o| start + o)
            .expect("button closes");
        assert!(
            html[start..body_end].contains("<svg"),
            "{id} must render an SVG icon, not a text label"
        );
    }
}

#[test]
fn rail_shows_save_hotkey() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    assert!(
        html.contains("id=\"rail-hotkey\""),
        "sidebar rail must expose the current save hotkey"
    );
    assert!(
        js.contains("function updateHotkeyLabels(")
            && js.contains("rail-hotkey")
            && js.contains("Save Replay ("),
        "main.js must keep rail and button hotkey labels in sync"
    );
    assert!(
        css.contains(".rail-hotkey"),
        "rail hotkey needs stable compact styling"
    );
}

#[test]
fn rail_shows_connected_cloud_identity() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    for required in [
        "<button id=\"rail-profile\"",
        "id=\"rail-profile-avatar\"",
        "id=\"rail-profile-name\"",
    ] {
        assert!(
            html.contains(required),
            "rail profile markup must include `{required}`"
        );
    }
    for required in [
        "function syncRailProfile",
        "function refreshRailProfileIdentity",
        "function loadRailProfileAvatar",
        "function openRailProfile",
        "invoke(\"cloud_user_profile\")",
        "invoke(\"cloud_user_avatar\")",
        "invoke(\"open_cloud_user_profile\")",
        "connected_display_name",
        "railProfileAvatarKey",
        "$(\"rail-profile-name\")",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire cloud rail profile behavior through `{required}`"
        );
    }
    for required in [
        ".rail-profile",
        ".rail-profile[hidden]",
        ".rail-profile-avatar",
        ".rail-profile-name",
    ] {
        assert!(
            css.contains(required),
            "rail cloud identity needs stable compact styling for `{required}`"
        );
    }
    assert!(
        app_rs().contains("crate::cloud::cloud_user_avatar"),
        "native command registry must expose cloud_user_avatar for the rail profile"
    );
    assert!(
        app_rs().contains("crate::cloud::cloud_user_profile"),
        "native command registry must expose cloud_user_profile for display-name refresh"
    );
    assert!(
        app_rs().contains("crate::cloud::open_cloud_user_profile"),
        "native command registry must expose open_cloud_user_profile for the rail profile button"
    );
}

#[test]
fn audio_preview_generation_is_not_eager_on_clip_open() {
    let js = main_js();
    let open_clip_start = js.find("function openClip(clip)").unwrap();
    let close_review_start = js.find("function closeReview()").unwrap();
    let open_clip = &js[open_clip_start..close_review_start];

    assert!(
        !open_clip.contains("applySelectedAudioTracksToPlayback()"),
        "opening a clip must not unconditionally remux or mix a full-session audio preview"
    );
    assert!(
        open_clip.contains("applyDefaultAudioSelectionIfNeeded({ shouldResume: true })"),
        "opening a clip should apply default audio only when source playback would not match it"
    );
    assert!(
        open_clip.contains("syncCloudClipStatus(clip);"),
        "opening a clip should refresh its cloud record in the background"
    );
    let source_play = open_clip
        .find("video.play().catch(() => syncPlayState());")
        .expect("openClip should still start direct source playback when no preview is needed");
    let default_preview = open_clip
        .find("applyDefaultAudioSelectionIfNeeded({ shouldResume: true })")
        .expect("openClip should request a resumed default preview when needed");
    assert!(
        default_preview < source_play,
        "preview-needed clips must not audibly play the unmixed source before the preview source is ready"
    );
    assert!(
        js.contains("function applyDefaultAudioSelectionIfNeeded({ shouldResume = false } = {})")
            && js.contains("PlayerCore.selectionNeedsPreview")
            && js.contains("applySelectedAudioTracksToPlayback({ forceResume: shouldResume });"),
        "default audio application must be gated by PlayerCore.selectionNeedsPreview"
    );
    assert!(
        js.contains("selected.length === tracks.length && currentReviewMediaPath === clip.path"),
        "all-track playback should keep the original source until the user changes selection"
    );
    assert!(
        js.contains("function applyCloudClipSyncResult(")
            && js.contains("removeCloudUploadRecordForPath(result.path)")
            && js.contains("upsertCloudUploadRecord(result.record)"),
        "cloud sync results must update or remove the local cloud record cache"
    );
    assert!(
        js.contains("if (forceResume && currentClip && currentClip.path === clip.path) {")
            && js.contains("video.play().catch(() => syncPlayState());"),
        "preview generation failure while opening a clip must fall back to source playback"
    );
    assert!(
        js.contains("function cloudUploadRecordForPath(path)")
            && js.contains("applyCloudClipSyncResult(result, {")
            && js.contains("expectedRecord, expectedLocalClipId, expectedUpdatedAtUnix"),
        "cloud open-sync must capture the record identity it started from"
    );
    assert!(
        js.contains("if (expectedRecord && current !== expectedRecord) return false;")
            && js.contains("current.local_clip_id !== expectedLocalClipId")
            && js.contains(
                "Number(current.updated_at_unix || 0) > Number(expectedUpdatedAtUnix || 0)"
            ),
        "cloud open-sync must ignore stale results once a newer upload record exists"
    );
}

#[test]
fn open_clip_clears_previous_playback_loop_and_pending_seek() {
    let js = main_js();
    let open_clip_start = js.find("function openClip(clip)").unwrap();
    let close_review_start = js.find("function closeReview()").unwrap();
    let open_clip = &js[open_clip_start..close_review_start];
    let cancel = open_clip
        .find("cancelAnimationFrame(rafId);")
        .expect("openClip cancels the previous playhead RAF");
    let clear_seek = open_clip
        .find("pendingSeek = null;")
        .expect("openClip clears pending seek from previous clip");
    let assign_clip = open_clip
        .find("currentClip = clip;")
        .expect("openClip assigns current clip");

    assert!(
        cancel < assign_clip,
        "RAF must be canceled before switching clips"
    );
    assert!(
        clear_seek < assign_clip,
        "pending seek must be cleared before switching clips"
    );
}

#[test]
fn initial_settings_tab_state_matches_visible_section() {
    let html = index_html();
    let tabs_start = html.find("id=\"settings-tabs\"").expect("settings tabs");
    let tabs_end = html[tabs_start..]
        .find("</nav>")
        .map(|offset| tabs_start + offset)
        .expect("settings tabs close");
    let tabs = &html[tabs_start..tabs_end];

    let mut active_tabs = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = tabs[cursor..].find("<button") {
        let start = cursor + offset;
        let end = tabs[start..]
            .find('>')
            .map(|tag_end| start + tag_end)
            .expect("tab button closes");
        let tag = &tabs[start..=end];
        if tag_attr(tag, "class")
            .is_some_and(|class| class.split_whitespace().any(|c| c == "active"))
        {
            active_tabs.push(tag_attr(tag, "data-tab").expect("active tab has data-tab"));
        }
        cursor = end + 1;
    }
    assert_eq!(
        active_tabs.len(),
        1,
        "settings must have exactly one active initial tab"
    );
    let active_tab = active_tabs[0];

    let mut visible_sections = Vec::new();
    cursor = 0;
    while let Some(offset) = html[cursor..].find("<div class=\"settings-section\"") {
        let start = cursor + offset;
        let end = html[start..]
            .find('>')
            .map(|tag_end| start + tag_end)
            .expect("settings section opens");
        let tag = &html[start..=end];
        let section = tag_attr(tag, "data-section").expect("settings section has data-section");
        let hidden = tag
            .split_whitespace()
            .any(|part| part == "hidden" || part == "hidden>");
        if hidden {
            assert_ne!(
                section, active_tab,
                "the initially active settings section must not be hidden"
            );
        } else {
            visible_sections.push(section);
        }
        cursor = end + 1;
    }
    assert_eq!(
        visible_sections,
        vec![active_tab],
        "only the active settings tab's section should be visible before first interaction"
    );
}

#[test]
fn timeline_navigator_and_zoom_controls_are_wired() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    // The whole-clip navigator sits between the ruler and the export row.
    let ruler = html.find("id=\"ruler\"").expect("ruler");
    let overview = html.find("id=\"overview\"").expect("overview");
    let export_row = html.find("class=\"export-row\"").expect("export row");
    assert!(
        ruler < overview && overview < export_row,
        "the navigator minimap must sit between the ruler and the export row"
    );

    // Central view setter + paint/rebuild split keep the navigator in sync, and
    // every view change routes through the pure helpers.
    for required in [
        "function applyView",
        "function paintOverview",
        "function renderOverviewMarkers",
        "function maybeFollow",
        "onOverviewPointerDown",
        "function zoomAtPlayhead",
        "function zoomToSelection",
        "zoomView(",
        "panView(",
        "setViewEdge(",
        "followView(",
        "snapTime(",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire the timeline through {required}"
        );
    }

    // Navigator window, markers, and snap feedback need styles.
    assert!(
        css.contains("#overview-window") && css.contains(".ov-marker") && css.contains(".snapped"),
        "navigator window, marker ticks, and snap feedback must be styled"
    );
    assert!(
        css.contains(".marker-death .glyph.img")
            && css.contains("-webkit-mask: var(--marker-img) center / 190% no-repeat"),
        "death marker art has extra transparent padding and must be scaled to match kill markers"
    );
}

#[test]
fn no_native_browser_dialogs() {
    let js = main_js();
    let css = styles_css();
    // window.confirm/alert render browser chrome ("tauri.localhost says") —
    // use the in-app #confirm-dialog instead.
    for banned in ["confirm(", "alert("] {
        assert!(
            !js.contains(banned),
            "main.js must not call native {banned}…) — use the in-app dialog"
        );
    }

    assert!(
        js.contains("document.addEventListener(\"contextmenu\", (ev) => {")
            && js.contains("ev.preventDefault();")
            && js.contains("showClipContextMenu(ev, c)")
            && js.contains("showCloudClipContextMenu(ev, entry)")
            && js.contains("$(\"clip-menu-play\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-open-cloud-page\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-copy-cloud-link\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-upload\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-rename\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-delete\").addEventListener(\"click\"")
            && js.contains("function beginClipRename")
            && js.contains("await invoke(\"rename_clip\"")
            && app_rs().contains("crate::library::rename_clip")
            && css.contains(".clip-title-edit")
            && css.contains(".context-menu button[hidden]")
            && css.contains(".context-menu button.danger-text"),
        "native context menus must be suppressed and library rows must expose an app-owned clip menu"
    );
}

#[test]
fn controls_have_custom_range_and_scrollbar_skin() {
    let css = styles_css();
    let js = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/main.js")).unwrap();

    assert!(
        css.contains("::-webkit-slider-thumb") && css.contains("::-moz-range-thumb"),
        "range inputs should use Clipline slider styling instead of native defaults"
    );
    assert!(
        css.contains("::-webkit-scrollbar-thumb") && css.contains("scrollbar-color"),
        "scrollable areas should use the app scrollbar styling"
    );
    assert!(
        css.contains("--range-progress") && js.contains("syncRangeProgress"),
        "slider fill must stay synced to the current value"
    );
    assert!(
        css.contains("background-position: right 12px center")
            && css.contains("-webkit-appearance: none"),
        "select arrows should use the app inset instead of the native edge-hugging arrow"
    );
}

#[test]
fn shell_shows_live_memory_usage() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    assert!(
        html.contains("id=\"memory-usage\"") && html.contains("Using -- RAM"),
        "sidebar chrome must include the RAM indicator placeholder"
    );
    assert!(
        js.contains("memory_status") && js.contains("setInterval(refreshMemoryUsage, 2000)"),
        "memory indicator must poll the backend command on a short interval"
    );
    assert!(
        css.contains(".memory-usage") && css.contains("font-variant-numeric: tabular-nums"),
        "memory usage should have stable numeric styling in the top-left chrome"
    );
}

#[test]
fn gallery_header_shows_library_storage_usage() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    assert!(
        html.contains("id=\"gallery-count\"") && html.contains("id=\"gallery-storage-used\""),
        "gallery header should include storage usage beside the clip count"
    );
    assert!(
        js.contains("$(\"gallery-storage-used\").textContent")
            && js.contains("const quotaGb = s.quota_bytes == null")
            && js.contains("fmtLibraryStorageUsage(s.total_bytes, quotaGb)"),
        "refreshStorage should render total library bytes and configured quota from storage_status into the gallery header"
    );
    assert!(
        css.contains(".gallery-storage-used"),
        "storage usage should have gallery header metadata styling"
    );
}

#[test]
fn library_has_cloud_source_tab() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    for required in [
        "id=\"gallery-source-tabs\"",
        "data-gallery-source=\"local\"",
        "data-gallery-source=\"cloud\"",
        "id=\"cloud-gallery-grid\"",
    ] {
        assert!(
            html.contains(required),
            "library markup must include cloud source tab contract `{required}`"
        );
    }
    for required in [
        "let gallerySource = \"local\"",
        "function renderCloudClips()",
        "function cloudLocalClipForEntry(entry)",
        "function openCloudEntryInApp(entry)",
        "function showCloudClipContextMenu(ev, entry)",
        "function observeCloudThumbnail(entry, thumb)",
        "function loadCloudThumbnail(entry, thumb)",
        "const cloudThumbnailInflight = new Map()",
        "posterQueue.set(thumb, { type: \"cloud-thumbnail\", entry })",
        "let cloudClipsCache = []",
        "function loadCloudClips",
        "if (gallerySource === \"cloud\") renderCloudClips();",
        "if (cloudClipsError && !force) return;",
        "error.className = \"gallery-empty cloud-error\"",
        "function isCloudOnlyReviewClip(clip = currentClip)",
        "function syncReviewLocalActions()",
        "invoke(\"list_cloud_clips\")",
        "invoke(\"cache_cloud_clip_media\"",
        "invoke(\"cloud_clip_thumbnail\"",
        "invoke(\"open_cloud_clip_url\"",
        "PlayerCore.cloudLibraryEntries",
        "localClip ? clipCard(localClip) : cloudClipCard(entry)",
        "showCloudClipContextMenu(ev, entry)",
        "$(\"cloud-gallery-grid\")",
        "querySelectorAll(\"#gallery-source-tabs .source-tab\")",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire cloud library behavior through `{required}`"
        );
    }
    assert!(
        !js.contains("actions.className = \"card-actions\""),
        "cloud-only cards should not render inline Play/Open/Copy buttons"
    );
    for required in [
        ".gallery-source-tabs",
        ".source-tab.active",
        ".cloud-gallery-grid",
        ".cloud-card",
        ".cloud-card-placeholder > svg",
        ".gallery-empty.cloud-error",
    ] {
        assert!(
            css.contains(required),
            "cloud library tab should have stable styling for `{required}`"
        );
    }
    assert!(
        app_rs().contains("crate::cloud::list_cloud_clips"),
        "native command registry must expose list_cloud_clips for the Cloud library tab"
    );
    assert!(
        app_rs().contains("crate::cloud::open_cloud_clip_url"),
        "native command registry must expose open_cloud_clip_url for Cloud card links"
    );
}

#[test]
fn games_ui_wires_detection_commands() {
    let js = main_js();

    for required in [
        "await invoke(\"list_game_plugins\")",
        "await invoke(\"list_game_windows\")",
        "listen(\"game-detection\"",
        "renderGamePlugins",
        "renderCustomGames",
        "refreshGameWindows",
        "$(\"add-custom-game\").addEventListener(\"click\", showGameWindowPicker)",
        "$(\"refresh-game-windows\").addEventListener(\"click\", refreshGameWindows)",
        "$(\"cancel-game-picker\").addEventListener(\"click\", hideGameWindowPicker)",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire the custom game workflow through {required}"
        );
    }
}

#[test]
fn deck_status_success_toasts_auto_clear() {
    let js = main_js();

    assert!(
        js.contains("const DECK_STATUS_TOAST_MS")
            && js.contains("let deckStatusToastTimer")
            && js.contains("function setDeckStatus(message, { transient = false } = {})"),
        "deck status messages should flow through a helper that can schedule transient toasts"
    );
    assert!(
        js.contains("window.setTimeout(() => {")
            && js.contains("if ($(\"deck-status\").textContent === message)"),
        "transient deck status toasts should clear themselves without erasing newer messages"
    );

    for required in [
        "setDeckStatus(audioSelectionLabel(clip), { transient: true })",
        "setDeckStatus(\"clip renamed\", { transient: true })",
        "setDeckStatus(`exported ${exported.name} · keyframe-aligned ${fmtTenths(exported.aligned_start_s)} – ${fmtTenths(exported.aligned_end_s)}`, { transient: true })",
        "setDeckStatus(\"clip copied to clipboard\", { transient: true })",
        "setDeckStatus(\"cloud link copied\", { transient: true })",
        "setDeckStatus(\"cloud upload ready\", { transient: true })",
    ] {
        assert!(
            js.contains(required),
            "success toast should auto-clear via `{required}`"
        );
    }

    for required in [
        "setDeckStatus(\"switching audio tracks...\")",
        "setDeckStatus(\"renaming clip...\")",
        "setDeckStatus(\"exporting…\")",
        "setDeckStatus(\"uploading to cloud...\")",
        "setDeckStatus(\"cloud upload processing\")",
    ] {
        assert!(
            js.contains(required),
            "progress status should stay explicit via `{required}`"
        );
    }
}

#[test]
fn clipboard_copy_sends_selected_audio_tracks() {
    let js = main_js();
    let app = app_rs();
    let library = library_rs();

    assert!(
        library.contains("pub struct CopyClipToClipboardRequest")
            && library.contains("request: CopyClipToClipboardRequest"),
        "clipboard command should accept a request object so selected audio can be passed"
    );
    assert!(
        app.contains("crate::library::copy_clip_to_clipboard"),
        "clipboard command should stay registered with Tauri"
    );
    assert!(
        js.contains("await invoke(\"copy_clip_to_clipboard\", {")
            && js.contains("request: {")
            && js.contains("path: currentClip.path")
            && js.contains("audioTrackIds: clipAudioTracks(currentClip).length")
            && js.contains("selectedAudioTrackIdsForClip(currentClip)"),
        "copy should send the current selected audio tracks to the native clipboard exporter"
    );
}

#[test]
fn app_notice_toasts_auto_clear() {
    let js = main_js();

    assert!(
        js.contains("const NOTICE_TOAST_MS")
            && js.contains("let noticeToastTimer")
            && js.contains("function setNotice(message, { transient = false } = {})"),
        "app-wide notices should flow through a helper that can schedule transient toasts"
    );
    assert!(
        js.contains("window.setTimeout(() => {")
            && js.contains("if ($(\"notice\").textContent === message)"),
        "transient app-wide notices should clear themselves without erasing newer messages"
    );

    for required in [
        "setNotice(\"clip renamed\", { transient: true })",
        "setNotice(\"clip deleted\", { transient: true })",
        "setNotice(s.gc_deleted",
        ": `saved ${fmtDur(s.seconds)} ${savedKind}`, { transient: true });",
    ] {
        assert!(
            js.contains(required),
            "app-wide success notice should auto-clear via `{required}`"
        );
    }
}

#[test]
fn ui_is_split_into_markup_styles_and_logic() {
    let html = index_html();

    for asset in [
        "href=\"styles.css\"",
        "src=\"player-core.js\"",
        "src=\"main.js\"",
    ] {
        assert!(html.contains(asset), "index.html must reference {asset}");
    }

    assert!(
        !html.contains("<style"),
        "styles belong in ui/styles.css, not inline in index.html"
    );

    for (i, chunk) in html.split("<script").skip(1).enumerate() {
        let tag_end = chunk.find('>').expect("script tag closes");
        assert!(
            chunk[..tag_end].contains("src="),
            "script tag #{i} must load an external file (logic belongs in player-core.js/main.js)"
        );
        let body_end = chunk.find("</script>").expect("script element closes");
        assert!(
            chunk[tag_end + 1..body_end].trim().is_empty(),
            "script tag #{i} must not have an inline body"
        );
    }
}
