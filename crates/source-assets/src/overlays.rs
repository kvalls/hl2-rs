//! BSP lump 45 info_overlay records (SDK bspfile.h doverlay_t, 352 bytes) and their fragments
//! on brush faces (retail COverlayMgr brush path; displacement faces are not handled yet).
use crate::{f32le, i16le, i32le, records, u16le, vec3};
use anyhow::{bail, Result};
use glam::{Vec2, Vec3};

pub const RECORD_SIZE: usize = 352;
const FACE_SLOTS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    pub id: i32,
    pub texinfo: i16,
    /// OVERLAY_RENDER_ORDER (top two bits of the face count field).
    pub render_order: u16,
    pub faces: Vec<i32>,
    pub u: [f32; 2],
    pub v: [f32; 2],
    /// Corner positions in the overlay plane (VBSP stores the U basis in their z).
    pub uv_points: [Vec2; 4],
    pub origin: Vec3,
    pub basis_normal: Vec3,
    /// SDK vbsp overlay.cpp: BasisU packed into vecUVPoints[0..3].z.
    pub basis_u: Vec3,
    /// vecUVPoints[3].z = 1 when the V axis is flipped.
    pub flip_v: bool,
}

pub fn read(lump: &[u8]) -> Result<Vec<Overlay>> {
    records(lump, RECORD_SIZE)?
        .map(|r| {
            let packed = u16le(r, 6)?;
            let count = usize::from(packed & 0x3fff);
            if count > FACE_SLOTS {
                bail!("overlay face count {count} exceeds {FACE_SLOTS}");
            }
            let faces = (0..count)
                .map(|i| i32le(r, 8 + 4 * i))
                .collect::<Result<Vec<_>>>()?;
            let at = 8 + 4 * FACE_SLOTS;
            let points = [
                vec3(r, at + 16)?,
                vec3(r, at + 28)?,
                vec3(r, at + 40)?,
                vec3(r, at + 52)?,
            ];
            Ok(Overlay {
                id: i32le(r, 0)?,
                texinfo: i16le(r, 4)?,
                render_order: packed >> 14,
                faces,
                u: [f32le(r, at)?, f32le(r, at + 4)?],
                v: [f32le(r, at + 8)?, f32le(r, at + 12)?],
                uv_points: points.map(|p| Vec2::new(p.x, p.y)),
                origin: vec3(r, at + 64)?,
                basis_normal: vec3(r, at + 76)?,
                basis_u: Vec3::new(points[0].z, points[1].z, points[2].z),
                flip_v: points[3].z == 1.,
            })
        })
        .collect()
}

/// One vertex of an overlay fragment: on the receiving face (pushed off it), with the
/// overlay's texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FragmentVertex {
    pub position: Vec3,
    pub uv: Vec2,
}

/// Distance the engine pushes fragments off their face (retail engine.dll 0x1031d910).
pub const SURFACE_OFFSET: f32 = 0.1;

impl Overlay {
    /// Overlay V axis: N x U, negated when VBSP recorded a flipped basis.
    pub fn basis_v(&self) -> Vec3 {
        let v = self.basis_normal.cross(self.basis_u).normalize_or_zero();
        if self.flip_v {
            -v
        } else {
            v
        }
    }

    /// Fragments of this overlay on one brush face (a convex polygon whose `normal` faces the
    /// rendered side), as convex polygons. Follows the retail COverlayMgr brush path: fan
    /// triangles of area <= 1 are skipped, each triangle is flattened onto the overlay plane
    /// and clips the overlay quad, every clipped vertex gets bilinear texcoords from its
    /// position inside the quad (SDK PointInQuadToBarycentric/TexCoordInQuadFromBarycentric),
    /// is projected back onto the face along the overlay normal and is pushed off the face by
    /// `SURFACE_OFFSET`.
    pub fn fragments(&self, face: &[Vec3], normal: Vec3) -> Vec<Vec<FragmentVertex>> {
        let n = self.basis_normal.normalize_or_zero();
        let u = self.basis_u.normalize_or_zero();
        let v = self.basis_v();
        let along = n.dot(normal);
        if n == Vec3::ZERO || u == Vec3::ZERO || v == Vec3::ZERO || along.abs() < 1e-4 {
            return Vec::new();
        }
        let local = |p: Vec3| {
            let d = p - self.origin;
            Vec2::new(d.dot(u), d.dot(v))
        };
        // Quad corners 0..3 with texcoords (U0,V0), (U0,V1), (U1,V1), (U1,V0).
        let quad = self.uv_points;
        let distance = normal.dot(face.first().copied().unwrap_or_default());
        let mut out = Vec::new();
        for i in 1..face.len().saturating_sub(1) {
            let tri = [face[0], face[i], face[i + 1]];
            if (tri[1] - tri[0]).cross(tri[2] - tri[0]).length() * 0.5 <= 1. {
                continue;
            }
            let flat = tri.map(local);
            let polygon = clip_to_triangle(&quad, flat);
            if polygon.len() < 3 {
                continue;
            }
            out.push(
                polygon
                    .into_iter()
                    .map(|p| {
                        let (s, t) = inverse_bilinear(quad[0], quad[3], quad[2], quad[1], p);
                        let on_plane = self.origin + u * p.x + v * p.y;
                        let position = on_plane - n * ((normal.dot(on_plane) - distance) / along)
                            + normal * SURFACE_OFFSET;
                        FragmentVertex {
                            position,
                            uv: Vec2::new(
                                self.u[0] + (self.u[1] - self.u[0]) * s,
                                self.v[0] + (self.v[1] - self.v[0]) * t,
                            ),
                        }
                    })
                    .collect(),
            );
        }
        out
    }
}

fn cross2(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Sutherland-Hodgman clip of a polygon by a triangle (either winding).
fn clip_to_triangle(polygon: &[Vec2], tri: [Vec2; 3]) -> Vec<Vec2> {
    let winding = cross2(tri[1] - tri[0], tri[2] - tri[0]).signum();
    let mut out = polygon.to_vec();
    for i in 0..3 {
        let (a, b) = (tri[i], tri[(i + 1) % 3]);
        let side = |p: Vec2| cross2(b - a, p - a) * winding;
        let input = std::mem::take(&mut out);
        for j in 0..input.len() {
            let (p, q) = (input[j], input[(j + 1) % input.len()]);
            let (dp, dq) = (side(p), side(q));
            if dp >= 0. {
                out.push(p);
            }
            if (dp >= 0.) != (dq >= 0.) {
                out.push(p.lerp(q, dp / (dp - dq)));
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
}

/// (s, t) with p = lerp(lerp(v1, v4, t), lerp(v2, v3, t), s): the SDK PointInQuadToBarycentric
/// convention (edge v1->v2 is s, edge v1->v4 is t), solved in closed form.
fn inverse_bilinear(v1: Vec2, v2: Vec2, v3: Vec2, v4: Vec2, p: Vec2) -> (f32, f32) {
    let e = v2 - v1;
    let f = v4 - v1;
    let g = v1 - v2 + v3 - v4;
    let h = p - v1;
    let k2 = cross2(g, f);
    let k1 = cross2(e, f) + cross2(h, g);
    let k0 = cross2(h, e);
    let t = if k2.abs() < 1e-6 * (k1.abs() + 1.) {
        if k1.abs() < 1e-12 {
            0.
        } else {
            -k0 / k1
        }
    } else {
        let disc = (k1 * k1 - 4. * k0 * k2).max(0.).sqrt();
        let a = (-k1 - disc) / (2. * k2);
        let b = (-k1 + disc) / (2. * k2);
        // The root inside (or nearest) the quad.
        if (a - 0.5).abs() <= (b - 0.5).abs() {
            a
        } else {
            b
        }
    };
    let denom = e + g * t;
    let s = if denom.x.abs() >= denom.y.abs() {
        if denom.x.abs() < 1e-12 {
            0.
        } else {
            (h.x - f.x * t) / denom.x
        }
    } else {
        (h.y - f.y * t) / denom.y
    };
    (s, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_doverlay_layout() {
        let mut r = vec![0u8; RECORD_SIZE];
        r[0..4].copy_from_slice(&7i32.to_le_bytes());
        r[4..6].copy_from_slice(&3i16.to_le_bytes());
        r[6..8].copy_from_slice(&(2u16 | (1 << 14)).to_le_bytes());
        r[8..12].copy_from_slice(&11i32.to_le_bytes());
        r[12..16].copy_from_slice(&12i32.to_le_bytes());
        let at = 8 + 4 * FACE_SLOTS;
        r[at + 4..at + 8].copy_from_slice(&1f32.to_le_bytes());
        r[at + 76 + 8..at + 88].copy_from_slice(&1f32.to_le_bytes());
        r[at + 16 + 8..at + 28].copy_from_slice(&1f32.to_le_bytes());
        r[at + 52 + 8..at + 64].copy_from_slice(&1f32.to_le_bytes());
        let o = &read(&r).unwrap()[0];
        assert_eq!((o.id, o.texinfo, o.render_order), (7, 3, 1));
        assert_eq!(o.faces, vec![11, 12]);
        assert_eq!(o.u, [0., 1.]);
        assert_eq!(o.basis_normal, Vec3::Z);
        assert_eq!(o.basis_u, Vec3::X);
        assert!(o.flip_v);
        r[6..8].copy_from_slice(&65u16.to_le_bytes());
        assert!(read(&r).is_err());
        assert!(read(&r[..100]).is_err());
    }
    fn flat_overlay(u: [f32; 2], v: [f32; 2], flip_v: bool) -> Overlay {
        Overlay {
            id: 0,
            texinfo: 0,
            render_order: 0,
            faces: vec![0],
            u,
            v,
            uv_points: [
                Vec2::new(-8., -4.),
                Vec2::new(-8., 4.),
                Vec2::new(8., 4.),
                Vec2::new(8., -4.),
            ],
            origin: Vec3::new(0., 0., 0.),
            basis_normal: Vec3::Z,
            basis_u: Vec3::X,
            flip_v,
        }
    }
    fn square(half: f32, z: f32) -> Vec<Vec3> {
        vec![
            Vec3::new(-half, -half, z),
            Vec3::new(half, -half, z),
            Vec3::new(half, half, z),
            Vec3::new(-half, half, z),
        ]
    }
    #[test]
    fn overlay_inside_one_face_keeps_its_quad_and_texcoords() {
        let o = flat_overlay([0., 1.], [0., 1.], false);
        assert_eq!(o.basis_v(), Vec3::Y);
        let frags = o.fragments(&square(100., 2.), Vec3::Z);
        let verts: Vec<_> = frags.iter().flatten().collect();
        assert!(!verts.is_empty());
        for v in &verts {
            // Projected onto the face at z = 2 and pushed 0.1 off it.
            assert!((v.position.z - 2.1).abs() < 1e-4);
            let expect = Vec2::new((v.position.x + 8.) / 16., (v.position.y + 4.) / 8.);
            assert!((v.uv - expect).length() < 1e-4, "{v:?}");
        }
        let corner = verts
            .iter()
            .find(|v| v.position.x < -7.99 && v.position.y < -3.99);
        assert_eq!(corner.unwrap().uv, Vec2::ZERO);
    }
    #[test]
    fn overlay_is_clipped_to_the_face() {
        let o = flat_overlay([0., 1.], [0., 1.], false);
        let face = vec![
            Vec3::new(0., -50., 0.),
            Vec3::new(50., -50., 0.),
            Vec3::new(50., 50., 0.),
            Vec3::new(0., 50., 0.),
        ];
        let verts: Vec<_> = o.fragments(&face, Vec3::Z).into_iter().flatten().collect();
        assert!(verts
            .iter()
            .all(|v| v.position.x >= -1e-4 && v.uv.x >= 0.5 - 1e-4));
        assert!(verts.iter().any(|v| (v.uv.x - 0.5).abs() < 1e-4));
        // Tiny or edge-on faces produce nothing.
        assert!(o.fragments(&square(0.5, 0.), Vec3::Z).is_empty());
        assert!(o.fragments(&square(100., 0.), Vec3::X).is_empty());
    }
    #[test]
    fn flipped_basis_negates_v_and_quads_map_bilinearly() {
        let o = flat_overlay([0., 1.], [0., 1.], true);
        assert_eq!(o.basis_v(), -Vec3::Y);
        // A trapezoid: corners map exactly to their texcoords.
        let (q0, q1, q2, q3) = (
            Vec2::new(0., 0.),
            Vec2::new(0., 2.),
            Vec2::new(4., 3.),
            Vec2::new(4., -1.),
        );
        for (p, st) in [
            (q0, (0., 0.)),
            (q1, (0., 1.)),
            (q2, (1., 1.)),
            (q3, (1., 0.)),
        ] {
            let (s, t) = inverse_bilinear(q0, q3, q2, q1, p);
            assert!(
                (s - st.0).abs() < 1e-4 && (t - st.1).abs() < 1e-4,
                "{p:?} {s} {t}"
            );
        }
        let (s, t) = inverse_bilinear(q0, q3, q2, q1, Vec2::new(2., 1.));
        assert!((s - 0.5).abs() < 1e-4 && (t - 0.5).abs() < 1e-4);
    }
    #[test]
    #[ignore = "requires owned HL2 installation"]
    fn owned_trainstation_overlays_reference_valid_faces() {
        let mut vfs = crate::vpk::Vfs::mount(&crate::install::discover().unwrap()).unwrap();
        let bytes = vfs.read("maps/d1_trainstation_02.bsp").unwrap().unwrap();
        let bsp = crate::bsp::Bsp::parse(&bytes).unwrap();
        vfs.mount_pak(bsp.lump(40)).unwrap();
        let overlays = read(bsp.lump(45)).unwrap();
        let faces = bsp.lump(7).len().max(bsp.lump(58).len()) / 56;
        let texinfos = bsp.lump(6).len() / 72;
        assert!(!overlays.is_empty());
        for o in &overlays {
            assert!(!o.faces.is_empty() && o.faces.iter().all(|&f| (f as usize) < faces));
            assert!((o.texinfo as usize) < texinfos);
            assert!((o.basis_normal.length() - 1.).abs() < 1e-3);
            assert!((o.basis_u.length() - 1.).abs() < 1e-3);
            assert!(o.basis_u.dot(o.basis_normal).abs() < 1e-3);
        }
        eprintln!("{} overlays", overlays.len());
    }
}
