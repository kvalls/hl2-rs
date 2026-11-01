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
        if self.dev_overlay {
            let (triangles, materials, textures) = status.geometry_statistics();
            let eye = sim.eye();
            self.lines = crate::overlay::developer_lines(crate::overlay::DeveloperFrame {
                map: &game.world.name,
                triangles,
                textures,
                entities: game.world.entities.len(),
                flying: sim.flying(),
                fps: hud.frame_statistics().0,
                friendly_fire: game.scene.player_hurts_friendlies,
                eye: eye.to_array(),
                frame_ms: hud.frame_statistics().1,
                materials,
                colliders: sim.physics.colliders.len(),
                bodies: sim.physics.bodies.len(),
            });
        }
        // Overlays stack down the left edge in the order F1, viewmodel tweak, F4; hidden ones leave no gap.
        {
            let stack_top = draw_debug_overlays(self, ui, sim, &self.lines, width, height);
            // F4 NPC spawn panel (ours, outside the hl2-ui menus).
            self.panel_switches = Some((game.ai.enabled, game.ai.ignore_player));
            let panel = self
                .overlay
                .open
                .then(|| self.overlay.panel(game.ai.enabled, game.ai.ignore_player));
            let hits = if let (Some(font), Some(panel)) = (&self.font, &panel) {
                let message = game
                    .spawn_log
                    .last()
                    .and_then(|entry| entry["message"].as_str());
                let hits = font.draw(&hud.source.canvas, panel, message, stack_top);
                // The panel can cover the screen center: mark where spawns aim (ours).
                let (x, y) = ((width / 2.).round(), (height / 2.).round());
                let canvas = &hud.source.canvas;
                let dark = source_ui::canvas::Color::new(0., 0., 0., 0.8);
                canvas.rectangle(x - 8., y - 2., 17., 5., dark);
                canvas.rectangle(x - 2., y - 8., 5., 17., dark);
                let mark = source_ui::canvas::Color::from_rgba(255, 238, 160, 255);
                canvas.rectangle(x - 7., y - 1., 15., 3., mark);
                canvas.rectangle(x - 1., y - 7., 3., 15., mark);
                hits
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
fn draw_debug_overlays(
    state: &State,
    ui: &hl2_bevy::ui_host::Console,
    _sim: &hl2_bevy::simulation::Simulation,
    debug_lines: &[String],
    width: f32,
    height: f32,
) -> f32 {
    let painter = crate::overlay::OverlayPainter {
        canvas: &ui.source.canvas,
        font: ui.source.overlay_font(),
    };
    let mut stack_top = 0.;
    if !debug_lines.is_empty() {
        stack_top = painter.draw_debug(debug_lines) + OVERLAY_GAP;
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
                ui.source.config.video.background_force_default_blur,
            )
            .to_degrees(),
            ui.source.config.video.background_force_default_blur,
        );
        stack_top = painter.draw_background_debug(&lines, stack_top) + OVERLAY_GAP;
    }
    stack_top
}
