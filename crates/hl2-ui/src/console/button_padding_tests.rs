use super::{Canvas, Console, Vec2, Vfs};

#[test]
#[ignore = "requires an owned Half-Life 2 installation"]
fn owned_confirmation_button_padding() {
    let root = source_assets::install::discover().expect("installed game");
    let vfs = Vfs::mount_game(&root, "hl2").unwrap();
    let canvas = Canvas::default();
    let console = Console::load(&vfs, canvas.clone());
    let mut quit = crate::gameui::menu::quit_box(&console.gameui.localization);
    for label in [
        quit.ok.clone(),
        "Beenden und zum Desktop zurückkehren".into(),
        "Quitter le jeu et retourner au bureau".into(),
    ] {
        quit.ok = label;
        for scale in [1., 1.25, 1.5, 2., 3.] {
            let painter = console.painter(Vec2::ZERO, scale);
            let layout = quit.layout_for(&painter);
            let rect = layout.buttons[0];
            let nominal = quit.layout(|s| console.painter(Vec2::ZERO, 1.).text_width(s, false));
            assert!(rect.w >= nominal.buttons[0].w, "only extend buttons");
            painter.button(rect, &quit.ok, source_ui::widgets::ButtonState::new(true));
            let quads = canvas.drain();
            let glyphs: Vec<_> = quads.iter().filter(|q| q.texture.is_some()).collect();
            let left = glyphs
                .iter()
                .map(|q| q.destination.x)
                .fold(f32::INFINITY, f32::min)
                - rect.x * scale;
            let right = (rect.x + rect.w) * scale
                - glyphs
                    .iter()
                    .map(|q| q.destination.x + q.destination.w)
                    .fold(0., f32::max);
            println!(
                "{} scale {scale}: left {left}, right {right}, width {}",
                quit.ok, rect.w
            );
            assert!(
                right + 1. >= left,
                "right padding must not be tighter than left"
            );
        }
    }
}
