//! Triggers with inputs of their own: `trigger_changelevel` (ChangeLevel).

use super::{Delivery, Scene};

/// ChangeLevel: the level transition to the entity's map and landmark.
pub(super) fn changelevel(scene: &mut Scene, d: &Delivery) -> bool {
    if d.input != "changelevel" {
        return false;
    }
    if let Some(map) = d.entity.get("map") {
        scene.transition = Some((map.into(), d.entity.get("landmark").unwrap_or("").into()));
    }
    true
}
