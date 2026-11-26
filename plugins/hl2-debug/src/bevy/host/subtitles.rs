//! Captions for the Console speaker, submitted to the shared game HUD.
use hl2_ui::closecaption::CaptionHud;
use source_ui::console::panel::{CHAT_REMOTE_RGB, Level, chat_label_end};
use std::collections::VecDeque;

const PENDING_LIMIT: usize = 4;
const CHARACTER_LIMIT: usize = 512;

#[derive(Default)]
pub(super) struct RemoteSubtitles(VecDeque<String>);

impl RemoteSubtitles {
    pub(super) fn push(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        let mut chars = text.chars();
        let mut text: String = chars.by_ref().take(CHARACTER_LIMIT).collect();
        if chars.next().is_some() {
            text.push_str("...");
        }
        if self.0.len() == PENDING_LIMIT {
            self.0.pop_front();
        }
        self.0.push_back(text);
    }

    pub(super) fn submit(&mut self, captions: &CaptionHud) {
        for line in self.0.drain(..) {
            let duration = (4. + line.chars().count() as f32 / 24.).min(15.);
            captions.speak(&console_speech(&line), duration);
        }
    }
}

fn console_speech(line: &str) -> String {
    let [r, g, b] = CHAT_REMOTE_RGB;
    let (label, body) = line.split_at(chat_label_end(Level::ChatRemote, line).unwrap_or_default());
    format!("{label}<clr:{r},{g},{b}>{body}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn speaker_label_precedes_body_color_and_native_tags_pass_through() {
        assert_eq!(
            super::console_speech("Console: <B>bold<B>"),
            "Console:<clr:115,230,255> <B>bold<B>"
        );
    }
}
