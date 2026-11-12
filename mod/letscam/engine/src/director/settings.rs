//! Every number of the director (spec section 6). Lives in the `[director]` table of the config.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DirectorConfig {
    /// Critically damped spring time for camera position and look target, seconds.
    pub spring_s: f64,
    /// Spring time for the final yaw/pitch, seconds.
    pub look_spring_s: f64,
    /// Field of view of the render camera, degrees.
    pub fov: f32,
    /// Follow: full distance from the eyes to the camera, blocks.
    pub follow_distance: f64,
    /// Follow: how much of that distance is height above the eyes.
    pub follow_height: f64,
    /// Follow: the camera looks at a point this far ahead of the player.
    pub look_ahead: f64,
    /// Trail: how far back in time (seconds) the walked path is searched for the camera point.
    pub trail_s: f64,
    /// Shoulder swap: pause between swaps, seconds (random in [min, max]).
    pub shoulder_every_min_s: f64,
    pub shoulder_every_max_s: f64,
    /// Shoulder swap: side angle, degrees (random in [min, max]).
    pub shoulder_angle_min_deg: f64,
    pub shoulder_angle_max_deg: f64,
    /// Shoulder swap: how long the camera stays at the shoulder, seconds.
    pub shoulder_hold_min_s: f64,
    pub shoulder_hold_max_s: f64,
    /// Shoulder swap: duration of moving to and from the shoulder, seconds.
    pub shoulder_move_s: f64,
    /// Ceiling: probe height above the eyes, blocks.
    pub ceiling_probe: f64,
    /// Ceiling: gap kept under the ceiling, blocks.
    pub ceiling_margin: f64,
    /// Ceiling: the closest the camera comes under a low ceiling, blocks.
    pub ceiling_min_distance: f64,
    /// Walls: gap kept before the hit block, blocks.
    pub wall_margin: f64,
    /// Walls: binary search iterations on the eye-to-camera ray.
    pub wall_iterations: u32,
    /// Walls: time constant for moving back out after a wall, seconds (moving in is instant).
    pub wall_push_out_s: f64,
    /// GUI orbit: radius, height above the eyes, seconds per full circle.
    pub gui_radius: f64,
    pub gui_height: f64,
    pub gui_period_s: f64,
    /// Establishing shot: duration, start height above the player, teleport distance trigger.
    pub establishing_s: f64,
    pub establishing_height: f64,
    pub teleport_distance: f64,
    /// Combat: seconds after the last hit, distance, height relative to the eyes.
    pub combat_window_s: f64,
    pub combat_distance: f64,
    pub combat_height: f64,
    /// Elytra: distance, height, side angle in degrees.
    pub elytra_distance: f64,
    pub elytra_height: f64,
    pub elytra_side_deg: f64,
    /// Idle: seconds without movement, radius, height, seconds per full circle, movement threshold.
    pub idle_after_s: f64,
    pub idle_radius: f64,
    pub idle_height: f64,
    pub idle_period_s: f64,
    pub idle_move_epsilon: f64,
    /// Blend time between shots, seconds.
    pub blend_s: f64,
    /// Presets: spring time for position (shorter than the director's).
    pub preset_spring_s: f64,
    /// Orbit preset: radius, height above the subject centre, seconds per full turn.
    pub orbit_radius: f64,
    pub orbit_height: f64,
    pub orbit_period_s: f64,
    /// Side preset: distance and height (camera at 90° to the subject's facing).
    pub side_distance: f64,
    pub side_height: f64,
    /// Front preset: distance and height (camera in front, facing the subject).
    pub front_distance: f64,
    pub front_height: f64,
    /// Top preset: height above the subject.
    pub top_height: f64,
    /// Shoulder preset: blocks behind and to the right of the eyes; share of the turn to the focus.
    pub shoulder_back: f64,
    pub shoulder_right: f64,
    pub shoulder_focus_turn: f64,
    /// Auto with a focus: seconds without movement before the camera orbits the subject.
    pub focus_idle_s: f64,
}

impl Default for DirectorConfig {
    fn default() -> Self {
        DirectorConfig {
            spring_s: 0.35,
            look_spring_s: 0.12,
            fov: 70.0,
            follow_distance: 4.5,
            follow_height: 1.5,
            look_ahead: 1.0,
            trail_s: 1.2,
            shoulder_every_min_s: 8.0,
            shoulder_every_max_s: 15.0,
            shoulder_angle_min_deg: 25.0,
            shoulder_angle_max_deg: 40.0,
            shoulder_hold_min_s: 4.0,
            shoulder_hold_max_s: 7.0,
            shoulder_move_s: 1.5,
            ceiling_probe: 4.0,
            ceiling_margin: 0.3,
            ceiling_min_distance: 2.5,
            wall_margin: 0.2,
            wall_iterations: 8,
            wall_push_out_s: 0.6,
            gui_radius: 3.0,
            gui_height: 0.8,
            gui_period_s: 20.0,
            establishing_s: 3.0,
            establishing_height: 12.0,
            teleport_distance: 32.0,
            combat_window_s: 2.0,
            combat_distance: 3.0,
            combat_height: -0.2,
            elytra_distance: 8.0,
            elytra_height: 2.0,
            elytra_side_deg: 35.0,
            idle_after_s: 6.0,
            idle_radius: 4.0,
            idle_height: 1.5,
            idle_period_s: 40.0,
            idle_move_epsilon: 0.3,
            blend_s: 0.8,
            preset_spring_s: 0.2,
            orbit_radius: 4.0,
            orbit_height: 1.0,
            orbit_period_s: 12.0,
            side_distance: 5.0,
            side_height: 0.3,
            front_distance: 4.0,
            front_height: 0.3,
            top_height: 10.0,
            shoulder_back: 1.3,
            shoulder_right: 0.5,
            shoulder_focus_turn: 0.5,
            focus_idle_s: 2.0,
        }
    }
}

fn clamp(v: f64, lo: f64, hi: f64, fallback: f64) -> f64 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        fallback
    }
}

/// Clamps a `[min, max]` pair into `[lo, hi]` and swaps it when reversed.
fn range(min: f64, max: f64, lo: f64, hi: f64, def: (f64, f64)) -> (f64, f64) {
    let a = clamp(min, lo, hi, def.0);
    let b = clamp(max, lo, hi, def.1);
    if a <= b { (a, b) } else { (b, a) }
}

impl DirectorConfig {
    /// Returns a copy with every value inside a safe range.
    pub fn validated(self) -> DirectorConfig {
        let d = DirectorConfig::default();
        let (every_min, every_max) = range(
            self.shoulder_every_min_s,
            self.shoulder_every_max_s,
            1.0,
            600.0,
            (d.shoulder_every_min_s, d.shoulder_every_max_s),
        );
        let (angle_min, angle_max) = range(
            self.shoulder_angle_min_deg,
            self.shoulder_angle_max_deg,
            0.0,
            90.0,
            (d.shoulder_angle_min_deg, d.shoulder_angle_max_deg),
        );
        let (hold_min, hold_max) = range(
            self.shoulder_hold_min_s,
            self.shoulder_hold_max_s,
            0.5,
            600.0,
            (d.shoulder_hold_min_s, d.shoulder_hold_max_s),
        );
        let follow_distance = clamp(self.follow_distance, 1.0, 32.0, d.follow_distance);
        DirectorConfig {
            spring_s: clamp(self.spring_s, 0.02, 5.0, d.spring_s),
            look_spring_s: clamp(self.look_spring_s, 0.0, 5.0, d.look_spring_s),
            fov: if self.fov.is_finite() {
                self.fov.clamp(30.0, 110.0)
            } else {
                d.fov
            },
            follow_distance,
            follow_height: clamp(
                self.follow_height,
                -follow_distance * 0.9,
                follow_distance * 0.9,
                d.follow_height,
            ),
            look_ahead: clamp(self.look_ahead, 0.0, 10.0, d.look_ahead),
            trail_s: clamp(self.trail_s, 0.0, 10.0, d.trail_s),
            shoulder_every_min_s: every_min,
            shoulder_every_max_s: every_max,
            shoulder_angle_min_deg: angle_min,
            shoulder_angle_max_deg: angle_max,
            shoulder_hold_min_s: hold_min,
            shoulder_hold_max_s: hold_max,
            shoulder_move_s: clamp(self.shoulder_move_s, 0.1, 10.0, d.shoulder_move_s),
            ceiling_probe: clamp(self.ceiling_probe, 0.5, 16.0, d.ceiling_probe),
            ceiling_margin: clamp(self.ceiling_margin, 0.0, 1.0, d.ceiling_margin),
            ceiling_min_distance: clamp(
                self.ceiling_min_distance,
                0.5,
                follow_distance,
                d.ceiling_min_distance.min(follow_distance),
            ),
            wall_margin: clamp(self.wall_margin, 0.0, 1.0, d.wall_margin),
            wall_iterations: self.wall_iterations.clamp(1, 24),
            wall_push_out_s: clamp(self.wall_push_out_s, 0.0, 10.0, d.wall_push_out_s),
            gui_radius: clamp(self.gui_radius, 0.5, 32.0, d.gui_radius),
            gui_height: clamp(self.gui_height, -4.0, 16.0, d.gui_height),
            gui_period_s: clamp(self.gui_period_s, 2.0, 600.0, d.gui_period_s),
            establishing_s: clamp(self.establishing_s, 0.0, 30.0, d.establishing_s),
            establishing_height: clamp(self.establishing_height, 0.0, 64.0, d.establishing_height),
            teleport_distance: clamp(self.teleport_distance, 4.0, 1000.0, d.teleport_distance),
            combat_window_s: clamp(self.combat_window_s, 0.0, 30.0, d.combat_window_s),
            combat_distance: clamp(self.combat_distance, 0.5, 32.0, d.combat_distance),
            combat_height: clamp(self.combat_height, -4.0, 16.0, d.combat_height),
            elytra_distance: clamp(self.elytra_distance, 1.0, 64.0, d.elytra_distance),
            elytra_height: clamp(self.elytra_height, -16.0, 16.0, d.elytra_height),
            elytra_side_deg: clamp(self.elytra_side_deg, -90.0, 90.0, d.elytra_side_deg),
            idle_after_s: clamp(self.idle_after_s, 1.0, 600.0, d.idle_after_s),
            idle_radius: clamp(self.idle_radius, 0.5, 32.0, d.idle_radius),
            idle_height: clamp(self.idle_height, -4.0, 16.0, d.idle_height),
            idle_period_s: clamp(self.idle_period_s, 2.0, 600.0, d.idle_period_s),
            idle_move_epsilon: clamp(self.idle_move_epsilon, 0.0, 10.0, d.idle_move_epsilon),
            blend_s: clamp(self.blend_s, 0.0, 10.0, d.blend_s),
            preset_spring_s: clamp(self.preset_spring_s, 0.0, 5.0, d.preset_spring_s),
            orbit_radius: clamp(self.orbit_radius, 0.5, 64.0, d.orbit_radius),
            orbit_height: clamp(self.orbit_height, -16.0, 32.0, d.orbit_height),
            orbit_period_s: clamp(self.orbit_period_s, 1.0, 600.0, d.orbit_period_s),
            side_distance: clamp(self.side_distance, 0.5, 64.0, d.side_distance),
            side_height: clamp(self.side_height, -16.0, 32.0, d.side_height),
            front_distance: clamp(self.front_distance, 0.5, 64.0, d.front_distance),
            front_height: clamp(self.front_height, -16.0, 32.0, d.front_height),
            top_height: clamp(self.top_height, 1.0, 64.0, d.top_height),
            shoulder_back: clamp(self.shoulder_back, 0.0, 8.0, d.shoulder_back),
            shoulder_right: clamp(self.shoulder_right, -4.0, 4.0, d.shoulder_right),
            shoulder_focus_turn: clamp(self.shoulder_focus_turn, 0.0, 1.0, d.shoulder_focus_turn),
            focus_idle_s: clamp(self.focus_idle_s, 0.2, 600.0, d.focus_idle_s),
        }
    }
}
