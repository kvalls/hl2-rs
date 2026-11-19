//! A scene MOVETO's style (SDK baseflex.cpp DetermineExpressionMoveActivity).
use super::{GoalBlockReason, MoveStyle};

impl MoveStyle {
    /// Parameter 2's activity word: Walk or Run (empty walks); a distance style may follow
    /// after a space (luggage_shove.vcd's "Walk Close_Distance_Once"). CrouchWalk, other
    /// activities and custom sequences are not modeled.
    pub fn parse(value: &str) -> Result<Self, GoalBlockReason> {
        let word = value.split_whitespace().next().unwrap_or("");
        match word.to_ascii_lowercase().as_str() {
            "" | "walk" => Ok(Self::Walk),
            "run" => Ok(Self::Run),
            _ => Err(GoalBlockReason::UnsupportedStyle(value.to_owned())),
        }
    }
}
