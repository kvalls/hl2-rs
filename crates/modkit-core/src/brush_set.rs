//! World brushes with a static bounds tree, so a hull trace visits only the brushes near it.
//!
//! The result is the full scan's, bit for bit. A brush's box is made only of its own
//! axis-aligned planes. A sweep whose hull stays farther than [`SLACK`] outside that box on
//! some axis is outside one of those planes by more than the collision epsilon at both ends,
//! and for such a plane the brush's plane loop reports no hit and no solid flag. The brushes
//! that remain run through that same loop in list order.
use crate::{sweep_planes, trace_brushes, Brush, Trace};
use glam::Vec3;

/// Separation beyond which a brush cannot affect a sweep: well above the 1/32 collision
/// epsilon plus the rounding of map-scale coordinates.
const SLACK: f32 = 0.125;
/// Keep rounding below SLACK: axial distances and sweep inputs are at most this magnitude.
/// Outside this domain the original scan is the authority, including non-finite inputs.
const COORDINATE_LIMIT: f32 = 65536.;
/// Brushes per tree leaf.
const LEAF: usize = 4;
/// Candidates gathered on the stack; a sweep that meets more uses the full scan.
const CANDIDATES: usize = 128;
/// Tree nodes pending during a walk; the tree is balanced, so its depth stays far below this.
const PENDING: usize = 64;

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}
impl Bounds {
    /// The box of a brush's own axis-aligned planes; unbounded where it has none.
    fn of(brush: &Brush) -> Self {
        let mut min = Vec3::NEG_INFINITY;
        let mut max = Vec3::INFINITY;
        if brush.planes.iter().any(|p| {
            !p.normal.is_finite() || !p.distance.is_finite() || p.distance.abs() > COORDINATE_LIMIT
        }) {
            return Self { min, max };
        }
        for plane in &brush.planes {
            for (axis, unit) in Vec3::AXES.into_iter().enumerate() {
                if plane.normal == unit {
                    max[axis] = max[axis].min(plane.distance);
                } else if plane.normal == -unit {
                    min[axis] = min[axis].max(-plane.distance);
                }
            }
        }
        Self { min, max }
    }
    fn sweep(start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> Option<Self> {
        if mins.cmpgt(maxs).any()
            || [start, end, mins, maxs]
                .iter()
                .any(|v| !v.is_finite() || v.abs().max_element() > COORDINATE_LIMIT)
        {
            return None;
        }
        Some(Self {
            min: start.min(end) + mins - SLACK,
            max: start.max(end) + maxs + SLACK,
        })
    }
    fn finite(&self) -> bool {
        self.min.is_finite() && self.max.is_finite()
    }
    fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
    fn misses(&self, sweep: &Bounds) -> bool {
        sweep.min.cmpgt(self.max).any() || sweep.max.cmplt(self.min).any()
    }
}

#[derive(Clone, Debug)]
struct Node {
    bounds: Bounds,
    /// A leaf holds `order[first..first + count]`; a branch has `count == 0`, its left child
    /// in the next node and its right child in node `first`.
    first: u32,
    count: u32,
}

/// A brush list with the tree that finds a sweep's candidates.
#[derive(Clone, Debug, Default)]
pub struct BrushSet {
    brushes: Vec<Brush>,
    nodes: Vec<Node>,
    /// Brush indices in leaf order.
    order: Vec<u32>,
    /// Brushes lacking an axis-aligned plane, with the sides they have; tested one by one.
    unbounded: Vec<(u32, Bounds)>,
}

impl BrushSet {
    pub fn new(brushes: Vec<Brush>) -> Self {
        let mut set = Self {
            brushes,
            ..Self::default()
        };
        let mut boxed = Vec::new();
        for (index, brush) in set.brushes.iter().enumerate() {
            let bounds = Bounds::of(brush);
            // A brush without planes never affects a trace.
            if bounds.finite() {
                boxed.push((index as u32, bounds));
            } else if !brush.planes.is_empty() {
                set.unbounded.push((index as u32, bounds));
            }
        }
        if !boxed.is_empty() {
            set.build(&mut boxed);
        }
        set
    }
    pub fn brushes(&self) -> &[Brush] {
        &self.brushes
    }
    /// Brushes lacking a complete finite axial box; these are checked individually.
    pub fn unbounded_count(&self) -> usize {
        self.unbounded.len()
    }
    /// Candidate count for workload diagnostics, or `None` if the full scan is required.
    /// This gathers candidates afresh; it does not add counters or work to normal tracing.
    pub fn candidate_count(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> Option<usize> {
        self.gather(
            &Bounds::sweep(start, end, mins, maxs)?,
            &mut [0; CANDIDATES],
        )
    }
    /// Adds the subtree for `items` and returns its root node.
    fn build(&mut self, items: &mut [(u32, Bounds)]) -> usize {
        let bounds = items[1..]
            .iter()
            .fold(items[0].1, |all, (_, bounds)| all.union(*bounds));
        let node = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            first: self.order.len() as u32,
            count: items.len() as u32,
        });
        if items.len() <= LEAF {
            self.order.extend(items.iter().map(|(index, _)| *index));
            return node;
        }
        let center = |bounds: &Bounds| bounds.min + bounds.max;
        let (low, high) = items.iter().fold(
            (Vec3::INFINITY, Vec3::NEG_INFINITY),
            |(low, high), (_, bounds)| (low.min(center(bounds)), high.max(center(bounds))),
        );
        let extent = high - low;
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let middle = items.len() / 2;
        items.select_nth_unstable_by(middle, |a, b| {
            center(&a.1)[axis].total_cmp(&center(&b.1)[axis])
        });
        let (left, right) = items.split_at_mut(middle);
        // The left child follows its parent; the right one is recorded.
        self.build(left);
        self.nodes[node].first = self.build(right) as u32;
        self.nodes[node].count = 0;
        node
    }
    /// Player or NPC hull sweep against the brushes matching `contents_mask`: the result of
    /// [`trace_brushes`] over the whole list.
    pub fn trace(
        &self,
        start: Vec3,
        end: Vec3,
        mins: Vec3,
        maxs: Vec3,
        contents_mask: u32,
    ) -> Trace {
        let mut found = [0u32; CANDIDATES];
        let count =
            Bounds::sweep(start, end, mins, maxs).and_then(|sweep| self.gather(&sweep, &mut found));
        let Some(count) = count else {
            return trace_brushes(&self.brushes, start, end, mins, maxs, contents_mask);
        };
        let found = &mut found[..count];
        found.sort_unstable();
        let mut result = Trace::miss();
        for &index in found.iter() {
            let brush = &self.brushes[index as usize];
            if brush.contents & contents_mask != 0 {
                sweep_planes(&brush.planes, start, end, mins, maxs, &mut result);
            }
        }
        result
    }
    /// The brushes a sweep may touch, unordered; `None` when they must all be scanned.
    fn gather(&self, sweep: &Bounds, found: &mut [u32; CANDIDATES]) -> Option<usize> {
        if !(sweep.min.is_finite() && sweep.max.is_finite()) {
            return None;
        }
        let mut count = 0;
        for (index, bounds) in &self.unbounded {
            if !bounds.misses(sweep) {
                *found.get_mut(count)? = *index;
                count += 1;
            }
        }
        let mut pending = [0u32; PENDING];
        let mut depth = usize::from(!self.nodes.is_empty());
        while depth > 0 {
            depth -= 1;
            let index = pending[depth];
            let node = &self.nodes[index as usize];
            if node.bounds.misses(sweep) {
                continue;
            }
            if node.count == 0 {
                if depth + 2 > PENDING {
                    return None;
                }
                pending[depth] = node.first;
                pending[depth + 1] = index + 1;
                depth += 2;
                continue;
            }
            let leaf = &self.order[node.first as usize..(node.first + node.count) as usize];
            if count + leaf.len() > CANDIDATES {
                return None;
            }
            found[count..count + leaf.len()].copy_from_slice(leaf);
            count += leaf.len();
        }
        Some(count)
    }
}

#[cfg(test)]
mod tests;
