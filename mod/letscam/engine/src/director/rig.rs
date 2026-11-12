//! The camera rig: turns a wish into a pose around an anchor point (the player's eyes or the
//! focus subject) — springs on offsets, ceiling/lintel height limit, walls, look angles.

use super::math::{UP, V3, add, dist, exp_alpha, len, pitch_of, scale, sub, yaw_of};
use super::probe::{WorldProbe, clear_distance, clearance_up};
use super::shots::Desired;
use super::spring;
use super::{CameraPose, Director, clearance};

/// Time constants of the height filter: fast down under ceilings/doors, slow back up.
const HEIGHT_DOWN_S: f64 = 0.12;
const HEIGHT_UP_S: f64 = 0.8;
/// Closer than this to the look target, the camera uses the fallback view angles.
const MIN_LOOK_DIST: f64 = 0.3;
/// The smoothed anchor height never lags more than this behind the real one, blocks.
const MAX_ANCHOR_LAG: f64 = 1.0;

/// One frame's input for [`Director::rig`].
pub(super) struct RigInput {
    pub t: f64,
    pub dt: f64,
    /// The point that must stay visible; offsets are measured from it.
    pub anchor: V3,
    /// View angles used when the camera is (almost) at the look target.
    pub fallback: (f64, f64),
    pub wish: Desired,
    pub cut: bool,
    pub spring_s: f64,
}

impl Director {
    /// The anchor with its height smoothed (jumps, steps), never more than `MAX_ANCHOR_LAG` off.
    fn update_anchor(&mut self, p: V3, dt: f64, cut: bool) -> V3 {
        if cut {
            self.anchor = (p[1], 0.0);
        } else {
            let (y, v) =
                spring::smooth_damp(self.anchor.0, self.anchor.1, p[1], self.cfg.spring_s, dt);
            self.anchor = (y.clamp(p[1] - MAX_ANCHOR_LAG, p[1] + MAX_ANCHOR_LAG), v);
        }
        [p[0], self.anchor.0, p[2]]
    }

    /// Pulls the camera towards `eye` so the line of sight is clear (fast in, slow out).
    fn resolve_walls(
        &mut self,
        world: &dyn WorldProbe,
        eye: V3,
        cand: V3,
        dt: f64,
        cut: bool,
    ) -> V3 {
        let c = &self.cfg;
        let full = dist(eye, cand);
        if full < 1e-6 {
            return cand;
        }
        let clear = clear_distance(world, eye, cand, c.wall_iterations);
        let allowed = if clear < full {
            (clear - c.wall_margin).max(0.0)
        } else {
            full
        };
        let need = full - allowed;
        if cut || need > self.pull {
            self.pull = need;
        } else {
            self.pull += (need - self.pull) * exp_alpha(dt, c.wall_push_out_s);
        }
        let used = (full - self.pull).clamp(0.0, allowed);
        add(eye, scale(sub(cand, eye), used / full))
    }

    /// Springs, height limit, walls and look angles for one frame.
    pub(super) fn rig(&mut self, world: &dyn WorldProbe, inp: RigInput) -> CameraPose {
        let c = self.cfg.clone();
        let RigInput {
            t,
            dt,
            anchor: eye,
            fallback,
            wish,
            cut,
            spring_s,
        } = inp;
        let over_eye = clearance_up(world, eye, c.ceiling_probe, c.wall_iterations);
        // Springs work on offsets from the (vertically smoothed) anchor: no lag behind a moving
        // subject, smooth changes of the shot, jumps softened.
        let anchor = self.update_anchor(eye, dt, cut);
        let rel = wish.relative_to(eye);
        let current_base = add(anchor, self.base.pos);
        let from = if cut { wish.base } else { current_base };
        let limit = clearance::height_limit(world, &c, eye, over_eye, &wish, from);
        let h_target = wish.height.min(limit);
        if cut {
            self.base.snap(rel.base);
            self.target.snap(rel.target);
            self.height = h_target;
        } else {
            self.base.update(rel.base, spring_s, dt);
            self.target.update(rel.target, spring_s, dt);
            let tau = if h_target < self.height {
                HEIGHT_DOWN_S
            } else {
                HEIGHT_UP_S
            };
            self.height += (h_target - self.height) * exp_alpha(dt, tau);
        }
        let cand = add(add(anchor, self.base.pos), scale(UP, self.height));
        let pos = self.resolve_walls(world, eye, cand, dt, cut);
        let look = sub(add(anchor, self.target.pos), pos);
        let (yaw, pitch) = if len(look) < MIN_LOOK_DIST {
            fallback
        } else {
            (yaw_of(look), pitch_of(look))
        };
        self.set_angles(yaw, pitch, dt, cut);
        self.finish_frame(t, pos)
    }

    /// Feeds the angle springs (snapped on a cut).
    pub(super) fn set_angles(&mut self, yaw: f64, pitch: f64, dt: f64, cut: bool) {
        if cut {
            self.yaw.snap(crate::track::wrap_deg(yaw));
            self.pitch.snap(pitch);
        } else {
            self.yaw.update(yaw, self.cfg.look_spring_s, dt);
            self.pitch.update(pitch, self.cfg.look_spring_s, dt);
        }
    }

    /// Records the frame and returns the pose with the current angles.
    pub(super) fn finish_frame(&mut self, t: f64, pos: V3) -> CameraPose {
        self.last_cam = pos;
        self.last_t = Some(t);
        CameraPose {
            pos,
            yaw: self.yaw.angle as f32,
            pitch: self.pitch.angle.clamp(-90.0, 90.0) as f32,
            fov: self.cfg.fov,
        }
    }
}
