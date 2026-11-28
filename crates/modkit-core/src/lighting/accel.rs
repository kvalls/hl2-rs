//! Precomputed bit-exact spatial/range filters for static world lights and leaf brushes.
use super::{EmitType, WorldLight, DWL_FLAGS_INAMBIENTCUBE, WORLD_LIGHT_MIN};
use crate::Brush;
use glam::Vec3;

const AABB_PAD: f32 = 0.05;

#[derive(Clone, Debug)]
pub(super) struct IndexedLight {
    pub(super) index: usize,
    pub(super) origin: Vec3,
    pub(super) max_dist_sq: f32,
    pub(super) is_sky: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct BrushBounds {
    pub(super) mins: Vec3,
    pub(super) maxs: Vec3,
    pub(super) axial_only: bool,
}

/// Pre-filtered light ranges and per-brush bounding slabs preserving `LightingData::state_at`
/// evaluation order and results bit-for-bit.
#[derive(Clone, Debug)]
pub struct LightingIndex {
    /// Non-SkyAmbient, non-DWL_FLAGS_INAMBIENTCUBE lights in exact ascending index order.
    pub(super) active_lights: Vec<IndexedLight>,
    /// All non-SkyAmbient lights in exact ascending index order (used when `leaf_at` is `None`).
    pub(super) all_lights: Vec<IndexedLight>,
    /// Per-brush bounds aligned 1:1 with `LightingData::brushes`.
    pub(super) brush_bounds: Vec<BrushBounds>,
}

impl LightingIndex {
    /// Builds conservative light distance bounds and brush AABBs from map lighting data.
    pub fn build(lights: &[WorldLight], brushes: &[Brush]) -> Self {
        let mut active_lights = Vec::with_capacity(lights.len());
        let mut all_lights = Vec::with_capacity(lights.len());
        for (index, light) in lights.iter().enumerate() {
            let Some(entry) = index_light(index, light) else {
                continue;
            };
            if light.flags & DWL_FLAGS_INAMBIENTCUBE == 0 {
                active_lights.push(entry.clone());
            }
            all_lights.push(entry);
        }
        let brush_bounds = brushes.iter().map(brush_bounds_of).collect();
        Self {
            active_lights,
            all_lights,
            brush_bounds,
        }
    }
}

fn index_light(index: usize, light: &WorldLight) -> Option<IndexedLight> {
    let max_dist_sq = match light.kind {
        EmitType::SkyAmbient => return None,
        EmitType::Sky => {
            return Some(IndexedLight {
                index,
                origin: light.origin,
                max_dist_sq: f32::INFINITY,
                is_sky: true,
            });
        }
        EmitType::Surface => {
            if light.radius != 0. {
                let r = f64::from(light.radius).abs() * 1.0001 + 0.001;
                (r * r) as f32
            } else {
                f32::INFINITY
            }
        }
        EmitType::Point | EmitType::Spot => point_or_spot_max_dist_sq(light)?,
        EmitType::Quake => {
            if light.attenuation[1] <= 0. || light.intensity.max_element() <= 0. {
                return None;
            }
            let d = f64::from(light.attenuation[1]) + 0.01;
            (d * d) as f32
        }
    };
    Some(IndexedLight {
        index,
        origin: light.origin,
        max_dist_sq,
        is_sky: false,
    })
}

fn point_or_spot_max_dist_sq(light: &WorldLight) -> Option<f32> {
    if light.radius < 0. {
        return None;
    }
    let max_i = light.intensity.max_element();
    if max_i <= 0. {
        return None;
    }
    let radius_sq = if light.radius > 0. {
        let r = f64::from(light.radius) * 1.0001 + 0.001;
        (r * r) as f32
    } else {
        f32::INFINITY
    };
    let [c, l, q] = light.attenuation;
    if !max_i.is_finite() || c < 0. || l < 0. || q < 0. {
        return Some(radius_sq);
    }
    let limit = f64::from(max_i) / f64::from(WORLD_LIGHT_MIN);
    let rem = limit - f64::from(c);
    if rem < -1e-6 {
        return None;
    }
    let rem = rem.max(0.0);
    let d = if q > 0. {
        let lf = f64::from(l);
        let qf = f64::from(q);
        (-lf + (lf * lf + 4.0 * qf * rem).sqrt()) / (2.0 * qf)
    } else if l > 0. {
        rem / f64::from(l)
    } else {
        f64::INFINITY
    };
    let d = d * 1.001 + 0.01;
    let d_sq = if d.is_finite() {
        (d * d) as f32
    } else {
        f32::INFINITY
    };
    Some(d_sq.min(radius_sq))
}

fn brush_bounds_of(brush: &Brush) -> BrushBounds {
    let mut mins = Vec3::splat(f32::NEG_INFINITY);
    let mut maxs = Vec3::splat(f32::INFINITY);
    let mut axial_only = !brush.planes.is_empty();
    for plane in &brush.planes {
        let n = plane.normal;
        if n == Vec3::X {
            maxs.x = maxs.x.min(plane.distance);
        } else if n == Vec3::NEG_X {
            mins.x = mins.x.max(-plane.distance);
        } else if n == Vec3::Y {
            maxs.y = maxs.y.min(plane.distance);
        } else if n == Vec3::NEG_Y {
            mins.y = mins.y.max(-plane.distance);
        } else if n == Vec3::Z {
            maxs.z = maxs.z.min(plane.distance);
        } else if n == Vec3::NEG_Z {
            mins.z = mins.z.max(-plane.distance);
        } else {
            axial_only = false;
        }
    }
    axial_only &= mins.is_finite()
        && maxs.is_finite()
        && mins.x <= maxs.x
        && mins.y <= maxs.y
        && mins.z <= maxs.z;
    BrushBounds {
        mins,
        maxs,
        axial_only,
    }
}

#[inline]
pub(super) fn segment_overlaps_bounds(seg_min: Vec3, seg_max: Vec3, b: &BrushBounds) -> bool {
    !(seg_max.x < b.mins.x - AABB_PAD
        || seg_min.x > b.maxs.x + AABB_PAD
        || seg_max.y < b.mins.y - AABB_PAD
        || seg_min.y > b.maxs.y + AABB_PAD
        || seg_max.z < b.mins.z - AABB_PAD
        || seg_min.z > b.maxs.z + AABB_PAD)
}

#[inline]
pub(super) fn entry_aabb(mins: Vec3, maxs: Vec3, start: Vec3, end: Vec3) -> Option<f32> {
    let start_outside = start.x > maxs.x
        || start.x < mins.x
        || start.y > maxs.y
        || start.y < mins.y
        || start.z > maxs.z
        || start.z < mins.z;
    if !start_outside {
        let end_outside = end.x > maxs.x
            || end.x < mins.x
            || end.y > maxs.y
            || end.y < mins.y
            || end.z > maxs.z
            || end.z < mins.z;
        return (!end_outside).then_some(0.);
    }
    let delta = end - start;
    let (mut enter, mut leave) = (0f32, 1f32);
    for i in 0..3 {
        let from_hi = start[i] - maxs[i];
        let from_lo = mins[i] - start[i];
        let d = delta[i];
        if d == 0. {
            if from_hi > 0. || from_lo > 0. {
                return None;
            }
            continue;
        }
        let t_hi = -from_hi / d;
        let t_lo = -from_lo / -d;
        if d < 0. {
            enter = enter.max(t_hi);
            leave = leave.min(t_lo);
        } else {
            leave = leave.min(t_hi);
            enter = enter.max(t_lo);
        }
        if enter > leave {
            return None;
        }
    }
    Some(enter)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{point_light, split_world};
    use super::*;
    use crate::Plane;

    fn make_brush(planes: Vec<(Vec3, f32)>) -> Brush {
        Brush {
            planes: planes
                .into_iter()
                .map(|(normal, distance)| Plane { normal, distance })
                .collect(),
            contents: 1,
        }
    }

    fn box_brush(min: Vec3, max: Vec3) -> Brush {
        make_brush(vec![
            (Vec3::X, max.x),
            (Vec3::NEG_X, -min.x),
            (Vec3::Y, max.y),
            (Vec3::NEG_Y, -min.y),
            (Vec3::Z, max.z),
            (Vec3::NEG_Z, -min.z),
        ])
    }

    #[test]
    fn indexed_state_at_matches_unindexed_across_all_light_kinds() {
        let mut unindexed = split_world();
        unindexed.lights = vec![
            WorldLight {
                kind: EmitType::SkyAmbient,
                ..point_light(Vec3::new(10., 0., 50.), 500.)
            },
            WorldLight {
                flags: DWL_FLAGS_INAMBIENTCUBE,
                ..point_light(Vec3::new(12., 5., 10.), 2000.)
            },
            WorldLight {
                radius: -5.,
                ..point_light(Vec3::new(10., 0., 10.), 500.)
            },
            point_light(Vec3::new(10., 0., 100.), 0.0),
            point_light(Vec3::new(10., 0., 2000.), 5.0),
            WorldLight {
                kind: EmitType::Spot,
                normal: Vec3::NEG_Z,
                stopdot: 0.9,
                stopdot2: 0.5,
                exponent: 2.0,
                radius: 400.,
                attenuation: [1., 0.01, 0.001],
                ..point_light(Vec3::new(25., 10., 80.), 800.)
            },
            WorldLight {
                kind: EmitType::Surface,
                normal: Vec3::NEG_X,
                radius: 250.,
                ..point_light(Vec3::new(60., 0., 0.), 300.)
            },
            WorldLight {
                kind: EmitType::Quake,
                attenuation: [0., 120., 0.],
                ..point_light(Vec3::new(40., 20., 0.), 250.)
            },
            point_light(Vec3::new(20., -10., 15.), 1200.),
        ];
        unindexed.brushes.push(box_brush(
            Vec3::new(15., -10., -10.),
            Vec3::new(25., 10., 10.),
        ));
        unindexed.leaves[0].brushes.push(0);

        let mut indexed = unindexed.clone();
        indexed.build_index();

        let points = [
            Vec3::new(5., 0., 0.),
            Vec3::new(35., 12., 5.),
            Vec3::new(500., 500., 500.),
            Vec3::new(-50., 0., 0.), // leaf_at == solid leaf with brushes or outside range
            Vec3::new(10., 0., 10.),
        ];
        for point in points {
            assert_eq!(
                indexed.state_at(point),
                unindexed.state_at(point),
                "{point}"
            );
        }
    }

    #[test]
    fn aabb_and_single_pass_brush_entry_match_two_pass_semantics() {
        let axial = box_brush(Vec3::new(10., -5., -5.), Vec3::new(20., 5., 5.));
        let bounds = brush_bounds_of(&axial);
        assert!(bounds.axial_only);
        let beveled = make_brush(vec![
            (Vec3::X, 20.),
            (Vec3::NEG_X, -10.),
            (Vec3::Y, 5.),
            (Vec3::NEG_Y, 5.),
            (Vec3::new(0., 0., 1.).normalize(), 5.),
            (Vec3::new(1., 1., 1.).normalize(), 12.),
        ]);
        let beveled_bounds = brush_bounds_of(&beveled);
        assert!(!beveled_bounds.axial_only);

        let segments = [
            (Vec3::ZERO, Vec3::new(40., 0., 0.)),
            (Vec3::ZERO, Vec3::new(5., 0., 0.)),
            (Vec3::new(0., 10., 0.), Vec3::new(40., 10., 0.)),
            (Vec3::new(15., 0., 0.), Vec3::new(40., 0., 0.)),
            (Vec3::new(12., 0., 0.), Vec3::new(18., 0., 0.)),
            (Vec3::new(10., 0., 0.), Vec3::new(20., 0., 0.)),
            (Vec3::new(30., 0., 0.), Vec3::ZERO),
            (Vec3::new(15., -20., 0.), Vec3::new(15., 20., 0.)),
            (Vec3::new(15., 0., -20.), Vec3::new(15., 0., 20.)),
        ];
        for (start, end) in segments {
            let two_pass_axial = two_pass_entry(&axial, start, end);
            assert_eq!(
                super::super::brushes::entry(&axial, start, end),
                two_pass_axial,
                "single-pass axial {start:?} -> {end:?}"
            );
            assert_eq!(
                entry_aabb(bounds.mins, bounds.maxs, start, end),
                two_pass_axial,
                "aabb {start:?} -> {end:?}"
            );
            let two_pass_beveled = two_pass_entry(&beveled, start, end);
            assert_eq!(
                super::super::brushes::entry(&beveled, start, end),
                two_pass_beveled,
                "beveled {start:?} -> {end:?}"
            );
            if two_pass_beveled.is_some() {
                assert!(segment_overlaps_bounds(
                    start.min(end),
                    start.max(end),
                    &beveled_bounds
                ));
            }
        }
    }

    fn two_pass_entry(brush: &Brush, start: Vec3, end: Vec3) -> Option<f32> {
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
}
