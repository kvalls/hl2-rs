//! When the world and the player advance: gameplay, a background level behind the menu, and a
//! gameplay map's spawn hold.
use super::Simulation;

impl Simulation {
    pub fn paused(&self) -> bool {
        self.paused || self.loading
    }
    /// Whether the world (entities, sounds, effects, the player) advances: in gameplay, behind
    /// the menu of a background level, and through a newly loaded map's spawn hold even while
    /// paused (retail's server runs those first ticks while the map connects, so a map opened
    /// under the console or menu shows its first frame there instead of black); never while a
    /// map loads.
    pub fn world_running(&self) -> bool {
        !self.loading
            && !(self.background_level && self.background_audio_pending)
            && (!self.paused || self.background_level || self.spawn_hold)
    }
    /// The player takes no input: a menu background level, a paused spawn hold, or a
    /// point_viewcontrol with `SF_CAMERA_PLAYER_TAKECONTROL`.
    pub(super) fn frozen(&self) -> bool {
        self.background_level
            || (self.paused && self.spawn_hold)
            || self.view_control.is_some_and(|v| v.freeze_player)
    }
    pub fn loading(&self) -> bool {
        self.loading
    }
}
