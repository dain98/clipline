//! System-audio capture: WASAPI loopback on the default render endpoint
//! (ddoc §10), QPC-stamped against the shared capture clock, assembled
//! into 20 ms frames and Opus-encoded behind `AudioSource`.

use std::mem::{size_of, ManuallyDrop};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use windows::core::{implement, Interface, Ref, Result as WindowsResult, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, RPC_E_CHANGED_MODE};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, ActivateAudioInterfaceAsync, AudioSessionStateExpired, EDataFlow,
    IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
    IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    DEVICE_STATE_ACTIVE, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVE_FORMAT_PCM,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PropVariantToString, PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0,
    PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemAlloc, CoTaskMemFree, IAgileObject,
    IAgileObject_Impl, BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::Variant::VT_BLOB;

use clipline_mp4::AudioTrackConfig;

use crate::clock::RelativeClock;
use crate::opus::{OpusFrameEncoder, FRAME_DURATION_S};
use crate::pcm::{
    apply_gain, extract_mono_centered, extract_stereo, LoopbackAssembler, PcmFrame, PcmFrameMixer,
    StereoResampler,
};
use crate::traits::{AudioPacket, AudioSource, CaptureError};

const OPUS_SAMPLE_RATE: u32 = 48_000;

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

#[derive(Debug, Clone)]
pub struct AudioProcessInfo {
    pub pid: u32,
    pub label: String,
    pub process_name: Option<String>,
    pub process_path: Option<String>,
}

#[derive(Debug, Clone)]
struct ProcessSnapshotEntry {
    parent_pid: u32,
    process_path: Option<String>,
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

fn wasapi_timestamp_valid(flags: u32) -> bool {
    flags & (AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32) == 0
}

fn wasapi_data_discontinuous(flags: u32) -> bool {
    flags & (AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32) != 0
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
    event_handle: Option<HANDLE>,
    clock: RelativeClock,
    channels: u16,
    sample_format: SampleFormat,
    mode: EndpointMode,
    volume: f32,
    level: AudioLevelAccumulator,
    resampler: Option<StereoResampler>,
    assembler: LoopbackAssembler,
    queue: std::collections::VecDeque<PcmFrame>,
}

pub struct WasapiLoopback {
    pcm: WasapiPcmCapture,
    opus: OpusFrameEncoder,
    queue: Vec<AudioPacket>,
}

pub struct WasapiMixedLoopback {
    sources: Vec<WasapiPcmCapture>,
    mixer: PcmFrameMixer,
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

    fn start_process_output(
        clock: RelativeClock,
        pid: u32,
        volume: f64,
    ) -> Result<Self, CaptureError> {
        init_com()?;
        let client = activate_process_loopback_client(pid)?;
        Self::start_client(
            clock,
            client,
            AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
            volume,
            EndpointMode::OutputLoopback,
            0,
            Some(process_loopback_format()),
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

            Self::start_client(clock, client, streamflags, volume, mode, 10_000_000, None)
        }
    }

    fn start_client(
        clock: RelativeClock,
        client: IAudioClient,
        streamflags: u32,
        volume: f64,
        mode: EndpointMode,
        buffer_duration_100ns: i64,
        fixed_mix_format: Option<WAVEFORMATEX>,
    ) -> Result<Self, CaptureError> {
        // SAFETY: IAudioClient initialization follows the WASAPI contract and
        // releases the mix-format allocation after Initialize consumes it.
        unsafe {
            let mut fixed_mix_format = fixed_mix_format;
            let (format_ptr, should_free_format) = if let Some(format) = fixed_mix_format.as_mut() {
                (format as *mut WAVEFORMATEX, false)
            } else {
                (client.GetMixFormat().map_err(init)?, true)
            };
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
                buffer_duration_100ns,
                0,
                format_ptr,
                None,
            );
            if should_free_format {
                CoTaskMemFree(Some(format_ptr as *const _));
            }
            r.map_err(|e| CaptureError::Init(format!("WASAPI Initialize: {e}")))?;

            let event_handle = if streamflags & AUDCLNT_STREAMFLAGS_EVENTCALLBACK != 0 {
                let handle = CreateEventW(None, false, false, PCWSTR::null())
                    .map_err(|e| CaptureError::Init(format!("WASAPI CreateEventW: {e}")))?;
                if let Err(error) = client.SetEventHandle(handle) {
                    let _ = CloseHandle(handle);
                    return Err(CaptureError::Init(format!(
                        "WASAPI SetEventHandle: {error}"
                    )));
                }
                Some(handle)
            } else {
                None
            };

            let capture: IAudioCaptureClient = client
                .GetService()
                .map_err(|e| CaptureError::Init(format!("WASAPI GetService: {e}")))?;
            client
                .Start()
                .map_err(|e| CaptureError::Init(format!("WASAPI Start: {e}")))?;

            // Anchor the audio timeline at the clock origin (recording
            // start): the gap fill turns any lead-in before the first
            // device buffer into silence, keeping the muxed track aligned
            // with video (both tracks start at t=0 in the file).
            let mut assembler = LoopbackAssembler::new();
            assembler.push_chunk(0.0, &[]);

            Ok(Self {
                client,
                capture,
                event_handle,
                clock,
                channels: mix.channels,
                sample_format: mix.sample_format,
                mode,
                volume: (volume.clamp(0.0, 2.0)) as f32,
                level: AudioLevelAccumulator::default(),
                resampler: (mix.sample_rate != OPUS_SAMPLE_RATE)
                    .then(|| StereoResampler::new(mix.sample_rate, OPUS_SAMPLE_RATE)),
                assembler,
                queue: std::collections::VecDeque::new(),
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
        if let Some(resampler) = &mut self.resampler {
            stereo = resampler.resample(&stereo);
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
                let timestamp_valid = wasapi_timestamp_valid(flags);
                let data_discontinuous = wasapi_data_discontinuous(flags);
                let pts_s = timestamp_valid.then(|| self.clock.pts_s(qpc_100ns as i64));
                let n = frames as usize * self.channels as usize;
                let samples: Vec<f32> = if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
                    vec![0.0; n]
                } else {
                    self.decode_samples(data as *const u8, frames)
                };
                self.capture.ReleaseBuffer(frames).map_err(lost)?;
                let stereo = self.stereo_samples(&samples);
                if let Some(pts_s) = pts_s {
                    self.assembler.push_chunk(pts_s, &stereo);
                } else {
                    self.assembler.push_contiguous_chunk(&stereo);
                }
                if data_discontinuous {
                    eprintln!("WASAPI data discontinuity; audio gap fill capped");
                }
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
        if let Some(handle) = self.event_handle.take() {
            let _ = unsafe { CloseHandle(handle) };
        }
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

    pub fn start_process_output(
        clock: RelativeClock,
        pid: u32,
        volume: f64,
    ) -> Result<Self, CaptureError> {
        Self::from_pcm(WasapiPcmCapture::start_process_output(clock, pid, volume)?)
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
        let mixer = PcmFrameMixer::new(sources.len());
        Ok(Self {
            sources,
            mixer,
            opus: OpusFrameEncoder::new().map_err(|e| CaptureError::Init(format!("opus: {e}")))?,
            queue: Vec::new(),
        })
    }

    fn push_mixed_packets(&mut self, until_pts_s: f64) -> Result<(), CaptureError> {
        for (pts_s, frame) in self.mixer.pop_mixed_frames(until_pts_s) {
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
        Ok(())
    }
}

impl AudioSource for WasapiMixedLoopback {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError> {
        for (source_index, source) in self.sources.iter_mut().enumerate() {
            let frames = source.poll_frames(until_pts_s)?;
            self.mixer.push_source_frames(source_index, frames);
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

pub fn enumerate_output_processes(
    device_id: Option<&str>,
) -> Result<Vec<AudioProcessInfo>, CaptureError> {
    init_com()?;
    // SAFETY: standard endpoint activation/session enumeration; COM results
    // are checked and any allocated strings are freed.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(init)?;
        let device = endpoint_device(&enumerator, eRender, device_id).map_err(init)?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).map_err(init)?;
        let session_enum = manager.GetSessionEnumerator().map_err(init)?;
        let process_snapshot = process_snapshot();
        let mut processes = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for index in 0..session_enum.GetCount().map_err(init)? {
            let Ok(session) = session_enum.GetSession(index) else {
                continue;
            };
            if session.GetState().ok() == Some(AudioSessionStateExpired) {
                continue;
            }
            let Ok(session2) = session.cast::<IAudioSessionControl2>() else {
                continue;
            };
            let pid = session2.GetProcessId().unwrap_or_default();
            if pid == 0 {
                continue;
            }
            let display_name = session
                .GetDisplayName()
                .ok()
                .and_then(|raw| pwstr_to_optional_string_and_free(raw).ok().flatten());
            let session_process_path = process_image_path(pid).or_else(|| {
                process_snapshot
                    .get(&pid)
                    .and_then(|entry| entry.process_path.clone())
            });
            let capture_pid =
                process_group_root(pid, session_process_path.as_deref(), &process_snapshot);
            if !seen.insert(capture_pid) {
                continue;
            }
            let process_path = process_image_path(capture_pid)
                .or_else(|| {
                    (capture_pid == pid)
                        .then(|| session_process_path.clone())
                        .flatten()
                })
                .or_else(|| {
                    process_snapshot
                        .get(&capture_pid)
                        .and_then(|entry| entry.process_path.clone())
                });
            let process_name = process_path
                .as_deref()
                .and_then(process_name_from_path)
                .or_else(|| display_name.clone());
            let label = display_name
                .filter(|name| !name.trim().is_empty())
                .or_else(|| process_name.clone())
                .unwrap_or_else(|| format!("Process {capture_pid}"));
            processes.push(AudioProcessInfo {
                pid: capture_pid,
                label,
                process_name,
                process_path,
            });
        }
        drop_duplicate_process_tree_ancestors(&mut processes, &process_snapshot);
        processes.sort_by(|a, b| {
            a.label
                .to_lowercase()
                .cmp(&b.label.to_lowercase())
                .then_with(|| a.pid.cmp(&b.pid))
        });
        Ok(processes)
    }
}

pub fn process_loopback_available() -> bool {
    // Per-process application loopback (ActivateAudioInterfaceAsync with
    // AUDIOCLIENT_PROCESS_LOOPBACK) is *documented* as Windows 10 build 20348+,
    // but in practice works on fully updated Windows 10 2004+ (build 19041):
    // OBS's Application Audio Capture relies on exactly this API there, and we
    // deliberately target it too (see ddoc.md). Below 2004 the activation fails
    // or its completion callback never fires — but `activate_process_loopback_client`
    // caps the wait at 1.5s and `add_output_audio_sources` falls back to
    // full-system mixed output, so attempting it on an unsupported build costs at
    // most one bounded stall. This gate only skips that pointless attempt on
    // pre-2004 builds; do not raise it to 20348 without revisiting that tradeoff.
    const MIN_PROCESS_LOOPBACK_BUILD: u32 = 19_041;
    windows_build_number().is_some_and(|build| build >= MIN_PROCESS_LOOPBACK_BUILD)
}

/// The OS build number via `RtlGetVersion` (the manifest-independent source of
/// truth). `None` if the query somehow fails.
fn windows_build_number() -> Option<u32> {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: RtlGetVersion fills the OSVERSIONINFOW we own; its size is set and
    // the call returns STATUS_SUCCESS on all supported systems.
    let status = unsafe { RtlGetVersion(&mut info) };
    status.is_ok().then_some(info.dwBuildNumber)
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
            enumerator
                .GetDevice(PCWSTR(wide.as_ptr()))
                .or_else(|_| enumerator.GetDefaultAudioEndpoint(dataflow, eConsole))
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

#[derive(Default)]
struct ProcessLoopbackActivationState {
    completed: Mutex<bool>,
    ready: Condvar,
}

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct ProcessLoopbackActivation {
    state: Arc<ProcessLoopbackActivationState>,
}

impl IAgileObject_Impl for ProcessLoopbackActivation_Impl {}

#[allow(non_snake_case)]
impl IActivateAudioInterfaceCompletionHandler_Impl for ProcessLoopbackActivation_Impl {
    fn ActivateCompleted(
        &self,
        _activateoperation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> WindowsResult<()> {
        let mut guard = self.state.completed.lock().expect("activation mutex");
        *guard = true;
        self.state.ready.notify_one();
        Ok(())
    }
}

fn activate_process_loopback_client(pid: u32) -> Result<IAudioClient, CaptureError> {
    let state = Arc::new(ProcessLoopbackActivationState::default());
    let handler: IActivateAudioInterfaceCompletionHandler = ProcessLoopbackActivation {
        state: Arc::clone(&state),
    }
    .into();

    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let params_size = std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>();
    // SAFETY: CoTaskMemAlloc returns an allocation suitable for PROPVARIANT
    // VT_BLOB ownership. The bytes copied are exactly AUDIOCLIENT_ACTIVATION_PARAMS.
    let params_blob = unsafe { CoTaskMemAlloc(params_size) };
    if params_blob.is_null() {
        return Err(CaptureError::Init(
            "WASAPI process loopback activation params allocation failed".into(),
        ));
    }
    // SAFETY: params_blob is a valid params_size allocation and params is live.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&params as *const AUDIOCLIENT_ACTIVATION_PARAMS).cast::<u8>(),
            params_blob.cast::<u8>(),
            params_size,
        );
    }
    let mut variant = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: params_size as u32,
                        pBlobData: params_blob.cast::<u8>(),
                    },
                },
            }),
        },
    };

    // SAFETY: the activation parameter PROPVARIANT owns its blob payload and is
    // valid for the duration of ActivateAudioInterfaceAsync.
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&variant),
            &handler,
        )
    };
    let operation = match operation {
        Ok(operation) => operation,
        Err(error) => {
            // SAFETY: clears the owned blob allocated by InitPropVariantFromBuffer.
            let _ = unsafe { PropVariantClear(&mut variant) };
            return Err(init(error));
        }
    };

    let deadline = Instant::now() + Duration::from_millis(1500);
    let mut guard = state.completed.lock().expect("activation mutex");
    loop {
        if *guard {
            drop(guard);
            let mut activate_result = HRESULT(0);
            let mut activated_interface = None;
            // SAFETY: the operation has signaled completion. The HRESULT and
            // returned interface are checked before use.
            if let Err(error) = unsafe {
                operation.GetActivateResult(&mut activate_result, &mut activated_interface)
            } {
                // SAFETY: clears the owned activation blob before returning.
                let _ = unsafe { PropVariantClear(&mut variant) };
                return Err(CaptureError::Init(format!(
                    "WASAPI GetActivateResult: {error}"
                )));
            }
            if let Err(error) = activate_result.ok() {
                // SAFETY: clears the owned activation blob before returning.
                let _ = unsafe { PropVariantClear(&mut variant) };
                return Err(CaptureError::Init(format!(
                    "WASAPI activation result: {error}"
                )));
            }
            let client = match activated_interface
                .ok_or_else(|| CaptureError::Init("WASAPI: activation returned no client".into()))
                .and_then(|unknown| {
                    unknown
                        .cast::<IAudioClient>()
                        .map_err(|e| CaptureError::Init(format!("WASAPI activation cast: {e}")))
                }) {
                Ok(client) => client,
                Err(error) => {
                    // SAFETY: clears the owned activation blob before returning.
                    let _ = unsafe { PropVariantClear(&mut variant) };
                    return Err(error);
                }
            };
            // SAFETY: activation is complete, so the owned activation blob can be released.
            let _ = unsafe { PropVariantClear(&mut variant) };
            return Ok(client);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            // SAFETY: clears the owned blob allocated by InitPropVariantFromBuffer.
            let _ = unsafe { PropVariantClear(&mut variant) };
            return Err(CaptureError::Init(format!(
                "WASAPI process loopback activation timed out for pid {pid}"
            )));
        };
        let (next_guard, timeout) = state
            .ready
            .wait_timeout(guard, remaining)
            .expect("activation result condvar");
        guard = next_guard;
        if timeout.timed_out() && !*guard {
            // SAFETY: clears the owned blob allocated by InitPropVariantFromBuffer.
            let _ = unsafe { PropVariantClear(&mut variant) };
            return Err(CaptureError::Init(format!(
                "WASAPI process loopback activation timed out for pid {pid}"
            )));
        }
    }
}

fn process_image_path(pid: u32) -> Option<String> {
    // SAFETY: the process handle is closed before return, and the query buffer
    // is valid for the duration of the call.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = vec![0u16; 32_768];
        let mut len = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        (!path.trim().is_empty()).then_some(path)
    }
}

fn process_snapshot() -> std::collections::HashMap<u32, ProcessSnapshotEntry> {
    let mut processes = std::collections::HashMap::new();
    // SAFETY: snapshot handle is closed before return; PROCESSENTRY32W is
    // initialized with the required size before ToolHelp reads into it.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return processes;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..PROCESSENTRY32W::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let pid = entry.th32ProcessID;
                if pid != 0 {
                    let fallback_name = utf16z_from_buf(&entry.szExeFile);
                    processes.insert(
                        pid,
                        ProcessSnapshotEntry {
                            parent_pid: entry.th32ParentProcessID,
                            process_path: (!fallback_name.trim().is_empty())
                                .then_some(fallback_name),
                        },
                    );
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    processes
}

fn process_group_root(
    pid: u32,
    process_path: Option<&str>,
    snapshot: &std::collections::HashMap<u32, ProcessSnapshotEntry>,
) -> u32 {
    let mut current_pid = pid;
    let mut current_path = process_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .or_else(|| {
            snapshot
                .get(&pid)
                .and_then(|entry| entry.process_path.clone())
        });

    for parent_pid in process_parent_pids(pid, snapshot) {
        let Some(path) = current_path.as_deref() else {
            break;
        };
        let Some(parent) = snapshot.get(&parent_pid) else {
            break;
        };
        let Some(parent_path) = parent.process_path.as_deref() else {
            break;
        };
        if !same_process_image(path, parent_path) {
            break;
        }
        current_pid = parent_pid;
        current_path = Some(parent_path.to_string());
    }

    current_pid
}

fn drop_duplicate_process_tree_ancestors(
    processes: &mut Vec<AudioProcessInfo>,
    snapshot: &std::collections::HashMap<u32, ProcessSnapshotEntry>,
) {
    // Keep the child app's split track label and drop launcher parents whose
    // process-tree capture would duplicate the child. Parent-owned launcher
    // sounds remain available in the mixed Output Audio safety track.
    let duplicate_ancestors: std::collections::HashSet<u32> = processes
        .iter()
        .filter(|candidate| {
            processes.iter().any(|other| {
                candidate.pid != other.pid
                    && process_is_ancestor(candidate.pid, other.pid, snapshot)
                    && process_images_differ(candidate, other, snapshot)
            })
        })
        .map(|process| process.pid)
        .collect();
    processes.retain(|process| !duplicate_ancestors.contains(&process.pid));
}

fn process_is_ancestor(
    ancestor_pid: u32,
    descendant_pid: u32,
    snapshot: &std::collections::HashMap<u32, ProcessSnapshotEntry>,
) -> bool {
    process_parent_pids(descendant_pid, snapshot).contains(&ancestor_pid)
}

fn process_parent_pids(
    pid: u32,
    snapshot: &std::collections::HashMap<u32, ProcessSnapshotEntry>,
) -> Vec<u32> {
    let mut parent_pids = Vec::new();
    let mut current_pid = pid;
    let mut visited = std::collections::HashSet::from([pid]);
    while let Some(current) = snapshot.get(&current_pid) {
        let parent_pid = current.parent_pid;
        if parent_pid == 0 || !visited.insert(parent_pid) {
            break;
        }
        parent_pids.push(parent_pid);
        current_pid = parent_pid;
    }
    parent_pids
}

fn process_images_differ(
    a: &AudioProcessInfo,
    b: &AudioProcessInfo,
    snapshot: &std::collections::HashMap<u32, ProcessSnapshotEntry>,
) -> bool {
    match (
        process_path_for(a.pid, a.process_path.as_deref(), snapshot),
        process_path_for(b.pid, b.process_path.as_deref(), snapshot),
    ) {
        (Some(a_path), Some(b_path)) => !same_process_image(a_path, b_path),
        _ => {
            let Some(a_name) = process_identity_name(a, snapshot) else {
                return false;
            };
            let Some(b_name) = process_identity_name(b, snapshot) else {
                return false;
            };
            !a_name.eq_ignore_ascii_case(&b_name)
        }
    }
}

fn process_path_for<'a>(
    pid: u32,
    path: Option<&'a str>,
    snapshot: &'a std::collections::HashMap<u32, ProcessSnapshotEntry>,
) -> Option<&'a str> {
    path.or_else(|| {
        snapshot
            .get(&pid)
            .and_then(|entry| entry.process_path.as_deref())
    })
}

fn process_identity_name(
    process: &AudioProcessInfo,
    snapshot: &std::collections::HashMap<u32, ProcessSnapshotEntry>,
) -> Option<String> {
    process_path_for(process.pid, process.process_path.as_deref(), snapshot)
        .and_then(process_name_from_path)
        .or_else(|| {
            process
                .process_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
}

fn same_process_image(a: &str, b: &str) -> bool {
    let a = a.trim();
    let b = b.trim();
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    match (process_name_from_path(a), process_name_from_path(b)) {
        (Some(a_name), Some(b_name)) => a_name.eq_ignore_ascii_case(&b_name),
        _ => false,
    }
}

fn process_name_from_path(path: &str) -> Option<String> {
    let file_name = Path::new(path)
        .file_stem()
        .or_else(|| Path::new(path).file_name())?
        .to_string_lossy();
    let trimmed = file_name.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn pwstr_to_string_and_free(raw: PWSTR) -> Result<String, std::string::FromUtf16Error> {
    // SAFETY: callers pass PWSTRs returned by Windows APIs and release them with CoTaskMemFree.
    let value = unsafe { raw.to_string() };
    unsafe { CoTaskMemFree(Some(raw.0 as *const _)) };
    value
}

fn pwstr_to_optional_string_and_free(
    raw: PWSTR,
) -> Result<Option<String>, std::string::FromUtf16Error> {
    if raw.0.is_null() {
        return Ok(None);
    }
    pwstr_to_string_and_free(raw).map(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
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

fn process_loopback_format() -> WAVEFORMATEX {
    const CHANNELS: u16 = 2;
    const BITS_PER_SAMPLE: u16 = 16;
    const SAMPLE_RATE: u32 = 44_100;
    let block_align = CHANNELS * (BITS_PER_SAMPLE / 8);
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: CHANNELS,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * block_align as u32,
        nBlockAlign: block_align,
        wBitsPerSample: BITS_PER_SAMPLE,
        cbSize: 0,
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

    #[test]
    fn process_name_from_path_uses_executable_stem() {
        assert_eq!(
            process_name_from_path(r"C:\Program Files\Discord\Discord.exe").as_deref(),
            Some("Discord")
        );
        assert_eq!(process_name_from_path("").as_deref(), None);
    }

    #[test]
    fn wasapi_timestamp_error_flag_marks_timestamp_invalid() {
        assert!(wasapi_timestamp_valid(0));
        assert!(wasapi_timestamp_valid(
            AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32
        ));
        assert!(!wasapi_timestamp_valid(
            AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32
        ));
        assert!(wasapi_data_discontinuous(
            AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32
        ));
    }

    #[test]
    fn process_group_root_collapses_same_executable_children() {
        let snapshot = std::collections::HashMap::from([
            (
                10724,
                ProcessSnapshotEntry {
                    parent_pid: 1000,
                    process_path: Some(r"C:\Users\dain\AppData\Local\Discord\Discord.exe".into()),
                },
            ),
            (
                18736,
                ProcessSnapshotEntry {
                    parent_pid: 10724,
                    process_path: Some(r"C:\Users\dain\AppData\Local\Discord\Discord.exe".into()),
                },
            ),
            (
                20732,
                ProcessSnapshotEntry {
                    parent_pid: 10724,
                    process_path: Some(r"C:\Users\dain\AppData\Local\Discord\Discord.exe".into()),
                },
            ),
        ]);

        assert_eq!(
            process_group_root(
                18736,
                Some(r"C:\Users\dain\AppData\Local\Discord\Discord.exe"),
                &snapshot
            ),
            10724
        );
        assert_eq!(
            process_group_root(
                20732,
                Some(r"C:\Users\dain\AppData\Local\Discord\Discord.exe"),
                &snapshot
            ),
            10724
        );
    }

    #[test]
    fn process_group_root_stops_at_different_executable_parent() {
        let snapshot = std::collections::HashMap::from([
            (
                10,
                ProcessSnapshotEntry {
                    parent_pid: 1,
                    process_path: Some(r"C:\Launchers\Launcher.exe".into()),
                },
            ),
            (
                20,
                ProcessSnapshotEntry {
                    parent_pid: 10,
                    process_path: Some(r"C:\Games\Game.exe".into()),
                },
            ),
        ]);

        assert_eq!(
            process_group_root(20, Some(r"C:\Games\Game.exe"), &snapshot),
            20
        );
    }

    #[test]
    fn process_candidates_drop_launcher_parent_when_child_also_has_audio() {
        let snapshot = std::collections::HashMap::from([
            (
                10,
                ProcessSnapshotEntry {
                    parent_pid: 1,
                    process_path: Some(r"C:\Program Files\Steam\steam.exe".into()),
                },
            ),
            (
                20,
                ProcessSnapshotEntry {
                    parent_pid: 10,
                    process_path: Some(r"C:\Games\SlayTheSpire2.exe".into()),
                },
            ),
        ]);
        let mut processes = vec![
            AudioProcessInfo {
                pid: 10,
                label: "steam".into(),
                process_name: Some("steam".into()),
                process_path: Some(r"C:\Program Files\Steam\steam.exe".into()),
            },
            AudioProcessInfo {
                pid: 20,
                label: "SlayTheSpire2".into(),
                process_name: Some("SlayTheSpire2".into()),
                process_path: Some(r"C:\Games\SlayTheSpire2.exe".into()),
            },
        ];

        drop_duplicate_process_tree_ancestors(&mut processes, &snapshot);

        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].label, "SlayTheSpire2");
    }

    #[test]
    fn process_candidates_drop_launcher_parent_when_parent_path_is_unknown() {
        let snapshot = std::collections::HashMap::from([
            (
                10,
                ProcessSnapshotEntry {
                    parent_pid: 1,
                    process_path: None,
                },
            ),
            (
                20,
                ProcessSnapshotEntry {
                    parent_pid: 10,
                    process_path: Some(r"C:\Games\SlayTheSpire2.exe".into()),
                },
            ),
        ]);
        let mut processes = vec![
            AudioProcessInfo {
                pid: 10,
                label: "steam".into(),
                process_name: Some("steam".into()),
                process_path: None,
            },
            AudioProcessInfo {
                pid: 20,
                label: "SlayTheSpire2".into(),
                process_name: Some("SlayTheSpire2".into()),
                process_path: Some(r"C:\Games\SlayTheSpire2.exe".into()),
            },
        ];

        drop_duplicate_process_tree_ancestors(&mut processes, &snapshot);

        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].label, "SlayTheSpire2");
    }

    #[test]
    fn process_loopback_format_matches_windows_sample_pcm16() {
        let format = process_loopback_format();
        let tag = format.wFormatTag;
        let channels = format.nChannels;
        let sample_rate = format.nSamplesPerSec;
        let bits = format.wBitsPerSample;
        let block_align = format.nBlockAlign;
        assert_eq!(tag as u32, WAVE_FORMAT_PCM);
        assert_eq!(channels, 2);
        assert_eq!(sample_rate, 44_100);
        assert_eq!(bits, 16);
        assert_eq!(block_align, 4);
    }

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
}
