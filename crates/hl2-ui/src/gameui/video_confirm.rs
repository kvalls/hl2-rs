//! Keep-or-revert confirmation after an Options change of aspect ratio. A mode the display
//! cannot show leaves the user unable to see the dialog, so the change reverts on its own unless
//! confirmed: the common desktop and game practice (Windows display settings, GNOME, KDE: a
//! countdown of 10 to 15 seconds, reverting by default). Retail Source has no such dialog.
//!
//! Text comes from the installed game where it has it (`#GameUI_Confirm` title, `#GameUI_OK`,
//! `#GameUI_Cancel`, the `#vgui_TimeLeftSecond(s)Remaining` countdown); the question is a
//! temporary [`placeholders`] token. The timeout is [`crate::tuning::video_confirm_seconds`].
//! OK keeps the change; Cancel, the close button, Escape and the timeout revert it.

use super::display::AspectRatio;
use super::localize::{placeholders, Localization};
use crate::config::{DisplayMode, VideoSettings};
use source_ui::message_box::MessageBox;

/// The window-affecting video settings the confirmation can restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    /// Writes this mode into `video`, leaving the other settings alone.
    pub fn restore(self, video: &mut VideoSettings) {
        video.width = self.width;
        video.height = self.height;
        video.display_mode = self.display_mode;
        video.flexible_aspect = self.flexible_aspect;
    }

    fn aspect(self) -> AspectRatio {
        if self.flexible_aspect {
            AspectRatio::Flexible
        } else {
            AspectRatio::of(self.width, self.height)
        }
    }
}

/// A pending confirmation: the mode to go back to and when.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoConfirm {
    pub previous: VideoMode,
    deadline: f64,
}

impl VideoConfirm {
    /// Whether applying `new` over `old` needs a confirmation: the aspect ratio changed.
    pub fn needed(old: &VideoSettings, new: &VideoSettings) -> bool {
        VideoMode::of(old).aspect() != VideoMode::of(new).aspect()
    }

    /// A confirmation opened at `now` (seconds on the UI clock).
    pub fn new(previous: VideoMode, now: f64) -> Self {
        Self {
            previous,
            deadline: now + crate::tuning::video_confirm_seconds(),
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
    pub fn text(loc: &Localization, remaining: u32) -> String {
        let countdown = if remaining == 1 {
            "#vgui_TimeLeftSecondRemaining"
        } else {
            "#vgui_TimeLeftSecondsRemaining"
        };
        format!(
            "{}\n{}",
            loc.text(placeholders::KEEP_VIDEO_SETTINGS),
            loc.text(countdown).replace("%s1", &remaining.to_string())
        )
    }

    /// The message box at `now`: Cancel (revert) has the keyboard focus, sized for the longest
    /// countdown so it does not change size while counting down.
    pub fn message_box(&self, loc: &Localization, now: f64) -> MessageBox {
        let longest = Self::text(loc, crate::tuning::video_confirm_seconds().ceil() as u32);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn video(width: u32, height: u32) -> VideoSettings {
        VideoSettings {
            width,
            height,
            ..VideoSettings::default()
        }
    }

    #[test]
    fn only_an_aspect_change_needs_confirming() {
        assert!(!VideoConfirm::needed(&video(1280, 720), &video(1920, 1080)));
        assert!(VideoConfirm::needed(&video(1280, 720), &video(1024, 768)));
        assert!(VideoConfirm::needed(&video(1920, 1080), &video(1080, 1920)));
        let flexible = VideoSettings {
            flexible_aspect: true,
            ..video(1920, 1080)
        };
        assert!(VideoConfirm::needed(&video(1920, 1080), &flexible));
        assert!(VideoConfirm::needed(&flexible, &video(1920, 1080)));
        assert!(!VideoConfirm::needed(
            &flexible,
            &VideoSettings {
                flexible_aspect: true,
                ..video(1024, 768)
            }
        ));
    }

    #[test]
    fn the_countdown_reaches_zero_at_the_timeout() {
        let seconds = crate::tuning::video_confirm_seconds();
        let confirm = VideoConfirm::new(VideoMode::of(&video(1280, 720)), 100.);
        assert_eq!(confirm.remaining(100.), seconds.ceil() as u32);
        assert_eq!(confirm.remaining(100. + seconds - 0.5), 1);
        assert!(!confirm.expired(100. + seconds - 0.01));
        assert!(confirm.expired(100. + seconds));
        assert_eq!(confirm.remaining(100. + seconds + 3.), 0);
    }

    #[test]
    fn text_uses_installed_tokens_and_the_placeholder_question() {
        let mut loc = Localization::default();
        loc.insert("#vgui_TimeLeftSecondsRemaining", "%s1 seconds remaining");
        loc.insert("#vgui_TimeLeftSecondRemaining", "%s1 second remaining");
        let text = VideoConfirm::text(&loc, 12);
        assert!(text.ends_with("\n12 seconds remaining"), "{text}");
        assert!(VideoConfirm::text(&loc, 1).ends_with("\n1 second remaining"));
        assert!(text.starts_with(placeholders::lookup(placeholders::KEEP_VIDEO_SETTINGS).unwrap()));
    }

    #[test]
    fn restoring_puts_back_only_the_mode() {
        let previous = VideoMode::of(&video(1280, 720));
        let mut now = video(1080, 1920);
        now.gamma = 1.8;
        now.flexible_aspect = true;
        previous.restore(&mut now);
        assert_eq!((now.width, now.height, now.gamma), (1280, 720, 1.8));
        assert!(!now.flexible_aspect, "the flexible choice reverts too");
    }
}
