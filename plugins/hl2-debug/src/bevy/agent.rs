//! Agent link on the host: answers `ui_dump` and `agent_resume` from the console and the
//! simulation.
use crate::agent::{World, ui_dump_line};

/// Prints the `ui_dump` line.
pub fn ui_dump(sim: &hl2_bevy::simulation::Simulation, ui: &mut hl2_bevy::ui_host::Console) {
    let eye = sim.eye();
    let world = World {
        map: ui.source.loaded_map.as_ref().map(|map| map.map.as_str()),
        eye: [eye.x, eye.y, eye.z],
        angles: [-sim.pitch.to_degrees(), sim.yaw.to_degrees()],
        paused: sim.paused,
        loading: sim.loading,
        background_level: sim.background_level,
    };
    let line = ui_dump_line(ui.report(), &world);
    ui.source.log(line);
}

/// Prints the `agent_resume` line: the arguments that restart where the player is.
pub fn agent_resume(sim: &hl2_bevy::simulation::Simulation, ui: &mut hl2_bevy::ui_host::Console) {
    let eye = sim.eye();
    let resume = crate::agent::Resume {
        map: ui.source.loaded_map.as_ref().map(|map| map.map.as_str()),
        background_level: sim.background_level,
        eye: [eye.x, eye.y, eye.z],
        yaw: sim.yaw.to_degrees(),
        pitch: sim.pitch.to_degrees(),
        mode: ui.source.mode(),
        windows: ui.source.open_windows(),
    };
    let line = format!(
        "{}{}",
        crate::agent::RESUME_PREFIX,
        crate::agent::resume_args(&resume).join(" ")
    );
    ui.source.log(line);
}
