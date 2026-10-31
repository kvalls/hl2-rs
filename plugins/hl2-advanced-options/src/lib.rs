//! Optional advanced video controls and motion reconstruction.
#[cfg(feature = "bevy")]
pub mod bevy;
pub mod display;
pub mod flexible_frame;
pub mod localize;
pub mod motion_blur;
pub mod settings;
pub const LAYOUT_ADDENDA: &[(&str, &str)] = &[(
    "optionssubvideoadvanceddlg",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/ui/optionssubvideoadvanceddlg_extra.res"
    )),
)];
pub const BLUR_TOKENS: &[&str] = &[
    "#gameui_disabled",
    "#gameui_enabled",
    "#GameUI_SettingAdvanced",
    "#HL2RS_MotionBlurUltra",
];
pub mod video_confirm;

pub mod fov;

mod commands;
pub mod ui;
pub use ui::register;

#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod ui_tests;
