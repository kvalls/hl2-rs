//! What a console command asks of the host, and the map it may name.

#[derive(Debug, PartialEq)]
pub enum Effect {
    Loadout,
    /// ai_enable: None toggles.
    AiEnable(Option<bool>),
    /// ai_disable (CC_AI_Disable): toggle the AI, Source's messages.
    AiDisable,
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
    /// viewmodel_tweak_mode: the host enables or disables the tuning overlay, or refuses with its
    /// reason.
    ViewmodelTweakMode(bool),
    /// `impulse 100`: the flashlight toggles (CBasePlayer::ImpulseCommands).
    Flashlight,
    /// viewmodel_tweak_reset: the host zeroes the selected weapon's framing, if the mode is on.
    ViewmodelTweakReset,
    /// Play a UI sound cue (e.g. button hover or release activation).
    PlaySound(String),
    /// Play a full-screen video file (Bink `.bik`) with optional exit command upon completion or skip.
    PlayVideo {
        filename: String,
        exit_command: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
    },
    /// background_debug_mode: toggle the background visual layout debug mode.
    BackgroundDebugMode(bool),
    /// background_blur_opacity: set opacity of background blur overlay (0..=100).
    BackgroundBlurOpacity(u8),
    /// background_map_fov: query (None) or set (Some) background map FOV.
    BackgroundMapFov(Option<f32>),
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
