//! GPU storage buffer and immutable bucketed template meshes for dynamic sprite/beam quads.
use super::{EffectMaterial, Geometry};
use bevy::{
    asset::{RenderAssetUsages, uuid_handle},
    camera::primitives::Aabb,
    mesh::{Mesh3d, MeshTag},
    prelude::{
        Assets, Component, Handle, Mesh, MeshMaterial3d, Query, ResMut, Vec3, Vec4, Visibility,
        With,
    },
    render::storage::ShaderBuffer,
};

pub const QUAD_ROWS: Handle<ShaderBuffer> = uuid_handle!("3c7a4d19-82e1-4f9b-b614-9a0e5c2d7f31");
pub const MAX_QUADS: usize = 4_096;
pub const ROWS_PER_QUAD: usize = 6;
const BUCKETS: [usize; 10] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512];
pub const MAX_BUCKET: usize = 512;

/// Marker on dynamic sprite/beam quad draw entities so [`QuadDraws`] is disjoint from other draws.
#[derive(Component)]
pub struct QuadDraw;

pub type QuadDraws<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Visibility,
        &'static mut Aabb,
        &'static mut Mesh3d,
        &'static mut MeshTag,
        &'static mut MeshMaterial3d<EffectMaterial>,
    ),
    With<QuadDraw>,
>;

/// Inserts the fixed-size quad storage buffer before any [`EffectMaterial`] binds it.
pub fn install_quad_buffer(mut buffers: ResMut<Assets<ShaderBuffer>>) {
    let mut buffer = ShaderBuffer::default();
    buffer.set_data(vec![Vec4::ZERO; MAX_QUADS * ROWS_PER_QUAD]);
    let _ = buffers.insert(QUAD_ROWS.id(), buffer);
}

/// CPU-side staging buffer and immutable template mesh cache for quad draws.
pub struct QuadGpu {
    rows: Vec<Vec4>,
    cursor: usize,
    dirty: Option<(usize, usize)>,
    templates: [Option<Handle<Mesh>>; 10],
}

impl Default for QuadGpu {
    fn default() -> Self {
        Self {
            rows: vec![Vec4::ZERO; MAX_QUADS * ROWS_PER_QUAD],
            cursor: 0,
            dirty: None,
            templates: Default::default(),
        }
    }
}

impl QuadGpu {
    pub fn begin_frame(&mut self) {
        self.cursor = 0;
    }

    /// Returns an immutable `RENDER_WORLD` template mesh for the smallest bucket `>= quad_count`.
    pub fn template(
        &mut self,
        quad_count: usize,
        meshes: &mut Assets<Mesh>,
    ) -> Option<Handle<Mesh>> {
        if quad_count == 0 || quad_count > MAX_BUCKET {
            return None;
        }
        let bucket = BUCKETS.iter().position(|&size| size >= quad_count)?;
        if let Some(handle) = &self.templates[bucket]
            && meshes.contains(handle)
        {
            return Some(handle.clone());
        }
        let handle = meshes.add(template_mesh(BUCKETS[bucket]));
        self.templates[bucket] = Some(handle.clone());
        Some(handle)
    }

    /// Packs `quads` into the staging buffer, returning `(tag, aabb)` or `None` on overflow.
    pub(super) fn pack(&mut self, quads: &[Geometry]) -> Option<(u32, Aabb)> {
        if quads.is_empty() || quads.len() > MAX_BUCKET || self.cursor + quads.len() > MAX_QUADS {
            return None;
        }
        let start = self.cursor;
        let first = Vec3::from_array(quads[0].bevy_positions()[0]);
        let (mut min, mut max) = (first, first);
        for (i, quad) in quads.iter().enumerate() {
            let p = quad.bevy_positions();
            for corner in p {
                let v = Vec3::from_array(corner);
                min = min.min(v);
                max = max.max(v);
            }
            let uv = quad.uv;
            let c = quad.unit_color();
            let packed = [
                Vec4::new(p[0][0], p[0][1], p[0][2], uv[0][0]),
                Vec4::new(p[1][0], p[1][1], p[1][2], uv[1][0]),
                Vec4::new(p[2][0], p[2][1], p[2][2], uv[2][0]),
                Vec4::new(p[3][0], p[3][1], p[3][2], uv[3][0]),
                Vec4::new(uv[0][1], uv[1][1], uv[2][1], uv[3][1]),
                Vec4::new(c[0], c[1], c[2], c[3]),
            ];
            let idx = (start + i) * ROWS_PER_QUAD;
            let dst = &mut self.rows[idx..idx + ROWS_PER_QUAD];
            if dst != packed {
                dst.copy_from_slice(&packed);
                self.dirty = Some(match self.dirty {
                    Some((lo, hi)) => (lo.min(idx), hi.max(idx + ROWS_PER_QUAD)),
                    None => (idx, idx + ROWS_PER_QUAD),
                });
            }
        }
        self.cursor += quads.len();
        let tag = ((start as u32) << 16) | (quads.len() as u32);
        Some((tag, Aabb::from_min_max(min, max)))
    }

    /// Uploads only the dirty row range to [`QUAD_ROWS`].
    pub fn flush(&mut self, buffers: &mut Assets<ShaderBuffer>) {
        if let Some((lo, hi)) = self.dirty.take()
            && let Some(mut buf) = buffers.get_mut(&QUAD_ROWS)
        {
            crate::material_slots::write_rows_range(&mut buf, &self.rows, lo..hi);
        }
    }

    #[cfg(test)]
    pub(super) fn rows(&self) -> &[Vec4] {
        &self.rows
    }

    #[cfg(test)]
    pub(super) fn dirty(&self) -> Option<(usize, usize)> {
        self.dirty
    }
}

fn template_mesh(quads: usize) -> Mesh {
    let vertices = quads * 4;
    let positions = (0..quads)
        .flat_map(|q| (0..4).map(move |c| [q as f32, c as f32, 0.0]))
        .collect();
    let indices = (0..quads as u32)
        .flat_map(|q| [4 * q, 4 * q + 1, 4 * q + 2, 4 * q, 4 * q + 2, 4 * q + 3])
        .collect();
    let mesh = super::super::mesh(
        positions,
        vec![[0.; 2]; vertices],
        vec![[1.; 4]; vertices],
        indices,
        false,
    );
    // Template meshes are immutable GPU index/corner streams only.
    debug_assert_eq!(mesh.asset_usage, RenderAssetUsages::RENDER_WORLD);
    mesh
}
