//! Profiling builds only (`trace` feature): the player, viewmodel and NPC state as zero-length
//! spans in the trace, so a slow frame shows what was happening in it and NPC cost can be
//! grouped by class, schedule and task. The player and viewmodel are written every frame; an
//! NPC when its AI state, schedule, task, activity or animation changes, and every NPC once
//! every `NPC_SNAPSHOT_FRAMES` frames (readers carry the last state forward).
use bevy::prelude::{App, IntoScheduleConfigs, Local, PostUpdate, Res};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

/// Every NPC is written at least this often, in frames.
const NPC_SNAPSHOT_FRAMES: u64 = 60;

pub fn install(app: &mut App) {
    app.add_systems(
        PostUpdate,
        record.run_if(
            bevy::ecs::schedule::common_conditions::resource_exists::<crate::gameplay::Gameplay>,
        ),
    );
}

/// What was last written per NPC, as a hash of the fields that trigger a new line.
#[derive(Default)]
struct Written {
    frame: u64,
    npcs: HashMap<usize, u64>,
}

fn record(
    sim: Res<crate::simulation::Simulation>,
    game: Res<crate::gameplay::Gameplay>,
    mut written: Local<Written>,
) {
    record_player(&sim, &game);
    written.frame += 1;
    let snapshot = written.frame % NPC_SNAPSHOT_FRAMES == 1;
    let feet = sim.player.feet;
    for (&id, actor) in &game.ai.actors {
        let Some(state) = game.scene.states.get(id).filter(|s| !s.killed) else {
            continue;
        };
        let npc = &actor.npc;
        let mut hasher = DefaultHasher::new();
        (std::mem::discriminant(&npc.state), &npc.schedule, npc.task).hash(&mut hasher);
        (&npc.activity, &state.animation).hash(&mut hasher);
        let key = hasher.finish();
        if !snapshot && written.npcs.get(&id) == Some(&key) {
            continue;
        }
        written.npcs.insert(id, key);
        let _npc = hl2_simulation::profile_span!(
            "state_npc",
            id,
            class = game.world.entities.get(id).map_or("", |e| e.class()),
            distance = state.origin.distance(feet).round(),
            ai = ?npc.state,
            schedule = npc.schedule.as_deref().unwrap_or(""),
            task = npc.task,
            activity = %npc.activity,
            animation = %state.animation,
            visible = state.visible,
        );
    }
}

fn record_player(sim: &crate::simulation::Simulation, game: &crate::gameplay::Gameplay) {
    let (player, input, eye, inventory) = (&sim.player, &sim.input, sim.eye(), &game.inventory);
    let _player = hl2_simulation::profile_span!(
        "state_player",
        x = eye.x.round(),
        y = eye.y.round(),
        z = eye.z.round(),
        speed = player.velocity.truncate().length().round(),
        grounded = player.grounded,
        crouched = player.crouched,
        fly = sim.fly,
        forward = input.forward,
        side = input.side,
        jump = input.jump,
        sprint = input.sprint,
        attack = game.primary,
        attack2 = game.secondary,
        health = inventory.health,
    );
    let _viewmodel = hl2_simulation::profile_span!(
        "state_viewmodel",
        weapon = %inventory.active,
        animation = %inventory.animation,
        age = (game.scene.time - inventory.animation_at) as f32,
    );
}
