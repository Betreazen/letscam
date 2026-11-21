fn letscam_replay_payload(game: &mut Game<'_>, packet: &horizon_protocol::packets::play::Clientbound) {
    if let horizon_protocol::packets::play::Clientbound::CustomPayload(p) = packet {
        crate::game::letscam_glue::replay_payload(game, &p.channel, &p.data);
    }
}
