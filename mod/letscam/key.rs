fn letscam_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    crate::game::letscam_glue::on_key(game, key, pressed);
}
