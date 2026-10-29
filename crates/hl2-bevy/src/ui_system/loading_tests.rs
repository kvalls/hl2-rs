//! Exercise the native per-frame input path while fixed simulation is frozen by loading.
use super::update_ui;
use bevy::ecs::system::RunSystemOnce;
use bevy::{
    app::AppExit,
    prelude::{App, ButtonInput, KeyCode, MouseButton, Real, Time, Window, default},
    window::{CursorOptions, PrimaryWindow},
};

#[test]
#[ignore = "requires owned HL2 installation"]
fn native_menu_wait_blocks_input_until_background_finishes() {
    let root = source_assets::install::discover().expect("owned install");
    let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
    let mut source = hl2_ui::console::Console::load(&vfs, Default::default());
    source.level_loaded_for_map("hl2", "background01", true);
    source.activate_menu(1);
    assert!(source.activate_menu(0).is_empty());
    assert_eq!(source.mode(), hl2_ui::console::Mode::MainMenu);
    assert!(source.is_background_loading());
    let mut sim = crate::simulation::Simulation::new(
        &modkit_core::World::default(),
        glam::Vec3::ZERO,
        0.,
        0.,
        false,
        vec![],
    );
    sim.loading = true;
    let mut app = App::new();
    app.insert_resource(sim)
        .insert_resource(crate::gameplay::Gameplay::synthetic(Default::default()))
        .insert_resource(crate::ui_host::Console::new(source))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Time<Real>>()
        .add_message::<AppExit>()
        .add_message::<bevy::input::keyboard::KeyboardInput>()
        .add_message::<bevy::input::mouse::MouseWheel>();
    app.world_mut().spawn((
        Window {
            focused: true,
            resolution: bevy::window::WindowResolution::new(1024, 768),
            ..default()
        },
        CursorOptions::default(),
        PrimaryWindow,
    ));
    key(&mut app, KeyCode::ArrowDown, true);
    key(&mut app, KeyCode::ArrowDown, false);
    key(&mut app, KeyCode::Enter, true);
    key(&mut app, KeyCode::Enter, false);
    key(&mut app, KeyCode::Escape, true);
    key(&mut app, KeyCode::Escape, false);
    let ui = app.world().resource::<crate::ui_host::Console>();
    assert_eq!(ui.source.mode(), hl2_ui::console::Mode::MainMenu);
    assert_eq!(ui.source.menu_category(), "episodic");
    assert!(ui.source.chapters_report().is_none());
    assert!(ui.map_request.is_none());
    assert!(ui.source.is_background_loading());
    app.world_mut()
        .resource_mut::<crate::ui_host::Console>()
        .source
        .level_loaded_for_map("episodic", "ep1_background01", true);
    app.world_mut()
        .resource_mut::<crate::simulation::Simulation>()
        .loading = false;
    app.world_mut()
        .resource_mut::<crate::ui_host::Console>()
        .source
        .activate_menu(0);
    key(&mut app, KeyCode::ArrowRight, true);
    assert_eq!(
        app.world()
            .resource::<crate::ui_host::Console>()
            .source
            .chapters_report()
            .unwrap()["selected"],
        1
    );
    key(&mut app, KeyCode::Enter, true);
    key(&mut app, KeyCode::Enter, false);
    let ui = app.world().resource::<crate::ui_host::Console>();
    assert!(ui.source.modal_dialog_open());
    assert!(!ui.source.is_background_loading());
    assert_eq!(ui.map_request.as_deref(), Some("ep1_citadel_03"));
    assert!(!ui.background_request);
}

fn key(app: &mut App, key: KeyCode, pressed: bool) {
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.clear();
    if pressed {
        keys.press(key);
    } else {
        keys.release(key);
    }
    app.world_mut().run_system_once(update_ui).unwrap();
}
