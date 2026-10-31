//! Optional aspect-change keep/revert policy using the shared MessageBox.
use source_ui::{
    localize::{Fallback, Localization},
    message_box::MessageBox,
};

pub trait AspectMode {
    fn aspect_id(&self) -> u8;
}
/// A pending confirmation: the mode to go back to and when.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoConfirm<M> {
    pub previous: M,
    deadline: f64,
}

impl<M> VideoConfirm<M> {
    /// Whether applying `new` over `old` needs a confirmation: the aspect ratio changed.
    pub fn needed<T: AspectMode>(old: &T, new: &T) -> bool {
        old.aspect_id() != new.aspect_id()
    }

    /// A confirmation opened at `now` (seconds on the UI clock).
    pub fn new(previous: M, now: f64) -> Self {
        Self {
            previous,
            deadline: now + crate::fov::video_confirm_seconds(),
        }
    }

    /// Whole seconds left at `now`, rounded up (0 once expired).
    pub fn remaining(&self, now: f64) -> u32 {
        (self.deadline - now).max(0.).ceil() as u32
    }

    pub fn expired(&self, now: f64) -> bool {
        now >= self.deadline
    }

    /// The question and the countdown line for `remaining` seconds.
    pub fn text<F: Fallback>(loc: &Localization<F>, remaining: u32) -> String {
        let countdown = if remaining == 1 {
            "#vgui_TimeLeftSecondRemaining"
        } else {
            "#vgui_TimeLeftSecondsRemaining"
        };
        format!(
            "{}\n{}",
            loc.text("#HL2RS_KeepVideoSettings"),
            loc.text(countdown).replace("%s1", &remaining.to_string())
        )
    }

    /// The message box at `now`: Cancel (revert) has the keyboard focus, sized for the longest
    /// countdown so it does not change size while counting down.
    pub fn message_box<F: Fallback>(&self, loc: &Localization<F>, now: f64) -> MessageBox {
        let longest = Self::text(loc, crate::fov::video_confirm_seconds().ceil() as u32);
        MessageBox::new(
            loc.text("#GameUI_Confirm"),
            Self::text(loc, self.remaining(now)),
            loc.text("#GameUI_OK"),
            Some(loc.text("#GameUI_Cancel")),
        )
        .with_close(true)
        .with_focus(1)
        .with_layout_text(longest)
    }
}

use hl2_ui::{
    config::{Config, DisplayMode, VideoSettings},
    console::{Console, Effect},
    extensions::{Confirmation, ConfirmationHandler},
    gameui::localize::Localization as UiLocalization,
};
#[derive(Clone, Copy, Debug)]
pub struct VideoMode {
    pub width: u32,
    pub height: u32,
    pub display_mode: DisplayMode,
    pub flexible_aspect: bool,
}
impl VideoMode {
    pub fn of(video: &VideoSettings) -> Self {
        Self {
            width: video.width,
            height: video.height,
            display_mode: video.display_mode,
            flexible_aspect: video.flexible_aspect,
        }
    }
    pub fn restore(self, video: &mut VideoSettings) {
        video.width = self.width;
        video.height = self.height;
        video.display_mode = self.display_mode;
        video.flexible_aspect = self.flexible_aspect;
    }
}
impl AspectMode for VideoSettings {
    fn aspect_id(&self) -> u8 {
        if self.effective_flexible_aspect() {
            hl2_ui::gameui::display::AspectRatio::Flexible as u8
        } else {
            self.policy.0.aspect_of(self.width, self.height) as u8
        }
    }
}
impl ConfirmationHandler for VideoConfirm<VideoMode> {
    fn kind(&self) -> &'static str {
        "keep_video"
    }
    fn message(&self, loc: &UiLocalization, now: f64) -> MessageBox {
        self.message_box(loc, now)
    }
    fn expired(&self, now: f64) -> bool {
        self.expired(now)
    }
    fn cancel(&self, console: &mut Console) -> Vec<Effect> {
        let mut config = console.config.clone();
        self.previous.restore(&mut config.video);
        let effect = console.replace_config(config);
        console.log(format!(
            "Video mode reverted to {}x{}",
            self.previous.width, self.previous.height
        ));
        vec![effect]
    }
}
pub fn after_apply(console: &mut Console, previous: &Config) {
    if VideoConfirm::<VideoMode>::needed(&previous.video, &console.config.video) {
        let keep = VideoConfirm::new(VideoMode::of(&previous.video), console.clock());
        console.open_extension_confirmation(Confirmation(std::sync::Arc::new(keep)));
    }
}
