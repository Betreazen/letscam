//! Per-frame dispatch: automatic director, automatic with a focus entity, or a preset.

use super::math::{dist, pitch_of, sub, yaw_of};
use super::path::PlayerState;
use super::preset::{Focus, ShotPreset};
use super::probe::{WorldProbe, clearance_up};
use super::rig::RigInput;
use super::subject::{Subject, SubjectKey};
use super::{CameraPose, Director};

/// How much focus history is kept, ms (covers idle detection and motion direction).
const FOCUS_KEEP_MS: f64 = 30_000.0;

impl Director {
    /// Camera pose at replay time `t_ms`. With `focus`, the camera watches that entity instead
    /// of the player (how depends on the preset). Call with increasing times.
    pub fn pose_focus(
        &mut self,
        t_ms: f64,
        world: &dyn WorldProbe,
        focus: Option<Focus>,
    ) -> CameraPose {
        let Some(state) = self.index.state_at(t_ms) else {
            return CameraPose {
                pos: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                fov: self.cfg.fov,
            };
        };
        let focus = focus.filter(|f| f.pos.iter().all(|v| v.is_finite()) && f.height.is_finite());
        let dt = self
            .last_t
            .map_or(0.0, |l| ((t_ms - l) / 1000.0).clamp(0.0, 0.5));
        let subject = Subject::of(&state, focus.as_ref());
        let cut = self.track_subject(t_ms, &subject);
        match (self.preset, focus) {
            (ShotPreset::FirstPerson, _) => {
                self.name = "first_person";
                self.set_angles(state.yaw, state.pitch, dt, true);
                self.finish_frame(t_ms, state.eye)
            }
            (ShotPreset::Fixed(p), _) => {
                self.name = "fixed";
                let look = sub(subject.center, p);
                self.set_angles(yaw_of(look), pitch_of(look), dt, true);
                self.finish_frame(t_ms, p)
            }
            (ShotPreset::Auto, None) => self.auto_player(t_ms, world, &state, dt),
            (ShotPreset::Auto, Some(f)) => {
                let wish = self.focus_auto_wish(t_ms, &subject, &f, cut);
                let input = RigInput {
                    t: t_ms,
                    dt,
                    anchor: subject.center,
                    fallback: (subject.facing, 0.0),
                    wish,
                    cut,
                    spring_s: self.cfg.spring_s,
                };
                self.rig(world, input)
            }
            (_, focus) => {
                let (wish, anchor, fallback) =
                    self.preset_wish(t_ms, &state, &subject, focus.as_ref(), cut);
                let input = RigInput {
                    t: t_ms,
                    dt,
                    anchor,
                    fallback,
                    wish,
                    cut,
                    spring_s: self.cfg.preset_spring_s,
                };
                self.rig(world, input)
            }
        }
    }

    /// Notices a change of subject (player ↔ focus) or a jump of it; returns true for a cut.
    fn track_subject(&mut self, t: f64, subject: &Subject) -> bool {
        let changed = self.subject_key != Some(subject.key);
        let jumped = dist(subject.center, self.last_center) > self.cfg.teleport_distance;
        let cut = self.last_t.is_none() || changed || jumped;
        if changed || jumped {
            self.focus_track.clear();
            // The automatic player path decides its own cut; make sure it cuts too.
            self.force_cut = true;
        }
        self.subject_key = Some(subject.key);
        self.last_center = subject.center;
        if subject.key == SubjectKey::Focus {
            self.focus_track.push(t, subject.center, FOCUS_KEEP_MS);
        }
        cut
    }

    /// The automatic director of spec section 6, following the player.
    fn auto_player(
        &mut self,
        t: f64,
        world: &dyn WorldProbe,
        state: &PlayerState,
        dt: f64,
    ) -> CameraPose {
        let cut = self.update_shot(t, state);
        let c = &self.cfg;
        let over_eye = clearance_up(world, state.eye, c.ceiling_probe, c.wall_iterations);
        let reach = (over_eye / c.ceiling_probe).clamp(0.0, 1.0);
        let wish = self.wish(t, state, reach);
        let input = RigInput {
            t,
            dt,
            anchor: state.eye,
            fallback: (state.yaw, state.pitch),
            wish,
            cut,
            spring_s: self.cfg.spring_s,
        };
        self.rig(world, input)
    }
}
