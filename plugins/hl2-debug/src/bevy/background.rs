//! Background visual debug mode: overlay, blur adjustment, and camera FOV tuning.

use bevy::prelude::KeyCode;
pub const BACKGROUND_ADJUST_KEYS: &[KeyCode] = &[KeyCode::ArrowDown, KeyCode::ArrowUp];
pub const BACKGROUND_OPACITY_KEYS: &[KeyCode] = &[KeyCode::Comma, KeyCode::Period];

/// Derives a readable aspect ratio label (e.g. "16:9", "4:3", "Fit Display").
pub fn aspect_label(aspect: f32, flexible: bool) -> &'static str {
    if flexible {
        "Fit Display"
    } else if (aspect - 16. / 9.).abs() < 0.05 {
        "16:9"
    } else if (aspect - 16. / 10.).abs() < 0.05 {
        "16:10"
    } else if (aspect - 4. / 3.).abs() < 0.05 {
        "4:3"
    } else if (aspect - 5. / 4.).abs() < 0.05 {
        "5:4"
    } else if (aspect - 21. / 9.).abs() < 0.1 {
        "21:9"
    } else if (aspect - 32. / 9.).abs() < 0.1 {
        "32:9"
    } else if (aspect - 1.).abs() < 0.05 {
        "1:1"
    } else if (aspect - 3. / 4.).abs() < 0.05 {
        "3:4"
    } else if (aspect - 9. / 16.).abs() < 0.05 {
        "9:16"
    } else {
        "Custom"
    }
}

/// Runtime background layout visual analysis state.
#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundDebug {
    pub enabled: bool,
    pub blur_opacity: u8,
    pub base_blur_opacity: u8,
    pub fov: Option<f32>,
    pub state_label: String,
    pub map_name: String,
    pub blur_texture_name: String,
    pub last_aspect: Option<f32>,
    pub pending_gameplay_finish: bool,
}

impl Default for BackgroundDebug {
    fn default() -> Self {
        Self {
            enabled: false,
            blur_opacity: 50,
            base_blur_opacity: 50,
            fov: None,
            state_label: "Ready".to_owned(),
            map_name: "background01".to_owned(),
            blur_texture_name: "background01_widescreen".to_owned(),
            last_aspect: None,
            pending_gameplay_finish: false,
        }
    }
}

impl BackgroundDebug {
    /// Current field of view in degrees.
    pub fn fov(&self, default_fov: f32) -> f32 {
        self.fov.unwrap_or(default_fov)
    }

    /// Formats the lines displayed in the debug overlay.
    pub fn lines(&self, default_fov: f32, force_default_blur: bool) -> Vec<String> {
        let fov = self.fov(default_fov);
        vec![
            format!("state: {}  |  map: {}", self.state_label, self.map_name),
            format!("blur_texture: {}", self.blur_texture_name),
            format!(
                "background_force_default_blur: {}  (-: toggle)",
                u8::from(force_default_blur)
            ),
            format!(
                "blur_opacity: {}%  (left: 0  right: 100  ,: -1%  .: +1%)",
                self.blur_opacity
            ),
            format!("fov: {:.2}°  (up: +0.1  down: -0.1  0: reset)", fov),
            "1-4: hl2/ep1/ep2/lc    [/]: prev/next chapter bg".to_owned(),
            "Shift = 1.0  Alt = 0.01    hold to repeat".to_owned(),
        ]
    }

    /// Adjusts blur opacity by 1% step for Comma (decrease) or Period (increase).
    pub fn adjust_opacity_step(&mut self, fired: &[KeyCode]) -> bool {
        let mut changed = false;
        for key in fired {
            match *key {
                KeyCode::Comma => {
                    self.blur_opacity = self.blur_opacity.saturating_sub(1);
                    self.base_blur_opacity = self.blur_opacity;
                    changed = true;
                }
                KeyCode::Period => {
                    self.blur_opacity = (self.blur_opacity + 1).min(100);
                    self.base_blur_opacity = self.blur_opacity;
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }

    /// Sets blur opacity to 0, or toggles back to base blur opacity.
    pub fn toggle_zero_opacity(&mut self) -> bool {
        if self.blur_opacity == 0 {
            self.blur_opacity = if self.base_blur_opacity == 0 {
                50
            } else {
                self.base_blur_opacity
            };
        } else {
            self.blur_opacity = 0;
        }
        true
    }

    /// Sets blur opacity to 100, or toggles back to base blur opacity.
    pub fn toggle_full_opacity(&mut self) -> bool {
        if self.blur_opacity == 100 {
            self.blur_opacity = if self.base_blur_opacity == 100 {
                50
            } else {
                self.base_blur_opacity
            };
        } else {
            self.blur_opacity = 100;
        }
        true
    }

    /// Adjusts FOV based on fired arrow keys.
    pub fn adjust_fov(&mut self, fired: &[KeyCode], step: f32, default_fov: f32) -> bool {
        let mut v = self.fov(default_fov);
        let mut changed = false;
        for key in fired {
            match *key {
                KeyCode::ArrowDown => {
                    v = ((v - step) * 100.0).round() / 100.0;
                    self.fov = Some(v.clamp(1.0, 170.0));
                    changed = true;
                }
                KeyCode::ArrowUp => {
                    v = ((v + step) * 100.0).round() / 100.0;
                    self.fov = Some(v.clamp(1.0, 170.0));
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }

    /// Reset an active override when the host reports a changed display aspect.
    pub fn check_aspect_change(&mut self, aspect: f32, default_fov: f32) -> bool {
        let changed = self.enabled
            && self
                .last_aspect
                .is_some_and(|last| (last - aspect).abs() > 1e-4);
        self.last_aspect = Some(aspect);
        if changed {
            self.fov = Some(default_fov);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_debug_default_fov_and_overrides() {
        let mut debug = BackgroundDebug::default();
        let def_v = debug.fov(60.);
        assert!((def_v - 59.85).abs() < 0.2);

        // Setting explicit FOV
        debug.fov = Some(65.0);
        assert_eq!(debug.fov(60.), 65.0);
    }

    #[test]
    fn background_debug_arrow_adjustments() {
        let mut debug = BackgroundDebug::default();
        let init_v = debug.fov(60.);

        // ArrowUp increases FOV
        assert!(debug.adjust_fov(&[KeyCode::ArrowUp], 2.0, 60.));
        assert!((debug.fov.unwrap() - (init_v + 2.0)).abs() < 0.02);

        // ArrowDown decreases FOV
        assert!(debug.adjust_fov(&[KeyCode::ArrowDown], 1.0, 60.));
        assert!((debug.fov.unwrap() - (init_v + 1.0)).abs() < 0.02);
    }

    #[test]
    fn background_debug_overlay_lines() {
        let debug = BackgroundDebug {
            enabled: true,
            blur_opacity: 50,
            map_name: "background01".to_owned(),
            blur_texture_name: "background01_widescreen".to_owned(),
            state_label: "Ready".to_owned(),
            ..Default::default()
        };
        let lines = debug.lines(16. / 9., false);
        assert_eq!(lines.len(), 7);
        assert!(lines[0].contains("state: Ready"));
        assert!(lines[0].contains("map: background01"));
        assert!(lines[1].contains("blur_texture: background01_widescreen"));
        assert_eq!(lines[2], "background_force_default_blur: 0  (-: toggle)");
        assert!(debug.lines(16. / 9., true)[2].contains("background_force_default_blur: 1"));
        assert_eq!(
            lines[3],
            "blur_opacity: 50%  (left: 0  right: 100  ,: -1%  .: +1%)"
        );
        assert!(lines[4].contains("fov: "));
        assert!(lines[4].contains("0: reset"));
        assert!(lines[5].contains("1-4:"));
        assert!(lines[5].contains("[/]"));
    }

    #[test]
    fn aspect_change_resets_only_active_overrides() {
        let mut debug = BackgroundDebug {
            enabled: true,
            last_aspect: Some(4. / 3.),
            ..Default::default()
        };
        assert!(debug.check_aspect_change(16. / 9., 60.));
        assert_eq!(debug.fov, Some(60.));
        assert!(!debug.check_aspect_change(16. / 9., 60.));
        debug.enabled = false;
        assert!(!debug.check_aspect_change(4. / 3., 70.));
        assert_eq!(debug.fov, Some(60.));
    }

    #[test]
    fn background_debug_opacity_adjustments_and_toggles() {
        let mut debug = BackgroundDebug::default();
        assert_eq!(debug.blur_opacity, 50);
        assert_eq!(debug.base_blur_opacity, 50);

        // Comma decreases opacity by 1
        assert!(debug.adjust_opacity_step(&[KeyCode::Comma]));
        assert_eq!(debug.blur_opacity, 49);
        assert_eq!(debug.base_blur_opacity, 49);

        // Period increases opacity by 1
        assert!(debug.adjust_opacity_step(&[KeyCode::Period]));
        assert_eq!(debug.blur_opacity, 50);
        assert_eq!(debug.base_blur_opacity, 50);

        // Toggle zero sets to 0
        assert!(debug.toggle_zero_opacity());
        assert_eq!(debug.blur_opacity, 0);
        assert_eq!(debug.base_blur_opacity, 50);

        // Repeated click on zero toggles back to base
        assert!(debug.toggle_zero_opacity());
        assert_eq!(debug.blur_opacity, 50);

        // Toggle full sets to 100
        assert!(debug.toggle_full_opacity());
        assert_eq!(debug.blur_opacity, 100);
        assert_eq!(debug.base_blur_opacity, 50);

        // Repeated click on full toggles back to base
        assert!(debug.toggle_full_opacity());
        assert_eq!(debug.blur_opacity, 50);
    }
}
