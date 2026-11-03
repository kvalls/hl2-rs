//! The Video page's HDR choice (`mat_hdr_level`).
//!
//! The SDK's `viewpostprocess.cpp` fixes what the levels mean: bloom needs level 1 or more, a
//! non-HDR frame (level below 2) takes no automatic exposure and scales its bloom by
//! `mat_non_hdr_bloom_scalefactor`, and the cvar defaults to 2.

use crate::config::VideoSettings;
use serde::{Deserialize, Serialize};

/// `mat_non_hdr_bloom_scalefactor`: the bloom of an LDR frame (SDK default).
pub const NON_HDR_BLOOM_SCALE: f32 = 0.3;

/// `mat_hdr_level`: 0 None, 1 Bloom, 2 Full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HdrMode {
    None,
    Bloom,
    #[default]
    Full,
}

impl HdrMode {
    pub const ALL: [Self; 3] = [Self::None, Self::Bloom, Self::Full];

    pub fn level(self) -> u8 {
        self as u8
    }

    pub fn from_level(level: u8) -> Option<Self> {
        Self::ALL.get(usize::from(level)).copied()
    }

    pub fn token(self) -> &'static str {
        match self {
            Self::None => "#GameUI_hdr_level0",
            Self::Bloom => "#GameUI_hdr_level1",
            Self::Full => "#GameUI_hdr_level2",
        }
    }

    /// Whether the frame is lit with the maps' HDR data (lightmaps, ambient cubes, world lights,
    /// cubemaps, sky) and tone mapped with an automatic exposure: only the full level.
    pub fn is_hdr(self) -> bool {
        self == Self::Full
    }

    /// The bloom strength factor of this level: none at level 0, scaled down on an LDR frame.
    pub fn bloom_factor(self) -> f32 {
        match self {
            Self::None => 0.,
            Self::Bloom => NON_HDR_BLOOM_SCALE,
            Self::Full => 1.,
        }
    }
}

/// Files written before the default became Full hold `"hdr": "none"` as the old default, not a
/// choice: the first load turns that into Full, and later loads keep whatever the player picks.
pub fn migrate_default(video: &mut VideoSettings) {
    if video.hdr_migrated.is_none() {
        if video.hdr == HdrMode::None {
            video.hdr = HdrMode::Full;
        }
        video.hdr_migrated = Some(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn levels_follow_the_sdk() {
        assert_eq!(HdrMode::default(), HdrMode::Full);
        assert_eq!(HdrMode::from_level(1), Some(HdrMode::Bloom));
        assert!(HdrMode::Full.is_hdr() && !HdrMode::Bloom.is_hdr() && !HdrMode::None.is_hdr());
        assert_eq!(HdrMode::None.bloom_factor(), 0.);
        assert_eq!(HdrMode::Bloom.bloom_factor(), 0.3);
        assert_eq!(HdrMode::Full.bloom_factor(), 1.);
    }

    #[test]
    fn an_old_default_none_becomes_full_once() {
        let old = Config::from_json(br#"{ "video": { "hdr": "none" } }"#).unwrap();
        assert_eq!(old.video.hdr, HdrMode::Full);
        let saved = serde_json::to_vec(&old).unwrap();
        let mut chosen = Config::from_json(&saved).unwrap();
        chosen.video.hdr = HdrMode::None;
        let saved = serde_json::to_vec(&chosen).unwrap();
        let again = Config::from_json(&saved).unwrap();
        assert_eq!(again.video.hdr, HdrMode::None, "a later choice is kept");
        let bloom = Config::from_json(br#"{ "video": { "hdr": "bloom" } }"#).unwrap();
        assert_eq!(bloom.video.hdr, HdrMode::Bloom);
        assert_eq!(Config::default().video.hdr, HdrMode::Full);
    }
}
