//! Mod settings (`config/letscam-client.toml`): load, create with comments, clamp.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub use crate::director::DirectorConfig;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Mp4,
    Mkv,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mkv => "mkv",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub ffmpeg: String,
    pub output_dir: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: Codec,
    pub hardware: bool,
    pub quality: u32,
    pub container: Container,
    pub record_mic: bool,
    pub mic_device: String,
    pub keep_sources: bool,
    pub gui_fps: u32,
    pub director: DirectorConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ffmpeg: "ffmpeg".to_string(),
            output_dir: String::new(),
            width: 1920,
            height: 1080,
            fps: 60,
            codec: Codec::Hevc,
            hardware: true,
            quality: 23,
            container: Container::Mp4,
            record_mic: true,
            mic_device: String::new(),
            keep_sources: true,
            gui_fps: 4,
            director: DirectorConfig::default(),
        }
    }
}

const MIN_W: u32 = 320;
const MAX_W: u32 = 7680;
const MIN_H: u32 = 240;
const MAX_H: u32 = 4320;

/// Rounds down to an even number (video encoders need even sizes).
fn even(v: u32) -> u32 {
    v & !1
}

impl Config {
    /// Reads the settings file. A missing file is created with defaults and Russian comments.
    /// A broken file is left untouched: defaults are used and a warning text is returned.
    pub fn load_or_create(path: &Path) -> (Config, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(cfg) => (cfg.validated(), None),
                Err(e) => (
                    Config::default(),
                    Some(format!(
                        "LetsCam: ошибка в файле настроек {}: {e}. Используются настройки по умолчанию, файл не изменён.",
                        path.display()
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let cfg = Config::default();
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(path, crate::config_template::render(&cfg)));
                match written {
                    Ok(()) => (cfg, None),
                    Err(e) => (
                        cfg,
                        Some(format!(
                            "LetsCam: не удалось создать файл настроек {}: {e}",
                            path.display()
                        )),
                    ),
                }
            }
            Err(e) => (
                Config::default(),
                Some(format!(
                    "LetsCam: не удалось прочитать файл настроек {}: {e}. Используются настройки по умолчанию.",
                    path.display()
                )),
            ),
        }
    }

    /// Returns a copy with all values clamped into supported ranges.
    pub fn validated(self) -> Config {
        let d = Config::default();
        Config {
            ffmpeg: if self.ffmpeg.trim().is_empty() {
                d.ffmpeg
            } else {
                self.ffmpeg.trim().to_string()
            },
            output_dir: self.output_dir.trim().to_string(),
            width: even(self.width.clamp(MIN_W, MAX_W)),
            height: even(self.height.clamp(MIN_H, MAX_H)),
            fps: if self.fps <= 45 { 30 } else { 60 },
            quality: self.quality.clamp(1, 51),
            gui_fps: self.gui_fps.clamp(1, 30),
            director: self.director.validated(),
            ..self
        }
    }

    /// Same settings with software encoding (used when NVENC is not available).
    pub fn with_software(&self) -> Config {
        Config {
            hardware: false,
            ..self.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    #[test]
    fn defaults_round_trip_through_toml() {
        let cfg = Config::default();
        let text = toml::to_string(&cfg).expect("serialize");
        assert!(text.contains("codec = \"hevc\""));
        assert!(text.contains("container = \"mp4\""));
        let back: Config = toml::from_str(&text).expect("parse");
        assert_eq!(back, cfg);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let cfg: Config = toml::from_str(
            "fps = 30\ncodec = \"av1\"\ncontainer = \"mkv\"\n[director]\ngui_radius = 5.0\n",
        )
        .expect("parse");
        assert_eq!(cfg.fps, 30);
        assert_eq!(cfg.codec, Codec::Av1);
        assert_eq!(cfg.container, Container::Mkv);
        assert_eq!(cfg.width, 1920);
        assert_eq!(cfg.director.gui_radius, 5.0);
        assert_eq!(cfg.director.follow_distance, 4.5);
    }

    #[test]
    fn invalid_values_are_clamped() {
        let cfg = Config {
            ffmpeg: "  ".into(),
            width: 99_999,
            height: 1081,
            fps: 144,
            quality: 0,
            gui_fps: 0,
            director: DirectorConfig {
                spring_s: -1.0,
                shoulder_every_min_s: 20.0,
                shoulder_every_max_s: 10.0,
                follow_distance: f64::NAN,
                ..DirectorConfig::default()
            },
            ..Config::default()
        }
        .validated();
        assert_eq!(cfg.ffmpeg, "ffmpeg");
        assert_eq!(cfg.width, 7680);
        assert_eq!(cfg.height, 1080);
        assert_eq!(cfg.fps, 60);
        assert_eq!(cfg.quality, 1);
        assert_eq!(cfg.gui_fps, 1);
        assert_eq!(cfg.director.spring_s, 0.02);
        assert_eq!(cfg.director.shoulder_every_min_s, 10.0);
        assert_eq!(cfg.director.shoulder_every_max_s, 20.0);
        assert_eq!(cfg.director.follow_distance, 4.5);
        assert_eq!(
            Config {
                fps: 24,
                ..Config::default()
            }
            .validated()
            .fps,
            30
        );
        assert_eq!(
            Config {
                fps: 50,
                ..Config::default()
            }
            .validated()
            .fps,
            60
        );
    }

    #[test]
    fn missing_file_is_created_with_comments() {
        let tmp = TempDir::new("cfg-new");
        let path = tmp.path().join("config").join("letscam-client.toml");
        let (cfg, warn) = Config::load_or_create(&path);
        assert!(warn.is_none(), "{warn:?}");
        assert_eq!(cfg, Config::default());
        let text = std::fs::read_to_string(&path).expect("file written");
        assert!(text.contains("# "), "has comments");
        assert!(text.contains("Путь к ffmpeg"));
        let (again, warn) = Config::load_or_create(&path);
        assert!(warn.is_none());
        assert_eq!(again, Config::default());
    }

    #[test]
    fn broken_file_gives_defaults_and_warning_and_is_untouched() {
        let tmp = TempDir::new("cfg-broken");
        let path = tmp.path().join("letscam-client.toml");
        let broken = "fps = = 60\n[[[";
        std::fs::write(&path, broken).expect("write");
        let (cfg, warn) = Config::load_or_create(&path);
        assert_eq!(cfg, Config::default());
        let warn = warn.expect("warning");
        assert!(warn.contains("ошибка"), "{warn}");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), broken);
    }

    #[test]
    fn loaded_values_are_validated() {
        let tmp = TempDir::new("cfg-clamp");
        let path = tmp.path().join("c.toml");
        std::fs::write(&path, "fps = 25\nwidth = 10\n").expect("write");
        let (cfg, warn) = Config::load_or_create(&path);
        assert!(warn.is_none());
        assert_eq!(cfg.fps, 30);
        assert_eq!(cfg.width, 320);
    }
}
