//! Split broad phase separating static world geometry from moving colliders.
//!
//! Keeping fixed-only world colliders in a separate `Qbvh` avoids refitting or self-testing
//! thousands of static brushes every step while allowing seamless promotion to the moving tree
//! when a fixed brush entity (such as a door) moves or changes body type.

use rapier3d::parry::partitioning::{Qbvh, QbvhDataGenerator, QbvhUpdateWorkspace};
use rapier3d::parry::query::visitors::BoundingVolumeIntersectionsVisitor;
use rapier3d::prelude::{
    Aabb, ActiveCollisionTypes, BoundingVolume, BroadPhase, BroadPhasePairEvent, Collider,
    ColliderHandle, ColliderPair, ColliderSet, Real, RigidBodySet,
};
use std::cmp::Ordering;
use std::collections::HashMap;

/// Refits that changed the moving tree between rebalances, at most.
const REBALANCE_EVERY: u32 = 64;

#[derive(Copy, Clone, Debug, PartialEq)]
struct MovingEntry {
    aabb: Aabb,
    fixed_only: bool,
}

struct UpdateContext<'a> {
    dt: Real,
    prediction_distance: Real,
    colliders: &'a ColliderSet,
    bodies: &'a RigidBodySet,
}

#[derive(Default)]
struct UpdateFlags {
    static_dirty: bool,
    moving_reshaped: bool,
    moving_touched: bool,
}

/// Two-tree `BroadPhase` separating static world geometry from moving and active colliders.
#[derive(Default)]
pub(crate) struct SplitBroadPhase {
    static_tree: Qbvh<ColliderHandle>,
    static_aabbs: HashMap<ColliderHandle, Aabb>,
    /// Sparse reverse map: static colliders actively overlapping at least one moving collider,
    /// each sorted by `handle_key`.
    static_partners: HashMap<ColliderHandle, Vec<ColliderHandle>>,
    static_initialized: bool,
    static_prediction: Real,
    moving_tree: Qbvh<ColliderHandle>,
    moving: HashMap<ColliderHandle, MovingEntry>,
    /// Per moving collider, sorted list of partner handles from `moving` or `static_aabbs`.
    active_pairs: HashMap<ColliderHandle, Vec<ColliderHandle>>,
    moving_refits: u32,
    workspace: QbvhUpdateWorkspace,
    traversal_stack: Vec<u32>,
    dirty_moving: Vec<ColliderHandle>,
    new_hits: Vec<ColliderHandle>,
    bulk_static: Vec<(ColliderHandle, Aabb)>,
}

struct SliceBoxes<'a>(&'a [(ColliderHandle, Aabb)]);

impl QbvhDataGenerator<ColliderHandle> for SliceBoxes<'_> {
    fn size_hint(&self) -> usize {
        self.0.len()
    }

    fn for_each(&mut self, mut f: impl FnMut(ColliderHandle, Aabb)) {
        for &(handle, aabb) in self.0 {
            f(handle, aabb);
        }
    }
}

fn handle_key(h: ColliderHandle) -> (u32, u32) {
    h.into_raw_parts()
}

fn canonical_pair(a: ColliderHandle, b: ColliderHandle) -> ColliderPair {
    if handle_key(a) <= handle_key(b) {
        ColliderPair::new(a, b)
    } else {
        ColliderPair::new(b, a)
    }
}

fn is_fixed_only(collider: &Collider, bodies: &RigidBodySet) -> bool {
    collider
        .parent()
        .is_none_or(|p| bodies.get(p).is_none_or(|b| b.is_fixed()))
        && !collider
            .active_collision_types()
            .contains(ActiveCollisionTypes::FIXED_FIXED)
}

fn collider_aabb(
    dt: Real,
    prediction_distance: Real,
    collider: &Collider,
    bodies: &RigidBodySet,
) -> Option<Aabb> {
    let half_pred = prediction_distance / 2.0;
    let mut aabb = collider.compute_collision_aabb(half_pred);
    if let Some((parent, wrt)) = collider
        .parent()
        .and_then(|p| bodies.get(p).zip(collider.position_wrt_parent()))
    {
        if parent.soft_ccd_prediction() > 0.0 {
            let next_pos = parent.predict_position_using_velocity_and_forces(dt) * *wrt;
            let next_aabb = collider
                .shape()
                .compute_aabb(&next_pos)
                .loosened(collider.contact_skin() + half_pred);
            aabb.merge(&next_aabb);
        }
    }
    let finite = aabb.mins.coords.iter().all(|c| c.is_finite())
        && aabb.maxs.coords.iter().all(|c| c.is_finite());
    finite.then_some(aabb)
}

fn insert_sorted(list: &mut Vec<ColliderHandle>, h: ColliderHandle) {
    if let Err(idx) = list.binary_search_by_key(&handle_key(h), |x| handle_key(*x)) {
        list.insert(idx, h);
    }
}

fn remove_sorted(list: &mut Vec<ColliderHandle>, h: ColliderHandle) -> bool {
    if let Ok(idx) = list.binary_search_by_key(&handle_key(h), |x| handle_key(*x)) {
        list.remove(idx);
        true
    } else {
        false
    }
}

impl SplitBroadPhase {
    /// Rebuilds the static tree when `prediction_distance` grows past what its AABBs include.
    fn rebuild_static_if_prediction_grew(
        &mut self,
        ctx: &UpdateContext<'_>,
        events: &mut Vec<BroadPhasePairEvent>,
    ) {
        if !self.static_initialized
            || ctx.prediction_distance <= self.static_prediction
            || self.static_aabbs.is_empty()
        {
            return;
        }
        self.bulk_static.clear();
        let static_keys: Vec<ColliderHandle> = self.static_aabbs.keys().copied().collect();
        for h in static_keys {
            let Some(co) = ctx.colliders.get(h).filter(|c| c.is_enabled()) else {
                self.remove_static(h, events);
                continue;
            };
            let Some(aabb) = collider_aabb(ctx.dt, ctx.prediction_distance, co, ctx.bodies) else {
                self.remove_static(h, events);
                continue;
            };
            self.static_aabbs.insert(h, aabb);
            self.bulk_static.push((h, aabb));
        }
        self.static_tree
            .clear_and_rebuild(SliceBoxes(&self.bulk_static), 0.0);
        self.static_prediction = ctx.prediction_distance;
        for (&m, entry) in &self.moving {
            if !entry.fixed_only {
                self.dirty_moving.push(m);
            }
        }
    }

    fn remove_static(&mut self, h: ColliderHandle, events: &mut Vec<BroadPhasePairEvent>) -> bool {
        if self.static_aabbs.remove(&h).is_none() {
            return false;
        }
        self.static_tree.remove(h);
        if let Some(partners) = self.static_partners.remove(&h) {
            for m in partners {
                if let Some(list) = self.active_pairs.get_mut(&m) {
                    remove_sorted(list, h);
                }
                events.push(BroadPhasePairEvent::DeletePair(canonical_pair(m, h)));
            }
        }
        true
    }

    fn remove_moving(&mut self, h: ColliderHandle, events: &mut Vec<BroadPhasePairEvent>) -> bool {
        if self.moving.remove(&h).is_none() {
            return false;
        }
        self.moving_tree.remove(h);
        if let Some(partners) = self.active_pairs.remove(&h) {
            for other in partners {
                if self.static_aabbs.contains_key(&other) {
                    if let Some(list) = self.static_partners.get_mut(&other) {
                        remove_sorted(list, h);
                        if list.is_empty() {
                            self.static_partners.remove(&other);
                        }
                    }
                } else if let Some(list) = self.active_pairs.get_mut(&other) {
                    remove_sorted(list, h);
                }
                events.push(BroadPhasePairEvent::DeletePair(canonical_pair(h, other)));
            }
        }
        true
    }

    fn promote_static_to_moving(&mut self, h: ColliderHandle, entry: MovingEntry) {
        self.static_aabbs.remove(&h);
        self.static_tree.remove(h);
        // Inherit active moving partners so ongoing overlaps emit no spurious DeletePair/AddPair.
        let inherited = self.static_partners.remove(&h).unwrap_or_default();
        self.active_pairs.insert(h, inherited);
        self.moving.insert(h, entry);
        self.moving_tree.pre_update_or_insert(h);
        self.dirty_moving.push(h);
    }

    fn process_modified(
        &mut self,
        ctx: &UpdateContext<'_>,
        modified_colliders: &[ColliderHandle],
        events: &mut Vec<BroadPhasePairEvent>,
    ) -> UpdateFlags {
        let initial_bulk = !self.static_initialized;
        self.bulk_static.clear();
        let mut flags = UpdateFlags::default();
        for &h in modified_colliders {
            self.process_one_modified(ctx, h, initial_bulk, &mut flags, events);
        }
        flags
    }

    fn process_one_modified(
        &mut self,
        ctx: &UpdateContext<'_>,
        h: ColliderHandle,
        initial_bulk: bool,
        flags: &mut UpdateFlags,
        events: &mut Vec<BroadPhasePairEvent>,
    ) {
        let Some(co) = ctx.colliders.get(h).filter(|c| c.is_enabled()) else {
            flags.static_dirty |= self.remove_static(h, events);
            flags.moving_reshaped |= self.remove_moving(h, events);
            return;
        };
        let fixed_only = is_fixed_only(co, ctx.bodies);
        if let Some(&old_aabb) = self.static_aabbs.get(&h) {
            let Some(new_aabb) = collider_aabb(ctx.dt, ctx.prediction_distance, co, ctx.bodies)
            else {
                flags.static_dirty |= self.remove_static(h, events);
                return;
            };
            if fixed_only && new_aabb == old_aabb {
                return;
            }
            self.promote_static_to_moving(
                h,
                MovingEntry {
                    aabb: new_aabb,
                    fixed_only,
                },
            );
            flags.static_dirty = true;
            flags.moving_reshaped = true;
            return;
        }
        if let Some(entry) = self.moving.get_mut(&h) {
            let Some(new_aabb) = collider_aabb(ctx.dt, ctx.prediction_distance, co, ctx.bodies)
            else {
                flags.moving_reshaped |= self.remove_moving(h, events);
                return;
            };
            if entry.fixed_only == fixed_only && entry.aabb == new_aabb {
                return;
            }
            entry.aabb = new_aabb;
            entry.fixed_only = fixed_only;
            self.moving_tree.pre_update_or_insert(h);
            flags.moving_touched = true;
            self.dirty_moving.push(h);
            return;
        }
        let Some(new_aabb) = collider_aabb(ctx.dt, ctx.prediction_distance, co, ctx.bodies) else {
            return;
        };
        if initial_bulk && fixed_only {
            self.static_aabbs.insert(h, new_aabb);
            self.bulk_static.push((h, new_aabb));
        } else {
            self.moving.insert(
                h,
                MovingEntry {
                    aabb: new_aabb,
                    fixed_only,
                },
            );
            self.active_pairs.entry(h).or_default();
            self.moving_tree.pre_update_or_insert(h);
            flags.moving_reshaped = true;
            self.dirty_moving.push(h);
        }
    }

    fn finalize_static_tree(
        &mut self,
        initial_bulk: bool,
        static_dirty: bool,
        prediction_distance: Real,
    ) {
        if initial_bulk && !self.bulk_static.is_empty() {
            self.bulk_static
                .retain(|(h, _)| self.static_aabbs.contains_key(h));
            self.bulk_static
                .sort_unstable_by_key(|(h, _)| handle_key(*h));
            self.bulk_static.dedup_by_key(|(h, _)| handle_key(*h));
            for (h, aabb) in &mut self.bulk_static {
                *aabb = self.static_aabbs[h];
            }
            self.static_tree
                .clear_and_rebuild(SliceBoxes(&self.bulk_static), 0.0);
            self.static_initialized = true;
            self.static_prediction = prediction_distance;
            // Existing non-fixed moving colliders (inserted before static world) must query it.
            for (&m, entry) in &self.moving {
                if !entry.fixed_only {
                    self.dirty_moving.push(m);
                }
            }
        } else if static_dirty {
            if self.static_aabbs.is_empty() {
                self.static_tree.clear_and_rebuild(SliceBoxes(&[]), 0.0);
            } else {
                self.static_tree
                    .refit(0.0, &mut self.workspace, |h| self.static_aabbs[h]);
            }
        }
    }

    fn finalize_moving_tree(&mut self, reshaped: bool, touched: bool) {
        if !reshaped && !touched {
            return;
        }
        if self.moving.is_empty() {
            self.moving_tree.clear_and_rebuild(SliceBoxes(&[]), 0.0);
            self.moving_refits = 0;
            return;
        }
        let changed = self
            .moving_tree
            .refit(0.0, &mut self.workspace, |h| self.moving[h].aabb);
        self.moving_refits += u32::from(changed > 0);
        if reshaped || self.moving_refits >= REBALANCE_EVERY {
            self.moving_tree.rebalance(0.0, &mut self.workspace);
            self.moving_refits = 0;
        }
    }

    fn collect_hits(&mut self, m: ColliderHandle, entry: MovingEntry) {
        self.new_hits.clear();
        let moving = &self.moving;
        let new_hits = &mut self.new_hits;
        let mut moving_visitor =
            BoundingVolumeIntersectionsVisitor::new(&entry.aabb, |other: &ColliderHandle| {
                if *other != m {
                    if let Some(other_entry) = moving.get(other) {
                        if !(entry.fixed_only && other_entry.fixed_only)
                            && entry.aabb.intersects(&other_entry.aabb)
                        {
                            new_hits.push(*other);
                        }
                    }
                }
                true
            });
        self.moving_tree
            .traverse_depth_first_with_stack(&mut moving_visitor, &mut self.traversal_stack);

        if !entry.fixed_only && !self.static_aabbs.is_empty() {
            let static_aabbs = &self.static_aabbs;
            let mut static_visitor =
                BoundingVolumeIntersectionsVisitor::new(&entry.aabb, |other: &ColliderHandle| {
                    if static_aabbs
                        .get(other)
                        .is_some_and(|b| entry.aabb.intersects(b))
                    {
                        new_hits.push(*other);
                    }
                    true
                });
            self.static_tree
                .traverse_depth_first_with_stack(&mut static_visitor, &mut self.traversal_stack);
        }

        self.new_hits.sort_unstable_by_key(|h| handle_key(*h));
        self.new_hits.dedup_by_key(|h| handle_key(*h));
    }

    fn emit_removed_pair(
        &mut self,
        m: ColliderHandle,
        other: ColliderHandle,
        events: &mut Vec<BroadPhasePairEvent>,
    ) {
        events.push(BroadPhasePairEvent::DeletePair(canonical_pair(m, other)));
        if self.static_aabbs.contains_key(&other) {
            if let Some(list) = self.static_partners.get_mut(&other) {
                remove_sorted(list, m);
                if list.is_empty() {
                    self.static_partners.remove(&other);
                }
            }
        } else if let Some(list) = self.active_pairs.get_mut(&other) {
            remove_sorted(list, m);
        }
    }

    fn emit_added_pair(
        &mut self,
        m: ColliderHandle,
        other: ColliderHandle,
        events: &mut Vec<BroadPhasePairEvent>,
    ) {
        events.push(BroadPhasePairEvent::AddPair(canonical_pair(m, other)));
        if self.static_aabbs.contains_key(&other) {
            let list = self.static_partners.entry(other).or_default();
            insert_sorted(list, m);
        } else {
            let list = self.active_pairs.entry(other).or_default();
            insert_sorted(list, m);
        }
    }

    fn diff_pairs(&mut self, m: ColliderHandle, events: &mut Vec<BroadPhasePairEvent>) {
        let mut old = self.active_pairs.remove(&m).unwrap_or_default();
        let new = std::mem::take(&mut self.new_hits);
        let (mut i, mut j) = (0, 0);
        while i < old.len() && j < new.len() {
            match handle_key(old[i]).cmp(&handle_key(new[j])) {
                Ordering::Less => {
                    self.emit_removed_pair(m, old[i], events);
                    i += 1;
                }
                Ordering::Greater => {
                    self.emit_added_pair(m, new[j], events);
                    j += 1;
                }
                Ordering::Equal => {
                    i += 1;
                    j += 1;
                }
            }
        }
        while i < old.len() {
            self.emit_removed_pair(m, old[i], events);
            i += 1;
        }
        while j < new.len() {
            self.emit_added_pair(m, new[j], events);
            j += 1;
        }
        old.clear();
        old.extend_from_slice(&new);
        self.new_hits = new;
        self.active_pairs.insert(m, old);
    }

    fn flush_dirty_moving(&mut self, events: &mut Vec<BroadPhasePairEvent>) {
        if self.dirty_moving.is_empty() {
            return;
        }
        self.dirty_moving.sort_unstable_by_key(|h| handle_key(*h));
        self.dirty_moving.dedup_by_key(|h| handle_key(*h));
        let mut dirty = std::mem::take(&mut self.dirty_moving);
        for &m in &dirty {
            let Some(entry) = self.moving.get(&m).copied() else {
                continue;
            };
            self.collect_hits(m, entry);
            self.diff_pairs(m, events);
        }
        dirty.clear();
        self.dirty_moving = dirty;
    }
}

impl BroadPhase for SplitBroadPhase {
    fn update(
        &mut self,
        dt: Real,
        prediction_distance: Real,
        colliders: &mut ColliderSet,
        bodies: &RigidBodySet,
        modified: &[ColliderHandle],
        removed: &[ColliderHandle],
        out_events: &mut Vec<BroadPhasePairEvent>,
    ) {
        let ctx = UpdateContext {
            dt,
            prediction_distance,
            colliders,
            bodies,
        };
        self.rebuild_static_if_prediction_grew(&ctx, out_events);

        let mut static_dirty = false;
        let mut moving_reshaped = false;
        for &h in removed {
            static_dirty |= self.remove_static(h, out_events);
            moving_reshaped |= self.remove_moving(h, out_events);
        }

        let initial_bulk = !self.static_initialized;
        let flags = self.process_modified(&ctx, modified, out_events);
        static_dirty |= flags.static_dirty;
        moving_reshaped |= flags.moving_reshaped;

        self.finalize_static_tree(initial_bulk, static_dirty, prediction_distance);
        self.finalize_moving_tree(moving_reshaped, flags.moving_touched);
        self.flush_dirty_moving(out_events);
    }
}

#[cfg(test)]
mod tests;
