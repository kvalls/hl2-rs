use crate::keyvalues;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub fn candidate_steam_roots() -> Vec<PathBuf> {
    let mut steam_roots = vec![
        PathBuf::from(r"C:\Program Files (x86)\Steam"),
        PathBuf::from(r"C:\Program Files\Steam"),
    ];
    #[cfg(windows)]
    if let Ok(out) = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = s.lines().find(|l| l.contains("REG_SZ")) {
            if let Some((_, v)) = line.split_once("REG_SZ") {
                steam_roots.insert(0, PathBuf::from(v.trim()));
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let h = PathBuf::from(home);
        steam_roots.push(h.join("Library/Application Support/Steam"));
        steam_roots.push(h.join(".steam/steam"));
        steam_roots.push(h.join(".local/share/Steam"));
    }
    steam_roots
}

pub fn discover() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("HL2_ROOT") {
        return validate(PathBuf::from(p));
    }
    discover_steam()
}

/// Search Steam independently of a possibly stale explicit environment override.
pub fn discover_steam() -> Result<PathBuf> {
    discover_in(&candidate_steam_roots())
}

fn discover_in(steam_roots: &[PathBuf]) -> Result<PathBuf> {
    let mut libraries = steam_roots.to_vec();
    for root in steam_roots {
        if let Ok(s) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            if let Ok(t) = keyvalues::tokens(&s) {
                for p in t.windows(2) {
                    if p[0] == "path" {
                        libraries.push(PathBuf::from(&p[1]));
                    }
                }
            }
        }
    }
    libraries.dedup();
    for library in libraries {
        let manifest = library.join("steamapps/appmanifest_220.acf");
        let dir = std::fs::read_to_string(&manifest)
            .ok()
            .and_then(|s| keyvalues::value(&s, "installdir").ok().flatten())
            .unwrap_or("Half-Life 2".into());
        let candidate = library.join("steamapps/common").join(dir);
        if let Ok(validated) = validate(candidate) {
            return Ok(validated);
        }
    }
    bail!("Half-Life 2 not found; pass --game <install-folder> or set HL2_ROOT")
}
pub fn validate(path: PathBuf) -> Result<PathBuf> {
    let path = if path.join("gameinfo.txt").is_file()
        && path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("hl2"))
    {
        path.canonicalize()
            .context("resolve HL2 content folder")?
            .parent()
            .context("HL2 folder has no parent")?
            .to_path_buf()
    } else {
        path
    };
    if !path.join("hl2/gameinfo.txt").is_file() {
        bail!(
            "{} is not an HL2 install (missing hl2/gameinfo.txt)",
            path.display()
        );
    }
    path.canonicalize().context("resolve game root")
}

/// A bounded launch-readiness check, not a complete content/integrity audit.
pub fn validate_content(path: PathBuf, map: &str) -> Result<PathBuf> {
    let root = validate(path)?;
    if map.is_empty() || !map.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
        bail!("Use a simple map name, for example d1_trainstation_02");
    }
    let map_path = root.join("hl2/maps").join(format!("{map}.bsp"));
    use std::io::Read;
    let mut header = [0; 8];
    let map_ok = std::fs::File::open(&map_path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok()
        && &header[..4] == b"VBSP";
    if !map_ok {
        bail!("Missing or invalid map hl2/maps/{map}.bsp. Select the full Half-Life 2 installation, or verify its files in Steam.");
    }
    let vfs = crate::vpk::Vfs::mount(&root).context("read game content archives")?;
    for file in [
        "scripts/game_sounds_manifest.txt",
        "resource/ClientScheme.res",
        "models/weapons/v_pistol.mdl",
    ] {
        if vfs
            .read(file)
            .with_context(|| {
                format!("Cannot read required content {file}; verify the game files in Steam")
            })?
            .is_none_or(|bytes| bytes.is_empty())
        {
            bail!("Required content is missing or empty: {file}. Select the full installation, or verify the game files in Steam.");
        }
    }
    Ok(root)
}
pub fn maps(root: &Path) -> Result<Vec<PathBuf>> {
    let mut maps = std::fs::read_dir(root.join("hl2/maps"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "bsp"))
        .collect::<Vec<_>>();
    maps.sort();
    Ok(maps)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("hl2-install-test-{}-{nonce}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn put(&self, name: &str, bytes: &[u8]) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovers_manifest_named_install_in_secondary_library() {
        let steam = Fixture::new();
        let library = Fixture::new();
        steam.put(
            "steamapps/libraryfolders.vdf",
            format!(
                "\"libraryfolders\" {{ \"1\" {{ \"path\" \"{}\" }} }}",
                library.0.display().to_string().replace('\\', "\\\\")
            )
            .as_bytes(),
        );
        library.put(
            "steamapps/appmanifest_220.acf",
            b"\"AppState\" { \"installdir\" \"Custom HL2 Name\" }",
        );
        library.put(
            "steamapps/common/Custom HL2 Name/hl2/gameinfo.txt",
            b"gameinfo",
        );
        assert_eq!(
            discover_in(std::slice::from_ref(&steam.0)).unwrap(),
            library
                .0
                .join("steamapps/common/Custom HL2 Name")
                .canonicalize()
                .unwrap()
        );
    }

    #[test]
    fn normalizes_hl2_subfolder_and_rejects_incomplete_content_and_bad_maps() {
        let game = Fixture::new();
        game.put("hl2/gameinfo.txt", b"gameinfo");
        assert_eq!(
            validate(game.0.join("hl2")).unwrap(),
            game.0.canonicalize().unwrap()
        );
        assert!(validate_content(game.0.clone(), "../elsewhere").is_err());
        game.put("hl2/maps/test.bsp", b"");
        assert!(validate_content(game.0.clone(), "test")
            .unwrap_err()
            .to_string()
            .contains("map"));
        game.put("hl2/maps/test.bsp", b"VBSP\x14\0\0\0");
        assert!(validate_content(game.0.clone(), "test")
            .unwrap_err()
            .to_string()
            .contains("Required content"));
        for name in [
            "scripts/game_sounds_manifest.txt",
            "resource/clientscheme.res",
            "models/weapons/v_pistol.mdl",
        ] {
            game.put(&format!("hl2/{name}"), b"fixture");
        }
        assert!(validate_content(game.0.clone(), "test").is_ok());
        game.put("hl2/models/weapons/v_pistol.mdl", b"");
        assert!(validate_content(game.0.clone(), "test").is_err());
    }

    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_install_passes_launch_readiness_check() {
        validate_content(discover().unwrap(), "d1_trainstation_02").unwrap();
    }

    #[test]
    fn invalid_root_fails_validation() {
        let temp = std::env::temp_dir();
        assert!(validate(temp).is_err());
    }

    #[test]
    fn candidate_roots_include_macos_steam_path_when_home_is_present() {
        if let Some(home) = std::env::var_os("HOME") {
            let roots = candidate_steam_roots();
            let expected = PathBuf::from(home).join("Library/Application Support/Steam");
            assert!(roots.contains(&expected));
        }
    }
}
