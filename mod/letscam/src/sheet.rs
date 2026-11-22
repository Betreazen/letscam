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

/// A label's text as `drawtext reinit` takes it inside a `sendcmd` file: the characters those
/// two parsers treat as syntax (and drawtext's `%` expansions) become spaces.
fn label_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if "'\\:;,%{}[]=\n\r".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// The `sendcmd` file for the labels: each `(video frame, text)` from that frame on.
pub fn labels_cmd(labels: &[(u64, String)], fps: u32) -> String {
    let fps = f64::from(fps.max(1));
    labels
        .iter()
        .map(|(frame, text)| {
            // Half a frame early, so the command is in before its frame arrives.
            let t = ((*frame as f64 - 0.5) / fps).max(0.0);
            format!("{t:.4} drawtext@lbl reinit 'text={}';\n", label_text(text))
        })
        .collect()
}

/// `<video stem>_labels.txt` next to the video: the labels' `sendcmd` file while sheets are made.
pub fn labels_path(video: &Path) -> PathBuf {
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    video.with_file_name(format!("{stem}_labels.txt"))
}

/// ffmpeg arguments for one page of a `fps` video; `ticks_per_video_s` turns video time into
/// game ticks. `labels`: a `sendcmd` file (`labels_cmd`) whose lines go at the bottom of the
/// stills. Each page seeks to its own part of the video (half a second early, the decoder starts
/// at a keyframe) and picks its stills by time, kept from the video's start (`-copyts
/// -start_at_zero`, so ticks and labels read as without the seek): decoding the whole video up
/// to every page made the sheets take twice as long.
#[allow(clippy::too_many_arguments)]
pub fn sheet_args(
    video: &Path,
    out_png: &Path,
    fps: u32,
    step: u64,
    page: Page,
    ticks_per_video_s: f64,
    font: Option<&str>,
    labels: Option<&Path>,
) -> Vec<String> {
    let fps = fps.max(1);
    let rows = page.stills.div_ceil(SHEET_COLS).max(1);
    let labels = labels.filter(|_| font.is_some());
    let mut chain = Vec::new();
    if let Some(cmd) = labels {
        chain.push(format!(
            "sendcmd=f='{}'",
            filter_path(&cmd.to_string_lossy())
        ));
    }
    chain.push(format!(
        "select='not(mod(round(t*{fps})\\,{step}))*between(round(t*{fps})\\,{}\\,{})'",
        page.first, page.last
    ));
    chain.push(format!("scale={THUMB_WIDTH}:-2"));
    if let Some(font) = font {
        chain.push(format!(
            "drawtext=fontfile='{}':text='tick %{{eif\\:floor(t*{ticks_per_video_s}+0.5)\\:d}}':\
             x=8:y=8:fontsize=24:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw=4",
            filter_path(font)
        ));
        if labels.is_some() {
            chain.push(format!(
                "drawtext@lbl=fontfile='{}':text='':x=8:y=h-th-8:fontsize=20:\
                 fontcolor=yellow:box=1:boxcolor=black@0.6:boxborderw=4",
                filter_path(font)
            ));
        }
    }
    chain.push(format!("tile={SHEET_COLS}x{rows}:padding=4:margin=4"));
    let seek = (page.first as f64 / f64::from(fps) - 0.5).max(0.0);
    let mut a: Vec<String> = ["-hide_banner", "-v", "error", "-y"]
        .map(String::from)
        .to_vec();
    a.extend(["-copyts", "-start_at_zero", "-ss"].map(String::from));
    a.push(format!("{seek:.3}"));
    a.push("-i".into());
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

/// Builds all pages for a finished video; returns their paths. `labels`: `(video frame, text)`
/// from that frame on, drawn at the bottom of the stills (the mobs' attack phases).
pub fn make_sheets(
    ffmpeg: &str,
    video: &Path,
    total_frames: u64,
    fps: u32,
    speed: f64,
    sheet_ticks: u32,
    labels: &[(u64, String)],
) -> Result<Vec<PathBuf>, String> {
    let step = sheet_step(fps, speed, sheet_ticks);
    let font = Path::new(FONT).is_file().then_some(FONT);
    if font.is_none() {
        log::warn!("LetsCam: шрифт {FONT} не найден, лист кадров будет без подписей тиков");
    }
    let cmd = labels_path(video);
    let with_labels = !labels.is_empty()
        && match std::fs::write(&cmd, labels_cmd(labels, fps)) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("LetsCam: {}: {e}", cmd.display());
                false
            }
        };
    let labels_file = with_labels.then_some(cmd.as_path());
    let page = |(i, page): (usize, Page)| {
        let png = sheet_path(video, i);
        let args = sheet_args(
            video,
            &png,
            fps,
            step,
            page,
            TICKS_PER_SECOND * speed,
            font,
            labels_file,
        );
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
        Ok(png)
    };
    // One ffmpeg per page, all at once: each takes over a second just to start (a 200 MB
    // executable), and one after another they kept the game waiting for every page.
    let out = std::thread::scope(|scope| {
        let workers: Vec<_> = pages(total_frames, step)
            .into_iter()
            .enumerate()
            .map(|job| scope.spawn(move || page(job)))
            .collect();
        workers
            .into_iter()
            .map(|w| {
                w.join()
                    .unwrap_or_else(|_| Err("поток листа кадров упал".into()))
            })
            .collect()
    });
    if with_labels {
        let _ = std::fs::remove_file(&cmd);
    }
    out
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
            60,
            12,
            page,
            5.0,
            Some(FONT),
            None,
        );
        let vf = &a[a.iter().position(|s| s == "-vf").expect("vf") + 1];
        assert!(vf.starts_with(
            "select='not(mod(round(t*60)\\,12))*between(round(t*60)\\,0\\,108)',scale=480:-2,"
        ));
        assert!(
            vf.contains("fontfile='C\\:/Windows/Fonts/arial.ttf'"),
            "{vf}"
        );
        assert!(
            vf.contains("text='tick %{eif\\:floor(t*5+0.5)\\:d}'"),
            "{vf}"
        );
        assert!(vf.ends_with("tile=6x2:padding=4:margin=4"));
        let plain = sheet_args(
            Path::new("v.mp4"),
            Path::new("s.png"),
            60,
            12,
            page,
            5.0,
            None,
            None,
        );
        assert!(!plain.iter().any(|s| s.contains("drawtext")));
        let ss =
            |args: &[String]| args[args.iter().position(|s| s == "-ss").expect("ss") + 1].clone();
        assert_eq!(ss(&plain), "0.000");
        let later = Page {
            first: 600,
            last: 888,
            stills: 25,
        };
        let a = sheet_args(
            Path::new("v.mp4"),
            Path::new("s.png"),
            60,
            12,
            later,
            5.0,
            None,
            None,
        );
        assert_eq!(
            ss(&a),
            "9.500",
            "half a second before the page's first frame"
        );
        assert!(a.windows(2).any(|w| w == ["-copyts", "-start_at_zero"]));
        assert_eq!(
            sheet_path(Path::new("d/letscam_a_x0.25.mp4"), 0),
            Path::new("d/letscam_a_x0.25_sheet_01.png")
        );
    }

    #[test]
    fn labels_go_through_sendcmd() {
        let page = Page {
            first: 0,
            last: 12,
            stills: 2,
        };
        let cmd = Path::new("C:/r/v_labels.txt");
        let a = sheet_args(
            Path::new("v.mp4"),
            Path::new("s.png"),
            60,
            12,
            page,
            5.0,
            Some(FONT),
            Some(cmd),
        );
        let vf = &a[a.iter().position(|s| s == "-vf").expect("vf") + 1];
        assert!(
            vf.starts_with("sendcmd=f='C\\:/r/v_labels.txt',select="),
            "{vf}"
        );
        assert!(vf.contains("drawtext@lbl=fontfile="), "{vf}");
        // Without a font there is nothing to draw the labels with.
        let a = sheet_args(
            Path::new("v.mp4"),
            Path::new("s.png"),
            60,
            12,
            page,
            5.0,
            None,
            Some(cmd),
        );
        assert!(!a.iter().any(|s| s.contains("sendcmd")));
        let file = labels_cmd(
            &[
                (0, "revenant overhead:hold 3".into()),
                (60, "a 'b', c;d%".into()),
            ],
            60,
        );
        assert_eq!(
            file,
            "0.0000 drawtext@lbl reinit 'text=revenant overhead hold 3';\n\
             0.9917 drawtext@lbl reinit 'text=a  b   c d ';\n"
        );
        assert_eq!(
            labels_path(Path::new("d/v_keys.mp4")),
            Path::new("d/v_keys_labels.txt")
        );
    }
}
