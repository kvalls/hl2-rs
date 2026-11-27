//! Ordered owned LDR background and miniature scenery, before playable-world depth.
use crate::{FlyCamera, assets::LoadedMap, bevy_to_source, rendering, source_to_bevy};
use bevy::{
    asset::RenderAssetUsages,
    camera::{RenderTarget, visibility::RenderLayers},
    core_pipeline::tonemapping::Tonemapping,
    mesh::Indices,
    prelude::*,
    render::render_resource::PrimitiveTopology,
};
use source_assets::{
    bsp::{Bsp, SkyVisibility},
    sky::Skybox,
};

#[derive(Component)]
pub struct SkyCamera;
/// A 2D sky face; it follows the sky camera at FACE_DISTANCE.
#[derive(Component)]
pub struct SkyFace;
/// How far out the 2D sky faces are from the sky camera (Source units).
const FACE_DISTANCE: f32 = 16000.;
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
    target: Option<&Handle<Image>>,
) {
    if let Some(sky) = &loaded.sky {
        spawn_skybox_faces(sky, commands, meshes, materials, images);
    }
    let target_comp = target
        .map(|t| RenderTarget::Image(t.clone().into()))
        .unwrap_or_default();
    // One camera draws the 2D sky faces (layer 3) and the miniature sky (layer 4) in one pass:
    // the faces sit FACE_DISTANCE out around it, behind every miniature-sky surface (checked by
    // source_assets' owned sky test), so a camera's whole fixed cost is not paid for six quads.
    commands.spawn((
        crate::campaign::MapOwned,
        Camera3d::default(),
        bevy::render::camera::CameraRenderGraph::new(crate::rendering::simple_pass::Simple3d),
        bevy::light::cluster::ClusterConfig::Single,
        Camera {
            order: -1,
            is_active: false,
            msaa_writeback: bevy::camera::MsaaWriteback::Off,
            ..default()
        },
        target_comp,
        RenderLayers::layer(4),
        SkyCamera,
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

fn spawn_skybox_faces(
    sky: &Skybox,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<rendering::SourceMaterial>,
    images: &mut Assets<Image>,
) {
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
                        Vec3::from_array(source_assets::sky::face_vector(face, s, t).to_array())
                            * FACE_DISTANCE,
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
            let image = Image::new(
                bevy::render::render_resource::Extent3d {
                    width: hdr.width.into(),
                    height: hdr.height.into(),
                    depth_or_array_layers: 1,
                },
                bevy::render::render_resource::TextureDimension::D2,
                hdr.rgba.iter().flat_map(|t| t.to_le_bytes()).collect(),
                bevy::render::render_resource::TextureFormat::Rgba16Float,
                RenderAssetUsages::RENDER_WORLD,
            );
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
            MeshMaterial3d(materials.add(face_material(base, linear, (&white, &black_cube)))),
            Transform::IDENTITY,
            RenderLayers::layer(3),
            SkyFace,
        ));
    }
}
type SkyCameras<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Camera,
        &'static mut Projection,
        &'static mut RenderLayers,
    ),
    (With<SkyCamera>, Without<FlyCamera>),
>;
type SkyFaces<'w, 's> =
    Query<'w, 's, &'static mut Transform, (With<SkyFace>, Without<SkyCamera>, Without<FlyCamera>)>;

pub fn present(
    mut sky: ResMut<Sky>,
    frame: Option<Res<crate::video::GameFrame>>,
    mut world: Query<(&Transform, &mut Camera, &Projection), With<FlyCamera>>,
    mut backgrounds: SkyCameras,
    mut faces: SkyFaces,
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
    let flat = frame.as_ref().and_then(|f| f.budget.flat_sky);
    let visible_2d = flat.is_none() && sky.visible.background_visible() && !sky.faces.is_null();
    let visible_3d = flat.is_none() && sky.visible.sky_3d && sky.background.is_some();
    world_camera.clear_color = if let Some([r, g, b]) = flat {
        ClearColorConfig::Custom(Color::linear_rgb(r, g, b))
    } else if visible_2d || visible_3d {
        ClearColorConfig::None
    } else {
        ClearColorConfig::Default
    };
    sky.frames_2d += u64::from(visible_2d);
    sky.frames_3d += u64::from(visible_3d);
    for (mut transform, mut camera, mut projection, mut layers) in &mut backgrounds {
        // Follow scripted world FOV changes; keep the sky's own clip planes.
        if let (Projection::Perspective(sky_p), Projection::Perspective(world_p)) =
            (&mut *projection, world_projection)
            && sky_p.fov != world_p.fov
        {
            sky_p.fov = world_p.fov;
        }
        if frame.as_ref().is_some_and(|f| f.suspended > 0) {
            camera.is_active = false;
            continue;
        }
        // Each sky draws only while the BSP says it is eligible: 2D faces on layer 3, the
        // miniature sky on layer 4, behind them nothing (the pass clears).
        camera.is_active = visible_2d || visible_3d;
        let wanted = sky_layers(visible_2d, visible_3d);
        if *layers != wanted {
            *layers = wanted;
        }
        transform.rotation = world_transform.rotation;
        let at = match (&sky.background, visible_3d) {
            (Some(b), true) => {
                source_to_bevy(Vec3::from_array((b.origin + eye / b.scale).to_array()))
            }
            _ => Vec3::ZERO,
        };
        transform.translation = at;
        // The faces stay centred on the camera, so they sit FACE_DISTANCE out in every view.
        for mut face in &mut faces {
            if face.translation != at {
                face.translation = at;
            }
        }
    }
}

/// The sky camera's layers for the skies the BSP allows this frame.
fn sky_layers(visible_2d: bool, visible_3d: bool) -> RenderLayers {
    let mut layers = RenderLayers::none();
    if visible_2d {
        layers = layers.with(3);
    }
    if visible_3d {
        layers = layers.with(4);
    }
    layers
}

/// A 2D sky face's material: its texture unlit (`linear`: already linear HDR), no flashlight.
fn face_material(
    base: Handle<Image>,
    linear: bool,
    (white, black_cube): (&Handle<Image>, &Handle<Image>),
) -> rendering::SourceMaterial {
    rendering::SourceMaterial {
        tint: Vec4::ONE,
        parameters: Vec4::new(0., 1., 0., if linear { -2. } else { 0. }),
        base,
        lightmap: white.clone(),
        iris: white.clone(),
        secondary_uv: Mat3::IDENTITY,
        lighting: rendering::ModelLighting {
            params: Vec4::W,
            ..Default::default()
        },
        flashlight: white.clone(),
        viewmodel_flashlight: Default::default(),
        eye_rows: crate::eyes::EYE_ROWS,
        material_rows: crate::material_slots::MATERIAL_ROWS,
        light_rows: crate::material_slots::LIGHT_ROWS,
        dynamic: bevy::math::Vec4::new(-1., 0., -1., 0.),
        envmap: black_cube.clone(),
        envmap_tint: Vec4::ZERO,
        envmap_contrast: Vec4::ZERO,
        envmap_saturation: Vec4::ZERO,
        envmap_mask: white.clone(),
        self_illum: Vec4::ZERO,
        decal_order: 0,
        alpha: AlphaMode::Opaque,
        two_sided: true,
    }
}

#[cfg(test)]
mod tests {
    use super::{RenderLayers, sky_layers};

    #[test]
    fn the_sky_pass_draws_only_the_skies_the_bsp_allows() {
        assert_eq!(sky_layers(true, true), RenderLayers::from_layers(&[3, 4]));
        assert_eq!(sky_layers(true, false), RenderLayers::layer(3));
        assert_eq!(sky_layers(false, true), RenderLayers::layer(4));
        assert_eq!(sky_layers(false, false), RenderLayers::none());
    }
}
