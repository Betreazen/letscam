//! Unit tests of the ffmpeg command line (no ffmpeg needed).

use std::path::{Path, PathBuf};

use crate::config::{Codec, Config, Container};
use crate::encode::{AudioInput, RawInput, build_args, ffmpeg_args, ffmpeg_args_with_duration};

fn audio(offsets: &[f64]) -> Vec<AudioInput> {
    offsets
        .iter()
        .enumerate()
        .map(|(i, &o)| AudioInput {
            path: PathBuf::from(format!("a{i}.wav")),
            offset_ms: o,
            title: if i == 0 {
                "Игра".into()
            } else {
                "Микрофон".into()
            },
        })
        .collect()
}

fn has(args: &[String], seq: &[&str]) -> bool {
    args.windows(seq.len())
        .any(|w| w.iter().zip(seq).all(|(a, b)| a == b))
}

fn input(pix: &str, size: &str) -> Vec<String> {
    ["-f", "rawvideo", "-pix_fmt", pix]
        .into_iter()
        .map(String::from)
        .chain(gpu_tags(pix))
        .chain(["-s", size, "-framerate", "60", "-i", "-"].map(String::from))
        .collect()
}

/// Colour tags that go with packed RGB input to NVENC.
fn gpu_tags(pix: &str) -> Vec<String> {
    if pix.ends_with('0') {
        ["-colorspace", "bt470bg", "-color_range", "tv"]
            .map(String::from)
            .to_vec()
    } else {
        Vec::new()
    }
}

fn has_input(args: &[String], pix: &str, size: &str) -> bool {
    let seq = input(pix, size);
    has(args, &seq.iter().map(String::as_str).collect::<Vec<_>>())
}

#[test]
fn nvenc_hevc_mp4_without_audio() {
    let cfg = Config::default();
    let a = ffmpeg_args(&cfg, Path::new("out.mp4"), 1920, 1080, true, &[]);
    assert!(has_input(&a, "bgr0", "1920x1080"), "{a:?}");
    assert!(has(
        &a,
        &[
            "-c:v",
            "hevc_nvenc",
            "-preset",
            "p5",
            "-rc",
            "vbr",
            "-cq",
            "23",
            "-b:v",
            "0"
        ]
    ));
    assert!(has(&a, &["-tag:v", "hvc1"]));
    assert!(has(&a, &["-map", "0:v:0"]));
    assert!(has(&a, &["-movflags", "+faststart"]));
    assert!(
        !a.iter()
            .any(|s| s == "-filter_complex" || s == "-c:a" || s == "-shortest")
    );
    assert_eq!(a.last().map(String::as_str), Some("out.mp4"));
}

#[test]
fn nvenc_without_scaling_takes_packed_rgb_and_skips_swscale() {
    for codec in [Codec::H264, Codec::Hevc, Codec::Av1] {
        let cfg = Config {
            codec,
            ..Config::default()
        };
        for (bgra, pix) in [(true, "bgr0"), (false, "rgb0")] {
            let a = ffmpeg_args(&cfg, Path::new("o.mp4"), 1920, 1080, bgra, &[]);
            assert!(has_input(&a, pix, "1920x1080"), "{codec:?} {a:?}");
            assert!(
                !a.iter().any(|s| s == "yuv420p"),
                "no CPU conversion: {a:?}"
            );
            assert!(!a.iter().any(|s| s == "bgra" || s == "rgba"), "{a:?}");
            assert!(!a.iter().any(|s| s == "-filter_complex"), "{a:?}");
        }
    }
}

#[test]
fn nvenc_with_scaling_keeps_the_cpu_path() {
    let a = ffmpeg_args(&Config::default(), Path::new("o.mp4"), 1280, 720, true, &[]);
    assert!(has_input(&a, "bgra", "1280x720"), "{a:?}");
    assert!(has(&a, &["-pix_fmt", "yuv420p"]));
    assert!(!a.iter().any(|s| s == "-colorspace" || s == "bgr0"));
}

#[test]
fn software_encoders_keep_the_cpu_path() {
    let cfg = Config {
        hardware: false,
        ..Config::default()
    };
    let a = ffmpeg_args(&cfg, Path::new("o.mp4"), 1920, 1080, false, &[]);
    assert!(has_input(&a, "rgba", "1920x1080"), "{a:?}");
    assert!(has(&a, &["-pix_fmt", "yuv420p"]));
    assert!(!a.iter().any(|s| s == "-colorspace" || s == "rgb0"));
}

#[test]
fn raw_input_can_force_the_cpu_path() {
    let cfg = Config::default();
    let gpu = RawInput::new(&cfg, 1920, 1080, true);
    assert!(gpu.gpu_rgb);
    let cpu = RawInput {
        gpu_rgb: false,
        ..gpu
    };
    let a = build_args(&cfg, Path::new("o.mp4"), cpu, &[], None);
    assert!(has_input(&a, "bgra", "1920x1080"), "{a:?}");
    assert!(has(&a, &["-pix_fmt", "yuv420p"]));
    assert_eq!(
        build_args(&cfg, Path::new("o.mp4"), gpu, &[], None),
        ffmpeg_args(&cfg, Path::new("o.mp4"), 1920, 1080, true, &[])
    );
}

#[test]
fn software_codecs() {
    let base = Config {
        hardware: false,
        quality: 20,
        ..Config::default()
    };
    let x264 = ffmpeg_args(
        &Config {
            codec: Codec::H264,
            ..base.clone()
        },
        Path::new("o.mp4"),
        8,
        8,
        false,
        &[],
    );
    assert!(has(
        &x264,
        &["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"]
    ));
    assert!(has(&x264, &["-pix_fmt", "rgba"]));
    let x265 = ffmpeg_args(&base, Path::new("o.mp4"), 8, 8, false, &[]);
    assert!(has(&x265, &["-c:v", "libx265"]));
    let av1 = ffmpeg_args(
        &Config {
            codec: Codec::Av1,
            ..base
        },
        Path::new("o.mkv"),
        8,
        8,
        false,
        &[],
    );
    assert!(has(&av1, &["-c:v", "libsvtav1"]));
    let av1_hw = ffmpeg_args(
        &Config {
            codec: Codec::Av1,
            ..Config::default()
        },
        Path::new("o.mp4"),
        8,
        8,
        false,
        &[],
    );
    assert!(has(&av1_hw, &["-c:v", "av1_nvenc"]));
    assert!(!has(&av1_hw, &["-tag:v", "hvc1"]));
}

#[test]
fn two_tracks_with_offsets_scaling_and_titles_mkv() {
    let cfg = Config {
        container: Container::Mkv,
        ..Config::default()
    };
    let a = ffmpeg_args_with_duration(
        &cfg,
        Path::new("o.mkv"),
        1280,
        720,
        true,
        &audio(&[-120.0, 250.0]),
        Some(1.5),
    );
    let graph = a
        .iter()
        .position(|s| s == "-filter_complex")
        .map(|i| a[i + 1].clone())
        .expect("graph");
    assert!(
        graph.contains("[0:v]scale=1920:1080:flags=lanczos[v]"),
        "{graph}"
    );
    assert!(
        graph.contains("[1:a:0]atrim=start=0.120000,asetpts=PTS-STARTPTS,atrim=end=1.500000[a0]"),
        "{graph}"
    );
    assert!(
        graph.contains("[2:a:0]adelay=delays=250.000:all=1,atrim=end=1.500000[a1]"),
        "{graph}"
    );
    assert!(has(&a, &["-map", "[v]", "-map", "[a0]", "-map", "[a1]"]));
    assert!(has(&a, &["-c:a", "flac"]));
    assert!(has(
        &a,
        &[
            "-metadata:s:a:0",
            "title=Игра",
            "-metadata:s:a:1",
            "title=Микрофон"
        ]
    ));
    assert!(!has(&a, &["-movflags", "+faststart"]));
    assert!(has(&a, &["-i", "a0.wav", "-i", "a1.wav"]));
}

#[test]
fn one_track_without_offset_is_mapped_directly() {
    let a = ffmpeg_args(
        &Config::default(),
        Path::new("o.mp4"),
        1920,
        1080,
        false,
        &audio(&[0.2]),
    );
    assert!(has(&a, &["-map", "0:v:0", "-map", "1:a:0"]));
    assert!(has(&a, &["-c:a", "aac", "-b:a", "256k"]));
    assert!(has(&a, &["-metadata:s:a:0", "handler_name=Игра"]));
    assert!(!a.iter().any(|s| s == "-filter_complex"));
}
