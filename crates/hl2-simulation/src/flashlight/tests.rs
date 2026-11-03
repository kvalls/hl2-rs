use super::*;

fn hl2(now: f64) -> Context {
    Context {
        suit: true,
        episodic: false,
        easy: false,
        now,
    }
}

/// Runs `seconds` of 15 ms frames.
fn run(light: &mut Flashlight, from: f64, seconds: f64, context: impl Fn(f64) -> Context) -> f64 {
    let mut now = from;
    while now < from + seconds - 1e-9 {
        now += 0.015;
        light.update(0.015, context(now));
    }
    now
}

#[test]
fn impulse_100_toggles_with_sounds_and_needs_the_suit() {
    let mut light = Flashlight::new(true);
    light.toggle(Context {
        suit: false,
        ..hl2(0.)
    });
    assert!(!light.on, "no suit, no flashlight");
    light.toggle(hl2(0.));
    assert!(light.on && light.suit_power.active());
    light.toggle(hl2(1.));
    assert!(!light.on && !light.suit_power.active());
    assert_eq!(light.sounds, [SOUND_ON, SOUND_OFF]);
    light.set_enabled(false, hl2(2.));
    light.toggle(hl2(2.));
    assert!(!light.on, "disabled");
}

#[test]
fn the_legacy_flashlight_drains_suit_power_in_45_seconds_and_turns_off_below_4_8() {
    let mut light = Flashlight::new(true);
    light.turn_on(hl2(0.));
    let now = run(&mut light, 0., 10., hl2);
    assert!(
        (light.suit_power.power - (100. - 22.22)).abs() < 0.1,
        "{}",
        light.suit_power.power
    );
    let mut now = now;
    while light.on && now < 60. {
        now = run(&mut light, now, 0.015, hl2);
    }
    assert!(!light.on, "off below 4.8 %");
    assert!(light.suit_power.power < LEGACY_CUTOFF);
    assert!((now - 42.9).abs() < 0.2, "about 95.2 / 2.222 s: {now}");
    // Episodic drains half as fast.
    let mut episodic = Flashlight::new(true);
    let ep1 = |now| Context {
        episodic: true,
        ..hl2(now)
    };
    episodic.turn_on(ep1(0.));
    run(&mut episodic, 0., 10., ep1);
    assert!((episodic.suit_power.power - (100. - 11.11)).abs() < 0.1);
}

#[test]
fn suit_power_recharges_half_a_second_after_the_last_device_and_off_costs_a_tenth_of_the_rate() {
    let mut light = Flashlight::new(true);
    light.turn_on(hl2(0.));
    let now = run(&mut light, 0., 4.5, hl2);
    let before = light.suit_power.power;
    light.turn_off(hl2(now));
    assert!((light.suit_power.power - (before - 0.2222)).abs() < 1e-3);
    let after_off = light.suit_power.power;
    let now = run(&mut light, now, 0.45, hl2);
    assert_eq!(
        light.suit_power.power, after_off,
        "no recharge within 0.5 s"
    );
    run(&mut light, now, 1., hl2);
    assert!(light.suit_power.power > after_off + 10.);
}

#[test]
fn episode_two_battery_is_its_own() {
    let mut light = Flashlight::new(false);
    let ep2 = |now| Context {
        episodic: true,
        ..hl2(now)
    };
    light.turn_on(ep2(0.));
    assert!(light.on && !light.suit_power.active());
    let now = run(&mut light, 0., 9., ep2);
    assert!((light.power() - 90.).abs() < 0.1);
    light.turn_off(ep2(now));
    run(&mut light, now, 0.3, ep2);
    assert_eq!(light.power(), 100.);
}

#[test]
fn the_light_sits_below_the_eye_and_pulls_back_from_near_walls() {
    let basis = (Vec3::new(0., 0., 64.), Vec3::X, Vec3::NEG_Y, Vec3::Z);
    let clear: Trace = &|_, _| 1.;
    let mut placer = Placer::default();
    placer.turn_on();
    let light = placer.place(basis, clear, (100., 0.));
    assert!(
        (light.origin.z - 44.).abs() < 1.,
        "20 below the eye: {}",
        light.origin
    );
    assert_eq!(
        (light.fov, light.far, light.attenuation),
        (45., 750., [0., 100., 0.])
    );
    // A wall 50 units ahead: the origin eases back toward 78 units behind it.
    let wall: Trace = &|from, to| {
        if to.x > from.x {
            ((50. - from.x) / (to.x - from.x)).clamp(0., 1.)
        } else {
            1.
        }
    };
    let mut placer = Placer::default();
    let mut light = placer.place(basis, wall, (100., 0.));
    for _ in 0..60 {
        light = placer.place(basis, wall, (100., 0.));
    }
    assert!(light.origin.x < -70., "{}", light.origin);
    assert!(light.near > 70.);
}

#[test]
fn low_power_narrows_and_flickers() {
    assert_eq!(flicker(50., 0.), (LINEAR, FOV));
    let (linear, fov) = flicker(8., 1.);
    assert!((linear - LINEAR).abs() <= 1.5 && fov == FOV);
    let (_, fov) = flicker(1., 1.);
    assert!(fov < FOV - 10.);
}
