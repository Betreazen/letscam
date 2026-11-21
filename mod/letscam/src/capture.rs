//! Shared pieces of the audio capture threads (game loopback and microphone).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Instant;

/// What a finished capture produced. `origin` is the moment of the file's first sample.
#[derive(Clone, Debug)]
pub struct CaptureSummary {
    pub origin: Instant,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
    pub dropped_samples: u64,
    /// The capture ended early with this error (the file is still valid up to that point).
    pub error: Option<String>,
}

impl CaptureSummary {
    /// Milliseconds from `replay_start` to the first sample (negative = audio started earlier).
    pub fn offset_ms(&self, replay_start: Instant) -> f64 {
        signed_ms(self.origin, replay_start)
    }
}

/// `a - b` in milliseconds, may be negative.
pub fn signed_ms(a: Instant, b: Instant) -> f64 {
    if a >= b {
        a.duration_since(b).as_secs_f64() * 1000.0
    } else {
        -(b.duration_since(a).as_secs_f64() * 1000.0)
    }
}

/// A capture running on its own thread until [`CaptureThread::stop`].
pub(crate) struct CaptureThread {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Result<CaptureSummary, String>>,
}

impl CaptureThread {
    pub fn new(
        stop: Arc<AtomicBool>,
        handle: JoinHandle<Result<CaptureSummary, String>>,
    ) -> CaptureThread {
        CaptureThread { stop, handle }
    }

    /// Signals the thread, waits for it and returns what it wrote.
    pub fn stop(self) -> Result<CaptureSummary, String> {
        self.stop.store(true, Ordering::Release);
        self.handle
            .join()
            .map_err(|_| "поток записи звука аварийно завершился".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn signed_offsets() {
        let b = Instant::now();
        let a = b + Duration::from_millis(250);
        assert!((signed_ms(a, b) - 250.0).abs() < 1e-6);
        assert!((signed_ms(b, a) + 250.0).abs() < 1e-6);
    }
}
