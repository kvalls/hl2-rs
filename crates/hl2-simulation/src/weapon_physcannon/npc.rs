//! The gravity gun's hand on an NPC its AI flies (DESIGN 14p session 50). In the SDK the manhack
//! has a VPhysics object, so the gun picks it up, holds, drops, launches and punts it as it does a
//! prop and tells it through OnPhysGunPickup and OnPhysGunDrop; here the AI applies what the gun
//! asks, through `Scene::npc_physgun`.

use super::{launch_speed, PULL_FORCE};
use crate::entities::Scene;
use crate::gameplay::Inventory;
use crate::grab::{GrabController, HoldView, HOLD_MAX_ERROR};
use crate::physics::Physics;
use glam::Vec3;

/// What the gravity gun does to an NPC this tick, for its AI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NpcPhysgun {
    /// OnPhysGunPickup(PUNTED_BY_CANNON), with the speed the punt adds.
    Punt(Vec3),
    /// A pull from afar: speed added, nothing else.
    Push(Vec3),
    /// OnPhysGunPickup(PICKED_UP_BY_CANNON).
    Pickup,
    /// The grab controller's velocity toward its target and the yaw it holds, in degrees.
    Hold { velocity: Vec3, yaw: f32 },
    /// OnPhysGunDrop: DROPPED_BY_CANNON without a launch, LAUNCHED_BY_CANNON with the speed it
    /// leaves at.
    Drop { launch: Option<Vec3> },
}

/// The manhack's box (HULL_TINY_CENTERED) stands in for its VPhysics shape's extent.
const FLYER_EXTENTS: Vec3 = Vec3::new(8., 8., 4.);

/// Whether the gun may take hold of this NPC: the AI stage lists them.
pub(super) fn holdable(scene: &Scene, id: usize) -> bool {
    scene.physgun_npcs.contains(&id)
}

/// FindObjectInCone's candidates among those NPCs: their middles.
pub(super) fn cone_candidates(scene: &Scene) -> impl Iterator<Item = (usize, Vec3)> + '_ {
    let origin = |id: usize| scene.states.get(id).map(|s| (id, s.origin));
    scene.physgun_npcs.iter().filter_map(move |&id| origin(id))
}

/// The pull's speed on a light body: ApplyForceCenter(pullDir x 4000) scaled by (mass + 0.5) / 50
/// under 50 kg, over its mass.
pub(super) fn pull_speed(toward: Vec3, mass: f32) -> Vec3 {
    let mut pull = toward.normalize_or_zero() * PULL_FORCE;
    if mass < 50. {
        pull *= (mass + 0.5) / 50.;
    }
    pull / mass.max(1e-3)
}

impl Inventory {
    /// AttachObject for an NPC: the grab starts where it is and keeps its rotation in player
    /// space; the NPC is told it is held.
    pub(super) fn attach_npc(&mut self, scene: &mut Scene, id: usize, forward: Vec3) {
        let Some(state) = scene.states.get(id) else {
            return;
        };
        let mass = crate::ai::manhack::PHYSICS_MASS;
        let grab = GrabController::attach_flyer(id, state.origin, state.rotation, forward, mass);
        self.physcannon.held = Some(id);
        self.physcannon.grab = Some(grab);
        self.physcannon.holds_npc = true;
        scene.npc_physgun.push((id, NpcPhysgun::Pickup));
    }
    /// UpdateObject for a held NPC: the velocity toward the target and the yaw it is held at go
    /// to its AI. False lets go: it died, or the error passed 12 units.
    pub(super) fn hold_npc(
        &mut self,
        physics: &Physics,
        scene: &mut Scene,
        view: HoldView,
    ) -> bool {
        let Some(grab) = self.physcannon.grab.as_mut() else {
            return false;
        };
        let id = grab.entity;
        let alive = scene.states.get(id).filter(|s| !s.killed && !s.corpse);
        let Some(origin) = alive.map(|s| s.origin) else {
            return false;
        };
        let Some(velocity) = grab.track(physics, origin, FLYER_EXTENTS, view, HOLD_MAX_ERROR)
        else {
            return false;
        };
        let forward = grab.target_rotation() * Vec3::X;
        let yaw = forward.y.atan2(forward.x).to_degrees();
        scene
            .npc_physgun
            .push((id, NpcPhysgun::Hold { velocity, yaw }));
        true
    }
    /// The NPC side of a drop or a launch, told at once when there is a scene, else at the next
    /// frame (a holster).
    pub(super) fn release_npc(
        &mut self,
        scene: Option<&mut Scene>,
        id: usize,
        launch: Option<Vec3>,
    ) {
        self.physcannon.holds_npc = false;
        let event = (id, NpcPhysgun::Drop { launch });
        match scene {
            Some(scene) => scene.npc_physgun.push(event),
            None => self.physcannon.npc_events.push(event),
        }
    }
}

/// LaunchObject's speed for a held NPC of this mass: cleared, then along the aim.
pub(super) fn launch_velocity(forward: Vec3, mass: f32) -> Vec3 {
    forward * launch_speed(mass)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pull_on_a_manhack_adds_about_ninety_units_a_second_toward_the_eye() {
        let speed = pull_speed(Vec3::new(0., -200., 0.), crate::ai::manhack::PHYSICS_MASS);
        // 4000 x (3.69 + 0.5) / 50 over 3.69 kg.
        let expected = 4000. * (crate::ai::manhack::PHYSICS_MASS + 0.5) / 50.;
        assert!((speed.length() - expected / crate::ai::manhack::PHYSICS_MASS).abs() < 1e-3);
        assert!(speed.y < 0. && speed.x == 0.);
    }
    #[test]
    fn a_launched_manhack_leaves_at_the_full_launch_speed() {
        let v = launch_velocity(Vec3::X, crate::ai::manhack::PHYSICS_MASS);
        assert_eq!(v, Vec3::X * 1500.);
    }
}
