//! Tauri shell: tray, Alt+F10 global hotkey, status webview — all thin
//! wiring around the recorder service thread.

use std::sync::mpsc::Sender;
use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

use crate::service::{self, Cmd, Event, ServiceOptions};

struct CmdChannel(Mutex<Sender<Cmd>>);

impl CmdChannel {
    fn send(&self, cmd: Cmd) {
        if let Ok(tx) = self.0.lock() {
            let _ = tx.send(cmd);
        }
    }
}

#[tauri::command]
fn save_replay(state: tauri::State<CmdChannel>) {
    state.send(Cmd::Save);
}

pub fn run() {
    let mut opts = ServiceOptions::default();
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--window") {
        opts.window_title = args.get(i + 1).cloned();
    }
    if let Some(i) = args.iter().position(|a| a == "--lol-url") {
        opts.lol_url = args.get(i + 1).cloned();
    }

    let (cmd_tx, event_rx) = service::spawn(opts);
    let quit_tx = cmd_tx.clone();
    let hotkey_tx = cmd_tx.clone();
    let alt_f10 = Shortcut::new(Some(Modifiers::ALT), Code::F10);

    tauri::Builder::default()
        .manage(CmdChannel(Mutex::new(cmd_tx)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |_app, shortcut, event| {
                    if shortcut == &alt_f10 && event.state() == ShortcutState::Pressed {
                        let _ = hotkey_tx.send(Cmd::Save);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![save_replay])
        .setup(move |app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            app.global_shortcut().register(alt_f10)?;

            let save_item = MenuItem::with_id(app, "save", "Save Replay (Alt+F10)", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&save_item, &quit_item])?;
            TrayIconBuilder::with_id("clipline")
                .icon(tray_icon())
                .tooltip("Clipline — replay buffer")
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "save" => app.state::<CmdChannel>().send(Cmd::Save),
                    "quit" => {
                        app.state::<CmdChannel>().send(Cmd::Stop);
                        app.exit(0);
                    }
                    _ => {}
                })
                .build(app)?;

            // Pump service events into the webview.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                for event in event_rx {
                    let _ = match &event {
                        Event::Status { .. } => handle.emit("status", &event),
                        Event::Saved { .. } => handle.emit("saved", &event),
                        Event::Error { message } => handle.emit("error", message.clone()),
                    };
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("build tauri app")
        .run(move |_app, event| {
            if let tauri::RunEvent::Exit = event {
                let _ = quit_tx.send(Cmd::Stop);
            }
        });
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
