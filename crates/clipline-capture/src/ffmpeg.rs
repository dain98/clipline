//! FFmpeg subprocess discovery and encoder probing (ddoc §4).
//!
//! Clipline drives a bundled `ffmpeg.exe` (LGPL shared build) as a child
//! process rather than linking libavcodec — see the milestone plan for the
//! rationale (no unsafe FFI, version-robust, clean LGPL separation). This
//! module locates the binary and reports which of our target encoders it
//! can actually use on this machine. `ffmpeg -encoders` lists every
//! *compiled* encoder regardless of hardware, so each hardware encoder is
//! confirmed with a one-frame test encode before being reported.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::probe::{Codec, EncoderApi, EncoderBackend, EncoderCapability};

/// Stop Windows from flashing a console window for each `ffmpeg` child we
/// spawn (startup probing alone launches ~11 of them). No-op off Windows.
#[cfg(windows)]
pub fn suppress_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn suppress_console(_cmd: &mut Command) {}

/// The FFmpeg encoder names Clipline targets, mapped to (backend, codec).
/// Software AV1 is SVT-AV1 (LGPL-clean); no GPL x264/x265, no software HEVC.
const KNOWN_ENCODERS: &[(&str, EncoderBackend, Codec)] = &[
    ("h264_nvenc", EncoderBackend::Nvenc, Codec::H264),
    ("hevc_nvenc", EncoderBackend::Nvenc, Codec::Hevc),
    ("av1_nvenc", EncoderBackend::Nvenc, Codec::Av1),
    ("h264_amf", EncoderBackend::Amf, Codec::H264),
    ("hevc_amf", EncoderBackend::Amf, Codec::Hevc),
    ("av1_amf", EncoderBackend::Amf, Codec::Av1),
    ("h264_qsv", EncoderBackend::QuickSync, Codec::H264),
    ("hevc_qsv", EncoderBackend::QuickSync, Codec::Hevc),
    ("av1_qsv", EncoderBackend::QuickSync, Codec::Av1),
    ("libsvtav1", EncoderBackend::SvtAv1, Codec::Av1),
];

/// The FFmpeg `-c:v` encoder name for a (backend, codec) pair, if Clipline
/// targets it. Used by `FfmpegVideoEncoder` to build the child command.
pub fn encoder_name(backend: EncoderBackend, codec: Codec) -> Option<&'static str> {
    KNOWN_ENCODERS
        .iter()
        .find(|(_, b, c)| *b == backend && *c == codec)
        .map(|(name, _, _)| *name)
}

/// Software backends are always usable if FFmpeg lists them; hardware
/// backends must pass a test encode (the list is hardware-blind).
fn is_hardware(backend: EncoderBackend) -> bool {
    matches!(
        backend,
        EncoderBackend::Nvenc | EncoderBackend::Amf | EncoderBackend::QuickSync
    )
}

/// Parse `ffmpeg -encoders` output into the subset of [`KNOWN_ENCODERS`]
/// that FFmpeg was compiled with. Pure string work — the encoder name is
/// the second column on lines whose first column is the 6-char flag field
/// (e.g. `V....D`), which avoids matching names that appear in a
/// description.
pub fn parse_available_encoders(encoders_output: &str) -> Vec<(EncoderBackend, Codec)> {
    let mut found = Vec::new();
    for line in encoders_output.lines() {
        let mut tokens = line.split_whitespace();
        let Some(flags) = tokens.next() else { continue };
        if flags.len() != 6 || !flags.chars().all(|c| c == '.' || c.is_ascii_uppercase()) {
            continue;
        }
        let Some(name) = tokens.next() else { continue };
        if let Some((_, backend, codec)) = KNOWN_ENCODERS.iter().find(|(n, _, _)| *n == name) {
            found.push((*backend, *codec));
        }
    }
    found
}

/// Group flat (backend, codec) pairs into one [`EncoderCapability`] per
/// backend (api = FFmpeg), codecs in preference order.
fn group_capabilities(mut pairs: Vec<(EncoderBackend, Codec)>) -> Vec<EncoderCapability> {
    pairs.sort();
    pairs.dedup();
    let mut caps: Vec<EncoderCapability> = Vec::new();
    for (backend, codec) in pairs {
        match caps.iter_mut().find(|c| c.backend == backend) {
            Some(cap) => cap.codecs.push(codec),
            None => caps.push(EncoderCapability {
                api: EncoderApi::Ffmpeg,
                backend,
                codecs: vec![codec],
            }),
        }
    }
    for cap in &mut caps {
        cap.codecs.sort();
    }
    caps
}

static BUNDLED_FFMPEG: OnceLock<PathBuf> = OnceLock::new();

/// Register the packaged ffmpeg resource path discovered by the desktop shell.
/// The explicit environment override remains first so developers and users can
/// intentionally replace the subprocess binary.
pub fn set_bundled_ffmpeg(path: PathBuf) {
    let _ = BUNDLED_FFMPEG.set(path);
}

fn ffmpeg_exe_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

fn search_paths_from(
    exe_name: &str,
    explicit: Option<PathBuf>,
    current_exe: Option<PathBuf>,
    appdata: Option<PathBuf>,
    bundled: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(explicit) = explicit {
        paths.push(explicit);
    }
    if let Some(bundled) = bundled {
        paths.push(bundled);
    }
    if let Some(exe) = current_exe {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join(exe_name));
        }
    }
    if let Some(appdata) = appdata {
        paths.push(appdata.join("Clipline").join("ffmpeg").join(exe_name));
    }
    paths.push(PathBuf::from(exe_name)); // PATH fallback
    paths
}

/// Candidate locations for `ffmpeg`, most-specific first: an explicit
/// `CLIPLINE_FFMPEG` override, the packaged app resource, next to our own exe,
/// the per-user `%APPDATA%\Clipline\ffmpeg` bundle, then a bare name for a PATH
/// lookup.
pub fn search_paths() -> Vec<PathBuf> {
    search_paths_from(
        ffmpeg_exe_name(),
        std::env::var_os("CLIPLINE_FFMPEG").map(PathBuf::from),
        std::env::current_exe().ok(),
        std::env::var_os("APPDATA").map(PathBuf::from),
        BUNDLED_FFMPEG.get().cloned(),
    )
}

/// Locate a runnable `ffmpeg`: the first search path that answers
/// `-version` with success. `None` means the FFmpeg encoder tier is simply
/// unavailable (CI, or a machine without the bundle) — never an error.
pub fn locate() -> Option<PathBuf> {
    search_paths().into_iter().find(|path| runs(path))
}

/// All probe subprocesses must finish well within this; a wedged ffmpeg is
/// killed rather than allowed to block startup probing indefinitely.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Run a probe command to completion, killing it if it exceeds
/// `PROBE_TIMEOUT`. stdout is captured (these commands emit little); stderr is
/// discarded. `None` on spawn failure or timeout — treated as "unavailable".
fn run_bounded(mut cmd: Command) -> Option<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    suppress_console(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }
    child.wait_with_output().ok()
}

fn runs(path: &Path) -> bool {
    let mut cmd = Command::new(path);
    cmd.args(["-hide_banner", "-version"]);
    run_bounded(cmd)
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Confirm a hardware encoder actually works on this machine with a
/// one-frame test encode discarded to the null muxer. The probe size is
/// 640x360, not a tiny placeholder: AMF rejects very small resolutions
/// (`Init() failed with error 5` at 128x72), which would wrongly drop a
/// working H.264/HEVC encoder.
fn test_encode(ffmpeg: &Path, encoder: &str) -> bool {
    let mut cmd = Command::new(ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=640x360:rate=30",
        "-frames:v",
        "1",
        "-c:v",
        encoder,
        "-f",
        "null",
        "-",
    ]);
    run_bounded(cmd)
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Probe one located `ffmpeg` binary: list compiled encoders, then confirm
/// each hardware encoder with a test encode (software is trusted).
pub fn probe_ffmpeg(ffmpeg: &Path) -> Vec<EncoderCapability> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-encoders"]);
    let Some(output) = run_bounded(cmd) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let listed = parse_available_encoders(&String::from_utf8_lossy(&output.stdout));
    let usable: Vec<(EncoderBackend, Codec)> = listed
        .into_iter()
        .filter(|&(backend, codec)| {
            !is_hardware(backend)
                || encoder_name(backend, codec)
                    .map(|name| test_encode(ffmpeg, name))
                    .unwrap_or(false)
        })
        .collect();
    group_capabilities(usable)
}

/// Locate FFmpeg and probe it. Empty when no usable FFmpeg is present —
/// the app falls back to the MFT-only matrix exactly as before.
pub fn probe() -> Vec<EncoderCapability> {
    match locate() {
        Some(ffmpeg) => probe_ffmpeg(&ffmpeg),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed real `ffmpeg -encoders` excerpt (FFmpeg 8.x): the AMD box
    // lists nvenc/qsv/amf and libsvtav1/libx265 even without that hardware.
    const ENCODERS: &str = "\
 Encoders:
 V..... = Video
 ------
 V....D libaom-av1           libaom AV1 (codec av1)
 V..... libsvtav1            SVT-AV1 encoder (codec av1)
 V....D av1_nvenc            NVIDIA NVENC av1 encoder (codec av1)
 V..... av1_qsv              AV1 (Intel Quick Sync Video) (codec av1)
 V....D av1_amf              AMD AMF AV1 encoder (codec av1)
 V....D h264_amf             AMD AMF H.264 Encoder (codec h264)
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 V....D libx265              libx265 H.265 / HEVC (codec hevc)
 V....D hevc_amf             AMD AMF HEVC encoder (codec hevc)
 V....D hevc_nvenc           NVIDIA NVENC hevc encoder (codec hevc)
 V..... hevc_qsv             HEVC (Intel Quick Sync Video) (codec hevc)
 V..... h264_qsv             H.264 (Intel Quick Sync Video) (codec h264)
 A....D aac                  AAC (Advanced Audio Coding)";

    #[test]
    fn parses_known_encoders_and_ignores_others() {
        let found = parse_available_encoders(ENCODERS);
        // libaom-av1 and libx265 are real lines but not Clipline targets.
        assert!(!found.contains(&(EncoderBackend::SvtAv1, Codec::Hevc)));
        assert!(found.contains(&(EncoderBackend::SvtAv1, Codec::Av1)));
        assert!(found.contains(&(EncoderBackend::Nvenc, Codec::H264)));
        assert!(found.contains(&(EncoderBackend::Amf, Codec::Hevc)));
        assert!(found.contains(&(EncoderBackend::QuickSync, Codec::Av1)));
        assert_eq!(found.len(), 10, "9 hw + libsvtav1");
    }

    #[test]
    fn name_appearing_only_in_a_description_is_not_matched() {
        // The encoder-name column is column 2; a name in prose must not match.
        let prose = " V....D somecodec            uses h264_nvenc internally (codec foo)";
        assert!(parse_available_encoders(prose).is_empty());
    }

    #[test]
    fn grouping_yields_one_capability_per_backend_with_sorted_codecs() {
        let caps = group_capabilities(vec![
            (EncoderBackend::Amf, Codec::H264),
            (EncoderBackend::Amf, Codec::Av1),
            (EncoderBackend::Nvenc, Codec::H264),
            (EncoderBackend::Amf, Codec::H264), // duplicate
        ]);
        assert_eq!(caps.len(), 2);
        let amf = caps
            .iter()
            .find(|c| c.backend == EncoderBackend::Amf)
            .unwrap();
        assert_eq!(amf.api, EncoderApi::Ffmpeg);
        assert_eq!(
            amf.codecs,
            vec![Codec::H264, Codec::Av1],
            "preference order, deduped"
        );
    }

    #[test]
    fn encoder_name_round_trips_known_pairs() {
        assert_eq!(
            encoder_name(EncoderBackend::Amf, Codec::Hevc),
            Some("hevc_amf")
        );
        assert_eq!(
            encoder_name(EncoderBackend::SvtAv1, Codec::Av1),
            Some("libsvtav1")
        );
        // No software H.264/HEVC through FFmpeg (LGPL: no x264/x265).
        assert_eq!(encoder_name(EncoderBackend::SvtAv1, Codec::H264), None);
        assert_eq!(encoder_name(EncoderBackend::MfSoftware, Codec::H264), None);
    }

    #[test]
    fn search_paths_end_with_a_bare_path_lookup() {
        let paths = search_paths();
        let last = paths.last().unwrap();
        assert!(last.as_os_str() == "ffmpeg" || last.as_os_str() == "ffmpeg.exe");
    }

    fn fixture_path(parts: &[&str]) -> PathBuf {
        parts.iter().collect()
    }

    #[test]
    fn bundled_ffmpeg_resource_wins_over_appdata_and_path() {
        let install_exe = fixture_path(&["clipline-install", "Clipline.exe"]);
        let appdata = fixture_path(&["user-profile", "AppData", "Roaming"]);
        let bundled = fixture_path(&["clipline-install", "resources", "ffmpeg", "ffmpeg.exe"]);

        let paths = search_paths_from(
            "ffmpeg.exe",
            None,
            Some(install_exe),
            Some(appdata.clone()),
            Some(bundled.clone()),
        );

        assert_eq!(paths[0], bundled);
        assert_eq!(paths[1], fixture_path(&["clipline-install", "ffmpeg.exe"]));
        assert_eq!(
            paths[2],
            appdata.join("Clipline").join("ffmpeg").join("ffmpeg.exe")
        );
    }

    #[test]
    fn explicit_ffmpeg_override_stays_first() {
        let explicit = fixture_path(&["tools", "ffmpeg.exe"]);
        let bundled = fixture_path(&["clipline-install", "resources", "ffmpeg", "ffmpeg.exe"]);

        let paths = search_paths_from(
            "ffmpeg.exe",
            Some(explicit.clone()),
            Some(fixture_path(&["clipline-install", "Clipline.exe"])),
            None,
            Some(bundled.clone()),
        );

        assert_eq!(paths[0], explicit);
        assert_eq!(paths[1], bundled);
    }
}
