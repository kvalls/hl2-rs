//! `func_tracktrain`, `path_track` and `point_teleport` inputs (SDK trains.cpp,
//! pathtrack.cpp, pointteleport.cpp).

use super::{Delivery, Scene};
use crate::trains::Teleport;
use glam::Vec3;

/// CFuncRotating's inputs (rotating.rs).
pub(super) fn rotating(scene: &mut Scene, d: &Delivery) -> bool {
    let time = scene.time;
    let value = d.pending.parameter.trim().parse().unwrap_or(0.);
    scene
        .rotating_state(d.world, d.id)
        .input(d.input, value, time)
}

/// Stop, StartForward, StartBackward, Toggle, Resume, Reverse, SetSpeed, SetSpeedReal and
/// SetSpeedDir (SetSpeedDirAccel without its acceleration).
pub(super) fn tracktrain(scene: &mut Scene, d: &Delivery) -> bool {
    let (world, id) = (d.world, d.id);
    scene.train(world, id);
    let train = scene.trains.trains[&id].clone();
    match d.input {
        "stop" => scene.stop_train(id),
        "startforward" | "startbackward" => {
            scene.set_train_forward(id, d.input == "startforward");
            scene.set_train_speed(world, id, train.max_speed);
        }
        "toggle" => {
            let speed = if train.speed == 0. {
                train.max_speed
            } else {
                0.
            };
            scene.set_train_speed(world, id, speed);
        }
        "resume" => {
            scene.trains.trains.get_mut(&id).expect("train").speed = 0.;
            scene.set_train_speed(world, id, train.old_speed);
        }
        "reverse" => {
            scene.set_train_forward(id, train.dir < 0.);
            let speed = scene.trains.trains[&id].speed;
            scene.trains.trains.get_mut(&id).expect("train").speed = 0.;
            scene.set_train_speed(world, id, speed);
        }
        "setspeed" => scene.set_train_speed(world, id, train.max_speed * d.value.clamp(0., 1.)),
        "setspeedreal" => scene.set_train_speed(world, id, d.value.clamp(0., train.max_speed)),
        "setspeeddir" | "setspeeddiraccel" => {
            scene.set_train_forward(id, d.value >= 0.);
            scene.set_train_speed(world, id, train.max_speed * d.value.abs().clamp(0., 1.));
        }
        _ => return false,
    }
    true
}

/// InPass (OnPass), EnablePath, DisablePath and TogglePath.
pub(super) fn path_track(scene: &mut Scene, d: &Delivery) -> bool {
    let id = d.id;
    let enabled = || {
        scene
            .trains
            .path_enabled
            .get(&id)
            .copied()
            .unwrap_or_else(|| {
                d.entity
                    .get("spawnflags")
                    .and_then(|f| f.trim().parse::<u32>().ok())
                    .unwrap_or(0)
                    & 1
                    == 0
            })
    };
    let value = match d.input {
        "inpass" => {
            scene.fire(id, "OnPass", d.pending.activator);
            return true;
        }
        "enablepath" => true,
        "disablepath" => false,
        "togglepath" => !enabled(),
        _ => return false,
    };
    scene.trains.path_enabled.insert(id, value);
    true
}

/// Teleport: its `target` (the player included) goes to the teleporter's origin and angles,
/// keeping its velocity; SF_TELEPORT_TO_SPAWN_POS (1) sends it back to where it spawned.
pub(super) fn point_teleport(scene: &mut Scene, d: &Delivery) -> bool {
    if d.input != "teleport" {
        return false;
    }
    let mut lookup = d.pending.clone();
    lookup.target = d.entity.get("target").unwrap_or("").to_string();
    let to_spawn = d
        .entity
        .get("spawnflags")
        .and_then(|f| f.trim().parse::<u32>().ok())
        .unwrap_or(0)
        & 1
        != 0;
    let angles = |e: &modkit_core::Entity| {
        modkit_core::parse_vec3(e.get("angles").unwrap_or("0 0 0")).unwrap_or(Vec3::ZERO)
    };
    for target in scene.targets(d.world, &lookup) {
        if target == usize::MAX {
            scene.trains.teleport = Some(Teleport {
                feet: d.entity.origin(),
                angles: angles(d.entity),
            });
        } else if let Some(state) = scene.states.get_mut(target) {
            let source = if to_spawn {
                &d.world.entities[target]
            } else {
                d.entity
            };
            state.origin = source.origin();
            state.rotation = crate::physics::angles(angles(source));
        }
    }
    true
}
