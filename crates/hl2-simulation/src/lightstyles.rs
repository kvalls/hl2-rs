//! Light styles (SDK lights.cpp CLight and the engine's lightstyle strings): a named `light`,
//! `light_spot` or `light_glspot` owns a style (32-63) whose value scales the lightmap samples
//! vrad baked for it. Spawn sets "a" (off) with SF_LIGHT_START_OFF, else its pattern or "m";
//! TurnOn, TurnOff, Toggle and SetPattern change it. The engine steps a pattern at 10 Hz; one
//! value a style is kept here: the pattern's mean, each letter (c - 'a') * 22 / 264 ('m' is 1).
//! FadeToPattern and the `defaultstyle` presets are not modeled.
use std::collections::BTreeMap;

/// Styles the engine keeps (MAX_LIGHTSTYLES).
const STYLES: usize = 64;
/// SF_LIGHT_START_OFF.
const START_OFF: u32 = 1;

/// One light entity's style, pattern (m_iszPattern) and off state.
#[derive(Clone, Debug)]
struct Light {
    style: usize,
    pattern: String,
    off: bool,
}

#[derive(Clone, Debug)]
pub struct LightStyles {
    values: [f32; STYLES],
    /// Bumped whenever a value changes, for the host's lightmap recomposition.
    generation: u64,
    lights: BTreeMap<usize, Light>,
}

impl Default for LightStyles {
    fn default() -> Self {
        Self {
            values: [1.; STYLES],
            generation: 0,
            lights: BTreeMap::new(),
        }
    }
}

/// A lightstyle string's value: the mean of its letters ('a' 0, 'm' 1, 'z' 25/12).
pub fn pattern_value(pattern: &str) -> f32 {
    let letters = pattern.as_bytes();
    if letters.is_empty() {
        return 1.;
    }
    let sum: f32 = letters
        .iter()
        .map(|&c| f32::from(c.saturating_sub(b'a')) * 22. / 264.)
        .sum();
    sum / letters.len() as f32
}

impl LightStyles {
    /// The current scale of `style` (1 for styles no light owns, like style 0).
    pub fn value(&self, style: u8) -> f32 {
        self.values.get(usize::from(style)).copied().unwrap_or(1.)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn apply(&mut self, id: usize) {
        let Some(light) = self.lights.get(&id) else {
            return;
        };
        let value = if light.off {
            0.
        } else if light.pattern.is_empty() {
            1.
        } else {
            pattern_value(&light.pattern)
        };
        if self.values[light.style] != value {
            self.values[light.style] = value;
            self.generation += 1;
        }
    }

    /// CLight::Spawn of entity `id` with its `style`, `pattern` and spawnflags; lights below
    /// style 32 have no switchable style.
    pub fn spawn(&mut self, id: usize, style: i32, pattern: &str, spawnflags: u32) {
        let Some(style) = usize::try_from(style)
            .ok()
            .filter(|s| (32..STYLES).contains(s))
        else {
            return;
        };
        let off = spawnflags & START_OFF != 0;
        let pattern = pattern.to_owned();
        self.lights.insert(
            id,
            Light {
                style,
                pattern,
                off,
            },
        );
        self.apply(id);
    }

    /// TurnOn (`Some(true)`), TurnOff (`Some(false)`) or Toggle (`None`).
    pub fn switch(&mut self, id: usize, on: Option<bool>) {
        if let Some(light) = self.lights.get_mut(&id) {
            light.off = !on.unwrap_or(light.off);
        }
        self.apply(id);
    }

    /// SetPattern: the new pattern, and the light is on.
    pub fn set_pattern(&mut self, id: usize, pattern: &str) {
        if let Some(light) = self.lights.get_mut(&id) {
            light.pattern = pattern.to_owned();
            light.off = false;
        }
        self.apply(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_light_starts_off_turns_on_toggles_and_takes_a_pattern() {
        let mut styles = LightStyles::default();
        styles.spawn(7, 33, "", START_OFF);
        styles.spawn(8, 35, "", 0);
        styles.spawn(9, 0, "", START_OFF);
        assert_eq!(
            (styles.value(33), styles.value(35), styles.value(0)),
            (0., 1., 1.)
        );
        let before = styles.generation();
        styles.switch(7, Some(true));
        assert_eq!(styles.value(33), 1.);
        assert!(styles.generation() > before);
        styles.switch(7, None);
        assert_eq!(styles.value(33), 0.);
        styles.set_pattern(8, "az");
        assert!((styles.value(35) - 25. / 24.).abs() < 1e-6);
        // Lights without a switchable style change nothing.
        let unchanged = styles.generation();
        styles.switch(9, Some(false));
        assert_eq!(styles.generation(), unchanged);
    }
}
