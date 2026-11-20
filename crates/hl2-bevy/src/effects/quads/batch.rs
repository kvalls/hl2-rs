//! Sprite quads drawn as one mesh: the additive quads of one material, or a run of consecutive
//! quads of one material.
//!
//! A quad that faces the camera changes every frame, and every changed mesh is uploaded
//! again; one mesh per quad makes that one upload per sprite. Quads that blend by pure
//! addition commute, so those of one material can share a mesh without changing the image
//! among themselves; such a batch is sorted against other translucent draws as one object. A run
//! keeps its quads in input order inside one draw.
use super::{EffectMaterial, Geometry};
use bevy::{
    camera::primitives::MeshAabb,
    mesh::VertexAttributeValues,
    prelude::{
        Assets, Commands, Entity, Handle, Mesh, Mesh3d, MeshMaterial3d, Transform, Visibility,
    },
};
use hl2_simulation::projectile_visuals::Quad;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Batch {
    pub(super) entity: Entity,
    pub(super) mesh: Handle<Mesh>,
    material: Handle<EffectMaterial>,
    /// The quads in the mesh, and those of the frame being gathered.
    drawn: Vec<Geometry>,
    next: Vec<Geometry>,
    visible: bool,
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
    /// Quads in the batch meshes.
    pub fn quads(&self) -> usize {
        let shown = self.by_material.values().filter(|batch| batch.visible);
        shown.map(|batch| batch.drawn.len()).sum()
    }
    /// Draws the quads whose material is in `additive`, one mesh per material in input order.
    /// Returns how many meshes were rewritten.
    pub fn draw(
        &mut self,
        (commands, meshes): (&mut Commands, &mut Assets<Mesh>),
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
                let batch = Batch::spawn(commands, meshes, material, layer);
                self.by_material.insert(quad.material.clone(), batch);
            }
            let batch = self
                .by_material
                .get_mut(&quad.material)
                .expect("batch just ensured");
            batch.push(quad);
            batch.set_material(commands, material);
        }
        let batches = self.by_material.values_mut();
        let rewritten = batches.map(|batch| batch.present(commands, meshes)).sum();
        self.most = self.most.max(self.quads());
        rewritten
    }
}

impl Batch {
    /// Starts gathering a frame's quads with `material`.
    pub(super) fn start(&mut self, commands: &mut Commands, material: &Handle<EffectMaterial>) {
        self.next.clear();
        self.set_material(commands, material);
    }
    /// Draws nothing this frame (hidden by [`Self::present`]).
    pub(super) fn empty(&mut self) {
        self.next.clear();
    }
    pub(super) fn push(&mut self, quad: &Quad) {
        self.next.push(quad.into());
    }
    fn set_material(&mut self, commands: &mut Commands, material: &Handle<EffectMaterial>) {
        if self.material != *material {
            self.material.clone_from(material);
            if let Ok(mut entity) = commands.get_entity(self.entity) {
                entity.insert(MeshMaterial3d(material.clone()));
            }
        }
    }
    pub(super) fn spawn(
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        material: &Handle<EffectMaterial>,
        layer: Option<usize>,
    ) -> Self {
        let mesh = meshes.add(batch_mesh(&[]));
        let mut entity = commands.spawn((
            crate::campaign::MapOwned,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            bevy::light::NotShadowCaster,
            Visibility::Hidden,
        ));
        if let Some(layer) = layer {
            entity.insert(bevy::camera::visibility::RenderLayers::layer(layer));
        }
        Self {
            entity: entity.id(),
            mesh,
            material: material.clone(),
            drawn: Vec::new(),
            next: Vec::new(),
            visible: false,
        }
    }
    /// Shows the gathered quads; 1 if the mesh was rewritten. A batch without quads is hidden
    /// and keeps its mesh, so the same quads returning cost no upload.
    pub(super) fn present(&mut self, commands: &mut Commands, meshes: &mut Assets<Mesh>) -> usize {
        let visible = !self.next.is_empty();
        let rewrite = visible && self.next != self.drawn;
        if rewrite && let Some(mut mesh) = meshes.get_mut(&self.mesh) {
            if !patch(&mut mesh, &self.next) {
                *mesh = batch_mesh(&self.next);
            }
            if let (Some(bounds), Ok(mut entity)) =
                (mesh.compute_aabb(), commands.get_entity(self.entity))
            {
                entity.insert(bounds);
            }
            std::mem::swap(&mut self.drawn, &mut self.next);
        }
        if visible != self.visible {
            self.visible = visible;
            if let Ok(mut entity) = commands.get_entity(self.entity) {
                entity.insert(if visible {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                });
            }
        }
        usize::from(rewrite)
    }
}

#[cfg(test)]
mod tests;
