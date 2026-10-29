//! System recommended settings detection and performance benchmark suite.
//!
//! Half-Life 2 / Source Engine inspects system hardware (CPU core count, GPU vendor/device,
//! and shader capabilities) to recommend optimal video settings for the host machine.
//!
//! In retail Source (`OptionsSubVideoAdvancedDlg.res`), `Label11` displays:
//! `* Setting recommended for your system` (`#GameUI_RecommendedSettings`),
//! and each combo box item matching the system recommendation is designated with an
//! asterisk (` *`) appended to its label (e.g. `High *`, `Reflect all *`, `Enabled *`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::config::{
    AdvancedVideoSettings, Antialiasing, Filtering, HdrMode, ModelDetail, ShaderDetail,
    ShadowDetail, TextureDetail, WaterDetail,
};
use crate::gameui::settings::AdvancedChoice;

/// System hardware performance tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HardwareTier {
    Low,
    Medium,
    High,
    Ultra,
}

impl HardwareTier {
    pub const ALL: [Self; 4] = [Self::Low, Self::Medium, Self::High, Self::Ultra];

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low Tier",
            Self::Medium => "Medium Tier",
            Self::High => "High Tier",
            Self::Ultra => "Ultra Tier",
        }
    }
}

/// Host CPU specifications relevant to Source Engine task scheduling and simulation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuSpecs {
    /// Number of available logical processing cores.
    pub logical_cores: usize,
}

impl CpuSpecs {
    pub fn new(logical_cores: usize) -> Self {
        Self { logical_cores }
    }

    /// Whether the system has multiple cores available for threaded tasks / multicore rendering.
    pub fn is_multicore(&self) -> bool {
        self.logical_cores > 1
    }
}

/// GPU hardware category classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuDeviceType {
    Discrete,
    Integrated,
    Virtual,
    Cpu,
    Other,
    Unknown,
}

/// Host GPU specifications for hardware capability classification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuSpecs {
    pub name: String,
    pub device_type: GpuDeviceType,
    pub vendor_id: Option<u32>,
    pub device_id: Option<u32>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub max_msaa_samples: Option<u32>,
}

impl GpuSpecs {
    pub fn new(name: impl Into<String>, device_type: GpuDeviceType) -> Self {
        Self {
            name: name.into(),
            device_type,
            vendor_id: None,
            device_id: None,
            backend: None,
            max_msaa_samples: None,
        }
    }

    /// Maximum multisampling sample count supported by the hardware for color and depth textures.
    ///
    /// WebGPU guarantees [1, 4] samples. Metal limits color (Rgba8UnormSrgb) and depth (Depth32Float)
    /// attachments to [1, 2, 4] samples. Many integrated GPUs and software renderers also do not support
    /// 8 samples. Discrete GPUs on Vulkan / DirectX 12 support 8x MSAA.
    pub fn max_msaa_samples(&self) -> u32 {
        if let Some(samples) = self.max_msaa_samples {
            return samples;
        }
        let name_lower = self.name.to_ascii_lowercase();
        let backend_lower = self.backend.as_deref().unwrap_or("").to_ascii_lowercase();

        // 1. Metal or Apple Silicon strictly supports up to 4 samples for color and depth render targets
        if backend_lower.contains("metal") || name_lower.contains("apple") {
            return 4;
        }

        // 2. Software / CPU renderers (llvmpipe, softpipe, basic render)
        if self.device_type == GpuDeviceType::Cpu
            || name_lower.contains("llvmpipe")
            || name_lower.contains("softpipe")
            || name_lower.contains("basic render")
        {
            return 2;
        }

        // 3. Integrated GPUs generally cap at 4 samples in WebGPU baseline
        if self.device_type == GpuDeviceType::Integrated
            || name_lower.contains("intel hd")
            || name_lower.contains("intel uhd")
            || name_lower.contains("adreno")
            || name_lower.contains("mali")
        {
            return 4;
        }

        // 4. Discrete GPUs (NVIDIA GeForce, AMD Radeon, discrete Intel Arc) on Vulkan / DX12
        if self.device_type == GpuDeviceType::Discrete {
            return 8;
        }

        // Safe fallback for virtual/unknown hardware
        4
    }

    /// Highest antialiasing mode supported by this hardware.
    pub fn max_antialiasing(&self) -> Antialiasing {
        match self.max_msaa_samples() {
            s if s >= 8 => Antialiasing::X8,
            s if s >= 4 => Antialiasing::X4,
            s if s >= 2 => Antialiasing::X2,
            _ => Antialiasing::None,
        }
    }

    /// Antialiasing modes supported by this hardware.
    pub fn supported_antialiasing(&self) -> Vec<Antialiasing> {
        let max = self.max_antialiasing();
        [
            Antialiasing::None,
            Antialiasing::X2,
            Antialiasing::X4,
            Antialiasing::X8,
        ]
        .into_iter()
        .filter(|&aa| aa.samples() <= max.samples())
        .collect()
    }
}

/// System recommended settings profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemRecommendation {
    pub tier: HardwareTier,
    pub cpu: CpuSpecs,
    pub gpu: GpuSpecs,
    pub video: AdvancedVideoSettings,
    pub hdr: HdrMode,
}

impl Default for SystemRecommendation {
    fn default() -> Self {
        Self::detect()
    }
}

impl SystemRecommendation {
    /// Automatically detects host system specifications and generates recommendations.
    pub fn detect() -> Self {
        let logical_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let cpu = CpuSpecs { logical_cores };
        let gpu = Self::detect_gpu();
        Self::from_specs(cpu, gpu)
    }

    /// Probes platform GPU details or sensible defaults.
    pub fn detect_gpu() -> GpuSpecs {
        #[cfg(target_os = "macos")]
        {
            GpuSpecs {
                name: "Apple Silicon GPU".to_string(),
                device_type: GpuDeviceType::Integrated,
                vendor_id: Some(0x106b),
                device_id: None,
                backend: Some("Metal".to_string()),
                max_msaa_samples: Some(4),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            GpuSpecs {
                name: "Standard 3D Graphics Accelerator".to_string(),
                device_type: GpuDeviceType::Discrete,
                vendor_id: None,
                device_id: None,
                backend: None,
                max_msaa_samples: None,
            }
        }
    }

    /// Derives recommendations deterministically from CPU and GPU specifications.
    pub fn from_specs(cpu: CpuSpecs, gpu: GpuSpecs) -> Self {
        let tier = Self::derive_tier(&cpu, &gpu);
        let (video, hdr) = Self::settings_for_tier(tier, &cpu, &gpu);
        Self {
            tier,
            cpu,
            gpu,
            video,
            hdr,
        }
    }

    /// Infers hardware performance tier from system specifications.
    fn derive_tier(cpu: &CpuSpecs, gpu: &GpuSpecs) -> HardwareTier {
        if gpu.device_type == GpuDeviceType::Cpu || cpu.logical_cores <= 1 {
            return HardwareTier::Low;
        }

        let name_lower = gpu.name.to_ascii_lowercase();
        let is_ultra_gpu = name_lower.contains("4090")
            || name_lower.contains("4080")
            || name_lower.contains("3090")
            || name_lower.contains("3080")
            || name_lower.contains("7900")
            || name_lower.contains("m1 max")
            || name_lower.contains("m1 ultra")
            || name_lower.contains("m2 max")
            || name_lower.contains("m2 ultra")
            || name_lower.contains("m3 max")
            || name_lower.contains("m3 ultra");

        let is_low_gpu = name_lower.contains("intel hd")
            || name_lower.contains("intel uhd")
            || name_lower.contains("llvmpipe")
            || name_lower.contains("softpipe")
            || name_lower.contains("basic render")
            || name_lower.contains("gma");

        if is_low_gpu {
            return HardwareTier::Low;
        }

        if is_ultra_gpu && cpu.logical_cores >= 8 {
            return HardwareTier::Ultra;
        }

        match gpu.device_type {
            GpuDeviceType::Discrete => {
                if cpu.logical_cores >= 8 {
                    HardwareTier::Ultra
                } else if cpu.logical_cores >= 4 {
                    HardwareTier::High
                } else {
                    HardwareTier::Medium
                }
            }
            GpuDeviceType::Integrated => {
                // Unified memory architecture (Apple Silicon) or modern high-count integrated GPUs
                if name_lower.contains("apple") || cpu.logical_cores >= 8 {
                    HardwareTier::High
                } else if cpu.logical_cores >= 4 {
                    HardwareTier::Medium
                } else {
                    HardwareTier::Low
                }
            }
            GpuDeviceType::Virtual | GpuDeviceType::Other | GpuDeviceType::Unknown => {
                if cpu.logical_cores >= 6 {
                    HardwareTier::Medium
                } else {
                    HardwareTier::Low
                }
            }
            GpuDeviceType::Cpu => HardwareTier::Low,
        }
    }

    /// Generates recommended advanced video settings and HDR mode for a tier and GPU hardware.
    pub fn settings_for_tier(
        tier: HardwareTier,
        cpu: &CpuSpecs,
        gpu: &GpuSpecs,
    ) -> (AdvancedVideoSettings, HdrMode) {
        let max_aa = gpu.max_antialiasing();
        match tier {
            HardwareTier::Ultra => (
                AdvancedVideoSettings {
                    model_detail: ModelDetail::High,
                    texture_detail: TextureDetail::VeryHigh,
                    shader_detail: ShaderDetail::High,
                    water_detail: WaterDetail::All,
                    shadow_detail: ShadowDetail::High,
                    color_correction: true,
                    antialiasing: if max_aa.samples() >= 8 {
                        Antialiasing::X8
                    } else {
                        max_aa
                    },
                    filtering: Filtering::Anisotropic16,
                    vsync: false,
                    motion_blur: false,
                    multicore: cpu.is_multicore(),
                    ..AdvancedVideoSettings::default()
                },
                HdrMode::Full,
            ),
            HardwareTier::High => (
                AdvancedVideoSettings {
                    model_detail: ModelDetail::High,
                    texture_detail: TextureDetail::High,
                    shader_detail: ShaderDetail::High,
                    water_detail: WaterDetail::All,
                    shadow_detail: ShadowDetail::High,
                    color_correction: true,
                    antialiasing: if max_aa.samples() >= 4 {
                        Antialiasing::X4
                    } else {
                        max_aa
                    },
                    filtering: Filtering::Anisotropic8,
                    vsync: false,
                    motion_blur: false,
                    multicore: cpu.is_multicore(),
                    ..AdvancedVideoSettings::default()
                },
                HdrMode::Full,
            ),
            HardwareTier::Medium => (
                AdvancedVideoSettings {
                    model_detail: ModelDetail::Medium,
                    texture_detail: TextureDetail::Medium,
                    shader_detail: ShaderDetail::High,
                    water_detail: WaterDetail::World,
                    shadow_detail: ShadowDetail::Medium,
                    color_correction: true,
                    antialiasing: if max_aa.samples() >= 2 {
                        Antialiasing::X2
                    } else {
                        max_aa
                    },
                    filtering: Filtering::Anisotropic4,
                    vsync: false,
                    motion_blur: false,
                    multicore: cpu.is_multicore(),
                    ..AdvancedVideoSettings::default()
                },
                HdrMode::Full,
            ),
            HardwareTier::Low => (
                AdvancedVideoSettings {
                    model_detail: ModelDetail::Low,
                    texture_detail: TextureDetail::Low,
                    shader_detail: ShaderDetail::Low,
                    water_detail: WaterDetail::Simple,
                    shadow_detail: ShadowDetail::Low,
                    color_correction: false,
                    antialiasing: Antialiasing::None,
                    filtering: Filtering::Trilinear,
                    vsync: false,
                    motion_blur: false,
                    multicore: cpu.is_multicore(),
                    d3d9ex: false,
                    ..AdvancedVideoSettings::default()
                },
                HdrMode::None,
            ),
        }
    }

    /// The zero-based index of the recommended item for an `AdvancedChoice` combo box.
    pub fn recommended_advanced_index(&self, choice: AdvancedChoice) -> usize {
        match choice {
            AdvancedChoice::Antialiasing => {
                let supported = self.gpu.supported_antialiasing();
                supported
                    .iter()
                    .position(|&aa| aa == self.video.antialiasing)
                    .unwrap_or(0)
            }
            other => other.selected(&self.video),
        }
    }

    /// The zero-based index of the recommended item for `Choice::Hdr` (0 = None, 1 = Bloom, 2 = Full).
    pub fn recommended_hdr_index(&self) -> usize {
        self.hdr.level() as usize
    }

    /// Whether a specific item index in an `AdvancedChoice` combo box is the recommended setting.
    pub fn is_recommended_advanced(&self, choice: AdvancedChoice, index: usize) -> bool {
        self.recommended_advanced_index(choice) == index
    }

    /// Whether a specific item index in `Choice::Hdr` is the recommended setting.
    pub fn is_recommended_hdr(&self, index: usize) -> bool {
        self.recommended_hdr_index() == index
    }
}

/// Evaluated performance score for a video settings configuration on target hardware.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkScore {
    pub cpu_time_ms: f64,
    pub gpu_time_ms: f64,
    pub frame_time_ms: f64,
    pub estimated_fps: f64,
    pub throughput_score: f64,
    pub target_fps_achieved: bool,
    pub in_accordance: bool,
}

/// Comprehensive benchmark report comparing profiles and tracking settings impact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub tier: HardwareTier,
    pub cpu_cores: usize,
    pub gpu_name: String,
    pub low_profile: BenchmarkScore,
    pub recommended_profile: BenchmarkScore,
    pub ultra_profile: BenchmarkScore,
    pub settings_impact: BTreeMap<String, f64>,
    pub accordance_summary: String,
}

/// Performance benchmark suite to evaluate video settings impact and system accordance.
pub struct VideoBenchmarkSuite;

impl VideoBenchmarkSuite {
    /// Evaluates simulated render and simulation frame timings for a settings profile on hardware.
    pub fn evaluate(
        settings: &AdvancedVideoSettings,
        hdr: HdrMode,
        hardware: &SystemRecommendation,
    ) -> BenchmarkScore {
        let tier_factor = match hardware.tier {
            HardwareTier::Ultra => 0.45,
            HardwareTier::High => 0.70,
            HardwareTier::Medium => 1.20,
            HardwareTier::Low => 2.20,
        };

        // GPU Workload simulation
        let base_gpu = 1.5;
        let model_gpu = match settings.model_detail {
            ModelDetail::Low => 0.4,
            ModelDetail::Medium => 0.9,
            ModelDetail::High => 1.6,
        };
        let tex_gpu = match settings.texture_detail {
            TextureDetail::Low => 0.4,
            TextureDetail::Medium => 0.8,
            TextureDetail::High => 1.3,
            TextureDetail::VeryHigh => 1.8,
        };
        let aniso_gpu = match settings.filtering {
            Filtering::Bilinear => 0.2,
            Filtering::Trilinear => 0.3,
            Filtering::Anisotropic2 => 0.5,
            Filtering::Anisotropic4 => 0.8,
            Filtering::Anisotropic8 => 1.1,
            Filtering::Anisotropic16 => 1.5,
        };
        let shader_gpu = match settings.shader_detail {
            ShaderDetail::Low => 0.6,
            ShaderDetail::High => 1.8,
            ShaderDetail::VeryHigh => 2.6,
        };
        let water_gpu = match settings.water_detail {
            WaterDetail::Simple => 0.3,
            WaterDetail::World => 0.9,
            WaterDetail::All => 1.8,
        };
        let shadow_gpu = match settings.shadow_detail {
            ShadowDetail::Low => 0.2,
            ShadowDetail::Medium => 0.8,
            ShadowDetail::High => 1.8,
        };
        let aa_gpu = match settings.antialiasing {
            Antialiasing::None => 0.0,
            Antialiasing::X2 => 0.8,
            Antialiasing::X4 => 1.8,
            Antialiasing::X8 => 3.6,
        };
        let hdr_gpu = match hdr {
            HdrMode::None => 0.0,
            HdrMode::Bloom => 0.5,
            HdrMode::Full => 1.2,
        };
        let post_gpu = (if settings.color_correction { 0.3 } else { 0.0 })
            + (if settings.motion_blur { 0.4 } else { 0.0 });

        let total_gpu = (base_gpu
            + model_gpu
            + tex_gpu
            + aniso_gpu
            + shader_gpu
            + water_gpu
            + shadow_gpu
            + aa_gpu
            + hdr_gpu
            + post_gpu)
            * tier_factor;

        // CPU Simulation & Draw Submission Workload
        let base_cpu = 4.0;
        let model_cpu = match settings.model_detail {
            ModelDetail::Low => 0.3,
            ModelDetail::Medium => 0.7,
            ModelDetail::High => 1.4,
        };

        let total_cpu = if settings.multicore && hardware.cpu.is_multicore() {
            let cores = hardware.cpu.logical_cores as f64;
            let speedup = f64::min(4.0, 1.0 + 0.55 * (cores - 1.0));
            (base_cpu + model_cpu) / speedup + 0.25
        } else {
            base_cpu + model_cpu
        };

        let presentation_overhead = 0.5;
        let unquantized_time = f64::max(total_cpu, total_gpu) + presentation_overhead;

        // VSync quantizes frame presentation to display refresh cycles (60Hz = 16.67ms increments)
        let frame_time_ms = if settings.vsync {
            let refresh_interval = 16.6667;
            (unquantized_time / refresh_interval).ceil() * refresh_interval
        } else {
            unquantized_time
        };

        let estimated_fps = 1000.0 / frame_time_ms;
        let throughput_score = 10000.0 / frame_time_ms;
        let target_fps_achieved = estimated_fps >= 59.5; // Within 60 FPS standard game budget

        // Accordance: settings strike an optimal balance for the tier without dropping below budget
        let in_accordance = target_fps_achieved;

        BenchmarkScore {
            cpu_time_ms: (total_cpu * 100.0).round() / 100.0,
            gpu_time_ms: (total_gpu * 100.0).round() / 100.0,
            frame_time_ms: (frame_time_ms * 100.0).round() / 100.0,
            estimated_fps: (estimated_fps * 10.0).round() / 10.0,
            throughput_score: (throughput_score * 10.0).round() / 10.0,
            target_fps_achieved,
            in_accordance,
        }
    }

    /// Evaluates isolated performance deltas (in milliseconds) across individual video settings.
    pub fn setting_impact(rec: &SystemRecommendation) -> BTreeMap<String, f64> {
        let mut impacts = BTreeMap::new();
        let base_settings = rec.video.clone();
        let base_score = Self::evaluate(&base_settings, rec.hdr, rec);

        // MSAA Impact (8x vs None)
        let mut no_aa = base_settings.clone();
        no_aa.antialiasing = Antialiasing::None;
        let mut x8_aa = base_settings.clone();
        x8_aa.antialiasing = Antialiasing::X8;
        let aa_delta = Self::evaluate(&x8_aa, rec.hdr, rec).frame_time_ms
            - Self::evaluate(&no_aa, rec.hdr, rec).frame_time_ms;
        impacts.insert(
            "msaa_8x_vs_none_ms".to_string(),
            (aa_delta * 100.0).round() / 100.0,
        );

        // Filtering Impact (Anisotropic 16x vs Bilinear)
        let mut bilinear = base_settings.clone();
        bilinear.filtering = Filtering::Bilinear;
        let mut aniso16 = base_settings.clone();
        aniso16.filtering = Filtering::Anisotropic16;
        let aniso_delta = Self::evaluate(&aniso16, rec.hdr, rec).frame_time_ms
            - Self::evaluate(&bilinear, rec.hdr, rec).frame_time_ms;
        impacts.insert(
            "aniso_16x_vs_bilinear_ms".to_string(),
            (aniso_delta * 100.0).round() / 100.0,
        );

        // Shadow Detail Impact (High vs Low)
        let mut low_shadow = base_settings.clone();
        low_shadow.shadow_detail = ShadowDetail::Low;
        let mut high_shadow = base_settings.clone();
        high_shadow.shadow_detail = ShadowDetail::High;
        let shadow_delta = Self::evaluate(&high_shadow, rec.hdr, rec).frame_time_ms
            - Self::evaluate(&low_shadow, rec.hdr, rec).frame_time_ms;
        impacts.insert(
            "shadow_high_vs_low_ms".to_string(),
            (shadow_delta * 100.0).round() / 100.0,
        );

        // Water Detail Impact (Reflect All vs Simple)
        let mut simple_water = base_settings.clone();
        simple_water.water_detail = WaterDetail::Simple;
        let mut all_water = base_settings.clone();
        all_water.water_detail = WaterDetail::All;
        let water_delta = Self::evaluate(&all_water, rec.hdr, rec).frame_time_ms
            - Self::evaluate(&simple_water, rec.hdr, rec).frame_time_ms;
        impacts.insert(
            "water_all_vs_simple_ms".to_string(),
            (water_delta * 100.0).round() / 100.0,
        );

        // Multicore CPU Benefit (Disabled vs Enabled)
        let mut single_core = base_settings.clone();
        single_core.multicore = false;
        let mut multi_core = base_settings.clone();
        multi_core.multicore = true;
        let multicore_benefit = Self::evaluate(&single_core, rec.hdr, rec).cpu_time_ms
            - Self::evaluate(&multi_core, rec.hdr, rec).cpu_time_ms;
        impacts.insert(
            "multicore_cpu_gain_ms".to_string(),
            (multicore_benefit * 100.0).round() / 100.0,
        );

        // HDR Impact (Full vs None)
        let hdr_delta = Self::evaluate(&base_settings, HdrMode::Full, rec).frame_time_ms
            - Self::evaluate(&base_settings, HdrMode::None, rec).frame_time_ms;
        impacts.insert(
            "hdr_full_vs_none_ms".to_string(),
            (hdr_delta * 100.0).round() / 100.0,
        );

        // Shader Detail Impact (High vs Low)
        let mut low_shader = base_settings.clone();
        low_shader.shader_detail = ShaderDetail::Low;
        let mut high_shader = base_settings;
        high_shader.shader_detail = ShaderDetail::High;
        let shader_delta = Self::evaluate(&high_shader, rec.hdr, rec).frame_time_ms
            - Self::evaluate(&low_shader, rec.hdr, rec).frame_time_ms;
        impacts.insert(
            "shader_high_vs_low_ms".to_string(),
            (shader_delta * 100.0).round() / 100.0,
        );

        impacts.insert(
            "recommended_frame_time_ms".to_string(),
            base_score.frame_time_ms,
        );
        impacts.insert("recommended_fps".to_string(), base_score.estimated_fps);

        impacts
    }

    /// Executes full profile benchmarks and produces an accordance report.
    pub fn run_preset_benchmarks(rec: &SystemRecommendation) -> BenchmarkReport {
        let (low_video, low_hdr) =
            SystemRecommendation::settings_for_tier(HardwareTier::Low, &rec.cpu, &rec.gpu);
        let (ultra_video, ultra_hdr) =
            SystemRecommendation::settings_for_tier(HardwareTier::Ultra, &rec.cpu, &rec.gpu);

        let low_profile = Self::evaluate(&low_video, low_hdr, rec);
        let recommended_profile = Self::evaluate(&rec.video, rec.hdr, rec);
        let ultra_profile = Self::evaluate(&ultra_video, ultra_hdr, rec);

        let settings_impact = Self::setting_impact(rec);

        let accordance_summary = format!(
            "Hardware: {} (Tier: {:?}, {} cores). Recommended Profile: {:.1} FPS ({:.2} ms), Target Achieved: {}. Accordance: {}.",
            rec.gpu.name,
            rec.tier,
            rec.cpu.logical_cores,
            recommended_profile.estimated_fps,
            recommended_profile.frame_time_ms,
            recommended_profile.target_fps_achieved,
            recommended_profile.in_accordance,
        );

        BenchmarkReport {
            tier: rec.tier,
            cpu_cores: rec.cpu.logical_cores,
            gpu_name: rec.gpu.name.clone(),
            low_profile,
            recommended_profile,
            ultra_profile,
            settings_impact,
            accordance_summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_recommendation_tier_inference_across_hardware_specs() {
        // Low tier: single core or software renderer
        let low_cpu = CpuSpecs::new(1);
        let gpu_discrete = GpuSpecs::new("GeForce GTX 1060", GpuDeviceType::Discrete);
        let rec_low_core = SystemRecommendation::from_specs(low_cpu, gpu_discrete.clone());
        assert_eq!(rec_low_core.tier, HardwareTier::Low);
        assert!(!rec_low_core.video.multicore);
        assert_eq!(rec_low_core.video.model_detail, ModelDetail::Low);
        assert_eq!(rec_low_core.hdr, HdrMode::None);

        let soft_gpu = GpuSpecs::new("llvmpipe (LLVM 15.0)", GpuDeviceType::Cpu);
        let rec_soft = SystemRecommendation::from_specs(CpuSpecs::new(8), soft_gpu);
        assert_eq!(rec_soft.tier, HardwareTier::Low);

        // Medium tier: 4-core with integrated GPU
        let intel_gpu = GpuSpecs::new("Intel Iris Xe Graphics", GpuDeviceType::Integrated);
        let rec_med = SystemRecommendation::from_specs(CpuSpecs::new(4), intel_gpu);
        assert_eq!(rec_med.tier, HardwareTier::Medium);
        assert!(rec_med.video.multicore);
        assert_eq!(rec_med.video.model_detail, ModelDetail::Medium);
        assert_eq!(rec_med.video.antialiasing, Antialiasing::X2);
        assert_eq!(rec_med.video.filtering, Filtering::Anisotropic4);

        // High tier: Apple Silicon / 4-core discrete GPU
        let apple_gpu = GpuSpecs::new("Apple M1", GpuDeviceType::Integrated);
        let rec_high_apple = SystemRecommendation::from_specs(CpuSpecs::new(8), apple_gpu);
        assert_eq!(rec_high_apple.tier, HardwareTier::High);
        assert_eq!(rec_high_apple.video.model_detail, ModelDetail::High);
        assert_eq!(rec_high_apple.video.antialiasing, Antialiasing::X4);
        assert_eq!(rec_high_apple.video.filtering, Filtering::Anisotropic8);
        assert_eq!(rec_high_apple.hdr, HdrMode::Full);

        // Ultra tier: 16-core with RTX 4090
        let rtx4090 = GpuSpecs::new("NVIDIA GeForce RTX 4090", GpuDeviceType::Discrete);
        let rec_ultra = SystemRecommendation::from_specs(CpuSpecs::new(16), rtx4090);
        assert_eq!(rec_ultra.tier, HardwareTier::Ultra);
        assert_eq!(rec_ultra.video.texture_detail, TextureDetail::VeryHigh);
        assert_eq!(rec_ultra.video.filtering, Filtering::Anisotropic16);
        assert_eq!(rec_ultra.video.antialiasing, Antialiasing::X8);
        assert_eq!(rec_ultra.hdr, HdrMode::Full);
    }

    #[test]
    fn recommended_combo_indices_match_expected_settings() {
        let rtx = GpuSpecs::new("GeForce RTX 3080", GpuDeviceType::Discrete);
        let rec = SystemRecommendation::from_specs(CpuSpecs::new(8), rtx);
        assert_eq!(rec.tier, HardwareTier::Ultra);

        assert_eq!(
            rec.recommended_advanced_index(AdvancedChoice::ModelDetail),
            2,
            "High model detail is index 2"
        );
        assert_eq!(
            rec.recommended_advanced_index(AdvancedChoice::TextureDetail),
            3,
            "Very High texture detail is index 3"
        );
        assert_eq!(
            rec.recommended_advanced_index(AdvancedChoice::ShaderDetail),
            1,
            "High shader detail is index 1"
        );
        assert_eq!(
            rec.recommended_advanced_index(AdvancedChoice::WaterDetail),
            2,
            "Reflect All is index 2"
        );
        assert_eq!(
            rec.recommended_advanced_index(AdvancedChoice::ShadowDetail),
            2,
            "High shadows is index 2"
        );
        assert_eq!(
            rec.recommended_advanced_index(AdvancedChoice::Antialiasing),
            3,
            "8X MSAA is index 3"
        );
        assert_eq!(
            rec.recommended_advanced_index(AdvancedChoice::Filtering),
            5,
            "Anisotropic 16X is index 5"
        );
        assert_eq!(
            rec.recommended_advanced_index(AdvancedChoice::Multicore),
            1,
            "Enabled is index 1"
        );
        assert_eq!(
            rec.recommended_advanced_index(AdvancedChoice::VSync),
            0,
            "Disabled is index 0"
        );
        assert_eq!(rec.recommended_hdr_index(), 2, "Full HDR is index 2");

        assert!(rec.is_recommended_advanced(AdvancedChoice::ModelDetail, 2));
        assert!(!rec.is_recommended_advanced(AdvancedChoice::ModelDetail, 0));
        assert!(rec.is_recommended_hdr(2));
        assert!(!rec.is_recommended_hdr(0));
    }

    #[test]
    fn performance_benchmark_suite_evaluates_profiles_and_tracks_accordance() {
        let gpu = GpuSpecs::new("Apple M2 Pro", GpuDeviceType::Integrated);
        let rec = SystemRecommendation::from_specs(CpuSpecs::new(10), gpu);

        let report = VideoBenchmarkSuite::run_preset_benchmarks(&rec);
        assert_eq!(report.tier, HardwareTier::High);
        assert_eq!(report.cpu_cores, 10);

        // Recommended profile meets target performance budget
        assert!(report.recommended_profile.target_fps_achieved);
        assert!(report.recommended_profile.in_accordance);
        assert!(
            report.recommended_profile.estimated_fps >= 60.0,
            "Recommended settings should maintain at least 60 FPS: got {:.1}",
            report.recommended_profile.estimated_fps
        );

        // Low profile has highest frame rate
        assert!(
            report.low_profile.estimated_fps >= report.recommended_profile.estimated_fps,
            "Low settings should yield higher or equal FPS"
        );

        // Settings impact metrics are populated and positive
        assert!(report.settings_impact.contains_key("msaa_8x_vs_none_ms"));
        assert!(report
            .settings_impact
            .contains_key("aniso_16x_vs_bilinear_ms"));
        assert!(report.settings_impact.contains_key("shadow_high_vs_low_ms"));
        assert!(report
            .settings_impact
            .contains_key("water_all_vs_simple_ms"));
        assert!(report.settings_impact.contains_key("multicore_cpu_gain_ms"));

        let multicore_gain = report.settings_impact["multicore_cpu_gain_ms"];
        assert!(
            multicore_gain > 0.0,
            "Multicore rendering must yield positive CPU frame time savings"
        );

        let shadow_cost = report.settings_impact["shadow_high_vs_low_ms"];
        assert!(
            shadow_cost >= 0.0,
            "High shadows cost should be non-negative"
        );
    }

    #[test]
    fn hardware_aware_antialiasing_supported_modes_and_gating() {
        // 1. Discrete GPU (Vulkan/DX12) - supports up to 8X
        let discrete = GpuSpecs::new("GeForce RTX 4080", GpuDeviceType::Discrete);
        assert_eq!(discrete.max_msaa_samples(), 8);
        assert_eq!(discrete.max_antialiasing(), Antialiasing::X8);
        assert_eq!(
            discrete.supported_antialiasing(),
            vec![
                Antialiasing::None,
                Antialiasing::X2,
                Antialiasing::X4,
                Antialiasing::X8
            ]
        );

        // 2. Apple Silicon / Metal - capped to 4X
        let apple = GpuSpecs::new("Apple M1 Pro", GpuDeviceType::Integrated);
        assert_eq!(apple.max_msaa_samples(), 4);
        assert_eq!(apple.max_antialiasing(), Antialiasing::X4);
        assert_eq!(
            apple.supported_antialiasing(),
            vec![Antialiasing::None, Antialiasing::X2, Antialiasing::X4]
        );

        // Apple Silicon at Ultra tier (e.g. M1 Ultra) clamps recommended antialiasing to X4
        let apple_ultra = GpuSpecs::new("Apple M1 Ultra", GpuDeviceType::Integrated);
        let rec_apple = SystemRecommendation::from_specs(CpuSpecs::new(24), apple_ultra);
        assert_eq!(rec_apple.tier, HardwareTier::Ultra);
        assert_eq!(rec_apple.video.antialiasing, Antialiasing::X4);
        assert_eq!(
            rec_apple.recommended_advanced_index(AdvancedChoice::Antialiasing),
            2,
            "4X MSAA is index 2 among [None, 2X, 4X]"
        );

        // 3. CPU Software Renderer (llvmpipe) - capped to 2X
        let cpu_gpu = GpuSpecs::new("llvmpipe", GpuDeviceType::Cpu);
        assert_eq!(cpu_gpu.max_msaa_samples(), 2);
        assert_eq!(cpu_gpu.max_antialiasing(), Antialiasing::X2);
        assert_eq!(
            cpu_gpu.supported_antialiasing(),
            vec![Antialiasing::None, Antialiasing::X2]
        );
    }
}
