//! Which chapter a map belongs to, from the game's own files: its chapters in play order
//! (`scripts/chapterbackgrounds.txt`), each chapter's start map (`cfg/chapter<key>.cfg`) and the
//! level transitions between its maps (the `trigger_changelevel` entities in each map).
//!
//! Retail decides this with a table built into the game code (map-name prefix -> chapter
//! title). Here the maps' transition graph replaces it. Removing the chapter start maps splits
//! the graph into stretches of play; a stretch bordered by one start belongs to that chapter
//! (Episode Two revisits `ep2_outland_02`: the loop through 03 and 04 borders only chapter 2's
//! start). A stretch bordered by several starts exists only where a map bypasses a start
//! (Half-Life 2's `d3_c17_02_camera` has the transitions of chapter 10's start): its maps are
//! ordered by transitions from the earliest bordering start, and each takes the farthest
//! bordering start not beyond it (the earlier on a tie), so a chapter's tail before the next
//! start stays in its chapter; the bypassing map itself, as far as the start, takes the earlier. Maps no transition reaches (test and menu maps) belong to no chapter.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A chapter of a game: its key (`1`, `9a`, ...), its start map, if its `cfg/chapter<key>.cfg`
/// names one, and its menu background map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    pub key: String,
    pub start: Option<String>,
    pub background: String,
}

/// A game's chapters in play order: the entries of its `scripts/chapterbackgrounds.txt`
/// (`backgrounds`), each with the start map its `cfg/chapter<key>.cfg` names (`read` reads a file
/// through the game's own search path).
pub fn list(backgrounds: &str, read: impl Fn(&str) -> Option<String>) -> Vec<Chapter> {
    let Ok(roots) = source_assets::keyvalues::parse_resource(backgrounds.as_bytes()) else {
        return Vec::new();
    };
    roots
        .iter()
        .flat_map(|root| root.children())
        .filter_map(|entry| {
            let background = entry.text()?.trim();
            (!background.is_empty()).then(|| Chapter {
                key: entry.key.trim().to_owned(),
                start: read(&format!("cfg/chapter{}.cfg", entry.key.trim()))
                    .and_then(|text| super::resources::chapter_map(&text))
                    .map(|map| map.to_ascii_lowercase()),
                background: background.to_owned(),
            })
        })
        .collect()
}

/// The maps a map's `trigger_changelevel` entities lead to (lower case).
pub fn changelevel_targets(entities: &[modkit_core::Entity]) -> BTreeSet<String> {
    entities
        .iter()
        .filter(|e| e.class().eq_ignore_ascii_case("trigger_changelevel"))
        .filter_map(|e| e.get("map"))
        .map(|map| map.trim().to_ascii_lowercase())
        .filter(|map| !map.is_empty())
        .collect()
}

/// The transition graph of a game: each map in its own `maps` folder and the maps its level
/// transitions lead to. Maps that cannot be read are left out.
pub fn map_links(root: &Path, game: &str) -> BTreeMap<String, BTreeSet<String>> {
    let folder = root.join(game).join("maps");
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
            let map = name.strip_suffix(".bsp")?.to_owned();
            let entities = source_assets::bsp::read_entities(&path).ok()?;
            Some((map, changelevel_targets(&entities)))
        })
        .collect()
}

/// Map -> chapter key for every map connected to a chapter start (see the module docs).
pub fn assign(
    links: &BTreeMap<String, BTreeSet<String>>,
    chapters: &[Chapter],
) -> BTreeMap<String, String> {
    // Transitions work both ways for this purpose.
    let mut neighbours: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (map, targets) in links {
        for target in targets {
            neighbours.entry(map).or_default().insert(target);
            neighbours.entry(target).or_default().insert(map);
        }
    }
    // Chapter start maps in play order (a map that starts two chapters keeps the first), only
    // maps this game ships: a chapter config another game supplies through the search path
    // names none of them.
    let mut starts: Vec<(&str, &str)> = Vec::new();
    for chapter in chapters {
        if let Some(start) = chapter
            .start
            .as_deref()
            .filter(|s| links.contains_key(*s) || neighbours.contains_key(s))
        {
            if !starts.iter().any(|(map, _)| *map == start) {
                starts.push((start, &chapter.key));
            }
        }
    }
    let order = |map: &str| starts.iter().position(|(start, _)| *start == map);
    let around = |map: &str| neighbours.get(map).into_iter().flatten().copied();
    let mut chapter_of: BTreeMap<String, String> = starts
        .iter()
        .map(|(map, key)| ((*map).to_owned(), (*key).to_owned()))
        .collect();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for &map in neighbours.keys() {
        if order(map).is_some() || !seen.insert(map) {
            continue;
        }
        // One stretch: the maps reachable from `map` without entering a start map, and the
        // start maps on its border.
        let mut stretch = vec![map];
        let mut border: BTreeSet<usize> = BTreeSet::new();
        let mut queue = vec![map];
        while let Some(current) = queue.pop() {
            for next in around(current) {
                match order(next) {
                    Some(index) => {
                        border.insert(index);
                    }
                    None if seen.insert(next) => {
                        stretch.push(next);
                        queue.push(next);
                    }
                    None => {}
                }
            }
        }
        let Some(&first) = border.first() else {
            continue;
        };
        if border.len() == 1 {
            for map in stretch {
                chapter_of.insert(map.to_owned(), starts[first].1.to_owned());
            }
            continue;
        }
        // Several starts border it: a map that bypasses a start joined two chapters' stretches.
        // Order the stretch by transitions from its earliest bordering start; each map takes
        // the farthest bordering start not beyond it, the earlier one on a tie (Half-Life 2's
        // `d2_coast_08` is as far as chapter 8's start and returns to `d2_coast_07`).
        let inside: BTreeSet<&str> = stretch.iter().copied().collect();
        let mut distance: BTreeMap<&str, usize> = BTreeMap::from([(starts[first].0, 0)]);
        let mut queue = std::collections::VecDeque::from([starts[first].0]);
        while let Some(current) = queue.pop_front() {
            let d = distance[current] + 1;
            if current != starts[first].0 && order(current).is_some() {
                continue;
            }
            for next in around(current) {
                let bordering = order(next).is_some_and(|i| border.contains(&i));
                if (inside.contains(next) || bordering) && !distance.contains_key(next) {
                    distance.insert(next, d);
                    queue.push_back(next);
                }
            }
        }
        for map in stretch {
            let Some(&d) = distance.get(map) else {
                continue;
            };
            let chosen = border
                .iter()
                .filter_map(|&i| Some((*distance.get(starts[i].0)?, i)))
                // A start as far in as the map is not before it.
                .filter(|(start, _)| *start < d)
                .max()
                .map_or(first, |(_, i)| i);
            chapter_of.insert(map.to_owned(), starts[chosen].1.to_owned());
        }
    }
    chapter_of
}

/// Map -> chapter key of every game, built once on a background thread (reading the maps'
/// transitions takes a fraction of a second per game).
#[derive(Clone, Debug, Default)]
pub struct ChapterIndex(
    std::sync::Arc<std::sync::OnceLock<BTreeMap<String, BTreeMap<String, String>>>>,
);

impl ChapterIndex {
    /// Starts building the index of `chapters` (game -> chapters) for the installation at `root`.
    pub fn build(root: std::path::PathBuf, chapters: BTreeMap<String, Vec<Chapter>>) -> Self {
        let index = Self::default();
        let cell = index.0.clone();
        let spawned = std::thread::Builder::new()
            .name("chapter-index".into())
            .spawn(move || {
                let built = chapters
                    .iter()
                    .map(|(game, list)| (game.clone(), assign(&map_links(&root, game), list)))
                    .collect();
                let _ = cell.set(built);
            });
        if spawned.is_err() {
            return Self::default();
        }
        index
    }

    /// An index already built from `game -> map -> chapter key` assignments.
    pub fn ready(built: BTreeMap<String, BTreeMap<String, String>>) -> Self {
        let index = Self::default();
        let _ = index.0.set(built);
        index
    }

    /// The chapter of `game`'s `map`, once the index is built.
    pub fn get(&self, game: &str, map: &str) -> Option<Option<&str>> {
        let index = self.0.get()?;
        Some(
            index
                .get(game)
                .and_then(|maps| maps.get(&map.to_ascii_lowercase()))
                .map(String::as_str),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(key: &str, start: &str, background: &str) -> Chapter {
        Chapter {
            key: key.into(),
            start: Some(start.into()),
            background: background.into(),
        }
    }

    fn links(pairs: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
        pairs
            .iter()
            .map(|(map, targets)| {
                (
                    (*map).to_owned(),
                    targets.iter().map(|t| (*t).to_owned()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn stretches_and_bypasses_follow_the_chapter_starts() {
        // a1 -> a2 -> a3 | b1 -> b2 | c1 -> c2 -> c3, transitions both ways where retail has
        // them, a side map off a2, and an unconnected test map.
        let graph = links(&[
            ("a1", &["a2"]),
            ("a2", &["a1", "a3", "side"]),
            ("a3", &["a2", "b1"]),
            ("b1", &["a3", "b2"]),
            ("b2", &["b1", "c1"]),
            ("c1", &["c2"]),
            ("c2", &["c3"]),
            ("c3", &[]),
            // A second version of b1 with its transitions bypasses it; a3_side is a detour off a3
            // as far in as b1 (it returns to a3 before b1).
            ("b1_camera", &["a3", "b2"]),
            ("a3_side", &["a3"]),
            ("test", &[]),
        ]);
        let chapters = [
            chapter("1", "a1", "background01"),
            chapter("2", "b1", "background01"),
            chapter("9a", "c1", "background02"),
        ];
        let of = assign(&graph, &chapters);
        let key = |map: &str| of.get(map).map(String::as_str);
        assert_eq!(key("a1"), Some("1"));
        assert_eq!(
            key("a3"),
            Some("1"),
            "between chapter 1 and 2 starts: chapter 1"
        );
        assert_eq!(key("side"), Some("1"));
        assert_eq!(key("b2"), Some("2"), "after b1 and before c1");
        assert_eq!(
            key("b1_camera"),
            Some("1"),
            "as far in as b1: the earlier chapter"
        );
        assert_eq!(
            key("a3_side"),
            Some("1"),
            "a detour as far as the next start stays"
        );
        assert_eq!(
            key("c3"),
            Some("9a"),
            "after the last start: the last chapter"
        );
        assert_eq!(key("test"), None, "not connected to any chapter");
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_maps_belong_to_their_installed_chapters() {
        let root = source_assets::install::discover().expect("installed game");
        let vfs = source_assets::vpk::Vfs::mount(&root).unwrap();
        let res = super::super::resources::GameUiResources::load(&vfs);
        for (game, cases) in [
            (
                "hl2",
                &[
                    ("d1_trainstation_02", "1"),
                    ("d1_trainstation_05", "2"),
                    ("d2_coast_01", "7"),
                    ("d2_coast_04", "7"),
                    ("d2_coast_08", "7"),
                    ("d2_prison_01", "8"),
                    ("d2_prison_07", "9a"),
                    ("d3_c17_05", "10"),
                    ("d3_c17_10a", "11"),
                    ("d3_breen_01", "13"),
                ][..],
            ),
            (
                "episodic",
                &[("ep1_c17_02", "4"), ("ep1_citadel_01", "1")][..],
            ),
            (
                "ep2",
                &[("ep2_outland_01", "1"), ("ep2_outland_03", "2")][..],
            ),
            ("lostcoast", &[("d2_lostcoast", "1")][..]),
        ] {
            let chapters = &res.chapters[game];
            let started = std::time::Instant::now();
            let of = assign(&map_links(&root, game), chapters);
            eprintln!("{game}: {} maps in {:?}", of.len(), started.elapsed());
            for (map, key) in cases {
                assert_eq!(of.get(*map).map(String::as_str), Some(*key), "{game}/{map}");
            }
        }
        let background = |game: &str, key: &str| {
            res.chapters[game]
                .iter()
                .find(|c| c.key == key)
                .map(|c| c.background.clone())
        };
        assert_eq!(background("hl2", "7").as_deref(), Some("background04"));
    }

    #[test]
    fn changelevel_targets_come_from_trigger_changelevel_maps() {
        let entity = |pairs: &[(&str, &str)]| modkit_core::Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        };
        let targets = changelevel_targets(&[
            entity(&[
                ("classname", "trigger_changelevel"),
                ("map", "D1_Canals_02"),
            ]),
            entity(&[("classname", "trigger_changelevel"), ("map", " ")]),
            entity(&[("classname", "info_landmark"), ("map", "nope")]),
        ]);
        assert_eq!(targets.into_iter().collect::<Vec<_>>(), ["d1_canals_02"]);
    }
}
