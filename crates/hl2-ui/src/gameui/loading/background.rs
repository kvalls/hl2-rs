//! Authored background aspect metadata and shared aspect-fill drawing.

/// A selected blur image, retaining its actual variant even when a wide lookup falls back.
#[derive(Clone, Copy)]
pub struct BackgroundTexture<'a> {
    pub texture: &'a source_ui::canvas::Texture2D,
    pub widescreen: bool,
}

/// Center-crop against the artwork's authored 4:3 or 16:9 aspect, independent of
/// its encoded bitmap dimensions and the target canvas. Returns texture-pixel bounds.
pub fn background_source(
    canvas: glam::Vec2,
    texture: glam::Vec2,
    widescreen: bool,
) -> source_ui::canvas::Rect {
    let aspect = canvas.x / canvas.y.max(1.);
    let base_aspect = if widescreen { 16. / 9. } else { 4. / 3. };
    if aspect < base_aspect {
        let kept = aspect / base_aspect;
        source_ui::canvas::Rect::new(
            texture.x * (1. - kept) * 0.5,
            0.,
            texture.x * kept,
            texture.y,
        )
    } else if aspect > base_aspect {
        let kept = base_aspect / aspect;
        source_ui::canvas::Rect::new(
            0.,
            texture.y * (1. - kept) * 0.5,
            texture.x,
            texture.y * kept,
        )
    } else {
        source_ui::canvas::Rect::new(0., 0., texture.x, texture.y)
    }
}

/// Draws a pre-blurred background texture over the whole canvas ([`background_source`]).
pub fn draw_background_texture(canvas: &source_ui::canvas::Canvas, texture: BackgroundTexture<'_>) {
    draw_background_texture_alpha(canvas, texture, 1.0);
}

/// Draws a pre-blurred background texture with custom alpha opacity over the canvas.
pub fn draw_background_texture_alpha(
    canvas: &source_ui::canvas::Canvas,
    texture: BackgroundTexture<'_>,
    alpha: f32,
) {
    let size = glam::vec2(canvas.width(), canvas.height());
    let color = source_ui::canvas::Color::new(1., 1., 1., alpha.clamp(0., 1.));
    source_ui::canvas::draw_texture_ex(
        canvas,
        texture.texture,
        0.,
        0.,
        color,
        source_ui::canvas::DrawTextureParams {
            source: Some(background_source(
                size,
                texture.texture.size(),
                texture.widescreen,
            )),
            dest_size: Some(size),
        },
    );
}
