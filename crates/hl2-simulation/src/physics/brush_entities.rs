//! Which brush entities get colliders.

use modkit_core::Entity;

/// A solid brush entity: one of the solid classes, not made non-solid by its keys
/// (`solid 0`, func_brush `Solidity 1`, a func_tracktrain's SF_TRACKTRAIN_PASSABLE).
pub(super) fn solid(e: &Entity) -> bool {
    let class = matches!(
        e.class(),
        "func_brush"
            | "func_wall"
            | "func_wall_toggle"
            | "func_door"
            | "func_door_rotating"
            | "func_movelinear"
            | "func_rotating"
            | "func_breakable"
            | "func_physbox"
            | "func_detail"
            | "func_tracktrain"
    );
    let passable_train = e.class() == "func_tracktrain"
        && e.get("spawnflags")
            .and_then(|f| f.trim().parse::<u32>().ok())
            .unwrap_or(0)
            & crate::trains::PASSABLE
            != 0;
    class && !passable_train && e.get("solid") != Some("0") && e.get("Solidity") != Some("1")
}
