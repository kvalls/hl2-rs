//! Additional FOV cvars use the application's shared stored-setting dispatcher.
use hl2_ui::console::{
    Console,
    commands::{Spec, setting},
};
use source_ui::console::command::{Arg, Invalid, parse};
pub const SPECS: [Spec; 3] = [
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
    setting(
        "potato",
        "potato [0/1]: query or set every Potato item; off leaves the combos at Low",
        &[Arg::Free],
        potato,
        set_potato,
    ),
];
fn potato(console: &Console) -> Option<String> {
    let on = crate::settings::potato_budget(&console.config.video.advanced) != Default::default();
    Some(u8::from(on).to_string())
}
fn set_potato(console: &mut Console, value: &str) -> Result<(), Invalid> {
    crate::settings::set_all_potato(&mut console.config.video.advanced, parse::<u8>(value)? != 0);
    Ok(())
}
fn vertical(console: &Console) -> Option<String> {
    Some(format!(
        "{:.0}",
        crate::settings::vertical_fov(&console.config.video)
    ))
}
fn viewmodel(console: &Console) -> Option<String> {
    Some(format!(
        "{:.0}",
        crate::settings::viewmodel_fov(&console.config.video)
    ))
}
fn set_vertical(console: &mut Console, value: &str) -> Result<(), Invalid> {
    let value = parse::<f32>(value)?;
    if !value.is_finite() {
        return Err(Invalid::Usage);
    }
    let degrees = if value <= 0. {
        0.
    } else {
        value.clamp(40., 100.)
    };
    crate::settings::set_vertical_fov(&mut console.config.video, degrees);
    Ok(())
}
fn set_viewmodel(console: &mut Console, value: &str) -> Result<(), Invalid> {
    let value = parse::<f32>(value)?;
    if !value.is_finite() {
        return Err(Invalid::Usage);
    }
    crate::settings::set_viewmodel_fov(&mut console.config.video, value.clamp(40., 80.));
    Ok(())
}
