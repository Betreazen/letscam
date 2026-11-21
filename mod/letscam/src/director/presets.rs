//! Wishes of the inspection presets (spec section 10) and of the automatic camera when it
//! follows a focus entity instead of the player.

use super::Director;
use super::math::{V3, add, lerp1, pitch_of, scale, sub, view_dir, yaw_dir, yaw_of};
use super::path::PlayerState;
use super::preset::{Focus, ShotPreset};
use super::shots::{Desired, ShotKind};
use super::subject::Subject;
use crate::track::lerp_deg;

/// The shoulder camera aims at a point this far along its view.
const AIM_DISTANCE: f64 = 10.0;
/// Top view pitch (straight down would make the yaw meaningless).
const TOP_PITCH_DEG: f64 = 89.0;
/// Motion over this window gives the focus entity's direction when it has no yaw.
const MOTION_WINDOW_MS: f64 = 500.0;

fn around(center: V3, yaw: f64, distance: f64, height: f64) -> Desired {
    Desired {
        base: add(center, scale(yaw_dir(yaw), distance)),
        height,
        target: center,
    }
}

impl Director {
    /// Wish, anchor (point kept in sight) and fallback angles for a non-auto preset.
    pub(super) fn preset_wish(
        &mut self,
        t: f64,
        state: &PlayerState,
        subject: &Subject,
        focus: Option<&Focus>,
        cut: bool,
    ) -> (Desired, V3, (f64, f64)) {
        let c = &self.cfg;
        let k = subject.scale;
        let center = subject.center;
        let facing = subject.facing;
        let fallback = (facing, 0.0);
        match self.preset {
            ShotPreset::Shoulder => {
                self.name = "over_shoulder";
                let pos = add(
                    add(
                        state.eye,
                        scale(view_dir(state.yaw, state.pitch), -c.shoulder_back),
                    ),
                    scale(yaw_dir(state.yaw + 90.0), c.shoulder_right),
                );
                let (mut yaw, mut pitch) = (state.yaw, state.pitch);
                if let Some(f) = focus {
                    let to = sub(f.pos, pos);
                    yaw = lerp_deg(yaw, yaw_of(to), c.shoulder_focus_turn);
                    pitch = lerp1(pitch, pitch_of(to), c.shoulder_focus_turn);
                }
                let wish = Desired {
                    base: pos,
                    height: 0.0,
                    target: add(pos, scale(view_dir(yaw, pitch), AIM_DISTANCE)),
                };
                (wish, state.eye, (yaw, pitch))
            }
            ShotPreset::Orbit => {
                self.name = "orbit";
                if cut {
                    self.orbit0 = (facing + 180.0, t);
                }
                let turn = 360.0 * (t - self.orbit0.1) / (c.orbit_period_s * 1000.0);
                let wish = around(
                    center,
                    self.orbit0.0 + turn,
                    c.orbit_radius * k,
                    c.orbit_height * k,
                );
                (wish, center, fallback)
            }
            ShotPreset::Side => {
                self.name = "side";
                let wish = around(
                    center,
                    facing + 90.0,
                    c.side_distance * k,
                    c.side_height * k,
                );
                (wish, center, fallback)
            }
            ShotPreset::Front => {
                self.name = "front";
                let wish = around(center, facing, c.front_distance * k, c.front_height * k);
                (wish, center, fallback)
            }
            _ => {
                self.name = "top";
                let h = c.top_height * k;
                let ahead = h * (90.0 - TOP_PITCH_DEG).to_radians().tan();
                let wish = Desired {
                    base: center,
                    height: h,
                    target: add(center, scale(yaw_dir(facing), ahead)),
                };
                (wish, center, (facing, TOP_PITCH_DEG))
            }
        }
    }

    /// Automatic camera around a focus entity: behind it (by its yaw or movement), orbiting
    /// when it stands still; shot changes blend like the player director's.
    pub(super) fn focus_auto_wish(
        &mut self,
        t: f64,
        subject: &Subject,
        f: &Focus,
        cut: bool,
    ) -> Desired {
        let c = self.cfg.clone();
        let center = subject.center;
        let facing = f
            .yaw
            .map(f64::from)
            .or_else(|| self.focus_track.motion_yaw(t, MOTION_WINDOW_MS))
            .or(self.focus_track.facing)
            .unwrap_or(subject.facing);
        self.focus_track.facing = Some(facing);
        let idle = self
            .focus_track
            .max_move(t, c.focus_idle_s * 1000.0)
            .is_some_and(|m| m <= c.idle_move_epsilon);
        let kind = if idle {
            ShotKind::Idle
        } else {
            ShotKind::Follow
        };
        self.switch_shot(t, kind, center, facing + 180.0, cut);
        let k = subject.scale;
        let wish = if kind == ShotKind::Idle {
            self.name = "focus_orbit";
            let turn = 360.0 * (t - self.shot.start_ms) / (c.idle_period_s * 1000.0);
            around(
                center,
                self.shot.orbit_deg0 + turn,
                c.idle_radius * k,
                c.idle_height * k,
            )
        } else {
            self.name = "focus_follow";
            let (d, h) = (c.follow_distance * k, c.follow_height * k);
            let back = (d * d - h * h).max(0.25).sqrt();
            Desired {
                base: add(center, scale(yaw_dir(facing), -back)),
                height: h,
                target: add(center, scale(yaw_dir(facing), c.look_ahead)),
            }
        };
        self.blended(t, center, wish)
    }
}
