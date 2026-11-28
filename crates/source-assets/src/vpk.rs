//! VPK v1/v2 reader with preload support, embedded data, and per-entry CRC checking.
use crate::{bytes, keyvalues, u16le, u32le};
use anyhow::{bail, Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
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
    archives: Mutex<HashMap<u16, Arc<(File, u64)>>>,
}
pub fn normalize(path: &str) -> Result<String> {
    let p = path.replace('\\', "/").to_lowercase();
    if p.starts_with('/') || p.contains(':') || p.split('/').any(|s| s == ".." || s == ".") {
        bail!("unsafe asset path: {path}");
    }
    Ok(p)
}
fn cstr<'a>(data: &'a [u8], pos: &mut usize) -> Result<&'a str> {
    let tail = data.get(*pos..).context("string offset outside tree")?;
    let len = tail
        .iter()
        .position(|b| *b == 0)
        .context("unterminated VPK string")?;
    let s = std::str::from_utf8(&tail[..len])?;
    *pos += len + 1;
    Ok(s)
}
fn build_vpk_path(directory: &str, stem: &str, extension: &str) -> Result<String> {
    let has_dir = directory != " ";
    let has_ext = extension != " ";
    let cap = (if has_dir { directory.len() + 1 } else { 0 })
        + stem.len()
        + (if has_ext { extension.len() + 1 } else { 0 });
    let mut out = String::with_capacity(cap);
    if has_dir {
        out.push_str(directory);
        out.push('/');
    }
    out.push_str(stem);
    if has_ext {
        out.push('.');
        out.push_str(extension);
    }
    if !out.is_ascii() || out.as_bytes().contains(&b'\\') {
        return normalize(&out);
    }
    if out.starts_with('/') || out.contains(':') || out.split('/').any(|s| s == ".." || s == ".") {
        bail!("unsafe asset path: {out}");
    }
    out.make_ascii_lowercase();
    Ok(out)
}
fn read_exact_at(file: &File, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut pos = offset;
        let mut rem = buf;
        while !rem.is_empty() {
            match file.seek_read(rem, pos) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "failed to fill whole buffer",
                    ));
                }
                Ok(n) => {
                    pos += n as u64;
                    rem = &mut rem[n..];
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        use std::io::{Seek, SeekFrom};
        let mut clone = file.try_clone()?;
        clone.seek(SeekFrom::Start(offset))?;
        clone.read_exact(buf)
    }
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
                    entries.insert(
                        build_vpk_path(directory, stem, extension)?,
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
            archives: Mutex::default(),
        })
    }
    fn archive_handle(&self, archive_index: u16) -> Result<Arc<(File, u64)>> {
        if let Some(cached) = self
            .archives
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&archive_index)
            .cloned()
        {
            return Ok(cached);
        }
        let archive = if archive_index == 0x7fff {
            self.path.clone()
        } else {
            let name = self
                .path
                .file_name()
                .context("VPK filename")?
                .to_string_lossy();
            let base = name
                .strip_suffix("_dir.vpk")
                .context("VPK directory filename must end in _dir.vpk")?;
            self.path
                .with_file_name(format!("{base}_{archive_index:03}.vpk"))
        };
        let file = File::open(&archive).with_context(|| format!("open {}", archive.display()))?;
        let len = file.metadata()?.len();
        let handle = Arc::new((file, len));
        let mut guard = self.archives.lock().unwrap_or_else(|e| e.into_inner());
        Ok(guard.entry(archive_index).or_insert(handle).clone())
    }
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.read_entry(name, None)
    }
    pub fn read_prefix(&self, name: &str, max_bytes: usize) -> Result<Option<Vec<u8>>> {
        self.read_entry(name, Some(max_bytes))
    }
    fn read_entry(&self, name: &str, max_bytes: Option<usize>) -> Result<Option<Vec<u8>>> {
        let Some(e) = self.entries.get(&normalize(name)?) else {
            return Ok(None);
        };
        if e.size as u64 + e.preload.len() as u64 > 512 * 1024 * 1024 {
            bail!("asset exceeds 512 MiB limit");
        }
        let limit = max_bytes.unwrap_or(usize::MAX);
        let preload_take = e.preload.len().min(limit);
        let mut data = e.preload[..preload_take].to_vec();
        let need = limit.saturating_sub(data.len()).min(e.size as usize);
        if e.size > 0 && need > 0 {
            let archive = self.archive_handle(e.archive)?;
            let base = if e.archive == 0x7fff {
                self.data_start as u64
            } else {
                0
            };
            let offset = base
                .checked_add(e.offset as u64)
                .context("entry offset overflow")?;
            let end = offset
                .checked_add(e.size as u64)
                .context("entry offset overflow")?;
            if end > archive.1 {
                bail!("truncated VPK archive entry {name}");
            }
            let old = data.len();
            data.resize(old + need, 0);
            read_exact_at(&archive.0, offset, &mut data[old..])?;
        }
        if max_bytes.is_none() && crc32fast::hash(&data) != e.crc {
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

#[derive(Default)]
pub struct Vfs {
    root: PathBuf,
    layers: Vec<Layer>,
    embedded: BTreeMap<String, Vec<u8>>,
    material_index: OnceLock<HashMap<String, Vec<String>>>,
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

fn collect_loose_files(
    root: &Path,
    start: PathBuf,
    prefix: &str,
    suffix: &str,
    found: &mut BTreeSet<String>,
) {
    let mut stack = vec![start];
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
                    .strip_prefix(root)
                    .ok()
                    .and_then(|p| normalize(&p.to_string_lossy()).ok());
                if let Some(name) = name.filter(|n| n.starts_with(prefix) && n.ends_with(suffix)) {
                    found.insert(name);
                }
            }
        }
    }
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
            material_index: OnceLock::new(),
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
            let name = normalize(&f.name()?)?;
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
        self.material_index = OnceLock::new();
        Ok(())
    }
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.read_layer_file(name, None)
    }

    pub fn read_prefix(&self, path: &str, max_bytes: usize) -> Result<Option<Vec<u8>>> {
        let name = normalize(path)?;
        if let Some(d) = self.embedded.get(&name) {
            return Ok(Some(d[..d.len().min(max_bytes)].to_vec()));
        }
        self.read_installed_prefix(&name, max_bytes)
    }

    /// Reads a normalized path from the installed search layers, excluding the active map's pak.
    pub(crate) fn read_installed_prefix(
        &self,
        name: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>> {
        for layer in &self.layers {
            match layer {
                Layer::Dir(folder) => {
                    let p = folder.join(name);
                    if p.is_file() {
                        let file = File::open(p)?;
                        let cap = file
                            .metadata()
                            .map_or(0, |m| (m.len() as usize).min(max_bytes));
                        let mut buf = Vec::with_capacity(cap);
                        file.take(max_bytes as u64).read_to_end(&mut buf)?;
                        return Ok(Some(buf));
                    }
                }
                Layer::Pak(vpk) => {
                    if let Some(d) = vpk.read_prefix(name, max_bytes)? {
                        return Ok(Some(d));
                    }
                }
            }
        }
        Ok(None)
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
                    collect_loose_files(folder, folder.join(prefix), prefix, suffix, &mut found);
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
    fn material_index(&self) -> &HashMap<String, Vec<String>> {
        self.material_index.get_or_init(|| {
            let mut index: HashMap<String, Vec<String>> = HashMap::new();
            let mut push_entry = |entry: &str| {
                let Some(material) = entry
                    .strip_prefix("materials/")
                    .and_then(|rest| rest.strip_suffix(".vmt"))
                else {
                    return;
                };
                let stem = material.rsplit('/').next().unwrap_or(material);
                let list = index.entry(stem.to_owned()).or_default();
                if !list.iter().any(|existing| existing == material) {
                    list.push(material.to_owned());
                }
            };
            for entry in self.embedded.keys() {
                push_entry(entry);
            }
            for layer in &self.layers {
                match layer {
                    Layer::Pak(vpk) => {
                        for entry in vpk.entries.keys() {
                            push_entry(entry);
                        }
                    }
                    Layer::Dir(folder) => {
                        let mut found = BTreeSet::new();
                        collect_loose_files(
                            folder,
                            folder.join("materials"),
                            "materials/",
                            ".vmt",
                            &mut found,
                        );
                        for entry in found {
                            push_entry(&entry);
                        }
                    }
                }
            }
            index
        })
    }
    pub fn resolve_material_name(&self, name: &str) -> Option<String> {
        let lower = name.replace('\\', "/").to_lowercase();
        let trimmed = lower
            .trim_start_matches("materials/")
            .trim_end_matches(".vmt");
        if trimmed.is_empty() {
            return None;
        }
        let stem = trimmed.rsplit('/').next().unwrap_or(trimmed);
        let candidates = self.material_index().get(stem)?;
        if trimmed.contains('/') {
            let suffix = format!("/{trimmed}");
            candidates
                .iter()
                .find(|m| *m == trimmed || m.ends_with(&suffix))
                .cloned()
        } else {
            candidates.first().cloned()
        }
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
                archives: Mutex::default(),
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
            material_index: OnceLock::new(),
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

    type VpkFixtureEntry<'a> = (&'a str, &'a str, &'a str, u16, &'a [u8], &'a [u8]);

    fn write_vpk_fixture(dir_path: &Path, archive_path: &Path, entries: &[VpkFixtureEntry<'_>]) {
        let mut tree = Vec::new();
        let mut embedded_data = Vec::new();
        let mut archive_data = Vec::new();
        for &(ext, dir, stem, archive, preload, body) in entries {
            let mut full = Vec::with_capacity(preload.len() + body.len());
            full.extend_from_slice(preload);
            full.extend_from_slice(body);
            let crc = crc32fast::hash(&full);
            let offset = if archive == 0x7fff {
                let o = embedded_data.len() as u32;
                embedded_data.extend_from_slice(body);
                o
            } else {
                let o = archive_data.len() as u32;
                archive_data.extend_from_slice(body);
                o
            };
            for part in [ext, dir, stem] {
                tree.extend_from_slice(part.as_bytes());
                tree.push(0);
            }
            tree.extend_from_slice(&crc.to_le_bytes());
            tree.extend_from_slice(&(preload.len() as u16).to_le_bytes());
            tree.extend_from_slice(&archive.to_le_bytes());
            tree.extend_from_slice(&offset.to_le_bytes());
            tree.extend_from_slice(&(body.len() as u32).to_le_bytes());
            tree.extend_from_slice(&0xffffu16.to_le_bytes());
            tree.extend_from_slice(preload);
            tree.push(0);
            tree.push(0);
        }
        tree.push(0);
        let mut dir_bytes = Vec::new();
        dir_bytes.extend_from_slice(&0x55aa1234u32.to_le_bytes());
        dir_bytes.extend_from_slice(&1u32.to_le_bytes());
        dir_bytes.extend_from_slice(&(tree.len() as u32).to_le_bytes());
        dir_bytes.extend_from_slice(&tree);
        dir_bytes.extend_from_slice(&embedded_data);
        std::fs::write(dir_path, dir_bytes).unwrap();
        std::fs::write(archive_path, archive_data).unwrap();
    }

    #[test]
    fn read_prefix_caches_handles_checks_bounds_and_respects_overrides() {
        let root = std::env::temp_dir().join(format!("hl2-rs-vpk-prefix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("hl2/maps")).unwrap();
        let dir_vpk = root.join("hl2/test_dir.vpk");
        let data_vpk = root.join("hl2/test_000.vpk");
        write_vpk_fixture(
            &dir_vpk,
            &data_vpk,
            &[
                ("bsp", "maps", "arc", 0, b"VB", b"SP_ARCHIVE_REST"),
                ("bsp", "maps", "emb", 0x7fff, b"", b"VBSP_EMBEDDED"),
                ("txt", " ", "readme", 0, b"PRELOAD_ONLY", b""),
            ],
        );
        let vpk = Vpk::open(&dir_vpk).unwrap();
        assert_eq!(
            vpk.read_prefix("maps/arc.bsp", 4).unwrap().unwrap(),
            b"VBSP"
        );
        assert_eq!(
            vpk.read_prefix("maps/emb.bsp", 4).unwrap().unwrap(),
            b"VBSP"
        );
        assert_eq!(
            vpk.read_prefix("readme.txt", 7).unwrap().unwrap(),
            b"PRELOAD"
        );
        // Both archive 0 and 0x7fff handles are cached once after reads.
        assert_eq!(vpk.archives.lock().unwrap().len(), 2);
        assert_eq!(
            vpk.read("maps/arc.bsp").unwrap().unwrap(),
            b"VBSP_ARCHIVE_REST"
        );
        assert_eq!(vpk.read("maps/emb.bsp").unwrap().unwrap(), b"VBSP_EMBEDDED");
        assert_eq!(vpk.archives.lock().unwrap().len(), 2);

        // Truncated archive still fails bounds validation in read_prefix.
        let trunc_dir = root.join("hl2/trunc_dir.vpk");
        let trunc_data = root.join("hl2/trunc_000.vpk");
        write_vpk_fixture(
            &trunc_dir,
            &trunc_data,
            &[("bsp", "maps", "bad", 0, b"", b"VBSP_LONG_DATA")],
        );
        std::fs::write(&trunc_data, b"VBSP").unwrap();
        let trunc_vpk = Vpk::open(&trunc_dir).unwrap();
        assert!(trunc_vpk.read_prefix("maps/bad.bsp", 4).is_err());

        // Vfs::read_prefix respects loose override and embedded pak override.
        std::fs::write(root.join("hl2/maps/arc.bsp"), b"LOOSE_FILE").unwrap();
        let vfs = Vfs {
            root: root.clone(),
            layers: vec![Layer::Dir(root.join("hl2")), Layer::Pak(vpk)],
            embedded: [("maps/emb.bsp".into(), b"PAK_OVERRIDE".to_vec())].into(),
            material_index: OnceLock::new(),
            warnings: Vec::new(),
        };
        assert_eq!(
            vfs.read_prefix("maps/arc.bsp", 5).unwrap().unwrap(),
            b"LOOSE"
        );
        assert_eq!(vfs.read_prefix("maps/emb.bsp", 3).unwrap().unwrap(), b"PAK");
        assert_eq!(
            vfs.read_installed_prefix("maps/emb.bsp", 4)
                .unwrap()
                .unwrap(),
            b"VBSP"
        );
        assert_eq!(
            vfs.read_prefix("readme.txt", 7).unwrap().unwrap(),
            b"PRELOAD"
        );
        assert!(vfs.read_prefix("missing.bin", 4).unwrap().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolve_material_name_preserves_precedence_and_pak_override() {
        let root = std::env::temp_dir().join(format!("hl2-rs-mat-idx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("hl2/materials/loose_dir")).unwrap();
        std::fs::write(root.join("hl2/materials/loose_dir/target.vmt"), b"VMT").unwrap();
        std::fs::write(root.join("hl2/materials/loose_dir/empty.vmt"), b"").unwrap();

        let dir_vpk1 = root.join("hl2/pak1_dir.vpk");
        let data_vpk1 = root.join("hl2/pak1_000.vpk");
        write_vpk_fixture(
            &dir_vpk1,
            &data_vpk1,
            &[
                ("vmt", "materials/vpk1", "shared", 0, b"1", b""),
                ("vmt", "materials/vpk1/sub", "nested", 0, b"1", b""),
                ("vmt", "materials/vpk1", "empty", 0, b"1", b""),
            ],
        );
        let dir_vpk2 = root.join("hl2/pak2_dir.vpk");
        let data_vpk2 = root.join("hl2/pak2_000.vpk");
        write_vpk_fixture(
            &dir_vpk2,
            &data_vpk2,
            &[
                ("vmt", "materials/vpk2", "shared", 0, b"2", b""),
                ("vmt", "materials/vpk2", "target", 0, b"2", b""),
            ],
        );

        let mut vfs = Vfs {
            root: root.clone(),
            layers: vec![
                Layer::Pak(Vpk::open(&dir_vpk1).unwrap()),
                Layer::Dir(root.join("hl2")),
                Layer::Pak(Vpk::open(&dir_vpk2).unwrap()),
            ],
            embedded: BTreeMap::new(),
            material_index: OnceLock::new(),
            warnings: Vec::new(),
        };
        // Earlier VPK wins over later VPK; loose dir wins over later VPK; zero-byte loose skipped.
        assert_eq!(
            vfs.resolve_material_name("shared").as_deref(),
            Some("vpk1/shared")
        );
        assert_eq!(
            vfs.resolve_material_name("target").as_deref(),
            Some("loose_dir/target")
        );
        assert_eq!(
            vfs.resolve_material_name("empty").as_deref(),
            Some("vpk1/empty")
        );
        assert_eq!(
            vfs.resolve_material_name("sub/nested").as_deref(),
            Some("vpk1/sub/nested")
        );

        // mount_pak invalidates cached material_index and takes highest precedence.
        let mut pak_bytes = Vec::new();
        {
            use std::io::Write;
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut pak_bytes));
            zip.start_file(
                "materials/pak_override/shared.vmt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"\"LightmappedGeneric\" {}").unwrap();
            zip.finish().unwrap();
        }
        vfs.mount_pak(&pak_bytes).unwrap();
        assert_eq!(
            vfs.resolve_material_name("shared").as_deref(),
            Some("pak_override/shared")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
