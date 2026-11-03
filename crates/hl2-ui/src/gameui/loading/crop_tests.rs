use super::{background_source, draw_background_texture, BackgroundTexture};
use glam::vec2;
use source_ui::canvas::{Canvas, Rect, Texture2D};

#[test]
fn normal_blur_crops_top_and_bottom_on_a_wide_frame() {
    assert_eq!(
        background_source(vec2(1280., 720.), vec2(512., 512.), false),
        Rect::new(0., 64., 512., 384.)
    );
    assert_eq!(
        background_source(vec2(2560., 720.), vec2(512., 512.), false),
        Rect::new(0., 160., 512., 192.)
    );
    assert_eq!(
        background_source(vec2(2560., 720.), vec2(512., 512.), true),
        Rect::new(0., 128., 512., 256.)
    );
}

#[test]
fn both_artwork_variants_fill_without_distortion_and_keep_equal_margins() {
    for wide in [false, true] {
        let authored = if wide { 16. / 9. } else { 4. / 3. };
        for aspect in [
            9. / 16.,
            1.,
            5. / 4.,
            4. / 3.,
            16. / 10.,
            16. / 9.,
            21. / 9.,
            32. / 9.,
        ] {
            for size in [vec2(512., 512.), vec2(1024., 512.), vec2(800., 600.)] {
                let crop = background_source(vec2(720. * aspect, 720.), size, wide);
                assert!(crop.x >= 0. && crop.y >= 0. && crop.w <= size.x && crop.h <= size.y);
                assert!((2. * crop.x + crop.w - size.x).abs() < 1e-3);
                assert!((2. * crop.y + crop.h - size.y).abs() < 1e-3);
                assert!((authored * (crop.w / size.x) / (crop.h / size.y) - aspect).abs() < 1e-5);
            }
        }
    }
}

#[test]
fn normal_artwork_draws_the_cropped_span_over_the_entire_wide_canvas() {
    let texture = Texture2D::from_rgba8(2, 2, &[255; 16]);
    let canvas = Canvas::default();
    canvas.resize(1280., 720.);
    draw_background_texture(
        &canvas,
        BackgroundTexture {
            texture: &texture,
            widescreen: false,
        },
    );
    let quads = canvas.drain();
    assert_eq!(quads.len(), 1);
    assert_eq!(quads[0].destination, Rect::new(0., 0., 1280., 720.));
    assert_eq!(quads[0].source, Rect::new(0., 0.25, 2., 1.5));
}

#[test]
fn loading_overlay_preserves_normal_artwork_crop_on_wide_frames() {
    let texture = Texture2D::from_rgba8(2, 2, &[255; 16]);
    let canvas = Canvas::default();
    canvas.resize(1280., 720.);
    let dialog = super::LoadingDialog::new_background("background01");
    dialog.draw_background_overlay(
        &canvas,
        Some(BackgroundTexture {
            texture: &texture,
            widescreen: false,
        }),
    );
    let quads = canvas.drain();
    assert_eq!(quads.len(), 2);
    assert_eq!(quads[0].source, Rect::new(0., 0.25, 2., 1.5));
    assert_eq!(quads[0].destination, Rect::new(0., 0., 1280., 720.));
}
