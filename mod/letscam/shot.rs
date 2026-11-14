// Where the render's camera looks and stands: the `focus=` entity and the director's pose on
// the studio free camera. Compiled as the child module `game::letscam_shot`.
use super::Game;
use horizon_client_render::letscam::engine;

/// `focus=` of `/letscam render`: an entity id, an entity type (`minecraft:pig`,
/// `soulscraft:revenant`, or without the namespace), or `ride`: what the player sits on.
#[derive(Clone, Debug)]
pub(super) enum FocusSpec {
    Id(u32),
    Kind(String),
    Ride,
}

impl FocusSpec {
    pub(super) fn parse(word: &str) -> FocusSpec {
        match word {
            "ride" | "mount" | "vehicle" => FocusSpec::Ride,
            _ => word
                .parse()
                .map_or_else(|_| FocusSpec::Kind(word.to_owned()), FocusSpec::Id),
        }
    }
}

/// Within this many blocks of the eye, the nearest creature is what the player rides.
const RIDE_RADIUS: f64 = 3.0;

/// The focus entity for this frame: the one already settled on while it lives, else the first
/// match for `spec` (the nearest to the recorded player's eye), which it then sticks to.
pub(super) fn find_focus(game: &Game<'_>, spec: &FocusSpec, locked: &mut Option<u32>) -> Option<engine::Focus> {
    let state = game.native_client_state()?;
    let eye = state.eye_position?;
    let entities = game.native_client_entities();
    let alive = || entities.iter().filter(|e| e.alive && !e.spectator);
    let distance = |p: [f64; 3]| {
        let d = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let nearest = |pick: &dyn Fn(&super::native_client_services::NativeClientEntity) -> bool| {
        alive()
            .filter(|e| pick(e))
            .min_by(|a, b| distance(a.position).total_cmp(&distance(b.position)))
    };
    let entity = match locked.and_then(|id| alive().find(|e| e.id == id)) {
        Some(entity) => entity,
        None => {
            let found = match spec {
                FocusSpec::Id(id) => alive().find(|e| e.id == *id),
                FocusSpec::Kind(kind) => nearest(&|e| {
                    e.kind == *kind || e.kind.rsplit(':').next() == Some(kind.as_str())
                }),
                FocusSpec::Ride => nearest(&|e| {
                    e.player_uuid.is_none() && distance(e.position) <= RIDE_RADIUS
                }),
            }?;
            *locked = Some(found.id);
            found
        }
    };
    let [x, y, z] = entity.position_at(state.world_partial_tick);
    Some(engine::Focus {
        pos: [x, y + f64::from(entity.height) / 2.0, z],
        height: entity.height,
        yaw: Some(entity.yaw),
    })
}

pub(super) struct Probe<'g, 'a>(pub(super) &'g Game<'a>);

impl engine::WorldProbe for Probe<'_, '_> {
    fn clear(&self, from: [f64; 3], to: [f64; 3]) -> bool {
        self.0.native_client_visual_clear(from, to)
    }
}

/// Poses the studio free camera. `fp` (`first_person`): the replay's own first-person view (the
/// recorded eye, yaw and pitch, with the hand) is that angle exactly, and only with the free
/// camera off is the own body kept out of it.
pub(super) fn apply_pose(game: &mut Game<'_>, pose: engine::CameraPose, first_person: bool) {
    if first_person {
        if game.studio_freecam.active {
            game.studio_freecam.deactivate();
        }
        return;
    }
    let position = glam::DVec3::from_array(pose.pos);
    let cam = &mut game.studio_freecam;
    if cam.active {
        cam.position = position;
        cam.yaw = pose.yaw;
        cam.pitch = pose.pitch;
        cam.fov = pose.fov;
        cam.velocity = glam::DVec3::ZERO;
        cam.locked_to_path = false;
    } else {
        cam.activate(position, pose.yaw, pose.pitch, pose.fov);
    }
}

/// The studio camera path's pose at `t_ms`, for the `keys` angle: a path needs two keyframes.
pub(super) fn keyed_pose(
    track: &horizon_client::studio::CameraTrack,
    t_ms: u32,
) -> Option<engine::CameraPose> {
    if track.len() < 2 {
        return None;
    }
    let pose = track.evaluate(t_ms)?;
    Some(engine::CameraPose {
        pos: pose.position.to_array(),
        yaw: pose.yaw,
        pitch: pose.pitch,
        fov: pose.fov,
    })
}
