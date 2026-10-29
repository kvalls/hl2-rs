//! Movement script commands and parser for automated replay validation.

use anyhow::{Result, bail};
use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub(crate) tick: u64,
    #[serde(default)]
    pub(crate) actions: Vec<crate::gameplay::Action>,
    #[serde(default)]
    pub(crate) ui: Vec<crate::ui_host::Action>,
    #[serde(default)]
    pub(crate) primary: bool,
    #[serde(default)]
    pub(crate) secondary: bool,
    pub(crate) label: Option<String>,
    pub(crate) eye: Option<[f32; 3]>,
    pub(crate) yaw: Option<f32>,
    pub(crate) pitch: Option<f32>,
    /// Horizontal 4:3 field of view in degrees, like the Source `fov` command.
    pub(crate) fov: Option<f32>,
    pub(crate) paused: Option<bool>,
    pub(crate) dev_overlay: Option<bool>,
    pub(crate) fly: Option<bool>,
    #[serde(default)]
    pub(crate) forward: f32,
    #[serde(default)]
    pub(crate) side: f32,
    #[serde(default)]
    pub(crate) jump: bool,
    #[serde(default)]
    pub(crate) crouch: bool,
    #[serde(default)]
    pub(crate) sprint: bool,
    #[serde(default)]
    pub(crate) slow: bool,
}

pub fn read_script(path: &Path) -> Result<Vec<Command>> {
    let bytes = std::fs::read(path)?;
    if bytes.len() > 1024 * 1024 {
        bail!("movement script exceeds 1 MiB");
    }
    let commands: Vec<Command> = serde_json::from_slice(&bytes)?;
    if commands.is_empty() || commands.len() > 1024 {
        bail!("movement script must have 1..1024 commands");
    }
    let mut previous = 0;
    for c in &commands {
        if c.ui.len() > 64
            || c.ui.iter().any(|a| !a.valid())
            || c.actions.len() > 64
            || c.actions.iter().any(|a| !a.valid())
            || c.tick <= previous
            || c.tick > 6000
            || !c.forward.is_finite()
            || !c.side.is_finite()
            || c.forward.abs() > 1.
            || c.side.abs() > 1.
            || c.eye.is_some_and(|p| p.iter().any(|n| !n.is_finite()))
            || c.yaw.is_some_and(|n| !n.is_finite())
            || c.pitch.is_some_and(|n| !(-89. ..=89.).contains(&n))
            || c.fov.is_some_and(|n| !(1. ..=170.).contains(&n))
            || c.label.as_ref().is_some_and(|s| s.len() > 128)
        {
            bail!("invalid movement command at tick {}", c.tick);
        }
        previous = c.tick;
    }
    Ok(commands)
}
