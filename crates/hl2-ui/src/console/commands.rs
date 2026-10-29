//! The console's implemented commands and cvars, described once: name, help text (`name args:
//! description`, the usage line `help` prints and the completion help line marks the argument
//! being typed in), what each argument completes from, the handler that runs it and, for a
//! cvar, its current value. A cvar stored in the settings is a value function and a setter
//! ([`setting`]); one handler queries, stores, saves and echoes all of them. The handlers, value
//! functions and setters live in submodules by area; the registry's types are the toolkit's
//! ([`source_ui::console::command`]).

mod audio;
mod cheats;
mod developer;
mod map;
mod mouse;
mod movie;
mod video;

use super::{Console, Effect};
use source_ui::console::command::{self, Context, Invalid, Setting};

/// The lists of names HL2's command arguments complete from (the host supplies them, see
/// [`super::complete::Sources`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Names {
    /// An installed map name.
    Map,
    /// A targetname of the loaded map's entities.
    EntityTarget,
    /// An input the loaded map's own connections fire.
    EntityInput,
}

impl Context for Console {
    type Effect = Effect;
    type Names = Names;
}

/// HL2's commands and cvars: the toolkit's [`command::Spec`] over the console.
pub type Spec = command::Spec<Console>;
/// What an HL2 command argument completes from.
pub type Arg = command::Arg<Names>;
/// What an HL2 command returns: an effect for the host, or the error line the console prints.
pub type Outcome = command::Outcome<Effect>;
type Handler = command::Handler<Console>;
type Value = command::Value<Console>;
type Set = command::Set<Console>;

const fn command(
    name: &'static str,
    help: &'static str,
    args: &'static [Arg],
    run: Handler,
) -> Spec {
    Spec {
        name,
        help,
        args,
        run,
        value: None,
        setting: None,
    }
}

const fn cvar(
    name: &'static str,
    help: &'static str,
    args: &'static [Arg],
    run: Handler,
    value: Value,
) -> Spec {
    Spec {
        name,
        help,
        args,
        run,
        value: Some(value),
        setting: None,
    }
}

/// A cvar stored in the settings: `value` is what the console prints, `set` stores a typed value.
const fn setting(
    name: &'static str,
    help: &'static str,
    args: &'static [Arg],
    value: Value,
    set: Set,
) -> Spec {
    Spec {
        name,
        help,
        args,
        run: run_setting,
        value: Some(value),
        setting: Some(Setting {
            set,
            prints_as: None,
        }),
    }
}

/// A [`setting`] alias that prints as the cvar it aliases.
const fn setting_alias(
    name: &'static str,
    prints_as: &'static str,
    help: &'static str,
    args: &'static [Arg],
    value: Value,
    set: Set,
) -> Spec {
    Spec {
        name,
        help,
        args,
        run: run_setting,
        value: Some(value),
        setting: Some(Setting {
            set,
            prints_as: Some(prints_as),
        }),
    }
}

/// Runs a [`setting`]: without an argument it prints `"name" = "value"`; with one it stores the
/// value, saves the settings, echoes `name value` and has the host apply them.
fn run_setting(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    let (Some(value), Some(setting)) = (spec.value, spec.setting) else {
        unreachable!("{} is built by `setting`", spec.name);
    };
    let name = setting.prints_as.unwrap_or(spec.name);
    match args {
        [] => {
            let current = value(console).unwrap_or_default();
            console.log(format!("\"{name}\" = \"{current}\""));
            Ok(None)
        }
        [arg] => {
            (setting.set)(console, arg).map_err(|invalid| match invalid {
                Invalid::Usage => spec.usage(),
                Invalid::Message(message) => message.to_owned(),
            })?;
            let _ = console.config.save();
            let stored = value(console).unwrap_or_default();
            console.log(format!("{name} {stored}"));
            Ok(Some(Effect::ApplyConfig(Box::new(console.config.clone()))))
        }
        _ => Err(spec.usage()),
    }
}

const BOOLEAN: &[Arg] = &[Arg::Values(&["0", "1"])];

pub const COMMANDS: &[Spec] = &[
    command(
        "help",
        "help [command]: describe implemented commands",
        &[Arg::Command],
        developer::help,
    ),
    command(
        "find",
        "find <text>: search implemented command names and descriptions",
        &[Arg::Free],
        developer::find,
    ),
    cvar(
        "sv_cheats",
        "sv_cheats [0|1]: query or enable local cheat commands",
        BOOLEAN,
        cheats::sv_cheats,
        cheats::sv_cheats_value,
    ),
    command(
        "impulse",
        "impulse 100: toggle the flashlight; impulse 101: grant the six implemented weapons, ammunition and suit (cheat)",
        &[Arg::Values(&["101"])],
        cheats::impulse,
    ),
    command(
        "noclip",
        "noclip [0|1]: toggle collision-free flight (cheat)",
        BOOLEAN,
        cheats::noclip,
    ),
    command(
        "viewmodel_tweak_mode",
        "viewmodel_tweak_mode <0|1>: toggle the runtime viewmodel tuning overlay (gameplay map only)",
        &[Arg::Values(&["0", "1"])],
        developer::viewmodel_tweak_mode,
    ),
    command(
        "viewmodel_tweak_reset",
        "viewmodel_tweak_reset: zero the selected weapon's framing offsets (viewmodel_tweak_mode 1 only)",
        &[],
        developer::viewmodel_tweak_reset,
    ),
    command(
        "background_debug_mode",
        "background_debug_mode <0|1>: toggle background visual debug mode (background map/blur only)",
        &[Arg::Values(&["0", "1"])],
        super::background_debug::background_debug_mode,
    ),
    command(
        "background_blur_opacity",
        "background_blur_opacity [0..100]: query or set blur background overlay opacity (background_debug_mode 1 only)",
        &[Arg::Free],
        super::background_debug::background_blur_opacity,
    ),
    command(
        "background_map_fov",
        "background_map_fov [degrees]: query or set background map field of view",
        &[Arg::Free],
        super::background_debug::background_map_fov,
    ),
    command(
        "getpos",
        "getpos: print camera position and Source pitch/yaw/roll in degrees; getpos 2 is not implemented",
        &[],
        cheats::getpos,
    ),
    command(
        "setpos",
        "setpos <x> <y> [z]: move the player origin, preserving height if z is omitted (cheat)",
        &[Arg::Free, Arg::Free, Arg::Free],
        cheats::setpos,
    ),
    command(
        "setang",
        "setang <pitch> <yaw> [roll]: set Source view angles; roll must be zero (cheat)",
        &[Arg::Free, Arg::Free, Arg::Free],
        cheats::setang,
    ),
    command(
        "map",
        "map <name>: load an installed map and reset the player inventory",
        &[Arg::Names(Names::Map)],
        map::map,
    ),
    command(
        "playvideo",
        "playvideo <filename> [width height]: play a video file from media/",
        &[Arg::Free, Arg::Free, Arg::Free],
        movie::playvideo,
    ),
    command(
        "playvideo_exitcommand",
        "playvideo_exitcommand <filename> <exit command>: play a video file and fire an exit command on completion",
        &[Arg::Free, Arg::Free],
        movie::playvideo_exitcommand,
    ),
    command(
        "npc_create",
        "npc_create <class> [name]: place an NPC at the crosshair (cheat; spawning not implemented yet)",
        &[Arg::Free, Arg::Free],
        cheats::create,
    ),
    command(
        "npc_create_aimed",
        "npc_create_aimed <class> [name]: like npc_create, facing the aim (cheat)",
        &[Arg::Free, Arg::Free],
        cheats::create,
    ),
    command(
        "npc_create_equipment",
        "npc_create_equipment <weapon>: weapon for the next npc_create",
        &[Arg::Free],
        cheats::npc_create_equipment,
    ),
    command(
        "prop_physics_create",
        "prop_physics_create <model>: physics prop at the crosshair, model relative to models/ (cheat)",
        &[Arg::Free],
        cheats::create,
    ),
    command(
        "ent_create",
        "ent_create <class> [key value ...]: create an entity at the crosshair; NPC classes and prop_physics with a model only (cheat)",
        &[Arg::Free, Arg::Free],
        cheats::create,
    ),
    command(
        "ai_disable",
        "ai_disable: toggle all NPC AI off/on; disabled NPCs idle (cheat, Source command)",
        &[],
        cheats::ai_disable,
    ),
    command(
        "ai_enable",
        "ai_enable [0|1]: switch the NPC AI think loop (ours, default 1; Source uses ai_disable)",
        BOOLEAN,
        cheats::ai_enable,
    ),
    command(
        "clear",
        "clear: clear console output",
        &[],
        developer::clear,
    ),
    command(
        "ent_fire",
        "ent_fire <targetname> [input=Use] [parameter] [delay]: queue entity input; names/globs supported (cheat)",
        &[Arg::Names(Names::EntityTarget), Arg::Names(Names::EntityInput), Arg::Free, Arg::Free],
        cheats::ent_fire,
    ),
    command(
        "echo",
        "echo <text>: print text",
        &[Arg::Free],
        developer::echo,
    ),
    command(
        "toggleconsole",
        "toggleconsole: close or open the console",
        &[],
        developer::toggleconsole,
    ),
    command(
        "quit",
        "quit: exit and save the runtime report",
        &[],
        developer::quit,
    ),
    setting(
        "volume",
        "volume [0..1]: query or set master audio volume",
        &[Arg::Free],
        audio::volume_value,
        audio::set_volume,
    ),
    setting(
        "snd_musicvolume",
        "snd_musicvolume [0..1]: query or set music volume",
        &[Arg::Free],
        audio::snd_musicvolume_value,
        audio::set_snd_musicvolume,
    ),
    setting(
        "snd_mute_losefocus",
        "snd_mute_losefocus [0|1]: query or toggle muting audio when window loses focus",
        BOOLEAN,
        audio::snd_mute_losefocus_value,
        audio::set_snd_mute_losefocus,
    ),
    setting(
        "closecaption",
        "closecaption [0|1]: query or toggle closed captioning",
        BOOLEAN,
        audio::closecaption_value,
        audio::set_closecaption,
    ),
    setting(
        "cc_subtitles",
        "cc_subtitles [0|1]: query or toggle subtitles only (dialogue only, no sound effects)",
        BOOLEAN,
        audio::cc_subtitles_value,
        audio::set_cc_subtitles,
    ),
    setting(
        "snd_surround_speakers",
        "snd_surround_speakers [0|2|4|5|7]: query or set speaker configuration (0=headphones, 2=stereo, 4=surround, 5=5.1, 7=7.1)",
        &[Arg::Values(&["0", "2", "4", "5", "7"])],
        audio::snd_surround_speakers_value,
        audio::set_snd_surround_speakers,
    ),
    setting(
        "snd_pitchquality",
        "snd_pitchquality [0|1]: query or toggle sound pitch quality (0=low, 1=high)",
        BOOLEAN,
        audio::snd_pitchquality_value,
        audio::set_snd_pitchquality,
    ),
    setting(
        "dsp_slow_cpu",
        "dsp_slow_cpu [0|1]: query or toggle reduced DSP CPU quality (1=slow CPU/low, 0=normal/high)",
        BOOLEAN,
        audio::dsp_slow_cpu_value,
        audio::set_dsp_slow_cpu,
    ),
    setting(
        "sensitivity",
        "sensitivity [value]: query or set mouse sensitivity",
        &[Arg::Free],
        mouse::sensitivity_value,
        mouse::set_sensitivity,
    ),
    setting(
        "m_pitch",
        "m_pitch [value]: query or set mouse pitch factor (negative inverts pitch)",
        &[Arg::Free],
        mouse::m_pitch_value,
        mouse::set_m_pitch,
    ),
    setting(
        "m_filter",
        "m_filter [0|1]: query or toggle two-frame mouse movement averaging",
        BOOLEAN,
        mouse::m_filter_value,
        mouse::set_m_filter,
    ),
    setting(
        "m_rawinput",
        "m_rawinput [0|1]: query or toggle raw mouse input",
        BOOLEAN,
        mouse::m_rawinput_value,
        mouse::set_m_rawinput,
    ),
    setting(
        "m_customaccel",
        "m_customaccel [0|1|3]: query or toggle custom mouse acceleration",
        &[Arg::Values(&["0", "1", "3"])],
        mouse::m_customaccel_value,
        mouse::set_m_customaccel,
    ),
    setting(
        "m_customaccel_exponent",
        "m_customaccel_exponent [value]: query or set mouse acceleration exponent",
        &[Arg::Free],
        mouse::m_customaccel_exponent_value,
        mouse::set_m_customaccel_exponent,
    ),
    setting(
        "sk_autoaim_mode",
        "sk_autoaim_mode [0|1|2]: query or set autoaim mode (0=off, 1=basic, 2=enhanced)",
        &[Arg::Values(&["0", "1", "2"])],
        mouse::autoaim_mode_value,
        mouse::set_autoaim_mode,
    ),
    setting(
        "m_autoaim_mode",
        "m_autoaim_mode [0|1|2]: query or set autoaim mode (alias for sk_autoaim_mode)",
        &[Arg::Values(&["0", "1", "2"])],
        mouse::autoaim_mode_value,
        mouse::set_autoaim_mode,
    ),
    setting(
        "sk_autoaim_scale1",
        "sk_autoaim_scale1 [scale]: query or set autoaim deflection scale",
        &[Arg::Free],
        mouse::autoaim_scale_value,
        mouse::set_autoaim_scale,
    ),
    setting(
        "m_autoaim_scale",
        "m_autoaim_scale [scale]: query or set autoaim deflection scale (alias for sk_autoaim_scale1)",
        &[Arg::Free],
        mouse::autoaim_scale_value,
        mouse::set_autoaim_scale,
    ),
    setting(
        "fov",
        "fov [degrees]: query or set camera field of view",
        &[Arg::Free],
        video::fov_value,
        video::set_fov,
    ),
    setting_alias(
        "fov_desired",
        "fov",
        "fov_desired [degrees]: query or set desired field of view (alias for fov)",
        &[Arg::Free],
        video::fov_value,
        video::set_fov,
    ),
    setting(
        "vertical_fov",
        "vertical_fov [degrees]: query or set vertical field of view (0 = automatic)",
        &[Arg::Free],
        video::vertical_fov_value,
        video::set_vertical_fov,
    ),
    setting(
        "viewmodel_fov",
        "viewmodel_fov [degrees]: query or set viewmodel field of view",
        &[Arg::Free],
        video::viewmodel_fov_value,
        video::set_viewmodel_fov,
    ),
    cvar(
        "mat_aaquality",
        "mat_aaquality [quality]: query or set antialiasing quality",
        &[Arg::Values(&["0"])],
        video::mat_aaquality,
        video::mat_aaquality_value,
    ),
    setting(
        "mat_antialias",
        "mat_antialias [0|1|2|4|8]: query or set multisample antialiasing mode",
        &[Arg::Values(&["0", "1", "2", "4", "8"])],
        video::mat_antialias_value,
        video::set_mat_antialias,
    ),
    setting(
        "mat_colorcorrection",
        "mat_colorcorrection [0|1]: query or toggle color correction",
        BOOLEAN,
        video::mat_colorcorrection_value,
        video::set_mat_colorcorrection,
    ),
    setting(
        "mat_disable_d3d9ex",
        "mat_disable_d3d9ex [0|1]: query or toggle disabling Direct3D 9Ex extensions (0=enabled, 1=disabled)",
        BOOLEAN,
        video::mat_disable_d3d9ex_value,
        video::set_mat_disable_d3d9ex,
    ),
    setting(
        "mat_forceaniso",
        "mat_forceaniso [1|2|4|8|16]: query or set anisotropic texture filtering mode",
        &[Arg::Values(&["1", "2", "4", "8", "16"])],
        video::mat_forceaniso_value,
        video::set_mat_forceaniso,
    ),
    setting(
        "mat_hdr_level",
        "mat_hdr_level [0|1|2]: query or set HDR level (0=None, 1=Bloom, 2=Full)",
        &[Arg::Values(&["0", "1", "2"])],
        video::mat_hdr_level_value,
        video::set_mat_hdr_level,
    ),
    setting(
        "mat_motion_blur_enabled",
        "mat_motion_blur_enabled [0|1]: query or toggle motion blur",
        BOOLEAN,
        video::mat_motion_blur_enabled_value,
        video::set_mat_motion_blur_enabled,
    ),
    setting(
        "mat_picmip",
        "mat_picmip [-1|0|1|2]: query or set texture detail (-1=very high, 0=high, 1=medium, 2=low)",
        &[Arg::Values(&["-1", "0", "1", "2"])],
        video::mat_picmip_value,
        video::set_mat_picmip,
    ),
    setting(
        "mat_queue_mode",
        "mat_queue_mode [-1|0|1|2]: query or set multicore rendering (-1=multicore, 0=single threaded)",
        &[Arg::Values(&["-1", "0", "1", "2"])],
        video::mat_queue_mode_value,
        video::set_mat_queue_mode,
    ),
    setting(
        "mat_reducefillrate",
        "mat_reducefillrate [0|1]: query or toggle reduced shader fillrate (0=high, 1=low)",
        BOOLEAN,
        video::mat_reducefillrate_value,
        video::set_mat_reducefillrate,
    ),
    setting(
        "mat_trilinear",
        "mat_trilinear [0|1]: query or toggle trilinear texture filtering",
        BOOLEAN,
        video::mat_trilinear_value,
        video::set_mat_trilinear,
    ),
    setting(
        "mat_vsync",
        "mat_vsync [0|1]: query or toggle vertical sync",
        BOOLEAN,
        video::mat_vsync_value,
        video::set_mat_vsync,
    ),
    setting(
        "r_classic_blood",
        "r_classic_blood [0|1]: query or toggle classic blood effects",
        BOOLEAN,
        video::classic_effects_value,
        video::set_classic_effects,
    ),
    setting(
        "r_classic_fire",
        "r_classic_fire [0|1]: query or toggle classic fire effects",
        BOOLEAN,
        video::classic_effects_value,
        video::set_classic_effects,
    ),
    setting(
        "r_flashlightdepthtexture",
        "r_flashlightdepthtexture [0|1]: query or toggle depth texture flashlight shadows (0=medium, 1=high)",
        BOOLEAN,
        video::r_flashlightdepthtexture_value,
        video::set_r_flashlightdepthtexture,
    ),
    setting(
        "r_rootlod",
        "r_rootlod [0|1|2]: query or set model detail (0=high, 1=medium, 2=low)",
        &[Arg::Values(&["0", "1", "2"])],
        video::r_rootlod_value,
        video::set_r_rootlod,
    ),
    setting(
        "r_shadowrendertotexture",
        "r_shadowrendertotexture [0|1]: query or toggle dynamic render-to-texture shadows (0=low, 1=medium/high)",
        BOOLEAN,
        video::r_shadowrendertotexture_value,
        video::set_r_shadowrendertotexture,
    ),
    setting(
        "r_waterforceexpensive",
        "r_waterforceexpensive [0|1]: query or toggle expensive water reflections (0=simple, 1=world/all)",
        BOOLEAN,
        video::r_waterforceexpensive_value,
        video::set_r_waterforceexpensive,
    ),
    setting(
        "r_waterforcereflectentities",
        "r_waterforcereflectentities [0|1]: query or toggle water entity reflections (0=world only, 1=all)",
        BOOLEAN,
        video::r_waterforcereflectentities_value,
        video::set_r_waterforcereflectentities,
    ),
    command(
        "mat_setvideomode",
        "mat_setvideomode <width> <height> <windowed 0|1>: set the video mode",
        &[Arg::Free, Arg::Free, Arg::Values(&["0", "1"])],
        video::mat_setvideomode,
    ),
    command(
        "mainmenu",
        "mainmenu: show the main menu",
        &[],
        map::mainmenu,
    ),
    cvar(
        "background_map",
        "background_map [name|default]: query or set the persistent main-menu background map",
        &[Arg::Names(Names::Map)],
        map::background_map,
        map::background_map_value,
    ),
    command(
        "map_background",
        "map_background <name>: load an installed map as the main-menu background and persist it",
        &[Arg::Names(Names::Map)],
        map::map_background,
    ),
    cvar(
        "con_filter_enable",
        "con_filter_enable [0|1|2]: filter console output by con_filter_text (1 hides other lines, 2 shows matches brighter)",
        &[Arg::Values(&["0", "1", "2"])],
        developer::con_filter_enable,
        developer::con_filter_enable_value,
    ),
    cvar(
        "con_filter_text",
        "con_filter_text [text]: the text con_filter_enable filters console output by",
        &[Arg::Free],
        developer::con_filter_text,
        developer::con_filter_text_value,
    ),
    cvar(
        "con_timestamp",
        "con_timestamp [0|1]: prefix console output with the time it was printed",
        BOOLEAN,
        developer::con_timestamp,
        developer::con_timestamp_value,
    ),
    setting(
        "skill",
        "skill [1|2|3]: query or set game skill level (1=easy, 2=normal, 3=hard)",
        &[Arg::Values(&["1", "2", "3"])],
        map::skill_value,
        map::set_skill,
    ),
    setting(
        "commentary",
        "commentary [0|1]: query or toggle developer commentary mode",
        BOOLEAN,
        map::commentary_value,
        map::set_commentary,
    ),
    command(
        "migrate_episodic_data",
        "migrate_episodic_data: migrate save data from standalone episodes (placeholder)",
        &[],
        map::migrate_episodic_data,
    ),
];

/// The implemented command or cvar named `name` (case-insensitive).
pub fn find(name: &str) -> Option<&'static Spec> {
    command::find(COMMANDS, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_ui::console::command::usage;

    #[test]
    fn every_spec_is_unique_and_its_help_starts_with_its_name() {
        for (i, spec) in COMMANDS.iter().enumerate() {
            assert!(spec.help.starts_with(spec.name), "{}", spec.name);
            assert!(
                COMMANDS[i + 1..]
                    .iter()
                    .all(|other| other.name != spec.name),
                "duplicate {}",
                spec.name
            );
        }
        assert_eq!(find("ENT_FIRE").map(|s| s.name), Some("ent_fire"));
    }

    /// Every command is one `COMMANDS` entry, so every spec has a handler by construction, and the
    /// handler modules are private: a handler no spec names fails the build as dead code. This
    /// checks that each spec runs its own handler: any spelling of its name dispatches, a cvar's
    /// query prints that cvar with its own value, the NPC console commands pass their own name
    /// on, and arguments no command takes answer with the spec's usage.
    #[test]
    fn every_spec_runs_its_own_handler() {
        let mut console = Console {
            cheats: true,
            ..Console::default()
        };
        console.config.gameplay.background_map = Some("background01".into());
        let unknown = |name: &str| Err(format!("Unknown or unimplemented command: {name}"));
        assert_eq!(console.interpret(&["nope".into()]), unknown("nope"));
        // Commands that take any arguments: the rest of the line or the host's NPC console.
        let free = [
            "echo",
            "npc_create",
            "npc_create_aimed",
            "npc_create_equipment",
            "prop_physics_create",
            "ent_create",
        ];
        let junk = vec!["x".to_owned(); 9];
        for spec in COMMANDS {
            let bare = console.interpret(&[spec.name.to_uppercase()]);
            assert_ne!(bare, unknown(&spec.name.to_uppercase()), "{}", spec.name);
            if let Some(value) = spec.value {
                // `fov_desired` answers as `fov`, like retail's alias.
                let label = if spec.name == "fov_desired" {
                    "fov"
                } else {
                    spec.name
                };
                let value = value(&console).unwrap_or_else(|| panic!("{} unset", spec.name));
                let printed = console
                    .shell
                    .panel
                    .output
                    .back()
                    .map(|line| line.text.as_str());
                assert_eq!(printed, Some(format!("\"{label}\" = \"{value}\"").as_str()));
            }
            let tokens: Vec<String> = std::iter::once(spec.name.to_owned())
                .chain(junk.iter().cloned())
                .collect();
            match console.interpret(&tokens) {
                Ok(Some(Effect::NpcCommand(line))) if free.contains(&spec.name) => {
                    assert!(line.starts_with(&format!("{} x", spec.name)), "{line}");
                }
                Ok(None) if spec.name == "echo" => {}
                Ok(Some(Effect::PlayVideo {
                    exit_command: Some(_),
                    ..
                })) if spec.name == "playvideo_exitcommand" => {}
                other => assert_eq!(other, Err(spec.usage()), "{}", spec.name),
            }
        }
    }

    #[test]
    fn usage_splits_arguments_from_the_description() {
        let (args, description) = usage(find("setpos").unwrap());
        assert_eq!(args, ["<x>", "<y>", "[z]"]);
        assert!(description.starts_with("move the player origin"));
        let (args, _) = usage(find("clear").unwrap());
        assert!(args.is_empty());
    }

    /// A value each cvar accepts, different from its default where it can be: the first listed
    /// value that is not the current one, else a typical value for free arguments.
    fn golden_value(spec: &Spec, current: &str) -> String {
        if let Some(Arg::Values(values)) = spec.args.first() {
            return values
                .iter()
                .find(|v| **v != current)
                .unwrap_or(&values[0])
                .to_string();
        }
        match spec.name {
            "volume" => "0.5",
            "snd_musicvolume" => "0.25",
            "sensitivity" => "2.5",
            "m_pitch" => "-0.022",
            "m_customaccel_exponent" => "1.2",
            "sk_autoaim_scale1" | "m_autoaim_scale" => "1.5",
            "fov" | "fov_desired" => "100",
            "vertical_fov" => "60",
            "viewmodel_fov" => "70",
            "con_filter_text" => "bridge",
            "background_map" => "background02",
            other => panic!("{other}: no golden value"),
        }
        .to_owned()
    }

    /// The settings that differ between two serialized configurations, `path: before -> after`.
    fn config_changes(before: &serde_json::Value, after: &serde_json::Value) -> Vec<String> {
        fn leaves(value: &serde_json::Value, path: String, out: &mut Vec<(String, String)>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, value) in map {
                        let path = if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{path}.{key}")
                        };
                        leaves(value, path, out);
                    }
                }
                other => out.push((path, other.to_string())),
            }
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        leaves(before, String::new(), &mut a);
        leaves(after, String::new(), &mut b);
        let a: std::collections::BTreeMap<_, _> = a.into_iter().collect();
        let b: std::collections::BTreeMap<_, _> = b.into_iter().collect();
        a.keys()
            .chain(b.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|key| a.get(*key) != b.get(*key))
            .map(|key| {
                let side = |m: &std::collections::BTreeMap<String, String>| {
                    m.get(key).cloned().unwrap_or_else(|| "-".into())
                };
                format!("{key}: {} -> {}", side(&a), side(&b))
            })
            .collect()
    }

    /// Every cvar on a fresh console: a query, a valid set, an invalid value, an out-of-range
    /// number and too many arguments, with the lines each logs, its error or effect (for
    /// `ApplyConfig`, the settings it changed) and the value afterwards. The transcript is recorded in `cvars.golden`;
    /// `UPDATE_GOLDEN=1` rewrites it after a reviewed change.
    #[test]
    fn every_cvar_matches_its_golden_transcript() {
        let mut transcript = String::new();
        for spec in COMMANDS.iter().filter(|spec| spec.is_cvar()) {
            let value = spec.value.expect("a cvar");
            let current = value(&Console::default()).unwrap_or_default();
            let mut cases = vec![
                Vec::new(),
                vec![golden_value(spec, &current)],
                vec!["x".to_owned()],
                vec!["9".to_owned()],
                vec!["1".to_owned(), "1".to_owned()],
            ];
            if spec.name == "background_map" {
                // `default` resets the background instead of naming a map.
                cases.insert(2, vec!["default".to_owned()]);
            }
            transcript += &format!("{}\n", spec.name);
            for args in cases {
                let mut console = Console::default();
                let before = serde_json::to_value(&console.config).unwrap();
                let printed = console.shell.panel.output.len();
                let tokens: Vec<String> = std::iter::once(spec.name.to_owned())
                    .chain(args.iter().cloned())
                    .collect();
                let outcome = match console.interpret(&tokens) {
                    Ok(None) => "no effect".to_owned(),
                    Ok(Some(Effect::ApplyConfig(config))) => {
                        assert_eq!(
                            *config, console.config,
                            "{}: applies the stored settings",
                            spec.name
                        );
                        let after = serde_json::to_value(&*config).unwrap();
                        format!(
                            "ApplyConfig [{}]",
                            config_changes(&before, &after).join(", ")
                        )
                    }
                    Ok(Some(effect)) => format!("{effect:?}"),
                    Err(error) => format!("error {error:?}"),
                };
                let call = if args.is_empty() {
                    "query".to_owned()
                } else {
                    args.join(" ")
                };
                transcript += &format!("  {call}: {outcome}\n");
                for line in console.shell.panel.output.iter().skip(printed) {
                    transcript += &format!("    | {}\n", line.text);
                }
                transcript += &format!("    value {:?}\n", value(&console));
            }
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/console/commands/cvars.golden");
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, &transcript).unwrap();
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            transcript == expected,
            "cvar transcript differs from {} (rerun with UPDATE_GOLDEN=1 after review):\n{transcript}",
            path.display()
        );
    }
}
