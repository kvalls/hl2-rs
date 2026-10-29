//! Mouse cvars (the Options Mouse page): sensitivity, pitch, filtering, acceleration and aim
//! assistance.

use crate::config::AimAssist;
use crate::console::Console;
use crate::input::MOUSE_DEGREES_PER_COUNT;
use source_ui::console::command::{flag, parse, Invalid};

pub(super) fn sensitivity_value(console: &Console) -> Option<String> {
    Some(format!("{:.2}", console.config.mouse.sensitivity))
}

pub(super) fn set_sensitivity(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.mouse.sensitivity = parse::<f32>(arg)?.clamp(0.1, 20.0);
    Ok(())
}

pub(super) fn m_pitch_value(console: &Console) -> Option<String> {
    Some(format!(
        "{:.3}",
        if console.config.mouse.invert {
            -MOUSE_DEGREES_PER_COUNT
        } else {
            MOUSE_DEGREES_PER_COUNT
        }
    ))
}

pub(super) fn set_m_pitch(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.mouse.invert = parse::<f32>(arg)? < 0.0;
    Ok(())
}

pub(super) fn m_filter_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.mouse.filter).to_string())
}

pub(super) fn set_m_filter(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.mouse.filter = flag(arg)?;
    Ok(())
}

pub(super) fn m_rawinput_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.mouse.raw_input).to_string())
}

pub(super) fn set_m_rawinput(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.mouse.raw_input = flag(arg)?;
    Ok(())
}

pub(super) fn m_customaccel_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.mouse.acceleration).to_string())
}

pub(super) fn set_m_customaccel(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.mouse.acceleration = match arg {
        "0" => false,
        "1" | "3" => true,
        _ => return Err(Invalid::Usage),
    };
    Ok(())
}

pub(super) fn m_customaccel_exponent_value(console: &Console) -> Option<String> {
    Some(format!("{:.2}", console.config.mouse.accel_exponent))
}

pub(super) fn set_m_customaccel_exponent(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.mouse.accel_exponent = parse::<f32>(arg)?.clamp(1.0, 1.4);
    Ok(())
}

/// `sk_autoaim_mode` and its alias `m_autoaim_mode`.
pub(super) fn autoaim_mode_value(console: &Console) -> Option<String> {
    Some(
        match console.config.mouse.aim_assist {
            AimAssist::Off => "0",
            AimAssist::Basic => "1",
            AimAssist::Enhanced => "2",
        }
        .to_string(),
    )
}

pub(super) fn set_autoaim_mode(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.mouse.aim_assist = match parse::<u8>(arg)? {
        0 => AimAssist::Off,
        1 => AimAssist::Basic,
        2 => AimAssist::Enhanced,
        _ => return Err(Invalid::Usage),
    };
    Ok(())
}

/// `sk_autoaim_scale1` and its alias `m_autoaim_scale`.
pub(super) fn autoaim_scale_value(console: &Console) -> Option<String> {
    Some(format!("{:.2}", console.config.mouse.aim_assist_scale))
}

pub(super) fn set_autoaim_scale(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.mouse.aim_assist_scale = parse::<f32>(arg)?.clamp(0.5, 2.0);
    Ok(())
}
