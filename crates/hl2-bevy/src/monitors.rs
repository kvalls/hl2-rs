//! One shared procedural _rt_Camera texture, rendered from map-authored point cameras.
use crate::{
    assets::LoadedMap, campaign::MapOwned, gameplay::Gameplay, simulation::Simulation, sky::Sky,
    source_to_bevy,
};
use bevy::{
    camera::{RenderTarget, visibility::RenderLayers},
    core_pipeline::tonemapping::Tonemapping,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::render_resource::TextureFormat,
};

#[derive(Component)]
pub struct MonitorCamera;
#[derive(Resource)]
pub struct Monitors {
    pub target: Handle<Image>,
    materials: Vec<String>,
    material_details: serde_json::Value,
    material_samples: Vec<serde_json::Value>,
    proxy_sample_time: f64,
    active: Vec<usize>,
    selected: Option<usize>,
    frames: u64,
    error: Option<String>,
}
impl Monitors {
    /// The point_camera rendering `_rt_Camera` this frame.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
    pub fn report(&self, game: &Gameplay) -> serde_json::Value {
        serde_json::json!({"texture":"_rt_Camera","asset":format!("{:?}",self.target.id()),"size":[256,256],"materials":self.materials,"material_details":self.material_details,"eligible_cameras":self.active,
            "selected":self.selected.and_then(|id| game.scene.states.get(id).map(|s| serde_json::json!({"entity":id,"name":game.world.entities[id].get("targetname"),
                "origin":s.origin.to_array(),"fov":game.scene.monitors.cameras[&id].fov}))),
            "rendered_frames":self.frames,"visibility_error":self.error,
            "proxy_sample_time":self.proxy_sample_time,"material_samples":self.material_samples,
            "state":game.scene.monitors,
            "limitations":"BSP bounds/PVS eligibility, shared live world/actor feed; native area connectivity, client list ordering, remaining monitor proxies/fog and monitor sky rendering remain incomplete; recursive monitor feedback is excluded"})
    }
}
pub fn install(
    loaded: &LoadedMap,
    commands: &mut Commands,
    images: &mut Assets<Image>,
) -> Handle<Image> {
    let mut image = Image::new_target_texture(256, 256, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    let image = images.add(image);
    commands.spawn((
        MapOwned,
        MonitorCamera,
        Camera3d::default(),
        Camera {
            order: -3,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderTarget::Image(image.clone().into()),
        RenderLayers::layer(0),
        Tonemapping::None,
        // Integer HDR draws monitor views before TurnOnToneMapping, after the previous main
        // view reset the scale to 1 (SDK viewrender.cpp 2080/2090/2214).
        bevy::camera::Exposure {
            ev100: crate::tonemap::ev100(1.),
        },
        Msaa::Off,
        Projection::Perspective(PerspectiveProjection {
            near: 1.,
            far: 32000.,
            ..default()
        }),
        Transform::IDENTITY,
    ));
    commands.insert_resource(Monitors {
        target: image.clone(),
        materials: loaded
            .materials
            .iter()
            .filter(|(_, m)| m.camera)
            .map(|(n, _)| n.clone())
            .collect(),
        material_details: serde_json::json!(loaded.materials.iter().filter(|(_,m)|m.camera).map(|(name,m)| serde_json::json!({"material":name,"overlay":m.camera_overlay_path,"unsupported_proxies":m.camera_animation.unsupported})).collect::<Vec<_>>()),
        material_samples: Vec::new(),
        proxy_sample_time: 0.,
        active: vec![],
        selected: None,
        frames: 0,
        error: None,
    });
    image
}
pub fn present(
    mut monitors: ResMut<Monitors>,
    game: Res<Gameplay>,
    sim: Res<Simulation>,
    sky: Res<Sky>,
    mut cameras: Query<(&mut Camera, &mut Projection, &mut Transform), With<MonitorCamera>>,
) {
    let Ok((mut camera, mut projection, mut transform)) = cameras.single_mut() else {
        return;
    };
    let active = game.scene.monitors.eligible(&game.scene.states, |id| {
        let Some(state) = game.scene.states.get(id) else {
            return Ok(false);
        };
        let entity = &game.world.entities[id];
        let brush = entity
            .get("model")
            .and_then(|m| m.strip_prefix('*'))
            .and_then(|m| m.parse::<usize>().ok())
            .and_then(|id| game.world.brush_models.iter().find(|b| b.id == id));
        let (mins, maxs) = brush.map_or((state.origin, state.origin), |b| {
            let center = state.origin + state.rotation * ((b.mins + b.maxs) * 0.5);
            let extent = (b.maxs - b.mins) * 0.5;
            let r = glam::Mat3::from_quat(state.rotation);
            let extent =
                r.x_axis.abs() * extent.x + r.y_axis.abs() * extent.y + r.z_axis.abs() * extent.z;
            (center - extent, center + extent)
        });
        sky.bounds_in_pvs(
            glam::Vec3::from_array(sim.view_origin().to_array()),
            mins,
            maxs,
        )
    });
    match active {
        Ok(active) => {
            monitors.active = active;
            monitors.error = None;
        }
        Err(error) => {
            monitors.active.clear();
            monitors.error = Some(format!("{error:#}"));
        }
    }
    monitors.selected = monitors.active.last().copied();
    camera.is_active = monitors.selected.is_some();
    if let Some(id) = monitors.selected {
        let (Some(state), Some(definition)) = (
            game.scene.states.get(id),
            game.scene.monitors.cameras.get(&id),
        ) else {
            camera.is_active = false;
            return;
        };
        if !(0.01..179.).contains(&definition.fov) {
            camera.is_active = false;
            monitors.error = Some(format!(
                "point_camera {id} has invalid FOV {}",
                definition.fov
            ));
            return;
        }
        *transform =
            Transform::from_translation(source_to_bevy(Vec3::from_array(state.origin.to_array())))
                .looking_to(
                    source_to_bevy(Vec3::from_array(
                        (state.rotation * glam::Vec3::X).to_array(),
                    )),
                    source_to_bevy(Vec3::from_array(
                        (state.rotation * glam::Vec3::Z).to_array(),
                    )),
                );
        if let Projection::Perspective(p) = &mut *projection {
            p.fov = definition.fov.to_radians();
            p.aspect_ratio = 1.;
            p.far = if definition.fog_enabled {
                definition.fog_end.max(p.near + 1.)
            } else {
                32000.
            };
        }
        monitors.frames += 1;
    }
}

#[derive(Component)]
pub struct MonitorMaterial {
    pub name: String,
    pub animation: source_assets::monitor_material::Animation,
    pub color: [f32; 3],
    pub color2: [f32; 3],
}
pub fn present_materials(
    game: Res<Gameplay>,
    mut monitors: ResMut<Monitors>,
    draws: Query<(
        &MonitorMaterial,
        &MeshMaterial3d<crate::rendering::SourceMaterial>,
    )>,
    mut materials: ResMut<Assets<crate::rendering::SourceMaterial>>,
) {
    // Shared materials use one authored proxy and scene clock on every draw.
    let mut updated = std::collections::HashSet::new();
    monitors.material_samples.clear();
    monitors.proxy_sample_time = game.scene.time;
    for (definition, handle) in &draws {
        if !updated.insert(handle.0.id()) {
            continue;
        }
        let (color, uv) = definition
            .animation
            .sample(definition.color, game.scene.time as f32);
        let tint = Vec3::from_array(std::array::from_fn(|i| {
            linear_modulation(color[i] * definition.color2[i])
        }));
        let secondary_uv = uv_matrix(uv);
        if materials
            .get(&handle.0)
            .is_some_and(|m| m.tint.truncate() != tint || m.secondary_uv != secondary_uv)
            && let Some(mut material) = materials.get_mut(&handle.0)
        {
            material.tint = tint.extend(material.tint.w);
            material.secondary_uv = secondary_uv;
        }
        if let Some(material) = materials.get(&handle.0) {
            monitors.material_samples.push(serde_json::json!({
                "material":definition.name,"handle":format!("{:?}",handle.0.id()),
                "color":material.tint.to_array(),
                "secondary_uv":material.secondary_uv.to_cols_array_2d()
            }));
        }
    }
}
fn uv_matrix(uv: [[f32; 3]; 2]) -> Mat3 {
    Mat3::from_cols(
        Vec3::new(uv[0][0], uv[1][0], 0.),
        Vec3::new(uv[0][1], uv[1][1], 0.),
        Vec3::new(uv[0][2], uv[1][2], 1.),
    )
}

/// SDK SetModulationPixelShaderDynamicState_LinearColorSpace: ($color * $color2) channels
/// above 1 stay as they are, the rest go through mathlib GammaToLinear (a 256-entry
/// pow 2.2 table that returns 1 from 0.95 up).
pub(crate) fn linear_modulation(channel: f32) -> f32 {
    if channel > 1. {
        channel
    } else if channel < 0. {
        0.
    } else if channel >= 0.95 {
        1.
    } else {
        ((channel * 255.).round() / 255.).powf(2.2)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scanline_matrix_translates_texture_coordinates_downward_over_time() {
        use bevy::prelude::{Vec2, Vec3};
        let uv = super::uv_matrix([[1., 0., 0.], [0., 1., -0.91]]);
        let transformed = (uv * Vec3::new(0.25, 0.75, 1.)).truncate();
        assert!((transformed - Vec2::new(0.25, -0.16)).length() < 1e-6);
    }
    #[test]
    fn modulation_matches_sdk_gamma_to_linear() {
        use super::linear_modulation as m;
        assert_eq!(m(2.5), 2.5);
        assert_eq!(m(0.96), 1.);
        assert_eq!(m(-1.), 0.);
        assert!((m(0.4) - (102f32 / 255.).powf(2.2)).abs() < 1e-6);
    }
}
