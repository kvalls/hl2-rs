//! Cheats and entities: `sv_cheats`, the cheat commands it gates, entity creation and inputs,
//! NPC AI switches and the player position.

use super::{Outcome, Spec};
use crate::console::{vector, Console, Effect};
use source_ui::console::command::boolean;

/// `Err` unless `sv_cheats` is on.
fn cheat(console: &Console, spec: &Spec) -> Result<(), String> {
    if console.cheats {
        Ok(())
    } else {
        Err(format!("Can't use {} without sv_cheats 1", spec.name))
    }
}

pub(super) fn sv_cheats(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    if args.is_empty() {
        console.log(format!("\"sv_cheats\" = \"{}\"", u8::from(console.cheats)));
    } else if args.len() == 1 {
        console.cheats = boolean(&args[0]).ok_or_else(|| spec.usage())?;
        console.log(format!("sv_cheats {}", u8::from(console.cheats)));
    } else {
        return Err(spec.usage());
    }
    Ok(None)
}

pub(super) fn sv_cheats_value(console: &Console) -> Option<String> {
    Some(u8::from(console.cheats).to_string())
}

/// `impulse 100` toggles the flashlight (not a cheat); `impulse 101` grants the loadout (a cheat).
pub(super) fn impulse(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    match args {
        [n] if n == "100" => Ok(Some(Effect::Flashlight)),
        [n] if n == "101" => {
            cheat(console, spec)?;
            Ok(Some(Effect::Loadout))
        }
        _ => Err(spec.usage()),
    }
}

pub(super) fn noclip(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    let value = match args {
        [] => None,
        [value] => Some(boolean(value).ok_or_else(|| spec.usage())?),
        _ => return Err(spec.usage()),
    };
    cheat(console, spec)?;
    Ok(Some(Effect::Noclip(value)))
}

/// `npc_create`, `npc_create_aimed`, `prop_physics_create` and `ent_create`: cheats the host's
/// NPC console runs.
pub(super) fn create(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    cheat(console, spec)?;
    npc_command(spec, args)
}

/// Not a cheat: it only picks the weapon for the next `npc_create`.
pub(super) fn npc_create_equipment(
    _console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    npc_command(spec, args)
}

fn npc_command(spec: &Spec, args: &[String]) -> Outcome {
    // Re-quote arguments with spaces (ent_create angles "0 90 0").
    Ok(Some(Effect::NpcCommand(
        std::iter::once(spec.name.to_owned())
            .chain(args.iter().map(|a| {
                if a.is_empty() || a.contains(char::is_whitespace) {
                    format!("\"{a}\"")
                } else {
                    a.clone()
                }
            }))
            .collect::<Vec<_>>()
            .join(" "),
    )))
}

pub(super) fn ai_disable(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    cheat(console, spec)?;
    Ok(Some(Effect::AiDisable))
}

pub(super) fn ai_enable(_console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    let value = match args {
        [] => None,
        [value] => Some(boolean(value).ok_or_else(|| spec.usage())?),
        _ => return Err(spec.usage()),
    };
    Ok(Some(Effect::AiEnable(value)))
}

pub(super) fn getpos(_console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    Ok(Some(Effect::Getpos))
}

pub(super) fn setpos(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    let value = vector(args).ok_or_else(|| spec.usage())?;
    cheat(console, spec)?;
    Ok(Some(Effect::Setpos {
        x: value.x,
        y: value.y,
        z: (args.len() == 3).then_some(value.z),
    }))
}

pub(super) fn setang(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    let value = vector(args).ok_or_else(|| spec.usage())?;
    cheat(console, spec)?;
    if value.z != 0. {
        return Err("setang: camera roll is not implemented; use zero roll".into());
    }
    Ok(Some(Effect::Setang(value)))
}

pub(super) fn ent_fire(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    let usage = || spec.usage();
    cheat(console, spec)?;
    if args.is_empty() || args.len() > 4 || args[0].is_empty() {
        return Err(usage());
    }
    let delay = args
        .get(3)
        .map_or(Ok(0.), |s| s.parse::<f64>())
        .map_err(|_| usage())?;
    if !delay.is_finite() || delay < 0. {
        return Err("ent_fire: delay must be finite and nonnegative".into());
    }
    let input = args.get(1).map_or("Use", String::as_str);
    if input.is_empty() {
        return Err(usage());
    }
    Ok(Some(Effect::Fire {
        target: args[0].clone(),
        input: input.into(),
        parameter: args.get(2).cloned().unwrap_or_default(),
        // Desktop ent_fire parses an integer delay; map I/O keeps fractions.
        delay: delay.trunc(),
    }))
}
