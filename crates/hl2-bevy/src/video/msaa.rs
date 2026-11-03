//! Antialiasing sample counts the setting and the adapter allow.
use bevy::prelude::Msaa;

/// Converts the UI antialiasing choice into Bevy's multisampling setting.
#[inline]
pub fn msaa_for(aa: hl2_ui::config::Antialiasing) -> Msaa {
    match aa {
        hl2_ui::config::Antialiasing::None => Msaa::Off,
        hl2_ui::config::Antialiasing::X2 => Msaa::Sample2,
        hl2_ui::config::Antialiasing::X4 => Msaa::Sample4,
        hl2_ui::config::Antialiasing::X8 => Msaa::Sample8,
    }
}

/// Determines the maximum MSAA sample count supported by the GPU adapter for color and depth textures.
///
/// WebGPU specifications guarantee sample counts `[1, 4]`. Metal on Apple Silicon strictly supports
/// `[1, 2, 4]` for color (`Rgba8UnormSrgb`) and depth (`Depth32Float`) textures. Many integrated GPUs
/// and CPU software renderers also cap at 4 or 2 samples. Discrete GPUs on Vulkan / DirectX 12 support
/// 8 samples.
pub fn adapter_max_msaa_samples(
    adapter: Option<&bevy::render::renderer::RenderAdapterInfo>,
) -> u32 {
    let Some(adapter) = adapter else {
        return 4;
    };
    let backend_str = format!("{:?}", adapter.0.backend);
    let device_str = format!("{:?}", adapter.0.device_type);
    let name_lower = adapter.0.name.to_ascii_lowercase();

    // 1. Metal or Apple Silicon strictly supports [1, 2, 4] samples for color & depth render targets
    if backend_str.contains("Metal") || name_lower.contains("apple") {
        return 4;
    }

    // 2. CPU software renderers (llvmpipe, softpipe, warp, etc.)
    if device_str.contains("Cpu")
        || name_lower.contains("llvmpipe")
        || name_lower.contains("softpipe")
        || name_lower.contains("basic render")
    {
        return 2;
    }

    // 3. Integrated GPUs generally cap at 4 samples in WebGPU baseline
    if device_str.contains("Integrated")
        || name_lower.contains("intel hd")
        || name_lower.contains("intel uhd")
        || name_lower.contains("adreno")
        || name_lower.contains("mali")
    {
        return 4;
    }

    // 4. Discrete GPUs (NVIDIA GeForce, AMD Radeon, discrete Intel Arc) on Vulkan / DX12
    if device_str.contains("Discrete") {
        return 8;
    }

    4
}

/// Converts the UI antialiasing choice into Bevy's multisampling setting, clamped to the
/// hardware adapter's maximum supported sample count for color and depth textures.
#[inline]
pub fn msaa_for_hardware(aa: hl2_ui::config::Antialiasing, max_samples: u32) -> Msaa {
    let effective = if aa.samples() > max_samples {
        match max_samples {
            s if s >= 4 => hl2_ui::config::Antialiasing::X4,
            s if s >= 2 => hl2_ui::config::Antialiasing::X2,
            _ => hl2_ui::config::Antialiasing::None,
        }
    } else {
        aa
    };
    msaa_for(effective)
}
