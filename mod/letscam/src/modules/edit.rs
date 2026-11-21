// LetsCam's key editor: `/letscam edit [папка]` (or «Ключи» in the window) leaves the world and
// opens the recording's replay paused, with the studio free camera above the player. The player
// flies the camera and keys the path (N), and the render's `keys` angle follows it. The path is
// Horizon's own studio track, which the replay archive keeps: the game saves it on close.
// Compiled as the child module `game::letscam_edit`.
use super::Game;
use super::letscam_glue as glue;
use super::letscam_world as world;
use horizon_client::hud::HudBatch;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// The editor's own record of how many keyframes the path has, read by the window.
pub const KEYS_FILE: &str = "keys.json";
const KEY_ADD: &str = "key.keyboard.n";
const KEY_REMOVE: &str = "key.keyboard.delete";
const KEY_PAUSE: &str = "key.keyboard.k";
const KEY_BACK: &str = "key.keyboard.left";
const KEY_FORWARD: &str = "key.keyboard.right";
const KEY_PREVIEW: &str = "key.keyboard.p";
const KEY_DONE: &str = "key.keyboard.enter";
const SEEK_STEP_MS: u32 = 1000;
/// A keyframe this close to the replay's time is the one Delete removes.
const REMOVE_WINDOW_MS: u32 = 750;
/// Where the free camera starts: behind and above the recorded player's eye.
const START_BACK: f64 = 4.0;
const START_UP: f64 = 1.5;
const START_FOV: f32 = 70.0;
const HELP: &str =
    "N — ключ · Del — убрать · K — пауза · ←/→ — 1 с · P — просмотр пути · Enter — сохранить";

enum Edit {
    /// Waiting for the world to close before the replay opens.
    Leaving(PathBuf),
    Open { dir: PathBuf, day_time: Option<i64>, camera_placed: bool },
}

thread_local! {
    static EDIT: RefCell<Option<Edit>> = const { RefCell::new(None) };
}

pub fn is_active() -> bool {
    EDIT.with_borrow(Option::is_some)
}

/// `/letscam edit [папка]`: the folder, or the newest recording.
pub fn start(game: &mut Game<'_>, folder: Option<&str>) {
    if !glue::is_idle() || is_active() {
        say(game, "Сначала остановите запись или дождитесь конца рендера.");
        return;
    }
    let config = glue::config();
    let dir = match folder {
        Some(name) => glue::output_root(&config).join(name),
        None => match glue::recordings(&config).pop() {
            Some(dir) => dir,
            None => {
                say(game, "Записей для правки ключей нет.");
                return;
            }
        },
    };
    if !dir.join(glue::REPLAY_FILE).is_file() {
        say(game, format!("В {} нет {}.", dir.display(), glue::REPLAY_FILE));
        return;
    }
    say(game, format!("Правка ключей {}: выхожу из мира.", dir.display()));
    EDIT.with_borrow_mut(|edit| *edit = Some(Edit::Leaving(dir)));
    if game.remote.is_some() {
        if let Err(error) = game.leave_server(None) {
            log::warn!("letscam: leaving the world: {error}");
        }
    }
}

fn say(game: &mut Game<'_>, line: impl Into<String>) {
    glue::say(game, line);
}

fn open(game: &mut Game<'_>, dir: PathBuf) {
    let path = dir.join(glue::REPLAY_FILE);
    world::queue_registries(&dir);
    let opened = game.open_studio_replay(&path.to_string_lossy());
    if !matches!(&opened, Ok(v) if v["ok"].as_bool() == Some(true)) {
        let why = match opened {
            Ok(v) => v["error"].as_str().unwrap_or("неизвестная ошибка").to_owned(),
            Err(e) => e.to_string(),
        };
        EDIT.with_borrow_mut(|edit| *edit = None);
        say(game, format!("Реплей не открылся: {why}"));
        return;
    }
    if let Some(driver) = game.replay.driver.as_mut() {
        driver.set_paused(true);
    }
    game.studio.freecam.locked_to_path = false;
    let keys = game.studio.freecam.track.len();
    let day_time = world::load_day_time(&dir);
    log::info!("letscam: key editor open, {keys} keyframes");
    say(game, format!("Правка ключей: ключей {keys}. {HELP}"));
    EDIT.with_borrow_mut(|edit| *edit = Some(Edit::Open { dir, day_time, camera_placed: false }));
}

/// The free camera over the recorded player's shoulder, once the replay shows the player.
fn place_camera(game: &mut Game<'_>) -> bool {
    let Some(eye) = game.eye_position() else { return false };
    let yaw = game.yaw.to_radians();
    let forward = [-f64::from(yaw.sin()), f64::from(yaw.cos())];
    let pos = glam::DVec3::new(
        eye[0] - forward[0] * START_BACK,
        eye[1] + START_UP,
        eye[2] - forward[1] * START_BACK,
    );
    game.studio.freecam.activate(pos, game.yaw, 15.0, START_FOV);
    true
}

pub fn on_frame(game: &mut Game<'_>) {
    let leaving = EDIT.with_borrow(|edit| match edit {
        Some(Edit::Leaving(dir)) => Some(dir.clone()),
        _ => None,
    });
    if let Some(dir) = leaving {
        if game.remote.is_none() && game.integrated.is_none() {
            open(game, dir);
        }
        return;
    }
    let Some((day_time, placed)) = EDIT.with_borrow(|edit| match edit {
        Some(Edit::Open { day_time, camera_placed, .. }) => Some((*day_time, *camera_placed)),
        _ => None,
    }) else {
        return;
    };
    if game.replay.driver.is_none() {
        // Closed some other way (`/studio replay close`): the game saved the path already.
        finish(game, false);
        return;
    }
    world::apply_day_time(game, day_time);
    world::pin_replay_player(game, false);
    if !placed && place_camera(game) {
        EDIT.with_borrow_mut(|edit| {
            if let Some(Edit::Open { camera_placed, .. }) = edit {
                *camera_placed = true;
            }
        });
    }
}

fn replay_ms(game: &Game<'_>) -> u32 {
    game.replay.driver.as_ref().map_or(0, |d| d.position_ms())
}

fn seek(game: &mut Game<'_>, dir: &Path, forward: bool) {
    let Some(driver) = game.replay.driver.as_ref() else { return };
    let now = driver.position_ms();
    let target = if forward {
        now.saturating_add(SEEK_STEP_MS).min(driver.duration_ms())
    } else {
        now.saturating_sub(SEEK_STEP_MS)
    };
    // A standalone seek rebuilds the replay's remote, and with it the empty biome map: the
    // recorded registries go back in at the next advance.
    world::queue_registries(dir);
    game.studio_command(&format!("studio replay seek {target}"));
}

/// The editor's keys; true when the key was one of them (and so is used up).
pub fn on_key(game: &mut Game<'_>, key: &str) -> bool {
    let Some(dir) = EDIT.with_borrow(|edit| match edit {
        Some(Edit::Open { dir, .. }) => Some(dir.clone()),
        _ => None,
    }) else {
        return false;
    };
    match key {
        KEY_ADD => {
            if !game.studio.freecam.active {
                say(game, "Свободная камера выключена: ключ ставится с неё.");
                return true;
            }
            let t = replay_ms(game);
            game.studio.freecam.add_keyframe_at_current(t);
            game.studio.camera_track_dirty = true;
            log::info!("letscam: keyframe at {t} ms ({} in all)", game.studio.freecam.track.len());
        }
        KEY_REMOVE => {
            let t = replay_ms(game);
            if game.studio.freecam.track.remove_near(t, REMOVE_WINDOW_MS) {
                game.studio.camera_track_dirty = true;
            }
        }
        KEY_PAUSE => {
            if let Some(driver) = game.replay.driver.as_mut() {
                let paused = driver.is_paused();
                driver.set_paused(!paused);
            }
        }
        KEY_BACK | KEY_FORWARD => seek(game, &dir, key == KEY_FORWARD),
        KEY_PREVIEW => {
            let cam = &mut game.studio.freecam;
            cam.locked_to_path = !cam.locked_to_path && cam.track.len() >= 2;
        }
        KEY_DONE => finish(game, true),
        _ => return false,
    }
    true
}

/// Saves the path (the game writes it into the replay as it closes) and notes its keyframes.
fn finish(game: &mut Game<'_>, close: bool) {
    let Some(Edit::Open { dir, .. }) = EDIT.with_borrow_mut(Option::take) else { return };
    let keys = game.studio.freecam.track.len();
    game.studio.freecam.locked_to_path = false;
    if close {
        game.close_studio_replay();
    }
    let note = serde_json::json!({ "keyframes": keys });
    if let Err(error) = std::fs::write(dir.join(KEYS_FILE), note.to_string()) {
        log::warn!("letscam: {KEYS_FILE}: {error}");
    }
    log::info!("letscam: key editor closed, {keys} keyframes");
    let line = if keys >= 2 {
        format!("Ключей {keys}: путь сохранён. Ролик по ним: /letscam render keys или «По ключам» в окне (F10).")
    } else {
        format!("Ключей {keys}: для пути нужно хотя бы два.")
    };
    say(game, line);
}

/// The keyframes a recording's path has, as the editor last saved them.
pub fn saved_keys(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join(KEYS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v["keyframes"].as_u64())
        .map_or(0, |n| n as usize)
}

/// The editor's line at the top of the screen: replay time, keyframes, pause and the keys.
pub fn on_hud(game: &Game<'_>, batch: &mut HudBatch) {
    if !EDIT.with_borrow(|edit| matches!(edit, Some(Edit::Open { .. }))) {
        return;
    }
    let Some(driver) = game.replay.driver.as_ref() else { return };
    let clock = |ms: u32| format!("{}:{:02}.{}", ms / 60_000, ms / 1000 % 60, ms / 100 % 10);
    let pause = if driver.is_paused() { "⏸" } else { "▶" };
    let preview = if game.studio.freecam.locked_to_path { " · просмотр пути" } else { "" };
    let line = format!(
        "LetsCam · ключи {pause} {} / {} · ключей {}{preview}",
        clock(driver.position_ms()),
        clock(driver.duration_ms()),
        game.studio.freecam.track.len()
    );
    let atlas = &game.gui.atlas;
    batch.text(atlas, &line, 6.0, 6.0, 0xFFFF_D060, true);
    batch.text(atlas, HELP, 6.0, 18.0, 0xFFE0_E0E0, true);
}
