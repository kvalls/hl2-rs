use super::super::texture::Place;
use glam::UVec2;
use hl2_simulation::shadows::{frame::Frame, silhouette};
use modkit_core::Surface;
use source_assets::vtf;
use std::{collections::BTreeMap, ops::Range, sync::Arc};

#[derive(Default)]
pub(crate) struct Batch {
    pub(super) vertices: Vec<u8>,
    pub(super) indices: Vec<u8>,
    pub(super) cells: Vec<Cell>,
    pub(super) draws: Vec<Draw>,
}

pub(super) struct Cell {
    pub at: UVec2,
    pub n: u32,
    pub draws: Range<usize>,
}

pub(super) struct Draw {
    pub picture: Option<Arc<vtf::Image>>,
    pub indices: Range<u32>,
}

impl Batch {
    pub(super) fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.cells.clear();
        self.draws.clear();
    }

    pub(in crate::shadows) fn cell(
        &mut self,
        n: u32,
        at: UVec2,
        frame: &Frame,
        (surfaces, place): (&[Surface], Place),
        alphas: &BTreeMap<String, (Arc<vtf::Image>, f32)>,
    ) {
        let first_draw = self.draws.len();
        for surface in surfaces {
            let base = (self.vertices.len() / 20) as u32;
            let alpha = alphas.get(&surface.material);
            let opacity = alpha.map_or(1., |(_, opacity)| *opacity);
            for v in &surface.vertices {
                let p = silhouette::project(frame, n, place(v));
                // Coordinates stay cell-local; the viewport places them in the page.
                self.vertex(p * (2. / n as f32) - 1., v.uv, opacity);
            }
            let first = (self.indices.len() / 4) as u32;
            for t in surface.indices.as_chunks::<3>().0 {
                if t.iter().all(|&i| (i as usize) < surface.vertices.len()) {
                    for i in t {
                        self.indices.extend_from_slice(&(base + i).to_ne_bytes());
                    }
                }
            }
            let last = (self.indices.len() / 4) as u32;
            if last > first {
                self.draws.push(Draw {
                    picture: alpha.map(|(p, _)| p.clone()),
                    indices: first..last,
                });
            }
        }
        self.cells.push(Cell {
            at,
            n,
            draws: first_draw..self.draws.len(),
        });
    }

    fn vertex(&mut self, p: glam::Vec2, uv: glam::Vec2, opacity: f32) {
        for value in [p.x, -p.y, uv.x, uv.y, opacity] {
            self.vertices.extend_from_slice(&value.to_ne_bytes());
        }
    }
}
