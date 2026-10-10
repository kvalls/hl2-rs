//! weapon_crossbow and crossbow_bolt from the singleplayer SDK (weapon_crossbow.cpp,
//! CWeaponCrossbow / CCrossbowBolt) plus the player FOV ramp it drives (SetFOV/GetFOV).
//! SDK behavior; retail server.dll and native captures are not compared yet.
use crate::{
    entities::Scene,
    gameplay::{activity_sequence, duration, play_sound, Inventory, Weapon},
    physics::{Physics, ProjectileHull, RayHit},
    projectiles::{Damage, DamageTarget, Projectile, ProjectileKind, ProjectileSpawn},
};
use glam::Vec3;
use modkit_core::World;
use serde::Serialize;
use std::collections::BTreeMap;

/// weapon_crossbow.cpp BOLT_AIR_VELOCITY / BOLT_WATER_VELOCITY.
pub const BOLT_AIR_VELOCITY: f32 = 2500.;
pub const BOLT_WATER_VELOCITY: f32 = 1500.;
/// CCrossbowBolt::Spawn sets this model (BOLT_MODEL is only precached).
pub const BOLT_MODEL: &str = "models/crossbow_bolt.mdl";
/// CCrossbowBolt glow (kRenderGlow, 255 255 255 alpha 128, scale 0.2).
pub const BOLT_GLOW_SPRITE: &str = "sprites/light_glow02_noz";
/// CWeaponCrossbow charger (bolt tip) and load-blast sprites.
pub const CHARGER_SPRITE: &str = "sprites/light_glow02_noz";
pub const LOAD_SPRITE: &str = "sprites/blueflare1";
/// CCrossbowBolt::Spawn SetGravity(0.05); a reflected bolt switches to 1.0.
const BOLT_GRAVITY: f32 = 0.05;
/// sv_gravity.
const GRAVITY: f32 = 600.;
/// UTIL_SetSize(-0.3, 0.3).
const BOLT_HALF_SIZE: f32 = 0.3;
/// CWeaponCrossbow::FireBolt: m_flNextPrimaryAttack = curtime + 0.75.
const FIRE_INTERVAL: f64 = 0.75;
/// CWeaponCrossbow::ToggleZoom.
const ZOOM_FOV: i32 = 20;
const ZOOM_IN_RATE: f32 = 0.1;
const ZOOM_OUT_RATE: f32 = 0.2;
/// Glow sprite FadeAndDie(3.0) after the bolt sticks.
pub const STUCK_GLOW_TIME: f64 = 3.;
/// C_TEStickyBolt models are FTENT_NEVERDIE temp entities; the client pool is bounded.
const MAX_STUCK_BOLTS: usize = 500;

/// CBasePlayer::SetFOV / C_BasePlayer::GetFOV: an integer target (0 = default) reached
/// from the integer start over `rate` seconds with SimpleSplineRemapValClamped.
#[derive(Clone, Debug, Serialize)]
pub struct FovRamp {
    pub target: i32,
    pub start: i32,
    pub time: f64,
    pub rate: f32,
    /// GetDefaultFOV (default_fov 75); the host keeps it equal to its view FOV.
    pub default_fov: f32,
}
impl Default for FovRamp {
    fn default() -> Self {
        Self {
            target: 0,
            start: 0,
            time: 0.,
            rate: 0.,
            default_fov: 75.,
        }
    }
}
impl FovRamp {
    pub fn get(&self, time: f64) -> f32 {
        let fov = if self.target == 0 {
            self.default_fov
        } else {
            self.target as f32
        };
        if self.rate <= 0. || fov == self.start as f32 {
            return fov;
        }
        let t = ((time - self.time) as f32 / self.rate).max(0.);
        if t >= 1. {
            return fov;
        }
        let start = self.start as f32;
        start + (fov - start) * modkit_core::animation::simple_spline(t)
    }
    /// SetFOV(requester, FOV, zoomRate) with iZoomStart 0: the start is the current
    /// GetFOV truncated to the integer m_iFOVStart.
    pub fn set(&mut self, target: i32, rate: f32, time: f64) {
        self.start = self.get(time) as i32;
        self.time = time;
        self.target = target;
        self.rate = rate;
    }
}

/// CWeaponCrossbow::ChargerState_t.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum ChargerState {
    StartLoad,
    StartCharge,
    Ready,
    Discharge,
    #[default]
    Off,
}

/// CWeaponCrossbow fields (m_bInZoom, m_bMustReload, m_nChargeState) and the base
/// weapon idle time, plus host-facing effect state.
#[derive(Clone, Debug, Default, Serialize)]
pub struct CrossbowState {
    pub in_zoom: bool,
    pub must_reload: bool,
    /// m_flTimeWeaponIdle.
    pub idle_at: f64,
    pub charger: ChargerState,
    /// Charger sprite brightness/scale ramps (CSprite SetBrightness/SetScale with time).
    pub charger_brightness: SpriteRamp,
    pub charger_scale: SpriteRamp,
    /// Viewmodel skin: BOLT_SKIN_GLOW 1, BOLT_SKIN_NORMAL 0.
    pub skin: u8,
    /// DoLoadEffect times (CrossbowLoad effect, blueflare1 FadeOutFromSpawn); host-drawn.
    pub load_effects: Vec<f64>,
    /// IN_ATTACK2 last tick, for m_afButtonPressed.
    secondary_held: bool,
    /// Viewmodel event cursor (animation, start, elapsed).
    cursor: Option<(String, f64, f32)>,
}

/// CSprite::SetBrightness/SetScale(value, duration): linear from the current value.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct SpriteRamp {
    pub from: f32,
    pub to: f32,
    pub at: f64,
    pub duration: f32,
}
impl SpriteRamp {
    pub fn value(&self, time: f64) -> f32 {
        if self.duration <= 0. {
            return self.to;
        }
        let t = (((time - self.at) as f32) / self.duration).clamp(0., 1.);
        self.from + (self.to - self.from) * t
    }
    fn set(&mut self, to: f32, duration: f32, time: f64) {
        *self = Self {
            from: self.value(time),
            to,
            at: time,
            duration,
        };
    }
}

impl Inventory {
    /// The view FOV the host draws this frame (Source horizontal 4:3 degrees).
    pub fn view_fov(&self, time: f64) -> f32 {
        self.fov.get(time)
    }
    /// CWeaponCrossbow::Deploy: the unloaded draw without a bolt, else the glowing skin.
    pub(crate) fn crossbow_deploy(&mut self, clip: i32) -> &'static str {
        self.crossbow.must_reload = false;
        if clip <= 0 {
            "ACT_CROSSBOW_DRAW_UNLOADED"
        } else {
            self.crossbow.skin = 1;
            "ACT_VM_DRAW"
        }
    }
    /// CWeaponCrossbow::Holster -> StopEffects.
    pub(crate) fn crossbow_holster(&mut self, time: f64) {
        if self.crossbow.in_zoom {
            self.toggle_zoom(time);
        }
        self.set_charger(ChargerState::Off, None, time);
        self.crossbow.secondary_held = false;
        self.crossbow.cursor = None;
    }
    fn toggle_zoom(&mut self, time: f64) {
        if self.crossbow.in_zoom {
            self.fov.set(0, ZOOM_OUT_RATE, time);
            self.crossbow.in_zoom = false;
        } else {
            self.fov.set(ZOOM_FOV, ZOOM_IN_RATE, time);
            self.crossbow.in_zoom = true;
        }
    }
    /// CWeaponCrossbow::SetChargerState (sound only for START_LOAD).
    fn set_charger(
        &mut self,
        state: ChargerState,
        sound: Option<(&mut Scene, &Weapon, &World)>,
        time: f64,
    ) {
        if state == self.crossbow.charger {
            return;
        }
        self.crossbow.charger = state;
        let c = &mut self.crossbow;
        match state {
            ChargerState::StartLoad => {
                if let Some((scene, weapon, world)) = sound {
                    play_sound(scene, weapon, "special1", world, "");
                }
                c.skin = 1;
                c.load_effects.push(time);
            }
            ChargerState::StartCharge => {
                c.charger_brightness.set(32., 0.5, time);
                c.charger_scale.set(0.025, 0.5, time);
            }
            ChargerState::Ready => {
                c.charger_brightness.set(80., 1., time);
                c.charger_scale.set(0.1, 0.5, time);
            }
            ChargerState::Discharge | ChargerState::Off => {
                c.skin = 0;
                c.charger_brightness.set(0., 0., time);
            }
        }
    }
    /// CWeaponCrossbow::ItemPostFrame / ItemBusyFrame (zoom toggle on the attack2 press,
    /// the reload once the fire sequence's idle time has passed) and the viewmodel events
    /// EVENT_WEAPON_THROW/2/3 that drive the charger. Firing is `attack`.
    pub(crate) fn crossbow_tick(
        &mut self,
        world: &World,
        scene: &mut Scene,
        weapons: &BTreeMap<String, Weapon>,
        _primary: bool,
        secondary: bool,
    ) {
        let Some(weapon) = weapons.get("weapon_crossbow").cloned() else {
            return;
        };
        let time = scene.time;
        let elapsed = (time - self.animation_at).max(0.) as f32;
        let previous = self
            .crossbow
            .cursor
            .as_ref()
            .filter(|(a, at, _)| *a == self.animation && *at == self.animation_at)
            .map_or(-f32::EPSILON, |(_, _, e)| *e);
        let events: Vec<i32> = world
            .rigs
            .get(&format!("{}#0", weapon.viewmodel.to_lowercase()))
            .and_then(|rig| rig.clips.get(&self.animation))
            .map(|clip| {
                clip.events_between(previous, elapsed)
                    .into_iter()
                    .map(|e| e.id)
                    .collect()
            })
            .unwrap_or_default();
        self.crossbow.cursor = Some((self.animation.clone(), self.animation_at, elapsed));
        for id in events {
            let state = match id {
                3005 => ChargerState::StartLoad,
                3013 => ChargerState::StartCharge,
                3016 => ChargerState::Ready,
                _ => continue,
            };
            self.set_charger(state, Some((scene, &weapon, world)), time);
        }
        // m_afButtonPressed & IN_ATTACK2, in both ItemPostFrame and ItemBusyFrame.
        if secondary && !self.crossbow.secondary_held {
            self.toggle_zoom(time);
        }
        self.crossbow.secondary_held = secondary;
        // ItemBusyFrame while the player's next attack (reload) is pending.
        if time < self.owner_attack_until {
            return;
        }
        // HasWeaponIdleTimeElapsed is strict.
        if self.crossbow.must_reload && time > self.crossbow.idle_at {
            self.reload(weapons, scene, world);
        }
        // SendWeaponAnim(ACT_VM_IDLE) becomes ACT_VM_FIDGET without a bolt.
        let clip = self.owned.get("weapon_crossbow").copied().unwrap_or(0);
        self.idle_override = Some(activity_sequence(
            world,
            &weapon,
            if clip <= 0 {
                "ACT_VM_FIDGET"
            } else {
                "ACT_VM_IDLE"
            },
        ));
    }
    /// CWeaponCrossbow::PrimaryAttack -> FireBolt (clip > 0; the empty cases are in
    /// `attack`). The bolt is queued in `projectile_spawns`.
    pub(crate) fn crossbow_fire(
        &mut self,
        weapon: &Weapon,
        world: &World,
        scene: &mut Scene,
        eye: Vec3,
        direction: Vec3,
    ) {
        let time = scene.time;
        let aim = direction.normalize_or_zero();
        if let Some(clip) = self.owned.get_mut("weapon_crossbow") {
            *clip -= 1;
        }
        self.shots += 1;
        self.last_shot = time;
        // Weapon_ShootPosition is the eye; the player is never at water level 3 here
        // (water movement is not implemented), so the air speed applies.
        self.projectile_spawns.push(ProjectileSpawn {
            kind: ProjectileKind::CrossbowBolt,
            position: eye,
            velocity: aim * BOLT_AIR_VELOCITY,
            angular_velocity: Vec3::ZERO,
            at: time,
            damage: weapon.damage,
            radius: 0.,
            mass: 0.,
            lifetime: 0.,
        });
        play_sound(scene, weapon, "single_shot", world, "");
        // FireBolt: ViewPunch(-2, 0, 0).
        self.punch.punch(Vec3::new(-2., 0., 0.));
        play_sound(scene, weapon, "special2", world, "");
        let animation = activity_sequence(world, weapon, "ACT_VM_PRIMARYATTACK");
        self.animate(&animation, time);
        if self.owned.get("weapon_crossbow") == Some(&0) && self.ammo(&weapon.ammo_type) <= 0 {
            // SetSuitUpdate("!HEV_AMO0", FALSE, 0): out of ammo.
            self.suit_updates.push(("HEV_AMO0".into(), 0.));
        }
        self.next_attack = time + FIRE_INTERVAL;
        self.next_secondary
            .insert("weapon_crossbow".into(), time + FIRE_INTERVAL);
        self.soonest_attack = time + FIRE_INTERVAL;
        // PrimaryAttack: m_bMustReload, idle after ACT_VM_PRIMARYATTACK.
        self.crossbow.must_reload = true;
        self.crossbow.idle_at = time + duration(world, weapon, &animation);
        self.crossbow.load_effects.push(time);
        self.set_charger(ChargerState::Discharge, None, time);
    }
}

/// What a bolt did this tick (CCrossbowBolt::BoltTouch).
#[derive(Default)]
pub(crate) struct BoltOutcome {
    pub alive: bool,
    /// UTIL_ImpactTrace(DMG_BULLET): decal and surface impact sound.
    pub impact: Option<RayHit>,
    /// "BoltImpact": a never-dying crossbow_bolt.mdl temp model (origin, direction).
    pub stuck: Option<(Vec3, Vec3)>,
    /// The glow sprite turns on and fades over 3 s where the bolt stuck.
    pub glow: Option<Vec3>,
    /// g_pEffects->Sparks at the bolt (world contact out of water).
    pub sparks: Option<Vec3>,
}

/// Whether BoltTouch's m_takedamage != DAMAGE_NO branch applies: NPCs, breakables and
/// physics props (DAMAGE_EVENTS_ONLY without health).
fn takes_damage(world: &World, physics: &Physics, id: usize) -> bool {
    physics.dynamic.contains_key(&id)
        || world.entities.get(id).is_some_and(|e| {
            e.class().starts_with("npc_")
                || e.class().starts_with("prop_physics")
                || matches!(e.class(), "func_breakable" | "func_physbox")
        })
}

/// One tick of MOVETYPE_FLYGRAVITY (half gravity before and after the move) with
/// MOVECOLLIDE_FLY_CUSTOM touch handling, plus BubbleThink's 0.1 s angle update.
pub(crate) fn bolt_tick(
    bolt: &mut Projectile,
    world: &World,
    scene: &mut Scene,
    physics: &mut Physics,
    dt: f32,
    damage: &mut Vec<Damage>,
) -> BoltOutcome {
    let gravity = GRAVITY * bolt.gravity;
    bolt.velocity.z -= 0.5 * gravity * dt;
    let end = bolt.position + bolt.velocity * dt;
    let Some(hit) = physics.projectile_sweep(
        bolt.position,
        end,
        ProjectileHull::Box(Vec3::splat(BOLT_HALF_SIZE)),
        &[],
    ) else {
        bolt.position = end;
        bolt.velocity.z -= 0.5 * gravity * dt;
        // BubbleThink every 0.1 s: angles follow the velocity.
        if scene.time + 1e-6 >= bolt.next_think {
            bolt.next_think = scene.time + 0.1;
            bolt.angles = velocity_angles(bolt.velocity);
        }
        return BoltOutcome {
            alive: true,
            ..Default::default()
        };
    };
    let normal = hit.normal.normalize_or_zero();
    bolt.position = hit.position + normal * 0.03125;
    let (direction, speed) = (bolt.velocity.normalize_or_zero(), bolt.velocity.length());
    let contact = RayHit {
        entity: hit.entity,
        position: hit.position - normal * BOLT_HALF_SIZE,
        normal,
    };
    let sound = |scene: &mut Scene, name: &str, at: Vec3| {
        scene.sounds.push(crate::sounds::SoundRequest {
            origin: Some(at),
            ..name.into()
        });
    };
    let world_hit = hit.entity == usize::MAX;
    if !world_hit && takes_damage(world, physics, hit.entity) {
        // sk_plr_dmg_crossbow (DMG_NEVERGIB to NPCs, DMG_BULLET | DMG_NEVERGIB to the
        // rest); skill 2 AdjustPlayerDamageInflictedForSkillLevel scales by 1.
        damage.push(Damage {
            target: DamageTarget::Entity(hit.entity),
            amount: bolt.damage,
            dissolve: false,
            direction,
            origin: bolt.position,
        });
        // CalculateMeleeDamageForce(0.7): damage x ImpulseScale(75, 4) x 0.7 kg in/s.
        physics.impulse(hit.entity, direction, bolt.damage * 300. * 0.7 / 39.37);
        sound(scene, "Weapon_Crossbow.BoltHitBody", bolt.position);
        // The bolt that went through pins the ragdoll to the wall behind (ragdoll
        // pinning is out of scope; the stuck bolt model is kept).
        let forward = crate::physics::angles(bolt.angles) * Vec3::X;
        let stuck = physics
            .projectile_ray(bolt.position, bolt.position + forward * 128., &[hit.entity])
            .filter(|behind| behind.entity == usize::MAX)
            .map(|behind| (behind.position, forward));
        return BoltOutcome {
            stuck,
            ..Default::default()
        };
    }
    if world_hit {
        sound(scene, "Weapon_Crossbow.BoltHitWorld", bolt.position);
        let hit_dot = normal.dot(-direction);
        let sparks = Some(bolt.position);
        if hit_dot < 0.5 && speed > 100. {
            // Glancing hit: reflect at 0.75 speed and start to sink (gravity 1.0).
            let reflection = 2. * normal * hit_dot + direction;
            bolt.velocity = reflection * speed * 0.75;
            bolt.angles = velocity_angles(reflection);
            bolt.gravity = 1.;
            return BoltOutcome {
                alive: true,
                sparks,
                ..Default::default()
            };
        }
        let forward = crate::physics::angles(bolt.angles) * Vec3::X;
        return BoltOutcome {
            impact: Some(contact),
            stuck: Some((bolt.position, forward)),
            glow: Some(bolt.position),
            sparks,
            ..Default::default()
        };
    }
    // Moving, non-damageable entities (doors, brushes): impact mark and removal.
    BoltOutcome {
        impact: Some(contact),
        ..Default::default()
    }
}

/// VectorAngles (pitch down positive).
pub(crate) fn velocity_angles(v: Vec3) -> Vec3 {
    let d = v.normalize_or_zero();
    Vec3::new(
        (-d.z).atan2(d.truncate().length()).to_degrees(),
        d.y.atan2(d.x).to_degrees(),
        0.,
    )
}

/// A stuck bolt temp model: origin - direction x 8 along the bolt (CreateCrossbowBolt).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct StuckBolt {
    /// Stable per stuck bolt (the host keys its model instance by it).
    pub id: u64,
    pub position: Vec3,
    pub angles: Vec3,
}
pub(crate) fn stick(stuck: &mut Vec<StuckBolt>, origin: Vec3, direction: Vec3) {
    let id = stuck.last().map_or(1, |b| b.id + 1);
    if stuck.len() >= MAX_STUCK_BOLTS {
        stuck.remove(0);
    }
    stuck.push(StuckBolt {
        id,
        position: origin - direction.normalize_or_zero() * 8.,
        angles: velocity_angles(direction),
    });
}

pub(crate) const INITIAL_GRAVITY: f32 = BOLT_GRAVITY;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectiles::Projectiles;
    use rapier3d::prelude::*;

    fn crossbow() -> BTreeMap<String, Weapon> {
        // Synthetic definition (not the installed script): clip 1, reserve ammo XBowBolt.
        let mut sounds = BTreeMap::new();
        for (k, v) in [
            ("single_shot", "Weapon_Crossbow.Single"),
            ("special1", "Weapon_Crossbow.BoltElectrify"),
            ("special2", "Weapon_Crossbow.BoltFly"),
            ("reload", "Weapon_Crossbow.Reload"),
        ] {
            sounds.insert(k.to_owned(), v.to_owned());
        }
        BTreeMap::from([(
            "weapon_crossbow".to_owned(),
            Weapon {
                class: "weapon_crossbow".into(),
                viewmodel: "models/weapons/v_crossbow.mdl".into(),
                magazine: 1,
                default_clip: 1,
                ammo_type: "XBowBolt".into(),
                ammo_max: 10,
                damage: 100.,
                slot: 3,
                sounds,
                ..Default::default()
            },
        )])
    }
    fn tick(
        inv: &mut Inventory,
        world: &World,
        scene: &mut Scene,
        weapons: &BTreeMap<String, Weapon>,
        primary: bool,
        secondary: bool,
    ) {
        let mut physics = Physics::new(world);
        inv.set_attack_input(primary, secondary);
        inv.tick(
            world,
            scene,
            weapons,
            Vec3::splat(9999.),
            primary || secondary,
            0.015,
        );
        if secondary {
            inv.secondary_attack(weapons, world, scene, &mut physics, Vec3::ZERO, Vec3::X);
        } else if primary {
            inv.attack(weapons, world, scene, &mut physics, Vec3::ZERO, Vec3::X);
        }
    }

    #[test]
    fn fov_ramp_follows_set_fov_spline_and_integer_start() {
        let mut fov = FovRamp::default();
        assert_eq!(fov.get(0.), 75.);
        fov.set(20, 0.1, 1.);
        assert_eq!(fov.get(1.), 75.);
        // SimpleSpline(0.5) = 0.5.
        assert!((fov.get(1.05) - 47.5).abs() < 1e-4);
        assert_eq!(fov.get(1.1), 20.);
        // Unzoom halfway through the zoom: start truncates to an integer.
        let mut fov = FovRamp::default();
        fov.set(20, 0.1, 2.);
        fov.set(0, 0.2, 2.05);
        assert_eq!(fov.start, 47);
        assert_eq!(fov.get(2.25), 75.);
    }

    #[test]
    fn fire_reload_and_zoom_follow_cweaponcrossbow_in_15ms_ticks() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let weapons = crossbow();
        let mut inv = Inventory::default();
        inv.give_ammo("XBowBolt", 4, &weapons);
        inv.give("weapon_crossbow", &weapons, 0.);
        assert_eq!(inv.animation, "ACT_VM_DRAW");
        assert_eq!(inv.crossbow.skin, 1);
        // Tick until the draw finishes (0.5 s fallback duration without a rig).
        let mut t = 0;
        let mut fire_at = None;
        let mut reload_at = None;
        let mut bolts = 0;
        while t < 200 {
            scene.time = f64::from(t) * 0.015;
            let primary = (40..45).contains(&t);
            let secondary = (60..62).contains(&t) || (100..102).contains(&t);
            tick(&mut inv, &world, &mut scene, &weapons, primary, secondary);
            if !inv.projectile_spawns.is_empty() {
                fire_at.get_or_insert(scene.time);
                bolts += inv.projectile_spawns.len();
                let spawn = inv.projectile_spawns.remove(0);
                assert_eq!(spawn.kind, ProjectileKind::CrossbowBolt);
                assert_eq!(spawn.velocity, Vec3::X * 2500.);
                assert_eq!(spawn.damage, 100.);
            }
            if inv.is_reloading() {
                reload_at.get_or_insert(scene.time);
            }
            if t == 66 {
                // Zoom pressed at tick 60: 20 degrees after 0.1 s.
                assert!(inv.crossbow.in_zoom);
                assert_eq!(inv.view_fov(scene.time + 0.01), 20.);
            }
            t += 1;
        }
        assert_eq!(bolts, 1, "one bolt per press");
        let fire_at = fire_at.unwrap();
        assert!((fire_at - 0.6).abs() < 1e-9);
        // Idle time after ACT_VM_PRIMARYATTACK (0.5 s fallback), strict: the next tick.
        let reload_at = reload_at.unwrap();
        assert!((reload_at - (fire_at + 0.51)).abs() < 1e-6, "{reload_at}");
        assert_eq!(inv.owned["weapon_crossbow"], 1);
        assert_eq!(inv.reserve_for("weapon_crossbow", &weapons), 3);
        // The second press (tick 100, during the reload busy frames) unzoomed.
        assert!(!inv.crossbow.in_zoom);
        assert_eq!(inv.view_fov(10.), 75.);
        assert!(scene
            .sounds
            .iter()
            .any(|s| s.name == "Weapon_Crossbow.BoltFly"));
    }

    #[test]
    fn holster_unzooms_over_two_tenths() {
        let mut weapons = crossbow();
        weapons.insert(
            "weapon_pistol".into(),
            Weapon {
                class: "weapon_pistol".into(),
                magazine: 18,
                ..Default::default()
            },
        );
        let mut inv = Inventory::default();
        inv.give("weapon_crossbow", &weapons, 0.);
        inv.crossbow.in_zoom = true;
        inv.fov.set(20, 0.1, 0.);
        inv.give("weapon_pistol", &weapons, 1.);
        assert!(!inv.crossbow.in_zoom);
        assert!((inv.view_fov(1.1) - 47.5).abs() < 1e-3);
        assert_eq!(inv.view_fov(1.2), 75.);
        assert_eq!(inv.crossbow.charger, ChargerState::Off);
    }

    fn bolt_world(entity: Option<(usize, &str)>) -> (World, Scene, Physics) {
        let mut world = World::default();
        if let Some((_, class)) = entity {
            world.entities.push(modkit_core::Entity {
                properties: vec![
                    ("classname".into(), class.into()),
                    ("origin".into(), "200 0 0".into()),
                    ("health".into(), "100".into()),
                ],
            });
        }
        let scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let wall = |physics: &mut Physics, x: f32, user: u128| {
            physics.colliders.insert(
                ColliderBuilder::cuboid(1. / 39.37, 200. / 39.37, 200. / 39.37)
                    .translation(vector![x / 39.37, 0., 0.])
                    .user_data(user),
            );
        };
        wall(&mut physics, 400., 0);
        if let Some((id, _)) = entity {
            wall(&mut physics, 200., id as u128 + 1);
        }
        physics.tick(0.015);
        (world, scene, physics)
    }

    fn fly(
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        velocity: Vec3,
    ) -> (Projectiles, Vec<Damage>) {
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                kind: ProjectileKind::CrossbowBolt,
                position: Vec3::ZERO,
                velocity,
                angular_velocity: Vec3::ZERO,
                at: 0.,
                damage: 100.,
                radius: 0.,
                mass: 0.,
                lifetime: 0.,
            },
            scene,
        );
        let mut damage = vec![];
        for t in 1..40 {
            scene.time = f64::from(t) * 0.015;
            damage.extend(projectiles.tick(
                world,
                scene,
                physics,
                Vec3::splat(9999.),
                false,
                0.015,
            ));
        }
        (projectiles, damage)
    }

    #[test]
    fn bolt_flies_at_2500_with_five_percent_gravity_and_sticks_in_the_world() {
        let (world, mut scene, mut physics) = bolt_world(None);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                kind: ProjectileKind::CrossbowBolt,
                position: Vec3::ZERO,
                velocity: Vec3::X * 2500.,
                angular_velocity: Vec3::ZERO,
                at: 0.,
                damage: 100.,
                radius: 0.,
                mass: 0.,
                lifetime: 0.,
            },
            &mut scene,
        );
        scene.time = 0.015;
        projectiles.tick(
            &world,
            &mut scene,
            &mut physics,
            Vec3::splat(9999.),
            false,
            0.015,
        );
        let bolt = &projectiles.active[0];
        assert!((bolt.position.x - 37.5).abs() < 1e-3);
        // 30 u/s^2 over one tick (half before, half after the move).
        assert!((bolt.velocity.z + 0.45).abs() < 1e-4, "{}", bolt.velocity);
        assert!((bolt.position.z + 0.003375).abs() < 1e-5);
        let (projectiles, damage) = fly(&world, &mut scene, &mut physics, Vec3::X * 2500.);
        assert!(damage.is_empty());
        assert!(projectiles.active.is_empty());
        assert_eq!(projectiles.stuck_bolts.len(), 1);
        assert!((projectiles.stuck_bolts[0].position.x - (399. - 0.3 - 8.)).abs() < 0.2);
        assert_eq!(projectiles.impacts.len(), 1);
        assert!(scene
            .sounds
            .iter()
            .any(|s| s.name == "Weapon_Crossbow.BoltHitWorld"));
        assert!(projectiles
            .effects
            .iter()
            .any(|e| e.kind == crate::projectiles::EffectKind::BoltGlow));
    }

    #[test]
    fn glancing_world_hit_reflects_at_three_quarters_speed() {
        let (world, mut scene, mut physics) = bolt_world(None);
        let velocity = Vec3::new(2500. * 0.8, 2500. * 0.6, 0.);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                kind: ProjectileKind::CrossbowBolt,
                position: Vec3::new(350., 0., 0.),
                velocity,
                angular_velocity: Vec3::ZERO,
                at: 0.,
                damage: 100.,
                radius: 0.,
                mass: 0.,
                lifetime: 0.,
            },
            &mut scene,
        );
        for t in 1..4 {
            scene.time = f64::from(t) * 0.015;
            projectiles.tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(9999.),
                false,
                0.015,
            );
        }
        // hitDot = 0.8 >= 0.5: this one sticks; a 0.4 hit reflects.
        assert!(projectiles.active.is_empty());
        let (world, mut scene, mut physics) = bolt_world(None);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                kind: ProjectileKind::CrossbowBolt,
                position: Vec3::new(390., -100., 0.),
                velocity: Vec3::new(1000., 2291.29, 0.),
                angular_velocity: Vec3::ZERO,
                at: 0.,
                damage: 100.,
                radius: 0.,
                mass: 0.,
                lifetime: 0.,
            },
            &mut scene,
        );
        scene.time = 0.015;
        projectiles.tick(
            &world,
            &mut scene,
            &mut physics,
            Vec3::splat(9999.),
            false,
            0.015,
        );
        let bolt = &projectiles.active[0];
        assert!(bolt.velocity.x < 0.);
        assert!(
            (bolt.velocity.length() - 1875.).abs() < 2.,
            "{}",
            bolt.velocity
        );
        assert_eq!(bolt.gravity, 1.);
    }

    #[test]
    fn bolt_damages_npc_and_leaves_a_stuck_bolt_on_the_wall_behind() {
        let (world, mut scene, mut physics) = bolt_world(Some((0, "npc_metropolice")));
        let (projectiles, damage) = fly(&world, &mut scene, &mut physics, Vec3::X * 2500.);
        assert!(projectiles.active.is_empty());
        assert_eq!(damage.len(), 1);
        assert!(matches!(damage[0].target, DamageTarget::Entity(0)));
        assert_eq!(damage[0].amount, 100.);
        assert!(scene
            .sounds
            .iter()
            .any(|s| s.name == "Weapon_Crossbow.BoltHitBody"));
        // The wall 200 units behind is beyond the 128-unit trace: no stuck bolt.
        assert!(projectiles.stuck_bolts.is_empty());
    }

    /// Local validation: the installed script, skill.cfg, viewmodel activities/events,
    /// bolt model and sprites the SDK code expects. Prints what it found.
    #[test]
    #[ignore = "requires an owned HL2 installation in HL2_ROOT"]
    fn owned_crossbow_script_viewmodel_and_assets_match_the_sdk_expectations() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let weapons = crate::gameplay::definitions(&vfs).unwrap();
        let w = &weapons["weapon_crossbow"];
        println!(
            "crossbow: clip {} default {} ammo {} max {} damage {} sounds {:?}",
            w.magazine, w.default_clip, w.ammo_type, w.ammo_max, w.damage, w.sounds
        );
        assert_eq!(w.magazine, 1);
        assert!(w.ammo_type.eq_ignore_ascii_case("XBowBolt"));
        assert!(w.damage > 0. && w.ammo_max > 0);
        for key in ["single_shot", "special1", "special2", "reload"] {
            assert!(w.sounds.contains_key(key), "{key}");
        }
        let mut world = World::default();
        crate::actors::prepare_weapons(&mut world, &vfs, &weapons).unwrap();
        let rig = &world.rigs[&format!("{}#0", w.viewmodel.to_lowercase())];
        for activity in [
            "ACT_VM_DRAW",
            "ACT_CROSSBOW_DRAW_UNLOADED",
            "ACT_VM_IDLE",
            "ACT_VM_FIDGET",
            "ACT_VM_PRIMARYATTACK",
            "ACT_VM_RELOAD",
        ] {
            let sequence = rig
                .lookup_sequence(activity, None, |_| 0)
                .unwrap_or_else(|| panic!("{activity} missing; sequences {:?}", rig.sequences));
            let clip = &rig.clips[sequence];
            println!(
                "{activity}: {sequence} {:.3} s events {:?}",
                clip.duration(),
                clip.events
                    .iter()
                    .map(|e| (e.id, e.cycle))
                    .collect::<Vec<_>>()
            );
        }
        // The charger events (EVENT_WEAPON_THROW/2/3) come from the reload sequence.
        let reload = &rig.clips[rig.lookup_sequence("ACT_VM_RELOAD", None, |_| 0).unwrap()];
        for id in [3005, 3013, 3016] {
            assert!(reload.events.iter().any(|e| e.id == id), "event {id}");
        }
        assert!(vfs.read(BOLT_MODEL).unwrap().is_some());
        for sprite in [BOLT_GLOW_SPRITE, LOAD_SPRITE] {
            crate::projectile_visuals::load_sprite(&vfs, sprite).unwrap();
        }
        let library = crate::sounds::Library::new(&vfs);
        for cue in [
            "Weapon_Crossbow.BoltHitWorld",
            "Weapon_Crossbow.BoltHitBody",
        ] {
            assert!(library.all_alternatives(cue).is_ok(), "{cue}");
        }
    }
}
