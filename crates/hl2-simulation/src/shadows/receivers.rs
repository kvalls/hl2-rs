//! The world triangles that receive shadows, on a horizontal grid, and the decal geometry one
//! shadow lays on them. The engine's shadow clipper (not in the Source SDK; observed in the retail
//! engine): a surface receives when it faces the light (`n . d <= -0.01`) and the shadow origin is
//! not behind it; its polygon is projected into the shadow's texture space, kept only when a
//! vertex lies short of the cast end, clipped to `0 <= u, v <= 1` and `w >= 0` (nothing is
//! clipped past the end: the fade carries on there), then to each clip plane (`n . p < dist`);
//! every vertex gets the fade byte of its depth ([`Frame::fade`]).
//!
//! Receivers are the world's brush faces and displacements, tested against their face plane (the
//! vertex normal), and static props (baked into the world's surfaces, told apart by their bone
//! weights), tested per triangle; only NPC and player shadows reach static props, and only at
//! render-to-texture detail (Source SDK `clientshadowmgr.cpp` `AddShadowToReceiver`).

use super::frame::Frame;
use glam::{Vec2, Vec3};
use modkit_core::Surface;
use std::collections::HashMap;

/// Grid cell edge, units: about the size of a shadow, so a query reads a few cells.
const CELL: f32 = 64.;
/// A receiver must face the light by more than this (`n . d` at most minus this).
const FACING: f32 = 0.01;

/// What a receiver triangle belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A brush face or displacement.
    World,
    /// A static prop baked into the world.
    StaticProp,
}

#[derive(Clone, Debug)]
struct Triangle {
    points: [Vec3; 3],
    /// The receiving plane: the face plane for the world, the triangle's own for props.
    normal: Vec3,
    dist: f32,
    kind: Kind,
}

/// The world's receiver triangles, bucketed by their bounds on an X/Y grid.
#[derive(Default)]
pub struct Receivers {
    triangles: Vec<Triangle>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

/// A shadow's decal geometry: world positions, 0..1 coordinates in its shadow texture, fade bytes
/// and a triangle list. Kept between rebuilds so its buffers are reused.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decal {
    pub positions: Vec<Vec3>,
    /// The receiving plane's normal.
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub fades: Vec<u8>,
    pub indices: Vec<u32>,
}

/// How one caster's shadow is laid: whether static props receive it, its falloff bias and how far
/// its geometry sits off the receivers (a drawing offset against depth fighting).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cast {
    pub static_props: bool,
    pub bias: u8,
    pub lift: f32,
}

impl Decal {
    pub fn clear(&mut self) {
        self.positions.clear();
        self.normals.clear();
        self.uvs.clear();
        self.fades.clear();
        self.indices.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Reusable buffers for [`Receivers::decal`].
#[derive(Default)]
pub struct Scratch {
    ids: Vec<u32>,
    polygon: Polygon,
}

/// A polygon being clipped and the buffer each clip writes into.
#[derive(Default)]
struct Polygon {
    points: Vec<Point>,
    clipped: Vec<Point>,
}

/// A polygon vertex: world position and shadow texture coordinates (u, v, depth).
#[derive(Clone, Copy, Debug)]
struct Point {
    p: Vec3,
    uvw: Vec3,
}

fn cell(v: f32) -> i32 {
    (v / CELL).floor() as i32
}

impl Receivers {
    /// The receivers among `surfaces` (the world's batches with its static props, then its
    /// displacements) whose material `receives` (not translucent, alpha-tested or water).
    pub fn new<'a>(
        surfaces: impl IntoIterator<Item = &'a Surface>,
        receives: impl Fn(&str) -> bool,
    ) -> Self {
        let mut out = Self::default();
        for surface in surfaces {
            if surface.background || !receives(&surface.material) {
                continue;
            }
            for t in surface.indices.as_chunks::<3>().0 {
                let [Some(a), Some(b), Some(c)] = t.map(|i| surface.vertices.get(i as usize))
                else {
                    continue;
                };
                let v = [a, b, c];
                let points = [v[0].position, v[1].position, v[2].position];
                let shading = (v[0].normal + v[1].normal + v[2].normal).normalize_or_zero();
                let kind = if v[0].skin.is_some() {
                    Kind::StaticProp
                } else {
                    Kind::World
                };
                let normal = match kind {
                    Kind::World => shading,
                    Kind::StaticProp => {
                        let n = (points[1] - points[0])
                            .cross(points[2] - points[0])
                            .normalize_or_zero();
                        if n.dot(shading) < 0. {
                            -n
                        } else {
                            n
                        }
                    }
                };
                if normal == Vec3::ZERO {
                    continue;
                }
                out.add(Triangle {
                    points,
                    normal,
                    dist: normal.dot(points[0]),
                    kind,
                });
            }
        }
        out
    }

    fn add(&mut self, triangle: Triangle) {
        let id = self.triangles.len() as u32;
        let [a, b, c] = triangle.points;
        let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
        for x in cell(lo.x)..=cell(hi.x) {
            for y in cell(lo.y)..=cell(hi.y) {
                self.grid.entry((x, y)).or_default().push(id);
            }
        }
        self.triangles.push(triangle);
    }

    /// Receiver triangles.
    pub fn len(&self) -> usize {
        self.triangles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// Writes into `out` the decal `frame` lays on the receivers as `cast` asks.
    pub fn decal(&self, frame: &Frame, cast: Cast, scratch: &mut Scratch, out: &mut Decal) {
        out.clear();
        let (lo, hi) = reach(frame);
        scratch.ids.clear();
        for x in cell(lo.x)..=cell(hi.x) {
            for y in cell(lo.y)..=cell(hi.y) {
                if let Some(ids) = self.grid.get(&(x, y)) {
                    scratch.ids.extend_from_slice(ids);
                }
            }
        }
        scratch.ids.sort_unstable();
        scratch.ids.dedup();
        for &id in &scratch.ids {
            let t = &self.triangles[id as usize];
            if t.kind == Kind::StaticProp && !cast.static_props {
                continue;
            }
            let [a, b, c] = t.points;
            if a.min(b).min(c).cmpgt(hi).any() || a.max(b).max(c).cmplt(lo).any() {
                continue;
            }
            if t.normal.dot(frame.d) > -FACING || t.dist > t.normal.dot(frame.origin) {
                continue;
            }
            clip_triangle(frame, t.points, &mut scratch.polygon);
            emit(frame, (cast, t.normal), &scratch.polygon.points, out);
        }
    }
}

/// The world box a shadow can reach: its texture rectangle swept from the origin to where the
/// fade reaches 255.
fn reach(frame: &Frame) -> (Vec3, Vec3) {
    let (hx, hy) = (frame.x * frame.size.x * 0.5, frame.y * frame.size.y * 0.5);
    let end = frame.falloff_start
        + (frame.max_height - frame.falloff_start).max(0.) * 255. / super::MAX_FALLOFF;
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for depth in [0., end.max(frame.max_height)] {
        for (sx, sy) in [(-1., -1.), (-1., 1.), (1., -1.), (1., 1.)] {
            let p = frame.origin + hx * sx + hy * sy + frame.d * depth;
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    (lo, hi)
}

/// Projects a triangle into shadow texture space and clips it; the result is left in
/// `polygon.points` (empty when nothing remains).
fn clip_triangle(frame: &Frame, points: [Vec3; 3], polygon: &mut Polygon) {
    polygon.points.clear();
    let mut short = false;
    for p in points {
        let uvw = frame.texture(p);
        short |= uvw.z < frame.max_height;
        polygon.points.push(Point { p, uvw });
    }
    if !short {
        polygon.points.clear();
        return;
    }
    let sides: [&dyn Fn(&Point) -> f32; 5] = [
        &|v| -v.uvw.x,
        &|v| v.uvw.x - 1.,
        &|v| -v.uvw.y,
        &|v| v.uvw.y - 1.,
        &|v| -v.uvw.z,
    ];
    for side in sides {
        clip(&mut polygon.points, &mut polygon.clipped, side);
    }
    for &(n, d) in &frame.clip {
        clip(&mut polygon.points, &mut polygon.clipped, |v| {
            n.dot(v.p) - d
        });
    }
}

/// Sutherland-Hodgman: keeps the part of `polygon` where `side` is at most 0.
fn clip(polygon: &mut Vec<Point>, out: &mut Vec<Point>, side: impl Fn(&Point) -> f32) {
    out.clear();
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        let (sa, sb) = (side(&a), side(&b));
        if sa <= 0. {
            out.push(a);
        }
        if (sa < 0. && sb > 0.) || (sa > 0. && sb < 0.) {
            let t = sa / (sa - sb);
            out.push(Point {
                p: a.p.lerp(b.p, t),
                uvw: a.uvw.lerp(b.uvw, t),
            });
        }
    }
    if out.len() < 3 {
        out.clear();
    }
    std::mem::swap(polygon, out);
}

/// Appends a clipped polygon on a receiver with `normal` as a triangle fan.
fn emit(frame: &Frame, (cast, normal): (Cast, Vec3), polygon: &[Point], out: &mut Decal) {
    if polygon.len() < 3 {
        return;
    }
    let base = out.positions.len() as u32;
    for v in polygon {
        out.positions.push(v.p + normal * cast.lift);
        out.normals.push(normal);
        out.uvs.push(v.uvw.truncate());
        out.fades.push(frame.fade(v.uvw.z, cast.bias));
    }
    for i in 1..polygon.len() as u32 - 1 {
        out.indices.extend([base, base + i, base + i + 1]);
    }
}
