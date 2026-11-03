//! Material variables as the retail material system evaluates them: key conditionals
//! (`test?$var`) and DX-level blocks, for a DX9 renderer in an HDR or LDR frame.
use source_assets::keyvalues::{Entry, Value};
use std::collections::BTreeMap;

/// Direct material vars, with retail MaterialSystem key conditionals (`test?$var`, optional
/// `!`): a key whose test fails is skipped, and a passing one replaces the plain value.
pub(crate) fn direct_properties(entries: &[Entry], hdr: bool) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::new();
    let mut conditional = Vec::new();
    for entry in entries {
        let Some(text) = entry.text() else {
            continue;
        };
        let key = entry.key.to_lowercase();
        match key.split_once('?') {
            Some((test, name)) if !test.is_empty() => {
                if material_condition(test, hdr) {
                    conditional.push((name.to_owned(), text.to_owned()));
                }
            }
            _ => {
                properties.insert(key, text.to_owned());
            }
        }
    }
    properties.extend(conditional);
    properties
}

/// Tests as evaluated on a DX9, sRGB-capable renderer in HDR mode (mat_hdr_level 2, the
/// Source default and the owner's setting). Unknown tests are false, as in retail (which
/// also warns).
fn material_condition(test: &str, hdr: bool) -> bool {
    let (negate, test) = test.strip_prefix('!').map_or((false, test), |t| (true, t));
    let value = match test {
        "srgb" => true,
        "hdr" => hdr,
        "ldr" => !hdr,
        _ => false,
    };
    value != negate
}

/// DirectX-level blocks inside a material (`">=dx90_20b" { ... }`, `"<dx90" { ... }`):
/// whether the block applies on a DX level 95 renderer (SM3 HDR, the owner's native
/// setting). `dx90_20b` (ps2.0b) sits between levels 90 and 95. Other keys are not
/// DX-level blocks (None).
fn dx_level_block(key: &str) -> Option<bool> {
    const LEVEL: f32 = 95.;
    let key = key.to_ascii_lowercase();
    let (op, rest) = ["<=", ">=", "<", ">"]
        .into_iter()
        .find_map(|op| key.strip_prefix(op).map(|rest| (op, rest)))
        .unwrap_or(("=", key.as_str()));
    let digits = rest.strip_prefix("dx")?;
    let (number, b20) = digits
        .strip_suffix("_20b")
        .map_or((digits, false), |n| (n, true));
    let level = number.parse::<f32>().ok()? + if b20 { 0.5 } else { 0. };
    Some(match op {
        "<=" => LEVEL <= level,
        ">=" => LEVEL >= level,
        "<" => LEVEL < level,
        ">" => LEVEL > level,
        _ => LEVEL == level,
    })
}
/// Direct material vars plus the DX-level blocks that apply (their vars win).
pub(crate) fn material_properties(entries: &[Entry], hdr: bool) -> BTreeMap<String, String> {
    let mut properties = direct_properties(entries, hdr);
    for entry in entries {
        if matches!(entry.value, Value::Block(_)) && dx_level_block(&entry.key) == Some(true) {
            properties.extend(direct_properties(entry.children(), hdr));
        }
    }
    properties
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_assets::keyvalues;
    #[test]
    fn dx_level_blocks_apply_like_a_dx95_renderer() {
        assert_eq!(dx_level_block(">=dx90_20b"), Some(true));
        assert_eq!(dx_level_block("<dx90_20b"), Some(false));
        assert_eq!(dx_level_block("<dx90"), Some(false));
        assert_eq!(dx_level_block(">dx90"), Some(true));
        assert_eq!(dx_level_block("dx95"), Some(true));
        assert_eq!(dx_level_block("Proxies"), None);
        // combine_soldier/combinesoldiersheet.vmt: the base texture lives only there.
        let text = "\"VertexLitGeneric\" { \"$model\" 1 \">=dx90_20b\" { \"$baseTexture\" \"a\" } \"<dx90_20b\" { \"$baseTexture\" \"b\" } }";
        let entries = keyvalues::parse(text).unwrap();
        let p = material_properties(entries[0].children(), true);
        assert_eq!(p.get("$basetexture").map(String::as_str), Some("a"));
    }

    #[test]
    fn conditional_material_vars_follow_retail_tests() {
        let entries = keyvalues::parse(
            r#""srgb?$color2" "[2.5 2.5 2.5]" "$color2" "[1 1 1]" "hdr?$alpha" "0.5"
            "!hdr?$nocull" "1" "ldr?$additive" "1" "360?$translucent" "1" "dx9?$x" "1""#,
        )
        .unwrap();
        let p = direct_properties(&entries, true);
        assert_eq!(p["$color2"], "[2.5 2.5 2.5]");
        assert_eq!(p["$alpha"], "0.5");
        for absent in ["$nocull", "$additive", "$translucent", "$x", "srgb?$color2"] {
            assert!(!p.contains_key(absent), "{absent}");
        }
        // An LDR frame (mat_hdr_level below 2) takes the other branches.
        let p = direct_properties(&entries, false);
        assert_eq!(p["$color2"], "[2.5 2.5 2.5]");
        assert_eq!((p["$nocull"].as_str(), p["$additive"].as_str()), ("1", "1"));
        assert!(!p.contains_key("$alpha"));
    }
}
