use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

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
    events: Arc<crate::host::events::ClientEventHub>,
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
            events,
            service_tx: Mutex::new(None),
            service_alive: Arc::new(AtomicBool::new(false)),
            service_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn settings(&self) -> crate::settings::AppSettings {
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    pub fn events(&self) -> Arc<crate::host::events::ClientEventHub> {
        self.events.clone()
    }

    pub fn save_replay(&self) -> bool {
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
                settings.to_service_options(None)?
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
