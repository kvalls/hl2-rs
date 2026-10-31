pub(super) fn log_debug_load_start(
    request: &hl2_bevy::extensions::MapLoad<'_>,
    state: &mut super::State,
    ui: &mut hl2_bevy::ui_host::Console,
) {
    if request.background {
        if state.background_debug.enabled {
            let is_wide = ui.source.is_widescreen();
            let blur_name = ui.source.blur_texture_name(&request.map, is_wide);
            ui.source.log(format!(
                "Started loading {}, applied {}.",
                request.map, blur_name
            ));
            state.background_debug.map_name = request.map.to_owned();
            state.background_debug.blur_texture_name = blur_name;
            state.background_debug.state_label = "Loading".to_owned();
        }
    } else if state.background_debug.enabled {
        let is_wide = ui.source.is_widescreen();
        let bg_map = ui
            .source
            .loading_background
            .as_ref()
            .map(|m| m.map.as_str())
            .unwrap_or("background01");
        let blur_name = ui.source.blur_texture_name(bg_map, is_wide);
        ui.source.log(format!(
            "Started loading map {}, applied {}.",
            request.map, blur_name
        ));
        state.background_debug.pending_gameplay_finish = true;
        state.background_debug.enabled = false;
        ui.source.presentation.capture_menu_navigation = false;
        state.background_debug.state_label = "Gameplay loading".to_owned();
    }
}

pub(super) fn log_debug_load_finish(
    request: &hl2_bevy::extensions::MapLoad<'_>,
    success: bool,
    state: &mut super::State,
    ui: &mut hl2_bevy::ui_host::Console,
) {
    if request.background {
        if state.background_debug.enabled {
            if success {
                let opacity = state.background_debug.blur_opacity;
                let blur_name = &state.background_debug.blur_texture_name;
                ui.source.log(format!(
                    "Finished loading {}, success, set blur overlay {} opacity to {}.",
                    request.map, blur_name, opacity
                ));
                state.background_debug.state_label = "Ready".to_owned();
            } else {
                ui.source
                    .log(format!("Finished loading {}, failure.", request.map));
                state.background_debug.state_label = "Failure".to_owned();
            }
        }
    } else if state.background_debug.pending_gameplay_finish {
        state.background_debug.pending_gameplay_finish = false;
        let status = if success { "success." } else { "failure." };
        ui.source
            .log(format!("Finished loading map {}, {}", request.map, status));
    }
}
