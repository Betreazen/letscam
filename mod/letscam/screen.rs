// LetsCam's window: start or stop a recording and render the recent ones with one click.
// Opened with its key (F10 by default) or a bare `/letscam`; a mod screen of the game
// (`native_mod_screen_open`), drawn by `screen_render_post` and clicked through
// `screen_mouse_button_pressed_pre`. Those hooks see the game read-only, so a click is queued
// and carried out on the next frame (`take_action`, from the glue's frame hook).
// Compiled as the child module `game::letscam_screen`.
use super::Game;
use super::letscam_glue as glue;
use crate::game::native_hooks::NativeScreenView;
use horizon_client::hud::HudBatch;
use std::cell::RefCell;
use std::path::PathBuf;

const LEFT_BUTTON: i32 = 0;
const WIDTH: f32 = 340.0;
const ROW_HEIGHT: f32 = 22.0;
const MAX_ROWS: usize = 8;
const PANEL: u32 = 0xE0181820;
const BUTTON: u32 = 0xFF3A3A46;
const BUTTON_HOVER: u32 = 0xFF5A5A70;
const RECORD: u32 = 0xFF8A2A2A;
const WHITE: u32 = 0xFFFFFF;
const GREY: u32 = 0xA0A0A0;
const GOLD: u32 = 0xFFD060;
/// Four angles of one recording in a row, each with its contact sheet.
const ANGLES: &str = "auto+side+orbit+top";

/// What a click asked for, done on the next frame with the game writable.
#[derive(Clone, Debug)]
pub enum Action {
    ToggleRecording,
    Render { folder: String, angles: Option<&'static str> },
    OpenFolder(PathBuf),
}

/// One recording as the window lists it.
#[derive(Clone, Debug)]
struct Entry {
    dir: PathBuf,
    name: String,
    seconds: u64,
    rendered: bool,
}

thread_local! {
    static ENTRIES: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static ACTION: RefCell<Option<Action>> = const { RefCell::new(None) };
}

fn entry(dir: PathBuf) -> Entry {
    let name = dir.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let seconds = std::fs::read_to_string(dir.join("session.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v["duration_ms"].as_u64())
        .map_or(0, |ms| ms / 1000);
    let rendered = std::fs::read_dir(&dir).is_ok_and(|mut files| {
        files.any(|f| {
            f.is_ok_and(|f| matches!(f.path().extension().and_then(|x| x.to_str()), Some("mp4" | "mkv")))
        })
    });
    Entry { dir, name, seconds, rendered }
}

/// Opens the window, reading the recording list once (not every frame).
pub fn open(game: &mut Game<'_>) {
    let config = glue::config();
    let mut dirs = glue::recordings(&config);
    dirs.reverse();
    let entries: Vec<Entry> = dirs.into_iter().take(MAX_ROWS).map(entry).collect();
    ENTRIES.with_borrow_mut(|list| *list = entries);
    if !game.native_mod_screen_open(glue::ID) {
        glue::say(game, "Окно LetsCam открывается только в мире, без другого открытого окна.");
    }
}

pub fn take_action() -> Option<Action> {
    ACTION.with_borrow_mut(Option::take)
}

fn is_ours(game: &Game<'_>, screen: &NativeScreenView<'_>) -> bool {
    screen.id == "mod" && game.native_mod_screen() == Some(glue::ID)
}

fn inside(rect: [f32; 4], at: [f32; 2]) -> bool {
    at[0] >= rect[0] && at[0] <= rect[0] + rect[2] && at[1] >= rect[1] && at[1] <= rect[1] + rect[3]
}

/// Every clickable rectangle of the window with what it does, in GUI pixels.
fn buttons(screen: &NativeScreenView<'_>, entries: &[Entry]) -> Vec<([f32; 4], Action, String)> {
    let [x, y] = panel_origin(screen, entries.len());
    let mut out = Vec::new();
    let record = if glue::is_recording() { "Остановить запись" } else { "Начать запись" };
    out.push(([x + 10.0, y + 22.0, WIDTH - 20.0, 18.0], Action::ToggleRecording, record.to_owned()));
    for (i, e) in entries.iter().enumerate() {
        let row_y = y + 52.0 + i as f32 * ROW_HEIGHT;
        let folder = e.name.clone();
        out.push((
            [x + 178.0, row_y, 46.0, 16.0],
            Action::Render { folder: folder.clone(), angles: None },
            "Ролик".into(),
        ));
        out.push((
            [x + 228.0, row_y, 62.0, 16.0],
            Action::Render { folder, angles: Some(ANGLES) },
            "4 ракурса".into(),
        ));
        out.push(([x + 294.0, row_y, 36.0, 16.0], Action::OpenFolder(e.dir.clone()), "Папка".into()));
    }
    out
}

fn panel_origin(screen: &NativeScreenView<'_>, rows: usize) -> [f32; 2] {
    let [sx, sy, sw, sh] = screen.rect;
    let height = panel_height(rows);
    [sx + (sw - WIDTH) / 2.0, sy + ((sh - height) / 2.0).max(4.0)]
}

fn panel_height(rows: usize) -> f32 {
    60.0 + rows.max(1) as f32 * ROW_HEIGHT + 18.0
}

pub fn draw(game: &Game<'_>, batch: &mut HudBatch, screen: &NativeScreenView<'_>) {
    if !is_ours(game, screen) {
        return;
    }
    ENTRIES.with_borrow(|entries| {
        let [x, y] = panel_origin(screen, entries.len());
        game.native_fill(batch, [x, y, WIDTH, panel_height(entries.len())], PANEL);
        game.native_text(batch, "LetsCam — записи летсплея", [x + 10.0, y + 7.0], GOLD, true);
        if let Some(elapsed) = glue::recording_elapsed() {
            let secs = elapsed.as_secs();
            let line = format!("● REC {}:{:02}", secs / 60, secs % 60);
            game.native_text(batch, &line, [x + WIDTH - 70.0, y + 7.0], 0xFF5555, true);
        }
        for (rect, action, label) in buttons(screen, entries) {
            let hover = inside(rect, screen.mouse);
            let colour = match (&action, hover) {
                (Action::ToggleRecording, false) if glue::is_recording() => RECORD,
                (_, true) => BUTTON_HOVER,
                _ => BUTTON,
            };
            game.native_fill(batch, rect, colour);
            game.native_text(batch, &label, [rect[0] + 4.0, rect[1] + 4.0], WHITE, true);
        }
        if entries.is_empty() {
            game.native_text(batch, "Записей пока нет: F9 или кнопка выше.", [x + 10.0, y + 52.0], GREY, true);
        }
        for (i, e) in entries.iter().enumerate() {
            let row_y = y + 52.0 + i as f32 * ROW_HEIGHT;
            let mark = if e.rendered { " ✓" } else { "" };
            let line = format!("{} · {}:{:02}{mark}", e.name, e.seconds / 60, e.seconds % 60);
            game.native_text(batch, &line, [x + 10.0, row_y + 4.0], WHITE, true);
        }
        let hint = "Ролик: камера-режиссёр. 4 ракурса: авто, сбоку, облёт, сверху.";
        let footer_y = y + panel_height(entries.len()) - 14.0;
        game.native_text(batch, hint, [x + 10.0, footer_y], GREY, true);
    });
}

pub fn on_press(game: &Game<'_>, screen: &NativeScreenView<'_>, button: i32, canceled: &mut bool) {
    if !is_ours(game, screen) || button != LEFT_BUTTON {
        return;
    }
    *canceled = true;
    let hit = ENTRIES.with_borrow(|entries| {
        buttons(screen, entries)
            .into_iter()
            .find(|(rect, ..)| inside(*rect, screen.mouse))
            .map(|(_, action, _)| action)
    });
    if let Some(action) = hit {
        ACTION.with_borrow_mut(|slot| *slot = Some(action));
    }
}
