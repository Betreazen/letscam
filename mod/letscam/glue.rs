// LetsCam's game side: F9 and `/letscam` start and stop a recording (the studio replay plus the
// crate's audio, mic, GUI snapshots and player track); `/letscam render` leaves the world, opens
// the replay standalone and drives it one video frame per drawn frame, the director's pose on
// the studio free camera, every frame read back through a marked screenshot request.
// Compiled as the child module `game::letscam_glue`, so it reads Game's private fields.
use super::Game;
use super::letscam_shot::{self as shot, FocusSpec};
use super::letscam_world as world;
use horizon_client::text::Text;
use horizon_client_render::letscam::{self as bridge, engine};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

pub const ID: &str = "letscam";
const KEY_TOGGLE: &str = "toggle";
const KEY_TOGGLE_DEFAULT: &str = "key.keyboard.f9";
const KEY_MENU: &str = "menu";
const KEY_MENU_DEFAULT: &str = "key.keyboard.f10";
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
    /// The studio recorder was seen stopped (and the player told) during this recording.
    recorder_stopped: bool,
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
    /// The time of day the recording began with (`world_time.json`), if it was saved.
    day_time: Option<i64>,
}

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

pub fn say(game: &mut Game<'_>, line: impl Into<String>) {
    let line = line.into();
    log::info!("letscam: {line}");
    game.system_message(Text::literal(format!("[LetsCam] {line}")));
}

pub fn config() -> engine::Config {
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

pub fn output_root(config: &engine::Config) -> PathBuf {
    if config.output_dir.trim().is_empty() {
        PathBuf::from(DEFAULT_OUTPUT)
    } else {
        PathBuf::from(config.output_dir.trim())
    }
}

/// `YYYY-MM-DD_HH-MM-SS` in the player's local time, for folder names that sort by time (chrono
/// with its clock is a dependency of horizon-client already).
fn stamp(now: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(now)
        .format("%Y-%m-%d_%H-%M-%S")
        .to_string()
}

pub fn is_recording() -> bool {
    STATE.with_borrow(|state| matches!(state.mode, Mode::Recording(_)))
}

pub fn is_rendering() -> bool {
    STATE.with_borrow(|state| matches!(state.mode, Mode::Rendering(_) | Mode::Leaving(..)))
}

// ---- recording ---------------------------------------------------------------------------

pub fn start_recording(game: &mut Game<'_>) {
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
    // The recorder's clock starts when it is made (recorder.rs `start_time`), before it writes the
    // snapshot of every loaded chunk at time 0, which can take seconds: taken after the call, the
    // track and the audio lagged the replay by that long. So the start is taken before it.
    let replay_start = Instant::now();
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
    world::save_world_time(game, &dir);
    world::save_registries(game, &dir);
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
            recorder_stopped: false,
        }));
    });
    let key = game.native_mod_key(ID, KEY_TOGGLE, KEY_TOGGLE_DEFAULT);
    say(
        game,
        format!("Запись началась: {}. Остановить: {key} или /letscam stop", dir.display()),
    );
}

pub fn stop_recording(game: &mut Game<'_>) {
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
    watch_recorder(game, now);
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

pub fn recordings(config: &engine::Config) -> Vec<PathBuf> {
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

/// One render's option words per angle: `orbit+side+top` in the words gives three renders of
/// the same recording, the other words shared (`fixed:x,y,z` keeps its commas, hence `+`).
fn expand_angles(words: &[&str]) -> Vec<Vec<String>> {
    let Some(at) = words.iter().position(|w| w.contains('+')) else {
        return vec![words.iter().map(|w| (*w).to_owned()).collect()];
    };
    words[at]
        .split('+')
        .filter(|angle| !angle.is_empty())
        .map(|angle| {
            let mut one: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
            one[at] = angle.to_owned();
            one
        })
        .collect()
}

thread_local! {
    /// Renders still to make of the current recording, each as its option words.
    static QUEUE: RefCell<Vec<Vec<String>>> = const { RefCell::new(Vec::new()) };
}

/// `/letscam render [папка] [ракурс[+ракурс…]] [x0.25] [sheet[=N]] [focus=...]`: the first word
/// is the recording's folder when such a folder exists, the rest are the crate's render options.
pub fn start_render(game: &mut Game<'_>, words: &[&str]) {
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
    let mut renders = expand_angles(option_words);
    // Every angle's words are checked before the first render starts.
    for one in &renders {
        let one: Vec<&str> = one.iter().map(String::as_str).collect();
        if let Err(error) = engine::RenderOptions::parse(&one) {
            say(game, error);
            return;
        }
    }
    let first = renders.remove(0);
    QUEUE.with_borrow_mut(|queue| *queue = renders);
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
        QUEUE.with_borrow_mut(Vec::clear);
        return;
    }
    start_job(game, &config, dir, &first);
}

/// Opens one render of `dir` with `words` and leaves the world for it (no-op on the title).
fn start_job(game: &mut Game<'_>, config: &engine::Config, dir: PathBuf, words: &[String]) {
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    let options = match engine::RenderOptions::parse(&words) {
        Ok(options) => options,
        Err(error) => {
            say(game, error);
            QUEUE.with_borrow_mut(Vec::clear);
            return;
        }
    };
    let focus = options.focus.as_deref().map(FocusSpec::parse);
    let job = match engine::RenderJob::open_with(&dir, config, options) {
        Ok(job) => job,
        Err(error) => {
            say(game, format!("Рендер не начался: {error}"));
            QUEUE.with_borrow_mut(Vec::clear);
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
    // The recorded registries go in at the replay's first advance, before any packet is decoded.
    world::queue_registries(&dir);
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
    let day_time = world::load_day_time(&dir);
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
            day_time,
        }));
    });
}

/// HEAD of `advance_studio_replay`: while rendering, the replay moves one video frame after each
/// captured frame and not at all otherwise, whatever the wall clock did.
pub fn before_replay_advance(game: &mut Game<'_>) {
    world::restore_pending(game);
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
    let (day_time, first_person) = STATE.with_borrow(|state| match &state.mode {
        Mode::Rendering(render) => (
            render.day_time,
            render.job.options().preset == engine::ShotPreset::FirstPerson,
        ),
        _ => (None, false),
    });
    world::apply_day_time(game, day_time);
    // The player's own body is drawn only while it is "in view": in a replay the camera type is
    // first person, so that hangs on the free camera alone. Third person behind keeps it drawn
    // whatever the free camera does; the replay's close puts the camera type back. The `fp`
    // angle is the opposite case: its camera sits in the head, so the body must stay hidden
    // (first person, and the free camera off below).
    game.camera_type = if first_person {
        super::perspective::CameraType::FirstPerson
    } else {
        super::perspective::CameraType::ThirdPersonBack
    };
    // The client's own physics still ticks in a standalone replay, on the wall clock and with
    // block collision (doors included), and moves the player between recorded positions, which
    // only come when the recorded player moved. Still velocity, and the physics' previous
    // position kept at the player, leave the recorded positions alone.
    if let Some(player) = game.player.as_mut() {
        player.velocity = Default::default();
        game.previous_position = player.position;
    }
    super::letscam_diag::log_frame(game);
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
                shot::apply_pose(game, pose, first_person);
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
                .and_then(|spec| shot::find_focus(game, spec, &mut locked));
            let pose = {
                let probe = shot::Probe(game);
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
                    shot::apply_pose(game, pose, first_person);
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
    let Some(render) = render else {
        QUEUE.with_borrow_mut(Vec::clear);
        return;
    };
    let Render { job, dir, hide_gui_before, frames, .. } = *render;
    let carry_on = outcome.is_ok();
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
    // The next angle of the same recording, straight from the title the replay left us on.
    let next = QUEUE.with_borrow_mut(|queue| {
        if !carry_on || queue.is_empty() {
            queue.clear();
            None
        } else {
            Some(queue.remove(0))
        }
    });
    if let Some(words) = next {
        start_job(game, &config(), dir, &words);
    }
}

// ---- hooks -------------------------------------------------------------------------------

pub fn on_frame(game: &mut Game<'_>) {
    run_screen_action(game);
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
    } else if key == game.native_mod_key(ID, KEY_MENU, KEY_MENU_DEFAULT) {
        super::letscam_screen::open(game);
    }
}

/// A click in the window, carried out here where the game is writable.
fn run_screen_action(game: &mut Game<'_>) {
    use super::letscam_screen::Action;
    let Some(action) = super::letscam_screen::take_action() else { return };
    match action {
        Action::ToggleRecording => {
            game.native_mod_screen_close(ID);
            if is_recording() {
                stop_recording(game);
            } else {
                start_recording(game);
            }
        }
        Action::Render { folder, angles } => {
            game.native_mod_screen_close(ID);
            let mut words = vec![folder.as_str()];
            words.extend(angles);
            start_render(game, &words);
        }
        Action::OpenFolder(dir) => {
            if let Err(error) = std::process::Command::new("explorer").arg(&dir).spawn() {
                say(game, format!("Папка {} не открылась: {error}", dir.display()));
            }
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
        None | Some("menu") => super::letscam_screen::open(game),
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
            "/letscam (окно, F10) | start | stop | list | cancel | reload | render [папка] [auto|fp|shoulder|orbit|side|front|top|fixed:x,y,z] [x0.25] [sheet[=тиков]] [focus=id|тип|ride]",
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
                      останавливает запись, вторая (F10) открывает окно записей с кнопками рендера."
            .to_owned(),
        keys: vec![
            [
                KEY_TOGGLE.to_owned(),
                "Начать/остановить запись".to_owned(),
                KEY_TOGGLE_DEFAULT.to_owned(),
            ],
            [KEY_MENU.to_owned(), "Окно записей".to_owned(), KEY_MENU_DEFAULT.to_owned()],
        ],
    });
}

/// A mod payload the server just sent, written into the studio recording the studio's own filter
/// leaves it out of (see horizon-mixin.toml, `mod_net_recorded`). Only while LetsCam records a
/// live world: in a replay the same functions run for the payloads read back.
pub fn record_payload(game: &Game<'_>, channel: &str, data: &[u8]) {
    if !is_recording() || game.studio_replay.is_some() {
        return;
    }
    let Some(remote) = game.remote.as_ref() else { return };
    if !remote.session.is_recording() {
        return;
    }
    let packet = horizon_protocol::packets::play::Clientbound::CustomPayload(
        horizon_protocol::packets::configuration::CustomPayload {
            channel: channel.to_owned(),
            data: data.to_vec(),
        },
    );
    remote.session.record_clientbound(&packet);
}

/// The studio recorder stops on its own when its queue overflows or its file write fails, and
/// says so only in its status: checked every frame while recording, told once in chat with the
/// time and the reason, since the replay (and so the video) ends there.
fn watch_recorder(game: &mut Game<'_>, now: Instant) {
    let Some(status) = game.remote.as_ref().and_then(|r| r.session.recorder_status()) else {
        return;
    };
    if status.active {
        return;
    }
    let at = STATE.with_borrow_mut(|state| match &mut state.mode {
        Mode::Recording(rec) if !rec.recorder_stopped => {
            rec.recorder_stopped = true;
            Some(now.saturating_duration_since(rec.started).as_secs())
        }
        _ => None,
    });
    if let Some(secs) = at {
        let why = status.error.unwrap_or_else(|| "без сообщения об ошибке".into());
        say(
            game,
            format!(
                "Реплей перестал записываться на {}:{:02} ({why}, пакетов {}): ролик закончится там. Остановите запись и начните новую.",
                secs / 60,
                secs % 60,
                status.packet_count
            ),
        );
    }
}

/// How long the running recording has gone, None when none runs (for the window).
pub fn recording_elapsed() -> Option<Duration> {
    STATE.with_borrow(|state| match &state.mode {
        Mode::Recording(rec) => Some(Instant::now().saturating_duration_since(rec.started)),
        _ => None,
    })
}
