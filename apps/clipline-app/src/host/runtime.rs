use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use tauri::Manager;

#[derive(Debug, serde::Serialize)]
pub struct FallbackCommandResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl FallbackCommandResult {
    pub fn ok(value: serde_json::Value) -> Self {
        Self {
            ok: true,
            value: Some(value),
            error: None,
        }
    }

    pub fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(error.into()),
        }
    }
}

#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
pub struct FallbackHostContext {
    settings: Mutex<crate::settings::AppSettings>,
    app: Mutex<Option<tauri::AppHandle<tauri::Wry>>>,
    events: Arc<crate::host::events::ClientEventHub>,
    decodable_codecs: Mutex<Vec<crate::service::Codec>>,
    mic_test_stop: Mutex<Option<mpsc::Sender<()>>>,
    service_tx: Mutex<Option<mpsc::Sender<crate::service::Cmd>>>,
    service_alive: Arc<AtomicBool>,
    service_generation: Arc<AtomicU64>,
}

#[allow(
    dead_code,
    reason = "staged for fallback client integration in later tasks"
)]
impl FallbackHostContext {
    pub fn new(
        settings: crate::settings::AppSettings,
        events: Arc<crate::host::events::ClientEventHub>,
    ) -> Self {
        Self {
            settings: Mutex::new(settings),
            app: Mutex::new(None),
            events,
            decodable_codecs: Mutex::new(vec![crate::service::Codec::H264]),
            mic_test_stop: Mutex::new(None),
            service_tx: Mutex::new(None),
            service_alive: Arc::new(AtomicBool::new(false)),
            service_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn attach_app_handle(&self, app: tauri::AppHandle<tauri::Wry>) -> Result<(), String> {
        let mut guard = self
            .app
            .lock()
            .map_err(|_| "fallback app handle lock poisoned".to_string())?;
        *guard = Some(app);
        Ok(())
    }

    fn app_handle(&self) -> Result<tauri::AppHandle<tauri::Wry>, String> {
        self.app
            .lock()
            .map_err(|_| "fallback app handle lock poisoned".to_string())?
            .clone()
            .ok_or_else(|| "fallback updater host is not attached".to_string())
    }

    fn attached_app_handle(&self) -> Option<tauri::AppHandle<tauri::Wry>> {
        self.app.lock().ok().and_then(|app| app.clone())
    }

    pub fn settings(&self) -> crate::settings::AppSettings {
        if let Some(app) = self.attached_app_handle() {
            let state = app.state::<crate::app::RuntimeState>();
            return crate::app::host_get_settings(&state);
        }
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    pub fn events(&self) -> Arc<crate::host::events::ClientEventHub> {
        self.events.clone()
    }

    pub async fn check_for_updates(&self) -> Result<crate::app::UpdateCheckResult, String> {
        let app = self.app_handle()?;
        let settings = self.settings();
        crate::app::host_check_for_updates(&app, &settings).await
    }

    pub async fn install_update(&self) -> Result<(), String> {
        let app = self.app_handle()?;
        let state = app.state::<crate::app::RuntimeState>();
        let update = crate::app::host_update_for_install(&app, &state).await?;
        self.stop_microphone_test();
        crate::app::host_install_available_update(&app, &state, update).await
    }

    pub fn save_settings(
        &self,
        mut settings: crate::settings::AppSettings,
    ) -> Result<crate::settings::AppSettings, String> {
        if let Some(app) = self.attached_app_handle() {
            let committed = crate::app::host_save_settings(&app, settings)?;
            let mut guard = self
                .settings
                .lock()
                .map_err(|_| "settings lock poisoned".to_string())?;
            *guard = committed.clone();
            return Ok(committed);
        }

        settings.hotkey = crate::settings::normalize_hotkey(&settings.hotkey)?;
        settings.validate()?;
        let media_dir = settings.media_dir_path()?;
        std::fs::create_dir_all(&media_dir)
            .map_err(|e| format!("create media folder {media_dir:?}: {e}"))?;
        settings.save()?;
        let mut guard = self
            .settings
            .lock()
            .map_err(|_| "settings lock poisoned".to_string())?;
        *guard = settings.clone();
        Ok(settings)
    }

    pub fn get_autostart_status(&self) -> Result<bool, String> {
        if let Some(app) = self.attached_app_handle() {
            return crate::app::host_get_autostart_status(&app);
        }
        Ok(crate::app::host_persisted_autostart_status(
            &self.settings(),
        ))
    }

    pub fn update_cloud_record_paths(&self, old_path: &str, new_path: &str) -> Result<(), String> {
        if old_path == new_path {
            return Ok(());
        }
        if let Some(app) = self.attached_app_handle() {
            let state = app.state::<crate::app::RuntimeState>();
            return crate::library::update_cloud_record_paths_for_host(&state, old_path, new_path);
        }

        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "settings lock poisoned".to_string())?;
        crate::library::update_cloud_record_paths_in_cloud(&mut settings.cloud, old_path, new_path);
        Ok(())
    }

    pub fn list_displays(&self) -> Result<Vec<crate::app::DisplayInfo>, String> {
        crate::app::host_list_displays()
    }

    pub fn list_audio_devices(&self) -> Result<crate::app::AudioDeviceLists, String> {
        crate::app::host_list_audio_devices()
    }

    pub fn probe_encoders(&self) -> Vec<crate::service::EncoderOption> {
        crate::service::available_encoder_options()
    }

    pub fn report_decode_support(&self, codecs: &[String]) {
        if let Some(app) = self.attached_app_handle() {
            let state = app.state::<crate::app::RuntimeState>();
            crate::app::host_report_decode_support(&state, codecs);
            return;
        }

        let mut decodable_codecs = vec![crate::service::Codec::H264];
        for codec in codecs {
            match codec.as_str() {
                "hevc" if !decodable_codecs.contains(&crate::service::Codec::Hevc) => {
                    decodable_codecs.push(crate::service::Codec::Hevc);
                }
                "av1" if !decodable_codecs.contains(&crate::service::Codec::Av1) => {
                    decodable_codecs.push(crate::service::Codec::Av1);
                }
                _ => {}
            }
        }
        if let Ok(mut guard) = self.decodable_codecs.lock() {
            *guard = decodable_codecs;
        }
    }

    pub fn start_microphone_test(
        &self,
        device_id: Option<String>,
        volume: f64,
        mono: bool,
    ) -> Result<(), String> {
        self.stop_microphone_test();
        let channels = if mono {
            clipline_capture::windows::wasapi::WasapiChannelMode::Mono
        } else {
            clipline_capture::windows::wasapi::WasapiChannelMode::Stereo
        };
        let (stop_tx, stop_rx) = mpsc::channel();
        {
            let mut guard = self
                .mic_test_stop
                .lock()
                .map_err(|_| "mic test state lock poisoned".to_string())?;
            *guard = Some(stop_tx);
        }
        let events = self.events.clone();
        std::thread::Builder::new()
            .name("clipline-fallback-mic-test".into())
            .spawn(move || {
                let run = || -> Result<(), String> {
                    let clock = clipline_capture::clock::RelativeClock::new(
                        clipline_capture::windows::qpc_now_ticks_100ns()
                            .map_err(|e| e.to_string())?,
                    );
                    let mut source =
                        clipline_capture::windows::wasapi::WasapiLoopback::start_microphone(
                            clock,
                            device_id.as_deref(),
                            volume,
                            channels,
                        )
                        .map_err(|e| e.to_string())?;
                    loop {
                        if stop_rx.try_recv().is_ok() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(30));
                        let chunk = source.poll_monitor_chunk().map_err(|e| e.to_string())?;
                        let samples = chunk
                            .samples
                            .into_iter()
                            .map(|sample| {
                                let scaled = (sample.clamp(-1.0, 1.0) * 32_768.0).round();
                                scaled.clamp(i16::MIN as f32, i16::MAX as f32) as i16
                            })
                            .collect();
                        events.emit(crate::host::events::ClientEvent::new(
                            "mic-test",
                            serde_json::to_value(FallbackMicMonitorEvent {
                                rms: chunk.level.rms,
                                peak: chunk.level.peak,
                                sample_count: chunk.level.sample_count,
                                samples,
                            })
                            .unwrap_or(serde_json::Value::Null),
                        ));
                    }
                    Ok(())
                };
                if let Err(e) = run() {
                    events.emit(crate::host::events::ClientEvent::new(
                        "mic-test-error",
                        serde_json::json!(e),
                    ));
                    events.emit(crate::host::events::ClientEvent::new(
                        "mic-test-stopped",
                        serde_json::Value::Null,
                    ));
                }
            })
            .map(|_| ())
            .map_err(|e| format!("spawn fallback microphone test: {e}"))
    }

    pub fn stop_microphone_test(&self) {
        match self.mic_test_stop.lock() {
            Ok(mut guard) => {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
            Err(e) => eprintln!("fallback mic test state lock poisoned: {e}"),
        }
    }

    pub fn save_replay(&self) -> bool {
        if let Some(app) = self.attached_app_handle() {
            let state = app.state::<crate::app::RuntimeState>();
            return crate::app::host_save_replay(&state);
        }

        let Some(tx) = self.live_service_tx() else {
            return false;
        };
        if tx.send(crate::service::Cmd::Save).is_ok() {
            return true;
        }
        self.mark_service_dead();
        false
    }

    pub fn recording_active(&self) -> bool {
        self.live_service_tx().is_some()
    }

    fn live_service_tx(&self) -> Option<mpsc::Sender<crate::service::Cmd>> {
        if !self.service_alive.load(Ordering::Acquire) {
            if let Ok(mut service_tx) = self.service_tx.lock() {
                service_tx.take();
            }
            return None;
        }
        self.service_tx
            .lock()
            .ok()
            .and_then(|service_tx| service_tx.as_ref().cloned())
    }

    fn mark_service_dead(&self) {
        self.service_alive.store(false, Ordering::Release);
        if let Ok(mut service_tx) = self.service_tx.lock() {
            service_tx.take();
        }
    }

    pub fn set_recording(&self, recording: bool) -> Result<bool, String> {
        if let Some(app) = self.attached_app_handle() {
            let state = app.state::<crate::app::RuntimeState>();
            return crate::app::host_set_recording(&state, app.clone(), recording);
        }

        if recording {
            let mut service_tx = self
                .service_tx
                .lock()
                .map_err(|_| "fallback recorder state lock poisoned".to_string())?;
            if !self.service_alive.load(Ordering::Acquire) {
                service_tx.take();
            }
            if service_tx.is_some() {
                return Ok(true);
            }

            let options = {
                let settings = self
                    .settings
                    .lock()
                    .map_err(|_| "fallback settings lock poisoned".to_string())?;
                let mut options = settings.to_service_options(None)?;
                options.decodable_codecs = self
                    .decodable_codecs
                    .lock()
                    .map_err(|_| "fallback codec state lock poisoned".to_string())?
                    .clone();
                options
            };
            let (tx, rx) = crate::service::spawn(options);
            let generation = self.service_generation.fetch_add(1, Ordering::AcqRel) + 1;
            self.service_alive.store(true, Ordering::Release);
            if let Err(e) = spawn_event_pump(
                self.events.clone(),
                self.service_alive.clone(),
                self.service_generation.clone(),
                generation,
                rx,
            ) {
                self.service_alive.store(false, Ordering::Release);
                let _ = tx.send(crate::service::Cmd::Stop { announce: false });
                return Err(e);
            }
            *service_tx = Some(tx);
            return Ok(true);
        }

        self.service_alive.store(false, Ordering::Release);
        self.service_generation.fetch_add(1, Ordering::AcqRel);
        let tx = self
            .service_tx
            .lock()
            .map_err(|_| "fallback recorder state lock poisoned".to_string())?
            .take();
        if let Some(tx) = tx {
            let _ = tx.send(crate::service::Cmd::Stop { announce: true });
        }
        Ok(false)
    }

    #[cfg(test)]
    fn for_tests(settings: crate::settings::AppSettings) -> Self {
        Self::new(
            settings,
            Arc::new(crate::host::events::ClientEventHub::default()),
        )
    }
}

#[derive(serde::Serialize)]
struct FallbackMicMonitorEvent {
    rms: f32,
    peak: f32,
    sample_count: usize,
    samples: Vec<i16>,
}

fn spawn_event_pump(
    events: Arc<crate::host::events::ClientEventHub>,
    service_alive: Arc<AtomicBool>,
    service_generation: Arc<AtomicU64>,
    generation: u64,
    event_rx: mpsc::Receiver<crate::service::Event>,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("clipline-fallback-event-pump".into())
        .spawn(move || {
            for event in event_rx {
                if service_generation.load(Ordering::Acquire) != generation {
                    break;
                }
                match &event {
                    crate::service::Event::Status { .. } => {
                        events.emit(crate::host::events::ClientEvent::new(
                            "status",
                            serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
                        ));
                    }
                    crate::service::Event::Saved { .. } => {
                        events.emit(crate::host::events::ClientEvent::new(
                            "saved",
                            serde_json::to_value(&event).unwrap_or(serde_json::Value::Null),
                        ));
                    }
                    crate::service::Event::Error { message } => {
                        events.emit(crate::host::events::ClientEvent::new(
                            "error",
                            serde_json::json!(message),
                        ));
                    }
                }
            }
            if service_generation.load(Ordering::Acquire) == generation {
                service_alive.store(false, Ordering::Release);
            }
        })
        .map(|_| ())
        .map_err(|e| format!("spawn fallback event pump: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        condition()
    }

    #[test]
    fn command_result_serializes_success_and_error_for_fallback_bridge() {
        let ok = FallbackCommandResult::ok(serde_json::json!({"recording": true}));
        let err = FallbackCommandResult::err("failed");

        assert_eq!(serde_json::to_value(ok).unwrap()["ok"], true);
        assert_eq!(serde_json::to_value(err).unwrap()["error"], "failed");
    }

    #[test]
    fn fallback_context_exposes_initial_settings() {
        let context = FallbackHostContext::for_tests(crate::settings::AppSettings::default());

        assert_eq!(context.settings().hotkey, "Alt+F10");
    }

    #[test]
    fn fallback_context_records_save_requests_without_panicking_when_service_absent() {
        let context = FallbackHostContext::for_tests(crate::settings::AppSettings::default());

        assert!(!context.save_replay());
    }

    #[test]
    fn fallback_context_reports_recording_state_from_service_presence() {
        let context = FallbackHostContext::for_tests(crate::settings::AppSettings::default());

        assert!(!context.recording_active());
    }

    #[test]
    fn fallback_context_clears_recording_state_when_event_pump_ends() {
        let context = FallbackHostContext::for_tests(crate::settings::AppSettings::default());
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let generation = context.service_generation.fetch_add(1, Ordering::AcqRel) + 1;
        context.service_alive.store(true, Ordering::Release);
        *context.service_tx.lock().unwrap() = Some(cmd_tx);

        spawn_event_pump(
            context.events(),
            context.service_alive.clone(),
            context.service_generation.clone(),
            generation,
            event_rx,
        )
        .expect("spawn test event pump");

        assert!(context.recording_active());
        drop(event_tx);

        assert!(wait_until(|| !context.recording_active()));
        assert!(context.service_tx.lock().unwrap().is_none());
    }

    #[test]
    fn stale_event_pump_generation_does_not_emit_recorder_events() {
        let context = FallbackHostContext::for_tests(crate::settings::AppSettings::default());
        let event_subscriber = context.events().subscribe();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let generation = context.service_generation.fetch_add(1, Ordering::AcqRel) + 1;
        context.service_alive.store(true, Ordering::Release);

        spawn_event_pump(
            context.events(),
            context.service_alive.clone(),
            context.service_generation.clone(),
            generation,
            event_rx,
        )
        .expect("spawn test event pump");

        context.service_generation.fetch_add(1, Ordering::AcqRel);
        event_tx
            .send(crate::service::Event::Status {
                recording: false,
                segments: 0,
                buffered_s: 0.0,
                buffered_mb: 0.0,
                full_session: false,
                encoder: String::new(),
            })
            .expect("send stale status");
        event_tx
            .send(crate::service::Event::Saved {
                path: "stale.mp4".to_string(),
                seconds: 1.0,
                markers: 0,
                full_session: false,
                gc_deleted: 0,
                gc_freed_bytes: 0,
                storage_total_bytes: 0,
                storage_quota_bytes: None,
                storage_over_quota: false,
            })
            .expect("send stale saved");
        event_tx
            .send(crate::service::Event::Error {
                message: "stale".to_string(),
            })
            .expect("send stale error");
        drop(event_tx);

        assert!(!wait_until(|| event_subscriber.try_recv().is_ok()));
    }
}
