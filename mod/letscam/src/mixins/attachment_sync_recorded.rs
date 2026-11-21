fn letscam_attachment_sync_recorded(game: &mut Game<'_>, data: &[u8]) {
    crate::game::letscam_glue::record_payload(
        game,
        horizon_protocol::attachment_sync::CHANNEL,
        data,
    );
}
