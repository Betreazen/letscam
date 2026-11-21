//! Regressions on real recordings, every frame at 60 fps, for several cameras:
//! - house (2026-11-12_18-15-17): the bot walks round a hollow 7×7×4 plank house on a
//!   superflat world, stands against its west wall with the inventory open, leaves past
//!   the south door;
//! - capsule (2026-11-12_18-24-53): RustCraft monorail ride, 77 blocks at 32 blocks/s over
//!   a stone deck with a raised stone run; the seat (focus) is stood in for by the eye track.
//!
//! Each frame: sight to the subject, camera not in a block, at least 1.5 blocks away, the
//! subject on screen, no jump (relative to the subject) over 0.6 blocks except declared cuts.

use super::math::{V3, add, dist, len, scale, sub, view_dir, yaw_dir};
use super::tests_world::Boxes;
use super::{CameraPose, Director, DirectorConfig, Focus, ShotPreset, WorldProbe};
use crate::session::Session;

const FPS: f64 = 60.0;
const BODY_BELOW_EYE: f64 = 0.72;
const MIN_DISTANCE: f64 = 1.5;
const MAX_JUMP: f64 = 0.6;
/// Hard cuts allowed in a whole clip (opening shot plus rescues).
const MAX_CUTS: usize = 3;
/// Screen: vertical field of view 70°, 16:9, the subject kept 10 % inside the edges.
const ASPECT: f64 = 16.0 / 9.0;
const SCREEN_MARGIN: f64 = 0.9;

fn load(json: &str) -> Session {
    serde_json::from_str(json).expect("test session")
}

/// Superflat ground (top y −60) and the hollow house: blocks x −503..−497, y −60..−57,
/// z −503..−497, walls 1 thick, roof y −57, open door x −500, y −60..−59 in wall z −497.
fn house() -> Boxes {
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

/// Stone deck y 149 over x 96..178, z 106..114; raised run x 130..139, y 150, z 110;
/// station blocks at (97, 150, 110) and (173, 150, 110). Rails are not solid.
fn capsule_line() -> Boxes {
    Boxes(vec![
        ([96.0, 149.0, 106.0], [179.0, 150.0, 115.0]),
        ([130.0, 150.0, 110.0], [140.0, 151.0, 111.0]),
        ([97.0, 150.0, 110.0], [98.0, 151.0, 111.0]),
        ([173.0, 150.0, 110.0], [174.0, 151.0, 111.0]),
    ])
}

struct Scene {
    name: &'static str,
    session: Session,
    world: Boxes,
    /// The capsule seat is not in the session: the eye stands in for it as a focus.
    eye_as_focus: bool,
}

fn house_scene() -> Scene {
    Scene {
        name: "house",
        session: load(include_str!(
            "../../testdata/house_2026-11-12_18-15-17.json"
        )),
        world: house(),
        eye_as_focus: false,
    }
}

fn capsule_scene() -> Scene {
    Scene {
        name: "capsule",
        session: load(include_str!(
            "../../testdata/capsule_2026-11-12_18-24-53.json"
        )),
        world: capsule_line(),
        eye_as_focus: true,
    }
}

/// True if `p` is in front of the camera and inside the frame with a margin.
fn on_screen(pose: &CameraPose, p: V3) -> bool {
    let (yaw, pitch) = (f64::from(pose.yaw), f64::from(pose.pitch));
    let fwd = view_dir(yaw, pitch);
    let right = yaw_dir(yaw + 90.0);
    let up = view_dir(yaw, pitch - 90.0);
    let d = sub(p, pose.pos);
    let dot = |a: V3| (0..3).map(|i| d[i] * a[i]).sum::<f64>();
    let z = dot(fwd);
    if z <= 0.05 {
        return false;
    }
    let (x, y) = (dot(right) / z, dot(up) / z);
    let half_v = (f64::from(pose.fov) / 2.0).to_radians().tan();
    y.abs() <= half_v * SCREEN_MARGIN && x.abs() <= half_v * ASPECT * SCREEN_MARGIN
}

fn check(scene: &Scene, preset: ShotPreset) -> Vec<(usize, String)> {
    let s = &scene.session;
    let world = &scene.world;
    let mut d = Director::new(
        s.track.clone(),
        s.gui_events.clone(),
        DirectorConfig::default(),
        7,
    )
    .with_preset(preset);
    let frames = (f64::from(s.duration_ms) * FPS / 1000.0).ceil() as usize;
    let mut out = Vec::new();
    let mut prev: Option<(CameraPose, V3)> = None;
    let mut cuts = 0;
    for i in 0..frames {
        let t = i as f64 * 1000.0 / FPS;
        let Some(eye) = d.index.state_at(t).map(|st| st.eye) else {
            continue;
        };
        let focus = scene.eye_as_focus.then_some(Focus {
            pos: eye,
            height: 1.0,
            yaw: None,
        });
        let p = d.pose_focus(t, world, focus);
        let points: Vec<V3> = if scene.eye_as_focus {
            vec![eye]
        } else {
            vec![eye, sub(eye, [0.0, BODY_BELOW_EYE, 0.0])]
        };
        let mut fail = |what: String| out.push((i, what));
        if world.solid_at(p.pos) {
            fail(format!("inside a block: {:?}", p.pos));
        }
        for q in &points {
            if !world.clear(p.pos, *q) {
                fail(format!("no sight: to {q:?} from {:?}", p.pos));
            }
        }
        if dist(p.pos, eye) < MIN_DISTANCE {
            fail(format!("too close: {:.2}", dist(p.pos, eye)));
        }
        if !on_screen(&p, eye) {
            fail(format!(
                "off screen: cam {:?} yaw {} pitch {} eye {eye:?}",
                p.pos, p.yaw, p.pitch
            ));
        }
        let rel_jump = prev.map_or(0.0, |(q, qe)| dist(sub(p.pos, eye), sub(q.pos, qe)));
        if d.last_was_cut() {
            cuts += 1;
        } else if rel_jump > MAX_JUMP {
            fail(format!("jump: {rel_jump:.2}"));
        }
        prev = Some((p, eye));
    }
    println!("{} {}: {cuts} cuts", scene.name, preset.word());
    if cuts > MAX_CUTS {
        out.push((frames, format!("cuts: {cuts}")));
    }
    out
}

/// Consecutive failing frames of one kind, e.g. `side: frames 376..=1031: too close`.
fn summary(name: &str, fails: &[(usize, String)]) -> String {
    let mut by_kind: std::collections::BTreeMap<String, Vec<&(usize, String)>> = Default::default();
    for f in fails {
        let kind = f.1.split(':').next().unwrap_or_default().to_string();
        by_kind.entry(kind).or_default().push(f);
    }
    let mut lines = Vec::new();
    for (kind, list) in by_kind {
        let mut i = 0;
        while i < list.len() {
            let (start, mut end, mut j) = (list[i].0, list[i].0, i + 1);
            while j < list.len() && list[j].0 <= end + 1 {
                end = list[j].0;
                j += 1;
            }
            lines.push(format!(
                "{name}: frames {start}..={end}: {kind} (e.g. {})",
                list[i].1
            ));
            i = j;
        }
    }
    lines.join("\n")
}

fn assert_clean(scene: &Scene, preset: ShotPreset) {
    let fails = check(scene, preset);
    let name = format!("{} {}", scene.name, preset.word());
    assert!(
        fails.is_empty(),
        "{} failures\n{}",
        fails.len(),
        summary(&name, &fails)
    );
}

#[test]
fn house_auto() {
    assert_clean(&house_scene(), ShotPreset::Auto);
}

#[test]
fn house_orbit() {
    assert_clean(&house_scene(), ShotPreset::Orbit);
}

#[test]
fn house_side() {
    assert_clean(&house_scene(), ShotPreset::Side);
}

#[test]
fn house_top() {
    assert_clean(&house_scene(), ShotPreset::Top);
}

#[test]
fn capsule_auto() {
    assert_clean(&capsule_scene(), ShotPreset::Auto);
}

#[test]
fn capsule_side() {
    assert_clean(&capsule_scene(), ShotPreset::Side);
}

#[test]
fn capsule_orbit() {
    assert_clean(&capsule_scene(), ShotPreset::Orbit);
}

#[test]
fn capsule_top() {
    assert_clean(&capsule_scene(), ShotPreset::Top);
}

#[test]
fn screen_check_works() {
    let pose = CameraPose {
        pos: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        fov: 70.0,
    };
    assert!(on_screen(&pose, [0.0, 0.0, 5.0]));
    assert!(!on_screen(&pose, [0.0, 0.0, -5.0]), "behind");
    assert!(
        !on_screen(&pose, [0.0, 5.0, 5.0]),
        "45° up is outside a 70° frame"
    );
    let wide = add([0.0, 0.0, 5.0], scale(yaw_dir(90.0), 4.0));
    assert!(on_screen(&pose, wide), "wide 16:9");
    assert!(len(view_dir(0.0, -90.0)) > 0.99);
}
