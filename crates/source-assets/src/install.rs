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
/// The installation folder of the base game, whose search path every other game extends.
pub const BASE_GAME: &str = "hl2";

/// The games of an installation: the base game first, then every other folder with a
/// `gameinfo.txt` (`episodic`, `ep2`, `lostcoast`, ...), sorted by name.
pub fn games(root: &Path) -> Vec<String> {
    let mut others = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("gameinfo.txt").is_file())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|name| name != BASE_GAME)
        .collect::<Vec<_>>();
    others.sort();
    std::iter::once(BASE_GAME.to_owned())
        .chain(others)
        .collect()
}

/// The game whose own folder ships `maps/<map>.bsp`, loose or in one of its own VPKs, for map
/// names given without a game (`map`, `--map`). Like the engine, which searches the running
/// game first, `current` wins, then the base game, then the other games: Half-Life 2 and Lost
/// Coast both ship `background01`.
pub fn game_for_map(root: &Path, map: &str, current: Option<&str>) -> Option<String> {
    let path = crate::vpk::normalize(&format!("maps/{}.bsp", map.trim_end_matches(".bsp"))).ok()?;
    let mut games = games(root);
    if let Some(current) = current.and_then(|c| games.iter().position(|g| g == c)) {
        let current = games.remove(current);
        games.insert(0, current);
    }
    games.into_iter().find(|game| {
        let folder = root.join(game);
        folder.join(&path).is_file()
            || std::fs::read_dir(&folder)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().ends_with("_dir.vpk"))
                })
                .any(|p| crate::vpk::Vpk::open(&p).is_ok_and(|v| v.entries.contains_key(&path)))
    })
}

/// Every map name the installed games ship (`maps/<name>.bsp`, loose or in a game's own VPKs),
/// sorted, each name once: what the console's `map` completes from.
pub fn installed_maps(root: &Path) -> Vec<String> {
    let mut maps = std::collections::BTreeSet::new();
    let map_name = |path: &str| {
        let name = path.strip_prefix("maps/")?.strip_suffix(".bsp")?;
        (!name.contains('/')).then(|| name.to_owned())
    };
    for game in games(root) {
        let folder = root.join(&game);
        for entry in std::fs::read_dir(folder.join("maps"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let file = entry.file_name().to_string_lossy().to_ascii_lowercase();
            maps.extend(map_name(&format!("maps/{file}")));
        }
        for vpk in std::fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with("_dir.vpk"))
            })
            .filter_map(|p| crate::vpk::Vpk::open(&p).ok())
        {
            maps.extend(vpk.entries.keys().filter_map(|path| map_name(path)));
        }
    }
    maps.into_iter().collect()
}

/// A bounded launch-readiness check, not a complete content/integrity audit. `map` is the map
/// requested on the command line, if any; without one only the shared content is checked.
pub fn validate_content(path: PathBuf, map: Option<&str>) -> Result<PathBuf> {
    let root = validate(path)?;
    if let Some(map) = map {
        validate_map(&root, map)?;
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

/// The requested map exists in some installed game and is a BSP file.
fn validate_map(root: &Path, map: &str) -> Result<()> {
    if map.is_empty() || !map.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
        bail!("Use a simple map name, for example d1_trainstation_02");
    }
    // The map may belong to any installed game (Episode One/Two, Lost Coast): find the game
    // that ships it and read it through that game's own search path.
    let game = game_for_map(root, map, None).with_context(|| {
        format!("Missing map {map}.bsp. Select the full Half-Life 2 installation, or verify its files in Steam.")
    })?;
    let vfs = crate::vpk::Vfs::mount_game(root, &game).context("read game content archives")?;
    let map_ok = vfs
        .read(&format!("maps/{map}.bsp"))
        .ok()
        .flatten()
        .is_some_and(|bytes| bytes.starts_with(b"VBSP"));
    if !map_ok {
        bail!("Invalid map {game}/maps/{map}.bsp. Verify the game files in Steam.");
    }
    Ok(())
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
            Self::with_nonce(nonce)
        }
        fn with_nonce(nonce: u128) -> Self {
            for attempt in 0..128 {
                let root = std::env::temp_dir().join(format!(
                    "hl2-install-test-{}-{nonce}-{attempt}",
                    std::process::id()
                ));
                match std::fs::create_dir(&root) {
                    Ok(()) => return Self(root),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("reserve installation fixture: {error}"),
                }
            }
            panic!("installation fixture collision limit reached")
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
    fn fixtures_with_the_same_clock_nonce_keep_independent_files_and_lifetimes() {
        let first = Fixture::with_nonce(0);
        let second = Fixture::with_nonce(0);
        assert_ne!(first.0, second.0);
        first.put("marker", b"first");
        second.put("marker", b"second");
        assert_eq!(std::fs::read(first.0.join("marker")).unwrap(), b"first");
        drop(first);
        assert_eq!(std::fs::read(second.0.join("marker")).unwrap(), b"second");
    }

    #[test]
    fn installed_maps_list_every_games_loose_maps_once() {
        let fixture = Fixture::new();
        for (path, bytes) in [
            ("hl2/gameinfo.txt", &b"x"[..]),
            ("hl2/maps/background01.bsp", b"VBSP"),
            ("hl2/maps/d1_trainstation_01.bsp", b"VBSP"),
            ("hl2/maps/d1_trainstation_01.nav", b"x"),
            ("lostcoast/gameinfo.txt", b"x"),
            ("lostcoast/maps/BACKGROUND01.bsp", b"VBSP"),
            ("lostcoast/maps/d2_lostcoast.bsp", b"VBSP"),
            ("notagame/maps/hidden.bsp", b"VBSP"),
        ] {
            fixture.put(path, bytes);
        }
        assert_eq!(
            installed_maps(&fixture.0),
            ["background01", "d1_trainstation_01", "d2_lostcoast"]
        );
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
        assert!(validate_content(game.0.clone(), Some("../elsewhere")).is_err());
        game.put("hl2/maps/test.bsp", b"");
        assert!(validate_content(game.0.clone(), Some("test"))
            .unwrap_err()
            .to_string()
            .contains("map"));
        game.put("hl2/maps/test.bsp", b"VBSP\x14\0\0\0");
        assert!(validate_content(game.0.clone(), Some("test"))
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
        assert!(validate_content(game.0.clone(), Some("test")).is_ok());
        assert!(
            validate_content(game.0.clone(), None).is_ok(),
            "without a requested map only the shared content is checked"
        );
        assert!(validate_content(game.0.clone(), Some("missing")).is_err());
        game.put("hl2/models/weapons/v_pistol.mdl", b"");
        assert!(validate_content(game.0.clone(), Some("test")).is_err());
    }

    #[test]
    #[ignore = "requires an owned installed Half-Life 2 copy"]
    fn owned_install_passes_launch_readiness_check() {
        validate_content(discover().unwrap(), Some("d1_trainstation_02")).unwrap();
    }

    #[test]
    fn invalid_root_fails_validation() {
        let temp = std::env::temp_dir();
        assert!(validate(temp).is_err());
    }

    #[test]
    fn game_for_map_reads_each_game_folder_base_game_first() {
        let root = std::env::temp_dir().join(format!("hl2-rs-games-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (game, maps) in [
            ("hl2", &["background01", "d1_trainstation_01"][..]),
            ("lostcoast", &["background01", "d2_lostcoast"][..]),
            ("episodic", &["ep1_citadel_00"][..]),
            ("hl2_complete", &[][..]),
        ] {
            std::fs::create_dir_all(root.join(game).join("maps")).unwrap();
            std::fs::write(root.join(game).join("gameinfo.txt"), "").unwrap();
            for map in maps {
                std::fs::write(root.join(game).join(format!("maps/{map}.bsp")), "").unwrap();
            }
        }
        std::fs::create_dir_all(root.join("platform")).unwrap();
        assert_eq!(
            games(&root),
            ["hl2", "episodic", "hl2_complete", "lostcoast"]
        );
        let game = |map: &str| game_for_map(&root, map, None);
        assert_eq!(game("background01").as_deref(), Some("hl2"));
        assert_eq!(game("d2_lostcoast.bsp").as_deref(), Some("lostcoast"));
        assert_eq!(game("ep1_citadel_00").as_deref(), Some("episodic"));
        assert_eq!(game("ep2_outland_01"), None);
        assert_eq!(game("../hl2/maps/background01"), None);
        let from_lostcoast = |map: &str| game_for_map(&root, map, Some("lostcoast"));
        assert_eq!(from_lostcoast("background01").as_deref(), Some("lostcoast"));
        assert_eq!(from_lostcoast("d1_trainstation_01").as_deref(), Some("hl2"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires an owned Half-Life 2 installation"]
    fn owned_install_maps_belong_to_the_game_that_ships_them() {
        let root = discover().expect("owned Half-Life 2 installation");
        let game = |map: &str| game_for_map(&root, map, None);
        assert_eq!(game("d1_trainstation_01").as_deref(), Some("hl2"));
        assert_eq!(game("background01").as_deref(), Some("hl2"));
        assert_eq!(game("ep1_citadel_00").as_deref(), Some("episodic"));
        assert_eq!(game("ep2_outland_01").as_deref(), Some("ep2"));
        assert_eq!(game("d2_lostcoast").as_deref(), Some("lostcoast"));
        assert_eq!(game("no_such_map"), None);
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
