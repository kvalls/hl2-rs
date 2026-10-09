//! Client screen fades: SDK CViewEffects::Fade / FadeCalculate (view_effects.cpp).
//! Each fade keeps its own timing; the frame uses the highest alpha, the average color
//! and a modulate blend when any fade modulates. Times are the paused game clock.
use hl2_simulation::player_damage::{
    ScreenFade, FFADE_IN, FFADE_MODULATE, FFADE_OUT, FFADE_PURGE, FFADE_STAYOUT,
};

#[derive(Clone, Copy, Debug)]
struct Active {
    color: [u8; 3],
    alpha: u8,
    flags: u32,
    speed: f32,
    end: f64,
    reset: f64,
}

#[derive(Clone, Debug, Default)]
pub struct ScreenFades {
    list: Vec<Active>,
    time: f64,
}
impl ScreenFades {
    pub fn add(&mut self, fade: ScreenFade, now: f64) {
        let [r, g, b, alpha] = fade.color;
        let mut active = Active {
            color: [r, g, b],
            alpha,
            flags: fade.flags,
            speed: 0.,
            end: f64::from(fade.duration),
            reset: f64::from(fade.hold),
        };
        if fade.duration > 0. {
            if fade.flags & FFADE_OUT != 0 {
                if active.end != 0. {
                    active.speed = -f32::from(alpha) / fade.duration;
                }
                active.end += now;
                active.reset += active.end;
            } else {
                if active.end != 0. {
                    active.speed = f32::from(alpha) / fade.duration;
                }
                active.reset += now;
                active.end += active.reset;
            }
        }
        if fade.flags & FFADE_PURGE != 0 {
            self.list.clear();
        }
        self.list.push(active);
    }
    pub fn clear(&mut self) {
        self.list.clear();
    }
    /// GetFadeParams: (r, g, b, a) and whether to modulate; None when nothing shows.
    pub fn params(&mut self, now: f64) -> Option<([u8; 4], bool)> {
        if now < self.time {
            // A new map clock: the old fades belonged to the previous timeline.
            self.list.clear();
        }
        self.time = now;
        for fade in &mut self.list {
            if fade.flags & FFADE_STAYOUT != 0 {
                fade.reset = now + 0.1;
            }
        }
        self.list.retain(|f| !(now > f.reset && now > f.end));
        if self.list.is_empty() {
            return None;
        }
        let mut sum = [0u32; 3];
        let mut alpha = 0i32;
        let mut modulate = false;
        for fade in &self.list {
            for (s, c) in sum.iter_mut().zip(fade.color) {
                *s += u32::from(c);
            }
            let value = if fade.flags & (FFADE_OUT | FFADE_IN) != 0 {
                // The SDK converts the product to an int before adding alpha.
                let mut v = (fade.speed * (fade.end - now) as f32) as i32;
                if fade.flags & FFADE_OUT != 0 {
                    v += i32::from(fade.alpha);
                }
                v.min(i32::from(fade.alpha)).max(0)
            } else {
                i32::from(fade.alpha)
            };
            alpha = alpha.max(value);
            modulate |= fade.flags & FFADE_MODULATE != 0;
        }
        let count = self.list.len() as u32;
        let color = sum.map(|s| (s / count) as u8);
        Some(([color[0], color[1], color[2], alpha as u8], modulate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_in_holds_then_clears_and_fade_out_stays() {
        let mut fades = ScreenFades::default();
        // DamageEffect crush: red 128 alpha, 1 s fade from 0.1 s hold.
        fades.add(ScreenFade::new([128, 0, 0, 128], 1., 0.1, FFADE_IN), 10.);
        assert_eq!(fades.params(10.05), Some(([128, 0, 0, 128], false)));
        let ([_, _, _, half], _) = fades.params(10.6).unwrap();
        assert!((63..=65).contains(&half), "{half}");
        assert_eq!(fades.params(11.2), None);
        // env_fade style out + stay: reaches full alpha and remains.
        fades.add(
            ScreenFade::new([0, 0, 0, 255], 2., 0., FFADE_OUT | FFADE_STAYOUT),
            20.,
        );
        let ([_, _, _, a], _) = fades.params(21.).unwrap();
        assert!((126..=128).contains(&a), "{a}");
        assert_eq!(fades.params(30.), Some(([0, 0, 0, 255], false)));
        // Purge clears it; modulate flags the blend; colors average.
        fades.add(
            ScreenFade::new([0, 0, 255, 100], 0.2, 0.4, FFADE_MODULATE | FFADE_PURGE),
            30.,
        );
        fades.add(ScreenFade::new([255, 0, 0, 50], 1., 0., 0), 30.);
        assert_eq!(fades.params(30.1), Some(([127, 0, 127, 100], true)));
        // A zero duration never adds the clock, so its hold has already expired.
        fades.add(ScreenFade::new([0, 255, 0, 255], 0., 5., FFADE_PURGE), 40.);
        assert_eq!(fades.params(40.), None);
    }
}
