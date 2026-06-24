#![allow(dead_code)]

use std::ffi::OsStr;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};

use clipline_capture::{CaptureEngine, CaptureError, Frame, FrameData};

const HELPER_ENV: &str = "CLIPLINE_SCK_HELPER";
const HELPER_NAME: &str = "clipline-sck-helper";
const STREAM_MAGIC: &[u8; 4] = b"CLNV";
const FRAME_MAGIC: &[u8; 4] = b"FRAM";
const PROTOCOL_VERSION: u16 = 1;

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

    fn new_with_helper(
        helper: PathBuf,
        config: ScreenCaptureKitConfig,
    ) -> Result<Self, CaptureError> {
        let mut cmd = Command::new(helper);
        cmd.arg("--fps").arg(config.fps.to_string());
        if let Some(max_height) = config.max_height {
            cmd.arg("--max-height").arg(max_height.to_string());
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| CaptureError::Init(format!("spawn ScreenCaptureKit helper: {e}")))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| CaptureError::Init("ScreenCaptureKit helper stdout missing".into()))?;
        let stderr = child.stderr.take();
        let info = match read_stream_header(&mut stdout) {
            Ok(info) => info,
            Err(err) => return Err(helper_startup_error(err, &mut child, stderr)),
        };
        Ok(Self {
            child,
            stdout,
            info,
        })
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

fn helper_path() -> Result<PathBuf, CaptureError> {
    if let Some(path) = std::env::var_os(HELPER_ENV) {
        return Ok(PathBuf::from(path));
    }
    let exe = std::env::current_exe()
        .map_err(|e| CaptureError::Init(format!("locate current exe: {e}")))?;
    if let Some(contents) = exe
        .ancestors()
        .find(|p| p.file_name().is_some_and(|n| n == OsStr::new("Contents")))
    {
        let bundled = contents.join("Resources").join(HELPER_NAME);
        if bundled.exists() {
            return Ok(bundled);
        }
    }
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/clipline-sidecars")
        .join(HELPER_NAME))
}

fn helper_startup_error(
    err: CaptureError,
    child: &mut Child,
    stderr: Option<ChildStderr>,
) -> CaptureError {
    let _ = child.kill();
    let status = child.wait().ok();
    let mut message = match err {
        CaptureError::Init(message) => message,
        other => other.to_string(),
    };
    if let Some(mut stderr) = stderr {
        let mut text = String::new();
        if stderr.read_to_string(&mut text).is_ok() {
            let text = text.trim();
            if !text.is_empty() {
                message.push_str("; helper stderr: ");
                message.push_str(text);
            }
        }
    }
    if let Some(status) = status {
        message.push_str("; helper exited with ");
        message.push_str(&status.to_string());
    }
    CaptureError::Init(message)
}

fn read_stream_header(mut r: impl Read) -> Result<StreamInfo, CaptureError> {
    let mut magic = [0; 4];
    r.read_exact(&mut magic)
        .map_err(init_io("read ScreenCaptureKit stream magic"))?;
    if &magic != STREAM_MAGIC {
        return Err(CaptureError::Init(
            "invalid ScreenCaptureKit stream magic".into(),
        ));
    }
    let version = read_u16(&mut r, init_io("read ScreenCaptureKit protocol version"))?;
    if version != PROTOCOL_VERSION {
        return Err(CaptureError::Init(format!(
            "unsupported ScreenCaptureKit protocol version {version}"
        )));
    }
    let width = read_u32(&mut r, init_io("read ScreenCaptureKit stream width"))?;
    let height = read_u32(&mut r, init_io("read ScreenCaptureKit stream height"))?;
    let fps = read_u32(&mut r, init_io("read ScreenCaptureKit stream fps"))?;
    if width < 2 || height < 2 || width % 2 != 0 || height % 2 != 0 {
        return Err(CaptureError::Init(format!(
            "invalid ScreenCaptureKit dimensions {width}x{height}"
        )));
    }
    if fps == 0 {
        return Err(CaptureError::Init("invalid ScreenCaptureKit fps 0".into()));
    }
    Ok(StreamInfo { width, height, fps })
}

fn read_frame(mut r: impl Read, width: u32, height: u32) -> Result<Option<Frame>, CaptureError> {
    let mut magic = [0; 4];
    if !read_exact_or_eof(&mut r, &mut magic)
        .map_err(frame_io("read ScreenCaptureKit frame magic"))?
    {
        return Ok(None);
    }
    if &magic != FRAME_MAGIC {
        return Err(CaptureError::DeviceLost(
            "invalid ScreenCaptureKit frame magic".into(),
        ));
    }
    let pts_ns = read_u64(&mut r, frame_io("read ScreenCaptureKit frame pts"))?;
    let len = read_u32(
        &mut r,
        frame_io("read ScreenCaptureKit frame payload length"),
    )? as usize;
    let expected = nv12_len(width, height)?;
    if len != expected {
        return Err(CaptureError::DeviceLost(format!(
            "ScreenCaptureKit NV12 payload length {len} did not match expected {expected}"
        )));
    }
    let mut data = vec![0; len];
    r.read_exact(&mut data)
        .map_err(frame_io("read ScreenCaptureKit frame payload"))?;
    Ok(Some(Frame {
        pts_s: pts_ns as f64 / 1_000_000_000.0,
        data: FrameData::Cpu(data),
    }))
}

fn read_exact_or_eof(mut r: impl Read, buf: &mut [u8]) -> io::Result<bool> {
    let mut read = 0;
    while read < buf.len() {
        match r.read(&mut buf[read..])? {
            0 if read == 0 => return Ok(false),
            0 => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "early EOF")),
            n => read += n,
        }
    }
    Ok(true)
}

fn read_u16(
    mut r: impl Read,
    map: impl FnOnce(io::Error) -> CaptureError,
) -> Result<u16, CaptureError> {
    let mut bytes = [0; 2];
    r.read_exact(&mut bytes).map_err(map)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(
    mut r: impl Read,
    map: impl FnOnce(io::Error) -> CaptureError,
) -> Result<u32, CaptureError> {
    let mut bytes = [0; 4];
    r.read_exact(&mut bytes).map_err(map)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(
    mut r: impl Read,
    map: impl FnOnce(io::Error) -> CaptureError,
) -> Result<u64, CaptureError> {
    let mut bytes = [0; 8];
    r.read_exact(&mut bytes).map_err(map)?;
    Ok(u64::from_le_bytes(bytes))
}

fn nv12_len(width: u32, height: u32) -> Result<usize, CaptureError> {
    let pixels = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| CaptureError::DeviceLost("ScreenCaptureKit dimensions overflow".into()))?;
    pixels
        .checked_mul(3)
        .map(|v| v / 2)
        .ok_or_else(|| CaptureError::DeviceLost("ScreenCaptureKit NV12 payload overflow".into()))
}

fn init_io(context: &'static str) -> impl FnOnce(io::Error) -> CaptureError {
    move |e| CaptureError::Init(format!("{context}: {e}"))
}

fn frame_io(context: &'static str) -> impl FnOnce(io::Error) -> CaptureError {
    move |e| CaptureError::DeviceLost(format!("{context}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipline_capture::FrameData;
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

    #[test]
    fn helper_startup_error_includes_stderr_when_header_is_missing() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "clipline-sck-helper-test-{}-{}",
            std::process::id(),
            unique_test_suffix()
        ));
        fs::create_dir_all(&dir).unwrap();
        let helper = dir.join("helper.sh");
        fs::write(
            &helper,
            "#!/bin/sh\necho 'ScreenCaptureKit permission denied' >&2\nexit 42\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&helper).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&helper, permissions).unwrap();

        let result = ScreenCaptureKitCapture::new_with_helper(
            helper,
            ScreenCaptureKitConfig {
                fps: 30,
                max_height: Some(720),
            },
        );
        let err = match result {
            Ok(_) => panic!("helper should fail before producing a stream header"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("permission denied"));
        assert!(!err
            .to_string()
            .contains("capture init failed: capture init failed"));
        let _ = fs::remove_dir_all(dir);
    }

    fn unique_test_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
