//! Brushes in the light visibility trace. The engine's world trace clips the brushes each leaf
//! lists (CM leafbrushes), so a detail brush, which never makes its leaf solid, still blocks
//! light: a facade built from detail brushes keeps the sun off the plaza props behind it.
use super::{LightLeaf, LightingData};
use crate::Brush;
use glam::Vec3;

/// Fraction along `start`->`end` where the segment enters `brush`. CM_ClipBoxToBrush: a segment
/// that starts inside only marks the trace startsolid and is not stopped, unless it never gets
/// out (allsolid, fraction 0): a prop whose lighting origin sits in a curb or planter still sees.
pub(super) fn entry(brush: &Brush, start: Vec3, end: Vec3) -> Option<f32> {
    let outside = |p: Vec3| {
        brush
            .planes
            .iter()
            .any(|plane| plane.normal.dot(p) > plane.distance)
    };
    if !outside(start) {
        return (!outside(end)).then_some(0.);
    }
    let delta = end - start;
    let (mut enter, mut leave) = (0f32, 1f32);
    for plane in &brush.planes {
        let from = plane.normal.dot(start) - plane.distance;
        let along = plane.normal.dot(delta);
        if along == 0. {
            if from > 0. {
                return None;
            }
            continue;
        }
        let t = -from / along;
        if along < 0. {
            enter = enter.max(t);
        } else {
            leave = leave.min(t);
        }
        if enter > leave {
            return None;
        }
    }
    Some(enter)
}

impl LightingData {
    /// The first of `leaf`'s brushes that `segment` enters by `f2`, its stretch through the
    /// leaf being `f1..f2` of the whole segment.
    pub(super) fn leaf_brush_hit(
        &self,
        leaf: &LightLeaf,
        (start, end): (Vec3, Vec3),
        (f1, f2): (f32, f32),
    ) -> Option<f32> {
        leaf.brushes
            .iter()
            .filter_map(|&id| self.brushes.get(id as usize))
            .filter_map(|brush| entry(brush, start, end))
            .filter(|&f| f <= f2)
            .min_by(f32::total_cmp)
            .map(|f| f.max(f1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Plane;

    fn cube(min: Vec3, max: Vec3) -> Brush {
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        let mut planes = Vec::new();
        for (i, axis) in axes.into_iter().enumerate() {
            planes.push(Plane {
                normal: axis,
                distance: max[i],
            });
            planes.push(Plane {
                normal: -axis,
                distance: -min[i],
            });
        }
        Brush {
            planes,
            contents: 0x800_0001,
        }
    }

    #[test]
    fn a_segment_enters_a_brush_where_it_crosses_the_nearest_face() {
        let brush = cube(Vec3::new(10., -5., -5.), Vec3::new(20., 5., 5.));
        let f = entry(&brush, Vec3::ZERO, Vec3::new(40., 0., 0.)).expect("hit");
        assert!((f - 0.25).abs() < 1e-6, "{f}");
        assert_eq!(
            entry(&brush, Vec3::ZERO, Vec3::new(5., 0., 0.)),
            None,
            "short"
        );
        assert_eq!(
            entry(&brush, Vec3::new(0., 10., 0.), Vec3::new(40., 10., 0.)),
            None,
            "beside"
        );
        // Starting inside and getting out is startsolid, not a hit; never getting out is.
        assert_eq!(
            entry(&brush, Vec3::new(15., 0., 0.), Vec3::new(40., 0., 0.)),
            None,
            "starts inside"
        );
        assert_eq!(
            entry(&brush, Vec3::new(12., 0., 0.), Vec3::new(18., 0., 0.)),
            Some(0.),
            "all inside"
        );
    }

    #[test]
    fn a_trace_starting_in_a_solid_leaf_is_stopped_only_by_a_brush_it_enters() {
        // split_world: leaf 0 open (x >= 0), leaf 1 solid (x < 0), made of one wall brush.
        let mut world = super::super::tests::split_world();
        world
            .brushes
            .push(cube(Vec3::new(-64., -64., -64.), Vec3::new(0., 64., 64.)));
        world.leaves[1].brushes.push(0);
        let light = super::super::tests::point_light(Vec3::new(50., 0., 0.), 100.);
        // A lighting origin 4 units inside the wall still sees the light in the room.
        let inside = Vec3::new(-4., 0., 0.);
        assert_eq!(world.trace(inside, light.origin), None);
        assert!(world.intensity_at(&light, inside).is_some());
        // From the room, the wall still blocks a light behind it.
        let behind = super::super::tests::point_light(Vec3::new(-30., 0., 0.), 100.);
        let (f, _) = world
            .trace(Vec3::new(20., 0., 0.), behind.origin)
            .expect("wall");
        assert!((f * 50. - 20.).abs() < 1e-3, "{f}");
    }

    #[test]
    fn a_detail_brush_in_an_open_leaf_blocks_the_light_behind_it() {
        let mut world = super::super::tests::split_world();
        let light = super::super::tests::point_light(Vec3::new(50., 0., 0.), 100.);
        let p = Vec3::new(5., 0., 0.);
        assert!(
            world.intensity_at(&light, p).is_some(),
            "open leaf, no brush"
        );
        // A detail brush never makes its leaf solid; the leaf only lists it.
        world
            .brushes
            .push(cube(Vec3::new(20., -10., -10.), Vec3::new(30., 10., 10.)));
        world.leaves[0].brushes.push(0);
        assert!(world.intensity_at(&light, p).is_none(), "20 units blocked");
        let (f, last) = world.trace(p, light.origin).expect("blocked");
        assert!(
            (f * 45. - 15.).abs() < 1e-3 && last == Some(0),
            "{f} {last:?}"
        );
        // A lamp inside its fixture brush: blocked for its last 6 units, within the 8 of slack.
        let lamp = super::super::tests::point_light(Vec3::new(34., 0., 0.), 100.);
        world.brushes[0] = cube(Vec3::new(28., -10., -10.), Vec3::new(40., 10., 10.));
        assert!(world.intensity_at(&lamp, p).is_some(), "6 units blocked");
        world.brushes[0] = cube(Vec3::new(20., -10., -10.), Vec3::new(40., 10., 10.));
        assert!(world.intensity_at(&lamp, p).is_none(), "14 units blocked");
    }
}
