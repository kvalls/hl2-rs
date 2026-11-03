use super::frame::{Frame, Placement};
use super::receivers::{Cast, Decal, Receivers, Scratch};
use super::*;
use glam::{Quat, Vec2};
use modkit_core::{Entity, Surface, Vertex};

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

fn close(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() < 1e-3, "{what}: {a} vs {b}");
}

fn close3(a: Vec3, b: Vec3, what: &str) {
    assert!((a - b).length() < 1e-3, "{what}: {a} vs {b}");
}

#[test]
fn shadow_control_reads_angles_colour_distance_and_the_level_defaults() {
    let control = |keys: &[(&str, &str)]| {
        ShadowControl::from_world(&world(vec![entity("shadow_control", keys)]))
    };
    let sun = control(&[
        ("angles", "60 43 0"),
        ("color", "159 168 181"),
        ("distance", "75"),
    ]);
    close3(
        sun.direction,
        Vec3::new(0.36568, 0.34100, -0.86603),
        "angles",
    );
    assert_eq!(sun.color, [159, 168, 181]);
    close(sun.distance, 75., "distance");
    assert!(!sun.disabled);
    let zero = control(&[("angles", "0 0 0")]);
    close3(
        zero.direction,
        Vec3::new(0.15038, 0.08682, -0.98481),
        "all-zero angles",
    );
    assert_eq!((zero.color, zero.distance), ([64; 3], 50.));
    let legacy = control(&[("direction", "0 0 -3"), ("disableallshadows", "1")]);
    close3(legacy.direction, Vec3::NEG_Z, "legacy direction");
    assert!(legacy.disabled);
    let none = ShadowControl::from_world(&World::default());
    close3(
        none.direction,
        Vec3::new(0.09901, 0.09901, -0.99015),
        "level default",
    );
    assert_eq!((none.color, none.distance), ([76; 3], 50.));
}

#[test]
fn shadow_colour_comes_from_the_ambient_light_and_shades_in_linear_space() {
    assert_eq!(
        ambient_shadow_color(Vec3::new(0.1, 0.2, 0.3)),
        [153, 229, 255]
    );
    close(linear_color([159; 3])[0], 0.35373, "gamma to linear");
    assert_eq!(linear_color([243; 3])[0], 1.);
}

/// An axis-aligned 32 x 32 x 48 box standing on the origin.
fn crate_box() -> ((Vec3, Vec3), Placement) {
    (
        (Vec3::new(-16., -16., 0.), Vec3::new(16., 16., 48.)),
        Placement::new(Vec3::ZERO, Quat::IDENTITY),
    )
}

#[test]
fn render_to_texture_frame_straight_down_starts_at_the_box_top() {
    let (bounds, at) = crate_box();
    let f = Frame::render_to_texture(bounds, &at, Vec3::NEG_Z, 50.);
    close3(f.origin, Vec3::new(0., 0., 48.), "origin");
    close(f.falloff_start, 56.985, "falloff start");
    close(f.max_height, 106.985, "end");
    assert_eq!(f.size, Vec2::splat(36.));
    assert_eq!(f.clip.len(), 3);
    close(
        Frame::render_to_texture(bounds, &at, Vec3::NEG_Z, 75.).max_height,
        131.985,
        "end at 75",
    );
    close3(
        f.uv(Vec3::new(0., 0., 0.)).extend(0.),
        Vec3::new(0.5, 0.5, 0.),
        "centre",
    );
}

#[test]
fn blob_frame_is_bloated_backed_up_twice_and_snapped() {
    let (bounds, _) = crate_box();
    let at = Placement::new(Vec3::new(0.3, 0.7, 0.2), Quat::IDENTITY);
    let f = Frame::blob(bounds, &at, Vec3::NEG_Z, 50.);
    assert_eq!(f.size, Vec2::splat(42.));
    close3(f.origin, Vec3::new(0., 0.5, 72.), "origin");
    close(f.falloff_start, 80.985, "falloff start");
    assert!(f.clip.is_empty());
}

#[test]
fn fade_ramps_to_240_at_the_end_then_to_255_beyond() {
    let (bounds, at) = crate_box();
    let f = Frame::render_to_texture(bounds, &at, Vec3::NEG_Z, 50.);
    let span = f.max_height - f.falloff_start;
    assert_eq!(f.fade(f.falloff_start - 10., 0), 0);
    assert_eq!(f.fade(f.falloff_start + span * 0.5, 0), 120);
    // Truncated like the engine's: the float product lands a hair under 240 here.
    assert!((239..=240).contains(&f.fade(f.max_height, 0)));
    assert_eq!(f.fade(f.max_height + 0.1, 0), 240);
    assert_eq!(f.fade(f.falloff_start + span * 1.1, 0), 255);
    assert_eq!(f.fade(f.falloff_start, 20), 20);
}

fn vertex(position: Vec3, normal: Vec3, prop: bool) -> Vertex {
    Vertex {
        position,
        uv: Vec2::ZERO,
        color: [255; 4],
        light_uv: Vec2::ZERO,
        skin: prop.then_some(modkit_core::animation::Weights {
            bones: [0; 3],
            weights: [1., 0., 0.],
        }),
        normal,
    }
}

/// A square of half-size `half` at height `z` facing `normal`, two triangles.
fn square(z: f32, half: f32, normal: Vec3, prop: bool) -> Surface {
    let corners = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)];
    Surface {
        background: false,
        material: "concrete/floor".into(),
        vertices: corners
            .iter()
            .map(|(x, y)| vertex(Vec3::new(x * half, y * half, z), normal, prop))
            .collect(),
        indices: vec![0, 1, 2, 0, 2, 3],
        lightmap: None,
        flex_source: None,
    }
}

fn area(decal: &Decal) -> f32 {
    decal
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| decal.positions[i as usize]);
            (b - a).cross(c - a).length() * 0.5
        })
        .sum()
}

fn decal_with(surfaces: &[Surface], cast: Cast) -> Decal {
    let (bounds, at) = crate_box();
    let f = Frame::render_to_texture(bounds, &at, Vec3::NEG_Z, 50.);
    let receivers = Receivers::new(surfaces, |_| true);
    let mut decal = Decal::default();
    receivers.decal(&f, cast, &mut Scratch::default(), &mut decal);
    decal
}

fn decal_on(surfaces: &[Surface], static_props: bool) -> Decal {
    decal_with(
        surfaces,
        Cast {
            static_props,
            ..Cast::default()
        },
    )
}

#[test]
fn a_floor_under_the_caster_gets_the_clipped_square_unfaded() {
    let decal = decal_on(&[square(0., 100., Vec3::Z, false)], false);
    // The texture square is 36 units; the box's two light-side faces with no light on them
    // (`ComputeExtraClipPlanes`) cut the bloat on their side: 34 x 34.
    close(area(&decal), 34. * 34., "area");
    assert!(decal.fades.iter().all(|f| *f == 0));
    assert!(decal
        .uvs
        .iter()
        .all(|uv| (0.0..=1.).contains(&uv.x) && (0.0..=1.).contains(&uv.y)));
    assert_eq!(decal.positions.len(), decal.uvs.len());
    assert!(decal.normals.iter().all(|n| *n == Vec3::Z));
}

#[test]
fn the_decal_sits_its_lift_off_the_receiver_with_the_bias_in_its_fade() {
    let cast = Cast {
        static_props: false,
        bias: 30,
        lift: 0.25,
    };
    let decal = decal_with(&[square(0., 100., Vec3::Z, false)], cast);
    assert!(!decal.is_empty());
    assert!(decal.positions.iter().all(|p| (p.z - 0.25).abs() < 1e-5));
    assert!(decal.fades.iter().all(|f| *f == 30));
}

#[test]
fn surfaces_facing_away_above_or_out_of_reach_receive_nothing() {
    assert!(
        decal_on(&[square(0., 100., Vec3::NEG_Z, false)], false).is_empty(),
        "ceiling"
    );
    assert!(
        decal_on(&[square(60., 100., Vec3::Z, false)], false).is_empty(),
        "floor above the origin"
    );
    assert!(
        decal_on(&[square(-200., 100., Vec3::Z, false)], false).is_empty(),
        "past the end"
    );
    // 103 units down: 240 * (103 - 56.985) / 50.
    let far = decal_on(&[square(-55., 100., Vec3::Z, false)], false);
    assert!(
        !far.is_empty() && far.fades.iter().all(|f| *f == 220),
        "{:?}",
        far.fades
    );
}

#[test]
fn static_props_receive_only_when_asked() {
    let prop = [square(0., 100., Vec3::Z, true)];
    assert!(decal_on(&prop, false).is_empty());
    close(area(&decal_on(&prop, true)), 34. * 34., "prop area");
}

#[test]
fn materials_that_do_not_receive_and_background_surfaces_are_left_out() {
    let (bounds, at) = crate_box();
    let f = Frame::render_to_texture(bounds, &at, Vec3::NEG_Z, 50.);
    let mut sky = square(0., 100., Vec3::Z, false);
    sky.background = true;
    let surfaces = [square(0., 100., Vec3::Z, false), sky];
    let receivers = Receivers::new(&surfaces, |m| m != "concrete/floor");
    assert!(receivers.is_empty());
    let receivers = Receivers::new(&surfaces, |_| true);
    assert_eq!(receivers.len(), 2);
    let mut decal = Decal::default();
    receivers.decal(&f, Cast::default(), &mut Scratch::default(), &mut decal);
    assert!(!decal.is_empty());
}

#[test]
fn a_slanted_shadow_reaches_a_wall_behind_the_caster() {
    // Light from -X toward +X and down; a wall at x = 30 facing -X.
    let d = Vec3::new(1., 0., -1.).normalize();
    let (bounds, at) = crate_box();
    let f = Frame::render_to_texture(bounds, &at, d, 50.);
    let wall = Surface {
        background: false,
        material: "brick/wall".into(),
        vertices: [(-100., 0.), (100., 0.), (100., 100.), (-100., 100.)]
            .iter()
            .map(|(y, z)| vertex(Vec3::new(30., *y, *z), Vec3::NEG_X, false))
            .collect(),
        indices: vec![0, 1, 2, 0, 2, 3],
        lightmap: None,
        flex_source: None,
    };
    let receivers = Receivers::new([&wall], |_| true);
    let mut decal = Decal::default();
    receivers.decal(&f, Cast::default(), &mut Scratch::default(), &mut decal);
    assert!(area(&decal) > 100.);
    assert!(decal.positions.iter().all(|p| (p.x - 30.).abs() < 1e-3));
}

#[test]
fn cell_sizes_follow_twice_the_largest_dimension() {
    use super::silhouette::cell_size;
    let size = |x: f32, y: f32, z: f32| cell_size((Vec3::ZERO, Vec3::new(x, y, z)));
    assert_eq!(size(30., 30., 10.), 64);
    assert_eq!(size(28., 28., 46.), 128);
    assert_eq!(size(32., 32., 72.), 256);
    assert_eq!(size(6., 6., 8.), 16);
    assert_eq!(size(32., 32., 300.), 256);
}

#[test]
fn a_silhouette_covers_the_caster_and_saturates_where_parts_overlap() {
    use super::silhouette::{clear, draw};
    let (bounds, at) = crate_box();
    let f = Frame::render_to_texture(bounds, &at, Vec3::NEG_Z, 50.);
    let n = 36;
    let mut cell = Vec::new();
    clear(&mut cell, n);
    // The box's top face, two triangles, drawn twice.
    let top = [
        Vec3::new(-16., -16., 48.),
        Vec3::new(16., -16., 48.),
        Vec3::new(16., 16., 48.),
        Vec3::new(-16., 16., 48.),
    ];
    for _ in 0..2 {
        for t in [[0, 1, 2], [0, 2, 3]] {
            draw(&mut cell, n, &f, (t.map(|i| top[i]), [Vec2::ZERO; 3]), None);
        }
    }
    let covered = cell.iter().filter(|c| **c > 0.).count();
    assert_eq!(
        covered,
        32 * 32,
        "one texel per unit, the 2-unit margin empty"
    );
    assert!(cell.iter().all(|c| *c == 0. || *c == 1.));
    // A half-transparent part covers half.
    clear(&mut cell, n);
    let half = |_: Vec2| 0.5;
    draw(
        &mut cell,
        n,
        &f,
        ([top[0], top[1], top[2]], [Vec2::ZERO; 3]),
        Some(&half),
    );
    assert!(cell.contains(&0.5));
}

#[test]
fn fragments_follow_screen_area_up_to_the_texture_size() {
    use super::allocator::{uv_rect, Allocator};
    let mut page = Allocator::new();
    let prop = page.allocate(128);
    // 30 px across: area 900 asks for 32 texels.
    assert!(page.use_texture(prop, false, 900.));
    assert_eq!(page.rect(prop).map(|r| r.1), Some(32));
    // Nothing changed: kept, no redraw.
    page.advance_frame();
    assert!(!page.use_texture(prop, false, 900.));
    // One size up and not redrawn anyway: kept (within one step).
    page.advance_frame();
    assert!(!page.use_texture(prop, false, 3000.));
    assert_eq!(page.rect(prop).map(|r| r.1), Some(32));
    // Much bigger on screen: capped at its own 128.
    page.advance_frame();
    assert!(page.use_texture(prop, false, 50_000.));
    assert_eq!(page.rect(prop).map(|r| r.1), Some(128));
    let rect = page.rect(prop).unwrap();
    let (origin, size) = uv_rect(rect);
    assert_eq!(size, glam::Vec2::splat(127. / 1024.));
    assert_eq!(origin, (rect.0.as_vec2() + 0.5) / 1024.);
}

#[test]
fn a_size_used_up_this_frame_falls_back_to_a_smaller_one() {
    use super::allocator::Allocator;
    let mut page = Allocator::new();
    // Six 256 fragments: the seventh caster of one frame gets a 128.
    let casters: Vec<_> = (0..7).map(|_| page.allocate(256)).collect();
    for &c in &casters {
        assert!(page.use_texture(c, true, 1e6));
    }
    let sizes: Vec<_> = casters.iter().map(|&c| page.rect(c).unwrap().1).collect();
    assert_eq!(sizes, [256, 256, 256, 256, 256, 256, 128]);
    // 256 fragments sit in the last six blocks of the page.
    let (at, _) = page.rect(casters[0]).unwrap();
    assert!(at.y >= 512);
}
