//! System-audio capture: WASAPI loopback on the default render endpoint
//! (ddoc §10), QPC-stamped against the shared capture clock, assembled
//! into 20 ms frames and Opus-encoded behind `AudioSource`.

use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::Media::KernelStreaming::WAVE_FORMAT_EXTENSIBLE;
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
};

use clipline_mp4::AudioTrackConfig;

use crate::clock::RelativeClock;
use crate::opus::{FRAME_DURATION_S, OpusFrameEncoder};
use crate::pcm::{extract_stereo, LoopbackAssembler};
use crate::traits::{AudioPacket, AudioSource, CaptureError};

pub struct WasapiLoopback {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    clock: RelativeClock,
    channels: u16,
    assembler: LoopbackAssembler,
    opus: OpusFrameEncoder,
    queue: Vec<AudioPacket>,
}

fn init(e: windows::core::Error) -> CaptureError {
    CaptureError::Init(format!("WASAPI: {e}"))
}

impl WasapiLoopback {
    /// Start capturing the default render endpoint in loopback. `clock`
    /// maps the buffers' QPC positions onto the recording timeline — pass
    /// the same origin the video capture uses (sub-ms apart until
    /// milestone 4 unifies them).
    pub fn start(clock: RelativeClock) -> Result<Self, CaptureError> {
        init_com()?;
        // SAFETY: standard MMDevice activation chain; all results checked.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(init)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).map_err(init)?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).map_err(init)?;

            let format_ptr = client.GetMixFormat().map_err(init)?;
            let format = &*format_ptr;
            // Copy packed fields to locals (references into packed structs are UB).
            let tag = format.wFormatTag;
            let ch = format.nChannels;
            let rate = format.nSamplesPerSec;
            let bits = format.wBitsPerSample;
            let (channels, ok) = validate_mix_format(format);
            if !ok {
                CoTaskMemFree(Some(format_ptr as *const _));
                return Err(CaptureError::Init(format!(
                    "unsupported mix format: tag {tag} ch {ch} rate {rate} bits {bits} \
                     (need 48 kHz float)"
                )));
            }
            // 1 s device buffer: poll_packets runs per video frame, this
            // gives ~60 polls of headroom.
            let r = client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
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
                channels,
                assembler,
                opus: OpusFrameEncoder::new()
                    .map_err(|e| CaptureError::Init(format!("opus: {e}")))?,
                queue: Vec::new(),
            })
        }
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
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc_100ns))
                    .map_err(lost)?;
                let pts_s = self.clock.pts_s(qpc_100ns as i64);
                let n = frames as usize * self.channels as usize;
                let floats: Vec<f32> = if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
                    vec![0.0; n]
                } else {
                    std::slice::from_raw_parts(data as *const f32, n).to_vec()
                };
                self.capture.ReleaseBuffer(frames).map_err(lost)?;
                self.assembler.push_chunk(pts_s, &extract_stereo(&floats, self.channels));
            }
        }
        Ok(())
    }
}

impl AudioSource for WasapiLoopback {
    fn poll_packets(&mut self, until_pts_s: f64) -> Result<Vec<AudioPacket>, CaptureError> {
        self.drain_device()?;
        while let Some((pts_s, frame)) = self.assembler.pop_frame() {
            let data = self
                .opus
                .encode_frame(&frame)
                .map_err(|e| CaptureError::DeviceLost(format!("opus encode: {e}")))?;
            self.queue.push(AudioPacket { data, pts_s, duration_s: FRAME_DURATION_S });
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

impl Drop for WasapiLoopback {
    fn drop(&mut self) {
        // SAFETY: Stop on a started client is always valid.
        let _ = unsafe { self.client.Stop() };
    }
}

/// (channels, acceptable): 48 kHz float32 in either plain or extensible form.
fn validate_mix_format(format: &WAVEFORMATEX) -> (u16, bool) {
    // Copy packed fields to locals (references into packed structs are UB).
    let channels = format.nChannels;
    let rate = format.nSamplesPerSec;
    let bits = format.wBitsPerSample;
    let tag = format.wFormatTag as u32;
    if rate != 48_000 || bits != 32 {
        return (channels, false);
    }
    let is_float = match tag {
        WAVE_FORMAT_IEEE_FLOAT => true,
        WAVE_FORMAT_EXTENSIBLE => {
            // SAFETY: extensible tag guarantees the larger layout.
            let ext = unsafe { &*(format as *const WAVEFORMATEX as *const WAVEFORMATEXTENSIBLE) };
            let sub = ext.SubFormat;
            sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
        }
        _ => false,
    };
    (channels, is_float)
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
            assert!((w[1].pts_s - w[0].pts_s - 0.02).abs() < 1e-6, "20 ms cadence");
        }
        for p in &packets {
            assert!(!p.data.is_empty());
        }
        eprintln!("captured {} opus packets in 300 ms", packets.len());
    }
}
