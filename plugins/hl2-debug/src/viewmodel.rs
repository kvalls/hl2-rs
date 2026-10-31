//! Temporary per-weapon and per-aspect debug edits.
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct ViewmodelDebug {
    pub enabled: bool,
    pub weapon: String,
    pub profile: Option<modkit_core::viewmodel::ViewmodelProfile>,
    pub framing: modkit_core::viewmodel::ViewmodelFraming,
    /// `viewmodel_tweak_reset` ran; `controls` zeroes the selected profile once it is selected.
    pub reset_pending: bool,
    /// Edits retained for the current tweak-mode session, keyed by weapon classname and direct profile.
    pub edits: BTreeMap<
        (String, modkit_core::viewmodel::ViewmodelProfile),
        modkit_core::viewmodel::ViewmodelFraming,
    >,
}
