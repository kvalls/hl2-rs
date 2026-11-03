//! Portable viewmodel framing values shared by hosts and extensions.
use serde::Deserialize;

/// Additive viewmodel horizontal field-of-view offset (degrees, measured at the 4:3 reference
/// aspect) and local camera-space translation offset (`[forward, left, up]` in Source units) for
/// frames narrower than `5:4` or wider than `16:9`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ViewmodelFraming {
    pub fov_offset: f32,
    pub offset: [f32; 3],
}

impl ViewmodelFraming {
    pub const ZERO: Self = Self {
        fov_offset: 0.0,
        offset: [0.0; 3],
    };
}
