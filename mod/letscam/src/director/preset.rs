//! Camera presets for inspecting animations (spec section 10) and the focus subject.

use std::str::FromStr;

/// Which camera the render uses. `Auto` is the director of spec section 6.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ShotPreset {
    #[default]
    Auto,
    FirstPerson,
    Shoulder,
    Orbit,
    Side,
    Front,
    Top,
    Fixed([f64; 3]),
    /// The camera path the player keyed in the replay (Horizon's studio track, spec section 15),
    /// posed by the glue; the director frames it like `Auto` where the path has no keyframes.
    Keys,
}

impl ShotPreset {
    /// Short word used in file names and logs.
    pub fn word(&self) -> &'static str {
        match self {
            ShotPreset::Auto => "auto",
            ShotPreset::FirstPerson => "fp",
            ShotPreset::Shoulder => "shoulder",
            ShotPreset::Orbit => "orbit",
            ShotPreset::Side => "side",
            ShotPreset::Front => "front",
            ShotPreset::Top => "top",
            ShotPreset::Fixed(_) => "fixed",
            ShotPreset::Keys => "keys",
        }
    }
}

pub const PRESET_WORDS: &str = "auto, fp, shoulder, orbit, side, front, top, fixed:x,y,z, keys";

impl FromStr for ShotPreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let w = s.trim().to_lowercase();
        if let Some(rest) = w.strip_prefix("fixed:") {
            return parse_point(rest).map(ShotPreset::Fixed);
        }
        Ok(match w.as_str() {
            "auto" => ShotPreset::Auto,
            "fp" | "first" => ShotPreset::FirstPerson,
            "shoulder" => ShotPreset::Shoulder,
            "orbit" => ShotPreset::Orbit,
            "side" => ShotPreset::Side,
            "front" => ShotPreset::Front,
            "top" => ShotPreset::Top,
            "keys" | "ключи" => ShotPreset::Keys,
            "fixed" => return Err("для fixed нужна точка: fixed:x,y,z".into()),
            _ => return Err(format!("неизвестный ракурс «{s}»; есть: {PRESET_WORDS}")),
        })
    }
}

fn parse_point(s: &str) -> Result<[f64; 3], String> {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    let bad = || format!("точка «{s}» должна быть вида x,y,z (три числа через запятую)");
    if parts.len() != 3 {
        return Err(bad());
    }
    let mut p = [0.0; 3];
    for (dst, src) in p.iter_mut().zip(&parts) {
        *dst = src
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(bad)?;
    }
    Ok(p)
}

/// The entity the camera should watch instead of the player, given per frame by the glue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    /// Centre of the subject (middle of its bounding box).
    pub pos: [f64; 3],
    /// Height of the subject, blocks.
    pub height: f32,
    /// Facing (Minecraft yaw) if the entity has one.
    pub yaw: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_words() {
        let p = |s: &str| s.parse::<ShotPreset>();
        assert_eq!(p("auto"), Ok(ShotPreset::Auto));
        assert_eq!(p("FP"), Ok(ShotPreset::FirstPerson));
        assert_eq!(p("keys"), Ok(ShotPreset::Keys));
        assert_eq!(p("Ключи"), Ok(ShotPreset::Keys));
        assert_eq!(ShotPreset::Keys.word(), "keys");
        assert_eq!(p("first"), Ok(ShotPreset::FirstPerson));
        assert_eq!(p("shoulder"), Ok(ShotPreset::Shoulder));
        assert_eq!(p("orbit"), Ok(ShotPreset::Orbit));
        assert_eq!(p("side"), Ok(ShotPreset::Side));
        assert_eq!(p("front"), Ok(ShotPreset::Front));
        assert_eq!(p("top"), Ok(ShotPreset::Top));
        assert_eq!(
            p("fixed:1, -2.5,3"),
            Ok(ShotPreset::Fixed([1.0, -2.5, 3.0]))
        );
    }

    #[test]
    fn rejects_bad_words_in_russian() {
        let e = "spin".parse::<ShotPreset>().expect_err("bad");
        assert!(e.contains("неизвестный ракурс"), "{e}");
        assert!("fixed".parse::<ShotPreset>().is_err());
        assert!(
            "fixed:1,2"
                .parse::<ShotPreset>()
                .expect_err("bad")
                .contains("x,y,z")
        );
        assert!("fixed:1,2,z".parse::<ShotPreset>().is_err());
        assert!("fixed:1,2,inf".parse::<ShotPreset>().is_err());
    }
}
