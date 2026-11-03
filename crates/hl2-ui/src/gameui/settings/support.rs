//! Platform availability and the current implementation status of settings controls.
//! Remove a development entry when its feature is implemented and validated.
use super::{AdvancedChoice, Binding, Choice, Command, Panel, Toggle, Value};
use source_ui::layout::Control;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
    Other,
}
impl Platform {
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

pub fn binding_for_control(panel: Panel, control: &Control) -> Option<Binding> {
    super::binding(panel, &control.name).or_else(|| super::command_binding(&control.command))
}

/// Labels inherit policy from their associate or conventional field-name suffix.
pub fn related_binding(panel: Panel, control: &Control) -> Option<Binding> {
    binding_for_control(panel, control)
        .or_else(|| super::binding(panel, &control.associate))
        .or_else(|| {
            let prefix = control.name.len().checked_sub(5)?;
            control
                .name
                .get(prefix..)?
                .eq_ignore_ascii_case("label")
                .then(|| control.name.get(..prefix))
                .flatten()
                .and_then(|name| super::binding(panel, name))
        })
}

/// UI availability only: values saved on another platform are never rewritten here.
pub fn available(panel: Panel, control: &Control, platform: Platform) -> bool {
    if !matches!(panel, Panel::Mouse | Panel::VideoAdvanced) {
        return true;
    }
    match related_binding(panel, control) {
        Some(Binding::Choice(Choice::Advanced(AdvancedChoice::D3D9Ex))) => {
            platform == Platform::Windows
        }
        Some(
            Binding::Toggle(Toggle::RawInput | Toggle::Acceleration)
            | Binding::Slider(Value::AccelExponent)
            | Binding::Entry(Value::AccelExponent),
        ) => platform != Platform::MacOs,
        _ => true,
    }
}

/// A localized, specific status report; absence means no development marker or hint.
pub fn development(binding: Binding) -> Option<&'static str> {
    use crate::gameui::localize::placeholders as text;
    Some(match binding {
        Binding::Toggle(Toggle::RawInput) => text::STATUS_RAW_INPUT,
        Binding::Toggle(Toggle::MuteLoseFocus) => text::STATUS_FOCUS_AUDIO,
        Binding::Choice(Choice::AimAssist | Choice::GamepadAimAssist)
        | Binding::Slider(Value::AimAssistScale | Value::GamepadAimAssistScale) => {
            text::STATUS_AIM_ASSIST
        }
        Binding::Choice(Choice::Speakers) => text::STATUS_SPEAKERS,
        Binding::Choice(Choice::Quality) => text::STATUS_SOUND_QUALITY,
        Binding::Choice(Choice::Language) => text::STATUS_LANGUAGE,
        Binding::Slider(Value::JoystickScale) | Binding::Choice(Choice::WeaponSelection) => {
            text::STATUS_GAMEPAD
        }
        Binding::Choice(Choice::Advanced(AdvancedChoice::ShaderDetail)) => text::STATUS_SHADERS,
        Binding::Choice(Choice::Advanced(AdvancedChoice::WaterDetail)) => text::STATUS_WATER,
        Binding::Choice(Choice::Advanced(AdvancedChoice::Multicore)) => text::STATUS_MULTICORE,
        Binding::Choice(Choice::Advanced(AdvancedChoice::ClassicEffects)) => {
            text::STATUS_CLASSIC_EFFECTS
        }
        Binding::Choice(Choice::Advanced(AdvancedChoice::D3D9Ex)) => text::STATUS_AERO,
        Binding::Command(Command::SteamInput) => text::STATUS_STEAM_INPUT,
        Binding::Command(command) if command.placeholder() => text::STATUS_UNIMPLEMENTED,
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
