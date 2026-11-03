//! App-loop setup and rendering policy before map cameras are created.
use bevy::{
    ecs::schedule::IntoScheduleConfigs,
    prelude::{App, Startup, Update},
};

pub(crate) fn make_app(background: bool, profile: bool) -> App {
    let mut app = App::new();
    if background {
        app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
            crate::performance::background_interval(profile),
        ));
    }
    app.add_systems(
        Startup,
        crate::rendering::drawing::configure.before(crate::setup),
    );
    app.add_systems(
        Update,
        crate::audio::deferred::poll.after(crate::campaign::poll),
    );
    app
}
