//! Entity models' skeletons and sequences, loaded on all cores before instances are placed.
use super::{
    ai_activities, prefetch, HUMANOID_AI_ACTIVITIES, METROPOLICE_AI_ACTIVITIES,
    NPC_BASE_AI_ACTIVITIES,
};
use crate::vpk::Vfs;
use modkit_core::{animation::Rig, ModelInstance, World};
use std::collections::{BTreeMap, BTreeSet};

/// Sequences every entity rig loads.
const COMMON_SEQUENCES: &[&str] = &[
    "ACT_IDLE",
    // Weapon-translated metrocop idles (weapon_pistol/smg1 acttables).
    "ACT_IDLE_PISTOL",
    "ACT_IDLE_SMG1",
    // Stunstick acttable results for the metrocop shove/baton chain.
    "ACT_MELEE_ATTACK_SWING",
    "ACT_IDLE_ANGRY_MELEE",
    "idle_subtle",
    "idle_baton",
    "walk_all",
    "run_all",
    "idle",
    "idle01",
    "fire",
    "reload",
    "draw",
    "swing",
    "attack",
    "death1",
    // npc_combine_camera deploy/retire activities (matched lowercase).
    "act_combine_camera_open_idle",
    "act_combine_camera_closed_idle",
    "act_combine_camera_close",
];

/// Entity keys naming sequences the map plays on any entity.
const SEQUENCE_KEYS: &[&str] = &[
    "DefaultAnim",
    "m_iszPlay",
    "m_iszIdle",
    "m_iszPostIdle",
    "animation",
];

/// One rig to load: the instance it belongs to, its model and the sequences it keeps.
struct Job {
    instance: usize,
    model: String,
    raggib: bool,
    wanted: BTreeSet<String>,
}

/// The rig of each placed entity model, keyed by the index of the first placed instance that
/// asks for it (in `world.model_instances` order, skipping rigs `world` already holds), so the
/// caller stores each rig exactly where a one-by-one load would.
pub(super) fn load(
    world: &World,
    vfs: &Vfs,
    placed: impl Fn(&ModelInstance) -> bool,
) -> BTreeMap<usize, (String, anyhow::Result<Rig>)> {
    let map_sequences: BTreeSet<String> = (world.entities.iter())
        .flat_map(|e| SEQUENCE_KEYS.iter().filter_map(|key| e.get(key)))
        .map(|name| name.to_lowercase())
        .collect();
    let mut claimed = BTreeSet::new();
    let jobs: Vec<Job> = (world.model_instances.iter().enumerate())
        .filter(|(_, instance)| instance.entity.is_some() && placed(instance))
        .filter(|(_, instance)| {
            let key = instance.asset_key();
            !world.rigs.contains_key(&key) && claimed.insert(key)
        })
        .map(|(index, instance)| Job {
            instance: index,
            model: instance.model.clone(),
            raggib: instance.kind == "raggib",
            wanted: wanted(world, instance, &map_sequences),
        })
        .collect();
    let rigs = prefetch::parallel_map(&jobs, |job| load_one(vfs, job));
    jobs.into_iter()
        .zip(rigs)
        .map(|(job, rig)| (job.instance, (job.model, rig)))
        .collect()
}

fn load_one(vfs: &Vfs, job: &Job) -> anyhow::Result<Rig> {
    let mut wanted = job.wanted.clone();
    // A raggib ragdolls from its sequence 0 (cycle 0).
    if job.raggib {
        if let Ok(Some(first)) = crate::animation::first_sequence_name(vfs, &job.model) {
            wanted.insert(first.to_lowercase());
        }
    }
    crate::animation::load(vfs, &job.model, &wanted)
}

/// The sequences an instance's rig keeps: the common ones, its class's AI activities and
/// the ones the map names.
fn wanted(
    world: &World,
    instance: &ModelInstance,
    map_sequences: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut wanted: BTreeSet<String> = COMMON_SEQUENCES.iter().map(|s| s.to_string()).collect();
    // npc_metropolice AI activities (hl2-simulation ai::metropolice/police and the weapon
    // acttables): the cloud AI branch requested them at runtime only, so their sequences
    // were never loaded and every task using them failed.
    if instance.kind == "npc_metropolice" {
        wanted.extend(METROPOLICE_AI_ACTIVITIES.iter().map(|a| a.to_lowercase()));
    }
    if matches!(
        instance.kind.as_str(),
        "npc_citizen" | "npc_barney" | "npc_alyx" | "npc_combine_s"
    ) {
        wanted.extend(HUMANOID_AI_ACTIVITIES.iter().map(|a| a.to_lowercase()));
    }
    if instance.kind.starts_with("npc_") {
        wanted.extend(NPC_BASE_AI_ACTIVITIES.iter().map(|a| a.to_lowercase()));
    }
    wanted.extend(
        ai_activities::instance_ai_sequences(world, instance)
            .iter()
            .map(|a| a.to_lowercase()),
    );
    wanted.extend(map_sequences.iter().cloned());
    wanted
}
