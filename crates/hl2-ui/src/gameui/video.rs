//! Modal full-screen video playback (`playvideo` / `playvideo_exitcommand`).
//!
//! Replicates retail `VideoPanel` behavior (Source SDK `game/client/vgui_video.cpp`):
//! - Full-screen modal blackout.
//! - Aspect-ratio preservation with centered letterboxing/pillarboxing.
//! - Skip on `Escape`, `Space`, `Enter`, or `~` (backquote).
//! - Automatic exit command dispatch upon completion or skip.

use glam::Vec2;
use source_assets::bink::BinkHeader;
use source_ui::canvas::{Canvas, Color, Rect};

/// State of an active full-screen video playback.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoPanelState {
    pub filename: String,
    pub exit_command: Option<String>,
    pub width: u32,
    pub height: u32,
    pub duration: f32,
    pub elapsed: f32,
    pub finished: bool,
}

impl VideoPanelState {
    pub fn new(
        filename: String,
        exit_command: Option<String>,
        header: Option<&BinkHeader>,
        width_override: Option<u32>,
        height_override: Option<u32>,
    ) -> Self {
        let (width, height, duration) = match header {
            Some(h) => (
                width_override.unwrap_or(h.width),
                height_override.unwrap_or(h.height),
                h.duration(),
            ),
            None => (
                width_override.unwrap_or(1280),
                height_override.unwrap_or(720),
                10.0,
            ),
        };
        Self {
            filename,
            exit_command,
            width: width.max(1),
            height: height.max(1),
            duration: duration.max(0.1),
            elapsed: 0.0,
            finished: false,
        }
    }

    /// Advances the playback clock by `dt` seconds. Returns true if finished.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.finished {
            return true;
        }
        self.elapsed += dt;
        if self.elapsed >= self.duration {
            self.finished = true;
        }
        self.finished
    }

    /// Explicitly skips or stops the video playback.
    pub fn skip(&mut self) {
        self.finished = true;
    }

    /// Computes the letterboxed or pillarboxed rect centered on the canvas.
    pub fn aspect_rect(&self, canvas: Vec2) -> Rect {
        let fl_frame_ratio = canvas.x / canvas.y.max(1.0);
        let fl_video_ratio = (self.width as f32) / (self.height as f32).max(1.0);
        let (w, h) = if fl_video_ratio > fl_frame_ratio {
            (canvas.x, canvas.x / fl_video_ratio)
        } else if fl_video_ratio < fl_frame_ratio {
            (canvas.y * fl_video_ratio, canvas.y)
        } else {
            (canvas.x, canvas.y)
        };
        let x = (canvas.x - w) / 2.0;
        let y = (canvas.y - h) / 2.0;
        Rect::new(x, y, w, h)
    }

    /// Draws the modal blackout and the centered video frame.
    pub fn draw(&self, canvas: &Canvas, canvas_size: Vec2) {
        // Solid black background covering the full canvas
        canvas.screen_rectangle(Color::from_rgba(0, 0, 0, 255));
        // Letterboxed / pillarboxed video viewport
        let rect = self.aspect_rect(canvas_size);
        canvas.rectangle(
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            Color::from_rgba(5, 5, 5, 255),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_rect_letterboxes_when_screen_taller() {
        // 16:9 video on a 4:3 screen (1024x768) -> letterbox (black bars top & bottom)
        let state =
            VideoPanelState::new("media/ep1_recap".into(), None, None, Some(1280), Some(720));
        let canvas = Vec2::new(1024.0, 768.0);
        let rect = state.aspect_rect(canvas);
        assert_eq!(rect.x, 0.0);
        assert_eq!(rect.w, 1024.0);
        // 1024 / (16/9) = 576
        assert!((rect.h - 576.0).abs() < 0.01);
        assert!((rect.y - (768.0 - 576.0) / 2.0).abs() < 0.01);
    }

    #[test]
    fn aspect_rect_pillarboxes_when_screen_wider() {
        // 4:3 video on a 16:9 screen (1920x1080) -> pillarbox (black bars left & right)
        let state = VideoPanelState::new("media/valve".into(), None, None, Some(1024), Some(768));
        let canvas = Vec2::new(1920.0, 1080.0);
        let rect = state.aspect_rect(canvas);
        assert_eq!(rect.y, 0.0);
        assert_eq!(rect.h, 1080.0);
        // 1080 * (4/3) = 1440
        assert!((rect.w - 1440.0).abs() < 0.01);
        assert!((rect.x - (1920.0 - 1440.0) / 2.0).abs() < 0.01);
    }

    #[test]
    fn tick_and_skip_lifecycle() {
        let mut state = VideoPanelState::new(
            "test".into(),
            Some("map ep2_outland_01".into()),
            None,
            None,
            None,
        );
        assert!(!state.finished);
        assert!(!state.tick(5.0));
        assert_eq!(state.elapsed, 5.0);
        assert!(state.tick(6.0));
        assert!(state.finished);

        let mut skippable = VideoPanelState::new("test".into(), None, None, None, None);
        skippable.skip();
        assert!(skippable.finished);
    }
}
