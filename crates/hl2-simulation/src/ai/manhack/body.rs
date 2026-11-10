//! The manhack's VPhysics body against what it flies into (DESIGN 14r session 52; Source SDK
//! ai_basenpc_physicsflyer.cpp Simulate, hl2/npc_manhack.cpp ClampMotorForces, CheckCollisions,
//! Bump, HitPhysicsObject). The AI keeps the velocity it wants (m_vCurrentVelocity, `hover`); the
//! body has its own, which a motor pulls toward the wanted one every tick and which what the body
//! meets stops. Knocks (bullets, a club, the gravity gun) move the body; a bump turns what it wants.
//! The arithmetic only: the stage carries the body and shows the bump.

use super::Manhack;
use crate::muzzle_effects::FlashLight;
use glam::Vec3;

/// ClampMotorForces: the motor's pull on each axis, u/s² at full blade speed (100).
const MOTOR: f32 = 3000.;
/// ... and upward never less than this.
const MOTOR_UP: f32 = 1200.;
/// What a contact gives back of the speed into the surface: VPhysics' elasticity of the
/// manhack's "metal" (0.25, owned scripts/surfaceproperties.txt) times a concrete wall's 0.2.
/// The product is our rigid bodies' rule (physics.rs); retail IVP's combination was not traced.
const RESTITUTION: f32 = 0.05;
/// Bump inside something: a random push of this speed, and no bump for this long.
const STUCK_SPEED: f32 = 50.;
const STUCK_QUIET: f64 = 0.5;
/// Bump: within this height of the floor a surface never pushes it down.
pub const FLOOR_NEAR: f32 = 36.;
/// HitPhysicsObject: "a 30 kg object going 100 in/s", kg in/s.
const BLADE_PUSH: f32 = 30. * 100.;
/// Bump's te->DynamicLight: orange, radius 50, 0.3 s, losing 150 a second.
pub const BUMP_LIGHT: FlashLight = FlashLight {
    origin: Vec3::ZERO,
    color: [255, 180, 100],
    radius: 50.,
    decay: 150.,
    lifetime: 0.3,
    exponent: 0,
};
/// Bump's UTIL_DecalTrace "ManhackCut" (owned scripts/decals_subrect.txt, equal weights).
pub const CUT_DECALS: [&str; 3] = [
    "decals/manhackcut_subrect",
    "decals/manhackcut2_subrect",
    "decals/manhackcut3_subrect",
];
/// Bump traces this far back along the surface's normal for the decal.
pub const DECAL_REACH: f32 = 24.;
/// The decal goes only on surfaces this far from level, up or down.
const DECAL_LEVEL: f32 = 0.5;

/// What a bump met, for the stage to show: where the hull stopped, the surface's normal, the prop
/// or NPC if not the world, whether the trace started inside something, and the spark fan's
/// direction when the surface stood across the blades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bumped {
    pub at: Vec3,
    pub normal: Vec3,
    pub entity: Option<usize>,
    pub start_solid: bool,
    pub sparks: Option<Vec3>,
}

/// What the manhacks' bumps leave for the host this tick: Bump's lights, its cut decals (the
/// surface found and the material), and pushes on props (the prop, an impulse in kg in/s, where).
#[derive(Clone, Debug, Default)]
pub struct BumpMarks {
    pub lights: Vec<FlashLight>,
    pub decals: Vec<(crate::physics::RayHit, &'static str)>,
    pub pushes: Vec<(usize, Vec3, Vec3)>,
}

/// Simulate's motor: the pull that brings the body to `wish` within the tick, each axis held to
/// ClampMotorForces' limit, which scales with the blades' speed. A stalled engine pulls only up
/// and down.
pub fn motor(body: Vec3, wish: Vec3, (blades, stalled): (f32, bool), dt: f32) -> Vec3 {
    if dt <= 0. {
        return body;
    }
    let most = MOTOR * blades / 100.;
    let across = if stalled { 0. } else { most };
    let pull = (wish - body) / dt;
    let pull = Vec3::new(
        pull.x.clamp(-across, across),
        pull.y.clamp(-across, across),
        pull.z.clamp(-most, most.max(MOTOR_UP)),
    );
    body + pull * dt
}

/// A contact: the speed into the surface is lost but for RESTITUTION of it; along it, kept.
pub fn rebound(body: Vec3, normal: Vec3) -> Vec3 {
    let into = body.dot(normal);
    if into >= 0. {
        return body;
    }
    body - normal * into * (1. + RESTITUTION)
}

/// Bump's deflection of the velocity it wants: off the surface by half its speed, never down
/// when it is close above the floor.
pub fn deflect(wish: Vec3, normal: Vec3, low: bool) -> Vec3 {
    let mut away = normal;
    if low && away.z < 0. {
        away.z = 0.;
    }
    let away = away.normalize_or_zero();
    let reflection = normal * 2. * normal.dot(away) - away;
    wish + reflection * wish.length() * 0.5
}

/// HitPhysicsObject: a push across the line to the prop, the way the blades spin, at the edge of
/// the manhack's box. The impulse (kg in/s) and where it lands.
pub fn blade_push(origin: Vec3, prop: Vec3, half_width: f32) -> (Vec3, Vec3) {
    let toward = (prop - origin).normalize_or_zero();
    let across = toward.cross(Vec3::Z).normalize_or_zero();
    (across * BLADE_PUSH, origin + toward * half_width)
}

/// The spark fan's direction (ManhackSparks' m_vNormal): halfway between the surface's normal and
/// the way it was flying, unnormalized as the SDK leaves it.
pub fn spark_normal(normal: Vec3, wish: Vec3) -> Vec3 {
    (normal + wish.normalize_or_zero()) * 0.5
}

/// Bump leaves a decal only on a surface that is neither floor nor ceiling.
pub fn takes_decal(normal: Vec3) -> bool {
    normal.z < DECAL_LEVEL && -normal.z < DECAL_LEVEL
}

impl Manhack {
    /// The gravity gun sets its body's velocity. What it wants follows, so a throw is not pulled
    /// back toward the flight it had before (the SDK keeps that; its stalled engine leaves only
    /// the height to the motor).
    pub fn throw_body(&mut self, velocity: Vec3) {
        self.body = velocity;
        self.hover.velocity = velocity;
    }
    /// A hit: VPhysicsTakeDamage's impulse on the body and m_vForceVelocity on what it wants.
    pub fn knock(&mut self, body: Vec3, wish: Vec3) {
        self.body += body;
        self.hover.velocity += wish;
    }
    /// Bump on the velocity it wants: inside something it takes a random push and does not bump
    /// again for half a second; otherwise it is deflected off the surface.
    pub fn deflect_off(&mut self, bumped: &Bumped, low: bool, now: f64) {
        if bumped.start_solid {
            let random = Vec3::new(self.dice(), self.dice(), self.dice()) * 2. - Vec3::ONE;
            self.hover.velocity = random * STUCK_SPEED;
            self.bump_quiet_until = now + STUCK_QUIET;
            return;
        }
        self.hover.velocity = deflect(self.hover.velocity, bumped.normal, low);
    }
    /// m_flBumpSuppressTime.
    pub fn bump_quiet(&self, now: f64) -> bool {
        now < self.bump_quiet_until
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_motor_reaches_what_it_wants_within_a_tick_when_the_blades_allow_it() {
        let body = motor(Vec3::ZERO, Vec3::new(30., -30., 10.), (100., false), 0.015);
        assert!(body.abs_diff_eq(Vec3::new(30., -30., 10.), 1e-3), "{body}");
    }

    #[test]
    fn slow_blades_hold_the_pull_to_their_share_and_upward_to_1200() {
        // Blades at 20 (after a hit): 600 u/s² across, 1200 up.
        let body = motor(Vec3::ZERO, Vec3::new(400., 0., 400.), (20., false), 0.1);
        assert!(body.abs_diff_eq(Vec3::new(60., 0., 120.), 1e-3), "{body}");
        let down = motor(Vec3::ZERO, Vec3::NEG_Z * 400., (20., false), 0.1);
        assert!((down.z + 60.).abs() < 1e-3, "{down}");
    }

    #[test]
    fn a_stalled_engine_lets_it_coast_across_but_still_holds_its_height() {
        let body = motor(Vec3::new(500., 0., 30.), Vec3::ZERO, (100., true), 0.015);
        assert_eq!((body.x, body.y), (500., 0.));
        assert!(body.z.abs() < 1e-3, "{body}");
    }

    #[test]
    fn a_wall_takes_nearly_all_the_speed_into_it_and_none_along_it() {
        let body = rebound(Vec3::new(-400., 300., 0.), Vec3::X);
        assert!(body.abs_diff_eq(Vec3::new(20., 300., 0.), 1e-3), "{body}");
        assert_eq!(rebound(Vec3::X * 50., Vec3::X), Vec3::X * 50.);
    }

    #[test]
    fn a_bump_turns_what_it_wants_off_the_surface_by_half_its_speed() {
        let wish = deflect(Vec3::new(-300., 400., 0.), Vec3::X, false);
        assert!(wish.abs_diff_eq(Vec3::new(-50., 400., 0.), 1e-3), "{wish}");
    }

    #[test]
    fn close_above_the_floor_a_ceiling_does_not_push_it_down() {
        let wish = Vec3::new(-100., 0., 0.);
        assert_eq!(deflect(wish, Vec3::NEG_Z, true), wish);
        let high = deflect(wish, Vec3::NEG_Z, false);
        assert!(high.abs_diff_eq(Vec3::new(-100., 0., -50.), 1e-3), "{high}");
    }

    #[test]
    fn the_blades_push_a_prop_across_the_line_to_it_from_the_box_edge() {
        let (impulse, at) = blade_push(Vec3::ZERO, Vec3::X * 40., 9.);
        assert!(impulse.abs_diff_eq(Vec3::NEG_Y * 3000., 1e-2), "{impulse}");
        assert!(at.abs_diff_eq(Vec3::X * 9., 1e-4), "{at}");
    }

    #[test]
    fn decals_go_on_walls_not_floors_or_ceilings() {
        assert!(takes_decal(Vec3::X));
        assert!(!takes_decal(Vec3::Z));
        assert!(!takes_decal(Vec3::NEG_Z));
        assert!(takes_decal(Vec3::new(0.9, 0., 0.43)));
    }

    #[test]
    fn inside_something_it_takes_a_slow_random_push_and_bumps_no_more_for_half_a_second() {
        let mut m = Manhack::new(false, 3);
        m.hover.velocity = Vec3::X * 400.;
        let stuck = Bumped {
            at: Vec3::ZERO,
            normal: Vec3::ZERO,
            entity: None,
            start_solid: true,
            sparks: None,
        };
        m.deflect_off(&stuck, false, 1.);
        assert!(m.hover.velocity.length() <= STUCK_SPEED * 3f32.sqrt() + 1e-3);
        assert!(m.bump_quiet(1.4) && !m.bump_quiet(1.5));
    }
}
