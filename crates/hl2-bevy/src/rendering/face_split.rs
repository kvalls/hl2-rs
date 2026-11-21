//! Face draws split into the triangles facial flexes can move and the rigid rest. A changed
//! mesh is uploaded whole, and a talking or blinking face changes every frame, while flexes move
//! a few percent of its vertices; the rigid draw keeps its flex references (the same LOD rules)
//! but none of its vertices ever moves, so it is never uploaded again.
use super::Batch;
use source_assets::flexes::FaceModel;
use std::collections::HashSet;

/// Every (bodypart, mesh, mesh-local vertex) a flex of the model's first model moves.
pub(super) fn movable(face: &FaceModel) -> HashSet<[u16; 3]> {
    let meshes = face.flex.meshes.iter().filter(|m| m.model == 0);
    meshes
        .flat_map(|mesh| {
            let key = (mesh.bodypart as u16, mesh.mesh as u16);
            let vertices = mesh.flexes.iter().flat_map(|flex| &flex.vertices);
            vertices.map(move |v| [key.0, key.1, v.index])
        })
        .collect()
}

/// Entity `id`'s batch split by its model's flexes, or whole when the model has none.
pub(super) fn split_entity_batch(
    batch: Batch,
    world: &modkit_core::World,
    faces: &std::collections::BTreeMap<String, std::sync::Arc<FaceModel>>,
    id: usize,
) -> Vec<Batch> {
    if !batch.flex.iter().any(Option::is_some) {
        return vec![batch];
    }
    let instance = world.model_instances.iter().find(|i| i.entity == Some(id));
    match instance.and_then(|i| faces.get(&i.asset_key())) {
        Some(face) => batch.split_flexed(&movable(face)),
        None => vec![batch],
    }
}

/// A runtime entity's draws: one batch per material, its faces split by their flexes.
pub(super) fn model_batches(
    surfaces: &[modkit_core::Surface],
    scale: f32,
    cache: &super::ModelCache,
    (world, id): (&modkit_core::World, usize),
) -> Vec<(String, Batch)> {
    let scale = bevy::math::Mat4::from_scale(bevy::math::Vec3::splat(scale));
    let mut batches = std::collections::BTreeMap::<String, Batch>::new();
    for surface in surfaces.iter().filter(|s| !s.indices.is_empty()) {
        let material = cache.definitions.get(&surface.material);
        (batches.entry(surface.material.clone()).or_default()).append(surface, scale, material);
    }
    let split = |(name, batch): (String, Batch)| {
        let parts = split_entity_batch(batch, world, &cache.faces, id);
        parts.into_iter().map(move |batch| (name.clone(), batch))
    };
    batches.into_iter().flat_map(split).collect()
}

impl Batch {
    /// Every per-vertex list has one entry per position and the indices make triangles.
    pub(super) fn consistent(&self) -> bool {
        let n = self.positions.len();
        self.indices.len().is_multiple_of(3)
            && self.indices.iter().all(|i| (*i as usize) < n)
            && [self.uvs.len(), self.light_uvs.len(), self.colors.len()]
                .iter()
                .all(|l| *l == n)
            && self.normals.len() == n
            && self.skin.len() == n
    }
    /// The rigid triangles, then those touching a vertex a flex can move (in input order); a
    /// batch with nothing to split stays whole.
    pub(super) fn split_flexed(self, movable: &HashSet<[u16; 3]>) -> Vec<Self> {
        if !self.consistent() || self.flex.len() != self.positions.len() {
            return vec![self];
        }
        let moves = |i: &u32| self.flex[*i as usize].is_some_and(|key| movable.contains(&key));
        let (mut rigid, mut moving) = (Vec::new(), Vec::new());
        for triangle in self.indices.as_chunks::<3>().0 {
            let group = if triangle.iter().any(moves) {
                &mut moving
            } else {
                &mut rigid
            };
            group.extend_from_slice(triangle);
        }
        if rigid.is_empty() || moving.is_empty() {
            return vec![self];
        }
        self.regroup(vec![rigid, moving])
    }
    /// One batch per index list (triangles of this batch), each with only the vertices it uses,
    /// in first-use order.
    pub(super) fn regroup(self, groups: Vec<Vec<u32>>) -> Vec<Self> {
        let flex = self.flex.len() == self.positions.len();
        groups
            .into_iter()
            .map(|indices| {
                let mut batch = Self {
                    model: self.model,
                    ..Self::default()
                };
                let mut remap = std::collections::HashMap::new();
                for index in indices {
                    let new = *remap.entry(index).or_insert_with(|| {
                        let (new, i) = (batch.positions.len() as u32, index as usize);
                        batch.positions.push(self.positions[i]);
                        batch.uvs.push(self.uvs[i]);
                        batch.light_uvs.push(self.light_uvs[i]);
                        batch.colors.push(self.colors[i]);
                        batch.normals.push(self.normals[i]);
                        batch.skin.push(self.skin[i].clone());
                        if flex {
                            batch.flex.push(self.flex[i]);
                        }
                        new
                    });
                    batch.indices.push(new);
                }
                batch
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
