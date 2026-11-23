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
#[derive(Default)]
pub(super) struct SharedLevels {
    built: BTreeMap<LevelsKey, Vec<(f32, Level)>>,
    /// The map's face models; without them every draw with flex references keeps its own levels.
    faces: Option<FaceModels>,
}

/// A model asset, material, scale (bits) and whether the draw is skinned.
type LevelsKey = (String, String, u32, bool);
type FaceModels = BTreeMap<String, std::sync::Arc<source_assets::flexes::FaceModel>>;

impl SharedLevels {
    /// Sharing for a map's draws, given its face models.
    pub(super) fn for_map(faces: &FaceModels) -> Self {
        Self {
            built: BTreeMap::new(),
            faces: Some(faces.clone()),
        }
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
    let flexes = animation.is_some_and(|a| shared.deforms(a));
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
    };
    let built = if flexes {
        coarser.build(meshes)
    } else {
        let key = (
            instance.asset_key(),
            material.to_owned(),
            instance.scale.to_bits(),
            animation.is_some(),
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
                mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
            }
            Level {
                mesh: meshes.add(mesh),
                triangles,
                vertices: self.flexes.then(|| {
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
