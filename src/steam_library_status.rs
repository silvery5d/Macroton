use crate::keyvalues::{self, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStatus {
    pub app_id: u32,
    pub steam_root: PathBuf,
    pub account_records: usize,
    pub max_playtime_minutes: Option<u64>,
    pub latest_last_played_unix: Option<u64>,
    pub cached_app_tickets: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryInventory {
    pub steam_root: PathBuf,
    pub entries: Vec<LibraryStatus>,
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

pub fn inspect(app_id: u32, data_root: &Path) -> Result<LibraryStatus, Error> {
    let steam_root = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix/drive_c/Program Files (x86)/Steam");
    inspect_steam_root(app_id, &steam_root)
}

pub fn inspect_steam_root(app_id: u32, steam_root: &Path) -> Result<LibraryStatus, Error> {
    let inventory = scan_steam_root(steam_root)?;
    Ok(inventory
        .entries
        .into_iter()
        .find(|status| status.app_id == app_id)
        .unwrap_or(LibraryStatus {
            app_id,
            steam_root: steam_root.to_path_buf(),
            account_records: 0,
            max_playtime_minutes: None,
            latest_last_played_unix: None,
            cached_app_tickets: 0,
        }))
}

pub fn scan_steam_root(steam_root: &Path) -> Result<LibraryInventory, Error> {
    let userdata = steam_root.join("userdata");
    let entries = fs::read_dir(&userdata).map_err(|error| Error {
        message: format!(
            "cannot inspect Steam userdata {}: {error}",
            userdata.display()
        ),
    })?;
    let mut statuses: BTreeMap<u32, LibraryStatus> = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| Error {
            message: format!(
                "cannot inspect Steam userdata {}: {error}",
                userdata.display()
            ),
        })?;
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
            || !entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let localconfig = entry.path().join("config/localconfig.vdf");
        let contents = match fs::read_to_string(&localconfig) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Error {
                    message: format!("cannot read Steam library metadata: {error}"),
                });
            }
        };
        let root = keyvalues::parse(&contents).map_err(|error| Error {
            message: format!("cannot parse Steam library metadata: {error}"),
        })?;
        let Some(store) = root.get("UserLocalConfigStore") else {
            continue;
        };
        if let Some(apps) =
            nested(store, &["Software", "Valve", "Steam", "apps"]).and_then(Value::entries)
        {
            for (app_id, app) in apps {
                let Ok(app_id) = app_id.parse::<u32>() else {
                    continue;
                };
                let status = status_entry(&mut statuses, app_id, steam_root);
                status.account_records += 1;
                status.max_playtime_minutes = max_optional(
                    status.max_playtime_minutes,
                    parse_u64(app.string("Playtime")),
                );
                status.latest_last_played_unix = max_optional(
                    status.latest_last_played_unix,
                    parse_u64(app.string("LastPlayed")),
                );
            }
        }
        if let Some(tickets) = nested(store, &["apptickets"]).and_then(Value::entries) {
            for (app_id, _) in tickets {
                let Ok(app_id) = app_id.parse::<u32>() else {
                    continue;
                };
                status_entry(&mut statuses, app_id, steam_root).cached_app_tickets += 1;
            }
        }
    }
    Ok(LibraryInventory {
        steam_root: steam_root.to_path_buf(),
        entries: statuses.into_values().collect(),
    })
}

fn status_entry<'a>(
    statuses: &'a mut BTreeMap<u32, LibraryStatus>,
    app_id: u32,
    steam_root: &Path,
) -> &'a mut LibraryStatus {
    statuses.entry(app_id).or_insert_with(|| LibraryStatus {
        app_id,
        steam_root: steam_root.to_path_buf(),
        account_records: 0,
        max_playtime_minutes: None,
        latest_last_played_unix: None,
        cached_app_tickets: 0,
    })
}

fn nested<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(key))
}

fn parse_u64(value: Option<&str>) -> Option<u64> {
    value.and_then(|value| value.parse().ok())
}

fn max_optional(first: Option<u64>, second: Option<u64>) -> Option<u64> {
    match (first, second) {
        (Some(first), Some(second)) => Some(first.max(second)),
        (first, second) => first.or(second),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(label: &str) -> PathBuf {
        let sequence = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("macroton-{label}-{sequence}"))
    }

    #[test]
    fn summarizes_history_and_ticket_without_exposing_account_paths() {
        let root = temporary_root("library-status");
        let steam = root.join("Steam");
        let config = steam.join("userdata/123/config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("localconfig.vdf"),
            r#"
            "UserLocalConfigStore" {
              "apptickets" { "42" "sensitive-ticket" }
              "Software" { "Valve" { "Steam" { "apps" {
                "42" { "LastPlayed" "100" "Playtime" "200" }
              } } } }
            }
            "#,
        )
        .unwrap();

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.account_records, 1);
        assert_eq!(status.max_playtime_minutes, Some(200));
        assert_eq!(status.latest_last_played_unix, Some(100));
        assert_eq!(status.cached_app_tickets, 1);
        assert!(!format!("{status:?}").contains("sensitive-ticket"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn distinguishes_absent_app_history() {
        let root = temporary_root("library-status-absent");
        let steam = root.join("Steam");
        fs::create_dir_all(steam.join("userdata")).unwrap();
        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.account_records, 0);
        assert_eq!(status.cached_app_tickets, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inventory_aggregates_multiple_accounts_without_identifiers() {
        let root = temporary_root("library-inventory");
        let steam = root.join("Steam");
        for (account, playtime, last_played) in [("123", 200, 100), ("456", 300, 90)] {
            let config = steam.join(format!("userdata/{account}/config"));
            fs::create_dir_all(&config).unwrap();
            fs::write(
                config.join("localconfig.vdf"),
                format!(
                    "\"UserLocalConfigStore\" {{ \"Software\" {{ \"Valve\" {{ \"Steam\" {{ \"apps\" {{ \"42\" {{ \"Playtime\" \"{playtime}\" \"LastPlayed\" \"{last_played}\" }} }} }} }} }} }}"
                ),
            )
            .unwrap();
        }
        let inventory = scan_steam_root(&steam).unwrap();
        assert_eq!(inventory.entries.len(), 1);
        assert_eq!(inventory.entries[0].account_records, 2);
        assert_eq!(inventory.entries[0].max_playtime_minutes, Some(300));
        assert_eq!(inventory.entries[0].latest_last_played_unix, Some(100));
        assert!(!format!("{inventory:?}").contains("userdata/123"));
        fs::remove_dir_all(root).unwrap();
    }
}
