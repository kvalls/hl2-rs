//! Portable input-binding policy shared by UI hosts.
//!
//! The host translates its native key identifiers at the edge. This module
//! intentionally knows only the persisted Source-style binding names.

use crate::config::{Config, MouseSettings};

/// Returns the configured binding name for an action, or its documented default when the
/// configuration has no entry. An explicitly unbound action returns `""`.
pub fn binding<'a>(config: Option<&'a Config>, action: &str, default: &'a str) -> &'a str {
    config
        .and_then(|config| config.keys.get(action))
        .map(String::as_str)
        .unwrap_or(default)
}

/// `m_yaw` / `m_pitch` default: degrees of view rotation per scaled mouse count.
pub const MOUSE_DEGREES_PER_COUNT: f32 = 0.022;

/// Source mouse processing (SDK `CInput::GetMouseDelta` then `CInput::ScaleMouse`): optional
/// two-sample averaging (`m_filter`) followed by sensitivity, with the Options page's
/// acceleration (`m_customaccel 3`, exponent `m_customaccel_exponent`).
#[derive(Clone, Copy, Debug, Default)]
pub struct MouseFilter {
    previous: [f32; 2],
}

impl MouseFilter {
    /// Returns the scaled delta in counts; multiply by [`MOUSE_DEGREES_PER_COUNT`] for degrees.
    pub fn apply(&mut self, raw: [f32; 2], mouse: &MouseSettings) -> [f32; 2] {
        let [mut x, mut y] = raw;
        if mouse.filter {
            x = (x + self.previous[0]) * 0.5;
            y = (y + self.previous[1]) * 0.5;
        }
        self.previous = raw;
        let scale = if mouse.acceleration {
            let exponent = ((mouse.accel_exponent - 1.) / 2.).max(0.);
            (x * x + y * y).powf(exponent) * mouse.sensitivity
        } else {
            mouse.sensitivity
        };
        [x * scale, y * scale]
    }
}

/// Source-style action names and their default physical key names.
///
/// Hosts use this as the single source of truth when building a native input
/// frame; UI configuration serializes the same names.
pub const MOVEMENT_BINDINGS: &[(&str, &str)] = &[
    ("+forward", "KeyW"),
    ("+back", "KeyS"),
    ("+moveleft", "KeyA"),
    ("+moveright", "KeyD"),
    ("+jump", "Space"),
    ("+duck", "ControlLeft"),
    ("+speed", "ShiftLeft"),
    ("+walk", "AltLeft"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_binding_overrides_the_documented_default() {
        let mut config = Config::default();
        config.keys.insert("+jump".into(), "KeyJ".into());
        assert_eq!(binding(Some(&config), "+jump", "Space"), "KeyJ");
        assert_eq!(binding(Some(&config), "+forward", "KeyW"), "KeyW");
        assert_eq!(binding(None, "+jump", "Space"), "Space");
    }

    #[test]
    fn mouse_filter_averages_two_samples_and_acceleration_follows_mode_3() {
        let mut mouse = MouseSettings {
            sensitivity: 2.,
            ..Default::default()
        };
        let mut filter = MouseFilter::default();
        assert_eq!(filter.apply([3., -4.], &mouse), [6., -8.]);
        mouse.filter = true;
        assert_eq!(
            filter.apply([1., 0.], &mouse),
            [4., -4.],
            "(1 + 3) / 2 and (0 - 4) / 2, scaled by 2"
        );
        mouse.filter = false;
        mouse.acceleration = true;
        mouse.accel_exponent = 1.4;
        // (3² + 4²)^((1.4 - 1) / 2) = 25^0.2
        let scale = 25f32.powf(0.2) * 2.;
        let [x, y] = filter.apply([3., 4.], &mouse);
        assert!((x - 3. * scale).abs() < 1e-4 && (y - 4. * scale).abs() < 1e-4);
        mouse.accel_exponent = 1.;
        assert_eq!(
            filter.apply([3., 4.], &mouse),
            [6., 8.],
            "exponent 1 is linear"
        );
        assert_eq!(filter.apply([0., 0.], &mouse), [0., 0.]);
    }
}
