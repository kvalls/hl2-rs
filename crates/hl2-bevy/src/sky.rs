//! Ordered owned LDR background and miniature scenery, before playable-world depth.
use crate::{FlyCamera, assets::LoadedMap, bevy_to_source, rendering, source_to_bevy};
use bevy::{
    asset::RenderAssetUsages, camera::visibility::RenderLayers,
    core_pipeline::tonemapping::Tonemapping, mesh::Indices, prelude::*,
    render::render_resource::PrimitiveTopology,
};
use source_assets::{
    bsp::{Bsp, SkyVisibility},
    sky::Skybox,
};

#[derive(Component)]
pub struct SkyCamera(bool);
#[derive(Resource)]
pub struct Sky {
    bsp: Bsp,
    background: Option<modkit_core::BackgroundCamera>,
    faces: serde_json::Value,
    visible: SkyVisibility,
    frames_2d: u64,
    frames_3d: u64,
    error: Option<String>,
}
impl Sky {
    pub fn new(
        bsp: Bsp,
        background: Option<modkit_core::BackgroundCamera>,
        sky: Option<&Skybox>,
    ) -> Self {
        Self { bsp, background, faces: sky.map_or(serde_json::Value::Null, |s| serde_json::json!({
            "name":s.name,"faces":s.faces.iter().map(|f|serde_json::json!({"suffix":f.suffix,"texture":f.texture,
            "width":f.image.width,"height":f.image.height,"uv_transform":f.transform.rows()})).collect::<Vec<_>>()
        })), visible: SkyVisibility::default(), frames_2d:0, frames_3d:0, error:None }
    }
    pub fn bounds_in_pvs(
        &self,
        eye: glam::Vec3,
        mins: glam::Vec3,
        maxs: glam::Vec3,
    ) -> anyhow::Result<bool> {
        self.bsp.bounds_in_pvs(eye, mins, maxs)
    }
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"ldr":self.faces,"leaf_visibility":self.visible,"frames_2d":self.frames_2d,
            "frames_3d":self.frames_3d,"visibility_error":self.error,
            "limitations":"LDR only; leaf eligibility and world-depth occlusion, not Source sky polygon masks/HDR/fog"})
    }
}
pub fn install(
    loaded: &LoadedMap,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<rendering::SourceMaterial>,
    images: &mut Assets<Image>,
) {
    if let Some(sky) = &loaded.sky {
        let white = images.add(rendering::image(1, 1, vec![255; 4], false));
        let black_cube = images.add(rendering::black_cube());
        for (face, data) in sky.faces.iter().enumerate() {
            let corners = [(-1., -1.), (-1., 1.), (1., 1.), (1., -1.)];
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            );
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                corners
                    .iter()
                    .map(|&(s, t)| {
                        source_to_bevy(
                            Vec3::from_array(
                                source_assets::sky::face_vector(face, s, t).to_array(),
                            ) * 16000.,
                        )
                        .to_array()
                    })
                    .collect::<Vec<_>>(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 1., 0.]; 4]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.; 4]; 4]);
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_UV_0,
                corners
                    .iter()
                    .map(|&(s, t)| source_assets::sky::face_uv(s, t, data.transform).to_array())
                    .collect::<Vec<_>>(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.; 2]; 4]);
            mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
            // HDR faces are linear RGBA16F, drawn through the shader's linear-base path
            // (parameters.w = -2 with a white second texture), times the tonemap scale.
            let (base, linear) = if let Some(hdr) = &data.hdr {
                let mut image = rendering::image(
                    hdr.width,
                    hdr.height,
                    hdr.rgba.iter().flat_map(|t| t.to_le_bytes()).collect(),
                    false,
                );
                image.texture_descriptor.format =
                    bevy::render::render_resource::TextureFormat::Rgba16Float;
                (images.add(image), true)
            } else {
                let image = rendering::image(
                    data.image.width,
                    data.image.height,
                    data.image.rgba.clone(),
                    false,
                );
                (images.add(image), false)
            };
            commands.spawn((
                crate::campaign::MapOwned,
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(rendering::SourceMaterial {
                    tint: Vec4::ONE,
                    parameters: Vec4::new(0., 1., 0., if linear { -2. } else { 0. }),
                    base,
                    lightmap: white.clone(),
                    iris: white.clone(),
                    secondary_uv: Mat3::IDENTITY,
                    lighting: Default::default(),
                    envmap: black_cube.clone(),
                    envmap_tint: Vec4::ZERO,
                    envmap_contrast: Vec4::ZERO,
                    envmap_saturation: Vec4::ZERO,
                    alpha: AlphaMode::Opaque,
                    two_sided: true,
                })),
                Transform::IDENTITY,
                RenderLayers::layer(3),
            ));
        }
    }
    for (is_2d, layer, order) in [(true, 3, -2), (false, 4, -1)] {
        commands.spawn((
            crate::campaign::MapOwned,
            Camera3d::default(),
            Camera {
                order,
                is_active: false,
                ..default()
            },
            RenderLayers::layer(layer),
            SkyCamera(is_2d),
            Tonemapping::None,
            // Sky_HDR_DX9 and the miniature 3D skybox both end in TONEMAP_SCALE_LINEAR.
            bevy::camera::Exposure::default(),
            crate::tonemap::ToneMapped,
            Msaa::Off,
            Projection::Perspective(PerspectiveProjection {
                fov: 2. * ((75f32.to_radians() / 2.).tan() / (4. / 3.)).atan(),
                near: 0.1,
                far: 32000.,
                ..default()
            }),
            Transform::IDENTITY,
        ));
    }
}
pub fn present(
    mut sky: ResMut<Sky>,
    mut world: Query<(&Transform, &mut Camera, &Projection), With<FlyCamera>>,
    mut backgrounds: Query<
        (&SkyCamera, &mut Transform, &mut Camera, &mut Projection),
        (Without<FlyCamera>,),
    >,
) {
    let Ok((world_transform, mut world_camera, world_projection)) = world.single_mut() else {
        return;
    };
    let eye = glam::Vec3::from_array(bevy_to_source(world_transform.translation).to_array());
    sky.visible = match sky.bsp.sky_visibility(eye) {
        Ok(v) => v,
        Err(e) => {
            if sky.error.is_none() {
                sky.error = Some(format!("{e:#}"));
            }
            SkyVisibility::default()
        }
    };
    let visible_2d = sky.visible.background_visible() && !sky.faces.is_null();
    let visible_3d = sky.visible.sky_3d && sky.background.is_some();
    world_camera.clear_color = if visible_2d || visible_3d {
        ClearColorConfig::None
    } else {
        ClearColorConfig::Default
    };
    sky.frames_2d += u64::from(visible_2d);
    sky.frames_3d += u64::from(visible_3d);
    for (kind, mut transform, mut camera, mut projection) in &mut backgrounds {
        // Follow scripted world FOV changes; keep the sky's own clip planes.
        if let (Projection::Perspective(sky_p), Projection::Perspective(world_p)) =
            (&mut *projection, world_projection)
            && sky_p.fov != world_p.fov
        {
            sky_p.fov = world_p.fov;
        }
        camera.is_active = if kind.0 { visible_2d } else { visible_3d };
        camera.clear_color = if !kind.0 && visible_2d {
            ClearColorConfig::None
        } else {
            ClearColorConfig::Default
        };
        transform.rotation = world_transform.rotation;
        transform.translation = if kind.0 {
            Vec3::ZERO
        } else {
            sky.background.as_ref().map_or(Vec3::ZERO, |b| {
                source_to_bevy(Vec3::from_array((b.origin + eye / b.scale).to_array()))
            })
        };
    }
}
