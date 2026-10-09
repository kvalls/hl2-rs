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
    additive: bool,
    commands: Vec<Quad>,
}
#[derive(Clone)]
pub struct Canvas(Arc<Mutex<State>>);
impl Default for Canvas {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(State {
            viewport: vec2(1280., 720.),
            additive: false,
            commands: vec![],
        })))
    }
}
impl Canvas {
    pub fn resize(&self, width: f32, height: f32) {
        self.0.lock().expect("HUD canvas").viewport = vec2(width, height);
    }
    pub fn width(&self) -> f32 {
        self.0.lock().expect("HUD canvas").viewport.x
    }
    pub fn height(&self) -> f32 {
        self.0.lock().expect("HUD canvas").viewport.y
    }
    pub fn additive(&self, value: bool) {
        self.0.lock().expect("HUD canvas").additive = value;
    }
    pub fn drain(&self) -> Vec<Quad> {
        std::mem::take(&mut self.0.lock().expect("HUD canvas").commands)
    }
    fn quad(&self, texture: Option<Texture2D>, source: Rect, destination: Rect, color: Color) {
        if destination.w <= 0. || destination.h <= 0. || color.a <= 0. {
            return;
        }
        let mut state = self.0.lock().expect("HUD canvas");
        let additive = state.additive;
        state.commands.push(Quad {
            texture,
            source,
            destination,
            color,
            additive,
            modulate: false,
            vertices: None,
        });
    }
    /// An untextured quad with per-vertex positions and colors (blend as set).
    pub fn polygon(&self, vertices: [(Vec2, Color); 4]) {
        if vertices.iter().all(|(_, c)| c.a <= 0.) {
            return;
        }
        let min = vertices.iter().fold(Vec2::MAX, |m, (p, _)| m.min(*p));
        let max = vertices.iter().fold(Vec2::MIN, |m, (p, _)| m.max(*p));
        let mut state = self.0.lock().expect("HUD canvas");
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
