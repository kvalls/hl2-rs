//! The scene query tree, owned here so that it is refitted on every update and rebalanced on a
//! schedule. rapier3d's own `QueryPipeline` rebalances on every incremental update, which
//! rebuilds the top of the tree whether or not anything moved.
//!
//! The tree always holds a box around every collider it was told about, so a query finds the
//! same colliders as before. What depends on when the last rebalance ran is the order
//! candidates are visited in (a cast that starts overlapping several colliders reports the
//! first one visited) and which stored boxes are looser than their collider (the box query may
//! return a few more or fewer near misses; its callers test each candidate exactly).
use rapier3d::parry::{
    partitioning::{IndexedData, Qbvh, QbvhDataGenerator, QbvhUpdateWorkspace},
    query::{
        details::{
            NormalConstraints, PointCompositeShapeProjBestFirstVisitor,
            RayCompositeShapeToiAndNormalBestFirstVisitor, RayCompositeShapeToiBestFirstVisitor,
            TOICompositeShapeShapeBestFirstVisitor,
        },
        visitors::BoundingVolumeIntersectionsVisitor,
        DefaultQueryDispatcher, QueryDispatcher, ShapeCastHit, ShapeCastOptions,
    },
    shape::{Shape, TypedSimdCompositeShape},
};
use rapier3d::prelude::{
    Aabb, BroadPhase, BroadPhaseMultiSap, BroadPhasePairEvent, ColliderHandle, ColliderSet,
    Isometry, Point, PointProjection, QueryFilter, Ray, RayIntersection, Real, RigidBodySet,
    Vector,
};
use std::collections::HashSet;

/// Refits that changed the tree between two rebalances, at most.
const REBALANCE_EVERY: u32 = 64;
/// Box growth on a full rebuild, as rapier3d's pipeline uses.
const REBUILD_DILATION: Real = 0.01;

#[derive(Clone, Default)]
pub(crate) struct Queries {
    tree: Qbvh<ColliderHandle>,
    workspace: QbvhUpdateWorkspace,
    /// Refits that changed a box since the last rebalance.
    refits: u32,
    /// A collider was inserted or removed since the last rebalance.
    reshaped: bool,
    /// Colliders the last step reported as modified and as removed.
    pub reported: (usize, usize),
}

/// The current box of every enabled collider, for a full rebuild.
struct EnabledBoxes<'a>(&'a ColliderSet);
impl QbvhDataGenerator<ColliderHandle> for EnabledBoxes<'_> {
    fn size_hint(&self) -> usize {
        self.0.len()
    }
    fn for_each(&mut self, mut f: impl FnMut(ColliderHandle, Aabb)) {
        for (handle, collider) in self.0.iter_enabled() {
            f(handle, collider.compute_aabb());
        }
    }
}

/// The colliders of a tree as one shape, for the best-first visitors.
struct Scene<'a> {
    tree: &'a Qbvh<ColliderHandle>,
    bodies: &'a RigidBodySet,
    colliders: &'a ColliderSet,
    filter: QueryFilter<'a>,
}
impl TypedSimdCompositeShape for Scene<'_> {
    type PartShape = dyn Shape;
    type PartNormalConstraints = dyn NormalConstraints;
    type PartId = ColliderHandle;

    fn map_typed_part_at(
        &self,
        id: ColliderHandle,
        mut f: impl FnMut(Option<&Isometry<Real>>, &dyn Shape, Option<&dyn NormalConstraints>),
    ) {
        if let Some(collider) = self.colliders.get(id) {
            if self.filter.test(self.bodies, id, collider) {
                f(Some(collider.position()), collider.shape(), None)
            }
        }
    }
    fn map_untyped_part_at(
        &self,
        id: ColliderHandle,
        f: impl FnMut(Option<&Isometry<Real>>, &dyn Shape, Option<&dyn NormalConstraints>),
    ) {
        self.map_typed_part_at(id, f);
    }
    fn typed_qbvh(&self) -> &Qbvh<ColliderHandle> {
        self.tree
    }
}

impl Queries {
    /// Rebuilds the tree from every enabled collider.
    pub fn update(&mut self, colliders: &ColliderSet) {
        self.tree
            .clear_and_rebuild(EnabledBoxes(colliders), REBUILD_DILATION);
        self.refits = 0;
        self.reshaped = false;
    }
    /// Takes `removed` out, puts `modified` in or marks them moved, and with `refit` brings every
    /// box up to date. Queries need a refit after any change.
    pub fn update_incremental(
        &mut self,
        colliders: &ColliderSet,
        modified: &[ColliderHandle],
        removed: &[ColliderHandle],
        refit: bool,
    ) {
        // Removals first: a handle index may be reused by a collider added right after.
        for handle in removed {
            self.reshaped |= self.tree.remove(*handle).is_some();
        }
        for handle in modified {
            if colliders.contains(*handle) {
                self.reshaped |= !self.holds(*handle);
                self.tree.pre_update_or_insert(*handle);
            }
        }
        if !refit {
            return;
        }
        let changed = self.tree.refit(0., &mut self.workspace, |handle| {
            colliders[*handle].compute_aabb()
        });
        self.refits += u32::from(changed > 0);
        if self.reshaped || self.refits >= REBALANCE_EVERY {
            self.tree.rebalance(0., &mut self.workspace);
            self.refits = 0;
            self.reshaped = false;
        }
    }
    /// The update the step would have made: the colliders it reported, then a refit.
    pub fn apply_step(&mut self, colliders: &ColliderSet, broad: &mut RecordingBroadPhase) {
        self.reported = (broad.modified.len(), broad.removed.len());
        self.update_incremental(colliders, &broad.modified, &broad.removed, true);
        broad.modified.clear();
        broad.removed.clear();
    }
    fn holds(&self, handle: ColliderHandle) -> bool {
        self.tree
            .raw_proxies()
            .get(handle.index())
            .is_some_and(|proxy| proxy.node.index != u32::MAX && proxy.data == handle)
    }
    fn scene<'a>(
        &'a self,
        bodies: &'a RigidBodySet,
        colliders: &'a ColliderSet,
        filter: QueryFilter<'a>,
    ) -> Scene<'a> {
        Scene {
            tree: &self.tree,
            bodies,
            colliders,
            filter,
        }
    }
    pub fn cast_ray(
        &self,
        bodies: &RigidBodySet,
        colliders: &ColliderSet,
        ray: &Ray,
        max_toi: Real,
        solid: bool,
        filter: QueryFilter,
    ) -> Option<(ColliderHandle, Real)> {
        let scene = self.scene(bodies, colliders, filter);
        let mut visitor = RayCompositeShapeToiBestFirstVisitor::new(&scene, ray, max_toi, solid);
        self.tree.traverse_best_first(&mut visitor).map(|hit| hit.1)
    }
    pub fn cast_ray_and_get_normal(
        &self,
        bodies: &RigidBodySet,
        colliders: &ColliderSet,
        ray: &Ray,
        max_toi: Real,
        solid: bool,
        filter: QueryFilter,
    ) -> Option<(ColliderHandle, RayIntersection)> {
        let scene = self.scene(bodies, colliders, filter);
        let mut visitor =
            RayCompositeShapeToiAndNormalBestFirstVisitor::new(&scene, ray, max_toi, solid);
        self.tree.traverse_best_first(&mut visitor).map(|hit| hit.1)
    }
    pub fn project_point(
        &self,
        bodies: &RigidBodySet,
        colliders: &ColliderSet,
        point: &Point<Real>,
        solid: bool,
        filter: QueryFilter,
    ) -> Option<(ColliderHandle, PointProjection)> {
        let scene = self.scene(bodies, colliders, filter);
        let mut visitor = PointCompositeShapeProjBestFirstVisitor::new(&scene, point, solid);
        self.tree
            .traverse_best_first(&mut visitor)
            .map(|hit| (hit.1 .1, hit.1 .0))
    }
    /// Every collider whose box in the tree overlaps `aabb`, until `callback` returns false.
    pub fn colliders_with_aabb_intersecting_aabb(
        &self,
        aabb: &Aabb,
        mut callback: impl FnMut(&ColliderHandle) -> bool,
    ) {
        let mut visitor = BoundingVolumeIntersectionsVisitor::new(aabb, &mut callback);
        self.tree.traverse_depth_first(&mut visitor);
    }
    pub fn cast_shape(
        &self,
        (bodies, colliders): (&RigidBodySet, &ColliderSet),
        shape_pos: &Isometry<Real>,
        shape_vel: &Vector<Real>,
        shape: &dyn Shape,
        options: ShapeCastOptions,
        filter: QueryFilter,
    ) -> Option<(ColliderHandle, ShapeCastHit)> {
        let scene = self.scene(bodies, colliders, filter);
        let mut visitor = TOICompositeShapeShapeBestFirstVisitor::new(
            &DefaultQueryDispatcher,
            shape_pos,
            shape_vel,
            &scene,
            shape,
            options,
        );
        self.tree.traverse_best_first(&mut visitor).map(|hit| hit.1)
    }
    /// Every collider passing `filter` that `shape` at `shape_pos` touches, until `callback`
    /// returns false.
    pub fn intersections_with_shape(
        &self,
        bodies: &RigidBodySet,
        colliders: &ColliderSet,
        shape_pos: &Isometry<Real>,
        shape: &dyn Shape,
        filter: QueryFilter,
        mut callback: impl FnMut(ColliderHandle) -> bool,
    ) {
        let inverse = shape_pos.inverse();
        let mut leaf = |handle: &ColliderHandle| {
            let Some(collider) = colliders.get(*handle) else {
                return true;
            };
            if !filter.test(bodies, *handle, collider) {
                return true;
            }
            let relative = inverse * collider.position();
            match DefaultQueryDispatcher.intersection_test(&relative, shape, collider.shape()) {
                Ok(true) => callback(*handle),
                _ => true,
            }
        };
        let aabb = shape.compute_aabb(shape_pos);
        let mut visitor = BoundingVolumeIntersectionsVisitor::new(&aabb, &mut leaf);
        self.tree.traverse_depth_first(&mut visitor);
    }
}

/// rapier3d's broad phase, noting the colliders each step reports as modified or removed (the
/// same lists the step would hand to a query pipeline), with trace-only colliders kept out.
#[derive(Default)]
pub(crate) struct RecordingBroadPhase {
    inner: BroadPhaseMultiSap,
    pub modified: Vec<ColliderHandle>,
    pub removed: Vec<ColliderHandle>,
    /// Colliders only traces see (NPC hitboxes). The inner broad phase never holds them, so
    /// they form no contact pairs: they never touch or wake a body (a moving hitbox woke every
    /// body whose box overlapped it), and moving them costs no broad-phase update (about 1,000
    /// in a crowd of 65 NPCs). Once Rapier paired them with props, which solved contacts
    /// against every animated hitbox (owner report: a pallet sliding off a plaza cop dropped
    /// 46 -> 17 fps). A collider joins the set before the next step, which would hand it on.
    pub trace_only: HashSet<ColliderHandle>,
    /// The step's modified and removed lists without trace-only colliders (reused).
    forwarded: (Vec<ColliderHandle>, Vec<ColliderHandle>),
}
impl BroadPhase for RecordingBroadPhase {
    fn update(
        &mut self,
        dt: Real,
        prediction_distance: Real,
        colliders: &mut ColliderSet,
        bodies: &RigidBodySet,
        modified_colliders: &[ColliderHandle],
        removed_colliders: &[ColliderHandle],
        events: &mut Vec<BroadPhasePairEvent>,
    ) {
        self.modified.extend_from_slice(modified_colliders);
        self.removed.extend_from_slice(removed_colliders);
        let (mut modified, mut removed) = std::mem::take(&mut self.forwarded);
        let pairing = |handle: &&ColliderHandle| !self.trace_only.contains(*handle);
        modified.clear();
        modified.extend(modified_colliders.iter().filter(pairing));
        removed.clear();
        removed.extend(removed_colliders.iter().filter(pairing));
        self.inner.update(
            dt,
            prediction_distance,
            colliders,
            bodies,
            &modified,
            &removed,
            events,
        );
        self.forwarded = (modified, removed);
    }
}

#[cfg(test)]
mod tests;
