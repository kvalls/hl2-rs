//! Persistent sprite geometry: one draw per quad, in input order with its own bounds, except
//! the additive quads, which [`batch`] draws as one mesh per material.
mod batch;
use super::EffectMaterial;
use bevy::{
    camera::primitives::MeshAabb,
    mesh::VertexAttributeValues,
    prelude::{
        Assets, Commands, Entity, Handle, Mesh, Mesh3d, MeshMaterial3d, Transform, Vec3, Visibility,
    },
};
use hl2_simulation::projectile_visuals::Quad;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct QuadDraw {
    entity: Entity,
    mesh: Handle<Mesh>,
    material: Handle<EffectMaterial>,
    previous: Geometry,
    visible: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct Geometry {
    positions: [glam::Vec3; 4],
    color: [u8; 4],
    uv: [[f32; 2]; 4],
}

impl From<&Quad> for Geometry {
    fn from(quad: &Quad) -> Self {
        Self {
            positions: quad.positions,
            color: quad.color,
            uv: quad.uv,
        }
    }
}

fn quad_mesh(quad: &Quad) -> Mesh {
    super::mesh(
        positions(quad).to_vec(),
        quad.uv.to_vec(),
        vec![color(quad); 4],
        vec![0, 1, 2, 0, 2, 3],
        true,
    )
}

impl Geometry {
    fn bevy_positions(&self) -> [[f32; 3]; 4] {
        self.positions
            .map(|p| crate::source_to_bevy(Vec3::from_array(p.to_array())).to_array())
    }
    fn unit_color(&self) -> [f32; 4] {
        self.color.map(|c| f32::from(c) / 255.)
    }
}

fn positions(quad: &Quad) -> [[f32; 3]; 4] {
    Geometry::from(quad).bevy_positions()
}

fn color(quad: &Quad) -> [f32; 4] {
    Geometry::from(quad).unit_color()
}

/// Only the varying attributes change. Indices, normals and the unused second UV
/// channel retain their storage, as do the varying arrays after initial allocation.
fn fill(mesh: &mut Mesh, quad: &Quad) {
    if !patch(mesh, quad) {
        *mesh = quad_mesh(quad);
    }
}

fn patch(mesh: &mut Mesh, quad: &Quad) -> bool {
    let Some(VertexAttributeValues::Float32x3(buffer)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    else {
        return false;
    };
    if buffer.len() != 4 {
        return false;
    }
    buffer.copy_from_slice(&positions(quad));
    let Some(VertexAttributeValues::Float32x2(buffer)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0)
    else {
        return false;
    };
    if buffer.len() != 4 {
        return false;
    }
    buffer.copy_from_slice(&quad.uv);
    let Some(VertexAttributeValues::Float32x4(buffer)) = mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR)
    else {
        return false;
    };
    if buffer.len() != 4 {
        return false;
    }
    buffer.fill(color(quad));
    true
}

fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    quad: &Quad,
    material: &Handle<EffectMaterial>,
    layer: Option<usize>,
) -> QuadDraw {
    let mesh = quad_mesh(quad);
    let bounds = mesh.compute_aabb();
    let mesh = meshes.add(mesh);
    let mut entity = commands.spawn((
        crate::campaign::MapOwned,
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Transform::IDENTITY,
        // Sprites and particles are translucent: no shadow depth (the flashlight's included).
        bevy::light::NotShadowCaster,
        Visibility::Visible,
    ));
    if let Some(bounds) = bounds {
        entity.insert(bounds);
    }
    if let Some(layer) = layer {
        entity.insert(bevy::camera::visibility::RenderLayers::layer(layer));
    }
    QuadDraw {
        entity: entity.id(),
        mesh,
        material: material.clone(),
        previous: quad.into(),
        visible: true,
    }
}

/// The effect materials by name, and the names of those whose quads are batched.
pub(super) type Materials<'a> = (
    &'a BTreeMap<String, Handle<EffectMaterial>>,
    &'a BTreeSet<String>,
);

/// The sprite quads of one render layer: a pooled draw each, or a batch per additive material.
#[derive(Default)]
pub(super) struct QuadLayer {
    pool: Vec<QuadDraw>,
    batches: batch::Batches,
}

impl QuadLayer {
    pub fn clear(&mut self) {
        self.pool.clear();
        self.batches.clear();
    }
    /// The most quads drawn at once (pooled draws and the batches' fullest frame), and the quads
    /// inside batches now.
    pub fn counts(&self) -> (usize, usize) {
        (self.pool.len() + self.batches.most(), self.batches.quads())
    }
    pub fn draw(
        &mut self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        quads: &[Quad],
        materials: Materials,
        layer: Option<usize>,
    ) {
        (self.batches).draw((&mut *commands, &mut *meshes), quads, materials, layer);
        draw_pool(commands, meshes, &mut self.pool, quads, materials, layer);
    }
}

/// Keep one draw per quad: alpha ordering and per-view visibility stay unchanged. Quads whose
/// material is in `batched` are drawn elsewhere and take no slot.
pub(super) fn draw_pool(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    pool: &mut Vec<QuadDraw>,
    quads: &[Quad],
    (materials, batched): Materials,
    layer: Option<usize>,
) {
    let mut used = 0;
    for quad in quads.iter().filter(|q| !batched.contains(&q.material)) {
        let Some(material) = materials.get(&quad.material) else {
            continue;
        };
        if used == pool.len() {
            pool.push(spawn(commands, meshes, quad, material, layer));
        }
        let slot = &mut pool[used];
        used += 1;
        let previous = &mut slot.previous;
        let moved = previous.positions != quad.positions;
        if moved || previous.uv != quad.uv || previous.color != quad.color {
            if let Some(mut mesh) = meshes.get_mut(&slot.mesh) {
                fill(&mut mesh, quad);
                if moved
                    && let Some(bounds) = mesh.compute_aabb()
                    && let Ok(mut entity) = commands.get_entity(slot.entity)
                {
                    entity.insert(bounds);
                }
            }
            previous.positions = quad.positions;
            previous.uv = quad.uv;
            previous.color = quad.color;
        }
        if slot.material != *material {
            if let Ok(mut entity) = commands.get_entity(slot.entity) {
                entity.insert(MeshMaterial3d(material.clone()));
            }
            slot.material.clone_from(material);
        }
        if !slot.visible {
            if let Ok(mut entity) = commands.get_entity(slot.entity) {
                entity.insert(Visibility::Visible);
            }
            slot.visible = true;
        }
    }
    for slot in pool.iter_mut().skip(used).filter(|slot| slot.visible) {
        if let Ok(mut entity) = commands.get_entity(slot.entity) {
            entity.insert(Visibility::Hidden);
        }
        slot.visible = false;
    }
}

#[cfg(test)]
mod tests;
