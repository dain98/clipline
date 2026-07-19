use std::io::{self, Seek, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use clipline_buffer::{DiskReplayRing, ReplayRing, SampleInfo, Segment, TrackSamples};
use clipline_mp4::{FragSampleRef, HybridMp4Writer, TrackConfig};

use crate::traits::{
    AudioPacket, AudioSource, CaptureEngine, CaptureError, EncodeError, EncodedPacket, Encoder,
};

const MAX_PENDING_GOP_BYTES: usize = 64 * 1024 * 1024;
/// Normal replay GOPs are about 500 ms. This generous ceiling prevents a
/// broken encoder from retaining an arbitrarily long video/audio segment.
const MAX_PENDING_GOP_DURATION_S: f64 = 10.0;
const FULL_SESSION_QUEUE_MAX_BYTES: usize = 128 * 1024 * 1024;
const FULL_SESSION_QUEUE_MAX_SEGMENTS: usize = 8;
const MID_STREAM_REPLAY_OPUS_PRE_SKIP: u16 = 960; // One 20 ms Opus frame at 48 kHz.

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Encode(#[from] EncodeError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug)]
pub enum ReplayStorageConfig {
    Memory { max_bytes: usize },
    Disk { max_bytes: usize, dir: PathBuf },
}

enum ReplayStorage {
    Memory(ReplayRing),
    Disk(DiskReplayRing),
}

pub trait WriteSeek: Write + Seek + Send {}

impl<T: Write + Seek + Send> WriteSeek for T {}

#[derive(Debug)]
pub struct FullSessionSummary {
    pub start_s: f64,
    pub end_s: f64,
    pub duration_s: f64,
}

struct FullSessionSink {
    tx: SyncSender<FullSessionWriteMsg>,
    join: JoinHandle<()>,
    queued_bytes: Arc<AtomicUsize>,
    max_queue_bytes: usize,
    audio_cfgs: Vec<clipline_mp4::AudioTrackConfig>,
    video_cfg: Option<clipline_mp4::VideoTrackConfig>,
    start_s: Option<f64>,
    end_s: Option<f64>,
    send_error: Option<String>,
}

struct FullSessionSegment {
    video_cfg: clipline_mp4::VideoTrackConfig,
    audio_cfgs: Vec<clipline_mp4::AudioTrackConfig>,
    segment: Arc<Segment>,
    reserved_bytes: usize,
}

enum FullSessionWriteMsg {
    Segment(FullSessionSegment),
    Finish(Sender<io::Result<()>>),
}

/// The recording pipeline (ddoc §3): capture → encode → GOP-aligned
/// segments → replay ring. Synchronous pull loop; production runs it on a
/// dedicated thread.
pub struct Recorder<C: CaptureEngine, E: Encoder> {
    capture: C,
    encoder: E,
    ring: ReplayStorage,
    pending: Vec<EncodedPacket>,
    /// Encoded video payload held for the current unsealed GOP.
    pending_bytes: usize,
    /// Encoded audio payload held across all tracks for the current GOP.
    pending_audio_bytes: usize,
    pending_byte_budget: usize,
    pre_keyframe_bytes: usize,
    pending_started_pts_s: Option<f64>,
    audio_sources: Vec<Box<dyn AudioSource>>,
    pending_audio: Vec<Vec<AudioPacket>>,
    /// pts of the first video packet — the recording's timeline start.
    /// Audio captured before it (engine-init lead-in) is dropped so both
    /// tracks begin together in the file.
    video_start_pts_s: Option<f64>,
    full_session: Option<FullSessionSink>,
}

impl<C: CaptureEngine, E: Encoder> Recorder<C, E> {
    pub fn new(capture: C, encoder: E, max_buffer_bytes: usize) -> Self {
        Self {
            capture,
            encoder,
            ring: ReplayStorage::Memory(ReplayRing::new(max_buffer_bytes)),
            pending: Vec::new(),
            pending_bytes: 0,
            pending_audio_bytes: 0,
            pending_byte_budget: pending_byte_budget(max_buffer_bytes),
            pre_keyframe_bytes: 0,
            pending_started_pts_s: None,
            audio_sources: Vec::new(),
            pending_audio: Vec::new(),
            video_start_pts_s: None,
            full_session: None,
        }
    }

    pub fn new_with_replay_storage(
        capture: C,
        encoder: E,
        storage: ReplayStorageConfig,
    ) -> io::Result<Self> {
        let storage_max_bytes = match &storage {
            ReplayStorageConfig::Memory { max_bytes } => *max_bytes,
            ReplayStorageConfig::Disk { max_bytes, .. } => *max_bytes,
        };
        let ring = match storage {
            ReplayStorageConfig::Memory { max_bytes } => {
                ReplayStorage::Memory(ReplayRing::new(max_bytes))
            }
            ReplayStorageConfig::Disk { max_bytes, dir } => {
                ReplayStorage::Disk(DiskReplayRing::new(max_bytes, dir)?)
            }
        };
        Ok(Self {
            capture,
            encoder,
            ring,
            pending: Vec::new(),
            pending_bytes: 0,
            pending_audio_bytes: 0,
            pending_byte_budget: pending_byte_budget(storage_max_bytes),
            pre_keyframe_bytes: 0,
            pending_started_pts_s: None,
            audio_sources: Vec::new(),
            pending_audio: Vec::new(),
            video_start_pts_s: None,
            full_session: None,
        })
    }

    /// Attach an audio source as the next audio track (ddoc §10:
    /// game / mic / system).
    pub fn with_audio(mut self, source: Box<dyn AudioSource>) -> Self {
        self.audio_sources.push(source);
        self.pending_audio.push(Vec::new());
        self
    }

    /// Process one captured frame (audio drain → encode → GOP sealing).
    /// `Ok(false)` = the capture source ended. Errors pass through —
    /// callers running live decide how to treat `CaptureError::Timeout`
    /// (an idle screen delivers no frames; that is not fatal).
    pub fn step(&mut self) -> Result<bool, PipelineError> {
        self.step_with_frame(|_| {})
    }

    /// Process one captured frame and expose it before encoding. This keeps
    /// side-channel observers (like the app's low-rate preview) out of the
    /// core capture/encode path when they are not installed.
    pub fn step_with_frame(
        &mut self,
        mut observe: impl FnMut(&crate::traits::Frame),
    ) -> Result<bool, PipelineError> {
        let Some(frame) = self.capture.next_frame()? else {
            return Ok(false);
        };
        observe(&frame);
        self.poll_audio_until(frame.pts_s)?;
        for pkt in self.encoder.encode(&frame)? {
            self.push_encoded_packet(pkt)?;
        }
        self.validate_pending_limits(frame.pts_s)?;
        Ok(true)
    }

    /// End of stream: drain the encoder, drain audio to the final GOP's
    /// end, seal the trailing partial GOP.
    pub fn finish_stream(&mut self) -> Result<(), PipelineError> {
        for pkt in self.encoder.finish()? {
            self.push_encoded_packet(pkt)?;
        }
        if self.pending.is_empty()
            && self.video_start_pts_s.is_none()
            && self.pending_payload_bytes() > 0
        {
            return Err(EncodeError::Backend(format!(
                "encoder ended before producing an initial keyframe ({} bytes were dropped before the first keyframe)",
                self.pending_payload_bytes()
            ))
            .into());
        }
        if !self.pending.is_empty() {
            let end = self
                .pending
                .last()
                .map(|p| p.pts_s + p.duration_s)
                .unwrap_or(0.0);
            self.poll_audio_until(end)?;
            self.validate_pending_limits(end)?;
            self.seal_pending(f64::INFINITY)?;
        }
        Ok(())
    }

    /// Drive the loop until the capture source ends, sealing a segment at
    /// every GOP boundary (a keyframe closes the previous GOP). Audio
    /// sources are drained per frame; packets ride in the segment whose
    /// GOP interval contains them.
    pub fn run_to_end(&mut self) -> Result<(), PipelineError> {
        while self.step()? {}
        self.finish_stream()
    }

    pub fn ring(&self) -> Option<&ReplayRing> {
        match &self.ring {
            ReplayStorage::Memory(ring) => Some(ring),
            ReplayStorage::Disk(_) => None,
        }
    }

    pub fn ring_len(&self) -> usize {
        match &self.ring {
            ReplayStorage::Memory(ring) => ring.len(),
            ReplayStorage::Disk(ring) => ring.len(),
        }
    }

    pub fn ring_bytes(&self) -> usize {
        match &self.ring {
            ReplayStorage::Memory(ring) => ring.bytes(),
            ReplayStorage::Disk(ring) => ring.bytes(),
        }
    }

    pub fn buffered_span_s(&self) -> f64 {
        let mut span = 0.0f64;
        let mut first_pts = None::<f64>;
        match &self.ring {
            ReplayStorage::Memory(ring) => {
                for seg in ring.segments() {
                    first_pts.get_or_insert(seg.pts_start_s);
                    span = seg.pts_end_s() - first_pts.unwrap();
                }
            }
            ReplayStorage::Disk(ring) => {
                for seg in ring.segments() {
                    first_pts.get_or_insert(seg.pts_start_s);
                    span = seg.pts_end_s() - first_pts.unwrap();
                }
            }
        }
        span
    }

    pub fn save_window_bounds(
        &self,
        window_s: f64,
        exclude_before_s: Option<f64>,
    ) -> Option<(f64, f64)> {
        match &self.ring {
            ReplayStorage::Memory(ring) => {
                let segs = ring.save_window(window_s, exclude_before_s);
                Some((segs.first()?.pts_start_s, segs.last()?.pts_end_s()))
            }
            ReplayStorage::Disk(ring) => {
                let segs = ring.save_window(window_s, exclude_before_s);
                Some((segs.first()?.pts_start_s, segs.last()?.pts_end_s()))
            }
        }
    }

    pub fn encoder(&self) -> &E {
        &self.encoder
    }

    pub fn start_full_session<W: Write + Seek + Send + 'static>(&mut self, w: W) -> io::Result<()> {
        self.start_full_session_with_limits(
            w,
            FULL_SESSION_QUEUE_MAX_BYTES,
            FULL_SESSION_QUEUE_MAX_SEGMENTS,
        )
    }

    fn start_full_session_with_limits<W: Write + Seek + Send + 'static>(
        &mut self,
        w: W,
        max_queue_bytes: usize,
        max_queue_segments: usize,
    ) -> io::Result<()> {
        if self.full_session.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "full session already recording",
            ));
        }
        let audio_cfgs: Vec<_> = self
            .audio_sources
            .iter()
            .map(|source| source.track_config())
            .collect();
        if max_queue_bytes == 0 || max_queue_segments == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "full session queue limits must be non-zero",
            ));
        }
        let (tx, join, queued_bytes) =
            spawn_full_session_writer(Box::new(w) as Box<dyn WriteSeek>, max_queue_segments)?;
        self.full_session = Some(FullSessionSink {
            tx,
            join,
            queued_bytes,
            max_queue_bytes,
            audio_cfgs,
            video_cfg: None,
            start_s: None,
            end_s: None,
            send_error: None,
        });
        Ok(())
    }

    pub fn finish_full_session(&mut self) -> io::Result<Option<FullSessionSummary>> {
        let Some(sink) = self.full_session.take() else {
            return Ok(None);
        };
        let start_s = sink.start_s;
        let end_s = sink.end_s;
        let send_error = sink.send_error.clone();
        finish_full_session_writer(sink)?;
        if let Some(error) = send_error {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, error));
        }
        let Some(start_s) = start_s else {
            return Ok(None);
        };
        let end_s = end_s.unwrap_or(start_s);
        Ok(Some(FullSessionSummary {
            start_s,
            end_s,
            duration_s: end_s - start_s,
        }))
    }

    /// Save the trailing `window_s` seconds as a finalized Hybrid MP4
    /// written to `w` (ddoc §6). `exclude_before_s` is the smart
    /// no-overlap mode. Returns the writer and the end pts of the saved
    /// footage — pass it back as `exclude_before_s` next time.
    ///
    /// Erroring (rather than writing an empty file) when no new footage
    /// exists lets the hotkey handler tell the user "nothing new to save".
    pub fn save_replay<W: Write + Seek>(
        &self,
        w: W,
        window_s: f64,
        exclude_before_s: Option<f64>,
    ) -> io::Result<(W, f64)> {
        let segments = self.save_window_segments(window_s, exclude_before_s)?;
        if segments.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no new footage in window",
            ));
        }
        let video_cfg = self.encoder.track_config();
        let starts_at_stream_origin = self.replay_starts_at_stream_origin(&segments);
        let audio_cfgs: Vec<_> = self
            .audio_sources
            .iter()
            .map(|s| {
                let mut cfg = s.track_config();
                if !starts_at_stream_origin {
                    cfg.pre_skip = MID_STREAM_REPLAY_OPUS_PRE_SKIP;
                }
                cfg
            })
            .collect();
        let mut track_cfgs = vec![TrackConfig::Video(video_cfg.clone())];
        for cfg in &audio_cfgs {
            track_cfgs.push(TrackConfig::Audio(cfg.clone()));
        }
        let mut writer = HybridMp4Writer::new_multi(w, track_cfgs)?;
        for seg in &segments {
            let per_track = segment_fragment_refs(seg, &video_cfg, &audio_cfgs);
            let slices: Vec<&[FragSampleRef<'_>]> =
                per_track.iter().map(|v| v.as_slice()).collect();
            writer.write_fragment_multi_borrowed(&slices)?;
        }
        let end_pts = segments.last().expect("non-empty").pts_end_s();
        Ok((writer.finalize()?, end_pts))
    }

    fn replay_starts_at_stream_origin(&self, segments: &[Segment]) -> bool {
        let Some(first) = segments.first() else {
            return false;
        };
        let origin = self.video_start_pts_s.unwrap_or(first.pts_start_s);
        (first.pts_start_s - origin).abs() <= 1e-9
    }

    fn save_window_segments(
        &self,
        window_s: f64,
        exclude_before_s: Option<f64>,
    ) -> io::Result<Vec<Segment>> {
        match &self.ring {
            ReplayStorage::Memory(ring) => Ok(ring
                .save_window(window_s, exclude_before_s)
                .into_iter()
                .cloned()
                .collect()),
            ReplayStorage::Disk(ring) => ring
                .save_window(window_s, exclude_before_s)
                .into_iter()
                .map(|seg| seg.load())
                .collect(),
        }
    }

    fn seal_pending(&mut self, boundary_pts_s: f64) -> Result<(), PipelineError> {
        let packets = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        let pts_start_s = packets[0].pts_s;
        let starts_with_keyframe = packets[0].is_keyframe;
        // ddoc §6: the timeline follows capture stamps, not encoder cadence
        // claims. Each sample lasts until the next pts; the sealing
        // keyframe's pts closes the GOP exactly; only the final seal
        // (boundary = ∞) trusts the encoder's own duration.
        let durations: Vec<f64> = (0..packets.len())
            .map(|i| {
                let next_pts = packets
                    .get(i + 1)
                    .map(|p| p.pts_s)
                    .unwrap_or(boundary_pts_s);
                if next_pts.is_finite() {
                    (next_pts - packets[i].pts_s).max(1e-4)
                } else {
                    packets[i].duration_s
                }
            })
            .collect();
        let duration_s: f64 = durations.iter().sum();
        let mut data = Vec::new();
        let mut samples = Vec::with_capacity(packets.len());
        for (p, &d) in packets.iter().zip(&durations) {
            samples.push(SampleInfo {
                size: p.data.len() as u32,
                duration_s: d,
                is_sync: p.is_keyframe,
            });
            data.extend_from_slice(&p.data);
        }

        // Audio captured before the first video packet is engine-init
        // lead-in: drop it, or video plays early by that offset.
        let timeline_start = self.video_start_pts_s.unwrap_or(pts_start_s);
        for pending in &mut self.pending_audio {
            pending.retain(|p| p.pts_s + p.duration_s > timeline_start + 1e-9);
        }
        // Audio packets ending at or before the boundary belong to this GOP.
        let mut audio = Vec::with_capacity(self.pending_audio.len());
        for pending in &mut self.pending_audio {
            let split = pending
                .iter()
                .position(|p| p.pts_s + p.duration_s > boundary_pts_s + 1e-9)
                .unwrap_or(pending.len());
            let mut track = TrackSamples::default();
            for p in pending.drain(..split) {
                track.samples.push(SampleInfo {
                    size: p.data.len() as u32,
                    duration_s: p.duration_s,
                    is_sync: true, // every Opus packet is independently decodable
                });
                track.data.extend_from_slice(&p.data);
            }
            audio.push(track);
        }
        self.recount_pending_audio_bytes();
        self.pending_started_pts_s = self
            .pending_audio
            .iter()
            .flat_map(|track| track.iter())
            .map(|packet| packet.pts_s)
            .filter(|pts| pts.is_finite())
            .min_by(f64::total_cmp);

        let seg = Arc::new(Segment {
            starts_with_keyframe,
            pts_start_s,
            duration_s,
            data,
            samples,
            audio,
        });
        let queue_full_session = self
            .full_session
            .as_ref()
            .is_some_and(|sink| sink.send_error.is_none());
        match &mut self.ring {
            ReplayStorage::Memory(ring) => ring.push_shared(Arc::clone(&seg)),
            ReplayStorage::Disk(ring) => ring.push_ref(&seg)?,
        }
        if queue_full_session {
            self.queue_full_session_segment(seg);
        }
        Ok(())
    }

    fn push_encoded_packet(&mut self, pkt: EncodedPacket) -> Result<(), PipelineError> {
        if self.video_start_pts_s.is_none() {
            if !pkt.is_keyframe {
                self.note_pending_start(pkt.pts_s);
                self.pre_keyframe_bytes = self.pre_keyframe_bytes.saturating_add(pkt.data.len());
                return Ok(());
            }
            self.video_start_pts_s = Some(pkt.pts_s);
            self.pre_keyframe_bytes = 0;
            for pending in &mut self.pending_audio {
                pending.retain(|audio| audio.pts_s + audio.duration_s > pkt.pts_s + 1e-9);
            }
            self.recount_pending_audio_bytes();
            self.pending_started_pts_s = Some(pkt.pts_s);
        }

        if pkt.is_keyframe && !self.pending.is_empty() {
            self.validate_pending_limits(pkt.pts_s)?;
            self.seal_pending(pkt.pts_s)?;
        }

        self.note_pending_start(pkt.pts_s);
        self.pending_bytes = self.pending_bytes.saturating_add(pkt.data.len());
        self.pending.push(pkt);
        Ok(())
    }

    fn poll_audio_until(&mut self, until_pts_s: f64) -> Result<(), PipelineError> {
        let mut added_bytes = 0usize;
        let mut first_pts_s: Option<f64> = None;
        for (source, pending) in self.audio_sources.iter_mut().zip(&mut self.pending_audio) {
            let packets = source.poll_packets(until_pts_s)?;
            added_bytes = packets.iter().fold(added_bytes, |total, packet| {
                total.saturating_add(packet.data.len())
            });
            for packet in &packets {
                if packet.pts_s.is_finite() {
                    first_pts_s =
                        Some(first_pts_s.map_or(packet.pts_s, |pts| pts.min(packet.pts_s)));
                }
            }
            pending.extend(packets);
        }
        self.pending_audio_bytes = self.pending_audio_bytes.saturating_add(added_bytes);
        if let Some(pts_s) = first_pts_s {
            self.note_pending_start(pts_s);
        }
        Ok(())
    }

    fn note_pending_start(&mut self, pts_s: f64) {
        if !pts_s.is_finite() {
            return;
        }
        self.pending_started_pts_s = Some(
            self.pending_started_pts_s
                .map_or(pts_s, |current| current.min(pts_s)),
        );
    }

    fn recount_pending_audio_bytes(&mut self) {
        self.pending_audio_bytes = self
            .pending_audio
            .iter()
            .flat_map(|track| track.iter())
            .fold(0usize, |total, packet| {
                total.saturating_add(packet.data.len())
            });
    }

    fn validate_pending_limits(&self, now_pts_s: f64) -> Result<(), PipelineError> {
        let pending_payload_bytes = self.pending_payload_bytes();
        if pending_payload_bytes > self.pending_byte_budget {
            return Err(EncodeError::Backend(format!(
                "encoder did not produce a keyframe before pending video/audio GOP budget was exceeded ({pending_payload_bytes} > {} bytes)",
                self.pending_byte_budget
            ))
            .into());
        }
        if let Some(start_pts_s) = self.pending_started_pts_s {
            let duration_s = now_pts_s - start_pts_s;
            if duration_s.is_finite() && duration_s > MAX_PENDING_GOP_DURATION_S {
                return Err(EncodeError::Backend(format!(
                    "encoder did not produce a keyframe before pending GOP duration exceeded {:.1} seconds ({duration_s:.3} seconds)",
                    MAX_PENDING_GOP_DURATION_S
                ))
                .into());
            }
        }
        Ok(())
    }

    fn pending_payload_bytes(&self) -> usize {
        self.pre_keyframe_bytes
            .saturating_add(self.pending_bytes)
            .saturating_add(self.pending_audio_bytes)
    }

    fn queue_full_session_segment(&mut self, seg: Arc<Segment>) {
        let Some(sink) = &mut self.full_session else {
            return;
        };
        if sink.send_error.is_some() {
            return;
        }
        let reserved_bytes = seg.byte_len();
        if !try_reserve_queue_bytes(&sink.queued_bytes, reserved_bytes, sink.max_queue_bytes) {
            sink.send_error = Some(format!(
                "full session writer queue byte budget exceeded ({reserved_bytes} byte segment, {} of {} bytes already queued); full-session recording stopped",
                sink.queued_bytes.load(Ordering::Acquire),
                sink.max_queue_bytes
            ));
            return;
        }
        let start_s = seg.pts_start_s;
        let end_s = seg.pts_end_s();
        let video_cfg = sink
            .video_cfg
            .get_or_insert_with(|| self.encoder.track_config())
            .clone();
        let msg = FullSessionWriteMsg::Segment(FullSessionSegment {
            video_cfg,
            audio_cfgs: sink.audio_cfgs.clone(),
            segment: seg,
            reserved_bytes,
        });
        match sink.tx.try_send(msg) {
            Ok(()) => {
                sink.start_s.get_or_insert(start_s);
                sink.end_s = Some(end_s);
            }
            Err(TrySendError::Full(msg)) => {
                release_message_reservation(&sink.queued_bytes, &msg);
                sink.send_error = Some(
                    "full session writer queue reached its segment limit; full-session recording stopped"
                        .into(),
                );
            }
            Err(TrySendError::Disconnected(msg)) => {
                release_message_reservation(&sink.queued_bytes, &msg);
                sink.send_error = Some("full session writer stopped".into());
            }
        }
    }
}

fn pending_byte_budget(max_buffer_bytes: usize) -> usize {
    max_buffer_bytes.clamp(1, MAX_PENDING_GOP_BYTES)
}

fn spawn_full_session_writer(
    target: Box<dyn WriteSeek>,
    max_queue_segments: usize,
) -> io::Result<(
    SyncSender<FullSessionWriteMsg>,
    JoinHandle<()>,
    Arc<AtomicUsize>,
)> {
    let (tx, rx) = mpsc::sync_channel(max_queue_segments);
    let queued_bytes = Arc::new(AtomicUsize::new(0));
    let writer_queued_bytes = Arc::clone(&queued_bytes);
    let join = thread::Builder::new()
        .name("clipline-full-session-writer".into())
        .spawn(move || full_session_writer_loop(target, rx, writer_queued_bytes))?;
    Ok((tx, join, queued_bytes))
}

fn finish_full_session_writer(sink: FullSessionSink) -> io::Result<()> {
    let (reply_tx, reply_rx) = mpsc::channel();
    sink.tx
        .send(FullSessionWriteMsg::Finish(reply_tx))
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "full session writer stopped"))?;
    let result = reply_rx.recv().unwrap_or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "full session writer stopped before finalizing",
        ))
    });
    let join_result = sink.join.join();
    match (result, join_result) {
        (Err(e), _) => Err(e),
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(_)) => Err(io::Error::other("full session writer thread panicked")),
    }
}

fn full_session_writer_loop(
    target: Box<dyn WriteSeek>,
    rx: Receiver<FullSessionWriteMsg>,
    queued_bytes: Arc<AtomicUsize>,
) {
    let mut target = Some(target);
    let mut writer: Option<HybridMp4Writer<Box<dyn WriteSeek>>> = None;
    let mut first_error: Option<io::Error> = None;
    while let Ok(msg) = rx.recv() {
        match msg {
            FullSessionWriteMsg::Segment(segment) => {
                let reserved_bytes = segment.reserved_bytes;
                if first_error.is_none() {
                    if let Err(e) = write_full_session_segment(
                        &mut target,
                        &mut writer,
                        segment.video_cfg,
                        segment.audio_cfgs,
                        segment.segment,
                    ) {
                        first_error = Some(e);
                        writer = None;
                    }
                }
                queued_bytes.fetch_sub(reserved_bytes, Ordering::AcqRel);
            }
            FullSessionWriteMsg::Finish(reply) => {
                let result = if let Some(e) = first_error.take() {
                    Err(e)
                } else if let Some(writer) = writer.take() {
                    writer.finalize().map(|_| ())
                } else {
                    Ok(())
                };
                let _ = reply.send(result);
                break;
            }
        }
    }
}

fn write_full_session_segment(
    target: &mut Option<Box<dyn WriteSeek>>,
    writer: &mut Option<HybridMp4Writer<Box<dyn WriteSeek>>>,
    video_cfg: clipline_mp4::VideoTrackConfig,
    audio_cfgs: Vec<clipline_mp4::AudioTrackConfig>,
    seg: Arc<Segment>,
) -> io::Result<()> {
    if writer.is_none() {
        let mut track_cfgs = vec![TrackConfig::Video(video_cfg.clone())];
        for cfg in &audio_cfgs {
            track_cfgs.push(TrackConfig::Audio(cfg.clone()));
        }
        let target = target.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "full session writer target missing",
            )
        })?;
        *writer = Some(HybridMp4Writer::new_multi(target, track_cfgs)?);
    }
    let per_track = segment_fragment_refs(&seg, &video_cfg, &audio_cfgs);
    let slices: Vec<&[FragSampleRef<'_>]> = per_track.iter().map(|v| v.as_slice()).collect();
    writer
        .as_mut()
        .expect("writer initialized")
        .write_fragment_multi_borrowed(&slices)
}

fn try_reserve_queue_bytes(queued: &AtomicUsize, bytes: usize, max_bytes: usize) -> bool {
    queued
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(bytes).filter(|next| *next <= max_bytes)
        })
        .is_ok()
}

fn release_message_reservation(queued: &AtomicUsize, msg: &FullSessionWriteMsg) {
    if let FullSessionWriteMsg::Segment(segment) = msg {
        queued.fetch_sub(segment.reserved_bytes, Ordering::AcqRel);
    }
}

fn segment_fragment_refs<'a>(
    seg: &'a Segment,
    video_cfg: &clipline_mp4::VideoTrackConfig,
    audio_cfgs: &[clipline_mp4::AudioTrackConfig],
) -> Vec<Vec<FragSampleRef<'a>>> {
    let video_ts = video_cfg.timescale as f64;
    let video: Vec<FragSampleRef<'a>> = seg
        .sample_slices()
        .zip(&seg.samples)
        .map(|(slice, info)| FragSampleRef {
            data: slice,
            duration: (info.duration_s * video_ts).round() as u32,
            is_sync: info.is_sync,
        })
        .collect();
    let mut per_track: Vec<Vec<FragSampleRef<'a>>> = vec![video];
    for (track, cfg) in seg.audio.iter().zip(audio_cfgs) {
        let ts = cfg.sample_rate as f64;
        per_track.push(
            track
                .sample_slices()
                .zip(&track.samples)
                .map(|(slice, info)| FragSampleRef {
                    data: slice,
                    duration: (info.duration_s * ts).round() as u32,
                    is_sync: info.is_sync,
                })
                .collect(),
        );
    }
    // Segments recorded before an audio source was attached have fewer audio
    // tracks; pad with empty runs to keep alignment.
    per_track.resize_with(1 + audio_cfgs.len(), Vec::new);
    per_track
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{MockCapture, MockEncoder};
    use crate::traits::{EncodedPacket, Frame};
    use clipline_mp4::VideoTrackConfig;
    use clipline_test_utils::TestDir;

    struct NeverKeyframeEncoder {
        fps: u32,
    }

    impl NeverKeyframeEncoder {
        fn new(fps: u32) -> Self {
            Self { fps }
        }
    }

    impl Encoder for NeverKeyframeEncoder {
        fn encode(&mut self, frame: &Frame) -> Result<Vec<EncodedPacket>, EncodeError> {
            Ok(vec![EncodedPacket {
                data: vec![0xEE; 128],
                pts_s: frame.pts_s,
                duration_s: 1.0 / self.fps as f64,
                is_keyframe: false,
            }])
        }

        fn track_config(&self) -> VideoTrackConfig {
            VideoTrackConfig::h264(
                128,
                128,
                90_000,
                vec![0x67, 0x64, 0x00, 0x0A, 0xAC],
                vec![0x68, 0xEE, 0x38, 0x80],
            )
        }
    }

    #[test]
    fn groups_packets_into_gop_aligned_segments() {
        // 90 frames at 30 fps, GOP 30 → exactly 3 keyframe-led segments.
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        rec.run_to_end().unwrap();
        let ring = rec.ring().unwrap();
        assert_eq!(ring.len(), 3);
        for seg in ring.segments() {
            assert!(seg.starts_with_keyframe);
            assert_eq!(seg.samples.len(), 30);
            assert!((seg.duration_s - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn byte_budget_evicts_oldest_gop() {
        // Each MockEncoder sample is 64–70 bytes → a GOP of 30 ≈ ~2 KB.
        // Budget for ~2 GOPs: the first of three must be evicted.
        let mut rec = Recorder::new(MockCapture::new(90, 30), MockEncoder::new(30, 30), 4 * 1024);
        rec.run_to_end().unwrap();
        let ring = rec.ring().unwrap();
        assert_eq!(ring.len(), 2);
        let first = ring.segments().next().unwrap();
        assert!((first.pts_start_s - 1.0).abs() < 1e-6, "GOP at t=0 evicted");
    }

    #[test]
    fn pending_packets_are_byte_budgeted_when_keyframes_never_arrive() {
        let mut rec = Recorder::new(MockCapture::new(20, 30), NeverKeyframeEncoder::new(30), 512);

        let err = rec
            .run_to_end()
            .expect_err("unkeyframed stream should fail");

        assert!(
            err.to_string().contains("keyframe") && err.to_string().contains("budget"),
            "error should explain the keyframe/budget guard, got {err}"
        );
    }

    #[test]
    fn pending_gop_budget_counts_audio_payloads() {
        use crate::mock::MockAudioSource;

        let mut video_only =
            Recorder::new(MockCapture::new(10, 30), MockEncoder::new(30, 30), 1024);
        video_only
            .run_to_end()
            .expect("video payload alone fits the pending budget");

        let mut with_audio =
            Recorder::new(MockCapture::new(10, 30), MockEncoder::new(30, 30), 1024)
                .with_audio(Box::new(MockAudioSource::new(48_000, 20)));
        let error = with_audio
            .run_to_end()
            .expect_err("audio must consume the same pending GOP budget");

        assert!(
            error.to_string().contains("video/audio GOP budget"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn pending_gop_duration_is_bounded_when_keyframes_stop() {
        let mut recorder = Recorder::new(
            MockCapture::new(360, 30),
            MockEncoder::new(1000, 30),
            usize::MAX,
        );

        let error = recorder
            .run_to_end()
            .expect_err("an encoder must not retain an arbitrarily long GOP");

        assert!(
            error.to_string().contains("GOP duration") && error.to_string().contains("keyframe"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn short_stream_without_initial_keyframe_is_reported() {
        let mut rec = Recorder::new(
            MockCapture::new(1, 30),
            NeverKeyframeEncoder::new(30),
            usize::MAX,
        );

        let err = rec
            .run_to_end()
            .expect_err("short unkeyframed stream should fail");

        assert!(
            err.to_string().contains("keyframe") && err.to_string().contains("ended"),
            "error should explain that the stream ended before an initial keyframe, got {err}"
        );
        assert_eq!(rec.ring().unwrap().len(), 0);
    }

    #[test]
    fn audio_packets_land_in_their_gop_segments() {
        use crate::mock::MockAudioSource;
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        )
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)));
        rec.run_to_end().unwrap();
        let ring = rec.ring().unwrap();
        assert_eq!(ring.len(), 3);
        for (i, seg) in ring.segments().enumerate() {
            assert_eq!(seg.audio.len(), 1, "one audio track");
            // 1 s GOP at 20 ms packets = 50 packets per segment.
            assert_eq!(seg.audio[0].samples.len(), 50, "segment {i}");
        }
        // First packet of the second segment starts at its GOP boundary.
        let seg2 = ring.segments().nth(1).unwrap();
        assert_eq!(&seg2.audio[0].data[..6], b"P00050");
    }

    #[test]
    fn pending_audio_reservation_is_released_when_each_gop_seals() {
        use crate::mock::MockAudioSource;

        let mut recorder =
            Recorder::new(MockCapture::new(90, 30), MockEncoder::new(30, 30), 8 * 1024)
                .with_audio(Box::new(MockAudioSource::new(48_000, 20)));

        recorder
            .run_to_end()
            .expect("each individual GOP fits even though all three do not");
        assert_eq!(
            recorder.ring().unwrap().len(),
            2,
            "ring still enforces its budget"
        );
    }

    #[test]
    fn save_replay_preserves_multiple_audio_tracks() {
        use crate::mock::MockAudioSource;
        use clipline_mp4::walker::{children, find, walk};

        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        )
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)))
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)));

        rec.run_to_end().unwrap();
        for seg in rec.ring().unwrap().segments() {
            assert_eq!(seg.audio.len(), 2, "system plus microphone tracks");
        }

        let (buf, _) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, e)| (w.into_inner(), e))
            .expect("multi-audio save");
        let boxes = walk(&buf);
        let moov = find(&boxes, b"moov").expect("moov");
        let kids = children(&buf, moov);
        let traks = kids.iter().filter(|b| &b.fourcc == b"trak").count();
        assert_eq!(traks, 3, "video plus two audio tracks");
    }

    fn first_opus_pre_skip(buf: &[u8]) -> u16 {
        let fourcc = buf
            .windows(4)
            .position(|window| window == b"dOps")
            .expect("dOps box");
        let p = fourcc + 4;
        u16::from_be_bytes(buf[p + 2..p + 4].try_into().expect("pre-skip bytes"))
    }

    #[test]
    fn save_replay_from_stream_start_keeps_opus_pre_skip() {
        use crate::mock::MockAudioSource;

        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        )
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)));

        rec.run_to_end().unwrap();
        let (buf, _) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, end)| (w.into_inner(), end))
            .expect("replay from stream start");

        assert_eq!(first_opus_pre_skip(&buf), 312);
    }

    #[test]
    fn save_replay_from_middle_discards_opus_start_preroll() {
        use crate::mock::MockAudioSource;

        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        )
        .with_audio(Box::new(MockAudioSource::new(48_000, 20)));

        rec.run_to_end().unwrap();
        let (buf, _) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 1.5, None)
            .map(|(w, end)| (w.into_inner(), end))
            .expect("replay from middle");

        assert_eq!(
            first_opus_pre_skip(&buf),
            960,
            "mid-stream replay clips discard only the first Opus frame to avoid cold decoder startup artifacts"
        );
    }

    #[test]
    fn disk_replay_storage_saves_same_bytes_as_memory_storage() {
        let mut ram = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        ram.run_to_end().unwrap();
        let (ram_buf, _) = ram
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, end)| (w.into_inner(), end))
            .unwrap();

        let dir = TestDir::new("clipline-pipeline", "disk-equivalence");
        let mut disk = Recorder::new_with_replay_storage(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            ReplayStorageConfig::Disk {
                max_bytes: usize::MAX,
                dir: dir.path().to_path_buf(),
            },
        )
        .unwrap();
        disk.run_to_end().unwrap();
        let (disk_buf, _) = disk
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, end)| (w.into_inner(), end))
            .unwrap();

        assert_eq!(disk.ring_len(), ram.ring_len());
        assert_eq!(disk.ring_bytes(), ram.ring_bytes());
        assert_eq!(disk_buf, ram_buf);
    }

    #[test]
    fn full_session_sink_keeps_segments_evicted_from_replay_ring() {
        let dir = TestDir::new("clipline-pipeline", "full-session");
        let path = dir.path().join("session.mp4");
        let file = std::fs::File::create(&path).unwrap();
        let mut rec = Recorder::new(MockCapture::new(90, 30), MockEncoder::new(30, 30), 4 * 1024);

        rec.start_full_session(file).unwrap();
        rec.run_to_end().unwrap();
        let summary = rec.finish_full_session().unwrap().expect("session summary");

        assert_eq!(
            rec.ring().unwrap().len(),
            2,
            "oldest GOP evicted from replay ring"
        );
        assert!((summary.start_s - 0.0).abs() < 1e-6);
        assert!((summary.duration_s - 3.0).abs() < 1e-6);
        let data = std::fs::read(&path).unwrap();
        let duration = clipline_mp4::walker::movie_duration_s(&data).unwrap();
        assert!(
            (duration - 3.0).abs() < 1e-3,
            "full-session file keeps all GOPs, got {duration}"
        );
    }

    #[test]
    fn full_session_initializes_muxer_after_encoder_config_is_ready() {
        let dir = TestDir::new("clipline-pipeline", "full-session-lazy-config");
        let path = dir.path().join("session.mp4");
        let file = std::fs::File::create(&path).unwrap();
        let mut rec = Recorder::new(
            MockCapture::new(60, 30),
            DelayedTrackConfig {
                inner: MockEncoder::new(30, 30),
                encoded_any: false,
            },
            usize::MAX,
        );

        rec.start_full_session(file).unwrap();
        rec.run_to_end().unwrap();
        rec.finish_full_session().unwrap().expect("session summary");

        let data = std::fs::read(&path).unwrap();
        assert!(
            data.windows(DELAYED_SPS.len()).any(|w| w == DELAYED_SPS),
            "full-session moov must use the encoder config populated by the first packets"
        );
    }

    #[test]
    fn full_session_write_failure_does_not_abort_replay_capture() {
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );

        rec.start_full_session(FailingWriter).unwrap();
        rec.run_to_end()
            .expect("secondary session sink must not stop capture");

        assert_eq!(rec.ring().unwrap().len(), 3);
        let err = rec.finish_full_session().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn full_session_queue_budget_failure_does_not_abort_replay_capture() {
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );

        rec.start_full_session_with_limits(std::io::Cursor::new(Vec::new()), 1, 1)
            .unwrap();
        rec.run_to_end()
            .expect("full-session backpressure must not stop replay capture");

        assert_eq!(rec.ring().unwrap().len(), 3);
        let err = rec.finish_full_session().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        assert!(
            err.to_string().contains("queue byte budget"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn full_session_queue_byte_reservations_never_exceed_the_limit() {
        let queued = AtomicUsize::new(0);

        assert!(try_reserve_queue_bytes(&queued, 6, 10));
        assert_eq!(queued.load(Ordering::Acquire), 6);
        assert!(!try_reserve_queue_bytes(&queued, 5, 10));
        assert_eq!(queued.load(Ordering::Acquire), 6);

        queued.fetch_sub(6, Ordering::AcqRel);
        assert!(try_reserve_queue_bytes(&queued, 10, 10));
        assert_eq!(queued.load(Ordering::Acquire), 10);
    }

    #[test]
    fn stalled_full_session_writer_hits_segment_limit_without_blocking_capture() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let writer = GatedWriter {
            inner: std::io::Cursor::new(Vec::new()),
            entered: Some(entered_tx),
            release: release_rx,
        };
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        rec.start_full_session_with_limits(writer, usize::MAX, 1)
            .unwrap();

        for _ in 0..31 {
            assert!(rec.step().unwrap());
        }
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("writer must begin the first segment");
        rec.run_to_end()
            .expect("stalled full-session output must not block capture");
        assert_eq!(rec.ring().unwrap().len(), 3);

        release_tx.send(()).unwrap();
        let err = rec.finish_full_session().unwrap_err();
        assert!(
            err.to_string().contains("segment limit"),
            "unexpected error: {err}"
        );
    }

    struct GatedWriter {
        inner: std::io::Cursor<Vec<u8>>,
        entered: Option<Sender<()>>,
        release: Receiver<()>,
    }

    impl Write for GatedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if let Some(entered) = self.entered.take() {
                entered
                    .send(())
                    .map_err(|_| io::Error::other("gate observer stopped"))?;
                self.release
                    .recv()
                    .map_err(|_| io::Error::other("gate released by disconnect"))?;
            }
            self.inner.write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    impl Seek for GatedWriter {
        fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("disk full"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Seek for FailingWriter {
        fn seek(&mut self, _pos: io::SeekFrom) -> io::Result<u64> {
            Ok(0)
        }
    }

    const DELAYED_SPS: &[u8] = &[0x67, 0x64, 0x00, 0x0A, 0xAC];

    struct DelayedTrackConfig {
        inner: MockEncoder,
        encoded_any: bool,
    }

    impl Encoder for DelayedTrackConfig {
        fn encode(
            &mut self,
            frame: &crate::traits::Frame,
        ) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            let packets = self.inner.encode(frame)?;
            self.encoded_any = true;
            Ok(packets)
        }

        fn track_config(&self) -> clipline_mp4::VideoTrackConfig {
            if self.encoded_any {
                return self.inner.track_config();
            }
            clipline_mp4::VideoTrackConfig::h264(128, 128, 90_000, Vec::new(), Vec::new())
        }
    }

    /// Wraps MockEncoder but holds back the latest packet until finish() —
    /// models real encoders' internal buffering.
    struct OneFrameLatency {
        inner: MockEncoder,
        held: Option<crate::traits::EncodedPacket>,
    }

    impl Encoder for OneFrameLatency {
        fn encode(
            &mut self,
            frame: &crate::traits::Frame,
        ) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            let mut out = self.inner.encode(frame)?;
            let newly = out.pop();
            let released = self.held.take();
            self.held = newly;
            Ok(released.into_iter().collect())
        }

        fn track_config(&self) -> clipline_mp4::VideoTrackConfig {
            self.inner.track_config()
        }

        fn finish(
            &mut self,
        ) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            Ok(self.held.take().into_iter().collect())
        }
    }

    /// Encoder echoing nominal durations while pts jitters (VRR-style):
    /// the sealed timeline must follow the STAMPS (ddoc §6), not the echo.
    struct JitteryEncoder {
        inner: MockEncoder,
    }

    impl Encoder for JitteryEncoder {
        fn encode(
            &mut self,
            frame: &crate::traits::Frame,
        ) -> Result<Vec<crate::traits::EncodedPacket>, crate::traits::EncodeError> {
            let mut pkts = self.inner.encode(frame)?;
            for p in &mut pkts {
                // Stamps: frames alternate 10 ms / 30 ms apart, while the
                // encoder still claims a flat 1/30 s duration.
                let idx = (p.pts_s * 30.0).round();
                p.pts_s = (idx / 2.0).floor() * 0.04 + if idx % 2.0 == 1.0 { 0.01 } else { 0.0 };
            }
            Ok(pkts)
        }
        fn track_config(&self) -> clipline_mp4::VideoTrackConfig {
            self.inner.track_config()
        }
    }

    #[test]
    fn sealed_durations_come_from_pts_deltas_not_encoder_claims() {
        // GOP of 4 over 8 frames → two segments, boundary at frame 4.
        let enc = JitteryEncoder {
            inner: MockEncoder::new(4, 30),
        };
        let mut rec = Recorder::new(MockCapture::new(8, 30), enc, usize::MAX);
        rec.run_to_end().unwrap();
        let segs: Vec<_> = rec.ring().unwrap().segments().collect();
        assert_eq!(segs.len(), 2);
        // Within a GOP: 10/30/10 ms gaps, NOT the encoder's flat 33.3 ms.
        let d: Vec<f64> = segs[0].samples.iter().map(|s| s.duration_s).collect();
        assert!((d[0] - 0.01).abs() < 1e-9, "got {d:?}");
        assert!((d[1] - 0.03).abs() < 1e-9, "got {d:?}");
        assert!((d[2] - 0.01).abs() < 1e-9, "got {d:?}");
        // Boundary: last sample of GOP 1 closes exactly at GOP 2's keyframe.
        let gop2_start = segs[1].pts_start_s;
        assert!(
            (segs[0].pts_end_s() - gop2_start).abs() < 1e-9,
            "no gap, no overlap"
        );
        // Final seal falls back to the encoder duration for the last sample.
        let last = segs[1].samples.last().unwrap();
        assert!((last.duration_s - 1.0 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn run_to_end_drains_encoder_via_finish() {
        let enc = OneFrameLatency {
            inner: MockEncoder::new(30, 30),
            held: None,
        };
        let mut rec = Recorder::new(MockCapture::new(30, 30), enc, usize::MAX);
        rec.run_to_end().unwrap();
        // All 30 frames present despite the encoder's one-frame latency.
        let total: usize = rec
            .ring()
            .unwrap()
            .segments()
            .map(|s| s.samples.len())
            .sum();
        assert_eq!(total, 30);
    }

    #[test]
    fn save_replay_works_between_steps_while_recording() {
        let mut rec = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        // Two GOPs in: a save must succeed without ending the recording.
        for _ in 0..60 {
            assert!(rec.step().unwrap());
        }
        let (buf, end) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, None)
            .map(|(w, e)| (w.into_inner(), e))
            .expect("mid-recording save");
        assert!(!buf.is_empty());
        assert!(
            (end - 1.0).abs() < 1e-6,
            "one sealed GOP at save time (second pending)"
        );
        // Recording continues; smart mode skips the already-saved second.
        for _ in 0..30 {
            assert!(rec.step().unwrap());
        }
        assert!(!rec.step().unwrap(), "source exhausted");
        rec.finish_stream().unwrap();
        let (_, end2) = rec
            .save_replay(std::io::Cursor::new(Vec::new()), 10.0, Some(end))
            .expect("post-finish save");
        assert!((end2 - 3.0).abs() < 1e-6, "everything sealed after finish");
        // run_to_end equivalence: same segment layout as the stepped path.
        let mut whole = Recorder::new(
            MockCapture::new(90, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        whole.run_to_end().unwrap();
        assert_eq!(whole.ring().unwrap().len(), rec.ring().unwrap().len());
    }

    /// Shifts a capture source's pts later — models the real lead-in
    /// between clock creation and the first WGC frame.
    struct OffsetCapture {
        inner: MockCapture,
        offset_s: f64,
    }

    impl crate::traits::CaptureEngine for OffsetCapture {
        fn next_frame(
            &mut self,
        ) -> Result<Option<crate::traits::Frame>, crate::traits::CaptureError> {
            Ok(self.inner.next_frame()?.map(|mut f| {
                f.pts_s += self.offset_s;
                f
            }))
        }
    }

    #[test]
    fn audio_lead_in_before_first_video_frame_is_dropped() {
        // Video starts 0.5 s after the shared clock origin; audio has been
        // capturing (and silence-filling) since t=0. The pre-video audio
        // must not ride in the file or video plays early by the lead-in.
        use crate::mock::MockAudioSource;
        let cap = OffsetCapture {
            inner: MockCapture::new(60, 30),
            offset_s: 0.5,
        };
        let mut rec = Recorder::new(cap, MockEncoder::new(30, 30), usize::MAX)
            .with_audio(Box::new(MockAudioSource::new(48_000, 20)));
        rec.run_to_end().unwrap();
        let segs: Vec<_> = rec.ring().unwrap().segments().collect();
        assert_eq!(segs.len(), 2);
        // First segment: audio coverage matches video duration within one
        // 20 ms packet (the packet straddling the boundary is dropped).
        let covered: f64 = segs[0].audio[0].samples.iter().map(|s| s.duration_s).sum();
        assert!(
            (covered - segs[0].duration_s).abs() <= 0.02 + 1e-9,
            "lead-in dropped: covered {covered}, video {}",
            segs[0].duration_s
        );
        // And the first kept packet starts at/after the video start.
        // (MockAudioSource stamps pts; we can't read them back from the
        // sealed track, but coverage bounds above imply it.)
    }

    #[test]
    fn trailing_partial_gop_is_sealed_at_end() {
        // 45 frames, GOP 30 → one full GOP + one 15-frame partial.
        let mut rec = Recorder::new(
            MockCapture::new(45, 30),
            MockEncoder::new(30, 30),
            usize::MAX,
        );
        rec.run_to_end().unwrap();
        let counts: Vec<usize> = rec
            .ring()
            .unwrap()
            .segments()
            .map(|s| s.samples.len())
            .collect();
        assert_eq!(counts, vec![30, 15]);
    }
}
