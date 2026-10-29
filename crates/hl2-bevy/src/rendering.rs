//! Baked-lightmap material, independent of Bevy's PBR lighting model.
use crate::{
    Status,
    assets::{LoadedMap, MaterialData, visible_entity},
    source_to_bevy,
};
use bevy::{
    asset::RenderAssetUsages,
    camera::primitives::{Aabb, MeshAabb},
    mesh::Indices,
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, Extent3d, PrimitiveTopology, ShaderType, TextureDimension, TextureFormat,
    },
};
use modkit_core::Surface;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod drawing;
mod lighting;
pub(crate) mod lod;
mod map_draw;
mod material;
pub use lighting::present_lighting;
pub use material::MaterialKey;
mod model_cache;
mod runtime;
use map_draw::{MapDrawContext, insert_map_draw_components};
use material::{alpha_mode, cube_image_filtered, image_filtered, mip_image_filtered};
pub(crate) use material::{black_cube, fallback_images, image, make_material, sampler_descriptor};
pub use model_cache::ModelCache;
use model_cache::strip_pixels;
pub use runtime::{despawn_killed_runtime_draws, spawn_runtime_draws};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(MaterialKey)]
pub struct SourceMaterial {
    #[uniform(0)]
    pub(crate) tint: Vec4,
    // x = alpha cutoff, y = ignore base alpha, z = additive output.
    #[uniform(0)]
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
    #[uniform(0)]
    pub(crate) secondary_uv: Mat3,
    #[uniform(0)]
    pub(crate) lighting: ModelLighting,
    /// LightmappedGeneric $envmap; a black fallback cube when absent.
    #[texture(10, dimension = "cube")]
    #[sampler(11)]
    pub(crate) envmap: Handle<Image>,
    /// xyz = $envmaptint, w = 1 when an envmap is bound.
    #[uniform(0)]
    pub(crate) envmap_tint: Vec4,
    /// xyz = $envmapcontrast, w = $fresnelreflection.
    #[uniform(0)]
    pub(crate) envmap_contrast: Vec4,
    /// xyz = $envmapsaturation, w = mask flags: 1 = 1 - base alpha
    /// ($basealphaenvmapmask), 2 = mask RGB ($envmapmask), 4 = mask alpha
    /// ($normalmapalphaenvmapmask on the $bumpmap), 8 = the mask texture is the $bumpmap
    /// and its normals perturb the reflection.
    #[uniform(0)]
    pub(crate) envmap_saturation: Vec4,
    /// The envmap mask texture; white when absent.
    #[texture(15)]
    #[sampler(16)]
    pub(crate) envmap_mask: Handle<Image>,
    /// xyz = $selfillumtint, w = 1 when $selfillum applies.
    #[uniform(0)]
    pub(crate) self_illum: Vec4,
    /// The flashlight's projected texture (one handle shared by every material).
    #[texture(18)]
    #[sampler(19)]
    pub(crate) flashlight: Handle<Image>,
    #[uniform(0)]
    pub(crate) viewmodel_flashlight: crate::flashlight::ViewmodelLight,
    pub(crate) alpha: AlphaMode,
    pub(crate) two_sided: bool,
}
/// Source model lighting for one entity: ambient cube plus up to four local
/// lights, positions/directions in Bevy space and colors linear.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct ModelLighting {
    /// x = lit model, y = local light count, z = half-Lambert, w = 1 when the flashlight does not light
    /// it (unlit, camera, additive and sky materials); 2 for the viewmodel beam path.
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
    /// Entity lights (muzzle elights) lit this draw last time: relight once they end.
    dynamic: bool,
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
impl AnimatedMesh {
    /// A batch's animation data (its skin taken; facial flexes only for an entity's own model).
    fn from_batch(
        batch: &mut Batch,
        (entity, weapon): (Option<usize>, Option<String>),
        (key, scale): (String, f32),
        flexes: bool,
    ) -> Self {
        Self {
            sampled: None,
            gpu: None,
            entity,
            weapon,
            key,
            scale,
            flex_base: if flexes {
                batch.skin.iter().map(|(p, _)| *p).collect()
            } else {
                Vec::new()
            },
            normals: batch.normals.clone(),
            bind: std::mem::take(&mut batch.skin),
            flex: if flexes {
                std::mem::take(&mut batch.flex)
            } else {
                Vec::new()
            },
            flex_signature: 0,
        }
    }
    /// A template draw's animation for another entity on its own skeleton.
    fn copy_for(&self, entity: usize, gpu: Option<crate::gpu_skinning::Skeleton>) -> Self {
        Self {
            sampled: None,
            gpu,
            entity: Some(entity),
            weapon: self.weapon.clone(),
            key: self.key.clone(),
            scale: self.scale,
            bind: self.bind.clone(),
            normals: self.normals.clone(),
            flex: self.flex.clone(),
            flex_base: self.flex_base.clone(),
            flex_signature: 0,
        }
    }
}
/// A private copy of an asset (a mesh or material an entity changes on its own).
pub(crate) fn copy_asset<A: Asset + Clone>(
    assets: &mut Assets<A>,
    handle: &Handle<A>,
) -> Option<Handle<A>> {
    let copy = assets.get(handle)?.clone();
    Some(assets.add(copy))
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
/// A bone-merged NPC weapon mesh ("<world model>#0"): drawn only while it is its
/// owner's active weapon.
#[derive(Component)]
pub struct NpcWeaponDraw(pub String);
/// Whether an entity's draw is hidden this frame: its entity is hidden or killed, or it
/// is an NPC weapon its owner does not hold now.
pub(crate) fn draw_hidden(
    game: &crate::gameplay::Gameplay,
    id: usize,
    weapon: Option<&NpcWeaponDraw>,
) -> bool {
    let Some(s) = game.scene.states.get(id) else {
        return true;
    };
    // A ragdolled NPC stays drawn (its bone-merged weapon does not).
    let ragdoll = game.scene.ragdoll_matrices.contains_key(&id);
    (!s.visible || s.killed) && !(ragdoll && weapon.is_none())
        || ragdoll && weapon.is_some()
        || weapon.is_some_and(|w| {
            game.ai
                .weapon_world_model(id)
                .is_none_or(|m| w.0.strip_suffix("#0") != Some(m.as_str()))
        })
}
#[allow(clippy::too_many_arguments)]
pub fn present_entities(
    game: Res<crate::gameplay::Gameplay>,
    sim: Res<crate::simulation::Simulation>,
    mut entities: Query<
        (
            &SourceEntity,
            &mut Transform,
            &mut Visibility,
            Option<&NpcWeaponDraw>,
        ),
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
    for (owner, mut transform, mut visibility, weapon) in &mut entities {
        let Some(state) = game.scene.states.get(owner.0) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let (origin, rotation) = sim
            .physics
            .entity_pose(owner.0)
            .unwrap_or((state.origin, state.rotation));
        let pose = entity_transform(origin, rotation);
        if *transform != pose {
            *transform = pose;
        }
        let visible = if draw_hidden(&game, owner.0, weapon) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != visible {
            *visibility = visible;
        }
    }
    let mut updated_joints = std::collections::BTreeSet::new();
    let mut poses: BTreeMap<(String, String, u32, u64), Vec<glam::Mat4>> = BTreeMap::new();
    for (mut animation, handle, aabb) in &mut animations {
        if animation.entity.is_some() && animation.weapon.is_some() {
            continue;
        }
        let Some(rig) = game.world.rigs.get(&animation.key) else {
            continue;
        };
        // Facial flexes deform the bind positions before skinning (both GPU and CPU paths).
        apply_facial_flexes(
            &mut animation,
            handle,
            &mut meshes,
            &game,
            flex_models.as_deref(),
            eyes.as_deref(),
        );
        let (clip, key) = if let Some(id) = animation.entity {
            let Some(state) = game.scene.states.get(id) else {
                continue;
            };
            if (!state.visible || state.killed) && !game.scene.ragdoll_matrices.contains_key(&id) {
                continue;
            }
            let clip = state.animation.as_str();
            let time = game.scene.animation_time(id);
            let Some(definition) = rig.clips.get(clip) else {
                continue;
            };
            (clip.to_owned(), sample_key(definition, time))
        } else {
            if animation.weapon.as_deref() != Some(game.inventory.active.as_str()) {
                continue;
            }
            let state = game
                .inventory
                .viewmodel_animation_state(rig, game.scene.time);
            state.cache_key(rig)
        };
        // Gesture layers make the pose actor-specific; the entity id keeps cache entries apart.
        let layers = animation
            .entity
            .map_or(0, |id| game.scene.pose_signature(id));
        // Viewmodel pose parameters (the gravity gun's "active") change the pose too.
        let viewmodel_params = animation
            .entity
            .is_none()
            .then(|| game.inventory.viewmodel_pose_values(rig));
        let layer_key = match (layers, animation.entity) {
            (_, None) => viewmodel_params.as_ref().map_or(0, |params| {
                params.iter().fold(0u64, |hash, v| {
                    (hash ^ u64::from(v.to_bits())).wrapping_mul(0x100000001b3)
                })
            }),
            (0, _) => 0,
            (signature, Some(id)) => signature ^ (id as u64).rotate_left(32),
        };
        if animation
            .sampled
            .as_ref()
            .is_some_and(|(name, previous, layered)| {
                name == &clip && *previous == key && *layered == layer_key
            })
        {
            continue;
        }
        let sampled = (clip.clone(), key, layer_key);
        let matrices = poses
            .entry((animation.key.clone(), clip, key, layer_key))
            .or_insert_with(|| match animation.entity {
                Some(id) => game.scene.actor_matrices(rig, id),
                None => game.inventory.viewmodel_matrices(rig, game.scene.time),
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

fn apply_facial_flexes(
    animation: &mut AnimatedMesh,
    handle: &Mesh3d,
    meshes: &mut Assets<Mesh>,
    game: &crate::gameplay::Gameplay,
    flex_models: Option<&FlexModels>,
    eyes: Option<&crate::eyes::Eyes>,
) {
    let (Some(id), Some(model)) = (
        animation.entity,
        flex_models
            .and_then(|f| f.0.get(&animation.key))
            .filter(|_| !animation.flex.is_empty()),
    ) else {
        return;
    };
    let values = game.scene.actor_flex_values(id);
    let weights = model.descriptor_weights(&values, |eye| {
        eyes.and_then(|e| e.lid_bases.get(&(id, eye.surface)))
            .copied()
    });
    let signature = flex_signature(&weights);
    if signature == animation.flex_signature {
        return;
    }
    let deltas = model.flex.vertex_deltas(&weights);
    let scale = animation.scale;
    let mut changed = Vec::new();
    for (i, key) in animation
        .flex
        .iter()
        .enumerate()
        .filter_map(|(i, k)| k.as_ref().map(|k| (i, k)))
    {
        let target = animation.flex_base[i] + deltas.get(key).copied().unwrap_or(glam::Vec3::ZERO);
        if animation.bind[i].0 != target {
            animation.bind[i].0 = target;
            changed.push(i);
        }
    }
    animation.flex_signature = signature;
    if changed.is_empty() {
        return;
    }
    if animation.gpu.is_some() {
        let mut mesh = meshes.get_mut(&handle.0);
        let positions = mesh
            .as_deref_mut()
            .and_then(|m| m.attribute_mut(Mesh::ATTRIBUTE_POSITION));
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = positions {
            for &i in &changed {
                if let Some(p) = positions.get_mut(i) {
                    *p = source_to_bevy(Vec3::from_array((animation.bind[i].0 * scale).to_array()))
                        .to_array();
                }
            }
        }
    } else {
        // Force the CPU skinning path to rebuild from the deformed bind.
        animation.sampled = None;
    }
}
/// C_ClientRagdoll::FadeOut: a retired ragdoll's model blends out (kRenderTransAlpha).
pub fn present_ragdoll_fades(
    game: Res<crate::gameplay::Gameplay>,
    draws: Query<(&ModelLit, &MeshMaterial3d<SourceMaterial>)>,
    mut materials: ResMut<Assets<SourceMaterial>>,
) {
    if game.ragdolls.fading() == 0 {
        return;
    }
    for (lit, material) in &draws {
        let Some(alpha) = lit.entity.and_then(|id| game.ragdolls.fade_alpha(id)) else {
            continue;
        };
        if let Some(mut m) = materials.get_mut(&material.0)
            && m.tint.w != alpha
        {
            m.tint.w = alpha;
            m.alpha = AlphaMode::Blend;
        }
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
pub struct WeaponMesh(pub(crate) String);
/// A viewmodel draw that belongs to one skin only (surfaces whose material differs
/// between skins 0 and 1).
#[derive(Component)]
pub struct WeaponSkin(u8);
pub fn present_weapons(
    game: Res<crate::gameplay::Gameplay>,
    mut draws: Query<(&WeaponMesh, Option<&WeaponSkin>, &mut Visibility)>,
) {
    let skin = game.inventory.viewmodel_skin();
    for (weapon, variant, mut visibility) in &mut draws {
        // The +USE carry holsters the weapon (CPlayerPickupController::Init).
        *visibility = if weapon.0 == game.inventory.active
            && !game.inventory.carrying()
            && variant.is_none_or(|v| v.0 == skin)
        {
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
    /// An NPC's weapon world model, bone-merged onto the NPC (EF_BONEMERGE).
    NpcWeapon(usize, String),
}
impl Owner {
    fn entity(&self) -> Option<usize> {
        match self {
            Self::Entity(id) | Self::NpcWeapon(id, _) => Some(*id),
            _ => None,
        }
    }
}

#[allow(dead_code)]
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
    spawn_map_filtered(
        loaded,
        commands,
        (meshes, inverse_binds),
        materials,
        images,
        status,
        (camera_target, cpu_skinning, world_partition),
        hl2_ui::config::Filtering::Trilinear,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_map_filtered(
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
    filtering: hl2_ui::config::Filtering,
) {
    let world = &loaded.world;
    let (white, black_cube, flashlight, missing) =
        fallback_images(images, loaded.flashlight_cookie.as_deref(), filtering);
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
                            images.add(image_filtered(
                                base.width,
                                base.height,
                                base.rgba.clone(),
                                true,
                                filtering,
                            ))
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
                            images.add(self::image_filtered(
                                image.width,
                                image.height,
                                image.rgba.clone(),
                                false,
                                filtering,
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
                            let mut overlay = image_filtered(
                                overlay.width,
                                overlay.height,
                                overlay.rgba.clone(),
                                true,
                                filtering,
                            );
                            overlay.texture_descriptor.format = TextureFormat::Rgba8UnormSrgb;
                            images.add(overlay)
                        })
                        .clone();
                    (name.clone(), handle)
                })
        })
        .collect();
    let two_textures: BTreeMap<_, _> = loaded
        .materials
        .iter()
        .filter_map(|(name, m)| {
            let textures = m.two_texture.as_ref()?;
            let mut upload = |path: &str, frames: &crate::assets::TextureFrames| {
                frames
                    .iter()
                    .enumerate()
                    .map(|(i, chain)| {
                        texture_handles
                            .entry(format!("{path}#frame{i}#mips#srgb"))
                            .or_insert_with(|| images.add(mip_image_filtered(chain, filtering)))
                            .clone()
                    })
                    .collect::<Vec<_>>()
            };
            let base = upload(m.base_path.as_deref()?, &textures.base_frames);
            let second = upload(&textures.second_path, &textures.second_frames);
            Some((name.clone(), (base, second)))
        })
        .collect();
    let envmaps: BTreeMap<_, _> = loaded
        .materials
        .iter()
        .filter_map(|(name, m)| {
            let (cube, path) = m.envmap.as_ref().zip(m.envmap_path.as_ref())?;
            let handle = texture_handles
                .entry(format!("{path}#cube"))
                .or_insert_with(|| {
                    images.add(cube_image_filtered(
                        cube.size,
                        cube.hdr,
                        cube.faces.concat(),
                        filtering,
                    ))
                })
                .clone();
            Some((name.clone(), handle))
        })
        .collect();
    // Masks are linear data, sampled with the base UVs (repeat), like base textures.
    let envmap_masks: BTreeMap<_, _> = loaded
        .materials
        .iter()
        .filter_map(|(name, m)| {
            let (mask, path) = m.envmap_mask.as_ref().zip(m.envmap_mask_path.as_ref())?;
            let handle = texture_handles
                .entry(path.clone())
                .or_insert_with(|| {
                    images.add(image_filtered(
                        mask.width,
                        mask.height,
                        mask.rgba.clone(),
                        true,
                        filtering,
                    ))
                })
                .clone();
            Some((name.clone(), handle))
        })
        .collect();
    let lightmaps: Vec<_> = world
        .lightmaps
        .iter()
        .map(|lm| {
            let lightmap = Image::new(
                Extent3d {
                    width: lm.width.into(),
                    height: lm.height.into(),
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                lm.rgba.iter().flat_map(|t| t.to_le_bytes()).collect(),
                TextureFormat::Rgba16Float,
                RenderAssetUsages::RENDER_WORLD,
            );
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
    for surface in &world.overlays {
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
    // NPC weapons: the active weapon's world model follows its owner (DESIGN 14w step 4).
    // A pooled F4 spawn gets one per weapon its class can use, shown only while it is
    // the active one (NpcWeaponDraw); the pool and this goes with DESIGN 14r.
    for instance in &world.model_instances {
        let Some(id) = instance.entity.filter(|id| visible_entity(world, *id)) else {
            continue;
        };
        let ai = &loaded.gameplay.ai;
        let mut models: BTreeSet<String> = ai.weapon_world_model(id).into_iter().collect();
        let entity = &world.entities[id];
        if let Some(label) = entity.get(hl2_simulation::ai::spawn::POOL_KEY) {
            models.extend(
                loaded
                    .gameplay
                    .spawn_overlay
                    .usable(label)
                    .iter()
                    .filter_map(|w| ai.weapon_model(w)),
            );
        }
        for model in models {
            let key = format!("{model}#0");
            if let Some(surfaces) = world.model_assets.get(&key) {
                let transform = Mat4::from_scale(Vec3::splat(instance.scale));
                for surface in surfaces {
                    append(surface, transform, Owner::NpcWeapon(id, key.clone()));
                }
            }
        }
    }
    // Skin-only materials per weapon: (skin 0 set, skin 1 set).
    let mut weapon_skins: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for (name, weapon) in &loaded.gameplay.weapons {
        let model = weapon.viewmodel.to_lowercase();
        if let Some(surfaces) = world.model_assets.get(&format!("{model}#0")) {
            for surface in surfaces {
                append(surface, Mat4::IDENTITY, Owner::Weapon(name.clone()));
            }
            if let Some(alternate) = world.model_assets.get(&format!("{model}#1")) {
                for (a, b) in surfaces.iter().zip(alternate) {
                    if a.material != b.material {
                        let sets = weapon_skins.entry(name.clone()).or_default();
                        sets.0.insert(a.material.clone());
                        sets.1.insert(b.material.clone());
                        append(b, Mat4::IDENTITY, Owner::Weapon(name.clone()));
                    }
                }
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
            && matches!(
                owner,
                Owner::Entity(_) | Owner::Weapon(_) | Owner::NpcWeapon(..)
            )
            && !definition.unlit
            && !definition.camera;
        let material = material_handles
            .entry((
                name.clone(),
                lm,
                modulation.to_array().map(f32::to_bits),
                (lit || definition.two_texture.is_some()).then(|| owner.clone()),
            ))
            .or_insert_with(|| {
                let mut material = make_material(
                    definition,
                    bases.get(&name).unwrap_or(&missing).clone(),
                    lm.and_then(|index| lightmaps.get(index))
                        .unwrap_or(&white)
                        .clone(),
                    irises.get(&name).or_else(|| overlays.get(&name)).cloned(),
                    (&white, &black_cube, &flashlight),
                );
                material.tint *= modulation;
                if let Some((base, second)) = two_textures.get(&name) {
                    material.base = base[0].clone();
                    material.iris = second[0].clone();
                    // SDK SetModulationPixelShaderDynamicState_LinearColorSpace: $color times
                    // the entity's color modulation (rendercolor, e.g. the dim vol_light
                    // shafts), then GammaToLinear per channel. The shader returns alpha 1.
                    material.tint = Vec3::from_array(std::array::from_fn(|i| {
                        crate::monitors::linear_modulation(definition.tint[i] * modulation[i])
                    }))
                    .extend(1.);
                }
                // World and brush surfaces only: model envmaps are not loaded yet.
                if let Some(envmap) = envmaps.get(&name).filter(|_| !batch.model) {
                    material.envmap = envmap.clone();
                    material.envmap_tint.w = 1.;
                    if let Some(mask) = envmap_masks.get(&name) {
                        material.envmap_mask = mask.clone();
                    }
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
                    .map(|instance| {
                        let model = (instance.asset_key(), instance.scale);
                        AnimatedMesh::from_batch(&mut batch, (Some(*id), None), model, true)
                    }),
                // `weapon` with an entity marks a bone-merged mesh: its joints are the
                // owner's, posed by the owner's own AnimatedMesh.
                Owner::NpcWeapon(id, key) => {
                    let scale = world
                        .model_instances
                        .iter()
                        .find(|i| i.entity == Some(*id))
                        .map_or(1., |i| i.scale);
                    let owner = (Some(*id), Some(key.clone()));
                    Some(AnimatedMesh::from_batch(
                        &mut batch,
                        owner,
                        (key.clone(), scale),
                        false,
                    ))
                }
                Owner::Weapon(name) => {
                    let key = loaded.gameplay.weapons[name].viewmodel.to_lowercase();
                    let owner = (None, Some(name.clone()));
                    Some(AnimatedMesh::from_batch(
                        &mut batch,
                        owner,
                        (format!("{key}#0"), 1.),
                        false,
                    ))
                }
                Owner::World | Owner::SkyWorld => None,
            }
        } else {
            None
        };
        // Bone merge: the weapon's bones follow the owner's same-named joints.
        let mut merged = false;
        if let (Owner::NpcWeapon(id, key), Some(animation)) = (&owner, &mut animation) {
            merged = true;
            let npc = skeletons.get(&Owner::Entity(*id)).cloned();
            let npc_rig = world
                .model_instances
                .iter()
                .find(|i| i.entity == Some(*id))
                .and_then(|i| world.rigs.get(&i.asset_key()));
            match (cpu_skinning, npc, npc_rig, world.rigs.get(key)) {
                (false, Some(npc), Some(npc_rig), Some(rig)) => {
                    animation.gpu = Some(crate::gpu_skinning::Skeleton::merged(
                        inverse_binds,
                        rig,
                        npc_rig,
                        &npc,
                        animation.scale,
                    ));
                }
                // CPU skinning or a missing skeleton: not drawn.
                _ => continue,
            }
        }
        if !cpu_skinning
            && !merged
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
        if let Some(skeleton) = animation.as_ref().and_then(|a| a.gpu.as_ref()) {
            skeleton.attributes(&mut mesh, &animation.as_ref().unwrap().bind);
        }
        if animated {
            mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        }
        let state = owner
            .entity()
            .and_then(|id| loaded.gameplay.scene.states.get(id));
        let transform = state.map_or(Transform::IDENTITY, |s| {
            entity_transform(s.origin, s.rotation)
        });
        let handle = meshes.add(mesh);
        let mut draw = commands.spawn((
            crate::campaign::MapOwned,
            Name::new(name.clone()),
            Mesh3d(handle.clone()),
            MeshMaterial3d(material),
            transform,
        ));
        let cx = MapDrawContext {
            owner: &owner,
            name: &name,
            triangles,
            definition,
            world,
            loaded,
            two_textures: &two_textures,
            weapon_skins: &weapon_skins,
        };
        insert_map_draw_components(&mut draw, &cx, &handle, animation.as_ref(), meshes);
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
                // The weapon is lit at its owner (bone-merged entities share the light state).
                Owner::NpcWeapon(id, _) => {
                    let instance = world.model_instances.iter().find(|i| i.entity == Some(*id));
                    (
                        Some(*id),
                        instance.map(|i| i.asset_key()).unwrap_or_default(),
                        instance.map_or(1., |i| i.scale),
                    )
                }
                Owner::World | Owner::SkyWorld => unreachable!("lit draws are model owned"),
            };
            draw.insert(ModelLit {
                entity,
                key,
                scale,
                half_lambert: definition.half_lambert,
                last: None,
                dynamic: false,
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
    drop(stats);
    commands.insert_resource(ModelCache {
        textures: texture_handles,
        definitions: loaded
            .materials
            .iter()
            .map(|(name, material)| {
                let mut material = material.clone();
                strip_pixels(&mut material);
                (name.clone(), material)
            })
            .collect(),
        white,
        black_cube,
        flashlight,
        missing,
    });
}

/// One runtime entity's studio-model draws (DESIGN 14r part B: models loaded after the
/// map): one mesh per material like the map build's entity batches, lit per entity,
/// GPU-skinned on its own skeleton when the model has bone weights.
pub(crate) fn build_model_draws(
    commands: &mut Commands,
    (meshes, materials, inverse_binds): (
        &mut Assets<Mesh>,
        &mut Assets<SourceMaterial>,
        &mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    ),
    cache: &ModelCache,
    world: &modkit_core::World,
    id: usize,
    transform: Transform,
) -> usize {
    let Some(instance) = world.model_instances.iter().find(|i| i.entity == Some(id)) else {
        return 0;
    };
    let Some(surfaces) = world.model_assets.get(&instance.asset_key()) else {
        return 0;
    };
    let scale = Mat4::from_scale(Vec3::splat(instance.scale));
    let mut batches: BTreeMap<String, Batch> = BTreeMap::new();
    for surface in surfaces.iter().filter(|s| !s.indices.is_empty()) {
        batches.entry(surface.material.clone()).or_default().append(
            surface,
            scale,
            cache.definitions.get(&surface.material),
        );
    }
    let rig = world
        .rigs
        .get(&instance.asset_key())
        .filter(|rig| !rig.bones.is_empty() && rig.bones.len() < 256);
    let modulation = entity_modulation(world, id);
    let mut skeleton: Option<crate::gpu_skinning::Skeleton> = None;
    let mut draws = 0;
    for (name, mut batch) in batches {
        let fallback = MaterialData::default();
        let definition = cache.definitions.get(&name).unwrap_or(&fallback);
        let base = definition
            .base_path
            .as_ref()
            .and_then(|p| cache.textures.get(p))
            .unwrap_or(&cache.missing)
            .clone();
        let iris = definition
            .iris_path
            .as_ref()
            .and_then(|p| cache.textures.get(p))
            .cloned();
        let lit = batch.model && !definition.unlit && !definition.camera;
        let mut material = make_material(
            definition,
            base,
            cache.white.clone(),
            iris,
            (&cache.white, &cache.black_cube, &cache.flashlight),
        );
        material.tint *= modulation;
        if lit {
            material.lighting = ModelLighting::fallback(definition.half_lambert);
        }
        let material = materials.add(material);
        let triangles = batch.indices.len() / 3;
        let mut animation = batch.skin.iter().any(|(_, w)| w.is_some()).then(|| {
            let model = (instance.asset_key(), instance.scale);
            AnimatedMesh::from_batch(&mut batch, (Some(id), None), model, true)
        });
        if let (Some(animation), Some(rig)) = (&mut animation, rig) {
            if skeleton.is_none() {
                let root = commands
                    .spawn((
                        crate::campaign::MapOwned,
                        Transform::IDENTITY,
                        Visibility::Inherited,
                        SourceEntity(id),
                    ))
                    .id();
                skeleton = Some(crate::gpu_skinning::Skeleton::spawn(
                    commands,
                    inverse_binds,
                    rig,
                    instance.scale,
                    root,
                ));
            }
            animation.gpu = skeleton.clone();
        }
        let mut mesh = batch.mesh();
        if let Some(animation) = &animation
            && let Some(skeleton) = &animation.gpu
        {
            skeleton.attributes(&mut mesh, &animation.bind);
        }
        if animation.is_some() {
            mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        }
        let handle = meshes.add(mesh);
        let mut draw = commands.spawn((
            crate::campaign::MapOwned,
            Name::new(name.clone()),
            Mesh3d(handle.clone()),
            MeshMaterial3d(material),
            transform,
            SourceEntity(id),
            crate::visibility::PvsDraw::default(),
            DrawTriangles(triangles),
        ));
        if let Some(levels) = lod::levels_for_draw(
            world,
            id,
            &name,
            definition,
            (&handle, animation.as_ref()),
            meshes,
        ) {
            draw.insert(levels);
        }
        if lit {
            draw.insert(ModelLit {
                entity: Some(id),
                key: instance.asset_key(),
                scale: instance.scale,
                half_lambert: definition.half_lambert,
                last: None,
                dynamic: false,
            });
        }
        if let Some(animation) = animation {
            if let Some(skeleton) = &animation.gpu {
                draw.insert((
                    skeleton.component(),
                    bevy::camera::visibility::DynamicSkinnedMeshBounds,
                ));
            }
            draw.insert(animation);
        }
        draws += 1;
    }
    draws
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
                    view_bounds: Default::default(),
                    pose_parameters: Vec::new(),
                    autoplay: Vec::new(),
                    attachments: Vec::new(),
                    activities: Default::default(),
                    hitboxes: Vec::new(),
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
        let sim = crate::simulation::Simulation::new(
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

    #[test]
    fn image_constructor_accepts_f16_hdr_pixels_without_panicking() {
        let hdr_data = vec![0u8; 16 * 16 * 8]; // 16x16, 8 bytes per pixel (4 * f16)
        let image = Image::new(
            Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            hdr_data,
            TextureFormat::Rgba16Float,
            RenderAssetUsages::RENDER_WORLD,
        );
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba16Float);
    }

    #[test]
    fn helper_creates_rgba8unorm_image_without_panicking() {
        let ldr_data = vec![255u8; 16 * 16 * 4]; // 16x16, 4 bytes per pixel (Rgba8Unorm)
        let img = image(16, 16, ldr_data, false);
        assert_eq!(img.texture_descriptor.format, TextureFormat::Rgba8Unorm);
    }

    #[test]
    #[should_panic(expected = "Pixel data, size and format have to match")]
    fn image_constructor_panics_if_ldr_pixels_are_provided_for_hdr() {
        let ldr_data = vec![0u8; 16 * 16 * 4]; // 4 bytes per pixel, invalid for Rgba16Float
        Image::new(
            Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            ldr_data,
            TextureFormat::Rgba16Float,
            RenderAssetUsages::RENDER_WORLD,
        );
    }

    #[test]
    #[should_panic(expected = "Pixel data, size and format have to match")]
    fn helper_panics_if_hdr_pixels_are_provided_for_rgba8unorm() {
        let hdr_data = vec![0u8; 16 * 16 * 8]; // 8 bytes per pixel, invalid for Rgba8Unorm
        // This is exactly the scenario that caused the original crash!
        image(16, 16, hdr_data, false);
    }
}
