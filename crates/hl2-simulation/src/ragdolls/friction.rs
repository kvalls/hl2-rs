//! C_ClientRagdoll's animated joint friction (HandleAnimatedFriction).

/// m_iFrictionAnimState (RAGDOLL_FRICTION_NONE, _IN, _HOLD, _OUT, _OFF).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum FrictionState {
    #[default]
    None,
    In,
    Hold,
    Out,
    Off,
}

/// C_ClientRagdoll's animated joint friction: every client think while it runs sets
/// SetAngularMotor(0, m_iCurrentFriction) on all ragdoll constraints.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AnimatedFriction {
    state: FrictionState,
    /// m_flFrictionTime and m_flFrictionModTime.
    until: f64,
    span: f32,
    /// m_iCurrentFriction (an int, 0 until the first think).
    current: i32,
}

impl AnimatedFriction {
    /// One HandleAnimatedFriction: the friction to apply to the joints, None once off.
    /// `params` is the model's animatedfriction (min, max, time in, time out, hold).
    pub(super) fn think(&mut self, params: Option<[f32; 5]>, now: f64) -> Option<i32> {
        if self.state == FrictionState::Off {
            return None;
        }
        let [min, max, time_in, time_out, hold] = params.unwrap_or_default();
        // iMin/iMaxAnimatedFriction are atoi'd.
        let (min, max) = (min.trunc(), max.trunc());
        let remap = |value: f32, a: f32, b: f32| -> i32 {
            // RemapValClamped, truncated into m_iCurrentFriction.
            if a == b {
                return if value >= b { max as i32 } else { min as i32 };
            }
            (min + (max - min) * ((value - a) / (b - a)).clamp(0., 1.)) as i32
        };
        match self.state {
            FrictionState::None => {
                if min != 0. || max != 0. {
                    self.state = FrictionState::In;
                    self.span = time_in;
                    self.until = now + f64::from(time_in);
                    self.current = min as i32;
                } else {
                    self.state = FrictionState::Off;
                }
            }
            FrictionState::In => {
                let left = (self.until - now) as f32;
                self.current = remap(left, self.span, 0.);
                if left <= 0. {
                    self.span = hold;
                    self.until = now + f64::from(hold);
                    self.state = FrictionState::Hold;
                }
            }
            FrictionState::Hold => {
                if self.until < now {
                    self.span = time_out;
                    self.until = now + f64::from(time_out);
                    self.state = FrictionState::Out;
                }
            }
            FrictionState::Out => {
                let left = (self.until - now) as f32;
                self.current = remap(left, 0., self.span);
                if left <= 0. {
                    self.state = FrictionState::Off;
                }
            }
            FrictionState::Off => {}
        }
        Some(self.current)
    }
}
