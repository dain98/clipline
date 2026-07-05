# Clipline Updates

Clipline uses Tauri's signed updater. The app checks a channel-specific
`latest.json` file uploaded as a GitHub Release asset.

## Nightly

The enabled channel is Nightly:

```text
https://github.com/dain98/clipline/releases/download/nightly/latest.json
```

Each nightly ships two installer variants built from the same commit:

- **Regular** (`Clipline_<ver>_x64-setup.exe`) — embeds the WebView2 Evergreen
  bootstrapper; small download.
- **Standalone** (`Clipline_<ver>_x64-standalone-setup.exe`) — bundles the
  WebView2 Fixed Version runtime inside the install folder, for users who do
  not want the system-wide WebView2 runtime. Nothing WebView2-related is
  installed system-wide. Adds ~150 MB to the installer.

Each variant has its own updater manifest (`latest.json` /
`latest-standalone.json`); the app picks the right one at runtime by checking
its baked-in `webviewInstallMode` (see `is_standalone_install` in `app.rs`),
so standalone installs never update into the Evergreen installer.

For now, publish Nightly manually from a Windows checkout:

```powershell
# One-time per machine (and when bumping the pinned runtime): download the
# WebView2 Fixed Version runtime and extract it where
# tauri.standalone.conf.json expects it. Get the current x64 .cab URL from
# https://developer.microsoft.com/en-us/microsoft-edge/webview2/ (Fixed
# Version section), then:
#   expand.exe -F:* <runtime>.cab apps\clipline-app\webview2-fixed
# Keep the folder name (with version) in sync with tauri.standalone.conf.json
# — both the resources glob and the webviewInstallMode path.

$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content .local-secrets\clipline-updater.key -Raw

# 1. Regular build (from apps/clipline-app so config discovery works)
Set-Location apps/clipline-app
cargo tauri build
# stage target/release/bundle/nsis/Clipline_<ver>_x64-setup.exe + .sig

# 2. Standalone build (overlay merges over tauri.conf.json)
cargo tauri build --config tauri.standalone.conf.json
# stage and rename to Clipline_<ver>_x64-standalone-setup.exe + .sig

# 3. Author latest.json and latest-standalone.json (version, pub_date,
#    platforms.windows-x86_64.{signature,url}); the url in each must point at
#    the corresponding renamed release asset.

gh release delete nightly --cleanup-tag --yes
gh release create nightly <bundle assets> --prerelease --title "Clipline Nightly"
```

The release must include both updater metadata assets (`latest.json`,
`latest-standalone.json`). When bumping the pinned WebView2 runtime, update
the version in `tauri.standalone.conf.json` and re-download the runtime.
A GitHub Actions workflow can automate this later, but pushing workflow files
requires a token with GitHub's `workflow` scope.

## Signing

The updater public key is committed in `apps/clipline-app/tauri.conf.json`.
The matching private key was generated locally at:

```text
.local-secrets/clipline-updater.key
```

Add the private key contents to the repository secret:

```text
TAURI_SIGNING_PRIVATE_KEY
```

The generated key has no password, so `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` can
be omitted or left empty.

If this private key is lost, future update bundles cannot be signed for
currently installed builds. Generate a new key only when you are ready to rotate
the public key in the app.

## Stable

Stable is modeled in settings but intentionally disabled until Clipline has
stable releases. When stable is ready:

1. Flip `STABLE_CHANNEL_ENABLED` in `apps/clipline-app/src/updates.rs`.
2. Publish non-prerelease GitHub releases with updater `latest.json`.
3. Re-enable the Stable option in the General settings UI.
