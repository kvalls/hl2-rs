//! Auxiliary suit power and Episode Two's separate flashlight battery.
use super::font::PreparedText;
use super::{
    number, HudFont, NumericHudEffects, NumericPanel, NumericProperty, PanelEffects, PanelLayout,
    WeaponHud,
};
use crate::gameui::localize::Localization;
use anyhow::{Context, Result};
use glam::{vec2, Vec2};
use hl2_simulation::{flashlight::Flashlight, gameplay::Inventory};
use source_assets::keyvalues::Entry;
use source_ui::{
    canvas::{Color, MutexCell, Rect},
    font::FontFace,
};

struct Bar {
    inset: Vec2,
    width: f32,
    height: f32,
    chunk: f32,
    gap: f32,
}
impl Bar {
    fn read(entry: &Entry, battery: bool) -> Self {
        let fallback = if battery {
            [2., 18., 28., 2., 2., 2.]
        } else {
            [8., 15., 92., 4., 6., 3.]
        };
        let values = [
            "BarInsetX",
            "BarInsetY",
            "BarWidth",
            "BarHeight",
            "BarChunkWidth",
            "BarChunkGap",
        ]
        .map(|key| {
            entry
                .get(key)
                .and_then(Entry::text)
                .and_then(|s| s.parse::<f32>().ok())
        });
        let values: [f32; 6] = std::array::from_fn(|i| {
            values[i]
                .filter(|v| v.is_finite() && *v >= 0.)
                .unwrap_or(fallback[i])
        });
        Self {
            inset: vec2(values[0], values[1]),
            width: values[2],
            height: values[3],
            chunk: values[4],
            gap: values[5],
        }
    }
    fn chunks(&self, power: f32) -> (usize, usize) {
        let stride = self.chunk + self.gap;
        let count = if stride > 0. {
            (self.width / stride).floor().clamp(0., 1024.) as usize
        } else {
            0
        };
        let enabled = (count as f32 * power.clamp(0., 100.) / 100. + 0.5) as usize;
        (count, enabled)
    }
    fn draw(&self, hud: &WeaponHud, rect: Rect, power: f32, lit: Color, exhausted: Color) {
        let scale = crate::proportional::scale(hud.canvas_size());
        let (count, enabled) = self.chunks(power);
        for i in 0..count {
            hud.canvas.rectangle(
                rect.x + (self.inset.x + i as f32 * (self.chunk + self.gap)) * scale,
                rect.y + self.inset.y * scale,
                self.chunk * scale,
                self.height * scale,
                if i < enabled { lit } else { exhausted },
            );
        }
    }
}

pub(super) struct PowerHud {
    layout: PanelLayout,
    battery_layout: PanelLayout,
    bar: Bar,
    battery_bar: Bar,
    text: PreparedText,
    icons: PreparedText,
    device_pos: Vec2,
    icon_pos: Vec2,
    disabled_alpha: f32,
    state: MutexCell<PowerState>,
}
struct PowerState {
    power: f32,
    device: bool,
    low: Option<bool>,
    viewport: Vec2,
    time: f64,
    battery_initialized: bool,
}
impl Default for PowerState {
    fn default() -> Self {
        Self {
            power: -1.,
            device: false,
            low: None,
            viewport: Vec2::ZERO,
            time: 0.,
            battery_initialized: false,
        }
    }
}
impl PowerHud {
    pub(super) fn load(
        layout: &Entry,
        fonts: &Entry,
        loc: &Localization,
        face: FontFace,
        icons: FontFace,
    ) -> Result<Self> {
        let aux = layout
            .get("HudSuitPower")
            .context("HudSuitPower layout missing")?;
        let battery = layout
            .get("HudFlashlight")
            .context("HudFlashlight layout missing")?;
        Ok(Self {
            layout: PanelLayout::read(aux),
            battery_layout: PanelLayout::read(battery),
            bar: Bar::read(aux, false),
            battery_bar: Bar::read(battery, true),
            text: PreparedText::new(
                HudFont::read(
                    fonts,
                    aux.get("TextFont")
                        .and_then(Entry::text)
                        .unwrap_or("Default"),
                    face,
                    12.,
                ),
                &[
                    &loc.text("#Valve_Hud_AUX_POWER"),
                    &loc.text("#Valve_Hud_FLASHLIGHT"),
                ],
            ),
            icons: PreparedText::new(
                HudFont::read(
                    fonts,
                    battery
                        .get("font")
                        .and_then(Entry::text)
                        .unwrap_or("WeaponIconsSmall"),
                    icons,
                    16.,
                ),
                &["\u{ae}", "\u{a9}"],
            ),
            device_pos: vec2(
                number(aux, "text2_xpos", 8.),
                number(aux, "text2_ypos", 22.),
            ),
            icon_pos: vec2(
                number(battery, "icon_xpos", 4.),
                number(battery, "icon_ypos", 4.),
            ),
            disabled_alpha: number(aux, "AuxPowerDisabledAlpha", 70.).clamp(0., 255.) / 255.,
            state: MutexCell::new(PowerState::default()),
        })
    }
}

impl PowerState {
    fn observe(
        &mut self,
        hud: &WeaponHud,
        light: &Flashlight,
        time: f64,
        effects: &mut NumericHudEffects,
    ) {
        let viewport = hud.canvas_size();
        if time < self.time {
            *self = Self::default();
        }
        let start = |event: &str, effects: &mut NumericHudEffects| {
            effects.start(
                event,
                time,
                &hud.animation_rules,
                &hud.animation_events,
                &hud.settings,
            );
        };
        let resized = self.viewport != viewport;
        if resized {
            self.viewport = viewport;
            effects.panels[NumericPanel::SuitPower as usize]
                .set_rect(hud.power.layout.rect_for(viewport));
            effects.panels[NumericPanel::Flashlight as usize]
                .set_rect(hud.power.battery_layout.rect_for(viewport));
        }
        if light.battery.is_some() && !self.battery_initialized {
            start("SuitFlashlightOn", effects);
            self.battery_initialized = true;
        }
        let power = light.suit_power.power;
        let device = light.battery.is_none() && light.on;
        let (count, enabled) = hud.power.bar.chunks(power);
        for event in self
            .transitions(power, device, enabled <= count / 4, resized)
            .into_iter()
            .flatten()
        {
            start(event, effects);
        }
        self.time = time;
    }
    fn transitions(
        &mut self,
        power: f32,
        device: bool,
        low: bool,
        resized: bool,
    ) -> [Option<&'static str>; 3] {
        let charge = if power == self.power {
            None
        } else if power >= 100. && self.power < 100. {
            Some("SuitAuxPowerMax")
        } else if power < 100. && (self.power >= 100. || self.power < 0.) {
            Some("SuitAuxPowerNotMax")
        } else {
            None
        };
        let items = (device != self.device || (resized && self.power >= 0.)).then_some(if device {
            "SuitAuxPowerOneItemActive"
        } else {
            "SuitAuxPowerNoItemsActive"
        });
        let colour = if self.low != Some(low) && (device || power < 100.) {
            self.low = Some(low);
            Some(if low {
                "SuitAuxPowerDecreasedBelow25"
            } else {
                "SuitAuxPowerIncreasedAbove25"
            })
        } else {
            None
        };
        self.power = power;
        self.device = device;
        [charge, items, colour]
    }
}

impl WeaponHud {
    /// The power display is independent of the active weapon and hidden without a live suited player.
    pub fn draw_power(&self, inv: &Inventory, light: Option<&Flashlight>, time: f64) {
        let Some(light) = light.filter(|_| inv.suit && inv.health > 0. && time.is_finite()) else {
            return;
        };
        let _frame = self.framed();
        let mut effects = self.numeric_effects.borrow_mut();
        self.power
            .state
            .borrow_mut()
            .observe(self, light, time, &mut effects);
        if let Some(battery) = light.battery {
            self.draw_battery(
                battery,
                light.on,
                effects.panels[NumericPanel::Flashlight as usize],
            );
        } else {
            self.draw_auxiliary(light, effects.panels[NumericPanel::SuitPower as usize]);
        }
    }
    fn draw_auxiliary(&self, light: &Flashlight, panel: PanelEffects) {
        let tint = panel.color(NumericProperty::AuxPowerColor);
        let background = panel.color(NumericProperty::Background);
        if tint.a <= 0. && background.a <= 0. {
            return;
        }
        let scale = crate::proportional::scale(self.canvas_size());
        let layout = &self.power.layout;
        let rect = panel
            .rect()
            .unwrap_or_else(|| layout.rect_for(self.canvas_size()));
        self.rounded_box(rect, background);
        let exhausted = Color {
            a: self.power.disabled_alpha * panel.opacity(),
            ..tint
        };
        self.power
            .bar
            .draw(self, rect, light.suit_power.power, tint, exhausted);
        self.power.text.draw(
            0,
            vec2(
                rect.x + layout.text.x * scale,
                rect.y + layout.text.y * scale,
            ),
            tint,
        );
        if light.on {
            self.power.text.draw(
                1,
                vec2(
                    rect.x + self.power.device_pos.x * scale,
                    rect.y + self.power.device_pos.y * scale,
                ),
                tint,
            );
        }
    }

    fn draw_battery(&self, power: f32, on: bool, panel: PanelEffects) {
        let scale = crate::proportional::scale(self.canvas_size());
        let rect = panel
            .rect()
            .unwrap_or_else(|| self.power.battery_layout.rect_for(self.canvas_size()));
        self.rounded_box(rect, panel.color(NumericProperty::Background));
        let (count, enabled) = self.power.battery_bar.chunks(power);
        let mut tint = if enabled < count / 4 {
            self.quick_info.caution
        } else {
            self.quick_info.normal
        };
        tint.a = if on { 1. } else { 32. / 255. };
        self.power.icons.draw(
            usize::from(on),
            vec2(
                rect.x + self.power.icon_pos.x * scale,
                rect.y + self.power.icon_pos.y * scale,
            ),
            tint,
        );
        if on || enabled != count {
            let exhausted = Color {
                a: (if on { 31. } else { 4. }) / 255.,
                ..tint
            };
            self.power
                .battery_bar
                .draw(self, rect, power, tint, exhausted);
        }
    }
    /// Diagnostic state for packaged checks; no per-frame serialization.
    pub fn power_report(&self) -> serde_json::Value {
        let state = self.power.state.borrow();
        let effects = self.numeric_effects.borrow();
        let panel = effects.panels[NumericPanel::SuitPower as usize];
        serde_json::json!({"suit_power":state.power,"device":state.device,"low":state.low,
            "color":panel.color(NumericProperty::AuxPowerColor).to_array(),
            "rect":panel.rect().map(|r| [r.x,r.y,r.w,r.h]),"battery_initialized":state.battery_initialized})
    }
}

#[cfg(test)]
mod tests;
