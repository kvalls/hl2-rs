//! Host bridge for background debug aspect changes.
pub use crate::bevy::background::{
    BACKGROUND_ADJUST_KEYS, BACKGROUND_OPACITY_KEYS, BackgroundDebug, aspect_label,
};
pub fn aspect_changed(
    debug: &mut BackgroundDebug,
    aspect: f32,
    label: &str,
    console: &mut hl2_ui::console::Console,
) {
    let fov = hl2_bevy::camera::background_fov(
        aspect,
        console.config.video.background_force_default_blur,
    )
    .to_degrees();
    if debug.check_aspect_change(aspect, fov) {
        debug.blur_texture_name =
            console.blur_texture_name(&debug.map_name, aspect > 4. / 3. + 1e-3);
        console.log(format!(
            "Changed aspect ratio to {label}, background FOV is set to {fov:.2}."
        ));
    }
}
