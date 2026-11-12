//! `session.json`: what a recording produced and how the tracks line up with the replay.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::track::TrackSample;

pub const SESSION_FILE: &str = "session.json";
pub const SESSION_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioTrack {
    /// File name relative to the recording dir.
    pub file: String,
    /// Time of the first sample relative to the replay start, ms (may be negative).
    pub offset_ms: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub dropped_samples: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GuiFrame {
    pub t_ms: u32,
    /// File name relative to the recording dir, e.g. `gui/000001.png`.
    pub file: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct GuiEvent {
    pub t_ms: u32,
    pub open: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub version: u32,
    pub duration_ms: u32,
    pub game_audio: Option<AudioTrack>,
    pub mic_audio: Option<AudioTrack>,
    pub gui_frames: Vec<GuiFrame>,
    pub gui_events: Vec<GuiEvent>,
    pub track: Vec<TrackSample>,
    pub warnings: Vec<String>,
}

impl Session {
    pub fn load(dir: &Path) -> Result<Session, String> {
        let path = dir.join(SESSION_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("не удалось прочитать {}: {e}", path.display()))?;
        let session: Session = serde_json::from_str(&text)
            .map_err(|e| format!("{} повреждён: {e}", path.display()))?;
        if session.version > SESSION_VERSION {
            return Err(format!(
                "{}: версия {} новее, чем понимает мод ({SESSION_VERSION})",
                path.display(),
                session.version
            ));
        }
        Ok(session)
    }

    /// Writes `dir/session.json` (via a temp file, so a crash never leaves half a file).
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let path = dir.join(SESSION_FILE);
        let tmp = dir.join("session.json.tmp");
        let text = serde_json::to_string(self).map_err(|e| format!("session.json: {e}"))?;
        std::fs::write(&tmp, text).map_err(|e| format!("запись {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("запись {}: {e}", path.display()))
    }

    /// Whether a GUI screen is open at `t_ms` according to the events.
    pub fn gui_open_at(events: &[GuiEvent], t_ms: f64) -> bool {
        let idx = events.partition_point(|e| f64::from(e.t_ms) <= t_ms);
        idx > 0 && events[idx - 1].open
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    fn sample() -> Session {
        Session {
            version: SESSION_VERSION,
            duration_ms: 1234,
            game_audio: Some(AudioTrack {
                file: "game.wav".into(),
                offset_ms: -12.5,
                sample_rate: 48_000,
                channels: 2,
                dropped_samples: 0,
            }),
            mic_audio: None,
            gui_frames: vec![GuiFrame {
                t_ms: 100,
                file: "gui/000001.png".into(),
            }],
            gui_events: vec![
                GuiEvent {
                    t_ms: 90,
                    open: true,
                },
                GuiEvent {
                    t_ms: 500,
                    open: false,
                },
            ],
            track: vec![TrackSample::at(0, [0.0, 65.62, 0.0], 0.0, 0.0)],
            warnings: vec!["микрофон не найден".into()],
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = TempDir::new("session");
        let s = sample();
        s.save(tmp.path()).expect("save");
        assert_eq!(Session::load(tmp.path()).expect("load"), s);
    }

    #[test]
    fn load_errors_are_readable() {
        let tmp = TempDir::new("session-bad");
        assert!(Session::load(tmp.path()).is_err());
        std::fs::write(tmp.path().join(SESSION_FILE), "{").expect("write");
        let e = Session::load(tmp.path()).expect_err("broken");
        assert!(e.contains("повреждён"), "{e}");
    }

    #[test]
    fn gui_open_lookup() {
        let ev = sample().gui_events;
        assert!(!Session::gui_open_at(&ev, 0.0));
        assert!(Session::gui_open_at(&ev, 90.0));
        assert!(Session::gui_open_at(&ev, 499.0));
        assert!(!Session::gui_open_at(&ev, 500.0));
    }
}
