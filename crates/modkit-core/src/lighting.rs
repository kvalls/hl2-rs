//! Source model lighting: leaf ambient cubes and world lights reduced to an
//! ambient cube plus up to four local lights per illumination origin.
//!
//! Behavior follows the retail engine light cache (reviewed privately): leaf
//! ambient samples weighted by 1/(d^2+1), world lights with falloff, style and a
//! world-only visibility trace, the strongest few kept as local lights and the
//! rest folded into the cube, and the optional ambient boost. The per-vertex
//! evaluation matches the SDK vertex shader (ambient cube, N.L or half-Lambert,
//! distance and spot attenuation).
use glam::Vec3;
use serde::{Deserialize, Serialize};

mod brushes;

/// Retail cvar defaults (r_worldlights, r_worldlightmin, r_ambientmin,
/// r_ambientfraction) and the literal boost clamp.
pub const MAX_LOCAL_LIGHTS: usize = 4;
pub const WORLD_LIGHT_MIN: f32 = 0.0002;
pub const AMBIENT_MIN: f32 = 0.3;
pub const AMBIENT_FRACTION: f32 = 0.1;
pub const AMBIENT_BOOST_MAX: f32 = 5.0;
/// Remaining blocked distance a visibility trace tolerates near the light.
const VISIBILITY_SLACK: f32 = 8.0;
/// Skylight visibility trace length (retail 1.74 * 32768).
const SKY_TRACE: f32 = 57016.32;
/// Opaque world contents: CONTENTS_SOLID | CONTENTS_OPAQUE | CONTENTS_MOVEABLE.
/// CONTENTS_SOLID | CONTENTS_MOVEABLE | CONTENTS_OPAQUE: what the visibility trace stops on.
pub const MASK_OPAQUE: i32 = 0x4081;
const DWL_FLAGS_INAMBIENTCUBE: i32 = 1;
/// +X, -X, +Y, -Y, +Z, -Z: Source ambient cube face order.
pub const CUBE_AXES: [Vec3; 6] = [
    Vec3::X,
    Vec3::NEG_X,
    Vec3::Y,
    Vec3::NEG_Y,
    Vec3::Z,
    Vec3::NEG_Z,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmitType {
    Surface,
    Point,
    Spot,
    Sky,
    Quake,
    SkyAmbient,
}
impl EmitType {
    pub fn from_raw(value: i32) -> Option<Self> {
        Some(match value {
            0 => Self::Surface,
            1 => Self::Point,
            2 => Self::Spot,
            3 => Self::Sky,
            4 => Self::Quake,
            5 => Self::SkyAmbient,
            _ => return None,
        })
    }
}
/// dworldlight_t, in Source units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldLight {
    pub origin: Vec3,
    pub intensity: Vec3,
    pub normal: Vec3,
    pub kind: EmitType,
    pub style: i32,
    pub stopdot: f32,
    pub stopdot2: f32,
    pub exponent: f32,
    pub radius: f32,
    /// Constant, linear and quadratic attenuation.
    pub attenuation: [f32; 3],
    pub flags: i32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct LightNode {
    pub normal: Vec3,
    pub distance: f32,
    /// Front (>= distance) then back; negative values are -(leaf + 1).
    pub children: [i32; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LightLeaf {
    pub contents: i32,
    pub flags: u16,
    pub mins: Vec3,
    pub maxs: Vec3,
    pub ambient_count: u16,
    pub ambient_first: u16,
    /// Sky-textured face polygons bounding this leaf, for skylight visibility.
    #[serde(default)]
    pub sky_faces: Vec<Vec<Vec3>>,
    /// The opaque brushes this leaf lists (CM leafbrushes, detail brushes included), as
    /// indices into [`LightingData::brushes`].
    #[serde(default)]
    pub brushes: Vec<u32>,
}
/// One leaf ambient sample: linear cube colors and a position as fractions
/// (0..255) of its leaf bounds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AmbientSample {
    pub cube: [Vec3; 6],
    pub position: [u8; 3],
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LightingData {
    pub root: i32,
    pub nodes: Vec<LightNode>,
    pub leaves: Vec<LightLeaf>,
    pub ambient: Vec<AmbientSample>,
    pub lights: Vec<WorldLight>,
    /// The engine's per-map floor for dynamic-light minlight (`dynamic_minlight_floor`).
    #[serde(default = "default_dynamic_minlight")]
    pub dynamic_minlight: f32,
    /// The opaque world brushes leaves list ([`LightLeaf::brushes`]).
    #[serde(default)]
    pub brushes: Vec<crate::Brush>,
}
/// One visibility trace's segment, its last open leaf and its node budget.
struct Walk {
    segment: (Vec3, Vec3),
    last: Option<usize>,
    budget: usize,
    /// Whether the walk has reached an open leaf: until then it is in the solid it started in.
    opened: bool,
}
fn default_dynamic_minlight() -> f32 {
    1.
}
/// Retail CMapLoadHelper::Init (engine.dll 0x101d7e30, reviewed privately) sets the
/// minlight floor of dlight -> dworldlight_t per map: BSP version < 20 -> 0.078125;
/// else from LUMP_MAP_FLAGS: LVLFLAGS_BAKED_STATIC_PROP_LIGHTING_NONHDR (0x100) ->
/// 0.078125, _HDR (0x200) -> 0.01953125, neither -> 1/256; without the lump it keeps
/// its previous value (1.0 initially). A material-system query (vtable +0xd8, likely
/// InEditorMode) forces 1/256; it is false in ordinary play (inferred).
pub fn dynamic_minlight_floor(bsp_version: u32, map_flags: Option<u32>) -> f32 {
    if bsp_version < 20 {
        return 0.078125;
    }
    match map_flags {
        Some(f) if f & 0x100 != 0 => 0.078125,
        Some(f) if f & 0x200 != 0 => 0.019_531_25,
        Some(_) => 1. / 256.,
        None => 1.,
    }
}

/// studiohdr_t illumposition (model space) and flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelIllumination {
    pub position: Vec3,
    pub flags: u32,
}
impl ModelIllumination {
    pub const AMBIENT_BOOST: u32 = 1 << 16;
    pub fn ambient_boost(&self) -> bool {
        self.flags & Self::AMBIENT_BOOST != 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ShaderLightKind {
    Point,
    Spot,
    Directional,
}
/// A local light as the vertex shader consumes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShaderLight {
    pub kind: ShaderLightKind,
    pub color: Vec3,
    pub position: Vec3,
    /// Spot axis, or the direction light travels for directional lights.
    pub direction: Vec3,
    pub attenuation: [f32; 3],
    pub exponent: f32,
    /// Cosine of the inner and outer cone; spot attenuation is
    /// (cos - outer) / (inner - outer), raised to `exponent`.
    pub inner: f32,
    pub outer: f32,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LightState {
    pub ambient: [Vec3; 6],
    pub lights: Vec<ShaderLight>,
    /// World light indices kept as local lights, strongest selection first;
    /// `DYNAMIC_SOURCE` for a dynamic (entity) light added by `add_dynamic`.
    #[serde(default)]
    pub sources: Vec<usize>,
    /// Selection importance of each local light: luminance(intensity) * ratio.
    #[serde(default)]
    pub importance: Vec<f32>,
}
/// Whether a dynamic light reaches the 32 x 32 x 128 light-cache cell around `point`
/// (retail engine.dll 0x101d71a0 cell, 0x1026b870 squared box distance <= radius^2).
pub fn dynamic_reaches(point: Vec3, light: &WorldLight) -> bool {
    let cell = Vec3::new(32., 32., 128.);
    let mins = (point / cell).floor() * cell;
    let maxs = mins + cell;
    let outside = (mins - light.origin)
        .max(light.origin - maxs)
        .max(Vec3::ZERO);
    outside.length_squared() <= light.radius * light.radius
}
/// `LightState::sources` entry of a dynamic light.
pub const DYNAMIC_SOURCE: usize = usize::MAX;

/// The dlight_t fields an entity light (CL_AllocElight) carries into model lighting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DynamicLight {
    pub origin: Vec3,
    pub radius: f32,
    /// ColorRGBExp32.
    pub color: [u8; 3],
    pub exponent: i8,
    pub minlight: f32,
}
impl DynamicLight {
    /// Retail engine dlight -> dworldlight_t (engine.dll 0x101d9920, reviewed privately):
    /// intensity = rgb * 2^exponent / 255, emit_point, radius >= 0.1, no constant or
    /// linear term and quadratic = 1 / (radius^2 * max(minlight, floor)), so the light is
    /// `minlight` times its intensity at its radius and grows as 1/d^2 inside it.
    /// `floor` is `LightingData::dynamic_minlight`.
    pub fn world_light(&self, floor: f32) -> WorldLight {
        let scale = 2f32.powi(i32::from(self.exponent)) / 255.;
        let radius = self.radius.max(0.1);
        let minlight = if self.minlight <= floor {
            floor
        } else {
            self.minlight
        };
        WorldLight {
            origin: self.origin,
            intensity: Vec3::from_array(self.color.map(f32::from)) * scale,
            normal: Vec3::ZERO,
            kind: EmitType::Point,
            style: 0,
            stopdot: 0.,
            stopdot2: 0.,
            exponent: 0.,
            radius,
            attenuation: [0., 0., 1. / (radius * minlight * radius)],
            flags: 0,
        }
    }
}

fn luminance(c: Vec3) -> f32 {
    c.x * 0.299 + c.y * 0.587 + c.z * 0.114
}
fn boost_luminance(c: Vec3) -> f32 {
    c.x * 0.30 + c.y * 0.59 + c.z * 0.11
}

/// Which light cache cell holds a point. Cells are 32 by 32 by 128 units; below zero the retail
/// cell count runs from -1, so a point exactly on a negative edge falls one cell lower.
pub fn cache_cell(point: Vec3) -> [i32; 3] {
    let cell = |value: f32, shift: u32| {
        let count = (value.abs() as i32) >> shift;
        (if value < 0. { -1 - count } else { count }) << shift
    };
    [cell(point.x, 5), cell(point.y, 5), cell(point.z, 7)]
}

impl LightingData {
    /// Where the light cache reads the world for `point` (r_lightcachecenter, on by default): the
    /// middle of its cell, then that at the point's own height, whichever the point can reach
    /// past opaque leaves, else the point itself.
    pub fn cache_point(&self, point: Vec3) -> Vec3 {
        let [x, y, z] = cache_cell(point).map(|edge| edge as f32);
        let center = Vec3::new(x + 16., y + 16., z + 64.);
        let level = Vec3::new(center.x, center.y, point.z);
        [center, level]
            .into_iter()
            .find(|target| self.trace(point, *target).is_none())
            .unwrap_or(point)
    }
    /// Containing leaf of a point, or None for malformed trees.
    pub fn leaf_at(&self, point: Vec3) -> Option<usize> {
        if !point.is_finite() {
            return None;
        }
        let mut id = self.root;
        for _ in 0..=self.nodes.len() {
            if id < 0 {
                let leaf = (-id - 1) as usize;
                return (leaf < self.leaves.len()).then_some(leaf);
            }
            let node = self.nodes.get(id as usize)?;
            id = node.children[usize::from(node.normal.dot(point) < node.distance)];
        }
        None
    }
    /// Fraction along start->end where the segment first enters an opaque leaf or one of
    /// the opaque brushes a leaf lists, and the last open leaf before it. A segment that
    /// starts in solid is startsolid: only a brush it enters stops it (CM_ClipBoxToBrush).
    pub fn trace(&self, start: Vec3, end: Vec3) -> Option<(f32, Option<usize>)> {
        let mut walk = Walk {
            segment: (start, end),
            last: None,
            budget: 4 * self.nodes.len() + 64,
            opened: false,
        };
        self.trace_node(self.root, (0., 1.), (start, end), &mut walk)
            .map(|f| (f, walk.last))
    }
    fn trace_node(
        &self,
        id: i32,
        (f1, f2): (f32, f32),
        (p1, p2): (Vec3, Vec3),
        walk: &mut Walk,
    ) -> Option<f32> {
        if walk.budget == 0 {
            return None;
        }
        walk.budget -= 1;
        if id < 0 {
            let leaf = (-id - 1) as usize;
            let Some(record) = self.leaves.get(leaf) else {
                return Some(f1);
            };
            let hit = self.leaf_brush_hit(record, walk.segment, (f1, f2));
            if record.contents & MASK_OPAQUE != 0 {
                // Entered from open space, a solid leaf stops it even where no brush is listed;
                // without brushes to say where the solid ends, one it starts in stops it too.
                return if walk.opened || record.brushes.is_empty() {
                    hit.or(Some(f1))
                } else {
                    hit
                };
            }
            walk.opened = true;
            walk.last = Some(leaf);
            return hit;
        }
        let node = self.nodes.get(id as usize)?;
        let t1 = node.normal.dot(p1) - node.distance;
        let t2 = node.normal.dot(p2) - node.distance;
        if t1 >= 0. && t2 >= 0. {
            return self.trace_node(node.children[0], (f1, f2), (p1, p2), walk);
        }
        if t1 < 0. && t2 < 0. {
            return self.trace_node(node.children[1], (f1, f2), (p1, p2), walk);
        }
        let side = usize::from(t1 < 0.);
        let frac = (t1 / (t1 - t2)).clamp(0., 1.);
        let mid = p1 + (p2 - p1) * frac;
        let fm = f1 + (f2 - f1) * frac;
        self.trace_node(node.children[side], (f1, fm), (p1, mid), walk)
            .or_else(|| self.trace_node(node.children[1 - side], (fm, f2), (mid, p2), walk))
    }
    /// Mod_LeafAmbientColorAtPos: inverse-square-plus-one weighted leaf samples.
    pub fn leaf_ambient(&self, leaf: usize, point: Vec3) -> [Vec3; 6] {
        let mut out = [Vec3::ZERO; 6];
        let Some(mut record) = self.leaves.get(leaf) else {
            return out;
        };
        if record.ambient_count == 0 && record.ambient_first != 0 {
            match self.leaves.get(usize::from(record.ambient_first)) {
                Some(shared) => record = shared,
                None => return out,
            }
        }
        let first = usize::from(record.ambient_first);
        let count = usize::from(record.ambient_count);
        let Some(samples) = self.ambient.get(first..first + count) else {
            return out;
        };
        if samples.is_empty() {
            return out;
        }
        let size = record.maxs - record.mins;
        let mut total = 0.;
        for sample in samples {
            let fraction = Vec3::from_array(sample.position.map(f32::from)) / 255.;
            let position = record.mins + fraction * size;
            let weight = 1. / (point.distance_squared(position) + 1.);
            total += weight;
            for (face, color) in out.iter_mut().zip(&sample.cube) {
                *face += *color * weight;
            }
        }
        for face in &mut out {
            *face /= total;
        }
        out
    }
    /// LightIntensityAndDirectionAtPoint: (ratio, unit direction toward light).
    fn intensity_at(&self, light: &WorldLight, point: Vec3) -> Option<(f32, Vec3)> {
        if light.kind == EmitType::Sky {
            let (fraction, last) = self.trace(point, point - light.normal * SKY_TRACE)?;
            let hit = point - light.normal * SKY_TRACE * fraction;
            let sees_sky = last
                .and_then(|leaf| self.leaves.get(leaf))
                .is_some_and(|leaf| leaf.sky_faces.iter().any(|f| point_on_face(f, hit)));
            return sees_sky.then_some((1., -light.normal));
        }
        if light.kind == EmitType::SkyAmbient {
            return None;
        }
        let delta = light.origin - point;
        let distance_squared = delta.length_squared();
        let distance = distance_squared.sqrt();
        let falloff = match light.kind {
            EmitType::Surface => {
                if light.radius != 0. && distance_squared > light.radius * light.radius {
                    return None;
                }
                1. / (distance_squared + 1e-10).max(1.)
            }
            EmitType::Point | EmitType::Spot => {
                if light.radius != 0. && distance > light.radius {
                    return None;
                }
                let [c, l, q] = light.attenuation;
                1. / (q * distance_squared + l * distance + c)
            }
            EmitType::Quake => (light.attenuation[1] - distance).max(0.),
            _ => 1.,
        };
        // Styled lights are treated as switched on at their authored brightness.
        let ratio = falloff;
        if !ratio.is_finite() || ratio <= 0. {
            return None;
        }
        if light.kind != EmitType::Surface
            && light.intensity.max_element() * ratio < WORLD_LIGHT_MIN
        {
            return None;
        }
        let direction = delta / (distance_squared + 1e-10).sqrt();
        if let Some((fraction, _)) = self.trace(point, light.origin) {
            if distance * (1. - fraction) > VISIBILITY_SLACK {
                return None;
            }
        }
        Some((ratio, direction))
    }
    /// Retail ComputeStaticLightingState for one illumination origin.
    pub fn state_at(&self, point: Vec3) -> LightState {
        let mut state = LightState::default();
        let leaf = self.leaf_at(point);
        let leaf_ambient = leaf.is_some();
        if let Some(leaf) = leaf {
            state.ambient = self.leaf_ambient(leaf, point);
        }
        let mut kept: Vec<(usize, f32)> = Vec::new();
        let mut sky_seen = false;
        for (index, light) in self.lights.iter().enumerate() {
            if leaf_ambient && light.flags & DWL_FLAGS_INAMBIENTCUBE != 0 {
                continue;
            }
            if light.kind == EmitType::Sky && sky_seen {
                continue;
            }
            let Some((ratio, direction)) = self.intensity_at(light, point) else {
                continue;
            };
            if light.kind == EmitType::Sky {
                sky_seen = true;
            }
            let lum = luminance(light.intensity) * ratio;
            let mut ambient = Some((index, ratio, direction));
            if light.kind == EmitType::Surface || lum >= WORLD_LIGHT_MIN {
                if kept.len() < MAX_LOCAL_LIGHTS {
                    kept.push((index, lum));
                    ambient = None;
                } else if let Some(weakest) = kept
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, l))| *l < lum)
                    .min_by(|a, b| a.1 .1.total_cmp(&b.1 .1))
                    .map(|(i, _)| i)
                {
                    let (evicted, evicted_lum) =
                        std::mem::replace(&mut kept[weakest], (index, lum));
                    let old = &self.lights[evicted];
                    let ratio = evicted_lum / luminance(old.intensity);
                    let direction = if old.kind == EmitType::Sky {
                        -old.normal
                    } else {
                        (old.origin - point).normalize_or_zero()
                    };
                    ambient = Some((evicted, ratio, direction));
                }
            }
            if let Some((index, ratio, direction)) = ambient {
                let light = &self.lights[index];
                let scale = angular(light, direction) * ratio;
                add_to_cube(&mut state.ambient, light.intensity, direction, scale);
            }
        }
        for (index, lum) in kept {
            if let Some(shader) = shader_light(&self.lights[index]) {
                state.lights.push(shader);
                state.sources.push(index);
                state.importance.push(lum);
            }
        }
        state
    }
    /// Retail ComputeDynamicLighting for one entity light (engine.dll 0x101d54a0 ->
    /// 0x101d5cf0 with the dynamic flags): skipped when the light is farther than its
    /// radius from the 32 x 32 x 128 light-cache cell around `point`; the ratio at
    /// `point` is the bare falloff (no radius test, no visibility trace); the light
    /// always takes a local slot, evicting the weakest light into the ambient cube.
    pub fn add_dynamic(&self, state: &mut LightState, point: Vec3, light: &WorldLight) {
        if !dynamic_reaches(point, light) {
            return;
        }
        let delta = light.origin - point;
        let distance_squared = delta.length_squared();
        let [c, l, q] = light.attenuation;
        let ratio = 1. / (q * distance_squared + l * distance_squared.sqrt() + c);
        if !ratio.is_finite()
            || ratio <= 0.
            || light.intensity.max_element() * ratio < WORLD_LIGHT_MIN
        {
            return;
        }
        let Some(shader) = shader_light(light) else {
            return;
        };
        let importance = luminance(light.intensity) * ratio;
        if state.lights.len() < MAX_LOCAL_LIGHTS {
            state.lights.push(shader);
            state.sources.push(DYNAMIC_SOURCE);
            state.importance.push(importance);
            return;
        }
        // The eviction threshold is 100000 for dynamic lights instead of their own importance.
        let Some(weakest) = state
            .importance
            .iter()
            .enumerate()
            .filter(|(_, i)| **i < 100_000.)
            .min_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
        else {
            return;
        };
        let old = std::mem::replace(&mut state.lights[weakest], shader);
        let old_source = std::mem::replace(&mut state.sources[weakest], DYNAMIC_SOURCE);
        let old_importance = std::mem::replace(&mut state.importance[weakest], importance);
        let ratio = old_importance / luminance(old.color);
        match self.lights.get(old_source) {
            Some(world) if world.kind == EmitType::Sky => {
                let direction = -world.normal;
                add_to_cube(
                    &mut state.ambient,
                    world.intensity,
                    direction,
                    angular(world, direction) * ratio,
                );
            }
            Some(world) => {
                let direction = (world.origin - point).normalize_or_zero();
                add_to_cube(
                    &mut state.ambient,
                    world.intensity,
                    direction,
                    angular(world, direction) * ratio,
                );
            }
            None => {
                let direction = (old.position - point).normalize_or_zero();
                add_to_cube(&mut state.ambient, old.color, direction, ratio);
            }
        }
    }
    /// Draw-time ambient boost for models with STUDIOHDR_FLAGS_AMBIENT_BOOST.
    pub fn boost(&self, state: &mut LightState, origin: Vec3) {
        if state.sources.is_empty() {
            return;
        }
        let faces = state.ambient.map(boost_luminance);
        let max = faces.iter().copied().fold(0f32, f32::max);
        let average = faces.iter().sum::<f32>() / 6.;
        let direct: f32 = state
            .sources
            .iter()
            .filter_map(|&i| self.lights.get(i))
            .map(|light| {
                let d2 = light.origin.distance_squared(origin);
                let [c, l, q] = light.attenuation;
                let denominator = d2.sqrt() * l + c + q * d2;
                let falloff = if denominator > 1e-5 {
                    1. / denominator
                } else {
                    1.
                };
                boost_luminance(light.intensity * falloff)
            })
            .sum();
        let threshold = AMBIENT_FRACTION * direct;
        if average < AMBIENT_MIN && average < threshold {
            let scale = (threshold / max).min(AMBIENT_BOOST_MAX);
            for face in &mut state.ambient {
                *face *= scale;
            }
        }
    }
}
/// Angular term for a light reaching `direction` (unit vector toward the light).
fn angular(light: &WorldLight, direction: Vec3) -> f32 {
    let toward = -light.normal.dot(direction);
    match light.kind {
        EmitType::Surface => {
            if toward > 0.01 {
                toward
            } else {
                0.
            }
        }
        EmitType::Point | EmitType::Quake | EmitType::SkyAmbient => 1.,
        EmitType::Spot => {
            if toward <= light.stopdot2 {
                0.
            } else if toward >= light.stopdot {
                1.
            } else {
                let ramp = (toward - light.stopdot2) / (light.stopdot - light.stopdot2);
                if light.exponent == 0. || light.exponent == 1. {
                    ramp
                } else {
                    ramp.powf(light.exponent)
                }
            }
        }
        EmitType::Sky => toward.max(0.),
    }
}
fn add_to_cube(cube: &mut [Vec3; 6], intensity: Vec3, direction: Vec3, scale: f32) {
    if scale == 0. {
        return;
    }
    for (face, axis) in cube.iter_mut().zip(CUBE_AXES) {
        let d = axis.dot(direction);
        if d > 0. {
            *face += intensity * d * scale;
        }
    }
}
/// Engine WorldLightToMaterialLight: quake and sky-ambient lights are dropped.
fn shader_light(light: &WorldLight) -> Option<ShaderLight> {
    let mut attenuation = light.attenuation;
    let (kind, exponent, inner, outer) = match light.kind {
        EmitType::Surface => {
            attenuation = [0., 0., 1.];
            (ShaderLightKind::Spot, 1., 0., 0.)
        }
        EmitType::Point => (ShaderLightKind::Point, 0., 0., 0.),
        EmitType::Spot => (
            ShaderLightKind::Spot,
            if light.exponent == 0. {
                1.
            } else {
                light.exponent
            },
            light.stopdot,
            light.stopdot2,
        ),
        EmitType::Sky => (ShaderLightKind::Directional, 0., 0., 0.),
        EmitType::Quake | EmitType::SkyAmbient => return None,
    };
    if attenuation == [0.; 3] {
        attenuation[0] = 1.;
    }
    Some(ShaderLight {
        kind,
        color: light.intensity,
        position: light.origin,
        direction: light.normal,
        attenuation,
        exponent,
        inner,
        outer,
    })
}
fn point_on_face(polygon: &[Vec3], point: Vec3) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let normal = (polygon[1] - polygon[0])
        .cross(polygon[2] - polygon[0])
        .normalize_or_zero();
    if normal == Vec3::ZERO || normal.dot(point - polygon[0]).abs() > 2. {
        return false;
    }
    let mut sign = 0f32;
    for (i, a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        let side = (b - *a).cross(point - *a).dot(normal);
        if side.abs() < 0.5 {
            continue;
        }
        if sign == 0. {
            sign = side.signum();
        } else if side.signum() != sign {
            return false;
        }
    }
    true
}
impl ShaderLight {
    /// SDK VertexAttenInternal.
    pub fn attenuation_at(&self, point: Vec3) -> f32 {
        if self.kind == ShaderLightKind::Directional {
            return 1.;
        }
        let delta = self.position - point;
        let d2 = delta.length_squared();
        let d = d2.sqrt();
        let [c, l, q] = self.attenuation;
        let distance = 1. / (c + l * d + q * d2);
        if self.kind == ShaderLightKind::Point {
            return distance;
        }
        let cos = self.direction.dot(-delta / d.max(1e-6));
        let scale = if self.inner > self.outer {
            1. / (self.inner - self.outer)
        } else {
            1.
        };
        let spot = ((cos - self.outer) * scale)
            .max(0.0001)
            .powf(self.exponent)
            .clamp(0., 1.);
        distance * spot
    }
    /// SDK CosineTermInternal.
    pub fn cosine(&self, point: Vec3, normal: Vec3, half_lambert: bool) -> f32 {
        let direction = if self.kind == ShaderLightKind::Directional {
            -self.direction
        } else {
            (self.position - point).normalize_or_zero()
        };
        let d = normal.dot(direction);
        if half_lambert {
            let h = d * 0.5 + 0.5;
            h * h
        } else {
            d.max(0.)
        }
    }
}
impl LightState {
    /// SDK AmbientLight: squared-normal weighting of the signed cube faces.
    pub fn ambient_light(&self, normal: Vec3) -> Vec3 {
        let n2 = normal * normal;
        let pick = |positive: usize, value: f32| self.ambient[positive + usize::from(value < 0.)];
        pick(0, normal.x) * n2.x + pick(2, normal.y) * n2.y + pick(4, normal.z) * n2.z
    }
    /// Linear diffuse lighting at a vertex (DoLighting without static light).
    pub fn vertex_light(&self, point: Vec3, normal: Vec3, half_lambert: bool) -> Vec3 {
        self.lights
            .iter()
            .fold(self.ambient_light(normal), |sum, light| {
                sum + light.color
                    * light.cosine(point, normal, half_lambert)
                    * light.attenuation_at(point)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// One plane at x=0: front leaf 0 is open, back leaf 1 is solid.
    pub(super) fn split_world() -> LightingData {
        let leaf = |contents| LightLeaf {
            contents,
            flags: 0,
            mins: Vec3::splat(-64.),
            maxs: Vec3::splat(64.),
            ambient_count: 0,
            ambient_first: 0,
            sky_faces: Vec::new(),
            brushes: Vec::new(),
        };
        LightingData {
            root: 0,
            nodes: vec![LightNode {
                normal: Vec3::X,
                distance: 0.,
                children: [-1, -2],
            }],
            leaves: vec![leaf(0), leaf(1)],
            ambient: Vec::new(),
            lights: Vec::new(),
            dynamic_minlight: 1.,
            brushes: Vec::new(),
        }
    }
    pub(super) fn point_light(origin: Vec3, intensity: f32) -> WorldLight {
        WorldLight {
            origin,
            intensity: Vec3::splat(intensity),
            normal: Vec3::ZERO,
            kind: EmitType::Point,
            style: 0,
            stopdot: 0.,
            stopdot2: 0.,
            exponent: 0.,
            radius: 0.,
            attenuation: [0., 0., 1.],
            flags: 0,
        }
    }
    #[test]
    fn leaf_ambient_weights_samples_by_inverse_square_plus_one() {
        let mut data = split_world();
        data.leaves[0].ambient_count = 2;
        data.ambient = vec![
            AmbientSample {
                cube: [Vec3::ONE; 6],
                position: [0, 0, 0],
            },
            AmbientSample {
                cube: [Vec3::ZERO; 6],
                position: [255, 255, 255],
            },
        ];
        // At the first sample: weights 1 and 1/(d^2+1) with d^2 = 3*128^2.
        let far = 1. / (3. * 128f32.powi(2) + 1.);
        let cube = data.leaf_ambient(0, Vec3::splat(-64.));
        assert!((cube[0].x - 1. / (1. + far)).abs() < 1e-6);
        // Shared redirect: count 0 with first != 0 uses that leaf's samples.
        data.leaves[1].ambient_count = 0;
        data.leaves[1].ambient_first = 0;
        assert_eq!(data.leaf_ambient(1, Vec3::ZERO), [Vec3::ZERO; 6]);
    }
    #[test]
    fn cache_cells_are_32_by_32_by_128_and_negative_ones_count_from_minus_one() {
        assert_eq!(cache_cell(Vec3::new(5., 40., 130.)), [0, 32, 128]);
        assert_eq!(cache_cell(Vec3::new(-5., -40., -130.)), [-32, -64, -256]);
        assert_eq!(cache_cell(Vec3::new(-32., 0., -128.)), [-64, 0, -256]);
    }

    #[test]
    fn the_cache_point_is_the_cell_middle_the_point_can_reach() {
        // split_world is solid below x = 0 and open from there on.
        let open = Vec3::new(40., 3., 10.);
        assert_eq!(split_world().cache_point(open), Vec3::new(48., 16., 64.));

        // Solid above z = 40: the cell's middle is out of reach, its own height is not.
        let mut ceiling = split_world();
        ceiling.nodes[0].normal = Vec3::NEG_Z;
        ceiling.nodes[0].distance = -40.;
        let low = Vec3::new(5., 3., 10.);
        assert_eq!(ceiling.cache_point(low), Vec3::new(16., 16., 10.));

        // Solid below x = 20: neither is in reach, so the point stands for itself.
        let mut wall = split_world();
        wall.nodes[0].distance = 20.;
        let beside = Vec3::new(25., 3., 10.);
        assert_eq!(wall.cache_point(beside), beside);

        // A point inside solid reaches nothing.
        let inside = Vec3::new(-40., 3., 10.);
        assert_eq!(split_world().cache_point(inside), inside);
    }

    #[test]
    fn traces_stop_at_solid_leaves_and_hide_lights_behind_them() {
        let mut data = split_world();
        let (fraction, last) = data
            .trace(Vec3::new(10., 0., 0.), Vec3::new(-10., 0., 0.))
            .unwrap();
        assert!((fraction - 0.5).abs() < 1e-6);
        assert_eq!(last, Some(0));
        assert!(data
            .trace(Vec3::new(10., 0., 0.), Vec3::new(20., 0., 0.))
            .is_none());
        data.lights = vec![
            point_light(Vec3::new(-100., 0., 0.), 1000.),
            point_light(Vec3::new(100., 0., 0.), 1000.),
        ];
        let state = data.state_at(Vec3::new(1., 0., 0.));
        assert_eq!(state.sources, vec![1]);
        // A light just inside a wall (within the 8-unit slack) still counts.
        data.lights = vec![point_light(Vec3::new(-4., 0., 0.), 1000.)];
        assert_eq!(data.state_at(Vec3::new(50., 0., 0.)).sources, vec![0]);
    }
    #[test]
    fn fifth_light_evicts_the_weakest_into_the_ambient_cube() {
        let mut data = split_world();
        data.lights = (1..=5)
            .map(|i| point_light(Vec3::new(10., 0., 100.), i as f32 * 1000.))
            .collect();
        let state = data.state_at(Vec3::new(10., 0., 0.));
        assert_eq!(state.sources, vec![4, 1, 2, 3]);
        // The evicted light (index 0, 1000/100^2 = 0.1) lands on the +Z face only.
        assert!((state.ambient[4].x - 0.1).abs() < 1e-5);
        assert_eq!(state.ambient[5], Vec3::ZERO);
        // Weak lights below r_worldlightmin are rejected outright.
        data.lights = vec![point_light(Vec3::new(10., 0., 100.), 1e-3)];
        assert_eq!(data.state_at(Vec3::new(10., 0., 0.)), LightState::default());
    }
    #[test]
    fn muzzle_elight_converts_and_always_takes_a_local_slot() {
        // C_BaseAnimating::ProcessMuzzleFlashEvent: (255, 192, 64) exponent 5, radius 48.
        let muzzle = DynamicLight {
            origin: Vec3::new(10., 0., 20.),
            radius: 48.,
            color: [255, 192, 64],
            exponent: 5,
            minlight: 0.,
        }
        .world_light(1.);
        assert!((muzzle.intensity - Vec3::new(32., 24.094118, 8.031373)).length() < 1e-4);
        assert_eq!(muzzle.attenuation, [0., 0., 1. / 2304.]);
        // HL2 BSP 20 maps with baked HDR static-prop lighting (0x200): 0.01953125.
        assert_eq!(dynamic_minlight_floor(20, Some(0x200)), 0.019_531_25);
        assert_eq!(dynamic_minlight_floor(20, Some(0x203)), 0.019_531_25);
        assert_eq!(dynamic_minlight_floor(20, Some(0x100)), 0.078125);
        assert_eq!(dynamic_minlight_floor(19, None), 0.078125);
        assert_eq!(dynamic_minlight_floor(20, Some(0)), 1. / 256.);
        let hdr = DynamicLight {
            origin: Vec3::ZERO,
            radius: 48.,
            color: [255, 192, 64],
            exponent: 5,
            minlight: 0.,
        }
        .world_light(0.019_531_25);
        assert_eq!(hdr.attenuation[2], 1. / (2304. * 0.019_531_25));
        let mut data = split_world();
        data.lights = (1..=4)
            .map(|i| point_light(Vec3::new(10., 0., 100.), i as f32 * 1000.))
            .collect();
        let point = Vec3::new(10., 0., 0.);
        let mut state = data.state_at(point);
        assert_eq!(state.sources, vec![0, 1, 2, 3]);
        assert_eq!(state.ambient[4], Vec3::ZERO);
        data.add_dynamic(&mut state, point, &muzzle);
        // The weakest world light (1000 / 100^2) moves into the +Z face.
        assert_eq!(state.sources, vec![DYNAMIC_SOURCE, 1, 2, 3]);
        assert!((state.ambient[4].x - 0.1).abs() < 1e-5);
        assert_eq!(state.lights[0].attenuation, muzzle.attenuation);
        // Ratio at the point: 48^2 / 20^2.
        let lum = 32. * 0.299 + 24.094118 * 0.587 + 8.031373 * 0.114;
        assert!((state.importance[0] - lum * 5.76).abs() < 1e-3);
        // Beyond its radius from the 32 x 32 x 128 cell around the point: no effect.
        let far = DynamicLight {
            origin: Vec3::new(10., 0., 200.),
            ..DynamicLight {
                origin: Vec3::ZERO,
                radius: 48.,
                color: [255, 192, 64],
                exponent: 5,
                minlight: 0.,
            }
        }
        .world_light(1.);
        let before = state.clone();
        data.add_dynamic(&mut state, point, &far);
        assert_eq!(state, before);
    }
    #[test]
    fn ambient_cube_cleared_lights_skip_baked_lights_and_boost_dark_cubes() {
        let mut data = split_world();
        let mut baked = point_light(Vec3::new(10., 0., 100.), 1000.);
        baked.flags = DWL_FLAGS_INAMBIENTCUBE;
        data.lights = vec![baked];
        assert!(data.state_at(Vec3::new(10., 0., 0.)).sources.is_empty());
        data.lights[0].flags = 0;
        let mut state = data.state_at(Vec3::new(10., 0., 0.));
        state.ambient = [Vec3::splat(0.01); 6];
        data.boost(&mut state, Vec3::new(10., 0., 0.));
        // direct = 0.1, threshold 0.01 > average 0.01? equal -> no boost.
        assert!((state.ambient[0].x - 0.01).abs() < 1e-6);
        state.ambient = [Vec3::splat(0.005); 6];
        data.boost(&mut state, Vec3::new(10., 0., 0.));
        assert!((state.ambient[0].x - 0.01).abs() < 1e-5);
    }
    #[test]
    fn vertex_light_matches_sdk_shader_terms() {
        let state = LightState {
            ambient: [
                Vec3::X,
                Vec3::Y,
                Vec3::Z,
                Vec3::ONE,
                Vec3::splat(2.),
                Vec3::splat(3.),
            ],
            lights: vec![ShaderLight {
                kind: ShaderLightKind::Point,
                color: Vec3::splat(100.),
                position: Vec3::new(0., 0., 10.),
                direction: Vec3::ZERO,
                attenuation: [0., 0., 1.],
                exponent: 0.,
                inner: 0.,
                outer: 0.,
            }],
            sources: vec![0],
            importance: vec![1.],
        };
        assert_eq!(state.ambient_light(Vec3::NEG_X), Vec3::Y);
        let up = state.vertex_light(Vec3::ZERO, Vec3::Z, false);
        assert!((up - Vec3::splat(2. + 1.)).length() < 1e-5);
        // Facing away: Lambert gives no direct light, half-Lambert gives 0.
        let down = state.vertex_light(Vec3::ZERO, Vec3::NEG_Z, true);
        assert!((down - Vec3::splat(3.)).length() < 1e-5);
        let side = state.vertex_light(Vec3::ZERO, Vec3::X, true);
        assert!((side - (Vec3::X + Vec3::splat(0.25))).length() < 1e-5);
    }
    #[test]
    fn surface_and_spot_lights_convert_like_the_engine() {
        let mut surface = point_light(Vec3::ZERO, 10.);
        surface.kind = EmitType::Surface;
        surface.normal = Vec3::NEG_Z;
        let shader = shader_light(&surface).unwrap();
        // 90-degree emitter: cosine falloff with 1/d^2.
        let below = shader.attenuation_at(Vec3::new(0., 0., -2.));
        assert!((below - 0.25).abs() < 1e-5);
        let slanted = shader.attenuation_at(Vec3::new(2., 0., -2.));
        assert!((slanted - (1. / 8.) * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        let mut spot = point_light(Vec3::ZERO, 10.);
        spot.kind = EmitType::Spot;
        spot.normal = Vec3::NEG_Z;
        spot.stopdot = 0.9;
        spot.stopdot2 = 0.5;
        assert!((angular(&spot, Vec3::Z) - 1.).abs() < 1e-6);
        assert_eq!(angular(&spot, Vec3::X), 0.);
        let mut quake = point_light(Vec3::ZERO, 1.);
        quake.kind = EmitType::Quake;
        assert!(shader_light(&quake).is_none());
    }
}
