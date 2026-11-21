//! Encode throughput: 1080p BGRA frames through [`Encoder`] with hevc_nvenc, CPU colour
//! conversion (`-pix_fmt yuv420p`, the old args) against packed RGB to the GPU (the new).
//! Run on demand (release is closer to the game):
//! `cargo test --release -p letscam-engine encode_bench -- --ignored --nocapture`

use std::time::{Duration, Instant};

use crate::config::{Codec, Config};
use crate::encode::RawInput;
use crate::encoder::{Encoder, probe_hardware};
use crate::encoder_tests::frame;
use crate::test_util::{TempDir, tool};

const W: u32 = 1920;
const H: u32 = 1080;
const FRAMES: u32 = 600;
/// Distinct frames cycled through (a moving bar), so the encoder sees motion.
const DISTINCT: u32 = 8;

/// CPU time (user + kernel) of a process, kept open so it can be read after it exits.
#[cfg(windows)]
mod cpu {
    use std::time::Duration;

    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub struct Process(HANDLE);

    fn ticks(t: FILETIME) -> u64 {
        (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
    }

    impl Process {
        pub fn open(pid: u32) -> Option<Process> {
            // SAFETY: plain Win32 call; the handle is closed in Drop.
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
                .ok()
                .map(Process)
        }

        pub fn cpu_time(&self) -> Option<Duration> {
            let mut t = [FILETIME::default(); 4];
            let [c, e, k, u] = &mut t;
            // SAFETY: valid handle and four valid out-pointers.
            unsafe { GetProcessTimes(self.0, c, e, k, u) }.ok()?;
            Some(Duration::from_nanos((ticks(t[2]) + ticks(t[3])) * 100))
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: the handle came from OpenProcess and is closed once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

struct Run {
    wall: Duration,
    queue_wait: Duration,
    pipe_write: Duration,
    ffmpeg_cpu: Option<Duration>,
}

fn run(cfg: &Config, gpu_rgb: bool, frames: &[Vec<u8>], dir: &std::path::Path) -> Run {
    let out = dir.join(format!("bench_{gpu_rgb}.mp4"));
    let src = RawInput {
        w: W,
        h: H,
        bgra: true,
        gpu_rgb,
    };
    let mut enc = Encoder::start_raw(cfg, &out, src, &[], None).expect("start");
    #[cfg(windows)]
    let process = enc.ffmpeg_pid().and_then(cpu::Process::open);
    let start = Instant::now();
    for i in 0..FRAMES {
        enc.push_frame(frames[(i % DISTINCT) as usize].clone())
            .expect("push");
    }
    let (queue_wait, pipe_write) = (enc.queue_wait(), enc.pipe_write());
    enc.finish().expect("finish");
    let wall = start.elapsed();
    #[cfg(windows)]
    let ffmpeg_cpu = process.and_then(|p| p.cpu_time());
    #[cfg(not(windows))]
    let ffmpeg_cpu = None;
    Run {
        wall,
        queue_wait,
        pipe_write,
        ffmpeg_cpu,
    }
}

fn report(label: &str, r: &Run) {
    let per = |d: Duration| d.as_secs_f64() * 1000.0 / f64::from(FRAMES);
    let cpu = r
        .ffmpeg_cpu
        .map_or("n/a".into(), |c| format!("{:.2} ms/frame", per(c)));
    println!(
        "{label}: {FRAMES} frames in {:.2} s = {:.0} fps; queue wait {:.2} ms/frame; \
         pipe write {:.2} ms/frame; ffmpeg CPU {cpu}",
        r.wall.as_secs_f64(),
        f64::from(FRAMES) / r.wall.as_secs_f64(),
        per(r.queue_wait),
        per(r.pipe_write),
    );
}

#[test]
#[ignore = "benchmark: needs ffmpeg with hevc_nvenc, takes ~20 s"]
fn encode_bench_hevc_nvenc_cpu_vs_gpu_colour() {
    if tool("ffmpeg").is_none() || !probe_hardware("ffmpeg", Codec::Hevc) {
        println!("NOTE: no ffmpeg with hevc_nvenc, benchmark skipped");
        return;
    }
    let cfg = Config::default();
    let frames: Vec<Vec<u8>> = (0..DISTINCT).map(|i| frame(W, H, i)).collect();
    let tmp = TempDir::new("encode-bench");
    // Warm-up (driver and file cache), then alternate so drift hits both sides.
    run(&cfg, true, &frames[..], tmp.path());
    for round in 1..=2 {
        report(
            &format!("round {round} old (yuv420p on CPU)"),
            &run(&cfg, false, &frames, tmp.path()),
        );
        report(
            &format!("round {round} new (RGB to NVENC)  "),
            &run(&cfg, true, &frames, tmp.path()),
        );
    }
}
