fn letscam_hud(game: &Game<'_>, batch: &mut HudBatch) {
    crate::game::letscam_glue::on_hud(game, batch);
}
