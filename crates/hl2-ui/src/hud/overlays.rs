//! What the client draws over the 3D view before the HUD's own panels: the screen fade
//! (ViewDrawFade) and the credits logo (SDK client/hl2/hud_credits.cpp CHudCredits::DrawLogo).
use super::WeaponHud;
use source_assets::{keyvalues, vpk::Vfs};
use source_ui::canvas::{Color, MutexCell};

/// The overlays' state: the queued screen fades and the credits parameters.
pub(super) struct Overlays {
    pub(super) fades: MutexCell<crate::fades::ScreenFades>,
    credits: CreditsParams,
}

impl Overlays {
    pub(super) fn load(vfs: &Vfs) -> Self {
        Self {
            fades: MutexCell::new(Default::default()),
            credits: CreditsParams::load(vfs),
        }
    }
}

/// scripts/credits.txt `CreditsParams`: the logo text and the credits color.
#[derive(Clone, Debug, Default)]
pub(super) struct CreditsParams {
    logo: String,
    color: [u8; 3],
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
            color: [0, 1, 2].map(|i| color.get(i).copied().unwrap_or(255)),
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
    pub fn draw_credits_logo(&self, time: f64, shown: Option<(f64, f32)>) {
        let Some((at, hold)) = shown else {
            return;
        };
        let Some(alpha) = logo_alpha((time - at) as f32, hold).filter(|a| *a > 0.) else {
            return;
        };
        if self.overlays.credits.logo.is_empty() {
            return;
        }
        let [r, g, b] = self.overlays.credits.color;
        let x = (self.canvas.width() - self.icons.width(&self.overlays.credits.logo)) * 0.5;
        let y = (self.canvas.height() - self.icons.size()) * 0.5;
        let color = Color::from_rgba(r, g, b, alpha as u8);
        self.icons.draw(&self.overlays.credits.logo, x, y, color);
    }
}

#[cfg(test)]
mod tests {
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
