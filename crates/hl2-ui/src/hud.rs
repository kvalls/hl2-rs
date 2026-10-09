//! The PC HL2 HUD layout is read from the owned game's scheme and resource files.
//! Bucket positioning and visibility follow CHudWeaponSelection, rather than a new UI design.
use crate::canvas::*;
use anyhow::{Context, Result};
use glam::{vec2, Vec2};
use hl2_simulation::{
    gameplay::{Inventory, Weapon},
    selection::{self, Selection, WEAPON_SLOTS},
};
use source_assets::{
    keyvalues::{self, Entry},
    vpk::Vfs,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

fn find<'a>(entries: &'a [Entry], name: &str) -> Option<&'a Entry> {
    entries.iter().find(|e| e.key.eq_ignore_ascii_case(name))
}
fn number(entry: &Entry, key: &str, fallback: f32) -> f32 {
    entry
        .get(key)
        .and_then(Entry::text)
        .and_then(|s| s.parse().ok())
        .unwrap_or(fallback)
}
fn rgba(value: &str) -> Option<Color> {
    let components = value
        .split_whitespace()
        .map(str::parse::<u8>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    (components.len() == 4)
        .then(|| Color::from_rgba(components[0], components[1], components[2], components[3]))
}
fn color(settings: &Entry, name: &str, fallback: Color) -> Color {
    settings
        .get(name)
        .and_then(Entry::text)
        .and_then(rgba)
        .unwrap_or(fallback)
}
fn alpha(mut color: Color, amount: f32) -> Color {
    color.a *= amount;
    color
}
fn uses_secondary_ammo(weapon: &Weapon) -> bool {
    !weapon.secondary_ammo_type.is_empty()
        && !weapon.secondary_ammo_type.eq_ignore_ascii_case("none")
}

#[derive(Clone)]
pub(crate) struct FontFace {
    canvas: Canvas,
    font: Arc<fontdue::Font>,
    glyphs: GlyphCache,
    cell_height: f32,
    ascent: f32,
}
type GlyphCache = Arc<MutexCell<HashMap<(char, u32, u16, u16), Glyph>>>;
#[derive(Clone)]
struct Glyph {
    texture: Option<Texture2D>,
    x: f32,
    y: f32,
    advance: f32,
}
impl FontFace {
    pub(crate) fn load(bytes: &[u8], canvas: Canvas) -> Result<Self> {
        let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(anyhow::Error::msg)?;
        let face = ttf_parser::Face::parse(bytes, 0).context("read HUD font metrics")?;
        let units = f32::from(face.units_per_em()).max(1.);
        let (ascent, cell_height) = face.tables().os2.map_or_else(
            || {
                let metrics = font.horizontal_line_metrics(1.).unwrap();
                (metrics.ascent, metrics.ascent - metrics.descent)
            },
            |table| {
                let ascent = f32::from(table.windows_ascender()) / units;
                let descent = f32::from(table.windows_descender()).abs() / units;
                (ascent, ascent + descent)
            },
        );
        Ok(Self {
            canvas,
            font: Arc::new(font),
            glyphs: Arc::new(MutexCell::new(HashMap::new())),
            cell_height: cell_height.max(0.01),
            ascent,
        })
    }
    fn width(&self, value: &str, size: f32) -> f32 {
        // Retail CWin32Font uses a positive CreateFontA height: a character cell,
        // whereas fontdue's size is pixels per EM. Windows OS/2 metrics supply the
        // conversion; hhea descent differs significantly in the owned HalfLife2 font.
        let size = size.round().max(1.) / self.cell_height;
        value
            .chars()
            .map(|c| self.font.metrics(c, size).advance_width)
            .sum()
    }
    fn glyph(&self, c: char, size: u16, blur: u16, scanlines: u16) -> Glyph {
        self.raster(c, f32::from(size) / self.cell_height, blur, scanlines)
    }
    fn raster(&self, c: char, em_size: f32, blur: u16, scanlines: u16) -> Glyph {
        let key = (c, em_size.to_bits(), blur, scanlines);
        if let Some(glyph) = self.glyphs.borrow().get(&key) {
            return glyph.clone();
        }
        let (metrics, coverage) = self.font.rasterize(c, em_size);
        let margin = usize::from(blur);
        let width = metrics.width + margin * 2;
        let height = metrics.height + margin * 2;
        let ascent = self.ascent * em_size;
        let texture = if metrics.width == 0 || metrics.height == 0 {
            None
        } else {
            let mut bitmap = vec![0.; width * height];
            for y in 0..metrics.height {
                for x in 0..metrics.width {
                    bitmap[(y + margin) * width + x + margin] =
                        f32::from(coverage[y * metrics.width + x]) / 255.;
                }
            }
            if blur > 0 {
                bitmap = gaussian_blur(&bitmap, width, height, margin);
            }
            let rgba = bitmap
                .iter()
                .enumerate()
                .flat_map(|(i, coverage)| {
                    // The configured scanline pitch is retained. Exact GDI scanline and
                    // blur rasterization are not claimed equivalent to VGUI's native font engine.
                    let brightness = if scanlines > 1 && (i / width) % usize::from(scanlines) != 0 {
                        180
                    } else {
                        255
                    };
                    [
                        brightness,
                        brightness,
                        brightness,
                        (coverage.clamp(0., 1.) * 255.).round() as u8,
                    ]
                })
                .collect::<Vec<_>>();
            let texture = Texture2D::from_rgba8(width as u16, height as u16, &rgba);
            texture.set_filter(FilterMode::Linear);
            Some(texture)
        };
        let glyph = Glyph {
            texture,
            x: metrics.xmin as f32 - f32::from(blur),
            y: ascent - metrics.ymin as f32 - metrics.height as f32 - f32::from(blur),
            advance: metrics.advance_width,
        };
        self.glyphs.borrow_mut().insert(key, glyph.clone());
        glyph
    }
    fn draw(&self, value: &str, mut x: f32, y: f32, size: f32, effects: (u16, u16), color: Color) {
        let (blur, scanlines) = effects;
        let size = size.round().clamp(1., 1024.) as u16;
        for c in value.chars() {
            let glyph = self.glyph(c, size, blur.min(64), scanlines);
            if let Some(texture) = &glyph.texture {
                draw_texture(&self.canvas, texture, x + glyph.x, y + glyph.y, color);
            }
            x += glyph.advance;
        }
    }
    pub(crate) fn face(&self) -> Arc<fontdue::Font> {
        self.font.clone()
    }
    pub(crate) fn width_em(&self, value: &str, em: f32) -> f32 {
        value
            .chars()
            .map(|c| self.font.metrics(c, em).advance_width)
            .sum()
    }
    pub(crate) fn draw_baseline(
        &self,
        value: &str,
        mut origin: Vec2,
        em: f32,
        scale: f32,
        color: Color,
    ) {
        for c in value.chars() {
            let glyph = self.raster(c, em, 0, 0);
            if let Some(texture) = &glyph.texture {
                draw_texture_ex(
                    &self.canvas,
                    texture,
                    origin.x + glyph.x * scale,
                    origin.y + (glyph.y - self.ascent * em) * scale,
                    color,
                    DrawTextureParams {
                        dest_size: Some(texture.size() * scale),
                        ..Default::default()
                    },
                );
            }
            origin.x += glyph.advance * scale;
        }
    }
    fn draw_cropped(
        &self,
        value: &str,
        origin: Vec2,
        size: f32,
        effects: (u16, u16),
        color: Color,
        rows: Vec2,
    ) {
        let (blur, scanlines) = effects;
        let size = size.round().clamp(1., 1024.) as u16;
        let mut x = origin.x;
        for c in value.chars() {
            let glyph = self.glyph(c, size, blur.min(64), scanlines);
            if let Some(texture) = &glyph.texture {
                let first = (rows.x - glyph.y).clamp(0., texture.height());
                let last = (rows.y - glyph.y).clamp(0., texture.height());
                if last > first {
                    draw_texture_ex(
                        &self.canvas,
                        texture,
                        x + glyph.x,
                        origin.y + glyph.y + first,
                        color,
                        DrawTextureParams {
                            source: Some(Rect::new(0., first, texture.width(), last - first)),
                            ..Default::default()
                        },
                    );
                }
            }
            x += glyph.advance;
        }
    }
}

fn gaussian_blur(bitmap: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    let sigma = (radius as f32 * 0.5).max(0.5);
    let mut kernel = (-(radius as isize)..=radius as isize)
        .map(|x| (-(x * x) as f32 / (2. * sigma * sigma)).exp())
        .collect::<Vec<_>>();
    let sum: f32 = kernel.iter().sum();
    for weight in &mut kernel {
        *weight /= sum;
    }
    let mut horizontal = vec![0.; bitmap.len()];
    let mut result = vec![0.; bitmap.len()];
    for y in 0..height {
        for x in 0..width {
            for (i, weight) in kernel.iter().enumerate() {
                let sample = x as isize + i as isize - radius as isize;
                if sample >= 0 && (sample as usize) < width {
                    horizontal[y * width + x] += bitmap[y * width + sample as usize] * weight;
                }
            }
        }
    }
    for y in 0..height {
        for x in 0..width {
            for (i, weight) in kernel.iter().enumerate() {
                let sample = y as isize + i as isize - radius as isize;
                if sample >= 0 && (sample as usize) < height {
                    result[y * width + x] += horizontal[sample as usize * width + x] * weight;
                }
            }
        }
    }
    result
}

struct FontRange {
    minimum: f32,
    maximum: f32,
    tall: f32,
    proportional: bool,
    blur: f32,
    scanlines: f32,
    #[cfg(windows)]
    antialias: bool,
}
struct HudFont {
    face: FontFace,
    ranges: Vec<FontRange>,
}
#[cfg(windows)]
impl FontRange {
    fn monochrome_crosshair(&self) -> bool {
        !self.antialias && !self.proportional && self.blur == 0. && self.scanlines == 0.
    }
}
impl HudFont {
    fn read(fonts: &Entry, name: &str, face: FontFace, fallback: f32) -> Self {
        let ranges = fonts
            .get(name)
            .map(Entry::children)
            .unwrap_or_default()
            .iter()
            .map(|entry| {
                let resolution = entry.get("yres").and_then(Entry::text).and_then(|s| {
                    let values = s
                        .split_whitespace()
                        .map(str::parse::<f32>)
                        .collect::<std::result::Result<Vec<_>, _>>()
                        .ok()?;
                    (values.len() == 2).then_some((values[0], values[1]))
                });
                FontRange {
                    minimum: resolution.map_or(0., |r| r.0),
                    maximum: resolution.map_or(f32::MAX, |r| r.1),
                    tall: number(entry, "tall", fallback),
                    proportional: resolution.is_none(),
                    blur: number(entry, "blur", 0.),
                    scanlines: number(entry, "scanlines", 0.),
                    #[cfg(windows)]
                    antialias: number(entry, "antialias", 1.) != 0.,
                }
            })
            .collect();
        Self { face, ranges }
    }
    fn size(&self) -> f32 {
        let height = self.face.canvas.height();
        let range = self
            .ranges
            .iter()
            .find(|r| height >= r.minimum && height <= r.maximum);
        range.map_or(12., |r| {
            r.tall * if r.proportional { height / 480. } else { 1. }
        })
    }
    fn effects(&self) -> (u16, u16) {
        let height = self.face.canvas.height();
        let range = self
            .ranges
            .iter()
            .find(|r| height >= r.minimum && height <= r.maximum);
        range.map_or((0, 0), |r| {
            let scale = if r.proportional { height / 480. } else { 1. };
            (
                (r.blur * scale).round() as u16,
                (r.scanlines * scale).round() as u16,
            )
        })
    }
    fn draw(&self, value: &str, x: f32, y: f32, color: Color) {
        let (blur, scanlines) = self.effects();
        self.face
            .draw(value, x, y, self.size(), (blur, scanlines), color);
    }
    fn width(&self, value: &str) -> f32 {
        self.face.width(value, self.size())
    }
    fn draw_cropped(&self, value: &str, origin: Vec2, rows: Vec2, color: Color) {
        self.face
            .draw_cropped(value, origin, self.size(), self.effects(), color, rows);
    }
}

#[derive(Clone)]
struct PanelLayout {
    x: String,
    y: f32,
    width: f32,
    height: f32,
    text: Vec2,
    digit: Vec2,
    secondary: Vec2,
}
impl PanelLayout {
    fn read(entry: &Entry) -> Self {
        Self {
            x: entry
                .get("xpos")
                .and_then(Entry::text)
                .unwrap_or("0")
                .into(),
            y: number(entry, "ypos", 432.),
            width: number(entry, "wide", 102.),
            height: number(entry, "tall", 36.),
            text: vec2(
                number(entry, "text_xpos", 8.),
                number(entry, "text_ypos", 20.),
            ),
            digit: vec2(
                number(entry, "digit_xpos", 50.),
                number(entry, "digit_ypos", 2.),
            ),
            secondary: vec2(
                number(entry, "digit2_xpos", 98.),
                number(entry, "digit2_ypos", 16.),
            ),
        }
    }
    fn rect(&self, scale: f32, viewport_width: f32) -> Rect {
        let x = panel_axis(&self.x, viewport_width, scale).unwrap_or(0.);
        Rect::new(x, self.y * scale, self.width * scale, self.height * scale)
    }
    fn rect_for(&self, viewport: Vec2) -> Rect {
        let scale = viewport.y / 480.;
        Rect::new(
            panel_axis(&self.x, viewport.x, scale).unwrap_or(0.),
            self.y * scale,
            self.width * scale,
            self.height * scale,
        )
    }
}
fn panel_axis(value: &str, extent: f32, scale: f32) -> Option<f32> {
    let (alignment, number) = if let Some(number) = value.strip_prefix('r') {
        (1, number)
    } else if let Some(number) = value.strip_prefix('c') {
        (2, number)
    } else {
        (0, value)
    };
    let number = number.parse::<f32>().ok()? * scale;
    let value = match alignment {
        1 => extent - number,
        2 => extent * 0.5 + number,
        _ => number,
    };
    value.is_finite().then_some(value)
}

#[derive(Clone)]
struct Animate {
    event: String,
    panel: String,
    property: String,
    target: String,
    delay: f64,
    duration: f64,
    interpolation: Interpolation,
}

#[derive(Clone, Copy)]
enum Interpolation {
    Linear,
    Accel,
    Deaccel,
    Spline,
}
impl Interpolation {
    fn read(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "linear" => Some(Self::Linear),
            "accel" => Some(Self::Accel),
            "deaccel" => Some(Self::Deaccel),
            "spline" => Some(Self::Spline),
            _ => None,
        }
    }
    fn sample(self, position: f32) -> f32 {
        let position = position.clamp(0., 1.);
        // VGUI AnimationController::GetInterpolatedValue, including its square-root Deaccel.
        match self {
            Self::Linear => position,
            Self::Accel => position * position,
            Self::Deaccel => position.sqrt(),
            Self::Spline => position * position * (3. - 2. * position),
        }
    }
}

#[derive(Clone)]
struct EventRule {
    event: String,
    target: String,
    delay: f64,
    run: bool,
    panel: bool,
    property: Option<String>,
}

fn animation_lines(source: &str) -> Result<Vec<(String, Vec<String>)>> {
    let mut event = String::new();
    let mut event_enabled = true;
    let mut lines = Vec::new();
    for line in source.lines() {
        let mut tokens = keyvalues::tokens(line)?;
        let enabled = if tokens
            .last()
            .is_some_and(|t| t.starts_with('[') && t.ends_with(']'))
        {
            keyvalues::resource_condition(&tokens.pop().unwrap())?
        } else {
            true
        };
        if tokens.first().map(String::as_str) == Some("event") && tokens.len() >= 2 {
            event = tokens[1].clone();
            event_enabled = enabled;
        } else if enabled && event_enabled && !tokens.is_empty() {
            lines.push((event.clone(), tokens));
        }
    }
    Ok(lines)
}
/// CHudDamageIndicator::GetDamagePosition: the yaw (degrees, 0..360) of a damage
/// direction around the flattened view; 90 is left, 180 behind, 270 right.
fn damage_angle(delta: glam::Vec3, view_yaw: f32) -> f32 {
    let yaw = view_yaw.to_radians();
    let forward = glam::Vec3::new(yaw.cos(), yaw.sin(), 0.);
    let right = glam::Vec3::Z.cross(forward);
    let front = delta.dot(forward);
    let side = delta.dot(right);
    let (x, y) = (360. * -side, 360. * -front);
    (x.atan2(y) + std::f32::consts::PI).to_degrees()
}
fn animations(source: &str) -> Result<Vec<Animate>> {
    let mut result = Vec::new();
    for (event, tokens) in animation_lines(source)? {
        if tokens.first().map(String::as_str) == Some("Animate") && tokens.len() >= 7 {
            let Some(interpolation) = Interpolation::read(&tokens[4]) else {
                // Pulse/Bias/Gain/Flicker take additional parameters and are not this subset.
                continue;
            };
            let (Ok(delay), Ok(duration)) = (tokens[5].parse::<f64>(), tokens[6].parse::<f64>())
            else {
                continue;
            };
            if !delay.is_finite() || !duration.is_finite() || delay < 0. || duration < 0. {
                continue;
            }
            result.push(Animate {
                event,
                panel: tokens[1].clone(),
                property: tokens[2].clone(),
                target: tokens[3].clone(),
                delay,
                duration,
                interpolation,
            });
        }
    }
    Ok(result)
}
fn event_rules(source: &str) -> Result<Vec<EventRule>> {
    Ok(animation_lines(source)?
        .into_iter()
        .filter_map(|(event, tokens)| {
            if !matches!(
                tokens.first().map(String::as_str),
                Some("StopEvent" | "RunEvent" | "StopPanelAnimations" | "StopAnimation")
            ) || tokens.len() < 3
            {
                return None;
            }
            let property = (tokens[0] == "StopAnimation")
                .then(|| tokens.get(2).cloned())
                .flatten();
            let delay = tokens
                .get(if property.is_some() { 3 } else { 2 })?
                .parse::<f64>()
                .ok()?;
            (delay.is_finite() && delay >= 0.).then(|| EventRule {
                event,
                target: tokens[1].clone(),
                delay,
                run: tokens[0] == "RunEvent",
                panel: tokens[0] == "StopPanelAnimations",
                property,
            })
        })
        .collect())
}
fn animation<'a>(
    rules: &'a [Animate],
    event: &str,
    panel: &str,
    property: &str,
) -> Option<&'a Animate> {
    rules.iter().find(|a| {
        a.event.eq_ignore_ascii_case(event)
            && a.panel.eq_ignore_ascii_case(panel)
            && a.property.eq_ignore_ascii_case(property)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NumericPanel {
    Health,
    Suit,
    Ammo,
    AmmoSecondary,
    DamageIndicator,
}
impl NumericPanel {
    fn read(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "hudhealth" => Some(Self::Health),
            "hudsuit" => Some(Self::Suit),
            "hudammo" => Some(Self::Ammo),
            "hudammosecondary" => Some(Self::AmmoSecondary),
            "huddamageindicator" => Some(Self::DamageIndicator),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum NumericProperty {
    Alpha,
    Blur,
    Background,
    Foreground,
    TextColor,
    Ammo2Color,
    Position,
    Size,
    DmgColorLeft,
    DmgColorRight,
    DmgHighColorLeft,
    DmgHighColorRight,
    DmgFullscreenColor,
}
impl NumericProperty {
    fn read(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "alpha" => Some(Self::Alpha),
            "blur" => Some(Self::Blur),
            "bgcolor" => Some(Self::Background),
            "fgcolor" => Some(Self::Foreground),
            "textcolor" => Some(Self::TextColor),
            "ammo2color" => Some(Self::Ammo2Color),
            "position" => Some(Self::Position),
            "size" => Some(Self::Size),
            "dmgcolorleft" => Some(Self::DmgColorLeft),
            "dmgcolorright" => Some(Self::DmgColorRight),
            "dmghighcolorleft" => Some(Self::DmgHighColorLeft),
            "dmghighcolorright" => Some(Self::DmgHighColorRight),
            "dmgfullscreencolor" => Some(Self::DmgFullscreenColor),
            _ => None,
        }
    }
    fn is_color(self) -> bool {
        matches!(
            self,
            Self::Background
                | Self::Foreground
                | Self::TextColor
                | Self::Ammo2Color
                | Self::DmgColorLeft
                | Self::DmgColorRight
                | Self::DmgHighColorLeft
                | Self::DmgHighColorRight
                | Self::DmgFullscreenColor
        )
    }
    fn geometry(self) -> bool {
        matches!(self, Self::Position | Self::Size)
    }
    fn target(self, value: &str, settings: &Entry, viewport: Vec2) -> Option<[f32; 4]> {
        if self.geometry() {
            let parts = value.split_whitespace().collect::<Vec<_>>();
            if parts.len() != 2 || viewport.x <= 0. || viewport.y <= 0. {
                return None;
            }
            let scale = viewport.y / 480.;
            let extent = if self == Self::Position {
                viewport
            } else {
                Vec2::ZERO
            };
            let x = panel_axis(parts[0], extent.x, scale)?;
            let y = panel_axis(parts[1], extent.y, scale)?;
            if self == Self::Size && (x < 0. || y < 0.) {
                return None;
            }
            Some([x, y, 0., 0.])
        } else if self.is_color() {
            let value =
                rgba(value).or_else(|| settings.get(value).and_then(Entry::text).and_then(rgba))?;
            Some([
                value.r * 255.,
                value.g * 255.,
                value.b * 255.,
                value.a * 255.,
            ])
        } else {
            let value = value.parse::<f32>().ok()?;
            value.is_finite().then_some([value, 0., 0., 0.])
        }
    }
}
#[derive(Clone, Copy)]
struct PanelEffects {
    values: [[f32; 4]; 13],
}
impl PanelEffects {
    fn new(settings: &Entry) -> Self {
        let foreground = color(settings, "FgColor", Color::from_rgba(255, 220, 0, 100));
        let background = color(settings, "BgColor", Color::from_rgba(0, 0, 0, 76));
        let components = |c: Color| [c.r * 255., c.g * 255., c.b * 255., c.a * 255.];
        Self {
            values: [
                [255., 0., 0., 0.],
                [0.; 4],
                components(background),
                components(foreground),
                components(foreground),
                [0.; 4],
                [0.; 4],
                components(foreground),
                // CHudDamageIndicator's CPanelAnimationVar defaults ("255 0 0 0").
                [255., 0., 0., 0.],
                [255., 0., 0., 0.],
                [255., 0., 0., 0.],
                [255., 0., 0., 0.],
                [255., 0., 0., 0.],
            ],
        }
    }
    fn opacity(self) -> f32 {
        self.values[NumericProperty::Alpha as usize][0].clamp(0., 255.) / 255.
    }
    fn color(self, property: NumericProperty) -> Color {
        // Native VGUI stores interpolated color channels as bytes.
        let channels = self.values[property as usize].map(|v| v.clamp(0., 255.) as u8);
        alpha(
            Color::from_rgba(channels[0], channels[1], channels[2], channels[3]),
            self.opacity(),
        )
    }
    fn glow(self) -> impl Iterator<Item = f32> {
        let blur = self.values[NumericProperty::Blur as usize][0].clamp(0., 64.);
        (0..blur.ceil() as usize).map(move |pass| (blur - pass as f32).min(1.))
    }
    fn set_rect(&mut self, rect: Rect) {
        self.values[NumericProperty::Position as usize] = [rect.x, rect.y, 0., 0.];
        self.values[NumericProperty::Size as usize] = [rect.w, rect.h, 0., 0.];
    }
    fn rect(self) -> Option<Rect> {
        let position = self.values[NumericProperty::Position as usize];
        let size = self.values[NumericProperty::Size as usize];
        (size[0] > 0. && size[1] > 0.).then(|| {
            // VGUI applies animated geometry through integer panel coordinates.
            Rect::new(
                position[0].trunc(),
                position[1].trunc(),
                size[0].trunc(),
                size[1].trunc(),
            )
        })
    }
}
struct PropertyAnimation {
    event: String,
    panel: NumericPanel,
    property: NumericProperty,
    target: [f32; 4],
    from: Option<[f32; 4]>,
    start: f64,
    end: f64,
    interpolation: Interpolation,
}
struct PostedEvent {
    event: String,
    target: String,
    at: f64,
    run: bool,
    panel: bool,
    property: Option<String>,
}
struct NumericHudEffects {
    panels: [PanelEffects; 5],
    animations: Vec<PropertyAnimation>,
    posted: Vec<PostedEvent>,
    health: i32,
    armor: i32,
    active: String,
    ammo: i32,
    reserve: i32,
    secondary_active: String,
    secondary_ammo: i32,
    time: f64,
    viewport: Vec2,
    initial_rects: Option<[Rect; 4]>,
}
impl NumericHudEffects {
    fn new(settings: &Entry) -> Self {
        let mut panels = [PanelEffects::new(settings); 5];
        // CHudSecondaryAmmo::Reset starts hidden until a secondary-ammo weapon is observed.
        panels[NumericPanel::AmmoSecondary as usize].values[NumericProperty::Alpha as usize][0] =
            0.;
        Self {
            panels,
            animations: Vec::new(),
            posted: Vec::new(),
            health: -1,
            armor: -1,
            active: String::new(),
            ammo: -1,
            reserve: -1,
            secondary_active: String::new(),
            secondary_ammo: -1,
            time: 0.,
            viewport: Vec2::ZERO,
            initial_rects: None,
        }
    }
    fn stop(&mut self, event: &str) {
        self.animations
            .retain(|a| !a.event.eq_ignore_ascii_case(event));
        self.posted.retain(|s| !s.event.eq_ignore_ascii_case(event));
    }
    fn stop_panel(&mut self, panel: &str) {
        if let Some(panel) = NumericPanel::read(panel) {
            self.animations.retain(|a| a.panel != panel);
        }
    }
    fn stop_property(&mut self, panel: &str, property: &str) {
        if let (Some(panel), Some(property)) =
            (NumericPanel::read(panel), NumericProperty::read(property))
        {
            self.animations
                .retain(|a| a.panel != panel || a.property != property);
        }
    }
    fn configure_layouts(&mut self, layouts: [&PanelLayout; 4], viewport: Vec2) {
        if self.viewport == viewport && self.initial_rects.is_some() {
            return;
        }
        self.viewport = viewport;
        let rects = layouts.map(|layout| layout.rect_for(viewport));
        self.initial_rects = Some(rects);
        for (panel, rect) in self.panels.iter_mut().zip(rects) {
            panel.set_rect(rect);
        }
        // A viewport change reapplies proportional resource geometry. Old pixel targets
        // no longer apply; current weapon sequences will supply freshly scaled targets.
        self.animations.retain(|a| !a.property.geometry());
        self.active.clear();
        self.secondary_active.clear();
    }
    fn start(
        &mut self,
        event: &str,
        time: f64,
        rules: &[Animate],
        events: &[EventRule],
        settings: &Entry,
    ) {
        // StartAnimationSequence removes earlier commands from the same sequence.
        self.stop(event);
        for rule in events
            .iter()
            .filter(|s| s.event.eq_ignore_ascii_case(event))
        {
            if let Some(property) = &rule.property {
                if rule.delay == 0. {
                    self.stop_property(&rule.target, property);
                    continue;
                }
            }
            if rule.panel && rule.delay == 0. {
                // The owned secondary-weapon sequences stop existing panel tracks
                // before scheduling their replacement color/alpha tracks.
                self.stop_panel(&rule.target);
                continue;
            }
            self.posted.push(PostedEvent {
                event: event.into(),
                target: rule.target.clone(),
                at: time + rule.delay,
                run: rule.run,
                panel: rule.panel,
                property: rule.property.clone(),
            });
        }
        for rule in rules.iter().filter(|r| r.event.eq_ignore_ascii_case(event)) {
            let Some(panel) = NumericPanel::read(&rule.panel) else {
                continue;
            };
            let Some(property) = NumericProperty::read(&rule.property) else {
                continue;
            };
            let Some(target) = property.target(&rule.target, settings, self.viewport) else {
                continue;
            };
            self.animations.push(PropertyAnimation {
                event: event.into(),
                panel,
                property,
                target,
                from: None,
                start: time + rule.delay,
                end: time + rule.delay + rule.duration,
                interpolation: rule.interpolation,
            });
        }
    }
    fn posted_events(
        &mut self,
        time: f64,
        rules: &[Animate],
        events: &[EventRule],
        settings: &Entry,
    ) {
        // AnimationController processes posted messages in insertion order, restarting
        // its traversal after each dispatch. Each RunEvent target may run once per frame.
        let mut ran = HashSet::new();
        while let Some(index) = self.posted.iter().position(|message| message.at <= time) {
            let message = self.posted.remove(index);
            if let Some(property) = message.property {
                self.stop_property(&message.target, &property);
            } else if message.panel {
                self.stop_panel(&message.target);
            } else if message.run {
                if ran.insert(message.target.to_ascii_lowercase()) {
                    // Source starts late events at the current frame, without backlog catch-up.
                    self.start(&message.target, time, rules, events, settings);
                }
            } else {
                self.stop(&message.target);
            }
        }
    }
    fn advance(&mut self, time: f64) {
        let mut index = 0;
        while index < self.animations.len() {
            let track = &mut self.animations[index];
            if time < track.start {
                index += 1;
                continue;
            }
            let value = &mut self.panels[track.panel as usize].values[track.property as usize];
            // Delayed commands capture the current property when they first become active.
            let from = *track.from.get_or_insert(*value);
            let position = if time >= track.end {
                1.
            } else {
                ((time - track.start) / (track.end - track.start)) as f32
            };
            let fraction = track.interpolation.sample(position);
            *value = std::array::from_fn(|channel| {
                from[channel] + (track.target[channel] - from[channel]) * fraction
            });
            if time >= track.end {
                self.animations.remove(index);
            } else {
                index += 1;
            }
        }
        self.time = time;
    }
    fn observe(
        &mut self,
        inv: &Inventory,
        weapons: &BTreeMap<String, Weapon>,
        time: f64,
        rules: &[Animate],
        events: &[EventRule],
        settings: &Entry,
    ) {
        if !time.is_finite() {
            return;
        }
        if time < self.time {
            let viewport = self.viewport;
            let rects = self.initial_rects;
            *self = Self::new(settings);
            self.viewport = viewport;
            self.initial_rects = rects;
            if let Some(rects) = rects {
                for (panel, rect) in self.panels.iter_mut().zip(rects) {
                    panel.set_rect(rect);
                }
            }
        }
        self.advance(time);
        let health = (inv.health as i32).max(0);
        if health != self.health {
            if health >= 20 {
                self.start("HealthIncreasedAbove20", time, rules, events, settings);
            } else if health > 0 {
                self.start("HealthIncreasedBelow20", time, rules, events, settings);
                self.start("HealthLow", time, rules, events, settings);
            }
            // CHudHealth starts nothing at zero; HudPlayerDeath comes from the
            // damage indicator's message handler.
            self.health = health;
        }
        let armor = inv.armor as i32;
        if armor != self.armor {
            if armor == 0 {
                self.start("SuitPowerZero", time, rules, events, settings);
            } else if armor < self.armor {
                self.start("SuitDamageTaken", time, rules, events, settings);
                if armor < 20 {
                    self.start("SuitArmorLow", time, rules, events, settings);
                }
            } else {
                let event = if self.armor == -1 || self.armor == 0 || armor >= 20 {
                    "SuitPowerIncreasedAbove20"
                } else {
                    "SuitPowerIncreasedBelow20"
                };
                self.start(event, time, rules, events, settings);
            }
            self.armor = armor;
        }
        if let Some(weapon) = weapons
            .get(&inv.active)
            .filter(|w| !w.ammo_type.is_empty() && !w.ammo_type.eq_ignore_ascii_case("none"))
        {
            let clip = inv.owned.get(&inv.active).copied().unwrap_or(-1);
            let reserve = inv.reserve_for(&inv.active, weapons);
            let ammo = if clip < 0 { reserve } else { clip };
            let ammo2 = if clip < 0 { 0 } else { reserve };
            if ammo != self.ammo {
                let event = if ammo == 0 {
                    "AmmoEmpty"
                } else if ammo < self.ammo {
                    "AmmoDecreased"
                } else {
                    "AmmoIncreased"
                };
                self.start(event, time, rules, events, settings);
                self.ammo = ammo;
            }
            if ammo2 != self.reserve {
                let event = if ammo2 == 0 {
                    "Ammo2Empty"
                } else if ammo2 < self.reserve {
                    "Ammo2Decreased"
                } else {
                    "Ammo2Increased"
                };
                self.start(event, time, rules, events, settings);
                self.reserve = ammo2;
            }
            if self.active != inv.active {
                // Retail/SDK SetAmmo's playAnimation argument is unused; WeaponChanged follows.
                self.start(
                    if clip < 0 {
                        "WeaponDoesNotUseClips"
                    } else {
                        "WeaponUsesClips"
                    },
                    time,
                    rules,
                    events,
                    settings,
                );
                self.start("WeaponChanged", time, rules, events, settings);
                self.active.clone_from(&inv.active);
            }
            let _ = weapon;
        }
        if let Some(weapon) = weapons.get(&inv.active) {
            let uses_secondary = uses_secondary_ammo(weapon);
            if uses_secondary {
                let ammo = inv.secondary_for(&inv.active, weapons);
                if ammo != self.secondary_ammo {
                    let event = if ammo == 0 {
                        "AmmoSecondaryEmpty"
                    } else if ammo < self.secondary_ammo {
                        "AmmoSecondaryDecreased"
                    } else {
                        "AmmoSecondaryIncreased"
                    };
                    self.start(event, time, rules, events, settings);
                    self.secondary_ammo = ammo;
                }
            }
            if self.secondary_active != inv.active {
                self.start(
                    if uses_secondary {
                        "WeaponUsesSecondaryAmmo"
                    } else {
                        "WeaponDoesNotUseSecondaryAmmo"
                    },
                    time,
                    rules,
                    events,
                    settings,
                );
                self.secondary_active.clone_from(&inv.active);
            }
        } else {
            self.secondary_active.clear();
        }
        self.posted_events(time, rules, events, settings);
        self.advance(time);
    }
}

struct QuickInfoFade {
    at: f64,
    duration: f64,
    from: f32,
    target: f32,
}
struct QuickInfoState {
    health: i32,
    ammo: i32,
    event_at: f64,
    time: f64,
    alpha: f32,
    dimmed: bool,
    fade: Option<QuickInfoFade>,
    warn_health: bool,
    warn_ammo: bool,
    health_warning: f32,
    ammo_warning: f32,
    pending_sounds: Vec<String>,
}
impl Default for QuickInfoState {
    fn default() -> Self {
        Self {
            health: 100,
            ammo: 0,
            event_at: 0.,
            time: 0.,
            alpha: 255.,
            dimmed: false,
            fade: None,
            warn_health: false,
            warn_ammo: false,
            health_warning: 0.,
            ammo_warning: 0.,
            pending_sounds: Vec::new(),
        }
    }
}
impl QuickInfoState {
    fn observe(&mut self, health: i32, ammo: i32, maximum: i32, time: f64) -> f32 {
        if time < self.time {
            *self = Self::default();
        }
        let dt = (time - self.time).max(0.) as f32;
        self.time = time;
        if let Some(fade) = &self.fade {
            let position = ((time - fade.at) / fade.duration).clamp(0., 1.) as f32;
            self.alpha = fade.from + (fade.target - fade.from) * position;
        }
        // Published OnThink runs before Paint updates the activity timestamp.
        let dimmed = time - self.event_at > 1.;
        if dimmed != self.dimmed {
            self.dimmed = dimmed;
            self.fade = Some(QuickInfoFade {
                at: time,
                duration: if dimmed { 2. } else { 0.5 },
                from: self.alpha,
                target: if dimmed { 64. } else { 255. },
            });
        }
        if health != self.health {
            self.health = health;
            self.event_at = time;
            let warn = health <= 25;
            if warn && !self.warn_health {
                self.health_warning = 255.;
                self.pending_sounds.push("HUDQuickInfo.LowHealth".into());
            }
            self.warn_health = warn;
        }
        if ammo != self.ammo {
            self.ammo = ammo;
            self.event_at = time;
            // Retail client float at 0x10370f58 is 0.25, corroborating the SDK threshold.
            let warn = maximum > 1 && ammo as f32 / maximum as f32 <= 0.25;
            if warn && !self.warn_ammo {
                self.ammo_warning = 255.;
                self.pending_sounds.push("HUDQuickInfo.LowAmmo".into());
            }
            self.warn_ammo = warn;
        }
        dt
    }
    fn drain_sounds(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_sounds)
    }
    fn warning_color(
        warn: bool,
        remaining: &mut f32,
        time: f64,
        dt: f32,
        normal: Color,
        caution: Color,
    ) -> Option<(Color, bool)> {
        let pulse = ((time as f32 * 8.).sin().abs() * 128.) as i32;
        let full = *remaining > 0.;
        if full {
            if *remaining <= dt * 200. {
                if pulse < 40 {
                    *remaining = 0.;
                    return None;
                }
                *remaining += dt * 200.;
            }
            *remaining -= dt * 200.;
        }
        let mut color = if full || warn { caution } else { normal };
        // The original Color constructor stores this product in an unsigned byte.
        color.a = f32::from(if full || warn {
            (pulse * 255) as u8
        } else {
            138
        }) / 255.;
        Some((color, full))
    }
}
struct QuickInfoHud {
    canvas: Canvas,
    font: HudFont,
    center: Texture2D,
    center_rect: Rect,
    left: String,
    left_empty: String,
    right: String,
    right_empty: String,
    normal: Color,
    caution: Color,
    state: MutexCell<QuickInfoState>,
}
impl QuickInfoHud {
    fn load(vfs: &Vfs, fonts: &Entry, colors: &Entry, canvas: Canvas) -> Result<Self> {
        let face = FontFace::load(
            &vfs.read("resource/HL2crosshairs.ttf")?
                .context("QuickInfo font missing")?,
            canvas.clone(),
        )?;
        let resources = keyvalues::parse_resource(
            &vfs.read("scripts/hud_textures.txt")?
                .context("HUD textures missing")?,
        )?;
        let textures = resources
            .first()
            .and_then(|e| e.get("TextureData"))
            .context("HUD TextureData missing")?;
        let character = |name| -> Result<String> {
            Ok(textures
                .get(name)
                .and_then(|e| e.get("character"))
                .and_then(Entry::text)
                .context("QuickInfo glyph missing")?
                .into())
        };
        let center = textures
            .get("crosshair")
            .context("QuickInfo center missing")?;
        let file = center
            .get("file")
            .and_then(Entry::text)
            .context("QuickInfo center material missing")?;
        Ok(Self {
            canvas,
            font: HudFont::read(fonts, "QuickInfo", face, 28.),
            center: texture(vfs, file)?,
            center_rect: Rect::new(
                number(center, "x", 0.),
                number(center, "y", 0.),
                number(center, "width", 40.),
                number(center, "height", 40.),
            ),
            left: character("crosshair_left_full")?,
            left_empty: character("crosshair_left_empty")?,
            right: character("crosshair_right_full")?,
            right_empty: character("crosshair_right_empty")?,
            normal: color(colors, "Normal", Color::from_rgba(255, 208, 64, 255)),
            caution: color(colors, "Caution", Color::from_rgba(255, 48, 0, 255)),
            state: MutexCell::new(QuickInfoState::default()),
        })
    }
    fn progress(&self, full: &str, empty: &str, origin: Vec2, percentage: f32, color: Color) {
        let height = self.font.size().round();
        let offset = (height * percentage.clamp(0., 1.)).trunc();
        self.font
            .draw_cropped(empty, origin, vec2(0., offset), color);
        self.font
            .draw_cropped(full, origin, vec2(offset, height), color);
    }
    fn draw(&self, inv: &Inventory, weapons: &BTreeMap<String, Weapon>, time: f64) {
        let Some(weapon) = weapons.get(&inv.active) else {
            return;
        };
        if inv.health <= 0. || !time.is_finite() {
            return;
        }
        let ammo = inv.owned.get(&inv.active).copied().unwrap_or(-1);
        let mut state = self.state.borrow_mut();
        let dt = state.observe(inv.health as i32, ammo, weapon.magazine, time);
        let opacity = state.alpha.clamp(0., 255.) / 255.;
        let x = (self.canvas.width() * 0.5).trunc();
        let y = (self.canvas.height() * 0.5).trunc() - self.font.size().round() * 0.5;
        self.canvas.additive(true);
        draw_texture_ex(
            &self.canvas,
            &self.center,
            x,
            y,
            alpha(
                Color {
                    a: 138. / 255.,
                    ..self.normal
                },
                opacity,
            ),
            DrawTextureParams {
                source: Some(self.center_rect),
                ..Default::default()
            },
        );
        let warn_health = state.warn_health;
        if let Some((color, full)) = QuickInfoState::warning_color(
            warn_health,
            &mut state.health_warning,
            time,
            dt,
            self.normal,
            self.caution,
        ) {
            let origin = vec2(x - self.font.width(&self.left) * 2., y);
            let color = alpha(color, opacity);
            if full {
                self.font.draw(&self.left, origin.x, origin.y, color);
            } else {
                self.progress(
                    &self.left,
                    &self.left_empty,
                    origin,
                    1. - inv.health / 100.,
                    color,
                );
            }
        }
        let warn_ammo = state.warn_ammo;
        if let Some((color, full)) = QuickInfoState::warning_color(
            warn_ammo,
            &mut state.ammo_warning,
            time,
            dt,
            self.normal,
            self.caution,
        ) {
            let origin = vec2(x + self.font.width(&self.right), y);
            let color = alpha(color, opacity);
            if full {
                self.font.draw(&self.right, origin.x, origin.y, color);
            } else {
                let percentage = if weapon.magazine <= 0 {
                    0.
                } else {
                    1. - ammo as f32 / weapon.magazine as f32
                };
                self.progress(&self.right, &self.right_empty, origin, percentage, color);
            }
        }
        self.canvas.additive(false);
    }
}

fn sprite_filter(vtf: &[u8]) -> Result<FilterMode> {
    let flags = u32::from_le_bytes(
        vtf.get(20..24)
            .context("HUD sprite VTF flags missing")?
            .try_into()?,
    );
    Ok(if flags & 1 != 0 {
        FilterMode::Nearest
    } else {
        FilterMode::Linear
    })
}

fn default_crosshair_rects(viewport: Vec2, atlas: Rect, texture_size: Vec2) -> (Rect, Rect) {
    // Retail CHudCrosshair::Paint scales texture icons using ScreenHeight, not ScreenWidth.
    // Keep that integer threshold separate from proportional VGUI font sizing.
    let scale = (viewport.y / 1600.).floor() + 1.;
    let width = (atlas.w * scale + 0.5).trunc();
    let height = (atlas.h * scale + 0.5).trunc();
    let destination = Rect::new(
        (viewport.x * 0.5 + 0.5).trunc() - (width * 0.5).trunc(),
        (viewport.y * 0.5 + 0.5).trunc() - (height * 0.5).trunc(),
        width,
        height,
    );
    // CHud insets the sprite UVs, then ISurface remaps them through its texture entry's
    // own half-texel bounds. Both stages matter for the outer dots of the default sprite.
    let texture_range = (texture_size - Vec2::ONE) / texture_size;
    let source = Rect::new(
        0.5 + (atlas.x + 0.5) * texture_range.x,
        0.5 + (atlas.y + 0.5) * texture_range.y,
        (atlas.w - 1.) * texture_range.x,
        (atlas.h - 1.) * texture_range.y,
    );
    (destination, source)
}

pub struct WeaponHud {
    corners: Vec<Texture2D>,
    pub canvas: Canvas,
    icons: HudFont,
    selected_icons: HudFont,
    crosshairs: HudFont,
    #[cfg(windows)]
    native_crosshairs: BTreeMap<(char, u16), crate::native_font::Glyph>,
    numbers: HudFont,
    number_glow: HudFont,
    small_numbers: HudFont,
    ammo_icon_font: HudFont,
    selection_numbers: HudFont,
    selection_text: HudFont,
    labels: HudFont,
    layout: Entry,
    settings: Entry,
    animation_rules: Vec<Animate>,
    animation_events: Vec<EventRule>,
    numeric_effects: MutexCell<NumericHudEffects>,
    ammo_icons: BTreeMap<String, String>,
    secondary_ammo_icons: BTreeMap<String, String>,
    weapon_crosshairs: BTreeMap<String, String>,
    default_crosshair: Texture2D,
    default_crosshair_rect: Rect,
    default_crosshair_additive: bool,
    crosshair_color: Color,
    health_panel: PanelLayout,
    suit_panel: PanelLayout,
    ammo_panel: PanelLayout,
    secondary_ammo_panel: PanelLayout,
    quick_info: QuickInfoHud,
    /// HudDamageIndicator dmg_xpos, dmg_ypos, dmg_wide, dmg_tall1, dmg_tall2
    /// (proportional, 480-line units).
    damage_layout: [f32; 5],
    /// Client screen fades (CViewEffects).
    fades: MutexCell<crate::fades::ScreenFades>,
}

impl WeaponHud {
    /// Local-player warnings emitted by QuickInfo's false-to-true threshold latches.
    /// Drain after draw_status and forward these owned sound-script names to the scene.
    pub fn drain_sounds(&self) -> Vec<String> {
        self.quick_info.state.borrow_mut().drain_sounds()
    }

    pub fn load(vfs: &Vfs, canvas: Canvas) -> Result<Self> {
        let scheme = keyvalues::parse_resource(
            &vfs.read("resource/ClientScheme.res")?
                .context("ClientScheme.res missing")?,
        )?;
        let scheme = find(&scheme, "Scheme").context("Scheme block missing")?;
        let fonts = scheme.get("Fonts").context("HUD font scheme missing")?;
        let settings = scheme
            .get("BaseSettings")
            .context("HUD color scheme missing")?
            .clone();
        let quick_info = QuickInfoHud::load(
            vfs,
            fonts,
            scheme.get("Colors").unwrap_or(&settings),
            canvas.clone(),
        )?;
        let layout = keyvalues::parse_resource(
            &vfs.read("scripts/HudLayout.res")?
                .context("HudLayout.res missing")?,
        )?;
        let layout = layout.first().context("HUD layout root missing")?;
        let weapon_layout = layout
            .get("HudWeaponSelection")
            .context("HudWeaponSelection layout missing")?
            .clone();
        let icon_font = vfs
            .read("resource/HALFLIFE2.ttf")?
            .context("HalfLife2 HUD font missing")?;
        let face = FontFace::load(&icon_font, canvas.clone())?;
        let windows = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/Windows"));
        let bold = std::fs::read(windows.join("Fonts/verdanab.ttf"))
            .ok()
            .or(vfs.read("resource/linux_fonts/DejaVuSans-Bold.ttf")?)
            .context("Verdana or bundled HUD fallback font missing")?;
        let bold = FontFace::load(&bold, canvas.clone())?;
        let animation_source = keyvalues::decode_text(
            &vfs.read("scripts/HudAnimations.txt")?
                .context("HudAnimations.txt missing")?,
        )?;
        let animation_rules = animations(&animation_source)?;
        let animation_events = event_rules(&animation_source)?;
        let numeric_effects = MutexCell::new(NumericHudEffects::new(&settings));
        let mut ammo_icons = BTreeMap::new();
        let mut secondary_ammo_icons = BTreeMap::new();
        let mut weapon_crosshairs = BTreeMap::<String, String>::new();
        for class in [
            "weapon_crowbar",
            "weapon_pistol",
            "weapon_357",
            "weapon_smg1",
            "weapon_ar2",
            "weapon_shotgun",
            "weapon_crossbow",
            "weapon_rpg",
            "weapon_frag",
        ] {
            if let Some(bytes) = vfs.read(&format!("scripts/{class}.txt"))? {
                if let Some(weapon) = keyvalues::parse_resource(&bytes)?.first() {
                    if let Some(textures) = weapon.get("TextureData") {
                        for (key, icons) in [
                            ("ammo", &mut ammo_icons),
                            ("ammo2", &mut secondary_ammo_icons),
                            ("crosshair", &mut weapon_crosshairs),
                        ] {
                            if let Some(icon) = textures
                                .get(key)
                                .and_then(|e| e.get("character"))
                                .and_then(Entry::text)
                            {
                                icons.insert(class.into(), icon.into());
                            }
                        }
                    }
                }
            }
        }
        let hud_textures = keyvalues::parse_resource(
            &vfs.read("scripts/hud_textures.txt")?
                .context("HUD textures missing")?,
        )?;
        let default_icon = hud_textures
            .first()
            .and_then(|root| root.get("TextureData"))
            .and_then(|textures| textures.get("crosshair_default"))
            .context("Default crosshair resource missing")?;
        let default_file = default_icon
            .get("file")
            .and_then(Entry::text)
            .context("Default crosshair material missing")?;
        let default_crosshair = texture(vfs, default_file)?;
        let default_base = vfs
            .base_texture(default_file)?
            .context("Default crosshair base texture missing")?;
        let default_vtf = vfs
            .read(&format!(
                "materials/{}.vtf",
                default_base
                    .trim_start_matches("materials/")
                    .trim_end_matches(".vtf")
            ))?
            .context("Default crosshair VTF missing")?;
        default_crosshair.set_filter(sprite_filter(&default_vtf)?);
        let default_crosshair_additive = vfs
            .material_value(default_file, "$additive")?
            .is_some_and(|value| value.parse::<i32>().is_ok_and(|value| value != 0));
        let mut crosshair_color = color(
            scheme.get("Colors").unwrap_or(&settings),
            "Normal",
            Color::from_rgba(255, 208, 64, 255),
        );
        // C_BaseCombatWeapon::DrawCrosshair explicitly makes the normal color opaque.
        crosshair_color.a = 1.;
        let crosshairs = HudFont::read(fonts, "Crosshairs", face.clone(), 40.);
        #[cfg(windows)]
        let native_crosshairs = {
            let requests = crosshairs
                .ranges
                .iter()
                .filter(|range| range.monochrome_crosshair())
                .flat_map(|range| {
                    weapon_crosshairs.values().flat_map(move |characters| {
                        characters.chars().map(move |character| {
                            (character, range.tall.round().clamp(1., 1024.) as u16)
                        })
                    })
                })
                .collect();
            crate::native_font::load(&icon_font, &requests)?
        };
        let mut corners = Vec::new();
        for number in 1..=4 {
            corners.push(texture(vfs, &format!("vgui/hud/8x800corner{number}"))?);
        }
        Ok(Self {
            corners,
            canvas,
            icons: HudFont::read(fonts, "WeaponIcons", face.clone(), 64.),
            selected_icons: HudFont::read(fonts, "WeaponIconsSelected", face.clone(), 64.),
            crosshairs,
            #[cfg(windows)]
            native_crosshairs,
            numbers: HudFont::read(fonts, "HudNumbers", face.clone(), 32.),
            number_glow: HudFont::read(fonts, "HudNumbersGlow", face.clone(), 32.),
            small_numbers: HudFont::read(fonts, "HudNumbersSmall", face.clone(), 16.),
            ammo_icon_font: HudFont::read(fonts, "WeaponIconsSmall", face, 16.),
            selection_numbers: HudFont::read(fonts, "HudSelectionNumbers", bold.clone(), 11.),
            selection_text: HudFont::read(fonts, "HudSelectionText", bold.clone(), 10.),
            labels: HudFont::read(fonts, "Default", bold, 12.),
            health_panel: PanelLayout::read(
                layout
                    .get("HudHealth")
                    .context("HudHealth layout missing")?,
            ),
            suit_panel: PanelLayout::read(layout.get("HudSuit").context("HudSuit layout missing")?),
            ammo_panel: PanelLayout::read(layout.get("HudAmmo").context("HudAmmo layout missing")?),
            secondary_ammo_panel: PanelLayout::read(
                layout
                    .get("HudAmmoSecondary")
                    .context("Secondary ammo HUD layout missing")?,
            ),
            layout: weapon_layout,
            settings,
            animation_rules,
            animation_events,
            numeric_effects,
            ammo_icons,
            secondary_ammo_icons,
            weapon_crosshairs,
            default_crosshair,
            default_crosshair_rect: Rect::new(
                number(default_icon, "x", 0.),
                number(default_icon, "y", 48.),
                number(default_icon, "width", 24.),
                number(default_icon, "height", 24.),
            ),
            default_crosshair_additive,
            crosshair_color,
            quick_info,
            damage_layout: {
                let entry = layout.get("HudDamageIndicator");
                // CPanelAnimationVarAliasType defaults when the layout omits a value.
                let value = |key, fallback| entry.map_or(fallback, |e| number(e, key, fallback));
                [
                    value("dmg_xpos", 10.),
                    value("dmg_ypos", 80.),
                    value("dmg_wide", 30.),
                    value("dmg_tall1", 300.),
                    value("dmg_tall2", 240.),
                ]
            },
            fades: MutexCell::new(Default::default()),
        })
    }

    /// A ScreenFade message (DamageEffect or env_fade) on the game clock.
    pub fn screen_fade(&self, fade: hl2_simulation::player_damage::ScreenFade, time: f64) {
        self.fades.borrow_mut().add(fade, time);
    }
    pub fn clear_fades(&self) {
        self.fades.borrow_mut().clear();
    }
    /// ViewDrawFade over the 3D view, before the HUD. The modulate material's exact
    /// use of the fade alpha is engine code; here the frame is multiplied by the color
    /// blended toward white by (1 - alpha), which is inferred, not verified.
    pub fn draw_fade(&self, time: f64) {
        let Some(([r, g, b, a], modulate)) = self.fades.borrow_mut().params(time) else {
            return;
        };
        if a == 0 {
            return;
        }
        if modulate {
            let t = f32::from(a) / 255.;
            let mix = |c: u8| 1. - t + t * f32::from(c) / 255.;
            self.canvas.modulate(Color::new(mix(r), mix(g), mix(b), 1.));
        } else {
            self.canvas.additive(false);
            self.canvas.rectangle(
                0.,
                0.,
                self.canvas.width(),
                self.canvas.height(),
                Color::from_rgba(r, g, b, a),
            );
        }
    }
    /// CHudDamageIndicator::MsgFunc_Damage. `view_yaw` is the Source view yaw in degrees.
    #[allow(clippy::too_many_arguments)]
    pub fn damage_message(
        &self,
        message: hl2_simulation::player_damage::DamageMessage,
        view_origin: glam::Vec3,
        view_yaw: f32,
        suit: bool,
        health: f32,
        time: f64,
    ) {
        use hl2_simulation::player_damage::{
            DMG_ACID, DMG_BURN, DMG_DROWN, DMG_POISON, DMG_RADIATION,
        };
        let start = |event: &str| {
            self.numeric_effects.borrow_mut().start(
                event,
                time,
                &self.animation_rules,
                &self.animation_events,
                &self.settings,
            )
        };
        if health <= 0. {
            start("HudPlayerDeath");
            return;
        }
        if message.from == glam::Vec3::ZERO && message.bits & DMG_DROWN == 0 {
            return;
        }
        if message.taken == 0 && message.armor == 0 {
            return;
        }
        let high = message.taken > 25 || !suit;
        let angle = damage_angle((message.from - view_origin).normalize_or_zero(), view_yaw);
        // g_DamageAnimations, first match: (event, bits, min angle, max angle, high only).
        const TABLE: [(&str, u32, f32, f32, bool); 12] = [
            ("HudTakeDamageDrown", DMG_DROWN, 0., 0., false),
            ("HudTakeDamagePoison", DMG_POISON, 0., 0., false),
            ("HudTakeDamageBurn", DMG_BURN, 0., 0., false),
            ("HudTakeDamageRadiation", DMG_RADIATION, 0., 0., false),
            ("HudTakeDamageRadiation", DMG_ACID, 0., 0., false),
            ("HudTakeDamageHighLeft", 0, 45., 135., true),
            ("HudTakeDamageHighRight", 0, 225., 315., true),
            ("HudTakeDamageHigh", 0, 0., 0., true),
            ("HudTakeDamageLeft", 0, 45., 135., false),
            ("HudTakeDamageRight", 0, 225., 315., false),
            ("HudTakeDamageBehind", 0, 135., 225., false),
            ("HudTakeDamageFront", 0, 0., 0., false),
        ];
        let chosen = TABLE.iter().find(|(_, bits, min, max, high_only)| {
            !(*bits != 0 && message.bits & bits == 0
                || *min != 0. && angle < *min
                || *max != 0. && angle > *max
                || *high_only && !high)
        });
        if let Some((event, ..)) = chosen {
            start(event);
        }
    }
    /// CHudDamageIndicator::Paint: the fullscreen flash, then both sides, additive
    /// (vgui/white_additive) with per-vertex alpha. Drawn while any color has alpha.
    fn draw_damage_indicator(&self, panel: PanelEffects) {
        use NumericProperty::*;
        let color = |p: NumericProperty| panel.values[p as usize].map(|v| v.clamp(0., 255.));
        let [left, right, high_left, high_right, full] = [
            DmgColorLeft,
            DmgColorRight,
            DmgHighColorLeft,
            DmgHighColorRight,
            DmgFullscreenColor,
        ]
        .map(color);
        let (width, height) = (self.canvas.width(), self.canvas.height());
        self.canvas.additive(true);
        let rgba = |c: [f32; 4], alpha: f32| {
            Color::from_rgba(c[0] as u8, c[1] as u8, c[2] as u8, (c[3] * alpha) as u8)
        };
        if full[3] > 0. {
            self.canvas.rectangle(0., 0., width, height, rgba(full, 1.));
        }
        // Proportional values are truncated to VGUI integers.
        let scale = height / 480.;
        let [xpos, ypos, wide, tall1, tall2] = self.damage_layout.map(|v| (v * scale).trunc());
        let high = high_right[3] > right[3] || high_left[3] > left[3];
        let inset = ((tall1 - tall2) / 2.).trunc();
        let (x1, x2, y, alpha) = if high {
            (
                0.,
                (width * 0.5).trunc(),
                [0., 0., height, height],
                [1., 0., 0., 1.],
            )
        } else {
            (
                xpos,
                xpos + wide,
                [ypos, ypos + inset, ypos + tall1 - inset, ypos + tall1],
                [0., 1., 1., 0.],
            )
        };
        for side in [0, 1] {
            let c = match (side, high) {
                (0, true) => high_left,
                (0, false) => left,
                (_, true) => high_right,
                _ => right,
            };
            if c[3] <= 0. {
                continue;
            }
            let v = |x: f32, i: usize| (vec2(x, y[i]), rgba(c, alpha[i]));
            // Left-top, left-bottom, right-bottom, right-top.
            self.canvas.polygon(if side == 0 {
                [v(x1, 0), v(x1, 3), v(x2, 2), v(x2, 1)]
            } else {
                let (outer, inner) = (width - x1, width - x2);
                [v(inner, 1), v(inner, 2), v(outer, 3), v(outer, 0)]
            });
        }
        self.canvas.additive(false);
    }

    fn rounded_box(&self, rect: Rect, color: Color) {
        self.canvas.additive(false);
        // Retail Panel::GetCornerTextureSize uses max(scaled 8 / 2, 8).
        let corner = ((8. * self.canvas.height() / 480.).round() / 2.)
            .floor()
            .max(8.)
            .min(rect.w / 2.)
            .min(rect.h / 2.);
        self.canvas
            .rectangle(rect.x + corner, rect.y, rect.w - corner * 2., corner, color);
        self.canvas
            .rectangle(rect.x, rect.y + corner, rect.w, rect.h - corner * 2., color);
        self.canvas.rectangle(
            rect.x + corner,
            rect.y + rect.h - corner,
            rect.w - corner * 2.,
            corner,
            color,
        );
        for (index, x, y) in [
            (0, rect.x, rect.y),
            (1, rect.x + rect.w - corner, rect.y),
            (3, rect.x, rect.y + rect.h - corner),
            (2, rect.x + rect.w - corner, rect.y + rect.h - corner),
        ] {
            draw_texture_ex(
                &self.canvas,
                &self.corners[index],
                x,
                y,
                color,
                DrawTextureParams {
                    dest_size: Some(vec2(corner, corner)),
                    ..Default::default()
                },
            );
        }
    }

    fn selection_alpha(&self, selection: &Selection, time: f64, property: &str) -> f32 {
        let open = animation(
            &self.animation_rules,
            "OpenWeaponSelectionMenu",
            "HudWeaponSelection",
            property,
        );
        let target = open
            .and_then(|a| a.target.parse::<f32>().ok())
            .unwrap_or(if property == "Alpha" { 128. } else { 255. });
        let opening = open.map_or(1., |a| {
            if a.duration > 0. {
                ((time - selection.opened_at - a.delay) / a.duration).clamp(0., 1.) as f32
            } else {
                1.
            }
        });
        let fade = animation(
            &self.animation_rules,
            "FadeOutWeaponSelectionMenu",
            "HudWeaponSelection",
            property,
        );
        let fading = fade.map_or(1., |a| {
            if a.duration > 0. {
                (1. - (time - selection.changed_at - selection::SELECTION_TIMEOUT - a.delay)
                    / a.duration)
                    .clamp(0., 1.) as f32
            } else {
                1.
            }
        });
        target / 255. * opening * fading
    }

    pub fn draw_selection(
        &self,
        selection: &Selection,
        inv: &Inventory,
        weapons: &BTreeMap<String, Weapon>,
        time: f64,
    ) {
        let Some(pending) = selection.pending.as_deref() else {
            return;
        };
        let Some(selected_weapon) = weapons.get(pending) else {
            return;
        };
        if !inv.suit || inv.health <= 0. || selection.fast_switch || time > selection.until {
            return;
        }
        let scale = self.canvas.height() / 480.;
        let small = number(&self.layout, "SmallBoxSize", 32.) * scale;
        let wide = number(&self.layout, "LargeBoxWide", 112.) * scale;
        let tall = number(&self.layout, "LargeBoxTall", 84.) * scale;
        let gap = number(&self.layout, "BoxGap", 8.) * scale;
        let y0 = number(&self.layout, "ypos", 16.) * scale;
        let mut x = (self.canvas.width() - (WEAPON_SLOTS - 1) as f32 * (small + gap) - wide) * 0.5;
        let normal_alpha = self.selection_alpha(selection, time, "Alpha");
        let selected_alpha = self.selection_alpha(selection, time, "SelectionAlpha");
        let number_color = color(&self.settings, "SelectionNumberFg", YELLOW);
        let normal = color(
            &self.settings,
            "SelectionBoxBg",
            Color::from_rgba(0, 0, 0, 80),
        );
        let empty = color(&self.settings, "SelectionEmptyBoxBg", normal);
        let selected = color(&self.settings, "SelectionSelectedBoxBg", normal);
        let names = selection::ordered(inv, weapons);
        for slot in 0..WEAPON_SLOTS {
            let slot_names = names
                .iter()
                .copied()
                .filter(|n| weapons[*n].slot == slot)
                .collect::<Vec<_>>();
            if slot == selected_weapon.slot {
                let last = slot_names
                    .iter()
                    .map(|n| weapons[*n].slot_position)
                    .max()
                    .unwrap_or(0);
                let mut y = y0;
                let mut numbered = false;
                for position in 0..=last {
                    let name = slot_names
                        .iter()
                        .copied()
                        .find(|n| weapons[*n].slot_position == position);
                    if name.is_none() && !selection.show_empty_positions {
                        continue;
                    }
                    let is_selected = name == Some(pending);
                    let opacity = if is_selected {
                        selected_alpha
                    } else {
                        normal_alpha * selected_alpha
                    };
                    let background = if name.is_none() {
                        empty
                    } else if is_selected {
                        selected
                    } else {
                        normal
                    };
                    self.rounded_box(Rect::new(x, y, wide, tall), alpha(background, opacity));
                    self.canvas.additive(true);
                    if !numbered {
                        self.selection_numbers.draw(
                            &(slot + 1).to_string(),
                            x + number(&self.layout, "SelectionNumberXPos", 4.) * scale,
                            y + number(&self.layout, "SelectionNumberYPos", 4.) * scale,
                            alpha(number_color, opacity),
                        );
                    }
                    if let Some(name) = name {
                        let weapon = &weapons[name];
                        let mut icon_color = color(
                            &self.settings,
                            "FgColor",
                            Color::from_rgba(255, 220, 0, 100),
                        );
                        if !selection::can_be_selected(inv, weapons, name) {
                            icon_color.r = 1.;
                            icon_color.g = 0.;
                            icon_color.b = 0.;
                        }
                        icon_color.a = if is_selected {
                            selected_alpha
                        } else {
                            icon_color.a * opacity
                        };
                        let icon_x = x + (wide - self.icons.width(&weapon.icon)) * 0.5;
                        let icon_y = y + (tall - self.icons.size()) * 0.5;
                        if is_selected {
                            self.selected_icons
                                .draw(&weapon.icon, icon_x, icon_y, icon_color);
                            let text_color =
                                alpha(color(&self.settings, "BrightFg", YELLOW), selected_alpha);
                            for (line, text) in weapon
                                .name
                                .split("\\n")
                                .flat_map(|s| s.split('\n'))
                                .enumerate()
                            {
                                self.selection_text.draw(
                                    text,
                                    x + (wide - self.selection_text.width(text)) * 0.5,
                                    y + number(&self.layout, "TextYPos", 64.) * scale
                                        + line as f32 * self.selection_text.size() * 1.1,
                                    text_color,
                                );
                            }
                        }
                        self.icons.draw(&weapon.icon, icon_x, icon_y, icon_color);
                    }
                    numbered = true;
                    y += tall + gap;
                }
                x += wide;
            } else {
                self.rounded_box(
                    Rect::new(x, y0, small, small),
                    alpha(
                        if slot_names.is_empty() { empty } else { normal },
                        normal_alpha,
                    ),
                );
                if !slot_names.is_empty() {
                    self.canvas.additive(true);
                    self.selection_numbers.draw(
                        &(slot + 1).to_string(),
                        x + number(&self.layout, "SelectionNumberXPos", 4.) * scale,
                        y0 + number(&self.layout, "SelectionNumberYPos", 4.) * scale,
                        alpha(number_color, normal_alpha),
                    );
                }
                x += small;
            }
            x += gap;
        }
        self.canvas.additive(false);
    }

    fn numeric_panel(
        &self,
        panel: &PanelLayout,
        label: &str,
        value: i32,
        secondary: Option<i32>,
        effects: PanelEffects,
    ) {
        if effects.opacity() <= 0. {
            return;
        }
        let background = effects.color(NumericProperty::Background);
        let foreground = effects.color(NumericProperty::Foreground);
        let scale = self.canvas.height() / 480.;
        let rect = effects
            .rect()
            .unwrap_or_else(|| panel.rect(scale, self.canvas.width()));
        self.rounded_box(rect, background);
        self.canvas.additive(true);
        self.labels.draw(
            label,
            rect.x + panel.text.x * scale,
            rect.y + panel.text.y * scale,
            foreground,
        );
        self.numbers.draw(
            &value.max(0).to_string(),
            rect.x + panel.digit.x * scale,
            rect.y + panel.digit.y * scale,
            foreground,
        );
        // CHudNumericDisplay repeats the glow font; Blur is an overbright pass count.
        for opacity in effects.glow() {
            self.number_glow.draw(
                &value.max(0).to_string(),
                rect.x + panel.digit.x * scale,
                rect.y + panel.digit.y * scale,
                alpha(foreground, opacity),
            );
        }
        if let Some(value) = secondary {
            self.small_numbers.draw(
                &value.max(0).to_string(),
                rect.x + panel.secondary.x * scale,
                rect.y + panel.secondary.y * scale,
                foreground,
            );
        }
        self.canvas.additive(false);
    }

    pub fn draw_status(
        &self,
        inv: &Inventory,
        weapons: &BTreeMap<String, Weapon>,
        _selection: &Selection,
        time: f64,
    ) {
        let mut effects = self.numeric_effects.borrow_mut();
        effects.configure_layouts(
            [
                &self.health_panel,
                &self.suit_panel,
                &self.ammo_panel,
                &self.secondary_ammo_panel,
            ],
            vec2(self.canvas.width(), self.canvas.height()),
        );
        effects.observe(
            inv,
            weapons,
            time,
            &self.animation_rules,
            &self.animation_events,
            &self.settings,
        );
        self.draw_damage_indicator(effects.panels[NumericPanel::DamageIndicator as usize]);
        self.quick_info.draw(inv, weapons, time);
        if !inv.suit || inv.health <= 0. {
            return;
        }
        self.numeric_panel(
            &self.health_panel,
            "HEALTH",
            inv.health as i32,
            None,
            effects.panels[NumericPanel::Health as usize],
        );
        self.numeric_panel(
            &self.suit_panel,
            "SUIT",
            inv.armor as i32,
            None,
            effects.panels[NumericPanel::Suit as usize],
        );
        // HIDEHUD_WEAPONSELECTION is a player hide-bit. Opening the bucket selector
        // does not set it; the active weapon's ammunition remains visible.
        let Some(weapon) = weapons.get(&inv.active) else {
            return;
        };
        if !weapon.ammo_type.is_empty() && !weapon.ammo_type.eq_ignore_ascii_case("none") {
            let clip = inv.owned.get(&inv.active).copied().unwrap_or(-1);
            let reserve = inv.reserve_for(&inv.active, weapons);
            self.numeric_panel(
                &self.ammo_panel,
                "AMMO",
                if clip < 0 { reserve } else { clip },
                (clip >= 0).then_some(reserve),
                effects.panels[NumericPanel::Ammo as usize],
            );
            self.ammo_icon(
                &self.ammo_panel,
                "AMMO",
                self.ammo_icons.get(&inv.active),
                effects.panels[NumericPanel::Ammo as usize],
            );
        }
        let secondary_effects = effects.panels[NumericPanel::AmmoSecondary as usize];
        self.numeric_panel(
            &self.secondary_ammo_panel,
            "ALT",
            effects.secondary_ammo,
            None,
            secondary_effects,
        );
        self.ammo_icon(
            &self.secondary_ammo_panel,
            "ALT",
            self.secondary_ammo_icons.get(&inv.active),
            secondary_effects,
        );
    }

    fn ammo_icon(
        &self,
        panel: &PanelLayout,
        label: &str,
        icon: Option<&String>,
        effects: PanelEffects,
    ) {
        if let Some(icon) = icon.filter(|_| effects.opacity() > 0.) {
            let scale = self.canvas.height() / 480.;
            let rect = effects
                .rect()
                .unwrap_or_else(|| panel.rect(scale, self.canvas.width()));
            let x = rect.x
                + panel.text.x * scale
                + (self.labels.width(label) - self.ammo_icon_font.width(icon)) * 0.5;
            let y = rect.y + panel.text.y * scale
                - self.labels.size()
                - self.ammo_icon_font.size() * 0.5;
            self.canvas.additive(true);
            self.ammo_icon_font
                .draw(icon, x, y, effects.color(NumericProperty::Foreground));
            self.canvas.additive(false);
        }
    }

    pub fn draw_crosshair(&self, inv: &Inventory) {
        if inv.health <= 0. {
            return;
        }
        let Some(character) = self.weapon_crosshairs.get(&inv.active) else {
            // CHudCrosshair::ResetCrosshair uses the installed default sprite and opaque white.
            let (destination, source) = default_crosshair_rects(
                vec2(self.canvas.width(), self.canvas.height()),
                self.default_crosshair_rect,
                self.default_crosshair.size(),
            );
            if self.default_crosshair_additive {
                self.canvas.additive(true);
            }
            draw_texture_ex(
                &self.canvas,
                &self.default_crosshair,
                destination.x,
                destination.y,
                WHITE,
                DrawTextureParams {
                    source: Some(source),
                    dest_size: Some(destination.size()),
                },
            );
            self.canvas.additive(false);
            return;
        };
        self.canvas.additive(true);
        #[cfg(windows)]
        if let Some(glyph) = character
            .chars()
            .next()
            .filter(|_| character.chars().count() == 1)
            .and_then(|c| {
                let range = self.crosshairs.ranges.iter().find(|range| {
                    self.canvas.height() >= range.minimum && self.canvas.height() <= range.maximum
                })?;
                range.monochrome_crosshair().then_some(())?;
                self.native_crosshairs.get(&(c, range.tall.round() as u16))
            })
        {
            if let Some(texture) = &glyph.texture {
                // Integer icon dimensions and GDI's actual cell height, not the requested
                // height or fractional fontdue EM metrics. No rasterization occurs here.
                let center = vec2(
                    (self.canvas.width() * 0.5 + 0.5).trunc(),
                    (self.canvas.height() * 0.5 + 0.5).trunc(),
                );
                let origin = center - vec2((glyph.advance / 2) as f32, (glyph.height / 2) as f32)
                    + glyph.origin;
                draw_texture(
                    &self.canvas,
                    texture,
                    origin.x,
                    origin.y,
                    self.crosshair_color,
                );
            }
            self.canvas.additive(false);
            return;
        }
        self.crosshairs.draw(
            character,
            (self.canvas.width() - self.crosshairs.width(character)) * 0.5,
            (self.canvas.height() - self.crosshairs.size()) * 0.5,
            self.crosshair_color,
        );
        self.canvas.additive(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an owned HL2 installation in HL2_ROOT"]
    fn owned_monochrome_crosshair_matches_native_pistol_and_smg_pixels() {
        let root = std::env::var_os("HL2_ROOT").expect("set HL2_ROOT to the owned installation");
        let vfs = Vfs::mount(std::path::Path::new(&root)).unwrap();
        let canvas = Canvas::default();
        let hud = WeaponHud::load(&vfs, canvas.clone()).unwrap();
        let mut inventory = Inventory::default();
        for weapon in ["weapon_pistol", "weapon_smg1"] {
            inventory.active = weapon.into();
            for viewport in [vec2(1280., 720.), vec2(1920., 1080.), vec2(1281., 721.)] {
                canvas.resize(viewport.x, viewport.y);
                hud.draw_crosshair(&inventory);
                let quads = canvas.drain();
                assert_eq!(quads.len(), 1);
                let quad = &quads[0];
                assert_eq!(quad.color.a, 1.);
                let texture = quad.texture.as_ref().unwrap();
                assert_eq!(texture.filter(), FilterMode::Nearest);
                let center = vec2(
                    (viewport.x * 0.5 + 0.5).trunc(),
                    (viewport.y * 0.5 + 0.5).trunc(),
                );
                let mut dots = Vec::new();
                for (i, pixel) in texture.0.rgba.as_chunks::<4>().0.iter().enumerate() {
                    assert!(pixel[3] == 0 || pixel[3] == 255);
                    if pixel[3] != 0 {
                        let x = i % usize::from(texture.0.width);
                        let y = i / usize::from(texture.0.width);
                        dots.push((
                            (quad.destination.x + x as f32 - center.x) as i32,
                            (quad.destination.y + y as f32 - center.y) as i32,
                        ));
                    }
                }
                assert_eq!(dots, [(-1, -8), (-11, 0), (-1, 0), (9, 0), (-1, 8)]);
            }
        }
    }

    #[test]
    #[ignore = "requires an owned HL2 installation in HL2_ROOT"]
    fn owned_canvas_crosshair_and_pending_selection_preserve_status() {
        let root = std::env::var_os("HL2_ROOT").expect("set HL2_ROOT to the owned installation");
        let vfs = Vfs::mount(std::path::Path::new(&root)).unwrap();
        let canvas = Canvas::default();
        let hud = WeaponHud::load(&vfs, canvas.clone()).unwrap();
        let mut inventory = Inventory::default();
        for viewport in [vec2(1280., 720.), vec2(1920., 1080.)] {
            canvas.resize(viewport.x, viewport.y);
            hud.draw_crosshair(&inventory);
            let quads = canvas.drain();
            assert_eq!(quads.len(), 1);
            assert_eq!(quads[0].color, WHITE);
            assert_eq!(
                quads[0].destination,
                Rect::new(viewport.x / 2. - 12., viewport.y / 2. - 12., 24., 24.)
            );
            assert_eq!(
                quads[0].texture.as_ref().unwrap().filter(),
                FilterMode::Nearest
            );
        }
        inventory.health = 0.;
        hud.draw_crosshair(&inventory);
        assert!(canvas.drain().is_empty());
        inventory.health = 100.;
        inventory.suit = true;
        let weapons = hl2_simulation::gameplay::definitions(&vfs).unwrap();
        inventory.give("weapon_smg1", &weapons, 0.);
        inventory.give("weapon_shotgun", &weapons, 0.);
        inventory.active = "weapon_smg1".into();
        inventory.refill_ammo(&weapons);
        let mut selection = Selection::new();
        hud.draw_status(&inventory, &weapons, &selection, 0.);
        canvas.drain();
        hud.draw_status(&inventory, &weapons, &selection, 1.);
        let before = canvas.drain();
        selection.pending = Some("weapon_shotgun".into());
        selection.until = 2.;
        hud.draw_status(&inventory, &weapons, &selection, 1.);
        let after = canvas.drain();
        let signature = |quads: &[Quad]| {
            quads
                .iter()
                .map(|q| {
                    (
                        q.destination,
                        q.source,
                        q.color,
                        q.additive,
                        q.texture.as_ref().map(Texture2D::id),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert!(
            before.len() > 12,
            "owned health, suit and ammunition must draw"
        );
        assert_eq!(
            signature(&before),
            signature(&after),
            "pending selection must not hide or replace the active ammunition"
        );
    }

    fn settings() -> Entry {
        keyvalues::parse_resource(
            br#"Colors { FgColor "255 220 0 100" BrightFg "255 220 0 255" BgColor "0 0 0 76" }"#,
        )
        .unwrap()
        .remove(0)
    }

    #[test]
    fn default_crosshair_keeps_native_size_at_720p_and_1080p() {
        let atlas = Rect::new(0., 48., 24., 24.);
        let texture = vec2(128., 128.);
        assert_eq!(
            default_crosshair_rects(vec2(1280., 720.), atlas, texture).0,
            Rect::new(628., 348., 24., 24.)
        );
        assert_eq!(
            default_crosshair_rects(vec2(1920., 1080.), atlas, texture).0,
            Rect::new(948., 528., 24., 24.)
        );
        // A wider viewport alone does not make a larger default crosshair.
        assert_eq!(
            default_crosshair_rects(vec2(7680., 1080.), atlas, texture)
                .0
                .size(),
            vec2(24., 24.)
        );
        assert_eq!(
            default_crosshair_rects(vec2(1280., 1599.), atlas, texture)
                .0
                .size(),
            vec2(24., 24.)
        );
        assert_eq!(
            default_crosshair_rects(vec2(1280., 1600.), atlas, texture)
                .0
                .size(),
            vec2(48., 48.)
        );
    }

    #[test]
    fn default_crosshair_uses_atlas_texel_centers_and_integer_centering() {
        let (destination, source) = default_crosshair_rects(
            vec2(1281., 721.),
            Rect::new(0., 48., 24., 24.),
            vec2(128., 128.),
        );
        assert_eq!(destination, Rect::new(629., 349., 24., 24.));
        assert_eq!(
            source,
            Rect::new(0.99609375, 48.621094, 22.820313, 22.820313)
        );
        let (_, non_square_source) = default_crosshair_rects(
            vec2(1921., 1081.),
            Rect::new(0., 48., 24., 24.),
            vec2(256., 128.),
        );
        assert_eq!(
            non_square_source,
            Rect::new(0.9980469, 48.621094, 22.910156, 22.820313)
        );
    }

    #[test]
    fn default_crosshair_point_samples_match_native_five_dot_offsets() {
        // A sparse test sprite records the independently measured owned default texels.
        // The expected output offsets came from accepted retail720/1080screenshots.
        let texels = [(11, 49), (1, 59), (11, 59), (21, 59), (11, 69)];
        let expected = [(-1, -12), (-12, -1), (-1, -1), (9, -1), (-1, 9)];
        for viewport in [vec2(1280., 720.), vec2(1920., 1080.), vec2(1281., 721.)] {
            let (destination, source) =
                default_crosshair_rects(viewport, Rect::new(0., 48., 24., 24.), vec2(128., 128.));
            let mut dots = Vec::new();
            for y in 0..destination.h as i32 {
                for x in 0..destination.w as i32 {
                    let texel = (
                        (source.x + (x as f32 + 0.5) * source.w / destination.w).floor() as i32,
                        (source.y + (y as f32 + 0.5) * source.h / destination.h).floor() as i32,
                    );
                    if texels.contains(&texel) {
                        dots.push((x - 12, y - 12));
                    }
                }
            }
            assert_eq!(dots, expected, "viewport{viewport:?}");
        }
    }

    #[test]
    fn hud_sprite_honors_owned_point_sampling_flag() {
        let mut header = [0u8; 24];
        header[20..24].copy_from_slice(&0x2341u32.to_le_bytes());
        assert_eq!(sprite_filter(&header).unwrap(), FilterMode::Nearest);
        header[20..24].copy_from_slice(&0x2340u32.to_le_bytes());
        assert_eq!(sprite_filter(&header).unwrap(), FilterMode::Linear);
        assert!(sprite_filter(&header[..23]).is_err());
    }

    #[test]
    fn source_interpolators_keep_square_root_deceleration() {
        assert_eq!(Interpolation::Linear.sample(0.25), 0.25);
        assert_eq!(Interpolation::Accel.sample(0.5), 0.25);
        assert_eq!(Interpolation::Deaccel.sample(0.25), 0.5);
        assert_eq!(Interpolation::Spline.sample(0.5), 0.5);
    }

    #[test]
    fn ammo_panel_slides_through_intermediate_positions_and_reverses_from_its_current_position() {
        let resource =
            keyvalues::parse_resource(br#"HudAmmo { xpos r150 ypos 432 wide 152 tall 36 }"#)
                .unwrap();
        let layout = PanelLayout::read(&resource[0]);
        let source = "event WeaponUsesClips\n{\nAnimate HudAmmo Position \"r150 432\" Deaccel 0 0.4\nAnimate HudAmmo Size \"132 36\" Deaccel 0 0.4\n}\nevent WeaponUsesSecondaryAmmo\n{\nStopAnimation HudAmmo Position 0\nStopAnimation HudAmmo Size 0\nAnimate HudAmmo Position \"r222 432\" Deaccel 0 0.5\nAnimate HudAmmo Size \"132 36\" Deaccel 0 0.4\n}";
        let rules = animations(source).unwrap();
        let events = event_rules(source).unwrap();
        let settings = settings();
        let mut effects = NumericHudEffects::new(&settings);
        effects.configure_layouts([&layout; 4], vec2(1280., 720.));
        effects.start("WeaponUsesClips", 0., &rules, &events, &settings);
        effects.start("WeaponUsesSecondaryAmmo", 0., &rules, &events, &settings);
        assert!(!effects
            .animations
            .iter()
            .any(|track| track.event == "WeaponUsesClips"));
        let panel = NumericPanel::Ammo as usize;
        effects.advance(0.);
        assert_eq!(
            effects.panels[panel].rect().unwrap(),
            Rect::new(1055., 648., 228., 54.)
        );
        effects.advance(0.1);
        assert_eq!(effects.panels[panel].rect().unwrap().w, 213.);
        effects.advance(0.125);
        assert_eq!(effects.panels[panel].rect().unwrap().x, 1001.);
        effects.advance(0.5);
        assert_eq!(
            effects.panels[panel].rect().unwrap(),
            Rect::new(947., 648., 198., 54.)
        );
        effects.start("WeaponUsesClips", 0.5, &rules, &events, &settings);
        effects.advance(0.5);
        assert_eq!(effects.panels[panel].rect().unwrap().x, 947.);
        effects.advance(0.6);
        assert_eq!(effects.panels[panel].rect().unwrap().x, 1001.);
        // Interrupt the return slide with a secondary weapon: start at1001, without snapping.
        effects.start("WeaponUsesSecondaryAmmo", 0.6, &rules, &events, &settings);
        effects.advance(0.6);
        assert_eq!(effects.panels[panel].rect().unwrap().x, 1001.);
        effects.advance(0.725);
        assert_eq!(effects.panels[panel].rect().unwrap().x, 974.);
        effects.advance(1.1);
        assert_eq!(effects.panels[panel].rect().unwrap().x, 947.);
    }

    #[test]
    fn geometry_targets_use_the_current_viewport_and_reject_invalid_coordinates() {
        let settings = settings();
        assert_eq!(
            NumericProperty::Position.target("r222 432", &settings, vec2(1920., 1080.)),
            Some([1420.5, 972., 0., 0.])
        );
        assert_eq!(
            NumericProperty::Size.target("132 36", &settings, vec2(1920., 1080.)),
            Some([297., 81., 0., 0.])
        );
        assert!(NumericProperty::Position
            .target("rNaN 432", &settings, vec2(1280., 720.))
            .is_none());
        assert!(NumericProperty::Size
            .target("-1 36", &settings, vec2(1280., 720.))
            .is_none());
        let resource =
            keyvalues::parse_resource(br#"HudAmmo { xpos r150 ypos 432 wide 132 tall 36 }"#)
                .unwrap();
        let layout = PanelLayout::read(&resource[0]);
        let mut effects = NumericHudEffects::new(&settings);
        effects.configure_layouts([&layout; 4], vec2(1280., 720.));
        effects.active = "weapon_smg1".into();
        effects.configure_layouts([&layout; 4], vec2(1920., 1080.));
        assert_eq!(
            effects.panels[NumericPanel::Ammo as usize].rect().unwrap(),
            Rect::new(1582., 972., 297., 81.)
        );
        assert!(effects.active.is_empty());
    }

    #[test]
    fn zero_armor_fades_and_a_pickup_stops_the_fade_immediately() {
        let source = "event SuitPowerZero\n{\nAnimate HudSuit Alpha 0 Linear 0 .4\n}\nevent SuitPowerIncreasedAbove20\n{\nStopEvent SuitPowerZero 0\nAnimate HudSuit Alpha 255 Linear 0 0\n}";
        let rules = animations(source).unwrap();
        let stops = event_rules(source).unwrap();
        let settings = settings();
        let mut effects = NumericHudEffects::new(&settings);
        let mut inv = Inventory::default();
        effects.observe(&inv, &BTreeMap::new(), 0., &rules, &stops, &settings);
        effects.observe(&inv, &BTreeMap::new(), 0.2, &rules, &stops, &settings);
        assert!((effects.panels[NumericPanel::Suit as usize].opacity() - 0.5).abs() < 0.001);
        effects.observe(&inv, &BTreeMap::new(), 0.4, &rules, &stops, &settings);
        assert_eq!(effects.panels[NumericPanel::Suit as usize].opacity(), 0.);
        inv.armor = 15.;
        effects.observe(&inv, &BTreeMap::new(), 0.5, &rules, &stops, &settings);
        assert_eq!(effects.panels[NumericPanel::Suit as usize].opacity(), 1.);
        inv.armor = 0.;
        effects.observe(&inv, &BTreeMap::new(), 1., &rules, &stops, &settings);
        inv.armor = 15.;
        effects.observe(&inv, &BTreeMap::new(), 1.1, &rules, &stops, &settings);
        effects.observe(&inv, &BTreeMap::new(), 1.4, &rules, &stops, &settings);
        assert_eq!(effects.panels[NumericPanel::Suit as usize].opacity(), 1.);
    }

    #[test]
    fn glow_capture_starts_at_the_delay_and_uses_fractional_additive_passes() {
        let source = "event HealthIncreasedAbove20\n{\nAnimate HudHealth Blur 3 Linear 0 .1\nAnimate HudHealth Blur 0 Deaccel .1 2\n}";
        let rules = animations(source).unwrap();
        let settings = settings();
        let mut effects = NumericHudEffects::new(&settings);
        effects.start("HealthIncreasedAbove20", 0., &rules, &[], &settings);
        effects.advance(0.);
        effects.advance(0.05);
        assert_eq!(
            effects.panels[0].values[NumericProperty::Blur as usize][0],
            1.5
        );
        effects.advance(0.1);
        effects.advance(0.6);
        assert_eq!(
            effects.panels[0].values[NumericProperty::Blur as usize][0],
            1.5
        );
        effects.advance(2.1);
        assert!(effects.panels[0].glow().next().is_none());
        effects.panels[0].values[NumericProperty::Blur as usize][0] = 2.25;
        assert_eq!(
            effects.panels[0].glow().collect::<Vec<_>>(),
            vec![1., 1., 0.25]
        );
    }

    #[test]
    fn shooting_cancels_ammo_pickup_animation_without_spurious_weapon_changes() {
        let source = "event AmmoIncreased\n{\nAnimate HudAmmo Blur 5 Linear 0 0\nAnimate HudAmmo Blur 0 Accel .01 1.5\n}\nevent AmmoDecreased\n{\nStopEvent AmmoIncreased 0\nAnimate HudAmmo Blur 7 Linear 0 0\nAnimate HudAmmo Blur 0 Deaccel .1 1.5\n}\nevent WeaponChanged\n{\nAnimate HudAmmo BgColor \"250 220 0 80\" Linear 0 .1\nAnimate HudAmmo BgColor BgColor Deaccel .1 1\n}";
        let rules = animations(source).unwrap();
        let stops = event_rules(source).unwrap();
        let settings = settings();
        let weapons = BTreeMap::from([
            (
                "weapon_pistol".into(),
                Weapon {
                    ammo_type: "Pistol".into(),
                    magazine: 18,
                    ..Default::default()
                },
            ),
            (
                "weapon_crowbar".into(),
                Weapon {
                    ammo_type: "None".into(),
                    magazine: -1,
                    ..Default::default()
                },
            ),
        ]);
        let mut inv = Inventory::default();
        inv.active.push_str("weapon_pistol");
        inv.owned.insert("weapon_pistol".into(), 18);
        inv.pistol_ammo = 36;
        let mut effects = NumericHudEffects::new(&settings);
        effects.observe(&inv, &weapons, 0., &rules, &stops, &settings);
        effects.observe(&inv, &weapons, 0.1, &rules, &stops, &settings);
        inv.owned.insert("weapon_pistol".into(), 17);
        effects.observe(&inv, &weapons, 0.2, &rules, &stops, &settings);
        assert_eq!(
            effects.panels[2].values[NumericProperty::Blur as usize][0],
            7.
        );
        assert!(!effects
            .animations
            .iter()
            .any(|a| a.event == "AmmoIncreased"));
        effects.advance(3.);
        inv.active = "weapon_crowbar".into();
        effects.observe(&inv, &weapons, 3.1, &rules, &stops, &settings);
        inv.active = "weapon_pistol".into();
        effects.observe(&inv, &weapons, 3.2, &rules, &stops, &settings);
        assert!(!effects
            .animations
            .iter()
            .any(|a| a.event == "WeaponChanged"));
    }

    #[test]
    fn secondary_panel_observes_real_inventory_and_switching_cancels_its_fade() {
        let source = "event WeaponUsesSecondaryAmmo\n{\nStopPanelAnimations HudAmmoSecondary 0\nAnimate HudAmmoSecondary Alpha 255 Linear 0 0.1\n}\nevent WeaponDoesNotUseSecondaryAmmo\n{\nStopPanelAnimations HudAmmoSecondary 0\nAnimate HudAmmoSecondary Alpha 0 Linear 0 0.1\n}\nevent AmmoSecondaryDecreased\n{\nAnimate HudAmmoSecondary Blur 7 Linear 0 0\n}\nevent AmmoSecondaryEmpty\n{\nAnimate HudAmmoSecondary FgColor \"255 0 0 255\" Linear 0 0\n}";
        let rules = animations(source).unwrap();
        let events = event_rules(source).unwrap();
        let settings = settings();
        let weapons = BTreeMap::from([
            (
                "weapon_smg1".into(),
                Weapon {
                    ammo_type: "SMG1".into(),
                    secondary_ammo_type: "SMG1_Grenade".into(),
                    secondary_ammo_max: 3,
                    ..Default::default()
                },
            ),
            (
                "weapon_pistol".into(),
                Weapon {
                    ammo_type: "Pistol".into(),
                    ..Default::default()
                },
            ),
        ]);
        let mut inv = Inventory::default();
        inv.active = "weapon_smg1".into();
        inv.give_ammo("SMG1_Grenade", 3, &weapons);
        let mut effects = NumericHudEffects::new(&settings);
        effects.observe(&inv, &weapons, 0., &rules, &events, &settings);
        effects.observe(&inv, &weapons, 0.1, &rules, &events, &settings);
        let secondary = NumericPanel::AmmoSecondary as usize;
        assert_eq!(effects.secondary_ammo, 3);
        assert_eq!(effects.panels[secondary].opacity(), 1.);
        inv.reserve_ammo.insert("SMG1_Grenade".into(), 2);
        effects.observe(&inv, &weapons, 0.2, &rules, &events, &settings);
        assert_eq!(effects.secondary_ammo, 2);
        assert_eq!(
            effects.panels[secondary].values[NumericProperty::Blur as usize][0],
            7.
        );
        inv.reserve_ammo.insert("SMG1_Grenade".into(), 0);
        effects.observe(&inv, &weapons, 0.3, &rules, &events, &settings);
        assert_eq!(
            effects.panels[secondary].color(NumericProperty::Foreground),
            Color::from_rgba(255, 0, 0, 255)
        );
        inv.active = "weapon_pistol".into();
        effects.observe(&inv, &weapons, 0.4, &rules, &events, &settings);
        effects.observe(&inv, &weapons, 0.45, &rules, &events, &settings);
        assert!((effects.panels[secondary].opacity() - 0.5).abs() < 0.001);
        inv.active = "weapon_smg1".into();
        effects.observe(&inv, &weapons, 0.45, &rules, &events, &settings);
        effects.observe(&inv, &weapons, 0.55, &rules, &events, &settings);
        assert_eq!(effects.panels[secondary].opacity(), 1.);
        assert!(!effects
            .animations
            .iter()
            .any(|a| a.event == "WeaponDoesNotUseSecondaryAmmo"));
    }

    #[test]
    fn damage_indicator_angles_and_owned_style_sequences() {
        // Facing +X (yaw 0): +Y is left, -X behind, -Y right; front wraps to 360.
        let angle = |d: [f32; 3], yaw| damage_angle(glam::Vec3::from_array(d), yaw);
        // Straight ahead is -0 sideways: atan2(-0, -360) + pi = 0, matching no bin.
        assert!(angle([1., 0., 0.], 0.).abs() < 0.01);
        assert!((angle([0., 1., 0.], 0.) - 90.).abs() < 0.01);
        assert!((angle([-1., 0., 0.], 0.) - 180.).abs() < 0.01);
        assert!((angle([0., -1., 0.], 0.) - 270.).abs() < 0.01);
        // The view yaw rotates the bins: facing +Y, damage from +X is on the right.
        assert!((angle([1., 0., 0.], 90.) - 270.).abs() < 0.01);
        // HudTakeDamageLeft drives DmgColorLeft through the indicator panel.
        let source = "event HudTakeDamageLeft\n{\nAnimate HudDamageIndicator DmgColorLeft \"255 88 0 200\" Linear 0.0 0.0\nAnimate HudDamageIndicator DmgColorLeft \"255 0 0 0\" Deaccel 0.3 0.5\n}\n";
        let rules = animations(source).unwrap();
        let events = event_rules(source).unwrap();
        let settings = settings();
        let mut effects = NumericHudEffects::new(&settings);
        effects.start("HudTakeDamageLeft", 1., &rules, &events, &settings);
        effects.advance(1.);
        let panel = effects.panels[NumericPanel::DamageIndicator as usize];
        assert_eq!(
            panel.values[NumericProperty::DmgColorLeft as usize],
            [255., 88., 0., 200.]
        );
        effects.advance(1.8);
        let panel = effects.panels[NumericPanel::DamageIndicator as usize];
        assert_eq!(panel.values[NumericProperty::DmgColorLeft as usize][3], 0.);
        assert_eq!(panel.values[NumericProperty::DmgColorRight as usize][3], 0.);
    }

    #[test]
    fn animation_subset_rejects_nonfinite_timings_and_extra_parameter_interpolators() {
        let rules = animations("event Invalid\nAnimate HudSuit Alpha 0 Linear NaN .4\nAnimate HudSuit Alpha 0 Linear 0 inf\nAnimate HudSuit Alpha 0 Pulse 3 0 .4\nAnimate HudSuit Alpha 0 Linear 0 .4").unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].duration, 0.4);
    }

    #[test]
    fn quickinfo_dims_after_one_second_and_brightens_after_activity() {
        let mut state = QuickInfoState::default();
        state.observe(100, 18, 18, 0.);
        state.observe(100, 18, 18, 1.);
        assert!(!state.dimmed);
        state.observe(100, 18, 18, 1.1);
        state.observe(100, 18, 18, 2.1);
        assert!((state.alpha - 159.5).abs() < 0.001);
        state.observe(100, 18, 18, 3.1);
        assert_eq!(state.alpha, 64.);
        state.observe(100, 17, 18, 3.2);
        state.observe(100, 17, 18, 3.3);
        assert!(!state.dimmed);
        state.observe(100, 17, 18, 3.8);
        assert_eq!(state.alpha, 255.);
    }

    #[test]
    fn quickinfo_warns_at_twenty_five_health_and_quarter_of_clip() {
        let mut state = QuickInfoState::default();
        state.observe(26, 5, 18, 0.);
        assert!(!state.warn_health && !state.warn_ammo);
        state.observe(25, 4, 18, 0.1);
        assert!(state.warn_health && state.warn_ammo);
        assert_eq!(state.health_warning, 255.);
        state.observe(100, 1, 1, 0.2);
        assert!(!state.warn_health && !state.warn_ammo);
        state.observe(100, -1, -1, 0.3);
        assert!(!state.warn_ammo);
    }

    #[test]
    fn installed_health_pulse_repeats_then_recovery_cancels_the_entire_chain() {
        let source = "event HealthLow\n{\nAnimate HudHealth Blur 5 Linear 0 .1\nAnimate HudHealth Blur 3 Deaccel .1 .9\nRunEvent HealthPulse 1\n}\nevent HealthPulse\n{\nAnimate HudHealth Blur 5 Linear 0 .1\nAnimate HudHealth Blur 2 Deaccel .1 .8\nRunEvent HealthLoop .8\n}\nevent HealthLoop\n{\nRunEvent HealthPulse 0\n}\nevent HealthIncreasedAbove20\n{\nStopEvent HealthLoop 0\nStopEvent HealthPulse 0\nStopEvent HealthLow 0\nAnimate HudHealth Blur 3 Linear 0 .1\nAnimate HudHealth Blur 0 Deaccel .1 2\n}";
        let rules = animations(source).unwrap();
        let events = event_rules(source).unwrap();
        let settings = settings();
        let mut effects = NumericHudEffects::new(&settings);
        let mut inv = Inventory::default();
        inv.health = 15.;
        effects.observe(&inv, &BTreeMap::new(), 0., &rules, &events, &settings);
        assert_eq!(effects.posted.len(), 1);
        assert_eq!(effects.posted[0].at, 1.);
        effects.observe(&inv, &BTreeMap::new(), 0.99, &rules, &events, &settings);
        assert!(!effects.animations.iter().any(|a| a.event == "HealthPulse"));
        effects.observe(&inv, &BTreeMap::new(), 1., &rules, &events, &settings);
        assert!(effects
            .animations
            .iter()
            .any(|a| a.event == "HealthPulse" && a.start == 1.));
        effects.observe(&inv, &BTreeMap::new(), 1.8, &rules, &events, &settings);
        assert!(effects
            .animations
            .iter()
            .any(|a| a.event == "HealthPulse" && a.start == 1.8));
        assert_eq!(effects.posted.len(), 1);
        // A stalled frame starts one new pulse now; Source does not replay missed loops.
        effects.observe(&inv, &BTreeMap::new(), 100., &rules, &events, &settings);
        assert!(effects
            .animations
            .iter()
            .any(|a| a.event == "HealthPulse" && a.start == 100.));
        assert_eq!(effects.posted.len(), 1);
        assert!(effects.animations.len() <= 4);
        inv.health = 20.;
        effects.observe(&inv, &BTreeMap::new(), 100.1, &rules, &events, &settings);
        assert!(effects.posted.is_empty());
        assert!(!effects
            .animations
            .iter()
            .any(|a| matches!(a.event.as_str(), "HealthLow" | "HealthPulse" | "HealthLoop")));
        effects.observe(&inv, &BTreeMap::new(), 200., &rules, &events, &settings);
        assert!(effects.posted.is_empty() && effects.animations.is_empty());
    }

    #[test]
    fn zero_delay_event_cycles_are_bounded_by_sources_per_frame_guard() {
        let source = "event A\nRunEvent B 0\nevent B\nRunEvent A 0";
        let rules = animations(source).unwrap();
        let events = event_rules(source).unwrap();
        let settings = settings();
        let mut effects = NumericHudEffects::new(&settings);
        effects.start("A", 0., &rules, &events, &settings);
        effects.posted_events(0., &rules, &events, &settings);
        assert!(effects.posted.is_empty());
    }

    #[test]
    fn player_death_stops_a_running_health_pulse() {
        let source = "event HealthLow\nRunEvent HealthPulse 1\nevent HealthPulse\nRunEvent HealthLoop .8\nevent HealthLoop\nRunEvent HealthPulse 0\nevent HudPlayerDeath\nStopEvent HealthLoop 0\nStopEvent HealthPulse 0";
        let rules = animations(source).unwrap();
        let events = event_rules(source).unwrap();
        let settings = settings();
        let mut effects = NumericHudEffects::new(&settings);
        let mut inv = Inventory::default();
        inv.health = 15.;
        effects.observe(&inv, &BTreeMap::new(), 0., &rules, &events, &settings);
        effects.observe(&inv, &BTreeMap::new(), 1., &rules, &events, &settings);
        assert_eq!(effects.posted[0].target, "HealthLoop");
        // CHudDamageIndicator::MsgFunc_Damage starts HudPlayerDeath for a dead player.
        inv.health = 0.;
        effects.observe(&inv, &BTreeMap::new(), 1.2, &rules, &events, &settings);
        assert!(!effects.posted.is_empty());
        effects.start("HudPlayerDeath", 1.2, &rules, &events, &settings);
        effects.observe(&inv, &BTreeMap::new(), 1.2, &rules, &events, &settings);
        assert!(effects.posted.is_empty());
    }

    #[test]
    fn quickinfo_audio_emits_once_per_warning_latch_and_drain_removes_events() {
        let mut state = QuickInfoState::default();
        state.observe(26, 5, 18, 0.);
        assert!(state.drain_sounds().is_empty());
        state.observe(25, 4, 18, 0.1);
        assert_eq!(
            state.drain_sounds(),
            ["HUDQuickInfo.LowHealth", "HUDQuickInfo.LowAmmo"]
        );
        assert!(state.drain_sounds().is_empty());
        state.observe(24, 3, 18, 0.2);
        state.observe(10, 0, 18, 1.2);
        assert!(state.drain_sounds().is_empty());
        state.observe(26, 5, 18, 1.3);
        state.observe(25, 4, 18, 1.4);
        assert_eq!(
            state.drain_sounds(),
            ["HUDQuickInfo.LowHealth", "HUDQuickInfo.LowAmmo"]
        );
    }

    #[test]
    fn changing_weapons_with_the_same_clip_count_does_not_recheck_quickinfo_warning() {
        let mut state = QuickInfoState::default();
        state.observe(100, 5, 18, 0.);
        state.observe(100, 5, 45, 0.1);
        // Retail compares Clip1 to m_lastAmmo before computing the new maximum's fraction.
        assert!(!state.warn_ammo && state.drain_sounds().is_empty());
        state.observe(100, 4, 45, 0.2);
        assert_eq!(state.drain_sounds(), ["HUDQuickInfo.LowAmmo"]);
    }
    #[test]
    fn desktop_resources_keep_nested_conditionals_and_resolution_font_ranges() {
        let resource = keyvalues::parse_resource(br#"Scheme { Fonts { Numbers { "1" { name HalfLife2 tall 70 [$DECK] tall 64 } } Text { "1" { tall 8 yres "1 599" } "2" { tall 10 yres "600 767" } } } Hud [$DECK] { wide 200 } Hud [!$DECK] { wide 112 } }"#).unwrap();
        let scheme = &resource[0];
        assert_eq!(
            scheme.get("Hud").unwrap().get("wide").and_then(Entry::text),
            Some("112")
        );
        let fonts = scheme.get("Fonts").unwrap();
        assert_eq!(
            fonts.get("Numbers").unwrap().children()[0]
                .get("tall")
                .and_then(Entry::text),
            Some("64")
        );
        assert_eq!(fonts.get("Text").unwrap().children().len(), 2);
    }
    #[test]
    fn utf16_localization_and_animation_conditions_are_not_corrupted() {
        let bytes = [0xff, 0xfe, b'H', 0, b'L', 0, b'2', 0];
        assert_eq!(keyvalues::decode_text(&bytes).unwrap(), "HL2");
        let rules = animations("event OpenWeaponSelectionMenu\n{\nAnimate HudWeaponSelection Alpha 128 Linear 0 0.1 [!$DECK]\nAnimate HudWeaponSelection Alpha 192 Linear 0 0.1 [$DECK]\n}\nevent DeckOnly [$DECK]\n{\nAnimate HudWeaponSelection Alpha 192 Linear 0 0.1\n}").unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].target, "128");
        assert!(keyvalues::parse_resource(b"Scheme { x 1").is_err());
    }
}
