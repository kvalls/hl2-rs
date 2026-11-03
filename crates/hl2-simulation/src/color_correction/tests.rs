use super::*;
use modkit_core::Entity;

fn entity(class: &str, keys: &[(&str, &str)]) -> Entity {
    let mut properties = vec![("classname".to_string(), class.to_string())];
    properties.extend(keys.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Entity { properties }
}

fn world(entities: Vec<Entity>) -> World {
    World {
        entities,
        ..World::default()
    }
}

fn point(file: &str, keys: &[(&str, &str)]) -> Entity {
    let mut all = vec![
        ("filename", file),
        ("minfalloff", "-1"),
        ("maxfalloff", "-1"),
    ];
    all.extend_from_slice(keys);
    entity("color_correction", &all)
}

fn run(cc: &mut ColorCorrection, from: f64, to: f64, feet: Vec3) {
    let mut now = from;
    while now < to {
        now += TICK;
        cc.update(now, feet, |_| false);
    }
}

#[test]
fn mix_fills_the_rest_with_the_default_and_normalizes_above_one() {
    let half = Mix::of([0.5, 0.25, 0., 0.]);
    assert_eq!((half.default, half.weights[0]), (0.25, 0.5));
    let over = Mix::of([1., 1., 0., 0.]);
    assert_eq!((over.default, over.weights), (0., [0.5, 0.5, 0., 0.]));
    assert!(!Mix::of([0.; 4]).is_active());
    assert_eq!(Mix::of([0.; 4]).default, 1.);
}

#[test]
fn an_enabled_entity_starts_at_weight_one_whatever_its_maxweight() {
    let mut cc = ColorCorrection::new(&world(vec![point("a.raw", &[("maxweight", ".5")])]));
    cc.update(0.015, Vec3::ZERO, |_| false);
    assert_eq!(cc.mix().weights[0], 1.);
    assert_eq!(cc.mix().default, 0.);
}

#[test]
fn a_start_disabled_entity_has_no_weight_until_enabled() {
    let mut cc = ColorCorrection::new(&world(vec![point(
        "a.raw",
        &[("StartDisabled", "1"), ("maxweight", ".5")],
    )]));
    cc.update(0.015, Vec3::ZERO, |_| false);
    assert!(!cc.mix().is_active());
    assert!(cc.input(0, "enable", 0., 0.015));
    cc.update(0.03, Vec3::ZERO, |_| false);
    assert_eq!(cc.mix().weights[0], 0.5);
    assert_eq!(cc.mix().default, 0.5);
    assert!(cc.input(0, "disable", 0., 0.03));
    cc.update(0.045, Vec3::ZERO, |_| false);
    assert!(!cc.mix().is_active());
}

#[test]
fn fades_take_their_duration() {
    let mut cc = ColorCorrection::new(&world(vec![point(
        "a.raw",
        &[
            ("StartDisabled", "1"),
            ("fadeInDuration", "2"),
            ("fadeOutDuration", "4"),
        ],
    )]));
    cc.input(0, "enable", 0., 0.);
    run(&mut cc, 0., 1., Vec3::ZERO);
    let halfway = cc.mix().weights[0];
    assert!((halfway - 0.5).abs() < 0.02, "{halfway}");
    run(&mut cc, 1., 3., Vec3::ZERO);
    assert_eq!(cc.mix().weights[0], 1.);
    // Fading out from full weight takes the whole duration.
    cc.input(0, "disable", 0., 3.);
    run(&mut cc, 3., 5., Vec3::ZERO);
    let halfway = cc.mix().weights[0];
    assert!((halfway - 0.5).abs() < 0.02, "{halfway}");
    run(&mut cc, 5., 8., Vec3::ZERO);
    assert!(!cc.mix().is_active());
}

#[test]
fn a_fade_in_that_starts_from_a_partial_weight_is_shorter() {
    let mut cc = ColorCorrection::new(&world(vec![point(
        "a.raw",
        &[
            ("StartDisabled", "1"),
            ("fadeInDuration", "4"),
            ("fadeOutDuration", "4"),
        ],
    )]));
    cc.input(0, "enable", 0., 0.);
    run(&mut cc, 0., 2., Vec3::ZERO);
    cc.input(0, "disable", 0., 2.);
    cc.input(0, "enable", 0., 2.);
    // From about 0.5 the remaining half takes 2 s.
    run(&mut cc, 2., 4.1, Vec3::ZERO);
    assert_eq!(cc.mix().weights[0], 1.);
}

#[test]
fn falloff_scales_the_weight_by_the_distance_to_the_player() {
    let mut cc = ColorCorrection::new(&world(vec![entity(
        "color_correction",
        &[
            ("filename", "a.raw"),
            ("origin", "100 0 0"),
            ("minfalloff", "100"),
            ("maxfalloff", "300"),
        ],
    )]));
    let at = |cc: &mut ColorCorrection, x: f32| {
        cc.update(1., Vec3::new(x, 0., 0.), |_| false);
        cc.mix().weights[0]
    };
    assert_eq!(at(&mut cc, 100.), 1.);
    assert_eq!(at(&mut cc, 300.), 0.5);
    assert_eq!(at(&mut cc, 500.), 0.);
    assert!(!cc.mix().is_active());
}

#[test]
fn a_second_entity_with_the_same_file_drives_nothing() {
    let mut cc = ColorCorrection::new(&world(vec![
        point("A.raw", &[("StartDisabled", "1")]),
        point("a.raw", &[]),
        point("b.raw", &[("maxweight", ".5")]),
    ]));
    assert_eq!(cc.lookups(), ["a.raw", "b.raw"]);
    cc.update(0.015, Vec3::ZERO, |_| false);
    // The first a.raw entity is disabled and the enabled duplicate is ignored; b drives its own.
    assert_eq!(cc.mix().weights, [0., 1., 0., 0.]);
}

#[test]
fn only_the_first_four_lookups_are_bound() {
    let entities = (0..5).map(|i| point(&format!("{i}.raw"), &[])).collect();
    let mut cc = ColorCorrection::new(&world(entities));
    assert_eq!(cc.lookups().len(), MAX_LOOKUPS);
    cc.update(0.015, Vec3::ZERO, |_| false);
    assert_eq!(cc.mix().weights, [0.25; 4]);
    assert_eq!(cc.mix().default, 0.);
}

fn volume(keys: &[(&str, &str)]) -> ColorCorrection {
    let mut all = vec![("filename", "v.raw")];
    all.extend_from_slice(keys);
    if !keys.iter().any(|(k, _)| *k == "fadeDuration") {
        all.push(("fadeDuration", "2"));
    }
    ColorCorrection::new(&world(vec![entity("color_correction_volume", &all)]))
}

#[test]
fn a_volume_ramps_in_while_the_player_is_inside_and_out_after() {
    let mut cc = volume(&[]);
    let weight_at = |cc: &mut ColorCorrection, now: f64, inside: bool| {
        cc.update(now, Vec3::ZERO, |_| inside);
        cc.mix().weights[0]
    };
    assert_eq!(weight_at(&mut cc, 1., false), 0.);
    let mut inside = true;
    assert_eq!(weight_at(&mut cc, 1.5, inside), 0.);
    // Entered at 1.5 from 0: weight = dt / fade.
    let w = weight_at(&mut cc, 2.5, inside);
    assert!((w - 0.5).abs() < 1e-5, "{w}");
    assert_eq!(weight_at(&mut cc, 10., inside), 1.);
    // Left at 11 from weight 1: w - dt / (w * fade).
    inside = false;
    weight_at(&mut cc, 11., inside);
    let w = weight_at(&mut cc, 12., inside);
    assert!((w - 0.5).abs() < 1e-5, "{w}");
    assert_eq!(weight_at(&mut cc, 20., inside), 0.);
}

#[test]
fn a_volume_without_a_fade_time_changes_at_once() {
    let mut cc = volume(&[("fadeDuration", "0")]);
    cc.update(1., Vec3::ZERO, |_| true);
    assert_eq!(cc.mix().weights[0], 1.);
    cc.update(2., Vec3::ZERO, |_| false);
    assert_eq!(cc.mix().weights[0], 0.);
}

#[test]
fn a_disabled_volume_has_no_weight() {
    let mut cc = volume(&[("StartDisabled", "1")]);
    cc.update(5., Vec3::ZERO, |_| true);
    assert!(!cc.mix().is_active());
    assert!(cc.input(0, "enable", 0., 5.));
    cc.update(8., Vec3::ZERO, |_| true);
    assert!(cc.mix().is_active());
}

#[test]
fn other_entities_and_inputs_are_not_taken() {
    let mut cc = ColorCorrection::new(&world(vec![
        entity("info_player_start", &[]),
        point("a.raw", &[]),
    ]));
    assert!(!cc.input(0, "enable", 0., 0.));
    assert!(!cc.input(1, "kill", 0., 0.));
    assert!(cc.input(1, "setfadeinduration", 3., 0.));
}

#[test]
#[ignore = "requires an installed owned HL2 copy with Episode One and Two"]
fn owned_maps_register_at_most_four_existing_lookups_of_the_retail_size() {
    let root = source_assets::install::discover().unwrap();
    let mut with_entities = 0;
    for game in ["episodic", "ep2"] {
        let vfs = source_assets::vpk::Vfs::mount_game(&root, game).unwrap();
        for entry in std::fs::read_dir(root.join(game).join("maps")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "bsp")
                || std::fs::metadata(&path).unwrap().len() == 0
            {
                continue;
            }
            let Ok(bsp) = source_assets::bsp::Bsp::parse(&std::fs::read(&path).unwrap()) else {
                continue; // a truncated stub map
            };
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let world = bsp.world(&name).unwrap();
            let cc = ColorCorrection::new(&world);
            assert!(
                cc.lookups().len() <= MAX_LOOKUPS,
                "{game}/{name}: {:?}",
                cc.lookups()
            );
            with_entities += usize::from(!cc.is_empty());
            for file in cc.lookups() {
                let bytes = vfs
                    .read(file)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{game}/{name}: {file}"));
                assert_eq!(bytes.len(), 32 * 32 * 32 * 3, "{file}");
            }
        }
    }
    assert!(with_entities >= 20, "{with_entities}");
}

#[test]
#[ignore = "requires an installed owned HL2 copy"]
fn owned_base_game_maps_have_no_color_correction() {
    let root = source_assets::install::discover().unwrap();
    for map in ["d1_trainstation_01", "d3_breen_01", "background01"] {
        let bsp = source_assets::bsp::Bsp::parse(
            &std::fs::read(root.join(format!("hl2/maps/{map}.bsp"))).unwrap(),
        )
        .unwrap();
        assert!(
            ColorCorrection::new(&bsp.world(map).unwrap()).is_empty(),
            "{map}"
        );
    }
}

#[test]
fn a_scene_delivers_entity_inputs_and_updates_the_weights_each_tick() {
    use crate::entities::Scene;
    let world = world(vec![point("a.raw", &[("targetname", "cc")])]);
    let mut scene = Scene::new(&world);
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert_eq!(scene.color_correction.mix().weights[0], 1.);
    scene.send(0, "Disable", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert!(!scene.color_correction.mix().is_active());
    scene.send(0, "Enable", "");
    scene.tick(&world, Vec3::ZERO, 0.015);
    scene.tick(&world, Vec3::ZERO, 0.015);
    assert_eq!(scene.color_correction.mix().weights[0], 1.);
}
