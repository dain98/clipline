//! Behavioral tests for the pure review-player logic in `ui/player-core.js`.
//!
//! The file is evaluated in Boa (pure-Rust JS interpreter), so these run on both
//! CI OSes with no Node/npm toolchain. `player-core.js` must stay DOM-free and
//! Tauri-free for this to work — that constraint is the point.

use boa_engine::{Context, Source};
use std::fs;
use std::path::Path;

fn player_core_context() -> Context {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/player-core.js");
    let source = fs::read_to_string(path).expect("read ui/player-core.js");
    let mut context = Context::default();
    context
        .eval(Source::from_bytes(&source))
        .expect("player-core.js evaluates standalone (no DOM, no Tauri globals)");
    context
}

/// Evaluate an expression and return its string conversion ("null" for null).
fn eval(context: &mut Context, expr: &str) -> String {
    let value = context
        .eval(Source::from_bytes(expr))
        .unwrap_or_else(|err| panic!("eval `{expr}`: {err}"));
    value
        .to_string(context)
        .unwrap_or_else(|err| panic!("stringify `{expr}`: {err}"))
        .to_std_string_escaped()
}

/// Evaluate an expression through JSON.stringify for structural comparison.
fn eval_json(context: &mut Context, expr: &str) -> String {
    eval(context, &format!("JSON.stringify({expr})"))
}

#[test]
fn fmt_dur_carries_seconds_into_minutes() {
    let mut ctx = player_core_context();
    // Splitting minutes before rounding renders 59.6 as "0:60".
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtDur(59.6)"), "1:00");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtDur(0)"), "0:00");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtDur(90)"), "1:30");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtDur(605.4)"), "10:05");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtDur(NaN)"), "?");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtDur(null)"), "?");
}

#[test]
fn encoder_caveat_only_warns_for_undecodable_non_h264_codecs() {
    let mut ctx = player_core_context();
    // H.264 always plays — never a caveat, regardless of the decodable set.
    assert_eq!(
        eval(&mut ctx, "PlayerCore.encoderCodecCaveat('h264', [])"),
        "null"
    );
    // HEVC/AV1 warn when not in the decodable set...
    assert!(eval(&mut ctx, "PlayerCore.encoderCodecCaveat('hevc', ['h264'])").contains("HEVC"));
    assert!(eval(&mut ctx, "PlayerCore.encoderCodecCaveat('av1', ['h264'])").contains("AV1"));
    // ...and stay quiet once the player reports it can decode them.
    assert_eq!(
        eval(
            &mut ctx,
            "PlayerCore.encoderCodecCaveat('av1', ['h264','av1'])"
        ),
        "null"
    );
}

#[test]
fn video_decode_probes_cover_hevc_and_av1_mp4_profiles() {
    let mut ctx = player_core_context();
    let codecs = eval_json(&mut ctx, "PlayerCore.videoDecodeProbes().map(p => p.codec)");
    assert_eq!(codecs, "[\"hevc\",\"av1\"]");
    // Each probe is a concrete mp4 codec query for canPlayType.
    assert!(eval(&mut ctx, "PlayerCore.videoDecodeProbes()[0].mime").contains("hvc1"));
    assert!(eval(&mut ctx, "PlayerCore.videoDecodeProbes()[1].mime").contains("av01"));
}

#[test]
fn fmt_tenths_keeps_a_tenth_and_carries() {
    let mut ctx = player_core_context();
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtTenths(0)"), "0:00.0");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtTenths(7.34)"), "0:07.3");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtTenths(59.97)"), "1:00.0");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtTenths(75.25)"), "1:15.3");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtTenths(NaN)"), "?");
}

#[test]
fn fmt_bytes_switches_units_at_a_gigabyte() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval(&mut ctx, "PlayerCore.fmtBytes(5 * 1024 * 1024)"),
        "5.0 MB"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.fmtBytes(1536 * 1024 * 1024)"),
        "1.5 GB"
    );
}

#[test]
fn setting_duration_labels_are_human_readable() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval(&mut ctx, "PlayerCore.settingDurationLabel(45)"),
        "45 sec"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.settingDurationLabel(60)"),
        "1 min"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.settingDurationLabel(90)"),
        "1 min 30 sec"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.settingDurationLabel(120)"),
        "2 min"
    );
}

#[test]
fn recording_quality_labels_hide_bitrate_jargon() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.recordingQualityPreset(0)"),
        r#"{"label":"Compact","bitrate":6,"hint":"smaller files"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.recordingQualityPreset(1)"),
        r#"{"label":"Balanced","bitrate":12,"hint":"good default"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.recordingQualityPreset(2)"),
        r#"{"label":"Sharp","bitrate":24,"hint":"more detail"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.recordingQualityPreset(3)"),
        r#"{"label":"Maximum","bitrate":40,"hint":"largest files"}"#
    );
    assert_eq!(eval(&mut ctx, "PlayerCore.qualityIndexForBitrate(13)"), "1");
    assert_eq!(eval(&mut ctx, "PlayerCore.qualityIndexForBitrate(35)"), "3");
}

#[test]
fn smoothness_slider_maps_to_valid_fps_values() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.smoothnessPreset(0)"),
        r#"{"fps":30,"label":"30 FPS","hint":"lighter on the PC"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.smoothnessPreset(1)"),
        r#"{"fps":60,"label":"60 FPS","hint":"good default for most games"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.smoothnessPreset(2)"),
        r#"{"fps":90,"label":"90 FPS","hint":"smoother for high-refresh play"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.smoothnessPreset(3)"),
        r#"{"fps":120,"label":"120 FPS","hint":"best for high-refresh footage"}"#
    );
    assert_eq!(eval(&mut ctx, "PlayerCore.smoothnessIndexForFps(60)"), "1");
    assert_eq!(eval(&mut ctx, "PlayerCore.smoothnessIndexForFps(90)"), "2");
    assert_eq!(eval(&mut ctx, "PlayerCore.smoothnessIndexForFps(120)"), "3");
}

#[test]
fn capture_source_labels_are_sidebar_friendly() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval(
            &mut ctx,
            "PlayerCore.captureSourceLabel({ capture_mode: 'primary_monitor' })"
        ),
        "Desktop"
    );
    assert_eq!(
        eval(
            &mut ctx,
            "PlayerCore.captureSourceLabel({ capture_mode: 'window_title', window_title: 'League of Legends' })"
        ),
        "Window: League of Legends"
    );
    assert_eq!(
        eval(
            &mut ctx,
            "PlayerCore.captureSourceLabel({ capture_mode: 'display_region' })"
        ),
        "Display region"
    );
}

#[test]
fn fmt_ago_is_pure_in_now() {
    let mut ctx = player_core_context();
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtAgo(1000, 958)"), "42s ago");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtAgo(1000, 700)"), "5m ago");
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtAgo(11800, 1000)"), "3h ago");
    // Clock skew must not produce negative ages.
    assert_eq!(eval(&mut ctx, "PlayerCore.fmtAgo(1000, 1005)"), "0s ago");
}

#[test]
fn clamp_time_and_percent_respect_duration() {
    let mut ctx = player_core_context();
    assert_eq!(eval(&mut ctx, "PlayerCore.clampTime(-3, 10)"), "0");
    assert_eq!(eval(&mut ctx, "PlayerCore.clampTime(12, 10)"), "10");
    // Unknown duration must not clamp the high side to zero.
    assert_eq!(eval(&mut ctx, "PlayerCore.clampTime(5, 0)"), "5");
    assert_eq!(eval(&mut ctx, "PlayerCore.percentFor(5, 10)"), "50");
    assert_eq!(eval(&mut ctx, "PlayerCore.percentFor(15, 10)"), "100");
    assert_eq!(eval(&mut ctx, "PlayerCore.percentFor(3, 0)"), "0");
}

#[test]
fn timeline_time_maps_pointer_x_to_clip_time() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval(&mut ctx, "PlayerCore.timelineTime(150, 100, 200, 10)"),
        "2.5"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.timelineTime(50, 100, 200, 10)"),
        "0"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.timelineTime(400, 100, 200, 10)"),
        "10"
    );
}

#[test]
fn resolve_trim_clamps_and_keeps_minimum_gap() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.resolveTrim(2, 8, 10)"),
        r#"{"start":2,"end":8}"#
    );
    // Non-finite inputs fall back to the full clip.
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.resolveTrim(NaN, NaN, 10)"),
        r#"{"start":0,"end":10}"#
    );
    // Inverted bounds push the out point forward by the minimum gap.
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.resolveTrim(5, 3, 10)"),
        r#"{"start":5,"end":5.1}"#
    );
    // At end-of-clip the gap is taken from the in point instead.
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.resolveTrim(12, 0, 10)"),
        r#"{"start":9.9,"end":10}"#
    );
    // Zero-duration (metadata not loaded yet) stays sane.
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.resolveTrim(0, 0, 0)"),
        r#"{"start":0,"end":0}"#
    );
}

#[test]
fn trim_drag_keeps_handles_apart() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.trimDrag('in', 9.95, 2, 10, 10)"),
        r#"{"start":9.9,"end":10}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.trimDrag('out', 1, 2, 8, 10)"),
        r#"{"start":2,"end":2.1}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.trimDrag('in', 4, 2, 10, 10)"),
        r#"{"start":4,"end":10}"#
    );
}

#[test]
fn trim_summary_reports_range_and_length() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval(&mut ctx, "PlayerCore.trimSummary(2, 18.4)"),
        "keeps 0:02.0 \u{2013} 0:18.4 \u{b7} 16.4 s"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.trimSummary(0, 0)"),
        "keeps 0:00.0 \u{2013} 0:00.0 \u{b7} 0.0 s"
    );
}

#[test]
fn marker_navigation_skips_nearby_and_wraps() {
    let mut ctx = player_core_context();
    ctx.eval(Source::from_bytes(
        "const M = [{ t_s: 1 }, { t_s: 5 }, { t_s: 9 }];",
    ))
    .expect("define markers");
    assert_eq!(eval(&mut ctx, "PlayerCore.nextMarker(M, 4.9).t_s"), "5");
    // A marker within the epsilon of the playhead is "current", not "next".
    assert_eq!(eval(&mut ctx, "PlayerCore.nextMarker(M, 4.96).t_s"), "9");
    assert_eq!(eval(&mut ctx, "PlayerCore.nextMarker(M, 9.5).t_s"), "1");
    assert_eq!(eval(&mut ctx, "PlayerCore.prevMarker(M, 5).t_s"), "1");
    assert_eq!(eval(&mut ctx, "PlayerCore.prevMarker(M, 0.5).t_s"), "9");
    assert_eq!(eval_json(&mut ctx, "PlayerCore.nextMarker([], 0)"), "null");
    assert_eq!(eval_json(&mut ctx, "PlayerCore.prevMarker([], 0)"), "null");
}

#[test]
fn marker_count_pluralizes() {
    let mut ctx = player_core_context();
    assert_eq!(eval(&mut ctx, "PlayerCore.markerSummary([])"), "no markers");
    assert_eq!(
        eval(&mut ctx, "PlayerCore.markerSummary([{ t_s: 1 }])"),
        "1 marker"
    );
    assert_eq!(
        eval(
            &mut ctx,
            "PlayerCore.markerSummary([{ t_s: 1 }, { t_s: 2 }])"
        ),
        "2 markers"
    );
}

#[test]
fn key_intents_cover_the_documented_shortcuts() {
    let mut ctx = player_core_context();
    for code in ["Space", "KeyK"] {
        assert_eq!(
            eval_json(&mut ctx, &format!("PlayerCore.keyIntent('{code}', false)")),
            r#"{"kind":"toggle-play"}"#
        );
    }
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('ArrowLeft', false)"),
        r#"{"kind":"seek-by","seconds":-5}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('ArrowLeft', true)"),
        r#"{"kind":"seek-by","seconds":-1}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('KeyL', false)"),
        r#"{"kind":"seek-by","seconds":5}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('ArrowRight', true)"),
        r#"{"kind":"seek-by","seconds":1}"#
    );
    // Fine stepping for precise trim placement.
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('Comma', false)"),
        r#"{"kind":"seek-by","seconds":-0.1}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('Period', false)"),
        r#"{"kind":"seek-by","seconds":0.1}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('KeyI', false)"),
        r#"{"kind":"set-in"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('KeyO', false)"),
        r#"{"kind":"set-out"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('KeyM', false)"),
        r#"{"kind":"next-marker"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('KeyM', true)"),
        r#"{"kind":"prev-marker"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('Escape', false)"),
        r#"{"kind":"close"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('KeyZ', false)"),
        "null"
    );
}

#[test]
fn hotkey_recorder_formats_modifier_function_keys() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.hotkeyFromKeyEvent({ code: 'F10', ctrlKey: false, altKey: false, shiftKey: false })"
        ),
        r#"{"kind":"captured","value":"F10"}"#
    );
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.hotkeyFromKeyEvent({ code: 'F9', ctrlKey: true, altKey: true, shiftKey: false })"
        ),
        r#"{"kind":"captured","value":"Ctrl+Alt+F9"}"#
    );
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.hotkeyFromKeyEvent({ key: 'F13', ctrlKey: false, altKey: true, shiftKey: true })"
        ),
        r#"{"kind":"captured","value":"Alt+Shift+F13"}"#
    );
}

#[test]
fn hotkey_recorder_reports_pending_cancel_and_invalid_inputs() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.hotkeyFromKeyEvent({ code: 'ControlLeft', ctrlKey: true })"
        ),
        r#"{"kind":"pending","message":"Now press an F-key."}"#
    );
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.hotkeyFromKeyEvent({ code: 'Escape' })"
        ),
        r#"{"kind":"cancel"}"#
    );
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.hotkeyFromKeyEvent({ code: 'F10', ctrlKey: false, altKey: false, shiftKey: false })"
        ),
        r#"{"kind":"captured","value":"F10"}"#
    );
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.hotkeyFromKeyEvent({ code: 'F12', ctrlKey: false, altKey: false, shiftKey: false })"
        ),
        r#"{"kind":"invalid","message":"F12 is reserved by Windows for debuggers."}"#
    );
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.hotkeyFromKeyEvent({ code: 'KeyS', ctrlKey: true, altKey: false, shiftKey: false })"
        ),
        r#"{"kind":"invalid","message":"Use F1-F11 or F13-F24 as the shortcut key."}"#
    );
}

#[test]
fn marker_styles_map_kinds_to_categories() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.markerStyle('ChampionKill')"),
        r#"{"glyph":"✕","cls":"kill"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.markerStyle('FirstBlood')"),
        r#"{"glyph":"✕","cls":"kill"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.markerStyle('Multikill')"),
        r#"{"glyph":"★","cls":"spree"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.markerStyle('BaronKill')"),
        r#"{"glyph":"◆","cls":"objective"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.markerStyle('TurretKilled')"),
        r#"{"glyph":"▣","cls":"structure"}"#
    );
    // Unknown / future kinds degrade to the info dot, never crash.
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.markerStyle('GameStart')"),
        r#"{"glyph":"•","cls":"info"}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.markerStyle('SomethingNew')"),
        r#"{"glyph":"•","cls":"info"}"#
    );
}

#[test]
fn ruler_marks_pick_nice_steps() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.rulerMarks(22, 8).map(m => m.t)"),
        "[0,5,10,15,20]"
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.rulerMarks(22, 8).map(m => m.label)"),
        r#"["0:00","0:05","0:10","0:15","0:20"]"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.rulerMarks(1200, 8).map(m => m.t)"),
        "[0,300,600,900,1200]"
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.rulerMarks(6, 8).map(m => m.t)"),
        "[0,1,2,3,4,5,6]"
    );
    assert_eq!(eval_json(&mut ctx, "PlayerCore.rulerMarks(0, 8)"), "[]");
}

#[test]
fn clip_titles_use_twelve_hour_time() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval(&mut ctx, "PlayerCore.formatClipTitle(5, 11, 22, 25)"),
        "Jun 11 · 10:25 PM"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.formatClipTitle(0, 1, 0, 5)"),
        "Jan 1 · 12:05 AM"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.formatClipTitle(11, 31, 12, 0)"),
        "Dec 31 · 12:00 PM"
    );
}

#[test]
fn clip_kind_distinguishes_trims_from_replays() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval(&mut ctx, "PlayerCore.clipKind('clip_20260612_212006.mp4')"),
        "replay"
    );
    assert_eq!(
        eval(
            &mut ctx,
            "PlayerCore.clipKind('clip_20260612_212006_trim_001000_002000.mp4')"
        ),
        "trim"
    );
    assert_eq!(eval(&mut ctx, "PlayerCore.clipKind('')"), "replay");
}

#[test]
fn marker_digest_collapses_categories() {
    let mut ctx = player_core_context();
    ctx.eval(Source::from_bytes(
        "const D = [{ kind: 'ChampionKill' }, { kind: 'ChampionKill' }, { kind: 'DragonKill' }];",
    ))
    .expect("define markers");
    assert_eq!(
        eval(&mut ctx, "PlayerCore.markerDigest(D)"),
        "2 kills · 1 objective"
    );
    assert_eq!(
        eval(&mut ctx, "PlayerCore.markerDigest([{ kind: 'Multikill' }])"),
        "1 spree"
    );
    assert_eq!(eval(&mut ctx, "PlayerCore.markerDigest([])"), "");
}

#[test]
fn session_groups_bucket_and_sort_by_newest() {
    let mut ctx = player_core_context();
    ctx.eval(Source::from_bytes(
        r#"const C = [
            { path: "c.mp4", session: "2026-06-12 14-30", modified_unix: 100 },
            { path: "a.mp4", session: null, modified_unix: 50 },
            { path: "d.mp4", session: "2026-06-12 14-30", modified_unix: 200 },
            { path: "b.mp4", session: "2026-06-11 09-00", modified_unix: 150 },
        ];"#,
    ))
    .expect("define clips");
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.sessionGroups(C).map(g => g.label)"),
        r#"["2026-06-12 14-30","2026-06-11 09-00","Earlier"]"#
    );
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.sessionGroups(C)[0].clips.map(c => c.path)"
        ),
        r#"["d.mp4","c.mp4"]"#
    );
    assert_eq!(eval_json(&mut ctx, "PlayerCore.sessionGroups([])"), "[]");
}

#[test]
fn focus_mode_has_a_key() {
    let mut ctx = player_core_context();
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.keyIntent('KeyF', false)"),
        r#"{"kind":"toggle-focus"}"#
    );
}

#[test]
fn overlay_pins_while_paused_and_fades_when_idle() {
    let mut ctx = player_core_context();
    // Paused: always visible, no matter how stale the activity.
    assert_eq!(
        eval(&mut ctx, "PlayerCore.overlayVisible(true, 999999)"),
        "true"
    );
    // Playing with fresh pointer activity: visible.
    assert_eq!(
        eval(&mut ctx, "PlayerCore.overlayVisible(false, 500)"),
        "true"
    );
    // Playing and idle past the threshold: hidden.
    assert_eq!(
        eval(&mut ctx, "PlayerCore.overlayVisible(false, 2500)"),
        "false"
    );
    assert_eq!(eval(&mut ctx, "PlayerCore.OVERLAY_HIDE_MS"), "2000");
}

#[test]
fn shared_constants_are_exposed() {
    let mut ctx = player_core_context();
    assert_eq!(eval(&mut ctx, "PlayerCore.MIN_TRIM_GAP_S"), "0.1");
    assert_eq!(eval(&mut ctx, "PlayerCore.MARKER_EPSILON_S"), "0.05");
}

#[test]
fn display_bounds_cover_the_virtual_desktop() {
    let mut ctx = player_core_context();
    ctx.eval(Source::from_bytes(
        r#"const DISPLAYS = [
            { id: 'DISPLAY1', name: 'Display 1', x: -1280, y: 120, width: 1280, height: 720, is_primary: false },
            { id: 'DISPLAY2', name: 'Display 2', x: 0, y: 0, width: 1920, height: 1080, is_primary: true },
        ];"#,
    ))
    .expect("define displays");

    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.displayBounds(DISPLAYS)"),
        r#"{"x":-1280,"y":0,"width":3200,"height":1080}"#
    );
}

#[test]
fn display_map_layout_fits_displays_into_viewport() {
    let mut ctx = player_core_context();
    ctx.eval(Source::from_bytes(
        r#"const DISPLAYS = [
            { id: 'DISPLAY1', name: 'Display 1', x: 0, y: 0, width: 1920, height: 1080, is_primary: true },
            { id: 'DISPLAY2', name: 'Display 2', x: 1920, y: 0, width: 2560, height: 1440, is_primary: false },
        ];
        const LAYOUT = PlayerCore.displayMapLayout(DISPLAYS, 500, 250, 10);"#,
    ))
    .expect("define layout");

    assert_eq!(
        eval_json(&mut ctx, "LAYOUT.bounds"),
        r#"{"x":0,"y":0,"width":4480,"height":1440}"#
    );
    assert_eq!(eval(&mut ctx, "Math.round(LAYOUT.scale * 1000)"), "107");
    assert_eq!(eval(&mut ctx, "Math.round(LAYOUT.displays[1].left)"), "216");
    assert_eq!(
        eval(&mut ctx, "Math.round(LAYOUT.displays[1].width)"),
        "274"
    );
}

#[test]
fn display_map_height_scales_with_virtual_desktop_shape() {
    let mut ctx = player_core_context();
    ctx.eval(Source::from_bytes(
        r#"const WIDE_STACK = [
            { id: 'DISPLAY2', name: 'Display 2', x: 0, y: 0, width: 5120, height: 1440, is_primary: true },
            { id: 'DISPLAY1', name: 'Display 1', x: 1920, y: 1440, width: 1920, height: 1080, is_primary: false },
        ];
        const SINGLE = [
            { id: 'DISPLAY1', name: 'Display 1', x: 0, y: 0, width: 1920, height: 1080, is_primary: true },
        ];"#,
    ))
    .expect("define displays");

    assert_eq!(
        eval(
            &mut ctx,
            "Math.round(PlayerCore.displayMapHeight(SINGLE, 620))"
        ),
        "358"
    );
    assert_eq!(
        eval(
            &mut ctx,
            "Math.round(PlayerCore.displayMapHeight(WIDE_STACK, 620))"
        ),
        "315"
    );
}

#[test]
fn region_helpers_set_align_and_clamp_to_display() {
    let mut ctx = player_core_context();
    ctx.eval(Source::from_bytes(
        r#"const DISPLAY = { id: 'DISPLAY2', name: 'Display 2', x: 1920, y: -120, width: 2560, height: 1440, is_primary: false };
        const FULL = PlayerCore.regionForDisplay(DISPLAY);
        const SMALL = { display_id: 'DISPLAY2', x: 2000, y: 0, width: 800, height: 450 };"#,
    ))
    .expect("define display and region");

    assert_eq!(
        eval_json(&mut ctx, "FULL"),
        r#"{"display_id":"DISPLAY2","x":1920,"y":-120,"width":2560,"height":1440}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.alignRegion(SMALL, DISPLAY, 'left')"),
        r#"{"display_id":"DISPLAY2","x":1920,"y":0,"width":800,"height":450}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.alignRegion(SMALL, DISPLAY, 'right')"),
        r#"{"display_id":"DISPLAY2","x":3680,"y":0,"width":800,"height":450}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.alignRegion(SMALL, DISPLAY, 'top')"),
        r#"{"display_id":"DISPLAY2","x":2000,"y":-120,"width":800,"height":450}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.alignRegion(SMALL, DISPLAY, 'bottom')"),
        r#"{"display_id":"DISPLAY2","x":2000,"y":870,"width":800,"height":450}"#
    );
    assert_eq!(
        eval_json(&mut ctx, "PlayerCore.alignRegion(SMALL, DISPLAY, 'center')"),
        r#"{"display_id":"DISPLAY2","x":2800,"y":375,"width":800,"height":450}"#
    );
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.clampRegionToDisplay({ display_id: 'DISPLAY2', x: 1000, y: -900, width: 4000, height: 2000 }, DISPLAY)"
        ),
        r#"{"display_id":"DISPLAY2","x":1920,"y":-120,"width":2560,"height":1440}"#
    );
    assert_eq!(
        eval_json(
            &mut ctx,
            "PlayerCore.clampRegionToDisplay({ display_id: 'DISPLAY2', x: 2000, y: 0, width: 801, height: 451 }, DISPLAY)"
        ),
        r#"{"display_id":"DISPLAY2","x":2000,"y":0,"width":800,"height":450}"#
    );
}
