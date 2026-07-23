use std::backtrace::Backtrace;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use uuid::Uuid;

#[cfg(test)]
use std::io::{Read, Seek, SeekFrom};

const GENERATION_BYTES: u64 = 4 * 1024 * 1024;
const GENERATIONS: usize = 5;
const MAX_RECORD_BYTES: usize = 16 * 1024;
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const PANIC_BYTES: u64 = 512 * 1024;
const MAX_PANIC_RECORD_BYTES: usize = 128 * 1024;
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(15);
const QUEUE_LINES: usize = 2_048;

static DIAGNOSTICS: OnceLock<DiagnosticsHandle> = OnceLock::new();
static PANIC_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
static PANIC_HOOK: Once = Once::new();
static PANIC_LOCK: Mutex<()> = Mutex::new(());

pub(super) struct DiagnosticsGuard {
    sender: SyncSender<WriterCommand>,
    worker: Option<JoinHandle<()>>,
}

#[derive(Clone)]
struct DiagnosticsHandle {
    sender: SyncSender<WriterCommand>,
    dropped: Arc<AtomicUsize>,
    write_errors: Arc<AtomicUsize>,
    directory: PathBuf,
    active_path: PathBuf,
}

enum WriterCommand {
    Record(Vec<u8>),
    Snapshot {
        destination: PathBuf,
        result: mpsc::Sender<Result<Vec<PathBuf>, String>>,
    },
    Shutdown,
}

#[derive(Clone)]
struct DiagnosticMakeWriter {
    sender: SyncSender<WriterCommand>,
    dropped: Arc<AtomicUsize>,
}

struct EventBuffer {
    sender: SyncSender<WriterCommand>,
    dropped: Arc<AtomicUsize>,
    bytes: Vec<u8>,
    sent: bool,
}

pub(super) struct RollingFileWriter {
    directory: PathBuf,
    active_path: PathBuf,
    file: File,
    bytes_written: u64,
    generation_bytes: u64,
    generations: usize,
}

impl RollingFileWriter {
    fn open(directory: PathBuf, generation_bytes: u64, generations: usize) -> io::Result<Self> {
        if generation_bytes < 2 || generations == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "diagnostic rotation limits are invalid",
            ));
        }
        std::fs::create_dir_all(&directory)?;
        prune_old_files(&directory, SystemTime::now(), MAX_AGE)?;
        let active_path = directory.join("clipline.jsonl");
        if std::fs::metadata(&active_path).is_ok_and(|metadata| {
            metadata.len() >= generation_bytes
                || metadata
                    .modified()
                    .ok()
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .is_some_and(|age| age > MAX_AGE)
        }) {
            rotate_generations(&directory, generations)?;
        }
        let file = open_append(&active_path)?;
        let bytes_written = file.metadata()?.len();
        Ok(Self {
            directory,
            active_path,
            file,
            bytes_written,
            generation_bytes,
            generations,
        })
    }

    fn write_record(&mut self, record: &[u8]) -> io::Result<()> {
        let record = bound_record(record);
        let record_bytes = u64::try_from(record.len() + 1).unwrap_or(u64::MAX);
        if self.bytes_written.saturating_add(record_bytes) > self.generation_bytes {
            self.rotate()?;
        }
        self.file.write_all(&record)?;
        self.file.write_all(b"\n")?;
        self.bytes_written = self.bytes_written.saturating_add(record_bytes);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        rotate_generations(&self.directory, self.generations)?;
        self.file = open_append(&self.active_path)?;
        self.bytes_written = self.file.metadata()?.len();
        Ok(())
    }

    fn snapshot(&mut self, destination: &Path) -> Result<Vec<PathBuf>, String> {
        self.file
            .flush()
            .map_err(|error| format!("flush diagnostic log: {error}"))?;
        std::fs::create_dir_all(destination)
            .map_err(|error| format!("create diagnostic snapshot directory: {error}"))?;
        let mut copied = Vec::new();
        for source in diagnostic_files(&self.directory) {
            if !source.is_file() {
                continue;
            }
            let Some(name) = source.file_name() else {
                continue;
            };
            let target = destination.join(name);
            std::fs::copy(&source, &target)
                .map_err(|error| format!("snapshot diagnostic log {source:?}: {error}"))?;
            copied.push(target);
        }

        if let Some(parent) = self.directory.parent() {
            for name in ["clipline.log", "clipline.old.log"] {
                let source = parent.join(name);
                if !legacy_log_is_recent(&source) {
                    continue;
                }
                let target = destination.join(name);
                std::fs::copy(&source, &target)
                    .map_err(|error| format!("snapshot legacy diagnostic {source:?}: {error}"))?;
                copied.push(target);
            }
        }
        Ok(copied)
    }
}

impl Write for EventBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let remaining = (MAX_RECORD_BYTES + 1).saturating_sub(self.bytes.len());
        self.bytes
            .extend_from_slice(&buffer[..buffer.len().min(remaining)]);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.send();
        Ok(())
    }
}

impl EventBuffer {
    fn send(&mut self) {
        if self.sent || self.bytes.is_empty() {
            return;
        }
        self.sent = true;
        match self
            .sender
            .try_send(WriterCommand::Record(std::mem::take(&mut self.bytes)))
        {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl Drop for EventBuffer {
    fn drop(&mut self) {
        self.send();
    }
}

impl<'writer> MakeWriter<'writer> for DiagnosticMakeWriter {
    type Writer = EventBuffer;

    fn make_writer(&'writer self) -> Self::Writer {
        EventBuffer {
            sender: self.sender.clone(),
            dropped: self.dropped.clone(),
            bytes: Vec::with_capacity(1024),
            sent: false,
        }
    }
}

impl Drop for DiagnosticsGuard {
    fn drop(&mut self) {
        let _ = self.sender.send(WriterCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) fn init() -> Result<DiagnosticsGuard, String> {
    let directory = choose_diagnostics_directory()?;
    let _ = PANIC_DIRECTORY.set(directory.clone());
    install_panic_hook();
    let rolling = RollingFileWriter::open(directory.clone(), GENERATION_BYTES, GENERATIONS)
        .map_err(|error| format!("open structured diagnostic log: {error}"))?;
    let active_path = rolling.active_path.clone();
    let session_id = Uuid::new_v4();
    let pid = std::process::id();
    let (sender, receiver) = mpsc::sync_channel(QUEUE_LINES);
    let dropped = Arc::new(AtomicUsize::new(0));
    let write_errors = Arc::new(AtomicUsize::new(0));
    let worker_write_errors = write_errors.clone();
    let worker = std::thread::Builder::new()
        .name("clipline-diagnostics".into())
        .spawn(move || {
            writer_thread(
                receiver,
                rolling,
                session_id,
                pid,
                &worker_write_errors,
            );
        })
        .map_err(|error| format!("start diagnostic writer thread: {error}"))?;
    let guard = DiagnosticsGuard {
        sender: sender.clone(),
        worker: Some(worker),
    };
    let writer = DiagnosticMakeWriter {
        sender: sender.clone(),
        dropped: dropped.clone(),
    };
    let filter = Targets::new()
        .with_default(LevelFilter::WARN)
        .with_target("clipline_app", LevelFilter::DEBUG)
        .with_target("clipline_capture", LevelFilter::DEBUG)
        .with_target("clipline_lol", LevelFilter::DEBUG)
        .with_target("clipline_storage", LevelFilter::DEBUG)
        .with_target("clipline_mp4", LevelFilter::DEBUG);
    let layer = tracing_subscriber::fmt::layer()
        .json()
        .flatten_event(true)
        .with_target(true)
        .with_thread_ids(true)
        .with_thread_names(true)
        .with_current_span(true)
        .with_span_list(true)
        .with_span_events(FmtSpan::CLOSE)
        .with_writer(writer.clone());
    tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init()
        .map_err(|error| format!("install diagnostic subscriber: {error}"))?;
    DIAGNOSTICS
        .set(DiagnosticsHandle {
            sender,
            dropped,
            write_errors,
            directory,
            active_path,
        })
        .map_err(|_| "diagnostics are already initialized".to_string())?;
    tracing::info!(
        event = "diagnostics_initialized",
        session_id = %session_id,
        generation_bytes = GENERATION_BYTES,
        generations = GENERATIONS,
        queue_lines = QUEUE_LINES
    );
    Ok(guard)
}

fn writer_thread(
    receiver: Receiver<WriterCommand>,
    mut rolling: RollingFileWriter,
    session_id: Uuid,
    pid: u32,
    write_errors: &AtomicUsize,
) {
    while let Ok(command) = receiver.recv() {
        match command {
            WriterCommand::Record(buffer) => {
                let record = structured_record(&buffer, session_id, pid);
                if rolling.write_record(&record).is_err() {
                    write_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            WriterCommand::Snapshot {
                destination,
                result,
            } => {
                let snapshot = rolling.snapshot(&destination);
                if snapshot.is_err() {
                    write_errors.fetch_add(1, Ordering::Relaxed);
                }
                let _ = result.send(snapshot);
            }
            WriterCommand::Shutdown => {
                let _ = rolling.file.flush();
                break;
            }
        }
    }
}

pub(super) fn diagnostics_directory() -> Option<PathBuf> {
    DIAGNOSTICS.get().map(|handle| handle.directory.clone())
}

pub(super) fn diagnostic_log_path() -> Option<PathBuf> {
    DIAGNOSTICS.get().map(|handle| handle.active_path.clone())
}

pub(super) fn dropped_lines() -> usize {
    DIAGNOSTICS
        .get()
        .map_or(0, |handle| handle.dropped.load(Ordering::Relaxed))
}

pub(super) fn write_errors() -> usize {
    DIAGNOSTICS
        .get()
        .map_or(0, |handle| handle.write_errors.load(Ordering::Relaxed))
}

pub(super) fn snapshot_to(destination: &Path) -> Result<Vec<PathBuf>, String> {
    let handle = DIAGNOSTICS
        .get()
        .ok_or_else(|| "diagnostics are not initialized".to_string())?;
    let dropped_lines = dropped_lines();
    tracing::info!(
        event = "diagnostics_snapshot_requested",
        dropped_lines
    );
    let (result_tx, result_rx) = mpsc::channel();
    handle
        .sender
        .send(WriterCommand::Snapshot {
            destination: destination.to_path_buf(),
            result: result_tx,
        })
        .map_err(|_| "diagnostic writer stopped before snapshot".to_string())?;
    result_rx
        .recv_timeout(SNAPSHOT_TIMEOUT)
        .map_err(|_| "timed out waiting for diagnostic snapshot barrier".to_string())?
}

pub(super) fn log_diagnostic(message: impl AsRef<str>) {
    tracing::debug!(
        event = "legacy_diagnostic",
        message = %single_line(message.as_ref())
    );
}

fn choose_diagnostics_directory() -> Result<PathBuf, String> {
    let preferred = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("Clipline").join("logs"));
    if let Some(path) = preferred {
        if std::fs::create_dir_all(&path).is_ok() {
            return Ok(path);
        }
    }
    let fallback = std::env::temp_dir().join("Clipline").join("logs");
    std::fs::create_dir_all(&fallback)
        .map_err(|error| format!("create fallback diagnostic directory {fallback:?}: {error}"))?;
    Ok(fallback)
}

fn structured_record(buffer: &[u8], session_id: Uuid, pid: u32) -> Vec<u8> {
    let text = String::from_utf8_lossy(buffer);
    let mut value = serde_json::from_str::<serde_json::Value>(text.trim()).unwrap_or_else(|_| {
        serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "level": "WARN",
            "target": "clipline_app::diagnostics",
            "event": "unparseable_diagnostic",
            "message": single_line(&text),
        })
    });
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "session_id".into(),
            serde_json::Value::String(session_id.to_string()),
        );
        object.insert("pid".into(), serde_json::Value::Number(pid.into()));
        object
            .entry("event")
            .or_insert_with(|| serde_json::Value::String("diagnostic".into()));
        let severity = object
            .get("level")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::String("WARN".into()));
        object.entry("severity").or_insert(severity);
        object
            .entry("outcome")
            .or_insert_with(|| serde_json::Value::String("observed".into()));
        object
            .entry("duration_ms")
            .or_insert(serde_json::Value::Null);
    }
    let mut record = serde_json::to_vec(&value).unwrap_or_else(|_| {
        br#"{"level":"ERROR","event":"diagnostic_serialization_failed"}"#.to_vec()
    });
    if record.len() > MAX_RECORD_BYTES {
        if let Some(object) = value.as_object_mut() {
            object.remove("stack");
            object.remove("spans");
            object.insert(
                "message".into(),
                serde_json::Value::String("<diagnostic record truncated>".into()),
            );
            object.insert("record_truncated".into(), serde_json::Value::Bool(true));
        }
        record = serde_json::to_vec(&value).unwrap_or_else(|_| {
            br#"{"level":"ERROR","event":"diagnostic_serialization_failed"}"#.to_vec()
        });
    }
    bound_record(&record)
}

fn bound_record(record: &[u8]) -> Vec<u8> {
    if record.len() <= MAX_RECORD_BYTES {
        return record.to_vec();
    }
    br#"{"level":"WARN","event":"diagnostic_record_too_large","record_truncated":true}"#.to_vec()
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn diagnostic_files(directory: &Path) -> Vec<PathBuf> {
    std::iter::once(directory.join("clipline.jsonl"))
        .chain((1..GENERATIONS).map(|index| directory.join(format!("clipline.{index}.jsonl"))))
        .chain(std::iter::once(directory.join("panic.log")))
        .chain(std::iter::once(directory.join("panic.old.log")))
        .collect()
}

fn generation_path(directory: &Path, index: usize) -> PathBuf {
    if index == 0 {
        directory.join("clipline.jsonl")
    } else {
        directory.join(format!("clipline.{index}.jsonl"))
    }
}

fn rotate_generations(directory: &Path, generations: usize) -> io::Result<()> {
    let oldest = generation_path(directory, generations - 1);
    if let Err(error) = std::fs::remove_file(&oldest) {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error);
        }
    }
    for index in (1..generations).rev() {
        let source = generation_path(directory, index - 1);
        let target = generation_path(directory, index);
        match std::fs::rename(source, target) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn prune_old_files(directory: &Path, now: SystemTime, max_age: Duration) -> io::Result<()> {
    for path in diagnostic_files(directory) {
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        let is_old = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > max_age);
        if is_old {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn legacy_log_is_recent(path: &Path) -> bool {
    path.metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age <= MAX_AGE)
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            write_panic_record(info);
            previous(info);
        }));
    });
}

fn write_panic_record(info: &std::panic::PanicHookInfo<'_>) {
    let Ok(_guard) = PANIC_LOCK.try_lock() else {
        return;
    };
    let Some(directory) = diagnostics_directory().or_else(|| PANIC_DIRECTORY.get().cloned()) else {
        return;
    };
    let path = directory.join("panic.log");
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>");
    let location = info
        .location()
        .map_or_else(|| "<unknown>".to_string(), ToString::to_string);
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>");
    let mut record = format!(
        "{} version={} pid={} thread={thread_name:?} location={location:?} payload={:?}\n{}\n",
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        single_line(payload),
        Backtrace::force_capture()
    );
    if record.len() > MAX_PANIC_RECORD_BYTES {
        record.truncate(floor_char_boundary(&record, MAX_PANIC_RECORD_BYTES - 32));
        record.push_str("\n<panic record truncated>\n");
    }
    let rotate = std::fs::metadata(&path).is_ok_and(|metadata| {
        metadata
            .len()
            .saturating_add(u64::try_from(record.len()).unwrap_or(u64::MAX))
            > PANIC_BYTES
    });
    if rotate {
        let old = directory.join("panic.old.log");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&path, old);
    }
    if let Ok(mut file) = open_append(&path) {
        let _ = file.write_all(record.as_bytes());
        let _ = file.flush();
    }
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
fn retain_file_tail(path: &Path, max_bytes: u64) -> io::Result<()> {
    let mut source = File::open(path)?;
    let length = source.metadata()?.len();
    source.seek(SeekFrom::Start(length.saturating_sub(max_bytes)))?;
    let mut tail = Vec::with_capacity(usize::try_from(max_bytes).unwrap_or(usize::MAX));
    source.take(max_bytes).read_to_end(&mut tail)?;
    while !tail.is_empty() && std::str::from_utf8(&tail).is_err() {
        tail.remove(0);
    }
    std::fs::write(path, tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipline_test_utils::TestDir;

    #[test]
    fn rolling_writer_bounds_every_generation_and_keeps_newest_records() {
        let directory = TestDir::new("clipline-app", "structured-log-rotation");
        let mut writer = RollingFileWriter::open(directory.path().to_path_buf(), 128, 3).unwrap();
        for index in 0..20 {
            writer
                .write_record(format!(r#"{{"event":"record","index":{index}}}"#).as_bytes())
                .unwrap();
        }
        writer.file.flush().unwrap();
        for index in 0..3 {
            let path = generation_path(directory.path(), index);
            if path.exists() {
                assert!(path.metadata().unwrap().len() <= 128);
            }
        }
        let active = std::fs::read_to_string(generation_path(directory.path(), 0)).unwrap();
        assert!(active.contains(r#""index":19"#));
    }

    #[test]
    fn oversized_record_becomes_valid_bounded_json() {
        let record = bound_record(&vec![b'x'; MAX_RECORD_BYTES + 1]);
        assert!(record.len() <= MAX_RECORD_BYTES);
        let parsed: serde_json::Value = serde_json::from_slice(&record).unwrap();
        assert_eq!(parsed["record_truncated"], true);
    }

    #[test]
    fn structured_record_adds_process_and_session_identity() {
        let session = Uuid::nil();
        let record = structured_record(
            br#"{"timestamp":"now","level":"INFO","event":"test"}"#,
            session,
            42,
        );
        let parsed: serde_json::Value = serde_json::from_slice(&record).unwrap();
        assert_eq!(parsed["session_id"], session.to_string());
        assert_eq!(parsed["pid"], 42);
        assert_eq!(parsed["severity"], "INFO");
        assert_eq!(parsed["outcome"], "observed");
        assert!(parsed["duration_ms"].is_null());
    }

    #[test]
    fn lossy_queue_overflow_is_counted_without_blocking() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicUsize::new(0));
        let writer = DiagnosticMakeWriter {
            sender,
            dropped: dropped.clone(),
        };
        {
            let mut first = writer.make_writer();
            first.write_all(br#"{"event":"first"}"#).unwrap();
        }
        {
            let mut second = writer.make_writer();
            second.write_all(br#"{"event":"second"}"#).unwrap();
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn snapshot_barrier_excludes_records_queued_after_it() {
        let source = TestDir::new("clipline-app", "structured-log-barrier-source");
        let target = TestDir::new("clipline-app", "structured-log-barrier-target");
        let rolling =
            RollingFileWriter::open(source.path().to_path_buf(), 4096, GENERATIONS).unwrap();
        let (sender, receiver) = mpsc::sync_channel(8);
        let write_errors = Arc::new(AtomicUsize::new(0));
        let worker_errors = write_errors.clone();
        let worker = std::thread::spawn(move || {
            writer_thread(receiver, rolling, Uuid::nil(), 42, &worker_errors);
        });
        sender
            .send(WriterCommand::Record(
                br#"{"level":"INFO","event":"before_barrier"}"#.to_vec(),
            ))
            .unwrap();
        let (result_tx, result_rx) = mpsc::channel();
        sender
            .send(WriterCommand::Snapshot {
                destination: target.path().to_path_buf(),
                result: result_tx,
            })
            .unwrap();
        sender
            .send(WriterCommand::Record(
                br#"{"level":"INFO","event":"after_barrier"}"#.to_vec(),
            ))
            .unwrap();
        result_rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        sender.send(WriterCommand::Shutdown).unwrap();
        worker.join().unwrap();
        assert_eq!(write_errors.load(Ordering::Relaxed), 0);

        let snapshot = std::fs::read_to_string(target.path().join("clipline.jsonl")).unwrap();
        assert!(snapshot.contains("before_barrier"));
        assert!(!snapshot.contains("after_barrier"));
        let live = std::fs::read_to_string(source.path().join("clipline.jsonl")).unwrap();
        assert!(live.contains("after_barrier"));
    }

    #[test]
    fn snapshot_copies_only_known_diagnostic_files() {
        let source = TestDir::new("clipline-app", "structured-log-source");
        let target = TestDir::new("clipline-app", "structured-log-snapshot");
        let mut writer = RollingFileWriter::open(source.path().to_path_buf(), 512, 3).unwrap();
        writer.write_record(br#"{"event":"kept"}"#).unwrap();
        std::fs::write(source.path().join("not-a-log.txt"), "private").unwrap();
        let copied = writer.snapshot(target.path()).unwrap();
        assert_eq!(copied.len(), 1);
        assert!(target.path().join("clipline.jsonl").is_file());
        assert!(!target.path().join("not-a-log.txt").exists());
    }

    #[test]
    fn tail_retention_helper_keeps_valid_utf8_boundary() {
        let directory = TestDir::new("clipline-app", "structured-log-tail");
        let path = directory.path().join("legacy.log");
        std::fs::write(&path, "αβγδε").unwrap();
        retain_file_tail(&path, 5).unwrap();
        assert!(std::fs::read_to_string(path).is_ok());
    }
}
