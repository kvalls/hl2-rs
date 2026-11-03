//! One caster's shadow projection (Source SDK `clientshadowmgr.cpp`: `BuildRenderToTextureShadow`,
//! `BuildOrthoShadow`, `ComputeLocalShadowOrigin`, `ComputeExtraClipPlanes`): where the shadow starts,
//! its texture axes and size, how far it reaches and where it starts to fade.

use super::TEXELS_PER_UNIT;
use glam::{Mat3, Vec2, Vec3};

/// A caster's projection: shadow space is `(x, y, d)` from `origin` (x and y are perpendicular to `d`,
/// but not always to each other for a blob), texture coordinates `coordinate / size + 0.5`.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub origin: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    /// Unit shadow direction (caster toward receiver).
    pub d: Vec3,
    pub size: Vec2,
    /// Depth along `d` where the fade starts.
    pub falloff_start: f32,
    /// Depth along `d` where the shadow ends (cast distance + falloff start).
    pub max_height: f32,
    /// Planes `n . p <= dist` a receiver point must satisfy (the caster box's light-facing faces).
    pub clip: Vec<(Vec3, f32)>,
    to_shadow: Mat3,
}

/// The caster's placement: its render origin and its local axes (forward, left, up; Source's
/// `AngleVectors` with the right vector negated).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub origin: Vec3,
    pub axes: [Vec3; 3],
}

impl Placement {
    /// A placement from an origin and a rotation (rotation of the model's X, Y, Z).
    pub fn new(origin: Vec3, rotation: glam::Quat) -> Self {
        Self {
            origin,
            axes: [rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z],
        }
    }

    fn world(&self, local: Vec3) -> Vec3 {
        self.origin + self.axes[0] * local.x + self.axes[1] * local.y + self.axes[2] * local.z
    }

    fn local_direction(&self, d: Vec3) -> Vec3 {
        Vec3::new(
            self.axes[0].dot(d),
            self.axes[1].dot(d),
            self.axes[2].dot(d),
        )
    }
}

/// `ComputeLocalShadowOrigin`: the point on the line through the box centroid along the local
/// direction at the box corner furthest back (times `backup`), and the falloff start (the bounding
/// sphere's radius less that signed distance).
fn local_origin(bounds: (Vec3, Vec3), local_d: Vec3, backup: f32) -> (Vec3, f32) {
    let (mins, maxs) = bounds;
    let centroid = (mins + maxs) * 0.5;
    let radius = (maxs - mins).length() * 0.5;
    let mut min_dist = -centroid.dot(local_d);
    for i in 0..3 {
        min_dist += local_d[i] * if local_d[i] > 0. { mins[i] } else { maxs[i] };
    }
    min_dist *= backup;
    (centroid + local_d * min_dist, radius - min_dist)
}

impl Frame {
    fn finish(mut self) -> Self {
        // World to shadow space: the inverse of the (x, y, d) basis (a general inverse, as x and y
        // need not be perpendicular).
        self.to_shadow = Mat3::from_cols(self.x, self.y, self.d).inverse();
        self
    }

    /// Shadow-space coordinates of a world point.
    pub fn local(&self, p: Vec3) -> Vec3 {
        self.to_shadow * (p - self.origin)
    }

    /// Texture coordinates of a world point: 0..1 across the shadow, then its shadow depth.
    pub fn texture(&self, p: Vec3) -> Vec3 {
        let l = self.local(p);
        Vec3::new(l.x / self.size.x + 0.5, l.y / self.size.y + 0.5, l.z)
    }

    /// 0..1 texture coordinates of a world point.
    pub fn uv(&self, p: Vec3) -> Vec2 {
        self.texture(p).truncate()
    }

    /// The fade byte a shadow decal vertex at shadow depth `z` carries (the receiver shader takes
    /// it from the coverage): `MAX_FALLOFF` (240) times the depth past the falloff start over the
    /// fade span, plus the caster's falloff `bias`, at most 255 and truncated. Points past the end
    /// keep fading up to 255 (the engine clips nothing there).
    pub fn fade(&self, z: f32, bias: u8) -> u8 {
        let span = self.max_height - self.falloff_start;
        let scale = if span > 0. { 1. / span } else { 1. };
        let fade =
            super::MAX_FALLOFF * ((z - self.falloff_start) * scale).max(0.) + f32::from(bias);
        fade.min(255.) as u8
    }

    /// `BuildRenderToTextureShadow` for render bounds `bounds` (model space) of a caster at
    /// `placement`, cast along `d` for `distance` units past it.
    pub fn render_to_texture(
        bounds: (Vec3, Vec3),
        placement: &Placement,
        d: Vec3,
        distance: f32,
    ) -> Self {
        let size3 = bounds.1 - bounds.0;
        let axes = placement.axes;
        let mut y = Vec3::ZERO;
        let mut longest = 0.;
        for i in 0..3 {
            let test = (axes[i] - d * d.dot(axes[i])) * size3[i];
            if test.length_squared() > longest {
                longest = test.length_squared();
                y = test;
            }
        }
        let y = y.normalize_or_zero();
        let x = y.cross(d);
        let across = |v: Vec3| (0..3).map(|i| size3[i] * axes[i].dot(v).abs()).sum::<f32>();
        let size = Vec2::new(across(x), across(y)) + Vec2::splat(2. * TEXELS_PER_UNIT);
        let local_d = placement.local_direction(d);
        let (local, falloff_start) = local_origin(bounds, local_d, 1.);
        Self {
            origin: placement.world(local),
            x,
            y,
            d,
            size,
            falloff_start,
            max_height: distance + falloff_start,
            clip: clip_planes(bounds, placement, local_d),
            to_shadow: Mat3::IDENTITY,
        }
        .finish()
    }

    /// `BuildOrthoShadow` (a blob): the two box axes most perpendicular to `d` projected across it,
    /// 10 units bigger (at least 10), the origin backed up twice as far and snapped to half a unit,
    /// no clip planes.
    pub fn blob(bounds: (Vec3, Vec3), placement: &Placement, d: Vec3, distance: f32) -> Self {
        let axes = placement.axes;
        let local_d = placement.local_direction(d);
        let order = sorted_axes(local_d);
        let project = |v: Vec3| (v - d * d.dot(v)).normalize_or_zero();
        let (x, y) = (project(axes[order[0]]), project(axes[order[1]]));
        let size3 = bounds.1 - bounds.0;
        let size = Vec2::new(
            size3[order[0]] * axes[order[0]].dot(x).abs()
                + size3[order[2]] * axes[order[2]].dot(x).abs(),
            size3[order[1]] * axes[order[1]].dot(y).abs()
                + size3[order[2]] * axes[order[2]].dot(y).abs(),
        );
        let size = (size + Vec2::splat(10.)).max(Vec2::splat(10.));
        let (local, falloff_start) = local_origin(bounds, local_d, 2.);
        let snap = 1. / TEXELS_PER_UNIT;
        let origin = (placement.world(local) / snap).trunc() * snap;
        Self {
            origin,
            x,
            y,
            d,
            size,
            falloff_start,
            max_height: distance + falloff_start,
            clip: Vec::new(),
            to_shadow: Mat3::IDENTITY,
        }
        .finish()
    }
}

/// `SortAbsVectorComponents`: the axis most along the direction last, the other two in
/// right-handed order before it.
fn sorted_axes(v: Vec3) -> [usize; 3] {
    let a = v.abs();
    let mut max = if a.x > a.y { 0 } else { 1 };
    if a.z > a[max] {
        max = 2;
    }
    match max {
        0 => [1, 2, 0],
        1 => [2, 0, 1],
        _ => [0, 1, 2],
    }
}

/// `ComputeExtraClipPlanes`: the three box faces facing the light, through the box corner toward
/// it, as `n . p <= dist`.
fn clip_planes(bounds: (Vec3, Vec3), placement: &Placement, local_d: Vec3) -> Vec<(Vec3, f32)> {
    let mut corner = placement.origin;
    let mut sign = [0f32; 3];
    for i in 0..3 {
        if local_d[i] < 0. {
            corner += placement.axes[i] * bounds.1[i];
            sign[i] = 1.;
        } else {
            corner += placement.axes[i] * bounds.0[i];
            sign[i] = -1.;
        }
    }
    (0..3)
        .map(|i| {
            let normal = placement.axes[i] * sign[i];
            (normal, normal.dot(corner))
        })
        .collect()
}
