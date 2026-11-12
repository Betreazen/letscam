//! Options of one render (spec section 10): camera preset, slow motion, contact sheet, focus.

use crate::director::{PRESET_WORDS, ShotPreset};

pub const MIN_SPEED: f64 = 0.05;
pub const MAX_SPEED: f64 = 1.0;
/// `sheet` without a number: one still every 2 game ticks.
pub const DEFAULT_SHEET_TICKS: u32 = 2;
const MAX_SHEET_TICKS: u32 = 1200;
const MAX_LABEL: usize = 40;

#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
    pub preset: ShotPreset,
    /// Replay speed: 1 = real time, 0.25 = four times slower (0.05..=1).
    pub speed: f64,
    /// Contact sheet: one still every N game ticks.
    pub sheet_ticks: Option<u32>,
    /// Text for the file name (usually the focus target).
    pub focus_label: Option<String>,
    /// What to focus on, as typed (`soulscraft:revenant`, an entity id…); the glue resolves it.
    pub focus: Option<String>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            preset: ShotPreset::Auto,
            speed: 1.0,
            sheet_ticks: None,
            focus_label: None,
            focus: None,
        }
    }
}

fn parse_speed(w: &str) -> Option<Result<f64, String>> {
    let num = w.strip_prefix('x').or_else(|| w.strip_suffix('x'))?;
    let value = num.replace(',', ".").parse::<f64>().ok()?;
    Some(
        if value.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(&value) {
            Ok(value)
        } else {
            Err(format!(
                "скорость «{w}» должна быть от {MIN_SPEED} до {MAX_SPEED} (например x0.25)"
            ))
        },
    )
}

fn parse_sheet(w: &str) -> Option<Result<u32, String>> {
    if w == "sheet" {
        return Some(Ok(DEFAULT_SHEET_TICKS));
    }
    let n = w.strip_prefix("sheet=")?;
    Some(
        n.parse::<u32>()
            .ok()
            .filter(|v| (1..=MAX_SHEET_TICKS).contains(v))
            .ok_or_else(|| format!("sheet={n}: нужно целое число тиков от 1 до {MAX_SHEET_TICKS}")),
    )
}

impl RenderOptions {
    /// Parses command words in any order: a preset word, `x0.25`/`0.25x`, `sheet`,
    /// `sheet=N`, `focus=…`. Errors are Russian text for the chat.
    pub fn parse(words: &[&str]) -> Result<RenderOptions, String> {
        let mut o = RenderOptions::default();
        let mut preset_word: Option<&str> = None;
        for raw in words {
            let w = raw.trim();
            if w.is_empty() {
                continue;
            }
            let lower = w.to_lowercase();
            if lower.starts_with("focus=") {
                let target = w.get(6..).unwrap_or_default().trim();
                if target.is_empty() {
                    return Err(
                        "focus= без цели: укажите, например, focus=soulscraft:revenant".into(),
                    );
                }
                o.focus = Some(target.to_string());
                o.focus_label = Some(target.to_string());
            } else if let Some(sheet) = parse_sheet(&lower) {
                o.sheet_ticks = Some(sheet?);
            } else if let Some(speed) = parse_speed(&lower) {
                o.speed = speed?;
            } else {
                let preset = w.parse::<ShotPreset>().map_err(|e| {
                    format!("{e}. Ещё можно: x0.25 (замедление), sheet или sheet=N, focus=…")
                })?;
                if let Some(prev) = preset_word {
                    return Err(format!(
                        "два ракурса сразу: «{prev}» и «{w}»; оставьте один ({PRESET_WORDS})"
                    ));
                }
                preset_word = Some(w);
                o.preset = preset;
            }
        }
        Ok(o)
    }

    /// Video file name: `letscam_<dir>[_<preset>][_<focus>][_x<speed>].<ext>`.
    pub fn file_name(&self, dir_name: &str, ext: &str) -> String {
        let mut name = format!("letscam_{dir_name}");
        if self.preset != ShotPreset::Auto {
            name.push('_');
            name.push_str(self.preset.word());
        }
        if let Some(label) = self
            .focus_label
            .as_deref()
            .map(sanitize)
            .filter(|l| !l.is_empty())
        {
            name.push('_');
            name.push_str(&label);
        }
        if self.speed < MAX_SPEED {
            name.push_str(&format!("_x{}", self.speed));
        }
        format!("{name}.{ext}")
    }
}

/// Keeps letters, digits, `-` and `_`; everything else becomes `-`. At most 40 characters.
pub fn sanitize(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(MAX_LABEL)
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_words_in_any_order() {
        let o = RenderOptions::parse(&["x0.25", "sheet=1", "orbit", "focus=soulscraft:revenant"])
            .expect("parse");
        assert_eq!(o.preset, ShotPreset::Orbit);
        assert_eq!(o.speed, 0.25);
        assert_eq!(o.sheet_ticks, Some(1));
        assert_eq!(o.focus.as_deref(), Some("soulscraft:revenant"));
        assert_eq!(o.focus_label.as_deref(), Some("soulscraft:revenant"));
        let o = RenderOptions::parse(&["sheet", "0.5x", "SIDE"]).expect("parse");
        assert_eq!(
            (o.preset, o.speed, o.sheet_ticks),
            (ShotPreset::Side, 0.5, Some(2))
        );
        let o = RenderOptions::parse(&["x0,1", "fixed:1,2,3"]).expect("comma decimal");
        assert_eq!(
            (o.speed, o.preset),
            (0.1, ShotPreset::Fixed([1.0, 2.0, 3.0]))
        );
        assert_eq!(
            RenderOptions::parse(&[]).expect("empty"),
            RenderOptions::default()
        );
        assert_eq!(RenderOptions::parse(&["", " x1 "]).expect("x1").speed, 1.0);
    }

    #[test]
    fn bad_words_give_russian_errors() {
        let err = |w: &[&str]| RenderOptions::parse(w).expect_err("must fail");
        assert!(err(&["x0.01"]).contains("скорость"));
        assert!(err(&["x2"]).contains("от 0.05 до 1"));
        assert!(err(&["sheet=0"]).contains("целое число"));
        assert!(err(&["sheet=abc"]).contains("целое число"));
        assert!(err(&["focus="]).contains("без цели"));
        assert!(err(&["zoom"]).contains("неизвестный ракурс"));
        assert!(err(&["orbit", "top"]).contains("два ракурса"));
        assert!(err(&["fixed:1,2"]).contains("x,y,z"));
    }

    #[test]
    fn file_names_keep_renders_apart() {
        let d = RenderOptions::default();
        assert_eq!(
            d.file_name("2026-11-12_10-00", "mp4"),
            "letscam_2026-11-12_10-00.mp4"
        );
        let o = RenderOptions {
            preset: ShotPreset::Side,
            speed: 0.25,
            focus_label: Some("soulscraft:revenant id=12".into()),
            ..RenderOptions::default()
        };
        assert_eq!(
            o.file_name("rec", "mkv"),
            "letscam_rec_side_soulscraft-revenant-id-12_x0.25.mkv"
        );
        let fixed = RenderOptions {
            preset: ShotPreset::Fixed([1.0, 2.0, 3.0]),
            ..d
        };
        assert_eq!(fixed.file_name("rec", "mp4"), "letscam_rec_fixed.mp4");
        assert_eq!(sanitize("::a/b\\c::"), "a-b-c");
        assert_eq!(sanitize(&"x".repeat(100)).len(), 40);
    }
}
