//! A model's prop_data block, resolved through propdata.txt's base chain.

fn section<'a>(
    entries: &'a [crate::keyvalues::Entry],
    name: &str,
) -> Option<&'a crate::keyvalues::Entry> {
    entries.iter().find_map(|e| {
        if e.key.eq_ignore_ascii_case(name) && !e.children().is_empty() {
            Some(e)
        } else if e.key.eq_ignore_ascii_case("BreakableModels") {
            None
        } else {
            section(e.children(), name)
        }
    })
}
fn apply(
    prop_data: &[crate::keyvalues::Entry],
    block: &crate::keyvalues::Entry,
    interactions: &[crate::keyvalues::Entry],
    depth: usize,
    data: &mut modkit_core::PropData,
) {
    if let Some(base) = block
        .get("base")
        .and_then(|b| b.text())
        .filter(|_| depth < 8)
        .and_then(|base| section(prop_data, base))
    {
        // ParsePropFromBase: a base's interactions are inside its own section.
        apply(prop_data, base, base.children(), depth + 1, data);
    }
    apply_interactions(interactions, data);
    apply_values(block, data);
}
/// The model's or base's interaction sections.
fn apply_interactions(interactions: &[crate::keyvalues::Entry], data: &mut modkit_core::PropData) {
    // sPropdataInteractionSections: a key set to exactly this value sets the flag.
    let interaction = |section: &str, key: &str, value: &str| {
        interactions
            .iter()
            .find(|e| e.key.eq_ignore_ascii_case(section))
            .and_then(|e| e.get(key))
            .and_then(|e| e.text())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case(value))
    };
    data.flammable |= interaction("fire_interactions", "flammable", "yes");
    data.explosive_resist |= interaction("fire_interactions", "explosive_resist", "yes");
    data.ignite_halfhealth |= interaction("fire_interactions", "ignite", "halfhealth");
    data.break_explode |= interaction("physgun_interactions", "onbreak", "explode_fire");
    data.spin_z |= interaction("physgun_interactions", "onlaunch", "spin_zaxis");
    data.world_stick |= interaction("physgun_interactions", "onworldimpact", "stick");
    // GetPropDataAngles: UTIL_StringToVector of the key's text.
    if let Some(angles) = interactions
        .iter()
        .find(|e| e.key.eq_ignore_ascii_case("physgun_interactions"))
        .and_then(|e| e.get("preferred_carryangles"))
        .and_then(|e| e.text())
    {
        let mut v = [0f32; 3];
        for (slot, word) in v.iter_mut().zip(angles.split_whitespace()) {
            *slot = word.parse().unwrap_or(0.);
        }
        data.carry_angles = Some(v);
    }
}
/// The block's own keys.
fn apply_values(block: &crate::keyvalues::Entry, data: &mut modkit_core::PropData) {
    let text = |key: &str| {
        block
            .children()
            .iter()
            .find(|e| e.key.eq_ignore_ascii_case(key))
            .and_then(|e| e.text())
            .map(str::trim)
    };
    let number = |key: &str| text(key).and_then(|v| v.parse::<f32>().ok());
    if let Some(v) = number("health") {
        data.health = v;
    }
    if let Some(v) = number("dmg.bullets") {
        data.dmg_bullets = v;
    }
    if let Some(v) = number("dmg.club") {
        data.dmg_club = v;
    }
    if let Some(v) = number("dmg.explosive") {
        data.dmg_explosive = v;
    }
    if let Some(v) = number("explosive_damage") {
        data.explosive_damage = Some(v);
    }
    if let Some(v) = number("explosive_radius") {
        data.explosive_radius = Some(v);
    }
    if let Some(v) = text("damage_table") {
        data.damage_table = (!v.is_empty()).then(|| v.to_string());
    }
    if let Some(v) = text("breakable_model") {
        data.breakable_model = (!v.is_empty()).then(|| v.to_string());
    }
    // GetInt: atoi of the text.
    if let Some(v) = number("breakable_count") {
        data.breakable_count = v as i32;
    }
    if let Some(v) = number("breakable_skin") {
        data.breakable_skin = v as i32;
    }
}

/// CPropData::ParsePropFromKV: the model's prop_data block over its propdata.txt "base"
/// chain (the base is parsed first, then the block's own keys override). None when the
/// model has no prop_data.
pub fn resolve_prop_data(
    prop_data: &[crate::keyvalues::Entry],
    model_keyvalues: &str,
) -> Option<modkit_core::PropData> {
    let entries = crate::keyvalues::parse(model_keyvalues).ok()?;
    // The studio keyvalue text wraps its blocks in "mdlkeyvalue { ... }".
    let block = entries
        .iter()
        .flat_map(|e| std::iter::once(e).chain(e.children()))
        .find(|e| e.key.eq_ignore_ascii_case("prop_data"))?;
    // The model's interaction sections are the prop_data block's siblings.
    let siblings: Vec<crate::keyvalues::Entry> = entries
        .iter()
        .find(|e| e.children().iter().any(|c| std::ptr::eq(c, block)))
        .map_or_else(|| entries.clone(), |parent| parent.children().to_vec());
    let mut data = modkit_core::PropData::default();
    apply(prop_data, block, &siblings, 0, &mut data);
    Some(data)
}
