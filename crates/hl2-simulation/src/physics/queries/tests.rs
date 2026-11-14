use super::{Queries, REBALANCE_EVERY};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::{
    nalgebra, vector, Aabb, Ball, ColliderBuilder, ColliderHandle, ColliderSet, Cuboid,
    IslandManager, Isometry, Point, QueryFilter, QueryPipeline, Ray, RigidBodySet,
};

/// Deterministic values in 0..1.
struct Sequence(u64);
impl Sequence {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn place(&mut self) -> nalgebra::Vector3<f32> {
        vector![self.next(), self.next(), self.next()] * 60.
    }
    fn pick(&mut self, count: usize) -> usize {
        ((self.next() * count as f32) as usize).min(count - 1)
    }
}

fn collider(values: &mut Sequence) -> rapier3d::prelude::Collider {
    let size = 0.3 + values.next() * 2.;
    let shape = if values.next() < 0.5 {
        ColliderBuilder::ball(size)
    } else {
        ColliderBuilder::cuboid(size, size * 0.5, size * 1.5)
    };
    shape.translation(values.place()).build()
}

struct World {
    bodies: RigidBodySet,
    colliders: ColliderSet,
    handles: Vec<ColliderHandle>,
    reference: QueryPipeline,
    owned: Queries,
}

impl World {
    /// Every query the game uses returns the same result from both trees.
    fn compare(&self, values: &mut Sequence) {
        let (bodies, colliders) = (&self.bodies, &self.colliders);
        let filter = QueryFilter::default();
        for _ in 0..12 {
            let origin = Point::from(values.place());
            let direction = (values.place() - vector![30., 30., 30.]).normalize();
            let ray = Ray::new(origin, direction);
            assert_eq!(
                self.reference
                    .cast_ray(bodies, colliders, &ray, 80., true, filter),
                self.owned
                    .cast_ray(bodies, colliders, &ray, 80., true, filter)
            );
            let normal = |hit: Option<(ColliderHandle, rapier3d::prelude::RayIntersection)>| {
                hit.map(|(handle, hit)| (handle, hit.time_of_impact, hit.normal))
            };
            assert_eq!(
                normal(
                    self.reference
                        .cast_ray_and_get_normal(bodies, colliders, &ray, 80., false, filter)
                ),
                normal(
                    self.owned
                        .cast_ray_and_get_normal(bodies, colliders, &ray, 80., false, filter)
                )
            );
            let projection = |hit: Option<(ColliderHandle, rapier3d::prelude::PointProjection)>| {
                hit.map(|(handle, hit)| (handle, hit.point, hit.is_inside))
            };
            assert_eq!(
                projection(
                    self.reference
                        .project_point(bodies, colliders, &origin, true, filter)
                ),
                projection(
                    self.owned
                        .project_point(bodies, colliders, &origin, true, filter)
                )
            );
            let at = Isometry::translation(origin.x, origin.y, origin.z);
            let probe = Cuboid::new(vector![1.5, 1., 2.]);
            let options = ShapeCastOptions::with_max_time_of_impact(40.);
            // A cast that starts overlapping several colliders may report any of them: the first
            // one visited wins, and the visiting order depends on the tree's shape.
            let cast = |hit: Option<(ColliderHandle, rapier3d::parry::query::ShapeCastHit)>| {
                hit.map(|(handle, hit)| {
                    let named = (hit.time_of_impact > 0.).then_some((handle, hit.normal1));
                    (named, hit.time_of_impact)
                })
            };
            assert_eq!(
                cast(
                    self.reference
                        .cast_shape(bodies, colliders, &at, &direction, &probe, options, filter)
                ),
                cast(self.owned.cast_shape(
                    (bodies, colliders),
                    &at,
                    &direction,
                    &probe,
                    options,
                    filter
                ))
            );
            let ball = Ball::new(4.);
            let (mut expected, mut found) = (Vec::new(), Vec::new());
            self.reference.intersections_with_shape(
                bodies,
                colliders,
                &at,
                &ball,
                filter,
                |handle| {
                    expected.push(handle);
                    true
                },
            );
            self.owned
                .intersections_with_shape(bodies, colliders, &at, &ball, filter, |handle| {
                    found.push(handle);
                    true
                });
            expected.sort_by_key(|handle| handle.into_raw_parts());
            found.sort_by_key(|handle| handle.into_raw_parts());
            assert_eq!(expected, found);
            // The box query answers from the boxes stored in the tree, which both trees keep
            // looser than the collider when a move stayed inside the stored box. Which boxes are
            // loose depends on how leaves are grouped, so both are checked against the colliders
            // themselves: neither may miss one whose box overlaps.
            let aabb = Aabb::from_half_extents(origin, vector![5., 3., 4.]);
            let (mut library, mut found) = (Vec::new(), Vec::new());
            self.reference
                .colliders_with_aabb_intersecting_aabb(&aabb, |handle| {
                    library.push(*handle);
                    true
                });
            self.owned
                .colliders_with_aabb_intersecting_aabb(&aabb, |handle| {
                    found.push(*handle);
                    true
                });
            for (handle, collider) in colliders.iter() {
                if rapier3d::prelude::BoundingVolume::intersects(&collider.compute_aabb(), &aabb) {
                    assert!(library.contains(&handle) && found.contains(&handle));
                }
            }
            assert!(
                found.len() <= library.len() + 8,
                "{} {}",
                found.len(),
                library.len()
            );
        }
    }
}

#[test]
fn queries_equal_the_library_pipeline_through_moves_inserts_and_removals() {
    let mut values = Sequence(11);
    let mut world = World {
        bodies: RigidBodySet::new(),
        colliders: ColliderSet::new(),
        handles: Vec::new(),
        reference: QueryPipeline::new(),
        owned: Queries::default(),
    };
    for _ in 0..500 {
        let handle = world.colliders.insert(collider(&mut values));
        world.handles.push(handle);
    }
    world.reference.update(&world.colliders);
    world.owned.update(&world.colliders);
    world.compare(&mut values);
    let mut islands = IslandManager::new();
    let mut without_rebalance = 0;
    for round in 0..300 {
        let (mut modified, mut removed) = (Vec::new(), Vec::new());
        for _ in 0..(round % 25) {
            let handle = world.handles[values.pick(world.handles.len())];
            let step = (values.place() - vector![30., 30., 30.]) * 0.05;
            let collider = world.colliders.get_mut(handle).expect("collider");
            collider.set_translation(collider.translation() + step);
            modified.push(handle);
        }
        // Some rounds also remove a collider and add one (which may reuse its handle index).
        if round % 7 == 3 {
            let handle = world.handles.swap_remove(values.pick(world.handles.len()));
            world
                .colliders
                .remove(handle, &mut islands, &mut world.bodies, false);
            removed.push(handle);
            let handle = world.colliders.insert(collider(&mut values));
            world.handles.push(handle);
            modified.push(handle);
        }
        world
            .reference
            .update_incremental(&world.colliders, &modified, &removed, true);
        world
            .owned
            .update_incremental(&world.colliders, &modified, &removed, true);
        without_rebalance += u32::from(world.owned.refits > 0);
        assert!(world.owned.refits < REBALANCE_EVERY && !world.owned.reshaped);
        world.compare(&mut values);
    }
    // Most rounds refit without rebalancing; the library pipeline rebalanced in every one.
    assert!(without_rebalance > 200, "{without_rebalance}");
}

#[test]
fn a_full_rebuild_drops_disabled_colliders_and_resets_the_schedule() {
    let mut values = Sequence(3);
    let mut colliders = ColliderSet::new();
    let handles: Vec<_> = (0..40)
        .map(|_| colliders.insert(collider(&mut values)))
        .collect();
    let mut owned = Queries::default();
    owned.update(&colliders);
    colliders
        .get_mut(handles[0])
        .unwrap()
        .set_translation(vector![500., 0., 0.]);
    owned.update_incremental(&colliders, &handles[..1], &[], true);
    assert_eq!(owned.refits, 1);
    colliders.get_mut(handles[0]).unwrap().set_enabled(false);
    owned.update(&colliders);
    assert_eq!(owned.refits, 0);
    let mut found = Vec::new();
    owned.colliders_with_aabb_intersecting_aabb(
        &Aabb::from_half_extents(Point::new(500., 0., 0.), vector![10., 10., 10.]),
        |handle| {
            found.push(*handle);
            true
        },
    );
    assert!(found.is_empty());
}

#[test]
fn an_entity_set_to_its_present_state_is_not_reported_to_the_step() {
    use glam::{Quat, Vec3};
    let mut physics = crate::physics::Physics::new(&modkit_core::World::default());
    let (mins, maxs) = (Vec3::new(-13., -13., 0.), Vec3::new(13., 13., 72.));
    let origin = Vec3::new(100., 0., 0.);
    physics.add_entity_hull(7, mins, maxs, origin);
    physics.tick(0.015);
    assert_eq!(physics.query.reported, (1, 0), "the new hull");
    // Enabled then disabled: the change is reported once, the repeats not at all. A disabled
    // collider that is touched again would be reported as removed on every step.
    for enabled in [true, false] {
        for _ in 0..3 {
            physics.set_entity(7, origin, Quat::IDENTITY, enabled);
            physics.tick(0.015);
        }
        assert_eq!(physics.query.reported, (0, 0), "enabled {enabled}");
    }
    // Moving while disabled is not reported either; enabling places the hull where it now is.
    let moved = origin + Vec3::Y * 300.;
    for step in 1..=3 {
        physics.set_entity(
            7,
            origin + Vec3::Y * 100. * step as f32,
            Quat::IDENTITY,
            false,
        );
        physics.tick(0.015);
        assert_eq!(physics.query.reported, (0, 0));
    }
    physics.set_entity(7, moved, Quat::IDENTITY, true);
    physics.refresh_entity_queries();
    let from = Vec3::new(0., 300., 36.);
    let hit = physics.projectile_ray(from, from + Vec3::X * 200., &[]);
    assert_eq!(hit.map(|hit| hit.entity), Some(7));
}
