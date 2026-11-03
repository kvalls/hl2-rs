use super::{
    PreparedAudio,
    deferred::{AudioOwner, PendingAudio, poll},
};
use crate::{assets::work::Work, gameplay::Gameplay};
use bevy::{
    ecs::system::RunSystemOnce,
    prelude::{App, Assets},
};
use std::sync::Arc;

fn prepared() -> PreparedAudio {
    let mut prepared = PreparedAudio::silent();
    prepared.ready = true;
    prepared
        .waves
        .push(("test.wav".into(), Arc::from(&b"test bytes"[..])));
    prepared
}
fn app(owner: Arc<()>, pending: PendingAudio) -> App {
    let mut app = App::new();
    let (options, _) =
        crate::Options::parse_from(std::iter::empty::<String>(), Default::default()).unwrap();
    app.init_resource::<Assets<bevy::audio::AudioSource>>()
        .insert_resource(options)
        .insert_resource(crate::ui_host::Console::new(Default::default()))
        .insert_resource(Gameplay::synthetic(Default::default()))
        .insert_resource(crate::simulation::Simulation::new(
            &Default::default(),
            glam::Vec3::ZERO,
            0.,
            0.,
            false,
            vec![],
        ))
        .insert_resource(AudioOwner(owner))
        .insert_resource(pending);
    app
}
#[test]
fn cancelled_and_replaced_world_results_never_install_sources_or_audio() {
    for cancelled in [false, true] {
        let owner = Arc::new(());
        let pending = PendingAudio::new(Work::ready(prepared()), owner.clone());
        if cancelled {
            pending.cancel();
        }
        let active = if cancelled { owner } else { Arc::new(()) };
        let mut app = app(active, pending);
        app.world_mut().run_system_once(poll).unwrap();
        assert!(!app.world().contains_resource::<super::Audio>());
        assert!(!app.world().contains_resource::<PendingAudio>());
        assert_eq!(
            app.world()
                .resource::<Assets<bevy::audio::AudioSource>>()
                .len(),
            0
        );
    }
}
#[test]
fn matching_generation_adopts_audio_after_world_copy_and_retains_request_order() {
    let owner = Arc::new(());
    let mut pending = PendingAudio::new(Work::ready(prepared()), owner.clone());
    let mut game = Gameplay::synthetic(Default::default());
    game.sound_requests.push("first.wav".into());
    pending.defer(&mut game);
    assert!(game.sound_requests.is_empty());
    let mut app = app(owner, pending);
    {
        let mut game = app.world_mut().resource_mut::<Gameplay>();
        let shared = game.world.clone();
        Arc::make_mut(&mut game.world).name = "active".into();
        drop(shared);
        game.sound_requests.push("second.wav".into());
    }
    app.world_mut().run_system_once(poll).unwrap();
    assert!(app.world().resource::<super::Audio>().ready);
    assert_eq!(
        app.world()
            .resource::<Assets<bevy::audio::AudioSource>>()
            .len(),
        1
    );
    assert!(!app.world().contains_resource::<PendingAudio>());
    let game = app.world().resource::<Gameplay>();
    assert_eq!(
        game.sound_requests
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["first.wav", "second.wav"]
    );
}

#[test]
fn late_background_audio_holds_world_clocks_without_holding_the_loading_ui() {
    let mut sim = crate::simulation::Simulation::new(
        &Default::default(),
        glam::Vec3::ZERO,
        0.,
        0.,
        false,
        vec![],
    );
    sim.background_level = true;
    sim.background_audio_pending = true;
    assert!(!sim.loading());
    assert!(!sim.world_running());
    sim.background_audio_pending = false;
    assert!(sim.world_running());
    sim.background_audio_pending = true;
    sim.background_level = false;
    assert!(
        sim.world_running(),
        "gameplay is unaffected by the background-only readiness gate"
    );
}
