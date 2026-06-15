# Clipline Updates

Clipline uses Tauri's signed updater. The app checks a channel-specific
`latest.json` file uploaded as a GitHub Release asset.

## Nightly

The enabled channel is Nightly:

```text
https://github.com/dain98/clipline/releases/download/nightly/latest.json
```

For now, publish Nightly manually from a Windows checkout:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content .local-secrets\clipline-updater.key -Raw
cargo tauri build --config apps/clipline-app/tauri.conf.json
gh release delete nightly --cleanup-tag --yes
gh release create nightly <bundle assets> --prerelease --title "Clipline Nightly"
```

The release must include the generated `latest.json` updater metadata asset.
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
