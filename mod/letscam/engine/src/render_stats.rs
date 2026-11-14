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
            "{} frames in {:.1} s ({:.1} fps); submit {:.1} ms/frame; queue wait {:.1} ms/frame; pipe write {:.1} ms/frame",
            self.frames,
            self.wall.as_secs_f64(),
            self.fps(),
            self.ms_per_frame(self.submit),
            self.ms_per_frame(self.queue_wait),
            self.ms_per_frame(self.pipe_write),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_summary() {
        let s = RenderStats {
            frames: 1283,
            wall: Duration::from_secs_f64(21.383),
            submit: Duration::from_secs_f64(1283.0 * 0.0031),
            queue_wait: Duration::from_secs_f64(1283.0 * 0.0004),
            pipe_write: Duration::from_secs_f64(1283.0 * 0.0022),
        };
        assert_eq!(
            s.to_string(),
            "1283 frames in 21.4 s (60.0 fps); submit 3.1 ms/frame; queue wait 0.4 ms/frame; pipe write 2.2 ms/frame"
        );
    }

    #[test]
    fn empty_stats_do_not_divide_by_zero() {
        let s = RenderStats::default();
        assert_eq!(s.fps(), 0.0);
        assert_eq!(
            s.to_string(),
            "0 frames in 0.0 s (0.0 fps); submit 0.0 ms/frame; queue wait 0.0 ms/frame; pipe write 0.0 ms/frame"
        );
    }
}
