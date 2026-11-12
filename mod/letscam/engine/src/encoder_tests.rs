//! Integration tests of the ffmpeg encoder (skipped with a note when ffmpeg is missing).

use std::path::{Path, PathBuf};

use crate::config::{Codec, Config, Container};
use crate::encode::AudioInput;
use crate::encoder::{Encoder, probe_hardware};
use crate::test_util::{TempDir, ffprobe_json, tool};
use crate::wav::{WavWriter, f32_to_i16};

/// Writes a 440 Hz sine WAV.
pub fn sine_wav(path: &Path, rate: u32, channels: u16, secs: f64) {
    let mut w = WavWriter::create(path, channels, rate).expect("wav");
    let n = (f64::from(rate) * secs) as u32;
    let mut buf = Vec::new();
    for i in 0..n {
        let v = f32_to_i16(
            (f64::from(i) * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin() as f32 * 0.5,
        );
        buf.extend(std::iter::repeat_n(v, usize::from(channels)));
    }
    w.write_samples(&buf).expect("write");
    w.finalize().expect("finalize");
}

/// A BGRA test frame with a moving bar.
pub fn frame(w: u32, h: u32, i: u32) -> Vec<u8> {
    let mut f = vec![0u8; (w * h * 4) as usize];
    for (p, px) in f.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = p as u32 % w;
        let on = (x + i * 8) % 128 < 16;
        px.copy_from_slice(&if on {
            [255, 255, 255, 255]
        } else {
            [40, 20, 10, 255]
        });
    }
    f
}

pub fn test_config(container: Container) -> Config {
    let hw = probe_hardware("ffmpeg", Codec::Hevc);
    println!("NVENC available: {hw}");
    Config {
        container,
        hardware: hw,
        ..Config::default()
    }
}

fn streams(json: &serde_json::Value, kind: &str) -> Vec<serde_json::Value> {
    json["streams"]
        .as_array()
        .map(|s| {
            s.iter()
                .filter(|x| x["codec_type"] == kind)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub fn assert_video(path: &Path, frames: u64, titles: &[&str]) {
    let Some(json) = ffprobe_json(path) else {
        return;
    };
    let video = streams(&json, "video");
    assert_eq!(video.len(), 1);
    assert_eq!(video[0]["width"], 1920);
    assert_eq!(video[0]["height"], 1080);
    assert_eq!(video[0]["r_frame_rate"], "60/1");
    assert_eq!(video[0]["nb_read_frames"], frames.to_string());
    let audio = streams(&json, "audio");
    assert_eq!(audio.len(), titles.len());
    for (s, t) in audio.iter().zip(titles) {
        let tags = &s["tags"];
        let name = if tags["title"].is_null() {
            &tags["name"]
        } else {
            &tags["title"]
        };
        assert_eq!(name, *t, "{tags}");
    }
    let dur: f64 = json["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse().ok())
        .expect("duration");
    let expect = frames as f64 / 60.0;
    assert!((dur - expect).abs() < 0.05, "duration {dur} vs {expect}");
}

fn encode(dir: &Path, container: Container) -> Option<PathBuf> {
    tool("ffmpeg")?;
    let game = dir.join("game.wav");
    let mic = dir.join("mic.wav");
    sine_wav(&game, 48_000, 2, 1.3);
    sine_wav(&mic, 44_100, 1, 1.0);
    let audio = [
        AudioInput {
            path: game,
            offset_ms: -100.0,
            title: "Игра".into(),
        },
        AudioInput {
            path: mic,
            offset_ms: 250.0,
            title: "Микрофон".into(),
        },
    ];
    let cfg = test_config(container);
    let out = dir.join(format!("out.{}", container.extension()));
    let mut enc = Encoder::start_with_duration(&cfg, &out, 1280, 720, true, &audio, Some(1.0))
        .expect("start");
    for i in 0..60 {
        enc.push_frame(frame(1280, 720, i)).expect("push");
    }
    assert_eq!(enc.frames(), 60);
    Some(enc.finish().expect("finish"))
}

/// Decodes one audio stream to mono s16 PCM at `rate`.
fn decode_audio(path: &Path, stream: usize, rate: u32) -> Vec<i16> {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map",
            &format!("0:a:{stream}"),
            "-ac",
            "1",
            "-ar",
            &rate.to_string(),
            "-f",
            "s16le",
            "-",
        ])
        .output()
        .expect("ffmpeg decode");
    out.stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect()
}

fn peak(s: &[i16]) -> u16 {
    s.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0)
}

#[test]
fn encodes_mp4_with_two_titled_tracks() {
    let tmp = TempDir::new("enc-mp4");
    let Some(out) = encode(tmp.path(), Container::Mp4) else {
        return;
    };
    assert_video(&out, 60, &["Игра", "Микрофон"]);
}

#[test]
fn encodes_mkv_and_offsets_are_applied() {
    let tmp = TempDir::new("enc-mkv");
    let Some(out) = encode(tmp.path(), Container::Mkv) else {
        return;
    };
    assert_video(&out, 60, &["Игра", "Микрофон"]);
    let mic = decode_audio(&out, 1, 44_100);
    let ms = |m: usize| m * 44_100 / 1000;
    assert!(
        peak(&mic[..ms(240)]) < 50,
        "mic must be silent before +250 ms"
    );
    assert!(
        peak(&mic[ms(260)..ms(900)]) > 10_000,
        "mic tone after +250 ms"
    );
    let game = decode_audio(&out, 0, 48_000);
    assert!(
        peak(&game[..4800]) > 10_000,
        "game audio starts at once (head trimmed)"
    );
    let secs = game.len() as f64 / 48_000.0;
    assert!(
        (secs - 1.0).abs() < 0.03,
        "game audio cut to video length: {secs}"
    );
}

#[test]
fn bad_frame_size_is_an_error() {
    let tmp = TempDir::new("enc-bad");
    if tool("ffmpeg").is_none() {
        return;
    }
    let cfg = Config {
        hardware: false,
        codec: Codec::H264,
        ..Config::default()
    };
    let out = tmp.path().join("x.mp4");
    let mut enc = Encoder::start(&cfg, &out, 64, 64, false, &[]).expect("start");
    assert!(enc.push_frame(vec![0; 10]).is_err());
    enc.cancel();
    assert!(!out.exists());
}

#[test]
fn ffmpeg_errors_are_reported_with_its_output() {
    let tmp = TempDir::new("enc-fail");
    if tool("ffmpeg").is_none() {
        return;
    }
    let audio = [AudioInput {
        path: tmp.path().join("missing.wav"),
        offset_ms: 0.0,
        title: "Игра".into(),
    }];
    let cfg = Config {
        hardware: false,
        codec: Codec::H264,
        ..Config::default()
    };
    let mut enc =
        Encoder::start(&cfg, &tmp.path().join("x.mp4"), 64, 64, false, &audio).expect("start");
    let mut err = None;
    for _ in 0..200 {
        if let Err(e) = enc.push_frame(vec![0; 64 * 64 * 4]) {
            err = Some(e);
            break;
        }
    }
    let err = match err {
        Some(e) => e,
        None => enc.finish().expect_err("must fail"),
    };
    assert!(err.contains("missing.wav"), "{err}");
}

#[test]
fn missing_ffmpeg_is_a_clear_error() {
    let cfg = Config {
        ffmpeg: "letscam-no-such-ffmpeg".into(),
        ..Config::default()
    };
    let err = Encoder::start(
        &cfg,
        &std::env::temp_dir().join("never.mp4"),
        8,
        8,
        false,
        &[],
    )
    .err()
    .expect("error");
    assert!(err.contains("ffmpeg"), "{err}");
    assert!(!probe_hardware("letscam-no-such-ffmpeg", Codec::Hevc));
}
