//! Bevy presentation and WGPU 2D batch rendering for the shared Source UI Canvas.
mod frame_distribution;
mod frame_windows;
mod game_hud;

use bevy::{
    asset::RenderAssetUsages,
    camera::{
        primitives::{Aabb, MeshAabb},
        visibility::RenderLayers,
    },
    image::{ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, MeshVertexBufferLayoutRef},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, Extent3d,
        PrimitiveTopology, RenderPipelineDescriptor, SpecializedMeshPipelineError,
        TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
    sprite_render::{AlphaMode2d, Material2d, Material2dKey},
    window::PrimaryWindow,
};
use source_ui::canvas::{FilterMode, Quad, Texture2D};
use std::collections::HashMap;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(HudMaterialKey)]
pub struct HudMaterial {
    #[texture(0)]
    #[sampler(1)]
    texture: Handle<Image>,
    additive: bool,
    modulate: bool,
}
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct HudMaterialKey {
    additive: bool,
    modulate: bool,
}
impl From<&HudMaterial> for HudMaterialKey {
    fn from(material: &HudMaterial) -> Self {
        Self {
            additive: material.additive,
            modulate: material.modulate,
        }
    }
}
impl Material2d for HudMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/hud.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let color = if key.bind_group_data.modulate {
            // Frame x color.
            Some(BlendComponent {
                src_factor: BlendFactor::Zero,
                dst_factor: BlendFactor::Src,
                operation: BlendOperation::Add,
            })
        } else if key.bind_group_data.additive {
            Some(BlendComponent {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            })
        } else {
            None
        };
        if let Some(color) = color {
            for target in descriptor
                .fragment
                .as_mut()
                .into_iter()
                .flat_map(|f| &mut f.targets)
                .flatten()
            {
                target.blend = Some(BlendState {
                    color,
                    alpha: BlendComponent::OVER,
                });
            }
        }
        Ok(())
    }
}
struct GpuTexture {
    // Retain CPU ownership so pointer identities cannot be reused for different glyphs.
    _cpu: Texture2D,
    normal: Handle<HudMaterial>,
    additive: Handle<HudMaterial>,
}
#[derive(Default)]
struct FrameTiming {
    frames: std::collections::VecDeque<f64>,
    frame_ms: f64,
    frames_seen: u64,
    warmup_end: Option<f64>,
    distribution: frame_distribution::Distribution,
    windows: frame_windows::Windows,
}
impl FrameTiming {
    fn observe(&mut self, now: f64, profile: bool) {
        self.frames_seen += 1;
        if self.frames_seen == 120 {
            self.warmup_end = Some(now);
        }
        if let Some(last) = self.frames.back() {
            self.frame_ms = (now - last).max(0.) * 1000.;
        }
        if profile && self.frames_seen > 120 {
            self.distribution.observe(self.frame_ms);
            self.windows.observe(now, self.frame_ms);
        }
        self.frames.push_back(now);
        while self.frames.len() > 600 || self.frames.front().is_some_and(|t| now - t > 1.) {
            self.frames.pop_front();
        }
    }
    fn benchmark(&self) -> serde_json::Value {
        let frames = self.frames_seen.saturating_sub(120);
        let elapsed = self
            .warmup_end
            .zip(self.frames.back())
            .map_or(0., |(start, end)| end - start);
        serde_json::json!({"frame_windows":self.windows.report(),"p95_frame_ms_upper_bound":self.distribution.percentile(95),"p99_frame_ms_upper_bound":self.distribution.percentile(99),"max_frame_ms":self.distribution.maximum(),"warmup_frames":120,"measured_frames":frames,"elapsed_seconds":elapsed,"mean_fps":if elapsed>0. {frames as f64/elapsed} else {0.},"mean_frame_ms":if frames>0 {elapsed*1000./frames as f64} else {0.}})
    }
    fn fps(&self) -> f64 {
        let span = self
            .frames
            .back()
            .zip(self.frames.front())
            .map_or(0., |(end, start)| end - start);
        if span > 0. {
            self.frames.len().saturating_sub(1) as f64 / span
        } else {
            0.
        }
    }
}
#[derive(Resource)]
pub struct Hud {
    pub source: hl2_ui::hud::WeaponHud,
    textures: HashMap<usize, GpuTexture>,
    white: Option<(Handle<HudMaterial>, Handle<HudMaterial>)>,
    modulate: Option<Handle<HudMaterial>>,
    pool: Vec<(Entity, Handle<Mesh>)>,
    pub quads: usize,
    timing: FrameTiming,
    diagnostic_font: Option<std::sync::Arc<source_ui::bitmap_text::BitmapFont>>,
}
impl Hud {
    fn observe_frame(&mut self, now: f64, profile: bool) {
        self.timing.observe(now, profile);
    }
    pub fn report(&self) -> serde_json::Value {
        let captions = &self.source.captions;
        let state = captions.state.borrow();
        let captions = serde_json::json!({"dictionaries":captions.captions.dictionaries.iter().map(|d| d.len()).collect::<Vec<_>>(),
            "errors":captions.errors,"processed":state.processed,"queued":state.streams().collect::<Vec<_>>(),"recent":state.recent,"missing":state.missing});
        let report = serde_json::json!({"quads":self.quads,"pooled_meshes":self.pool.len(),"owned_textures":self.textures.len(),"viewport":[self.source.canvas.width(),self.source.canvas.height()], "frame":self.source.frame(), "panels":self.source.panel_rects(), "power":self.source.power_report(), "captions":captions});
        report
    }
    pub fn new(source: hl2_ui::hud::WeaponHud) -> Self {
        let diagnostic_font =
            source_ui::bitmap_text::BitmapFont::load(18.).map(std::sync::Arc::new);
        Self {
            source,
            textures: HashMap::new(),
            white: None,
            modulate: None,
            pool: vec![],
            quads: 0,
            timing: FrameTiming::default(),
            diagnostic_font,
        }
    }
    pub fn bitmap_font(&self) -> Option<std::sync::Arc<source_ui::bitmap_text::BitmapFont>> {
        self.diagnostic_font.clone()
    }
    pub fn frame_statistics(&self) -> (f64, f64, serde_json::Value) {
        (
            self.timing.fps(),
            self.timing.frame_ms,
            self.timing.benchmark(),
        )
    }
    fn material(
        &mut self,
        quad: &Quad,
        images: &mut Assets<Image>,
        materials: &mut Assets<HudMaterial>,
    ) -> Handle<HudMaterial> {
        fn upload(
            texture: &Texture2D,
            images: &mut Assets<Image>,
            materials: &mut Assets<HudMaterial>,
        ) -> (Handle<HudMaterial>, Handle<HudMaterial>) {
            let mut image = Image::new(
                Extent3d {
                    width: u32::from(texture.0.width),
                    height: u32::from(texture.0.height),
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                texture.0.rgba.clone(),
                TextureFormat::Rgba8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.sampler = ImageSampler::Descriptor(match texture.filter() {
                FilterMode::Nearest => ImageSamplerDescriptor::nearest(),
                FilterMode::Linear => ImageSamplerDescriptor::linear(),
            });
            let handle = images.add(image);
            (
                materials.add(HudMaterial {
                    texture: handle.clone(),
                    additive: false,
                    modulate: false,
                }),
                materials.add(HudMaterial {
                    texture: handle,
                    additive: true,
                    modulate: false,
                }),
            )
        }
        if quad.modulate {
            return self
                .modulate
                .get_or_insert_with(|| {
                    let white = Texture2D::from_rgba8(1, 1, &[255; 4]);
                    let mut image = Image::new(
                        Extent3d {
                            width: 1,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                        TextureDimension::D2,
                        white.0.rgba.clone(),
                        TextureFormat::Rgba8Unorm,
                        RenderAssetUsages::RENDER_WORLD,
                    );
                    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::nearest());
                    materials.add(HudMaterial {
                        texture: images.add(image),
                        additive: false,
                        modulate: true,
                    })
                })
                .clone();
        }
        let handles = if let Some(cpu) = &quad.texture {
            let texture = self.textures.entry(cpu.id()).or_insert_with(|| {
                let (normal, additive) = upload(cpu, images, materials);
                GpuTexture {
                    _cpu: cpu.clone(),
                    normal,
                    additive,
                }
            });
            (&texture.normal, &texture.additive)
        } else {
            let handles = self.white.get_or_insert_with(|| {
                upload(&Texture2D::from_rgba8(1, 1, &[255; 4]), images, materials)
            });
            (&handles.0, &handles.1)
        };
        if quad.additive {
            handles.1.clone()
        } else {
            handles.0.clone()
        }
    }
}
#[derive(Component)]
pub struct HudDraw;
type DrawQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut MeshMaterial2d<HudMaterial>,
        &'static mut Transform,
        &'static mut Visibility,
        Option<&'static mut Aabb>,
    ),
    With<HudDraw>,
>;
type HudAssets<'w> = (
    ResMut<'w, Assets<Image>>,
    ResMut<'w, Assets<HudMaterial>>,
    ResMut<'w, Assets<Mesh>>,
);
type HudResources<'w> = (
    ResMut<'w, crate::ui_host::Console>,
    Res<'w, Time<Real>>,
    ResMut<'w, crate::simulation::Simulation>,
    Res<'w, crate::Status>,
    Option<Res<'w, crate::performance::Performance>>,
);

#[allow(clippy::too_many_arguments)]
pub fn present(
    mut commands: Commands,
    mut hud: ResMut<Hud>,
    mut game: ResMut<crate::gameplay::Gameplay>,
    (mut ui, clock, mut sim, status, performance): HudResources,
    windows: Query<&Window, With<PrimaryWindow>>,
    video_viewport: Option<Res<crate::video::VideoViewport>>,
    (mut images, mut materials, mut meshes): HudAssets,
    mut draws: DrawQuery,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "hud");
    let Ok(window) = windows.single() else {
        return;
    };
    // The HUD draws into the game frame, so it is laid out in frame pixels.
    let (width, height) = video_viewport
        .as_ref()
        .map(|vp| (vp.target_resolution.x as f32, vp.target_resolution.y as f32))
        .unwrap_or_else(|| (window.width(), window.height()));
    hud.source.canvas.resize(width, height);
    hud.source.aspect = ui.source.config.video.hud_aspect;
    // Client order: queued ScreenFade/Damage messages, the fade over the 3D view, HUD.
    let time = game.scene.time;
    for fade in std::mem::take(&mut game.player_damage.fades)
        .into_iter()
        .chain(std::mem::take(&mut game.scene.screen_fades))
    {
        hud.source.screen_fade(fade, time);
    }
    let eye = glam::Vec3::from_array(sim.eye().to_array());
    for message in std::mem::take(&mut game.damage_messages) {
        hud.source.damage_message(
            message,
            eye,
            sim.yaw.to_degrees(),
            game.inventory.suit,
            game.inventory.health,
            time,
        );
    }
    hud.source.draw_fade(time);
    // While GameUI is up in a level (pause menu, console, dialogs) the engine hides the client's
    // HUD panel: native pause captures (hl2_complete oracle, 2026-10-29) show no HUD or crosshair.
    let hud_hidden = ui.source.paused() && !ui.source.background_level();
    // Closed captions (Options > Audio) also show behind the main menu, under GameUI.
    let captions = hl2_ui::closecaption::Settings::from_config(ui.source.config.audio.captions);
    let messages = std::mem::take(&mut game.caption_messages);
    hud.source.captions.emit(&messages, &captions, time);
    if !hud_hidden {
        hud.source.draw_captions(&captions, time);
    }
    // Background levels behind the main menu draw no HUD.
    if !ui.source.background_level() && !hud_hidden {
        game_hud::draw(&hud, &game, &ui);
        draw_ai_disabled(&hud, &game, width, height);
    }
    hud.observe_frame(clock.elapsed_secs_f64(), performance.is_some());
    ui.source.draw(clock.elapsed_secs_f64());
    sim.with_extensions(|extension, simulation| {
        extension.present(
            &mut crate::extensions::Context {
                simulation,
                gameplay: Some(&mut game),
                console: Some(&mut ui),
            },
            &mut hud,
            &status,
        )
    });
    game.scene
        .sounds
        .extend(hud.source.drain_sounds().into_iter().map(Into::into));
    let quads = hud.source.canvas.drain();
    hud.quads = quads.len();
    for (i, quad) in quads.iter().enumerate() {
        let material = hud.material(quad, &mut images, &mut materials);
        let mesh = quad_mesh(quad, Vec2::new(width, height));
        if let Some((entity, handle)) = hud.pool.get(i) {
            if let Some(mut old) = meshes.get_mut(handle) {
                *old = mesh;
            }
            if let Ok((mut old_material, mut transform, mut visibility, aabb)) =
                draws.get_mut(*entity)
            {
                old_material.0 = material;
                transform.translation.z = i as f32 * 0.01;
                *visibility = Visibility::Inherited;
                if let (Some(mut aabb), Some(mesh)) = (aabb, meshes.get(handle))
                    && let Some(bounds) = mesh.compute_aabb()
                {
                    *aabb = bounds;
                }
            }
        } else {
            let mesh = meshes.add(mesh);
            let entity = commands
                .spawn((
                    crate::campaign::MapOwned,
                    HudDraw,
                    Mesh2d(mesh.clone()),
                    MeshMaterial2d(material),
                    Transform::from_xyz(0., 0., i as f32 * 0.01),
                    RenderLayers::layer(2),
                ))
                .id();
            hud.pool.push((entity, mesh));
        }
    }
    for (entity, _) in hud.pool.iter().skip(quads.len()) {
        if let Ok((_, _, mut visibility, _)) = draws.get_mut(*entity) {
            *visibility = Visibility::Hidden;
        }
    }
}
fn quad_mesh(quad: &Quad, viewport: Vec2) -> Mesh {
    let r = quad.destination;
    let corners = quad.vertices.map_or(
        [
            (r.x, r.y),
            (r.x, r.y + r.h),
            (r.x + r.w, r.y + r.h),
            (r.x + r.w, r.y),
        ],
        |v| v.map(|(p, _)| (p.x, p.y)),
    );
    let positions = corners.map(|(x, y)| [x - viewport.x * 0.5, viewport.y * 0.5 - y, 0.]);
    let colors = quad
        .vertices
        .map_or([quad.color.to_array(); 4], |v| v.map(|(_, c)| c.to_array()));
    let (tw, th) = quad
        .texture
        .as_ref()
        .map_or((1., 1.), |t| (t.width(), t.height()));
    let s = quad.source;
    let uvs = [
        (s.x, s.y),
        (s.x, s.y + s.h),
        (s.x + s.w, s.y + s.h),
        (s.x + s.w, s.y),
    ]
    .map(|(x, y)| [x / tw, y / th]);
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors.to_vec());
    mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
    mesh
}

fn draw_ai_disabled(hud: &Hud, game: &crate::gameplay::Gameplay, width: f32, height: f32) {
    // CAI_BaseNPC::PreThink: "A.I. Disabled..." at (0.7, 0.65) while ai_disable is on
    // (HUD message font not reproduced: the developer font).
    if ai_disabled_message(&game.ai)
        && let Some(font) = &hud.diagnostic_font
    {
        font.text(
            &hud.source.canvas,
            "A.I. Disabled...",
            width * 0.7,
            height * 0.65,
            source_ui::canvas::WHITE,
        );
    }
}

/// Source sends the message from an NPC's think, so a world without NPCs never shows it
/// (the startup placeholder's AI is off only because no map has set it yet).
fn ai_disabled_message(ai: &hl2_simulation::ai::runtime::AiRuntime) -> bool {
    !ai.enabled && !ai.actors.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_ui::canvas::{Color, Rect};
    #[test]
    fn a_world_without_npcs_shows_no_ai_disabled_message() {
        let game = crate::gameplay::Gameplay::synthetic(modkit_core::World::default());
        assert!(!game.ai.enabled);
        assert!(!ai_disabled_message(&game.ai));
    }
    #[test]
    fn frame_profile_excludes_warmup_and_keeps_tail_bounds() {
        let mut timing = FrameTiming::default();
        for frame in 0..120 {
            timing.observe(f64::from(frame), true);
        }
        assert!(timing.benchmark()["max_frame_ms"].is_null());
        timing.observe(119.125, true);
        let result = timing.benchmark();
        assert_eq!(result["measured_frames"], 1);
        assert_eq!(result["max_frame_ms"], 125.);
        assert_eq!(result["p95_frame_ms_upper_bound"], 126.);
        assert_eq!(result["p99_frame_ms_upper_bound"], 126.);
        assert_eq!(result["mean_frame_ms"], 125.);
    }

    #[test]
    fn ordinary_frames_do_not_collect_profile_histograms() {
        let mut timing = FrameTiming::default();
        for frame in 0..125 {
            timing.observe(f64::from(frame) * 0.125, false);
        }
        let result = timing.benchmark();
        assert_eq!(result["measured_frames"], 5);
        assert!(
            result["frame_windows"]["windows"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(result["max_frame_ms"].is_null());
        assert!(result["p95_frame_ms_upper_bound"].is_null());
        assert!(result["p99_frame_ms_upper_bound"].is_null());
    }

    #[test]
    fn atlas_crop_remains_pixel_aligned_in_camera_coordinates() {
        let quad = Quad {
            texture: Some(Texture2D::from_rgba8(128, 128, &vec![255; 128 * 128 * 4])),
            source: Rect::new(0.5, 48.5, 23., 23.),
            destination: Rect::new(948., 528., 24., 24.),
            color: Color::from_rgba(255, 255, 255, 255),
            additive: true,
            modulate: false,
            vertices: None,
        };
        let mesh = quad_mesh(&quad, Vec2::new(1920., 1080.));
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("HUD positions");
        };
        assert_eq!(
            positions,
            &[
                [-12., 12., 0.],
                [-12., -12., 0.],
                [12., -12., 0.],
                [12., 12., 0.]
            ]
        );
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) =
            mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!("HUD UVs");
        };
        assert_eq!(uvs[0], [0.5 / 128., 48.5 / 128.]);
        assert_eq!(uvs[2], [23.5 / 128., 71.5 / 128.]);
    }

    #[test]
    fn backdrop_quad_mesh_geometry() {
        let quad = Quad {
            texture: None,
            source: Rect::new(0., 0., 1., 1.),
            destination: Rect::new(0., 0., 1920., 1080.),
            color: Color::from_rgba(0, 0, 0, 156),
            additive: false,
            modulate: false,
            vertices: None,
        };
        let mesh = quad_mesh(&quad, Vec2::new(1920., 1080.));
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("HUD positions");
        };
        assert_eq!(
            positions,
            &[
                [-960., 540., 0.],
                [-960., -540., 0.],
                [960., -540., 0.],
                [960., 540., 0.]
            ]
        );
        let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) =
            mesh.attribute(Mesh::ATTRIBUTE_COLOR)
        else {
            panic!("HUD colors");
        };
        assert_eq!(colors[0], [0., 0., 0., 156.0 / 255.0]);
    }
}
