//! Fixed-tick, Z-up movement. SDK-derived rules; surface modifiers/water/ladders remain separate work.
use crate::{Trace, World};
use glam::Vec3;
use serde::Serialize;
pub const TICK: f32 = 0.015;
pub trait CollisionWorld {
    fn trace_hull(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> Trace;
}
impl CollisionWorld for World {
    fn trace_hull(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> Trace {
        self.trace(start, end, mins, maxs)
    }
}
#[derive(Clone, Copy, Default)]
pub struct Input {
    pub forward: f32,
    pub side: f32,
    pub yaw: f32,
    pub jump: bool,
    pub crouch: bool,
    pub sprint: bool,
    pub slow: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct Player {
    pub feet: Vec3,
    pub velocity: Vec3,
    pub grounded: bool,
    pub crouched: bool,
    pub ticks: u64,
    pub eye_height: f32,
    jump_held: bool,
    surface_friction: f32,
    /// This step started a jump (SDK CheckJumpButton success).
    pub jumped: bool,
    /// This step landed; the fall speed carried from the last airborne tick.
    pub landed: Option<f32>,
    /// Dead players take no input and view from VEC_DEAD_VIEWHEIGHT (14 units).
    pub dead: bool,
}
impl Player {
    pub fn new(eye: Vec3) -> Self {
        Self {
            feet: eye - Vec3::Z * 64.,
            velocity: Vec3::ZERO,
            grounded: false,
            crouched: false,
            ticks: 0,
            eye_height: 64.,
            jump_held: false,
            surface_friction: 1.,
            jumped: false,
            landed: None,
            dead: false,
        }
    }
    pub fn eye(&self) -> Vec3 {
        self.feet + Vec3::Z * self.eye_height
    }
    pub fn step(&mut self, mut input: Input, world: &impl CollisionWorld, dt: f32) {
        self.ticks += 1;
        if self.dead {
            input = Input {
                yaw: input.yaw,
                ..Input::default()
            };
        }
        // PlayerMove stores the fall velocity while airborne; CheckFalling uses it on landing.
        let fall_speed = if self.grounded { 0. } else { -self.velocity.z };
        self.jumped = false;
        self.landed = None;
        let mins = Vec3::new(-16., -16., 0.);
        let standing = Vec3::new(16., 16., 72.);
        let ducked = Vec3::new(16., 16., 36.);
        // Classify the existing hull before changing it. New players and hosts
        // which reposition the player cannot rely on the previous ground flag.
        let old_maxs = if self.crouched { ducked } else { standing };
        let old_ground = world.trace_hull(self.feet, self.feet - Vec3::Z * 2., mins, old_maxs);
        let was_grounded =
            old_ground.fraction < 1. && old_ground.normal.z >= 0.7 && self.velocity.z <= 140.;
        if input.crouch {
            if !self.crouched {
                self.crouched = true;
                if !was_grounded {
                    // FinishDuck tucks the feet while preserving head height.
                    // Apply once, rather than adding height every held tick.
                    self.feet += standing - ducked;
                }
            }
        } else if self.crouched {
            let new_feet = if was_grounded {
                self.feet
            } else {
                self.feet - (standing - ducked)
            };
            // CanUnduck traces the standing hull over the entire origin change;
            // testing only the destination misses obstructions along the sweep.
            let unduck = world.trace_hull(self.feet, new_feet, mins, standing);
            if !unduck.start_solid && unduck.fraction == 1. {
                self.feet = new_feet;
                self.crouched = false;
            }
        }
        let maxs = if self.crouched { ducked } else { standing };
        let target_eye = if self.dead {
            14.
        } else if self.crouched {
            28.
        } else {
            64.
        };
        if self.dead {
            // Event_Killed sets the dead view offset at once.
            self.eye_height = target_eye;
        } else if was_grounded {
            // Ground transition timers and the special duck-jump eye state
            // remain separate work; retain the existing ground interpolation.
            self.eye_height += (target_eye - self.eye_height).clamp(-dt * 180., dt * 180.);
        } else {
            // Ordinary airborne duck/unduck completion changes view immediately.
            self.eye_height = target_eye;
        }
        let ground = world.trace_hull(self.feet, self.feet - Vec3::Z * 2., mins, maxs);
        self.grounded = ground.fraction < 1. && ground.normal.z >= 0.7 && self.velocity.z <= 140.;
        if self.grounded {
            self.velocity.z = 0.;
        }
        // Source starts the first gravity half-step before checking for a jump,
        // including while grounded. Ground walking clears it again below.
        self.velocity.z -= 300. * dt;
        let forward = Vec3::new(input.yaw.cos(), input.yaw.sin(), 0.);
        let right = Vec3::new(input.yaw.sin(), -input.yaw.cos(), 0.);
        // CheckParameters crops the command components before CheckJumpButton
        // uses forward movement for its boost, rather than only capping wishspeed.
        let input_length = input.forward.hypot(input.side).max(1.);
        let forward_move = input.forward / input_length;
        let side_move = input.side / input_length;
        let wish = forward * forward_move + right * side_move;
        let max_speed = if self.crouched {
            // Settled duck keeps normal maxspeed; the command crop is separate.
            // Full suit/sprint/walk transition gating remains outside this model.
            190.
        } else if input.sprint {
            320.
        } else if input.slow {
            150.
        } else {
            190.
        };
        // HandleDuckingSpeedCrop scales command components only while already
        // ducked on ground. It does not scale m_flMaxSpeed, and air ducking has
        // the full command speed for AirAccelerate (including strafe input).
        let command_speed = if self.crouched && self.grounded {
            max_speed * (1. / 3.)
        } else {
            max_speed
        };
        let wishspeed = wish.length().min(1.) * command_speed;
        let wishdir = wish.normalize_or_zero();
        let jumping = input.jump && !self.jump_held && self.grounded;
        self.jump_held = input.jump;
        if jumping {
            self.jumped = true;
            self.grounded = false;
            if self.crouched {
                self.velocity.z = 160.;
            } else {
                self.velocity.z += 160.;
            }
            let boost = if input.sprint || self.crouched {
                0.1
            } else {
                0.5
            };
            let addition = (forward_move.abs() * command_speed * boost)
                .min(max_speed * (1. + boost) - self.velocity.truncate().length());
            // Retail permits a negative addition when already above the boost
            // limit. Its sign only flips for a negative forward command.
            self.velocity += forward
                * if forward_move < 0. {
                    -addition
                } else {
                    addition
                };
            self.velocity.z -= 300. * dt;
        }
        if self.grounded {
            self.velocity.z = 0.;
            let planar = Vec3::new(self.velocity.x, self.velocity.y, 0.);
            let magnitude = planar.length();
            if magnitude > 0. {
                let retained = (magnitude - magnitude.max(100.) * 4. * dt).max(0.) / magnitude;
                self.velocity.x *= retained;
                self.velocity.y *= retained;
            }
        }
        let cap = if self.grounded {
            wishspeed
        } else {
            wishspeed.min(30.)
        };
        let add = cap - self.velocity.dot(wishdir);
        if add > 0. {
            let friction = if self.grounded {
                1.
            } else {
                self.surface_friction
            };
            self.velocity += wishdir * (10. * wishspeed * dt * friction).min(add);
        }
        let before = self.feet;
        let old_velocity = self.velocity;
        let (flat, flat_velocity) =
            slide(world, before, old_velocity, dt, mins, maxs, self.grounded);
        self.feet = flat;
        self.velocity = flat_velocity;
        if self.grounded
            && (flat - before).truncate().length_squared() + 0.01
                < (old_velocity * dt).truncate().length_squared()
        {
            let up = world.trace_hull(before, before + Vec3::Z * 18.03125, mins, maxs);
            if !up.start_solid {
                let (raised, _) = slide(
                    world,
                    before + Vec3::Z * 18.03125 * up.fraction,
                    old_velocity,
                    dt,
                    mins,
                    maxs,
                    true,
                );
                let down = world.trace_hull(raised, raised - Vec3::Z * 18.03125, mins, maxs);
                if !down.start_solid && down.normal.z >= 0.7 {
                    let stepped = raised - Vec3::Z * 18.03125 * down.fraction;
                    if (stepped - before).truncate().length_squared()
                        > (flat - before).truncate().length_squared()
                    {
                        self.feet = stepped;
                        self.velocity = old_velocity;
                        self.velocity.z = flat_velocity.z;
                    }
                }
            }
        }
        let ground = world.trace_hull(self.feet, self.feet - Vec3::Z * 2., mins, maxs);
        self.grounded = self.velocity.z <= 140. && ground.fraction < 1. && ground.normal.z >= 0.7;
        // CategorizePosition resets friction first. Its rapid-ascent return
        // precedes the failed-ground test which assigns quarter friction.
        // Optimized WALK carries this result into the next tick's AirAccelerate.
        self.surface_friction = if self.velocity.z <= 140. && self.velocity.z > 0. && !self.grounded
        {
            0.25
        } else {
            1.
        };
        // FullWalkMove categorizes the swept position before its final half-step.
        // In particular, walking off a ledge must not retain the old ground flag.
        self.velocity.z -= 300. * dt;
        if self.grounded {
            self.velocity.z = 0.;
            self.feet -= Vec3::Z * 2. * ground.fraction;
            if fall_speed > 0. {
                self.landed = Some(fall_speed);
            }
        }
    }
}
fn slide(
    world: &impl CollisionWorld,
    mut position: Vec3,
    mut velocity: Vec3,
    dt: f32,
    mins: Vec3,
    maxs: Vec3,
    grounded: bool,
) -> (Vec3, Vec3) {
    let mut remaining = dt;
    let primal = velocity;
    let mut segment_velocity = velocity;
    let mut all_fraction = 0.;
    let mut planes = Vec::<Vec3>::new();
    for _ in 0..4 {
        if velocity.length_squared() == 0. {
            break;
        }
        let hit = world.trace_hull(position, position + velocity * remaining, mins, maxs);
        all_fraction += hit.fraction;
        // A player which starts overlapping may still escape the solid. Source
        // only stops this sweep immediately when its entire path is solid.
        if hit.all_solid {
            return (position, Vec3::ZERO);
        }
        if hit.fraction > 0. {
            let end = position + velocity * remaining * hit.fraction;
            if hit.fraction == 1. {
                let stuck = world.trace_hull(end, end, mins, maxs);
                if stuck.start_solid || stuck.fraction != 1. {
                    velocity = Vec3::ZERO;
                    break;
                }
            }
            position = end;
            // A completed segment no longer shares the previous contact point.
            // Keep only the planes at this new point and its incoming velocity.
            segment_velocity = velocity;
            planes.clear();
        }
        if hit.fraction == 1. {
            break;
        }
        remaining *= 1. - hit.fraction;
        planes.push(hit.normal);
        if planes.len() == 1 && !grounded {
            // Default WALK has no bounce. Surface-dependent sv_bounce and slide
            // redirection are still separate work; ordinary clipping uses one.
            velocity = clip_velocity(segment_velocity, hit.normal);
            segment_velocity = velocity;
            continue;
        }
        let mut candidate = Vec3::ZERO;
        let mut found = false;
        for (i, &plane) in planes.iter().enumerate() {
            let clipped = clip_velocity(segment_velocity, plane);
            candidate = clipped;
            if planes
                .iter()
                .enumerate()
                .all(|(j, normal)| i == j || clipped.dot(*normal) >= 0.)
            {
                candidate = clipped;
                found = true;
                break;
            }
        }
        if !found {
            if planes.len() != 2 {
                velocity = Vec3::ZERO;
                break;
            }
            let axis = planes[0].cross(planes[1]).normalize_or_zero();
            candidate = axis * candidate.dot(axis);
        }
        velocity = candidate;
        if velocity.dot(primal) <= 0. {
            velocity = Vec3::ZERO;
            break;
        }
    }
    if all_fraction == 0. {
        velocity = Vec3::ZERO;
    }
    (position, velocity)
}
fn clip_velocity(incoming: Vec3, normal: Vec3) -> Vec3 {
    let mut clipped = incoming - normal * incoming.dot(normal);
    // Retail TryPlayerMove inlines this correction from ClipVelocity. It does
    // not apply the unrelated STOP_EPSILON macro as component snapping here.
    let adjust = clipped.dot(normal);
    if adjust < 0. {
        clipped -= normal * adjust;
    }
    clipped
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Brush, Plane};
    #[test]
    fn landing_reports_fall_speed_and_dead_players_take_no_input() {
        let world = floor();
        let mut p = Player::new(Vec3::new(0., 0., 400. + 64.));
        let mut landed = None;
        for _ in 0..200 {
            p.step(Input::default(), &world, 0.015);
            landed = landed.or(p.landed);
        }
        // Free fall from 400 units at 600 u/s^2 lands near sqrt(2 * 600 * 400) = 693 u/s.
        let speed = landed.expect("landed");
        assert!((speed - 693.).abs() < 15., "{speed}");
        p.dead = true;
        let start = p.feet;
        let push = Input {
            forward: 1.,
            jump: true,
            crouch: true,
            ..Input::default()
        };
        for _ in 0..30 {
            p.step(push, &world, 0.015);
        }
        assert!((p.feet - start).length() < 0.1);
        assert_eq!(p.eye_height, 14.);
        assert!(!p.crouched);
    }
    fn floor() -> World {
        World {
            brushes: vec![Brush {
                contents: 1,
                planes: vec![
                    Plane {
                        normal: Vec3::Z,
                        distance: 0.,
                    },
                    Plane {
                        normal: -Vec3::Z,
                        distance: 1000.,
                    },
                ],
            }],
            ..Default::default()
        }
    }
    fn solid_box(min: Vec3, max: Vec3) -> Brush {
        Brush {
            contents: 1,
            planes: vec![
                Plane {
                    normal: Vec3::X,
                    distance: max.x,
                },
                Plane {
                    normal: -Vec3::X,
                    distance: -min.x,
                },
                Plane {
                    normal: Vec3::Y,
                    distance: max.y,
                },
                Plane {
                    normal: -Vec3::Y,
                    distance: -min.y,
                },
                Plane {
                    normal: Vec3::Z,
                    distance: max.z,
                },
                Plane {
                    normal: -Vec3::Z,
                    distance: -min.z,
                },
            ],
        }
    }
    #[test]
    fn slide_leaves_a_finite_wall_before_clipping_the_next_surface() {
        let diagonal = Vec3::new(1., -1., 0.).normalize();
        let w = World {
            brushes: vec![
                solid_box(Vec3::new(1., -10., -10.), Vec3::new(2., 1., 10.)),
                Brush {
                    contents: 1,
                    planes: vec![Plane {
                        normal: diagonal,
                        distance: -0.5 / 2f32.sqrt(),
                    }],
                },
            ],
            ..Default::default()
        };
        let (position, velocity) = slide(
            &w,
            Vec3::ZERO,
            Vec3::new(4., 2., 1.),
            1.,
            Vec3::ZERO,
            Vec3::ZERO,
            false,
        );
        // After passing the finite wall's end, the diagonal surface redirects
        // motion toward +X. Retaining the old -X plane falsely forbids that.
        assert!(position.x > 1.1 && position.y > 1.5, "{position:?}");
        // The four-bump limit and float32 tangent contacts can leave a small
        // part of this deliberately long sweep unused, but must retain ascent.
        assert!(position.z > 0.9, "{position:?}, velocity {velocity:?}");
        assert!(velocity.abs_diff_eq(Vec3::ONE, 0.0001), "{velocity:?}");
    }
    #[test]
    fn simultaneous_ground_contacts_clip_the_segment_velocity() {
        let diagonal = Vec3::new(-1., -1., 0.).normalize();
        let w = World {
            brushes: vec![
                Brush {
                    contents: 1,
                    planes: vec![Plane {
                        normal: -Vec3::X,
                        distance: -0.02,
                    }],
                },
                Brush {
                    contents: 1,
                    planes: vec![Plane {
                        normal: diagonal,
                        distance: -0.02,
                    }],
                },
            ],
            ..Default::default()
        };
        // Both walls are within the trace skin. First remove +X, then resolve
        // their shared contact using the original (2,4,1) segment velocity.
        // Sequential projection instead incorrectly produces (-2,2,1).
        let (position, velocity) = slide(
            &w,
            Vec3::ZERO,
            Vec3::new(2., 4., 1.),
            1.,
            Vec3::ZERO,
            Vec3::ZERO,
            true,
        );
        assert!(
            velocity.abs_diff_eq(Vec3::new(-1., 1., 1.), 0.0001),
            "{velocity:?}"
        );
        assert!(
            position.abs_diff_eq(Vec3::new(-1., 1., 1.), 0.0001),
            "{position:?}"
        );
    }
    #[test]
    fn slide_can_exit_an_initial_overlap_but_stops_if_the_whole_path_is_solid() {
        let w = World {
            brushes: vec![solid_box(
                Vec3::new(0., -10., -10.),
                Vec3::new(10., 10., 10.),
            )],
            ..Default::default()
        };
        let start = Vec3::new(5., 0., 0.);
        let (escaped, velocity) =
            slide(&w, start, Vec3::X * 15., 1., Vec3::ZERO, Vec3::ZERO, false);
        assert_eq!(escaped, Vec3::new(20., 0., 0.));
        assert_eq!(velocity, Vec3::X * 15.);
        let (trapped, velocity) = slide(&w, start, Vec3::X, 1., Vec3::ZERO, Vec3::ZERO, false);
        assert_eq!(trapped, start);
        assert_eq!(velocity, Vec3::ZERO);
    }
    #[test]
    fn airborne_duck_tucks_once_and_preserves_view_and_momentum() {
        let w = floor();
        let mut standing = Player::new(Vec3::Z * 64.05);
        standing.step(
            Input {
                jump: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        assert!(!standing.grounded);
        let mut ducked = standing.clone();
        for _ in 0..8 {
            standing.step(Input::default(), &w, TICK);
            ducked.step(
                Input {
                    crouch: true,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            assert!(!ducked.grounded);
            assert!(ducked.crouched);
            assert!((ducked.feet.z - standing.feet.z - 36.).abs() < 0.0001);
            assert!((ducked.eye().z - standing.eye().z).abs() < 0.0001);
            assert_eq!(ducked.velocity, standing.velocity);
            assert_eq!(ducked.eye_height, 28.);
        }
    }
    #[test]
    fn airborne_unduck_restores_origin_and_preserves_view_and_momentum() {
        let w = World::default();
        let mut ducked = Player::new(Vec3::Z * 1064.);
        ducked.velocity = Vec3::new(120., 20., 100.);
        ducked.step(
            Input {
                crouch: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        let mut standing = ducked.clone();
        ducked.step(
            Input {
                crouch: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        standing.step(Input::default(), &w, TICK);
        assert!(!standing.crouched && !standing.grounded);
        assert_eq!(standing.eye_height, 64.);
        assert!((ducked.feet.z - standing.feet.z - 36.).abs() < 0.0001);
        assert!((ducked.eye().z - standing.eye().z).abs() < 0.0001);
        assert_eq!(ducked.velocity, standing.velocity);
    }
    #[test]
    fn duck_classifies_current_hull_instead_of_a_stale_ground_flag() {
        for old_grounded in [false, true] {
            let w = floor();
            let mut ground_player = Player::new(Vec3::Z * 64.05);
            ground_player.grounded = old_grounded;
            ground_player.step(
                Input {
                    crouch: true,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            assert!(ground_player.grounded && ground_player.crouched);
            assert!(ground_player.feet.z < 0.1);

            let mut air_player = Player::new(Vec3::Z * 1064.);
            air_player.grounded = old_grounded;
            air_player.step(
                Input {
                    crouch: true,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            assert!(!air_player.grounded && air_player.crouched);
            assert!((air_player.feet.z - 1035.9325).abs() < 0.0001);
            assert_eq!(air_player.eye_height, 28.);
        }
    }
    #[test]
    fn airborne_unduck_requires_a_clear_standing_hull_sweep() {
        for ceiling in [false, true] {
            let mut w = floor();
            if ceiling {
                w.brushes.push(solid_box(
                    Vec3::new(-100., -100., 64.),
                    Vec3::new(100., 100., 128.),
                ));
            }
            let mut p = Player::new(Vec3::Z * 84.);
            p.crouched = true;
            p.eye_height = 28.;
            // The current crouched hull is clear. A standing-hull sweep starts
            // inside the ceiling, or hits the floor while lowering the origin.
            let start = w.trace(
                p.feet,
                p.feet,
                Vec3::new(-16., -16., 0.),
                Vec3::new(16., 16., 36.),
            );
            assert!(!start.start_solid);
            p.step(Input::default(), &w, TICK);
            assert!(p.crouched);
            assert_eq!(p.eye_height, 28.);
            assert!(!p.grounded);
            assert!(p.feet.z > 19.);
        }
    }
    #[test]
    fn crouch_jump_clears_a_ledge_above_the_ordinary_jump_apex() {
        let mut w = floor();
        w.brushes.push(solid_box(
            Vec3::new(40., -100., 0.),
            Vec3::new(200., 100., 40.),
        ));
        let mut standing = Player::new(Vec3::Z * 64.05);
        standing.step(Input::default(), &w, TICK);
        standing.velocity.x = 180.;
        standing.step(
            Input {
                forward: 1.,
                jump: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        let mut ducked = standing.clone();
        for _ in 0..40 {
            standing.step(
                Input {
                    forward: 1.,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            ducked.step(
                Input {
                    forward: 1.,
                    crouch: true,
                    ..Default::default()
                },
                &w,
                TICK,
            );
        }
        assert!(
            standing.feet.x < 24.01,
            "ordinary jump crossed: {:?}",
            standing.feet
        );
        assert!(standing.feet.z < 0.1);
        assert!(
            ducked.feet.x > 50.,
            "crouch jump blocked: {:?}",
            ducked.feet
        );
        assert!(ducked.grounded && ducked.feet.z >= 40.);
    }
    #[test]
    fn airborne_duck_next_to_a_wall_retains_vertical_motion() {
        let mut w = floor();
        w.brushes.push(solid_box(
            Vec3::new(32., -100., 0.),
            Vec3::new(48., 100., 128.),
        ));
        let mut p = Player::new(Vec3::new(15.95, 0., 74.));
        p.velocity = Vec3::new(190., 0., 100.);
        p.step(
            Input {
                forward: 1.,
                crouch: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        assert!(p.crouched && !p.grounded);
        assert!(p.feet.x < 16.);
        assert!((p.feet.z - 47.4325).abs() < 0.0001);
        assert!((p.velocity.z - 91.).abs() < 0.0001);
    }
    #[test]
    fn ground_acceleration_friction_and_speed_limit() {
        let w = floor();
        let mut p = Player::new(Vec3::Z * 64.05);
        for _ in 0..100 {
            p.step(
                Input {
                    forward: 1.,
                    ..Default::default()
                },
                &w,
                TICK,
            );
        }
        assert!((p.velocity.x - 190.).abs() < 0.01);
        assert!(p.grounded);
        for _ in 0..100 {
            p.step(Input::default(), &w, TICK);
        }
        assert!(p.velocity.length() < 0.01);
    }
    #[test]
    fn jump_boost_crops_diagonal_forward_command() {
        let w = floor();
        let mut p = Player::new(Vec3::Z * 64.05);
        p.step(Input::default(), &w, TICK);
        p.step(
            Input {
                forward: 1.,
                side: 1.,
                jump: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        // The cropped forward command is190/sqrt(2); its half-speed boost
        // already exceeds the30-unit wish-direction air cap, so no air gain.
        assert!((p.velocity.x - 190. / 2f32.sqrt() * 0.5).abs() < 0.0001);
        assert!(p.velocity.y.abs() < 0.0001);
    }
    #[test]
    fn jump_boost_keeps_negative_cap_addition() {
        let w = floor();
        for forward in [0., 1.] {
            let mut p = Player::new(Vec3::Z * 64.05);
            p.step(Input::default(), &w, TICK);
            p.velocity.x = 400.;
            p.step(
                Input {
                    forward,
                    jump: true,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            // Native caps to190*1.5 even with no forward input: the signed
            // addition can be negative and zero input does not erase it.
            assert!((p.velocity.x - 285.).abs() < 0.0001);
        }
    }
    #[test]
    fn grounded_duck_jump_crops_addition_but_preserves_normal_speed_cap() {
        let w = floor();
        for (speed, forward, expected) in [
            (180., 1., 180. + (190. / 3.) * 0.1),
            (400., 1., 209.),
            (-400., 0., -591.),
        ] {
            let mut p = Player::new(Vec3::Z * 64.05);
            p.step(
                Input {
                    crouch: true,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            p.velocity.x = speed;
            p.step(
                Input {
                    forward,
                    jump: true,
                    crouch: true,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            // Retail uses abs(cropped forwardmove * .1) for the addition but
            // maxspeed190 *1.1 for its cap. Neutral backward overspeed therefore
            // retains the signed -191 addition, rather than a crouch-speed cap.
            assert!(
                (p.velocity.x - expected).abs() < 0.0001,
                "{speed} -> {}",
                p.velocity.x
            );
            assert!((p.velocity.z - 151.).abs() < 0.0001);
            assert!(!p.grounded);
        }
    }
    #[test]
    fn airborne_duck_matches_standing_air_acceleration_without_ground_crop() {
        let w = World::default();
        for (vertical, expected) in [(144.5, 7.125), (144.6, 28.5)] {
            for crouch in [false, true] {
                let mut p = Player::new(Vec3::Z * 1064.);
                p.velocity.z = vertical;
                p.step(
                    Input {
                        crouch,
                        ..Default::default()
                    },
                    &w,
                    TICK,
                );
                p.step(
                    Input {
                        side: 1.,
                        crouch,
                        ..Default::default()
                    },
                    &w,
                    TICK,
                );
                // Air duck has wishspeed190; previous categorization provides
                // friction.25 or1, giving gains7.125 or28.5 along the strafe axis.
                assert!((p.velocity.y + expected).abs() < 0.0001);
                assert!(p.velocity.x.abs() < 0.0001);
                assert!(!p.grounded);
            }
        }
    }
    #[test]
    fn released_jump_can_chain_landings_without_a_ground_friction_tick() {
        let w = floor();
        for crouch in [false, true] {
            let mut p = Player::new(Vec3::Z * 64.05);
            p.step(
                Input {
                    crouch,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            p.velocity.x = 200.;
            for _ in 0..3 {
                assert!(p.grounded);
                let start = p.feet.z;
                p.step(
                    Input {
                        jump: true,
                        crouch,
                        ..Default::default()
                    },
                    &w,
                    TICK,
                );
                let (displacement, vertical) = if crouch {
                    (2.3325, 151.)
                } else {
                    (2.265, 146.5)
                };
                assert!((p.feet.z - start - displacement).abs() < 0.0001);
                assert!((p.velocity.z - vertical).abs() < 0.0001);
                assert!((p.velocity.x - 200.).abs() < 0.0001);
                // Release in air, then jump on the first already-grounded tick.
                // Landing is categorized after AirMove; no friction runs until
                // a later grounded tick, and CheckJumpButton precedes friction.
                for _ in 0..100 {
                    if p.grounded {
                        break;
                    }
                    p.step(
                        Input {
                            crouch,
                            ..Default::default()
                        },
                        &w,
                        TICK,
                    );
                }
                assert!(p.grounded);
                assert!((p.velocity.x - 200.).abs() < 0.0001);
            }
        }
    }
    #[test]
    fn air_friction_respects_rapid_ascent_return_and_next_tick_order() {
        let w = World::default();
        for (initial_z_velocity, expected_gain) in [(144.5, 7.125), (144.6, 28.5)] {
            let mut p = Player::new(Vec3::Z * 1064.);
            p.velocity.z = initial_z_velocity;
            p.step(Input::default(), &w, TICK);
            p.step(
                Input {
                    forward: 1.,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            // At categorization before final gravity, vz140 assigns0.25;
            // vz140.1 takes the rapid-ascent return and retains the reset1.
            assert!((p.velocity.x - expected_gain).abs() < 0.0001);
        }
    }
    #[test]
    fn air_friction_from_ascent_survives_the_first_descending_acceleration() {
        let w = World::default();
        let mut p = Player::new(Vec3::Z * 1064.);
        p.velocity.z = 10.;
        p.step(Input::default(), &w, TICK);
        assert!((p.velocity.z - 1.).abs() < 0.0001);
        let input = Input {
            forward: 1.,
            ..Default::default()
        };
        p.step(input, &w, TICK);
        assert!(p.velocity.z < 0.);
        assert!((p.velocity.x - 7.125).abs() < 0.0001);
        p.velocity.x = 0.;
        p.step(input, &w, TICK);
        assert!((p.velocity.x - 28.5).abs() < 0.0001);
    }
    #[test]
    fn standing_jump_first_tick_preserves_start_gravity() {
        let w = floor();
        let mut p = Player::new(Vec3::Z * 64.05);
        p.step(Input::default(), &w, TICK);
        assert!(p.grounded);
        let start = p.feet.z;
        p.step(
            Input {
                jump: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        // Retail FullWalkMove: -4.5, +160, -4.5, sweep, -4.5.
        assert!((p.feet.z - start - 2.265).abs() < 0.0001);
        assert!((p.velocity.z - 146.5).abs() < 0.0001);
        assert!(!p.grounded);
    }
    #[test]
    fn crouched_jump_first_tick_replaces_start_gravity() {
        let w = floor();
        let mut p = Player::new(Vec3::Z * 64.05);
        p.step(
            Input {
                crouch: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        assert!(p.grounded && p.crouched);
        let start = p.feet.z;
        p.step(
            Input {
                jump: true,
                crouch: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        // The ducked branch assigns 160, then applies the two remaining half-steps.
        assert!((p.feet.z - start - 2.3325).abs() < 0.0001);
        assert!((p.velocity.z - 151.).abs() < 0.0001);
        assert!(!p.grounded);
    }
    #[test]
    fn leaving_a_ledge_categorizes_before_finish_gravity() {
        let mut w = floor();
        w.brushes[0].planes.push(Plane {
            normal: Vec3::X,
            distance: 0.,
        });
        let mut p = Player::new(Vec3::new(15.5, 0., 64.05));
        p.step(Input::default(), &w, TICK);
        assert!(p.grounded);
        let height = p.feet.z;
        p.velocity.x = 190.;
        p.step(Input::default(), &w, TICK);
        assert!(p.feet.x > 16.);
        assert!(!p.grounded);
        assert!((p.feet.z - height).abs() < 0.0001);
        assert!((p.velocity.z + 4.5).abs() < 0.0001);
    }
    #[test]
    fn hl2_jump_apex_and_release_gate() {
        let w = floor();
        let mut p = Player::new(Vec3::Z * 64.05);
        p.step(Input::default(), &w, TICK);
        let start = p.feet.z;
        let mut top = 0f32;
        for _ in 0..100 {
            p.step(
                Input {
                    jump: true,
                    ..Default::default()
                },
                &w,
                TICK,
            );
            top = top.max(p.feet.z);
        }
        // At this fixed tick, upward sweep velocities are151,142,...,7.
        assert!((top - start - 20.145).abs() < 0.001, "apex {top}");
        assert!(p.grounded);
        assert!(p.feet.z < 0.1);
        p.step(Input::default(), &w, TICK);
        p.step(
            Input {
                jump: true,
                ..Default::default()
            },
            &w,
            TICK,
        );
        assert!(!p.grounded);
        assert!((p.velocity.z - 146.5).abs() < 0.0001);
    }
    #[test]
    fn fixed_input_replay_is_identical() {
        let w = floor();
        let mut a = Player::new(Vec3::Z * 64.05);
        let mut b = a.clone();
        for i in 0..200 {
            let input = Input {
                forward: 1.,
                side: 0.2,
                jump: i == 70,
                ..Default::default()
            };
            a.step(input, &w, TICK);
            b.step(input, &w, TICK);
        }
        assert_eq!(a.feet, b.feet);
        assert_eq!(a.velocity, b.velocity);
    }
}
