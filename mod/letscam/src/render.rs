//! A render job: the glue asks for the next camera pose, draws the replay frame there and
//! hands the pixels back; the job composes the GUI picture-in-picture and feeds ffmpeg.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::director::{CameraPose, Director, Focus, WorldProbe};
use crate::encode::AudioInput;
use crate::encoder::{Encoder, probe_hardware};
use crate::pip::{Image, compose, load_png};
use crate::recording::GUI_DIR;
use crate::render_options::{MAX_SPEED, MIN_SPEED, RenderOptions};
use crate::render_stats::RenderStats;
use crate::session::Session;
use crate::sheet::make_sheets;

/// PiP width as a share of the frame width (spec: about 45 %).
const PIP_SCALE: f32 = 0.45;

pub struct RenderJob {
    dir: PathBuf,
    cfg: Config,
    opts: RenderOptions,
    session: Session,
    director: Director,
    fresh: Director,
    out: PathBuf,
    audio: Vec<AudioInput>,
    total_frames: u64,
    posed: u64,
    submitted: u64,
    encoder: Option<Encoder>,
    size: (u32, u32, bool),
    gui_cache: Option<(String, Option<Image>)>,
    /// Entry into the first successful `submit_frame` and exit from the latest one.
    submit_span: Option<(Instant, Instant)>,
    submit_time: Duration,
    /// Sheet labels: `(video frame, text)` from that frame on.
    labels: Vec<(u64, String)>,
}

/// FNV-1a: a stable director seed from the recording name.
fn seed_of(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn audio_inputs(dir: &Path, session: &Session) -> Vec<AudioInput> {
    [
        (&session.game_audio, "Игра"),
        (&session.mic_audio, "Микрофон"),
    ]
    .into_iter()
    .filter_map(|(track, title)| {
        let t = track.as_ref()?;
        let path = dir.join(&t.file);
        if path.is_file() {
            Some(AudioInput {
                path,
                offset_ms: t.offset_ms,
                title: title.to_string(),
            })
        } else {
            log::warn!(
                "LetsCam: нет файла {}, дорожка «{title}» пропущена",
                path.display()
            );
            None
        }
    })
    .collect()
}

/// Output frames for `duration_ms` of replay at `speed` (frame N shows replay time
/// N × 1000/fps × speed).
pub fn frames_for(duration_ms: u32, fps: u32, speed: f64) -> u64 {
    let real_time = (u64::from(duration_ms) * u64::from(fps)).div_ceil(1000);
    if speed >= MAX_SPEED {
        return real_time;
    }
    (f64::from(duration_ms) * f64::from(fps) / (1000.0 * speed) - 1e-9).ceil() as u64
}

impl RenderJob {
    /// [`RenderJob::open_with`] with default options: automatic director, real time.
    pub fn open(recording_dir: &Path, cfg: &Config) -> Result<RenderJob, String> {
        RenderJob::open_with(recording_dir, cfg, RenderOptions::default())
    }

    /// Loads `session.json`, builds the director (with the options' preset) and picks
    /// `dir/letscam_<dirname>[_preset][_focus][_xSPEED].<ext>`. Slow motion drops the audio.
    /// Falls back to software encoding when NVENC is not available.
    pub fn open_with(
        recording_dir: &Path,
        cfg: &Config,
        opts: RenderOptions,
    ) -> Result<RenderJob, String> {
        if !opts.speed.is_finite() || !(MIN_SPEED..=MAX_SPEED).contains(&opts.speed) {
            return Err(format!(
                "скорость {} вне диапазона {MIN_SPEED}..{MAX_SPEED}",
                opts.speed
            ));
        }
        let session = Session::load(recording_dir)?;
        let mut cfg = cfg.clone().validated();
        if cfg.hardware && !probe_hardware(&cfg.ffmpeg, cfg.codec) {
            log::warn!("LetsCam: NVENC недоступен, кодирую на процессоре");
            cfg = cfg.with_software();
        }
        let name = recording_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video".into());
        let total_frames = frames_for(session.duration_ms, cfg.fps, opts.speed);
        if total_frames == 0 {
            return Err(format!("запись {name} пустая: нечего рендерить"));
        }
        let director = Director::new(
            session.track.clone(),
            session.gui_events.clone(),
            cfg.director.clone(),
            seed_of(&name),
        )
        .with_preset(opts.preset);
        let audio = if opts.speed < MAX_SPEED {
            log::info!("LetsCam: замедление x{}: ролик без звука", opts.speed);
            Vec::new()
        } else {
            audio_inputs(recording_dir, &session)
        };
        Ok(RenderJob {
            dir: recording_dir.to_path_buf(),
            out: recording_dir.join(opts.file_name(&name, cfg.container.extension())),
            audio,
            fresh: director.clone(),
            director,
            cfg,
            opts,
            session,
            total_frames,
            posed: 0,
            submitted: 0,
            encoder: None,
            size: (0, 0, false),
            gui_cache: None,
            submit_span: None,
            submit_time: Duration::ZERO,
            labels: Vec::new(),
        })
    }

    /// The sheet label for the next frame to be submitted and those after it, until it changes
    /// (the mobs' attack phases; empty: none).
    pub fn label_next(&mut self, text: &str) {
        let frame = self.submitted;
        match self.labels.last_mut() {
            Some((f, t)) if *f == frame => *t = text.to_owned(),
            Some((_, t)) if t == text => {}
            None if text.is_empty() => {}
            _ => self.labels.push((frame, text.to_owned())),
        }
    }

    /// The labels set so far, `(video frame, text)`.
    pub fn labels(&self) -> &[(u64, String)] {
        &self.labels
    }

    pub fn fps(&self) -> u32 {
        self.cfg.fps
    }

    pub fn frame_step_ms(&self) -> f64 {
        1000.0 / f64::from(self.cfg.fps)
    }

    /// The replay time the next posed frame should show (ms).
    pub fn next_frame_ms(&self) -> f64 {
        self.posed as f64 * self.replay_step_ms()
    }

    /// Replay time advanced per output frame: `frame_step_ms() × speed`.
    pub fn replay_step_ms(&self) -> f64 {
        self.frame_step_ms() * self.opts.speed
    }

    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    pub fn output_path(&self) -> &Path {
        &self.out
    }

    pub fn options(&self) -> &RenderOptions {
        &self.opts
    }

    /// The session being rendered (warnings, durations).
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Pose for the next frame to draw, `None` when all frames were posed.
    pub fn next_pose(&mut self, world: &dyn WorldProbe) -> Option<CameraPose> {
        self.next_pose_focus(world, None)
    }

    /// Like [`RenderJob::next_pose`], watching `focus` (an entity the glue found in the replay
    /// world for this frame) instead of the player when given.
    pub fn next_pose_focus(
        &mut self,
        world: &dyn WorldProbe,
        focus: Option<Focus>,
    ) -> Option<CameraPose> {
        if self.posed >= self.total_frames {
            return None;
        }
        let t = self.posed as f64 * self.replay_step_ms();
        self.posed += 1;
        Some(self.director.pose_focus(t, world, focus))
    }

    /// Frame 0 pose without world probes and without touching the job's state (for parking
    /// the camera while terrain loads).
    pub fn peek_first_pose(&self) -> Option<CameraPose> {
        (self.total_frames > 0).then(|| self.fresh.peek_pose(0.0))
    }

    /// The GUI snapshot to show at `t_ms`, if a screen is open then.
    fn gui_overlay(&mut self, t_ms: f64) -> Option<&Image> {
        if !Session::gui_open_at(&self.session.gui_events, t_ms) {
            return None;
        }
        let frames = &self.session.gui_frames;
        let idx = frames.partition_point(|f| f64::from(f.t_ms) <= t_ms);
        let file = &frames.get(idx.checked_sub(1)?)?.file;
        if self.gui_cache.as_ref().is_none_or(|(f, _)| f != file) {
            let img = load_png(&self.dir.join(file))
                .map_err(|e| log::warn!("LetsCam: кадр интерфейса: {e}"))
                .ok();
            self.gui_cache = Some((file.clone(), img));
        }
        self.gui_cache.as_ref()?.1.as_ref()
    }

    /// Takes the drawn frame (4 bytes per pixel), adds the GUI PiP and sends it to ffmpeg.
    /// The first frame starts ffmpeg with its size and pixel format.
    pub fn submit_frame(
        &mut self,
        width: u32,
        height: u32,
        pixels: Vec<[u8; 4]>,
        bgra: bool,
    ) -> Result<(), String> {
        let start = Instant::now();
        self.submit_inner(width, height, pixels, bgra)?;
        let end = Instant::now();
        self.submit_time += end - start;
        let first = self.submit_span.map_or(start, |(first, _)| first);
        self.submit_span = Some((first, end));
        Ok(())
    }

    /// Timings so far (cheap: a few reads, for an overlay or a log line).
    pub fn stats(&self) -> RenderStats {
        let enc = self.encoder.as_ref();
        RenderStats {
            frames: self.submitted,
            wall: self.submit_span.map_or(Duration::ZERO, |(a, b)| b - a),
            submit: self.submit_time,
            queue_wait: enc.map_or(Duration::ZERO, Encoder::queue_wait),
            pipe_write: enc.map_or(Duration::ZERO, Encoder::pipe_write),
        }
    }

    fn submit_inner(
        &mut self,
        width: u32,
        height: u32,
        pixels: Vec<[u8; 4]>,
        bgra: bool,
    ) -> Result<(), String> {
        if self.submitted >= self.total_frames {
            return Err("все кадры уже отрендерены".into());
        }
        if pixels.len() != width as usize * height as usize {
            return Err(format!("кадр {width}x{height}: {} пикселей", pixels.len()));
        }
        if self.encoder.is_none() {
            let duration = self.total_frames as f64 / f64::from(self.cfg.fps);
            let enc = Encoder::start_with_duration(
                &self.cfg,
                &self.out,
                width,
                height,
                bgra,
                &self.audio,
                Some(duration),
            )?;
            self.encoder = Some(enc);
            self.size = (width, height, bgra);
        } else if self.size != (width, height, bgra) {
            return Err(format!(
                "размер окна изменился во время рендера ({}x{} → {width}x{height})",
                self.size.0, self.size.1
            ));
        }
        let t = self.submitted as f64 * self.replay_step_ms();
        let mut bytes = pixels.into_flattened();
        let margin = (height / 40).max(4);
        if let Some(img) = self.gui_overlay(t) {
            compose(&mut bytes, width, height, bgra, img, PIP_SCALE, margin);
        }
        let enc = self.encoder.as_mut().ok_or("кодировщик не запущен")?;
        enc.push_frame(bytes)?;
        self.submitted += 1;
        Ok(())
    }

    pub fn progress(&self) -> f32 {
        (self.submitted as f64 / self.total_frames as f64) as f32
    }

    /// Finishes the video (and the contact sheet if asked); returns the video path.
    pub fn finish(self) -> Result<PathBuf, String> {
        self.finish_with_sheets().map(|(video, _)| video)
    }

    /// Finishes the video, builds the contact sheet pages if the options ask for them, and
    /// without `keep_sources` deletes the WAVs and GUI snapshots.
    pub fn finish_with_sheets(mut self) -> Result<(PathBuf, Vec<PathBuf>), String> {
        log::info!("LetsCam: {}", self.stats());
        let enc = self
            .encoder
            .take()
            .ok_or("не отрендерено ни одного кадра")?;
        let frames = enc.frames();
        let out = enc.finish()?;
        let sheets = match self.opts.sheet_ticks {
            Some(ticks) => make_sheets(
                &self.cfg.ffmpeg,
                &out,
                frames,
                self.cfg.fps,
                self.opts.speed,
                ticks,
                &self.labels,
            )
            .map_err(|e| format!("ролик готов ({}), но {e}", out.display()))?,
            None => Vec::new(),
        };
        if !self.cfg.keep_sources {
            let wavs = [&self.session.game_audio, &self.session.mic_audio];
            for track in wavs.into_iter().flatten() {
                remove(&self.dir.join(&track.file), false);
            }
            remove(&self.dir.join(GUI_DIR), true);
        }
        log::info!("LetsCam: ролик готов: {}", out.display());
        Ok((out, sheets))
    }

    /// Stops ffmpeg and removes the partial video.
    pub fn cancel(mut self) {
        if let Some(enc) = self.encoder.take() {
            enc.cancel();
        }
    }
}

fn remove(path: &Path, dir: bool) {
    let r = if dir {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    if let Err(e) = r
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("LetsCam: не удалось удалить {}: {e}", path.display());
    }
}
