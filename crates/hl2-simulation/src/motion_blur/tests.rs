use super::*;

const FRAME: f32 = 1. / 60.;

fn view(yaw: f32) -> View {
    View {
        origin: Vec3::new(100., 200., 64.),
        pitch: 0.,
        yaw,
        fov: 90.,
        aspect: 4. / 3.,
    }
}

/// Runs frames at 60 fps from a still view so the previous-frame state is set.
fn settled() -> (MotionBlur, f32) {
    let mut blur = MotionBlur::default();
    let mut now = 10.;
    for _ in 0..3 {
        now += FRAME;
        blur.update(&view(0.), now, &Settings::default());
    }
    (blur, now)
}

#[test]
fn a_still_view_does_not_blur() {
    let (mut blur, now) = settled();
    assert_eq!(
        blur.update(&view(0.), now + FRAME, &Settings::default()),
        [0.; 4]
    );
}

#[test]
fn turning_blurs_by_the_yaw_change_over_the_field_of_view() {
    let (mut blur, now) = settled();
    let values = blur.update(&view(3.), now + FRAME, &Settings::default());
    // 3 degrees of 90 at 15% rotation intensity; yaw 0 -> 3 is a decrease of -3 (previous - current).
    assert!((values[0] - (-3. / 90. * 0.15)).abs() < 1e-5, "{values:?}");
    assert_eq!(values[1], 0.);
    assert_eq!(values[3], 0.);
}

#[test]
fn the_blur_fades_between_50_and_30_fps_and_stops_below_15() {
    let at = |dt: f32| {
        let mut blur = MotionBlur::default();
        blur.update(&view(0.), 10., &Settings::default());
        blur.update(&view(3.), 10. + dt, &Settings::default())[0]
    };
    let full = at(1. / 60.);
    assert!(full < 0.);
    assert!(
        (at(1. / 40.) / full - 0.5).abs() < 0.2 / 1.0,
        "halfway at 40 fps"
    );
    assert_eq!(at(1. / 30.), 0.);
    assert_eq!(at(1. / 10.), 0.);
}

#[test]
fn the_first_frame_and_a_far_jump_after_a_pause_do_not_blur() {
    let mut blur = MotionBlur::default();
    assert_eq!(blur.update(&view(50.), 20., &Settings::default()), [0.; 4]);
    let mut far = view(50.);
    far.origin += Vec3::new(500., 0., 0.);
    assert_eq!(blur.update(&far, 21., &Settings::default()), [0.; 4]);
}

#[test]
fn a_portal_jump_holds_rotation_blur_off_for_a_second() {
    let (mut blur, now) = settled();
    let mut jumped = view(5.);
    jumped.origin += Vec3::new(200., 0., 0.);
    assert_eq!(
        blur.update(&jumped, now + FRAME, &Settings::default()),
        [0.; 4]
    );
    let turned = View { yaw: 8., ..jumped };
    assert_eq!(
        blur.update(&turned, now + 2. * FRAME, &Settings::default())[0],
        0.
    );
    let mut last = [0.; 4];
    for frame in 3..80 {
        let turning = View {
            yaw: 8. + frame as f32,
            ..jumped
        };
        last = blur.update(&turning, now + frame as f32 * FRAME, &Settings::default());
    }
    assert_ne!(last[0], 0., "the hold ended after a second");
}

#[test]
fn looking_down_turns_yaw_blur_into_roll() {
    let mut blur = MotionBlur::default();
    let down = |yaw| View {
        pitch: 90.,
        ..view(yaw)
    };
    blur.update(&down(0.), 10., &Settings::default());
    let values = blur.update(&down(3.), 10. + FRAME, &Settings::default());
    assert_eq!(values[0], 0., "yaw blur is damped to nothing at 90 degrees");
    assert!((values[3] - (-3. / 90. * 0.3)).abs() < 1e-5, "{values:?}");
}

#[test]
fn falling_blurs_above_ten_units_per_thirtieth_of_a_second() {
    let mut blur = MotionBlur::default();
    let looking_down = |z| View {
        pitch: 90.,
        origin: Vec3::new(0., 0., z),
        ..view(0.)
    };
    blur.update(&looking_down(1000.), 10., &Settings::default());
    // 20 units in a sixtieth: 40 per thirtieth (above the maximum of 20), moving down.
    let values = blur.update(&looking_down(980.), 10. + FRAME, &Settings::default());
    assert!(values[2].abs() > 0.);
    assert!((values[2].abs() - 1. / 30.).abs() < 1e-5, "{values:?}");
    // Standing still while looking down does not.
    let still = blur.update(&looking_down(980.), 10. + 2. * FRAME, &Settings::default());
    assert_eq!(still[2], 0.);
}

#[test]
fn strength_scales_every_value_and_zero_strength_removes_them() {
    let (mut blur, now) = settled();
    let mut off = blur;
    let settings = Settings {
        strength: 0.,
        ..Settings::default()
    };
    assert_eq!(off.update(&view(3.), now + FRAME, &settings), [0.; 4]);
    let half = Settings {
        strength: 0.5,
        ..Settings::default()
    };
    let full = blur.update(&view(3.), now + FRAME, &Settings::default())[0];
    let mut again = MotionBlur::default();
    for i in 0..3 {
        again.update(&view(0.), 10. + FRAME * (i + 1) as f32, &half);
    }
    let halved = again.update(&view(3.), now + FRAME, &half)[0];
    assert!((halved - full / 2.).abs() < 1e-6);
}

#[test]
fn samples_follow_the_frame_height_and_vanish_without_blur() {
    assert_eq!(samples(1080, [0.; 4]), 0);
    assert_eq!(samples(2160, [0., 0.1, 0., 0.]), 15);
    assert_eq!(samples(1079, [0., 0., -0.1, 0.]), 11);
    assert_eq!(samples(720, [0.1, 0., 0., 0.]), 11);
    assert_eq!(samples(719, [0.1, 0., 0., 0.]), 7);
}
