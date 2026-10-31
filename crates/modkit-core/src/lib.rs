//! Game-neutral data and extension boundary. All positions are Z-up, in Source units.
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
pub mod animation;
pub mod decals;
pub mod lighting;
pub mod movement;
pub mod studio_decals;

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
    /// Material (lower-case, "" for none) of each side of `brushes[i]`, side order as
    /// in the brush's planes; empty when the loader did not record them.
    #[serde(default)]
    pub brush_side_materials: Vec<Vec<String>>,
    /// Material -> $surfaceprop (lower-case) for brush-side materials, resolved at
    /// load from the owned VMTs.
    #[serde(default)]
    pub material_surfaceprops: std::collections::BTreeMap<String, String>,
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
    /// Ragdolls (.phy ragdoll section) per NPC model asset key, prepared at load.
    #[serde(default)]
    pub model_ragdolls: std::collections::BTreeMap<String, std::sync::Arc<RagdollModel>>,
    /// Studio header mass (Studio_GetMass, the NPC physics shadow's mass) per NPC model
    /// asset key.
    #[serde(default)]
    pub model_mass: std::collections::BTreeMap<String, f32>,
    /// Studio body parts (submodel count, body value stride) per lowercase model path,
    /// for models whose bodygroups change at runtime (SetBodygroup); their body variants
    /// are preloaded in `model_assets`.
    #[serde(default)]
    pub model_body_parts: std::collections::BTreeMap<String, Vec<(usize, i32)>>,
    /// prop_data health per model asset key (CPropData::ParsePropData: the MDL's
    /// prop_data block over its propdata.txt base chain); 0 = unbreakable.
    #[serde(default)]
    pub model_prop_health: std::collections::BTreeMap<String, f32>,
    /// Gibs (.phy `break` blocks) per breakable model asset key; each gib model's
    /// physics is in `model_physics`/`model_collision` under its skin-0 key.
    #[serde(default)]
    pub model_breaks: std::collections::BTreeMap<String, Vec<BreakModel>>,
    /// The resolved prop_data block per breakable model asset key (damage modifiers,
    /// damage table, propdata chunk sets).
    #[serde(default)]
    pub model_prop_data: std::collections::BTreeMap<String, PropData>,
    /// scripts/propdata.txt "BreakableModels": chunk set name -> chunk models (smallest
    /// first), each model's physics under its skin-0 key when a map prop can use it.
    #[serde(default)]
    pub breakable_chunks: Vec<(String, Vec<String>)>,
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
    /// The collision model's precomputed IVP mass center (Source units) and principal
    /// rotation inertia per kilogram (m^2, Source axes), when the solid was read.
    #[serde(default)]
    pub compact_inertia: Option<(Vec3, Vec3)>,
    /// The collision file's stored drag fractions (VPHY header, stored order), when read.
    #[serde(default)]
    pub drag_fractions: Option<Vec3>,
}
/// A prop's prop_data after CPropData::ParsePropFromKV: the propdata.txt "base" chain
/// first, then the model's own block. Defaults follow CBreakableProp (modifiers 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropData {
    pub health: f32,
    pub dmg_bullets: f32,
    pub dmg_club: f32,
    pub dmg_explosive: f32,
    /// Custom physics impact damage table ("glass"); None = the NPC table.
    pub damage_table: Option<String>,
    /// Propdata chunk set ("WoodChunks") used when the model has no .phy gibs.
    pub breakable_model: Option<String>,
    pub breakable_count: i32,
    pub breakable_skin: i32,
    /// explosive_damage / explosive_radius when the prop_data section sets them (they
    /// override the entity's ExplodeDamage / ExplodeRadius keyvalues).
    pub explosive_damage: Option<f32>,
    pub explosive_radius: Option<f32>,
    /// fire_interactions (PROPINTER_FIRE_FLAMMABLE, _EXPLOSIVE_RESIST,
    /// _IGNITE_HALFHEALTH): the model's sections beside prop_data, or a base's own.
    #[serde(default)]
    pub flammable: bool,
    #[serde(default)]
    pub explosive_resist: bool,
    #[serde(default)]
    pub ignite_halfhealth: bool,
    /// physgun_interactions onbreak explode_fire (PROPINTER_PHYSGUN_BREAK_EXPLODE: the gas
    /// can's PropaneTank.Burst instead of the explosion sound).
    #[serde(default)]
    pub break_explode: bool,
    /// physgun_interactions onlaunch spin_zaxis (PROPINTER_PHYSGUN_LAUNCH_SPIN_Z: the
    /// sawblade): a slicing object (FVPHYSICS_DMG_SLICE), held at its preferred carry
    /// angles, launched spinning about its z axis.
    #[serde(default)]
    pub spin_z: bool,
    /// physgun_interactions onworldimpact stick (PROPINTER_PHYSGUN_WORLD_STICK): the
    /// first world hit after a gravity gun launch can embed it (the sawblade).
    #[serde(default)]
    pub world_stick: bool,
    /// The model's physgun_interactions preferred_carryangles (pitch yaw roll), read by
    /// CPhysicsProp::GetPropDataAngles.
    #[serde(default)]
    pub carry_angles: Option<[f32; 3]>,
}
impl Default for PropData {
    fn default() -> Self {
        Self {
            health: 0.,
            dmg_bullets: 1.,
            dmg_club: 1.,
            dmg_explosive: 1.,
            damage_table: None,
            breakable_model: None,
            breakable_count: 0,
            breakable_skin: 0,
            explosive_damage: None,
            explosive_radius: None,
            flammable: false,
            explosive_resist: false,
            ignite_halfhealth: false,
            break_explode: false,
            spin_z: false,
            world_stick: false,
            carry_angles: None,
        }
    }
}
/// One gib of a breakable model: a .phy `break` block (CBreakParser defaults: health 1,
/// fadetime 20 s, not a ragdoll, no offset).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BreakModel {
    /// Normalized "models/....mdl" path (FixupModelName).
    pub model: String,
    pub ragdoll: bool,
    pub offset: Vec3,
    pub health: f32,
    pub fade_time: f32,
    /// Some(true): COLLISION_GROUP_DEBRIS ("debris" > 0, or a "fadetime" without an
    /// earlier "debris"); Some(false): INTERACTIVE; None: the breaking prop's group.
    pub debris: Option<bool>,
    pub motion_disabled: bool,
    /// "burst": outward speed (units/s) away from the parent origin; absent = the
    /// breaking prop's default (CBreakableProp::Break: 100).
    #[serde(default)]
    pub burst: Option<f32>,
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
            compact_inertia: None,
            drag_fractions: None,
        }
    }
}
/// surfacedata_t fields that gameplay uses (vphysics_interface.h), values from
/// scripts/surfaceproperties*.txt after "base" inheritance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// surfacegameprops_t jumpFactor / maxSpeedFactor (player movement on this
    /// ground; 1 unless a script lowers them, e.g. slipperyslime's jumpfactor).
    #[serde(default = "one")]
    pub jump_factor: f32,
    #[serde(default = "one")]
    pub max_speed_factor: f32,
    /// surfacephysicsparams_t thickness (inches; sheet materials) and dampening
    /// (contact drag). Parsed for reference: vphysics' use of them is not in the SDK.
    #[serde(default)]
    pub thickness: f32,
    #[serde(default)]
    pub dampening: f32,
}
fn one() -> f32 {
    1.
}
impl Default for SurfaceMaterial {
    fn default() -> Self {
        Self {
            friction: 0.,
            elasticity: 0.,
            density: 0.,
            hardness_factor: 0.,
            hard_threshold: 0.,
            hard_velocity_threshold: 0.,
            roughness_factor: 0.,
            rough_threshold: 0.,
            impact_hard: None,
            impact_soft: None,
            scrape_smooth: None,
            scrape_rough: None,
            break_sound: None,
            game_material: None,
            jump_factor: 1.,
            max_speed_factor: 1.,
            thickness: 0.,
            dampening: 0.,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConvexPiece {
    pub vertices: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
}
/// A model's ragdoll (ragdoll_shared.cpp RagdollCreateObjects): one body per .phy solid,
/// its convex pieces in its bone's space, joined by ragdoll constraints.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RagdollModel {
    pub bodies: Vec<RagdollBody>,
    pub joints: Vec<RagdollJoint>,
    /// CRagdollCollisionRules: false ("selfcollisions" "0") lets only `collision_pairs`
    /// (body indices) collide with each other.
    pub self_collisions: bool,
    pub collision_pairs: Vec<(usize, usize)>,
    /// CRagdollAnimatedFriction ("animatedfriction"): min and max joint friction, then the
    /// ramp-in, ramp-out and hold times (C_ClientRagdoll::HandleAnimatedFriction).
    pub animated_friction: Option<[f32; 5]>,
}
/// One ragdoll solid: the studio bone it simulates and its authored parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RagdollBody {
    pub bone: usize,
    /// Convex pieces in bone space (Source units and axes).
    pub pieces: Vec<ConvexPiece>,
    /// Kilograms.
    pub mass: f32,
    pub inertia: f32,
    pub damping: f32,
    pub rotdamping: f32,
    pub surfaceprop: String,
    /// The solid's IVP mass center (bone space) and rotation inertia per kilogram.
    pub compact_inertia: Option<(Vec3, Vec3)>,
}
/// constraint_ragdollparams_t: the child body's rotation limits about its own x, y and
/// z axes (degrees) relative to the parent, with the friction torque per axis.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RagdollJoint {
    pub parent: usize,
    pub child: usize,
    pub limits: [(f32, f32); 3],
    pub friction: [f32; 3],
    /// Studio_CalcBoneToBoneTransform at the reference pose: the child bone's frame in
    /// the parent bone's space (translation, rotation).
    pub child_in_parent: (Vec3, glam::Quat),
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
    /// The studio body value (m_nBody: each body part's submodel, SetBodygroup).
    #[serde(default)]
    pub body: i32,
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
        if self.body == 0 {
            format!("{}#{}", self.model.to_lowercase(), self.skin)
        } else {
            format!("{}#{}@{}", self.model.to_lowercase(), self.skin, self.body)
        }
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
