//! NPC hull traces against world brushes, actor bounds and rigid colliders. Walking NPCs run
//! several per tick, so a trace skips whatever its swept box proves it cannot hit.
use super::{
    player_convex, pose, trace_brushes, vector, ActorHull, Brush, Collider, ColliderHandle, Cuboid,
    Hull, HullTrace, NpcCollisionWorld, Physics, Point, Quat, Query, QueryFilter, ShapeCastOptions,
    Trace, Vec3, CONTENTS_MONSTER, HULL_GROUP, PROJECTILE_GROUP, QUERY_GROUPS, SCALE, SHADOW_GROUP,
    WEAPON_GROUP,
};

/// Beyond the 1/32 unit trace distance and the Parry fallback's 0.001 minimum half extent.
const SWEEP_MARGIN: f32 = 0.04;

#[derive(Clone, Copy)]
struct Sweep {
    start: Vec3,
    end: Vec3,
    hull: Hull,
    min: Vec3,
    max: Vec3,
}
impl Sweep {
    fn new(start: Vec3, end: Vec3, hull: Hull) -> Self {
        let margin = Vec3::splat(SWEEP_MARGIN);
        Self {
            start,
            end,
            hull,
            min: start.min(end) + hull.mins - margin,
            max: start.max(end) + hull.maxs + margin,
        }
    }
    fn misses_box(&self, min: Vec3, max: Vec3) -> bool {
        self.max.cmplt(min).any() || self.min.cmpgt(max).any()
    }
    fn against(&self, brush: &Brush, contents_mask: u32) -> Trace {
        trace_brushes(
            std::slice::from_ref(brush),
            self.start,
            self.end,
            self.hull.mins,
            self.hull.maxs,
            contents_mask,
        )
    }
}

fn merge(result: &mut HullTrace, hit: Trace, entity: Option<usize>, transient: bool) {
    if hit.fraction < result.trace.fraction || (hit.start_solid && !result.trace.start_solid) {
        result.entity = entity;
        result.transient = transient;
        result.trace.normal = hit.normal;
    }
    result.trace.fraction = result.trace.fraction.min(hit.fraction);
    result.trace.start_solid |= hit.start_solid;
    result.trace.all_solid |= hit.all_solid;
}

fn entity_of(collider: &Collider) -> Option<usize> {
    usize::try_from(collider.user_data)
        .ok()
        .and_then(|id| id.checked_sub(1))
}

fn merge_actors(result: &mut HullTrace, sweep: &Sweep, query: Query<'_>) {
    if query.contents_mask & CONTENTS_MONSTER == 0 {
        return;
    }
    for ActorHull {
        entity,
        feet,
        hull: actor_hull,
    } in query.transients
    {
        if entity.is_some_and(|id| query.excluded_entities.contains(&id)) {
            continue;
        }
        let min = *feet + actor_hull.mins;
        let max = *feet + actor_hull.maxs;
        if sweep.misses_box(min, max) {
            continue;
        }
        let brush = Brush {
            contents: CONTENTS_MONSTER,
            planes: [
                (Vec3::X, max.x),
                (-Vec3::X, -min.x),
                (Vec3::Y, max.y),
                (-Vec3::Y, -min.y),
                (Vec3::Z, max.z),
                (-Vec3::Z, -min.z),
            ]
            .into_iter()
            .map(|(normal, distance)| modkit_core::Plane { normal, distance })
            .collect(),
        };
        merge(
            result,
            sweep.against(&brush, query.contents_mask),
            *entity,
            true,
        );
    }
}

impl Physics {
    fn accepts_for_npc(
        &self,
        handle: ColliderHandle,
        collider: &Collider,
        query: Query<'_>,
    ) -> bool {
        let entity = entity_of(collider);
        collider.is_enabled()
            && !collider.is_sensor()
            && !matches!(
                collider.collision_groups().memberships,
                PROJECTILE_GROUP | SHADOW_GROUP | HULL_GROUP | WEAPON_GROUP
            )
            && entity.is_none_or(|id| !query.excluded_entities.contains(&id))
            // Explicit actor bounds supersede an NPC's render-mesh collider.
            && !entity.is_some_and(|id| {
                self.npc_collider_contents.get(&handle) == Some(&CONTENTS_MONSTER)
                    && query.transients.iter().any(|a| a.entity == Some(id))
            })
            && self
                .npc_collider_contents
                .get(&handle)
                .copied()
                .unwrap_or(1)
                & query.contents_mask
                != 0
    }

    /// Convex colliders by their planes. Returns whether a collider only Parry can trace is
    /// near: the shape queries that follow are needed only then.
    fn merge_convex_colliders(
        &self,
        result: &mut HullTrace,
        sweep: &Sweep,
        query: Query<'_>,
    ) -> bool {
        let bounds = rapier3d::parry::bounding_volume::Aabb::new(
            Point::from(vector(sweep.min)),
            Point::from(vector(sweep.max)),
        );
        let mut cache = self
            .player_convex_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut fallback_near = false;
        self.query
            .colliders_with_aabb_intersecting_aabb(&bounds, |handle| {
                let collider = &self.colliders[*handle];
                if self.accepts_for_npc(*handle, collider, query)
                    && !self.world_brush_contents.contains_key(handle)
                {
                    fallback_near |= !player_convex::supported(collider);
                    if let Some(brush) = cache.brush(*handle, collider) {
                        merge(result, sweep.against(brush, 1), entity_of(collider), false);
                    }
                }
                true
            });
        fallback_near
    }

    /// Non-convex colliders through Parry.
    fn merge_fallback_colliders(&self, result: &mut HullTrace, sweep: &Sweep, query: Query<'_>) {
        let fallback = |handle: ColliderHandle, collider: &Collider| {
            self.accepts_for_npc(handle, collider, query)
                && !self.world_brush_contents.contains_key(&handle)
                && !player_convex::supported(collider)
        };
        let filter = QueryFilter::default()
            .groups(QUERY_GROUPS)
            .exclude_sensors()
            .predicate(&fallback);
        let Sweep {
            start, end, hull, ..
        } = *sweep;
        let center = (hull.maxs + hull.mins) * 0.5;
        let half = (hull.maxs - hull.mins) * 0.5;
        let position = pose(start + center, Quat::IDENTITY);
        let end_position = pose(end + center, Quat::IDENTITY);
        let stationary = (end - start).length_squared() < 0.000001;
        // Stand probes have zero-height contact hulls. Only the Parry fallback
        // needs a tiny thickness; brush/convex planes keep the exact bounds.
        let shape = Cuboid::new(vector(half.max(Vec3::splat(0.001))));
        let overlap_shape = Cuboid::new(vector(
            (half - Vec3::splat(0.015625)).max(Vec3::splat(0.001)),
        ));
        self.query.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &position,
            &overlap_shape,
            filter,
            |handle| {
                let collider = &self.colliders[handle];
                let all_solid = stationary
                    || (collider.shape().is_convex()
                        && rapier3d::parry::query::intersection_test(
                            collider.position(),
                            collider.shape(),
                            &end_position,
                            &overlap_shape,
                        )
                        .unwrap_or(false));
                let hit = Trace {
                    fraction: if all_solid { 0. } else { 1. },
                    normal: Vec3::ZERO,
                    start_solid: true,
                    all_solid,
                };
                merge(result, hit, entity_of(collider), false);
                true
            },
        );
        if stationary || result.trace.all_solid {
            return;
        }
        let options = ShapeCastOptions {
            max_time_of_impact: 1.,
            target_distance: 0.03125 * SCALE,
            stop_at_penetration: false,
            compute_impact_geometry_on_penetration: true,
        };
        if let Some((handle, hit)) = self.query.cast_shape(
            &self.bodies,
            &self.colliders,
            &position,
            &vector(end - start),
            &shape,
            options,
            filter,
        ) {
            let fraction = hit.time_of_impact.clamp(0., 1.);
            if fraction < result.trace.fraction {
                result.trace.fraction = fraction;
                result.trace.normal = Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z);
                result.entity = entity_of(&self.colliders[handle]);
                result.transient = false;
            }
        }
    }
}

impl NpcCollisionWorld for Physics {
    fn npc_trace_hull(&self, start: Vec3, end: Vec3, hull: Hull, query: Query<'_>) -> HullTrace {
        if !start.is_finite()
            || !end.is_finite()
            || !hull.valid()
            || !(start + hull.mins).is_finite()
            || !(end + hull.maxs).is_finite()
            || query
                .transients
                .iter()
                .any(|actor| !actor.feet.is_finite() || !actor.hull.valid())
        {
            return HullTrace::invalid();
        }
        let sweep = Sweep::new(start, end, hull);
        let mut result = HullTrace {
            trace: trace_brushes(
                &self.player_world_brushes,
                start,
                end,
                hull.mins,
                hull.maxs,
                query.contents_mask,
            ),
            ..HullTrace::clear()
        };
        merge_actors(&mut result, &sweep, query);
        if self.merge_convex_colliders(&mut result, &sweep, query) {
            self.merge_fallback_colliders(&mut result, &sweep, query);
        }
        if result.trace.all_solid {
            result.trace.fraction = 0.;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{npc_config, npc_floor_world};
    use super::*;

    #[test]
    fn ground_move_from_a_valid_start_matches_the_full_check() {
        use crate::npc_probe::{ground_move, ground_move_from_valid};
        let physics = Physics::new(&npc_floor_world());
        let config = npc_config();
        let end = Vec3::X * 40.;
        let full = ground_move(&physics, Vec3::ZERO, end, config, Query::default());
        let valid = ground_move_from_valid(&physics, Vec3::ZERO, end, config, Query::default());
        assert!(full.completed && valid.completed);
        assert_eq!(full.end, valid.end);
    }
    #[test]
    fn distant_actors_do_not_affect_a_hull_trace_and_near_ones_block_it() {
        let physics = Physics::new(&npc_floor_world());
        let hull = npc_config().hull;
        let actor = |x: f32| ActorHull {
            entity: Some(7),
            feet: Vec3::new(x, 0., 1.),
            hull,
        };
        let (start, end) = (Vec3::new(0., 0., 1.), Vec3::new(40., 0., 1.));
        let trace_with = |actors: &[ActorHull]| {
            physics.npc_trace_hull(
                start,
                end,
                hull,
                Query {
                    transients: actors,
                    ..Default::default()
                },
            )
        };
        assert!(trace_with(&[actor(500.)]).clear_path());
        let blocked = trace_with(&[actor(30.)]);
        assert!(blocked.trace.fraction < 1. && blocked.transient);
        assert_eq!(blocked.entity, Some(7));
    }
}
