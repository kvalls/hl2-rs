//! The HUD's panel animations: scripts/HudAnimations.txt commands as VGUI's
//! AnimationController runs them (events, delayed RunEvent/StopEvent messages and the
//! interpolated panel properties), and the events the numeric panels start.
use super::{alpha, color, panel_axis, rgba, uses_secondary_ammo, PanelLayout, Result};
use glam::Vec2;
use hl2_simulation::gameplay::{Inventory, Weapon};
use source_assets::keyvalues::{self, Entry};
use source_ui::canvas::{Color, Rect};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone)]
pub(super) struct Animate {
    pub(super) event: String,
    pub(super) panel: String,
    pub(super) property: String,
    pub(super) target: String,
    pub(super) delay: f64,
    pub(super) duration: f64,
    pub(super) interpolation: Interpolation,
}

#[derive(Clone, Copy)]
pub(super) enum Interpolation {
    Linear,
    Accel,
    Deaccel,
    Spline,
}
impl Interpolation {
    pub(super) fn read(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "linear" => Some(Self::Linear),
            "accel" => Some(Self::Accel),
            "deaccel" => Some(Self::Deaccel),
            "spline" => Some(Self::Spline),
            _ => None,
        }
    }
    pub(super) fn sample(self, position: f32) -> f32 {
        let position = position.clamp(0., 1.);
        // VGUI AnimationController::GetInterpolatedValue, including its square-root Deaccel.
        match self {
            Self::Linear => position,
            Self::Accel => position * position,
            Self::Deaccel => position.sqrt(),
            Self::Spline => position * position * (3. - 2. * position),
        }
    }
}

#[derive(Clone)]
pub(super) struct EventRule {
    pub(super) event: String,
    pub(super) target: String,
    pub(super) delay: f64,
    pub(super) run: bool,
    pub(super) panel: bool,
    pub(super) property: Option<String>,
}

pub(super) fn animation_lines(source: &str) -> Result<Vec<(String, Vec<String>)>> {
    let mut event = String::new();
    let mut event_enabled = true;
    let mut lines = Vec::new();
    for line in source.lines() {
        let mut tokens = keyvalues::tokens(line)?;
        let enabled = if tokens
            .last()
            .is_some_and(|t| t.starts_with('[') && t.ends_with(']'))
        {
            keyvalues::resource_condition(&tokens.pop().unwrap())?
        } else {
            true
        };
        if tokens.first().map(String::as_str) == Some("event") && tokens.len() >= 2 {
            event = tokens[1].clone();
            event_enabled = enabled;
        } else if enabled && event_enabled && !tokens.is_empty() {
            lines.push((event.clone(), tokens));
        }
    }
    Ok(lines)
}

pub(super) fn animation<'a>(
    rules: &'a [Animate],
    event: &str,
    panel: &str,
    property: &str,
) -> Option<&'a Animate> {
    rules.iter().find(|a| {
        a.event.eq_ignore_ascii_case(event)
            && a.panel.eq_ignore_ascii_case(panel)
            && a.property.eq_ignore_ascii_case(property)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NumericPanel {
    Health,
    Suit,
    Ammo,
    AmmoSecondary,
    DamageIndicator,
    SuitPower,
    Flashlight,
    PoisonDamageIndicator,
    HintKeyDisplay,
}
impl NumericPanel {
    pub(super) fn read(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "hudhealth" => Some(Self::Health),
            "hudsuit" => Some(Self::Suit),
            "hudammo" => Some(Self::Ammo),
            "hudammosecondary" => Some(Self::AmmoSecondary),
            "huddamageindicator" => Some(Self::DamageIndicator),
            "hudsuitpower" => Some(Self::SuitPower),
            "hudflashlight" => Some(Self::Flashlight),
            "hudpoisondamageindicator" => Some(Self::PoisonDamageIndicator),
            "hudhintkeydisplay" => Some(Self::HintKeyDisplay),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NumericProperty {
    Alpha,
    Blur,
    Background,
    Foreground,
    TextColor,
    Ammo2Color,
    Position,
    Size,
    DmgColorLeft,
    DmgColorRight,
    DmgHighColorLeft,
    DmgHighColorRight,
    DmgFullscreenColor,
    AuxPowerColor,
}
impl NumericProperty {
    pub(super) fn read(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "alpha" => Some(Self::Alpha),
            "blur" => Some(Self::Blur),
            "bgcolor" => Some(Self::Background),
            "fgcolor" => Some(Self::Foreground),
            "textcolor" => Some(Self::TextColor),
            "ammo2color" => Some(Self::Ammo2Color),
            "position" => Some(Self::Position),
            "size" => Some(Self::Size),
            "dmgcolorleft" => Some(Self::DmgColorLeft),
            "dmgcolorright" => Some(Self::DmgColorRight),
            "dmghighcolorleft" => Some(Self::DmgHighColorLeft),
            "dmghighcolorright" => Some(Self::DmgHighColorRight),
            "dmgfullscreencolor" => Some(Self::DmgFullscreenColor),
            "auxpowercolor" => Some(Self::AuxPowerColor),
            _ => None,
        }
    }
    pub(super) fn is_color(self) -> bool {
        matches!(
            self,
            Self::Background
                | Self::Foreground
                | Self::TextColor
                | Self::Ammo2Color
                | Self::DmgColorLeft
                | Self::DmgColorRight
                | Self::DmgHighColorLeft
                | Self::DmgHighColorRight
                | Self::DmgFullscreenColor
                | Self::AuxPowerColor
        )
    }
    pub(super) fn geometry(self) -> bool {
        matches!(self, Self::Position | Self::Size)
    }
    pub(super) fn target(self, value: &str, settings: &Entry, viewport: Vec2) -> Option<[f32; 4]> {
        if self.geometry() {
            let parts = value.split_whitespace().collect::<Vec<_>>();
            if parts.len() != 2 || viewport.x <= 0. || viewport.y <= 0. {
                return None;
            }
            let scale = crate::proportional::scale(viewport);
            let y_scale = if self == Self::Position {
                crate::proportional::y_scale(viewport)
            } else {
                scale
            };
            let extent = if self == Self::Position {
                viewport
            } else {
                Vec2::ZERO
            };
            let x = panel_axis(parts[0], extent.x, scale)?;
            let y = panel_axis(parts[1], extent.y, y_scale)?;
            if self == Self::Size && (x < 0. || y < 0.) {
                return None;
            }
            Some([x, y, 0., 0.])
        } else if self.is_color() {
            let value =
                rgba(value).or_else(|| settings.get(value).and_then(Entry::text).and_then(rgba))?;
            Some([
                value.r * 255.,
                value.g * 255.,
                value.b * 255.,
                value.a * 255.,
            ])
        } else {
            let value = value.parse::<f32>().ok()?;
            value.is_finite().then_some([value, 0., 0., 0.])
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct PanelEffects {
    pub(super) values: [[f32; 4]; 14],
}
impl PanelEffects {
    pub(super) fn new(settings: &Entry) -> Self {
        let foreground = color(settings, "FgColor", Color::from_rgba(255, 220, 0, 100));
        let background = color(settings, "BgColor", Color::from_rgba(0, 0, 0, 76));
        let components = |c: Color| [c.r * 255., c.g * 255., c.b * 255., c.a * 255.];
        Self {
            values: [
                [255., 0., 0., 0.],
                [0.; 4],
                components(background),
                components(foreground),
                components(foreground),
                [0.; 4],
                [0.; 4],
                components(foreground),
                // CHudDamageIndicator's CPanelAnimationVar defaults ("255 0 0 0").
                [255., 0., 0., 0.],
                [255., 0., 0., 0.],
                [255., 0., 0., 0.],
                [255., 0., 0., 0.],
                [255., 0., 0., 0.],
                [255., 0., 0., 255.],
            ],
        }
    }
    pub(super) fn opacity(self) -> f32 {
        self.values[NumericProperty::Alpha as usize][0].clamp(0., 255.) / 255.
    }
    pub(super) fn color(self, property: NumericProperty) -> Color {
        // Native VGUI stores interpolated color channels as bytes.
        let channels = self.values[property as usize].map(|v| v.clamp(0., 255.) as u8);
        alpha(
            Color::from_rgba(channels[0], channels[1], channels[2], channels[3]),
            self.opacity(),
        )
    }
    pub(super) fn glow(self) -> impl Iterator<Item = f32> {
        let blur = self.values[NumericProperty::Blur as usize][0].clamp(0., 64.);
        (0..blur.ceil() as usize).map(move |pass| (blur - pass as f32).min(1.))
    }
    pub(super) fn set_rect(&mut self, rect: Rect) {
        self.values[NumericProperty::Position as usize] = [rect.x, rect.y, 0., 0.];
        self.values[NumericProperty::Size as usize] = [rect.w, rect.h, 0., 0.];
    }
    pub(super) fn rect(self) -> Option<Rect> {
        let position = self.values[NumericProperty::Position as usize];
        let size = self.values[NumericProperty::Size as usize];
        (size[0] > 0. && size[1] > 0.).then(|| {
            // VGUI applies animated geometry through integer panel coordinates.
            Rect::new(
                position[0].trunc(),
                position[1].trunc(),
                size[0].trunc(),
                size[1].trunc(),
            )
        })
    }
}
pub(super) struct PropertyAnimation {
    pub(super) event: String,
    pub(super) panel: NumericPanel,
    pub(super) property: NumericProperty,
    pub(super) target: [f32; 4],
    pub(super) from: Option<[f32; 4]>,
    pub(super) start: f64,
    pub(super) end: f64,
    pub(super) interpolation: Interpolation,
}
pub(super) struct PostedEvent {
    pub(super) event: String,
    pub(super) target: String,
    pub(super) at: f64,
    pub(super) run: bool,
    pub(super) panel: bool,
    pub(super) property: Option<String>,
}
pub(super) struct NumericHudEffects {
    pub(super) panels: [PanelEffects; 9],
    /// The local player's m_bPoisoned as last reported, and as the indicator shows it
    /// (CHudPoisonDamageIndicator::m_bDamageIndicatorVisible).
    pub(super) poisoned: bool,
    pub(super) poison_shown: bool,
    /// The indicator's layout TextColor (its animations start from it).
    pub(super) poison_text: [f32; 4],
    pub(super) animations: Vec<PropertyAnimation>,
    pub(super) posted: Vec<PostedEvent>,
    pub(super) health: i32,
    pub(super) armor: i32,
    pub(super) active: String,
    pub(super) ammo: i32,
    pub(super) reserve: i32,
    pub(super) secondary_active: String,
    pub(super) secondary_ammo: i32,
    pub(super) time: f64,
    pub(super) viewport: Vec2,
    pub(super) initial_rects: Option<[Rect; 4]>,
}
impl NumericHudEffects {
    pub(super) fn new(settings: &Entry) -> Self {
        let mut panels = [PanelEffects::new(settings); 9];
        // CHudHintKeyDisplay starts invisible (SetAlpha(0)).
        panels[NumericPanel::HintKeyDisplay as usize].values[NumericProperty::Alpha as usize][0] =
            0.;
        // CHudSecondaryAmmo::Reset starts hidden until a secondary-ammo weapon is observed.
        panels[NumericPanel::AmmoSecondary as usize].values[NumericProperty::Alpha as usize][0] =
            0.;
        // CHudPoisonDamageIndicator::Reset: SetAlpha(0).
        panels[NumericPanel::PoisonDamageIndicator as usize].values
            [NumericProperty::Alpha as usize][0] = 0.;
        Self {
            panels,
            poisoned: false,
            poison_shown: false,
            poison_text: [255., 170., 0., 220.],
            animations: Vec::new(),
            posted: Vec::new(),
            health: -1,
            armor: -1,
            active: String::new(),
            ammo: -1,
            reserve: -1,
            secondary_active: String::new(),
            secondary_ammo: -1,
            time: 0.,
            viewport: Vec2::ZERO,
            initial_rects: None,
        }
    }
    /// The indicator's layout TextColor (kept across resets).
    pub(super) fn with_poison_text(mut self, text: [f32; 4]) -> Self {
        self.poison_text = text;
        self.panels[NumericPanel::PoisonDamageIndicator as usize].values
            [NumericProperty::TextColor as usize] = text;
        self
    }
    pub(super) fn stop(&mut self, event: &str) {
        self.animations
            .retain(|a| !a.event.eq_ignore_ascii_case(event));
        self.posted.retain(|s| !s.event.eq_ignore_ascii_case(event));
    }
    pub(super) fn stop_panel(&mut self, panel: &str) {
        if let Some(panel) = NumericPanel::read(panel) {
            self.animations.retain(|a| a.panel != panel);
        }
    }
    pub(super) fn stop_property(&mut self, panel: &str, property: &str) {
        if let (Some(panel), Some(property)) =
            (NumericPanel::read(panel), NumericProperty::read(property))
        {
            self.animations
                .retain(|a| a.panel != panel || a.property != property);
        }
    }
    pub(super) fn configure_layouts(&mut self, layouts: [&PanelLayout; 4], viewport: Vec2) {
        if self.viewport == viewport && self.initial_rects.is_some() {
            return;
        }
        self.viewport = viewport;
        let rects = layouts.map(|layout| layout.rect_for(viewport));
        self.initial_rects = Some(rects);
        for (panel, rect) in self.panels.iter_mut().zip(rects) {
            panel.set_rect(rect);
        }
        // A viewport change reapplies proportional resource geometry. Old pixel targets
        // no longer apply; current weapon sequences will supply freshly scaled targets.
        self.animations.retain(|a| !a.property.geometry());
        self.active.clear();
        self.secondary_active.clear();
    }
    pub(super) fn start(
        &mut self,
        event: &str,
        time: f64,
        rules: &[Animate],
        events: &[EventRule],
        settings: &Entry,
    ) {
        // StartAnimationSequence removes earlier commands from the same sequence.
        self.stop(event);
        for rule in events
            .iter()
            .filter(|s| s.event.eq_ignore_ascii_case(event))
        {
            if let Some(property) = &rule.property {
                if rule.delay == 0. {
                    self.stop_property(&rule.target, property);
                    continue;
                }
            }
            if rule.panel && rule.delay == 0. {
                // The owned secondary-weapon sequences stop existing panel tracks
                // before scheduling their replacement color/alpha tracks.
                self.stop_panel(&rule.target);
                continue;
            }
            self.posted.push(PostedEvent {
                event: event.into(),
                target: rule.target.clone(),
                at: time + rule.delay,
                run: rule.run,
                panel: rule.panel,
                property: rule.property.clone(),
            });
        }
        for rule in rules.iter().filter(|r| r.event.eq_ignore_ascii_case(event)) {
            let Some(panel) = NumericPanel::read(&rule.panel) else {
                continue;
            };
            let Some(property) = NumericProperty::read(&rule.property) else {
                continue;
            };
            let Some(target) = property.target(&rule.target, settings, self.viewport) else {
                continue;
            };
            self.animations.push(PropertyAnimation {
                event: event.into(),
                panel,
                property,
                target,
                from: None,
                start: time + rule.delay,
                end: time + rule.delay + rule.duration,
                interpolation: rule.interpolation,
            });
        }
    }
    pub(super) fn posted_events(
        &mut self,
        time: f64,
        rules: &[Animate],
        events: &[EventRule],
        settings: &Entry,
    ) {
        // AnimationController processes posted messages in insertion order, restarting
        // its traversal after each dispatch. Each RunEvent target may run once per frame.
        let mut ran = HashSet::new();
        while let Some(index) = self.posted.iter().position(|message| message.at <= time) {
            let message = self.posted.remove(index);
            if let Some(property) = message.property {
                self.stop_property(&message.target, &property);
            } else if message.panel {
                self.stop_panel(&message.target);
            } else if message.run {
                if ran.insert(message.target.to_ascii_lowercase()) {
                    // Source starts late events at the current frame, without backlog catch-up.
                    self.start(&message.target, time, rules, events, settings);
                }
            } else {
                self.stop(&message.target);
            }
        }
    }
    pub(super) fn advance(&mut self, time: f64) {
        let mut index = 0;
        while index < self.animations.len() {
            let track = &mut self.animations[index];
            if time < track.start {
                index += 1;
                continue;
            }
            let value = &mut self.panels[track.panel as usize].values[track.property as usize];
            // Delayed commands capture the current property when they first become active.
            let from = *track.from.get_or_insert(*value);
            let position = if time >= track.end {
                1.
            } else {
                ((time - track.start) / (track.end - track.start)) as f32
            };
            let fraction = track.interpolation.sample(position);
            *value = std::array::from_fn(|channel| {
                from[channel] + (track.target[channel] - from[channel]) * fraction
            });
            if time >= track.end {
                self.animations.remove(index);
            } else {
                index += 1;
            }
        }
        self.time = time;
    }
    pub(super) fn observe(
        &mut self,
        inv: &Inventory,
        weapons: &BTreeMap<String, Weapon>,
        time: f64,
        rules: &[Animate],
        events: &[EventRule],
        settings: &Entry,
    ) {
        if !time.is_finite() {
            return;
        }
        if time < self.time {
            let viewport = self.viewport;
            let rects = self.initial_rects;
            let (poison_text, poisoned) = (self.poison_text, self.poisoned);
            *self = Self::new(settings).with_poison_text(poison_text);
            self.poisoned = poisoned;
            self.viewport = viewport;
            self.initial_rects = rects;
            if let Some(rects) = rects {
                for (panel, rect) in self.panels.iter_mut().zip(rects) {
                    panel.set_rect(rect);
                }
            }
        }
        self.advance(time);
        // CHudPoisonDamageIndicator::OnThink: PoisonDamageTaken / PoisonDamageCured when
        // the player's m_bPoisoned changes.
        if self.poisoned != self.poison_shown {
            self.poison_shown = self.poisoned;
            let event = if self.poisoned {
                "PoisonDamageTaken"
            } else {
                "PoisonDamageCured"
            };
            self.start(event, time, rules, events, settings);
        }
        self.observe_vitals(inv, time, rules, events, settings);
        self.observe_ammo(inv, weapons, time, rules, events, settings);
        if let Some(weapon) = weapons.get(&inv.active) {
            let uses_secondary = uses_secondary_ammo(weapon);
            if uses_secondary {
                let ammo = inv.secondary_for(&inv.active, weapons);
                if ammo != self.secondary_ammo {
                    let event = if ammo == 0 {
                        "AmmoSecondaryEmpty"
                    } else if ammo < self.secondary_ammo {
                        "AmmoSecondaryDecreased"
                    } else {
                        "AmmoSecondaryIncreased"
                    };
                    self.start(event, time, rules, events, settings);
                    self.secondary_ammo = ammo;
                }
            }
            if self.secondary_active != inv.active {
                self.start(
                    if uses_secondary {
                        "WeaponUsesSecondaryAmmo"
                    } else {
                        "WeaponDoesNotUseSecondaryAmmo"
                    },
                    time,
                    rules,
                    events,
                    settings,
                );
                self.secondary_active.clone_from(&inv.active);
            }
        } else {
            self.secondary_active.clear();
        }
        self.posted_events(time, rules, events, settings);
        self.advance(time);
    }
    /// CHudAmmo: the primary ammunition events as the clip, reserve or weapon changes.
    fn observe_ammo(
        &mut self,
        inv: &Inventory,
        weapons: &BTreeMap<String, Weapon>,
        time: f64,
        rules: &[Animate],
        events: &[EventRule],
        settings: &Entry,
    ) {
        if let Some(weapon) = weapons
            .get(&inv.active)
            .filter(|w| !w.ammo_type.is_empty() && !w.ammo_type.eq_ignore_ascii_case("none"))
        {
            let clip = inv.owned.get(&inv.active).copied().unwrap_or(-1);
            let reserve = inv.reserve_for(&inv.active, weapons);
            let ammo = if clip < 0 { reserve } else { clip };
            let ammo2 = if clip < 0 { 0 } else { reserve };
            if ammo != self.ammo {
                let event = if ammo == 0 {
                    "AmmoEmpty"
                } else if ammo < self.ammo {
                    "AmmoDecreased"
                } else {
                    "AmmoIncreased"
                };
                self.start(event, time, rules, events, settings);
                self.ammo = ammo;
            }
            if ammo2 != self.reserve {
                let event = if ammo2 == 0 {
                    "Ammo2Empty"
                } else if ammo2 < self.reserve {
                    "Ammo2Decreased"
                } else {
                    "Ammo2Increased"
                };
                self.start(event, time, rules, events, settings);
                self.reserve = ammo2;
            }
            if self.active != inv.active {
                // Retail/SDK SetAmmo's playAnimation argument is unused; WeaponChanged follows.
                self.start(
                    if clip < 0 {
                        "WeaponDoesNotUseClips"
                    } else {
                        "WeaponUsesClips"
                    },
                    time,
                    rules,
                    events,
                    settings,
                );
                self.start("WeaponChanged", time, rules, events, settings);
                self.active.clone_from(&inv.active);
            }
            let _ = weapon;
        }
    }
}
