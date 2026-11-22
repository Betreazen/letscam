//! RenderJob over a synthetic recording dir: frame count, audio tracks, PiP, cleanup.

use std::path::Path;
use std::time::Duration;

use crate::config::{Config, Container};
use crate::director::OpenWorld;
use crate::encoder_tests::{assert_video, sine_wav, test_config};
use crate::pip::{Image, save_png};
use crate::render::RenderJob;
use crate::session::{AudioTrack, GuiEvent, GuiFrame, SESSION_VERSION, Session};
use crate::test_util::{TempDir, tool};
use crate::track::TrackSample;

const W: u32 = 640;
const H: u32 = 360;
const GRAY: [u8; 4] = [90, 90, 90, 255];

fn make_session(dir: &Path) {
    std::fs::create_dir_all(dir.join("gui")).expect("dirs");
    sine_wav(&dir.join("game.wav"), 48_000, 2, 2.2);
    sine_wav(&dir.join("mic.wav"), 44_100, 1, 1.5);
    let red = Image {
        width: 320,
        height: 180,
        rgba: [230, 20, 20, 255].repeat(320 * 180),
    };
    save_png(&dir.join("gui/000001.png"), &red).expect("png");
    let track = (0..=40)
        .map(|i| TrackSample::at(i * 50, [0.0, 65.62, f64::from(i) * 0.2], 0.0, 0.0))
        .collect();
    let audio = |file: &str, offset_ms, sample_rate, channels| AudioTrack {
        file: file.into(),
        offset_ms,
        sample_rate,
        channels,
        dropped_samples: 0,
    };
    Session {
        version: SESSION_VERSION,
        duration_ms: 2000,
        game_audio: Some(audio("game.wav", -50.0, 48_000, 2)),
        mic_audio: Some(audio("mic.wav", 300.0, 44_100, 1)),
        gui_frames: vec![GuiFrame {
            t_ms: 400,
            file: "gui/000001.png".into(),
        }],
        gui_events: vec![
            GuiEvent {
                t_ms: 500,
                open: true,
            },
            GuiEvent {
                t_ms: 1500,
                open: false,
            },
        ],
        track,
        warnings: Vec::new(),
    }
    .save(dir)
    .expect("session");
}

/// Reads one decoded RGB frame at `t` seconds from the video.
fn rgb_at(path: &Path, t: f64) -> Vec<u8> {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &format!("{t}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("ffmpeg");
    out.stdout
}

fn rgb(frame: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * 1920 + x) * 3;
    [frame[i], frame[i + 1], frame[i + 2]]
}

#[test]
fn renders_session_with_pip_and_cleans_up() {
    if tool("ffmpeg").is_none() {
        return;
    }
    let tmp = TempDir::new("render");
    let dir = tmp.path().join("2026-11-12_12-00-00");
    make_session(&dir);
    let cfg = Config {
        keep_sources: false,
        ..test_config(Container::Mp4)
    };
    let mut job = RenderJob::open(&dir, &cfg).expect("open");
    assert_eq!(job.fps(), 60);
    assert_eq!(job.total_frames(), 120);
    let first = job.peek_first_pose().expect("peek");
    let mut frames = 0;
    while let Some(pose) = job.next_pose(&OpenWorld) {
        if frames == 0 {
            assert_eq!(pose, first, "peek must match the real first pose");
        }
        job.submit_frame(W, H, vec![GRAY; (W * H) as usize].into_flattened(), true)
            .expect("submit");
        frames += 1;
    }
    assert_eq!(frames, 120);
    assert!((job.progress() - 1.0).abs() < 1e-6);
    assert!(
        job.submit_frame(W, H, vec![GRAY; (W * H) as usize].into_flattened(), true)
            .is_err()
    );
    let out = job.finish().expect("finish");
    assert_eq!(out, dir.join("letscam_2026-11-12_12-00-00.mp4"));
    assert_video(&out, 120, &["Игра", "Микрофон"]);
    let plain = rgb_at(&out, 0.2);
    let with_gui = rgb_at(&out, 1.0);
    let (x, y) = (1700, 950);
    let p0 = rgb(&plain, x, y);
    let p1 = rgb(&with_gui, x, y);
    assert!(
        p0[0].abs_diff(90) < 12 && p0[1].abs_diff(90) < 12,
        "plain frame {p0:?}"
    );
    assert!(
        p1[0] > 180 && p1[1] < 70,
        "PiP expected in the corner, got {p1:?}"
    );
    assert!(
        !dir.join("game.wav").exists() && !dir.join("gui").exists(),
        "sources removed"
    );
    assert!(dir.join("session.json").exists());
}

#[test]
fn rejects_wrong_sizes_and_empty_sessions() {
    let tmp = TempDir::new("render-bad");
    let dir = tmp.path().join("rec");
    make_session(&dir);
    let cfg = Config {
        hardware: false,
        ..Config::default()
    };
    let mut job = RenderJob::open(&dir, &cfg).expect("open");
    assert!(
        job.submit_frame(4, 4, vec![GRAY; 3].into_flattened(), false)
            .is_err()
    );
    if tool("ffmpeg").is_some() {
        job.submit_frame(64, 64, vec![GRAY; 64 * 64].into_flattened(), false)
            .expect("first");
        assert!(
            job.submit_frame(32, 32, vec![GRAY; 32 * 32].into_flattened(), false)
                .is_err()
        );
    }
    job.cancel();
    assert!(!dir.join("letscam_rec.mp4").exists());

    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(&empty).expect("dir");
    Session {
        version: SESSION_VERSION,
        duration_ms: 0,
        game_audio: None,
        mic_audio: None,
        gui_frames: Vec::new(),
        gui_events: Vec::new(),
        track: Vec::new(),
        warnings: Vec::new(),
    }
    .save(&empty)
    .expect("save");
    assert!(RenderJob::open(&empty, &cfg).is_err());
    assert!(RenderJob::open(&tmp.path().join("missing"), &cfg).is_err());
}

#[test]
fn stats_account_for_submitted_frames() {
    let tmp = TempDir::new("render-stats");
    let dir = tmp.path().join("rec");
    make_session(&dir);
    let (w, h) = (320, 240);
    let cfg = Config {
        hardware: false,
        codec: crate::config::Codec::H264,
        width: w,
        height: h,
        ..Config::default()
    };
    let mut job = RenderJob::open(&dir, &cfg).expect("open");
    assert_eq!(job.stats(), crate::render_stats::RenderStats::default());
    if tool("ffmpeg").is_none() {
        return;
    }
    for _ in 0..30 {
        job.submit_frame(w, h, vec![GRAY; (w * h) as usize].into_flattened(), true)
            .expect("submit");
    }
    let s = job.stats();
    assert_eq!(s.frames, 30);
    assert!(s.wall > Duration::ZERO && s.wall >= s.submit, "{s:?}");
    assert!(s.submit >= s.queue_wait, "{s:?}");
    assert!(s.pipe_write > Duration::ZERO, "{s:?}");
    assert!(s.to_string().starts_with("30 frames in "), "{s}");
    assert!(
        job.submit_frame(4, 4, vec![GRAY; 3].into_flattened(), true)
            .is_err()
    );
    assert_eq!(job.stats().frames, 30, "failed submits are not counted");
    job.cancel();
}

#[test]
fn a_prepared_ffmpeg_takes_the_frames_of_its_size() {
    if tool("ffmpeg").is_none() {
        return;
    }
    let tmp = TempDir::new("render-prepared");
    let dir = tmp.path().join("rec");
    make_session(&dir);
    let cfg = Config {
        hardware: false,
        codec: crate::config::Codec::H264,
        ..Config::default()
    };
    let mut job = RenderJob::open(&dir, &cfg).expect("open");
    job.prepare_encoder(64, 64, false);
    for _ in 0..job.total_frames() {
        job.submit_frame(64, 64, vec![GRAY; 64 * 64].into_flattened(), false)
            .expect("submit");
    }
    let video = job.finish().expect("finish");
    assert!(video.is_file());

    // A prepared ffmpeg of another size is stopped and its file goes; the frame starts its own.
    let mut job = RenderJob::open(&dir, &cfg).expect("open again");
    job.prepare_encoder(32, 32, false);
    job.submit_frame(64, 64, vec![GRAY; 64 * 64].into_flattened(), false)
        .expect("other size");
    job.cancel();
    assert!(!video.exists(), "the cancelled render leaves no video");

    // Cancelled before any frame: the prepared ffmpeg leaves nothing behind either.
    let mut job = RenderJob::open(&dir, &cfg).expect("open a third time");
    job.prepare_encoder(64, 64, false);
    job.cancel();
    assert!(!video.exists());
}

#[test]
fn recording_and_render_job_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<crate::recording::Recording>();
    assert_send::<RenderJob>();
}

#[test]
fn labels_hold_from_the_frame_they_were_set_on() {
    let tmp = TempDir::new("render-labels");
    let dir = tmp.path().join("rec");
    make_session(&dir);
    let cfg = Config {
        hardware: false,
        ..Config::default()
    };
    let mut job = RenderJob::open(&dir, &cfg).expect("open");
    job.label_next("");
    assert!(job.labels().is_empty());
    job.label_next("revenant overhead +1");
    job.label_next("revenant overhead +2");
    assert_eq!(job.labels(), &[(0, "revenant overhead +2".to_owned())]);
    if tool("ffmpeg").is_some() {
        job.submit_frame(64, 64, vec![GRAY; 64 * 64].into_flattened(), false)
            .expect("frame");
        job.label_next("revenant overhead +2");
        job.label_next("");
        assert_eq!(
            job.labels(),
            &[(0, "revenant overhead +2".to_owned()), (1, String::new())]
        );
    }
    job.cancel();
}
