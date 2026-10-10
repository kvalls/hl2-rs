//! Shared projectile sprite geometry and owned-material explosion emitters.
//! Native ambient lighting, pixel visibility and beam tessellation remain partial.
use crate::{
    explosion_particles::Particles,
    physics::Physics,
    projectiles::{EffectKind, ProjectileKind, Projectiles},
};
use anyhow::{Context, Result};
use glam::Vec3;
use source_assets::{vpk::Vfs, vtf};
use std::collections::{BTreeMap, HashMap};
pub const SPRITES: &[&str] = &[
    "effects/ar2_altfire1",
    "effects/ar2_altfire1b",
    "effects/combinemuzzle1",
    "effects/combinemuzzle1_nocull",
    "effects/combinemuzzle2_nocull",
    "effects/fire_cloud1",
    "effects/fire_cloud2",
    "sprites/redglow1",
    "sprites/bluelaser1",
    // weapon_crossbow (bolt glow, charger, load blast).
    "sprites/light_glow02_noz",
    "sprites/blueflare1",
    // weapon_rpg (RocketTrail puffs; the dot uses redglow1; RPG_BEAM_SPRITE).
    "particle/particle_smokegrenade",
    "effects/laser1_noz",
    // FX_ElectricSpark (crossbow bolt sparks).
    "effects/spark",
    "effects/yellowflare_noz",
    // weapon_physcannon PHYSCANNON_* sprites and beam.
    "sprites/orangelight1",
    "sprites/glow04_noz",
    "sprites/orangeflare1",
    "sprites/orangecore1",
    "sprites/orangecore2",
];
pub struct Sprite {
    pub image: std::sync::Arc<vtf::Image>,
    pub kind: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Quad {
    pub material: String,
    pub positions: [Vec3; 4],
    pub color: [u8; 4],
    pub uv: [[f32; 2]; 4],
}
pub struct ProjectileVisuals {
    pub sprites: HashMap<String, Sprite>,
    pub quads: Vec<Quad>,
    previous: HashMap<u64, glam::Vec3>,
    rng: u64,
    pub errors: BTreeMap<String, String>,
    pub ball_frames: u64,
    pub effect_frames: u64,
    pub particles: Particles,
    pub missing_particle_draws: u64,
    /// w_grenade.mdl "fuse" attachment offset (bone 0 treated as the model origin).
    fuse_offset: Vec3,
    /// Frag trail points (time, position) per grenade, 0.5 s long.
    trails: HashMap<u64, std::collections::VecDeque<(f64, Vec3)>>,
    /// RocketTrail emitter position per missile and its live puffs.
    rocket_emitters: HashMap<u64, Vec3>,
    puffs: Vec<Puff>,
    last_frame: f64,
    /// FX_ElectricSpark trail particles and glows, and the Sparks effects already spawned.
    sparks: Vec<Spark>,
    spark_glows: Vec<(f64, Vec3, f32, f32)>,
    sparks_seen: std::collections::HashSet<u64>,
    spark_frame: f64,
    /// Viewmodel-space sprite/beam quads for the viewmodel camera (viewmodel_effects.rs).
    pub viewmodel_quads: Vec<Quad>,
}
/// One CTrailParticles TrailParticle of FX_ElectricSpark.
#[derive(Clone, Debug)]
struct Spark {
    position: Vec3,
    velocity: Vec3,
    life: f32,
    die: f32,
    width: f32,
    length: f32,
    gravity: f32,
    dampen: f32,
}
/// One C_RocketTrail SimpleParticle (size/alpha lerp over its life, drifting).
#[derive(Clone, Debug)]
struct Puff {
    born: f64,
    life: f32,
    position: Vec3,
    velocity: Vec3,
    sizes: (f32, f32),
    alpha: f32,
    color: u8,
    roll: f32,
    roll_delta: f32,
    material: usize,
}
pub fn load_sprite(vfs: &Vfs, material: &str) -> Result<Sprite> {
    let base = vfs
        .base_texture(material)?
        .context("projectile sprite base texture missing")?;
    let data = vfs
        .read(&format!("materials/{}.vtf", base.trim_end_matches(".vtf")))?
        .context("projectile sprite texture missing")?;
    let image = vtf::decode(&data, 512)?;
    Ok(Sprite {
        image: std::sync::Arc::new(image),
        // Entity render modes, not the VMTs, make these additive: the frag's
        // redglow1 is kRenderGlow and its bluelaser1 trail kRenderTransAdd.
        kind: if matches!(
            material,
            "sprites/lgtning"
                | "sprites/redglow1"
                | "sprites/bluelaser1"
                | "sprites/light_glow02_noz"
                | "sprites/blueflare1"
                // CBeam/CSprite kRenderTransAdd on the RPG and gravity gun.
                | "effects/laser1_noz"
                | "sprites/orangelight1"
                | "sprites/glow04_noz"
                | "sprites/orangeflare1"
                | "sprites/orangecore1"
                | "sprites/orangecore2"
        ) {
            3
        } else {
            material_kind(vfs, material)
        },
    })
}
/// Preserves the retained host's exact legacy flag precedence.
pub fn material_kind(vfs: &Vfs, name: &str) -> usize {
    if vfs
        .material_value(name, "$additive")
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
    {
        3
    } else if vfs
        .material_value(name, "$translucent")
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
    {
        2
    } else if vfs
        .material_value(name, "$alphatest")
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
    {
        1
    } else {
        0
    }
}

pub(crate) fn basis(direction: Vec3) -> (Vec3, Vec3) {
    let direction = direction.normalize_or_zero();
    let mut right = direction.cross(Vec3::Z).normalize_or_zero();
    if right.length_squared() == 0. {
        right = Vec3::Y;
    }
    (right, right.cross(direction).normalize_or_zero())
}
fn blur(displacement: f32, index: usize) -> (f32, f32) {
    let speed = displacement.clamp(0., 32.);
    (
        (speed * 0.5).min(4.) * (index + 1) as f32,
        ((speed - 4.) / 28.).clamp(0., 1.) * (1. - index as f32 / 12.),
    )
}
impl ProjectileVisuals {
    pub fn empty() -> Self {
        Self {
            sprites: HashMap::new(),
            quads: Vec::new(),
            previous: HashMap::new(),
            rng: 0x953732fb,
            errors: BTreeMap::new(),
            ball_frames: 0,
            effect_frames: 0,
            particles: Particles::default(),
            missing_particle_draws: 0,
            fuse_offset: Vec3::ZERO,
            trails: HashMap::new(),
            rocket_emitters: HashMap::new(),
            puffs: Vec::new(),
            last_frame: 0.,
            sparks: Vec::new(),
            spark_glows: Vec::new(),
            sparks_seen: std::collections::HashSet::new(),
            spark_frame: 0.,
            viewmodel_quads: Vec::new(),
        }
    }
    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.random()
    }
    /// FX_ElectricSpark(pos, 1, 1, NULL) (client fx_sparks.cpp): RandomFloat(2, 4) big
    /// sparks (1-2 s, 64-300 u/s up-biased, gravity 800, dampen 0.3, width 2-5, length
    /// 0.02-0.05), RandomInt(16, 32) little ones (0.1-0.2 s, 128-256 u/s, gravity 400,
    /// width 2-4, length 0.02-0.03) and two 0.2 s yellowflare_noz caps.
    fn spawn_sparks(&mut self, position: Vec3, time: f64) {
        let big = self.range(2., 4.) as usize;
        for _ in 0..big {
            let direction = Vec3::new(
                self.range(-1., 1.),
                self.range(-1., 1.),
                self.range(0.5, 1.),
            );
            let speed = self.range(64., 300.);
            let spark = Spark {
                position,
                velocity: direction * speed,
                life: 0.,
                die: self.range(1., 2.),
                width: self.range(2., 5.),
                length: self.range(0.02, 0.05),
                gravity: 800.,
                dampen: 0.3,
            };
            self.sparks.push(spark);
        }
        let little = 16 + (self.random() * 17.) as usize;
        for _ in 0..little.min(32) {
            let direction = Vec3::new(
                self.range(-1., 1.),
                self.range(-1., 1.),
                self.range(-1., 1.),
            );
            let speed = self.range(128., 256.);
            let spark = Spark {
                position,
                velocity: direction * speed,
                life: 0.,
                die: self.range(0.1, 0.2),
                width: self.range(2., 4.),
                length: self.range(0.02, 0.03),
                gravity: 400.,
                // The second emitter's m_flVelocityDampen is never Setup (0).
                dampen: 0.,
            };
            self.sparks.push(spark);
        }
        let inner = (4 + (self.random() * 5.) as u32).min(8) as f32;
        let outer = (32 + (self.random() * 33.) as u32).min(64) as f32;
        let shade = (32 + (self.random() * 33.) as u32).min(64) as f32 / 255.;
        self.spark_glows.push((time, position, inner, 1.));
        self.spark_glows.push((time, position, outer, shade));
    }
    /// Simulate (when not paused) and draw sparks as camera-facing tracers.
    fn draw_sparks(&mut self, eye: Vec3, axes: (Vec3, Vec3), time: f64, dt: f32) {
        for spark in &mut self.sparks {
            spark.velocity.z -= spark.gravity * dt;
            spark.position += spark.velocity * dt;
            spark.velocity *= (1. - dt * spark.dampen).max(0.);
            spark.life += dt;
        }
        self.sparks.retain(|s| s.life < s.die);
        let sparks = std::mem::take(&mut self.sparks);
        // The host's material table only holds loaded sprites.
        let drawable = self.sprites.contains_key("effects/spark");
        for spark in sparks.iter().filter(|_| drawable) {
            let life = 1. - spark.life / spark.die;
            let scale = (spark.length * life).max(0.01);
            let delta = spark.velocity * scale;
            let length = delta.length();
            let width = length.min(spark.width);
            let side = delta.cross(spark.position - eye).normalize_or_zero() * width * 0.5;
            if side == Vec3::ZERO {
                continue;
            }
            let (a, b) = (spark.position, spark.position + delta);
            self.quads.push(Quad {
                material: "effects/spark".into(),
                positions: [a - side, b - side, b + side, a + side],
                color: [255; 4],
                uv: [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
            });
        }
        self.sparks = sparks;
        self.spark_glows.retain(|(at, ..)| time - at < 0.2);
        for (at, position, size, shade) in self.spark_glows.clone() {
            let t = ((time - at) / 0.2) as f32;
            self.quad(
                "effects/yellowflare_noz",
                position,
                axes,
                size * (1. - t),
                0.,
                shade * if shade < 1. { 1. - t } else { 1. },
            );
        }
    }
    pub fn new(vfs: &Vfs) -> Self {
        let mut result = Self::empty();
        for name in SPRITES
            .iter()
            .chain(crate::explosion_particles::MATERIALS)
            .collect::<std::collections::BTreeSet<_>>()
        {
            match load_sprite(vfs, name) {
                Ok(sprite) => {
                    result.sprites.insert((*name).into(), sprite);
                }
                Err(error) => {
                    result.errors.insert((*name).into(), format!("{error:#}"));
                }
            }
        }
        match vfs
            .read(crate::projectiles::FRAG_MODEL)
            .ok()
            .flatten()
            .map(|data| source_assets::eyes::read_attachments(&data))
        {
            Some(Ok(attachments)) => {
                if let Some((_, fuse)) = attachments
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("fuse"))
                {
                    result.fuse_offset = fuse.local.w_axis.truncate();
                }
            }
            other => {
                result.errors.insert(
                    crate::projectiles::FRAG_MODEL.into(),
                    format!("fuse attachment: {:?}", other.map(|r| r.err())),
                );
            }
        }
        result
    }
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u32 << 24) as f32
    }
    #[allow(clippy::too_many_arguments)]
    fn quad(
        &mut self,
        material: &str,
        center: Vec3,
        axes: (Vec3, Vec3),
        size: f32,
        roll: f32,
        intensity: f32,
    ) {
        if !self.sprites.contains_key(material) {
            return;
        }
        if size <= 0. || intensity <= 0. {
            return;
        }
        let (sin, cos) = roll.sin_cos();
        let right = (axes.0 * cos + axes.1 * sin) * size;
        let up = (axes.0 * -sin + axes.1 * cos) * size;
        let shade = (intensity.clamp(0., 1.) * 255.).round() as u8;
        let positions = [
            center - right - up,
            center - right + up,
            center + right + up,
            center + right - up,
        ];
        self.quads.push(Quad {
            material: material.into(),
            positions,
            color: [shade, shade, shade, 255],
            uv: crate::explosion_particles::UV,
        });
    }

    /// C_RocketTrail::Update for ignited missiles: puffs every StartSize/2 units moved
    /// (at most 50 per frame), life 0.5 + [0.45, 0.55], size 8 -> 32 (x 0.75..1.25 and
    /// 1..1.25), alpha 0.2 (x 0.75..1.25) -> 0, gray 0.65 (x 0.75..1.25), drifting 2..16
    /// u/s. The muzzle-flash flare (m_flFlareScale) is not drawn.
    fn rocket_trails(
        &mut self,
        projectiles: &Projectiles,
        axes: (Vec3, Vec3),
        time: f64,
        paused: bool,
    ) {
        let trail = &crate::weapon_rpg::ROCKET_TRAIL;
        self.rocket_emitters
            .retain(|id, _| projectiles.active.iter().any(|p| p.id == *id));
        let dt = if paused {
            0.
        } else {
            (time - self.last_frame).clamp(0., 0.1) as f32
        };
        self.last_frame = time;
        if !paused {
            for missile in projectiles
                .active
                .iter()
                .filter(|p| p.kind == ProjectileKind::RpgMissile && p.ignited)
            {
                let last = *self
                    .rocket_emitters
                    .entry(missile.id)
                    .or_insert(missile.position);
                let delta = missile.position - last;
                let length = delta.length();
                let count = ((length / (trail.start_size / 2.)) as usize).min(50);
                for i in 1..=count {
                    let position = last + delta * (i as f32 / count as f32);
                    let mut r = || self.random();
                    let life =
                        trail.particle_lifetime + trail.particle_lifetime * (0.9 + 0.2 * r());
                    let direction = Vec3::new(r() * 2. - 1., r() * 2. - 1., r() * 2. - 1.);
                    let speed = trail.min_speed + (trail.max_speed - trail.min_speed) * r();
                    let gray = (trail.start_color[0] * (0.75 + 0.5 * r())).clamp(0., 1.);
                    let puff = Puff {
                        born: time,
                        life,
                        position,
                        velocity: direction * speed,
                        sizes: (
                            trail.start_size * (0.75 + 0.5 * r()),
                            trail.end_size * (1. + 0.25 * r()),
                        ),
                        alpha: (trail.opacity * (0.75 + 0.5 * r())).clamp(0., 1.),
                        color: (gray * 255.) as u8,
                        roll: r() * std::f32::consts::TAU,
                        roll_delta: (r() * 2. - 1.) * 8.,
                        material: usize::from(r() >= 0.5),
                    };
                    if self.puffs.len() < 2048 {
                        self.puffs.push(puff);
                    }
                }
                self.rocket_emitters.insert(missile.id, missile.position);
            }
        }
        self.puffs.retain(|p| time - p.born < f64::from(p.life));
        for i in 0..self.puffs.len() {
            let mut puff = self.puffs[i].clone();
            puff.position += puff.velocity * dt;
            puff.roll += puff.roll_delta * dt;
            self.puffs[i] = puff.clone();
            let t = ((time - puff.born) as f32 / puff.life).clamp(0., 1.);
            let size = puff.sizes.0 + (puff.sizes.1 - puff.sizes.0) * t;
            let alpha = puff.alpha * (1. - t);
            let material = crate::weapon_rpg::TRAIL_MATERIALS[puff.material];
            let before = self.quads.len();
            self.quad(material, puff.position, axes, size * 0.5, puff.roll, 1.);
            if let Some(quad) = self.quads.get_mut(before) {
                quad.color = [puff.color, puff.color, puff.color, (alpha * 255.) as u8];
            }
        }
    }

    /// C_SpriteTrail::DrawModel through CBeamSegDraw: one camera-facing strip whose
    /// points share their edge vertices (normal from the neighbouring points), alpha
    /// and width follow each point's remaining life (width end..start), u spans the
    /// width and v is the point's texture coordinate. grenade_frag never sets a
    /// texture resolution (zeroed entity field), so v stays 0 along the whole trail.
    fn sprite_trail(
        &mut self,
        material: &str,
        color: [u8; 3],
        points: &[(f64, Vec3)],
        eye: Vec3,
        time: f64,
        (life, start_width, end_width): (f64, f32, f32),
    ) {
        let n = points.len();
        if n < 2 {
            return;
        }
        let edge: Vec<(Vec3, f32)> = (0..n)
            .map(|i| {
                let (at, position) = points[i];
                let along = points[(i + 1).min(n - 1)].1 - points[i.saturating_sub(1)].1;
                let side = along.cross(position - eye).normalize_or_zero();
                let perc = (1. - (time - at) / life).clamp(0., 1.) as f32;
                let width = end_width + (start_width - end_width) * perc;
                (side * width * 0.5, perc)
            })
            .collect();
        for i in 0..n - 1 {
            let ((a, ea), (b, eb)) = ((points[i].1, edge[i]), (points[i + 1].1, edge[i + 1]));
            if ea.0 == Vec3::ZERO && eb.0 == Vec3::ZERO {
                continue;
            }
            let alpha = ((ea.1 + eb.1) * 0.5 * 255.).round() as u8;
            self.quads.push(Quad {
                material: material.into(),
                positions: [a - ea.0, b - eb.0, b + eb.0, a + ea.0],
                color: [color[0], color[1], color[2], alpha],
                uv: [[0., 0.], [0., 0.], [1., 0.], [1., 0.]],
            });
        }
    }

    pub fn frame(
        &mut self,
        projectiles: &Projectiles,
        eye: Vec3,
        direction: Vec3,
        time: f64,
        paused: bool,
        physics: &Physics,
    ) {
        self.quads.clear();
        let axes = basis(direction);
        self.previous
            .retain(|id, _| projectiles.active.iter().any(|p| p.id == *id));
        for ball in projectiles
            .active
            .iter()
            .filter(|p| p.kind == ProjectileKind::CombineBall)
        {
            let position = Vec3::new(ball.position.x, ball.position.y, ball.position.z);
            let (brightness, size) = if paused {
                (0.2, 1.5)
            } else {
                (0.2 + self.random() * 0.1, 1.5 + self.random())
            };
            self.quad(
                "effects/combinemuzzle1",
                position,
                axes,
                ball.radius * size,
                0.,
                brightness,
            );
            let previous = self
                .previous
                .insert(ball.id, ball.position)
                .unwrap_or(ball.previous_position);
            let displacement = ball.position - previous;
            let movement = displacement.normalize_or_zero();
            for i in 0..8 {
                let (distance, brightness) = blur(displacement.length(), i);
                let center = ball.position - movement * distance;
                self.quad(
                    "effects/ar2_altfire1b",
                    Vec3::new(center.x, center.y, center.z),
                    axes,
                    ball.radius,
                    0.,
                    brightness,
                );
            }
            // DrawHaloOriented's roll is radians, and the free ball uses a
            // pulsating billboard rather than the held-ball model.
            self.quad(
                "effects/ar2_altfire1",
                position,
                axes,
                ball.radius + (time as f32 * 25.).sin(),
                ball.spawned_at as f32,
                1.,
            );
            self.ball_frames += 1;
        }
        // CGrenadeFrag::CreateEffects: redglow1 (kRenderGlow, alpha 200, scale 0.2) and a
        // bluelaser1 trail (additive red, width 8 to 1, 0.5 s) at the fuse attachment.
        self.trails
            .retain(|id, _| projectiles.active.iter().any(|p| p.id == *id));
        for frag in projectiles
            .active
            .iter()
            .filter(|p| p.kind == ProjectileKind::FragGrenade)
        {
            let fuse = frag.position + crate::physics::angles(frag.angles) * self.fuse_offset;
            let glow = self
                .sprites
                .get("sprites/redglow1")
                .map_or(0., |s| s.image.width as f32 * 0.5 * 0.2);
            self.quad("sprites/redglow1", fuse, axes, glow, 0., 200. / 255.);
            let trail = self.trails.entry(frag.id).or_default();
            if !paused && trail.back().is_none_or(|(at, _)| *at < time) {
                trail.push_back((time, fuse));
            }
            while trail.front().is_some_and(|(at, _)| time - at > 0.5) {
                trail.pop_front();
            }
            if !self.sprites.contains_key("sprites/bluelaser1") {
                continue;
            }
            let points: Vec<(f64, Vec3)> = trail.iter().copied().collect();
            self.sprite_trail(
                "sprites/bluelaser1",
                [255, 0, 0],
                &points,
                eye,
                time,
                (0.5, 8., 1.),
            );
        }
        self.rocket_trails(projectiles, axes, time, paused);
        // CLaserDot: redglow1, kRenderGlow 255, LaserThink scale.
        if let Some(dot) = projectiles.laser_dot.filter(|d| d.on) {
            let size = self
                .sprites
                .get(crate::weapon_rpg::DOT_SPRITE)
                .map_or(0., |s| s.image.width as f32 * 0.5 * dot.scale);
            self.quad(
                crate::weapon_rpg::DOT_SPRITE,
                dot.position,
                axes,
                size,
                0.,
                1.,
            );
        }
        for effect in &projectiles.effects {
            let age = (time - effect.at).max(0.) as f32;
            let position = Vec3::new(effect.position.x, effect.position.y, effect.position.z);
            match effect.kind {
                EffectKind::BallImpact => {
                    let normal = Vec3::new(effect.normal.x, effect.normal.y, effect.normal.z)
                        .normalize_or_zero();
                    let axes = basis(normal);
                    self.quad(
                        "effects/combinemuzzle1_nocull",
                        position + normal * 0.5,
                        axes,
                        effect.radius * 10.,
                        effect.at as f32,
                        (1. - age / 0.25).max(0.),
                    );
                    self.quad(
                        "effects/combinemuzzle2_nocull",
                        position + normal * 0.5,
                        axes,
                        effect.radius * (2. + age / 0.5 * 2.),
                        effect.at as f32,
                        (1. - age / 0.5).max(0.),
                    );
                }
                EffectKind::BoltGlow => {
                    // CCrossbowBolt glow: kRenderGlow, alpha 128, scale 0.2, FadeAndDie(3).
                    let size = self
                        .sprites
                        .get(crate::weapon_crossbow::BOLT_GLOW_SPRITE)
                        .map_or(0., |s| s.image.width as f32 * 0.5 * 0.2);
                    let fade = (1. - age / crate::weapon_crossbow::STUCK_GLOW_TIME as f32).max(0.);
                    self.quad(
                        crate::weapon_crossbow::BOLT_GLOW_SPRITE,
                        position,
                        axes,
                        size,
                        0.,
                        128. / 255. * fade,
                    );
                }
                EffectKind::Sparks => {
                    if self.sparks_seen.insert(effect.id) {
                        self.spawn_sparks(position, time);
                    }
                }
                EffectKind::GrenadeExplosion | EffectKind::BallExplosion => (),
            }
            self.effect_frames += 1;
        }
        self.sparks_seen
            .retain(|id| projectiles.effects.iter().any(|e| e.id == *id));
        let dt = if paused {
            0.
        } else {
            ((time - self.spark_frame).clamp(0., 0.1)) as f32
        };
        self.spark_frame = time;
        self.draw_sparks(eye, axes, time, dt);
        self.particles.sync(&projectiles.effects, time, physics);
        for quad in self.particles.draw(eye, direction, time, physics) {
            if self.sprites.contains_key(&quad.material) {
                self.quads.push(quad);
            } else {
                self.missing_particle_draws += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn electric_sparks_spawn_sdk_counts_fall_and_expire() {
        let mut visuals = ProjectileVisuals::empty();
        visuals.spawn_sparks(Vec3::ZERO, 0.);
        let count = visuals.sparks.len();
        assert!((18..=36).contains(&count), "{count}");
        assert_eq!(visuals.spark_glows.len(), 2);
        let before: Vec<f32> = visuals.sparks.iter().map(|s| s.velocity.z).collect();
        visuals.draw_sparks(Vec3::X * -100., basis(Vec3::X), 0.015, 0.015);
        // No spark material loaded: nothing is drawn, but the sparks still simulate.
        assert!(visuals.quads.is_empty());
        for (spark, z) in visuals.sparks.iter().zip(before) {
            assert!(spark.velocity.z < z);
        }
        for i in 2..200 {
            visuals.draw_sparks(Vec3::X * -100., basis(Vec3::X), i as f64 * 0.015, 0.015);
        }
        assert!(visuals.sparks.is_empty() && visuals.spark_glows.is_empty());
    }
    #[test]
    fn blur_uses_rendered_frame_displacement_and_retains_eight_samples() {
        assert_eq!(blur(0., 0), (0., 0.));
        assert_eq!(blur(4., 7), (16., 0.));
        assert_eq!(blur(32., 0), (4., 1.));
        let (distance, intensity) = blur(100., 7);
        assert_eq!(distance, 32.);
        assert!((intensity - 5. / 12.).abs() < 0.00001);
    }
    #[test]
    fn billboard_axes_stay_orthogonal_at_horizontal_and_vertical_views() {
        for direction in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1., 2., 3.)] {
            let (right, up) = basis(direction);
            assert!((right.length() - 1.).abs() < 0.00001);
            assert!((up.length() - 1.).abs() < 0.00001);
            assert!(right.dot(up).abs() < 0.00001);
            assert!(direction.normalize().dot(right).abs() < 0.00001);
        }
    }
    #[test]
    fn sprite_trail_is_one_strip_fading_with_point_life() {
        let mut visuals = ProjectileVisuals::empty();
        let points: Vec<(f64, Vec3)> = (0..5)
            .map(|i| {
                (
                    f64::from(i) * 0.1,
                    Vec3::new(f32::from(i as u8) * 20., 0., f32::from((i * i) as u8)),
                )
            })
            .collect();
        visuals.sprite_trail(
            "sprites/bluelaser1",
            [255, 0, 0],
            &points,
            Vec3::new(40., -200., 0.),
            0.4,
            (0.5, 8., 1.),
        );
        assert_eq!(visuals.quads.len(), 4);
        for pair in visuals.quads.windows(2) {
            // Adjacent segments share the edge vertices (no kinks or gaps at joints).
            assert_eq!(pair[0].positions[1], pair[1].positions[0]);
            assert_eq!(pair[0].positions[2], pair[1].positions[3]);
            assert!(pair[0].color[3] < pair[1].color[3]);
        }
        // The newest point is full width (8) and the 0.4 s old one is near the end width.
        let head = &visuals.quads[3];
        assert!((head.positions[2].distance(head.positions[1]) - 8.).abs() < 1e-3);
        let tail = &visuals.quads[0];
        let tail_width = tail.positions[3].distance(tail.positions[0]);
        assert!((tail_width - (1. + 7. * 0.2)).abs() < 1e-3, "{tail_width}");
        assert!(visuals
            .quads
            .iter()
            .all(|q| q.uv.iter().all(|uv| uv[1] == 0.)));
    }
}
