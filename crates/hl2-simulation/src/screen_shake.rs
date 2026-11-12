//! Screen shakes (Source SDK game/client/view_effects.cpp CViewEffects::Shake and CalcShake;
//! game/server/util.cpp UTIL_ScreenShake; game/server/EnvShake.cpp env_shake).
//!
//! Each shake keeps a random offset (each axis within +-amplitude) and roll (+-amplitude / 4),
//! re-rolled `frequency` times a second, and contributes them times fraction^2 x
//! sin(time x frequency / fraction), fraction being the share of its duration left; its amplitude
//! drops by frametime / (duration x frequency) of itself each frame. The view takes the sum
//! (ApplyShake factor 1), the viewmodel a tenth (CalcViewModelView).
//!
//! Ours is a function of time: the per-frame amplitude drop is its continuous limit,
//! exp(-elapsed / (duration x frequency)), and each re-roll is seeded by its slot, so the host can
//! read the shake at its own frame times. Not in: the in-air check (UTIL_ScreenShake skips a
//! player off the ground unless the shake says otherwise), physics and rope shakes, rumble.
use glam::Vec3;

/// MAX_SHAKES: the view keeps at most this many.
const MAX_SHAKES: usize = 32;
/// UTIL_ScreenShake caps a shake's amplitude here.
const MAX_AMPLITUDE: f32 = 16.;

/// ShakeCommand_t (shake.h).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Start,
    Stop,
    Amplitude,
    Frequency,
}

#[derive(Clone, Copy, Debug)]
struct Shake {
    amplitude: f32,
    frequency: f32,
    duration: f32,
    end: f64,
    /// When the amplitude was last set: the decay runs from here.
    set: f64,
    seed: u64,
}

/// The view's active shakes.
#[derive(Clone, Debug, Default)]
pub struct ScreenShakes {
    shakes: Vec<Shake>,
    started: u64,
    /// env_shake SetAmplitude / SetFrequency: what each one's Amplitude and Frequency inputs set,
    /// used by its later shakes instead of its keys.
    tuned: std::collections::BTreeMap<usize, (Option<f32>, Option<f32>)>,
}

impl ScreenShakes {
    /// Keeps an env_shake's new amplitude (Command::Amplitude) or frequency (Command::Frequency).
    pub fn tune(&mut self, entity: usize, command: Command, value: f32) {
        let tuned = self.tuned.entry(entity).or_default();
        match command {
            Command::Amplitude => tuned.0 = Some(value),
            Command::Frequency => tuned.1 = Some(value),
            Command::Start | Command::Stop => {}
        }
    }
    /// The amplitude and frequency an env_shake's inputs set, if any.
    pub fn tuned(&self, entity: usize) -> (Option<f32>, Option<f32>) {
        self.tuned.get(&entity).copied().unwrap_or_default()
    }
    /// CViewEffects::Shake: Start adds a shake (at most MAX_SHAKES), Stop clears them all, and
    /// Amplitude or Frequency retunes the one that ends last.
    pub fn command(
        &mut self,
        command: Command,
        (amplitude, frequency, duration): (f32, f32, f32),
        now: f64,
    ) {
        self.shakes.retain(|s| s.end >= now);
        match command {
            Command::Start if self.shakes.len() < MAX_SHAKES => {
                self.started += 1;
                self.shakes.push(Shake {
                    amplitude,
                    frequency,
                    duration,
                    end: now + f64::from(duration),
                    set: now,
                    seed: self.started,
                });
            }
            Command::Start => {}
            Command::Stop => self.shakes.clear(),
            Command::Amplitude | Command::Frequency => {
                let longest = self
                    .shakes
                    .iter_mut()
                    .max_by(|a, b| a.end.total_cmp(&b.end));
                if let Some(shake) = longest {
                    if command == Command::Amplitude {
                        (shake.amplitude, shake.set) = (amplitude, now);
                    } else {
                        shake.frequency = frequency;
                    }
                }
            }
        }
    }
    /// CalcShake at `now`: the view's offset (units) and roll (degrees).
    pub fn applied(&self, now: f64) -> (Vec3, f32) {
        let (mut offset, mut roll) = (Vec3::ZERO, 0.);
        for shake in &self.shakes {
            if let Some((o, r)) = shake.at(now) {
                offset += o;
                roll += r;
            }
        }
        (offset, roll)
    }
}

impl Shake {
    fn at(&self, now: f64) -> Option<(Vec3, f32)> {
        if now > self.end || self.duration <= 0. || self.amplitude <= 0. || self.frequency <= 0. {
            return None;
        }
        let decay = |at: f64| (-((at - self.set) as f32) / (self.duration * self.frequency)).exp();
        // nextShake starts at 0: the first roll is at once, then one each 1 / frequency.
        let start = self.end - f64::from(self.duration);
        let slot = ((now - start) * f64::from(self.frequency)).floor().max(0.);
        let rolled = (start + slot / f64::from(self.frequency)).max(self.set);
        let amplitude = self.amplitude * decay(rolled);
        let mut random = Random(self.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ slot as u64);
        let offset = Vec3::new(random.float(), random.float(), random.float()) * amplitude;
        let angle = random.float() * amplitude * 0.25;
        let fraction = ((self.end - now) / f64::from(self.duration)) as f32;
        let frequency = if fraction > 0. {
            self.frequency / fraction
        } else {
            0.
        };
        let wave = fraction * fraction * ((now as f32 * frequency).min(1e8)).sin();
        Some((offset * wave, angle * wave))
    }
}

/// ComputeShakeAmplitude with UTIL_ScreenShake's cap: the amplitude a player at `at` feels from
/// a shake at `center`, falling to nothing at `radius` (0: everywhere); None outside it.
pub fn local_amplitude(center: Vec3, at: Vec3, amplitude: f32, radius: f32) -> Option<f32> {
    let amplitude = amplitude.min(MAX_AMPLITUDE);
    if radius <= 0. {
        return Some(amplitude);
    }
    let distance = center.distance(at);
    (distance <= radius).then(|| amplitude * (1. - distance / radius))
}

/// A splitmix64 stream giving floats in -1..1.
struct Random(u64);
impl Random {
    fn float(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 23) as f32 - 1.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shake_moves_within_its_amplitude_settles_and_ends() {
        let mut shakes = ScreenShakes::default();
        shakes.command(Command::Start, (4., 40., 2.), 10.);
        let samples: Vec<(Vec3, f32)> = (0..200)
            .map(|i| shakes.applied(10. + i as f64 * 0.01))
            .collect();
        assert!(samples.iter().any(|(o, _)| o.length() > 0.5));
        for (offset, roll) in &samples {
            assert!(offset.abs().max_element() <= 4. && roll.abs() <= 1.);
        }
        // Near its end the fraction squared has nearly stilled it; after it, nothing.
        assert!(shakes.applied(11.98).0.length() < 0.01);
        assert_eq!(shakes.applied(12.5), (Vec3::ZERO, 0.));
    }

    #[test]
    fn stop_clears_and_amplitude_retunes_the_longest() {
        let mut shakes = ScreenShakes::default();
        shakes.command(Command::Start, (4., 40., 2.), 0.);
        shakes.command(Command::Start, (4., 40., 5.), 0.);
        shakes.command(Command::Amplitude, (0., 0., 0.), 1.);
        // The 5 s shake is silenced; the 2 s one still moves the view.
        assert!((0..50).any(|i| shakes.applied(1. + i as f64 * 0.01).0.length() > 0.));
        assert_eq!(shakes.applied(2.5), (Vec3::ZERO, 0.));
        shakes.command(Command::Stop, (0., 0., 0.), 1.5);
        assert_eq!(shakes.applied(1.6), (Vec3::ZERO, 0.));
    }

    #[test]
    fn amplitude_falls_off_to_the_radius_and_is_capped() {
        assert_eq!(
            local_amplitude(Vec3::ZERO, Vec3::X * 50., 10., 100.),
            Some(5.)
        );
        assert_eq!(local_amplitude(Vec3::ZERO, Vec3::X * 150., 10., 100.), None);
        assert_eq!(
            local_amplitude(Vec3::ZERO, Vec3::X * 1e4, 40., 0.),
            Some(16.)
        );
    }
}
