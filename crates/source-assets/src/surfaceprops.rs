//! scripts/surfaceproperties*.txt reader (surfacedata_t, SDK vphysics_interface.h).
//! Entries inherit every unspecified field from "base", or from "default" when no
//! base is named (the vphysics parser itself is not in the SDK; this follows the
//! documented behavior of the shipped scripts' "base" keys).
use crate::{keyvalues, vpk::Vfs};
use anyhow::Result;
use modkit_core::SurfaceMaterial;
use std::collections::BTreeMap;

/// Reads the manifest and every listed file. Later definitions of a name replace
/// earlier ones (the manifest lists overrides last).
pub fn read(vfs: &Vfs) -> Result<BTreeMap<String, SurfaceMaterial>> {
    let mut raw = Vec::new();
    if let Some(manifest) = vfs.read("scripts/surfaceproperties_manifest.txt")? {
        let entries = keyvalues::parse(&String::from_utf8_lossy(&manifest))?;
        for file in entries
            .iter()
            .flat_map(|e| e.children())
            .filter(|e| e.key.eq_ignore_ascii_case("file"))
            .filter_map(|e| e.text())
        {
            if let Some(data) = vfs.read(file)? {
                raw.extend(keyvalues::parse(&String::from_utf8_lossy(&data))?);
            }
        }
    }
    Ok(resolve(&raw))
}

/// Resolves parsed entries (top-level blocks keyed by surface name).
pub fn resolve(entries: &[keyvalues::Entry]) -> BTreeMap<String, SurfaceMaterial> {
    let mut by_name: BTreeMap<String, &keyvalues::Entry> = BTreeMap::new();
    let mut order = Vec::new();
    for entry in entries {
        let name = entry.key.to_lowercase();
        if by_name.insert(name.clone(), entry).is_none() {
            order.push(name);
        }
    }
    let mut resolved = BTreeMap::new();
    for name in order {
        resolve_one(&name, &by_name, &mut resolved, 0);
    }
    resolved
}

fn resolve_one(
    name: &str,
    by_name: &BTreeMap<String, &keyvalues::Entry>,
    resolved: &mut BTreeMap<String, SurfaceMaterial>,
    depth: usize,
) -> SurfaceMaterial {
    if let Some(done) = resolved.get(name) {
        return done.clone();
    }
    let Some(entry) = by_name.get(name) else {
        return SurfaceMaterial::default();
    };
    let base = entry
        .get("base")
        .and_then(|e| e.text())
        .map(str::to_lowercase)
        .unwrap_or_else(|| "default".into());
    let mut material = if base != name && depth < 32 {
        resolve_one(&base, by_name, resolved, depth + 1)
    } else {
        SurfaceMaterial::default()
    };
    for child in entry.children() {
        let Some(value) = child.text() else {
            continue;
        };
        let number = value.trim().parse::<f32>().ok();
        let text = Some(value.to_string());
        match child.key.to_lowercase().as_str() {
            "friction" => material.friction = number.unwrap_or(material.friction),
            "elasticity" => material.elasticity = number.unwrap_or(material.elasticity),
            "density" => material.density = number.unwrap_or(material.density),
            "audiohardnessfactor" => {
                material.hardness_factor = number.unwrap_or(material.hardness_factor)
            }
            "impacthardthreshold" => {
                material.hard_threshold = number.unwrap_or(material.hard_threshold)
            }
            "audiohardminvelocity" => {
                material.hard_velocity_threshold =
                    number.unwrap_or(material.hard_velocity_threshold)
            }
            "audioroughnessfactor" => {
                material.roughness_factor = number.unwrap_or(material.roughness_factor)
            }
            "scraperoughthreshold" => {
                material.rough_threshold = number.unwrap_or(material.rough_threshold)
            }
            "impacthard" => material.impact_hard = text,
            "impactsoft" => material.impact_soft = text,
            "scrapesmooth" => material.scrape_smooth = text,
            "scraperough" => material.scrape_rough = text,
            "break" => material.break_sound = text,
            "gamematerial" => material.game_material = value.trim().chars().next(),
            _ => {}
        }
    }
    resolved.insert(name.to_string(), material.clone());
    material
}

/// physicssound::PlayImpactSounds (vphysics_sound.h): impactHard unless the hit
/// surface is softer than this one's hard threshold or the speed is below its hard
/// velocity threshold, when an impactSoft exists. None when no impactHard is set.
pub fn impact_sound<'a>(
    surface: &'a SurfaceMaterial,
    hit: Option<&SurfaceMaterial>,
    speed: f32,
) -> Option<&'a str> {
    let hard = surface.impact_hard.as_deref()?;
    if let (Some(hit), Some(soft)) = (hit, surface.impact_soft.as_deref()) {
        if hit.hardness_factor < surface.hard_threshold
            || (surface.hard_velocity_threshold > 0.
                && surface.hard_velocity_threshold > speed + 1e-4)
        {
            return Some(soft);
        }
    }
    Some(hard)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic surfaceproperties text written for this test (not a game file).
    const SYNTHETIC: &str = r#"
"default" { "friction" "0.8" "elasticity" "0.25" "density" "2000"
  "audiohardnessfactor" "1.0" "impacthardthreshold" "0.5" "audiohardminvelocity" "0"
  "impacthard" "Synthetic.HardDefault" "impactsoft" "Synthetic.SoftDefault" "gamematerial" "C" }
"crate" { "base" "default" "friction" "0.6" "density" "500" "audiohardnessfactor" "0.3"
  "impacthard" "Synthetic.CrateHard" "impactsoft" "Synthetic.CrateSoft"
  "audiohardminvelocity" "300" }
"crate_child" { "base" "crate" "elasticity" "0.1" }
"sky" { "gamematerial" "X" }
"#;

    #[test]
    fn inheritance_and_sound_choice_follow_the_sdk_rules() {
        let table = resolve(&keyvalues::parse(SYNTHETIC).unwrap());
        let child = &table["crate_child"];
        assert_eq!(child.friction, 0.6);
        assert_eq!(child.elasticity, 0.1);
        assert_eq!(child.density, 500.);
        assert_eq!(table["sky"].friction, 0.8, "no base inherits default");
        assert_eq!(table["sky"].game_material, Some('X'));
        let concrete = &table["default"];
        // Fast impact on a hard surface: hard. Slow (below 300): soft.
        assert_eq!(
            impact_sound(child, Some(concrete), 400.),
            Some("Synthetic.CrateHard")
        );
        assert_eq!(
            impact_sound(child, Some(concrete), 200.),
            Some("Synthetic.CrateSoft")
        );
        // Hitting something softer than the default's 0.5 threshold: soft.
        assert_eq!(
            impact_sound(concrete, Some(child), 900.),
            Some("Synthetic.SoftDefault")
        );
    }
}
