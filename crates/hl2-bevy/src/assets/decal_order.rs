//! Load-time ordering of translucent infodecals that share a patch of wall.
use crate::coords::source_to_bevy;
use bevy::math::{Vec3, bounding::Aabb3d};
use modkit_core::Surface;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub(crate) struct DecalLayer {
    pub bounds: Aabb3d,
    pub order: usize,
}

fn bounds(surface: &Surface) -> Aabb3d {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = -min;
    for vertex in &surface.vertices {
        let p = source_to_bevy(Vec3::from_array(vertex.position.to_array()));
        min = min.min(p);
        max = max.max(p);
    }
    Aabb3d::from_min_max(min, max)
}

const PLANE_EPSILON: f32 = 0.01;

/// For a counterclockwise triangle, each inward edge normal defines a half-plane.
/// A separating edge on either triangle rules out an area of shared paint.
fn inside_edges(a: [glam::Vec3; 3], b: [glam::Vec3; 3], normal: glam::Vec3) -> bool {
    (0..3).all(|i| {
        let inward = normal.cross(a[(i + 1) % 3] - a[i]);
        b.iter().any(|p| inward.dot(*p - a[i]) > 0.)
    })
}

fn coplanar_overlap(a: &Surface, b: &Surface) -> bool {
    a.indices.as_chunks::<3>().0.iter().any(|ta| {
        let vertices = ta.map(|i| &a.vertices[i as usize]);
        let normal = vertices[0].normal;
        let pa = vertices.map(|v| v.position);
        b.indices.as_chunks::<3>().0.iter().any(|tb| {
            let vertices = tb.map(|i| &b.vertices[i as usize]);
            let pb = vertices.map(|v| v.position);
            normal.dot(vertices[0].normal) > 0.9999
                && pb
                    .iter()
                    .all(|p| normal.dot(*p - pa[0]).abs() < PLANE_EPSILON)
                && inside_edges(pa, pb, normal)
                && inside_edges(pb, pa, normal)
        })
    })
}

fn root(parents: &mut [usize], mut i: usize) -> usize {
    while parents[i] != i {
        parents[i] = parents[parents[i]];
        i = parents[i];
    }
    i
}

/// Keep ordinary material batching for isolated pieces. The remaining pieces
/// must survive batching in their original placement order.
pub(super) fn separate(
    pieces: Vec<(usize, Surface)>,
    ordinary: &mut Vec<Surface>,
) -> Vec<(Surface, DecalLayer)> {
    let boxes: Vec<_> = pieces.iter().map(|(_, s)| bounds(s)).collect();
    let mut parents: Vec<_> = (0..pieces.len()).collect();
    let mut sweep = parents.clone();
    sweep.sort_unstable_by(|&a, &b| boxes[a].min.x.total_cmp(&boxes[b].min.x));
    for (k, &a) in sweep.iter().enumerate() {
        for &b in &sweep[k + 1..] {
            let gap = boxes[a].min.max(boxes[b].min) - boxes[a].max.min(boxes[b].max);
            if boxes[b].min.x > boxes[a].max.x + PLANE_EPSILON {
                break;
            }
            if pieces[a].0 != pieces[b].0
                && gap.max_element() <= PLANE_EPSILON
                && coplanar_overlap(&pieces[a].1, &pieces[b].1)
            {
                let ra = root(&mut parents, a);
                parents[ra] = root(&mut parents, b);
            }
        }
    }
    let mut groups = BTreeMap::new();
    for (i, &bounds) in boxes.iter().enumerate() {
        let group = root(&mut parents, i);
        let (union, count) = groups.entry(group).or_insert((bounds, 0));
        union.min = union.min.min(bounds.min);
        union.max = union.max.max(bounds.max);
        *count += 1;
    }
    let mut layered = Vec::new();
    let mut orders = BTreeMap::new();
    for (i, (placement, surface)) in pieces.into_iter().enumerate() {
        let group = root(&mut parents, i);
        let (bounds, count) = groups[&group];
        if count == 1 {
            ordinary.push(surface);
        } else {
            let (last, order) = orders.entry(group).or_insert((placement, 0));
            if *last != placement {
                *last = placement;
                *order += 1;
            }
            layered.push((
                surface,
                DecalLayer {
                    bounds,
                    order: *order,
                },
            ));
        }
    }
    layered
}
