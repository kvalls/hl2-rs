//! Sprite quads drawn as one mesh: the additive quads of one material, or a run of consecutive
//! quads of one material.
//!
//! A quad that faces the camera changes every frame, and every changed mesh is uploaded
//! again; one mesh per quad makes that one upload per sprite. Quads that blend by pure
//! addition commute, so those of one material can share a mesh without changing the image
//! among themselves; such a batch is sorted against other translucent draws as one object. A run
//! keeps its quads in input order inside one draw.
use super::{EffectMaterial, Geometry, gpu};
use bevy::{
    camera::primitives::{Aabb, MeshAabb},
    mesh::{MeshTag, VertexAttributeValues},
    prelude::{
        Assets, Commands, Entity, Handle, Mesh, Mesh3d, MeshMaterial3d, Transform, Visibility,
    },
};
use hl2_simulation::projectile_visuals::Quad;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Batch {
    pub(super) entity: Entity,
    pub(super) mesh: Handle<Mesh>,
    tag: u32,
    bounds: Aabb,
    material: Handle<EffectMaterial>,
    pending_material: Handle<EffectMaterial>,
    /// Quads gathered for the current frame.
    next: Vec<Geometry>,
    visible: bool,
    quad_count: usize,
    pub(super) cpu_mesh: Option<Handle<Mesh>>,
    cpu_drawn: Vec<Geometry>,
}

/// The batches of one layer, by material name.
#[derive(Default)]
pub(in super::super) struct Batches {
    by_material: BTreeMap<String, Batch>,
    /// The most quads drawn in one frame.
    most: usize,
}

pub(super) fn batch_mesh(quads: &[Geometry]) -> Mesh {
    let corners = |f: fn(&Geometry) -> [[f32; 3]; 4]| quads.iter().flat_map(f).collect();
    super::super::mesh(
        corners(Geometry::bevy_positions),
        quads.iter().flat_map(|quad| quad.uv).collect(),
        quads
            .iter()
            .flat_map(|quad| [quad.unit_color(); 4])
            .collect(),
        (0..quads.len() as u32)
            .flat_map(|quad| [0, 1, 2, 0, 2, 3].map(|corner| quad * 4 + corner))
            .collect(),
        true,
    )
}

/// Writes the quads over a mesh that holds as many; false when its storage does not fit.
pub(super) fn patch(mesh: &mut Mesh, quads: &[Geometry]) -> bool {
    let count = quads.len() * 4;
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    else {
        return false;
    };
    if positions.len() != count {
        return false;
    }
    for (corners, quad) in positions.as_chunks_mut::<4>().0.iter_mut().zip(quads) {
        *corners = quad.bevy_positions();
    }
    let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0)
    else {
        return false;
    };
    if uvs.len() != count {
        return false;
    }
    for (corners, quad) in uvs.as_chunks_mut::<4>().0.iter_mut().zip(quads) {
        *corners = quad.uv;
    }
    let Some(VertexAttributeValues::Float32x4(colors)) = mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR)
    else {
        return false;
    };
    if colors.len() != count {
        return false;
    }
    for (corners, quad) in colors.as_chunks_mut::<4>().0.iter_mut().zip(quads) {
        *corners = [quad.unit_color(); 4];
    }
    true
}

impl Batches {
    pub fn clear(&mut self) {
        self.by_material.clear();
        self.most = 0;
    }
    /// The most quads the batches drew in one frame.
    pub fn most(&self) -> usize {
        self.most
    }
    /// Quads in the visible batches.
    pub fn quads(&self) -> usize {
        self.by_material
            .values()
            .filter(|batch| batch.visible)
            .map(|batch| batch.quad_count)
            .sum()
    }
    /// Draws the quads whose material is in `additive`, one draw per material in input order.
    /// Returns how many CPU fallback meshes were rewritten.
    pub fn draw(
        &mut self,
        (commands, meshes): (&mut Commands, &mut Assets<Mesh>),
        (gpu, quad_draws): (&mut gpu::QuadGpu, &mut gpu::QuadDraws),
        quads: &[Quad],
        (materials, additive): (&BTreeMap<String, Handle<EffectMaterial>>, &BTreeSet<String>),
        layer: Option<usize>,
    ) -> usize {
        self.by_material
            .values_mut()
            .for_each(|batch| batch.next.clear());
        for quad in quads.iter().filter(|q| additive.contains(&q.material)) {
            let Some(material) = materials.get(&quad.material) else {
                continue;
            };
            if !self.by_material.contains_key(&quad.material) {
                let batch = Batch::spawn(commands, material, layer);
                self.by_material.insert(quad.material.clone(), batch);
            }
            let batch = self
                .by_material
                .get_mut(&quad.material)
                .expect("batch just ensured");
            batch.push(quad);
            batch.set_material(material);
        }
        let rewritten = self
            .by_material
            .values_mut()
            .map(|batch| batch.present(commands, meshes, gpu, quad_draws))
            .sum();
        self.most = self.most.max(self.quads());
        rewritten
    }
}

fn has_valid_storage(mesh: &Mesh, quad_count: usize) -> bool {
    let expected = quad_count * 4;
    [
        Mesh::ATTRIBUTE_POSITION,
        Mesh::ATTRIBUTE_UV_0,
        Mesh::ATTRIBUTE_COLOR,
    ]
    .into_iter()
    .all(|attr| mesh.attribute(attr).is_some_and(|v| v.len() == expected))
}

impl Batch {
    /// Starts gathering a frame's quads with `material`.
    pub(super) fn start(&mut self, material: &Handle<EffectMaterial>) {
        self.next.clear();
        self.set_material(material);
    }
    /// Draws nothing this frame (hidden by [`Self::present`]).
    pub(super) fn empty(&mut self) {
        self.next.clear();
    }
    pub(super) fn push(&mut self, quad: &Quad) {
        self.next.push(quad.into());
    }
    pub(super) fn is_full(&self) -> bool {
        self.next.len() >= gpu::MAX_BUCKET
    }
    fn set_material(&mut self, material: &Handle<EffectMaterial>) {
        if self.pending_material != *material {
            self.pending_material.clone_from(material);
        }
    }
    pub(super) fn spawn(
        commands: &mut Commands,
        material: &Handle<EffectMaterial>,
        layer: Option<usize>,
    ) -> Self {
        let empty = Handle::<Mesh>::default();
        let mut entity = commands.spawn((
            crate::campaign::MapOwned,
            gpu::QuadDraw,
            Mesh3d(empty.clone()),
            MeshTag::new(0),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            Aabb::default(),
            bevy::camera::visibility::NoAutoAabb,
            bevy::light::NotShadowCaster,
            Visibility::Hidden,
        ));
        if let Some(layer) = layer {
            entity.insert(bevy::camera::visibility::RenderLayers::layer(layer));
        }
        Self {
            entity: entity.id(),
            mesh: empty,
            tag: 0,
            bounds: Aabb::default(),
            material: material.clone(),
            pending_material: material.clone(),
            next: Vec::new(),
            visible: false,
            quad_count: 0,
            cpu_mesh: None,
            cpu_drawn: Vec::new(),
        }
    }
    fn present_cpu(&mut self, meshes: &mut Assets<Mesh>) -> (Handle<Mesh>, u32, Aabb, bool) {
        if let Some(handle) = self.cpu_mesh.as_ref().cloned()
            && let Some(mut mesh) = meshes.get_mut(&handle)
        {
            let needs_write =
                self.cpu_drawn != self.next || !has_valid_storage(&mesh, self.next.len());
            if needs_write {
                if !patch(&mut mesh, &self.next) {
                    *mesh = batch_mesh(&self.next);
                }
                self.cpu_drawn.clear();
                self.cpu_drawn.extend_from_slice(&self.next);
            }
            let bounds = if needs_write {
                mesh.get_aabb().unwrap_or_default()
            } else {
                self.bounds
            };
            return (handle, 0, bounds, needs_write);
        }
        let built = batch_mesh(&self.next);
        let bounds = built.get_aabb().unwrap_or_default();
        let handle = meshes.add(built);
        self.cpu_mesh = Some(handle.clone());
        self.cpu_drawn.clear();
        self.cpu_drawn.extend_from_slice(&self.next);
        (handle, 0, bounds, true)
    }
    fn apply_target(
        &mut self,
        commands: &mut Commands,
        quad_draws: &mut gpu::QuadDraws,
        target: Option<(Handle<Mesh>, u32, Aabb)>,
        visible: bool,
    ) {
        let vis_value = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        let mat_changed = target.is_some() && self.material != self.pending_material;
        if let Ok((mut vis, mut aabb, mut mesh3d, mut mesh_tag, mut mat)) =
            quad_draws.get_mut(self.entity)
        {
            if self.visible != visible {
                self.visible = visible;
                *vis = vis_value;
            }
            if let Some((mesh, tag, bounds)) = target {
                if self.mesh != mesh {
                    self.mesh.clone_from(&mesh);
                    mesh3d.0 = mesh;
                }
                if self.tag != tag {
                    self.tag = tag;
                    mesh_tag.value = tag;
                }
                if self.bounds != bounds {
                    self.bounds = bounds;
                    *aabb = bounds;
                }
                if mat_changed {
                    self.material.clone_from(&self.pending_material);
                    mat.0 = self.pending_material.clone();
                }
            }
        } else if let Ok(mut cmd) = commands.get_entity(self.entity) {
            if self.visible != visible {
                self.visible = visible;
                cmd.insert(vis_value);
            }
            if let Some((mesh, tag, bounds)) = target {
                if self.mesh != mesh {
                    self.mesh.clone_from(&mesh);
                    cmd.insert(Mesh3d(mesh));
                }
                if self.tag != tag {
                    self.tag = tag;
                    cmd.insert(MeshTag::new(tag));
                }
                if self.bounds != bounds {
                    self.bounds = bounds;
                    cmd.insert(bounds);
                }
                if mat_changed {
                    self.material.clone_from(&self.pending_material);
                    cmd.insert(MeshMaterial3d(self.pending_material.clone()));
                }
            }
        }
    }
    /// Shows the gathered quads on the GPU path, or falls back to a CPU mesh on overflow.
    /// Returns 1 if a CPU fallback mesh was rewritten.
    pub(super) fn present(
        &mut self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        gpu: &mut gpu::QuadGpu,
        quad_draws: &mut gpu::QuadDraws,
    ) -> usize {
        if self.next.is_empty() {
            if let Some(old) = self.cpu_mesh.take() {
                meshes.remove(&old);
                self.cpu_drawn.clear();
            }
            self.apply_target(commands, quad_draws, None, false);
            return 0;
        }
        self.quad_count = self.next.len();
        let (mesh, tag, bounds, rewritten) = if let Some((tag, aabb)) = gpu.pack(&self.next)
            && let Some(template) = gpu.template(self.next.len(), meshes)
        {
            if let Some(old) = self.cpu_mesh.take() {
                meshes.remove(&old);
                self.cpu_drawn.clear();
            }
            (template, tag, aabb, false)
        } else {
            self.present_cpu(meshes)
        };
        self.apply_target(commands, quad_draws, Some((mesh, tag, bounds)), true);
        usize::from(rewritten)
    }
}

#[cfg(test)]
mod tests;
