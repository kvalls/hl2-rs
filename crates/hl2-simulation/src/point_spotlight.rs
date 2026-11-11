//! point_spotlight (DESIGN 14t): a lamp's volumetric beam and the halo at its start. Source SDK
//! point_spotlight.cpp: an unparented spotlight is made once, at Activate
//! (CreateEfficientSpotlight and ComputeRenderInfo): a trace along its angles that only brushes
//! stop, then a glow_test02 beam from its origin to the hit, drawn like a scanner's. The beam has
//! a halo texture, so the client draws it through DrawBeamWithHalo (view_beams.cpp): at its start
//! width from end to end, whatever ComputeRenderInfo's end width says.
use crate::ai::scanner::spotlight::Beam;
use crate::physics::Physics;
use glam::Vec3;
use modkit_core::{Entity, World};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// SF_SPOTLIGHT_START_LIGHT_ON.
const START_ON: i64 = 1;
/// Spawn's SpotlightLength and SpotlightWidth where a map gives 0 or less, and MAX_BEAM_WIDTH.
const DEFAULT_LENGTH: f32 = 500.;
const DEFAULT_WIDTH: f32 = 10.;
const MAX_BEAM_WIDTH: f32 = 102.3;
/// SpotlightCreate: SetBrightness(64). Its SetHaloScale(60) halo is not drawn: Source fades it
/// by an occlusion query (pixel visibility), which hides d1_trainstation_02's spawn lamp halo
/// inside its shade, and our halo's one-ray test does not (DESIGN 14t).
const BRIGHTNESS: f32 = 64.;
const HALO_SCALE: f32 = 0.;

/// The map's point_spotlights: which are lit, and their beams, traced once.
#[derive(Debug, Default)]
pub struct Spotlights {
    /// Lit by spawnflag 1, LightOn and LightOff.
    pub lit: BTreeSet<usize>,
    beams: OnceLock<Vec<Beam>>,
}

impl Spotlights {
    pub fn new(world: &World) -> Self {
        let lit = world
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.class() == "point_spotlight" && flags(e) & START_ON != 0)
            .map(|(id, _)| id)
            .collect();
        Self {
            lit,
            beams: OnceLock::new(),
        }
    }
    /// InputLightOn and InputLightOff; false for any other input.
    pub fn input(&mut self, id: usize, input: &str) -> bool {
        match input {
            "lighton" => self.lit.insert(id),
            "lightoff" => self.lit.remove(&id),
            _ => return false,
        };
        true
    }
    /// The lit spotlights' beams; the first call traces every unparented spotlight's.
    pub fn beams<'a>(
        &'a self,
        world: &World,
        physics: &Physics,
    ) -> impl Iterator<Item = &'a Beam> + 'a {
        let all = self.beams.get_or_init(|| {
            world
                .entities
                .iter()
                .enumerate()
                .filter(|(_, e)| e.class() == "point_spotlight")
                .filter_map(|(id, e)| beam(id, e, &|a, b| physics.brush_ray(a, b)))
                .collect()
        });
        all.iter().filter(|b| self.lit.contains(&b.entity))
    }
}

fn flags(entity: &Entity) -> i64 {
    entity
        .get("spawnflags")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

fn number(entity: &Entity, key: &str, default: f32) -> f32 {
    entity
        .get(key)
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

/// One spotlight's beam; None for a parented one (it follows its parent each think: not in)
/// or one without an origin. `trace` is MASK_SOLID_BRUSHONLY's hit between two points.
fn beam(id: usize, entity: &Entity, trace: &dyn Fn(Vec3, Vec3) -> Option<Vec3>) -> Option<Beam> {
    if entity
        .get("parentname")
        .is_some_and(|p| !p.trim().is_empty())
    {
        return None;
    }
    let origin = entity.get("origin").and_then(modkit_core::parse_vec3)?;
    let angles = entity
        .get("angles")
        .and_then(modkit_core::parse_vec3)
        .unwrap_or_default();
    let positive = |value: f32, default| if value > 0. { value } else { default };
    let length = positive(number(entity, "SpotlightLength", 0.), DEFAULT_LENGTH);
    let width = positive(number(entity, "SpotlightWidth", 0.), DEFAULT_WIDTH).min(MAX_BEAM_WIDTH);
    // SpotlightCurrentPos: twice the length, through everything with IgnoreSolid.
    let far = origin + crate::shadows::forward(angles.x, angles.y) * 2. * length;
    let end = if number(entity, "IgnoreSolid", 0.) != 0. {
        far
    } else {
        trace(origin, far).unwrap_or(far)
    };
    let reached = origin.distance(end);
    let hdr = number(entity, "HDRColorScale", 1.);
    let color = entity
        .get("rendercolor")
        .and_then(modkit_core::parse_vec3)
        .unwrap_or(Vec3::splat(255.));
    Some(Beam {
        entity: id,
        start: origin,
        end,
        width,
        end_width: width,
        fade_length: reached.min(length),
        color: color.to_array().map(|c| c.clamp(0., 255.) as u8),
        intensity: BRIGHTNESS * hdr,
        halo_size: HALO_SCALE,
        halo_hdr: hdr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spotlight(pairs: &[(&str, &str)]) -> Entity {
        let mut properties = vec![("classname".to_owned(), "point_spotlight".to_owned())];
        properties.extend(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
        );
        Entity { properties }
    }

    /// d1_trainstation_02's spawn lamp, 100 units over a floor at z = 0.
    fn lamp() -> Entity {
        spotlight(&[
            ("origin", "0 0 100"),
            ("angles", "90 0 0"),
            ("spotlightwidth", "64"),
            ("spotlightlength", "256"),
            ("rendercolor", "254 216 146"),
            ("HDRColorScale", ".7"),
            ("spawnflags", "3"),
        ])
    }

    fn floor(a: Vec3, b: Vec3) -> Option<Vec3> {
        (b.z < 0.).then(|| Vec3::new(a.x, a.y, 0.))
    }

    #[test]
    fn spawnflag_one_and_the_inputs_light_it() {
        let world = World {
            entities: vec![lamp(), spotlight(&[("spawnflags", "2")])],
            ..Default::default()
        };
        let mut lights = Spotlights::new(&world);
        assert_eq!(lights.lit, BTreeSet::from([0]));
        assert!(lights.input(1, "lighton"));
        assert!(lights.input(0, "lightoff"));
        assert_eq!(lights.lit, BTreeSet::from([1]));
        assert!(!lights.input(1, "toggle"));
    }

    #[test]
    fn the_beam_ends_on_the_floor_as_wide_as_it_starts() {
        let beam = beam(4, &lamp(), &floor).expect("beam");
        assert_eq!(beam.start, Vec3::new(0., 0., 100.));
        assert!(beam.end.distance(Vec3::ZERO) < 1e-3, "{:?}", beam.end);
        assert_eq!((beam.width, beam.end_width), (64., 64.));
        assert!((beam.fade_length - 100.).abs() < 1e-3);
        assert_eq!(beam.color, [254, 216, 146]);
        assert!((beam.intensity - 64. * 0.7).abs() < 1e-4);
        assert_eq!((beam.halo_size, beam.halo_hdr), (HALO_SCALE, 0.7));
    }

    #[test]
    fn ignore_solid_and_nothing_hit_reach_twice_the_length_and_parented_ones_are_out() {
        let open = beam(0, &lamp(), &|_, _| None).expect("beam");
        assert!((open.end.z - (100. - 512.)).abs() < 1e-3);
        assert!((open.fade_length - 256.).abs() < 1e-3);
        assert_eq!(open.end_width, 64.);
        let mut through = lamp();
        through.properties.push(("IgnoreSolid".into(), "1".into()));
        assert!((beam(0, &through, &floor).expect("beam").end.z + 412.).abs() < 1e-3);
        let mut parented = lamp();
        parented
            .properties
            .push(("parentname".into(), "train".into()));
        assert!(beam(0, &parented, &floor).is_none());
    }
}
