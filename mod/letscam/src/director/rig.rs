//! The camera rig: turns a wish into a pose around an anchor point (the player's eyes or the
//! focus subject) — springs on offsets, ceiling/lintel height limit, walls, look angles.

use super::detour::{self, Sight, near, sees};
use super::math::{UP, V3, add, dist, exp_alpha, len, pitch_of, scale, sub, yaw_dir, yaw_of};
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
/// A camera move longer than this between two frames is treated as a cut, blocks.
const MAX_STEP: f64 = 0.6;
/// How far a rescued camera moves from its previous spot towards the goal per frame.
const STEP_TOWARDS: f64 = 0.4;
/// The smoothed view never trails the wanted one by more than this, degrees.
const MAX_LOOK_LAG_DEG: f64 = 12.0;

/// What to do with a smoothed pose that fails validation.
enum Rescue {
    Keep,
    Move(V3),
    Cut,
}

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
    /// Points of the subject the camera must see (eyes and body, or the entity).
    pub sight: Sight,
    /// The camera keeps at least this far from the sight points when it can.
    pub min_dist: f64,
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

    /// If the wished spot cannot see the subject, returns a clear spot to head for instead
    /// (kept while it stays clear, so the camera does not hop between alternatives).
    fn steer(
        &mut self,
        world: &dyn WorldProbe,
        eye: V3,
        sight: &Sight,
        wish_spot: V3,
        min_dist: f64,
    ) -> Option<V3> {
        let ok = |p: V3| sees(world, p, sight) && near(p, sight) >= min_dist;
        if ok(wish_spot) {
            self.detour = None;
            return None;
        }
        if let Some(p) = self.detour.map(|off| add(eye, off)).filter(|p| ok(*p)) {
            return Some(p);
        }
        // Continuity: measure candidates against where the camera is now, moved with the subject.
        let prev = if self.last_t.is_some() {
            add(self.last_cam, sub(eye, self.last_eye))
        } else {
            wish_spot
        };
        let found = detour::search(world, eye, sight, wish_spot, prev, min_dist);
        self.detour = found.map(|p| sub(p, eye));
        found
    }

    /// Springs, height limit, detours, walls and look angles for one frame.
    pub(super) fn rig(&mut self, world: &dyn WorldProbe, inp: RigInput) -> CameraPose {
        let c = self.cfg.clone();
        let RigInput {
            t,
            dt,
            anchor: eye,
            fallback,
            wish,
            mut cut,
            spring_s,
            sight,
            min_dist,
        } = inp;
        let over_eye = clearance_up(world, eye, c.ceiling_probe, c.wall_iterations);
        // Springs work on offsets from the (vertically smoothed) anchor: no lag behind a moving
        // subject, smooth changes of the shot, jumps softened.
        let anchor = self.update_anchor(eye, dt, cut);
        let mut rel = wish.relative_to(eye);
        let current_base = add(anchor, self.base.pos);
        let from = if cut { wish.base } else { current_base };
        let limit = clearance::height_limit(world, &c, eye, over_eye, &wish, from);
        let mut h_target = wish.height.min(limit);
        let wish_spot = add(wish.base, scale(UP, h_target));
        if let Some(spot) = self.steer(world, eye, &sight, wish_spot, min_dist) {
            rel.base = sub(spot, eye);
            h_target = 0.0;
        }
        if cut {
            self.snap_to(rel.base, rel.target, h_target);
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
        let mut pos = self.resolve_walls(world, eye, cand, dt, cut);
        // Validate the final, smoothed pose: never look at a wall or into the head.
        let goal = add(add(eye, rel.base), scale(UP, h_target));
        if !cut {
            match self.validate(world, eye, &sight, min_dist, pos, goal) {
                Rescue::Keep => {}
                Rescue::Move(p) => {
                    self.base.snap(sub(sub(p, anchor), scale(UP, self.height)));
                    self.pull = 0.0;
                    pos = p;
                }
                Rescue::Cut => {
                    self.snap_to(rel.base, rel.target, h_target);
                    pos = goal;
                    cut = true;
                }
            }
        }
        // A jump this big relative to the subject reads as a cut anyway; let the view snap too.
        if self.last_t.is_some()
            && dist(sub(pos, eye), sub(self.last_cam, self.last_eye)) > MAX_STEP
        {
            cut = true;
        }
        self.cut_flag = cut;
        self.last_eye = eye;
        let look = sub(add(anchor, self.target.pos), pos);
        let (yaw, pitch) = if len(look) < MIN_LOOK_DIST {
            fallback
        } else {
            (yaw_of(look), pitch_of(look))
        };
        self.set_angles(yaw, pitch, dt, cut);
        self.finish_frame(t, pos)
    }

    /// Checks the smoothed pose `pos`; if it cannot see the subject (or is too close, or
    /// jumped), prefers a short step from the previous spot towards `goal`, then holding the
    /// previous spot (moving with the subject), and only then a cut to `goal`.
    fn validate(
        &self,
        world: &dyn WorldProbe,
        eye: V3,
        sight: &Sight,
        min_dist: f64,
        pos: V3,
        goal: V3,
    ) -> Rescue {
        let fine = |p: V3| sees(world, p, sight) && near(p, sight) >= min_dist;
        let Some(held_rel) = self.last_t.map(|_| sub(self.last_cam, self.last_eye)) else {
            return Rescue::Keep;
        };
        let smooth = |p: V3| dist(sub(p, eye), held_rel) <= MAX_STEP;
        if fine(pos) && smooth(pos) {
            return Rescue::Keep;
        }
        let held = add(eye, held_rel);
        let to_goal = sub(goal, held);
        let towards = scale(to_goal, (STEP_TOWARDS / len(to_goal).max(1e-9)).min(1.0));
        // Small moves from the previous spot, the ones nearest the goal first.
        let mut moves: Vec<V3> = vec![towards, [0.0, STEP_TOWARDS, 0.0], [0.0; 3]];
        moves.extend((0..8).map(|k| {
            let a = f64::from(k) * 45.0;
            add(
                scale(yaw_dir(a), STEP_TOWARDS),
                [0.0, STEP_TOWARDS * 0.5, 0.0],
            )
        }));
        let mut spots: Vec<V3> = moves.into_iter().map(|m| add(held, m)).collect();
        spots.sort_by(|a, b| dist(*a, goal).total_cmp(&dist(*b, goal)));
        if let Some(p) = spots.into_iter().find(|p| fine(*p)) {
            return Rescue::Move(p);
        }
        // The wall-pulled pose lies on the clear ray to the subject: a jump, but never through
        // a wall. Only when it is no good either, cut to the clear goal.
        if !fine(pos) && fine(goal) {
            Rescue::Cut
        } else {
            Rescue::Keep
        }
    }

    /// Puts the springs straight at the goal (a cut).
    fn snap_to(&mut self, base: V3, target: V3, height: f64) {
        self.base.snap(base);
        self.target.snap(target);
        self.height = height;
        self.pull = 0.0;
    }

    /// Feeds the angle springs (snapped on a cut).
    pub(super) fn set_angles(&mut self, yaw: f64, pitch: f64, dt: f64, cut: bool) {
        if cut {
            self.yaw.snap(crate::track::wrap_deg(yaw));
            self.pitch.snap(pitch);
        } else {
            self.yaw.update(yaw, self.cfg.look_spring_s, dt);
            self.pitch.update(pitch, self.cfg.look_spring_s, dt);
            // Smooth, but never so late that the subject leaves the frame.
            let dy = crate::track::delta_deg(yaw, self.yaw.angle);
            if dy.abs() > MAX_LOOK_LAG_DEG {
                self.yaw.angle = crate::track::wrap_deg(yaw + MAX_LOOK_LAG_DEG.copysign(dy));
            }
            self.pitch.angle = self
                .pitch
                .angle
                .clamp(pitch - MAX_LOOK_LAG_DEG, pitch + MAX_LOOK_LAG_DEG);
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
