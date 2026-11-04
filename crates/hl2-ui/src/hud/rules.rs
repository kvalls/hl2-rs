//! scripts/HudAnimations.txt: the animation commands and their event rules.
use super::{animation_lines, Animate, EventRule, Interpolation, Result};

pub(super) fn animations(source: &str) -> Result<Vec<Animate>> {
    let mut result = Vec::new();
    for (event, tokens) in animation_lines(source)? {
        if tokens.first().map(String::as_str) == Some("Animate") && tokens.len() >= 7 {
            let Some(interpolation) = Interpolation::read(&tokens[4]) else {
                // Pulse/Bias/Gain/Flicker take additional parameters and are not this subset.
                continue;
            };
            let (Ok(delay), Ok(duration)) = (tokens[5].parse::<f64>(), tokens[6].parse::<f64>())
            else {
                continue;
            };
            if !delay.is_finite() || !duration.is_finite() || delay < 0. || duration < 0. {
                continue;
            }
            result.push(Animate {
                event,
                panel: tokens[1].clone(),
                property: tokens[2].clone(),
                target: tokens[3].clone(),
                delay,
                duration,
                interpolation,
            });
        }
    }
    Ok(result)
}
pub(super) fn event_rules(source: &str) -> Result<Vec<EventRule>> {
    Ok(animation_lines(source)?
        .into_iter()
        .filter_map(|(event, tokens)| {
            if !matches!(
                tokens.first().map(String::as_str),
                Some("StopEvent" | "RunEvent" | "StopPanelAnimations" | "StopAnimation")
            ) || tokens.len() < 3
            {
                return None;
            }
            let property = (tokens[0] == "StopAnimation")
                .then(|| tokens.get(2).cloned())
                .flatten();
            let delay = tokens
                .get(if property.is_some() { 3 } else { 2 })?
                .parse::<f64>()
                .ok()?;
            (delay.is_finite() && delay >= 0.).then(|| EventRule {
                event,
                target: tokens[1].clone(),
                delay,
                run: tokens[0] == "RunEvent",
                panel: tokens[0] == "StopPanelAnimations",
                property,
            })
        })
        .collect())
}
