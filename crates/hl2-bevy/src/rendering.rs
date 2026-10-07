//! Baked-lightmap material, independent of Bevy's PBR lighting model.
use crate::{
    Status,
    assets::{LoadedMap, MaterialData, visible_entity},
    source_to_bevy,
};
use bevy::{
    asset::RenderAssetUsages,
    camera::primitives::{Aabb, MeshAabb},
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, MeshVertexBufferLayoutRef},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, Extent3d, FrontFace, PrimitiveTopology, RenderPipelineDescriptor, ShaderType,
        SpecializedMeshPipelineError, TextureDimension, TextureFormat, TextureViewDescriptor,
        TextureViewDimension,
    },
    shader::ShaderRef,
};
use modkit_core::Surface;
use std::collections::BTreeMap;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(MaterialKey)]
pub struct SourceMaterial {
    #[uniform(0)]
    pub(crate) tint: Vec4,
    // x = alpha cutoff, y = ignore base alpha, z = additive output.
    #[uniform(3)]
    pub(crate) parameters: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub(crate) base: Handle<Image>,
    #[texture(4)]
    #[sampler(5)]
    pub(crate) lightmap: Handle<Image>,
    #[texture(6)]
    #[sampler(7)]
    pub(crate) iris: Handle<Image>,
    #[uniform(8)]
    pub(crate) secondary_uv: Mat3,
    #[uniform(9)]
    pub(crate) lighting: ModelLighting,
    /// LightmappedGeneric $envmap; a black fallback cube when absent.
    #[texture(10, dimension = "cube")]
    #[sampler(11)]
    pub(crate) envmap: Handle<Image>,
    /// xyz = $envmaptint, w = 1 when an envmap is bound.
    #[uniform(12)]
    pub(crate) envmap_tint: Vec4,
    /// xyz = $envmapcontrast, w = $fresnelreflection.
    #[uniform(13)]
    pub(crate) envmap_contrast: Vec4,
    /// xyz = $envmapsaturation, w = $basealphaenvmapmask.
    #[uniform(14)]
    pub(crate) envmap_saturation: Vec4,
    pub(crate) alpha: AlphaMode,
    pub(crate) two_sided: bool,
}
/// Fallback for materials without an envmap (never sampled; envmap_tint.w is 0). Held by a
/// strong handle like the white fallback: a UUID handle does not keep the image alive, and
/// materials re-prepared later (per-frame model lighting) would then fail to bind.
pub(crate) fn black_cube() -> Image {
    cube_image(1, false, vec![0; 6 * 4])
}
/// A six-layer cube texture: RGBA16F linear halves (HDR) or sRGB-encoded RGBA8 (LDR).
pub(crate) fn cube_image(size: u16, hdr: bool, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: size.into(),
            height: size.into(),
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        if hdr {
            TextureFormat::Rgba16Float
        } else {
            TextureFormat::Rgba8UnormSrgb
        },
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}
/// Source model lighting for one entity: ambient cube plus up to four local
/// lights, positions/directions in Bevy space and colors linear.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct ModelLighting {
    /// x = lit model, y = local light count, z = half-Lambert.
    pub params: Vec4,
    /// Draw space to Bevy world space (Mat4 default is identity), except for the
    /// view model, which is drawn by a fixed camera at the origin.
    pub basis: Mat4,
    /// Source cube faces +X, -X, +Y, -Y, +Z, -Z.
    pub ambient: [Vec4; 6],
    /// w = 0 point, 1 spot, 2 directional.
    pub position: [Vec4; 4],
    pub color: [Vec4; 4],
    pub direction: [Vec4; 4],
    /// Constant, linear, quadratic.
    pub attenuation: [Vec4; 4],
    /// Exponent, outer cosine, 1 / (inner - outer).
    pub spot: [Vec4; 4],
}
impl ModelLighting {
    /// Shared CPU light state converted for the shader.
    pub fn from_state(state: &modkit_core::lighting::LightState, half_lambert: bool) -> Self {
        use modkit_core::lighting::ShaderLightKind;
        let mut out = Self {
            params: Vec4::new(
                1.,
                state.lights.len().min(4) as f32,
                f32::from(half_lambert),
                0.,
            ),
            ..Default::default()
        };
        for (face, color) in out.ambient.iter_mut().zip(&state.ambient) {
            *face = Vec3::from_array(color.to_array()).extend(0.);
        }
        for (i, light) in state.lights.iter().take(4).enumerate() {
            let kind = match light.kind {
                ShaderLightKind::Point => 0.,
                ShaderLightKind::Spot => 1.,
                ShaderLightKind::Directional => 2.,
            };
            out.position[i] =
                source_to_bevy(Vec3::from_array(light.position.to_array())).extend(kind);
            out.color[i] = Vec3::from_array(light.color.to_array()).extend(0.);
            out.direction[i] =
                source_to_bevy(Vec3::from_array(light.direction.to_array())).extend(0.);
            out.attenuation[i] = Vec3::from_array(light.attenuation).extend(0.);
            let scale = if light.inner > light.outer {
                1. / (light.inner - light.outer)
            } else {
                1.
            };
            out.spot[i] = Vec4::new(light.exponent, light.outer, scale, 0.);
        }
        out
    }
    /// Unlit-data fallback: the former constant 180/255 gamma gray.
    fn fallback(half_lambert: bool) -> Self {
        let gray = (180f32 / 255.).powf(2.2);
        Self {
            params: Vec4::new(1., 0., f32::from(half_lambert), 0.),
            ambient: [Vec4::new(gray, gray, gray, 0.); 6],
            ..Default::default()
        }
    }
}
/// A model draw lit from its entity's (or the view's) illumination origin.
#[derive(Component)]
pub struct ModelLit {
    entity: Option<usize>,
    key: String,
    scale: f32,
    half_lambert: bool,
    last: Option<(glam::Vec3, bool)>,
}
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct MaterialKey {
    two_sided: bool,
}
impl From<&SourceMaterial> for MaterialKey {
    fn from(material: &SourceMaterial) -> Self {
        Self {
            two_sided: material.two_sided,
        }
    }
}
impl Material for SourceMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/source.wgsl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        self.alpha
    }
    fn specialize(
        _: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // BSP render triangles are normalized before model append; vmdl already
        // supplies CCW model triangles. Both now use Bevy's CCW front face.
        descriptor.primitive.front_face = FrontFace::Ccw;
        if key.bind_group_data.two_sided {
            descriptor.primitive.cull_mode = None;
        }
        Ok(())
    }
}
pub(crate) fn image(width: u16, height: u16, rgba: Vec<u8>, repeat: bool) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: width.into(),
            height: height.into(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: if repeat {
            ImageAddressMode::Repeat
        } else {
            ImageAddressMode::ClampToEdge
        },
        address_mode_v: if repeat {
            ImageAddressMode::Repeat
        } else {
            ImageAddressMode::ClampToEdge
        },
        ..ImageSamplerDescriptor::linear()
    });
    image
}
#[derive(Default)]
struct Batch {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    light_uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    /// Source-space unit normals.
    normals: Vec<glam::Vec3>,
    /// Studio-model surfaces (lit per entity rather than by lightmap).
    model: bool,
    indices: Vec<u32>,
    skin: Vec<(glam::Vec3, Option<modkit_core::animation::Weights>)>,
    /// Studio (bodypart, mesh, mesh-local vertex) of each vertex, for facial flexes.
    flex: Vec<Option<[u16; 3]>>,
}
impl Batch {
    /// Group whole triangles, never cut them at cell boundaries. Actual mesh
    /// bounds (including spanning triangles) remain the culling bounds.
    fn spatial_chunks(self) -> Vec<Self> {
        const CELL: f64 = 1024.;
        const MAX_CHUNKS: usize = 64;
        if !self.indices.len().is_multiple_of(3)
            || self.positions.iter().flatten().any(|v| !v.is_finite())
            || self.uvs.len() != self.positions.len()
            || self.light_uvs.len() != self.positions.len()
            || self.colors.len() != self.positions.len()
            || self.normals.len() != self.positions.len()
            || self.skin.len() != self.positions.len()
        {
            return vec![self];
        }
        let mut groups = BTreeMap::<[i32; 3], Vec<u32>>::new();
        for triangle in self.indices.as_chunks::<3>().0 {
            let [Some(a), Some(b), Some(c)] = triangle.map(|i| self.positions.get(i as usize))
            else {
                return vec![self];
            };
            let vertices = [a, b, c];
            let cell = std::array::from_fn(|axis| {
                ((vertices.iter().map(|p| f64::from(p[axis])).sum::<f64>() / 3.) / CELL).floor()
                    as i32
            });
            groups.entry(cell).or_default().extend_from_slice(triangle);
            if groups.len() > MAX_CHUNKS {
                return vec![self];
            }
        }
        if groups.len() <= 1 {
            return vec![self];
        }
        groups
            .into_values()
            .map(|indices| {
                let mut batch = Self::default();
                let mut remap = std::collections::HashMap::new();
                for index in indices {
                    let new = *remap.entry(index).or_insert_with(|| {
                        let new = batch.positions.len() as u32;
                        let index = index as usize;
                        batch.positions.push(self.positions[index]);
                        batch.uvs.push(self.uvs[index]);
                        batch.light_uvs.push(self.light_uvs[index]);
                        batch.colors.push(self.colors[index]);
                        batch.normals.push(self.normals[index]);
                        batch.skin.push(self.skin[index].clone());
                        new
                    });
                    batch.indices.push(new);
                }
                batch
            })
            .collect()
    }
    fn append(&mut self, surface: &Surface, transform: Mat4, material: Option<&MaterialData>) {
        let offset = self.positions.len() as u32;
        self.model |= surface.flex_source.is_some();
        let rotation = Mat3::from_mat4(transform);
        let rows = material
            .map(|m| m.uv_transform)
            .unwrap_or([[1., 0., 0.], [0., 1., 0.]]);
        for (i, vertex) in surface.vertices.iter().enumerate() {
            self.skin.push((vertex.position, vertex.skin.clone()));
            self.flex.push(surface.flex_source.as_ref().and_then(|f| {
                Some([
                    u16::try_from(f.bodypart).ok()?,
                    u16::try_from(f.mesh).ok()?,
                    *f.vertex_ids.get(i)?,
                ])
            }));
            self.positions.push(
                source_to_bevy(
                    transform.transform_point3(Vec3::from_array(vertex.position.to_array())),
                )
                .to_array(),
            );
            let uv = vertex.uv;
            self.uvs.push([
                rows[0][0] * uv.x + rows[0][1] * uv.y + rows[0][2],
                rows[1][0] * uv.x + rows[1][1] * uv.y + rows[1][2],
            ]);
            self.light_uvs.push(vertex.light_uv.to_array());
            self.colors.push(vertex.color.map(|v| f32::from(v) / 255.));
            let normal = glam::Vec3::from_array(vertex.normal.to_array());
            self.normals.push(
                glam::Mat3::from_cols_array(&rotation.to_cols_array())
                    .mul_vec3(normal)
                    .normalize_or_zero(),
            );
        }
        self.indices
            .extend(surface.indices.iter().map(|i| i + offset));
    }
    fn mesh(self) -> Mesh {
        let count = self.positions.len();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            if self.skin.iter().any(|(_, weights)| weights.is_some()) {
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD
            } else {
                RenderAssetUsages::RENDER_WORLD
            },
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            self.normals
                .iter()
                .map(|n| {
                    let n = source_to_bevy(Vec3::from_array(n.to_array()));
                    if n == Vec3::ZERO {
                        [0., 1., 0.]
                    } else {
                        n.to_array()
                    }
                })
                .collect::<Vec<_>>(),
        );
        debug_assert_eq!(self.normals.len(), count);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.light_uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}
#[derive(Component)]
pub struct SourceEntity(pub usize);
#[derive(Component)]
pub struct DrawTriangles(pub usize);
#[derive(Component)]
pub struct EyeMesh {
    pub key: String,
    pub eye: source_assets::eyes::Eyeball,
    pub scale: f32,
}
fn sample_key(clip: &modkit_core::animation::Clip, time: f32) -> u32 {
    if clip.frames.len() <= 1 {
        0
    } else if !clip.looping && time * clip.fps >= (clip.frames.len() - 1) as f32 {
        u32::MAX
    } else {
        time.to_bits()
    }
}
#[derive(Component)]
pub struct AnimatedMesh {
    sampled: Option<(String, u32, u64)>,
    gpu: Option<crate::gpu_skinning::Skeleton>,
    entity: Option<usize>,
    weapon: Option<String>,
    key: String,
    scale: f32,
    bind: Vec<(glam::Vec3, Option<modkit_core::animation::Weights>)>,
    /// Source-space bind normals for the CPU skinning path.
    normals: Vec<glam::Vec3>,
    /// Facial flex vertex references and the undeformed bind positions they start from.
    flex: Vec<Option<[u16; 3]>>,
    flex_base: Vec<glam::Vec3>,
    flex_signature: u64,
}
/// Facial flex and eyeball data per model asset key (setup-loaded; no frame IO).
#[derive(Resource, Default)]
pub struct FlexModels(pub BTreeMap<String, std::sync::Arc<source_assets::flexes::FaceModel>>);
/// Descriptor weights quantized to 1/1024: deformation is rebuilt only when it changes.
fn flex_signature(weights: &[f32]) -> u64 {
    weights.iter().fold(0x9e37u64, |hash, w| {
        (hash ^ (w * 1024.).round() as i64 as u64).wrapping_mul(0x100000001b3)
    })
}
/// Conjugate rotation by the same basis change used for vertex positions.
pub(crate) fn entity_transform(origin: glam::Vec3, rotation: glam::Quat) -> Transform {
    let basis = Mat3::from_cols(
        source_to_bevy(Vec3::X),
        source_to_bevy(Vec3::Y),
        source_to_bevy(Vec3::Z),
    );
    Transform::from_translation(source_to_bevy(Vec3::from_array(origin.to_array()))).with_rotation(
        Quat::from_mat3(
            &(basis * Mat3::from_quat(Quat::from_array(rotation.to_array())) * basis.transpose()),
        ),
    )
}
#[allow(clippy::too_many_arguments)]
pub fn present_entities(
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::movement::Simulation>,
    mut entities: Query<
        (&SourceEntity, &mut Transform, &mut Visibility),
        Without<crate::gpu_skinning::Joint>,
    >,
    mut joints: Query<&mut Transform, (With<crate::gpu_skinning::Joint>, Without<SourceEntity>)>,
    mut animations: Query<(&mut AnimatedMesh, &Mesh3d, Option<&mut Aabb>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    performance: Option<Res<crate::performance::Performance>>,
    flex_models: Option<Res<FlexModels>>,
    eyes: Option<Res<crate::eyes::Eyes>>,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "animation");
    for (owner, mut transform, mut visibility) in &mut entities {
        let state = &game.scene.states[owner.0];
        let (origin, rotation) = sim
            .physics
            .entity_pose(owner.0)
            .unwrap_or((state.origin, state.rotation));
        let pose = entity_transform(origin, rotation);
        if *transform != pose {
            *transform = pose;
        }
        let visible = if state.visible && !state.killed {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != visible {
            *visibility = visible;
        }
    }
    let mut updated_joints = std::collections::BTreeSet::new();
    let mut poses: BTreeMap<(String, String, u32, u64), Vec<glam::Mat4>> = BTreeMap::new();
    for (mut animation, handle, aabb) in &mut animations {
        let Some(rig) = game.world.rigs.get(&animation.key) else {
            continue;
        };
        // Facial flexes deform the bind positions before skinning (both GPU and CPU paths).
        if let (Some(id), Some(model)) = (
            animation.entity,
            flex_models
                .as_deref()
                .and_then(|f| f.0.get(&animation.key))
                .filter(|_| !animation.flex.is_empty()),
        ) {
            let values = game.scene.actor_flex_values(id);
            let weights = model.descriptor_weights(&values, |eye| {
                eyes.as_deref()
                    .and_then(|e| e.lid_bases.get(&(id, eye.surface)))
                    .copied()
            });
            let signature = flex_signature(&weights);
            if signature != animation.flex_signature {
                let deltas = model.flex.vertex_deltas(&weights);
                let scale = animation.scale;
                let mut changed = Vec::new();
                let anim = &mut *animation;
                for (i, key) in anim.flex.iter().enumerate() {
                    let Some(key) = key else {
                        continue;
                    };
                    let target =
                        anim.flex_base[i] + deltas.get(key).copied().unwrap_or(glam::Vec3::ZERO);
                    if anim.bind[i].0 != target {
                        anim.bind[i].0 = target;
                        changed.push(i);
                    }
                }
                anim.flex_signature = signature;
                if !changed.is_empty() {
                    if anim.gpu.is_some() {
                        if let Some(mut mesh) = meshes.get_mut(&handle.0)
                            && let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                                mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
                        {
                            for &i in &changed {
                                if let Some(p) = positions.get_mut(i) {
                                    *p = source_to_bevy(Vec3::from_array(
                                        (anim.bind[i].0 * scale).to_array(),
                                    ))
                                    .to_array();
                                }
                            }
                        }
                    } else {
                        // Force the CPU skinning path to rebuild from the deformed bind.
                        anim.sampled = None;
                    }
                }
            }
        }
        let (clip, time) = if let Some(id) = animation.entity {
            let state = &game.scene.states[id];
            if !state.visible || state.killed {
                continue;
            }
            (state.animation.as_str(), game.scene.animation_time(id))
        } else {
            if animation.weapon.as_deref() != Some(game.inventory.active.as_str()) {
                continue;
            }
            let elapsed = (game.scene.time - game.inventory.animation_at).max(0.) as f32;
            if rig
                .clips
                .get(&game.inventory.animation)
                .is_some_and(|c| elapsed < c.duration())
            {
                (game.inventory.animation.as_str(), elapsed)
            } else {
                (game.inventory.idle_animation(), game.scene.time as f32)
            }
        };
        let Some(definition) = rig.clips.get(clip) else {
            continue;
        };
        let key = sample_key(definition, time);
        // Gesture layers make the pose actor-specific; the entity id keeps cache entries apart.
        let layers = animation
            .entity
            .map_or(0, |id| game.scene.pose_signature(id));
        let layer_key = match (layers, animation.entity) {
            (0, _) | (_, None) => 0,
            (signature, Some(id)) => signature ^ (id as u64).rotate_left(32),
        };
        if animation
            .sampled
            .as_ref()
            .is_some_and(|(name, previous, layered)| {
                name == clip && *previous == key && *layered == layer_key
            })
        {
            continue;
        }
        let sampled = (clip.to_owned(), key, layer_key);
        let matrices = poses
            .entry((animation.key.clone(), clip.to_owned(), key, layer_key))
            .or_insert_with(|| match animation.entity {
                Some(id) => game.scene.actor_matrices(rig, id),
                None => rig.matrices(clip, time),
            });
        if let Some(skeleton) = &animation.gpu {
            if updated_joints.insert(skeleton.joints[0]) {
                for ((joint, matrix), bone) in
                    skeleton.joints.iter().zip(matrices.iter()).zip(&rig.bones)
                {
                    if let Ok(mut transform) = joints.get_mut(*joint) {
                        let pose = Transform::from_matrix(crate::gpu_skinning::convert(
                            *matrix * bone.inverse_bind.inverse(),
                            animation.scale,
                        ));
                        if *transform != pose {
                            *transform = pose;
                        }
                    }
                }
            }
            animation.sampled = Some(sampled);
            continue;
        }
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            let positions: Vec<_> = animation
                .bind
                .iter()
                .map(|(bind, weights)| {
                    let p = weights
                        .as_ref()
                        .map_or(*bind, |w| modkit_core::animation::skin(*bind, w, matrices));
                    source_to_bevy(Vec3::from_array((p * animation.scale).to_array())).to_array()
                })
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            if animation.normals.len() == animation.bind.len() {
                let normals: Vec<_> = animation
                    .bind
                    .iter()
                    .zip(&animation.normals)
                    .map(|((_, weights), normal)| {
                        let n = weights.as_ref().map_or(*normal, |w| {
                            skin_normal(*normal, w, matrices).normalize_or_zero()
                        });
                        source_to_bevy(Vec3::from_array(n.to_array())).to_array()
                    })
                    .collect();
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            }
            if let (Some(mut aabb), Some(bounds)) = (aabb, mesh.compute_aabb()) {
                *aabb = bounds;
            }
            animation.sampled = Some(sampled);
        }
    }
}
/// Recompute each lit model's light state when its illumination origin moves
/// (retail light cache at origin + rotation * illumposition * scale).
pub fn present_lighting(
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::movement::Simulation>,
    mut draws: Query<(
        &mut ModelLit,
        &MeshMaterial3d<SourceMaterial>,
        &InheritedVisibility,
    )>,
    mut materials: ResMut<Assets<SourceMaterial>>,
    performance: Option<Res<crate::performance::Performance>>,
) {
    let _timing = crate::performance::scope(performance.as_deref(), "model lighting");
    let Some(data) = game.world.lighting.as_deref() else {
        return;
    };
    let mut states =
        BTreeMap::<Option<usize>, (glam::Vec3, modkit_core::lighting::LightState)>::new();
    // The view model camera sits at the origin looking down +X; map that space onto the
    // player's view so world-space lights and the ambient cube line up.
    let view = Transform::from_translation(source_to_bevy(sim.eye()))
        .looking_to(
            source_to_bevy(crate::source_direction(sim.yaw, sim.pitch)),
            Vec3::Y,
        )
        .to_matrix()
        * Transform::IDENTITY
            .looking_to(Vec3::X, Vec3::Y)
            .to_matrix()
            .inverse();
    for (mut lit, material, visible) in &mut draws {
        if !visible.get() && lit.last.is_some() {
            continue;
        }
        let illumination = game
            .world
            .illumination
            .get(&lit.key)
            .copied()
            .unwrap_or_default();
        let origin = match lit.entity {
            Some(id) => {
                let state = &game.scene.states[id];
                let (origin, rotation) = sim
                    .physics
                    .entity_pose(id)
                    .unwrap_or((state.origin, state.rotation));
                origin + rotation * (illumination.position * lit.scale)
            }
            None => glam::Vec3::from_array(sim.eye().to_array()),
        };
        if lit.entity.is_some()
            && lit.last.is_some_and(|(last, hl)| {
                last.distance_squared(origin) < 1. && hl == lit.half_lambert
            })
        {
            continue;
        }
        let (_, state) = states.entry(lit.entity).or_insert_with(|| {
            let mut state = data.state_at(origin);
            if illumination.ambient_boost() {
                data.boost(&mut state, origin);
            }
            (origin, state)
        });
        let mut uniform = ModelLighting::from_state(state, lit.half_lambert);
        if lit.entity.is_none() {
            uniform.basis = view;
        }
        if let Some(mut material) = materials.get_mut(&material.0)
            && material.lighting != uniform
        {
            material.lighting = uniform;
        }
        lit.last = Some((origin, lit.half_lambert));
    }
}
/// Rotate a bind normal by the weighted bone matrices (CPU skinning path).
fn skin_normal(
    normal: glam::Vec3,
    weights: &modkit_core::animation::Weights,
    matrices: &[glam::Mat4],
) -> glam::Vec3 {
    let mut out = glam::Vec3::ZERO;
    let mut total = 0.;
    for (&bone, &weight) in weights.bones.iter().zip(&weights.weights) {
        if weight > 0.
            && let Some(m) = matrices.get(usize::from(bone))
        {
            out += m.transform_vector3(normal) * weight;
            total += weight;
        }
    }
    if total > 0. { out } else { normal }
}
#[derive(Component)]
pub struct WeaponMesh(String);
pub fn present_weapons(
    game: Res<crate::gameplay::Gameplay>,
    mut draws: Query<(&WeaponMesh, &mut Visibility)>,
) {
    for (weapon, mut visibility) in &mut draws {
        *visibility = if weapon.0 == game.inventory.active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Owner {
    World,
    SkyWorld,
    Entity(usize),
    Weapon(String),
}
impl Owner {
    fn entity(&self) -> Option<usize> {
        if let Self::Entity(id) = self {
            Some(*id)
        } else {
            None
        }
    }
}

pub fn spawn_map(
    loaded: &LoadedMap,
    commands: &mut Commands,
    (meshes, inverse_binds): (
        &mut Assets<Mesh>,
        &mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    ),
    materials: &mut Assets<SourceMaterial>,
    images: &mut Assets<Image>,
    status: &Status,
    (camera_target, cpu_skinning, world_partition): (&Handle<Image>, bool, bool),
) {
    let world = &loaded.world;
    let white = images.add(image(1, 1, vec![255; 4], false));
    let black_cube = images.add(black_cube());
    let missing = images.add(image(
        2,
        2,
        vec![
            255, 0, 255, 255, 30, 30, 30, 255, 30, 30, 30, 255, 255, 0, 255, 255,
        ],
        true,
    ));
    let mut texture_handles: BTreeMap<String, Handle<Image>> = BTreeMap::new();
    let bases: BTreeMap<_, _> = loaded
        .materials
        .iter()
        .map(|(name, material)| {
            let handle = if material.camera {
                camera_target.clone()
            } else {
                match (&material.base, &material.base_path) {
                    (Some(base), Some(path)) => texture_handles
                        .entry(path.clone())
                        .or_insert_with(|| {
                            images.add(image(base.width, base.height, base.rgba.clone(), true))
                        })
                        .clone(),
                    _ => missing.clone(),
                }
            };
            (name.clone(), handle)
        })
        .collect();
    let irises: BTreeMap<_, _> = loaded
        .materials
        .iter()
        .filter_map(|(name, m)| {
            m.iris
                .as_ref()
                .zip(m.iris_path.as_ref())
                .map(|(image, path)| {
                    let handle = texture_handles
                        .entry(path.clone())
                        .or_insert_with(|| {
                            images.add(self::image(
                                image.width,
                                image.height,
                                image.rgba.clone(),
                                false,
                            ))
                        })
                        .clone();
                    (name.clone(), handle)
                })
        })
        .collect();
    let overlays: BTreeMap<_, _> = loaded
        .materials
        .iter()
        .filter_map(|(name, m)| {
            m.camera_overlay
                .as_ref()
                .zip(m.camera_overlay_path.as_ref())
                .map(|(overlay, path)| {
                    // UnlitTwoTexture reads $texture2 through an sRGB view (SDK
                    // EnableSRGBRead), so the camera branch multiplies linear colors.
                    let handle = texture_handles
                        .entry(format!("{path}#srgb"))
                        .or_insert_with(|| {
                            let mut overlay =
                                image(overlay.width, overlay.height, overlay.rgba.clone(), true);
                            overlay.texture_descriptor.format = TextureFormat::Rgba8UnormSrgb;
                            images.add(overlay)
                        })
                        .clone();
                    (name.clone(), handle)
                })
        })
        .collect();
    let envmaps: BTreeMap<_, _> = loaded
        .materials
        .iter()
        .filter_map(|(name, m)| {
            let (cube, path) = m.envmap.as_ref().zip(m.envmap_path.as_ref())?;
            let handle = texture_handles
                .entry(format!("{path}#cube"))
                .or_insert_with(|| images.add(cube_image(cube.size, cube.hdr, cube.faces.concat())))
                .clone();
            Some((name.clone(), handle))
        })
        .collect();
    let lightmaps: Vec<_> = world
        .lightmaps
        .iter()
        .map(|lm| {
            let mut lightmap = image(
                lm.width,
                lm.height,
                lm.rgba.iter().flat_map(|t| t.to_le_bytes()).collect(),
                false,
            );
            lightmap.texture_descriptor.format = TextureFormat::Rgba16Float;
            images.add(lightmap)
        })
        .collect();
    let mut batches: BTreeMap<(Owner, String, Option<usize>, usize), Batch> = BTreeMap::new();
    let skipped = 0usize;
    let mut transparent_id = 0usize;
    let mut append = |surface: &Surface, transform: Mat4, owner: Owner| {
        let owner = if surface.background && owner == Owner::World {
            Owner::SkyWorld
        } else {
            owner
        };
        if surface.indices.is_empty() {
            return;
        }
        let definition = loaded.materials.get(&surface.material);
        let draw_id = if definition
            .is_some_and(|m| matches!(alpha_mode(m), AlphaMode::Blend | AlphaMode::Add))
        {
            transparent_id += 1;
            transparent_id
        } else {
            0
        };
        batches
            .entry((owner, surface.material.clone(), surface.lightmap, draw_id))
            .or_default()
            .append(surface, transform, loaded.materials.get(&surface.material));
    };
    // BSP::world includes displacements here; terrain is the collision copy.
    for surface in &world.surfaces {
        append(surface, Mat4::IDENTITY, Owner::World);
    }
    for (id, entity) in world.entities.iter().enumerate() {
        if !visible_entity(world, id) {
            continue;
        }
        let brush = entity
            .get("model")
            .and_then(|s| s.strip_prefix('*'))
            .and_then(|s| s.parse::<usize>().ok())
            .and_then(|id| world.brush_models.iter().find(|model| model.id == id));
        if let Some(brush) = brush {
            for surface in &brush.surfaces {
                append(surface, Mat4::IDENTITY, Owner::Entity(id));
            }
        }
    }
    for instance in &world.model_instances {
        // append_models already baked entity=None static props into surfaces.
        if instance.entity.is_none_or(|id| !visible_entity(world, id)) {
            continue;
        }
        if let Some(surfaces) = world.model_assets.get(&instance.asset_key()) {
            let transform = Mat4::from_scale(Vec3::splat(instance.scale));
            for surface in surfaces {
                append(surface, transform, Owner::Entity(instance.entity.unwrap()));
            }
        }
    }
    for (name, weapon) in &loaded.gameplay.weapons {
        if let Some(surfaces) = world
            .model_assets
            .get(&format!("{}#0", weapon.viewmodel.to_lowercase()))
        {
            for surface in surfaces {
                append(surface, Mat4::IDENTITY, Owner::Weapon(name.clone()));
            }
        }
    }
    let mut stats = status.0.lock().expect("status lock");
    stats.skipped_background_surfaces = skipped;
    stats.world_partition = world_partition;
    let batches = batches
        .into_iter()
        .flat_map(|(key, batch)| {
            let partition = world_partition
                && key.0 == Owner::World
                && loaded.materials.get(&key.1).is_none_or(|m| {
                    matches!(alpha_mode(m), AlphaMode::Opaque | AlphaMode::Mask(_))
                });
            let chunks = if partition {
                batch.spatial_chunks()
            } else {
                vec![batch]
            };
            if chunks.len() > 1 {
                stats.partitioned_world_batches += 1;
            }
            chunks.into_iter().map(move |batch| (key.clone(), batch))
        })
        .collect::<Vec<_>>();
    let mut material_handles = BTreeMap::new();
    let mut skeletons = BTreeMap::<Owner, crate::gpu_skinning::Skeleton>::new();
    for ((owner, name, lm, _), mut batch) in batches {
        let fallback = MaterialData::default();
        let definition = loaded.materials.get(&name).unwrap_or(&fallback);
        // Uniforms and proxy clocks belong to this material/lightmap pair, not its owner.
        // Eye projection remains per-mesh UV data, so sharing this handle cannot share gaze.
        // Entity color modulation: rendercolor tints the model; renderamt is its alpha
        // outside kRenderNormal (e.g. the trainstation's dim additive vol_light shafts).
        let modulation = owner
            .entity()
            .map_or(Vec4::ONE, |id| entity_modulation(world, id));
        // Studio models carry per-entity light state, so their materials are not shared.
        let lit = batch.model
            && matches!(owner, Owner::Entity(_) | Owner::Weapon(_))
            && !definition.unlit
            && !definition.camera;
        let material = material_handles
            .entry((
                name.clone(),
                lm,
                modulation.to_array().map(f32::to_bits),
                lit.then(|| owner.clone()),
            ))
            .or_insert_with(|| {
                let mut material = make_material(
                    definition,
                    bases.get(&name).unwrap_or(&missing).clone(),
                    lm.and_then(|index| lightmaps.get(index))
                        .unwrap_or(&white)
                        .clone(),
                    irises.get(&name).or_else(|| overlays.get(&name)).cloned(),
                    (&white, &black_cube),
                );
                material.tint *= modulation;
                // World and brush surfaces only: model envmaps are not loaded yet.
                if let Some(envmap) = envmaps.get(&name).filter(|_| !batch.model) {
                    material.envmap = envmap.clone();
                    material.envmap_tint.w = 1.;
                }
                if lit {
                    material.lighting = ModelLighting::fallback(definition.half_lambert);
                }
                materials.add(material)
            })
            .clone();
        stats.meshes += 1;
        let triangles = batch.indices.len() / 3;
        stats.triangles += triangles;
        let mut animation = if batch.skin.iter().any(|(_, w)| w.is_some()) {
            match &owner {
                Owner::Entity(id) => world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(*id))
                    .map(|instance| AnimatedMesh {
                        sampled: None,
                        gpu: None,
                        entity: Some(*id),
                        weapon: None,
                        key: instance.asset_key(),
                        scale: instance.scale,
                        flex_base: batch.skin.iter().map(|(p, _)| *p).collect(),
                        normals: batch.normals.clone(),
                        bind: std::mem::take(&mut batch.skin),
                        flex: std::mem::take(&mut batch.flex),
                        flex_signature: 0,
                    }),
                Owner::Weapon(name) => Some(AnimatedMesh {
                    sampled: None,
                    gpu: None,
                    entity: None,
                    weapon: Some(name.clone()),
                    key: format!(
                        "{}#0",
                        loaded.gameplay.weapons[name].viewmodel.to_lowercase()
                    ),
                    scale: 1.,
                    normals: batch.normals.clone(),
                    bind: std::mem::take(&mut batch.skin),
                    flex: Vec::new(),
                    flex_base: Vec::new(),
                    flex_signature: 0,
                }),
                Owner::World | Owner::SkyWorld => None,
            }
        } else {
            None
        };
        if !cpu_skinning
            && let Some(animation) = &mut animation
            && let Some(rig) = world
                .rigs
                .get(&animation.key)
                .filter(|rig| !rig.bones.is_empty() && rig.bones.len() < 256)
        {
            let skeleton = skeletons.entry(owner.clone()).or_insert_with(|| {
                let mut root = commands.spawn((
                    crate::campaign::MapOwned,
                    Transform::IDENTITY,
                    Visibility::Inherited,
                ));
                match &owner {
                    Owner::Entity(id) => {
                        root.insert(SourceEntity(*id));
                    }
                    Owner::Weapon(name) => {
                        root.insert(WeaponMesh(name.clone()));
                    }
                    _ => unreachable!("only owned model meshes animate"),
                }
                let root = root.id();
                crate::gpu_skinning::Skeleton::spawn(
                    commands,
                    inverse_binds,
                    rig,
                    animation.scale,
                    root,
                )
            });
            animation.gpu = Some(skeleton.clone());
        }
        // CPU assets retain bounds and eye projection data; GPU skinning keeps their positions immutable.
        let animated = animation.is_some() || irises.contains_key(&name);
        let mut mesh = batch.mesh();
        if let Some(animation) = &animation
            && let Some(skeleton) = &animation.gpu
        {
            skeleton.attributes(&mut mesh, &animation.bind);
        }
        if animated {
            mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        }
        let transform = owner.entity().map_or(Transform::IDENTITY, |id| {
            let state = &loaded.gameplay.scene.states[id];
            entity_transform(state.origin, state.rotation)
        });
        let mut draw = commands.spawn((
            crate::campaign::MapOwned,
            Name::new(name.clone()),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            transform,
        ));
        if matches!(owner, Owner::SkyWorld)
            || owner
                .entity()
                .is_some_and(|id| world.background_entities.contains(&id))
        {
            draw.insert(bevy::camera::visibility::RenderLayers::layer(4));
        }
        if matches!(&owner, Owner::World | Owner::Entity(_))
            && !owner
                .entity()
                .is_some_and(|id| world.background_entities.contains(&id))
        {
            draw.insert((
                crate::visibility::PvsDraw::default(),
                DrawTriangles(triangles),
            ));
        }
        if definition.camera {
            draw.insert(crate::monitors::MonitorMaterial {
                animation: definition.camera_animation.clone(),
                color: definition.tint,
                color2: definition.camera_color2,
            });
            // Avoid sampling a render attachment while drawing into that same attachment.
            draw.insert(bevy::camera::visibility::RenderLayers::layer(5));
        }
        if let Some(id) = owner.entity() {
            draw.insert(SourceEntity(id));
            if let Some(instance) = world.model_instances.iter().find(|i| i.entity == Some(id))
                && let Some(eye) = loaded.eyes.get(&(instance.asset_key(), name.clone()))
            {
                draw.insert(EyeMesh {
                    key: instance.asset_key(),
                    eye: eye.clone(),
                    scale: instance.scale,
                });
            }
            let state = &loaded.gameplay.scene.states[id];
            if !state.visible || state.killed {
                draw.insert(Visibility::Hidden);
            }
        }
        if let Owner::Weapon(name) = &owner {
            draw.insert((
                WeaponMesh(name.clone()),
                bevy::camera::visibility::RenderLayers::layer(1),
                Visibility::Hidden,
            ));
        }
        if lit {
            let (entity, key, scale) = match &owner {
                Owner::Entity(id) => {
                    let instance = world.model_instances.iter().find(|i| i.entity == Some(*id));
                    (
                        Some(*id),
                        instance.map(|i| i.asset_key()).unwrap_or_default(),
                        instance.map_or(1., |i| i.scale),
                    )
                }
                Owner::Weapon(weapon) => (
                    None,
                    format!(
                        "{}#0",
                        loaded.gameplay.weapons[weapon].viewmodel.to_lowercase()
                    ),
                    1.,
                ),
                Owner::World | Owner::SkyWorld => unreachable!("lit draws are model owned"),
            };
            draw.insert(ModelLit {
                entity,
                key,
                scale,
                half_lambert: definition.half_lambert,
                last: None,
            });
        }
        if let Some(animation) = animation {
            if let Some(skeleton) = &animation.gpu {
                stats.gpu_skinned_meshes += 1;
                draw.insert((
                    skeleton.component(),
                    bevy::camera::visibility::DynamicSkinnedMeshBounds,
                ));
            }
            if animation.gpu.is_none() {
                stats.cpu_skinned_meshes += 1;
            }
            draw.insert(animation);
        }
    }
    stats.skin_joints = skeletons.values().map(|s| s.joints.len()).sum();
    stats.materials = material_handles.len();
}
/// Source render color/alpha of an entity (rendercolor, renderamt when rendermode != 0).
fn entity_modulation(world: &modkit_core::World, id: usize) -> Vec4 {
    let Some(e) = world.entities.get(id) else {
        return Vec4::ONE;
    };
    let color = e
        .get("rendercolor")
        .and_then(modkit_core::parse_vec3)
        .map_or(Vec3::ONE, |c| {
            Vec3::from_array(c.to_array()).clamp(Vec3::ZERO, Vec3::splat(255.)) / 255.
        });
    let alpha = if e.get("rendermode").is_some_and(|m| m.trim() != "0") {
        e.get("renderamt")
            .and_then(|a| a.trim().parse::<f32>().ok())
            .map_or(1., |a| (a / 255.).clamp(0., 1.))
    } else {
        1.
    };
    color.extend(alpha)
}
pub(crate) fn mesh_from_surface(surface: &Surface, definition: &MaterialData) -> Mesh {
    let mut batch = Batch::default();
    batch.append(surface, Mat4::IDENTITY, Some(definition));
    batch.mesh()
}
pub(crate) fn make_material(
    definition: &MaterialData,
    base: Handle<Image>,
    lightmap: Handle<Image>,
    iris: Option<Handle<Image>>,
    (white, black_cube): (&Handle<Image>, &Handle<Image>),
) -> SourceMaterial {
    let alpha = alpha_mode(definition);
    SourceMaterial {
        tint: Vec4::new(
            definition.tint[0],
            definition.tint[1],
            definition.tint[2],
            definition.opacity,
        ),
        parameters: Vec4::new(
            definition
                .alpha_cutoff
                .unwrap_or(if matches!(alpha, AlphaMode::Opaque) {
                    0.
                } else {
                    0.001
                }),
            f32::from(matches!(alpha, AlphaMode::Opaque)),
            f32::from(matches!(alpha, AlphaMode::Add)),
            if definition.camera {
                if definition.camera_vertex_color {
                    -2.
                } else {
                    -1.
                }
            } else {
                f32::from(iris.is_some())
            },
        ),
        iris: iris.unwrap_or_else(|| white.clone()),
        secondary_uv: Mat3::IDENTITY,
        lighting: ModelLighting::default(),
        envmap: black_cube.clone(),
        envmap_tint: Vec3::from_array(definition.envmap_tint).extend(0.),
        envmap_contrast: Vec3::from_array(definition.envmap_contrast)
            .extend(definition.fresnel_reflection),
        envmap_saturation: Vec3::from_array(definition.envmap_saturation)
            .extend(f32::from(definition.base_alpha_envmap_mask)),
        base,
        lightmap: if definition.unlit {
            white.clone()
        } else {
            lightmap
        },
        alpha,
        two_sided: definition.two_sided,
    }
}

fn alpha_mode(material: &MaterialData) -> AlphaMode {
    if material.additive {
        AlphaMode::Add
    } else if let Some(cutoff) = material.alpha_cutoff {
        AlphaMode::Mask(cutoff)
    } else if material.translucent || material.opacity < 1. {
        AlphaMode::Blend
    } else {
        AlphaMode::Opaque
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn partition_fixture() -> Batch {
        let positions = vec![
            [-2048., 0., 0.],
            [-2030., 1., 0.],
            [-2035., 0., 2.],
            [2050., 0., 0.],
            [2060., 1., 0.],
            [2055., 0., 2.],
        ];
        Batch {
            flex: Vec::new(),
            uvs: (0..6).map(|i| [i as f32 * 0.1, -i as f32]).collect(),
            light_uvs: (0..6).map(|i| [0.3, i as f32 * 0.2]).collect(),
            colors: (0..6).map(|i| [i as f32, 0.25, 0.5, 1.]).collect(),
            normals: (0..6).map(|i| glam::Vec3::new(0., 0., i as f32)).collect(),
            model: false,
            skin: positions
                .iter()
                .map(|p| (glam::Vec3::from_array(*p), None))
                .collect(),
            positions,
            // Reversed winding, shared vertices and a degenerate triangle must survive.
            indices: vec![0, 1, 2, 3, 5, 4, 0, 3, 4, 1, 2, 2],
        }
    }
    fn triangle_payload(batch: &Batch) -> Vec<Vec<u32>> {
        batch
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|triangle| {
                triangle
                    .iter()
                    .flat_map(|&i| {
                        let i = i as usize;
                        batch.positions[i]
                            .into_iter()
                            .chain(batch.uvs[i])
                            .chain(batch.light_uvs[i])
                            .chain(batch.colors[i])
                            .map(f32::to_bits)
                    })
                    .collect()
            })
            .collect()
    }
    #[test]
    fn spatial_partition_preserves_triangle_attributes_and_spanning_bounds() {
        let mut expected = triangle_payload(&partition_fixture());
        let chunks = partition_fixture().spatial_chunks();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks.iter().map(|b| b.indices.len()).sum::<usize>(), 12);
        let mut actual: Vec<_> = chunks.iter().flat_map(triangle_payload).collect();
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
        // Bounds come from whole triangles; the central group spans over four cells.
        assert!(
            chunks
                .into_iter()
                .any(|b| b.mesh().compute_aabb().unwrap().half_extents.x > 2000.)
        );
    }
    #[test]
    fn spatial_partition_keeps_original_for_invalid_data_or_excessive_chunks() {
        let mut invalid = partition_fixture();
        invalid.indices[0] = u32::MAX;
        assert_eq!(invalid.spatial_chunks().len(), 1);
        let mut invalid = partition_fixture();
        invalid.uvs.pop();
        assert_eq!(invalid.spatial_chunks().len(), 1);
        let mut invalid = partition_fixture();
        invalid.indices.push(99);
        let unchanged = invalid.spatial_chunks();
        assert_eq!(unchanged.len(), 1);
        assert_eq!(unchanged[0].indices.last(), Some(&99));
        let mut invalid = partition_fixture();
        invalid.positions[0][0] = f32::NAN;
        assert_eq!(invalid.spatial_chunks().len(), 1);
        let mut broad = Batch::default();
        for cell in 0..65 {
            for vertex in 0..3 {
                broad
                    .positions
                    .push([cell as f32 * 2048., vertex as f32, 0.]);
                broad.uvs.push([0.; 2]);
                broad.light_uvs.push([0.; 2]);
                broad.colors.push([1.; 4]);
                broad.skin.push((glam::Vec3::ZERO, None));
                broad.indices.push(broad.indices.len() as u32);
            }
        }
        let unchanged = broad.spatial_chunks();
        assert_eq!(unchanged.len(), 1);
        assert_eq!(unchanged[0].indices.len(), 195);
    }
    #[test]
    fn animation_keys_keep_live_samples_and_stop_reuploading_constant_poses() {
        use modkit_core::animation::Clip;
        let mut clip = Clip {
            layer: Default::default(),
            fps: 30.,
            looping: false,
            frames: vec![vec![]; 31],
            events: vec![],
        };
        assert_ne!(sample_key(&clip, 0.2), sample_key(&clip, 0.3));
        assert_eq!(sample_key(&clip, 1.), sample_key(&clip, 50.));
        clip.looping = true;
        assert_ne!(sample_key(&clip, 1.), sample_key(&clip, 50.));
        clip.frames.truncate(1);
        assert_eq!(sample_key(&clip, 0.), sample_key(&clip, 50.));
    }
    #[test]
    fn entity_pose_conversion_preserves_scaled_local_points_at_arbitrary_angles() {
        let origin = glam::Vec3::new(10., -300., 45.);
        let rotation = hl2_simulation::physics::angles(glam::Vec3::new(17., 90., -12.));
        let local = glam::Vec3::new(40., 4., 16.) * 1.3;
        let converted = entity_transform(origin, rotation)
            .transform_point(source_to_bevy(Vec3::from_array(local.to_array())));
        let expected = source_to_bevy(Vec3::from_array((origin + rotation * local).to_array()));
        assert!(converted.distance(expected) < 0.0001);
    }
    #[test]
    fn animation_system_updates_mesh_bounds_pose_and_scripted_visibility() {
        use bevy::ecs::system::RunSystemOnce;
        use modkit_core::animation::{Bone, Clip, Pose, Rig, Weights};
        let key = "synthetic#0".to_owned();
        let world = modkit_core::World {
            entities: vec![modkit_core::Entity {
                properties: vec![
                    ("classname".into(), "prop_dynamic".into()),
                    ("DefaultAnim".into(), "move".into()),
                    ("origin".into(), "100 50 10".into()),
                    ("angles".into(), "0 90 0".into()),
                ],
            }],
            rigs: BTreeMap::from([(
                key.clone(),
                Rig {
                    pose_parameters: Vec::new(),
                    autoplay: Vec::new(),
                    attachments: Vec::new(),
                    bones: vec![Bone {
                        name: "root".into(),
                        parent: None,
                        bind: Pose {
                            position: glam::Vec3::ZERO,
                            rotation: glam::Quat::IDENTITY,
                        },
                        inverse_bind: glam::Mat4::IDENTITY,
                    }],
                    clips: BTreeMap::from([(
                        "move".into(),
                        Clip {
                            layer: Default::default(),
                            fps: 1.,
                            looping: false,
                            events: vec![],
                            frames: vec![
                                vec![Pose {
                                    position: glam::Vec3::ZERO,
                                    rotation: glam::Quat::IDENTITY,
                                }],
                                vec![Pose {
                                    position: glam::Vec3::Z * 8.,
                                    rotation: glam::Quat::IDENTITY,
                                }],
                            ],
                        },
                    )]),
                    warnings: vec![],
                    sequences: vec![],
                },
            )]),
            ..default()
        };
        let mut game = crate::gameplay::Gameplay::synthetic(world);
        game.scene.time = 0.5;
        let sim = crate::movement::Simulation::new(
            &game.world,
            glam::Vec3::Z * 128.,
            0.,
            0.,
            false,
            vec![],
        );
        let mut meshes = Assets::<Mesh>::default();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[1., 0., 0.]]);
        let handle = meshes.add(mesh);
        let mut app = App::new();
        app.insert_resource(game)
            .insert_resource(sim)
            .insert_resource(meshes);
        let entity = app
            .world_mut()
            .spawn((
                crate::campaign::MapOwned,
                SourceEntity(0),
                Transform::IDENTITY,
                Visibility::Inherited,
                AnimatedMesh {
                    sampled: None,
                    gpu: None,
                    entity: Some(0),
                    weapon: None,
                    key,
                    scale: 2.,
                    normals: Vec::new(),
                    flex: Vec::new(),
                    flex_base: Vec::new(),
                    flex_signature: 0,
                    bind: vec![(
                        glam::Vec3::X,
                        Some(Weights {
                            bones: [0; 3],
                            weights: [1., 0., 0.],
                        }),
                    )],
                },
                Mesh3d(handle.clone()),
                Aabb::default(),
            ))
            .id();
        app.world_mut().run_system_once(present_entities).unwrap();
        let meshes = app.world().resource::<Assets<Mesh>>();
        let mesh = meshes.get(&handle).unwrap();
        assert_eq!(
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                .unwrap()
                .as_float3()
                .unwrap(),
            &[[2., 8., 0.]]
        );
        assert!(
            Vec3::from(app.world().get::<Aabb>(entity).unwrap().center)
                .distance(Vec3::new(2., 8., 0.))
                < 0.0001
        );
        let point = app
            .world()
            .get::<Transform>(entity)
            .unwrap()
            .transform_point(Vec3::new(2., 8., 0.));
        assert!(point.distance(source_to_bevy(Vec3::new(100., 52., 18.))) < 0.0001);
        app.world_mut()
            .resource_mut::<crate::gameplay::Gameplay>()
            .scene
            .states[0]
            .visible = false;
        app.world_mut().run_system_once(present_entities).unwrap();
        assert_eq!(
            *app.world().get::<Visibility>(entity).unwrap(),
            Visibility::Hidden
        );
    }
    #[test]
    fn alpha_test_takes_precedence_over_translucency_and_additive_over_both() {
        let mut material = MaterialData {
            alpha_cutoff: Some(0.5),
            translucent: true,
            ..default()
        };
        assert_eq!(alpha_mode(&material), AlphaMode::Mask(0.5));
        material.additive = true;
        assert_eq!(alpha_mode(&material), AlphaMode::Add);
    }
}
