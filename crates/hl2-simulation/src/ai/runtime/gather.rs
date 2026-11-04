//! Stages of `AiRuntime::think` that only need the actor and what surrounds it: the zombies'
//! door and swat conditions, the soldiers' grenade and alt-fire probes, move and shoot.
use super::super::conditions::{COND_NO_PRIMARY_AMMO, COND_SEE_ENEMY};
use super::super::host::{PhysicsSight, SceneMotor};
use super::super::humanoid::{self, Humanoid};
use super::super::relationships::{classify, Class};
use super::super::schedule::{Context, Schedules};
use super::super::senses::{self, SightWorld, Viewer};
use super::super::state::NpcState;
use super::{
    swat_candidates, vec_check_throw, vec_check_toss, zombie, AiActor, AiClass, Diagnostics,
    PlayerState, PLAYER,
};
use crate::entities::Scene;
use crate::npc_weapons::{self, FireSpec, NpcWeaponDef};
use crate::physics::Physics;
use glam::Vec3;
use modkit_core::World;
use std::collections::BTreeMap;

/// What one think of `id` sees around it.
#[derive(Clone, Copy)]
pub(super) struct ThinkView<'a> {
    pub id: usize,
    pub now: f64,
    pub origin: Vec3,
    pub eye: Vec3,
    pub facing: Vec3,
    pub world: &'a World,
    pub scene: &'a Scene,
    pub physics: &'a Physics,
    pub sight: &'a PhysicsSight<'a>,
    pub viewer: &'a Viewer,
    pub player: PlayerState,
    pub precriminal: bool,
}

/// The zombies' GatherConditions.
pub(super) fn zombie_conditions(actor: &mut AiActor, v: &ThinkView<'_>) {
    let ThinkView {
        id,
        now,
        origin,
        eye,
        world,
        scene,
        physics,
        sight,
        player,
        ..
    } = *v;
    // CZombie::GatherConditions: the blocking door opened (or is opening): COND_DOOR_OPENED
    // and forget it; still closed: COND_BLOCKED_BY_DOOR.
    if let AiClass::Zombie(z) = &mut actor.class {
        actor.npc.conditions.clear(zombie::COND_BLOCKED_BY_DOOR);
        actor.npc.conditions.clear(zombie::COND_DOOR_OPENED);
        if let Some((door, _)) = z.blocking_door {
            if scene.states.get(door).is_some_and(|s| s.door_closed()) {
                actor.npc.conditions.set(zombie::COND_BLOCKED_BY_DOOR);
            } else {
                actor.npc.conditions.set(zombie::COND_DOOR_OPENED);
                z.blocking_door = None;
            }
        }
    }
    // CNPC_BaseZombie::GatherConditions: in combat, every 2 s while it has no object,
    // FindNearestPhysicsObject; COND_ZOMBIE_CAN_SWAT_ATTACK while it has one, m_flNextSwat
    // has passed and it sees the enemy. The object's center follows its body.
    if let AiClass::Zombie(z) = &mut actor.class {
        z.physics_ent = z.physics_ent.and_then(|(object, _)| {
            physics
                .entity_body(object)
                .and_then(|body| physics.bounds_center(body))
                .map(|center| (object, center))
        });
        let enemy_feet = actor.npc.enemy.map(|e| {
            if e == PLAYER {
                player.feet
            } else {
                scene.states[e].origin
            }
        });
        if let (NpcState::Combat, Some(enemy_feet), true) = (actor.npc.state, enemy_feet, z.swats())
        {
            if now >= z.next_swat_scan && z.physics_ent.is_none() {
                // The human hull: 72 units tall, knees at a quarter of it.
                if let Some(half) = zombie::swat_search_box(origin, enemy_feet, 72.) {
                    let candidates = swat_candidates(physics, world, sight, id, origin, eye, half);
                    z.physics_ent = zombie::find_nearest_physics_object(
                        origin,
                        enemy_feet,
                        origin.z + 18.,
                        eye.z,
                        &candidates,
                    )
                    .and_then(|object| {
                        candidates
                            .iter()
                            .find(|c| c.entity == object)
                            .map(|c| (object, c.center))
                    });
                }
                z.next_swat_scan = now + zombie::SWAT_SCAN_INTERVAL;
            }
        }
        if z.physics_ent.is_some() && now >= z.next_swat && actor.npc.conditions.has(COND_SEE_ENEMY)
        {
            actor
                .npc
                .conditions
                .set(zombie::COND_ZOMBIE_CAN_SWAT_ATTACK);
        } else {
            actor
                .npc
                .conditions
                .clear(zombie::COND_ZOMBIE_CAN_SWAT_ATTACK);
        }
    }
}

/// What the soldiers and citizens probe each think for their schedules to read.
pub(super) fn humanoid_probes(
    actor: &mut AiActor,
    v: &ThinkView<'_>,
    diagnostics: &mut Diagnostics,
) {
    let ThinkView {
        id,
        now,
        origin,
        scene,
        player,
        ..
    } = *v;
    // CNPC_Combine grenades without a squad: CanGrenadeEnemy(false) (the flush, at the
    // last seen position) and CanGrenadeEnemy() (the last known position, e.g. the 20%
    // grenade before reloading) -> CanThrowGrenade -> CheckCanThrowGrenade (VecCheckThrow
    // at 650 in view and visible, else a high VecCheckToss with 64 units of headroom).
    // Probed here without side effects; the class applies them when it asks. Squad
    // slots and the 4-unit hull of the throw traces are not modeled.
    let moving = scene.states[id].is_moving();
    let enemy_now = actor.npc.enemy.map(|e| {
        if e == PLAYER {
            player.feet
        } else {
            scene.states[e].origin
        }
    });
    let seen_enemy = actor.npc.conditions.has(COND_SEE_ENEMY);
    let enemy = actor.npc.enemy;
    let memory = actor
        .npc
        .enemy
        .and_then(|e| actor.npc.enemies.find(Some(e)))
        .map(|m| (m.time_last_seen, m.last_seen_position, m.last_known));
    if let AiClass::Humanoid(h) = &mut actor.class {
        // CWeaponRPG state the citizen's RPG schedules read (m_hMissile).
        let rpg = actor.weapon.as_ref().filter(|w| w.class == "weapon_rpg");
        h.rpg = rpg.is_some();
        h.missile_out = rpg.is_some_and(|w| w.missile_out);
        h.shoot_position = origin + Vec3::Z * 55.;
        // The previous think's CanGrenadeEnemy outcomes.
        for outcome in h.grenade_outcomes.drain(..) {
            *diagnostics
                .grenade_checks
                .entry(outcome.into())
                .or_default() += 1;
        }
        h.throw_probes = Default::default();
        h.flush_window = false;
        // CanAltFireEnemy(true)'s trace: the enemy (the player or a player ally) at its
        // last known position plus 0.75 of its view offset, a 24-unit hull from the
        // shoot position along MASK_SHOT; valid when it travels >= 65% and > 128 units.
        h.alt_fire_probe = None;
        if h.elite {
            alt_fire_probe(h, enemy, memory, v);
        }
        if let (true, Some((last_time, last_seen, last_known))) =
            (h.kind == humanoid::Kind::Combine && h.grenades > 0, memory)
        {
            h.flush_window = !seen_enemy
                && now - last_time <= 3.
                && enemy_now.is_some_and(|p| p.distance(last_seen) <= 256.);
            let probe = |target: Vec3| throw_probe(v, target, moving);
            if now >= h.next_grenade_check {
                if h.flush_window {
                    h.throw_probes.last_seen = Some(probe(last_seen));
                }
                h.throw_probes.last_known = Some(probe(last_known));
            }
        }
    }
}

/// CheckCanThrowGrenade toward `target`, without side effects.
fn throw_probe(v: &ThinkView<'_>, target: Vec3, moving: bool) -> humanoid::ThrowProbe {
    let ThinkView {
        id,
        now,
        origin,
        eye,
        facing,
        physics,
        sight,
        viewer,
        ..
    } = *v;
    if !(128. ..=1024.).contains(&target.distance(origin)) {
        return humanoid::ThrowProbe::OutOfRange;
    }
    if moving {
        return humanoid::ThrowProbe::Moving;
    }
    let in_view = senses::in_view_cone(viewer, target) && sight.visible(eye, target, &[id]);
    let bits = ((now * 1000.) as u64) ^ (id as u64).wrapping_mul(0x9e37_79b9);
    let jitter = [0u64, 1, 2, 3].map(|i| {
        (bits
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(i.wrapping_mul(1_442_695_040_888_963_407))
            >> 40) as f32
            / (1u64 << 24) as f32
    });
    let toss = if in_view {
        vec_check_throw(physics, id, eye, target, 650.)
    } else if physics
        .projectile_ray(eye, eye + Vec3::Z * 64., &[id])
        .is_none()
    {
        vec_check_toss(physics, id, eye, target, facing, jitter)
    } else {
        None
    };
    match toss {
        Some(velocity) => humanoid::ThrowProbe::Ready { velocity, in_view },
        None => humanoid::ThrowProbe::Blocked { in_view },
    }
}

/// CanAltFireEnemy's probe for an elite soldier.
fn alt_fire_probe(
    h: &mut Humanoid,
    enemy: Option<usize>,
    memory: Option<(f64, Vec3, Vec3)>,
    v: &ThinkView<'_>,
) {
    let ThinkView {
        id,
        origin,
        world,
        physics,
        precriminal,
        ..
    } = *v;
    let ally = |e: usize| {
        e == PLAYER
            || world.entities.get(e).is_some_and(|ent| {
                matches!(
                    classify(ent.class(), precriminal),
                    Class::PlayerAlly | Class::PlayerAllyVital
                )
            })
    };
    if let (Some(enemy), Some((_, _, last_known))) = (enemy, memory) {
        if ally(enemy) {
            let target = last_known + Vec3::Z * 64. * 0.75;
            let start = origin + Vec3::Z * 55.;
            let length = start.distance(target);
            let travelled = physics
                .projectile_sweep(
                    start,
                    target,
                    crate::physics::ProjectileHull::Box(Vec3::splat(12.)),
                    &[id],
                )
                .map_or(length, |hit| hit.position.distance(start));
            if length > 0. && travelled / length >= 0.65 && travelled > 128. {
                h.alt_fire_probe = Some(target);
            }
        }
    }
}

// StartTaskOverlay / RunTaskOverlay: move and shoot (bits_CAP_MOVE_SHOOT: the
// soldier, the companions and non-efficient cops).
pub(super) fn move_and_shoot(
    actor: &mut AiActor,
    motor: &mut SceneMotor<'_>,
    schedules: &Schedules,
    weapon_defs: &BTreeMap<String, NpcWeaponDef>,
    ctx: &Context,
    id: usize,
) {
    let (now, origin) = (ctx.now, ctx.origin);
    let schedule = actor.npc.schedule.clone();
    let task_name = schedule.as_deref().and_then(|s| {
        schedules
            .0
            .get(s)
            .and_then(|sc| sc.tasks.get(actor.npc.task))
            .map(|(t, _)| t.clone())
    });
    let combine = matches!(&actor.class, AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine);
    // CNPC_Combine::ShouldMoveAndShoot: not while fleeing a danger sound.
    let allowed = !(combine
        && matches!(
            schedule.as_deref(),
            Some(
                "SCHED_TAKE_COVER_FROM_BEST_SOUND"
                    | "SCHED_COMBINE_TAKE_COVER_FROM_BEST_SOUND"
                    | "SCHED_COMBINE_RUN_AWAY_FROM_BEST_SOUND"
            )
        ));
    let ranged = ranged_fire(actor, weapon_defs);
    if combine && actor.move_regulator.is_none() {
        if let Some(spec) = ranged.as_ref() {
            actor.move_regulator = Some(npc_weapons::ShotRegulator::for_weapon(
                spec,
                npc_weapons::RegulatorRule::Weapon,
                now,
                0x9e37_79b9_7f4a_7c15 ^ id as u64,
            ));
        }
    }
    if combine {
        stop_aim_to_reload(actor, schedule.as_deref(), ctx);
    }
    let regulator = if combine {
        actor.move_regulator.as_mut()
    } else {
        actor.npc.shot_regulator.as_mut()
    };
    let moving = motor.goal.is_some() && motor.scene.states[id].is_moving();
    let has_aim = motor.controller.has_move_aim(id);
    let out = actor.move_shoot.think(
        schedule
            .as_deref()
            .zip(task_name.as_deref())
            .map(|(s, t)| (s, t, actor.npc.task)),
        allowed,
        (
            actor.npc.state == NpcState::Script,
            ranged.is_some(),
            has_aim,
        ),
        moving,
        actor.npc.conditions,
        actor.npc.enemy.is_some(),
        regulator,
        now,
    );
    if let Some(aim) = out.aim {
        motor.controller.set_move_aim(id, aim);
    }
    if out.shoot {
        // OnRangeAttack1: OnFiredWeapon (and the class's attack time).
        let distance = ctx.enemy.map(|(_, p)| p.distance(origin));
        let regulator = if combine {
            actor.move_regulator.as_mut()
        } else {
            actor.npc.shot_regulator.as_mut()
        };
        if let Some(r) = regulator {
            r.on_range_attack1(now, distance);
        }
        if let AiClass::Humanoid(h) = &mut actor.class {
            h.last_attack = now;
        }
        actor.move_fire += 1;
    }
    let lkp = ctx.enemy_lkp.or(ctx.enemy.map(|(_, p)| p));
    motor.scene.states[id].ai_move_facing = match (out.face_enemy, lkp) {
        (true, Some(point)) => Some((point, now + 0.8)),
        _ => motor.scene.states[id]
            .ai_move_facing
            .filter(|(_, until)| now < *until),
    };
}

/// The active weapon's firing data, unless it is the RPG.
fn ranged_fire(actor: &AiActor, weapon_defs: &BTreeMap<String, NpcWeaponDef>) -> Option<FireSpec> {
    actor
        .weapon
        .as_ref()
        .and_then(|w| {
            weapon_defs
                .get(&w.class)
                .cloned()
                .or_else(|| npc_weapons::builtin(&w.class))
        })
        .and_then(|d| d.fire)
        .filter(|_| {
            actor
                .weapon
                .as_ref()
                .is_some_and(|w| w.class != "weapon_rpg")
        })
}

/// m_flStopMoveShootTime: hiding to reload (or empty) stops aiming at the enemy
/// after U(0.4, 0.6) s for 5 s, so the soldier faces the way it runs.
fn stop_aim_to_reload(actor: &mut AiActor, schedule: Option<&str>, ctx: &Context) {
    let now = ctx.now;
    let reloading = schedule == Some("SCHED_COMBINE_HIDE_AND_RELOAD")
        || actor.npc.conditions.has(COND_NO_PRIMARY_AMMO);
    if !reloading {
        actor.stop_move_shoot = None;
    } else if actor.stop_move_shoot.is_none() {
        actor.stop_move_shoot = Some(now + 0.4 + 0.2 * f64::from(ctx.random));
    }
    if actor.stop_move_shoot.is_some_and(|t| now >= t) {
        actor
            .move_shoot
            .suspend(5., actor.move_regulator.as_mut(), now);
        actor.stop_move_shoot = Some(f64::INFINITY);
    }
}
