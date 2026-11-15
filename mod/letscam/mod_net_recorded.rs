fn letscam_mod_net_recorded(game: &mut crate::game::Game<'_>, data: &[u8]) {
    crate::game::letscam_glue::record_payload(game, horizon_protocol::mod_net::CHANNEL, data);
    crate::game::letscam_glue::replay_payload(game, data);
}
