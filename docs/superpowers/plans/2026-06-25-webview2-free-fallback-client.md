# WebView2-Free Fallback Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a full-parity fallback client that automatically opens the existing Clipline UI in the user's default browser when WebView2 is missing or dead.

**Architecture:** Keep the current HTML/CSS/JavaScript UI as the only product UI, and introduce a host bridge that can talk either to Tauri IPC or to a tokenized localhost server. Extract the Tauri command bodies into shared host modules so the WebView2 client and fallback browser client use the same Rust behavior, event stream, media validation, recorder state, cloud state, native actions, and update flow.

**Tech Stack:** Rust/Tauri 2, vanilla JavaScript, localhost HTTP/SSE fallback transport, Windows-only native APIs through existing `windows-sys`, existing `rfd` dialogs, existing recorder service thread, Rust source-contract tests, targeted unit tests, Playwright/manual runtime validation.

---

## File Structure

- Create `apps/clipline-app/ui/client-bridge.js`: host bridge used by both Tauri and fallback browser modes.
- Modify `apps/clipline-app/ui/index.html`: load `client-bridge.js` before `main.js`.
- Modify `apps/clipline-app/ui/main.js`: use `window.cliplineHost` instead of directly reading `window.__TAURI__`.
- Create `apps/clipline-app/src/fallback/mod.rs`: fallback module root.
- Create `apps/clipline-app/src/fallback/manifest.rs`: static command/event/media contract lists used by tests and route registration.
- Create `apps/clipline-app/src/fallback/security.rs`: loopback token generation and request authentication.
- Create `apps/clipline-app/src/fallback/media.rs`: opaque media-id registry and range-capable scoped file responses.
- Create `apps/clipline-app/src/fallback/server.rs`: localhost server, static UI routes, invoke routes, event route, media route.
- Create `apps/clipline-app/src/fallback/startup.rs`: preflight and health decision logic for when to prefer fallback.
- Create `apps/clipline-app/src/host/mod.rs`: shared command/control surface called by both Tauri commands and fallback routes.
- Create `apps/clipline-app/src/host/events.rs`: client event names, typed event payload conversion, and event fanout.
- Create `apps/clipline-app/src/host/native.rs`: native Windows actions that must work outside Tauri wrappers.
- Modify `apps/clipline-app/src/app.rs`: keep Tauri-specific setup/window/tray wiring, delegate command bodies into `host`, and launch fallback when WebView2 is unavailable.
- Modify `apps/clipline-app/src/cloud.rs`: move reusable cloud command bodies behind host-callable functions and emit upload progress through the shared event hub.
- Modify `apps/clipline-app/src/library.rs`: move reusable library command bodies behind host-callable functions and reuse path validation for fallback media.
- Modify `apps/clipline-app/src/main.rs`: register the new `fallback` and `host` modules.
- Modify `apps/clipline-app/Cargo.toml`: add Windows-gated HTTP/server dependencies if needed.
- Modify `apps/clipline-app/tests/ui_contract.rs`: add source-contract tests for bridge use, fallback command/event parity, direct Tauri access, and startup contract.
- Modify `README.md` and `handoff.md`: document fallback behavior and testing notes after implementation is verified.

Keep the plan checkboxes unticked in commits, matching this repo's convention.

---

### Task 1: Lock The Full-Parity Command And Event Contract

**Files:**
- Create: `apps/clipline-app/src/fallback/mod.rs`
- Create: `apps/clipline-app/src/fallback/manifest.rs`
- Modify: `apps/clipline-app/src/main.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write the failing source-contract tests**

Add these helpers and tests to `apps/clipline-app/tests/ui_contract.rs`:

```rust
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

    assert_eq!(events.len(), 8, "main.js event inventory changed; update this assertion and the fallback manifest together");
    for event in events {
        assert!(
            manifest.contains(&format!("\"{event}\"")),
            "fallback manifest must register frontend event {event}"
        );
    }
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract fallback_manifest_covers_every_frontend_command fallback_manifest_covers_every_frontend_event_listener
```

Expected: fail because `src/fallback/manifest.rs` does not exist yet.

- [ ] **Step 3: Add the fallback manifest module**

Create `apps/clipline-app/src/fallback/mod.rs`:

```rust
pub mod manifest;
```

Create `apps/clipline-app/src/fallback/manifest.rs`:

```rust
pub const FALLBACK_COMMANDS: &[&str] = &[
    "cache_cloud_clip_media",
    "check_for_updates",
    "choose_media_folder",
    "choose_replay_cache_folder",
    "clip_poster",
    "cloud_clip_thumbnail",
    "cloud_connect",
    "cloud_disconnect",
    "cloud_user_avatar",
    "cloud_user_profile",
    "copy_clip_to_clipboard",
    "delete_clip",
    "export_clip",
    "extract_window_icon",
    "frontend_ready",
    "get_autostart_status",
    "get_settings",
    "install_update",
    "list_audio_devices",
    "list_clips",
    "list_cloud_clips",
    "list_displays",
    "list_game_plugins",
    "list_game_windows",
    "memory_status",
    "minimize_main_window",
    "open_cloud_clip_url",
    "open_cloud_user_profile",
    "preview_clip_audio_tracks",
    "probe_encoders",
    "rename_clip",
    "report_decode_support",
    "reveal_clip",
    "save_replay",
    "save_settings",
    "set_recording",
    "start_microphone_test",
    "stop_microphone_test",
    "storage_status",
    "sync_cloud_clip_status",
    "upload_clip_to_cloud",
];

pub const FALLBACK_EVENTS: &[&str] = &[
    "cloud-upload-progress",
    "error",
    "game-detection",
    "mic-test",
    "mic-test-error",
    "mic-test-stopped",
    "saved",
    "status",
];

pub fn is_fallback_command(command: &str) -> bool {
    FALLBACK_COMMANDS.contains(&command)
}

pub fn is_fallback_event(event: &str) -> bool {
    FALLBACK_EVENTS.contains(&event)
}
```

Modify `apps/clipline-app/src/main.rs`:

```rust
#[cfg(windows)]
mod fallback;
```

Place it with the other Windows-only modules.

- [ ] **Step 4: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app --test ui_contract fallback_manifest_covers_every_frontend_command fallback_manifest_covers_every_frontend_event_listener
```

Expected: pass.

- [ ] **Step 5: Commit**

Run:

```powershell
git add apps/clipline-app/src/fallback/mod.rs apps/clipline-app/src/fallback/manifest.rs apps/clipline-app/src/main.rs apps/clipline-app/tests/ui_contract.rs
git commit -m "test(app): lock fallback client parity surface"
```

Expected: commit succeeds.

---

### Task 2: Introduce The Shared JavaScript Host Bridge

**Files:**
- Create: `apps/clipline-app/ui/client-bridge.js`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write the failing bridge contract**

Add this test to `apps/clipline-app/tests/ui_contract.rs`:

```rust
fn client_bridge_js() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/client-bridge.js");
    fs::read_to_string(path).expect("read ui/client-bridge.js")
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
        !js.contains("window.__TAURI__"),
        "main.js must use window.cliplineHost instead of direct Tauri globals"
    );
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract frontend_uses_host_bridge_instead_of_tauri_directly
```

Expected: fail because `ui/client-bridge.js` does not exist and `main.js` still reads `window.__TAURI__`.

- [ ] **Step 3: Add the Tauri-mode bridge**

Create `apps/clipline-app/ui/client-bridge.js`:

```javascript
(function () {
  function requireTauri() {
    if (!window.__TAURI__) {
      throw new Error("Clipline host bridge could not find Tauri or fallback transport");
    }
    return window.__TAURI__;
  }

  const tauri = window.__TAURI__;
  const fallbackConfig = window.__CLIPLINE_FALLBACK__;

  if (tauri) {
    const appWindow = tauri.window.getCurrentWindow();
    window.cliplineHost = {
      mode: "tauri",
      invoke: tauri.core.invoke,
      listen: tauri.event.listen,
      convertFileSrc: tauri.core.convertFileSrc,
      window: {
        minimize: () => tauri.core.invoke("minimize_main_window"),
        toggleMaximize: () => appWindow.toggleMaximize(),
        close: () => null,
      },
    };
    return;
  }

  if (!fallbackConfig) {
    requireTauri();
  }

  window.cliplineHost = {
    mode: "fallback",
    invoke(command, args = {}) {
      return fetch(`${fallbackConfig.baseUrl}/invoke/${encodeURIComponent(command)}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(args),
      }).then(async (response) => {
        const payload = await response.json().catch(() => ({}));
        if (!response.ok || payload.ok === false) {
          throw payload.error || `command failed: ${command}`;
        }
        return payload.value;
      });
    },
    listen(event, handler) {
      return fallbackSubscribe(fallbackConfig, event, handler);
    },
    convertFileSrc(path) {
      return `${fallbackConfig.baseUrl}/media/${encodeURIComponent(path)}`;
    },
    window: {
      minimize: () => fallbackWindowAction(fallbackConfig, "minimize"),
      toggleMaximize: () => fallbackWindowAction(fallbackConfig, "toggle_maximize"),
      close: () => fallbackWindowAction(fallbackConfig, "close"),
    },
  };

  function fallbackWindowAction(config, action) {
    return fetch(`${config.baseUrl}/window/${action}`, { method: "POST" }).then(() => null);
  }

  function fallbackSubscribe(config, event, handler) {
    const source = new EventSource(`${config.baseUrl}/events?name=${encodeURIComponent(event)}`);
    source.addEventListener(event, (message) => {
      handler({ event, payload: JSON.parse(message.data) });
    });
    source.onerror = () => {
      const error = document.getElementById("error");
      if (error) error.textContent = "Clipline fallback event stream disconnected";
    };
    return Promise.resolve(() => source.close());
  }
})();
```

- [ ] **Step 4: Load the bridge before main.js**

In `apps/clipline-app/ui/index.html`, add this script tag immediately before the existing `main.js` script:

```html
  <script src="client-bridge.js"></script>
```

- [ ] **Step 5: Change `main.js` to use the bridge**

Replace the current top-level Tauri destructuring in `apps/clipline-app/ui/main.js`:

```javascript
const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const appWindow = window.__TAURI__.window.getCurrentWindow();
```

with:

```javascript
const { invoke, listen, convertFileSrc } = window.cliplineHost;
const appWindow = window.cliplineHost.window;
```

Keep the existing titlebar listeners; the bridge provides the same methods.

- [ ] **Step 6: Verify GREEN and syntax**

Run:

```powershell
cargo test -p clipline-app --test ui_contract frontend_uses_host_bridge_instead_of_tauri_directly frontend_reports_webview_readiness_to_native_shell
node --check apps/clipline-app/ui/client-bridge.js
node --check apps/clipline-app/ui/main.js
```

Expected: both tests pass and both `node --check` commands exit successfully.

- [ ] **Step 7: Commit**

Run:

```powershell
git add apps/clipline-app/ui/client-bridge.js apps/clipline-app/ui/index.html apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(app): add shared client host bridge"
```

Expected: commit succeeds.

---

### Task 3: Add A Shared Event Hub

**Files:**
- Create: `apps/clipline-app/src/host/mod.rs`
- Create: `apps/clipline-app/src/host/events.rs`
- Modify: `apps/clipline-app/src/main.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Test: `apps/clipline-app/src/host/events.rs`

- [ ] **Step 1: Write failing event hub unit tests**

Create `apps/clipline-app/src/host/mod.rs`:

```rust
pub mod events;
```

Create `apps/clipline-app/src/host/events.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_hub_fans_out_to_multiple_subscribers() {
        let hub = ClientEventHub::default();
        let first = hub.subscribe();
        let second = hub.subscribe();

        hub.emit(ClientEvent::new("status", serde_json::json!({"recording": true})));

        assert_eq!(first.try_recv().unwrap().name, "status");
        assert_eq!(second.try_recv().unwrap().name, "status");
    }

    #[test]
    fn event_hub_drops_disconnected_subscribers() {
        let hub = ClientEventHub::default();
        let dropped = hub.subscribe();
        drop(dropped);
        let live = hub.subscribe();

        hub.emit(ClientEvent::new("saved", serde_json::json!({"path": "clip.mp4"})));

        assert_eq!(live.try_recv().unwrap().name, "saved");
        assert_eq!(hub.subscriber_count(), 1);
    }
}
```

Modify `apps/clipline-app/src/main.rs` to add:

```rust
#[cfg(windows)]
mod host;
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app host::events::tests::event_hub_fans_out_to_multiple_subscribers host::events::tests::event_hub_drops_disconnected_subscribers
```

Expected: fail because `ClientEventHub` and `ClientEvent` do not exist.

- [ ] **Step 3: Implement the event hub**

Replace `apps/clipline-app/src/host/events.rs` content above the tests with:

```rust
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ClientEvent {
    pub name: &'static str,
    pub payload: serde_json::Value,
}

impl ClientEvent {
    pub fn new(name: &'static str, payload: serde_json::Value) -> Self {
        Self { name, payload }
    }
}

#[derive(Default)]
pub struct ClientEventHub {
    subscribers: Mutex<Vec<Sender<ClientEvent>>>,
}

impl ClientEventHub {
    pub fn subscribe(&self) -> Receiver<ClientEvent> {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(tx);
        }
        rx
    }

    pub fn emit(&self, event: ClientEvent) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|subscriber| subscriber.send(event.clone()).is_ok());
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .map(|subscribers| subscribers.len())
            .unwrap_or(0)
    }
}
```

Keep the tests from Step 1 below this implementation.

- [ ] **Step 4: Route recorder events through the hub while preserving Tauri emits**

In `apps/clipline-app/src/app.rs`, change the app setup to manage an event hub:

```rust
.manage(crate::host::events::ClientEventHub::default())
```

Add it next to the existing `RuntimeState`, `MicTestState`, and `StorageSettings` state.

Change `pump_events` to read the hub before emitting:

```rust
fn pump_events<R: Runtime>(handle: AppHandle<R>, event_rx: Receiver<Event>) {
    std::thread::Builder::new()
        .name("clipline-event-pump".into())
        .spawn(move || {
            let hub = handle.state::<crate::host::events::ClientEventHub>();
            for event in event_rx {
                match &event {
                    Event::Status { .. } => {
                        hub.emit(crate::host::events::ClientEvent::new(
                            "status",
                            serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
                        ));
                        let _ = handle.emit("status", &event);
                    }
                    Event::Saved { .. } => {
                        hub.emit(crate::host::events::ClientEvent::new(
                            "saved",
                            serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
                        ));
                        let _ = handle.emit("saved", &event);
                    }
                    Event::Error { message } => {
                        hub.emit(crate::host::events::ClientEvent::new(
                            "error",
                            serde_json::json!(message),
                        ));
                        let _ = handle.emit("error", message.clone());
                    }
                }
            }
        })
        .expect("spawn event pump");
}
```

Use the current `pump_events` function body as the replacement site.

- [ ] **Step 5: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app host::events::tests::event_hub_fans_out_to_multiple_subscribers host::events::tests::event_hub_drops_disconnected_subscribers
```

Expected: pass.

- [ ] **Step 6: Commit**

Run:

```powershell
git add apps/clipline-app/src/host/mod.rs apps/clipline-app/src/host/events.rs apps/clipline-app/src/main.rs apps/clipline-app/src/app.rs
git commit -m "feat(app): add shared client event hub"
```

Expected: commit succeeds.

---

### Task 4: Add Fallback Startup Decisions And Debug Flags

**Files:**
- Create: `apps/clipline-app/src/fallback/startup.rs`
- Modify: `apps/clipline-app/src/fallback/mod.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing startup decision tests**

Create `apps/clipline-app/src/fallback/startup.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_flag_selects_fallback() {
        let args = vec!["clipline-app.exe".to_string(), "--force-fallback-client".to_string()];
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
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app fallback::startup::tests::force_flag_selects_fallback fallback::startup::tests::missing_webview_selects_fallback fallback::startup::tests::available_webview_uses_tauri
```

Expected: fail because the decision types and function do not exist.

- [ ] **Step 3: Implement startup decision types**

Add this above the tests in `apps/clipline-app/src/fallback/startup.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebviewPreflight {
    Available,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackLaunchPreference {
    UseTauri,
    StartFallback,
}

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

pub fn requested_fallback_port(args: &[String]) -> Option<u16> {
    args.windows(2)
        .find(|window| window[0] == "--fallback-port")
        .and_then(|window| window[1].parse::<u16>().ok())
}
```

Modify `apps/clipline-app/src/fallback/mod.rs`:

```rust
pub mod manifest;
pub mod startup;
```

- [ ] **Step 4: Add a source contract for the debug flag**

Add to `apps/clipline-app/tests/ui_contract.rs`:

```rust
#[test]
fn app_exposes_force_fallback_client_flag() {
    let startup = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fallback/startup.rs"),
    )
    .expect("read fallback startup");

    assert!(
        startup.contains("--force-fallback-client"),
        "fallback implementation must expose a debug flag for forced fallback runtime testing"
    );
}
```

- [ ] **Step 5: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app fallback::startup::tests::force_flag_selects_fallback fallback::startup::tests::missing_webview_selects_fallback fallback::startup::tests::available_webview_uses_tauri
cargo test -p clipline-app --test ui_contract app_exposes_force_fallback_client_flag
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

Run:

```powershell
git add apps/clipline-app/src/fallback/mod.rs apps/clipline-app/src/fallback/startup.rs apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(app): model fallback startup selection"
```

Expected: commit succeeds.

---

### Task 5: Build Token Security And The Loopback Server Skeleton

**Files:**
- Create: `apps/clipline-app/src/fallback/security.rs`
- Create: `apps/clipline-app/src/fallback/server.rs`
- Modify: `apps/clipline-app/src/fallback/mod.rs`
- Modify: `apps/clipline-app/Cargo.toml`

- [ ] **Step 1: Add Windows-gated server dependencies**

In `apps/clipline-app/Cargo.toml`, under `[target.'cfg(windows)'.dependencies]`, add:

```toml
axum = { version = "0.8", default-features = false, features = ["tokio", "http1", "json"] }
tokio = { workspace = true, features = ["net", "sync"] }
```

Keep the existing `tokio = { workspace = true }` line by replacing it with the line above, not by adding a duplicate.

- [ ] **Step 2: Write failing token tests**

Create `apps/clipline-app/src/fallback/security.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_url_safe_and_unique() {
        let first = FallbackToken::generate_for_tests(0x1234_5678_9abc_def0);
        let second = FallbackToken::generate_for_tests(0xfedc_ba98_7654_3210);

        assert_ne!(first.as_str(), second.as_str());
        assert!(first.as_str().chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert!(first.matches(first.as_str()));
        assert!(!first.matches(second.as_str()));
    }
}
```

- [ ] **Step 3: Verify RED**

Run:

```powershell
cargo test -p clipline-app fallback::security::tests::generated_tokens_are_url_safe_and_unique
```

Expected: fail because `FallbackToken` does not exist.

- [ ] **Step 4: Implement token generation**

Add above the tests in `apps/clipline-app/src/fallback/security.rs`:

```rust
#[derive(Debug, Clone)]
pub struct FallbackToken(String);

impl FallbackToken {
    pub fn generate() -> Result<Self, String> {
        let mut seed = [0u8; 16];
        fill_random_bytes(&mut seed)?;
        Ok(Self(base64_url_no_pad(&seed)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn matches(&self, candidate: &str) -> bool {
        self.0 == candidate
    }

    #[cfg(test)]
    fn generate_for_tests(seed: u64) -> Self {
        Self(base64_url_no_pad(&seed.to_le_bytes()))
    }
}

fn fill_random_bytes(bytes: &mut [u8]) -> Result<(), String> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };

    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err(format!("generate fallback token: BCryptGenRandom failed with {status:#x}"));
    }
    Ok(())
}

fn base64_url_no_pad(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
```

Add the required Windows feature in `apps/clipline-app/Cargo.toml` under `windows-sys` features:

```toml
"Win32_Security_Cryptography",
```

- [ ] **Step 5: Add server skeleton compile point**

Create `apps/clipline-app/src/fallback/server.rs`:

```rust
use std::net::SocketAddr;

use super::security::FallbackToken;

#[derive(Debug, Clone)]
pub struct FallbackServerInfo {
    pub addr: SocketAddr,
    pub token: String,
    pub base_url: String,
}

pub async fn start_fallback_server(port: Option<u16>) -> Result<FallbackServerInfo, String> {
    let token = FallbackToken::generate()?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port.unwrap_or(0)));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind fallback server: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("read fallback server address: {e}"))?;

    let token_string = token.as_str().to_string();
    let base_url = format!("http://{addr}/{token_string}");

    tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/health",
            axum::routing::get(|| async { axum::Json(serde_json::json!({"ok": true})) }),
        );
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("fallback server stopped: {e}");
        }
    });

    Ok(FallbackServerInfo {
        addr,
        token: token_string,
        base_url,
    })
}
```

Modify `apps/clipline-app/src/fallback/mod.rs`:

```rust
pub mod manifest;
pub mod security;
pub mod server;
pub mod startup;
```

- [ ] **Step 6: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app fallback::security::tests::generated_tokens_are_url_safe_and_unique
cargo check -p clipline-app
```

Expected: token test passes and app crate checks.

- [ ] **Step 7: Commit**

Run:

```powershell
git add apps/clipline-app/Cargo.toml Cargo.lock apps/clipline-app/src/fallback/mod.rs apps/clipline-app/src/fallback/security.rs apps/clipline-app/src/fallback/server.rs
git commit -m "feat(app): add fallback loopback server skeleton"
```

Expected: commit succeeds.

---

### Task 6: Serve The Shared UI And Fallback Bridge Config

**Files:**
- Modify: `apps/clipline-app/src/fallback/server.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write failing static route contract**

Add to `apps/clipline-app/tests/ui_contract.rs`:

```rust
#[test]
fn fallback_server_serves_shared_ui_assets() {
    let server = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fallback/server.rs"),
    )
    .expect("read fallback server");

    assert!(server.contains("index.html"), "fallback server must serve the shared index.html");
    assert!(server.contains("client-bridge.js"), "fallback server must serve the shared client bridge");
    assert!(server.contains("__CLIPLINE_FALLBACK__"), "fallback index must inject fallback bridge config");
    assert!(server.contains("/{token}/ui/{*asset}"), "fallback server must serve nested UI assets");
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract fallback_server_serves_shared_ui_assets
```

Expected: fail because the server does not serve shared UI assets yet.

- [ ] **Step 3: Implement static UI serving**

In `apps/clipline-app/src/fallback/server.rs`, add:

```rust
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
struct FallbackServerState {
    token: String,
    ui_dir: PathBuf,
    base_url: String,
}

fn ui_dir() -> Result<PathBuf, String> {
    Ok(std::env::current_exe()
        .map_err(|e| format!("read current exe path: {e}"))?
        .parent()
        .ok_or_else(|| "current exe has no parent directory".to_string())?
        .join("ui"))
}

fn dev_ui_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui")
}

async fn index(
    State(state): State<Arc<FallbackServerState>>,
    Path(token): Path<String>,
) -> Response {
    if token != state.token {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let path = state.ui_dir.join("index.html");
    let Ok(mut html) = std::fs::read_to_string(&path) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "read index.html").into_response();
    };
    let config = format!(
        r#"<script>window.__CLIPLINE_FALLBACK__ = {{ baseUrl: "{}" }};</script>"#,
        state.base_url
    );
    html = html.replace("<head>", &format!("<head>\n  {config}"));
    Html(html).into_response()
}

async fn ui_asset(
    State(state): State<Arc<FallbackServerState>>,
    Path((token, asset)): Path<(String, String)>,
) -> Response {
    if token != state.token {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if asset.contains("..") || asset.contains('\\') || asset.starts_with('/') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let path = state.ui_dir.join(&asset);
    let Ok(bytes) = std::fs::read(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match path.extension().and_then(|ext| ext.to_str()) {
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    };
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type),
    );
    response
}
```

Then replace the `Router::new()` in `start_fallback_server` with:

```rust
let ui_dir = ui_dir().unwrap_or_else(|_| dev_ui_dir());
let state = Arc::new(FallbackServerState {
    token: token_string.clone(),
    ui_dir,
    base_url: base_url.clone(),
});
let app = axum::Router::new()
    .route("/{token}/", get(index))
    .route("/{token}/ui/{*asset}", get(ui_asset))
    .route(
        "/health",
        get(|| async { axum::Json(serde_json::json!({"ok": true})) }),
    )
    .with_state(state);
```

This task intentionally validates the token even for static UI assets so every fallback URL remains scoped to the generated launch token.

- [ ] **Step 4: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app --test ui_contract fallback_server_serves_shared_ui_assets
cargo check -p clipline-app
```

Expected: test passes and app crate checks.

- [ ] **Step 5: Commit**

Run:

```powershell
git add apps/clipline-app/src/fallback/server.rs apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(app): serve shared UI from fallback host"
```

Expected: commit succeeds.

---

### Task 7: Add Token-Checked Invoke And Event Routes

**Files:**
- Modify: `apps/clipline-app/src/fallback/server.rs`
- Modify: `apps/clipline-app/src/fallback/security.rs`
- Modify: `apps/clipline-app/src/host/events.rs`

- [ ] **Step 1: Write failing route security tests**

Add to `apps/clipline-app/src/fallback/security.rs` tests:

```rust
#[test]
fn token_guard_accepts_only_exact_token() {
    let token = FallbackToken::generate_for_tests(7);

    assert_eq!(token_guard(&token, token.as_str()), Ok(()));
    assert_eq!(token_guard(&token, "wrong"), Err(FallbackAuthError::InvalidToken));
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app fallback::security::tests::token_guard_accepts_only_exact_token
```

Expected: fail because `token_guard` and `FallbackAuthError` do not exist.

- [ ] **Step 3: Implement token guard**

Add to `apps/clipline-app/src/fallback/security.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackAuthError {
    InvalidToken,
}

pub fn token_guard(token: &FallbackToken, candidate: &str) -> Result<(), FallbackAuthError> {
    if token.matches(candidate) {
        Ok(())
    } else {
        Err(FallbackAuthError::InvalidToken)
    }
}
```

- [ ] **Step 4: Add invoke and event route skeletons**

In `apps/clipline-app/src/fallback/server.rs`, add:

```rust
use axum::extract::Query;
use serde::Deserialize;

#[derive(Deserialize)]
struct EventQuery {
    name: Option<String>,
}

async fn invoke(
    State(state): State<Arc<FallbackServerState>>,
    Path((token, command)): Path<(String, String)>,
    axum::Json(args): axum::Json<serde_json::Value>,
) -> Response {
    if token != state.token {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({"ok": false, "error": "invalid fallback token"})),
        )
            .into_response();
    }
    if !super::manifest::is_fallback_command(&command) {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"ok": false, "error": format!("unknown command: {command}")})),
        )
            .into_response();
    }
    (
        StatusCode::NOT_IMPLEMENTED,
        axum::Json(serde_json::json!({"ok": false, "error": format!("fallback command not wired yet: {command}"), "args": args})),
    )
        .into_response()
}

async fn events(
    State(state): State<Arc<FallbackServerState>>,
    Path(token): Path<String>,
    Query(query): Query<EventQuery>,
) -> Response {
    if token != state.token {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(name) = query.name.as_deref() {
        if !super::manifest::is_fallback_event(name) {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    let body = "event: status\ndata: {\"recording\":false}\n\n";
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
}
```

Add routes:

```rust
.route("/{token}/invoke/{command}", axum::routing::post(invoke))
.route("/{token}/events", get(events))
```

- [ ] **Step 5: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app fallback::security::tests::token_guard_accepts_only_exact_token
cargo check -p clipline-app
```

Expected: test passes and app crate checks.

- [ ] **Step 6: Commit**

Run:

```powershell
git add apps/clipline-app/src/fallback/security.rs apps/clipline-app/src/fallback/server.rs apps/clipline-app/src/host/events.rs
git commit -m "feat(app): add fallback invoke and event routes"
```

Expected: commit succeeds.

---

### Task 8: Add Secure Media Registration And `convertFileSrc` Support

**Files:**
- Create: `apps/clipline-app/src/fallback/media.rs`
- Modify: `apps/clipline-app/src/fallback/mod.rs`
- Modify: `apps/clipline-app/src/fallback/server.rs`
- Modify: `apps/clipline-app/ui/client-bridge.js`

- [ ] **Step 1: Write failing media registry tests**

Create `apps/clipline-app/src/fallback/media.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_registry_returns_stable_opaque_ids() {
        let registry = MediaRegistry::default();
        let path = std::path::PathBuf::from(r"C:\Videos\Clipline\clip.mp4");

        let first = registry.register(path.clone(), MediaKind::Clip);
        let second = registry.register(path, MediaKind::Clip);

        assert_eq!(first, second);
        assert_eq!(registry.lookup(&first).unwrap().kind, MediaKind::Clip);
    }

    #[test]
    fn media_registry_rejects_unknown_ids() {
        let registry = MediaRegistry::default();

        assert!(registry.lookup("missing").is_none());
    }
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app fallback::media::tests::media_registry_returns_stable_opaque_ids fallback::media::tests::media_registry_rejects_unknown_ids
```

Expected: fail because media registry types do not exist.

- [ ] **Step 3: Implement media registry**

Add above the tests in `apps/clipline-app/src/fallback/media.rs`:

```rust
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Clip,
    Poster,
    AudioPreview,
    CloudCache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEntry {
    pub id: String,
    pub path: PathBuf,
    pub kind: MediaKind,
}

#[derive(Default)]
pub struct MediaRegistry {
    entries: Mutex<BTreeMap<String, MediaEntry>>,
    reverse: Mutex<BTreeMap<(PathBuf, MediaKind), String>>,
}

impl MediaRegistry {
    pub fn register(&self, path: PathBuf, kind: MediaKind) -> String {
        let key = (path.clone(), kind);
        if let Ok(reverse) = self.reverse.lock() {
            if let Some(id) = reverse.get(&key) {
                return id.clone();
            }
        }

        let id = format!("m{}", stable_media_hash(&path, kind));
        let entry = MediaEntry {
            id: id.clone(),
            path,
            kind,
        };
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(id.clone(), entry.clone());
        }
        if let Ok(mut reverse) = self.reverse.lock() {
            reverse.insert((entry.path, entry.kind), id.clone());
        }
        id
    }

    pub fn lookup(&self, id: &str) -> Option<MediaEntry> {
        self.entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(id).cloned())
    }
}

fn stable_media_hash(path: &std::path::Path, kind: MediaKind) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    kind.hash(&mut hasher);
    hasher.finish()
}

impl std::hash::Hash for MediaKind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (*self as u8).hash(state);
    }
}
```

Modify `apps/clipline-app/src/fallback/mod.rs`:

```rust
pub mod media;
```

- [ ] **Step 4: Wire fallback `convertFileSrc` to opaque media URLs**

In `apps/clipline-app/ui/client-bridge.js`, replace fallback `convertFileSrc`:

```javascript
convertFileSrc(path) {
  return `${fallbackConfig.baseUrl}/media/${encodeURIComponent(path)}`;
},
```

with:

```javascript
convertFileSrc(path) {
  return `${fallbackConfig.baseUrl}/media-path?path=${encodeURIComponent(path)}`;
},
```

This route will register the path server-side and redirect to the opaque media URL in the same task.

- [ ] **Step 5: Add media route skeletons**

In `apps/clipline-app/src/fallback/server.rs`, include `MediaRegistry` in `FallbackServerState`:

```rust
media: Arc<super::media::MediaRegistry>,
```

Initialize it:

```rust
media: Arc::new(super::media::MediaRegistry::default()),
```

Add handlers:

```rust
#[derive(Deserialize)]
struct MediaPathQuery {
    path: String,
}

async fn media_path(
    State(state): State<Arc<FallbackServerState>>,
    Query(query): Query<MediaPathQuery>,
) -> Response {
    let id = state
        .media
        .register(std::path::PathBuf::from(query.path), super::media::MediaKind::Clip);
    (
        StatusCode::FOUND,
        [(header::LOCATION, format!("{}/media/{id}", state.base_url))],
    )
        .into_response()
}

async fn media(
    State(state): State<Arc<FallbackServerState>>,
    Path((token, id)): Path<(String, String)>,
) -> Response {
    if token != state.token {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(entry) = state.media.lookup(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match std::fs::read(&entry.path) {
        Ok(bytes) => bytes.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
```

Add routes:

```rust
.route("/{token}/media-path", get(media_path))
.route("/{token}/media/{id}", get(media))
```

This is intentionally not the final path validator. Task 9 replaces raw path registration with host-validated registration.

- [ ] **Step 6: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app fallback::media::tests::media_registry_returns_stable_opaque_ids fallback::media::tests::media_registry_rejects_unknown_ids
node --check apps/clipline-app/ui/client-bridge.js
cargo check -p clipline-app
```

Expected: tests pass, JavaScript parses, app crate checks.

- [ ] **Step 7: Commit**

Run:

```powershell
git add apps/clipline-app/src/fallback/mod.rs apps/clipline-app/src/fallback/media.rs apps/clipline-app/src/fallback/server.rs apps/clipline-app/ui/client-bridge.js
git commit -m "feat(app): add fallback media URL registry"
```

Expected: commit succeeds.

---

### Task 9: Extract Library Commands Into Shared Host Functions

**Files:**
- Modify: `apps/clipline-app/src/host/mod.rs`
- Create: `apps/clipline-app/src/host/library.rs`
- Modify: `apps/clipline-app/src/library.rs`
- Modify: `apps/clipline-app/src/fallback/server.rs`
- Test: `apps/clipline-app/src/host/library.rs`

- [ ] **Step 1: Write failing host library test**

Create `apps/clipline-app/src/host/library.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_validates_library_media_kind_from_extension() {
        assert_eq!(media_kind_for_path("clip.mp4"), Some(HostMediaKind::Clip));
        assert_eq!(media_kind_for_path("poster.png"), Some(HostMediaKind::Poster));
        assert_eq!(media_kind_for_path("notes.txt"), None);
    }
}
```

Modify `apps/clipline-app/src/host/mod.rs`:

```rust
pub mod events;
pub mod library;
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app host::library::tests::host_validates_library_media_kind_from_extension
```

Expected: fail because `HostMediaKind` and `media_kind_for_path` do not exist.

- [ ] **Step 3: Implement host library media helpers**

Add above tests in `apps/clipline-app/src/host/library.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostMediaKind {
    Clip,
    Poster,
    AudioPreview,
    CloudCache,
}

pub fn media_kind_for_path(path: &str) -> Option<HostMediaKind> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())?;
    match ext.to_ascii_lowercase().as_str() {
        "mp4" => Some(HostMediaKind::Clip),
        "png" | "jpg" | "jpeg" | "webp" => Some(HostMediaKind::Poster),
        _ => None,
    }
}
```

- [ ] **Step 4: Extract pure command bodies from `library.rs`**

In `apps/clipline-app/src/library.rs`, make these existing helpers public within the crate:

```rust
pub(crate) fn list_clips_from_dir(dir: PathBuf) -> Result<Vec<ClipInfo>, String>
pub(crate) fn storage_status_for_dir(dir: PathBuf, quota_bytes: Option<u64>) -> Result<StorageInfo, String>
pub(crate) fn export_clip_file(source: PathBuf, start_s: f64, end_s: f64) -> Result<ExportedClipInfo, String>
pub(crate) fn preview_clip_audio_tracks_file(source: PathBuf, display_path: String, audio_track_ids: Vec<String>) -> Result<String, String>
pub(crate) fn open_folder_path(dir: &Path) -> Result<(), String>
pub(crate) fn copy_file_to_clipboard(path: &Path) -> Result<(), String>
```

Do not change their bodies in this task.

- [ ] **Step 5: Add host wrappers for library commands**

In `apps/clipline-app/src/host/library.rs`, add wrappers:

```rust
pub fn list_clips(settings: &crate::library::StorageSettings) -> Result<Vec<crate::library::ClipInfo>, String> {
    crate::library::list_clips_from_dir(settings.clips_dir()?)
}

pub fn storage_status(settings: &crate::library::StorageSettings) -> Result<crate::library::StorageInfo, String> {
    crate::library::storage_status_for_dir(settings.clips_dir()?, settings.quota_bytes())
}

pub fn reveal_clip(path: &str, settings: &crate::library::StorageSettings) -> Result<(), String> {
    let target = crate::library::validate_clip_path(settings, path)?;
    let dir = target
        .parent()
        .ok_or_else(|| "clip has no containing folder".to_string())?;
    crate::library::open_folder_path(dir)
}
```

If `StorageSettings::clips_dir` is private, make it `pub(crate)` in `library.rs`.

- [ ] **Step 6: Route Tauri wrappers through host wrappers**

In `apps/clipline-app/src/library.rs`, update the Tauri command bodies for `list_clips`, `storage_status`, and `reveal_clip` to call `crate::host::library::*` wrappers. Keep the same signatures and async behavior.

- [ ] **Step 7: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app host::library::tests::host_validates_library_media_kind_from_extension
cargo test -p clipline-app --test ui_contract audio_preview_command_scopes_generated_preview_files
cargo check -p clipline-app
```

Expected: tests pass and app crate checks.

- [ ] **Step 8: Commit**

Run:

```powershell
git add apps/clipline-app/src/host/mod.rs apps/clipline-app/src/host/library.rs apps/clipline-app/src/library.rs apps/clipline-app/src/fallback/server.rs
git commit -m "refactor(app): share library commands with fallback host"
```

Expected: commit succeeds.

---

### Task 10: Extract Core Recorder And Settings Commands Into Shared Host Functions

**Files:**
- Create: `apps/clipline-app/src/host/runtime.rs`
- Modify: `apps/clipline-app/src/host/mod.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/src/fallback/server.rs`

- [ ] **Step 1: Write failing runtime unit tests**

Create `apps/clipline-app/src/host/runtime.rs`:

```rust
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
```

Modify `apps/clipline-app/src/host/mod.rs`:

```rust
pub mod events;
pub mod library;
pub mod runtime;
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app host::runtime::tests::command_result_serializes_success_and_error_for_fallback_bridge
```

Expected: fail because `FallbackCommandResult` does not exist.

- [ ] **Step 3: Implement fallback command result**

Add above tests in `apps/clipline-app/src/host/runtime.rs`:

```rust
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
```

- [ ] **Step 4: Extract simple app commands**

In `apps/clipline-app/src/app.rs`, add host-callable methods or free functions for these commands, then call them from the existing Tauri command wrappers:

```rust
pub(crate) fn host_save_replay(state: &RuntimeState) {
    state.request_save();
}

pub(crate) fn host_get_settings(state: &RuntimeState) -> AppSettings {
    state.settings()
}

pub(crate) fn host_set_recording<R: Runtime>(
    state: &RuntimeState,
    app: AppHandle<R>,
    recording: bool,
) -> Result<bool, String> {
    state.set_recording(app, recording)
}

pub(crate) fn host_report_decode_support(state: &RuntimeState, codecs: &[String]) {
    state.set_decodable_codecs(codecs);
}
```

Then update existing Tauri command bodies to call these helpers. This is a temporary bridge; later tasks can move `RuntimeState` fully into `host`.

- [ ] **Step 5: Add fallback dispatch for simple commands**

In `apps/clipline-app/src/fallback/server.rs`, replace the `NOT_IMPLEMENTED` response for these commands with JSON success:

```rust
match command.as_str() {
    "frontend_ready" => {
        return axum::Json(crate::host::runtime::FallbackCommandResult::ok(serde_json::Value::Null)).into_response();
    }
    "save_replay" => {
        return axum::Json(crate::host::runtime::FallbackCommandResult::ok(serde_json::Value::Null)).into_response();
    }
    _ => {}
}
```

This task proves dispatch shape. Later tasks attach real runtime state to fallback routes and replace these temporary no-op branches.

- [ ] **Step 6: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app host::runtime::tests::command_result_serializes_success_and_error_for_fallback_bridge
cargo check -p clipline-app
```

Expected: test passes and app crate checks.

- [ ] **Step 7: Commit**

Run:

```powershell
git add apps/clipline-app/src/host/mod.rs apps/clipline-app/src/host/runtime.rs apps/clipline-app/src/app.rs apps/clipline-app/src/fallback/server.rs
git commit -m "refactor(app): share core runtime command shapes"
```

Expected: commit succeeds.

---

### Task 11: Wire Real Host State Into The Fallback Server

**Files:**
- Modify: `apps/clipline-app/src/fallback/server.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/src/host/runtime.rs`

- [ ] **Step 1: Write failing host state test**

Add to `apps/clipline-app/src/host/runtime.rs` tests:

```rust
#[test]
fn fallback_context_exposes_initial_settings() {
    let context = FallbackHostContext::for_tests(crate::settings::AppSettings::default());

    assert_eq!(context.settings().hotkey, "Alt+F10");
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app host::runtime::tests::fallback_context_exposes_initial_settings
```

Expected: fail because `FallbackHostContext` does not exist.

- [ ] **Step 3: Implement fallback host context**

In `apps/clipline-app/src/host/runtime.rs`, add:

```rust
use std::sync::Arc;

pub struct FallbackHostContext {
    settings: std::sync::Mutex<crate::settings::AppSettings>,
    events: Arc<crate::host::events::ClientEventHub>,
}

impl FallbackHostContext {
    pub fn new(
        settings: crate::settings::AppSettings,
        events: Arc<crate::host::events::ClientEventHub>,
    ) -> Self {
        Self {
            settings: std::sync::Mutex::new(settings),
            events,
        }
    }

    pub fn settings(&self) -> crate::settings::AppSettings {
        self.settings.lock().map(|settings| settings.clone()).unwrap_or_default()
    }

    pub fn events(&self) -> Arc<crate::host::events::ClientEventHub> {
        self.events.clone()
    }

    #[cfg(test)]
    fn for_tests(settings: crate::settings::AppSettings) -> Self {
        Self::new(settings, Arc::new(crate::host::events::ClientEventHub::default()))
    }
}
```

If the real recorder `RuntimeState` cannot move in this task, keep this as the fallback context shell and move command wiring in later tasks. Do not claim full fallback recorder control until Task 13.

- [ ] **Step 4: Pass context to the fallback server**

Change `start_fallback_server` signature in `apps/clipline-app/src/fallback/server.rs`:

```rust
pub async fn start_fallback_server(
    port: Option<u16>,
    host: Arc<crate::host::runtime::FallbackHostContext>,
) -> Result<FallbackServerInfo, String>
```

Add `host` to `FallbackServerState`.

Wire `get_settings` in `invoke`:

```rust
"get_settings" => {
    return axum::Json(crate::host::runtime::FallbackCommandResult::ok(
        serde_json::to_value(state.host.settings()).unwrap_or(serde_json::Value::Null),
    ))
    .into_response();
}
```

- [ ] **Step 5: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app host::runtime::tests::fallback_context_exposes_initial_settings
cargo check -p clipline-app
```

Expected: test passes and app crate checks.

- [ ] **Step 6: Commit**

Run:

```powershell
git add apps/clipline-app/src/fallback/server.rs apps/clipline-app/src/app.rs apps/clipline-app/src/host/runtime.rs
git commit -m "feat(app): wire fallback server to host state"
```

Expected: commit succeeds.

---

### Task 12: Add The Force-Fallback Launch Path

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/src/fallback/server.rs`
- Modify: `apps/clipline-app/src/fallback/startup.rs`

- [ ] **Step 1: Write failing source contract for fallback launch**

Add to `apps/clipline-app/tests/ui_contract.rs`:

```rust
#[test]
fn app_run_can_launch_fallback_server_before_tauri() {
    let app = app_rs();

    assert!(
        app.contains("fallback_launch_preference") && app.contains("start_fallback_server"),
        "app startup must be able to launch fallback before depending on a working WebView"
    );
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract app_run_can_launch_fallback_server_before_tauri
```

Expected: fail because `app.rs` does not yet call the fallback launch path.

- [ ] **Step 3: Add fallback launch helper**

In `apps/clipline-app/src/app.rs`, add a helper near `run()`:

```rust
fn launch_forced_fallback_if_requested(
    args: &[String],
    settings: AppSettings,
) -> Result<bool, String> {
    let preference = crate::fallback::startup::fallback_launch_preference(
        args,
        crate::fallback::startup::WebviewPreflight::Available,
    );
    if preference != crate::fallback::startup::FallbackLaunchPreference::StartFallback {
        return Ok(false);
    }

    let port = crate::fallback::startup::requested_fallback_port(args);
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("create fallback runtime: {e}"))?;
    let host = std::sync::Arc::new(crate::host::runtime::FallbackHostContext::new(
        settings,
        std::sync::Arc::new(crate::host::events::ClientEventHub::default()),
    ));
    let info = runtime.block_on(crate::fallback::server::start_fallback_server(port, host))?;
    open_url_in_default_browser(&info.base_url)?;
    loop {
        std::thread::park();
    }
}

fn open_url_in_default_browser(url: &str) -> Result<(), String> {
    crate::cloud::open_cloud_url_for_host(url, "fallback client")
}
```

Then, inside `run()` after settings are loaded and validated but before `tauri::Builder::default()`, call:

```rust
if launch_forced_fallback_if_requested(&args, settings.clone())? {
    return;
}
```

If `run()` does not return `Result`, log the error and continue to Tauri for now:

```rust
if let Err(e) = launch_forced_fallback_if_requested(&args, settings.clone()) {
    log_diagnostic(format!("forced fallback launch failed: {e}"));
    eprintln!("forced fallback launch failed: {e}");
}
```

Expose `open_cloud_url_for_host` in Task 15 if it is still private; until then the app may call a new native helper.

- [ ] **Step 4: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app --test ui_contract app_run_can_launch_fallback_server_before_tauri
cargo check -p clipline-app
```

Expected: test passes and app crate checks.

- [ ] **Step 5: Manual smoke**

Run:

```powershell
cargo run -p clipline-app -- --force-fallback-client --fallback-port 47651
```

Expected: command keeps running, logs a fallback URL, and the default browser opens the shared Clipline UI. Stop it with Ctrl+C after confirming the page opens.

- [ ] **Step 6: Commit**

Run:

```powershell
git add apps/clipline-app/src/app.rs apps/clipline-app/src/fallback/server.rs apps/clipline-app/src/fallback/startup.rs apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(app): launch forced fallback client"
```

Expected: commit succeeds.

---

### Task 13: Move Recorder Commands To Shared Host State

**Files:**
- Modify: `apps/clipline-app/src/host/runtime.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/src/fallback/server.rs`

- [ ] **Step 1: Write failing tests for recorder command behavior**

Add to `apps/clipline-app/src/host/runtime.rs` tests:

```rust
#[test]
fn fallback_context_records_save_requests_without_panicking_when_service_absent() {
    let context = FallbackHostContext::for_tests(crate::settings::AppSettings::default());

    assert!(!context.save_replay());
}

#[test]
fn fallback_context_reports_recording_state_from_service_presence() {
    let context = FallbackHostContext::for_tests(crate::settings::AppSettings::default());

    assert!(!context.recording_active());
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app host::runtime::tests::fallback_context_records_save_requests_without_panicking_when_service_absent host::runtime::tests::fallback_context_reports_recording_state_from_service_presence
```

Expected: fail because the methods do not exist.

- [ ] **Step 3: Add recorder command methods to fallback context**

Extend `FallbackHostContext` with service command state:

```rust
service_tx: std::sync::Mutex<Option<std::sync::mpsc::Sender<crate::service::Cmd>>>,
```

Initialize it to `None` in `new`.

Add:

```rust
pub fn save_replay(&self) -> bool {
    let Ok(guard) = self.service_tx.lock() else {
        return false;
    };
    let Some(tx) = guard.as_ref() else {
        return false;
    };
    tx.send(crate::service::Cmd::Save).is_ok()
}

pub fn recording_active(&self) -> bool {
    self.service_tx
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false)
}
```

Then add real `set_recording` by reusing `service::spawn` and `AppSettings::to_service_options`:

```rust
pub fn set_recording(&self, recording: bool) -> Result<bool, String> {
    if recording {
        let mut tx_guard = self.service_tx.lock().map_err(|_| "service state lock poisoned")?;
        if tx_guard.is_some() {
            return Ok(true);
        }
        let options = self.settings().to_service_options(None)?;
        let (tx, rx) = crate::service::spawn(options);
        *tx_guard = Some(tx);
        drop(tx_guard);
        self.spawn_event_pump(rx);
        Ok(true)
    } else {
        let tx = self
            .service_tx
            .lock()
            .map_err(|_| "service state lock poisoned")?
            .take();
        if let Some(tx) = tx {
            let _ = tx.send(crate::service::Cmd::Stop { announce: true });
        }
        Ok(false)
    }
}

fn spawn_event_pump(&self, rx: std::sync::mpsc::Receiver<crate::service::Event>) {
    let events = self.events();
    std::thread::Builder::new()
        .name("clipline-fallback-event-pump".into())
        .spawn(move || {
            for event in rx {
                let name = match &event {
                    crate::service::Event::Status { .. } => "status",
                    crate::service::Event::Saved { .. } => "saved",
                    crate::service::Event::Error { .. } => "error",
                };
                let payload = match &event {
                    crate::service::Event::Error { message } => serde_json::json!(message),
                    _ => serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
                };
                events.emit(crate::host::events::ClientEvent::new(name, payload));
            }
        })
        .expect("spawn fallback event pump");
}
```

- [ ] **Step 4: Wire fallback invoke commands**

In `apps/clipline-app/src/fallback/server.rs`, implement:

```rust
"save_replay" => {
    state.host.save_replay();
    return axum::Json(crate::host::runtime::FallbackCommandResult::ok(serde_json::Value::Null)).into_response();
}
"set_recording" => {
    let recording = args
        .get("recording")
        .and_then(|value| value.as_bool())
        .ok_or_else(|| "set_recording requires boolean recording".to_string());
    let result = recording.and_then(|recording| state.host.set_recording(recording));
    return command_response(result);
}
```

Add helper:

```rust
fn command_response<T: serde::Serialize>(result: Result<T, String>) -> Response {
    match result {
        Ok(value) => axum::Json(crate::host::runtime::FallbackCommandResult::ok(
            serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
        ))
        .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            axum::Json(crate::host::runtime::FallbackCommandResult::err(error)),
        )
            .into_response(),
    }
}
```

- [ ] **Step 5: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app host::runtime::tests::fallback_context_records_save_requests_without_panicking_when_service_absent host::runtime::tests::fallback_context_reports_recording_state_from_service_presence
cargo check -p clipline-app
```

Expected: tests pass and app crate checks.

- [ ] **Step 6: Commit**

Run:

```powershell
git add apps/clipline-app/src/host/runtime.rs apps/clipline-app/src/app.rs apps/clipline-app/src/fallback/server.rs
git commit -m "feat(app): route fallback recorder commands"
```

Expected: commit succeeds.

---

### Task 14: Wire Settings, Device, Encoder, Game, And Mic Commands

**Files:**
- Modify: `apps/clipline-app/src/host/runtime.rs`
- Modify: `apps/clipline-app/src/fallback/server.rs`
- Modify: `apps/clipline-app/src/app.rs`

- [ ] **Step 1: Write failing fallback dispatch coverage test**

Add to `apps/clipline-app/src/fallback/server.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_table_contains_settings_and_probe_commands() {
        for command in [
            "get_settings",
            "save_settings",
            "list_displays",
            "list_audio_devices",
            "probe_encoders",
            "list_game_plugins",
            "list_game_windows",
            "extract_window_icon",
            "memory_status",
            "report_decode_support",
            "start_microphone_test",
            "stop_microphone_test",
        ] {
            assert!(fallback_dispatches_command(command), "missing dispatch for {command}");
        }
    }
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app fallback::server::tests::dispatch_table_contains_settings_and_probe_commands
```

Expected: fail because `fallback_dispatches_command` does not exist.

- [ ] **Step 3: Add dispatch coverage helper**

In `apps/clipline-app/src/fallback/server.rs`, add:

```rust
pub fn fallback_dispatches_command(command: &str) -> bool {
    matches!(
        command,
        "frontend_ready"
            | "save_replay"
            | "set_recording"
            | "get_settings"
            | "save_settings"
            | "list_displays"
            | "list_audio_devices"
            | "probe_encoders"
            | "list_game_plugins"
            | "list_game_windows"
            | "extract_window_icon"
            | "memory_status"
            | "report_decode_support"
            | "start_microphone_test"
            | "stop_microphone_test"
    )
}
```

- [ ] **Step 4: Implement host wrappers for probes**

Add methods to `FallbackHostContext`:

```rust
pub fn save_settings(&self, settings: crate::settings::AppSettings) -> Result<crate::settings::AppSettings, String> {
    settings.validate()?;
    settings.save()?;
    let mut guard = self.settings.lock().map_err(|_| "settings lock poisoned")?;
    *guard = settings.clone();
    Ok(settings)
}

pub fn list_displays(&self) -> Result<Vec<crate::app::DisplayInfo>, String> {
    crate::app::host_list_displays()
}

pub fn list_audio_devices(&self) -> Result<crate::app::AudioDeviceLists, String> {
    crate::app::host_list_audio_devices()
}

pub fn probe_encoders(&self) -> Vec<crate::service::EncoderOption> {
    crate::service::available_encoder_options()
}
```

Move `DisplayInfo`, `AudioDeviceInfo`, and `AudioDeviceLists` visibility to `pub(crate)` in `app.rs`, and add `host_list_displays` / `host_list_audio_devices` wrappers there if moving them to `host` is too large for this task.

- [ ] **Step 5: Wire fallback invoke branches**

In `invoke` in `apps/clipline-app/src/fallback/server.rs`, add branches for all commands listed in Step 1. Parse JSON args with typed `serde_json::from_value` for `save_settings`, `extract_window_icon`, `report_decode_support`, and mic commands. Return `command_response(...)` for fallible calls and `FallbackCommandResult::ok(...)` for infallible calls.

- [ ] **Step 6: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app fallback::server::tests::dispatch_table_contains_settings_and_probe_commands
cargo check -p clipline-app
```

Expected: test passes and app crate checks.

- [ ] **Step 7: Commit**

Run:

```powershell
git add apps/clipline-app/src/host/runtime.rs apps/clipline-app/src/fallback/server.rs apps/clipline-app/src/app.rs
git commit -m "feat(app): route fallback settings and probe commands"
```

Expected: commit succeeds.

---

### Task 15: Wire Native Actions And Update Commands

**Files:**
- Create: `apps/clipline-app/src/host/native.rs`
- Modify: `apps/clipline-app/src/host/mod.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/src/cloud.rs`
- Modify: `apps/clipline-app/src/library.rs`
- Modify: `apps/clipline-app/src/fallback/server.rs`

- [ ] **Step 1: Write failing native action tests**

Create `apps/clipline-app/src/host/native.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_url_validator_accepts_only_http_urls() {
        assert!(validate_external_url("https://clipline.test/user").is_ok());
        assert!(validate_external_url("http://127.0.0.1:3000").is_ok());
        assert!(validate_external_url("file:///C:/secret.txt").is_err());
    }
}
```

Modify `apps/clipline-app/src/host/mod.rs`:

```rust
pub mod native;
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app host::native::tests::cloud_url_validator_accepts_only_http_urls
```

Expected: fail because `validate_external_url` does not exist.

- [ ] **Step 3: Implement native URL open helpers**

Add above tests in `apps/clipline-app/src/host/native.rs`:

```rust
pub fn validate_external_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        Ok(())
    } else {
        Err("only http and https URLs can be opened".into())
    }
}

pub fn open_external_url(url: &str, context: &str) -> Result<(), String> {
    validate_external_url(url)?;
    crate::cloud::open_cloud_url_for_host(url, context)
}
```

In `apps/clipline-app/src/cloud.rs`, rename private `open_cloud_url` to `pub(crate) fn open_cloud_url_for_host`, and update `open_cloud_user_profile` and `open_cloud_clip_url` to call the new name.

- [ ] **Step 4: Extract folder and clipboard helpers**

In `apps/clipline-app/src/library.rs`, keep `open_folder_path` and `copy_file_to_clipboard` as `pub(crate)`. Add host native wrappers:

```rust
pub fn open_folder(path: &std::path::Path) -> Result<(), String> {
    crate::library::open_folder_path(path)
}

pub fn copy_file_to_clipboard(path: &std::path::Path) -> Result<(), String> {
    crate::library::copy_file_to_clipboard(path)
}
```

- [ ] **Step 5: Wire fallback native and update dispatch**

In `fallback_dispatches_command`, include:

```rust
"choose_media_folder"
| "choose_replay_cache_folder"
| "get_autostart_status"
| "check_for_updates"
| "install_update"
| "reveal_clip"
| "copy_clip_to_clipboard"
| "open_cloud_user_profile"
| "open_cloud_clip_url"
```

For folder pickers, call `rfd::FileDialog` in `tokio::task::spawn_blocking` or a standard thread and return the selected path string.

For update commands, preserve parity by extracting the existing update check/download/install work into host-callable functions that accept plain Rust state instead of relying on `tauri::AppHandle` at the dispatch boundary. The fallback branch must either perform the same updater behavior as WebView2 mode or use the same shared helper that WebView2 mode calls; do not leave an implementation-error response for update install.

- [ ] **Step 6: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app host::native::tests::cloud_url_validator_accepts_only_http_urls
cargo check -p clipline-app
```

Expected: test passes and app crate checks.

- [ ] **Step 7: Commit**

Run:

```powershell
git add apps/clipline-app/src/host/mod.rs apps/clipline-app/src/host/native.rs apps/clipline-app/src/app.rs apps/clipline-app/src/cloud.rs apps/clipline-app/src/library.rs apps/clipline-app/src/fallback/server.rs
git commit -m "feat(app): share native fallback actions"
```

Expected: commit succeeds.

---

### Task 16: Wire Local Library, Media Validation, And Range Playback

**Files:**
- Modify: `apps/clipline-app/src/fallback/media.rs`
- Modify: `apps/clipline-app/src/fallback/server.rs`
- Modify: `apps/clipline-app/src/host/library.rs`
- Modify: `apps/clipline-app/src/library.rs`

- [ ] **Step 1: Write failing range parser tests**

Add to `apps/clipline-app/src/fallback/media.rs` tests:

```rust
#[test]
fn parses_single_byte_range() {
    assert_eq!(
        parse_range_header("bytes=10-19", 100).unwrap(),
        ByteRange { start: 10, end: 19 }
    );
    assert!(parse_range_header("items=10-19", 100).is_none());
    assert!(parse_range_header("bytes=90-120", 100).is_none());
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app fallback::media::tests::parses_single_byte_range
```

Expected: fail because `parse_range_header` and `ByteRange` do not exist.

- [ ] **Step 3: Implement byte range parsing**

Add to `apps/clipline-app/src/fallback/media.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

pub fn parse_range_header(header: &str, len: u64) -> Option<ByteRange> {
    let raw = header.strip_prefix("bytes=")?;
    let (start, end) = raw.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = end.parse::<u64>().ok()?;
    if start > end || end >= len {
        return None;
    }
    Some(ByteRange { start, end })
}
```

- [ ] **Step 4: Replace raw media registration with validated host media**

Change `/media-path` so it validates through host/library rules before registering:

- If the path is a local clip, call `crate::library::validate_clip_path`.
- If the path is an audio preview, require it starts under `crate::settings::audio_preview_cache_dir()` and has extension `.mp4`.
- If the path is a poster, require it was returned by `clip_poster` or is under the poster cache for a validated clip.
- If the path is cloud cache, require cloud module cache validation.

Add small helper functions in `host/library.rs` for each validator and call them from the server.

- [ ] **Step 5: Add range responses**

In the `/media/{id}` handler, read metadata first. If a `Range` header exists, parse it with `parse_range_header`, seek the file, read only the requested bytes, and return:

```rust
StatusCode::PARTIAL_CONTENT
header::CONTENT_RANGE: bytes start-end/len
header::ACCEPT_RANGES: bytes
```

If no range exists, stream or read the whole file with `ACCEPT_RANGES: bytes`.

- [ ] **Step 6: Wire local library fallback commands**

In fallback dispatch, implement:

- `list_clips`
- `clip_poster`
- `preview_clip_audio_tracks`
- `delete_clip`
- `rename_clip`
- `export_clip`
- `storage_status`
- `reveal_clip`
- `copy_clip_to_clipboard`

Use existing request shapes from `main.js` and existing structs in `library.rs`.

- [ ] **Step 7: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app fallback::media::tests::parses_single_byte_range
cargo test -p clipline-app --test ui_contract audio_preview_command_scopes_generated_preview_files
cargo check -p clipline-app
```

Expected: tests pass and app crate checks.

- [ ] **Step 8: Commit**

Run:

```powershell
git add apps/clipline-app/src/fallback/media.rs apps/clipline-app/src/fallback/server.rs apps/clipline-app/src/host/library.rs apps/clipline-app/src/library.rs
git commit -m "feat(app): serve validated fallback media"
```

Expected: commit succeeds.

---

### Task 17: Wire Cloud Commands And Upload Progress Events

**Files:**
- Modify: `apps/clipline-app/src/cloud.rs`
- Modify: `apps/clipline-app/src/fallback/server.rs`
- Modify: `apps/clipline-app/src/host/events.rs`
- Modify: `apps/clipline-app/src/host/runtime.rs`

- [ ] **Step 1: Write failing fallback dispatch coverage test for cloud commands**

Extend `fallback_dispatches_command` test in `apps/clipline-app/src/fallback/server.rs`:

```rust
#[test]
fn dispatch_table_contains_cloud_commands() {
    for command in [
        "cloud_status",
        "cloud_connect",
        "cloud_disconnect",
        "list_cloud_clips",
        "cloud_clip_thumbnail",
        "cache_cloud_clip_media",
        "cloud_user_profile",
        "cloud_user_avatar",
        "open_cloud_user_profile",
        "open_cloud_clip_url",
        "sync_cloud_clip_status",
        "upload_clip_to_cloud",
    ] {
        assert!(fallback_dispatches_command(command), "missing dispatch for {command}");
    }
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app fallback::server::tests::dispatch_table_contains_cloud_commands
```

Expected: fail until all cloud commands are listed.

- [ ] **Step 3: Extract cloud command bodies**

In `apps/clipline-app/src/cloud.rs`, split each Tauri command into:

- a Tauri wrapper with the existing `#[tauri::command]` signature;
- a `pub(crate)` host function that accepts plain Rust state references and typed request structs.

Use this naming pattern:

```rust
pub(crate) async fn host_cloud_connect(
    state: &crate::host::runtime::FallbackHostContext,
    request: CloudConnectRequest,
) -> Result<CloudConnectionStatus, String>
```

Use the existing logic from `cloud_connect` and call through from the Tauri wrapper so behavior stays identical.

- [ ] **Step 4: Emit cloud upload progress through the shared event hub**

Change upload progress emission to call both Tauri `app.emit` and the shared event hub. Add:

```rust
pub(crate) fn cloud_upload_progress_event_value(event: &CloudUploadProgressEvent) -> serde_json::Value {
    serde_json::to_value(event).unwrap_or(serde_json::Value::Null)
}
```

Fallback upload code should call:

```rust
state.host.events().emit(crate::host::events::ClientEvent::new(
    CLOUD_UPLOAD_PROGRESS_EVENT,
    cloud_upload_progress_event_value(&event),
));
```

- [ ] **Step 5: Wire fallback cloud dispatch**

In `fallback/server.rs`, parse each cloud request type with `serde_json::from_value`, call the new `host_*` functions, and return `command_response`.

- [ ] **Step 6: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app fallback::server::tests::dispatch_table_contains_cloud_commands
cargo check -p clipline-app
```

Expected: test passes and app crate checks.

- [ ] **Step 7: Commit**

Run:

```powershell
git add apps/clipline-app/src/cloud.rs apps/clipline-app/src/fallback/server.rs apps/clipline-app/src/host/events.rs apps/clipline-app/src/host/runtime.rs
git commit -m "feat(app): route fallback cloud commands"
```

Expected: commit succeeds.

---

### Task 18: Replace Repair-Only WebView Failure With Fallback Launch

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/src/fallback/startup.rs`
- Modify: `handoff.md`

- [ ] **Step 1: Write failing decision tests for WebView health failures**

Add to `apps/clipline-app/src/fallback/startup.rs` tests:

```rust
#[test]
fn dead_webview_health_signal_selects_fallback() {
    assert_eq!(
        launch_decision_after_webview_health(WebviewHealthSignal::GetterFailedToReceiveMessage, false),
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
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app fallback::startup::tests::dead_webview_health_signal_selects_fallback fallback::startup::tests::fallback_failure_shows_repair_notice
```

Expected: fail because health decision types do not exist.

- [ ] **Step 3: Implement WebView failure actions**

Add to `apps/clipline-app/src/fallback/startup.rs`:

```rust
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
```

- [ ] **Step 4: Replace repair notice calls with fallback launch attempts**

In `app.rs`, change the getter failure and frontend-ready timeout paths so they call a new `start_fallback_or_show_notice(reason)` helper. The helper should:

1. atomically guard against duplicate fallback launches;
2. create the fallback host context from current settings;
3. start the fallback server;
4. open the fallback URL in the default browser;
5. only call `show_webview_repair_notice_once` if fallback launch fails.

- [ ] **Step 5: Update `handoff.md`**

Add a dated note under recent fixes:

```markdown
- WebView2-free fallback client plan is now in execution: dead WebView2 startup will launch the shared UI through a tokenized localhost browser fallback instead of stopping at a repair-only dialog. Full parity is guarded by source-contract tests over every `invoke` command and `listen` event.
```

- [ ] **Step 6: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app fallback::startup::tests::dead_webview_health_signal_selects_fallback fallback::startup::tests::fallback_failure_shows_repair_notice
cargo check -p clipline-app
```

Expected: tests pass and app crate checks.

- [ ] **Step 7: Commit**

Run:

```powershell
git add apps/clipline-app/src/app.rs apps/clipline-app/src/fallback/startup.rs handoff.md
git commit -m "feat(app): launch fallback on dead WebView2"
```

Expected: commit succeeds.

---

### Task 19: End-To-End Fallback Contract And Browser Smoke

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs`
- Modify: `apps/clipline-app/ui/client-bridge.js`
- Modify: `apps/clipline-app/src/fallback/server.rs`

- [ ] **Step 1: Add full parity source-contract test**

Add to `apps/clipline-app/tests/ui_contract.rs`:

```rust
#[test]
fn every_fallback_manifest_command_has_dispatch_branch() {
    let manifest = fallback_manifest_rs();
    let server = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/fallback/server.rs"),
    )
    .expect("read fallback server");

    let commands = quoted_manifest_array(&manifest, "FALLBACK_COMMANDS");
    assert_eq!(commands.len(), 41, "fallback command manifest inventory changed");

    for command in commands {
        assert!(
            server.contains(&format!("\"{command}\"")),
            "fallback server dispatch must mention manifest command {command}"
        );
    }
}
```

Add this helper next to the existing UI contract helpers if it does not already exist:

```rust
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
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract every_fallback_manifest_command_has_dispatch_branch
```

Expected: fail for any manifest command not wired into server dispatch.

- [ ] **Step 3: Fill dispatch gaps**

Add explicit dispatch branches for every missing command. Each branch must either call a real shared host function or, for a command that is semantically a no-op in fallback (`minimize_main_window`, `frontend_ready`), return a documented successful no-op.

Do not leave a command returning "not wired yet" after this task.

- [ ] **Step 4: Run forced fallback browser smoke**

Run:

```powershell
cargo run -p clipline-app -- --force-fallback-client --fallback-port 47651
```

Expected:

- default browser opens `http://127.0.0.1:47651/<token>/`;
- the gallery/settings UI renders;
- `get_settings` succeeds;
- Save Replay button does not throw;
- `list_clips` succeeds or shows an empty library without an IPC error.

Stop the app after validation.

- [ ] **Step 5: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app --test ui_contract every_fallback_manifest_command_has_dispatch_branch fallback_manifest_covers_every_frontend_command fallback_manifest_covers_every_frontend_event_listener frontend_uses_host_bridge_instead_of_tauri_directly
node --check apps/clipline-app/ui/client-bridge.js
node --check apps/clipline-app/ui/main.js
cargo check -p clipline-app
```

Expected: all tests pass, JavaScript parses, app crate checks.

- [ ] **Step 6: Commit**

Run:

```powershell
git add apps/clipline-app/tests/ui_contract.rs apps/clipline-app/ui/client-bridge.js apps/clipline-app/src/fallback/server.rs
git commit -m "test(app): verify fallback client command parity"
```

Expected: commit succeeds.

---

### Task 20: Final Verification, Docs, And Runtime Launch

**Files:**
- Modify: `README.md`
- Modify: `handoff.md`
- Modify: `docs/superpowers/plans/2026-06-25-webview2-free-fallback-client.md`

- [ ] **Step 1: Update README Windows compatibility note**

Add a concise note near the existing WebView2 install note:

```markdown
If WebView2 is unavailable or broken, Clipline starts a local browser fallback that uses the same first-party UI through a tokenized `127.0.0.1` connection. The fallback is intended for Windows 10 machines where WebView2 cannot be kept installed; it still requires a normal browser for the UI.
```

- [ ] **Step 2: Update `handoff.md` with implementation status**

Add a dated note that includes:

- fallback selected automatically on missing/dead WebView2;
- `--force-fallback-client` test flag;
- tokenized loopback URL;
- source-contract parity tests over commands/events;
- media routes are path-validated and range-capable.

- [ ] **Step 3: Run workspace tests**

Run:

```powershell
cargo test --workspace
```

Expected: all tests pass. Hardware/device tests may self-skip where appropriate.

- [ ] **Step 4: Run fresh clippy for changed app crate**

Run:

```powershell
cargo clean -p clipline-app
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: no warnings or errors.

- [ ] **Step 5: Stop existing app process before runtime launch**

Run:

```powershell
Get-Process clipline-app -ErrorAction SilentlyContinue | Stop-Process
```

Expected: no running `clipline-app.exe` remains.

- [ ] **Step 6: Launch normal app for healthy WebView validation**

Run:

```powershell
cargo run -p clipline-app
```

Expected: app opens normally through Tauri/WebView2 on a healthy machine, `frontend_ready received` appears in `%APPDATA%\Clipline\clipline.log`, and no fallback browser opens.

- [ ] **Step 7: Launch forced fallback for local validation**

Run:

```powershell
cargo run -p clipline-app -- --force-fallback-client --fallback-port 47651
```

Expected: default browser opens the fallback UI and the UI can load settings, list clips, play an H.264 clip through the media route, save replay, open Settings, and show storage status.

- [ ] **Step 8: Validate on Nate's Windows 10 machine or a WebView2-removed VM**

Install or run the build on a machine where WebView2 is absent. Expected:

- Clipline opens the fallback browser client automatically.
- The native repair-only dialog does not block normal fallback use.
- Recorder controls, settings, local library, review playback, trim/export, rename/delete, share, cloud login/upload/progress, updater checks, and tray Save Replay are available.

- [ ] **Step 9: Commit docs**

Run:

```powershell
git add README.md handoff.md docs/superpowers/plans/2026-06-25-webview2-free-fallback-client.md
git commit -m "docs(app): document WebView2-free fallback client"
```

Expected: commit succeeds.

- [ ] **Step 10: Final status**

Run:

```powershell
git status --short --branch
```

Expected: only unrelated pre-existing untracked files remain, such as `.claude/`, unless the user has added other work.

**Execution note (2026-06-27):** Task 20 local validation ran `cargo test --workspace`
successfully, then ran `cargo clean -p clipline-app` followed by
`cargo clippy --workspace --all-targets -- -D warnings`. The clean clippy pass required four
mechanical Rust lint cleanups in `apps/clipline-app/src/app.rs`,
`apps/clipline-app/src/cloud.rs`, and `apps/clipline-app/src/library.rs`. Runtime smoke covered a
normal WebView2 launch with a fresh `frontend_ready received` log entry and no fresh fallback log
lines, then a forced fallback launch on port 47651. The forced fallback URL loaded the shared UI in
a browser, opened Settings and Storage, returned `ok` for `get_settings`, `list_clips`,
`storage_status`, `memory_status`, and `save_replay`, the `list_clips` HTTP invoke returned 44
clips after saving a replay, and a real clip was served through tokenized media routes with `206`
range support while rejecting an outside path. A follow-up fix wired the logged WebView2 registry
probe into startup fallback selection so missing runtime registry entries can select fallback
before creating the WebView. Nate/real WebView2-removed Windows 10 validation was not available
locally and remains external validation.

---

## Self-Review Notes

- Spec coverage: the plan covers shared UI bridge, loopback server, token security, command/event parity, scoped media routes, native capability parity, startup detection, docs, and runtime validation.
- Intentional staging: early tasks create contract tests and host seams; later tasks replace temporary no-op dispatch with real shared host functions before parity is claimed.
- Release gate: completion requires both full workspace verification and manual fallback validation on a WebView2-missing Windows 10 machine or equivalent VM.
