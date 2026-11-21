fn letscam_advance_replay(game: &mut Game<'_>) {
    crate::game::letscam_glue::before_replay_advance(game);
}
