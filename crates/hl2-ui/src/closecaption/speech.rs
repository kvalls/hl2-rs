//! Single speech cues supplied by runtime contributors.
use super::{CaptionHud, CloseCaption, Item, Settings, MAX_CHARACTERS};

impl CloseCaption {
    pub(super) fn record_processed(&mut self, stream: &str) {
        self.processed += 1;
        if self.recent.len() == 16 {
            self.recent.remove(0);
        }
        self.recent.push(stream.to_owned());
    }

    pub(super) fn retain_runtime_speech(&mut self) -> bool {
        self.items.retain(|item| item.runtime_speech);
        !self.items.is_empty()
    }

    pub(super) fn speak(&mut self, text: &str, duration: f32) {
        if text.trim().is_empty() || !duration.is_finite() || duration <= 0. {
            return;
        }
        let stream: String = text
            .chars()
            .take(MAX_CHARACTERS - 1)
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .collect();
        self.record_processed(&stream);
        let settings = Settings::default();
        let lifespan = duration + settings.linger_time;
        self.items.push(Item {
            stream,
            runtime_speech: true,
            time_to_live: lifespan,
            initial_life_span: lifespan,
            added_time: 0.,
            predisplay: settings.predisplay_time,
            valid: true,
            from_player: false,
            work: None,
            width: 0.,
            height: 0.,
        });
    }
}

impl CaptionHud {
    /// Queues a single dialogue cue with Source formatting and a caller-supplied duration.
    /// Unlike authored audio captions, runtime speech is independent of the caption setting.
    pub fn speak(&self, text: &str, duration: f32) {
        self.state.borrow_mut().speak(text, duration);
    }
}
