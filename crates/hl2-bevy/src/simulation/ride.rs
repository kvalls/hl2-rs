//! The local player moved by map entities before its own move: a point_teleport, and the
//! func_tracktrain it stands on (SDK CPhysicsPushedEntities: a player on a pusher moves with
//! it, and FinishRotPushedEntity turns its view by the pusher's yaw).

use super::Simulation;
use crate::gameplay::Gameplay;
use glam::Vec3;

pub(super) fn apply(sim: &mut Simulation, game: &mut Gameplay) {
    if let Some(teleport) = game.scene.trains.teleport.take() {
        // CBasePlayer::Teleport with angles snaps the view; the velocity stays.
        sim.player.feet = teleport.feet;
        sim.eye = sim.player.eye();
        sim.pitch = (-teleport.angles.x).to_radians().clamp(-1.53, 1.53);
        sim.yaw = teleport.angles.y.to_radians();
    }
    if !sim.player.grounded || sim.player.dead {
        return;
    }
    let time = game.scene.time;
    let pushes = game.scene.trains.pushes.iter().filter(|p| p.time == time);
    for push in pushes {
        let ground = sim
            .physics
            .ground_ray(sim.player.feet + Vec3::Z * 2., Vec3::NEG_Z, 8.);
        if ground.is_none_or(|hit| hit.entity != push.entity) {
            continue;
        }
        let (feet, yaw) = push.carry(sim.player.feet);
        sim.player.feet = feet;
        sim.eye = sim.player.eye();
        sim.yaw += yaw.to_radians();
    }
}
