//! RenderJob with options: slow motion, presets, focus, naming and contact sheets.

use std::path::Path;

use crate::config::{Config, Container};
use crate::director::{Focus, OpenWorld, ShotPreset};
use crate::encoder_tests::{sine_wav, test_config};
use crate::render::{RenderJob, frames_for};
use crate::render_options::RenderOptions;
use crate::session::{AudioTrack, SESSION_VERSION, Session};
use crate::test_util::{TempDir, ffprobe_json, tool};
use crate::track::TrackSample;

const W: u32 = 640;
const H: u32 = 360;

/// A 0.5 s recording with game audio, the player walking +Z.
fn short_session(dir: &Path) {
    std::fs::create_dir_all(dir).expect("dir");
    sine_wav(&dir.join("game.wav"), 48_000, 2, 0.6);
    Session {
        version: SESSION_VERSION,
        duration_ms: 500,
        game_audio: Some(AudioTrack {
            file: "game.wav".into(),
            offset_ms: 0.0,
            sample_rate: 48_000,
            channels: 2,
            dropped_samples: 0,
        }),
        mic_audio: None,
        gui_frames: Vec::new(),
        gui_events: Vec::new(),
        track: (0..=10)
            .map(|i| TrackSample::at(i * 50, [0.0, 65.62, f64::from(i) * 0.2], 0.0, 0.0))
            .collect(),
        warnings: Vec::new(),
    }
    .save(dir)
    .expect("session");
}

fn opts(words: &[&str]) -> RenderOptions {
    RenderOptions::parse(words).expect("options")
}

#[test]
fn frame_counts_for_speeds() {
    assert_eq!(frames_for(2000, 60, 1.0), 120);
    assert_eq!(frames_for(2000, 60, 0.25), 480);
    assert_eq!(frames_for(500, 60, 0.25), 120);
    assert_eq!(frames_for(500, 30, 0.1), 150);
    assert_eq!(frames_for(1, 60, 1.0), 1);
}

#[test]
fn slow_motion_steps_and_names() {
    let tmp = TempDir::new("slow-open");
    let dir = tmp.path().join("rec1");
    short_session(&dir);
    let cfg = Config {
        hardware: false,
        ..Config::default()
    };
    let job = RenderJob::open_with(
        &dir,
        &cfg,
        opts(&["side", "x0.25", "focus=soulscraft:revenant"]),
    )
    .expect("open");
    assert_eq!(job.total_frames(), 120);
    assert!((job.replay_step_ms() - 1000.0 / 60.0 * 0.25).abs() < 1e-9);
    assert!((job.frame_step_ms() - 1000.0 / 60.0).abs() < 1e-9);
    assert_eq!(
        job.output_path(),
        dir.join("letscam_rec1_side_soulscraft-revenant_x0.25.mp4")
    );
    assert_eq!(job.options().focus.as_deref(), Some("soulscraft:revenant"));
    let plain = RenderJob::open(&dir, &cfg).expect("open");
    assert_eq!(plain.output_path(), dir.join("letscam_rec1.mp4"));
    assert_eq!(plain.total_frames(), 30);
    let bad = RenderOptions {
        speed: 3.0,
        ..RenderOptions::default()
    };
    assert!(RenderJob::open_with(&dir, &cfg, bad).is_err());
}

#[test]
fn preset_poses_reach_the_job_and_follow_replay_time() {
    let tmp = TempDir::new("slow-poses");
    let dir = tmp.path().join("rec2");
    short_session(&dir);
    let cfg = Config {
        hardware: false,
        ..Config::default()
    };
    let mut fp = RenderJob::open_with(&dir, &cfg, opts(&["fp", "x0.5"])).expect("open");
    let mut poses = Vec::new();
    while let Some(p) = fp.next_pose(&OpenWorld) {
        poses.push(p);
    }
    assert_eq!(poses.len(), 60);
    // Frame 30 at x0.5 shows replay time 250 ms: the eye is at z = 1.0.
    assert!((poses[30].pos[2] - 1.0).abs() < 1e-6, "{:?}", poses[30].pos);
    let mut top = RenderJob::open_with(&dir, &cfg, opts(&["top"])).expect("open");
    let mob = Focus {
        pos: [5.0, 65.0, 5.0],
        height: 1.0,
        yaw: None,
    };
    let p = top.next_pose_focus(&OpenWorld, Some(mob)).expect("pose");
    assert!(
        (p.pos[0] - 5.0).abs() < 0.2 && p.pos[1] > 74.0,
        "{:?}",
        p.pos
    );
    assert_eq!(top.options().preset, ShotPreset::Top);
}

#[test]
fn slow_motion_render_with_contact_sheet() {
    if tool("ffmpeg").is_none() {
        return;
    }
    let tmp = TempDir::new("slow-sheet");
    let dir = tmp.path().join("rec3");
    short_session(&dir);
    let cfg = test_config(Container::Mp4);
    let mut job =
        RenderJob::open_with(&dir, &cfg, opts(&["orbit", "x0.25", "sheet=1"])).expect("open");
    let mut n = 0u32;
    while job.next_pose(&OpenWorld).is_some() {
        match n {
            0 => job.label_next("revenant overhead:hold 3"),
            60 => job.label_next(""),
            _ => {}
        }
        let shade = (n * 2 % 255) as u8;
        job.submit_frame(
            W,
            H,
            vec![[shade, 80, 160, 255]; (W * H) as usize].into_flattened(),
            true,
        )
        .expect("submit");
        n += 1;
    }
    assert_eq!(n, 120);
    let (video, sheets) = job.finish_with_sheets().expect("finish");
    assert_eq!(video, dir.join("letscam_rec3_orbit_x0.25.mp4"));
    assert_eq!(
        sheets,
        vec![dir.join("letscam_rec3_orbit_x0.25_sheet_01.png")]
    );
    let img = crate::pip::load_png(&sheets[0]).expect("sheet png");
    // 120 frames, a still every 12 (one game tick at x0.25): 10 stills in a 6×2 grid of
    // 480×270 thumbnails with 4 px padding and margin.
    assert_eq!((img.width, img.height), (6 * 480 + 7 * 4, 2 * 270 + 3 * 4));
    // The label is on the stills of frames 0..48 and gone from frame 60 on (the 6th still).
    assert!(yellow_in(&img, 0), "label on the first still");
    assert!(!yellow_in(&img, 5), "no label after it was cleared");
    assert!(!dir.join("letscam_rec3_orbit_x0.25_labels.txt").exists());
    let Some(json) = ffprobe_json(&video) else {
        return;
    };
    let streams = json["streams"].as_array().expect("streams");
    assert!(
        streams.iter().all(|s| s["codec_type"] != "audio"),
        "slow motion has no audio"
    );
    assert_eq!(streams[0]["nb_read_frames"], "120");
}

/// Whether the bottom-left corner of still `col` in the sheet's first row has yellow label text.
fn yellow_in(img: &crate::pip::Image, col: u32) -> bool {
    let x0 = 4 + col * (480 + 4);
    (4 + 230..4 + 266).any(|y| {
        (x0 + 8..x0 + 240).any(|x| {
            let i = ((y * img.width + x) * 4) as usize;
            let p = &img.rgba[i..i + 3];
            p[0] > 200 && p[1] > 200 && p[2] < 80
        })
    })
}
