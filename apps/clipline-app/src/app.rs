//! Tauri shell: tray, Alt+F10 global hotkey, status webview — all thin
//! wiring around the recorder service thread.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_updater::UpdaterExt;

use crate::game_plugins::GamePluginInfo;
use crate::games::{DetectedGame, GameWindowInfo};
use crate::platform;
use crate::platform::{AudioDeviceLists, DisplayInfo};
use crate::platform::PlatformCapabilities;
use crate::service::{self, Cmd, Event, ServiceOptions};
use crate::settings::{
    parse_hotkey, quota_bytes_from_gb, AppSettings, CaptureMode, GameRecordingMode,
};
use crate::updates::UpdateChannel;

#[derive(serde::Serialize, Clone)]
struct GameDetectionEvent {
    active: bool,
    name: Option<String>,
    window_title: Option<String>,
    process_id: Option<u32>,
    exe_name: Option<String>,
    recording_mode: Option<GameRecordingMode>,
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
        match detected {
            Some(game) => Self {
                active: true,
                name: Some(game.name.clone()),
                window_title: Some(game.window_title.clone()),
                process_id: Some(game.process_id),
                exe_name: Some(game.exe_name.clone()),
                recording_mode: Some(game.recording_mode),
            },
            None => Self {
                active: false,
                name: None,
                window_title: None,
                process_id: None,
                exe_name: None,
                recording_mode: None,
            },
        }
    }
}

#[tauri::command]
fn memory_status() -> Result<crate::memory::MemoryStatus, String> {
    crate::platform::memory_status()
}

#[derive(serde::Serialize, Clone)]
// Tauri events are JSON, so the live monitor keeps 30 ms chunks as compact
// i16 samples instead of shipping f32 PCM through IPC.
#[cfg(windows)]
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
    fn set_hotkey_label(&self, hotkey: &str) -> Result<(), String> {
        self.save_item
            .set_text(save_menu_text(hotkey))
            .map_err(|e| e.to_string())
    }
}

struct RuntimeInner {
    tx: Option<Sender<Cmd>>,
    settings: AppSettings,
    lol_url: Option<String>,
    active_game: Option<DetectedGame>,
    last_save_request: Option<Instant>,
    /// Codecs WebView2 can decode, reported by the frontend. Drives the
    /// recorder's Automatic selection; H.264 is the always-safe default.
    decodable_codecs: Vec<service::Codec>,
}

struct PreparedRuntimeRestart {
    old_tx: Option<Sender<Cmd>>,
    next_options: Option<ServiceOptions>,
    cleared_active_game: bool,
}

impl RuntimeState {
    fn new(tx: Sender<Cmd>, settings: AppSettings, lol_url: Option<String>) -> Self {
        Self(Mutex::new(RuntimeInner {
            tx: Some(tx),
            settings,
            lol_url,
            active_game: None,
            last_save_request: None,
            decodable_codecs: vec![service::Codec::H264],
        }))
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

    /// Build service options for the current settings with the reported
    /// decodable codecs injected. Caller holds the lock.
    fn options(inner: &RuntimeInner) -> Result<service::ServiceOptions, String> {
        let mut opts = inner.settings.to_service_options(inner.lol_url.clone())?;
        opts.decodable_codecs = inner.decodable_codecs.clone();
        if let Some(game) = &inner.active_game {
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

    fn prepare_service_restart(
        inner: &mut RuntimeInner,
    ) -> Result<(Option<Sender<Cmd>>, Option<ServiceOptions>), String> {
        if inner.tx.is_none() {
            return Ok((None, None));
        }
        let next_options = Self::options(inner)?;
        let old_tx = inner.tx.take();
        inner.last_save_request = None;
        Ok((old_tx, Some(next_options)))
    }

    fn prepare_settings_restart(
        &self,
        settings: AppSettings,
    ) -> Result<PreparedRuntimeRestart, String> {
        let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
        let cleared_active_game = inner.active_game.is_some()
            && !active_game_still_configured(&settings, inner.active_game.as_ref());
        if cleared_active_game {
            inner.active_game = None;
        }
        inner.settings = settings;
        let (old_tx, next_options) = Self::prepare_service_restart(&mut inner)?;
        Ok(PreparedRuntimeRestart {
            old_tx,
            next_options,
            cleared_active_game,
        })
    }

    fn finish_prepared_restart<R: Runtime>(
        &self,
        app: AppHandle<R>,
        prepared: PreparedRuntimeRestart,
    ) -> Result<(), String> {
        if let Some(tx) = prepared.old_tx {
            let _ = tx.send(Cmd::Stop { announce: false });
        }
        if let Some(options) = prepared.next_options {
            let (tx, rx) = service::spawn(options);
            {
                let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
                inner.tx = Some(tx);
                inner.last_save_request = None;
            }
            pump_events(app.clone(), rx);
        }
        if prepared.cleared_active_game {
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

    fn lock_cloud_settings_save() -> Result<MutexGuard<'static, ()>, String> {
        CLOUD_SETTINGS_SAVE_LOCK
            .lock()
            .map_err(|_| "cloud settings save lock poisoned".to_string())
    }

    fn active_shortcut_matches(&self, shortcut: &Shortcut) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|inner| parse_hotkey(&inner.settings.hotkey).ok())
            .is_some_and(|active| &active == shortcut)
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
        let rx = {
            let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            if inner.tx.is_some() {
                return Ok(true);
            }
            let (tx, rx) = service::spawn(Self::options(&inner)?);
            inner.tx = Some(tx);
            inner.last_save_request = None;
            rx
        };
        pump_events(app, rx);
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
            {
                let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
                inner.tx = Some(tx);
                inner.last_save_request = None;
            }
            pump_events(app.clone(), rx);
        }
        if emit_event {
            let _ = app.emit("game-detection", event);
        }
        Ok(())
    }
}

fn preserve_backend_cloud_fields(settings: &mut AppSettings, backend: &AppSettings) {
    settings.cloud.host_url = backend.cloud.host_url.clone();
    settings.cloud.public_url = backend.cloud.public_url.clone();
    settings.cloud.connected_user_id = backend.cloud.connected_user_id.clone();
    settings.cloud.connected_username = backend.cloud.connected_username.clone();
    settings.cloud.credential_target = backend.cloud.credential_target.clone();
    settings.cloud.uploads = backend.cloud.uploads.clone();
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

fn rebind_global_hotkey<E>(
    old_shortcut: Shortcut,
    new_shortcut: Shortcut,
    old_is_registered: bool,
    mut register: impl FnMut(Shortcut) -> Result<(), E>,
    mut unregister: impl FnMut(Shortcut) -> Result<(), E>,
) -> Result<(), String>
where
    E: std::fmt::Display,
{
    register(new_shortcut).map_err(|e| format!("register hotkey: {e}"))?;
    if old_is_registered {
        if let Err(e) = unregister(old_shortcut) {
            let _ = unregister(new_shortcut);
            return Err(format!("replace hotkey: {e}"));
        }
    }
    Ok(())
}

fn retry_missing_global_hotkey<E>(
    shortcut: Shortcut,
    is_registered: bool,
    register: impl FnOnce(Shortcut) -> Result<(), E>,
) -> Option<String>
where
    E: std::fmt::Display,
{
    if is_registered {
        None
    } else {
        register(shortcut).err().map(|e| e.to_string())
    }
}

fn send_main_window_to_tray<R: Runtime>(app: &AppHandle<R>, destroy: bool) -> Result<(), String> {
    app.state::<MicTestState>().stop();
    if let Some(window) = app.get_webview_window("main") {
        if destroy {
            window.destroy().map_err(|e| e.to_string())?;
        } else {
            window.hide().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn quit_app<R: Runtime>(app: &AppHandle<R>) {
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
    state: tauri::State<RuntimeState>,
) -> Result<(), String> {
    match minimize_request_action(&state.settings()) {
        MinimizeRequestAction::Taskbar => {
            if let Some(window) = app.get_webview_window("main") {
                window.minimize().map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        MinimizeRequestAction::Tray => send_main_window_to_tray(&app, false),
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

async fn check_update_for_channel<R: Runtime>(
    app: &AppHandle<R>,
    channel: UpdateChannel,
) -> Result<(Option<tauri_plugin_updater::Update>, Option<String>), String> {
    if !channel.enabled() {
        return Err(format!("{} updates are not available yet", channel.label()));
    }

    let endpoint = channel
        .endpoint()
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
        endpoint: channel.endpoint(),
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
    crate::platform::list_displays()
}

#[tauri::command]
fn list_audio_devices() -> Result<AudioDeviceLists, String> {
    crate::platform::list_audio_devices()
}

#[tauri::command]
fn platform_capabilities() -> PlatformCapabilities {
    platform::capabilities()
}

#[cfg(windows)]
fn start_microphone_test_windows<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<MicTestState>,
    device_id: Option<String>,
    volume: f64,
    mono: bool,
) -> Result<(), String> {
    use std::sync::mpsc;

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
fn start_microphone_test<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<MicTestState>,
    device_id: Option<String>,
    volume: f64,
    mono: bool,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        start_microphone_test_windows(app, state, device_id, volume, mono)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (app, state, device_id, volume, mono);
        Err("macOS microphone test is not implemented in Milestone 1".into())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (app, state, device_id, volume, mono);
        Err("Microphone test is unsupported on this platform".into())
    }
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
fn stop_microphone_test(state: tauri::State<MicTestState>) {
    state.stop();
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

    if settings.hotkey != old.hotkey {
        let old_shortcut = parse_hotkey(&old.hotkey)?;
        let new_shortcut = parse_hotkey(&settings.hotkey)?;
        let shortcuts = app.global_shortcut();
        rebind_global_hotkey(
            old_shortcut,
            new_shortcut,
            shortcuts.is_registered(old_shortcut),
            |shortcut| shortcuts.register(shortcut),
            |shortcut| shortcuts.unregister(shortcut),
        )?;
    } else {
        let shortcut = parse_hotkey(&settings.hotkey)?;
        let shortcuts = app.global_shortcut();
        if let Some(e) =
            retry_missing_global_hotkey(shortcut, shortcuts.is_registered(shortcut), |shortcut| {
                shortcuts.register(shortcut)
            })
        {
            let message = format!("global save hotkey still unavailable: {e}");
            eprintln!("{message}");
            let _ = app.emit("error", message);
        }
    }

    let cloud_save_guard = RuntimeState::lock_cloud_settings_save()?;
    // Cloud connection + upload state is backend-owned (mutated by cloud_connect
    // and upload_clip_to_cloud via update_cloud). A settings Save carries the
    // frontend's snapshot of these fields, which can be stale — e.g. a Save
    // fired during an in-flight upload would clobber freshly written records or
    // the connection identity. Keep the authoritative backend values; only the
    // user-editable cloud preferences below come from the payload.
    preserve_backend_cloud_fields(&mut settings, &state.settings());
    // (default_visibility, delete_local_after_upload, auto_upload_rules stay as sent.)

    if let Err(e) = settings.save() {
        if settings.hotkey != old.hotkey {
            let _ = app
                .global_shortcut()
                .unregister(parse_hotkey(&settings.hotkey)?);
            let _ = app.global_shortcut().register(parse_hotkey(&old.hotkey)?);
        }
        return Err(e);
    }

    let quota_bytes = quota_bytes_from_gb(settings.disk_quota_gb)?;
    let prepared_restart = state.prepare_settings_restart(settings.clone())?;
    drop(cloud_save_guard);
    state.finish_prepared_restart(app, prepared_restart)?;
    tray_items.set_hotkey_label(&settings.hotkey)?;
    crate::hotkeys::set_save_hotkey(&settings.hotkey)?;
    storage_settings.set_quota_bytes(quota_bytes);
    storage_settings.set_media_dir(media_dir);
    Ok(settings)
}

pub fn run() {
    let mut settings = AppSettings::load_or_default();
    let args: Vec<String> = std::env::args().collect();
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
        eprintln!("invalid settings, using defaults: {e}");
        settings = AppSettings::default();
    }

    let quota_bytes = quota_bytes_from_gb(settings.disk_quota_gb)
        .unwrap_or(Some(service::DEFAULT_DISK_QUOTA_BYTES));
    let media_dir = settings
        .media_dir_path()
        .unwrap_or_else(|_| service::default_clips_dir());
    let scope_dir = media_dir.clone();
    let audio_preview_scope_dir = crate::settings::audio_preview_cache_dir();
    let (cmd_tx, event_rx) = service::spawn(
        settings
            .to_service_options(lol_url.clone())
            .unwrap_or_else(|_| ServiceOptions::default()),
    );
    let hotkey =
        parse_hotkey(&settings.hotkey).unwrap_or_else(|_| parse_hotkey("Alt+F10").unwrap());

    tauri::Builder::default()
        .manage(RuntimeState::new(cmd_tx, settings.clone(), lol_url))
        .manage(MicTestState::default())
        .manage(crate::library::StorageSettings::new(quota_bytes, media_dir))
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
            extract_window_icon,
            platform_capabilities,
            memory_status,
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
            crate::library::list_clips,
            crate::library::clip_poster,
            crate::library::delete_clip,
            crate::library::rename_clip,
            crate::library::export_clip,
            crate::library::preview_clip_audio_tracks,
            crate::library::reveal_clip,
            crate::library::copy_clip_to_clipboard,
            crate::library::open_media_folder,
            crate::library::storage_status
        ])
        .setup(move |app| {
            if let Err(e) = app.global_shortcut().register(hotkey) {
                let message = format!("global save hotkey unavailable; continuing without it: {e}");
                eprintln!("{message}");
                let _ = app.handle().emit("error", message);
            }
            if let Err(e) = crate::hotkeys::install_save_hook(&settings.hotkey, {
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

            let save_item = MenuItem::with_id(
                app,
                "save",
                save_menu_text(&settings.hotkey),
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
                        if let Err(e) = open_main_window(app) {
                            eprintln!("open window: {e}");
                        }
                    }
                    "save" => {
                        app.state::<RuntimeState>().request_save();
                    }
                    "quit" => {
                        quit_app(app);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if should_open_on_tray_event(&event) {
                        if let Err(e) = open_main_window(tray.app_handle()) {
                            eprintln!("open window: {e}");
                        }
                    }
                })
                .build(app)?;

            pump_events(app.handle().clone(), event_rx);
            spawn_game_detector(app.handle().clone());

            // The main window is created hidden by default so autostart launches
            // don't flash it. Show it for normal launches.
            if !launched_by_autostart {
                if let Err(e) = open_main_window(app.handle()) {
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
            } if label == "main" => {
                api.prevent_close();
                match close_request_action(&app.state::<RuntimeState>().settings()) {
                    CloseRequestAction::Tray => {
                        if let Err(e) = send_main_window_to_tray(app, true) {
                            eprintln!("close to tray: {e}");
                        }
                    }
                    CloseRequestAction::Quit => quit_app(app),
                }
            }
            tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } => {
                api.prevent_exit();
            }
            tauri::RunEvent::Exit => {
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
                std::thread::sleep(Duration::from_secs(2));
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
    if let Some(window) = app.get_webview_window("main") {
        return reveal_main_window(
            || window.show(),
            || window.unminimize(),
            || window.set_focus(),
        );
    }

    let config = app
        .config()
        .app
        .windows
        .first()
        .ok_or_else(|| "missing main window config".to_string())?
        .clone();
    let window = WebviewWindowBuilder::from_config(app, &config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    reveal_main_window(
        || window.show(),
        || window.unminimize(),
        || window.set_focus(),
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

fn pump_events<R: Runtime>(handle: AppHandle<R>, event_rx: Receiver<Event>) {
    std::thread::spawn(move || {
        for event in event_rx {
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

fn save_menu_text(hotkey: &str) -> String {
    format!("Save Replay ({hotkey})")
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
    fn rebind_global_hotkey_skips_unregister_when_old_shortcut_is_stale() {
        let old_shortcut = parse_hotkey("Alt+F10").unwrap();
        let new_shortcut = parse_hotkey("Ctrl+F8").unwrap();
        let mut registered = Vec::new();
        let mut unregistered = Vec::new();

        let result = rebind_global_hotkey(
            old_shortcut,
            new_shortcut,
            false,
            |shortcut| {
                registered.push(shortcut);
                Ok::<_, &'static str>(())
            },
            |shortcut| {
                unregistered.push(shortcut);
                Err::<(), _>("old shortcut was never registered")
            },
        );

        assert!(result.is_ok());
        assert_eq!(registered, vec![new_shortcut]);
        assert!(unregistered.is_empty());
    }

    #[test]
    fn missing_unchanged_global_hotkey_is_retried_without_blocking_save() {
        let shortcut = parse_hotkey("Alt+F10").unwrap();
        let mut registered = Vec::new();

        let retry_error = retry_missing_global_hotkey(shortcut, false, |shortcut| {
            registered.push(shortcut);
            Err::<(), _>("still owned by another app")
        });

        assert_eq!(retry_error, Some("still owned by another app".to_string()));
        assert_eq!(registered, vec![shortcut]);
    }

    #[test]
    fn request_save_debounces_only_immediate_duplicate_triggers() {
        let (tx, rx) = mpsc::channel();
        let state = RuntimeState::new(tx, AppSettings::default(), None);

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
    fn recording_sender_survives_restart_option_error() {
        let (tx, _rx) = mpsc::channel();
        let mut inner = RuntimeInner {
            tx: Some(tx),
            settings: invalid_disk_replay_settings(),
            lol_url: None,
            active_game: None,
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
    fn recording_sender_survives_game_restart_option_error() {
        let (tx, _rx) = mpsc::channel();
        let mut inner = RuntimeInner {
            tx: Some(tx),
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
    fn preserve_backend_cloud_fields_keeps_upload_state_but_allows_preferences() {
        let mut frontend = AppSettings::default();
        frontend.cloud.host_url = "https://stale.example.com".into();
        frontend.cloud.public_url = Some("https://stale-public.example.com".into());
        frontend.cloud.connected_user_id = Some("stale-user".into());
        frontend.cloud.connected_username = Some("stale-name".into());
        frontend.cloud.credential_target = Some("stale-target".into());
        frontend.cloud.default_visibility = "public".into();
        frontend.cloud.delete_local_after_upload = true;
        frontend.cloud.auto_upload_rules = true;

        let mut backend = AppSettings::default();
        backend.cloud.host_url = "https://cloud.example.com".into();
        backend.cloud.public_url = Some("https://public.example.com".into());
        backend.cloud.connected_user_id = Some("user-1".into());
        backend.cloud.connected_username = Some("dain".into());
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

        preserve_backend_cloud_fields(&mut frontend, &backend);

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
            frontend.cloud.credential_target,
            backend.cloud.credential_target
        );
        assert_eq!(frontend.cloud.uploads, backend.cloud.uploads);
        assert_eq!(frontend.cloud.default_visibility, "public");
        assert!(frontend.cloud.delete_local_after_upload);
        assert!(frontend.cloud.auto_upload_rules);
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
