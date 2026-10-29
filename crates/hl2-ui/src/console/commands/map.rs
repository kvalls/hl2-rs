//! Maps and the campaign: loading maps, the main menu and its background map, skill and save
//! data.

use super::{Outcome, Spec};
use crate::console::{Console, Effect, Mode};
use source_ui::console::command::{flag, parse, Invalid};

pub(super) fn map(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
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

pub(super) fn mainmenu(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    console.set_mode(Mode::MainMenu);
    Ok(None)
}

pub(super) fn background_map(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    if args.is_empty() {
        match console.background_map() {
            Some(map) => console.log(format!("\"background_map\" = \"{map}\"")),
            None => console.log("\"background_map\" is not set"),
        }
    } else if args.len() == 1 {
        let map = args[0].strip_suffix(".bsp").unwrap_or(&args[0]);
        if map.eq_ignore_ascii_case("default")
            || map == "0"
            || map.eq_ignore_ascii_case("none")
            || map.is_empty()
        {
            console.set_background(None);
            let _ = console.config.save();
            console.log(format!(
                "background_map reset to default ({})",
                console.gameui.background_map.as_deref().unwrap_or("none")
            ));
        } else {
            if map.len() > 128 || !map.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err("background_map: use a simple installed map name".into());
            }
            let current = console.loaded_map.as_ref().map(|m| m.game.as_str());
            let background = console.game_map(map, current);
            console.set_background(Some(background));
            let _ = console.config.save();
            console.log(format!("background_map {map}"));
        }
        return Ok(Some(Effect::ApplyConfig(Box::new(console.config.clone()))));
    } else {
        return Err(spec.usage());
    }
    Ok(None)
}

pub(super) fn background_map_value(console: &Console) -> Option<String> {
    Some(console.background_map()?.to_owned())
}

pub(super) fn map_background(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
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
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    console.log("migrate_episodic_data: save data migration is not implemented in this build");
    Ok(None)
}
