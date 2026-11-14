//! `func_tracktrain` on `path_track` paths (SDK trains.cpp `CFuncTrackTrain`, pathtrack.cpp
//! `CPathTrack`) and the moves the trains and `point_teleport` give the local player (SDK
//! physics_main.cpp `CPhysicsPushedEntities`: a player standing on a pusher moves with it and
//! its view turns by the pusher's yaw).
use glam::{Quat, Vec3};
use modkit_core::World;
use std::collections::BTreeMap;

/// SF_TRACKTRAIN_NOPITCH.
pub const NO_PITCH: u32 = 0x1;
/// SF_TRACKTRAIN_NOCONTROL: the speed of each path_track it reaches sets the train's speed.
pub const NO_CONTROL: u32 = 0x2;
/// SF_TRACKTRAIN_PASSABLE: not solid.
pub const PASSABLE: u32 = 0x8;
/// SF_TRACKTRAIN_FIXED_ORIENTATION.
pub const FIXED_ORIENTATION: u32 = 0x10;
/// SF_PATH_DISABLED.
const PATH_DISABLED: u32 = 0x1;
/// SF_PATH_DISABLE_TRAIN: a train reaching it loses its controls (NOCONTROL).
pub const PATH_DISABLE_TRAIN: u32 = 0x8;

/// One `func_tracktrain`'s state.
#[derive(Clone, Debug, Default)]
pub struct Train {
    /// m_ppath: the path_track the train is on (the start of its current segment).
    pub path: Option<usize>,
    pub speed: f32,
    pub old_speed: f32,
    /// m_maxSpeed (`startspeed`; else `speed`; else 100).
    pub max_speed: f32,
    /// m_dir: 1 forward, -1 backward.
    pub dir: f32,
    pub height: f32,
    /// m_length (`wheels`): how far ahead the train looks to face along the path.
    pub length: f32,
    pub flags: u32,
    pub velocity: Vec3,
    /// Local angular velocity in degrees per second (pitch, yaw, roll).
    pub angular: Vec3,
    /// Local angles in degrees (pitch, yaw, roll).
    pub angles: Vec3,
    /// When the move toward a dead end's last node ends (SetMoveDone DeadEnd).
    pub dead_end_at: Option<f64>,
    /// m_eVelocityType: 0 instantaneous, 1 linear blend, 2 ease in/out.
    pub velocity_type: u8,
    /// m_eOrientationType: 0 fixed, 1 at path tracks, 2 linear blend, 3 ease in/out.
    pub orientation_type: u8,
    pub sound: TrainSound,
}

/// The train's sounds (MoveSound, StartSound, StopSound) and its move sound's state.
#[derive(Clone, Debug, Default)]
pub struct TrainSound {
    pub moving: String,
    pub start: String,
    pub stop: String,
    /// m_flVolume (`volume` / 10).
    pub volume: f32,
    /// MoveSoundMinPitch / MoveSoundMaxPitch (60 / 200 when 0).
    pub pitch: (f32, f32),
    /// m_bSoundPlaying, and the pitch last sent.
    pub playing: Option<f32>,
}

/// SF_TRACKTRAIN_USE_MAXSPEED_FOR_PITCH.
pub const MAXSPEED_PITCH: u32 = 0x100;
/// TRAIN_MAXSPEED: the speed of the highest pitch without that flag.
pub const SOUND_MAX_SPEED: f32 = 1000.;

/// A pusher's pose before and after one scene tick.
#[derive(Clone, Copy, Debug)]
pub struct Push {
    pub entity: usize,
    /// The scene time after the move.
    pub time: f64,
    pub from: (Vec3, Quat),
    pub to: (Vec3, Quat),
}

impl Push {
    /// Where a point riding the pusher ends up, and the yaw it turned by in degrees.
    pub fn carry(&self, point: Vec3) -> (Vec3, f32) {
        let turn = self.to.1 * self.from.1.inverse();
        let moved = self.to.0 + turn * (point - self.from.0);
        let forward = turn * Vec3::X;
        (moved, forward.y.atan2(forward.x).to_degrees())
    }
}

/// point_teleport's move of the local player: feet and view angles in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Teleport {
    pub feet: Vec3,
    pub angles: Vec3,
}

/// Each path_track's next node (its `target`) and previous node.
#[derive(Clone, Debug, Default)]
pub struct Links {
    next: BTreeMap<usize, usize>,
    previous: BTreeMap<usize, usize>,
}

impl Links {
    pub fn new(world: &World) -> Self {
        let mut named = BTreeMap::new();
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() == "path_track" {
                if let Some(name) = e.get("targetname") {
                    named.entry(name.to_lowercase()).or_insert(id);
                }
            }
        }
        let mut links = Self::default();
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() != "path_track" {
                continue;
            }
            let target = e.get("target").map(str::to_lowercase);
            if let Some(&next) = target.as_deref().and_then(|t| named.get(t)) {
                links.next.insert(id, next);
                links.previous.entry(next).or_insert(id);
            }
        }
        links
    }
    pub fn next_in_dir(&self, node: usize, forward: bool) -> Option<usize> {
        if forward {
            self.next.get(&node).copied()
        } else {
            self.previous.get(&node).copied()
        }
    }
}

/// The trains of a map and the player moves of this tick.
#[derive(Clone, Debug, Default)]
pub struct Trains {
    pub trains: BTreeMap<usize, Train>,
    /// path_track links, built when a train first needs them.
    pub links: Option<Links>,
    /// EnablePath / DisablePath / TogglePath overrides of SF_PATH_DISABLED.
    pub path_enabled: BTreeMap<usize, bool>,
    /// The pushers that moved in the latest scene tick.
    pub pushes: Vec<Push>,
    /// A point_teleport of the local player, applied before its next move.
    pub teleport: Option<Teleport>,
    /// func_rotating state, created at each one's first tick (rotating.rs).
    pub rotating: BTreeMap<usize, crate::rotating::Rotating>,
}

/// The end of a `look_ahead`: the node whose segment holds the point, the node after it,
/// and the point.
pub struct Ahead {
    pub node: Option<usize>,
    pub next: Option<usize>,
    pub point: Vec3,
}

impl Trains {
    /// CPathTrack::ValidPath: a node, and with `test` an enabled one.
    fn valid(&self, world: &World, node: Option<usize>, test: bool) -> Option<usize> {
        let node = node?;
        let disabled = self.path_enabled.get(&node).map_or_else(
            || {
                world.entities[node]
                    .get("spawnflags")
                    .and_then(|f| f.trim().parse::<u32>().ok())
                    .unwrap_or(0)
                    & PATH_DISABLED
                    != 0
            },
            |enabled| !enabled,
        );
        (!test || !disabled).then_some(node)
    }
    /// CPathTrack::LookAhead from `start` at `origin`, `dist` along the path (negative:
    /// backward). With `moving`, disabled nodes end the path; without it, the point runs on
    /// past the last node along its last segment (Project).
    pub fn look_ahead(
        &self,
        world: &World,
        start: usize,
        origin: Vec3,
        dist: f32,
        moving: bool,
    ) -> Ahead {
        let links = self.links.as_ref().expect("path links are built first");
        let at = |n: usize| world.entities[n].origin();
        let forward = dist >= 0.;
        let original = dist.abs();
        let mut dist = original;
        let mut current = start;
        let mut position = origin;
        let mut point = origin;
        while dist > 0. {
            let Some(next) = self.valid(world, links.next_in_dir(current, forward), moving) else {
                if !moving {
                    if let Some(before) = links.next_in_dir(current, !forward) {
                        point = at(current) + (at(current) - at(before)).normalize_or_zero() * dist;
                    }
                }
                return Ahead {
                    node: None,
                    next: None,
                    point,
                };
            };
            let segment = at(next) - position;
            let length = segment.length();
            let after = self.valid(world, links.next_in_dir(next, forward), moving);
            if length == 0. && after.is_none() {
                let node = (dist != original).then_some(next);
                return Ahead {
                    node,
                    next: None,
                    point,
                };
            }
            if length > dist {
                point = position + segment * (dist / length);
                return Ahead {
                    node: Some(current),
                    next: Some(next),
                    point,
                };
            }
            dist -= length;
            position = at(next);
            current = next;
            point = position;
        }
        Ahead {
            node: Some(current),
            next: links.next_in_dir(current, forward),
            point,
        }
    }
}

/// UTIL_AngleDistance: `next - current` wrapped into -180..180 degrees.
pub fn angle_distance(next: f32, current: f32) -> f32 {
    let delta = (next - current).rem_euclid(360.);
    if delta > 180. {
        delta - 360.
    } else {
        delta
    }
}
