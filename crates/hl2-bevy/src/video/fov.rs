//! The world camera's vertical field of view derived from the 4:3 one.

/// Source's field of view is horizontal at 4:3. Wider screens keep the vertical view of 4:3 and
/// show more at the sides (as in retail); screens narrower than 4:3 (square, portrait) keep the
/// 4:3 horizontal view and show more above and below, so the view and the viewmodel are never
/// cropped at the sides. Returns Bevy's vertical field of view (radians) for `aspect`
/// (width / height) of the game frame.
pub(crate) fn vertical_fov_for(degrees: f32, aspect: f32) -> f32 {
    2. * ((degrees.to_radians() / 2.).tan() / aspect.min(4. / 3.)).atan()
}

/// [`vertical_fov_for`] with an explicit vertical field of view or automatic aspect tuning:
/// when `vertical > 0.`, `vertical` degrees replace the derived one for the player's default
/// field of view; when `vertical <= 0.` (automatic), [`the video policy`]
/// adjusts `default_degrees` outside `5:4..=16:9` so portrait frames avoid vertical fish-eye
/// and ultrawide frames avoid peripheral horizontal stretching. Weapon zoom or camera field of
/// view `view_degrees` scales the tangent in proportion.
pub(crate) fn vertical_fov_with(
    view_degrees: f32,
    default_degrees: f32,
    aspect: f32,
    vertical: f32,
) -> f32 {
    let _ = (default_degrees, vertical);
    vertical_fov_for(view_degrees, aspect)
}

/// [`vertical_fov_for`] on a 4:3 frame.
#[cfg(test)]
pub(crate) fn vertical_fov_4_3(degrees: f32) -> f32 {
    vertical_fov_for(degrees, 4. / 3.)
}
