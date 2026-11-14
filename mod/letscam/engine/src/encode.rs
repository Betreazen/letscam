//! ffmpeg command line for a render: raw frames on stdin, audio tracks from WAV files.
//!
//! Audio alignment is done in the filter graph, so every output track starts exactly at
//! video time 0 (no edit lists, which some editors ignore):
//! - positive offset (audio started after the replay): `adelay` pads silence in front;
//! - negative offset (audio started before): `atrim=start` cuts the head, `asetpts` resets;
//! - with a known video duration, `atrim=end` cuts the audio tail so it never outlasts the
//!   video (`-shortest` is not used: it can cut video when an audio track is shorter).
//!
//! Colour conversion: with NVENC and no scaling the frames go to the encoder as packed RGB
//! (`bgr0`/`rgb0`, alpha ignored) and the GPU converts them to 4:2:0 — ffmpeg runs no swscale
//! step. NVENC converts RGB with the BT.601 matrix to limited range and says so in the
//! bitstream; the input is tagged the same (`-colorspace bt470bg -color_range tv`), otherwise
//! MKV would carry the RGB input's `gbr`/`pc` tags. Software encoders and scaling keep the
//! CPU path: `bgra`/`rgba` in, `-pix_fmt yuv420p` out.

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

/// The raw frames on ffmpeg's stdin: size, channel order and where they become YUV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RawInput {
    pub w: u32,
    pub h: u32,
    /// Bytes are B, G, R, A (else R, G, B, A).
    pub bgra: bool,
    /// Packed RGB straight to NVENC, which converts on the GPU (else swscale on the CPU).
    pub gpu_rgb: bool,
}

impl RawInput {
    /// GPU conversion whenever the encoder is NVENC and the frame needs no scaling.
    pub(crate) fn new(cfg: &Config, w: u32, h: u32, bgra: bool) -> RawInput {
        RawInput {
            w,
            h,
            bgra,
            gpu_rgb: cfg.hardware && (w, h) == (cfg.width, cfg.height),
        }
    }

    fn input_args(self, fps: u32) -> Vec<String> {
        let pix = match (self.gpu_rgb, self.bgra) {
            (true, true) => "bgr0",
            (true, false) => "rgb0",
            (false, true) => "bgra",
            (false, false) => "rgba",
        };
        let mut a: Vec<String> = ["-f", "rawvideo", "-pix_fmt", pix]
            .map(String::from)
            .to_vec();
        if self.gpu_rgb {
            a.extend(["-colorspace", "bt470bg", "-color_range", "tv"].map(String::from));
        }
        a.extend([
            "-s".into(),
            format!("{}x{}", self.w, self.h),
            "-framerate".into(),
            fps.to_string(),
            "-i".into(),
            "-".into(),
        ]);
        a
    }
}

fn video_codec_args(cfg: &Config, gpu_rgb: bool) -> Vec<String> {
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
    if !gpu_rgb {
        out.extend(["-pix_fmt", "yuv420p"].map(String::from));
    }
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
    let src = RawInput::new(cfg, src_w, src_h, src_bgra);
    build_args(cfg, out, src, audio, duration_s)
}

/// The full command line for `src` (lets a caller force the CPU colour path).
pub(crate) fn build_args(
    cfg: &Config,
    out: &Path,
    src: RawInput,
    audio: &[AudioInput],
    duration_s: Option<f64>,
) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostats", "-loglevel", "error", "-y"]
        .map(String::from)
        .to_vec();
    a.extend(src.input_args(cfg.fps));
    for track in audio {
        a.push("-i".into());
        a.push(track.path.to_string_lossy().into_owned());
    }
    let mut graph = Vec::new();
    let video_map = if (src.w, src.h) != (cfg.width, cfg.height) {
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
    a.extend(video_codec_args(cfg, src.gpu_rgb));
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
