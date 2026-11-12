//! Preset cameras: first person, shoulder, orbit, side, front, top, fixed.

use super::math::{V3, dist, pitch_of, sub, yaw_dir, yaw_of};
use super::tests_world::{Boxes, EYE_Y, eye_at, horiz, stand, walk};
use super::{CameraPose, Director, DirectorConfig, Focus, OpenWorld, ShotPreset, WorldProbe};
use crate::track::{TrackSample, delta_deg};

const STEP: f64 = 1000.0 / 60.0;

fn director(track: &[TrackSample], preset: ShotPreset) -> Director {
    Director::new(track.to_vec(), Vec::new(), DirectorConfig::default(), 9).with_preset(preset)
}

/// Runs to `until_ms` with a focus function; returns (t, pose) per frame.
pub fn run_focus(
    d: &mut Director,
    world: &dyn WorldProbe,
    until_ms: f64,
    focus: impl Fn(f64) -> Option<Focus>,
) -> Vec<(f64, CameraPose)> {
    let n = (until_ms / STEP).floor() as usize;
    (0..=n)
        .map(|i| {
            let t = i as f64 * STEP;
            (t, d.pose_focus(t, world, focus(t)))
        })
        .collect()
}

/// Angle between where the camera looks and the direction to `p`, degrees.
pub fn aim_error(pose: &CameraPose, p: V3) -> f64 {
    let to = sub(p, pose.pos);
    let dy = delta_deg(f64::from(pose.yaw), yaw_of(to)).abs();
    dy.hypot(f64::from(pose.pitch) - pitch_of(to))
}

fn mob(pos: V3, yaw: Option<f32>) -> Option<Focus> {
    Some(Focus {
        pos,
        height: 1.8,
        yaw,
    })
}

#[test]
fn first_person_is_the_track_view() {
    let mut track = walk([0.0, EYE_Y, 0.0], 30.0, 4.3, 3.0);
    for (i, s) in track.iter_mut().enumerate() {
        s.pitch = i as f32;
    }
    let mut d = director(&track, ShotPreset::FirstPerson);
    for (t, p) in run_focus(&mut d, &OpenWorld, 3000.0, |_| None) {
        assert!(dist(p.pos, eye_at(&track, t)) < 1e-9, "eye at {t}");
        assert!((f64::from(p.yaw) - 30.0).abs() < 1e-4);
        assert!(
            (f64::from(p.pitch) - t / 50.0).abs() < 1e-3,
            "interpolated pitch at {t}"
        );
    }
    assert_eq!(d.shot_name(), "first_person");
}

#[test]
fn side_is_perpendicular_to_subject_facing_at_five() {
    let track = stand([0.0, EYE_Y, 0.0], 0.0, 6.0);
    let center = [10.0, 65.0, 10.0];
    let mut d = director(&track, ShotPreset::Side);
    let frames = run_focus(&mut d, &OpenWorld, 4000.0, |_| mob(center, Some(45.0)));
    let (_, p) = frames.last().copied().expect("frames");
    assert!(
        (horiz(p.pos, center) - 5.0).abs() < 0.1,
        "distance {}",
        horiz(p.pos, center)
    );
    let side = delta_deg(45.0, yaw_of(sub(p.pos, center))).abs();
    assert!((side - 90.0).abs() < 2.0, "angle {side}");
    assert!(aim_error(&p, center) < 3.0);
    assert_eq!(d.shot_name(), "side");
}

#[test]
fn side_without_focus_yaw_uses_player_line() {
    let track = stand([0.0, EYE_Y, 0.0], 0.0, 6.0);
    let center = [8.0, 65.0, 0.0];
    let mut d = director(&track, ShotPreset::Side);
    let frames = run_focus(&mut d, &OpenWorld, 3000.0, |_| mob(center, None));
    let p = frames.last().expect("frames").1;
    // Player→subject line points +X; the camera sits off that line, on the Z axis.
    assert!(
        (p.pos[0] - 8.0).abs() < 0.2 && (p.pos[2].abs() - 5.0).abs() < 0.2,
        "{:?}",
        p.pos
    );
}

#[test]
fn front_faces_the_subject() {
    let track = stand([0.0, EYE_Y, 0.0], 0.0, 6.0);
    let center = [0.0, 65.0, 10.0];
    let mut d = director(&track, ShotPreset::Front);
    let p = run_focus(&mut d, &OpenWorld, 3000.0, |_| mob(center, Some(-90.0)))
        .last()
        .expect("frames")
        .1;
    let expect = [4.0, 65.3, 10.0];
    assert!(dist(p.pos, expect) < 0.15, "{:?}", p.pos);
    assert!(
        delta_deg(f64::from(p.yaw), 90.0).abs() < 3.0,
        "looks back at it: {}",
        p.yaw
    );
}

#[test]
fn top_is_above_looking_down() {
    let track = walk([0.0, EYE_Y, 0.0], 0.0, 2.0, 5.0);
    let mut d = director(&track, ShotPreset::Top);
    let frames = run_focus(&mut d, &OpenWorld, 5000.0, |_| None);
    for (t, p) in frames.iter().skip(1) {
        let center = eye_at(&track, *t);
        assert!(
            (p.pos[1] - (center[1] - 0.6) - 10.0).abs() < 0.2,
            "height at {t}"
        );
        assert!(horiz(p.pos, center) < 0.3);
        assert!((f64::from(p.pitch) - 89.0).abs() < 1.0, "pitch {}", p.pitch);
    }
}

#[test]
fn top_stays_under_a_roof_with_sight() {
    let roof = Boxes(vec![(
        [-50.0, EYE_Y + 3.0, -50.0],
        [50.0, EYE_Y + 4.0, 50.0],
    )]);
    let track = stand([0.0, EYE_Y, 0.0], 0.0, 4.0);
    let mut d = director(&track, ShotPreset::Top);
    let frames = run_focus(&mut d, &roof, 3000.0, |_| None);
    for (_, p) in &frames {
        assert!(p.pos[1] < EYE_Y + 3.0 && !roof.solid_at(p.pos));
        assert!(roof.clear(p.pos, [0.0, EYE_Y - 0.6, 0.0]));
    }
}

#[test]
fn fixed_stays_put_and_watches_the_subject() {
    let track = walk([0.0, EYE_Y, 0.0], -90.0, 4.0, 5.0);
    let spot = [5.0, 70.0, -6.0];
    let mut d = director(&track, ShotPreset::Fixed(spot));
    for (t, p) in run_focus(&mut d, &OpenWorld, 5000.0, |_| None) {
        assert_eq!(p.pos, spot);
        let center = sub(eye_at(&track, t), [0.0, 0.6, 0.0]);
        assert!(aim_error(&p, center) < 0.5, "aim at {t}");
    }
}

#[test]
fn orbit_radius_and_angular_speed() {
    let track = stand([0.0, EYE_Y, 0.0], 0.0, 10.0);
    let mut d = director(&track, ShotPreset::Orbit);
    let frames = run_focus(&mut d, &OpenWorld, 8000.0, |_| None);
    let center = [0.0, EYE_Y - 0.6, 0.0];
    let at = |ms: f64| frames[(ms / STEP).round() as usize].1;
    for (_, p) in frames.iter().skip(60) {
        assert!((horiz(p.pos, center) - 4.0).abs() < 0.15);
        assert!(aim_error(p, center) < 5.0);
    }
    let a = yaw_of(sub(at(4000.0).pos, center));
    let b = yaw_of(sub(at(7000.0).pos, center));
    assert!(
        (delta_deg(a, b).abs() - 90.0).abs() < 4.0,
        "3 s of a 12 s turn: {}",
        delta_deg(a, b)
    );
}

#[test]
fn shoulder_is_behind_right_and_turns_to_focus() {
    let mut track = stand([0.0, EYE_Y, 0.0], 0.0, 4.0);
    for s in &mut track {
        s.pitch = 0.0;
    }
    let mut d = director(&track, ShotPreset::Shoulder);
    let p = run_focus(&mut d, &OpenWorld, 2000.0, |_| None)
        .last()
        .expect("f")
        .1;
    let right = yaw_dir(90.0);
    let expect = [0.5 * right[0], EYE_Y, -1.3];
    assert!(dist(p.pos, expect) < 0.05, "{:?}", p.pos);
    assert!(f64::from(p.yaw).abs() < 1.0 && f64::from(p.pitch).abs() < 1.0);
    let mut d = director(&track, ShotPreset::Shoulder);
    let target = [-20.0, EYE_Y, 0.0];
    let p = run_focus(&mut d, &OpenWorld, 2000.0, |_| mob(target, None))
        .last()
        .expect("f")
        .1;
    let full = yaw_of(sub(target, p.pos));
    assert!(
        (f64::from(p.yaw) - full / 2.0).abs() < 3.0,
        "half turn: {} of {full}",
        p.yaw
    );
}
