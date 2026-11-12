//! Streaming 16-bit PCM WAV writer. Sizes in the header are patched on every
//! [`WavWriter::flush`] (and automatically about once a second while writing), so a file
//! cut short by a crash is still playable up to the last patch.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant};

const HEADER_LEN: u64 = 44;
const PATCH_EVERY: Duration = Duration::from_secs(1);

pub struct WavWriter {
    out: BufWriter<File>,
    channels: u16,
    sample_rate: u32,
    data_bytes: u64,
    last_patch: Instant,
    scratch: Vec<u8>,
}

/// The 44-byte canonical header for 16-bit PCM.
pub fn header(channels: u16, sample_rate: u32, data_bytes: u32) -> [u8; 44] {
    let block_align = channels.saturating_mul(2);
    let byte_rate = sample_rate.saturating_mul(u32::from(block_align));
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&data_bytes.saturating_add(36).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&block_align.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

/// Converts a float sample in [-1, 1] to i16 with clamping.
pub fn f32_to_i16(s: f32) -> i16 {
    if s.is_nan() {
        return 0;
    }
    (s.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

impl WavWriter {
    pub fn create(path: &Path, channels: u16, sample_rate: u32) -> Result<WavWriter, String> {
        if channels == 0 || sample_rate == 0 {
            return Err(format!(
                "WAV {}: неверный формат ({channels} каналов, {sample_rate} Гц)",
                path.display()
            ));
        }
        let file = File::create(path)
            .map_err(|e| format!("не удалось создать {}: {e}", path.display()))?;
        let mut out = BufWriter::with_capacity(256 * 1024, file);
        out.write_all(&header(channels, sample_rate, 0))
            .map_err(|e| format!("запись {}: {e}", path.display()))?;
        Ok(WavWriter {
            out,
            channels,
            sample_rate,
            data_bytes: 0,
            last_patch: Instant::now(),
            scratch: Vec::new(),
        })
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Interleaved samples written so far (all channels).
    pub fn samples_written(&self) -> u64 {
        self.data_bytes / 2
    }

    /// Appends interleaved samples. The caller keeps whole frames together.
    pub fn write_samples(&mut self, samples: &[i16]) -> Result<(), String> {
        self.scratch.clear();
        for s in samples {
            self.scratch.extend_from_slice(&s.to_le_bytes());
        }
        self.out
            .write_all(&self.scratch)
            .map_err(|e| format!("запись WAV: {e}"))?;
        self.data_bytes += self.scratch.len() as u64;
        if self.last_patch.elapsed() >= PATCH_EVERY {
            self.flush()?;
        }
        Ok(())
    }

    /// Writes buffered data and patches the RIFF and data sizes.
    pub fn flush(&mut self) -> Result<(), String> {
        self.last_patch = Instant::now();
        let data = u32::try_from(self.data_bytes).unwrap_or(u32::MAX - 36);
        let h = header(self.channels, self.sample_rate, data);
        let patch = |out: &mut BufWriter<File>| -> std::io::Result<()> {
            out.flush()?;
            let file = out.get_mut();
            file.seek(SeekFrom::Start(4))?;
            file.write_all(&h[4..8])?;
            file.seek(SeekFrom::Start(40))?;
            file.write_all(&h[40..44])?;
            file.seek(SeekFrom::End(0))?;
            Ok(())
        };
        patch(&mut self.out).map_err(|e| format!("обновление заголовка WAV: {e}"))
    }

    /// Flushes, patches sizes and closes the file. Returns the data size in bytes.
    pub fn finalize(mut self) -> Result<u64, String> {
        self.flush()?;
        self.out
            .get_ref()
            .sync_all()
            .map_err(|e| format!("закрытие WAV: {e}"))?;
        Ok(self.data_bytes)
    }
}

/// Length of the data chunk of a canonical WAV written by [`WavWriter`] (for tests and checks).
pub fn data_len_on_disk(path: &Path) -> Result<u64, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    Ok(len.saturating_sub(HEADER_LEN))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{TempDir, ffprobe_json};

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    #[test]
    fn header_bytes_for_known_format() {
        let h = header(2, 48_000, 400);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32_at(&h, 4), 436);
        assert_eq!(&h[8..16], b"WAVEfmt ");
        assert_eq!(u32_at(&h, 16), 16);
        assert_eq!(&h[20..22], &[1, 0]);
        assert_eq!(&h[22..24], &[2, 0]);
        assert_eq!(u32_at(&h, 24), 48_000);
        assert_eq!(u32_at(&h, 28), 192_000);
        assert_eq!(&h[32..34], &[4, 0]);
        assert_eq!(&h[34..36], &[16, 0]);
        assert_eq!(&h[36..40], b"data");
        assert_eq!(u32_at(&h, 40), 400);
    }

    #[test]
    fn sizes_are_patched_after_finalize() {
        let tmp = TempDir::new("wav");
        let path = tmp.path().join("a.wav");
        let mut w = WavWriter::create(&path, 1, 44_100).expect("create");
        w.write_samples(&[1, -1, 32767, -32768]).expect("write");
        w.write_samples(&[5; 6]).expect("write");
        assert_eq!(w.samples_written(), 10);
        assert_eq!(w.finalize().expect("finalize"), 20);
        let b = std::fs::read(&path).expect("read");
        assert_eq!(b.len(), 64);
        assert_eq!(u32_at(&b, 4), 56);
        assert_eq!(u32_at(&b, 40), 20);
        assert_eq!(&b[44..46], &1i16.to_le_bytes());
        assert_eq!(&b[48..50], &32767i16.to_le_bytes());
        assert_eq!(data_len_on_disk(&path).expect("len"), 20);
    }

    #[test]
    fn flush_makes_file_valid_without_finalize() {
        let tmp = TempDir::new("wav-flush");
        let path = tmp.path().join("b.wav");
        let mut w = WavWriter::create(&path, 2, 48_000).expect("create");
        w.write_samples(&[7; 8]).expect("write");
        w.flush().expect("flush");
        let b = std::fs::read(&path).expect("read");
        assert_eq!(u32_at(&b, 40), 16);
        drop(w);
    }

    #[test]
    fn rejects_zero_format() {
        let tmp = TempDir::new("wav-bad");
        assert!(WavWriter::create(&tmp.path().join("c.wav"), 0, 48_000).is_err());
        assert!(WavWriter::create(&tmp.path().join("c.wav"), 2, 0).is_err());
    }

    #[test]
    fn float_conversion_clamps() {
        assert_eq!(f32_to_i16(0.0), 0);
        assert_eq!(f32_to_i16(1.0), 32767);
        assert_eq!(f32_to_i16(-2.0), -32767);
        assert_eq!(f32_to_i16(f32::NAN), 0);
    }

    #[test]
    fn ffprobe_reads_written_wav() {
        let tmp = TempDir::new("wav-probe");
        let path = tmp.path().join("sine.wav");
        let mut w = WavWriter::create(&path, 2, 48_000).expect("create");
        let samples: Vec<i16> = (0..48_000)
            .flat_map(|i| {
                let v =
                    f32_to_i16((i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5);
                [v, v]
            })
            .collect();
        w.write_samples(&samples).expect("write");
        w.finalize().expect("finalize");
        let Some(json) = ffprobe_json(&path) else {
            return;
        };
        let s = &json["streams"][0];
        assert_eq!(s["codec_name"], "pcm_s16le");
        assert_eq!(s["sample_rate"], "48000");
        assert_eq!(s["channels"], 2);
        let dur: f64 = json["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse().ok())
            .expect("duration");
        assert!((dur - 1.0).abs() < 0.01, "duration {dur}");
    }
}
