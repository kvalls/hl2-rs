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
/// Models that CreateRagGib turns into client ragdolls at runtime (CZombie::Precache
/// precaches the halves): one hidden "raggib" template each, label `raggib <model>`.
pub const GIB_MODELS: &[&str] = &["models/zombie/classic_legs.mdl"];
pub fn gib_label(model: &str) -> String {
    format!("raggib {model}")
}
/// Append the hidden raggib templates (after `append_pool`, before models load).
pub fn append_gib_pool(world: &mut modkit_core::World) {
    for model in GIB_MODELS {
        world.entities.push(modkit_core::Entity {
            properties: vec![
                ("classname".into(), "raggib".into()),
                (
                    "origin".into(),
                    format!("{} {} {}", POOL_ORIGIN.x, POOL_ORIGIN.y, POOL_ORIGIN.z),
                ),
                ("model".into(), (*model).into()),
                (POOL_KEY.into(), gib_label(model)),
            ],
        });
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
    fn label(self) -> &'static str {
        match self {
            Self::Default => "class default",
            Self::Foe => "foe (hates the player)",
            Self::Friend => "friend (likes the player)",
        }
    }
    fn next(self) -> Self {
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

/// F4 overlay keys (keyboard or movement-script `spawn_overlay` actions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayKey {
    Toggle,
    Up,
    Down,
    Left,
    Right,
    Enter,
    /// Switch the NPC / Props tab.
    Tab,
    PageUp,
    PageDown,
    /// Start or stop typing into the Props filter.
    Filter,
    /// Delete the filter's last character (while typing).
    Backspace,
}
/// What an overlay key asks the host to do.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum OverlayCommand {
    Spawn(SpawnRequest),
    /// prop_physics_create with this argument (relative to models/).
    SpawnProp(String),
    /// Toggle notarget (the AI does not sense the player).
    ToggleNotarget,
    /// Toggle the AI think loop (ai_enable).
    ToggleAi,
    ToggleFriendlyFire,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum Column {
    #[default]
    Classes,
    Options,
}
/// One option row of the right column.
#[derive(Clone, Debug, PartialEq, Eq)]
enum OptionRow {
    /// A weapon ("" = none).
    Weapon(String),
    Allegiance,
    /// NumGrenades (classes that read it).
    Grenades,
    Notarget,
    Ai,
    /// Debug: the player's hits hurt friendly-immune NPCs (ours).
    FriendlyFire,
}
/// What a pointer click on a drawn panel row means (`Overlay::click`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum PanelTarget {
    Tab(Tab),
    /// The Props tab's filter box.
    Filter,
    /// An item of a column (its index in that column's list).
    Row(Column, usize),
    /// A "more above/below" note: scroll that column a page (-1 up, 1 down).
    Page(Column, isize),
}
/// A drawn panel row: text, selected (the current choice), under the cursor.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PanelRow {
    pub text: String,
    pub selected: bool,
    pub cursor: bool,
    #[serde(skip)]
    pub target: Option<PanelTarget>,
}
/// The overlay as the host draws it: a title, a tab bar and two columns.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Panel {
    pub title: String,
    /// Tab buttons (NPCs, Props; the filter box on the Props tab).
    pub tabs: Vec<PanelRow>,
    /// Column headings (left, right).
    pub headings: [String; 2],
    pub classes: Vec<PanelRow>,
    pub options: Vec<PanelRow>,
}
/// The F4 grenade counts Enter cycles through (a developer choice: native npc_create
/// spawns soldiers with NumGrenades 0, so they never throw; maps set it per soldier).
pub const GRENADE_CHOICES: &[i32] = &[0, 1, 2, 3, 5, 10, 99];
/// Classes whose spawn reads the NumGrenades keyvalue (CNPC_Combine::KeyValue).
pub fn reads_num_grenades(class: &str) -> bool {
    class.eq_ignore_ascii_case("npc_combine_s")
}
/// Class rows shown at once; the list scrolls to keep the selection visible.
pub const VISIBLE_CLASSES: usize = 14;
/// The overlay's two tabs (Tab switches): NPC classes, or physics props by folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum Tab {
    #[default]
    Npcs,
    Props,
}
/// The rows of `items` drawn around `selected` (at most `visible`, plus "more" notes), so
/// the selection is always visible.
fn scrolled_rows(
    items: &[String],
    selected: usize,
    focused: bool,
    visible: usize,
    column: Column,
) -> Vec<PanelRow> {
    let total = items.len();
    let first = selected
        .saturating_sub(visible / 2)
        .min(total.saturating_sub(visible));
    let last = (first + visible).min(total);
    let note = |text: String, page| PanelRow {
        text,
        selected: false,
        cursor: false,
        target: Some(PanelTarget::Page(column, page)),
    };
    let mut rows = Vec::new();
    if first > 0 {
        rows.push(note(format!("({first} more above)"), -1));
    }
    for (index, text) in items.iter().enumerate().take(last).skip(first) {
        rows.push(PanelRow {
            text: text.clone(),
            selected: index == selected,
            cursor: focused && index == selected,
            target: Some(PanelTarget::Row(column, index)),
        });
    }
    if last < total {
        rows.push(note(format!("({} more below)", total - last), 1));
    }
    rows
}
/// The Props tab's first folder: the common HL2 props (`SPAWN_PROPS`) that are installed.
pub const COMMON_FOLDER: &str = "* common";
/// A common prop's plain name ("explosive barrel"), if it has one.
pub fn prop_name(model: &str) -> Option<&'static str> {
    SPAWN_PROP_NAMES
        .iter()
        .find(|(m, _)| *m == model)
        .map(|(_, name)| *name)
}
/// Characters the Props filter accepts (model paths are lowercase ASCII).
fn filter_char(c: char) -> Option<char> {
    let c = c.to_ascii_lowercase();
    (c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '.' | '-' | ' ')).then_some(c)
}

/// The F4 overlay. NPC tab: a scrolling class column and an options column with the
/// weapons the selected class can use (`set_usable`), its allegiance, notarget and the AI
/// switch. Props tab: model folders (first the installed common props) and the selected
/// folder's physics props, narrowed by a typed filter. Keys, mouse clicks
/// (`click`) and the wheel (`scroll`) drive it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Overlay {
    pub open: bool,
    /// Shown state of the friendly-fire debug option (the host owns the switch).
    pub friendly_fire: bool,
    pub tab: Tab,
    pub class: usize,
    /// Props tab cursors: folder index and model index inside it.
    pub folder: usize,
    pub model: usize,
    /// Props tab filter (lowercase substring of the model path) and whether typed keys go
    /// into it.
    pub filter: String,
    pub typing: bool,
    /// List rows that fit on screen (host, from the window height; 0 = VISIBLE_CLASSES).
    pub rows_visible: usize,
    /// Physics props by folder (relative to models/), from the installation at map load.
    #[serde(skip)]
    props: Vec<(String, Vec<String>)>,
    pub column: Column,
    /// Cursor row in the options column.
    pub row: usize,
    /// The chosen weapon ("" = none).
    pub weapon: String,
    pub allegiance: Allegiance,
    /// The NumGrenades given to classes that read it (`reads_num_grenades`).
    pub grenades: i32,
    /// Usable weapons per class (bits_CAP_USE_WEAPONS + Weapon_CanUse on the model the
    /// spawn uses); classes missing here have none.
    usable: std::collections::BTreeMap<String, Vec<String>>,
}
impl Overlay {
    /// The weapons each class can be given, from the loaded models (host, map load).
    pub fn set_usable(&mut self, usable: std::collections::BTreeMap<String, Vec<String>>) {
        self.usable = usable;
        self.select_class(self.class);
    }
    pub fn usable(&self, class: &str) -> &[String] {
        self.usable.get(class).map_or(&[], Vec::as_slice)
    }
    /// The Props tab's models by folder (host, map load: every installed model with a .phy).
    pub fn set_props(&mut self, folders: std::collections::BTreeMap<String, Vec<String>>) {
        self.props = folders.into_iter().filter(|(_, m)| !m.is_empty()).collect();
        self.folder = self.folder.min(self.prop_folders().len().saturating_sub(1));
        self.model = 0;
    }
    /// The installed prop folders: the installation listing, else `SPAWN_PROPS` by folder.
    fn installed_folders(&self) -> std::borrow::Cow<'_, [(String, Vec<String>)]> {
        if !self.props.is_empty() {
            return std::borrow::Cow::Borrowed(&self.props);
        }
        let mut folders = std::collections::BTreeMap::<String, Vec<String>>::new();
        for model in SPAWN_PROPS {
            let folder = model.rsplit_once('/').map_or("", |(f, _)| f);
            folders
                .entry(folder.into())
                .or_default()
                .push((*model).into());
        }
        std::borrow::Cow::Owned(folders.into_iter().collect())
    }
    /// The Props tab's folders as listed: without a filter the installed common props
    /// first, then every folder; with one, all matches first, then the folders that
    /// have matches (only their matching models).
    fn prop_folders(&self) -> Vec<(String, Vec<String>)> {
        let installed = self.installed_folders();
        if self.filter.is_empty() {
            let all: std::collections::BTreeSet<&str> = installed
                .iter()
                .flat_map(|(_, m)| m.iter().map(String::as_str))
                .collect();
            let common = SPAWN_PROPS
                .iter()
                .filter(|m| all.contains(**m))
                .map(|m| (*m).to_owned())
                .collect();
            let mut folders = vec![(COMMON_FOLDER.to_owned(), common)];
            folders.extend(installed.iter().cloned());
            folders.retain(|(_, m)| !m.is_empty());
            return folders;
        }
        let matches: Vec<(String, Vec<String>)> = installed
            .iter()
            .map(|(folder, models)| {
                let models = models
                    .iter()
                    .filter(|m| {
                        m.to_ascii_lowercase().contains(&self.filter)
                            || prop_name(m).is_some_and(|n| n.contains(&self.filter))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                (folder.clone(), models)
            })
            .filter(|(_, m)| !m.is_empty())
            .collect();
        let all = matches
            .iter()
            .flat_map(|(_, m)| m.iter().cloned())
            .collect();
        let mut folders = vec![(format!("* matches \"{}\"", self.filter), all)];
        folders.extend(matches);
        folders
    }
    /// Rows per page and per drawn list.
    fn page(&self) -> usize {
        if self.rows_visible == 0 {
            VISIBLE_CLASSES
        } else {
            self.rows_visible
        }
    }
    fn label(&self) -> &'static str {
        SPAWN_CLASSES.get(self.class).copied().unwrap_or("")
    }
    /// The selected prop's model argument (Props tab).
    fn prop(&self) -> Option<String> {
        if self.tab != Tab::Props {
            return None;
        }
        let folders = self.prop_folders();
        folders.get(self.folder)?.1.get(self.model).cloned()
    }
    fn class_name(&self) -> &'static str {
        spawn_class(self.label()).0
    }
    fn rows(&self) -> Vec<OptionRow> {
        let mut rows = vec![OptionRow::Weapon(String::new())];
        rows.extend(
            self.usable(self.label())
                .iter()
                .cloned()
                .map(OptionRow::Weapon),
        );
        rows.push(OptionRow::Allegiance);
        if reads_num_grenades(self.class_name()) {
            rows.push(OptionRow::Grenades);
        }
        rows.extend([OptionRow::Notarget, OptionRow::Ai, OptionRow::FriendlyFire]);
        rows
    }
    fn select_class(&mut self, class: usize) {
        self.class = class;
        let label = self.label();
        let default = default_weapon(label);
        self.weapon = if self.usable(label).iter().any(|w| w == default) {
            default.into()
        } else {
            String::new()
        };
        let weapon = OptionRow::Weapon(self.weapon.clone());
        self.row = self.rows().iter().position(|r| *r == weapon).unwrap_or(0);
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.typing = false;
    }
    fn switch_tab(&mut self, tab: Tab) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        self.column = Column::Classes;
        self.typing = false;
        if self.tab == Tab::Npcs {
            self.select_class(self.class);
        }
    }
    /// Typed text for the Props filter (only while `typing`); the selection restarts at
    /// the matches.
    pub fn type_text(&mut self, text: &str) {
        if !self.open || !self.typing {
            return;
        }
        let before = self.filter.len();
        self.filter.extend(text.chars().filter_map(filter_char));
        if self.filter.len() != before {
            (self.folder, self.model) = (0, 0);
        }
    }
    /// Escape: stop typing, else close the overlay. False when it was closed already.
    pub fn escape(&mut self) -> bool {
        if !self.open {
            return false;
        }
        if self.typing {
            self.typing = false;
        } else {
            self.open = false;
        }
        true
    }
    /// Apply one key: Tab switches NPCs / Props; Up/Down (PageUp/PageDown: a page) move
    /// in the focused column (lists wrap; moving onto a weapon row chooses it), Left/Right
    /// switch columns, Enter toggles the option under the cursor (allegiance, notarget,
    /// AI) or else spawns. Props: Enter on a folder opens its models; Filter starts or
    /// stops typing into the filter, Backspace edits it.
    pub fn key(&mut self, key: OverlayKey) -> Option<OverlayCommand> {
        if key == OverlayKey::Toggle {
            self.toggle();
            return None;
        }
        if !self.open {
            return None;
        }
        match key {
            OverlayKey::Tab => {
                self.switch_tab(match self.tab {
                    Tab::Npcs => Tab::Props,
                    Tab::Props => Tab::Npcs,
                });
                return None;
            }
            OverlayKey::Filter => {
                self.switch_tab(Tab::Props);
                self.typing = !self.typing;
                return None;
            }
            OverlayKey::Backspace => {
                if self.typing && self.filter.pop().is_some() {
                    (self.folder, self.model) = (0, 0);
                }
                return None;
            }
            // Enter finishes typing; the arrows still move while typing.
            OverlayKey::Enter if self.typing => {
                self.typing = false;
                self.column = Column::Options;
                return None;
            }
            _ => {}
        }
        let page = self.page() as isize;
        let step = match key {
            OverlayKey::Up => -1,
            OverlayKey::PageUp => -page,
            OverlayKey::PageDown => page,
            _ => 1,
        };
        let paged = matches!(key, OverlayKey::PageUp | OverlayKey::PageDown);
        // A page clamps at the ends; single steps wrap.
        let moved = |at: usize, n: usize| -> usize {
            if n == 0 {
                0
            } else if paged {
                (at as isize + step).clamp(0, n as isize - 1) as usize
            } else {
                (at as isize + step).rem_euclid(n as isize) as usize
            }
        };
        if self.tab == Tab::Props {
            match key {
                OverlayKey::Left => self.column = Column::Classes,
                OverlayKey::Right => self.column = Column::Options,
                OverlayKey::Up | OverlayKey::Down | OverlayKey::PageUp | OverlayKey::PageDown => {
                    let folders = self.prop_folders();
                    let models = folders.get(self.folder).map_or(0, |f| f.1.len());
                    let count = folders.len();
                    if self.column == Column::Classes {
                        self.folder = moved(self.folder, count);
                        self.model = 0;
                    } else {
                        self.model = moved(self.model, models);
                    }
                }
                OverlayKey::Enter => {
                    if self.column == Column::Classes {
                        self.column = Column::Options;
                        return None;
                    }
                    return self.prop().map(OverlayCommand::SpawnProp);
                }
                _ => {}
            }
            return None;
        }
        match key {
            OverlayKey::Left => self.column = Column::Classes,
            OverlayKey::Right => self.column = Column::Options,
            OverlayKey::Up | OverlayKey::Down | OverlayKey::PageUp | OverlayKey::PageDown => {
                match self.column {
                    Column::Classes => {
                        self.select_class(moved(self.class, entry_count()));
                    }
                    Column::Options => {
                        let rows = self.rows();
                        self.row = moved(self.row, rows.len());
                        if let OptionRow::Weapon(weapon) = &rows[self.row] {
                            self.weapon = weapon.clone();
                        }
                    }
                }
            }
            OverlayKey::Enter => {
                if self.column == Column::Options {
                    match self.rows()[self.row] {
                        OptionRow::Allegiance => {
                            self.allegiance = self.allegiance.next();
                            return None;
                        }
                        OptionRow::Grenades => {
                            let next = GRENADE_CHOICES
                                .iter()
                                .position(|&n| n == self.grenades)
                                .map_or(0, |i| (i + 1) % GRENADE_CHOICES.len());
                            self.grenades = GRENADE_CHOICES[next];
                            return None;
                        }
                        OptionRow::Notarget => return Some(OverlayCommand::ToggleNotarget),
                        OptionRow::Ai => return Some(OverlayCommand::ToggleAi),
                        OptionRow::FriendlyFire => return Some(OverlayCommand::ToggleFriendlyFire),
                        OptionRow::Weapon(_) => {}
                    }
                }
                return Some(OverlayCommand::Spawn(self.request()));
            }
            _ => {}
        }
        None
    }
    /// A pointer click on a drawn row: tabs switch, the filter box starts typing, a "more"
    /// note pages, an item is selected and a click on the selected item acts like Enter
    /// (spawn, open a folder); option toggles act on every click.
    pub fn click(&mut self, target: PanelTarget) -> Option<OverlayCommand> {
        if !self.open {
            return None;
        }
        match target {
            PanelTarget::Tab(tab) => {
                self.switch_tab(tab);
                self.typing = false;
                None
            }
            PanelTarget::Filter => {
                self.switch_tab(Tab::Props);
                self.typing = true;
                None
            }
            PanelTarget::Page(column, direction) => {
                self.typing = false;
                self.column = column;
                self.scroll(Some(column), direction * self.page() as isize);
                None
            }
            PanelTarget::Row(column, index) => {
                self.typing = false;
                let focused = self.column == column;
                self.column = column;
                match (self.tab, column) {
                    (Tab::Props, Column::Classes) => {
                        if focused && self.folder == index {
                            self.column = Column::Options;
                        } else if self.folder != index {
                            (self.folder, self.model) = (index, 0);
                        }
                        None
                    }
                    (Tab::Props, Column::Options) => {
                        if focused && self.model == index {
                            return self.prop().map(OverlayCommand::SpawnProp);
                        }
                        self.model = index;
                        None
                    }
                    (Tab::Npcs, Column::Classes) => {
                        if focused && self.class == index {
                            return Some(OverlayCommand::Spawn(self.request()));
                        }
                        if index < entry_count() {
                            self.select_class(index);
                        }
                        None
                    }
                    (Tab::Npcs, Column::Options) => {
                        let rows = self.rows();
                        let row = rows.get(index)?;
                        let again = focused && self.row == index;
                        self.row = index;
                        match row {
                            OptionRow::Weapon(weapon) if !again => {
                                self.weapon = weapon.clone();
                                None
                            }
                            // Toggles act on every click; the chosen weapon spawns.
                            _ => self.key(OverlayKey::Enter),
                        }
                    }
                }
            }
        }
    }
    /// Mouse wheel over a column (None: the focused one): moves that column's selection
    /// by `rows`, clamped at the ends (a weapon row reached is chosen).
    pub fn scroll(&mut self, column: Option<Column>, rows: isize) {
        if !self.open || rows == 0 {
            return;
        }
        let column = column.unwrap_or(self.column);
        self.column = column;
        let moved =
            |at: usize, n: usize| (at as isize + rows).clamp(0, n.max(1) as isize - 1) as usize;
        match (self.tab, column) {
            (Tab::Props, Column::Classes) => {
                let folder = moved(self.folder, self.prop_folders().len());
                if folder != self.folder {
                    (self.folder, self.model) = (folder, 0);
                }
            }
            (Tab::Props, Column::Options) => {
                let models = self
                    .prop_folders()
                    .get(self.folder)
                    .map_or(0, |f| f.1.len());
                self.model = moved(self.model, models);
            }
            (Tab::Npcs, Column::Classes) => self.select_class(moved(self.class, entry_count())),
            (Tab::Npcs, Column::Options) => {
                let options = self.rows();
                self.row = moved(self.row, options.len());
                if let OptionRow::Weapon(weapon) = &options[self.row] {
                    self.weapon = weapon.clone();
                }
            }
        }
    }
    /// The overlay spawns at the crosshair like npc_create_aimed (facing the aim).
    pub fn request(&self) -> SpawnRequest {
        SpawnRequest {
            classname: self.class_name().into(),
            name: None,
            equipment: self.weapon.clone(),
            aimed: true,
            allegiance: self.allegiance,
            variant: self.label().into(),
            grenades: reads_num_grenades(self.class_name()).then_some(self.grenades),
            ..Default::default()
        }
    }
    /// The panel to draw; `ai` and `notarget` are the host's live switches.
    pub fn panel(&self, ai: bool, notarget: bool) -> Panel {
        let total = entry_count();
        let page = self.page();
        let classes_focused = self.column == Column::Classes;
        let tab = |text: &str, t: Tab| PanelRow {
            text: text.into(),
            selected: self.tab == t,
            cursor: false,
            target: Some(PanelTarget::Tab(t)),
        };
        let mut tabs = vec![tab("NPCs", Tab::Npcs), tab("Props", Tab::Props)];
        if self.tab == Tab::Props {
            let text = if self.typing {
                format!("filter: {}_", self.filter)
            } else if self.filter.is_empty() {
                "filter: (click or /)".into()
            } else {
                format!("filter: {}", self.filter)
            };
            tabs.push(PanelRow {
                text,
                selected: !self.filter.is_empty(),
                cursor: self.typing,
                target: Some(PanelTarget::Filter),
            });
        }
        if self.tab == Tab::Props {
            let folders = self.prop_folders();
            let names: Vec<String> = folders
                .iter()
                .map(|(folder, models)| {
                    let folder = if folder.is_empty() {
                        "(models)"
                    } else {
                        folder
                    };
                    format!("{folder}/ ({})", models.len())
                })
                .collect();
            // The common/matches list mixes folders: it shows each model's folder too.
            let mixed = self.folder == 0;
            let models: Vec<String> = folders.get(self.folder).map_or(Vec::new(), |(_, m)| {
                m.iter()
                    .map(|m| {
                        let shown = if mixed {
                            m.as_str()
                        } else {
                            m.rsplit('/').next().unwrap_or(m)
                        };
                        let shown = shown.trim_end_matches(".mdl");
                        match prop_name(m) {
                            Some(name) => format!("{shown}  ({name})"),
                            None => shown.into(),
                        }
                    })
                    .collect()
            });
            let count: usize = self.installed_folders().iter().map(|f| f.1.len()).sum();
            return Panel {
                title: format!(
                    "F4 Props ({count} models): folder {}/{}, model {}/{}  click: select, again: spawn",
                    self.folder + 1,
                    folders.len(),
                    self.model + 1,
                    models.len()
                ),
                tabs,
                headings: ["folder".into(), "model (Enter spawns)".into()],
                classes: scrolled_rows(&names, self.folder, classes_focused, page, Column::Classes),
                options: scrolled_rows(&models, self.model, !classes_focused, page, Column::Options),
            };
        }
        let names: Vec<String> = SPAWN_CLASSES.iter().map(|c| (*c).to_owned()).collect();
        let classes = scrolled_rows(&names, self.class, classes_focused, page, Column::Classes);
        let natural = !crate::npc_weapons::uses_weapons(self.class_name());
        let options = self
            .rows()
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let (text, selected) = match &row {
                    OptionRow::Weapon(w) if w.is_empty() => (
                        if natural {
                            "no weapons (natural attacks)".into()
                        } else {
                            "no weapon".into()
                        },
                        self.weapon.is_empty(),
                    ),
                    // Where the choice comes from: the class's HL2 map loadouts or
                    // Weapon_CanUse on its model.
                    OptionRow::Weapon(w) => (
                        if crate::npc_weapons::campaign_loadout(self.class_name())
                            .contains(&w.as_str())
                        {
                            format!("{w} (HL2 maps)")
                        } else {
                            format!("{w} (can use)")
                        },
                        *w == self.weapon,
                    ),
                    OptionRow::Allegiance => {
                        (format!("allegiance: {}", self.allegiance.label()), false)
                    }
                    OptionRow::Grenades => {
                        (format!("grenades: {} (NumGrenades)", self.grenades), false)
                    }
                    OptionRow::Notarget => (
                        format!("notarget: {}", if notarget { "ON" } else { "off" }),
                        false,
                    ),
                    OptionRow::Ai => (
                        format!("AI: {}", if ai { "ON" } else { "OFF (idle)" }),
                        false,
                    ),
                    OptionRow::FriendlyFire => (
                        format!(
                            "player hurts friendlies: {} (debug)",
                            if self.friendly_fire { "ON" } else { "off" }
                        ),
                        false,
                    ),
                };
                PanelRow {
                    text,
                    selected,
                    cursor: !classes_focused && index == self.row,
                    target: Some(PanelTarget::Row(Column::Options, index)),
                }
            })
            .collect();
        Panel {
            title: format!(
                "F4 spawn: NPCs ({}/{total}): click: select, again: spawn / toggle; Enter spawns",
                self.class + 1
            ),
            tabs,
            headings: ["NPC class".into(), "weapon / options".into()],
            classes,
            options,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn props_have_their_own_tab_with_folders() {
        let mut overlay = Overlay::default();
        overlay.key(OverlayKey::Toggle);
        // The NPC tab wraps among NPC classes only.
        overlay.key(OverlayKey::Up);
        assert_eq!(SPAWN_CLASSES[overlay.class], "npc_combinedropship");
        overlay.key(OverlayKey::Down);
        // Tab switches to the props: folders on the left, the folder's models on the right.
        let mut folders = std::collections::BTreeMap::new();
        folders.insert(
            "props_c17".to_string(),
            vec!["props_c17/oildrum001.mdl".to_string()],
        );
        folders.insert(
            "props_junk".to_string(),
            (0..30)
                .map(|i| format!("props_junk/item{i:02}.mdl"))
                .collect(),
        );
        overlay.set_props(folders);
        overlay.key(OverlayKey::Tab);
        assert_eq!(overlay.tab, Tab::Props);
        let panel = overlay.panel(true, false);
        assert!(panel.title.starts_with("F4 Props (31 models)"));
        assert_eq!(panel.headings[0], "folder");
        // The installed common props come first (with their folders), then each folder.
        assert_eq!(panel.classes[0].text, "* common/ (1)");
        assert_eq!(panel.classes[1].text, "props_c17/ (1)");
        assert!(panel.classes[0].cursor);
        assert_eq!(panel.options[0].text, "props_c17/oildrum001  (oil drum)");
        // Enter on a folder opens it; Enter on a model spawns it.
        overlay.key(OverlayKey::Down);
        overlay.key(OverlayKey::Down);
        assert_eq!(overlay.folder, 2);
        assert_eq!(overlay.key(OverlayKey::Enter), None);
        assert_eq!(overlay.column, Column::Options);
        overlay.key(OverlayKey::PageDown);
        assert_eq!(overlay.model, VISIBLE_CLASSES);
        overlay.key(OverlayKey::PageDown);
        overlay.key(OverlayKey::PageDown);
        // Pages clamp at the end; single steps wrap.
        assert_eq!(overlay.model, 29);
        overlay.key(OverlayKey::Down);
        assert_eq!(overlay.model, 0);
        overlay.key(OverlayKey::Up);
        let panel = overlay.panel(true, false);
        assert!(panel.options.iter().any(|r| r.cursor && r.text == "item29"));
        assert!(panel.options[0].text.contains("more above"));
        assert_eq!(
            overlay.key(OverlayKey::Enter),
            Some(OverlayCommand::SpawnProp("props_junk/item29.mdl".into()))
        );
        // Changing folders restarts at its first model; Tab returns to the NPCs.
        overlay.key(OverlayKey::Left);
        overlay.key(OverlayKey::Down);
        assert_eq!((overlay.folder, overlay.model), (0, 0));
        overlay.key(OverlayKey::Tab);
        assert_eq!(overlay.class, 0);
        assert!(matches!(
            overlay.key(OverlayKey::Enter),
            Some(OverlayCommand::Spawn(_))
        ));
        // Without an installation listing the Props tab falls back to SPAWN_PROPS.
        let mut fallback = Overlay::default();
        fallback.key(OverlayKey::Toggle);
        fallback.key(OverlayKey::Tab);
        fallback.key(OverlayKey::Enter);
        assert_eq!(
            fallback.key(OverlayKey::Enter),
            Some(OverlayCommand::SpawnProp("props_c17/oildrum001.mdl".into()))
        );
    }
    #[test]
    fn the_mouse_and_the_filter_drive_the_overlay() {
        let mut o = Overlay::default();
        o.set_usable(usable());
        assert_eq!(o.click(PanelTarget::Tab(Tab::Props)), None);
        assert_eq!(o.tab, Tab::Npcs, "closed: clicks do nothing");
        o.key(OverlayKey::Toggle);
        // NPC tab: a click selects a class, a second click on it spawns it.
        let cop = SPAWN_CLASSES
            .iter()
            .position(|c| *c == "npc_metropolice")
            .unwrap();
        assert_eq!(o.click(PanelTarget::Row(Column::Classes, cop + 1)), None);
        assert_eq!(o.click(PanelTarget::Row(Column::Classes, cop)), None);
        assert_eq!(o.class, cop);
        let Some(OverlayCommand::Spawn(r)) = o.click(PanelTarget::Row(Column::Classes, cop)) else {
            panic!()
        };
        assert_eq!(r.classname, "npc_metropolice");
        // Options: a weapon click chooses it; toggles act on every click.
        o.click(PanelTarget::Row(Column::Options, 2));
        assert_eq!(o.weapon, "weapon_pistol");
        let allegiance = o
            .rows()
            .iter()
            .position(|r| *r == OptionRow::Allegiance)
            .unwrap();
        o.click(PanelTarget::Row(Column::Options, allegiance));
        o.click(PanelTarget::Row(Column::Options, allegiance));
        assert_eq!(o.allegiance, Allegiance::Foe.next());
        // The wheel moves the hovered column's selection, clamped.
        o.scroll(Some(Column::Classes), -1000);
        assert_eq!((o.class, o.column), (0, Column::Classes));
        o.scroll(Some(Column::Classes), 3);
        assert_eq!(o.class, 3);
        // The drawn rows carry their click targets; notes page the list.
        o.rows_visible = 5;
        let panel = o.panel(true, false);
        assert_eq!(panel.classes.len(), 7);
        assert_eq!(
            panel.classes[0].target,
            Some(PanelTarget::Page(Column::Classes, -1))
        );
        assert_eq!(
            panel.classes[1].target,
            Some(PanelTarget::Row(Column::Classes, 1))
        );
        o.click(PanelTarget::Page(Column::Classes, 1));
        assert_eq!(o.class, 8);
        assert_eq!(panel.tabs.len(), 2);
        // Props: the filter box takes typed text; matches come first across folders.
        let mut folders = std::collections::BTreeMap::new();
        folders.insert(
            "props_c17".to_string(),
            vec![
                "props_c17/oildrum001.mdl".to_string(),
                "props_c17/oildrum001_explosive.mdl".to_string(),
            ],
        );
        folders.insert(
            "props_junk".to_string(),
            vec![
                "props_junk/wood_crate001a.mdl".to_string(),
                "props_junk/wood_crate002a.mdl".to_string(),
                "props_junk/popcan01a.mdl".to_string(),
            ],
        );
        o.set_props(folders);
        o.click(PanelTarget::Filter);
        assert!(o.typing && o.tab == Tab::Props);
        o.type_text("Crate!");
        assert_eq!(o.filter, "crate");
        let panel = o.panel(true, false);
        assert_eq!(panel.classes[0].text, "* matches \"crate\"/ (2)");
        assert_eq!(panel.classes[1].text, "props_junk/ (2)");
        assert_eq!(panel.tabs[2].text, "filter: crate_");
        o.key(OverlayKey::Backspace);
        o.type_text("e");
        assert_eq!(o.filter, "crate");
        // Enter ends typing in the models column; click a model, again to spawn.
        o.key(OverlayKey::Enter);
        assert!(!o.typing);
        assert_eq!(o.click(PanelTarget::Row(Column::Options, 1)), None);
        assert_eq!(
            o.click(PanelTarget::Row(Column::Options, 1)),
            Some(OverlayCommand::SpawnProp(
                "props_junk/wood_crate002a.mdl".into()
            ))
        );
        // Without a filter the common folder lists the installed common props.
        o.click(PanelTarget::Filter);
        for _ in 0..5 {
            o.key(OverlayKey::Backspace);
        }
        o.key(OverlayKey::Filter);
        let panel = o.panel(true, false);
        assert_eq!(panel.classes[0].text, "* common/ (5)");
        // Common props are found by their plain names too.
        o.key(OverlayKey::Filter);
        o.type_text("explosive barrel");
        let panel = o.panel(true, false);
        assert_eq!(
            panel.options[0].text,
            "props_c17/oildrum001_explosive  (explosive barrel)"
        );
        o.key(OverlayKey::Enter);
        // Escape stops typing first, then closes the overlay.
        o.key(OverlayKey::Filter);
        assert!(o.escape() && o.open && !o.typing);
        assert!(o.escape() && !o.open);
        assert!(!o.escape());
    }
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
    fn usable() -> std::collections::BTreeMap<String, Vec<String>> {
        [
            (
                "npc_metropolice",
                &["weapon_stunstick", "weapon_pistol", "weapon_smg1"][..],
            ),
            ("npc_combine_s", &["weapon_smg1", "weapon_ar2"][..]),
        ]
        .into_iter()
        .map(|(c, w)| (c.to_owned(), w.iter().map(|w| (*w).to_owned()).collect()))
        .collect()
    }
    #[test]
    fn overlay_columns_offer_only_usable_weapons_and_keep_the_selection_visible() {
        let mut o = Overlay::default();
        o.set_usable(usable());
        assert_eq!(o.key(OverlayKey::Up), None, "closed: keys do nothing");
        o.key(OverlayKey::Toggle);
        // The cop starts with its default weapon; the column lists none + usable + rows.
        assert_eq!(o.request().equipment, "weapon_stunstick");
        let panel = o.panel(false, false);
        let texts: Vec<&str> = panel.options.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "no weapon",
                "weapon_stunstick (HL2 maps)",
                "weapon_pistol (HL2 maps)",
                "weapon_smg1 (HL2 maps)",
                "allegiance: class default",
                "notarget: off",
                "AI: OFF (idle)",
                "player hurts friendlies: off (debug)"
            ]
        );
        assert!(panel
            .classes
            .iter()
            .any(|r| r.cursor && r.text == "npc_metropolice"));
        // Options column: Down moves onto the pistol and chooses it.
        o.key(OverlayKey::Right);
        o.key(OverlayKey::Down);
        assert_eq!(o.request().equipment, "weapon_pistol");
        // Enter on allegiance cycles it; on notarget/AI it asks the host.
        o.key(OverlayKey::Down);
        o.key(OverlayKey::Down);
        assert_eq!(o.key(OverlayKey::Enter), None);
        assert_eq!(o.allegiance, Allegiance::Foe);
        o.key(OverlayKey::Down);
        assert_eq!(
            o.key(OverlayKey::Enter),
            Some(OverlayCommand::ToggleNotarget)
        );
        o.key(OverlayKey::Down);
        assert_eq!(o.key(OverlayKey::Enter), Some(OverlayCommand::ToggleAi));
        o.key(OverlayKey::Down);
        assert_eq!(
            o.key(OverlayKey::Enter),
            Some(OverlayCommand::ToggleFriendlyFire)
        );
        // On a weapon row (wrapping down to "no weapon"), Enter spawns with the choices.
        o.key(OverlayKey::Down);
        let Some(OverlayCommand::Spawn(r)) = o.key(OverlayKey::Enter) else {
            panic!()
        };
        assert_eq!(
            (
                r.classname.as_str(),
                r.equipment.as_str(),
                r.allegiance,
                r.aimed
            ),
            ("npc_metropolice", "", Allegiance::Foe, true)
        );
        // A zombie: no weapon column, natural attacks only.
        o.key(OverlayKey::Left);
        let zombie = SPAWN_CLASSES
            .iter()
            .position(|c| *c == "npc_zombie")
            .unwrap();
        for _ in 0..zombie {
            o.key(OverlayKey::Down);
        }
        let panel = o.panel(true, true);
        assert_eq!(panel.options[0].text, "no weapons (natural attacks)");
        assert_eq!(panel.options.len(), 5);
        assert_eq!(o.request().equipment, "");
        // The class list scrolls: the selection is always one of the drawn rows.
        o.key(OverlayKey::Up);
        for _ in 0..entry_count() {
            o.key(OverlayKey::Down);
            let panel = o.panel(true, false);
            assert!(panel.classes.len() <= VISIBLE_CLASSES + 2);
            let text = SPAWN_CLASSES[o.class].to_owned();
            assert!(
                panel.classes.iter().any(|r| r.selected && r.text == text),
                "{panel:?}"
            );
        }
        // Wrapping up from the first class reaches the last NPC class, with "more above".
        while o.class != 0 {
            o.key(OverlayKey::Down);
        }
        o.key(OverlayKey::Up);
        let panel = o.panel(false, false);
        assert!(panel.classes[0].text.contains("more above"));
        assert_eq!(o.class, entry_count() - 1);
        assert_eq!(o.request().classname, "npc_combinedropship");
        // Combine soldiers have no stunstick: their default SMG1 is preselected.
        while SPAWN_CLASSES[o.class] != "npc_combine_s" {
            o.key(OverlayKey::Down);
        }
        assert_eq!(o.request().equipment, "weapon_smg1");
        assert_eq!(o.request().grenades, Some(0));
        // Their options add NumGrenades after the allegiance; Enter cycles the count.
        let texts: Vec<String> = o
            .panel(true, false)
            .options
            .into_iter()
            .map(|r| r.text)
            .collect();
        assert_eq!(texts[3], "allegiance: foe (hates the player)");
        assert_eq!(texts[4], "grenades: 0 (NumGrenades)");
        o.key(OverlayKey::Right);
        for _ in 0..3 {
            o.key(OverlayKey::Down);
        }
        for expected in [1, 2, 3, 5, 10, 99, 0, 1] {
            assert_eq!(o.key(OverlayKey::Enter), None);
            assert_eq!(o.grenades, expected);
        }
        assert_eq!(o.request().grenades, Some(1));
        // Classes that never read NumGrenades neither show nor send it.
        o.key(OverlayKey::Left);
        o.key(OverlayKey::Up);
        assert!(o
            .panel(true, false)
            .options
            .iter()
            .all(|r| !r.text.starts_with("grenades")));
        assert_eq!(o.request().grenades, None);
    }
}
