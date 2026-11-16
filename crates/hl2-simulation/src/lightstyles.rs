//! Light styles: the engine's 64 style strings and the values baked light is scaled by. The
//! world sets styles 0-12 to its default patterns and 63 to "a" (SDK world.cpp
//! g_DefaultLightstyles; the same strings are in the retail server.dll). A named `light`,
//! `light_spot` or `light_glspot` (SDK lights.cpp CLight) owns a style 32-63 whose samples
//! vrad baked: Spawn sets "a" with SF_LIGHT_START_OFF, else its pattern, its `defaultstyle`
//! preset or "m"; TurnOn, TurnOff, Toggle, SetPattern and FadeToPattern change it. The engine
//! steps every style each client frame (retail engine.dll 0x1011b830): the letter at
//! (int)(time x 10) modulo the string's length, worth (letter - 'a') x 22 of 264 ('m' is 1,
//! 'z' 25/12); a style without a string is worth 256. Only styles a face uses animate here, so
//! the generation (the host's cue to recompose lightmaps) moves only for light that is seen.
use std::collections::BTreeMap;

/// Styles the engine keeps (MAX_LIGHTSTYLES).
const STYLES: usize = 64;
/// SF_LIGHT_START_OFF.
const START_OFF: u32 = 1;
/// CLight::FadeThink's interval.
const FADE_STEP: f64 = 0.1;
/// SDK world.cpp g_DefaultLightstyles: 0 normal, 1 flicker, 2 slow strong pulse, 3 candle,
/// 4 fast strobe, 5 gentle pulse, 6 flicker, 7 and 8 candles, 9 slow strobe, 10 fluorescent
/// flicker, 11 slow pulse not fading to black, 12 underwater mutation.
const DEFAULT_STYLES: [&str; 13] = [
    "m",
    "mmnmmommommnonmmonqnmmo",
    "abcdefghijklmnopqrstuvwxyzyxwvutsrqponmlkjihgfedcba",
    "mmmmmaaaaammmmmaaaaaabcdefgabcdefg",
    "mamamamamama",
    "jklmnopqrstuvwxyzyxwvutsrqponmlkj",
    "nmonqnmomnmomomno",
    "mmmaaaabcdefgmmmmaaaammmaamm",
    "mmmaaammmaaammmabcdefaaaammmmabcdefmmmaaaa",
    "aaaaaaaazzzzzzzz",
    "mmamammmmammamamaaamammma",
    "abcdefghijklmnopqrrqponmlkjihgfedcba",
    "mmnnmmnnnmmnn",
];

/// GetDefaultLightstyleString: a preset, or "m" past the table.
fn default_string(style: usize) -> &'static str {
    DEFAULT_STYLES.get(style).copied().unwrap_or("m")
}

/// A style string's value in this frame of the 10 Hz animation; letters below 'a' (a fade from
/// a light without a pattern starts at NUL) light nothing.
fn string_value(string: &str, frame: i64) -> f32 {
    let letters = string.as_bytes();
    if letters.is_empty() {
        return 256. / 264.;
    }
    let letter = letters[frame.rem_euclid(letters.len() as i64) as usize];
    ((f32::from(letter) - f32::from(b'a')) * 22. / 264.).max(0.)
}

/// CLight::InputFadeToPattern's think: the letter shown and the one it walks to.
#[derive(Clone, Debug)]
struct Fade {
    current: u8,
    target: u8,
    next: f64,
}

/// One light entity's style, pattern (m_iszPattern), off state and fade.
#[derive(Clone, Debug)]
struct Light {
    style: usize,
    pattern: String,
    off: bool,
    fade: Option<Fade>,
}

#[derive(Clone, Debug)]
pub struct LightStyles {
    strings: Vec<String>,
    values: [f32; STYLES],
    /// Styles a lightmap face uses: only these animate.
    used: [bool; STYLES],
    /// The animation frame the values were taken at.
    frame: i64,
    /// Bumped whenever a used style's value changes, for the host's lightmap recomposition.
    generation: u64,
    lights: BTreeMap<usize, Light>,
}

impl Default for LightStyles {
    fn default() -> Self {
        let strings: Vec<String> = (0..STYLES)
            .map(|s| match s {
                0..=12 => default_string(s),
                63 => "a",
                _ => "",
            })
            .map(str::to_owned)
            .collect();
        let mut values = [0.; STYLES];
        for (value, string) in values.iter_mut().zip(&strings) {
            *value = string_value(string, 0);
        }
        Self {
            strings,
            values,
            used: [false; STYLES],
            frame: 0,
            generation: 0,
            lights: BTreeMap::new(),
        }
    }
}

impl LightStyles {
    /// The current scale of `style`.
    pub fn value(&self, style: u8) -> f32 {
        self.values.get(usize::from(style)).copied().unwrap_or(1.)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Marks the styles lightmap faces use.
    pub fn use_styles(&mut self, styles: impl IntoIterator<Item = u8>) {
        for style in styles {
            if let Some(used) = self.used.get_mut(usize::from(style)) {
                *used = true;
            }
        }
    }

    fn refresh(&mut self, style: usize) {
        let value = string_value(&self.strings[style], self.frame);
        if self.values[style] != value {
            self.values[style] = value;
            if self.used[style] {
                self.generation += 1;
            }
        }
    }

    /// engine->LightStyle(style, string).
    fn set(&mut self, style: usize, string: &str) {
        string.clone_into(&mut self.strings[style]);
        self.refresh(style);
    }

    /// The fades' thinks and this frame's letters of the used styles.
    pub fn animate(&mut self, time: f64) {
        self.think_fades(time);
        let frame = (time * 10.) as i64;
        if frame == self.frame {
            return;
        }
        self.frame = frame;
        for style in 0..STYLES {
            if self.used[style] && self.strings[style].len() > 1 {
                self.refresh(style);
            }
        }
    }

    /// CLight::FadeThink: one letter toward the target every 0.1 s, then the new pattern.
    fn think_fades(&mut self, time: f64) {
        let due: Vec<usize> = (self.lights.iter())
            .filter(|(_, l)| l.fade.as_ref().is_some_and(|f| f.next <= time))
            .map(|(&id, _)| id)
            .collect();
        for id in due {
            let light = self.lights.get_mut(&id).expect("due light");
            let style = light.style;
            let fade = light.fade.as_mut().expect("due fade");
            fade.current = match fade.current.cmp(&fade.target) {
                std::cmp::Ordering::Less => fade.current + 1,
                std::cmp::Ordering::Greater => fade.current - 1,
                std::cmp::Ordering::Equal => fade.current,
            };
            if fade.current == fade.target {
                let pattern = light.pattern.clone();
                light.fade = None;
                self.set(style, &pattern);
            } else {
                fade.next = time + FADE_STEP;
                let letter = char::from(fade.current).to_string();
                self.set(style, &letter);
            }
        }
    }

    /// CLight::Spawn of entity `id` with its `style`, `pattern`, `defaultstyle` and spawnflags;
    /// lights below style 32 have no switchable style.
    pub fn spawn(&mut self, id: usize, style: i32, pattern: &str, preset: i32, spawnflags: u32) {
        let Some(style) = usize::try_from(style)
            .ok()
            .filter(|s| (32..STYLES).contains(s))
        else {
            return;
        };
        let pattern = match usize::try_from(preset) {
            Ok(preset) if pattern.is_empty() && preset > 0 => default_string(preset),
            _ => pattern,
        }
        .to_owned();
        let off = spawnflags & START_OFF != 0;
        let string = match (off, pattern.is_empty()) {
            (true, _) => "a".to_owned(),
            (false, true) => "m".to_owned(),
            (false, false) => pattern.clone(),
        };
        self.lights.insert(
            id,
            Light {
                style,
                pattern,
                off,
                fade: None,
            },
        );
        self.set(style, &string);
    }

    /// TurnOn (`Some(true)`), TurnOff (`Some(false)`) or Toggle (`None`).
    pub fn switch(&mut self, id: usize, on: Option<bool>) {
        let Some(light) = self.lights.get_mut(&id) else {
            return;
        };
        light.off = !on.unwrap_or(light.off);
        let string = match (light.off, light.pattern.is_empty()) {
            (true, _) => "a".to_owned(),
            (false, true) => "m".to_owned(),
            (false, false) => light.pattern.clone(),
        };
        let style = light.style;
        self.set(style, &string);
    }

    /// SetPattern: the new pattern, and the light is on.
    pub fn set_pattern(&mut self, id: usize, pattern: &str) {
        let Some(light) = self.lights.get_mut(&id) else {
            return;
        };
        pattern.clone_into(&mut light.pattern);
        light.off = false;
        let style = light.style;
        self.set(style, pattern);
    }

    /// FadeToPattern: from the old pattern's first letter (NUL without one) toward the new
    /// one's, starting with the next think; the light is on.
    pub fn fade_to_pattern(&mut self, id: usize, pattern: &str, time: f64) {
        let Some(light) = self.lights.get_mut(&id) else {
            return;
        };
        light.fade = Some(Fade {
            current: light.pattern.bytes().next().unwrap_or(0),
            target: pattern.bytes().next().unwrap_or(0),
            next: time,
        });
        pattern.clone_into(&mut light.pattern);
        light.off = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_light_starts_off_turns_on_toggles_and_takes_a_pattern() {
        let mut styles = LightStyles::default();
        styles.use_styles([33, 35, 0]);
        styles.spawn(7, 33, "", 0, START_OFF);
        styles.spawn(8, 35, "", 0, 0);
        styles.spawn(9, 0, "", 0, START_OFF);
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
        assert_eq!(styles.value(35), 0.);
        styles.animate(0.1);
        assert!((styles.value(35) - 25. / 12.).abs() < 1e-6);
        // Lights without a switchable style change nothing.
        let unchanged = styles.generation();
        styles.switch(9, Some(false));
        assert_eq!(styles.generation(), unchanged);
    }

    #[test]
    fn used_styles_step_through_their_letters_ten_times_a_second() {
        let mut styles = LightStyles::default();
        // Style 10, fluorescent flicker "mmamammm...": 'm', 'm', 'a', 'm', 'a'.
        styles.use_styles([10]);
        let seen: Vec<f32> = [0.05, 0.15, 0.25, 0.35, 0.45]
            .iter()
            .map(|&t| {
                styles.animate(t);
                styles.value(10)
            })
            .collect();
        assert_eq!(seen, [1., 1., 0., 1., 0.]);
        // An unused style keeps its first letter (style 1's sixth is 'o').
        styles.animate(0.55);
        assert_eq!(styles.value(1), 1.);
        // Styles without a string are worth 256 of 264; 63 is off.
        assert_eq!((styles.value(20), styles.value(63)), (256. / 264., 0.));
    }

    #[test]
    fn defaultstyle_presets_and_fade_to_pattern_walk_one_letter_a_tenth() {
        let mut styles = LightStyles::default();
        styles.use_styles([40, 41]);
        // defaultstyle 4 (fast strobe) for a light without a pattern.
        styles.spawn(1, 40, "", 4, 0);
        styles.animate(0.15);
        assert_eq!(styles.value(40), 0.);
        styles.spawn(2, 41, "d", 0, 0);
        styles.fade_to_pattern(2, "gab", 1.);
        // Frames a little over 0.1 s apart, so every think is due: 'e', 'f', then "gab" from
        // frame 12 on ('g', 'a', 'b', 'g'). Letter indices: the value times 12.
        let letters: Vec<f32> = (0..6)
            .map(|tick| {
                styles.animate(1. + f64::from(tick) * 0.1005);
                styles.value(41) * 12.
            })
            .collect();
        for (got, want) in letters.iter().zip([4., 5., 6., 0., 1., 6.]) {
            assert!((got - want).abs() < 1e-4, "{letters:?}");
        }
    }
}
