//! Player lifetime and authored input adapter for the portable flashlight rules.
use super::{Context, Flashlight};

#[derive(Default)]
pub struct PlayerFlashlight {
    state: Option<Flashlight>,
    enabled_requests: Vec<bool>,
}

impl PlayerFlashlight {
    pub fn state(&self) -> Option<&Flashlight> {
        self.state.as_ref()
    }

    /// Deferred so the host supplies the current game's drain rules, suit and clock.
    pub fn request_enabled(&mut self, enabled: bool) {
        self.enabled_requests.push(enabled);
    }

    /// Apply authored inputs, player impulses, death and optionally power progression.
    pub fn step(
        &mut self,
        legacy: bool,
        toggles: u32,
        alive: bool,
        dt: Option<f32>,
        context: Context,
    ) -> &mut Flashlight {
        let state = self.state.get_or_insert_with(|| Flashlight::new(legacy));
        for enabled in self.enabled_requests.drain(..) {
            state.set_enabled(enabled, context);
        }
        if alive {
            for _ in 0..toggles {
                state.toggle(context);
            }
        } else if state.on {
            // CBasePlayer::Event_Killed; dead player input cannot reactivate the light.
            state.turn_off(context);
        }
        if let Some(dt) = dt {
            state.update(dt, context);
        }
        state
    }

    /// Campaign changes carry the player while the new level's clock starts afresh.
    pub fn rebase_clock(&mut self, previous: f64, next: f64) {
        if let Some(state) = &mut self.state {
            state.suit_power.all_off_at += next - previous;
        }
    }
}

#[cfg(test)]
mod tests;
