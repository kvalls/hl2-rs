//! Navigation jumps and climbs on the AI side (DESIGN 14o session 44): the movement
//! controller's launches become step falls, landings go back to it, and the fast zombie
//! follows its climbs (the moan, CFastZombie::ClimbTouch).
use super::{AiActor, AiClass, AiRuntime, PlayerHit, PlayerState};
use crate::ai_hull::HullType;
use crate::entities::Scene;
use crate::npc::Controller;
use crate::physics::Physics;
use glam::Vec3;
use modkit_core::World;

/// ClimbTouch's shove, and how close counts as touching.
const CLIMB_TOUCH_PUNCH: f32 = 200.;
const CLIMB_TOUCH_MARGIN: f32 = 1.;

/// Strict box overlap (touching faces do not overlap).
pub(super) fn overlaps(lo: Vec3, hi: Vec3, box_lo: Vec3, box_hi: Vec3) -> bool {
    lo.cmplt(box_hi).all() && hi.cmpgt(box_lo).all()
}
impl AiClass {
    /// In a navigation climb: gravity off, and not on the ground for attacks.
    fn climbing(&self) -> bool {
        matches!(self, AiClass::Zombie(z) if z.fast.climbing)
    }
}
impl AiActor {
    /// FL_ONGROUND: neither falling nor climbing.
    pub(super) fn on_ground(&self) -> bool {
        self.fall.is_none() && !self.class.climbing()
    }
}
impl AiRuntime {
    /// This tick's step falls, with what the movement controller adds to them.
    pub(super) fn step_falls_and_links(
        &mut self,
        world: &World,
        scene: &mut Scene,
        controller: &mut Controller,
        physics: &Physics,
        player: PlayerState,
    ) {
        for (id, velocity) in controller.take_launches() {
            match self.actors.get_mut(&id) {
                Some(actor) if !scene.states[id].killed => {
                    // Off the ground first, as every NPC jump.
                    scene.states[id].origin.z += 1.;
                    actor.fall = Some((velocity, scene.time));
                }
                _ => controller.landed(id),
            }
        }
        for id in controller.take_drops() {
            if let Some(actor) = self.actors.get_mut(&id) {
                actor.fall.get_or_insert((Vec3::ZERO, scene.time));
            }
        }
        self.climb_edges(scene, controller, player);
        self.fly_birds(world, scene, physics);
        self.fly_scanners(world, scene, physics, player);
        self.step_falls(world, scene, physics, player);
        for id in controller.jumpers() {
            if self.actors.get(&id).is_none_or(|a| a.fall.is_none()) {
                controller.landed(id);
            }
        }
    }
    /// The fast zombies' climbs starting and ending, and ClimbTouch for those on the way up.
    fn climb_edges(&mut self, scene: &Scene, controller: &Controller, player: PlayerState) {
        let climbers = controller.climbers();
        for (&id, actor) in self.actors.iter_mut() {
            let AiClass::Zombie(z) = &mut actor.class else {
                continue;
            };
            let climb = climbers.iter().find(|(c, _)| *c == id);
            if climb.is_some() != z.fast.climbing {
                z.fast_climb_edge(climb.is_some());
            }
            let Some(&(_, dismounting)) = climb else {
                continue;
            };
            if !z.fast.climb_touch_armed || player.dead || player.noclip {
                continue;
            }
            let hull = HullType::Human.bounds();
            let margin = Vec3::splat(CLIMB_TOUCH_MARGIN);
            let origin = scene.states[id].origin;
            let height = if player.crouched { 36. } else { 72. };
            let (lo, hi) = (
                player.feet - Vec3::new(16., 16., 0.),
                player.feet + Vec3::new(16., 16., height),
            );
            if overlaps(
                origin + hull.mins - margin,
                origin + hull.maxs + margin,
                lo,
                hi,
            ) {
                let away = (player.feet - origin).truncate().normalize_or_zero();
                self.player_hits.push(PlayerHit {
                    attacker: id,
                    damage: 0.,
                    punch: Vec3::ZERO,
                    impulse: away.extend(0.) * CLIMB_TOUCH_PUNCH,
                    fade: None,
                });
                z.fast.climb_touch_armed = false;
                z.fast.climb_touch = !dismounting;
            }
        }
    }
}
