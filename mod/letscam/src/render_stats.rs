//! Where a render's time goes, as seen from the game's render loop.

use std::fmt;
use std::time::Duration;

/// Timings of a [`crate::RenderJob`] so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderStats {
    /// Frames handed to the encoder.
    pub frames: u64,
    /// From entering the first `submit_frame` to leaving the latest one.
    pub wall: Duration,
    /// Total time inside `submit_frame`: PiP compose and hand-off to the writer thread (the
    /// first call also starts ffmpeg).
    pub submit: Duration,
    /// Part of `submit` spent on the GUI picture-in-picture (finding and composing it).
    pub pip: Duration,
    /// Part of `submit` spent starting ffmpeg on the first frame.
    pub start: Duration,
    /// Part of `submit` spent waiting for room in the queue to the writer thread, i.e. for
    /// ffmpeg to catch up.
    pub queue_wait: Duration,
    /// Total time the writer thread was blocked writing frames to ffmpeg's stdin.
    pub pipe_write: Duration,
}

impl RenderStats {
    /// Output frames per second of wall time (0 before the first frame).
    pub fn fps(&self) -> f64 {
        let secs = self.wall.as_secs_f64();
        if secs > 0.0 {
            self.frames as f64 / secs
        } else {
            0.0
        }
    }

    /// `total` spread over the frames, in milliseconds (0 before the first frame).
    pub fn ms_per_frame(&self, total: Duration) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            total.as_secs_f64() * 1000.0 / self.frames as f64
        }
    }
}

impl fmt::Display for RenderStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} frames in {:.1} s ({:.1} fps); submit {:.1} ms/frame (PiP {:.1} ms/frame, ffmpeg start {} ms); queue wait {:.1} ms/frame; pipe write {:.1} ms/frame",
            self.frames,
            self.wall.as_secs_f64(),
            self.fps(),
            self.ms_per_frame(self.submit),
            self.ms_per_frame(self.pip),
            self.start.as_millis(),
            self.ms_per_frame(self.queue_wait),
            self.ms_per_frame(self.pipe_write),
        )
    }
}

/// The game's side of a render, measured by the glue: how many game frames each video frame
/// took and where the glue's own time per frame went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoopStats {
    /// Video frames captured.
    pub frames: u64,
    /// Game frames after the warm-up (each one asks for a pose or waits for a capture).
    pub ticks: u64,
    /// Game frames that found the previous capture not back yet and asked again.
    pub retries: u64,
    /// From the first game frame after the warm-up to the latest one.
    pub wall: Duration,
    /// Finding the `focus=` entity.
    pub focus: Duration,
    /// The camera pose with its obstacle rays, and the sheet label.
    pub pose: Duration,
    /// Replay position minus the time the frame should show, at the latest frame (ms): it stays
    /// near 0 when the replay moves exactly one video frame per captured frame.
    pub drift_ms: i64,
    /// The largest drift seen, with its sign.
    pub max_drift_ms: i64,
}

impl LoopStats {
    /// Takes the drift of the frame just posed: the latest, and the largest so far.
    pub fn note_drift(&mut self, drift_ms: i64) {
        self.drift_ms = drift_ms;
        if drift_ms.abs() > self.max_drift_ms.abs() {
            self.max_drift_ms = drift_ms;
        }
    }

    /// Takes the drift of a frame that should show `expected_ms` while the replay stands at
    /// `replay_ms`, unless that time lies past the replay's last packet (`replay_end_ms`): a
    /// replay can't move past its end, and the session it was recorded in runs a little longer,
    /// so those last frames would read as drift that no timing fault caused.
    pub fn note_frame(&mut self, replay_ms: i64, expected_ms: i64, replay_end_ms: i64) {
        if expected_ms <= replay_end_ms {
            self.note_drift(replay_ms - expected_ms);
        }
    }
}

/// `total` over `n` in milliseconds (0 for none).
fn ms_per(total: Duration, n: u64) -> f64 {
    if n == 0 {
        0.0
    } else {
        total.as_secs_f64() * 1000.0 / n as f64
    }
}

impl fmt::Display for LoopStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let each = if self.frames == 0 {
            0.0
        } else {
            self.ticks as f64 / self.frames as f64
        };
        write!(
            f,
            "{} game frames for {} video frames ({each:.2} each, {} retries); game frame {:.1} ms; focus {:.2} ms/frame; pose {:.2} ms/frame; replay drift {} ms (max {})",
            self.ticks,
            self.frames,
            self.retries,
            ms_per(self.wall, self.ticks),
            ms_per(self.focus, self.frames),
            ms_per(self.pose, self.frames),
            self.drift_ms,
            self.max_drift_ms,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_line_shows_game_frames_per_video_frame() {
        let s = LoopStats {
            frames: 1000,
            ticks: 2000,
            retries: 990,
            wall: Duration::from_secs(40),
            focus: Duration::from_millis(500),
            pose: Duration::from_millis(1500),
            drift_ms: -3,
            max_drift_ms: 40,
        };
        assert_eq!(
            s.to_string(),
            "2000 game frames for 1000 video frames (2.00 each, 990 retries); game frame 20.0 ms; focus 0.50 ms/frame; pose 1.50 ms/frame; replay drift -3 ms (max 40)"
        );
        assert!(
            LoopStats::default()
                .to_string()
                .starts_with("0 game frames for 0 video frames (0.00 each")
        );
    }

    #[test]
    fn drift_keeps_the_latest_and_the_largest() {
        let mut s = LoopStats::default();
        for d in [0, 2, -7, 3, 1] {
            s.note_drift(d);
        }
        assert_eq!((s.drift_ms, s.max_drift_ms), (1, -7));
    }

    #[test]
    fn frames_past_the_replay_end_are_not_drift() {
        let mut s = LoopStats::default();
        // A 27 173 ms replay of a 27 249 ms session: the replay stops at its end while the
        // last frames still ask for later times.
        for (replay, expected) in [
            (27_150, 27_150),
            (27_166, 27_167),
            (27_173, 27_183),
            (27_173, 27_200),
        ] {
            s.note_frame(replay, expected, 27_173);
        }
        assert_eq!((s.drift_ms, s.max_drift_ms), (-1, -1));
    }

    #[test]
    fn one_line_summary() {
        let s = RenderStats {
            frames: 1283,
            wall: Duration::from_secs_f64(21.383),
            submit: Duration::from_secs_f64(1283.0 * 0.0031),
            pip: Duration::from_secs_f64(1283.0 * 0.0012),
            start: Duration::from_millis(85),
            queue_wait: Duration::from_secs_f64(1283.0 * 0.0004),
            pipe_write: Duration::from_secs_f64(1283.0 * 0.0022),
        };
        assert_eq!(
            s.to_string(),
            "1283 frames in 21.4 s (60.0 fps); submit 3.1 ms/frame (PiP 1.2 ms/frame, ffmpeg start 85 ms); queue wait 0.4 ms/frame; pipe write 2.2 ms/frame"
        );
    }

    #[test]
    fn empty_stats_do_not_divide_by_zero() {
        let s = RenderStats::default();
        assert_eq!(s.fps(), 0.0);
        assert_eq!(
            s.to_string(),
            "0 frames in 0.0 s (0.0 fps); submit 0.0 ms/frame (PiP 0.0 ms/frame, ffmpeg start 0 ms); queue wait 0.0 ms/frame; pipe write 0.0 ms/frame"
        );
    }
}
