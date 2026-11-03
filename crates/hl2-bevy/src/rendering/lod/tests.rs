use super::*;

#[test]
fn the_metric_grows_with_depth_and_field_of_view_and_shrinks_with_resolution() {
    // 90 degrees vertical: tan 45 = 1. 768 lines, 384 units deep: 100 * 384 / 768 = 50.
    assert!((metric(384., 1., 768.) - 50.).abs() < 1e-4);
    // Twice the lines: a finer level at the same distance (retail measures pixels).
    assert!((metric(384., 1., 1536.) - 25.).abs() < 1e-4);
    // Behind the camera or at the eye: the material system's 0.001 guard (finest level).
    assert!(metric(-50., 1., 768.) < 1e-3);
    assert_eq!(metric(-50., 1., 768.), metric(0., 1., 768.));
}

#[test]
fn the_level_is_the_last_whose_switch_point_the_metric_reached() {
    // Barney's levels: switch points 0, 10, 20, 30, 40 (as authored in the model).
    let switches = [0., 10., 20., 30., 40.];
    assert_eq!(level_for(&switches, 0.), 0);
    assert_eq!(level_for(&switches, 9.99), 0);
    assert_eq!(
        level_for(&switches, 10.),
        1,
        "a switch point reached switches"
    );
    assert_eq!(level_for(&switches, 25.), 2);
    assert_eq!(level_for(&switches, 1000.), 4);
    assert_eq!(level_for(&[0.], 1000.), 0, "a single level");
    // From a root LOD of 2 (model detail Low) the levels start there.
    assert_eq!(level_for(&[0., 30., 40.], 25.), 0);
    assert_eq!(level_for(&[0., 30., 40.], 35.), 1);
}
