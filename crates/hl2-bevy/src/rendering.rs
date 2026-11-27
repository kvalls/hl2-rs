//! Baked-lightmap material, independent of Bevy's PBR lighting model.
use crate::{
    Status,
    assets::{LoadedMap, MaterialData},
    source_to_bevy,
};
use bevy::{
    asset::RenderAssetUsages,
    mesh::Indices,
    prelude::*,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, PrimitiveTopology},
};
use hl2_ui::extensions::RenderBudget;
use modkit_core::Surface;
use std::collections::BTreeMap;

pub(crate) mod actor_poses;
mod animated;
pub use animated::{AnimatedMesh, present_entities};
mod cpu_skin;
pub(crate) mod cubemaps;
pub(crate) mod drawing;
mod face_flex;
mod face_split;
pub use face_flex::FlexModels;
mod lighting;
pub(crate) use lighting::ModelLit;
pub use lighting::{present_lighting, present_ragdoll_fades, present_render_colors};
mod lightstyles;
pub use lightstyles::{LightmapPages, LightmapWritesPlugin, apply_lightstyles};
pub(crate) mod lod;
mod map_draw;
mod map_geometry;
mod material;
pub use material::MaterialKey;
mod mesh_build;
mod model_cache;
mod model_lighting;
pub use model_lighting::ModelLighting;
mod npc_weapon_draws;
mod placement;
pub(crate) use placement::draw_hidden;
pub(crate) mod runtime;
pub(crate) mod simple_pass;
pub(crate) mod variant_warmup;
mod weapons;
use map_draw::{MapDrawContext, insert_map_draw_components};
pub(crate) use map_geometry::MapGeometry;
pub(crate) use material::{
    alpha_mode, black_cube, fallback_images, gpu_image_filtered, image, make_material,
    sampler_descriptor,
};
pub use model_cache::ModelCache;
use model_cache::strip_pixels;
pub use runtime::{despawn_killed_runtime_draws, spawn_runtime_draws, swap_runtime_bodies};
pub use weapons::{WeaponMesh, present_weapons, sync_viewmodel_camera};
pub(super) use weapons::{WeaponSkin, viewmodel_sample};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(MaterialKey)]
pub struct SourceMaterial {
    #[uniform(0)]
    pub(crate) tint: Vec4,
    // x = alpha cutoff, y = ignore base alpha, z = output (1 additive, 2 mod2x), w = second slot
    // (1 iris, 2 WorldVertexTransition base by vertex alpha, -1/-2 camera, -3 two-texture, 0 none).
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
    /// `material_slots`: x material slot, y 1 when it holds the tint colour, z lighting slot (-1 none),
    /// w 1 when detached from a shared lit material for per-draw fade or render color.
    #[uniform(0)]
    pub(crate) dynamic: Vec4,
    // The source fragment shader owns these buffers. Reserving them in the vertex stage
    // consumes Metal slots needed by skinned morph prepasses.
    /// Every eye's iris projection rows ([`crate::eyes::EYE_ROWS`]).
    #[storage(20, read_only, visibility(fragment))]
    pub(crate) eye_rows: Handle<bevy::render::storage::ShaderBuffer>,
    /// Per-frame material values ([`crate::material_slots`]).
    #[storage(21, read_only, visibility(fragment))]
    pub(crate) material_rows: Handle<bevy::render::storage::ShaderBuffer>,
    /// Per-entity model lighting rows ([`crate::material_slots`]).
    #[storage(22, read_only, visibility(fragment))]
    pub(crate) light_rows: Handle<bevy::render::storage::ShaderBuffer>,
    pub(crate) decal_order: usize,
    pub(crate) alpha: AlphaMode,
    pub(crate) two_sided: bool,
}
/// A model draw lit from its entity's (or the view's) illumination origin.
#[derive(Default)]
struct Batch {
    decal_layer: Option<crate::assets::DecalLayer>,
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
        if !self.consistent() || self.positions.iter().flatten().any(|v| !v.is_finite()) {
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
        self.regroup(groups.into_values().collect())
    }
    fn reserve_for(&mut self, surface: &Surface) {
        let verts = surface.vertices.len();
        self.positions.reserve(verts);
        self.uvs.reserve(verts);
        self.light_uvs.reserve(verts);
        self.colors.reserve(verts);
        self.normals.reserve(verts);
        self.skin.reserve(verts);
        self.flex.reserve(verts);
        self.indices.reserve(surface.indices.len());
    }
    fn append(&mut self, surface: &Surface, transform: Mat4, material: Option<&MaterialData>) {
        self.reserve_for(surface);
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
    /// Bevy recomputes a render mesh's bounds from its positions when it uploads the
    /// mesh (`final_aabb` does not survive) and sorts translucent draws by the center
    /// of those bounds. Two unreferenced vertices at the group's corners give every
    /// layer of an overlapping decal group the same sort center, so only the
    /// placement-order bias decides their order, from any view direction.
    fn anchor_decal_layer_bounds(&mut self) {
        let (Some(layer), Some(&position)) = (self.decal_layer, self.positions.first()) else {
            return;
        };
        let (uv, light_uv, color, normal) = (
            self.uvs[0],
            self.light_uvs[0],
            self.colors[0],
            self.normals[0],
        );
        for corner in [layer.bounds.min, layer.bounds.max] {
            debug_assert!(corner.is_finite(), "decal group bounds {position:?}");
            self.positions.push(corner.to_array());
            self.uvs.push(uv);
            self.light_uvs.push(light_uv);
            self.colors.push(color);
            self.normals.push(normal);
        }
    }
    fn mesh(mut self) -> Mesh {
        self.anchor_decal_layer_bounds();
        let count = self.positions.len();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            if self.skin.iter().any(|(_, weights)| weights.is_some()) {
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD
            } else {
                RenderAssetUsages::RENDER_WORLD
            },
        );
        mesh.final_aabb = self.decal_layer.map(|layer| layer.bounds);
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
pub use crate::eyes::EyeMesh;
fn sample_key(clip: &modkit_core::animation::Clip, time: f32) -> u32 {
    if clip.frames.len() <= 1 {
        0
    } else if !clip.looping && time * clip.fps >= (clip.frames.len() - 1) as f32 {
        u32::MAX
    } else {
        time.to_bits()
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

#[allow(clippy::too_many_arguments)]
pub fn spawn_map_filtered(
    loaded: &mut LoadedMap,
    commands: &mut Commands,
    (meshes, inverse_binds): (
        &mut Assets<Mesh>,
        &mut Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>,
    ),
    (materials, slots): (
        &mut Assets<SourceMaterial>,
        &mut crate::material_slots::MaterialSlots,
    ),
    images: &mut Assets<Image>,
    status: &Status,
    (camera_target, cpu_skinning, world_partition): (&Handle<Image>, bool, bool),
    (
        filtering,
        RenderBudget {
            plain_materials,
            nearest,
            ..
        },
    ): hl2_ui::config::TextureSampling,
) {
    // The loader worker built the draw geometry (O49); a map loaded another way builds it here,
    // before the texture upload takes the iris images the private-mesh rule reads.
    let mut geometry = loaded
        .geometry
        .take()
        .filter(|g| g.built_for == (cpu_skinning, world_partition))
        .unwrap_or_else(|| MapGeometry::build(loaded, cpu_skinning, world_partition));
    geometry.record(status, world_partition);
    let (white, black_cube, flashlight, missing) =
        fallback_images(images, loaded.flashlight_cookie.as_deref(), filtering);
    let mut uploaded = material::upload_map_textures(
        loaded,
        images,
        camera_target,
        (filtering, nearest),
        &missing,
    );
    let pages = lightstyles::upload(&loaded.world, images);
    let lightmaps = pages.handles().to_vec();
    commands.insert_resource(pages);
    commands.insert_resource(cubemaps::upload(
        &loaded.cubemaps,
        images,
        &mut uploaded.handles,
        filtering,
    ));
    let prepare_cx = mesh_build::PrepareContext {
        loaded,
        uploaded: &uploaded,
        lightmaps: &lightmaps,
        white: &white,
        black_cube: &black_cube,
        flashlight: &flashlight,
        missing: &missing,
        plain_materials,
        cpu_skinning,
    };
    mesh_build::resolve_draws(
        &mut geometry,
        &prepare_cx,
        commands,
        (materials, slots),
        inverse_binds,
        status,
    );
    let MapGeometry {
        draws,
        weapon_skins,
        mut shared_levels,
        ..
    } = geometry;
    spawn_prepared_draws(
        draws,
        loaded,
        commands,
        (meshes, slots),
        &mut shared_levels,
        (&uploaded.two_textures, &weapon_skins, status),
    );
    commands.insert_resource(ModelCache {
        textures: uploaded.handles,
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
        plain: plain_materials,
        faces: loaded.flexes.clone(),
    });
}

fn spawn_prepared_draws(
    prepared: Vec<mesh_build::PreparedDraw>,
    loaded: &LoadedMap,
    commands: &mut Commands,
    (meshes, slots): (&mut Assets<Mesh>, &mut crate::material_slots::MaterialSlots),
    shared_levels: &mut lod::SharedLevels,
    (two_textures, weapon_skins, status): (
        &map_draw::TwoTextures,
        &mesh_build::WeaponSkins,
        &Status,
    ),
) {
    let world = &loaded.world;
    let fallback = MaterialData::default();
    let (mut gpu_skinned, mut cpu_skinned) = (0usize, 0usize);
    for mut item in prepared {
        let handle =
            shared_levels.take_prebuilt_root(item.root_key.take(), item.animation.as_mut(), meshes);
        let definition = loaded.materials.get(&item.name).unwrap_or(&fallback);
        let state = item
            .owner
            .entity()
            .and_then(|id| loaded.gameplay.scene.states.get(id));
        let transform = state.map_or(Transform::IDENTITY, |s| {
            entity_transform(s.origin, s.rotation)
        });
        let mut draw = commands.spawn((
            crate::campaign::MapOwned,
            Name::new(item.name.clone()),
            Mesh3d(handle.clone()),
            MeshMaterial3d(item.material),
            transform,
        ));
        let cx = MapDrawContext {
            owner: &item.owner,
            name: &item.name,
            triangles: item.triangles,
            definition,
            world,
            loaded,
            two_textures,
            weapon_skins,
        };
        let lit_draw = item.lit.then(|| map_draw::lit_target(&cx));
        let lods = (&mut *meshes, &mut *shared_levels);
        insert_map_draw_components(&mut draw, &cx, &handle, item.animation.as_ref(), lods);
        if let Some((entity, key, scale)) = lit_draw {
            if let Some(slot_key) = item.shared_lit_key
                && let Some(tag) = slots.assign_draw_tag(draw.id(), slot_key)
            {
                draw.insert(bevy::mesh::MeshTag::new(tag));
            }
            draw.insert(
                ModelLit::new(entity, key, scale, definition.half_lambert)
                    .reflecting(item.reflects),
            );
        }
        if let Some(animation) = item.animation {
            if let Some(skeleton) = &animation.gpu {
                gpu_skinned += 1;
                draw.insert((
                    skeleton.component(),
                    bevy::camera::visibility::DynamicSkinnedMeshBounds,
                ));
            } else {
                cpu_skinned += 1;
            }
            animation.insert_into(&mut draw);
        }
    }
    let mut stats = status.0.lock().expect("status lock");
    stats.gpu_skinned_meshes += gpu_skinned;
    stats.cpu_skinned_meshes += cpu_skinned;
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
    body: Option<i32>,
) -> usize {
    let Some(instance) = world.model_instances.iter().find(|i| i.entity == Some(id)) else {
        return 0;
    };
    // A runtime body draws its preloaded variant's surfaces; the rig, lighting and
    // animation stay on the spawn key.
    let surface_key = match body {
        Some(body) => modkit_core::ModelInstance {
            body,
            ..instance.clone()
        }
        .asset_key(),
        None => instance.asset_key(),
    };
    let Some(surfaces) = world.model_assets.get(&surface_key) else {
        return 0;
    };
    let rig = world
        .rigs
        .get(&instance.asset_key())
        .filter(|rig| !rig.bones.is_empty() && rig.bones.len() < 256);
    let batches =
        face_split::model_batches(surfaces, instance.scale, cache, (world, id), rig.is_none());
    let modulation = entity_modulation(world, id);
    let mut skeleton: Option<crate::gpu_skinning::Skeleton> = None;
    let mut shared_levels = lod::SharedLevels::for_map(&cache.faces);
    let mut draws = 0;
    for (name, mut batch) in batches {
        let fallback = MaterialData::default();
        let definition = cache.definitions.get(&name).unwrap_or(&fallback);
        let lit = batch.model && !definition.unlit && !definition.camera;
        let material = materials.add(runtime::draw_material(cache, definition, lit, modulation));
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
        let handle = shared_levels.mesh_for_draw(batch, &name, animation.as_mut(), meshes, false);
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
            (&mut *meshes, &mut shared_levels),
        ) {
            draw.insert(levels);
        }
        if lit {
            draw.insert(cache.model_lit(id, instance, definition));
        }
        if let Some(animation) = animation {
            if let Some(skeleton) = &animation.gpu {
                draw.insert((
                    skeleton.component(),
                    bevy::camera::visibility::DynamicSkinnedMeshBounds,
                ));
            }
            animation.insert_into(&mut draw);
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
    use bevy::camera::primitives::{Aabb, MeshAabb};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    use std::sync::Arc;
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
            decal_layer: None,
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
                .any(|b| b.mesh().get_aabb().unwrap().half_extents.x > 2000.)
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
                    bone_surfaces: Vec::new(),
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
            .insert_resource(meshes)
            .init_resource::<actor_poses::ActorPoses>();
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
                    vertices: Arc::new(lod::Vertices {
                        normals: Vec::new(),
                        flex: Vec::new(),
                        bind: vec![(
                            glam::Vec3::X,
                            Some(Weights {
                                bones: [0; 3],
                                weights: [1., 0., 0.],
                            }),
                        )],
                    }),
                    deformed: None,
                    morph: None,
                    flex_state: Default::default(),
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
        // DecalModulate multiplies the frame (decals/glass/shot*).
        material.modulate = true;
        assert_eq!(alpha_mode(&material), AlphaMode::Multiply);
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
