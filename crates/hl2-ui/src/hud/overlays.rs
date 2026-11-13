//! What the client draws over the 3D view before the HUD's own panels: the screen fade
//! (ViewDrawFade) and the credits logo (SDK client/hl2/hud_credits.cpp CHudCredits::DrawLogo).
use super::{font::HudFont, WeaponHud};
use source_assets::keyvalues::Entry;
use source_assets::{keyvalues, vpk::Vfs};
use source_ui::canvas::{Color, MutexCell};
use source_ui::font::FontFace;

mod messages;

/// The overlays' state: the queued screen fades and the credits parameters.
pub(super) struct Overlays {
    pub(super) fades: MutexCell<crate::fades::ScreenFades>,
    credits: CreditsParams,
    /// ClientScheme's CreditsText (the installed scheme has none: the HUD's installed sans
    /// face at 20, its fallback), the intro names' font.
    credits_text: HudFont,
    /// env_message's titles.txt messages (CHudMessage).
    messages: messages::HudMessages,
}

impl Overlays {
    pub(super) fn load(vfs: &Vfs, (fonts, face): (&Entry, FontFace)) -> Self {
        Self {
            fades: MutexCell::new(Default::default()),
            credits: CreditsParams::load(vfs),
            credits_text: HudFont::read(fonts, "CreditsText", face.clone(), 20.),
            messages: messages::HudMessages::load(vfs, fonts, face),
        }
    }
}

/// scripts/credits.txt `CreditsParams`: the logo text and the credits color.
#[derive(Clone, Debug, Default)]
pub(super) struct CreditsParams {
    logo: String,
    color: [u8; 4],
    timing: Timing,
    /// (x, y) in 640x480 units.
    position: (f32, f32),
    /// IntroCreditsNames: (name, font).
    names: Vec<(String, String)>,
}

/// CreditsParams fade times (CHudCredits::PrepareCredits, its defaults when a key is missing).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub fade_in: f32,
    pub hold: f32,
    pub fade_out: f32,
    pub next: f32,
    pub pause: f32,
    pub logo_time: f32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            fade_in: 1.,
            hold: 3.,
            fade_out: 2.,
            next: 2.,
            pause: 2.,
            logo_time: 2.,
        }
    }
}

/// FadeBlend: 0..1 over a fade in, `hold` seconds and a fade out.
fn fade_blend(t: &Timing, hold: f32, local: f32) -> f32 {
    if local < 0. {
        return 0.;
    }
    let held = t.fade_in + hold;
    let blend = if local < t.fade_in {
        local / t.fade_in
    } else if local > held {
        if t.fade_out > 0. {
            1. - (local - held) / t.fade_out
        } else {
            0.
        }
    } else {
        1.
    };
    blend.max(0.)
}

/// When each intro name starts after RollCredits and how long it holds (PrepareIntroCredits,
/// DrawIntroCreditsName): three slots; the first three names start at once; while a name
/// shows, the name three after it starts `next` seconds before it ends (not before the first
/// wave's end plus `logo_time`) and holds longer by its slot + 1 in the second wave, by
/// `pause` after.
pub fn intro_schedule(t: &Timing, count: usize) -> Vec<(f32, f32)> {
    let life = |hold: f32| t.fade_in + hold + t.fade_out;
    let gate = life(t.hold) + t.logo_time;
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(count);
    for i in 0..count {
        if i < 3 {
            out.push((0., t.hold));
            continue;
        }
        let (start, hold) = out[i - 3];
        let add = if i < 6 { (i - 3 + 1) as f32 } else { t.pause };
        out.push(((start + life(hold) - t.next).max(gate), t.hold + add));
    }
    out
}

impl CreditsParams {
    pub(super) fn load(vfs: &Vfs) -> Self {
        let Some(bytes) = vfs.read("scripts/credits.txt").ok().flatten() else {
            return Self::default();
        };
        let Ok(text) = keyvalues::decode_text(&bytes) else {
            return Self::default();
        };
        let roots = keyvalues::parse(&text).unwrap_or_default();
        let Some(params) = roots.first().and_then(|r| r.get("CreditsParams")) else {
            return Self::default();
        };
        let number = |key: &str, default: f32| {
            params
                .get(key)
                .and_then(|v| v.text())
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(default)
        };
        let defaults = Timing::default();
        let timing = Timing {
            fade_in: number("fadeintime", defaults.fade_in),
            hold: number("fadeholdtime", defaults.hold),
            fade_out: number("fadeouttime", defaults.fade_out),
            next: number("nextfadetime", defaults.next),
            pause: number("pausebetweenwaves", defaults.pause),
            logo_time: number("logotime", defaults.logo_time),
        };
        let names = roots
            .first()
            .and_then(|r| r.get("IntroCreditsNames"))
            .map(|list| {
                let pair = |e: &Entry| (e.key.clone(), e.text().unwrap_or("").to_string());
                list.children().iter().map(pair).collect()
            })
            .unwrap_or_default();
        let color: Vec<u8> = params
            .get("color")
            .and_then(|c| c.text())
            .unwrap_or("255 255 255")
            .split_whitespace()
            .filter_map(|v| v.parse().ok())
            .collect();
        Self {
            logo: params
                .get("logo")
                .and_then(|l| l.text())
                .unwrap_or("")
                .to_string(),
            color: [0, 1, 2, 3].map(|i| color.get(i).copied().unwrap_or(255)),
            timing,
            position: (number("posx", 2.), number("posy", 2.)),
            names,
        }
    }
}

/// The logo's alpha `elapsed` seconds after it was shown (PrepareLogo, DrawLogo): a 5 s fade
/// in from -128 (clamped at 0), `hold` seconds at 255, then a 2 s fade out.
pub fn logo_alpha(elapsed: f32, hold: f32) -> Option<f32> {
    if elapsed < 0. {
        return None;
    }
    if elapsed < 5. {
        return Some((-128. + 383. * elapsed / 5.).max(0.));
    }
    if elapsed < 5. + hold {
        return Some(255.);
    }
    let out = elapsed - 5. - hold;
    (out < 2.).then(|| 255. * (1. - out / 2.))
}

impl WeaponHud {
    /// ViewDrawFade over the 3D view, before the HUD. The modulate material's exact
    /// use of the fade alpha is engine code; here the frame is multiplied by the color
    /// blended toward white by (1 - alpha), which is inferred, not verified.
    pub fn draw_fade(&self, time: f64, intro: Option<[f32; 4]>) {
        let Some(([r, g, b, a], modulate)) = self.overlays.fades.borrow_mut().params(time, intro)
        else {
            return;
        };
        if a == 0 {
            return;
        }
        if modulate {
            let t = f32::from(a) / 255.;
            let mix = |c: u8| 1. - t + t * f32::from(c) / 255.;
            self.canvas.modulate(Color::new(mix(r), mix(g), mix(b), 1.));
        } else {
            self.canvas.additive(false);
            self.canvas.rectangle(
                0.,
                0.,
                self.canvas.width(),
                self.canvas.height(),
                Color::from_rgba(r, g, b, a),
            );
        }
    }
    /// CHudCredits::DrawLogo: the logo in the WeaponIcons font, centered, in the credits color
    /// with the fade's alpha; `shown` is when env_credits showed it and how long it holds.
    pub fn draw_credits(&self, time: f64, intro: &hl2_simulation::script_intro::Intro) {
        if let Some(rolled) = intro.credits {
            self.draw_intro_names((time - rolled) as f32);
        }
        let Some((at, hold)) = intro.logo else {
            return;
        };
        let Some(alpha) = logo_alpha((time - at) as f32, hold).filter(|a| *a > 0.) else {
            return;
        };
        if self.overlays.credits.logo.is_empty() {
            return;
        }
        let [r, g, b, _] = self.overlays.credits.color;
        let x = (self.canvas.width() - self.icons.width(&self.overlays.credits.logo)) * 0.5;
        let y = (self.canvas.height() - self.icons.size()) * 0.5;
        let color = Color::from_rgba(r, g, b, alpha as u8);
        self.icons.draw(&self.overlays.credits.logo, x, y, color);
    }
}

impl WeaponHud {
    /// DrawIntroCreditsName: each name of its wave at XRES(posx), YRES(posy + slot x font
    /// tall), in its font (CreditsText; the Valve "V" in WeaponIcons), in the credits color
    /// with FadeBlend x the color's alpha.
    fn draw_intro_names(&self, elapsed: f32) {
        let credits = &self.overlays.credits;
        let t = &credits.timing;
        let (width, height) = (self.canvas.width(), self.canvas.height());
        let schedule = intro_schedule(t, credits.names.len());
        for (i, ((name, font), (start, hold))) in credits.names.iter().zip(schedule).enumerate() {
            let blend = fade_blend(t, hold, elapsed - start);
            if blend <= 0. || name.trim().is_empty() {
                continue;
            }
            let font = if font == "WeaponIcons" {
                &self.icons
            } else {
                &self.overlays.credits_text
            };
            let (x, slot) = (credits.position.0 * width / 640., (i % 3) as f32);
            let y = (credits.position.1 + slot * font.size()) * height / 480.;
            let [r, g, b, a] = credits.color;
            font.draw(
                name,
                x,
                y,
                Color::from_rgba(r, g, b, (blend * f32::from(a)) as u8),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn intro_names_come_in_waves_of_three_like_the_sdk() {
        use super::{intro_schedule, Timing};
        // The installed credits.txt timing: fade in 2.5, hold 1, fade out 1, next 1, pause 1.
        let t = Timing {
            fade_in: 2.5,
            hold: 1.,
            fade_out: 1.,
            next: 1.,
            pause: 1.,
            logo_time: 1.,
        };
        let s = intro_schedule(&t, 8);
        assert_eq!(s[..3], [(0., 1.); 3]);
        // The second wave waits for the first's end (4.5) plus logotime; holds 2, 3, 4.
        assert_eq!(s[3], (5.5, 2.));
        assert_eq!(s[5], (5.5, 4.));
        // Then each slot's next name starts 1 s before the previous one ends, holding 2.
        assert_eq!(s[6], (5.5 + 5.5 - 1., 2.));
    }

    #[test]
    fn the_logo_fades_in_over_five_seconds_holds_and_fades_out_over_two() {
        use super::logo_alpha;
        assert_eq!(logo_alpha(1., 5.), Some(0.));
        assert_eq!(logo_alpha(2.5, 5.), Some(63.5));
        assert_eq!(logo_alpha(7., 5.), Some(255.));
        assert_eq!(logo_alpha(11., 5.), Some(127.5));
        assert_eq!(logo_alpha(12., 5.), None);
    }
}
