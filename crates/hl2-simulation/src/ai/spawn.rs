//! Developer NPC spawning: the F4 overlay's data model (class and weapon choice, spawn
//! at the crosshair) and the Source console commands npc_create / npc_create_aimed /
//! npc_create_equipment (ai_concommands.cpp CC_NPC_Create, CC_NPC_Create_Aimed). The
//! class/weapon lists are developer choices, not SDK data. Placement follows the SDK.
//! Hidden template entities per class are appended at map load (`append_pool`), so
//! every load-time system (models, animation, physics, AI) loads and builds that
//! class like a map NPC; each spawn then creates a new entity copied from a template
//! (DESIGN 14r part A: `instantiate`), so any number can be created, like
//! CreateEntityByName after the class is precached.
use glam::Vec3;
use serde::Serialize;

/// Classes offered by the overlay (developer choice, not SDK data). The weapon column
/// comes from the SDK rules instead (bits_CAP_USE_WEAPONS + Weapon_CanUse on the
/// loaded model; `Overlay::set_usable`).
pub const SPAWN_CLASSES: &[&str] = &[
    "npc_metropolice",
    "npc_combine_s",
    "npc_combine_s_shotgunner",
    "npc_combine_s_elite",
    "npc_combine_s_prisonguard",
    "npc_citizen",
    "npc_citizen_rebel",
    "npc_citizen_refugee",
    "npc_citizen_medic",
    "npc_barney",
    "npc_alyx",
    "npc_vortigaunt",
    "npc_dog",
    "npc_kleiner",
    "npc_eli",
    "npc_mossman",
    "npc_breen",
    "npc_gman",
    "npc_monk",
    "npc_zombie",
    "npc_zombie_torso",
    "npc_fastzombie",
    "npc_poisonzombie",
    "npc_headcrab",
    "npc_headcrab_fast",
    "npc_headcrab_black",
    "npc_headcrab_poison",
    "npc_barnacle",
    "npc_antlion",
    "npc_antlionguard",
    "npc_ichthyosaur",
    "npc_crow",
    "npc_pigeon",
    "npc_seagull",
    "npc_stalker",
    "npc_manhack",
    "npc_cscanner",
    "npc_clawscanner",
    "npc_rollermine",
    "npc_turret_floor",
    "npc_turret_ceiling",
    "npc_combine_camera",
    "npc_sniper",
    "npc_strider",
    "npc_combinegunship",
    "npc_helicopter",
    "npc_combinedropship",
];
/// The overlay's Props tab when no installation listing was set (`Overlay::set_props`;
/// unit tests): common HL2 props, every one with a .phy, spawned like
/// `prop_physics_create <model>` (the argument is relative to models/).
pub const SPAWN_PROPS: &[&str] = &[
    "props_c17/oildrum001.mdl",
    "props_c17/oildrum001_explosive.mdl",
    "props_junk/wood_crate001a.mdl",
    "props_junk/wood_crate002a.mdl",
    "props_junk/cardboard_box001a.mdl",
    "props_junk/watermelon01.mdl",
    "props_junk/popcan01a.mdl",
    "props_junk/gascan001a.mdl",
    "props_junk/propane_tank001a.mdl",
    "props_junk/trafficcone001a.mdl",
    "props_junk/metal_paintcan001a.mdl",
    "props_junk/garbage_metalcan001a.mdl",
    "props_junk/cinderblock01a.mdl",
    "props_junk/sawblade001a.mdl",
    "props_junk/harpoon002a.mdl",
    "props_c17/furniturechair001a.mdl",
    "props_c17/furnituretable001a.mdl",
    "props_c17/furniturestove001a.mdl",
    "props_c17/furniturebathtub001a.mdl",
    "props_interiors/furniture_couch01a.mdl",
    "props_wasteland/cafeteria_bench001a.mdl",
    "props_borealis/bluebarrel001.mdl",
    "props_c17/canister01a.mdl",
    "props_c17/metalpot001a.mdl",
    "props_c17/doll01.mdl",
    "props_lab/monitor01a.mdl",
    "props_debris/metal_panel01a.mdl",
    "props_vehicles/car002a_physics.mdl",
];
/// Plain names for the common props (shown and searchable in the Props tab).
pub const SPAWN_PROP_NAMES: &[(&str, &str)] = &[
    ("props_c17/oildrum001.mdl", "oil drum"),
    ("props_c17/oildrum001_explosive.mdl", "explosive barrel"),
    ("props_junk/wood_crate001a.mdl", "wooden crate"),
    ("props_junk/wood_crate002a.mdl", "wooden crate, long"),
    ("props_junk/cardboard_box001a.mdl", "cardboard box"),
    ("props_junk/watermelon01.mdl", "watermelon"),
    ("props_junk/popcan01a.mdl", "soda can"),
    ("props_junk/gascan001a.mdl", "gas can"),
    ("props_junk/propane_tank001a.mdl", "propane tank"),
    ("props_junk/trafficcone001a.mdl", "traffic cone"),
    ("props_junk/metal_paintcan001a.mdl", "paint can"),
    ("props_junk/garbage_metalcan001a.mdl", "metal can"),
    ("props_junk/cinderblock01a.mdl", "cinder block"),
    ("props_junk/sawblade001a.mdl", "sawblade"),
    ("props_junk/harpoon002a.mdl", "harpoon"),
    ("props_c17/furniturechair001a.mdl", "chair"),
    ("props_c17/furnituretable001a.mdl", "table"),
    ("props_c17/furniturestove001a.mdl", "stove"),
    ("props_c17/furniturebathtub001a.mdl", "bathtub"),
    ("props_interiors/furniture_couch01a.mdl", "couch"),
    ("props_wasteland/cafeteria_bench001a.mdl", "cafeteria bench"),
    ("props_borealis/bluebarrel001.mdl", "blue barrel"),
    ("props_c17/canister01a.mdl", "gas canister"),
    ("props_c17/metalpot001a.mdl", "metal pot"),
    ("props_c17/doll01.mdl", "doll"),
    ("props_lab/monitor01a.mdl", "monitor"),
    ("props_debris/metal_panel01a.mdl", "metal panel"),
    ("props_vehicles/car002a_physics.mdl", "car"),
];
/// NPC tab entries.
pub fn entry_count() -> usize {
    SPAWN_CLASSES.len()
}
/// An overlay entry's entity class and extra keyvalues: citizen variants are npc_citizen
/// with a `citizentype` (CT_REFUGEE 2, CT_REBEL 3) and the medic spawnflag; every pooled
/// citizen has SF_CITIZEN_RANDOM_HEAD, so CNPC_Citizen::SelectModel gives each slot its
/// own least-used head (`citizens::select_models`). Plain npc_citizen takes the map's
/// citizen type like native npc_create.
pub fn spawn_class(label: &str) -> (&str, Vec<(&'static str, String)>) {
    use crate::citizens::{SF_CITIZEN_MEDIC, SF_CITIZEN_RANDOM_HEAD};
    let head = SF_CITIZEN_RANDOM_HEAD.to_string();
    match label {
        "npc_citizen" => ("npc_citizen", vec![("spawnflags", head)]),
        "npc_citizen_rebel" => (
            "npc_citizen",
            vec![("citizentype", "3".into()), ("spawnflags", head)],
        ),
        "npc_citizen_refugee" => (
            "npc_citizen",
            vec![("citizentype", "2".into()), ("spawnflags", head)],
        ),
        "npc_citizen_medic" => (
            "npc_citizen",
            vec![
                ("citizentype", "3".into()),
                (
                    "spawnflags",
                    (SF_CITIZEN_RANDOM_HEAD | SF_CITIZEN_MEDIC).to_string(),
                ),
            ],
        ),
        // CNPC_Combine::CalcWeaponProficiency gives a shotgun holder COMBINE_SKIN_SHOTGUNNER
        // (1); the pool entity is built with it (the F4 shotgun choice on the plain
        // soldier keeps skin 0: skins are chosen at map load).
        "npc_combine_s_shotgunner" => ("npc_combine_s", vec![("skin", "1".into())]),
        // CNPC_CombineS::Precache: m_fIsElite for this model (sk_combine_guard_health,
        // AR2 combine balls).
        "npc_combine_s_elite" => (
            "npc_combine_s",
            vec![("model", "models/combine_super_soldier.mdl".into())],
        ),
        "npc_combine_s_prisonguard" => (
            "npc_combine_s",
            vec![("model", "models/combine_soldier_prisonguard.mdl".into())],
        ),
        class => (class, Vec::new()),
    }
}
/// The weapon the overlay preselects for an entry (developer choice; native npc_create
/// gives none unless npc_create_equipment is set). Used only when the class can use it.
pub fn default_weapon(label: &str) -> &'static str {
    match label {
        "npc_citizen_rebel" | "npc_citizen_medic" => "weapon_smg1",
        "npc_metropolice" => "weapon_stunstick",
        "npc_combine_s" | "npc_combine_s_prisonguard" => "weapon_smg1",
        "npc_combine_s_shotgunner" => "weapon_shotgun",
        "npc_combine_s_elite" => "weapon_ar2",
        "npc_barney" => "weapon_pistol",
        "npc_alyx" => "weapon_alyxgun",
        "npc_monk" => "weapon_annabelle",
        _ => "",
    }
}
/// Hidden pooled entities per overlay class (appended at map load).
pub const POOL_PER_CLASS: usize = 3;
/// Keyvalue marking a pooled spawn entity (not a Source key).
pub const POOL_KEY: &str = "hl2rs_spawn_pool";
/// Where pooled entities wait (far below any HL2 map).
pub const POOL_ORIGIN: Vec3 = Vec3::new(0., 0., -15000.);
/// The model each class's Spawn/Precache sets when the map gives none (SDK 2013
/// hl2/npc_*.cpp; citizens pick from groups at random in the SDK, one is used here).
pub fn default_model(class: &str) -> Option<&'static str> {
    Some(match class {
        "npc_metropolice" => "models/police.mdl",
        "npc_combine_s" | "npc_sniper" => "models/combine_soldier.mdl",
        "npc_citizen" => "models/humans/group01/male_07.mdl",
        "npc_barney" => "models/barney.mdl",
        "npc_alyx" => "models/alyx.mdl",
        "npc_vortigaunt" => "models/vortigaunt.mdl",
        "npc_dog" => "models/dog.mdl",
        "npc_kleiner" => "models/kleiner.mdl",
        "npc_eli" => "models/eli.mdl",
        "npc_mossman" => "models/mossman.mdl",
        "npc_breen" => "models/breen.mdl",
        "npc_gman" => "models/gman.mdl",
        "npc_monk" => "models/monk.mdl",
        "npc_zombie" => "models/zombie/classic.mdl",
        "npc_zombie_torso" => "models/zombie/classic_torso.mdl",
        "npc_fastzombie" => "models/zombie/fast.mdl",
        "npc_poisonzombie" => "models/zombie/poison.mdl",
        "npc_headcrab" => "models/headcrabclassic.mdl",
        "npc_headcrab_fast" => "models/headcrab.mdl",
        "npc_headcrab_black" | "npc_headcrab_poison" => "models/headcrabblack.mdl",
        "npc_barnacle" => "models/barnacle.mdl",
        "npc_antlion" => "models/antlion.mdl",
        "npc_antlionguard" => "models/antlion_guard.mdl",
        "npc_ichthyosaur" => "models/ichthyosaur.mdl",
        "npc_crow" => "models/crow.mdl",
        "npc_pigeon" => "models/pigeon.mdl",
        "npc_seagull" => "models/seagull.mdl",
        "npc_stalker" => "models/stalker.mdl",
        "npc_manhack" => "models/manhack.mdl",
        "npc_cscanner" | "npc_clawscanner" => "models/combine_scanner.mdl",
        "npc_rollermine" => "models/roller.mdl",
        "npc_turret_floor" => "models/combine_turrets/floor_turret.mdl",
        "npc_turret_ceiling" => "models/combine_turrets/ceiling_turret.mdl",
        "npc_combine_camera" => "models/combine_camera/combine_camera.mdl",
        "npc_strider" => "models/combine_strider.mdl",
        "npc_combinegunship" => "models/gunship.mdl",
        "npc_helicopter" => "models/combine_helicopter.mdl",
        "npc_combinedropship" => "models/combine_dropship.mdl",
        _ => return None,
    })
}
/// Pooled slots per class: one for the very large models (load time), else
/// POOL_PER_CLASS.
pub fn pool_count(class: &str) -> usize {
    match class {
        "npc_strider"
        | "npc_combinegunship"
        | "npc_helicopter"
        | "npc_combinedropship"
        | "npc_antlionguard"
        | "npc_ichthyosaur" => 1,
        _ => POOL_PER_CLASS,
    }
}
/// Pooled slots per overlay entry: plain npc_citizen keeps one template per random head
/// (each spawn picks the least-used head like CNPC_Citizen::SelectModel), the rest
/// `pool_count` of their class.
pub fn pool_slots(label: &str) -> usize {
    if label == "npc_citizen" {
        crate::citizens::RANDOM_HEADS.len()
    } else {
        pool_count(spawn_class(label).0)
    }
}
/// Append the hidden spawn pool to a freshly loaded map's entities (before models
/// load). Existing entity ids are unchanged; the pool goes at the end.
pub fn append_pool(world: &mut modkit_core::World) {
    for label in SPAWN_CLASSES {
        let (class, keyvalues) = spawn_class(label);
        let Some(model) = default_model(class) else {
            continue;
        };
        // Plain npc_citizen keeps one template per random head: each spawn picks the
        // least-used head like CNPC_Citizen::SelectModel (citizens::least_used_template);
        // the variants keep POOL_PER_CLASS heads chosen at load.
        let heads = *label == "npc_citizen";
        for slot in 0..pool_slots(label) {
            let mut properties: Vec<(String, String)> = vec![
                ("classname".into(), class.into()),
                (
                    "origin".into(),
                    format!("{} {} {}", POOL_ORIGIN.x, POOL_ORIGIN.y, POOL_ORIGIN.z),
                ),
                (POOL_KEY.into(), (*label).into()),
            ];
            if heads {
                // A fixed head: SelectModel's non-random branch adds the group folder.
                properties.push(("model".into(), crate::citizens::RANDOM_HEADS[slot].into()));
                world.entities.push(modkit_core::Entity { properties });
                continue;
            }
            // Citizens get their head from SelectModel (citizens::select_models).
            if class != "npc_citizen" && !keyvalues.iter().any(|(k, _)| *k == "model") {
                properties.push(("model".into(), model.into()));
            }
            properties.extend(keyvalues.iter().map(|(k, v)| ((*k).into(), v.clone())));
            if class == "npc_metropolice" {
                properties.push(("additionalequipment".into(), "weapon_stunstick".into()));
            }
            world.entities.push(modkit_core::Entity { properties });
        }
    }
}
/// Source's edict limit (MAX_EDICT_BITS 11, a compile-time constant tied to the
/// network entity handles): native npc_create past it fails in ED_Alloc ("no free
/// edicts"). Runtime spawns stop at this many live entities; the host's
/// `--max-edicts N` changes it (0: no limit), which native cannot.
pub static MAX_EDICTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(2048);
/// The hidden template entities of an overlay entry (a class or citizen variant), in
/// load order: spawns cycle through them (citizen templates carry different heads).
pub fn templates(world: &modkit_core::World, label: &str) -> Vec<usize> {
    world
        .entities
        .iter()
        .enumerate()
        .filter_map(|(id, e)| (e.get(POOL_KEY) == Some(label)).then_some(id))
        .collect()
}
/// A new entity copied from a template (DESIGN 14r part A): its keyvalues without the
/// pool marker, at `origin` facing `yaw` (DispatchSpawn + Teleport of CC_NPC_Create),
/// and its model instance (same model/skin/scale) for the new id. The caller appends
/// both (`Scene::spawn_entity`, then `world.model_instances`).
pub fn instantiate(
    world: &modkit_core::World,
    template: usize,
    id: usize,
    origin: Vec3,
    yaw: f32,
) -> (modkit_core::Entity, Option<modkit_core::ModelInstance>) {
    let mut entity = world.entities[template].clone();
    entity.properties.retain(|(k, _)| k != POOL_KEY);
    for (key, value) in [
        ("origin", format!("{} {} {}", origin.x, origin.y, origin.z)),
        ("angles", format!("0 {yaw} 0")),
    ] {
        entity
            .properties
            .retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        entity.properties.push((key.into(), value));
    }
    let instance = world
        .model_instances
        .iter()
        .find(|i| i.entity == Some(template))
        .map(|i| modkit_core::ModelInstance {
            origin,
            angles: Vec3::new(0., yaw, 0.),
            entity: Some(id),
            background: false,
            ..i.clone()
        });
    (entity, instance)
}
/// The first unused pooled entity of an overlay entry (a class or citizen variant).
pub fn free_slot(
    world: &modkit_core::World,
    label: &str,
    used: &std::collections::BTreeSet<usize>,
) -> Option<usize> {
    world
        .entities
        .iter()
        .enumerate()
        .find_map(|(id, e)| (e.get(POOL_KEY) == Some(label) && !used.contains(&id)).then_some(id))
}
/// Flying classes (bits_CAP_MOVE_FLY) are placed 36 units back along the aim.
fn flies(class: &str) -> bool {
    matches!(
        class,
        "npc_manhack"
            | "npc_cscanner"
            | "npc_clawscanner"
            | "npc_combinegunship"
            | "npc_helicopter"
            | "npc_combinedropship"
    )
}
/// MAX_TRACE_LENGTH.
pub const MAX_TRACE_LENGTH: f32 = 56755.84;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SpawnRequest {
    pub classname: String,
    pub name: Option<String>,
    pub equipment: String,
    /// npc_create_aimed: face the player's aim yaw (pitch and roll 0).
    pub aimed: bool,
    /// The F4 overlay's per-spawn side (npc_create: the class default).
    pub allegiance: Allegiance,
    /// The overlay entry (citizen variants); empty: `classname`.
    pub variant: String,
    /// The F4 NumGrenades keyvalue for classes that read it (CNPC_Combine); None: the
    /// entity's own value (npc_create: none, so 0 like native).
    pub grenades: Option<i32>,
    /// Created by ent_create: CC_Ent_Create's placement (no hull test, no flying offset).
    pub ent_create: bool,
    /// The yaw of an ent_create `angles` keyvalue (degrees).
    pub yaw: Option<f32>,
    /// ent_create's `squadname` keyvalue (CAI_BaseNPC m_SquadName).
    pub squad: Option<String>,
}
impl SpawnRequest {
    /// Keyvalues the new entity gets before it spawns (targetname, squadname).
    pub fn keyvalues(&self) -> Vec<(&'static str, String)> {
        let mut keyvalues = Vec::new();
        if let Some(name) = &self.name {
            keyvalues.push(("targetname", name.clone()));
        }
        if let Some(squad) = &self.squad {
            keyvalues.push(("squadname", squad.clone()));
        }
        keyvalues
    }
    /// The pool entry this request draws from.
    pub fn pool_label(&self) -> &str {
        if self.variant.is_empty() {
            &self.classname
        } else {
            &self.variant
        }
    }
}
/// The F4 overlay's per-spawn side toward the player, like an ai_relationship entity
/// aimed at the spawned NPC (Foe: D_HT, Friend: D_LI; Default: the class relationship
/// table). The opposite side (friends hating foes) needs NPC-vs-NPC enemies, which the
/// AI does not choose yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Allegiance {
    #[default]
    Default,
    Foe,
    Friend,
}
impl Allegiance {
    pub fn disposition(self) -> Option<super::relationships::Disposition> {
        use super::relationships::Disposition;
        match self {
            Self::Default => None,
            Self::Foe => Some(Disposition::HT),
            Self::Friend => Some(Disposition::LI),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "class default",
            Self::Foe => "foe (hates the player)",
            Self::Friend => "friend (likes the player)",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::Foe,
            Self::Foe => Self::Friend,
            Self::Friend => Self::Default,
        }
    }
}

/// Console state: the npc_create_equipment cvar.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Console {
    pub equipment: String,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum Command {
    Spawn(SpawnRequest),
    SetEquipment(String),
    /// prop_physics_create (or ent_create of a physics prop) with this model argument,
    /// relative to models/.
    SpawnProp(String),
}
/// The ent_create classes that are CPhysicsProp (they need a `model` keyvalue).
const PHYSICS_PROP_CLASSES: &[&str] = &[
    "prop_physics",
    "prop_physics_override",
    "prop_physics_multiplayer",
    "physics_prop",
];
impl Console {
    /// Parse one console line: `npc_create <class> [name]`, `npc_create_aimed <class>
    /// [name]`, `npc_create_equipment <weapon>`, `prop_physics_create <model>` (SDK
    /// CC_Prop_Physics_Create: exactly one argument, else nothing happens) and
    /// `ent_create <class> [key value ...]` (CC_Ent_Create; here only NPC classes, with
    /// targetname / squadname / additionalequipment / NumGrenades / angles, and physics
    /// props with a model).
    /// Ok(None) for other commands.
    pub fn parse(&mut self, line: &str) -> Result<Option<Command>, String> {
        let args = tokens(line);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let Some(command) = args.first() else {
            return Ok(None);
        };
        match command.to_ascii_lowercase().as_str() {
            "npc_create_equipment" => {
                let value = args
                    .get(1)
                    .copied()
                    .unwrap_or("")
                    .trim_matches('"')
                    .to_owned();
                self.equipment = value.clone();
                Ok(Some(Command::SetEquipment(value)))
            }
            c @ ("npc_create" | "npc_create_aimed") => {
                let class = args
                    .get(1)
                    .ok_or_else(|| format!("{c}: missing npc class"))?;
                if !class.starts_with("npc_") {
                    // CreateEntityByName succeeds only for CAI_BaseNPC classes.
                    return Err(format!("{c}: {class} is not an NPC class"));
                }
                // Our citizen variants (npc_citizen_rebel ...) are npc_citizen with
                // keyvalues; native npc_create has only npc_citizen.
                let (classname, _) = spawn_class(class);
                let variant = if classname != *class {
                    (*class).to_owned()
                } else {
                    String::new()
                };
                Ok(Some(Command::Spawn(SpawnRequest {
                    classname: classname.to_owned(),
                    // CC_NPC_Create names the NPC when given exactly two arguments.
                    name: (args.len() == 3).then(|| args[2].to_owned()),
                    equipment: self.equipment.clone(),
                    aimed: c == "npc_create_aimed",
                    allegiance: Allegiance::Default,
                    variant,
                    grenades: None,
                    ..Default::default()
                })))
            }
            "prop_physics_create" => {
                Ok((args.len() == 2)
                    .then(|| Command::SpawnProp(args[1].trim_matches('"').to_owned())))
            }
            "ent_create" => {
                let Some(class) = args
                    .get(1)
                    .map(|c| c.trim_matches('"').to_ascii_lowercase())
                else {
                    return Ok(None);
                };
                let pairs: Vec<(String, &str)> = args[2..]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|kv| {
                        (
                            kv[0].trim_matches('"').to_ascii_lowercase(),
                            kv[1].trim_matches('"'),
                        )
                    })
                    .collect();
                let value = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| *v);
                if PHYSICS_PROP_CLASSES.contains(&class.as_str()) {
                    // CBaseProp::Spawn removes a prop without a model.
                    let model = value("model")
                        .ok_or_else(|| format!("ent_create: {class} missing modelname, removed"))?;
                    return Ok(Some(Command::SpawnProp(model.to_owned())));
                }
                if class.starts_with("npc_") && spawn_class(&class).0 == class {
                    // ent_create does not read npc_create_equipment; the weapon comes from
                    // the additionalequipment keyvalue (CAI_BaseNPC::KeyValue).
                    return Ok(Some(Command::Spawn(SpawnRequest {
                        classname: class.clone(),
                        name: value("targetname").map(str::to_owned),
                        equipment: value("additionalequipment").unwrap_or("").to_owned(),
                        aimed: false,
                        allegiance: Allegiance::Default,
                        variant: String::new(),
                        grenades: value("numgrenades")
                            .filter(|_| reads_num_grenades(&class))
                            .and_then(|n| n.parse().ok()),
                        ent_create: true,
                        squad: value("squadname").map(str::to_owned),
                        // "pitch yaw roll" (CBaseEntity::KeyValue angles).
                        yaw: value("angles")
                            .and_then(|a| a.split_whitespace().nth(1)?.parse().ok()),
                    })));
                }
                Err(format!(
                    "ent_create: runtime creation of {class} is not implemented (NPC classes and physics props only)"
                ))
            }
            _ => Ok(None),
        }
    }
}

/// Console arguments like CCommand::Tokenize: whitespace separates them, a double-quoted
/// argument keeps its spaces (without the quotes).
pub fn tokens(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            args.push(chars.by_ref().take_while(|c| *c != '"').collect());
        } else {
            let mut arg = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '"' {
                    break;
                }
                arg.push(c);
                chars.next();
            }
            args.push(arg);
        }
    }
    args
}
/// Where the spawned NPC goes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Placement {
    pub origin: Vec3,
    /// Yaw in degrees (npc_create keeps the spawn angles, 0).
    pub yaw: f32,
}
/// World queries placement needs (MASK_NPCSOLID).
pub trait PlacementWorld {
    /// AI_TraceLine: the end point, or None when nothing is hit.
    fn trace_line(&self, start: Vec3, end: Vec3) -> Option<Vec3>;
    /// UTIL_DropToFloor: the floor position below, if any.
    fn drop_to_floor(&self, class: &str, origin: Vec3) -> Option<Vec3>;
    /// The 1-unit upward hull check: true when the NPC's hull fits.
    fn hull_fits(&self, class: &str, origin: Vec3) -> bool;
}
/// CC_NPC_Create / CC_NPC_Create_Aimed placement. Err for "Bad Position" (the SDK
/// removes the NPC). A miss leaves the NPC at its spawn origin (world origin here).
/// CC_Ent_Create: the aim end raised 12 units and dropped to the floor, for every class,
/// with no hull test; the yaw comes from its `angles` keyvalue.
pub fn place(
    request: &SpawnRequest,
    eye: Vec3,
    forward: Vec3,
    world: &dyn PlacementWorld,
) -> Result<Placement, String> {
    let forward = forward.normalize_or_zero();
    let yaw = request.yaw.unwrap_or(if request.aimed {
        forward.y.atan2(forward.x).to_degrees()
    } else {
        0.
    });
    let Some(end) = world.trace_line(eye, eye + forward * MAX_TRACE_LENGTH) else {
        return Ok(Placement {
            origin: Vec3::ZERO,
            yaw,
        });
    };
    let origin = if flies(&request.classname) && !request.ent_create {
        end - forward * 36.
    } else {
        let raised = end + Vec3::Z * 12.;
        world
            .drop_to_floor(&request.classname, raised)
            .unwrap_or(raised)
    };
    if !request.ent_create && !world.hull_fits(&request.classname, origin) {
        // The SDK message, then our diagnostics (aim end and the tested origin).
        return Err(format!(
            "Can't create {}.  Bad Position! (aim end {:.1} {:.1} {:.1}, origin {:.1} {:.1} {:.1})",
            request.classname, end.x, end.y, end.z, origin.x, origin.y, origin.z
        ));
    }
    Ok(Placement { origin, yaw })
}

pub fn reads_num_grenades(class: &str) -> bool {
    class.eq_ignore_ascii_case("npc_combine_s")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_pool_has_slots_for_every_overlay_class() {
        let mut world = modkit_core::World::default();
        append_pool(&mut world);
        assert_eq!(
            world.entities.len(),
            SPAWN_CLASSES.iter().map(|c| pool_slots(c)).sum::<usize>()
        );
        // Plain citizens: one template per random head, each with its fixed head.
        let heads: Vec<&str> = templates(&world, "npc_citizen")
            .into_iter()
            .map(|id| world.entities[id].get("model").unwrap())
            .collect();
        assert_eq!(heads, crate::citizens::RANDOM_HEADS);
        assert!(SPAWN_CLASSES
            .iter()
            .all(|c| default_model(spawn_class(c).0).is_some()));
        // Citizen variants: rebel slots carry CT_REBEL and a random head, no model.
        let rebel = free_slot(&world, "npc_citizen_rebel", &Default::default()).unwrap();
        assert_eq!(world.entities[rebel].class(), "npc_citizen");
        assert_eq!(world.entities[rebel].get("citizentype"), Some("3"));
        assert_eq!(world.entities[rebel].get("model"), None);
        let mut used = std::collections::BTreeSet::new();
        for _ in 0..POOL_PER_CLASS {
            used.insert(free_slot(&world, "npc_combine_s", &used).unwrap());
        }
        assert!(free_slot(&world, "npc_combine_s", &used).is_none());
        let cop = free_slot(&world, "npc_metropolice", &used).unwrap();
        assert_eq!(world.entities[cop].get("model"), Some("models/police.mdl"));
    }
    struct Floor;
    impl PlacementWorld for Floor {
        fn trace_line(&self, start: Vec3, end: Vec3) -> Option<Vec3> {
            // Ground plane z = 0, a wall at x = 1000.
            let d = end - start;
            let t_floor = if d.z < 0. {
                -start.z / d.z
            } else {
                f32::INFINITY
            };
            let t_wall = if d.x > 0. {
                (1000. - start.x) / d.x
            } else {
                f32::INFINITY
            };
            let t = t_floor.min(t_wall);
            (t <= 1.).then(|| start + d * t)
        }
        fn drop_to_floor(&self, _: &str, origin: Vec3) -> Option<Vec3> {
            Some(Vec3::new(origin.x, origin.y, 0.))
        }
        fn hull_fits(&self, _: &str, origin: Vec3) -> bool {
            origin.x < 990.
        }
    }
    #[test]
    fn prop_physics_create_and_ent_create_parse_like_the_sdk_commands() {
        let mut console = Console::default();
        // CC_Prop_Physics_Create: exactly one argument, else nothing happens.
        assert_eq!(
            console
                .parse("prop_physics_create props_c17/oildrum001")
                .unwrap(),
            Some(Command::SpawnProp("props_c17/oildrum001".into()))
        );
        assert_eq!(console.parse("prop_physics_create").unwrap(), None);
        assert_eq!(console.parse("prop_physics_create a b").unwrap(), None);
        // ent_create: keyvalue pairs; physics props need a model.
        assert_eq!(
            console
                .parse("ent_create prop_physics model models/props_junk/watermelon01.mdl")
                .unwrap(),
            Some(Command::SpawnProp(
                "models/props_junk/watermelon01.mdl".into()
            ))
        );
        assert!(console.parse("ent_create prop_physics").is_err());
        // NPCs take targetname/additionalequipment/NumGrenades, not npc_create_equipment.
        console
            .parse("npc_create_equipment weapon_stunstick")
            .unwrap();
        let Some(Command::Spawn(r)) = console
            .parse("ent_create npc_combine_s targetname s1 additionalequipment weapon_ar2 NumGrenades 3")
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (
                r.classname.as_str(),
                r.name.as_deref(),
                r.equipment.as_str(),
                r.grenades,
                r.aimed
            ),
            ("npc_combine_s", Some("s1"), "weapon_ar2", Some(3), false)
        );
        assert!(r.ent_create && r.yaw.is_none());
        assert_eq!(r.keyvalues(), [("targetname", "s1".to_string())]);
        let Some(Command::Spawn(r)) = console
            .parse("ent_create npc_metropolice squadname cops")
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(r.keyvalues(), [("squadname", "cops".to_string())]);
        // Quoted values keep their spaces: angles "pitch yaw roll".
        let Some(Command::Spawn(r)) = console
            .parse("ent_create npc_citizen \"angles\" \"0 135 0\"")
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(r.yaw, Some(135.));
        let Some(Command::Spawn(r)) = console.parse("ent_create npc_metropolice").unwrap() else {
            panic!()
        };
        assert_eq!((r.equipment.as_str(), r.grenades), ("", None));
        // Our citizen variants are overlay labels, not entity classes; other classes are
        // not runtime-creatable yet.
        assert!(console.parse("ent_create npc_citizen_rebel").is_err());
        assert!(console.parse("ent_create item_healthkit").is_err());
        assert_eq!(console.parse("ent_create").unwrap(), None);
    }
    #[test]
    fn console_commands_follow_source_semantics() {
        let mut console = Console::default();
        assert_eq!(
            console
                .parse("npc_create_equipment weapon_stunstick")
                .unwrap(),
            Some(Command::SetEquipment("weapon_stunstick".into()))
        );
        let Some(Command::Spawn(r)) = console.parse("npc_create npc_metropolice cop1").unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (
                r.classname.as_str(),
                r.name.as_deref(),
                r.equipment.as_str(),
                r.aimed
            ),
            ("npc_metropolice", Some("cop1"), "weapon_stunstick", false)
        );
        let Some(Command::Spawn(r)) = console.parse("npc_create_aimed npc_citizen").unwrap() else {
            panic!()
        };
        assert!(r.aimed && r.name.is_none());
        let Some(Command::Spawn(r)) = console.parse("npc_create npc_citizen_rebel").unwrap() else {
            panic!()
        };
        assert_eq!(
            (r.classname.as_str(), r.pool_label()),
            ("npc_citizen", "npc_citizen_rebel")
        );
        assert!(console.parse("npc_create prop_physics").is_err());
        assert!(console.parse("npc_create").is_err());
        assert_eq!(console.parse("sv_cheats 1").unwrap(), None);
    }
    #[test]
    fn placement_drops_raised_hits_to_the_floor_and_rejects_bad_positions() {
        let eye = Vec3::new(0., 0., 64.);
        let request = |class: &str, aimed| SpawnRequest {
            classname: class.into(),
            name: None,
            equipment: String::new(),
            aimed,
            ..Default::default()
        };
        let down = Vec3::new(1., 0., -0.25).normalize();
        let p = place(&request("npc_metropolice", true), eye, down, &Floor).unwrap();
        assert!(
            (p.origin - Vec3::new(256., 0., 0.)).length() < 1e-2,
            "{p:?}"
        );
        assert_eq!(p.yaw, 0.);
        let p = place(
            &request("npc_metropolice", false),
            eye,
            Vec3::new(0., 1., -0.25),
            &Floor,
        )
        .unwrap();
        assert_eq!(p.yaw, 0.);
        let p = place(&request("npc_manhack", true), eye, down, &Floor).unwrap();
        assert!((p.origin - (Vec3::new(256., 0., 0.) - down * 36.)).length() < 1e-2);
        // Into the wall: the hull does not fit.
        assert!(place(&request("npc_metropolice", true), eye, Vec3::X, &Floor).is_err());
        // ent_create: no hull test, no flying offset, the angles keyvalue's yaw.
        let ent = |class: &str| SpawnRequest {
            ent_create: true,
            yaw: Some(90.),
            ..request(class, false)
        };
        let p = place(&ent("npc_metropolice"), eye, Vec3::X, &Floor).unwrap();
        assert_eq!((p.origin, p.yaw), (Vec3::new(1000., 0., 0.), 90.));
        let p = place(&ent("npc_manhack"), eye, down, &Floor).unwrap();
        assert!((p.origin - Vec3::new(256., 0., 0.)).length() < 1e-2);
        // Straight up: no hit, the NPC stays at its spawn origin.
        assert_eq!(
            place(&request("npc_citizen", true), eye, Vec3::Z, &Floor)
                .unwrap()
                .origin,
            Vec3::ZERO
        );
    }
}
