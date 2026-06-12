//! System-audio capture: WASAPI loopback on the default render endpoint
//! (ddoc §10), QPC-stamped against the shared capture clock, assembled
//! into 20 ms frames and Opus-encoded behind `AudioSource`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IAudioCaptureClient, IAudioClient, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    WAVE_FORMAT_PCM,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToString};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};

use clipline_mp4::AudioTrackConfig;

use crate::clock::RelativeClock;
use crate::opus::{OpusFrameEncoder, FRAME_DURATION_S, FRAME_LEN};
use crate::pcm::{
    apply_gain, extract_mono_centered, extract_stereo, resample_stereo_linear, LoopbackAssembler,
};
use crate::traits::{AudioPacket, AudioSource, CaptureError};

const OPUS_SAMPLE_RATE: u32 = 48_000;
const MIX_FRAME_EPSILON_S: f64 = FRAME_DURATION_S / 4.0;
const MISSING_SOURCE_GRACE_S: f64 = FRAME_DURATION_S * 3.0;

type PcmFrame = (f64, Vec<f32>);

#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub struct AudioDeviceList {
    pub outputs: Vec<AudioDeviceInfo>,
    pub inputs: Vec<AudioDeviceInfo>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AudioLevel {
    pub rms: f32,
    pub peak: f32,
    pub sample_count: usize,
}

#[derive(Debug, Clone)]
pub struct WasapiMonitorChunk {
    pub level: AudioLevel,
    pub samples: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasapiChannelMode {
    Mono,
    Stereo,
}

#[derive(Debug, Clone, Copy)]
enum EndpointMode {
    OutputLoopback,
    InputCapture(WasapiChannelMode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleFormat {
    Float32,
    Pcm16,
    Pcm24,
    Pcm32,
}

#[derive(Debug, Clone, Copy)]
struct MixFormat {
    channels: u16,
    sample_rate: u32,
    sample_format: SampleFormat,
}

#[derive(Debug, Default)]
struct AudioLevelAccumulator {
    sum_squares: f64,
    peak: f32,
    sample_count: usize,
}

impl AudioLevelAccumulator {
    fn add(&mut self, samples: &[f32]) {
        for &sample in samples {
            let abs = sample.abs();
            self.peak = self.peak.max(abs);
            self.sum_squares += sample as f64 * sample as f64;
        }
        self.sample_count += samples.len();
    }

    fn take(&mut self) -> AudioLevel {
        let rms = if self.sample_count == 0 {
            0.0
        } else {
            (self.sum_squares / self.sample_count as f64).sqrt() as f32
        };
        let level = AudioLevel {
            rms,
            peak: self.peak,
            sample_count: self.sample_count,
        };
        *self = Self::default();
        level
    }
}

struct WasapiPcmCapture {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    clock: RelativeClock,
    channels: u16,
    sample_rate: u32,
    sample_format: SampleFormat,
    mode: EndpointMode,
    volume: f32,
    level: AudioLevelAccumulator,
    assembler: LoopbackAssembler,
    queue: VecDeque<PcmFrame>,
}

pub struct WasapiLoopback {
    pcm: WasapiPcmCapture,
    opus: OpusFrameEncoder,
    queue: Vec<AudioPacket>,
}

pub struct WasapiMixedLoopback {
    sources: Vec<WasapiPcmCapture>,
    pending: Vec<VecDeque<PcmFrame>>,
    source_ready_until_s: Vec<f64>,
    mixed_until_s: f64,
    opus: OpusFrameEncoder,
    queue: Vec<AudioPacket>,
}

fn init(e: windows::core::Error) -> CaptureError {
    CaptureError::Init(format!("WASAPI: {e}"))
}

impl WasapiPcmCapture {
    fn start_output(
        clock: RelativeClock,
        device_id: Option<&str>,
        volume: f64,
    ) -> Result<Self, CaptureError> {
        Self::start_endpoint(
            clock,
            eRender,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            device_id,
            volume,
            EndpointMode::OutputLoopback,
        )
    }

    fn start_microphone(
        clock: RelativeClock,
        device_id: Option<&str>,
        volume: f64,
        channels: WasapiChannelMode,
    ) -> Result<Self, CaptureError> {
        Self::start_endpoint(
            clock,
            eCapture,
            0,
            device_id,
            volume,
            EndpointMode::InputCapture(channels),
        )
    }

    fn start_endpoint(
        clock: RelativeClock,
        dataflow: EDataFlow,
        streamflags: u32,
        device_id: Option<&str>,
        volume: f64,
        mode: EndpointMode,
    ) -> Result<Self, CaptureError> {
        init_com()?;
        // SAFETY: standard MMDevice activation chain; all results checked.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(init)?;
            let device = endpoint_device(&enumerator, dataflow, device_id).map_err(init)?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(init)?;

            let format_ptr = client.GetMixFormat().map_err(init)?;
            let format = &*format_ptr;
            // Copy packed fields to locals (references into packed structs are UB).
            let tag = format.wFormatTag;
            let ch = format.nChannels;
            let rate = format.nSamplesPerSec;
            let bits = format.wBitsPerSample;
            let Some(mix) = parse_mix_format(format) else {
                CoTaskMemFree(Some(format_ptr as *const _));
                return Err(CaptureError::Init(format!(
                    "unsupported mix format: tag {tag} ch {ch} rate {rate} bits {bits} \
                     (need float32 or signed PCM)"
                )));
            };
            // 1 s device buffer: poll_packets runs per video frame, this
            // gives ~60 polls of headroom.
            let r = client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                streamflags,
                10_000_000,
                0,
                format_ptr,
                None,
            );
            CoTaskMemFree(Some(format_ptr as *const _));
            r.map_err(init)?;

            let capture: IAudioCaptureClient = client.GetService().map_err(init)?;
            client.Start().map_err(init)?;

            // Anchor the audio timeline at the clock origin (recording
            // start): the gap fill turns any lead-in before the first
            // device buffer into silence, keeping the muxed track aligned
            // with video (both tracks start at t=0 in the file).
            let mut assembler = LoopbackAssembler::new();
            assembler.push_chunk(0.0, &[]);

            Ok(Self {
                client,
                capture,
                clock,
                channels: mix.channels,
                sample_rate: mix.sample_rate,
                sample_format: mix.sample_format,
                mode,
                volume: (volume.clamp(0.0, 2.0)) as f32,
                level: AudioLevelAccumulator::default(),
                assembler,
                queue: VecDeque::new(),
            })
        }
    }

    pub fn take_level(&mut self) -> AudioLevel {
        self.level.take()
    }

    fn decode_samples(&self, data: *const u8, frames: u32) -> Vec<f32> {
        let sample_count = frames as usize * self.channels as usize;
        // SAFETY: WASAPI's buffer is valid until ReleaseBuffer. Callers copy
        // before releasing, and each branch reads exactly the active frames.
        unsafe {
            match self.sample_format {
                SampleFormat::Float32 => {
                    std::slice::from_raw_parts(data as *const f32, sample_count).to_vec()
                }
                SampleFormat::Pcm16 => std::slice::from_raw_parts(data as *const i16, sample_count)
                    .iter()
                    .map(|&s| s as f32 / 32_768.0)
                    .collect(),
                SampleFormat::Pcm24 => std::slice::from_raw_parts(data, sample_count * 3)
                    .chunks_exact(3)
                    .map(|b| {
                        let raw = b[0] as i32 | ((b[1] as i32) << 8) | ((b[2] as i32) << 16);
                        let signed = (raw << 8) >> 8;
                        signed as f32 / 8_388_608.0
                    })
                    .collect(),
                SampleFormat::Pcm32 => std::slice::from_raw_parts(data as *const i32, sample_count)
                    .iter()
                    .map(|&s| s as f32 / 2_147_483_648.0)
                    .collect(),
            }
        }
    }

    fn stereo_samples(&mut self, samples: &[f32]) -> Vec<f32> {
        let mut stereo = match self.mode {
            EndpointMode::OutputLoopback
            | EndpointMode::InputCapture(WasapiChannelMode::Stereo) => {
                extract_stereo(samples, self.channels)
            }
            EndpointMode::InputCapture(WasapiChannelMode::Mono) => {
                extract_mono_centered(samples, self.channels)
            }
        };
        if self.sample_rate != OPUS_SAMPLE_RATE {
            stereo = resample_stereo_linear(&stereo, self.sample_rate, OPUS_SAMPLE_RATE);
        }
        apply_gain(&mut stereo, self.volume);
        self.level.add(&stereo);
        stereo
    }

    /// Drain everything the device has buffered into the assembler.
    fn drain_device(&mut self) -> Result<(), CaptureError> {
        let lost = |e: windows::core::Error| CaptureError::DeviceLost(format!("WASAPI: {e}"));
        // SAFETY: GetBuffer/ReleaseBuffer pairs per the capture-client
        // contract; the data pointer is valid for `frames` frames until
        // ReleaseBuffer.
        unsafe {
            while self.capture.GetNextPacketSize().map_err(lost)? > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                let mut qpc_100ns = 0u64;
                self.capture
                    .GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        None,
                        Some(&mut qpc_100ns),
                    )
                    .map_err(lost)?;
                let pts_s = self.clock.pts_s(qpc_100ns as i64);
                let n = frames as usize * self.channels as usize;
                let samples: Vec<f32> = if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
                    vec![0.0; n]
                } else {
                    self.decode_samples(data as *const u8, frames)
                };
                self.capture.ReleaseBuffer(frames).map_err(lost)?;
                let stereo = self.stereo_samples(&samples);
                self.assembler.push_chunk(pts_s, &stereo);
            }
        }
        Ok(())
    }

    fn poll_frames(&mut self, until_pts_s: f64) -> Result<Vec<PcmFrame>, CaptureError> {
        self.drain_device()?;
        while let Some(frame) = self.assembler.pop_frame() {
            self.queue.push_back(frame);
        }
        let split = self
            .queue
            .iter()
            .position(|(pts_s, _)| pts_s + FRAME_DURATION_S > until_pts_s + 1e-9)
            .unwrap_or(self.queue.len());
        Ok(self.queue.drain(..split).collect())
    }
}

impl Drop for WasapiPcmCapture {
    fn drop(&mut self) {
        // SAFETY: Stop on a started client is always valid.
        let _ = unsafe { self.client.Stop() };
    }
}

impl WasapiLoopback {
    /// Start capturing the default render endpoint in loopback. `clock`
    /// maps the buffers' QPC positions onto the recording timeline — pass
    /// the same origin the video capture uses.
    pub fn start(clock: RelativeClock) -> Result<Self, CaptureError> {
        Self::start_output(clock, None, 1.0)
    }

    pub fn start_output(
        clock: RelativeClock,
        device_id: Option<&str>,
        volume: f64,
    ) -> Result<Self, CaptureError> {
        Self::from_pcm(WasapiPcmCapture::start_output(clock, device_id, volume)?)
    }

    pub fn start_microphone(
        clock: RelativeClock,
        device_id: Option<&str>,
        volume: f64,
        channels: WasapiChannelMode,
    ) -> Result<Self, CaptureError> {
        Self::from_pcm(WasapiPcmCapture::start_microphone(
            clock, device_id, volume, channels,
        )?)
    }

    fn from_pcm(pcm: WasapiPcmCapture) -> Result<Self, CaptureError> {
        Ok(Self {
            pcm,
            opus: OpusFrameEncoder::new().map_err(|e| CaptureError::Init(format!("opus: {e}")))?,
            queue: Vec::new(),
        })
    }

    pub fn take_level(&mut self) -> AudioLevel {
        self.pcm.take_level()
    }

    pub fn poll_monitor_chunk(&mut self) -> Result<WasapiMonitorChunk, CaptureError> {
        let samples = self
            .pcm
            .poll_frames(f64::MAX)?
            .into_iter()
            .flat_map(|(_, frame)| frame)
            .collect();
        Ok(WasapiMonitorChunk {
            level: self.pcm.take_level(),
            samples,
        })
    }
}

impl WasapiMixedLoopback {
    pub fn start(
        clock: RelativeClock,
        output: Option<(Option<&str>, f64)>,
        microphone: Option<(Option<&str>, f64, WasapiChannelMode)>,
    ) -> Result<Self, CaptureError> {
        let mut sources = Vec::new();
        if let Some((device_id, volume)) = output {
            sources.push(WasapiPcmCapture::start_output(clock, device_id, volume)?);
        }
        if let Some((device_id, volume, channels)) = microphone {
            sources.push(WasapiPcmCapture::start_microphone(
                clock, device_id, volume, channels,
            )?);
        }
        if sources.is_empty() {
            return Err(CaptureError::Init(
                "mixed WASAPI source needs at least one input".into(),
            ));
        }
        let source_count = sources.len();
        let pending = (0..source_count).map(|_| VecDeque::new()).collect();
        Ok(Self {
            sources,
            pending,
            source_ready_until_s: vec![0.0; source_count],
            mixed_until_s: 0.0,
            opus: OpusFrameEncoder::new().map_err(|e| CaptureError::Init(format!("opus: {e}")))?,
            queue: Vec::new(),
        })
    }

    fn drop_consumed_frames(&mut self) {
        for pending in &mut self.pending {
            while pending
                .front()
                .is_some_and(|(pts_s, _)| pts_s + FRAME_DURATION_S <= self.mixed_until_s + 1e-9)
            {
                pending.pop_front();
            }
        }
    }

    fn next_pending_pts(&self) -> Option<f64> {
        self.pending
            .iter()
            .filter_map(|pending| pending.front().map(|(pts_s, _)| *pts_s))
            .min_by(|a, b| a.total_cmp(b))
    }

    fn push_mixed_packets(&mut self, until_pts_s: f64) -> Result<(), CaptureError> {
        self.drop_consumed_frames();
        while let Some(pts_s) = self.next_pending_pts() {
            if pts_s + FRAME_DURATION_S <= self.mixed_until_s + 1e-9 {
                self.drop_consumed_frames();
                continue;
            }
            let all_sources_ready = self.pending.iter().zip(&self.source_ready_until_s).all(
                |(pending, &ready_until_s)| {
                    source_ready_for_mix(pending, ready_until_s, pts_s, until_pts_s)
                },
            );
            if !all_sources_ready {
                break;
            }
            let mut frames = Vec::with_capacity(self.pending.len());
            for pending in &mut self.pending {
                let should_pop = pending.front().is_some_and(|(frame_pts_s, _)| {
                    (*frame_pts_s - pts_s).abs() <= MIX_FRAME_EPSILON_S
                });
                let frame = if should_pop {
                    Some(pending.pop_front().expect("checked front").1)
                } else {
                    None
                };
                frames.push(frame);
            }
            let frame = mix_optional_frames(&frames);
            let data = self
                .opus
                .encode_frame(&frame)
                .map_err(|e| CaptureError::DeviceLost(format!("opus encode: {e}")))?;
            self.queue.push(AudioPacket {
                data,
                pts_s,
                duration_s: FRAME_DURATION_S,
            });
            self.mixed_until_s = pts_s + FRAME_DURATION_S;
        }
        Ok(())
    }
}

impl AudioSource for WasapiMixedLoopback {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError> {
        for ((source, pending), ready_until_s) in self
            .sources
            .iter_mut()
            .zip(&mut self.pending)
            .zip(&mut self.source_ready_until_s)
        {
            let frames = source.poll_frames(until_pts_s)?;
            for (pts_s, _) in &frames {
                *ready_until_s = ready_until_s.max(pts_s + FRAME_DURATION_S);
            }
            pending.extend(frames);
        }
        self.push_mixed_packets(until_pts_s)?;
        let split = self
            .queue
            .iter()
            .position(|p| p.pts_s + p.duration_s > until_pts_s + 1e-9)
            .unwrap_or(self.queue.len());
        Ok(self.queue.drain(..split).collect())
    }

    fn track_config(&self) -> AudioTrackConfig {
        self.opus.track_config()
    }
}

fn source_ready_for_mix(
    pending: &VecDeque<PcmFrame>,
    ready_until_s: f64,
    target_pts_s: f64,
    until_pts_s: f64,
) -> bool {
    if pending.front().is_some() {
        return true;
    }
    ready_until_s >= target_pts_s + FRAME_DURATION_S - 1e-9
        || until_pts_s >= target_pts_s + MISSING_SOURCE_GRACE_S
}

fn mix_optional_frames(frames: &[Option<Vec<f32>>]) -> Vec<f32> {
    let mut mixed = vec![0.0; FRAME_LEN];
    for frame in frames.iter().filter_map(|frame| frame.as_ref()) {
        for (out, sample) in mixed.iter_mut().zip(frame.iter().copied()) {
            *out += sample;
        }
    }
    for sample in &mut mixed {
        *sample = sample.clamp(-1.0, 1.0);
    }
    mixed
}

pub fn enumerate_audio_devices() -> Result<AudioDeviceList, CaptureError> {
    init_com()?;
    // SAFETY: standard MMDevice enumeration; all COM results are checked.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(init)?;
        Ok(AudioDeviceList {
            outputs: enumerate_endpoints(&enumerator, eRender)?,
            inputs: enumerate_endpoints(&enumerator, eCapture)?,
        })
    }
}

pub fn test_microphone_level(
    device_id: Option<&str>,
    volume: f64,
    channels: WasapiChannelMode,
    duration: Duration,
) -> Result<AudioLevel, CaptureError> {
    let clock = RelativeClock::new(super::qpc_now_ticks_100ns().map_err(init)?);
    let mut source = WasapiLoopback::start_microphone(clock, device_id, volume, channels)?;
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        let _ = source.poll_packets(f64::MAX)?;
    }
    let _ = source.poll_packets(f64::MAX)?;
    Ok(source.take_level())
}

fn endpoint_device(
    enumerator: &IMMDeviceEnumerator,
    dataflow: EDataFlow,
    device_id: Option<&str>,
) -> windows::core::Result<IMMDevice> {
    // SAFETY: the optional PCWSTR is null-terminated for the duration of GetDevice.
    unsafe {
        if let Some(id) = device_id.filter(|id| !id.trim().is_empty()) {
            let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
            enumerator.GetDevice(PCWSTR(wide.as_ptr()))
        } else {
            enumerator.GetDefaultAudioEndpoint(dataflow, eConsole)
        }
    }
}

fn enumerate_endpoints(
    enumerator: &IMMDeviceEnumerator,
    dataflow: EDataFlow,
) -> Result<Vec<AudioDeviceInfo>, CaptureError> {
    // SAFETY: collection count and indexed access are checked by the COM methods.
    unsafe {
        let default_id = enumerator
            .GetDefaultAudioEndpoint(dataflow, eConsole)
            .ok()
            .and_then(|device| device_id_string(&device).ok());
        let collection = enumerator
            .EnumAudioEndpoints(dataflow, DEVICE_STATE_ACTIVE)
            .map_err(init)?;
        let mut devices = Vec::new();
        for i in 0..collection.GetCount().map_err(init)? {
            let device = collection.Item(i).map_err(init)?;
            let id = device_id_string(&device)?;
            let name = friendly_name(&device).unwrap_or_else(|| id.clone());
            devices.push(AudioDeviceInfo {
                is_default: default_id.as_deref() == Some(id.as_str()),
                id,
                name,
            });
        }
        Ok(devices)
    }
}

fn device_id_string(device: &IMMDevice) -> Result<String, CaptureError> {
    // SAFETY: IMMDevice::GetId returns a CoTaskMem-allocated null-terminated string.
    unsafe {
        let raw = device.GetId().map_err(init)?;
        pwstr_to_string_and_free(raw)
            .map_err(|e| CaptureError::Init(format!("device id utf16: {e}")))
    }
}

fn friendly_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: property store and PROPVARIANT lifecycle follow the Windows API contract.
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut prop = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let mut buf = [0u16; 256];
        let result = PropVariantToString(&prop, &mut buf)
            .ok()
            .map(|_| utf16z_from_buf(&buf))
            .filter(|s| !s.trim().is_empty());
        let _ = PropVariantClear(&mut prop);
        result
    }
}

fn pwstr_to_string_and_free(raw: PWSTR) -> Result<String, std::string::FromUtf16Error> {
    // SAFETY: callers pass PWSTRs returned by Windows APIs and release them with CoTaskMemFree.
    let value = unsafe { raw.to_string() };
    unsafe { CoTaskMemFree(Some(raw.0 as *const _)) };
    value
}

fn utf16z_from_buf(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

impl AudioSource for WasapiLoopback {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError> {
        for (pts_s, frame) in self.pcm.poll_frames(until_pts_s)? {
            let data = self
                .opus
                .encode_frame(&frame)
                .map_err(|e| CaptureError::DeviceLost(format!("opus encode: {e}")))?;
            self.queue.push(AudioPacket {
                data,
                pts_s,
                duration_s: FRAME_DURATION_S,
            });
        }
        // Mock semantics: every packet that ends at or before `until`.
        let split = self
            .queue
            .iter()
            .position(|p| p.pts_s + p.duration_s > until_pts_s + 1e-9)
            .unwrap_or(self.queue.len());
        Ok(self.queue.drain(..split).collect())
    }

    fn track_config(&self) -> AudioTrackConfig {
        self.opus.track_config()
    }
}

fn parse_mix_format(format: &WAVEFORMATEX) -> Option<MixFormat> {
    // Copy packed fields to locals (references into packed structs are UB).
    let channels = format.nChannels;
    let rate = format.nSamplesPerSec;
    let bits = format.wBitsPerSample;
    if channels == 0 || rate == 0 {
        return None;
    }
    let tag = format.wFormatTag as u32;
    let sample_format = match tag {
        WAVE_FORMAT_IEEE_FLOAT if bits == 32 => SampleFormat::Float32,
        WAVE_FORMAT_PCM => pcm_sample_format(bits)?,
        WAVE_FORMAT_EXTENSIBLE => {
            // SAFETY: extensible tag guarantees the larger layout.
            let ext = unsafe { &*(format as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE) };
            let sub = ext.SubFormat;
            if sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT && bits == 32 {
                SampleFormat::Float32
            } else if sub == KSDATAFORMAT_SUBTYPE_PCM {
                pcm_sample_format(bits)?
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(MixFormat {
        channels,
        sample_rate: rate,
        sample_format,
    })
}

fn pcm_sample_format(bits: u16) -> Option<SampleFormat> {
    match bits {
        16 => Some(SampleFormat::Pcm16),
        24 => Some(SampleFormat::Pcm24),
        32 => Some(SampleFormat::Pcm32),
        _ => None,
    }
}

/// Best-effort COM init (MTA); an STA thread is fine too.
fn init_com() -> Result<(), CaptureError> {
    // SAFETY: CoInitializeEx is safe to call repeatedly per thread.
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_ok() || hr == RPC_E_CHANGED_MODE {
        Ok(())
    } else {
        Err(CaptureError::Init(format!("CoInitializeEx: {hr}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::RelativeClock;
    use crate::traits::AudioSource;

    /// Real loopback against the default render endpoint. CI-skipped (no
    /// audio endpoint on runners); lenient about an idle/silent desktop —
    /// the assembler's gap fill makes silence a valid outcome.
    #[test]
    fn captures_system_loopback_audio() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: audio endpoint test");
            return;
        }
        let clock = RelativeClock::new(crate::windows::qpc_now_ticks_100ns().unwrap());
        let mut src = match WasapiLoopback::start(clock) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("SKIP: loopback unavailable: {e}");
                return;
            }
        };
        let cfg = src.track_config();
        assert_eq!((cfg.channels, cfg.sample_rate), (2, 48_000));
        assert!(cfg.pre_skip > 0);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let packets = src.poll_packets(f64::MAX).expect("poll");
        for w in packets.windows(2) {
            assert!(
                (w[1].pts_s - w[0].pts_s - 0.02).abs() < 1e-6,
                "20 ms cadence"
            );
        }
        for p in &packets {
            assert!(!p.data.is_empty());
        }
        eprintln!("captured {} opus packets in 300 ms", packets.len());
    }

    #[test]
    fn enumerates_audio_endpoints_when_available() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: audio endpoint test");
            return;
        }
        let devices = match enumerate_audio_devices() {
            Ok(devices) => devices,
            Err(e) => {
                eprintln!("SKIP: audio endpoint enumeration unavailable: {e}");
                return;
            }
        };
        for device in devices.outputs.iter().chain(devices.inputs.iter()) {
            assert!(!device.id.is_empty());
            assert!(!device.name.is_empty());
        }
    }

    #[test]
    fn tests_default_microphone_level_when_available() {
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIP: audio endpoint test");
            return;
        }
        let level = match test_microphone_level(
            None,
            1.0,
            WasapiChannelMode::Mono,
            std::time::Duration::from_millis(300),
        ) {
            Ok(level) => level,
            Err(e) => {
                eprintln!("SKIP: microphone unavailable: {e}");
                return;
            }
        };
        assert!(level.sample_count > 0, "microphone delivered samples");
    }

    #[test]
    fn mixed_frames_sum_tracks_and_clamp() {
        let mut output = vec![0.0; FRAME_LEN];
        let mut mic = vec![0.0; FRAME_LEN];
        output[0] = 0.75;
        output[1] = -0.75;
        mic[0] = 0.5;
        mic[1] = -0.5;

        let mixed = mix_optional_frames(&[Some(output), Some(mic), None]);

        assert_eq!(mixed.len(), FRAME_LEN);
        assert_eq!(mixed[0], 1.0);
        assert_eq!(mixed[1], -1.0);
        assert!(mixed[2..].iter().all(|&sample| sample == 0.0));
    }

    #[test]
    fn mixed_source_waits_briefly_for_late_frames() {
        let pending = VecDeque::new();

        assert!(
            !source_ready_for_mix(&pending, 0.0, 1.0, 1.02),
            "missing source should not be treated as silence immediately"
        );
        assert!(
            source_ready_for_mix(&pending, 0.0, 1.0, 1.08),
            "missing source becomes silence after the grace window"
        );
    }

    #[test]
    fn mixed_source_accepts_sources_that_advanced_past_a_frame() {
        let pending = VecDeque::new();

        assert!(
            source_ready_for_mix(&pending, 1.04, 1.0, 1.01),
            "a source that advanced past the target will not deliver it later"
        );
    }
}
