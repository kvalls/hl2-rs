//! `func_tracktrain` thinks and moves (SDK trains.cpp: Find on the first think, then Next
//! every tick; MOVETYPE_PUSH integrates its velocity and angular velocity).

use super::{number, Scene};
use crate::trains::{
    angle_distance, Links, Push, Train, FIXED_ORIENTATION, NO_CONTROL, NO_PITCH, PATH_DISABLE_TRAIN,
};
use crate::view_control::angles_toward;
use glam::Vec3;
use modkit_core::World;

impl Scene {
    /// One tick of a train: its first think (Find), then Next while it has a speed, then the
    /// move by its velocity and angular velocity.
    pub(super) fn advance_train(&mut self, world: &World, id: usize, dt: f32) {
        self.train(world, id);
        if self
            .trains
            .pushes
            .first()
            .is_some_and(|p| p.time != self.time)
        {
            self.trains.pushes.clear();
        }
        let train = &self.trains.trains[&id];
        if train.dead_end_at.is_some_and(|t| t <= self.time) {
            self.train_dead_end(world, id);
        } else if train.speed != 0. {
            self.train_next(world, id);
        }
        let train = self.trains.trains.get_mut(&id).expect("train");
        if train.velocity == Vec3::ZERO && train.angular == Vec3::ZERO {
            return;
        }
        let state = &mut self.states[id];
        let from = (state.origin, state.rotation);
        state.origin += train.velocity * dt;
        train.angles += train.angular * dt;
        state.rotation = crate::physics::angles(train.angles);
        let to = (state.origin, state.rotation);
        let time = self.time;
        self.trains.pushes.push(Push {
            entity: id,
            time,
            from,
            to,
        });
    }

    /// The train's state, created by its first think (CFuncTrackTrain::Spawn and Find: it
    /// goes to its first path_track, faces `wheels` units along the path and passes the node).
    pub(super) fn train(&mut self, world: &World, id: usize) {
        if self.trains.trains.contains_key(&id) {
            return;
        }
        if self.trains.links.is_none() {
            self.trains.links = Some(Links::new(world));
        }
        let e = &world.entities[id];
        let speed = number(e, "speed", 0.);
        let start = number(e, "startspeed", 0.);
        let mut train = Train {
            max_speed: if start != 0. {
                start
            } else if speed != 0. {
                speed
            } else {
                100.
            },
            speed,
            dir: 1.,
            height: number(e, "height", 0.),
            length: number(e, "wheels", 0.),
            flags: number(e, "spawnflags", 0.) as u32,
            angles: modkit_core::parse_vec3(e.get("angles").unwrap_or("0 0 0"))
                .unwrap_or(Vec3::ZERO),
            velocity_type: number(e, "velocitytype", 0.) as u8,
            orientation_type: number(e, "orientationtype", 1.) as u8,
            sound: crate::trains::TrainSound {
                moving: e.get("MoveSound").unwrap_or("").to_string(),
                start: e.get("StartSound").unwrap_or("").to_string(),
                stop: e.get("StopSound").unwrap_or("").to_string(),
                volume: number(e, "volume", 10.).trunc() * 0.1,
                pitch: (
                    Some(number(e, "MoveSoundMinPitch", 0.))
                        .filter(|p| *p != 0.)
                        .unwrap_or(60.),
                    Some(number(e, "MoveSoundMaxPitch", 0.))
                        .filter(|p| *p != 0.)
                        .unwrap_or(200.),
                ),
                playing: None,
            },
            ..Train::default()
        };
        let target = e.get("target").map(str::to_lowercase);
        let path = world.entities.iter().position(|p| {
            p.class() == "path_track" && p.get("targetname").map(str::to_lowercase) == target
        });
        train.path = path;
        if let Some(path) = path {
            let at = world.entities[path].origin();
            let look = self
                .trains
                .look_ahead(world, path, at, train.length, false)
                .point;
            if train.flags & FIXED_ORIENTATION == 0 {
                let (pitch, yaw) = angles_toward(look - at);
                let pitch = if train.flags & NO_PITCH != 0 {
                    0.
                } else {
                    pitch
                };
                train.angles = Vec3::new(pitch, yaw, 0.);
            }
            self.states[id].origin = at + Vec3::Z * train.height;
            self.states[id].rotation = crate::physics::angles(train.angles);
        }
        self.trains.trains.insert(id, train);
        if let Some(path) = path {
            self.arrive_at_node(world, id, path);
        }
    }

    /// ArriveAtNode: the node's OnPass; SF_PATH_DISABLE_TRAIN takes the controls; without
    /// controls the node's own speed becomes the train's.
    fn arrive_at_node(&mut self, world: &World, id: usize, node: usize) {
        self.fire(node, "OnPass", id);
        let flags = number(&world.entities[node], "spawnflags", 0.) as u32;
        let train = self.trains.trains.get_mut(&id).expect("train");
        if flags & PATH_DISABLE_TRAIN != 0 {
            train.flags |= NO_CONTROL;
        }
        let speed = number(&world.entities[node], "speed", 0.);
        if train.flags & NO_CONTROL != 0 && speed != 0. {
            self.set_train_speed(world, id, speed);
        }
    }

    /// CFuncTrackTrain::SetSpeed (without the manual acceleration): starting fires OnStart.
    pub(super) fn set_train_speed(&mut self, world: &World, id: usize, speed: f32) {
        self.train(world, id);
        let train = self.trains.trains.get_mut(&id).expect("train");
        let old = train.speed;
        train.speed = speed.abs() * train.dir;
        if train.speed == old {
            return;
        }
        if train.speed == 0. {
            self.stop_train(id);
            return;
        }
        if old == 0. {
            self.fire(id, "OnStart", id);
        }
        self.train_next(world, id);
    }

    pub(super) fn stop_train(&mut self, id: usize) {
        self.train_sound_stop(id);
        if let Some(train) = self.trains.trains.get_mut(&id) {
            train.velocity = Vec3::ZERO;
            train.angular = Vec3::ZERO;
            train.old_speed = train.speed;
            train.speed = 0.;
            train.dead_end_at = None;
        }
    }

    /// SetDirForward: turning around moves the current node back or ahead by one.
    pub(super) fn set_train_forward(&mut self, id: usize, forward: bool) {
        let links = self.trains.links.clone().unwrap_or_default();
        let Some(train) = self.trains.trains.get_mut(&id) else {
            return;
        };
        let dir = if forward { 1. } else { -1. };
        if train.dir != dir {
            if let Some(next) = train.path.and_then(|p| links.next_in_dir(p, !forward)) {
                train.path = Some(next);
            }
            train.dir = dir;
        }
    }

    /// CFuncTrackTrain::Next: velocity toward the point 0.1 s ahead along the path, facing
    /// `wheels` units ahead, passing the node it reached; a dead end moves to the last node.
    fn train_next(&mut self, world: &World, id: usize) {
        let dt = if self.tick_dt > 0. {
            self.tick_dt
        } else {
            0.015
        };
        let Some(path) = self.trains.trains[&id].path else {
            self.train_sound_stop(id);
            self.trains.trains.get_mut(&id).expect("train").speed = 0.;
            return;
        };
        self.train_sound_update(id);
        let origin = self.states[id].origin;
        let train = &self.trains.trains[&id];
        let (speed, height) = (train.speed, train.height);
        let ahead =
            self.trains
                .look_ahead(world, path, origin - Vec3::Z * height, speed * 0.1, true);
        let target = ahead.point + Vec3::Z * height;
        let Some(next) = ahead.node else {
            self.train_sound_stop(id);
            let train = self.trains.trains.get_mut(&id).expect("train");
            let to = target - origin;
            train.angular = Vec3::ZERO;
            train.old_speed = train.speed;
            train.speed = 0.;
            let distance = to.length();
            train.velocity = to.normalize_or_zero() * train.old_speed.abs();
            train.dead_end_at = Some(self.time + f64::from(distance / train.old_speed.abs()));
            return;
        };
        let velocity_speed = self.train_blend_speed(world, id, next, ahead.next);
        let face = self.train_facing(world, id, next);
        let train = self.trains.trains.get_mut(&id).expect("train");
        train.speed = velocity_speed;
        train.velocity = (target - origin).normalize_or_zero() * velocity_speed.abs();
        if let Some(angles) = face {
            let pitch = if train.flags & NO_PITCH != 0 {
                0.
            } else {
                angle_distance(angles.x, train.angles.x)
            };
            let yaw = angle_distance(angles.y, train.angles.y);
            let cut = |v: f32| if v.abs() < 0.1 { 0. } else { v / dt };
            train.angular = Vec3::new(cut(pitch), cut(yaw), train.angular.z);
        }
        if next != path {
            train.path = Some(next);
            self.arrive_at_node(world, id, next);
        }
        self.fire(id, "OnNext", id);
    }

    /// UpdateTrainVelocity: linear or eased blends between the speeds of the nodes around it.
    fn train_blend_speed(&self, world: &World, id: usize, prev: usize, next: Option<usize>) -> f32 {
        let train = &self.trains.trains[&id];
        let Some(next) = next.filter(|_| train.velocity_type != 0) else {
            return train.speed;
        };
        let node_speed = |n: usize| number(&world.entities[n], "speed", 0.);
        let from = if node_speed(prev) != 0. {
            node_speed(prev)
        } else {
            train.speed
        };
        let to = if node_speed(next) != 0. {
            node_speed(next)
        } else {
            from
        };
        if from == to {
            return train.dir * from.abs();
        }
        let (a, b) = (world.entities[prev].origin(), world.entities[next].origin());
        let length = (b - a).length();
        if length == 0. {
            return train.speed;
        }
        let mut p = ((self.states[id].origin - a).length() / length).clamp(0., 1.);
        if train.velocity_type == 2 {
            p = p * p * (3. - 2. * p);
        }
        train.dir * (from * (1. - p) + to * p)
    }

    /// UpdateOrientationAtPathTracks: face the point `wheels` units ahead along the path
    /// (the blend orientation types use it too; fixed ones keep their angles).
    fn train_facing(&self, world: &World, id: usize, prev: usize) -> Option<Vec3> {
        let train = &self.trains.trains[&id];
        if train.flags & FIXED_ORIENTATION != 0 || train.orientation_type == 0 {
            return None;
        }
        let origin = self.states[id].origin;
        let length = if train.length > 0. {
            train.length
        } else {
            100.
        };
        let length = if train.dir > 0. { length } else { -length };
        let path = train.path.unwrap_or(prev);
        let ahead =
            self.trains
                .look_ahead(world, path, origin - Vec3::Z * train.height, length, false);
        let mut face = ahead.point + Vec3::Z * train.height - origin;
        if train.dir < 0. {
            face = -face;
        }
        if face.x == 0. && face.y == 0. {
            return None;
        }
        let (pitch, yaw) = angles_toward(face);
        Some(Vec3::new(pitch, yaw, 0.))
    }

    /// DeadEnd: stopped at the last enabled node, which it passes.
    fn train_dead_end(&mut self, world: &World, id: usize) {
        let links = self.trains.links.clone().unwrap_or_default();
        let train = self.trains.trains.get_mut(&id).expect("train");
        train.dead_end_at = None;
        train.velocity = Vec3::ZERO;
        train.angular = Vec3::ZERO;
        let (forward, start) = (train.old_speed >= 0., train.path);
        let enabled =
            |n: &usize| {
                self.trains.path_enabled.get(n).copied().unwrap_or_else(|| {
                    number(&world.entities[*n], "spawnflags", 0.) as u32 & 1 == 0
                })
            };
        let mut node = start;
        while let Some(next) = node
            .and_then(|n| links.next_in_dir(n, forward))
            .filter(enabled)
        {
            node = Some(next);
            if Some(next) == start {
                break;
            }
        }
        if let Some(node) = node {
            self.fire(node, "OnPass", id);
        }
    }

    /// SoundUpdate: the start sound and the move sound when it starts, then the move sound's
    /// pitch, 60..200 by |speed| / 1000 (or / max speed with USE_MAXSPEED_FOR_PITCH).
    fn train_sound_update(&mut self, id: usize) {
        let origin = self.states[id].origin;
        let train = self.trains.trains.get_mut(&id).expect("train");
        let sound = &mut train.sound;
        if sound.moving.is_empty() {
            return;
        }
        let top = if train.flags & crate::trains::MAXSPEED_PITCH != 0 {
            train.max_speed
        } else {
            crate::trains::SOUND_MAX_SPEED
        };
        let ratio = (train.speed.abs() / top).clamp(0., 1.);
        let pitch = (sound.pitch.0 + (sound.pitch.1 - sound.pitch.0) * ratio).trunc();
        let volume = sound.volume;
        let op = match sound.playing {
            None => crate::sounds::PatchOp::Play { volume, pitch },
            Some(sent) if sent != pitch => crate::sounds::PatchOp::Pitch { pitch, seconds: 0. },
            Some(_) => return,
        };
        let starting = sound.playing.is_none();
        sound.playing = Some(pitch);
        let (moving, start) = (sound.moving.clone(), sound.start.clone());
        if starting && !start.is_empty() {
            self.sounds
                .push(train_sound(&start, id, origin, volume, None));
        }
        self.sounds
            .push(train_sound(&moving, id, origin, volume, Some(op)));
    }

    /// SoundStop: the move sound stops and the stop sound plays.
    fn train_sound_stop(&mut self, id: usize) {
        let origin = self.states[id].origin;
        let Some(train) = self.trains.trains.get_mut(&id) else {
            return;
        };
        let sound = &mut train.sound;
        if sound.playing.take().is_none() {
            return;
        }
        let (moving, stop, volume) = (sound.moving.clone(), sound.stop.clone(), sound.volume);
        let destroy = Some(crate::sounds::PatchOp::Destroy);
        self.sounds
            .push(train_sound(&moving, id, origin, volume, destroy));
        if !stop.is_empty() {
            self.sounds
                .push(train_sound(&stop, id, origin, volume, None));
        }
    }
}

/// A sound the train emits from its origin (a patch op on its looping move sound, else once).
fn train_sound(
    name: &str,
    id: usize,
    origin: Vec3,
    volume: f32,
    patch: Option<crate::sounds::PatchOp>,
) -> crate::sounds::SoundRequest {
    crate::sounds::SoundRequest {
        emitter: Some(id as u64),
        origin: Some(origin),
        volume: Some(volume),
        patch,
        ..name.into()
    }
}
