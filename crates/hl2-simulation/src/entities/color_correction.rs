//! The color correction entities follow the player: one update per tick.

use super::Scene;
use modkit_core::World;

impl Scene {
    /// Server thinks and client weights of `color_correction` and `color_correction_volume` for
    /// the player's feet of this tick. A map without them does nothing.
    pub(super) fn update_color_correction(&mut self, world: &World) {
        if self.color_correction.is_empty() {
            return;
        }
        let feet = self.player_feet;
        // Taken out so the touch test can borrow the scene.
        let mut color_correction = std::mem::take(&mut self.color_correction);
        color_correction.update(self.time, feet, |id| self.player_inside(world, id, feet));
        self.color_correction = color_correction;
    }
}
