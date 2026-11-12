//! When the wished camera spot cannot see the subject, look for the nearest spot that can:
//! other angles around the subject, higher, closer — instead of pulling into the head.

use super::math::{V3, add, dist, scale, sub, yaw_dir, yaw_of};
use super::probe::WorldProbe;

/// Azimuth offsets tried around the wished direction, degrees (nearest first).
const AZIMUTHS: [f64; 14] = [
    0.0, 25.0, -25.0, 50.0, -50.0, 75.0, -75.0, 100.0, -100.0, 130.0, -130.0, 160.0, -160.0, 180.0,
];
/// Extra heights tried over the wished one, blocks.
const RAISES: [f64; 4] = [0.0, 1.0, 2.0, 3.5];
/// Below this horizontal radius the wished direction is undefined (camera straight above).
const MIN_RADIUS_FOR_AZIMUTH: f64 = 0.3;

/// The points of the subject that must be visible from the camera.
pub(super) type Sight = [V3; 2];

/// True if `p` sees every sight point (which also means `p` is not inside a block).
pub(super) fn sees(world: &dyn WorldProbe, p: V3, sight: &Sight) -> bool {
    sight.iter().all(|s| world.clear(p, *s))
}

/// Distance from `p` to the nearest sight point.
pub(super) fn near(p: V3, sight: &Sight) -> f64 {
    sight.iter().map(|s| dist(p, *s)).fold(f64::INFINITY, f64::min)
}

struct Candidate {
    pos: V3,
    cost: f64,
}

/// The cheapest clear spot around `anchor` near `wish` (also near `prev`, the camera's
/// current position, so the camera does not hop around). `None` if nothing is clear.
pub(super) fn search(
    world: &dyn WorldProbe,
    anchor: V3,
    sight: &Sight,
    wish: V3,
    prev: V3,
    min_dist: f64,
) -> Option<V3> {
    let o = sub(wish, anchor);
    let flat = [o[0], 0.0, o[2]];
    let rh = flat[0].hypot(flat[2]);
    let base_az = if rh > MIN_RADIUS_FOR_AZIMUTH {
        yaw_of(flat)
    } else {
        let p = sub(prev, anchor);
        yaw_of([p[0], 0.0, p[2]])
    };
    let wish_r = rh.max(min_dist);
    let radii = [wish_r, (wish_r * 0.6).max(min_dist + 0.2)];
    let mut cands = Vec::with_capacity(AZIMUTHS.len() * RAISES.len() * radii.len());
    for &da in &AZIMUTHS {
        for &dh in &RAISES {
            for &r in &radii {
                let pos = add(
                    add(anchor, scale(yaw_dir(base_az + da), r)),
                    [0.0, o[1] + dh, 0.0],
                );
                if near(pos, sight) < min_dist {
                    continue;
                }
                let cost = da.abs() / 25.0 + dh * 0.6 + (wish_r - r) * 0.4 + dist(pos, prev) * 0.25;
                cands.push(Candidate { pos, cost });
            }
        }
    }
    cands.sort_by(|a, b| a.cost.total_cmp(&b.cost));
    cands
        .into_iter()
        .find(|c| sees(world, c.pos, sight))
        .map(|c| c.pos)
}
