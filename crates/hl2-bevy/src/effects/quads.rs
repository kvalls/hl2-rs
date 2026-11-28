//! Persistent sprite geometry. Additive quads are drawn as one mesh per material ([`batch`]); the
//! rest as runs: consecutive quads of one material share a mesh. Run draws have identity
//! transforms, so they tie in the transparent sort and keep their entity order: they draw in input
//! order, as one draw per quad did, at one upload per changed run instead of one per sprite.
mod batch;
mod gpu;
use super::EffectMaterial;
use bevy::{
    prelude::{Assets, Commands, Handle, Mesh, Vec3},
    render::storage::ShaderBuffer,
};
pub use gpu::{QUAD_ROWS, QuadDraw, QuadDraws, QuadGpu, install_quad_buffer};
use hl2_simulation::projectile_visuals::Quad;
use std::collections::{BTreeMap, BTreeSet};

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

impl Geometry {
    fn bevy_positions(&self) -> [[f32; 3]; 4] {
        self.positions
            .map(|p| crate::source_to_bevy(Vec3::from_array(p.to_array())).to_array())
    }
    fn unit_color(&self) -> [f32; 4] {
        self.color.map(|c| f32::from(c) / 255.)
    }
}

/// The effect materials by name, and the names of those whose quads are batched.
pub(super) type Materials<'a> = (
    &'a BTreeMap<String, Handle<EffectMaterial>>,
    &'a BTreeSet<String>,
);

/// The sprite quads of one render layer: runs in input order, and a batch per additive material.
#[derive(Default)]
pub(super) struct QuadLayer {
    runs: Vec<batch::Batch>,
    /// The most quads the runs drew in one frame.
    most_in_runs: usize,
    batches: batch::Batches,
}

impl QuadLayer {
    pub fn clear(&mut self) {
        self.runs.clear();
        self.most_in_runs = 0;
        self.batches.clear();
    }
    /// The most quads drawn at once (runs and the batches' fullest frame), and the quads inside
    /// batches now.
    pub fn counts(&self) -> (usize, usize) {
        (
            self.most_in_runs + self.batches.most(),
            self.batches.quads(),
        )
    }
    pub fn draw(
        &mut self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        (gpu, quad_draws): (&mut QuadGpu, &mut QuadDraws),
        quads: &[Quad],
        materials: Materials,
        layer: Option<usize>,
    ) {
        (self.batches).draw(
            (&mut *commands, &mut *meshes),
            (&mut *gpu, &mut *quad_draws),
            quads,
            materials,
            layer,
        );
        let drawn = draw_runs(
            commands,
            meshes,
            (gpu, quad_draws),
            &mut self.runs,
            quads,
            materials,
            layer,
        );
        self.most_in_runs = self.most_in_runs.max(drawn);
    }
}

/// Presents both the world quad layer and the viewmodel quad layer, flushing dirty GPU rows once.
pub(super) fn present_layers(
    commands: &mut Commands,
    (meshes, buffers): (&mut Assets<Mesh>, &mut Assets<ShaderBuffer>),
    quad_draws: &mut QuadDraws,
    effects: &mut super::Effects,
    viewmodel: &[Quad],
) {
    let super::Effects {
        source,
        materials,
        additive,
        quads,
        viewmodel_quads,
        quad_gpu,
        ..
    } = effects;
    quad_gpu.begin_frame();
    quads.draw(
        commands,
        meshes,
        (quad_gpu, quad_draws),
        &source.quads,
        (materials, additive),
        None,
    );
    // Viewmodel quads (placed against the gun) are never batched by material, only run.
    let unbatched = BTreeSet::new();
    viewmodel_quads.draw(
        commands,
        meshes,
        (quad_gpu, quad_draws),
        viewmodel,
        (materials, &unbatched),
        Some(1),
    );
    quad_gpu.flush(buffers);
}

/// Draws the quads whose material is not in `batched`, consecutive quads of one material sharing
/// a draw (split at [`gpu::MAX_BUCKET`]), in input order; returns how many quads it drew.
fn draw_runs(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    (gpu, quad_draws): (&mut QuadGpu, &mut QuadDraws),
    runs: &mut Vec<batch::Batch>,
    quads: &[Quad],
    (materials, batched): Materials,
    layer: Option<usize>,
) -> usize {
    let (mut used, mut drawn) = (0, 0);
    let mut current: Option<&str> = None;
    for quad in quads.iter().filter(|q| !batched.contains(&q.material)) {
        let Some(material) = materials.get(&quad.material) else {
            continue;
        };
        if current != Some(quad.material.as_str()) || runs[used - 1].is_full() {
            if used == runs.len() {
                runs.push(batch::Batch::spawn(commands, material, layer));
            }
            runs[used].start(material);
            used += 1;
            current = Some(&quad.material);
        }
        runs[used - 1].push(quad);
        drawn += 1;
    }
    for run in &mut runs[used..] {
        run.empty();
    }
    for run in runs.iter_mut() {
        run.present(commands, meshes, gpu, quad_draws);
    }
    drawn
}

#[cfg(test)]
mod tests;
