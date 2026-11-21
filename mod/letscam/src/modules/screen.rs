// LetsCam's window: start or stop a recording and render the recent ones with one click.
// Opened with its key (F10 by default) or a bare `/letscam`; a mod screen of the game
// (`mod_screen_open`), drawn by `screen_render_post` and clicked through
// `screen_mouse_button_pressed_pre`. Those hooks see the game read-only, so a click is queued
// and carried out on the next frame (`take_action`, from the glue's frame hook).
// Compiled as the child module `game::letscam_screen`.
use super::Game;
use super::letscam_glue as glue;
use mod_api::ScreenView;
use horizon_client::hud::HudBatch;
use std::cell::RefCell;
use std::path::PathBuf;

const LEFT_BUTTON: i32 = 0;
const WIDTH: f32 = 460.0;
/// The window's footer, longest first: the first that fits the panel is drawn.
const HINTS: [&str; 3] = [
    "Ролик: камера-режиссёр. 4 ракурса: авто, сбоку, облёт, сверху. Ключи: свой путь камеры.",
    "Ролик: режиссёр · 4 ракурса: авто, сбоку, облёт, сверху · Ключи: свой путь",
    "4 ракурса: авто, сбоку, облёт, сверху · Ключи: свой путь",
];
const ROW_HEIGHT: f32 = 22.0;
const MAX_ROWS: usize = 8;
const PANEL: u32 = 0xE0181820;
const BUTTON: u32 = 0xFF3A3A46;
const BUTTON_HOVER: u32 = 0xFF5A5A70;
const RECORD: u32 = 0xFF8A2A2A;
// Text is ARGB for `native_text` (vanilla's `GuiGraphics`): without the alpha byte it draws nothing.
const WHITE: u32 = 0xFFFF_FFFF;
const GREY: u32 = 0xFFA0_A0A0;
const GOLD: u32 = 0xFFFF_D060;
const REC_TEXT: u32 = 0xFFFF_5555;
/// Four angles of one recording in a row, each with its contact sheet.
const FOUR_ANGLES: &[&str] = &["auto+side+orbit+top", "sheet"];

/// What a click asked for, done on the next frame with the game writable.
#[derive(Clone, Debug)]
pub enum Action {
    ToggleRecording,
    /// `/letscam render <folder> <words…>`.
    Render { folder: String, words: &'static [&'static str] },
    /// `/letscam edit <folder>`: the key editor.
    Edit { folder: String },
    OpenFolder(PathBuf),
}

/// One recording as the window lists it.
#[derive(Clone, Debug)]
struct Entry {
    dir: PathBuf,
    name: String,
    seconds: u64,
    rendered: bool,
    /// Keyframes of the camera path the key editor saved.
    keys: usize,
}

thread_local! {
    static ENTRIES: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static ACTION: RefCell<Option<Action>> = const { RefCell::new(None) };
    /// The window logged where its buttons are since it opened (for the bench bot's `click-at`).
    static LOGGED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
    let keys = super::letscam_edit::saved_keys(&dir);
    Entry { dir, name, seconds, rendered, keys }
}

/// Opens the window, reading the recording list once (not every frame).
pub fn open(game: &mut Game<'_>) {
    let config = glue::config();
    let mut dirs = glue::recordings(&config);
    dirs.reverse();
    let entries: Vec<Entry> = dirs.into_iter().take(MAX_ROWS).map(entry).collect();
    ENTRIES.with_borrow_mut(|list| *list = entries);
    LOGGED.set(false);
    if !game.mod_screen_open(glue::ID) {
        glue::say(game, "Окно LetsCam открывается только в мире, без другого открытого окна.");
    }
}

pub fn take_action() -> Option<Action> {
    ACTION.with_borrow_mut(Option::take)
}

fn is_ours(game: &Game<'_>, screen: &ScreenView<'_>) -> bool {
    screen.id == "mod" && game.mod_screen() == Some(glue::ID)
}

fn inside(rect: [f32; 4], at: [f32; 2]) -> bool {
    at[0] >= rect[0] && at[0] <= rect[0] + rect[2] && at[1] >= rect[1] && at[1] <= rect[1] + rect[3]
}

/// Every clickable rectangle of the window with what it does, in GUI pixels.
fn buttons(screen: &ScreenView<'_>, entries: &[Entry]) -> Vec<([f32; 4], Action, String)> {
    let [x, y] = panel_origin(screen, entries.len());
    let mut out = Vec::new();
    let record = if glue::is_recording() { "Остановить запись" } else { "Начать запись" };
    out.push(([x + 10.0, y + 22.0, WIDTH - 20.0, 18.0], Action::ToggleRecording, record.to_owned()));
    for (i, e) in entries.iter().enumerate() {
        let row_y = y + 52.0 + i as f32 * ROW_HEIGHT;
        let render = |words: &'static [&'static str]| Action::Render { folder: e.name.clone(), words };
        out.push(([x + 178.0, row_y, 46.0, 16.0], render(&[]), "Ролик".into()));
        out.push(([x + 228.0, row_y, 62.0, 16.0], render(FOUR_ANGLES), "4 ракурса".into()));
        let edit = if e.keys > 0 { format!("Ключи {}", e.keys) } else { "Ключи".to_owned() };
        out.push(([x + 294.0, row_y, 48.0, 16.0], Action::Edit { folder: e.name.clone() }, edit));
        // A path needs two keyframes before there is anything to follow.
        if e.keys >= 2 {
            out.push(([x + 346.0, row_y, 62.0, 16.0], render(&["keys"]), "По ключам".into()));
        }
        out.push(([x + 412.0, row_y, 38.0, 16.0], Action::OpenFolder(e.dir.clone()), "Папка".into()));
    }
    out
}

fn panel_origin(screen: &ScreenView<'_>, rows: usize) -> [f32; 2] {
    let [sx, sy, sw, sh] = screen.rect;
    let height = panel_height(rows);
    [sx + (sw - WIDTH) / 2.0, sy + ((sh - height) / 2.0).max(4.0)]
}

fn panel_height(rows: usize) -> f32 {
    60.0 + rows.max(1) as f32 * ROW_HEIGHT + 18.0
}

/// From the HEAD of `draw_screen_with_post_hook`, after the open screen drew itself: the game
/// builds no `ScreenView` for `native_screen_render_post` unless a mod subscribes to a screen
/// render hook through the trait API, which a mixin-only mod never does, so the view is built
/// here, only while LetsCam's own window is open.
pub fn draw_open(game: &Game<'_>, batch: &mut HudBatch) {
    if game.mod_screen() != Some(glue::ID) {
        return;
    }
    let view = game.screen_view(batch, game.screen);
    draw(game, batch, &view);
}

pub fn draw(game: &Game<'_>, batch: &mut HudBatch, screen: &ScreenView<'_>) {
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
            game.native_text(batch, &line, [x + WIDTH - 70.0, y + 7.0], REC_TEXT, true);
        }
        if !LOGGED.replace(true) {
            let centre = |r: [f32; 4]| (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
            let rects: Vec<(String, (f32, f32))> =
                buttons(screen, entries).into_iter().map(|(r, _, label)| (label, centre(r))).collect();
            log::info!("letscam: window open, buttons {rects:?}");
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
        // The longest hint that fits the panel (the full one ran past its right edge).
        let hint = HINTS
            .iter()
            .copied()
            .find(|h| game.native_text_width(batch, h) <= WIDTH - 20.0)
            .unwrap_or(HINTS[HINTS.len() - 1]);
        let footer_y = y + panel_height(entries.len()) - 14.0;
        game.native_text(batch, hint, [x + 10.0, footer_y], GREY, true);
    });
}

pub fn on_press(game: &Game<'_>, screen: &ScreenView<'_>, button: i32, canceled: &mut bool) {
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
