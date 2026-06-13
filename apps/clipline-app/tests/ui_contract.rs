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
        "id=\"review-empty\"",
        "id=\"play-toggle\"",
        "id=\"seek-back\"",
        "id=\"seek-forward\"",
        "id=\"prev-marker\"",
        "id=\"next-marker\"",
        "id=\"marker-count\"",
        "id=\"timeline\"",
        "id=\"handle-in\"",
        "id=\"handle-out\"",
        "id=\"time-readout\"",
        "id=\"rate-select\"",
        "id=\"mute-toggle\"",
        "id=\"volume-slider\"",
        "id=\"export-clip\"",
        "id=\"trim-summary\"",
        "id=\"delete-clip\"",
        "id=\"ruler\"",
        "id=\"open-folder\"",
        "id=\"stage-frame\"",
        "id=\"stage-overlay\"",
        "id=\"sidebar-toggle\"",
        "id=\"memory-usage\"",
        "id=\"capture-status\"",
        "id=\"capture-status-label\"",
        "id=\"rail-dot\"",
        "id=\"rail-status\"",
        "id=\"rail-save\"",
        "id=\"rail-settings\"",
        "id=\"confirm-dialog\"",
        "id=\"confirm-accept\"",
        "id=\"confirm-cancel\"",
        "id=\"settings-page\"",
        "id=\"settings-tabs\"",
        "id=\"open-settings\"",
        "id=\"set-capture\"",
        "id=\"set-output-enabled\"",
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
        "id=\"set-buffer\"",
        "id=\"set-encoder\"",
        "id=\"encoder-summary\"",
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

    // Removed on purpose (2026-06-12): the path lives in #pmeta, and clicking
    // the active library row again closes the clip.
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

    // Conventional ordering: transport glued to the stage, timeline below it.
    let transport = html.find("id=\"play-toggle\"").expect("play toggle");
    let timeline = html.find("id=\"timeline\"").expect("timeline");
    assert!(
        transport < timeline,
        "transport row must precede the timeline in the deck"
    );
    assert!(
        styles_css().contains(".stage-frame") && main_js().contains("updateStageFrame"),
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
        "id=\"sidebar-toggle\"",
        "id=\"open-folder\"",
        "id=\"rail-save\"",
        "id=\"rail-settings\"",
        "id=\"delete-clip\"",
        "id=\"export-clip\"",
        "id=\"open-settings\"",
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
fn no_native_browser_dialogs() {
    let js = main_js();
    // window.confirm/alert render browser chrome ("tauri.localhost says") —
    // use the in-app #confirm-dialog instead.
    for banned in ["confirm(", "alert("] {
        assert!(
            !js.contains(banned),
            "main.js must not call native {banned}…) — use the in-app dialog"
        );
    }
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
        "list_game_windows",
        "game-detection",
        "renderCustomGames",
        "refreshGameWindows",
        "customGames",
    ] {
        assert!(
            js.contains(required),
            "main.js must wire the custom game workflow through {required}"
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
