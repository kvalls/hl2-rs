//! Video modes, the game frame and its presentation in the window.
//!
//! The configured video resolution is the resolution of the *game frame*: the sky, world,
//! viewmodel and HUD/menu cameras all render into one offscreen image of exactly that size.
//! A presentation camera then draws the frame into the window with linear filtering, scaled
//! to fit and centred (letterbox/pillarbox when the aspect ratios differ). The window size,
//! the display's native resolution and its scale factor therefore never change the rendering
//! resolution: full screen at 1280x720 renders 1280x720 pixels and upscales them to the
//! display, and a macOS window put in full screen with the green title-bar button behaves
//! the same way.
//!
//! In windowed mode, applying a resolution also sizes the window's client area to it (in
//! logical points, clamped to the monitor), as Source sizes a windowed game to its mode.

use bevy::camera::{RenderTarget, Viewport, visibility::RenderLayers};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::window::{
    Monitor, MonitorSelection, PrimaryMonitor, PrimaryWindow, Window, WindowMode, WindowPosition,
};

pub mod display {
    use super::*;

    /// Metrics and bounds for the primary display.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct DisplayMetrics {
        pub physical_width: u32,
        pub physical_height: u32,
        pub scale_factor: f32,
    }

    impl DisplayMetrics {
        /// Metrics of a monitor.
        pub fn of(monitor: &Monitor) -> Self {
            Self {
                physical_width: monitor.physical_width,
                physical_height: monitor.physical_height,
                scale_factor: monitor.scale_factor as f32,
            }
        }

        /// Queries display metrics from the primary monitor, if available.
        #[allow(dead_code)]
        pub fn from_monitors(monitors: &Query<&Monitor, With<PrimaryMonitor>>) -> Option<Self> {
            monitors.iter().next().map(Self::of)
        }

        /// Detects if a window currently in `WindowMode::Windowed` is actually occupying the full
        /// display (such as macOS native workspace fullscreen via the green zoom button).
        pub fn is_workspace_fullscreen(&self, window: &Window) -> bool {
            let tolerance = (window.scale_factor().max(1.0).ceil() as u32).max(2);
            window.mode == WindowMode::Windowed
                && window
                    .resolution
                    .physical_width()
                    .abs_diff(self.physical_width)
                    <= tolerance
                && window
                    .resolution
                    .physical_height()
                    .abs_diff(self.physical_height)
                    <= tolerance
        }
    }

    /// Resolutions offered on the Video page: the primary monitor's reported video modes plus
    /// the standard list, filtered and sorted by [`hl2_ui::gameui::display::usable_modes`], and
    /// the desktop size in pixels that windowed choices must fit (windowed mode shows one frame
    /// pixel per display pixel). Falls back to the current window size when nothing is usable.
    /// A `--fake-monitor` spec replaces the monitor's size and reported modes.
    pub fn modes(
        monitors: &Query<&Monitor, With<PrimaryMonitor>>,
        window: &Window,
        fake: Option<&hl2_ui::gameui::display::MonitorSpec>,
    ) -> hl2_ui::gameui::display::DisplayModes {
        use hl2_ui::gameui::display::{DisplayModes, Resolution, usable_modes};
        let current = Resolution::new(
            window.resolution.physical_width(),
            window.resolution.physical_height(),
        );
        if let Some(fake) = fake {
            return fake.display_modes(current);
        }
        let monitor = monitors.iter().next();
        let reported = monitor
            .into_iter()
            .flat_map(|m| m.video_modes.iter())
            .map(|mode| Resolution::new(mode.physical_size.x, mode.physical_size.y));
        let size = monitor.map(|m| Resolution::new(m.physical_width, m.physical_height));
        DisplayModes {
            fullscreen: usable_modes(reported, size, current),
            desktop: size,
            current: Some(current),
        }
    }
}

pub mod mode {
    use super::*;
    use hl2_ui::config::DisplayMode;

    /// The Bevy window mode for a configured display mode on this platform.
    ///
    /// Full screen is exclusive full screen in the desktop's display mode: the game frame renders
    /// at the configured resolution and is scaled to the display. "Windowed (No Border)" is a
    /// borderless window covering the current monitor (on macOS the native full-screen Space,
    /// where the menu bar and title bar show when the pointer reaches the top edge).
    pub fn desired_mode(mode: DisplayMode) -> WindowMode {
        on_monitor(mode, MonitorSelection::Current)
    }

    /// [`desired_mode`] for window creation, where no current monitor exists yet.
    pub fn startup_mode(mode: DisplayMode) -> WindowMode {
        on_monitor(mode, MonitorSelection::Primary)
    }

    fn on_monitor(mode: DisplayMode, monitor: MonitorSelection) -> WindowMode {
        match mode.effective() {
            // The current video mode: the panel keeps its scaling (no 1x mode, normal-sized
            // cursor on macOS); the game frame is scaled to it.
            DisplayMode::Fullscreen => {
                WindowMode::Fullscreen(monitor, bevy::window::VideoModeSelection::Current)
            }
            DisplayMode::Windowed => WindowMode::Windowed,
            DisplayMode::NoBorder => WindowMode::BorderlessFullscreen(monitor),
        }
    }
}

/// Render layer of the presentation pass (only the frame quad and its camera).
pub const PRESENT_LAYER: usize = 31;
/// The presentation camera draws after every game-frame camera.
const PRESENT_ORDER: isize = 100;
/// Frames during which a windowed-mode resize is re-requested until the OS applies it (window
/// mode transitions, such as leaving full screen, finish asynchronously).
const WINDOW_RESIZE_FRAMES: u32 = 120;
/// Room kept around a windowed window that would not fit the monitor (title bar, menu bar,
/// task bar), in logical points (scaled to pixels by the display's scale factor).
///
/// On macOS, system chrome includes the menu bar (~33-37 pt), title bar (~28-32 pt), and
/// dock (~65-80 pt). An oversized window must be clamped with at least 160 pt vertically so it
/// fits within `NSScreen.visibleFrame` without OS clamping/desync.
#[cfg(target_os = "macos")]
const WINDOW_MARGIN: Vec2 = Vec2::new(48.0, 160.0);
#[cfg(not(target_os = "macos"))]
const WINDOW_MARGIN: Vec2 = Vec2::new(32.0, 96.0);

/// Frames during which game-frame rendering is suspended around a frame resize.
///
/// Bevy recomputes a camera's render-target size when it reads the image's `AssetEvent`,
/// which arrives a frame after the image changes, while the GPU texture is replaced at once.
/// Rendering through that frame pairs the new color texture with depth textures of the old
/// size (a wgpu validation error that quits the app). The frame cameras are therefore
/// stopped first, the image is resized while nothing renders into it, and rendering resumes
/// once the cameras have picked up the new size:
/// - `SUSPEND_FRAMES`: cameras stopped;
/// - `SUSPEND_FRAMES - 1`: the image is resized;
/// - remaining frames: the size change reaches the cameras; then rendering resumes.
const SUSPEND_FRAMES: u32 = 4;

/// The offscreen game frame: every scene, viewmodel and HUD camera renders into it.
#[derive(Resource, Clone, Debug)]
pub struct GameFrame {
    pub image: Handle<Image>,
    /// Frame size in pixels: the configured video resolution, or for a flexible frame the size
    /// [`hl2_ui::flexible_frame::flexible_frame_resolution`] derives from it and the window.
    pub resolution: UVec2,
    /// Configured video resolution, in frame pixels (also the requested windowed size).
    pub configured: UVec2,
    /// The frame follows the window's aspect ratio (Video > Aspect Ratio "Fit Display").
    pub flexible: bool,
    /// Remaining frames to size a windowed-mode window to `configured`.
    pub pending_window: Option<u32>,
    /// Remaining frames of suspended rendering while the frame is resized.
    pub suspended: u32,
    /// Place the window in the [`DesktopArea`] once a windowed resize is done (macOS).
    pub recenter: bool,
}

impl GameFrame {
    /// The frame size for `window`: the configured resolution, or for a flexible frame the
    /// window-shaped size of the same detail. A window with no area (minimized) keeps the
    /// current size.
    pub fn size_for(&self, window: &Window) -> UVec2 {
        let physical = UVec2::new(
            window.resolution.physical_width(),
            window.resolution.physical_height(),
        );
        if !self.flexible {
            self.configured.max(UVec2::ONE)
        } else if physical.x == 0 || physical.y == 0 {
            self.resolution
        } else {
            let size = hl2_ui::flexible_frame::flexible_frame_resolution(
                glam::UVec2::new(self.configured.x, self.configured.y),
                glam::UVec2::new(physical.x, physical.y),
            );
            UVec2::new(size.x, size.y)
        }
    }
}

/// The part of the window's monitor a window may cover (macOS `NSScreen.visibleFrame`: the
/// screen without the menu bar and Dock), in logical points with the origin at the top-left of
/// the main display, as winit places windows. Read by [`read_desktop_area`] on macOS only; other
/// platforms keep centring on the whole monitor.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct DesktopArea {
    pub visible: Rect,
    /// Height of the window's title bar in points (outer frame minus client area).
    pub title_bar: f32,
}

impl DesktopArea {
    /// The largest client area, in physical pixels at `scale`, whose window fits the area.
    pub fn client_room(&self, scale: f32) -> Vec2 {
        (self.visible.size() - Vec2::new(0., self.title_bar)).max(Vec2::ONE) * scale
    }

    /// Outer top-left position, in physical pixels at `scale` (what `WindowPosition::At` takes),
    /// that centres a window with a `client` physical-pixel client area in the area. A window
    /// larger than the area keeps its top-left corner inside it.
    pub fn centered(&self, client: UVec2, scale: f32) -> IVec2 {
        let scale = scale.max(f32::EPSILON);
        let outer = client.as_vec2() / scale + Vec2::new(0., self.title_bar);
        let free = (self.visible.size() - outer).max(Vec2::ZERO);
        ((self.visible.min + free / 2.).round() * scale)
            .round()
            .as_ivec2()
    }
}

/// The camera presenting the game frame in the window.
#[derive(Component)]
pub struct PresentCamera;

/// The quad showing the game frame.
#[derive(Component)]
pub struct PresentQuad;

/// `mat_monitorgamma` at which the presented frame is unchanged (the SDK's 2.2 display gamma).
pub const NEUTRAL_MONITOR_GAMMA: f32 = 2.2;

/// Exponent of the monitor gamma ramp applied to display-encoded values: `v^(gamma / 2.2)`.
/// The Options slider runs from 1.6 ("Light") to 2.6 ("Dark") in the installed
/// `OptionsSubVideoGammaDlg.res`, so lower values brighten. Retail sets a hardware gamma ramp,
/// which only works in full screen; the presentation shader applies it in every display mode.
pub fn monitor_gamma_exponent(gamma: f32) -> f32 {
    gamma.clamp(1.6, 2.6) / NEUTRAL_MONITOR_GAMMA
}

/// Draws the game frame (3D view, HUD and menus) in the window through the monitor gamma ramp.
#[derive(Asset, TypePath, bevy::render::render_resource::AsBindGroup, Debug, Clone)]
pub struct PresentMaterial {
    /// x: [`monitor_gamma_exponent`].
    #[uniform(0)]
    pub params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub frame: Handle<Image>,
}

impl bevy::sprite_render::Material2d for PresentMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/present.wgsl".into()
    }
}

/// A game-frame camera stopped during a frame resize, with its `is_active` before.
#[derive(Component)]
pub struct SuspendedCamera(bool);

/// A render-target image for the game frame, linearly filtered when scaled.
pub fn frame_image(resolution: UVec2) -> Image {
    let size = resolution.max(UVec2::ONE);
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}

/// Creates the game frame and its presentation. `windowed` sizes the window to the frame
/// (clamped to the monitor) once the monitor is known; `flexible` makes the frame follow the
/// window's aspect ratio from the first presented frame on.
pub(crate) fn install_frame(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    present: &mut Assets<PresentMaterial>,
    resolution: UVec2,
    flexible: bool,
    windowed: bool,
) {
    let resolution = resolution.max(UVec2::ONE);
    let image = images.add(frame_image(resolution));
    commands.insert_resource(GameFrame {
        image: image.clone(),
        resolution,
        configured: resolution,
        flexible,
        pending_window: windowed.then_some(WINDOW_RESIZE_FRAMES),
        suspended: 0,
        recenter: windowed,
    });
    commands.spawn((
        PresentCamera,
        Camera2d,
        Camera {
            order: PRESENT_ORDER,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        RenderLayers::layer(PRESENT_LAYER),
    ));
    // A unit square scaled to the presented size by `present_frame`.
    commands.spawn((
        PresentQuad,
        Mesh2d(meshes.add(Rectangle::new(1., 1.))),
        bevy::sprite_render::MeshMaterial2d(present.add(PresentMaterial {
            params: Vec4::new(monitor_gamma_exponent(NEUTRAL_MONITOR_GAMMA), 0., 0., 0.),
            frame: image,
        })),
        Transform::from_scale(resolution.as_vec2().extend(1.)),
        RenderLayers::layer(PRESENT_LAYER),
    ));
}

/// Keeps the present material's gamma ramp on the displayed `mat_monitorgamma`: the applied
/// setting, or the brightness dialog's value while it is open (live preview, as in retail).
pub(crate) fn update_present_gamma(
    ui: Option<Res<crate::ui_host::Console>>,
    quads: Query<&bevy::sprite_render::MeshMaterial2d<PresentMaterial>, With<PresentQuad>>,
    mut materials: ResMut<Assets<PresentMaterial>>,
) {
    let Some(ui) = ui else {
        return;
    };
    let exponent = monitor_gamma_exponent(ui.source.display_gamma());
    for handle in &quads {
        let current = materials.get(&handle.0).map(|m| m.params.x);
        if current.is_some_and(|x| x != exponent)
            && let Some(mut material) = materials.get_mut(&handle.0)
        {
            material.params.x = exponent;
        }
    }
}

/// Placement of the presented game frame in the window.
///
/// `target_resolution` is the game-frame resolution (what the HUD/menu canvas is laid out in);
/// the rects describe where the scaled frame appears inside the window, with black stripes
/// (letterbox or pillarbox) when the aspect ratios differ.
#[derive(Resource, Clone, Debug)]
pub struct VideoViewport {
    /// Game-frame resolution configured in video settings (e.g. 1920x1080 or 1024x768).
    pub target_resolution: UVec2,
    /// Presented frame rect in physical window pixels.
    pub physical_rect: Viewport,
    /// Top-left offset of the presented frame in logical window coordinates.
    pub logical_offset: Vec2,
    /// Presented frame size in logical window coordinates.
    pub logical_size: Vec2,
}

impl PartialEq for VideoViewport {
    fn eq(&self, other: &Self) -> bool {
        self.target_resolution == other.target_resolution
            && self.physical_rect.physical_position == other.physical_rect.physical_position
            && self.physical_rect.physical_size == other.physical_rect.physical_size
            && self.logical_offset == other.logical_offset
            && self.logical_size == other.logical_size
    }
}

impl Default for VideoViewport {
    fn default() -> Self {
        Self {
            target_resolution: UVec2::new(1280, 720),
            physical_rect: Viewport {
                physical_position: UVec2::ZERO,
                physical_size: UVec2::new(1280, 720),
                depth: 0.0..1.0,
            },
            logical_offset: Vec2::ZERO,
            logical_size: Vec2::new(1280.0, 720.0),
        }
    }
}

impl VideoViewport {
    /// Computes where a frame of the target resolution is presented inside a window.
    #[allow(dead_code)]
    pub fn compute(target_w: u32, target_h: u32, window: &Window) -> Self {
        Self::compute_with_mode(target_w, target_h, window, false)
    }

    /// [`Self::compute`] for a fixed or flexible frame
    /// ([`source_ui::viewport::ViewportBounds::compute_with_mode`]).
    pub fn compute_with_mode(
        target_w: u32,
        target_h: u32,
        window: &Window,
        flexible: bool,
    ) -> Self {
        let win_w = window.resolution.physical_width().max(1);
        let win_h = window.resolution.physical_height().max(1);
        let scale = window.scale_factor().max(1.0);

        let bounds = hl2_ui::flexible_frame::compute_with_mode(
            glam::UVec2::new(target_w, target_h),
            glam::UVec2::new(win_w, win_h),
            scale,
            flexible,
        );

        let physical_rect = Viewport {
            physical_position: UVec2::new(bounds.physical_offset.x, bounds.physical_offset.y),
            physical_size: UVec2::new(bounds.physical_size.x, bounds.physical_size.y),
            depth: 0.0..1.0,
        };

        Self {
            target_resolution: UVec2::new(bounds.target_resolution.x, bounds.target_resolution.y),
            physical_rect,
            logical_offset: Vec2::new(bounds.logical_offset.x, bounds.logical_offset.y),
            logical_size: Vec2::new(bounds.logical_size.x, bounds.logical_size.y),
        }
    }

    /// Returns the portable viewport bounds.
    pub fn bounds(&self) -> source_ui::viewport::ViewportBounds {
        source_ui::viewport::ViewportBounds {
            target_resolution: glam::UVec2::new(self.target_resolution.x, self.target_resolution.y),
            physical_offset: glam::UVec2::new(
                self.physical_rect.physical_position.x,
                self.physical_rect.physical_position.y,
            ),
            physical_size: glam::UVec2::new(
                self.physical_rect.physical_size.x,
                self.physical_rect.physical_size.y,
            ),
            logical_offset: glam::Vec2::new(self.logical_offset.x, self.logical_offset.y),
            logical_size: glam::Vec2::new(self.logical_size.x, self.logical_size.y),
        }
    }

    /// Whether black stripes are present (aspect ratio mismatch).
    #[allow(dead_code)]
    pub fn has_black_stripes(&self) -> bool {
        self.bounds().has_black_stripes()
    }

    /// True if vertical black stripes (pillarboxing) on left/right sides.
    #[allow(dead_code)]
    pub fn is_vertical_stripes(&self) -> bool {
        self.bounds().is_vertical_stripes()
    }

    /// True if horizontal black stripes (letterboxing) on top/bottom sides.
    #[allow(dead_code)]
    pub fn is_horizontal_stripes(&self) -> bool {
        self.bounds().is_horizontal_stripes()
    }
}

/// Applies applied video settings: the window mode at once, the game-frame resolution, and in
/// windowed mode a window resize carried out by [`resize_window`].
fn apply_video_config_to_window(
    window: &mut Window,
    cfg: &hl2_ui::config::VideoSettings,
    frame: Option<&mut GameFrame>,
) {
    let target_mode = mode::desired_mode(cfg.display_mode);
    debug!(
        "[video] apply {}x{} {:?}: window mode {:?} -> {:?}, window {}x{} (scale {})",
        cfg.width,
        cfg.height,
        cfg.display_mode,
        window.mode,
        target_mode,
        window.resolution.width(),
        window.resolution.height(),
        window.scale_factor()
    );
    if window.mode != target_mode {
        window.mode = target_mode;
    }
    if let Some(frame) = frame {
        let mut configured = UVec2::new(cfg.width, cfg.height).max(UVec2::ONE);
        if cfg.flexible_aspect {
            let physical = UVec2::new(
                window.resolution.physical_width(),
                window.resolution.physical_height(),
            );
            if physical.x > 0 && physical.y > 0 {
                let size = hl2_ui::flexible_frame::flexible_presented_resolution(glam::UVec2::new(
                    physical.x, physical.y,
                ));
                let native = UVec2::new(size.x, size.y);
                configured = UVec2::new(configured.x.max(native.x), configured.y.max(native.y));
            }
        }
        frame.configured = configured;
        frame.flexible = cfg.flexible_aspect;
        frame.resolution = frame.size_for(window);
        frame.pending_window =
            (target_mode == WindowMode::Windowed).then_some(WINDOW_RESIZE_FRAMES);
    }
}

/// Carries out video settings applied from the Options dialog or the console.
/// The swap-chain present mode: `--uncapped` always presents without waiting; otherwise
/// Video > Advanced "Wait for vertical sync" (`mat_vsync`) decides.
pub fn present_mode(uncapped: bool, vsync: bool) -> bevy::window::PresentMode {
    if vsync && !uncapped {
        bevy::window::PresentMode::AutoVsync
    } else {
        bevy::window::PresentMode::AutoNoVsync
    }
}

/// Keeps the window's present mode on the configured V-sync setting. It touches only the present
/// mode, never resolution or display mode, and writes the window only when the mode differs.
pub(crate) fn sync_present_mode(
    options: Res<crate::Options>,
    ui: Option<Res<crate::ui_host::Console>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let vsync = ui
        .as_deref()
        .is_none_or(|ui| ui.source.config.video.advanced.vsync);
    let mode = present_mode(options.uncapped, vsync);
    // Reading through `Mut` does not mark the window changed; only the assignment does.
    if let Ok(mut window) = windows.single_mut()
        && window.present_mode != mode
    {
        window.present_mode = mode;
    }
}

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

/// Keeps every scene camera's `Msaa` in sync with the configured antialiasing mode.
/// The 2D presentation camera remains `Msaa::Off` because it displays the resolved frame quad.
pub(crate) fn sync_antialiasing(
    ui: Option<Res<crate::ui_host::Console>>,
    adapter: Option<Res<bevy::render::renderer::RenderAdapterInfo>>,
    mut cameras: Query<&mut Msaa, (With<Camera>, Without<PresentCamera>)>,
) {
    let Some(ui) = ui else {
        return;
    };
    let max_samples = adapter_max_msaa_samples(adapter.as_deref());
    let want = msaa_for_hardware(ui.source.config.video.advanced.antialiasing, max_samples);
    for mut msaa in &mut cameras {
        if *msaa != want {
            *msaa = want;
        }
    }
}

/// Updates texture sampler descriptors on active scene images to reflect the configured filtering mode.
pub fn apply_filtering_to_images(images: &mut Assets<Image>, filtering: hl2_ui::config::Filtering) {
    let mipmap_filter = if filtering.is_trilinear() {
        ImageFilterMode::Linear
    } else {
        ImageFilterMode::Nearest
    };
    let aniso = filtering.aniso();

    for (_id, image) in images.iter_mut() {
        if let ImageSampler::Descriptor(ref mut desc) = image.sampler {
            // Apply filtering mode to 3D scene textures (repeat mode, linear mag filter).
            // Preserve nearest-filter glyph caches and 1x presentation quad samplers.
            if desc.mag_filter == ImageFilterMode::Linear
                && desc.anisotropy_clamp > 0
                && (desc.mipmap_filter != mipmap_filter || desc.anisotropy_clamp != aniso)
            {
                desc.mipmap_filter = mipmap_filter;
                desc.anisotropy_clamp = aniso;
            }
        }
    }
}

/// Keeps active material texture samplers in step with `mat_trilinear` / `mat_forceaniso`.
pub(crate) fn sync_filtering(
    ui: Option<Res<crate::ui_host::Console>>,
    mut images: ResMut<Assets<Image>>,
    mut last_filtering: Local<Option<hl2_ui::config::Filtering>>,
) {
    let Some(ui) = ui else {
        return;
    };
    let current = ui.source.config.video.advanced.filtering;
    if *last_filtering == Some(current) {
        return;
    }
    *last_filtering = Some(current);
    apply_filtering_to_images(&mut images, current);
}

pub(crate) fn apply_video_settings(
    mut sim: ResMut<crate::simulation::Simulation>,
    mut frame: Option<ResMut<GameFrame>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Some(cfg) = sim.pending_video_config.take() else {
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    apply_video_config_to_window(&mut window, &cfg, frame.as_deref_mut());
}

/// What a pending windowed-mode resize should do this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowResize {
    /// The window left windowed mode; drop the request.
    Cancel,
    /// The window fills the display in OS full screen (macOS green button); keep the request
    /// for when it returns to a normal window, and leave it alone meanwhile.
    Wait,
    /// The window already has the requested size.
    Done,
    /// Request this physical (pixel) client size.
    Resize(UVec2),
}

/// Decides a windowed-mode resize to `resolution` in physical pixels (one frame pixel per display
/// pixel, also on high-DPI displays), clamped to the monitor.
#[cfg(test)]
pub fn window_resize(
    window: &Window,
    monitor: Option<&Monitor>,
    resolution: UVec2,
) -> WindowResize {
    window_resize_in(window, monitor, resolution, None, false)
}

/// [`window_resize`] with the monitor's usable [`DesktopArea`] when it is known (macOS): the
/// client size then fits that area exactly instead of the monitor minus [`WINDOW_MARGIN`].
/// A `flexible` frame fits each side on its own ([`hl2_ui::flexible_frame::flexible_windowed_size`])
/// instead of shrinking the requested shape uniformly.
pub fn window_resize_in(
    window: &Window,
    monitor: Option<&Monitor>,
    resolution: UVec2,
    area: Option<&DesktopArea>,
    flexible: bool,
) -> WindowResize {
    if window.mode != WindowMode::Windowed {
        return WindowResize::Cancel;
    }
    let metrics = monitor.map(display::DisplayMetrics::of);
    if metrics.is_some_and(|m| m.is_workspace_fullscreen(window)) {
        return WindowResize::Wait;
    }
    let scale = window.scale_factor().max(1.0);
    let sized = |room: Option<Vec2>| {
        let resolution = glam::UVec2::new(resolution.x, resolution.y);
        let room = room.map(|room| glam::Vec2::new(room.x, room.y));
        let size = if flexible {
            hl2_ui::flexible_frame::flexible_windowed_size(resolution, room)
        } else {
            source_ui::viewport::windowed_size(resolution, room)
        };
        Vec2::new(size.x, size.y)
    };
    let available = metrics.map(|m| Vec2::new(m.physical_width as f32, m.physical_height as f32));
    let mut size = match (area, available) {
        (Some(area), _) => sized(Some(area.client_room(scale))),
        (None, Some(available)) => sized(Some(available)),
        (None, None) => sized(None),
    };
    if area.is_none()
        && let Some(available) = available
        && (size.x >= available.x || size.y >= available.y)
    {
        let room = (available - WINDOW_MARGIN * scale).max(Vec2::ONE);
        size = sized(Some(room));
    }
    let size = UVec2::new(size.x as u32, size.y as u32).max(UVec2::ONE);
    let current = UVec2::new(
        window.resolution.physical_width(),
        window.resolution.physical_height(),
    );
    // The OS sizes windows in points: allow the rounding of one point.
    let tolerance = scale.ceil() as u32;
    if current.x.abs_diff(size.x) <= tolerance && current.y.abs_diff(size.y) <= tolerance {
        WindowResize::Done
    } else {
        WindowResize::Resize(size)
    }
}

/// Carries out a pending windowed-mode resize, re-requesting it while the OS finishes a
/// window-mode transition. A window in OS full screen is left alone.
pub(crate) fn resize_window(
    frame: Option<ResMut<GameFrame>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    monitors: Query<&Monitor, With<PrimaryMonitor>>,
    area: Option<Res<DesktopArea>>,
) {
    let Some(mut frame) = frame else {
        return;
    };
    let Some(frames) = frame.pending_window else {
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let monitor = monitors.iter().next();
    if monitor.is_none() && frames > 1 {
        // The monitor appears a few frames after window creation; clamp against it.
        frame.pending_window = Some(frames - 1);
        return;
    }
    let area = area.as_deref();
    let decision = window_resize_in(&window, monitor, frame.configured, area, frame.flexible);
    debug!(
        "[video] resize_window frames={frames} mode={:?} window={}x{} px -> {decision:?}",
        window.mode,
        window.resolution.physical_width(),
        window.resolution.physical_height()
    );
    match decision {
        WindowResize::Cancel => {
            frame.pending_window = None;
            frame.recenter = false;
        }
        WindowResize::Done => {
            frame.pending_window = None;
            // macOS places the window once it has its size: winit turns a position into AppKit
            // coordinates with the window's current height, and Bevy applies a position before a
            // size in the same frame, so a combined request lands off by the height change.
            if std::mem::take(&mut frame.recenter)
                && let Some(area) = area
            {
                let client = UVec2::new(
                    window.resolution.physical_width(),
                    window.resolution.physical_height(),
                );
                let at = area.centered(client, window.scale_factor());
                debug!("[video] window placed in the desktop area {area:?} at {at}");
                window.position = WindowPosition::At(at);
            }
        }
        WindowResize::Wait => {}
        WindowResize::Resize(size) => {
            window.resolution.set_physical_resolution(size.x, size.y);
            if area.is_some() {
                frame.recenter = true;
            } else {
                window.position = WindowPosition::Centered(MonitorSelection::Current);
            }
            frame.pending_window = frames.checked_sub(1).filter(|left| *left > 0);
        }
    }
}

/// The window mode matching an OS full-screen state the user changed outside the game (macOS
/// green button), or `None` when the window mode already agrees.
///
/// On macOS "Windowed (No Border)" is the native full-screen Space ([`mode::desired_mode`] maps
/// it to `BorderlessFullscreen`), so entering native full screen from a windowed game means No
/// Border, and leaving full screen (either kind) means Windowed.
#[cfg(any(target_os = "macos", test))]
pub fn native_fullscreen_sync(mode: WindowMode, native_fullscreen: bool) -> Option<WindowMode> {
    match (mode, native_fullscreen) {
        (WindowMode::Windowed, true) => {
            Some(WindowMode::BorderlessFullscreen(MonitorSelection::Current))
        }
        (WindowMode::BorderlessFullscreen(_) | WindowMode::Fullscreen(..), false) => {
            Some(WindowMode::Windowed)
        }
        _ => None,
    }
}

/// Recognizes macOS native full screen entered or left with the window's green button.
///
/// Reads winit's real full-screen state (main thread only, read-only) and reacts only when that
/// state changes: transitions the game starts itself reach winit asynchronously and then agree
/// with `Window::mode`, so they are never reverted. A user transition updates the window mode,
/// the applied configuration's display mode (saved, so the next launch uses it) and drops any
/// pending windowed resize.
#[cfg(target_os = "macos")]
pub(crate) fn sync_native_fullscreen(
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut last: Local<Option<bool>>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut ui: ResMut<crate::ui_host::Console>,
    mut sim: ResMut<crate::simulation::Simulation>,
    frame: Option<ResMut<GameFrame>>,
) {
    let Ok((entity, mut window)) = windows.single_mut() else {
        return;
    };
    let Some(native) = bevy::winit::WINIT_WINDOWS.with_borrow(|windows| {
        windows
            .get_window(entity)
            .map(|winit| winit.fullscreen().is_some())
    }) else {
        return;
    };
    // Act only on a change after the first observation.
    if last.replace(native) != Some(!native) {
        return;
    }
    let Some(mode) = native_fullscreen_sync(window.mode, native) else {
        return;
    };
    debug!(
        "[video] native full screen {native}: window mode {:?} -> {mode:?}",
        window.mode
    );
    window.mode = mode;
    if let Some(mut frame) = frame {
        frame.pending_window = None;
    }
    let display_mode = if native {
        hl2_ui::config::DisplayMode::NoBorder
    } else {
        hl2_ui::config::DisplayMode::Windowed
    };
    // The window now carries this mode, so a later unrelated OK/Apply must not re-mode it.
    if let Some(applied) = sim.applied_window.as_mut() {
        applied.display_mode = display_mode;
    }
    if ui.source.config.video.display_mode.effective() != display_mode {
        ui.source.set_display_mode_external(display_mode);
        if let Err(error) = ui.source.config.save() {
            ui.source.log(format!("Could not save settings: {error}"));
        }
    }
}

/// Reads the [`DesktopArea`] of the window's screen while a windowed resize is pending (macOS:
/// `NSScreen.visibleFrame` of the screen the window is on, and the window's title bar height).
#[cfg(target_os = "macos")]
pub(crate) fn read_desktop_area(
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut commands: Commands,
    frame: Option<Res<GameFrame>>,
    windows: Query<&bevy::window::RawHandleWrapper, With<PrimaryWindow>>,
) {
    if frame.is_none_or(|f| f.pending_window.is_none()) {
        return;
    }
    let Ok(handle) = windows.single() else {
        return;
    };
    let raw_window_handle::RawWindowHandle::AppKit(appkit) = handle.get_window_handle() else {
        return;
    };
    let Some(main_thread) = objc2::MainThreadMarker::new() else {
        return;
    };
    match macos_desktop_area(appkit.ns_view, main_thread) {
        Some(area) => commands.insert_resource(area),
        None => commands.remove_resource::<DesktopArea>(),
    }
}

#[cfg(target_os = "macos")]
fn macos_desktop_area(
    view: std::ptr::NonNull<std::ffi::c_void>,
    main_thread: objc2::MainThreadMarker,
) -> Option<DesktopArea> {
    use objc2_app_kit::{NSScreen, NSView};
    // SAFETY: the AppKit window handle is the window's content `NSView`, which lives as long as
    // the window entity it was read from; this runs on the main thread.
    let view: &NSView = unsafe { view.cast::<NSView>().as_ref() };
    let window = view.window()?;
    let screen = window.screen()?;
    // Window coordinates count down from the top of the main (menu bar) display.
    let main_height = NSScreen::screens(main_thread)
        .firstObject()?
        .frame()
        .size
        .height;
    let visible = screen.visibleFrame();
    let frame = window.frame();
    let content = window.contentRectForFrameRect(frame);
    let top = main_height - visible.origin.y - visible.size.height;
    let area = DesktopArea {
        visible: Rect::new(
            visible.origin.x as f32,
            top as f32,
            (visible.origin.x + visible.size.width) as f32,
            (top + visible.size.height) as f32,
        ),
        title_bar: (frame.size.height - content.size.height).max(0.) as f32,
    };
    (area.visible.width() >= 1. && area.visible.height() >= 1.).then_some(area)
}

type FrameCameras<'w, 's> = Query<
    'w,
    's,
    (&'static mut Camera, &'static mut RenderTarget),
    (
        Without<crate::monitors::MonitorCamera>,
        Without<PresentCamera>,
    ),
>;

/// Routes every game camera into the game frame and places the presented frame in the window.
/// A flexible frame takes the window's current shape here first, so live window resizes (and
/// display changes) reshape it; [`suspend_for_resize`] then resizes the image safely.
pub(crate) fn present_frame(
    frame: Option<ResMut<GameFrame>>,
    mut video_viewport: ResMut<VideoViewport>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cameras: FrameCameras,
    mut quads: Query<(&mut Transform, &mut Visibility), With<PresentQuad>>,
) {
    let Some(mut frame) = frame else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    if frame.flexible {
        let size = frame.size_for(window);
        if frame.resolution != size {
            debug!(
                "[video] flexible frame {} -> {size} for window {}x{} px",
                frame.resolution,
                window.resolution.physical_width(),
                window.resolution.physical_height()
            );
            frame.resolution = size;
        }
    }
    for (mut camera, mut target) in &mut cameras {
        let routed = matches!(&*target, RenderTarget::Image(t) if t.handle == frame.image);
        if !routed {
            *target = RenderTarget::Image(frame.image.clone().into());
        }
        if camera.viewport.is_some() {
            camera.viewport = None;
        }
    }
    let computed = VideoViewport::compute_with_mode(
        frame.resolution.x,
        frame.resolution.y,
        window,
        frame.flexible,
    );
    if *video_viewport != computed {
        debug!(
            "[video] viewport updated: target={}x{} window={}x{} px (scale {}) -> physical_rect={:?} logical_offset={:?} logical_size={:?}",
            frame.resolution.x,
            frame.resolution.y,
            window.resolution.physical_width(),
            window.resolution.physical_height(),
            window.scale_factor(),
            computed.physical_rect,
            computed.logical_offset,
            computed.logical_size
        );
        *video_viewport = computed;
    }
    // The frame holds no complete picture while it is being resized: show black.
    let visibility = if frame.suspended > 0 {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for (mut quad, mut shown) in &mut quads {
        let scale = video_viewport.logical_size.extend(1.);
        if quad.scale != scale {
            quad.scale = scale;
        }
        if *shown != visibility {
            *shown = visibility;
        }
    }
}

type SuspendCameras<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Camera,
        Option<&'static SuspendedCamera>,
    ),
    (
        Without<crate::monitors::MonitorCamera>,
        Without<PresentCamera>,
    ),
>;

/// Resizes the game frame safely when the configured resolution changes (see
/// [`SUSPEND_FRAMES`]). Runs after every system that sets camera activity this frame, so a
/// stopped camera cannot be re-enabled before rendering.
pub(crate) fn suspend_for_resize(
    mut commands: Commands,
    frame: Option<ResMut<GameFrame>>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: SuspendCameras,
    quads: Query<&bevy::sprite_render::MeshMaterial2d<PresentMaterial>, With<PresentQuad>>,
    mut materials: ResMut<Assets<PresentMaterial>>,
) {
    let Some(mut frame) = frame else {
        return;
    };
    let size = images.get(&frame.image).map(Image::size);
    if frame.suspended == 0 && size.is_some_and(|s| s != frame.resolution) {
        frame.suspended = SUSPEND_FRAMES;
    }
    if frame.suspended == 0 {
        for (entity, mut camera, suspended) in &mut cameras {
            if let Some(SuspendedCamera(was_active)) = suspended {
                camera.is_active = *was_active;
                if let Ok(mut cmd) = commands.get_entity(entity) {
                    cmd.remove::<SuspendedCamera>();
                }
            }
        }
        return;
    }
    for (entity, mut camera, suspended) in &mut cameras {
        if let (None, Ok(mut cmd)) = (suspended, commands.get_entity(entity)) {
            cmd.insert(SuspendedCamera(camera.is_active));
        }
        camera.is_active = false;
    }
    if frame.suspended == SUSPEND_FRAMES - 1 {
        debug!(
            "[video] resizing frame image {:?} -> {}",
            images.get(&frame.image).map(Image::size),
            frame.resolution
        );
        // A new image rather than the old one resized in place: the present material's bind
        // group keeps the texture it was built with, so the window would go on showing the old
        // frame. `present_frame` routes the cameras to the new image.
        let image = images.add(frame_image(frame.resolution));
        let old = std::mem::replace(&mut frame.image, image.clone());
        for handle in &quads {
            if let Some(mut material) = materials.get_mut(&handle.0) {
                material.frame = image.clone();
            }
        }
        images.remove(&old);
    }
    frame.suspended -= 1;
    if frame.suspended == 0 {
        debug!("[video] frame resize done: resuming rendering");
    }
}

/// Keeps [`crate::simulation::Simulation::frame_size`] on the size of the frame image.
pub(crate) fn record_frame_size(
    frame: Option<Res<GameFrame>>,
    images: Res<Assets<Image>>,
    mut sim: ResMut<crate::simulation::Simulation>,
) {
    let size = frame
        .and_then(|frame| images.get(&frame.image).map(Image::size))
        .map(|size| [size.x, size.y]);
    if sim.frame_size != size {
        sim.frame_size = size;
    }
}

/// Source's field of view is horizontal at 4:3. Wider screens keep the vertical view of 4:3 and
/// show more at the sides (as in retail); screens narrower than 4:3 (square, portrait) keep the
/// 4:3 horizontal view and show more above and below, so the view and the viewmodel are never
/// cropped at the sides. Returns Bevy's vertical field of view (radians) for `aspect`
/// (width / height) of the game frame.
pub(crate) fn vertical_fov_for(degrees: f32, aspect: f32) -> f32 {
    2. * ((degrees.to_radians() / 2.).tan() / aspect.min(4. / 3.)).atan()
}

/// [`vertical_fov_for`] with an explicit vertical field of view or automatic aspect tuning:
/// when `vertical > 0.`, `vertical` degrees replace the derived one for the player's default
/// field of view; when `vertical <= 0.` (automatic), [`hl2_ui::tuning::world_fov_offset`]
/// adjusts `default_degrees` outside `5:4..=16:9` so portrait frames avoid vertical fish-eye
/// and ultrawide frames avoid peripheral horizontal stretching. Weapon zoom or camera field of
/// view `view_degrees` scales the tangent in proportion.
pub(crate) fn vertical_fov_with(
    view_degrees: f32,
    default_degrees: f32,
    aspect: f32,
    vertical: f32,
) -> f32 {
    let derived = vertical_fov_for(view_degrees, aspect);
    let base = vertical_fov_for(default_degrees, aspect);
    let target_base = if vertical > 0. {
        vertical.to_radians()
    } else {
        let offset = hl2_ui::tuning::world_fov_offset(aspect);
        if offset == 0. {
            return derived;
        }
        vertical_fov_for((default_degrees + offset).clamp(1., 160.), aspect)
    };
    let tangent = (derived / 2.).tan() * (target_base / 2.).tan() / (base / 2.).tan();
    2. * tangent.atan()
}

/// [`vertical_fov_for`] on a 4:3 frame.
#[cfg(test)]
pub(crate) fn vertical_fov(degrees: f32) -> f32 {
    vertical_fov_for(degrees, 4. / 3.)
}

/// Width / height of the game frame (4:3 before the frame exists).
pub(crate) fn frame_aspect(frame: Option<&GameFrame>) -> f32 {
    frame.map_or(4. / 3., |f| {
        f.resolution.x as f32 / f.resolution.y.max(1) as f32
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::window::{PrimaryWindow, WindowMode, WindowResolution};

    #[test]
    fn an_explicit_vertical_fov_replaces_the_derived_one_and_scales_with_zoom() {
        for aspect in [5. / 4., 4. / 3., 16. / 10., 16. / 9.] {
            assert_eq!(
                vertical_fov_with(75., 75., aspect, 0.),
                vertical_fov_for(75., aspect),
                "retail aspects are untouched in automatic mode"
            );
        }
        // Outside 5:4..=16:9, automatic mode applies `world_fov_offset` to tame vertical fish-eye
        // in portrait and horizontal peripheral stretching in ultrawide.
        assert!(vertical_fov_with(75., 75., 9. / 16., 0.) < vertical_fov_for(75., 9. / 16.));
        assert!(vertical_fov_with(75., 75., 9. / 16., 0.) > vertical_fov_for(75., 4. / 3.));
        assert!(vertical_fov_with(75., 75., 32. / 9., 0.) < vertical_fov_for(75., 32. / 9.));

        for aspect in [9. / 16., 1., 4. / 3., 16. / 9., 32. / 9.] {
            let derived = vertical_fov_for(75., aspect);
            let auto = vertical_fov_with(75., 75., aspect, 0.);
            let explicit = vertical_fov_with(75., 75., aspect, 90.);
            assert!(
                (explicit.to_degrees() - 90.).abs() < 1e-3,
                "{aspect}: {explicit}"
            );
            // Zoom (a smaller view FOV) narrows both automatic and explicit vertical FOV in the
            // same proportion as the derived one.
            let derived_ratio = (vertical_fov_for(20., aspect) / 2.).tan() / (derived / 2.).tan();
            let auto_zoomed = vertical_fov_with(20., 75., aspect, 0.);
            let auto_ratio = (auto_zoomed / 2.).tan() / (auto / 2.).tan();
            assert!((auto_ratio - derived_ratio).abs() < 1e-5, "{aspect}");
            let zoomed = vertical_fov_with(20., 75., aspect, 90.);
            let ratio = (zoomed / 2.).tan() / (explicit / 2.).tan();
            assert!((ratio - derived_ratio).abs() < 1e-5, "{aspect}");
            assert!(zoomed < explicit);
        }
    }

    #[test]
    fn vsync_follows_the_setting_unless_uncapped() {
        use bevy::window::PresentMode;
        assert_eq!(present_mode(false, true), PresentMode::AutoVsync);
        assert_eq!(present_mode(false, false), PresentMode::AutoNoVsync);
        assert_eq!(present_mode(true, true), PresentMode::AutoNoVsync);
    }

    fn monitor(width: u32, height: u32, scale_factor: f64) -> Monitor {
        Monitor {
            name: Some("Test".into()),
            video_modes: vec![],
            physical_position: IVec2::ZERO,
            physical_width: width,
            physical_height: height,
            refresh_rate_millihertz: Some(60000),
            scale_factor,
        }
    }

    fn window(width: u32, height: u32, scale: f32, mode: WindowMode) -> Window {
        let mut resolution = WindowResolution::new(width, height);
        resolution.set_scale_factor(scale);
        Window {
            resolution,
            mode,
            ..default()
        }
    }

    fn frame(resolution: UVec2) -> GameFrame {
        GameFrame {
            image: Handle::default(),
            resolution,
            configured: resolution,
            flexible: false,
            pending_window: None,
            suspended: 0,
            recenter: false,
        }
    }

    #[test]
    fn applies_video_config_mode_safely() {
        let mut window = window(1920, 1080, 1.0, WindowMode::Windowed);
        let cfg = hl2_ui::config::VideoSettings {
            width: 1280,
            height: 720,
            display_mode: hl2_ui::config::DisplayMode::NoBorder,
            ..Default::default()
        };
        let mut frame = frame(UVec2::new(1920, 1080));
        apply_video_config_to_window(&mut window, &cfg, Some(&mut frame));
        if cfg!(target_os = "macos") {
            // No borderless choice on macOS: full screen, kept on the desktop's display mode.
            assert_eq!(
                window.mode,
                WindowMode::BorderlessFullscreen(MonitorSelection::Current)
            );
        } else {
            assert!(
                matches!(window.mode, WindowMode::BorderlessFullscreen(_)),
                "{:?}",
                window.mode
            );
        }
        assert_eq!(frame.resolution, UVec2::new(1280, 720));
        assert_eq!(
            frame.pending_window, None,
            "no window resize outside windowed"
        );
        // The OS window frame is not resized for full-screen modes.
        assert_eq!(window.resolution.physical_width(), 1920);
    }

    #[test]
    fn fit_display_replaces_a_low_fixed_frame_with_current_physical_resolution() {
        // Switching from 640x480 to Fit Display must not retain 480 lines just because that was
        // the previously selected fixed mode. On high-DPI panels `Window` reports physical
        // pixels, which are also the game frame's resolution.
        let mut window = window(
            3024,
            1964,
            2.0,
            WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        );
        let cfg = hl2_ui::config::VideoSettings {
            width: 640,
            height: 480,
            flexible_aspect: true,
            display_mode: hl2_ui::config::DisplayMode::NoBorder,
            ..Default::default()
        };
        let mut frame = frame(UVec2::new(640, 480));
        apply_video_config_to_window(&mut window, &cfg, Some(&mut frame));
        assert_eq!(frame.configured, UVec2::new(3024, 1964));
        assert_eq!(frame.resolution, UVec2::new(3024, 1964));
    }

    #[test]
    fn monitor_gamma_ramp_is_neutral_at_2_2_and_lighter_below() {
        assert_eq!(monitor_gamma_exponent(2.2), 1.);
        assert!(monitor_gamma_exponent(1.6) < 1., "1.6 is the Light end");
        assert!(monitor_gamma_exponent(2.6) > 1., "2.6 is the Dark end");
        // Mid grey under the ramp: lighter below 2.2, darker above.
        let v: f32 = 0.5;
        assert!(v.powf(monitor_gamma_exponent(1.6)) > v);
        assert!(v.powf(monitor_gamma_exponent(2.6)) < v);
        // Out-of-range stored values clamp to the slider range.
        assert_eq!(monitor_gamma_exponent(0.5), monitor_gamma_exponent(1.6));
    }

    #[test]
    fn display_modes_map_to_window_modes() {
        use hl2_ui::config::DisplayMode;
        assert_eq!(
            mode::desired_mode(DisplayMode::Windowed),
            WindowMode::Windowed
        );
        let fullscreen = mode::desired_mode(DisplayMode::Fullscreen);
        let no_border = mode::desired_mode(DisplayMode::NoBorder);
        // The same on every platform (macOS included): exclusive full screen in the current
        // video mode, and a borderless full-screen window.
        assert_eq!(
            fullscreen,
            WindowMode::Fullscreen(
                MonitorSelection::Current,
                bevy::window::VideoModeSelection::Current
            )
        );
        assert_eq!(
            no_border,
            WindowMode::BorderlessFullscreen(MonitorSelection::Current)
        );
    }

    #[test]
    fn windowed_mode_fits_and_centres_in_the_desktop_area() {
        // M1 Pro panel at 2x: 1728x1117 pt, menu bar 37 pt, Dock 70 pt at the bottom.
        let area = DesktopArea {
            visible: Rect::new(0., 37., 1728., 1047.),
            title_bar: 28.,
        };
        let retina = monitor(3456, 2234, 2.0);
        let window = window(2048, 1536, 2.0, WindowMode::Windowed);
        let bottom =
            |at: IVec2, client: UVec2| at.y as f32 / 2. + area.title_bar + client.y as f32 / 2.;

        // Small: requested size, centred in the visible area (not the whole screen).
        let client = UVec2::new(1280, 720);
        assert_eq!(
            window_resize_in(&window, Some(&retina), client, Some(&area), false),
            WindowResize::Resize(client)
        );
        assert_eq!(area.centered(client, 2.), IVec2::new(1088, 696));

        // Fits only below the Dock: kept, and clear of the Dock.
        let client = UVec2::new(2560, 1600);
        assert_eq!(
            window_resize_in(&window, Some(&retina), client, Some(&area), false),
            WindowResize::Resize(client)
        );
        let at = area.centered(client, 2.);
        assert_eq!(at, IVec2::new(448, 256));
        assert!(bottom(at, client) <= area.visible.max.y);

        // Taller than the visible area: scaled down to it, top at the menu bar, bottom at the Dock.
        let WindowResize::Resize(client) = window_resize_in(
            &window,
            Some(&retina),
            UVec2::new(3456, 2160),
            Some(&area),
            false,
        ) else {
            panic!("a resize");
        };
        assert_eq!(
            client.y, 1964,
            "visible height minus the title bar, in pixels"
        );
        let at = area.centered(client, 2.);
        assert_eq!(at.y, 74, "just below the menu bar");
        assert!(bottom(at, client) <= area.visible.max.y);

        // Flexible frame: each side is limited to the area on its own without scaling the aspect ratio.
        let WindowResize::Resize(client) = window_resize_in(
            &window,
            Some(&retina),
            UVec2::new(3456, 2160),
            Some(&area),
            true,
        ) else {
            panic!("a resize");
        };
        assert_eq!(client, UVec2::new(3456, 1964));
        let at = area.centered(client, 2.);
        assert_eq!(at, IVec2::new(0, 74), "fills width and sits below menu bar");
        assert!(bottom(at, client) <= area.visible.max.y);
        let at = area.centered(client, 2.);
        assert_eq!(at.y, 74, "just below the menu bar");
        assert!(
            bottom(at, client) <= area.visible.max.y,
            "not under the Dock"
        );
        assert!(at.x >= 0);

        // A Dock on the left shifts the centre right.
        let left_dock = DesktopArea {
            visible: Rect::new(70., 37., 1728., 1117.),
            title_bar: 28.,
        };
        assert_eq!(
            left_dock.centered(UVec2::new(1280, 720), 2.).x,
            (70 + (1658 - 640) / 2) * 2
        );
    }

    #[test]
    fn windowed_resolution_requests_a_window_resize() {
        let mut window = window(1920, 1080, 1.0, WindowMode::Windowed);
        let cfg = hl2_ui::config::VideoSettings {
            width: 1024,
            height: 768,
            display_mode: hl2_ui::config::DisplayMode::Windowed,
            ..Default::default()
        };
        let mut frame = frame(UVec2::new(1920, 1080));
        apply_video_config_to_window(&mut window, &cfg, Some(&mut frame));
        assert_eq!(frame.resolution, UVec2::new(1024, 768));
        assert!(frame.pending_window.is_some());
        let monitor = monitor(2560, 1440, 1.0);
        assert_eq!(
            window_resize(&window, Some(&monitor), frame.resolution),
            WindowResize::Resize(UVec2::new(1024, 768))
        );
    }

    #[test]
    fn windowed_size_is_one_frame_pixel_per_display_pixel() {
        // Retina 3024x1964 at 2x: a 1280x720 window is 1280x720 pixels (640x360 points), so the
        // frame is shown 1:1 without scaling.
        let window = window(1000, 600, 2.0, WindowMode::Windowed);
        let retina = monitor(3024, 1964, 2.0);
        assert_eq!(
            window_resize(&window, Some(&retina), UVec2::new(1280, 720)),
            WindowResize::Resize(UVec2::new(1280, 720))
        );
        let sized = self::window(1280, 720, 2.0, WindowMode::Windowed);
        assert_eq!(
            window_resize(&sized, Some(&retina), UVec2::new(1280, 720)),
            WindowResize::Done
        );
        let rounded = self::window(1282, 720, 2.0, WindowMode::Windowed);
        assert_eq!(
            window_resize(&rounded, Some(&retina), UVec2::new(1281, 720)),
            WindowResize::Done,
            "one point of OS rounding is accepted"
        );
        assert_eq!(
            window_resize(&window, Some(&retina), UVec2::new(2560, 1600)),
            WindowResize::Resize(UVec2::new(2560, 1600)),
            "large Retina resolutions fit in windowed mode"
        );
    }

    #[test]
    fn oversized_windowed_resolution_is_clamped_to_the_monitor() {
        // The panel's native 3024x1964 does not fit with a title bar: the window shrinks
        // uniformly inside the monitor, keeping the frame's aspect ratio.
        let window = window(1000, 600, 2.0, WindowMode::Windowed);
        let retina = monitor(3024, 1964, 2.0);
        let WindowResize::Resize(size) =
            window_resize(&window, Some(&retina), UVec2::new(3024, 1964))
        else {
            panic!("expected a resize");
        };
        assert!(size.x <= 3024 - 64 && size.y <= 1964 - 192, "{size}");
        assert!(
            (size.x as f32 / size.y as f32 - 3024.0 / 1964.0).abs() < 0.01,
            "{size}"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn oversized_retina_windowed_resolution_clamps_cleanly_on_macos() {
        let window = window(1000, 600, 2.0, WindowMode::Windowed);
        let retina = monitor(3456, 2234, 2.0);

        // 3456x2160 (16:10) is not falsely detected as workspace fullscreen.
        let metrics = display::DisplayMetrics::of(&retina);
        let win_2160 = self::window(3456, 2160, 2.0, WindowMode::Windowed);
        assert!(!metrics.is_workspace_fullscreen(&win_2160));

        // Clamps cleanly to fit within macOS visibleFrame.
        let WindowResize::Resize(size_2160) =
            window_resize(&window, Some(&retina), UVec2::new(3456, 2160))
        else {
            panic!("expected a resize");
        };
        assert!(
            size_2160.y <= 1978,
            "must fit inside macOS visible frame: {size_2160}"
        );

        let WindowResize::Resize(size_2234) =
            window_resize(&window, Some(&retina), UVec2::new(3456, 2234))
        else {
            panic!("expected a resize");
        };
        assert!(
            size_2234.y <= 1978,
            "must fit inside macOS visible frame: {size_2234}"
        );
    }

    #[test]
    fn native_full_screen_changes_map_to_the_game_modes() {
        let full = WindowMode::BorderlessFullscreen(MonitorSelection::Current);
        // Green button on a windowed game: Windowed (No Border).
        assert_eq!(
            native_fullscreen_sync(WindowMode::Windowed, true),
            Some(full)
        );
        // Green button out of No Border or exclusive full screen: Windowed.
        assert_eq!(
            native_fullscreen_sync(full, false),
            Some(WindowMode::Windowed)
        );
        assert_eq!(
            native_fullscreen_sync(
                WindowMode::Fullscreen(
                    MonitorSelection::Current,
                    bevy::window::VideoModeSelection::Current
                ),
                false
            ),
            Some(WindowMode::Windowed)
        );
        // Already agreeing: nothing to do.
        assert_eq!(native_fullscreen_sync(full, true), None);
        assert_eq!(native_fullscreen_sync(WindowMode::Windowed, false), None);
    }

    #[test]
    fn os_full_screen_window_is_left_alone_until_it_returns() {
        // macOS green-button full screen keeps WindowMode::Windowed but fills the display.
        let retina = monitor(3024, 1964, 2.0);
        let window = window(3024, 1964, 2.0, WindowMode::Windowed);
        assert_eq!(
            window_resize(&window, Some(&retina), UVec2::new(1280, 720)),
            WindowResize::Wait
        );
        let full = self::window(
            3024,
            1964,
            2.0,
            WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        );
        assert_eq!(
            window_resize(&full, Some(&retina), UVec2::new(1280, 720)),
            WindowResize::Cancel
        );
    }

    #[test]
    fn pillarbox_vertical_stripes_for_narrow_aspect_on_wide_window() {
        let window = window(1920, 1080, 1.0, WindowMode::Windowed);
        // 4:3 frame presented in a 16:9 window.
        let vp = VideoViewport::compute(1024, 768, &window);
        assert!(vp.has_black_stripes());
        assert!(vp.is_vertical_stripes());
        assert!(!vp.is_horizontal_stripes());
        // 1080 * (4/3) = 1440 width. Remaining 480 split as 240 left & right.
        assert_eq!(vp.physical_rect.physical_size, UVec2::new(1440, 1080));
        assert_eq!(vp.physical_rect.physical_position, UVec2::new(240, 0));
        assert_eq!(vp.logical_size, Vec2::new(1440.0, 1080.0));
        assert_eq!(vp.logical_offset, Vec2::new(240.0, 0.0));
    }

    #[test]
    fn letterbox_horizontal_stripes_for_wide_aspect_on_narrow_window() {
        let window = window(1024, 768, 1.0, WindowMode::Windowed);
        // 16:9 frame presented in a 4:3 window.
        let vp = VideoViewport::compute(1920, 1080, &window);
        assert!(vp.has_black_stripes());
        assert!(!vp.is_vertical_stripes());
        assert!(vp.is_horizontal_stripes());
        // 1024 / (16/9) = 576 height. Remaining 192 split as 96 top & bottom.
        assert_eq!(vp.physical_rect.physical_size, UVec2::new(1024, 576));
        assert_eq!(vp.physical_rect.physical_position, UVec2::new(0, 96));
        assert_eq!(vp.logical_size, Vec2::new(1024.0, 576.0));
        assert_eq!(vp.logical_offset, Vec2::new(0.0, 96.0));
    }

    #[test]
    fn exact_aspect_ratio_fills_entire_frame() {
        let window = window(1920, 1080, 1.0, WindowMode::Windowed);
        // A 1280x720 frame is upscaled to fill a 1920x1080 window.
        let vp = VideoViewport::compute(1280, 720, &window);
        assert!(!vp.has_black_stripes());
        assert_eq!(vp.target_resolution, UVec2::new(1280, 720));
        assert_eq!(vp.physical_rect.physical_size, UVec2::new(1920, 1080));
        assert_eq!(vp.physical_rect.physical_position, UVec2::ZERO);
        assert_eq!(vp.logical_size, Vec2::new(1920.0, 1080.0));
        assert_eq!(vp.logical_offset, Vec2::ZERO);
    }

    #[test]
    fn viewport_with_scale_factor_calculates_correct_logical_offset() {
        let window = window(3840, 2160, 2.0, WindowMode::Windowed);
        // 4:3 frame on a 16:9 4K Retina display
        let vp = VideoViewport::compute(1024, 768, &window);
        // Physical: 2160 * (4/3) = 2880 width. Offset = (3840 - 2880) / 2 = 480.
        assert_eq!(vp.physical_rect.physical_size, UVec2::new(2880, 2160));
        assert_eq!(vp.physical_rect.physical_position, UVec2::new(480, 0));
        // Logical: divided by scale factor 2.0
        assert_eq!(vp.logical_size, Vec2::new(1440.0, 1080.0));
        assert_eq!(vp.logical_offset, Vec2::new(240.0, 0.0));
    }

    #[test]
    fn game_cameras_render_into_the_frame_at_the_configured_resolution() {
        let mut app = App::new();
        app.init_resource::<VideoViewport>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<PresentMaterial>>();
        let image = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(frame_image(UVec2::new(640, 480)));
        app.insert_resource(GameFrame {
            image: image.clone(),
            resolution: UVec2::new(1280, 720),
            configured: UVec2::new(1280, 720),
            flexible: false,
            pending_window: None,
            suspended: 0,
            recenter: false,
        });
        // Full screen on a Retina panel.
        app.world_mut().spawn((
            window(
                3024,
                1964,
                2.0,
                WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            ),
            PrimaryWindow,
        ));
        let scene_camera = app
            .world_mut()
            .spawn((
                Camera {
                    viewport: Some(Viewport::default()),
                    ..default()
                },
                RenderTarget::default(),
            ))
            .id();
        let monitor_camera = app
            .world_mut()
            .spawn((
                crate::monitors::MonitorCamera,
                Camera::default(),
                RenderTarget::default(),
            ))
            .id();
        let present_camera = app
            .world_mut()
            .spawn((PresentCamera, Camera::default(), RenderTarget::default()))
            .id();
        let present_material = app
            .world_mut()
            .resource_mut::<Assets<PresentMaterial>>()
            .add(PresentMaterial {
                params: Vec4::ONE,
                frame: image.clone(),
            });
        let quad = app
            .world_mut()
            .spawn((
                PresentQuad,
                bevy::sprite_render::MeshMaterial2d(present_material.clone()),
                Transform::default(),
                Visibility::default(),
            ))
            .id();

        app.world_mut().run_system_once(present_frame).unwrap();

        let target = app.world().get::<RenderTarget>(scene_camera).unwrap();
        assert!(matches!(target, RenderTarget::Image(t) if t.handle == image));
        assert!(
            app.world()
                .get::<Camera>(scene_camera)
                .unwrap()
                .viewport
                .is_none()
        );
        assert!(matches!(
            app.world().get::<RenderTarget>(monitor_camera).unwrap(),
            RenderTarget::Window(_)
        ));
        assert!(matches!(
            app.world().get::<RenderTarget>(present_camera).unwrap(),
            RenderTarget::Window(_)
        ));
        // 16:9 frame on a 3024x1964 panel: full width, letterboxed, in logical points.
        let vp = app.world().resource::<VideoViewport>().clone();
        assert_eq!(vp.target_resolution, UVec2::new(1280, 720));
        assert_eq!(vp.physical_rect.physical_size, UVec2::new(3024, 1701));
        let placed = app.world().get::<Transform>(quad).unwrap();
        assert_eq!(placed.scale.truncate(), vp.logical_size);

        // The resize: cameras stop first, the image changes only while they are stopped, and
        // they resume with their previous activity once the new size has had time to arrive.
        // The frame becomes a new image that the window's material shows: a material keeps
        // the texture it was built with, so resizing the old image in place froze the picture.
        let frame_handle = |app: &App| app.world().resource::<GameFrame>().image.clone();
        let size = |app: &App| {
            app.world()
                .resource::<Assets<Image>>()
                .get(&frame_handle(app))
                .map(Image::size)
        };
        let active = |app: &App, camera| app.world().get::<Camera>(camera).unwrap().is_active;
        app.world_mut().run_system_once(suspend_for_resize).unwrap();
        assert!(!active(&app, scene_camera), "stopped before the resize");
        assert!(active(&app, monitor_camera), "monitor camera untouched");
        assert_eq!(size(&app), Some(UVec2::new(640, 480)), "not resized yet");
        app.world_mut().run_system_once(suspend_for_resize).unwrap();
        assert_eq!(
            size(&app),
            Some(UVec2::new(1280, 720)),
            "resized while stopped"
        );
        let resized = frame_handle(&app);
        assert_ne!(resized, image, "a new frame image");
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&image)
                .is_none(),
            "the old one is released"
        );
        assert_eq!(
            app.world()
                .resource::<Assets<PresentMaterial>>()
                .get(&present_material)
                .map(|m| m.frame.clone()),
            Some(resized.clone()),
            "the window shows the new image"
        );
        assert!(!active(&app, scene_camera));
        app.world_mut().run_system_once(present_frame).unwrap();
        assert_eq!(
            app.world().get::<Visibility>(quad),
            Some(&Visibility::Hidden),
            "black while resizing"
        );
        for _ in 0..SUSPEND_FRAMES - 2 {
            app.world_mut().run_system_once(suspend_for_resize).unwrap();
            assert!(!active(&app, scene_camera));
        }
        app.world_mut().run_system_once(suspend_for_resize).unwrap();
        assert!(active(&app, scene_camera), "rendering resumed");
        assert!(app.world().get::<SuspendedCamera>(scene_camera).is_none());
        app.world_mut().run_system_once(present_frame).unwrap();
        assert!(
            matches!(app.world().get::<RenderTarget>(scene_camera).unwrap(), RenderTarget::Image(t) if t.handle == resized),
            "cameras render into the new image"
        );
        assert_eq!(
            app.world().get::<Visibility>(quad),
            Some(&Visibility::Inherited)
        );
    }

    #[test]
    fn test_msaa_for_antialiasing() {
        assert_eq!(msaa_for(hl2_ui::config::Antialiasing::None), Msaa::Off);
        assert_eq!(msaa_for(hl2_ui::config::Antialiasing::X2), Msaa::Sample2);
        assert_eq!(msaa_for(hl2_ui::config::Antialiasing::X4), Msaa::Sample4);
        assert_eq!(msaa_for(hl2_ui::config::Antialiasing::X8), Msaa::Sample8);
    }

    #[test]
    fn test_msaa_for_hardware() {
        use hl2_ui::config::Antialiasing;
        // On 8-sample capable hardware (discrete GPU on DX12/Vulkan):
        assert_eq!(msaa_for_hardware(Antialiasing::X8, 8), Msaa::Sample8);
        assert_eq!(msaa_for_hardware(Antialiasing::X4, 8), Msaa::Sample4);
        assert_eq!(msaa_for_hardware(Antialiasing::X2, 8), Msaa::Sample2);
        assert_eq!(msaa_for_hardware(Antialiasing::None, 8), Msaa::Off);

        // On 4-sample capped hardware (Apple Silicon / Metal / integrated GPU):
        assert_eq!(msaa_for_hardware(Antialiasing::X8, 4), Msaa::Sample4);
        assert_eq!(msaa_for_hardware(Antialiasing::X4, 4), Msaa::Sample4);
        assert_eq!(msaa_for_hardware(Antialiasing::X2, 4), Msaa::Sample2);
        assert_eq!(msaa_for_hardware(Antialiasing::None, 4), Msaa::Off);

        // On 2-sample capped hardware (CPU software renderer):
        assert_eq!(msaa_for_hardware(Antialiasing::X8, 2), Msaa::Sample2);
        assert_eq!(msaa_for_hardware(Antialiasing::X4, 2), Msaa::Sample2);
        assert_eq!(msaa_for_hardware(Antialiasing::X2, 2), Msaa::Sample2);
        assert_eq!(msaa_for_hardware(Antialiasing::None, 2), Msaa::Off);
    }

    #[test]
    fn test_sync_antialiasing_updates_scene_cameras_and_leaves_present_camera_off() {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = App::new();
        let mut console = hl2_ui::console::Console::default();
        console.config.video.advanced.antialiasing = hl2_ui::config::Antialiasing::X4;
        let ui = crate::ui_host::Console::new(console);
        app.insert_resource(ui);

        let scene_cam = app.world_mut().spawn((Camera3d::default(), Msaa::Off)).id();
        let viewmodel_cam = app.world_mut().spawn((Camera3d::default(), Msaa::Off)).id();
        let present_cam = app
            .world_mut()
            .spawn((PresentCamera, Camera2d, Msaa::Off))
            .id();

        app.world_mut().run_system_once(sync_antialiasing).unwrap();

        assert_eq!(*app.world().get::<Msaa>(scene_cam).unwrap(), Msaa::Sample4);
        assert_eq!(
            *app.world().get::<Msaa>(viewmodel_cam).unwrap(),
            Msaa::Sample4
        );
        assert_eq!(*app.world().get::<Msaa>(present_cam).unwrap(), Msaa::Off);

        // Update config to 8X: with default adapter limit (4), clamps safely to Sample4
        app.world_mut()
            .resource_mut::<crate::ui_host::Console>()
            .source
            .config
            .video
            .advanced
            .antialiasing = hl2_ui::config::Antialiasing::X8;

        app.world_mut().run_system_once(sync_antialiasing).unwrap();

        assert_eq!(*app.world().get::<Msaa>(scene_cam).unwrap(), Msaa::Sample4);
        assert_eq!(
            *app.world().get::<Msaa>(viewmodel_cam).unwrap(),
            Msaa::Sample4
        );
        assert_eq!(*app.world().get::<Msaa>(present_cam).unwrap(), Msaa::Off);

        // Update config to 2X
        app.world_mut()
            .resource_mut::<crate::ui_host::Console>()
            .source
            .config
            .video
            .advanced
            .antialiasing = hl2_ui::config::Antialiasing::X2;

        app.world_mut().run_system_once(sync_antialiasing).unwrap();

        assert_eq!(*app.world().get::<Msaa>(scene_cam).unwrap(), Msaa::Sample2);
        assert_eq!(
            *app.world().get::<Msaa>(viewmodel_cam).unwrap(),
            Msaa::Sample2
        );
        assert_eq!(*app.world().get::<Msaa>(present_cam).unwrap(), Msaa::Off);
    }

    #[test]
    fn test_sync_filtering_updates_active_images() {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = App::new();
        app.init_resource::<Assets<Image>>();

        let mut console = hl2_ui::console::Console::default();
        console.config.video.advanced.filtering = hl2_ui::config::Filtering::Anisotropic16;
        let ui = crate::ui_host::Console::new(console);
        app.insert_resource(ui);

        let scene_img = Image {
            sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                anisotropy_clamp: 1,
                ..Default::default()
            }),
            ..Default::default()
        };

        let glyph_img = Image {
            sampler: ImageSampler::Descriptor(ImageSamplerDescriptor {
                mag_filter: ImageFilterMode::Nearest,
                min_filter: ImageFilterMode::Nearest,
                mipmap_filter: ImageFilterMode::Nearest,
                anisotropy_clamp: 1,
                ..Default::default()
            }),
            ..Default::default()
        };

        let scene_handle = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(scene_img);
        let glyph_handle = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(glyph_img);

        app.world_mut().run_system_once(sync_filtering).unwrap();

        let images = app.world().resource::<Assets<Image>>();
        let scene_desc = match &images.get(&scene_handle).unwrap().sampler {
            ImageSampler::Descriptor(d) => d,
            _ => panic!("expected descriptor"),
        };
        assert_eq!(scene_desc.anisotropy_clamp, 16);
        assert_eq!(scene_desc.mipmap_filter, ImageFilterMode::Linear);

        let glyph_desc = match &images.get(&glyph_handle).unwrap().sampler {
            ImageSampler::Descriptor(d) => d,
            _ => panic!("expected descriptor"),
        };
        assert_eq!(glyph_desc.anisotropy_clamp, 1);
        assert_eq!(glyph_desc.mag_filter, ImageFilterMode::Nearest);

        // Switch to Bilinear
        app.world_mut()
            .resource_mut::<crate::ui_host::Console>()
            .source
            .config
            .video
            .advanced
            .filtering = hl2_ui::config::Filtering::Bilinear;

        app.world_mut().run_system_once(sync_filtering).unwrap();

        let images = app.world().resource::<Assets<Image>>();
        let scene_desc = match &images.get(&scene_handle).unwrap().sampler {
            ImageSampler::Descriptor(d) => d,
            _ => panic!("expected descriptor"),
        };
        assert_eq!(scene_desc.anisotropy_clamp, 1);
        assert_eq!(scene_desc.mipmap_filter, ImageFilterMode::Nearest);
    }
}
