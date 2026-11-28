//! Single speech cues supplied by runtime contributors.
use super::{CaptionHud, CloseCaption, Item, Paint, Settings, MAX_CHARACTERS};

fn advance_clock(clock: &source_ui::canvas::MutexCell<Option<f64>>, time: f64) -> f32 {
    let mut guard = clock.borrow_mut();
    let dt = guard.map_or(0., |last| (time - last).max(0.)) as f32;
    *guard = Some(time);
    dt
}

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

    pub(super) fn has_runtime_speech(&self) -> bool {
        self.items.iter().any(|item| item.runtime_speech)
    }

    /// Advances authored captions on `game_dt` and runtime speech on `runtime_dt`.
    pub(super) fn tick_split(&mut self, game_dt: f32, runtime_dt: f32) {
        self.tick_count += 1;
        for item in &mut self.items {
            let dt = if item.runtime_speech {
                runtime_dt
            } else {
                game_dt
            };
            if item.predisplay > 0. {
                item.predisplay = (item.predisplay - dt).max(0.);
            } else {
                item.time_to_live = (item.time_to_live - dt).max(0.);
            }
        }
        let mut alive_authored = false;
        self.items.retain(|item| {
            if item.predisplay > 0. {
                return true;
            }
            if item.runtime_speech {
                return item.time_to_live > 0.;
            }
            if item.time_to_live > 0. {
                alive_authored = true;
                return true;
            }
            alive_authored
        });
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

    /// Advances authored captions on `game_time` and runtime speech on `real_time`, keeping
    /// paused authored captions queued while only runtime speech renders behind hidden HUDs.
    pub(crate) fn tick_with_clocks(
        &self,
        game_time: f64,
        real_time: f64,
        authored_enabled: bool,
        show_authored: bool,
        allow_runtime_paint: bool,
    ) -> bool {
        let game_dt = advance_clock(&self.clock, game_time);
        let real_dt = advance_clock(&self.runtime_clock, real_time);
        let mut state = self.state.borrow_mut();
        state.tick_split(game_dt, real_dt);
        if !authored_enabled {
            state.retain_runtime_speech();
        }
        if show_authored {
            state.runtime_only_visible = false;
            return authored_enabled
                || state.has_runtime_speech()
                || (state.goal_height > 0. && state.current_alpha > 0.);
        }
        if !allow_runtime_paint {
            state.runtime_only_visible = false;
            return false;
        }
        if state.has_runtime_speech() {
            state.runtime_only_visible = true;
            return true;
        }
        if state.runtime_only_visible {
            if state.goal_height > 0. && state.current_alpha > 0. {
                return true;
            }
            state.runtime_only_visible = false;
        }
        false
    }

    /// Lays out the panel for a canvas `size` in pixels, filtering authored captions when paused.
    pub(crate) fn paint_filtered(
        &self,
        size: (f32, f32),
        show_authored: bool,
        time: f64,
    ) -> Option<(source_ui::canvas::Rect, Paint)> {
        let scale = crate::proportional::scale(glam::vec2(size.0, size.1));
        let measure = self.measure(scale)?;
        let rect = self.panel_rect(size, scale);
        let paint = self.state.borrow_mut().paint_filtered(
            (rect.w, rect.h),
            scale,
            super::paint::CaptionAppearance {
                layout: &self.layout,
                measure: &measure,
                background: self.background,
            },
            show_authored,
            time,
        );
        Some((rect, paint))
    }
}
