//! CHudHintKeyDisplay (SDK client/hud_hintdisplay.cpp; the retail client keeps its row
//! layout): the key hints env_hudhint sends as the KeyHintText message. SetHintText looks the
//! message up as a localization token (the raw text when there is none; nothing for a missing
//! `#token`) and splits it at `%`: text runs are HudHintTextSmall labels and each `%command%`
//! becomes the key bound to that command in HudHintTextLarge. The labels alternate between
//! two columns, each row as tall as its taller label, and the panel grows from its right and
//! bottom edges (`r` positions) to fit them. KeyHintMessageShow / KeyHintMessageHide animate it.
use super::{font::HudFont, panel_axis, NumericProperty, PanelEffects, WeaponHud};
use crate::gameui::localize::{placeholders, Localization};
use glam::{vec2, Vec2};
use source_assets::{keyvalues::Entry, vpk::Vfs};
use source_ui::canvas::{Canvas, MutexCell, Rect};
use source_ui::font::{CellBaseline, FontFace};
use std::collections::BTreeMap;

/// The faces drawn for the hint fonts' Verdana, which the installation does not have: DejaVu
/// Sans Bold and DejaVu Sans, the installed faces nearest Verdana's widths (native's FORWARD is
/// 69 px wide at 720p, DejaVu Sans about 74, Liberation Sans about 78); the HUD's bold face
/// without them.
pub(super) fn faces(vfs: &Vfs, canvas: &Canvas, fallback: &FontFace) -> (FontFace, FontFace) {
    let face = |path: &str| {
        vfs.read(path)
            .ok()
            .flatten()
            .and_then(|bytes| FontFace::load_with(&bytes, canvas.clone(), CellBaseline::Gdi).ok())
            .unwrap_or_else(|| fallback.clone())
    };
    (
        face("resource/linux_fonts/dejavusans-bold.ttf"),
        face("resource/linux_fonts/dejavusans.ttf"),
    )
}

/// One label: its text (lines split at `\n`) and whether it names a key (HudHintTextLarge).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct HintLabel {
    pub(super) text: String,
    pub(super) key: bool,
}

/// SetHintText's labels for a KeyHintText message; `None` hides the hint.
pub(super) fn labels(
    message: &str,
    loc: &Localization,
    keys: &BTreeMap<String, String>,
) -> Option<Vec<HintLabel>> {
    if message.is_empty() {
        return None;
    }
    let text = match loc.lookup(message).filter(|text| !text.is_empty()) {
        Some(text) => text.to_owned(),
        None if message.starts_with('#') => return None,
        None => message.to_owned(),
    };
    let mut labels = Vec::new();
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let key = rest.starts_with('%');
        if key {
            rest = &rest[1..];
        }
        let end = rest.find('%').unwrap_or(rest.len());
        let token = &rest[..end];
        rest = &rest[end..];
        if key {
            rest = rest.strip_prefix('%').unwrap_or(rest);
        }
        labels.push(HintLabel {
            text: if key {
                key_name(token, loc, keys)
            } else {
                token.to_owned()
            },
            key,
        });
    }
    Some(labels)
}

/// The label of a hint's `%command%`: the key Key_LookupBinding finds for it (a leading `+`
/// ignored on both sides), upper-cased, or the text of a localization token of that name;
/// the retail client's `< not bound >` when no key runs the command.
fn key_name(command: &str, loc: &Localization, keys: &BTreeMap<String, String>) -> String {
    let bare = |name: &str| name.trim().trim_start_matches('+').to_ascii_lowercase();
    let wanted = bare(command);
    let name = keys
        .iter()
        .find(|(action, key)| !key.is_empty() && bare(action) == wanted)
        .map_or_else(
            || loc.text(placeholders::KEY_NOT_BOUND),
            |(_, key)| source_ui::keys::display_name(key),
        )
        .to_ascii_uppercase();
    loc.lookup(&name)
        .filter(|text| !text.is_empty())
        .map_or(name, str::to_owned)
}

/// HudHintKeyDisplay in scripts/HudLayout.res (480-line units).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layout {
    x: String,
    y: String,
    /// wide, tall.
    size: Vec2,
    /// text_xpos, text_ypos, text_xgap, text_ygap.
    text: [f32; 4],
}

impl Layout {
    pub(super) fn read(entry: Option<&Entry>) -> Self {
        let value = |key: &str, fallback: f32| {
            entry
                .and_then(|e| e.get(key))
                .and_then(Entry::text)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(fallback)
        };
        let position = |key: &str| {
            entry
                .and_then(|e| e.get(key))
                .and_then(Entry::text)
                .unwrap_or("0")
                .trim()
                .to_owned()
        };
        Self {
            x: position("xpos"),
            y: position("ypos"),
            size: vec2(value("wide", 100.), value("tall", 200.)),
            text: ["text_xpos", "text_ypos", "text_xgap", "text_ygap"].map(|k| value(k, 8.)),
        }
    }

    /// The panel and each label's top-left corner (relative to the panel) for labels of the
    /// given sizes on a canvas of `viewport` pixels.
    pub(super) fn place(&self, sizes: &[Vec2], viewport: Vec2) -> (Rect, Vec<Vec2>) {
        let scale = crate::proportional::scale(viewport);
        let [text_x, text_y, gap_x, gap_y] = self.text.map(|v| (v * scale).trunc());
        let widest = |column: usize| {
            sizes
                .iter()
                .skip(column)
                .step_by(2)
                .fold(0f32, |w, s| w.max(s.x))
        };
        let second = text_x + widest(0) + gap_x;
        let mut origins = vec![Vec2::ZERO; sizes.len()];
        let mut y = text_y;
        for row in (0..sizes.len()).step_by(2) {
            let first = sizes[row].y;
            let height = sizes.get(row + 1).map_or(first, |s| first.max(s.y));
            if let Some(size) = sizes.get(row + 1) {
                origins[row + 1] = vec2(second, y + ((height - size.y) / 2.).trunc());
            }
            origins[row] = vec2(text_x, y + ((height - first) / 2.).trunc());
            y += height + gap_y;
        }
        let size = vec2(text_x + second + widest(1), y);
        // Panel::IsRightAligned / IsBottomAligned (`r` positions) keep the far edge in place.
        let edge = |value: &str, extent: f32, axis_scale: f32, layout: f32, grown: f32| {
            let start = panel_axis(value, extent, axis_scale).unwrap_or(0.).trunc();
            if value.starts_with('r') {
                start + (layout * scale).trunc() - grown
            } else {
                start
            }
        };
        let x = edge(&self.x, viewport.x, scale, self.size.x, size.x);
        let y_scale = crate::proportional::y_scale(viewport);
        let y = edge(&self.y, viewport.y, y_scale, self.size.y, size.y);
        (Rect::new(x, y, size.x, size.y), origins)
    }
}

#[derive(Default)]
struct State {
    /// When the shown message was sent: each send arrives once.
    sent: Option<f64>,
    labels: Vec<HintLabel>,
    /// The animation event the next frame starts.
    event: Option<&'static str>,
    /// The panel and label origins for the canvas size they were placed at.
    placed: Option<(Vec2, Rect, Vec<Vec2>)>,
}

pub(super) struct KeyHintHud {
    layout: Layout,
    large: HudFont,
    small: HudFont,
    loc: Localization,
    state: MutexCell<State>,
}

impl KeyHintHud {
    /// `faces`: the installed Verdana stand-ins (bold for HudHintTextLarge, regular for
    /// HudHintTextSmall).
    pub(super) fn load(
        layout: Option<&Entry>,
        fonts: &Entry,
        faces: (FontFace, FontFace),
        loc: Localization,
    ) -> Self {
        Self {
            layout: Layout::read(layout),
            large: HudFont::read(fonts, "HudHintTextLarge", faces.0, 14.),
            small: HudFont::read(fonts, "HudHintTextSmall", faces.1, 11.),
            loc,
            state: MutexCell::new(State::default()),
        }
    }

    /// The scene's last KeyHintText message: a new one builds the labels (or hides them) and
    /// queues its animation event; none (a new map) forgets the last one.
    fn receive(&self, hint: Option<&(String, f64)>, keys: &BTreeMap<String, String>) {
        let mut state = self.state.borrow_mut();
        let sent = hint.map(|h| h.1);
        if sent == state.sent {
            return;
        }
        state.sent = sent;
        let Some((message, _)) = hint else {
            return;
        };
        match labels(message, &self.loc, keys) {
            Some(labels) => {
                state.labels = labels;
                state.placed = None;
                state.event = Some("KeyHintMessageShow");
            }
            None => state.event = Some("KeyHintMessageHide"),
        }
    }

    pub(super) fn take_event(&self) -> Option<&'static str> {
        self.state.borrow_mut().event.take()
    }

    fn font(&self, label: &HintLabel) -> &HudFont {
        if label.key {
            &self.large
        } else {
            &self.small
        }
    }

    /// Label::SizeToContents: the widest line and the font height per line.
    fn size(&self, label: &HintLabel) -> Vec2 {
        let font = self.font(label);
        let lines = label.text.split('\n');
        let wide = lines
            .clone()
            .fold(0f32, |w, line| w.max(font.width(line).round()));
        vec2(wide, font.gdi_height() * lines.count() as f32)
    }
}

impl WeaponHud {
    /// env_hudhint's last KeyHintText message (the scene's) and the key bindings SetHintText
    /// shows for its `%command%`s.
    pub fn set_key_hint(&self, hint: Option<&(String, f64)>, keys: &BTreeMap<String, String>) {
        self.key_hint.receive(hint, keys);
    }

    /// CHudHintKeyDisplay's background (PaintBackgroundType 2) and labels, all in the panel's
    /// FgColor (OnThink), under its alpha.
    pub(super) fn draw_key_hint(&self, effects: PanelEffects) {
        if effects.opacity() <= 0. {
            return;
        }
        let hint = &self.key_hint;
        let viewport = self.canvas_size();
        let mut state = hint.state.borrow_mut();
        if state.labels.is_empty() {
            return;
        }
        if state.placed.as_ref().is_none_or(|p| p.0 != viewport) {
            let sizes: Vec<Vec2> = state.labels.iter().map(|l| hint.size(l)).collect();
            let (panel, origins) = hint.layout.place(&sizes, viewport);
            state.placed = Some((viewport, panel, origins));
        }
        let state = &*state;
        let Some((_, panel, origins)) = &state.placed else {
            return;
        };
        self.rounded_box(*panel, effects.color(NumericProperty::Background));
        let color = effects.color(NumericProperty::Foreground);
        self.canvas.additive(true);
        for (label, origin) in state.labels.iter().zip(origins) {
            let font = hint.font(label);
            let height = font.gdi_height();
            for (row, line) in label.text.split('\n').enumerate() {
                let top = panel.y + origin.y + row as f32 * height;
                font.draw(line, panel.x + origin.x, top, color);
            }
        }
        self.canvas.additive(false);
    }
}

#[cfg(test)]
mod tests;
