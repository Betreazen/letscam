//! Player track: one sample per client tick (20 Hz) with eye position, view angles and flags.

use serde::{Deserialize, Serialize};

/// Minecraft convention: yaw in degrees, 0 = looking +Z (south), 90 = -X (west);
/// pitch in degrees, positive = looking down.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TrackSample {
    pub t_ms: u32,
    pub eye: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub gui_open: bool,
    pub fall_flying: bool,
    pub hurt_or_attacking: bool,
    pub dimension: String,
}

impl TrackSample {
    /// A plain sample (handy for tests and the glue).
    pub fn at(t_ms: u32, eye: [f64; 3], yaw: f32, pitch: f32) -> TrackSample {
        TrackSample {
            t_ms,
            eye,
            yaw,
            pitch,
            gui_open: false,
            fall_flying: false,
            hurt_or_attacking: false,
            dimension: "minecraft:overworld".to_string(),
        }
    }
}

/// Wraps degrees into (-180, 180].
pub fn wrap_deg(a: f64) -> f64 {
    let mut r = a % 360.0;
    if r <= -180.0 {
        r += 360.0;
    } else if r > 180.0 {
        r -= 360.0;
    }
    r
}

/// Shortest signed difference `to - from` in degrees, in (-180, 180].
pub fn delta_deg(from: f64, to: f64) -> f64 {
    wrap_deg(to - from)
}

/// Interpolates angles along the shortest arc.
pub fn lerp_deg(a: f64, b: f64, t: f64) -> f64 {
    a + delta_deg(a, b) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_and_delta() {
        assert_eq!(wrap_deg(190.0), -170.0);
        assert_eq!(wrap_deg(-190.0), 170.0);
        assert_eq!(wrap_deg(180.0), 180.0);
        assert_eq!(wrap_deg(-180.0), 180.0);
        assert!((delta_deg(179.0, -179.0) - 2.0).abs() < 1e-9);
        assert!((delta_deg(-179.0, 179.0) + 2.0).abs() < 1e-9);
        assert!((lerp_deg(179.0, -179.0, 0.5) - 180.0).abs() < 1e-9);
    }

    #[test]
    fn sample_serializes() {
        let s = TrackSample::at(50, [1.0, 2.0, 3.0], 90.0, -10.0);
        let json = serde_json::to_string(&s).expect("ser");
        let back: TrackSample = serde_json::from_str(&json).expect("de");
        assert_eq!(back, s);
    }
}
