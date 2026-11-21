//! Seeded xorshift RNG and the shoulder-swap schedule built from it.

use super::math::smoothstep;
use super::settings::DirectorConfig;

#[derive(Clone, Debug)]
pub struct XorShift64(u64);

impl XorShift64 {
    pub fn new(seed: u64) -> XorShift64 {
        // Zero is a fixed point of xorshift; mix the seed (splitmix64 step) to avoid it.
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        XorShift64(if z == 0 { 0x2545_F491_4F6C_DD1D } else { z })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Swap {
    start_ms: f64,
    move_ms: f64,
    hold_ms: f64,
    angle_deg: f64,
}

impl Swap {
    fn end_ms(&self) -> f64 {
        self.start_ms + 2.0 * self.move_ms + self.hold_ms
    }

    fn angle_at(&self, t: f64) -> f64 {
        let x = t - self.start_ms;
        let env = if x < self.move_ms {
            smoothstep(x / self.move_ms)
        } else if x < self.move_ms + self.hold_ms {
            1.0
        } else {
            1.0 - smoothstep((x - self.move_ms - self.hold_ms) / self.move_ms)
        };
        self.angle_deg * env
    }
}

/// When and how far the follow camera moves to a shoulder, precomputed for the whole track.
#[derive(Clone, Debug, PartialEq)]
pub struct ShoulderSchedule {
    swaps: Vec<Swap>,
}

impl ShoulderSchedule {
    pub fn new(cfg: &DirectorConfig, start_ms: f64, end_ms: f64, seed: u64) -> ShoulderSchedule {
        let mut rng = XorShift64::new(seed);
        let mut swaps = Vec::new();
        let mut t = start_ms;
        let move_ms = cfg.shoulder_move_s * 1000.0;
        while t < end_ms {
            let wait = rng.range(cfg.shoulder_every_min_s, cfg.shoulder_every_max_s) * 1000.0;
            let hold = rng.range(cfg.shoulder_hold_min_s, cfg.shoulder_hold_max_s) * 1000.0;
            let angle = rng.range(cfg.shoulder_angle_min_deg, cfg.shoulder_angle_max_deg);
            let side = if rng.next_u64() & 1 == 0 { 1.0 } else { -1.0 };
            let swap = Swap {
                start_ms: t + wait,
                move_ms,
                hold_ms: hold,
                angle_deg: angle * side,
            };
            t = swap.end_ms();
            swaps.push(swap);
        }
        ShoulderSchedule { swaps }
    }

    /// Side angle (degrees, signed) at time `t`; 0 between swaps.
    pub fn angle_at(&self, t: f64) -> f64 {
        let idx = self.swaps.partition_point(|s| s.start_ms <= t);
        if idx == 0 {
            return 0.0;
        }
        let s = &self.swaps[idx - 1];
        if t < s.end_ms() { s.angle_at(t) } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = XorShift64::new(7);
        let mut b = XorShift64::new(7);
        for _ in 0..1000 {
            let x = a.next_f64();
            assert_eq!(x, b.next_f64());
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(XorShift64::new(0).next_u64(), 0);
    }

    #[test]
    fn schedule_respects_config() {
        let cfg = DirectorConfig::default();
        let s = ShoulderSchedule::new(&cfg, 0.0, 120_000.0, 42);
        assert!(!s.swaps.is_empty());
        let mut prev_end = 0.0;
        for w in &s.swaps {
            let gap = (w.start_ms - prev_end) / 1000.0;
            assert!((8.0..=15.0).contains(&gap), "gap {gap}");
            assert!((25.0..=40.0).contains(&w.angle_deg.abs()));
            prev_end = w.end_ms();
        }
        assert_eq!(s.angle_at(0.0), 0.0);
        let first = &s.swaps[0];
        let mid = first.start_ms + first.move_ms + 1.0;
        assert_eq!(s.angle_at(mid), first.angle_deg);
        assert_eq!(s, ShoulderSchedule::new(&cfg, 0.0, 120_000.0, 42));
    }
}
