//! The automatic "director" camera (spec section 6) and the inspection presets (section 10).
//! Pure logic: the track, an optional focus entity and world probes go in, a camera pose per
//! output frame comes out. Deterministic for the same input.

mod clearance;
mod framing;
mod math;
mod path;
mod preset;
mod presets;
mod probe;
mod rig;
mod rng;
mod settings;
mod shots;
mod spring;
mod subject;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_focus;
#[cfg(test)]
mod tests_presets;
#[cfg(test)]
mod tests_shots;
#[cfg(test)]
mod tests_world;

use std::sync::Arc;

pub use preset::{Focus, PRESET_WORDS, ShotPreset};
pub use probe::{OpenWorld, WorldProbe};
pub use settings::DirectorConfig;

use math::{V3, smoothstep, sub, yaw_of};
use path::{PlayerState, TrackIndex};
use rng::ShoulderSchedule;
use shots::{Desired, Shot, ShotCtx, ShotKind};
use spring::{AngleSpring, Spring3};
use subject::{FocusTrack, SubjectKey};

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

#[derive(Clone, Debug)]
struct Blend {
    /// The blended wish at the moment of the switch, relative to the anchor.
    from: Desired,
    start_ms: f64,
}

#[derive(Clone)]
pub struct Director {
    cfg: DirectorConfig,
    preset: ShotPreset,
    index: Arc<TrackIndex>,
    gui_events: Arc<Vec<GuiEvent>>,
    shoulder: Arc<ShoulderSchedule>,
    shot: Shot,
    name: &'static str,
    blend: Option<Blend>,
    last_t: Option<f64>,
    handled_trigger: Option<f64>,
    force_cut: bool,
    last_wish: Desired,
    base: Spring3,
    target: Spring3,
    height: f64,
    pull: f64,
    anchor: (f64, f64),
    yaw: AngleSpring,
    pitch: AngleSpring,
    last_cam: V3,
    subject_key: Option<SubjectKey>,
    last_center: V3,
    focus_track: FocusTrack,
    /// Orbit preset: start angle and start time.
    orbit0: (f64, f64),
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
            preset: ShotPreset::Auto,
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
            force_cut: false,
            last_wish: zero,
            base: Spring3::new([0.0; 3]),
            target: Spring3::new([0.0; 3]),
            height: 0.0,
            pull: 0.0,
            anchor: (0.0, 0.0),
            yaw: AngleSpring::new(0.0),
            pitch: AngleSpring::new(0.0),
            last_cam: [0.0; 3],
            subject_key: None,
            last_center: [0.0; 3],
            focus_track: FocusTrack::default(),
            orbit0: (0.0, 0.0),
        }
    }

    /// Chooses the camera preset (default: the automatic director).
    pub fn with_preset(mut self, preset: ShotPreset) -> Director {
        self.preset = preset;
        self
    }

    pub fn preset(&self) -> ShotPreset {
        self.preset
    }

    /// Name of the current shot (for logs and tests).
    pub fn shot_name(&self) -> &'static str {
        self.name
    }

    /// The pose at `t_ms` computed on a copy in an empty world: no probing, no state change.
    pub fn peek_pose(&self, t_ms: f64) -> CameraPose {
        self.clone().pose(t_ms, &OpenWorld)
    }

    /// Camera pose for the output frame at `t_ms` (increasing replay times) without a focus.
    pub fn pose(&mut self, t_ms: f64, world: &dyn WorldProbe) -> CameraPose {
        self.pose_focus(t_ms, world, None)
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

    /// Switches shots of the automatic director; returns true when the camera must cut.
    fn update_shot(&mut self, t: f64, state: &PlayerState) -> bool {
        let first = self.last_t.is_none();
        let trigger = self.index.last_trigger(t);
        let mut cut = first || std::mem::take(&mut self.force_cut);
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
        self.switch_shot(t, kind, state.eye, state.yaw + 180.0, cut);
        cut
    }

    /// Starts shot `kind` if it differs from the current one: blended, or hard on a cut.
    fn switch_shot(&mut self, t: f64, kind: ShotKind, anchor: V3, cut_orbit_deg: f64, cut: bool) {
        if cut {
            self.blend = None;
        }
        if kind == self.shot.kind {
            return;
        }
        let orbit_deg0 = if cut {
            cut_orbit_deg
        } else {
            self.blend = Some(Blend {
                from: self.last_wish,
                start_ms: t,
            });
            yaw_of(sub(self.last_cam, anchor))
        };
        self.shot = Shot {
            kind,
            start_ms: t,
            orbit_deg0,
        };
    }

    /// Mixes in the previous shot during a blend and remembers the wish (relative to anchor).
    fn blended(&mut self, t: f64, anchor: V3, mut wish: Desired) -> Desired {
        if let Some(b) = &self.blend {
            let blend_ms = self.cfg.blend_s * 1000.0;
            let w = if blend_ms <= 0.0 {
                1.0
            } else {
                smoothstep((t - b.start_ms) / blend_ms)
            };
            wish = b.from.absolute_from(anchor).lerp(wish, w);
            if w >= 1.0 {
                self.blend = None;
            }
        }
        self.last_wish = wish.relative_to(anchor);
        wish
    }

    /// The (blended) wish of the current automatic shot for this frame.
    fn wish(&mut self, t: f64, state: &PlayerState, reach: f64) -> Desired {
        let ctx = ShotCtx {
            index: &self.index,
            cfg: &self.cfg,
            t,
            state,
            reach,
            shoulder_deg: self.shoulder.angle_at(t),
        };
        let wish = self.shot.desired(&ctx);
        self.name = if self.shot.kind == ShotKind::Follow && ctx.shoulder_deg.abs() > 1.0 {
            "shoulder"
        } else {
            self.shot.kind.name()
        };
        self.blended(t, state.eye, wish)
    }
}
