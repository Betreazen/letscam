//! Director tests: follow, walls, ceilings, doorway, yaw wrap, determinism.

use super::math::dist;
use super::tests_world::{
    Boxes, EYE_Y, Frame, frames_between, horiz, look_error, run, stand, walk,
};
use super::{Director, DirectorConfig, OpenWorld};
use crate::track::delta_deg;

const FPS: f64 = 60.0;

fn director(track: &[crate::track::TrackSample]) -> Director {
    Director::new(track.to_vec(), Vec::new(), DirectorConfig::default(), 1)
}

fn assert_smooth(frames: &[Frame], max_step: f64, max_turn: f64) {
    for w in frames.windows(2) {
        let step = dist(w[0].pose.pos, w[1].pose.pos);
        assert!(step <= max_step, "jump {step:.3} at t={}", w[1].t);
        let turn = delta_deg(f64::from(w[0].pose.yaw), f64::from(w[1].pose.yaw)).abs();
        assert!(turn <= max_turn, "turn {turn:.2}° at t={}", w[1].t);
    }
}

#[test]
fn straight_walk_camera_behind_and_above() {
    let track = walk([0.0, EYE_Y, 0.0], 0.0, 4.3, 20.0);
    let mut d = director(&track);
    let frames = run(&mut d, &OpenWorld, &track, 20_000.0, FPS);
    assert_eq!(frames[0].shot, "establishing");
    for f in frames_between(&frames, 4500.0, 7500.0) {
        let r = dist(f.pose.pos, f.eye);
        assert!((r - 4.5).abs() < 0.35, "distance {r:.2} at {}", f.t);
        let up = f.pose.pos[1] - f.eye[1];
        assert!((1.0..2.0).contains(&up), "height {up:.2} at {}", f.t);
        assert!(f.pose.pos[2] < f.eye[2] - 3.5, "not behind at {}", f.t);
        assert!((f.pose.pos[0] - f.eye[0]).abs() < 0.3);
        assert!(matches!(f.shot, "follow" | "shoulder"));
    }
    assert_smooth(&frames[1..], 0.2, 3.0);
    for f in frames.iter().skip(12) {
        assert!(
            look_error(f) < 20.0,
            "looks away {:.1}° at {}",
            look_error(f),
            f.t
        );
    }
}

#[test]
fn wall_behind_pulls_camera_in_front_of_it() {
    let wall = Boxes(vec![([-10.0, 60.0, -2.5], [10.0, 80.0, -1.5])]);
    let track = stand([0.0, EYE_Y, 0.0], 0.0, 6.0);
    let mut d = director(&track);
    let frames = run(&mut d, &wall, &track, 5500.0, FPS);
    for f in &frames {
        assert!(!wall.solid_at(f.pose.pos), "inside wall at {}", f.t);
        assert!(
            super::WorldProbe::clear(&wall, f.eye, f.pose.pos),
            "no sight at {}",
            f.t
        );
    }
    for f in frames_between(&frames, 3500.0, 5500.0) {
        assert!(f.pose.pos[2] > -1.5, "camera behind wall at {}", f.t);
        assert!(
            f.pose.pos[2] < -0.8,
            "camera too close at {}: {:?}",
            f.t,
            f.pose.pos
        );
    }
}

#[test]
fn low_ceiling_keeps_camera_below_and_closer() {
    let ceiling_y = EYE_Y + 2.5;
    let world = Boxes(vec![(
        [-100.0, ceiling_y, -100.0],
        [100.0, ceiling_y + 1.0, 100.0],
    )]);
    let track = walk([0.0, EYE_Y, 0.0], 0.0, 4.3, 10.0);
    let mut d = director(&track);
    let frames = run(&mut d, &world, &track, 10_000.0, FPS);
    for f in &frames {
        assert!(
            f.pose.pos[1] < ceiling_y - 0.2,
            "y {:.2} at {}",
            f.pose.pos[1],
            f.t
        );
    }
    for f in frames_between(&frames, 4500.0, 7500.0) {
        let h = horiz(f.pose.pos, f.eye);
        assert!((2.5..4.0).contains(&h), "horizontal {h:.2} at {}", f.t);
    }
    assert_smooth(&frames[1..], 0.2, 3.0);
}

/// A 1-wide, 2-high doorway in a wall at z = 10..11; the player walks straight through.
fn door_world() -> Boxes {
    Boxes(vec![
        ([-20.0, 60.0, 10.0], [0.0, 75.0, 11.0]),
        ([1.0, 60.0, 10.0], [20.0, 75.0, 11.0]),
        ([0.0, 66.0, 10.0], [1.0, 75.0, 11.0]),
        ([0.0, 60.0, 10.0], [1.0, 64.0, 11.0]),
    ])
}

#[test]
fn camera_follows_through_doorway() {
    let world = door_world();
    let track = walk([0.5, EYE_Y, 0.0], 0.0, 4.3, 6.0);
    let mut d = director(&track);
    let frames = run(&mut d, &world, &track, 6000.0, FPS);
    let mut in_door = false;
    for w in frames.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        assert!(
            !world.solid_at(b.pose.pos),
            "inside wall at {}: {:?}",
            b.t,
            b.pose.pos
        );
        assert!(
            super::WorldProbe::clear(&world, b.eye, b.pose.pos),
            "sight blocked at {}",
            b.t
        );
        assert!(
            super::WorldProbe::clear(&world, a.pose.pos, b.pose.pos),
            "clipped through wall at {}: {:?} -> {:?}",
            b.t,
            a.pose.pos,
            b.pose.pos
        );
        let p = b.pose.pos;
        if (10.0..=11.0).contains(&p[2]) {
            in_door = true;
            assert!(
                (0.0..=1.0).contains(&p[0]) && (64.0..=66.0).contains(&p[1]),
                "{p:?}"
            );
        }
    }
    assert!(in_door, "camera never passed the doorway");
    let last = frames.last().expect("frames");
    assert!(
        last.pose.pos[2] > 11.0,
        "camera stayed outside: {:?}",
        last.pose.pos
    );
}

#[test]
fn camera_follows_through_doorway_then_turn() {
    let world = door_world();
    let mut track = walk([0.5, EYE_Y, 0.0], 0.0, 4.3, 3.0);
    let turn_at = track.last().expect("walk").clone();
    let side = walk(turn_at.eye, -90.0, 4.3, 4.0);
    track.extend(side.into_iter().skip(1).map(|mut s| {
        s.t_ms += turn_at.t_ms;
        s
    }));
    let mut d = director(&track);
    let frames = run(&mut d, &world, &track, 7000.0, FPS);
    for w in frames.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        assert!(!world.solid_at(b.pose.pos), "inside wall at {}", b.t);
        assert!(
            super::WorldProbe::clear(&world, b.eye, b.pose.pos),
            "sight blocked at {}",
            b.t
        );
        assert!(
            b.cut || super::WorldProbe::clear(&world, a.pose.pos, b.pose.pos),
            "clipped through wall at {}: {:?} -> {:?}",
            b.t,
            a.pose.pos,
            b.pose.pos
        );
    }
    // When the player steps out of the door line the camera behind the wall may cut once
    // (declared) onto the clear ray; any other move must not pass through the wall.
    let cuts = frames.iter().skip(1).filter(|f| f.cut).count();
    assert!(cuts <= 1, "{cuts} cuts");
    let last = frames.last().expect("frames");
    assert!(
        last.pose.pos[2] > 11.0,
        "camera stayed outside: {:?}",
        last.pose.pos
    );
}

#[test]
fn yaw_wrap_does_not_spin() {
    let mut track = walk([0.0, EYE_Y, 0.0], 180.0, 3.0, 12.0);
    for (i, s) in track.iter_mut().enumerate() {
        s.yaw = if i % 2 == 0 { 179.0 } else { -179.0 };
    }
    let mut d = director(&track);
    let frames = run(&mut d, &OpenWorld, &track, 12_000.0, FPS);
    assert_smooth(&frames[1..], 0.2, 2.0);
    for f in frames_between(&frames, 4000.0, 7000.0) {
        assert!(
            f.pose.pos[2] > f.eye[2] + 3.0,
            "camera not behind at {}",
            f.t
        );
    }
}

#[test]
fn same_seed_same_poses_other_seed_differs() {
    let track = walk([0.0, EYE_Y, 0.0], 30.0, 4.3, 60.0);
    let poses = |seed| {
        let mut d = Director::new(track.clone(), Vec::new(), DirectorConfig::default(), seed);
        run(&mut d, &OpenWorld, &track, 60_000.0, 30.0)
            .into_iter()
            .map(|f| f.pose)
            .collect::<Vec<_>>()
    };
    let a = poses(5);
    assert_eq!(a, poses(5));
    assert_ne!(a, poses(6));
}

#[test]
fn shoulder_swap_happens_and_moves_camera_sideways() {
    let track = walk([0.0, EYE_Y, 0.0], 0.0, 4.3, 40.0);
    let mut d = director(&track);
    let frames = run(&mut d, &OpenWorld, &track, 40_000.0, 30.0);
    let side = frames
        .iter()
        .filter(|f| f.shot == "shoulder")
        .map(|f| (f.pose.pos[0] - f.eye[0]).abs())
        .fold(0.0, f64::max);
    assert!(side > 1.5, "max side offset {side:.2}");
    assert_smooth(&frames[1..], 0.4, 6.0);
}
