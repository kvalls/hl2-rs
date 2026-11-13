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
            let mut zoom = scene.view_control.zoom.clone().unwrap_or_default();
            zoom.set(
                number(d.entity, "FOV", 0.) as i32,
                number(d.entity, "Rate", 0.),
                scene.time,
            );
            scene.view_control.zoom = Some(zoom);
        }
        "unzoom" => scene.view_control.zoom = None,
        _ => return false,
    }
    true
}

/// script_intro (crate::script_intro): Activate, Deactivate, SetCameraViewEntity, the blend
/// modes and times, the camera view's field of view and the fades.
pub(super) fn script_intro(scene: &mut Scene, d: &Delivery) -> bool {
    let camera = (d.input == "setcameraviewentity")
        .then(|| {
            let mut lookup = d.pending.clone();
            lookup.target = d.pending.parameter.clone();
            let found = scene.targets(d.world, &lookup).into_iter();
            found.into_iter().find(|&id| id != usize::MAX)
        })
        .flatten();
    let player_fov = scene
        .view_control
        .zoom
        .as_ref()
        .map_or(75., |z| z.get(scene.time));
    let spline = number(d.entity, "alternatefovchange", 0.) != 0.;
    scene.intro.script.set_spline_fov(spline);
    let parameter = &d.pending.parameter;
    scene
        .intro
        .script
        .input(d.input, parameter, scene.time, camera, player_fov)
}

/// CCredits::InputShowLogo: the logo holds `logolength` seconds (LogoTimeMsg), else 5
/// (CreditsMsg CREDITS_LOGO). RollCredits and RollOutroCredits are not in.
pub(super) fn credits(scene: &mut Scene, d: &Delivery) -> bool {
    if d.input != "showlogo" {
        return false;
    }
    let length = number(d.entity, "logolength", 0.);
    scene.intro.logo = Some((scene.time, if length != 0. { length } else { 5. }));
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

/// env_shake's StartShake, StopShake, Amplitude and Frequency (EnvShake.cpp ApplyShake through
/// UTIL_ScreenShake): the player within `radius` (any distance with spawnflag 1, Everyone) gets a
/// view shake whose amplitude falls off with distance; Amplitude and Frequency keep the new value
/// for later starts. "Don't shake view" (0x20) and the physics and rope shakes are not drawn.
pub(super) fn shake(scene: &mut Scene, d: &Delivery) -> bool {
    use crate::screen_shake::{local_amplitude, Command};
    let key = |name: &str| d.entity.get(name).and_then(|v| v.parse::<f32>().ok());
    let command = match d.input {
        "startshake" => Command::Start,
        "stopshake" => Command::Stop,
        "amplitude" => Command::Amplitude,
        "frequency" => Command::Frequency,
        _ => return false,
    };
    scene.shakes.tune(d.id, command, d.value);
    let (tuned_amplitude, tuned_frequency) = scene.shakes.tuned(d.id);
    let amplitude = tuned_amplitude.unwrap_or_else(|| key("amplitude").unwrap_or(0.));
    let frequency = tuned_frequency.unwrap_or_else(|| key("frequency").unwrap_or(0.));
    let spawnflags = key("spawnflags").unwrap_or(0.) as i64;
    if spawnflags & 0x20 != 0 {
        return true;
    }
    let radius = if spawnflags & 1 != 0 {
        0.
    } else {
        key("radius").unwrap_or(0.)
    };
    // UTIL_ScreenShake measures to the player's WorldSpaceCenter, 36 units above the feet.
    let player = d.player_feet + glam::Vec3::Z * 36.;
    let Some(local) = local_amplitude(scene.states[d.id].origin, player, amplitude, radius) else {
        return true;
    };
    let duration = key("duration").unwrap_or(0.);
    let local = if command == Command::Stop { 0. } else { local };
    let now = scene.time;
    if local > 0. || command == Command::Stop {
        scene
            .shakes
            .command(command, (local, frequency, duration), now);
    }
    true
}

/// point_spotlight's LightOn and LightOff (its beam, DESIGN 14t).
pub(super) fn point_spotlight(scene: &mut Scene, d: &Delivery) -> bool {
    scene.spotlights.input(d.id, d.input)
}
