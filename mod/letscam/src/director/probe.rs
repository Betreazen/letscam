//! World queries for the director: line of sight via the engine's block raycast.

use super::math::{V3, add, dist, lerp};

/// Implemented by the engine glue over the replay world.
pub trait WorldProbe {
    /// True if the segment `from`→`to` is not blocked by solid blocks.
    fn clear(&self, from: V3, to: V3) -> bool;
}

/// A world with nothing in it (used for previews without probing).
pub struct OpenWorld;

impl WorldProbe for OpenWorld {
    fn clear(&self, _from: V3, _to: V3) -> bool {
        true
    }
}

/// Distance along `from`→`to` that is known to be clear: the full length if the segment is
/// clear, otherwise the boundary found by `iterations` steps of binary search.
pub fn clear_distance(world: &dyn WorldProbe, from: V3, to: V3, iterations: u32) -> f64 {
    let full = dist(from, to);
    if full < 1e-9 || world.clear(from, to) {
        return full;
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..iterations {
        let mid = (lo + hi) * 0.5;
        if world.clear(from, lerp(from, to, mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo * full
}

/// Free height above `p`, up to `max` blocks.
pub fn clearance_up(world: &dyn WorldProbe, p: V3, max: f64, iterations: u32) -> f64 {
    if max <= 0.0 {
        return 0.0;
    }
    clear_distance(world, p, add(p, [0.0, max, 0.0]), iterations)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ceiling(f64);
    impl WorldProbe for Ceiling {
        fn clear(&self, from: V3, to: V3) -> bool {
            from[1].max(to[1]) < self.0
        }
    }

    #[test]
    fn binary_search_finds_boundary() {
        let w = Ceiling(67.0);
        let d = clearance_up(&w, [0.0, 65.0, 0.0], 4.0, 12);
        assert!((d - 2.0).abs() < 4.0 / 4096.0 + 1e-9, "{d}");
        assert!(d <= 2.0);
        assert_eq!(clearance_up(&OpenWorld, [0.0; 3], 4.0, 8), 4.0);
    }
}
