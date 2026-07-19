use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 1_048_576;
static LOG: OnceLock<Mutex<Option<DiagnosticLogWriter>>> = OnceLock::new();

pub(super) fn diagnostic_log_path_from_appdata(appdata: &Path) -> PathBuf {
    appdata.join("Clipline").join("clipline.log")
}

pub(super) fn diagnostic_log_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|appdata| diagnostic_log_path_from_appdata(&appdata))
}

pub(super) struct DiagnosticLogWriter {
    path: PathBuf,
    file: Option<File>,
    bytes_written: u64,
    max_bytes: u64,
}

impl DiagnosticLogWriter {
    pub(super) fn open_at(path: PathBuf, max_bytes: u64) -> Result<Self, String> {
        if max_bytes < 2 {
            return Err("diagnostic log cap must leave room for content and a newline".into());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create log directory: {e}"))?;
        }
        rotate_diagnostic_log_if_needed(&path, max_bytes)?;
        let file = open_diagnostic_log_file(&path)?;
        let bytes_written = file
            .metadata()
            .map_err(|e| format!("read diagnostic log metadata {path:?}: {e}"))?
            .len();
        Ok(Self {
            path,
            file: Some(file),
            bytes_written,
            max_bytes,
        })
    }

    pub(super) fn write_line(&mut self, line: &str) -> Result<(), String> {
        let max_content_bytes = usize::try_from(self.max_bytes - 1).unwrap_or(usize::MAX);
        let line = truncate_utf8(line, max_content_bytes);
        let record_bytes = u64::try_from(line.len() + 1).unwrap_or(u64::MAX);
        if self.bytes_written.saturating_add(record_bytes) > self.max_bytes {
            self.rotate()?;
        }

        let Some(file) = self.file.as_mut() else {
            return Err("diagnostic log is not open".into());
        };
        if let Err(error) = writeln!(file, "{line}") {
            self.bytes_written = file
                .metadata()
                .map_or(self.bytes_written, |meta| meta.len());
            return Err(format!("write diagnostic log {:?}: {error}", self.path));
        }
        self.bytes_written = self.bytes_written.saturating_add(record_bytes);
        Ok(())
    }

    fn rotate(&mut self) -> Result<(), String> {
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
        let rotation_result = rotate_diagnostic_log(&self.path, self.max_bytes);
        let reopen_result = open_diagnostic_log_file(&self.path);
        match reopen_result {
            Ok(file) => {
                self.bytes_written = file.metadata().map_or(0, |meta| meta.len());
                self.file = Some(file);
            }
            Err(error) => return Err(error),
        }
        rotation_result
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn open_diagnostic_log() -> Result<DiagnosticLogWriter, String> {
    let path = diagnostic_log_path().ok_or_else(|| "APPDATA is not set".to_string())?;
    DiagnosticLogWriter::open_at(path, MAX_BYTES)
}

fn open_diagnostic_log_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open diagnostic log {path:?}: {e}"))
}

fn rotate_diagnostic_log_if_needed(path: &Path, max_bytes: u64) -> Result<(), String> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() < max_bytes {
        return Ok(());
    }
    rotate_diagnostic_log(path, max_bytes)
}

fn rotate_diagnostic_log(path: &Path, max_bytes: u64) -> Result<(), String> {
    let rotated = path.with_file_name("clipline.old.log");
    if let Err(e) = std::fs::remove_file(&rotated) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("remove old diagnostic log {rotated:?}: {e}"));
        }
    }
    std::fs::rename(path, &rotated).map_err(|e| format!("rotate diagnostic log: {e}"))?;
    if std::fs::metadata(&rotated).is_ok_and(|metadata| metadata.len() > max_bytes) {
        if let Err(error) = retain_file_tail(&rotated, max_bytes) {
            let _ = std::fs::remove_file(&rotated);
            return Err(error);
        }
    }
    Ok(())
}

fn retain_file_tail(path: &Path, max_bytes: u64) -> Result<(), String> {
    let mut source = File::open(path).map_err(|e| format!("open oversized log {path:?}: {e}"))?;
    let length = source
        .metadata()
        .map_err(|e| format!("read oversized log metadata {path:?}: {e}"))?
        .len();
    source
        .seek(SeekFrom::Start(length.saturating_sub(max_bytes)))
        .map_err(|e| format!("seek oversized log {path:?}: {e}"))?;
    let mut tail = Vec::with_capacity(usize::try_from(max_bytes).unwrap_or(usize::MAX));
    source
        .take(max_bytes)
        .read_to_end(&mut tail)
        .map_err(|e| format!("read oversized log tail {path:?}: {e}"))?;
    std::fs::write(path, tail).map_err(|e| format!("bound oversized log {path:?}: {e}"))
}

fn diagnostic_log() -> &'static Mutex<Option<DiagnosticLogWriter>> {
    LOG.get_or_init(|| Mutex::new(open_diagnostic_log().ok()))
}

pub(super) fn format_diagnostic_log_line(
    timestamp: chrono::DateTime<chrono::Utc>,
    pid: u32,
    message: &str,
) -> String {
    let message = message.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{} pid={pid} {message}",
        timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    )
}

pub(super) fn log_diagnostic(message: impl AsRef<str>) {
    let line = format_diagnostic_log_line(chrono::Utc::now(), std::process::id(), message.as_ref());
    if let Ok(mut log) = diagnostic_log().lock() {
        if let Some(log) = log.as_mut() {
            let _ = log.write_line(&line);
        }
    }
}
