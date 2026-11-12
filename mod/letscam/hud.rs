fn letscam_hud(game: &mut crate::game::Game<'_>, batch: &mut horizon_client::hud::HudBatch) {
    crate::game::letscam_glue::on_hud(game, batch);
}
