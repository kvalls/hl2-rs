//! HUD text messages (SDK client/message.cpp CHudMessage): env_message's titles.txt messages
//! scanned or faded in the CenterPrintText font. Native (this build) draws the stored text as it
//! is, so d1_trainstation_01's chapter title reads `#HL2_Chapter1_Title`.
use super::super::{font::HudFont, WeaponHud};
use source_assets::keyvalues::Entry;
use source_assets::titles::TextMessage;
use source_assets::{keyvalues, vpk::Vfs};
use source_ui::canvas::Color;
use source_ui::font::FontFace;
use std::collections::BTreeMap;

/// The installed titles.txt and the message font.
pub(in crate::hud) struct HudMessages {
    titles: BTreeMap<String, TextMessage>,
    /// ClientScheme's CenterPrintText (Trebuchet MS 900, tall 18 proportional, additive) in
    /// the HUD's installed bold face.
    font: HudFont,
}

impl HudMessages {
    pub(super) fn load(vfs: &Vfs, fonts: &Entry, face: FontFace) -> Self {
        let titles = vfs
            .read("scripts/titles.txt")
            .ok()
            .flatten()
            .and_then(|data| keyvalues::decode_text(&data).ok())
            .map(|text| source_assets::titles::parse(&text))
            .unwrap_or_default();
        Self {
            titles,
            font: HudFont::read(fonts, "CenterPrintText", face, 18.),
        }
    }
}

/// How long a message shows (CHudMessage::Paint's endTime), from its start.
pub fn duration(m: &TextMessage) -> f32 {
    let fadein = if m.effect == 2 {
        m.fadein * m.text.len() as f32
    } else {
        m.fadein
    };
    fadein + m.fadeout + m.holdtime
}

/// MessageScanStart's fade (0 shown .. 255 gone) at `time` into the message; `length` counts
/// every character, line breaks included.
fn fade_blend(m: &TextMessage, time: f32, length: usize) -> f32 {
    let (fade_time, fading_in) = if m.effect == 2 {
        (m.fadein * length as f32 + m.holdtime, false)
    } else {
        (m.fadein + m.holdtime, time < m.fadein)
    };
    if fading_in {
        (m.fadein - time) / m.fadein * 255.
    } else if time > fade_time {
        if m.fadeout > 0. {
            (time - fade_time) / m.fadeout * 255.
        } else if m.effect == 2 {
            0.
        } else {
            255.
        }
    } else {
        0.
    }
}

/// MessageScanNextChar: character `index`'s color and alpha at `time` into the message.
pub fn char_color(m: &TextMessage, time: f32, index: usize, length: usize) -> ([u8; 3], u8) {
    let fade = fade_blend(m, time, length).clamp(0., 255.) as i32;
    let rgb = |c: [u8; 4]| [i32::from(c[0]), i32::from(c[1]), i32::from(c[2])];
    let (mut source, mut dest, mut blend) = (rgb(m.color), [0; 3], 0);
    if m.effect == 2 {
        let char_time = m.fadein * (index + 1) as f32;
        if char_time > time {
            source = [0; 3];
        } else if time > m.fadein * length as f32 + m.holdtime {
            blend = fade;
        } else if time - char_time <= m.fxtime {
            dest = rgb(m.color2);
            blend = 255 - ((time - char_time) / m.fxtime * 255. + 0.5) as i32;
        }
    } else {
        blend = fade;
    }
    let blend = blend.clamp(0, 255);
    let mix = |i: usize| ((source[i] * (255 - blend) + dest[i] * blend) >> 8) as u8;
    ([mix(0), mix(1), mix(2)], (255 - fade) as u8)
}

/// XPosition / YPosition: -1 centers, other negatives align to the far edge, kept on screen.
fn position(fraction: f32, size: f32, total: f32, screen: f32) -> f32 {
    let at = if fraction == -1. {
        (screen - size) * 0.5
    } else if fraction < 0. {
        (1. + fraction) * screen - total
    } else {
        fraction * screen
    };
    at.min(screen - size).max(0.)
}

impl WeaponHud {
    /// CHudMessage::Paint over the messages env_message showed: (titles.txt name, start).
    pub fn draw_messages(&self, time: f64, shown: &[(String, f64)]) {
        let messages = &self.overlays.messages;
        let font = &messages.font;
        let (width, height) = (self.canvas.width(), self.canvas.height());
        let mut buffer = [0u8; 4];
        for (name, start) in shown {
            let key = name.trim_start_matches('#').to_ascii_uppercase();
            let Some(m) = messages.titles.get(&key) else {
                continue;
            };
            let t = (time - start) as f32;
            if !(0. ..=duration(m)).contains(&t) {
                continue;
            }
            let length = m.text.chars().count();
            let line_width = |line: &str| -> f32 {
                line.chars()
                    .map(|c| font.width(c.encode_utf8(&mut [0u8; 4])))
                    .sum()
            };
            let total_width = m.text.split('\n').map(line_width).fold(0., f32::max);
            let lines = m.text.split('\n').count() as f32;
            let mut y = position(m.y, lines * font.size(), lines * font.size(), height);
            let mut index = 0;
            self.canvas.additive(true);
            for line in m.text.split('\n') {
                let mut x = position(m.x, line_width(line), total_width, width);
                for c in line.chars() {
                    let ([r, g, b], a) = char_color(m, t, index, length);
                    let glyph = c.encode_utf8(&mut buffer);
                    if r | g | b != 0 {
                        font.draw(glyph, x, y, Color::from_rgba(r, g, b, a));
                    }
                    x += font.width(glyph);
                    index += 1;
                }
                // The line break takes no scan time (only MessageDrawScan's length counts it).
                y += font.size();
            }
            self.canvas.additive(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title() -> TextMessage {
        TextMessage {
            effect: 2,
            color: [100, 100, 100, 255],
            color2: [240, 110, 0, 255],
            x: -1.,
            y: 0.58,
            fadein: 0.01,
            fadeout: 1.5,
            holdtime: 3.5,
            fxtime: 0.25,
            text: "#HL2_Chapter1_Title".into(),
        }
    }

    #[test]
    fn the_chapter_title_scans_out_highlighted_then_fades() {
        let m = title();
        let length = m.text.len();
        // The third character appears at 0.03 s in the highlight color, gray after fxtime.
        assert_eq!(char_color(&m, 0.025, 2, length).0, [0, 0, 0]);
        assert_eq!(char_color(&m, 0.031, 2, length), ([238, 109, 0], 255));
        assert_eq!(char_color(&m, 0.4, 2, length), ([99, 99, 99], 255));
        // Fade out after fadein x length + holdtime (3.69 s), over 1.5 s.
        let fading = char_color(&m, 3.69 + 0.75, 2, length);
        assert!(fading.1 > 120 && fading.1 < 135, "{fading:?}");
        assert!((duration(&m) - 5.19).abs() < 1e-4);
        assert_eq!(position(-1., 100., 100., 1280.), 590.);
        assert_eq!(position(0.58, 27., 27., 720.), 0.58 * 720.);
    }
}
