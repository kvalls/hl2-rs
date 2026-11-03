//! What the runtime hands every zombie class: its animation events
//! (CNPC_BaseZombie::HandleAnimEvent and the classes' overrides) and its sound patches
//! each think, on pain and at death.
use super::super::metropolice::CopRequest;
use super::super::schedule::Context;
use super::super::state::NpcState;
use super::{claw_punch, event_sound, fast, swat_velocity, Kind, Zombie};
use super::{
    AE_POISON_PICKUP_CRAB, AE_POISON_THROW_CRAB, AE_POISON_THROW_SOUND, AE_POISON_THROW_WARN_SOUND,
    AE_SWATITEM, SWAT_DELAY,
};
use crate::sounds::PatchOp;
use glam::Vec3;

impl Zombie {
    /// One animation event: the poison zombie's crab throw, a swat, the fast zombie's leap
    /// and gallop, claws and class sounds. `center` is the zombie's WorldSpaceCenter and
    /// `enemy_center` its enemy's; `rolls` are two draws in [0, 1).
    pub fn anim_event(
        &mut self,
        name: &str,
        ctx: &Context,
        center: Vec3,
        enemy_center: Option<Vec3>,
        rolls: (f32, f32),
    ) {
        let is = |event: &str| name.eq_ignore_ascii_case(event);
        if is(AE_POISON_PICKUP_CRAB) {
            self.pick_up_crab();
        } else if is(AE_POISON_THROW_WARN_SOUND) || is(AE_POISON_THROW_SOUND) {
            let cue = if is(AE_POISON_THROW_SOUND) {
                "NPC_PoisonZombie.Throw"
            } else {
                "NPC_PoisonZombie.ThrowWarn"
            };
            self.requests.push(CopRequest::Sound(cue));
            self.breathe_off_short();
        } else if is(AE_POISON_THROW_CRAB) {
            if self.holding_crab {
                self.throw_held_crab(ctx.now, ctx.enemy_eye);
            }
        } else if is(AE_SWATITEM) {
            // Send the object toward the enemy's center, forget it and scan again no
            // sooner than ZOMBIE_SWAT_DELAY.
            if let (Some((entity, object)), Some(target)) = (self.physics_ent.take(), enemy_center)
            {
                let (velocity, angular) = swat_velocity(object, target, rolls);
                self.requests.push(CopRequest::Swat {
                    entity,
                    velocity,
                    angular,
                });
                self.next_swat_scan = ctx.now + SWAT_DELAY;
            }
        } else if self.kind == Kind::Fast && is(fast::AE_LEAP) {
            self.fast_leap(ctx.now, ctx.origin, enemy_center);
        } else if self.kind == Kind::Fast && is("AE_FASTZOMBIE_GALLOP_LEFT") {
            self.requests
                .push(CopRequest::Sound("NPC_FastZombie.GallopLeft"));
        } else if self.kind == Kind::Fast && is("AE_FASTZOMBIE_GALLOP_RIGHT") {
            self.requests
                .push(CopRequest::Sound("NPC_FastZombie.GallopRight"));
        } else if let Some((punch, impulse, damage)) = self.claw(name, ctx.yaw, rolls) {
            let yaw = ctx.yaw.to_radians();
            self.requests.push(CopRequest::Claw {
                start: center,
                forward: Vec3::new(yaw.cos(), yaw.sin(), 0.),
                punch,
                impulse,
                reach: self.kind.claw_reach(),
                bite: is("AE_ZOMBIE_POISON_SPIT"),
                damage,
            });
        } else if let Some(sound) = event_sound(name) {
            let cue = self.kind.cue(sound);
            if !cue.is_empty() {
                self.requests.push(CopRequest::Sound(cue));
            }
        }
    }
    /// A claw event's view punch, velocity punch and damage (None: the class default).
    fn claw(&self, name: &str, yaw: f32, rolls: (f32, f32)) -> Option<(Vec3, Vec3, Option<f32>)> {
        if self.kind == Kind::Fast {
            fast::claw_punch(name, yaw).map(|(p, i)| (p, i, Some(fast::CLAW_DAMAGE)))
        } else {
            claw_punch(name, yaw, rolls).map(|(p, i)| (p, i, None))
        }
    }
    /// MaxYawSpeed of the class (CFastZombie's own table; torsos the base zombie's).
    pub fn yaw_speed(&self, activity: &str) -> f32 {
        if self.kind == Kind::Fast && !self.torso {
            fast::max_yaw_speed(activity)
        } else {
            super::max_yaw_speed(activity, self.torso)
        }
    }
    /// The class's sounds each think: the fast zombie's patches and idle sounds, else
    /// CZombie's idle clock.
    pub fn think_sounds(&mut self, now: f64, state: NpcState, enemy_distance: Option<f32>) {
        if self.kind == Kind::Fast {
            self.fast_sounds(now, state, enemy_distance);
        } else {
            self.moan(now, state == NpcState::Combat);
        }
    }
    /// PainSound's patch side: the poison zombie's breath drops out; the fast zombie's
    /// gurgle swells.
    pub fn pain_patches(&mut self) {
        match self.kind {
            Kind::Poison => self.breathe_off_short(),
            Kind::Fast => self.fast_pain(),
            Kind::Classic => {}
        }
    }
    /// StopLoopingSounds (death): the breathing, moan and gurgle patches go.
    pub fn stop_looping_sounds(&mut self) {
        self.stop_breathing();
        self.stop_moan();
        if self.kind == Kind::Fast && std::mem::take(&mut self.fast.sound_init) {
            self.requests.push(CopRequest::Patch {
                cue: fast::GURGLE,
                op: PatchOp::Destroy,
            });
        }
    }
}

/// GetHeadcrabModel / GetHeadcrabClassname: the crab a zombie class releases.
pub fn headcrab_of(class: &str) -> (&'static str, &'static str) {
    match Kind::for_class(class) {
        Some(Kind::Poison) => ("models/headcrabblack.mdl", "npc_headcrab_poison"),
        Some(Kind::Fast) => ("models/headcrab.mdl", "npc_headcrab_fast"),
        _ => ("models/headcrabclassic.mdl", "npc_headcrab"),
    }
}
