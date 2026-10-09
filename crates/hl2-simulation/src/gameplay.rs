//! Partial HL2 weapon behavior, reconstructed from installed scripts and the singleplayer SDK.
//! Projectile attacks queue actual moving projectiles, independently of hitscan fire.
use crate::{entities::Scene, physics::Physics, projectiles::ProjectileSpawn};
use anyhow::{Context, Result};
use glam::Vec3;
use modkit_core::World;
use serde::{Deserialize, Serialize};
use source_assets::{keyvalues, vpk::Vfs};
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub struct Weapon {
    pub name: String,
    pub slot: usize,
    pub slot_position: usize,
    pub icon: String,
    pub viewmodel: String,
    pub magazine: i32,
    pub default_clip: i32,
    pub ammo_type: String,
    pub ammo_max: i32,
    pub secondary_ammo_type: String,
    pub secondary_ammo_max: i32,
    pub secondary_damage: f32,
    pub secondary_radius: f32,
    pub secondary_mass: f32,
    pub secondary_lifetime: f32,
    pub pellets: usize,
    pub damage: f32,
    pub sounds: BTreeMap<String, String>,
}
const IMPLEMENTED: [&str; 7] = [
    "weapon_crowbar",
    "weapon_pistol",
    "weapon_357",
    "weapon_smg1",
    "weapon_ar2",
    "weapon_shotgun",
    "weapon_frag",
];
/// Thrown frag kinds by viewmodel event (npcevent.h EVENT_WEAPON_THROW/2/3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum FragThrow {
    Throw,
    Roll,
    Lob,
}
/// CWeaponFrag state: m_AttackPaused, m_fDrawbackFinished, m_bRedraw.
#[derive(Clone, Debug, Default, Serialize)]
pub struct FragState {
    paused: u8,
    drawback_finished: bool,
    redraw: bool,
    /// Viewmodel event cursor (animation, start, elapsed).
    cursor: Option<(String, f64, f32)>,
    /// Throws whose animation event fired this tick; the host launches them.
    pub pending: Vec<FragThrow>,
}

pub fn definitions(vfs: &Vfs) -> Result<BTreeMap<String, Weapon>> {
    let skill = vfs.read("cfg/skill.cfg")?.context("skill.cfg missing")?;
    let t = keyvalues::tokens(&String::from_utf8_lossy(&skill))?;
    let number = |name: &str| {
        t.windows(2)
            .find(|p| p[0].eq_ignore_ascii_case(name))
            .and_then(|p| p[1].parse::<f32>().ok())
            .filter(|value| value.is_finite())
    };
    let localization = localization(vfs)?;
    let mut weapons = BTreeMap::new();
    for class in IMPLEMENTED {
        let data = vfs
            .read(&format!("scripts/{class}.txt"))?
            .context("weapon script missing")?;
        let script = keyvalues::parse(&String::from_utf8_lossy(&data))?;
        let root = script.first().context("empty weapon script")?;
        let value = |key| root.get(key).and_then(|e| e.text());
        let sounds = root
            .get("SoundData")
            .map(|e| {
                e.children()
                    .iter()
                    .filter_map(|entry| entry.text().map(|sound| (entry.key.clone(), sound.into())))
                    .collect()
            })
            .unwrap_or_default();
        let magazine: i32 = value("clip_size").context("clip size absent")?.parse()?;
        let ammo_type = value("primary_ammo").unwrap_or("None");
        let secondary_ammo_type = value("secondary_ammo").unwrap_or("None");
        // AR2AltFire's registered carry cvar has an underscore absent from its ammo name.
        let secondary_skill_name = if secondary_ammo_type.eq_ignore_ascii_case("AR2AltFire") {
            "ar2_altfire".into()
        } else {
            secondary_ammo_type.to_ascii_lowercase()
        };
        // Buckshot is the ammunition/damage name; sk_plr_dmg_shotgun does not exist.
        let skill_name = if class == "weapon_crowbar" {
            "crowbar".into()
        } else {
            ammo_type.to_ascii_lowercase()
        };
        let printname = value("printname").unwrap_or(class);
        let name = localization
            .get(&printname.trim_start_matches('#').to_lowercase())
            .cloned()
            .unwrap_or_else(|| printname.trim_start_matches('#').into());
        weapons.insert(
            class.into(),
            Weapon {
                name,
                slot: value("bucket").and_then(|s| s.parse().ok()).unwrap_or(0),
                slot_position: value("bucket_position")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                icon: root
                    .get("TextureData")
                    .and_then(|e| e.get("weapon"))
                    .and_then(|e| e.get("character"))
                    .and_then(|e| e.text())
                    .unwrap_or_default()
                    .into(),
                viewmodel: value("viewmodel").context("viewmodel absent")?.into(),
                magazine,
                default_clip: value("default_clip")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(magazine),
                ammo_type: ammo_type.into(),
                ammo_max: number(&format!("sk_max_{skill_name}"))
                    .unwrap_or(0.)
                    .max(0.) as i32,
                secondary_ammo_type: secondary_ammo_type.into(),
                secondary_ammo_max: number(&format!("sk_max_{secondary_skill_name}"))
                    .unwrap_or(0.)
                    .max(0.) as i32,
                secondary_damage: number(&format!("sk_plr_dmg_{secondary_skill_name}"))
                    .unwrap_or(0.)
                    .max(0.),
                secondary_radius: if class == "weapon_smg1" {
                    number("sk_smg1_grenade_radius").unwrap_or(0.).max(0.)
                } else if class == "weapon_frag" {
                    number("sk_fraggrenade_radius").unwrap_or(0.).max(0.)
                } else if class == "weapon_ar2" {
                    number("sk_weapon_ar2_alt_fire_radius")
                        .unwrap_or(10.)
                        .clamp(1., 12.)
                } else {
                    0.
                },
                secondary_mass: number("sk_weapon_ar2_alt_fire_mass")
                    .unwrap_or(150.)
                    .max(0.),
                secondary_lifetime: number("sk_weapon_ar2_alt_fire_duration")
                    .unwrap_or(2.)
                    .max(0.),
                pellets: if class == "weapon_shotgun" {
                    // Published CHalfLife2 cvar default; installed skill.cfg may override.
                    number("sk_plr_num_shotgun_pellets")
                        .unwrap_or(7.)
                        .clamp(1., 64.) as usize
                } else {
                    1
                },
                // CGrenadeFrag::Spawn: a player's grenade uses sk_plr_dmg_fraggrenade.
                damage: if class == "weapon_frag" {
                    number("sk_plr_dmg_fraggrenade").unwrap_or(0.)
                } else {
                    number(&format!("sk_plr_dmg_{skill_name}")).unwrap_or(0.)
                },
                sounds,
            },
        );
    }
    Ok(weapons)
}
fn localization(vfs: &Vfs) -> Result<BTreeMap<String, String>> {
    let Some(bytes) = vfs.read("resource/hl2_english.txt")? else {
        return Ok(BTreeMap::new());
    };
    let entries = keyvalues::parse_resource(&bytes)?;
    Ok(entries
        .first()
        .and_then(|e| e.get("Tokens"))
        .map(|e| {
            e.children()
                .iter()
                .filter_map(|e| e.text().map(|v| (e.key.to_lowercase(), v.into())))
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Clone, Copy, Serialize, Deserialize)]
enum ReloadPhase {
    Magazine,
    ShellStart,
    ShellInsert,
}
#[derive(Clone, Serialize, Deserialize)]
struct Reload {
    weapon: String,
    at: f64,
    phase: ReloadPhase,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct EmptyFire {
    latched: bool,
    next_sound: f64,
}

/// CSingleplayRules::FlPlayerFallDamage with the HL2_DLL constants: damage grows linearly
/// from the maximum safe fall speed (526.5) to 100 at the fatal speed (922.5).
pub fn fall_damage(fall_speed: f32) -> f32 {
    const MAX_SAFE: f32 = 526.5;
    const FATAL: f32 = 922.5;
    ((fall_speed - MAX_SAFE) * (100. / (FATAL - MAX_SAFE))).max(0.)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Inventory {
    #[serde(skip)]
    pub impacts: Vec<(crate::physics::RayHit, bool)>,
    #[serde(skip)]
    pub projectile_spawns: Vec<ProjectileSpawn>,
    pub previous: String,
    pub health: f32,
    pub armor: f32,
    pub suit: bool,
    pub active: String,
    pub owned: BTreeMap<String, i32>,
    /// Existing reports/consumers retain this authoritative Pistol reserve.
    pub pistol_ammo: i32,
    pub reserve_ammo: BTreeMap<String, i32>,
    pub shots: u64,
    pub bullets: u64,
    pub hits: u64,
    pub kills: u64,
    #[serde(skip)]
    pub animation: String,
    #[serde(skip)]
    pub animation_at: f64,
    #[serde(skip)]
    pub delayed_attack: bool,
    #[serde(skip)]
    pub delayed_secondary_attack: bool,
    #[serde(skip)]
    attack_input: Option<(bool, bool)>,
    next_attack: f64,
    next_secondary: BTreeMap<String, f64>,
    owner_attack_until: f64,
    #[serde(skip)]
    ar2_charge_until: Option<f64>,
    soonest_attack: f64,
    reload: Option<Reload>,
    empty_fire: BTreeMap<String, EmptyFire>,
    shotgun_need_pump: bool,
    #[serde(skip)]
    holding_attack: bool,
    /// weapon_frag throw state.
    #[serde(skip)]
    pub frag: FragState,
    /// IN_DUCK for the frag roll, set by the host before tick.
    #[serde(skip)]
    pub ducking: bool,
    burst: usize,
    last_shot: f64,
    penalty: f32,
    rng: u64,
}
impl Default for Inventory {
    fn default() -> Self {
        Self {
            impacts: Vec::new(),
            projectile_spawns: Vec::new(),
            previous: String::new(),
            health: 100.,
            armor: 0.,
            suit: false,
            active: String::new(),
            owned: BTreeMap::new(),
            pistol_ammo: 0,
            reserve_ammo: BTreeMap::new(),
            shots: 0,
            bullets: 0,
            hits: 0,
            kills: 0,
            animation: "idle01".into(),
            animation_at: 0.,
            delayed_attack: false,
            delayed_secondary_attack: false,
            attack_input: None,
            next_attack: 0.,
            next_secondary: BTreeMap::new(),
            owner_attack_until: 0.,
            ar2_charge_until: None,
            soonest_attack: 0.,
            reload: None,
            empty_fire: BTreeMap::new(),
            shotgun_need_pump: false,
            holding_attack: false,
            frag: FragState::default(),
            ducking: false,
            burst: 0,
            last_shot: -1e30,
            penalty: 0.,
            rng: 1,
        }
    }
}
impl Inventory {
    /// Retain remaining weapon deadlines when the destination map resets scene time.
    /// Transient hit/spawn queues and held input belong to the old world.
    pub fn rebase_clock(&mut self, old: f64, new: f64) {
        let delta = new - old;
        self.animation_at += delta;
        self.next_attack += delta;
        self.owner_attack_until += delta;
        self.soonest_attack += delta;
        self.last_shot += delta;
        for time in self.next_secondary.values_mut() {
            *time += delta;
        }
        for empty in self.empty_fire.values_mut() {
            empty.next_sound += delta;
        }
        if let Some(time) = &mut self.ar2_charge_until {
            *time += delta;
        }
        if let Some(reload) = &mut self.reload {
            reload.at += delta;
        }
        self.impacts.clear();
        self.projectile_spawns.clear();
        self.attack_input = None;
        self.holding_attack = false;
    }

    pub fn reserve_for(&self, class: &str, weapons: &BTreeMap<String, Weapon>) -> i32 {
        weapons
            .get(class)
            .map(|w| self.ammo(&w.ammo_type))
            .unwrap_or(0)
    }
    pub fn secondary_for(&self, class: &str, weapons: &BTreeMap<String, Weapon>) -> i32 {
        weapons
            .get(class)
            .map(|w| self.ammo(&w.secondary_ammo_type))
            .unwrap_or(0)
    }
    fn ammo(&self, ammo_type: &str) -> i32 {
        if ammo_type.eq_ignore_ascii_case("Pistol") {
            self.pistol_ammo.max(0)
        } else {
            self.reserve_ammo
                .get(ammo_type)
                .copied()
                .unwrap_or(0)
                .max(0)
        }
    }
    fn set_ammo(&mut self, ammo_type: &str, value: i32) {
        if ammo_type.eq_ignore_ascii_case("Pistol") {
            self.pistol_ammo = value.max(0);
        } else if !ammo_type.eq_ignore_ascii_case("None") && !ammo_type.is_empty() {
            self.reserve_ammo.insert(ammo_type.into(), value.max(0));
        }
    }
    /// Returns rounds actually accepted, so full pickups remain in the map.
    pub fn give_ammo(
        &mut self,
        ammo_type: &str,
        amount: i32,
        weapons: &BTreeMap<String, Weapon>,
    ) -> i32 {
        let Some((canonical, maximum)) = weapons.values().find_map(|w| {
            if w.ammo_type.eq_ignore_ascii_case(ammo_type) && w.ammo_max > 0 {
                Some((&w.ammo_type, w.ammo_max))
            } else if w.secondary_ammo_type.eq_ignore_ascii_case(ammo_type)
                && w.secondary_ammo_max > 0
            {
                Some((&w.secondary_ammo_type, w.secondary_ammo_max))
            } else {
                None
            }
        }) else {
            return 0;
        };
        let old = self.ammo(canonical);
        let added = amount.max(0).min((maximum - old).max(0));
        self.set_ammo(canonical, old + added);
        added
    }
    pub fn refill_ammo(&mut self, weapons: &BTreeMap<String, Weapon>) {
        for w in weapons.values().filter(|w| w.ammo_max > 0) {
            self.set_ammo(&w.ammo_type, w.ammo_max);
        }
        // Both developer loadouts use this path. Retail impulse101 grants these
        // fixed quantities through GiveAmmo, which clamps them to skill.cfg carry limits.
        self.give_ammo("SMG1_Grenade", 3, weapons);
        self.give_ammo("AR2AltFire", 5, weapons);
    }
    pub fn is_reloading(&self) -> bool {
        self.reload.is_some()
    }
    pub fn can_holster(&self) -> bool {
        self.ar2_charge_until.is_none()
    }
    pub fn charge_until(&self) -> Option<f64> {
        self.ar2_charge_until
    }
    /// Supply physical/script button requests before tick; delayed latches are separate.
    /// Without this call, tick's attack flag retains its primary-only behavior.
    pub fn set_attack_input(&mut self, primary: bool, secondary: bool) {
        self.attack_input = Some((primary, secondary));
    }
    pub fn idle_animation(&self) -> &'static str {
        match self.active.as_str() {
            "weapon_ar2" => "ir_idle",
            "weapon_pistol" if self.owned.get(&self.active) == Some(&0) => "idle01empty",
            _ => "idle01",
        }
    }
    pub fn give(&mut self, class: &str, weapons: &BTreeMap<String, Weapon>, time: f64) {
        if let Some(w) = weapons.get(class) {
            if w.magazine < 0 && !self.owned.contains_key(class) && w.ammo_max > 0 {
                // CBaseCombatWeapon::GiveDefaultAmmo: a clipless weapon's default clip
                // is reserve ammunition.
                self.owned.insert(class.into(), -1);
                let ammo = w.ammo_type.clone();
                self.give_ammo(&ammo, w.default_clip.max(0), weapons);
            }
            self.owned.entry(class.into()).or_insert(w.default_clip);
            if self.active == class {
                return;
            }
            // AR2 CanHolster rejects an ordinary switch during its charged shot.
            // Weapon acquisition still succeeds, but the charge keeps its owner.
            if !self.can_holster() {
                return;
            }
            self.previous = self.active.clone();
            self.reload = None;
            self.delayed_attack = false;
            self.delayed_secondary_attack = false;
            self.attack_input = None;
            self.active = class.into();
            self.burst = 0;
            self.penalty = 0.;
            self.holding_attack = false;
            // CWeaponFrag::Deploy/Holster clear the throw state.
            self.frag = FragState::default();
            let animation = if class == "weapon_ar2" {
                "ir_draw"
            } else if class == "weapon_pistol" && self.owned[class] == 0 {
                "drawempty"
            } else {
                "draw"
            };
            self.animate(animation, time);
            self.next_attack = time + fallback_duration(class, animation);
            self.owner_attack_until = self.next_attack;
            let secondary = self.next_secondary.entry(class.into()).or_default();
            *secondary = secondary.max(self.next_attack);
            // Releasing the trigger may shorten pistol firing recovery, never draw recovery.
            self.soonest_attack = self.next_attack;
        }
    }
    pub fn tick(
        &mut self,
        world: &World,
        scene: &mut Scene,
        weapons: &BTreeMap<String, Weapon>,
        feet: Vec3,
        attack: bool,
        dt: f32,
    ) {
        if !attack {
            if self.active != "weapon_pistol" {
                self.burst = 0;
            }
            if scene.time > self.soonest_attack && self.active == "weapon_pistol" {
                self.penalty = (self.penalty - dt).clamp(0., 1.5);
                if self.reload.is_none() {
                    self.next_attack = self.next_attack.min(scene.time);
                }
            }
        } else if !self.holding_attack
            && is_automatic(&self.active)
            && self.next_attack <= scene.time
            && self.owned.get(&self.active).is_some_and(|clip| *clip > 0)
        {
            // Base ItemPostFrame rebases an overdue timer on a new press; no idle burst.
            self.next_attack = scene.time;
        }
        self.holding_attack = attack;
        let (primary, secondary) = self.attack_input.take().unwrap_or((attack, false));
        self.advance_reload(world, scene, weapons, primary, secondary);
        if self.active == "weapon_frag" && self.health > 0. {
            self.frag_tick(world, scene, weapons, primary, secondary);
        }

        for (id, e) in world.entities.iter().enumerate() {
            if scene.states[id].killed
                || !scene.states[id].enabled
                || !e.class().starts_with("item_") && !weapons.contains_key(e.class())
                || scene.states[id].origin.distance(feet + Vec3::Z * 24.) > 48.
            {
                continue;
            }
            let taken = if let Some((ammo, amount)) = ammo_pickup(e.class()) {
                self.give_ammo(ammo, amount, weapons) > 0
            } else {
                match e.class() {
                    "item_suit" if !self.suit => {
                        self.suit = true;
                        true
                    }
                    "item_healthkit" if self.health < 100. => {
                        self.health = (self.health + 25.).min(100.);
                        true
                    }
                    "item_healthvial" if self.health < 100. => {
                        self.health = (self.health + 10.).min(100.);
                        true
                    }
                    "item_battery" if self.suit && self.armor < 100. => {
                        self.armor = (self.armor + 15.).min(100.);
                        true
                    }
                    class if weapons.contains_key(class) => {
                        let w = &weapons[class];
                        if !self.owned.contains_key(class) {
                            self.give(class, weapons, scene.time);
                            true
                        } else {
                            let ammo = e
                                .get("ammo")
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(w.default_clip);
                            self.give_ammo(&w.ammo_type, ammo, weapons) > 0
                        }
                    }
                    _ => false,
                }
            };
            if taken {
                scene.fire(id, "OnPlayerTouch", usize::MAX);
                scene.states[id].killed = true;
                scene.states[id].visible = false;
                // CHealthKit/CHealthVial::MyTouch and CHL2_Player::ApplyBattery
                // each emit their own installed sound. Suit logon uses HEV sentences,
                // whose scheduler is not implemented; it must not emit an ammo pickup.
                let sound = match e.class() {
                    "item_healthkit" => Some("HealthKit.Touch"),
                    "item_healthvial" => Some("HealthVial.Touch"),
                    "item_battery" => Some("ItemBattery.Touch"),
                    "item_suit" => None,
                    _ => Some("BaseCombatCharacter.AmmoPickup"),
                };
                if let Some(sound) = sound {
                    scene.sounds.push(sound.into());
                }
            }
        }
        // Base ReloadOrSwitchWeapons clears the empty latch and requires a
        // strictly elapsed attack deadline. Unsupported secondary attacks and
        // next-best-weapon selection remain separate reconstruction work.
        if is_automatic(&self.active) && self.ar2_charge_until.is_none() {
            if !primary && !secondary {
                self.try_automatic_empty_reload(weapons, scene, world);
            }
        } else if !attack
            && self.reload.is_none()
            && scene.time >= self.next_attack
            && self.owned.get(&self.active) == Some(&0)
            && self.reserve_for(&self.active, weapons) > 0
        {
            self.reload(weapons, scene, world);
        }
    }
    fn try_automatic_empty_reload(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        scene: &mut Scene,
        world: &World,
    ) {
        self.empty_fire
            .entry(self.active.clone())
            .or_default()
            .latched = false;
        if self.reload.is_none()
            && scene.time > self.next_attack
            && self.owned.get(&self.active) == Some(&0)
            && self.reserve_for(&self.active, weapons) > 0
        {
            self.reload(weapons, scene, world);
        }
    }
    fn advance_reload(
        &mut self,
        world: &World,
        scene: &mut Scene,
        weapons: &BTreeMap<String, Weapon>,
        primary: bool,
        secondary: bool,
    ) {
        if self.active == "weapon_shotgun" && self.reload.is_some() {
            let shells = self.owned.get(&self.active).copied().unwrap_or(0);
            // ItemPostFrame checks primary first, then secondary. Aborting preserves
            // the insertion deadline, even when the button is subsequently released.
            if primary && shells >= 1 {
                self.reload = None;
                self.shotgun_need_pump = false;
                self.delayed_attack = true;
            } else if secondary && shells >= 2 {
                self.reload = None;
                self.shotgun_need_pump = false;
                self.delayed_secondary_attack = true;
            }
        }
        if let Some(reload) = self.reload.clone().filter(|r| r.at <= scene.time) {
            if reload.weapon != self.active {
                self.reload = None;
                return;
            }
            let Some(w) = weapons.get(&reload.weapon) else {
                self.reload = None;
                return;
            };
            match reload.phase {
                ReloadPhase::Magazine => {
                    self.transfer_ammo(w, w.magazine);
                    self.reload = None;
                }
                ReloadPhase::ShellStart | ReloadPhase::ShellInsert => {
                    if self.owned[&self.active] < w.magazine && self.ammo(&w.ammo_type) > 0 {
                        // CWeaponShotgun::Reload fills a shell before its insertion animation.
                        self.transfer_ammo(w, 1);
                        let end = scene.time + duration(world, w, "reload2");
                        self.reload = Some(Reload {
                            weapon: self.active.clone(),
                            at: end,
                            phase: ReloadPhase::ShellInsert,
                        });
                        self.next_attack = end;
                        self.animate("reload2", scene.time);
                        play_sound(scene, w, "reload", world, "reload2");
                    } else {
                        self.reload = None;
                        self.next_attack = scene.time + duration(world, w, "reload3");
                        self.animate("reload3", scene.time);
                    }
                    return;
                }
            }
        }
        if self.active == "weapon_shotgun"
            && self.reload.is_none()
            && self.shotgun_need_pump
            && self.next_attack <= scene.time
        {
            if let Some(w) = weapons.get(&self.active) {
                self.shotgun_need_pump = false;
                self.next_attack = scene.time + duration(world, w, "pump");
                self.animate("pump", scene.time);
                play_sound(scene, w, "special1", world, "pump");
            }
        }
    }
    fn transfer_ammo(&mut self, weapon: &Weapon, limit: i32) {
        let reserve = self.ammo(&weapon.ammo_type);
        let Some(clip) = self.owned.get_mut(&self.active) else {
            return;
        };
        let add = (weapon.magazine - *clip).max(0).min(reserve).min(limit);
        *clip += add;
        self.set_ammo(&weapon.ammo_type, reserve - add);
    }
    pub fn reload(&mut self, weapons: &BTreeMap<String, Weapon>, scene: &mut Scene, world: &World) {
        let Some(w) = weapons.get(&self.active) else {
            return;
        };
        if w.magazine <= 0
            || self.ammo(&w.ammo_type) <= 0
            || self.owned.get(&self.active).copied().unwrap_or(w.magazine) >= w.magazine
            || self.reload.is_some()
            || self.ar2_charge_until.is_some()
        {
            return;
        }
        let (animation, phase) = if self.active == "weapon_shotgun" {
            if self.owned[&self.active] == 0 {
                self.shotgun_need_pump = true;
            }
            ("reload1", ReloadPhase::ShellStart)
        } else {
            (
                if self.active == "weapon_ar2" {
                    "ir_reload"
                } else {
                    "reload"
                },
                ReloadPhase::Magazine,
            )
        };
        let end = scene.time + duration(world, w, animation);
        self.reload = Some(Reload {
            weapon: self.active.clone(),
            at: end,
            phase,
        });
        self.next_attack = end;
        // Retail SMG reload restores the secondary/owner timer so that a
        // grenade can interrupt a magazine reload. AR2 retains the owner gate.
        self.owner_attack_until = if self.active == "weapon_smg1" {
            self.next_secondary.get(&self.active).copied().unwrap_or(0.)
        } else {
            end
        };
        self.animate(animation, scene.time);
        self.penalty = 0.;
        if self.active != "weapon_shotgun" {
            play_sound(scene, w, "reload", world, animation);
        }
    }
    pub fn attack(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        eye: Vec3,
        direction: Vec3,
    ) {
        if self.health <= 0.
            || self.reload.is_some()
            || self.ar2_charge_until.is_some()
            || scene.time < self.next_attack
            || scene.time < self.owner_attack_until
            || self.active == "weapon_frag"
        {
            return;
        }
        let Some(w) = weapons.get(&self.active) else {
            return;
        };
        let Some(&clip) = self.owned.get(&self.active) else {
            return;
        };
        self.delayed_attack = false;
        if clip == 0 {
            if is_automatic(&self.active) {
                // Retail SMG1/AR2 share HandleFireOnEmpty: the first call
                // clicks and latches; the next tries reload. Neither changes
                // the primary deadline nor selects a dry-fire animation.
                let empty = self.empty_fire.entry(self.active.clone()).or_default();
                if empty.latched {
                    self.try_automatic_empty_reload(weapons, scene, world);
                } else {
                    if scene.time > empty.next_sound {
                        play_sound(scene, w, "empty", world, "");
                        empty.next_sound = scene.time + 0.5;
                    }
                    empty.latched = true;
                }
            } else if self.ammo(&w.ammo_type) > 0 {
                self.reload(weapons, scene, world);
            } else {
                self.soonest_attack = scene.time + 0.2;
                self.next_attack = scene.time + duration(world, w, "dryfire");
                self.animate("dryfire", scene.time);
                play_sound(scene, w, "empty", world, "dryfire");
            }
            return;
        }
        let automatic = is_automatic(&self.active);
        // CHLMachineGun advances an existing deadline, retaining cadence across ticks.
        let rounds = if automatic {
            let rate = fire_rate(&self.active);
            let due = ((scene.time - self.next_attack) / rate + 1e-7).floor() as usize + 1;
            self.next_attack += due as f64 * rate;
            due.min(clip.max(0) as usize)
        } else {
            self.next_attack = scene.time
                + if self.active == "weapon_shotgun" {
                    duration(world, w, "fire01")
                } else {
                    fire_rate(&self.active)
                };
            1
        };
        if let Some(clip) = self.owned.get_mut(&self.active) {
            if *clip >= 0 {
                *clip -= rounds as i32;
            }
        }
        let melee = self.active == "weapon_crowbar";
        let shotgun = self.active == "weapon_shotgun";
        if self.active == "weapon_pistol" {
            self.burst = if scene.time - self.last_shot > 0.5 {
                0
            } else {
                self.burst + 1
            };
        } else if automatic {
            self.burst += 1;
        }
        self.last_shot = scene.time;
        self.soonest_attack = scene.time + 0.1;
        self.shots += rounds as u64;
        self.animate(fire_animation(&self.active, self.burst), scene.time);
        let spread_cone = match self.active.as_str() {
            "weapon_pistol" => {
                let ramp = (self.penalty / 1.5).clamp(0., 1.);
                // Source VectorLerp interpolates cone components, not angles.
                cone(1.) + (cone(6.) - cone(1.)) * ramp
            }
            "weapon_smg1" => cone(5.),
            "weapon_ar2" => cone(3.),
            "weapon_shotgun" => cone(10.),
            _ => 0.,
        };
        if self.active == "weapon_pistol" {
            self.penalty += 0.2;
        }
        let direction = direction.normalize_or_zero();
        for _ in 0..rounds {
            play_sound(scene, w, "single_shot", world, "");
            for pellet in 0..w.pellets.max(1) {
                // The first primary shotgun pellet is always on the aiming ray.
                let aim = if spread_cone == 0. || shotgun && pellet == 0 {
                    direction
                } else {
                    self.spread(direction, spread_cone)
                };
                self.bullets += 1;
                let hit = if melee {
                    melee_trace(physics, scene, eye, aim)
                } else {
                    physics.impact_ray(eye, aim, f32::from_bits(0x475db3d7))
                };
                if let Some(hit) = hit {
                    self.hit(world, scene, physics, w, hit, aim, melee);
                }
            }
        }
        if shotgun && self.owned[&self.active] > 0 {
            self.shotgun_need_pump = true;
        }
    }
    /// Secondary launch timing is independent of the primary attack timer.
    pub fn secondary_attack(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        eye: Vec3,
        direction: Vec3,
    ) {
        if self.active == "weapon_frag" {
            return;
        }
        if matches!(self.active.as_str(), "weapon_smg1" | "weapon_ar2") {
            self.projectile_secondary(weapons, world, scene, eye, direction);
            return;
        }
        if self.active != "weapon_shotgun"
            || self.health <= 0.
            || self.reload.is_some()
            || scene.time < self.next_attack
        {
            return;
        }
        let Some(w) = weapons.get(&self.active) else {
            return;
        };
        let Some(&shells) = self.owned.get(&self.active) else {
            return;
        };
        self.delayed_secondary_attack = false;
        if shells < 2 {
            // Secondary ItemPostFrame falls back to PrimaryAttack with one shell
            // (or reload/dry fire when empty), without consuming a primary latch.
            let delayed_primary = self.delayed_attack;
            self.attack(weapons, world, scene, physics, eye, direction);
            self.delayed_attack = delayed_primary;
            return;
        }
        play_sound(scene, w, "double_shot", world, "");
        self.owned.insert(self.active.clone(), shells - 2);
        self.next_attack = scene.time + duration(world, w, "altfire");
        self.last_shot = scene.time;
        self.shots += 1;
        self.animate("altfire", scene.time);
        let direction = direction.normalize_or_zero();
        // CWeaponShotgun::SecondaryAttack fires a fixed twelve pellets, rather
        // than twice the configurable primary count. Every pellet uses spread.
        for _ in 0..12 {
            let aim = self.spread(direction, cone(10.));
            self.bullets += 1;
            if let Some(hit) = physics.impact_ray(eye, aim, f32::from_bits(0x475db3d7)) {
                self.hit(world, scene, physics, w, hit, aim, false);
            }
        }
        if shells > 2 {
            self.shotgun_need_pump = true;
        }
    }
    fn projectile_secondary(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        world: &World,
        scene: &mut Scene,
        eye: Vec3,
        direction: Vec3,
    ) {
        if self.health <= 0.
            || self.ar2_charge_until.is_some()
            || scene.time < self.owner_attack_until
            || scene.time < self.next_secondary.get(&self.active).copied().unwrap_or(0.)
            || self.active == "weapon_ar2" && self.reload.is_some()
        {
            return;
        }
        let Some(w) = weapons.get(&self.active) else {
            return;
        };
        if !self.owned.contains_key(&self.active) {
            return;
        }
        if self.ammo(&w.secondary_ammo_type) <= 0 {
            self.next_secondary
                .insert(self.active.clone(), scene.time + 0.5);
            self.animate("dryfire", scene.time);
            play_sound(scene, w, "empty", world, "dryfire");
            return;
        }
        if self.active == "weapon_smg1" {
            // The retail SMG cancels reload before launching; unfinished rounds
            // are not transferred. Its reserve grenade is separate from clip1.
            self.reload = None;
            self.owner_attack_until = scene.time;
            self.launch_projectile(w, scene, world, eye, direction);
        } else {
            let end = scene.time + 0.5;
            self.ar2_charge_until = Some(end);
            self.next_attack = end;
            self.next_secondary.insert(self.active.clone(), end);
            self.animate("shake", scene.time);
            play_sound(scene, w, "special1", world, "shake");
        }
    }
    /// Call each unpaused simulation tick, even if buttons were released or the
    /// selector is open. Retail ItemPostFrame releases strictly after the timer
    /// using the player's current shoot position and aim, not their charge pose.
    pub fn advance_projectile_fire(
        &mut self,
        world: &World,
        scene: &mut Scene,
        weapons: &BTreeMap<String, Weapon>,
        eye: Vec3,
        direction: Vec3,
    ) {
        let Some(deadline) = self.ar2_charge_until else {
            return;
        };
        if scene.time <= deadline {
            return;
        }
        self.ar2_charge_until = None;
        if self.health <= 0. || self.active != "weapon_ar2" {
            return;
        }
        if let Some(w) = weapons.get(&self.active) {
            self.launch_projectile(w, scene, world, eye, direction);
        }
    }
    fn launch_projectile(
        &mut self,
        weapon: &Weapon,
        scene: &mut Scene,
        world: &World,
        eye: Vec3,
        direction: Vec3,
    ) {
        let ball = self.active == "weapon_ar2";
        let animation = if ball { "ir_fire2" } else { "altfire" };
        play_sound(scene, weapon, "double_shot", world, "");
        self.animate(animation, scene.time);
        self.shots += 1;
        self.last_shot = scene.time;
        self.set_ammo(
            &weapon.secondary_ammo_type,
            self.ammo(&weapon.secondary_ammo_type) - 1,
        );
        self.next_attack = scene.time + 0.5;
        self.next_secondary
            .insert(self.active.clone(), scene.time + 1.);
        self.owner_attack_until = if ball {
            scene.time + duration(world, weapon, animation)
        } else {
            scene.time
        };
        let angular_velocity = if ball {
            Vec3::ZERO
        } else {
            Vec3::new(self.random(), self.random(), self.random()) * 400.
        };
        self.projectile_spawns.push(ProjectileSpawn {
            kind: if ball {
                crate::projectiles::ProjectileKind::CombineBall
            } else {
                crate::projectiles::ProjectileKind::SmgGrenade
            },
            position: eye,
            velocity: direction.normalize_or_zero() * 1000.,
            angular_velocity,
            at: scene.time,
            damage: weapon.secondary_damage,
            radius: weapon.secondary_radius,
            mass: weapon.secondary_mass,
            lifetime: weapon.secondary_lifetime,
        });
    }
    pub fn apply_projectile_damage(
        &mut self,
        events: Vec<crate::projectiles::Damage>,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
    ) {
        use crate::projectiles::DamageTarget;
        for damage in events {
            if !damage.amount.is_finite() || damage.amount < 0. {
                continue;
            }
            match damage.target {
                DamageTarget::Player => {
                    // Blasts reach the player through the SDK player damage path
                    // (skill, armor, HUD message); knockback is not modeled.
                    scene.player_damage.push(crate::player_damage::DamageInfo {
                        amount: damage.amount,
                        kind: crate::player_damage::DMG_BLAST,
                        inflictor: damage.origin,
                    });
                }
                DamageTarget::Entity(id) => {
                    let Some(entity) = world.entities.get(id) else {
                        continue;
                    };
                    let Some(state) = scene.states.get_mut(id) else {
                        continue;
                    };
                    if state.killed {
                        continue;
                    }
                    let npc = entity.class().starts_with("npc_");
                    let health = entity
                        .get("health")
                        .and_then(|s| s.parse::<f32>().ok())
                        .filter(|v| *v > 0.);
                    if !npc
                        && !matches!(entity.class(), "func_breakable" | "func_physbox")
                        && !entity.class().starts_with("prop_physics")
                    {
                        continue;
                    }
                    self.hits += 1;
                    if damage.dissolve {
                        // The native dissolver keeps a timed ragdoll and effects.
                        // Here its lethal result removes the live collider while
                        // leaving the missing dissolve animation explicit.
                        state.value = 0.;
                    } else {
                        if state.value <= 0. {
                            state.value = health.unwrap_or(40.);
                        }
                        state.value -= damage.amount;
                    }
                    let dead = state.value <= 0.;
                    scene.fire(id, "OnDamaged", usize::MAX);
                    if dead {
                        self.kills += 1;
                        scene.fire(id, if npc { "OnDeath" } else { "OnBreak" }, usize::MAX);
                        scene.states[id].killed = true;
                        scene.states[id].visible = false;
                        physics.set_entity(
                            id,
                            scene.states[id].origin,
                            scene.states[id].rotation,
                            false,
                        );
                    }
                    let _ = damage.direction; // Full native damage-force transport is separate.
                }
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn hit(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        weapon: &Weapon,
        hit: crate::physics::RayHit,
        direction: Vec3,
        melee: bool,
    ) {
        let id = hit.entity;
        let entity = world.entities.get(id);
        let flesh = entity.is_some_and(|e| e.class().starts_with("npc_"));
        // SDK UTIL_ImpactTrace: every bullet and player-crowbar hit plays the hit surface's
        // impact sound (Impacts::add). The player crowbar has no melee_hit cue of its own;
        // WeaponSound(MELEE_HIT) is the NPC-operator path.
        self.impacts.push((hit, melee));
        if melee {
            self.animate("hitcenter1", scene.time);
        }
        self.hits += 1;
        physics.impulse(id, direction, if melee { 4. } else { 1. });
        if let Some(e) = entity {
            let explicit_health = e.get("health").and_then(|v| v.parse::<f32>().ok());
            if flesh
                || matches!(e.class(), "func_breakable" | "func_physbox")
                || e.class() == "prop_physics" && explicit_health.is_some_and(|v| v > 0.)
            {
                if scene.states[id].value <= 0. {
                    scene.states[id].value = explicit_health.filter(|v| *v > 0.).unwrap_or(40.);
                }
                scene.states[id].value -= weapon.damage;
                scene.fire(id, "OnDamaged", usize::MAX);
                if scene.states[id].value <= 0. {
                    self.kills += 1;
                    scene.fire(id, if flesh { "OnDeath" } else { "OnBreak" }, usize::MAX);
                    scene.states[id].killed = true;
                    scene.states[id].visible = false;
                    physics.set_entity(
                        id,
                        scene.states[id].origin,
                        scene.states[id].rotation,
                        false,
                    );
                }
            }
        }
    }
    /// CWeaponFrag::ItemPostFrame with the base attack dispatch: animation events
    /// (SEQUENCE_FINISHED, THROW/2/3), release after the drawback, pullback on
    /// attack2 then attack, 0.5 s rethrow and the redraw once the throw has finished.
    fn frag_tick(
        &mut self,
        world: &World,
        scene: &mut Scene,
        weapons: &BTreeMap<String, Weapon>,
        primary: bool,
        secondary: bool,
    ) {
        let Some(weapon) = weapons.get("weapon_frag").cloned() else {
            return;
        };
        let time = scene.time;
        let elapsed = (time - self.animation_at).max(0.) as f32;
        let previous = self
            .frag
            .cursor
            .as_ref()
            .filter(|(animation, started, _)| {
                *animation == self.animation && *started == self.animation_at
            })
            .map_or(-f32::EPSILON, |(_, _, elapsed)| *elapsed);
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
        self.frag.cursor = Some((self.animation.clone(), self.animation_at, elapsed));
        for id in events {
            let kind = match id {
                3900 => {
                    self.frag.drawback_finished = true;
                    continue;
                }
                3005 => FragThrow::Throw,
                3013 => FragThrow::Roll,
                3016 => FragThrow::Lob,
                _ => continue,
            };
            // ThrowGrenade/RollGrenade/LobGrenade, DecrementAmmo, RETHROW_DELAY 0.5.
            self.frag.pending.push(kind);
            let ammo = self.ammo(&weapon.ammo_type);
            self.set_ammo(&weapon.ammo_type, (ammo - 1).max(0));
            self.frag.redraw = true;
            self.next_attack = time + 0.5;
            self.next_secondary.insert("weapon_frag".into(), time + 0.5);
            let key = match kind {
                FragThrow::Throw => "single_shot",
                FragThrow::Lob => "double_shot",
                FragThrow::Roll => "special1",
            };
            play_sound(scene, &weapon, key, world, &self.animation);
        }
        if self.frag.drawback_finished {
            let release = match self.frag.paused {
                1 if !primary => Some("throw"),
                2 if !secondary => Some(if self.ducking { "roll" } else { "lob" }),
                _ => None,
            };
            if let Some(animation) = release {
                self.animate(animation, time);
                self.frag.drawback_finished = false;
            }
        }
        let ammo = self.ammo(&weapon.ammo_type);
        let next_secondary = self
            .next_secondary
            .get("weapon_frag")
            .copied()
            .unwrap_or(0.);
        // Base ItemPostFrame: attack2 before attack; no ammo is HandleFireOnEmpty.
        if secondary && next_secondary <= time && ammo > 0 && !self.frag.redraw {
            self.frag.paused = 2;
            self.animate("drawbacklow", time);
            self.next_secondary
                .insert("weapon_frag".into(), f64::INFINITY);
        } else if primary && self.next_attack <= time && ammo > 0 && !self.frag.redraw {
            self.frag.paused = 1;
            self.animate("drawbackhigh", time);
            self.next_attack = f64::INFINITY;
        }
        // m_bRedraw: once the throw sequence has finished, Reload draws a new grenade.
        if self.frag.redraw && f64::from(elapsed) >= duration(world, &weapon, &self.animation) {
            let next_secondary = self
                .next_secondary
                .get("weapon_frag")
                .copied()
                .unwrap_or(0.);
            if ammo > 0 && self.next_attack <= time && next_secondary <= time {
                self.animate("draw", time);
                let draw = duration(world, &weapon, "draw");
                self.next_attack = time + draw;
                self.next_secondary
                    .insert("weapon_frag".into(), time + draw);
                self.frag.redraw = false;
            }
        }
    }
    /// Launch this tick's thrown grenades (ThrowGrenade, LobGrenade, RollGrenade).
    /// `forward` is the eye direction and `velocity` the player's; `ground` is the
    /// floor normal under the roll start when one is within 16 units.
    #[allow(clippy::too_many_arguments)]
    pub fn launch_frags(
        &mut self,
        weapons: &BTreeMap<String, Weapon>,
        physics: &Physics,
        eye: Vec3,
        forward: Vec3,
        velocity: Vec3,
        feet: Vec3,
        ground: Option<Vec3>,
        time: f64,
    ) {
        let Some(weapon) = weapons.get("weapon_frag") else {
            self.frag.pending.clear();
            return;
        };
        for kind in std::mem::take(&mut self.frag.pending) {
            let random = self.random() * 2. - 1.;
            let (position, throw, angular) =
                frag_launch(physics, kind, eye, forward, velocity, feet, ground, random);
            self.projectile_spawns.push(ProjectileSpawn {
                kind: crate::projectiles::ProjectileKind::FragGrenade,
                position,
                velocity: throw,
                angular_velocity: angular,
                at: time,
                damage: weapon.damage,
                radius: weapon.secondary_radius,
                mass: 0.,
                // GRENADE_TIMER
                lifetime: 3.,
            });
        }
    }
    fn animate(&mut self, name: &str, time: f64) {
        self.animation = name.into();
        self.animation_at = time;
    }
    fn spread(&mut self, direction: Vec3, spread_cone: f32) -> Vec3 {
        // CShotManipulator sums two uniforms per axis, rejecting outside the disk.
        // This deterministic generator is not Source's prediction-seeded RNG.
        let (x, y) = loop {
            let x = (self.random() + self.random()) * 0.5;
            let y = (self.random() + self.random()) * 0.5;
            if x * x + y * y <= 1. {
                break (x, y);
            }
        };
        let axis = if direction.z.abs() > 0.99 {
            Vec3::Y
        } else {
            Vec3::Z
        };
        let right = direction.cross(axis).normalize_or_zero();
        let up = right.cross(direction).normalize_or_zero();
        (direction + right * x * spread_cone + up * y * spread_cone).normalize_or_zero()
    }
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng as u32 as f32 / u32::MAX as f32) * 2. - 1.
    }
}
fn melee_trace(
    physics: &Physics,
    scene: &Scene,
    eye: Vec3,
    forward: Vec3,
) -> Option<crate::physics::RayHit> {
    // CBaseHLBludgeonWeapon::Swing first tries the full-range ray, then a +/-16 hull
    // shortened by its diagonal radius. The target-origin dot rejects side/back hits.
    if let Some(hit) = physics.impact_ray(eye, forward, 75.) {
        return Some(hit);
    }
    let hit = physics.impact_hull(eye, forward, 75. - 1.732 * 16., Vec3::splat(16.))?;
    let target = physics
        .entity_pose(hit.entity)
        .map(|(origin, _)| origin)
        .or_else(|| scene.states.get(hit.entity).map(|state| state.origin))
        .unwrap_or(Vec3::ZERO);
    if (target - eye).normalize_or_zero().dot(forward) < 0.70721 {
        return None;
    }
    let extended = (hit.position - eye) * 2.;
    if let Some(ray) = physics.impact_ray(eye, extended.normalize_or_zero(), extended.length()) {
        return Some(ray);
    }
    // ChooseIntersectionPointAndActivity tries the eight hull corners and keeps the closest ray.
    let mut closest = None;
    let mut distance = f32::INFINITY;
    for x in [-16., 16.] {
        for y in [-16., 16.] {
            for z in [-16., 16.] {
                let delta = extended + Vec3::new(x, y, z);
                if let Some(ray) =
                    physics.impact_ray(eye, delta.normalize_or_zero(), delta.length())
                {
                    let length = eye.distance_squared(ray.position);
                    if length < distance {
                        closest = Some(ray);
                        distance = length;
                    }
                }
            }
        }
    }
    closest.or(Some(hit))
}
fn play_sound(scene: &mut Scene, weapon: &Weapon, key: &str, world: &World, animation: &str) {
    let Some(sound) = weapon.sounds.get(key) else {
        return;
    };
    // Keep distinct engine/animation sounds. Only the same symbolic sound is
    // suppressed here when the installed sequence emits it at its own event time.
    let event_sounds = world
        .rigs
        .get(&format!("{}#0", weapon.viewmodel.to_lowercase()))
        .and_then(|r| r.clips.get(animation))
        .is_some_and(|c| {
            c.events.iter().any(|e| {
                (e.id == 5004 || e.name.eq_ignore_ascii_case("AE_CL_PLAYSOUND"))
                    && e.options.trim().eq_ignore_ascii_case(sound)
            })
        });
    if !event_sounds {
        scene.sounds.push(sound.clone().into());
    }
}
/// Launch position, velocity and spin for a frag throw kind (weapon_frag.cpp).
/// CheckThrowPosition pulls the start back along a 6-unit hull trace from the eye
/// (the body center for rolls). `random` in [-1, 1] picks the authored spin range.
#[allow(clippy::too_many_arguments)]
pub fn frag_launch(
    physics: &Physics,
    kind: FragThrow,
    eye: Vec3,
    forward: Vec3,
    velocity: Vec3,
    feet: Vec3,
    ground: Option<Vec3>,
    random: f32,
) -> (Vec3, Vec3, Vec3) {
    let right = forward.cross(Vec3::Z).normalize_or_zero();
    let check = |from: Vec3, to: Vec3| {
        physics
            .projectile_sweep(
                from,
                to,
                crate::physics::ProjectileHull::Box(Vec3::splat(6.)),
                &[],
            )
            .map_or(to, |hit| hit.position)
    };
    match kind {
        FragThrow::Throw => {
            let source = check(eye, eye + forward * 18. + right * 8.);
            let mut aim = forward;
            aim.z += 0.1;
            (
                source,
                velocity + aim * 1200.,
                Vec3::new(600., (random * 1200.).round(), 0.),
            )
        }
        FragThrow::Lob => {
            let source = check(eye, eye + forward * 18. + right * 8. - Vec3::Z * 8.);
            (
                source,
                velocity + forward * 350. + Vec3::Z * 50.,
                Vec3::new(200., (random * 600.).round(), 0.),
            )
        }
        FragThrow::Roll => {
            // Bottom center of the player box plus GRENADE_RADIUS, along the floor.
            let mut facing = Vec3::new(forward.x, forward.y, 0.).normalize_or_zero();
            if let Some(normal) = ground {
                let tangent = facing.cross(normal);
                facing = normal.cross(tangent);
            }
            let start = feet + Vec3::Z * 4.;
            let source = check(feet + Vec3::Z * 36., start + facing * 18.);
            (source, velocity + facing * 700., Vec3::new(0., 0., 720.))
        }
    }
}
fn is_automatic(class: &str) -> bool {
    matches!(class, "weapon_smg1" | "weapon_ar2")
}
fn fire_rate(class: &str) -> f64 {
    match class {
        // Retail vtable methods corroborate SMG1/AR2. .357 PrimaryAttack loads 0.75.
        "weapon_smg1" => 0.075f32 as f64,
        "weapon_ar2" => 0.1f32 as f64,
        "weapon_357" => 0.75,
        "weapon_crowbar" => 0.4f32 as f64,
        _ => 0.5,
    }
}
fn fire_animation(class: &str, burst: usize) -> &'static str {
    match class {
        "weapon_crowbar" => "misscenter1",
        "weapon_shotgun" => "fire01",
        "weapon_smg1" => ["fire01", "fire02", "fire03", "fire04"][burst.saturating_sub(1).min(3)],
        "weapon_ar2" => ["ir_fire", "fire2", "fire3", "fire4"][burst.saturating_sub(1).min(3)],
        "weapon_pistol" => ["fire", "fire1", "fire2", "fire3"][burst.min(3)],
        _ => "fire",
    }
}
fn cone(degrees: f32) -> f32 {
    // Source uses these rounded float32 vector constants. In particular retail
    // AR2/SMG1 GetBulletSpread return bit patterns 3cd67770 / 3d32aae3.
    match degrees {
        1. => 0.00873,
        3. => 0.02618,
        5. => 0.04362,
        6. => 0.05234,
        10. => 0.08716,
        _ => (degrees.to_radians() * 0.5).sin(),
    }
}
fn duration(world: &World, weapon: &Weapon, animation: &str) -> f64 {
    world
        .rigs
        .get(&format!("{}#0", weapon.viewmodel.to_lowercase()))
        .and_then(|r| r.clips.get(animation))
        .map(|c| c.duration() as f64)
        .filter(|v| *v > 0.)
        .unwrap_or_else(|| fallback_duration(weapon_class(weapon), animation))
}
fn weapon_class(weapon: &Weapon) -> &str {
    match weapon.ammo_type.as_str() {
        "Pistol" => "weapon_pistol",
        "357" => "weapon_357",
        "SMG1" => "weapon_smg1",
        "AR2" => "weapon_ar2",
        "Buckshot" => "weapon_shotgun",
        _ => "weapon_crowbar",
    }
}
// Measured installed MDL durations (frames - 1)/fps; loaded clips take precedence.
fn fallback_duration(class: &str, animation: &str) -> f64 {
    match (class, animation) {
        ("weapon_smg1", "draw") | ("weapon_ar2", "ir_draw") => 25. / 30.,
        ("weapon_357", "draw") => 1.,
        ("weapon_pistol", "draw") => 24. / 30.,
        ("weapon_pistol", "drawempty") => 29. / 30.,
        ("weapon_crowbar", "draw") => 28. / 30.,
        ("weapon_shotgun", "draw") => 22. / 30.,
        ("weapon_smg1", "reload") => 1.5,
        ("weapon_smg1", "altfire") => 16. / 30.,
        ("weapon_ar2", "ir_fire2") => 21. / 30.,
        ("weapon_ar2", "shake") => 1.,
        ("weapon_ar2", "ir_reload") => 47. / 30.,
        ("weapon_357", "reload") => 110. / 30.,
        ("weapon_pistol", "reload") => 43. / 30.,
        ("weapon_shotgun", "reload1") => 0.5,
        ("weapon_shotgun", "reload2") => 0.4,
        ("weapon_shotgun", "reload3") => 13. / 30.,
        ("weapon_shotgun", "pump") => 16. / 30.,
        ("weapon_shotgun", "fire01" | "dryfire") => 10. / 30.,
        ("weapon_shotgun", "altfire") => 15. / 30.,
        (_, "dryfire") => 0.2,
        _ => 0.5,
    }
}
// SDK items.h/item_ammo.cpp quantities/aliases; difficulty scaling and crates remain separate.
fn ammo_pickup(class: &str) -> Option<(&'static str, i32)> {
    Some(match class {
        "item_ammo_pistol" | "item_box_srounds" => ("Pistol", 20),
        "item_ammo_pistol_large" | "item_large_box_srounds" => ("Pistol", 100),
        "item_ammo_smg1" | "item_box_mrounds" => ("SMG1", 45),
        "item_ammo_smg1_large" | "item_large_box_mrounds" => ("SMG1", 225),
        "item_ammo_ar2" | "item_box_lrounds" => ("AR2", 20),
        "item_ammo_ar2_large" | "item_large_box_lrounds" => ("AR2", 100),
        "item_ammo_357" => ("357", 6),
        "item_ammo_357_large" => ("357", 20),
        "item_box_buckshot" => ("Buckshot", 20),
        "item_ammo_smg1_grenade" => ("SMG1_Grenade", 1),
        "item_ammo_ar2_altfire" => ("AR2AltFire", 1),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn frag_launches_follow_weapon_frag_math() {
        use super::*;
        let world = World::default();
        let physics = Physics::new(&world);
        let eye = Vec3::new(0., 0., 64.);
        let (source, velocity, spin) = frag_launch(
            &physics,
            FragThrow::Throw,
            eye,
            Vec3::X,
            Vec3::Y * 10.,
            Vec3::ZERO,
            None,
            0.5,
        );
        // eye + 18 forward + 8 right (right of +X is -Y); forward.z + 0.1 at 1200 u/s.
        assert_eq!(source, Vec3::new(18., -8., 64.));
        assert_eq!(velocity, Vec3::new(1200., 10., 120.));
        assert_eq!(spin, Vec3::new(600., 600., 0.));
        let (source, velocity, _) = frag_launch(
            &physics,
            FragThrow::Lob,
            eye,
            Vec3::X,
            Vec3::ZERO,
            Vec3::ZERO,
            None,
            0.,
        );
        assert_eq!(source, Vec3::new(18., -8., 56.));
        assert_eq!(velocity, Vec3::new(350., 0., 50.));
        let (source, velocity, spin) = frag_launch(
            &physics,
            FragThrow::Roll,
            eye,
            Vec3::new(0.6, 0., -0.8),
            Vec3::ZERO,
            Vec3::ZERO,
            Some(Vec3::Z),
            0.,
        );
        assert!((source - Vec3::new(18., 0., 4.)).length() < 1e-4);
        assert!((velocity - Vec3::new(700., 0., 0.)).length() < 1e-3);
        assert_eq!(spin, Vec3::new(0., 0., 720.));
    }
    #[test]
    fn fall_damage_and_armor_exceptions_follow_sdk() {
        use super::*;
        assert_eq!(fall_damage(500.), 0.);
        assert!((fall_damage(922.5) - 100.).abs() < 1e-3);
        assert!((fall_damage(693.) - 42.05).abs() < 0.05);
        let mut inv = Inventory {
            health: 100.,
            armor: 50.,
            ..Default::default()
        };
        // Falls ignore armor; other damage spends one armor point per point absorbed.
        use crate::player_damage::{DamageInfo, PlayerDamage, DMG_BULLET, DMG_FALL};
        let mut damage = PlayerDamage::default();
        let mut hit = |inv: &mut Inventory, amount, kind| {
            damage.take(
                inv,
                DamageInfo {
                    amount,
                    kind,
                    inflictor: glam::Vec3::ZERO,
                },
            )
        };
        assert_eq!(hit(&mut inv, 30., DMG_FALL), 30.);
        assert_eq!(inv.armor, 50.);
        assert_eq!(hit(&mut inv, 30., DMG_BULLET), 6.);
        assert_eq!(inv.armor, 26.);
        assert_eq!(hit(&mut inv, 500., DMG_FALL), 64.);
        assert_eq!(hit(&mut inv, 10., DMG_BULLET), 0.);
    }
    #[test]
    fn map_clock_transfer_preserves_remaining_cooldown_reload_and_charge() {
        use super::*;
        let mut inv = Inventory {
            health: 42.,
            suit: true,
            next_attack: 101.5,
            owner_attack_until: 102.,
            soonest_attack: 100.25,
            last_shot: 99.,
            animation_at: 98.,
            ar2_charge_until: Some(100.5),
            reload: Some(Reload {
                weapon: "weapon_smg1".into(),
                at: 103.,
                phase: ReloadPhase::Magazine,
            }),
            ..Default::default()
        };
        inv.next_secondary.insert("weapon_smg1".into(), 104.);
        inv.empty_fire.insert(
            "weapon_smg1".into(),
            EmptyFire {
                latched: true,
                next_sound: 100.4,
            },
        );
        inv.rebase_clock(100., 0.);
        assert_eq!(
            (inv.next_attack, inv.owner_attack_until, inv.soonest_attack),
            (1.5, 2., 0.25)
        );
        assert_eq!((inv.last_shot, inv.animation_at), (-1., -2.));
        assert_eq!(inv.ar2_charge_until, Some(0.5));
        assert_eq!(inv.reload.as_ref().unwrap().at, 3.);
        assert_eq!(inv.next_secondary["weapon_smg1"], 4.);
        assert!((inv.empty_fire["weapon_smg1"].next_sound - 0.4).abs() < 1e-9);
        assert!(inv.suit && inv.health == 42.);
    }

    use super::*;
    fn melee_target(origin: Vec3) -> (Physics, Scene) {
        use rapier3d::prelude::*;
        let mut physics = Physics::default();
        physics.colliders.insert(
            ColliderBuilder::cuboid(5. / 39.37, 5. / 39.37, 5. / 39.37)
                .translation(vector![
                    origin.x / 39.37,
                    origin.y / 39.37,
                    origin.z / 39.37
                ])
                .user_data(1),
        );
        physics.tick(0.015);
        let world = World {
            entities: vec![modkit_core::Entity {
                properties: vec![
                    ("classname".into(), "prop_physics".into()),
                    (
                        "origin".into(),
                        format!("{} {} {}", origin.x, origin.y, origin.z),
                    ),
                ],
            }],
            ..Default::default()
        };
        (physics, Scene::new(&world))
    }
    #[test]
    fn melee_hull_reaches_offset_target_and_rejects_side_origin() {
        let (physics, mut scene) = melee_target(Vec3::new(40., 20., 0.));
        assert!(physics.impact_ray(Vec3::ZERO, Vec3::X, 75.).is_none());
        assert_eq!(
            melee_trace(&physics, &scene, Vec3::ZERO, Vec3::X)
                .unwrap()
                .entity,
            0
        );
        scene.states[0].origin = Vec3::Y * 40.;
        assert!(melee_trace(&physics, &scene, Vec3::ZERO, Vec3::X).is_none());
    }
    #[test]
    fn melee_full_range_ray_precedes_hull_facing_filter() {
        let (physics, mut scene) = melee_target(Vec3::X * 70.);
        scene.states[0].origin = -Vec3::X * 70.;
        assert_eq!(
            melee_trace(&physics, &scene, Vec3::ZERO, Vec3::X)
                .unwrap()
                .entity,
            0
        );
    }
    #[test]
    fn melee_facing_uses_live_rigid_body_origin() {
        use rapier3d::prelude::*;
        let (mut physics, mut scene) = melee_target(Vec3::new(40., 20., 0.));
        scene.states[0].origin = -Vec3::X * 40.;
        let body = physics
            .bodies
            .insert(RigidBodyBuilder::fixed().translation(vector![40. / 39.37, 20. / 39.37, 0.]));
        physics.dynamic.insert(0, body);
        assert!(melee_trace(&physics, &scene, Vec3::ZERO, Vec3::X).is_some());
    }
    fn definitions() -> BTreeMap<String, Weapon> {
        [
            ("weapon_crowbar", "None", -1, 0, 1),
            ("weapon_pistol", "Pistol", 18, 150, 1),
            ("weapon_357", "357", 6, 12, 1),
            ("weapon_smg1", "SMG1", 45, 225, 1),
            ("weapon_ar2", "AR2", 30, 60, 1),
            ("weapon_shotgun", "Buckshot", 6, 30, 7),
        ]
        .into_iter()
        .map(|(class, ammo, clip, ammo_max, pellets)| {
            (
                class.into(),
                Weapon {
                    name: class.into(),
                    ammo_type: ammo.into(),
                    magazine: clip,
                    default_clip: clip,
                    ammo_max,
                    secondary_ammo_type: match class {
                        "weapon_smg1" => "SMG1_Grenade",
                        "weapon_ar2" => "AR2AltFire",
                        _ => "None",
                    }
                    .into(),
                    secondary_ammo_max: if matches!(class, "weapon_smg1" | "weapon_ar2") {
                        3
                    } else {
                        0
                    },
                    pellets,
                    ..Default::default()
                },
            )
        })
        .collect()
    }
    #[test]
    fn health_and_battery_pickups_clamp_and_leave_full_or_suitless_items() {
        let world = World {
            entities: [
                "item_healthkit",
                "item_healthvial",
                "item_battery",
                "item_healthkit",
                "item_battery",
            ]
            .into_iter()
            .map(|class| modkit_core::Entity {
                properties: vec![("classname".into(), class.into())],
            })
            .collect(),
            ..Default::default()
        };
        let mut inv = Inventory {
            health: 65.,
            armor: 95.,
            suit: true,
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        inv.tick(&world, &mut scene, &definitions(), Vec3::ZERO, false, 0.015);
        assert_eq!((inv.health, inv.armor), (100., 100.));
        assert!(scene.states[..3].iter().all(|state| state.killed));
        assert!(scene.states[3..].iter().all(|state| !state.killed));
        assert_eq!(
            scene.sounds,
            ["HealthKit.Touch", "HealthVial.Touch", "ItemBattery.Touch"]
        );
        inv.suit = false;
        inv.armor = 95.;
        let mut scene = Scene::new(&world);
        inv.tick(&world, &mut scene, &definitions(), Vec3::ZERO, false, 0.015);
        assert!(scene.states.iter().all(|state| !state.killed));
        assert!(scene.sounds.is_empty());
    }
    fn shotgun_definitions() -> BTreeMap<String, Weapon> {
        let mut defs = definitions();
        let w = defs.get_mut("weapon_shotgun").unwrap();
        w.viewmodel = "models/weapons/v_shotgun.mdl".into();
        w.sounds = [
            ("single_shot", "Weapon_Shotgun.Single"),
            ("double_shot", "Weapon_Shotgun.Double"),
            ("special1", "Weapon_Shotgun.Special1"),
            ("empty", "Weapon_Shotgun.Empty"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        defs
    }
    fn impact_wall() -> World {
        let mut world = World::default();
        world.brushes.push(modkit_core::Brush {
            contents: 1,
            planes: [
                (Vec3::X, 101.),
                (-Vec3::X, -100.),
                (Vec3::Y, 50.),
                (-Vec3::Y, 50.),
                (Vec3::Z, 50.),
                (-Vec3::Z, 50.),
            ]
            .into_iter()
            .map(|(normal, distance)| modkit_core::Plane { normal, distance })
            .collect(),
        });
        world
    }
    #[test]
    fn reload_transfers_only_available_ammunition() {
        let defs = definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_pistol", &defs, 0.);
        inv.owned.insert("weapon_pistol".into(), 12);
        inv.pistol_ammo = 3;
        inv.reload(&defs, &mut scene, &world);
        scene.time = 2.;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert_eq!(inv.owned["weapon_pistol"], 15);
        assert_eq!(inv.pistol_ammo, 0);
    }
    #[test]
    fn revolver_reload_waits_and_preserves_other_ammunition() {
        let defs = definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_357", &defs, 0.);
        inv.owned.insert("weapon_357".into(), 1);
        inv.give_ammo("357", 4, &defs);
        inv.pistol_ammo = 20;
        inv.give_ammo("SMG1", 90, &defs);
        inv.reload(&defs, &mut scene, &world);
        scene.time = 3.6;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert_eq!(inv.owned["weapon_357"], 1);
        scene.time = 3.7;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert_eq!(inv.owned["weapon_357"], 5);
        assert_eq!(inv.reserve_for("weapon_357", &defs), 0);
        assert_eq!(inv.pistol_ammo, 20);
        assert_eq!(inv.reserve_for("weapon_smg1", &defs), 90);
    }
    #[test]
    fn switching_cancels_reload_without_refilling_either_weapon() {
        let defs = definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_pistol", &defs, 0.);
        inv.owned.insert("weapon_pistol".into(), 1);
        inv.pistol_ammo = 20;
        inv.reload(&defs, &mut scene, &world);
        inv.give("weapon_ar2", &defs, 0.3);
        inv.owned.insert("weapon_ar2".into(), 3);
        inv.give_ammo("AR2", 10, &defs);
        scene.time = 4.;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert_eq!(inv.owned["weapon_ar2"], 3);
        assert_eq!(inv.reserve_for("weapon_ar2", &defs), 10);
        assert_eq!(inv.owned["weapon_pistol"], 1);
        assert_eq!(inv.pistol_ammo, 20);
        assert!(!inv.is_reloading());
    }
    #[test]
    fn shotgun_inserts_one_shell_and_retains_it_when_interrupted() {
        let defs = definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut inv = Inventory::default();
        let mut physics = Physics::new(&world);
        inv.give("weapon_shotgun", &defs, 0.);
        inv.owned.insert("weapon_shotgun".into(), 0);
        inv.give_ammo("Buckshot", 3, &defs);
        inv.reload(&defs, &mut scene, &world);
        scene.time = 0.49;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert_eq!(inv.owned["weapon_shotgun"], 0);
        scene.time = 0.5;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert_eq!(inv.owned["weapon_shotgun"], 1);
        assert_eq!(inv.reserve_for("weapon_shotgun", &defs), 2);
        assert_eq!(inv.animation, "reload2");
        scene.time = 0.6;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert!(inv.delayed_attack);
        assert_eq!(inv.shots, 0);
        scene.time = 0.9;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 1);
        assert_eq!(inv.bullets, 7);
        assert_eq!(inv.owned["weapon_shotgun"], 0);
        assert_eq!(inv.reserve_for("weapon_shotgun", &defs), 2);
    }
    #[test]
    fn automatic_cadence_catches_up_without_an_idle_burst() {
        let defs = definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut inv = Inventory::default();
        let mut physics = Physics::new(&world);
        inv.give("weapon_smg1", &defs, 0.);
        scene.time = 1.;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        // After three float32 0.075 intervals, rather than just before the rounded deadline.
        scene.time = 1.226;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 4);
        assert_eq!(inv.owned["weapon_smg1"], 41);
        scene.time = 10.;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        scene.time = 20.;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 5);
        assert_eq!(inv.animation, "fire01");
    }
    fn automatic_definitions() -> BTreeMap<String, Weapon> {
        let mut defs = definitions();
        for (class, sound) in [
            ("weapon_smg1", "Weapon_SMG1.Empty"),
            ("weapon_ar2", "Weapon_AR2.Empty"),
        ] {
            defs.get_mut(class)
                .unwrap()
                .sounds
                .insert("empty".into(), sound.into());
        }
        defs
    }
    #[test]
    fn automatic_empty_trigger_clicks_before_reloading_while_held() {
        let defs = automatic_definitions();
        let world = World::default();
        for class in ["weapon_smg1", "weapon_ar2"] {
            let mut scene = Scene::new(&world);
            let mut physics = Physics::new(&world);
            let mut inv = Inventory::default();
            inv.give(class, &defs, 0.);
            inv.owned.insert(class.into(), 0);
            inv.give_ammo(&defs[class].ammo_type, 5, &defs);
            let draw_deadline = inv.next_attack;
            let draw_animation = inv.animation.clone();

            scene.time = 1.;
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
            inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
            assert!(!inv.is_reloading());
            assert_eq!(
                scene.sounds,
                [crate::sounds::SoundRequest::from(
                    defs[class].sounds["empty"].clone()
                )]
            );
            assert_eq!(inv.next_attack, draw_deadline);
            assert_eq!(inv.animation, draw_animation);
            assert_eq!(inv.animation_at, 0.);

            scene.time = 1.015;
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
            inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
            assert!(inv.is_reloading());
            let reload_animation = if class == "weapon_ar2" {
                "ir_reload"
            } else {
                "reload"
            };
            assert_eq!(inv.animation, reload_animation);
            let completion = 1.015 + fallback_duration(class, reload_animation);
            assert_eq!(inv.next_attack, completion);
            assert_eq!(inv.shots, 0);

            scene.time = completion - 0.001;
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
            assert_eq!(inv.owned[class], 0);
            assert_eq!(inv.reserve_for(class, &defs), 5);
            scene.time = completion;
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
            inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
            assert!(!inv.is_reloading());
            assert_eq!(inv.owned[class], 4);
            assert_eq!(inv.reserve_for(class, &defs), 0);
            assert_eq!(inv.shots, 1);
        }
    }
    #[test]
    fn automatic_empty_sound_throttle_survives_trigger_release() {
        let defs = automatic_definitions();
        let world = World::default();
        for class in ["weapon_smg1", "weapon_ar2"] {
            let mut scene = Scene::new(&world);
            let mut physics = Physics::new(&world);
            let mut inv = Inventory::default();
            inv.give(class, &defs, 0.);
            inv.owned.insert(class.into(), 0);
            let draw_deadline = inv.next_attack;
            let draw_animation = inv.animation.clone();
            for (time, clicks) in [(1., 1), (1.1, 1), (1.3, 1), (1.5, 1), (1.5001, 2)] {
                scene.time = time;
                inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
                inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
                inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
                assert_eq!(scene.sounds.len(), clicks);
                assert_eq!(inv.next_attack, draw_deadline);
                assert_eq!(inv.animation, draw_animation);
                assert_eq!(inv.shots, 0);
                assert!(!inv.is_reloading());
            }
        }
    }
    #[test]
    fn automatic_idle_reload_requires_deadline_to_have_elapsed() {
        let defs = definitions();
        let world = World::default();
        for class in ["weapon_smg1", "weapon_ar2"] {
            let mut scene = Scene::new(&world);
            let mut inv = Inventory::default();
            inv.give(class, &defs, 0.);
            inv.owned.insert(class.into(), 0);
            inv.give_ammo(&defs[class].ammo_type, 5, &defs);
            scene.time = inv.next_attack;
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
            assert!(!inv.is_reloading());
            scene.time += 0.015;
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
            assert!(inv.is_reloading());
            assert_eq!(inv.animation_at, scene.time);
        }
    }
    #[test]
    fn automatic_empty_sound_is_per_weapon_and_survives_switches() {
        let defs = automatic_definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_smg1", &defs, 0.);
        inv.owned.insert("weapon_smg1".into(), 0);
        inv.owned.insert("weapon_ar2".into(), 0);
        scene.time = 1.;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        // Keep draw deadlines out of this sound-state comparison. Ordinary
        // switching still sets its recorded draw duration.
        inv.give("weapon_ar2", &defs, 0.);
        scene.time = 1.1;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(scene.sounds, ["Weapon_SMG1.Empty", "Weapon_AR2.Empty"]);
        inv.give("weapon_smg1", &defs, 0.);
        scene.time = 1.2;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(scene.sounds.len(), 2);
    }
    #[test]
    fn revolver_cooldown_and_circular_spread_bounds() {
        let defs = definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut inv = Inventory::default();
        let mut physics = Physics::new(&world);
        inv.give("weapon_357", &defs, 0.);
        for time in [1., 1.1, 1.74, 1.75] {
            scene.time = time;
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
            inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        }
        assert_eq!(inv.shots, 2);
        for direction in [Vec3::X, Vec3::Z] {
            for _ in 0..1000 {
                let spread = inv.spread(direction, cone(10.));
                assert!(spread.is_finite());
                assert!(spread.angle_between(direction) <= cone(10.).atan() + 1e-5);
            }
        }
    }
    #[test]
    fn full_reserve_leaves_pickup_available_and_types_independent() {
        let defs = definitions();
        let mut world = World::default();
        world.entities.push(modkit_core::Entity {
            properties: vec![
                ("classname".into(), "item_ammo_ar2".into()),
                ("origin".into(), "0 0 24".into()),
            ],
        });
        let mut scene = Scene::new(&world);
        let mut inv = Inventory::default();
        assert_eq!(inv.give_ammo("AR2", 100, &defs), 60);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert!(!scene.states[0].killed);
        inv.set_ammo("AR2", 55);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert!(scene.states[0].killed);
        assert_eq!(inv.reserve_for("weapon_ar2", &defs), 60);
        assert_eq!(inv.reserve_for("weapon_pistol", &defs), 0);
    }

    #[test]
    fn secondary_ammo_pickups_clamp_independently_and_full_pickups_remain() {
        let defs = definitions();
        let mut inv = Inventory::default();
        assert_eq!(inv.give_ammo("smg1_grenade", 10, &defs), 3);
        assert_eq!(inv.secondary_for("weapon_smg1", &defs), 3);
        assert_eq!(inv.secondary_for("weapon_ar2", &defs), 0);
        assert_eq!(inv.reserve_for("weapon_smg1", &defs), 0);
        let mut world = World::default();
        world.entities.push(modkit_core::Entity {
            properties: vec![
                ("classname".into(), "item_ammo_smg1_grenade".into()),
                ("origin".into(), "0 0 24".into()),
            ],
        });
        let mut scene = Scene::new(&world);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert!(!scene.states[0].killed);
        inv.set_ammo("SMG1_Grenade", 2);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        assert!(scene.states[0].killed);
        assert_eq!(inv.secondary_for("weapon_smg1", &defs), 3);
    }

    #[test]
    fn developer_loadout_grants_native_secondary_quantities_and_draw_blocks_launch() {
        let mut defs = definitions();
        // Preserve the native fixed GiveAmmo amounts even if custom carry limits are larger.
        defs.get_mut("weapon_ar2").unwrap().secondary_ammo_max = 8;
        defs.get_mut("weapon_smg1").unwrap().secondary_ammo_max = 7;
        let mut inv = Inventory::default();
        inv.refill_ammo(&defs);
        assert_eq!(inv.secondary_for("weapon_smg1", &defs), 3);
        assert_eq!(inv.secondary_for("weapon_ar2", &defs), 5);
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        for class in ["weapon_smg1", "weapon_ar2"] {
            inv.give(class, &defs, 0.);
            let ammo = inv.secondary_for(class, &defs);
            inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
            assert_eq!(inv.secondary_for(class, &defs), ammo);
            assert_eq!(inv.shots, 0);
        }
        assert_eq!(inv.reserve_for("weapon_smg1", &defs), 225);
        assert_eq!(inv.reserve_for("weapon_ar2", &defs), 60);
    }

    fn projectile_definitions() -> BTreeMap<String, Weapon> {
        let mut defs = automatic_definitions();
        let smg = defs.get_mut("weapon_smg1").unwrap();
        smg.secondary_damage = 100.;
        smg.secondary_radius = 250.;
        smg.sounds
            .insert("double_shot".into(), "Weapon_SMG1.Double".into());
        let ar2 = defs.get_mut("weapon_ar2").unwrap();
        ar2.secondary_radius = 10.;
        ar2.secondary_mass = 150.;
        ar2.secondary_lifetime = 2.;
        ar2.sounds
            .insert("special1".into(), "Weapon_CombineGuard.Special1".into());
        ar2.sounds
            .insert("double_shot".into(), "Weapon_IRifle.Single".into());
        defs
    }
    #[test]
    fn smg_grenade_has_separate_ammo_and_deadlines_without_hitscan_damage() {
        let defs = projectile_definitions();
        let world = impact_wall();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_smg1", &defs, 0.);
        inv.refill_ammo(&defs);
        scene.time = 1.;
        inv.secondary_attack(
            &defs,
            &world,
            &mut scene,
            &mut physics,
            Vec3::Z * 64.,
            Vec3::X,
        );
        assert_eq!(inv.owned["weapon_smg1"], 45);
        assert_eq!(inv.secondary_for("weapon_smg1", &defs), 2);
        assert_eq!(inv.shots, 1);
        assert_eq!(inv.bullets, 0);
        assert!(inv.impacts.is_empty());
        assert_eq!(inv.animation, "altfire");
        assert_eq!(scene.sounds, ["Weapon_SMG1.Double"]);
        assert_eq!(inv.projectile_spawns.len(), 1);
        assert_eq!(inv.projectile_spawns[0].velocity, Vec3::X * 1000.);
        for time in [1.015, 1.49, 1.5, 1.999] {
            scene.time = time;
            inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        }
        assert_eq!(inv.projectile_spawns.len(), 1);
        scene.time = 1.5;
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.owned["weapon_smg1"], 44);
        scene.time = 2.;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.projectile_spawns.len(), 2);
        assert_eq!(inv.secondary_for("weapon_smg1", &defs), 1);
    }
    #[test]
    fn smg_grenade_cancels_unfinished_magazine_reload_and_keeps_reserves() {
        let defs = projectile_definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_smg1", &defs, 0.);
        inv.owned.insert("weapon_smg1".into(), 0);
        inv.give_ammo("SMG1", 45, &defs);
        inv.give_ammo("SMG1_Grenade", 1, &defs);
        scene.time = 1.;
        inv.reload(&defs, &mut scene, &world);
        assert!(inv.is_reloading());
        scene.time = 1.1;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert!(!inv.is_reloading());
        assert_eq!(inv.owned["weapon_smg1"], 0);
        assert_eq!(inv.reserve_for("weapon_smg1", &defs), 45);
        assert_eq!(inv.secondary_for("weapon_smg1", &defs), 0);
        scene.time = 2.5;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        assert_eq!(inv.owned["weapon_smg1"], 0);
        assert_eq!(inv.reserve_for("weapon_smg1", &defs), 45);
    }
    #[test]
    fn empty_projectile_secondary_uses_its_own_half_second_deadline() {
        let defs = projectile_definitions();
        let world = World::default();
        for class in ["weapon_smg1", "weapon_ar2"] {
            let mut scene = Scene::new(&world);
            let mut physics = Physics::new(&world);
            let mut inv = Inventory::default();
            inv.give(class, &defs, 0.);
            let primary_deadline = inv.next_attack;
            scene.time = 1.;
            inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
            scene.time = 1.49;
            inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
            assert_eq!(scene.sounds.len(), 1);
            assert_eq!(inv.animation, "dryfire");
            assert_eq!(inv.next_attack, primary_deadline);
            assert_eq!(inv.next_secondary[class], 1.5);
            assert!(inv.projectile_spawns.is_empty());
            assert!(inv.can_holster());
        }
    }
    #[test]
    fn ar2_charge_uses_current_aim_after_strict_deadline_and_vetoes_holster_reload() {
        let defs = projectile_definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_ar2", &defs, 0.);
        inv.refill_ammo(&defs);
        scene.time = 1.;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.animation, "shake");
        assert_eq!(inv.charge_until(), Some(1.5));
        assert_eq!(inv.secondary_for("weapon_ar2", &defs), 3);
        assert!(!inv.can_holster());
        inv.give("weapon_pistol", &defs, 1.1);
        assert!(inv.owned.contains_key("weapon_pistol"));
        assert_eq!(inv.active, "weapon_ar2");
        inv.owned.insert("weapon_ar2".into(), 20);
        inv.reload(&defs, &mut scene, &world);
        assert!(!inv.is_reloading());
        scene.time = 1.5;
        inv.advance_projectile_fire(&world, &mut scene, &defs, Vec3::Y * 10., Vec3::Y);
        assert!(inv.projectile_spawns.is_empty());
        scene.time = 1.515;
        inv.set_attack_input(false, false);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, false, 0.015);
        inv.advance_projectile_fire(&world, &mut scene, &defs, Vec3::Y * 10., Vec3::Y);
        assert!(inv.can_holster());
        assert_eq!(inv.projectile_spawns.len(), 1);
        assert_eq!(inv.projectile_spawns[0].position, Vec3::Y * 10.);
        assert_eq!(inv.projectile_spawns[0].velocity, Vec3::Y * 1000.);
        assert_eq!(inv.projectile_spawns[0].radius, 10.);
        assert_eq!(inv.projectile_spawns[0].lifetime, 2.);
        assert_eq!(inv.projectile_spawns[0].mass, 150.);
        assert_eq!(inv.secondary_for("weapon_ar2", &defs), 2);
        assert_eq!(inv.owned["weapon_ar2"], 20);
        assert_eq!(inv.shots, 1);
        assert_eq!(inv.bullets, 0);
        assert_eq!(inv.animation, "ir_fire2");
        assert_eq!(
            scene.sounds,
            ["Weapon_CombineGuard.Special1", "Weapon_IRifle.Single"]
        );
        inv.advance_projectile_fire(&world, &mut scene, &defs, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.projectile_spawns.len(), 1);
        scene.time = 2.015;
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.owned["weapon_ar2"], 20);
        scene.time = inv.owner_attack_until;
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert!(inv.owned["weapon_ar2"] < 20);
    }
    #[test]
    fn ar2_does_not_cancel_magazine_reload_for_a_secondary_charge() {
        let defs = projectile_definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_ar2", &defs, 0.);
        inv.refill_ammo(&defs);
        inv.owned.insert("weapon_ar2".into(), 10);
        scene.time = 1.;
        inv.reload(&defs, &mut scene, &world);
        scene.time = 1.1;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert!(inv.is_reloading());
        assert_eq!(inv.charge_until(), None);
        assert_eq!(inv.secondary_for("weapon_ar2", &defs), 3);
        assert!(inv.projectile_spawns.is_empty());
    }
    #[test]
    fn projectile_damage_applies_blast_armor_and_lethal_outputs_once() {
        use crate::projectiles::{Damage, DamageTarget};
        let world = World {
            entities: vec![modkit_core::Entity {
                properties: vec![
                    ("classname".into(), "npc_metropolice".into()),
                    ("health".into(), "90".into()),
                ],
            }],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory {
            armor: 50.,
            ..Default::default()
        };
        inv.apply_projectile_damage(
            vec![Damage {
                target: DamageTarget::Player,
                amount: 100.,
                dissolve: false,
                direction: Vec3::X,
                origin: Vec3::X,
            }],
            &world,
            &mut scene,
            &mut physics,
        );
        let blast = scene.player_damage.pop().unwrap();
        assert_eq!(
            (blast.amount, blast.kind),
            (100., crate::player_damage::DMG_BLAST)
        );
        crate::player_damage::PlayerDamage::default().take(&mut inv, blast);
        assert_eq!(inv.armor, 0.);
        assert_eq!(inv.health, 50.);
        let dissolve = Damage {
            target: DamageTarget::Entity(0),
            amount: 0.,
            dissolve: true,
            direction: Vec3::X,
            origin: Vec3::ZERO,
        };
        inv.apply_projectile_damage(vec![dissolve, dissolve], &world, &mut scene, &mut physics);
        assert!(scene.states[0].killed);
        assert!(!scene.states[0].visible);
        assert_eq!((inv.hits, inv.kills), (1, 1));
    }

    #[test]
    fn shotgun_primary_has_seven_impacts_with_an_exact_first_pellet() {
        let defs = definitions();
        let mut world = World::default();
        world.brushes.push(modkit_core::Brush {
            contents: 1,
            planes: vec![
                modkit_core::Plane {
                    normal: Vec3::X,
                    distance: 101.,
                },
                modkit_core::Plane {
                    normal: -Vec3::X,
                    distance: -100.,
                },
                modkit_core::Plane {
                    normal: Vec3::Y,
                    distance: 50.,
                },
                modkit_core::Plane {
                    normal: -Vec3::Y,
                    distance: 50.,
                },
                modkit_core::Plane {
                    normal: Vec3::Z,
                    distance: 50.,
                },
                modkit_core::Plane {
                    normal: -Vec3::Z,
                    distance: 50.,
                },
            ],
        });
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        physics.tick(0.015);
        let mut inv = Inventory::default();
        inv.give("weapon_shotgun", &defs, 0.);
        scene.time = 1.;
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 1);
        assert_eq!(inv.bullets, 7);
        assert_eq!(inv.impacts.len(), 7);
        assert!(inv.impacts[0].0.position.distance(Vec3::X * 100.) < 0.001);
        assert!(inv.impacts[1..]
            .iter()
            .any(|(hit, _)| hit.position.y.abs() > 0.1));
        assert_eq!(inv.owned["weapon_shotgun"], 5);
    }

    #[test]
    fn shotgun_fire_is_blocked_until_fire_and_pump_animations_finish() {
        let defs = definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_shotgun", &defs, 0.);
        scene.time = 1.;
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        scene.time = 1.334;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        assert_eq!(inv.animation, "pump");
        scene.time = 1.8;
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 1);
        scene.time = 1.868;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 2);
        assert_eq!(inv.owned["weapon_shotgun"], 4);
    }

    #[test]
    fn shotgun_secondary_consumes_two_shells_and_spreads_all_twelve_pellets() {
        let mut defs = shotgun_definitions();
        // The secondary count is fixed independently of the primary pellet cvar.
        defs.get_mut("weapon_shotgun").unwrap().pellets = 3;
        let world = impact_wall();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        physics.tick(0.015);
        let mut inv = Inventory::default();
        inv.give("weapon_shotgun", &defs, 0.);
        inv.give_ammo("Buckshot", 10, &defs);
        scene.time = 1.;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.owned["weapon_shotgun"], 4);
        assert_eq!(inv.reserve_for("weapon_shotgun", &defs), 10);
        assert_eq!(inv.shots, 1);
        assert_eq!(inv.bullets, 12);
        assert_eq!(inv.impacts.len(), 12);
        assert!(inv.impacts[0].0.position.distance(Vec3::X * 100.) > 0.1);
        assert_eq!(inv.animation, "altfire");
        assert_eq!(scene.sounds, ["Weapon_Shotgun.Double"]);
        assert!(inv.shotgun_need_pump);
        assert_eq!(inv.next_attack, 1.5);
    }

    #[test]
    fn shotgun_secondary_with_one_shell_uses_primary_and_preserves_primary_latch() {
        let defs = shotgun_definitions();
        let world = impact_wall();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        physics.tick(0.015);
        let mut inv = Inventory::default();
        inv.give("weapon_shotgun", &defs, 0.);
        inv.owned.insert("weapon_shotgun".into(), 1);
        inv.delayed_attack = true;
        inv.delayed_secondary_attack = true;
        scene.time = 1.;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.owned["weapon_shotgun"], 0);
        assert_eq!(inv.shots, 1);
        assert_eq!(inv.bullets, 7);
        assert!(inv.impacts[0].0.position.distance(Vec3::X * 100.) < 0.001);
        assert_eq!(inv.animation, "fire01");
        assert_eq!(scene.sounds, ["Weapon_Shotgun.Single"]);
        assert!(!inv.shotgun_need_pump);
        assert!(inv.delayed_attack);
        assert!(!inv.delayed_secondary_attack);
        assert!((inv.next_attack - (1. + 10. / 30.)).abs() < 1e-8);
    }

    #[test]
    fn shotgun_secondary_interrupt_waits_for_two_shells_and_retains_insertion_deadline() {
        let defs = shotgun_definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_shotgun", &defs, 0.);
        inv.owned.insert("weapon_shotgun".into(), 0);
        inv.give_ammo("Buckshot", 4, &defs);
        inv.reload(&defs, &mut scene, &world);
        for time in [0.5, 0.6, 0.9] {
            scene.time = time;
            inv.set_attack_input(false, true);
            inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
            inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
            assert!(inv.is_reloading());
            assert!(!inv.delayed_secondary_attack);
            assert_eq!(inv.shots, 0);
        }
        assert_eq!(inv.owned["weapon_shotgun"], 2);
        let insertion_end = inv.next_attack;
        scene.time = 1.;
        inv.set_attack_input(false, true);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert!(!inv.is_reloading());
        assert!(inv.delayed_secondary_attack);
        assert!(!inv.delayed_attack);
        assert_eq!(inv.next_attack, insertion_end);
        assert_eq!(inv.shots, 0);
        // Released buttons do not lose the queued shot or cancel its deadline.
        scene.time = insertion_end - 0.001;
        inv.set_attack_input(false, false);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 0);
        scene.time = insertion_end;
        inv.set_attack_input(false, false);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 1);
        assert_eq!(inv.bullets, 12);
        assert_eq!(inv.owned["weapon_shotgun"], 0);
        assert_eq!(inv.reserve_for("weapon_shotgun", &defs), 2);
        assert!(!inv.delayed_secondary_attack);
        assert!(!inv.shotgun_need_pump);
    }

    #[test]
    fn shotgun_secondary_and_pump_wait_for_loaded_clip_durations() {
        let defs = shotgun_definitions();
        let mut world = World::default();
        let mut rig = modkit_core::animation::Rig::default();
        // Different fixture durations prove that loaded model clips override fallbacks.
        for (name, frame_count) in [("altfire", 26), ("pump", 11)] {
            rig.clips.insert(
                name.into(),
                modkit_core::animation::Clip {
                    layer: Default::default(),
                    fps: 25.,
                    looping: false,
                    frames: vec![Vec::new(); frame_count],
                    events: Vec::new(),
                },
            );
        }
        world
            .rigs
            .insert("models/weapons/v_shotgun.mdl#0".into(), rig);
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_shotgun", &defs, 0.);
        scene.time = 1.;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.next_attack, 2.);
        scene.time = 1.999;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.animation, "altfire");
        assert_eq!(inv.shots, 1);
        scene.time = 2.;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        assert_eq!(inv.animation, "pump");
        let pump_end = inv.next_attack;
        assert!((pump_end - 2.4).abs() < 1e-6);
        scene.time = pump_end - 0.001;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 1);
        scene.time = pump_end;
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 2);
        assert_eq!(inv.owned["weapon_shotgun"], 2);
        assert_eq!(
            scene.sounds,
            [
                "Weapon_Shotgun.Double",
                "Weapon_Shotgun.Special1",
                "Weapon_Shotgun.Double"
            ]
        );
    }

    #[test]
    fn shotgun_both_buttons_latch_primary_but_dispatch_secondary_first() {
        let defs = shotgun_definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_shotgun", &defs, 0.);
        inv.owned.insert("weapon_shotgun".into(), 3);
        inv.give_ammo("Buckshot", 3, &defs);
        scene.time = 1.;
        inv.reload(&defs, &mut scene, &world);
        scene.time = 1.1;
        inv.set_attack_input(true, true);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        assert!(inv.delayed_attack);
        assert!(!inv.delayed_secondary_attack);
        assert!(!inv.is_reloading());
        assert_eq!(inv.next_attack, 1.5);
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 0);
        scene.time = 1.5;
        inv.set_attack_input(true, true);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        // The host dispatches secondary first even though reload latched primary.
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 1);
        assert_eq!(inv.bullets, 12);
        assert_eq!(inv.owned["weapon_shotgun"], 1);
        assert!(inv.delayed_attack);
        scene.time = 2.;
        inv.set_attack_input(false, false);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.animation, "pump");
        assert_eq!(inv.shots, 1);
        scene.time = inv.next_attack;
        inv.set_attack_input(false, false);
        inv.tick(&world, &mut scene, &defs, Vec3::ZERO, true, 0.015);
        inv.attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 2);
        assert_eq!(inv.bullets, 19);
        assert_eq!(inv.owned["weapon_shotgun"], 0);
        assert!(!inv.delayed_attack);
    }

    #[test]
    fn shotgun_empty_secondary_dry_fires_and_switching_clears_both_latches() {
        let defs = shotgun_definitions();
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut inv = Inventory::default();
        inv.give("weapon_shotgun", &defs, 0.);
        inv.owned.insert("weapon_shotgun".into(), 0);
        inv.delayed_secondary_attack = true;
        scene.time = 1.;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.shots, 0);
        assert_eq!(inv.animation, "dryfire");
        assert_eq!(scene.sounds, ["Weapon_Shotgun.Empty"]);
        assert!(!inv.delayed_secondary_attack);
        inv.delayed_attack = true;
        inv.delayed_secondary_attack = true;
        inv.set_attack_input(false, true);
        inv.give("weapon_pistol", &defs, 1.1);
        assert!(!inv.delayed_attack);
        assert!(!inv.delayed_secondary_attack);
        assert!(inv.attack_input.is_none());
        scene.time = 3.;
        inv.secondary_attack(&defs, &world, &mut scene, &mut physics, Vec3::ZERO, Vec3::X);
        assert_eq!(inv.owned["weapon_pistol"], 18);
        assert_eq!(inv.shots, 0);
    }

    #[test]
    fn animation_sounds_suppress_only_the_same_symbolic_engine_sound() {
        let mut weapon = Weapon {
            viewmodel: "test".into(),
            ..Default::default()
        };
        weapon
            .sounds
            .insert("reload".into(), "Weapon_AR2.Reload".into());
        let mut world = World::default();
        let mut rig = modkit_core::animation::Rig::default();
        rig.clips.insert(
            "ir_reload".into(),
            modkit_core::animation::Clip {
                layer: Default::default(),
                fps: 30.,
                looping: false,
                frames: Vec::new(),
                events: vec![modkit_core::animation::ClipEvent {
                    cycle: 0.5,
                    id: 5004,
                    flags: 0,
                    name: String::new(),
                    options: "Weapon_AR2.Reload_Rotate".into(),
                }],
            },
        );
        world.rigs.insert("test#0".into(), rig);
        let mut scene = Scene::new(&world);
        play_sound(&mut scene, &weapon, "reload", &world, "ir_reload");
        assert_eq!(scene.sounds, ["Weapon_AR2.Reload"]);
        scene.sounds.clear();
        world
            .rigs
            .get_mut("test#0")
            .unwrap()
            .clips
            .get_mut("ir_reload")
            .unwrap()
            .events[0]
            .options = "Weapon_AR2.Reload".into();
        play_sound(&mut scene, &weapon, "reload", &world, "ir_reload");
        assert!(scene.sounds.is_empty());
    }
}
