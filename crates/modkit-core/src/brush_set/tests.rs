use super::{Bounds, BrushSet, CANDIDATES};
use crate::{trace_brushes, Brush, Plane, Trace};
use glam::Vec3;

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
    fn point(&mut self, extent: f32) -> Vec3 {
        Vec3::new(self.next(), self.next(), self.next()) * extent
    }
}

fn box_planes(min: Vec3, max: Vec3) -> Vec<Plane> {
    let mut planes = Vec::new();
    for (axis, unit) in Vec3::AXES.into_iter().enumerate() {
        planes.push(Plane {
            normal: unit,
            distance: max[axis],
        });
        planes.push(Plane {
            normal: -unit,
            distance: -min[axis],
        });
    }
    planes
}

/// Boxes, boxes cut by a slanted plane, and wedges that lack an axis-aligned plane.
fn field(count: usize, extent: f32, seed: u64) -> Vec<Brush> {
    let mut values = Sequence(seed);
    (0..count)
        .map(|index| {
            let min = values.point(extent);
            let max = min + values.point(96.) + 4.;
            let mut planes = box_planes(min, max);
            let slant = Vec3::new(values.next() - 0.5, values.next() - 0.5, 1.).normalize();
            match index % 7 {
                // A slanted cut through the box.
                0 | 1 => planes.push(Plane {
                    normal: slant,
                    distance: slant.dot((min + max) * 0.5),
                }),
                // The top replaced by a slanted plane: no +Z axial plane.
                2 => {
                    planes.remove(4);
                    planes.push(Plane {
                        normal: slant,
                        distance: slant.dot(max),
                    });
                }
                _ => {}
            }
            Brush {
                planes,
                contents: [1, 1, 1, 0x10000, 0x2, 0x4000b][index % 6],
            }
        })
        .collect()
}

fn same(a: Trace, b: Trace) -> bool {
    a.fraction.to_bits() == b.fraction.to_bits()
        && a.normal.to_array().map(f32::to_bits) == b.normal.to_array().map(f32::to_bits)
        && a.start_solid == b.start_solid
        && a.all_solid == b.all_solid
}

const HULLS: [(Vec3, Vec3); 4] = [
    (Vec3::ZERO, Vec3::ZERO),
    (Vec3::new(-16., -16., 0.), Vec3::new(16., 16., 72.)),
    (Vec3::new(-8., -8., -8.), Vec3::new(8., 8., 8.)),
    (Vec3::new(-40., -40., 0.), Vec3::new(40., 40., 100.)),
];

#[test]
fn indexed_trace_equals_the_full_scan() {
    for (count, extent, seed) in [(600, 1500., 1), (40, 120., 2), (3, 50., 3), (0, 1., 4)] {
        let brushes = field(count, extent, seed);
        let set = BrushSet::new(brushes.clone());
        let mut values = Sequence(seed + 100);
        let (mut hits, mut solid, mut indexed) = (0, 0, 0);
        for case in 0..6000 {
            let (mins, maxs) = HULLS[case % HULLS.len()];
            let start = values.point(extent + 200.) - 100.;
            let reach = [0., 0.01, 2., 30., 400., 4000.][case % 6];
            let end = start + (values.point(2.) - 1.) * reach;
            let mask = [1, 0x1400b, 0x2, 0xffff_ffff][case % 4];
            let expected = trace_brushes(&brushes, start, end, mins, maxs, mask);
            let found = set.trace(start, end, mins, maxs, mask);
            assert!(same(expected, found), "{case}: {expected:?} {found:?}");
            hits += usize::from(expected.fraction < 1.);
            solid += usize::from(expected.start_solid);
            let sweep = Bounds {
                min: start.min(end) + mins,
                max: start.max(end) + maxs,
            };
            indexed += usize::from(set.gather(&sweep, &mut [0; CANDIDATES]).is_some());
        }
        if count == 600 {
            // The cases exercise hits, solid starts, the tree and the full-scan fallback.
            assert!(hits > 500 && solid > 200, "{hits} {solid}");
            assert!(indexed > 3000 && indexed < 6000, "{indexed}");
        }
    }
}

#[test]
fn touching_and_boundary_sweeps_equal_the_full_scan() {
    let brushes = field(200, 600., 9);
    let set = BrushSet::new(brushes.clone());
    let (mins, maxs) = HULLS[1];
    // Starts and ends placed on and just off every brush face along each axis.
    for brush in &brushes {
        let bounds = Bounds::of(brush);
        if !bounds.finite() {
            continue;
        }
        for axis in 0..3 {
            for offset in [
                -0.0625, -0.03125, -0.015625, 0., 0.015625, 0.03125, 0.0625, 0.126,
            ] {
                let mut start = (bounds.min + bounds.max) * 0.5;
                start[axis] = bounds.max[axis] - mins[axis] + offset;
                for travel in [-1., -0.03125, 0., 0.03125, 1.] {
                    let mut end = start;
                    end[axis] += travel;
                    end[(axis + 1) % 3] += 3.;
                    let expected = trace_brushes(&brushes, start, end, mins, maxs, 1);
                    let found = set.trace(start, end, mins, maxs, 1);
                    assert!(same(expected, found), "{expected:?} {found:?}");
                }
            }
        }
    }
}

#[test]
fn tree_narrows_a_short_sweep_and_non_finite_input_is_scanned() {
    let brushes = field(2000, 4000., 5);
    let set = BrushSet::new(brushes.clone());
    assert!(!set.unbounded.is_empty());
    let sweep = Bounds {
        min: Vec3::splat(2000.),
        max: Vec3::splat(2040.),
    };
    let count = set.gather(&sweep, &mut [0; CANDIDATES]);
    assert!(count.is_some_and(|count| count < 60), "{count:?}");
    let (mins, maxs) = HULLS[1];
    let start = Vec3::new(f32::NAN, 0., 0.);
    let expected = trace_brushes(&brushes, start, Vec3::ZERO, mins, maxs, 1);
    assert!(same(expected, set.trace(start, Vec3::ZERO, mins, maxs, 1)));
}

#[test]
fn unsupported_sweeps_use_the_original_scan() {
    let set = BrushSet::new(field(200, 400., 9));
    for value in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1e20,
        -1e20,
        65536.25,
    ] {
        for parameter in 0..4 {
            let mut vectors = [Vec3::splat(200.), Vec3::splat(210.), HULLS[1].0, HULLS[1].1];
            vectors[parameter].x = value;
            let [start, end, mins, maxs] = vectors;
            assert_eq!(set.candidate_count(start, end, mins, maxs), None);
            assert!(same(
                trace_brushes(set.brushes(), start, end, mins, maxs, 1),
                set.trace(start, end, mins, maxs, 1)
            ));
        }
    }
    let (start, end, mins, maxs) = (Vec3::ZERO, Vec3::ONE, Vec3::ONE, Vec3::ZERO);
    assert_eq!(set.candidate_count(start, end, mins, maxs), None);
    assert!(same(
        trace_brushes(set.brushes(), start, end, mins, maxs, 1),
        set.trace(start, end, mins, maxs, 1)
    ));
}

#[test]
fn original_order_keeps_tie_normals_and_the_normal_before_all_solid() {
    let box_brush = |min, max| Brush {
        planes: box_planes(min, max),
        contents: 1,
    };
    let x = box_brush(Vec3::splat(-10.), Vec3::new(10., 30., 10.));
    let y = box_brush(Vec3::splat(-10.), Vec3::new(30., 10., 10.));
    let containing = box_brush(Vec3::splat(-30.), Vec3::splat(30.));
    let start = Vec3::new(20., 20., 0.);
    for (front, normal, solid) in [
        (vec![x.clone(), y.clone()], Vec3::X, false),
        (vec![y, x.clone()], Vec3::Y, false),
        (vec![x.clone(), containing.clone()], Vec3::X, true),
        (vec![containing, x], Vec3::ZERO, true),
    ] {
        let mut brushes = front;
        brushes.extend(field(200, 2000., 7).into_iter().map(|mut b| {
            b.contents = 2;
            b
        }));
        let set = BrushSet::new(brushes);
        let got = set.trace(start, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 1);
        assert_eq!(got.normal, normal);
        assert_eq!(got.all_solid, solid);
        assert!(same(
            got,
            trace_brushes(set.brushes(), start, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 1)
        ));
    }
}

#[test]
fn coordinate_limit_and_malformed_brushes_remain_exact() {
    for origin in [-65500., 65500.] {
        let brush = Brush {
            planes: box_planes(Vec3::splat(origin - 8.), Vec3::splat(origin + 8.)),
            contents: 1,
        };
        let set = BrushSet::new(vec![brush]);
        for offset in [-0.25, -0.03125, 0., 0.03125, 0.25] {
            let start = Vec3::splat(origin + 24. + offset);
            let end = Vec3::splat(origin + 25.);
            let (mins, maxs) = (-Vec3::splat(16.), Vec3::splat(16.));
            assert!(same(
                set.trace(start, end, mins, maxs, 1),
                trace_brushes(set.brushes(), start, end, mins, maxs, 1)
            ));
        }
    }
    for distance in [f32::NAN, f32::INFINITY, 1e20] {
        let mut planes = box_planes(Vec3::splat(-1.), Vec3::ONE);
        planes[0].distance = distance;
        let set = BrushSet::new(vec![Brush {
            planes,
            contents: 1,
        }]);
        assert_eq!(set.unbounded_count(), 1);
        let start = Vec3::splat(100.);
        assert!(same(
            set.trace(start, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 1),
            trace_brushes(set.brushes(), start, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 1)
        ));
    }
}
