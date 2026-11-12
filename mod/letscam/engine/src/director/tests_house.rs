//! Regression on a real recording (2026-11-12_18-15-17): the bot walks round a hollow 7×7×4
//! plank house on a superflat world, stands against its west wall with the inventory open,
//! then leaves past the south door. Every frame of every camera must see the player.

use super::math::{V3, dist, sub};
use super::tests_world::Boxes;
use super::{CameraPose, Director, DirectorConfig, ShotPreset, WorldProbe};
use crate::session::Session;

const FPS: f64 = 60.0;
/// The body centre is this far below the eyes (feet + 0.9).
const BODY_BELOW_EYE: f64 = 0.72;
const MIN_DISTANCE: f64 = 1.5;
const MAX_JUMP: f64 = 0.6;
/// Hard cuts allowed in the whole clip (establishing shot plus rescues).
const MAX_CUTS: usize = 6;

fn session() -> Session {
    serde_json::from_str(include_str!(
        "../../testdata/house_2026-11-12_18-15-17.json"
    ))
    .expect("test session")
}

/// Superflat ground (top at y −60) and the hollow house: blocks x −503..−497,
/// y −60..−57, z −503..−497, walls 1 thick, roof at y −57, open door at x −500,
/// y −60..−59 in the south wall (z −497).
pub fn house() -> Boxes {
    Boxes(vec![
        ([-2000.0, -64.0, -2000.0], [2000.0, -60.0, 2000.0]),
        ([-503.0, -57.0, -503.0], [-496.0, -56.0, -496.0]),
        ([-503.0, -60.0, -503.0], [-502.0, -57.0, -496.0]),
        ([-497.0, -60.0, -503.0], [-496.0, -57.0, -496.0]),
        ([-502.0, -60.0, -503.0], [-497.0, -57.0, -502.0]),
        ([-502.0, -60.0, -497.0], [-500.0, -57.0, -496.0]),
        ([-499.0, -60.0, -497.0], [-497.0, -57.0, -496.0]),
        ([-500.0, -58.0, -497.0], [-499.0, -57.0, -496.0]),
    ])
}

struct Failure {
    frame: usize,
    what: String,
}

fn check(preset: ShotPreset) -> Vec<Failure> {
    let s = session();
    let world = house();
    let mut d = Director::new(s.track.clone(), s.gui_events.clone(), DirectorConfig::default(), 7)
        .with_preset(preset);
    let frames = (f64::from(s.duration_ms) * FPS / 1000.0).ceil() as usize;
    let mut out = Vec::new();
    let mut prev: Option<(CameraPose, V3)> = None;
    let mut cuts = 0;
    for i in 0..frames {
        let t = i as f64 * 1000.0 / FPS;
        let p = d.pose(t, &world);
        let Some(eye) = d.index.state_at(t).map(|st| st.eye) else { continue };
        let body: V3 = sub(eye, [0.0, BODY_BELOW_EYE, 0.0]);
        let mut fail = |what: String| out.push(Failure { frame: i, what });
        if world.solid_at(p.pos) {
            fail(format!("inside a block at {:?}", p.pos));
        }
        if !world.clear(p.pos, eye) {
            fail("no sight to the eyes".into());
        }
        if !world.clear(p.pos, body) {
            fail("no sight to the body".into());
        }
        let r = dist(p.pos, eye);
        if r < MIN_DISTANCE {
            fail(format!("too close: {r:.2}"));
        }
        // Jumps are measured relative to the subject: a camera riding along is not a jump.
        let rel_jump = prev.map_or(0.0, |(q, qe)| dist(sub(p.pos, eye), sub(q.pos, qe)));
        if d.last_was_cut() {
            cuts += 1;
        } else if rel_jump > MAX_JUMP {
            fail(format!("jump {rel_jump:.2}"));
        }
        prev = Some((p, eye));
    }
    if cuts > MAX_CUTS {
        out.push(Failure { frame: frames, what: format!("cuts: {cuts}") });
    }
    out
}

/// Groups consecutive failing frames with the same kind of failure for a readable report.
fn summary(name: &str, fails: &[Failure]) -> String {
    let mut by_kind: std::collections::BTreeMap<String, Vec<&Failure>> = Default::default();
    for f in fails {
        let kind = f.what.split(':').next().unwrap_or_default().to_string();
        by_kind.entry(kind).or_default().push(f);
    }
    let mut lines = Vec::new();
    for (kind, list) in by_kind {
        let mut i = 0;
        while i < list.len() {
            let (start, mut end, mut j) = (list[i].frame, list[i].frame, i + 1);
            while j < list.len() && list[j].frame <= end + 1 {
                end = list[j].frame;
                j += 1;
            }
            lines.push(format!("{name}: frames {start}..={end}: {kind} (e.g. {})", list[i].what));
            i = j;
        }
    }
    lines.join("\n")
}

fn assert_clean(preset: ShotPreset, name: &str) {
    let fails = check(preset);
    assert!(fails.is_empty(), "{} failures\n{}", fails.len(), summary(name, &fails));
}

#[test]
fn house_auto_always_sees_the_player() {
    assert_clean(ShotPreset::Auto, "auto");
}

#[test]
fn house_orbit_always_sees_the_player() {
    assert_clean(ShotPreset::Orbit, "orbit");
}

#[test]
fn house_side_always_sees_the_player() {
    assert_clean(ShotPreset::Side, "side");
}

#[test]
fn house_top_always_sees_the_player() {
    assert_clean(ShotPreset::Top, "top");
}

#[test]
#[ignore]
fn house_dump() {
    let s = session();
    let world = house();
    let mut d = Director::new(s.track.clone(), s.gui_events.clone(), DirectorConfig::default(), 7);
    for i in 0..1525usize {
        let t = i as f64 * 1000.0 / FPS;
        let p = d.pose(t, &world);
        let eye = d.index.state_at(t).map(|st| st.eye).unwrap_or_default();
        if (i % 10 == 0 && (300..1200).contains(&i)) || (i % 60 == 0) {
            let to = sub(eye, p.pos);
            let aim = crate::track::delta_deg(f64::from(p.yaw), super::math::yaw_of(to));
            println!(
                "{i:5} {:13} cam [{:8.2} {:6.2} {:8.2}] eye [{:8.2} {:6.2} {:8.2}] r {:4.2} yaw {:7.1} pitch {:5.1} aim {:6.1}",
                d.shot_name(), p.pos[0], p.pos[1], p.pos[2], eye[0], eye[1], eye[2], dist(p.pos, eye), p.yaw, p.pitch, aim
            );
        }
    }
}

#[test]
#[ignore]
fn house_cuts_dump() {
    let s = session();
    let world = house();
    for preset in [ShotPreset::Auto, ShotPreset::Orbit, ShotPreset::Side] {
        let mut d = Director::new(s.track.clone(), s.gui_events.clone(), DirectorConfig::default(), 7)
            .with_preset(preset);
        let mut prev: Option<V3> = None;
        for i in 0..1525usize {
            let t = i as f64 * 1000.0 / FPS;
            let p = d.pose(t, &world);
            if d.last_was_cut() {
                let eye = d.index.state_at(t).map(|st| st.eye).unwrap_or_default();
                println!(
                    "{:?} cut at {i} ({}) cam {:?} prev {:?} eye {:?} detour {:?}",
                    preset, d.shot_name(), p.pos, prev, eye, d.detour
                );
            }
            prev = Some(p.pos);
        }
    }
}
