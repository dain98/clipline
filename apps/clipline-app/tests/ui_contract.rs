use std::fs;
use std::path::Path;

fn index_html() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/index.html");
    fs::read_to_string(path).expect("read ui/index.html")
}

#[test]
fn review_workspace_owns_player_controls() {
    let html = index_html();
    let video_start = html.find("<video").expect("video element exists");
    let video_end = html[video_start..]
        .find('>')
        .map(|offset| video_start + offset)
        .expect("video element closes");
    let video_tag = &html[video_start..=video_end];

    assert!(
        !video_tag.contains("controls"),
        "Clipline review workspace must hide native browser video controls"
    );

    for required in [
        "id=\"review-workspace\"",
        "id=\"play-toggle\"",
        "id=\"seek-back\"",
        "id=\"seek-forward\"",
        "id=\"mute-toggle\"",
        "id=\"rate-select\"",
        "id=\"volume-slider\"",
        "id=\"timeline\"",
        "id=\"trim-range\"",
        "id=\"trim-handle-in\"",
        "id=\"trim-handle-out\"",
        "id=\"prev-marker\"",
        "id=\"next-marker\"",
        "id=\"marker-summary\"",
        "id=\"clip-export\"",
        "id=\"clip-delete\"",
        "id=\"clip-reveal\"",
    ] {
        assert!(
            html.contains(required),
            "review workspace is missing required control {required}"
        );
    }
}
