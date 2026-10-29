//! The developer console itself: help, output, filters and leaving.

use super::{Outcome, Spec, COMMANDS};
use crate::console::{Console, Effect};
use source_ui::console::command::boolean;

pub(super) fn help(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    if args.len() > 1 {
        return Err(spec.usage());
    }
    list(console, args, |spec, query| spec.name == query)
}

pub(super) fn find(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    if args.len() != 1 {
        return Err(spec.usage());
    }
    list(console, args, |spec, query| {
        spec.help.to_ascii_lowercase().contains(query)
    })
}

/// Prints the help of every command `matches` the query (all without one).
fn list(console: &mut Console, args: &[String], matches: fn(&Spec, &str) -> bool) -> Outcome {
    let query = args.first().map(|s| s.to_ascii_lowercase());
    let found: Vec<_> = COMMANDS
        .iter()
        .filter(|spec| query.as_ref().is_none_or(|q| matches(spec, q)))
        .collect();
    if found.is_empty() {
        console.log("No implemented command matches.");
    }
    for spec in found {
        console.log(spec.help);
    }
    Ok(None)
}

pub(super) fn clear(console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    console.shell.panel.clear_output();
    Ok(None)
}

pub(super) fn echo(console: &mut Console, _spec: &'static Spec, args: &[String]) -> Outcome {
    console.log(args.join(" "));
    Ok(None)
}

pub(super) fn toggleconsole(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    console.toggle();
    Ok(None)
}

pub(super) fn quit(_console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    if !args.is_empty() {
        return Err(spec.usage());
    }
    Ok(Some(Effect::Quit))
}

/// A console cvar: prints its value without an argument, or `set`s it and scrolls the output to
/// the bottom (the filter changes what is shown).
fn console_cvar(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
    set: impl FnOnce(&mut Console, &String) -> Result<(), String>,
) -> Outcome {
    match args {
        [] => {
            let value = console.cvar_value(spec.name).unwrap_or_default();
            console.log(format!("\"{}\" = \"{value}\"", spec.name));
        }
        [value] => {
            set(console, value)?;
            console.shell.panel.scroll = 0;
        }
        _ => return Err(spec.usage()),
    }
    Ok(None)
}

pub(super) fn viewmodel_tweak_mode(
    _console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    match args {
        [value] => Ok(Some(Effect::ViewmodelTweakMode(
            boolean(value).ok_or_else(|| spec.usage())?,
        ))),
        _ => Err(spec.usage()),
    }
}

pub(super) fn viewmodel_tweak_reset(
    _console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    if args.is_empty() {
        Ok(Some(Effect::ViewmodelTweakReset))
    } else {
        Err(spec.usage())
    }
}

pub(super) fn con_filter_enable(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    console_cvar(console, spec, args, |console, value| {
        console.shell.panel.con_filter = value
            .parse::<u8>()
            .ok()
            .filter(|v| *v <= 2)
            .ok_or_else(|| spec.usage())?;
        Ok(())
    })
}

pub(super) fn con_filter_enable_value(console: &Console) -> Option<String> {
    Some(console.shell.panel.con_filter.to_string())
}

pub(super) fn con_filter_text(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    console_cvar(console, spec, args, |console, value| {
        console.shell.panel.con_filter_text = value.clone();
        Ok(())
    })
}

pub(super) fn con_filter_text_value(console: &Console) -> Option<String> {
    Some(console.shell.panel.con_filter_text.clone())
}

pub(super) fn con_timestamp(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    console_cvar(console, spec, args, |console, value| {
        console.shell.panel.con_timestamp = boolean(value).ok_or_else(|| spec.usage())?;
        Ok(())
    })
}

pub(super) fn con_timestamp_value(console: &Console) -> Option<String> {
    Some(u8::from(console.shell.panel.con_timestamp).to_string())
}
