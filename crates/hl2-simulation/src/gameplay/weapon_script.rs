//! Read weapon script sound entries once while building the weapon definitions.
use source_assets::keyvalues::Entry;
use std::collections::BTreeMap;

pub(super) fn sounds(root: &Entry) -> BTreeMap<String, String> {
    root.get("SoundData")
        .map(|e| {
            e.children()
                .iter()
                .filter_map(|entry| entry.text().map(|sound| (entry.key.clone(), sound.into())))
                .collect()
        })
        .unwrap_or_default()
}
