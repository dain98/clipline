# Reliability and Playback Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix seven verified data-loss, lifecycle, format, request-race, and playback-position defects without changing intentional marker navigation.

**Architecture:** Each defect is an independent TDD task and an independent conventional commit. Rust filesystem and MP4 fixes stay in their owning crates; cloud request arbitration gets a small DOM-free JavaScript core; playback source-swap math stays in `player-core.js` and is consumed by `review-player.js` only after asynchronous preview generation completes.

**Tech Stack:** Rust 2021 workspace, Tauri 2, vanilla HTML/CSS/JavaScript, Boa JavaScript tests hosted from Rust, serde JSON sidecars, ISO BMFF/Hybrid MP4.

## Global Constraints

- Preserve the current uncommitted review-player shortcut changes.
- Use failing tests before implementation for every defect.
- Keep neutral logic cross-platform and Windows-specific behavior behind existing platform boundaries.
- Do not introduce a new media format, database, frontend framework, or background service.
- Keep each fix independently reviewable and commit it as one logical change.
- Keep marker-navigation wrapping unchanged; the reported reset travels through the five-second seek and asynchronous preview path.
- Stage only the hunks belonging to the current task. In particular, use interactive staging for `apps/clipline-app/tests/player_core.rs`, `apps/clipline-app/tests/ui_contract.rs`, `apps/clipline-app/ui/index.html`, `apps/clipline-app/ui/player-core.js`, and `apps/clipline-app/ui/review-player.js` because they contain pre-existing user edits.

## File Map

- `apps/clipline-app/src/service.rs`: full-session finalization and recovery policy.
- `apps/clipline-app/src/app.rs`: non-mutating settings restart preparation and final commit.
- `apps/clipline-app/ui/cloud-core.js`: new pure request-generation/account-key gate.
- `apps/clipline-app/tests/cloud_core.rs`: new Boa behavioral tests for cloud request arbitration.
- `apps/clipline-app/ui/app-core.js`, `cloud.js`, `index.html`: cloud gate state, guarded loading, and script order.
- `apps/clipline-app/src/library.rs`: transactional clip rename and shared app-side sidecar enumeration.
- `apps/clipline-app/src/osu_enrichment.rs`: existing pending-enrichment data contract consumed by rename.
- `crates/clipline-storage/src/lib.rs`: storage inventory, byte accounting, and quota GC.
- `apps/clipline-app/src/cloud.rs`: delete-after-upload sidecar cleanup.
- `crates/clipline-mp4/src/init.rs`: version-0/version-1 `mvhd`, `tkhd`, and `mdhd` layouts.
- `crates/clipline-mp4/src/writer.rs`: overflow-safe media-to-movie duration rescaling.
- `apps/clipline-app/ui/player-core.js`, `review-player.js`: latest-position resolution and cumulative rapid seeking.
- `apps/clipline-app/tests/player_core.rs`, `ui_contract.rs`: pure playback math and wiring contracts.
- `handoff.md`: final reliability milestone and sharp-edge notes.

---

### Task 1: Preserve Recoverable Full-Session Recordings

**Files:**
- Modify: `apps/clipline-app/src/service.rs:1515-1614`
- Test: `apps/clipline-app/src/service.rs:2523-2556`

**Interfaces:**
- Consumes: startup recovery of non-empty `*.mp4.recording` files from `clipline_storage::recover_recording_files`.
- Produces: `handle_full_session_finish_error(temp_path: &Path, events: &Sender<Event>, error: &str)`, which deletes only empty failed recordings and warns with the retained path for non-empty recordings.

- [ ] **Step 1: Write failing recovery-policy tests**

Add these tests beside the existing finalized-session rename tests:

```rust
#[test]
fn finalized_session_rename_preserves_non_empty_temp_on_failure() {
    let dir = TestDir::new("clipline-service", "session-rename-preserve");
    let temp_path = dir.path().join("session.mp4.recording");
    std::fs::write(&temp_path, b"recoverable hybrid mp4").unwrap();
    let recording = FullSessionRecording {
        final_path: dir.path().join("missing-parent").join("session.mp4"),
        temp_path: temp_path.clone(),
        wall_start_unix: 0,
        min_duration_s: 0.0,
    };
    let (tx, rx) = mpsc::channel();

    assert!(!rename_finalized_session(&recording, &tx));
    assert_eq!(std::fs::read(&temp_path).unwrap(), b"recoverable hybrid mp4");
    let Event::Error { message } = rx.try_recv().unwrap() else {
        panic!("expected recovery warning");
    };
    assert!(message.contains("recoverable"), "{message}");
    assert!(message.contains("session.mp4.recording"), "{message}");
}

#[test]
fn failed_full_session_finish_preserves_non_empty_and_removes_empty_temp() {
    let dir = TestDir::new("clipline-service", "session-finish-preserve");
    let recoverable = dir.path().join("recoverable.mp4.recording");
    let empty = dir.path().join("empty.mp4.recording");
    std::fs::write(&recoverable, b"hybrid mp4").unwrap();
    std::fs::write(&empty, b"").unwrap();
    let (tx, rx) = mpsc::channel();

    handle_full_session_finish_error(&recoverable, &tx, "writer failed");
    handle_full_session_finish_error(&empty, &tx, "writer failed");

    assert!(recoverable.exists());
    assert!(!empty.exists());
    let Event::Error { message } = rx.try_recv().unwrap() else {
        panic!("expected recovery warning");
    };
    assert!(message.contains("recoverable.mp4.recording"), "{message}");
}
```

- [ ] **Step 2: Run the focused tests and verify red**

Run: `cargo test -p clipline-app session -- --nocapture`

Expected: FAIL because rename currently deletes the temporary file and `handle_full_session_finish_error` does not exist.

- [ ] **Step 3: Implement the preservation helper and use it on finalization errors**

Add the helper before `rename_finalized_session`:

```rust
fn handle_full_session_finish_error(
    temp_path: &Path,
    events: &Sender<Event>,
    error: &str,
) {
    match std::fs::metadata(temp_path) {
        Ok(metadata) if metadata.is_file() && metadata.len() == 0 => {
            let _ = std::fs::remove_file(temp_path);
            warn_user(events, format!("finish full session: {error}"));
        }
        Ok(_) => warn_user(
            events,
            format!(
                "finish full session: {error}; recoverable recording kept at {temp_path:?}"
            ),
        ),
        Err(metadata_error) if metadata_error.kind() == std::io::ErrorKind::NotFound => {
            warn_user(events, format!("finish full session: {error}"));
        }
        Err(metadata_error) => warn_user(
            events,
            format!(
                "finish full session: {error}; could not inspect {temp_path:?} ({metadata_error}), so it was kept for recovery"
            ),
        ),
    }
}
```

Replace the `finish_full_session_recording` error branch with:

```rust
Err(error) => {
    handle_full_session_finish_error(&recording.temp_path, ctx.events, &error.to_string());
}
```

Replace the ordinary rename-error branch with:

```rust
Err(error) => {
    let recovery = recording
        .temp_path
        .is_file()
        .then(|| format!("; recoverable recording kept at {:?}", recording.temp_path))
        .unwrap_or_default();
    warn_user(
        events,
        format!(
            "finalize full session {:?} -> {:?}: {error}{recovery}",
            recording.temp_path, recording.final_path
        ),
    );
    false
}
```

Do not change `discard_full_session_recording`; the short osu! startup transient remains an intentional discard.

- [ ] **Step 4: Run the service regression tests and verify green**

Run: `cargo test -p clipline-app session -- --nocapture`

Expected: PASS, including the existing missing-file and short-session policy tests.

- [ ] **Step 5: Commit only the recording fix**

```powershell
git add -- apps/clipline-app/src/service.rs
git diff --cached --check
git commit -m "fix(recording): preserve recoverable full sessions"
```

---

### Task 2: Make Settings Restart Preparation Non-Mutating

**Files:**
- Modify: `apps/clipline-app/src/app.rs:466-613`
- Modify: `apps/clipline-app/src/app.rs:1419-1503`
- Test: `apps/clipline-app/src/app.rs:2251-2298`

**Interfaces:**
- Consumes: `AppSettings::to_service_options`, current active-game state, and current decodable codecs.
- Produces: `PreparedRuntimeRestart { settings, next_options, cleared_active_game }`; preparation owns no sender and changes no runtime state. `finish_prepared_restart` becomes the sole commit point.

- [ ] **Step 1: Write the failing late-failure regression test**

Add this app test:

```rust
#[test]
fn prepared_settings_restart_is_non_mutating_until_commit() {
    let (tx, rx) = mpsc::channel();
    let original = AppSettings::default();
    let state = RuntimeState::with_sender(tx, original.clone(), None);
    let mut changed = original.clone();
    changed.fps = 120;

    let prepared = state.prepare_settings_restart(changed).unwrap();

    assert_eq!(state.settings().fps, original.fps);
    assert!(state.send(Cmd::Save), "active sender must remain installed");
    assert!(matches!(rx.try_recv(), Ok(Cmd::Save)));
    assert_eq!(prepared.next_options.as_ref().unwrap().fps, 120);

    drop(prepared); // Simulates a later tray-label or hook-registration failure.
    assert!(state.send(Cmd::Save), "dropping a plan must not stop recording");
    assert!(matches!(rx.try_recv(), Ok(Cmd::Save)));
}
```

- [ ] **Step 2: Run the regression test and verify red**

Run: `cargo test -p clipline-app prepared_settings_restart_is_non_mutating_until_commit -- --nocapture`

Expected: FAIL because current preparation installs the new settings and takes `inner.tx`.

- [ ] **Step 3: Add a prospective-options builder and change the prepared type**

Change the prepared type to:

```rust
struct PreparedRuntimeRestart {
    settings: AppSettings,
    next_options: Option<ServiceOptions>,
    cleared_active_game: bool,
}
```

Replace the body of `RuntimeState::options` with a delegating call and add this helper:

```rust
fn options_for(
    settings: &AppSettings,
    lol_url: Option<String>,
    active_game: Option<&DetectedGame>,
    decodable_codecs: &[service::Codec],
) -> Result<service::ServiceOptions, String> {
    let mut opts = settings.to_service_options(lol_url)?;
    opts.decodable_codecs = decodable_codecs.to_vec();
    if let Some(game) = active_game {
        opts.capture_source = service::CaptureSource::WindowHandle {
            hwnd: game.hwnd,
            title: game.window_title.clone(),
        };
        opts.recording_mode = game.recording_mode.into();
        if crate::game_plugins::contains(&game.id) {
            opts.active_game_plugin_id = Some(game.id.clone());
        }
        opts.active_game = Some(service::ActiveGame {
            id: game.id.clone(),
            name: game.name.clone(),
        });
    }
    Ok(opts)
}

fn options(inner: &RuntimeInner) -> Result<service::ServiceOptions, String> {
    Self::options_for(
        &inner.settings,
        inner.lol_url.clone(),
        inner.active_game.as_ref(),
        &inner.decodable_codecs,
    )
}
```

Replace `prepare_settings_restart` with:

```rust
fn prepare_settings_restart(
    &self,
    settings: AppSettings,
) -> Result<PreparedRuntimeRestart, String> {
    let inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
    let cleared_active_game = inner.active_game.is_some()
        && !active_game_still_configured(&settings, inner.active_game.as_ref());
    let active_game = if cleared_active_game {
        None
    } else {
        inner.active_game.as_ref()
    };
    let next_options = if inner.tx.is_some() {
        let mut options = Self::options_for(
            &settings,
            inner.lol_url.clone(),
            active_game,
            &inner.decodable_codecs,
        )?;
        options.recover_abandoned_recordings = false;
        Some(options)
    } else {
        None
    };
    Ok(PreparedRuntimeRestart {
        settings,
        next_options,
        cleared_active_game,
    })
}
```

- [ ] **Step 4: Make `finish_prepared_restart` the no-gap commit point**

At the start of `finish_prepared_restart`, replace sender handling with:

```rust
let PreparedRuntimeRestart {
    settings,
    next_options,
    cleared_active_game,
} = prepared;
let (old_tx, next_options) = {
    let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
    inner.settings = settings;
    if cleared_active_game {
        inner.active_game = None;
    }
    let old_tx = if next_options.is_some() {
        inner.tx.take()
    } else {
        None
    };
    if old_tx.is_some() {
        inner.last_save_request = None;
    }
    let next_options = if old_tx.is_some() { next_options } else { None };
    (old_tx, next_options)
};
if let Some(tx) = old_tx {
    let _ = tx.send(Cmd::Stop { announce: false });
}
if let Some(options) = next_options {
    let (tx, rx) = service::spawn(options);
    let generation = {
        let mut inner = self.0.lock().map_err(|_| "runtime state lock poisoned")?;
        Self::install_recording_sender(&mut inner, tx)
    };
    pump_events(app.clone(), rx, generation);
}
if cleared_active_game {
    let _ = app.emit("game-detection", GameDetectionEvent::from_detected(None));
}
Ok(())
```

Remove the old duplicate stop/spawn/event block. In `save_settings`, replace the region from quota calculation through restart completion with this ordering:

```rust
let quota_bytes = quota_bytes_from_gb(settings.disk_quota_gb)?;
let prepared_restart = state.prepare_settings_restart(settings.clone())?;
if let Err(error) = settings.save() {
    let shortcuts = app.global_shortcut();
    let _ = sync_global_hotkeys(
        &new_global_hotkeys,
        &old_global_hotkeys,
        |shortcut| shortcuts.is_registered(shortcut),
        |shortcut| shortcuts.register(shortcut),
        |shortcut| shortcuts.unregister(shortcut),
    );
    return Err(error);
}
tray_items.set_hotkey_label(&save_hotkey_label(&settings))?;
crate::hotkeys::set_save_hotkeys(&settings.hotkeys())?;
drop(cloud_save_guard);
state.finish_prepared_restart(app, prepared_restart)?;
storage_settings.set_quota_bytes(quota_bytes);
storage_settings.set_media_dir(media_dir);
Ok(settings)
```

- [ ] **Step 5: Run settings restart tests and verify green**

Run: `cargo test -p clipline-app restart -- --nocapture`

Expected: PASS; the channel remains connected when a prepared restart is dropped.

- [ ] **Step 6: Commit only the settings transaction**

```powershell
git add -- apps/clipline-app/src/app.rs
git diff --cached --check
git commit -m "fix(settings): stage recorder restart transactionally"
```

---

### Task 3: Guard Cloud Library Requests by Generation and Account

**Files:**
- Create: `apps/clipline-app/ui/cloud-core.js`
- Create: `apps/clipline-app/tests/cloud_core.rs`
- Modify: `apps/clipline-app/ui/app-core.js:83-91`
- Modify: `apps/clipline-app/ui/cloud.js:210-239`
- Modify: `apps/clipline-app/ui/index.html:980-986`
- Modify: `apps/clipline-app/tests/ui_contract.rs:13-33`

**Interfaces:**
- Produces: global `CloudCore` with `accountKey(cloud)` and `createRequestGate()`.
- Produces: gate methods `begin(accountKey) -> { generation, accountKey }`, `invalidate() -> number`, and `isCurrent(request, accountKey) -> boolean`.
- Consumes: `cloudClipsRequestGate` from `app-core.js` inside `cloud.js`.

- [ ] **Step 1: Create the failing Boa tests**

Create `apps/clipline-app/tests/cloud_core.rs`:

```rust
use boa_engine::{Context, Source};
use std::fs;
use std::path::Path;

fn context() -> Context {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/cloud-core.js");
    let source = fs::read_to_string(path).expect("read ui/cloud-core.js");
    let mut context = Context::default();
    context
        .eval(Source::from_bytes(&source))
        .expect("cloud-core.js evaluates without DOM or Tauri globals");
    context
}

fn eval(context: &mut Context, expression: &str) -> String {
    context
        .eval(Source::from_bytes(expression))
        .unwrap_or_else(|error| panic!("eval `{expression}`: {error}"))
        .to_string(context)
        .expect("stringify result")
        .to_std_string_escaped()
}

#[test]
fn request_gate_rejects_superseded_and_invalidated_requests() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "const gate = CloudCore.createRequestGate();\
             const first = gate.begin('host|user-a|credential-a');\
             const second = gate.begin('host|user-a|credential-a');\
             JSON.stringify([\
               gate.isCurrent(first, 'host|user-a|credential-a'),\
               gate.isCurrent(second, 'host|user-a|credential-a'),\
               gate.isCurrent(second, 'host|user-b|credential-b'),\
               gate.invalidate(),\
               gate.isCurrent(second, 'host|user-a|credential-a')\
             ])",
        ),
        "[false,true,false,3,false]"
    );
}

#[test]
fn account_key_is_stable_and_account_scoped() {
    let mut context = context();
    assert_eq!(
        eval(
            &mut context,
            "CloudCore.accountKey({\
               host_url: 'https://clips.example',\
               connected_user_id: 'user-7',\
               credential_target: 'credential-7'\
             })",
        ),
        "https://clips.example|user-7|credential-7"
    );
}
```

- [ ] **Step 2: Run the cloud-core tests and verify red**

Run: `cargo test -p clipline-app --test cloud_core -- --nocapture`

Expected: FAIL because `ui/cloud-core.js` does not exist.

- [ ] **Step 3: Implement the pure request gate**

Create `apps/clipline-app/ui/cloud-core.js`:

```javascript
// Pure Clipline Cloud request arbitration. Keep DOM- and Tauri-free for Boa tests.
var CloudCore = (() => {
  const accountKey = (cloud = {}) => [
    cloud.host_url || "",
    cloud.connected_user_id || "",
    cloud.credential_target || "",
  ].map(String).join("|");

  const createRequestGate = () => {
    let generation = 0;
    return {
      begin(key) {
        generation += 1;
        return { generation, accountKey: String(key || "") };
      },
      invalidate() {
        generation += 1;
        return generation;
      },
      isCurrent(request, key) {
        return !!request
          && request.generation === generation
          && request.accountKey === String(key || "");
      },
    };
  };

  return { accountKey, createRequestGate };
})();

globalThis.CloudCore = CloudCore;
```

Run: `cargo test -p clipline-app --test cloud_core -- --nocapture`

Expected: PASS.

- [ ] **Step 4: Write the failing UI wiring contract**

Add `cloud-core.js` before `app-core.js` in `APP_UI_JS`, then add:

```rust
#[test]
fn cloud_library_loader_guards_every_async_result_and_force_supersedes() {
    let cloud = read_ui_js("cloud.js");
    let loader = js_function_body(&cloud, "loadCloudClips");
    assert!(loader.contains("cloudClipsLoading && !force"));
    assert!(loader.contains("cloudClipsRequestGate.begin(accountKey)"));
    assert!(loader.contains("cloudClipsRequestGate.isCurrent(request, cloudAccountKey())"));
    assert!(!loader.contains("if (cloudClipsLoading) return"));

    let html = index_html();
    let cloud_core = html.find("src=\"cloud-core.js\"").unwrap();
    let app_core = html.find("src=\"app-core.js\"").unwrap();
    assert!(cloud_core < app_core);
}
```

Run: `cargo test -p clipline-app --test ui_contract cloud_library_loader_guards_every_async_result_and_force_supersedes -- --nocapture`

Expected: FAIL because the loader has no gate and drops every refresh while loading.

- [ ] **Step 5: Wire the guarded loader**

Load `cloud-core.js` between `player-core.js` and `app-core.js` in `index.html`. Add this state after the cloud clip flags in `app-core.js`:

```javascript
var cloudClipsRequestGate = CloudCore.createRequestGate();
```

Add this helper before `resetCloudClipsCache`:

```javascript
function cloudAccountKey() {
  return CloudCore.accountKey(cloudSettings());
}
```

Replace reset and load with:

```javascript
function resetCloudClipsCache() {
  cloudClipsRequestGate.invalidate();
  cloudClipsCache = [];
  cloudClipsLoaded = false;
  cloudClipsLoading = false;
  cloudClipsError = "";
}

async function loadCloudClips({ force = false } = {}) {
  if (!cloudConnected()) {
    resetCloudClipsCache();
    if (gallerySource === "cloud") renderCloudClips();
    return;
  }
  if (cloudClipsLoading && !force) return;
  if (cloudClipsError && !force) return;
  if (cloudClipsLoaded && !force) return;

  const accountKey = cloudAccountKey();
  const request = cloudClipsRequestGate.begin(accountKey);
  const isCurrent = () => cloudClipsRequestGate.isCurrent(request, cloudAccountKey());
  cloudClipsLoading = true;
  cloudClipsError = "";
  if (gallerySource === "cloud") renderClips();
  try {
    const result = await invoke("list_cloud_clips");
    if (!isCurrent()) return;
    cloudClipsCache = result && Array.isArray(result.clips) ? result.clips : [];
    cloudClipsLoaded = true;
  } catch (error) {
    if (!isCurrent()) return;
    cloudClipsError = String(error);
  } finally {
    if (!isCurrent()) return;
    cloudClipsLoading = false;
    if (gallerySource === "cloud") renderClips();
  }
}
```

Keep the existing connect and disconnect calls to `resetCloudClipsCache`; they now invalidate in-flight work. Replace `reloadSettings` so an identity change cannot leave a stale loading flag:

```javascript
async function reloadSettings() {
  const previousAccountKey = cloudAccountKey();
  const settings = await invoke("get_settings");
  fillSettings(settings);
  if (cloudAccountKey() !== previousAccountKey) resetCloudClipsCache();
  if (clipsCache.length) renderClips();
}
```

- [ ] **Step 6: Run cloud tests and commit only cloud request arbitration**

Run: `cargo test -p clipline-app --test cloud_core --test ui_contract cloud -- --nocapture`

Expected: PASS.

Stage the new files and only the cloud-core script-tag hunk from dirty `index.html`/`ui_contract.rs`:

```powershell
git add -- apps/clipline-app/ui/cloud-core.js apps/clipline-app/tests/cloud_core.rs apps/clipline-app/ui/app-core.js apps/clipline-app/ui/cloud.js
git add -p -- apps/clipline-app/ui/index.html apps/clipline-app/tests/ui_contract.rs
git diff --cached --check
git commit -m "fix(cloud): ignore stale library responses"
```

---

### Task 4: Move and Rewrite Pending osu! Enrichment During Rename

**Files:**
- Modify: `apps/clipline-app/src/library.rs:435-545`
- Test: `apps/clipline-app/src/library.rs:2410-2512`
- Read contract: `apps/clipline-app/src/osu_enrichment.rs:42-56`

**Interfaces:**
- Consumes: `crate::osu_enrichment::pending_path` and `OsuPendingEnrichment`.
- Produces: `PreparedOsuSidecarMove::stage(source_clip, target_clip)`, `commit`, `finish`, and `rollback`, with staged-file cleanup owned by `Drop`.

- [ ] **Step 1: Write failing success, malformed-input, collision, and rollback tests**

Add a test helper:

```rust
fn pending_osu_enrichment(clip: &Path) -> crate::osu_enrichment::OsuPendingEnrichment {
    crate::osu_enrichment::OsuPendingEnrichment {
        schema_version: 1,
        clip_path: clip.display().to_string(),
        recording_start_unix: 10,
        recording_end_unix: 20,
        clip_duration_s: 10.0,
        status: crate::osu_enrichment::OsuEnrichmentStatus::Pending,
        attempts: 0,
        pagination_ceiling_reached: false,
        title_events: Vec::new(),
        message: None,
    }
}
```

Add these cases around existing rename tests:

```rust
#[test]
fn rename_clip_file_moves_pending_osu_sidecar_and_rewrites_clip_path() {
    let dir = TestDir::new("clipline-library", "rename-osu-pending");
    let source = dir.path().join("session_1.mp4");
    let target = dir.path().join("Ranked win.mp4");
    touch_mp4(&source);
    std::fs::write(
        crate::osu_enrichment::pending_path(&source),
        serde_json::to_vec_pretty(&pending_osu_enrichment(&source)).unwrap(),
    )
    .unwrap();

    rename_clip_files(
        source.clone(),
        source.display().to_string(),
        normalized_clip_file_name("Ranked win").unwrap(),
    )
    .unwrap();

    assert!(!crate::osu_enrichment::pending_path(&source).exists());
    let moved: crate::osu_enrichment::OsuPendingEnrichment = serde_json::from_slice(
        &std::fs::read(crate::osu_enrichment::pending_path(&target)).unwrap(),
    )
    .unwrap();
    assert_eq!(moved.clip_path, target.display().to_string());
}

#[test]
fn rename_clip_file_rejects_malformed_pending_osu_before_moving_mp4() {
    let dir = TestDir::new("clipline-library", "rename-osu-malformed");
    let source = dir.path().join("session_1.mp4");
    touch_mp4(&source);
    std::fs::write(crate::osu_enrichment::pending_path(&source), b"not json").unwrap();

    let error = match rename_clip_files(
        source.clone(),
        source.display().to_string(),
        normalized_clip_file_name("Ranked win").unwrap(),
    ) {
        Ok(_) => panic!("malformed pending enrichment must stop the rename"),
        Err(error) => error,
    };

    assert!(error.contains("osu! enrichment"), "{error}");
    assert!(source.exists());
    assert!(crate::osu_enrichment::pending_path(&source).exists());
    assert!(!dir.path().join("Ranked win.mp4").exists());
}

#[test]
fn rename_clip_file_rejects_pending_osu_destination_collision() {
    let dir = TestDir::new("clipline-library", "rename-osu-collision");
    let source = dir.path().join("session_1.mp4");
    let target = dir.path().join("Ranked win.mp4");
    touch_mp4(&source);
    std::fs::write(
        crate::osu_enrichment::pending_path(&source),
        serde_json::to_vec(&pending_osu_enrichment(&source)).unwrap(),
    )
    .unwrap();
    std::fs::write(crate::osu_enrichment::pending_path(&target), b"occupied").unwrap();

    let error = match rename_clip_files(
        source.clone(),
        source.display().to_string(),
        normalized_clip_file_name("Ranked win").unwrap(),
    ) {
        Ok(_) => panic!("pending enrichment destination collision must stop the rename"),
        Err(error) => error,
    };

    assert!(error.contains("osu! enrichment sidecar"), "{error}");
    assert!(source.exists());
}
```

Extend `rename_clip_file_rolls_back_when_final_metadata_write_fails` to create a valid pending sidecar and assert that the source pending file is restored, the target pending file is absent, and its embedded `clip_path` still names the source.

Insert this setup immediately after `touch_mp4(&source)` in that test:

```rust
let original_pending = pending_osu_enrichment(&source);
std::fs::write(
    crate::osu_enrichment::pending_path(&source),
    serde_json::to_vec_pretty(&original_pending).unwrap(),
)
.unwrap();
```

After the existing error and MP4/marker rollback assertions, insert:

```rust
assert!(crate::osu_enrichment::pending_path(&source).exists());
assert!(!crate::osu_enrichment::pending_path(&target).exists());
let restored: crate::osu_enrichment::OsuPendingEnrichment = serde_json::from_slice(
    &std::fs::read(crate::osu_enrichment::pending_path(&source)).unwrap(),
)
.unwrap();
assert_eq!(restored.clip_path, source.display().to_string());
```

- [ ] **Step 2: Run rename tests and verify red**

Run: `cargo test -p clipline-app rename_clip_file_ -- --nocapture`

Expected: FAIL because pending enrichment is ignored and malformed input does not stop the MP4 move.

- [ ] **Step 3: Add the staged pending-sidecar transaction**

Add this transaction type near `rename_clip_files`:

```rust
struct PreparedOsuSidecarMove {
    source: PathBuf,
    target: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
}

impl PreparedOsuSidecarMove {
    fn stage(source_clip: &Path, target_clip: &Path) -> Result<Option<Self>, String> {
        let source = crate::osu_enrichment::pending_path(source_clip);
        if !source.exists() {
            return Ok(None);
        }
        let target = crate::osu_enrichment::pending_path(target_clip);
        let target_is_source = match (target.canonicalize(), source.canonicalize()) {
            (Ok(target), Ok(source)) => target == source,
            _ => target == source,
        };
        if target.exists() && !target_is_source {
            return Err("an osu! enrichment sidecar with that name already exists".into());
        }
        let bytes = std::fs::read(&source)
            .map_err(|error| format!("read osu! enrichment sidecar {source:?}: {error}"))?;
        let mut pending: crate::osu_enrichment::OsuPendingEnrichment =
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("parse osu! enrichment sidecar {source:?}: {error}"))?;
        pending.clip_path = target_clip.display().to_string();
        let staged = target.with_extension("osu-enrichment.rename.tmp");
        let backup = source.with_extension("osu-enrichment.rename.backup");
        if staged.exists() {
            return Err(format!("staged osu! enrichment path already exists: {staged:?}"));
        }
        if backup.exists() {
            return Err(format!("backup osu! enrichment path already exists: {backup:?}"));
        }
        let json = serde_json::to_vec_pretty(&pending)
            .map_err(|error| format!("serialize osu! enrichment sidecar: {error}"))?;
        std::fs::write(&staged, json)
            .map_err(|error| format!("stage osu! enrichment sidecar {staged:?}: {error}"))?;
        Ok(Some(Self {
            source,
            target,
            staged,
            backup,
        }))
    }

    fn commit(&self) -> Result<(), String> {
        std::fs::rename(&self.source, &self.backup)
            .map_err(|error| format!("stage old osu! enrichment sidecar: {error}"))?;
        std::fs::rename(&self.staged, &self.target)
            .map_err(|error| {
                let _ = std::fs::rename(&self.backup, &self.source);
                format!("install renamed osu! enrichment sidecar: {error}")
            })?;
        Ok(())
    }

    fn finish(&self) {
        let _ = std::fs::remove_file(&self.backup);
    }

    fn rollback(&self) {
        let _ = std::fs::remove_file(&self.target);
        if self.backup.exists() && !self.source.exists() {
            let _ = std::fs::rename(&self.backup, &self.source);
        }
        let _ = std::fs::remove_file(&self.staged);
    }
}

impl Drop for PreparedOsuSidecarMove {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.staged);
    }
}
```

Stage the object before the first MP4 move:

```rust
let pending_osu_move = PreparedOsuSidecarMove::stage(&source, &target)?;
```

The type's `Drop` implementation cleans the staged JSON on every existing MP4/marker/metadata early-return path. After the MP4, marker, and existing metadata moves succeed, commit the pending sidecar and roll back those moves if installation fails:

```rust
if let Some(pending) = &pending_osu_move {
    if let Err(error) = pending.commit() {
        rollback_renamed_clip_files(
            &source,
            &target,
            &source_markers,
            &target_markers,
            moved_metadata.then_some((source_metadata.as_path(), target_metadata.as_path())),
        );
        return Err(error);
    }
}
```

If the final `write_clip_metadata` fails, restore the pending sidecar before the existing rollback:

```rust
if let Err(error) = write_clip_metadata(&target, &target_metadata_value) {
    if let Some(pending) = &pending_osu_move {
        pending.rollback();
    }
    rollback_renamed_clip_files(
        &source,
        &target,
        &source_markers,
        &target_markers,
        moved_metadata.then_some((source_metadata.as_path(), target_metadata.as_path())),
    );
    return Err(error);
}
if let Some(pending) = &pending_osu_move {
    pending.finish();
}
```

Move poster handling after the final metadata write succeeds. A poster remains a best-effort regenerable cache and therefore cannot complicate persistent-data rollback.

- [ ] **Step 4: Run rename tests and verify green**

Run: `cargo test -p clipline-app rename_clip_file_ -- --nocapture`

Expected: PASS for success, malformed input, destination collision, and rollback.

- [ ] **Step 5: Commit the rename transaction**

```powershell
git add -- apps/clipline-app/src/library.rs
git diff --cached --check
git commit -m "fix(library): carry osu enrichment through rename"
```

---

### Task 5: Include Clip Metadata in Every Sidecar Lifecycle

**Files:**
- Modify: `crates/clipline-storage/src/lib.rs:257-287`
- Test: `crates/clipline-storage/src/lib.rs:334-446`
- Modify: `apps/clipline-app/src/library.rs:313-350`
- Test: `apps/clipline-app/src/library.rs:2617-2679`
- Modify: `apps/clipline-app/src/cloud.rs:1517-1525`
- Test: `apps/clipline-app/src/cloud.rs:2066-2085`

**Interfaces:**
- Produces: storage-local `clip_metadata_path(path) -> PathBuf`.
- Produces: app-local `pub(crate) fn clip_sidecar_paths(path: &Path) -> [PathBuf; 4]` shared by manual delete and cloud delete-after-upload.

- [ ] **Step 1: Write failing storage accounting and GC tests**

Rename the status test to `status_counts_clip_metadata_and_other_sidecars`, create `a.clipline.json` with five bytes, and expect `25` total bytes. Add:

```rust
#[test]
fn enforce_quota_deletes_clip_metadata_sidecar_with_clip() {
    let dir = TestDir::new("clipline-storage", "clip-metadata-delete");
    let old = dir.write("old.mp4", 10);
    let metadata = dir.write("old.clipline.json", 6);
    tick_mtime();
    let keep = dir.write("keep.mp4", 10);

    let report = enforce_quota(dir.path(), Some(10), None).unwrap();

    assert_eq!(report.deleted_clips, 1);
    assert_eq!(report.freed_bytes, 16);
    assert!(!old.exists());
    assert!(!metadata.exists());
    assert!(keep.exists());
    assert_eq!(report.status.total_bytes, 10);
}
```

Add `old.clipline.json` to `enforce_quota_crosses_folders_and_removes_emptied_session_dirs` and assert both the metadata file and emptied session folder disappear.

Run: `cargo test -p clipline-storage clip_metadata -- --nocapture`

Expected: FAIL because `.clipline.json` is not in `clip_sidecars`.

- [ ] **Step 2: Add clip metadata to storage inventory**

Add:

```rust
fn clip_metadata_path(path: &Path) -> PathBuf {
    path.with_extension("clipline.json")
}
```

Use this complete candidate list in `clip_sidecars`:

```rust
for candidate in [
    sidecar_path(clip),
    clip_metadata_path(clip),
    osu_pending_path(clip),
    poster_path(clip),
] {
```

Update the `ClipFile::sidecars` and `clip_sidecars` comments to name all four files.

Run: `cargo test -p clipline-storage -- --nocapture`

Expected: PASS.

- [ ] **Step 3: Write failing app deletion tests**

In `delete_clips_impl_handles_partial_success_and_sidecars`, create `a.clipline.json` and `b.clipline.json`, then assert both are deleted and an unselected `c.clipline.json` remains. In `delete_uploaded_local_files_removes_poster_sidecar`, create `clip.clipline.json` and assert it is deleted.

Run: `cargo test -p clipline-app delete_ -- --nocapture`

Expected: the library assertion already passes through its direct metadata call, while the cloud assertion FAILS because delete-after-upload omits clip metadata. The mixed result proves the lifecycle implementations have diverged.

- [ ] **Step 4: Centralize app-side sidecar enumeration**

Add in `library.rs`:

```rust
pub(crate) fn clip_sidecar_paths(target: &Path) -> [PathBuf; 4] {
    [
        target.with_extension("markers.json"),
        clip_metadata_path(target),
        crate::osu_enrichment::pending_path(target),
        crate::poster::poster_path(target),
    ]
}
```

Replace individual best-effort removals in `remove_clip_files` with:

```rust
for sidecar in clip_sidecar_paths(target) {
    let _ = std::fs::remove_file(sidecar);
}
```

Replace cloud's individual best-effort removals with the same loop over `crate::library::clip_sidecar_paths(target)`.

- [ ] **Step 5: Run storage/app tests and commit lifecycle coverage**

Run: `cargo test -p clipline-storage`

Then run: `cargo test -p clipline-app delete_ -- --nocapture`

Expected: PASS.

```powershell
git add -- crates/clipline-storage/src/lib.rs apps/clipline-app/src/library.rs apps/clipline-app/src/cloud.rs
git diff --cached --check
git commit -m "fix(storage): include clip metadata in cleanup"
```

---

### Task 6: Emit Version-1 MP4 Duration Headers When Needed

**Files:**
- Modify: `crates/clipline-mp4/src/init.rs:152-272`
- Test: `crates/clipline-mp4/src/init.rs:434-587`
- Modify: `crates/clipline-mp4/src/writer.rs:349-356`
- Test: `crates/clipline-mp4/src/writer.rs:668-685`

**Interfaces:**
- Produces: `mdhd(timescale: u32, duration_media_ts: u64) -> Vec<u8>` with version selected by duration width.
- Produces: `rescale_duration(duration: u64, source_timescale: u32, target_timescale: u32) -> u64` using `u128` arithmetic.
- Keeps all public writer APIs unchanged.

- [ ] **Step 1: Write failing box-boundary tests**

Add test helpers and tests inside `init.rs`:

```rust
fn box_version(bytes: &[u8], info: &crate::walker::BoxInfo) -> u8 {
    bytes[info.payload_offset as usize]
}

#[test]
fn duration_headers_keep_version_zero_at_u32_max() {
    let duration = u32::MAX as u64;
    let movie = mvhd(duration, 2);
    let movie_box = walk(&movie).remove(0);
    assert_eq!(box_version(&movie, &movie_box), 0);

    let track = video_trak_with_tables(&cfg(), 1, duration, duration, empty_stbl_tail());
    let trak = walk(&track).remove(0);
    let trak_children = children(&track, &trak);
    let tkhd = find(&trak_children, b"tkhd").unwrap();
    assert_eq!(box_version(&track, tkhd), 0);
    let mdia = find(&trak_children, b"mdia").unwrap();
    let mdhd = find(&children(&track, mdia), b"mdhd").unwrap().clone();
    assert_eq!(box_version(&track, &mdhd), 0);
}

#[test]
fn duration_headers_use_version_one_and_preserve_first_u64_value() {
    let duration = u32::MAX as u64 + 1;
    let movie = mvhd(duration, 2);
    let movie_box = walk(&movie).remove(0);
    let movie_payload = movie_box.payload_offset as usize;
    assert_eq!(box_version(&movie, &movie_box), 1);
    assert_eq!(
        u64::from_be_bytes(movie[movie_payload + 24..movie_payload + 32].try_into().unwrap()),
        duration
    );

    let track = video_trak_with_tables(&cfg(), 1, duration, duration, empty_stbl_tail());
    let trak = walk(&track).remove(0);
    let trak_children = children(&track, &trak);
    let tkhd = find(&trak_children, b"tkhd").unwrap();
    let tkhd_payload = tkhd.payload_offset as usize;
    assert_eq!(box_version(&track, tkhd), 1);
    assert_eq!(
        u64::from_be_bytes(track[tkhd_payload + 28..tkhd_payload + 36].try_into().unwrap()),
        duration
    );
    let mdia = find(&trak_children, b"mdia").unwrap();
    let mdhd = find(&children(&track, mdia), b"mdhd").unwrap().clone();
    let mdhd_payload = mdhd.payload_offset as usize;
    assert_eq!(box_version(&track, &mdhd), 1);
    assert_eq!(
        u64::from_be_bytes(track[mdhd_payload + 24..mdhd_payload + 32].try_into().unwrap()),
        duration
    );
}
```

Run: `cargo test -p clipline-mp4 duration_headers_ -- --nocapture`

Expected: FAIL because every box is version 0 and truncates the duration.

- [ ] **Step 2: Implement conditional version-1 layouts**

For each header, select `let version = u8::from(duration > u32::MAX as u64)`. Use these exact leading layouts before appending each box's unchanged common fields:

```rust
// mvhd
if version == 1 {
    p.u64(0)
        .u64(0)
        .u32(MOVIE_TIMESCALE)
        .u64(duration_movie_ts);
} else {
    p.u32(0)
        .u32(0)
        .u32(MOVIE_TIMESCALE)
        .u32(duration_movie_ts as u32);
}

// tkhd
if version == 1 {
    p.u64(0)
        .u64(0)
        .u32(track_id)
        .u32(0)
        .u64(duration_movie_ts);
} else {
    p.u32(0)
        .u32(0)
        .u32(track_id)
        .u32(0)
        .u32(duration_movie_ts as u32);
}

// mdhd
if version == 1 {
    p.u64(0)
        .u64(0)
        .u32(timescale)
        .u64(duration_media_ts);
} else {
    p.u32(0)
        .u32(0)
        .u32(timescale)
        .u32(duration_media_ts as u32);
}
```

Pass `version` to each `full_box`. Extract mdhd construction from `mdia_generic` into `fn mdhd(timescale, duration_media_ts)` so the layout is isolated and testable. Append all existing rate, volume, matrix, flags, language, and geometry fields unchanged.

- [ ] **Step 3: Write the failing overflow-safe rescale test**

Add in `writer.rs`:

```rust
#[test]
fn duration_rescale_uses_wide_intermediate() {
    let duration = u64::MAX;
    let expected = ((duration as u128 * MOVIE_TIMESCALE as u128) / 90_000u128) as u64;
    assert_eq!(rescale_duration(duration, 90_000, MOVIE_TIMESCALE), expected);
}
```

Run: `cargo test -p clipline-mp4 duration_rescale_uses_wide_intermediate -- --nocapture`

Expected: FAIL because `rescale_duration` does not exist; using the old expression with this input would overflow before division.

- [ ] **Step 4: Implement u128 rescaling and run MP4 tests**

Add:

```rust
fn rescale_duration(duration: u64, source_timescale: u32, target_timescale: u32) -> u64 {
    let scaled = duration as u128 * target_timescale as u128 / source_timescale as u128;
    scaled.min(u64::MAX as u128) as u64
}
```

Change `TrackState::duration_movie_ts` to:

```rust
fn duration_movie_ts(&self) -> u64 {
    rescale_duration(
        self.duration_media_ts(),
        self.cfg.timescale(),
        MOVIE_TIMESCALE,
    )
}
```

Run: `cargo test -p clipline-mp4 -- --nocapture`

Expected: PASS, including ordinary short-file version-0 tests and version-1 boundary tests.

- [ ] **Step 5: Commit the MP4 format fix**

```powershell
git add -- crates/clipline-mp4/src/init.rs crates/clipline-mp4/src/writer.rs
git diff --cached --check
git commit -m "fix(mp4): write 64-bit duration boxes when needed"
```

---

### Task 7: Preserve the Latest Position Across Audio-Preview Swaps

**Files:**
- Modify: `apps/clipline-app/ui/player-core.js`
- Test: `apps/clipline-app/tests/player_core.rs`
- Modify: `apps/clipline-app/ui/review-player.js:115-189`
- Modify: `apps/clipline-app/ui/review-player.js:891-927`
- Test: `apps/clipline-app/tests/ui_contract.rs`

**Interfaces:**
- Produces: `PlayerCore.sourceSwapResumeTime(pendingSeek, currentTime, fallbackTime) -> number`.
- Produces: `PlayerCore.relativeSeekTarget(currentTime, pendingSeek, delta, duration) -> number`.
- Consumes: `pendingSeek` in `review-player.js`; source swap consumes it once, and repeated relative seeks accumulate from it.

- [ ] **Step 1: Write failing pure playback tests**

Add to `tests/player_core.rs`:

```rust
#[test]
fn source_swap_resume_time_prefers_latest_queued_seek() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval(&mut ctx, "PlayerCore.sourceSwapResumeTime(25, 5, 0)"),
        "25"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.sourceSwapResumeTime(null, 18, 0)"),
        "18"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.sourceSwapResumeTime(null, NaN, 7)"),
        "7"
    );
}

#[test]
fn relative_seek_accumulates_from_pending_target() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval(&mut ctx, "PlayerCore.relativeSeekTarget(5, 10, 5, 60)"),
        "15"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.relativeSeekTarget(58, null, 5, 60)"),
        "60"
    );
}
```

Run: `cargo test -p clipline-app --test player_core source_swap_resume_time -- --nocapture`

Then run: `cargo test -p clipline-app --test player_core relative_seek_accumulates -- --nocapture`

Expected: FAIL because neither pure helper exists.

- [ ] **Step 2: Implement and export the pure playback rules**

Add beside `clampTime`:

```javascript
const sourceSwapResumeTime = (pendingSeek, currentTime, fallbackTime) => {
  if (Number.isFinite(pendingSeek)) return Math.max(0, pendingSeek);
  if (Number.isFinite(currentTime)) return Math.max(0, currentTime);
  return Number.isFinite(fallbackTime) ? Math.max(0, fallbackTime) : 0;
};

const relativeSeekTarget = (currentTime, pendingSeek, delta, duration) => {
  const base = Number.isFinite(pendingSeek)
    ? pendingSeek
    : Number.isFinite(currentTime) ? currentTime : 0;
  return clampTime(base + (Number.isFinite(delta) ? delta : 0), duration);
};
```

Add both names to the returned `PlayerCore` object.

Run: `cargo test -p clipline-app --test player_core source_swap_resume_time -- --nocapture`

Then run: `cargo test -p clipline-app --test player_core relative_seek_accumulates -- --nocapture`

Expected: PASS.

- [ ] **Step 3: Write the failing async-wiring contract**

Add to `tests/ui_contract.rs`:

```rust
#[test]
fn audio_preview_resolves_resume_position_after_await() {
    let review = read_ui_js("review-player.js");
    let body = js_function_body(&review, "applySelectedAudioTracksToPlayback");
    let await_preview = body.find("await invoke(\"preview_clip_audio_tracks\"").unwrap();
    let consume_latest = body.find("consumeSourceSwapResumeTime(resumeTime)").unwrap();
    assert!(await_preview < consume_latest);

    let seek_by = js_function_body(&review, "seekBy");
    assert!(seek_by.contains("PlayerCore.relativeSeekTarget"));
    assert!(seek_by.contains("pendingSeek"));
}
```

Run: `cargo test -p clipline-app --test ui_contract audio_preview_resolves_resume_position_after_await -- --nocapture`

Expected: FAIL because `resumeTime` is captured before the await and `seekBy` ignores the queued target.

- [ ] **Step 4: Consume the latest seek only when the source is ready to swap**

Add after the `pendingSeek` declaration:

```javascript
function consumeSourceSwapResumeTime(fallbackTime) {
  const resumeTime = PlayerCore.sourceSwapResumeTime(
    pendingSeek,
    video.currentTime,
    fallbackTime,
  );
  pendingSeek = null;
  return resumeTime;
}
```

Keep the request-start `resumeTime` only as a fallback. Immediately after the sequence/current-clip guard following `await invoke`, add:

```javascript
const latestResumeTime = consumeSourceSwapResumeTime(resumeTime);
setReviewVideoSource(path, {
  resumeTime: latestResumeTime,
  shouldResume,
  rate,
  trimRange,
});
```

Remove the old `setReviewVideoSource(path, { resumeTime, shouldResume, rate, trimRange })`. In the ffmpeg-unavailable catch branch, call `consumeSourceSwapResumeTime(resumeTime)` before swapping back to the original clip path so the fallback path obeys the same rule.

Replace `seekBy` with:

```javascript
function seekBy(delta) {
  seekTo(PlayerCore.relativeSeekTarget(
    video.currentTime,
    pendingSeek,
    delta,
    clipDuration(),
  ));
}
```

This makes five rapid +5-second clicks request +25 seconds even while the first seek remains in flight.

- [ ] **Step 5: Run player and UI tests**

Run: `cargo test -p clipline-app --test player_core --test ui_contract -- --nocapture`

Expected: PASS, including the existing intentional `marker_navigation_skips_nearby_and_wraps` test.

- [ ] **Step 6: Commit only the playback-race hunks**

Use interactive staging because all four files contain unrelated user shortcut edits:

```powershell
git add -p -- apps/clipline-app/ui/player-core.js apps/clipline-app/ui/review-player.js apps/clipline-app/tests/player_core.rs apps/clipline-app/tests/ui_contract.rs
git diff --cached --check
git diff --cached
git commit -m "fix(player): preserve seeks across audio preview swaps"
```

Confirm the cached diff contains the new helpers/tests and preview-swap/seekBy wiring only. The user's Arrow/J/L shortcut hunks must remain unstaged.

---

### Task 8: Full Verification, Handoff, and User Test Build

**Files:**
- Modify: `handoff.md`

**Interfaces:**
- Consumes: all seven independently committed fixes.
- Produces: green workspace tests/clippy, an updated handoff, and a running app for manual playback verification.

- [ ] **Step 1: Run formatting without absorbing unrelated files**

Run: `cargo fmt --all -- --check`

Expected: PASS. If it fails only in files changed by these tasks, run `cargo fmt --all`, inspect the full diff, and keep unrelated user hunks unchanged.

- [ ] **Step 2: Run the complete workspace test gate**

Run: `cargo test --workspace`

Expected: PASS with no failed tests. Hardware-only tests may self-skip according to their existing guards.

- [ ] **Step 3: Run the complete lint gate**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS with zero warnings.

- [ ] **Step 4: Recheck cache-sensitive changed crates**

Run:

```powershell
cargo clean -p clipline-storage
cargo clippy -p clipline-storage --all-targets -- -D warnings
cargo clean -p clipline-mp4
cargo clippy -p clipline-mp4 --all-targets -- -D warnings
cargo clean -p clipline-app
cargo clippy -p clipline-app --all-targets -- -D warnings
```

Expected: all three fresh clippy runs PASS with zero warnings.

- [ ] **Step 5: Update the handoff**

Update the current-state date to `2026-07-09`, change the count to thirty-five milestones, and add this milestone after item 34:

```markdown
35. **Reliability and playback hardening** — Full-session finalization now retains non-empty
    `.mp4.recording` files for startup recovery when writer finalization or the final rename fails.
    Settings changes plan recorder options without taking the active command sender and commit the
    restart only after persistence/tray/hook work succeeds. Cloud-library loads are account-scoped
    and generation-guarded, forced refreshes supersede in-flight requests, renamed clips carry and
    rewrite pending osu! enrichment, and all deletion/quota paths include markers, clip metadata,
    pending enrichment, and posters. Finalized MP4s switch `mvhd`/`tkhd`/`mdhd` to version 1 above
    `u32::MAX`, with `u128` duration rescaling. Multi-audio preview swaps resolve the playhead after
    generation completes, consume the latest queued seek, and rapid relative seeks accumulate.
```

Add these bullets under Sharp Edges:

```markdown
- **Async audio previews replace the video source:** never restore a playhead captured before the
  preview await. Resolve and consume `pendingSeek` immediately before `video.src` changes, and base
  repeated relative seeks on the queued target rather than stale `video.currentTime`.
- **Long finalized MP4s need version-1 duration boxes:** `mvhd`, `tkhd`, and each `mdhd` must switch
  independently when its duration exceeds `u32::MAX`; use a `u128` intermediate when rescaling.
```

Do not claim manual playback verification until Step 7 is complete.

- [ ] **Step 6: Commit the verified handoff**

```powershell
git add -- handoff.md
git diff --cached --check
git commit -m "docs: update reliability fix handoff"
```

- [ ] **Step 7: Stop the old app and launch the verified build**

Run:

```powershell
Get-Process clipline-app -ErrorAction SilentlyContinue | Stop-Process
cargo run -p clipline-app
```

Expected: Clipline starts successfully. Leave it open for the user.

- [ ] **Step 8: Give the user exact manual checks**

Ask the user to verify:

1. Open a clip with multiple audio tracks and immediately click Forward 5s five times while the preview is still being generated; the playhead should land about 25 seconds later, never at zero.
2. Repeat across several visible timeline events; event boundaries must not alter the result.
3. Change selected audio tracks after seeking; the preview swap must preserve the latest playhead and play/pause state.
4. Confirm ordinary marker Next/Previous still wraps at clip ends.

Do not require the user to manufacture filesystem rename failures, 13-hour MP4s, cloud account races, or settings hook failures; those are covered by deterministic regression tests.
