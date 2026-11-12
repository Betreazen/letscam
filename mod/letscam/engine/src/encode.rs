//! ffmpeg command line for a render: raw frames on stdin, audio tracks from WAV files.
//!
//! Audio alignment is done in the filter graph, so every output track starts exactly at
//! video time 0 (no edit lists, which some editors ignore):
//! - positive offset (audio started after the replay): `adelay` pads silence in front;
//! - negative offset (audio started before): `atrim=start` cuts the head, `asetpts` resets;
//! - with a known video duration, `atrim=end` cuts the audio tail so it never outlasts the
//!   video (`-shortest` is not used: it can cut video when an audio track is shorter).

use std::path::{Path, PathBuf};

use crate::config::{Codec, Config, Container};

/// One audio track for the output.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioInput {
    pub path: PathBuf,
    /// Time of the first sample relative to video start, ms (may be negative).
    pub offset_ms: f64,
    /// Track title shown by players and editors («Игра», «Микрофон»).
    pub title: String,
}

/// Offsets smaller than this are treated as zero.
const OFFSET_EPS_MS: f64 = 0.5;

fn video_codec_args(cfg: &Config) -> Vec<String> {
    let q = cfg.quality.to_string();
    let v: Vec<&str> = if cfg.hardware {
        let enc = match cfg.codec {
            Codec::H264 => "h264_nvenc",
            Codec::Hevc => "hevc_nvenc",
            Codec::Av1 => "av1_nvenc",
        };
        vec![
            "-c:v", enc, "-preset", "p5", "-rc", "vbr", "-cq", &q, "-b:v", "0",
        ]
    } else {
        match cfg.codec {
            Codec::H264 => vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", &q],
            Codec::Hevc => vec![
                "-c:v",
                "libx265",
                "-preset",
                "fast",
                "-crf",
                &q,
                "-x265-params",
                "log-level=error",
            ],
            Codec::Av1 => vec!["-c:v", "libsvtav1", "-preset", "8", "-crf", &q],
        }
    };
    let mut out: Vec<String> = v.into_iter().map(String::from).collect();
    out.extend(["-pix_fmt", "yuv420p"].map(String::from));
    if cfg.container == Container::Mp4 && cfg.codec == Codec::Hevc {
        // Apple players and some editors need the hvc1 tag for HEVC in MP4.
        out.extend(["-tag:v", "hvc1"].map(String::from));
    }
    out
}

/// Filter chain for one audio input, or `None` when it can be mapped as is.
fn audio_chain(a: &AudioInput, duration_s: Option<f64>) -> Option<String> {
    let mut parts = Vec::new();
    if a.offset_ms > OFFSET_EPS_MS {
        parts.push(format!("adelay=delays={:.3}:all=1", a.offset_ms));
    } else if a.offset_ms < -OFFSET_EPS_MS {
        parts.push(format!(
            "atrim=start={:.6},asetpts=PTS-STARTPTS",
            -a.offset_ms / 1000.0
        ));
    }
    if let Some(d) = duration_s {
        parts.push(format!("atrim=end={d:.6}"));
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

/// Arguments (without the program name) for encoding raw `src_w`x`src_h` frames from stdin.
pub fn ffmpeg_args(
    cfg: &Config,
    out: &Path,
    src_w: u32,
    src_h: u32,
    src_bgra: bool,
    audio: &[AudioInput],
) -> Vec<String> {
    ffmpeg_args_with_duration(cfg, out, src_w, src_h, src_bgra, audio, None)
}

/// Like [`ffmpeg_args`]; with `duration_s` the audio tracks are cut to the video length.
pub fn ffmpeg_args_with_duration(
    cfg: &Config,
    out: &Path,
    src_w: u32,
    src_h: u32,
    src_bgra: bool,
    audio: &[AudioInput],
    duration_s: Option<f64>,
) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostats", "-loglevel", "error", "-y"]
        .map(String::from)
        .to_vec();
    let pix = if src_bgra { "bgra" } else { "rgba" };
    a.extend([
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        pix.into(),
        "-s".into(),
        format!("{src_w}x{src_h}"),
        "-framerate".into(),
        cfg.fps.to_string(),
        "-i".into(),
        "-".into(),
    ]);
    for track in audio {
        a.push("-i".into());
        a.push(track.path.to_string_lossy().into_owned());
    }
    let mut graph = Vec::new();
    let video_map = if (src_w, src_h) != (cfg.width, cfg.height) {
        graph.push(format!(
            "[0:v]scale={}:{}:flags=lanczos[v]",
            cfg.width, cfg.height
        ));
        "[v]".to_string()
    } else {
        "0:v:0".to_string()
    };
    let mut audio_maps = Vec::new();
    for (i, track) in audio.iter().enumerate() {
        match audio_chain(track, duration_s) {
            Some(chain) => {
                graph.push(format!("[{}:a:0]{chain}[a{i}]", i + 1));
                audio_maps.push(format!("[a{i}]"));
            }
            None => audio_maps.push(format!("{}:a:0", i + 1)),
        }
    }
    if !graph.is_empty() {
        a.push("-filter_complex".into());
        a.push(graph.join(";"));
    }
    a.push("-map".into());
    a.push(video_map);
    for m in audio_maps {
        a.push("-map".into());
        a.push(m);
    }
    a.extend(video_codec_args(cfg));
    if !audio.is_empty() {
        match cfg.container {
            Container::Mp4 => a.extend(["-c:a", "aac", "-b:a", "256k"].map(String::from)),
            Container::Mkv => a.extend(["-c:a", "flac"].map(String::from)),
        }
    }
    for (i, track) in audio.iter().enumerate() {
        a.push(format!("-metadata:s:a:{i}"));
        a.push(format!("title={}", track.title));
        if cfg.container == Container::Mp4 {
            // MP4 keeps the title as the track name; many editors show the handler name.
            a.push(format!("-metadata:s:a:{i}"));
            a.push(format!("handler_name={}", track.title));
        }
    }
    if cfg.container == Container::Mp4 {
        a.extend(["-movflags", "+faststart"].map(String::from));
    }
    a.push(out.to_string_lossy().into_owned());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn nvenc_hevc_mp4_without_audio() {
        let cfg = Config::default();
        let a = ffmpeg_args(&cfg, Path::new("out.mp4"), 1920, 1080, true, &[]);
        assert!(has(
            &a,
            &[
                "-f",
                "rawvideo",
                "-pix_fmt",
                "bgra",
                "-s",
                "1920x1080",
                "-framerate",
                "60",
                "-i",
                "-"
            ]
        ));
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
        assert!(has(&a, &["-pix_fmt", "yuv420p"]));
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
            graph.contains(
                "[1:a:0]atrim=start=0.120000,asetpts=PTS-STARTPTS,atrim=end=1.500000[a0]"
            ),
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
}
