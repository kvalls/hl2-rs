//! NPC weapons (DESIGN 14w): the SDK CBaseCombatWeapon as held by an NPC operator.
//! CAI_BaseNPC::Spawn equips `additionalequipment` for classes with bits_CAP_USE_WEAPONS
//! (Weapon_Create + Weapon_Equip), Weapon_CanUse needs every required acttable activity on
//! the NPC's model, Weapon_TranslateActivity maps activities through the weapon's acttable,
//! and the operator's animation events 3000-3999 reach the active weapon
//! (Operator_HandleAnimEvent), which fires FireBullets or the melee swing. Definitions
//! come from the installed weapon scripts and skill.cfg (`sk_npc_dmg_*`) plus SDK 2013
//! constants (cones, bursts, rates, ranges, proficiency tables). SDK behavior; retail
//! numbers are not compared yet.
use crate::npc_acttables::ACTTABLES;
use anyhow::{Context, Result};
use glam::Vec3;
use serde::Serialize;
use source_assets::{keyvalues, vpk::Vfs};
use std::collections::BTreeMap;

/// npcevent.h weapon animation events.
pub const EVENT_WEAPON_MELEE_HIT: i32 = 3001;
pub const EVENT_WEAPON_SMG1: i32 = 3002;
pub const EVENT_WEAPON_SHOTGUN_FIRE: i32 = 3004;
pub const EVENT_WEAPON_AR2: i32 = 3007;
pub const EVENT_WEAPON_PISTOL_FIRE: i32 = 3014;
pub const EVENT_WEAPON_RELOAD: i32 = 3015;
pub const EVENT_WEAPON_RELOAD_SOUND: i32 = 3017;
pub const EVENT_WEAPON_RELOAD_FILL_CLIP: i32 = 3018;

/// HL2 campaign classes with bits_CAP_USE_WEAPONS (CapabilitiesAdd in their Spawn:
/// npc_metropolice, npc_combine (soldiers), CNPC_PlayerCompanion (citizens, Barney),
/// npc_alyx, npc_monk).
pub const WEAPON_USERS: &[&str] = &[
    "npc_metropolice",
    "npc_combine_s",
    "npc_citizen",
    "npc_barney",
    "npc_alyx",
    "npc_monk",
];
pub fn uses_weapons(class: &str) -> bool {
    WEAPON_USERS.iter().any(|c| c.eq_ignore_ascii_case(class))
}

/// The weapons NPCs carry in HL2 (npc_create_equipment / additionalequipment values).
pub const NPC_WEAPONS: &[&str] = &[
    "weapon_stunstick",
    "weapon_pistol",
    "weapon_smg1",
    "weapon_ar2",
    "weapon_shotgun",
    "weapon_crowbar",
    "weapon_rpg",
    "weapon_alyxgun",
    "weapon_annabelle",
    "weapon_citizenpackage",
    "weapon_citizensuitcase",
];

/// Every weapon the map's outputs give an NPC by name (`<name>,GiveWeapon,<weapon>`), by
/// lowercase target name: Source creates and precaches them at the input, so the host
/// prepares their held models at load.
pub fn given_weapons(world: &modkit_core::World) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entity in &world.entities {
        for (key, value) in &entity.properties {
            let Some(output) = crate::entities::output(key, value) else {
                continue;
            };
            if output.input.eq_ignore_ascii_case("giveweapon") {
                let weapons = out.entry(output.target.to_ascii_lowercase()).or_default();
                let weapon = output.parameter.trim().to_ascii_lowercase();
                if !weapon.is_empty() && !weapons.contains(&weapon) {
                    weapons.push(weapon);
                }
            }
        }
    }
    out
}

/// WeaponProficiency_t.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Proficiency {
    Poor,
    Average,
    Good,
    VeryGood,
    Perfect,
}
impl Proficiency {
    fn index(self) -> usize {
        self as usize
    }
}
/// CalcWeaponProficiency. npc_metropolice: pistol poor, SMG1 very good; npc_combine_s:
/// AR2/SMG1 good, shotgun perfect; the companions: AR2 very good, else perfect; the base
/// class (and classes not reconstructed yet) average.
pub fn proficiency(npc_class: &str, weapon: &str) -> Proficiency {
    match (npc_class, weapon) {
        ("npc_metropolice", "weapon_pistol") => Proficiency::Poor,
        ("npc_metropolice", "weapon_smg1") => Proficiency::VeryGood,
        // CNPC_Combine (HL2, not episodic).
        ("npc_combine_s", "weapon_ar2" | "weapon_smg1") => Proficiency::Good,
        ("npc_combine_s", "weapon_shotgun") => Proficiency::Perfect,
        // CNPC_PlayerCompanion.
        ("npc_citizen" | "npc_barney" | "npc_alyx", "weapon_ar2") => Proficiency::VeryGood,
        ("npc_citizen" | "npc_barney" | "npc_alyx", _) => Proficiency::Perfect,
        _ => Proficiency::Average,
    }
}

/// VECTOR_CONE_nDEGREES: each component is sin(n / 2).
pub fn vector_cone(degrees: f32) -> f32 {
    (degrees.to_radians() * 0.5).sin()
}

/// CBaseHLCombatWeapon::GetDefaultProficiencyValues (spread scale, bias).
const DEFAULT_PROFICIENCY: [(f32, f32); 5] = [(2.5, 1.), (2., 1.), (1.5, 1.), (1.25, 1.), (1., 1.)];
/// CWeaponSMG1::GetProficiencyValues.
const SMG1_PROFICIENCY: [(f32, f32); 5] = [
    (7., 0.75),
    (5., 0.75),
    (10. / 3., 0.75),
    (5. / 3., 0.75),
    (1., 1.),
];
/// CWeaponAR2::GetProficiencyValues.
const AR2_PROFICIENCY: [(f32, f32); 5] = [
    (7., 0.75),
    (5., 0.75),
    (3., 0.85),
    (5. / 3., 0.75),
    (1., 1.),
];

/// An NPC weapon's FireNPCPrimaryAttack / Operator_HandleAnimEvent firing.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct FireSpec {
    /// The animation event that fires a shot.
    pub event: i32,
    pub pellets: u32,
    /// GetBulletSpread for an NPC owner, in degrees (VECTOR_CONE_n).
    pub cone_degrees: f32,
    /// VECTOR_CONE_PRECALCULATED: the trajectory already carries the spread; otherwise
    /// FireBullets spreads each pellet by the weapon cone again (the shotgun).
    pub precalculated: bool,
    pub min_burst: u32,
    pub max_burst: u32,
    /// GetFireRate (seconds between shots in a burst).
    pub fire_rate: f32,
    /// GetMinRestTime / GetMaxRestTime (the shot regulator's rest).
    pub min_rest: f32,
    pub max_rest: f32,
    /// m_fMinRange1 / m_fMaxRange1.
    pub min_range: f32,
    pub max_range: f32,
    pub proficiency: [(f32, f32); 5],
    /// Tracer every nth bullet (0: none).
    pub tracer_freq: u32,
    /// CSoundEnt combat sound volume (SOUNDENT_VOLUME_*).
    pub sound_volume: f32,
}
fn fire_spec(class: &str) -> Option<FireSpec> {
    Some(match class {
        "weapon_pistol" => FireSpec {
            event: EVENT_WEAPON_PISTOL_FIRE,
            pellets: 1,
            cone_degrees: 5.,
            precalculated: true,
            min_burst: 1,
            max_burst: 3,
            fire_rate: 0.5,
            min_rest: 0.3,
            max_rest: 0.6,
            min_range: 24.,
            max_range: 1500.,
            proficiency: DEFAULT_PROFICIENCY,
            tracer_freq: 2,
            sound_volume: 1500.,
        },
        "weapon_smg1" => FireSpec {
            event: EVENT_WEAPON_SMG1,
            pellets: 1,
            cone_degrees: 5.,
            precalculated: true,
            min_burst: 2,
            max_burst: 5,
            fire_rate: 0.075,
            min_rest: 0.3,
            max_rest: 0.6,
            min_range: 0.,
            max_range: 1400.,
            proficiency: SMG1_PROFICIENCY,
            tracer_freq: 2,
            sound_volume: 1500.,
        },
        "weapon_ar2" => FireSpec {
            event: EVENT_WEAPON_AR2,
            pellets: 1,
            cone_degrees: 3.,
            precalculated: true,
            min_burst: 2,
            max_burst: 5,
            fire_rate: 0.1,
            min_rest: 0.3,
            max_rest: 0.6,
            min_range: 65.,
            max_range: 2048.,
            proficiency: AR2_PROFICIENCY,
            tracer_freq: 2,
            sound_volume: 1500.,
        },
        "weapon_shotgun" => FireSpec {
            event: EVENT_WEAPON_SHOTGUN_FIRE,
            pellets: 8,
            cone_degrees: 10.,
            precalculated: false,
            min_burst: 1,
            max_burst: 3,
            fire_rate: 0.7,
            min_rest: 0.3,
            max_rest: 0.6,
            min_range: 0.,
            max_range: 500.,
            proficiency: DEFAULT_PROFICIENCY,
            tracer_freq: 0,
            sound_volume: 1500.,
        },
        // CWeaponAlyxGun (HL2, not episodic): CHLSelectFireMachineGun, WeaponSound (not
        // realtime) per EVENT_WEAPON_PISTOL_FIRE, VECTOR_CONE_2DEGREES, range 1..5000.
        "weapon_alyxgun" => FireSpec {
            event: EVENT_WEAPON_PISTOL_FIRE,
            pellets: 1,
            cone_degrees: 2.,
            precalculated: true,
            min_burst: 4,
            max_burst: 7,
            fire_rate: 0.1,
            min_rest: 0.3,
            max_rest: 0.6,
            min_range: 1.,
            max_range: 5000.,
            proficiency: DEFAULT_PROFICIENCY,
            tracer_freq: 2,
            sound_volume: 0.,
        },
        // CWeaponRPG: EVENT_WEAPON_SMG1 launches a missile (CMissile::Create) instead of
        // bullets; one at a time, GetFireRate 1, bursts of 1, 4 s rests, 480..6000 units
        // of horizontal distance, VECTOR_CONE_3DEGREES; no combat sound insertion.
        "weapon_rpg" => FireSpec {
            event: EVENT_WEAPON_SMG1,
            pellets: 1,
            cone_degrees: 3.,
            precalculated: true,
            min_burst: 1,
            max_burst: 1,
            fire_rate: 1.,
            min_rest: 4.,
            max_rest: 4.,
            min_range: 40. * 12.,
            max_range: 500. * 12.,
            proficiency: DEFAULT_PROFICIENCY,
            tracer_freq: 0,
            sound_volume: 0.,
        },
        _ => return None,
    })
}

/// CWeaponSMG1::Equip: 3000 units of range for a CLASS_PLAYER_ALLY owner, else 1400.
pub fn equip_max_range(weapon: &str, ally: bool, spec: &FireSpec) -> f32 {
    if weapon == "weapon_smg1" && ally {
        3000.
    } else {
        spec.max_range
    }
}

/// One NPC weapon type.
#[derive(Clone, Debug, Serialize)]
pub struct NpcWeaponDef {
    pub class: String,
    /// Script "playermodel" (the world model an NPC holds).
    pub world_model: String,
    pub max_clip1: i32,
    pub default_clip1: i32,
    pub ammo_type: String,
    /// sk_npc_dmg_<ammo> for bullets, sk_npc_dmg_<weapon> for melee weapons.
    pub damage: f32,
    pub sounds: BTreeMap<String, String>,
    pub fire: Option<FireSpec>,
    /// Swings on EVENT_WEAPON_MELEE_HIT (stunstick, crowbar).
    pub melee: bool,
}

/// NPC weapon definitions from the installed scripts and skill.cfg.
pub fn definitions(vfs: &Vfs) -> Result<BTreeMap<String, NpcWeaponDef>> {
    let skill = vfs.read("cfg/skill.cfg")?.context("skill.cfg missing")?;
    let tokens = keyvalues::tokens(&String::from_utf8_lossy(&skill))?;
    let number = |name: &str| {
        tokens
            .windows(2)
            .find(|p| p[0].eq_ignore_ascii_case(name))
            .and_then(|p| p[1].parse::<f32>().ok())
            .filter(|v| v.is_finite())
    };
    let mut out = BTreeMap::new();
    for class in NPC_WEAPONS {
        let Some(data) = vfs.read(&format!("scripts/{class}.txt"))? else {
            continue;
        };
        let script = keyvalues::parse(&String::from_utf8_lossy(&data))?;
        let Some(root) = script.first() else {
            continue;
        };
        let value = |key: &str| root.get(key).and_then(|e| e.text());
        let sounds = root
            .get("SoundData")
            .map(|e| {
                e.children()
                    .iter()
                    .filter_map(|s| s.text().map(|t| (s.key.to_lowercase(), t.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let max_clip1 = value("clip_size")
            .and_then(|v| v.parse().ok())
            .unwrap_or(-1);
        let ammo_type = value("primary_ammo").unwrap_or("None").to_owned();
        let melee = matches!(*class, "weapon_stunstick" | "weapon_crowbar");
        let damage = if melee {
            number(&format!(
                "sk_npc_dmg_{}",
                class.trim_start_matches("weapon_")
            ))
        } else {
            number(&format!("sk_npc_dmg_{}", ammo_type.to_lowercase()))
        }
        .unwrap_or(0.)
        .max(0.);
        out.insert(
            (*class).to_owned(),
            NpcWeaponDef {
                class: (*class).to_owned(),
                world_model: value("playermodel").unwrap_or_default().replace('\\', "/"),
                max_clip1,
                default_clip1: value("default_clip")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(max_clip1),
                ammo_type,
                damage,
                sounds,
                fire: fire_spec(class),
                melee,
            },
        );
    }
    Ok(out)
}

/// A definition without the installation (unit tests and synthetic worlds): SDK
/// constants with an unknown world model, a clip of one and no damage (skill.cfg absent).
pub fn builtin(class: &str) -> Option<NpcWeaponDef> {
    let class = NPC_WEAPONS.iter().find(|w| w.eq_ignore_ascii_case(class))?;
    let melee = matches!(*class, "weapon_stunstick" | "weapon_crowbar");
    Some(NpcWeaponDef {
        class: (*class).to_owned(),
        world_model: String::new(),
        max_clip1: if melee { -1 } else { 1 },
        default_clip1: if melee { -1 } else { 1 },
        ammo_type: "None".into(),
        damage: 0.,
        sounds: BTreeMap::new(),
        fire: fire_spec(class),
        melee,
    })
}

/// The weapon's acttable (weapon activity, translated activity, required).
pub fn acttable(weapon: &str) -> &'static [(&'static str, &'static str, bool)] {
    ACTTABLES
        .iter()
        .find(|(class, _)| class.eq_ignore_ascii_case(weapon))
        .map_or(&[], |(_, table)| *table)
}
/// Weapon_TranslateActivity: the acttable row for an activity.
pub fn translate(weapon: &str, activity: &str) -> Option<&'static str> {
    acttable(weapon)
        .iter()
        .find(|(from, _, _)| from.eq_ignore_ascii_case(activity))
        .map(|(_, to, _)| *to)
}
/// CBaseCombatCharacter::Weapon_CanUse: every required row's translated activity (after
/// the NPC's own NPC_TranslateActivity, identity for the classes modeled here) must have
/// a sequence on the model. A weapon without an acttable cannot be used.
pub fn can_use(weapon: &str, has_activity: &dyn Fn(&str) -> bool) -> bool {
    let table = acttable(weapon);
    !table.is_empty()
        && table
            .iter()
            .filter(|(_, _, required)| *required)
            .all(|(_, to, _)| has_activity(to))
}
/// The `additionalequipment` each class carries in the installed HL2 campaign maps
/// (read-only scan of the 79 maps of build 19307283, session 20; npc_maker children
/// not split by class). CAI_BaseNPC::Spawn equips these without Weapon_CanUse, which
/// rejects several of them on the retail models (police.mdl has no crouch-walk rifle
/// activities for weapon_smg1; combine_soldier.mdl has no ACT_RELOAD_SMG1).
pub const CAMPAIGN_LOADOUTS: &[(&str, &[&str])] = &[
    (
        "npc_metropolice",
        &["weapon_stunstick", "weapon_pistol", "weapon_smg1"],
    ),
    (
        "npc_combine_s",
        &["weapon_smg1", "weapon_ar2", "weapon_shotgun"],
    ),
    (
        "npc_citizen",
        &[
            "weapon_smg1",
            "weapon_ar2",
            "weapon_shotgun",
            "weapon_rpg",
            "weapon_alyxgun",
        ],
    ),
    ("npc_barney", &["weapon_smg1", "weapon_ar2"]),
    ("npc_alyx", &["weapon_alyxgun"]),
    ("npc_monk", &["weapon_annabelle"]),
];
pub fn campaign_loadout(npc_class: &str) -> &'static [&'static str] {
    CAMPAIGN_LOADOUTS
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(npc_class))
        .map_or(&[], |(_, w)| *w)
}
/// The weapons an NPC class can be given, for bits_CAP_USE_WEAPONS classes: its
/// campaign loadout (equipped at spawn without Weapon_CanUse) first, then the other
/// weapons Weapon_CanUse accepts on its model's activities (upper case; NPC pickups).
pub fn usable_weapons(
    npc_class: &str,
    activities: &std::collections::BTreeSet<String>,
    definitions: &BTreeMap<String, NpcWeaponDef>,
) -> Vec<String> {
    if !uses_weapons(npc_class) {
        return Vec::new();
    }
    let campaign = campaign_loadout(npc_class);
    campaign
        .iter()
        .copied()
        .chain(NPC_WEAPONS.iter().copied().filter(|w| {
            !campaign.contains(w) && can_use(w, &|a| activities.contains(&a.to_uppercase()))
        }))
        .filter(|w| definitions.contains_key(*w))
        .map(str::to_owned)
        .collect()
}
/// Activities to preload on an NPC model for a weapon (its acttable results).
pub fn preload_activities(weapon: &str) -> impl Iterator<Item = &'static str> {
    acttable(weapon).iter().map(|(_, to, _)| *to)
}

/// An equipped weapon (the active weapon; NPCs here hold one).
#[derive(Clone, Debug, Serialize)]
pub struct NpcWeapon {
    pub class: String,
    pub clip1: i32,
    /// Bullets fired (tracer cadence).
    pub shots: u32,
    /// The world model's current sequence (Weapon_SetActivity).
    pub sequence: Option<WeaponSequence>,
    /// CHLMachineGun::m_flNextSoundTime (WeaponSoundRealtime).
    pub next_sound_time: f64,
    /// CWeaponRPG::m_hMissile: the launched missile still flies.
    pub missile_out: bool,
}
/// A weapon world-model sequence started by Weapon_SetActivity: its events are
/// dispatched to the operator (CBaseCombatWeapon::Operator_FrameUpdate), with the
/// playback rate fitted so the sequence spans the NPC's sequence duration.
#[derive(Clone, Debug, Serialize)]
pub struct WeaponSequence {
    pub name: String,
    pub started: f64,
    /// The NPC sequence's duration the weapon sequence is stretched over (0: its own).
    pub npc_duration: f32,
    events_until: f64,
}
impl NpcWeapon {
    /// Weapon_Equip: the clip filled from the default clip (clipless weapons: -1).
    pub fn equip(definition: &NpcWeaponDef) -> Self {
        let clip1 = if definition.max_clip1 < 0 {
            -1
        } else {
            definition.default_clip1.min(definition.max_clip1).max(0)
        };
        Self {
            class: definition.class.clone(),
            clip1,
            shots: 0,
            sequence: None,
            next_sound_time: 0.,
            missile_out: false,
        }
    }
    /// CHLMachineGun::WeaponSoundRealtime(SINGLE_NPC), which FireNPCPrimaryAttack of
    /// weapon_smg1 and weapon_ar2 calls once per shot event: "make enough sound events to
    /// fill up the next estimated think interval" `dt` (the weapon's last animation
    /// interval, clamped to 0.2 s): up to two SINGLE_NPC sounds GetFireRate apart from
    /// m_flNextSoundTime (clamped to now). Returns their start times; later ones play
    /// ahead of the next think (WeaponSound's soundtime). Only one bullet is fired (the
    /// SDK's FIXME), so an NPC burst sounds faster than it shoots.
    pub fn realtime_sound_times(&mut self, now: f64, dt: f32, fire_rate: f32) -> Vec<f64> {
        if self.next_sound_time < now {
            self.next_sound_time = now;
        }
        let until = now + f64::from(dt.clamp(0., 0.2));
        let mut times = Vec::new();
        for _ in 0..2 {
            if self.next_sound_time < until {
                times.push(self.next_sound_time);
                self.next_sound_time += f64::from(fire_rate);
            }
        }
        times
    }
    /// Weapon_SetActivity(weapon activity, NPC sequence duration): the world model's
    /// sequence for the activity, ACT_VM_IDLE when the model has none
    /// (CBaseCombatWeapon::SetIdealActivity "FORCE IDLE").
    pub fn set_activity(
        &mut self,
        rig: &modkit_core::animation::Rig,
        activity: &str,
        npc_duration: f32,
        now: f64,
    ) {
        let find = |a: &str| {
            rig.sequences
                .iter()
                .find(|s| s.activity.eq_ignore_ascii_case(a))
                .map(|s| s.name.clone())
        };
        self.sequence = find(activity)
            .or_else(|| find("ACT_VM_IDLE"))
            .map(|name| WeaponSequence {
                name,
                started: now,
                npc_duration,
                // Cycle-0 events belong to the start of the sequence.
                events_until: now - 1e-6,
            });
    }
    /// The weapon sequence's events in the window since the last call (played once).
    pub fn due_events(
        &mut self,
        rig: &modkit_core::animation::Rig,
        now: f64,
    ) -> Vec<(String, i32, String)> {
        let Some(sequence) = self.sequence.as_mut() else {
            return Vec::new();
        };
        let Some(clip) = rig.clips.get(&sequence.name) else {
            return Vec::new();
        };
        let span = if sequence.npc_duration > 0. {
            sequence.npc_duration
        } else {
            clip.duration()
        }
        .max(0.015);
        let from = sequence.events_until;
        sequence.events_until = now;
        clip.events
            .iter()
            .filter(|e| {
                let at = sequence.started + f64::from(e.cycle * span);
                at > from && at <= now
            })
            .map(|e| (e.name.clone(), e.id, e.options.clone()))
            .collect()
    }
}
/// CHLMachineGun subclasses (their NPC shots use WeaponSoundRealtime; the shotgun and
/// pistol call WeaponSound(SINGLE_NPC) once).
pub fn machine_gun(class: &str) -> bool {
    matches!(class, "weapon_smg1" | "weapon_ar2")
}
/// The rig key of a weapon world model in `World::rigs`.
pub fn rig_key(world_model: &str) -> String {
    format!("{}#0", world_model.to_lowercase())
}

/// Which OnUpdateShotRegulator sets the regulator's ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum RegulatorRule {
    /// CAI_BaseNPC::OnUpdateShotRegulator: the weapon's GetFireRate (both burst interval
    /// bounds), GetMinBurst/GetMaxBurst and GetMinRestTime/GetMaxRestTime (0.3 / 0.6 s
    /// for the HL2 weapons modeled).
    Weapon,
    /// CNPC_MetroPolice::OnUpdateShotRegulator with a pistol (burst mode inactive): burst
    /// sizes and rests scaled by the distance to the enemy, shots U(0.2, 0.5) s apart.
    MetropolicePistol,
    /// CNPC_PlayerCompanion::OnUpdateShotRegulator (HL2, not episodic): the base ranges,
    /// then within PC_LARGER_BURST_RANGE (120 units) of an enemy it can range attack the
    /// remaining burst doubles and the rest interval bounds scale by 0.6.
    PlayerCompanion,
}

/// npc_playercompanion.cpp PC_LARGER_BURST_RANGE (10 feet).
const PC_LARGER_BURST_RANGE: f32 = 12. * 10.;
/// npc_metropolice.cpp MIN/MAX_PISTOL_MODIFY_DIST (units).
const PISTOL_MODIFY_DIST: (f32, f32) = (15. * 12., 150. * 12.);

/// CAI_ShotRegulator (ai_utils.cpp): bursts of min..=max shots U(min, max burst interval)
/// apart, then a rest of U(min_rest, max_rest). The owner's OnUpdateShotRegulator
/// (`rule`) sets the ranges at equip and whenever a burst ends (OnRangeAttack1).
#[derive(Clone, Debug, Serialize)]
pub struct ShotRegulator {
    pub rule: RegulatorRule,
    pub min_burst: u32,
    pub max_burst: u32,
    pub min_burst_interval: f32,
    pub max_burst_interval: f32,
    pub min_rest: f32,
    pub max_rest: f32,
    pub next_shot: f64,
    pub burst_remaining: u32,
    pub in_rest: bool,
    weapon_burst: (u32, u32),
    weapon_rest: (f32, f32),
    fire_rate: f32,
    seed: u64,
}
impl ShotRegulator {
    /// OnChangeActiveWeapon: OnUpdateShotRegulator (no enemy yet) + Reset(true).
    pub fn for_weapon(spec: &FireSpec, rule: RegulatorRule, now: f64, seed: u64) -> Self {
        let mut regulator = Self {
            rule,
            min_burst: spec.min_burst,
            max_burst: spec.max_burst,
            min_burst_interval: spec.fire_rate,
            max_burst_interval: spec.fire_rate,
            min_rest: 0.3,
            max_rest: 0.6,
            next_shot: now,
            burst_remaining: 1,
            in_rest: false,
            weapon_burst: (spec.min_burst, spec.max_burst),
            weapon_rest: (spec.min_rest, spec.max_rest),
            fire_rate: spec.fire_rate,
            seed: seed | 1,
        };
        regulator.on_update(None);
        regulator.reset(true, now);
        regulator
    }
    /// OnUpdateShotRegulator; `enemy_distance` is WorldSpaceCenter to the enemy's
    /// WorldSpaceCenter (None without an enemy).
    pub fn on_update(&mut self, enemy_distance: Option<f32>) {
        (self.min_burst, self.max_burst) = self.weapon_burst;
        (self.min_burst_interval, self.max_burst_interval) = (self.fire_rate, self.fire_rate);
        (self.min_rest, self.max_rest) = self.weapon_rest;
        if self.rule == RegulatorRule::MetropolicePistol {
            if let Some(distance) = enemy_distance {
                let (near, far) = PISTOL_MODIFY_DIST;
                let factor = (distance.clamp(near, far) - near) / (far - near);
                // The SDK's int initializers truncate.
                self.min_burst = (2. + 2. * (1. - factor)) as u32;
                self.max_burst = (5. + 3. * (1. - factor)) as u32;
                self.min_rest = 0.6 + 0.6 * factor;
                self.max_rest = 1.2 + 0.8 * factor;
            } else {
                self.min_rest = 0.6;
                self.max_rest = 1.4;
            }
            (self.min_burst_interval, self.max_burst_interval) = (0.2, 0.5);
        }
        if self.rule == RegulatorRule::PlayerCompanion
            && enemy_distance.is_some_and(|d| d <= PC_LARGER_BURST_RANGE)
        {
            self.burst_remaining *= 2;
            self.min_rest *= 0.6;
            self.max_rest *= 0.6;
        }
    }
    fn unit(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u32 << 24) as f32
    }
    /// Reset: a new burst size; shooting now, or after a rest.
    pub fn reset(&mut self, start_shooting: bool, now: f64) {
        let span = self.max_burst.saturating_sub(self.min_burst) + 1;
        self.burst_remaining = self.min_burst + ((self.unit() * span as f32) as u32).min(span - 1);
        if start_shooting {
            self.next_shot = now;
            self.in_rest = false;
        } else {
            let rest = self.min_rest + (self.max_rest - self.min_rest) * self.unit();
            self.next_shot = now + f64::from(rest);
            self.in_rest = true;
        }
    }
    /// FireNoEarlierThan.
    pub fn fire_no_earlier_than(&mut self, time: f64) {
        self.next_shot = self.next_shot.max(time);
    }
    pub fn should_shoot(&self, now: f64) -> bool {
        self.next_shot <= now
    }
    pub fn is_in_rest_interval(&self, now: f64) -> bool {
        self.in_rest && !self.should_shoot(now)
    }
    /// OnFiredWeapon.
    pub fn on_fired(&mut self, now: f64) {
        self.burst_remaining = self.burst_remaining.saturating_sub(1);
        if self.burst_remaining == 0 {
            self.reset(false, now);
        } else {
            self.in_rest = false;
            let interval = self.min_burst_interval
                + (self.max_burst_interval - self.min_burst_interval) * self.unit();
            self.next_shot = (self.next_shot + f64::from(interval)).max(now);
        }
    }
    /// CAI_BaseNPC::OnRangeAttack1: OnFiredWeapon, then OnUpdateShotRegulator once the
    /// burst ended (the new ranges apply from the next Reset).
    pub fn on_range_attack1(&mut self, now: f64, enemy_distance: Option<f32>) {
        self.on_fired(now);
        if self.is_in_rest_interval(now) {
            self.on_update(enemy_distance);
        }
    }
}

/// CShotManipulator::ApplySpread: circular gaussian spread with the ai_shot_bias_min/max
/// (-1, 1) range; `uniform` returns values in [-1, 1]. The result is normalized.
pub fn apply_spread(
    direction: Vec3,
    spread: f32,
    bias: f32,
    uniform: &mut dyn FnMut() -> f32,
) -> Vec3 {
    let direction = direction.normalize_or_zero();
    let shot_bias = 2. * bias.clamp(0., 1.) - 1.;
    let flatness = shot_bias.abs() * 0.5;
    let (x, y) = loop {
        let mut x = uniform() * flatness + uniform() * (1. - flatness);
        let mut y = uniform() * flatness + uniform() * (1. - flatness);
        if shot_bias < 0. {
            x = if x >= 0. { 1. - x } else { -1. - x };
            y = if y >= 0. { 1. - y } else { -1. - y };
        }
        if x * x + y * y <= 1. {
            break (x, y);
        }
    };
    let (right, up) = vector_vectors(direction);
    (direction + right * (x * spread) + up * (y * spread)).normalize_or_zero()
}
/// mathlib VectorVectors: right and up for a forward vector.
pub fn vector_vectors(forward: Vec3) -> (Vec3, Vec3) {
    if forward.x == 0. && forward.y == 0. {
        (Vec3::new(0., -1., 0.), Vec3::new(-forward.z, 0., 0.))
    } else {
        let right = forward.cross(Vec3::Z).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        (right, up)
    }
}

/// A FireBullets request from an NPC weapon, resolved by the host's bullet path.
#[derive(Clone, Debug, Serialize)]
pub struct NpcShot {
    pub attacker: usize,
    pub weapon: String,
    pub origin: Vec3,
    /// Pellet directions (spread already applied).
    pub directions: Vec<Vec3>,
    pub damage: f32,
    pub ammo_type: String,
    /// Tracer on this shot (every `tracer_freq` bullets).
    pub tracer: bool,
    /// An RPG launch (CMissile::Create along `directions[0]`) instead of bullets.
    pub missile: bool,
}

/// GetActualShootTrajectory for a shot from `origin` at a target point (the enemy's
/// LKP plus its noisy BodyTarget offset): the proficiency spread with the focus-scaled
/// bias (GetSpreadBias: ai_spread_pattern_focus_time 0.8 s after the enemy became
/// valid), then, for weapons whose FireBullets spreads again, one direction per pellet.
pub fn shot_directions(
    spec: &FireSpec,
    proficiency: Proficiency,
    origin: Vec3,
    target: Vec3,
    since_valid_enemy: f32,
    uniform: &mut dyn FnMut() -> f32,
) -> Vec<Vec3> {
    let (scale, bias) = spec.proficiency[proficiency.index()];
    let focus = (since_valid_enemy.max(0.) / 0.8).min(1.);
    let aim = apply_spread(
        target - origin,
        vector_cone(spec.cone_degrees) * scale,
        bias * focus,
        uniform,
    );
    if spec.precalculated {
        return vec![aim];
    }
    (0..spec.pellets)
        .map(|_| apply_spread(aim, vector_cone(spec.cone_degrees), 1., uniform))
        .collect()
}

/// What an operator's weapon animation event did.
#[derive(Clone, Debug, PartialEq)]
pub enum WeaponEvent {
    /// Not a weapon event, or no weapon handles it.
    None,
    /// EVENT_WEAPON_MELEE_HIT on a melee weapon: the operator's swing trace runs.
    MeleeHit,
    /// A shot was fired (clip decremented); the request is to be resolved by the host.
    Fired,
    /// EVENT_WEAPON_RELOAD: the clip refilled, with WeaponSound(RELOAD_NPC).
    Reloaded,
    /// EVENT_WEAPON_RELOAD_SOUND: WeaponSound(RELOAD_NPC) only.
    ReloadSound,
    /// EVENT_WEAPON_RELOAD_FILL_CLIP: the clip refilled silently.
    FilledClip,
}
impl NpcWeapon {
    /// Operator_HandleAnimEvent for event `id`. `aim` gives (shoot origin, target point,
    /// seconds since the enemy became valid) when the operator has a target.
    #[allow(clippy::too_many_arguments)]
    pub fn handle_event(
        &mut self,
        id: i32,
        definition: &NpcWeaponDef,
        attacker: usize,
        npc_class: &str,
        aim: Option<(Vec3, Vec3, f32)>,
        uniform: &mut dyn FnMut() -> f32,
        shots: &mut Vec<NpcShot>,
    ) -> WeaponEvent {
        if id == EVENT_WEAPON_MELEE_HIT && definition.melee {
            return WeaponEvent::MeleeHit;
        }
        // CAI_BaseNPC::HandleAnimEvent's reload events act on the active weapon.
        match id {
            EVENT_WEAPON_RELOAD => {
                self.clip1 = definition.max_clip1;
                return WeaponEvent::Reloaded;
            }
            EVENT_WEAPON_RELOAD_SOUND => return WeaponEvent::ReloadSound,
            EVENT_WEAPON_RELOAD_FILL_CLIP => {
                self.clip1 = definition.max_clip1;
                return WeaponEvent::FilledClip;
            }
            _ => {}
        }
        let Some(spec) = definition.fire.filter(|s| s.event == id) else {
            return WeaponEvent::None;
        };
        let Some((origin, target, since_valid)) = aim else {
            return WeaponEvent::None;
        };
        let missile = self.class == "weapon_rpg";
        if missile && self.missile_out {
            return WeaponEvent::None;
        }
        self.missile_out |= missile;
        let directions = shot_directions(
            &spec,
            proficiency(npc_class, &self.class),
            origin,
            target,
            since_valid,
            uniform,
        );
        self.shots += 1;
        if self.clip1 > 0 {
            self.clip1 -= 1;
        }
        shots.push(NpcShot {
            attacker,
            weapon: self.class.clone(),
            origin,
            directions,
            damage: definition.damage,
            ammo_type: definition.ammo_type.clone(),
            tracer: spec.tracer_freq > 0 && self.shots.is_multiple_of(spec.tracer_freq),
            missile,
        });
        WeaponEvent::Fired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: u64) -> impl FnMut() -> f32 {
        let mut state = seed;
        move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as u32 as f32 / u32::MAX as f32) * 2. - 1.
        }
    }

    #[test]
    fn machine_gun_npc_shots_fill_the_think_interval_with_realtime_sounds() {
        let mut smg = NpcWeapon::equip(&builtin("weapon_smg1").unwrap());
        // A soldier's shot every 0.21 s (shootSMG1s, 0.133 s, ends at the second think):
        // two sounds 0.075 apart per shot.
        let close = |a: &[f64], b: &[f64]| {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
        };
        assert!(close(
            &smg.realtime_sound_times(1., 0.105, 0.075),
            &[1., 1.075]
        ));
        assert!(close(
            &smg.realtime_sound_times(1.21, 0.105, 0.075),
            &[1.21, 1.285]
        ));
        // A cop's shot every think (shoot_smg1, 0.1 s): the pending time continues.
        assert!(close(
            &smg.realtime_sound_times(1.315, 0.105, 0.075),
            &[1.36]
        ));
        assert!(close(
            &smg.realtime_sound_times(1.42, 0.105, 0.075),
            &[1.435, 1.51]
        ));
        // dt clamps to 0.2; the AR2 (0.1 s) still fits two in a 0.105 s think.
        let mut ar2 = NpcWeapon::equip(&builtin("weapon_ar2").unwrap());
        assert!(close(&ar2.realtime_sound_times(2., 0.105, 0.1), &[2., 2.1]));
        assert!(close(&ar2.realtime_sound_times(3., 5., 0.1), &[3., 3.1]));
        assert!(machine_gun("weapon_smg1") && machine_gun("weapon_ar2"));
        assert!(!machine_gun("weapon_shotgun") && !machine_gun("weapon_pistol"));
    }
    #[test]
    fn metropolice_pistol_regulator_scales_with_distance() {
        let spec = fire_spec("weapon_pistol").unwrap();
        let mut r = ShotRegulator::for_weapon(&spec, RegulatorRule::MetropolicePistol, 0., 7);
        // Equip without an enemy: the weapon's bursts, rest 0.6..1.4, shots 0.2..0.5 apart.
        assert_eq!((r.min_burst, r.max_burst), (1, 3));
        assert_eq!((r.min_rest, r.max_rest), (0.6, 1.4));
        assert_eq!((r.min_burst_interval, r.max_burst_interval), (0.2, 0.5));
        // Native canals fixture distance (~335 units): bursts 3..=7, rest ~0.66..1.28.
        r.on_update(Some(335.));
        assert_eq!((r.min_burst, r.max_burst), (3, 7));
        assert!((r.min_rest - 0.657).abs() < 0.01 && (r.max_rest - 1.277).abs() < 0.01);
        // Clamped at 15 ft and 150 ft.
        r.on_update(Some(10.));
        assert_eq!(
            (r.min_burst, r.max_burst, r.min_rest, r.max_rest),
            (4, 8, 0.6, 1.2)
        );
        r.on_update(Some(5000.));
        assert_eq!(
            (r.min_burst, r.max_burst, r.min_rest, r.max_rest),
            (2, 5, 1.2, 2.0)
        );
        // Shots within a burst land 0.2..0.5 s apart; the burst end starts a rest.
        r.on_update(Some(335.));
        r.reset(true, 10.);
        let shots = r.burst_remaining;
        assert!((3..=7).contains(&shots));
        let mut last = 10.;
        for _ in 1..shots {
            r.on_range_attack1(last, Some(335.));
            assert!(!r.in_rest);
            let gap = r.next_shot - last;
            assert!((0.2..=0.5).contains(&gap), "{gap}");
            last = r.next_shot;
        }
        r.on_range_attack1(last, Some(335.));
        assert!(r.is_in_rest_interval(last));
        let rest = r.next_shot - last;
        assert!((0.65..=1.28).contains(&rest), "{rest}");
        // Other weapons keep the weapon's fixed rate.
        let smg = fire_spec("weapon_smg1").unwrap();
        let w = ShotRegulator::for_weapon(&smg, RegulatorRule::Weapon, 0., 7);
        assert_eq!(
            (w.min_burst_interval, w.max_burst_interval),
            (smg.fire_rate, smg.fire_rate)
        );
    }

    #[test]
    fn capability_and_required_activities_decide_weapon_use() {
        let mut activities = std::collections::BTreeSet::new();
        for a in ["ACT_MELEE_ATTACK_SWING", "ACT_IDLE_ANGRY_MELEE"] {
            activities.insert(a.to_owned());
        }
        let defs: BTreeMap<_, _> = NPC_WEAPONS
            .iter()
            .map(|w| {
                (
                    (*w).to_owned(),
                    NpcWeaponDef {
                        class: (*w).to_owned(),
                        world_model: String::new(),
                        max_clip1: -1,
                        default_clip1: -1,
                        ammo_type: "None".into(),
                        damage: 0.,
                        sounds: BTreeMap::new(),
                        fire: fire_spec(w),
                        melee: false,
                    },
                )
            })
            .collect();
        // A Barney model with only the baton activities: his campaign loadout (SMG1,
        // AR2) first, then the stunstick and crowbar (same required rows) by
        // Weapon_CanUse, not the pistol; the package and suitcase have no required rows.
        let usable = usable_weapons("npc_barney", &activities, &defs);
        assert_eq!(
            usable,
            [
                "weapon_smg1",
                "weapon_ar2",
                "weapon_stunstick",
                "weapon_crowbar",
                "weapon_citizenpackage",
                "weapon_citizensuitcase"
            ]
        );
        // A zombie has no bits_CAP_USE_WEAPONS: nothing, whatever its model has.
        assert!(usable_weapons("npc_zombie", &activities, &defs).is_empty());
        assert_eq!(
            translate("weapon_stunstick", "ACT_MELEE_ATTACK1"),
            Some("ACT_MELEE_ATTACK_SWING")
        );
        assert_eq!(
            translate("weapon_pistol", "ACT_IDLE"),
            Some("ACT_IDLE_PISTOL")
        );
    }

    #[test]
    fn spread_stays_in_the_cone_and_bias_concentrates_it() {
        let mut uniform = rng(7);
        let forward = Vec3::X;
        let cone = vector_cone(5.) * 2.5;
        let mut sum_gaussian = 0.;
        let mut sum_flat = 0.;
        for _ in 0..4000 {
            let d = apply_spread(forward, cone, 1., &mut uniform);
            let off = (d - forward * d.dot(forward)).length() / d.dot(forward);
            assert!(off <= cone * 1.0001, "{off} > {cone}");
            sum_gaussian += off;
            sum_flat += {
                let d = apply_spread(forward, cone, 0.5, &mut uniform);
                (d - forward * d.dot(forward)).length() / d.dot(forward)
            };
        }
        // Bias 1 (gaussian) clusters nearer the aim point than bias 0.5 (flat).
        assert!(sum_gaussian < sum_flat, "{sum_gaussian} {sum_flat}");
    }

    #[test]
    fn pistol_event_fires_one_round_and_reload_refills() {
        let def = NpcWeaponDef {
            class: "weapon_pistol".into(),
            world_model: "models/weapons/w_pistol.mdl".into(),
            max_clip1: 18,
            default_clip1: 18,
            ammo_type: "Pistol".into(),
            damage: 3.,
            sounds: BTreeMap::new(),
            fire: fire_spec("weapon_pistol"),
            melee: false,
        };
        let mut weapon = NpcWeapon::equip(&def);
        assert_eq!(weapon.clip1, 18);
        let mut shots = Vec::new();
        let mut uniform = rng(3);
        let aim = Some((Vec3::new(0., 0., 55.), Vec3::new(500., 0., 60.), 2.));
        // The melee event does nothing for a pistol; the SMG1 event neither.
        for id in [EVENT_WEAPON_MELEE_HIT, EVENT_WEAPON_SMG1] {
            let e = weapon.handle_event(
                id,
                &def,
                5,
                "npc_metropolice",
                aim,
                &mut uniform,
                &mut shots,
            );
            assert_eq!(e, WeaponEvent::None);
        }
        let e = weapon.handle_event(
            EVENT_WEAPON_PISTOL_FIRE,
            &def,
            5,
            "npc_metropolice",
            aim,
            &mut uniform,
            &mut shots,
        );
        assert_eq!(e, WeaponEvent::Fired);
        assert_eq!(
            (weapon.clip1, shots.len(), shots[0].directions.len()),
            (17, 1, 1)
        );
        assert_eq!(shots[0].damage, 3.);
        // Poor proficiency: within 2.5 x the 5-degree cone of the target.
        let d = shots[0].directions[0];
        let to = (Vec3::new(500., 0., 60.) - Vec3::new(0., 0., 55.)).normalize();
        assert!(d.angle_between(to).to_degrees() < 2.5 * 2.6);
        weapon.handle_event(
            EVENT_WEAPON_RELOAD,
            &def,
            5,
            "npc_metropolice",
            aim,
            &mut uniform,
            &mut shots,
        );
        assert_eq!(weapon.clip1, 18);
    }

    #[test]
    fn shotgun_spreads_eight_pellets_around_the_trajectory() {
        let spec = fire_spec("weapon_shotgun").unwrap();
        let mut uniform = rng(11);
        let d = shot_directions(
            &spec,
            Proficiency::Average,
            Vec3::ZERO,
            Vec3::new(300., 0., 0.),
            2.,
            &mut uniform,
        );
        assert_eq!(d.len(), 8);
        assert!(d.windows(2).any(|p| p[0] != p[1]));
    }
    #[test]
    #[ignore = "requires owned installed HL2"]
    fn owned_weapon_columns_follow_campaign_loadouts_then_weapon_can_use() {
        let vfs =
            source_assets::vpk::Vfs::mount(&source_assets::install::discover().unwrap()).unwrap();
        let defs = definitions(&vfs).unwrap();
        let usable = |class: &str, model: &str| {
            let rig = source_assets::animation::load(&vfs, model, &Default::default()).unwrap();
            usable_weapons(class, &rig.activities, &defs)
        };
        // Retail police.mdl fails Weapon_CanUse for the SMG1 its maps give it; the
        // campaign loadout keeps it, CanUse adds the crowbar and the alyxgun.
        assert_eq!(
            usable("npc_metropolice", "models/police.mdl"),
            [
                "weapon_stunstick",
                "weapon_pistol",
                "weapon_smg1",
                "weapon_crowbar",
                "weapon_alyxgun"
            ]
        );
        // combine_soldier.mdl passes CanUse for nothing (no ACT_RELOAD_SMG1).
        assert_eq!(
            usable("npc_combine_s", "models/combine_soldier.mdl"),
            ["weapon_smg1", "weapon_ar2", "weapon_shotgun"]
        );
        let citizen = usable("npc_citizen", "models/humans/group01/male_07.mdl");
        assert!(
            !citizen.contains(&"weapon_pistol".to_owned()),
            "{citizen:?}"
        );
        assert!(citizen.starts_with(&["weapon_smg1".to_owned()]));
        assert!(usable("npc_zombie", "models/zombie/classic.mdl").is_empty());
    }
    #[test]
    fn humanoid_model_preloads_cover_every_acttable_result_of_their_weapons() {
        let preloaded = source_assets::models::HUMANOID_AI_ACTIVITIES;
        for weapon in [
            "weapon_smg1",
            "weapon_ar2",
            "weapon_shotgun",
            "weapon_pistol",
            "weapon_rpg",
            "weapon_alyxgun",
            "weapon_crowbar",
        ] {
            for (_, to, _) in acttable(weapon) {
                assert!(preloaded.contains(to), "{weapon}: {to} not preloaded");
            }
        }
    }
}
