use crate::binary::{self, Architecture, BinaryKind, MachOFileType, PeArchitecture};
use crate::keyvalues::{self, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_FILES_PER_GAME: usize = 50_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    NativeMac,
    LegacyMac32,
    WindowsPayload,
    Unknown,
}

impl Classification {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NativeMac => "native-mac",
            Self::LegacyMac32 => "legacy-mac-32",
            Self::WindowsPayload => "windows-payload",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedBinary {
    pub relative_path: PathBuf,
    pub kind: BinaryKind,
    pub executable: bool,
    pub pe_architecture: Option<PeArchitecture>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamGame {
    pub app_id: u32,
    pub name: String,
    pub install_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub state_flags: Option<u64>,
    pub build_id: Option<String>,
    pub installed_depots: BTreeMap<u32, u64>,
    pub classification: Classification,
    pub binaries: Vec<ObservedBinary>,
    pub inspection_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppManifestEvidence {
    pub app_id: u32,
    pub name: Option<String>,
    pub install_dir: Option<String>,
    pub build_id: Option<u64>,
    pub installed_depots: BTreeMap<u32, u64>,
}

#[derive(Debug)]
pub enum Error {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Vdf {
        path: PathBuf,
        source: keyvalues::Error,
    },
    InvalidManifest {
        path: PathBuf,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "{}: {}", path.display(), source),
            Self::Vdf { path, source } => write!(formatter, "{}: {}", path.display(), source),
            Self::InvalidManifest { path, message } => {
                write!(formatter, "{}: {}", path.display(), message)
            }
        }
    }
}

impl std::error::Error for Error {}

pub fn default_steam_root() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Steam")
    })
}

pub fn scan(steam_root: &Path) -> Result<Vec<SteamGame>, Error> {
    let primary_steamapps = normalize_steamapps(steam_root);
    let mut steamapps_dirs = BTreeSet::new();
    steamapps_dirs.insert(primary_steamapps.clone());

    let library_file = primary_steamapps.join("libraryfolders.vdf");
    if library_file.exists() {
        let document = read_vdf(&library_file)?;
        if let Some(libraries) = document.get("libraryfolders").and_then(Value::entries) {
            for (_, library) in libraries {
                if let Some(path) = library.string("path")
                    && let Some(path) = resolve_library_path(path, steam_root)
                {
                    let steamapps = normalize_steamapps(&path);
                    if steamapps.is_dir() {
                        steamapps_dirs.insert(steamapps);
                    }
                }
            }
        }
    }

    let mut games = Vec::new();
    for steamapps in steamapps_dirs {
        let entries = fs::read_dir(&steamapps).map_err(|source| Error::Io {
            path: steamapps.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| Error::Io {
                path: steamapps.clone(),
                source,
            })?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with("appmanifest_") && name.ends_with(".acf") {
                games.push(parse_manifest(&path, &steamapps)?);
            }
        }
    }
    games.sort_by_key(|game| game.app_id);
    Ok(games)
}

pub fn inspect_app_manifest(path: &Path) -> Result<AppManifestEvidence, Error> {
    let document = read_vdf(path)?;
    let app_state = document
        .get("AppState")
        .ok_or_else(|| Error::InvalidManifest {
            path: path.to_path_buf(),
            message: "missing AppState object".to_owned(),
        })?;
    let app_id = app_state
        .string("appid")
        .ok_or_else(|| Error::InvalidManifest {
            path: path.to_path_buf(),
            message: "missing appid".to_owned(),
        })?
        .parse()
        .map_err(|_| Error::InvalidManifest {
            path: path.to_path_buf(),
            message: "appid is not an unsigned integer".to_owned(),
        })?;
    let build_id = app_state
        .string("buildid")
        .map(|value| {
            value.parse().map_err(|_| Error::InvalidManifest {
                path: path.to_path_buf(),
                message: "buildid is not an unsigned integer".to_owned(),
            })
        })
        .transpose()?;
    let installed_depots = app_state
        .get("InstalledDepots")
        .and_then(Value::entries)
        .map(|entries| {
            entries
                .iter()
                .map(|(id, depot)| {
                    let id = id.parse().map_err(|_| Error::InvalidManifest {
                        path: path.to_path_buf(),
                        message: format!("depot id {id:?} is not an unsigned integer"),
                    })?;
                    let manifest = depot
                        .string("manifest")
                        .ok_or_else(|| Error::InvalidManifest {
                            path: path.to_path_buf(),
                            message: format!("depot {id} has no manifest"),
                        })?
                        .parse()
                        .map_err(|_| Error::InvalidManifest {
                            path: path.to_path_buf(),
                            message: format!("depot {id} manifest is not an unsigned integer"),
                        })?;
                    Ok((id, manifest))
                })
                .collect::<Result<BTreeMap<_, _>, Error>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(AppManifestEvidence {
        app_id,
        name: app_state.string("name").map(str::to_owned),
        install_dir: app_state.string("installdir").map(str::to_owned),
        build_id,
        installed_depots,
    })
}

fn resolve_library_path(configured: &str, steam_root: &Path) -> Option<PathBuf> {
    let bytes = configured.as_bytes();
    let is_windows_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    if !is_windows_absolute {
        return Some(PathBuf::from(configured));
    }

    let drive = bytes[0].to_ascii_lowercase() as char;
    let suffix = configured[3..]
        .split(['\\', '/'])
        .filter(|component| !component.is_empty())
        .collect::<PathBuf>();
    let normalized_root = if steam_root
        .file_name()
        .is_some_and(|name| name == "steamapps")
    {
        steam_root.parent().unwrap_or(steam_root)
    } else {
        steam_root
    };
    let drive_c = normalized_root
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == "drive_c"))?;
    if drive == 'c' {
        Some(drive_c.join(suffix))
    } else {
        let prefix = drive_c.parent()?;
        Some(
            prefix
                .join("dosdevices")
                .join(format!("{drive}:"))
                .join(suffix),
        )
    }
}

fn normalize_steamapps(path: &Path) -> PathBuf {
    if path.file_name().is_some_and(|name| name == "steamapps") {
        path.to_path_buf()
    } else {
        path.join("steamapps")
    }
}

fn read_vdf(path: &Path) -> Result<Value, Error> {
    let contents = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    keyvalues::parse(&contents).map_err(|source| Error::Vdf {
        path: path.to_path_buf(),
        source,
    })
}

fn parse_manifest(path: &Path, steamapps: &Path) -> Result<SteamGame, Error> {
    let document = read_vdf(path)?;
    let app_state = document
        .get("AppState")
        .ok_or_else(|| Error::InvalidManifest {
            path: path.to_path_buf(),
            message: "missing AppState object".to_owned(),
        })?;
    let required = |key: &str| {
        app_state.string(key).ok_or_else(|| Error::InvalidManifest {
            path: path.to_path_buf(),
            message: format!("missing {key}"),
        })
    };
    let app_id = required("appid")?
        .parse()
        .map_err(|_| Error::InvalidManifest {
            path: path.to_path_buf(),
            message: "appid is not an unsigned integer".to_owned(),
        })?;
    let name = required("name")?.to_owned();
    let install_dir = steamapps.join("common").join(required("installdir")?);
    let (binaries, inspection_truncated) = inspect_game_dir(&install_dir);
    let classification = classify(&binaries);
    let installed_depots = app_state
        .get("InstalledDepots")
        .and_then(Value::entries)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(id, depot)| {
                    Some((id.parse().ok()?, depot.string("manifest")?.parse().ok()?))
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(SteamGame {
        app_id,
        name,
        install_dir,
        manifest_path: path.to_path_buf(),
        state_flags: app_state
            .string("StateFlags")
            .and_then(|value| value.parse().ok()),
        build_id: app_state.string("buildid").map(str::to_owned),
        installed_depots,
        classification,
        binaries,
        inspection_truncated,
    })
}

fn inspect_game_dir(root: &Path) -> (Vec<ObservedBinary>, bool) {
    let mut binaries = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0;
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_FILES_PER_GAME {
                return (binaries, true);
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                // Macroton's depot views intentionally expose the verified
                // Windows payload through file symlinks. Follow only symlinks
                // whose target is a regular file; never recurse through a
                // directory symlink while scanning an untrusted library.
                let Ok(target_metadata) = fs::metadata(&path) else {
                    continue;
                };
                if !target_metadata.is_file() {
                    continue;
                }
            }
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !is_binary_candidate(&path) {
                continue;
            }
            let Ok(kind) = binary::inspect(&path) else {
                continue;
            };
            if kind != BinaryKind::Unknown {
                let pe_architecture = if kind == BinaryKind::PortableExecutable {
                    binary::inspect_pe_architecture(&path).ok().flatten()
                } else {
                    None
                };
                let executable = match &kind {
                    BinaryKind::PortableExecutable => true,
                    BinaryKind::MachO(_) => matches!(
                        binary::inspect_macho_file_type(&path),
                        Ok(Some(MachOFileType::Executable))
                    ),
                    BinaryKind::Unknown => false,
                };
                binaries.push(ObservedBinary {
                    relative_path: path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
                    kind,
                    executable,
                    pe_architecture,
                });
            }
        }
    }
    binaries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    (binaries, false)
}

fn is_binary_candidate(path: &Path) -> bool {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    {
        return true;
    }
    let rendered = path.to_string_lossy();
    if rendered.contains(".app/Contents/MacOS/") {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[allow(unreachable_code)]
    false
}

fn classify(binaries: &[ObservedBinary]) -> Classification {
    let mut i386 = false;
    let mut modern_mac = false;
    let mut windows = false;
    for binary in binaries {
        if !binary.executable {
            continue;
        }
        match &binary.kind {
            BinaryKind::PortableExecutable => windows = true,
            BinaryKind::MachO(architectures) => {
                i386 |= architectures.contains(&Architecture::I386);
                modern_mac |= architectures.contains(&Architecture::X86_64)
                    || architectures.contains(&Architecture::Arm64);
            }
            BinaryKind::Unknown => {}
        }
    }
    if modern_mac {
        Classification::NativeMac
    } else if i386 {
        Classification::LegacyMac32
    } else if windows {
        Classification::WindowsPayload
    } else {
        Classification::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("macroton-test-{nonce}"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn scans_primary_and_secondary_libraries() {
        let temp = TempDir::new();
        let primary = temp.0.join("Steam");
        let secondary = temp.0.join("External");
        fs::create_dir_all(primary.join("steamapps/common/Old Game/Game.app/Contents/MacOS"))
            .unwrap();
        fs::create_dir_all(secondary.join("steamapps/common/Windows Game")).unwrap();
        fs::write(
            primary.join("steamapps/libraryfolders.vdf"),
            format!(
                r#""libraryfolders" {{ "0" {{ "path" "{}" }} "1" {{ "path" "{}" }} }}"#,
                primary.display(),
                secondary.display()
            ),
        )
        .unwrap();
        fs::write(
            primary.join("steamapps/appmanifest_10.acf"),
            r#""AppState" { "appid" "10" "name" "Old Game" "installdir" "Old Game" "StateFlags" "4" }"#,
        )
        .unwrap();
        fs::write(
            secondary.join("steamapps/appmanifest_20.acf"),
            r#""AppState" { "appid" "20" "name" "Windows Game" "installdir" "Windows Game" }"#,
        )
        .unwrap();
        fs::write(
            primary.join("steamapps/common/Old Game/Game.app/Contents/MacOS/game"),
            [0xce, 0xfa, 0xed, 0xfe, 7, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0],
        )
        .unwrap();
        let mut pe32 = vec![0_u8; 0x80];
        pe32[0..2].copy_from_slice(b"MZ");
        pe32[0x3c..0x40].copy_from_slice(&0x40_u32.to_le_bytes());
        pe32[0x40..0x44].copy_from_slice(b"PE\0\0");
        pe32[0x44..0x46].copy_from_slice(&0x014c_u16.to_le_bytes());
        fs::write(
            secondary.join("steamapps/common/Windows Game/game.exe"),
            pe32,
        )
        .unwrap();

        let games = scan(&primary).unwrap();
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].classification, Classification::LegacyMac32);
        assert_eq!(games[1].classification, Classification::WindowsPayload);
        assert_eq!(
            games[1].binaries[0].pe_architecture,
            Some(PeArchitecture::I386)
        );
    }

    #[test]
    fn inspects_exact_build_and_depot_manifest_evidence() {
        let temp = TempDir::new();
        let manifest = temp.0.join("appmanifest_42.acf");
        fs::write(
            &manifest,
            r#""AppState" { "appid" "42" "name" "Fixture" "installdir" "Fixture" "buildid" "9" "InstalledDepots" { "43" { "manifest" "100" } "44" { "manifest" "200" } } }"#,
        )
        .unwrap();
        let evidence = inspect_app_manifest(&manifest).unwrap();
        assert_eq!(evidence.app_id, 42);
        assert_eq!(evidence.build_id, Some(9));
        assert_eq!(
            evidence.installed_depots,
            BTreeMap::from([(43, 100), (44, 200)])
        );
    }

    #[test]
    fn maps_windows_library_paths_back_into_the_wine_prefix() {
        let temp = TempDir::new();
        let prefix = temp.0.join("prefix");
        let steam = prefix.join("drive_c/Program Files (x86)/Steam");
        let steamapps = steam.join("steamapps");
        let game = steamapps.join("common/Mass Effect/Binaries");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            steamapps.join("libraryfolders.vdf"),
            r#""libraryfolders" { "0" { "path" "C:\\Program Files (x86)\\Steam" } }"#,
        )
        .unwrap();
        fs::write(
            steamapps.join("appmanifest_17460.acf"),
            r#""AppState" { "appid" "17460" "name" "Mass Effect" "installdir" "Mass Effect" "buildid" "2794" }"#,
        )
        .unwrap();
        fs::write(game.join("MassEffect.exe"), b"MZfixture").unwrap();

        let games = scan(&steam).unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].app_id, 17_460);
        assert_eq!(games[0].install_dir, steamapps.join("common/Mass Effect"));
    }

    #[cfg(unix)]
    #[test]
    fn scans_verified_pe_through_payload_view_file_symlink() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let source = temp.0.join("source");
        let view = temp.0.join("view");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&view).unwrap();
        let mut pe32 = vec![0_u8; 0x80];
        pe32[0..2].copy_from_slice(b"MZ");
        pe32[0x3c..0x40].copy_from_slice(&0x40_u32.to_le_bytes());
        pe32[0x40..0x44].copy_from_slice(b"PE\0\0");
        pe32[0x44..0x46].copy_from_slice(&0x014c_u16.to_le_bytes());
        fs::write(source.join("game.exe"), pe32).unwrap();
        symlink(source.join("game.exe"), view.join("game.exe")).unwrap();

        let (binaries, truncated) = inspect_game_dir(&view);
        assert!(!truncated);
        assert_eq!(binaries.len(), 1);
        assert_eq!(binaries[0].relative_path, PathBuf::from("game.exe"));
        assert_eq!(binaries[0].pe_architecture, Some(PeArchitecture::I386));
    }

    #[test]
    fn records_installed_depot_manifest_identity() {
        let temp = TempDir::new();
        let steam = temp.0.join("Steam");
        let steamapps = steam.join("steamapps");
        fs::create_dir_all(steamapps.join("common/Example")).unwrap();
        fs::write(
            steamapps.join("appmanifest_42.acf"),
            r#""AppState" { "appid" "42" "name" "Example" "installdir" "Example" "buildid" "9" "InstalledDepots" { "43" { "manifest" "100" } "44" { "manifest" "200" } } }"#,
        )
        .unwrap();

        let games = scan(&steam).unwrap();
        assert_eq!(games[0].installed_depots.get(&43), Some(&100));
        assert_eq!(games[0].installed_depots.get(&44), Some(&200));
    }

    #[test]
    fn ignores_an_unavailable_secondary_windows_library() {
        let temp = TempDir::new();
        let steam = temp.0.join("prefix/drive_c/Program Files (x86)/Steam");
        let steamapps = steam.join("steamapps");
        fs::create_dir_all(&steamapps).unwrap();
        fs::write(
            steamapps.join("libraryfolders.vdf"),
            r#""libraryfolders" { "0" { "path" "C:\\Program Files (x86)\\Steam" } "1" { "path" "D:\\SteamLibrary" } }"#,
        )
        .unwrap();

        assert!(scan(&steam).unwrap().is_empty());
    }

    #[test]
    fn modern_library_slice_does_not_hide_i386_executable() {
        let binaries = vec![
            ObservedBinary {
                relative_path: PathBuf::from("Silver.app/Contents/MacOS/Silver"),
                kind: BinaryKind::MachO(vec![Architecture::I386]),
                executable: true,
                pe_architecture: None,
            },
            ObservedBinary {
                relative_path: PathBuf::from("Silver.app/Contents/MacOS/libsteam_api.dylib"),
                kind: BinaryKind::MachO(vec![Architecture::I386, Architecture::X86_64]),
                executable: false,
                pe_architecture: None,
            },
        ];
        assert_eq!(classify(&binaries), Classification::LegacyMac32);
    }

    #[test]
    fn modern_native_mac_executable_takes_precedence_over_legacy_helpers() {
        // A game such as Baba Is You may ship legacy helper binaries beside
        // its current native Mac executable. The modern executable is the
        // authoritative target signal; do not route it through Macroton's
        // legacy Win32 compatibility path.
        let binaries = vec![
            ObservedBinary {
                relative_path: PathBuf::from("Game.app/Contents/MacOS/Game"),
                kind: BinaryKind::MachO(vec![Architecture::Arm64]),
                executable: true,
                pe_architecture: None,
            },
            ObservedBinary {
                relative_path: PathBuf::from("Game.app/Contents/MacOS/legacy-helper"),
                kind: BinaryKind::MachO(vec![Architecture::I386]),
                executable: true,
                pe_architecture: None,
            },
        ];
        assert_eq!(classify(&binaries), Classification::NativeMac);
    }
}
