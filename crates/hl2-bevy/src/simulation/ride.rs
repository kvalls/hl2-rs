//! The local player moved by map entities before its own move: a point_teleport, and a
//! func_tracktrain it stands on or that moved into it (SDK CPhysicsPushedEntities: a player on
//! a pusher, or one CPushBlockerEnum::IntersectsPushers finds inside the pusher's new pose,
//! moves with it, and FinishRotPushedEntity turns its view by the pusher's yaw). The scene
//! also learns whether the player is off the ground (UTIL_ScreenShake skips such a player).

use super::Simulation;
use crate::gameplay::Gameplay;
use glam::Vec3;

pub(super) fn apply(sim: &mut Simulation, game: &mut Gameplay) {
    game.scene.shakes.player_airborne = !sim.player.grounded;
    if let Some(teleport) = game.scene.trains.teleport.take() {
        // CBasePlayer::Teleport with angles snaps the view; the velocity stays.
        sim.player.feet = teleport.feet;
        sim.eye = sim.player.eye();
        sim.pitch = (-teleport.angles.x).to_radians().clamp(-1.53, 1.53);
        sim.yaw = teleport.angles.y.to_radians();
    }
    if sim.player.dead {
        return;
    }
    let time = game.scene.time;
    let pushes = game.scene.trains.pushes.iter().filter(|p| p.time == time);
    let height = if sim.player.crouched { 36. } else { 72. };
    let (mins, maxs) = (Vec3::new(-16., -16., 0.), Vec3::new(16., 16., height));
    for push in pushes {
        let ground = sim
            .physics
            .ground_ray(sim.player.feet + Vec3::Z * 2., Vec3::NEG_Z, 8.)
            .filter(|_| sim.player.grounded);
        let standing = ground.is_some_and(|hit| hit.entity == push.entity);
        if !standing
            && !sim
                .physics
                .hull_overlaps_entity(sim.player.feet, mins, maxs, push.entity)
        {
            continue;
        }
        let (feet, yaw) = push.carry(sim.player.feet);
        sim.player.feet = feet;
        sim.eye = sim.player.eye();
        sim.yaw += yaw.to_radians();
    }
}
