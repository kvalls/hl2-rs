//! Agent link on the host: answers `ui_dump`, `npc_dump`, and `agent_resume` from the console
//! and the simulation.
use std::collections::BTreeMap;

use crate::agent::{World, npc_dump_line, ui_dump_line};
use crate::commands::NpcDumpQuery;

const MAX_NEARBY_NPCS: usize = 12;

/// Prints the `ui_dump` line.
pub fn ui_dump(sim: &hl2_bevy::simulation::Simulation, ui: &mut hl2_bevy::ui_host::Console) {
    let eye = sim.eye();
    let world = World {
        map: ui.source.loaded_map.as_ref().map(|map| map.map.as_str()),
        eye: [eye.x, eye.y, eye.z],
        angles: [-sim.pitch.to_degrees(), sim.yaw.to_degrees()],
        paused: sim.paused,
        loading: sim.loading,
        background_level: sim.background_level,
    };
    let line = ui_dump_line(ui.report(), &world);
    ui.source.log(line);
}

/// Prints the `agent_resume` line: the arguments that restart where the player is.
pub fn agent_resume(sim: &hl2_bevy::simulation::Simulation, ui: &mut hl2_bevy::ui_host::Console) {
    let eye = sim.eye();
    let resume = crate::agent::Resume {
        map: ui.source.loaded_map.as_ref().map(|map| map.map.as_str()),
        background_level: sim.background_level,
        eye: [eye.x, eye.y, eye.z],
        yaw: sim.yaw.to_degrees(),
        pitch: sim.pitch.to_degrees(),
        mode: ui.source.mode(),
        windows: ui.source.open_windows(),
    };
    let line = format!(
        "{}{}",
        crate::agent::RESUME_PREFIX,
        crate::agent::resume_args(&resume).join(" ")
    );
    ui.source.log(line);
}

/// Prints the `npc_dump` line: live AI, actbusy, and goal state for nearby or selected NPCs.
pub fn npc_dump(
    sim: &hl2_bevy::simulation::Simulation,
    game: Option<&hl2_bevy::gameplay::Gameplay>,
    ui: &mut hl2_bevy::ui_host::Console,
    query: NpcDumpQuery,
) {
    let Some(game) = game else {
        ui.source.log(npc_dump_line(
            serde_json::json!({"error": "no gameplay world"}),
        ));
        return;
    };
    let eye = sim.eye();
    let eye = glam::Vec3::new(eye.x, eye.y, eye.z);
    let ai_value = serde_json::to_value(&game.ai).unwrap_or(serde_json::Value::Null);
    let actors = ai_value
        .get("actors")
        .and_then(serde_json::Value::as_object);
    let goals: BTreeMap<usize, _> = game
        .npcs
        .snapshots()
        .into_iter()
        .map(|g| (g.key.actor, g))
        .collect();
    let ids = select_npcs(game, eye, query);
    let npcs: Vec<_> = ids
        .into_iter()
        .map(|(id, dist)| npc_entry(game, id, dist, actors, goals.get(&id)))
        .collect();
    let payload = serde_json::json!({
        "time": game.scene.time,
        "eye": eye.to_array(),
        "count": npcs.len(),
        "npcs": npcs,
    });
    ui.source.log(npc_dump_line(payload));
}

fn select_npcs(
    game: &hl2_bevy::gameplay::Gameplay,
    eye: glam::Vec3,
    query: NpcDumpQuery,
) -> Vec<(usize, f32)> {
    match query {
        NpcDumpQuery::Entity(id) => game
            .scene
            .states
            .get(id)
            .map(|state| vec![(id, state.origin.distance(eye))])
            .unwrap_or_default(),
        NpcDumpQuery::Nearby(radius) => {
            let mut found: Vec<(usize, f32)> = game
                .world
                .entities
                .iter()
                .enumerate()
                .filter_map(|(id, entity)| {
                    if !entity.class().starts_with("npc_") {
                        return None;
                    }
                    let state = game.scene.states.get(id)?;
                    if state.killed || !state.visible || state.corpse {
                        return None;
                    }
                    let dist = state.origin.distance(eye);
                    (dist <= radius).then_some((id, dist))
                })
                .collect();
            found.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
            found.truncate(MAX_NEARBY_NPCS);
            found
        }
    }
}

fn npc_entry(
    game: &hl2_bevy::gameplay::Gameplay,
    id: usize,
    distance: f32,
    actors: Option<&serde_json::Map<String, serde_json::Value>>,
    goal: Option<&hl2_simulation::npc::GoalSnapshot>,
) -> serde_json::Value {
    let entity = &game.world.entities[id];
    let state = &game.scene.states[id];
    let actor = actors.and_then(|map| map.get(&id.to_string()));
    let npc = actor.and_then(|a| a.get("npc"));
    let actbusy = actor
        .and_then(|a| a.get("class"))
        .and_then(serde_json::Value::as_object)
        .and_then(|class_map| class_map.values().next())
        .and_then(|inner| inner.get("actbusy"));
    let goal_json = goal.map(|g| {
        serde_json::json!({
            "state": g.state,
            "target_feet": g.target_feet.to_array(),
            "feet": g.feet.to_array(),
            "yaw_degrees": g.yaw_degrees,
            "tolerance": g.tolerance,
            "next_waypoint": g.next_waypoint,
            "route_len": g.route_feet.len(),
            "next_point": g.route_feet.get(g.next_waypoint).map(|v| v.to_array()),
            "sequence": g.sequence,
            "contact_normal": g.contact_normal.to_array(),
            "traversal": g.traversal,
        })
    });
    serde_json::json!({
        "id": id,
        "class": entity.class(),
        "name": entity.get("targetname").unwrap_or(""),
        "model": entity.get("model").unwrap_or(""),
        "distance": (distance * 10.0).round() / 10.0,
        "origin": state.origin.to_array(),
        "yaw": actor.and_then(|a| a.get("yaw")),
        "animation": state.animation,
        "health": state.value,
        "visible": state.visible,
        "killed": state.killed,
        "corpse": state.corpse,
        "schedule": npc.and_then(|n| n.get("schedule")),
        "task": npc.and_then(|n| n.get("task")),
        "activity": npc.and_then(|n| n.get("activity")),
        "ai_state": npc.and_then(|n| n.get("state")),
        "schedule_changes": npc.and_then(|n| n.get("schedule_changes")),
        "last_failed_task": npc.and_then(|n| n.get("last_failed_task")),
        "conditions": npc.and_then(|n| n.get("conditions")),
        "enemy": npc.and_then(|n| n.get("enemy")),
        "actbusy": actbusy,
        "goal": goal_json,
    })
}
