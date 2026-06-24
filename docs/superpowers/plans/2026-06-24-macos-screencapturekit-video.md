# macOS ScreenCaptureKit Video Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Clipline record and save primary-display replay clips on macOS using ScreenCaptureKit video frames and the existing replay/muxing pipeline.

**Architecture:** A small Swift helper owns ScreenCaptureKit because Rust has no existing Objective-C/Swift bridge in this app. The helper emits compact NV12 frames over stdout using a binary protocol; Rust treats that process as a `CaptureEngine`, pipes frames into the existing FFmpeg subprocess encoder, and reuses the existing replay ring and MP4 save path. This slice is intentionally video-only and primary-display-only.

**Tech Stack:** Rust, Tauri 2, Swift 6, ScreenCaptureKit, CoreVideo, FFmpeg `h264_videotoolbox`, existing `clipline-capture` recorder/muxer.

## Global Constraints

- Keep FFmpeg as a separate subprocess; do not link GPL or LGPL FFmpeg libraries.
- Add macOS encoding through `h264_videotoolbox`; do not introduce GPL `libx264` as a target encoder.
- The helper protocol is binary: stream header magic `CLNV`, frame magic `FRAM`, little-endian integers, and compact NV12 bytes.
- The helper emits `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange` (`420v`) and row-copies padded CVPixelBuffer planes into contiguous NV12.
- This slice supports `CaptureSource::PrimaryMonitor` only.
- This slice returns explicit errors for window capture, region capture, audio capture, and full-session recording on macOS.
- Development helper path is `target/clipline-sidecars/clipline-sck-helper` at the repository root.
- Bundled helper path is `Contents/Resources/clipline-sck-helper`.
- Preserve Windows capture behavior and settings ids except for adding the inert `video_toolbox_h264` settings id.
- Follow TDD: write a focused failing test, verify it fails for the expected reason, implement minimal production code, verify it passes, then commit.

---

## File Structure

- `crates/clipline-capture/src/probe.rs`: add `EncoderBackend::VideoToolbox` and ranking coverage.
- `crates/clipline-capture/src/ffmpeg.rs`: discover/probe `h264_videotoolbox`.
- `crates/clipline-capture/src/ffmpeg_encoder.rs`: add VideoToolbox FFmpeg argument flags.
- `apps/clipline-app/macos/ScreenCaptureKitHelper.swift`: Swift ScreenCaptureKit helper.
- `apps/clipline-app/build.rs`: compile the Swift helper into the repository `target/clipline-sidecars` folder on macOS.
- `apps/clipline-app/tauri.conf.json`: bundle `target/clipline-sidecars/clipline-sck-helper` as a root resource and allow macOS `Movies/Clipline` playback paths.
- `apps/clipline-app/src/main.rs`: wire `macos_capture` only on macOS.
- `apps/clipline-app/src/macos_capture.rs`: Rust helper process wrapper and binary protocol parser implementing `CaptureEngine`.
- `apps/clipline-app/src/service.rs`: add the cross-platform `VideoToolboxH264` settings id so settings round-trip everywhere.
- `apps/clipline-app/src/service_macos.rs`: replace the stub recorder loop with primary-display video replay recording.
- `apps/clipline-app/src/platform/macos.rs`: advertise implemented display capture and hardware encode while leaving unsupported features unavailable.
- `apps/clipline-app/tests/macos_shell_contract.rs`: update static macOS contracts from stub behavior to real helper/service behavior.
- `handoff.md`: update current state and the next sharp edges after verification.

---

### Task 1: Add FFmpeg VideoToolbox Encoder Support

**Files:**
- Modify: `crates/clipline-capture/src/probe.rs`
- Modify: `crates/clipline-capture/src/ffmpeg.rs`
- Modify: `crates/clipline-capture/src/ffmpeg_encoder.rs`
- Modify: `apps/clipline-app/src/service.rs`

**Interfaces:**
- Produces: `EncoderBackend::VideoToolbox`
- Produces: `ffmpeg::encoder_name(EncoderBackend::VideoToolbox, Codec::H264) == Some("h264_videotoolbox")`
- Produces: `VideoEncoder::VideoToolboxH264` with settings id `video_toolbox_h264`
- Consumed by Task 4: macOS service ranks/opens H.264 VideoToolbox candidates through the existing FFmpeg encoder constructor.

- [ ] **Step 1: Write failing encoder discovery tests**

Add tests before production code:

```rust
// crates/clipline-capture/src/ffmpeg.rs tests
#[test]
fn parses_videotoolbox_h264_encoder() {
    let output = "\
 Encoders:
 V..... = Video
 ------
 V....D h264_videotoolbox    VideoToolbox H.264 Encoder (codec h264)";
    let found = parse_available_encoders(output);
    assert_eq!(found, vec![(EncoderBackend::VideoToolbox, Codec::H264)]);
}

#[test]
fn encoder_name_knows_videotoolbox_h264_only() {
    assert_eq!(
        encoder_name(EncoderBackend::VideoToolbox, Codec::H264),
        Some("h264_videotoolbox")
    );
    assert_eq!(encoder_name(EncoderBackend::VideoToolbox, Codec::Hevc), None);
    assert_eq!(encoder_name(EncoderBackend::VideoToolbox, Codec::Av1), None);
}
```

```rust
// crates/clipline-capture/src/probe.rs tests
#[test]
fn auto_uses_videotoolbox_h264_before_software_av1() {
    let caps = vec![
        cap(EncoderApi::Ffmpeg, EncoderBackend::SvtAv1, &[Codec::Av1]),
        cap(EncoderApi::Ffmpeg, EncoderBackend::VideoToolbox, &[Codec::H264]),
    ];
    let ranked = rank_encoders(&caps, ALL_CODECS, EncoderPreference::Auto);
    assert_eq!(
        ranked[0],
        cand(EncoderApi::Ffmpeg, EncoderBackend::VideoToolbox, Codec::H264)
    );
}
```

```rust
// crates/clipline-capture/src/ffmpeg_encoder.rs tests
#[test]
fn backend_rate_control_videotoolbox_uses_realtime_h264_flags() {
    let rc = backend_rate_control(EncoderBackend::VideoToolbox, 4_000_000, 8_000_000);
    let joined = rc.join(" ");
    assert!(joined.contains("-b:v 4000000"));
    assert!(joined.contains("-constant_bit_rate true"));
    assert!(joined.contains("-allow_sw true"));
    assert!(joined.contains("-realtime true"));
}
```

```rust
// apps/clipline-app/src/service.rs tests
#[test]
fn video_toolbox_h264_has_stable_settings_id() {
    let enc = VideoEncoder::VideoToolboxH264;
    assert_eq!(enc.id(), "video_toolbox_h264");
    assert_eq!(
        VideoEncoder::from_parts(EncoderBackend::VideoToolbox, Codec::H264),
        Some(VideoEncoder::VideoToolboxH264)
    );
}
```

- [ ] **Step 2: Run tests to verify RED**

Run:

```bash
cargo test -p clipline-capture ffmpeg::tests::parses_videotoolbox_h264_encoder -- --exact
cargo test -p clipline-capture probe::tests::auto_uses_videotoolbox_h264_before_software_av1 -- --exact
cargo test -p clipline-capture ffmpeg_encoder::tests::backend_rate_control_videotoolbox_uses_realtime_h264_flags -- --exact
cargo test -p clipline-app service::tests::video_toolbox_h264_has_stable_settings_id -- --exact
```

Expected: each fails because `VideoToolbox`/`VideoToolboxH264` does not exist.

- [ ] **Step 3: Implement minimal encoder/backend support**

Make these exact shape changes:

```rust
// crates/clipline-capture/src/probe.rs
pub enum EncoderBackend {
    Nvenc,
    Amf,
    QuickSync,
    /// Apple VideoToolbox H.264 through FFmpeg on macOS.
    VideoToolbox,
    /// Software AV1 (SVT-AV1) via FFmpeg.
    SvtAv1,
    /// Microsoft software H.264 MFT — last resort.
    MfSoftware,
}
```

```rust
// crates/clipline-capture/src/ffmpeg.rs
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
    ("h264_videotoolbox", EncoderBackend::VideoToolbox, Codec::H264),
    ("libsvtav1", EncoderBackend::SvtAv1, Codec::Av1),
];

fn is_hardware(backend: EncoderBackend) -> bool {
    matches!(
        backend,
        EncoderBackend::Nvenc
            | EncoderBackend::Amf
            | EncoderBackend::QuickSync
            | EncoderBackend::VideoToolbox
    )
}
```

```rust
// crates/clipline-capture/src/ffmpeg_encoder.rs
match backend {
    EncoderBackend::VideoToolbox => vec![
        s("-b:v"),
        b,
        s("-constant_bit_rate"),
        s("true"),
        s("-allow_sw"),
        s("true"),
        s("-realtime"),
        s("true"),
    ],
    // existing arms unchanged
}
```

```rust
// apps/clipline-app/src/service.rs
pub enum VideoEncoder {
    Auto,
    NvencH264,
    NvencHevc,
    NvencAv1,
    AmfH264,
    AmfHevc,
    AmfAv1,
    QuickSyncH264,
    QuickSyncHevc,
    QuickSyncAv1,
    VideoToolboxH264,
    SvtAv1,
}
```

Add `VideoToolboxH264` to `preference`, `id`, `from_parts`, and existing round-trip tests.
Add `EncoderBackend::VideoToolbox => "Apple VideoToolbox"` to `encoder_label`.

- [ ] **Step 4: Run focused GREEN tests**

Run:

```bash
cargo test -p clipline-capture ffmpeg::tests::parses_videotoolbox_h264_encoder -- --exact
cargo test -p clipline-capture ffmpeg::tests::encoder_name_knows_videotoolbox_h264_only -- --exact
cargo test -p clipline-capture probe::tests::auto_uses_videotoolbox_h264_before_software_av1 -- --exact
cargo test -p clipline-capture ffmpeg_encoder::tests::backend_rate_control_videotoolbox_uses_realtime_h264_flags -- --exact
cargo test -p clipline-app service::tests::video_toolbox_h264_has_stable_settings_id -- --exact
```

Expected: all pass.

- [ ] **Step 5: Run changed-crate tests**

Run:

```bash
cargo test -p clipline-capture
cargo test -p clipline-app service::tests
```

Expected: both pass.

- [ ] **Step 6: Commit**

```bash
git add crates/clipline-capture/src/probe.rs crates/clipline-capture/src/ffmpeg.rs crates/clipline-capture/src/ffmpeg_encoder.rs apps/clipline-app/src/service.rs
git commit -m "feat(capture): add videotoolbox ffmpeg backend"
```

---

### Task 2: Build And Bundle The ScreenCaptureKit Helper

**Files:**
- Create: `apps/clipline-app/macos/ScreenCaptureKitHelper.swift`
- Modify: `apps/clipline-app/build.rs`
- Modify: `apps/clipline-app/tauri.conf.json`
- Modify: `apps/clipline-app/tests/macos_shell_contract.rs`

**Interfaces:**
- Produces: helper binary at repo-root `target/clipline-sidecars/clipline-sck-helper` on macOS builds.
- Produces: Tauri resource mapping `"../../target/clipline-sidecars/clipline-sck-helper": ""`.
- Consumed by Task 3: Rust helper lookup expects the dev path and bundled resource basename `clipline-sck-helper`.

- [ ] **Step 1: Write failing macOS build/bundle contract tests**

Add tests to `apps/clipline-app/tests/macos_shell_contract.rs`:

```rust
#[test]
fn macos_screencapturekit_helper_is_built_and_bundled() {
    let build = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs"))
        .expect("read build.rs");
    let config = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
        .expect("read tauri.conf.json");

    assert!(build.contains("ScreenCaptureKitHelper.swift"));
    assert!(build.contains("xcrun"));
    assert!(build.contains("swiftc"));
    assert!(build.contains("clipline-sidecars"));
    assert!(build.contains("clipline-sck-helper"));
    assert!(build.contains("cargo:rerun-if-changed=macos/ScreenCaptureKitHelper.swift"));
    assert!(config.contains("\"../../target/clipline-sidecars/clipline-sck-helper\": \"\""));
}

#[test]
fn macos_asset_scope_allows_default_movies_folder() {
    let config = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"))
        .expect("read tauri.conf.json");
    assert!(config.contains("\"**/Movies/Clipline/*.mp4\""));
    assert!(config.contains("\"**/Movies/Clipline/**/*.mp4\""));
}
```

- [ ] **Step 2: Run tests to verify RED**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_screencapturekit_helper_is_built_and_bundled -- --exact
cargo test -p clipline-app --test macos_shell_contract macos_asset_scope_allows_default_movies_folder -- --exact
```

Expected: first fails because no helper build/bundle contract exists; second fails because the asset scope only includes `Videos/Clipline`.

- [ ] **Step 3: Add a compiling Swift helper skeleton**

Create `apps/clipline-app/macos/ScreenCaptureKitHelper.swift`:

```swift
import Foundation

let version = "clipline-sck-helper 1"

if CommandLine.arguments.contains("--version") {
    FileHandle.standardOutput.write(Data((version + "\n").utf8))
    exit(0)
}

FileHandle.standardError.write(Data("ScreenCaptureKit capture entrypoint is not wired\n".utf8))
exit(64)
```

- [ ] **Step 4: Compile helper from `build.rs` on macOS**

Modify `apps/clipline-app/build.rs` so the macOS branch compiles before `tauri_build::build()`:

```rust
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" {
        build_macos_helper();
        tauri_build::build();
    } else if target_os == "windows" {
        tauri_build::build();
    }
}

fn build_macos_helper() {
    println!("cargo:rerun-if-changed=macos/ScreenCaptureKitHelper.swift");
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let helper_src = manifest_dir.join("macos/ScreenCaptureKitHelper.swift");
    let out_dir = repo_root(&manifest_dir).join("target/clipline-sidecars");
    std::fs::create_dir_all(&out_dir).expect("create target/clipline-sidecars");
    let helper_out = out_dir.join("clipline-sck-helper");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-O",
            "-target",
            "arm64-apple-macosx13.0",
            "-framework",
            "Foundation",
            "-o",
        ])
        .arg(&helper_out)
        .arg(&helper_src)
        .status()
        .expect("spawn xcrun swiftc for ScreenCaptureKit helper");
    assert!(status.success(), "swiftc failed for ScreenCaptureKit helper");
}

fn repo_root(manifest_dir: &Path) -> PathBuf {
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("apps/clipline-app lives two levels below repo root")
        .to_path_buf()
}
```

- [ ] **Step 5: Bundle helper as a root resource and add Movies asset scope**

Modify `apps/clipline-app/tauri.conf.json`:

```json
"assetProtocol": {
  "enable": true,
  "scope": [
    "**/Videos/Clipline/*.mp4",
    "**/Videos/Clipline/**/*.mp4",
    "**/Movies/Clipline/*.mp4",
    "**/Movies/Clipline/**/*.mp4"
  ]
}
```

Inside `"bundle"` add:

```json
"resources": {
  "../../target/clipline-sidecars/clipline-sck-helper": ""
}
```

- [ ] **Step 6: Run focused GREEN tests and helper smoke**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_screencapturekit_helper_is_built_and_bundled -- --exact
cargo test -p clipline-app --test macos_shell_contract macos_asset_scope_allows_default_movies_folder -- --exact
cargo build -p clipline-app
target/clipline-sidecars/clipline-sck-helper --version
```

Expected: tests pass, build succeeds, helper prints `clipline-sck-helper 1`.

- [ ] **Step 7: Commit**

```bash
git add apps/clipline-app/macos/ScreenCaptureKitHelper.swift apps/clipline-app/build.rs apps/clipline-app/tauri.conf.json apps/clipline-app/tests/macos_shell_contract.rs
git commit -m "feat(app): build macos screencapturekit helper"
```

---

### Task 3: Implement Helper Protocol And Rust Capture Wrapper

**Files:**
- Modify: `apps/clipline-app/macos/ScreenCaptureKitHelper.swift`
- Modify: `apps/clipline-app/build.rs`
- Create: `apps/clipline-app/src/macos_capture.rs`
- Modify: `apps/clipline-app/src/main.rs`
- Modify: `apps/clipline-app/tests/macos_shell_contract.rs`

**Interfaces:**
- Produces: `ScreenCaptureKitConfig { fps: u32, max_height: Option<u32> }`.
- Produces: `ScreenCaptureKitCapture::new(config) -> Result<ScreenCaptureKitCapture, CaptureError>`.
- Produces: `ScreenCaptureKitCapture::stream_info(&self) -> StreamInfo`.
- Produces: `impl CaptureEngine for ScreenCaptureKitCapture`.
- Consumed by Task 4: macOS service starts this capture, reads `stream_info()`, and passes CPU NV12 frames to `FfmpegVideoEncoder::new`.

- [ ] **Step 1: Write failing Rust protocol parser tests**

Create `apps/clipline-app/src/macos_capture.rs` with tests first and a minimal module shell. The tests should use in-memory bytes, not a real capture session:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_stream_header() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"CLNV");
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1920u32.to_le_bytes());
        bytes.extend_from_slice(&1080u32.to_le_bytes());
        bytes.extend_from_slice(&60u32.to_le_bytes());

        let info = read_stream_header(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(info.width, 1920);
        assert_eq!(info.height, 1080);
        assert_eq!(info.fps, 60);
    }

    #[test]
    fn parses_one_frame_with_pts_and_nv12_payload() {
        let payload = vec![7, 8, 9, 10, 11, 12];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FRAM");
        bytes.extend_from_slice(&12_500_000u64.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);

        let frame = read_frame(&mut Cursor::new(bytes), 4, 1).unwrap().unwrap();

        assert_eq!(frame.pts_s, 0.0125);
        match frame.data {
            FrameData::Cpu(data) => assert_eq!(data, payload),
        }
    }

    #[test]
    fn rejects_frame_payload_size_that_does_not_match_nv12_dimensions() {
        let payload = vec![1, 2, 3];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FRAM");
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);

        let err = read_frame(&mut Cursor::new(bytes), 4, 2).unwrap_err();

        assert!(err.to_string().contains("NV12 payload"));
    }
}
```

Add a static contract test:

```rust
#[test]
fn macos_capture_wrapper_uses_helper_resource_and_protocol_magic() {
    let main_rs = main_rs();
    let capture = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/macos_capture.rs"))
        .expect("read macos_capture.rs");
    assert!(main_rs.contains("#[cfg(target_os = \"macos\")]\nmod macos_capture;"));
    assert!(capture.contains("CLIPLINE_SCK_HELPER"));
    assert!(capture.contains("clipline-sck-helper"));
    assert!(capture.contains("b\"CLNV\""));
    assert!(capture.contains("b\"FRAM\""));
    assert!(capture.contains("impl CaptureEngine for ScreenCaptureKitCapture"));
}
```

- [ ] **Step 2: Run tests to verify RED**

Run:

```bash
cargo test -p clipline-app macos_capture::tests::parses_stream_header -- --exact
cargo test -p clipline-app --test macos_shell_contract macos_capture_wrapper_uses_helper_resource_and_protocol_magic -- --exact
```

Expected: fails because `macos_capture.rs` and module wiring do not exist.

- [ ] **Step 3: Implement Rust wrapper and parser**

Create the production module with this shape:

```rust
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};

use clipline_capture::{CaptureEngine, CaptureError, Frame, FrameData};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenCaptureKitConfig {
    pub fps: u32,
    pub max_height: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

pub struct ScreenCaptureKitCapture {
    child: Child,
    stdout: ChildStdout,
    info: StreamInfo,
}

impl ScreenCaptureKitCapture {
    pub fn new(config: ScreenCaptureKitConfig) -> Result<Self, CaptureError> {
        let helper = helper_path()?;
        Self::new_with_helper(helper, config)
    }

    fn new_with_helper(helper: PathBuf, config: ScreenCaptureKitConfig) -> Result<Self, CaptureError> {
        let mut cmd = Command::new(helper);
        cmd.arg("--fps").arg(config.fps.to_string());
        if let Some(max_height) = config.max_height {
            cmd.arg("--max-height").arg(max_height.to_string());
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| CaptureError::Init(format!("spawn ScreenCaptureKit helper: {e}")))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| CaptureError::Init("ScreenCaptureKit helper stdout missing".into()))?;
        let info = read_stream_header(&mut stdout)?;
        Ok(Self { child, stdout, info })
    }

    pub fn stream_info(&self) -> StreamInfo {
        self.info
    }
}

impl CaptureEngine for ScreenCaptureKitCapture {
    fn next_frame(&mut self) -> Result<Option<Frame>, CaptureError> {
        read_frame(&mut self.stdout, self.info.width, self.info.height)
    }
}

impl Drop for ScreenCaptureKitCapture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
```

Implement `helper_path`, `read_stream_header`, `read_frame`, `read_exact_or_eof`, and `nv12_len` in the same module:

```rust
const STREAM_MAGIC: &[u8; 4] = b"CLNV";
const FRAME_MAGIC: &[u8; 4] = b"FRAM";
const PROTOCOL_VERSION: u16 = 1;

fn helper_path() -> Result<PathBuf, CaptureError> {
    if let Some(path) = std::env::var_os("CLIPLINE_SCK_HELPER") {
        return Ok(PathBuf::from(path));
    }
    let exe = std::env::current_exe()
        .map_err(|e| CaptureError::Init(format!("locate current exe: {e}")))?;
    if let Some(contents) = exe.ancestors().find(|p| p.file_name().is_some_and(|n| n == "Contents")) {
        let bundled = contents.join("Resources/clipline-sck-helper");
        if bundled.exists() {
            return Ok(bundled);
        }
    }
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/clipline-sidecars/clipline-sck-helper"))
}

fn read_stream_header(mut r: impl Read) -> Result<StreamInfo, CaptureError> {
    let mut magic = [0; 4];
    r.read_exact(&mut magic).map_err(init_io("read ScreenCaptureKit stream magic"))?;
    if &magic != STREAM_MAGIC {
        return Err(CaptureError::Init("invalid ScreenCaptureKit stream magic".into()));
    }
    let version = read_u16(&mut r)?;
    if version != PROTOCOL_VERSION {
        return Err(CaptureError::Init(format!("unsupported ScreenCaptureKit protocol version {version}")));
    }
    let width = read_u32(&mut r)?;
    let height = read_u32(&mut r)?;
    let fps = read_u32(&mut r)?;
    if width < 2 || height < 2 || width % 2 != 0 || height % 2 != 0 {
        return Err(CaptureError::Init(format!("invalid ScreenCaptureKit dimensions {width}x{height}")));
    }
    Ok(StreamInfo { width, height, fps })
}
```

- [ ] **Step 4: Replace Swift skeleton with protocol-capable ScreenCaptureKit helper**

Keep `--version`. Add args `--fps <u32>` and `--max-height <u32>`. Update `build.rs` so the `xcrun swiftc` invocation links these frameworks:

```rust
for framework in [
    "Foundation",
    "ScreenCaptureKit",
    "CoreGraphics",
    "CoreMedia",
    "CoreVideo",
] {
    command.arg("-framework").arg(framework);
}
```

The helper should:

```swift
import CoreGraphics
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit

let streamMagic = [UInt8]("CLNV".utf8)
let frameMagic = [UInt8]("FRAM".utf8)
let protocolVersion: UInt16 = 1
```

Use `SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)`, select the display whose `displayID` equals `CGMainDisplayID()` with fallback to the first display, compute even output dimensions preserving aspect ratio, set:

```swift
configuration.width = outputWidth
configuration.height = outputHeight
configuration.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(fps))
configuration.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
configuration.queueDepth = 3
configuration.showsCursor = true
configuration.capturesAudio = false
configuration.colorMatrix = SCStreamConfiguration.ColorMatrix.ITU_R_709_2
```

In the stream output callback, lock the pixel buffer read-only and row-copy plane 0 followed by plane 1 into `width * height * 3 / 2` bytes. Write:

```swift
writeBytes(frameMagic)
writeLittleEndian(UInt64(max(0, CMTimeGetSeconds(CMSampleBufferGetPresentationTimeStamp(sampleBuffer)) * 1_000_000_000)))
writeLittleEndian(UInt32(payload.count))
FileHandle.standardOutput.write(Data(payload))
```

On failure, write a single-line diagnostic to stderr and exit non-zero. Do not print text to stdout except `--version`; stdout is binary after capture starts.

- [ ] **Step 5: Run focused GREEN tests and helper compile**

Run:

```bash
cargo test -p clipline-app macos_capture::tests -- --nocapture
cargo test -p clipline-app --test macos_shell_contract macos_capture_wrapper_uses_helper_resource_and_protocol_magic -- --exact
cargo build -p clipline-app
target/clipline-sidecars/clipline-sck-helper --version
```

Expected: tests pass, build succeeds, helper prints `clipline-sck-helper 1`.

- [ ] **Step 6: Commit**

```bash
git add apps/clipline-app/macos/ScreenCaptureKitHelper.swift apps/clipline-app/build.rs apps/clipline-app/src/macos_capture.rs apps/clipline-app/src/main.rs apps/clipline-app/tests/macos_shell_contract.rs
git commit -m "feat(app): add screencapturekit frame source"
```

---

### Task 4: Wire macOS Service To Record Replay Clips

**Files:**
- Modify: `apps/clipline-app/src/service_macos.rs`
- Modify: `apps/clipline-app/src/platform/macos.rs`
- Modify: `apps/clipline-app/tests/macos_shell_contract.rs`

**Interfaces:**
- Consumes: `ScreenCaptureKitCapture`, `ScreenCaptureKitConfig`, `StreamInfo`.
- Consumes: `EncoderBackend::VideoToolbox` and `FfmpegVideoEncoder::new`.
- Produces: `ensure_recording_available() -> Ok(())` when the helper path and VideoToolbox FFmpeg encoder are available.
- Produces: `spawn(opts)` starts a real `clipline-recorder` thread for `CaptureSource::PrimaryMonitor`.
- Produces: `Cmd::Save` writes a replay MP4 and emits `Event::Saved`.

- [ ] **Step 1: Write failing service/platform contract tests**

Replace the old stub expectations in `macos_shell_contract.rs` with:

```rust
#[test]
fn macos_service_wires_real_display_replay_recording() {
    let service =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service_macos.rs"))
            .expect("read service_macos.rs");

    for required in [
        "ScreenCaptureKitCapture::new",
        "FfmpegVideoEncoder::new",
        "EncoderBackend::VideoToolbox",
        "Recorder::new_with_replay_storage",
        "rec.step()",
        "save(&rec, &path, opts.replay_window_s)",
        "Event::Saved",
        "message: \"macOS window capture is not implemented in this slice\".into()",
        "message: \"macOS region capture is not implemented in this slice\".into()",
        "macOS full-session recording is not implemented in this slice",
    ] {
        assert!(
            service.contains(required),
            "missing macOS recording behavior: {required}"
        );
    }
    assert!(!service.contains("clipline-recorder-stub"));
    assert!(!service.contains("macOS recording is not implemented in Milestone 1"));
}

#[test]
fn macos_capabilities_advertise_display_capture_only() {
    let macos =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform/macos.rs"))
            .expect("read platform/macos.rs");

    assert!(macos.contains("display_capture: CapabilityStatus::available()"));
    assert!(macos.contains("hardware_encode: CapabilityStatus::available()"));
    assert!(macos.contains("window_capture: CapabilityStatus::unavailable("));
    assert!(macos.contains("display_region_capture: CapabilityStatus::unavailable("));
    assert!(macos.contains("system_audio: CapabilityStatus::unavailable("));
    assert!(macos.contains("microphone: CapabilityStatus::unavailable("));
}
```

Update `macos_recording_start_fails_before_spawning_stub_service` to assert the guard remains before spawn, but remove the requirement that `ensure_recording_available` always returns `Err`.

- [ ] **Step 2: Run tests to verify RED**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_service_wires_real_display_replay_recording -- --exact
cargo test -p clipline-app --test macos_shell_contract macos_capabilities_advertise_display_capture_only -- --exact
```

Expected: fail because the service is still a stub and capabilities still mark display capture unavailable.

- [ ] **Step 3: Implement macOS encoder selection**

In `service_macos.rs`, import:

```rust
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clipline_capture::ffmpeg;
use clipline_capture::ffmpeg_encoder::FfmpegVideoEncoder;
use clipline_capture::probe::{
    rank_encoders, EncoderApi, EncoderBackend, EncoderCandidate, EncoderCapability,
    EncoderPreference,
};
use clipline_capture::{CaptureEngine, Encoder, Recorder, ReplayStorageConfig};
use clipline_storage::{enforce_quota, storage_status, StorageStatus};
use clipline_storage::sessions::{session_label, SessionTracker};

use crate::macos_capture::{ScreenCaptureKitCapture, ScreenCaptureKitConfig};
```

Mirror `VideoEncoder::preference`, `id`, `from_parts`, `codec_id`, `encoder_label`, and `available_encoder_options` from Windows with the new `VideoToolboxH264` variant. `available_encoder_options` should filter `ffmpeg::probe()` to `EncoderBackend::VideoToolbox`.

Implement:

```rust
fn build_encoder(
    opts: &ServiceOptions,
    width: u32,
    height: u32,
    events: &Sender<Event>,
) -> Result<(FfmpegVideoEncoder, EncoderCandidate), String> {
    let preference = opts.video_encoder.preference();
    let caps = macos_encoder_capabilities();
    let candidates = rank_encoders(&caps, &opts.decodable_codecs, preference);
    if candidates.is_empty() {
        return Err("init: no usable macOS H.264 VideoToolbox encoder found".into());
    }
    let ffmpeg = ffmpeg::locate().ok_or_else(|| "init: ffmpeg not located".to_string())?;
    let mut last_err = String::new();
    for candidate in candidates {
        match FfmpegVideoEncoder::new(
            &ffmpeg,
            candidate.backend,
            candidate.codec,
            width,
            height,
            opts.fps,
            opts.bitrate_bps,
        ) {
            Ok(enc) => return Ok((enc, candidate)),
            Err(e) => last_err = e.to_string(),
        }
    }
    let _ = events.send(Event::Error {
        message: format!("macOS VideoToolbox encoder failed: {last_err}"),
    });
    Err(format!("init: macOS VideoToolbox encoder failed: {last_err}"))
}
```

- [ ] **Step 4: Implement replay-only service loop**

`spawn(opts)` should pass the full `ServiceOptions` into a thread named `clipline-recorder`. The thread calls `run(opts, cmd_rx, &event_tx)`; if `run` returns `Err`, emit `Event::Error` and a stopped status.

`run` should:

1. Reject unsupported source/mode/audio combinations with explicit strings:
   - `CaptureSource::WindowTitle(_) | CaptureSource::WindowHandle { .. }` -> `macOS window capture is not implemented in this slice`
   - `CaptureSource::DisplayRegion(_)` -> `macOS region capture is not implemented in this slice`
   - `RecordingMode::FullSession` -> `macOS full-session recording is not implemented in this slice`
   - any enabled audio option -> `macOS audio capture is not implemented in this slice`
2. Start `ScreenCaptureKitCapture::new(ScreenCaptureKitConfig { fps: opts.fps, max_height: max_height(opts.output_resolution) })`.
3. Read `stream_info()` for encoder dimensions.
4. Start `FfmpegVideoEncoder::new` through `build_encoder`.
5. Build `Recorder::new_with_replay_storage` using memory or disk options.
6. Resolve/create `clips_dir(&opts.media_dir)`.
7. Create `SessionTracker::new(local_session_label(false))`.
8. Send initial recording status.
9. Loop:
   - `rec.step()` once.
   - Every second emit status with `ring_len`, `buffered_span_s`, and `ring_bytes`.
   - Drain commands with `try_recv`.
   - On `Cmd::Save`, create the session folder, call `save(&rec, &path, opts.replay_window_s)`, and emit `Event::Saved`.
   - On `Cmd::Stop { announce }` or disconnect, emit stopped status when requested and return.

Use this save helper:

```rust
fn save(
    rec: &Recorder<impl CaptureEngine, impl Encoder>,
    path: &Path,
    window_s: f64,
) -> Result<(f64, f64), String> {
    let saved_from = rec
        .save_window_bounds(window_s, None)
        .map(|(start, _)| start);
    let file = std::fs::File::create(path).map_err(|e| format!("create {path:?}: {e}"))?;
    let (_, end) = rec
        .save_replay(file, window_s, None)
        .map_err(|e| format!("save: {e}"))?;
    Ok((end, end - saved_from.unwrap_or(end)))
}
```

Use quota reporting in `Event::Saved`:

```rust
fn emit_saved_clip(events: &Sender<Event>, clips_dir: &Path, path: &Path, seconds: f64, opts: &ServiceOptions) {
    let report = enforce_quota(clips_dir, opts.disk_quota_bytes, Some(path)).unwrap_or_else(|_| {
        let status = storage_status(clips_dir, opts.disk_quota_bytes).unwrap_or(StorageStatus {
            clip_count: 0,
            total_bytes: 0,
            quota_bytes: opts.disk_quota_bytes,
        });
        clipline_storage::GcReport {
            deleted_clips: 0,
            freed_bytes: 0,
            status,
        }
    });
    let _ = events.send(Event::Saved {
        path: path.display().to_string(),
        seconds,
        markers: 0,
        full_session: false,
        gc_deleted: report.deleted_clips,
        gc_freed_bytes: report.freed_bytes,
        storage_total_bytes: report.status.total_bytes,
        storage_quota_bytes: report.status.quota_bytes,
        storage_over_quota: report.status.is_over_quota(),
    });
}
```

- [ ] **Step 5: Implement availability and capabilities**

`ensure_recording_available` should return `Ok(())` when:

```rust
ffmpeg::locate().is_some()
    && ffmpeg::probe().iter().any(|cap| {
        cap.api == EncoderApi::Ffmpeg
            && cap.backend == EncoderBackend::VideoToolbox
            && cap.codecs.contains(&Codec::H264)
    })
```

If false, return `Err("macOS recording requires FFmpeg with h264_videotoolbox".into())`.

In `platform/macos.rs`, change:

```rust
display_capture: CapabilityStatus::available(),
hardware_encode: CapabilityStatus::available(),
```

Keep window, region, audio, per-process audio, focused-game hotkey fallback, and HDR unavailable.

- [ ] **Step 6: Run focused GREEN tests**

Run:

```bash
cargo test -p clipline-app --test macos_shell_contract macos_service_wires_real_display_replay_recording -- --exact
cargo test -p clipline-app --test macos_shell_contract macos_capabilities_advertise_display_capture_only -- --exact
cargo test -p clipline-app service::tests
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add apps/clipline-app/src/service_macos.rs apps/clipline-app/src/platform/macos.rs apps/clipline-app/tests/macos_shell_contract.rs
git commit -m "feat(app): record macos display replays"
```

---

### Task 5: Verify Bundle, Runtime, And Handoff

**Files:**
- Modify: `handoff.md`

**Interfaces:**
- Consumes: all previous tasks.
- Produces: verified local dev run, app bundle smoke, and updated handoff.

- [ ] **Step 1: Write failing handoff freshness check**

Before editing `handoff.md`, inspect it and identify the lines that still say macOS recording is unavailable. The expected stale strings are:

```text
macOS recording is not implemented
ScreenCaptureKit display capture is not implemented
```

Run:

```bash
rg "macOS recording is not implemented|ScreenCaptureKit display capture is not implemented" handoff.md
```

Expected: matching stale text appears before the update.

- [ ] **Step 2: Update handoff**

Update `handoff.md` to record:

- macOS primary-display video replay recording is implemented through ScreenCaptureKit helper + FFmpeg VideoToolbox H.264.
- macOS clip audio, window capture, region capture, and full-session recording are intentionally unsupported in this slice.
- Manual verification needs Screen Recording permission in System Settings if the helper reports a permission error.
- Next recommended work: audio capture or window/region capture, not both in the same slice.

- [ ] **Step 3: Run full verification**

Run:

```bash
cargo test -p clipline-capture
cargo test -p clipline-app --test macos_shell_contract
cargo test -p clipline-app
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all pass.

- [ ] **Step 4: Build and inspect bundle**

Run:

```bash
PATH="$PWD/target/tauri-cli/bin:$PATH" cargo tauri build --bundles app,dmg --no-sign --ci
test -x target/release/bundle/macos/Clipline.app/Contents/Resources/clipline-sck-helper
hdiutil verify target/release/bundle/dmg/Clipline_0.1.10_aarch64.dmg
```

Expected: app and DMG build, helper is executable in `Contents/Resources`, DMG verification passes.

- [ ] **Step 5: Run local recording smoke**

Run the app:

```bash
cargo run -p clipline-app
```

Manual smoke:

1. Start recording.
2. If macOS asks for Screen Recording permission, grant Clipline or the helper, quit, and start again.
3. Wait at least 3 seconds.
4. Trigger Save.
5. Verify a new MP4 exists under `$HOME/Movies/Clipline`.
6. Open the clip in the in-app review player.
7. Stop recording.

Expected: a video-only MP4 saves and plays; no audio tracks are expected.

- [ ] **Step 6: Commit**

```bash
git add handoff.md
git commit -m "docs: update macos recording handoff"
```
