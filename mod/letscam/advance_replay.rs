fn letscam_advance_replay(game: &mut crate::game::Game<'_>) {
    crate::game::letscam_glue::before_replay_advance(game);
}
