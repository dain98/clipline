# WebView2 Bootstrapper Compatibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve Clipline's default Windows 10 WebView2 compatibility while keeping the installer small: fresh installs and updates should fetch/repair WebView2 through Microsoft's bootstrapper when needed, and already-broken installs should get a native repair notice instead of a silent tray-only app.

**Architecture:** Keep Tauri as the only UI runtime. Switch the NSIS WebView2 install mode from bundled offline runtime to embedded Evergreen bootstrapper with the existing minimum version. Detect dead WebView2 content after window reveal through getter probes and a frontend-readiness watchdog. The frontend reports readiness once its JavaScript executes; the Rust shell shows one native `rfd` repair dialog per process from a short worker thread so the Tauri event loop is not blocked.

**Tech Stack:** Rust/Tauri 2 app shell, Tauri NSIS bundler config, `rfd` native message dialog, vanilla JS frontend, Rust unit tests, `ui_contract.rs` structural tests, GitHub nightly release artifacts.

---

### Task 1: Lock Installer Contract With A Failing Test

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Strengthen the WebView2 installer contract**

Replace the current `windows_installer_requires_modern_webview2_runtime` body with assertions for both the minimum version and the bootstrapper mode:

```rust
#[test]
fn windows_installer_repairs_webview2_with_bootstrapper() {
    let config = tauri_config();

    assert!(
        config.contains("\"minimumWebview2Version\": \"120.0.2210.55\""),
        "Windows 10 installs must repair/update stale WebView2 runtimes before Clipline starts"
    );
    assert!(
        config.contains("\"webviewInstallMode\"") && config.contains("\"type\": \"embedBootstrapper\""),
        "the default NSIS installer should embed the small Evergreen bootstrapper instead of bundling the offline WebView2 installer"
    );
}
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract windows_installer_repairs_webview2_with_bootstrapper
```

Expected: fail because `tauri.conf.json` still has `"type": "offlineInstaller"`.

### Task 2: Switch NSIS WebView2 Install Mode

**Files:**
- Modify: `apps/clipline-app/tauri.conf.json`

- [ ] **Step 1: Change the installer config**

Change:

```json
"webviewInstallMode": {
  "type": "offlineInstaller"
}
```

to:

```json
"webviewInstallMode": {
  "type": "embedBootstrapper",
  "silent": true
}
```

Keep:

```json
"minimumWebview2Version": "120.0.2210.55"
```

- [ ] **Step 2: Verify GREEN**

Run:

```powershell
cargo test -p clipline-app --test ui_contract windows_installer_repairs_webview2_with_bootstrapper
```

Expected: pass.

### Task 3: Add Frontend-Ready Contract Tests First

**Files:**
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Write the failing contract**

Add a test that requires a Rust command and a frontend invoke:

```rust
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
```

- [ ] **Step 2: Verify RED**

Run:

```powershell
cargo test -p clipline-app --test ui_contract frontend_reports_webview_readiness_to_native_shell
```

Expected: fail because no command or invoke exists yet.

### Task 4: Add WebView Health Decision Unit Tests

**Files:**
- Modify: `apps/clipline-app/src/app.rs`
- Modify: `apps/clipline-app/Cargo.toml`

- [ ] **Step 1: Add `tauri-runtime` as a direct Windows dependency**

The typed getter failure is `tauri::Error::Runtime(tauri_runtime::Error::FailedToReceiveMessage)`. Add this under `[target.'cfg(windows)'.dependencies]` so `app.rs` can match the public variant directly:

```toml
tauri-runtime = "2"
```

- [ ] **Step 2: Add failing pure tests**

Add test-only coverage near the existing `app.rs` tests for the health decision and one-shot notice gate:

```rust
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
```

- [ ] **Step 3: Verify RED**

Run:

```powershell
cargo test -p clipline-app app::tests::webview_repair_notice_is_only_needed_for_dead_webview_signals app::tests::classifies_tauri_runtime_receive_failure_as_dead_webview
```

Expected: fail because the enum and helper functions do not exist.

### Task 5: Implement Native WebView Repair Notice Logic

**Files:**
- Modify: `apps/clipline-app/src/app.rs`

- [ ] **Step 1: Add process-wide readiness and notice gates**

Add atomics near the existing diagnostics statics:

```rust
static FRONTEND_READY: AtomicBool = AtomicBool::new(false);
static WEBVIEW_READY_WATCHDOG_ARMED: AtomicBool = AtomicBool::new(false);
static WEBVIEW_REPAIR_NOTICE_SHOWN: AtomicBool = AtomicBool::new(false);
const WEBVIEW_READY_TIMEOUT: Duration = Duration::from_secs(5);
```

Use `std::sync::atomic::{AtomicBool, Ordering}` and the existing `Duration` import.

- [ ] **Step 2: Add the pure decision helpers**

Implement:

```rust
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
```

- [ ] **Step 3: Add the non-blocking dialog wrapper**

Implement a one-shot dialog function:

```rust
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
    std::thread::Builder::new()
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
        })
        .ok();
}
```

Keep the URL in plain text because `rfd` message dialogs are not reliable hyperlink controls.

- [ ] **Step 4: Probe getter health after reveal**

After every `reveal_logged_window` call in `open_main_window`, run an explicit getter probe:

```rust
fn probe_webview_after_reveal<R: Runtime>(window: &WebviewWindow<R>, context: &str) {
    match window.is_visible() {
        Ok(visible) => log_diagnostic(format!("{context} health probe is_visible=ok({visible})")),
        Err(e) => {
            let reason = classify_webview_getter_error(&e);
            log_diagnostic(format!("{context} health probe is_visible=err({e}) reason={reason:?}"));
            show_webview_repair_notice_once(reason);
        }
    }
}
```

Call it after `log_window_state("... after reveal", &window);`.

- [ ] **Step 5: Arm the frontend-readiness watchdog only when opening the UI**

Add:

```rust
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
    std::thread::Builder::new()
        .name("clipline-webview-readiness-watchdog".into())
        .spawn(|| {
            std::thread::sleep(WEBVIEW_READY_TIMEOUT);
            if !FRONTEND_READY.load(Ordering::Acquire) {
                log_diagnostic("webview readiness watchdog expired before frontend_ready");
                show_webview_repair_notice_once(WebviewRepairNoticeReason::FrontendReadyTimeout);
            } else {
                log_diagnostic("webview readiness watchdog observed frontend_ready");
            }
        })
        .ok();
}
```

Call this from `open_main_window` after a reveal attempt is made, not during autostart-only hidden setup.

- [ ] **Step 6: Add the command and invoke handler**

Add:

```rust
#[tauri::command]
fn frontend_ready() {
    let was_ready = FRONTEND_READY.swap(true, Ordering::AcqRel);
    if !was_ready {
        log_diagnostic("frontend_ready received");
    }
}
```

Include `frontend_ready,` in `tauri::generate_handler![...]`.

- [ ] **Step 7: Verify unit tests GREEN**

Run:

```powershell
cargo test -p clipline-app app::tests::webview_repair_notice_is_only_needed_for_dead_webview_signals app::tests::classifies_tauri_runtime_receive_failure_as_dead_webview
```

Expected: pass.

### Task 6: Send Frontend Ready From JavaScript

**Files:**
- Modify: `apps/clipline-app/ui/main.js`
- Modify: `apps/clipline-app/tests/ui_contract.rs`

- [ ] **Step 1: Add a small reporting helper**

Near the boot section, add:

```javascript
function reportFrontendReady() {
  invoke("frontend_ready").catch((e) => console.warn("frontend_ready failed:", e));
}
```

- [ ] **Step 2: Call it after JS boot has scheduled initial work**

After the existing `afterNextPaint().then(...)` setup block is registered, call:

```javascript
reportFrontendReady();
```

The signal means "frontend JavaScript executed and IPC works", not "all async lists have finished loading".

- [ ] **Step 3: Verify contract GREEN**

Run:

```powershell
cargo test -p clipline-app --test ui_contract frontend_reports_webview_readiness_to_native_shell
```

Expected: pass.

### Task 7: Update Docs And Handoff

**Files:**
- Modify: `README.md`
- Modify: `handoff.md`

- [ ] **Step 1: Document installer behavior**

Add a concise Windows install note:

```markdown
On Windows, the standard installer embeds Microsoft's small WebView2 Evergreen bootstrapper. If WebView2 is missing or older than Clipline's minimum supported runtime, the installer may download the current runtime from Microsoft. Offline or Microsoft-blocked machines may need the WebView2 Runtime installed manually first.
```

- [ ] **Step 2: Document the native recovery path**

In `handoff.md`, add a current-state note that:

- `0.1.14` switches from WebView2 offline installer to embedded bootstrapper.
- The app now logs `frontend_ready received` when the UI boots.
- If getter probes fail with `FailedToReceiveMessage` or the frontend-ready watchdog expires, Clipline shows one native repair dialog per process.
- This only repairs fresh installs/updates automatically; already-broken installs need reinstall/manual WebView2 repair because the frontend updater cannot run.

### Task 8: Bump Version For Release

**Files:**
- Modify: `apps/clipline-app/Cargo.toml`
- Modify: `apps/clipline-app/tauri.conf.json`
- Modify: `Cargo.lock`
- Modify: `README.md`
- Modify: `handoff.md`

- [ ] **Step 1: Bump from `0.1.13` to `0.1.14`**

Update the app crate version and Tauri package version. Update README/handoff latest-version references if present.

- [ ] **Step 2: Refresh lockfile**

Run:

```powershell
cargo check -p clipline-app
```

Expected: lockfile reflects `clipline-app 0.1.14` and the direct `tauri-runtime` dependency if Cargo needed to record it.

### Task 9: Full Verification

**Files:**
- No source edits unless verification fails.

- [ ] **Step 1: Stop any running local app before rebuilding**

Run:

```powershell
Get-Process clipline-app -ErrorAction SilentlyContinue | Stop-Process
```

Expected: no running `clipline-app.exe` remains.

- [ ] **Step 2: Run workspace tests**

Run:

```powershell
cargo test --workspace
```

Expected: all tests pass.

- [ ] **Step 3: Clean changed crate before clippy**

Run:

```powershell
cargo clean -p clipline-app
```

Expected: package build artifacts removed.

- [ ] **Step 4: Run workspace clippy**

Run:

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] **Step 5: Launch local app for smoke test**

Run:

```powershell
cargo run -p clipline-app
```

Expected: app opens on Windows 11, `clipline.log` contains `frontend_ready received`, and window getter health probes are `ok(...)`.

### Task 10: Build And Publish Nightly Release

**Files:**
- Release artifacts under `apps/clipline-app/target/release/bundle/nsis/`
- GitHub `nightly` release

- [ ] **Step 1: Build signed NSIS artifacts**

Run:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content .local-secrets\clipline-updater.key -Raw
cargo tauri build --config apps/clipline-app/tauri.conf.json
```

Expected:

- `Clipline_0.1.14_x64-setup.exe`
- `.sig`
- `latest.json`

If the build hangs after producing the installer but before signing, use the existing local signer flow with `.local-secrets\clipline-updater.key`, then verify `latest.json` points to `0.1.14`.

- [ ] **Step 2: Verify installer size and metadata**

Run:

```powershell
Get-Item apps\clipline-app\target\release\bundle\nsis\Clipline_0.1.14_x64-setup.exe | Select-Object Name,Length
Get-Content apps\clipline-app\target\release\bundle\nsis\latest.json
```

Expected: installer is much smaller than the previous offline-installer build, and `latest.json` references `0.1.14`.

- [ ] **Step 3: Commit implementation**

Run:

```powershell
git status --short
git add apps/clipline-app/Cargo.toml apps/clipline-app/tauri.conf.json apps/clipline-app/src/app.rs apps/clipline-app/tests/ui_contract.rs apps/clipline-app/ui/main.js Cargo.lock README.md handoff.md docs/superpowers/plans/2026-06-25-webview2-bootstrapper-compat.md
git commit -m "fix(app): repair missing WebView2 installs"
```

Expected: commit includes only intended tracked files; leave unrelated `.claude/` untracked.

- [ ] **Step 4: Push and update nightly release**

Run:

```powershell
git push origin main
git tag -f nightly
git push origin nightly --force
gh release upload nightly `
  apps/clipline-app/target/release/bundle/nsis/Clipline_0.1.14_x64-setup.exe `
  apps/clipline-app/target/release/bundle/nsis/Clipline_0.1.14_x64-setup.exe.sig `
  apps/clipline-app/target/release/bundle/nsis/latest.json `
  --clobber
```

Expected: the public nightly release serves the `0.1.14` installer, signature, and updater manifest.

### Task 11: User Test Notes

**Files:**
- No source edits.

- [ ] **Step 1: Report specific smoke tests**

Ask the user to test:

- On Windows 11: normal open from launch and tray still works; `clipline.log` includes `frontend_ready received`.
- On the failing Windows 10 machine: reinstall `0.1.14` with internet available; installer should repair/download WebView2 or the app should show the native WebView2 repair dialog instead of silently doing nothing.
- On an offline Windows 10 machine: expect manual WebView2 install may be required.

---

## Self-Review

- Spec coverage: covers installer bootstrapper, minimum version, install-time vs already-broken recovery split, getter-based detection, frontend-readiness watchdog, non-blocking native dialog, docs, verification, and release.
- TDD shape: starts each behavior with a failing contract/unit test before implementation.
- Platform discipline: new runtime matching and dialog code stay in Windows-only `app.rs`; non-Windows stub remains unaffected.
- Risk notes: direct `tauri-runtime` dependency is added only to match the public runtime error variant; if Cargo/Tauri exposes a cleaner re-export during implementation, use that and omit the direct dependency.
- No placeholders or TBDs remain.
