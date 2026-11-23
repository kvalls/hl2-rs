//! On-demand entity inventory and spatial lookup for the remote console.
use crate::commands::MapDumpQuery;
use glam::Vec3;
use hl2_bevy::{gameplay::Gameplay, simulation::Simulation, ui_host::Console};
use serde_json::json;

// Small pages keep a normal map query readable in the local console as well as over RCON.
const PAGE: usize = 12;

pub(super) fn dump(
    sim: &Simulation,
    game: Option<&Gameplay>,
    ui: &mut Console,
    query: MapDumpQuery,
) {
    let Some(game) = game else {
        ui.source.log(format!(
            "map_dump {}",
            json!({"error": "no gameplay world"})
        ));
        return;
    };
    let total = game.world.entities.len();
    let origin = |id: usize| {
        let entity = &game.world.entities[id];
        sim.physics.entity_pose(id).map(|(p, _)| p).or_else(|| {
            let state = game.scene.states.get(id)?;
            // An absent BSP origin often defaults to zero for simulation. Do not report
            // a spurious point at the world origin when querying unpositioned entities.
            (state.origin != Vec3::ZERO
                || entity
                    .get("origin")
                    .and_then(modkit_core::parse_vec3)
                    .is_some())
            .then_some(state.origin)
        })
    };
    let mut result = json!({
        "game": ui.source.loaded_map.as_ref().map(|m| &m.game),
        "map": game.world.name, "total": total, "time": game.scene.time,
    });
    let ids: Vec<(usize, Option<f32>)> = match query {
        MapDumpQuery::Page(first) => {
            let end = first.saturating_add(PAGE).min(total);
            result["next_id"] = json!((end < total).then_some(end));
            (first..end).map(|id| (id, None)).collect()
        }
        MapDumpQuery::Near { point, radius } => {
            let center = Vec3::from_array(point);
            let mut hits: Vec<_> = (0..total)
                .filter_map(|id| {
                    let state = game.scene.states.get(id)?;
                    let dist = origin(id)?.distance(center);
                    (!state.killed && dist <= radius).then_some((id, dist))
                })
                .collect();
            hits.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            result["point"] = json!(point);
            result["radius"] = json!(radius);
            result["matches"] = json!(hits.len());
            hits.into_iter()
                .take(PAGE)
                .map(|(id, d)| (id, Some(d)))
                .collect()
        }
    };
    result["entities"] = json!(
        ids.into_iter()
            .map(|(id, distance)| {
                let entity = &game.world.entities[id];
                json!({
                    "id": id, "class": entity.class(), "name": entity.get("targetname"),
                    "initial_origin": entity.get("origin").and_then(modkit_core::parse_vec3),
                    "origin": origin(id), "distance": distance,
                    "killed": game.scene.states.get(id).map(|s| s.killed),
                })
            })
            .collect::<Vec<_>>()
    );
    ui.source.log(format!("map_dump {result}"));
}
