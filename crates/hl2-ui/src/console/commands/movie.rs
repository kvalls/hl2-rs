//! Movie playback console commands (`playvideo`, `playvideo_exitcommand`).

use super::{Outcome, Spec};
use crate::console::{Console, Effect};

pub(super) fn playvideo(_console: &mut Console, spec: &'static Spec, args: &[String]) -> Outcome {
    let (filename, width, height) = match args {
        [filename] => (filename.clone(), None, None),
        [filename, w, h] => {
            let width = w.parse::<u32>().map_err(|_| spec.usage())?;
            let height = h.parse::<u32>().map_err(|_| spec.usage())?;
            (filename.clone(), Some(width), Some(height))
        }
        _ => return Err(spec.usage()),
    };
    Ok(Some(Effect::PlayVideo {
        filename,
        exit_command: None,
        width,
        height,
    }))
}

pub(super) fn playvideo_exitcommand(
    _console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    if args.len() < 2 {
        return Err(spec.usage());
    }
    let filename = args[0].clone();
    let exit_command = Some(args[1..].join(" "));
    Ok(Some(Effect::PlayVideo {
        filename,
        exit_command,
        width: None,
        height: None,
    }))
}
