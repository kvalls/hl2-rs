//! Point and environment entities: view control, zoom, screen fades, the tonemap controller,
//! physics explosions and impacts, ambient sounds and Combine cameras.

use super::{Delivery, Scene};
use crate::entities::number;
use glam::Vec3;
use modkit_core::parse_vec3;

/// CTriggerCamera Enable / Disable: the view follows this camera until Disable (or its wait
/// ends).
pub(super) fn viewcontrol(scene: &mut Scene, d: &Delivery) -> bool {
    let (id, p) = (d.id, d.pending);
    match d.input {
        "enable" => {
            for key in crate::view_control::ViewControl::unsupported(d.world, id) {
                scene.unsupported_input(d.class, key);
            }
            if let Some(previous) = scene.view_control.enable(id, scene.time) {
                scene.fire(previous, "OnEndFollow", p.activator);
            }
        }
        "disable" => {
            if scene.view_control.disable(id) {
                scene.fire(id, "OnEndFollow", p.activator);
            }
        }
        _ => return false,
    }
    true
}

/// CEnvZoom Zoom / UnZoom: the player's field of view moves to FOV over Rate seconds.
pub(super) fn zoom(scene: &mut Scene, d: &Delivery) -> bool {
    match d.input {
        "zoom" => {
            scene.view_control.zoom = Some(crate::view_control::Zoom {
                fov: number(d.entity, "FOV", 0.),
                rate: number(d.entity, "Rate", 0.),
                started: scene.time,
            });
        }
        "unzoom" => scene.view_control.zoom = None,
        _ => return false,
    }
    true
}

/// CEnvFade::InputFade.
pub(super) fn fade(scene: &mut Scene, d: &Delivery) -> bool {
    if d.input != "fade" {
        return false;
    }
    let (e, p) = (d.entity, d.pending);
    // CEnvFade::InputFade: SF_FADE_IN 1, MODULATE 2, ONLYONE 4 (the activating
    // player only), STAYOUT 8; everyone else gets FFADE_PURGE.
    use crate::player_damage::{
        ScreenFade, FFADE_IN, FFADE_MODULATE, FFADE_OUT, FFADE_PURGE, FFADE_STAYOUT,
    };
    let flags = number(e, "spawnflags", 0.) as u32;
    let mut fade = if flags & 1 != 0 { FFADE_IN } else { FFADE_OUT };
    if flags & 2 != 0 {
        fade |= FFADE_MODULATE;
    }
    if flags & 8 != 0 {
        fade |= FFADE_STAYOUT;
    }
    if flags & 4 == 0 {
        fade |= FFADE_PURGE;
    }
    if flags & 4 == 0 || p.activator == usize::MAX {
        let rgb =
            parse_vec3(e.get("rendercolor").unwrap_or("255 255 255")).unwrap_or(Vec3::splat(255.));
        let alpha = number(e, "renderamt", 255.);
        let byte = |v: f32| v.clamp(0., 255.) as u8;
        scene.screen_fades.push(ScreenFade::new(
            [byte(rgb.x), byte(rgb.y), byte(rgb.z), byte(alpha)],
            number(e, "duration", 0.),
            number(e, "holdtime", 0.),
            fade,
        ));
    }
    scene.fire(d.id, "OnBeginFade", p.activator);
    true
}

/// CEnvTonemapController: custom auto-exposure limits and the manual tonemap rate.
pub(super) fn tonemap_controller(scene: &mut Scene, d: &Delivery) -> bool {
    let value = d.value;
    match d.input {
        "setautoexposuremin" => scene.tonemap.min = value,
        "setautoexposuremax" => scene.tonemap.max = value,
        "settonemaprate" => scene.tonemap.rate = value,
        "setbloomscale" => scene.tonemap.bloom = value,
        "usedefaultautoexposure" => {
            scene.tonemap = crate::tonemap::Control {
                rate: scene.tonemap.rate,
                bloom: scene.tonemap.bloom,
                ..Default::default()
            };
        }
        _ => return false,
    }
    true
}

/// color_correction and color_correction_volume: Enable, Disable and the fade durations.
pub(super) fn color_correction(scene: &mut Scene, d: &Delivery) -> bool {
    scene
        .color_correction
        .input(d.id, d.input, d.value, scene.time)
}

/// Explode.
pub(super) fn physexplosion(scene: &mut Scene, d: &Delivery) -> bool {
    if d.input != "explode" {
        return false;
    }
    scene.phys_explosions.push(d.id);
    true
}

/// Impact.
pub(super) fn physimpact(scene: &mut Scene, d: &Delivery) -> bool {
    if d.input != "impact" {
        return false;
    }
    scene.phys_impacts.push(d.id);
    true
}

/// PlaySound, StopSound, ToggleSound.
pub(super) fn ambient_generic(scene: &mut Scene, d: &Delivery) -> bool {
    let (input, id, e) = (d.input, d.id, d.entity);
    if !matches!(input, "playsound" | "stopsound" | "togglesound") {
        return false;
    }
    // SDK CAmbientGeneric: looping sounds are active from spawn unless they start
    // silent; PlaySound restarts an inactive sound, StopSound stops an active one,
    // and only looping sounds stay active.
    let flags = e
        .get("spawnflags")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(0);
    let looping = flags & 32 == 0;
    let active = *scene
        .ambient_active
        .get(&id)
        .unwrap_or(&(looping && flags & 16 == 0));
    let play = match input {
        "playsound" => !active,
        "stopsound" => false,
        _ => !active,
    };
    if let Some(sound) = e.get("message").filter(|m| !m.is_empty()) {
        if play {
            scene.ambient_active.insert(id, looping);
            scene.sounds.push(crate::sounds::SoundRequest {
                ambient: Some(crate::sounds::AmbientControl::Play(id)),
                ..sound.into()
            });
        } else if active && input != "playsound" {
            scene.ambient_active.insert(id, false);
            scene.sounds.push(crate::sounds::SoundRequest {
                ambient: Some(crate::sounds::AmbientControl::Stop(id)),
                ..sound.into()
            });
        }
    }
    true
}

/// CNPC_CombineCamera Enable / Disable / Toggle drive its open/closed activity.
pub(super) fn combine_camera(scene: &mut Scene, d: &Delivery) -> bool {
    let id = d.id;
    let enabled = match d.input {
        "enable" => true,
        "disable" => false,
        "toggle" => !scene.states[id].enabled,
        _ => return false,
    };
    if enabled != scene.states[id].enabled {
        scene.states[id].enabled = enabled;
        scene.camera_activity(d.world, id, true);
    }
    true
}

/// CPointCommentaryNode StartCommentary / StartUnstoppableCommentary / Enable / Disable.
pub(super) fn commentary_node(scene: &mut Scene, d: &Delivery) -> bool {
    crate::commentary::CommentaryState::handle_input(scene, d.world, d.id, d.input)
}
