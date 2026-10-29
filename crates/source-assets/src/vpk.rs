//! VPK v1/v2 reader with preload support, embedded data, and per-entry CRC checking.
use crate::{bytes, keyvalues, u16le, u32le};
use anyhow::{bail, Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Entry {
    pub crc: u32,
    pub archive: u16,
    pub offset: u32,
    pub size: u32,
    pub preload: Vec<u8>,
}
pub struct Vpk {
    pub path: PathBuf,
    pub version: u32,
    pub entries: BTreeMap<String, Entry>,
    data_start: usize,
}
pub fn normalize(path: &str) -> Result<String> {
    let p = path.replace('\\', "/").to_lowercase();
    if p.starts_with('/') || p.contains(':') || p.split('/').any(|s| s == ".." || s == ".") {
        bail!("unsafe asset path: {path}");
    }
    Ok(p)
}
fn cstr(data: &[u8], pos: &mut usize) -> Result<String> {
    let tail = data.get(*pos..).context("string offset outside tree")?;
    let len = tail
        .iter()
        .position(|b| *b == 0)
        .context("unterminated VPK string")?;
    let s = std::str::from_utf8(&tail[..len])?.to_string();
    *pos += len + 1;
    Ok(s)
}
impl Vpk {
    pub fn open(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)?;
        if u32le(&data, 0)? != 0x55aa1234 {
            bail!("invalid VPK signature");
        }
        let version = u32le(&data, 4)?;
        let header = match version {
            1 => 12,
            2 => 28,
            _ => bail!("unsupported VPK version {version}"),
        };
        let tree_size = u32le(&data, 8)? as usize;
        let tree = bytes(&data, header, tree_size)?;
        let mut p = 0;
        let mut entries = BTreeMap::new();
        loop {
            let extension = cstr(tree, &mut p)?;
            if extension.is_empty() {
                break;
            }
            loop {
                let directory = cstr(tree, &mut p)?;
                if directory.is_empty() {
                    break;
                }
                loop {
                    let stem = cstr(tree, &mut p)?;
                    if stem.is_empty() {
                        break;
                    }
                    let crc = u32le(tree, p)?;
                    let preload = u16le(tree, p + 4)? as usize;
                    let archive = u16le(tree, p + 6)?;
                    let offset = u32le(tree, p + 8)?;
                    let size = u32le(tree, p + 12)?;
                    if u16le(tree, p + 16)? != 0xffff {
                        bail!("VPK entry terminator missing");
                    }
                    p += 18;
                    let preload = bytes(tree, p, preload)?.to_vec();
                    p += preload.len();
                    let name = format!(
                        "{}{}{}",
                        if directory == " " {
                            String::new()
                        } else {
                            format!("{directory}/")
                        },
                        stem,
                        if extension == " " {
                            String::new()
                        } else {
                            format!(".{extension}")
                        }
                    );
                    entries.insert(
                        normalize(&name)?,
                        Entry {
                            crc,
                            archive,
                            offset,
                            size,
                            preload,
                        },
                    );
                }
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            version,
            entries,
            data_start: header + tree_size,
        })
    }
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(e) = self.entries.get(&normalize(name)?) else {
            return Ok(None);
        };
        if e.size as u64 + e.preload.len() as u64 > 512 * 1024 * 1024 {
            bail!("asset exceeds 512 MiB limit");
        }
        let mut data = e.preload.clone();
        if e.size > 0 {
            let (archive, offset) = if e.archive == 0x7fff {
                (self.path.clone(), self.data_start as u64 + e.offset as u64)
            } else {
                let name = self
                    .path
                    .file_name()
                    .context("VPK filename")?
                    .to_string_lossy();
                let base = name
                    .strip_suffix("_dir.vpk")
                    .context("VPK directory filename must end in _dir.vpk")?;
                (
                    self.path
                        .with_file_name(format!("{base}_{:03}.vpk", e.archive)),
                    e.offset as u64,
                )
            };
            let mut file =
                File::open(&archive).with_context(|| format!("open {}", archive.display()))?;
            let end = offset
                .checked_add(e.size as u64)
                .context("entry offset overflow")?;
            if end > file.metadata()?.len() {
                bail!("truncated VPK archive entry {name}");
            }
            file.seek(SeekFrom::Start(offset))?;
            let old = data.len();
            data.resize(old + e.size as usize, 0);
            file.read_exact(&mut data[old..])?;
        }
        if crc32fast::hash(&data) != e.crc {
            bail!("VPK CRC mismatch for {name}");
        }
        Ok(Some(data))
    }
}

/// One entry of a game's search path: a loose folder or a VPK.
enum Layer {
    Dir(PathBuf),
    Pak(Vpk),
}

pub struct Vfs {
    root: PathBuf,
    layers: Vec<Layer>,
    embedded: BTreeMap<String, Vec<u8>>,
    pub warnings: Vec<String>,
}

/// The search path a game declares in `<game>/gameinfo.txt` (`GameInfo/FileSystem/SearchPaths`),
/// in order, as paths under the installation `root`. Only `game`, `mod` and `platform` entries
/// are content; `gamebin`, the low-violence `game_lv` and `root_mod` are not. `|gameinfo_path|`
/// is the game's own folder and `|all_source_engine_paths|` the installation. A game without a
/// readable search path gets its own folder, then `hl2`, then `platform`.
pub fn search_paths(root: &Path, game: &str) -> Vec<PathBuf> {
    let declared = std::fs::read(root.join(game).join("gameinfo.txt"))
        .ok()
        .and_then(|bytes| keyvalues::parse_resource(&bytes).ok())
        .map(|roots| {
            roots
                .iter()
                .flat_map(|info| info.children())
                .filter(|e| e.key.eq_ignore_ascii_case("FileSystem"))
                .flat_map(|fs| fs.children())
                .filter(|e| e.key.eq_ignore_ascii_case("SearchPaths"))
                .flat_map(|paths| paths.children())
                .filter(|e| {
                    e.key.split('+').any(|id| {
                        ["game", "mod", "platform"]
                            .iter()
                            .any(|content| id.eq_ignore_ascii_case(content))
                    })
                })
                .filter_map(|e| e.text())
                .map(|value| {
                    let value = value.trim();
                    if let Some(rest) = value.strip_prefix("|gameinfo_path|") {
                        root.join(game).join(rest)
                    } else {
                        root.join(value.trim_start_matches("|all_source_engine_paths|"))
                    }
                })
                .collect::<Vec<_>>()
        })
        .filter(|paths| !paths.is_empty());
    declared.unwrap_or_else(|| {
        [game, crate::install::BASE_GAME, "platform"]
            .iter()
            .map(|folder| root.join(folder))
            .collect()
    })
}

/// A numbered VPK data file (`*_000.vpk`), opened through its `*_dir.vpk`.
fn vpk_data_file(path: &Path) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.rsplit_once('_'))
        .is_some_and(|(_, n)| n.len() == 3 && n.bytes().all(|b| b.is_ascii_digit()))
}

impl Vfs {
    /// Mounts the base Half-Life 2 search path of the installation.
    pub fn mount(root: &Path) -> Result<Self> {
        Self::mount_game(root, crate::install::BASE_GAME)
    }

    /// Mounts the search path of the game installed in folder `game` of the installation
    /// (`hl2`, `episodic`, `ep2`, `lostcoast`), as its `gameinfo.txt` declares it
    /// (see [`search_paths`]). Episode Two, for example, searches its own VPKs, then Episode
    /// One's and Half-Life 2's, then the loose `ep2`, `episodic` and `hl2` folders.
    pub fn mount_game(root: &Path, game: &str) -> Result<Self> {
        let mut vfs = Self {
            root: root.to_path_buf(),
            layers: Vec::new(),
            embedded: BTreeMap::new(),
            warnings: Vec::new(),
        };
        let mut mounted = std::collections::BTreeSet::new();
        for path in search_paths(root, game) {
            vfs.add(path, &mut mounted)?;
        }
        Ok(vfs)
    }

    /// Adds one search path entry: `<folder>/*` mounts the folder's subfolders and VPKs in
    /// name order, `<name>.vpk` its `<name>_dir.vpk`, anything else a loose folder. Entries
    /// that are not installed (another language's voice pack) are skipped, repeats too.
    fn add(
        &mut self,
        path: PathBuf,
        mounted: &mut std::collections::BTreeSet<PathBuf>,
    ) -> Result<()> {
        if path.file_name().is_some_and(|n| n == "*") {
            let Some(folder) = path.parent().filter(|f| f.is_dir()) else {
                return Ok(());
            };
            let mut entries = std::fs::read_dir(folder)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir() || p.extension().is_some_and(|e| e == "vpk"))
                .filter(|p| !vpk_data_file(p))
                .collect::<Vec<_>>();
            entries.sort();
            for entry in entries {
                self.add(entry, mounted)?;
            }
        } else if path.extension().is_some_and(|e| e == "vpk") {
            let dir = path.with_file_name(format!(
                "{}_dir.vpk",
                path.file_stem().unwrap_or_default().to_string_lossy()
            ));
            let Some(file) = [dir, path].into_iter().find(|p| p.is_file()) else {
                return Ok(());
            };
            if mounted.insert(file.clone()) {
                match Vpk::open(&file) {
                    Ok(vpk) => self.layers.push(Layer::Pak(vpk)),
                    Err(e) => self.warnings.push(format!("{}: {e}", file.display())),
                }
            }
        } else if path.is_dir() && mounted.insert(path.clone()) {
            self.layers.push(Layer::Dir(path));
        }
        Ok(())
    }

    pub fn mount_pak(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data))?;
        let mut total = 0;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            if f.is_dir() {
                continue;
            }
            let name = normalize(f.name())?;
            if f.size() > 64 * 1024 * 1024 {
                bail!("embedded asset too large");
            }
            total += f.size();
            if total > 256 * 1024 * 1024 {
                bail!("map pak exceeds limit");
            }
            let mut bytes = Vec::new();
            f.read_to_end(&mut bytes)?;
            self.embedded.insert(name, bytes);
        }
        Ok(())
    }
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.read_layer_file(name, None)
    }

    /// Read a MOD-owned file, excluding inherited games and map-embedded resources.
    pub fn read_game_file(&self, game: &str, name: &str) -> Result<Option<Vec<u8>>> {
        self.read_layer_file(name, Some(&self.root.join(game)))
    }

    fn content_layers<'a>(
        &'a self,
        own_folder: Option<&'a Path>,
    ) -> impl Iterator<Item = &'a Layer> {
        self.layers.iter().filter(move |layer| {
            let path = match layer {
                Layer::Dir(path) => path,
                Layer::Pak(vpk) => &vpk.path,
            };
            own_folder.is_none_or(|folder| path.starts_with(folder))
        })
    }

    fn read_layer_file(&self, name: &str, own_folder: Option<&Path>) -> Result<Option<Vec<u8>>> {
        let name = normalize(name)?;
        if let Some(d) = self.embedded.get(&name).filter(|_| own_folder.is_none()) {
            return Ok(Some(d.clone()));
        }
        for layer in self.content_layers(own_folder) {
            match layer {
                Layer::Dir(folder) => {
                    let p = folder.join(&name);
                    if p.is_file() {
                        return Ok(Some(std::fs::read(p)?));
                    }
                }
                Layer::Pak(vpk) => {
                    if let Some(d) = vpk.read(&name)? {
                        return Ok(Some(d));
                    }
                }
            }
        }
        Ok(None)
    }
    /// Every copy of `name` on the search path, in search order (a map's embedded file first),
    /// for resources that each search path contributes, such as caption dictionaries.
    pub fn read_all(&self, name: &str) -> Result<Vec<Vec<u8>>> {
        let name = normalize(name)?;
        let mut copies = Vec::new();
        if let Some(d) = self.embedded.get(&name) {
            copies.push(d.clone());
        }
        for layer in &self.layers {
            match layer {
                Layer::Dir(folder) => {
                    let p = folder.join(&name);
                    if p.is_file() {
                        copies.push(std::fs::read(p)?);
                    }
                }
                Layer::Pak(vpk) => {
                    if let Some(d) = vpk.read(&name)? {
                        copies.push(d);
                    }
                }
            }
        }
        Ok(copies)
    }
    /// The installation root passed to [`Vfs::mount_game`].
    pub fn game_root(&self) -> Option<&Path> {
        Some(&self.root)
    }
    fn archives(&self) -> impl Iterator<Item = &Vpk> {
        self.layers.iter().filter_map(|layer| match layer {
            Layer::Pak(vpk) => Some(vpk),
            Layer::Dir(_) => None,
        })
    }
    pub fn archive_counts(&self) -> Vec<(String, usize)> {
        self.archives()
            .map(|v| {
                (
                    v.path.file_name().unwrap().to_string_lossy().to_string(),
                    v.entries.len(),
                )
            })
            .collect()
    }
    pub fn base_texture(&self, material: &str) -> Result<Option<String>> {
        self.material_texture(material, 0)
    }
    pub fn material_value(&self, material: &str, key: &str) -> Result<Option<String>> {
        self.material_property(material, key, 0)
    }
    fn material_property(&self, material: &str, key: &str, depth: usize) -> Result<Option<String>> {
        if depth > 8 {
            bail!("VMT include cycle/depth limit");
        }
        let path = format!(
            "materials/{}.vmt",
            material
                .trim_end_matches(".vmt")
                .trim_start_matches("materials/")
        );
        let Some(data) = self.read(&path)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&data)?;
        if let Some(v) = keyvalues::value(text, key)? {
            return Ok(Some(v));
        }
        if let Some(include) = keyvalues::value(text, "include")? {
            return self.material_property(&include, key, depth + 1);
        }
        Ok(None)
    }
    /// Every mounted file under `prefix` (a normalized directory such as "models/")
    /// ending in `suffix`: map pak, custom folders/VPKs, base VPKs and loose folders.
    /// Zero-byte loose placeholders are skipped (they never shadow archived files).
    pub fn list_files(&self, prefix: &str, suffix: &str) -> BTreeSet<String> {
        self.list_layer_files(prefix, suffix, None)
    }

    /// Files belonging to one game's own mounted folders/custom packs/archives (MOD),
    /// excluding inherited games and map-embedded content. Used for chapter discovery.
    pub fn list_game_files(&self, game: &str, prefix: &str, suffix: &str) -> BTreeSet<String> {
        self.list_layer_files(prefix, suffix, Some(&self.root.join(game)))
    }

    fn list_layer_files(
        &self,
        prefix: &str,
        suffix: &str,
        own_folder: Option<&Path>,
    ) -> BTreeSet<String> {
        let wanted = |p: &str| p.starts_with(prefix) && p.ends_with(suffix);
        let mut found: BTreeSet<String> = self
            .embedded
            .keys()
            .filter(|p| own_folder.is_none() && wanted(p))
            .cloned()
            .collect();
        for layer in self.content_layers(own_folder) {
            match layer {
                Layer::Pak(vpk) => {
                    for entry in vpk.entries.keys() {
                        if wanted(entry) {
                            found.insert(entry.clone());
                        }
                    }
                }
                Layer::Dir(folder) => {
                    let mut stack = vec![folder.join(prefix)];
                    while let Some(dir) = stack.pop() {
                        let Ok(read) = std::fs::read_dir(&dir) else {
                            continue;
                        };
                        for entry in read.flatten() {
                            let path = entry.path();
                            if path.is_dir() {
                                stack.push(path);
                            } else if entry.metadata().is_ok_and(|m| m.len() > 0) {
                                let name = path
                                    .strip_prefix(folder)
                                    .ok()
                                    .and_then(|p| normalize(&p.to_string_lossy()).ok());
                                if let Some(name) = name.filter(|n| wanted(n)) {
                                    found.insert(name);
                                }
                            }
                        }
                    }
                }
            }
        }
        found
    }
    /// Models that can spawn as physics props (`prop_physics_create`: CreatePhysicsProp
    /// requires the model's vcollide, i.e. a .phy beside the .mdl), relative to models/
    /// and grouped by folder.
    pub fn physics_prop_models(&self) -> BTreeMap<String, Vec<String>> {
        let phys = self.list_files("models/", ".phy");
        let mut folders: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for mdl in self.list_files("models/", ".mdl") {
            if !phys.contains(&format!("{}.phy", mdl.trim_end_matches(".mdl"))) {
                continue;
            }
            let relative = &mdl["models/".len()..];
            let folder = relative.rsplit_once('/').map_or("", |(f, _)| f);
            folders
                .entry(folder.to_owned())
                .or_default()
                .push(relative.to_owned());
        }
        folders
    }
    pub fn resolve_material_name(&self, name: &str) -> Option<String> {
        let suffix = format!("/{}.vmt", name.to_lowercase());
        let mut found = self
            .archives()
            .flat_map(|a| a.entries.keys())
            .filter(|p| p.starts_with("materials/") && p.ends_with(&suffix));
        let first = found.next()?;
        if found.next().is_some() {
            return None;
        }
        Some(
            first
                .trim_start_matches("materials/")
                .trim_end_matches(".vmt")
                .into(),
        )
    }
    fn material_texture(&self, material: &str, depth: usize) -> Result<Option<String>> {
        if depth > 8 {
            bail!("VMT include cycle/depth limit");
        }
        let path = format!(
            "materials/{}.vmt",
            material
                .trim_end_matches(".vmt")
                .trim_start_matches("materials/")
        );
        let Some(data) = self.read(&path)? else {
            return Ok(None);
        };
        let text = std::str::from_utf8(&data)?;
        if let Some(v) = keyvalues::value(text, "$basetexture")? {
            return Ok(Some(v));
        }
        if let Some(include) = keyvalues::value(text, "include")? {
            return self.material_texture(&include, depth + 1);
        }
        Ok(None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal() {
        assert!(normalize("../save/x").is_err());
        assert!(normalize("C:\\x").is_err());
        assert_eq!(normalize("Materials\\A.VTF").unwrap(), "materials/a.vtf");
    }
    #[test]
    fn rejects_truncated_vpk() {
        let p = std::env::temp_dir().join(format!("hl2-rs-vpk-{}.vpk", std::process::id()));
        std::fs::write(&p, b"bad").unwrap();
        assert!(Vpk::open(&p).is_err());
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn chapter_discovery_lists_only_the_games_own_nonempty_files() {
        let root = std::env::temp_dir().join(format!("hl2-rs-own-chapters-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for folder in ["ep2/cfg", "ep2/custom/test/cfg", "hl2/cfg"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        for (file, contents) in [
            ("ep2/cfg/chapter1.cfg", "map own"),
            ("ep2/cfg/chapter2.cfg", ""),
            ("ep2/custom/test/cfg/chapter9a.cfg", "map custom"),
            ("hl2/cfg/chapter14.cfg", "map inherited"),
            ("hl2/cfg/chapter1.cfg", "map wrong"),
        ] {
            std::fs::write(root.join(file), contents).unwrap();
        }
        let archive = |game: &str, name: &str| {
            Layer::Pak(Vpk {
                path: root.join(game).join("test_dir.vpk"),
                version: 2,
                data_start: 0,
                entries: [(
                    name.to_owned(),
                    Entry {
                        crc: 0,
                        archive: 0x7fff,
                        offset: 0,
                        size: 1,
                        preload: Vec::new(),
                    },
                )]
                .into(),
            })
        };
        let vfs = Vfs {
            root: root.clone(),
            layers: vec![
                Layer::Dir(root.join("ep2/custom/test")),
                Layer::Dir(root.join("ep2")),
                Layer::Dir(root.join("hl2")),
                archive("ep2", "cfg/chapter3.cfg"),
                archive("hl2", "cfg/chapter15.cfg"),
            ],
            embedded: [
                ("cfg/chapter16.cfg".into(), vec![1]),
                ("cfg/chapter1.cfg".into(), b"map embedded".to_vec()),
            ]
            .into(),
            warnings: Vec::new(),
        };
        assert_eq!(
            vfs.list_game_files("ep2", "cfg/", ".cfg"),
            ["cfg/chapter1.cfg", "cfg/chapter3.cfg", "cfg/chapter9a.cfg"]
                .map(String::from)
                .into()
        );
        assert_eq!(
            vfs.read("cfg/chapter1.cfg").unwrap().unwrap(),
            b"map embedded"
        );
        assert_eq!(
            vfs.read_game_file("ep2", "cfg/chapter1.cfg")
                .unwrap()
                .unwrap(),
            b"map own"
        );
        assert!(vfs
            .read_game_file("ep2", "cfg/chapter14.cfg")
            .unwrap()
            .is_none());
        assert!(vfs
            .read_game_file("ep2", "cfg/chapter16.cfg")
            .unwrap()
            .is_none());
        assert!(vfs.list_files("cfg/", ".cfg").contains("cfg/chapter16.cfg"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn games_mount_the_search_paths_their_gameinfo_declares() {
        let root = std::env::temp_dir().join(format!("hl2-rs-gameinfo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for folder in ["hl2", "episodic", "ep2/custom/mymod", "platform"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        std::fs::write(
            root.join("ep2/gameinfo.txt"),
            r#""GameInfo" { FileSystem { SearchPaths {
                game+mod    ep2/custom/*
                game_lv     hl2/hl2_lv.vpk
                game+mod    ep2/ep2_pak.vpk
                mod+mod_write+default_write_path |gameinfo_path|.
                game+game_write ep2
                gamebin     episodic/bin
                game        |all_source_engine_paths|episodic
                game        |all_source_engine_paths|hl2
                platform    |all_source_engine_paths|platform
            } } }"#,
        )
        .unwrap();
        let declared = search_paths(&root, "ep2");
        let rel = |p: &PathBuf| {
            p.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        };
        assert_eq!(
            declared.iter().map(rel).collect::<Vec<_>>(),
            [
                "ep2/custom/*",
                "ep2/ep2_pak.vpk",
                "ep2",
                "ep2",
                "episodic",
                "hl2",
                "platform"
            ]
        );
        // Loose files follow the declared order; the missing VPK and the repeated `ep2` folder
        // are skipped, the custom folder comes first.
        std::fs::write(root.join("episodic/a.txt"), "episodic").unwrap();
        std::fs::write(root.join("hl2/a.txt"), "hl2").unwrap();
        std::fs::write(root.join("hl2/b.txt"), "hl2").unwrap();
        std::fs::write(root.join("ep2/b.txt"), "ep2").unwrap();
        std::fs::write(root.join("ep2/custom/mymod/b.txt"), "custom").unwrap();
        let vfs = Vfs::mount_game(&root, "ep2").unwrap();
        let read = |name| String::from_utf8(vfs.read(name).unwrap().unwrap()).unwrap();
        assert_eq!(read("a.txt"), "episodic");
        assert_eq!(read("b.txt"), "custom");
        assert_eq!(vfs.game_root(), Some(root.as_path()));
        // A game without a search path falls back to its folder, then hl2 and platform.
        let episodic = Vfs::mount_game(&root, "episodic").unwrap();
        assert_eq!(
            episodic.read("a.txt").unwrap().as_deref(),
            Some(&b"episodic"[..])
        );
        assert_eq!(
            episodic.read("b.txt").unwrap().as_deref(),
            Some(&b"hl2"[..])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_install_games_mount_vpks_before_loose_folders() {
        let root = crate::install::discover().expect("owned install");
        let rel = |p: &PathBuf| {
            p.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        };
        let order = search_paths(&root, "ep2")
            .iter()
            .map(rel)
            .collect::<Vec<_>>();
        let at = |path: &str| {
            order
                .iter()
                .position(|p| p == path)
                .unwrap_or_else(|| panic!("{path} not in {order:?}"))
        };
        assert!(at("ep2/ep2_pak.vpk") < at("episodic/ep1_pak.vpk"));
        assert!(at("episodic/ep1_pak.vpk") < at("hl2/hl2_pak.vpk"));
        assert!(at("platform/platform_misc.vpk") < at("ep2"));
        assert!(at("ep2") < at("episodic") && at("episodic") < at("hl2"));
        assert!(!order
            .iter()
            .any(|p| p.ends_with("bin") || p.ends_with("hl2_lv.vpk")));
        let lostcoast = search_paths(&root, "lostcoast")
            .iter()
            .map(rel)
            .collect::<Vec<_>>();
        assert!(
            !lostcoast.iter().any(|p| p.starts_with("episodic")),
            "{lostcoast:?}"
        );
        for (game, map) in [
            ("hl2", "maps/background01.bsp"),
            ("episodic", "maps/ep1_citadel_00.bsp"),
            ("ep2", "maps/ep2_outland_01.bsp"),
            ("ep2", "maps/ep1_citadel_00.bsp"),
            ("lostcoast", "maps/d2_lostcoast.bsp"),
        ] {
            let vfs = Vfs::mount_game(&root, game).unwrap();
            assert!(vfs.read(map).unwrap().is_some(), "{game}: {map}");
        }
        let lostcoast = Vfs::mount_game(&root, "lostcoast").unwrap();
        assert!(lostcoast.read("maps/ep1_citadel_00.bsp").unwrap().is_none());
        // The installation ships zero-byte loose placeholders for some weapon models; the
        // archived models searched first must win over them.
        let hl2 = Vfs::mount(&root).unwrap();
        for model in [
            "models/weapons/v_crossbow.mdl",
            "models/weapons/v_bugbait.mdl",
        ] {
            let bytes = hl2.read(model).unwrap().unwrap_or_default();
            assert!(
                !bytes.is_empty(),
                "{model} resolved to an empty placeholder"
            );
        }
    }
}
