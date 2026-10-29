//! The HEV flashlight: `impulse 100` toggling it, its power (Source SDK `game/server/hl2/hl2_player.cpp`:
//! the legacy flashlight is a suit power device of HL2, Episode One and Lost Coast; Episode Two's has its own
//! battery) and where its light is each frame (`game/client/flashlighteffect.cpp` `UpdateLightNew`). The host
//! draws the light and plays the sounds this module asks for.

mod controller;
pub use controller::PlayerFlashlight;

use glam::Vec3;
use serde::Serialize;

/// Suit power device bit of the flashlight (`bits_SUIT_DEVICE_FLASHLIGHT`).
const FLASHLIGHT_DEVICE: u32 = 2;
/// Suit power drain of the flashlight, percent per second: HL2's, and with `hl2_episodic`.
const DRAIN: f32 = 2.222;
const EPISODIC_DRAIN: f32 = 1.111;
/// Suit power recharge, percent per second, and the device-free time before it starts.
const CHARGE_RATE: f32 = 12.5;
const RECHARGE_DELAY: f64 = 0.5;
/// The legacy flashlight turns off a little below one bar notch (5 %).
const LEGACY_CUTOFF: f32 = 4.8;
/// Episode Two's battery, percent per second.
const BATTERY_DRAIN: f32 = 1.1111;
const BATTERY_CHARGE: f32 = 50.;

pub const SOUND_ON: &str = "HL2Player.FlashLightOn";
pub const SOUND_OFF: &str = "HL2Player.FlashLightOff";

/// The HEV suit's auxiliary power (`SuitPower_*`), with the devices the rewrite has (the flashlight).
#[derive(Clone, Debug, Serialize)]
pub struct SuitPower {
    /// 0..100.
    pub power: f32,
    devices: u32,
    load: f32,
    all_off_at: f64,
}

impl Default for SuitPower {
    fn default() -> Self {
        Self {
            power: 100.,
            devices: 0,
            load: 0.,
            all_off_at: 0.,
        }
    }
}

impl SuitPower {
    fn add(&mut self, device: u32, rate: f32) -> bool {
        if self.devices & device != 0 {
            return false;
        }
        self.devices |= device;
        self.load += rate;
        true
    }

    /// Removing a device costs a tenth of its drain rate (the engine's defence against tapping).
    fn remove(&mut self, device: u32, rate: f32, now: f64) -> bool {
        if self.devices & device == 0 {
            return false;
        }
        self.drain(rate * 0.1);
        self.devices &= !device;
        self.load -= rate;
        if self.devices == 0 {
            self.all_off_at = now;
        }
        true
    }

    /// False when the power ran out.
    fn drain(&mut self, amount: f32) -> bool {
        self.power -= amount;
        if self.power < 0.01 {
            self.power = 0.;
            return false;
        }
        true
    }

    /// One frame: recharge when no device has run for half a second, else drain the load; false when
    /// it ran out.
    fn update(&mut self, dt: f32, now: f64) -> bool {
        if self.devices == 0 {
            if self.power < 100. && now >= self.all_off_at + RECHARGE_DELAY {
                self.power = (self.power + CHARGE_RATE * dt).min(100.);
            }
            return true;
        }
        self.drain(self.load * dt)
    }

    pub fn active(&self) -> bool {
        self.devices != 0
    }
}

/// What the flashlight's rules depend on.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    /// The player wears the suit.
    pub suit: bool,
    /// `hl2_episodic` (Episode One and Two).
    pub episodic: bool,
    /// Skill level 1.
    pub easy: bool,
    pub now: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Flashlight {
    pub on: bool,
    /// `DisableFlashlight` input.
    pub disabled: bool,
    /// Episode Two's own battery (0..100); `None`: the legacy suit power device.
    pub battery: Option<f32>,
    pub suit_power: SuitPower,
    /// Sounds to play (soundscript names), drained by the host.
    #[serde(skip)]
    pub sounds: Vec<&'static str>,
}

impl Flashlight {
    /// A flashlight on suit power (`legacy`) or with its own battery.
    pub fn new(legacy: bool) -> Self {
        Self {
            on: false,
            disabled: false,
            battery: (!legacy).then_some(100.),
            suit_power: SuitPower::default(),
            sounds: Vec::new(),
        }
    }

    fn drain_rate(context: Context) -> f32 {
        match (context.episodic, context.easy) {
            (true, true) => EPISODIC_DRAIN * 0.5,
            (true, false) => EPISODIC_DRAIN,
            _ => DRAIN,
        }
    }

    /// `impulse 100`.
    pub fn toggle(&mut self, context: Context) {
        if self.on {
            self.turn_off(context);
        } else {
            self.turn_on(context);
        }
    }

    pub fn turn_on(&mut self, context: Context) {
        if self.disabled || !context.suit {
            return;
        }
        if self.battery.is_none()
            && !self
                .suit_power
                .add(FLASHLIGHT_DEVICE, Self::drain_rate(context))
        {
            return;
        }
        self.on = true;
        self.sounds.push(SOUND_ON);
    }

    pub fn turn_off(&mut self, context: Context) {
        if self.battery.is_none()
            && !self
                .suit_power
                .remove(FLASHLIGHT_DEVICE, Self::drain_rate(context), context.now)
        {
            return;
        }
        if std::mem::take(&mut self.on) {
            self.sounds.push(SOUND_OFF);
        }
    }

    /// `DisableFlashlight` / `EnableFlashlight`.
    pub fn set_enabled(&mut self, enabled: bool, context: Context) {
        if !enabled && self.on {
            self.turn_off(context);
        }
        self.disabled = !enabled;
    }

    /// One frame of power (`SuitPower_Update`, Episode Two's battery in `UpdateClientData`).
    pub fn update(&mut self, dt: f32, context: Context) {
        match &mut self.battery {
            None => {
                let ok = self.suit_power.update(dt, context.now);
                if self.on && (!ok || self.suit_power.power < LEGACY_CUTOFF) {
                    self.turn_off(context);
                }
            }
            Some(battery) if self.on => {
                *battery -= BATTERY_DRAIN * dt;
                if *battery < 0. {
                    *battery = 0.;
                    self.turn_off(context);
                }
            }
            Some(battery) => *battery = (*battery + BATTERY_CHARGE * dt).min(100.),
        }
        if self.battery.is_some() {
            self.suit_power.update(dt, context.now);
        }
    }

    /// The power the light's low-power flicker reads: the battery, else suit power.
    pub fn power(&self) -> f32 {
        self.battery.unwrap_or(self.suit_power.power)
    }
}

/// The light this frame: a 45-degree projected texture from `origin` along `direction`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Light {
    pub origin: Vec3,
    pub direction: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// Field of view, degrees, both ways.
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    /// Attenuation constant, linear, quadratic.
    pub attenuation: [f32; 3],
}

const FOV: f32 = 45.;
const OFFSET_Y: f32 = -20.;
const NEAR: f32 = 4.;
const FAR: f32 = 750.;
const LINEAR: f32 = 100.;
/// Pull-back starts this close to a surface; it eases at this rate a frame.
const DIST_CUTOFF: f32 = 128.;
const DIST_DRAG: f32 = 0.2;
const EPSILON: f32 = 0.1;

/// The flashlight's light placement state (`CFlashlightEffect`).
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Placer {
    dist_mod: f32,
}

/// A hull trace (±4 units) from a point to another: the fraction travelled before a hit (1: clear).
pub type Trace<'a> = &'a dyn Fn(Vec3, Vec3) -> f32;

impl Placer {
    /// `TurnOn`.
    pub fn turn_on(&mut self) {
        self.dist_mod = 1.;
    }

    /// `UpdateLightNew` from the eye and the view basis; `power` for the low-power flicker, `time` its clock.
    pub fn place(
        &mut self,
        (eye, forward, right, up): (Vec3, Vec3, Vec3, Vec3),
        trace: Trace,
        (power, time): (f32, f32),
    ) -> Light {
        let mut offset_y = OFFSET_Y;
        // Looking down, the light rises by how far a point behind the eye is above it (r_swingflashlight).
        let swing = eye + forward * -12.;
        if swing.z > eye.z {
            offset_y += swing.z - eye.z;
        }
        let mut origin = eye + up * offset_y;
        if trace(eye, origin) < 1. {
            origin = eye;
        }
        let target = eye + forward * FAR;
        let direction = (target - origin).normalize_or(forward);
        let up = (up - direction * direction.dot(up)).normalize_or(up);
        let right = (right - direction * direction.dot(right)).normalize_or(right);
        let right = (right - up * up.dot(right)).normalize_or(right);
        let dist = trace(origin, target) * (target - origin).length();
        if dist < DIST_CUTOFF {
            let pull_back = DIST_CUTOFF - dist;
            self.dist_mod += (pull_back - self.dist_mod) * DIST_DRAG;
            let back = origin - direction * (pull_back - EPSILON);
            let fraction = trace(origin, back);
            if fraction < 1. {
                let max = fraction * (pull_back - EPSILON) - EPSILON;
                self.dist_mod = self.dist_mod.min(max);
            }
        } else {
            self.dist_mod += (0. - self.dist_mod) * DIST_DRAG;
        }
        let (linear, fov) = flicker(power, time);
        Light {
            origin: origin - direction * self.dist_mod,
            direction,
            right,
            up,
            fov,
            near: NEAR + self.dist_mod,
            far: FAR,
            attenuation: [0., linear, 0.],
        }
    }
}

/// The low-power flicker (the client's `HL2_EPISODIC` code): (linear attenuation, field of view).
fn flicker(power: f32, time: f32) -> (f32, f32) {
    if power > 10. {
        return (LINEAR, FOV);
    }
    let scale = if power >= 0. {
        if power <= 4.5 {
            simple_spline_remap(power, (4.5, 0.), (1., 0.))
        } else {
            1.
        }
    } else {
        simple_spline_remap(power, (10., 4.8), (1., 0.))
    }
    .clamp(0., 1.);
    let linear = if scale < 0.35 {
        let flick = (time * 6.).cos() * (time * 15.).sin();
        if flick > 0.25 && flick < 0.75 {
            LINEAR * scale
        } else {
            0.
        }
    } else {
        LINEAR * scale + 1.5 * (time * 7.).cos() * (time * 25.).sin()
    };
    (linear, FOV - 16. * (1. - scale))
}

/// `SimpleSplineRemapVal`: `value` mapped from `a` to `b` through the smoothstep spline.
fn simple_spline_remap(value: f32, (a0, a1): (f32, f32), (b0, b1): (f32, f32)) -> f32 {
    if a0 == a1 {
        return if value >= a1 { b1 } else { b0 };
    }
    let t = (value - a0) / (a1 - a0);
    let s = t * t * (3. - 2. * t);
    b0 + (b1 - b0) * s
}

#[cfg(test)]
mod tests;
