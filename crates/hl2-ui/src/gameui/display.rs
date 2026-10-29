//! Display-mode choices for the Video page: aspect-ratio buckets and resolution filtering.
//!
//! Resolutions are the host's monitor query (Bevy `Monitor::video_modes`) merged with
//! [`standard_modes`] and, for each aspect choice, the largest exact mode of that ratio that fits
//! the monitor ([`native_fits`]; a 3456x2234 Retina panel gets 2976x2232 for 4:3). The numbers
//! (standard modes grouped by ratio, the exact ratio of each aspect choice, the minimum size and
//! the native-fit step) live in the crate's `assets/display/modes.json`, embedded at build time
//! and parsed once. The game renders into an offscreen frame of the configured size and scales
//! it to the window or display, so a resolution does not need to be a mode the panel can switch
//! to. Some displays report only a few modes: Apple Retina panels list nothing below 1920x1200
//! pixels and nothing 4:3, which would leave the Normal (4:3) bucket and small sizes empty.
//! Full-screen choices are limited to the monitor's native size; windowed choices must fit the
//! desktop in pixels, since a windowed game shows one frame pixel per display pixel (retail hides
//! resolutions larger than the desktop in windowed mode).
//!
//! The aspect buckets follow the retail Video page grouping ("Normal (4:3)" also takes 5:4;
//! ratios within 0.05 of 1.6 are 16:10; the rest up to 2.1 is 16:9) and add HL2RS choices for
//! other displays: 21:9 (2560x1080, 3440x1440, ...) below 3.0 and 32:9 above, and, narrower than
//! 4:3, square 1:1 (0.875..1.125) and portrait 3:4, 10:16 and 9:16 (rotated monitors, phones).
//! The installed GameUI has none of these choices, so their labels are temporary
//! [`placeholders`]. Portrait modes count against [`min_size`] with their sides swapped. The bounds are in the data file too ([`AspectRatio::of`]). When the host
//! reports no usable mode, the current window size is offered so the list is never empty.

use std::{collections::BTreeMap, sync::LazyLock};

use serde::{Deserialize, Serialize};

use super::localize::placeholders;

/// The display-mode data embedded from `assets/display/modes.json`.
#[derive(Debug, Deserialize)]
struct ModeData {
    /// Smallest mode the Video page offers (Source rejects smaller desktop modes as well).
    min_size: (u32, u32),
    /// Both sides of a [`native_fits`] mode are multiples of this.
    native_fit_step: u32,
    /// The exact ratio and bucket bounds of each aspect choice ([`AspectRatio::exact_ratio`],
    /// [`AspectRatio::of`]), narrowest bucket first.
    aspect_ratios: Vec<AspectEntry>,
    /// Standard resolutions grouped by ratio label (the labels only document the file).
    standard_modes: BTreeMap<String, Vec<(u32, u32)>>,
}

#[derive(Debug, Deserialize)]
struct AspectEntry {
    aspect: AspectRatio,
    ratio: (u32, u32),
    /// The bucket takes ratios below this (the last bucket has no bound).
    #[serde(default)]
    below: Option<f32>,
    /// A window bucket instead: ratios this close to the exact ratio, checked first.
    #[serde(default)]
    tolerance: Option<f32>,
}

impl AspectEntry {
    fn exact(&self) -> f32 {
        self.ratio.0 as f32 / self.ratio.1 as f32
    }
}

fn data() -> &'static ModeData {
    static DATA: LazyLock<ModeData> = LazyLock::new(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/display/modes.json"
        )))
        .expect("embedded display mode data is valid")
    });
    &DATA
}

/// Smallest mode the Video page offers (Source rejects smaller desktop modes as well).
pub fn min_size() -> Resolution {
    let (width, height) = data().min_size;
    Resolution::new(width, height)
}

/// Common 4:3, 5:4, 16:9, 16:10 and ultrawide (21:9, 32:9) resolutions offered on every display
/// that is large enough, in no particular order.
pub fn standard_modes() -> &'static [Resolution] {
    static MODES: LazyLock<Vec<Resolution>> = LazyLock::new(|| {
        data()
            .standard_modes
            .values()
            .flatten()
            .map(|&(width, height)| Resolution::new(width, height))
            .collect()
    });
    &MODES
}

/// For each aspect choice, the largest mode of exactly that ratio inside `monitor`, both sides
/// multiples of the data file's `native_fit_step`: displays that report few modes (Retina panels
/// list no 4:3 mode at all) still get every aspect choice up to the panel's size.
pub fn native_fits(monitor: Resolution) -> impl Iterator<Item = Resolution> {
    let step = data().native_fit_step;
    AspectRatio::FIXED.into_iter().filter_map(move |aspect| {
        let (w, h) = aspect.exact_ratio();
        let k = (monitor.width / w).min(monitor.height / h) / step * step;
        (k > 0).then(|| Resolution::new(w * k, h * k))
    })
}

/// The Aspect Ratio combo choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AspectRatio {
    /// `#GameUI_AspectNormal`.
    Normal,
    /// `#GameUI_AspectWide16x9`.
    #[default]
    Wide16x9,
    /// `#GameUI_AspectWide16x10`.
    Wide16x10,
    /// 21:9 displays; placeholder label [`placeholders::ASPECT_ULTRAWIDE_21X9`].
    Ultrawide21x9,
    /// 32:9 displays; placeholder label [`placeholders::ASPECT_ULTRAWIDE_32X9`].
    Ultrawide32x9,
    /// Square displays; placeholder label [`placeholders::ASPECT_SQUARE_1X1`].
    Square1x1,
    /// 3:4 portrait displays; placeholder label [`placeholders::ASPECT_PORTRAIT_3X4`].
    Portrait3x4,
    /// 10:16 portrait displays; placeholder label [`placeholders::ASPECT_PORTRAIT_10X16`].
    Portrait10x16,
    /// 9:16 portrait displays; placeholder label [`placeholders::ASPECT_PORTRAIT_9X16`].
    Portrait9x16,
    /// The frame takes the aspect ratio of the window or display, letterboxed or pillarboxed
    /// only beyond the supported `9:16..=32:9` range ([`crate::tuning::clamp_supported_aspect`]);
    /// the resolution sets the frame's detail (its short side). Labelled with the installed
    /// `#GameUI_HUDAspectRatio_FitDisplay` ("Fit Display"). Not a bucket: no resolution falls in
    /// it, so it is stored as [`crate::config::VideoSettings::flexible_aspect`].
    Flexible,
}

impl AspectRatio {
    /// Combo order: the flexible frame first (Fit Display), then the retail choices, then the
    /// wider ones, then the narrower ones.
    pub const ALL: [Self; 10] = [
        Self::Flexible,
        Self::Normal,
        Self::Wide16x9,
        Self::Wide16x10,
        Self::Ultrawide21x9,
        Self::Ultrawide32x9,
        Self::Square1x1,
        Self::Portrait3x4,
        Self::Portrait10x16,
        Self::Portrait9x16,
    ];

    /// The fixed-ratio buckets of the mode data (every choice but [`Self::Flexible`]).
    pub const FIXED: [Self; 9] = [
        Self::Normal,
        Self::Wide16x9,
        Self::Wide16x10,
        Self::Ultrawide21x9,
        Self::Ultrawide32x9,
        Self::Square1x1,
        Self::Portrait3x4,
        Self::Portrait10x16,
        Self::Portrait9x16,
    ];

    /// Bucket of a width/height pair: a window bucket (`tolerance`) around its exact ratio wins,
    /// otherwise the first bucket whose `below` bound exceeds the ratio.
    pub fn of(width: u32, height: u32) -> Self {
        let ratio = width as f32 / height.max(1) as f32;
        let entries = &data().aspect_ratios;
        entries
            .iter()
            .find(|e| e.tolerance.is_some_and(|t| (ratio - e.exact()).abs() < t))
            .or_else(|| {
                entries
                    .iter()
                    .filter(|e| e.tolerance.is_none())
                    .find(|e| e.below.is_none_or(|below| ratio < below))
            })
            .map_or_else(Self::default, |e| e.aspect)
    }

    /// The exact width:height ratio of this choice, from the mode data. [`Self::Flexible`] has
    /// none and reports the default bucket's ratio.
    pub fn exact_ratio(self) -> (u32, u32) {
        let aspect = if self == Self::Flexible {
            Self::default()
        } else {
            self
        };
        data()
            .aspect_ratios
            .iter()
            .find(|entry| entry.aspect == aspect)
            .map(|entry| entry.ratio)
            .expect("mode data has a ratio for every aspect choice")
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::Normal => "#GameUI_AspectNormal",
            Self::Wide16x9 => "#GameUI_AspectWide16x9",
            Self::Wide16x10 => "#GameUI_AspectWide16x10",
            Self::Ultrawide21x9 => placeholders::ASPECT_ULTRAWIDE_21X9,
            Self::Ultrawide32x9 => placeholders::ASPECT_ULTRAWIDE_32X9,
            Self::Square1x1 => placeholders::ASPECT_SQUARE_1X1,
            Self::Portrait3x4 => placeholders::ASPECT_PORTRAIT_3X4,
            Self::Portrait10x16 => placeholders::ASPECT_PORTRAIT_10X16,
            Self::Portrait9x16 => placeholders::ASPECT_PORTRAIT_9X16,
            Self::Flexible => "#GameUI_HUDAspectRatio_FitDisplay",
        }
    }
}

/// One selectable resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn aspect(self) -> AspectRatio {
        AspectRatio::of(self.width, self.height)
    }

    pub fn label(self) -> String {
        format!("{} x {}", self.width, self.height)
    }

    /// Returns the active resolution without letterbox or pillarbox black stripes when flexible
    /// aspect ratio is enabled (clamped to the supported 9:16..=32:9 range).
    pub fn without_stripes_for_flexible(self) -> Self {
        let size = crate::flexible_frame::flexible_presented_resolution(glam::UVec2::new(
            self.width,
            self.height,
        ));
        Self::new(size.x, size.y)
    }

    /// Whether this resolution is at least `min` in either orientation (a portrait mode is
    /// measured with its sides swapped).
    pub fn at_least(self, min: Resolution) -> bool {
        let (long, short) = (self.width.max(self.height), self.width.min(self.height));
        long >= min.width.max(min.height) && short >= min.width.min(min.height)
    }

    /// Whether this resolution fits inside `bounds` in both dimensions.
    pub fn fits(self, bounds: Resolution) -> bool {
        self.width <= bounds.width && self.height <= bounds.height
    }
}

impl std::str::FromStr for Resolution {
    type Err = anyhow::Error;

    /// `WIDTHxHEIGHT`, e.g. `7680x4320`.
    fn from_str(text: &str) -> anyhow::Result<Self> {
        let (width, height) = text
            .trim()
            .split_once(['x', 'X'])
            .ok_or_else(|| anyhow::anyhow!("expected WIDTHxHEIGHT, got {text:?}"))?;
        Ok(Self::new(width.parse()?, height.parse()?))
    }
}

/// A monitor described on the command line instead of queried from the host (testing the Video
/// page as another display would show it): `SIZE[,MODE...]`, e.g. `7680x4320,3840x2160`. The
/// first entry is the panel's size and also a reported mode; the others are further reported
/// modes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorSpec {
    pub size: Resolution,
    pub reported: Vec<Resolution>,
}

impl MonitorSpec {
    /// The modes the Video page offers on this monitor, with the panel as the desktop.
    pub fn display_modes(&self, current: Resolution) -> DisplayModes {
        DisplayModes {
            fullscreen: usable_modes(self.reported.iter().copied(), Some(self.size), current),
            desktop: Some(self.size),
            current: Some(current),
        }
    }
}

impl std::str::FromStr for MonitorSpec {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> anyhow::Result<Self> {
        let reported = text
            .split(',')
            .map(str::parse)
            .collect::<anyhow::Result<Vec<Resolution>>>()?;
        let size = reported[0];
        anyhow::ensure!(
            size.at_least(min_size()),
            "the monitor must be at least {}",
            min_size().label()
        );
        anyhow::ensure!(
            reported.iter().all(|m| m.fits(size)),
            "reported modes must fit the monitor"
        );
        Ok(Self { size, reported })
    }
}

/// Usable modes, deduplicated and sorted largest first: the reported modes plus
/// [`standard_modes`] and the monitor's [`native_fits`]. Modes smaller than [`min_size`] or
/// larger than the monitor (when known) are dropped. Falls back to `current` when nothing remains.
pub fn usable_modes(
    reported: impl IntoIterator<Item = Resolution>,
    monitor: Option<Resolution>,
    current: Resolution,
) -> Vec<Resolution> {
    let min = min_size();
    let mut modes: Vec<_> = reported
        .into_iter()
        .chain(standard_modes().iter().copied())
        .chain(monitor.into_iter().flat_map(native_fits))
        .filter(|m| m.at_least(min))
        .filter(|m| monitor.is_none_or(|max| m.fits(max)))
        .collect();
    modes.sort_by(|a, b| b.cmp(a));
    modes.dedup();
    if modes.is_empty() {
        modes.push(current);
    }
    modes
}

/// The resolutions the Video page can offer on this display.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayModes {
    /// Full-screen choices, largest first (see [`usable_modes`]).
    pub fullscreen: Vec<Resolution>,
    /// The desktop size in pixels, when known. A windowed game shows one frame pixel per display
    /// pixel, so windowed choices must fit inside it.
    pub desktop: Option<Resolution>,
    /// The current window/display resolution in physical pixels, if known.
    pub current: Option<Resolution>,
}

impl DisplayModes {
    /// Full-screen choices with no windowed limit.
    pub fn new(fullscreen: Vec<Resolution>) -> Self {
        Self {
            fullscreen,
            desktop: None,
            current: None,
        }
    }

    /// The choices for a display mode: windowed resolutions must fit the desktop. When none fits,
    /// the smallest full-screen mode is offered so the list is never empty.
    pub fn choices(&self, windowed: bool) -> Vec<Resolution> {
        let Some(desktop) = self.desktop.filter(|_| windowed) else {
            return self.fullscreen.clone();
        };
        let fitting: Vec<_> = self
            .fullscreen
            .iter()
            .copied()
            .filter(|m| m.fits(desktop))
            .collect();
        if fitting.is_empty() {
            self.fullscreen.last().copied().into_iter().collect()
        } else {
            fitting
        }
    }
}

/// Modes of one aspect bucket, keeping the largest-first order. [`AspectRatio::Flexible`] takes
/// every mode: there the resolution only sets the frame's detail.
pub fn modes_for(modes: &[Resolution], aspect: AspectRatio) -> Vec<Resolution> {
    if aspect == AspectRatio::Flexible {
        return modes.to_vec();
    }
    modes
        .iter()
        .copied()
        .filter(|m| m.aspect() == aspect)
        .collect()
}

/// The resolution selected after the Aspect Ratio combo changes: the previous one if it is still
/// listed, otherwise the largest mode of the new bucket (empty buckets fall back to all modes).
pub fn choose_after_aspect_change(
    modes: &[Resolution],
    aspect: AspectRatio,
    previous: Resolution,
) -> Option<Resolution> {
    let candidates = resolution_choices(modes, aspect);
    if candidates.contains(&previous) {
        Some(previous)
    } else {
        candidates.first().copied()
    }
}

/// The list shown in the Resolution combo for an aspect bucket; empty buckets show all modes.
pub fn resolution_choices(modes: &[Resolution], aspect: AspectRatio) -> Vec<Resolution> {
    let bucket = modes_for(modes, aspect);
    if bucket.is_empty() {
        modes.to_vec()
    } else {
        bucket
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(width: u32, height: u32) -> Resolution {
        Resolution::new(width, height)
    }

    #[test]
    fn mode_data_parses_and_every_entry_is_usable() {
        let min = min_size();
        let modes = standard_modes();
        assert!(!modes.is_empty());
        assert!(
            modes.iter().all(|m| m.at_least(min)),
            "every mode >= min_size"
        );
        let mut unique = modes.to_vec();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), modes.len(), "no duplicate modes");
        assert_eq!(data().aspect_ratios.len(), AspectRatio::FIXED.len());
        for aspect in AspectRatio::FIXED {
            let (w, h) = aspect.exact_ratio();
            assert_eq!(
                AspectRatio::of(w, h),
                aspect,
                "{aspect:?} ratio is in its bucket"
            );
        }
        let last = data().aspect_ratios.iter().rfind(|e| e.tolerance.is_none());
        assert!(
            last.is_some_and(|e| e.below.is_none()),
            "the widest bucket is unbounded"
        );
        assert!(data().native_fit_step > 0);
    }

    #[test]
    fn aspect_buckets_match_the_video_page_grouping() {
        assert_eq!(AspectRatio::of(1024, 768), AspectRatio::Normal);
        assert_eq!(
            AspectRatio::of(1280, 1024),
            AspectRatio::Normal,
            "5:4 joins 4:3"
        );
        assert_eq!(AspectRatio::of(1680, 1050), AspectRatio::Wide16x10);
        assert_eq!(
            AspectRatio::of(1512, 982),
            AspectRatio::Wide16x9,
            "1.54 is wider than 16:10's window"
        );
        assert_eq!(AspectRatio::of(1920, 1080), AspectRatio::Wide16x9);
        assert_eq!(AspectRatio::of(2160, 1080), AspectRatio::Wide16x9, "2:1");
        for (w, h) in [(2560, 1080), (3440, 1440), (3840, 1600), (5120, 2160)] {
            assert_eq!(AspectRatio::of(w, h), AspectRatio::Ultrawide21x9, "{w}x{h}");
        }
        assert_eq!(AspectRatio::of(3840, 1080), AspectRatio::Ultrawide32x9);
        assert_eq!(AspectRatio::of(1080, 1080), AspectRatio::Square1x1);
        assert_eq!(AspectRatio::of(768, 1024), AspectRatio::Portrait3x4);
        assert_eq!(AspectRatio::of(1080, 1350), AspectRatio::Portrait3x4, "4:5");
        assert_eq!(AspectRatio::of(1200, 1920), AspectRatio::Portrait10x16);
        assert_eq!(AspectRatio::of(1080, 1920), AspectRatio::Portrait9x16);
        assert_eq!(
            AspectRatio::of(1170, 2532),
            AspectRatio::Portrait9x16,
            "phone"
        );
        assert_eq!(AspectRatio::of(5120, 1440), AspectRatio::Ultrawide32x9);
        assert_eq!(
            AspectRatio::of(10, 0),
            AspectRatio::Ultrawide32x9,
            "zero height is safe"
        );
    }

    #[test]
    fn modes_are_filtered_sorted_deduplicated_with_a_fallback() {
        let modes = usable_modes(
            [
                r(800, 600),
                r(320, 240),
                r(1920, 1080),
                r(800, 600),
                r(2560, 1440),
                r(1366, 1000),
            ],
            Some(r(1366, 1000)),
            r(1280, 720),
        );
        assert_eq!(
            modes,
            [
                r(1366, 1000),
                r(1366, 768),
                r(1312, 984),
                r(1280, 960),
                r(1280, 800),
                r(1280, 720),
                r(1152, 864),
                r(1024, 768),
                r(1000, 1000),
                r(800, 600),
                r(744, 992),
                r(720, 720),
                r(640, 480),
                r(560, 896),
                r(504, 896),
            ],
            "reported, standard and native-fit modes up to the monitor, largest first, no duplicates"
        );
        assert_eq!(
            usable_modes([], Some(r(320, 240)), r(320, 240)),
            [r(320, 240)],
            "nothing usable falls back to the current size"
        );
    }

    #[test]
    fn retina_panels_get_small_and_four_by_three_modes() {
        // A 3456x2234 Retina panel reports only 16:10-like modes from 1920x1200 up.
        let reported = [r(3456, 2234), r(3456, 2160), r(2992, 1934), r(1920, 1200)];
        let modes = DisplayModes {
            fullscreen: usable_modes(reported, Some(r(3456, 2234)), r(1728, 1117)),
            desktop: Some(r(3456, 2234)),
            current: None,
        };
        let fullscreen = modes.choices(false);
        assert_eq!(fullscreen.first(), Some(&r(3456, 2234)));
        assert!(fullscreen.contains(&r(1280, 720)));
        assert!(fullscreen.contains(&r(2048, 1536)));
        assert!(
            !fullscreen.contains(&r(3840, 2160)),
            "larger than the panel"
        );
        assert_eq!(
            resolution_choices(&fullscreen, AspectRatio::Normal).last(),
            Some(&r(640, 480))
        );
        assert_eq!(
            modes.choices(true),
            fullscreen,
            "windowed choices fit the desktop in pixels"
        );
        let small_desktop = DisplayModes {
            desktop: Some(r(1728, 1117)),
            ..modes.clone()
        };
        let windowed = small_desktop.choices(true);
        assert_eq!(windowed.first(), Some(&r(1680, 1050)));
        assert!(windowed.iter().all(|m| m.fits(r(1728, 1117))));
        assert!(windowed.contains(&r(1280, 720)) && windowed.contains(&r(640, 480)));
        assert_eq!(
            DisplayModes {
                fullscreen: vec![r(1024, 768)],
                desktop: Some(r(800, 600)),
                current: None,
            }
            .choices(true),
            [r(1024, 768)],
            "never empty"
        );
        assert_eq!(
            DisplayModes::new(vec![r(1024, 768)]).choices(true),
            [r(1024, 768)]
        );
    }

    #[test]
    fn every_aspect_choice_reaches_the_panel_size() {
        // A 3456x2234 Retina panel reports no 4:3 mode; 4:3 used to stop at 2048x1536.
        let modes = usable_modes(
            [r(3456, 2234), r(3456, 2160), r(1920, 1200)],
            Some(r(3456, 2234)),
            r(1728, 1117),
        );
        let top = |aspect| resolution_choices(&modes, aspect)[0];
        assert_eq!(top(AspectRatio::Normal), r(2976, 2232));
        assert_eq!(top(AspectRatio::Wide16x10), r(3456, 2160));
        assert_eq!(
            top(AspectRatio::Wide16x9),
            r(3456, 2234),
            "the native size (1.55) groups with 16:9"
        );
        assert!(modes.contains(&r(3456, 1944)));
        for m in native_fits(r(3456, 2234)) {
            assert!(
                m.fits(r(3456, 2234)) && m.width % 8 == 0 && m.height % 8 == 0,
                "{m:?}"
            );
        }
        assert!(modes.contains(&r(2560, 1920)) && modes.contains(&r(2800, 2100)));
        assert_eq!(
            native_fits(r(4, 3)).count(),
            0,
            "nothing for a tiny monitor"
        );
    }

    #[test]
    fn ultrawide_panels_get_their_own_aspect_choices() {
        let modes = usable_modes([r(3440, 1440)], Some(r(3440, 1440)), r(1920, 1080));
        let wide = resolution_choices(&modes, AspectRatio::Ultrawide21x9);
        assert_eq!(wide[0], r(3440, 1440));
        assert!(wide.contains(&r(2560, 1080)));
        assert!(!modes.contains(&r(5120, 1440)), "larger than the panel");
        assert_eq!(
            resolution_choices(&modes, AspectRatio::Wide16x9)[0],
            r(2560, 1440),
            "16:9 no longer lists ultrawide modes"
        );
        let super_wide = usable_modes([], Some(r(5120, 1440)), r(1920, 1080));
        let thirty_two = resolution_choices(&super_wide, AspectRatio::Ultrawide32x9);
        assert_eq!(thirty_two, [r(5120, 1440), r(3840, 1080)]);
        assert_eq!(
            resolution_choices(&super_wide, AspectRatio::Normal)[0],
            r(1920, 1440),
            "4:3 up to the panel's height"
        );
    }

    #[test]
    fn an_8k_panel_offers_every_aspect_up_to_its_size() {
        let panel = r(7680, 4320);
        let reported = [r(7680, 4320), r(3840, 2160), r(1920, 1080), r(1280, 720)];
        let modes = DisplayModes {
            fullscreen: usable_modes(reported, Some(panel), r(3840, 2160)),
            desktop: Some(panel),
            current: None,
        };
        let top = |aspect| resolution_choices(&modes.choices(false), aspect)[0];
        assert_eq!(top(AspectRatio::Normal), r(5760, 4320));
        assert_eq!(top(AspectRatio::Wide16x9), r(7680, 4320));
        assert_eq!(top(AspectRatio::Wide16x10), r(6912, 4320));
        assert_eq!(top(AspectRatio::Ultrawide21x9), r(7680, 3240));
        assert_eq!(top(AspectRatio::Ultrawide32x9), r(7680, 2160));
        let all = modes.choices(false);
        assert!(all.contains(&r(5120, 2880)) && all.contains(&r(640, 480)));
        assert_eq!(
            modes.choices(true),
            all,
            "every mode fits an 8K desktop in pixels"
        );
    }

    #[test]
    fn a_rotated_monitor_offers_portrait_and_landscape_modes() {
        let panel = r(1080, 1920);
        let modes = usable_modes([panel], Some(panel), r(1080, 1920));
        let top = |aspect| resolution_choices(&modes, aspect)[0];
        assert_eq!(top(AspectRatio::Portrait9x16), r(1080, 1920));
        assert_eq!(top(AspectRatio::Portrait10x16), r(1050, 1680));
        assert_eq!(top(AspectRatio::Portrait3x4), r(1080, 1440));
        assert_eq!(top(AspectRatio::Square1x1), r(1080, 1080));
        assert_eq!(top(AspectRatio::Normal), r(1056, 792), "landscape fits too");
        assert!(modes
            .iter()
            .all(|m| m.fits(panel) && m.at_least(min_size())));
        assert!(
            r(480, 640).at_least(min_size()) && !r(480, 480).at_least(min_size()),
            "portrait counts with its sides swapped"
        );
        assert_eq!(
            "1080x1920".parse::<MonitorSpec>().unwrap().size,
            panel,
            "portrait monitors can be faked"
        );
    }

    #[test]
    fn monitor_specs_parse_from_the_command_line() {
        let spec: MonitorSpec = "7680x4320,3840X2160".parse().unwrap();
        assert_eq!(spec.size, r(7680, 4320));
        assert_eq!(spec.reported, [r(7680, 4320), r(3840, 2160)]);
        assert_eq!(
            spec.display_modes(r(1024, 768)).fullscreen[0],
            r(7680, 4320)
        );
        for bad in [
            "",
            "7680",
            "7680x",
            "x4320",
            "320x240",
            "1920x1080,3840x2160",
        ] {
            assert!(bad.parse::<MonitorSpec>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_flexible_choice_lists_every_mode_and_keeps_the_resolution() {
        let modes = [r(1920, 1080), r(1680, 1050), r(1024, 768), r(1080, 1920)];
        assert_eq!(resolution_choices(&modes, AspectRatio::Flexible), modes);
        assert_eq!(
            choose_after_aspect_change(&modes, AspectRatio::Flexible, r(1024, 768)),
            Some(r(1024, 768))
        );
        assert_eq!(AspectRatio::ALL.first(), Some(&AspectRatio::Flexible));
        assert_eq!(
            AspectRatio::ALL[1..],
            AspectRatio::FIXED,
            "the fixed choices keep their combo positions"
        );
        assert!(!AspectRatio::FIXED.contains(&AspectRatio::Flexible));
        assert!(AspectRatio::FIXED
            .iter()
            .all(
                |&a| AspectRatio::of(a.exact_ratio().0, a.exact_ratio().1) != AspectRatio::Flexible
            ));
    }

    #[test]
    fn aspect_change_keeps_a_valid_resolution() {
        let modes = [
            r(1920, 1080),
            r(1680, 1050),
            r(1280, 720),
            r(1024, 768),
            r(800, 600),
        ];
        assert_eq!(
            resolution_choices(&modes, AspectRatio::Normal),
            [r(1024, 768), r(800, 600)]
        );
        assert_eq!(
            choose_after_aspect_change(&modes, AspectRatio::Normal, r(1920, 1080)),
            Some(r(1024, 768)),
            "largest mode of the new bucket"
        );
        assert_eq!(
            choose_after_aspect_change(&modes, AspectRatio::Wide16x9, r(1280, 720)),
            Some(r(1280, 720)),
            "a still-valid selection is kept"
        );
        let only_wide = [r(1920, 1080)];
        assert_eq!(
            resolution_choices(&only_wide, AspectRatio::Wide16x10),
            [r(1920, 1080)]
        );
        assert_eq!(
            choose_after_aspect_change(&only_wide, AspectRatio::Wide16x10, r(800, 600)),
            Some(r(1920, 1080)),
            "empty bucket falls back to every mode"
        );
        assert_eq!(
            choose_after_aspect_change(&[], AspectRatio::Normal, r(800, 600)),
            None
        );
    }

    #[test]
    fn resolution_without_stripes_for_flexible() {
        // Fits within supported 9:16..=32:9 range: unchanged
        assert_eq!(
            Resolution::new(1920, 1080).without_stripes_for_flexible(),
            Resolution::new(1920, 1080)
        );
        assert_eq!(
            Resolution::new(5120, 1440).without_stripes_for_flexible(),
            Resolution::new(5120, 1440)
        );
        assert_eq!(
            Resolution::new(1080, 1920).without_stripes_for_flexible(),
            Resolution::new(1080, 1920)
        );

        // Exceeds 32:9: pillarbox black stripes excluded
        assert_eq!(
            Resolution::new(8000, 1000).without_stripes_for_flexible(),
            Resolution::new(3556, 1000)
        );

        // Taller than 9:16: letterbox black stripes excluded
        assert_eq!(
            Resolution::new(500, 2000).without_stripes_for_flexible(),
            Resolution::new(500, 889)
        );
    }
}
