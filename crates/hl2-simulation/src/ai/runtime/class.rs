//! What each AI class answers for itself: its goals, requests, senses and schedule hooks.

use super::super::{bird, scanner};
use super::{
    assault, headcrab, zombie, AiClass, Behavior, CopRequest, Humanoid, Metropolice,
    METROPOLICE_FOV, METROPOLICE_SOUNDS,
};
use glam::Vec3;

impl AiClass {
    pub fn cop(&self) -> Option<&Metropolice> {
        match self {
            Self::Metropolice(cop) => Some(cop),
            _ => None,
        }
    }
    pub fn cop_mut(&mut self) -> Option<&mut Metropolice> {
        match self {
            Self::Metropolice(cop) => Some(cop),
            _ => None,
        }
    }
    pub fn humanoid(&self) -> Option<&Humanoid> {
        match self {
            Self::Humanoid(h) => Some(h),
            _ => None,
        }
    }
    /// The hosted CAI_AssaultBehavior.
    pub fn assault(&self) -> Option<&assault::Assault> {
        match self {
            Self::Metropolice(cop) => cop.assault.as_ref(),
            Self::Humanoid(h) => h.assault.as_ref(),
            Self::Zombie(z) => z.assault.as_ref(),
            Self::Headcrab(h) => h.assault.as_ref(),
            Self::Bird(b) => b.assault.as_ref(),
            Self::Scanner(s) => s.assault.as_ref(),
        }
    }
    pub fn assault_mut(&mut self) -> &mut Option<assault::Assault> {
        match self {
            Self::Metropolice(cop) => &mut cop.assault,
            Self::Humanoid(h) => &mut h.assault,
            Self::Zombie(z) => &mut z.assault,
            Self::Headcrab(h) => &mut h.assault,
            Self::Bird(b) => &mut b.assault,
            Self::Scanner(s) => &mut s.assault,
        }
    }
    /// The aiscripted_schedule goal (position, run).
    pub fn script_goal(&self) -> Option<(Vec3, bool)> {
        match self {
            Self::Metropolice(cop) => cop.script_goal,
            Self::Humanoid(h) => h.script_goal,
            Self::Zombie(z) => z.script_goal,
            Self::Headcrab(h) => h.script_goal,
            Self::Bird(b) => b.script_goal,
            Self::Scanner(s) => s.script_goal,
        }
    }
    pub(super) fn script_goal_mut(&mut self) -> &mut Option<(Vec3, bool)> {
        match self {
            Self::Metropolice(cop) => &mut cop.script_goal,
            Self::Humanoid(h) => &mut h.script_goal,
            Self::Zombie(z) => &mut z.script_goal,
            Self::Headcrab(h) => &mut h.script_goal,
            Self::Bird(b) => &mut b.script_goal,
            Self::Scanner(s) => &mut s.script_goal,
        }
    }
    pub(super) fn requests_mut(&mut self) -> &mut Vec<CopRequest> {
        match self {
            Self::Metropolice(cop) => &mut cop.requests,
            Self::Humanoid(h) => &mut h.requests,
            Self::Zombie(z) => &mut z.requests,
            Self::Headcrab(h) => &mut h.requests,
            Self::Bird(b) => &mut b.requests,
            Self::Scanner(s) => &mut s.requests,
        }
    }
    pub(super) fn behavior_mut(&mut self) -> &mut dyn Behavior {
        match self {
            Self::Metropolice(cop) => cop.as_mut(),
            Self::Humanoid(h) => h.as_mut(),
            Self::Zombie(z) => z.as_mut(),
            Self::Headcrab(h) => h.as_mut(),
            Self::Bird(b) => b.as_mut(),
            Self::Scanner(s) => s.as_mut(),
        }
    }
    /// m_flFieldOfView.
    pub(super) fn field_of_view(&self) -> f32 {
        match self {
            Self::Metropolice(_) => METROPOLICE_FOV,
            Self::Humanoid(h) => h.kind.field_of_view(),
            Self::Zombie(_) => zombie::FIELD_OF_VIEW,
            Self::Headcrab(_) => headcrab::FIELD_OF_VIEW,
            Self::Bird(_) => bird::FIELD_OF_VIEW,
            Self::Scanner(_) => scanner::FIELD_OF_VIEW,
        }
    }
    /// GetSoundInterests.
    pub(super) fn sound_interests(&self) -> u32 {
        match self {
            Self::Metropolice(_) => METROPOLICE_SOUNDS,
            Self::Humanoid(h) => h.kind.sound_interests(),
            Self::Bird(b) => b.sound_interests(),
            Self::Scanner(s) => s.sound_interests(),
            // CAI_BaseNPC::GetSoundInterests (SOUND_PLAYER_VEHICLE has no source here).
            Self::Zombie(_) | Self::Headcrab(_) => {
                use super::senses::sound;
                sound::WORLD | sound::COMBAT | sound::PLAYER | sound::BULLET_IMPACT
            }
        }
    }
}
