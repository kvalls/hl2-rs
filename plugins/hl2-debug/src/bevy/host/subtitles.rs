//! Literal remote chat in the gameplay caption area.
use hl2_bevy::canvas_renderer::Hud;
use source_ui::{
    canvas::Color,
    widgets::{truncate_ellipsis, wrap},
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MESSAGE_LIMIT: usize = 4;
const CHARACTER_LIMIT: usize = 512;
const ROW_LIMIT: usize = 6;

struct Message {
    text: String,
    expires: Instant,
}

#[derive(Default)]
pub(super) struct RemoteSubtitles {
    messages: VecDeque<Message>,
    rows: Vec<String>,
    metrics: Option<(f32, f32, usize)>,
    visible: bool,
}

impl RemoteSubtitles {
    pub(super) fn has_messages(&self) -> bool {
        !self.messages.is_empty()
    }

    pub(super) fn push(&mut self, text: &str, now: Instant) {
        let mut chars = text.chars();
        let mut text: String = chars
            .by_ref()
            .take(CHARACTER_LIMIT)
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .collect();
        if chars.next().is_some() {
            text.push_str("...");
        }
        if text.trim().is_empty() {
            return;
        }
        let seconds = (5. + text.chars().count() as f64 / 24.).min(16.);
        if self.messages.len() == MESSAGE_LIMIT {
            self.messages.pop_front();
        }
        self.messages.push_back(Message {
            text,
            expires: now + Duration::from_secs_f64(seconds),
        });
        self.metrics = None;
    }

    fn expire(&mut self, now: Instant) {
        let before = self.messages.len();
        self.messages.retain(|message| message.expires > now);
        if self.messages.len() != before {
            self.metrics = None;
        }
    }

    fn layout(&mut self, (width, tall, limit): (f32, f32, usize), measure: impl Fn(&str) -> f32) {
        let metrics = Some((width, tall, limit));
        if self.metrics == metrics {
            return;
        }
        self.metrics = metrics;
        self.rows.clear();
        for message in self.messages.iter().rev() {
            let remaining = limit - self.rows.len();
            if remaining == 0 {
                break;
            }
            let mut rows = wrap(&message.text, width, &measure);
            if rows.len() > remaining {
                rows.truncate(remaining);
                let last = rows.last_mut().expect("at least one subtitle row");
                last.push_str("...");
                *last = truncate_ellipsis(last, width, &measure).into_owned();
            }
            self.rows.extend(rows.into_iter().rev());
        }
        self.rows.reverse();
    }

    pub(super) fn present(&mut self, hud: &Hud, gameplay: bool, native_captions: bool) {
        self.visible = false;
        self.expire(Instant::now());
        if !gameplay || self.messages.is_empty() {
            return;
        }
        let Some(weapon) = &hud.source.weapon else {
            return;
        };
        let canvas = &hud.source.canvas;
        let size = (canvas.width(), canvas.height());
        let Some((panel, face, tall)) = weapon.captions.text_surface(size) else {
            return;
        };
        let padding = (tall * 0.4).ceil();
        let width = panel.w.min(size.0);
        let x = panel.x.clamp(0., size.0 - width);
        // Reserve the native panel while dialogue captions are enabled.
        let bottom = if native_captions {
            panel.y - padding
        } else {
            panel.y + panel.h
        }
        .clamp(0., size.1);
        let limit = (((bottom - 2. * padding) / tall) as usize).min(ROW_LIMIT);
        if width <= 2. * padding || limit == 0 {
            return;
        }
        self.layout((width - 2. * padding, tall, limit), |row| {
            face.drawn_width_cell(row, tall)
        });
        let height = self.rows.len() as f32 * tall + 2. * padding;
        let top = bottom - height;
        canvas.rectangle(x, top, width, height, Color::new(0., 0., 0., 0.75));
        for (index, row) in self.rows.iter().enumerate() {
            let pos = glam::vec2(x + padding, top + padding + index as f32 * tall);
            face.draw_cell(row, pos, tall, Color::new(0.8, 0.96, 1., 1.));
        }
        self.visible = true;
    }

    pub(super) fn visible_rows(&self) -> &[String] {
        if self.visible { &self.rows } else { &[] }
    }
}

#[cfg(test)]
mod tests {
    use super::{Instant, RemoteSubtitles};
    use std::time::Duration;

    #[test]
    fn remote_text_wraps_literally_and_keeps_newest_messages_in_order() {
        let now = Instant::now();
        let mut subtitles = RemoteSubtitles::default();
        for text in ["old 1", "old 2", "old 3", "hello", "<sfx> café"] {
            subtitles.push(text, now);
        }
        assert_eq!(subtitles.messages.len(), 4);
        subtitles.layout((8., 1., 3), |text| text.chars().count() as f32);
        assert_eq!(subtitles.rows, ["hello", "<sfx>", "café"]);
        assert!(subtitles.visible_rows().is_empty());
        subtitles.expire(now + Duration::from_secs(30));
        assert!(subtitles.messages.is_empty());
        assert!(subtitles.metrics.is_none());
    }

    #[test]
    fn visible_rows_fit_and_truncation_is_marked() {
        let mut subtitles = RemoteSubtitles::default();
        subtitles.push("abcdefghijk\n followed by more", Instant::now());
        subtitles.layout((8., 1., 1), |text| text.chars().count() as f32);
        assert_eq!(subtitles.rows, ["abcde..."]);
    }
}
