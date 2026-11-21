//! Critically damped springs (the "SmoothDamp" closed form) for positions and angles.

use super::math::V3;
use crate::track::delta_deg;

/// One step of a critically damped spring towards `target` with time scale `smooth` seconds.
pub fn smooth_damp(x: f64, v: f64, target: f64, smooth: f64, dt: f64) -> (f64, f64) {
    if smooth <= 1e-6 {
        return (target, 0.0);
    }
    if dt <= 0.0 {
        return (x, v);
    }
    let omega = 2.0 / smooth;
    let k = omega * dt;
    let decay = 1.0 / (1.0 + k + 0.48 * k * k + 0.235 * k * k * k);
    let change = x - target;
    let temp = (v + omega * change) * dt;
    let v2 = (v - omega * temp) * decay;
    let x2 = target + (change + temp) * decay;
    (x2, v2)
}

#[derive(Clone, Debug)]
pub struct Spring3 {
    pub pos: V3,
    vel: V3,
}

impl Spring3 {
    pub fn new(pos: V3) -> Spring3 {
        Spring3 { pos, vel: [0.0; 3] }
    }

    pub fn snap(&mut self, pos: V3) {
        self.pos = pos;
        self.vel = [0.0; 3];
    }

    pub fn update(&mut self, target: V3, smooth: f64, dt: f64) -> V3 {
        for ((p, v), t) in self.pos.iter_mut().zip(self.vel.iter_mut()).zip(target) {
            (*p, *v) = smooth_damp(*p, *v, t, smooth, dt);
        }
        self.pos
    }
}

/// Spring on an angle in degrees; always moves along the shortest arc (no spin at ±180).
#[derive(Clone, Debug)]
pub struct AngleSpring {
    pub angle: f64,
    vel: f64,
}

impl AngleSpring {
    pub fn new(angle: f64) -> AngleSpring {
        AngleSpring { angle, vel: 0.0 }
    }

    pub fn snap(&mut self, angle: f64) {
        self.angle = angle;
        self.vel = 0.0;
    }

    pub fn update(&mut self, target: f64, smooth: f64, dt: f64) -> f64 {
        let unwrapped = self.angle + delta_deg(self.angle, target);
        let (a, v) = smooth_damp(self.angle, self.vel, unwrapped, smooth, dt);
        // Keep the stored angle bounded without changing the direction it represents.
        let wrapped = crate::track::wrap_deg(a);
        self.angle = wrapped;
        self.vel = v;
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_without_overshoot() {
        let (mut x, mut v) = (0.0, 0.0);
        let mut max: f64 = 0.0;
        for _ in 0..120 {
            (x, v) = smooth_damp(x, v, 1.0, 0.35, 1.0 / 60.0);
            max = max.max(x);
        }
        assert!((x - 1.0).abs() < 0.01, "{x}");
        assert!(max <= 1.0 + 1e-6, "overshoot {max}");
    }

    #[test]
    fn angle_spring_takes_short_way() {
        let mut s = AngleSpring::new(170.0);
        let mut prev = s.angle;
        for _ in 0..60 {
            let a = s.update(-170.0, 0.2, 1.0 / 60.0);
            assert!(delta_deg(prev, a).abs() < 5.0, "{prev} -> {a}");
            prev = a;
        }
        assert!(delta_deg(prev, -170.0).abs() < 1.0);
    }
}
