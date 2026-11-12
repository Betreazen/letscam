//! Game audio capture. Windows: WASAPI process loopback of our own process tree (only the
//! game's sound). If that is unavailable (older Windows), falls back to loopback of the
//! default output device (all system sound) with a warning. Other OS: an error.

use std::path::Path;

use crate::capture::{CaptureSummary, CaptureThread};

pub const SYSTEM_WIDE_WARNING: &str = "звук игры пишется вместе со всем звуком системы";

pub struct GameAudioCapture {
    inner: CaptureThread,
    warning: Option<String>,
    system_wide: bool,
}

impl GameAudioCapture {
    /// Starts writing the game's sound to `path` (16-bit PCM WAV, silence kept on the timeline).
    pub fn start(path: &Path) -> Result<GameAudioCapture, String> {
        start_impl(path)
    }

    /// A warning for the user (e.g. the system-wide fallback is in use).
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    /// True when the fallback captures all system sound, not only the game.
    pub fn is_system_wide(&self) -> bool {
        self.system_wide
    }

    pub fn stop(self) -> Result<CaptureSummary, String> {
        self.inner.stop()
    }
}

#[cfg(windows)]
fn start_impl(path: &Path) -> Result<GameAudioCapture, String> {
    match crate::loopback_win::start(path.to_path_buf()) {
        Ok(inner) => {
            log::info!("LetsCam: звук игры (захват процесса) → {}", path.display());
            Ok(GameAudioCapture {
                inner,
                warning: None,
                system_wide: false,
            })
        }
        Err(process_err) => {
            log::warn!("LetsCam: захват звука процесса недоступен: {process_err}");
            let (inner, device) = crate::cpal_capture::start(
                crate::cpal_capture::Source::OutputLoopback,
                path.to_path_buf(),
                false,
            )
            .map_err(|e| {
                format!("захват звука процесса: {process_err}; захват системного звука: {e}")
            })?;
            log::info!(
                "LetsCam: звук игры (весь звук «{device}») → {}",
                path.display()
            );
            Ok(GameAudioCapture {
                inner,
                warning: Some(format!("{SYSTEM_WIDE_WARNING} ({process_err})")),
                system_wide: true,
            })
        }
    }
}

#[cfg(not(windows))]
fn start_impl(_path: &Path) -> Result<GameAudioCapture, String> {
    Err("запись звука игры пока поддерживается только в Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{TempDir, ffprobe_json};
    use std::time::Duration;

    /// The test process plays nothing: the WAV must still cover the whole time with silence.
    #[test]
    fn loopback_records_silence_for_the_whole_time() {
        let tmp = TempDir::new("loopback");
        let path = tmp.path().join("game.wav");
        let cap = match GameAudioCapture::start(&path) {
            Ok(c) => c,
            Err(e) => {
                println!("NOTE: game audio capture unavailable here ({e}), skipped");
                return;
            }
        };
        println!("system-wide fallback: {}", cap.is_system_wide());
        std::thread::sleep(Duration::from_millis(1000));
        let s = cap.stop().expect("stop");
        assert!(s.error.is_none(), "{:?}", s.error);
        let secs = s.frames as f64 / f64::from(s.sample_rate);
        assert!((secs - 1.0).abs() < 0.1, "captured {secs} s");
        let Some(json) = ffprobe_json(&path) else {
            return;
        };
        assert_eq!(json["streams"][0]["codec_name"], "pcm_s16le");
        let dur: f64 = json["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse().ok())
            .expect("duration");
        assert!((dur - 1.0).abs() < 0.1, "ffprobe duration {dur}");
    }

    /// Plays a quiet 440 Hz tone from this process and checks that it lands in the WAV.
    /// Ignored by default because it makes sound: `cargo test -- --ignored loopback_hears`.
    #[test]
    #[ignore]
    fn loopback_hears_own_process() {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let tmp = TempDir::new("loopback-tone");
        let path = tmp.path().join("game.wav");
        let Ok(cap) = GameAudioCapture::start(&path) else {
            println!("NOTE: capture unavailable, skipped");
            return;
        };
        let host = cpal::default_host();
        let Some(dev) = host.default_output_device() else {
            println!("NOTE: no output device, skipped");
            return;
        };
        let cfg = dev.default_output_config().expect("output config").config();
        let (rate, ch) = (cfg.sample_rate.0 as f32, usize::from(cfg.channels));
        let mut phase = 0f32;
        let stream = dev
            .build_output_stream::<f32, _, _>(
                &cfg,
                move |out: &mut [f32], _| {
                    for frame in out.chunks_mut(ch) {
                        phase = (phase + 440.0 / rate) % 1.0;
                        frame.fill((phase * std::f32::consts::TAU).sin() * 0.05);
                    }
                },
                |e| eprintln!("{e}"),
                None,
            )
            .expect("output stream");
        stream.play().expect("play");
        std::thread::sleep(Duration::from_millis(800));
        drop(stream);
        cap.stop().expect("stop");
        let bytes = std::fs::read(&path).expect("read");
        let peak = bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b).unsigned_abs())
            .max()
            .unwrap_or(0);
        println!("peak sample {peak}");
        assert!(peak > 500, "tone not captured (peak {peak})");
    }
}
