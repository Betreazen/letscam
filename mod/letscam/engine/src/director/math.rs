//! Tiny vector and angle helpers in Minecraft conventions.

pub type V3 = [f64; 3];

pub const UP: V3 = [0.0, 1.0, 0.0];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

pub fn len(a: V3) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

pub fn dist(a: V3, b: V3) -> f64 {
    len(sub(a, b))
}

pub fn lerp(a: V3, b: V3, t: f64) -> V3 {
    add(a, scale(sub(b, a), t))
}

pub fn lerp1(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Unit vector, or `fallback` for (near) zero vectors.
pub fn norm_or(a: V3, fallback: V3) -> V3 {
    let l = len(a);
    if l > 1e-9 {
        scale(a, 1.0 / l)
    } else {
        fallback
    }
}

/// Horizontal unit vector a player with this yaw looks along (yaw 0 = +Z, 90 = -X).
pub fn yaw_dir(yaw_deg: f64) -> V3 {
    let r = yaw_deg.to_radians();
    [-r.sin(), 0.0, r.cos()]
}

/// Yaw of a direction (degrees, Minecraft convention).
pub fn yaw_of(d: V3) -> f64 {
    (-d[0]).atan2(d[2]).to_degrees()
}

/// Pitch of a direction (degrees, positive = down).
pub fn pitch_of(d: V3) -> f64 {
    (-d[1])
        .atan2((d[0] * d[0] + d[2] * d[2]).sqrt())
        .to_degrees()
}

/// Rotates around the vertical axis so that the yaw of the vector grows by `deg`.
pub fn rotate_yaw(v: V3, deg: f64) -> V3 {
    let (s, c) = deg.to_radians().sin_cos();
    [v[0] * c - v[2] * s, v[1], v[0] * s + v[2] * c]
}

/// Horizontal unit vector from `a` towards `b` (zero if they are above each other).
pub fn horiz_dir_to(a: V3, b: V3) -> V3 {
    norm_or([b[0] - a[0], 0.0, b[2] - a[2]], [0.0; 3])
}

pub fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

pub fn smootherstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

/// Fraction of the way an exponential filter moves in `dt` with time constant `tau`.
pub fn exp_alpha(dt: f64, tau: f64) -> f64 {
    if tau <= 1e-6 {
        1.0
    } else {
        1.0 - (-dt / tau).exp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        dist(a, b) < 1e-9
    }

    #[test]
    fn yaw_convention() {
        assert!(close(yaw_dir(0.0), [0.0, 0.0, 1.0]));
        assert!(close(yaw_dir(90.0), [-1.0, 0.0, 0.0]));
        assert!((yaw_of([0.0, 0.0, 1.0])).abs() < 1e-9);
        assert!((yaw_of([-1.0, 0.0, 0.0]) - 90.0).abs() < 1e-9);
        assert!((pitch_of([0.0, -1.0, 1.0]) - 45.0).abs() < 1e-9);
        assert!(close(rotate_yaw(yaw_dir(10.0), 30.0), yaw_dir(40.0)));
    }

    #[test]
    fn easing_bounds() {
        assert_eq!(smoothstep(-1.0), 0.0);
        assert_eq!(smoothstep(2.0), 1.0);
        assert!((smootherstep(0.5) - 0.5).abs() < 1e-12);
        assert_eq!(exp_alpha(0.1, 0.0), 1.0);
    }
}
