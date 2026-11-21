fn letscam_screen_render_post(game: &Game<'_>, batch: &mut horizon_client::hud::HudBatch) {
    crate::game::letscam_screen::draw_open(game, batch);
}
