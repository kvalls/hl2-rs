//! Headcrabs and zombies in the AI think: GatherAttackConditions' class parts (the crab's
//! leap, zombie claws, the poison zombie's crab throws, the fast zombie's leap) and the
//! zombies' per-think sounds.
use super::super::conditions::{COND_CAN_RANGE_ATTACK1, COND_CAN_RANGE_ATTACK2, COND_NEW_ENEMY};
use super::super::host::PhysicsSight;
use super::super::senses::SightWorld;
use super::super::state::NpcState;
use super::{center_z, headcrab, zombie, AiActor, AiClass, PlayerState, PLAYER};
use crate::entities::Scene;
use crate::npc_probe::{NpcCollisionWorld, Query};
use crate::physics::Physics;
use glam::Vec3;
use modkit_core::World;

/// What GatherAttackConditions sees of the enemy this think (positions are world space;
/// `facing` is the NPC's forward vector).
pub(super) struct AttackView {
    pub id: usize,
    pub enemy: usize,
    pub now: f64,
    pub origin: Vec3,
    pub center: Vec3,
    pub eye: Vec3,
    pub facing: Vec3,
    pub t_feet: Vec3,
    pub t_center: Vec3,
    pub t_eye: Vec3,
    /// IsMoving: the NPC's navigation goal is active.
    pub moving: bool,
    /// The enemy's FInViewCone(me); None without an enemy.
    pub enemy_sees_me: Option<bool>,
}
/// IsMoving: `actor`'s navigation goal is active.
pub(super) fn goal_active(actor: &AiActor, controller: &crate::npc::Controller) -> bool {
    use crate::npc::GoalState::{Blocked, Moving};
    let status = actor.goal.and_then(|key| controller.status(key));
    status.is_some_and(|s| matches!(s, Moving | Blocked(_)))
}
impl AttackView {
    /// CAI_BaseNPC::EnemyDistance: center to center less the height the two boxes can
    /// overlap (half the sum of their heights).
    fn enemy_distance(&self, heights: f32) -> f32 {
        let mut delta = self.t_center - self.center;
        let overlap = heights * 0.5;
        delta.z = if delta.z > overlap {
            delta.z - overlap
        } else if delta.z < -overlap {
            delta.z + overlap
        } else {
            0.
        };
        delta.length()
    }
    /// GatherAttackConditions' flDot: origin to origin against the facing, in 2D.
    fn dot(&self) -> f32 {
        (self.t_feet - self.origin)
            .truncate()
            .normalize_or_zero()
            .dot(self.facing.truncate().normalize_or_zero())
    }
    fn hull(&self, world: &World, default: crate::ai_hull::HullType) -> crate::npc_probe::Hull {
        world
            .entities
            .get(self.id)
            .and_then(|e| crate::ai_hull::class_hull(e.class()))
            .unwrap_or(default)
            .bounds()
    }
}

/// The class parts of GatherAttackConditions for headcrabs and zombies.
pub(super) fn attack_conditions(
    actor: &mut AiActor,
    v: &AttackView,
    world: &World,
    scene: &Scene,
    physics: &Physics,
) {
    let sight = PhysicsSight { physics, world };
    crab_leap(actor, v, world, physics, &sight);
    claws(actor, v, world);
    poison_ranges(actor, v, world, scene, &sight);
    fast_leap(actor, v, physics);
}

/// CBaseHeadcrab::RangeAttack1Conditions (EnemyDistance between the centers, less the
/// height the two boxes can overlap).
fn crab_leap(
    actor: &mut AiActor,
    v: &AttackView,
    world: &World,
    physics: &Physics,
    sight: &PhysicsSight,
) {
    let on_ground = actor.on_ground();
    let AiClass::Headcrab(crab) = &mut actor.class else {
        return;
    };
    // An enemy whose eyes are more than 36 above my origin needs headroom: the crab's
    // hull from 1 unit up, 8 units toward those eyes, meeting anything but the enemy (the
    // world included: a step's face) is COND_TOO_CLOSE_TO_ATTACK, so the crab closes in
    // instead of leaping.
    let high_blocked = v.t_eye.z - 36. > v.origin.z && {
        let hull = v.hull(world, crate::ai_hull::HullType::Tiny);
        let end = v.origin + (v.t_eye - v.origin).normalize_or_zero() * 8.;
        let excluded = [v.id];
        let hit = physics.npc_trace_hull(
            v.origin + Vec3::Z,
            end,
            hull,
            Query {
                excluded_entities: &excluded,
                ..Default::default()
            },
        );
        (hit.trace.fraction < 1. || hit.trace.start_solid) && hit.entity != Some(v.enemy)
    };
    let check = headcrab::LeapCheck {
        ready: v.now >= crab.next_attack,
        on_ground,
        dot: v.dot(),
        distance: v.enemy_distance(72. + 24.),
        clear: sight.visible(v.eye, v.t_eye, &[v.id, v.enemy]),
        high_blocked,
    };
    let condition = headcrab::range_condition(check);
    crab.last_check = Some((check.distance, check.dot, condition.map(|c| c.0 as i32)));
    if let Some(c) = condition {
        actor.npc.conditions.set(c);
    }
}

/// CNPC_BaseZombie::MeleeAttack1Conditions: within the claw reach (origin to origin),
/// facing within 0.7, and the cube hull swept along the facing meets the enemy (an NPC or
/// breakable in the way is not modeled); CFastZombie's only on the ground once
/// m_flNextMeleeAttack has passed.
fn claws(actor: &mut AiActor, v: &AttackView, world: &World) {
    let on_ground = actor.on_ground();
    let AiClass::Zombie(z) = &mut actor.class else {
        return;
    };
    let reach = z.kind.claw_reach();
    let to = v.t_feet - v.origin;
    let forward = v.facing.truncate().normalize_or_zero().extend(0.);
    let dot = to.truncate().normalize_or_zero().dot(forward.truncate());
    let end = v.center + forward * reach;
    let half = if v.enemy == PLAYER { 16. } else { 13. };
    let cube = Vec3::splat(v.hull(world, crate::ai_hull::HullType::Tiny).maxs.x);
    let reaches = super::super::metropolice::segment_hits_box(
        v.center,
        end,
        v.t_feet + Vec3::new(-half, -half, 0.) - cube,
        v.t_feet + Vec3::new(half, half, 72.) + cube,
    );
    let distance = to.length() * zombie::CLAW_REACH / reach;
    let condition = zombie::melee_condition(distance, dot, reaches);
    let condition = if z.kind == zombie::Kind::Fast {
        let ready = v.now >= z.fast.next_melee_attack;
        zombie::fast::melee_condition(condition, on_ground, ready)
    } else {
        Some(condition)
    };
    if let Some(c) = condition {
        actor.npc.conditions.set(c);
    }
}

/// CNPC_PoisonZombie::RangeAttack1/2Conditions (EnemyDistance; a CAN condition needs the
/// line from the eyes, GatherAttackConditions' weapon LOS).
fn poison_ranges(
    actor: &mut AiActor,
    v: &AttackView,
    world: &World,
    scene: &Scene,
    sight: &PhysicsSight,
) {
    let z = match &mut actor.class {
        AiClass::Zombie(z) if z.kind == zombie::Kind::Poison && !z.torso => z,
        _ => return,
    };
    if z.nest.is_none() {
        z.nest = Some(std::array::from_fn(|i| {
            scene.bodygroup(world, v.id, zombie::BODYGROUP_NEST_BASE + i) == Some(1)
        }));
    }
    let los = sight.visible(v.eye, v.t_eye, &[v.id, v.enemy]);
    for condition in z.poison_range_conditions(v.now, v.enemy_distance(144.), v.dot()) {
        let Some(c) = condition else { continue };
        if !matches!(c, COND_CAN_RANGE_ATTACK1 | COND_CAN_RANGE_ATTACK2) || los {
            actor.npc.conditions.set(c);
        }
    }
}

/// CFastZombie::RangeAttack1Conditions: the leap. Its "unless he's facing me" test casts any
/// enemy to CBasePlayer, so it holds for an NPC enemy's view cone too.
fn fast_leap(actor: &mut AiActor, v: &AttackView, physics: &Physics) {
    let on_ground = actor.on_ground();
    let z = match &mut actor.class {
        AiClass::Zombie(z) if z.kind == zombie::Kind::Fast && !z.torso => z,
        _ => return,
    };
    if std::mem::take(&mut z.fast.climb_touch) {
        actor
            .npc
            .conditions
            .set(zombie::COND_FASTZOMBIE_CLIMB_TOUCH);
    }
    let seen = v.enemy_sees_me.unwrap_or(false);
    // A 32-unit cube from my center halfway to the enemy's center (MASK_NPCSOLID).
    let clear = || {
        let half = crate::npc_probe::Hull {
            mins: Vec3::splat(-16.),
            maxs: Vec3::splat(16.),
        };
        let end = v.center + (v.t_center - v.center) * 0.5;
        let hit = physics.npc_trace_hull(
            v.center,
            end,
            half,
            Query {
                excluded_entities: &[v.id],
                ..Default::default()
            },
        );
        hit.trace.fraction >= 1. && !hit.trace.start_solid
    };
    let mut check = zombie::fast::LeapCheck {
        on_ground,
        ready: v.now >= z.fast.next_attack,
        height: (v.t_feet.z - v.origin.z).abs(),
        distance: v.enemy_distance(144.),
        dot: v.dot(),
        moving: v.moving,
        seen,
        clear: false,
    };
    // The trace only when everything else allows the leap.
    let unblocked = zombie::fast::LeapCheck {
        clear: true,
        ..check
    };
    check.clear =
        zombie::fast::range_condition(unblocked) == Some(COND_CAN_RANGE_ATTACK1) && clear();
    if let Some(c) = zombie::fast::range_condition(check) {
        actor.npc.conditions.set(c);
    }
}

/// CNPC_PoisonZombie's per-think parts: its pain and crab throw timing, and its breathing.
pub(super) fn poison_stage(
    actor: &mut AiActor,
    id: usize,
    world: &World,
    scene: &Scene,
    player: &PlayerState,
    now: f64,
) {
    let new_enemy = actor.npc.conditions.has(COND_NEW_ENEMY);
    let largest_hit = std::mem::take(&mut actor.largest_hit);
    let enemy = actor.npc.enemy;
    let AiClass::Zombie(z) = &mut actor.class else {
        return;
    };
    if z.kind == zombie::Kind::Poison {
        let max_health = scene
            .npc_max_health
            .get(&id)
            .copied()
            .unwrap_or_else(|| crate::gameplay::npc_spawn_health(scene, &world.entities[id]))
            .max(1.);
        z.poison_think(now, new_enemy, largest_hit / max_health);
        let origin = scene.states[id].origin;
        let enemy_distance = enemy.and_then(|e| {
            let position = if e == PLAYER {
                Some(player.feet)
            } else {
                scene.states.get(e).map(|s| s.origin)
            };
            position.map(|p| (p - origin).length())
        });
        z.breathe(enemy_distance);
    }
}

/// The zombie's sounds this think (m_iMoanSound from the zombies spawned before it; the
/// enemy's distance between centers less the hull width for the fast zombie's moan).
pub(super) fn think_sounds(
    actor: &mut AiActor,
    id: usize,
    world: &World,
    scene: &Scene,
    player: &PlayerState,
    now: f64,
) {
    let center = scene.states[id].origin + Vec3::Z * center_z(world, id);
    let enemy_distance = actor.npc.enemy.map(|e| {
        let target = if e == PLAYER {
            player.center()
        } else {
            scene.states[e].origin + Vec3::Z * center_z(world, e)
        };
        (target - center).length() - 26.
    });
    let state: NpcState = actor.npc.state;
    let AiClass::Zombie(z) = &mut actor.class else {
        return;
    };
    // m_iMoanSound = g_numZombies at Spawn: map zombies spawn in entity order.
    if z.moan_index.is_none() {
        z.moan_index = Some(
            world.entities[..id.min(world.entities.len())]
                .iter()
                .filter(|e| zombie::is_zombie_class(e.class()))
                .count(),
        );
    }
    z.think_sounds(now, state, enemy_distance);
}

/// CFastZombie::LeapAttackTouch when a falling fast zombie touched a solid this step (the
/// player, an NPC, a wall or the floor it lands on): the horizontal motion stops and the
/// leap's claw swings.
pub(super) fn leap_touch(
    class: &mut AiClass,
    velocity: &mut Vec3,
    center: Vec3,
    scene: &Scene,
    id: usize,
) {
    let AiClass::Zombie(z) = class else { return };
    if !z.fast.leap_touch {
        return;
    }
    velocity.x = 0.;
    velocity.y = 0.;
    let forward = scene.states[id].rotation * Vec3::X;
    z.fast_leap_touch(center, forward.y.atan2(forward.x).to_degrees());
}
/// The last 16 landings of step falls (entity, height, ground entity, stuck start, the
/// step's fraction), for the report.
pub(super) fn record_landing(
    diagnostics: &mut super::Diagnostics,
    (id, z, ground, stuck): (usize, f32, Option<usize>, bool),
    fraction: f32,
) {
    let landings = &mut diagnostics.fall_landings;
    if landings.len() >= 16 {
        landings.remove(0);
    }
    landings.push((id, z, ground, stuck, fraction));
}
