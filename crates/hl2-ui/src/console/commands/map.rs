//! Maps and the campaign: loading maps, the main menu and its background map, skill and save
//! data.

use super::{Outcome, Spec};
use crate::console::{Console, Effect, Mode};
use source_ui::console::command::{flag, parse, Invalid};

pub(super) fn map(console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
    if args.len() != 1 {
        return Err(spec.usage());
    }
    let map = args[0].strip_suffix(".bsp").unwrap_or(&args[0]);
    if map.is_empty()
        || map.len() > 128
        || !map.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("map: use a simple installed map name".into());
    }
    console.start_loading(map);
    Ok(Some(Effect::Map(map.to_owned())))
}

pub(super) fn mainmenu(console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    console.set_mode(Mode::MainMenu);
    Ok(None)
}

pub(super) fn map_background(console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
    if args.len() != 1 {
        return Err(spec.usage());
    }
    let map = args[0].strip_suffix(".bsp").unwrap_or(&args[0]);
    if map.is_empty()
        || map.len() > 128
        || !map.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("map_background: use a simple installed map name".into());
    }
    let current = console.loaded_map.as_ref().map(|m| m.game.as_str());
    let background = console.game_map(map, current);
    console.set_background(Some(background.clone()));
    let _ = console.config.save();
    if console.is_loading() && !console.background_level {
        console.log(
            "Gameplay map load in progress; background map persisted without interrupting load.",
        );
        return Ok(None);
    }
    console.start_background_loading(&background.game, &background.map);
    Ok(Some(Effect::MainMenuMap(background)))
}

pub(super) fn skill_value(console: &Console) -> Option<String> {
    Some(console.config.gameplay.skill.to_string())
}

pub(super) fn set_skill(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    let skill = parse::<u8>(arg)?;
    if !(1..=3).contains(&skill) {
        return Err(Invalid::Usage);
    }
    console.config.gameplay.skill = skill;
    Ok(())
}

pub(super) fn commentary_value(console: &Console) -> Option<String> {
    Some(
        if console.config.gameplay.commentary {
            "1"
        } else {
            "0"
        }
        .into(),
    )
}

pub(super) fn set_commentary(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.gameplay.commentary = flag(arg)?;
    Ok(())
}

pub(super) fn migrate_episodic_data(
    console: &mut Console,
    spec: &Spec,
    args: &[String],
) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    console.log("migrate_episodic_data: save data migration is not implemented in this build");
    Ok(None)
}

/// `gamemenucommand <command>`: a game menu command as a menu entry runs it.
pub(super) fn gamemenucommand(console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
    if args.is_empty() {
        return Err(spec.usage());
    }
    let effects = console.game_menu_command(&args.join(" "));
    console.deferred.extend(effects);
    Ok(None)
}

/// `gameui_activate`: shows the pause menu over a game (the main menu is already shown).
pub(super) fn gameui_activate(console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    if console.mode() == Mode::Gameplay && !console.background_level() {
        console.set_mode(Mode::Pause);
    }
    Ok(None)
}

/// `gameui_hide`: back to the game, closing the menus (not over a menu background level).
pub(super) fn gameui_hide(console: &mut Console, spec: &Spec, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    if !console.background_level() {
        console.set_mode(Mode::Gameplay);
    }
    Ok(None)
}

#[cfg(test)]
mod game_ui_tests {
    use crate::console::{Console, Mode};

    #[test]
    fn showconsole_and_hideconsole_open_and_close_without_toggling() {
        let mut console = Console::default();
        console.submit("showconsole");
        assert!(console.console_open());
        console.submit("showconsole");
        assert!(console.console_open(), "a second showconsole keeps it open");
        console.submit("hideconsole");
        assert!(!console.console_open());
        console.submit("hideconsole");
        assert!(!console.console_open());
    }

    #[test]
    fn game_ui_commands_open_the_pause_menu_and_options_and_close_them() {
        let mut console = Console::default();
        assert_eq!(console.mode(), Mode::Gameplay);
        console.submit("gameui_activate");
        assert_eq!(console.mode(), Mode::Pause);
        console.submit("gamemenucommand OpenOptionsDialog");
        assert_eq!(console.mode(), Mode::Options);
        console.submit("gameui_hide");
        assert_eq!(console.mode(), Mode::Gameplay);
        console.submit("gamemenucommand OpenOptionsDialog");
        assert_eq!(console.mode(), Mode::Options);
    }

    #[test]
    fn startup_commands_wait_for_the_first_level_and_their_menus_stay_open() {
        let mut console = Console::default();
        console.queue_startup_commands(vec![
            "gamemenucommand OpenOptionsDialog".into(),
            "toggleconsole".into(),
        ]);
        assert!(console.startup_pending());
        assert_eq!(console.mode(), Mode::Gameplay);
        console.level_loaded(false);
        assert!(!console.startup_pending());
        assert_eq!(console.mode(), Mode::Console);
        assert_eq!(
            console.open_windows(),
            [
                crate::console::Window::Options,
                crate::console::Window::Console
            ]
        );
    }
}
