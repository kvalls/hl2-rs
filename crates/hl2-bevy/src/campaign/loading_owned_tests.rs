//! Real scene replacement while a displayed background still owns a blocked late resource.
use super::{Campaign, MapLoaded, poll};
use crate::{audio::deferred, gameplay::Gameplay, simulation::Simulation, ui_host};
use bevy::prelude::{App, Assets, Image, IntoScheduleConfigs, Mesh, Update};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

pub(super) fn init_map_assets(app: &mut App) -> &mut App {
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<crate::rendering::SourceMaterial>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<bevy::audio::AudioSource>>()
        .init_resource::<Assets<crate::effects::EffectMaterial>>()
        .init_resource::<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>()
}

#[test]
#[ignore = "requires owned HL2 installation"]
fn displayed_background_late_audio_is_cancelled_by_gameplay_and_background_replacements() {
    let root = source_assets::install::discover().expect("owned install");
    for background in [false, true] {
        let mut app = App::new();
        init_map_assets(&mut app).init_resource::<crate::Status>();
        app.insert_resource(super::tests::test_options(&root));
        let world = modkit_core::World {
            name: "background01".into(),
            ..Default::default()
        };
        app.insert_resource(Gameplay::synthetic(world));
        let old_map_entity = app.world_mut().spawn(super::MapOwned).id();
        let mut sim = Simulation::new(&Default::default(), glam::Vec3::ZERO, 0., 0., false, vec![]);
        sim.background_level = true;
        app.insert_resource(sim);
        app.insert_resource(Campaign {
            loaded_map: Some(hl2_ui::console::GameMap::new("hl2", "background01")),
            last_event: Some(MapLoaded {
                game: "hl2".into(),
                map: "background01".into(),
                background: true,
            }),
            ..Default::default()
        });
        let mut ui = ui_host::Console::new(Default::default());
        let target = if background {
            "ep2_background01"
        } else {
            "d1_trainstation_02"
        };
        ui.map_request = Some(target.into());
        ui.map_request_game = Some(if background { "ep2" } else { "hl2" }.into());
        ui.background_request = background;
        app.insert_resource(ui);
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let work = crate::assets::work::Work::spawn("held-old-background-audio", move |cancel| {
            started_tx.send(()).unwrap();
            while cancel.check().is_ok() {
                std::thread::yield_now();
            }
            stopped_tx.send(()).unwrap();
            cancel.check()?;
            unreachable!()
        })
        .unwrap();
        let cancellation = work.cancel.clone();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        deferred::stage(&mut app.world_mut().commands(), Some(work));
        app.world_mut().flush();
        app.add_systems(Update, (poll, deferred::poll.after(poll)));
        app.update();
        assert!(
            cancellation.cancelled(),
            "new request cancels old late work before replacement completes"
        );
        stopped_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while app
            .world()
            .resource::<Campaign>()
            .loaded_map
            .as_ref()
            .is_none_or(|m| m.map != target)
        {
            assert!(Instant::now() < deadline, "replacement timeout");
            std::thread::sleep(Duration::from_millis(2));
            app.update();
        }
        let campaign = app.world().resource::<Campaign>();
        assert!(campaign.failures.is_empty(), "{:?}", campaign.failures);
        assert_eq!(app.world().resource::<Gameplay>().world.name, target);
        assert!(
            app.world().get_entity(old_map_entity).is_err(),
            "old scene entity replaced"
        );
        assert!(!app.world().resource::<Simulation>().loading());
        let mut query = app
            .world_mut()
            .query_filtered::<bevy::prelude::Entity, bevy::prelude::With<super::MapOwned>>();
        assert!(
            query.iter(app.world()).count() > 10,
            "replacement scene installed"
        );
    }
}
