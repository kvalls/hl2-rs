//! Projected, triangle-clipped impact marks. Positions stay in the receiver's local space.
use crate::Surface;
use glam::{Vec2, Vec3};
#[derive(Clone, Copy, Debug)]
pub struct DecalVertex {
    pub position: Vec3,
    pub uv: Vec2,
}
const EMPTY_POLYGON: [DecalVertex; 16] = [DecalVertex {
    position: Vec3::ZERO,
    uv: Vec2::ZERO,
}; 16];
/// A square mark of half-width `radius` around `center`, facing `normal`, turned by `angle`.
struct Frame {
    center: Vec3,
    normal: Vec3,
    u: Vec3,
    v: Vec3,
    radius: f32,
}
impl Frame {
    fn new(center: Vec3, normal: Vec3, radius: f32, angle: f32) -> Option<Self> {
        if !radius.is_finite() || radius <= 0. || normal.length_squared() < 0.5 {
            return None;
        }
        let normal = normal.normalize();
        let axis = if normal.z.abs() < 0.9 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let right = normal.cross(axis).normalize();
        let up = normal.cross(right);
        let u = right * angle.cos() + up * angle.sin();
        Some(Self {
            center,
            normal,
            u,
            v: normal.cross(u),
            radius,
        })
    }
    /// The part of triangle `p` inside the mark (mark-plane uv, not yet normalized): writes up
    /// to 16 vertices into `out` and returns the count (`0` for a triangle turned away from the
    /// mark or more than half a unit off its plane).
    fn clip(&self, p: [Vec3; 3], out: &mut [DecalVertex; 16]) -> usize {
        let tri_normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        if tri_normal.dot(self.normal).abs() < 0.85
            || p.iter()
                .any(|p| (*p - self.center).dot(self.normal).abs() > 0.5)
        {
            return 0;
        }
        let mut len = 3;
        for (i, position) in p.into_iter().enumerate() {
            let offset = position - self.center;
            out[i] = DecalVertex {
                position,
                uv: Vec2::new(offset.dot(self.u), offset.dot(self.v)),
            };
        }
        for (axis, sign) in [(0, 1.), (0, -1.), (1, 1.), (1, -1.)] {
            if len == 0 {
                break;
            }
            let mut next = EMPTY_POLYGON;
            let mut next_len = 0;
            for i in 0..len {
                let a = out[i];
                let b = out[(i + 1) % len];
                let da = a.uv[axis] * sign - self.radius;
                let db = b.uv[axis] * sign - self.radius;
                if da <= 0. && next_len < next.len() {
                    next[next_len] = a;
                    next_len += 1;
                }
                if (da <= 0.) != (db <= 0.) && next_len < next.len() {
                    let t = da / (da - db);
                    next[next_len] = DecalVertex {
                        position: a.position.lerp(b.position, t),
                        uv: a.uv.lerp(b.uv, t),
                    };
                    next_len += 1;
                }
            }
            *out = next;
            len = next_len;
        }
        len
    }
}
/// Slack around `reach` for rounding: a box a little too large only adds triangles to test.
const REACH_MARGIN: f32 = 1.;
/// Every triangle of `surfaces` as (surface, triangle) indices, in order.
pub fn all_triangles(surfaces: &[Surface]) -> impl Iterator<Item = (usize, usize)> + '_ {
    (surfaces.iter().enumerate())
        .flat_map(|(s, surface)| (0..surface.indices.len() / 3).map(move |t| (s, t)))
}
fn triangle(surface: &Surface, t: usize) -> [Vec3; 3] {
    [0, 1, 2].map(|k| surface.vertices[surface.indices[3 * t + k] as usize].position)
}
/// A world box around everything a mark of half-width `radius` at `center`, facing `normal`,
/// reaches at any turn: the in-plane square (within `radius` x sqrt 2 of the center) and half a
/// unit off its plane. Only a triangle whose box meets it can take part of the mark.
pub fn reach(center: Vec3, normal: Vec3, radius: f32) -> (Vec3, Vec3) {
    let n = normal.normalize_or_zero().to_array();
    let half =
        n.map(|c| radius * std::f32::consts::SQRT_2 * (1. - c * c).max(0.).sqrt() + 0.5 * c.abs());
    let half = Vec3::from_array(half) + Vec3::splat(REACH_MARGIN);
    (center - half, center + half)
}
pub fn project(
    surfaces: &[Surface],
    center: Vec3,
    normal: Vec3,
    radius: f32,
    angle: f32,
) -> Vec<DecalVertex> {
    let triangles = all_triangles(surfaces);
    project_triangles(surfaces, triangles, center, normal, radius, angle)
}
/// `project` on the given (surface, triangle) indices, in their order: the same mark when they
/// include every triangle `reach` meets.
pub fn project_triangles(
    surfaces: &[Surface],
    triangles: impl IntoIterator<Item = (usize, usize)>,
    center: Vec3,
    normal: Vec3,
    radius: f32,
    angle: f32,
) -> Vec<DecalVertex> {
    let Some(frame) = Frame::new(center, normal, radius, angle) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut polygon = EMPTY_POLYGON;
    for (s, t) in triangles {
        let surface = &surfaces[s];
        if surface.background {
            continue;
        }
        let len = frame.clip(triangle(surface, t), &mut polygon);
        for i in 1..len.saturating_sub(1) {
            for mut vertex in [polygon[0], polygon[i], polygon[i + 1]] {
                vertex.position += frame.normal * 0.06;
                vertex.uv = vertex.uv / (radius * 2.) + Vec2::splat(0.5);
                result.push(vertex);
            }
        }
    }
    result
}
/// Whether `project` would put any of a mark on `surface`, without building it: stops at the
/// first triangle the mark reaches.
pub fn touches(surface: &Surface, center: Vec3, normal: Vec3, radius: f32) -> bool {
    let triangles = 0..surface.indices.len() / 3;
    touches_triangles(surface, triangles, center, normal, radius)
}
/// `touches` on the given triangles of `surface`.
pub fn touches_triangles(
    surface: &Surface,
    triangles: impl IntoIterator<Item = usize>,
    center: Vec3,
    normal: Vec3,
    radius: f32,
) -> bool {
    !surface.background
        && Frame::new(center, normal, radius, 0.).is_some_and(|frame| {
            let mut polygon = EMPTY_POLYGON;
            (triangles.into_iter()).any(|t| frame.clip(triangle(surface, t), &mut polygon) >= 3)
        })
}
/// Static map decal (infodecal) on lightmapped world faces: centered on the plane point
/// nearest `center`, `size` world units wide/high, oriented by each face's texture axes
/// (decals follow the receiving surface's texture u/v directions), clipped to the decal
/// rectangle and lit by the receiver's lightmap. Faces farther than `reach` from `center`
/// are skipped (the engine's +-5 unit placement trace).
pub fn static_decal(
    surfaces: &[Surface],
    center: Vec3,
    size: Vec2,
    reach: f32,
    material: &str,
) -> Vec<Surface> {
    use crate::Vertex;
    if !center.is_finite() || size.min_element() <= 0. || !size.is_finite() {
        return Vec::new();
    }
    let half = size * 0.5;
    let mut out: Vec<Surface> = Vec::new();
    for surface in surfaces
        .iter()
        .filter(|s| s.lightmap.is_some() && !s.background)
    {
        let mut decal = Surface {
            background: false,
            material: material.into(),
            vertices: Vec::new(),
            indices: Vec::new(),
            lightmap: surface.lightmap,
            flex_source: None,
        };
        for tri in surface.indices.as_chunks::<3>().0 {
            let v = tri.map(|i| &surface.vertices[i as usize]);
            let (p0, p1, p2) = (v[0].position, v[1].position, v[2].position);
            let normal = (p1 - p0).cross(p2 - p0).normalize_or_zero();
            if normal == Vec3::ZERO || normal.dot(center - p0).abs() > reach {
                continue;
            }
            // World directions of increasing texture u and v on this triangle.
            let (e1, e2) = (p1 - p0, p2 - p0);
            let (d1, d2) = (v[1].uv - v[0].uv, v[2].uv - v[0].uv);
            let det = d1.x * d2.y - d1.y * d2.x;
            if det.abs() < 1e-9 {
                continue;
            }
            let u_axis = ((e1 * d2.y - e2 * d1.y) / det).normalize_or_zero();
            let v_axis = ((e2 * d1.x - e1 * d2.x) / det).normalize_or_zero();
            if u_axis == Vec3::ZERO || v_axis == Vec3::ZERO {
                continue;
            }
            let origin = center - normal * normal.dot(center - p0);
            let local = |p: Vec3| Vec2::new((p - origin).dot(u_axis), (p - origin).dot(v_axis));
            // Clip in decal space, carrying barycentric weights for lightmap/color.
            let mut polygon: Vec<(Vec2, Vec3)> = vec![
                (local(p0), Vec3::X),
                (local(p1), Vec3::Y),
                (local(p2), Vec3::Z),
            ];
            for (axis, sign) in [(0, 1.), (0, -1.), (1, 1.), (1, -1.)] {
                let input = std::mem::take(&mut polygon);
                for i in 0..input.len() {
                    let (a, b) = (input[i], input[(i + 1) % input.len()]);
                    let da = a.0[axis] * sign - half[axis];
                    let db = b.0[axis] * sign - half[axis];
                    if da <= 0. {
                        polygon.push(a);
                    }
                    if (da <= 0.) != (db <= 0.) {
                        let t = da / (da - db);
                        polygon.push((a.0.lerp(b.0, t), a.1.lerp(b.1, t)));
                    }
                }
                if polygon.is_empty() {
                    break;
                }
            }
            if polygon.len() < 3 {
                continue;
            }
            let base = decal.vertices.len() as u32;
            for (uv, w) in &polygon {
                let position = p0 * w.x + p1 * w.y + p2 * w.z;
                let color = std::array::from_fn(|c| {
                    (f32::from(v[0].color[c]) * w.x
                        + f32::from(v[1].color[c]) * w.y
                        + f32::from(v[2].color[c]) * w.z)
                        .round() as u8
                });
                decal.vertices.push(Vertex {
                    position: position + normal * 0.1,
                    uv: *uv / size + Vec2::splat(0.5),
                    color,
                    light_uv: v[0].light_uv * w.x + v[1].light_uv * w.y + v[2].light_uv * w.z,
                    skin: None,
                    normal,
                });
            }
            for i in 1..polygon.len() as u32 - 1 {
                decal.indices.extend([base, base + i, base + i + 1]);
            }
        }
        if !decal.indices.is_empty() {
            out.push(decal);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn impact_is_clipped_at_surface_edge_and_never_spills_to_parallel_wall() {
        let surface = Surface {
            flex_source: None,
            background: false,
            material: String::new(),
            lightmap: None,
            vertices: [Vec3::ZERO, Vec3::X * 10., Vec3::Y * 10.]
                .into_iter()
                .map(|position| crate::Vertex {
                    normal: Default::default(),
                    position,
                    uv: Vec2::ZERO,
                    color: [255; 4],
                    light_uv: Vec2::ZERO,
                    skin: None,
                })
                .collect(),
            indices: vec![0, 1, 2],
        };
        let vertices = project(
            std::slice::from_ref(&surface),
            Vec3::new(1., 1., 0.),
            Vec3::Z,
            3.,
            0.,
        );
        assert!(!vertices.is_empty());
        assert!(vertices.iter().all(|v| v.position.x >= 0.
            && v.position.y >= 0.
            && v.position.x + v.position.y <= 10.01
            && v.uv.min_element() >= -0.001
            && v.uv.max_element() <= 1.001));
        assert!(touches(&surface, Vec3::new(1., 1., 0.), Vec3::Z, 0.15));
        assert!(!touches(&surface, Vec3::new(1., 1., 3.), Vec3::Z, 0.15));
        assert!(!touches(&surface, Vec3::new(20., 20., 0.), Vec3::Z, 0.15));
        assert!(!touches(&surface, Vec3::new(1., 1., 0.), Vec3::X, 0.15));
        assert!(project(&[surface], Vec3::new(1., 1., 3.), Vec3::Z, 3., 0.).is_empty());
    }
    #[test]
    fn static_decal_follows_texture_axes_clips_to_its_rectangle_and_keeps_lightmap() {
        // Wall in the XZ plane facing -Y; texture u runs +X, v runs -Z (down).
        let corner = |x: f32, z: f32| crate::Vertex {
            normal: Default::default(),
            position: Vec3::new(x, 0., z),
            uv: Vec2::new(x / 100., -z / 100.),
            color: [255; 4],
            light_uv: Vec2::new(x / 100., z / 100.),
            skin: None,
        };
        let wall = Surface {
            flex_source: None,
            background: false,
            material: "wall".into(),
            lightmap: Some(3),
            vertices: vec![
                corner(0., 0.),
                corner(100., 100.),
                corner(100., 0.),
                corner(0., 100.),
            ],
            indices: vec![0, 1, 2, 0, 3, 1],
        };
        let decals = static_decal(
            std::slice::from_ref(&wall),
            Vec3::new(50., 2., 50.),
            Vec2::new(20., 10.),
            5.,
            "decal",
        );
        assert_eq!(decals.len(), 1);
        let d = &decals[0];
        assert_eq!((d.lightmap, d.material.as_str()), (Some(3), "decal"));
        for v in &d.vertices {
            assert!((v.position.x - 50.).abs() <= 10.01 && (v.position.z - 50.).abs() <= 5.01);
            assert!(v.uv.min_element() >= -1e-4 && v.uv.max_element() <= 1. + 1e-4);
            // Top of the decal (high z) maps to texture v = 0.
            if v.position.z > 54.9 {
                assert!(v.uv.y < 1e-3, "{v:?}");
            }
            assert!((v.light_uv.x - v.position.x / 100.).abs() < 1e-4);
        }
        // Out of reach: nothing.
        assert!(
            static_decal(&[wall], Vec3::new(50., 20., 50.), Vec2::splat(10.), 5., "d").is_empty()
        );
    }
}
