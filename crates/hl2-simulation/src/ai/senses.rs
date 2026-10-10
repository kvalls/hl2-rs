//! CSoundEnt sounds (soundent.cpp/h) and CAI_Senses look/listen (ai_senses.cpp,
//! FInViewCone in basecombatcharacter.cpp), producing conditions and the best enemy.
//! Line of sight goes through `SightWorld`, which the host implements with its world
//! queries. SDK behavior, retail not compared.
use super::conditions::*;
use super::relationships::{Class, Disposition, Relationships};
use glam::Vec3;
use serde::Serialize;

/// soundent.h sound type bits.
pub mod sound {
    pub const COMBAT: u32 = 0x1;
    pub const WORLD: u32 = 0x2;
    pub const PLAYER: u32 = 0x4;
    pub const DANGER: u32 = 0x8;
    pub const BULLET_IMPACT: u32 = 0x10;
    pub const CARCASS: u32 = 0x20;
    pub const MEAT: u32 = 0x40;
    pub const GARBAGE: u32 = 0x80;
    pub const THUMPER: u32 = 0x100;
    pub const BUGBAIT: u32 = 0x200;
    pub const PHYSICS_DANGER: u32 = 0x400;
    pub const DANGER_SNIPERONLY: u32 = 0x800;
    pub const MOVE_AWAY: u32 = 0x1000;
    pub const ALL_SCENTS: u32 = CARCASS | MEAT | GARBAGE;
}
/// CAI_Senses defaults: m_LookDist 2048; HearingSensitivity 1.0.
pub const LOOK_DISTANCE: f32 = 2048.;

/// A CSound.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Sound {
    pub kind: u32,
    pub origin: Vec3,
    /// Audible radius (m_iVolume).
    pub volume: f32,
    pub expires: f64,
    pub owner: Option<usize>,
}
/// The CSoundEnt list.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SoundEnt {
    pub sounds: Vec<Sound>,
}
impl SoundEnt {
    /// CSoundEnt::InsertSound(type, origin, volume, duration, owner).
    pub fn insert(
        &mut self,
        kind: u32,
        origin: Vec3,
        volume: f32,
        duration: f64,
        owner: Option<usize>,
        now: f64,
    ) {
        // MAX_WORLD_SOUNDS_SP is 64; the oldest is dropped.
        if self.sounds.len() >= 64 {
            self.sounds.remove(0);
        }
        self.sounds.push(Sound {
            kind,
            origin,
            volume,
            expires: now + duration,
            owner,
        });
    }
    /// CSoundEnt::Think: expired sounds are freed.
    pub fn purge(&mut self, now: f64) {
        self.sounds.retain(|s| s.expires > now);
    }
}

/// Visibility from an eye to a point (FVisible: MASK_BLOCKLOS trace).
pub trait SightWorld {
    fn visible(&self, from: Vec3, to: Vec3, ignore: &[usize]) -> bool;
}

/// Something the NPC might see.
#[derive(Clone, Copy, Debug)]
pub struct Seen {
    pub entity: usize,
    pub class: Class,
    /// Eye or center position used for the cone and the trace.
    pub position: Vec3,
    pub alive: bool,
    pub player: bool,
}
/// The looking NPC.
#[derive(Clone, Copy, Debug)]
pub struct Viewer {
    pub entity: usize,
    pub class: Class,
    pub eye: Vec3,
    /// EyeDirection2D (yaw only).
    pub facing: Vec3,
    /// m_flFieldOfView (a dot product; e.g. 0.5 = 120 degrees total).
    pub field_of_view: f32,
    pub look_distance: f32,
}

/// FInViewCone: a 2D dot against the field of view.
pub fn in_view_cone(viewer: &Viewer, spot: Vec3) -> bool {
    let los = (spot - viewer.eye).truncate().normalize_or_zero();
    los.dot(viewer.facing.truncate().normalize_or_zero()) > viewer.field_of_view
}

/// What Look found: conditions and the visible entities with their dispositions.
#[derive(Clone, Debug, Default)]
pub struct Sight {
    pub conditions: Conditions,
    pub visible: Vec<(usize, Disposition, i32, f32)>,
}
/// CAI_Senses::Look + CAI_BaseNPC::OnLooked condition bits (SEE_PLAYER, SEE_HATE,
/// SEE_FEAR, SEE_DISLIKE) for entities within the look distance, in the cone and
/// visible.
pub fn look(
    viewer: &Viewer,
    candidates: &[Seen],
    relationships: &Relationships,
    world: &dyn SightWorld,
) -> Sight {
    let mut sight = Sight::default();
    for c in candidates {
        if c.entity == viewer.entity || !c.alive {
            continue;
        }
        let distance = c.position.distance(viewer.eye);
        if distance > viewer.look_distance || !in_view_cone(viewer, c.position) {
            continue;
        }
        if !world.visible(viewer.eye, c.position, &[viewer.entity, c.entity]) {
            continue;
        }
        let (disposition, priority) = relationships.get(viewer.class, c.class);
        if c.player {
            sight.conditions.set(COND_SEE_PLAYER);
        }
        match disposition {
            Disposition::HT => sight.conditions.set(COND_SEE_HATE),
            Disposition::FR => sight.conditions.set(COND_SEE_FEAR),
            _ => {}
        }
        sight
            .visible
            .push((c.entity, disposition, priority, distance));
    }
    sight
}
/// GetBestEnemy: the highest-priority hated/feared entity, then the nearest.
pub fn best_enemy(sight: &Sight) -> Option<usize> {
    sight
        .visible
        .iter()
        .filter(|(_, d, _, _)| matches!(d, Disposition::HT | Disposition::FR))
        .min_by(|a, b| b.2.cmp(&a.2).then(a.3.total_cmp(&b.3)))
        .map(|v| v.0)
}

/// CAI_Senses::Listen + CAI_BaseNPC::OnListened: audible sounds of interest set the
/// COND_HEAR_* bits (scents set COND_SMELL). Returns the conditions and the best sound
/// (danger first, then the nearest), as GetBestSound approximates it here.
pub fn listen(
    ear: Vec3,
    me: usize,
    interests: u32,
    sounds: &SoundEnt,
) -> (Conditions, Option<Sound>) {
    let mut conditions = Conditions::default();
    let mut best: Option<(bool, f32, Sound)> = None;
    for s in &sounds.sounds {
        if s.kind & interests == 0 || s.owner == Some(me) {
            continue;
        }
        let distance = s.origin.distance(ear);
        if distance > s.volume {
            continue;
        }
        for (bit, cond) in [
            (sound::COMBAT, COND_HEAR_COMBAT),
            (sound::WORLD, COND_HEAR_WORLD),
            (sound::PLAYER, COND_HEAR_PLAYER),
            (sound::DANGER, COND_HEAR_DANGER),
            (sound::BULLET_IMPACT, COND_HEAR_BULLET_IMPACT),
            (sound::THUMPER, COND_HEAR_THUMPER),
            (sound::BUGBAIT, COND_HEAR_BUGBAIT),
            (sound::PHYSICS_DANGER, COND_HEAR_PHYSICS_DANGER),
            (sound::MOVE_AWAY, COND_HEAR_MOVE_AWAY),
        ] {
            if s.kind & bit != 0 {
                conditions.set(cond);
            }
        }
        if s.kind & sound::ALL_SCENTS != 0 {
            conditions.set(COND_SMELL);
        }
        let danger = s.kind & sound::DANGER != 0;
        if best.is_none_or(|(d, dist, _)| danger && !d || danger == d && distance < dist) {
            best = Some((danger, distance, *s));
        }
    }
    if !conditions.has(COND_HEAR_DANGER) {
        conditions.set(COND_NO_HEAR_DANGER);
    }
    (conditions, best.map(|b| b.2))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Open;
    impl SightWorld for Open {
        fn visible(&self, _: Vec3, to: Vec3, _: &[usize]) -> bool {
            // A wall at x = 500 blocks everything beyond it.
            to.x < 500.
        }
    }
    fn viewer() -> Viewer {
        Viewer {
            entity: 1,
            class: Class::Metropolice,
            eye: Vec3::new(0., 0., 64.),
            facing: Vec3::X,
            field_of_view: 0.5,
            look_distance: LOOK_DISTANCE,
        }
    }
    #[test]
    fn metrocop_sees_the_player_in_the_cone_and_hates_him() {
        let r = Relationships::default();
        let player = |p: Vec3| Seen {
            entity: 0,
            class: Class::Player,
            position: p,
            alive: true,
            player: true,
        };
        let sight = look(&viewer(), &[player(Vec3::new(300., 100., 64.))], &r, &Open);
        assert!(sight.conditions.has(COND_SEE_PLAYER) && sight.conditions.has(COND_SEE_HATE));
        assert_eq!(best_enemy(&sight), Some(0));
        // Behind (outside the 0.5 dot cone), occluded or too far: nothing.
        for p in [
            Vec3::new(-300., 0., 64.),
            Vec3::new(600., 0., 64.),
            Vec3::new(2100., 0., 64.),
        ] {
            assert!(
                look(&viewer(), &[player(p)], &r, &Open).visible.is_empty(),
                "{p}"
            );
        }
    }
    #[test]
    fn sounds_are_heard_within_their_volume_until_they_expire() {
        let mut ent = SoundEnt::default();
        ent.insert(sound::COMBAT, Vec3::new(400., 0., 0.), 500., 0.2, None, 0.);
        ent.insert(sound::DANGER, Vec3::new(900., 0., 0.), 100., 1., None, 0.);
        let (c, best) = listen(Vec3::ZERO, 1, sound::COMBAT | sound::DANGER, &ent);
        assert!(c.has(COND_HEAR_COMBAT) && !c.has(COND_HEAR_DANGER) && c.has(COND_NO_HEAR_DANGER));
        assert_eq!(best.unwrap().kind, sound::COMBAT);
        ent.purge(0.3);
        assert_eq!(ent.sounds.len(), 1);
        let (c, _) = listen(Vec3::ZERO, 1, sound::COMBAT | sound::DANGER, &ent);
        assert!(!c.has(COND_HEAR_COMBAT));
    }
}
