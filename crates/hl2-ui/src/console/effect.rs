//! What a console command asks of the host, and the map it may name.

#[derive(Debug, PartialEq)]
pub enum Effect {
    /// An opaque contribution event, interpreted by an application-installed handler.
    Extension(serde_json::Value),
    Loadout,
    /// ai_disable (CC_AI_Disable): toggle the AI, Source's messages.
    AiDisable,
    /// notarget (CC_Notarget_f): toggle whether NPCs sense the player, Source's messages.
    Notarget,
    /// npc_create / npc_create_aimed / npc_create_equipment / prop_physics_create /
    /// ent_create, the whole line.
    NpcCommand(String),
    Noclip(Option<bool>),
    Getpos,
    Setpos {
        x: f32,
        y: f32,
        z: Option<f32>,
    },
    Setang(glam::Vec3),
    Map(String),
    /// Start an installed chapter in its selected game.
    ChapterMap(GameMap),
    Fire {
        target: String,
        input: String,
        parameter: String,
        delay: f64,
    },
    Quit,
    ApplyConfig(Box<crate::config::Config>),
    /// Load a game's main-menu background map and show the main menu over it.
    MainMenuMap(GameMap),
    /// `impulse 100`: the flashlight toggles (CBasePlayer::ImpulseCommands).
    Flashlight,
    /// `screenshot`: save the next game frame (the host picks the file).
    Screenshot,
    /// Play a UI sound cue (e.g. button hover or release activation).
    PlaySound(String),
    /// Play a full-screen video file (Bink `.bik`) with optional exit command upon completion or skip.
    PlayVideo {
        filename: String,
        exit_command: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
    },
}

/// A map of one installed game, named by the game's installation folder (`hl2`, `episodic`,
/// `ep2`, `lostcoast`): Half-Life 2 and Lost Coast both ship a `background01`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameMap {
    pub game: String,
    pub map: String,
}

impl GameMap {
    pub fn new(game: impl Into<String>, map: impl Into<String>) -> Self {
        Self {
            game: game.into(),
            map: map.into(),
        }
    }
}

/// getpos output (Source SDK `game/client/view.cpp`): the view (eye) origin and the view
/// angles in Source's convention (positive pitch looks down), as `setpos`/`setang` take them.
/// Pasting it back lifts the player by the eye height, since `setpos` places the feet, as in
/// retail. The developer overlay prints the same line so its numbers match getpos.
pub fn getpos_text(eye: [f32; 3], pitch: f32, yaw: f32) -> String {
    let [x, y, z] = eye;
    format!("setpos {x:.6} {y:.6} {z:.6}; setang {pitch:.6} {yaw:.6} 0")
}

#[cfg(test)]
mod tests {
    #[test]
    fn getpos_text_uses_source_order_and_six_decimals() {
        assert_eq!(
            super::getpos_text([-715.2727, 2911., -761.5138], -13.393998, 1083.4136),
            "setpos -715.272705 2911.000000 -761.513794; setang -13.393998 1083.413574 0"
        );
    }
}
