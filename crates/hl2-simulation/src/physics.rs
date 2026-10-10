//! Rapier collision/rigid-body adapter. This is not Valve's proprietary VPhysics solver.
use crate::npc_probe::{ActorHull, Hull, HullTrace, NpcCollisionWorld, Query, CONTENTS_MONSTER};
use glam::{Mat4, Quat, Vec3};
use modkit_core::{movement::CollisionWorld, trace_brushes, Brush, Surface, Trace, World};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
#[path = "player_convex.rs"]
mod player_convex;
#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub entity: usize,
    pub position: Vec3,
    pub normal: Vec3,
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
// Retail CGameMovement::PlayerSolidMask(true): NPC-only clip volumes do not
// block players. Keep their colliders available for other query categories.
const PLAYER_BRUSH_MASK: u32 = 0x1400b;
/// VPhysics projectiles (thrown frags): COLLISION_GROUP_WEAPON-like bodies that touch
/// the world and props but not characters, and that traces/player movement ignore.
const PROJECTILE_GROUP: Group = Group::GROUP_2;
/// Colliders of npc_* model instances.
const NPC_GROUP: Group = Group::GROUP_3;
const QUERY_GROUPS: InteractionGroups =
    InteractionGroups::new(Group::ALL, Group::ALL.difference(PROJECTILE_GROUP));
fn enabled_filter() -> QueryFilter<'static> {
    static ENABLED: fn(ColliderHandle, &Collider) -> bool = |_, c| c.is_enabled();
    QueryFilter::default()
        .groups(QUERY_GROUPS)
        .exclude_sensors()
        .predicate(&ENABLED)
}
fn vector(p: Vec3) -> Vector<Real> {
    vector![p.x * SCALE, p.y * SCALE, p.z * SCALE]
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
        SharedShape::trimesh_with_flags(
            vertices,
            indices,
            TriMeshFlags::MERGE_DUPLICATE_VERTICES | TriMeshFlags::FIX_INTERNAL_EDGES,
        )
        .ok()
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
fn world_material(world: &World) -> impl Fn(ColliderBuilder) -> ColliderBuilder + '_ {
    let material = world.surface_materials.get("default");
    move |builder| match material {
        Some(m) => builder
            .friction(m.friction)
            .restitution(m.elasticity)
            .friction_combine_rule(CoefficientCombineRule::Multiply)
            .restitution_combine_rule(CoefficientCombineRule::Multiply),
        None => builder.friction(0.8),
    }
}
/// The body's mass properties from the authored total mass: uniform density over
/// the convex pieces, then the whole inertia tensor times the solid's "inertia"
/// multiplier (VPhysics scales the object's tensor, parallel-axis terms included).
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
/// Frag fit of the VPhysics air drag (projectiles.rs FRAG_AIR_DRAG), per unit.
pub const AIR_DRAG: f32 = 6.4e-4;
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
    touching: std::collections::HashSet<(ColliderHandle, ColliderHandle)>,
    pair_times: HashMap<(ColliderHandle, ColliderHandle), f64>,
    /// Simulated seconds (sum of tick dt).
    pub time: f64,
    /// dv/dt = -air_drag |v| v for awake dynamic bodies (0 disables).
    pub air_drag: f32,
    pub native_shape_count: usize,
    pub native_shape_fallbacks: usize,
    pub skipped: usize,
}
impl Physics {
    pub fn new(world: &World) -> Self {
        let mut p = Self {
            player_world_brushes: world.brushes.clone(),
            air_drag: AIR_DRAG,
            ..Self::default()
        };
        // World brushes carry no per-side material here, so they use the
        // "default" surfaceprop (friction/elasticity) when the table is loaded.
        let world_material = world_material(world);
        for brush in &world.brushes {
            if let Some(shape) = brush_shape(brush) {
                let collider = p
                    .colliders
                    .insert(world_material(ColliderBuilder::new(shape)));
                p.world_brush_contents.insert(collider, brush.contents);
                p.npc_collider_contents.insert(collider, brush.contents);
            } else {
                p.skipped += 1;
            }
        }
        for surface in &world.terrain {
            if let Some(shape) = mesh_shape(std::slice::from_ref(surface), false, None) {
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
            if instance.background {
                continue;
            }
            if !instance.solid {
                continue;
            }
            let Some(asset) = world.model_assets.get(&instance.asset_key()) else {
                continue;
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
            let native_eligible = (instance.kind == "static_prop"
                && instance.solid_mode == Some(6))
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
                p.skipped += 1;
                continue;
            }
            // Scaled props need separately scaled meshes; skip their collider rather than use a wrong size.
            if (instance.scale - 1.).abs() > 0.001 {
                p.skipped += 1;
                continue;
            }
            p.native_shape_count += usize::from(cached.native) * cached.shapes.len();
            p.native_shape_fallbacks += usize::from(cached.native_failed);
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
                p.bodies.insert(
                    RigidBodyBuilder::dynamic()
                        .additional_mass_properties(masses.unwrap_or_default())
                        .position(position)
                        .sleeping(prop_flags & 1 != 0)
                        .ccd_enabled(true)
                        .linear_damping(linear_damping)
                        .angular_damping(angular_damping),
                )
            });
            if let (Some(id), Some(body)) = (entity, body) {
                p.dynamic.insert(id, body);
            }
            let material = solid
                .as_ref()
                .and_then(|s| surface_material(world, &s.surfaceprop));
            if let (Some(id), Some(s)) = (entity, &solid) {
                p.prop_materials.insert(id, s.surfaceprop.clone());
            }
            let surfaceprop = solid.as_ref().map(|s| s.surfaceprop.clone());
            for shape in &cached.shapes {
                let collider = match material {
                    // VPhysics multiplies both surfaces' friction and elasticity
                    // (frag fit: grenade 0.9 x concrete 0.8). Rapier restitution is
                    // not IVP elasticity: "native fit needed".
                    Some(m) => ColliderBuilder::new(shape.clone())
                        .friction(m.friction)
                        .restitution(m.elasticity)
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
                        };
                        p.colliders
                            .insert_with_parent(collider, body, &mut p.bodies)
                    }
                    None => p.colliders.insert(collider.position(position)),
                };
                if let Some(id) = entity {
                    p.entity_colliders.entry(id).or_default().push(c);
                }
                if let Some(name) = &surfaceprop {
                    p.collider_materials.insert(c, name.clone());
                }
                if instance.kind.starts_with("npc_") {
                    p.colliders[c]
                        .set_collision_groups(InteractionGroups::new(NPC_GROUP, Group::ALL));
                }
                p.npc_collider_contents.insert(
                    c,
                    if instance.kind.starts_with("npc_") {
                        CONTENTS_MONSTER
                    } else {
                        1
                    },
                );
            }
        }
        // Fold additional (authored) mass properties in before the first query.
        for handle in p.dynamic.values() {
            if let Some(body) = p.bodies.get_mut(*handle) {
                body.recompute_mass_properties_from_colliders(&p.colliders);
            }
        }
        p.query.update(&p.colliders);
        p
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
        self.player_convex_cache
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(&self.colliders);
        // VPhysics air drag on moving objects, dv/dt = -c |v| v (units). The drag
        // model is in vphysics.dll; c is the frag fit and applies to every body until
        // per-object fits exist ("native fit needed").
        let mut before = HashMap::new();
        for (handle, body) in self.bodies.iter_mut() {
            if !body.is_dynamic() || body.is_sleeping() || !body.is_enabled() {
                continue;
            }
            let v = *body.linvel();
            let speed = v.norm() / SCALE;
            if self.air_drag > 0. && speed > 0. {
                body.set_linvel(v * (1. - self.air_drag * speed * dt).max(0.), false);
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
            &(),
            &(),
        );
        self.time += f64::from(dt);
        self.collect_collisions(&before);
        self.changed_entity_colliders.clear();
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
            if before
                .get(&c1.parent().unwrap_or(RigidBodyHandle::invalid()))
                .is_none()
                && before
                    .get(&c2.parent().unwrap_or(RigidBodyHandle::invalid()))
                    .is_none()
            {
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
            self.collisions.push(PhysicsCollision {
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
        // Forget pair times of colliders that no longer exist.
        if self.pair_times.len() > 4096 {
            let colliders = &self.colliders;
            self.pair_times
                .retain(|(a, b), _| colliders.contains(*a) && colliders.contains(*b));
        }
        self.touching = touching;
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
                        SharedShape::convex_hull(
                            &piece
                                .vertices
                                .iter()
                                .map(|point| Point::from(vector(*point)))
                                .collect::<Vec<_>>(),
                        )
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
        let masses = authored_mass_properties(&shapes, solid.mass.max(0.001), solid.inertia);
        let angular = rotation * (local_angular_degrees * std::f32::consts::PI / 180.);
        let body = self.bodies.insert(
            RigidBodyBuilder::dynamic()
                .additional_mass_properties(masses)
                .position(pose(position, rotation))
                .linvel(vector(velocity))
                .angvel(vector![angular.x, angular.y, angular.z])
                .ccd_enabled(true)
                .linear_damping(solid.damping)
                .angular_damping(solid.rotdamping),
        );
        let material = surface_material(world, &solid.surfaceprop);
        for shape in shapes {
            let builder =
                ColliderBuilder::new(shape)
                    .density(0.)
                    .collision_groups(InteractionGroups::new(
                        PROJECTILE_GROUP,
                        Group::ALL
                            .difference(NPC_GROUP)
                            .difference(PROJECTILE_GROUP),
                    ));
            let builder = match material {
                Some(m) => builder
                    .friction(m.friction)
                    .restitution(m.elasticity)
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
        body
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
    pub fn remove_body(&mut self, handle: RigidBodyHandle) {
        if let Some(body) = self.bodies.get(handle) {
            for collider in body.colliders() {
                self.collider_materials.remove(collider);
            }
        }
        self.bodies.remove(
            handle,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulses,
            &mut self.multibody,
            true,
        );
    }
    /// Position, rotation, velocity (units/s) and local angular velocity (deg/s).
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
    pub fn entity_pose(&self, id: usize) -> Option<(Vec3, Quat)> {
        let b = self.bodies.get(*self.dynamic.get(&id)?)?;
        let p = b.position();
        let q = p.rotation.quaternion();
        Some((
            Vec3::new(p.translation.x, p.translation.y, p.translation.z) / SCALE,
            Quat::from_xyzw(q.i, q.j, q.k, q.w),
        ))
    }
    pub fn ray(&self, origin: Vec3, direction: Vec3, distance: f32) -> Option<(usize, Vec3)> {
        self.impact_ray(origin, direction, distance)
            .map(|h| (h.entity, h.position))
    }
    pub fn impact_ray(&self, origin: Vec3, direction: Vec3, distance: f32) -> Option<RayHit> {
        let ray = Ray::new(
            Point::from(vector(origin)),
            vector![direction.x, direction.y, direction.z],
        );
        let (h, hit) = self.query.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            distance * SCALE,
            true,
            enabled_filter(),
        )?;
        let id = self.colliders[h].user_data as usize;
        Some(RayHit {
            entity: id.wrapping_sub(1),
            position: origin + direction * (hit.time_of_impact / SCALE),
            normal: Vec3::new(hit.normal.x, hit.normal.y, hit.normal.z),
        })
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
        let delta = end - start;
        let length = delta.length();
        if !length.is_finite() || length <= 1e-5 {
            return None;
        }
        let direction = delta / length;
        let predicate = |handle: ColliderHandle, collider: &Collider| {
            let entity = (collider.user_data as usize).wrapping_sub(1);
            collider.is_enabled()
                && !excluded.contains(&entity)
                && self
                    .world_brush_contents
                    .get(&handle)
                    .is_none_or(|contents| contents & 0x0200_400b != 0)
        };
        let (handle, hit) = self.query.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &Ray::new(
                Point::from(vector(start)),
                vector![direction.x, direction.y, direction.z],
            ),
            length * SCALE,
            true,
            QueryFilter::default()
                .groups(QUERY_GROUPS)
                .exclude_sensors()
                .predicate(&predicate),
        )?;
        Some(RayHit {
            entity: (self.colliders[handle].user_data as usize).wrapping_sub(1),
            position: start + direction * (hit.time_of_impact / SCALE),
            normal: Vec3::new(hit.normal.x, hit.normal.y, hit.normal.z),
        })
    }
    /// Closest collision point on an entity, for radius attenuation. Models
    /// without a collider are handled explicitly by the caller.
    pub fn entity_closest_point(&self, id: usize, point: Vec3) -> Option<Vec3> {
        let point = Point::from(vector(point));
        self.entity_colliders
            .get(&id)?
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
        if distance <= 0. || direction.length_squared() < 1e-8 {
            return None;
        }
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
            enabled_filter(),
        )?;
        Some(RayHit {
            entity: (self.colliders[handle].user_data as usize).wrapping_sub(1),
            position: origin + direction * distance * hit.time_of_impact,
            normal: Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z),
        })
    }
}
impl NpcCollisionWorld for Physics {
    fn npc_trace_hull(&self, start: Vec3, end: Vec3, hull: Hull, query: Query<'_>) -> HullTrace {
        if !start.is_finite()
            || !end.is_finite()
            || !hull.valid()
            || !(start + hull.mins).is_finite()
            || !(end + hull.maxs).is_finite()
            || query
                .transients
                .iter()
                .any(|actor| !actor.feet.is_finite() || !actor.hull.valid())
        {
            return HullTrace::invalid();
        }
        let entity_of = |collider: &Collider| {
            usize::try_from(collider.user_data)
                .ok()
                .and_then(|id| id.checked_sub(1))
        };
        let accepts = |handle: ColliderHandle, collider: &Collider| {
            let entity = entity_of(collider);
            collider.is_enabled()
                && !collider.is_sensor()
                && collider.collision_groups().memberships != PROJECTILE_GROUP
                && entity.is_none_or(|id| !query.excluded_entities.contains(&id))
                // Explicit actor bounds supersede an NPC's render-mesh collider.
                && !entity.is_some_and(|id| {
                    self.npc_collider_contents.get(&handle) == Some(&CONTENTS_MONSTER)
                        && query.transients.iter().any(|a| a.entity == Some(id))
                })
                && self
                    .npc_collider_contents
                    .get(&handle)
                    .copied()
                    .unwrap_or(1)
                    & query.contents_mask
                    != 0
        };
        let fallback = |handle: ColliderHandle, collider: &Collider| {
            accepts(handle, collider)
                && !self.world_brush_contents.contains_key(&handle)
                && !player_convex::supported(collider)
        };
        let filter = QueryFilter::default()
            .groups(QUERY_GROUPS)
            .exclude_sensors()
            .predicate(&fallback);
        let mut result = HullTrace {
            trace: trace_brushes(
                &self.player_world_brushes,
                start,
                end,
                hull.mins,
                hull.maxs,
                query.contents_mask,
            ),
            ..HullTrace::clear()
        };
        let mut merge = |hit: Trace, entity: Option<usize>, transient: bool| {
            if hit.fraction < result.trace.fraction
                || (hit.start_solid && !result.trace.start_solid)
            {
                result.entity = entity;
                result.transient = transient;
                result.trace.normal = hit.normal;
            }
            result.trace.fraction = result.trace.fraction.min(hit.fraction);
            result.trace.start_solid |= hit.start_solid;
            result.trace.all_solid |= hit.all_solid;
        };
        if query.contents_mask & CONTENTS_MONSTER != 0 {
            for ActorHull {
                entity,
                feet,
                hull: actor_hull,
            } in query.transients
            {
                if entity.is_some_and(|id| query.excluded_entities.contains(&id)) {
                    continue;
                }
                let min = *feet + actor_hull.mins;
                let max = *feet + actor_hull.maxs;
                let brush = Brush {
                    contents: CONTENTS_MONSTER,
                    planes: [
                        (Vec3::X, max.x),
                        (-Vec3::X, -min.x),
                        (Vec3::Y, max.y),
                        (-Vec3::Y, -min.y),
                        (Vec3::Z, max.z),
                        (-Vec3::Z, -min.z),
                    ]
                    .into_iter()
                    .map(|(normal, distance)| modkit_core::Plane { normal, distance })
                    .collect(),
                };
                merge(
                    trace_brushes(
                        std::slice::from_ref(&brush),
                        start,
                        end,
                        hull.mins,
                        hull.maxs,
                        query.contents_mask,
                    ),
                    *entity,
                    true,
                );
            }
        }
        let margin = Vec3::splat(0.03125);
        let bounds = rapier3d::parry::bounding_volume::Aabb::new(
            Point::from(vector(start.min(end) + hull.mins - margin)),
            Point::from(vector(start.max(end) + hull.maxs + margin)),
        );
        let mut cache = self
            .player_convex_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.query
            .colliders_with_aabb_intersecting_aabb(&bounds, |handle| {
                let collider = &self.colliders[*handle];
                if accepts(*handle, collider) && !self.world_brush_contents.contains_key(handle) {
                    if let Some(brush) = cache.brush(*handle, collider) {
                        merge(
                            trace_brushes(
                                std::slice::from_ref(brush),
                                start,
                                end,
                                hull.mins,
                                hull.maxs,
                                1,
                            ),
                            entity_of(collider),
                            false,
                        );
                    }
                }
                true
            });
        drop(cache);
        let center = (hull.maxs + hull.mins) * 0.5;
        let half = (hull.maxs - hull.mins) * 0.5;
        let position = pose(start + center, Quat::IDENTITY);
        let end_position = pose(end + center, Quat::IDENTITY);
        let stationary = (end - start).length_squared() < 0.000001;
        // Stand probes have zero-height contact hulls. Only the Parry fallback
        // needs a tiny thickness; brush/convex planes keep the exact bounds.
        let shape = Cuboid::new(vector(half.max(Vec3::splat(0.001))));
        let overlap_shape = Cuboid::new(vector(
            (half - Vec3::splat(0.015625)).max(Vec3::splat(0.001)),
        ));
        self.query.intersections_with_shape(
            &self.bodies,
            &self.colliders,
            &position,
            &overlap_shape,
            filter,
            |handle| {
                let collider = &self.colliders[handle];
                let all_solid = stationary
                    || (collider.shape().is_convex()
                        && rapier3d::parry::query::intersection_test(
                            collider.position(),
                            collider.shape(),
                            &end_position,
                            &overlap_shape,
                        )
                        .unwrap_or(false));
                merge(
                    Trace {
                        fraction: if all_solid { 0. } else { 1. },
                        normal: Vec3::ZERO,
                        start_solid: true,
                        all_solid,
                    },
                    entity_of(collider),
                    false,
                );
                true
            },
        );
        if !stationary && !result.trace.all_solid {
            let options = ShapeCastOptions {
                max_time_of_impact: 1.,
                target_distance: 0.03125 * SCALE,
                stop_at_penetration: false,
                compute_impact_geometry_on_penetration: true,
            };
            if let Some((handle, hit)) = self.query.cast_shape(
                &self.bodies,
                &self.colliders,
                &position,
                &vector(end - start),
                &shape,
                options,
                filter,
            ) {
                let fraction = hit.time_of_impact.clamp(0., 1.);
                if fraction < result.trace.fraction {
                    result.trace.fraction = fraction;
                    result.trace.normal = Vec3::new(hit.normal1.x, hit.normal1.y, hit.normal1.z);
                    result.entity = entity_of(&self.colliders[handle]);
                    result.transient = false;
                }
            }
        }
        if result.trace.all_solid {
            result.trace.fraction = 0.;
        }
        result
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
                && !player_convex::supported(collider)
        };
        let filter = QueryFilter::default()
            .groups(QUERY_GROUPS)
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
                    && collider.collision_groups().memberships != PROJECTILE_GROUP
                    && !self.world_brush_contents.contains_key(handle)
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

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::movement::{Input, Player, TICK};
    fn box_surface(min: Vec3, max: Vec3) -> Surface {
        let vertices = (0..8)
            .map(|i| modkit_core::Vertex {
                normal: Default::default(),
                position: Vec3::new(
                    if i & 1 == 0 { min.x } else { max.x },
                    if i & 2 == 0 { min.y } else { max.y },
                    if i & 4 == 0 { min.z } else { max.z },
                ),
                uv: glam::Vec2::ZERO,
                light_uv: glam::Vec2::ZERO,
                color: [255; 4],
                skin: Some(modkit_core::animation::Weights {
                    bones: [0; 3],
                    weights: [1., 0., 0.],
                }),
            })
            .collect();
        Surface {
            flex_source: None,
            background: false,
            material: "synthetic".into(),
            lightmap: None,
            vertices,
            indices: vec![
                0, 2, 3, 0, 3, 1, 4, 5, 7, 4, 7, 6, 0, 1, 5, 0, 5, 4, 2, 6, 7, 2, 7, 3, 0, 4, 6, 0,
                6, 2, 1, 3, 7, 1, 7, 5,
            ],
        }
    }
    fn door_world() -> World {
        use modkit_core::animation::{Bone, Clip, Pose, Rig};
        let instance = modkit_core::ModelInstance {
            background: false,
            model: "synthetic_door.mdl".into(),
            origin: Vec3::ZERO,
            angles: Vec3::ZERO,
            skin: 0,
            scale: 1.,
            kind: "prop_door_rotating".into(),
            solid: true,
            solid_mode: None,
            entity: Some(0),
        };
        let idle = Pose {
            position: Vec3::new(-1., -1., 0.),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        };
        let key = instance.asset_key();
        World {
            entities: vec![modkit_core::Entity {
                properties: vec![("classname".into(), "prop_door_rotating".into())],
            }],
            model_assets: BTreeMap::from([(
                key.clone(),
                vec![box_surface(
                    Vec3::new(0., -2., -54.),
                    Vec3::new(48., 0., 54.),
                )],
            )]),
            rigs: BTreeMap::from([(
                key,
                Rig {
                    pose_parameters: Vec::new(),
                    autoplay: Vec::new(),
                    attachments: Vec::new(),
                    bones: vec![Bone {
                        name: "door".into(),
                        parent: None,
                        bind: Pose {
                            position: Vec3::ZERO,
                            rotation: Quat::IDENTITY,
                        },
                        inverse_bind: Mat4::IDENTITY,
                    }],
                    clips: BTreeMap::from([(
                        "idle".into(),
                        Clip {
                            layer: Default::default(),
                            events: Vec::new(),
                            fps: 1.,
                            looping: true,
                            frames: vec![vec![idle.clone()], vec![idle]],
                        },
                    )]),
                    warnings: Vec::new(),
                    sequences: Vec::new(),
                },
            )]),
            model_instances: vec![instance],
            ..Default::default()
        }
    }
    #[test]
    fn rotating_door_collision_matches_idle_panel_before_and_after_entity_swing() {
        let world = door_world();
        let mut physics = Physics::new(&world);
        let start = Vec3::new(-40., 24., -54.);
        let end = Vec3::new(40., 24., -54.);
        let mins = Vec3::new(-16., -16., 0.);
        let maxs = Vec3::new(16., 16., 72.);
        let closed = physics.trace_hull(start, end, mins, maxs);
        assert!(closed.fraction > 0. && closed.fraction < 0.5);
        assert!(!closed.start_solid);
        assert_eq!(
            physics
                .impact_ray(start + Vec3::Z * 54., Vec3::X, 80.)
                .unwrap()
                .entity,
            0
        );
        // The historical mesh collider blocked the open-side panel instead.
        let mut raw = world.clone();
        raw.rigs.clear();
        let raw = Physics::new(&raw);
        assert_eq!(raw.trace_hull(start, end, mins, maxs).fraction, 1.);
        physics.set_entity(
            0,
            Vec3::ZERO,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            true,
        );
        physics.tick(TICK);
        assert_eq!(physics.trace_hull(start, end, mins, maxs).fraction, 1.);
        assert!(physics
            .impact_ray(start + Vec3::Z * 54., Vec3::X, 80.)
            .is_none());
        // Returning to closed must move queries back, including weapon rays.
        physics.set_entity(0, Vec3::ZERO, Quat::IDENTITY, true);
        physics.tick(TICK);
        assert!(
            (physics.trace_hull(start, end, mins, maxs).fraction - closed.fraction).abs() < 0.00001
        );
    }
    #[test]
    fn rotating_door_pose_cache_does_not_change_other_instances_of_the_same_model() {
        let mut world = door_world();
        let mut static_prop = world.model_instances[0].clone();
        static_prop.kind = "static_prop".into();
        static_prop.entity = None;
        static_prop.origin = Vec3::Z * 200.;
        world.model_instances.push(static_prop);
        let physics = Physics::new(&world);
        // A regular prop with this asset keeps the raw model geometry.
        assert!(physics
            .impact_ray(Vec3::new(24., -40., 200.), Vec3::Y, 80.)
            .is_some());
        assert!(physics
            .impact_ray(Vec3::new(-40., 24., 200.), Vec3::X, 80.)
            .is_none());
        assert_eq!(physics.colliders.len(), 2);
    }
    #[test]
    fn incremental_entity_queries_match_full_rebuild_across_moves_rotations_and_visibility() {
        let mut incremental = Physics::new(&door_world());
        let mut rebuilt = Physics::new(&door_world());
        for i in 0..80 {
            let origin = Vec3::new((i % 5) as f32 * 30., (i % 3) as f32 * -20., 0.);
            let rotation = angles(Vec3::new(0., (i % 4) as f32 * 45., 0.));
            let enabled = i % 7 != 0;
            incremental.set_entity(0, origin, rotation, enabled);
            // Repeating an identical entity pose must preserve the result without marking it again.
            incremental.set_entity(0, origin, rotation, enabled);
            rebuilt.set_entity(0, origin, rotation, enabled);
            incremental.refresh_entity_queries();
            rebuilt.refresh_queries();
            for y in [-80., -20., 24., 60., 100.] {
                let start = Vec3::new(-100., y, 54.);
                let end = Vec3::new(250., y, 54.);
                let a = incremental.projectile_sweep(start, end, ProjectileHull::Sphere(3.), &[]);
                let b = rebuilt.projectile_sweep(start, end, ProjectileHull::Sphere(3.), &[]);
                assert_eq!(a.is_some(), b.is_some(), "pose {i}, ray {y}");
                if let (Some(a), Some(b)) = (a, b) {
                    assert_eq!(a.entity, b.entity);
                    assert!((a.fraction - b.fraction).abs() < 1e-6);
                    assert!(a.normal.distance(b.normal) < 1e-5);
                }
            }
        }
    }
    #[test]
    fn projectile_queries_follow_scripted_door_poses_without_advancing_simulation() {
        let world = door_world();
        let mut physics = Physics::new(&world);
        let start = Vec3::new(60., 24., 54.);
        let end = Vec3::new(140., 24., 54.);
        assert!(physics
            .projectile_sweep(start, end, ProjectileHull::Sphere(10.), &[])
            .is_none());
        physics.set_entity(0, Vec3::X * 100., Quat::IDENTITY, true);
        physics.refresh_queries();
        let hit = physics
            .projectile_sweep(start, end, ProjectileHull::Sphere(10.), &[])
            .unwrap();
        assert_eq!(hit.entity, 0);
        assert!(hit.position.x > 88. && hit.position.x < 92.);
        assert!(physics.projectile_ray(start, end, &[]).is_some());
        assert!(physics.projectile_ray(start, end, &[0]).is_none());
        let closest = physics.entity_closest_point(0, start).unwrap();
        assert!((closest.x - 99.).abs() < 0.01);
        physics.set_entity(0, Vec3::X * 100., Quat::IDENTITY, false);
        physics.refresh_queries();
        assert!(physics
            .projectile_sweep(start, end, ProjectileHull::Box(Vec3::splat(3.)), &[])
            .is_none());
        assert!(physics.entity_closest_point(0, start).is_none());
    }
    fn native_prop_world() -> World {
        let instance = modkit_core::ModelInstance {
            background: false,
            model: "synthetic_separated_prop.mdl".into(),
            origin: Vec3::new(100., 200., 20.),
            angles: Vec3::new(0., 90., 0.),
            skin: 0,
            scale: 1.,
            kind: "static_prop".into(),
            solid: true,
            solid_mode: Some(6),
            entity: None,
        };
        let piece = |min, max| {
            let surface = box_surface(min, max);
            modkit_core::ConvexPiece {
                vertices: surface.vertices.iter().map(|v| v.position).collect(),
                indices: surface.indices.as_chunks::<3>().0.to_vec(),
            }
        };
        let key = instance.asset_key();
        World {
            model_assets: BTreeMap::from([(
                key.clone(),
                vec![box_surface(
                    Vec3::new(-20., -20., 0.),
                    Vec3::new(20., 20., 40.),
                )],
            )]),
            model_collision: BTreeMap::from([(
                key,
                vec![
                    piece(Vec3::new(-20., -20., 0.), Vec3::new(-5., 20., 40.)),
                    piece(Vec3::new(5., -20., 0.), Vec3::new(20., 20., 40.)),
                ],
            )]),
            model_instances: vec![instance],
            ..Default::default()
        }
    }
    #[test]
    fn native_convex_prop_pieces_preserve_rotated_gaps_and_weapon_rays() {
        let world = native_prop_world();
        let physics = Physics::new(&world);
        assert_eq!(physics.colliders.len(), 2);
        assert_eq!(physics.native_shape_count, 2);
        assert_eq!(physics.native_shape_fallbacks, 0);
        assert!(physics
            .colliders
            .iter()
            .all(|(_, c)| player_convex::supported(c)));
        let origin = world.model_instances[0].origin;
        let center = origin + Vec3::Z * 10.;
        let mins = Vec3::new(-1., -1., 0.);
        let maxs = Vec3::new(1., 1., 2.);
        // Rotating the local X gap by90degrees makes a world X corridor.
        let start = center - Vec3::X * 40.;
        let end = center + Vec3::X * 40.;
        let through_gap = physics.trace_hull(start, end, mins, maxs);
        assert_eq!(through_gap.fraction, 1.);
        assert!(!through_gap.start_solid);
        assert!(physics.impact_ray(start, Vec3::X, 80.).is_none());
        let across = physics.trace_hull(center - Vec3::Y * 40., center + Vec3::Y * 40., mins, maxs);
        assert!(across.fraction > 0. && across.fraction < 0.5);
        assert!(across.normal.distance(-Vec3::Y) < 0.00001);
        assert!(physics
            .impact_ray(start + Vec3::Y * 12., Vec3::X, 80.)
            .is_some());
        // Confirm this tests native decomposition rather than the broader
        // synthetic render mesh, which fills the gap.
        let mut fallback = world;
        fallback.model_collision.clear();
        assert!(
            Physics::new(&fallback)
                .trace_hull(start, end, mins, maxs)
                .fraction
                < 1.
        );
    }
    #[test]
    fn invalid_native_piece_falls_back_as_a_whole_and_dynamic_props_keep_prior_policy() {
        let mut world = native_prop_world();
        let key = world.model_instances[0].asset_key();
        world.model_collision.get_mut(&key).unwrap()[1]
            .vertices
            .clear();
        let fallback = Physics::new(&world);
        assert_eq!(fallback.colliders.len(), 1);
        assert_eq!(fallback.native_shape_count, 0);
        assert_eq!(fallback.native_shape_fallbacks, 1);
        // Dynamic prop fallback remains one body and one convex render hull.
        world = native_prop_world();
        world.model_instances[0].kind = "prop_physics".into();
        let dynamic = Physics::new(&world);
        assert_eq!(dynamic.colliders.len(), 1);
        assert_eq!(dynamic.bodies.len(), 1);
        assert_eq!(dynamic.native_shape_count, 0);
        assert_eq!(dynamic.native_shape_fallbacks, 0);
    }
    #[test]
    fn authored_vphysics_props_use_phy_mass_pieces_damping_and_surface_material() {
        // Synthetic parameters (not read from a game file).
        let mut world = native_prop_world();
        world.model_instances[0].kind = "prop_physics".into();
        world.model_instances[0].entity = Some(0);
        world.entities.push(modkit_core::Entity {
            properties: vec![("classname".into(), "prop_physics".into())],
        });
        let key = world.model_instances[0].asset_key();
        world.model_physics.insert(
            key,
            modkit_core::PhysicsSolid {
                mass: 35.,
                damping: 0.,
                rotdamping: 0.2,
                inertia: 2.,
                surfaceprop: "synthetic_wood".into(),
                ..Default::default()
            },
        );
        world.surface_materials.insert(
            "synthetic_wood".into(),
            modkit_core::SurfaceMaterial {
                friction: 0.6,
                elasticity: 0.3,
                ..Default::default()
            },
        );
        let physics = Physics::new(&world);
        assert_eq!(
            physics.native_shape_count, 2,
            "authored pieces, not the hull"
        );
        let body = &physics.bodies[physics.dynamic[&0]];
        assert!((body.mass() - 35.).abs() < 1e-3, "{}", body.mass());
        assert_eq!(body.linear_damping(), 0.);
        assert_eq!(body.angular_damping(), 0.2);
        assert_eq!(physics.prop_materials[&0], "synthetic_wood");
        for (_, collider) in physics.colliders.iter() {
            assert_eq!(collider.friction(), 0.6);
            assert_eq!(collider.restitution(), 0.3);
            assert_eq!(
                collider.friction_combine_rule(),
                CoefficientCombineRule::Multiply
            );
        }
        let inertia = body.mass_properties().local_mprops.principal_inertia();
        // massScale doubles the mass; inertiaScale 0.1 clamps to 0.5 x the authored 2.
        world.entities[0]
            .properties
            .push(("massScale".into(), "2".into()));
        world.entities[0]
            .properties
            .push(("inertiaScale".into(), "0.1".into()));
        let scaled = Physics::new(&world);
        let body = &scaled.bodies[scaled.dynamic[&0]];
        assert!((body.mass() - 70.).abs() < 1e-3);
        let scaled_inertia = body.mass_properties().local_mprops.principal_inertia();
        // 2x mass and a 0.5 / 2 inertia multiplier: half the tensor.
        assert!((scaled_inertia - inertia * 0.5).norm() < inertia.norm() * 1e-3);
    }
    #[test]
    fn free_bodies_report_new_contacts_once_and_feel_air_drag() {
        // Synthetic floor and solid (not game data).
        let mut world = World {
            brushes: vec![bounds_brush(
                Vec3::new(-2000., -2000., -16.),
                Vec3::new(2000., 2000., 0.),
            )],
            ..Default::default()
        };
        world.model_physics.insert(
            "synthetic#0".into(),
            modkit_core::PhysicsSolid {
                mass: 5.,
                damping: 0.,
                rotdamping: 0.,
                surfaceprop: "synthetic_metal".into(),
                ..Default::default()
            },
        );
        let mut physics = Physics::new(&world);
        assert_eq!(physics.air_drag, AIR_DRAG);
        let body = physics.add_projectile_body(
            &world,
            "synthetic#0",
            Vec3::splat(4.),
            Vec3::new(0., 0., 1000.),
            Quat::IDENTITY,
            Vec3::new(1000., 0., 0.),
            Vec3::ZERO,
        );
        physics.tick(0.015);
        let v = physics.body_state(body).unwrap().velocity;
        // dv/dt = -c |v| v over one tick (gravity is vertical; |v| ~ 1000).
        let expected = 1000. * (1. - AIR_DRAG * 1000. * 0.015);
        assert!((v.x - expected).abs() < 0.5, "{v} vs {expected}");
        // Drop onto the floor: one event at the fall speed, then resting is silent.
        physics.air_drag = 0.;
        physics.set_body_velocity(body, Vec3::ZERO, Vec3::ZERO);
        if let Some(b) = physics.bodies.get_mut(body) {
            b.set_translation(vector(Vec3::new(0., 0., 40.)), true);
        }
        let mut events = vec![];
        for _ in 0..120 {
            physics.tick(0.015);
            events.extend(physics.collisions.iter().cloned());
        }
        assert!(!events.is_empty());
        let first = &events[0];
        let fall = (2. * 600. * 36f32).sqrt();
        assert!(
            (first.speed - fall).abs() < 25.,
            "{} vs {fall}",
            first.speed
        );
        assert_eq!(first.delta_time, f32::MAX);
        assert!(first.surfaceprops.contains(&Some("synthetic_metal".into())));
        assert!(first.entities.iter().all(Option::is_none));
        // Any later contact events are small settling contacts, not new impacts.
        assert!(events[1..].iter().all(|e| e.speed < 70.), "{events:?}");
        let rest = physics.body_state(body).unwrap().position;
        assert!((rest.z - 4.).abs() < 1., "{rest}");
    }
    #[test]
    fn static_prop_bbox_and_unknown_modes_do_not_reuse_native_hulls() {
        for mode in [None, Some(2)] {
            let mut world = native_prop_world();
            world.model_instances[0].solid_mode = mode;
            let physics = Physics::new(&world);
            assert_eq!(physics.colliders.len(), 1);
            assert_eq!(physics.native_shape_count, 0);
            assert_eq!(physics.native_shape_fallbacks, 0);
            let center = world.model_instances[0].origin + Vec3::Z * 10.;
            let hit = physics.trace_hull(
                center - Vec3::X * 40.,
                center + Vec3::X * 40.,
                Vec3::new(-1., -1., 0.),
                Vec3::new(1., 1., 2.),
            );
            assert!(
                hit.fraction < 1.,
                "mode {mode:?} must use the complete render fallback"
            );
            // Mixed instances of one model must not share collision policy
            // merely because its native model pieces were already cached.
            world = native_prop_world();
            let mut other = world.model_instances[0].clone();
            other.solid_mode = mode;
            other.origin.z += 200.;
            world.model_instances.push(other);
            let mixed = Physics::new(&world);
            assert_eq!(mixed.colliders.len(), 3);
            assert_eq!(mixed.native_shape_count, 2);
            assert!(mixed
                .impact_ray(center - Vec3::X * 40., Vec3::X, 80.)
                .is_none());
            assert!(mixed
                .impact_ray(center + Vec3::Z * 200. - Vec3::X * 40., Vec3::X, 80.)
                .is_some());
        }
    }
    fn bounds_brush(min: Vec3, max: Vec3) -> Brush {
        Brush {
            contents: 1,
            planes: [
                (Vec3::X, max.x),
                (-Vec3::X, -min.x),
                (Vec3::Y, max.y),
                (-Vec3::Y, -min.y),
                (Vec3::Z, max.z),
                (-Vec3::Z, -min.z),
            ]
            .into_iter()
            .map(|(normal, distance)| modkit_core::Plane { normal, distance })
            .collect(),
        }
    }
    #[test]
    fn brush_wall_tangent_sweep_has_no_false_vertical_contact() {
        let physics = Physics::new(&World {
            brushes: vec![bounds_brush(
                Vec3::new(32., -1000., 0.),
                Vec3::new(100., 1000., 300.),
            )],
            ..Default::default()
        });
        let start = Vec3::new(15.96875, 0., 100.);
        let hit = physics.trace_hull(
            start,
            start + Vec3::new(0., 2., 1.),
            Vec3::new(-16., -16., 0.),
            Vec3::new(16., 16., 72.),
        );
        assert_eq!(hit.fraction, 1.);
        assert!(!hit.start_solid);
        assert!(!hit.all_solid);
    }
    #[test]
    fn pressing_into_brush_wall_does_not_interrupt_jump_or_inject_sideways_motion() {
        let floor = bounds_brush(
            Vec3::new(-1000., -1000., -100.),
            Vec3::new(1000., 1000., 0.),
        );
        let open = Physics::new(&World {
            brushes: vec![floor.clone()],
            ..Default::default()
        });
        let walled = Physics::new(&World {
            brushes: vec![
                floor,
                bounds_brush(Vec3::new(32., -1000., 0.), Vec3::new(100., 1000., 300.)),
            ],
            ..Default::default()
        });
        let mut free_player = Player::new(Vec3::new(0., 0., 64.03125));
        let mut wall_player = Player::new(Vec3::new(15.96875, 0., 64.03125));
        free_player.step(Input::default(), &open, TICK);
        wall_player.step(Input::default(), &walled, TICK);
        for tick in 0..45 {
            let input = Input {
                forward: 1.,
                jump: tick == 0,
                ..Default::default()
            };
            free_player.step(input, &open, TICK);
            wall_player.step(input, &walled, TICK);
            assert!(
                (wall_player.feet.z - free_player.feet.z).abs() < 0.001,
                "tick {tick}: wall={wall_player:?}, open={free_player:?}"
            );
            assert!(wall_player.feet.y.abs() < 0.0001);
            assert!(wall_player.feet.x <= 15.969);
        }
    }
    #[test]
    fn moving_overlap_reports_start_solid_and_allows_an_exit() {
        let mins = Vec3::new(-1., -1., 0.);
        let maxs = Vec3::new(1., 1., 2.);
        let mut rapier = Physics::default();
        rapier.colliders.insert(
            ColliderBuilder::cuboid(5. * SCALE, 8. * SCALE, 50. * SCALE).translation(vector![
                15. * SCALE,
                0.,
                50. * SCALE
            ]),
        );
        rapier.query.update(&rapier.colliders);
        for physics in [Physics::new(&clip_box(1)), rapier] {
            let start = Vec3::new(15., 0., 40.);
            let exit = physics.trace_hull(start, Vec3::new(-5., 0., 40.), mins, maxs);
            assert!(exit.start_solid);
            assert!(!exit.all_solid);
            assert_eq!(exit.fraction, 1.);
            let inside = physics.trace_hull(start, start + Vec3::Y, mins, maxs);
            assert!(inside.start_solid);
            assert!(inside.all_solid);
            assert_eq!(inside.fraction, 0.);
        }
    }
    fn convex_box(min: Vec3, max: Vec3, polyhedron: bool) -> ColliderBuilder {
        let half = (max - min) * 0.5;
        let shape = if polyhedron {
            let mut vertices = Vec::new();
            for x in [-half.x, half.x] {
                for y in [-half.y, half.y] {
                    for z in [-half.z, half.z] {
                        vertices.push(Point::from(vector(Vec3::new(x, y, z))));
                    }
                }
            }
            SharedShape::convex_hull(&vertices).unwrap()
        } else {
            SharedShape::cuboid(half.x * SCALE, half.y * SCALE, half.z * SCALE)
        };
        ColliderBuilder::new(shape).position(pose((min + max) * 0.5, Quat::IDENTITY))
    }
    #[test]
    fn convex_prop_wall_preserves_jump_and_tangent_motion() {
        for polyhedron in [false, true] {
            let mut open = Physics::default();
            open.colliders.insert(convex_box(
                Vec3::new(-1000., -1000., -100.),
                Vec3::new(1000., 1000., 0.),
                polyhedron,
            ));
            open.query.update(&open.colliders);
            let mut walled = Physics::default();
            walled.colliders.insert(convex_box(
                Vec3::new(-1000., -1000., -100.),
                Vec3::new(1000., 1000., 0.),
                polyhedron,
            ));
            walled.colliders.insert(convex_box(
                Vec3::new(32., -1000., 0.),
                Vec3::new(100., 1000., 300.),
                polyhedron,
            ));
            walled.query.update(&walled.colliders);
            let start = Vec3::new(15.96875, 0., 100.);
            let tangent = walled.trace_hull(
                start,
                start + Vec3::new(0., 2., 1.),
                Vec3::new(-16., -16., 0.),
                Vec3::new(16., 16., 72.),
            );
            assert_eq!(tangent.fraction, 1.);
            assert!(!tangent.start_solid);
            let mut free_player = Player::new(Vec3::new(0., 0., 64.03125));
            let mut wall_player = Player::new(Vec3::new(15.96875, 0., 64.03125));
            free_player.step(Input::default(), &open, TICK);
            wall_player.step(Input::default(), &walled, TICK);
            for tick in 0..45 {
                let input = Input {
                    forward: 1.,
                    jump: tick == 0,
                    ..Default::default()
                };
                free_player.step(input, &open, TICK);
                wall_player.step(input, &walled, TICK);
                assert!(
                    (wall_player.feet.z - free_player.feet.z).abs() < 0.001,
                    "polyhedron {polyhedron}, tick {tick}: {wall_player:?}"
                );
                assert!(wall_player.feet.y.abs() < 0.0001);
                assert!(wall_player.feet.x <= 15.969);
            }
        }
    }
    #[test]
    fn rotated_convex_edge_bevel_allows_clearance_and_blocks_fast_crossing() {
        let rotation = Quat::from_rotation_z(35f32.to_radians())
            * Quat::from_rotation_y(40f32.to_radians())
            * Quat::from_rotation_x(25f32.to_radians());
        for polyhedron in [false, true] {
            let mut physics = Physics::default();
            let handle = physics.colliders.insert(
                convex_box(Vec3::new(-2., -4., -6.), Vec3::new(2., 4., 6.), polyhedron)
                    .position(pose(Vec3::ZERO, rotation)),
            );
            physics.query.update(&physics.colliders);
            let start = Vec3::new(5.968779, 5.9419513, 6.3287473);
            let half = Vec3::new(1., 2., 3.);
            // This box is separated by an edge-cross-edge axis by .4119 units,
            // while all convex-face and player XYZ projections overlap.
            let collider = &physics.colliders[handle];
            assert!(!rapier3d::parry::query::intersection_test(
                collider.position(),
                collider.shape(),
                &pose(start, Quat::IDENTITY),
                &Cuboid::new(vector(half)),
            )
            .unwrap());
            assert!(!physics.trace_hull(start, start, -half, half).start_solid);
            let enter = physics.trace_hull(start, Vec3::ZERO, -half, half);
            assert!((enter.fraction - 0.05053343).abs() < 0.00001);
            assert!(enter.normal.distance(Vec3::new(0.94934547, 0.31423426, 0.)) < 0.0001);
            let crossing = physics.trace_hull(-Vec3::X * 200., Vec3::X * 200., -half, half);
            assert!(crossing.fraction > 0. && crossing.fraction < 0.5);
            assert!(!crossing.start_solid);
        }
    }
    #[test]
    fn convex_cache_tracks_translation_rotation_shape_and_disabled_state() {
        let mut physics = Physics::default();
        let handle = physics
            .colliders
            .insert(convex_box(Vec3::splat(-2.), Vec3::splat(2.), false));
        let start = Vec3::X * 20.;
        let mins = -Vec3::ONE;
        let maxs = Vec3::ONE;
        physics.query.update(&physics.colliders);
        let initial = physics.trace_hull(start, Vec3::ZERO, mins, maxs);
        for i in 0..1000 {
            let t = Vec3::new((i as f32 * 0.17).sin() * 4., (i as f32 * 0.09).cos(), 0.);
            physics.colliders[handle].set_position(pose(t, Quat::IDENTITY));
            physics.query.update(&physics.colliders);
            physics.trace_hull(start + t, t, mins, maxs);
        }
        physics.colliders[handle].set_position(pose(Vec3::ZERO, Quat::IDENTITY));
        physics.query.update(&physics.colliders);
        assert_eq!(
            physics.trace_hull(start, Vec3::ZERO, mins, maxs).fraction,
            initial.fraction
        );
        physics.colliders[handle].set_shape(SharedShape::cuboid(
            4. * SCALE,
            2. * SCALE,
            2. * SCALE,
        ));
        physics.query.update(&physics.colliders);
        assert!(physics.trace_hull(start, Vec3::ZERO, mins, maxs).fraction < initial.fraction);
        physics.colliders[handle].set_position(pose(
            Vec3::ZERO,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        ));
        physics.query.update(&physics.colliders);
        assert!(
            (physics.trace_hull(start, Vec3::ZERO, mins, maxs).fraction - initial.fraction).abs()
                < 0.00001
        );
        physics.colliders[handle].set_enabled(false);
        physics.query.update(&physics.colliders);
        assert_eq!(
            physics.trace_hull(start, Vec3::ZERO, mins, maxs).fraction,
            1.
        );
    }
    fn clip_box(contents: u32) -> World {
        let mut world = World::default();
        world.brushes.push(Brush {
            contents,
            planes: [
                (Vec3::X, 20.),
                (-Vec3::X, -10.),
                (Vec3::Y, 8.),
                (-Vec3::Y, 8.),
                (Vec3::Z, 100.),
                (-Vec3::Z, 0.),
            ]
            .into_iter()
            .map(|(normal, distance)| modkit_core::Plane { normal, distance })
            .collect(),
        });
        world
    }
    #[test]
    fn npc_only_clip_does_not_block_player_sweeps_or_overlap_queries() {
        // The installed station uses DETAIL|MONSTERCLIP boxes around benches.
        let physics = Physics::new(&clip_box(0x8020000));
        let mins = Vec3::new(-1., -1., 0.);
        let maxs = Vec3::new(1., 1., 2.);
        let sweep =
            physics.trace_hull(Vec3::new(-5., 0., 40.), Vec3::new(35., 0., 40.), mins, maxs);
        assert_eq!(sweep.fraction, 1.);
        assert!(!sweep.start_solid);
        let inside = Vec3::new(15., 0., 40.);
        assert!(!physics.trace_hull(inside, inside, mins, maxs).start_solid);
        // This change is limited to player queries; existing weapon traces and
        // retained world brush/collider data keep their prior behavior.
        assert!(physics
            .impact_ray(Vec3::new(-5., 0., 40.), Vec3::X, 40.)
            .is_some());
        assert_eq!(physics.colliders.len(), 1);
    }
    #[test]
    fn player_clip_solid_and_mixed_monster_solid_still_block_players() {
        let mins = Vec3::new(-1., -1., 0.);
        let maxs = Vec3::new(1., 1., 2.);
        for contents in [1, 0x10000, 1 | 0x20000] {
            let physics = Physics::new(&clip_box(contents));
            let sweep =
                physics.trace_hull(Vec3::new(-5., 0., 40.), Vec3::new(35., 0., 40.), mins, maxs);
            assert!(sweep.fraction < 1., "contents {contents:#x}");
            let inside = Vec3::new(15., 0., 40.);
            assert!(
                physics.trace_hull(inside, inside, mins, maxs).start_solid,
                "contents {contents:#x}"
            );
        }
    }
    #[test]
    fn melee_box_sweep_reaches_a_nearby_offset_target_but_not_outside_its_width() {
        let mut physics = Physics::default();
        physics.colliders.insert(
            ColliderBuilder::cuboid(5. * SCALE, 5. * SCALE, 5. * SCALE)
                .translation(vector![40. * SCALE, 20. * SCALE, 0.])
                .user_data(1),
        );
        physics.query.update(&physics.colliders);
        assert!(physics.impact_ray(Vec3::ZERO, Vec3::X, 75.).is_none());
        assert_eq!(
            physics
                .impact_hull(Vec3::ZERO, Vec3::X, 75. - 1.732 * 16., Vec3::splat(16.))
                .unwrap()
                .entity,
            0
        );
        assert!(physics
            .impact_hull(
                Vec3::new(0., -20., 0.),
                Vec3::X,
                75. - 1.732 * 16.,
                Vec3::splat(16.)
            )
            .is_none());
    }
    #[test]
    fn touching_floor_allows_standing_but_penetration_does_not() {
        let mut p = Physics::default();
        p.colliders
            .insert(ColliderBuilder::cuboid(10., 10., 1.).translation(vector![0., 0., -1.]));
        p.query.update(&p.colliders);
        let mins = Vec3::new(-16., -16., 0.);
        let maxs = Vec3::new(16., 16., 72.);
        assert!(!p.trace_hull(Vec3::ZERO, Vec3::ZERO, mins, maxs).start_solid);
        let below = -Vec3::Z;
        assert!(p.trace_hull(below, below, mins, maxs).start_solid);
    }
    #[test]
    fn killed_dynamic_entity_no_longer_blocks_raycasts() {
        let mut p = Physics::default();
        let body = p.bodies.insert(RigidBodyBuilder::dynamic());
        let c = p.colliders.insert_with_parent(
            ColliderBuilder::ball(1.).user_data(1),
            body,
            &mut p.bodies,
        );
        p.dynamic.insert(0, body);
        p.entity_colliders.insert(0, vec![c]);
        p.tick(0.015);
        assert!(p.ray(Vec3::new(-100., 0., 0.), Vec3::X, 200.).is_some());
        p.set_entity(0, Vec3::ZERO, Quat::IDENTITY, false);
        p.tick(0.015);
        assert!(p.ray(Vec3::new(-100., 0., 0.), Vec3::X, 200.).is_none());
    }
    fn npc_config() -> crate::npc_probe::GroundConfig {
        crate::npc_probe::GroundConfig {
            hull: Hull {
                mins: Vec3::new(-13., -13., 0.),
                maxs: Vec3::new(13., 13., 72.),
            },
            step_height: 18.,
            step_down_multiplier: 1.,
        }
    }
    fn npc_floor_world() -> World {
        World {
            brushes: vec![bounds_brush(
                Vec3::new(-200., -200., -20.),
                Vec3::new(200., 200., 0.),
            )],
            ..Default::default()
        }
    }
    #[test]
    fn npc_clip_masks_are_distinct_from_player_clip_and_preserve_player_queries() {
        for (contents, npc_blocked, player_blocked) in [
            (0x20000, true, false),
            (0x10000, false, true),
            (1, true, true),
        ] {
            let mut brush = bounds_brush(Vec3::new(25., -100., 0.), Vec3::new(35., 100., 100.));
            brush.contents = contents;
            let physics = Physics::new(&World {
                brushes: vec![brush],
                ..Default::default()
            });
            let hull = npc_config().hull;
            let npc = physics.npc_trace_hull(Vec3::ZERO, Vec3::X * 60., hull, Query::default());
            let player = physics.trace_hull(Vec3::ZERO, Vec3::X * 60., hull.mins, hull.maxs);
            assert_eq!(
                npc.trace.fraction < 1.,
                npc_blocked,
                "contents {contents:x}"
            );
            assert_eq!(
                player.fraction < 1.,
                player_blocked,
                "contents {contents:x}"
            );
        }
    }
    #[test]
    fn npc_transient_hulls_exclude_self_but_block_on_player_and_other_actors() {
        use crate::npc_probe::NPC_BRUSH_ONLY_MASK;
        let physics = Physics::default();
        let hull = npc_config().hull;
        let actors = [
            ActorHull {
                entity: Some(7),
                feet: Vec3::ZERO,
                hull,
            },
            ActorHull {
                entity: Some(8),
                feet: Vec3::X * 60.,
                hull,
            },
        ];
        let excluded = [7];
        let query = Query {
            excluded_entities: &excluded,
            transients: &actors,
            ..Default::default()
        };
        let hit = physics.npc_trace_hull(Vec3::ZERO, Vec3::X * 100., hull, query);
        assert!(!hit.trace.start_solid);
        assert_eq!(hit.entity, Some(8));
        assert!(hit.transient && hit.trace.fraction < 1.);
        assert!(physics
            .npc_trace_hull(
                Vec3::ZERO,
                Vec3::X * 100.,
                hull,
                Query {
                    contents_mask: NPC_BRUSH_ONLY_MASK,
                    ..query
                }
            )
            .clear_path());
        let player = [ActorHull {
            entity: None,
            feet: Vec3::X * 60.,
            hull,
        }];
        let hit = physics.npc_trace_hull(
            Vec3::ZERO,
            Vec3::X * 100.,
            hull,
            Query {
                transients: &player,
                ..Default::default()
            },
        );
        assert!(hit.transient && hit.entity.is_none() && hit.trace.fraction < 1.);
        let inside = physics.npc_trace_hull(
            Vec3::ZERO,
            Vec3::ZERO,
            hull,
            Query {
                transients: &actors,
                ..Default::default()
            },
        );
        assert!(inside.trace.start_solid && inside.trace.all_solid);
        assert_eq!(inside.entity, Some(7));
    }
    #[test]
    fn npc_brush_door_hits_have_entity_and_follow_pose_before_simulation_tick() {
        let world = World {
            entities: vec![modkit_core::Entity {
                properties: vec![
                    ("classname".into(), "func_door".into()),
                    ("model".into(), "*1".into()),
                ],
            }],
            brush_models: vec![modkit_core::BrushModel {
                id: 1,
                brushes: vec![bounds_brush(
                    Vec3::new(0., -100., 0.),
                    Vec3::new(4., 100., 100.),
                )],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut physics = Physics::new(&world);
        let hull = npc_config().hull;
        let start = Vec3::new(-40., 0., 0.);
        let end = Vec3::new(40., 0., 0.);
        let hit = physics.npc_trace_hull(start, end, hull, Query::default());
        assert_eq!(hit.entity, Some(0));
        assert!(hit.trace.fraction < 1. && hit.trace.normal.x < -0.99);
        let excluded = [0];
        assert!(physics
            .npc_trace_hull(
                start,
                end,
                hull,
                Query {
                    excluded_entities: &excluded,
                    ..Default::default()
                }
            )
            .clear_path());
        physics.set_entity(0, Vec3::X * 200., Quat::IDENTITY, true);
        physics.refresh_queries();
        assert!(physics
            .npc_trace_hull(start, end, hull, Query::default())
            .clear_path());
        physics.set_entity(0, Vec3::ZERO, Quat::IDENTITY, true);
        physics.refresh_queries();
        assert!(
            (physics
                .npc_trace_hull(start, end, hull, Query::default())
                .trace
                .fraction
                - hit.trace.fraction)
                .abs()
                < 1e-5
        );
    }
    #[test]
    fn npc_point_visibility_and_model_door_follow_rotation_and_disabled_state() {
        let mut physics = Physics::new(&door_world());
        let start = Vec3::new(-40., 24., 0.);
        let end = Vec3::new(40., 24., 0.);
        let point = Hull {
            mins: Vec3::ZERO,
            maxs: Vec3::ZERO,
        };
        let query = Query {
            contents_mask: crate::npc_probe::NPC_BRUSH_ONLY_MASK,
            ..Default::default()
        };
        let visible = physics.npc_trace_hull(start, end, point, query);
        assert!(visible.trace.fraction < 1.);
        assert_eq!(visible.entity, Some(0));
        let blocked = physics.npc_trace_hull(start, end, npc_config().hull, query);
        assert!(blocked.trace.fraction < visible.trace.fraction);
        physics.set_entity(
            0,
            Vec3::ZERO,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            true,
        );
        physics.refresh_queries();
        assert!(physics
            .npc_trace_hull(start, end, point, query)
            .clear_path());
        assert!(physics
            .npc_trace_hull(start, end, npc_config().hull, query)
            .clear_path());
        physics.set_entity(0, Vec3::ZERO, Quat::IDENTITY, false);
        physics.refresh_queries();
        assert!(physics
            .npc_trace_hull(start, end, npc_config().hull, query)
            .clear_path());
        physics.set_entity(0, Vec3::ZERO, Quat::IDENTITY, true);
        physics.refresh_queries();
        assert_eq!(
            physics.npc_trace_hull(start, end, point, query).entity,
            Some(0)
        );
    }
    #[test]
    fn npc_stand_and_ground_move_validate_floor_and_do_not_teleport_to_target_z() {
        use crate::npc_probe::{fits, ground_move, stand};
        let physics = Physics::new(&npc_floor_world());
        let config = npc_config();
        assert!(stand(&physics, Vec3::ZERO, config, Query::default()));
        assert!(fits(&physics, Vec3::ZERO, config.hull, Query::default()));
        let movement = ground_move(
            &physics,
            Vec3::ZERO,
            Vec3::new(80., 0., 200.),
            config,
            Query::default(),
        );
        assert!(movement.completed, "{movement:?}");
        assert!((movement.end.x - 80.).abs() < 1e-4);
        assert!(movement.end.z < 0.2 && movement.end.z >= 0.);
        let empty = Physics::default();
        assert!(!stand(&empty, Vec3::ZERO, config, Query::default()));
        assert!(
            !ground_move(&empty, Vec3::ZERO, Vec3::X * 40., config, Query::default()).completed
        );
    }
    #[test]
    fn npc_ground_probe_steps_onto_low_step_and_rejects_tall_step_and_wall() {
        use crate::npc_probe::ground_move;
        for (height, expected) in [(12., true), (25., false), (100., false)] {
            let mut world = npc_floor_world();
            world.brushes.push(bounds_brush(
                Vec3::new(20., -100., 0.),
                Vec3::new(150., 100., height),
            ));
            let physics = Physics::new(&world);
            let movement = ground_move(
                &physics,
                Vec3::ZERO,
                Vec3::X * 80.,
                npc_config(),
                Query::default(),
            );
            assert_eq!(
                movement.completed, expected,
                "height {height}: {movement:?}"
            );
            if expected {
                assert!((movement.end.z - height).abs() < 0.2);
            } else {
                assert!(movement.end.x < 7. && movement.end.z < 0.2);
            }
        }
    }
    #[test]
    fn npc_ground_probe_blocks_gap_large_drop_and_low_ceiling() {
        use crate::npc_probe::ground_move;
        for floor in [None, Some(-30.)] {
            let mut world = World {
                brushes: vec![bounds_brush(
                    Vec3::new(-100., -100., -20.),
                    Vec3::new(20., 100., 0.),
                )],
                ..Default::default()
            };
            if let Some(z) = floor {
                world.brushes.push(bounds_brush(
                    Vec3::new(20., -100., z - 10.),
                    Vec3::new(200., 100., z),
                ));
            }
            let movement = ground_move(
                &Physics::new(&world),
                Vec3::ZERO,
                Vec3::X * 80.,
                npc_config(),
                Query::default(),
            );
            assert!(!movement.completed && movement.end.x < 40., "{movement:?}");
        }
        let mut world = npc_floor_world();
        world.brushes.push(bounds_brush(
            Vec3::new(20., -100., 0.),
            Vec3::new(150., 100., 12.),
        ));
        world.brushes.push(bounds_brush(
            Vec3::new(0., -100., 76.),
            Vec3::new(150., 100., 90.),
        ));
        let movement = ground_move(
            &Physics::new(&world),
            Vec3::ZERO,
            Vec3::X * 80.,
            npc_config(),
            Query::default(),
        );
        assert!(!movement.completed && movement.end.x < 7., "{movement:?}");
    }
    #[test]
    fn npc_ground_probe_rejects_embedded_nan_and_excessive_travel() {
        use crate::npc_probe::{ground_move, BlockReason};
        let physics = Physics::new(&npc_floor_world());
        for (start, end, reason) in [
            (
                Vec3::new(0., 0., -10.),
                Vec3::X * 50.,
                BlockReason::StartSolid,
            ),
            (
                Vec3::ZERO,
                Vec3::new(f32::NAN, 0., 0.),
                BlockReason::InvalidInput,
            ),
            (Vec3::ZERO, Vec3::X * 10000., BlockReason::TravelBudget),
        ] {
            let movement = ground_move(&physics, start, end, npc_config(), Query::default());
            assert!(!movement.completed);
            assert_eq!(movement.reason, Some(reason));
            assert_eq!(movement.end, start);
        }
    }
}
