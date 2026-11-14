// What a standalone replay lacks from the live session, saved when a recording starts and put
// back while it renders: the time of day and the registry order (biomes, world clock).
// Compiled as the child module `game::letscam_world`.
use super::Game;
use std::cell::RefCell;
use std::path::Path;

/// The world's time of day when the recording began. A standalone replay builds its registries
/// from the game's own data, so the recorded SetTime packets find no `minecraft:world_clock` and
/// the sky stays at night: the render sets the time from this file instead.
pub(super) const WORLD_TIME_FILE: &str = "world_time.json";
/// Game ticks per millisecond of replay (20 a second).
const TICKS_PER_MS: f64 = 0.02;

pub(super) fn save_world_time(game: &Game<'_>, dir: &Path) {
    let value = serde_json::json!({
        "day_time": game.world.day_time,
        "game_time": game.world.game_time,
    });
    if let Err(error) = std::fs::write(dir.join(WORLD_TIME_FILE), value.to_string()) {
        log::warn!("letscam: {WORLD_TIME_FILE}: {error}");
    }
}

pub(super) fn load_day_time(dir: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(dir.join(WORLD_TIME_FILE)).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?["day_time"].as_i64()
}

/// The recorded time of day at the replay's current position.
pub(super) fn apply_day_time(game: &mut Game<'_>, start: Option<i64>) {
    let (Some(start), Some(driver)) = (start, game.studio_replay.as_ref()) else {
        return;
    };
    let ticks = (f64::from(driver.position_ms()) * TICKS_PER_MS) as i64;
    game.world.day_time = start + ticks;
}

/// The live session's registry order that a standalone replay lacks: the configuration phase's
/// RegistryData never reaches the .mcpr, so `init_remote_for_replay` leaves the chunk biome map
/// empty and every biome decodes as built-in biome 0 (badlands: brown grass), and the world clock
/// in SetTime finds no name. Saved when the recording starts, put back before the first packet.
const REGISTRIES_FILE: &str = "registries.json";
const BIOME_REGISTRY: &str = "minecraft:worldgen/biome";
const CLOCK_REGISTRY: &str = "minecraft:world_clock";

thread_local! {
    /// Registries to put into the replay's remote before its first packet is dispatched.
    static PENDING_REGISTRIES: RefCell<Option<(Vec<String>, Vec<String>)>> = const { RefCell::new(None) };
}

pub(super) fn save_registries(game: &Game<'_>, dir: &Path) {
    let Some(remote) = game.remote.as_ref() else { return };
    let names = |key: &str| -> Vec<String> {
        remote
            .registries
            .registries
            .get(key)
            .map(|list| list.iter().map(|(name, _)| name.clone()).collect())
            .unwrap_or_default()
    };
    let value = serde_json::json!({ "biomes": names(BIOME_REGISTRY), "clocks": names(CLOCK_REGISTRY) });
    if let Err(error) = std::fs::write(dir.join(REGISTRIES_FILE), value.to_string()) {
        log::warn!("letscam: {REGISTRIES_FILE}: {error}");
    }
}

fn load_registries(dir: &Path) -> Option<(Vec<String>, Vec<String>)> {
    let text = std::fs::read_to_string(dir.join(REGISTRIES_FILE)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let list = |key: &str| -> Vec<String> {
        value[key]
            .as_array()
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    };
    Some((list("biomes"), list("clocks")))
}

/// The recorded biome order as the replay's chunk biome map (what `set_registries` does on a
/// live join) and both registries into `remote.registries`.
fn inject_registries(game: &mut Game<'_>, biomes: Vec<String>, clocks: Vec<String>) {
    if biomes.is_empty() {
        return;
    }
    let ids: Vec<u16> = biomes.iter().map(|name| world_biomes!(game).id(name)).collect();
    let Some(remote) = game.remote.as_mut() else { return };
    let layout = remote.chunk_format.layout();
    let bits = horizon_protocol::registry::ceil_log2(biomes.len());
    remote.chunk_format = std::sync::Arc::new(
        super::remote_chunks::ChunkFormat::new(&remote.dimension, ids, bits).in_column(layout),
    );
    let mut registries = (*remote.registries).clone();
    let count = biomes.len();
    registries
        .registries
        .insert(BIOME_REGISTRY.to_owned(), biomes.into_iter().map(|n| (n, None)).collect());
    if !clocks.is_empty() {
        registries
            .registries
            .insert(CLOCK_REGISTRY.to_owned(), clocks.into_iter().map(|n| (n, None)).collect());
    }
    remote.registries = std::sync::Arc::new(registries);
    log::info!("letscam: replay registries restored ({count} biomes)");
}

/// Reads the recording's registries, to be put into the replay before its first packet.
pub(super) fn queue_registries(dir: &Path) {
    PENDING_REGISTRIES.with_borrow_mut(|pending| *pending = load_registries(dir));
}

/// Puts the queued registries into the replay's remote, once (HEAD of the replay's advance).
pub(super) fn restore_pending(game: &mut Game<'_>) {
    if let Some((biomes, clocks)) = PENDING_REGISTRIES.with_borrow_mut(Option::take) {
        inject_registries(game, biomes, clocks);
    }
}
