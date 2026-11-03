//! CNPC_PoisonZombie (npc_PoisonZombie.cpp): the crab throw and leap conditions, the nest,
//! its combat schedule choice and its breathing loops.
use super::super::conditions::{
    Cond, COND_CAN_MELEE_ATTACK1, COND_CAN_RANGE_ATTACK1, COND_CAN_RANGE_ATTACK2,
    COND_ENEMY_OCCLUDED, COND_NOT_FACING_ATTACK, COND_SEE_ENEMY, COND_TOO_CLOSE_TO_ATTACK,
    COND_TOO_FAR_TO_ATTACK,
};
use super::super::metropolice::CopRequest;
use super::super::schedule::{AiNpc, Context};
use super::Zombie;
use crate::sounds::PatchOp;
use glam::Vec3;

/// CNPC_PoisonZombie (npc_PoisonZombie.cpp): the throw (range attack 2) and the
/// headcrab leap off its back (range attack 1) ranges and cones, the first throw after a
/// new enemy and the time between throws (RandomInt seconds).
pub const THROW_RANGE: (f32, f32) = (250., 800.);
pub const THROW_CONE: f32 = 0.6;
pub const HC_LEAP_RANGE: (f32, f32) = (12., 256.);
pub const HC_LEAP_CONE: f32 = 0.6;
pub const THROW_FIRST_DELAY: (i32, i32) = (1, 2);
pub const THROW_DELAY: (i32, i32) = (4, 10);
/// ZOMBIE_BODYGROUP_NEST_BASE (three nest crabs from here) and ZOMBIE_BODYGROUP_THROW.
pub const BODYGROUP_NEST_BASE: usize = 2;
pub const BODYGROUP_THROW: usize = 5;
pub const AE_POISON_PICKUP_CRAB: &str = "AE_ZOMBIE_POISON_PICKUP_CRAB";
pub const AE_POISON_THROW_WARN_SOUND: &str = "AE_ZOMBIE_POISON_THROW_WARN_SOUND";
pub const AE_POISON_THROW_SOUND: &str = "AE_ZOMBIE_POISON_THROW_SOUND";
pub const AE_POISON_THROW_CRAB: &str = "AE_ZOMBIE_POISON_THROW_CRAB";
/// CNPC_PoisonZombie::RangeAttack1Conditions (`leap`: the crab leap off its back) or
/// RangeAttack2Conditions (the throw): nothing without crabs or before the next throw
/// time, then the range, then the cone.
pub fn poison_range_condition(
    leap: bool,
    crabs: usize,
    ready: bool,
    distance: f32,
    dot: f32,
) -> Option<Cond> {
    let ((min, max), cone, can) = if leap {
        (HC_LEAP_RANGE, HC_LEAP_CONE, COND_CAN_RANGE_ATTACK1)
    } else {
        (THROW_RANGE, THROW_CONE, COND_CAN_RANGE_ATTACK2)
    };
    if crabs == 0 || !ready {
        None
    } else if distance < min {
        Some(COND_TOO_CLOSE_TO_ATTACK)
    } else if distance > max {
        Some(COND_TOO_FAR_TO_ATTACK)
    } else if dot < cone {
        Some(COND_NOT_FACING_ATTACK)
    } else {
        Some(can)
    }
}
/// CNPC_PoisonZombie's breathing loops: m_pFastBreathSound and m_pSlowBreathSound.
pub const POISON_FAST_BREATH: &str = "NPC_PoisonZombie.FastBreath";
pub const POISON_SLOW_BREATH: &str = "NPC_PoisonZombie.Moan1";
/// BREATH_VOL_MAX and ZOMBIE_ENEMY_BREATHE_DIST.
pub const BREATH_VOL_MAX: f32 = 0.6;
pub const ENEMY_BREATHE_DIST: f32 = 300.;
impl Zombie {
    /// m_nCrabCount: the nest crabs left.
    pub fn crab_count(&self) -> usize {
        self.nest.map_or(0, |n| n.iter().filter(|c| **c).count())
    }
    /// RandomThrowCrab: a random nest crab that is still there.
    fn random_throw_crab(&mut self) -> usize {
        let nest = self.nest.unwrap_or_default();
        if !nest.contains(&true) {
            return 0;
        }
        loop {
            let crab = ((self.random() * 3.) as usize).min(2);
            if nest[crab] {
                return crab;
            }
        }
    }
    /// The two range conditions of a think; a CAN condition picks the crab to throw.
    pub fn poison_range_conditions(
        &mut self,
        now: f64,
        distance: f32,
        dot: f32,
    ) -> [Option<Cond>; 2] {
        let ready = now >= self.next_crab_throw;
        let crabs = self.crab_count();
        let conditions =
            [true, false].map(|leap| poison_range_condition(leap, crabs, ready, distance, dot));
        if conditions
            .iter()
            .any(|c| matches!(*c, Some(COND_CAN_RANGE_ATTACK1 | COND_CAN_RANGE_ATTACK2)))
        {
            self.throw_crab = self.random_throw_crab();
        }
        conditions
    }
    /// CNPC_PoisonZombie::PrescheduleThink's first throw 1-2 s after a new enemy, and
    /// OnTakeDamage_Alive's throw at once after a hit of 20% of max health or more.
    pub fn poison_think(&mut self, now: f64, new_enemy: bool, damage_fraction: f32) {
        if new_enemy {
            let (min, max) = THROW_FIRST_DELAY;
            self.next_crab_throw = now + self.random_int(min, max);
        }
        if damage_fraction >= 0.2 {
            self.next_crab_throw = now;
        }
    }
    /// AE_ZOMBIE_POISON_PICKUP_CRAB: EnableCrab(m_nThrowCrab, false) and the crab in hand.
    pub fn pick_up_crab(&mut self) {
        let crab = self.throw_crab.min(2);
        if let Some(nest) = &mut self.nest {
            nest[crab] = false;
        }
        self.holding_crab = true;
        self.requests.push(CopRequest::SetBodygroup {
            group: BODYGROUP_NEST_BASE + crab,
            value: 0,
        });
        self.requests.push(CopRequest::SetBodygroup {
            group: BODYGROUP_THROW,
            value: 1,
        });
    }
    /// AE_ZOMBIE_POISON_THROW_CRAB: the hand empties, a black crab is thrown at the
    /// enemy's eyes and the next throw waits RandomInt(4, 10) s.
    pub fn throw_held_crab(&mut self, now: f64, target: Option<Vec3>) {
        self.holding_crab = false;
        self.requests.push(CopRequest::SetBodygroup {
            group: BODYGROUP_THROW,
            value: 0,
        });
        self.requests.push(CopRequest::ThrowCrab { target });
        self.crabs_thrown += 1;
        let (min, max) = THROW_DELAY;
        self.next_crab_throw = now + self.random_int(min, max);
    }
    /// RandomInt(min, max) seconds.
    fn random_int(&mut self, min: i32, max: i32) -> f64 {
        let span = (max - min + 1) as f32;
        f64::from(min + ((self.random() * span) as i32).min(max - min))
    }
    /// CAI_BaseNPC::SelectCombatSchedule as CNPC_PoisonZombie gets it (innate melee and,
    /// while it has crabs, innate range attacks): back away when a throw is too close,
    /// leap, throw, claw, face, else shamble on.
    pub(super) fn poison_combat_schedule(&self, npc: &AiNpc, ctx: &Context) -> &'static str {
        let c = &npc.conditions;
        let range = self.crab_count() > 0 || self.holding_crab;
        if npc.wakes_angry(ctx.now) {
            "SCHED_WAKE_ANGRY"
        } else if !c.has(COND_SEE_ENEMY) {
            if !c.has(COND_ENEMY_OCCLUDED) {
                "SCHED_COMBAT_FACE"
            } else if range {
                "SCHED_ESTABLISH_LINE_OF_FIRE"
            } else {
                "SCHED_CHASE_ENEMY"
            }
        } else if c.has(COND_TOO_CLOSE_TO_ATTACK) {
            "SCHED_BACK_AWAY_FROM_ENEMY"
        } else if c.has(COND_CAN_RANGE_ATTACK1) {
            "SCHED_RANGE_ATTACK1"
        } else if c.has(COND_CAN_RANGE_ATTACK2) {
            "SCHED_RANGE_ATTACK2"
        } else if c.has(COND_CAN_MELEE_ATTACK1) {
            "SCHED_MELEE_ATTACK1"
        } else if c.has(COND_NOT_FACING_ATTACK) {
            "SCHED_COMBAT_FACE"
        } else if c.has(COND_TOO_FAR_TO_ATTACK) && range {
            "SCHED_MOVE_TO_WEAPON_RANGE"
        } else {
            "SCHED_CHASE_ENEMY"
        }
    }
    /// CNPC_PoisonZombie's breathing (Spawn and PrescheduleThink): FastBreath (ATTN_IDLE)
    /// silent and Moan1 at BREATH_VOL_MAX; an enemy within 300 units ramps the fast
    /// breath up over U(1, 2) s (pitch 100-120 over U(1, 2) s) and the slow one out. The
    /// SDK's far branch also ramps the fast breath to BREATH_VOL_MAX (over U(2, 4) s), so
    /// once near it keeps breathing fast; ported as written.
    pub fn breathe(&mut self, enemy_distance: Option<f32>) {
        if !self.breathing {
            self.breathing = true;
            self.requests.push(CopRequest::Patch {
                cue: POISON_FAST_BREATH,
                op: PatchOp::Play {
                    volume: 0.,
                    pitch: 100.,
                },
            });
            self.requests.push(CopRequest::Patch {
                cue: POISON_SLOW_BREATH,
                op: PatchOp::Play {
                    volume: BREATH_VOL_MAX,
                    pitch: 100.,
                },
            });
        }
        let near = enemy_distance.is_some_and(|d| d < ENEMY_BREATHE_DIST);
        if near == self.near_enemy {
            return;
        }
        self.near_enemy = near;
        let seconds = if near {
            1. + self.random()
        } else {
            2. + 2. * self.random()
        };
        self.requests.push(CopRequest::Patch {
            cue: POISON_FAST_BREATH,
            op: PatchOp::Volume {
                volume: BREATH_VOL_MAX,
                seconds,
            },
        });
        if near {
            let pitch = 100. + (self.random() * 21.).floor().min(20.);
            let pitch_seconds = 1. + self.random();
            self.requests.push(CopRequest::Patch {
                cue: POISON_FAST_BREATH,
                op: PatchOp::Pitch {
                    pitch,
                    seconds: pitch_seconds,
                },
            });
        }
        self.requests.push(CopRequest::Patch {
            cue: POISON_SLOW_BREATH,
            op: PatchOp::Volume {
                volume: 0.,
                seconds,
            },
        });
    }
    /// CNPC_PoisonZombie::BreatheOffShort before a vocal sound: the current breath drops
    /// out for two seconds (envPoisonZombieBreatheVolumeOffShort).
    pub fn breathe_off_short(&mut self) {
        if !self.breathing {
            return;
        }
        let cue = if self.near_enemy {
            POISON_FAST_BREATH
        } else {
            POISON_SLOW_BREATH
        };
        self.requests.push(CopRequest::Patch {
            cue,
            op: PatchOp::Envelope {
                points: [(0., 0.1), (0., 2.), (BREATH_VOL_MAX, 1.)],
            },
        });
    }
    /// StopLoopingSounds (death): both breathing patches go.
    pub fn stop_breathing(&mut self) {
        if std::mem::take(&mut self.breathing) {
            for cue in [POISON_FAST_BREATH, POISON_SLOW_BREATH] {
                self.requests.push(CopRequest::Patch {
                    cue,
                    op: PatchOp::Destroy,
                });
            }
        }
    }
}
