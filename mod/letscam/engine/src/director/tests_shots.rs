//! Director tests for the event-driven shots: GUI orbit, establishing, combat, elytra, idle.

use super::math::{dist, sub, yaw_of};
use super::tests_world::{EYE_Y, Frame, frames_between, horiz, look_error, run, stand, walk};
use super::{Director, DirectorConfig, OpenWorld};
use crate::session::GuiEvent;
use crate::track::{TrackSample, delta_deg};

const FPS: f64 = 60.0;

fn director(track: &[TrackSample], gui: Vec<GuiEvent>) -> Director {
    Director::new(track.to_vec(), gui, DirectorConfig::default(), 3)
}

fn at(frames: &[Frame], t: f64) -> &Frame {
    frames
        .iter()
        .min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs()))
        .expect("frame")
}

fn max_step(frames: &[Frame]) -> f64 {
    frames
        .windows(2)
        .map(|w| dist(w[0].pose.pos, w[1].pose.pos))
        .fold(0.0, f64::max)
}

#[test]
fn gui_open_orbits_at_radius_three() {
    let track = stand([0.0, EYE_Y, 0.0], 0.0, 30.0);
    let gui = vec![
        GuiEvent {
            t_ms: 5000,
            open: true,
        },
        GuiEvent {
            t_ms: 25_000,
            open: false,
        },
    ];
    let mut d = director(&track, gui);
    let frames = run(&mut d, &OpenWorld, &track, 30_000.0, FPS);
    for f in frames_between(&frames, 7000.0, 24_000.0) {
        assert_eq!(f.shot, "gui_orbit");
        let r = horiz(f.pose.pos, f.eye);
        assert!((r - 3.0).abs() < 0.3, "radius {r:.2} at {}", f.t);
        assert!(look_error(f) < 15.0);
    }
    let angle = |t| yaw_of(sub(at(&frames, t).pose.pos, at(&frames, t).eye));
    let advanced = delta_deg(angle(10_000.0), angle(15_000.0));
    assert!(
        (advanced.abs() - 90.0).abs() < 15.0,
        "advanced {advanced:.1}°"
    );
    assert!(max_step(&frames[1..]) < 0.25);
    assert_ne!(at(&frames, 27_000.0).shot, "gui_orbit");
}

#[test]
fn teleport_starts_establishing_shot() {
    let mut track = walk([0.0, EYE_Y, 0.0], 0.0, 4.3, 16.0);
    for s in track.iter_mut().filter(|s| s.t_ms >= 10_000) {
        s.eye[0] += 200.0;
    }
    let mut d = director(&track, Vec::new());
    let frames = run(&mut d, &OpenWorld, &track, 16_000.0, FPS);
    let just_after = at(&frames, 10_050.0);
    assert_eq!(just_after.shot, "establishing");
    assert!(just_after.pose.pos[1] - just_after.eye[1] > 8.0);
    assert!(
        horiz(just_after.pose.pos, just_after.eye) < 4.0,
        "cut to the new place"
    );
    let settled = at(&frames, 13_600.0);
    assert!(
        matches!(settled.shot, "follow" | "shoulder"),
        "{}",
        settled.shot
    );
    assert!((dist(settled.pose.pos, settled.eye) - 4.5).abs() < 0.4);
}

#[test]
fn dimension_change_starts_establishing_shot() {
    let mut track = walk([0.0, EYE_Y, 0.0], 0.0, 4.3, 12.0);
    for s in track.iter_mut().filter(|s| s.t_ms >= 8000) {
        s.dimension = "minecraft:the_nether".into();
    }
    let mut d = director(&track, Vec::new());
    let frames = run(&mut d, &OpenWorld, &track, 12_000.0, FPS);
    assert_eq!(at(&frames, 8100.0).shot, "establishing");
    assert_ne!(at(&frames, 7900.0).shot, "establishing");
}

#[test]
fn combat_moves_camera_closer_and_lower() {
    let mut track = walk([0.0, EYE_Y, 0.0], 0.0, 2.0, 14.0);
    for s in track.iter_mut().filter(|s| (8000..=8200).contains(&s.t_ms)) {
        s.hurt_or_attacking = true;
    }
    let mut d = director(&track, Vec::new());
    let frames = run(&mut d, &OpenWorld, &track, 14_000.0, FPS);
    let f = at(&frames, 9800.0);
    assert_eq!(f.shot, "combat");
    let r = dist(f.pose.pos, f.eye);
    assert!((r - 3.0).abs() < 0.5, "combat distance {r:.2}");
    assert!((f.pose.pos[1] - f.eye[1]).abs() < 0.6, "shoulder height");
    assert!(matches!(at(&frames, 13_000.0).shot, "follow" | "shoulder"));
    assert!(max_step(&frames[1..]) < 0.25);
}

#[test]
fn elytra_flight_uses_far_side_camera() {
    let mut track = walk([0.0, 100.0, 0.0], 0.0, 25.0, 12.0);
    for s in &mut track {
        s.fall_flying = true;
    }
    let mut d = director(&track, Vec::new());
    let frames = run(&mut d, &OpenWorld, &track, 12_000.0, FPS);
    let f = at(&frames, 8000.0);
    assert_eq!(f.shot, "elytra");
    let r = dist(f.pose.pos, f.eye);
    assert!((r - 8.0).abs() < 1.0, "elytra distance {r:.2}");
    assert!(
        (f.pose.pos[0] - f.eye[0]).abs() > 2.0,
        "camera should be to the side"
    );
    assert!(f.pose.pos[1] > f.eye[1] + 1.0);
    assert!(look_error(f) < 25.0);
}

#[test]
fn idle_player_gets_slow_orbit() {
    let track = stand([0.0, EYE_Y, 0.0], 45.0, 20.0);
    let mut d = director(&track, Vec::new());
    let frames = run(&mut d, &OpenWorld, &track, 20_000.0, FPS);
    assert_ne!(at(&frames, 5000.0).shot, "idle_orbit");
    for f in frames_between(&frames, 9000.0, 20_000.0) {
        assert_eq!(f.shot, "idle_orbit");
        assert!((horiz(f.pose.pos, f.eye) - 4.0).abs() < 0.4);
    }
    assert!(max_step(&frames[1..]) < 0.25);
}

#[test]
fn peek_pose_does_not_change_state() {
    let track = walk([0.0, EYE_Y, 0.0], 0.0, 4.3, 5.0);
    let mut a = director(&track, Vec::new());
    let mut b = director(&track, Vec::new());
    let peek = a.peek_pose(0.0);
    assert_eq!(peek, b.pose(0.0, &OpenWorld));
    assert_eq!(a.pose(0.0, &OpenWorld), peek);
    assert!(
        peek.pos[1] > EYE_Y + 8.0,
        "starts high above (establishing)"
    );
}

#[test]
fn empty_track_is_harmless() {
    let mut d = director(&[], Vec::new());
    let p = d.pose(0.0, &OpenWorld);
    assert!(p.pos.iter().all(|v| v.is_finite()));
    let p = d.pose(16.6, &OpenWorld);
    assert!(p.yaw.is_finite());
}
