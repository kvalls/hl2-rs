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
use std::{collections::BTreeMap, sync::Arc};

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
type LevelsKey = (String, String, u32, bool, bool);
type RootKey = (String, String, u32, usize, usize, usize);
type FaceModels = BTreeMap<String, std::sync::Arc<source_assets::flexes::FaceModel>>;

#[derive(Default)]
pub(super) struct SharedLevels {
    built: BTreeMap<LevelsKey, Vec<(f32, Level)>>,
    roots: BTreeMap<RootKey, (Handle<Mesh>, Option<Arc<super::face_flex::MorphChannels>>)>,
    /// The map's face models; without them every draw with flex references keeps its own levels.
    faces: Option<FaceModels>,
}

impl SharedLevels {
    /// Sharing for a map's draws, given its face models.
    pub(super) fn for_map(faces: &FaceModels) -> Self {
        Self {
            built: BTreeMap::new(),
            roots: BTreeMap::new(),
            faces: Some(faces.clone()),
        }
    }

    fn face(&self, key: &str) -> Option<&source_assets::flexes::FaceModel> {
        self.faces.as_ref()?.get(key).map(Arc::as_ref)
    }

    fn face_arc(&self, key: &str) -> Option<Arc<source_assets::flexes::FaceModel>> {
        self.faces.as_ref()?.get(key).cloned()
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
        let index_count = batch.indices.len();
        let root_key = (!private_mesh)
            .then(|| {
                let anim = animation.as_deref()?;
                let skeleton = anim.gpu.as_ref()?;
                Some((
                    anim.key.clone(),
                    name.to_owned(),
                    anim.scale.to_bits(),
                    anim.vertices.bind.len(),
                    index_count,
                    skeleton.joints.len(),
                ))
            })
            .flatten();
        if let Some(key) = &root_key
            && let Some((handle, morph)) = self.roots.get(key)
        {
            if let Some(anim) = animation {
                anim.morph = morph.clone();
            }
            return handle.clone();
        }
        let mut mesh = batch.mesh();
        let mut built_morph = None;
        let mut cpu_flex = false;
        if let Some(anim) = animation {
            if let Some(skeleton) = &anim.gpu {
                skeleton.attributes(&mut mesh, &anim.vertices.bind);
                if !anim.vertices.flex.is_empty()
                    && let Some(face) = self.face(&anim.key)
                {
                    built_morph = super::face_flex::build_morph_targets(
                        &mut mesh,
                        face,
                        &anim.vertices.flex,
                        anim.scale,
                    )
                    .map(Arc::new);
                    anim.morph = built_morph.clone();
                    cpu_flex = built_morph.is_none() && self.deforms(anim);
                }
            }
            mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        } else if private_mesh {
            mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        }
        let handle = meshes.add(mesh);
        // CPU flexes mutate mesh positions independently for each actor.
        if let Some(key) = root_key.filter(|_| !cpu_flex) {
            self.roots.insert(key, (handle.clone(), built_morph));
        }
        handle
    }

    /// Whether facial flexes deform the draw: its model's flexes move a studio mesh one of its
    /// vertices comes from (every studio vertex carries a reference to its source mesh).
    fn deforms(&self, animation: &AnimatedMesh) -> bool {
        if animation.vertices.flex.is_empty() {
            return false;
        }
        let Some(faces) = &self.faces else {
            return true;
        };
        let Some(face) = faces.get(&animation.key) else {
            return false;
        };
        let flexed: std::collections::BTreeSet<(usize, usize)> = (face.flex.meshes.iter())
            .map(|m| (m.bodypart, m.mesh))
            .collect();
        (animation.vertices.flex.iter().flatten())
            .any(|[bodypart, mesh, _]| flexed.contains(&((*bodypart).into(), (*mesh).into())))
    }
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
    let lods = world.model_lods.get(&instance.asset_key())?;
    let first = world
        .model_assets
        .get(&instance.asset_key())?
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
            triangles: world
                .model_assets
                .get(&instance.asset_key())?
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
        animation,
        flexes,
        morph,
        face: face.as_deref(),
    };
    let built = if flexes {
        coarser.build(meshes)
    } else {
        let key = (
            instance.asset_key(),
            material.to_owned(),
            instance.scale.to_bits(),
            animation.is_some(),
            morph.is_some(),
        );
        (shared.built.entry(key))
            .or_insert_with(|| coarser.build(meshes))
            .clone()
    };
    for (switch, level) in built {
        levels.switches.push(switch);
        levels.levels.push(level);
    }
    Some(levels)
}

/// What a draw's coarser levels are built from.
struct Coarser<'a> {
    lods: &'a [modkit_core::ModelLod],
    material: &'a str,
    definition: &'a MaterialData,
    /// A surface of the material, shown collapsed at a level without its triangles.
    first: &'a Surface,
    scale: f32,
    animation: Option<&'a AnimatedMesh>,
    flexes: bool,
    morph: Option<&'a super::face_flex::MorphChannels>,
    face: Option<&'a source_assets::flexes::FaceModel>,
}

impl Coarser<'_> {
    /// Each coarser level's switch point and level, finest first.
    fn build(&self, meshes: &mut Assets<Mesh>) -> Vec<(f32, Level)> {
        let scale = Mat4::from_scale(Vec3::splat(self.scale));
        let level = |lod: &modkit_core::ModelLod, meshes: &mut Assets<Mesh>| {
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
            if let Some(animation) = self.animation {
                if let Some(skeleton) = &animation.gpu {
                    skeleton.attributes(&mut mesh, &skin);
                }
                if let Some(channels) = self.morph {
                    super::face_flex::apply_morph_channels(
                        &mut mesh, channels, self.face, &flex, self.scale,
                    );
                }
                mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
            }
            Level {
                mesh: meshes.add(mesh),
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
            .map(|lod| (lod.switch_point, level(lod, meshes)))
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
    mut draws: Query<Draw>,
) {
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
    for (transform, mut levels, mut mesh, animation, triangles) in &mut draws {
        let at = transform.translation;
        let wanted = view.level(&levels.switches, at);
        let wanted = monitor
            .as_ref()
            .map_or(wanted, |m| wanted.min(m.level(&levels.switches, at)));
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
