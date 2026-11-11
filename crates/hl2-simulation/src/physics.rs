//! Rapier collision/rigid-body adapter. This is not Valve's proprietary VPhysics solver.
use crate::npc_probe::{ActorHull, Hull, HullTrace, NpcCollisionWorld, Query, CONTENTS_MONSTER};
use glam::{Mat4, Quat, Vec3};
use modkit_core::{movement::CollisionWorld, trace_brushes, Brush, Surface, Trace, World};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
#[path = "physics_npc_trace.rs"]
mod npc_trace;
#[path = "player_convex.rs"]
mod player_convex;
mod rays;
#[derive(Clone, Copy, Debug, Default)]
pub struct RayHit {
    pub entity: usize,
    pub position: Vec3,
    pub normal: Vec3,
    /// The studio hitbox's hitgroup when an NPC hitbox was hit (HITGROUP_GENERIC 0
    /// otherwise).
    pub hitgroup: i32,
    /// The hit studio hitbox's bone (the trace's physicsbone source), when one was hit.
    pub bone: Option<usize>,
}
/// An NPC's studio hitboxes (hitbox set 0) as trace colliders, posed from the
/// animated skeleton every tick (DESIGN 14b).
#[derive(Clone, Debug)]
struct StudioHitboxSet {
    /// (collider, bone, box center in bone space, bind-pose bone model matrix)
    boxes: Vec<(ColliderHandle, usize, Vec3, Mat4)>,
    asset_key: String,
}
#[derive(Clone, Copy)]
pub enum ProjectileHull {
    Box(Vec3),
    Sphere(f32),
}
#[derive(Clone, Copy, Debug)]
pub struct ProjectileHit {
    pub entity: usize,
    /// Projectile center at impact; the contact point is not its origin.
    pub position: Vec3,
    pub normal: Vec3,
    pub fraction: f32,
}
const SCALE: f32 = 1. / 39.37;
/// k_flMaxVelocity / k_flMaxAngularVelocity (SDK public/vphysics/performance.h).
pub(crate) const MAX_VELOCITY: f32 = 2000.;
const MAX_ANGULAR_VELOCITY: f32 = 360. * 10.;
// Retail CGameMovement::PlayerSolidMask(true): NPC-only clip volumes do not
// block players. Keep their colliders available for other query categories.
const PLAYER_BRUSH_MASK: u32 = 0x1400b;
/// VPhysics projectiles (thrown frags): COLLISION_GROUP_WEAPON-like bodies that touch
/// the world and props but not characters, and that traces/player movement ignore.
const PROJECTILE_GROUP: Group = Group::GROUP_2;
/// Colliders of npc_* model instances.
const NPC_GROUP: Group = Group::GROUP_3;
/// The player's VPhysics shadow (CBasePlayer::SetupVPhysicsShadow): it only pushes and
/// rests on movable props; traces and player movement ignore it.
const SHADOW_GROUP: Group = Group::GROUP_4;
/// NPC movement hulls (SOLID_BBOX: an axis-aligned box of the class hull, e.g.
/// HULL_HUMAN): player movement and physics props collide with them; traces use the
/// model-shaped colliders instead.
const HULL_GROUP: Group = Group::GROUP_5;
/// MASK_SOLID (CONTENTS_SOLID | MOVEABLE | WINDOW | MONSTER | GRATE): what physics
/// objects collide with (CBaseEntity::PhysicsSolidMaskForEntity).
const MASK_SOLID: u32 = 0x1 | 0x4000 | 0x2 | 0x2000000 | 0x8;
/// MASK_SHOT (CONTENTS_SOLID | MOVEABLE | MONSTER | WINDOW | DEBRIS | HITBOX): bullets
/// pass CONTENTS_GRATE brushes (chain-link fences) that block players and props.
pub const MASK_SHOT: u32 = 0x1 | 0x4000 | 0x2000000 | 0x2 | 0x4000000 | 0x4000_0000;
/// prop_physics with SF_PHYSPROP_DEBRIS (4): COLLISION_GROUP_DEBRIS collides only with
/// COLLISION_GROUP_NONE (CGameRules::ShouldCollide): the world and ordinary props, not
/// the player (movement or shadow), NPCs, thrown frags or other debris. Traces (bullets,
/// +USE, the gravity gun) still hit it.
const DEBRIS_GROUP: Group = Group::GROUP_6;
const DEBRIS_GROUPS: InteractionGroups = InteractionGroups::new(
    DEBRIS_GROUP,
    Group::ALL
        .difference(DEBRIS_GROUP)
        .difference(SHADOW_GROUP)
        .difference(HULL_GROUP)
        .difference(NPC_GROUP)
        .difference(PROJECTILE_GROUP)
        .difference(INTERACTIVE_DEBRIS_GROUP),
);
/// COLLISION_GROUP_INTERACTIVE_DEBRIS: a debris prop the player picked up, punted or
/// threw (CPlayerPickupController::Init, Physgun_OnPhysGunPickup) collides with
/// everything but debris, other interactive debris and the player (movement and
/// shadow), so a thrown can hits NPCs; it converts back to debris when its body falls
/// asleep (CBaseEntity::VPhysicsUpdate).
const INTERACTIVE_DEBRIS_GROUP: Group = Group::GROUP_7;
const INTERACTIVE_DEBRIS_GROUPS: InteractionGroups = InteractionGroups::new(
    INTERACTIVE_DEBRIS_GROUP,
    Group::ALL
        .difference(DEBRIS_GROUP)
        .difference(INTERACTIVE_DEBRIS_GROUP)
        .difference(SHADOW_GROUP),
);
/// Client-side ragdolls (C_BaseAnimating::BecomeRagdollOnClient): the world and props
/// block them but props are not pushed (dominance), and the player, NPCs, debris and
/// every server trace ignore them. Self collisions follow the model's collision rules
/// (contact pair filter).
const RAGDOLL_GROUP: Group = Group::GROUP_8;
/// Rest thresholds for client ragdoll solids (see spawn_ragdoll): angular speed (rad/s)
/// and seconds every solid must stay under them (linear: Rapier's 0.4 m/s, ~16 units/s).
/// Tuned so landed hall corpses rest within about a second; IVP's own deactivation
/// constants are not read (retail vphysics.dll).
const RAGDOLL_SLEEP_ANGULAR: f32 = 2.;
const RAGDOLL_SLEEP_TIME: f32 = 0.5;
const RAGDOLL_GROUPS: InteractionGroups = InteractionGroups::new(
    RAGDOLL_GROUP,
    Group::ALL
        .difference(SHADOW_GROUP)
        .difference(HULL_GROUP)
        .difference(NPC_GROUP)
        .difference(PROJECTILE_GROUP)
        .difference(DEBRIS_GROUP)
        .difference(INTERACTIVE_DEBRIS_GROUP),
);
/// Dropped weapons (CBaseCombatWeapon::FallInit, COLLISION_GROUP_WEAPON):
/// CGameRules::ShouldCollide keeps the player (movement and shadow), NPCs (models and
/// hulls) and debris off them, so the player walks over them; the world, props, thrown
/// frags and other weapons still collide, and bullets, +USE and the gravity gun still hit
/// them (COLLISION_GROUP_NONE traces).
const WEAPON_GROUP: Group = Group::GROUP_9;
const WEAPON_GROUPS: InteractionGroups = InteractionGroups::new(
    WEAPON_GROUP,
    Group::ALL
        .difference(NPC_GROUP)
        .difference(HULL_GROUP)
        .difference(SHADOW_GROUP)
        .difference(DEBRIS_GROUP)
        .difference(RAGDOLL_GROUP),
);
const QUERY_GROUPS: InteractionGroups = InteractionGroups::new(
    Group::ALL,
    Group::ALL
        .difference(PROJECTILE_GROUP)
        .difference(SHADOW_GROUP)
        .difference(HULL_GROUP)
        .difference(RAGDOLL_GROUP),
);
/// Player movement: everything solid to the player, NPC hull boxes included (not
/// debris props).
const PLAYER_GROUPS: InteractionGroups = InteractionGroups::new(
    Group::ALL,
    Group::ALL
        .difference(PROJECTILE_GROUP)
        .difference(SHADOW_GROUP)
        .difference(DEBRIS_GROUP)
        .difference(INTERACTIVE_DEBRIS_GROUP)
        .difference(RAGDOLL_GROUP)
        .difference(WEAPON_GROUP),
);
/// The SDK hull (mins, maxs) of npc_* classes (SetHullType), for those done so far.
pub fn npc_hull(class: &str) -> Option<(Vec3, Vec3)> {
    crate::ai_hull::body_box(class)
}
/// Traces with COLLISION_GROUP_NONE (FindUseEntity, the gravity gun's FindObject and
/// punt): they also hit COLLISION_GROUP_WEAPON bodies such as a thrown frag.
const PICK_GROUPS: InteractionGroups = InteractionGroups::new(
    Group::ALL,
    Group::ALL
        .difference(SHADOW_GROUP)
        .difference(HULL_GROUP)
        .difference(RAGDOLL_GROUP),
);
/// Entity ids of VPhysics projectile bodies (thrown frags), above every map entity
/// index, so the entity-keyed +USE and gravity gun holds can take them.
pub const PROJECTILE_ENTITY_BASE: usize = 1 << 40;
pub fn projectile_entity(projectile: u64) -> usize {
    PROJECTILE_ENTITY_BASE + projectile as usize
}
pub fn is_projectile_entity(id: usize) -> bool {
    (PROJECTILE_ENTITY_BASE..usize::MAX).contains(&id)
}
/// SetupVPhysicsShadow: mass 85, no rotation, no drag; SetPushMassLimit(350).
pub const PLAYER_SHADOW_MASS: f32 = 85.;
pub const PLAYER_PUSH_MASS_LIMIT: f32 = 350.;
/// VPhysics keeps objects apart by IVP's collision margin: the maps' authored rest
/// poses put .phy hull bottoms 0.497 (PlasticCrate01a) and 0.500 (a lying
/// garbage_glassbottle003a) units above the floor, while the stored hulls fit the render
/// meshes, so without it props sank half a unit into floors (owner report). Applied as
/// a Rapier contact skin on dynamic prop colliders (contacts only; traces unchanged).
pub const VPHYSICS_CONTACT_MARGIN: f32 = 0.5;
/// Contact depth (units) past which a CCD-gated prop keeps continuous collision.
const EMBEDDED_DEPTH: f32 = 1.;
/// phys_lengthconstraint as Rapier rope joints (off: see `add_constraints`).
const ROPE_CONSTRAINTS: bool = false;
/// SetPushSpeedLimit(50): the shadow's speed into a pushable object's contact normal.
pub const PLAYER_PUSH_SPEED_LIMIT: f32 = 50.;
/// The smallest extent of a body's pieces together, in its own frame (meters).
fn body_thickness(body: &RigidBody, colliders: &ColliderSet) -> f32 {
    let mut bounds: Option<rapier3d::parry::bounding_volume::Aabb> = None;
    for c in body.colliders().iter().filter_map(|c| colliders.get(*c)) {
        let local = c
            .position_wrt_parent()
            .copied()
            .unwrap_or_else(Isometry::identity);
        let aabb = c.shape().compute_aabb(&local);
        bounds = Some(bounds.map_or(aabb, |b| b.merged(&aabb)));
    }
    bounds.map_or(f32::INFINITY, |b| b.extents().min())
}
fn groups_filter(groups: InteractionGroups) -> QueryFilter<'static> {
    static ENABLED: fn(ColliderHandle, &Collider) -> bool = |_, c| c.is_enabled();
    QueryFilter::default()
        .groups(groups)
        .exclude_sensors()
        .predicate(&ENABLED)
}
/// The surfaceprop a model instance's colliders report: its authored VPhysics solid's, or a
/// static prop's .phy surfaceprop (World::model_surfaceprops).
fn solid_surfaceprop(
    world: &World,
    instance: &modkit_core::ModelInstance,
    authored: Option<&modkit_core::PhysicsSolid>,
) -> Option<String> {
    authored
        .map(|solid| &solid.surfaceprop)
        .or_else(|| world.model_surfaceprops.get(&instance.asset_key()))
        .cloned()
}
fn vector(p: Vec3) -> Vector<Real> {
    vector![p.x * SCALE, p.y * SCALE, p.z * SCALE]
}
/// A rigid Mat4 (Source units) as a Rapier pose.
fn mat4_pose(m: Mat4) -> Isometry<Real> {
    let (_, rotation, translation) = m.to_scale_rotation_translation();
    pose(translation, rotation.normalize())
}
fn pose(origin: Vec3, rotation: Quat) -> Isometry<Real> {
    Isometry::from_parts(
        Translation::from(vector(origin)),
        Rotation::from_quaternion(rapier3d::na::Quaternion::new(
            rotation.w, rotation.x, rotation.y, rotation.z,
        )),
    )
}
pub fn angles(a: Vec3) -> Quat {
    Quat::from_rotation_z(a.y.to_radians())
        * Quat::from_rotation_y(a.x.to_radians())
        * Quat::from_rotation_x(a.z.to_radians())
}
fn brush_shape(brush: &Brush) -> Option<SharedShape> {
    let p = &brush.planes;
    let mut points = Vec::new();
    for i in 0..p.len() {
        for j in i + 1..p.len() {
            for k in j + 1..p.len() {
                let det = p[i].normal.dot(p[j].normal.cross(p[k].normal));
                if det.abs() < 1e-5 {
                    continue;
                }
                let point = (p[j].normal.cross(p[k].normal) * p[i].distance
                    + p[k].normal.cross(p[i].normal) * p[j].distance
                    + p[i].normal.cross(p[j].normal) * p[k].distance)
                    / det;
                if point.is_finite()
                    && p.iter().all(|v| v.normal.dot(point) <= v.distance + 0.05)
                    && !points
                        .iter()
                        .any(|v: &Vec3| v.distance_squared(point) < 0.0001)
                {
                    points.push(point);
                }
            }
        }
    }
    SharedShape::convex_hull(
        &points
            .into_iter()
            .map(|p| Point::from(vector(p)))
            .collect::<Vec<_>>(),
    )
}
fn mesh_shape(
    surfaces: &[Surface],
    convex: bool,
    matrices: Option<&[Mat4]>,
) -> Option<SharedShape> {
    mesh_shape_with(
        surfaces,
        convex,
        matrices,
        TriMeshFlags::MERGE_DUPLICATE_VERTICES | TriMeshFlags::FIX_INTERNAL_EDGES,
    )
}
fn mesh_shape_with(
    surfaces: &[Surface],
    convex: bool,
    matrices: Option<&[Mat4]>,
    flags: TriMeshFlags,
) -> Option<SharedShape> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for s in surfaces {
        let base = vertices.len() as u32;
        vertices.extend(s.vertices.iter().map(|v| {
            let position = matrices.zip(v.skin.as_ref()).map_or(v.position, |(m, w)| {
                modkit_core::animation::skin(v.position, w, m)
            });
            Point::from(vector(position))
        }));
        indices.extend(
            s.indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|i| [i[0] + base, i[1] + base, i[2] + base]),
        );
    }
    if convex {
        SharedShape::convex_hull(&vertices)
    } else {
        SharedShape::trimesh_with_flags(vertices, indices, flags).ok()
    }
}
/// The .phy solid after CPhysicsProp::CreateVPhysics' keyvalue scaling: mass x
/// massScale (when > 0), inertia x inertiaScale (when > 0, at least 0.5).
fn authored_solid(
    solid: &modkit_core::PhysicsSolid,
    keyvalue: impl Fn(&str) -> Option<f32>,
) -> modkit_core::PhysicsSolid {
    let mut solid = solid.clone();
    if let Some(scale) = keyvalue("massScale").filter(|s| *s > 0.) {
        solid.mass *= scale;
    }
    if let Some(scale) = keyvalue("inertiaScale").filter(|s| *s > 0.) {
        solid.inertia = (solid.inertia * scale).max(0.5);
    }
    // VPhysics rejects non-positive masses; keep the body simulable.
    solid.mass = solid.mass.max(0.001);
    solid
}
fn surface_material<'a>(world: &'a World, name: &str) -> Option<&'a modkit_core::SurfaceMaterial> {
    world
        .surface_materials
        .get(name)
        .or_else(|| world.surface_materials.get("default"))
}
/// Rapier restitution from a surfaceprop's elasticity, clamped to 0..1: the shipped
/// scripts carry out-of-range values (metal_bouncy 1000, quicksand 2) whose IVP
/// meaning is not in the SDK, and Rapier restitution above 1 adds energy per bounce.
fn restitution(m: &modkit_core::SurfaceMaterial) -> f32 {
    m.elasticity.clamp(0., 1.)
}
/// Side materials of world brush `index`, or None when every side resolves to the
/// "default" surfaceprop (or nothing is recorded).
fn brush_sides(world: &World, index: usize, brush: &Brush) -> Option<Vec<SideMaterial>> {
    let names = world.brush_side_materials.get(index)?;
    let default = world.surface_materials.get("default")?;
    let mut any = false;
    let sides = brush
        .planes
        .iter()
        .zip(names)
        .map(|(plane, material)| {
            let m = world
                .material_surfaceprops
                .get(material)
                .and_then(|prop| world.surface_materials.get(prop))
                .unwrap_or(default);
            any |= m.friction != default.friction || m.elasticity != default.elasticity;
            SideMaterial {
                normal: plane.normal,
                friction: m.friction,
                restitution: restitution(m),
            }
        })
        .collect();
    any.then_some(sides)
}
fn world_material(world: &World) -> impl Fn(ColliderBuilder) -> ColliderBuilder + '_ {
    let material = world.surface_materials.get("default");
    move |builder| match material {
        Some(m) => builder
            .friction(m.friction)
            .restitution(restitution(m))
            .friction_combine_rule(CoefficientCombineRule::Multiply)
            .restitution_combine_rule(CoefficientCombineRule::Multiply),
        None => builder.friction(0.8),
    }
}
/// The body's mass properties from the authored total mass: uniform density over
/// the convex pieces, then the whole inertia tensor times the solid's "inertia"
/// multiplier (VPhysics scales the object's tensor, parallel-axis terms included).
/// A hull that is a prism of at least 10 equal-radius sides about the model z axis (the
/// frag's w_grenade.phy: 12 sides, radius 1.808) as a smooth Rapier cylinder. Native IVP
/// rolls such a hull along the floor with little loss (lob/roll samples: ~150 u/s held for
/// most of a second); Rapier's faceted hull loses energy on every edge impact and stops
/// within ~1 s, so the round body stands in for IVP's rolling contact.
fn prism_cylinder(vertices: &[Vec3]) -> Option<SharedShape> {
    let rim: Vec<_> = vertices
        .iter()
        .filter(|v| v.truncate().length() > 0.5)
        .collect();
    if rim.len() < 20 {
        return None;
    }
    let radius = rim.iter().map(|v| v.truncate().length()).sum::<f32>() / rim.len() as f32;
    if rim
        .iter()
        .any(|v| (v.truncate().length() - radius).abs() > 0.02 * radius)
    {
        return None;
    }
    let (low, high) = rim
        .iter()
        .fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(v.z), h.max(v.z)));
    if high - low < radius {
        return None;
    }
    // Rapier cylinders run along their local y; turn that onto the model z axis.
    Some(SharedShape::compound(vec![(
        Isometry::new(
            vector![0., 0., (low + high) * 0.5 * SCALE],
            vector![std::f32::consts::FRAC_PI_2, 0., 0.],
        ),
        SharedShape::cylinder((high - low) * 0.5 * SCALE, radius * SCALE),
    )]))
}
fn authored_mass_properties(shapes: &[SharedShape], mass: f32, inertia: f32) -> MassProperties {
    let unit = shapes
        .iter()
        .map(|s| s.mass_properties(1.))
        .fold(MassProperties::default(), |sum, m| sum + m);
    let composite = if unit.mass() > 1e-9 {
        let mut scaled = unit;
        scaled.set_mass(mass, true);
        scaled
    } else {
        MassProperties::new(Point::origin(), mass, Vector::repeat(mass * 0.01))
    };
    MassProperties::with_principal_inertia_frame(
        composite.local_com,
        mass,
        composite.principal_inertia() * inertia,
        composite.principal_inertia_local_frame,
    )
}
struct PropShapes {
    shapes: Vec<SharedShape>,
    native: bool,
    native_failed: bool,
}
/// One VPhysics-style collision event (gamevcollisionevent_t subset).
#[derive(Clone, Debug)]
pub struct PhysicsCollision {
    pub colliders: [ColliderHandle; 2],
    /// Entity ids; None for world geometry and free bodies (thrown frags).
    pub entities: [Option<usize>; 2],
    /// Surfaceprop of authored bodies; None when the collider carries none (world).
    pub surfaceprops: [Option<String>; 2],
    pub position: Vec3,
    /// Each side's object origin (pObject->GetPosition), the contact for fixed geometry.
    pub origins: [Vec3; 2],
    /// Contact normal (from the first collider toward the second).
    pub normal: Vec3,
    /// collisionSpeed: relative normal speed before the impact, units/s.
    pub speed: f32,
    /// deltaCollisionTime: seconds since this pair last started touching.
    pub delta_time: f32,
    /// Each side's body velocity before and after the step (units/s; zero for fixed or
    /// kinematic colliders) and mass (0 when not dynamic): gamevcollisionevent_t
    /// preVelocity/postVelocity and GetMass for impact damage.
    pub pre_velocities: [Vec3; 2],
    pub post_velocities: [Vec3; 2],
    pub masses: [f32; 2],
    /// Each side's angular speed before the step (deg/s) and DotProductAbs(GetInertia,
    /// local preAngularVelocity): the principal inertia (kg m^2: the retail
    /// vphysics.dll applies no length conversion to it) times the local angular
    /// velocity in deg/s; zero for non-dynamic sides.
    pub angular: [(f32, f32); 2],
}
/// IVP reports a step's impacts in time order, so a fast prop's first collision is the
/// face it meets head-on. Rapier finds them together, one event per collider pair, in a
/// pair order that changes between runs (a launched sawblade meeting a pillar made of
/// two stacked brushes logged either the face, dot 1.0, or the lower brush's top edge,
/// dot 0.1). Hardest impacts first (collisionSpeed, stable) stands in for the time
/// order, so first-collision rules (HandleInteractionStick, the launch damage bonus) see
/// the head-on impact.
pub fn impact_order(collisions: &mut [PhysicsCollision]) {
    collisions.sort_by(|a, b| b.speed.total_cmp(&a.speed));
}
#[derive(Clone, Copy, Debug)]
pub struct BodyState {
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub local_angular_degrees: Vec3,
}
/// Linear velocity, angular velocity and center of mass before a step.
type PreStep = (Vector<Real>, Vector<Real>, Point<Real>);
/// One world brush side's material: outward plane normal, friction, restitution.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SideMaterial {
    pub normal: Vec3,
    pub friction: f32,
    pub restitution: f32,
}
/// Per-side world materials (VBSP sets each world collision triangle's material from
/// its brush side; Rapier has one material per collider, so the solver contacts of a
/// brush collider take the side facing the other body).
#[derive(Default)]
pub struct WorldSideMaterials {
    pub sides: HashMap<ColliderHandle, Vec<SideMaterial>>,
    /// The player shadow's colliders and the bodies it must not touch this step (the
    /// held object).
    pub shadow: Vec<ColliderHandle>,
    pub shadow_excluded: Vec<RigidBodyHandle>,
    /// Model-shaped NPC colliders of hull-box NPCs: trace hitboxes only. Rapier enables
    /// a pair when either collider's active collision types allow it, so a dynamic prop
    /// still generated and solved contacts against every animated hitbox (owner report:
    /// a pallet sliding off a plaza cop dropped 46 -> 17 fps); the pair filter drops them.
    pub hitboxes: std::collections::HashSet<ColliderHandle>,
    /// Entity pairs that never collide (logic_collision_pair with startdisabled 1:
    /// PhysDisableEntityCollisions), smaller id first.
    pub disabled_pairs: std::collections::HashSet<(usize, usize)>,
    /// World brushes that are only player/NPC clips (no MASK_SOLID contents). VBSP
    /// builds them as separate world solids whose contents CCollisionEvent::
    /// ShouldCollide tests against PhysicsSolidMaskForEntity: physics objects (MASK_SOLID)
    /// pass through them. Player movement and traces keep their own clip handling.
    pub clip_brushes: std::collections::HashSet<ColliderHandle>,
    /// Bodies whose world constraint has SF_CONSTRAINT_DISABLE_COLLISION: the
    /// constraint disables the entity pair with the world entity
    /// (PhysDisableEntityCollisions on g_PhysWorldObject), which also owns the static
    /// props, so these bodies ignore every static collider without an entity.
    pub world_disabled_bodies: std::collections::HashSet<RigidBodyHandle>,
    /// Ragdoll bodies (ragdoll serial) and the body pairs of one ragdoll that collide
    /// (CRagdollCollisionRules), both orders.
    pub ragdoll_bodies: HashMap<RigidBodyHandle, u64>,
    pub ragdoll_pairs: std::collections::HashSet<(RigidBodyHandle, RigidBodyHandle)>,
}
impl WorldSideMaterials {
    /// The side whose outward normal best matches `outward` (from the brush toward
    /// the other body).
    pub fn side(&self, collider: ColliderHandle, outward: Vec3) -> Option<SideMaterial> {
        self.sides
            .get(&collider)?
            .iter()
            .copied()
            .max_by(|a, b| a.normal.dot(outward).total_cmp(&b.normal.dot(outward)))
    }
}
impl PhysicsHooks for WorldSideMaterials {
    /// The player controller's contact filter: the shadow touches only movable
    /// (dynamic) bodies within the push mass limit, never world geometry, doors or
    /// the object the player holds.
    fn filter_contact_pair(&self, context: &PairFilterContext) -> Option<SolverFlags> {
        if self.hitboxes.contains(&context.collider1) || self.hitboxes.contains(&context.collider2)
        {
            return None;
        }
        if let (Some(a), Some(b)) = (context.rigid_body1, context.rigid_body2) {
            if let (Some(ra), Some(rb)) = (self.ragdoll_bodies.get(&a), self.ragdoll_bodies.get(&b))
            {
                if ra == rb && !self.ragdoll_pairs.contains(&(a, b)) {
                    return None;
                }
            }
        }
        if self.clip_brushes.contains(&context.collider1)
            || self.clip_brushes.contains(&context.collider2)
        {
            return None;
        }
        if !self.world_disabled_bodies.is_empty() {
            let world = |c| {
                context
                    .colliders
                    .get(c)
                    .is_some_and(|c| c.parent().is_none() && c.user_data == 0)
            };
            let disabled = |b: Option<RigidBodyHandle>| {
                b.is_some_and(|b| self.world_disabled_bodies.contains(&b))
            };
            if (disabled(context.rigid_body1) && world(context.collider2))
                || (disabled(context.rigid_body2) && world(context.collider1))
            {
                return None;
            }
        }
        if !self.disabled_pairs.is_empty() {
            let entity = |c| {
                context
                    .colliders
                    .get(c)
                    .and_then(|c| (c.user_data as usize).checked_sub(1))
            };
            if let (Some(a), Some(b)) = (entity(context.collider1), entity(context.collider2)) {
                if self.disabled_pairs.contains(&(a.min(b), a.max(b))) {
                    return None;
                }
            }
        }
        let other = if self.shadow.contains(&context.collider1) {
            context.rigid_body2
        } else if self.shadow.contains(&context.collider2) {
            context.rigid_body1
        } else {
            return Some(SolverFlags::COMPUTE_IMPULSES);
        };
        let (handle, body) = other.and_then(|h| context.bodies.get(h).map(|b| (h, b)))?;
        (body.is_dynamic()
            && body.mass() <= PLAYER_PUSH_MASS_LIMIT
            && !self.shadow_excluded.contains(&handle))
        .then_some(SolverFlags::COMPUTE_IMPULSES)
    }
    fn modify_solver_contacts(&self, context: &mut ContactModificationContext) {
        // VPhysics keeps one collision margin per contact pair, but Rapier adds both
        // colliders' skins: two props would rest a full margin further apart than a
        // prop on the floor (owner report: stacked crates sat higher than native).
        let skin = |c| context.colliders.get(c).map_or(0., |c| c.contact_skin());
        let shared = skin(context.collider1).min(skin(context.collider2));
        if shared > 0. {
            for contact in context.solver_contacts.iter_mut() {
                contact.dist += shared;
            }
            return;
        }
        let n = Vec3::new(context.normal.x, context.normal.y, context.normal.z);
        let (side, other) = if let Some(side) = self.side(context.collider1, n) {
            (side, context.collider2)
        } else if let Some(side) = self.side(context.collider2, -n) {
            (side, context.collider1)
        } else {
            return;
        };
        // Multiply combine, as for every other surfaceprop pair.
        let (friction, restitution) = context
            .colliders
            .get(other)
            .map_or((1., 0.), |c| (c.friction(), c.restitution()));
        for contact in context.solver_contacts.iter_mut() {
            contact.friction = side.friction * friction;
            contact.restitution = side.restitution * restitution;
        }
    }
}
#[derive(Default)]
pub struct Physics {
    pub bodies: RigidBodySet,
    pub colliders: ColliderSet,
    query: QueryPipeline,
    changed_entity_colliders: Vec<ColliderHandle>,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad: BroadPhaseMultiSap,
    narrow: NarrowPhase,
    impulses: ImpulseJointSet,
    multibody: MultibodyJointSet,
    ccd: CCDSolver,
    entity_colliders: BTreeMap<usize, Vec<ColliderHandle>>,
    world_brush_contents: HashMap<ColliderHandle, u32>,
    pub world_sides: WorldSideMaterials,
    npc_collider_contents: HashMap<ColliderHandle, u32>,
    player_world_brushes: Vec<Brush>,
    player_convex_cache: Mutex<player_convex::ConvexCache>,
    pub dynamic: BTreeMap<usize, RigidBodyHandle>,
    /// Surfaceprop of each authored VPhysics prop (entity id -> lowercase name).
    pub prop_materials: BTreeMap<usize, String>,
    /// Surfaceprop per collider of authored bodies (props and free bodies).
    collider_materials: HashMap<ColliderHandle, String>,
    /// This step's new contacts (see collect_collisions).
    pub collisions: Vec<PhysicsCollision>,
    /// Last step's new contacts with post velocities taken after this step, when the
    /// contact has been resolved (gamevcollisionevent_t postVelocity), for impact damage.
    pub resolved_collisions: Vec<PhysicsCollision>,
    touching: std::collections::HashSet<(ColliderHandle, ColliderHandle)>,
    pair_times: HashMap<(ColliderHandle, ColliderHandle), f64>,
    /// Simulated seconds (sum of tick dt).
    pub time: f64,
    /// VPhysics air density for the drag controller (drag.rs; 0 disables drag).
    pub air_density: f32,
    /// Drag geometry per dynamic body, computed once from its colliders.
    drag_geometry: HashMap<RigidBodyHandle, Option<crate::drag::DragGeometry>>,
    /// Debris bodies the player touched (COLLISION_GROUP_INTERACTIVE_DEBRIS).
    interactive_debris: std::collections::HashSet<RigidBodyHandle>,
    /// Bodies with drag disabled (EnableDrag(false): a held object's controlled body).
    pub drag_disabled: std::collections::HashSet<RigidBodyHandle>,
    /// Bodies whose angular drag coefficient is 0 (a launched spin_zaxis prop).
    pub angular_drag_disabled: std::collections::HashSet<RigidBodyHandle>,
    /// VPhysics constraints created at load, and those that could not be.
    pub constraints: usize,
    pub constraint_errors: Vec<String>,
    /// Prop bodies whose continuous collision runs only while their linear travel per
    /// step exceeds the body's own thickness (cached, 0 = not computed yet), so a body
    /// never skips past anything thicker than itself. Rapier's own test uses the
    /// thinnest piece (a pallet slat) and counts rotation too: its time-of-impact
    /// search against the world cost 17-40 ms per step while a pallet rocked on and
    /// tipped off a cop (owner report: 46 -> 17 fps) and 2-4 ms with props at rest.
    ccd_gated: HashMap<RigidBodyHandle, f32>,
    spawn_embedded_checked: bool,
    /// Stored drag fractions of bodies from a read collision file (PhysicsSolid).
    drag_fractions: HashMap<RigidBodyHandle, Vec3>,
    pub native_shape_count: usize,
    pub native_shape_fallbacks: usize,
    pub skipped: usize,
    /// The player's VPhysics shadow body and whether it uses the crouch hull.
    pub player_shadow: Option<(RigidBodyHandle, bool)>,
    /// NPC hull box collider and its (mins, maxs) per entity.
    npc_hulls: BTreeMap<usize, (ColliderHandle, Vec3, Vec3)>,
    /// Model-shaped colliders of NPCs that have a hull box: hitboxes for traces only,
    /// not solid to player movement and without contacts.
    npc_hitboxes: std::collections::HashSet<ColliderHandle>,
    /// NPCs whose traces use their animated studio hitboxes.
    studio_hitboxes: BTreeMap<usize, StudioHitboxSet>,
    collider_hitgroups: HashMap<ColliderHandle, i32>,
    collider_hitbox_bones: HashMap<ColliderHandle, usize>,
    /// Diagnostics: wall-clock milliseconds of the last tick.
    pub last_step_ms: f64,
    /// Props launched by the gravity gun whose first collision has not happened yet
    /// (CPhysicsProp m_bFirstCollisionAfterLaunch).
    pub launched: std::collections::HashSet<usize>,
    /// Props a launch stuck in the world (StickAtPosition: motion disabled, debris,
    /// SF_PHYSPROP_ENABLE_ON_PHYSCANNON until a gravity gun or +USE pickup) and the
    /// position they return to when freed (SetEnableMotionPosition: before the embed).
    pub stuck: HashMap<usize, Vec3>,
    /// Props the player threw from a +USE carry (CPhysicsProp m_bThrownByPlayer): the
    /// next NPC they hit gets g_interactionHitByPlayerThrownPhysObj; cleared then or
    /// when the body sleeps.
    pub thrown_by_player: std::collections::HashSet<usize>,
    /// Props with SF_PHYSPROP_PREVENT_PICKUP (0x200): FVPHYSICS_NO_PLAYER_PICKUP and
    /// CPhysicsProp::CanBePickedUpByPhyscannon false.
    pub no_pickup: std::collections::HashSet<usize>,
}
/// Body-local hull points (metres) of a body's colliders for its drag geometry.
fn drag_geometry(body: &RigidBody, colliders: &ColliderSet) -> Option<crate::drag::DragGeometry> {
    let mut points = Vec::new();
    for handle in body.colliders() {
        let Some(collider) = colliders.get(*handle) else {
            continue;
        };
        let local = collider
            .position_wrt_parent()
            .copied()
            .unwrap_or_else(Isometry::identity);
        let shape = collider.shape();
        let mut add = |p: Point<Real>| {
            let q = local * p;
            points.push(Vec3::new(q.x, q.y, q.z));
        };
        if let Some(hull) = shape.as_convex_polyhedron() {
            hull.points().iter().copied().for_each(&mut add);
        } else {
            let aabb = shape.compute_local_aabb();
            for i in 0..8 {
                add(point![
                    if i & 1 == 0 { aabb.mins.x } else { aabb.maxs.x },
                    if i & 2 == 0 { aabb.mins.y } else { aabb.maxs.y },
                    if i & 4 == 0 { aabb.mins.z } else { aabb.maxs.z }
                ]);
            }
        }
    }
    crate::drag::DragGeometry::from_points(&points)
}
/// One drag-controller step on a body (retail 10012ec0 order: linear, then angular).
fn apply_drag(
    body: &mut RigidBody,
    geometry: &crate::drag::DragGeometry,
    density: f32,
    dt: f32,
    angular: bool,
) {
    use crate::drag::{angular_query, linear_query, step_factor, DEFAULT_COEFFICIENT};
    let props = body.mass_properties().local_mprops;
    let inertia = props.inv_principal_inertia_sqrt;
    let inv_inertia = Vec3::new(
        inertia.x * inertia.x,
        inertia.y * inertia.y,
        inertia.z * inertia.z,
    );
    let rotation = *body.rotation();
    let v = *body.linvel();
    let local = rotation.inverse_transform_vector(&v);
    let factor = step_factor(
        linear_query(
            geometry.linear_basis(props.inv_mass),
            Vec3::new(local.x, local.y, local.z),
            DEFAULT_COEFFICIENT,
        ),
        -0.5,
        density,
        dt,
    );
    if factor < 0. {
        body.set_linvel(v + v * factor, false);
    }
    if !angular {
        return;
    }
    let w = *body.angvel();
    let local = rotation.inverse_transform_vector(&w);
    let factor = step_factor(
        angular_query(
            geometry.angular_basis(inv_inertia),
            Vec3::new(local.x, local.y, local.z),
            DEFAULT_COEFFICIENT,
        ),
        -1.,
        density,
        dt,
    );
    if factor < 0. {
        body.set_angvel(w + w * factor, false);
    }
}
impl Physics {
    pub fn new(world: &World) -> Self {
        let mut p = Self {
            player_world_brushes: world.brushes.clone(),
            air_density: crate::drag::AIR_DENSITY,
            ..Self::default()
        };
        // World brushes use the "default" surfaceprop (friction/elasticity) when the
        // table is loaded; brushes with other side materials also get per-contact
        // side materials through WorldSideMaterials.
        let world_material = world_material(world);
        for (index, brush) in world.brushes.iter().enumerate() {
            if let Some(shape) = brush_shape(brush) {
                let sides = brush_sides(world, index, brush);
                let builder = world_material(ColliderBuilder::new(shape));
                let builder = if sides.is_some() {
                    builder.active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS)
                } else {
                    builder
                };
                // Clip-only brushes: no contacts with physics objects (see clip_brushes).
                let clip = brush.contents & MASK_SOLID == 0;
                let builder = if clip {
                    let hooks = builder.active_hooks | ActiveHooks::FILTER_CONTACT_PAIRS;
                    builder.active_hooks(hooks)
                } else {
                    builder
                };
                let collider = p.colliders.insert(builder);
                if clip {
                    p.world_sides.clip_brushes.insert(collider);
                }
                if let Some(sides) = sides {
                    p.world_sides.sides.insert(collider, sides);
                }
                p.world_brush_contents.insert(collider, brush.contents);
                p.npc_collider_contents.insert(collider, brush.contents);
            } else {
                p.skipped += 1;
            }
        }
        for surface in &world.terrain {
            // Displacement triangles keep the BSP's clockwise-front winding (their
            // geometric normals point away from the face normal), while Rapier's
            // oriented trimesh (FIX_INTERNAL_EDGES) treats the counter-clockwise side as
            // the outside: reversed, props rest on displacements instead of falling
            // through them (terracotta pot #491 on the tr02 playground fell 9 units).
            // Without internal-edge correction: with it, a pot resting on the flat
            // playground displacement jittered, wandered and tipped over within 5 s
            // (native rests upright); VPhysics displacement collision has none either.
            let mut surface = surface.clone();
            for triangle in surface.indices.as_chunks_mut::<3>().0 {
                triangle.swap(1, 2);
            }
            if let Some(shape) = mesh_shape_with(
                std::slice::from_ref(&surface),
                false,
                None,
                TriMeshFlags::MERGE_DUPLICATE_VERTICES,
            ) {
                p.colliders
                    .insert(world_material(ColliderBuilder::new(shape)));
            }
        }
        for (id, e) in world.entities.iter().enumerate() {
            let Some(model_id) = e
                .get("model")
                .and_then(|m| m.strip_prefix('*'))
                .and_then(|s| s.parse::<usize>().ok())
            else {
                continue;
            };
            let Some(model) = world.brush_models.iter().find(|m| m.id == model_id) else {
                continue;
            };
            let solid = matches!(
                e.class(),
                "func_brush"
                    | "func_wall"
                    | "func_wall_toggle"
                    | "func_door"
                    | "func_door_rotating"
                    | "func_movelinear"
                    | "func_rotating"
                    | "func_breakable"
                    | "func_physbox"
                    | "func_detail"
            );
            if !solid || e.get("solid") == Some("0") || e.get("Solidity") == Some("1") {
                continue;
            }
            let position = pose(
                e.origin(),
                angles(
                    modkit_core::parse_vec3(e.get("angles").unwrap_or("0 0 0"))
                        .unwrap_or(Vec3::ZERO),
                ),
            );
            for b in &model.brushes {
                if let Some(shape) = brush_shape(b) {
                    let c = p.colliders.insert(
                        ColliderBuilder::new(shape)
                            .position(position)
                            .user_data(id as u128 + 1)
                            .friction(0.8),
                    );
                    p.entity_colliders.entry(id).or_default().push(c);
                    p.npc_collider_contents.insert(c, b.contents);
                }
            }
        }
        let mut shape_cache = BTreeMap::new();
        for instance in &world.model_instances {
            p.add_model_instance(world, instance, &mut shape_cache);
        }
        // Fold additional (authored) mass properties in before the first query.
        for handle in p.dynamic.values() {
            if let Some(body) = p.bodies.get_mut(*handle) {
                body.recompute_mass_properties_from_colliders(&p.colliders);
            }
        }
        p.add_constraints(world);
        p.add_collision_pairs(world);
        p.query.update(&p.colliders);
        p
    }
    /// logic_collision_pair (logicentities.cpp CLogicCollisionPair::Activate): with
    /// startdisabled 1 the two named entities never collide with each other, e.g. the
    /// d1_trainstation_02 teeter-totter seat and the static base it is authored inside.
    /// Not modeled: the EnableCollisions/DisableCollisions inputs.
    fn add_collision_pairs(&mut self, world: &World) {
        let named = |name: &str| {
            world.entities.iter().position(|e| {
                e.get("targetname")
                    .is_some_and(|n| n.eq_ignore_ascii_case(name.trim()))
            })
        };
        for e in world.entities.iter() {
            if e.class() != "logic_collision_pair"
                || e.get("startdisabled")
                    .and_then(|v| v.trim().parse::<i32>().ok())
                    != Some(1)
            {
                continue;
            }
            let (Some(a), Some(b)) = (
                e.get("attach1").and_then(named),
                e.get("attach2").and_then(named),
            ) else {
                continue;
            };
            self.world_sides.disabled_pairs.insert((a.min(b), a.max(b)));
            for id in [a, b] {
                for c in self.entity_colliders.get(&id).cloned().unwrap_or_default() {
                    if let Some(collider) = self.colliders.get_mut(c) {
                        let hooks = collider.active_hooks() | ActiveHooks::FILTER_CONTACT_PAIRS;
                        collider.set_active_hooks(hooks);
                    }
                }
            }
        }
    }
    /// VPhysics constraint entities (SDK physconstraint.cpp CPhysConstraint::
    /// ActivateConstraint) as Rapier impulse joints between the attached bodies; a
    /// missing or non-physics attachment is the world. phys_hinge: revolute about
    /// `hingeaxis - origin`, with SetAxisFriction(hingefriction x 1000, kg in^2/s^2) as a
    /// capped zero-velocity motor; phys_ballsocket: spherical at the origin;
    /// phys_ragdollconstraint: spherical with the x/y/z min/max limits (degrees) in the
    /// constraint's angles; phys_lengthconstraint: rope from the origin (attach1 side) to
    /// `attachpoint` (attach2 side), length + addlength; phys_constraint: fixed.
    /// SF_CONSTRAINT_DISABLE_COLLISION (1) turns contacts off between the pair.
    /// Not modeled: breaking (force/torque limits), constraint systems' solver
    /// grouping, StartInactive/TurnOn/TurnOff, sounds, sliders, pulleys, phys_thruster.
    fn add_constraints(&mut self, world: &World) {
        let named = |name: &str| {
            world.entities.iter().position(|e| {
                e.get("targetname")
                    .is_some_and(|n| n.eq_ignore_ascii_case(name.trim()))
            })
        };
        let mut fixed = None;
        for (id, e) in world.entities.iter().enumerate() {
            let class = e.class();
            if !matches!(
                class,
                "phys_hinge"
                    | "phys_ballsocket"
                    | "phys_ragdollconstraint"
                    | "phys_lengthconstraint"
                    | "phys_constraint"
            ) {
                continue;
            }
            let body = |key: &str| {
                e.get(key)
                    .filter(|n| !n.trim().is_empty())
                    .and_then(named)
                    .and_then(|target| self.dynamic.get(&target).copied())
            };
            let (a, b) = (body("attach1"), body("attach2"));
            if a.is_none() && b.is_none() {
                self.constraint_errors
                    .push(format!("{class} #{id}: no physics attachment"));
                continue;
            }
            let world_hinge = a.is_none() || b.is_none();
            let world_body =
                *fixed.get_or_insert_with(|| self.bodies.insert(RigidBodyBuilder::fixed()));
            // The world side first (a multibody's fixed root is its parent).
            let (a, b) = match (a, b) {
                (Some(a), Some(b)) => (a, b),
                (Some(body), None) | (None, Some(body)) => (world_body, body),
                (None, None) => unreachable!("checked above"),
            };
            let flags = e
                .get("spawnflags")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(0);
            let origin = e.origin();
            // SF_CONSTRAINT_ASSUME_WORLD_GEOMETRY on a world hinge (CPhysHinge::Spawn):
            // the object's mass center is snapped onto the hinge axis
            // (masscenteroverride_t::SnapToAxis) and IVP makes it a BecomeHinged object;
            // here the snapped body keeps an ordinary revolute joint to the world. Session
            // 15's impulse joint drifted 115 units because the teeter-totter seat also
            // collided with the base it is authored inside; logic_collision_pair now
            // disables that pair.
            if class == "phys_hinge" && flags & 8 != 0 && world_hinge {
                if let Some(axis) = e
                    .get("hingeaxis")
                    .and_then(modkit_core::parse_vec3)
                    .map(|p| (p - origin).normalize_or_zero())
                    .filter(|a| a.length_squared() > 0.5)
                {
                    self.snap_mass_center_to_axis(b, origin, axis);
                }
            }
            let point = |key: &str| e.get(key).and_then(modkit_core::parse_vec3);
            let number = |key: &str| {
                e.get(key)
                    .and_then(|v| v.trim().parse::<f32>().ok())
                    .unwrap_or(0.)
            };
            let frame_rotation = if class == "phys_hinge" {
                let Some(axis) = point("hingeaxis")
                    .map(|p| (p - origin).normalize_or_zero())
                    .filter(|a| a.length_squared() > 0.5)
                else {
                    self.constraint_errors
                        .push(format!("{class} #{id}: bad hinge axis"));
                    continue;
                };
                Quat::from_rotation_arc(Vec3::X, axis)
            } else {
                angles(point("angles").unwrap_or(Vec3::ZERO))
            };
            let frame = pose(origin, frame_rotation);
            let local = |handle: RigidBodyHandle| {
                self.bodies
                    .get(handle)
                    .map_or(frame, |body| body.position().inv_mul(&frame))
            };
            let contacts = flags & 1 == 0;
            if !contacts && world_hinge {
                self.world_sides.world_disabled_bodies.insert(b);
                for c in self
                    .bodies
                    .get(b)
                    .map(|body| body.colliders().to_vec())
                    .unwrap_or_default()
                {
                    if let Some(collider) = self.colliders.get_mut(c) {
                        let hooks = collider.active_hooks() | ActiveHooks::FILTER_CONTACT_PAIRS;
                        collider.set_active_hooks(hooks);
                    }
                }
            }
            let joint: GenericJoint = match class {
                "phys_hinge" => {
                    // SetAxisFriction(hingefriction x 1000), x DEG2RAD into IVP (retail
                    // vphysics InitHinge FUN_1000b110).
                    let friction = (number("hingefriction") * 1000.).to_radians();
                    let mut joint = GenericJointBuilder::new(JointAxesMask::LOCKED_REVOLUTE_AXES)
                        .local_frame1(local(a))
                        .local_frame2(local(b))
                        .contacts_enabled(contacts);
                    if friction > 0. {
                        joint = joint
                            .motor_velocity(JointAxis::AngX, 0., 1e3)
                            .motor_max_force(JointAxis::AngX, friction);
                    }
                    joint.build()
                }
                "phys_ballsocket" => GenericJointBuilder::new(JointAxesMask::LOCKED_SPHERICAL_AXES)
                    .local_frame1(local(a))
                    .local_frame2(local(b))
                    .contacts_enabled(contacts)
                    .build(),
                "phys_ragdollconstraint" => {
                    let limit =
                        |min: &str, max: &str| [number(min).to_radians(), number(max).to_radians()];
                    GenericJointBuilder::new(JointAxesMask::LOCKED_SPHERICAL_AXES)
                        .local_frame1(local(a))
                        .local_frame2(local(b))
                        .contacts_enabled(contacts)
                        .limits(JointAxis::AngX, limit("xmin", "xmax"))
                        .limits(JointAxis::AngY, limit("ymin", "ymax"))
                        .limits(JointAxis::AngZ, limit("zmin", "zmax"))
                        .build()
                }
                // Rejected for now (session 15, two attempts): the d1_trainstation_02
                // swing seats hung from two ropes each first dropped ~19 units, then
                // were flung to z 245 (also with CCD off for jointed bodies). Ropes stay
                // off until a different approach is validated; seats fall as before.
                "phys_lengthconstraint" if !ROPE_CONSTRAINTS => {
                    self.constraint_errors
                        .push(format!("{class} #{id}: rope constraints disabled"));
                    continue;
                }
                "phys_lengthconstraint" => {
                    let Some(attach) = point("attachpoint") else {
                        continue;
                    };
                    let length = origin.distance(attach) + number("addlength");
                    let anchor = |handle: RigidBodyHandle, at: Vec3| {
                        let p = Point::from(vector(at));
                        self.bodies
                            .get(handle)
                            .map_or(p, |body| body.position().inverse_transform_point(&p))
                    };
                    RopeJointBuilder::new(length * SCALE)
                        .local_anchor1(anchor(a, origin))
                        .local_anchor2(anchor(b, attach))
                        .contacts_enabled(contacts)
                        .build()
                        .data
                }
                _ => GenericJointBuilder::new(JointAxesMask::LOCKED_FIXED_AXES)
                    .local_frame1(local(a))
                    .local_frame2(local(b))
                    .contacts_enabled(contacts)
                    .build(),
            };
            self.impulses.insert(a, b, joint, true);
            self.constraints += 1;
        }
    }
    /// masscenteroverride_t::SnapToAxis: move a body's mass center to the closest point
    /// on the line through `point` along `axis` (world space), keeping mass and inertia.
    fn snap_mass_center_to_axis(&mut self, handle: RigidBodyHandle, point: Vec3, axis: Vec3) {
        let Some(body) = self.bodies.get_mut(handle) else {
            return;
        };
        let props = body.mass_properties().local_mprops;
        let com = body.position() * props.local_com;
        let com = Vec3::new(com.x, com.y, com.z) / SCALE;
        let snapped = point + axis * (com - point).dot(axis);
        let local = body
            .position()
            .inverse_transform_point(&Point::from(vector(snapped)));
        let mass = props.mass();
        let snapped_props = MassProperties::with_principal_inertia_frame(
            local,
            mass,
            props.principal_inertia(),
            props.principal_inertia_local_frame,
        );
        // Map props carry their mass as additional properties over zero-density
        // colliders, so replacing them moves the center.
        body.set_additional_mass_properties(snapped_props, true);
        body.recompute_mass_properties_from_colliders(&self.colliders);
    }
    /// One model instance's colliders (props, NPC hull + studio hitboxes), shared by
    /// map load and runtime spawns (DESIGN 14r). `shape_cache` shares convex shapes
    /// between instances of one model.
    fn add_model_instance(
        &mut self,
        world: &World,
        instance: &modkit_core::ModelInstance,
        shape_cache: &mut BTreeMap<(String, bool, Option<String>, bool), PropShapes>,
    ) {
        if instance.background {
            return;
        }
        if !instance.solid {
            return;
        }
        let Some(asset) = world.model_assets.get(&instance.asset_key()) else {
            return;
        };
        // prop_physics spawnflags: 1 start asleep, 8 motion disabled (static until
        // EnableMotion, which is not implemented).
        let prop_flags = instance
            .entity
            .and_then(|id| world.entities.get(id))
            .and_then(|e| e.get("spawnflags"))
            .and_then(|f| f.parse::<u32>().ok())
            .unwrap_or(0);
        let dynamic = instance.kind.starts_with("prop_physics") && prop_flags & 8 == 0;
        // The installed rotating door's VVD bind mesh points along X, but
        // its idle sequence turns the panel along Y. Use the same initial
        // skinned pose as rendering before applying the entity's swing.
        // Keep other model collision policies unchanged.
        let clip = (instance.kind == "prop_door_rotating").then(|| {
            instance
                .entity
                .and_then(|id| world.entities.get(id))
                .and_then(|e| e.get("DefaultAnim"))
                .unwrap_or("idle")
                .to_lowercase()
        });
        // VPhysics props use their authored .phy pieces once the model loader
        // read the solid parameters (CPhysicsProp::CreateVPhysics); otherwise
        // the prior render-hull policy stays.
        let authored = dynamic
            .then(|| world.model_physics.get(&instance.asset_key()))
            .flatten();
        let native_eligible = (instance.kind == "static_prop" && instance.solid_mode == Some(6))
            || authored.is_some();
        let key = (instance.asset_key(), dynamic, clip.clone(), native_eligible);
        let cached = shape_cache.entry(key).or_insert_with(|| {
            let pieces = native_eligible
                .then(|| world.model_collision.get(&instance.asset_key()))
                .flatten();
            if let Some(pieces) = pieces.filter(|pieces| !pieces.is_empty()) {
                // Preserve authored convex decomposition. Merging all
                // vertices into one hull fills bench/railing openings.
                let shapes = pieces
                    .iter()
                    .map(|piece| {
                        if piece.vertices.len() < 4
                            || !piece.vertices.iter().all(|point| point.is_finite())
                        {
                            return None;
                        }
                        SharedShape::convex_hull(
                            &piece
                                .vertices
                                .iter()
                                .map(|point| Point::from(vector(*point)))
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(shapes) = shapes {
                    return PropShapes {
                        shapes,
                        native: true,
                        native_failed: false,
                    };
                }
            }
            let matrices = clip.as_ref().and_then(|name| {
                world
                    .rigs
                    .get(&instance.asset_key())
                    .filter(|rig| rig.clips.contains_key(name))
                    .map(|rig| rig.matrices(name, 0.))
            });
            PropShapes {
                shapes: mesh_shape(asset, dynamic, matrices.as_deref())
                    .into_iter()
                    .collect(),
                native: false,
                // An invalid native piece never silently removes part of
                // the collider: the complete model uses the mesh fallback.
                native_failed: pieces.is_some(),
            }
        });
        if cached.shapes.is_empty() {
            self.skipped += 1;
            return;
        }
        // Scaled props need separately scaled meshes; skip their collider rather than use a wrong size.
        if (instance.scale - 1.).abs() > 0.001 {
            self.skipped += 1;
            return;
        }
        self.native_shape_count += usize::from(cached.native) * cached.shapes.len();
        self.native_shape_fallbacks += usize::from(cached.native_failed);
        let position = pose(instance.origin, angles(instance.angles));
        let entity = instance.entity;
        let keyvalue = |key: &str| {
            instance
                .entity
                .and_then(|id| world.entities.get(id))
                .and_then(|e| e.get(key))
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite())
        };
        let solid = authored.map(|solid| authored_solid(solid, keyvalue));
        let (linear_damping, angular_damping) = solid
            .as_ref()
            .map_or((0.05, 0.1), |s| (s.damping, s.rotdamping));
        let masses = solid
            .as_ref()
            .map(|s| authored_mass_properties(&cached.shapes, s.mass, s.inertia));
        let body = dynamic.then(|| {
            self.bodies.insert(
                RigidBodyBuilder::dynamic()
                    .additional_mass_properties(masses.unwrap_or_default())
                    .position(position)
                    .sleeping(prop_flags & 1 != 0)
                    .linear_damping(linear_damping)
                    .angular_damping(angular_damping),
            )
        });
        if let (Some(id), Some(body)) = (entity, body) {
            self.ccd_gated.insert(body, 0.);
            self.dynamic.insert(id, body);
            if world
                .entities
                .get(id)
                .and_then(|e| e.get("spawnflags"))
                .and_then(|v| v.trim().parse::<u32>().ok())
                .is_some_and(|flags| flags & 0x200 != 0)
            {
                self.no_pickup.insert(id);
            }
        }
        if let (Some(body), Some(fractions)) = (body, solid.as_ref().and_then(|s| s.drag_fractions))
        {
            self.drag_fractions.insert(body, fractions);
        }
        let material = solid
            .as_ref()
            .and_then(|s| surface_material(world, &s.surfaceprop));
        if let (Some(id), Some(s)) = (entity, &solid) {
            self.prop_materials.insert(id, s.surfaceprop.clone());
        }
        let surfaceprop = solid_surfaceprop(world, instance, solid.as_ref());
        for shape in &cached.shapes {
            let collider = match material {
                // VPhysics multiplies both surfaces' friction and elasticity
                // (frag fit: grenade 0.9 x concrete 0.8). Rapier restitution is
                // not IVP elasticity: "native fit needed".
                Some(m) => ColliderBuilder::new(shape.clone())
                    .friction(m.friction)
                    .restitution(restitution(m))
                    .friction_combine_rule(CoefficientCombineRule::Multiply)
                    .restitution_combine_rule(CoefficientCombineRule::Multiply),
                None => ColliderBuilder::new(shape.clone())
                    .friction(0.7)
                    .restitution(0.05),
            }
            .user_data(entity.map_or(0, |i| i as u128 + 1));
            let c = match body {
                Some(body) => {
                    // Authored bodies carry their mass on the body; pieces add none.
                    let collider = match &masses {
                        Some(_) => collider.density(0.),
                        None => collider.mass(10.),
                    }
                    .contact_skin(VPHYSICS_CONTACT_MARGIN * SCALE)
                    .active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS);
                    self.colliders
                        .insert_with_parent(collider, body, &mut self.bodies)
                }
                None => self.colliders.insert(collider.position(position)),
            };
            if let Some(id) = entity {
                self.entity_colliders.entry(id).or_default().push(c);
            }
            if let Some(name) = &surfaceprop {
                self.collider_materials.insert(c, name.clone());
            }
            if instance.kind.starts_with("npc_") {
                self.colliders[c]
                    .set_collision_groups(InteractionGroups::new(NPC_GROUP, Group::ALL));
            } else if body.is_some() && prop_flags & 4 != 0 {
                self.colliders[c].set_collision_groups(DEBRIS_GROUPS);
            }
            self.npc_collider_contents.insert(
                c,
                if instance.kind.starts_with("npc_") {
                    CONTENTS_MONSTER
                } else {
                    1
                },
            );
        }
        // SOLID_BBOX NPCs: movement and physics collide with the class hull box at
        // the entity origin (never rotated); the model shape stays for traces.
        if let (Some(id), Some((mins, maxs))) = (entity, npc_hull(&instance.kind)) {
            for c in self.entity_colliders.get(&id).cloned().unwrap_or_default() {
                if let Some(collider) = self.colliders.get_mut(c) {
                    collider.set_active_collision_types(ActiveCollisionTypes::empty());
                    collider.set_active_hooks(ActiveHooks::FILTER_CONTACT_PAIRS);
                }
                self.npc_hitboxes.insert(c);
                self.world_sides.hitboxes.insert(c);
            }
            let half = (maxs - mins) * 0.5;
            let hull = self.colliders.insert(
                ColliderBuilder::cuboid(half.x * SCALE, half.y * SCALE, half.z * SCALE)
                    .translation(vector(instance.origin + (mins + maxs) * 0.5))
                    .friction(0.7)
                    .user_data(id as u128 + 1)
                    .collision_groups(npc_trace::hull_groups(&instance.kind)),
            );
            self.npc_collider_contents.insert(hull, CONTENTS_MONSTER);
            if let Some(s) = &solid {
                self.collider_materials.insert(hull, s.surfaceprop.clone());
            }
            self.npc_hulls.insert(id, (hull, mins, maxs));
            // Bullets trace the studio hitboxes (CBaseAnimating::TestHitboxes),
            // not the render mesh: one box per hitbox, posed per tick.
            if let Some(rig) = world
                .rigs
                .get(&instance.asset_key())
                .filter(|rig| !rig.hitboxes.is_empty())
            {
                self.use_studio_hitboxes(id, instance, rig);
            }
        }
    }
    /// A runtime-spawned entity's model instance (DESIGN 14r part A): the same
    /// colliders as a map entity of its model; queries see them after the next
    /// refresh_entity_queries.
    pub fn add_spawned_instance(&mut self, world: &World, instance: &modkit_core::ModelInstance) {
        let before: std::collections::HashSet<ColliderHandle> =
            self.colliders.iter().map(|(h, _)| h).collect();
        self.add_model_instance(world, instance, &mut BTreeMap::new());
        let added: Vec<ColliderHandle> = self
            .colliders
            .iter()
            .map(|(h, _)| h)
            .filter(|h| !before.contains(h))
            .collect();
        self.changed_entity_colliders.extend(added);
        let handle = instance
            .entity
            .and_then(|id| self.dynamic.get(&id))
            .copied();
        if let Some(body) = handle.and_then(|h| self.bodies.get_mut(h)) {
            body.recompute_mass_properties_from_colliders(&self.colliders);
        }
    }
    /// Dynamic bodies the island manager keeps awake.
    pub fn awake_bodies(&self) -> usize {
        self.islands.active_dynamic_bodies().len()
    }
    /// A runtime-spawned NPC's collider (DESIGN 14r step 3): an axis box from `mins`
    /// to `maxs` (the class hull, e.g. HULL_HUMAN -13 -13 0 .. 13 13 72) that
    /// set_entity then moves with the scene state, CONTENTS_MONSTER for NPC traces.
    /// The model's own collision pieces replace it once the late precache supplies
    /// them (step 4).
    pub fn add_entity_hull(&mut self, id: usize, mins: Vec3, maxs: Vec3, origin: Vec3) {
        let half = (maxs - mins) * 0.5 * SCALE;
        let center = (maxs + mins) * 0.5 * SCALE;
        let shape = SharedShape::compound(vec![(
            Isometry::translation(center.x, center.y, center.z),
            SharedShape::cuboid(half.x, half.y, half.z),
        )]);
        let collider = ColliderBuilder::new(shape)
            .friction(0.7)
            .restitution(0.05)
            .user_data(id as u128 + 1)
            .position(pose(origin, Quat::IDENTITY));
        let handle = self.colliders.insert(collider);
        self.entity_colliders.entry(id).or_default().push(handle);
        self.npc_collider_contents.insert(handle, CONTENTS_MONSTER);
        self.changed_entity_colliders.push(handle);
    }
    /// Replace an NPC's render-mesh trace collider with its studio hitboxes at the bind
    /// pose (the hull box stays for movement and contacts).
    fn use_studio_hitboxes(
        &mut self,
        id: usize,
        instance: &modkit_core::ModelInstance,
        rig: &modkit_core::animation::Rig,
    ) {
        for c in self.entity_colliders.remove(&id).unwrap_or_default() {
            self.colliders
                .remove(c, &mut self.islands, &mut self.bodies, false);
            self.npc_hitboxes.remove(&c);
            self.world_sides.hitboxes.remove(&c);
            self.npc_collider_contents.remove(&c);
            self.collider_materials.remove(&c);
        }
        let base = Mat4::from_rotation_translation(angles(instance.angles), instance.origin);
        let mut boxes = Vec::new();
        for hitbox in &rig.hitboxes {
            let Some(bone) = rig.bones.get(hitbox.bone) else {
                continue;
            };
            let half = ((hitbox.maxs - hitbox.mins) * 0.5).max(Vec3::splat(0.05));
            let center = (hitbox.maxs + hitbox.mins) * 0.5;
            let bind = bone.inverse_bind.inverse();
            let collider = ColliderBuilder::cuboid(half.x * SCALE, half.y * SCALE, half.z * SCALE)
                .position(mat4_pose(base * bind * Mat4::from_translation(center)))
                .user_data(id as u128 + 1)
                .collision_groups(InteractionGroups::new(NPC_GROUP, Group::ALL))
                .active_collision_types(ActiveCollisionTypes::empty())
                .active_hooks(ActiveHooks::FILTER_CONTACT_PAIRS);
            let c = self.colliders.insert(collider);
            self.npc_hitboxes.insert(c);
            self.world_sides.hitboxes.insert(c);
            self.npc_collider_contents.insert(c, CONTENTS_MONSTER);
            self.collider_hitgroups.insert(c, hitbox.group);
            self.collider_hitbox_bones.insert(c, hitbox.bone);
            boxes.push((c, hitbox.bone, center, bind));
        }
        self.studio_hitboxes.insert(
            id,
            StudioHitboxSet {
                boxes,
                asset_key: instance.asset_key(),
            },
        );
    }
    /// NPCs with studio hitbox colliders and their model asset keys.
    pub fn studio_hitbox_entities(&self) -> Vec<(usize, String)> {
        self.studio_hitboxes
            .iter()
            .map(|(id, set)| (*id, set.asset_key.clone()))
            .collect()
    }
    /// Pose an NPC's hitboxes from its skinning matrices (bone pose x inverse bind, model
    /// space) and entity frame; `enabled` follows the entity.
    pub fn set_npc_hitboxes(&mut self, id: usize, frame: Mat4, skin: &[Mat4], enabled: bool) {
        let Some(set) = self.studio_hitboxes.get(&id) else {
            return;
        };
        for (c, bone, center, bind) in &set.boxes {
            let Some(collider) = self.colliders.get_mut(*c) else {
                continue;
            };
            if enabled {
                let model = skin.get(*bone).map_or(*bind, |m| *m * *bind);
                let at = mat4_pose(frame * model * Mat4::from_translation(*center));
                if *collider.position() != at {
                    collider.set_position(at);
                    self.changed_entity_colliders.push(*c);
                }
            }
            if collider.is_enabled() != enabled {
                collider.set_enabled(enabled);
                self.changed_entity_colliders.push(*c);
            }
        }
    }
    fn hitgroup(&self, handle: ColliderHandle) -> i32 {
        self.collider_hitgroups.get(&handle).copied().unwrap_or(0)
    }
    fn hitbox_bone(&self, handle: ColliderHandle) -> Option<usize> {
        self.collider_hitbox_bones.get(&handle).copied()
    }
    pub fn set_entity(&mut self, id: usize, origin: Vec3, rotation: Quat, enabled: bool) {
        let dynamic = self.dynamic.get(&id);
        if dynamic
            .and_then(|h| self.bodies.get(*h))
            .is_some_and(|body| body.is_enabled() != enabled)
        {
            self.bodies
                .get_mut(*dynamic.unwrap())
                .unwrap()
                .set_enabled(enabled);
        }
        if let Some(&(hull, mins, maxs)) = self.npc_hulls.get(&id) {
            let at = pose(origin + (mins + maxs) * 0.5, Quat::IDENTITY);
            if let Some(c) = self.colliders.get_mut(hull) {
                if *c.position() != at || c.is_enabled() != enabled {
                    c.set_position(at);
                    c.set_enabled(enabled);
                    self.changed_entity_colliders.push(hull);
                }
            }
        }
        if let Some(set) = self.studio_hitboxes.get(&id) {
            for (c, ..) in &set.boxes {
                if let Some(collider) = self.colliders.get_mut(*c) {
                    if collider.is_enabled() != enabled {
                        collider.set_enabled(enabled);
                        self.changed_entity_colliders.push(*c);
                    }
                }
            }
        }
        let target = dynamic.is_none().then(|| pose(origin, rotation));
        if let Some(colliders) = self.entity_colliders.get(&id) {
            for h in colliders {
                let Some(c) = self.colliders.get(*h) else {
                    continue;
                };
                let move_pose = target.as_ref().is_some_and(|target| c.position() != target);
                let change_enabled = c.is_enabled() != enabled;
                if !move_pose && !change_enabled {
                    continue;
                }
                let c = self.colliders.get_mut(*h).unwrap();
                if move_pose {
                    c.set_position(*target.as_ref().unwrap());
                }
                if change_enabled {
                    c.set_enabled(enabled);
                }
                self.changed_entity_colliders.push(*h);
            }
        }
    }
    /// Refresh just entity poses changed through set_entity; physics.step updates dynamic bodies.
    /// Direct external collider edits still require the full refresh_queries entry point.
    pub fn refresh_entity_queries(&mut self) {
        if !self.changed_entity_colliders.is_empty() {
            self.query.update_incremental(
                &self.colliders,
                &self.changed_entity_colliders,
                &[],
                true,
            );
            self.changed_entity_colliders.clear();
        }
    }
    /// Refresh once after a batch of scripted pose/solid updates and before
    /// projectile queries; otherwise broad-phase bounds lag a moving door.
    pub fn refresh_queries(&mut self) {
        self.query.update(&self.colliders);
        self.changed_entity_colliders.clear();
    }
    pub fn tick(&mut self, dt: f32) {
        let started = std::time::Instant::now();
        self.tick_inner(dt);
        self.last_step_ms = started.elapsed().as_secs_f64() * 1000.;
    }
    /// Props authored interpenetrating a static entity (the teeter-totter seat in its
    /// prop_dynamic base) keep continuous collision: Rapier's penetration recovery ejects
    /// them hundreds of units once CCD is off. Found once, after the first step's
    /// contacts; props pressed into geometry later (a pallet on a cop hull, props sunk
    /// into terrain) stay gated.
    fn ungate_spawn_embedded(&mut self) {
        if self.spawn_embedded_checked || self.time <= 0. {
            return;
        }
        self.spawn_embedded_checked = true;
        let static_entity = |c: &Collider| {
            c.parent().is_none()
                && c.user_data != 0
                && c.collision_groups().memberships != HULL_GROUP
        };
        for pair in self.narrow.contact_pairs() {
            if !pair.has_any_active_contact
                || !pair
                    .manifolds
                    .iter()
                    .any(|m| m.points.iter().any(|p| p.dist < -EMBEDDED_DEPTH * SCALE))
            {
                continue;
            }
            let (Some(c1), Some(c2)) = (
                self.colliders.get(pair.collider1),
                self.colliders.get(pair.collider2),
            ) else {
                continue;
            };
            for (body, other) in [(c1, c2), (c2, c1)] {
                if let (Some(parent), true) = (body.parent(), static_entity(other)) {
                    self.ccd_gated.remove(&parent);
                    if let Some(b) = self.bodies.get_mut(parent) {
                        b.enable_ccd(true);
                    }
                }
            }
        }
    }
    fn tick_inner(&mut self, dt: f32) {
        self.player_convex_cache
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(&self.colliders);
        self.ungate_spawn_embedded();
        // VPhysics drag controller (drag.rs: retail per-object bases, native air density).
        let mut before = HashMap::new();
        for (handle, body) in self.bodies.iter_mut() {
            if !body.is_dynamic() || body.is_sleeping() || !body.is_enabled() {
                continue;
            }
            if let Some(thickness) = self.ccd_gated.get_mut(&handle) {
                if *thickness <= 0. {
                    *thickness = body_thickness(body, &self.colliders);
                }
                let fast = body.linvel().norm() * dt > *thickness;
                if body.is_ccd_enabled() != fast {
                    body.enable_ccd(fast);
                }
            }
            if self.air_density > 0. && !self.drag_disabled.contains(&handle) {
                let colliders = &self.colliders;
                let stored = self.drag_fractions.get(&handle).copied();
                if let Some(geometry) = *self.drag_geometry.entry(handle).or_insert_with(|| {
                    drag_geometry(body, colliders)
                        .map(|g| stored.map_or(g, |fractions| g.with_fractions(fractions)))
                }) {
                    let angular = !self.angular_drag_disabled.contains(&handle);
                    apply_drag(body, &geometry, self.air_density, dt, angular);
                }
            }
            before.insert(
                handle,
                (*body.linvel(), *body.angvel(), *body.center_of_mass()),
            );
        }
        self.pipeline.step(
            &vector![0., 0., -600. * SCALE],
            &IntegrationParameters {
                dt,
                ..Default::default()
            },
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulses,
            &mut self.multibody,
            &mut self.ccd,
            Some(&mut self.query),
            &self.world_sides,
            &(),
        );
        self.time += f64::from(dt);
        // physics_performanceparams_t::Defaults (SDK performance.h; CPhysicsHook keeps
        // them): world-space speed limits of 2000 u/s and 3600 deg/s per object.
        for (_, body) in self.bodies.iter_mut() {
            if !body.is_dynamic() || body.is_sleeping() {
                continue;
            }
            let (v, w) = (*body.linvel(), *body.angvel());
            let (max_v, max_w) = (MAX_VELOCITY * SCALE, MAX_ANGULAR_VELOCITY.to_radians());
            if v.norm() > max_v {
                body.set_linvel(v * (max_v / v.norm()), false);
            }
            if w.norm() > max_w {
                body.set_angvel(w * (max_w / w.norm()), false);
            }
        }
        // Rapier finds a new contact at the end of a step and resolves it in the next:
        // last step's events get their post-impact velocities now.
        let mut resolved = std::mem::take(&mut self.collisions);
        for event in &mut resolved {
            for side in 0..2 {
                if let Some(body) = self
                    .colliders
                    .get(event.colliders[side])
                    .and_then(|c| c.parent())
                    .and_then(|b| self.bodies.get(b))
                    .filter(|b| b.is_dynamic())
                {
                    let v = body.linvel();
                    event.post_velocities[side] = Vec3::new(v.x, v.y, v.z) / SCALE;
                }
            }
        }
        self.resolved_collisions = resolved;
        self.collect_collisions(&before);
        self.changed_entity_colliders.clear();
        self.settle_interactive_debris();
    }
    /// Forget the pair times of colliders that no longer exist (past 4096 pairs).
    fn forget_pair_times(&mut self) {
        if self.pair_times.len() > 4096 {
            let colliders = &self.colliders;
            self.pair_times
                .retain(|(a, b), _| colliders.contains(*a) && colliders.contains(*b));
        }
    }
    /// New contacts of this step as VPhysics collision events: the pre-step relative
    /// speed along the contact normal (collisionSpeed) and the time since the pair last
    /// touched (deltaCollisionTime). Resting contacts that persist raise no event.
    fn collect_collisions(&mut self, before: &HashMap<RigidBodyHandle, PreStep>) {
        self.collisions.clear();
        let mut touching = std::collections::HashSet::new();
        for pair in self.narrow.contact_pairs() {
            if !pair.has_any_active_contact {
                continue;
            }
            let key = (pair.collider1, pair.collider2);
            touching.insert(key);
            if self.touching.contains(&key) {
                continue;
            }
            let (Some(c1), Some(c2)) = (
                self.colliders.get(pair.collider1),
                self.colliders.get(pair.collider2),
            ) else {
                continue;
            };
            let Some((manifold, point)) = pair.manifolds.iter().find_map(|m| {
                m.points
                    .iter()
                    .find(|p| p.dist <= 0.01)
                    .or(m.points.first())
                    .map(|p| (m, p))
            }) else {
                continue;
            };
            let normal = manifold.data.normal;
            let contact = c1.position() * point.local_p1;
            let velocity = |c: &Collider| {
                c.parent()
                    .and_then(|b| before.get(&b))
                    .map_or(Vector::zeros(), |(v, w, com)| v + w.cross(&(contact - com)))
            };
            let relative = velocity(c1) - velocity(c2);
            let speed = relative.dot(&normal).abs() / SCALE;
            let stepped = |c: &Collider| c.parent().is_some_and(|b| before.contains_key(&b));
            if !stepped(c1) && !stepped(c2) {
                continue;
            }
            let last = self.pair_times.insert(key, self.time);
            let entity = |c: &Collider| (c.user_data as usize).checked_sub(1);
            let contact_units = Vec3::new(contact.x, contact.y, contact.z) / SCALE;
            let origin = |c: &Collider| {
                c.parent()
                    .and_then(|b| self.bodies.get(b))
                    .map_or(contact_units, |b| {
                        let t = b.position().translation;
                        Vec3::new(t.x, t.y, t.z) / SCALE
                    })
            };
            let dynamic = |c: &Collider| {
                c.parent()
                    .and_then(|b| self.bodies.get(b))
                    .filter(|b| b.is_dynamic())
            };
            let pre = |c: &Collider| {
                c.parent()
                    .and_then(|b| before.get(&b))
                    .filter(|_| dynamic(c).is_some())
                    .map_or(Vec3::ZERO, |(v, _, _)| Vec3::new(v.x, v.y, v.z) / SCALE)
            };
            let post = |c: &Collider| {
                dynamic(c).map_or(Vec3::ZERO, |b| {
                    let v = b.linvel();
                    Vec3::new(v.x, v.y, v.z) / SCALE
                })
            };
            let mass = |c: &Collider| dynamic(c).map_or(0., |b| b.mass());
            let angular = |c: &Collider| match (dynamic(c), c.parent().and_then(|h| before.get(&h)))
            {
                (Some(b), Some((_, w, _))) => angular_motion(b, w),
                _ => (0., 0.),
            };
            self.collisions.push(PhysicsCollision {
                angular: [angular(c1), angular(c2)],
                pre_velocities: [pre(c1), pre(c2)],
                post_velocities: [post(c1), post(c2)],
                masses: [mass(c1), mass(c2)],
                colliders: [pair.collider1, pair.collider2],
                entities: [entity(c1), entity(c2)],
                surfaceprops: [
                    self.collider_materials.get(&pair.collider1).cloned(),
                    self.collider_materials.get(&pair.collider2).cloned(),
                ],
                position: contact_units,
                origins: [origin(c1), origin(c2)],
                normal: Vec3::new(normal.x, normal.y, normal.z),
                speed,
                delta_time: last.map_or(f32::MAX, |t| (self.time - t) as f32),
            });
        }
        self.forget_pair_times();
        self.touching = touching;
        impact_order(&mut self.collisions);
    }
    /// A free VPhysics body (thrown frag) from an authored solid: the .phy pieces in
    /// model space (a box of `half_extents` without them), the solid's mass, inertia
    /// multiplier, damping and surfaceprop. Velocity is world units/s; the angular
    /// velocity is degrees/s about the body's local axes (IPhysicsObject::SetVelocity).
    #[allow(clippy::too_many_arguments)]
    pub fn add_projectile_body(
        &mut self,
        world: &World,
        model_key: &str,
        half_extents: Vec3,
        position: Vec3,
        rotation: Quat,
        velocity: Vec3,
        local_angular_degrees: Vec3,
    ) -> RigidBodyHandle {
        self.add_runtime_body(
            world,
            model_key,
            half_extents,
            position,
            rotation,
            velocity,
            local_angular_degrees,
            false,
        )
    }
    /// A runtime prop_physics body (BreakModelCreate_Prop: a gib): the model's .phy
    /// solid like `add_projectile_body`, but colliding as an ordinary prop (NPC hulls,
    /// other props and gibs, the player shadow) with the VPhysics margin and the .phy's
    /// stored drag fractions.
    #[allow(clippy::too_many_arguments)]
    pub fn add_prop_body(
        &mut self,
        world: &World,
        model_key: &str,
        half_extents: Vec3,
        position: Vec3,
        rotation: Quat,
        velocity: Vec3,
        local_angular_degrees: Vec3,
    ) -> RigidBodyHandle {
        let body = self.add_runtime_body(
            world,
            model_key,
            half_extents,
            position,
            rotation,
            velocity,
            local_angular_degrees,
            true,
        );
        if let Some(fractions) = world
            .model_physics
            .get(model_key)
            .and_then(|s| s.drag_fractions)
        {
            self.drag_fractions.insert(body, fractions);
        }
        body
    }
    #[allow(clippy::too_many_arguments)]
    fn add_runtime_body(
        &mut self,
        world: &World,
        model_key: &str,
        half_extents: Vec3,
        position: Vec3,
        rotation: Quat,
        velocity: Vec3,
        local_angular_degrees: Vec3,
        prop: bool,
    ) -> RigidBodyHandle {
        let solid = world
            .model_physics
            .get(model_key)
            .cloned()
            .unwrap_or_default();
        let shapes: Vec<SharedShape> = world
            .model_collision
            .get(model_key)
            .and_then(|pieces| {
                pieces
                    .iter()
                    .map(|piece| {
                        prism_cylinder(&piece.vertices).or_else(|| {
                            SharedShape::convex_hull(
                                &piece
                                    .vertices
                                    .iter()
                                    .map(|point| Point::from(vector(*point)))
                                    .collect::<Vec<_>>(),
                            )
                        })
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .filter(|shapes| !shapes.is_empty())
            .unwrap_or_else(|| {
                vec![SharedShape::cuboid(
                    half_extents.x * SCALE,
                    half_extents.y * SCALE,
                    half_extents.z * SCALE,
                )]
            });
        let mass = solid.mass.max(0.001);
        // VPhysics takes the inertia precomputed in the collision model (IVP compact
        // surface, per kilogram) rather than one derived from the hull it simulates.
        let masses = match solid.compact_inertia {
            Some((center, inertia)) if inertia.is_finite() && inertia.min_element() > 0. => {
                let inertia = inertia * mass * solid.inertia;
                MassProperties::new(
                    Point::from(vector(center)),
                    mass,
                    vector![inertia.x, inertia.y, inertia.z],
                )
            }
            _ => authored_mass_properties(&shapes, mass, solid.inertia),
        };
        let angular = rotation * (local_angular_degrees * std::f32::consts::PI / 180.);
        let body = self.bodies.insert(
            RigidBodyBuilder::dynamic()
                .additional_mass_properties(masses)
                .position(pose(position, rotation))
                .linvel(vector(velocity))
                .angvel(vector![angular.x, angular.y, angular.z])
                .linear_damping(solid.damping)
                .angular_damping(solid.rotdamping),
        );
        self.ccd_gated.insert(body, 0.);
        let material = surface_material(world, &solid.surfaceprop);
        for shape in shapes {
            let builder = ColliderBuilder::new(shape).density(0.);
            let builder = if prop {
                builder
                    .contact_skin(VPHYSICS_CONTACT_MARGIN * SCALE)
                    .active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS)
            } else {
                builder.collision_groups(InteractionGroups::new(
                    PROJECTILE_GROUP,
                    Group::ALL
                        .difference(NPC_GROUP)
                        .difference(PROJECTILE_GROUP),
                ))
            };
            let builder = match material {
                Some(m) => builder
                    .friction(m.friction)
                    .restitution(restitution(m))
                    .friction_combine_rule(CoefficientCombineRule::Multiply)
                    .restitution_combine_rule(CoefficientCombineRule::Multiply),
                None => builder,
            };
            let collider = self
                .colliders
                .insert_with_parent(builder, body, &mut self.bodies);
            self.collider_materials
                .insert(collider, solid.surfaceprop.clone());
        }
        if let Some(b) = self.bodies.get_mut(body) {
            b.recompute_mass_properties_from_colliders(&self.colliders);
        }
        // The frag keeps the projected-hull fractions: its native replay
        // (owned_frag_throws_follow_native_samples) was validated with them, and the
        // stored fractions moved the plaza throw from RMS 12.8 to 40.1 (session 14)
        // while fitting the native pallet launch: an unmodelled frame difference
        // (physics-research-20261013b/drag-axes) remains open.
        body
    }
    /// CRagdollCollisionRules: every pair, or only the listed pairs without self
    /// collisions; joined pairs are excluded by their joints.
    fn ragdoll_collision_rules(
        &mut self,
        model: &modkit_core::RagdollModel,
        handles: &[RigidBodyHandle],
    ) {
        for i in 0..handles.len() {
            for j in i + 1..handles.len() {
                let listed = model
                    .collision_pairs
                    .iter()
                    .any(|&(a, b)| (a, b) == (i, j) || (a, b) == (j, i));
                if model.self_collisions || listed {
                    self.world_sides
                        .ragdoll_pairs
                        .insert((handles[i], handles[j]));
                    self.world_sides
                        .ragdoll_pairs
                        .insert((handles[j], handles[i]));
                }
            }
        }
    }
    /// RagdollCreate: one body per ragdoll solid at its bone's world pose (`poses`, one
    /// per body: RagdollAddSolid's SetPositionMatrix(pCurrentBones)), the authored mass,
    /// damping and IVP inertia, joined by ragdoll constraints (child limits about the
    /// child bone's axes relative to the reference pose, Studio_CalcBoneToBoneTransform
    /// frames; joined pairs never collide). Every body starts with `velocity`.
    pub fn spawn_ragdoll(
        &mut self,
        world: &World,
        model: &modkit_core::RagdollModel,
        poses: &[(Vec3, Quat)],
        velocity: Vec3,
        serial: u64,
    ) -> Vec<RigidBodyHandle> {
        let mut handles = Vec::with_capacity(model.bodies.len());
        for (body, &(position, rotation)) in model.bodies.iter().zip(poses) {
            let shapes: Vec<SharedShape> = body
                .pieces
                .iter()
                .filter_map(|piece| {
                    SharedShape::convex_hull(
                        &piece
                            .vertices
                            .iter()
                            .map(|p| Point::from(vector(*p)))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            let mass = body.mass.max(0.001);
            let masses = match body.compact_inertia {
                Some((center, inertia)) if inertia.is_finite() && inertia.min_element() > 0. => {
                    let inertia = inertia * mass * body.inertia;
                    MassProperties::new(
                        Point::from(vector(center)),
                        mass,
                        vector![inertia.x, inertia.y, inertia.z],
                    )
                }
                _ => authored_mass_properties(&shapes, mass, body.inertia),
            };
            let handle = self.bodies.insert(
                RigidBodyBuilder::dynamic()
                    .additional_mass_properties(masses)
                    .position(pose(position, rotation))
                    .linvel(vector(velocity))
                    .linear_damping(body.damping)
                    .angular_damping(body.rotdamping)
                    // Client ragdolls never push server props.
                    .dominance_group(-1),
            );
            // IVP deactivates a resting ragdoll moments after it lands (FX_AffectRagdolls
            // then counts it as asleep); Rapier's defaults (0.5 rad/s on every solid for
            // 2 s) never let a landed ragdoll sleep: its light limbs keep jittering at up to
            // ~1 rad/s while the root moves ~1 unit/s (session 35 hall corpses).
            let activation = self.bodies[handle].activation_mut();
            activation.angular_threshold = RAGDOLL_SLEEP_ANGULAR;
            activation.time_until_sleep = RAGDOLL_SLEEP_TIME;
            let material = surface_material(world, &body.surfaceprop);
            for shape in shapes {
                let builder = ColliderBuilder::new(shape)
                    .density(0.)
                    .collision_groups(RAGDOLL_GROUPS)
                    .contact_skin(VPHYSICS_CONTACT_MARGIN * SCALE)
                    .active_hooks(
                        ActiveHooks::FILTER_CONTACT_PAIRS | ActiveHooks::MODIFY_SOLVER_CONTACTS,
                    );
                let builder = match material {
                    Some(m) => builder
                        .friction(m.friction)
                        .restitution(restitution(m))
                        .friction_combine_rule(CoefficientCombineRule::Multiply)
                        .restitution_combine_rule(CoefficientCombineRule::Multiply),
                    None => builder,
                };
                let collider = self
                    .colliders
                    .insert_with_parent(builder, handle, &mut self.bodies);
                self.collider_materials
                    .insert(collider, body.surfaceprop.clone());
            }
            if let Some(b) = self.bodies.get_mut(handle) {
                b.recompute_mass_properties_from_colliders(&self.colliders);
            }
            self.world_sides.ragdoll_bodies.insert(handle, serial);
            handles.push(handle);
        }
        if handles.len() != model.bodies.len() {
            return handles;
        }
        self.ragdoll_collision_rules(model, &handles);
        for joint in &model.joints {
            let (translation, rotation) = joint.child_in_parent;
            let limit = |axis: usize| {
                let (min, max) = joint.limits[axis];
                [min.to_radians(), max.to_radians()]
            };
            let mut builder = GenericJointBuilder::new(JointAxesMask::LOCKED_SPHERICAL_AXES)
                .local_frame1(pose(translation, rotation))
                .local_frame2(Isometry::identity())
                .contacts_enabled(false)
                .limits(JointAxis::AngX, limit(0))
                .limits(JointAxis::AngY, limit(1))
                .limits(JointAxis::AngZ, limit(2));
            // constraint_axislimit_t torque: joint friction, x DEG2RAD (as SetAngularMotor
            // FUN_1000c820 and InitHinge FUN_1000b110). Retail InitRagdoll FUN_1000b890
            // instead puts the raw .phy friction x the reference (child) GetMass() into the
            // IVP constraint template; that template's conversion is not read, and that
            // value made kills fly 2-13% further than ours already do (session 35b), so it
            // is not used. The first animated-friction think replaces it either way.
            for (axis, friction) in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ]
                .into_iter()
                .zip(joint.friction)
            {
                let friction = friction.to_radians();
                if friction > 0. {
                    builder = builder
                        .motor_velocity(axis, 0., 1e3)
                        .motor_max_force(axis, friction);
                }
            }
            self.impulses.insert(
                handles[joint.parent],
                handles[joint.child],
                builder.build(),
                true,
            );
        }
        handles
    }
    /// RagdollCreate's nudge: the full force at the force body's center, a mass share of
    /// it at that body's position on every other body (impulses, kg in/s).
    pub fn ragdoll_nudge(&mut self, handles: &[RigidBodyHandle], force_body: usize, force: Vec3) {
        let total: f32 = handles
            .iter()
            .filter_map(|h| self.bodies.get(*h))
            .map(|b| b.mass())
            .sum::<f32>()
            .max(1.);
        let Some(at) = handles
            .get(force_body)
            .and_then(|h| self.bodies.get(*h))
            .map(|b| *b.center_of_mass())
        else {
            return;
        };
        for (i, handle) in handles.iter().enumerate() {
            let Some(body) = self.bodies.get_mut(*handle) else {
                continue;
            };
            let share = if i == force_body {
                1.
            } else {
                body.mass() / total
            };
            // kg in/s -> kg m/s.
            let f = force * share / 39.37;
            body.apply_impulse_at_point(vector![f.x, f.y, f.z], at, true);
        }
    }
    /// The first ragdoll solid along `start` -> `end` (the client's ClipRayToEntity
    /// against ragdolls; their solids stand in for the ragdoll-driven hitboxes): the
    /// body, the hit point and the fraction of the segment. Each ragdoll solid is tested
    /// directly (at most 8 ragdolls): a ragdoll built this tick is not in the query tree
    /// until the next step, and the client's effects already see it.
    pub fn ragdoll_ray(&self, start: Vec3, end: Vec3) -> Option<(RigidBodyHandle, Vec3, f32)> {
        let delta = end - start;
        let length = delta.length();
        if length < 1e-3 {
            return None;
        }
        let direction = delta / length;
        let ray = Ray::new(
            Point::from(vector(start)),
            vector![direction.x, direction.y, direction.z],
        );
        let (body, toi) = self
            .world_sides
            .ragdoll_bodies
            .keys()
            .filter_map(|handle| self.bodies.get(*handle).map(|b| (*handle, b)))
            .flat_map(|(handle, body)| body.colliders().iter().map(move |c| (handle, *c)))
            .filter_map(|(handle, collider)| {
                let collider = self.colliders.get(collider)?;
                collider
                    .shape()
                    .cast_ray(collider.position(), &ray, length * SCALE, true)
                    .map(|toi| (handle, toi))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        let distance = toi / SCALE;
        Some((body, start + direction * distance, distance / length))
    }
    /// IPhysicsObject::ApplyForceOffset (kg in/s at a world point, Source units).
    pub fn ragdoll_impulse_at(&mut self, handle: RigidBodyHandle, impulse: Vec3, point: Vec3) {
        if let Some(body) = self.bodies.get_mut(handle) {
            let f = impulse / 39.37;
            body.apply_impulse_at_point(vector![f.x, f.y, f.z], Point::from(vector(point)), true);
        }
    }
    /// IPhysicsConstraint::SetAngularMotor(0, friction) on every joint between two of these
    /// ragdoll bodies (crossbow pins excluded): retail vphysics FUN_1000c820 hands a
    /// ragdoll constraint friction x DEG2RAD (N m) to all its axes (FUN_10031810).
    pub fn ragdoll_joint_friction(&mut self, handles: &[RigidBodyHandle], friction: f32) {
        let torque = friction.abs().to_radians();
        let joints: Vec<ImpulseJointHandle> = handles
            .iter()
            .flat_map(|&body| {
                self.impulses
                    .attached_joints(body)
                    .filter(move |(_, child, _, _)| *child == body)
                    .filter(|(parent, ..)| handles.contains(parent))
                    .map(|(_, _, joint, _)| joint)
            })
            .collect();
        for handle in joints {
            if let Some(joint) = self.impulses.get_mut(handle, true) {
                for axis in [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ] {
                    joint.data.set_motor_velocity(axis, 0., 1e3);
                    joint.data.set_motor_max_force(axis, torque);
                }
            }
        }
    }
    /// Diagnostics: each awake ragdoll solid's index, speed (units/s), angular speed (rad/s)
    /// and Rapier time_since_can_sleep, fastest first.
    pub fn ragdoll_activity(&self, handles: &[RigidBodyHandle]) -> Vec<(usize, f32, f32, f32)> {
        let mut out: Vec<_> = handles
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                let b = self.bodies.get(*h)?;
                (!b.is_sleeping()).then(|| {
                    let a = b.activation();
                    (
                        i,
                        b.linvel().norm() / SCALE,
                        b.angvel().norm(),
                        a.time_since_can_sleep,
                    )
                })
            })
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1));
        out
    }
    /// CRagdoll::PhysForceRagdollToSleep: PhysForceClearVelocity and Sleep on every solid.
    pub fn ragdoll_sleep(&mut self, handles: &[RigidBodyHandle]) {
        for handle in handles {
            if let Some(body) = self.bodies.get_mut(*handle) {
                body.sleep();
            }
        }
    }
    /// IPhysicsObject::AddVelocity: linear (Source units/s) and world angular (rad/s).
    pub fn ragdoll_add_velocity(&mut self, handle: RigidBodyHandle, linear: Vec3, angular: Vec3) {
        if let Some(body) = self.bodies.get_mut(handle) {
            let v = *body.linvel() + vector(linear);
            let w = *body.angvel() + vector![angular.x, angular.y, angular.z];
            body.set_linvel(v, true);
            body.set_angvel(w, true);
        }
    }
    /// A ragdoll body's center of mass (Source units) and whether it is simulating.
    pub fn ragdoll_body_center(&self, handle: RigidBodyHandle) -> Option<(Vec3, bool)> {
        self.bodies.get(handle).map(|b| {
            let c = b.center_of_mass();
            (Vec3::new(c.x, c.y, c.z) / SCALE, !b.is_sleeping())
        })
    }
    /// CRagdollBoltEnumerator (StickRagdollNow): the struck ragdoll solid doubles its
    /// mass and gets a ball-socket constraint to the world, the world anchor at
    /// `wall` and the body anchor at `point` (where the ray met the solid). Returns the
    /// world anchor body (removed with the ragdoll).
    pub fn pin_ragdoll(
        &mut self,
        handle: RigidBodyHandle,
        wall: Vec3,
        point: Vec3,
    ) -> Option<RigidBodyHandle> {
        let body = self.bodies.get_mut(handle)?;
        let local = body
            .position()
            .inverse_transform_point(&Point::from(vector(point)));
        let props = body.mass_properties().local_mprops;
        let doubled = MassProperties::with_principal_inertia_frame(
            props.local_com,
            props.mass() * 2.,
            props.principal_inertia() * 2.,
            props.principal_inertia_local_frame,
        );
        body.set_additional_mass_properties(doubled, true);
        // The colliders carry no density: their mass is the authored additional mass.
        body.recompute_mass_properties_from_colliders(&self.colliders);
        let anchor = self
            .bodies
            .insert(RigidBodyBuilder::fixed().translation(vector(wall)));
        self.impulses.insert(
            anchor,
            handle,
            SphericalJointBuilder::new()
                .local_anchor1(Point::origin())
                .local_anchor2(local),
            true,
        );
        Some(anchor)
    }
    /// The ragdoll bodies' world poses (Source units).
    pub fn ragdoll_poses(&self, handles: &[RigidBodyHandle]) -> Option<Vec<(Vec3, Quat)>> {
        handles
            .iter()
            .map(|h| self.body_state(*h).map(|s| (s.position, s.rotation)))
            .collect()
    }
    pub fn remove_ragdoll(&mut self, handles: &[RigidBodyHandle]) {
        for handle in handles {
            self.world_sides.ragdoll_bodies.remove(handle);
            self.world_sides
                .ragdoll_pairs
                .retain(|(a, b)| a != handle && b != handle);
            self.remove_body(*handle);
        }
    }
    /// IPhysicsObject::GetContactPoint != 0: the entity's body touches something.
    pub fn entity_in_contact(&self, id: usize) -> bool {
        let Some(body) = self.dynamic.get(&id).and_then(|h| self.bodies.get(*h)) else {
            return false;
        };
        body.colliders().iter().any(|c| {
            self.narrow
                .contact_pairs_with(*c)
                .any(|pair| pair.has_any_active_contact)
        })
    }
    /// Diagnostics: colliders (entity id, or None for the world; dynamic body?) that
    /// overlap the player hull at `feet`, shrunk by half the collision epsilon like
    /// trace_hull's overlap test.
    pub fn hull_contacts(&self, feet: Vec3, maxs_z: f32) -> Vec<(Option<usize>, bool, f32, Vec3)> {
        let half = Vec3::new(16., 16., maxs_z * 0.5) - Vec3::splat(0.015625);
        let position = pose(feet + Vec3::Z * maxs_z * 0.5, Quat::IDENTITY);
        let mut found = Vec::new();
        self.query.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &position,
            &Cuboid::new(vector(half)),
            groups_filter(PLAYER_GROUPS),
            |handle| {
                let c = &self.colliders[handle];
                // Penetration depth (units) and normal from the hull into the collider.
                let (depth, normal) = rapier3d::parry::query::contact(
                    &position,
                    &Cuboid::new(vector(half)),
                    c.position(),
                    c.shape(),
                    0.,
                )
                .ok()
                .flatten()
                .map_or((0., Vec3::ZERO), |k| {
                    (
                        -k.dist / SCALE,
                        Vec3::new(k.normal1.x, k.normal1.y, k.normal1.z),
                    )
                });
                found.push((
                    (c.user_data as usize).checked_sub(1),
                    c.parent()
                        .and_then(|b| self.bodies.get(b))
                        .is_some_and(|b| b.is_dynamic()),
                    depth,
                    normal,
                ));
                true
            },
        );
        found
    }
    /// The body touches a fixed body (world, static prop) or one heavier than the push
    /// mass limit on its far side along `direction` (its contact normal points within
    /// ~45 degrees of it). The player shadow's own contacts do not count.
    fn blocked_along(&self, body: RigidBodyHandle, direction: Vec3) -> bool {
        let Some(b) = self.bodies.get(body) else {
            return false;
        };
        b.colliders().iter().any(|collider| {
            self.narrow.contact_pairs_with(*collider).any(|pair| {
                if !pair.has_any_active_contact
                    || self.world_sides.shadow.contains(&pair.collider1)
                    || self.world_sides.shadow.contains(&pair.collider2)
                {
                    return false;
                }
                let (sign, other) = if pair.collider1 == *collider {
                    (1., pair.collider2)
                } else {
                    (-1., pair.collider1)
                };
                let immovable = self
                    .colliders
                    .get(other)
                    .and_then(|c| c.parent())
                    .and_then(|h| self.bodies.get(h))
                    .is_none_or(|o| !o.is_dynamic() || o.mass() > PLAYER_PUSH_MASS_LIMIT);
                immovable
                    && pair.manifolds.iter().any(|m| {
                        let n = m.data.normal * sign;
                        !m.data.solver_contacts.is_empty()
                            && Vec3::new(n.x, n.y, n.z).dot(direction) > 0.7
                    })
            })
        })
    }
    /// Teleport an entity's dynamic body (test fixtures): Source angles, world
    /// velocity, no spin. False when the entity has no dynamic body.
    pub fn place_body(
        &mut self,
        id: usize,
        origin: Vec3,
        angles_deg: Vec3,
        velocity: Vec3,
    ) -> bool {
        let Some(body) = self.dynamic.get(&id).and_then(|h| self.bodies.get_mut(*h)) else {
            return false;
        };
        body.set_position(pose(origin, angles(angles_deg)), true);
        body.set_linvel(vector(velocity), true);
        body.set_angvel(vector![0., 0., 0.], true);
        true
    }
    /// Diagnostics: the player VPhysics shadow's center.
    pub fn player_shadow_center(&self) -> Option<Vec3> {
        let body = self.bodies.get(self.player_shadow?.0)?;
        let t = *body.translation() / SCALE;
        Some(Vec3::new(t.x, t.y, t.z))
    }
    /// Give a projectile body an entity id (`projectile_entity`): traces report it,
    /// and the +USE / gravity gun holds find it through `dynamic`.
    pub fn bind_projectile_entity(&mut self, handle: RigidBodyHandle, id: usize) {
        let Some(body) = self.bodies.get(handle) else {
            return;
        };
        let colliders = body.colliders().to_vec();
        for collider in &colliders {
            if let Some(c) = self.colliders.get_mut(*collider) {
                c.user_data = id as u128 + 1;
            }
        }
        if let Some(surface) = colliders
            .first()
            .and_then(|c| self.collider_materials.get(c))
        {
            self.prop_materials.insert(id, surface.clone());
        }
        self.entity_colliders.insert(id, colliders);
        self.dynamic.insert(id, handle);
    }
    /// COLLISION_GROUP_DEBRIS -> COLLISION_GROUP_INTERACTIVE_DEBRIS when the player
    /// picks up, punts or holds the entity (other groups are left alone).
    /// CBreakableProp::StickAtPosition: teleport to `position`, EnableMotion(false),
    /// SF_PHYSPROP_ENABLE_ON_PHYSCANNON and COLLISION_GROUP_DEBRIS; `save` is the
    /// position before the embed, where EnableMotion puts it back.
    pub fn stick(&mut self, id: usize, position: Vec3, save: Vec3) {
        let Some(&handle) = self.dynamic.get(&id) else {
            return;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return;
        };
        let mut pose = *body.position();
        let p = position * SCALE;
        pose.translation = vector![p.x, p.y, p.z].into();
        body.set_body_type(RigidBodyType::Fixed, true);
        body.set_position(pose, true);
        body.set_linvel(vector![0., 0., 0.], false);
        body.set_angvel(vector![0., 0., 0.], false);
        for collider in body.colliders().to_vec() {
            if let Some(c) = self.colliders.get_mut(collider) {
                c.set_collision_groups(DEBRIS_GROUPS);
            }
        }
        self.interactive_debris.remove(&handle);
        self.stuck.insert(id, save);
    }
    /// Physgun_OnPhysGunPickup (gravity gun pickups and punts, +USE pickups): held
    /// debris becomes interactive debris, and a stuck prop
    /// (SF_PHYSPROP_ENABLE_ON_PHYSCANNON) moves again (CPhysicsProp::EnableMotion: back
    /// at its enable-motion position, out of the wall, before motion resumes).
    pub fn make_interactive_debris(&mut self, id: usize) {
        let Some(&handle) = self.dynamic.get(&id) else {
            return;
        };
        if let Some(save) = self.stuck.remove(&id) {
            if let Some(body) = self.bodies.get_mut(handle) {
                let mut pose = *body.position();
                let p = save * SCALE;
                pose.translation = vector![p.x, p.y, p.z].into();
                body.set_body_type(RigidBodyType::Dynamic, true);
                body.set_position(pose, true);
            }
        }
        let Some(body) = self.bodies.get(handle) else {
            return;
        };
        let mut changed = false;
        for collider in body.colliders().to_vec() {
            if let Some(c) = self.colliders.get_mut(collider) {
                if c.collision_groups() == DEBRIS_GROUPS {
                    c.set_collision_groups(INTERACTIVE_DEBRIS_GROUPS);
                    changed = true;
                }
            }
        }
        if changed {
            self.interactive_debris.insert(handle);
        }
    }
    pub fn is_interactive_debris(&self, id: usize) -> bool {
        self.dynamic
            .get(&id)
            .is_some_and(|h| self.interactive_debris.contains(h))
    }
    /// Interactive debris converts back to debris when it comes to rest.
    fn settle_interactive_debris(&mut self) {
        if self.interactive_debris.is_empty() {
            return;
        }
        let bodies = &self.bodies;
        let asleep: Vec<RigidBodyHandle> = self
            .interactive_debris
            .iter()
            .copied()
            .filter(|h| bodies.get(*h).is_none_or(|b| b.is_sleeping()))
            .collect();
        for handle in asleep {
            self.interactive_debris.remove(&handle);
            let Some(body) = self.bodies.get(handle) else {
                continue;
            };
            for collider in body.colliders().to_vec() {
                if let Some(c) = self.colliders.get_mut(collider) {
                    if c.collision_groups() == INTERACTIVE_DEBRIS_GROUPS {
                        c.set_collision_groups(DEBRIS_GROUPS);
                    }
                }
            }
        }
    }
    /// COLLISION_GROUP_WEAPON for a dropped weapon's body (`WEAPON_GROUPS`).
    pub fn set_weapon_group(&mut self, id: usize) {
        let Some(body) = self.dynamic.get(&id).and_then(|h| self.bodies.get(*h)) else {
            return;
        };
        for collider in body.colliders().to_vec() {
            if let Some(c) = self.colliders.get_mut(collider) {
                c.set_collision_groups(WEAPON_GROUPS);
            }
        }
    }
    /// COLLISION_GROUP_DEBRIS for a runtime prop body (a debris gib): it keeps colliding
    /// with the world and ordinary props only.
    pub fn set_body_debris(&mut self, handle: RigidBodyHandle) {
        let Some(body) = self.bodies.get(handle) else {
            return;
        };
        for collider in body.colliders().to_vec() {
            if let Some(c) = self.colliders.get_mut(collider) {
                c.set_collision_groups(DEBRIS_GROUPS);
            }
        }
    }
    pub fn remove_body(&mut self, handle: RigidBodyHandle) {
        let removed: Vec<ColliderHandle> = self
            .bodies
            .get(handle)
            .map(|body| body.colliders().to_vec())
            .unwrap_or_default();
        for collider in &removed {
            self.collider_materials.remove(collider);
        }
        if let Some(id) = self
            .dynamic
            .iter()
            .find(|(id, h)| **h == handle && is_projectile_entity(**id))
            .map(|(id, _)| *id)
        {
            self.dynamic.remove(&id);
            self.entity_colliders.remove(&id);
            self.prop_materials.remove(&id);
        }
        self.drag_fractions.remove(&handle);
        self.drag_geometry.remove(&handle);
        self.ccd_gated.remove(&handle);
        self.bodies.remove(
            handle,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulses,
            &mut self.multibody,
            true,
        );
        // Queries between now and the next step must not reach the removed colliders
        // (a faded gib panicked a later ray in the query pipeline).
        if !removed.is_empty() {
            self.query
                .update_incremental(&self.colliders, &[], &removed, true);
        }
    }
    /// Position, rotation, velocity (units/s) and local angular velocity (deg/s).
    /// CBasePlayer::PostThinkVPhysics / UpdateVPhysicsPosition for one tick: the 85 kg
    /// shadow box follows the player's hull. When a grounded player is blocked by a
    /// physics object they do not stand on, its target moves halfway toward where they
    /// wanted to go (old feet + wish velocity x dt), so the shadow shoves the object
    /// (the contact filter keeps it to movable bodies <= 350 kg). Retail contact-plane
    /// limits: at most 50 u/s into a pushable object's contact normal, 0 into a heavy or
    /// blocked one, also checked ahead of contact by a sweep. A step up teleports the
    /// shadow. Not done: the retail two/multi-normal constraint cases, bounded per-axis
    /// corrections, weighing light supports down, the shadow pushing the player back
    /// (VPhysicsShadowUpdate), Jump.
    #[allow(clippy::too_many_arguments)]
    pub fn update_player_shadow(
        &mut self,
        old_feet: Vec3,
        feet: Vec3,
        crouched: bool,
        grounded: bool,
        step_height: f32,
        wish_velocity: Vec3,
        held: Option<usize>,
        dt: f32,
    ) {
        if !feet.is_finite() || !old_feet.is_finite() || !wish_velocity.is_finite() || dt <= 0. {
            return;
        }
        // VEC_HULL / VEC_DUCK_HULL; the body origin sits at the hull center.
        let half = Vec3::new(16., 16., if crouched { 18. } else { 36. });
        // The collider is the hull at its sides and top; props keep their
        // VPHYSICS_CONTACT_MARGIN skin off it, so props resting against the shadow stay
        // outside the player's movement hull (without a margin they sat inside Rapier's
        // contact tolerance and player traces started solid). Its bottom is raised by
        // that margin so it never presses a supporting prop into the floor.
        let skin = Vec3::new(half.x, half.y, half.z - VPHYSICS_CONTACT_MARGIN * 0.5);
        let skin_offset = Vec3::Z * (VPHYSICS_CONTACT_MARGIN * 0.5);
        let center = |feet: Vec3| feet + Vec3::Z * half.z;
        let (handle, current) = match self.player_shadow {
            Some((handle, hull)) if self.bodies.contains(handle) => (handle, hull),
            _ => {
                // Retail shadow: a dynamic object of mass 85 with ~infinite inertia, no
                // gravity and no drag, driven by the controller's velocity corrections.
                let body = self.bodies.insert(
                    RigidBodyBuilder::dynamic()
                        .position(pose(center(feet), Quat::IDENTITY))
                        .lock_rotations()
                        .gravity_scale(0.)
                        .additional_mass(PLAYER_SHADOW_MASS)
                        .ccd_enabled(true),
                );
                let collider = self.colliders.insert_with_parent(
                    ColliderBuilder::cuboid(skin.x * SCALE, skin.y * SCALE, skin.z * SCALE)
                        .translation(vector(skin_offset))
                        .density(0.)
                        .friction(0.8)
                        .active_hooks(ActiveHooks::FILTER_CONTACT_PAIRS)
                        .collision_groups(InteractionGroups::new(
                            SHADOW_GROUP,
                            Group::ALL
                                .difference(PROJECTILE_GROUP)
                                .difference(NPC_GROUP)
                                .difference(SHADOW_GROUP),
                        )),
                    body,
                    &mut self.bodies,
                );
                self.world_sides.shadow = vec![collider];
                self.drag_disabled.insert(body);
                self.player_shadow = Some((body, crouched));
                (body, crouched)
            }
        };
        if current != crouched {
            for collider in &self.world_sides.shadow {
                if let Some(c) = self.colliders.get_mut(*collider) {
                    c.set_shape(SharedShape::cuboid(
                        skin.x * SCALE,
                        skin.y * SCALE,
                        skin.z * SCALE,
                    ));
                }
            }
            self.player_shadow = Some((handle, crouched));
        }
        let excluded = held.and_then(|id| self.dynamic.get(&id).copied());
        // Bodies under the feet (GetGroundVPhysics) and in the way of the wish move
        // (m_touchedPhysObject), from short hull sweeps against movable bodies.
        let movable = |physics: &Self, from: Vec3, to: Vec3| -> Option<RigidBodyHandle> {
            let shape = Cuboid::new(vector![half.x * SCALE, half.y * SCALE, half.z * SCALE]);
            let filter = QueryFilter::default()
                .groups(QUERY_GROUPS)
                .exclude_sensors();
            let (collider, _) = physics.query.cast_shape(
                &physics.bodies,
                &physics.colliders,
                &pose(center(from), Quat::IDENTITY),
                &vector(to - from),
                &shape,
                ShapeCastOptions::with_max_time_of_impact(1.),
                filter,
            )?;
            let body = physics.colliders.get(collider)?.parent()?;
            physics
                .bodies
                .get(body)
                .filter(|b| b.is_dynamic() && Some(body) != excluded && body != handle)
                .map(|_| body)
        };
        let ground = if grounded {
            movable(self, feet + Vec3::Z * 0.5, feet - Vec3::Z * 2.)
        } else {
            None
        };
        let planar = Vec3::new(wish_velocity.x, wish_velocity.y, 0.);
        // m_outStepHeight: this move stepped up (StepMove).
        let step = step_height;
        let touched = grounded
            && step <= 0.
            && ground.is_none()
            && planar.length_squared() > 1.
            && movable(
                self,
                feet + Vec3::Z,
                feet + Vec3::Z + planar.normalize() * (planar.length() * dt + 1.),
            )
            .is_some();
        let mut target = center(feet);
        if touched {
            target = center(feet) * 0.5 + center(old_feet + planar * dt) * 0.5;
        }
        self.world_sides.shadow_excluded = excluded.into_iter().collect();
        // The controller's contact-plane limits (retail player controller callback):
        // into each touching object's contact normal the shadow may move at most the
        // push speed limit (50) when the object is movable within the push mass limit,
        // otherwise not at all. Normals are clipped one after another; the retail
        // two-normal (cross product) and over-two cases are not reproduced.
        let mut limits = Vec::new();
        for collider in &self.world_sides.shadow {
            for pair in self.narrow.contact_pairs_with(*collider) {
                if !pair.has_any_active_contact {
                    continue;
                }
                let (sign, other) = if pair.collider1 == *collider {
                    (1., pair.collider2)
                } else {
                    (-1., pair.collider1)
                };
                let Some(other) = self.colliders.get(other).and_then(|c| c.parent()) else {
                    continue;
                };
                let pushable = self
                    .bodies
                    .get(other)
                    .is_some_and(|b| b.is_dynamic() && b.mass() <= PLAYER_PUSH_MASS_LIMIT);
                for manifold in &pair.manifolds {
                    if manifold.data.solver_contacts.is_empty() {
                        continue;
                    }
                    let n = manifold.data.normal * sign;
                    let n = Vec3::new(n.x, n.y, n.z);
                    // A body pressed against something fixed (or too heavy) behind it
                    // along the push cannot respond: no push speed (shoving it into a
                    // wall left it inside the player's hull).
                    let limit = if pushable && !self.blocked_along(other, n) {
                        PLAYER_PUSH_SPEED_LIMIT
                    } else {
                        0.
                    };
                    limits.push((n, limit));
                }
            }
        }
        let Some(t) = self.bodies.get(handle).map(|b| *b.translation() / SCALE) else {
            return;
        };
        let mut position = Vec3::new(t.x, t.y, t.z);
        if step > 0.1 && position.distance(target) <= 64. {
            // The game code teleports the shadow up by the step height (SetPosition
            // above 4 units, IPhysicsPlayerController::StepUp otherwise) instead of
            // driving it through the stepped-on object.
            position.z = target.z;
        }
        let velocity = if position.distance(target) > 64. {
            position = target;
            Vec3::ZERO
        } else {
            let mut velocity = (target - position) / dt;
            for (normal, limit) in &limits {
                let into = velocity.dot(*normal);
                if into > *limit {
                    velocity -= *normal * (into - limit);
                }
            }
            // The same limits ahead of contact: the shadow follows a falling or
            // walking player several units per tick, so a first contact found only
            // after the step was already deep (a landing rammed a 5 kg box ~1.7 units
            // into the floor, and its rebound filled the player's hull).
            let shape = Cuboid::new(vector(skin));
            let shadow_colliders = self.world_sides.shadow.clone();
            let skip = |c: ColliderHandle, collider: &Collider| {
                !shadow_colliders.contains(&c)
                    && collider.parent().is_some_and(|h| {
                        Some(h) != excluded && self.bodies.get(h).is_some_and(|b| b.is_dynamic())
                    })
            };
            let filter = QueryFilter::default()
                .groups(InteractionGroups::new(
                    SHADOW_GROUP,
                    Group::ALL
                        .difference(PROJECTILE_GROUP)
                        .difference(NPC_GROUP)
                        .difference(SHADOW_GROUP)
                        .difference(RAGDOLL_GROUP),
                ))
                .exclude_sensors()
                .predicate(&skip);
            let delta = velocity * dt;
            if let Some((collider, hit)) = self.query.cast_shape(
                &self.bodies,
                &self.colliders,
                &pose(position + skin_offset, Quat::IDENTITY),
                &vector(delta),
                &shape,
                ShapeCastOptions::with_max_time_of_impact(1.),
                filter,
            ) {
                let n = Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z);
                let other = self.colliders.get(collider).and_then(|c| c.parent());
                let pushable = other
                    .and_then(|b| self.bodies.get(b))
                    .is_some_and(|b| b.mass() <= PLAYER_PUSH_MASS_LIMIT)
                    && other.is_some_and(|b| !self.blocked_along(b, n));
                let limit = if pushable {
                    PLAYER_PUSH_SPEED_LIMIT
                } else {
                    0.
                };
                let into = velocity.dot(n);
                let allowed = (delta * hit.time_of_impact).dot(n) / dt + limit;
                if into > allowed {
                    velocity -= n * (into - allowed.max(0.));
                }
            }
            velocity
        };
        if let Some(body) = self.bodies.get_mut(handle) {
            body.set_translation(vector(position), true);
            body.set_linvel(vector(velocity), true);
        }
        // No explicit weight on movable ground: the retail shadow has no gravity (the
        // controller's Update clears it) and light supports are not rideable. An 85 kg
        // impulse here sank a 5 kg box ~1.5 units into the floor and its rebound filled
        // the player's hull (stuck on top of props). Weighing light props down stays
        // open (lead: the controller's support-contact case, normal Z < -0.7).
    }
    pub fn body_state(&self, handle: RigidBodyHandle) -> Option<BodyState> {
        let b = self.bodies.get(handle)?;
        let p = b.position();
        let q = p.rotation.quaternion();
        let rotation = Quat::from_xyzw(q.i, q.j, q.k, q.w);
        let w = b.angvel();
        let local = rotation.inverse() * Vec3::new(w.x, w.y, w.z);
        let v = b.linvel();
        Some(BodyState {
            position: Vec3::new(p.translation.x, p.translation.y, p.translation.z) / SCALE,
            rotation,
            velocity: Vec3::new(v.x, v.y, v.z) / SCALE,
            local_angular_degrees: local * 180. / std::f32::consts::PI,
        })
    }
    /// IPhysicsObject::SetVelocity with world velocity and local angular deg/s.
    pub fn set_body_velocity(
        &mut self,
        handle: RigidBodyHandle,
        velocity: Vec3,
        local_angular_degrees: Vec3,
    ) {
        if let Some(b) = self.bodies.get_mut(handle) {
            let q = b.position().rotation.quaternion();
            let rotation = Quat::from_xyzw(q.i, q.j, q.k, q.w);
            let w = rotation * (local_angular_degrees * std::f32::consts::PI / 180.);
            b.set_linvel(vector(velocity), true);
            b.set_angvel(vector![w.x, w.y, w.z], true);
        }
    }
    /// The size of the world-space box around the entity's colliders (an approximation
    /// of its OBB size for CEntityFlame's m_flSize).
    pub fn entity_extents(&self, id: usize) -> Option<Vec3> {
        let colliders = self.entity_colliders.get(&id)?;
        let mut aabb: Option<rapier3d::parry::bounding_volume::Aabb> = None;
        for c in colliders {
            if let Some(collider) = self.colliders.get(*c) {
                let b = collider.compute_aabb();
                aabb = Some(match aabb {
                    Some(a) => a.merged(&b),
                    None => b,
                });
            }
        }
        let e = aabb?.extents();
        Some(Vec3::new(e.x, e.y, e.z) / SCALE)
    }
    /// The world-space box around the entity's colliders (units).
    pub fn entity_world_bounds(&self, id: usize) -> Option<(Vec3, Vec3)> {
        let colliders = self.entity_colliders.get(&id)?;
        let mut aabb: Option<rapier3d::parry::bounding_volume::Aabb> = None;
        for c in colliders {
            if let Some(collider) = self.colliders.get(*c) {
                let b = collider.compute_aabb();
                aabb = Some(match aabb {
                    Some(a) => a.merged(&b),
                    None => b,
                });
            }
        }
        let a = aabb?;
        Some((
            Vec3::new(a.mins.x, a.mins.y, a.mins.z) / SCALE,
            Vec3::new(a.maxs.x, a.maxs.y, a.maxs.z) / SCALE,
        ))
    }
    /// The entity's VPhysics body (map prop or bound runtime body).
    pub fn entity_body(&self, id: usize) -> Option<RigidBodyHandle> {
        self.dynamic.get(&id).copied()
    }
    /// World center of mass (units); the body's linear velocity is the velocity there.
    pub fn center_of_mass(&self, handle: RigidBodyHandle) -> Option<Vec3> {
        let c = self.bodies.get(handle)?.center_of_mass();
        Some(Vec3::new(c.x, c.y, c.z) / SCALE)
    }
    /// CCollisionProperty::WorldSpaceCenter of a VPhysics body: the center of its
    /// collision bounds in body space (the OBB), placed at the body's pose.
    pub fn bounds_center(&self, handle: RigidBodyHandle) -> Option<Vec3> {
        let body = self.bodies.get(handle)?;
        let local = body
            .colliders()
            .iter()
            .filter_map(|c| self.colliders.get(*c))
            .filter_map(|c| {
                let pose = c.position_wrt_parent().copied()?;
                Some(c.shape().compute_aabb(&pose))
            })
            .reduce(|a, b| a.merged(&b))?;
        let c = body.position() * local.center();
        Some(Vec3::new(c.x, c.y, c.z) / SCALE)
    }
    /// Mass (kg) of an entity's dynamic VPhysics body.
    pub fn entity_mass(&self, id: usize) -> Option<f32> {
        Some(self.bodies.get(*self.dynamic.get(&id)?)?.mass())
    }
    /// The entity's body mass (kg) and whether it is asleep and moveable (IsAsleep, IsMoveable:
    /// simulated, not motion-disabled), for CZombieSwatEntitiesEnum.
    pub fn entity_rest_state(&self, id: usize) -> Option<(f32, bool, bool)> {
        let body = self.bodies.get(*self.dynamic.get(&id)?)?;
        Some((
            body.mass(),
            body.is_sleeping(),
            body.is_dynamic() && body.is_enabled(),
        ))
    }
    /// IPhysicsObject::AddVelocity: world velocity (units/s) and local angular velocity (deg/s)
    /// added to the entity's body, which wakes.
    pub fn add_entity_velocity(&mut self, id: usize, velocity: Vec3, local_angular_degrees: Vec3) {
        let Some(handle) = self.dynamic.get(&id).copied() else {
            return;
        };
        if let Some(b) = self.bodies.get_mut(handle) {
            let q = b.position().rotation.quaternion();
            let rotation = Quat::from_xyzw(q.i, q.j, q.k, q.w);
            let w = rotation * (local_angular_degrees * std::f32::consts::PI / 180.);
            let linvel = *b.linvel() + vector(velocity);
            let angvel = *b.angvel() + vector![w.x, w.y, w.z];
            b.set_linvel(linvel, true);
            b.set_angvel(angvel, true);
        }
    }
    pub fn entity_pose(&self, id: usize) -> Option<(Vec3, Quat)> {
        let b = self.bodies.get(*self.dynamic.get(&id)?)?;
        let p = b.position();
        let q = p.rotation.quaternion();
        Some((
            Vec3::new(p.translation.x, p.translation.y, p.translation.z) / SCALE,
            Quat::from_xyzw(q.i, q.j, q.k, q.w),
        ))
    }
    /// IPhysicsObject::ApplyForceOffset: an impulse (kg m/s, world) at a world point
    /// (units) on the entity's body.
    pub fn impulse_at(&mut self, id: usize, impulse: Vec3, point: Vec3) {
        if let Some(b) = self.dynamic.get(&id).and_then(|h| self.bodies.get_mut(*h)) {
            let p = vector(point);
            b.apply_impulse_at_point(
                vector![impulse.x, impulse.y, impulse.z],
                Point::from(p),
                true,
            );
        }
    }
    pub fn impulse(&mut self, id: usize, direction: Vec3, strength: f32) {
        if let Some(h) = self.dynamic.get(&id) {
            if let Some(b) = self.bodies.get_mut(*h) {
                b.apply_impulse(
                    vector![
                        direction.x * strength,
                        direction.y * strength,
                        direction.z * strength
                    ],
                    true,
                );
            }
        }
    }
    /// Continuous projectile query against enabled solid geometry. These are
    /// Rapier shape casts, not VPhysics contacts or complete Source group rules.
    pub fn projectile_sweep(
        &self,
        start: Vec3,
        end: Vec3,
        hull: ProjectileHull,
        excluded: &[usize],
    ) -> Option<ProjectileHit> {
        if !start.is_finite() || !end.is_finite() {
            return None;
        }
        let shape = match hull {
            ProjectileHull::Box(half) if half.is_finite() && half.min_element() > 0. => {
                SharedShape::cuboid(half.x * SCALE, half.y * SCALE, half.z * SCALE)
            }
            ProjectileHull::Sphere(radius) if radius.is_finite() && radius > 0. => {
                SharedShape::ball(radius * SCALE)
            }
            _ => return None,
        };
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let entity = (collider.user_data as usize).wrapping_sub(1);
            collider.is_enabled()
                && !excluded.contains(&entity)
                && self
                    .world_brush_contents
                    .get(&handle)
                    .is_none_or(|contents| contents & 0x0200_400b != 0)
        };
        let (handle, hit) = self.query.cast_shape(
            &self.bodies,
            &self.colliders,
            &pose(start, Quat::IDENTITY),
            &vector(end - start),
            shape.as_ref(),
            ShapeCastOptions {
                max_time_of_impact: 1.,
                target_distance: 0.,
                stop_at_penetration: true,
                compute_impact_geometry_on_penetration: true,
            },
            QueryFilter::default()
                .groups(QUERY_GROUPS)
                .exclude_sensors()
                .predicate(&predicate),
        )?;
        Some(ProjectileHit {
            entity: (self.colliders[handle].user_data as usize).wrapping_sub(1),
            position: start.lerp(end, hit.time_of_impact),
            normal: Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z),
            fraction: hit.time_of_impact,
        })
    }
    /// Solid-mask visibility used by local projectile damage, excluding the
    /// damage receiver. Player/NPC-only clip brushes do not block this trace.
    pub fn projectile_ray(&self, start: Vec3, end: Vec3, excluded: &[usize]) -> Option<RayHit> {
        self.filtered_ray(start, end, &|entity| !excluded.contains(&entity))
    }
    /// `projectile_ray` that only tests colliders whose entity passes `keep`.
    pub fn filtered_ray(
        &self,
        start: Vec3,
        end: Vec3,
        keep: &dyn Fn(usize) -> bool,
    ) -> Option<RayHit> {
        self.masked_ray(start, end, 0x0200_400b, keep)
    }
    /// Closest collision point on an entity, for radius attenuation. Models
    /// without a collider are handled explicitly by the caller.
    pub fn entity_closest_point(&self, id: usize, point: Vec3) -> Option<Vec3> {
        let point = Point::from(vector(point));
        let hitboxes: Vec<ColliderHandle> = self
            .studio_hitboxes
            .get(&id)
            .map(|set| set.boxes.iter().map(|b| b.0).collect())
            .unwrap_or_default();
        self.entity_colliders
            .get(&id)
            .filter(|c| !c.is_empty())
            .or((!hitboxes.is_empty()).then_some(&hitboxes))?
            .iter()
            .filter_map(|handle| self.colliders.get(*handle))
            .filter(|collider| collider.is_enabled())
            .map(|collider| {
                collider
                    .shape()
                    .project_point(collider.position(), &point, true)
                    .point
            })
            .min_by(|a, b| {
                (a - point)
                    .norm_squared()
                    .total_cmp(&(b - point).norm_squared())
            })
            .map(|p| Vec3::new(p.x, p.y, p.z) / SCALE)
    }
    /// Symmetric box sweep for melee's secondary trace. Geometry still uses Rapier shapes.
    pub fn impact_hull(
        &self,
        origin: Vec3,
        direction: Vec3,
        distance: f32,
        half: Vec3,
    ) -> Option<RayHit> {
        self.hull_with(origin, direction, distance, half, QUERY_GROUPS, false)
    }
    fn hull_with(
        &self,
        origin: Vec3,
        direction: Vec3,
        distance: f32,
        half: Vec3,
        groups: InteractionGroups,
        clips: bool,
    ) -> Option<RayHit> {
        if distance <= 0. || direction.length_squared() < 1e-8 {
            return None;
        }
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            collider.is_enabled() && (clips || !self.world_sides.clip_brushes.contains(&handle))
        };
        let shape = Cuboid::new(vector(half));
        let options = ShapeCastOptions {
            max_time_of_impact: 1.,
            target_distance: 0.,
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: true,
        };
        let (handle, hit) = self.query.cast_shape(
            &self.bodies,
            &self.colliders,
            &pose(origin, Quat::IDENTITY),
            &vector(direction * distance),
            &shape,
            options,
            groups_filter(groups).predicate(&predicate),
        )?;
        Some(RayHit {
            entity: (self.colliders[handle].user_data as usize).wrapping_sub(1),
            position: origin + direction * distance * hit.time_of_impact,
            normal: Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z),
            hitgroup: self.hitgroup(handle),
            bone: self.hitbox_bone(handle),
        })
    }
}
impl CollisionWorld for Physics {
    fn trace_hull(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> Trace {
        let half = (maxs - mins) * 0.5;
        let center = (maxs + mins) * 0.5;
        let delta = end - start;
        let stationary = delta.length_squared() < 0.000001;
        // World brushes use their real planes below. Retain Rapier colliders for
        // weapons/rigid bodies, but do not approximate these player normals with GJK.
        let player_contents_filter = |handle: ColliderHandle, collider: &Collider| {
            collider.is_enabled()
                && !self.world_brush_contents.contains_key(&handle)
                && !self.npc_hitboxes.contains(&handle)
                && !player_convex::supported(collider)
        };
        let filter = QueryFilter::default()
            .groups(PLAYER_GROUPS)
            .exclude_sensors()
            .predicate(&player_contents_filter);
        // A standing hull merely touching the floor must not prevent uncrouching.
        // Shrink a stationary overlap probe by half the Source collision epsilon.
        let shape = Cuboid::new(vector(half));
        let overlap_shape = Cuboid::new(vector(
            (half - Vec3::splat(0.015625)).max(Vec3::splat(0.001)),
        ));
        let position = Isometry::translation(
            (start.x + center.x) * SCALE,
            (start.y + center.y) * SCALE,
            (start.z + center.z) * SCALE,
        );
        let mut result = trace_brushes(
            &self.player_world_brushes,
            start,
            end,
            mins,
            maxs,
            PLAYER_BRUSH_MASK,
        );
        // Continuous SAT for cuboids/convex polyhedra includes their face planes,
        // player box face axes, and every convex-edge/box-edge bevel axis. Cache
        // planes by current shape/pose and only query broadphase candidates.
        let margin = Vec3::splat(0.03125);
        let bounds = rapier3d::parry::bounding_volume::Aabb::new(
            Point::from(vector(start.min(end) + mins - margin)),
            Point::from(vector(start.max(end) + maxs + margin)),
        );
        let mut cache = self
            .player_convex_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.query
            .colliders_with_aabb_intersecting_aabb(&bounds, |handle| {
                let collider = &self.colliders[*handle];
                if collider.is_enabled()
                    && !collider.is_sensor()
                    && !matches!(
                        collider.collision_groups().memberships,
                        PROJECTILE_GROUP
                            | SHADOW_GROUP
                            | DEBRIS_GROUP
                            | INTERACTIVE_DEBRIS_GROUP
                            | RAGDOLL_GROUP
                            | WEAPON_GROUP
                    )
                    && !self.world_brush_contents.contains_key(handle)
                    && !self.npc_hitboxes.contains(handle)
                {
                    if let Some(brush) = cache.brush(*handle, collider) {
                        let hit =
                            trace_brushes(std::slice::from_ref(brush), start, end, mins, maxs, 1);
                        result.start_solid |= hit.start_solid;
                        result.all_solid |= hit.all_solid;
                        if hit.fraction < result.fraction {
                            result.fraction = hit.fraction;
                            result.normal = hit.normal;
                        }
                    }
                }
                true
            });
        drop(cache);
        let mut overlapping = Vec::new();
        self.query.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &position,
            &overlap_shape,
            filter,
            |handle| {
                overlapping.push(handle);
                true
            },
        );
        result.start_solid |= !overlapping.is_empty();
        // The overlap region of a convex collider and a translating hull is
        // convex: overlapping both endpoints proves this entire segment is solid.
        // The same inference is invalid for concave meshes, whose sweep remains
        // handled by Parry rather than inventing an all-solid result.
        let end_position = Isometry::translation(
            (end.x + center.x) * SCALE,
            (end.y + center.y) * SCALE,
            (end.z + center.z) * SCALE,
        );
        result.all_solid |= overlapping.iter().any(|handle| {
            let collider = &self.colliders[*handle];
            stationary
                || (collider.shape().is_convex()
                    && rapier3d::parry::query::intersection_test(
                        collider.position(),
                        collider.shape(),
                        &end_position,
                        &overlap_shape,
                    )
                    .unwrap_or(false))
        });
        if result.all_solid {
            result.fraction = 0.;
        }
        if stationary || result.all_solid {
            return result;
        }
        let options = ShapeCastOptions {
            max_time_of_impact: 1.,
            target_distance: 0.03125 * SCALE,
            stop_at_penetration: false,
            compute_impact_geometry_on_penetration: true,
        };
        if let Some((_, hit)) = self.query.cast_shape(
            &self.bodies,
            &self.colliders,
            &position,
            &vector(delta),
            &shape,
            options,
            filter,
        ) {
            let n = hit.normal1;
            let fraction = hit.time_of_impact.clamp(0., 1.);
            if fraction < result.fraction {
                result.fraction = fraction;
                result.normal = Vec3::new(n.x, n.y, n.z);
            }
        }
        result
    }
}

/// A body's angular speed (degrees per second, in its principal frame) for an angular
/// velocity `w`, and the summed |I_k w_k| over its principal axes.
fn angular_motion(body: &RigidBody, w: &Vector<Real>) -> (f32, f32) {
    let props = body.mass_properties().local_mprops;
    let frame = body.rotation() * props.principal_inertia_local_frame;
    let local = frame.inverse_transform_vector(w) * (180. / std::f32::consts::PI);
    let s = props.inv_principal_inertia_sqrt;
    let inertia = |k: f32| if k > 0. { 1. / (k * k) } else { 0. };
    let momentum = (inertia(s.x) * local.x).abs()
        + (inertia(s.y) * local.y).abs()
        + (inertia(s.z) * local.z).abs();
    (local.norm(), momentum)
}

#[cfg(test)]
mod tests;
