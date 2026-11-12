//! Contact sheets: stills from a rendered video tiled into PNG pages, each labelled with the
//! replay time in game ticks, so a frame-by-frame check needs no video player.

use std::path::{Path, PathBuf};

use crate::encoder::command;

pub const SHEET_COLS: u64 = 6;
pub const SHEET_ROWS: u64 = 8;
pub const THUMB_WIDTH: u32 = 480;
/// Game ticks per second of replay time.
pub const TICKS_PER_SECOND: f64 = 20.0;
const FONT: &str = "C:/Windows/Fonts/arial.ttf";

/// Every how many video frames a still is taken (≥ 1).
pub fn sheet_step(fps: u32, speed: f64, sheet_ticks: u32) -> u64 {
    let k = (f64::from(fps) * f64::from(sheet_ticks) / TICKS_PER_SECOND / speed).round();
    if k.is_finite() && k >= 1.0 {
        k as u64
    } else {
        1
    }
}

/// One sheet page: video frame range and how many stills it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub first: u64,
    pub last: u64,
    pub stills: u64,
}

/// Splits the stills of `total_frames` into pages of at most 6×8.
pub fn pages(total_frames: u64, step: u64) -> Vec<Page> {
    let step = step.max(1);
    let stills = total_frames.div_ceil(step);
    let per_page = SHEET_COLS * SHEET_ROWS;
    (0..stills.div_ceil(per_page))
        .map(|p| {
            let s0 = p * per_page;
            let s1 = ((p + 1) * per_page).min(stills) - 1;
            Page {
                first: s0 * step,
                last: s1 * step,
                stills: s1 - s0 + 1,
            }
        })
        .collect()
}

/// Escapes a path for a filter option value (`:` and `'` are special there).
fn filter_path(p: &str) -> String {
    p.replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "\\'")
}

/// ffmpeg arguments for one page; `ticks_per_video_s` turns video time into game ticks.
pub fn sheet_args(
    video: &Path,
    out_png: &Path,
    step: u64,
    page: Page,
    ticks_per_video_s: f64,
    font: Option<&str>,
) -> Vec<String> {
    let rows = page.stills.div_ceil(SHEET_COLS).max(1);
    let mut chain = vec![
        format!(
            "select='not(mod(n\\,{step}))*between(n\\,{}\\,{})'",
            page.first, page.last
        ),
        format!("scale={THUMB_WIDTH}:-2"),
    ];
    if let Some(font) = font {
        chain.push(format!(
            "drawtext=fontfile='{}':text='tick %{{eif\\:floor(t*{ticks_per_video_s}+0.5)\\:d}}':\
             x=8:y=8:fontsize=24:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw=4",
            filter_path(font)
        ));
    }
    chain.push(format!("tile={SHEET_COLS}x{rows}:padding=4:margin=4"));
    let mut a: Vec<String> = ["-hide_banner", "-v", "error", "-y", "-i"]
        .map(String::from)
        .to_vec();
    a.push(video.to_string_lossy().into_owned());
    a.push("-vf".into());
    a.push(chain.join(","));
    a.extend(["-fps_mode", "vfr", "-frames:v", "1", "-update", "1"].map(String::from));
    a.push(out_png.to_string_lossy().into_owned());
    a
}

/// `<video stem>_sheet_NN.png` next to the video.
pub fn sheet_path(video: &Path, page: usize) -> PathBuf {
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    video.with_file_name(format!("{stem}_sheet_{:02}.png", page + 1))
}

/// Builds all pages for a finished video; returns their paths.
pub fn make_sheets(
    ffmpeg: &str,
    video: &Path,
    total_frames: u64,
    fps: u32,
    speed: f64,
    sheet_ticks: u32,
) -> Result<Vec<PathBuf>, String> {
    let step = sheet_step(fps, speed, sheet_ticks);
    let font = Path::new(FONT).is_file().then_some(FONT);
    if font.is_none() {
        log::warn!("LetsCam: шрифт {FONT} не найден, лист кадров будет без подписей тиков");
    }
    let mut out = Vec::new();
    for (i, page) in pages(total_frames, step).into_iter().enumerate() {
        let png = sheet_path(video, i);
        let args = sheet_args(video, &png, step, page, TICKS_PER_SECOND * speed, font);
        let res = command(ffmpeg)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("не удалось запустить ffmpeg для листа кадров: {e}"))?;
        if !res.status.success() || !png.is_file() {
            return Err(format!(
                "лист кадров {}: ffmpeg: {}",
                png.display(),
                String::from_utf8_lossy(&res.stderr).trim()
            ));
        }
        out.push(png);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_matches_ticks_and_speed() {
        assert_eq!(sheet_step(60, 0.25, 1), 12);
        assert_eq!(sheet_step(60, 1.0, 2), 6);
        assert_eq!(sheet_step(30, 1.0, 1), 2);
        assert_eq!(sheet_step(30, 1.0, 0), 1);
    }

    #[test]
    fn pages_hold_at_most_48_stills() {
        assert_eq!(
            pages(120, 12),
            vec![Page {
                first: 0,
                last: 108,
                stills: 10
            }]
        );
        let p = pages(1200, 12);
        assert_eq!(p.len(), 3);
        assert_eq!(
            p[0],
            Page {
                first: 0,
                last: 47 * 12,
                stills: 48
            }
        );
        assert_eq!(
            p[2],
            Page {
                first: 96 * 12,
                last: 99 * 12,
                stills: 4
            }
        );
        assert!(pages(0, 5).is_empty());
    }

    #[test]
    fn args_escape_the_filter() {
        let page = Page {
            first: 0,
            last: 108,
            stills: 10,
        };
        let a = sheet_args(
            Path::new("v.mp4"),
            Path::new("s.png"),
            12,
            page,
            5.0,
            Some(FONT),
        );
        let vf = &a[a.iter().position(|s| s == "-vf").expect("vf") + 1];
        assert!(vf.starts_with("select='not(mod(n\\,12))*between(n\\,0\\,108)',scale=480:-2,"));
        assert!(
            vf.contains("fontfile='C\\:/Windows/Fonts/arial.ttf'"),
            "{vf}"
        );
        assert!(
            vf.contains("text='tick %{eif\\:floor(t*5+0.5)\\:d}'"),
            "{vf}"
        );
        assert!(vf.ends_with("tile=6x2:padding=4:margin=4"));
        let plain = sheet_args(Path::new("v.mp4"), Path::new("s.png"), 12, page, 5.0, None);
        assert!(!plain.iter().any(|s| s.contains("drawtext")));
        assert_eq!(
            sheet_path(Path::new("d/letscam_a_x0.25.mp4"), 0),
            Path::new("d/letscam_a_x0.25_sheet_01.png")
        );
    }
}
