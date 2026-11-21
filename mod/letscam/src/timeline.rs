//! A WAV file that stays on the wall-clock timeline: sample 0 is `origin`, gaps (no packets
//! from the device, e.g. while nothing plays) are filled with silence, so the n-th sample
//! always plays at `origin + n / rate`.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::wav::WavWriter;

/// Gaps shorter than this between packet time and written length are timing jitter, not gaps.
const JITTER: Duration = Duration::from_millis(40);

pub(crate) struct TimelineWav {
    wav: WavWriter,
    origin: Instant,
    rate: u32,
    channels: u16,
    frames: u64,
    zeros: Vec<i16>,
}

impl TimelineWav {
    pub fn create(
        path: &Path,
        channels: u16,
        rate: u32,
        origin: Instant,
    ) -> Result<TimelineWav, String> {
        Ok(TimelineWav {
            wav: WavWriter::create(path, channels, rate)?,
            origin,
            rate,
            channels,
            frames: 0,
            zeros: Vec::new(),
        })
    }

    pub fn origin(&self) -> Instant {
        self.origin
    }

    #[cfg(test)]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    fn frame_at(&self, at: Instant) -> u64 {
        let d = at.saturating_duration_since(self.origin);
        (d.as_secs_f64() * f64::from(self.rate)) as u64
    }

    fn pad(&mut self, frames: u64) -> Result<(), String> {
        const CHUNK: u64 = 4096;
        let mut left = frames;
        while left > 0 {
            let n = left.min(CHUNK);
            let len = n as usize * usize::from(self.channels);
            self.zeros.resize(len, 0);
            self.wav.write_samples(&self.zeros[..len])?;
            self.frames += n;
            left -= n;
        }
        Ok(())
    }

    /// Writes interleaved samples whose first frame was captured at `at`. A gap before it
    /// (beyond jitter) is filled with silence. Partial trailing frames are ignored.
    pub fn write_at(&mut self, at: Instant, samples: &[i16]) -> Result<(), String> {
        let ch = usize::from(self.channels);
        let whole = samples.len() - samples.len() % ch;
        let target = self.frame_at(at);
        let jitter = (JITTER.as_secs_f64() * f64::from(self.rate)) as u64;
        if target > self.frames + jitter {
            self.pad(target - self.frames)?;
        }
        self.wav.write_samples(&samples[..whole])?;
        self.frames += (whole / ch) as u64;
        Ok(())
    }

    /// Fills silence up to the moment `until` (used while the device delivers nothing).
    pub fn pad_until(&mut self, until: Instant) -> Result<(), String> {
        let target = self.frame_at(until);
        if target > self.frames {
            self.pad(target - self.frames)?;
        }
        Ok(())
    }

    pub fn finalize(self) -> Result<u64, String> {
        self.wav.finalize()?;
        Ok(self.frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;
    use crate::wav::data_len_on_disk;

    #[test]
    fn packets_in_time_are_appended_without_padding() {
        let tmp = TempDir::new("tl");
        let path = tmp.path().join("t.wav");
        let origin = Instant::now();
        let mut t = TimelineWav::create(&path, 2, 1000, origin).expect("create");
        t.write_at(origin, &[1; 20]).expect("w");
        t.write_at(origin + Duration::from_millis(15), &[1; 20])
            .expect("w");
        assert_eq!(t.frames(), 20);
        assert_eq!(t.finalize().expect("fin"), 20);
        assert_eq!(data_len_on_disk(&path).expect("len"), 80);
    }

    #[test]
    fn gap_is_filled_with_silence() {
        let tmp = TempDir::new("tl-gap");
        let path = tmp.path().join("t.wav");
        let origin = Instant::now();
        let mut t = TimelineWav::create(&path, 1, 1000, origin).expect("create");
        t.write_at(origin + Duration::from_millis(500), &[7; 10])
            .expect("w");
        assert_eq!(t.frames(), 510);
        t.pad_until(origin + Duration::from_millis(1000))
            .expect("pad");
        assert_eq!(t.frames(), 1000);
        t.finalize().expect("fin");
        let bytes = std::fs::read(&path).expect("read");
        let sample = |i: usize| i16::from_le_bytes([bytes[44 + 2 * i], bytes[45 + 2 * i]]);
        assert_eq!(sample(0), 0);
        assert_eq!(sample(499), 0);
        assert_eq!(sample(500), 7);
        assert_eq!(sample(509), 7);
        assert_eq!(sample(510), 0);
    }

    #[test]
    fn partial_frames_are_dropped() {
        let tmp = TempDir::new("tl-part");
        let origin = Instant::now();
        let mut t = TimelineWav::create(&tmp.path().join("t.wav"), 2, 1000, origin).expect("c");
        t.write_at(origin, &[1, 2, 3]).expect("w");
        assert_eq!(t.frames(), 1);
    }
}
