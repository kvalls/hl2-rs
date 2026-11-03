//! Per-playback cue state; the empty default performs no content discovery or precache.
use super::ScriptParams;
use crate::sounds::AudioSummary;
use source_assets::sounds::{ActorRegistry, Wave};
use std::collections::{BTreeMap, BTreeSet, HashMap};
#[derive(Default)]
pub struct Library {
    pub(super) names: BTreeMap<String, Vec<Wave>>,
    /// Playback paths of waves marked CHAR_DISTVARIANT ('^').
    pub(super) distance_variants: BTreeSet<String>,
    pub(super) params: BTreeMap<String, ScriptParams>,
    pub(super) actors: ActorRegistry,
    pub(super) available: HashMap<String, Vec<bool>>,
    pub(super) rng: u64,
    /// Separate stream for volume/pitch draws so wave selection is unchanged.
    pub(super) param_rng: u64,
    pub variants_played: BTreeMap<String, usize>,
    pub decoded: BTreeMap<String, AudioSummary>,
    pub errors: BTreeMap<String, String>,
    pub played: usize,
}
