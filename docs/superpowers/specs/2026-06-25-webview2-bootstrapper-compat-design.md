# WebView2 Bootstrapper Compatibility Design

## Context

Clipline's UI is Tauri/WebView2. On Windows 11 this is normally preinstalled and healthy. A
Windows 10 tester removed Edge/WebView2 registry state, leaving Clipline able to start its tray and
recorder but unable to create a responsive webview. Version 0.1.13 added WebView2 registry
diagnostics and a minimum runtime version, but the installer still embeds the full offline
Evergreen installer, making the nightly installer about 209 MB.

## Goals

- Improve the default Windows 10 compatibility/installer-size balance for users whose WebView2
  install is missing, stale, or registry-broken and who can reach Microsoft's WebView2 runtime
  download endpoint during install.
- Reduce the normal installer size by avoiding the bundled offline WebView2 runtime.
- Preserve the signed Tauri updater path and nightly release workflow.
- Give users a visible native diagnostic when the WebView2 UI cannot open.

## Non-Goals

- Do not switch to a WebView2 fixed runtime in the default installer. That remains a future
  compatibility installer option if bootstrapper repair is not enough.
- Do not build a non-WebView replacement UI.
- Do not remove the WebView2 runtime dependency; Tauri needs an available WebView2 runtime to render
  the Clipline interface.
- Do not claim air-gapped compatibility for the default installer. Switching from the offline
  Evergreen installer to the bootstrapper intentionally trades offline repair for a smaller
  installer.

## Approach

Use Tauri's `embedBootstrapper` WebView2 install mode instead of `offlineInstaller`. The installer
will carry Microsoft's small Evergreen bootstrapper rather than the full offline runtime. When
WebView2 is missing or older than `minimumWebview2Version`, the installer runs the bootstrapper in
silent mode and lets it download/install the current Evergreen runtime.

This repair path is install/update-time only. It does not help an already-installed copy of Clipline
whose WebView2 registry/runtime state is removed later until the user runs a new installer again.
The in-app updater is also frontend-driven, so a machine with a dead WebView2 UI cannot self-update
through the UI. For that reported failure mode, the native dialog is the in-product recovery path;
the bootstrapper helps fresh installs, manual reinstall, and future updater installer runs.

Keep `minimumWebview2Version = "120.0.2210.55"` so machines with stale runtime registration still
trigger repair. Keep the startup log line that records the WebView2 runtime registry `pv` values.

Add a native fallback notice for UI-open failure. The reveal calls (`show`, `unminimize`,
`set_focus`) are fire-and-forget window-manager operations and must not drive this decision. Instead
use explicit health signals:

- Immediately after a reveal attempt, run a window-state getter probe such as `is_visible()`. Match
  the typed Tauri runtime `FailedToReceiveMessage` error when available. This is the same signal the
  Windows 10 logs showed through `window_state_summary`; those getter results are currently only
  stringified for logging, so the implementation needs a separate decision probe.
- Add a frontend-readiness watchdog for the actual "webview content never executes" symptom. Arm it
  only when the UI is revealed, disarm it when the frontend sends a small ready command/event after
  boot, and show the same native notice if it expires.

Show the native `rfd::MessageDialog` once per process, preferably from a short spawned thread so the
Tauri event loop is not blocked by the modal dialog. The message should say that Microsoft Edge
WebView2 Runtime is missing or broken, point users to the official WebView2 runtime download page,
and mention the diagnostic log path.

## Components

- `apps/clipline-app/tauri.conf.json`: switch `webviewInstallMode` to embedded bootstrapper, keep
  the minimum version.
- `apps/clipline-app/src/app.rs`: keep startup diagnostics; add a one-shot native WebView2 repair
  notice driven by a post-reveal getter probe and frontend-readiness timeout.
- `apps/clipline-app/ui/main.js`: send the frontend-ready signal after the UI boots.
- `apps/clipline-app/tests/ui_contract.rs`: guard the bootstrapper install mode and the presence of
  the native fallback diagnostic.
- `README.md` and `handoff.md`: document that the normal installer may need internet on Windows 10
  when repairing WebView2, and that a fixed-runtime compatibility installer is the heavier fallback
  option.

## Error Handling

The bootstrapper cannot repair machines without internet access, machines that block Microsoft
runtime downloads, or users who intentionally refuse WebView2. In those cases Clipline should still
start its tray/recorder if possible, write diagnostics to `%APPDATA%\Clipline\clipline.log`, and
show the native fallback notice when the UI cannot open.

Because the NSIS installer is configured as `currentUser`, verify that the silent Evergreen
bootstrapper path repairs a clean/broken Windows 10 machine without hiding a UAC prompt or failing
quietly. If it fails under current-user install conditions, keep the smaller bootstrapper installer
but document that users may need to run the Microsoft WebView2 Runtime installer directly.

## Testing

- Add a contract test that `tauri.conf.json` uses `embedBootstrapper` and keeps
  `minimumWebview2Version`.
- Add focused app tests for the fallback-diagnostic decision logic without invoking native UI:
  getter-failure classification, frontend-readiness timeout classification, and one-shot dialog
  gating.
- Add a UI contract test that the frontend sends the ready signal during startup.
- Run `cargo test --workspace`.
- Run `cargo clean -p clipline-app && cargo clippy --workspace --all-targets -- -D warnings`.
- Build the NSIS installer and verify the installer asset is much smaller than the offline-runtime
  builds while still producing signed updater artifacts.

## Release Criteria

- Nightly installer size drops materially from the current ~209 MB offline-runtime installer.
- Public `latest.json` points to the new installer and contains a matching updater signature.
- On a healthy Windows 11 machine, local `cargo run -p clipline-app` still opens normally and logs a
  WebView2 runtime version.
- A fresh/manual reinstall on a Windows 10 machine with missing WebView2 registry state should
  either repair WebView2 through the bootstrapper or show the native repair notice with actionable
  instructions.
- If the affected tester cannot reach Microsoft WebView2 downloads, the native notice is still the
  expected behavior for the normal installer; a separate fixed-runtime compatibility installer is
  the follow-up option for that environment.
