//! Tauri shell: tray, Alt+F10 global hotkey, status webview — all thin
//! wiring around the recorder service thread.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::path::BaseDirectory;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, Runtime, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_updater::UpdaterExt;

use crate::game_discovery::DetectedGameCandidate;
use crate::game_plugins::GamePluginInfo;
use crate::games::{DetectedGame, GameWindowInfo};
use crate::osu_enrichment::OsuTitleEvent;
use crate::service::{self, Cmd, Event, ServiceOptions};
use crate::settings::{
    is_global_shortcut_hotkey, parse_hotkey, quota_bytes_from_gb, AppSettings, CaptureMode,
    CustomGameSettings, GameRecordingMode,
};
use crate::updates::UpdateChannel;

const DIAGNOSTIC_LOG_MAX_BYTES: u64 = 1_048_576;
const MAIN_WINDOW_LABEL: &str = "main";
const WEBVIEW_READY_TIMEOUT: Duration = Duration::from_secs(5);
const GAME_DETECTOR_INTERVAL: Duration = Duration::from_millis(500);
static DIAGNOSTIC_LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static FRONTEND_READY: AtomicBool = AtomicBool::new(false);
static WEBVIEW_READY_WATCHDOG_ARMED: AtomicBool = AtomicBool::new(false);
static WEBVIEW_REPAIR_NOTICE_SHOWN: AtomicBool = AtomicBool::new(false);

#[derive(serde::Serialize)]
struct DisplayInfo {
    id: String,
    name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    is_primary: bool,
}

#[derive(serde::Serialize)]
struct AudioDeviceInfo {
    id: String,
    name: String,
    is_default: bool,
}

#[derive(serde::Serialize)]
struct AudioDeviceLists {
    outputs: Vec<AudioDeviceInfo>,
    inputs: Vec<AudioDeviceInfo>,
}

#[derive(serde::Serialize, Clone)]
struct GameDetectionEvent {
    active: bool,
    name: Option<String>,
    window_title: Option<String>,
    process_id: Option<u32>,
    exe_name: Option<String>,
    recording_mode: Option<GameRecordingMode>,
    elevated_hotkeys_blocked: bool,
}

#[derive(serde::Serialize)]
struct UpdateCheckResult {
    channel: UpdateChannel,
    channel_label: &'static str,
    current_version: String,
    available: bool,
    version: Option<String>,
    date: Option<String>,
    notes: Option<String>,
    endpoint: &'static str,
    status: Option<String>,
}

impl GameDetectionEvent {
    fn from_detected(detected: Option<&DetectedGame>) -> Self {
        Self::from_detected_with_elevation(
            detected,
            crate::windows::current_process_is_elevated(),
            crate::windows::process_is_elevated,
        )
    }

    fn from_detected_with_elevation(
        detected: Option<&DetectedGame>,
        clipline_elevated: Result<bool, String>,
        game_is_elevated: impl FnOnce(u32) -> Result<bool, String>,
    ) -> Self {
        match detected {
            Some(game) => {
                let elevated_hotkeys_blocked = matches!(clipline_elevated, Ok(false))
                    && game_is_elevated(game.process_id).unwrap_or(true);
                Self {
                    active: true,
                    name: Some(game.name.clone()),
                    window_title: Some(game.window_title.clone()),
                    process_id: Some(game.process_id),
                    exe_name: Some(game.exe_name.clone()),
                    recording_mode: Some(game.recording_mode),
                    elevated_hotkeys_blocked,
                }
            }
            None => Self {
                active: false,
                name: None,
                window_title: None,
                process_id: None,
                exe_name: None,
                recording_mode: None,
                elevated_hotkeys_blocked: false,
            },
        }
    }
}

fn diagnostic_log_path_from_appdata(appdata: &Path) -> PathBuf {
    appdata.join("Clipline").join("clipline.log")
}

fn diagnostic_log_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|appdata| diagnostic_log_path_from_appdata(&appdata))
}

fn open_diagnostic_log() -> Result<File, String> {
    let path = diagnostic_log_path().ok_or_else(|| "APPDATA is not set".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create log directory: {e}"))?;
    }
    rotate_diagnostic_log_if_needed(&path)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open diagnostic log {path:?}: {e}"))
}

fn rotate_diagnostic_log_if_needed(path: &Path) -> Result<(), String> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() <= DIAGNOSTIC_LOG_MAX_BYTES {
        return Ok(());
    }

    let rotated = path.with_file_name("clipline.old.log");
    if let Err(e) = std::fs::remove_file(&rotated) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("remove old diagnostic log {rotated:?}: {e}"));
        }
    }
    std::fs::rename(path, &rotated).map_err(|e| format!("rotate diagnostic log: {e}"))
}

fn diagnostic_log() -> &'static Mutex<Option<File>> {
    DIAGNOSTIC_LOG.get_or_init(|| Mutex::new(open_diagnostic_log().ok()))
}

fn format_diagnostic_log_line(
    timestamp: chrono::DateTime<chrono::Utc>,
    pid: u32,
    message: &str,
) -> String {
    let message = message.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{} pid={pid} {message}",
        timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    )
}

fn log_diagnostic(message: impl AsRef<str>) {
    let line = format_diagnostic_log_line(chrono::Utc::now(), std::process::id(), message.as_ref());
    if let Ok(mut log) = diagnostic_log().lock() {
        if let Some(file) = log.as_mut() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

fn configure_bundled_ffmpeg<R: Runtime>(app: &tauri::App<R>) {
    match app
        .path()
        .resolve("ffmpeg/ffmpeg.exe", BaseDirectory::Resource)
    {
        Ok(path) if path.exists() => {
            clipline_capture::ffmpeg::set_bundled_ffmpeg(path.clone());
            log_diagnostic(format!("bundled ffmpeg resource={path:?}"));
        }
        Ok(path) => {
            log_diagnostic(format!("bundled ffmpeg resource missing at {path:?}"));
        }
        Err(e) => {
            log_diagnostic(format!("resolve bundled ffmpeg resource failed: {e}"));
        }
    }
}

fn result_debug<T, E>(result: Result<T, E>) -> String
where
    T: std::fmt::Debug,
    E: std::fmt::Display,
{
    match result {
        Ok(value) => format!("ok({value:?})"),
        Err(e) => format!("err({e})"),
    }
}

fn webview_labels<R: Runtime>(app: &AppHandle<R>) -> String {
    let mut labels = app.webview_windows().into_keys().collect::<Vec<_>>();
    labels.sort();
    format!("[{}]", labels.join(","))
}

fn is_app_window_label(label: &str) -> bool {
    label == MAIN_WINDOW_LABEL
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainWindowOpenTarget {
    ExistingMain,
    NewMain,
}

fn main_window_open_target(main_window_present: bool) -> MainWindowOpenTarget {
    if main_window_present {
        MainWindowOpenTarget::ExistingMain
    } else {
        MainWindowOpenTarget::NewMain
    }
}

fn window_state_summary<R: Runtime>(window: &WebviewWindow<R>) -> String {
    format!(
        "label={} visible={} minimized={} focused={} outer_position={} outer_size={} inner_size={}",
        window.label(),
        result_debug(window.is_visible()),
        result_debug(window.is_minimized()),
        result_debug(window.is_focused()),
        result_debug(window.outer_position()),
        result_debug(window.outer_size()),
        result_debug(window.inner_size())
    )
}

fn log_window_state<R: Runtime>(context: &str, window: &WebviewWindow<R>) {
    log_diagnostic(format!("{context}: {}", window_state_summary(window)));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebviewRepairNoticeReason {
    GetterFailedToReceiveMessage,
    FrontendReadyTimeout,
    OtherGetterError,
}

fn classify_webview_getter_error(error: &tauri::Error) -> WebviewRepairNoticeReason {
    match error {
        tauri::Error::Runtime(tauri_runtime::Error::FailedToReceiveMessage) => {
            WebviewRepairNoticeReason::GetterFailedToReceiveMessage
        }
        _ => WebviewRepairNoticeReason::OtherGetterError,
    }
}

fn should_show_webview_repair_notice(
    reason: WebviewRepairNoticeReason,
    already_shown: bool,
) -> bool {
    !already_shown
        && matches!(
            reason,
            WebviewRepairNoticeReason::GetterFailedToReceiveMessage
                | WebviewRepairNoticeReason::FrontendReadyTimeout
        )
}

fn show_webview_repair_notice_once(reason: WebviewRepairNoticeReason) {
    if !should_show_webview_repair_notice(
        reason,
        WEBVIEW_REPAIR_NOTICE_SHOWN.load(Ordering::Relaxed),
    ) {
        return;
    }
    if WEBVIEW_REPAIR_NOTICE_SHOWN
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    log_diagnostic(format!("webview2 repair notice shown reason={reason:?}"));
    let _ = std::thread::Builder::new()
        .name("clipline-webview2-repair-notice".into())
        .spawn(move || {
            let _ = rfd::MessageDialog::new()
                .set_title("Clipline needs Microsoft WebView2")
                .set_description(
                    "Clipline is running, but the Windows WebView2 runtime did not start. \
Install or repair Microsoft Edge WebView2 Runtime, then reopen Clipline.\n\n\
You can get it from Microsoft: https://developer.microsoft.com/microsoft-edge/webview2/",
                )
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        });
}

fn probe_webview_after_reveal<R: Runtime>(window: &WebviewWindow<R>, context: &str) {
    match window.is_visible() {
        Ok(visible) => log_diagnostic(format!("{context} health probe is_visible=ok({visible})")),
        Err(e) => {
            let reason = classify_webview_getter_error(&e);
            log_diagnostic(format!(
                "{context} health probe is_visible=err({e}) reason={reason:?}"
            ));
            show_webview_repair_notice_once(reason);
        }
    }
}

fn arm_frontend_ready_watchdog() {
    if FRONTEND_READY.load(Ordering::Acquire) {
        return;
    }
    if WEBVIEW_READY_WATCHDOG_ARMED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    log_diagnostic("webview readiness watchdog armed");
    let _ = std::thread::Builder::new()
        .name("clipline-webview-readiness-watchdog".into())
        .spawn(|| {
            std::thread::sleep(WEBVIEW_READY_TIMEOUT);
            if !FRONTEND_READY.load(Ordering::Acquire) {
                log_diagnostic("webview readiness watchdog expired before frontend_ready");
                show_webview_repair_notice_once(WebviewRepairNoticeReason::FrontendReadyTimeout);
            } else {
                log_diagnostic("webview readiness watchdog observed frontend_ready");
            }
        });
}

const WEBVIEW2_RUNTIME_CLIENT_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

fn webview2_runtime_registry_keys() -> [String; 3] {
    [
        format!(
            r"HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_RUNTIME_CLIENT_GUID}"
        ),
        format!(r"HKLM\SOFTWARE\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_RUNTIME_CLIENT_GUID}"),
        format!(r"HKCU\Software\Microsoft\EdgeUpdate\Clients\{WEBVIEW2_RUNTIME_CLIENT_GUID}"),
    ]
}

fn parse_reg_pv_output(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let name = fields.next()?;
        let kind = fields.next()?;
        if !name.eq_ignore_ascii_case("pv") || !kind.eq_ignore_ascii_case("REG_SZ") {
            return None;
        }
        let value = fields.collect::<Vec<_>>().join(" ");
        (!value.is_empty()).then_some(value)
    })
}

fn query_registry_pv(key: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let output = std::process::Command::new("reg.exe")
        .args(["query", key, "/v", "pv"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_reg_pv_output(&String::from_utf8_lossy(&output.stdout))
}

fn webview2_runtime_diagnostic() -> String {
    let entries = webview2_runtime_registry_keys()
        .into_iter()
        .map(|key| {
            let version = query_registry_pv(&key).unwrap_or_else(|| "missing".to_string());
            format!("{key}={version}")
        })
        .collect::<Vec<_>>();
    format!("webview2_runtime_versions {}", entries.join("; "))
}

#[tauri::command]
fn memory_status() -> Result<crate::memory::MemoryStatus, String> {
    crate::memory::current_process_tree_memory()
}

#[tauri::command]
fn frontend_ready() {
    let was_ready = FRONTEND_READY.swap(true, Ordering::AcqRel);
    if !was_ready {
        log_diagnostic("frontend_ready received");
    }
}

#[derive(serde::Serialize, Clone)]
// Tauri events are JSON, so the live monitor keeps 30 ms chunks as compact
// i16 samples instead of shipping f32 PCM through IPC.
struct MicMonitorEvent {
    rms: f32,
    peak: f32,
    sample_count: usize,
    samples: Vec<i16>,
}

#[derive(Default)]
struct MicTestState(Mutex<Option<Sender<()>>>);

impl MicTestState {
    fn stop(&self) {
        match self.0.lock() {
            Ok(mut guard) => {
                if let Some(tx) = guard.take() {
                    // Receiver gone means the test thread already exited — not an error.
                    let _ = tx.send(());
                }
            }
            Err(e) => eprintln!("mic test state lock poisoned: {e}"),
        }
    }
}

pub(crate) struct RuntimeState(Mutex<RuntimeInner>);

static CLOUD_SETTINGS_SAVE_LOCK: Mutex<()> = Mutex::new(());

struct TrayItems<R: Runtime> {
    save_item: MenuItem<R>,
}

impl<R: Runtime> TrayItems<R> {
    fn set_hotkey_label(&self, label: &str) -> Result<(), String> {
        self.save_item
            .set_text(save_menu_text(label))
            .map_err(|e| e.to_string())
    }
}

struct RuntimeInner {
    tx: Option<Sender<Cmd>>,
    recording_generation: u64,
    settings: AppSettings,
    lol_url: Option<String>,
    active_game: Option<DetectedGame>,
    osu_title_events: Vec<OsuTitleEvent>,
    last_save_request: Option<Instant>,
    /// Codecs WebView2 can decode, reported by the frontend. Drives the
    /// recorder's Automatic selection; H.264 is the always-safe default.
    decodable_codecs: Vec<service::Codec>,
}

struct PreparedRuntimeRestart {
    settings: AppSettings,
}

#[derive(Debug)]
struct CommittedRuntimeRestart<T> {
    old_tx: Option<Sender<Cmd>>,
    replacement: Option<(T, u64)>,
    cleared_active_game: bool,
}

impl RuntimeState {
    fn new(settings: AppSettings, lol_url: Option<String>) -> Self {
        Self::from_parts(None, settings, lol_url)
    }

    #[cfg(test)]
    fn with_sender(tx: Sender<Cmd>, settings: AppSettings, lol_url: Option<String>) -> Self {
        Self::from_parts(Some(tx), settings, lol_url)
    }

    fn from_parts(tx: Option<Sender<Cmd>>, settings: AppSettings, lol_url: Option<String>) -> Self {
        let mut inner = RuntimeInner {
            tx: None,
            recording_generation: 0,
            settings,
            lol_url,
            active_game: None,
            osu_title_events: Vec::new(),
            last_save_request: None,
            decodable_codecs: vec![service::Codec::H264],
        };
        if let Some(tx) = tx {
            Self::install_recording_sender(&mut inner, tx);
        }
        Self(Mutex::new(inner))
    }

    fn install_recording_sender(inner: &mut RuntimeInner, tx: Sender<Cmd>) -> u64 {
        inner.recording_generation = inner.recording_generation.wrapping_add(1);
        inner.tx = Some(tx);
        inner.last_save_request = None;
        inner.recording_generation
    }

    fn clear_recording_sender_for_generation(&self, generation: u64) -> bool {
        let Ok(mut inner) = self.0.lock() else {
            return false;
        };
        if inner.recording_generation != generation || inner.tx.is_none() {
            return false;
        }
        inner.tx = None;
        inner.last_save_request = None;
        true
    }

    /// Replace the decodable-codec set from the frontend's canPlayType probe.
    /// Unknown keys are ignored; H.264 is always retained as the safe floor.
    fn set_decodable_codecs(&self, keys: &[String]) {
        let mut codecs = vec![service::Codec::H264];
        for key in keys {
            match key.as_str() {
                "hevc" if !codecs.contains(&service::Codec::Hevc) => {
                    codecs.push(service::Codec::Hevc)
                }
                "av1" if !codecs.contains(&service::Codec::Av1) => codecs.push(service::Codec::Av1),
                _ => {}
            }
        }
        match self.0.lock() {
            Ok(mut inner) => inner.decodable_codecs = codecs,
            Err(e) => eprintln!("set_decodable_codecs lock poisoned: {e}"),
        }
    }

    /// Build service options for the supplied settings and runtime context.
    fn options_for(
        settings: &AppSettings,
        lol_url: Option<String>,
        active_game: Option<&DetectedGame>,
        decodable_codecs: &[service::Codec],
    ) -> Result<service::ServiceOptions, String> {
        let mut opts = settings.to_service_options(lol_url)?;
        opts.decodable_codecs = decodable_codecs.to_vec();
        if let Some(game) = active_game {
            opts.capture_source = service::CaptureSource::WindowHandle {
                hwnd: game.hwnd,
                title: game.window_title.clone(),
            };
            opts.recording_mode = game.recording_mode.into();
            if crate::game_plugins::contains(&game.id) {
                opts.active_game_plugin_id = Some(game.id.clone());
            }
            // Tag clips with the active game (plugin or custom) so the library
            // can show its icon; this is independent of the plugin-only id above.
            opts.active_game = Some(service::ActiveGame {
                id: game.id.clone(),
                name: game.name.clone(),
            });
        }
        Ok(opts)
    }

    fn options(inner: &RuntimeInner) -> Result<service::ServiceOptions, String> {
        Self::options_for(
            &inner.settings,
            inner.lol_url.clone(),
            inner.active_game.as_ref(),
            &inner.decodable_codecs,
        )
    }

    fn prepare_service_restart(
        inner: &mut RuntimeInner,
    ) -> Result<(Option<Sender<Cmd>>, Option<ServiceOptions>), String> {
        if inner.tx.is_none() {
            return Ok((None, None));
        }
        let mut next_options = Self::options(inner)?;
        next_options.recover_abandoned_recordings = false;
        let old_tx = inner.tx.take();
        inner.last_save_request = None;
        Ok((old_tx, Some(next_options)))
    }

    fn prepare_settings_restart(
        &self,
        settings: AppSettings,
    ) -> Result<PreparedRuntimeRestart, String> {
        let inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
        let cleared_active_game = inner.active_game.is_some()
            && !active_game_still_configured(&settings, inner.active_game.as_ref());
        let active_game = if cleared_active_game {
            None
        } else {
            inner.active_game.as_ref()
        };
        if inner.tx.is_some() {
            Self::options_for(
                &settings,
                inner.lol_url.clone(),
                active_game,
                &inner.decodable_codecs,
            )?;
        }
        Ok(PreparedRuntimeRestart { settings })
    }

    fn commit_prepared_restart_with<T, F>(
        inner: &mut RuntimeInner,
        prepared: PreparedRuntimeRestart,
        spawn: F,
    ) -> Result<CommittedRuntimeRestart<T>, String>
    where
        F: FnOnce(ServiceOptions) -> (Sender<Cmd>, T),
    {
        let PreparedRuntimeRestart { settings } = prepared;
        let cleared_active_game = inner.active_game.is_some()
            && !active_game_still_configured(&settings, inner.active_game.as_ref());
        let active_game = if cleared_active_game {
            None
        } else {
            inner.active_game.as_ref()
        };
        let next_options = if inner.tx.is_some() {
            let mut options = Self::options_for(
                &settings,
                inner.lol_url.clone(),
                active_game,
                &inner.decodable_codecs,
            )?;
            options.recover_abandoned_recordings = false;
            Some(options)
        } else {
            None
        };

        inner.settings = settings;
        if cleared_active_game {
            inner.active_game = None;
        }
        let (old_tx, replacement) = if let Some(options) = next_options {
            let old_tx = inner.tx.take();
            let (tx, spawned) = spawn(options);
            let generation = Self::install_recording_sender(inner, tx);
            (old_tx, Some((spawned, generation)))
        } else {
            (None, None)
        };

        Ok(CommittedRuntimeRestart {
            old_tx,
            replacement,
            cleared_active_game,
        })
    }

    fn finish_prepared_restart<R: Runtime>(
        &self,
        app: AppHandle<R>,
        prepared: PreparedRuntimeRestart,
    ) -> Result<(), String> {
        let CommittedRuntimeRestart {
            old_tx,
            replacement,
            cleared_active_game,
        } = {
            let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            Self::commit_prepared_restart_with(&mut inner, prepared, service::spawn)?
        };
        if let Some(tx) = old_tx {
            let _ = tx.send(Cmd::Stop { announce: false });
        }
        if let Some((rx, generation)) = replacement {
            pump_events(app.clone(), rx, generation);
        }
        if cleared_active_game {
            let _ = app.emit("game-detection", GameDetectionEvent::from_detected(None));
        }
        Ok(())
    }

    fn request_save(&self) -> bool {
        const DOUBLE_TRIGGER_DEBOUNCE: Duration = Duration::from_millis(150);

        if let Ok(mut inner) = self.0.lock() {
            let Some(tx) = inner.tx.as_ref().cloned() else {
                return false;
            };
            let now = Instant::now();
            if inner
                .last_save_request
                .is_some_and(|last| now.duration_since(last) < DOUBLE_TRIGGER_DEBOUNCE)
            {
                return false;
            }
            if tx.send(Cmd::Save).is_ok() {
                inner.last_save_request = Some(now);
                return true;
            }
        }
        false
    }

    fn send(&self, cmd: Cmd) -> bool {
        if let Ok(inner) = self.0.lock() {
            if let Some(tx) = &inner.tx {
                let _ = tx.send(cmd);
                return true;
            }
        }
        false
    }

    fn osu_title_events_for_window(
        &self,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Vec<OsuTitleEvent> {
        let Some(start) = start else {
            return Vec::new();
        };
        let end = end.unwrap_or_else(unix_now);
        self.0
            .lock()
            .map(|inner| filter_osu_title_events(&inner.osu_title_events, start, end))
            .unwrap_or_default()
    }

    pub(crate) fn settings(&self) -> AppSettings {
        self.0
            .lock()
            .map(|inner| inner.settings.clone())
            .unwrap_or_default()
    }

    pub(crate) fn update_cloud<F>(&self, update: F) -> Result<AppSettings, String>
    where
        F: FnOnce(&mut crate::settings::CloudSettings),
    {
        // Serialize cloud settings saves so concurrent uploads preserve their
        // read-modify-write order without holding runtime state during disk I/O.
        let _save_guard = CLOUD_SETTINGS_SAVE_LOCK
            .lock()
            .map_err(|_| "cloud settings save lock poisoned")?;
        let next = {
            let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            update(&mut inner.settings.cloud);
            inner.settings.cloud.normalize();
            inner.settings.clone()
        };
        next.save()?;
        Ok(next)
    }

    pub(crate) fn update_osu<F>(&self, update: F) -> Result<AppSettings, String>
    where
        F: FnOnce(&mut crate::settings::OsuApiSettings),
    {
        let _save_guard = CLOUD_SETTINGS_SAVE_LOCK
            .lock()
            .map_err(|_| "settings save lock poisoned")?;
        let next = {
            let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            update(&mut inner.settings.osu);
            inner.settings.osu.normalize();
            inner.settings.clone()
        };
        next.save()?;
        Ok(next)
    }

    fn lock_cloud_settings_save() -> Result<MutexGuard<'static, ()>, String> {
        CLOUD_SETTINGS_SAVE_LOCK
            .lock()
            .map_err(|_| "cloud settings save lock poisoned".to_string())
    }

    fn active_shortcut_matches(&self, shortcut: &Shortcut) -> bool {
        let Ok(inner) = self.0.lock() else {
            return false;
        };
        inner
            .settings
            .hotkeys()
            .into_iter()
            .filter_map(|raw| parse_global_hotkey(raw).ok().flatten())
            .any(|active| &active == shortcut)
    }

    fn set_recording<R: Runtime>(
        &self,
        app: AppHandle<R>,
        recording: bool,
    ) -> Result<bool, String> {
        if recording {
            self.start_recording(app)
        } else {
            self.stop_recording()
        }
    }

    fn start_recording<R: Runtime>(&self, app: AppHandle<R>) -> Result<bool, String> {
        let (rx, generation) = {
            let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            if inner.tx.is_some() {
                return Ok(true);
            }
            let (tx, rx) = service::spawn(Self::options(&inner)?);
            let generation = Self::install_recording_sender(&mut inner, tx);
            (rx, generation)
        };
        pump_events(app, rx, generation);
        Ok(true)
    }

    fn stop_recording(&self) -> Result<bool, String> {
        let tx = {
            let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            let tx = inner.tx.take();
            inner.last_save_request = None;
            tx
        };
        if let Some(tx) = tx {
            let _ = tx.send(Cmd::Stop { announce: true });
        }
        Ok(false)
    }

    fn set_detected_game<R: Runtime>(
        &self,
        app: AppHandle<R>,
        detected: Option<DetectedGame>,
    ) -> Result<(), String> {
        let event = GameDetectionEvent::from_detected(detected.as_ref());
        let (old_tx, next_options, emit_event) = {
            let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            record_osu_title_event(&mut inner, detected.as_ref(), unix_now());
            if same_game_window(inner.active_game.as_ref(), detected.as_ref()) {
                if game_recording_mode_changed(inner.active_game.as_ref(), detected.as_ref()) {
                    inner.active_game = detected;
                    let (old_tx, next_options) = Self::prepare_service_restart(&mut inner)?;
                    (old_tx, next_options, true)
                } else if inner.active_game != detected {
                    inner.active_game = detected;
                    (None, None, true)
                } else {
                    (None, None, false)
                }
            } else {
                inner.active_game = detected;
                let (old_tx, next_options) = Self::prepare_service_restart(&mut inner)?;
                (old_tx, next_options, true)
            }
        };
        if let Some(tx) = old_tx {
            let _ = tx.send(Cmd::Stop { announce: false });
        }
        if let Some(options) = next_options {
            let (tx, rx) = service::spawn(options);
            let generation = {
                let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
                Self::install_recording_sender(&mut inner, tx)
            };
            pump_events(app.clone(), rx, generation);
        }
        if emit_event {
            let _ = app.emit("game-detection", event);
        }
        Ok(())
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn record_osu_title_event(inner: &mut RuntimeInner, detected: Option<&DetectedGame>, unix_s: i64) {
    const MAX_OSU_TITLE_EVENTS: usize = 512;
    let Some(game) = detected else {
        return;
    };
    if game.id != crate::game_plugins::OSU_ID {
        return;
    }
    let title = game.window_title.trim();
    if title.is_empty() {
        return;
    }
    if inner
        .osu_title_events
        .last()
        .is_some_and(|event| event.title == title)
    {
        return;
    }
    inner.osu_title_events.push(OsuTitleEvent {
        unix_s,
        title: title.to_string(),
    });
    if inner.osu_title_events.len() > MAX_OSU_TITLE_EVENTS {
        let overflow = inner.osu_title_events.len() - MAX_OSU_TITLE_EVENTS;
        inner.osu_title_events.drain(0..overflow);
    }
}

fn filter_osu_title_events(events: &[OsuTitleEvent], start: i64, end: i64) -> Vec<OsuTitleEvent> {
    let start = start - 5;
    let end = end.max(start) + 5;
    events
        .iter()
        .filter(|event| event.unix_s >= start && event.unix_s <= end)
        .cloned()
        .collect()
}

fn preserve_backend_owned_settings_fields(settings: &mut AppSettings, backend: &AppSettings) {
    settings.cloud.host_url = backend.cloud.host_url.clone();
    settings.cloud.public_url = backend.cloud.public_url.clone();
    settings.cloud.connected_user_id = backend.cloud.connected_user_id.clone();
    settings.cloud.connected_username = backend.cloud.connected_username.clone();
    settings.cloud.connected_display_name = backend.cloud.connected_display_name.clone();
    settings.cloud.credential_target = backend.cloud.credential_target.clone();
    settings.cloud.uploads = backend.cloud.uploads.clone();
    settings.osu = backend.osu.clone();
}

fn same_game_window(current: Option<&DetectedGame>, next: Option<&DetectedGame>) -> bool {
    match (current, next) {
        (Some(current), Some(next)) => current.id == next.id && current.hwnd == next.hwnd,
        (None, None) => true,
        _ => false,
    }
}

fn game_recording_mode_changed(
    current: Option<&DetectedGame>,
    next: Option<&DetectedGame>,
) -> bool {
    match (current, next) {
        (Some(current), Some(next)) => current.recording_mode != next.recording_mode,
        _ => false,
    }
}

fn active_game_still_configured(settings: &AppSettings, active: Option<&DetectedGame>) -> bool {
    let Some(active) = active else { return true };
    settings.games.auto_detect
        && (crate::games::built_in_game_still_configured(&settings.games, &active.id)
            || settings
                .games
                .custom_games
                .iter()
                .any(|game| game.enabled && game.id == active.id))
}

#[tauri::command]
fn save_replay(state: tauri::State<RuntimeState>) {
    state.request_save();
}

#[tauri::command]
fn restart_as_administrator<R: Runtime>(app: AppHandle<R>) -> Result<bool, String> {
    if crate::windows::current_process_is_elevated()? {
        return Ok(false);
    }
    crate::windows::launch_elevated_after(std::process::id())?;
    quit_app(&app);
    Ok(true)
}

#[tauri::command]
fn get_autostart_status<R: Runtime>(app: AppHandle<R>) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

fn set_autostart<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> Result<bool, String> {
    if !autostart_should_mutate_for_current_build() {
        return Ok(enabled);
    }
    let autostart = app.autolaunch();
    if enabled {
        autostart.enable().map_err(|e| e.to_string())?;
    } else {
        autostart.disable().map_err(|e| e.to_string())?;
    }
    autostart.is_enabled().map_err(|e| e.to_string())
}

fn autostart_should_mutate_for_current_build() -> bool {
    autostart_should_mutate_for_build(cfg!(debug_assertions))
}

fn autostart_should_mutate_for_build(debug_build: bool) -> bool {
    !debug_build
}

fn saved_autostart_preference_for_current_build(requested: bool, previous: bool) -> bool {
    saved_autostart_preference_for_build(requested, previous, cfg!(debug_assertions))
}

fn saved_autostart_preference_for_build(
    requested: bool,
    previous: bool,
    debug_build: bool,
) -> bool {
    if debug_build {
        previous
    } else {
        requested
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseRequestAction {
    Tray,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MinimizeRequestAction {
    Taskbar,
    Tray,
}

fn close_request_action(settings: &AppSettings) -> CloseRequestAction {
    if settings.close_to_tray {
        CloseRequestAction::Tray
    } else {
        CloseRequestAction::Quit
    }
}

fn minimize_request_action(settings: &AppSettings) -> MinimizeRequestAction {
    if settings.minimize_to_tray {
        MinimizeRequestAction::Tray
    } else {
        MinimizeRequestAction::Taskbar
    }
}

/// Brings the OS registrations in line with the configured global shortcuts:
/// registers shortcuts new in `new`, unregisters ones dropped from `old`.
/// A registration failure for a shortcut that was already configured (a
/// retry of one that was unavailable earlier) is returned as a warning; a
/// failure for a newly added or removed shortcut rolls back this call's
/// registrations and aborts.
fn sync_global_hotkeys<E>(
    old: &[Shortcut],
    new: &[Shortcut],
    is_registered: impl Fn(Shortcut) -> bool,
    mut register: impl FnMut(Shortcut) -> Result<(), E>,
    mut unregister: impl FnMut(Shortcut) -> Result<(), E>,
) -> Result<Vec<String>, String>
where
    E: std::fmt::Display,
{
    let mut warnings = Vec::new();
    let mut added = Vec::new();
    for shortcut in new {
        if is_registered(*shortcut) {
            continue;
        }
        match register(*shortcut) {
            Ok(()) => added.push(*shortcut),
            Err(e) if old.contains(shortcut) => {
                warnings.push(format!("global save hotkey still unavailable: {e}"));
            }
            Err(e) => {
                for shortcut in added {
                    let _ = unregister(shortcut);
                }
                return Err(format!("register hotkey: {e}"));
            }
        }
    }
    for shortcut in old {
        if new.contains(shortcut) || !is_registered(*shortcut) {
            continue;
        }
        if let Err(e) = unregister(*shortcut) {
            for shortcut in added {
                let _ = unregister(shortcut);
            }
            return Err(format!("replace hotkey: {e}"));
        }
    }
    Ok(warnings)
}

fn send_main_window_to_tray<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    app.state::<MicTestState>().stop();
    let _ = app.emit("suspend-review-playback", ());
    log_diagnostic(format!(
        "send main window to tray webviews={}",
        webview_labels(app)
    ));
    let mut windows = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| is_app_window_label(label))
        .collect::<Vec<_>>();
    windows.sort_by(|a, b| a.0.cmp(&b.0));

    if windows.is_empty() {
        log_diagnostic("send-to-tray skipped: app window not found");
    }
    for (label, window) in windows {
        log_window_state(&format!("send-to-tray before label={label}"), &window);
        window.hide().map_err(|e| e.to_string())?;
        log_diagnostic(format!("send-to-tray hide ok label={label}"));
        log_window_state(&format!("send-to-tray after hide label={label}"), &window);
    }
    Ok(())
}

fn quit_app<R: Runtime>(app: &AppHandle<R>) {
    log_diagnostic("quit app requested");
    app.state::<MicTestState>().stop();
    app.state::<RuntimeState>()
        .send(Cmd::Stop { announce: false });
    app.exit(0);
}

fn should_open_on_tray_event(event: &TrayIconEvent) -> bool {
    match event {
        TrayIconEvent::Click {
            button,
            button_state,
            ..
        } => should_open_on_tray_click(*button, *button_state),
        _ => false,
    }
}

fn should_open_on_tray_click(button: MouseButton, button_state: MouseButtonState) -> bool {
    button == MouseButton::Left && button_state == MouseButtonState::Up
}

#[tauri::command]
fn minimize_main_window<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    state: tauri::State<RuntimeState>,
) -> Result<(), String> {
    match minimize_request_action(&state.settings()) {
        MinimizeRequestAction::Taskbar => {
            window.minimize().map_err(|e| e.to_string())?;
            Ok(())
        }
        MinimizeRequestAction::Tray => send_main_window_to_tray(&app),
    }
}

#[tauri::command]
fn set_recording<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<RuntimeState>,
    recording: bool,
) -> Result<bool, String> {
    state.set_recording(app, recording)
}

/// Whether this build bundles a fixed WebView2 runtime (the "standalone"
/// installer variant). The install mode comes from the Tauri config baked in
/// at compile time, so the answer is a property of the installed binary, not
/// of the machine it runs on.
fn is_standalone_install<R: Runtime>(app: &AppHandle<R>) -> bool {
    matches!(
        app.config().bundle.windows.webview_install_mode,
        tauri::utils::config::WebviewInstallMode::FixedRuntime { .. }
    )
}

async fn check_update_for_channel<R: Runtime>(
    app: &AppHandle<R>,
    channel: UpdateChannel,
) -> Result<(Option<tauri_plugin_updater::Update>, Option<String>), String> {
    if !channel.enabled() {
        return Err(format!("{} updates are not available yet", channel.label()));
    }

    let endpoint = channel
        .endpoint(is_standalone_install(app))
        .parse()
        .map_err(|e| format!("parse update endpoint: {e}"))?;
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(20))
        .endpoints(vec![endpoint])
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;

    match updater.check().await {
        Ok(update) => Ok((update, None)),
        Err(tauri_plugin_updater::Error::ReleaseNotFound) => {
            Ok((None, Some(missing_release_metadata_message(channel))))
        }
        Err(e) => Err(e.to_string()),
    }
}

fn missing_release_metadata_message(channel: UpdateChannel) -> String {
    format!(
        "No {} release metadata is published yet. Publish a {} release first.",
        channel.label(),
        channel.label()
    )
}

#[tauri::command]
async fn check_for_updates<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, RuntimeState>,
) -> Result<UpdateCheckResult, String> {
    let settings = state.settings();
    let channel = settings.update_channel;
    let current_version = app.package_info().version.to_string();
    let (update, status) = check_update_for_channel(&app, channel).await?;

    Ok(UpdateCheckResult {
        channel,
        channel_label: channel.label(),
        current_version,
        available: update.is_some(),
        version: update.as_ref().map(|update| update.version.clone()),
        date: update
            .as_ref()
            .and_then(|update| update.date.map(|date| date.to_string())),
        notes: update.as_ref().and_then(|update| update.body.clone()),
        endpoint: channel.endpoint(is_standalone_install(&app)),
        status,
    })
}

#[tauri::command]
async fn install_update<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, RuntimeState>,
) -> Result<(), String> {
    let channel = state.settings().update_channel;
    let (update, status) = check_update_for_channel(&app, channel).await?;
    let Some(update) = update else {
        return Err(status.unwrap_or_else(|| "no update is available".into()));
    };

    app.state::<MicTestState>().stop();
    state.send(Cmd::Stop { announce: false });
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_settings(state: tauri::State<RuntimeState>) -> AppSettings {
    state.settings()
}

#[tauri::command]
async fn choose_media_folder(
    state: tauri::State<'_, RuntimeState>,
    current: Option<String>,
) -> Result<Option<String>, String> {
    let current_dir = current
        .as_deref()
        .and_then(|path| crate::settings::normalize_media_dir(path).ok())
        .filter(|path| path.exists())
        .or_else(|| state.settings().media_dir_path().ok())
        .unwrap_or_else(service::default_clips_dir);

    // Run the native modal off the main thread so recorder status and other
    // IPC keep flowing while the picker is open.
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_title("Choose Clipline Media Folder");
        if current_dir.exists() {
            dialog = dialog.set_directory(current_dir);
        }
        dialog.pick_folder().map(|path| path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn choose_replay_cache_folder(
    state: tauri::State<'_, RuntimeState>,
    current: Option<String>,
) -> Result<Option<String>, String> {
    let current_dir = current
        .as_deref()
        .and_then(|path| crate::settings::normalize_replay_cache_dir(path).ok())
        .filter(|path| path.exists())
        .or_else(|| state.settings().media_dir_path().ok())
        .unwrap_or_else(service::default_clips_dir);

    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_title("Choose Clipline Replay Cache Folder");
        if current_dir.exists() {
            dialog = dialog.set_directory(current_dir);
        }
        dialog.pick_folder().map(|path| path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_displays() -> Result<Vec<DisplayInfo>, String> {
    clipline_capture::windows::display::enumerate_displays()
        .map_err(|e| e.to_string())
        .map(|displays| {
            displays
                .into_iter()
                .map(|display| DisplayInfo {
                    id: display.id,
                    name: display.name,
                    x: display.x,
                    y: display.y,
                    width: display.width,
                    height: display.height,
                    is_primary: display.is_primary,
                })
                .collect()
        })
}

#[tauri::command]
fn list_audio_devices() -> Result<AudioDeviceLists, String> {
    clipline_capture::windows::wasapi::enumerate_audio_devices()
        .map_err(|e| e.to_string())
        .map(|devices| AudioDeviceLists {
            outputs: devices
                .outputs
                .into_iter()
                .map(|device| AudioDeviceInfo {
                    id: device.id,
                    name: device.name,
                    is_default: device.is_default,
                })
                .collect(),
            inputs: devices
                .inputs
                .into_iter()
                .map(|device| AudioDeviceInfo {
                    id: device.id,
                    name: device.name,
                    is_default: device.is_default,
                })
                .collect(),
        })
}

/// Every encoder this machine can use, for the Settings dropdown. Each
/// option carries its codec key so the frontend can flag codecs the in-app
/// player cannot decode.
///
/// `(async)` so Tauri runs this off the main thread: the first call triggers
/// FFmpeg encoder probing (several test-encode subprocesses, ~5s), which would
/// otherwise freeze the UI since synchronous commands run on the main thread.
#[tauri::command(async)]
fn probe_encoders() -> Vec<service::EncoderOption> {
    service::available_encoder_options()
}

#[tauri::command]
fn list_game_windows() -> Vec<GameWindowInfo> {
    crate::games::list_game_windows()
}

#[tauri::command(async)]
fn detect_installed_games(
    existing_custom_games: Vec<CustomGameSettings>,
) -> Vec<DetectedGameCandidate> {
    crate::game_discovery::detect_installed_games(&existing_custom_games)
}

/// Extract an executable's icon as a PNG `data:` URL for the custom-games UI.
/// Returns `None` when the path has no usable icon.
#[tauri::command]
fn extract_window_icon(exe_path: String) -> Option<String> {
    crate::game_icon::extract_exe_icon_data_url(&exe_path)
}

#[tauri::command]
fn list_game_plugins() -> Vec<GamePluginInfo> {
    crate::games::game_plugin_catalog()
}

/// The frontend reports which codecs WebView2 can decode (canPlayType) so
/// Automatic selection never records a clip the review player can't show.
/// Takes effect on the next recorder (re)start.
#[tauri::command]
fn report_decode_support(state: tauri::State<RuntimeState>, codecs: Vec<String>) {
    state.set_decodable_codecs(&codecs);
}

#[tauri::command]
fn start_microphone_test<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<MicTestState>,
    device_id: Option<String>,
    volume: f64,
    mono: bool,
) -> Result<(), String> {
    state.stop();
    let channels = if mono {
        clipline_capture::windows::wasapi::WasapiChannelMode::Mono
    } else {
        clipline_capture::windows::wasapi::WasapiChannelMode::Stereo
    };
    let (stop_tx, stop_rx) = mpsc::channel();
    {
        let mut guard = state.0.lock().map_err(|_| "mic test state lock poisoned")?;
        *guard = Some(stop_tx);
    }
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            let clock = clipline_capture::clock::RelativeClock::new(
                clipline_capture::windows::qpc_now_ticks_100ns().map_err(|e| e.to_string())?,
            );
            let mut source = clipline_capture::windows::wasapi::WasapiLoopback::start_microphone(
                clock,
                device_id.as_deref(),
                volume,
                channels,
            )
            .map_err(|e| e.to_string())?;
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
                let chunk = source.poll_monitor_chunk().map_err(|e| e.to_string())?;
                let samples = chunk
                    .samples
                    .into_iter()
                    .map(|sample| {
                        let scaled = (sample.clamp(-1.0, 1.0) * 32_768.0).round();
                        scaled.clamp(i16::MIN as f32, i16::MAX as f32) as i16
                    })
                    .collect();
                let _ = app.emit(
                    "mic-test",
                    MicMonitorEvent {
                        rms: chunk.level.rms,
                        peak: chunk.level.peak,
                        sample_count: chunk.level.sample_count,
                        samples,
                    },
                );
            }
            Ok(())
        };
        if let Err(e) = run() {
            let _ = app.emit("mic-test-error", e);
            let _ = app.emit("mic-test-stopped", ());
        }
    });
    Ok(())
}

#[tauri::command]
fn stop_microphone_test(state: tauri::State<MicTestState>) {
    state.stop();
}

fn parse_global_hotkey(raw: &str) -> Result<Option<Shortcut>, String> {
    if is_global_shortcut_hotkey(raw)? {
        parse_hotkey(raw).map(Some)
    } else {
        Ok(None)
    }
}

/// The configured Save Replay keybinds that go through the OS global-shortcut
/// registry (mouse and modified keyboard binds use the low-level hook instead).
fn global_hotkeys(settings: &AppSettings) -> Result<Vec<Shortcut>, String> {
    let mut shortcuts = Vec::new();
    for raw in settings.hotkeys() {
        if let Some(shortcut) = parse_global_hotkey(raw)? {
            shortcuts.push(shortcut);
        }
    }
    Ok(shortcuts)
}

fn save_hotkey_label(settings: &AppSettings) -> String {
    settings.hotkeys().join(" / ")
}

fn run_before_releasing_settings_save_lock<T>(
    save_guard: MutexGuard<'_, ()>,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let result = operation();
    drop(save_guard);
    result
}

#[tauri::command]
fn save_settings<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<RuntimeState>,
    tray_items: tauri::State<TrayItems<R>>,
    storage_settings: tauri::State<crate::library::StorageSettings>,
    mut settings: AppSettings,
) -> Result<AppSettings, String> {
    settings.hotkey = crate::settings::normalize_hotkey(&settings.hotkey)?;
    settings.hotkey_secondary = match settings.hotkey_secondary.as_deref() {
        Some(raw) if !raw.trim().is_empty() => Some(crate::settings::normalize_hotkey(raw)?),
        _ => None,
    };
    settings.validate()?;
    let media_dir = settings.media_dir_path()?;
    std::fs::create_dir_all(&media_dir)
        .map_err(|e| format!("create media folder {media_dir:?}: {e}"))?;
    // Extend the asset-protocol scope to the (possibly custom) root so the
    // webview can play clips from it, without granting the whole disk.
    app.asset_protocol_scope()
        .allow_directory(&media_dir, true)
        .map_err(|e| format!("scope media folder for playback: {e}"))?;

    let old = state.settings();

    // Apply the autostart registry change before persisting so settings.json
    // can never say "enabled" while the Run key update failed. Debug builds
    // share settings with installed builds, so they preserve this preference
    // and leave the shared Run key alone.
    let requested_open_on_startup = settings.open_on_startup;
    settings.open_on_startup = saved_autostart_preference_for_current_build(
        requested_open_on_startup,
        old.open_on_startup,
    );
    if settings.open_on_startup != old.open_on_startup
        && autostart_should_mutate_for_current_build()
    {
        settings.open_on_startup = set_autostart(&app, settings.open_on_startup)
            .map_err(|e| format!("update Windows startup registration: {e}"))?;
    }

    let old_global_hotkeys = global_hotkeys(&old)?;
    let new_global_hotkeys = global_hotkeys(&settings)?;
    let shortcuts = app.global_shortcut();
    let warnings = sync_global_hotkeys(
        &old_global_hotkeys,
        &new_global_hotkeys,
        |shortcut| shortcuts.is_registered(shortcut),
        |shortcut| shortcuts.register(shortcut),
        |shortcut| shortcuts.unregister(shortcut),
    )?;
    for message in warnings {
        eprintln!("{message}");
        let _ = app.emit("error", message);
    }

    let cloud_save_guard = RuntimeState::lock_cloud_settings_save()?;
    // Cloud connection/upload state and osu credential metadata are backend-owned
    // (mutated through dedicated commands). A settings Save carries the frontend's
    // snapshot of these fields, which can be stale; keep the authoritative backend
    // values while allowing user-editable Cloud preferences from the payload.
    preserve_backend_owned_settings_fields(&mut settings, &state.settings());
    // (Cloud default_visibility, delete_local_after_upload, auto_upload_rules stay as sent.)

    let quota_bytes = quota_bytes_from_gb(settings.disk_quota_gb)?;
    let prepared_restart = state.prepare_settings_restart(settings.clone())?;
    if let Err(error) = settings.save() {
        // Best-effort revert to the old registrations.
        let shortcuts = app.global_shortcut();
        let _ = sync_global_hotkeys(
            &new_global_hotkeys,
            &old_global_hotkeys,
            |shortcut| shortcuts.is_registered(shortcut),
            |shortcut| shortcuts.register(shortcut),
            |shortcut| shortcuts.unregister(shortcut),
        );
        return Err(error);
    }
    tray_items.set_hotkey_label(&save_hotkey_label(&settings))?;
    crate::hotkeys::set_save_hotkeys(&settings.hotkeys())?;
    run_before_releasing_settings_save_lock(cloud_save_guard, || {
        state.finish_prepared_restart(app, prepared_restart)
    })?;
    storage_settings.set_quota_bytes(quota_bytes);
    storage_settings.set_media_dir(media_dir);
    Ok(settings)
}

pub fn run() {
    let mut settings = AppSettings::load_or_default();
    let args: Vec<String> = std::env::args().collect();
    log_diagnostic(format!(
        "run start version={} args={args:?} log_path={:?}",
        env!("CARGO_PKG_VERSION"),
        diagnostic_log_path()
    ));
    log_diagnostic(webview2_runtime_diagnostic());
    let mut lol_url = None::<String>;
    if let Some(i) = args.iter().position(|a| a == "--window") {
        if let Some(title) = args.get(i + 1) {
            settings.capture_mode = CaptureMode::WindowTitle;
            settings.window_title = title.clone();
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--lol-url") {
        lol_url = args.get(i + 1).cloned();
    }
    if let Some(i) = args.iter().position(|a| a == "--disk-quota-gb") {
        match args
            .get(i + 1)
            .ok_or("missing --disk-quota-gb value")
            .and_then(|v| parse_quota_gb(v).map(|_| v))
        {
            Ok(v) => {
                if let Ok(gb) = v.parse::<f64>() {
                    settings.disk_quota_gb = gb;
                }
            }
            Err(e) => eprintln!("invalid disk quota: {e}"),
        }
    }
    if let Err(e) = settings.validate() {
        log_diagnostic(format!("settings invalid; using defaults: {e}"));
        eprintln!("invalid settings, using defaults: {e}");
        settings = AppSettings::default();
    }

    let quota_bytes = quota_bytes_from_gb(settings.disk_quota_gb)
        .unwrap_or(Some(service::DEFAULT_DISK_QUOTA_BYTES));
    let media_dir = settings
        .media_dir_path()
        .unwrap_or_else(|_| service::default_clips_dir());
    let scope_dir = media_dir.clone();
    let media_dir_for_setup = media_dir.clone();
    let audio_preview_scope_dir = crate::settings::audio_preview_cache_dir();
    let startup_global_hotkeys =
        global_hotkeys(&settings).unwrap_or_else(|_| vec![parse_hotkey("Alt+F10").unwrap()]);

    tauri::Builder::default()
        .manage(RuntimeState::new(settings.clone(), lol_url))
        .manage(MicTestState::default())
        .manage(crate::library::StorageSettings::new(quota_bytes, media_dir))
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            let launched_by_autostart = args.iter().any(|arg| arg == "--autostart");
            log_diagnostic(format!(
                "single-instance secondary launch launched_by_autostart={launched_by_autostart} cwd={cwd:?} args={args:?}"
            ));
            if !launched_by_autostart {
                if let Err(e) = open_main_window(app) {
                    log_diagnostic(format!("single-instance open existing failed: {e}"));
                    eprintln!("open existing window: {e}");
                }
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |_app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let state = _app.state::<RuntimeState>();
                        if state.active_shortcut_matches(shortcut) {
                            state.request_save();
                        }
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            save_replay,
            restart_as_administrator,
            set_recording,
            get_settings,
            minimize_main_window,
            choose_media_folder,
            choose_replay_cache_folder,
            list_displays,
            list_audio_devices,
            probe_encoders,
            report_decode_support,
            list_game_plugins,
            list_game_windows,
            detect_installed_games,
            extract_window_icon,
            memory_status,
            frontend_ready,
            start_microphone_test,
            stop_microphone_test,
            get_autostart_status,
            check_for_updates,
            install_update,
            save_settings,
            crate::cloud::cloud_status,
            crate::cloud::cloud_connect,
            crate::cloud::cloud_disconnect,
            crate::cloud::upload_clip_to_cloud,
            crate::cloud::sync_cloud_clip_status,
            crate::cloud::list_cloud_clips,
            crate::cloud::cloud_clip_thumbnail,
            crate::cloud::cache_cloud_clip_media,
            crate::cloud::cloud_user_profile,
            crate::cloud::cloud_user_avatar,
            crate::cloud::open_cloud_user_profile,
            crate::cloud::open_cloud_clip_url,
            crate::osu_api::osu_api_status,
            crate::osu_api::save_osu_api_settings,
            crate::osu_api::test_osu_api_connection,
            crate::osu_api::open_osu_api_setup_guide,
            crate::library::list_clips,
            crate::library::clip_poster,
            crate::library::delete_clip,
            crate::library::delete_clips,
            crate::library::rename_clip,
            crate::library::rename_clip_file,
            crate::library::export_clip,
            crate::library::prepare_clip_audio_sidecars,
            crate::library::reveal_clip,
            crate::library::copy_clip_to_clipboard,
            crate::library::open_media_folder,
            crate::library::storage_status
        ])
        .setup(move |app| {
            configure_bundled_ffmpeg(app);
            let osu_app = app.handle().clone();
            let osu_media_root = media_dir_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::osu_api::retry_pending_enrichment(&osu_app, osu_media_root).await
                {
                    eprintln!("retry osu! enrichment on launch: {e}");
                }
            });
            for hotkey in &startup_global_hotkeys {
                if let Err(e) = app.global_shortcut().register(*hotkey) {
                    let message =
                        format!("global save hotkey unavailable; continuing without it: {e}");
                    eprintln!("{message}");
                    let _ = app.handle().emit("error", message);
                }
            }
            if let Err(e) = crate::hotkeys::install_save_hook(&settings.hotkeys(), {
                let app = app.handle().clone();
                move || {
                    app.state::<RuntimeState>().request_save();
                }
            }) {
                let message = format!("low-level save hotkey unavailable: {e}");
                eprintln!("{message}");
                let _ = app.handle().emit("error", message);
            }
            // Bound the asset protocol to the configured media folder so clips
            // under a custom root play back, while the static config scope stays
            // narrow (the default Videos/Clipline location).
            if let Err(e) = app.asset_protocol_scope().allow_directory(&scope_dir, true) {
                eprintln!("could not scope media folder {scope_dir:?} for playback: {e}");
            }
            if let Err(e) = std::fs::create_dir_all(&audio_preview_scope_dir) {
                eprintln!(
                    "could not create audio preview cache {audio_preview_scope_dir:?}: {e}"
                );
            } else if let Err(e) = app
                .asset_protocol_scope()
                .allow_directory(&audio_preview_scope_dir, true)
            {
                eprintln!(
                    "could not scope audio preview cache {audio_preview_scope_dir:?} for playback: {e}"
                );
            }
            if let Err(e) = crate::library::prune_audio_preview_cache_on_startup() {
                eprintln!("could not prune audio preview cache on startup: {e}");
            }

            // Keep release builds in sync with the user's setting. Debug builds
            // share settings and registry state with installed builds, so cargo
            // runs must not disable or replace the installed autostart entry.
            if autostart_should_mutate_for_current_build() {
                let autostart = app.autolaunch();
                let _ = if settings.open_on_startup {
                    autostart.enable()
                } else {
                    autostart.disable()
                };
            }

            // When launched by the autostart registry entry, start in the tray
            // instead of flashing the main window.
            let launched_by_autostart = std::env::args().any(|arg| arg == "--autostart");
            log_diagnostic(format!(
                "setup start launched_by_autostart={launched_by_autostart} webviews={}",
                webview_labels(app.handle())
            ));

            let save_item = MenuItem::with_id(
                app,
                "save",
                save_menu_text(&save_hotkey_label(&settings)),
                true,
                None::<&str>,
            )?;
            let open_item = MenuItem::with_id(app, "open", "Open Clipline", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_item, &save_item, &quit_item])?;
            app.manage(TrayItems {
                save_item: save_item.clone(),
            });
            TrayIconBuilder::with_id("clipline")
                .icon(tray_icon())
                .tooltip("Clipline — replay buffer")
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "open" => {
                        log_diagnostic("tray menu event: open");
                        if let Err(e) = open_main_window(app) {
                            log_diagnostic(format!("tray menu open failed: {e}"));
                            eprintln!("open window: {e}");
                        }
                    }
                    "save" => {
                        log_diagnostic("tray menu event: save");
                        app.state::<RuntimeState>().request_save();
                    }
                    "quit" => {
                        log_diagnostic("tray menu event: quit");
                        quit_app(app);
                    }
                    other => {
                        log_diagnostic(format!("tray menu event: unknown id={other}"));
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if !matches!(event, TrayIconEvent::Move { .. }) {
                        log_diagnostic(format!("tray icon event: {event:?}"));
                    }
                    if should_open_on_tray_event(&event) {
                        log_diagnostic("tray icon event requests open");
                        if let Err(e) = open_main_window(tray.app_handle()) {
                            log_diagnostic(format!("tray icon open failed: {e}"));
                            eprintln!("open window: {e}");
                        }
                    }
                })
                .build(app)?;
            log_diagnostic(format!("tray build complete webviews={}", webview_labels(app.handle())));

            if let Err(e) = app
                .state::<RuntimeState>()
                .start_recording(app.handle().clone())
            {
                let message = format!("recorder startup failed: {e}");
                eprintln!("{message}");
                let _ = app.handle().emit("error", message);
            }
            spawn_game_detector(app.handle().clone());

            // The main window is created hidden by default so autostart launches
            // don't flash it. Show it for normal launches.
            if !launched_by_autostart {
                log_diagnostic("normal launch opening main window");
                if let Err(e) = open_main_window(app.handle()) {
                    log_diagnostic(format!("normal launch open failed: {e}"));
                    eprintln!("show main window on launch: {e}");
                }
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build tauri app")
        .run(move |app, event| match event {
            tauri::RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } if is_app_window_label(&label) => {
                log_diagnostic(format!("window event: app close requested label={label}"));
                api.prevent_close();
                match close_request_action(&app.state::<RuntimeState>().settings()) {
                    CloseRequestAction::Tray => {
                        log_diagnostic("close request action: tray");
                        if let Err(e) = send_main_window_to_tray(app) {
                            log_diagnostic(format!("close to tray failed: {e}"));
                            eprintln!("close to tray: {e}");
                        }
                    }
                    CloseRequestAction::Quit => {
                        log_diagnostic("close request action: quit");
                        quit_app(app);
                    }
                }
            }
            tauri::RunEvent::WindowEvent { label, event, .. } => {
                log_diagnostic(format!("window event: label={label} event={event:?}"));
            }
            tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } => {
                log_diagnostic("exit requested without code; preventing exit");
                api.prevent_exit();
            }
            tauri::RunEvent::Exit => {
                log_diagnostic("run event: exit");
                app.state::<MicTestState>().stop();
                app.state::<RuntimeState>()
                    .send(Cmd::Stop { announce: false });
            }
            _ => {}
        });
}

fn spawn_game_detector<R: Runtime>(app: AppHandle<R>) {
    std::thread::Builder::new()
        .name("clipline-game-detector".into())
        .spawn(move || {
            let mut last_error = None::<String>;
            loop {
                std::thread::sleep(GAME_DETECTOR_INTERVAL);
                let settings = app.state::<RuntimeState>().settings();
                let detected = crate::games::detect_active_game(&settings.games);
                match app
                    .state::<RuntimeState>()
                    .set_detected_game(app.clone(), detected)
                {
                    Ok(()) => last_error = None,
                    Err(e) if last_error.as_deref() != Some(e.as_str()) => {
                        last_error = Some(e.clone());
                        let _ = app.emit("error", format!("game detection: {e}"));
                    }
                    Err(_) => {}
                }
            }
        })
        .expect("spawn game detector thread");
}

fn open_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    log_diagnostic(format!(
        "open_main_window start webviews={}",
        webview_labels(app)
    ));
    let main_window = app.get_webview_window(MAIN_WINDOW_LABEL);

    match main_window_open_target(main_window.is_some()) {
        MainWindowOpenTarget::ExistingMain => {
            let window = main_window.expect("target requires main window");
            log_window_state("open existing before reveal", &window);
            let result = reveal_logged_window(&window, "open existing");
            log_window_state("open existing after reveal", &window);
            probe_webview_after_reveal(&window, "open existing after reveal");
            arm_frontend_ready_watchdog();
            result
        }
        MainWindowOpenTarget::NewMain => {
            log_diagnostic("open_main_window rebuilding missing main window");
            let window = build_main_window(app, MAIN_WINDOW_LABEL)?;
            log_window_state("open rebuilt before reveal", &window);
            let result = reveal_logged_window(&window, "open rebuilt");
            log_window_state("open rebuilt after reveal", &window);
            probe_webview_after_reveal(&window, "open rebuilt after reveal");
            arm_frontend_ready_watchdog();
            result
        }
    }
}

fn build_main_window<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
) -> Result<WebviewWindow<R>, String> {
    let mut config = app
        .config()
        .app
        .windows
        .first()
        .ok_or_else(|| "missing main window config".to_string())?
        .clone();
    config.label = label.to_string();
    WebviewWindowBuilder::from_config(app, &config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

fn reveal_logged_window<R: Runtime>(
    window: &WebviewWindow<R>,
    context: &str,
) -> Result<(), String> {
    reveal_main_window(
        || {
            let result = window.show();
            log_diagnostic(format!("{context} show: {}", result_debug(result.as_ref())));
            result
        },
        || {
            let result = window.unminimize();
            log_diagnostic(format!(
                "{context} unminimize: {}",
                result_debug(result.as_ref())
            ));
            result
        },
        || {
            let result = window.set_focus();
            log_diagnostic(format!(
                "{context} set_focus: {}",
                result_debug(result.as_ref())
            ));
            result
        },
    )
}

fn reveal_main_window<E>(
    show: impl FnOnce() -> Result<(), E>,
    unminimize: impl FnOnce() -> Result<(), E>,
    focus: impl FnOnce() -> Result<(), E>,
) -> Result<(), String>
where
    E: std::fmt::Display,
{
    show().map_err(|e| e.to_string())?;
    unminimize().map_err(|e| e.to_string())?;
    focus().map_err(|e| e.to_string())
}

fn pump_events<R: Runtime>(handle: AppHandle<R>, event_rx: Receiver<Event>, generation: u64) {
    std::thread::spawn(move || {
        for event in event_rx {
            if let Event::Status {
                recording: false, ..
            } = &event
            {
                handle
                    .state::<RuntimeState>()
                    .clear_recording_sender_for_generation(generation);
            }
            let _ = match &event {
                Event::Status { .. } => handle.emit("status", &event),
                Event::Saved { .. } => handle.emit("saved", &event),
                Event::Error { message } => handle.emit("error", message.clone()),
            };
            if let Event::Saved {
                full_session: false,
                ..
            } = &event
            {
                crate::sound::play_replay_saved();
            }
            if let Event::Saved {
                path,
                seconds,
                recording_start_unix,
                recording_end_unix,
                full_session: true,
                ..
            } = &event
            {
                let title_events = handle
                    .state::<RuntimeState>()
                    .osu_title_events_for_window(*recording_start_unix, *recording_end_unix);
                let saved = crate::osu_enrichment::OsuSavedClip {
                    path: std::path::PathBuf::from(path),
                    seconds: *seconds,
                    full_session: true,
                    recording_start_unix: *recording_start_unix,
                    recording_end_unix: *recording_end_unix,
                    title_events,
                };
                match crate::osu_enrichment::write_pending_for_saved_clip(&saved) {
                    Ok(Some(_)) => {
                        let app = handle.clone();
                        let media_root = saved
                            .path
                            .parent()
                            .map(std::path::Path::to_path_buf)
                            .unwrap_or_else(|| std::path::PathBuf::from("."));
                        tauri::async_runtime::spawn(async move {
                            if let Err(e) =
                                crate::osu_api::retry_pending_enrichment(&app, media_root).await
                            {
                                eprintln!("retry osu! enrichment after save: {e}");
                            }
                        });
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("queue osu! enrichment: {e}");
                    }
                }
            }
        }
    });
}

fn parse_quota_gb(raw: &str) -> Result<Option<u64>, &'static str> {
    let gb = raw.parse::<f64>().map_err(|_| "expected a number of GiB")?;
    if !gb.is_finite() || gb < 0.0 {
        return Err("quota must be a non-negative finite number");
    }
    if gb == 0.0 {
        return Ok(None);
    }
    quota_bytes_from_gb(gb).map_err(|_| "quota is too large")
}

fn save_menu_text(label: &str) -> String {
    format!("Save Replay ({label})")
}

/// Procedural 32x32 tray icon: a recording dot on a dark rounded square —
/// no asset files, no bundler.
fn tray_icon() -> Image<'static> {
    const N: usize = 32;
    let mut rgba = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let i = (y * N + x) * 4;
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let r = (dx * dx + dy * dy).sqrt();
            let (px, a) = if r < 7.0 {
                ([229u8, 72, 77], 255) // recording red
            } else if r < 15.0 {
                ([24u8, 26, 32], 255) // dark disc
            } else {
                ([0u8, 0, 0], 0)
            };
            rgba[i..i + 3].copy_from_slice(&px);
            rgba[i + 3] = a;
        }
    }
    Image::new_owned(rgba, N as u32, N as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        CloudUploadRecord, GameRecordingMode, ReplayStorageMode, ReplayStorageSettings,
    };

    #[test]
    fn quota_parser_converts_gib_to_bytes() {
        assert_eq!(parse_quota_gb("1").unwrap(), Some(1024 * 1024 * 1024));
        assert_eq!(parse_quota_gb("0.5").unwrap(), Some(512 * 1024 * 1024));
    }

    #[test]
    fn quota_parser_zero_disables_gc() {
        assert_eq!(parse_quota_gb("0").unwrap(), None);
    }

    #[test]
    fn quota_parser_rejects_negative_or_non_numeric_values() {
        assert!(parse_quota_gb("-1").is_err());
        assert!(parse_quota_gb("nope").is_err());
    }

    #[test]
    fn sync_global_hotkeys_skips_unregister_when_old_shortcut_is_stale() {
        let old_shortcut = parse_hotkey("Alt+F10").unwrap();
        let new_shortcut = parse_hotkey("Ctrl+F8").unwrap();
        let mut registered = Vec::new();
        let mut unregistered = Vec::new();

        let result = sync_global_hotkeys(
            &[old_shortcut],
            &[new_shortcut],
            |_| false,
            |shortcut| {
                registered.push(shortcut);
                Ok::<_, &'static str>(())
            },
            |shortcut| {
                unregistered.push(shortcut);
                Err::<(), _>("old shortcut was never registered")
            },
        );

        assert_eq!(result, Ok(Vec::new()));
        assert_eq!(registered, vec![new_shortcut]);
        assert!(unregistered.is_empty());
    }

    #[test]
    fn missing_unchanged_global_hotkey_is_retried_without_blocking_save() {
        let shortcut = parse_hotkey("Alt+F10").unwrap();
        let mut registered = Vec::new();

        let result = sync_global_hotkeys(
            &[shortcut],
            &[shortcut],
            |_| false,
            |shortcut| {
                registered.push(shortcut);
                Err::<(), _>("still owned by another app")
            },
            |_| Ok(()),
        );

        let warnings = result.expect("retry failure must not block save");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("still owned by another app"));
        assert_eq!(registered, vec![shortcut]);
    }

    #[test]
    fn sync_global_hotkeys_adds_secondary_and_keeps_registered_primary() {
        let primary = parse_hotkey("Alt+F10").unwrap();
        let secondary = parse_hotkey("Ctrl+F8").unwrap();
        let mut registered = Vec::new();
        let mut unregistered = Vec::new();

        let result = sync_global_hotkeys(
            &[primary],
            &[primary, secondary],
            |shortcut| shortcut == primary,
            |shortcut| {
                registered.push(shortcut);
                Ok::<_, &'static str>(())
            },
            |shortcut| {
                unregistered.push(shortcut);
                Ok(())
            },
        );

        assert_eq!(result, Ok(Vec::new()));
        assert_eq!(registered, vec![secondary]);
        assert!(unregistered.is_empty());
    }

    #[test]
    fn sync_global_hotkeys_rolls_back_new_registrations_on_failure() {
        let secondary = parse_hotkey("Ctrl+F8").unwrap();
        let removed = parse_hotkey("Alt+F10").unwrap();
        let mut unregistered = Vec::new();

        let result = sync_global_hotkeys(
            &[removed],
            &[secondary],
            |shortcut| shortcut == removed,
            |_| Ok::<_, &'static str>(()),
            |shortcut| {
                unregistered.push(shortcut);
                if shortcut == removed {
                    Err("cannot unregister")
                } else {
                    Ok(())
                }
            },
        );

        assert!(result.is_err());
        assert_eq!(unregistered, vec![removed, secondary]);
    }

    #[test]
    fn sync_global_hotkeys_removes_dropped_secondary() {
        let primary = parse_hotkey("Alt+F10").unwrap();
        let secondary = parse_hotkey("Ctrl+F8").unwrap();
        let mut registered = Vec::new();
        let mut unregistered = Vec::new();

        let result = sync_global_hotkeys(
            &[primary, secondary],
            &[primary],
            |_| true,
            |shortcut| {
                registered.push(shortcut);
                Ok::<_, &'static str>(())
            },
            |shortcut| {
                unregistered.push(shortcut);
                Ok(())
            },
        );

        assert_eq!(result, Ok(Vec::new()));
        assert!(registered.is_empty());
        assert_eq!(unregistered, vec![secondary]);
    }

    #[test]
    fn request_save_debounces_only_immediate_duplicate_triggers() {
        let (tx, rx) = mpsc::channel();
        let state = RuntimeState::with_sender(tx, AppSettings::default(), None);

        assert!(state.request_save());
        assert!(matches!(rx.try_recv(), Ok(Cmd::Save)));

        assert!(!state.request_save());
        assert!(rx.try_recv().is_err());

        {
            let mut inner = state.0.lock().unwrap();
            inner.last_save_request = Some(Instant::now() - Duration::from_millis(151));
        }

        assert!(state.request_save());
        assert!(matches!(rx.try_recv(), Ok(Cmd::Save)));
    }

    #[test]
    fn stopped_status_clears_matching_recording_sender() {
        let (tx, rx) = mpsc::channel();
        let state = RuntimeState::with_sender(tx, AppSettings::default(), None);
        let generation = {
            let mut inner = state.0.lock().unwrap();
            inner.last_save_request = Some(Instant::now());
            inner.recording_generation
        };

        assert!(state.clear_recording_sender_for_generation(generation));

        let inner = state.0.lock().unwrap();
        assert!(inner.tx.is_none());
        assert!(inner.last_save_request.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn stale_stopped_status_does_not_clear_newer_recording_sender() {
        let (old_tx, _old_rx) = mpsc::channel();
        let state = RuntimeState::with_sender(old_tx, AppSettings::default(), None);
        let stale_generation = state.0.lock().unwrap().recording_generation;
        let (new_tx, new_rx) = mpsc::channel();
        {
            let mut inner = state.0.lock().unwrap();
            RuntimeState::install_recording_sender(&mut inner, new_tx);
        }

        assert!(!state.clear_recording_sender_for_generation(stale_generation));
        assert!(state.send(Cmd::Save));
        assert!(matches!(new_rx.try_recv(), Ok(Cmd::Save)));
    }

    #[test]
    fn prepared_settings_restart_is_non_mutating_until_commit() {
        let (tx, rx) = mpsc::channel();
        let original = AppSettings::default();
        let state = RuntimeState::with_sender(tx, original.clone(), None);
        let mut changed = original.clone();
        changed.fps = 120;

        let prepared = state.prepare_settings_restart(changed).unwrap();

        assert_eq!(state.settings().fps, original.fps);
        assert!(state.send(Cmd::Save), "active sender must remain installed");
        assert!(matches!(rx.try_recv(), Ok(Cmd::Save)));
        assert_eq!(prepared.settings.fps, 120);

        drop(prepared); // Simulates a later tray-label or hook-registration failure.
        assert!(
            state.send(Cmd::Save),
            "dropping a plan must not stop recording"
        );
        assert!(matches!(rx.try_recv(), Ok(Cmd::Save)));
    }

    #[test]
    fn settings_save_lock_remains_held_through_runtime_commit() {
        let save_lock = Mutex::new(());
        let save_guard = save_lock.lock().unwrap();
        let original = AppSettings::default();
        let state = RuntimeState::new(original.clone(), None);
        let changed = AppSettings {
            fps: 120,
            ..original
        };
        let prepared = state.prepare_settings_restart(changed).unwrap();

        run_before_releasing_settings_save_lock(save_guard, || {
            let committed: CommittedRuntimeRestart<()> = {
                let mut inner = state.0.lock().unwrap();
                RuntimeState::commit_prepared_restart_with(&mut inner, prepared, |_| {
                    unreachable!("inactive runtime must not spawn a replacement")
                })
                .unwrap()
            };

            assert!(committed.old_tx.is_none());
            assert_eq!(state.settings().fps, 120);
            assert!(
                matches!(
                    save_lock.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "settings save lock was released before the runtime commit completed"
            );
            Ok(())
        })
        .unwrap();

        assert!(save_lock.try_lock().is_ok());
    }

    fn detected_game(id: &str, name: &str, hwnd: isize) -> DetectedGame {
        DetectedGame {
            id: id.into(),
            name: name.into(),
            hwnd,
            window_title: format!("{name} Window"),
            process_id: hwnd as u32,
            exe_name: format!("{name}.exe"),
            recording_mode: GameRecordingMode::FullSession,
        }
    }

    #[test]
    fn elevated_game_warning_requires_lower_privilege_clipline() {
        let game = detected_game("endfield", "Arknights: Endfield", 42);

        let blocked = GameDetectionEvent::from_detected_with_elevation(
            Some(&game),
            Ok(false),
            |process_id| Ok(process_id == 42),
        );
        assert!(blocked.elevated_hotkeys_blocked);

        let already_elevated =
            GameDetectionEvent::from_detected_with_elevation(Some(&game), Ok(true), |_| Ok(true));
        assert!(!already_elevated.elevated_hotkeys_blocked);

        let ordinary_game =
            GameDetectionEvent::from_detected_with_elevation(Some(&game), Ok(false), |_| Ok(false));
        assert!(!ordinary_game.elevated_hotkeys_blocked);

        let inactive =
            GameDetectionEvent::from_detected_with_elevation(None, Ok(false), |_| Ok(true));
        assert!(!inactive.elevated_hotkeys_blocked);
    }

    #[test]
    fn elevated_game_warning_is_conservative_when_elevation_cannot_be_queried() {
        let game = detected_game("endfield", "Arknights: Endfield", 42);

        let blocked =
            GameDetectionEvent::from_detected_with_elevation(Some(&game), Ok(false), |_| {
                Err("protected process".to_string())
            });
        assert!(blocked.elevated_hotkeys_blocked);

        let unknown_clipline = GameDetectionEvent::from_detected_with_elevation(
            Some(&game),
            Err("token query failed".to_string()),
            |_| Ok(true),
        );
        assert!(!unknown_clipline.elevated_hotkeys_blocked);
    }

    #[test]
    fn prepared_settings_restart_uses_current_game_and_sender_at_commit() {
        let (initial_tx, _initial_rx) = mpsc::channel();
        let state = RuntimeState::with_sender(initial_tx, AppSettings::default(), None);
        {
            state.0.lock().unwrap().active_game = Some(detected_game(
                crate::game_plugins::LEAGUE_OF_LEGENDS_ID,
                "League",
                41,
            ));
        }
        let changed = AppSettings {
            fps: 120,
            ..AppSettings::default()
        };
        let prepared = state.prepare_settings_restart(changed).unwrap();

        let (newer_tx, newer_rx) = mpsc::channel();
        let (replacement_tx, replacement_rx) = mpsc::channel();
        let mut committed_options = None;
        let committed = {
            let mut inner = state.0.lock().unwrap();
            inner.active_game = Some(detected_game(crate::game_plugins::OSU_ID, "osu!", 84));
            RuntimeState::install_recording_sender(&mut inner, newer_tx);
            RuntimeState::commit_prepared_restart_with(&mut inner, prepared, |options| {
                committed_options = Some(options);
                (replacement_tx, ())
            })
            .unwrap()
        };

        let options = committed_options.unwrap();
        assert_eq!(options.fps, 120);
        assert_eq!(
            options.capture_source,
            service::CaptureSource::WindowHandle {
                hwnd: 84,
                title: "osu! Window".into(),
            }
        );
        assert_eq!(
            options.active_game.as_ref().map(|game| game.id.as_str()),
            Some(crate::game_plugins::OSU_ID)
        );
        committed.old_tx.unwrap().send(Cmd::Save).unwrap();
        assert!(matches!(newer_rx.try_recv(), Ok(Cmd::Save)));
        assert!(committed.replacement.is_some());
        assert!(state.send(Cmd::Save));
        assert!(matches!(replacement_rx.try_recv(), Ok(Cmd::Save)));
    }

    #[test]
    fn prepared_settings_restart_restarts_sender_that_started_before_commit() {
        let state = RuntimeState::new(AppSettings::default(), None);
        let changed = AppSettings {
            fps: 120,
            ..AppSettings::default()
        };
        let prepared = state.prepare_settings_restart(changed).unwrap();

        let (started_tx, started_rx) = mpsc::channel();
        let (replacement_tx, replacement_rx) = mpsc::channel();
        let mut committed_options = None;
        let committed = {
            let mut inner = state.0.lock().unwrap();
            RuntimeState::install_recording_sender(&mut inner, started_tx);
            RuntimeState::commit_prepared_restart_with(&mut inner, prepared, |options| {
                committed_options = Some(options);
                (replacement_tx, ())
            })
            .unwrap()
        };

        assert_eq!(committed_options.unwrap().fps, 120);
        committed.old_tx.unwrap().send(Cmd::Save).unwrap();
        assert!(matches!(started_rx.try_recv(), Ok(Cmd::Save)));
        assert!(committed.replacement.is_some());
        assert!(state.send(Cmd::Save));
        assert!(matches!(replacement_rx.try_recv(), Ok(Cmd::Save)));
    }

    #[test]
    fn prepared_settings_restart_does_not_resurrect_sender_stopped_before_commit() {
        let (tx, _rx) = mpsc::channel();
        let state = RuntimeState::with_sender(tx, AppSettings::default(), None);
        let changed = AppSettings {
            fps: 120,
            ..AppSettings::default()
        };
        let prepared = state.prepare_settings_restart(changed).unwrap();

        let mut spawned = false;
        let committed = {
            let mut inner = state.0.lock().unwrap();
            inner.tx.take();
            RuntimeState::commit_prepared_restart_with(&mut inner, prepared, |_| {
                spawned = true;
                let (replacement_tx, _replacement_rx) = mpsc::channel();
                (replacement_tx, ())
            })
            .unwrap()
        };

        assert!(!spawned);
        assert!(committed.old_tx.is_none());
        assert!(committed.replacement.is_none());
        assert!(!state.send(Cmd::Save));
        assert_eq!(state.settings().fps, 120);
    }

    #[test]
    fn commit_time_restart_option_error_keeps_current_sender_and_settings() {
        let (tx, rx) = mpsc::channel();
        let original = AppSettings::default();
        let state = RuntimeState::with_sender(tx, original.clone(), None);
        let prepared = PreparedRuntimeRestart {
            settings: invalid_disk_replay_settings(),
        };

        let mut spawned = false;
        let error = {
            let mut inner = state.0.lock().unwrap();
            RuntimeState::commit_prepared_restart_with(&mut inner, prepared, |_| {
                spawned = true;
                let (replacement_tx, _replacement_rx) = mpsc::channel();
                (replacement_tx, ())
            })
            .unwrap_err()
        };

        assert!(error.contains("replay cache folder"), "{error}");
        assert!(!spawned);
        assert_eq!(state.settings().replay_storage, original.replay_storage);
        assert!(state.send(Cmd::Save));
        assert!(matches!(rx.try_recv(), Ok(Cmd::Save)));
    }

    #[test]
    fn recording_sender_survives_restart_option_error() {
        let (tx, _rx) = mpsc::channel();
        let mut inner = RuntimeInner {
            tx: Some(tx),
            recording_generation: 1,
            settings: invalid_disk_replay_settings(),
            lol_url: None,
            active_game: None,
            osu_title_events: Vec::new(),
            last_save_request: Some(Instant::now()),
            decodable_codecs: vec![service::Codec::H264],
        };

        let err = match RuntimeState::prepare_service_restart(&mut inner) {
            Ok(_) => panic!("restart options should fail"),
            Err(err) => err,
        };

        assert!(err.contains("replay cache folder"), "{err}");
        assert!(inner.tx.is_some(), "failed options must not drop sender");
        assert!(
            inner.last_save_request.is_some(),
            "failed options must not clear debounce state"
        );
    }

    #[test]
    fn prepared_restart_skips_abandoned_recording_recovery() {
        let (tx, _rx) = mpsc::channel();
        let mut inner = RuntimeInner {
            tx: Some(tx),
            recording_generation: 1,
            settings: AppSettings::default(),
            lol_url: None,
            active_game: None,
            osu_title_events: Vec::new(),
            last_save_request: Some(Instant::now()),
            decodable_codecs: vec![service::Codec::H264],
        };

        let (_old_tx, next_options) = RuntimeState::prepare_service_restart(&mut inner).unwrap();

        assert!(
            !next_options.unwrap().recover_abandoned_recordings,
            "internal recorder restarts must not recover another active recorder's temp file"
        );
    }

    #[test]
    fn recording_sender_survives_game_restart_option_error() {
        let (tx, _rx) = mpsc::channel();
        let mut inner = RuntimeInner {
            tx: Some(tx),
            recording_generation: 1,
            settings: invalid_disk_replay_settings(),
            lol_url: None,
            active_game: Some(DetectedGame {
                id: "custom-game".into(),
                name: "Game".into(),
                hwnd: 42,
                window_title: "Game".into(),
                process_id: 7,
                exe_name: "game.exe".into(),
                recording_mode: GameRecordingMode::FullSession,
            }),
            osu_title_events: Vec::new(),
            last_save_request: Some(Instant::now()),
            decodable_codecs: vec![service::Codec::H264],
        };

        let err = match RuntimeState::prepare_service_restart(&mut inner) {
            Ok(_) => panic!("restart options should fail"),
            Err(err) => err,
        };

        assert!(err.contains("replay cache folder"), "{err}");
        assert!(inner.tx.is_some(), "failed options must not drop sender");
        assert!(
            inner.last_save_request.is_some(),
            "failed options must not clear debounce state"
        );
    }

    #[test]
    fn preserve_backend_owned_settings_fields_keeps_upload_state_but_allows_preferences() {
        let mut frontend = AppSettings::default();
        frontend.cloud.host_url = "https://stale.example.com".into();
        frontend.cloud.public_url = Some("https://stale-public.example.com".into());
        frontend.cloud.connected_user_id = Some("stale-user".into());
        frontend.cloud.connected_username = Some("stale-name".into());
        frontend.cloud.connected_display_name = Some("Stale".into());
        frontend.cloud.credential_target = Some("stale-target".into());
        frontend.cloud.default_visibility = "public".into();
        frontend.cloud.delete_local_after_upload = true;
        frontend.cloud.auto_upload_rules = true;

        let mut backend = AppSettings::default();
        backend.cloud.host_url = "https://cloud.example.com".into();
        backend.cloud.public_url = Some("https://public.example.com".into());
        backend.cloud.connected_user_id = Some("user-1".into());
        backend.cloud.connected_username = Some("dain".into());
        backend.cloud.connected_display_name = Some("Dain".into());
        backend.cloud.credential_target = Some("clipline:user-1".into());
        backend.cloud.uploads.insert(
            "local-1".into(),
            CloudUploadRecord {
                local_clip_id: "local-1".into(),
                path: "D:\\Videos\\Clipline\\clip.mp4".into(),
                remote_clip_id: Some("remote-1".into()),
                remote_url: Some("https://public.example.com/remote-1".into()),
                visibility: "private".into(),
                upload_status: "uploaded_private".into(),
                error: None,
                updated_at_unix: 42,
            },
        );

        preserve_backend_owned_settings_fields(&mut frontend, &backend);

        assert_eq!(frontend.cloud.host_url, backend.cloud.host_url);
        assert_eq!(frontend.cloud.public_url, backend.cloud.public_url);
        assert_eq!(
            frontend.cloud.connected_user_id,
            backend.cloud.connected_user_id
        );
        assert_eq!(
            frontend.cloud.connected_username,
            backend.cloud.connected_username
        );
        assert_eq!(
            frontend.cloud.connected_display_name,
            backend.cloud.connected_display_name
        );
        assert_eq!(
            frontend.cloud.credential_target,
            backend.cloud.credential_target
        );
        assert_eq!(frontend.cloud.uploads, backend.cloud.uploads);
        assert_eq!(frontend.cloud.default_visibility, "public");
        assert!(frontend.cloud.delete_local_after_upload);
        assert!(frontend.cloud.auto_upload_rules);
    }

    #[test]
    fn preserve_backend_owned_settings_fields_keeps_osu_credentials_from_backend() {
        let mut frontend = AppSettings::default();
        frontend.osu.client_id = None;
        frontend.osu.user = None;
        frontend.osu.credential_target = None;
        frontend.osu.last_connected_username = None;

        let mut backend = AppSettings::default();
        backend.osu.client_id = Some("61835".into());
        backend.osu.user = Some("3426414".into());
        backend.osu.credential_target = Some("Clipline osu!:61835:3426414".into());
        backend.osu.last_connected_username = Some("Dain".into());

        preserve_backend_owned_settings_fields(&mut frontend, &backend);

        assert_eq!(frontend.osu, backend.osu);
    }

    #[test]
    fn detected_game_identity_ignores_volatile_window_title() {
        let current = DetectedGame {
            id: "custom-game".into(),
            name: "Game".into(),
            hwnd: 42,
            window_title: "Loading".into(),
            process_id: 7,
            exe_name: "game.exe".into(),
            recording_mode: GameRecordingMode::ReplaysOnly,
        };
        let updated_title = DetectedGame {
            window_title: "Paused".into(),
            ..current.clone()
        };
        let different_window = DetectedGame {
            hwnd: 43,
            ..current.clone()
        };

        assert!(same_game_window(Some(&current), Some(&updated_title)));
        assert!(!same_game_window(Some(&current), Some(&different_window)));
    }

    fn invalid_disk_replay_settings() -> AppSettings {
        AppSettings {
            replay_storage: ReplayStorageSettings {
                mode: ReplayStorageMode::Disk,
                disk_dir: String::new(),
                disk_quota_gb: 2.0,
                disk_acknowledged: true,
            },
            ..AppSettings::default()
        }
    }

    #[test]
    fn detected_game_recording_mode_change_requires_service_restart() {
        let current = DetectedGame {
            id: "custom-game".into(),
            name: "Game".into(),
            hwnd: 42,
            window_title: "Game".into(),
            process_id: 7,
            exe_name: "game.exe".into(),
            recording_mode: GameRecordingMode::ReplaysOnly,
        };
        let updated_mode = DetectedGame {
            recording_mode: GameRecordingMode::FullSession,
            ..current.clone()
        };
        let updated_title = DetectedGame {
            window_title: "Game - Loading".into(),
            ..current.clone()
        };

        assert!(same_game_window(Some(&current), Some(&updated_mode)));
        assert!(game_recording_mode_changed(
            Some(&current),
            Some(&updated_mode)
        ));
        assert!(!game_recording_mode_changed(
            Some(&current),
            Some(&updated_title)
        ));
    }

    #[test]
    fn osu_title_events_record_only_changed_osu_titles() {
        let mut inner = RuntimeInner {
            tx: None,
            recording_generation: 0,
            settings: AppSettings::default(),
            lol_url: None,
            active_game: None,
            osu_title_events: Vec::new(),
            last_save_request: None,
            decodable_codecs: vec![service::Codec::H264],
        };
        let osu = DetectedGame {
            id: crate::game_plugins::OSU_ID.into(),
            name: "osu!".into(),
            hwnd: 42,
            window_title: "osu! - xi - Blue Zenith [FOUR DIMENSIONS]".into(),
            process_id: 7,
            exe_name: "osu!.exe".into(),
            recording_mode: GameRecordingMode::FullSession,
        };
        let league = DetectedGame {
            id: crate::game_plugins::LEAGUE_OF_LEGENDS_ID.into(),
            name: "League of Legends".into(),
            window_title: "League".into(),
            exe_name: "League of Legends.exe".into(),
            ..osu.clone()
        };

        record_osu_title_event(&mut inner, Some(&osu), 100);
        record_osu_title_event(&mut inner, Some(&osu), 101);
        record_osu_title_event(&mut inner, Some(&league), 102);
        record_osu_title_event(
            &mut inner,
            Some(&DetectedGame {
                window_title: "osu!".into(),
                ..osu.clone()
            }),
            103,
        );

        assert_eq!(
            inner.osu_title_events,
            vec![
                OsuTitleEvent {
                    unix_s: 100,
                    title: "osu! - xi - Blue Zenith [FOUR DIMENSIONS]".into(),
                },
                OsuTitleEvent {
                    unix_s: 103,
                    title: "osu!".into(),
                }
            ]
        );
    }

    #[test]
    fn osu_title_events_for_window_filters_to_saved_recording_window() {
        let state = RuntimeState::new(AppSettings::default(), None);
        {
            let mut inner = state.0.lock().unwrap();
            inner.osu_title_events = vec![
                OsuTitleEvent {
                    unix_s: 90,
                    title: "too early".into(),
                },
                OsuTitleEvent {
                    unix_s: 96,
                    title: "start margin".into(),
                },
                OsuTitleEvent {
                    unix_s: 150,
                    title: "inside".into(),
                },
                OsuTitleEvent {
                    unix_s: 206,
                    title: "too late".into(),
                },
            ];
        }

        let titles: Vec<_> = state
            .osu_title_events_for_window(Some(100), Some(200))
            .into_iter()
            .map(|event| event.title)
            .collect();

        assert_eq!(titles, vec!["start margin", "inside"]);
    }

    #[test]
    fn built_in_league_profile_counts_as_active_game_configuration() {
        let active = DetectedGame {
            id: crate::game_plugins::LEAGUE_OF_LEGENDS_ID.into(),
            name: "League of Legends".into(),
            hwnd: 42,
            window_title: "League of Legends (TM) Client".into(),
            process_id: 7,
            exe_name: "League of Legends.exe".into(),
            recording_mode: GameRecordingMode::FullSession,
        };
        let mut settings = AppSettings::default();

        assert!(active_game_still_configured(&settings, Some(&active)));

        settings.games.plugins.insert(
            crate::game_plugins::LEAGUE_OF_LEGENDS_ID.into(),
            crate::settings::GamePluginSettings {
                enabled: false,
                recording_mode: GameRecordingMode::FullSession,
                review: Default::default(),
            },
        );
        assert!(!active_game_still_configured(&settings, Some(&active)));
    }

    #[test]
    fn window_request_actions_follow_general_settings() {
        let defaults = AppSettings::default();
        assert_eq!(close_request_action(&defaults), CloseRequestAction::Tray);
        assert_eq!(
            minimize_request_action(&defaults),
            MinimizeRequestAction::Taskbar
        );

        let settings = AppSettings {
            close_to_tray: false,
            minimize_to_tray: true,
            ..AppSettings::default()
        };
        assert_eq!(close_request_action(&settings), CloseRequestAction::Quit);
        assert_eq!(
            minimize_request_action(&settings),
            MinimizeRequestAction::Tray
        );
    }

    #[test]
    fn debug_build_autostart_policy_skips_registry_mutation() {
        assert!(!autostart_should_mutate_for_build(true));
        assert!(autostart_should_mutate_for_build(false));
    }

    #[test]
    fn debug_build_preserves_saved_autostart_preference() {
        assert!(saved_autostart_preference_for_build(false, true, true));
        assert!(!saved_autostart_preference_for_build(true, false, true));
        assert!(saved_autostart_preference_for_build(true, false, false));
        assert!(!saved_autostart_preference_for_build(false, true, false));
    }

    #[test]
    fn release_build_autostart_policy_honors_user_choice() {
        assert!(saved_autostart_preference_for_build(true, false, false));
        assert!(!saved_autostart_preference_for_build(false, true, false));
    }

    #[test]
    fn native_shell_starts_recorder_after_single_instance_accepts_process() {
        let app = include_str!("app.rs");
        let run_start = app.find("pub fn run()").expect("run function should exist");
        let run_body = &app[run_start..];
        let run_end = run_body
            .find("\nfn spawn_game_detector")
            .expect("run function should be followed by spawn_game_detector");
        let run_body = &run_body[..run_end];
        let single_instance = run_body
            .find("tauri_plugin_single_instance::init")
            .expect("single-instance plugin should be installed");
        let setup = run_body
            .find(".setup(move |app|")
            .expect("app setup should be registered");
        let recorder_start = run_body
            .find("start_recording(app.handle().clone())")
            .expect("setup should start the recorder after plugins are installed");

        assert!(
            single_instance < setup,
            "single-instance plugin must be installed before setup runs"
        );
        assert!(
            setup < recorder_start,
            "initial recorder startup must happen from setup after single-instance registration"
        );
        assert!(
            !run_body[..single_instance].contains("service::spawn("),
            "run() must not spawn the recorder before single-instance can reject a duplicate launch"
        );
    }

    #[test]
    fn webview_repair_notice_is_only_needed_for_dead_webview_signals() {
        assert!(should_show_webview_repair_notice(
            WebviewRepairNoticeReason::GetterFailedToReceiveMessage,
            false,
        ));
        assert!(should_show_webview_repair_notice(
            WebviewRepairNoticeReason::FrontendReadyTimeout,
            false,
        ));
        assert!(!should_show_webview_repair_notice(
            WebviewRepairNoticeReason::OtherGetterError,
            false,
        ));
        assert!(!should_show_webview_repair_notice(
            WebviewRepairNoticeReason::GetterFailedToReceiveMessage,
            true,
        ));
    }

    #[test]
    fn classifies_tauri_runtime_receive_failure_as_dead_webview() {
        let err = tauri::Error::Runtime(tauri_runtime::Error::FailedToReceiveMessage);

        assert_eq!(
            classify_webview_getter_error(&err),
            WebviewRepairNoticeReason::GetterFailedToReceiveMessage
        );
    }

    #[test]
    fn tray_left_click_opens_only_on_button_release() {
        assert!(should_open_on_tray_click(
            MouseButton::Left,
            MouseButtonState::Up
        ));
        assert!(!should_open_on_tray_click(
            MouseButton::Left,
            MouseButtonState::Down
        ));
        assert!(!should_open_on_tray_click(
            MouseButton::Right,
            MouseButtonState::Up
        ));
        assert!(!should_open_on_tray_click(
            MouseButton::Middle,
            MouseButtonState::Up
        ));
    }

    #[test]
    fn app_window_labels_include_only_main_window() {
        assert!(is_app_window_label("main"));
        assert!(!is_app_window_label("main-recovery-1"));
        assert!(!is_app_window_label("settings"));
        assert!(!is_app_window_label("mainframe"));
    }

    #[test]
    fn unresponsive_main_window_reveals_existing_handle() {
        assert_eq!(
            main_window_open_target(true),
            MainWindowOpenTarget::ExistingMain
        );
        assert_eq!(
            main_window_open_target(false),
            MainWindowOpenTarget::NewMain
        );
    }

    #[test]
    fn parses_webview2_runtime_version_from_reg_output() {
        let output = r#"
HKEY_CURRENT_USER\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}
    pv    REG_SZ    120.0.2210.55
"#;

        assert_eq!(
            parse_reg_pv_output(output).as_deref(),
            Some("120.0.2210.55")
        );
    }

    #[test]
    fn opening_main_window_restores_before_focus() {
        let calls = std::cell::RefCell::new(Vec::new());

        reveal_main_window(
            || {
                calls.borrow_mut().push("show");
                Ok::<(), String>(())
            },
            || {
                calls.borrow_mut().push("unminimize");
                Ok::<(), String>(())
            },
            || {
                calls.borrow_mut().push("focus");
                Ok::<(), String>(())
            },
        )
        .unwrap();

        assert_eq!(*calls.borrow(), ["show", "unminimize", "focus"]);
    }

    #[test]
    fn diagnostic_log_path_uses_clipline_appdata_file() {
        let path = diagnostic_log_path_from_appdata(std::path::Path::new(
            r"C:\Users\friend\AppData\Roaming",
        ));

        assert_eq!(
            path,
            std::path::PathBuf::from(r"C:\Users\friend\AppData\Roaming\Clipline\clipline.log")
        );
    }

    #[test]
    fn diagnostic_log_line_is_timestamped_and_single_line() {
        let timestamp = chrono::DateTime::parse_from_rfc3339("2026-06-24T12:34:56.789Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        assert_eq!(
            format_diagnostic_log_line(timestamp, 42, "tray open\nshow: ok"),
            "2026-06-24T12:34:56.789Z pid=42 tray open show: ok"
        );
    }

    #[test]
    fn disabled_stable_channel_cannot_check_updates_yet() {
        assert!(!UpdateChannel::Stable.enabled());
        assert!(UpdateChannel::Nightly.enabled());
    }

    #[test]
    fn missing_release_metadata_message_names_channel_workflow() {
        assert_eq!(
            missing_release_metadata_message(UpdateChannel::Nightly),
            "No Nightly release metadata is published yet. Publish a Nightly release first."
        );
    }

    #[test]
    fn active_full_session_game_sets_service_recording_mode() {
        let inner = RuntimeInner {
            tx: None,
            recording_generation: 0,
            settings: AppSettings::default(),
            lol_url: None,
            active_game: Some(DetectedGame {
                id: "custom-game".into(),
                name: "Game".into(),
                hwnd: 42,
                window_title: "Game Window".into(),
                process_id: 7,
                exe_name: "game.exe".into(),
                recording_mode: GameRecordingMode::FullSession,
            }),
            osu_title_events: Vec::new(),
            last_save_request: None,
            decodable_codecs: vec![service::Codec::H264],
        };

        let opts = RuntimeState::options(&inner).unwrap();

        assert_eq!(opts.active_game_plugin_id, None);
        assert_eq!(opts.recording_mode, service::RecordingMode::FullSession);
        assert_eq!(
            opts.capture_source,
            service::CaptureSource::WindowHandle {
                hwnd: 42,
                title: "Game Window".into(),
            }
        );
    }

    #[test]
    fn active_built_in_game_sets_service_plugin_id_for_event_sources() {
        let inner = RuntimeInner {
            tx: None,
            recording_generation: 0,
            settings: AppSettings::default(),
            lol_url: Some("http://mock".into()),
            active_game: Some(DetectedGame {
                id: crate::game_plugins::LEAGUE_OF_LEGENDS_ID.into(),
                name: "League of Legends".into(),
                hwnd: 42,
                window_title: "League".into(),
                process_id: 7,
                exe_name: "League of Legends.exe".into(),
                recording_mode: GameRecordingMode::FullSession,
            }),
            osu_title_events: Vec::new(),
            last_save_request: None,
            decodable_codecs: vec![service::Codec::H264],
        };

        let opts = RuntimeState::options(&inner).unwrap();

        assert_eq!(
            opts.active_game_plugin_id.as_deref(),
            Some(crate::game_plugins::LEAGUE_OF_LEGENDS_ID)
        );
        assert_eq!(opts.lol_url.as_deref(), Some("http://mock"));
    }
}
