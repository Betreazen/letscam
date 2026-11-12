//! A recording in progress: game audio, microphone, GUI snapshots and the player track,
//! all timed against the replay start. `stop` writes `session.json`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::capture::CaptureSummary;
use crate::config::Config;
use crate::gui_capture::{GuiJob, GuiWriter};
use crate::loopback::GameAudioCapture;
use crate::mic::MicCapture;
use crate::session::{AudioTrack, GuiEvent, GuiFrame, SESSION_VERSION, Session};
use crate::track::TrackSample;

pub const GAME_WAV: &str = "game.wav";
pub const MIC_WAV: &str = "mic.wav";
pub const GUI_DIR: &str = "gui";

pub struct Recording {
    dir: PathBuf,
    gui_interval: Duration,
    replay_start: Instant,
    warnings: Vec<String>,
    track: Vec<TrackSample>,
    gui_frames: Vec<GuiFrame>,
    gui_events: Vec<GuiEvent>,
    last_gui: Option<Instant>,
    gui_seq: u32,
    gui: GuiWriter,
    game: Option<GameAudioCapture>,
    mic: Option<MicCapture>,
}

fn ms(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

/// Turns a finished capture into the session entry (or a warning).
fn audio_track(
    name: &str,
    what: &str,
    result: Result<CaptureSummary, String>,
    replay_start: Instant,
    warnings: &mut Vec<String>,
) -> Option<AudioTrack> {
    match result {
        Ok(s) => {
            if let Some(e) = &s.error {
                warnings.push(format!("{what}: запись прервалась: {e}"));
            }
            if s.dropped_samples > 0 {
                warnings.push(format!("{what}: потеряно {} сэмплов", s.dropped_samples));
            }
            (s.frames > 0).then(|| AudioTrack {
                file: name.to_string(),
                offset_ms: (s.offset_ms(replay_start) * 1000.0).round() / 1000.0,
                sample_rate: s.sample_rate,
                channels: s.channels,
                dropped_samples: s.dropped_samples,
            })
        }
        Err(e) => {
            warnings.push(format!("{what}: {e}"));
            None
        }
    }
}

impl Recording {
    /// Creates `dir` and `dir/gui`, starts game audio (and the microphone if enabled).
    /// Audio failures become warnings: the recording goes on without that track.
    pub fn start(dir: &Path, cfg: &Config, replay_start: Instant) -> Result<Recording, String> {
        std::fs::create_dir_all(dir.join(GUI_DIR))
            .map_err(|e| format!("не удалось создать папку записи {}: {e}", dir.display()))?;
        let mut warnings = Vec::new();
        let game = match GameAudioCapture::start(&dir.join(GAME_WAV)) {
            Ok(g) => {
                if let Some(w) = g.warning() {
                    warnings.push(format!("Звук игры: {w}"));
                }
                Some(g)
            }
            Err(e) => {
                warnings.push(format!("Звук игры не записывается: {e}"));
                None
            }
        };
        let mic = if cfg.record_mic {
            MicCapture::start(&dir.join(MIC_WAV), &cfg.mic_device)
                .map_err(|e| warnings.push(format!("Микрофон не записывается: {e}")))
                .ok()
        } else {
            None
        };
        for w in &warnings {
            log::warn!("LetsCam: {w}");
        }
        Ok(Recording {
            dir: dir.to_path_buf(),
            gui_interval: Duration::from_secs_f64(1.0 / f64::from(cfg.gui_fps.clamp(1, 30))),
            replay_start,
            warnings,
            track: Vec::new(),
            gui_frames: Vec::new(),
            gui_events: Vec::new(),
            last_gui: None,
            gui_seq: 0,
            gui: GuiWriter::start()?,
            game,
            mic,
        })
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.replay_start)
    }

    pub fn push_track(&mut self, sample: TrackSample) {
        self.track.push(sample);
    }

    /// True at most `gui_fps` times per second.
    pub fn gui_frame_due(&self, now: Instant) -> bool {
        self.last_gui
            .is_none_or(|last| now.saturating_duration_since(last) >= self.gui_interval)
    }

    /// Hands a GUI snapshot to the PNG thread (halved there). Never blocks: if the thread is
    /// behind, the snapshot is skipped.
    pub fn push_gui_frame(
        &mut self,
        now: Instant,
        width: u32,
        height: u32,
        pixels: Vec<[u8; 4]>,
        bgra: bool,
    ) {
        self.last_gui = Some(now);
        if pixels.len() != width as usize * height as usize {
            log::warn!(
                "LetsCam: кадр интерфейса {width}x{height} с {} пикселями пропущен",
                pixels.len()
            );
            return;
        }
        self.gui_seq += 1;
        let file = format!("{GUI_DIR}/{:06}.png", self.gui_seq);
        let job = GuiJob {
            path: self.dir.join(&file),
            file: file.clone(),
            width,
            height,
            pixels,
            bgra,
        };
        if self.gui.submit(job) {
            self.gui_frames.push(GuiFrame {
                t_ms: ms(self.elapsed(now)),
                file,
            });
        }
    }

    /// A GUI screen opened or closed (repeated states are ignored).
    pub fn gui_event(&mut self, now: Instant, open: bool) {
        if self.gui_events.last().is_some_and(|e| e.open == open) {
            return;
        }
        self.gui_events.push(GuiEvent {
            t_ms: ms(self.elapsed(now)),
            open,
        });
    }

    /// Stops all captures and writes `session.json`.
    pub fn stop(self, now: Instant) -> Result<Session, String> {
        let duration_ms = ms(self.elapsed(now));
        let mut warnings = self.warnings;
        let game = self.game.map(GameAudioCapture::stop);
        let mic = self.mic.map(MicCapture::stop);
        let game_audio = game
            .and_then(|r| audio_track(GAME_WAV, "Звук игры", r, self.replay_start, &mut warnings));
        let mic_audio =
            mic.and_then(|r| audio_track(MIC_WAV, "Микрофон", r, self.replay_start, &mut warnings));
        let failed = self.gui.finish();
        let mut gui_frames = self.gui_frames;
        gui_frames.retain(|f| !failed.contains(&f.file));
        let session = Session {
            version: SESSION_VERSION,
            duration_ms,
            game_audio,
            mic_audio,
            gui_frames,
            gui_events: self.gui_events,
            track: self.track,
            warnings,
        };
        session.save(&self.dir)?;
        log::info!(
            "LetsCam: запись {} остановлена, {:.1} с",
            self.dir.display(),
            f64::from(duration_ms) / 1000.0
        );
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    #[test]
    fn records_session_with_track_gui_and_game_audio() {
        let tmp = TempDir::new("rec");
        let dir = tmp.path().join("2026-11-12_10-00-00");
        let cfg = Config {
            record_mic: false,
            gui_fps: 4,
            ..Config::default()
        };
        let start = Instant::now();
        let replay_start = start
            .checked_sub(Duration::from_millis(100))
            .unwrap_or(start);
        let mut rec = Recording::start(&dir, &cfg, replay_start).expect("start");
        assert!(dir.join(GUI_DIR).is_dir());
        for i in 0..10 {
            rec.push_track(TrackSample::at(i * 50, [0.0, 65.0, f64::from(i)], 0.0, 0.0));
        }
        let now = Instant::now();
        assert!(rec.gui_frame_due(now));
        rec.gui_event(now, true);
        rec.gui_event(now, true);
        rec.push_gui_frame(now, 8, 8, vec![[1, 2, 3, 255]; 64], true);
        assert!(!rec.gui_frame_due(now + Duration::from_millis(100)));
        assert!(rec.gui_frame_due(now + Duration::from_millis(250)));
        rec.push_gui_frame(now, 8, 8, vec![[0; 4]; 3], false);
        std::thread::sleep(Duration::from_millis(400));
        let stop = Instant::now();
        rec.gui_event(stop, false);
        let session = rec.stop(stop).expect("stop");
        assert_eq!(session.track.len(), 10);
        assert_eq!(session.gui_events.len(), 2);
        assert_eq!(session.gui_frames.len(), 1);
        assert!(dir.join(&session.gui_frames[0].file).is_file());
        assert!(session.duration_ms >= 500, "{}", session.duration_ms);
        let loaded = Session::load(&dir).expect("load");
        assert_eq!(loaded.track, session.track);
        assert_eq!(loaded.gui_frames, session.gui_frames);
        let offsets = |s: &Session| s.game_audio.as_ref().map(|a| a.offset_ms);
        if let (Some(a), Some(b)) = (offsets(&loaded), offsets(&session)) {
            assert!((a - b).abs() < 1e-6);
        }
        match &session.game_audio {
            Some(a) => {
                assert!(dir.join(&a.file).is_file());
                assert!((a.offset_ms - 100.0).abs() < 60.0, "offset {}", a.offset_ms);
            }
            None => println!("NOTE: no game audio here: {:?}", session.warnings),
        }
        assert!(session.mic_audio.is_none());
    }
}
