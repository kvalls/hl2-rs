//! Moving SMG grenades and weapon-launched Combine balls. Launch rules use
//! reviewed retail methods; Rapier contacts and effects remain approximations.
use crate::{
    entities::Scene,
    physics::{Physics, ProjectileHull},
};
use glam::Vec3;
use modkit_core::World;
use serde::Serialize;

pub const GRENADE_MODEL: &str = "models/weapons/ar2_grenade.mdl";
pub const BALL_MODEL: &str = "models/effects/combineball.mdl";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ProjectileKind {
    SmgGrenade,
    CombineBall,
}

#[derive(Clone, Copy, Debug)]
pub struct ProjectileSpawn {
    pub kind: ProjectileKind,
    pub position: Vec3,
    pub velocity: Vec3,
    /// Source degrees per second, retained for the renderer's moving model.
    pub angular_velocity: Vec3,
    pub at: f64,
    pub damage: f32,
    /// Blast radius for a grenade; physical radius for a Combine ball.
    pub radius: f32,
    pub mass: f32,
    pub lifetime: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Projectile {
    pub id: u64,
    pub kind: ProjectileKind,
    pub position: Vec3,
    pub previous_position: Vec3,
    pub velocity: Vec3,
    pub angles: Vec3,
    pub angular_velocity: Vec3,
    pub spawned_at: f64,
    pub radius: f32,
    pub mass: f32,
    expires_at: Option<f64>,
    damage: f32,
    last_bounce: f64,
    struck_entity: bool,
    next_whiz: f64,
    hit_entities: Vec<usize>,
}
impl Projectile {
    pub fn model(&self) -> &'static str {
        match self.kind {
            ProjectileKind::SmgGrenade => GRENADE_MODEL,
            ProjectileKind::CombineBall => BALL_MODEL,
        }
    }
    pub fn physical_radius(&self) -> f32 {
        match self.kind {
            ProjectileKind::SmgGrenade => 3.,
            ProjectileKind::CombineBall => self.radius,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum EffectKind {
    GrenadeExplosion,
    BallImpact,
    BallExplosion,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Effect {
    pub id: u64,
    pub magnitude: f32,
    pub kind: EffectKind,
    pub position: Vec3,
    pub normal: Vec3,
    pub at: f64,
    pub radius: f32,
}
impl Effect {
    pub fn duration(&self) -> f64 {
        match self.kind {
            EffectKind::BallImpact => 4.,
            EffectKind::GrenadeExplosion => 3.,
            EffectKind::BallExplosion => 8.,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum DamageTarget {
    Entity(usize),
    Player,
}
#[derive(Clone, Copy, Debug)]
pub struct Damage {
    pub target: DamageTarget,
    pub amount: f32,
    pub dissolve: bool,
    pub direction: Vec3,
    /// The inflictor's origin (the projectile or the trigger).
    pub origin: Vec3,
}
#[derive(Default, Serialize)]
pub struct Diagnostics {
    pub grenades_spawned: u64,
    pub balls_spawned: u64,
    pub grenade_detonations: u64,
    pub ball_bounces: u64,
    pub ball_expirations: u64,
    pub ball_collision_removals: u64,
    pub damage_events: u64,
    pub occluded_blast_targets: u64,
    pub unsupported_damage_filters: u64,
    pub capacity_rejections: u64,
    pub invalid_launches: u64,
    pub out_of_world: u64,
    pub collision_budget_exhaustions: u64,
}
#[derive(Default)]
pub struct Projectiles {
    pub active: Vec<Projectile>,
    pub effects: Vec<Effect>,
    pub diagnostics: Diagnostics,
    next_id: u64,
    next_effect_id: u64,
}
impl Projectiles {
    fn effect(&mut self, mut effect: Effect) {
        self.next_effect_id += 1;
        effect.id = self.next_effect_id;
        self.effects.push(effect);
    }
    pub fn spawn(&mut self, launch: ProjectileSpawn, scene: &mut Scene) {
        if self.active.len() >= 128 {
            self.diagnostics.capacity_rejections += 1;
            return;
        }
        if !launch.position.is_finite()
            || !launch.velocity.is_finite()
            || !launch.angular_velocity.is_finite()
            || !launch.at.is_finite()
            || [launch.damage, launch.radius, launch.mass, launch.lifetime]
                .iter()
                .any(|value| !value.is_finite())
        {
            self.diagnostics.invalid_launches += 1;
            return;
        }
        self.next_id += 1;
        let ball = launch.kind == ProjectileKind::CombineBall;
        if ball {
            self.diagnostics.balls_spawned += 1;
            scene.sounds.push("NPC_CombineBall.Launch".into());
        } else {
            self.diagnostics.grenades_spawned += 1;
        }
        let direction = launch.velocity.normalize_or_zero();
        self.active.push(Projectile {
            id: self.next_id,
            kind: launch.kind,
            position: launch.position,
            previous_position: launch.position,
            velocity: launch.velocity,
            angles: Vec3::new(
                (-direction.z)
                    .atan2(direction.truncate().length())
                    .to_degrees(),
                direction.y.atan2(direction.x).to_degrees(),
                0.,
            ),
            angular_velocity: launch.angular_velocity,
            spawned_at: launch.at,
            radius: if ball {
                launch.radius.clamp(1., 12.)
            } else {
                launch.radius.max(0.)
            },
            mass: launch.mass,
            expires_at: ball.then_some(launch.at + launch.lifetime.max(0.) as f64),
            damage: launch.damage.max(0.),
            last_bounce: -1.,
            struck_entity: false,
            next_whiz: launch.at + 0.03,
            hit_entities: Vec::new(),
        });
    }
    /// Advance only on simulation ticks. Returns damage to be applied through
    /// Inventory so player health and weapon hit/kill counters stay authoritative.
    pub fn tick(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        player_feet: Vec3,
        player_ducked: bool,
        dt: f32,
    ) -> Vec<Damage> {
        if !dt.is_finite() || dt <= 0. {
            return Vec::new();
        }
        self.effects
            .retain(|effect| scene.time - effect.at <= effect.duration());
        let mut damage = Vec::new();
        let mut survivors = Vec::with_capacity(self.active.len());
        for mut projectile in std::mem::take(&mut self.active) {
            projectile.previous_position = projectile.position;
            if projectile.position.abs().max_element() > 16384. {
                self.diagnostics.out_of_world += 1;
                continue;
            }
            if projectile.expires_at.is_some_and(|end| scene.time >= end) {
                self.ball_explosion(&projectile, scene, true);
                continue;
            }
            let mut alive = true;
            match projectile.kind {
                ProjectileKind::SmgGrenade => {
                    // FLYGRAVITY applies half of its 400-unit gravity before the
                    // move and half after. Stock Spawn starts live immediately.
                    projectile.velocity.z -= 200. * dt;
                    let end = projectile.position + projectile.velocity * dt;
                    if let Some(hit) = physics.projectile_sweep(
                        projectile.position,
                        end,
                        ProjectileHull::Box(Vec3::splat(3.)),
                        &[],
                    ) {
                        projectile.position = hit.position;
                        self.grenade_explosion(
                            &projectile,
                            hit.normal,
                            world,
                            scene,
                            physics,
                            player_feet,
                            player_ducked,
                            &mut damage,
                        );
                        alive = false;
                    } else {
                        projectile.position = end;
                        projectile.velocity.z -= 200. * dt;
                        projectile.angles += projectile.angular_velocity * dt;
                    }
                }
                ProjectileKind::CombineBall => {
                    let mut remaining = dt;
                    let mut excluded = projectile.hit_entities.clone();
                    for attempt in 0..8 {
                        let end = projectile.position + projectile.velocity * remaining;
                        let Some(hit) = physics.projectile_sweep(
                            projectile.position,
                            end,
                            ProjectileHull::Sphere(projectile.radius),
                            &excluded,
                        ) else {
                            projectile.position = end;
                            break;
                        };
                        projectile.position = hit.position;
                        let entity = world.entities.get(hit.entity);
                        let npc = entity.is_some_and(|entity| entity.class().starts_with("npc_"));
                        if npc && !projectile.hit_entities.contains(&hit.entity) {
                            let entity = entity.unwrap();
                            // Avoid repeating this contact in the same tick;
                            // protected/filtered actors stay solid on later ticks.
                            excluded.push(hit.entity);
                            if entity.class() == "npc_strider" {
                                self.ball_explosion(&projectile, scene, false);
                                alive = false;
                                break;
                            }
                            if entity
                                .get("damagefilter")
                                .is_some_and(|name| !name.is_empty())
                            {
                                self.diagnostics.unsupported_damage_filters += 1;
                            } else if !friendly_or_vital(entity.class()) {
                                projectile.hit_entities.push(hit.entity);
                                damage.push(Damage {
                                    target: DamageTarget::Entity(hit.entity),
                                    amount: 0.,
                                    dissolve: true,
                                    direction: projectile.velocity.normalize_or_zero(),
                                    origin: projectile.position,
                                });
                                projectile.struck_entity = true;
                                scene.sounds.push("NPC_CombineBall.KillImpact".into());
                            }
                        }
                        physics.impulse(
                            hit.entity,
                            projectile.velocity.normalize_or_zero(),
                            projectile.mass * projectile.velocity.length() / 39.37,
                        );
                        // Elastic reflection preserves the reviewed desired speed.
                        // Full VPhysics post-collision velocity/materials are absent.
                        let normal = hit.normal.normalize_or_zero();
                        let reflected =
                            projectile.velocity - 2. * projectile.velocity.dot(normal) * normal;
                        if reflected.length_squared() < 1e-6 {
                            self.ball_explosion(&projectile, scene, false);
                            alive = false;
                            break;
                        }
                        projectile.velocity = reflected.normalize() * projectile.velocity.length();
                        projectile.position += normal * 0.03125;
                        remaining *= 1. - hit.fraction.clamp(0., 1.);
                        // Movable props/characters return after OnHitEntity;
                        // world and non-damageable push brushes dispatch effects.
                        if !entity.is_some_and(ball_hittable_entity) {
                            self.effect(Effect {
                                id: 0,
                                magnitude: 0.,
                                kind: EffectKind::BallImpact,
                                position: projectile.position,
                                normal,
                                at: scene.time,
                                radius: 16.,
                            });
                            scene.sounds.push("NPC_CombineBall.Impact".into());
                            if scene.time - projectile.last_bounce >= 0.25 {
                                projectile.last_bounce = scene.time;
                                self.diagnostics.ball_bounces += 1;
                                guide_ball(&mut projectile, world, scene, physics);
                            }
                        }
                        if remaining <= 1e-6 {
                            break;
                        }
                        if attempt == 7 {
                            self.diagnostics.collision_budget_exhaustions += 1;
                        }
                    }
                    // Whiz audio is not emitted every frame; one nearby pass has
                    // the native half-second sound throttle. Mixing is still 2D.
                    if alive && scene.time >= projectile.next_whiz {
                        if whiz_near_player(projectile.position, projectile.velocity, player_feet) {
                            scene.sounds.push("NPC_CombineBall.WhizFlyby".into());
                            projectile.next_whiz = scene.time + 0.5;
                        } else {
                            projectile.next_whiz = scene.time + 0.03;
                        }
                    }
                }
            }
            if alive {
                survivors.push(projectile);
            }
        }
        self.active = survivors;
        self.diagnostics.damage_events += damage.len() as u64;
        damage
    }
    fn ball_explosion(&mut self, projectile: &Projectile, scene: &mut Scene, expired: bool) {
        if expired {
            self.diagnostics.ball_expirations += 1;
        } else {
            self.diagnostics.ball_collision_removals += 1;
        }
        self.effect(Effect {
            id: 0,
            magnitude: 0.,
            kind: EffectKind::BallExplosion,
            position: projectile.position,
            normal: Vec3::Z,
            at: scene.time,
            radius: projectile.radius,
        });
        // Weapon balls expire with effects, not radial blast damage.
        scene.sounds.push("NPC_CombineBall.Explosion".into());
    }
    #[allow(clippy::too_many_arguments)]
    fn grenade_explosion(
        &mut self,
        projectile: &Projectile,
        normal: Vec3,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        player_feet: Vec3,
        player_ducked: bool,
        damage: &mut Vec<Damage>,
    ) {
        self.diagnostics.grenade_detonations += 1;
        self.effect(Effect {
            id: 0,
            magnitude: projectile.damage,
            kind: EffectKind::GrenadeExplosion,
            position: projectile.position,
            normal,
            at: scene.time,
            radius: projectile.radius,
        });
        scene.sounds.push("BaseExplosionEffect.Sound".into());
        let origin = projectile.position + Vec3::Z;
        if projectile.radius <= 0. {
            return;
        }
        for (id, entity) in world.entities.iter().enumerate() {
            let state = &scene.states[id];
            if state.killed || !state.enabled {
                continue;
            }
            let damageable = entity.class().starts_with("npc_")
                || matches!(entity.class(), "func_breakable" | "func_physbox")
                || entity.class().starts_with("prop_physics")
                    && entity
                        .get("health")
                        .and_then(|s| s.parse::<f32>().ok())
                        .is_some_and(|v| v > 0.);
            // BodyTarget, damage filters, water partition and blocked fraction
            // differ among native classes; supported collision shapes supply a
            // real closest point rather than an arbitrary origin-only radius.
            let target = physics.entity_closest_point(id, origin).unwrap_or(
                state.origin
                    + if entity.class().starts_with("npc_") {
                        Vec3::Z * 36.
                    } else {
                        Vec3::ZERO
                    },
            );
            let delta = target - origin;
            let amount = projectile.damage * (1. - delta.length() / projectile.radius);
            if amount <= 0. {
                continue;
            }
            if physics.projectile_ray(origin, target, &[id]).is_some() {
                self.diagnostics.occluded_blast_targets += 1;
                continue;
            }
            physics.impulse(id, delta.normalize_or_zero(), amount);
            if damageable {
                if entity
                    .get("damagefilter")
                    .is_some_and(|name| !name.is_empty())
                {
                    self.diagnostics.unsupported_damage_filters += 1;
                } else {
                    damage.push(Damage {
                        target: DamageTarget::Entity(id),
                        amount,
                        dissolve: false,
                        direction: delta.normalize_or_zero(),
                        origin,
                    });
                }
            }
        }
        let maxs = player_feet + Vec3::new(16., 16., if player_ducked { 36. } else { 72. });
        let mins = player_feet - Vec3::new(16., 16., 0.);
        let target = origin.clamp(mins, maxs);
        let amount = projectile.damage * (1. - origin.distance(target) / projectile.radius);
        if amount > 0. && physics.projectile_ray(origin, target, &[]).is_none() {
            damage.push(Damage {
                target: DamageTarget::Player,
                amount,
                dissolve: false,
                direction: (target - origin).normalize_or_zero(),
                origin,
            });
        }
    }
}
fn friendly_or_vital(class: &str) -> bool {
    // Native class relationships/EFL_NO_DISSOLVE are not reconstructed yet.
    // These known HL2 allies/vital actors must not silently be treated as enemies.
    matches!(
        class,
        "npc_citizen"
            | "npc_barney"
            | "npc_alyx"
            | "npc_eli"
            | "npc_kleiner"
            | "npc_mossman"
            | "npc_vortigaunt"
            | "npc_gman"
            | "npc_dog"
            | "npc_bullseye"
    )
}
fn ball_hittable_entity(entity: &modkit_core::Entity) -> bool {
    let push = matches!(
        entity.class(),
        "func_door"
            | "func_door_rotating"
            | "func_movelinear"
            | "func_rotating"
            | "func_brush"
            | "func_wall"
            | "func_wall_toggle"
            | "prop_door_rotating"
    );
    !push
        || entity
            .get("health")
            .and_then(|value| value.parse::<f32>().ok())
            .is_some_and(|health| health > 0.)
}
fn whiz_near_player(position: Vec3, velocity: Vec3, player_feet: Vec3) -> bool {
    let delta = player_feet - position;
    // Native WhizSoundThink uses a normalized target direction dotted with
    // velocity, then distance to the next two Source ticks, not an origin sphere.
    if delta.normalize_or_zero().dot(velocity) <= 0.5 {
        return false;
    }
    let segment = velocity * 0.03;
    let along = (delta.dot(segment) / segment.length_squared().max(1e-6)).clamp(0., 1.);
    player_feet.distance(position + segment * along) < 200.
}
fn guide_ball(projectile: &mut Projectile, world: &World, scene: &Scene, physics: &Physics) {
    let direction = projectile.velocity.normalize_or_zero();
    let mut best = None;
    for (id, entity) in world.entities.iter().enumerate() {
        if !entity.class().starts_with("npc_")
            || friendly_or_vital(entity.class())
            || entity.class() == "npc_strider"
            || scene.states[id].killed
            || !scene.states[id].visible
        {
            continue;
        }
        let target = scene.states[id].origin + Vec3::Z * 36.;
        let delta = target - projectile.position;
        let distance = delta.length();
        let dot = direction.dot(delta.normalize_or_zero());
        let attractive = if projectile.struck_entity {
            distance <= 512. && dot > 0.
        } else {
            distance <= 2048. && dot > 0.966
        };
        if attractive
            && physics
                .projectile_ray(projectile.position, target, &[id])
                .is_none()
            && best.is_none_or(|(_, old)| distance < old)
        {
            best = Some((delta, distance));
        }
    }
    if let Some((delta, _)) = best {
        projectile.velocity = delta.normalize_or_zero() * projectile.velocity.length();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rapier3d::prelude::*;

    fn launch(kind: ProjectileKind, position: Vec3, velocity: Vec3) -> ProjectileSpawn {
        ProjectileSpawn {
            kind,
            position,
            velocity,
            angular_velocity: Vec3::ZERO,
            at: 0.,
            damage: 100.,
            radius: if kind == ProjectileKind::SmgGrenade {
                250.
            } else {
                10.
            },
            mass: 150.,
            lifetime: 2.,
        }
    }
    fn cube(physics: &mut Physics, center: Vec3, half: Vec3, entity: Option<usize>) {
        physics.colliders.insert(
            ColliderBuilder::cuboid(half.x / 39.37, half.y / 39.37, half.z / 39.37)
                .translation(vector![
                    center.x / 39.37,
                    center.y / 39.37,
                    center.z / 39.37
                ])
                .user_data(entity.map_or(0, |id| id as u128 + 1)),
        );
        physics.tick(0.015);
    }
    fn npc(class: &str, origin: Vec3) -> modkit_core::Entity {
        modkit_core::Entity {
            properties: vec![
                ("classname".into(), class.into()),
                (
                    "origin".into(),
                    format!("{} {} {}", origin.x, origin.y, origin.z),
                ),
                ("health".into(), "100".into()),
            ],
        }
    }
    #[test]
    fn grenade_follows_ballistic_motion_without_an_arbitrary_fuse() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            launch(ProjectileKind::SmgGrenade, Vec3::Z * 100., Vec3::X * 1000.),
            &mut scene,
        );
        for tick in 1..=10 {
            scene.time = tick as f64 * 0.015;
            assert!(projectiles
                .tick(
                    &world,
                    &mut scene,
                    &mut physics,
                    Vec3::splat(1000.),
                    false,
                    0.015
                )
                .is_empty());
        }
        assert_eq!(projectiles.active.len(), 1);
        let grenade = &projectiles.active[0];
        assert!((grenade.position.x - 150.).abs() < 0.001);
        assert!((grenade.position.z - 95.5).abs() < 0.001);
        assert!((grenade.velocity.z + 60.).abs() < 0.001);
        assert_eq!(projectiles.diagnostics.grenade_detonations, 0);
    }
    #[test]
    fn live_grenade_box_cannot_tunnel_through_a_thin_wall() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        cube(
            &mut physics,
            Vec3::new(100., 0., 0.),
            Vec3::new(1., 200., 200.),
            None,
        );
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            launch(ProjectileKind::SmgGrenade, Vec3::ZERO, Vec3::X * 1000.),
            &mut scene,
        );
        scene.time = 0.15;
        projectiles.tick(
            &world,
            &mut scene,
            &mut physics,
            Vec3::splat(1000.),
            false,
            0.15,
        );
        assert!(projectiles.active.is_empty());
        assert_eq!(projectiles.diagnostics.grenade_detonations, 1);
        assert!((projectiles.effects[0].position.x - 96.).abs() < 0.01);
        assert!(projectiles.effects[0].normal.dot(-Vec3::X) > 0.999);
        scene.time = 0.3;
        projectiles.tick(
            &world,
            &mut scene,
            &mut physics,
            Vec3::splat(1000.),
            false,
            0.15,
        );
        assert_eq!(projectiles.diagnostics.grenade_detonations, 1);
    }
    #[test]
    fn grenade_blast_uses_real_occlusion_and_linear_distance_falloff() {
        let world = World {
            entities: vec![
                npc("npc_metropolice", Vec3::new(40., 0., -36.)),
                npc("npc_metropolice", Vec3::new(150., 0., -36.)),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        cube(
            &mut physics,
            Vec3::new(100., 0., 0.),
            Vec3::new(1., 200., 200.),
            None,
        );
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            launch(ProjectileKind::SmgGrenade, Vec3::ZERO, Vec3::X * 1000.),
            &mut scene,
        );
        scene.time = 0.15;
        let damage = projectiles.tick(
            &world,
            &mut scene,
            &mut physics,
            Vec3::splat(1000.),
            false,
            0.15,
        );
        assert_eq!(damage.len(), 1);
        assert!(matches!(damage[0].target, DamageTarget::Entity(0)));
        let origin = projectiles.effects[0].position + Vec3::Z;
        assert!(
            (damage[0].amount - 100. * (1. - origin.distance(Vec3::new(40., 0., 0.)) / 250.)).abs()
                < 0.001
        );
        assert_eq!(projectiles.diagnostics.occluded_blast_targets, 1);
    }
    #[test]
    fn ball_sphere_bounces_preserves_speed_and_expiry_causes_no_blast_damage() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        cube(
            &mut physics,
            Vec3::new(0., 0., -1.),
            Vec3::new(100., 100., 1.),
            None,
        );
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            launch(ProjectileKind::CombineBall, Vec3::Z * 15., -Vec3::Z * 1000.),
            &mut scene,
        );
        scene.time = 0.02;
        assert!(projectiles
            .tick(&world, &mut scene, &mut physics, Vec3::ZERO, false, 0.02)
            .is_empty());
        assert_eq!(projectiles.active.len(), 1);
        assert!(projectiles.active[0].position.z > 24.9);
        assert!((projectiles.active[0].velocity.length() - 1000.).abs() < 0.01);
        assert!(projectiles.active[0].velocity.z > 999.);
        assert_eq!(projectiles.diagnostics.ball_bounces, 1);
        scene.time = 2.;
        assert!(projectiles
            .tick(&world, &mut scene, &mut physics, Vec3::ZERO, false, 0.02)
            .is_empty());
        assert!(projectiles.active.is_empty());
        assert_eq!(projectiles.diagnostics.ball_expirations, 1);
        assert!(projectiles
            .effects
            .iter()
            .any(|effect| effect.kind == EffectKind::BallExplosion));
    }
    #[test]
    fn ball_hit_dissolves_hostile_npc_once_and_retains_a_moving_projectile() {
        let mut world = World::default();
        world.entities.push(npc("npc_metropolice", Vec3::X * 40.));
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        cube(&mut physics, Vec3::X * 40., Vec3::splat(5.), Some(0));
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            launch(ProjectileKind::CombineBall, Vec3::ZERO, Vec3::X * 1000.),
            &mut scene,
        );
        scene.time = 0.06;
        let damage = projectiles.tick(
            &world,
            &mut scene,
            &mut physics,
            Vec3::splat(1000.),
            false,
            0.06,
        );
        assert_eq!(damage.len(), 1);
        assert!(damage[0].dissolve);
        assert_eq!(projectiles.active.len(), 1);
        scene.time = 0.12;
        assert!(projectiles
            .tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(1000.),
                false,
                0.06
            )
            .is_empty());
    }
    #[test]
    fn projectile_queries_filter_npc_only_clip_and_respect_solid_brushes() {
        let mut world = World::default();
        let box_brush = |contents, center: f32| modkit_core::Brush {
            contents,
            planes: vec![
                modkit_core::Plane {
                    normal: Vec3::X,
                    distance: center + 1.,
                },
                modkit_core::Plane {
                    normal: -Vec3::X,
                    distance: -center + 1.,
                },
                modkit_core::Plane {
                    normal: Vec3::Y,
                    distance: 40.,
                },
                modkit_core::Plane {
                    normal: -Vec3::Y,
                    distance: 40.,
                },
                modkit_core::Plane {
                    normal: Vec3::Z,
                    distance: 40.,
                },
                modkit_core::Plane {
                    normal: -Vec3::Z,
                    distance: 40.,
                },
            ],
        };
        world.brushes = vec![box_brush(0x20000, 20.), box_brush(1, 60.)];
        let mut physics = Physics::new(&world);
        physics.tick(0.015);
        let hit = physics
            .projectile_sweep(
                Vec3::ZERO,
                Vec3::X * 100.,
                ProjectileHull::Box(Vec3::splat(3.)),
                &[],
            )
            .unwrap();
        assert!((hit.position.x - 56.).abs() < 0.01);
        assert!(physics
            .projectile_ray(Vec3::ZERO, Vec3::X * 40., &[])
            .is_none());
        assert!(physics
            .projectile_ray(Vec3::ZERO, Vec3::X * 100., &[])
            .is_some());
    }
    #[test]
    fn ball_prop_contact_does_not_count_as_a_world_guidance_bounce() {
        let world = World {
            entities: vec![npc("prop_physics", Vec3::X * 40.)],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        cube(&mut physics, Vec3::X * 40., Vec3::splat(5.), Some(0));
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            launch(ProjectileKind::CombineBall, Vec3::ZERO, Vec3::X * 1000.),
            &mut scene,
        );
        scene.time = 0.06;
        assert!(projectiles
            .tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(1000.),
                false,
                0.06
            )
            .is_empty());
        assert_eq!(projectiles.active.len(), 1);
        assert!(projectiles.active[0].velocity.x < 0.);
        assert_eq!(projectiles.diagnostics.ball_bounces, 0);
        assert!(projectiles.effects.is_empty());
        assert!(!scene
            .sounds
            .iter()
            .any(|sound| sound == "NPC_CombineBall.Impact"));
    }
    #[test]
    fn nonfinite_launch_parameters_are_rejected_without_creating_particles_or_audio() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut projectiles = Projectiles::default();
        let mut invalid = launch(ProjectileKind::CombineBall, Vec3::ZERO, Vec3::X);
        invalid.radius = f32::NAN;
        projectiles.spawn(invalid, &mut scene);
        assert!(projectiles.active.is_empty());
        assert!(scene.sounds.is_empty());
        assert_eq!(projectiles.diagnostics.invalid_launches, 1);
    }
    #[test]
    fn whiz_sound_uses_forward_segment_distance_and_half_second_throttle() {
        assert!(!whiz_near_player(
            Vec3::ZERO,
            Vec3::X * 1000.,
            Vec3::X * 250.
        ));
        assert!(whiz_near_player(
            Vec3::ZERO,
            Vec3::X * 1000.,
            Vec3::X * 220.
        ));
        assert!(!whiz_near_player(
            Vec3::ZERO,
            -Vec3::X * 1000.,
            Vec3::X * 100.
        ));
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            launch(ProjectileKind::CombineBall, Vec3::ZERO, Vec3::X * 1000.),
            &mut scene,
        );
        scene.time = 0.03;
        projectiles.tick(
            &world,
            &mut scene,
            &mut physics,
            Vec3::X * 100.,
            false,
            0.015,
        );
        scene.time = 0.045;
        projectiles.tick(
            &world,
            &mut scene,
            &mut physics,
            Vec3::X * 100.,
            false,
            0.015,
        );
        assert_eq!(
            scene
                .sounds
                .iter()
                .filter(|sound| sound.as_ref() == "NPC_CombineBall.WhizFlyby")
                .count(),
            1
        );
        assert_eq!(projectiles.active[0].next_whiz, 0.53);
    }
}
