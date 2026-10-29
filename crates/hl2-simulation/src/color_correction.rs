//! `color_correction` and `color_correction_volume`: the weights of the lookup volumes the
//! engine post pass applies (Source SDK `game/server/colorcorrection.cpp`,
//! `game/server/colorcorrectionvolume.cpp`, `game/client/c_colorcorrection.cpp`,
//! `game/client/c_colorcorrectionvolume.cpp`). The server half is the fade state of each
//! entity, the client half the falloff from the player and the per-lookup weights; the host
//! draws the lookups named by [`ColorCorrection::lookups`] with the [`Mix`].

use glam::Vec3;
use modkit_core::World;

/// The post pass binds at most four lookups (Engine_Post `COL_CORRECT_NUM_LOOKUPS`).
pub const MAX_LOOKUPS: usize = 4;
/// Server tick interval, the think rate of the fades (`TICK_INTERVAL`).
const TICK: f64 = 0.015;

/// The weights of the post pass: `rgb * default + sum(lookup[i](rgb) * weights[i])`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mix {
    pub default: f32,
    pub weights: [f32; MAX_LOOKUPS],
}
impl Mix {
    /// Retail `CColorCorrectionSystem` weights: the first four lookups are summed; up to 1 the
    /// rest is the default weight, above 1 the default is 0 and the weights are normalized.
    fn of(weights: [f32; MAX_LOOKUPS]) -> Self {
        let sum: f32 = weights.iter().sum();
        if sum <= 1. {
            Self {
                default: 1. - sum,
                weights,
            }
        } else {
            Self {
                default: 0.,
                weights: weights.map(|w| w / sum),
            }
        }
    }
    /// Whether any lookup contributes (otherwise the pass would return its input).
    pub fn is_active(&self) -> bool {
        self.weights.iter().any(|&w| w > 0.)
    }
}

/// The map's color correction: one lookup per distinct file, one source per entity.
#[derive(Clone, Debug, Default)]
pub struct ColorCorrection {
    /// Lookup files in registration order, lowercase.
    lookups: Vec<String>,
    sources: Vec<Source>,
    mix: Mix,
}

#[derive(Clone, Debug)]
struct Source {
    entity: usize,
    /// The lookup this entity drives; `None` for a file an earlier entity registered
    /// (retail refuses a second lookup of one name, so the entity drives nothing).
    lookup: Option<usize>,
    kind: Kind,
    /// The weight this entity sent to its lookup at the latest update.
    weight: f32,
}

#[derive(Clone, Debug)]
enum Kind {
    Point(Point),
    Volume(Volume),
}

/// `CColorCorrection` (server) with the client's falloff.
#[derive(Clone, Debug)]
struct Point {
    origin: Vec3,
    min_falloff: f32,
    max_falloff: f32,
    max_weight: f32,
    fade_in: f32,
    fade_out: f32,
    enabled: bool,
    current: f32,
    start_in: f32,
    start_out: f32,
    time_in: f64,
    time_out: f64,
    next_in: Option<f64>,
    next_out: Option<f64>,
}

/// `CColorCorrectionVolume`.
#[derive(Clone, Debug)]
struct Volume {
    enabled: bool,
    fade: f32,
    weight: f32,
    inside: bool,
    enter: (f64, f32),
    exit: (f64, f32),
}

fn number(e: &modkit_core::Entity, key: &str, default: f32) -> f32 {
    e.get(key)
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

impl ColorCorrection {
    /// Registers the map's entities in entity order.
    pub fn new(world: &World) -> Self {
        let mut this = Self::default();
        for (entity, e) in world.entities.iter().enumerate() {
            let volume = match e.class() {
                "color_correction" => false,
                "color_correction_volume" => true,
                _ => continue,
            };
            let file = e
                .get("filename")
                .unwrap_or("")
                .trim()
                .replace('\\', "/")
                .to_lowercase();
            if file.is_empty() {
                continue;
            }
            let disabled = number(e, "StartDisabled", 0.) != 0.;
            let kind = if volume {
                Kind::Volume(Volume {
                    enabled: !disabled,
                    fade: number(e, "fadeDuration", 0.),
                    weight: 0.,
                    inside: false,
                    enter: (0., 0.),
                    exit: (0., 0.),
                })
            } else {
                Kind::Point(Point {
                    origin: e.origin(),
                    min_falloff: number(e, "minfalloff", 0.),
                    max_falloff: number(e, "maxfalloff", 1000.),
                    max_weight: number(e, "maxweight", 1.),
                    fade_in: number(e, "fadeInDuration", 0.),
                    fade_out: number(e, "fadeOutDuration", 0.),
                    // Spawn: weight 1 (not maxweight) unless StartDisabled.
                    enabled: !disabled,
                    current: if disabled { 0. } else { 1. },
                    start_in: 0.,
                    start_out: 0.,
                    time_in: 0.,
                    time_out: 0.,
                    next_in: None,
                    next_out: None,
                })
            };
            let lookup = match this.lookups.iter().position(|l| *l == file) {
                Some(_) => None,
                None => {
                    this.lookups.push(file);
                    Some(this.lookups.len() - 1)
                }
            };
            this.sources.push(Source {
                entity,
                lookup,
                kind,
                weight: 0.,
            });
        }
        this.mix = Mix::of([0.; MAX_LOOKUPS]);
        this
    }

    /// The lookup files the post pass binds, in the order of the weights of [`Mix`].
    pub fn lookups(&self) -> &[String] {
        &self.lookups[..self.lookups.len().min(MAX_LOOKUPS)]
    }

    /// Whether the map has any color correction entity.
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// The weights of the latest update.
    pub fn mix(&self) -> Mix {
        self.mix
    }

    /// Enable, Disable, SetFadeInDuration and SetFadeOutDuration of one entity; `false` when the
    /// entity is not a color correction entity or does not take the input.
    pub fn input(&mut self, entity: usize, input: &str, value: f32, now: f64) -> bool {
        let Some(source) = self.sources.iter_mut().find(|s| s.entity == entity) else {
            return false;
        };
        match (&mut source.kind, input) {
            (Kind::Point(p), "enable") => p.enable(now),
            (Kind::Point(p), "disable") => p.disable(now),
            (Kind::Point(p), "setfadeinduration") => p.fade_in = value,
            (Kind::Point(p), "setfadeoutduration") => p.fade_out = value,
            (Kind::Volume(v), "enable") => v.enabled = true,
            (Kind::Volume(v), "disable") => v.enabled = false,
            _ => return false,
        }
        true
    }

    /// One server tick followed by the client's weights for a player at `feet`. `inside` tells
    /// whether the player touches a volume entity.
    pub fn update(&mut self, now: f64, feet: Vec3, inside: impl Fn(usize) -> bool) {
        for source in &mut self.sources {
            source.weight = match &mut source.kind {
                Kind::Point(p) => {
                    p.think(now);
                    p.client_weight(feet)
                }
                Kind::Volume(v) => {
                    v.touch(now, inside(source.entity));
                    v.think(now);
                    v.weight
                }
            };
        }
        // The weights of the bound lookups only: later ones never reach the pass.
        let mut weights = [0.; MAX_LOOKUPS];
        for source in &self.sources {
            if let Some(slot) = source.lookup.and_then(|l| weights.get_mut(l)) {
                *slot = source.weight;
            }
        }
        self.mix = Mix::of(weights);
    }
}

fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + (b - a) * t
}

impl Point {
    fn enable(&mut self, now: f64) {
        self.enabled = true;
        if self.fade_in > 0. {
            self.time_in = now;
            self.start_in = self.current;
            self.next_in = Some(now + TICK);
        } else {
            self.current = self.max_weight;
        }
    }

    fn disable(&mut self, now: f64) {
        self.enabled = false;
        if self.fade_out > 0. {
            self.time_out = now;
            self.start_out = self.current;
            self.next_out = Some(now + TICK);
        } else {
            self.current = 0.;
        }
    }

    fn think(&mut self, now: f64) {
        if self.next_in.is_some_and(|t| t <= now) {
            self.fade_in_think(now);
        }
        if self.next_out.is_some_and(|t| t <= now) {
            self.fade_out_think(now);
        }
    }

    /// `FadeInThink`: the weight moves from where the fade started to `maxweight`.
    fn fade_in_think(&mut self, now: f64) {
        let max = self.max_weight;
        if self.fade_in <= 0.
            || self.current >= max
            || !self.enabled
            || max == 0.
            || self.start_in >= max
        {
            self.next_in = None;
            return;
        }
        // A fade that starts from a partial weight takes a truncated time.
        let mut span = self.fade_in;
        if self.start_in > 0. {
            span = self.fade_in * (1. - (self.start_in / max).clamp(0., 0.99));
        }
        let ratio = (((now - self.time_in) as f32) / span).clamp(0., 1.);
        self.start_in = self.start_in.clamp(0., 1.);
        self.current = lerp(ratio, self.start_in, max);
        self.next_in = Some(now + TICK);
    }

    /// `FadeOutThink`: the weight moves from where the fade started to 0.
    fn fade_out_think(&mut self, now: f64) {
        let max = self.max_weight;
        if self.fade_out <= 0.
            || self.current <= 0.
            || self.enabled
            || max == 0.
            || self.start_out <= 0.
        {
            self.next_out = None;
            return;
        }
        let mut span = self.fade_out;
        if self.start_out < max {
            span = self.fade_out * (self.start_out / max).clamp(0.01, 1.);
        }
        let ratio = (((now - self.time_out) as f32) / span).clamp(0., 1.);
        self.start_out = self.start_out.clamp(0., 1.);
        self.current = lerp(1. - ratio, 0., self.start_out);
        self.next_out = Some(now + TICK);
    }

    /// The client think: the weight scaled by the falloff from the player.
    fn client_weight(&self, feet: Vec3) -> f32 {
        let mut falloff = 0.;
        if self.min_falloff != -1.
            && self.max_falloff != -1.
            && self.min_falloff != self.max_falloff
        {
            let distance = (feet - self.origin).length();
            falloff = ((distance - self.min_falloff) / (self.max_falloff - self.min_falloff))
                .clamp(0., 1.);
        }
        self.current * (1. - falloff)
    }
}

impl Volume {
    /// `StartTouch` / `EndTouch` for the local player.
    fn touch(&mut self, now: f64, inside: bool) {
        if inside && !self.inside {
            self.enter = (now, self.weight);
        } else if !inside && self.inside {
            self.exit = (now, self.weight);
        }
        self.inside = inside;
    }

    /// `ThinkFunc`. The SDK divides by the fade time; a zero one is an instant change.
    fn think(&mut self, now: f64) {
        if !self.enabled {
            self.weight = 0.;
        } else if self.enter.0 > self.exit.0 {
            if self.weight < 1. {
                let (time, from) = self.enter;
                let denominator = (1. - from) * self.fade;
                self.weight = if denominator > 0. {
                    (from + (now - time) as f32 / denominator).min(1.)
                } else {
                    1.
                };
            }
        } else if self.weight > 0. {
            let (time, from) = self.exit;
            let denominator = from * self.fade;
            self.weight = if denominator > 0. {
                1. - ((1. - from) + (now - time) as f32 / denominator).min(1.)
            } else {
                0.
            };
        }
    }
}

#[cfg(test)]
mod tests;
