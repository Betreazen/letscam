// The contact sheets' line of the mobs' attack phases: SoulsCraft's `soulscraft:state` mod
// messages, which LetsCam writes into the recording (`record_payload`), come back as the replay
// plays; each frame's line names the mobs by their entity type. Compiled as the child module
// `game::letscam_labels`.
//
// TODO(mod-api): the messages came through `horizon_protocol::mod_net`, which is gone (mod
// payloads now use the raw payload transport), and `take_payload` decoded them from it. Until
// LetsCam records and replays SoulsCraft's state from the new transport, nothing feeds
// `MobLabels`, so `line` gives an empty line and the sheets carry no mob phases.
use super::Game;
use letscam_engine::mob_labels::MobLabels;

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
