//! Tauri shell: tray, Alt+F10 global hotkey, status webview — all thin
//! wiring around the recorder service thread.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::games::{DetectedGame, GameWindowInfo};
use crate::service::{self, Cmd, Event, ServiceOptions};
use crate::settings::{parse_hotkey, quota_bytes_from_gb, AppSettings, CaptureMode};

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
            },
            None => Self {
                active: false,
                name: None,
                window_title: None,
                process_id: None,
                exe_name: None,
            },
        }
    }
}

#[tauri::command]
fn memory_status() -> Result<crate::memory::MemoryStatus, String> {
    crate::memory::current_process_tree_memory()
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
        if let Ok(mut tx) = self.0.lock() {
            if let Some(tx) = tx.take() {
                let _ = tx.send(());
            }
        }
    }
}

struct RuntimeState(Mutex<RuntimeInner>);

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
    /// Codecs WebView2 can decode, reported by the frontend. Drives the
    /// recorder's Automatic selection; H.264 is the always-safe default.
    decodable_codecs: Vec<service::Codec>,
}

impl RuntimeState {
    fn new(tx: Sender<Cmd>, settings: AppSettings, lol_url: Option<String>) -> Self {
        Self(Mutex::new(RuntimeInner {
            tx: Some(tx),
            settings,
            lol_url,
            active_game: None,
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
        if let Ok(mut inner) = self.0.lock() {
            inner.decodable_codecs = codecs;
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
        }
        Ok(opts)
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

    fn settings(&self) -> AppSettings {
        self.0
            .lock()
            .map(|inner| inner.settings.clone())
            .unwrap_or_default()
    }

    fn active_shortcut_matches(&self, shortcut: &Shortcut) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|inner| parse_hotkey(&inner.settings.hotkey).ok())
            .is_some_and(|active| &active == shortcut)
    }

    fn restart<R: Runtime>(&self, app: AppHandle<R>, settings: AppSettings) -> Result<(), String> {
        let (old_tx, next_options, cleared_active_game) = {
            let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            let old_tx = inner.tx.take();
            let cleared_active_game = inner.active_game.is_some()
                && !active_game_still_configured(&settings, inner.active_game.as_ref());
            if cleared_active_game {
                inner.active_game = None;
            }
            inner.settings = settings;
            let next_options = if old_tx.is_some() {
                Some(Self::options(&inner)?)
            } else {
                None
            };
            (old_tx, next_options, cleared_active_game)
        };
        if let Some(tx) = old_tx {
            let _ = tx.send(Cmd::Stop { announce: false });
        }
        if let Some(options) = next_options {
            let (tx, rx) = service::spawn(options);
            {
                let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
                inner.tx = Some(tx);
            }
            pump_events(app.clone(), rx);
        }
        if cleared_active_game {
            let _ = app.emit("game-detection", GameDetectionEvent::from_detected(None));
        }
        Ok(())
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
            rx
        };
        pump_events(app, rx);
        Ok(true)
    }

    fn stop_recording(&self) -> Result<bool, String> {
        let tx = {
            let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            inner.tx.take()
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
                if inner.active_game != detected {
                    inner.active_game = detected;
                    (None, None, true)
                } else {
                    (None, None, false)
                }
            } else {
                inner.active_game = detected;
                let old_tx = inner.tx.take();
                let next_options = if old_tx.is_some() {
                    Some(Self::options(&inner)?)
                } else {
                    None
                };
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
            }
            pump_events(app.clone(), rx);
        }
        if emit_event {
            let _ = app.emit("game-detection", event);
        }
        Ok(())
    }
}

fn same_game_window(current: Option<&DetectedGame>, next: Option<&DetectedGame>) -> bool {
    match (current, next) {
        (Some(current), Some(next)) => current.id == next.id && current.hwnd == next.hwnd,
        (None, None) => true,
        _ => false,
    }
}

fn active_game_still_configured(settings: &AppSettings, active: Option<&DetectedGame>) -> bool {
    let Some(active) = active else { return true };
    settings.games.auto_detect
        && settings
            .games
            .custom_games
            .iter()
            .any(|game| game.enabled && game.id == active.id)
}

#[tauri::command]
fn save_replay(state: tauri::State<RuntimeState>) {
    state.send(Cmd::Save);
}

#[tauri::command]
fn set_recording<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<RuntimeState>,
    recording: bool,
) -> Result<bool, String> {
    state.set_recording(app, recording)
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
#[tauri::command]
fn probe_encoders() -> Vec<service::EncoderOption> {
    service::available_encoder_options()
}

#[tauri::command]
fn list_game_windows() -> Vec<GameWindowInfo> {
    crate::games::list_game_windows()
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
    if settings.hotkey != old.hotkey {
        let old_shortcut = parse_hotkey(&old.hotkey)?;
        let new_shortcut = parse_hotkey(&settings.hotkey)?;
        app.global_shortcut()
            .register(new_shortcut)
            .map_err(|e| format!("register hotkey: {e}"))?;
        if let Err(e) = app.global_shortcut().unregister(old_shortcut) {
            let _ = app.global_shortcut().unregister(new_shortcut);
            return Err(format!("replace hotkey: {e}"));
        }
    }

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
    state.restart(app, settings.clone())?;
    tray_items.set_hotkey_label(&settings.hotkey)?;
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
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |_app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let state = _app.state::<RuntimeState>();
                        if state.active_shortcut_matches(shortcut) {
                            state.send(Cmd::Save);
                        }
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            save_replay,
            set_recording,
            get_settings,
            choose_media_folder,
            choose_replay_cache_folder,
            list_displays,
            list_audio_devices,
            probe_encoders,
            report_decode_support,
            list_game_windows,
            memory_status,
            start_microphone_test,
            stop_microphone_test,
            save_settings,
            crate::library::list_clips,
            crate::library::delete_clip,
            crate::library::export_clip,
            crate::library::reveal_clip,
            crate::library::copy_clip_to_clipboard,
            crate::library::open_media_folder,
            crate::library::storage_status
        ])
        .setup(move |app| {
            app.global_shortcut().register(hotkey)?;
            // Bound the asset protocol to the configured media folder so clips
            // under a custom root play back, while the static config scope stays
            // narrow (the default Videos/Clipline location).
            if let Err(e) = app.asset_protocol_scope().allow_directory(&scope_dir, true) {
                eprintln!("could not scope media folder {scope_dir:?} for playback: {e}");
            }

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
                        app.state::<RuntimeState>().send(Cmd::Save);
                    }
                    "quit" => {
                        app.state::<MicTestState>().stop();
                        app.state::<RuntimeState>()
                            .send(Cmd::Stop { announce: false });
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        if let Err(e) = open_main_window(tray.app_handle()) {
                            eprintln!("open window: {e}");
                        }
                    }
                })
                .build(app)?;

            pump_events(app.handle().clone(), event_rx);
            spawn_game_detector(app.handle().clone());
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
                app.state::<MicTestState>().stop();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.destroy();
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
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
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
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

fn pump_events<R: Runtime>(handle: AppHandle<R>, event_rx: Receiver<Event>) {
    std::thread::spawn(move || {
        for event in event_rx {
            let _ = match &event {
                Event::Status { .. } => handle.emit("status", &event),
                Event::Saved { .. } => handle.emit("saved", &event),
                Event::Error { message } => handle.emit("error", message.clone()),
            };
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
    use crate::settings::GameRecordingMode;

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
            decodable_codecs: vec![service::Codec::H264],
        };

        let opts = RuntimeState::options(&inner).unwrap();

        assert_eq!(opts.recording_mode, service::RecordingMode::FullSession);
        assert_eq!(
            opts.capture_source,
            service::CaptureSource::WindowHandle {
                hwnd: 42,
                title: "Game Window".into(),
            }
        );
    }
}
