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

/// The per-vertex data a facial-flex draw's [`AnimatedMesh`] deforms, for one level.
#[derive(Clone)]
struct Vertices {
    bind: Vec<(glam::Vec3, Option<modkit_core::animation::Weights>)>,
    normals: Vec<glam::Vec3>,
    flex: Vec<Option<[u16; 3]>>,
    flex_base: Vec<glam::Vec3>,
}

#[derive(Clone)]
struct Level {
    mesh: Handle<Mesh>,
    vertices: Option<Vertices>,
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

/// The coarser levels of an entity's draw of `material`, `root` being the draw's mesh and its
/// animation; `None` when the model has no coarser level or the draw keeps the root LOD.
pub(super) fn levels_for_draw(
    world: &World,
    id: usize,
    material: &str,
    definition: &MaterialData,
    (root, animation): (&Handle<Mesh>, Option<&AnimatedMesh>),
    meshes: &mut Assets<Mesh>,
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
    let flexes = animation.is_some_and(|a| !a.flex.is_empty());
    let root_vertices = animation.filter(|_| flexes).map(|a| Vertices {
        bind: a.bind.clone(),
        normals: a.normals.clone(),
        flex: a.flex.clone(),
        flex_base: a.flex_base.clone(),
    });
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
    let scale = Mat4::from_scale(Vec3::splat(instance.scale));
    for lod in lods {
        let mut batch = Batch::default();
        for surface in lod.surfaces.iter().filter(|s| s.material == material) {
            batch.append(surface, scale, Some(definition));
        }
        let triangles = batch.indices.len() / 3;
        if triangles == 0 {
            batch.append(&nothing(first), scale, Some(definition));
        }
        let skin = std::mem::take(&mut batch.skin);
        let flex = std::mem::take(&mut batch.flex);
        let normals = batch.normals.clone();
        let mut mesh = batch.mesh();
        if let Some(animation) = animation {
            if let Some(skeleton) = &animation.gpu {
                skeleton.attributes(&mut mesh, &skin);
            }
            mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        }
        levels.switches.push(lod.switch_point);
        levels.levels.push(Level {
            mesh: meshes.add(mesh),
            triangles,
            vertices: flexes.then(|| Vertices {
                flex_base: skin.iter().map(|(p, _)| *p).collect(),
                bind: skin,
                normals,
                flex,
            }),
        });
    }
    Some(levels)
}

type Camera<'a> = (&'a Transform, &'a Projection);
type Draw<'a> = (
    &'a Transform,
    &'a mut ModelLevels,
    &'a mut Mesh3d,
    Option<&'a mut AnimatedMesh>,
    Option<&'a mut super::DrawTriangles>,
);

/// Each entity draw shows the level its depth in the world camera's view asks for.
pub(crate) fn switch_levels(
    camera: Query<Camera, With<crate::FlyCamera>>,
    frame: Option<Res<crate::video::GameFrame>>,
    mut draws: Query<Draw>,
) {
    let Ok((camera, projection)) = camera.single() else {
        return;
    };
    let tan_half_fov = match projection {
        Projection::Perspective(p) => (p.fov / 2.).tan(),
        _ => return,
    };
    let height = frame.map_or(768., |f| f.resolution.y as f32);
    let forward = camera.forward().as_vec3();
    for (transform, mut levels, mut mesh, animation, triangles) in &mut draws {
        let depth = (transform.translation - camera.translation).dot(forward);
        let wanted = level_for(&levels.switches, metric(depth, tan_half_fov, height));
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
            animation.bind = vertices.bind.clone();
            animation.normals = vertices.normals.clone();
            animation.flex = vertices.flex.clone();
            animation.flex_base = vertices.flex_base.clone();
            // The new mesh starts undeformed: rebuild its flexes.
            animation.flex_signature = 0;
        }
    }
}

#[cfg(test)]
mod tests;
