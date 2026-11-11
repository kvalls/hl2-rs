//! A scanner's spotlight (DESIGN 14o session 47): where the beam points and where it ends. The rules
//! are the SDK's SpotlightCreate, SpotlightTargetPos, SpotlightCurrentPos and SpotlightUpdate, run
//! once per think; `runtime::scanners` traces the beam and the host draws it.

use crate::projectile_visuals::Quad;
use glam::Vec3;
use serde::Serialize;

/// The beam's material and the halo at its start.
pub const BEAM_SPRITE: &str = "sprites/glow_test02";
pub const HALO_SPRITE: &str = "sprites/light_glow03";
/// SPOTLIGHT_WIDTH at the full length of the beam.
const WIDTH: f32 = 32.;
/// SpotlightLength where a map leaves it out or gives nothing usable.
pub const DEFAULT_REACH: f32 = 500.;
/// How far a new beam looks for what it lands on.
pub const FIRST_TRACE: f32 = 2024.;
/// How far ahead the turn rate reckons from: the beam turns slower the longer it is.
const TURN_REACH: f32 = 50.;
/// The share of last think's turning that carries over, and the most it can turn in one.
const TURN_KEEP: f32 = 0.4;
const MOST_TURN: f32 = 1.;
/// How fast the end chases the point the beam lands on, per unit away; past `END_JUMP` it jumps.
const END_CHASE: f32 = 10.;
const END_JUMP: f32 = 200.;
/// The share of the beam's length a think takes in.
const LENGTH_TAKEN: f32 = 0.2;
/// How bright the beam is at its start, of 255: its 32 at SetHDRColorScale's 0.75, since the owned
/// game runs with HDR. And how many pieces carry its fade.
const BRIGHTNESS: f32 = 24.;
const PIECES: usize = 12;
/// SetHaloScale, and the HDR scale the halo's white takes as the beam's color does.
const HALO_SIZE: f32 = 32.;
const HDR_SCALE: f32 = 0.75;
/// How many widths from the beam's line the view starts to see it dim and its halo grow.
const NEAR_LINE: f32 = 4.;
/// On patrol it sweeps ahead of its flight: how far ahead against the noise, and how much noise.
const SWEEP_AHEAD: f32 = 5.;
const SWEEP_NOISE: f32 = 2.5;
const SWEEP_REACH: f32 = 2024.;
/// How long it stays off after being shot, after a flash, on the way to a photograph and after
/// swinging behind the body.
pub const OFF_AFTER_HIT: f64 = 2.;
pub const OFF_AFTER_FLASH: f64 = 1.;
pub const OFF_FOR_PHOTO: f64 = 2.;
pub const OFF_WHEN_BEHIND: f64 = 3.;

/// A lit spotlight.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Spotlight {
    /// m_vSpotlightDir and m_vSpotlightAngVelocity.
    pub direction: Vec3,
    pub turning: Vec3,
    /// The end of the beam (the spotlight_end entity) and its speed.
    pub end: Vec3,
    pub end_velocity: Vec3,
    /// m_flSpotlightCurLength, how far the beam shows before it fades.
    pub length: f32,
    /// SpotlightLength.
    pub reach: f32,
}

/// What the host draws (CBeam with FBEAM_SHADEOUT): a scanner's or a point_spotlight's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beam {
    pub entity: usize,
    pub start: Vec3,
    pub end: Vec3,
    /// Its width at the start and at the end.
    pub width: f32,
    pub end_width: f32,
    /// How far along the beam it fades out.
    pub fade_length: f32,
    /// Its color, and its brightness of 255 with the HDR color scale applied.
    pub color: [u8; 3],
    pub intensity: f32,
    /// SetHaloScale (half the halo's side) and the HDR scale on the halo's color.
    pub halo_size: f32,
    pub halo_hdr: f32,
}

impl Spotlight {
    /// A beam just lit, pointing along `direction` and landing at `end`.
    pub fn new(direction: Vec3, end: Vec3, reach: f32) -> Self {
        Self {
            direction,
            turning: Vec3::ZERO,
            end,
            end_velocity: Vec3::ZERO,
            length: reach,
            reach,
        }
    }
    /// One think of the beam swinging toward `target`: it has momentum, and a long beam turns
    /// slower, so its end moves over the ground at about the same pace.
    pub fn swing(&mut self, origin: Vec3, target: Vec3) {
        let wanted = (target - origin).normalize_or_zero();
        let rate = (TURN_REACH / self.end.distance(origin).max(1.)).atan();
        let turn = rate * (wanted - self.direction);
        self.turning = TURN_KEEP * self.turning + (1. - TURN_KEEP) * turn;
        self.turning = self.turning.clamp_length_max(MOST_TURN);
        self.direction = (self.direction + self.turning).normalize_or(self.direction);
    }
    /// How far a think's trace looks along the beam.
    pub fn trace_reach(&self) -> f32 {
        2. * self.reach
    }
    /// Where the beam now lands: the end sets off for it, or jumps there when it is far off, as
    /// over the edge of a roof. The beam's shown length eases toward its new length.
    pub fn land(&mut self, origin: Vec3, landing: Vec3) {
        let away = landing - self.end;
        let chase = away * END_CHASE;
        if chase.length() > END_JUMP {
            self.end_velocity = chase.normalize_or_zero() * END_JUMP;
            self.end = landing;
        } else {
            self.end_velocity = chase;
        }
        let beam = self.end.distance(origin);
        self.length += (beam - self.length) * LENGTH_TAKEN;
    }
    /// A tick of the end's travel.
    pub fn carry(&mut self, dt: f32) {
        self.end += self.end_velocity * dt;
    }
    /// What the host draws, from where the light is this frame.
    pub fn beam(&self, entity: usize, start: Vec3) -> Beam {
        let width = WIDTH * self.end.distance(start) / self.reach;
        Beam {
            entity,
            start,
            end: self.end,
            width,
            end_width: width,
            fade_length: self.length.min(self.reach),
            color: [255; 3],
            intensity: BRIGHTNESS,
            halo_size: HALO_SIZE,
            halo_hdr: HDR_SCALE,
        }
    }
}

/// The glow at a beam's start, as seen from one place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Halo {
    pub at: Vec3,
    /// The beam's color, which the halo takes.
    pub color: [u8; 3],
    /// Half its side, in units.
    pub size: f32,
    pub brightness: f32,
    pub entity: usize,
}

/// `value`'s place between `from` and `to`, carried over to between `low` and `high`.
fn remap(value: f32, (from, to): (f32, f32), (low, high): (f32, f32)) -> f32 {
    low + (high - low) * (value - from) / (to - from)
}

impl Beam {
    /// Whether any of it lies ahead of a view at `eye` looking along `direction`.
    pub fn ahead_of(&self, eye: Vec3, direction: Vec3) -> bool {
        direction.dot(self.start - eye) > 0. || direction.dot(self.end - eye) > 0.
    }
    /// How far `eye` is from the beam's line, which runs on past its end.
    fn distance_to_line(&self, eye: Vec3) -> f32 {
        let along = (self.end - self.start).normalize_or_zero();
        let offset = eye - self.start;
        (offset - along * offset.dot(along)).length()
    }
    /// A beam aimed at the view dims out: it would fill the screen otherwise.
    fn dimming(&self, eye: Vec3) -> f32 {
        let distance = self.distance_to_line(eye);
        let near = self.width * NEAR_LINE;
        if distance >= near {
            return 1.;
        }
        remap(distance, (near, self.width), (1., 0.)).clamp(0., 1.)
    }
    /// The halo as seen from `eye`, when that is in front of the light. It grows as the view
    /// nears the line and brightens as it comes round to face the light.
    pub fn halo(&self, eye: Vec3) -> Option<Halo> {
        let along = (self.end - self.start).normalize_or_zero();
        let facing = along.dot((eye - self.start).normalize_or_zero());
        if facing <= 0. {
            return None;
        }
        let near = self.width * NEAR_LINE;
        let grown = remap(
            self.distance_to_line(eye),
            (near, self.width * 0.5),
            (1., 2.),
        );
        let fade = facing * 2.;
        Some(Halo {
            at: self.start,
            color: self.color,
            size: grown.clamp(1., 2.) * self.halo_size,
            brightness: (fade * fade).clamp(0., 1.) * self.halo_hdr,
            entity: self.entity,
        })
    }
    /// How bright it is `along` the way from its start, from 1 down to 0 at the fade length.
    fn brightness(&self, along: f32) -> f32 {
        let length = self.start.distance(self.end).max(1.);
        let fade = (self.fade_length / length).clamp(1e-6, 1.);
        (1. - along / fade).clamp(0., 1.)
    }
    /// A strip cut into pieces that share their edges, each edge facing the view and each piece
    /// as bright as its start: a quad has one color and the beam fades along its length.
    pub fn quads(&self, eye: Vec3) -> Vec<Quad> {
        let run = self.end - self.start;
        let at = |piece: usize| piece as f32 / PIECES as f32;
        let edge = |piece: usize| {
            let point = self.start + run * at(piece);
            let width = self.width + (self.end_width - self.width) * at(piece);
            let side = run.cross(point - eye).normalize_or_zero() * width * 0.5;
            [point - side, point + side]
        };
        let dimming = self.dimming(eye);
        if dimming <= 0. {
            return Vec::new();
        }
        let lit = (0..PIECES).filter(|&piece| self.brightness(at(piece)) > 0.);
        lit.filter_map(|piece| {
            let ([a, b], [c, d]) = (edge(piece), edge(piece + 1));
            if a == b || c == d {
                return None;
            }
            let (near, far) = (at(piece), at(piece + 1));
            let shade = self.brightness(near) * dimming * self.intensity / 255.;
            let [red, green, blue] = self.color.map(|c| (f32::from(c) * shade).round() as u8);
            Some(Quad {
                material: BEAM_SPRITE.into(),
                positions: [a, b, d, c],
                color: [red, green, blue, 255],
                uv: [[1., near], [0., near], [0., far], [1., far]],
            })
        })
        .collect()
    }
}

/// The patrol's sweep: ahead of its flight, tipped down by the noise, a long way off.
pub fn sweep_target(origin: Vec3, velocity: Vec3, bob: Vec3, now: f32) -> Vec3 {
    let wave = |rate: f32| rate * now + rate;
    let ahead = velocity.truncate().normalize_or_zero().extend(0.) * SWEEP_AHEAD;
    let noise = Vec3::new(
        wave(bob.x).sin(),
        wave(bob.y).cos(),
        -wave(bob.z).cos().abs(),
    ) * SWEEP_NOISE;
    origin + (ahead + noise) * SWEEP_REACH
}

/// A key's beam length: what is not a positive number gets the default.
pub fn reach_from(key: Option<&str>) -> f32 {
    let given = key.and_then(|value| value.trim().parse::<f32>().ok());
    given.filter(|&reach| reach > 0.).unwrap_or(DEFAULT_REACH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointing_down() -> Spotlight {
        Spotlight::new(Vec3::NEG_Z, Vec3::NEG_Z * 200., DEFAULT_REACH)
    }

    #[test]
    fn it_swings_toward_its_target_without_snapping() {
        let mut light = pointing_down();
        let target = Vec3::new(200., 0., -200.);
        light.swing(Vec3::ZERO, target);
        assert!(light.direction.x > 0., "{:?}", light.direction);
        let wanted = target.normalize();
        assert!(light.direction.dot(wanted) < 0.95, "{:?}", light.direction);
    }

    #[test]
    fn its_turning_carries_over_so_it_gathers_speed() {
        let mut light = pointing_down();
        let target = Vec3::new(200., 0., -200.);
        light.swing(Vec3::ZERO, target);
        let first = light.turning.length();
        light.swing(Vec3::ZERO, target);
        assert!(light.turning.length() > first);
    }

    #[test]
    fn a_long_beam_turns_slower_than_a_short_one() {
        let turned = |distance: f32| {
            let mut light = Spotlight::new(Vec3::NEG_Z, Vec3::NEG_Z * distance, DEFAULT_REACH);
            light.swing(Vec3::ZERO, Vec3::X);
            light.direction.x
        };
        assert!(turned(100.) > turned(1000.));
    }

    #[test]
    fn it_settles_on_a_target_that_holds_still() {
        let mut light = pointing_down();
        let target = Vec3::new(100., 0., -200.);
        for _ in 0..200 {
            light.swing(Vec3::ZERO, target);
        }
        assert!(light.direction.dot(target.normalize()) > 0.9999);
    }

    #[test]
    fn its_turning_has_a_limit() {
        let mut light = Spotlight::new(Vec3::X, Vec3::X * 0.5, DEFAULT_REACH);
        light.turning = Vec3::NEG_X * 3.;
        light.swing(Vec3::ZERO, Vec3::NEG_X);
        assert!(light.turning.length() <= MOST_TURN + 1e-4);
    }

    #[test]
    fn the_end_chases_a_near_landing_and_jumps_to_a_far_one() {
        let mut light = pointing_down();
        let start = light.end;
        light.land(Vec3::ZERO, start + Vec3::X * 10.);
        assert_eq!(light.end, start);
        assert_eq!(light.end_velocity, Vec3::X * 100.);
        light.carry(0.05);
        assert_eq!(light.end, start + Vec3::X * 5.);

        let far = start + Vec3::X * 300.;
        light.land(Vec3::ZERO, far);
        assert_eq!(light.end, far);
        assert!((light.end_velocity.length() - END_JUMP).abs() < 1e-3);
    }

    #[test]
    fn the_shown_length_eases_toward_the_beam() {
        let mut light = pointing_down();
        assert_eq!(light.length, 500.);
        light.land(Vec3::ZERO, light.end);
        assert_eq!(light.length, 440.);
    }

    #[test]
    fn the_width_grows_with_the_length_and_the_fade_stops_at_its_reach() {
        let mut light = Spotlight::new(Vec3::NEG_Z, Vec3::NEG_Z * 250., 500.);
        assert_eq!(light.beam(3, Vec3::ZERO).width, 16.);
        light.end = Vec3::NEG_Z * 1000.;
        light.length = 900.;
        let beam = light.beam(3, Vec3::ZERO);
        assert_eq!(beam.width, 64.);
        assert_eq!(beam.fade_length, 500.);
        light.length = 120.;
        assert_eq!(light.beam(3, Vec3::ZERO).fade_length, 120.);
    }

    fn beam(fade_length: f32) -> Beam {
        Beam {
            entity: 3,
            start: Vec3::ZERO,
            end: Vec3::NEG_Z * 400.,
            width: 32.,
            end_width: 32.,
            fade_length,
            color: [255; 3],
            intensity: BRIGHTNESS,
            halo_size: HALO_SIZE,
            halo_hdr: HDR_SCALE,
        }
    }

    #[test]
    fn the_beam_is_brightest_at_the_eye_and_fades_along_its_length() {
        let quads = beam(400.).quads(Vec3::new(300., 0., -200.));
        assert_eq!(quads.len(), PIECES);
        assert_eq!(quads[0].color[0], 24);
        let shades: Vec<_> = quads.iter().map(|q| q.color[0]).collect();
        assert!(
            shades.windows(2).all(|pair| pair[0] > pair[1]),
            "{shades:?}"
        );
    }

    #[test]
    fn past_the_fade_length_nothing_is_drawn() {
        // Fading out at a quarter of the way: three pieces of twelve.
        assert_eq!(beam(100.).quads(Vec3::new(300., 0., -200.)).len(), 3);
    }

    #[test]
    fn its_pieces_share_their_edges() {
        let quads = beam(400.).quads(Vec3::new(300., 0., -200.));
        for pair in quads.windows(2) {
            assert_eq!(pair[0].positions[2], pair[1].positions[1]);
            assert_eq!(pair[0].positions[3], pair[1].positions[0]);
        }
    }

    #[test]
    fn seen_straight_along_its_length_it_has_no_width_to_draw() {
        assert!(beam(400.).quads(Vec3::Z * 100.).is_empty());
    }

    #[test]
    fn a_beam_wholly_behind_the_view_is_not_ahead_of_it() {
        let side = Vec3::new(-300., 0., -200.);
        assert!(beam(400.).ahead_of(side, Vec3::X));
        assert!(!beam(400.).ahead_of(side, Vec3::NEG_X));
        // One end in front is enough.
        assert!(beam(400.).ahead_of(Vec3::Z * -200., Vec3::Z));
    }

    #[test]
    fn it_dims_out_as_the_view_nears_its_line() {
        // The beam runs down the z axis, 32 wide: dimming starts at 128 units and ends at 32.
        let at = |x: f32| Vec3::new(x, 0., -200.);
        assert_eq!(beam(400.).dimming(at(128.)), 1.);
        assert_eq!(beam(400.).dimming(at(500.)), 1.);
        assert_eq!(beam(400.).dimming(at(80.)), 0.5);
        assert_eq!(beam(400.).dimming(at(32.)), 0.);
        assert_eq!(beam(400.).dimming(at(5.)), 0.);
        assert!(beam(400.).quads(at(20.)).is_empty());
        let shade = |x: f32| beam(400.).quads(at(x))[0].color[0];
        assert_eq!(shade(80.), 12);
    }

    #[test]
    fn behind_the_light_there_is_no_halo() {
        assert_eq!(beam(400.).halo(Vec3::Z * 50.), None);
        assert_eq!(beam(400.).halo(Vec3::new(100., 0., 50.)), None);
    }

    #[test]
    fn the_halo_grows_toward_the_line_and_stays_at_the_light() {
        let far = beam(400.)
            .halo(Vec3::new(200., 0., -600.))
            .expect("in front");
        assert_eq!(far.size, HALO_SIZE);
        assert_eq!(far.at, Vec3::ZERO);
        let on_line = beam(400.).halo(Vec3::new(4., 0., -300.)).expect("in front");
        assert_eq!(on_line.size, HALO_SIZE * 2.);
    }

    #[test]
    fn the_halo_brightens_as_the_view_comes_round_to_face_the_light() {
        let side = beam(400.)
            .halo(Vec3::new(400., 0., -100.))
            .expect("in front");
        let ahead = beam(400.)
            .halo(Vec3::new(10., 0., -300.))
            .expect("in front");
        assert!(side.brightness < ahead.brightness);
        assert_eq!(ahead.brightness, HDR_SCALE);
    }

    #[test]
    fn a_patrol_sweeps_down_and_ahead_of_its_flight() {
        let origin = Vec3::Z * 500.;
        let target = sweep_target(origin, Vec3::X * 100., Vec3::ONE, 0.5);
        assert!(target.x > origin.x, "{target:?}");
        assert!(target.z < origin.z, "{target:?}");
    }

    #[test]
    fn a_climb_or_a_stop_leaves_the_sweep_with_only_its_noise() {
        let origin = Vec3::ZERO;
        let climbing = sweep_target(origin, Vec3::Z * 100., Vec3::ONE, 0.5);
        let stopped = sweep_target(origin, Vec3::ZERO, Vec3::ONE, 0.5);
        assert_eq!(climbing, stopped);
    }

    #[test]
    fn the_length_key_falls_back_on_anything_unusable() {
        assert_eq!(reach_from(Some("800")), 800.);
        assert_eq!(reach_from(Some(" 0 ")), DEFAULT_REACH);
        assert_eq!(reach_from(Some("-5")), DEFAULT_REACH);
        assert_eq!(reach_from(Some("far")), DEFAULT_REACH);
        assert_eq!(reach_from(None), DEFAULT_REACH);
    }
}
