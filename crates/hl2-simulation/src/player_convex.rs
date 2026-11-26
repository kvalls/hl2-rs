//! Stable continuous SAT planes for a translating player AABB and convex props.
//! These are query shapes only; Rapier retains the rigid-body/weapon colliders.
use super::SCALE;
use glam::{Quat, Vec3};
use modkit_core::{Brush, Plane};
use rapier3d::prelude::{Collider, ColliderHandle, SharedShape};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct ConvexCache {
    entries: HashMap<ColliderHandle, Cached>,
}

struct Geometry {
    vertices: Vec<Vec3>,
    faces: Vec<Vec3>,
    edges: Vec<Vec3>,
}

struct Cached {
    shape: SharedShape,
    geometry: Geometry,
    rotation: Quat,
    translation: Vec3,
    brush: Brush,
    base_distances: Vec<f32>,
}

pub(super) fn supported(collider: &Collider) -> bool {
    collider.shape().as_cuboid().is_some() || collider.shape().as_convex_polyhedron().is_some()
}

impl Geometry {
    fn new(collider: &Collider) -> Option<Self> {
        if let Some(cuboid) = collider.shape().as_cuboid() {
            let h = Vec3::new(
                cuboid.half_extents.x,
                cuboid.half_extents.y,
                cuboid.half_extents.z,
            ) / SCALE;
            let mut vertices = Vec::with_capacity(8);
            for x in [-h.x, h.x] {
                for y in [-h.y, h.y] {
                    for z in [-h.z, h.z] {
                        vertices.push(Vec3::new(x, y, z));
                    }
                }
            }
            return Some(Self {
                vertices,
                faces: vec![Vec3::X, Vec3::Y, Vec3::Z],
                edges: vec![Vec3::X, Vec3::Y, Vec3::Z],
            });
        }
        let polyhedron = collider.shape().as_convex_polyhedron()?;
        Some(Self {
            vertices: polyhedron
                .points()
                .iter()
                .map(|p| Vec3::new(p.x, p.y, p.z) / SCALE)
                .collect(),
            faces: polyhedron
                .faces()
                .iter()
                .map(|f| Vec3::new(f.normal.x, f.normal.y, f.normal.z))
                .collect(),
            edges: polyhedron
                .edges()
                .iter()
                .map(|e| Vec3::new(e.dir.x, e.dir.y, e.dir.z))
                .collect(),
        })
    }

    fn planes(&self, rotation: Quat) -> Brush {
        let mut axes = Vec::new();
        let mut seen = HashSet::new();
        let mut add = |axis: Vec3| {
            let Some(mut n) = axis.try_normalize() else {
                return;
            };
            // Opposite SAT axes specify the same projection interval. Deduplicate
            // exact directions only, without discarding nearly parallel bevels.
            let first = if n.x != 0. {
                n.x
            } else if n.y != 0. {
                n.y
            } else {
                n.z
            };
            if first < 0. {
                n = -n;
            }
            let key = (
                if n.x == 0. { 0 } else { n.x.to_bits() },
                if n.y == 0. { 0 } else { n.y.to_bits() },
                if n.z == 0. { 0 } else { n.z.to_bits() },
            );
            if seen.insert(key) {
                axes.push(n);
            }
        };
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            add(axis);
        }
        for normal in &self.faces {
            add(rotation * *normal);
        }
        // Face planes alone omit the Minkowski edge bevels. The full polyhedron
        // versus AABB SAT also requires every convex edge crossed with box edges.
        for edge in &self.edges {
            let world_edge = rotation * *edge;
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                add(world_edge.cross(axis));
            }
        }
        let vertices: Vec<_> = self.vertices.iter().map(|v| rotation * *v).collect();
        let mut planes = Vec::with_capacity(axes.len() * 2);
        for n in axes {
            let (low, high) =
                vertices
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), vertex| {
                        let projection = n.dot(*vertex);
                        (low.min(projection), high.max(projection))
                    });
            planes.push(Plane {
                normal: n,
                distance: high,
            });
            planes.push(Plane {
                normal: -n,
                distance: -low,
            });
        }
        Brush {
            contents: 1,
            planes,
        }
    }
}

impl ConvexCache {
    pub(super) fn brush(&mut self, handle: ColliderHandle, collider: &Collider) -> Option<&Brush> {
        let position = collider.position();
        let q = position.rotation;
        let rotation = Quat::from_xyzw(q.x, q.y, q.z, q.w);
        let t = position.translation;
        let translation = Vec3::new(t.x, t.y, t.z) / SCALE;
        let replace = self
            .entries
            .get(&handle)
            .is_none_or(|entry| !Arc::ptr_eq(&entry.shape.0, &collider.shared_shape().0));
        if replace {
            let geometry = Geometry::new(collider)?;
            let mut brush = geometry.planes(rotation);
            let base_distances: Vec<_> = brush.planes.iter().map(|p| p.distance).collect();
            translate(&mut brush, &base_distances, translation);
            self.entries.insert(
                handle,
                Cached {
                    shape: collider.shared_shape().clone(),
                    geometry,
                    rotation,
                    translation,
                    brush,
                    base_distances,
                },
            );
        }
        let cached = self.entries.get_mut(&handle)?;
        let rotated = cached.rotation != rotation;
        if rotated {
            cached.brush = cached.geometry.planes(rotation);
            cached.base_distances = cached.brush.planes.iter().map(|p| p.distance).collect();
            cached.rotation = rotation;
        }
        if rotated || cached.translation != translation {
            // Pure translation preserves axes and support extrema, so a moving
            // unrotated door/prop only updates its plane distances.
            translate(&mut cached.brush, &cached.base_distances, translation);
            cached.translation = translation;
        }
        Some(&cached.brush)
    }

    pub(super) fn retain(&mut self, colliders: &rapier3d::prelude::ColliderSet) {
        self.entries.retain(|handle, _| colliders.contains(*handle));
    }
}

fn translate(brush: &mut Brush, base_distances: &[f32], translation: Vec3) {
    for (plane, base) in brush.planes.iter_mut().zip(base_distances) {
        // Recompute from the absolute pose: incremental distance additions would
        // accumulate rounding error across repeated prop/door movements.
        plane.distance = *base + plane.normal.dot(translation);
    }
}
