fn letscam_payload_recorded(remote: &mut Remote, events: &mut std::collections::VecDeque<Received>) {
    // `Remote::receive_credited_backlog` as it is at 8f83e5c, plus the recording of SoulsCraft's
    // payloads (horizon-mod.toml, `payload_recorded`): keep it in step with the pinned source.
    for receipt in remote.session.poll() {
        let (event, credit) = receipt.into_parts();
        if let Event::RawPayload(incoming) = &event
            && let horizon_client::raw_payloads::Body::Payload(bytes) = &incoming.body
            && let Some(packet) = crate::game::letscam_glue::live_payload(&incoming.id.0, &bytes.0)
        {
            remote.session.record_clientbound(&packet);
        }
        events.push_back(match event {
            Event::Packet(p) if ::core::matches!(*p, C::LevelChunkWithLight(_)) => {
                match remote.decoder.submit(remote.chunk_format.clone(), p) {
                    None => Received::Chunk(remote.chunk_format.clone(), credit),
                    Some(p) => Received::Event(Event::Packet(p), credit),
                }
            }
            event => Received::Event(event, credit),
        });
    }
}
