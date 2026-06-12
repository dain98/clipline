//! Tauri shell: tray, Alt+F10 global hotkey, status webview — all thin
//! wiring around the recorder service thread.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::service::{self, Cmd, Event, ServiceOptions};
use crate::settings::{parse_hotkey, quota_bytes_from_gb, AppSettings, CaptureMode};

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
    tx: Sender<Cmd>,
    settings: AppSettings,
    lol_url: Option<String>,
}

impl RuntimeState {
    fn new(tx: Sender<Cmd>, settings: AppSettings, lol_url: Option<String>) -> Self {
        Self(Mutex::new(RuntimeInner {
            tx,
            settings,
            lol_url,
        }))
    }

    fn send(&self, cmd: Cmd) {
        if let Ok(inner) = self.0.lock() {
            let _ = inner.tx.send(cmd);
        }
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
        let (old_tx, lol_url) = {
            let inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
            (inner.tx.clone(), inner.lol_url.clone())
        };
        let (tx, rx) = service::spawn(settings.to_service_options(lol_url.clone())?);
        pump_events(app, rx);
        let _ = old_tx.send(Cmd::Stop);
        let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
        inner.tx = tx;
        inner.settings = settings;
        Ok(())
    }
}

#[tauri::command]
fn save_replay(state: tauri::State<RuntimeState>) {
    state.send(Cmd::Save);
}

#[tauri::command]
fn get_settings(state: tauri::State<RuntimeState>) -> AppSettings {
    state.settings()
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
    let (cmd_tx, event_rx) = service::spawn(
        settings
            .to_service_options(lol_url.clone())
            .unwrap_or_else(|_| ServiceOptions::default()),
    );
    let hotkey =
        parse_hotkey(&settings.hotkey).unwrap_or_else(|_| parse_hotkey("Alt+F10").unwrap());

    tauri::Builder::default()
        .manage(RuntimeState::new(cmd_tx, settings.clone(), lol_url))
        .manage(crate::library::StorageSettings::new(quota_bytes))
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
            get_settings,
            save_settings,
            crate::library::list_clips,
            crate::library::delete_clip,
            crate::library::export_clip,
            crate::library::storage_status
        ])
        .setup(move |app| {
            app.global_shortcut().register(hotkey)?;

            let save_item = MenuItem::with_id(
                app,
                "save",
                save_menu_text(&settings.hotkey),
                true,
                None::<&str>,
            )?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&save_item, &quit_item])?;
            app.manage(TrayItems {
                save_item: save_item.clone(),
            });
            TrayIconBuilder::with_id("clipline")
                .icon(tray_icon())
                .tooltip("Clipline — replay buffer")
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "save" => app.state::<RuntimeState>().send(Cmd::Save),
                    "quit" => {
                        app.state::<RuntimeState>().send(Cmd::Stop);
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            pump_events(app.handle().clone(), event_rx);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build tauri app")
        .run(move |app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<RuntimeState>().send(Cmd::Stop);
            }
        });
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
}
