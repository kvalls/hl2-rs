//! Where on a surface list an impact lands: its receiver, its decal and the baked light under it.
//! The static world's surfaces (map faces and static props, batched by material across the map)
//! hold every triangle of the map, so for them a tree of triangle boxes picks the few a mark can
//! reach; the triangles are still tested exactly and in the same order, so the results do not
//! change.
use glam::Vec3 as GVec3;
use modkit_core::{decals, Surface, World};
use rapier3d::parry::{
    bounding_volume::Aabb,
    partitioning::{Qbvh, QbvhDataGenerator},
    query::visitors::BoundingVolumeIntersectionsVisitor,
};
use rapier3d::prelude::Point;

/// A lightmap sample may fall outside its triangle: barycentric weights down to -0.01 reach up
/// to 2% of the triangle's extent past its box (plus rounding).
const SAMPLE_SLACK: f32 = 0.02;
const SAMPLE_MARGIN: f32 = 0.05;
/// How far off a lightmapped face a sample point may lie.
const SAMPLE_DEPTH: f32 = 1.;

/// The triangles of a surface list by box: one leaf per triangle of a drawn surface.
pub(crate) struct SurfaceIndex {
    tree: Qbvh<u32>,
    build_time: std::time::Duration,
    /// (surface, triangle) of each leaf, in surface then triangle order.
    triangles: Vec<(u32, u32)>,
    /// Drawn triangle counts in the immutable map geometry, in surface order.
    surface_triangles: Vec<usize>,
}

fn drawn_triangles(surface: &Surface) -> usize {
    if surface.background {
        0
    } else {
        surface.indices.len() / 3
    }
}

fn aabb((min, max): (GVec3, GVec3)) -> Aabb {
    Aabb::new(
        Point::new(min.x, min.y, min.z),
        Point::new(max.x, max.y, max.z),
    )
}

/// Each leaf's triangle box, widened by the lightmap sample slack.
struct Boxes<'a>(&'a [Surface], &'a [(u32, u32)]);
impl QbvhDataGenerator<u32> for Boxes<'_> {
    fn size_hint(&self) -> usize {
        self.1.len()
    }
    fn for_each(&mut self, mut f: impl FnMut(u32, Aabb)) {
        for (leaf, &(s, t)) in self.1.iter().enumerate() {
            let surface = &self.0[s as usize];
            let corners = [0, 1, 2].map(|k| {
                let index = surface.indices[3 * t as usize + k];
                surface.vertices[index as usize].position
            });
            let min = corners[0].min(corners[1]).min(corners[2]);
            let max = corners[0].max(corners[1]).max(corners[2]);
            let slack = (max - min) * SAMPLE_SLACK + GVec3::splat(SAMPLE_MARGIN);
            f(leaf as u32, aabb((min - slack, max + slack)));
        }
    }
}

impl SurfaceIndex {
    pub fn new(surfaces: &[Surface]) -> Self {
        let started = std::time::Instant::now();
        let triangles: Vec<(u32, u32)> = decals::all_triangles(surfaces)
            .filter(|&(s, _)| !surfaces[s].background)
            .map(|(s, t)| (s as u32, t as u32))
            .collect();
        let mut tree = Qbvh::new();
        tree.clear_and_rebuild(Boxes(surfaces, &triangles), 0.);
        Self {
            tree,
            triangles,
            surface_triangles: surfaces.iter().map(drawn_triangles).collect(),
            build_time: started.elapsed(),
        }
    }
    /// Clones of this map share its static geometry. A changed batch layout needs a full scan.
    pub fn covers(&self, surfaces: &[Surface]) -> bool {
        self.surface_triangles.len() == surfaces.len()
            && self
                .surface_triangles
                .iter()
                .copied()
                .eq(surfaces.iter().map(drawn_triangles))
    }
    /// The drawn triangles whose boxes meet `bounds`, as (surface, triangle) in order.
    pub fn near(&self, bounds: (GVec3, GVec3)) -> Vec<(usize, usize)> {
        let mut leaves = Vec::new();
        let mut visit = |leaf: &u32| {
            leaves.push(*leaf);
            true
        };
        let bounds = aabb(bounds);
        let mut visitor = BoundingVolumeIntersectionsVisitor::new(&bounds, &mut visit);
        self.tree.traverse_depth_first(&mut visitor);
        leaves.sort_unstable();
        (leaves.into_iter())
            .map(|leaf| self.triangles[leaf as usize])
            .map(|(s, t)| (s as usize, t as usize))
            .collect()
    }
}

#[derive(serde::Serialize)]
struct IndexReport {
    triangles: usize,
    payload_bytes: usize,
    build_ms: f64,
}

impl super::Impacts {
    /// Map-load diagnostic. Payload counts used triangle records, tree nodes and proxies.
    pub fn decal_index_report(&self) -> Option<impl serde::Serialize> {
        let index = self.world_index.as_ref()?;
        let bytes = std::mem::size_of_val(index.triangles.as_slice())
            + std::mem::size_of_val(index.surface_triangles.as_slice())
            + std::mem::size_of_val(index.tree.raw_nodes())
            + std::mem::size_of_val(index.tree.raw_proxies());
        Some(IndexReport {
            triangles: index.triangles.len(),
            payload_bytes: bytes,
            build_ms: index.build_time.as_secs_f64() * 1000.,
        })
    }
}

/// The triangles of surface `s` in a sorted (surface, triangle) list.
fn of_surface(near: &[(usize, usize)], s: usize) -> impl Iterator<Item = usize> + '_ {
    let start = near.partition_point(|&(n, _)| n < s);
    let end = near.partition_point(|&(n, _)| n <= s);
    near[start..end].iter().map(|&(_, t)| t)
}

/// `touches` on surface `s`, on its triangles in `near` when the index found them.
fn touches(
    surfaces: &[Surface],
    s: usize,
    near: Option<&[(usize, usize)]>,
    center: GVec3,
    normal: GVec3,
) -> bool {
    match near {
        Some(near) => {
            decals::touches_triangles(&surfaces[s], of_surface(near, s), center, normal, 0.15)
        }
        None => decals::touches(&surfaces[s], center, normal, 0.15),
    }
}

/// The first surface a small mark at the hit reaches; else, within `gap` along the normal, the
/// surface nearest the hit (a static prop's drawn mesh off its collision hull).
pub(super) fn receiver<'s>(
    surfaces: &'s [Surface],
    index: Option<&SurfaceIndex>,
    center: GVec3,
    normal: GVec3,
    gap: f32,
) -> Option<(&'s Surface, GVec3)> {
    // Every triangle a mark within `gap` of the hit can reach.
    let (min, max) = decals::reach(center, normal, 0.15);
    let offset = normal.abs() * gap;
    let near = index.map(|index| index.near((min - offset, max + offset)));
    let near = near.as_deref();
    if let Some(s) = (0..surfaces.len()).find(|&s| touches(surfaces, s, near, center, normal)) {
        return Some((&surfaces[s], center));
    }
    if gap <= 0. {
        return None;
    }
    let mut nearest = None;
    let mut distance = gap;
    for (s, surface) in surfaces.iter().enumerate().filter(|(_, s)| !s.background) {
        // A surface with no triangle near the hit takes no mark, whatever its planes.
        if near.is_some_and(|near| of_surface(near, s).next().is_none()) {
            continue;
        }
        for triangle in surface.indices.as_chunks::<3>().0 {
            let p = triangle.map(|i| surface.vertices[i as usize].position);
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            let facing = normal.dot(n);
            if facing.abs() < 0.85 {
                continue;
            }
            let offset = (p[0] - center).dot(n) / facing;
            if offset.abs() >= distance {
                continue;
            }
            let projected = center + normal * offset;
            if touches(surfaces, s, near, projected, normal) {
                nearest = Some((surface, projected));
                distance = offset.abs();
            }
        }
    }
    nearest
}

/// `decals::project` for a located mark, on the triangles near it when the index applies.
pub(super) fn project(
    surfaces: &[Surface],
    index: Option<&SurfaceIndex>,
    (center, normal): (GVec3, GVec3),
    radius: f32,
    angle: f32,
) -> Vec<decals::DecalVertex> {
    match index {
        Some(index) => {
            let near = index.near(decals::reach(center, normal, radius));
            decals::project_triangles(surfaces, near, center, normal, radius, angle)
        }
        None => decals::project(surfaces, center, normal, radius, angle),
    }
}

/// Baked light (linear) at `point` on the lightmapped face of `surfaces` it lies on.
pub(super) fn lightmap_light(
    world: &World,
    surfaces: &[Surface],
    index: Option<&SurfaceIndex>,
    point: GVec3,
    normal: GVec3,
) -> Option<GVec3> {
    let normal = normal.normalize_or_zero();
    let near = |index: &SurfaceIndex| {
        let reach = GVec3::splat(SAMPLE_DEPTH + SAMPLE_MARGIN);
        index.near((point - reach, point + reach))
    };
    match index {
        Some(index) => sample(world, surfaces, near(index), point, normal),
        None => sample(
            world,
            surfaces,
            decals::all_triangles(surfaces),
            point,
            normal,
        ),
    }
}

fn sample(
    world: &World,
    surfaces: &[Surface],
    triangles: impl IntoIterator<Item = (usize, usize)>,
    point: GVec3,
    normal: GVec3,
) -> Option<GVec3> {
    for (s, t) in triangles {
        let surface = &surfaces[s];
        let Some(lightmap) = surface.lightmap.and_then(|i| world.lightmaps.get(i)) else {
            continue;
        };
        if surface.background {
            continue;
        }
        let v = [0, 1, 2].map(|k| &surface.vertices[surface.indices[3 * t + k] as usize]);
        let (a, b, c) = (v[0].position, v[1].position, v[2].position);
        let n = (b - a).cross(c - a);
        if n.length_squared() < 1e-8
            || n.normalize().dot(normal).abs() < 0.85
            || (point - a).dot(n.normalize()).abs() > SAMPLE_DEPTH
        {
            continue;
        }
        // Barycentric coordinates of the point projected onto the triangle.
        let (e0, e1, e2) = (b - a, c - a, point - a);
        let (d00, d01, d11) = (e0.dot(e0), e0.dot(e1), e1.dot(e1));
        let (d20, d21) = (e2.dot(e0), e2.dot(e1));
        let denom = d00 * d11 - d01 * d01;
        if denom.abs() < 1e-8 {
            continue;
        }
        let w1 = (d11 * d20 - d01 * d21) / denom;
        let w2 = (d00 * d21 - d01 * d20) / denom;
        let w0 = 1. - w1 - w2;
        if w0 < -0.01 || w1 < -0.01 || w2 < -0.01 {
            continue;
        }
        let uv = v[0].light_uv * w0 + v[1].light_uv * w1 + v[2].light_uv * w2;
        let x = ((uv.x * f32::from(lightmap.width)) as usize).min(usize::from(lightmap.width) - 1);
        let y =
            ((uv.y * f32::from(lightmap.height)) as usize).min(usize::from(lightmap.height) - 1);
        let texel = (y * usize::from(lightmap.width) + x) * 4;
        let rgb = lightmap.rgba.get(texel..texel + 3)?;
        return Some(GVec3::new(half(rgb[0]), half(rgb[1]), half(rgb[2])));
    }
    None
}

/// IEEE half-float bits to f32.
fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1. } else { 1. };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => mantissa * 2f32.powi(-24),
        31 => f32::INFINITY,
        e => (1. + mantissa / 1024.) * 2f32.powi(e - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use modkit_core::{Lightmap, Vertex};

    fn quad(surface: &mut Surface, corners: [GVec3; 4]) {
        let base = surface.vertices.len() as u32;
        for (k, position) in corners.into_iter().enumerate() {
            let light_uv = Vec2::new((k % 2) as f32, (k / 2) as f32) * 0.5 + 0.25;
            let (uv, color, skin, normal) = (Vec2::ZERO, [255; 4], None, GVec3::ZERO);
            (surface.vertices).push(Vertex {
                position,
                uv,
                color,
                light_uv,
                skin,
                normal,
            });
        }
        (surface.indices).extend([base, base + 1, base + 2, base + 2, base + 1, base + 3]);
    }

    /// Synthetic batches spanning the map like material batches: floor tiles of three materials,
    /// a wall in pieces, a huge plane 0.4 above the floor, a background quad and a sliver.
    fn surfaces() -> Vec<Surface> {
        let mut surfaces: Vec<Surface> = (0..4)
            .map(|m| Surface {
                background: m == 3,
                material: format!("m{m}"),
                vertices: Vec::new(),
                indices: Vec::new(),
                lightmap: Some(0),
                flex_source: None,
            })
            .collect();
        let p = GVec3::new;
        for i in 0..8 {
            for j in 0..8 {
                let (x, y) = (-64. + 16. * i as f32, -64. + 16. * j as f32);
                let corners = [
                    p(x, y, 0.),
                    p(x + 16., y, 0.),
                    p(x, y + 16., 0.),
                    p(x + 16., y + 16., 0.),
                ];
                quad(&mut surfaces[(i + j) % 3], corners);
            }
        }
        for k in 0..4 {
            let y = -64. + 32. * k as f32;
            quad(
                &mut surfaces[1],
                [
                    p(10., y, 0.),
                    p(10., y + 32., 0.),
                    p(10., y, 64.),
                    p(10., y + 32., 64.),
                ],
            );
        }
        let far = 1000.;
        quad(
            &mut surfaces[2],
            [
                p(-far, -far, 0.4),
                p(far, -far, 0.4),
                p(-far, far, 0.4),
                p(far, far, 0.4),
            ],
        );
        quad(
            &mut surfaces[3],
            [
                p(-8., -8., 0.1),
                p(8., -8., 0.1),
                p(-8., 8., 0.1),
                p(8., 8., 0.1),
            ],
        );
        quad(
            &mut surfaces[0],
            [
                p(0., 0., 0.2),
                p(1., 0., 0.2),
                p(2., 0., 0.2),
                p(3., 0., 0.2),
            ],
        );
        surfaces
    }

    #[test]
    fn the_index_finds_what_scanning_every_triangle_finds() {
        let surfaces = surfaces();
        let index = SurfaceIndex::new(&surfaces);
        assert!(index.covers(&surfaces));
        let rgba = (0..16).map(|i| 0x3c00 + i * 64).collect();
        let world = World {
            lightmaps: vec![Lightmap {
                width: 2,
                height: 2,
                rgba,
            }],
            ..Default::default()
        };
        let normals = [
            GVec3::Z,
            -GVec3::Z,
            GVec3::X,
            GVec3::new(1., 0., 1.).normalize(),
            GVec3::new(0.3, 0.2, 0.93).normalize(),
        ];
        let (mut received, mut marked, mut lit) = (0, 0, 0);
        for x in (0..20).map(|i| -70. + 7.3 * i as f32) {
            for y in (0..20).map(|i| -70. + 7.3 * i as f32) {
                for z in [0., 0.3, -0.4, 1., 5.] {
                    let center = GVec3::new(x, y, z);
                    for normal in normals {
                        for gap in [0., 1.5] {
                            let found = |index| {
                                receiver(&surfaces, index, center, normal, gap)
                                    .map(|(s, at)| (s as *const Surface, at))
                            };
                            assert_eq!(found(Some(&index)), found(None), "{center} {normal} {gap}");
                            received += usize::from(found(None).is_some());
                        }
                        for radius in [2., 6.] {
                            let mark = |index| {
                                project(&surfaces, index, (center, normal), radius, 0.7)
                                    .iter()
                                    .map(|v| (v.position, v.uv))
                                    .collect::<Vec<_>>()
                            };
                            assert_eq!(
                                mark(Some(&index)),
                                mark(None),
                                "{center} {normal} {radius}"
                            );
                            marked += usize::from(!mark(None).is_empty());
                        }
                        let light =
                            |index| lightmap_light(&world, &surfaces, index, center, normal);
                        assert_eq!(light(Some(&index)), light(None), "{center} {normal}");
                        lit += usize::from(light(None).is_some());
                    }
                }
            }
        }
        // Not vacuous: plenty of hits, marks and samples on each path.
        assert!(
            received > 1000 && marked > 1000 && lit > 1000,
            "{received} {marked} {lit}"
        );
    }

    #[test]
    fn the_index_skips_background_triangles_and_notices_a_changed_list() {
        let mut surfaces = surfaces();
        let index = SurfaceIndex::new(&surfaces);
        let everywhere = (GVec3::splat(-2000.), GVec3::splat(2000.));
        assert!(index
            .near(everywhere)
            .iter()
            .all(|&(s, _)| !surfaces[s].background));
        // Redistribute one triangle without changing the total number of indices.
        let start = surfaces[0].indices.len() - 3;
        let triangle = surfaces[0].indices.split_off(start);
        surfaces[1].indices.extend(triangle);
        assert!(!index.covers(&surfaces));
    }
}
