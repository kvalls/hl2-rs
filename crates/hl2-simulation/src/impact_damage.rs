//! Physics impact damage to NPCs: CBaseCombatCharacter::VPhysicsShadowCollision ->
//! CalculatePhysicsImpactDamage (SDK physics_impact_damage.cpp) with the NPC table. The
//! retail server.dll table (private physics-research-20261011, damage registry "npc")
//! has the same entries and thresholds. The angular (slice) path runs for
//! FVPHYSICS_DMG_SLICE props (spin_zaxis prop data or Damagetype 1: the sawblade);
//! FVPHYSICS_HEAVY_OBJECT, FVPHYSICS_NO_IMPACT_DMG/NO_NPC_IMPACT_DMG, dissolve and
//! floating NPCs are not set by anything here yet.
use crate::{
    entities::Scene,
    physics::PhysicsCollision,
    projectiles::{Attacker, Damage, DamageTarget},
};
use glam::Vec3;
use modkit_core::World;

/// impactdamagetable_t (npc): linear (speed^2 threshold, damage) entries.
const NPC_LINEAR: [(f32, f32); 5] = [
    (150. * 150., 5.),
    (250. * 250., 10.),
    (350. * 350., 50.),
    (500. * 500., 100.),
    (1000. * 1000., 500.),
];
/// npcAngularTable: (DotProductAbs(inertia, local deg/s) threshold, damage).
const NPC_ANGULAR: [(f32, f32); 4] = [
    (100. * 100., 10.),
    (150. * 150., 25.),
    (200. * 200., 50.),
    (250. * 250., 500.),
];
const MIN_SPEED_SQR: f32 = 24. * 24.;
/// 360 deg/s squared: the least spin that slices.
const MIN_ROT_SPEED_SQR: f32 = 360. * 360.;
const MIN_MASS: f32 = 2.;
const SMALL_MASS_MAX: f32 = 5.;
const SMALL_MASS_CAP: f32 = 5.;
const SMALL_MASS_MIN_SPEED_SQR: f32 = 36. * 36.;
const LARGE_MASS_MIN: f32 = 500.;
const LARGE_MASS_SCALE: f32 = 4.;
const LARGE_MASS_FALLING_SCALE: f32 = 5.;
/// CBaseCombatCharacter m_impactEnergyScale (keyvalue physdamagescale, default 1). The
/// d1_trainstation_02 plaza cops set 0: native thrown crates break on them and only
/// make them flinch (oracle T015556Z).
pub fn energy_scale(entity: &modkit_core::Entity) -> f32 {
    entity
        .get("physdamagescale")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v >= 0.)
        .unwrap_or(1.)
}

/// ReadDamageTable: the entry before the first threshold above `energy`.
fn read_table(table: &[(f32, f32)], energy: f32) -> f32 {
    let i = table
        .iter()
        .take_while(|(threshold, _)| energy >= *threshold)
        .count();
    if i > 0 {
        table[i - 1].1
    } else {
        0.
    }
}

/// The colliding object as the NPC's shadow sees it. `angular` (speed in deg/s,
/// DotProductAbs(inertia, local angular velocity)) only for FVPHYSICS_DMG_SLICE objects.
#[derive(Clone, Copy, Debug, Default)]
pub struct Impactor {
    pub mass: f32,
    pub pre: Vec3,
    pub post: Vec3,
    pub angular: Option<(f32, f32)>,
}
/// FVPHYSICS_DMG_SLICE: CPhysicsProp::Spawn sets it for m_damageType 1 (keyvalue
/// Damagetype) and "fixes up noncompliant blades" with spin_zaxis prop data.
pub fn slices(world: &World, entity: usize) -> bool {
    let Some(e) = world.entities.get(entity) else {
        return false;
    };
    if !e.class().starts_with("prop_physics") {
        return false;
    }
    e.get("Damagetype").is_some_and(|v| v.trim() == "1")
        || world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(entity))
            .and_then(|i| world.model_prop_data.get(&i.asset_key()))
            .is_some_and(|d| d.spin_z)
}

/// CalculatePhysicsImpactDamage for an NPC shadow of `npc_mass` (it does not lose
/// speed itself) hit by `other`; allowStaticDamage false. Returns the damage and
/// whether the spin added DMG_SLASH.
pub fn npc_damage(npc_mass: f32, energy_scale: f32, other: Impactor) -> (f32, bool) {
    let speed_sqr = other.pre.length_squared();
    let (angular_speed, momentum) = other.angular.unwrap_or_default();
    let angular_sqr = angular_speed * angular_speed;
    if other.mass < MIN_MASS
        || (other.mass < SMALL_MASS_MAX && speed_sqr < SMALL_MASS_MIN_SPEED_SQR)
        || (speed_sqr < MIN_SPEED_SQR && angular_sqr < MIN_ROT_SPEED_SQR)
        || npc_mass <= 0.
    {
        return (0., false);
    }
    // Spin damage (never from slowly spinning objects).
    let spin = if angular_sqr > MIN_ROT_SPEED_SQR {
        read_table(&NPC_ANGULAR, momentum * energy_scale)
    } else {
        0.
    };
    let delta = other.pre.length() - other.post.length();
    let mut energy = delta * delta * other.mass;
    if other.mass >= LARGE_MASS_MIN {
        energy *= LARGE_MASS_SCALE;
        let dz = other.pre.z - other.post.z;
        if delta > 0. && dz < 0. && other.pre.z < 0. {
            let factor = (dz / delta).abs();
            energy *= 1. + factor * (LARGE_MASS_FALLING_SCALE - 1.);
        }
    }
    energy *= energy_scale / npc_mass;
    let damage = spin + read_table(&NPC_LINEAR, energy);
    // Small dynamic objects never do more than the cap.
    let damage = if other.mass < SMALL_MASS_MAX {
        damage.clamp(0., SMALL_MASS_CAP)
    } else {
        damage
    };
    (damage, spin > 0.)
}

/// Queue DMG_CRUSH damage for NPCs struck by moving physics objects (resolved collision
/// events of the previous step; applied
/// with the other entity damage next tick, like the deferred damage queue). Objects the
/// player holds do no damage (bDamageFromHeldObjects false); NPCs ignore what they
/// stand on (not tracked here).
pub fn queue_npc_damage(
    collisions: &[PhysicsCollision],
    world: &World,
    scene: &mut Scene,
    held: &dyn Fn(usize) -> bool,
) {
    for event in collisions {
        for index in 0..2 {
            let other = 1 - index;
            let Some(npc) = event.entities[index] else {
                continue;
            };
            let Some(entity) = world.entities.get(npc) else {
                continue;
            };
            if !entity.class().starts_with("npc_")
                || scene.states.get(npc).is_none_or(|s| s.killed)
                || event.masses[other] <= 0.
                || event.entities[other].is_some_and(|id| id == npc || held(id))
            {
                continue;
            }
            let Some(mass) = world
                .model_instances
                .iter()
                .find(|i| i.entity == Some(npc))
                .and_then(|i| world.model_mass.get(&i.asset_key()))
            else {
                continue;
            };
            // An AddOutput "physdamagescale <v>" at runtime overrides the map keyvalue.
            let scale = scene
                .keyvalue_overrides
                .get(&(npc, "physdamagescale".to_string()))
                .and_then(|v| v.trim().parse::<f32>().ok())
                .filter(|v| v.is_finite() && *v >= 0.)
                .unwrap_or_else(|| energy_scale(entity));
            let slicer = event.entities[other].is_some_and(|id| slices(world, id));
            let (damage, slash) = npc_damage(
                *mass,
                scale,
                Impactor {
                    mass: event.masses[other],
                    pre: event.pre_velocities[other],
                    post: event.post_velocities[other],
                    angular: slicer.then_some(event.angular[other]),
                },
            );
            if damage > 0. {
                // The object that hit, or the player within 1 s of picking it up or
                // dropping it (HasPhysicsAttacker(1.0), retail FUN_100f58e0).
                let attacker = event.entities[other].map_or(Attacker::None, |prop| {
                    match scene.physics_attacker(prop, 1.) {
                        Attacker::None => Attacker::Entity(prop),
                        player => player,
                    }
                });
                scene.entity_damage.push(Damage {
                    target: DamageTarget::Entity(npc),
                    amount: damage,
                    kind: crate::player_damage::DMG_CRUSH
                        | if slash {
                            crate::player_damage::DMG_SLASH
                        } else {
                            0
                        },
                    dissolve: false,
                    direction: event.pre_velocities[other].normalize_or_zero(),
                    origin: event.origins[other],
                    attacker,
                });
            }
        }
    }
}

/// The cue StickAtPosition plays (the host preloads it).
pub const SOUNDS: &[&str] = &["Metal.SawbladeStick"];
/// CPhysicsProp::VPhysicsCollision's first collision after a gravity gun launch ->
/// HandleFirstCollisionInteractions -> HandleInteractionStick for onworldimpact stick
/// props hitting the world: faster than 1000 u/s, the velocity within acos(0.3) of the
/// surface normal pointing away from the prop, and not into metal, grate or 'X'
/// (sky, nodraw) surfaces (`material`: the game material of a collision side). Returns
/// each prop to stick and where: its origin pushed 8 units along its velocity
/// (StickAtPosition runs after the simulation).
pub fn world_sticks(
    collisions: &[PhysicsCollision],
    world: &World,
    launched: &std::collections::HashSet<usize>,
    material: impl Fn(&PhysicsCollision, usize) -> char,
) -> Vec<(usize, Vec3)> {
    let mut seen = std::collections::HashSet::new();
    let mut sticks = Vec::new();
    for event in collisions {
        for index in 0..2 {
            let other = 1 - index;
            let Some(id) = event.entities[index].filter(|id| launched.contains(id)) else {
                continue;
            };
            // Only the first collision after the launch counts.
            if !seen.insert(id) {
                continue;
            }
            let sticky = world
                .entities
                .get(id)
                .is_some_and(|e| e.class().starts_with("prop_physics"))
                && world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(id))
                    .and_then(|i| world.model_prop_data.get(&i.asset_key()))
                    .is_some_and(|d| d.world_stick);
            // IsWorld: world geometry (no entity, not a free body).
            if !sticky || event.entities[other].is_some() || event.masses[other] > 0. {
                continue;
            }
            let velocity = event.pre_velocities[index];
            let speed = velocity.length();
            let away = if index == 0 {
                event.normal
            } else {
                -event.normal
            };
            if speed <= 1000. || velocity.dot(away) / speed <= 0.3 {
                continue;
            }
            if matches!(material(event, other), 'M' | 'G' | 'X') {
                continue;
            }
            sticks.push((id, event.origins[index] + velocity / speed * 8.));
        }
    }
    sticks
}

/// The impactdamagetable_t fields the prop path reads (allowStaticDamage true skips
/// the minimum mass/speed gates).
pub struct ImpactTable {
    pub linear: &'static [(f32, f32)],
    pub small_mass_max: f32,
    pub small_mass_cap: f32,
    pub large_mass_min: f32,
    pub large_mass_scale: f32,
    pub large_mass_falling_scale: f32,
}
pub static NPC_TABLE: ImpactTable = ImpactTable {
    linear: &NPC_LINEAR,
    small_mass_max: SMALL_MASS_MAX,
    small_mass_cap: SMALL_MASS_CAP,
    large_mass_min: LARGE_MASS_MIN,
    large_mass_scale: LARGE_MASS_SCALE,
    large_mass_falling_scale: LARGE_MASS_FALLING_SCALE,
};
/// gGlassImpactDamageTable ("glass", propdata Glass.Small/Glass.Window/Pottery). The
/// retail registry entry (physics-research-20261011 SI-25) equals the SDK, including
/// the out-of-order last row (ReadDamageTable stops at the first threshold above the
/// energy, so from 250000 up it runs past the 62500 row and reads 500: the 100 row is
/// never the answer) and a falling scale of 0.
const GLASS_LINEAR: [(f32, f32); 6] = [
    (25. * 25., 10.),
    (50. * 50., 20.),
    (100. * 100., 50.),
    (200. * 200., 75.),
    (500. * 500., 100.),
    (250. * 250., 500.),
];
pub static GLASS_TABLE: ImpactTable = ImpactTable {
    linear: &GLASS_LINEAR,
    small_mass_max: 1.,
    small_mass_cap: 10.,
    large_mass_min: 50.,
    large_mass_scale: 4.,
    large_mass_falling_scale: 0.,
};
/// CalculateDefaultPhysicsDamage's table lookup: a registered name ("npc", "glass";
/// the player tables are not modeled here) or the NPC table.
pub fn table_named(name: Option<&str>) -> &'static ImpactTable {
    match name {
        Some("glass") => &GLASS_TABLE,
        _ => &NPC_TABLE,
    }
}

/// CBreakableProp m_impactEnergyScale: keyvalue physdamagescale, 0 or absent -> 0.1
/// (CBreakableProp::Spawn).
pub fn prop_energy_scale(entity: &modkit_core::Entity, overridden: Option<&str>) -> f32 {
    let scale = overridden
        .or(entity.get("physdamagescale"))
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v >= 0.)
        .unwrap_or(0.);
    if scale == 0. {
        0.1
    } else {
        scale
    }
}

/// CalculatePhysicsImpactDamage (linear) for a physics prop of `mass` (allowStaticDamage
/// true, as CPhysicsProp::VPhysicsCollision passes): the speed both bodies lost, the
/// other weighted by its mass, per kilogram of this prop. A static or unmoving other
/// contributes only this prop's own loss.
pub fn prop_damage(
    table: &ImpactTable,
    mass: f32,
    energy_scale: f32,
    own: Impactor,
    other: Impactor,
) -> f32 {
    if mass <= 0. || energy_scale <= 0. {
        return 0.;
    }
    // myMinVelocity is 0 in both tables: no filtering of this prop's own loss.
    let delta = own.pre.length() - own.post.length();
    let mut energy = delta * delta * mass;
    let delta = other.pre.length() - other.post.length();
    let mut other_energy = delta * delta * other.mass;
    if other.mass >= table.large_mass_min {
        other_energy *= table.large_mass_scale;
        let dz = other.pre.z - other.post.z;
        if delta > 0. && dz < 0. && other.pre.z < 0. {
            let factor = (dz / delta).abs();
            other_energy *= 1. + factor * (table.large_mass_falling_scale - 1.);
        }
    }
    energy += other_energy;
    energy *= energy_scale / mass;
    let damage = read_table(table.linear, energy);
    if other.mass > 0. && other.mass < table.small_mass_max && table.small_mass_cap > 0. {
        damage.clamp(0., table.small_mass_cap)
    } else {
        damage
    }
}

/// CPhysicsProp::VPhysicsCollision physics damage for prop_physics (breakables take it
/// through the entity damage path; unbreakable props ignore it there): the prop_data
/// damage_table ("glass") or the NPC table, x10 on the first collision after a gravity
/// gun launch (`launched`, cleared by any collision). Objects the player holds take
/// none from the player and no static damage; NPCs and props hit by them are handled
/// by their own side of the event.
pub fn queue_prop_damage(
    collisions: &[PhysicsCollision],
    world: &World,
    scene: &mut Scene,
    launched: &mut std::collections::HashSet<usize>,
    thrown: &mut std::collections::HashSet<usize>,
    held: &dyn Fn(usize) -> bool,
) {
    for event in collisions {
        for index in 0..2 {
            let other = 1 - index;
            let Some(id) = event.entities[index] else {
                continue;
            };
            let Some(entity) = world.entities.get(id) else {
                continue;
            };
            if !entity.class().starts_with("prop_physics")
                || scene.states.get(id).is_none_or(|s| s.killed)
                || event.entities[other] == Some(id)
            {
                continue;
            }
            let first_after_launch = launched.remove(&id);
            // m_bThrownByPlayer || m_bFirstCollisionAfterLaunch: an NPC hit gets
            // DispatchInteraction(g_interactionHitByPlayerThrownPhysObj).
            if let Some(npc) = event.entities[other].filter(|n| {
                world
                    .entities
                    .get(*n)
                    .is_some_and(|e| e.class().starts_with("npc_"))
            }) {
                if thrown.remove(&id) || first_after_launch {
                    scene.thrown_hits.push((id, npc));
                }
            }
            // Held objects: no static damage (other side static) and none from the player.
            if held(id) && event.masses[other] <= 0. {
                continue;
            }
            let overridden = scene
                .keyvalue_overrides
                .get(&(id, "physdamagescale".to_string()))
                .map(String::as_str);
            // CBreakableProp's damage_table (prop_data), else the NPC table.
            let table = table_named(
                world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(id))
                    .and_then(|i| world.model_prop_data.get(&i.asset_key()))
                    .and_then(|d| d.damage_table.as_deref()),
            );
            let mut damage = prop_damage(
                table,
                event.masses[index],
                prop_energy_scale(entity, overridden),
                Impactor {
                    mass: event.masses[index],
                    pre: event.pre_velocities[index],
                    post: event.post_velocities[index],
                    angular: None,
                },
                Impactor {
                    mass: event.masses[other],
                    pre: event.pre_velocities[other],
                    post: event.post_velocities[other],
                    angular: None,
                },
            );
            if first_after_launch {
                damage *= 10.;
            }
            if damage > 0. {
                scene.entity_damage.push(Damage {
                    target: DamageTarget::Entity(id),
                    amount: damage,
                    kind: crate::player_damage::DMG_CRUSH,
                    dissolve: false,
                    direction: event.pre_velocities[index].normalize_or_zero(),
                    origin: event.origins[index],
                    // CTakeDamageInfo(pHitEntity, pHitEntity): what the prop hit.
                    attacker: event.entities[other].map_or(Attacker::None, Attacker::Entity),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hit(mass: f32, speed: f32, after: f32) -> Impactor {
        Impactor {
            mass,
            pre: Vec3::X * speed,
            post: Vec3::X * after,
            angular: None,
        }
    }
    #[test]
    fn launched_barrel_kills_and_light_junk_is_capped() {
        // 30 kg at the gravity gun's 1500 u/s stopping on a 90 kg cop: 1500^2 x 30 / 90
        // = 750000 -> the 500^2 entry, 100 damage.
        assert_eq!(npc_damage(90., 1., hit(30., 1500., 0.)).0, 100.);
        // A 7 kg paint can: 175000 -> 50.
        assert_eq!(npc_damage(90., 1., hit(7., 1500., 0.)).0, 50.);
        // A 1 kg bottle is under the 2 kg minimum; a 3 kg object caps at 5.
        assert_eq!(npc_damage(90., 1., hit(1., 1500., 0.)).0, 0.);
        assert_eq!(npc_damage(90., 1., hit(3., 1500., 0.)).0, 5.);
        // Slow or glancing (no speed lost) contacts do nothing.
        assert_eq!(npc_damage(90., 1., hit(30., 20., 0.)).0, 0.);
        assert_eq!(npc_damage(90., 1., hit(30., 800., 800.)).0, 0.);
        // Large masses read 4x the energy.
        assert_eq!(npc_damage(90., 1., hit(600., 120., 0.)).0, 100.);
        // physdamagescale 0 (the trainstation_02 plaza cops) takes no impact damage.
        assert_eq!(npc_damage(90., 0., hit(30., 1500., 0.)).0, 0.);
        let cop = modkit_core::Entity {
            properties: vec![("physdamagescale".into(), "0".into())],
        };
        assert_eq!(energy_scale(&cop), 0.);
        assert_eq!(
            energy_scale(&modkit_core::Entity { properties: vec![] }),
            1.
        );
    }
    #[test]
    fn a_spinning_sawblade_slices_and_slow_spin_does_not() {
        // The owned sawblade001a: 50 kg; a launched blade spins 5000 deg/s about z. A
        // disk of radius 0.5 m has I_z = 0.5 x 50 x 0.25 = 6.25 kg m^2: 31250 -> 25.
        let blade = |speed: f32, spin: f32, inertia: f32| Impactor {
            mass: 50.,
            pre: Vec3::X * speed,
            post: Vec3::X * speed,
            angular: Some((spin, spin * inertia)),
        };
        assert_eq!(npc_damage(90., 1., blade(800., 5000., 6.25)), (25., true));
        // 40000 (200^2) -> 50: enough to chop a full-health classic zombie.
        assert_eq!(npc_damage(90., 1., blade(800., 5000., 8.)), (50., true));
        // Under 360 deg/s no spin damage; with no linear speed lost, nothing.
        assert_eq!(npc_damage(90., 1., blade(800., 300., 8.)), (0., false));
        // A spinning blade at rest still slices (the speed gate passes on the spin).
        assert_eq!(npc_damage(90., 1., blade(0., 5000., 8.)), (50., true));
        // Non-slicing objects never read the spin.
        assert_eq!(
            npc_damage(
                90.,
                1.,
                Impactor {
                    angular: None,
                    ..blade(800., 5000., 8.)
                }
            ),
            (0., false)
        );
    }
    #[test]
    fn a_launched_blade_sticks_in_concrete_head_on_but_not_in_metal_or_slow_or_glancing() {
        let prop = modkit_core::Entity {
            properties: vec![("classname".into(), "prop_physics".into())],
        };
        let mut world = World {
            entities: vec![prop],
            ..Default::default()
        };
        world.model_instances.push(modkit_core::ModelInstance {
            background: false,
            model: "models/props_junk/sawblade001a.mdl".into(),
            origin: Vec3::ZERO,
            angles: Vec3::ZERO,
            skin: 0,
            body: 0,
            scale: 1.,
            kind: "prop_physics".into(),
            solid: true,
            solid_mode: None,
            entity: Some(0),
        });
        let key = world.model_instances[0].asset_key();
        world.model_prop_data.insert(
            key,
            modkit_core::PropData {
                world_stick: true,
                ..Default::default()
            },
        );
        let hit = |speed: f32, normal: Vec3| PhysicsCollision {
            colliders: [rapier3d::prelude::ColliderHandle::invalid(); 2],
            entities: [Some(0), None],
            surfaceprops: [None, None],
            position: Vec3::new(100., 0., 0.),
            origins: [Vec3::new(90., 0., 0.), Vec3::new(100., 0., 0.)],
            normal,
            speed,
            delta_time: f32::MAX,
            pre_velocities: [Vec3::X * speed, Vec3::ZERO],
            post_velocities: [Vec3::ZERO; 2],
            masses: [50., 0.],
            angular: [(0., 0.); 2],
        };
        let launched: std::collections::HashSet<usize> = [0].into();
        let concrete = |_: &PhysicsCollision, _: usize| 'C';
        assert_eq!(
            world_sticks(&[hit(1500., Vec3::X)], &world, &launched, concrete),
            [(0, Vec3::new(98., 0., 0.))]
        );
        // Metal, too slow, too glancing, or not launched: no stick.
        let metal = |_: &PhysicsCollision, _: usize| 'M';
        assert!(world_sticks(&[hit(1500., Vec3::X)], &world, &launched, metal).is_empty());
        assert!(world_sticks(&[hit(900., Vec3::X)], &world, &launched, concrete).is_empty());
        let glancing = Vec3::new(0.25, 1., 0.).normalize();
        assert!(world_sticks(&[hit(1500., glancing)], &world, &launched, concrete).is_empty());
        let none = std::collections::HashSet::new();
        assert!(world_sticks(&[hit(1500., Vec3::X)], &world, &none, concrete).is_empty());
    }
    #[test]
    fn launched_crate_breaks_on_the_floor_only_with_the_launch_bonus() {
        let crate_ = modkit_core::Entity {
            properties: vec![("physdamagescale".into(), "0".into())],
        };
        // physdamagescale 0 on a prop means the 0.1 default.
        assert_eq!(prop_energy_scale(&crate_, None), 0.1);
        assert_eq!(prop_energy_scale(&crate_, Some("1")), 1.);
        let floor = Impactor::default();
        // A 20 kg crate stopping from 500 u/s on the world: 500^2 x 0.1 = 25000 -> 5,
        // x10 after a gravity gun launch: 50 (wood_crate001a breaks at 30 health).
        let stop = hit(20., 500., 0.);
        assert_eq!(prop_damage(&NPC_TABLE, 20., 0.1, stop, floor), 5.);
        // A gentle 100 u/s landing does nothing.
        assert_eq!(
            prop_damage(&NPC_TABLE, 20., 0.1, hit(20., 100., 0.), floor),
            0.
        );
        // A 1 kg bottle thrown at the crate is capped at 5.
        assert_eq!(
            prop_damage(&NPC_TABLE, 20., 1., hit(20., 0., 0.), hit(1., 1500., 0.)),
            5.
        );
    }

    #[test]
    fn glass_table_breaks_bottles_on_light_drops_and_keeps_its_row_order() {
        let floor = Impactor::default();
        // A 1 kg glass prop (energy scale 0.1) stopping from 100 u/s on the world:
        // 100^2 x 0.1 = 1000 -> the 625 row: 10 (a 10-health Glass.Small bottle breaks),
        // where the NPC table reads nothing.
        let drop = hit(1., 100., 0.);
        assert_eq!(prop_damage(&GLASS_TABLE, 1., 0.1, drop, floor), 10.);
        assert_eq!(prop_damage(&NPC_TABLE, 1., 0.1, drop, floor), 0.);
        // 300 u/s at scale 1: 90000 stops before the 250000 row -> 75; from 250000
        // the lookup runs past the out-of-order 62500 row: 500, never 100.
        assert_eq!(
            prop_damage(&GLASS_TABLE, 1., 1., hit(1., 300., 0.), floor),
            75.
        );
        assert_eq!(
            prop_damage(&GLASS_TABLE, 1., 1., hit(1., 600., 0.), floor),
            500.
        );
        // A lighter-than-1 kg moving object caps glass damage at 10.
        assert_eq!(
            prop_damage(&GLASS_TABLE, 1., 1., hit(1., 0., 0.), hit(0.5, 1500., 0.)),
            10.
        );
        assert!(std::ptr::eq(table_named(Some("glass")), &GLASS_TABLE));
        assert!(std::ptr::eq(table_named(None), &NPC_TABLE));
    }

    #[test]
    fn prop_launched_into_a_wall_queues_launch_damage() {
        use rapier3d::prelude::*;
        const S: f32 = 1. / 39.37;
        // Synthetic scene (not game data): a wall at x 0, a 20 kg prop entity 100 units
        // out flying at it at 1500 u/s, as if launched by the gravity gun.
        let world = World {
            entities: vec![modkit_core::Entity {
                properties: vec![("classname".into(), "prop_physics".into())],
            }],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        let mut physics = crate::physics::Physics::new(&world);
        physics.colliders.insert(
            ColliderBuilder::cuboid(8. * S, 200. * S, 200. * S).translation(vector![
                -8. * S,
                0.,
                0.
            ]),
        );
        let body = physics.bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(vector![100. * S, 0., 0.])
                .linvel(vector![-1500. * S, 0., 0.])
                .gravity_scale(0.)
                .ccd_enabled(true),
        );
        physics.colliders.insert_with_parent(
            ColliderBuilder::cuboid(16. * S, 16. * S, 4. * S)
                .mass(20.)
                .user_data(1),
            body,
            &mut physics.bodies,
        );
        physics.dynamic.insert(0, body);
        physics.launched.insert(0);
        let mut total = 0.;
        for _ in 0..20 {
            physics.tick(0.015);
            queue_prop_damage(
                &physics.resolved_collisions,
                &world,
                &mut scene,
                &mut physics.launched,
                &mut Default::default(),
                &|_| false,
            );
            total += scene.entity_damage.drain(..).map(|d| d.amount).sum::<f32>();
        }
        assert!(physics.launched.is_empty());
        assert!(total >= 50., "launch damage {total}");
    }
}
