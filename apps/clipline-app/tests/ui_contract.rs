//! Structural contract for the review player DOM: Clipline owns the controls,
//! the browser owns nothing, and the UI stays split into testable assets.

use std::fs;
use std::path::Path;

fn index_html() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/index.html");
    fs::read_to_string(path).expect("read ui/index.html")
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
        "id=\"rail-dot\"",
        "id=\"rail-save\"",
        "id=\"rail-settings\"",
        "id=\"confirm-dialog\"",
        "id=\"confirm-accept\"",
        "id=\"confirm-cancel\"",
    ] {
        assert!(
            html.contains(required),
            "review player is missing required control {required}"
        );
    }

    // Removed on purpose (2026-06-12): the path lives in #pmeta, and clicking
    // the active library row again closes the clip.
    for gone in ["id=\"copy-path\"", "id=\"close-review\"", "id=\"focus-toggle\""] {
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/main.js");
    let js = fs::read_to_string(path).expect("read ui/main.js");
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
