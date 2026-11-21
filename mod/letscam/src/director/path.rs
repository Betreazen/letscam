//! Index over the player track: interpolation, walked-path queries, shot triggers.

use super::math::{V3, dist, lerp};
use crate::track::{TrackSample, lerp_deg};

/// Player state at an arbitrary time.
#[derive(Clone, Debug)]
pub struct PlayerState {
    pub eye: V3,
    pub yaw: f64,
    pub pitch: f64,
    pub fall_flying: bool,
    pub gui_flag: bool,
}

#[derive(Clone, Debug)]
pub struct TrackIndex {
    samples: Vec<TrackSample>,
    /// `seg_start[i]`: first sample of the continuous stretch containing `i`
    /// (stretches are split at teleports and dimension changes).
    seg_start: Vec<usize>,
    /// Establishing triggers: track start, teleports, dimension changes (ms).
    triggers: Vec<f64>,
    /// Times of samples with `hurt_or_attacking` (ms).
    hurt: Vec<f64>,
}

fn t_of(s: &TrackSample) -> f64 {
    f64::from(s.t_ms)
}

impl TrackIndex {
    pub fn new(mut samples: Vec<TrackSample>, teleport_distance: f64) -> TrackIndex {
        samples.retain(|s| s.eye.iter().all(|v| v.is_finite()));
        samples.sort_by_key(|s| s.t_ms);
        let mut seg_start = Vec::with_capacity(samples.len());
        let mut triggers = Vec::new();
        let mut hurt = Vec::new();
        for i in 0..samples.len() {
            let is_break = i == 0 || {
                let (a, b) = (&samples[i - 1], &samples[i]);
                dist(a.eye, b.eye) > teleport_distance || a.dimension != b.dimension
            };
            if is_break {
                triggers.push(t_of(&samples[i]));
                seg_start.push(i);
            } else {
                seg_start.push(seg_start[i - 1]);
            }
            if samples[i].hurt_or_attacking {
                hurt.push(t_of(&samples[i]));
            }
        }
        TrackIndex {
            samples,
            seg_start,
            triggers,
            hurt,
        }
    }

    pub fn first_t(&self) -> f64 {
        self.samples.first().map_or(0.0, t_of)
    }

    pub fn last_t(&self) -> f64 {
        self.samples.last().map_or(0.0, t_of)
    }

    /// Index of the last sample at or before `t` (0 if `t` is before the track).
    fn index_at(&self, t: f64) -> usize {
        self.samples
            .partition_point(|s| t_of(s) <= t)
            .saturating_sub(1)
    }

    pub fn state_at(&self, t: f64) -> Option<PlayerState> {
        let a = self.index_at(t);
        let sa = self.samples.get(a)?;
        let mut st = PlayerState {
            eye: sa.eye,
            yaw: f64::from(sa.yaw),
            pitch: f64::from(sa.pitch),
            fall_flying: sa.fall_flying,
            gui_flag: sa.gui_open,
        };
        let Some(sb) = self.samples.get(a + 1) else {
            return Some(st);
        };
        let span = t_of(sb) - t_of(sa);
        // No interpolation across teleports: the player jumps.
        if t > t_of(sa) && span > 0.0 && self.seg_start[a + 1] == self.seg_start[a] {
            let f = ((t - t_of(sa)) / span).clamp(0.0, 1.0);
            st.eye = lerp(sa.eye, sb.eye, f);
            st.yaw = lerp_deg(st.yaw, f64::from(sb.yaw), f);
            st.pitch = st.pitch + (f64::from(sb.pitch) - st.pitch) * f;
        }
        Some(st)
    }

    fn eye_at(&self, t: f64) -> Option<V3> {
        self.state_at(t).map(|s| s.eye)
    }

    /// Walks back along the path from the position at `t` by `back` blocks, but not further
    /// back in time than `window_ms` and not across a teleport. Returns the point reached and
    /// how much of `back` is left (positive when the path was too short).
    pub fn walk_back(&self, t: f64, back: f64, window_ms: f64) -> Option<(V3, f64)> {
        let now = self.eye_at(t)?;
        let a = self.index_at(t);
        let seg = self.seg_start[a];
        let t_min = (t - window_ms).max(t_of(&self.samples[seg]));
        let mut prev = now;
        let mut acc = 0.0;
        let mut j = a as isize;
        loop {
            let (p, last) = if j < seg as isize || t_of(&self.samples[j as usize]) < t_min {
                (self.eye_at(t_min).unwrap_or(prev), true)
            } else if t_of(&self.samples[j as usize]) > t {
                j -= 1;
                continue;
            } else {
                (self.samples[j as usize].eye, false)
            };
            let d = dist(prev, p);
            if d > 1e-9 && acc + d >= back {
                return Some((lerp(prev, p, (back - acc) / d), 0.0));
            }
            acc += d;
            prev = p;
            if last {
                return Some((prev, (back - acc).max(0.0)));
            }
            j -= 1;
        }
    }

    /// Largest distance of the eye from its position at `t` during `[t - window, t]`,
    /// or `None` if the track (or current stretch) does not cover that window yet.
    pub fn max_move(&self, t: f64, window_ms: f64) -> Option<f64> {
        let now = self.eye_at(t)?;
        let a = self.index_at(t);
        let seg = self.seg_start[a];
        if t - t_of(&self.samples[seg]) < window_ms {
            return None;
        }
        let from = t - window_ms;
        let mut max: f64 = 0.0;
        for s in self.samples[seg..=a].iter().rev() {
            if t_of(s) < from {
                break;
            }
            max = max.max(dist(now, s.eye));
        }
        Some(max)
    }

    /// The latest establishing trigger at or before `t`.
    pub fn last_trigger(&self, t: f64) -> Option<f64> {
        let idx = self.triggers.partition_point(|&x| x <= t);
        // Before the first sample the track start counts as the trigger (the recording begins).
        if idx == 0 {
            return self.triggers.first().copied();
        }
        Some(self.triggers[idx - 1])
    }

    /// The latest hurt/attack time at or before `t`.
    pub fn last_hurt(&self, t: f64) -> Option<f64> {
        let idx = self.hurt.partition_point(|&x| x <= t);
        (idx > 0).then(|| self.hurt[idx - 1])
    }

    /// Direction of movement over the last `window_ms`, if the player moved.
    pub fn velocity_dir(&self, t: f64, window_ms: f64) -> Option<V3> {
        let now = self.eye_at(t)?;
        let before = self.eye_at(t - window_ms)?;
        let d = super::math::sub(now, before);
        (super::math::len(d) > 0.05).then(|| super::math::norm_or(d, [0.0, 0.0, 1.0]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(n: u32, step: f64) -> Vec<TrackSample> {
        (0..n)
            .map(|i| TrackSample::at(i * 50, [0.0, 65.0, f64::from(i) * step], 0.0, 0.0))
            .collect()
    }

    #[test]
    fn interpolates_between_samples() {
        let idx = TrackIndex::new(walk(10, 0.2), 32.0);
        let s = idx.state_at(75.0).expect("state");
        assert!((s.eye[2] - 0.3).abs() < 1e-9);
        assert!(idx.state_at(-10.0).is_some());
        assert!(TrackIndex::new(Vec::new(), 32.0).state_at(0.0).is_none());
    }

    #[test]
    fn walk_back_follows_the_path() {
        let idx = TrackIndex::new(walk(40, 0.2), 32.0);
        // At t = 1950 the player is at z = 7.8; 4 blocks back is z = 3.8.
        let (p, left) = idx.walk_back(1950.0, 4.0, 1200.0).expect("walk");
        assert!((p[2] - 3.8).abs() < 1e-6, "{p:?}");
        assert_eq!(left, 0.0);
        // Only 1 s of history at 4 b/s: 4 blocks; asking for 6 leaves 2.
        let (p, left) = idx.walk_back(1950.0, 6.0, 1000.0).expect("walk");
        assert!((p[2] - 3.8).abs() < 1e-6, "{p:?}");
        assert!((left - 2.0).abs() < 1e-6, "{left}");
    }

    #[test]
    fn teleport_splits_the_path_and_triggers() {
        let mut s = walk(10, 0.2);
        for x in s.iter_mut().skip(5) {
            x.eye[0] += 100.0;
        }
        for x in s.iter_mut().skip(8) {
            x.dimension = "minecraft:the_nether".into();
        }
        let idx = TrackIndex::new(s, 32.0);
        assert_eq!(idx.last_trigger(10.0), Some(0.0));
        assert_eq!(idx.last_trigger(260.0), Some(250.0));
        assert_eq!(idx.last_trigger(450.0), Some(400.0));
        // No interpolation across the jump.
        assert!(idx.state_at(240.0).expect("s").eye[0] < 1.0);
        let (p, left) = idx.walk_back(350.0, 10.0, 5000.0).expect("walk");
        assert!(p[0] > 99.0 && left > 0.0);
    }

    #[test]
    fn idle_and_hurt_queries() {
        let mut s: Vec<TrackSample> = (0..200)
            .map(|i| TrackSample::at(i * 50, [0.0, 65.0, 0.0], 0.0, 0.0))
            .collect();
        s[10].hurt_or_attacking = true;
        let idx = TrackIndex::new(s, 32.0);
        assert_eq!(idx.max_move(3000.0, 6000.0), None);
        assert_eq!(idx.max_move(7000.0, 6000.0), Some(0.0));
        assert_eq!(idx.last_hurt(400.0), None);
        assert_eq!(idx.last_hurt(900.0), Some(500.0));
    }
}
