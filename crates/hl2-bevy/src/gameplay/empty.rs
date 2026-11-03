//! Resource-free gameplay state for the startup loading screen and small fake apps.
use super::Gameplay;
use hl2_simulation::{
    entities::Scene, gameplay::Inventory, npc::Controller, projectiles::Projectiles,
    selection::Selection,
};
use modkit_core::World;
use std::{collections::BTreeMap, sync::Arc};
impl Gameplay {
    pub(crate) fn synthetic(world: World) -> Self {
        let scene = Scene::new(&world);
        Self::from_prepared(
            world,
            PreparedGameplay {
                scene,
                weapons: BTreeMap::new(),
                npcs: Controller::new(None, Default::default()),
                impacts: Default::default(),
                player_damage: Default::default(),
                skill_config: None,
                ai: hl2_simulation::ai::runtime::AiRuntime::new(&World::default(), None),
            },
        )
    }
    pub(super) fn from_prepared(world: World, prepared: PreparedGameplay) -> Self {
        Self {
            scene: prepared.scene,
            world: Arc::new(world),
            inventory: Inventory::default(),
            weapons: prepared.weapons,
            selection: Selection::new(),
            npcs: prepared.npcs,
            projectiles: Projectiles::default(),
            gibs: Default::default(),
            ragdolls: Default::default(),
            phys_explosions: Default::default(),
            impacts: prepared.impacts,
            primary: false,
            secondary: false,
            actions: vec![],
            queued_primary: false,
            queued_secondary: false,
            primary_consumed: false,
            secondary_consumed: false,
            unplayed_sounds: 0,
            sound_cues: vec![],
            sound_requests: vec![],
            captions: vec![],
            caption_messages: vec![],
            weapon_sounds: Default::default(),
            entity_sounds: Default::default(),
            footsteps: Default::default(),
            player_damage: prepared.player_damage,
            damage_messages: Vec::new(),
            suit: Default::default(),
            skill: 2,
            skill_config: prepared.skill_config,
            ai: prepared.ai,
            player_impulse: glam::Vec3::ZERO,
            spawn_console: Default::default(),
            spawn_log: Vec::new(),
            shot_effects: Vec::new(),
            shot_serial: 0,
            stunstick_glows: Vec::new(),
            spawn_counts: Default::default(),
            render_spawns: Vec::new(),
            zombie_splits: Vec::new(),
            crab_releases: Vec::new(),
            released_crabs: Vec::new(),
            thrown_crabs: Vec::new(),
            evacuated_crabs: Vec::new(),
            npc_ignitions: Vec::new(),
            zombie_split_failures: Vec::new(),
            runtime_entities: Vec::new(),
            despawned_draws: Default::default(),
            world_copies: 0,
            drawn_bodies: Default::default(),
            body_swaps: 0,
            body_swap_failures: Vec::new(),
            launch_impacts: Vec::new(),
            prop_requests: Vec::new(),
            model_draws: Vec::new(),
        }
    }
}
pub(super) struct PreparedGameplay {
    pub scene: Scene,
    pub weapons: BTreeMap<String, hl2_simulation::gameplay::Weapon>,
    pub npcs: Controller,
    pub impacts: hl2_simulation::impacts::Impacts,
    pub player_damage: hl2_simulation::player_damage::PlayerDamage,
    pub skill_config: Option<String>,
    pub ai: hl2_simulation::ai::runtime::AiRuntime,
}
