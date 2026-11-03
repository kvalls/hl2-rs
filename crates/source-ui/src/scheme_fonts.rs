//! Scheme fonts resolved to installed font files.
//!
//! A VGUI scheme names fonts by face and weight (`Fonts/MenuLarge/1 { name GorDIN weight 600 }`)
//! and registers the font files it ships under `CustomFontFiles`. A face is drawn from the
//! registered file whose family matches the name and whose weight is closest to the requested
//! one, as the installed system would pick among registered fonts. No host fonts are read.

use source_assets::keyvalues::Entry;

/// A scheme font request: face name and weight (CSS-style 100-900; 0 means regular).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemeFont {
    pub name: String,
    pub weight: u16,
}

/// The first definition of the named scheme font (`Fonts/<name>/1`).
pub fn scheme_font(root: &Entry, name: &str) -> Option<SchemeFont> {
    let definition = root.get("Fonts")?.get(name)?.children().first()?;
    let face = definition.get("name")?.text()?.trim();
    if face.is_empty() {
        return None;
    }
    let weight = definition
        .get("weight")
        .and_then(Entry::text)
        .and_then(|w| w.trim().parse::<u16>().ok())
        .unwrap_or(0);
    Some(SchemeFont {
        name: face.to_owned(),
        weight,
    })
}

/// The character-cell height (`tall`) of the first definition of the named scheme font (`Fonts/<name>/1`).
pub fn scheme_font_tall(root: &Entry, name: &str) -> Option<f32> {
    root.get("Fonts")?
        .get(name)?
        .children()
        .first()?
        .get("tall")
        .and_then(Entry::text)
        .and_then(|t| t.trim().parse::<f32>().ok())
        .filter(|&t| t > 0.)
}

/// The font files a scheme registers (`CustomFontFiles`), in listed order.
pub fn custom_font_files(root: &Entry) -> Vec<String> {
    root.get("CustomFontFiles")
        .map(|files| {
            files
                .children()
                .iter()
                .filter_map(|entry| entry.text())
                .map(|path| path.trim().to_owned())
                .filter(|path| !path.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Family names and weight of a font file, from its `name` and `OS/2` tables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFileInfo {
    pub families: Vec<String>,
    pub weight: u16,
}

impl FontFileInfo {
    pub fn read(bytes: &[u8]) -> Option<Self> {
        let face = ttf_parser::Face::parse(bytes, 0).ok()?;
        let families = face
            .names()
            .into_iter()
            .filter(|n| {
                n.name_id == ttf_parser::name_id::TYPOGRAPHIC_FAMILY
                    || n.name_id == ttf_parser::name_id::FAMILY
            })
            .filter_map(|n| n.to_string())
            .collect::<Vec<_>>();
        (!families.is_empty()).then(|| Self {
            families,
            weight: face.weight().to_number(),
        })
    }

    pub fn matches(&self, name: &str) -> bool {
        self.families.iter().any(|f| f.eq_ignore_ascii_case(name))
    }
}

/// The candidate drawn for `font`: a matching family, closest weight (a scheme weight of 0 asks
/// for regular, 400); the first listed wins a tie.
pub fn pick(candidates: &[FontFileInfo], font: &SchemeFont) -> Option<usize> {
    let wanted = if font.weight == 0 { 400 } else { font.weight };
    candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.matches(&font.name))
        .min_by_key(|(_, c)| c.weight.abs_diff(wanted))
        .map(|(i, _)| i)
}

/// Bytes of the registered font file drawn for the scheme font `name`.
pub fn resolve(
    root: &Entry,
    name: &str,
    read: impl Fn(&str) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let font = scheme_font(root, name)?;
    let files: Vec<(Vec<u8>, FontFileInfo)> = custom_font_files(root)
        .iter()
        .filter_map(|path| read(path))
        .filter_map(|bytes| FontFileInfo::read(&bytes).map(|info| (bytes, info)))
        .collect();
    let infos: Vec<FontFileInfo> = files.iter().map(|(_, info)| info.clone()).collect();
    let index = pick(&infos, &font)?;
    files.into_iter().nth(index).map(|(bytes, _)| bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_assets::keyvalues::parse_resource;

    fn info(family: &str, weight: u16) -> FontFileInfo {
        FontFileInfo {
            families: vec![family.to_owned()],
            weight,
        }
    }

    #[test]
    fn scheme_fonts_and_registered_files_come_from_the_scheme() {
        let root = parse_resource(
            br#"Scheme {
                Fonts { MenuLarge { "1" { name GorDIN tall 12 weight 600 } "2" { name Other } } }
                CustomFontFiles { "1" "resource/HALFLIFE2.ttf" "13" "resource/GorDIN-Semibold.ttf" }
            }"#,
        )
        .unwrap()
        .remove(0);
        assert_eq!(
            scheme_font(&root, "MenuLarge"),
            Some(SchemeFont {
                name: "GorDIN".into(),
                weight: 600
            })
        );
        assert_eq!(scheme_font(&root, "Missing"), None);
        assert_eq!(
            custom_font_files(&root),
            ["resource/HALFLIFE2.ttf", "resource/GorDIN-Semibold.ttf"]
        );
    }

    #[test]
    fn the_matching_family_with_the_closest_weight_is_drawn() {
        let files = [
            info("HalfLife2", 400),
            info("GorDIN", 400),
            info("GorDIN", 700),
            info("GorDIN", 600),
            info("Liberation Sans", 600),
        ];
        let font = |name: &str, weight| SchemeFont {
            name: name.into(),
            weight,
        };
        assert_eq!(pick(&files, &font("GorDIN", 600)), Some(3));
        assert_eq!(
            pick(&files, &font("gordin", 0)),
            Some(1),
            "weight 0 is regular"
        );
        assert_eq!(pick(&files, &font("GorDIN", 900)), Some(2));
        assert_eq!(
            pick(&files, &font("Verdana", 400)),
            None,
            "never another family"
        );
    }
}
