//! weapon_rpg, rpg_missile and env_laserdot from the singleplayer SDK (weapon_rpg.cpp:
//! CWeaponRPG, CMissile, CLaserDot). SDK behavior; retail and native not compared yet.
use crate::{
    entities::Scene,
    gameplay::{activity_sequence, duration, play_sound, Inventory, Weapon},
    physics::{Physics, ProjectileHull},
    projectiles::{Projectile, ProjectileKind, ProjectileSpawn},
};
use glam::Vec3;
use modkit_core::World;
use serde::Serialize;
use std::collections::BTreeMap;

/// RPG_SPEED.
pub const RPG_SPEED: f32 = 1500.;
/// RPG_HOMING_SPEED: the share of the target direction blended in per SeekThink tick
/// (sv_alternateticks is off on PC, so it is not doubled).
pub const RPG_HOMING_SPEED: f32 = 0.125;
/// CMissile::Spawn SetDamage(200); the player's PrimaryAttack never changes it, so
/// sk_plr_dmg_rpg_round only names the ammo type's damage cvar (retail check needed).
pub const MISSILE_DAMAGE: f32 = 200.;
/// CMissile::EXPLOSION_RADIUS (weapon_rpg.h).
pub const EXPLOSION_RADIUS: f32 = 200.;
/// CMissile::Spawn model and the one IgniteThink switches to.
pub const MISSILE_LAUNCH_MODEL: &str = "models/weapons/w_missile_launch.mdl";
pub const MISSILE_MODEL: &str = "models/weapons/w_missile.mdl";
/// CLaserDot sprite (kRenderGlow 255, scale 0.5 then LaserThink's distance scale).
pub const DOT_SPRITE: &str = "sprites/redglow1";
/// RPG_BEAM_SPRITE (viewmodel "laser" -> "laser_end" beam, red, width 0.5, brightness 128).
pub const BEAM_SPRITE: &str = "effects/laser1_noz";
/// RPG_LASER_SPRITE at the viewmodel "laser" attachment.
pub const MUZZLE_SPRITE: &str = "sprites/redglow1";
/// CWeaponRPG::PrimaryAttack grace period when the first 128 units are clear.
const GRACE_PERIOD: f32 = 0.3;
/// MAX_TRACE_LENGTH (1.732050807569 * 2 * 16384).
const MAX_TRACE_LENGTH: f32 = 56755.84;
const GRAVITY: f32 = 600.;
/// CMissile::Spawn UTIL_SetSize(-4, 4); IgniteThink shrinks it to a point.
const LAUNCH_HALF_SIZE: f32 = 4.;
const POINT_HALF_SIZE: f32 = 0.01;

/// CMissile::CreateSmokeTrail RocketTrail parameters (client C_RocketTrail::Update with
/// particle/particle_smokegrenade and particle_noisesphere puffs every StartSize/2 units).
pub struct RocketTrail {
    pub opacity: f32,
    pub spawn_rate: f32,
    pub particle_lifetime: f32,
    pub start_color: [f32; 3],
    pub end_color: [f32; 3],
    pub start_size: f32,
    pub end_size: f32,
    pub spawn_radius: f32,
    pub min_speed: f32,
    pub max_speed: f32,
}
pub const ROCKET_TRAIL: RocketTrail = RocketTrail {
    opacity: 0.2,
    spawn_rate: 100.,
    particle_lifetime: 0.5,
    start_color: [0.65, 0.65, 0.65],
    end_color: [0., 0., 0.],
    start_size: 8.,
    end_size: 32.,
    spawn_radius: 4.,
    min_speed: 2.,
    max_speed: 16.,
};
pub const TRAIL_MATERIALS: [&str; 2] = [
    "particle/particle_smokegrenade",
    "particle/particle_noisesphere",
];

/// env_laserdot (CLaserDot): position and surface normal from the owner's aim trace.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct LaserDot {
    pub position: Vec3,
    pub normal: Vec3,
    /// SetTargetEntity: a non-world entity that takes damage under the dot.
    pub target: Option<usize>,
    pub on: bool,
    /// LaserThink's sprite scale (every 0.05 s).
    pub scale: f32,
    /// The owner's WorldSpaceCenter (GetShootPosition).
    pub owner_center: Vec3,
    next_think: f64,
}
impl LaserDot {
    /// CLaserDot::GetChasePosition.
    pub fn chase_position(&self) -> Vec3 {
        self.position - self.normal * 10.
    }
}

/// CWeaponRPG state (m_bInitialStateUpdate, m_bGuiding, m_bHideGuiding, m_hMissile,
/// m_hLaserDot) with the viewmodel activity it tests.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RpgState {
    pub guiding: bool,
    initial_state_update: bool,
    pub(crate) hide_guiding: bool,
    pub missile_out: bool,
    pub dot: Option<LaserDot>,
    /// GetActivity() of the viewmodel.
    pub activity: String,
    /// m_flTimeWeaponIdle (SetIdealActivity sets it to the sequence end).
    pub(crate) idle_at: f64,
    /// UpdateLaserEffects: beam brightness 128 +/- 8, muzzle sprite scale 0.1 +/- 0.025.
    pub beam_brightness: f32,
    pub muzzle_scale: f32,
}

/// What happened to the player's missile this tick (CMissile -> NotifyRocketDied).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum MissileEvent {
    /// Explode/ShotDown: NotifyRocketDied, the RPG reloads.
    Died,
    /// Removed without exploding (left the world): the handle just clears.
    Removed,
}

fn basis(forward: Vec3) -> (Vec3, Vec3) {
    // AngleVectors: right points to the right of the view, up completes the frame.
    let right = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
    (right, right.cross(forward).normalize_or_zero())
}

impl Inventory {
    pub(crate) fn rpg_deploy(&mut self) {
        self.rpg.initial_state_update = true;
        self.rpg.activity = "ACT_VM_DRAW".into();
    }
    /// CWeaponRPG::Holster is refused while a missile is out.
    pub fn rpg_can_holster(&self) -> bool {
        !(self.active == "weapon_rpg" && self.rpg.missile_out)
    }
    /// CWeaponRPG::Holster -> StopGuiding (WeaponSound(SPECIAL2), dot removed).
    pub(crate) fn rpg_holster(&mut self, scene: Option<(&mut Scene, &Weapon, &World)>) {
        if self.rpg.guiding {
            if let Some((scene, weapon, world)) = scene {
                play_sound(scene, weapon, "special2", world, "");
            }
        }
        self.rpg.guiding = false;
        self.rpg.dot = None;
    }
    fn rpg_send(&mut self, world: &World, weapon: &Weapon, activity: &str, time: f64) {
        let sequence = activity_sequence(world, weapon, activity);
        self.rpg.idle_at = time + duration(world, weapon, &sequence);
        self.rpg.activity = activity.into();
        self.animate(&sequence, time);
    }
    /// CWeaponRPG::StartGuiding.
    fn start_guiding(&mut self, scene: &mut Scene, weapon: &Weapon, world: &World) {
        if self.rpg.hide_guiding {
            return;
        }
        self.rpg.guiding = true;
        play_sound(scene, weapon, "special1", world, "");
        // CreateLaserPointer: created off; SuppressGuiding turns it on.
        self.rpg.dot.get_or_insert(LaserDot {
            position: Vec3::ZERO,
            normal: Vec3::ZERO,
            target: None,
            on: false,
            scale: 0.5,
            owner_center: Vec3::ZERO,
            next_think: scene.time + 0.1,
        });
        self.rpg.beam_brightness = 128.;
        self.rpg.muzzle_scale = 0.25;
    }
    /// CWeaponRPG::ItemPostFrame (base attack dispatch first; firing is `attack`), the
    /// initial guiding after the draw, SuppressGuiding during the reload, WeaponIdle.
    pub(crate) fn rpg_tick(
        &mut self,
        world: &World,
        scene: &mut Scene,
        weapons: &BTreeMap<String, Weapon>,
        primary: bool,
        secondary: bool,
    ) {
        let Some(weapon) = weapons.get("weapon_rpg").cloned() else {
            return;
        };
        let time = scene.time;
        // ItemBusyFrame during the draw (the player's next attack).
        if time < self.owner_attack_until {
            return;
        }
        // Base ItemPostFrame, no buttons: CBaseHLCombatWeapon::WeaponIdle. The RPG has no
        // clip, so ReloadOrSwitchWeapons never reloads. CWeaponRPG::WeaponShouldBeLowered:
        // out of rockets (HasAnyAmmo counts a missile in flight) while the ideal activity
        // is an idle one; lowered idles repeat when their idle time passes (strict), a
        // raised weapon leaves ACT_VM_IDLE_LOWERED at once.
        if !primary && !secondary {
            let idle_like = matches!(
                self.rpg.activity.as_str(),
                "ACT_VM_IDLE"
                    | "ACT_VM_IDLE_LOWERED"
                    | "ACT_VM_IDLE_TO_LOWERED"
                    | "ACT_VM_LOWERED_TO_IDLE"
            );
            let lowered = idle_like && !self.rpg.missile_out && self.ammo(&weapon.ammo_type) <= 0;
            if lowered {
                if self.rpg.activity != "ACT_VM_IDLE_LOWERED" || time > self.rpg.idle_at {
                    self.rpg_send(world, &weapon, "ACT_VM_IDLE_LOWERED", time);
                }
            } else if self.rpg.activity == "ACT_VM_IDLE_LOWERED"
                || time > self.rpg.idle_at && self.rpg.activity != "ACT_VM_IDLE"
            {
                self.rpg_send(world, &weapon, "ACT_VM_IDLE", time);
            }
        }
        if self.rpg.initial_state_update && self.rpg.activity != "ACT_VM_DRAW" {
            self.start_guiding(scene, &weapon, world);
            self.rpg.initial_state_update = false;
        }
        // SuppressGuiding while lowered or reloading.
        let suppress = matches!(
            self.rpg.activity.as_str(),
            "ACT_VM_RELOAD" | "ACT_VM_IDLE_LOWERED"
        );
        self.rpg.hide_guiding = suppress;
        if self.rpg.dot.is_none() {
            self.start_guiding(scene, &weapon, world);
        }
        if let Some(dot) = &mut self.rpg.dot {
            dot.on = !suppress;
        }
        // Players cannot toggle the laser in singleplayer (attack2 is multiplayer only).
        if self.rpg.guiding {
            self.rpg.beam_brightness = 128. + (self.random() * 8.).round();
            self.rpg.muzzle_scale = 0.1 + self.random() * 0.025;
        }
        let idle = if self.rpg.activity == "ACT_VM_IDLE_LOWERED" {
            "ACT_VM_IDLE_LOWERED"
        } else {
            "ACT_VM_IDLE"
        };
        self.idle_override = Some(activity_sequence(world, &weapon, idle));
    }
    /// UpdateLaserPosition: the dot follows the eye trace (MASK_SHOT without windows),
    /// targets damageable non-world entities, and LaserThink rescales it every 0.05 s.
    /// Call after the weapon tick with the current eye, aim and player center/feet.
    #[allow(clippy::too_many_arguments)]
    pub fn rpg_aim(
        &mut self,
        physics: &Physics,
        world: &World,
        eye: Vec3,
        forward: Vec3,
        center: Vec3,
        feet: Vec3,
        time: f64,
    ) {
        if self.active != "weapon_rpg" {
            return;
        }
        let forward = forward.normalize_or_zero();
        let hit = physics.impact_ray(eye, forward, MAX_TRACE_LENGTH);
        let mut random = self.random();
        let Some(dot) = &mut self.rpg.dot else {
            return;
        };
        let (position, normal, target) = match hit {
            Some(hit) => (
                hit.position,
                hit.normal,
                (hit.entity != usize::MAX
                    && (physics.dynamic.contains_key(&hit.entity)
                        || world.entities.get(hit.entity).is_some_and(|e| {
                            e.class().starts_with("npc_")
                                || matches!(e.class(), "func_breakable" | "func_physbox")
                                || e.class().starts_with("prop_physics")
                        })))
                .then_some(hit.entity),
            ),
            None => (eye + forward * MAX_TRACE_LENGTH, Vec3::ZERO, None),
        };
        dot.position = position;
        dot.normal = normal;
        dot.target = target;
        dot.owner_center = center;
        if time + 1e-6 >= dot.next_think {
            dot.next_think = time + 0.05;
            // RemapVal(dist, 32, 1024, 0.01, 0.5) +/- a quarter, clamped to [0.1, 32].
            let distance = position.distance(feet);
            let scale = 0.01 + (distance - 32.) * (0.5 - 0.01) / (1024. - 32.);
            random *= scale * 0.25;
            dot.scale = (scale + random).clamp(0.1, 32.);
        }
    }
    /// CWeaponRPG::PrimaryAttack (and HandleFireOnEmpty without rockets).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rpg_fire(
        &mut self,
        weapon: &Weapon,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        eye: Vec3,
        direction: Vec3,
    ) {
        let time = scene.time;
        if self.ammo(&weapon.ammo_type) <= 0 {
            if self.rpg.missile_out {
                return;
            }
            let empty = self.empty_fire.entry("weapon_rpg".into()).or_default();
            if time > empty.next_sound {
                play_sound(scene, weapon, "empty", world, "");
                empty.next_sound = time + 0.5;
            }
            return;
        }
        if self.rpg.missile_out || self.rpg.activity == "ACT_VM_RELOAD" {
            return;
        }
        self.next_attack = time + 0.5;
        let forward = direction.normalize_or_zero();
        let (right, up) = basis(forward);
        let muzzle = eye + forward * 12. + right * 6. - up * 3.;
        // A grace period (non-solid) only when the first 128 units are clear.
        let grace = if physics.impact_ray(eye, forward, 128.).is_none() {
            GRACE_PERIOD
        } else {
            0.
        };
        self.projectile_spawns.push(ProjectileSpawn {
            kind: ProjectileKind::RpgMissile,
            position: muzzle,
            // CMissile::Create: forward x 300 + 128 up until IgniteThink.
            velocity: forward * 300. + Vec3::Z * 128.,
            angular_velocity: Vec3::ZERO,
            at: time,
            damage: MISSILE_DAMAGE,
            radius: EXPLOSION_RADIUS,
            mass: 0.,
            // Used as the grace period for missiles.
            lifetime: grace,
        });
        self.rpg.missile_out = true;
        let ammo = self.ammo(&weapon.ammo_type);
        self.set_ammo(&weapon.ammo_type, ammo - 1);
        self.shots += 1;
        self.last_shot = time;
        self.rpg_send(world, weapon, "ACT_VM_PRIMARYATTACK", time);
        play_sound(scene, weapon, "single_shot", world, "");
    }
    /// NotifyRocketDied -> CWeaponRPG::Reload (sound and ACT_VM_RELOAD; no clip).
    pub fn rpg_missile_event(
        &mut self,
        event: MissileEvent,
        weapons: &BTreeMap<String, Weapon>,
        world: &World,
        scene: &mut Scene,
    ) {
        self.rpg.missile_out = false;
        if event != MissileEvent::Died || self.active != "weapon_rpg" {
            return;
        }
        let Some(weapon) = weapons.get("weapon_rpg").cloned() else {
            return;
        };
        if self.ammo(&weapon.ammo_type) <= 0 {
            return;
        }
        play_sound(scene, &weapon, "reload", world, "");
        self.rpg_send(world, &weapon, "ACT_VM_RELOAD", scene.time);
    }
}

/// What the missile did this tick.
pub(crate) enum MissileOutcome {
    Flying,
    /// MissileTouch -> Explode at the projectile position.
    Explode,
}

/// rpg_missile think and movement for one tick: IgniteThink 0.3 s after launch
/// (MOVETYPE_FLY at RPG_SPEED along the launch angles, Missile.Ignite, the white
/// FFADE_IN), then SeekThink every tick toward the laser dot; FLYGRAVITY before
/// ignition. Non-solid during the grace period.
pub(crate) fn missile_tick(
    missile: &mut Projectile,
    dot: Option<&LaserDot>,
    scene: &mut Scene,
    physics: &Physics,
    dt: f32,
) -> MissileOutcome {
    let time = scene.time;
    if !missile.ignited {
        if time + 1e-6 >= missile.next_think {
            missile.ignited = true;
            missile.solid_at = missile.solid_at.min(time);
            missile.velocity = crate::physics::angles(missile.angles) * Vec3::X * RPG_SPEED;
            scene.sounds.push(crate::sounds::SoundRequest {
                origin: Some(missile.position),
                emitter: Some(crate::sounds::projectile_emitter(missile.id)),
                .."Missile.Ignite".into()
            });
            scene
                .screen_fades
                .push(crate::player_damage::ScreenFade::new(
                    [255, 225, 205, 64],
                    0.1,
                    0.,
                    0x0001,
                ));
        }
    } else {
        // SeekThink: go solid when the grace period ends, then home on the dot.
        if let Some(dot) = dot.filter(|d| d.on) {
            let target = actual_dot_position(dot, missile.position);
            let to_target = (target - missile.position).normalize_or_zero();
            let speed = missile.velocity.length();
            let direction = missile.velocity / speed.max(1e-6);
            let mut new = if speed != 0. {
                let blended = RPG_HOMING_SPEED * to_target + (1. - RPG_HOMING_SPEED) * direction;
                if blended.length() < 1e-3 {
                    if target != missile.position {
                        to_target
                    } else {
                        direction
                    }
                } else {
                    blended.normalize()
                }
            } else {
                to_target
            };
            missile.angles = crate::weapon_crossbow::velocity_angles(new);
            new *= speed;
            missile.velocity = new;
            if missile.velocity == Vec3::ZERO {
                return MissileOutcome::Explode;
            }
        }
    }
    let gravity = if missile.ignited {
        0.
    } else {
        GRAVITY * missile.gravity
    };
    missile.velocity.z -= 0.5 * gravity * dt;
    let end = missile.position + missile.velocity * dt;
    let solid = time >= missile.solid_at;
    let half = if missile.ignited {
        POINT_HALF_SIZE
    } else {
        LAUNCH_HALF_SIZE
    };
    if solid {
        if let Some(hit) = physics.projectile_sweep(
            missile.position,
            end,
            ProjectileHull::Box(Vec3::splat(half)),
            &[],
        ) {
            missile.position = hit.position;
            return MissileOutcome::Explode;
        }
    }
    missile.position = end;
    missile.velocity.z -= 0.5 * gravity * dt;
    MissileOutcome::Flying
}

/// CMissile::ComputeActualDotPosition: chase a targeted entity directly; otherwise
/// follow the laser line 256 units ahead of the missile's nearest point while the
/// missile is nearer the shooter than the dot or within 512 units of it.
pub fn actual_dot_position(dot: &LaserDot, missile: Vec3) -> Vec3 {
    let chase = dot.chase_position();
    if dot.target.is_some() {
        return chase;
    }
    let start = dot.owner_center;
    let laser = chase - start;
    let laser_length = laser.length();
    let laser_dir = laser / laser_length.max(1e-6);
    let missile_length = missile.distance(start);
    let target_length = missile.distance(chase);
    if missile_length < laser_length || target_length <= 512. {
        // UTIL_PointOnLineNearestPoint without clamping.
        let along = (missile - start).dot(laser_dir);
        start + laser_dir * along + laser_dir * 256.
    } else {
        chase
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectiles::{DamageTarget, Projectiles};
    use rapier3d::prelude::*;

    fn rpg() -> BTreeMap<String, Weapon> {
        // Synthetic definition (not the installed script): clipless, RPG_Round ammo.
        let sounds = [
            ("single_shot", "Weapon_RPG.Single"),
            ("special1", "Weapon_RPG.LaserOn"),
            ("special2", "Weapon_RPG.LaserOff"),
            ("reload", "Weapon_RPG.Reload"),
            ("empty", "Weapon_SMG1.Empty"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        BTreeMap::from([
            (
                "weapon_rpg".to_owned(),
                Weapon {
                    class: "weapon_rpg".into(),
                    viewmodel: "models/weapons/v_rpg.mdl".into(),
                    magazine: -1,
                    default_clip: 3,
                    ammo_type: "RPG_Round".into(),
                    ammo_max: 3,
                    slot: 4,
                    slot_position: 1,
                    sounds,
                    ..Default::default()
                },
            ),
            (
                "weapon_pistol".to_owned(),
                Weapon {
                    class: "weapon_pistol".into(),
                    magazine: 18,
                    ..Default::default()
                },
            ),
        ])
    }

    #[test]
    fn rpg_without_rockets_idles_lowered_suppresses_the_dot_and_raises_with_ammo() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let weapons = rpg();
        let mut inv = Inventory::default();
        inv.give("weapon_rpg", &weapons, 0.);
        inv.set_ammo("RPG_Round", 0);
        for t in 0..80 {
            scene.time = f64::from(t) * 0.015;
            inv.set_attack_input(false, false);
            inv.tick(&world, &mut scene, &weapons, Vec3::ZERO, false, 0.015);
        }
        assert_eq!(inv.rpg.activity, "ACT_VM_IDLE_LOWERED");
        assert!(inv.rpg.dot.is_none_or(|d| !d.on));
        inv.set_ammo("RPG_Round", 1);
        scene.time += 0.015;
        inv.tick(&world, &mut scene, &weapons, Vec3::ZERO, false, 0.015);
        assert_eq!(inv.rpg.activity, "ACT_VM_IDLE");
        scene.time += 0.015;
        inv.tick(&world, &mut scene, &weapons, Vec3::ZERO, false, 0.015);
        assert!(inv.rpg.dot.unwrap().on);
    }

    #[test]
    fn rpg_guides_after_the_draw_fires_one_missile_and_reloads_when_it_dies() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let physics = Physics::new(&world);
        let weapons = rpg();
        let mut inv = Inventory::default();
        inv.give("weapon_rpg", &weapons, 0.);
        assert_eq!(inv.reserve_for("weapon_rpg", &weapons), 3);
        let mut fired_at = None;
        let mut guiding_at = None;
        for t in 0..120 {
            scene.time = f64::from(t) * 0.015;
            let primary = (50..60).contains(&t) || (70..80).contains(&t);
            inv.set_attack_input(primary, false);
            inv.tick(&world, &mut scene, &weapons, Vec3::ZERO, primary, 0.015);
            if primary {
                let mut p = Physics::new(&world);
                inv.attack(&weapons, &world, &mut scene, &mut p, Vec3::Z * 64., Vec3::X);
            }
            inv.rpg_aim(
                &physics,
                &world,
                Vec3::Z * 64.,
                Vec3::X,
                Vec3::Z * 36.,
                Vec3::ZERO,
                scene.time,
            );
            if inv.rpg.guiding {
                guiding_at.get_or_insert(scene.time);
            }
            if !inv.projectile_spawns.is_empty() {
                fired_at.get_or_insert(scene.time);
                let spawn = inv.projectile_spawns.remove(0);
                assert_eq!(spawn.kind, ProjectileKind::RpgMissile);
                assert_eq!(spawn.position, Vec3::new(12., -6., 61.));
                assert_eq!(spawn.velocity, Vec3::new(300., 0., 128.));
                assert_eq!(spawn.lifetime, GRACE_PERIOD);
            }
            assert!(inv.projectile_spawns.is_empty(), "one missile at a time");
        }
        // Draw (0.5 s fallback) then the first post frame turns the laser on.
        assert!((guiding_at.unwrap() - 0.51).abs() < 1e-6, "{guiding_at:?}");
        assert!(inv.rpg.dot.unwrap().on);
        assert!((fired_at.unwrap() - 0.75).abs() < 1e-9);
        assert!(inv.rpg.missile_out);
        assert!(!inv.rpg_can_holster());
        inv.give("weapon_pistol", &weapons, 2.);
        assert_eq!(
            inv.active, "weapon_rpg",
            "holster refused with a missile out"
        );
        assert_eq!(inv.reserve_for("weapon_rpg", &weapons), 2);
        inv.rpg_missile_event(MissileEvent::Died, &weapons, &world, &mut scene);
        assert_eq!(inv.rpg.activity, "ACT_VM_RELOAD");
        assert!(scene.sounds.iter().any(|s| s.name == "Weapon_RPG.Reload"));
        // During the reload the dot is suppressed.
        scene.time += 0.015;
        inv.set_attack_input(false, false);
        inv.tick(&world, &mut scene, &weapons, Vec3::ZERO, false, 0.015);
        assert!(!inv.rpg.dot.unwrap().on);
        // After the reload sequence WeaponIdle returns to idle and the dot comes back.
        scene.time += 0.6;
        inv.tick(&world, &mut scene, &weapons, Vec3::ZERO, false, 0.015);
        assert_eq!(inv.rpg.activity, "ACT_VM_IDLE");
        assert!(inv.rpg.dot.unwrap().on);
    }

    #[test]
    fn dot_position_follows_the_laser_line_then_the_dot() {
        let dot = LaserDot {
            position: Vec3::new(2000., 0., 0.),
            normal: Vec3::new(-1., 0., 0.),
            target: None,
            on: true,
            scale: 0.5,
            owner_center: Vec3::ZERO,
            next_think: 0.,
        };
        // Nearer the shooter than the dot: 256 units ahead on the laser line.
        let p = actual_dot_position(&dot, Vec3::new(500., 100., 0.));
        assert!((p - Vec3::new(756., 0., 0.)).length() < 1e-3);
        // Past the dot and farther than 512 units from it: the chase position itself.
        let p = actual_dot_position(&dot, Vec3::new(2700., 100., 0.));
        assert_eq!(p, Vec3::new(2010., 0., 0.));
        let mut targeted = dot;
        targeted.target = Some(3);
        assert_eq!(
            actual_dot_position(&targeted, Vec3::new(500., 100., 0.)),
            Vec3::new(2010., 0., 0.)
        );
    }

    fn missile(at: Vec3, velocity: Vec3, grace: f32) -> (Projectiles, Scene, World) {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                kind: ProjectileKind::RpgMissile,
                position: at,
                velocity,
                angular_velocity: Vec3::ZERO,
                at: 0.,
                damage: MISSILE_DAMAGE,
                radius: EXPLOSION_RADIUS,
                mass: 0.,
                lifetime: grace,
            },
            &mut scene,
        );
        (projectiles, scene, world)
    }

    #[test]
    fn missile_falls_until_ignition_then_flies_at_1500_and_homes_at_one_eighth_per_tick() {
        let (mut projectiles, mut scene, world) =
            missile(Vec3::ZERO, Vec3::new(300., 0., 128.), 0.3);
        let mut physics = Physics::new(&world);
        projectiles.laser_dot = Some(LaserDot {
            position: Vec3::new(3000., 3000., 0.),
            normal: Vec3::ZERO,
            target: Some(1),
            on: true,
            scale: 0.5,
            owner_center: Vec3::ZERO,
            next_think: 0.,
        });
        let mut ignited_at = None;
        for t in 1..=40 {
            scene.time = f64::from(t) * 0.015;
            let before = projectiles.active[0].velocity;
            projectiles.tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(9999.),
                false,
                0.015,
            );
            let m = &projectiles.active[0];
            if m.ignited && ignited_at.is_none() {
                ignited_at = Some(scene.time);
                // Launch angles (pitch 0, yaw 0) at RPG_SPEED.
                assert!(
                    (m.velocity - Vec3::X * RPG_SPEED).length() < 1e-2,
                    "{}",
                    m.velocity
                );
                assert!(scene.sounds.iter().any(|s| s.name == "Missile.Ignite"));
                assert_eq!(scene.screen_fades.len(), 1);
            } else if !m.ignited {
                // FLYGRAVITY: 600 u/s^2.
                assert!((before.z - m.velocity.z - 9.).abs() < 1e-3);
            } else {
                // SeekThink: 1/8 of the way toward the target direction each tick.
                let dir = before.normalize();
                let to =
                    (Vec3::new(3000., 3000., 0.) - (m.position - m.velocity * 0.015)).normalize();
                let expected = (0.125 * to + 0.875 * dir).normalize() * 1500.;
                assert!(
                    (m.velocity - expected).length() < 0.5,
                    "{} {}",
                    m.velocity,
                    expected
                );
            }
        }
        assert!((ignited_at.unwrap() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn missile_ignores_walls_during_the_grace_period_then_explodes_with_200_damage() {
        let (mut projectiles, _, _) = missile(Vec3::ZERO, Vec3::new(300., 0., 128.), 0.3);
        let world = World {
            entities: vec![modkit_core::Entity {
                properties: vec![
                    ("classname".into(), "npc_metropolice".into()),
                    ("origin".into(), "330 60 0".into()),
                    ("health".into(), "40".into()),
                ],
            }],
            ..World::default()
        };
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        for (x, user) in [(20., 0u128), (400., 0)] {
            physics.colliders.insert(
                ColliderBuilder::cuboid(1. / 39.37, 200. / 39.37, 200. / 39.37)
                    .translation(vector![x / 39.37, 0., 0.])
                    .user_data(user),
            );
        }
        physics.tick(0.015);
        let mut damage = vec![];
        for t in 1..60 {
            scene.time = f64::from(t) * 0.015;
            damage.extend(projectiles.tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(9999.),
                false,
                0.015,
            ));
            if projectiles.active.is_empty() {
                break;
            }
        }
        assert!(projectiles.active.is_empty());
        let blast = projectiles
            .effects
            .iter()
            .find(|e| e.kind == crate::projectiles::EffectKind::GrenadeExplosion)
            .unwrap();
        // Passed the near wall (x = 20) while non-solid; exploded on the far one.
        assert!(blast.position.x > 390., "{}", blast.position);
        assert_eq!(projectiles.missile_events, vec![MissileEvent::Died]);
        // CMissile::Explode stops the ignition loop on the missile's own emitter.
        let ignite = scene
            .sounds
            .iter()
            .find(|s| s.name == "Missile.Ignite" && !s.stop)
            .expect("ignite");
        let stop = scene
            .sounds
            .iter()
            .find(|s| s.name == "Missile.Ignite" && s.stop)
            .expect("stop");
        assert!(ignite.emitter.is_some());
        assert_eq!(ignite.emitter, stop.emitter);
        let hit = damage
            .iter()
            .find(|d| matches!(d.target, DamageTarget::Entity(0)))
            .unwrap();
        assert!(hit.amount > 0. && hit.amount < MISSILE_DAMAGE);
    }

    #[test]
    #[ignore = "requires an owned HL2 installation in HL2_ROOT"]
    fn owned_rpg_script_viewmodel_and_assets_match_the_sdk_expectations() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let weapons = crate::gameplay::definitions(&vfs).unwrap();
        let w = &weapons["weapon_rpg"];
        println!(
            "rpg: clip {} default {} ammo {} max {} sk damage {} sounds {:?}",
            w.magazine, w.default_clip, w.ammo_type, w.ammo_max, w.damage, w.sounds
        );
        assert!(w.ammo_type.eq_ignore_ascii_case("RPG_Round"));
        assert!(w.ammo_max > 0);
        for key in ["single_shot", "special1", "reload"] {
            assert!(w.sounds.contains_key(key), "{key}");
        }
        let mut world = World::default();
        crate::actors::prepare_weapons(&mut world, &vfs, &weapons).unwrap();
        let rig = &world.rigs[&format!("{}#0", w.viewmodel.to_lowercase())];
        for activity in [
            "ACT_VM_DRAW",
            "ACT_VM_IDLE",
            "ACT_VM_PRIMARYATTACK",
            "ACT_VM_RELOAD",
        ] {
            let sequence = rig
                .lookup_sequence(activity, None, |_| 0)
                .unwrap_or_else(|| panic!("{activity} missing; {:?}", rig.sequences));
            println!(
                "{activity}: {sequence} {:.3} s",
                rig.clips[sequence].duration()
            );
        }
        for name in ["laser", "laser_end", "muzzle"] {
            println!(
                "attachment {name}: {:?}",
                rig.attachment(name).map(|a| a.local.w_axis)
            );
        }
        assert!(rig.attachment("laser").is_some());
        for model in [MISSILE_LAUNCH_MODEL, MISSILE_MODEL] {
            assert!(vfs.read(model).unwrap().is_some(), "{model}");
        }
        for sprite in [DOT_SPRITE, BEAM_SPRITE] {
            crate::projectile_visuals::load_sprite(&vfs, sprite).unwrap();
        }
        for material in TRAIL_MATERIALS {
            crate::projectile_visuals::load_sprite(&vfs, material).unwrap();
        }
        let library = crate::sounds::Library::new(&vfs);
        assert!(library.all_alternatives("Missile.Ignite").is_ok());
    }
}
