//! Studio models' levels of detail. Retail `CModelRender::ComputeLOD` (with `r_lod -1`) picks the
//! level from `100 / ComputePixelWidthOfSphere(render origin, 0.5)`; that width is twice the
//! sphere's pixel diameter in the current viewport (Source SDK `imaterialsystem.h`), so the
//! metric is `100 * depth * tan(vfov / 2) / viewport height` (the model's size does not enter),
//! and `studiohwdata_t::GetLODForMetric` (SDK `public/studio.h`) takes the last level whose switch
//! point the metric reached, never finer than the root LOD (model detail).
//!
//! Each map or runtime entity draw (one mesh per material) keeps its coarser levels' meshes for
//! the same material ([`ModelLevels`]) and swaps its `Mesh3d` handle when its level changes; a
//! facial-flex draw also swaps the per-vertex data its [`AnimatedMesh`] deforms. Static props are
//! baked into the world's batches and keep the root LOD; eye meshes, CPU-skinned meshes,
//! viewmodels and NPC weapons keep it too.

use super::{AnimatedMesh, Batch, MaterialData};
use bevy::{
    asset::{Assets, Handle, RenderAssetUsages},
    camera::Projection,
    ecs::{
        component::Component,
        query::With,
        system::{Query, Res},
    },
    math::{Mat4, Vec3},
    mesh::{Mesh, Mesh3d},
    transform::components::Transform,
};
use modkit_core::{Surface, World};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

/// The LOD metric of a point at view depth `depth` (units along the view direction) for a view
/// with `tan_half_fov` (tangent of half the vertical field of view) and `height` pixels: the
/// projected width below 0.001 depth is taken at 0.001 (the material system's guard).
pub fn metric(depth: f32, tan_half_fov: f32, height: f32) -> f32 {
    100. * depth.max(0.001) * tan_half_fov / height.max(1.)
}

/// `GetLODForMetric` over levels from the root: `switches[0]` is the root's (unused).
pub fn level_for(switches: &[f32], metric: f32) -> usize {
    (0..switches.len().saturating_sub(1))
        .find(|&i| switches[i + 1] > metric)
        .unwrap_or(switches.len().saturating_sub(1))
}

/// The immutable per-vertex data a facial-flex draw's [`AnimatedMesh`] reads, for one level.
pub(super) struct Vertices {
    pub(super) bind: Vec<(glam::Vec3, Option<modkit_core::animation::Weights>)>,
    pub(super) normals: Vec<glam::Vec3>,
    pub(super) flex: Vec<Option<[u16; 3]>>,
}

#[derive(Clone)]
struct Level {
    mesh: Handle<Mesh>,
    vertices: Option<Arc<Vertices>>,
    /// Triangles drawn at this level (render statistics).
    triangles: usize,
}

/// A draw's levels of detail from the root LOD down and the level it draws.
#[derive(Component, Clone)]
pub struct ModelLevels {
    switches: Vec<f32>,
    levels: Vec<Level>,
    current: usize,
}

impl ModelLevels {
    /// A copy for another entity's draw (a runtime spawn from a template): meshes the draw
    /// deforms itself are copied, the rest shared.
    pub(super) fn copy_for(&self, meshes: &mut Assets<Mesh>, private: bool) -> Option<Self> {
        let mut copy = self.clone();
        copy.current = 0;
        if private {
            for level in &mut copy.levels {
                level.mesh = meshes.add(meshes.get(&level.mesh)?.clone());
            }
        }
        Some(copy)
    }

    /// The root level's mesh (the draw's own after a copy).
    pub(super) fn root(&self) -> Handle<Mesh> {
        self.levels[0].mesh.clone()
    }
}

/// A mesh of one triangle collapsed to a point: what a draw shows at a level whose model has no
/// triangles of its material (`removemesh` / `blank` in the model's `$lod`).
fn nothing(surface: &Surface) -> Surface {
    let vertex = surface.vertices[0].clone();
    Surface {
        vertices: vec![vertex.clone(), vertex.clone(), vertex],
        indices: vec![0, 1, 2],
        flex_source: None,
        ..surface.clone()
    }
}

/// Coarser levels already built, by model asset, material, scale and skinning: map entity draws
/// of one model share these meshes, as runtime copies do, unless facial flexes deform them. The
/// skin attributes depend only on the model's weights and its rig's bone count.
/// A model asset, material, scale (bits), whether the draw is skinned, and whether GPU morphs
/// are attached.
pub(super) type LevelsKey = (String, String, u32, bool, bool);
pub(super) type RootKey = (String, String, u32, usize, usize, usize);
type FaceModels = BTreeMap<String, std::sync::Arc<source_assets::flexes::FaceModel>>;

pub(super) struct BuiltRoot {
    pub(super) mesh: Mesh,
    pub(super) morph: Option<Arc<super::face_flex::MorphChannels>>,
    pub(super) cpu_flex: bool,
}

pub(super) struct RootBuildInput<'a> {
    pub(super) batch: Batch,
    pub(super) joint_count: Option<usize>,
    pub(super) vertices: Option<&'a Vertices>,
    pub(super) scale: f32,
    pub(super) face: Option<&'a source_assets::flexes::FaceModel>,
    pub(super) has_animation: bool,
    pub(super) private_mesh: bool,
}

pub(super) struct BuiltLevel {
    pub(super) mesh: Mesh,
    pub(super) vertices: Option<Arc<Vertices>>,
    pub(super) triangles: usize,
}

pub(super) type BuiltLevels = Vec<(f32, BuiltLevel)>;
pub(super) type PrivateBuiltLevels = VecDeque<(usize, String, BuiltLevels)>;

pub(super) struct PrebuiltMeshes {
    pub(super) roots_shared: BTreeMap<RootKey, (BuiltRoot, usize)>,
    pub(super) roots_private: VecDeque<BuiltRoot>,
    pub(super) levels_shared: BTreeMap<LevelsKey, BuiltLevels>,
    pub(super) levels_private: PrivateBuiltLevels,
}

pub(super) fn root_key(
    name: &str,
    index_count: usize,
    animation: Option<&AnimatedMesh>,
    private_mesh: bool,
) -> Option<RootKey> {
    (!private_mesh).then(|| {
        let anim = animation?;
        let skeleton = anim.gpu.as_ref()?;
        Some((
            anim.key.clone(),
            name.to_owned(),
            anim.scale.to_bits(),
            anim.vertices.bind.len(),
            index_count,
            skeleton.joints.len(),
        ))
    })?
}

pub(super) fn levels_key(
    asset_key: String,
    material: String,
    scale: f32,
    has_animation: bool,
    has_morph: bool,
) -> LevelsKey {
    (
        asset_key,
        material,
        scale.to_bits(),
        has_animation,
        has_morph,
    )
}

pub(super) fn apply_skin_attributes(
    mesh: &mut Mesh,
    vertices: &[(glam::Vec3, Option<modkit_core::animation::Weights>)],
    joint_count: usize,
) {
    let (indices, weights): (Vec<_>, Vec<_>) = vertices
        .iter()
        .map(|(_, weights)| crate::gpu_skinning::influences(weights.as_ref(), joint_count - 1))
        .unzip();
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        bevy::mesh::VertexAttributeValues::Uint16x4(indices),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
    mesh.generate_skinned_mesh_bounds()
        .expect("validated Source skin influences");
}

pub(super) fn face_deforms(
    face: &source_assets::flexes::FaceModel,
    flex: &[Option<[u16; 3]>],
) -> bool {
    if flex.is_empty() {
        return false;
    }
    let flexed: std::collections::BTreeSet<(usize, usize)> = face
        .flex
        .meshes
        .iter()
        .map(|m| (m.bodypart, m.mesh))
        .collect();
    flex.iter()
        .flatten()
        .any(|[bodypart, mesh, _]| flexed.contains(&((*bodypart).into(), (*mesh).into())))
}

pub(super) fn build_root_mesh(input: RootBuildInput<'_>) -> BuiltRoot {
    let mut mesh = input.batch.mesh();
    let mut built_morph = None;
    let mut cpu_flex = false;
    if input.has_animation {
        if let (Some(joint_count), Some(vertices)) = (input.joint_count, input.vertices) {
            apply_skin_attributes(&mut mesh, &vertices.bind, joint_count);
            if !vertices.flex.is_empty()
                && let Some(face) = input.face
            {
                built_morph = super::face_flex::build_morph_targets(
                    &mut mesh,
                    face,
                    &vertices.flex,
                    input.scale,
                )
                .map(Arc::new);
                cpu_flex = built_morph.is_none() && face_deforms(face, &vertices.flex);
            }
        }
        mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
    } else if input.private_mesh {
        mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
    }
    BuiltRoot {
        mesh,
        morph: built_morph,
        cpu_flex,
    }
}

#[derive(Default)]
pub(super) struct SharedLevels {
    built: BTreeMap<LevelsKey, Vec<(f32, Level)>>,
    roots: BTreeMap<RootKey, (Handle<Mesh>, Option<Arc<super::face_flex::MorphChannels>>)>,
    /// The map's face models; without them every draw with flex references keeps its own levels.
    faces: Option<FaceModels>,
    prebuilt: Option<PrebuiltMeshes>,
}

impl SharedLevels {
    /// Sharing for a map's draws, given its face models.
    pub(super) fn for_map(faces: &FaceModels) -> Self {
        Self {
            built: BTreeMap::new(),
            roots: BTreeMap::new(),
            faces: Some(faces.clone()),
            prebuilt: None,
        }
    }

    pub(super) fn set_prebuilt(&mut self, prebuilt: PrebuiltMeshes) {
        self.prebuilt = Some(prebuilt);
    }

    fn face(&self, key: &str) -> Option<&source_assets::flexes::FaceModel> {
        self.faces.as_ref()?.get(key).map(Arc::as_ref)
    }

    pub(super) fn face_arc(&self, key: &str) -> Option<Arc<source_assets::flexes::FaceModel>> {
        self.faces.as_ref()?.get(key).cloned()
    }

    pub(super) fn take_prebuilt_root(
        &mut self,
        root_key: Option<RootKey>,
        animation: Option<&mut AnimatedMesh>,
        meshes: &mut Assets<Mesh>,
    ) -> Handle<Mesh> {
        if let Some(key) = &root_key
            && let Some((handle, morph)) = self.roots.get(key)
        {
            if let Some(anim) = animation {
                anim.morph = morph.clone();
            }
            return handle.clone();
        }
        let prebuilt = self
            .prebuilt
            .as_mut()
            .expect("prebuilt meshes populated before map spawn");
        let built = if let Some(key) = &root_key {
            let clone_needed = {
                let entry = prebuilt
                    .roots_shared
                    .get_mut(key)
                    .expect("prebuilt shared root entry");
                entry.1 -= 1;
                entry.0.cpu_flex && entry.1 > 0
            };
            if clone_needed {
                let (src, _) = &prebuilt.roots_shared[key];
                BuiltRoot {
                    mesh: src.mesh.clone(),
                    morph: src.morph.clone(),
                    cpu_flex: src.cpu_flex,
                }
            } else {
                prebuilt.roots_shared.remove(key).unwrap().0
            }
        } else {
            prebuilt
                .roots_private
                .pop_front()
                .expect("prebuilt private root entry")
        };
        let handle = meshes.add(built.mesh);
        if let Some(anim) = animation {
            anim.morph = built.morph.clone();
        }
        if let Some(key) = root_key.filter(|_| !built.cpu_flex) {
            self.roots.insert(key, (handle.clone(), built.morph));
        }
        handle
    }

    /// Build or share a root draw's mesh, attaching GPU skinning and GPU facial morph targets.
    pub(super) fn mesh_for_draw(
        &mut self,
        batch: Batch,
        name: &str,
        animation: Option<&mut AnimatedMesh>,
        meshes: &mut Assets<Mesh>,
        private_mesh: bool,
    ) -> Handle<Mesh> {
        let root_key = root_key(
            name,
            batch.indices.len(),
            animation.as_deref(),
            private_mesh,
        );
        if let Some(key) = &root_key
            && let Some((handle, morph)) = self.roots.get(key)
        {
            if let Some(anim) = animation {
                anim.morph = morph.clone();
            }
            return handle.clone();
        }
        let (has_animation, joint_count, vertices, scale, face) = match animation.as_deref() {
            Some(anim) => (
                true,
                anim.gpu.as_ref().map(|s| s.joints.len()),
                Some(Arc::clone(&anim.vertices)),
                anim.scale,
                self.face_arc(&anim.key),
            ),
            None => (false, None, None, 1., None),
        };
        let built = build_root_mesh(RootBuildInput {
            batch,
            joint_count,
            vertices: vertices.as_deref(),
            scale,
            face: face.as_deref(),
            has_animation,
            private_mesh,
        });
        let cpu_flex = built.cpu_flex
            || (has_animation
                && built.morph.is_none()
                && self.faces.is_none()
                && vertices.as_ref().is_some_and(|v| !v.flex.is_empty()));
        let handle = meshes.add(built.mesh);
        if let Some(anim) = animation {
            anim.morph = built.morph.clone();
        }
        // CPU flexes mutate mesh positions independently for each actor.
        if let Some(key) = root_key.filter(|_| !cpu_flex) {
            self.roots.insert(key, (handle.clone(), built.morph));
        }
        handle
    }

    /// Whether facial flexes deform the draw: its model's flexes move a studio mesh one of its
    /// vertices comes from (every studio vertex carries a reference to its source mesh).
    pub(super) fn deforms(&self, animation: &AnimatedMesh) -> bool {
        if animation.vertices.flex.is_empty() {
            return false;
        }
        let Some(face) = self.face(&animation.key) else {
            return self.faces.is_none();
        };
        face_deforms(face, &animation.vertices.flex)
    }
}

fn upload_levels(built: Vec<(f32, BuiltLevel)>, meshes: &mut Assets<Mesh>) -> Vec<(f32, Level)> {
    built
        .into_iter()
        .map(|(switch, level)| {
            (
                switch,
                Level {
                    mesh: meshes.add(level.mesh),
                    vertices: level.vertices,
                    triangles: level.triangles,
                },
            )
        })
        .collect()
}

fn resolve_coarser_levels(
    id: usize,
    asset_key: String,
    coarser: Coarser<'_>,
    meshes: &mut Assets<Mesh>,
    shared: &mut SharedLevels,
) -> Vec<(f32, Level)> {
    if coarser.flexes {
        let raw = if let Some(prebuilt) = shared.prebuilt.as_mut() {
            let (expected_id, expected_mat, levels) = prebuilt
                .levels_private
                .pop_front()
                .expect("prebuilt private levels entry");
            debug_assert_eq!((expected_id, expected_mat.as_str()), (id, coarser.material));
            levels
        } else {
            coarser.build_meshes()
        };
        return upload_levels(raw, meshes);
    }
    let key = levels_key(
        asset_key,
        coarser.material.to_owned(),
        coarser.scale,
        coarser.has_animation,
        coarser.morph.is_some(),
    );
    if let Some(cached) = shared.built.get(&key) {
        return cached.clone();
    }
    let raw = if let Some(prebuilt) = shared.prebuilt.as_mut() {
        prebuilt
            .levels_shared
            .remove(&key)
            .expect("prebuilt shared levels entry")
    } else {
        coarser.build_meshes()
    };
    let built = upload_levels(raw, meshes);
    shared.built.insert(key, built.clone());
    built
}

/// The coarser levels of an entity's draw of `material`, `root` being the draw's mesh and its
/// animation; `None` when the model has no coarser level or the draw keeps the root LOD.
pub(super) fn levels_for_draw(
    world: &World,
    id: usize,
    material: &str,
    definition: &MaterialData,
    (root, animation): (&Handle<Mesh>, Option<&AnimatedMesh>),
    (meshes, shared): (&mut Assets<Mesh>, &mut SharedLevels),
) -> Option<ModelLevels> {
    if animation.is_some_and(|a| a.gpu.is_none() || a.weapon.is_some()) {
        return None;
    }
    let instance = world
        .model_instances
        .iter()
        .find(|i| i.entity == Some(id))?;
    let asset_key = instance.asset_key();
    let lods = world.model_lods.get(&asset_key)?;
    let surfaces = world.model_assets.get(&asset_key)?;
    let first = surfaces
        .iter()
        .find(|s| s.material == material && !s.vertices.is_empty())?;
    let morph = animation.and_then(|a| a.morph.as_deref());
    let flexes = morph.is_none() && animation.is_some_and(|a| shared.deforms(a));
    let face = morph.and_then(|_| animation.and_then(|a| shared.face_arc(&a.key)));
    let root_vertices = animation
        .filter(|_| flexes)
        .map(|a| Arc::clone(&a.vertices));
    let mut levels = ModelLevels {
        switches: vec![0.],
        levels: vec![Level {
            mesh: root.clone(),
            vertices: root_vertices,
            triangles: surfaces
                .iter()
                .filter(|s| s.material == material)
                .map(|s| s.indices.len() / 3)
                .sum(),
        }],
        current: 0,
    };
    let coarser = Coarser {
        lods,
        material,
        definition,
        first,
        scale: instance.scale,
        joint_count: animation.and_then(|a| a.gpu.as_ref().map(|s| s.joints.len())),
        has_animation: animation.is_some(),
        flexes,
        morph,
        face: face.as_deref(),
    };
    for (switch, level) in resolve_coarser_levels(id, asset_key, coarser, meshes, shared) {
        levels.switches.push(switch);
        levels.levels.push(level);
    }
    Some(levels)
}

/// What a draw's coarser levels are built from.
pub(super) struct Coarser<'a> {
    pub(super) lods: &'a [modkit_core::ModelLod],
    pub(super) material: &'a str,
    pub(super) definition: &'a MaterialData,
    /// A surface of the material, shown collapsed at a level without its triangles.
    pub(super) first: &'a Surface,
    pub(super) scale: f32,
    pub(super) joint_count: Option<usize>,
    pub(super) has_animation: bool,
    pub(super) flexes: bool,
    pub(super) morph: Option<&'a super::face_flex::MorphChannels>,
    pub(super) face: Option<&'a source_assets::flexes::FaceModel>,
}

impl Coarser<'_> {
    /// Each coarser level's switch point and level, finest first.
    pub(super) fn build_meshes(&self) -> Vec<(f32, BuiltLevel)> {
        let scale = Mat4::from_scale(Vec3::splat(self.scale));
        let level = |lod: &modkit_core::ModelLod| {
            let mut batch = Batch::default();
            for surface in lod.surfaces.iter().filter(|s| s.material == self.material) {
                batch.append(surface, scale, Some(self.definition));
            }
            let triangles = batch.indices.len() / 3;
            if triangles == 0 {
                batch.append(&nothing(self.first), scale, Some(self.definition));
            }
            let skin = std::mem::take(&mut batch.skin);
            let flex = std::mem::take(&mut batch.flex);
            let normals = batch.normals.clone();
            let mut mesh = batch.mesh();
            if self.has_animation {
                if let Some(joint_count) = self.joint_count {
                    apply_skin_attributes(&mut mesh, &skin, joint_count);
                }
                if let Some(channels) = self.morph {
                    super::face_flex::apply_morph_channels(
                        &mut mesh, channels, self.face, &flex, self.scale,
                    );
                }
                mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
            }
            BuiltLevel {
                mesh,
                triangles,
                vertices: (self.flexes && self.morph.is_none()).then(|| {
                    Arc::new(Vertices {
                        bind: skin,
                        normals,
                        flex,
                    })
                }),
            }
        };
        (self.lods.iter())
            .map(|lod| (lod.switch_point, level(lod)))
            .collect()
    }
}

type Camera<'a> = (&'a Transform, &'a Projection);
type Draw<'a> = (
    &'a Transform,
    &'a mut ModelLevels,
    &'a mut Mesh3d,
    Option<&'a mut AnimatedMesh>,
    Option<&'a mut super::DrawTriangles>,
);

/// A view the levels are chosen for: its origin, forward and LOD scale.
struct View {
    origin: Vec3,
    forward: Vec3,
    tan_half_fov: f32,
    height: f32,
}
impl View {
    fn of(transform: &Transform, projection: &Projection, height: f32) -> Option<Self> {
        let Projection::Perspective(p) = projection else {
            return None;
        };
        Some(Self {
            origin: transform.translation,
            forward: transform.forward().as_vec3(),
            tan_half_fov: (p.fov / 2.).tan(),
            height,
        })
    }
    fn level(&self, switches: &[f32], at: Vec3) -> usize {
        let depth = (at - self.origin).dot(self.forward);
        level_for(switches, metric(depth, self.tan_half_fov, self.height))
    }
}

/// Each entity draw shows the level its depth asks for in the views drawn this frame: the
/// world camera's and, while it renders `_rt_Camera`, the monitor camera's (256 pixels
/// tall). Source picks a level per view; a draw has one mesh here, so the finer level
/// wins: Breen before the broadcast camera keeps his close-up detail on the station's
/// screens while the player is far away.
pub(crate) fn switch_levels(
    camera: Query<Camera, With<crate::FlyCamera>>,
    monitor: Query<(Camera, &bevy::camera::Camera), With<crate::monitors::MonitorCamera>>,
    frame: Option<Res<crate::video::GameFrame>>,
    isolation: Option<Res<crate::isolation::Isolation>>,
    mut draws: Query<Draw>,
) {
    let forced = isolation.and_then(|i| i.force_lod);
    let views = match forced {
        Some(_) => None,
        None => {
            let Ok((camera, projection)) = camera.single() else {
                return;
            };
            let height = frame.map_or(768., |f| f.resolution.y as f32);
            let Some(view) = View::of(camera, projection, height) else {
                return;
            };
            let monitor = monitor
                .single()
                .ok()
                .filter(|(_, camera)| camera.is_active)
                .and_then(|((t, p), _)| View::of(t, p, crate::monitors::SIZE as f32));
            Some((view, monitor))
        }
    };
    for (transform, mut levels, mut mesh, animation, triangles) in &mut draws {
        let wanted = if let Some(lod) = forced {
            lod.min(levels.levels.len().saturating_sub(1))
        } else if let Some((view, monitor)) = &views {
            let at = transform.translation;
            let wanted = view.level(&levels.switches, at);
            monitor
                .as_ref()
                .map_or(wanted, |m| wanted.min(m.level(&levels.switches, at)))
        } else {
            continue;
        };
        if wanted == levels.current {
            continue;
        }
        levels.current = wanted;
        let level = &levels.levels[wanted];
        mesh.0 = level.mesh.clone();
        if let Some(mut triangles) = triangles {
            triangles.0 = level.triangles;
        }
        if let (Some(mut animation), Some(vertices)) = (animation, &level.vertices) {
            animation.vertices = Arc::clone(vertices);
            animation.deformed = None;
            // The new mesh starts undeformed: rebuild its flexes.
            animation.flex_state = Default::default();
        }
    }
}

#[cfg(test)]
mod tests;
