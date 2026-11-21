// Render diagnostics in the game's log: when the own body enters or leaves the view, and twice
// a second where the replay shows the player next to the model the game draws (#16527).
// Compiled as the child module `game::letscam_diag`.
use super::Game;

thread_local! {
    /// The last half second of replay time the sync diagnostics logged.
    static LAST_SYNC_LOG: std::cell::Cell<u32> = const { std::cell::Cell::new(u32::MAX) };
    /// The last (own body in view, free camera active) the render logged, to log only changes.
    static LAST_BODY_IN_VIEW: std::cell::Cell<Option<(bool, bool)>> = const { std::cell::Cell::new(None) };
}

/// Logs this frame's body-in-view change and, each half second of replay time, the sync line.
pub(super) fn log_frame(game: &Game<'_>) {
    let in_view = (game.own_body_in_view(), game.studio.freecam.active);
    if LAST_BODY_IN_VIEW.get() != Some(in_view) {
        LAST_BODY_IN_VIEW.set(Some(in_view));
        let t = game.replay.driver.as_ref().map_or(0, |d| d.position_ms());
        log::info!("letscam: body in view={} freecam={} at t_ms={t}", in_view.0, in_view.1);
    }
    // Sync diagnostics: where the replay shows the player, at which replay time (twice a second).
    if let (Some(driver), Some(eye)) = (game.replay.driver.as_ref(), game.eye_position()) {
        let t = driver.position_ms();
        if t / 500 != LAST_SYNC_LOG.get() {
            LAST_SYNC_LOG.set(t / 500);
            let model = game.player_model.as_ref().map(|m| [m.position.x, m.position.y, m.position.z]);
            let model_prev = game.player_model.as_ref().map(|m| [m.previous.x, m.previous.y, m.previous.z]);
            let model_yaw = game.player_model.as_ref().map(|m| (m.yaw, m.body_yaw));
            let game_yaw = game.yaw;
            // The light the own model is drawn with (perspective.rs player_model_geometry): the
            // cell at eye height, sky light in the high nibble. A door there is the suspect.
            let light = game.player_model.as_ref().zip(game.player.as_ref()).map(|(m, p)| {
                let eye = f64::from(p.dimensions.eye_height);
                let probe = super::perspective::entity_light_block(m.position, m.extra.crouching, eye);
                let feet = [
                    m.position.x.floor() as i32,
                    m.position.y.floor() as i32,
                    m.position.z.floor() as i32,
                ];
                (
                    probe,
                    format!("{:#04x}", game.world.light(probe[0], probe[1], probe[2])),
                    format!("{:#04x}", game.world.light(feet[0], feet[1], feet[2])),
                )
            });
            // Every creature within 8 blocks of the replay player, of any type: a second figure of
            // the player (drawn ~4 s ahead of the own model in the doorway scene) shows up here.
            let near = |p: [f64; 3]| {
                (p[0] - eye[0]).powi(2) + (p[1] - eye[1]).powi(2) + (p[2] - eye[2]).powi(2) < 64.0
            };
            let players: Vec<(u32, String, [f64; 3], f32, bool)> = super::letscam_shot::entities(game)
                .into_iter()
                .filter(|e| near(e.position))
                .map(|e| (e.id, e.kind, e.position, e.yaw, e.player))
                .collect();
            // Why the own model may not be drawn (perspective.rs player_model_shown / own_body_visible).
            let shown = game.player_model_shown();
            let body = game.own_body_visible();
            let dead = game.player.as_ref().map(|p| p.health.dead_or_dying());
            let freecam = game.studio.freecam.active;
            let first = game.camera_type.is_first_person();
            log::info!(
                "letscam: sync t_ms={t} replay_eye={eye:?} model={model:?} players={players:?} \
                 shown={shown} body={body} dead={dead:?} freecam={freecam} first_person={first} \
                 model_prev={model_prev:?} model_yaw={model_yaw:?} game_yaw={game_yaw} \
                 light(probe,eye,feet)={light:?}"
            );
        }
    }
}
