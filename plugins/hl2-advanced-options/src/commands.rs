//! Additional FOV cvars use the application's shared stored-setting dispatcher.
use hl2_ui::console::{
    Console,
    commands::{Spec, setting},
};
use source_ui::console::command::{Arg, Invalid, parse};
pub const SPECS: [Spec; 2] = [
    setting(
        "vertical_fov",
        "vertical_fov [degrees]: query or set vertical field of view (0 = automatic)",
        &[Arg::Free],
        vertical,
        set_vertical,
    ),
    setting(
        "viewmodel_fov",
        "viewmodel_fov [degrees]: query or set viewmodel field of view",
        &[Arg::Free],
        viewmodel,
        set_viewmodel,
    ),
];
fn vertical(console: &Console) -> Option<String> {
    Some(format!("{:.0}", console.config.video.vertical_fov))
}
fn viewmodel(console: &Console) -> Option<String> {
    Some(format!("{:.0}", console.config.video.viewmodel_fov))
}
fn set_vertical(console: &mut Console, value: &str) -> Result<(), Invalid> {
    let value = parse::<f32>(value)?;
    if !value.is_finite() {
        return Err(Invalid::Usage);
    }
    console.config.video.vertical_fov = if value <= 0. {
        0.
    } else {
        value.clamp(40., 100.)
    };
    Ok(())
}
fn set_viewmodel(console: &mut Console, value: &str) -> Result<(), Invalid> {
    let value = parse::<f32>(value)?;
    if !value.is_finite() {
        return Err(Invalid::Usage);
    }
    console.config.video.viewmodel_fov = value.clamp(40., 80.);
    Ok(())
}
