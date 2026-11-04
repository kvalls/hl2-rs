//! Moving SMG grenades and weapon-launched Combine balls. Launch rules use
//! reviewed retail methods; Rapier contacts and effects remain approximations.
use crate::{
    entities::Scene,
    physics::{Physics, ProjectileHull},
};
use glam::Vec3;

mod effects;
pub use effects::{Effect, EffectKind};
use modkit_core::World;
use serde::Serialize;

pub const GRENADE_MODEL: &str = "models/weapons/ar2_grenade.mdl";
pub const BALL_MODEL: &str = "models/effects/combineball.mdl";
pub const FRAG_MODEL: &str = "models/weapons/w_grenade.mdl";
/// grenade_frag.cpp: blips every 1 s, every 0.3 s once the AI warning (1.5 s before
/// detonation) has gone out.
const FRAG_BLIP: f64 = 1.;
const FRAG_BLIP_FAST: f64 = 0.3;
const FRAG_WARN: f64 = 1.5;
/// VPhysics contact with the world (surfaceprops grenade x default/concrete/tile).
const FRAG_FRICTION: f32 = 0.9 * 0.8;
/// GRENADE_COEFFICIENT_OF_RESTITUTION, only for characters (entity ray in VPhysicsUpdate).
const GRENADE_RESTITUTION: f32 = 0.2;
const FRAG_ELASTICITY: f32 = 0.01 * 0.2;
/// Normal speeds below this are resting/rolling contact rather than impacts
/// (gravity adds 9 u/s per tick); fitted with the native samples.
const FRAG_RESTING_SPEED: f32 = 60.;
/// Constant rolling/sliding deceleration on the ground, fitted to native (u/s^2).
const FRAG_ROLL_DECELERATION: f32 = 190.;
/// Effective VPhysics air drag of the thrown frag: dv/dt = -c |v| v (per unit).
/// The drag code is in vphysics.dll (not the SDK) and w_grenade.phy sets damping 0;
/// c is fitted to native HDR captures (2026-10-10: 8 ent_text samples of one throw,
/// RMS 3.7 units; an independent throw at another pitch/yaw predicted within 3 units).
const FRAG_AIR_DRAG: f32 = 6.4e-4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ProjectileKind {
    SmgGrenade,
    CombineBall,
    FragGrenade,
    /// crossbow_bolt (weapon_crossbow.rs).
    CrossbowBolt,
    /// rpg_missile (weapon_rpg.rs).
    RpgMissile,
    /// npc_grenade_bugbait (weapon_bugbait.rs).
    BugBait,
}
/// npc_grenade_frag timer state (CGrenadeFrag::SetTimer / DelayThink).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Fuse {
    pub detonate: f64,
    pub warn: f64,
    pub next_blip: f64,
    pub warned: bool,
    /// DelayThink runs every 0.1 s from SetTimer.
    pub next_think: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct ProjectileSpawn {
    /// Spawn orientation (Source angles); None faces the launch direction.
    pub angles: Option<Vec3>,
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
    pub(crate) damage: f32,
    last_bounce: f64,
    struck_entity: bool,
    next_whiz: f64,
    hit_entities: Vec<usize>,
    pub fuse: Option<Fuse>,
    /// MOVETYPE_FLYGRAVITY entity gravity scale (crossbow bolt 0.05, then 1.0).
    pub gravity: f32,
    /// Next think time (crossbow BubbleThink, missile IgniteThink).
    pub(crate) next_think: f64,
    /// rpg_missile after IgniteThink (MOVETYPE_FLY, w_missile.mdl).
    pub ignited: bool,
    /// FSOLID_NOT_SOLID until this time (missile grace period).
    pub(crate) solid_at: f64,
    /// The thrown frag's VPhysics body (w_grenade.phy) once the solid is loaded.
    #[serde(skip)]
    pub body: Option<rapier3d::prelude::RigidBodyHandle>,
    /// The swept frag lies on a floor and rolls about its long axis.
    pub grounded: bool,
    /// The thrower/owner (m_hThrower, GetOwnerEntity): the player for `spawn`, the NPC for
    /// `spawn_npc`; the attacker of the projectile's damage.
    pub owner: Attacker,
    /// That NPC's guiding laser dot (TASK_CIT_RPG_AUGER), set by the host each tick.
    pub npc_dot: Option<crate::weapon_rpg::LaserDot>,
}
impl Projectile {
    /// The NPC that launched this projectile, if not the player.
    pub fn npc_owner(&self) -> Option<usize> {
        match self.owner {
            Attacker::Entity(id) => Some(id),
            _ => None,
        }
    }
    pub fn model(&self) -> &'static str {
        match self.kind {
            ProjectileKind::SmgGrenade => GRENADE_MODEL,
            ProjectileKind::CombineBall => BALL_MODEL,
            ProjectileKind::FragGrenade => FRAG_MODEL,
            ProjectileKind::CrossbowBolt => crate::weapon_crossbow::BOLT_MODEL,
            ProjectileKind::RpgMissile if self.ignited => crate::weapon_rpg::MISSILE_MODEL,
            ProjectileKind::RpgMissile => crate::weapon_rpg::MISSILE_LAUNCH_MODEL,
            ProjectileKind::BugBait => crate::weapon_bugbait::BAIT_MODEL,
        }
    }
    pub fn physical_radius(&self) -> f32 {
        match self.kind {
            ProjectileKind::SmgGrenade => 3.,
            ProjectileKind::CombineBall => self.radius,
            // CGrenadeFrag::Spawn SetSize(-4, 4).
            ProjectileKind::FragGrenade => 4.,
            ProjectileKind::CrossbowBolt => 0.3,
            ProjectileKind::RpgMissile if self.ignited => 0.,
            ProjectileKind::RpgMissile => 4.,
            ProjectileKind::BugBait => 2.,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DamageTarget {
    Entity(usize),
    Player,
}
/// CTakeDamageInfo's attacker: who the damage is credited to (a projectile's thrower or
/// owner, the player for their weapons). PassesDamageFilter reads it
/// (`gameplay::damage_refused`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum Attacker {
    /// The world, triggers, flames and physics explosions (the entity itself or nothing:
    /// never refused).
    #[default]
    None,
    Player,
    Entity(usize),
}
#[derive(Clone, Copy, Debug)]
pub struct Damage {
    pub target: DamageTarget,
    pub amount: f32,
    /// DMG_* bits (player_damage).
    pub kind: u32,
    pub dissolve: bool,
    pub direction: Vec3,
    /// The inflictor's origin (the projectile or the trigger).
    pub origin: Vec3,
    pub attacker: Attacker,
}
#[derive(Default, Serialize)]
pub struct Diagnostics {
    pub grenades_spawned: u64,
    pub bolts_spawned: u64,
    pub missiles_spawned: u64,
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
    /// Crossbow bolts stuck in the world (C_TEStickyBolt temp models, never die in SP).
    pub stuck_bolts: Vec<crate::weapon_crossbow::StuckBolt>,
    /// UTIL_ImpactTrace requests from projectiles (decal and surface impact sound);
    /// the host drains them into its impact system.
    pub impacts: Vec<crate::physics::RayHit>,
    /// UTIL_DecalTrace requests: (trace hit, decal material).
    pub decals: Vec<(crate::physics::RayHit, &'static str)>,
    /// The player's RPG laser dot, copied from the weapon each tick by the host.
    pub laser_dot: Option<crate::weapon_rpg::LaserDot>,
    /// Missile deaths/removals this tick; the host notifies the RPG.
    pub missile_events: Vec<crate::weapon_rpg::MissileEvent>,
    /// NPC-owned missiles that exploded or left the world this tick (by owner).
    pub npc_missiles_died: Vec<usize>,
    /// Thrown bug bait splats this tick (step 14 antlion reactions consume them).
    pub bugbait_events: Vec<crate::weapon_bugbait::BugBaitEvent>,
    next_id: u64,
    next_effect_id: u64,
}
impl Projectiles {
    /// Whether this tick runs; it takes the tick's bullet impact effects first.
    fn start_tick(&mut self, scene: &mut Scene, dt: f32) -> bool {
        let runs = dt.is_finite() && dt > 0.;
        if runs {
            for effect in std::mem::take(&mut scene.impact_effects) {
                self.effect(effect);
            }
        }
        runs
    }
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
        let bolt = launch.kind == ProjectileKind::CrossbowBolt;
        if ball {
            self.diagnostics.balls_spawned += 1;
            scene.sounds.push("NPC_CombineBall.Launch".into());
        } else if bolt {
            self.diagnostics.bolts_spawned += 1;
        } else if launch.kind == ProjectileKind::RpgMissile {
            self.diagnostics.missiles_spawned += 1;
        } else {
            self.diagnostics.grenades_spawned += 1;
        }
        let frag = launch.kind == ProjectileKind::FragGrenade;
        if frag {
            // Spawn blips once; the timer (lifetime) starts with the throw.
            scene.sounds.push(crate::sounds::SoundRequest {
                origin: Some(launch.position),
                .."Grenade.Blip".into()
            });
        }
        // CMissile::Create keeps the aim angles while the launch velocity is
        // forward x 300 + 128 up; recover forward from that.
        let direction = if launch.kind == ProjectileKind::RpgMissile {
            ((launch.velocity - Vec3::Z * 128.) / 300.).normalize_or_zero()
        } else {
            launch.velocity.normalize_or_zero()
        };
        self.active.push(Projectile {
            id: self.next_id,
            kind: launch.kind,
            position: launch.position,
            previous_position: launch.position,
            velocity: launch.velocity,
            angles: if let Some(angles) = launch.angles {
                angles
            } else {
                Vec3::new(
                    (-direction.z)
                        .atan2(direction.truncate().length())
                        .to_degrees(),
                    direction.y.atan2(direction.x).to_degrees(),
                    0.,
                )
            },
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
            fuse: frag.then(|| {
                let detonate = launch.at + f64::from(launch.lifetime.max(0.));
                Fuse {
                    detonate,
                    warn: detonate - FRAG_WARN,
                    next_blip: launch.at + FRAG_BLIP,
                    warned: false,
                    next_think: launch.at,
                }
            }),
            gravity: if bolt {
                crate::weapon_crossbow::INITIAL_GRAVITY
            } else {
                1.
            },
            // CCrossbowBolt::Spawn: BubbleThink 0.1 s after spawn; CMissile::Spawn:
            // IgniteThink after 0.3 s.
            next_think: launch.at
                + if launch.kind == ProjectileKind::RpgMissile {
                    0.3
                } else {
                    0.1
                },
            ignited: false,
            // The missile's grace period travels in `lifetime`.
            solid_at: if matches!(
                launch.kind,
                ProjectileKind::RpgMissile | ProjectileKind::BugBait
            ) {
                launch.at + f64::from(launch.lifetime.max(0.))
            } else {
                launch.at
            },
            body: None,
            grounded: false,
            owner: Attacker::Player,
            npc_dot: None,
        });
    }
    /// A projectile an NPC launched (Fraggrenade_Create's thrower, CMissile::Create's
    /// owner). Returns the projectile id.
    pub fn spawn_npc(
        &mut self,
        launch: ProjectileSpawn,
        owner: usize,
        scene: &mut Scene,
    ) -> Option<u64> {
        let count = self.active.len();
        self.spawn(launch, scene);
        let projectile = self.active.get_mut(count)?;
        projectile.owner = Attacker::Entity(owner);
        Some(projectile.id)
    }
    /// An NPC's combine ball (CWeaponAR2::FireNPCSecondaryAttack -> CreateCombineBall with
    /// the NPC as owner: the ball never strikes its shooter).
    pub fn spawn_npc_ball(&mut self, launch: ProjectileSpawn, owner: usize, scene: &mut Scene) {
        let count = self.active.len();
        self.spawn_npc(launch, owner, scene);
        if let Some(ball) = self.active.get_mut(count) {
            ball.hit_entities.push(owner);
        }
    }
    /// An NPC's RPG missile (CWeaponRPG::Operator_HandleAnimEvent -> CMissile::Create with
    /// the NPC as owner). Returns the projectile id.
    pub fn spawn_npc_missile(
        &mut self,
        launch: ProjectileSpawn,
        owner: usize,
        scene: &mut Scene,
    ) -> Option<u64> {
        self.spawn_npc(launch, owner, scene)
    }
    /// CBreakableProp::Break -> ExplosionCreate for the explosive props broken by this
    /// tick's damage (SURFACEONLY). Native explodes before PropBreakableCreateAll, so
    /// the host calls this after damage and before gibs spawn; the broken prop is
    /// already removed, so RadiusDamage's ignored entity needs no filter. Returns the
    /// blast damage (which can break further explosive props: the host repeats).
    pub fn explode_props(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &mut Physics,
        player_feet: Vec3,
        player_ducked: bool,
    ) -> Vec<Damage> {
        let mut damage = Vec::new();
        for (center, magnitude, radius, burst, attacker) in
            std::mem::take(&mut scene.prop_explosions)
        {
            let sounds = scene.sounds.len();
            self.grenade_explosion(
                center,
                magnitude,
                radius,
                Vec3::Z,
                attacker,
                world,
                scene,
                physics,
                player_feet,
                player_ducked,
                &mut damage,
            );
            if burst {
                scene.sounds.truncate(sounds);
                scene.sounds.push(crate::sounds::SoundRequest {
                    origin: Some(center),
                    .."PropaneTank.Burst".into()
                });
            }
        }
        damage
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
        if !self.start_tick(scene, dt) {
            return Vec::new();
        }
        self.effects
            .retain(|effect| scene.time - effect.at <= effect.duration());
        let mut damage = Vec::new();
        let mut survivors = Vec::with_capacity(self.active.len());
        let bodies: Vec<_> = self.active.iter().filter_map(|p| p.body).collect();
        for mut projectile in std::mem::take(&mut self.active) {
            projectile.previous_position = projectile.position;
            if projectile.position.abs().max_element() > 16384. {
                self.diagnostics.out_of_world += 1;
                if projectile.kind == ProjectileKind::RpgMissile {
                    if let Some(owner) = projectile.npc_owner() {
                        self.npc_missiles_died.push(owner);
                    } else {
                        self.missile_events
                            .push(crate::weapon_rpg::MissileEvent::Removed);
                    }
                }
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
                            projectile.position,
                            projectile.damage,
                            projectile.radius,
                            hit.normal,
                            projectile.owner,
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
                ProjectileKind::FragGrenade => {
                    let mut fuse = projectile.fuse.expect("frag fuse");
                    // Think times are whole ticks (TIME_TO_TICKS); compare without float drift.
                    let think = scene.time + 1e-6 >= fuse.next_think;
                    if think {
                        fuse.next_think = scene.time + 0.1;
                    }
                    if think && scene.time > fuse.detonate {
                        self.grenade_explosion(
                            projectile.position,
                            projectile.damage,
                            projectile.radius,
                            Vec3::Z,
                            projectile.owner,
                            world,
                            scene,
                            physics,
                            player_feet,
                            player_ducked,
                            &mut damage,
                        );
                        alive = false;
                    } else {
                        fuse.warned |= think && scene.time >= fuse.warn;
                        if think && scene.time > fuse.next_blip {
                            scene.sounds.push(crate::sounds::SoundRequest {
                                origin: Some(projectile.position),
                                .."Grenade.Blip".into()
                            });
                            fuse.next_blip = scene.time
                                + if fuse.warned {
                                    FRAG_BLIP_FAST
                                } else {
                                    FRAG_BLIP
                                };
                        }
                        projectile.fuse = Some(fuse);
                        let npc = |id: usize| {
                            world
                                .entities
                                .get(id)
                                .is_some_and(|e| e.class().starts_with("npc_"))
                        };
                        if world.model_physics.contains_key(&frag_key()) {
                            // VPhysics body: contacts, sounds (physics collision events)
                            // and rotation come from the rigid-body step.
                            let body = *projectile.body.get_or_insert_with(|| {
                                let body = physics.add_projectile_body(
                                    world,
                                    &frag_key(),
                                    Vec3::splat(4.),
                                    projectile.position,
                                    crate::physics::angles(projectile.angles),
                                    projectile.velocity,
                                    projectile.angular_velocity,
                                );
                                // CBaseGrenade: FCAP_IMPULSE_USE | FCAP_USE_IN_RADIUS, and
                                // Use(USE_TOGGLE) -> PickupObject; gravity guns hold it too.
                                physics.bind_projectile_entity(
                                    body,
                                    crate::physics::projectile_entity(projectile.id),
                                );
                                body
                            });
                            frag_vphysics_update(&mut projectile, body, physics, dt, &npc);
                            projectile.fuse = Some(fuse);
                            if alive {
                                survivors.push(projectile);
                            }
                            continue;
                        }
                        let impact = frag_motion(&mut projectile, physics, dt, &npc);
                        // PhysCollisionSound: >= 70 u/s and 0.05 s since the last collision,
                        // volume (speed / 320)^2; the grenade surfaceprop always picks its
                        // ImpactHard sound (hardness 1.0, no hard thresholds).
                        if impact >= 70. && scene.time - projectile.last_bounce >= 0.05 {
                            projectile.last_bounce = scene.time;
                            scene.sounds.push(crate::sounds::SoundRequest {
                                origin: Some(projectile.position),
                                volume: Some((impact / 320.).powi(2).min(1.)),
                                .."Grenade.ImpactHard".into()
                            });
                        }
                    }
                }
                ProjectileKind::CrossbowBolt => {
                    let outcome = crate::weapon_crossbow::bolt_tick(
                        &mut projectile,
                        world,
                        scene,
                        physics,
                        dt,
                        &mut damage,
                    );
                    alive = outcome.alive;
                    if let Some(hit) = outcome.impact {
                        self.impacts.push(hit);
                    }
                    if let Some((origin, direction)) = outcome.stuck {
                        // The BoltImpact effect: StickRagdollNow pins a ragdoll in front
                        // of the wall point, then CreateCrossbowBolt.
                        scene.ragdoll_impacts.push(crate::ragdolls::Impact::Bolt {
                            origin,
                            direction,
                            retried: false,
                        });
                        crate::weapon_crossbow::stick(&mut self.stuck_bolts, origin, direction);
                    }
                    for (kind, at) in [
                        (EffectKind::BoltGlow, outcome.glow),
                        (EffectKind::Sparks, outcome.sparks),
                    ] {
                        if let Some(position) = at {
                            self.effect(Effect {
                                id: 0,
                                magnitude: 0.,
                                kind,
                                position,
                                normal: Vec3::Z,
                                at: scene.time,
                                radius: 0.,
                            });
                        }
                    }
                }
                ProjectileKind::RpgMissile => {
                    // SeekThink follows only dots of the missile's own owner.
                    let dot = if projectile.npc_owner().is_some() {
                        projectile.npc_dot
                    } else {
                        self.laser_dot
                    };
                    if let crate::weapon_rpg::MissileOutcome::Explode =
                        crate::weapon_rpg::missile_tick(
                            &mut projectile,
                            dot.as_ref(),
                            scene,
                            physics,
                            dt,
                        )
                    {
                        // Explode: StopSound("Missile.Ignite"), DoExplosion
                        // (ExplosionCreate 200/200) through the shared blast, then
                        // NotifyRocketDied.
                        scene.sounds.push(crate::sounds::SoundRequest::stop(
                            "Missile.Ignite",
                            crate::sounds::projectile_emitter(projectile.id),
                        ));
                        self.grenade_explosion(
                            projectile.position,
                            projectile.damage,
                            projectile.radius,
                            Vec3::Z,
                            projectile.owner,
                            world,
                            scene,
                            physics,
                            player_feet,
                            player_ducked,
                            &mut damage,
                        );
                        if let Some(owner) = projectile.npc_owner() {
                            self.npc_missiles_died.push(owner);
                        } else {
                            self.missile_events
                                .push(crate::weapon_rpg::MissileEvent::Died);
                        }
                        alive = false;
                    }
                }
                ProjectileKind::BugBait => {
                    if let Some(at) =
                        crate::weapon_bugbait::bait_tick(&mut projectile, physics, scene.time, dt)
                    {
                        // BugBaitTouch: SporeExplosion facing back along the flight, a
                        // BeerSplash decal where a 64-unit MASK_SHOT trace along the
                        // velocity hits, the splat sound, sensors/antlion call, removal.
                        let direction = projectile.velocity.normalize_or_zero();
                        self.effect(Effect {
                            id: 0,
                            magnitude: 0.,
                            kind: EffectKind::SporeExplosion,
                            position: at,
                            normal: -direction,
                            at: scene.time,
                            radius: 32.,
                        });
                        if let Some(hit) = physics.projectile_ray(at, at + direction * 64., &[]) {
                            self.decals.push((hit, crate::weapon_bugbait::SPLAT_DECAL));
                        }
                        scene.sounds.push(crate::sounds::SoundRequest {
                            origin: Some(at),
                            .."GrenadeBugBait.Splat".into()
                        });
                        let (suppress, combine) = crate::weapon_bugbait::activate_targets(
                            world,
                            scene,
                            Some(physics),
                            at,
                            false,
                        );
                        self.bugbait_events
                            .push(crate::weapon_bugbait::BugBaitEvent {
                                position: at,
                                squeezed: false,
                                call: !suppress,
                                combine,
                            });
                        alive = false;
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
                            } else if ball_dissolves(
                                world,
                                entity.class(),
                                projectile.owner,
                                scene.globals.is_on("gordon_precriminal")
                                    || scene.globals.is_on("citizens_passive"),
                            ) {
                                projectile.hit_entities.push(hit.entity);
                                damage.push(Damage {
                                    target: DamageTarget::Entity(hit.entity),
                                    amount: 0.,
                                    kind: crate::player_damage::DMG_DISSOLVE,
                                    dissolve: true,
                                    direction: projectile.velocity.normalize_or_zero(),
                                    origin: projectile.position,
                                    attacker: projectile.owner,
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
        // Removed projectiles (detonated, out of the world) release their bodies.
        for body in bodies {
            if !survivors.iter().any(|p| p.body == Some(body)) {
                physics.remove_body(body);
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
        position: Vec3,
        magnitude: f32,
        radius: f32,
        normal: Vec3,
        attacker: Attacker,
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
            magnitude,
            kind: EffectKind::GrenadeExplosion,
            position,
            normal,
            at: scene.time,
            radius,
        });
        scene.sounds.push("BaseExplosionEffect.Sound".into());
        // The explosion temp entity pushes client ragdolls (C_TEExplosion::AffectRagdolls).
        scene.ragdoll_impacts.push(crate::ragdolls::Impact::Blast {
            origin: position,
            magnitude,
            radius,
        });
        let origin = position + Vec3::Z;
        if radius <= 0. {
            return;
        }
        for (id, entity) in world.entities.iter().enumerate() {
            let state = &scene.states[id];
            if state.killed || !state.enabled {
                continue;
            }
            // prop_physics health comes from prop_data or the health keyvalue; the
            // entity damage dispatch resolves both and skips unbreakable props.
            let damageable = entity.class().starts_with("npc_")
                || matches!(entity.class(), "func_breakable" | "func_physbox")
                || entity.class().starts_with("prop_physics");
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
            let mut amount = magnitude * (1. - delta.length() / radius);
            if amount <= 0. {
                continue;
            }
            match blast_blocked_fraction(world, physics, origin, target, id) {
                Some(blocked) => amount -= amount * blocked,
                None => {
                    self.diagnostics.occluded_blast_targets += 1;
                    continue;
                }
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
                        kind: crate::player_damage::DMG_BLAST,
                        dissolve: false,
                        direction: delta.normalize_or_zero(),
                        origin,
                        attacker,
                    });
                }
            }
        }
        let maxs = player_feet + Vec3::new(16., 16., if player_ducked { 36. } else { 72. });
        let mins = player_feet - Vec3::new(16., 16., 0.);
        let target = origin.clamp(mins, maxs);
        let amount = magnitude * (1. - origin.distance(target) / radius);
        if amount > 0. && physics.projectile_ray(origin, target, &[]).is_none() {
            damage.push(Damage {
                target: DamageTarget::Player,
                amount,
                kind: crate::player_damage::DMG_BLAST,
                dissolve: false,
                direction: (target - origin).normalize_or_zero(),
                origin,
                attacker,
            });
        }
    }
}
/// CGameRules::RadiusDamage line of sight (MASK_RADIUS_DAMAGE, then the CONTENTS_SOLID
/// re-trace): `None` when the world or a brush entity blocks the blast, else the
/// fraction of the damage an interposing entity absorbs. The re-trace's
/// StandardFilterRules skips every non-brush model (props, NPCs) without
/// CONTENTS_MONSTER, so a prop in the way never shields fully: a physics object absorbs
/// mass / 350 (all from 350 kg), anything else 25%. Not modeled: the robust NPC edge
/// spill (sv_robust_explosions) and door classes beyond brush models; the target point
/// is the closest collision point rather than BodyTarget.
fn blast_blocked_fraction(
    world: &World,
    physics: &Physics,
    origin: Vec3,
    target: Vec3,
    id: usize,
) -> Option<f32> {
    let Some(hit) = physics.projectile_ray(origin, target, &[id]) else {
        return Some(0.);
    };
    let brush = |entity: usize| {
        world
            .entities
            .get(entity)
            .map_or(entity == usize::MAX, |e| {
                e.get("model").is_some_and(|m| m.starts_with('*'))
            })
    };
    if brush(hit.entity)
        || physics
            .filtered_ray(origin, target, &|entity| entity != id && brush(entity))
            .is_some()
    {
        return None;
    }
    match physics.entity_mass(hit.entity) {
        Some(mass) if mass >= 350. => None,
        Some(mass) => Some(mass / 350.),
        None => Some(0.25),
    }
}
/// Frag grenade motion. Native uses a VPhysics object (w_grenade.phy, surfaceprop
/// "grenade": friction 0.9, elasticity 0.01). CGrenadeFrag::VPhysicsUpdate's 0.2
/// restitution reflection only applies to entities its collision group skips; world
/// contacts are VPhysics: practically no bounce, and an impact removes tangential speed
/// by the Coulomb impulse friction x |normal speed| (0.9 x the 0.8 of default, concrete,
/// tile and metal). Resting/rolling contact slows by a constant deceleration and air
/// drag. The impact and rolling values match native ent_text samples (2026-10-10:
/// lob landing 350 -> ~170 u/s, roll curb drop ~530 -> 335 u/s, street roll decay).
/// The body is a swept 4-unit box under sv_gravity 600.
/// Returns the strongest impact (normal) speed of this tick, 0 without an impact.
fn frag_motion(
    projectile: &mut Projectile,
    physics: &Physics,
    dt: f32,
    npc: &dyn Fn(usize) -> bool,
) -> f32 {
    let speed = projectile.velocity.length();
    projectile.velocity *= (1. - FRAG_AIR_DRAG * speed * dt).max(0.);
    projectile.velocity.z -= 600. * dt;
    let mut remaining = dt;
    let mut rolling = false;
    let mut impact = 0f32;
    for _ in 0..4 {
        let end = projectile.position + projectile.velocity * remaining;
        let Some(hit) = physics.projectile_sweep(
            projectile.position,
            end,
            ProjectileHull::Box(Vec3::splat(4.)),
            &[],
        ) else {
            projectile.position = end;
            break;
        };
        let normal = hit.normal.normalize_or_zero();
        projectile.position = hit.position + normal * 0.03125;
        let into = projectile.velocity.dot(normal);
        if npc(hit.entity) {
            // COLLISION_GROUP_WEAPON passes through characters in VPhysics, so
            // VPhysicsUpdate's ray reflects off them: keep 0.2, reverse half the spin.
            // (Its 0.1 DMG_CRUSH "bonk" for NPC reactions belongs to step 14.)
            if into < 0. {
                projectile.velocity =
                    (projectile.velocity - 2. * into * normal) * GRENADE_RESTITUTION;
                projectile.angular_velocity *= -0.5;
            }
        } else if into < 0. {
            let tangent = projectile.velocity - into * normal;
            let tangent_speed = tangent.length();
            if -into < FRAG_RESTING_SPEED {
                // Resting contact: keep the tangential motion so gravity can carry the
                // body off edges and slopes; floors add the rolling resistance.
                rolling |= normal.z > 0.7;
                projectile.velocity = tangent;
            } else {
                impact = impact.max(-into);
                let friction = (FRAG_FRICTION * -into).min(tangent_speed);
                projectile.velocity = tangent * (1. - friction / tangent_speed.max(1e-6))
                    - into * normal * FRAG_ELASTICITY;
            }
        }
        remaining *= 1. - hit.fraction.clamp(0., 1.);
        if remaining <= 1e-6 {
            break;
        }
    }
    if rolling {
        let speed = projectile.velocity.length();
        projectile.velocity *= (1. - FRAG_ROLL_DECELERATION * dt / speed.max(1e-6)).max(0.);
    }
    if rolling || (impact > 0. && projectile.grounded) {
        projectile.grounded = true;
    } else if projectile.velocity.z.abs() > 60. {
        projectile.grounded = false;
    }
    if projectile.grounded {
        frag_roll(projectile, dt);
    } else {
        projectile.angles += projectile.angular_velocity * dt;
    }
    impact
}
/// w_grenade.phy's radius about its long (model z) axis, units.
const FRAG_ROLL_RADIUS: f32 = 1.8;
/// On a floor the swept frag lies on its side, long axis horizontal and across its
/// motion, and rolls about that axis by distance / radius (native frags roll along their
/// own axis after landing; the swept box has no rotational dynamics, so the free spin
/// is dropped here instead of turning the model about its origin into the floor).
fn frag_roll(projectile: &mut Projectile, dt: f32) {
    use glam::Quat;
    let current = crate::physics::angles(projectile.angles);
    let horizontal = Vec3::new(projectile.velocity.x, projectile.velocity.y, 0.);
    let speed = horizontal.length();
    // The roll axis: across the motion, else the current long axis laid flat.
    let mut axis = if speed > 1. {
        Vec3::new(-horizontal.y, horizontal.x, 0.) / speed
    } else {
        let long = current * Vec3::Z;
        Vec3::new(long.x, long.y, 0.).normalize_or(Vec3::Y)
    };
    let long = current * Vec3::Z;
    if long.dot(axis) < 0. {
        axis = -axis;
    }
    // Swing the long axis onto the roll axis by the shortest rotation, keeping the
    // grenade's own roll, then roll about it.
    let lay = Quat::from_rotation_arc(long.normalize_or(Vec3::Z), axis);
    let roll = Quat::from_axis_angle(axis, speed * dt / FRAG_ROLL_RADIUS);
    projectile.angles = crate::projectiles::quat_angles((roll * lay * current).normalize());
    projectile.angular_velocity = Vec3::ZERO;
}
/// World::model_physics key of the frag model (skin 0).
pub fn frag_key() -> String {
    format!("{FRAG_MODEL}#0")
}
/// Source angles (pitch, yaw, roll) of a rotation built by physics::angles.
pub fn quat_angles(rotation: glam::Quat) -> Vec3 {
    let (yaw, pitch, roll) = rotation.to_euler(glam::EulerRot::ZYX);
    Vec3::new(pitch, yaw, roll) * (180. / std::f32::consts::PI)
}
/// CGrenadeFrag::VPhysicsUpdate on the rigid body: read back the body's motion, then
/// ray-cast this tick's travel against characters (the entities COLLISION_GROUP_WEAPON
/// skips) and reflect at GRENADE_COEFFICIENT_OF_RESTITUTION with -0.5 spin. Not done:
/// the startsolid "bounce backwards" case and the 0.1 DMG_CRUSH bonk (step 14).
fn frag_vphysics_update(
    projectile: &mut Projectile,
    body: rapier3d::prelude::RigidBodyHandle,
    physics: &mut Physics,
    dt: f32,
    npc: &dyn Fn(usize) -> bool,
) {
    let Some(state) = physics.body_state(body) else {
        return;
    };
    projectile.position = state.position;
    projectile.velocity = state.velocity;
    projectile.angles = quat_angles(state.rotation);
    projectile.angular_velocity = state.local_angular_degrees;
    let end = state.position + state.velocity * dt;
    if let Some(hit) = physics.projectile_ray(state.position, end, &[]) {
        if npc(hit.entity) {
            let v = state.velocity;
            let reflected = (v - 2. * hit.normal * v.dot(hit.normal)) * GRENADE_RESTITUTION;
            let spin = state.local_angular_degrees * -0.5;
            physics.set_body_velocity(body, reflected, spin);
            projectile.velocity = reflected;
            projectile.angular_velocity = spin;
        }
    }
}
/// CPropCombineBall::IsAttractiveTarget for a player's ball: not striders, nor the
/// player's allies (CLASS_PLAYER_ALLY / _VITAL); passive citizens still attract it.
fn ball_attracted(class: &str, citizens_passive: bool) -> bool {
    use crate::ai::relationships::{classify, Class};
    class != "npc_strider"
        && !matches!(
            classify(class, citizens_passive),
            Class::Player | Class::PlayerAlly | Class::PlayerAllyVital
        )
}
/// CPropCombineBall::OnHitEntity for a player's ball: no hit on an NPC whose
/// IRelationType toward the owner (the player) is D_LI; otherwise DissolveEntity, which
/// refuses EFL_NO_DISSOLVE (set in Spawn by every citizen, Alyx, Barney, Breen, Eli,
/// Kleiner, Mossman, the G-Man, scanners, manhacks, rollermines, turrets, gunships,
/// antlion guards and bullseyes; SDK 2013 hl2/npc_*.cpp). The ball bounces either way.
/// F4 per-spawn allegiance is not consulted here yet.
/// CPropCombineBall::OnHitEntity's bShouldHit: the struck character's disposition toward
/// the ball's owner is not D_LI (balls never dissolve the owner's friends).
fn ball_dissolves(world: &World, class: &str, owner: Attacker, citizens_passive: bool) -> bool {
    use crate::ai::relationships::{classify, Class, Disposition, Relationships};
    const NO_DISSOLVE: &[&str] = &[
        "npc_citizen",
        "npc_alyx",
        "npc_barney",
        "npc_breen",
        "npc_eli",
        "npc_kleiner",
        "npc_mossman",
        "npc_gman",
        "npc_fisherman",
        "npc_cscanner",
        "npc_clawscanner",
        "npc_manhack",
        "npc_rollermine",
        "npc_turret_floor",
        "npc_turret_ceiling",
        "npc_turret_ground",
        "npc_combinegunship",
        "npc_antlionguard",
        "npc_bullseye",
        "npc_strider",
    ];
    if NO_DISSOLVE.contains(&class) {
        return false;
    }
    let subject = classify(class, citizens_passive);
    let owner = match owner {
        Attacker::Player => Class::Player,
        Attacker::Entity(id) => world
            .entities
            .get(id)
            .map_or(Class::None, |e| classify(e.class(), citizens_passive)),
        Attacker::None => Class::None,
    };
    subject == Class::None
        || owner == Class::None
        || Relationships::default().get(subject, owner).0 != Disposition::LI
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
            || !ball_attracted(
                entity.class(),
                scene.globals.is_on("gordon_precriminal")
                    || scene.globals.is_on("citizens_passive"),
            )
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
    #[test]
    fn player_combine_balls_dissolve_enemies_and_bounce_off_no_dissolve_and_allies() {
        use super::{ball_dissolves, Attacker};
        let world = World {
            entities: vec![modkit_core::Entity {
                properties: vec![("classname".into(), "npc_combine_s".into())],
            }],
            ..Default::default()
        };
        let player = |class, passive| ball_dissolves(&world, class, Attacker::Player, passive);
        assert!(player("npc_metropolice", true));
        assert!(player("npc_combine_s", false));
        assert!(player("npc_zombie", false));
        assert!(player("npc_antlion", false));
        // EFL_NO_DISSOLVE: citizens (passive or rebel), vital allies, scanners.
        assert!(!player("npc_citizen", true));
        assert!(!player("npc_citizen", false));
        assert!(!player("npc_barney", false));
        assert!(!player("npc_cscanner", false));
        // D_LI toward the player: the vortigaunt and the dog are not hit.
        assert!(!player("npc_vortigaunt", false));
        assert!(!player("npc_dog", false));
        // An elite's ball: the struck character's disposition toward the soldier.
        let elite = |class| ball_dissolves(&world, class, Attacker::Entity(0), false);
        assert!(!elite("npc_combine_s"));
        assert!(elite("npc_zombie"));
    }
    #[test]
    fn interposing_props_absorb_blast_by_mass_instead_of_shielding() {
        // Synthetic: a 5 kg and a 400 kg prop between a blast at the origin and a target
        // at x 100 (RadiusDamage: physics blockers absorb mass / 350, all from 350 kg).
        let mut world = World::default();
        for mass in [5., 400., 1.] {
            world.model_physics.insert(
                format!("models/m{mass}.mdl#0"),
                modkit_core::PhysicsSolid {
                    mass,
                    ..Default::default()
                },
            );
        }
        world.entities = vec![modkit_core::Entity { properties: vec![] }; 3];
        let mut physics = Physics::new(&world);
        let mut add = |id: usize, mass: f32, x: f32, y: f32| {
            let body = physics.add_prop_body(
                &world,
                &format!("models/m{mass}.mdl#0"),
                Vec3::splat(4.),
                Vec3::new(x, y, 0.),
                glam::Quat::IDENTITY,
                Vec3::ZERO,
                Vec3::ZERO,
            );
            physics.bind_projectile_entity(body, id);
        };
        add(0, 5., 50., 0.);
        add(1, 400., 50., 40.);
        add(2, 1., 100., 0.);
        physics.refresh_queries();
        let target = Vec3::new(96., 0., 0.);
        let light = blast_blocked_fraction(&world, &physics, Vec3::ZERO, target, 2).unwrap();
        assert!((light - 5. / 350.).abs() < 1e-3, "{light}");
        let heavy = Vec3::new(96., 80., 0.);
        assert_eq!(
            blast_blocked_fraction(&world, &physics, Vec3::ZERO, heavy, 2),
            None
        );
        let clear = Vec3::new(0., -96., 0.);
        assert_eq!(
            blast_blocked_fraction(&world, &physics, Vec3::ZERO, clear, 2),
            Some(0.)
        );
    }
    #[test]
    fn frag_fuse_blips_then_detonates_after_three_seconds() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                angles: None,
                kind: ProjectileKind::FragGrenade,
                position: Vec3::Z * 1000.,
                velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                at: 0.,
                damage: 125.,
                radius: 250.,
                mass: 0.,
                lifetime: 3.,
            },
            &mut scene,
        );
        let mut blips = vec![];
        let mut detonated = None;
        for tick in 0..240 {
            scene.time = f64::from(tick) * 0.015;
            projectiles.tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(9999.),
                false,
                0.015,
            );
            for s in scene.sounds.drain(..) {
                if s.name == "Grenade.Blip" {
                    blips.push(scene.time);
                }
            }
            if detonated.is_none() && projectiles.diagnostics.grenade_detonations == 1 {
                detonated = Some(scene.time);
            }
        }
        // DelayThink every 0.1 s rounds to 7 ticks (0.105 s): spawn blip, 1 s spacing,
        // 0.3 s after the 1.5 s warning, detonation on the first think after 3 s.
        let expected = [0., 1.05, 2.1, 2.415, 2.73];
        assert_eq!(blips.len(), expected.len(), "{blips:?}");
        for (blip, at) in blips.iter().zip(expected) {
            assert!((blip - at).abs() < 0.005, "{blips:?}");
        }
        let at = detonated.unwrap();
        assert!((at - 3.045).abs() < 0.005, "{at}");
    }
    #[test]
    fn frag_flight_follows_native_vphysics_drag_samples() {
        // Native d1_trainstation_02 throw (eye -3104,-2018,128; setang -30 -20), positions
        // read from ent_text at host_timescale 0.1; spawn ~0.045 s after the release.
        let samples = [
            (0.199, Vec3::new(-2954.4, -2080.9, 231.4)),
            (0.524, Vec3::new(-2695.9, -2175.1, 365.2)),
            (0.847, Vec3::new(-2489.1, -2250.2, 416.1)),
            (1.166, Vec3::new(-2298.1, -2319.8, 402.9)),
            (1.335, Vec3::new(-2210.7, -2351.5, 373.4)),
        ];
        let (pitch, yaw) = (30f32.to_radians(), (-20f32).to_radians());
        let forward = Vec3::new(
            pitch.cos() * yaw.cos(),
            pitch.cos() * yaw.sin(),
            pitch.sin(),
        );
        let right = Vec3::new(yaw.sin(), -yaw.cos(), 0.);
        let eye = Vec3::new(-3104., -2018., 128.);
        let position = eye + forward * 18. + right * 8.;
        let mut projectile = Projectile {
            id: 1,
            kind: ProjectileKind::FragGrenade,
            position,
            previous_position: position,
            velocity: (forward + Vec3::Z * 0.1) * 1200.,
            angles: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            spawned_at: 0.,
            radius: 250.,
            mass: 0.,
            expires_at: None,
            damage: 125.,
            last_bounce: -1.,
            struck_entity: false,
            next_whiz: 0.,
            hit_entities: vec![],
            fuse: None,
            gravity: 1.,
            next_think: 0.,
            ignited: false,
            solid_at: 0.,
            body: None,
            grounded: false,
            owner: Attacker::Player,
            npc_dot: None,
        };
        let physics = Physics::new(&World::default());
        let mut t = 0.045;
        for (at, expected) in samples {
            while t + 0.015 <= at {
                frag_motion(&mut projectile, &physics, 0.015, &|_| false);
                t += 0.015;
            }
            let position = projectile.position + projectile.velocity * (at - t) as f32;
            assert!(
                position.distance(expected) < 8.,
                "{at}: {position} vs {expected}"
            );
        }
    }
    use super::*;
    use rapier3d::prelude::*;

    fn launch(kind: ProjectileKind, position: Vec3, velocity: Vec3) -> ProjectileSpawn {
        ProjectileSpawn {
            angles: None,
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
    fn frag_reflects_off_characters_with_sdk_restitution_and_no_physics_sound() {
        let world = World {
            entities: vec![npc("npc_metropolice", Vec3::new(60., 0., 1000.))],
            ..World::default()
        };
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        cube(
            &mut physics,
            Vec3::new(60., 0., 1000.),
            Vec3::new(8., 16., 36.),
            Some(0),
        );
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                angles: None,
                kind: ProjectileKind::FragGrenade,
                position: Vec3::new(0., 0., 1000.),
                velocity: Vec3::new(500., 0., 0.),
                angular_velocity: Vec3::new(600., 400., 0.),
                at: 0.,
                damage: 125.,
                radius: 250.,
                mass: 0.,
                lifetime: 3.,
            },
            &mut scene,
        );
        let mut reflected = None;
        for tick in 0..20 {
            scene.time = f64::from(tick) * 0.015;
            projectiles.tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(9999.),
                false,
                0.015,
            );
            assert!(scene.sounds.iter().all(|s| s.name != "Grenade.ImpactHard"));
            scene.sounds.clear();
            let frag = &projectiles.active[0];
            if reflected.is_none() && frag.velocity.x < 0. {
                reflected = Some((frag.velocity, frag.angular_velocity));
            }
        }
        let (velocity, spin) = reflected.expect("bounced off the NPC");
        // 0.2 of the incoming ~485 u/s (air drag before the hit).
        assert!((velocity.x + 97.).abs() < 3., "{velocity}");
        assert_eq!(spin, Vec3::new(-300., -200., 0.));
    }
    #[test]
    fn frag_floor_impact_plays_one_sound_and_applies_the_friction_impulse() {
        let world = World::default();
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        cube(
            &mut physics,
            Vec3::new(0., 0., -8.),
            Vec3::new(2000., 2000., 8.),
            None,
        );
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                angles: None,
                kind: ProjectileKind::FragGrenade,
                position: Vec3::new(0., 0., 60.),
                velocity: Vec3::new(350., 0., 0.),
                angular_velocity: Vec3::ZERO,
                at: 0.,
                damage: 125.,
                radius: 250.,
                mass: 0.,
                lifetime: 3.,
            },
            &mut scene,
        );
        let mut impacts = vec![];
        let mut before = Vec3::ZERO;
        let mut after = None;
        for tick in 0..120 {
            scene.time = f64::from(tick) * 0.015;
            let previous = projectiles.active[0].velocity;
            projectiles.tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(9999.),
                false,
                0.015,
            );
            for sound in scene.sounds.drain(..) {
                if sound.name == "Grenade.ImpactHard" {
                    impacts.push(sound.volume.unwrap());
                    before = previous;
                    after.get_or_insert(projectiles.active[0].velocity);
                }
            }
        }
        // One landing (rolling contact afterwards is silent), volume (speed / 320)^2.
        assert_eq!(impacts.len(), 1, "{impacts:?}");
        let fall = -before.z + 9.;
        assert!(
            (impacts[0] - (fall / 320.).powi(2).min(1.)).abs() < 0.08,
            "{impacts:?} {fall}"
        );
        // Tangential speed loses friction 0.72 x the normal speed; practically no bounce.
        let after = after.unwrap();
        assert!(
            (after.x - (before.x - 0.72 * fall)).abs() < 12.,
            "{before} -> {after}"
        );
        assert!(after.z.abs() < 5., "{after}");
    }
    #[test]
    fn frag_with_a_loaded_solid_is_a_rigid_body_that_lands_rolls_sounds_and_is_released() {
        // Synthetic solid/surface values written for this test (not game data).
        let mut world = World::default();
        world.model_physics.insert(
            frag_key(),
            modkit_core::PhysicsSolid {
                mass: 1.,
                damping: 0.,
                rotdamping: 0.,
                surfaceprop: "synthetic_grenade".into(),
                ..Default::default()
            },
        );
        world.surface_materials.insert(
            "synthetic_grenade".into(),
            modkit_core::SurfaceMaterial {
                friction: 0.9,
                elasticity: 0.01,
                hardness_factor: 1.,
                impact_hard: Some("Synthetic.GrenadeHard".into()),
                ..Default::default()
            },
        );
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        physics.colliders.insert(
            ColliderBuilder::cuboid(2000. / 39.37, 2000. / 39.37, 8. / 39.37)
                .translation(vector![0., 0., -8. / 39.37])
                .friction(0.8)
                .restitution(0.2)
                .friction_combine_rule(CoefficientCombineRule::Multiply)
                .restitution_combine_rule(CoefficientCombineRule::Multiply),
        );
        physics.tick(0.015);
        let impacts = crate::impacts::Impacts::default();
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                angles: None,
                kind: ProjectileKind::FragGrenade,
                position: Vec3::new(0., 0., 60.),
                velocity: Vec3::new(350., 0., 0.),
                angular_velocity: Vec3::new(600., 0., 0.),
                at: 0.,
                damage: 125.,
                radius: 250.,
                mass: 0.,
                lifetime: 3.,
            },
            &mut scene,
        );
        let bodies = physics.bodies.len();
        let mut sounds = vec![];
        let mut rest = None;
        for tick in 0..230 {
            scene.time = f64::from(tick) * 0.015;
            projectiles.tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(9999.),
                false,
                0.015,
            );
            physics.tick(0.015);
            impacts.physics_collision_sounds(&physics, &world, &mut scene);
            for sound in scene.sounds.drain(..) {
                if sound.name == "Synthetic.GrenadeHard" {
                    sounds.push((scene.time, sound.volume.unwrap()));
                }
            }
            if tick == 150 {
                let frag = &projectiles.active[0];
                assert!(frag.body.is_some());
                rest = Some(frag.position);
                // On the floor (box half-height 4), moved forward, slowed by contact.
                assert!((frag.position.z - 4.).abs() < 1.5, "{}", frag.position);
                assert!(frag.position.x > 100., "{}", frag.position);
                assert!(frag.velocity.length() < 350., "{}", frag.velocity);
            }
        }
        // The landing sounds at (speed / 320)^2 of the contact point's normal speed:
        // the ~252 u/s fall minus the 600 deg/s spin of the touching corner.
        assert!(!sounds.is_empty());
        let (at, volume) = sounds[0];
        assert!((0.3..0.45).contains(&at), "{sounds:?}");
        assert!((0.3..0.7).contains(&volume), "{sounds:?}");
        assert!(
            sounds.iter().all(|(t, _)| *t < 1.),
            "resting contact is silent: {sounds:?}"
        );
        assert!(rest.is_some());
        // Detonation (3.045 s) removes the projectile and its body.
        assert!(projectiles.active.is_empty());
        assert_eq!(physics.bodies.len(), bodies);
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
    #[test]
    fn frag_prism_lands_on_its_side_and_rolls_about_its_long_axis() {
        // A 12-sided prism like w_grenade.phy (synthetic copy; radius 1.808, 8.9 long),
        // thrown upright like Fraggrenade_Create's vec3_angle lob onto a floor at z 64.
        let mut world = World::default();
        let mut vertices = Vec::new();
        for k in 0..12 {
            let a = k as f32 * std::f32::consts::TAU / 12.;
            for z in [-4.353, 4.509] {
                vertices.push(Vec3::new(1.808 * a.cos(), 1.808 * a.sin(), z));
            }
        }
        world.model_collision.insert(
            frag_key(),
            vec![modkit_core::ConvexPiece {
                vertices,
                indices: vec![],
            }],
        );
        world.model_physics.insert(
            frag_key(),
            modkit_core::PhysicsSolid {
                mass: 1.,
                surfaceprop: "synthetic_grenade".into(),
                compact_inertia: Some((Vec3::ZERO, Vec3::new(0.00428, 0.00428, 0.000711))),
                ..Default::default()
            },
        );
        world.surface_materials.insert(
            "synthetic_grenade".into(),
            modkit_core::SurfaceMaterial {
                friction: 0.9,
                elasticity: 0.01,
                ..Default::default()
            },
        );
        let mut scene = Scene::new(&world);
        let mut physics = Physics::new(&world);
        physics.colliders.insert(
            ColliderBuilder::cuboid(2000. / 39.37, 2000. / 39.37, 8. / 39.37)
                .translation(vector![0., 0., 56. / 39.37])
                .friction(0.8)
                .restitution(0.2)
                .friction_combine_rule(CoefficientCombineRule::Multiply)
                .restitution_combine_rule(CoefficientCombineRule::Multiply),
        );
        physics.tick(0.015);
        let mut projectiles = Projectiles::default();
        projectiles.spawn(
            ProjectileSpawn {
                angles: Some(Vec3::ZERO),
                kind: ProjectileKind::FragGrenade,
                position: Vec3::new(0., 0., 120.),
                velocity: Vec3::new(317., 0., -98.),
                angular_velocity: Vec3::new(200., 0., 0.),
                at: 0.,
                damage: 125.,
                radius: 250.,
                mass: 0.,
                lifetime: 3.,
            },
            &mut scene,
        );
        let mut samples = vec![];
        for tick in 0..100 {
            scene.time = f64::from(tick) * 0.015;
            projectiles.tick(
                &world,
                &mut scene,
                &mut physics,
                Vec3::splat(9999.),
                false,
                0.015,
            );
            physics.tick(0.015);
            if tick == 60 || tick == 90 {
                samples.push(projectiles.active[0].clone());
            }
        }
        for frag in &samples {
            let axis = crate::physics::angles(frag.angles) * Vec3::Z;
            // Lying on its side: the origin one radius above the floor, long axis flat.
            assert!((frag.position.z - 65.8).abs() < 0.5, "{}", frag.position);
            assert!(axis.z.abs() < 0.1, "{axis}");
        }
        // Still rolling forward across its axis (native lob: ~150 u/s held after landing).
        let (a, b) = (&samples[0], &samples[1]);
        assert!(
            b.position.x - a.position.x > 15.,
            "{} {}",
            a.position,
            b.position
        );
        assert!(a.velocity.length() > 40., "{}", a.velocity);
    }
}
