//! Video playback integration for the developer console and GameUI host.

use super::{Console, Effect};
use crate::gameui::video::VideoPanelState;
use source_assets::bink::BinkHeader;

impl Console {
    /// Begins full-screen modal video playback.
    pub fn start_video_playback(
        &mut self,
        filename: String,
        exit_command: Option<String>,
        header: Option<&BinkHeader>,
        width: Option<u32>,
        height: Option<u32>,
    ) {
        self.video_panel = Some(VideoPanelState::new(
            filename,
            exit_command,
            header,
            width,
            height,
        ));
    }

    /// Whether a full-screen video is currently actively playing.
    pub fn is_video_playing(&self) -> bool {
        self.video_panel.as_ref().is_some_and(|v| !v.finished)
    }

    /// Skips active video playback and submits its exit command if any, returning the resulting effects.
    pub fn skip_video(&mut self) -> Option<Vec<Effect>> {
        let video = self.video_panel.as_mut()?;
        video.skip();
        let cmd = video.exit_command.clone();
        self.video_panel = None;
        if let Some(cmd) = cmd {
            Some(self.submit(&cmd))
        } else {
            Some(Vec::new())
        }
    }

    /// Advances video playback by `dt` seconds, returning exit command effects upon completion.
    pub fn tick_video(&mut self, dt: f32) -> Vec<Effect> {
        if let Some(video) = &mut self.video_panel {
            if video.tick(dt) {
                let cmd = video.exit_command.clone();
                self.video_panel = None;
                if let Some(cmd) = cmd {
                    return self.submit(&cmd);
                }
            }
        }
        Vec::new()
    }

    /// Draws the modal video panel if active. Returns true if video was drawn.
    pub fn draw_video(&self) -> bool {
        if let Some(video) = &self.video_panel {
            video.draw(&self.canvas, self.canvas_size());
            true
        } else {
            false
        }
    }

    /// Replay / telemetry report of the active video state.
    pub fn video_report(&self) -> serde_json::Value {
        match &self.video_panel {
            Some(v) => serde_json::json!({
                "active": !v.finished,
                "filename": v.filename,
                "exit_command": v.exit_command,
                "width": v.width,
                "height": v.height,
                "duration": v.duration,
                "elapsed": v.elapsed,
                "finished": v.finished,
            }),
            None => serde_json::Value::Null,
        }
    }
}
