// LetsCam's game side: F9 and `/letscam` start and stop a recording (the studio replay plus the
// crate's audio, mic, GUI snapshots and player track); `/letscam render` leaves the world, opens
// the replay standalone and drives it one video frame per drawn frame, the director's pose on
// the studio free camera, every frame read back through a marked screenshot request.
// Compiled as the child module `game::letscam_glue`, so it reads Game's private fields.
use super::Game;
use horizon_client::text::Text;
use horizon_client_render::letscam::{self as bridge, engine};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const ID: &str = "letscam";
const KEY_TOGGLE: &str = "toggle";
const KEY_TOGGLE_DEFAULT: &str = "key.keyboard.f9";
const KEY_CANCEL: &str = "key.keyboard.escape";
const CONFIG_FILE: &str = "config/letscam-client.toml";
const DEFAULT_OUTPUT: &str = "letscam";
const REPLAY_FILE: &str = "replay.mcpr";
const TRACK_PERIOD: Duration = Duration::from_millis(50);
/// Paused replay frames drawn before the first captured one, so the terrain around the camera
/// is meshed and the first frames aren't empty.
const WARMUP_FRAMES: u32 = 120;
/// Frames in a row whose capture never came back before the render gives up.
const MAX_MISSED: u32 = 60;
const TAG_GUI: &str = "gui";
const TAG_RENDER: &str = "render";

struct Rec {
    recording: engine::Recording,
    started: Instant,
    dir: PathBuf,
    studio_path: PathBuf,
    last_sample: Option<Instant>,
    gui_open: bool,
    swings_seen: u32,
}

struct Render {
    job: engine::RenderJob,
    dir: PathBuf,
    warmup: u32,
    /// A frame was asked for and its pixels haven't come back yet.
    awaiting: bool,
    missed: u32,
    frames: u64,
    last_report: u32,
    hide_gui_before: bool,
    /// What `focus=` asked for, and the entity it settled on.
    focus: Option<FocusSpec>,
    focus_id: Option<u32>,
}

/// `focus=` of `/letscam render`: an entity id, an entity type (`minecraft:pig`,
/// `soulscraft:revenant`, or without the namespace), or `ride`: what the player sits on.
#[derive(Clone, Debug)]
enum FocusSpec {
    Id(u32),
    Kind(String),
    Ride,
}

impl FocusSpec {
    fn parse(word: &str) -> FocusSpec {
        match word {
            "ride" | "mount" | "vehicle" => FocusSpec::Ride,
            _ => word
                .parse()
                .map_or_else(|_| FocusSpec::Kind(word.to_owned()), FocusSpec::Id),
        }
    }
}

/// Within this many blocks of the eye, the nearest creature is what the player rides.
const RIDE_RADIUS: f64 = 3.0;

#[derive(Default)]
enum Mode {
    #[default]
    Idle,
    Recording(Box<Rec>),
    /// Waiting for the world to close before the replay opens.
    Leaving(Box<engine::RenderJob>, PathBuf, Option<FocusSpec>),
    Rendering(Box<Render>),
}

#[derive(Default)]
struct State {
    config: Option<engine::Config>,
    mode: Mode,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn say(game: &mut Game<'_>, line: impl Into<String>) {
    let line = line.into();
    log::info!("letscam: {line}");
    game.system_message(Text::literal(format!("[LetsCam] {line}")));
}

fn config() -> engine::Config {
    STATE.with_borrow_mut(|state| {
        state
            .config
            .get_or_insert_with(|| {
                let (config, warning) = engine::Config::load_or_create(Path::new(CONFIG_FILE));
                if let Some(warning) = warning {
                    log::warn!("letscam: {warning}");
                }
                config.validated()
            })
            .clone()
    })
}

fn output_root(config: &engine::Config) -> PathBuf {
    if config.output_dir.trim().is_empty() {
        PathBuf::from(DEFAULT_OUTPUT)
    } else {
        PathBuf::from(config.output_dir.trim())
    }
}

/// `YYYY-MM-DD_HH-MM-SS` in UTC, for folder names that sort by time.
fn stamp(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil_from_days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}-{:02}-{:02}",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

fn is_recording() -> bool {
    STATE.with_borrow(|state| matches!(state.mode, Mode::Recording(_)))
}

fn is_rendering() -> bool {
    STATE.with_borrow(|state| matches!(state.mode, Mode::Rendering(_) | Mode::Leaving(..)))
}

// ---- recording ---------------------------------------------------------------------------

fn start_recording(game: &mut Game<'_>) {
    if !STATE.with_borrow(|s| matches!(s.mode, Mode::Idle)) {
        say(game, "Уже идёт запись или рендер.");
        return;
    }
    if game.remote.is_none() || game.studio_replay.is_some() {
        say(game, "Запись работает только в мире.");
        return;
    }
    let config = config();
    let dir = output_root(&config).join(stamp(SystemTime::now()));
    let started = match game.start_studio_recording(Some("letscam")) {
        Ok(v) if v["ok"].as_bool() == Some(true) => v["path"].as_str().map(PathBuf::from),
        Ok(v) => {
            let error = v["error"].as_str().unwrap_or("неизвестная ошибка").to_owned();
            say(game, format!("Реплей не начался: {error}"));
            return;
        }
        Err(error) => {
            say(game, format!("Реплей не начался: {error}"));
            return;
        }
    };
    let replay_start = Instant::now();
    let Some(studio_path) = started else {
        say(game, "Реплей не сообщил путь к файлу.");
        let _ = game.stop_studio_recording();
        return;
    };
    let recording = match engine::Recording::start(&dir, &config, replay_start) {
        Ok(recording) => recording,
        Err(error) => {
            let _ = game.stop_studio_recording();
            say(game, format!("Запись не началась: {error}"));
            return;
        }
    };
    for warning in recording.warnings() {
        say(game, warning.clone());
    }
    let swings_seen = game.swings_started;
    STATE.with_borrow_mut(|state| {
        state.mode = Mode::Recording(Box::new(Rec {
            recording,
            started: replay_start,
            dir: dir.clone(),
            studio_path,
            last_sample: None,
            gui_open: false,
            swings_seen,
        }));
    });
    let key = game.native_mod_key(ID, KEY_TOGGLE, KEY_TOGGLE_DEFAULT);
    say(
        game,
        format!("Запись началась: {}. Остановить: {key} или /letscam stop", dir.display()),
    );
}

fn stop_recording(game: &mut Game<'_>) {
    let rec = STATE.with_borrow_mut(|state| match std::mem::take(&mut state.mode) {
        Mode::Recording(rec) => Some(rec),
        other => {
            state.mode = other;
            None
        }
    });
    let Some(rec) = rec else {
        say(game, "Запись не идёт.");
        return;
    };
    let studio_path = match game.stop_studio_recording() {
        Ok(v) if v["ok"].as_bool() == Some(true) => v["path"]
            .as_str()
            .map_or_else(|| rec.studio_path.clone(), PathBuf::from),
        // The world already closed the recorder (the player left): its file is where it began.
        _ => rec.studio_path.clone(),
    };
    let target = rec.dir.join(REPLAY_FILE);
    if let Err(error) = move_file(&studio_path, &target) {
        say(game, format!("Реплей не перенесён в папку записи: {error}"));
    }
    match rec.recording.stop(Instant::now()) {
        Ok(session) => {
            for warning in &session.warnings {
                say(game, warning.clone());
            }
            let secs = session.duration_ms / 1000;
            say(
                game,
                format!(
                    "Запись сохранена ({}:{:02}): {}. Ролик: /letscam render",
                    secs / 60,
                    secs % 60,
                    rec.dir.display()
                ),
            );
        }
        Err(error) => say(game, format!("Запись остановлена с ошибкой: {error}")),
    }
}

fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))?;
    std::fs::remove_file(from).map_err(|e| format!("{}: {e}", from.display()))
}

fn track_sample(game: &Game<'_>, rec: &mut Rec, now: Instant) -> Option<engine::TrackSample> {
    let state = game.native_client_state()?;
    let eye = state.eye_position?;
    let swings = game.swings_started;
    let attacking = swings != rec.swings_seen;
    rec.swings_seen = swings;
    Some(engine::TrackSample {
        t_ms: now.saturating_duration_since(rec.started).as_millis() as u32,
        eye,
        yaw: state.yaw,
        pitch: state.pitch,
        gui_open: rec.gui_open,
        fall_flying: game.player.as_ref().is_some_and(|p| p.fall_flying),
        hurt_or_attacking: attacking,
        dimension: String::new(),
    })
}

fn tick_recording(game: &mut Game<'_>) {
    let now = Instant::now();
    if game.remote.is_none() {
        // The world closed under the recording: keep what was recorded.
        stop_recording(game);
        return;
    }
    let gui_open = game
        .native_client_state()
        .is_some_and(|s| s.screen == super::native_client_services::NativeScreenKind::Other);
    let captured = bridge::take();
    let want_gui = STATE.with_borrow_mut(|state| {
        let Mode::Recording(rec) = &mut state.mode else {
            return false;
        };
        if gui_open != rec.gui_open {
            rec.gui_open = gui_open;
            rec.recording.gui_event(now, gui_open);
        }
        if rec
            .last_sample
            .is_none_or(|last| now.saturating_duration_since(last) >= TRACK_PERIOD)
            && let Some(sample) = track_sample(game, rec, now)
        {
            rec.last_sample = Some(now);
            rec.recording.push_track(sample);
        }
        if let Some(frame) = captured
            && frame.tag == TAG_GUI
            && rec.gui_open
        {
            rec.recording
                .push_gui_frame(now, frame.size[0], frame.size[1], frame.pixels, frame.bgra);
        }
        gui_open && rec.recording.gui_frame_due(now)
    });
    if want_gui {
        request_capture(game, TAG_GUI);
    }
}

/// Ask for the next drawn frame's pixels under `tag`; false while another screenshot waits.
fn request_capture(game: &mut Game<'_>, tag: &str) -> bool {
    let Some(renderer) = game.renderer.as_mut() else {
        return false;
    };
    if renderer.screenshot.is_some() {
        return false;
    }
    renderer.screenshot = Some(horizon_client::gfx::ScreenshotRequest {
        path: bridge::capture_path(tag),
        size: None,
        scaled: false,
    });
    true
}

// ---- rendering ---------------------------------------------------------------------------

fn recordings(config: &engine::Config) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(output_root(config)) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join(REPLAY_FILE).is_file())
        .collect();
    dirs.sort();
    dirs
}

fn list(game: &mut Game<'_>) {
    let config = config();
    let dirs = recordings(&config);
    if dirs.is_empty() {
        say(game, format!("Записей нет в {}.", output_root(&config).display()));
        return;
    }
    for dir in dirs.iter().rev().take(10) {
        let name = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let rendered = std::fs::read_dir(dir).is_ok_and(|mut it| {
            it.any(|e| {
                e.is_ok_and(|e| {
                    let p = e.path();
                    matches!(p.extension().and_then(|x| x.to_str()), Some("mp4" | "mkv"))
                })
            })
        });
        say(game, format!("{name}{}", if rendered { "  (ролик готов)" } else { "" }));
    }
}

/// `/letscam render [папка] [ракурс] [x0.25] [sheet[=N]] [focus=...]`: the first word is the
/// recording's folder when such a folder exists, the rest are the crate's render options.
fn start_render(game: &mut Game<'_>, words: &[&str]) {
    if !STATE.with_borrow(|s| matches!(s.mode, Mode::Idle)) {
        say(game, "Сначала остановите запись или дождитесь конца рендера.");
        return;
    }
    let config = config();
    let (name, option_words) = match words.split_first() {
        Some((first, rest)) if output_root(&config).join(first).join(REPLAY_FILE).is_file() => {
            (Some(*first), rest)
        }
        _ => (None, words),
    };
    let options = match engine::RenderOptions::parse(option_words) {
        Ok(options) => options,
        Err(error) => {
            say(game, error);
            return;
        }
    };
    let focus = options.focus.as_deref().map(FocusSpec::parse);
    let dir = match name {
        Some(name) => output_root(&config).join(name),
        None => match recordings(&config).pop() {
            Some(dir) => dir,
            None => {
                say(game, "Записей для рендера нет.");
                return;
            }
        },
    };
    if !dir.join(REPLAY_FILE).is_file() {
        say(game, format!("В {} нет {REPLAY_FILE}.", dir.display()));
        return;
    }
    let job = match engine::RenderJob::open_with(&dir, &config, options) {
        Ok(job) => job,
        Err(error) => {
            say(game, format!("Рендер не начался: {error}"));
            return;
        }
    };
    say(
        game,
        format!(
            "Рендер {} ({}): {} кадров. Отмена: Esc.",
            dir.display(),
            job.options().preset.word(),
            job.total_frames()
        ),
    );
    STATE.with_borrow_mut(|state| state.mode = Mode::Leaving(Box::new(job), dir, focus));
    if game.remote.is_some() {
        if let Err(error) = game.leave_server(None) {
            log::warn!("letscam: leaving the world: {error}");
        }
    }
}

fn open_replay(game: &mut Game<'_>) {
    if game.remote.is_some() || game.integrated.is_some() {
        return; // the world is still closing
    }
    let taken = STATE.with_borrow_mut(|state| match std::mem::take(&mut state.mode) {
        Mode::Leaving(job, dir, focus) => Some((job, dir, focus)),
        other => {
            state.mode = other;
            None
        }
    });
    let Some((job, dir, focus)) = taken else { return };
    let path = dir.join(REPLAY_FILE);
    let opened = game.open_studio_replay(&path.to_string_lossy());
    let ok = matches!(&opened, Ok(v) if v["ok"].as_bool() == Some(true));
    if !ok {
        let why = match opened {
            Ok(v) => v["error"].as_str().unwrap_or("неизвестная ошибка").to_owned(),
            Err(e) => e.to_string(),
        };
        job.cancel();
        say(game, format!("Реплей не открылся: {why}"));
        return;
    }
    if let Some(driver) = game.studio_replay.as_mut() {
        driver.set_paused(true);
    }
    let hide_gui_before = game.ui.hide_gui;
    game.ui.hide_gui = true;
    STATE.with_borrow_mut(|state| {
        state.mode = Mode::Rendering(Box::new(Render {
            job: *job,
            dir,
            warmup: WARMUP_FRAMES,
            awaiting: false,
            missed: 0,
            frames: 0,
            last_report: 0,
            hide_gui_before,
            focus,
            focus_id: None,
        }));
    });
}

/// The focus entity for this frame: the one already settled on while it lives, else the first
/// match for `spec` (the nearest to the recorded player's eye), which it then sticks to.
fn find_focus(game: &Game<'_>, spec: &FocusSpec, locked: &mut Option<u32>) -> Option<engine::Focus> {
    let state = game.native_client_state()?;
    let eye = state.eye_position?;
    let entities = game.native_client_entities();
    let alive = || entities.iter().filter(|e| e.alive && !e.spectator);
    let distance = |p: [f64; 3]| {
        let d = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let nearest = |pick: &dyn Fn(&super::native_client_services::NativeClientEntity) -> bool| {
        alive()
            .filter(|e| pick(e))
            .min_by(|a, b| distance(a.position).total_cmp(&distance(b.position)))
    };
    let entity = match locked.and_then(|id| alive().find(|e| e.id == id)) {
        Some(entity) => entity,
        None => {
            let found = match spec {
                FocusSpec::Id(id) => alive().find(|e| e.id == *id),
                FocusSpec::Kind(kind) => nearest(&|e| {
                    e.kind == *kind || e.kind.rsplit(':').next() == Some(kind.as_str())
                }),
                FocusSpec::Ride => nearest(&|e| {
                    e.player_uuid.is_none() && distance(e.position) <= RIDE_RADIUS
                }),
            }?;
            *locked = Some(found.id);
            found
        }
    };
    let [x, y, z] = entity.position_at(state.world_partial_tick);
    Some(engine::Focus {
        pos: [x, y + f64::from(entity.height) / 2.0, z],
        height: entity.height,
        yaw: Some(entity.yaw),
    })
}

struct Probe<'g, 'a>(&'g Game<'a>);

impl engine::WorldProbe for Probe<'_, '_> {
    fn clear(&self, from: [f64; 3], to: [f64; 3]) -> bool {
        self.0.native_client_visual_clear(from, to)
    }
}

fn apply_pose(game: &mut Game<'_>, pose: engine::CameraPose) {
    let position = glam::DVec3::from_array(pose.pos);
    let cam = &mut game.studio_freecam;
    if cam.active {
        cam.position = position;
        cam.yaw = pose.yaw;
        cam.pitch = pose.pitch;
        cam.fov = pose.fov;
        cam.velocity = glam::DVec3::ZERO;
        cam.locked_to_path = false;
    } else {
        cam.activate(position, pose.yaw, pose.pitch, pose.fov);
    }
}

/// HEAD of `advance_studio_replay`: while rendering, the replay moves one video frame after each
/// captured frame and not at all otherwise, whatever the wall clock did.
pub fn before_replay_advance(game: &mut Game<'_>) {
    let now = Instant::now();
    let step = STATE.with_borrow_mut(|state| {
        let Mode::Rendering(render) = &mut state.mode else {
            return None;
        };
        if render.warmup > 0 || !render.awaiting {
            return Some(Duration::ZERO);
        }
        let Some(frame) = bridge::take() else {
            return Some(Duration::ZERO);
        };
        if frame.tag != TAG_RENDER {
            return Some(Duration::ZERO);
        }
        render.awaiting = false;
        render.missed = 0;
        if let Err(error) =
            render
                .job
                .submit_frame(frame.size[0], frame.size[1], frame.pixels, frame.bgra)
        {
            log::error!("letscam: encoding: {error}");
            render.missed = MAX_MISSED; // ends the render on this frame
            return Some(Duration::ZERO);
        }
        render.frames += 1;
        Some(Duration::from_secs_f64(render.job.replay_step_ms() / 1000.0))
    });
    if let Some(step) = step {
        game.last_replay_tick = Some(now.checked_sub(step).unwrap_or(now));
    }
}

fn tick_render(game: &mut Game<'_>) {
    enum Next {
        Wait,
        Warm,
        Pose,
        Retry,
        Fail(String),
    }
    let next = STATE.with_borrow_mut(|state| {
        let Mode::Rendering(render) = &mut state.mode else {
            return Next::Wait;
        };
        if render.missed >= MAX_MISSED {
            return Next::Fail("кадры перестали приходить с видеокарты или кодировщик остановился".into());
        }
        if render.warmup > 0 {
            render.warmup -= 1;
            return Next::Warm;
        }
        if render.awaiting {
            render.missed += 1;
            return Next::Retry;
        }
        let percent = (render.job.progress() * 100.0) as u32;
        if percent / 10 > render.last_report / 10 {
            render.last_report = percent;
            log::info!("letscam: render {percent}% ({} frames)", render.frames);
        }
        Next::Pose
    });
    match next {
        Next::Wait => {}
        Next::Fail(why) => finish_render(game, Err(why)),
        Next::Warm => {
            let pose = STATE.with_borrow_mut(|state| match &mut state.mode {
                Mode::Rendering(render) => render.job.peek_first_pose(),
                _ => None,
            });
            if let Some(pose) = pose {
                apply_pose(game, pose);
            }
            if STATE.with_borrow(|s| matches!(&s.mode, Mode::Rendering(r) if r.warmup == 0))
                && let Some(driver) = game.studio_replay.as_mut()
            {
                driver.set_paused(false);
            }
        }
        Next::Retry => {
            request_capture(game, TAG_RENDER);
        }
        Next::Pose => {
            let (spec, mut locked) = STATE.with_borrow(|state| match &state.mode {
                Mode::Rendering(render) => (render.focus.clone(), render.focus_id),
                _ => (None, None),
            });
            let focus = spec
                .as_ref()
                .and_then(|spec| find_focus(game, spec, &mut locked));
            let pose = {
                let probe = Probe(game);
                STATE.with_borrow_mut(|state| match &mut state.mode {
                    Mode::Rendering(render) => {
                        if render.focus_id.is_none() && locked.is_some() {
                            log::info!("letscam: focus on entity {locked:?}");
                        }
                        render.focus_id = locked;
                        render.job.next_pose_focus(&probe, focus)
                    }
                    _ => None,
                })
            };
            match pose {
                Some(pose) => {
                    apply_pose(game, pose);
                    if request_capture(game, TAG_RENDER) {
                        STATE.with_borrow_mut(|state| {
                            if let Mode::Rendering(render) = &mut state.mode {
                                render.awaiting = true;
                            }
                        });
                    }
                }
                None => finish_render(game, Ok(())),
            }
        }
    }
}

fn finish_render(game: &mut Game<'_>, outcome: Result<(), String>) {
    let render = STATE.with_borrow_mut(|state| match std::mem::take(&mut state.mode) {
        Mode::Rendering(render) => Some(render),
        Mode::Leaving(job, ..) => {
            job.cancel();
            None
        }
        other => {
            state.mode = other;
            None
        }
    });
    let Some(render) = render else { return };
    let Render { job, dir, hide_gui_before, frames, .. } = *render;
    let message = match outcome {
        Ok(()) => match job.finish_with_sheets() {
            Ok((path, sheets)) if sheets.is_empty() => {
                format!("Ролик готов ({frames} кадров): {}", path.display())
            }
            Ok((path, sheets)) => format!(
                "Ролик готов ({frames} кадров): {}; листов кадров: {}",
                path.display(),
                sheets.len()
            ),
            Err(error) => format!("Рендер {} не удался: {error}", dir.display()),
        },
        Err(why) => {
            job.cancel();
            format!("Рендер остановлен: {why}")
        }
    };
    game.ui.hide_gui = hide_gui_before;
    game.close_studio_replay();
    say(game, message);
}

// ---- hooks -------------------------------------------------------------------------------

pub fn on_frame(game: &mut Game<'_>) {
    enum Phase {
        Idle,
        Recording,
        Leaving,
        Rendering,
    }
    let phase = STATE.with_borrow(|state| match state.mode {
        Mode::Idle => Phase::Idle,
        Mode::Recording(_) => Phase::Recording,
        Mode::Leaving(..) => Phase::Leaving,
        Mode::Rendering(_) => Phase::Rendering,
    });
    match phase {
        Phase::Idle => {}
        Phase::Recording => tick_recording(game),
        Phase::Leaving => open_replay(game),
        Phase::Rendering => {
            if game.studio_replay.is_none() {
                finish_render(game, Err("реплей закрыт".into()));
            } else {
                tick_render(game);
            }
        }
    }
}

pub fn on_key(game: &mut Game<'_>, key: &str, pressed: bool) {
    if !pressed {
        return;
    }
    if is_rendering() {
        if key == KEY_CANCEL {
            finish_render(game, Err("отменён".into()));
        }
        return;
    }
    if key == game.native_mod_key(ID, KEY_TOGGLE, KEY_TOGGLE_DEFAULT) {
        if is_recording() {
            stop_recording(game);
        } else {
            start_recording(game);
        }
    }
}

pub fn on_command(game: &mut Game<'_>, command: &str, ran: &mut bool) {
    let mut words = command.split_whitespace();
    if words.next() != Some("letscam") {
        return;
    }
    *ran = true;
    match words.next() {
        Some("start") => start_recording(game),
        Some("stop") => stop_recording(game),
        Some("list") => list(game),
        Some("render") => start_render(game, &words.collect::<Vec<_>>()),
        Some("cancel") => finish_render(game, Err("отменён".into())),
        Some("reload") => {
            STATE.with_borrow_mut(|state| state.config = None);
            let _ = config();
            say(game, format!("Настройки перечитаны из {CONFIG_FILE}."));
        }
        _ => say(
            game,
            "/letscam start | stop | list | cancel | reload | render [папка] [auto|fp|shoulder|orbit|side|front|top|fixed:x,y,z] [x0.25] [sheet[=тиков]] [focus=id|тип|ride]",
        ),
    }
}

pub fn on_hud(game: &mut Game<'_>, batch: &mut horizon_client::hud::HudBatch) {
    let elapsed = STATE.with_borrow(|state| match &state.mode {
        Mode::Recording(rec) => Some(Instant::now().saturating_duration_since(rec.started)),
        _ => None,
    });
    let Some(elapsed) = elapsed else { return };
    let Some(state) = game.native_client_state() else { return };
    if state.gui_hidden {
        return;
    }
    let secs = elapsed.as_secs();
    // Blinks once a second, as a camera's REC light does.
    let dot = if secs % 2 == 0 { "●" } else { " " };
    let text = format!("{dot} REC {}:{:02}", secs / 60, secs % 60);
    let atlas = &game.gui.atlas;
    let width = horizon_client::hud::HudBatch::text_width(atlas, &text);
    let x = state.gui_size[0] - width - 6.0;
    batch.text(atlas, &text, x, 6.0, 0xFFFF_5555, true);
}

pub fn on_mods(mods: &mut Vec<crate::mod_api::NativeMod>) {
    mods.push(crate::mod_api::NativeMod {
        id: ID.to_owned(),
        title: "LetsCam".to_owned(),
        description: "Запись летсплеев: невидимая камера от третьего лица снимает игру по \
                      реплею, звук игры и микрофон — отдельными дорожками. Клавиша начинает и \
                      останавливает запись; ролик: /letscam render."
            .to_owned(),
        keys: vec![[
            KEY_TOGGLE.to_owned(),
            "Начать/остановить запись".to_owned(),
            KEY_TOGGLE_DEFAULT.to_owned(),
        ]],
    });
}
