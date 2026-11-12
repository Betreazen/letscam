//! The subject the camera watches: the player, or a focus entity supplied per frame.

use std::collections::VecDeque;

use super::math::{V3, dist, len, sub, yaw_of};
use super::path::PlayerState;
use super::preset::Focus;

/// The player's body centre is this far below the eyes.
const PLAYER_CENTER_BELOW_EYE: f64 = 0.6;
/// A standing player is this tall; bigger focus entities push preset cameras further away.
const PLAYER_HEIGHT: f64 = 1.8;
/// Closer than this (horizontally), the player→subject line gives no direction.
const MIN_LINE: f64 = 0.5;
/// Movement below this over the motion window is not a direction.
const MIN_MOTION: f64 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SubjectKey {
    Player,
    Focus,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Subject {
    pub key: SubjectKey,
    /// Look-at point and anchor for presets.
    pub center: V3,
    /// Facing: the entity's yaw, else the player→subject line, else the player's yaw.
    pub facing: f64,
    /// Distances are multiplied by this for big entities (≥ 1).
    pub scale: f64,
}

impl Subject {
    pub fn of(state: &PlayerState, focus: Option<&Focus>) -> Subject {
        let Some(f) = focus else {
            return Subject {
                key: SubjectKey::Player,
                center: [
                    state.eye[0],
                    state.eye[1] - PLAYER_CENTER_BELOW_EYE,
                    state.eye[2],
                ],
                facing: state.yaw,
                scale: 1.0,
            };
        };
        let line = sub(f.pos, state.eye);
        let flat = [line[0], 0.0, line[2]];
        let facing = match f.yaw {
            Some(y) => f64::from(y),
            None if len(flat) > MIN_LINE => yaw_of(flat),
            None => state.yaw,
        };
        Subject {
            key: SubjectKey::Focus,
            center: f.pos,
            facing,
            scale: (f64::from(f.height) / PLAYER_HEIGHT).max(1.0),
        }
    }
}

/// Recent positions of the focus entity (the glue gives them frame by frame).
#[derive(Clone, Debug, Default)]
pub(super) struct FocusTrack {
    hist: VecDeque<(f64, V3)>,
    /// Last facing used for the auto camera (kept while the entity stands still).
    pub facing: Option<f64>,
}

impl FocusTrack {
    pub fn clear(&mut self) {
        self.hist.clear();
        self.facing = None;
    }

    /// Adds a position and forgets the ones older than `keep_ms`.
    pub fn push(&mut self, t: f64, pos: V3, keep_ms: f64) {
        self.hist.push_back((t, pos));
        while self.hist.front().is_some_and(|(ht, _)| *ht < t - keep_ms) {
            self.hist.pop_front();
        }
    }

    /// Largest distance from the newest position during the last `window_ms`, or `None`
    /// while the history is shorter than the window.
    pub fn max_move(&self, t: f64, window_ms: f64) -> Option<f64> {
        let (t0, _) = *self.hist.front()?;
        let (_, now) = *self.hist.back()?;
        if t - t0 < window_ms {
            return None;
        }
        Some(
            self.hist
                .iter()
                .filter(|(ht, _)| *ht >= t - window_ms)
                .map(|(_, p)| dist(*p, now))
                .fold(0.0, f64::max),
        )
    }

    /// Direction of horizontal movement over the last `window_ms`, if it moved enough.
    pub fn motion_yaw(&self, t: f64, window_ms: f64) -> Option<f64> {
        let (_, now) = *self.hist.back()?;
        let (_, then) = *self.hist.iter().find(|(ht, _)| *ht >= t - window_ms)?;
        let d = sub(now, then);
        let flat = [d[0], 0.0, d[2]];
        (len(flat) > MIN_MOTION).then(|| yaw_of(flat))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PlayerState {
        PlayerState {
            eye: [0.0, 65.0, 0.0],
            yaw: 30.0,
            pitch: 0.0,
            fall_flying: false,
            gui_flag: false,
        }
    }

    #[test]
    fn subject_facing_rules() {
        let s = Subject::of(&state(), None);
        assert_eq!(s.key, SubjectKey::Player);
        assert_eq!(s.facing, 30.0);
        let f = Focus {
            pos: [0.0, 65.0, 10.0],
            height: 3.6,
            yaw: Some(-90.0),
        };
        let s = Subject::of(&state(), Some(&f));
        assert_eq!((s.key, s.facing), (SubjectKey::Focus, -90.0));
        assert!((s.scale - 2.0).abs() < 1e-6);
        let s = Subject::of(&state(), Some(&Focus { yaw: None, ..f }));
        assert!(
            s.facing.abs() < 1e-9,
            "player→subject line points +Z: {}",
            s.facing
        );
        let near = Focus {
            pos: [0.1, 64.0, 0.1],
            height: 1.0,
            yaw: None,
        };
        assert_eq!(Subject::of(&state(), Some(&near)).facing, 30.0);
    }

    #[test]
    fn focus_track_motion_and_idle() {
        let mut ft = FocusTrack::default();
        for i in 0..40 {
            let t = f64::from(i) * 50.0;
            ft.push(t, [f64::from(i) * 0.1, 64.0, 0.0], 3000.0);
        }
        let yaw = ft.motion_yaw(1950.0, 500.0).expect("moving");
        assert!((yaw + 90.0).abs() < 1e-6, "moving +X is yaw -90: {yaw}");
        assert!(ft.max_move(1950.0, 1000.0).expect("covered") > 1.0);
        assert_eq!(ft.max_move(1950.0, 5000.0), None);
        ft.clear();
        assert!(ft.motion_yaw(0.0, 500.0).is_none());
    }
}
