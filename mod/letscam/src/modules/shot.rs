// Where the render's camera looks and stands: the `focus=` entity and the director's pose on
// the studio free camera. Compiled as the child module `game::letscam_shot`.
use super::Game;
use super::entity::Entity;
use letscam_engine as engine;

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

/// An entity the client shows (the replay's, while rendering), as the focus, the sheets' labels
/// and the diagnostics read it. The old `native_client_entities` helper is gone from the game,
/// so this reads the game's own lists: the mobs (the other players among them) and what a
/// player rides besides them (boats, minecarts).
pub(super) struct Seen {
    pub(super) id: u32,
    /// The entity type, `minecraft:pig`, `soulscraft:revenant`.
    pub(super) kind: String,
    pub(super) position: [f64; 3],
    previous: [f64; 3],
    pub(super) yaw: f32,
    pub(super) height: f32,
    pub(super) alive: bool,
    pub(super) spectator: bool,
    pub(super) player: bool,
}

impl Seen {
    fn of(entity: &impl Entity, spectator: bool) -> Seen {
        let at = |v: horizon_core::geometry::Vec3| [v.x, v.y, v.z];
        let bounds = entity.bounds();
        let kind = entity.native_type().into_owned();
        Seen {
            id: entity.id(),
            player: kind == "minecraft:player",
            kind,
            position: at(entity.position()),
            previous: at(entity.previous_position()),
            yaw: entity.rotation()[0],
            height: (bounds.max_position().y - bounds.min_position().y) as f32,
            alive: entity.is_alive(),
            spectator,
        }
    }

    /// Where it is drawn at `partial` of the tick: between the last tick's position and this one.
    pub(super) fn position_at(&self, partial: f64) -> [f64; 3] {
        std::array::from_fn(|i| self.previous[i] + (self.position[i] - self.previous[i]) * partial)
    }
}

pub(super) fn entities(game: &Game<'_>) -> Vec<Seen> {
    let mobs = game.mobs.iter().map(|m| Seen::of(m, m.extra.spectator));
    let boats = game.boats.iter().map(|b| Seen::of(b, false));
    let carts = game.carts.iter().map(|c| Seen::of(c, false));
    mobs.chain(boats).chain(carts).collect()
}

/// The focus entity for this frame: the one already settled on while it lives, else the first
/// match for `spec` (the nearest to the recorded player's eye), which it then sticks to.
pub(super) fn find_focus(game: &Game<'_>, spec: &FocusSpec, locked: &mut Option<u32>) -> Option<engine::Focus> {
    let eye = game.eye_position()?;
    let entities = entities(game);
    let alive = || entities.iter().filter(|e| e.alive && !e.spectator);
    let distance = |p: [f64; 3]| {
        let d = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let nearest = |pick: &dyn Fn(&Seen) -> bool| {
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
                FocusSpec::Ride => nearest(&|e| !e.player && distance(e.position) <= RIDE_RADIUS),
            }?;
            *locked = Some(found.id);
            found
        }
    };
    let [x, y, z] = entity.position_at(game.frame_world_partial_tick());
    Some(engine::Focus {
        pos: [x, y + f64::from(entity.height) / 2.0, z],
        height: entity.height,
        yaw: Some(entity.yaw),
    })
}

pub(super) struct Probe<'g, 'a>(pub(super) &'g Game<'a>);

impl engine::WorldProbe for Probe<'_, '_> {
    /// `Level.clip` with VISUAL shapes and no fluids (what the old `native_client_visual_clear`
    /// asked): true when nothing drawn stands between the two points.
    fn clear(&self, from: [f64; 3], to: [f64; 3]) -> bool {
        let v = |p: [f64; 3]| horizon_core::geometry::Vec3::new(p[0], p[1], p[2]);
        self.0.mob_world_base(&[]).visual_clear_between(v(from), v(to))
    }
}

/// Poses the studio free camera. `fp` (`first_person`): the replay's own first-person view (the
/// recorded eye, yaw and pitch, with the hand) is that angle exactly, and only with the free
/// camera off is the own body kept out of it.
pub(super) fn apply_pose(game: &mut Game<'_>, pose: engine::CameraPose, first_person: bool) {
    if first_person {
        if game.studio.freecam.active {
            game.studio.freecam.deactivate();
        }
        return;
    }
    let position = glam::DVec3::from_array(pose.pos);
    let cam = &mut game.studio.freecam;
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
