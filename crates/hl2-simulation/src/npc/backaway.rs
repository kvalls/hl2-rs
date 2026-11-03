//! CAI_TacticalServices::FindBackAwayPos: where an NPC backs away from a threat.
use super::{nearest_usable, Controller};
use crate::npc_probe::{self, GroundConfig, NpcCollisionWorld, Query};
use glam::Vec3;

/// FindBackAwayPos's first vector goal: 10 feet, and no less.
const BACKAWAY_DIST: f32 = 10. * 12.;

impl Controller {
    /// CAI_TacticalServices::FindBackAwayPos for `entity` standing at `feet`: a ground move
    /// straight away from `threat` (CAI_Navigator::FindVectorGoal: 120 units, deflected along
    /// what blocks it, 120 covered in all), else FindBackAwayNode, else a shorter vector goal
    /// (hull width x 4, at least x 2). None: no place to back away.
    pub fn find_backaway<W: NpcCollisionWorld>(
        &self,
        world: &W,
        entity: usize,
        feet: Vec3,
        threat: Vec3,
        query: Query<'_>,
    ) -> Option<Vec3> {
        let ground = self.actors.get(&entity)?.ground;
        // NAV_GROUND moves stay on the floor: the away direction is horizontal.
        let away = Vec3::new(feet.x - threat.x, feet.y - threat.y, 0.).normalize_or_zero();
        if away == Vec3::ZERO {
            return None;
        }
        let width = ground.hull.maxs.x - ground.hull.mins.x;
        vector_goal(
            world,
            feet,
            away,
            (BACKAWAY_DIST, BACKAWAY_DIST),
            ground,
            query,
        )
        .or_else(|| self.backaway_node(feet, threat))
        .or_else(|| vector_goal(world, feet, away, (width * 4., width * 2.), ground, query))
    }
    /// CAI_TacticalServices::FindBackAwayNode: the first usable link of the usable node
    /// nearest `origin` to a node farther from `threat` that does not lie toward it.
    fn backaway_node(&self, origin: Vec3, threat: Vec3) -> Option<Vec3> {
        let (positions, adjacent) = self.usable_graph()?;
        let me = nearest_usable(&positions, origin)?;
        let current = threat.distance(origin);
        let to_threat = (threat - origin).normalize_or_zero();
        adjacent[me]
            .iter()
            .filter_map(|&node| positions[node])
            .find(|&p| {
                threat.distance(p) > current && (p - origin).normalize_or_zero().dot(to_threat) < 0.
            })
    }
}

/// CAI_Navigator::FindVectorGoal with deflection: a ground move `target` units along `dir`;
/// when something blocks it, the rest continues along the blocking plane
/// (CalculateDeflection). The end of the last move, or None when less than `min` was
/// covered.
fn vector_goal<W: NpcCollisionWorld>(
    world: &W,
    start: Vec3,
    dir: Vec3,
    (target, min): (f32, f32),
    ground: GroundConfig,
    query: Query<'_>,
) -> Option<Vec3> {
    let flat = |v: Vec3| Vec3::new(v.x, v.y, 0.);
    let first = npc_probe::ground_move(world, start, start + dir * target, ground, query);
    if first.completed {
        return Some(first.end);
    }
    let mut covered = flat(first.end - start).length();
    let mut end = first.end;
    let normal = flat(first.normal);
    if normal != Vec3::ZERO {
        let deflect = normal.cross(dir.cross(normal)).normalize_or_zero();
        let rest = first.end + deflect * (target - covered);
        let second = npc_probe::ground_move(world, first.end, rest, ground, query);
        covered += flat(second.end - first.end).length();
        end = second.end;
    }
    (covered >= min + 0.01).then_some(end)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{cube, make_controller, world};
    use crate::physics::Physics;
    use glam::Vec3;

    #[test]
    fn backing_away_moves_ten_feet_from_the_threat_slides_along_walls_and_can_fail() {
        let excluded = [7];
        let query = crate::npc_probe::Query {
            excluded_entities: &excluded,
            ..Default::default()
        };
        let controller = make_controller(None);
        // Open floor: straight away, 120 units.
        let physics = Physics::new(&world(vec![]));
        let p = controller
            .find_backaway(&physics, 7, Vec3::ZERO, Vec3::X * 100., query)
            .unwrap();
        assert!((p - Vec3::new(-120., 0., 0.)).length() < 1., "{p}");
        // A wall 40 units behind, across the away direction at an angle: the move slides
        // along it and still covers 120 units.
        let wall = |x: f32| cube(Vec3::new(x - 8., -500., 0.), Vec3::new(x, 500., 200.));
        let physics = Physics::new(&world(vec![wall(-40.)]));
        let threat = Vec3::new(100., -60., 0.);
        let p = controller
            .find_backaway(&physics, 7, Vec3::ZERO, threat, query)
            .unwrap();
        assert!(p.x > -40. && p.y > 30., "{p}");
        // Head-on into a wall right behind: no deflection, so the shorter goal fails too.
        let physics = Physics::new(&world(vec![wall(-20.)]));
        assert!(controller
            .find_backaway(&physics, 7, Vec3::ZERO, Vec3::X * 100., query)
            .is_none());
    }
}
