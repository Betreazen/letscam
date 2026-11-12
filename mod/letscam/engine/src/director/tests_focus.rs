//! Automatic camera with a focus entity instead of the player.

use super::math::{dist, sub, yaw_of};
use super::tests_presets::{aim_error, run_focus};
use super::tests_world::{EYE_Y, horiz, stand};
use super::{Director, DirectorConfig, Focus, OpenWorld, ShotPreset};
use crate::track::delta_deg;

fn mob_at(t: f64) -> [f64; 3] {
    // Walks +X at 3 blocks/s for 6 s, then stands.
    let s = (t / 1000.0).min(6.0);
    [20.0 + 3.0 * s, 65.0, 20.0]
}

fn director() -> Director {
    let track = stand([0.0, EYE_Y, 0.0], 0.0, 20.0);
    Director::new(track, Vec::new(), DirectorConfig::default(), 4)
}

#[test]
fn follows_moving_focus_from_behind() {
    let mut d = director();
    let frames = run_focus(&mut d, &OpenWorld, 5500.0, |t| {
        Some(Focus {
            pos: mob_at(t),
            height: 1.9,
            yaw: None,
        })
    });
    for (t, p) in frames.iter().filter(|(t, _)| *t > 2000.0) {
        let m = mob_at(*t);
        assert!(
            p.pos[0] < m[0] - 3.0,
            "behind the walking mob at {t}: {:?}",
            p.pos
        );
        assert!((dist(p.pos, m) - 4.5).abs() < 0.5, "distance at {t}");
        assert!(aim_error(p, m) < 20.0, "looks at the mob at {t}");
    }
    assert_eq!(d.shot_name(), "focus_follow");
}

#[test]
fn focus_yaw_wins_over_motion() {
    let mut d = director();
    let frames = run_focus(&mut d, &OpenWorld, 4000.0, |t| {
        Some(Focus {
            pos: mob_at(t),
            height: 1.9,
            yaw: Some(0.0),
        })
    });
    let (t, p) = *frames.last().expect("frames");
    let m = mob_at(t);
    let side = delta_deg(180.0, yaw_of(sub(p.pos, m))).abs();
    assert!(
        side < 5.0,
        "camera behind the mob's facing (+Z), got {side}"
    );
}

#[test]
fn still_focus_gets_an_orbit() {
    let mut d = director();
    let frames = run_focus(&mut d, &OpenWorld, 12_000.0, |t| {
        Some(Focus {
            pos: mob_at(t),
            height: 1.9,
            yaw: None,
        })
    });
    let still = mob_at(10_000.0);
    for (_, p) in frames.iter().filter(|(t, _)| *t > 10_000.0) {
        assert!((horiz(p.pos, still) - 4.0).abs() < 0.4);
    }
    assert_eq!(d.shot_name(), "focus_orbit");
    let max_step = frames
        .windows(2)
        .skip(1)
        .map(|w| dist(w[0].1.pos, w[1].1.pos))
        .fold(0.0, f64::max);
    assert!(max_step < 0.3, "smooth switch, max step {max_step}");
}

#[test]
fn losing_focus_cuts_back_to_player() {
    let mut d = director();
    let frames = run_focus(&mut d, &OpenWorld, 3000.0, |t| {
        (t < 1500.0).then_some(Focus {
            pos: [40.0, 65.0, 40.0],
            height: 1.9,
            yaw: None,
        })
    });
    let after = frames.iter().find(|(t, _)| *t >= 1500.0).expect("frame").1;
    assert!(
        dist(after.pos, [0.0, EYE_Y, 0.0]) < 13.0,
        "back at the player: {:?}",
        after.pos
    );
    assert_eq!(d.preset(), ShotPreset::Auto);
}

#[test]
fn presets_and_focus_are_deterministic() {
    let run = || {
        let mut d = director().with_preset(ShotPreset::Orbit);
        run_focus(&mut d, &OpenWorld, 3000.0, |t| {
            Some(Focus {
                pos: mob_at(t),
                height: 1.9,
                yaw: None,
            })
        })
    };
    assert_eq!(run(), run());
}
