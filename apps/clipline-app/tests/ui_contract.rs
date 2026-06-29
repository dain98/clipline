//! Structural contract for the review player DOM: Clipline owns the controls,
//! the browser owns nothing, and the UI stays split into testable assets.

use std::fs;
use std::path::Path;

fn index_html() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/index.html");
    fs::read_to_string(path).expect("read ui/index.html")
}

fn main_js() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/main.js");
    fs::read_to_string(path).expect("read ui/main.js")
}

fn styles_css() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/styles.css");
    fs::read_to_string(path).expect("read ui/styles.css")
}

fn app_rs() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app.rs");
    fs::read_to_string(path).expect("read src/app.rs")
}

fn tauri_config() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    fs::read_to_string(path).expect("read tauri.conf.json")
}

fn cargo_toml() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    fs::read_to_string(path).expect("read Cargo.toml")
}

#[test]
fn default_capability_only_targets_main_window() {
    let capability =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities/default.json"))
            .expect("read default capability");

    assert!(
        capability.contains("\"windows\": [\"main\"]"),
        "frontend commands should only target Clipline's main window"
    );
    assert!(
        !capability.contains("main-recovery"),
        "recovery windows are intentionally not created or granted frontend command permissions"
    );
}

#[test]
fn native_shell_prevents_duplicate_clipline_instances() {
    let manifest = cargo_toml();
    let app = app_rs();
    let single_instance_plugin = "tauri_plugin_single_instance::init";
    let single_instance = app
        .find(single_instance_plugin)
        .expect("native shell should register the Tauri single-instance plugin");
    let autostart = app
        .find("tauri_plugin_autostart::init")
        .expect("native shell should register autostart");

    assert!(
        manifest.contains("tauri-plugin-single-instance"),
        "Cargo.toml should depend on the single-instance plugin"
    );
    assert!(
        single_instance < autostart,
        "single-instance plugin must be registered before autostart or other shell plugins"
    );
}

#[test]
fn windows_installer_repairs_webview2_with_bootstrapper() {
    let config = tauri_config();

    assert!(
        config.contains("\"minimumWebview2Version\": \"120.0.2210.55\""),
        "Windows 10 installs must repair/update stale WebView2 runtimes before Clipline starts"
    );
    assert!(
        config.contains("\"webviewInstallMode\"")
            && config.contains("\"type\": \"embedBootstrapper\""),
        "the default NSIS installer should embed the small Evergreen bootstrapper instead of bundling the offline WebView2 installer"
    );
}

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

fn library_rs() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/library.rs");
    fs::read_to_string(path).expect("read src/library.rs")
}

fn tag_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=\"");
    let start = tag.find(&prefix)? + prefix.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

#[test]
fn audio_preview_command_scopes_generated_preview_files() {
    let library = library_rs();

    assert!(
        library.contains("AppHandle")
            && library.contains("allow_audio_preview_asset")
            && library.contains("asset_protocol_scope")
            && library.contains("allow_file(preview"),
        "selected-audio preview MP4s are generated under AppData and must be exact-scoped before the player loads them"
    );
}

#[test]
fn review_player_owns_all_controls() {
    let html = index_html();

    let video_start = html.find("<video").expect("video element exists");
    let video_end = html[video_start..]
        .find('>')
        .map(|offset| video_start + offset)
        .expect("video element closes");
    assert!(
        !html[video_start..=video_end].contains("controls"),
        "the review player must not expose native browser video controls"
    );

    for required in [
        "id=\"play-toggle\"",
        "id=\"seek-back\"",
        "id=\"seek-forward\"",
        "id=\"prev-marker\"",
        "id=\"next-marker\"",
        "id=\"marker-count\"",
        "id=\"timeline\"",
        "id=\"trim-band\"",
        "id=\"handle-in\"",
        "id=\"handle-out\"",
        "id=\"time-readout\"",
        "id=\"rate-select\"",
        "id=\"mute-toggle\"",
        "id=\"volume-slider\"",
        "id=\"export-clip\"",
        "id=\"trim-summary\"",
        "id=\"keys-dialog\"",
        "id=\"keys-close\"",
        "id=\"delete-clip\"",
        "id=\"ruler\"",
        "id=\"overview\"",
        "id=\"overview-trim\"",
        "id=\"overview-markers\"",
        "id=\"overview-playhead\"",
        "id=\"overview-window\"",
        "id=\"overview-window-l\"",
        "id=\"overview-window-r\"",
        "id=\"game-event-rail\"",
        "id=\"game-event-rail-title\"",
        "id=\"game-event-rail-summary\"",
        "id=\"game-event-rail-toggle\"",
        "id=\"game-event-list\"",
        "id=\"game-metadata-panel\"",
        "id=\"game-metadata-fields\"",
        "id=\"zoom-out\"",
        "id=\"zoom-fit\"",
        "id=\"zoom-in\"",
        "id=\"snap-toggle\"",
        "id=\"audio-track-panel\"",
        "id=\"audio-track-summary\"",
        "id=\"audio-track-list\"",
        "id=\"open-folder\"",
        "id=\"stage-frame\"",
        "id=\"copy-clip\"",
        "id=\"stage-overlay\"",
        "id=\"memory-usage\"",
        "id=\"rail-hotkey\"",
        "id=\"rail-dot\"",
        "id=\"rail-status-text\"",
        "id=\"rail-status\"",
        "id=\"rail-status\" title=\"Stop recording\" aria-pressed=\"true\"",
        "id=\"rail-save\"",
        "id=\"rail-library-status\"",
        "id=\"rail-clips-count\"",
        "id=\"rail-profile\"",
        "id=\"rail-profile-avatar\"",
        "id=\"rail-profile-name\"",
        "id=\"rail-settings\"",
        "id=\"confirm-dialog\"",
        "id=\"confirm-accept\"",
        "id=\"confirm-cancel\"",
        "id=\"quit-dialog\"",
        "id=\"quit-accept\"",
        "id=\"quit-cancel\"",
        "id=\"update-dialog\"",
        "id=\"update-install\"",
        "id=\"update-cancel\"",
        "id=\"update-dialog-title\"",
        "id=\"update-dialog-body\"",
        "id=\"settings-page\"",
        "id=\"settings-tabs\"",
        "id=\"set-open-on-startup\"",
        "id=\"set-close-to-tray\"",
        "id=\"set-minimize-to-tray\"",
        "id=\"set-update-channel\"",
        "id=\"check-updates\"",
        "id=\"update-status\"",
        "id=\"set-capture\"",
        "id=\"set-backend\"",
        "id=\"backend-summary\"",
        "id=\"set-output-enabled\"",
        "id=\"set-audio-split-output\"",
        "id=\"set-output-device\"",
        "id=\"set-output-volume\"",
        "id=\"output-volume-summary\"",
        "id=\"set-mic-enabled\"",
        "id=\"set-mic-device\"",
        "id=\"set-mic-volume\"",
        "id=\"mic-volume-summary\"",
        "id=\"set-mic-mono\"",
        "id=\"test-mic\"",
        "id=\"mic-meter-fill\"",
        "id=\"mic-test-status\"",
        "id=\"capture-region-editor\"",
        "id=\"display-map\"",
        "id=\"display-map-inner\"",
        "id=\"region-box\"",
        "id=\"region-display-label\"",
        "id=\"set-region-width\"",
        "id=\"set-region-height\"",
        "id=\"set-region-x\"",
        "id=\"set-region-y\"",
        "id=\"capture-region-menu\"",
        "id=\"region-align-menu\"",
        "id=\"region-display-menu\"",
        "id=\"clip-context-menu\"",
        "id=\"clip-menu-play\"",
        "id=\"clip-menu-open-cloud-page\"",
        "id=\"clip-menu-copy-cloud-link\"",
        "id=\"clip-menu-upload\"",
        "id=\"clip-menu-rename\"",
        "id=\"clip-menu-delete\"",
        "id=\"clip-title-display\"",
        "id=\"rename-clip\"",
        "id=\"clip-title-edit\"",
        "id=\"rename-input\"",
        "id=\"rename-save\"",
        "id=\"rename-cancel\"",
        "id=\"upload-dialog\"",
        "id=\"upload-title\"",
        "id=\"upload-description\"",
        "id=\"upload-visibility\"",
        "id=\"upload-audio-section\"",
        "id=\"upload-audio-list\"",
        "id=\"upload-confirm\"",
        "id=\"upload-cancel\"",
        "id=\"upload-dialog-status\"",
        "id=\"set-buffer\"",
        "id=\"set-encoder\"",
        "id=\"encoder-summary\"",
        "id=\"set-output-resolution\"",
        "id=\"output-resolution-summary\"",
        "id=\"set-replay\"",
        "id=\"replay-summary\"",
        "id=\"replay-scale\"",
        "id=\"set-bitrate\"",
        "id=\"quality-summary\"",
        "id=\"quality-scale\"",
        "id=\"set-fps\"",
        "id=\"fps-summary\"",
        "id=\"fps-scale\"",
        "id=\"set-media-dir\"",
        "id=\"choose-media-folder\"",
        "id=\"set-quota\"",
        "id=\"set-replay-disk-enabled\"",
        "id=\"replay-disk-fields\"",
        "id=\"set-replay-disk-dir\"",
        "id=\"choose-replay-cache-folder\"",
        "id=\"set-replay-disk-quota\"",
        "id=\"replay-disk-estimate\"",
        "id=\"set-replay-disk-ack\"",
        "data-tab=\"cloud\"",
        "data-section=\"cloud\"",
        "id=\"cloud-connect-fields\"",
        "id=\"cloud-host-url\"",
        "id=\"cloud-username\"",
        "id=\"cloud-password\"",
        "id=\"cloud-http-warning\"",
        "id=\"cloud-connect\"",
        "id=\"cloud-disconnect\"",
        "id=\"cloud-connect-status\"",
        "id=\"cloud-connection-status\"",
        "id=\"cloud-default-visibility\"",
        "id=\"cloud-delete-local-after-upload\"",
        "id=\"cloud-auto-upload-rules\"",
        "data-tab=\"games\"",
        "data-section=\"games\"",
        "id=\"set-games-auto-detect\"",
        "id=\"supported-games\"",
        "id=\"custom-games\"",
        "id=\"add-custom-game\"",
        "id=\"game-window-picker\"",
        "id=\"refresh-game-windows\"",
        "id=\"game-window-list\"",
        "id=\"game-detection-status\"",
        "id=\"set-hotkey\"",
        "id=\"settings-save\"",
        "id=\"settings-close\"",
    ] {
        assert!(
            html.contains(required),
            "review player is missing required control {required}"
        );
    }

    assert!(
        html.contains("value=\"display_region\""),
        "capture target must expose the display_region mode"
    );
    assert!(
        html.contains("Experimental")
            && html.contains("set-audio-split-output")
            && main_js().contains("split_output_by_process")
            && main_js().contains("split_output_by_process: false"),
        "capture settings must expose and persist the experimental audio-splitting toggle"
    );
    assert!(
        html.contains("Close to Tray")
            && html.contains("Minimize to Tray")
            && html.contains("Updates")
            && html.contains("value=\"stable\" disabled")
            && main_js().contains("close_to_tray")
            && main_js().contains("minimize_to_tray")
            && main_js().contains("update_channel")
            && main_js().contains("check_for_updates")
            && main_js().contains("install_update")
            && main_js().contains("function updateUpToDateStatus(update)")
            && main_js().contains("update.current_version")
            && main_js().contains("update.status || updateUpToDateStatus(update)")
            && main_js().contains("checkForUpdates({ manual: false })")
            && app_rs().contains("tauri_plugin_updater::Builder::new().build()")
            && main_js().contains("minimize_main_window"),
        "general settings must expose and persist tray close/minimize/preview/update behavior"
    );
    assert!(
        main_js().contains("requestWindowClose")
            && main_js().contains("confirmQuit")
            && main_js().contains("close_to_tray === false")
            && styles_css().contains("#quit-dialog"),
        "the window close button must confirm before quitting when Close to Tray is disabled"
    );
    assert!(
        !html.contains(">primary monitor<")
            && main_js().contains("renderCaptureTargetSelect")
            && main_js().contains("displayCaptureValue")
            && main_js().contains("display:")
            && html.contains(">SET REGION<")
            && main_js().find("displayCaptureValue").unwrap()
                < main_js().find("region.value = \"display_region\"").unwrap(),
        "capture target must list available displays before the display-region option"
    );
    assert!(
        !html.contains("value=\"window_title\"") && !html.contains("id=\"set-window\""),
        "manual window-title capture was replaced by custom game detection"
    );
    assert!(
        html.contains("data-replay-preset=\"30\"")
            && html.contains("data-replay-preset=\"60\"")
            && html.contains("data-replay-preset=\"120\""),
        "recording tab must expose quick save-length presets up to two minutes"
    );
    assert!(
        !html.contains("data-replay-preset=\"300\""),
        "save length must not expose presets beyond two minutes"
    );
    let replay_start = html
        .find("id=\"set-replay\"")
        .expect("replay control exists");
    let replay_tag_end = html[replay_start..]
        .find('>')
        .map(|offset| replay_start + offset)
        .expect("replay control tag closes");
    assert!(
        html[replay_start..=replay_tag_end].contains("max=\"120\""),
        "save length slider must stop at two minutes"
    );
    let fps_start = html.find("id=\"set-fps\"").expect("fps control exists");
    let fps_tag_end = html[fps_start..]
        .find('>')
        .map(|offset| fps_start + offset)
        .expect("fps control tag closes");
    assert!(
        html[fps_start..=fps_tag_end].contains("type=\"range\""),
        "smoothness must be a slider, not a dropdown"
    );
    assert!(
        html.contains("id=\"hotkey-status\""),
        "hotkeys page must expose recorder status text"
    );
    let hotkey_start = html.find("id=\"set-hotkey\"").expect("hotkey input exists");
    let hotkey_tag_end = html[hotkey_start..]
        .find('>')
        .map(|offset| hotkey_start + offset)
        .expect("hotkey input tag closes");
    assert!(
        html[hotkey_start..=hotkey_tag_end].contains("readonly"),
        "hotkey input must record shortcuts instead of accepting free text"
    );
    let media_dir_start = html
        .find("id=\"set-media-dir\"")
        .expect("media folder input exists");
    let media_dir_tag_end = html[media_dir_start..]
        .find('>')
        .map(|offset| media_dir_start + offset)
        .expect("media folder input tag closes");
    assert!(
        html[media_dir_start..=media_dir_tag_end].contains("readonly"),
        "media folder should be chosen with the native folder picker"
    );
    assert!(
        html.contains("Choose Folder"),
        "storage settings must expose a native-folder-picker action"
    );
    assert!(
        html.contains("Disk replay buffer (advanced)")
            && html.contains("Only turn this on if you know what you're doing")
            && html.contains("can add significant SSD wear")
            && html.contains(
                "I understand this continuously writes to disk and can shorten SSD life."
            ),
        "disk replay buffer settings must carry explicit advanced SSD-wear warnings"
    );
    assert!(
        html.contains(">Cloud<")
            && html.contains("This host uses HTTP. Your password will be sent without TLS.")
            && !html.contains("id=\"cloud-http-confirm\"")
            && main_js().contains("cloud_connect")
            && main_js().contains("cloud_disconnect")
            && main_js().contains("function cloudHostUsesInsecureHttp()")
            && main_js().contains("plain_http_confirmed: cloudHostUsesInsecureHttp()")
            && main_js().contains(
                "$(\"cloud-host-url\").addEventListener(\"input\", syncCloudHttpWarning)"
            )
            && main_js().contains(
                "$(\"cloud-host-url\").value = connected ? \"\" : cloud.host_url || \"\""
            )
            && main_js().contains(
                "$(\"cloud-username\").value = connected ? \"\" : cloud.connected_username || \"\""
            )
            && main_js().contains("$(\"cloud-connect-fields\").hidden = connected")
            && main_js().contains("$(\"cloud-connect\").hidden = connected")
            && main_js().contains("$(\"cloud-disconnect\").hidden = !connected")
            && main_js().contains("upload_clip_to_cloud")
            && main_js().contains("function openUploadDialog(clip)")
            && main_js().contains("title: request.title || clipUploadDefaultTitle(clip)")
            && main_js().contains("visibility: request.visibility || cloudSettings().default_visibility || \"private\"")
            && html.contains("id=\"upload-dialog\"")
            && html.contains("id=\"upload-title\"")
            && html.contains("id=\"upload-description\"")
            && html.contains("maxlength=\"5000\"")
            && !html.contains("Not supported by Clipline Cloud yet")
            && !main_js().contains("Descriptions are not supported by Clipline Cloud yet.")
            && main_js().contains("description: request.description || null")
            && html.contains("id=\"upload-visibility\"")
            && html.contains("id=\"upload-audio-section\"")
            && html.contains("id=\"upload-audio-list\"")
            && main_js().contains("function clipAudioTracks(clip = currentClip)")
            && main_js().contains("function renderAudioTrackPanel()")
            && main_js().contains("function applySelectedAudioTracksToPlayback({ forceResume = false } = {})")
            && main_js().contains("preview_clip_audio_tracks")
            && main_js().contains("function renderUploadAudioTracks(clip = uploadDialogClip)")
            && main_js().contains("audioTrackIds: request.audioTrackIds || null")
            && !main_js().contains("video.audioTracks")
            && !main_js().contains("applyNativeAudioTrackSelection")
            && main_js().contains("audio-track-label")
            && styles_css().contains(".audio-track-panel")
            && styles_css().contains(".audio-track-row")
            && styles_css().contains(".audio-track-label")
            && styles_css().contains(".upload-audio-section[hidden] { display: none; }")
            && main_js().contains("listen(\"cloud-upload-progress\"")
            && main_js().contains("navigator.clipboard.writeText(record.remote_url)")
            && main_js().contains("syncUploadClipButton();")
            && main_js().contains("Connect Clipline Cloud before uploading.")
            && main_js().contains("function clipCloudVisibility(record)")
            && main_js().contains("CLOUD_VISIBILITY_ICONS")
            && main_js().contains("clip-cloud-visibility")
            && !main_js().contains(" · cloud:")
            && app_rs().contains("crate::cloud::cloud_connect")
            && app_rs().contains("crate::cloud::upload_clip_to_cloud")
            && app_rs().contains("crate::cloud::sync_cloud_clip_status")
            && app_rs().contains("crate::library::preview_clip_audio_tracks")
            && main_js().contains("sync_cloud_clip_status")
            && styles_css().contains(".cloud-connect-grid")
            && styles_css().contains(".cloud-connect-fields")
            && styles_css().contains(".cloud-connect-fields[hidden] { display: none; }")
            && styles_css().contains(".cloud-http-warning")
            && styles_css().contains(".cloud-http-warning[hidden] { display: none; }")
            && styles_css().contains(".clip-cloud-visibility.public")
            && styles_css().contains(".clip-cloud-visibility.unlisted")
            && styles_css().contains(".clip-cloud-visibility.private")
            && styles_css().contains(".clip .clip-title")
            && styles_css().contains(".review-head .clip-title")
            && styles_css().contains("#upload-dialog")
            && html.contains("id=\"upload-clip\"")
            && styles_css().contains(".review-actions .icon-button.uploaded"),
        "cloud settings, upload controls, and per-clip visibility icons must stay wired"
    );
    assert!(
        html.contains(">Games<") && html.contains("Add Custom Game"),
        "settings must expose the Games tab and custom game action"
    );
    assert!(
        main_js().contains("gameRecordingModeControl")
            && main_js().contains("custom-game-recording-mode")
            && main_js().contains("recording_mode")
            && main_js().contains("replays_only")
            && main_js().contains("full_session")
            && styles_css().contains(".custom-game-mode"),
        "custom games must expose and persist per-game recording mode choices"
    );
    assert!(
        main_js().contains("await invoke(\"list_game_plugins\")")
            && main_js().contains("renderGamePlugins")
            && main_js().contains("gamePluginSettings")
            && main_js().contains("plugin.presentation")
            && main_js().contains("games.plugins")
            && main_js().contains("dataset.gamePluginEnabled")
            && main_js().contains("game-plugin-mode-")
            && main_js().contains("normalizeGamePluginId")
            && main_js().contains("Takes priority over matching custom games.")
            && main_js().contains("check_game_plugin_package")
            && main_js().contains("update_game_plugin_package")
            && main_js().contains("reinstall_game_plugin_package")
            && main_js().contains("reset_game_plugin_to_seed")
            && main_js().contains("plugin.latest_version")
            && main_js().contains("plugin.latest_source_label")
            && main_js().contains("dataset.gamePluginAction")
            && styles_css().contains(".game-profile-mode"),
        "supported games must render from backend game plugins, including first-party package actions, not hardcoded rows"
    );
    assert!(
        main_js().contains("function pluginPresentationForClip(clip)")
            && main_js().contains("function clipGameSummary(clip)")
            && main_js().contains("function renderGameEventRail")
            && main_js().contains("gameEventRailItem")
            && main_js().contains("game-event-duel")
            && main_js().contains("game-event-actor-event")
            && main_js().contains("game-event-objective-icon")
            && !main_js().contains("game-event-objective-label")
            && main_js().contains("game-event-portrait")
            && main_js().contains("game-event-kind-icon")
            && main_js().contains("function syncGameEventRail")
            && main_js().contains("function setGameEventRailCollapsed")
            && main_js().contains("function selectGameEvent")
            && main_js().contains("function clearGameEventSelection")
            && main_js().contains("gameEventActiveIndex")
            && main_js().contains("keepGameEventSelection")
            && main_js().contains("gameEventRailCollapsed")
            && main_js().contains("event-rail-collapsed")
            && main_js().contains("game-event-rail-toggle\").addEventListener(\"click\"")
            && main_js().contains("setGameEventRailCollapsed(!gameEventRailCollapsed)")
            && main_js().contains("syncGameEventRail(video.currentTime || 0, { force: true })")
            && main_js().contains("function renderGameMetadataPanel")
            && main_js().contains("presentation.event_rail")
            && main_js().contains("presentation.metadata_panel")
            && main_js().contains("playerSummaryFields")
            && main_js().contains("data_dragon: presentation && presentation.data_dragon")
            && main_js().contains("data-game-event-index")
            && main_js().contains("gallery.summary === \"player_summary_kda\"")
            && main_js().contains("const gameMeta = clipGameSummary(c)")
            && main_js().contains(
                "const gameSessionTitle = isPluginSummaryFullSessionTitle(c, kind, gameMeta)"
            )
            && main_js().contains("? gameMeta")
            && main_js().contains("function clipLibraryTitle(clip, fallbackTitle)")
            && main_js().contains("if (usesFallbackTitleForPluginClip(clip)) return fallbackTitle")
            && main_js().contains("const clipName = clip && String(clip.name || \"\").trim()")
            && main_js().contains("return clipName || fallbackTitle")
            && main_js().contains("detail.className = \"game-meta\"")
            && main_js().contains("if (gameMeta && !gameSessionTitle)")
            && main_js().contains("const infoParts = []")
            && main_js().contains(
                "if (Number.isFinite(c.duration_s)) infoParts.push(fmtDur(c.duration_s))"
            )
            && main_js().contains("if (!gameMeta && digest) infoParts.push(digest)")
            && !main_js().contains("LEAGUE_OF_LEGENDS_ID")
            && !main_js().contains("isLeagueClip")
            && !main_js().contains("function renderGamePanel")
            && index_html().contains("aria-controls=\"game-event-list\"")
            && styles_css().contains(".review-body.has-event-rail.event-rail-collapsed")
            && styles_css().contains(".game-event-rail-tab")
            && styles_css().contains(".game-event-row-friendly")
            && styles_css().contains(".game-event-row-enemy")
            && styles_css().contains(".game-event-rail ol button.game-event-row-friendly")
            && styles_css().contains(".game-event-rail ol button.game-event-row-enemy")
            && styles_css().contains(".game-event-rail .game-event-objective-icon")
            && styles_css().contains("grid-column: 4;")
            && styles_css().contains("grid-template-columns: 38px minmax(46px, 1fr) 34px minmax(46px, 1fr);")
            && styles_css().contains(".game-event-rail ol button.game-event-duel .game-event-kind-icon")
            && styles_css().contains("width: 34px;\n  height: 34px;\n  overflow: visible;")
            && !styles_css().contains("align-self: start;\n  margin-top: 7px;")
            && styles_css().contains(".game-event-rail ol button.marker-kill .game-event-kind-icon img")
            && styles_css().contains(".game-event-rail ol button.marker-death .game-event-kind-icon img")
            && styles_css().contains("width: 36px;\n  height: 36px;")
            && styles_css().contains("border: 0;\n  border-radius: 0;\n  background: transparent;")
            && styles_css().contains("filter:\n    drop-shadow(1px 0 0 rgba(2, 6, 23, 0.9))")
            && styles_css().contains(".game-event-name")
            && styles_css().contains(".game-event-rail:hover .game-event-rail-tab")
            && styles_css().contains("--game-event-rail-pad: 10px;")
            && styles_css().contains("left: 0;")
            && !styles_css().contains("left: var(--game-event-rail-pad);")
            && styles_css().contains("top: 50%;")
            && styles_css().contains("transform: translate(-100%, -50%)")
            && styles_css().contains(".game-event-rail::before")
            && styles_css().contains("left: -34px;")
            && styles_css().contains("height: 72px;")
            && styles_css().contains("pointer-events: auto;")
            && styles_css().contains("transition: opacity 120ms ease, background 120ms ease;")
            && styles_css().contains(".game-event-rail-tab:active")
            && !styles_css().contains("translate(calc(-100% + 8px), -50%)")
            && !styles_css().contains("transition: opacity 120ms ease, transform")
            && styles_css().contains("padding: var(--game-event-rail-pad);")
            && styles_css().contains(".game-event-rail.is-collapsed")
            && index_html().contains(
                "<svg class=\"i-collapse\" viewBox=\"0 0 24 24\"><path d=\"M8.6 16.6 10 18l6-6-6-6-1.4 1.4 4.6 4.6-4.6 4.6z\"/></svg>"
            )
            && index_html().contains(
                "<svg class=\"i-expand\" viewBox=\"0 0 24 24\"><path d=\"M15.4 7.4 14 6l-6 6 6 6 1.4-1.4L10.8 12l4.6-4.6z\"/></svg>"
            )
            && styles_css().contains(".clip .game-meta"),
        "plugin-driven game rows must keep League title/KDA behavior and render right-side events plus declarative bottom metadata"
    );
    assert!(
        !index_html().contains("game-profile planned"),
        "planned game cards should not sit in static HTML where renderGamePlugins wipes them"
    );

    // Settings is a page in the main pane now, not a sidebar fold.
    assert!(
        !html.contains("settings-fold"),
        "the sidebar settings fold was replaced by #settings-page"
    );
    // Reversed (2026-06-12, PR #5): the footer now carries an explicit Close
    // button after Save, replacing the earlier "close only from the rail" rule.
    let settings_save = html
        .find("id=\"settings-save\"")
        .expect("settings save button");
    let settings_close = html
        .find("id=\"settings-close\"")
        .expect("settings close button");
    assert!(
        settings_save < settings_close,
        "Close must come after Save in the footer markup"
    );

    // Removed on purpose (2026-06-12): clicking the active library row again
    // closes the clip; the new copy affordance must not revive the old path-only id.
    for gone in [
        "id=\"copy-path\"",
        "id=\"close-review\"",
        "id=\"focus-toggle\"",
    ] {
        assert!(
            !html.contains(gone),
            "{gone} was removed from the header — do not reintroduce it"
        );
    }
    let upload_clip = html.find("id=\"upload-clip\"").expect("upload clip button");
    let open_folder = html.find("id=\"open-folder\"").expect("open folder button");
    let copy_clip = html.find("id=\"copy-clip\"").expect("copy clip button");
    let delete_clip = html.find("id=\"delete-clip\"").expect("delete clip button");
    assert!(
        upload_clip < open_folder,
        "upload button must sit immediately left of Open Folder in the review header"
    );
    assert!(
        open_folder < copy_clip && copy_clip < delete_clip,
        "copy clip must sit beside Open Folder before the destructive action"
    );

    // Conventional ordering: transport glued to the stage, timeline below it.
    let transport = html.find("id=\"play-toggle\"").expect("play toggle");
    let timeline = html.find("id=\"timeline\"").expect("timeline");
    assert!(
        transport < timeline,
        "transport row must precede the timeline in the deck"
    );
    assert!(
        styles_css().contains(".stage-frame")
            && styles_css().contains("object-fit: contain")
            && main_js().contains("updateStageFrame"),
        "the review stage must size an aspect-locked frame around the video"
    );
    // Icon buttons carry SVG icons; text labels are a regression.
    for id in [
        "id=\"play-toggle\"",
        "id=\"seek-back\"",
        "id=\"seek-forward\"",
        "id=\"prev-marker\"",
        "id=\"next-marker\"",
        "id=\"mute-toggle\"",
        "id=\"upload-clip\"",
        "id=\"open-folder\"",
        "id=\"copy-clip\"",
        "id=\"rail-save\"",
        "id=\"rail-settings\"",
        "id=\"delete-clip\"",
        "id=\"export-clip\"",
        "id=\"zoom-out\"",
        "id=\"zoom-fit\"",
        "id=\"zoom-in\"",
        "id=\"snap-toggle\"",
    ] {
        let start = html.find(id).expect("transport button exists");
        let body_end = html[start..]
            .find("</button>")
            .map(|o| start + o)
            .expect("button closes");
        assert!(
            html[start..body_end].contains("<svg"),
            "{id} must render an SVG icon, not a text label"
        );
    }
}

#[test]
fn rail_shows_save_hotkey() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    assert!(
        html.contains("id=\"rail-hotkey\""),
        "sidebar rail must expose the current save hotkey"
    );
    assert!(
        js.contains("function updateHotkeyLabels(")
            && js.contains("rail-hotkey")
            && js.contains("Save Replay ("),
        "main.js must keep rail and button hotkey labels in sync"
    );
    assert!(
        css.contains(".rail-hotkey"),
        "rail hotkey needs stable compact styling"
    );
}

#[test]
fn rail_shows_connected_cloud_identity() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    for required in [
        "<button id=\"rail-profile\"",
        "id=\"rail-profile-avatar\"",
        "id=\"rail-profile-name\"",
    ] {
        assert!(
            html.contains(required),
            "rail profile markup must include `{required}`"
        );
    }
    for required in [
        "function syncRailProfile",
        "function refreshRailProfileIdentity",
        "function loadRailProfileAvatar",
        "function openRailProfile",
        "invoke(\"cloud_user_profile\")",
        "invoke(\"cloud_user_avatar\")",
        "invoke(\"open_cloud_user_profile\")",
        "connected_display_name",
        "railProfileAvatarKey",
        "$(\"rail-profile-name\")",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire cloud rail profile behavior through `{required}`"
        );
    }
    for required in [
        ".rail-profile",
        ".rail-profile[hidden]",
        ".rail-profile-avatar",
        ".rail-profile-name",
    ] {
        assert!(
            css.contains(required),
            "rail cloud identity needs stable compact styling for `{required}`"
        );
    }
    assert!(
        app_rs().contains("crate::cloud::cloud_user_avatar"),
        "native command registry must expose cloud_user_avatar for the rail profile"
    );
    assert!(
        app_rs().contains("crate::cloud::cloud_user_profile"),
        "native command registry must expose cloud_user_profile for display-name refresh"
    );
    assert!(
        app_rs().contains("crate::cloud::open_cloud_user_profile"),
        "native command registry must expose open_cloud_user_profile for the rail profile button"
    );
}

#[test]
fn default_audio_preview_is_gated_and_degrades_to_source_on_failure() {
    let js = main_js();
    let open_clip_start = js.find("function openClip(clip)").unwrap();
    let close_review_start = js.find("function closeReview()").unwrap();
    let open_clip = &js[open_clip_start..close_review_start];

    assert!(
        !open_clip.contains("applySelectedAudioTracksToPlayback()"),
        "opening a clip must not unconditionally remux or mix a full-session audio preview"
    );
    assert!(
        open_clip.contains("applyDefaultAudioSelectionIfNeeded({ shouldResume: true })"),
        "opening a clip should apply default audio only when source playback would not match it"
    );
    assert!(
        open_clip.contains("syncCloudClipStatus(clip);"),
        "opening a clip should refresh its cloud record in the background"
    );
    let source_play = open_clip
        .find("video.play().catch(() => syncPlayState());")
        .expect("openClip should still start direct source playback when no preview is needed");
    let default_preview = open_clip
        .find("applyDefaultAudioSelectionIfNeeded({ shouldResume: true })")
        .expect("openClip should request a resumed default preview when needed");
    assert!(
        default_preview < source_play,
        "preview-needed clips must not audibly play the unmixed source before the preview source is ready"
    );
    assert!(
        js.contains("function applyDefaultAudioSelectionIfNeeded({ shouldResume = false } = {})")
            && js.contains("if (audioPreviewUnavailable && selected.length > 1) return false;")
            && js.contains("PlayerCore.selectionNeedsPreview")
            && js.contains("applySelectedAudioTracksToPlayback({ forceResume: shouldResume });"),
        "default audio application must be gated by PlayerCore.selectionNeedsPreview"
    );
    assert!(
        js.contains("currentReviewAudioKey === audioSelectionKey(clip, selected)"),
        "reapplying the same selected audio tracks should not remux the current review source"
    );
    assert!(
        js.contains("function applyCloudClipSyncResult(")
            && js.contains("removeCloudUploadRecordForPath(result.path)")
            && js.contains("upsertCloudUploadRecord(result.record)"),
        "cloud sync results must update or remove the local cloud record cache"
    );
    assert!(
        js.contains("setDeckStatus(\"audio mix unavailable; playing source\", { transient: true });")
            && js.contains("audioPreviewUnavailable = true;")
            && js.contains("if (currentReviewMediaPath !== clip.path) {")
            && js.contains("setReviewVideoSource(clip.path, { resumeTime, shouldResume, rate, trimRange });"),
        "preview generation failure should fall back to source playback without a persistent error banner"
    );
    assert!(
        js.contains("function cloudUploadRecordForPath(path)")
            && js.contains("applyCloudClipSyncResult(result, {")
            && js.contains("expectedRecord, expectedLocalClipId, expectedUpdatedAtUnix"),
        "cloud open-sync must capture the record identity it started from"
    );
    assert!(
        js.contains("if (expectedRecord && current !== expectedRecord) return false;")
            && js.contains("current.local_clip_id !== expectedLocalClipId")
            && js.contains(
                "Number(current.updated_at_unix || 0) > Number(expectedUpdatedAtUnix || 0)"
            ),
        "cloud open-sync must ignore stale results once a newer upload record exists"
    );
}

#[test]
fn open_clip_clears_previous_playback_loop_and_pending_seek() {
    let js = main_js();
    let open_clip_start = js.find("function openClip(clip)").unwrap();
    let close_review_start = js.find("function closeReview()").unwrap();
    let open_clip = &js[open_clip_start..close_review_start];
    let cancel = open_clip
        .find("cancelAnimationFrame(rafId);")
        .expect("openClip cancels the previous playhead RAF");
    let clear_seek = open_clip
        .find("pendingSeek = null;")
        .expect("openClip clears pending seek from previous clip");
    let assign_clip = open_clip
        .find("currentClip = clip;")
        .expect("openClip assigns current clip");

    assert!(
        cancel < assign_clip,
        "RAF must be canceled before switching clips"
    );
    assert!(
        clear_seek < assign_clip,
        "pending seek must be cleared before switching clips"
    );
}

#[test]
fn close_to_tray_suspends_review_playback() {
    let app = app_rs();
    let js = main_js();
    let tray_start = app
        .find("fn send_main_window_to_tray")
        .expect("send-to-tray helper");
    let tray_end = app[tray_start..]
        .find("fn quit_app")
        .map(|offset| tray_start + offset)
        .expect("quit helper follows tray helper");
    let tray_helper = &app[tray_start..tray_end];
    let suspend_start = js
        .find("function suspendReviewPlayback()")
        .expect("frontend suspend helper");
    let close_review_start = js.find("function closeReview()").unwrap();
    let suspend_helper = &js[suspend_start..close_review_start];

    assert!(
        tray_helper.contains("emit(\"suspend-review-playback\""),
        "native close-to-tray must tell the WebView to stop clip playback before hiding it"
    );
    assert!(
        js.contains("listen(\"suspend-review-playback\"") && js.contains("suspendReviewPlayback"),
        "frontend must listen for the native close-to-tray suspend event"
    );
    assert!(
        suspend_helper.contains("audioPreviewSeq += 1;")
            && suspend_helper.contains("cancelAnimationFrame(rafId);")
            && suspend_helper.contains("video.pause();")
            && suspend_helper.contains("video.removeAttribute(\"src\");")
            && suspend_helper.contains("video.load();"),
        "suspending playback must cancel preview work, stop the RAF loop, and unload the video"
    );
    assert!(
        suspend_helper.contains("currentClip = null;")
            && suspend_helper.contains("currentReviewMediaPath = null;")
            && suspend_helper.contains("updateViews();"),
        "suspending playback must also leave the editor state so reopening from tray cannot show a src-less current clip"
    );
}

#[test]
fn initial_settings_tab_state_matches_visible_section() {
    let html = index_html();
    let tabs_start = html.find("id=\"settings-tabs\"").expect("settings tabs");
    let tabs_end = html[tabs_start..]
        .find("</nav>")
        .map(|offset| tabs_start + offset)
        .expect("settings tabs close");
    let tabs = &html[tabs_start..tabs_end];

    let mut active_tabs = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = tabs[cursor..].find("<button") {
        let start = cursor + offset;
        let end = tabs[start..]
            .find('>')
            .map(|tag_end| start + tag_end)
            .expect("tab button closes");
        let tag = &tabs[start..=end];
        if tag_attr(tag, "class")
            .is_some_and(|class| class.split_whitespace().any(|c| c == "active"))
        {
            active_tabs.push(tag_attr(tag, "data-tab").expect("active tab has data-tab"));
        }
        cursor = end + 1;
    }
    assert_eq!(
        active_tabs.len(),
        1,
        "settings must have exactly one active initial tab"
    );
    let active_tab = active_tabs[0];

    let mut visible_sections = Vec::new();
    cursor = 0;
    while let Some(offset) = html[cursor..].find("<div class=\"settings-section\"") {
        let start = cursor + offset;
        let end = html[start..]
            .find('>')
            .map(|tag_end| start + tag_end)
            .expect("settings section opens");
        let tag = &html[start..=end];
        let section = tag_attr(tag, "data-section").expect("settings section has data-section");
        let hidden = tag
            .split_whitespace()
            .any(|part| part == "hidden" || part == "hidden>");
        if hidden {
            assert_ne!(
                section, active_tab,
                "the initially active settings section must not be hidden"
            );
        } else {
            visible_sections.push(section);
        }
        cursor = end + 1;
    }
    assert_eq!(
        visible_sections,
        vec![active_tab],
        "only the active settings tab's section should be visible before first interaction"
    );
}

#[test]
fn settings_tabs_preserve_unsaved_draft_until_save() {
    let js = main_js();
    let tab_handler_start = js
        .find("document.querySelectorAll(\"#settings-tabs .tab\")")
        .expect("settings tab handler");
    let timeline_start = js[tab_handler_start..]
        .find("$(\"timeline\")")
        .map(|offset| tab_handler_start + offset)
        .expect("timeline handler follows settings tabs");
    let tab_handler = &js[tab_handler_start..timeline_start];
    let save_handler_start = js
        .find("$(\"settings-save\").addEventListener")
        .expect("settings save handler");
    let video_start = js[save_handler_start..]
        .find("video.addEventListener")
        .map(|offset| save_handler_start + offset)
        .expect("video handlers follow settings save");
    let save_handler = &js[save_handler_start..video_start];
    let sync_start = js
        .find("function syncSettingsDraftFromForm()")
        .expect("settings draft sync helper");
    let fill_start = js[sync_start..]
        .find("function fillSettings")
        .map(|offset| sync_start + offset)
        .expect("fillSettings follows settings draft sync helper");
    let sync_helper = &js[sync_start..fill_start];

    assert!(
        js.contains("let settingsDraft = null;")
            && js.contains("function settingsFormSource()")
            && js.contains("function syncSettingsDraftFromForm()"),
        "settings must keep an explicit unsaved draft while the settings page is open"
    );
    assert!(
        tab_handler.contains("syncSettingsDraftFromForm();"),
        "switching tabs must snapshot edits before the current section is hidden"
    );
    assert!(
        save_handler.contains("settings: syncSettingsDraftFromForm()"),
        "Save Settings must submit the accumulated draft, not only the visible tab state"
    );
    assert!(
        sync_helper.contains("settingsDraft = readSettings();")
            && !sync_helper.contains("return settingsDraft || {};"),
        "Save Settings must fall back to a full form snapshot, not {{}}, when settings are not loaded yet"
    );
    assert!(
        js.contains("settings-page\").addEventListener(\"input\", () => syncSettingsDraftFromForm())")
            && js.contains("settings-page\").addEventListener(\"change\", () => syncSettingsDraftFromForm())"),
        "settings form edits must continuously refresh the draft before async tab renderers repaint controls"
    );
    assert!(
        js.contains("const audio = settingsFormSource().audio || defaultAudioSettings();")
            && js.contains("const selected = settingsFormSource().video_encoder || \"auto\";")
            && js.contains("function captureSettingsValue(settings = settingsFormSource())"),
        "async settings renderers must use the draft as their source while settings are being edited"
    );

    for renderer in [
        "function renderCaptureTargetSelect()",
        "function renderAudioDeviceSelects()",
        "function renderVideoEncoderSelect()",
    ] {
        let start = js.find(renderer).expect("settings option renderer");
        let end = js[start + renderer.len()..]
            .find("\nfunction ")
            .map(|offset| start + renderer.len() + offset)
            .expect("next function follows renderer");
        let body = &js[start..end];
        assert!(
            !body.contains("syncSettingsDraftFromForm()"),
            "settings option renderers must not re-read stale DOM state while fillSettings is repainting"
        );
    }
}

#[test]
fn timeline_navigator_and_zoom_controls_are_wired() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    // The whole-clip navigator sits between the ruler and the export row.
    let ruler = html.find("id=\"ruler\"").expect("ruler");
    let overview = html.find("id=\"overview\"").expect("overview");
    let export_row = html.find("class=\"export-row\"").expect("export row");
    assert!(
        ruler < overview && overview < export_row,
        "the navigator minimap must sit between the ruler and the export row"
    );

    // Central view setter + paint/rebuild split keep the navigator in sync, and
    // every view change routes through the pure helpers.
    for required in [
        "function applyView",
        "function paintOverview",
        "function renderOverviewMarkers",
        "function maybeFollow",
        "onOverviewPointerDown",
        "function zoomAtPlayhead",
        "function zoomToSelection",
        "zoomView(",
        "panView(",
        "setViewEdge(",
        "followView(",
        "snapTime(",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire the timeline through {required}"
        );
    }

    // Navigator window, markers, and snap feedback need styles.
    assert!(
        css.contains("#overview-window") && css.contains(".ov-marker") && css.contains(".snapped"),
        "navigator window, marker ticks, and snap feedback must be styled"
    );
    assert!(
        css.contains(".marker-death .glyph.img")
            && css.contains("-webkit-mask: var(--marker-img) center / 190% no-repeat"),
        "death marker art has extra transparent padding and must be scaled to match kill markers"
    );
    assert!(
        css.contains(".marker .glyph.img")
            && css.contains("mask: var(--marker-img) center / contain no-repeat;\n  filter:\n    drop-shadow(1px 0 0 rgba(2, 6, 23, 0.9))"),
        "timeline marker image glyphs must use the same black alpha-outline as event rail icons"
    );
}

#[test]
fn no_native_browser_dialogs() {
    let js = main_js();
    let css = styles_css();
    // window.confirm/alert render browser chrome ("tauri.localhost says") —
    // use the in-app #confirm-dialog instead.
    for banned in ["confirm(", "alert("] {
        assert!(
            !js.contains(banned),
            "main.js must not call native {banned}…) — use the in-app dialog"
        );
    }

    assert!(
        js.contains("document.addEventListener(\"contextmenu\", (ev) => {")
            && js.contains("ev.preventDefault();")
            && js.contains("showClipContextMenu(ev, c)")
            && js.contains("showCloudClipContextMenu(ev, entry)")
            && js.contains("$(\"clip-menu-play\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-open-cloud-page\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-copy-cloud-link\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-upload\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-rename\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-delete\").addEventListener(\"click\"")
            && js.contains("function beginClipRename")
            && js.contains("await invoke(\"rename_clip\"")
            && app_rs().contains("crate::library::rename_clip")
            && css.contains(".clip-title-edit")
            && css.contains(".context-menu button[hidden]")
            && css.contains(".context-menu button.danger-text"),
        "native context menus must be suppressed and library rows must expose an app-owned clip menu"
    );
}

#[test]
fn controls_have_custom_range_and_scrollbar_skin() {
    let css = styles_css();
    let js = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/main.js")).unwrap();

    assert!(
        css.contains("::-webkit-slider-thumb") && css.contains("::-moz-range-thumb"),
        "range inputs should use Clipline slider styling instead of native defaults"
    );
    assert!(
        css.contains("::-webkit-scrollbar-thumb") && css.contains("scrollbar-color"),
        "scrollable areas should use the app scrollbar styling"
    );
    assert!(
        css.contains("--range-progress") && js.contains("syncRangeProgress"),
        "slider fill must stay synced to the current value"
    );
    assert!(
        css.contains("background-position: right 12px center")
            && css.contains("-webkit-appearance: none"),
        "select arrows should use the app inset instead of the native edge-hugging arrow"
    );
}

#[test]
fn shell_shows_live_memory_usage() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    assert!(
        html.contains("id=\"memory-usage\"") && html.contains("Using -- RAM"),
        "sidebar chrome must include the RAM indicator placeholder"
    );
    assert!(
        js.contains("memory_status") && js.contains("setInterval(refreshMemoryUsage, 2000)"),
        "memory indicator must poll the backend command on a short interval"
    );
    assert!(
        css.contains(".memory-usage") && css.contains("font-variant-numeric: tabular-nums"),
        "memory usage should have stable numeric styling in the top-left chrome"
    );
}

#[test]
fn gallery_header_shows_library_storage_usage() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    assert!(
        html.contains("id=\"gallery-count\"") && html.contains("id=\"gallery-storage-used\""),
        "gallery header should include storage usage beside the clip count"
    );
    assert!(
        js.contains("$(\"gallery-storage-used\").textContent")
            && js.contains("const quotaGb = s.quota_bytes == null")
            && js.contains("fmtLibraryStorageUsage(s.total_bytes, quotaGb)"),
        "refreshStorage should render total library bytes and configured quota from storage_status into the gallery header"
    );
    assert!(
        css.contains(".gallery-storage-used"),
        "storage usage should have gallery header metadata styling"
    );
}

#[test]
fn library_has_cloud_source_tab() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();

    for required in [
        "id=\"gallery-source-tabs\"",
        "data-gallery-source=\"local\"",
        "data-gallery-source=\"cloud\"",
        "id=\"cloud-gallery-grid\"",
    ] {
        assert!(
            html.contains(required),
            "library markup must include cloud source tab contract `{required}`"
        );
    }
    for required in [
        "let gallerySource = \"local\"",
        "function renderCloudClips()",
        "function cloudLocalClipForEntry(entry)",
        "function openCloudEntryInApp(entry)",
        "function showCloudClipContextMenu(ev, entry)",
        "function observeCloudThumbnail(entry, thumb)",
        "function loadCloudThumbnail(entry, thumb)",
        "const cloudThumbnailInflight = new Map()",
        "posterQueue.set(thumb, { type: \"cloud-thumbnail\", entry })",
        "let cloudClipsCache = []",
        "function loadCloudClips",
        "if (gallerySource === \"cloud\") renderCloudClips();",
        "if (cloudClipsError && !force) return;",
        "error.className = \"gallery-empty cloud-error\"",
        "function isCloudOnlyReviewClip(clip = currentClip)",
        "function syncReviewLocalActions()",
        "invoke(\"list_cloud_clips\")",
        "invoke(\"cache_cloud_clip_media\"",
        "invoke(\"cloud_clip_thumbnail\"",
        "invoke(\"open_cloud_clip_url\"",
        "PlayerCore.cloudLibraryEntries",
        "localClip ? clipCard(localClip) : cloudClipCard(entry)",
        "showCloudClipContextMenu(ev, entry)",
        "$(\"cloud-gallery-grid\")",
        "querySelectorAll(\"#gallery-source-tabs .source-tab\")",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire cloud library behavior through `{required}`"
        );
    }
    assert!(
        !js.contains("actions.className = \"card-actions\""),
        "cloud-only cards should not render inline Play/Open/Copy buttons"
    );
    for required in [
        "const POSTER_UNAVAILABLE = Symbol(\"poster unavailable\")",
        "function markPosterUnavailable(path)",
        ".card-markers",
        "img.addEventListener(\"error\", () => {",
        "img.remove();",
        "if (onError) onError();",
        "posterCache.set(path, POSTER_UNAVAILABLE);",
        "if (cachedPoster === POSTER_UNAVAILABLE) {",
        ".catch(() => markPosterUnavailable(path));",
        "loadCardPoster(path, thumb)",
        "observePoster(c.path, thumb)",
        "insertThumbMedia(thumb, makePosterImg(cached))",
        "insertThumbMedia(thumb, makePosterImg(url))",
    ] {
        assert!(
            js.contains(required),
            "local clip thumbnails must safely cache poster failures and preserve overlays through `{required}`"
        );
    }
    for forbidden in [
        "function makePosterFallbackVideo(",
        "function showPosterFallback(",
        "video.className = \"card-thumb-img\"",
        "img.addEventListener(\"error\", () => onError && onError())",
        "thumb.appendChild(makePosterImg(cached))",
    ] {
        assert!(
            !js.contains(forbidden),
            "thumbnail fallbacks must not keep source media open or bypass overlay-safe insertion via `{forbidden}`"
        );
    }
    for required in [
        ".gallery-source-tabs",
        ".source-tab.active",
        ".cloud-gallery-grid",
        ".cloud-card",
        ".cloud-card-placeholder > svg",
        ".gallery-empty.cloud-error",
    ] {
        assert!(
            css.contains(required),
            "cloud library tab should have stable styling for `{required}`"
        );
    }
    assert!(
        app_rs().contains("crate::cloud::list_cloud_clips"),
        "native command registry must expose list_cloud_clips for the Cloud library tab"
    );
    assert!(
        app_rs().contains("crate::cloud::open_cloud_clip_url"),
        "native command registry must expose open_cloud_clip_url for Cloud card links"
    );
}

#[test]
fn games_ui_wires_detection_commands() {
    let js = main_js();

    for required in [
        "await invoke(\"list_game_plugins\")",
        "await invoke(\"list_game_windows\")",
        "listen(\"game-detection\"",
        "renderGamePlugins",
        "renderCustomGames",
        "refreshGameWindows",
        "$(\"add-custom-game\").addEventListener(\"click\", showGameWindowPicker)",
        "$(\"refresh-game-windows\").addEventListener(\"click\", refreshGameWindows)",
        "$(\"cancel-game-picker\").addEventListener(\"click\", hideGameWindowPicker)",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire the custom game workflow through {required}"
        );
    }
}

#[test]
fn deck_status_success_toasts_auto_clear() {
    let js = main_js();

    assert!(
        js.contains("const DECK_STATUS_TOAST_MS")
            && js.contains("let deckStatusToastTimer")
            && js.contains("function setDeckStatus(message, { transient = false } = {})"),
        "deck status messages should flow through a helper that can schedule transient toasts"
    );
    assert!(
        js.contains("window.setTimeout(() => {")
            && js.contains("if ($(\"deck-status\").textContent === message)"),
        "transient deck status toasts should clear themselves without erasing newer messages"
    );

    for required in [
        "setDeckStatus(audioSelectionLabel(clip), { transient: true })",
        "setDeckStatus(\"clip renamed\", { transient: true })",
        "setDeckStatus(`exported ${exported.name} · keyframe-aligned ${fmtTenths(exported.aligned_start_s)} – ${fmtTenths(exported.aligned_end_s)}`, { transient: true })",
        "setDeckStatus(\"clip copied to clipboard\", { transient: true })",
        "setDeckStatus(\"cloud link copied\", { transient: true })",
        "setDeckStatus(\"cloud upload ready\", { transient: true })",
    ] {
        assert!(
            js.contains(required),
            "success toast should auto-clear via `{required}`"
        );
    }

    for required in [
        "setDeckStatus(\"switching audio tracks...\")",
        "setDeckStatus(\"renaming clip...\")",
        "setDeckStatus(\"exporting…\")",
        "setDeckStatus(\"uploading to cloud...\")",
        "setDeckStatus(\"cloud upload processing\")",
    ] {
        assert!(
            js.contains(required),
            "progress status should stay explicit via `{required}`"
        );
    }
}

#[test]
fn clipboard_copy_sends_selected_audio_tracks() {
    let js = main_js();
    let app = app_rs();
    let library = library_rs();

    assert!(
        library.contains("pub struct CopyClipToClipboardRequest")
            && library.contains("request: CopyClipToClipboardRequest"),
        "clipboard command should accept a request object so selected audio can be passed"
    );
    assert!(
        app.contains("crate::library::copy_clip_to_clipboard"),
        "clipboard command should stay registered with Tauri"
    );
    assert!(
        js.contains("await invoke(\"copy_clip_to_clipboard\", {")
            && js.contains("request: {")
            && js.contains("path: currentClip.path")
            && js.contains("audioTrackIds: clipAudioTracks(currentClip).length")
            && js.contains("selectedAudioTrackIdsForClip(currentClip)"),
        "copy should send the current selected audio tracks to the native clipboard exporter"
    );
}

#[test]
fn app_notice_toasts_auto_clear() {
    let js = main_js();

    assert!(
        js.contains("const NOTICE_TOAST_MS")
            && js.contains("let noticeToastTimer")
            && js.contains("function setNotice(message, { transient = false } = {})"),
        "app-wide notices should flow through a helper that can schedule transient toasts"
    );
    assert!(
        js.contains("window.setTimeout(() => {")
            && js.contains("if ($(\"notice\").textContent === message)"),
        "transient app-wide notices should clear themselves without erasing newer messages"
    );

    for required in [
        "setNotice(\"clip renamed\", { transient: true })",
        "setNotice(\"clip deleted\", { transient: true })",
        "setNotice(s.gc_deleted",
        ": `saved ${fmtDur(s.seconds)} ${savedKind}`, { transient: true });",
    ] {
        assert!(
            js.contains(required),
            "app-wide success notice should auto-clear via `{required}`"
        );
    }
}

#[test]
fn ui_is_split_into_markup_styles_and_logic() {
    let html = index_html();

    for asset in [
        "href=\"styles.css\"",
        "src=\"player-core.js\"",
        "src=\"main.js\"",
    ] {
        assert!(html.contains(asset), "index.html must reference {asset}");
    }

    assert!(
        !html.contains("<style"),
        "styles belong in ui/styles.css, not inline in index.html"
    );

    for (i, chunk) in html.split("<script").skip(1).enumerate() {
        let tag_end = chunk.find('>').expect("script tag closes");
        assert!(
            chunk[..tag_end].contains("src="),
            "script tag #{i} must load an external file (logic belongs in player-core.js/main.js)"
        );
        let body_end = chunk.find("</script>").expect("script element closes");
        assert!(
            chunk[tag_end + 1..body_end].trim().is_empty(),
            "script tag #{i} must not have an inline body"
        );
    }
}

#[test]
fn gallery_supports_multi_select_bulk_actions() {
    let html = index_html();
    let js = main_js();
    let css = styles_css();
    let library = library_rs();
    let app = app_rs();

    for required in [
        "id=\"gallery-select-toggle\"",
        ">Select multiple</button>",
        "class=\"gallery-filter-row\"",
        "class=\"gallery-filter-chips\"",
        "class=\"gallery-filter-actions\"",
        "id=\"gallery-bulk-bar\"",
        "id=\"bulk-count\"",
        "id=\"bulk-select-all\"",
        "id=\"bulk-clear\"",
        "id=\"bulk-delete\"",
        "id=\"bulk-cancel\"",
        "id=\"confirm-title\"",
    ] {
        assert!(
            html.contains(required),
            "gallery multi-select markup must include `{required}`"
        );
    }

    let filter_chips = html
        .find("class=\"gallery-filter-chips\"")
        .expect("gallery filter chip row exists");
    let select_toggle = html
        .find("id=\"gallery-select-toggle\"")
        .expect("gallery select toggle exists");
    let bulk_bar = html
        .find("id=\"gallery-bulk-bar\"")
        .expect("gallery bulk action bar exists");
    let gallery_grid = html
        .find("id=\"gallery-grid\"")
        .expect("gallery grid exists");
    assert!(
        filter_chips < select_toggle && select_toggle < bulk_bar && bulk_bar < gallery_grid,
        "bulk action bar must live inside the filter toolbar before the grid"
    );
    assert!(
        html.find("id=\"gallery-sort\"")
            .expect("gallery sort exists")
            < filter_chips,
        "Select multiple should live in the filter toolbar, not the main gallery header"
    );

    for required in [
        "let selectedClipPaths",
        "let selectMode",
        "function toggleClipSelection",
        "function clearSelection",
        "function selectAllVisible",
        "function exitSelectMode",
        "function syncSelectionControls",
        "function syncBulkBar",
        "function applyDeletion",
        "function deletionNotice",
        "function confirmBulkDelete",
        "function bulkDeleteSelected",
        "const DEFAULT_DELETE_CONFIRM_TITLE",
        "dataset.clipPath",
        "selectedClipPaths.has(c.path)",
        "Select multiple",
        "selectMode || count > 0",
        "await invoke(\"delete_clips\"",
        "gallerySource !== \"local\"",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire multi-select behavior through `{required}`"
        );
    }

    assert!(
        library.contains("pub async fn delete_clips")
            && library.contains("fn delete_clips_impl")
            && library.contains("fn remove_clip_files")
            && library.contains("DeletedClipsReport"),
        "library.rs must expose a shared deletion helper, batch delete command, testable core, and report struct"
    );
    assert!(
        app.contains("crate::library::delete_clips"),
        "native command registry must register delete_clips"
    );

    for required in [
        ".gallery-bulk-bar",
        ".gallery-bulk-bar[hidden]",
        ".gallery-filter-row",
        ".gallery-filter-chips",
        ".gallery-filter-actions",
        ".gallery-grid.select-mode .card",
        ".gallery-grid.select-mode .card-del",
        ".card.selected",
    ] {
        assert!(
            css.contains(required),
            "multi-select UI needs stable styling for `{required}`"
        );
    }
    assert!(
        !js.contains("card-check")
            && !js.contains("check.addEventListener")
            && !js.contains("bulkUploadSelected")
            && !js.contains("uploadOneClipBulk")
            && !css.contains(".card-check"),
        "multi-select mode should use whole-card selection, not a competing per-card checkbox"
    );
    assert!(
        !html.contains("id=\"bulk-upload\"") && !html.contains("Upload to cloud"),
        "bulk actions should not expose bulk cloud upload"
    );

    let delete_clip_fn = js
        .split("async function deleteClip")
        .nth(1)
        .and_then(|rest| rest.split("async function openFolder").next())
        .expect("deleteClip function body exists");
    assert!(
        delete_clip_fn.contains("await applyDeletion([path]);"),
        "single delete should use the shared post-delete reconciliation helper"
    );

    let bulk_delete_fn = js
        .split("async function bulkDeleteSelected")
        .nth(1)
        .and_then(|rest| rest.split("/* ---- backend events ---- */").next())
        .expect("bulkDeleteSelected function body exists");
    assert!(
        bulk_delete_fn.contains("await applyDeletion(report.deleted);"),
        "bulk delete should refresh storage and close the current review through the shared helper"
    );
    assert!(
        bulk_delete_fn.contains("deletionNotice(report.deleted.length)"),
        "bulk delete should suppress zero-delete notices and pluralize nonzero deletes"
    );
    assert!(
        bulk_delete_fn.contains("formatDeletionFailures(report.failed)"),
        "bulk delete must surface partial failures even when the current clip was removed"
    );

    let select_all_fn = js
        .split("function selectAllVisible")
        .nth(1)
        .and_then(|rest| rest.split("function exitSelectMode").next())
        .expect("selectAllVisible function body exists");
    assert!(
        !select_all_fn.contains("galleryGroups(sortGalleryClips(filterGalleryClips(clipsCache)))"),
        "selectAllVisible should select from rendered card paths without re-running the gallery pipeline"
    );

    let select_toggle_handler = js
        .split("$(\"gallery-select-toggle\").addEventListener(\"click\"")
        .nth(1)
        .and_then(|rest| rest.split("$(\"bulk-select-all\")").next())
        .expect("gallery select toggle handler exists");
    assert!(
        !select_toggle_handler.contains("gallery-grid")
            && !select_toggle_handler.contains("classList.add(\"select-mode\")")
            && !select_toggle_handler.contains("classList.remove(\"select-mode\")"),
        "select-mode visual class ownership should live in the selection sync helpers"
    );

    let delete_clip_rs = library
        .split("pub fn delete_clip")
        .nth(1)
        .and_then(|rest| rest.split("pub struct DeletedClipsReport").next())
        .expect("delete_clip command body exists");
    assert!(
        delete_clip_rs.contains("remove_clip_files(&target)"),
        "single delete should call the same file-removal helper as bulk delete"
    );
    let delete_clips_impl_rs = library
        .split("fn delete_clips_impl")
        .nth(1)
        .and_then(|rest| rest.split("pub async fn delete_clips").next())
        .expect("delete_clips_impl body exists");
    assert!(
        delete_clips_impl_rs.contains("remove_clip_files(&target)"),
        "bulk delete should call the shared file-removal helper"
    );
}
