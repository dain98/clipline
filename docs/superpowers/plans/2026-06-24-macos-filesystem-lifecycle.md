# macOS Filesystem, Credentials, And Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the first macOS shell placeholders for file actions, startup/update copy, memory status, and cloud credentials with native macOS behavior while preserving Windows behavior.

**Architecture:** Keep the current platform facade and app command surface. macOS-specific behavior lives behind `#[cfg(target_os = "macos")]` in the existing modules; shared UI reads `platform_capabilities()` once during startup to choose platform-neutral copy. Keychain uses the maintained `security-framework` crate, and memory status uses a small process-tree RSS reader backed by `/bin/ps` and `/usr/bin/pgrep`.

**Tech Stack:** Rust 2021, Tauri 2, vanilla HTML/CSS/JS, `security-framework 3.7.0` on macOS, existing `windows-sys` code on Windows, `/usr/bin/open`, `/usr/bin/osascript`, `/bin/ps`, `/usr/bin/pgrep`, existing Rust/Boa contract tests.

## Global Constraints

- The project remains one cross-platform Clipline repository.
- Windows feature behavior is not intentionally reduced.
- macOS feature parity is represented by concrete platform capabilities and implementation plans.
- Do not implement ScreenCaptureKit, CoreAudio capture, VideoToolbox, or CGEventTap in this milestone; expose capability stubs instead.
- Use native macOS behavior only where this milestone has a testable contract: Finder reveal, Finder pasteboard copy, Keychain credential storage, login-item status reconciliation, updater no-artifact messaging, and process memory status.
- Keep Linux/non-macOS non-Windows builds on the cheap stub path so Ubuntu CI does not need system webview libraries.
- Do not write tests that require real cloud credentials, network access, or mutating the user's persistent Keychain.
- Do not remove existing macOS Milestone 1 capture/audio/game-window stubs.

---

## File Structure

- Modify `apps/clipline-app/src/library.rs`: split "reveal clip" from "open media folder"; use Finder reveal on macOS, Explorer select on Windows, and an AppleScript pasteboard file copy on macOS.
- Modify `apps/clipline-app/src/platform/macos.rs`: mark `file_clipboard` available once Finder pasteboard copy exists.
- Modify `apps/clipline-app/tests/macos_shell_contract.rs`: add static contracts for native macOS file actions, Keychain wiring, macOS memory status, and lifecycle/update copy.
- Modify `apps/clipline-app/ui/index.html` and `apps/clipline-app/ui/main.js`: replace hard-coded Windows copy with platform-specific text at runtime.
- Modify `apps/clipline-app/tests/ui_contract.rs`: assert the platform-copy wiring exists and old static Windows-only startup copy is gone.
- Modify `apps/clipline-app/src/memory_macos.rs`: implement process-tree RSS memory status with parsing helpers and tests.
- Modify `apps/clipline-app/src/app.rs`: treat missing macOS updater artifacts as an actionable status instead of a raw updater error.
- Modify `apps/clipline-app/tauri.conf.json`: include macOS bundle targets and product copy that no longer says the app is Windows-only.
- Modify `apps/clipline-app/Cargo.toml`: add a direct macOS target dependency on `security-framework = "3.7.0"`.
- Modify `apps/clipline-app/src/cloud.rs`: remove the macOS cloud-connect availability guard and implement generic-password Keychain write/read/delete.

---

### Task 1: Native macOS File Actions And Platform Copy

**Files:**
- Modify: `apps/clipline-app/src/library.rs`
- Modify: `apps/clipline-app/src/platform/macos.rs`
- Modify: `apps/clipline-app/tests/macos_shell_contract.rs`
- Modify: `apps/clipline-app/ui/index.html`
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

**Interfaces:**
- Consumes: `reveal_clip(path, settings)`, `copy_clip_to_clipboard(path, settings)`, `open_media_folder(settings)`, `platform_capabilities()`.
- Produces: `reveal_file_path(path: &Path) -> Result<(), String>`, `escape_applescript_string(raw: &str) -> String`, runtime platform labels in the existing settings UI.

- [ ] **Step 1: Write failing contracts for macOS file actions**

Append to `apps/clipline-app/tests/macos_shell_contract.rs`:

```rust
#[test]
fn macos_file_actions_are_native_and_available() {
    let library = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/library.rs"))
        .expect("read library.rs");
    let macos =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/macos.rs"))
            .expect("read platform/macos.rs");

    assert!(
        library.contains("fn reveal_file_path(path: &Path) -> Result<(), String>"),
        "reveal_clip should reveal the selected clip, not only open its parent folder"
    );
    assert!(
        library.contains("Command::new(\"open\")") && library.contains(".arg(\"-R\")"),
        "macOS reveal should use Finder's open -R behavior"
    );
    assert!(
        library.contains("Command::new(\"osascript\")")
            && library.contains("set the clipboard to POSIX file"),
        "macOS clipboard copy should put a Finder file on the pasteboard"
    );
    assert!(
        library.contains("fn escape_applescript_string(raw: &str) -> String"),
        "AppleScript command text must escape paths before invoking osascript"
    );
    assert!(
        macos.contains("file_clipboard: CapabilityStatus::available()"),
        "Finder clipboard copy should be advertised once implemented"
    );
}
```

- [ ] **Step 2: Write the AppleScript escaping unit test**

Add this inside `#[cfg(test)] mod tests` in `apps/clipline-app/src/library.rs`:

```rust
    #[cfg(target_os = "macos")]
    #[test]
    fn applescript_string_escapes_quotes_and_backslashes() {
        assert_eq!(
            escape_applescript_string(r#"/tmp/a "quoted" \ clip.mp4"#),
            r#"/tmp/a \"quoted\" \\ clip.mp4"#
        );
    }
```

- [ ] **Step 3: Write failing UI copy contracts**

Update or add tests in `apps/clipline-app/tests/ui_contract.rs`:

```rust
#[test]
fn general_settings_copy_is_platform_aware() {
    let html = index_html();
    let js = main_js();

    assert!(
        html.contains("id=\"open-startup-description\"")
            && html.contains("id=\"open-startup-label\""),
        "startup setting text should have ids for platform-specific copy"
    );
    assert!(
        !html.contains("Start Clipline on Windows login")
            && !html.contains("sign in to Windows"),
        "startup copy should not be hard-coded to Windows in static HTML"
    );
    assert!(
        js.contains("function platformLabel")
            && js.contains("function applyPlatformCopy")
            && js.contains("open-startup-description")
            && js.contains("open-startup-label"),
        "main.js should install platform-specific settings copy"
    );
}
```

- [ ] **Step 4: Run the focused tests and verify they fail**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_file_actions_are_native_and_available
cargo test -p clipline-app library::tests::applescript_string_escapes_quotes_and_backslashes
cargo test -p clipline-app --test ui_contract general_settings_copy_is_platform_aware
```

Expected: FAIL because the reveal helper, AppleScript escape helper, macOS clipboard copy, and platform copy ids/functions do not exist.

- [ ] **Step 5: Implement reveal helpers and macOS pasteboard copy**

In `apps/clipline-app/src/library.rs`, change `reveal_clip` to reveal the target file:

```rust
#[tauri::command]
pub fn reveal_clip(path: String, settings: tauri::State<StorageSettings>) -> Result<(), String> {
    let target = validate_clip_path(&settings, &path)?;
    reveal_file_path(&target)
}
```

Keep `open_media_folder` calling `open_folder_path(&dir)`.

Add these helpers near `open_folder_path`:

```rust
#[cfg(windows)]
fn reveal_file_path(path: &Path) -> Result<(), String> {
    Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("reveal clip {path:?}: {e}"))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn reveal_file_path(path: &Path) -> Result<(), String> {
    Command::new("open")
        .arg("-R")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("reveal clip {path:?}: {e}"))?;
    Ok(())
}
```

Replace the macOS `copy_file_to_clipboard` stub:

```rust
#[cfg(target_os = "macos")]
fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    let script = format!(
        "set the clipboard to POSIX file \"{}\"",
        escape_applescript_string(&path.display().to_string())
    );
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("copy file to Finder clipboard: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "copy file to Finder clipboard failed".into()
        } else {
            format!("copy file to Finder clipboard: {stderr}")
        });
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn escape_applescript_string(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('"', "\\\"")
}
```

- [ ] **Step 6: Advertise file clipboard availability**

In `apps/clipline-app/src/platform/macos.rs`, change:

```rust
file_clipboard: CapabilityStatus::unavailable(
    "Finder clipboard copy is not implemented in Milestone 1",
),
```

to:

```rust
file_clipboard: CapabilityStatus::available(),
```

- [ ] **Step 7: Add runtime platform copy**

In `apps/clipline-app/ui/index.html`, change the startup row text to:

```html
<span id="open-startup-description">Launch Clipline automatically when you sign in.</span>
...
<span id="open-startup-label">Start Clipline on login</span>
```

In `apps/clipline-app/ui/main.js`, add a global:

```js
let platformCapabilities = { os: "windows" };
```

Add helpers near the settings functions:

```js
function platformLabel() {
  return platformCapabilities && platformCapabilities.os === "macos" ? "Mac" : "Windows";
}

function applyPlatformCopy() {
  const label = platformLabel();
  $("open-startup-description").textContent =
    label === "Mac"
      ? "Launch Clipline automatically when you sign in to macOS."
      : "Launch Clipline automatically when you sign in to Windows.";
  $("open-startup-label").textContent =
    label === "Mac" ? "Start Clipline at Mac login" : "Start Clipline on Windows login";
}
```

At the start of `loadInitialSettings`, load and apply capabilities before `fillSettings`:

```js
  try {
    platformCapabilities = await invoke("platform_capabilities");
    applyPlatformCopy();
  } catch (e) {
    console.warn("could not read platform capabilities:", e);
    applyPlatformCopy();
  }
```

- [ ] **Step 8: Run the focused tests**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_file_actions_are_native_and_available
cargo test -p clipline-app library::tests::applescript_string_escapes_quotes_and_backslashes
cargo test -p clipline-app --test ui_contract general_settings_copy_is_platform_aware
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add apps/clipline-app/src/library.rs apps/clipline-app/src/platform/macos.rs apps/clipline-app/tests/macos_shell_contract.rs apps/clipline-app/ui/index.html apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs
git commit -m "feat(app): add native macos file actions"
```

---

### Task 2: macOS Memory, Startup, And Updater Polish

**Files:**
- Modify: `apps/clipline-app/src/memory_macos.rs`
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/tauri.conf.json`
- Modify: `apps/clipline-app/tests/macos_shell_contract.rs`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

**Interfaces:**
- Consumes: `crate::platform::memory_status()`, `check_for_updates`, `UpdateCheckResult`.
- Produces: `parse_ps_rss_kib(output: &str) -> Result<u64, String>`, `parse_pgrep_children(output: &str) -> Vec<u32>`, `macos_update_artifact_missing_message(channel: UpdateChannel) -> String`.

- [ ] **Step 1: Write failing memory tests**

Replace the `macos_hotkey_and_memory_stubs_exist` memory expectation in `apps/clipline-app/tests/macos_shell_contract.rs` so it expects implementation text:

```rust
assert!(memory.contains("Command::new(\"ps\")"));
assert!(memory.contains("Command::new(\"pgrep\")"));
assert!(memory.contains("parse_ps_rss_kib"));
assert!(!memory.contains("macOS memory status is not implemented in Milestone 1"));
```

Add unit tests to `apps/clipline-app/src/memory_macos.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ps_rss_kib_reads_first_number_as_bytes() {
        assert_eq!(parse_ps_rss_kib("  2048\n").unwrap(), 2 * 1024 * 1024);
    }

    #[test]
    fn parse_ps_rss_kib_rejects_empty_output() {
        assert!(parse_ps_rss_kib("   \n").is_err());
    }

    #[test]
    fn parse_pgrep_children_skips_invalid_lines() {
        assert_eq!(parse_pgrep_children("12\nbad\n34\n"), vec![12, 34]);
    }
}
```

- [ ] **Step 2: Write failing updater/bundle tests**

Append to `apps/clipline-app/tests/macos_shell_contract.rs`:

```rust
#[test]
fn macos_bundle_and_update_status_are_explicit() {
    let config =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
            .expect("read tauri.conf.json");
    let app = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs"))
        .expect("read app.rs");

    assert!(config.contains("\"targets\": [\"nsis\", \"dmg\", \"app\"]"));
    assert!(
        !config.contains("for Windows"),
        "bundle product copy should not describe Clipline as Windows-only"
    );
    assert!(
        app.contains("macos_update_artifact_missing_message")
            && app.contains("No macOS update artifact is published yet"),
        "macOS updater artifact gaps should return an actionable status"
    );
}
```

Add an app unit test:

```rust
#[test]
fn macos_update_artifact_message_names_channel() {
    assert_eq!(
        macos_update_artifact_missing_message(UpdateChannel::Nightly),
        "No macOS update artifact is published yet for Nightly. Publish a signed macOS app or DMG artifact first."
    );
}
```

- [ ] **Step 3: Run focused tests and verify they fail**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_hotkey_and_memory_stubs_exist macos_bundle_and_update_status_are_explicit
cargo test -p clipline-app memory_macos::tests::parse_ps_rss_kib_reads_first_number_as_bytes memory_macos::tests::parse_ps_rss_kib_rejects_empty_output memory_macos::tests::parse_pgrep_children_skips_invalid_lines
cargo test -p clipline-app app::tests::macos_update_artifact_message_names_channel
```

Expected: FAIL because the functions and bundle targets are not implemented.

- [ ] **Step 4: Implement macOS process-tree memory**

Replace `apps/clipline-app/src/memory_macos.rs` with:

```rust
use std::collections::VecDeque;
use std::process::Command;

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct MemoryStatus {
    pub private_working_set_bytes: u64,
}

pub fn current_process_tree_memory() -> Result<MemoryStatus, String> {
    let root = std::process::id();
    let mut total = 0u64;
    let mut queue = VecDeque::from([root]);
    while let Some(pid) = queue.pop_front() {
        total = total.saturating_add(rss_bytes_for_pid(pid)?);
        for child in child_pids(pid) {
            queue.push_back(child);
        }
    }
    Ok(MemoryStatus {
        private_working_set_bytes: total,
    })
}

fn rss_bytes_for_pid(pid: u32) -> Result<u64, String> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .map_err(|e| format!("read process memory: {e}"))?;
    if !output.status.success() {
        return Ok(0);
    }
    parse_ps_rss_kib(&String::from_utf8_lossy(&output.stdout))
}

fn child_pids(pid: u32) -> Vec<u32> {
    let Ok(output) = Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_pgrep_children(&String::from_utf8_lossy(&output.stdout))
}

fn parse_ps_rss_kib(output: &str) -> Result<u64, String> {
    let kib = output
        .split_whitespace()
        .next()
        .ok_or_else(|| "process memory output was empty".to_string())?
        .parse::<u64>()
        .map_err(|e| format!("parse process memory: {e}"))?;
    Ok(kib.saturating_mul(1024))
}

fn parse_pgrep_children(output: &str) -> Vec<u32> {
    output
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}
```

Keep the unit tests from Step 1 at the bottom of the file.

- [ ] **Step 5: Add macOS update artifact status**

In `apps/clipline-app/src/app.rs`, add:

```rust
fn macos_update_artifact_missing_message(channel: UpdateChannel) -> String {
    format!(
        "No macOS update artifact is published yet for {}. Publish a signed macOS app or DMG artifact first.",
        channel.label()
    )
}

fn updater_error_status(channel: UpdateChannel, error: &tauri_plugin_updater::Error) -> Option<String> {
    let message = error.to_string();
    if cfg!(target_os = "macos") && message.contains("darwin") && message.contains("platforms") {
        Some(macos_update_artifact_missing_message(channel))
    } else {
        None
    }
}
```

Change the `check_update_for_channel` error arm from:

```rust
Err(e) => Err(e.to_string()),
```

to:

```rust
Err(e) => match updater_error_status(channel, &e) {
    Some(status) => Ok((None, Some(status))),
    None => Err(e.to_string()),
},
```

- [ ] **Step 6: Update bundle copy and targets**

In `apps/clipline-app/tauri.conf.json`, change bundle targets to:

```json
"targets": ["nsis", "dmg", "app"],
```

Change `longDescription` so it no longer says "for Windows":

```json
"longDescription": "Clipline is a lightweight, ad-free, open-source game recorder — anti-cheat-safe capture, a ShadowPlay-style replay buffer, and automatic in-game event markers (League of Legends), with a built-in review/trim player.",
```

- [ ] **Step 7: Run focused tests**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_hotkey_and_memory_stubs_exist macos_bundle_and_update_status_are_explicit
cargo test -p clipline-app memory_macos::tests::parse_ps_rss_kib_reads_first_number_as_bytes memory_macos::tests::parse_ps_rss_kib_rejects_empty_output memory_macos::tests::parse_pgrep_children_skips_invalid_lines
cargo test -p clipline-app app::tests::macos_update_artifact_message_names_channel
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add apps/clipline-app/src/memory_macos.rs apps/clipline-app/src/app.rs apps/clipline-app/tauri.conf.json apps/clipline-app/tests/macos_shell_contract.rs
git commit -m "feat(app): polish macos lifecycle status"
```

---

### Task 3: macOS Keychain Cloud Credentials

**Files:**
- Modify: `apps/clipline-app/Cargo.toml`
- Modify: `apps/clipline-app/src/cloud.rs`
- Modify: `apps/clipline-app/tests/macos_shell_contract.rs`

**Interfaces:**
- Consumes: `credential_target(host_url, user_id) -> String`, `write_credential(target, username, token)`, `read_credential(target)`, `delete_credential(target)`, `cloud_connect`.
- Produces: macOS Keychain generic-password storage using service `Clipline Cloud` and account equal to the credential target string.

- [ ] **Step 1: Write failing Keychain contract tests**

Replace the existing `macos_cloud_connect_fails_before_network_request` test in `apps/clipline-app/tests/macos_shell_contract.rs` with:

```rust
#[test]
fn macos_cloud_credentials_use_keychain_before_network_uploads() {
    let manifest = manifest();
    let cloud = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cloud.rs"))
        .expect("read cloud.rs");

    assert!(
        manifest.contains("[target.'cfg(target_os = \"macos\")'.dependencies]")
            && manifest.contains("security-framework = \"3.7.0\""),
        "macOS builds should depend directly on security-framework for Keychain access"
    );
    assert!(
        !cloud.contains("macOS cloud connect is unavailable until Keychain storage is implemented"),
        "cloud connect should no longer be blocked on macOS"
    );
    assert!(cloud.contains("const KEYCHAIN_SERVICE: &str = \"Clipline Cloud\";"));
    assert!(cloud.contains("security_framework::os::macos::keychain::SecKeychain"));
    assert!(cloud.contains("security_framework::os::macos::passwords::find_generic_password"));
    assert!(cloud.contains("set_generic_password(KEYCHAIN_SERVICE, target, token.as_bytes())"));
    assert!(cloud.contains("find_generic_password(None, KEYCHAIN_SERVICE, target)"));
    assert!(
        cloud.contains("item.delete();"),
        "macOS disconnect should delete the Keychain item"
    );
    let network = cloud
        .find("clipline_cloud_api::connect_with_device_token")
        .expect("cloud_connect should use real network connect");
    let write = cloud
        .find("write_credential(&target, &result.username, &result.token)?;")
        .expect("cloud_connect should persist the returned token");
    assert!(network < write, "token should be stored after a successful connect");
}
```

- [ ] **Step 2: Run the contract and verify it fails**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_cloud_credentials_use_keychain_before_network_uploads
```

Expected: FAIL because macOS still has placeholder credential functions and no direct `security-framework` dependency.

- [ ] **Step 3: Add the macOS dependency**

In `apps/clipline-app/Cargo.toml`, add:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
security-framework = "3.7.0"
```

Keep `windows-sys` in the Windows-only dependency block.

- [ ] **Step 4: Implement Keychain storage**

In `apps/clipline-app/src/cloud.rs`, add macOS imports:

```rust
#[cfg(target_os = "macos")]
use security_framework::os::macos::keychain::SecKeychain;
#[cfg(target_os = "macos")]
use security_framework::os::macos::passwords::find_generic_password;
```

Add near the cloud constants:

```rust
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "Clipline Cloud";
```

Change the macOS availability guard:

```rust
#[cfg(target_os = "macos")]
fn ensure_cloud_connect_available() -> Result<(), String> {
    Ok(())
}
```

Replace macOS credential functions:

```rust
#[cfg(target_os = "macos")]
fn write_credential(target: &str, _username: &str, token: &str) -> Result<(), String> {
    SecKeychain::default()
        .map_err(|e| format!("open Keychain: {e}"))?
        .set_generic_password(KEYCHAIN_SERVICE, target, token.as_bytes())
        .map_err(|e| format!("store cloud token in Keychain: {e}"))
}

#[cfg(target_os = "macos")]
fn read_credential(target: &str) -> Result<String, String> {
    let (password, _) = find_generic_password(None, KEYCHAIN_SERVICE, target)
        .map_err(|e| format!("read cloud token from Keychain: {e}"))?;
    String::from_utf8(password.as_ref().to_vec())
        .map_err(|_| "cloud token is not valid UTF-8".to_string())
}

#[cfg(target_os = "macos")]
fn delete_credential(target: &str) -> Result<(), String> {
    match find_generic_password(None, KEYCHAIN_SERVICE, target) {
        Ok((_, item)) => {
            item.delete();
            Ok(())
        }
        Err(_) => Ok(()),
    }
}
```

- [ ] **Step 5: Run focused tests**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_cloud_credentials_use_keychain_before_network_uploads
cargo test -p clipline-app cloud::tests::credential_target_includes_server_and_user
```

Expected: PASS.

- [ ] **Step 6: Compile the app with the new dependency**

Run:

```bash
cargo check -p clipline-app
```

Expected: PASS on macOS. If the compiler rejects an exact `security-framework` path, inspect the installed crate source under `~/.cargo/registry/src/*/security-framework-3.7.0/src/os/macos/passwords.rs` and adjust imports while keeping the same public contract from Step 1.

- [ ] **Step 7: Commit**

```bash
git add apps/clipline-app/Cargo.toml apps/clipline-app/src/cloud.rs apps/clipline-app/tests/macos_shell_contract.rs Cargo.lock
git commit -m "feat(cloud): store macos tokens in keychain"
```

---

### Task 4: Final Verification And macOS Launch Smoke

**Files:**
- Modify only if verification reveals an issue in files touched by Tasks 1-3.

**Interfaces:**
- Consumes: all changes from Tasks 1-3.
- Produces: verified branch state and final notes for the next macOS milestone.

- [ ] **Step 1: Run focused contract tests**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract
cargo test -p clipline-app --test ui_contract
```

Expected: PASS.

- [ ] **Step 2: Run app and workspace verification**

Run:

```bash
cargo test -p clipline-app
cargo clippy -p clipline-app --all-targets -- -D warnings
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 3: Launch-smoke the current macOS build**

Run:

```bash
log=$(mktemp -t clipline-lifecycle-smoke)
cargo run -p clipline-app >"$log" 2>&1 &
pid=$!
for _ in {1..40}; do
  pgrep -P "$pid" >/dev/null 2>&1 || true
  if pgrep -fl 'clipline-app' >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
pgrep -fl 'clipline-app' || true
tail -n 80 "$log"
pkill -f 'target/debug/clipline-app' || true
wait "$pid" 2>/dev/null || true
```

Expected: the app process starts, no `No such file or directory (os error 2)` appears, and remaining logs are limited to honest unavailable capability messages.

- [ ] **Step 4: Inspect final diff**

Run:

```bash
git status --short
git diff --stat HEAD~3..HEAD
```

Expected: only the macOS filesystem/lifecycle/credentials files changed after the plan commits.

- [ ] **Step 5: Commit fixes if needed**

If Step 1-4 required changes, commit them:

```bash
git add apps/clipline-app/src/library.rs apps/clipline-app/src/platform/macos.rs apps/clipline-app/ui/index.html apps/clipline-app/ui/main.js apps/clipline-app/tests/ui_contract.rs apps/clipline-app/src/memory_macos.rs apps/clipline-app/src/app.rs apps/clipline-app/tauri.conf.json apps/clipline-app/Cargo.toml apps/clipline-app/src/cloud.rs apps/clipline-app/tests/macos_shell_contract.rs Cargo.lock
git commit -m "fix(app): harden macos lifecycle polish"
```

If no changes were needed, do not create an empty commit.
