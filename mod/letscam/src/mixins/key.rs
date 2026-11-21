fn letscam_key(game: &mut Game<'_>, key: &str, action: mod_api::KeyAction) {
    let pressed = !matches!(action, mod_api::KeyAction::Release);
    crate::game::letscam_glue::on_key(game, key, pressed);
}
