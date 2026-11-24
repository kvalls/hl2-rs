use super::State;
use hl2_bevy::extensions::Context;
const OVERLAY_GAP: f32 = 5.;
impl State {
    pub(super) fn draw(
        &mut self,
        context: &mut Context<'_>,
        hud: &mut hl2_bevy::canvas_renderer::Hud,
        status: &hl2_bevy::Status,
    ) {
        self.adopt_finished_usable();
        self.adopt_finished_props();
        let (Some(game), Some(ui)) = (
            context.gameplay.as_deref_mut(),
            context.console.as_deref_mut(),
        ) else {
            return;
        };
        let sim = &*context.simulation;
        let (width, height) = (hud.source.canvas.width(), hud.source.canvas.height());
        if self.font.is_none() {
            self.font = hud.bitmap_font().map(crate::spawn_panel::PanelFont);
        }
        self.lines.clear();
        self.profiling_lines.clear();
        if self.dev_overlay {
            self.profiling_lines = profiling_rows(hud.profiling());
            let (triangles, materials, textures) = status.geometry_statistics();
            let eye = sim.eye();
            let (yaw, pitch) = sim.view_angles();
            self.lines = crate::overlay::developer_lines(crate::overlay::DeveloperFrame {
                map: &game.world.name,
                triangles,
                textures,
                entities: game.world.entities.len(),
                flying: sim.flying(),
                fps: hud.frame_statistics().0,
                friendly_fire: game.scene.player_hurts_friendlies,
                eye: eye.to_array(),
                angles: [yaw.to_degrees(), pitch.to_degrees()],
                frame_ms: hud.frame_statistics().1,
                materials,
                colliders: sim.physics.colliders.len(),
                bodies: sim.physics.bodies.len(),
            });
        }
        // Overlays stack down the left edge in the order F1, viewmodel tweak, F4; hidden ones leave no gap.
        {
            let stack_top = draw_debug_overlays(self, ui, sim, width, height);
            // F4 NPC spawn panel (ours, outside the hl2-ui menus).
            self.panel_switches = Some((game.ai.enabled, game.ai.ignore_player));
            refresh_loading_status(&mut self.loading_status, game.spawn_log.len());
            let panel = self
                .overlay
                .open
                .then(|| self.overlay.panel(game.ai.enabled, game.ai.ignore_player));
            let hits = if let (Some(font), Some(panel)) = (&self.font, &panel) {
                let message = footer_message(&mut self.loading_status, &game.spawn_log);
                font.draw(&hud.source.canvas, panel, message, stack_top)
            } else {
                vec![]
            };
            self.hits = hits;
            self.rows = self
                .font
                .as_ref()
                .map_or(0, |font| font.rows_fit(stack_top, height));
        }
    }
}
pub(super) fn refresh_loading_status(
    loading_status: &mut Option<(usize, String)>,
    spawn_log_len: usize,
) {
    if loading_status
        .as_ref()
        .is_some_and(|(len, _)| *len != spawn_log_len)
    {
        *loading_status = None;
    }
}
pub(super) fn footer_message<'a>(
    loading_status: &'a mut Option<(usize, String)>,
    spawn_log: &'a [serde_json::Value],
) -> Option<&'a str> {
    refresh_loading_status(loading_status, spawn_log.len());
    loading_status
        .as_ref()
        .map(|(_, msg)| msg.as_str())
        .or_else(|| spawn_log.last().and_then(|entry| entry["message"].as_str()))
}
/// F1 header rows from the host's profiler status; none when no profiler runs.
fn profiling_rows(
    snapshot: Option<hl2_bevy::performance::status::ProfilingSnapshot>,
) -> Vec<String> {
    use crate::overlay::{ProfilerState, StageProfiler, TraceProfiler};
    use hl2_bevy::performance::status::RecorderState;
    let Some(snapshot) = snapshot else {
        return vec![];
    };
    let state = |state| match state {
        RecorderState::Idle => ProfilerState::Idle,
        RecorderState::Recording => ProfilerState::Profiling,
        RecorderState::Finished => ProfilerState::Finished,
    };
    crate::overlay::profiling_lines(
        snapshot.stages.map(|s| StageProfiler {
            state: state(s.state),
            elapsed: s.elapsed,
            frames: s.frames,
            limit: s.limit,
        }),
        snapshot.trace.map(|t| TraceProfiler {
            state: state(t.state),
            elapsed: t.elapsed,
            bytes: t.bytes,
        }),
    )
}
fn draw_debug_overlays(
    state: &State,
    ui: &hl2_bevy::ui_host::Console,
    _sim: &hl2_bevy::simulation::Simulation,

    width: f32,
    height: f32,
) -> f32 {
    let painter = crate::overlay::OverlayPainter {
        canvas: &ui.source.canvas,
        font: ui.source.overlay_font(),
    };
    let mut stack_top = 0.;
    if !state.lines.is_empty() {
        stack_top = painter.draw_debug(&state.lines, &state.profiling_lines) + OVERLAY_GAP;
    }
    if state.viewmodel_debug.enabled
        && let Some(profile) = state.viewmodel_debug.profile
    {
        let value = state.viewmodel_debug.framing;
        let lines = vec![
            format!("aspect_ratio: {}", profile),
            format!("weapon: {}", state.viewmodel_debug.weapon),
            format!("fov_offset: {:.2}    [ ]", value.fov_offset),
            format!("offset.x: {:.2}    ,/.", value.offset[0]),
            format!("offset.y: {:.2}    L/R", value.offset[1]),
            format!("offset.z: {:.2}    U/D", value.offset[2]),
            "Shift = 1.0  Alt = 0.01    hold to repeat    0: reset".to_owned(),
        ];
        stack_top = painter.draw_viewmodel_debug(&lines, stack_top) + OVERLAY_GAP;
    }
    if state.background_debug.enabled {
        let aspect = width / height.max(1.);
        let lines = state.background_debug.lines(
            hl2_bevy::camera::background_fov(
                aspect,
                crate::standard_background::forced(&ui.source.config.video),
            )
            .to_degrees(),
            crate::standard_background::forced(&ui.source.config.video),
        );
        stack_top = painter.draw_background_debug(&lines, stack_top) + OVERLAY_GAP;
    }
    stack_top
}
