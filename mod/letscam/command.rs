fn letscam_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    crate::game::letscam_glue::on_command(game, command, ran);
}
