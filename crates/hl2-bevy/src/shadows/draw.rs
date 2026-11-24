//! Batched shadow draws: packs all visible shadow decals into one GPU vertex storage buffer
//! (`SHADOW_VERTS`) and two pooled `ShadowDraw` entities (atlas and blob), falling back to CPU
//! meshes (`MeshTag(0)`) if total triangles exceed `MAX_SHADOW_TRIS` or the buffer is absent.
#[cfg(test)]
mod tests;

use super::{Caster, ShadowDraw, ShadowMaterial};
use crate::campaign::MapOwned;
use bevy::{
    asset::{Assets, Handle, RenderAssetUsages, uuid_handle},
    camera::{
        primitives::Aabb,
        visibility::{NoAutoAabb, Visibility},
    },
    ecs::{
        entity::Entity,
        query::With,
        system::{Commands, Query, ResMut},
    },
    math::{Vec2, Vec3, Vec4},
    mesh::{Mesh, Mesh3d, MeshTag, PrimitiveTopology},
    pbr::MeshMaterial3d,
    render::storage::ShaderBuffer,
    transform::components::Transform,
};

pub const SHADOW_VERTS: Handle<ShaderBuffer> = uuid_handle!("9f2e6b41-7c38-4a15-8d02-6e1b9a4c3f82");
pub const MAX_SHADOW_TRIS: usize = 8_192;
const MAX_SHADOW_VERTS: usize = MAX_SHADOW_TRIS * 3;
const TOTAL_ROWS: usize = MAX_SHADOW_VERTS * 2;
const BUCKETS: [usize; 8] = [64, 128, 256, 512, 1024, 2048, 4096, 8192];

/// Allocates the fixed-size shadow vertex storage buffer once at startup.
pub fn install_shadow_buffer(mut buffers: ResMut<Assets<ShaderBuffer>>) {
    let mut buffer = ShaderBuffer::default();
    buffer.set_data(vec![Vec4::ZERO; TOTAL_ROWS]);
    let _ = buffers.insert(SHADOW_VERTS.id(), buffer);
}

pub type Draws<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Visibility,
        &'static mut Aabb,
        &'static mut Mesh3d,
        &'static mut MeshTag,
        &'static mut MeshMaterial3d<ShadowMaterial>,
    ),
    With<ShadowDraw>,
>;

#[derive(Default)]
struct BatchDraw {
    entity: Option<Entity>,
    overflow: Option<Handle<Mesh>>,
}

/// Pooled `atlas` and `blob` shadow draw batches backed by `SHADOW_VERTS`.
pub(super) struct ShadowBatches {
    atlas: BatchDraw,
    blob: BatchDraw,
    rows: Vec<Vec4>,
    dirty: Option<(usize, usize)>,
    templates: [Option<Handle<Mesh>>; 8],
    synced: bool,
}

impl Default for ShadowBatches {
    fn default() -> Self {
        Self {
            atlas: BatchDraw::default(),
            blob: BatchDraw::default(),
            rows: vec![Vec4::ZERO; TOTAL_ROWS],
            dirty: None,
            templates: std::array::from_fn(|_| None),
            synced: false,
        }
    }
}

impl ShadowBatches {
    pub fn is_synced(&self) -> bool {
        self.synced
    }

    pub fn sync(
        &mut self,
        casters: &[Caster],
        (atlas, blob): (
            Option<Handle<ShadowMaterial>>,
            Option<Handle<ShadowMaterial>>,
        ),
        (commands, meshes, buffers): (&mut Commands, &mut Assets<Mesh>, &mut Assets<ShaderBuffer>),
        draws: &mut Draws,
    ) {
        let atlas_tris = if atlas.is_some() {
            count_tris(casters, true)
        } else {
            0
        };
        let blob_tris = if blob.is_some() {
            count_tris(casters, false)
        } else {
            0
        };
        let total_tris = atlas_tris + blob_tris;
        let use_gpu = total_tris <= MAX_SHADOW_TRIS && buffers.get(&SHADOW_VERTS).is_some();

        self.sync_batch(
            (true, use_gpu),
            (0, atlas_tris),
            atlas,
            casters,
            (commands, meshes),
            draws,
        );
        self.sync_batch(
            (false, use_gpu),
            (atlas_tris, blob_tris),
            blob,
            casters,
            (commands, meshes),
            draws,
        );

        if let Some((lo, hi)) = self.dirty.take()
            && let Some(mut buffer) = buffers.get_mut(&SHADOW_VERTS)
        {
            crate::material_slots::write_rows_range(&mut buffer, &self.rows, lo..hi);
        }
        self.synced = true;
    }

    fn sync_batch(
        &mut self,
        (want_atlas, use_gpu): (bool, bool),
        (start_tri, tris): (usize, usize),
        material: Option<Handle<ShadowMaterial>>,
        casters: &[Caster],
        (commands, meshes): (&mut Commands, &mut Assets<Mesh>),
        draws: &mut Draws,
    ) {
        let Some(material) = material.filter(|_| tris > 0) else {
            let slot = if want_atlas {
                &mut self.atlas
            } else {
                &mut self.blob
            };
            hide_slot(slot, draws);
            return;
        };
        let (aabb, mesh, tag) = if use_gpu {
            let aabb = self.pack_gpu_batch(casters, want_atlas, start_tri);
            let mesh = self.template(tris, meshes);
            let tag = MeshTag(((start_tri as u32) << 16) | (tris as u32));
            (aabb, mesh, tag)
        } else {
            let slot = if want_atlas {
                &mut self.atlas
            } else {
                &mut self.blob
            };
            let (aabb, mesh) = build_cpu_mesh(slot, meshes, casters, want_atlas);
            (aabb, mesh, MeshTag(0))
        };
        let slot = if want_atlas {
            &mut self.atlas
        } else {
            &mut self.blob
        };
        apply_slot(slot, (commands, draws), (aabb, mesh, tag, material));
    }

    fn pack_gpu_batch(&mut self, casters: &[Caster], want_atlas: bool, start_tri: usize) -> Aabb {
        let mut row = start_tri * 6;
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for caster in visible_casters(casters, want_atlas) {
            for (pos, _, uv, fade) in decal_vertices(caster) {
                min = min.min(pos);
                max = max.max(pos);
                let pair = [
                    Vec4::new(pos.x, pos.y, pos.z, fade),
                    Vec4::new(uv.x, uv.y, 0.0, 0.0),
                ];
                if !self.synced || self.rows[row..row + 2] != pair {
                    self.rows[row..row + 2].copy_from_slice(&pair);
                    mark_dirty(&mut self.dirty, row, row + 2);
                }
                row += 2;
            }
        }
        Aabb::from_min_max(min, max)
    }

    fn template(&mut self, tris: usize, meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
        let bucket = BUCKETS
            .iter()
            .position(|&cap| tris <= cap)
            .expect("tris <= MAX_SHADOW_TRIS");
        if let Some(handle) = &self.templates[bucket] {
            return handle.clone();
        }
        let vert_count = BUCKETS[bucket] * 3;
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            (0..vert_count)
                .map(|v| [v as f32, 0.0, 0.0])
                .collect::<Vec<_>>(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; vert_count]);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            vec![[1.0, 1.0, 1.0, 0.0]; vert_count],
        );
        let handle = meshes.add(mesh);
        self.templates[bucket] = Some(handle.clone());
        handle
    }
}

fn visible_casters(casters: &[Caster], want_atlas: bool) -> impl Iterator<Item = &Caster> {
    casters
        .iter()
        .filter(move |c| c.shown && !c.decal.is_empty() && c.use_atlas == want_atlas)
}

fn count_tris(casters: &[Caster], want_atlas: bool) -> usize {
    visible_casters(casters, want_atlas)
        .map(|c| c.decal.indices.len() / 3)
        .sum()
}

fn decal_vertices(caster: &Caster) -> impl Iterator<Item = (Vec3, Vec3, Vec2, f32)> + '_ {
    let (origin, size) = caster.rect;
    let tri_indices = (caster.decal.indices.len() / 3) * 3;
    caster.decal.indices[..tri_indices].iter().map(move |&idx| {
        let i = idx as usize;
        let pos = crate::source_to_bevy(Vec3::from_array(caster.decal.positions[i].to_array()));
        let normal = crate::source_to_bevy(Vec3::from_array(caster.decal.normals[i].to_array()));
        let uv = origin + Vec2::from_array(caster.decal.uvs[i].to_array()) * size;
        let fade = f32::from(caster.decal.fades[i]) / 255.0;
        (pos, normal, uv, fade)
    })
}

fn build_cpu_mesh(
    slot: &mut BatchDraw,
    meshes: &mut Assets<Mesh>,
    casters: &[Caster],
    want_atlas: bool,
) -> (Aabb, Handle<Mesh>) {
    let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    for caster in visible_casters(casters, want_atlas) {
        for (pos, normal, uv, fade) in decal_vertices(caster) {
            min = min.min(pos);
            max = max.max(pos);
            positions.push(pos.to_array());
            normals.push(normal.to_array());
            uvs.push(uv.to_array());
            colors.push([1.0, 1.0, 1.0, fade]);
        }
    }
    let handle = match &slot.overflow {
        Some(handle) if meshes.get(handle).is_some() => handle.clone(),
        _ => {
            let handle = meshes.add(Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
            ));
            slot.overflow = Some(handle.clone());
            handle
        }
    };
    let mut mesh = meshes.get_mut(&handle).expect("overflow mesh exists");
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    (Aabb::from_min_max(min, max), handle)
}

fn hide_slot(slot: &mut BatchDraw, draws: &mut Draws) {
    if let Some(entity) = slot.entity
        && let Ok((mut vis, ..)) = draws.get_mut(entity)
        && *vis != Visibility::Hidden
    {
        *vis = Visibility::Hidden;
    }
}

fn apply_slot(
    slot: &mut BatchDraw,
    (commands, draws): (&mut Commands, &mut Draws),
    (aabb, mesh, tag, material): (Aabb, Handle<Mesh>, MeshTag, Handle<ShadowMaterial>),
) {
    if let Some(entity) = slot.entity
        && let Ok((mut vis, mut cur_aabb, mut cur_mesh, mut cur_tag, mut cur_mat)) =
            draws.get_mut(entity)
    {
        if *vis != Visibility::Inherited {
            *vis = Visibility::Inherited;
        }
        if *cur_aabb != aabb {
            *cur_aabb = aabb;
        }
        if cur_mesh.0 != mesh {
            cur_mesh.0 = mesh;
        }
        if *cur_tag != tag {
            *cur_tag = tag;
        }
        if cur_mat.0 != material {
            cur_mat.0 = material;
        }
        return;
    }
    let entity = commands
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            Visibility::Inherited,
            aabb,
            tag,
            ShadowDraw,
            bevy::light::NotShadowCaster,
            NoAutoAabb,
            MapOwned,
        ))
        .id();
    slot.entity = Some(entity);
}

fn mark_dirty(range: &mut Option<(usize, usize)>, start: usize, end: usize) {
    *range = Some(match *range {
        Some((lo, hi)) => (lo.min(start), hi.max(end)),
        None => (start, end),
    });
}
