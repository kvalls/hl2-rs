//! Developer commentary integration for the fixed simulation loop and balloon skin updates.

use super::Simulation;
use crate::{
    gameplay::{Action, Gameplay},
    rendering::{ModelCache, SourceEntity, SourceMaterial},
    ui_host::Console,
};
use bevy::{
    asset::Assets,
    ecs::system::{Query, Res, ResMut},
    prelude::MeshMaterial3d,
};
use modkit_core::movement::{Player, TICK};

pub(super) fn before_gameplay_tick(
    sim: &mut Simulation,
    game: &mut Gameplay,
    ui: Option<&mut Console>,
) {
    if !game.scene.commentary.enabled {
        return;
    }
    let use_pressed = game.actions.iter().any(|a| matches!(a, Action::Use));
    let secondary_pressed = game.queued_secondary && !game.secondary_consumed;
    let eye = glam::Vec3::from_array(sim.eye.to_array());
    let view_angles = [-sim.pitch.to_degrees(), sim.yaw.to_degrees(), 0.0];
    let prev_active = game.scene.commentary.active_node;
    let world = game.world.clone();
    hl2_simulation::commentary::CommentaryState::tick(
        &mut game.scene,
        &world,
        TICK,
        eye,
        view_angles,
        use_pressed,
        secondary_pressed,
    );
    if use_pressed
        && (game.scene.commentary.hovered_node.is_some()
            || game.scene.commentary.active_node != prev_active)
    {
        game.actions.retain(|a| !matches!(a, Action::Use));
    }
    if let Some((dest_feet, look_angles)) = game.scene.commentary.pending_teleport.take() {
        game.queued_secondary = false;
        game.secondary_consumed = true;
        let dest_eye = dest_feet + glam::Vec3::new(0.0, 0.0, 64.0);
        sim.player = Player::new(glam::Vec3::from_array(dest_eye.to_array()));
        sim.player.feet = glam::Vec3::from_array(dest_feet.to_array());
        sim.eye = sim.player.eye();
        sim.pitch = -look_angles[0].to_radians().clamp(-1.53, 1.53);
        sim.yaw = look_angles[1].to_radians();
    }
    if !game.scene.commentary.pending_commands.is_empty() {
        let commands = std::mem::take(&mut game.scene.commentary.pending_commands);
        if let Some(ui) = ui {
            for cmd in commands {
                let effects = ui.source.submit(&cmd);
                sim.console_effects(ui, game, effects);
            }
        }
    }
}

pub(super) fn finish_ground_step(
    sim: &mut Simulation,
    game: &mut Gameplay,
    before: hl2_simulation::footsteps::State,
    feet: glam::Vec3,
) {
    game.step_sounds(&sim.physics, before, feet, &sim.player);
    if game.scene.commentary.enabled {
        game.scene.player_damage.clear();
        sim.player.landed = None;
    }
    if game.player_outcome(&mut sim.player) {
        sim.death_buttons_released = false;
        sim.eye = sim.player.eye();
    }
}

pub(super) fn after_gameplay_step(sim: &mut Simulation, game: &mut Gameplay) {
    let world = game.world.clone();
    game.scene
        .update_soundscape(&world, glam::Vec3::from_array(sim.eye.to_array()));
    sim.refresh_scripted_view(game);
}

pub(crate) fn update_commentary_skins(
    game: Option<Res<Gameplay>>,
    cache: Option<Res<ModelCache>>,
    draws: Query<(&SourceEntity, &MeshMaterial3d<SourceMaterial>)>,
    mut materials: ResMut<Assets<SourceMaterial>>,
) {
    let Some(game) = game else {
        return;
    };
    if !game.scene.commentary.enabled {
        return;
    }
    let Some(cache) = cache else {
        return;
    };
    let tex_unused = cache
        .definitions
        .get(hl2_simulation::commentary::SPEECH_MAT_UNUSED)
        .and_then(|d| d.base_path.as_ref())
        .and_then(|p| cache.textures.get(p));
    let tex_used = cache
        .definitions
        .get(hl2_simulation::commentary::SPEECH_MAT_USED)
        .and_then(|d| d.base_path.as_ref())
        .and_then(|p| cache.textures.get(p));
    let (Some(tex_unused), Some(tex_used)) = (tex_unused, tex_used) else {
        return;
    };
    for (entity, mat_handle) in &draws {
        if game
            .world
            .entities
            .get(entity.0)
            .is_none_or(|e| e.class() != "point_commentary_node")
        {
            continue;
        }
        let wanted = if game.scene.commentary.node_skin(entity.0) == 1 {
            tex_used
        } else {
            tex_unused
        };
        if let Some(mat) = materials.get(mat_handle)
            && &mat.base != wanted
            && let Some(mut mat) = materials.get_mut(mat_handle)
        {
            mat.base = wanted.clone();
        }
    }
}

/// A labelled sample's gameplay report, with the commentary state added while it is on.
pub(super) fn sample_report(
    game: &Gameplay,
    physics: &hl2_simulation::physics::Physics,
) -> serde_json::Value {
    let mut report = game.report(physics);
    if game.scene.commentary.enabled
        && let Some(map) = report.as_object_mut()
    {
        map.insert("commentary".into(), state_report(&game.scene.commentary));
    }
    report
}

/// Replay report: the nodes (with origins) and the active, hovered and pending state.
fn state_report(state: &hl2_simulation::commentary::CommentaryState) -> serde_json::Value {
    serde_json::json!({
        "enabled": state.enabled,
        "nodes": state.nodes.iter().map(|n| serde_json::json!({
            "entity": n.entity_id, "number": n.node_number, "origin": n.origin.to_array(),
            "speakers": n.speakers, "listened": n.listened, "active": n.active,
            "duration": n.duration, "cue": n.commentaryfile,
        })).collect::<Vec<_>>(),
        "active": state.active_node,
        "hovered": state.hovered_node,
        "view_override": state.view_override.is_some(),
        "pending_commands": state.pending_commands,
    })
}
