//! Video cvars (the Options Video page and its Advanced dialog): fields of view, the video mode
//! and the material/render detail settings.

use super::{Outcome, Spec};
use crate::config::{
    Antialiasing, DisplayMode, Filtering, HdrMode, ModelDetail, ShaderDetail, ShadowDetail,
    TextureDetail, WaterDetail,
};
use crate::console::{Console, Effect};
use source_ui::console::command::{flag, parse, Invalid};

/// `fov` and its alias `fov_desired`.
pub(super) fn fov_value(console: &Console) -> Option<String> {
    Some(format!("{:.0}", console.config.video.fov))
}

pub(super) fn set_fov(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.fov = parse::<f32>(arg)?.clamp(50.0, 120.0);
    Ok(())
}

pub(super) fn vertical_fov_value(console: &Console) -> Option<String> {
    Some(format!("{:.0}", console.config.video.vertical_fov))
}

pub(super) fn set_vertical_fov(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    let fov = parse::<f32>(arg)?;
    console.config.video.vertical_fov = if fov <= 0.0 {
        0.0
    } else {
        fov.clamp(40.0, 100.0)
    };
    Ok(())
}

pub(super) fn viewmodel_fov_value(console: &Console) -> Option<String> {
    Some(format!("{:.0}", console.config.video.viewmodel_fov))
}

pub(super) fn set_viewmodel_fov(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.viewmodel_fov = parse::<f32>(arg)?.clamp(40.0, 80.0);
    Ok(())
}

pub(super) fn mat_colorcorrection_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.color_correction).to_string())
}

pub(super) fn set_mat_colorcorrection(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.color_correction = flag(arg)?;
    Ok(())
}

pub(super) fn mat_hdr_level_value(console: &Console) -> Option<String> {
    Some(console.config.video.hdr.level().to_string())
}

pub(super) fn set_mat_hdr_level(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.hdr = HdrMode::from_level(parse(arg)?).ok_or(Invalid::Message(
        "mat_hdr_level must be 0 (None), 1 (Bloom) or 2 (Full)",
    ))?;
    Ok(())
}

pub(super) fn mat_motion_blur_enabled_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.motion_blur).to_string())
}

pub(super) fn set_mat_motion_blur_enabled(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.motion_blur = flag(arg)?;
    console.config.video.advanced.motion_blur_viewmodel = false;
    console.config.video.advanced.motion_blur_world = false;
    Ok(())
}

pub(super) fn mat_queue_mode_value(console: &Console) -> Option<String> {
    Some(if console.config.video.advanced.multicore {
        "-1".to_owned()
    } else {
        "0".to_owned()
    })
}

pub(super) fn set_mat_queue_mode(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.multicore = parse::<i32>(arg)? != 0;
    Ok(())
}

pub(super) fn mat_antialias_value(console: &Console) -> Option<String> {
    Some(
        match console.config.video.advanced.antialiasing {
            Antialiasing::None => "0",
            Antialiasing::X2 => "2",
            Antialiasing::X4 => "4",
            Antialiasing::X8 => "8",
        }
        .to_string(),
    )
}

pub(super) fn set_mat_antialias(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.antialiasing = match arg {
        "0" | "1" => Antialiasing::None,
        "2" => Antialiasing::X2,
        "4" => Antialiasing::X4,
        "8" => Antialiasing::X8,
        _ => return Err(Invalid::Usage),
    };
    Ok(())
}

pub(super) fn mat_aaquality(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    if args.is_empty() {
        console.log("\"mat_aaquality\" = \"0\"");
    } else if args.len() == 1 {
        let q: u32 = args[0].parse().map_err(|_| spec.usage())?;
        console.log(format!("mat_aaquality {q}"));
    } else {
        return Err(spec.usage());
    }
    Ok(None)
}

pub(super) fn mat_aaquality_value(_console: &Console) -> Option<String> {
    Some("0".to_owned())
}

pub(super) fn mat_disable_d3d9ex_value(console: &Console) -> Option<String> {
    Some(u8::from(!console.config.video.advanced.d3d9ex).to_string())
}

pub(super) fn set_mat_disable_d3d9ex(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.d3d9ex = !flag(arg)?;
    Ok(())
}

pub(super) fn mat_forceaniso_value(console: &Console) -> Option<String> {
    Some(
        match console.config.video.advanced.filtering {
            Filtering::Bilinear | Filtering::Trilinear => "1",
            Filtering::Anisotropic2 => "2",
            Filtering::Anisotropic4 => "4",
            Filtering::Anisotropic8 => "8",
            Filtering::Anisotropic16 => "16",
        }
        .to_string(),
    )
}

pub(super) fn set_mat_forceaniso(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.filtering = match arg {
        "0" | "1" => {
            if matches!(console.config.video.advanced.filtering, Filtering::Bilinear) {
                Filtering::Bilinear
            } else {
                Filtering::Trilinear
            }
        }
        "2" => Filtering::Anisotropic2,
        "4" => Filtering::Anisotropic4,
        "8" => Filtering::Anisotropic8,
        "16" => Filtering::Anisotropic16,
        _ => return Err(Invalid::Usage),
    };
    Ok(())
}

pub(super) fn mat_trilinear_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.filtering != Filtering::Bilinear).to_string())
}

pub(super) fn set_mat_trilinear(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.filtering =
        match (flag(arg)?, console.config.video.advanced.filtering) {
            (false, _) => Filtering::Bilinear,
            (true, Filtering::Bilinear) => Filtering::Trilinear,
            (true, other) => other,
        };
    Ok(())
}

pub(super) fn mat_vsync_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.vsync).to_string())
}

pub(super) fn set_mat_vsync(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.vsync = flag(arg)?;
    Ok(())
}

pub(super) fn mat_picmip_value(console: &Console) -> Option<String> {
    Some(
        match console.config.video.advanced.texture_detail {
            TextureDetail::VeryHigh => "-1",
            TextureDetail::High => "0",
            TextureDetail::Medium => "1",
            TextureDetail::Low => "2",
        }
        .to_string(),
    )
}

pub(super) fn set_mat_picmip(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.texture_detail = match arg {
        "-1" => TextureDetail::VeryHigh,
        "0" => TextureDetail::High,
        "1" => TextureDetail::Medium,
        "2" => TextureDetail::Low,
        _ => return Err(Invalid::Usage),
    };
    Ok(())
}

pub(super) fn mat_reducefillrate_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.shader_detail == ShaderDetail::Low).to_string())
}

pub(super) fn set_mat_reducefillrate(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.shader_detail = if flag(arg)? {
        ShaderDetail::Low
    } else {
        ShaderDetail::High
    };
    Ok(())
}

/// `r_classic_blood` and `r_classic_fire` share one setting.
pub(super) fn classic_effects_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.classic_effects).to_string())
}

pub(super) fn set_classic_effects(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.classic_effects = flag(arg)?;
    Ok(())
}

pub(super) fn r_rootlod_value(console: &Console) -> Option<String> {
    Some(
        match console.config.video.advanced.model_detail {
            ModelDetail::High => "0",
            ModelDetail::Medium => "1",
            ModelDetail::Low => "2",
        }
        .to_string(),
    )
}

pub(super) fn set_r_rootlod(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.model_detail = match arg {
        "0" => ModelDetail::High,
        "1" => ModelDetail::Medium,
        "2" => ModelDetail::Low,
        _ => return Err(Invalid::Usage),
    };
    Ok(())
}

pub(super) fn r_waterforceexpensive_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.water_detail != WaterDetail::Simple).to_string())
}

pub(super) fn set_r_waterforceexpensive(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.water_detail =
        match (flag(arg)?, console.config.video.advanced.water_detail) {
            (false, _) => WaterDetail::Simple,
            (true, WaterDetail::Simple) => WaterDetail::World,
            (true, other) => other,
        };
    Ok(())
}

pub(super) fn r_waterforcereflectentities_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.water_detail == WaterDetail::All).to_string())
}

pub(super) fn set_r_waterforcereflectentities(
    console: &mut Console,
    arg: &str,
) -> Result<(), Invalid> {
    console.config.video.advanced.water_detail = if flag(arg)? {
        WaterDetail::All
    } else if console.config.video.advanced.water_detail == WaterDetail::All {
        WaterDetail::World
    } else {
        console.config.video.advanced.water_detail
    };
    Ok(())
}

pub(super) fn r_shadowrendertotexture_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.shadow_detail != ShadowDetail::Low).to_string())
}

pub(super) fn set_r_shadowrendertotexture(console: &mut Console, arg: &str) -> Result<(), Invalid> {
    console.config.video.advanced.shadow_detail =
        match (flag(arg)?, console.config.video.advanced.shadow_detail) {
            (false, _) => ShadowDetail::Low,
            (true, ShadowDetail::Low) => ShadowDetail::High,
            (true, other) => other,
        };
    Ok(())
}

pub(super) fn r_flashlightdepthtexture_value(console: &Console) -> Option<String> {
    Some(u8::from(console.config.video.advanced.shadow_detail == ShadowDetail::High).to_string())
}

pub(super) fn set_r_flashlightdepthtexture(
    console: &mut Console,
    arg: &str,
) -> Result<(), Invalid> {
    console.config.video.advanced.shadow_detail = if flag(arg)? {
        ShadowDetail::High
    } else if console.config.video.advanced.shadow_detail == ShadowDetail::High {
        ShadowDetail::Medium
    } else {
        console.config.video.advanced.shadow_detail
    };
    Ok(())
}

pub(super) fn mat_setvideomode(
    console: &mut Console,
    spec: &'static Spec,
    args: &[String],
) -> Outcome {
    let usage = || spec.usage();
    let [w, h, windowed] = args else {
        return Err(usage());
    };
    let width: u32 = w.parse().map_err(|_| usage())?;
    let height: u32 = h.parse().map_err(|_| usage())?;
    if !(320..=8192).contains(&width) || !(240..=8192).contains(&height) {
        return Err(usage());
    }
    let display_mode = match windowed.as_str() {
        "1" => DisplayMode::Windowed,
        "0" => DisplayMode::Fullscreen,
        _ => return Err(usage()),
    };
    console.config.video.width = width;
    console.config.video.height = height;
    console.config.video.display_mode = display_mode;
    let _ = console.config.save();
    console.log(format!("mat_setvideomode {width} {height} {windowed}"));
    Ok(Some(Effect::ApplyConfig(Box::new(console.config.clone()))))
}
