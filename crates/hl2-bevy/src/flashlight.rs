//! The HEV flashlight in the host: `impulse 100` (the F key and the console) toggles it, its power runs each
//! frame ([`hl2_simulation::flashlight`]), its sounds play, and its light is a Bevy `SpotLight` placed as
//! `UpdateLightNew` places the flashlight. `source.wgsl` reads that light: it projects the installed cookie
//! (`effects/flashlight_border`, bound in every Source material) with the engine's attenuation, Lambert term
//! and far fade; the light's radius carries the linear attenuation (the low-power flicker). Its square
//! depth frustum matches the cookie; the circle-bounded cookie fits the spotlight cone. Episode Two keeps
//! its own battery; HL2, Episode One and Lost Coast run the flashlight on suit power.
mod depth;
mod viewmodel;
pub(crate) use viewmodel::ViewmodelLight;

use crate::{
    FlyCamera, campaign::Campaign, gameplay::Gameplay, simulation::Simulation, ui_host::Console,
};
use bevy::{
    ecs::{
        entity::Entity,
        query::{With, Without},
        resource::Resource,
        system::{Commands, Query, Res, ResMut},
    },
    light::SpotLight,
    math::Vec3,
    time::{Time, Virtual},
    transform::components::Transform,
};
use hl2_simulation::flashlight::{Context, Flashlight, Light, Placer};
use source_assets::{vpk::Vfs, vtf};

/// The flashlight's projected texture (`CFlashlightEffect`, hardware with border colour).
pub const COOKIE: &str = "materials/effects/flashlight_border.vtf";

/// The cookie's pixels, read on the map worker.
pub fn load_cookie(vfs: &Vfs) -> Option<vtf::Image> {
    let data = vfs.read(COOKIE).ok()??;
    vtf::decode_picmip(&data, 2048, 0).ok()
}

/// The cookie as a GPU texture: sRGB, clamped (its border is black); black without one.
pub fn cookie_image(cookie: Option<&vtf::Image>) -> bevy::image::Image {
    let mut image = match cookie {
        Some(c) => crate::rendering::image(c.width, c.height, c.rgba.clone(), false),
        None => crate::rendering::image(1, 1, vec![0, 0, 0, 255], false),
    };
    image.texture_descriptor.format = bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb;
    image
}

#[derive(Resource, Default)]
pub struct HostFlashlight {
    placer: Placer,
    light: Option<Light>,
    entity: Option<Entity>,
    depth_shadows: bool,
}

impl HostFlashlight {
    pub(crate) fn light(&self) -> Option<Light> {
        self.light
    }

    pub fn report(&self, state: Option<&Flashlight>) -> serde_json::Value {
        serde_json::json!({"state": state, "light": self.light, "depth_shadows":self.depth_shadows, "depth_resolution":depth::RESOLUTION})
    }
}

/// The world camera's eye and view basis in Source axes.
fn view(camera: &Transform) -> (glam::Vec3, glam::Vec3, glam::Vec3, glam::Vec3) {
    let source = |v: Vec3| glam::Vec3::from_array(crate::bevy_to_source(v).to_array());
    (
        source(camera.translation),
        source(camera.forward().as_vec3()),
        source(camera.right().as_vec3()),
        source(camera.up().as_vec3()),
    )
}

type Scene<'w> = (
    Option<Res<'w, Console>>,
    Res<'w, Campaign>,
    Option<ResMut<'w, Gameplay>>,
    ResMut<'w, Simulation>,
);

/// Applies toggles, runs the power and places the light.
pub(crate) fn update(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    (ui, campaign, game, mut sim): Scene,
    mut host: ResMut<HostFlashlight>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut lights: Query<(&mut Transform, &mut SpotLight), Without<FlyCamera>>,
) {
    let Some(mut game) = game else {
        return;
    };
    let name = campaign
        .loaded_map
        .as_ref()
        .map_or("hl2", |m| m.game.as_str());
    let context = Context {
        suit: game.inventory.suit,
        episodic: matches!(name, "episodic" | "ep2"),
        easy: ui
            .as_ref()
            .is_some_and(|ui| ui.source.config.gameplay.skill == 1),
        now: game.scene.time,
    };
    let host = &mut *host;
    let Gameplay {
        scene,
        sound_requests,
        ..
    } = &mut *game;
    let was_on = scene.flashlight.state().is_some_and(|state| state.on);
    let state = scene.flashlight.step(
        name != "ep2",
        std::mem::take(&mut sim.flashlight_toggles),
        !sim.player.dead,
        (!sim.paused && !sim.loading).then_some(time.delta_secs()),
        context,
    );
    if state.on && !was_on {
        host.placer.turn_on();
    }
    let on = state.on && !sim.loading;
    let power = state.power();
    for sound in state.sounds.drain(..) {
        sound_requests.push(sound.into());
    }
    let placed = camera.single().ok().filter(|_| on).map(|camera| {
        let physics = &sim.physics;
        let trace = |from: glam::Vec3, to: glam::Vec3| {
            let delta = to - from;
            let length = delta.length();
            if length < 1e-3 {
                return 1.;
            }
            physics
                .impact_hull(from, delta / length, length, glam::Vec3::splat(4.))
                .map_or(1., |hit| {
                    ((hit.position - from).length() / length).clamp(0., 1.)
                })
        };
        host.placer
            .place(view(camera), &trace, (power, context.now as f32))
    });
    host.light = placed;
    let detail = ui
        .as_ref()
        .map(|ui| ui.source.config.video.advanced.shadow_detail);
    host.depth_shadows = depth::enabled(placed, detail == Some(hl2_ui::config::ShadowDetail::High));
    show(&mut commands, host, &mut lights);
}

/// Spawns, moves or removes the flashlight's spot light.
fn show(
    commands: &mut Commands,
    host: &mut HostFlashlight,
    lights: &mut Query<(&mut Transform, &mut SpotLight), Without<FlyCamera>>,
) {
    let Some(light) = host.light else {
        if let Some(entity) = host.entity.take() {
            commands.entity(entity).despawn();
        }
        return;
    };
    let bevy = |v: glam::Vec3| crate::source_to_bevy(Vec3::from_array(v.to_array()));
    let transform = Transform::from_translation(bevy(light.origin))
        .looking_to(bevy(light.direction), bevy(light.up));
    let spot = depth::spot(light, host.depth_shadows);
    match host.entity.and_then(|e| lights.get_mut(e).ok()) {
        Some((mut current, mut current_spot)) => {
            *current = transform;
            *current_spot = spot;
        }
        None => host.entity = Some(commands.spawn((transform, spot)).id()),
    }
}

/// The flashlight resource and its update, after the camera is placed for the frame.
pub struct FlashlightPlugin;

impl bevy::app::Plugin for FlashlightPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        use bevy::ecs::schedule::IntoScheduleConfigs;
        app.insert_resource(bevy::light::DirectionalLightShadowMap {
            size: depth::RESOLUTION,
        })
        .init_resource::<HostFlashlight>()
        .add_systems(
            bevy::app::PostUpdate,
            update
                .before(crate::rendering::present_lighting)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn the_installed_cookie_is_square_with_a_black_border() {
        let root = source_assets::install::discover().unwrap();
        let vfs = Vfs::mount(&root).unwrap();
        let cookie = load_cookie(&vfs).expect("effects/flashlight_border");
        let (w, h) = (usize::from(cookie.width), usize::from(cookie.height));
        let texel = |x: usize, y: usize| &cookie.rgba[(y * w + x) * 4..(y * w + x) * 4 + 3];
        let centre: u32 = texel(w / 2, h / 2).iter().map(|&c| u32::from(c)).sum();
        let edge: u32 = (0..w)
            .map(|x| texel(x, 0).iter().map(|&c| u32::from(c)).sum::<u32>())
            .max()
            .unwrap();
        eprintln!("cookie {w}x{h} centre {centre} brightest top-edge texel {edge}");
        assert_eq!(w, h);
        assert!(centre > 300, "a bright centre");
        assert!(edge < 30, "a black border");
        let outside_circle = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let dx = (x as f32 + 0.5) * 2. / w as f32 - 1.;
                let dy = (y as f32 + 0.5) * 2. / h as f32 - 1.;
                dx * dx + dy * dy > 1.
            })
            .map(|(x, y)| texel(x, y).iter().copied().max().unwrap())
            .max()
            .unwrap_or(0);
        assert!(
            outside_circle <= 1,
            "the cookie fits the matching spotlight cone: {outside_circle}"
        );
    }
}
