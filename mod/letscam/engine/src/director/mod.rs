//! The automatic "director" camera (spec section 6). Pure logic: the track and world probes
//! go in, a camera pose per output frame comes out. Deterministic for the same input.

mod clearance;
mod math;
mod path;
mod probe;
mod rng;
mod settings;
mod shots;
mod spring;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_shots;
#[cfg(test)]
mod tests_world;

use std::sync::Arc;

pub use probe::{OpenWorld, WorldProbe};
pub use settings::DirectorConfig;

use math::{UP, V3, add, dist, exp_alpha, len, pitch_of, scale, smoothstep, sub, yaw_of};
use path::{PlayerState, TrackIndex};
use probe::{clear_distance, clearance_up};
use rng::ShoulderSchedule;
use shots::{Desired, Shot, ShotCtx, ShotKind};
use spring::{AngleSpring, Spring3};

use crate::session::{GuiEvent, Session};
use crate::track::TrackSample;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraPose {
    pub pos: [f64; 3],
    /// Minecraft convention, degrees, wrapped to (-180, 180].
    pub yaw: f32,
    pub pitch: f32,
    pub fov: f32,
}

/// Time constants of the height filter: fast down under ceilings/doors, slow back up.
const HEIGHT_DOWN_S: f64 = 0.12;
const HEIGHT_UP_S: f64 = 0.8;
/// Closer than this to the look target, the camera uses the player's own view angles.
const MIN_LOOK_DIST: f64 = 0.3;
/// The smoothed eye height never lags more than this behind the real one, blocks.
const MAX_ANCHOR_LAG: f64 = 1.0;

#[derive(Clone, Debug)]
struct Blend {
    /// The blended wish at the moment of the switch, relative to the eye.
    from: Desired,
    start_ms: f64,
}

#[derive(Clone)]
pub struct Director {
    cfg: DirectorConfig,
    index: Arc<TrackIndex>,
    gui_events: Arc<Vec<GuiEvent>>,
    shoulder: Arc<ShoulderSchedule>,
    shot: Shot,
    name: &'static str,
    blend: Option<Blend>,
    last_t: Option<f64>,
    handled_trigger: Option<f64>,
    last_wish: Desired,
    base: Spring3,
    target: Spring3,
    height: f64,
    pull: f64,
    anchor: (f64, f64),
    yaw: AngleSpring,
    pitch: AngleSpring,
    last_cam: V3,
}

impl Director {
    pub fn new(
        track: Vec<TrackSample>,
        mut gui_events: Vec<GuiEvent>,
        cfg: DirectorConfig,
        seed: u64,
    ) -> Director {
        let cfg = cfg.validated();
        let index = TrackIndex::new(track, cfg.teleport_distance);
        gui_events.sort_by_key(|e| e.t_ms);
        let shoulder = ShoulderSchedule::new(&cfg, index.first_t(), index.last_t(), seed);
        let zero = Desired {
            base: [0.0; 3],
            height: 0.0,
            target: [0.0; 3],
        };
        Director {
            cfg,
            index: Arc::new(index),
            gui_events: Arc::new(gui_events),
            shoulder: Arc::new(shoulder),
            shot: Shot {
                kind: ShotKind::Follow,
                start_ms: 0.0,
                orbit_deg0: 0.0,
            },
            name: ShotKind::Follow.name(),
            blend: None,
            last_t: None,
            handled_trigger: None,
            last_wish: zero,
            base: Spring3::new([0.0; 3]),
            target: Spring3::new([0.0; 3]),
            height: 0.0,
            pull: 0.0,
            anchor: (0.0, 0.0),
            yaw: AngleSpring::new(0.0),
            pitch: AngleSpring::new(0.0),
            last_cam: [0.0; 3],
        }
    }

    /// Name of the current shot (for logs and tests).
    pub fn shot_name(&self) -> &'static str {
        self.name
    }

    /// The pose at `t_ms` computed on a copy in an empty world: no probing, no state change.
    pub fn peek_pose(&self, t_ms: f64) -> CameraPose {
        self.clone().pose(t_ms, &OpenWorld)
    }

    fn gui_open(&self, t: f64, state: &PlayerState) -> bool {
        if self.gui_events.is_empty() {
            state.gui_flag
        } else {
            Session::gui_open_at(&self.gui_events, t)
        }
    }

    fn pick_kind(&self, t: f64, state: &PlayerState) -> ShotKind {
        let c = &self.cfg;
        if self.shot.kind == ShotKind::Establishing
            && t - self.shot.start_ms < c.establishing_s * 1000.0
        {
            return ShotKind::Establishing;
        }
        if self.gui_open(t, state) {
            return ShotKind::Gui;
        }
        if state.fall_flying {
            return ShotKind::Elytra;
        }
        if self
            .index
            .last_hurt(t)
            .is_some_and(|h| t - h <= c.combat_window_s * 1000.0)
        {
            return ShotKind::Combat;
        }
        let idle = self
            .index
            .max_move(t, c.idle_after_s * 1000.0)
            .is_some_and(|m| m <= c.idle_move_epsilon);
        if idle {
            ShotKind::Idle
        } else {
            ShotKind::Follow
        }
    }

    /// Switches shots; returns true when the camera must cut (springs snap).
    fn update_shot(&mut self, t: f64, state: &PlayerState) -> bool {
        let first = self.last_t.is_none();
        let trigger = self.index.last_trigger(t);
        let mut cut = first;
        if trigger != self.handled_trigger {
            self.handled_trigger = trigger;
            if let Some(tr) = trigger
                && t - tr < self.cfg.establishing_s * 1000.0
            {
                self.shot = Shot {
                    kind: ShotKind::Establishing,
                    start_ms: tr,
                    orbit_deg0: 0.0,
                };
                self.blend = None;
                cut = true;
            }
        }
        let kind = self.pick_kind(t, state);
        if kind != self.shot.kind {
            let orbit_deg0 = if cut {
                state.yaw + 180.0
            } else {
                yaw_of(sub(self.last_cam, state.eye))
            };
            if !cut {
                self.blend = Some(Blend {
                    from: self.last_wish,
                    start_ms: t,
                });
            }
            self.shot = Shot {
                kind,
                start_ms: t,
                orbit_deg0,
            };
        }
        cut
    }

    /// The (blended) wish of the current shot for this frame.
    fn wish(&mut self, t: f64, state: &PlayerState, reach: f64) -> Desired {
        let ctx = ShotCtx {
            index: &self.index,
            cfg: &self.cfg,
            t,
            state,
            reach,
            shoulder_deg: self.shoulder.angle_at(t),
        };
        let mut wish = self.shot.desired(&ctx);
        if let Some(b) = &self.blend {
            let blend_ms = self.cfg.blend_s * 1000.0;
            let w = if blend_ms <= 0.0 {
                1.0
            } else {
                smoothstep((t - b.start_ms) / blend_ms)
            };
            wish = b.from.absolute_from(state.eye).lerp(wish, w);
            if w >= 1.0 {
                self.blend = None;
            }
        }
        self.name = if self.shot.kind == ShotKind::Follow && ctx.shoulder_deg.abs() > 1.0 {
            "shoulder"
        } else {
            self.shot.kind.name()
        };
        self.last_wish = wish.relative_to(state.eye);
        wish
    }

    /// The eye with its height smoothed (jumps, steps), never more than `MAX_ANCHOR_LAG` off.
    fn update_anchor(&mut self, eye: V3, dt: f64, cut: bool) -> V3 {
        if cut {
            self.anchor = (eye[1], 0.0);
        } else {
            let (y, v) =
                spring::smooth_damp(self.anchor.0, self.anchor.1, eye[1], self.cfg.spring_s, dt);
            self.anchor = (y.clamp(eye[1] - MAX_ANCHOR_LAG, eye[1] + MAX_ANCHOR_LAG), v);
        }
        [eye[0], self.anchor.0, eye[2]]
    }

    /// Pulls the camera towards the eye so the line of sight is clear (fast in, slow out).
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

    /// Camera pose for the output frame at `t_ms` (call with increasing times, 1000/fps apart).
    pub fn pose(&mut self, t_ms: f64, world: &dyn WorldProbe) -> CameraPose {
        let c = self.cfg.clone();
        let Some(state) = self.index.state_at(t_ms) else {
            return CameraPose {
                pos: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                fov: c.fov,
            };
        };
        let dt = self
            .last_t
            .map_or(0.0, |l| ((t_ms - l) / 1000.0).clamp(0.0, 0.5));
        let cut = self.update_shot(t_ms, &state);
        let over_eye = clearance_up(world, state.eye, c.ceiling_probe, c.wall_iterations);
        let reach = (over_eye / c.ceiling_probe).clamp(0.0, 1.0);
        let wish = self.wish(t_ms, &state, reach);
        // Springs work on offsets from the (vertically smoothed) eye: no lag behind a walking
        // player, smooth changes of the shot, jumps softened by the anchor.
        let anchor = self.update_anchor(state.eye, dt, cut);
        let rel = wish.relative_to(state.eye);
        let current_base = add(anchor, self.base.pos);
        let from = if cut { wish.base } else { current_base };
        let limit = clearance::height_limit(world, &c, state.eye, over_eye, &wish, from);
        let h_target = wish.height.min(limit);
        if cut {
            self.base.snap(rel.base);
            self.target.snap(rel.target);
            self.height = h_target;
        } else {
            self.base.update(rel.base, c.spring_s, dt);
            self.target.update(rel.target, c.spring_s, dt);
            let tau = if h_target < self.height {
                HEIGHT_DOWN_S
            } else {
                HEIGHT_UP_S
            };
            self.height += (h_target - self.height) * exp_alpha(dt, tau);
        }
        let cand = add(add(anchor, self.base.pos), scale(UP, self.height));
        let pos = self.resolve_walls(world, state.eye, cand, dt, cut);
        let look = sub(add(anchor, self.target.pos), pos);
        let (yaw, pitch) = if len(look) < MIN_LOOK_DIST {
            (state.yaw, state.pitch)
        } else {
            (yaw_of(look), pitch_of(look))
        };
        if cut {
            self.yaw.snap(crate::track::wrap_deg(yaw));
            self.pitch.snap(pitch);
        } else {
            self.yaw.update(yaw, c.look_spring_s, dt);
            self.pitch.update(pitch, c.look_spring_s, dt);
        }
        self.last_cam = pos;
        self.last_t = Some(t_ms);
        CameraPose {
            pos,
            yaw: self.yaw.angle as f32,
            pitch: self.pitch.angle.clamp(-90.0, 90.0) as f32,
            fov: c.fov,
        }
    }
}
