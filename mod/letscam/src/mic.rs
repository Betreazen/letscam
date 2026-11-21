//! Microphone capture: cpal input device (by name substring or default), mono, 16-bit WAV at
//! the device rate, on the wall-clock timeline (see [`crate::capture::CaptureSummary::origin`]).

use std::path::Path;

use crate::capture::{CaptureSummary, CaptureThread};
use crate::cpal_capture::{self, Source};

pub struct MicCapture {
    inner: CaptureThread,
    device: String,
}

impl MicCapture {
    /// Opens the device and starts writing `path`. Errors are user-facing (Russian) text;
    /// the caller records without the microphone in that case.
    pub fn start(path: &Path, device_name: &str) -> Result<MicCapture, String> {
        let (inner, device) = cpal_capture::start(
            Source::Input(device_name.to_string()),
            path.to_path_buf(),
            true,
        )?;
        log::info!("LetsCam: микрофон «{device}» → {}", path.display());
        Ok(MicCapture { inner, device })
    }

    pub fn device_name(&self) -> &str {
        &self.device
    }

    /// Stops the stream and finalizes the WAV.
    pub fn stop(self) -> Result<CaptureSummary, String> {
        self.inner.stop()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    #[test]
    fn missing_device_returns_error_text() {
        let tmp = TempDir::new("mic-missing");
        let err = MicCapture::start(&tmp.path().join("mic.wav"), "letscam-no-such-mic-42")
            .err()
            .expect("error");
        assert!(!err.is_empty());
    }

    /// Records the real default microphone for 0.5 s. Ignored by default (needs a device and
    /// touches the user's microphone): `cargo test -- --ignored mic_records`.
    #[test]
    #[ignore]
    fn mic_records_default_device() {
        let tmp = TempDir::new("mic-real");
        let path = tmp.path().join("mic.wav");
        let begun = std::time::Instant::now();
        let Ok(mic) = MicCapture::start(&path, "") else {
            println!("NOTE: no default microphone, skipped");
            return;
        };
        std::thread::sleep(std::time::Duration::from_millis(500));
        let s = mic.stop().expect("stop");
        let wall = begun.elapsed().as_secs_f64();
        assert_eq!(s.channels, 1);
        // The file starts at `origin` (just before the device opens) and runs until stop.
        let secs = s.frames as f64 / f64::from(s.sample_rate);
        assert!(secs >= 0.45 && secs <= wall + 0.02, "{secs} s of {wall} s");
    }
}
