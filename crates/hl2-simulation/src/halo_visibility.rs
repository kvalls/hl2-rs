//! Beam halos' pixel visibility (Source SDK game/client/c_pixel_visibility.cpp
//! PixelVisibility_FractionVisible, as view_beams.cpp DrawBeamWithHalo uses it).
//!
//! Source draws a proxy for each halo, a square facing the view, its proxy size from its center to
//! each corner and moved that far toward the view, as an occlusion query against the drawn scene.
//! With r_pixelvisibility_partial (on by default, also in the retail client) the halo's color is
//! scaled by the share of the proxy's pixels that pass (0.95 or more counting as all), approached
//! from the last frame's over PIXELVIS_DEFAULT_FADE_TIME. Here rays from the view to the square's
//! center and corners stand in for its pixels, against our colliders rather than drawn meshes
//! (with five samples the 0.95 rule never applies). Not in: the frustum clip share of the proxy
//! and its 5-pixel minimum size.
use crate::ai::scanner::spotlight::Halo;
use glam::Vec3;
use std::collections::HashMap;

/// PIXELVIS_DEFAULT_FADE_TIME: seconds for a halo to fade fully in or out.
const FADE_TIME: f32 = 0.0625;
/// The longest frame a fade steps by, so a hitch does not skip the fade.
const MAX_STEP: f64 = 0.1;

/// Each beam's halo visibility, by the beam's entity, faded over frames.
#[derive(Debug, Default)]
pub struct HaloVisibility {
    faded: HashMap<usize, f32>,
    last_time: Option<f64>,
    step: f32,
}

impl HaloVisibility {
    /// Starts a frame at `time`: the fades move by the time since the last frame.
    pub fn begin_frame(&mut self, time: f64) {
        let elapsed = self
            .last_time
            .map_or(0., |last| (time - last).clamp(0., MAX_STEP));
        self.step = elapsed as f32 / FADE_TIME;
        self.last_time = Some(time);
    }
    /// How visible `halo` is from `view` this frame, 0 to 1: the share of its proxy's sample
    /// points that `clear` reaches from the view, approached from last frame's value.
    pub fn fraction(
        &mut self,
        halo: &Halo,
        view: Vec3,
        axes: (Vec3, Vec3),
        clear: impl Fn(Vec3) -> bool,
    ) -> f32 {
        let points = proxy_points(halo.at, halo.proxy, view, axes);
        let seen = points.iter().filter(|point| clear(**point)).count() as f32;
        let share = seen / points.len() as f32;
        let faded = self.faded.entry(halo.entity).or_insert(0.);
        *faded = crate::attention::approach(share, *faded, self.step);
        *faded
    }
}

/// The proxy's center and corners: a square facing the view whose corners lie `size` from its
/// center (DrawProxy's pyramid base), `size` nearer the view than `at`.
fn proxy_points(at: Vec3, size: f32, view: Vec3, (right, up): (Vec3, Vec3)) -> [Vec3; 5] {
    let center = at - (at - view).normalize_or_zero() * size;
    let half = size * std::f32::consts::FRAC_1_SQRT_2;
    [
        center,
        center + (up - right) * half,
        center + (up + right) * half,
        center + (right - up) * half,
        center - (up + right) * half,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn halo(proxy: f32) -> Halo {
        Halo {
            at: Vec3::new(100., 0., 0.),
            color: [255; 3],
            size: 60.,
            brightness: 0.7,
            entity: 3,
            proxy,
        }
    }

    #[test]
    fn proxy_faces_the_view_nearer_by_its_size_with_corners_at_its_size() {
        let points = proxy_points(Vec3::new(100., 0., 0.), 8., Vec3::ZERO, (Vec3::Y, Vec3::Z));
        assert_eq!(points[0], Vec3::new(92., 0., 0.));
        for corner in &points[1..] {
            assert!((corner.distance(points[0]) - 8.).abs() < 1e-4);
            assert!((corner.x - 92.).abs() < 1e-4);
        }
    }

    #[test]
    fn fraction_fades_toward_the_visible_share_over_the_fade_time() {
        let mut visibility = HaloVisibility::default();
        let axes = (Vec3::Y, Vec3::Z);
        visibility.begin_frame(0.);
        // The first frame has no elapsed time: the halo starts unseen.
        assert_eq!(
            visibility.fraction(&halo(8.), Vec3::ZERO, axes, |_| true),
            0.
        );
        // Half the fade time later it is half in; one more half and it is fully in.
        visibility.begin_frame(FADE_TIME as f64 / 2.);
        let half = visibility.fraction(&halo(8.), Vec3::ZERO, axes, |_| true);
        assert!((half - 0.5).abs() < 1e-4);
        visibility.begin_frame(FADE_TIME as f64);
        assert_eq!(
            visibility.fraction(&halo(8.), Vec3::ZERO, axes, |_| true),
            1.
        );
        // Only the points above the halo's center seen: 2 of 5, approached at the same rate.
        visibility.begin_frame(1.);
        let above = visibility.fraction(&halo(8.), Vec3::ZERO, axes, |p| p.z > 0.1);
        assert!((above - 0.4).abs() < 1e-4);
    }
}
