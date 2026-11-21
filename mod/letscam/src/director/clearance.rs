//! How high above its base point the camera may go: under ceilings, under door lintels, and
//! low enough to keep the line of sight to the eyes (lower instead of pulling in).

use super::math::{V3, add, horiz_dir_to, lerp, scale};
use super::probe::{WorldProbe, clearance_up};
use super::settings::DirectorConfig;
use super::shots::Desired;

/// How far ahead of the camera (towards the player) the free space above is checked.
const LOOK_AHEAD_BLOCKS: [f64; 2] = [0.5, 1.0];
/// Gap kept between the line of sight and the obstacle that limits the height.
const SIGHT_MARGIN: f64 = 0.1;

/// Highest height (≤ `max`) at which `base + up*h` still sees `eye`; `None` if even the
/// base point itself has no line of sight (walls will pull the camera in instead).
fn sight_height(
    world: &dyn WorldProbe,
    eye: V3,
    base: V3,
    max: f64,
    iterations: u32,
) -> Option<f64> {
    let at = |h: f64| add(base, [0.0, h, 0.0]);
    if world.clear(eye, at(max)) {
        return Some(max);
    }
    if !world.clear(eye, base) {
        return None;
    }
    let (mut lo, mut hi) = (0.0, max);
    for _ in 0..iterations {
        let mid = (lo + hi) * 0.5;
        if world.clear(eye, at(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some((lo - SIGHT_MARGIN).max(0.0))
}

/// Limit for the camera height above its base.
/// - spec ceiling rule: a solid block within `ceiling_probe` over the eyes caps the height;
/// - free space over the camera's path from its current base to the wished one, and a bit
///   ahead of it, so it ducks under a door lintel before reaching it;
/// - line of sight from both bases to the eyes.
pub fn height_limit(
    world: &dyn WorldProbe,
    cfg: &DirectorConfig,
    eye: V3,
    over_eye: f64,
    wish: &Desired,
    current_base: V3,
) -> f64 {
    let it = cfg.wall_iterations;
    let mut limit = wish.height;
    if over_eye < cfg.ceiling_probe {
        limit = limit.min(over_eye - cfg.ceiling_margin);
    }
    if wish.height <= 0.0 {
        return limit;
    }
    let probe_h = wish.height + cfg.ceiling_margin;
    let mut points: Vec<V3> = (0..=3)
        .map(|k| lerp(current_base, wish.base, f64::from(k) / 3.0))
        .collect();
    let towards = horiz_dir_to(current_base, eye);
    points.extend(
        LOOK_AHEAD_BLOCKS
            .iter()
            .map(|&d| add(current_base, scale(towards, d))),
    );
    for p in points {
        limit = limit.min(clearance_up(world, p, probe_h, it) - cfg.ceiling_margin);
    }
    for base in [wish.base, current_base] {
        if let Some(h) = sight_height(world, eye, base, limit.max(0.0), it) {
            limit = limit.min(h);
        }
    }
    limit
}
