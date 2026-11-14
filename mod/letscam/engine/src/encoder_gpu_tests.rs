//! NVENC with packed RGB input: the stream must stay 4:2:0 8-bit with the right colours and
//! tags (skipped with a note when ffmpeg or NVENC is missing).

use std::path::Path;

use crate::config::{Codec, Config, Container};
use crate::encoder::{Encoder, probe_hardware};
use crate::test_util::{TempDir, ffprobe_json, tool};

const W: u32 = 1920;
const H: u32 = 1080;
const FRAMES: u64 = 30;
/// Red, green, blue and a brown, as four vertical bars.
const BARS: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [200, 100, 50]];
const TOLERANCE: u8 = 8;

fn bars_frame(bgra: bool) -> Vec<u8> {
    let row: Vec<u8> = (0..W)
        .flat_map(|x| {
            let [r, g, b] = BARS[(x * 4 / W) as usize];
            if bgra { [b, g, r, 255] } else { [r, g, b, 255] }
        })
        .collect();
    row.repeat(H as usize)
}

/// First frame of the video as raw RGB24.
fn first_frame_rgb(path: &Path) -> Vec<u8> {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .expect("ffmpeg decode");
    assert_eq!(out.stdout.len(), (W * H * 3) as usize, "decoded frame size");
    out.stdout
}

fn check_colours(path: &Path, label: &str) {
    let rgb = first_frame_rgb(path);
    for (i, want) in BARS.iter().enumerate() {
        let x = (2 * i as u32 + 1) * W / 8;
        let at = ((H / 2 * W + x) * 3) as usize;
        let got = &rgb[at..at + 3];
        assert!(
            got.iter()
                .zip(want)
                .all(|(g, w)| g.abs_diff(*w) <= TOLERANCE),
            "{label}: bar {i} decoded {got:?}, expected {want:?}"
        );
    }
}

fn check_stream(path: &Path, codec_name: &str, label: &str) {
    let Some(json) = ffprobe_json(path) else {
        return;
    };
    let v = &json["streams"][0];
    assert_eq!(v["codec_type"], "video", "{label}");
    assert_eq!(v["codec_name"], codec_name, "{label}");
    assert_eq!(v["pix_fmt"], "yuv420p", "{label}: must stay 4:2:0 8-bit");
    assert_eq!(
        (v["width"].clone(), v["height"].clone()),
        (W.into(), H.into())
    );
    assert_eq!(v["r_frame_rate"], "60/1", "{label}");
    assert_eq!(v["nb_read_frames"], FRAMES.to_string(), "{label}");
    assert_eq!(
        v["color_space"], "bt470bg",
        "{label}: tags match NVENC's matrix"
    );
    assert_eq!(v["color_range"], "tv", "{label}");
    let dur: f64 = json["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse().ok())
        .expect("duration");
    let expect = FRAMES as f64 / 60.0;
    assert!(
        (dur - expect).abs() < 0.05,
        "{label}: duration {dur} vs {expect}"
    );
}

fn encode(dir: &Path, codec: Codec, container: Container, bgra: bool) -> std::path::PathBuf {
    let cfg = Config {
        codec,
        container,
        ..Config::default()
    };
    let out = dir.join(format!("{codec:?}_{bgra}.{}", container.extension()));
    let mut enc = Encoder::start(&cfg, &out, W, H, bgra, &[]).expect("start");
    let frame = bars_frame(bgra);
    for _ in 0..FRAMES {
        enc.push_frame(frame.clone()).expect("push");
    }
    enc.finish().expect("finish")
}

/// Encodes each `(container, bgra)` case with `codec` and checks stream and colours.
fn check_codec(codec: Codec, name: &str, cases: &[(Container, bool)]) {
    if tool("ffmpeg").is_none() {
        return;
    }
    if !probe_hardware("ffmpeg", codec) {
        println!("NOTE: no NVENC for {codec:?}, check skipped");
        return;
    }
    let tmp = TempDir::new("enc-gpu-rgb");
    for &(container, bgra) in cases {
        let label = format!("{codec:?} {container:?} bgra={bgra}");
        let out = encode(tmp.path(), codec, container, bgra);
        check_stream(&out, name, &label);
        check_colours(&out, &label);
    }
}

#[test]
fn nvenc_hevc_rgb_input_keeps_420_and_colours() {
    check_codec(
        Codec::Hevc,
        "hevc",
        &[(Container::Mp4, true), (Container::Mkv, false)],
    );
}

#[test]
fn nvenc_h264_rgb_input_keeps_420_and_colours() {
    check_codec(
        Codec::H264,
        "h264",
        &[(Container::Mkv, true), (Container::Mp4, false)],
    );
}

#[test]
fn nvenc_av1_rgb_input_keeps_420_and_colours() {
    check_codec(Codec::Av1, "av1", &[(Container::Mkv, false)]);
}
