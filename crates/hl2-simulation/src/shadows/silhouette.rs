//! A caster's silhouette as the engine renders it into its shadow texture (Source SDK
//! `clientshadowmgr.cpp` `DrawRenderToTextureShadow`, `shadowbuild_dx9.cpp`): its triangles seen
//! orthographically along the shadow direction into an `n` x `n` cell, both faces, coverage 1
//! (alpha-tested and translucent parts by their texture alpha) added and saturating.

use super::frame::Frame;
use glam::{Vec2, Vec3};

/// Cell sizes, texels.
pub const MIN_CELL: u32 = 16;
pub const MAX_CELL: u32 = 256;

/// A caster's cell size (`SetupRenderToTextureShadow`, `AllocateTexture`): two texels per unit of
/// its largest dimension, truncated, the power of two at or above it, 16 to 256.
pub fn cell_size(bounds: (Vec3, Vec3)) -> u32 {
    let texels = (super::TEXELS_PER_UNIT * (bounds.1 - bounds.0).max_element()) as u32;
    texels.max(1).next_power_of_two().clamp(MIN_CELL, MAX_CELL)
}

/// Clears `cell` to `n * n` texels of no coverage.
pub fn clear(cell: &mut Vec<f32>, n: u32) {
    cell.clear();
    cell.resize((n * n) as usize, 0.);
}

/// Adds one world-space triangle to the cell; `alpha` gives its coverage at a texture coordinate
/// (`None`: opaque).
pub fn draw(
    cell: &mut [f32],
    n: u32,
    frame: &Frame,
    (points, uvs): ([Vec3; 3], [Vec2; 3]),
    alpha: Option<&dyn Fn(Vec2) -> f32>,
) {
    let scale = n as f32;
    let p = points.map(|p| frame.uv(p) * scale);
    let area = (p[1] - p[0]).perp_dot(p[2] - p[0]);
    if area.abs() < 1e-9 || !area.is_finite() {
        return;
    }
    let lo = p[0].min(p[1]).min(p[2]).floor().max(Vec2::ZERO);
    let hi = p[0].max(p[1]).max(p[2]).ceil().min(Vec2::splat(scale - 1.));
    if lo.x > hi.x || lo.y > hi.y {
        return;
    }
    // Barycentric weights are linear in the texel centre: an edge function (a - c) perp (b - c) is
    // a perp b + c.x (a.y - b.y) + c.y (b.x - a.x). Along a row each weight is `at + x * slope`;
    // the texels where all three are non-negative form one span.
    let inv = 1. / area;
    let edge = |a: Vec2, b: Vec2| (a.perp_dot(b) * inv, (a.y - b.y) * inv, (b.x - a.x) * inv);
    let (e0, e1) = (edge(p[1], p[2]), edge(p[2], p[0]));
    let slopes = [e0.1, e1.1, -e0.1 - e1.1];
    for y in lo.y as u32..=hi.y as u32 {
        let cy = y as f32 + 0.5;
        let (a0, a1) = (e0.0 + cy * e0.2, e1.0 + cy * e1.2);
        let at = [a0, a1, 1. - a0 - a1];
        let Some((first, last)) = span(at, slopes, (lo.x, hi.x)) else {
            continue;
        };
        let row = &mut cell[(y * n) as usize..((y + 1) * n) as usize];
        let texels = &mut row[first as usize..=last as usize];
        match alpha {
            None => texels.fill(1.),
            Some(a) => {
                for (texel, x) in texels.iter_mut().zip(first..) {
                    let cx = x as f32 + 0.5;
                    let (w0, w1) = (at[0] + cx * slopes[0], at[1] + cx * slopes[1]);
                    let uv = uvs[0] * w0 + uvs[1] * w1 + uvs[2] * (1. - w0 - w1);
                    *texel = (*texel + a(uv)).min(1.);
                }
            }
        }
    }
}

/// The texels of a row (within `lo..=hi`) whose centres `x + 0.5` keep every weight
/// `at + centre * slope` non-negative.
fn span(at: [f32; 3], slopes: [f32; 3], (lo, hi): (f32, f32)) -> Option<(u32, u32)> {
    let (mut left, mut right) = (lo + 0.5, hi + 0.5);
    for (a, s) in at.into_iter().zip(slopes) {
        if s > 0. {
            left = left.max(-a / s);
        } else if s < 0. {
            right = right.min(-a / s);
        } else if a < 0. {
            return None;
        }
    }
    let (first, last) = ((left - 0.5).ceil(), (right - 0.5).floor());
    (first <= last).then_some((first as u32, last as u32))
}
