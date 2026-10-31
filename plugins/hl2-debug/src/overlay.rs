//! Shared developer overlay painter. Fonts are supplied by the host.
use glam::{Vec2, vec2};
use source_ui::{
    canvas::{Canvas, Color, WHITE},
    font::FontFace,
};
const DEBUG_HEADER_HEIGHT: f32 = 105.;
pub struct OverlayPainter<'a> {
    pub canvas: &'a Canvas,
    pub font: Option<&'a FontFace>,
}
fn text(
    _canvas: &Canvas,
    value: &str,
    origin: Vec2,
    size: f32,
    scale: f32,
    font: Option<&FontFace>,
    color: Color,
) {
    if let Some(font) = font {
        font.draw_baseline(value, origin, size, scale, color);
    }
}
impl OverlayPainter<'_> {
    /// Four compact header rows and a footer, preserving the retained host's developer layout.
    /// Font data is bundled under its own MIT license, independent of installed game content.
    /// Returns the height the overlay occupies at the top of the canvas.
    pub fn draw_debug(&self, lines: &[String]) -> f32 {
        self.canvas.rectangle(
            0.,
            0.,
            self.canvas.width(),
            DEBUG_HEADER_HEIGHT,
            Color::new(0.025, 0.045, 0.07, 0.88),
        );
        let rows = [
            (vec2(22., 30.), 27., WHITE),
            (vec2(22., 55.), 19., Color::new(0.784, 0.784, 0.784, 1.)),
            (vec2(22., 78.), 17., Color::new(0.784, 0.784, 0.784, 1.)),
            (vec2(22., 98.), 16., Color::from_rgba(240, 180, 90, 255)),
        ];
        let draw = |line: &str, origin: Vec2, size: f32, color| {
            if let Some(font) = self.font {
                font.draw_baseline(line, origin, size, 1., color);
            } else {
                text(self.canvas, line, origin, size, 1., self.font, color);
            }
        };
        for (line, (origin, size, color)) in lines.iter().take(4).zip(rows) {
            draw(line, origin, size, color);
        }
        if let Some(footer) = lines.get(4) {
            self.canvas.rectangle(
                0.,
                self.canvas.height() - 38.,
                self.canvas.width(),
                38.,
                Color::new(0.025, 0.045, 0.07, 0.85),
            );
            draw(footer, vec2(20., self.canvas.height() - 13.), 18., WHITE);
        }
        DEBUG_HEADER_HEIGHT
    }

    fn draw_debug_lines_panel(&self, lines: &[String], top: f32, max_lines: usize) -> f32 {
        let count = lines.len().min(max_lines);
        let height = 20. + count as f32 * 22. + 10.;
        self.canvas.rectangle(
            0.,
            top,
            self.canvas.width(),
            height,
            Color::new(0.025, 0.045, 0.07, 0.9),
        );
        let draw = |line: &str, origin: Vec2, color| {
            if let Some(font) = self.font {
                font.draw_baseline(line, origin, 16., 1., color);
            } else {
                text(self.canvas, line, origin, 16., 1., self.font, color);
            }
        };
        let normal = Color::new(0.784, 0.784, 0.784, 1.);
        let last_idx = count.saturating_sub(1);
        for (i, line) in lines.iter().take(count).enumerate() {
            let y = top + 20. + i as f32 * 22.;
            let color = if i == last_idx {
                Color::from_rgba(240, 180, 90, 255)
            } else if i == 0 {
                WHITE
            } else {
                normal
            };
            draw(line, vec2(22., y), color);
        }
        top + height
    }

    /// Draws the live viewmodel tuning values with their top edge at `top`; returns the bottom
    /// edge.
    pub fn draw_viewmodel_debug(&self, lines: &[String], top: f32) -> f32 {
        self.draw_debug_lines_panel(lines, top, lines.len())
    }

    /// Draws the live background visual debug values with their top edge at `top`; returns the bottom
    /// edge.
    pub fn draw_background_debug(&self, lines: &[String], top: f32) -> f32 {
        self.draw_debug_lines_panel(lines, top, lines.len())
    }
}

/// Snapshot for F1. The host chooses which diagnostics to sample.
pub struct DeveloperFrame<'a> {
    pub map: &'a str,
    pub triangles: usize,
    pub textures: u64,
    pub entities: usize,
    pub flying: bool,
    pub fps: f64,
    pub friendly_fire: bool,
    pub eye: [f32; 3],
    pub frame_ms: f64,
    pub materials: usize,
    pub colliders: usize,
    pub bodies: usize,
}
pub fn developer_lines(frame: DeveloperFrame<'_>) -> Vec<String> {
    vec![
            format!("HL2-RS  /  Rust runtime  /  {}", frame.map),
            format!("{} triangles  |  {} frame.textures  |  {} entities  |  {}  |  {:.0} fps", frame.triangles, frame.textures, frame.entities, if frame.flying { "FLY" } else { "WALK" }, frame.fps),
            "WASD / click: mouse / Esc pause / tilde console / F2 fly / Space,Ctrl vertical in fly / G prop impulse".into(),
            "E use / Ctrl crouch / Space jump / R reload / 1-6 / wheel: weapon menu / Q last / F3 dev loadout / F1 overlay / F4 NPC spawn".into(),
            format!("Player hurts friendlies: {} (F4 options, debug)", if frame.friendly_fire { "ON" } else { "off" }),
            format!("Map reconstruction preview  |  position {:.1}, {:.1}, {:.1}  |  Bevy/wgpu  |  {:.2} ms  |  {} materials / {} colliders / {} bodies", frame.eye[0], frame.eye[1], frame.eye[2], frame.frame_ms, frame.materials, frame.colliders, frame.bodies),
        ]
}
