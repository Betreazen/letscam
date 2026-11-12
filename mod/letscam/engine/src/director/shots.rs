//! The shots of spec section 6: where each one wants the camera and what it looks at.

use super::math::{V3, add, lerp, lerp1, rotate_yaw, scale, smootherstep, sub, yaw_dir};
use super::path::{PlayerState, TrackIndex};
use super::settings::DirectorConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotKind {
    Follow,
    Establishing,
    Gui,
    Combat,
    Elytra,
    Idle,
}

impl ShotKind {
    pub fn name(self) -> &'static str {
        match self {
            ShotKind::Follow => "follow",
            ShotKind::Establishing => "establishing",
            ShotKind::Gui => "gui_orbit",
            ShotKind::Combat => "combat",
            ShotKind::Elytra => "elytra",
            ShotKind::Idle => "idle_orbit",
        }
    }
}

/// A wanted camera: `base` is the camera point at path/eye height, `height` is added on top
/// (and may be lowered under ceilings), `target` is the look-at point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Desired {
    pub base: V3,
    pub height: f64,
    pub target: V3,
}

impl Desired {
    pub fn relative_to(self, eye: V3) -> Desired {
        Desired {
            base: sub(self.base, eye),
            height: self.height,
            target: sub(self.target, eye),
        }
    }

    pub fn absolute_from(self, eye: V3) -> Desired {
        Desired {
            base: add(self.base, eye),
            height: self.height,
            target: add(self.target, eye),
        }
    }

    pub fn lerp(self, other: Desired, w: f64) -> Desired {
        Desired {
            base: lerp(self.base, other.base, w),
            height: lerp1(self.height, other.height, w),
            target: lerp(self.target, other.target, w),
        }
    }
}

/// Everything a shot needs to compute its wish for one frame.
pub struct ShotCtx<'a> {
    pub index: &'a TrackIndex,
    pub cfg: &'a DirectorConfig,
    pub t: f64,
    pub state: &'a PlayerState,
    /// 1 = open sky; less = a ceiling within the probe height (camera comes closer).
    pub reach: f64,
    pub shoulder_deg: f64,
}

impl ShotCtx<'_> {
    /// Horizontal distances shrink towards `ceiling_min_distance` under a low ceiling.
    fn shrink(&self, d: f64) -> f64 {
        let min = self.cfg.ceiling_min_distance;
        if d <= min {
            d
        } else {
            min + (d - min) * self.reach
        }
    }

    fn look_point(&self) -> V3 {
        add(self.state.eye, [0.0, -0.3, 0.0])
    }

    /// Camera behind the player on the walked path (trail), optionally swung to the side.
    fn boom(&self, distance: f64, height: f64, side_deg: f64) -> V3 {
        let eye = self.state.eye;
        let back = self.shrink((distance * distance - height * height).max(0.25).sqrt());
        let window = self.cfg.trail_s * 1000.0;
        let (q, left) = self
            .index
            .walk_back(self.t, back, window)
            .unwrap_or((eye, back));
        let behind = yaw_dir(self.state.yaw + 180.0);
        let base = add(q, scale(behind, left));
        if side_deg.abs() < 1e-9 {
            base
        } else {
            add(eye, rotate_yaw(sub(base, eye), side_deg))
        }
    }

    pub fn follow(&self) -> Desired {
        let c = self.cfg;
        Desired {
            base: self.boom(c.follow_distance, c.follow_height, self.shoulder_deg),
            height: c.follow_height,
            target: add(self.state.eye, scale(yaw_dir(self.state.yaw), c.look_ahead)),
        }
    }

    fn combat(&self) -> Desired {
        let c = self.cfg;
        Desired {
            base: self.boom(c.combat_distance, c.combat_height, 0.0),
            height: c.combat_height,
            target: self.look_point(),
        }
    }

    fn elytra(&self) -> Desired {
        let c = self.cfg;
        let dir = self
            .index
            .velocity_dir(self.t, 250.0)
            .unwrap_or_else(|| yaw_dir(self.state.yaw));
        Desired {
            base: self.boom(c.elytra_distance, c.elytra_height, c.elytra_side_deg),
            height: c.elytra_height,
            target: add(self.state.eye, scale(dir, 2.0)),
        }
    }

    fn orbit(&self, shot: &Shot, radius: f64, height: f64, period_s: f64) -> Desired {
        let angle = shot.orbit_deg0 + 360.0 * (self.t - shot.start_ms) / (period_s * 1000.0);
        Desired {
            base: add(self.state.eye, scale(yaw_dir(angle), self.shrink(radius))),
            height,
            target: self.look_point(),
        }
    }

    fn establishing(&self, shot: &Shot) -> Desired {
        let c = self.cfg;
        let eye = self.state.eye;
        let from = Desired {
            base: add(eye, scale(yaw_dir(self.state.yaw + 180.0), 2.0)),
            height: c.establishing_height,
            target: eye,
        };
        let w = if c.establishing_s <= 0.0 {
            1.0
        } else {
            smootherstep((self.t - shot.start_ms) / (c.establishing_s * 1000.0))
        };
        from.lerp(self.follow(), w)
    }
}

/// The active shot and the parameters fixed when it started.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub kind: ShotKind,
    pub start_ms: f64,
    /// Orbits start from the camera's angle around the player at the switch (no jump).
    pub orbit_deg0: f64,
}

impl Shot {
    pub fn desired(&self, ctx: &ShotCtx) -> Desired {
        let c = ctx.cfg;
        match self.kind {
            ShotKind::Follow => ctx.follow(),
            ShotKind::Establishing => ctx.establishing(self),
            ShotKind::Gui => ctx.orbit(self, c.gui_radius, c.gui_height, c.gui_period_s),
            ShotKind::Combat => ctx.combat(),
            ShotKind::Elytra => ctx.elytra(),
            ShotKind::Idle => ctx.orbit(self, c.idle_radius, c.idle_height, c.idle_period_s),
        }
    }
}
