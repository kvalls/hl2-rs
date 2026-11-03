use crate::{config::HudAspect, hud::WeaponHud};
use glam::vec2;
use hl2_simulation::gameplay::Inventory;
use source_assets::vpk::Vfs;
use source_ui::canvas::{Canvas, Quad, Rect};

fn bounds(quads: &[Quad]) -> Rect {
    let left = quads
        .iter()
        .map(|q| q.destination.x)
        .fold(f32::INFINITY, f32::min);
    let top = quads
        .iter()
        .map(|q| q.destination.y)
        .fold(f32::INFINITY, f32::min);
    let right = quads
        .iter()
        .map(|q| q.destination.x + q.destination.w)
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = quads
        .iter()
        .map(|q| q.destination.y + q.destination.h)
        .fold(f32::NEG_INFINITY, f32::max);
    Rect::new(left, top, right - left, bottom - top)
}

#[test]
#[ignore = "requires an owned Half-Life 2 installation"]
fn owned_quickinfo_arcs_center_on_crosshair_across_layouts_and_states() {
    let root = source_assets::install::discover().expect("installed game");
    let vfs = Vfs::mount(&root).unwrap();
    let canvas = Canvas::default();
    let mut hud = WeaponHud::load(&vfs, canvas.clone()).unwrap();
    let weapons = hl2_simulation::gameplay::definitions(&vfs).unwrap();
    let mut inv = Inventory::default();
    inv.give("weapon_smg1", &weapons, 0.);
    inv.active = "weapon_smg1".into();
    for viewport in [
        vec2(640., 480.),
        vec2(1280., 720.),
        vec2(1920., 1080.),
        vec2(1281., 721.),
        vec2(3440., 1440.),
        vec2(720., 1280.),
    ] {
        for aspect in [
            HudAspect::Fit,
            HudAspect::Normal,
            HudAspect::Wide16x9,
            HudAspect::Wide16x10,
        ] {
            canvas.resize(viewport.x, viewport.y);
            hud.aspect = aspect;
            hud.draw_crosshair(&inv);
            let crosshair = canvas.drain();
            let crosshair_center = bounds(&crosshair).center();
            assert!(
                (crosshair_center.y - viewport.y * 0.5).abs() <= 1.,
                "weapon crosshair must center on the view: {crosshair_center:?}"
            );
            assert!(
                (crosshair_center.x - viewport.x * 0.5).abs() <= 1.,
                "weapon crosshair must center horizontally: {crosshair_center:?}"
            );
            for (health, clip) in [(100., 45), (50., 23), (10., 2)] {
                inv.health = health;
                inv.owned.insert(inv.active.clone(), clip);
                *hud.quick_info.state.borrow_mut() = Default::default();
                let _frame = hud.framed();
                hud.quick_info.draw(&inv, &weapons, 0.2);
                let quads = canvas.drain();
                let arc_quads = &quads[1..];
                assert!(arc_quads.len() >= 2, "both arcs must draw");
                let arcs = bounds(arc_quads);
                // Measured owned retail captures: 720p approx 55x42, 1080p 85x63.
                // Fontdue antialias fringes can extend the bitmap by up to two pixels.
                for (size, retail) in [
                    (vec2(1280., 720.), vec2(55., 42.)),
                    (vec2(1920., 1080.), vec2(85., 63.)),
                ] {
                    if viewport == size {
                        assert!(
                            (arcs.w - retail.x).abs() <= 2.,
                            "retail arc width: {arcs:?}"
                        );
                        assert!(
                            (arcs.h - retail.y).abs() <= 1.,
                            "retail arc height: {arcs:?}"
                        );
                    }
                }
                assert!(
                    (arcs.center().x - viewport.x * 0.5).abs() < 0.01,
                    "horizontal arc center: {viewport:?} {aspect:?}: {arcs:?}"
                );
                assert!(
                    (arcs.center().y - crosshair_center.y).abs() <= 1.,
                    "arc and weapon crosshair must align"
                );
                for right in [false, true] {
                    let side: Vec<_> = arc_quads
                        .iter()
                        .filter(|q| (q.destination.x > viewport.x * 0.5) == right)
                        .cloned()
                        .collect();
                    let rect = bounds(&side);
                    assert!((rect.center().y - viewport.y * 0.5).abs() <= 0.5);
                    if side.len() == 2 {
                        assert_eq!(
                            side[0].destination.y + side[0].destination.h,
                            side[1].destination.y,
                            "progress pieces must meet without a gap or overlap"
                        );
                    }
                    assert!(side.iter().all(|q| q.additive));
                }
                assert!(
                    (arcs.center().y - viewport.y * 0.5).abs() <= 0.5,
                    "{viewport:?} {aspect:?} health={health} clip={clip}: {arcs:?}"
                );
                assert!(arcs.x < viewport.x * 0.5 && arcs.x + arcs.w > viewport.x * 0.5);
            }
        }
    }
}
