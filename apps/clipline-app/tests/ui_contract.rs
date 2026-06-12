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
        "id=\"stage-overlay\"",
        "id=\"sidebar-toggle\"",
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
        "id=\"set-window\"",
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
        "id=\"set-hotkey\"",
        "id=\"settings-save\"",
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

    // Settings is a page in the main pane now, not a sidebar fold.
    assert!(
        !html.contains("settings-fold"),
        "the sidebar settings fold was replaced by #settings-page"
    );
    assert!(
        !html.contains("id=\"settings-close\""),
        "the settings page closes from the bottom-left Settings control, not an extra X button"
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
