use std::fmt;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureDiagnostic {
    WasapiDataDiscontinuity {
        suppressed_since_last: u64,
    },
    WasapiLateAudioReanchored {
        source: &'static str,
        correction_ms: u64,
        suppressed_since_last: u64,
    },
}

impl fmt::Display for CaptureDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WasapiDataDiscontinuity {
                suppressed_since_last,
            } => write!(
                formatter,
                "capture event=wasapi_data_discontinuity suppressed_since_last={suppressed_since_last} action=audio_gap_fill_capped"
            ),
            Self::WasapiLateAudioReanchored {
                source,
                correction_ms,
                suppressed_since_last,
            } => write!(
                formatter,
                "capture event=wasapi_late_audio_reanchored source={source} correction_ms={correction_ms} suppressed_since_last={suppressed_since_last} action=preserve_live_audio"
            ),
        }
    }
}

type DiagnosticHandler = dyn Fn(CaptureDiagnostic) + Send + Sync + 'static;
static DIAGNOSTIC_HANDLER: OnceLock<Box<DiagnosticHandler>> = OnceLock::new();

pub fn install_diagnostic_handler(
    handler: impl Fn(CaptureDiagnostic) + Send + Sync + 'static,
) -> Result<(), &'static str> {
    DIAGNOSTIC_HANDLER
        .set(Box::new(handler))
        .map_err(|_| "capture diagnostic handler is already installed")
}

pub fn emit_diagnostic(event: CaptureDiagnostic) {
    if let Some(handler) = DIAGNOSTIC_HANDLER.get() {
        handler(event);
    }
}

#[derive(Debug)]
pub struct DiagnosticRateLimiter {
    interval: Duration,
    last_emitted: Option<Instant>,
    suppressed: u64,
}

impl DiagnosticRateLimiter {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_emitted: None,
            suppressed: 0,
        }
    }

    pub fn observe(&mut self, now: Instant) -> Option<u64> {
        let should_emit = self
            .last_emitted
            .is_none_or(|last| now.saturating_duration_since(last) >= self.interval);
        if should_emit {
            self.last_emitted = Some(now);
            return Some(std::mem::take(&mut self.suppressed));
        }
        self.suppressed = self.suppressed.saturating_add(1);
        None
    }
}
