//! Test helpers: a world of solid axis-aligned boxes and track builders.

use super::math::{V3, dist, len, pitch_of, sub, yaw_dir, yaw_of};
use super::{CameraPose, Director, WorldProbe};
use crate::track::{TrackSample, delta_deg};

/// Solid boxes `(min, max)`.
pub struct Boxes(pub Vec<(V3, V3)>);

fn seg_hits(b: &(V3, V3), a: V3, c: V3) -> bool {
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for i in 0..3 {
        let d = c[i] - a[i];
        if d.abs() < 1e-12 {
            if a[i] < b.0[i] || a[i] > b.1[i] {
                return false;
            }
        } else {
            let (mut u, mut v) = ((b.0[i] - a[i]) / d, (b.1[i] - a[i]) / d);
            if u > v {
                std::mem::swap(&mut u, &mut v);
            }
            t0 = t0.max(u);
            t1 = t1.min(v);
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

impl Boxes {
    pub fn solid_at(&self, p: V3) -> bool {
        self.0
            .iter()
            .any(|b| (0..3).all(|i| p[i] > b.0[i] && p[i] < b.1[i]))
    }
}

impl WorldProbe for Boxes {
    fn clear(&self, from: V3, to: V3) -> bool {
        !self.0.iter().any(|b| seg_hits(b, from, to))
    }
}

pub const EYE_Y: f64 = 65.62;

/// Walks from `start` along `yaw` at `speed` blocks/s for `secs`, 20 samples per second.
pub fn walk(start: V3, yaw: f32, speed: f64, secs: f64) -> Vec<TrackSample> {
    let dir = yaw_dir(f64::from(yaw));
    let n = (secs * 20.0).round() as u32;
    (0..=n)
        .map(|i| {
            let d = speed * f64::from(i) / 20.0;
            let eye = [start[0] + dir[0] * d, start[1], start[2] + dir[2] * d];
            TrackSample::at(i * 50, eye, yaw, 10.0)
        })
        .collect()
}

/// Stands still at `eye` for `secs`.
pub fn stand(eye: V3, yaw: f32, secs: f64) -> Vec<TrackSample> {
    walk(eye, yaw, 0.0, secs)
}

pub struct Frame {
    pub t: f64,
    pub pose: CameraPose,
    pub eye: V3,
    pub shot: &'static str,
    /// The director declared this frame a hard cut.
    pub cut: bool,
}

/// Runs the director at `fps` over `[0, until_ms]`; `eye_of` gives the eye for checks.
pub fn run(
    d: &mut Director,
    world: &dyn WorldProbe,
    track: &[TrackSample],
    until_ms: f64,
    fps: f64,
) -> Vec<Frame> {
    let step = 1000.0 / fps;
    let n = (until_ms / step).floor() as usize;
    (0..=n)
        .map(|i| {
            let t = i as f64 * step;
            let pose = d.pose(t, world);
            Frame {
                t,
                pose,
                eye: eye_at(track, t),
                shot: d.shot_name(),
                cut: d.last_was_cut(),
            }
        })
        .collect()
}

/// Linear interpolation of the eye (test-side, independent of the director's index).
pub fn eye_at(track: &[TrackSample], t: f64) -> V3 {
    let i = track.partition_point(|s| f64::from(s.t_ms) <= t).max(1) - 1;
    let a = &track[i];
    let Some(b) = track.get(i + 1) else {
        return a.eye;
    };
    let f = ((t - f64::from(a.t_ms)) / f64::from(b.t_ms - a.t_ms)).clamp(0.0, 1.0);
    super::math::lerp(a.eye, b.eye, f)
}

/// Angle between the camera's view direction and the direction to the eye, degrees.
pub fn look_error(f: &Frame) -> f64 {
    let to_eye = sub(f.eye, f.pose.pos);
    if len(to_eye) < 0.5 {
        return 0.0;
    }
    let dyaw = delta_deg(f64::from(f.pose.yaw), yaw_of(to_eye)).abs();
    let dpitch = (f64::from(f.pose.pitch) - pitch_of(to_eye)).abs();
    dyaw.hypot(dpitch)
}

pub fn horiz(a: V3, b: V3) -> f64 {
    dist([a[0], 0.0, a[2]], [b[0], 0.0, b[2]])
}

pub fn frames_between(frames: &[Frame], from_ms: f64, to_ms: f64) -> impl Iterator<Item = &Frame> {
    frames
        .iter()
        .filter(move |f| f.t >= from_ms && f.t <= to_ms)
}

#[test]
fn boxes_world_works() {
    let w = Boxes(vec![([0.0, 0.0, 0.0], [1.0, 1.0, 1.0])]);
    assert!(!w.clear([-1.0, 0.5, 0.5], [2.0, 0.5, 0.5]));
    assert!(w.clear([-1.0, 1.5, 0.5], [2.0, 1.5, 0.5]));
    assert!(w.solid_at([0.5, 0.5, 0.5]));
    assert!(!w.solid_at([1.5, 0.5, 0.5]));
}
