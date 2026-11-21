// The contact sheets' line of the mobs' attack phases: SoulsCraft's `soulscraft:state` payloads,
// which LetsCam writes into the recording as they arrive (`glue::live_payload`), come back as the
// replay plays (`glue::replay_payload`); each frame's line names the mobs by their entity type.
// Compiled as the child module `game::letscam_labels`.
use super::Game;
use letscam_engine::mob_labels::{self, MobLabels};

/// A `soulscraft:state` payload the replay just played at `at_ms`, into `mobs`.
pub fn take_payload(data: &[u8], at_ms: u32, mobs: &mut MobLabels) {
    match mob_labels::decode(data) {
        Some(batch) => mobs.apply(batch, at_ms),
        None => log::warn!("letscam: a {} message LetsCam can't read", mob_labels::STATE_CHANNEL),
    }
}

/// The line for the frame at replay time `t_ms`; empty without SoulsCraft mobs at work.
pub fn line(game: &Game<'_>, mobs: &MobLabels, t_ms: u32) -> String {
    if mobs.is_empty() {
        return String::new();
    }
    let entities = super::letscam_shot::entities(game);
    mobs.line(t_ms, |id| {
        entities
            .iter()
            .find(|e| e.id == id && e.alive)
            .map(|e| e.kind.rsplit(':').next().unwrap_or(&e.kind).to_owned())
    })
}
