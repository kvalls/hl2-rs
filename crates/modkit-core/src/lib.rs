//! Game-neutral data and extension boundary. All positions are Z-up, in Source units.
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
pub mod animation;
pub mod decals;
pub mod lighting;
pub mod movement;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vertex {
    pub position: Vec3,
    pub uv: Vec2,
    pub color: [u8; 4],
    #[serde(default)]
    pub light_uv: Vec2,
    #[serde(default)]
    pub skin: Option<animation::Weights>,
    /// Unit model-space normal (studio models); zero where unused.
    #[serde(default)]
    pub normal: Vec3,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Surface {
    #[serde(default)]
    pub background: bool,
    pub material: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    #[serde(default)]
    pub lightmap: Option<usize>,
    /// Studio mesh this surface came from and each vertex's mesh-local index, so facial
    /// flex vertex deltas can be applied.
    #[serde(default)]
    pub flex_source: Option<FlexSource>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlexSource {
    pub bodypart: usize,
    pub model: usize,
    pub mesh: usize,
    pub vertex_ids: Vec<u16>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entity {
    pub properties: Vec<(String, String)>,
}
impl Entity {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
    pub fn class(&self) -> &str {
        self.get("classname").unwrap_or("unknown")
    }
    pub fn origin(&self) -> Vec3 {
        parse_vec3(self.get("origin").unwrap_or("0 0 0")).unwrap_or(Vec3::ZERO)
    }
}
pub fn parse_vec3(s: &str) -> Option<Vec3> {
    let v: Vec<f32> = s
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (v.len() == 3 && v.iter().all(|v| v.is_finite())).then(|| Vec3::new(v[0], v[1], v[2]))
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Plane {
    pub normal: Vec3,
    pub distance: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Brush {
    pub planes: Vec<Plane>,
    pub contents: u32,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct World {
    #[serde(default)]
    pub background_camera: Option<BackgroundCamera>,
    #[serde(default)]
    pub background_entities: Vec<usize>,
    pub name: String,
    pub surfaces: Vec<Surface>,
    pub brushes: Vec<Brush>,
    pub entities: Vec<Entity>,
    #[serde(default)]
    pub model_instances: Vec<ModelInstance>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub brush_models: Vec<BrushModel>,
    #[serde(default)]
    pub terrain: Vec<Surface>,
    /// info_overlay fragments on world faces (drawn like world surfaces; not decal or impact
    /// receivers).
    #[serde(default)]
    pub overlays: Vec<Surface>,
    #[serde(default)]
    pub model_assets: std::collections::BTreeMap<String, Vec<Surface>>,
    /// Collision convexes are separate from visible model triangles.
    #[serde(default)]
    pub model_collision: std::collections::BTreeMap<String, Vec<ConvexPiece>>,
    #[serde(default)]
    pub lightmaps: Vec<Lightmap>,
    #[serde(default)]
    pub rigs: std::collections::BTreeMap<String, animation::Rig>,
    /// Leaf ambient samples and world lights for model lighting (None if unreadable).
    #[serde(default)]
    pub lighting: Option<std::sync::Arc<lighting::LightingData>>,
    /// Studio illumination origin and flags per model asset key.
    #[serde(default)]
    pub illumination: std::collections::BTreeMap<String, lighting::ModelIllumination>,
    /// Authored VPhysics solid parameters (.phy "solid" block) per model asset key.
    #[serde(default)]
    pub model_physics: std::collections::BTreeMap<String, PhysicsSolid>,
    /// Resolved surfaceproperties entries (lowercase name, "base" inheritance applied).
    #[serde(default)]
    pub surface_materials: std::collections::BTreeMap<String, SurfaceMaterial>,
}
/// One .phy "solid" block. Defaults follow SDK g_PhysDefaultObjectParams
/// (physics_shared.cpp): mass 1, inertia 1, damping 0.1, rotdamping 0.1, "DEFAULT".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicsSolid {
    pub index: i32,
    /// Kilograms.
    pub mass: f32,
    /// Multiplier on the shape inertia tensor.
    pub inertia: f32,
    pub damping: f32,
    pub rotdamping: f32,
    pub surfaceprop: String,
    /// Cubic inches, 0 when absent.
    pub volume: f32,
}
impl Default for PhysicsSolid {
    fn default() -> Self {
        Self {
            index: 0,
            mass: 1.,
            inertia: 1.,
            damping: 0.1,
            rotdamping: 0.1,
            surfaceprop: "default".into(),
            volume: 0.,
        }
    }
}
/// surfacedata_t fields that gameplay uses (vphysics_interface.h), values from
/// scripts/surfaceproperties*.txt after "base" inheritance.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SurfaceMaterial {
    pub friction: f32,
    pub elasticity: f32,
    /// kg / m^3.
    pub density: f32,
    pub hardness_factor: f32,
    pub hard_threshold: f32,
    pub hard_velocity_threshold: f32,
    pub roughness_factor: f32,
    pub rough_threshold: f32,
    pub impact_hard: Option<String>,
    pub impact_soft: Option<String>,
    pub scrape_smooth: Option<String>,
    pub scrape_rough: Option<String>,
    pub break_sound: Option<String>,
    /// Game material character ('X' marks no-sound surfaces such as sky/nodraw).
    pub game_material: Option<char>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConvexPiece {
    pub vertices: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackgroundCamera {
    pub origin: Vec3,
    pub scale: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lightmap {
    pub width: u16,
    pub height: u16,
    /// Linear, unclamped baked light as IEEE half-float bits (RGBA16F texels).
    pub rgba: Vec<u16>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BrushModel {
    pub id: usize,
    pub surfaces: Vec<Surface>,
    pub brushes: Vec<Brush>,
    pub mins: Vec3,
    pub maxs: Vec3,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInstance {
    #[serde(default)]
    pub background: bool,
    pub model: String,
    pub origin: Vec3,
    /// Pitch, yaw, roll, in degrees, applied in Source's Z-up convention.
    pub angles: Vec3,
    pub skin: usize,
    pub scale: f32,
    pub kind: String,
    #[serde(default)]
    pub solid: bool,
    /// Original Source static-prop SolidType byte; absent for entity models and
    /// portable inputs that do not supply this engine-specific collision mode.
    #[serde(default)]
    pub solid_mode: Option<u8>,
    #[serde(default)]
    pub entity: Option<usize>,
}
impl ModelInstance {
    pub fn asset_key(&self) -> String {
        format!("{}#{}", self.model.to_lowercase(), self.skin)
    }
}
impl World {
    pub fn spawn(&self) -> (Vec3, f32) {
        if let Some(e) = self
            .entities
            .iter()
            .find(|e| matches!(e.class(), "info_player_start" | "info_player_deathmatch"))
        {
            let yaw = parse_vec3(e.get("angles").unwrap_or("0 0 0"))
                .unwrap_or(Vec3::ZERO)
                .y
                .to_radians();
            return (e.origin() + Vec3::Z * 64., yaw);
        }
        (Vec3::new(0., 0., 128.), 0.)
    }
    /// Player hull sweep against convex brush planes and the player brush mask.
    /// This geometric query does not implement material or moving-ground behavior.
    pub fn trace(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> Trace {
        trace_brushes(&self.brushes, start, end, mins, maxs, 0x1400b)
    }
    pub fn slide(&self, mut position: Vec3, mut delta: Vec3, mins: Vec3, maxs: Vec3) -> Vec3 {
        for _ in 0..4 {
            let hit = self.trace(position, position + delta, mins, maxs);
            if hit.all_solid {
                break;
            }
            position += delta * hit.fraction;
            if hit.fraction >= 1. {
                break;
            }
            delta *= 1. - hit.fraction;
            delta -= hit.normal * delta.dot(hit.normal).min(0.);
        }
        position
    }
    pub fn raycast(&self, origin: Vec3, direction: Vec3, max_distance: f32) -> Option<Vec3> {
        let mut nearest = max_distance;
        for surface in &self.surfaces {
            for tri in surface.indices.as_chunks::<3>().0 {
                let a = surface.vertices[tri[0] as usize].position;
                let b = surface.vertices[tri[1] as usize].position;
                let c = surface.vertices[tri[2] as usize].position;
                let edge1 = b - a;
                let edge2 = c - a;
                let p = direction.cross(edge2);
                let determinant = edge1.dot(p);
                if determinant.abs() < 1e-6 {
                    continue;
                }
                let inv = 1. / determinant;
                let t = origin - a;
                let u = t.dot(p) * inv;
                if !(0. ..=1.).contains(&u) {
                    continue;
                }
                let q = t.cross(edge1);
                let v = direction.dot(q) * inv;
                if v < 0. || u + v > 1. {
                    continue;
                }
                let distance = edge2.dot(q) * inv;
                if distance > 0. && distance < nearest {
                    nearest = distance;
                }
            }
        }
        (nearest < max_distance).then(|| origin + direction * nearest)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Trace {
    pub fraction: f32,
    pub normal: Vec3,
    pub start_solid: bool,
    /// The sweep stays inside a solid throughout, rather than merely starting inside.
    pub all_solid: bool,
}

/// Sweep an axis-aligned hull against the actual convex brush planes, including
/// BSP bevel planes. Preserve the collision epsilon without approximate GJK normals.
/// Overlap uses half that epsilon so touching a floor is not a penetrating start.
pub fn trace_brushes(
    brushes: &[Brush],
    start: Vec3,
    end: Vec3,
    mins: Vec3,
    maxs: Vec3,
    contents_mask: u32,
) -> Trace {
    let mut result = Trace {
        fraction: 1.,
        normal: Vec3::ZERO,
        start_solid: false,
        all_solid: false,
    };
    for brush in brushes {
        if brush.contents & contents_mask == 0 || brush.planes.is_empty() {
            continue;
        }
        let mut enter = -1f32;
        let mut leave = 1f32;
        let mut normal = Vec3::ZERO;
        let mut start_inside = true;
        let mut end_inside = true;
        let mut reject = false;
        for plane in &brush.planes {
            let n = plane.normal;
            let support = Vec3::new(
                if n.x >= 0. { mins.x } else { maxs.x },
                if n.y >= 0. { mins.y } else { maxs.y },
                if n.z >= 0. { mins.z } else { maxs.z },
            );
            let distance = plane.distance - n.dot(support);
            let a = n.dot(start) - distance;
            let b = n.dot(end) - distance;
            start_inside &= a < -0.015625;
            end_inside &= b < -0.015625;
            if a > 0. && b >= a {
                reject = true;
                break;
            }
            if a <= 0. && b <= 0. {
                // A touching/shallow start can move tangentially or outward;
                // movement into its boundary still has a time-zero contact.
                if a >= -0.015625 && b < a && enter < 0. {
                    enter = 0.;
                    normal = n;
                }
                continue;
            }
            if a > b {
                let f = ((a - 0.03125) / (a - b)).max(0.);
                if f > enter {
                    enter = f;
                    normal = n;
                }
            } else {
                leave = leave.min((a + 0.03125) / (a - b));
            }
        }
        if reject {
            continue;
        }
        if start_inside {
            result.start_solid = true;
            if end_inside {
                result.all_solid = true;
                result.fraction = 0.;
            }
            continue;
        }
        if enter >= 0. && enter < leave && enter < result.fraction {
            result.fraction = enter;
            result.normal = normal;
        }
    }
    result
}

/// Hosts supply assets and collision; mods add portable content without depending on Source.
pub trait ModPlugin {
    fn name(&self) -> &str;
    fn on_load(&mut self, world: &World);
    fn tick(&mut self, dt: f32, context: &mut ModContext<'_>);
}
pub struct ModContext<'a> {
    pub world: &'a World,
    pub blocks: &'a mut Vec<Block>,
    pub player: Vec3,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub position: Vec3,
    pub size: f32,
    pub color: [u8; 4],
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cube() -> World {
        World {
            brushes: vec![Brush {
                contents: 1,
                planes: vec![
                    Plane {
                        normal: Vec3::X,
                        distance: 1.,
                    },
                    Plane {
                        normal: -Vec3::X,
                        distance: 1.,
                    },
                    Plane {
                        normal: Vec3::Y,
                        distance: 1.,
                    },
                    Plane {
                        normal: -Vec3::Y,
                        distance: 1.,
                    },
                    Plane {
                        normal: Vec3::Z,
                        distance: 1.,
                    },
                    Plane {
                        normal: -Vec3::Z,
                        distance: 1.,
                    },
                ],
            }],
            ..Default::default()
        }
    }
    #[test]
    fn sweep_stops_at_wall() {
        let t = cube().trace(Vec3::new(3., 0., 0.), Vec3::ZERO, Vec3::ZERO, Vec3::ZERO);
        assert!((t.fraction - 2. / 3.).abs() < 0.02);
        assert_eq!(t.normal, Vec3::X);
    }
    #[test]
    fn hull_expands_obstacle() {
        let t = cube().trace(
            Vec3::new(4., 0., 0.),
            Vec3::ZERO,
            Vec3::splat(-1.),
            Vec3::splat(1.),
        );
        assert!((t.fraction - 0.5).abs() < 0.02);
    }
    #[test]
    fn slide_preserves_tangent() {
        let p = cube().slide(
            Vec3::new(3., 0., 0.),
            Vec3::new(-3., 0.5, 0.),
            Vec3::ZERO,
            Vec3::ZERO,
        );
        assert!(p.x > 1. && p.x < 1.05);
        assert!((p.y - 0.5).abs() < 0.01);
    }
    #[test]
    fn duplicate_entity_outputs_survive() {
        let e = Entity {
            properties: vec![
                ("OnTrigger".into(), "a".into()),
                ("OnTrigger".into(), "b".into()),
            ],
        };
        assert_eq!(e.properties.len(), 2);
    }
}
