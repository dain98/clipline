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
        "id=\"rail-dot\"",
        "id=\"rail-status-text\"",
        "id=\"rail-status\"",
        "id=\"rail-status\" title=\"Stop recording\" aria-pressed=\"true\"",
        "id=\"rail-save\"",
        "id=\"rail-library-status\"",
        "id=\"rail-clips-count\"",
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
            && main_js().contains("function applySelectedAudioTracksToPlayback()")
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
            && app_rs().contains("crate::library::preview_clip_audio_tracks")
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
            && main_js().contains("games.plugins")
            && main_js().contains("dataset.gamePluginEnabled")
            && main_js().contains("game-plugin-mode-")
            && main_js().contains("normalizeGamePluginId")
            && main_js().contains("Takes priority over matching custom games.")
            && styles_css().contains(".game-profile-mode"),
        "supported games must render from backend game plugins, not hardcoded rows"
    );
    assert!(
        main_js().contains("const leagueMeta = playerSummaryLabel")
            && main_js().contains(
                "const leagueSessionTitle = isLeagueFullSessionClip(c, kind) && leagueMeta"
            )
            && main_js().contains("? leagueMeta")
            && main_js().contains("function clipLibraryTitle(clip, fallbackTitle)")
            && main_js().contains("if (isLeagueClip(clip)) return fallbackTitle")
            && main_js().contains("const clipName = clip && String(clip.name || \"\").trim()")
            && main_js().contains("return clipName || fallbackTitle")
            && main_js().contains("detail.className = \"league-meta\"")
            && main_js().contains("if (leagueMeta && !leagueSessionTitle)")
            && main_js().contains("const infoParts = []")
            && main_js().contains(
                "if (Number.isFinite(c.duration_s)) infoParts.push(fmtDur(c.duration_s))"
            )
            && main_js().contains("if (!leagueMeta && digest) infoParts.push(digest)")
            && styles_css().contains(".clip .league-meta"),
        "League rows must keep their title rules while non-League rows use actual clip names"
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
fn audio_preview_generation_is_not_eager_on_clip_open() {
    let js = main_js();
    let open_clip_start = js.find("function openClip(clip)").unwrap();
    let close_review_start = js.find("function closeReview()").unwrap();
    let open_clip = &js[open_clip_start..close_review_start];

    assert!(
        !open_clip.contains("applySelectedAudioTracksToPlayback()"),
        "opening a clip must not eagerly remux or mix a full-session audio preview"
    );
    assert!(
        js.contains("selected.length === tracks.length && currentReviewMediaPath === clip.path"),
        "all-track playback should keep the original source until the user changes selection"
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
            && js.contains("$(\"clip-menu-upload\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-rename\").addEventListener(\"click\"")
            && js.contains("$(\"clip-menu-delete\").addEventListener(\"click\"")
            && js.contains("function beginClipRename")
            && js.contains("await invoke(\"rename_clip\"")
            && app_rs().contains("crate::library::rename_clip")
            && css.contains(".clip-title-edit")
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
