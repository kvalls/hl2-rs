//! Scenes and templates: `logic_choreographed_scene`, `scripted_sequence` and `point_template`.

use super::{Delivery, Scene};
use crate::entities::ScenePause;

/// Start, Pause, Resume, Cancel.
pub(super) fn choreographed_scene(scene: &mut Scene, d: &Delivery) -> bool {
    let id = d.id;
    match d.input {
        "start" => scene.start_choreography(d.world, id, d.pending.activator),
        "pause" => {
            if let Some(play) = scene
                .choreography
                .get_mut(&id)
                .and_then(|s| s.playback.as_mut())
            {
                play.pause = Some(ScenePause::Input);
            }
        }
        "resume" => {
            if let Some(play) = scene
                .choreography
                .get_mut(&id)
                .and_then(|s| s.playback.as_mut())
            {
                if !matches!(play.pause, Some(ScenePause::UnsupportedControl)) {
                    play.pause = None;
                }
            }
        }
        "cancel" => scene.cancel_choreography(id),
        _ => return false,
    }
    true
}

/// BeginSequence, CancelSequence.
pub(super) fn scripted_sequence(scene: &mut Scene, d: &Delivery) -> bool {
    match d.input {
        "beginsequence" => scene.begin_sequence(d.world, d.id),
        "cancelsequence" => scene.end_sequence(d.world, d.id, true),
        _ => return false,
    }
    true
}

/// ForceSpawn.
pub(super) fn template(scene: &mut Scene, d: &Delivery) -> bool {
    if d.input != "forcespawn" {
        return false;
    }
    // Templates spawn copies; each authored child is created once here. Repeat
    // spawns would need new entity instances, which are not implemented.
    if let Some(children) = scene.templates.remove(&d.id) {
        for (child, visible, enabled) in children {
            let state = &mut scene.states[child];
            state.killed = false;
            state.visible = visible;
            state.enabled = enabled;
        }
        scene.fire(d.id, "OnEntitySpawned", d.pending.activator);
    } else {
        scene.unsupported_input("point_template", "ForceSpawn:repeat");
    }
    true
}
