//! Stages of the damage paths: what a lethal or splitting hit leaves behind.
use super::{npc_spawn_health, Attacker, Physics, Scene, World};
use crate::projectiles::Damage;
use glam::Vec3;
use modkit_core::{Entity, PropData};

/// The rest of OnTakeDamage_Alive: a lethal blast releases the crab (headless, not modeled yet),
/// then ShouldBecomeTorso / BecomeTorso.
pub(super) fn zombie_split(
    scene: &mut Scene,
    entity: &Entity,
    id: usize,
    damage: &Damage,
    amount: f32,
    max: f32,
) {
    let Some(kind) = crate::ai::zombie::Kind::for_class(entity.class()) else {
        return;
    };
    let (origin, health) = (scene.states[id].origin, scene.states[id].value);
    let time = scene.time;
    let roll = |salt| crate::fire::roll(id, time, salt);
    // IsChopped -> DieChopped (Ravenholm's blades): the slicer
    // sound, half the time a live crab, legs and a torso.
    let sliced = kind == crate::ai::zombie::Kind::Classic
        && entity.class() == "npc_zombie"
        && crate::ai::zombie::is_chopped(damage.kind, amount, max, health);
    if sliced {
        let force = damage.direction * (damage.amount * 300.).min(30000.) * (0.85 + 0.3 * roll(11));
        let rolls = [roll(41), roll(42), roll(43), roll(44), roll(45)];
        scene.zombie_chops += 1;
        scene.sounds.push(crate::sounds::SoundRequest {
            origin: Some(origin + Vec3::Z * 36.),
            .."E3_Phystown.Slicer".into()
        });
        scene.crab_releases.push(crate::ai::zombie::CrabRelease {
            entity: id,
            crab_force: force * 0.005,
            chopped: Some(crate::ai::zombie::chopped_forces(force, rolls)),
            live: roll(46) < 0.5,
            sliced: true,
        });
    }
    let (split, headless) = if sliced {
        (crate::ai::zombie::Split::None, false)
    } else {
        crate::ai::zombie::split(
            kind,
            entity.class() != "npc_zombie",
            false,
            (damage.kind, amount, max, health),
            roll(31) < 1. / 3.,
        )
    };
    // CalculateExplosiveDamageForce: the info's damage force.
    let force = damage.direction * (damage.amount * 300.).min(30000.) * (0.85 + 0.3 * roll(11));
    match split {
        crate::ai::zombie::Split::Torso { dies } => {
            scene.zombie_splits.push(crate::ai::zombie::TorsoSplit {
                entity: id,
                legs_force: force * 0.5,
                max_health: max * crate::ai::zombie::TORSO_HEALTH_FACTOR,
                dies: dies.then_some(force),
                ignite: (roll(32) < 0.5).then(|| 5. + 5. * roll(33)),
            });
        }
        crate::ai::zombie::Split::Chopped => {
            scene.zombie_chops += 1;
        }
        crate::ai::zombie::Split::None => {}
    }
    if headless {
        scene.zombie_crab_releases += 1;
        let rolls = [roll(41), roll(42), roll(43), roll(44), roll(45)];
        scene.crab_releases.push(crate::ai::zombie::CrabRelease {
            entity: id,
            crab_force: force * 0.25,
            chopped: (split == crate::ai::zombie::Split::Chopped)
                .then(|| crate::ai::zombie::chopped_forces(force, rolls)),
            live: false,
            sliced: false,
        });
    }
}

/// The death force of an NPC killed by a blast or a projectile.
pub(super) fn npc_death_force(scene: &mut Scene, id: usize, damage: &Damage) {
    // The ragdoll's nudge lands on m_nForceBone: the last trace
    // attack's solid, else the root (the server's default 0).
    let force_point = scene.npc_force_points.get(&id).copied();
    if damage.kind & crate::player_damage::DMG_BLAST != 0 {
        scene.npc_death_blasts.insert(id, damage.amount);
        // CalculateExplosiveDamageForce: the (falloff) damage x
        // ImpulseScale(75, 4), clamped to ImpulseScale(75, 400),
        // x U(0.85, 1.15); CalcDamageForceVector fudges it again:
        // x and y by U(0.85, 1.15), z x 1.15 when it points up.
        let time = scene.time;
        let fudge = |salt| 0.85 + 0.3 * crate::fire::roll(id, time, salt);
        let mut force = damage.direction * (damage.amount * 300.).min(30000.) * fudge(11);
        let scale = fudge(12);
        force.x *= scale;
        force.y *= scale;
        force.z *= if force.z > 0. { 1.15 } else { scale };
        scene.npc_death_forces.insert(id, (force, force_point));
    } else if let Some((force, at)) = scene.npc_hit_forces.remove(&id) {
        // Projectile hits carry their own damage force (bolts).
        scene.npc_death_forces.insert(id, (force, Some(at)));
    }
}

/// CBreakableProp::Break's explosion. `attacker` is the damage's.
pub(super) fn prop_explosion(
    scene: &mut Scene,
    physics: &Physics,
    entity: &Entity,
    id: usize,
    data: Option<&PropData>,
    attacker: Attacker,
) {
    // The explosion's attacker: m_hLastAttacker, else the physics
    // attacker within 2 s (retail FUN_1021ce50), else this damage's.
    // "Player allies get immunity from barrels exploded by the
    // player."
    let attacker =
        scene
            .prop_attackers
            .get(&id)
            .copied()
            .unwrap_or(match scene.physics_attacker(id, 2.) {
                Attacker::None => attacker,
                physics => physics,
            });
    let key = |k: &str| {
        entity
            .get(k)
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(0.)
    };
    let own = data.filter(|_| !entity.class().ends_with("_override"));
    let explode_damage = own
        .and_then(|d| d.explosive_damage)
        .unwrap_or_else(|| key("ExplodeDamage"));
    let explode_radius = own
        .and_then(|d| d.explosive_radius)
        .unwrap_or_else(|| key("ExplodeRadius"));
    let burst = own.is_some_and(|d| d.break_explode);
    let scale = entity
        .get("modelscale")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| *v > 0.)
        .unwrap_or(1.);
    if burst {
        // "Find and ignite all NPC's within the radius" around the
        // break origin, after the explosion (the host runs it once
        // the blast damage is applied).
        scene.prop_ignitions.push((
            scene.states[id].origin,
            (explode_radius * scale).trunc(),
            attacker,
        ));
    }
    if explode_damage > 0. || explode_radius > 0. {
        let magnitude = (explode_damage * scale).trunc();
        let radius = (explode_radius * scale).trunc();
        let radius = if radius > 0. {
            radius
        } else {
            (magnitude * 2.5).trunc()
        };
        let center = physics
            .entity_body(id)
            .and_then(|b| physics.center_of_mass(b))
            .unwrap_or(scene.states[id].origin);
        // PROPINTER_PHYSGUN_BREAK_EXPLODE: SF_ENVEXPLOSION_NOSOUND
        // and PropaneTank.Burst instead.
        scene
            .prop_explosions
            .push((center, magnitude, radius, burst, attacker));
    } else if burst {
        // No explode damage: ExplosionCreate(origin, 1, radius)
        // with its sound.
        let center = physics
            .entity_body(id)
            .and_then(|b| physics.center_of_mass(b))
            .unwrap_or(scene.states[id].origin);
        scene
            .prop_explosions
            .push((center, 1., 2., false, attacker));
    }
}

/// The pose and motion a broken prop's gibs start from.
pub(super) fn record_prop_break(world: &World, scene: &mut Scene, physics: &Physics, id: usize) {
    // PropBreakableCreateAll takes the prop's pose and motion.
    let body = physics.entity_body(id);
    if let (Some(body), Some(instance), Some(state)) = (
        body,
        world.model_instances.iter().find(|i| i.entity == Some(id)),
        body.and_then(|b| physics.body_state(b)),
    ) {
        let angular = state.rotation * (state.local_angular_degrees * std::f32::consts::PI / 180.);
        let center_of_mass = physics.center_of_mass(body).unwrap_or(state.position);
        scene.prop_breaks.push(crate::gibs::PropBreak {
            entity: id,
            model_key: instance.asset_key(),
            origin: state.position,
            rotation: state.rotation,
            velocity: state.velocity,
            angular_velocity: angular,
            center_of_mass,
            skin: instance.skin as i32,
        });
    }
}

/// A bullet or club's damage kind against a breakable.
pub(super) fn bullet_kind(ammo_type: &str, melee: bool) -> u32 {
    use crate::player_damage::{DMG_BUCKSHOT, DMG_BULLET, DMG_CLUB};
    if melee {
        DMG_CLUB
    } else if ammo_type.eq_ignore_ascii_case("Buckshot") {
        DMG_BULLET | DMG_BUCKSHOT
    } else {
        DMG_BULLET
    }
}

/// Takes a dead entity out of the world. A barnacle stays on its ceiling as a corpse and plays
/// its own death.
pub(super) fn remove_killed(scene: &mut Scene, e: &Entity, id: usize) {
    if e.class() == "npc_barnacle" {
        scene.states[id].corpse = true;
    } else {
        scene.states[id].killed = true;
        scene.states[id].visible = false;
    }
}
/// A killed entity's flame ends; an NPC's passes to its corpse.
pub(super) fn flame_out(scene: &mut Scene, id: usize, origin: Vec3, npc: bool) {
    if npc {
        let now = scene.time;
        scene.fires.burn_corpse(id, origin, now);
    }
    scene.fires.extinguish(id, origin);
}

/// A bullet kill's flame and crab. The tuples are the damage and force, then whether it was
/// melee and a headshot.
pub(super) fn bullet_crab_release(
    scene: &mut Scene,
    e: &Entity,
    id: usize,
    (damage, force): (f32, Vec3),
    (melee, headshot): (bool, bool),
) {
    let origin = scene.states[id].origin;
    flame_out(scene, id, origin, true);
    // ShouldReleaseHeadcrab for a bullet kill of a classic or poison zombie (or a
    // torso): CNPC_BaseZombie's, with the class's GetHeadcrabClassname/Model.
    if !melee
        && matches!(
            crate::ai::zombie::Kind::for_class(e.class()),
            Some(crate::ai::zombie::Kind::Classic | crate::ai::zombie::Kind::Poison)
        )
    {
        let max = scene
            .npc_max_health
            .get(&id)
            .copied()
            .unwrap_or_else(|| npc_spawn_health(scene, e));
        let threshold = (damage / max.max(1e-6)).min(1.);
        let release = crate::ai::zombie::bullet_release(headshot, threshold);
        if release != crate::ai::zombie::BulletRelease::No {
            scene.zombie_crab_releases += 1;
            scene.crab_releases.push(crate::ai::zombie::CrabRelease {
                entity: id,
                crab_force: force * 0.25,
                chopped: None,
                live: release == crate::ai::zombie::BulletRelease::Immediate,
                sliced: false,
            });
        }
    }
}

/// TraceAttack's record of where an NPC was hit, for its death force and ragdoll.
pub(super) fn record_hit_location(scene: &mut Scene, id: usize, hit: &crate::physics::RayHit) {
    scene.npc_force_points.insert(id, hit.position);
    scene.npc_hit_groups.insert(id, hit.hitgroup);
    match hit.bone {
        Some(bone) => scene.npc_force_bones.insert(id, bone),
        None => scene.npc_force_bones.remove(&id),
    };
}

/// A bullet or club's damage to a breakable, through the shared entity damage path. `at` is the
/// direction and the hit position.
pub(super) fn entity_damage(
    id: usize,
    amount: f32,
    kind: u32,
    (direction, origin): (Vec3, Vec3),
    attacker: Attacker,
) -> Damage {
    Damage {
        target: crate::projectiles::DamageTarget::Entity(id),
        amount,
        kind,
        dissolve: false,
        direction,
        origin,
        attacker,
    }
}

/// An NPC hit before its health was set starts from the spawn health.
pub(super) fn ensure_health(scene: &mut Scene, e: &Entity, id: usize) {
    if scene.states[id].value <= 0. {
        scene.states[id].value = npc_spawn_health(scene, e);
    }
}
