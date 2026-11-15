// The contact sheets' line of the mobs' attack phases: SoulsCraft's `soulscraft:state` mod
// messages, which LetsCam writes into the recording (`record_payload`), come back as the replay
// plays; each frame's line names the mobs by their entity type. Compiled as the child module
// `game::letscam_labels`.
use super::Game;
use horizon_client_render::letscam::engine::mob_labels::{self, MobLabels};

/// A mod message the replay just played at `at_ms`: SoulsCraft's mob state goes into `mobs`.
pub fn take_payload(data: &[u8], at_ms: u32, mobs: &mut MobLabels) {
    let Ok(message) = horizon_protocol::mod_net::decode(data) else { return };
    if message.channel != mob_labels::STATE_CHANNEL {
        return;
    }
    match mob_labels::decode(&message.data) {
        Some(batch) => mobs.apply(batch, at_ms),
        None => log::warn!("letscam: a {} message LetsCam can't read", mob_labels::STATE_CHANNEL),
    }
}

/// The line for the frame at replay time `t_ms`; empty without SoulsCraft mobs at work.
pub fn line(game: &Game<'_>, mobs: &MobLabels, t_ms: u32) -> String {
    if mobs.is_empty() {
        return String::new();
    }
    let entities = game.native_client_entities();
    mobs.line(t_ms, |id| {
        entities
            .iter()
            .find(|e| e.id == id && e.alive)
            .map(|e| e.kind.rsplit(':').next().unwrap_or(&e.kind).to_owned())
    })
}
