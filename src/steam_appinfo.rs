use crate::profile::CompatibilityProfile;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const APPINFO_MAGIC_V29: u32 = 0x0756_4429;
const CACHE_HEADER_SIZE: usize = 16;
const RECORD_METADATA_SIZE: usize = 65;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchEntry {
    pub executable: String,
    pub working_dir: Option<String>,
    pub description: Option<String>,
    pub os_list: Option<String>,
    pub os_arch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepotEntry {
    pub depot_id: u32,
    pub manifest_id: String,
    pub size: Option<u64>,
    pub language: Option<String>,
    pub os_list: Option<String>,
    pub dlc_app_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableReference {
    pub metadata_path: String,
    pub executable: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSaveRule {
    pub rule_id: String,
    pub root: Option<String>,
    pub path: Option<String>,
    pub pattern: Option<String>,
    pub recursive: Option<bool>,
    pub platforms: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudRootOverride {
    pub rule_id: String,
    pub root: Option<String>,
    pub os: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMetadata {
    pub app_id: u32,
    pub cache_path: PathBuf,
    pub name: Option<String>,
    pub app_os_list: Option<String>,
    pub install_dir: Option<String>,
    pub public_build_id: Option<String>,
    pub launches: Vec<LaunchEntry>,
    pub executable_references: Vec<ExecutableReference>,
    pub windows_public_depots: Vec<DepotEntry>,
    pub cloud_save_rules: Vec<CloudSaveRule>,
    pub cloud_root_overrides: Vec<CloudRootOverride>,
}

pub fn profile_mismatches(metadata: &AppMetadata, profile: &CompatibilityProfile) -> Vec<String> {
    let mut mismatches = Vec::new();
    if metadata.app_id != profile.app_id {
        mismatches.push(format!(
            "cached AppID {} does not match profile AppID {}",
            metadata.app_id, profile.app_id
        ));
    }
    if let Some(expected) = &profile.windows_build_id
        && metadata.public_build_id.as_ref() != Some(expected)
    {
        mismatches.push(format!(
            "cached public BuildID {} does not match profile BuildID {expected}",
            metadata.public_build_id.as_deref().unwrap_or("missing")
        ));
    }
    if let Some(expected) = &profile.steam_install_dir
        && metadata.install_dir.as_ref() != Some(expected)
    {
        mismatches.push(format!(
            "cached install directory {} does not match profile {expected}",
            metadata.install_dir.as_deref().unwrap_or("missing")
        ));
    }
    let expected_executable = profile.executable.to_string_lossy().replace('\\', "/");
    if !metadata
        .executable_references
        .iter()
        .any(|reference| reference.executable.replace('\\', "/") == expected_executable)
    {
        mismatches.push(format!(
            "cached executable metadata does not contain profile executable {expected_executable}"
        ));
    }
    for expected in &profile.windows_depots {
        let observed = metadata
            .windows_public_depots
            .iter()
            .find(|depot| depot.depot_id == expected.id);
        match (observed, expected.manifest_id) {
            (None, _) => mismatches.push(format!(
                "cached public metadata does not contain profile depot {}",
                expected.id
            )),
            (Some(observed), Some(expected_manifest))
                if observed.manifest_id != expected_manifest.to_string() =>
            {
                mismatches.push(format!(
                    "cached depot {} manifest {} does not match profile {}",
                    expected.id, observed.manifest_id, expected_manifest
                ));
            }
            _ => {}
        }
    }
    mismatches
}

#[derive(Debug)]
pub struct Error {
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub fn inspect(app_id: u32, data_root: &Path) -> Result<AppMetadata, Error> {
    let steam_root = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix/drive_c/Program Files (x86)/Steam");
    inspect_steam_root(app_id, &steam_root)
}

pub fn inspect_steam_root(app_id: u32, steam_root: &Path) -> Result<AppMetadata, Error> {
    let cache_path = steam_root.join("appcache/appinfo.vdf");
    let bytes = fs::read(&cache_path).map_err(|error| Error {
        message: format!(
            "cannot read Steam app-info cache {}: {error}",
            cache_path.display()
        ),
    })?;
    parse(app_id, &cache_path, &bytes)
}

fn parse(app_id: u32, cache_path: &Path, bytes: &[u8]) -> Result<AppMetadata, Error> {
    if read_u32(bytes, 0)? != APPINFO_MAGIC_V29 {
        return Err(error("unsupported Steam app-info cache format"));
    }
    let string_table_offset = read_u32(bytes, 8)? as usize;
    if !(CACHE_HEADER_SIZE..bytes.len()).contains(&string_table_offset) {
        return Err(error("Steam app-info string table offset is invalid"));
    }
    let strings = parse_string_table(bytes, string_table_offset)?;
    let (record_start, record_size) = find_record(bytes, string_table_offset, app_id)?;
    if record_size < RECORD_METADATA_SIZE {
        return Err(error("Steam app-info record metadata is truncated"));
    }
    let record_end = record_start
        .checked_add(8)
        .and_then(|value| value.checked_add(record_size))
        .filter(|value| *value <= string_table_offset)
        .ok_or_else(|| error("Steam app-info record exceeds the cache"))?;
    let leading_marker = record_start + 8 + RECORD_METADATA_SIZE - 1;
    if bytes.get(leading_marker) != Some(&0) {
        return Err(error("Steam app-info record has an invalid payload marker"));
    }
    let mut cursor = leading_marker + 1;
    let mut leaves = Vec::new();
    parse_object(
        bytes,
        &strings,
        &mut cursor,
        record_end,
        &mut Vec::new(),
        &mut leaves,
    )?;
    build_metadata(app_id, cache_path, &leaves)
}

fn parse_string_table(bytes: &[u8], offset: usize) -> Result<Vec<String>, Error> {
    let count = read_u32(bytes, offset)? as usize;
    let mut cursor = offset + 4;
    let mut strings = Vec::with_capacity(count);
    for _ in 0..count {
        strings.push(read_cstring(bytes, &mut cursor, bytes.len())?);
    }
    Ok(strings)
}

fn find_record(bytes: &[u8], table_offset: usize, app_id: u32) -> Result<(usize, usize), Error> {
    let mut cursor = CACHE_HEADER_SIZE;
    while cursor + 8 <= table_offset {
        let current_app_id = read_u32(bytes, cursor)?;
        if current_app_id == 0 {
            break;
        }
        let size = read_u32(bytes, cursor + 4)? as usize;
        if current_app_id == app_id {
            return Ok((cursor, size));
        }
        cursor = cursor
            .checked_add(8)
            .and_then(|value| value.checked_add(size))
            .filter(|value| *value <= table_offset)
            .ok_or_else(|| error("Steam app-info record chain is invalid"))?;
    }
    Err(error(format!(
        "Steam app-info cache does not contain AppID {app_id}"
    )))
}

#[derive(Debug, Clone, PartialEq)]
struct Leaf {
    path: Vec<String>,
    value: String,
}

fn parse_object(
    bytes: &[u8],
    strings: &[String],
    cursor: &mut usize,
    end: usize,
    path: &mut Vec<String>,
    leaves: &mut Vec<Leaf>,
) -> Result<(), Error> {
    loop {
        let value_type = read_byte(bytes, cursor, end)?;
        if value_type == 8 {
            return Ok(());
        }
        let key_index = read_u32_bounded(bytes, cursor, end)? as usize;
        let key = strings
            .get(key_index)
            .ok_or_else(|| error(format!("Steam app-info key index {key_index} is invalid")))?
            .clone();
        path.push(key);
        match value_type {
            0 => parse_object(bytes, strings, cursor, end, path, leaves)?,
            1 => leaves.push(Leaf {
                path: path.clone(),
                value: read_cstring(bytes, cursor, end)?,
            }),
            2 => leaves.push(numeric_leaf(
                path,
                read_u32_bounded(bytes, cursor, end)? as i32,
            )),
            3 => {
                let bits = read_u32_bounded(bytes, cursor, end)?;
                leaves.push(Leaf {
                    path: path.clone(),
                    value: f32::from_bits(bits).to_string(),
                });
            }
            4 | 6 => leaves.push(numeric_leaf(path, read_u32_bounded(bytes, cursor, end)?)),
            7 | 10 => leaves.push(numeric_leaf(path, read_u64_bounded(bytes, cursor, end)?)),
            9 => leaves.push(numeric_leaf(
                path,
                read_u64_bounded(bytes, cursor, end)? as i64,
            )),
            other => {
                return Err(error(format!(
                    "unsupported Steam app-info value type {other} at {}",
                    path.join("/")
                )));
            }
        }
        path.pop();
    }
}

fn numeric_leaf(path: &[String], value: impl ToString) -> Leaf {
    Leaf {
        path: path.to_vec(),
        value: value.to_string(),
    }
}

fn build_metadata(app_id: u32, cache_path: &Path, leaves: &[Leaf]) -> Result<AppMetadata, Error> {
    let value = |wanted: &[&str]| {
        leaves
            .iter()
            .find(|leaf| path_matches(&leaf.path, wanted))
            .map(|leaf| leaf.value.clone())
    };
    let mut launches: BTreeMap<String, LaunchEntry> = BTreeMap::new();
    let mut depots: BTreeMap<u32, DepotEntry> = BTreeMap::new();
    let mut cloud_save_rules: BTreeMap<String, CloudSaveRule> = BTreeMap::new();
    let mut cloud_root_overrides: BTreeMap<String, CloudRootOverride> = BTreeMap::new();
    for leaf in leaves {
        if leaf.path.len() >= 4 && path_matches(&leaf.path[..2], &["config", "launch"]) {
            let launch = launches.entry(leaf.path[2].clone()).or_insert(LaunchEntry {
                executable: String::new(),
                working_dir: None,
                description: None,
                os_list: None,
                os_arch: None,
            });
            match leaf.path[3..].as_ref() {
                [field] if field == "executable" => launch.executable.clone_from(&leaf.value),
                [field] if field == "workingdir" => launch.working_dir = Some(leaf.value.clone()),
                [field] if field == "description" => launch.description = Some(leaf.value.clone()),
                [object, field] if object == "config" && field == "oslist" => {
                    launch.os_list = Some(leaf.value.clone());
                }
                [object, field] if object == "config" && field == "osarch" => {
                    launch.os_arch = Some(leaf.value.clone());
                }
                _ => {}
            }
        }
        if leaf.path.len() >= 3 && leaf.path[0] == "depots" {
            let Ok(depot_id) = leaf.path[1].parse::<u32>() else {
                continue;
            };
            let depot = depots.entry(depot_id).or_insert(DepotEntry {
                depot_id,
                manifest_id: String::new(),
                size: None,
                language: None,
                os_list: None,
                dlc_app_id: None,
            });
            match leaf.path[2..].as_ref() {
                [object, field] if object == "config" && field == "language" => {
                    depot.language = Some(leaf.value.clone());
                }
                [object, field] if object == "config" && field == "oslist" => {
                    depot.os_list = Some(leaf.value.clone());
                }
                [object, branch, field]
                    if object == "manifests" && branch == "public" && field == "gid" =>
                {
                    depot.manifest_id.clone_from(&leaf.value);
                }
                [object, branch, field]
                    if object == "manifests" && branch == "public" && field == "size" =>
                {
                    depot.size = leaf.value.parse().ok();
                }
                [field] if field == "dlcappid" => depot.dlc_app_id = leaf.value.parse().ok(),
                _ => {}
            }
        }
        if leaf.path.len() == 4 && path_matches(&leaf.path[..2], &["ufs", "savefiles"]) {
            let rule = cloud_save_rules
                .entry(leaf.path[2].clone())
                .or_insert(CloudSaveRule {
                    rule_id: leaf.path[2].clone(),
                    root: None,
                    path: None,
                    pattern: None,
                    recursive: None,
                    platforms: None,
                });
            match leaf.path[3].as_str() {
                "root" => rule.root = Some(leaf.value.clone()),
                "path" => rule.path = Some(leaf.value.clone()),
                "pattern" => rule.pattern = Some(leaf.value.clone()),
                "recursive" => rule.recursive = parse_bool(&leaf.value),
                "platforms" => rule.platforms = Some(leaf.value.clone()),
                _ => {}
            }
        }
        if leaf.path.len() == 4 && path_matches(&leaf.path[..2], &["ufs", "rootoverrides"]) {
            let rule =
                cloud_root_overrides
                    .entry(leaf.path[2].clone())
                    .or_insert(CloudRootOverride {
                        rule_id: leaf.path[2].clone(),
                        root: None,
                        os: None,
                        path: None,
                    });
            match leaf.path[3].as_str() {
                "root" => rule.root = Some(leaf.value.clone()),
                "os" => rule.os = Some(leaf.value.clone()),
                "path" => rule.path = Some(leaf.value.clone()),
                _ => {}
            }
        }
    }
    let launches = launches
        .into_values()
        .filter(|launch| !launch.executable.is_empty())
        .collect();
    let executable_references = leaves
        .iter()
        .filter(|leaf| leaf.value.to_ascii_lowercase().ends_with(".exe"))
        .map(|leaf| ExecutableReference {
            metadata_path: leaf.path.join("/"),
            executable: leaf.value.clone(),
        })
        .collect();
    let windows_public_depots = depots
        .into_values()
        .filter(|depot| {
            !depot.manifest_id.is_empty()
                && depot
                    .os_list
                    .as_deref()
                    .is_none_or(|os_list| os_list.split(',').any(|os| os.trim() == "windows"))
        })
        .collect();
    Ok(AppMetadata {
        app_id,
        cache_path: cache_path.to_path_buf(),
        name: value(&["common", "name"]),
        app_os_list: value(&["common", "oslist"]),
        install_dir: value(&["config", "installdir"]),
        public_build_id: value(&["depots", "branches", "public", "buildid"]),
        launches,
        executable_references,
        windows_public_depots,
        cloud_save_rules: cloud_save_rules.into_values().collect(),
        cloud_root_overrides: cloud_root_overrides.into_values().collect(),
    })
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

fn path_matches(path: &[String], wanted: &[&str]) -> bool {
    path.len() == wanted.len()
        && path
            .iter()
            .zip(wanted)
            .all(|(actual, wanted)| actual == wanted)
}

fn read_byte(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u8, Error> {
    let value = bytes
        .get(*cursor)
        .copied()
        .filter(|_| *cursor < end)
        .ok_or_else(|| error("Steam app-info payload is truncated"))?;
    *cursor += 1;
    Ok(value)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let array: [u8; 4] = bytes
        .get(offset..offset + 4)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| error("Steam app-info cache is truncated"))?;
    Ok(u32::from_le_bytes(array))
}

fn read_u32_bounded(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u32, Error> {
    if *cursor + 4 > end {
        return Err(error("Steam app-info payload is truncated"));
    }
    let value = read_u32(bytes, *cursor)?;
    *cursor += 4;
    Ok(value)
}

fn read_u64_bounded(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u64, Error> {
    let array: [u8; 8] = bytes
        .get(*cursor..*cursor + 8)
        .filter(|_| *cursor + 8 <= end)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| error("Steam app-info payload is truncated"))?;
    *cursor += 8;
    Ok(u64::from_le_bytes(array))
}

fn read_cstring(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<String, Error> {
    let remaining = bytes
        .get(*cursor..end)
        .ok_or_else(|| error("Steam app-info string starts outside its container"))?;
    let length = remaining
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| error("Steam app-info string is not terminated"))?;
    let value = std::str::from_utf8(&remaining[..length])
        .map_err(|_| error("Steam app-info string is not UTF-8"))?
        .to_owned();
    *cursor += length + 1;
    Ok(value)
}

fn error(message: impl Into<String>) -> Error {
    Error {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_entry(payload: &mut Vec<u8>, value_type: u8, key: u32) {
        payload.push(value_type);
        payload.extend_from_slice(&key.to_le_bytes());
    }

    #[test]
    fn extracts_launch_build_and_windows_depot_metadata() {
        let strings = [
            "appid",
            "common",
            "name",
            "config",
            "installdir",
            "launch",
            "0",
            "executable",
            "oslist",
            "depots",
            "42",
            "manifests",
            "public",
            "gid",
            "size",
            "branches",
            "buildid",
            "osarch",
            "workingdir",
        ];
        let mut payload = vec![0];
        push_entry(&mut payload, 2, 0);
        payload.extend_from_slice(&7_u32.to_le_bytes());
        push_entry(&mut payload, 0, 1);
        push_entry(&mut payload, 1, 2);
        payload.extend_from_slice(b"Example\0");
        push_entry(&mut payload, 1, 8);
        payload.extend_from_slice(b"windows\0");
        payload.push(8);
        push_entry(&mut payload, 0, 3);
        push_entry(&mut payload, 1, 4);
        payload.extend_from_slice(b"ExampleDir\0");
        push_entry(&mut payload, 0, 5);
        push_entry(&mut payload, 0, 6);
        push_entry(&mut payload, 1, 7);
        payload.extend_from_slice(b"Binaries/Game.exe\0");
        push_entry(&mut payload, 1, 18);
        payload.extend_from_slice(b"Binaries\0");
        push_entry(&mut payload, 0, 3);
        push_entry(&mut payload, 1, 8);
        payload.extend_from_slice(b"windows\0");
        push_entry(&mut payload, 1, 17);
        payload.extend_from_slice(b"32\0");
        payload.push(8);
        payload.push(8);
        payload.push(8);
        payload.push(8);
        push_entry(&mut payload, 0, 9);
        push_entry(&mut payload, 0, 10);
        push_entry(&mut payload, 0, 3);
        push_entry(&mut payload, 1, 8);
        payload.extend_from_slice(b"windows\0");
        payload.push(8);
        push_entry(&mut payload, 0, 11);
        push_entry(&mut payload, 0, 12);
        push_entry(&mut payload, 1, 13);
        payload.extend_from_slice(b"123456\0");
        push_entry(&mut payload, 1, 14);
        payload.extend_from_slice(b"999\0");
        payload.push(8);
        payload.push(8);
        payload.push(8);
        push_entry(&mut payload, 0, 15);
        push_entry(&mut payload, 0, 12);
        push_entry(&mut payload, 1, 16);
        payload.extend_from_slice(b"77\0");
        payload.push(8);
        payload.push(8);
        payload.push(8);
        payload.push(8);
        payload.push(8);
        payload.push(0);

        let record_size = RECORD_METADATA_SIZE - 1 + payload.len();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&APPINFO_MAGIC_V29.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&7_u32.to_le_bytes());
        bytes.extend_from_slice(&(record_size as u32).to_le_bytes());
        bytes.extend_from_slice(&[0; RECORD_METADATA_SIZE - 1]);
        bytes.extend_from_slice(&payload);
        let table_offset = bytes.len();
        bytes[8..12].copy_from_slice(&(table_offset as u32).to_le_bytes());
        bytes.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for string in strings {
            bytes.extend_from_slice(string.as_bytes());
            bytes.push(0);
        }

        let metadata = parse(7, Path::new("appinfo.vdf"), &bytes).unwrap();
        assert_eq!(metadata.name.as_deref(), Some("Example"));
        assert_eq!(metadata.app_os_list.as_deref(), Some("windows"));
        assert_eq!(metadata.install_dir.as_deref(), Some("ExampleDir"));
        assert_eq!(metadata.public_build_id.as_deref(), Some("77"));
        assert_eq!(metadata.launches[0].executable, "Binaries/Game.exe");
        assert_eq!(metadata.launches[0].working_dir.as_deref(), Some("Binaries"));
        assert_eq!(metadata.launches[0].os_arch.as_deref(), Some("32"));
        assert_eq!(
            metadata.executable_references,
            vec![ExecutableReference {
                metadata_path: "config/launch/0/executable".to_owned(),
                executable: "Binaries/Game.exe".to_owned(),
            }]
        );
        assert_eq!(metadata.windows_public_depots[0].depot_id, 42);
        assert_eq!(metadata.windows_public_depots[0].manifest_id, "123456");
        assert_eq!(metadata.windows_public_depots[0].size, Some(999));
    }

    #[test]
    fn rejects_an_unknown_cache_magic() {
        let error = parse(7, Path::new("appinfo.vdf"), &[0; 32]).unwrap_err();
        assert!(error.message.contains("unsupported"));
    }

    #[test]
    fn extracts_ufs_save_rules_and_root_overrides_without_inference() {
        let leaves = [
            Leaf {
                path: vec!["ufs", "savefiles", "0", "root"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                value: "WinMyDocuments".to_owned(),
            },
            Leaf {
                path: vec!["ufs", "savefiles", "0", "path"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                value: "My Games/Example/Saves".to_owned(),
            },
            Leaf {
                path: vec!["ufs", "savefiles", "0", "pattern"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                value: "*.sav".to_owned(),
            },
            Leaf {
                path: vec!["ufs", "savefiles", "0", "recursive"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                value: "1".to_owned(),
            },
            Leaf {
                path: vec!["ufs", "savefiles", "0", "platforms"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                value: "Windows".to_owned(),
            },
            Leaf {
                path: vec!["ufs", "rootoverrides", "0", "root"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                value: "WinMyDocuments".to_owned(),
            },
            Leaf {
                path: vec!["ufs", "rootoverrides", "0", "os"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                value: "MacOS".to_owned(),
            },
            Leaf {
                path: vec!["ufs", "rootoverrides", "0", "path"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                value: "Library/Application Support/Example".to_owned(),
            },
        ];
        let metadata = build_metadata(42, Path::new("appinfo.vdf"), &leaves).unwrap();
        assert_eq!(
            metadata.cloud_save_rules,
            vec![CloudSaveRule {
                rule_id: "0".to_owned(),
                root: Some("WinMyDocuments".to_owned()),
                path: Some("My Games/Example/Saves".to_owned()),
                pattern: Some("*.sav".to_owned()),
                recursive: Some(true),
                platforms: Some("Windows".to_owned()),
            }]
        );
        assert_eq!(
            metadata.cloud_root_overrides,
            vec![CloudRootOverride {
                rule_id: "0".to_owned(),
                root: Some("WinMyDocuments".to_owned()),
                os: Some("MacOS".to_owned()),
                path: Some("Library/Application Support/Example".to_owned()),
            }]
        );
    }

    #[test]
    fn profile_comparison_allows_extra_cached_depots_but_rejects_drift() {
        let metadata = AppMetadata {
            app_id: 42,
            cache_path: PathBuf::from("appinfo.vdf"),
            name: Some("Example".to_owned()),
            app_os_list: Some("windows".to_owned()),
            install_dir: Some("Example".to_owned()),
            public_build_id: Some("7".to_owned()),
            launches: vec![LaunchEntry {
                executable: "Bin/Game.exe".to_owned(),
                working_dir: None,
                description: None,
                os_list: Some("windows".to_owned()),
                os_arch: None,
            }],
            executable_references: vec![ExecutableReference {
                metadata_path: "config/launch/0/executable".to_owned(),
                executable: "Bin/Game.exe".to_owned(),
            }],
            windows_public_depots: vec![
                DepotEntry {
                    depot_id: 43,
                    manifest_id: "99".to_owned(),
                    size: None,
                    language: None,
                    os_list: Some("windows".to_owned()),
                    dlc_app_id: None,
                },
                DepotEntry {
                    depot_id: 44,
                    manifest_id: "100".to_owned(),
                    size: None,
                    language: None,
                    os_list: Some("windows".to_owned()),
                    dlc_app_id: Some(44),
                },
            ],
            cloud_save_rules: Vec::new(),
            cloud_root_overrides: Vec::new(),
        };
        let matching = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:99\nwindows_payload_root=steam-installed-tree\nruntime=x\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Example\nwindows_build_id=7\nexecutable=Bin/Game.exe",
        )
        .unwrap();
        assert!(profile_mismatches(&metadata, &matching).is_empty());

        let check_guid_executable = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:99\nwindows_payload_root=steam-installed-tree\nruntime=x\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Example\nwindows_build_id=7\nexecutable=Bin/CheckedGame.exe",
        )
        .unwrap();
        let mut check_guid_metadata = metadata.clone();
        check_guid_metadata
            .executable_references
            .push(ExecutableReference {
                metadata_path: "config/checkguids".to_owned(),
                executable: "Bin/CheckedGame.exe".to_owned(),
            });
        assert!(profile_mismatches(&check_guid_metadata, &check_guid_executable).is_empty());

        let drifted = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:98\nwindows_payload_root=steam-installed-tree\nruntime=x\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Example\nwindows_build_id=8\nexecutable=Other.exe",
        )
        .unwrap();
        let mismatches = profile_mismatches(&metadata, &drifted);
        assert_eq!(mismatches.len(), 3);
    }
}
