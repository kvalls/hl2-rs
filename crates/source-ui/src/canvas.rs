//! CPU texture data and ordered HUD primitives, without GPU or global state.
//! Explicit VFS loading happens at setup; drawing only queues in-memory commands.
use glam::{vec2, Vec2};
use std::sync::{Arc, Mutex, MutexGuard};

pub struct MutexCell<T>(Mutex<T>);
impl<T> MutexCell<T> {
    pub fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }
    pub fn borrow(&self) -> MutexGuard<'_, T> {
        self.0.lock().expect("HUD state lock")
    }
    pub fn borrow_mut(&self) -> MutexGuard<'_, T> {
        self.borrow()
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}
impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
    pub const fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.,
            g: g as f32 / 255.,
            b: b as f32 / 255.,
            a: a as f32 / 255.,
        }
    }
    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}
pub const WHITE: Color = Color::from_rgba(255, 255, 255, 255);
pub const YELLOW: Color = Color::from_rgba(253, 249, 0, 255);
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(self, point: Vec2) -> bool {
        point.x >= self.x
            && point.y >= self.y
            && point.x <= self.x + self.w
            && point.y <= self.y + self.h
    }
    pub fn size(self) -> Vec2 {
        vec2(self.w, self.h)
    }
    /// Grown by `by` on every side.
    pub fn pad(self, by: f32) -> Self {
        Self::new(self.x - by, self.y - by, self.w + 2. * by, self.h + 2. * by)
    }
    pub fn center(self) -> Vec2 {
        vec2(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMode {
    Nearest,
    Linear,
}
pub struct TextureData {
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
    filter: MutexCell<FilterMode>,
}
#[derive(Clone)]
pub struct Texture2D(pub Arc<TextureData>);
impl Texture2D {
    pub fn from_rgba8(width: u16, height: u16, rgba: &[u8]) -> Self {
        assert_eq!(rgba.len(), usize::from(width) * usize::from(height) * 4);
        Self(Arc::new(TextureData {
            width,
            height,
            rgba: rgba.to_vec(),
            filter: MutexCell::new(FilterMode::Linear),
        }))
    }
    pub fn width(&self) -> f32 {
        f32::from(self.0.width)
    }
    pub fn height(&self) -> f32 {
        f32::from(self.0.height)
    }
    pub fn size(&self) -> Vec2 {
        vec2(self.width(), self.height())
    }
    pub fn set_filter(&self, filter: FilterMode) {
        *self.0.filter.borrow_mut() = filter;
    }
    pub fn filter(&self) -> FilterMode {
        *self.0.filter.borrow()
    }
    pub fn id(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}
#[derive(Default)]
pub struct DrawTextureParams {
    pub source: Option<Rect>,
    pub dest_size: Option<Vec2>,
}
#[derive(Clone)]
pub struct Quad {
    pub texture: Option<Texture2D>,
    pub source: Rect,
    pub destination: Rect,
    pub color: Color,
    pub additive: bool,
    /// Multiplies the frame by the color (a modulate ScreenFade).
    pub modulate: bool,
    /// Per-vertex destination corners and colors, in the order top-left, bottom-left,
    /// bottom-right, top-right of `destination` (the damage indicator's trapezoids).
    pub vertices: Option<[(Vec2, Color); 4]>,
}
struct State {
    viewport: Vec2,
    /// The horizontal span HUD drawing is laid out in, `(x, width)`: [`Canvas::width`] is its
    /// width and every drawn quad moves right by its x. `None`: the whole viewport.
    frame: Option<(f32, f32)>,
    additive: bool,
    alpha_multiplier: f32,
    occluders: Vec<Rect>,
    commands: Vec<Quad>,
}
#[derive(Clone)]
pub struct Canvas(Arc<Mutex<State>>);
impl Default for Canvas {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(State {
            viewport: vec2(1280., 720.),
            frame: None,
            additive: false,
            alpha_multiplier: 1.0,
            occluders: vec![],
            commands: vec![],
        })))
    }
}
impl Canvas {
    pub fn resize(&self, width: f32, height: f32) {
        self.0.lock().expect("HUD canvas").viewport = vec2(width, height);
    }
    /// Sets the current alpha multiplier (0..=1) for all subsequent drawing, returning the old value.
    pub fn set_alpha_multiplier(&self, mult: f32) -> f32 {
        let mut state = self.0.lock().expect("HUD canvas");
        let old = state.alpha_multiplier;
        state.alpha_multiplier = mult.clamp(0., 1.);
        old
    }
    /// The current alpha multiplier.
    pub fn alpha_multiplier(&self) -> f32 {
        self.0.lock().expect("HUD canvas").alpha_multiplier
    }
    /// Pushes an occluder rectangle in canvas pixels. Subsequent drawn quads
    /// and polygons will have any portion falling inside this rectangle clipped/hidden.
    pub fn push_occluder(&self, rect: Rect) {
        if rect.w > 0. && rect.h > 0. {
            self.0.lock().expect("HUD canvas").occluders.push(rect);
        }
    }
    /// Pops the topmost occluder rectangle.
    pub fn pop_occluder(&self) {
        self.0.lock().expect("HUD canvas").occluders.pop();
    }
    /// Clears all active occluders.
    pub fn clear_occluders(&self) {
        self.0.lock().expect("HUD canvas").occluders.clear();
    }
    /// The width drawing is laid out in: the frame's, or the viewport's.
    pub fn width(&self) -> f32 {
        let state = self.0.lock().expect("HUD canvas");
        state.frame.map_or(state.viewport.x, |(_, width)| width)
    }
    /// The whole viewport, whatever the frame.
    pub fn screen_size(&self) -> Vec2 {
        self.0.lock().expect("HUD canvas").viewport
    }
    /// Lays out the following drawing in the span starting at `x` and `width` wide (centered HUD
    /// aspects); [`Self::clear_frame`] returns to the whole viewport.
    pub fn set_frame(&self, x: f32, width: f32) {
        self.0.lock().expect("HUD canvas").frame = Some((x, width));
    }
    pub fn clear_frame(&self) {
        self.0.lock().expect("HUD canvas").frame = None;
    }
    /// The frame's left edge in screen pixels (0 without a frame).
    pub fn frame_x(&self) -> f32 {
        self.0
            .lock()
            .expect("HUD canvas")
            .frame
            .map_or(0., |(x, _)| x)
    }
    pub fn height(&self) -> f32 {
        self.0.lock().expect("HUD canvas").viewport.y
    }
    pub fn additive(&self, value: bool) {
        self.0.lock().expect("HUD canvas").additive = value;
    }
    pub fn drain(&self) -> Vec<Quad> {
        let mut state = self.0.lock().expect("HUD canvas");
        state.occluders.clear();
        std::mem::take(&mut state.commands)
    }
    fn quad(&self, texture: Option<Texture2D>, source: Rect, destination: Rect, color: Color) {
        let mut state = self.0.lock().expect("HUD canvas");
        let a = color.a * state.alpha_multiplier;
        if destination.w <= 0. || destination.h <= 0. || a <= 0. {
            return;
        }
        let additive = state.additive;
        let destination = match state.frame {
            Some((x, _)) => Rect::new(
                destination.x + x,
                destination.y,
                destination.w,
                destination.h,
            ),
            None => destination,
        };

        if state.occluders.is_empty() {
            state.commands.push(Quad {
                texture,
                source,
                destination,
                color: Color::new(color.r, color.g, color.b, a),
                additive,
                modulate: false,
                vertices: None,
            });
            return;
        }

        let mut pieces = vec![(destination, source)];
        for occ in &state.occluders {
            let mut next = Vec::new();
            for (d, s) in pieces {
                clip_quad_against_occluder(d, s, *occ, &mut next);
            }
            pieces = next;
            if pieces.is_empty() {
                break;
            }
        }

        for (dest, src) in pieces {
            state.commands.push(Quad {
                texture: texture.clone(),
                source: src,
                destination: dest,
                color: Color::new(color.r, color.g, color.b, a),
                additive,
                modulate: false,
                vertices: None,
            });
        }
    }
    /// An untextured quad with per-vertex positions and colors (blend as set).
    pub fn polygon(&self, vertices: [(Vec2, Color); 4]) {
        let mut state = self.0.lock().expect("HUD canvas");
        let mult = state.alpha_multiplier;
        if mult <= 0. || vertices.iter().all(|(_, c)| c.a * mult <= 0.) {
            return;
        }
        let vertices = match state.frame {
            Some((x, _)) => vertices.map(|(p, mut c)| {
                c.a *= mult;
                (p + vec2(x, 0.), c)
            }),
            None => vertices.map(|(p, mut c)| {
                c.a *= mult;
                (p, c)
            }),
        };
        if state
            .occluders
            .iter()
            .any(|occ| vertices.iter().all(|(p, _)| occ.contains(*p)))
        {
            return;
        }
        let min = vertices.iter().fold(Vec2::MAX, |m, (p, _)| m.min(*p));
        let max = vertices.iter().fold(Vec2::MIN, |m, (p, _)| m.max(*p));
        let additive = state.additive;
        state.commands.push(Quad {
            texture: None,
            source: Rect::new(0., 0., 1., 1.),
            destination: Rect::new(min.x, min.y, max.x - min.x, max.y - min.y),
            color: Color::new(1., 1., 1., 1.),
            additive,
            modulate: false,
            vertices: Some(vertices),
        });
    }
    /// A full-viewport quad that multiplies the frame by `color` (alpha ignored).
    pub fn modulate(&self, color: Color) {
        let mut state = self.0.lock().expect("HUD canvas");
        let viewport = state.viewport;
        state.commands.push(Quad {
            texture: None,
            source: Rect::new(0., 0., 1., 1.),
            destination: Rect::new(0., 0., viewport.x, viewport.y),
            color,
            additive: false,
            modulate: true,
            vertices: None,
        });
    }
    /// A rectangle over the whole viewport, outside the frame.
    pub fn screen_rectangle(&self, color: Color) {
        let frame = self.0.lock().expect("HUD canvas").frame.take();
        let screen = self.screen_size();
        self.rectangle(0., 0., screen.x, screen.y, color);
        self.0.lock().expect("HUD canvas").frame = frame;
    }
    pub fn rectangle(&self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.quad(
            None,
            Rect::new(0., 0., 1., 1.),
            Rect::new(x, y, w, h),
            color,
        );
    }
}
pub fn draw_texture(canvas: &Canvas, texture: &Texture2D, x: f32, y: f32, color: Color) {
    draw_texture_ex(canvas, texture, x, y, color, DrawTextureParams::default());
}
pub fn draw_texture_ex(
    canvas: &Canvas,
    texture: &Texture2D,
    x: f32,
    y: f32,
    color: Color,
    params: DrawTextureParams,
) {
    let source = params
        .source
        .unwrap_or(Rect::new(0., 0., texture.width(), texture.height()));
    let size = params.dest_size.unwrap_or(source.size());
    canvas.quad(
        Some(texture.clone()),
        source,
        Rect::new(x, y, size.x, size.y),
        color,
    );
}
pub fn texture(vfs: &source_assets::vpk::Vfs, name: &str) -> anyhow::Result<Texture2D> {
    use anyhow::Context;
    let base = vfs
        .base_texture(name)?
        .context("HUD base texture missing")?;
    let file = format!(
        "materials/{}.vtf",
        base.trim_start_matches("materials/")
            .trim_end_matches(".vtf")
    );
    let image = source_assets::vtf::decode(&vfs.read(&file)?.context("HUD VTF missing")?, 512)?;
    Ok(Texture2D::from_rgba8(
        image.width,
        image.height,
        &image.rgba,
    ))
}

/// Clips a quad's destination rectangle and source texture coordinates against an occluder
/// rectangle using 2D axis-aligned box subtraction. Any visible portions outside `occ` are pushed
/// to `out` with proportionally scaled source coordinates.
fn clip_quad_against_occluder(dest: Rect, src: Rect, occ: Rect, out: &mut Vec<(Rect, Rect)>) {
    let x0 = dest.x;
    let x1 = dest.x + dest.w;
    let y0 = dest.y;
    let y1 = dest.y + dest.h;

    let ox0 = occ.x;
    let ox1 = occ.x + occ.w;
    let oy0 = occ.y;
    let oy1 = occ.y + occ.h;

    let ix0 = x0.max(ox0);
    let ix1 = x1.min(ox1);
    let iy0 = y0.max(oy0);
    let iy1 = y1.min(oy1);

    // No intersection
    if ix1 <= ix0 || iy1 <= iy0 {
        out.push((dest, src));
        return;
    }

    // Completely covered by occluder
    if ix0 <= x0 && ix1 >= x1 && iy0 <= y0 && iy1 >= y1 {
        return;
    }

    let u_scale = if dest.w > 0. { src.w / dest.w } else { 0. };
    let v_scale = if dest.h > 0. { src.h / dest.h } else { 0. };

    let mut add_slice = |sx_dest: f32, sy_dest: f32, sw_dest: f32, sh_dest: f32| {
        if sw_dest > 0. && sh_dest > 0. {
            let d = Rect::new(sx_dest, sy_dest, sw_dest, sh_dest);
            let s = Rect::new(
                src.x + (sx_dest - x0) * u_scale,
                src.y + (sy_dest - y0) * v_scale,
                sw_dest * u_scale,
                sh_dest * v_scale,
            );
            out.push((d, s));
        }
    };

    // 1. Top slice (above intersection)
    if y0 < iy0 {
        add_slice(x0, y0, x1 - x0, iy0 - y0);
    }
    // 2. Bottom slice (below intersection)
    if y1 > iy1 {
        add_slice(x0, iy1, x1 - x0, y1 - iy1);
    }
    // 3. Left slice (left of intersection, vertical span of intersection)
    if x0 < ix0 {
        add_slice(x0, iy0, ix0 - x0, iy1 - iy0);
    }
    // 4. Right slice (right of intersection, vertical span of intersection)
    if x1 > ix1 {
        add_slice(ix1, iy0, x1 - ix1, iy1 - iy0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white() -> Color {
        Color::new(1., 1., 1., 1.)
    }

    #[test]
    fn occluder_clips_quads_and_proportional_texture_coordinates() {
        let canvas = Canvas::default();
        // Push an occluder covering (20, 20, 20, 20)
        canvas.push_occluder(Rect::new(20., 20., 20., 20.));

        // 1. Quad completely outside: should be untouched
        canvas.quad(
            None,
            Rect::new(0., 0., 1., 1.),
            Rect::new(50., 50., 10., 10.),
            white(),
        );

        // 2. Quad completely inside: should be dropped
        canvas.quad(
            None,
            Rect::new(0., 0., 1., 1.),
            Rect::new(22., 22., 5., 5.),
            white(),
        );

        // 3. Quad partially overlapping: (10, 10, 20, 20), overlaps (20, 20, 10, 10)
        // Pieces should be: top (10, 10, 20, 10) and left (10, 20, 10, 10)
        canvas.quad(
            None,
            Rect::new(0., 0., 100., 100.),
            Rect::new(10., 10., 20., 20.),
            white(),
        );

        let quads = canvas.drain();
        assert_eq!(quads.len(), 3, "1 outside + 0 inside + 2 split pieces");
        assert_eq!(quads[0].destination, Rect::new(50., 50., 10., 10.));

        // Slices of quad 3:
        let top = &quads[1];
        assert_eq!(top.destination, Rect::new(10., 10., 20., 10.));
        assert_eq!(top.source, Rect::new(0., 0., 100., 50.));

        let left = &quads[2];
        assert_eq!(left.destination, Rect::new(10., 20., 10., 10.));
        assert_eq!(left.source, Rect::new(0., 50., 50., 50.));

        // Drain should have cleared occluders
        canvas.rectangle(25., 25., 10., 10., white());
        let quads = canvas.drain();
        assert_eq!(quads.len(), 1, "drain clears occluders");
    }

    #[test]
    fn a_frame_narrows_the_width_and_shifts_every_quad() {
        let canvas = Canvas::default();
        canvas.resize(3440., 1440.);
        canvas.rectangle(10., 20., 30., 40., white());
        canvas.set_frame(440., 2560.);
        assert_eq!((canvas.width(), canvas.frame_x()), (2560., 440.));
        assert_eq!(canvas.screen_size(), vec2(3440., 1440.));
        canvas.rectangle(10., 20., 30., 40., white());
        canvas.polygon([
            (vec2(0., 0.), white()),
            (vec2(0., 10.), white()),
            (vec2(5., 10.), white()),
            (vec2(5., 0.), white()),
        ]);
        canvas.screen_rectangle(white());
        canvas.clear_frame();
        assert_eq!((canvas.width(), canvas.frame_x()), (3440., 0.));
        let quads = canvas.drain();
        assert_eq!(quads[0].destination.x, 10., "before the frame");
        assert_eq!(quads[1].destination.x, 450., "inside the frame");
        assert_eq!(quads[2].destination.x, 440.);
        assert_eq!(quads[2].vertices.unwrap()[2].0, vec2(445., 10.));
        let full = quads[3].destination;
        assert_eq!(
            (full.x, full.w),
            (0., 3440.),
            "screen rectangles ignore the frame"
        );
    }
}
