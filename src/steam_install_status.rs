use crate::keyvalues;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallState {
    NotRequested,
    AwaitingConfirmation,
    AwaitingEula,
    EulaPromptUnconfirmed,
    Queued,
    Downloading,
    Stalled,
    ManifestOnly,
    Installed,
    Failed,
}

impl InstallState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRequested => "not-requested",
            Self::AwaitingConfirmation => "awaiting-confirmation",
            Self::AwaitingEula => "awaiting-eula",
            Self::EulaPromptUnconfirmed => "eula-prompt-unconfirmed",
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Stalled => "stalled",
            Self::ManifestOnly => "manifest-only",
            Self::Installed => "installed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstallStatus {
    pub app_id: u32,
    pub steam_root: PathBuf,
    pub manifest_path: PathBuf,
    pub content_log_path: PathBuf,
    pub state: InstallState,
    pub name: Option<String>,
    pub installed_build_id: Option<u64>,
    pub target_build_id: Option<u64>,
    pub manifest_size_on_disk: Option<u64>,
    pub installed_payload_directory_present: bool,
    pub installed_payload_directory_is_symlink: bool,
    pub installed_payload_entry_observed: bool,
    pub bytes_to_download: Option<u64>,
    pub manifest_bytes_downloaded: Option<u64>,
    pub bytes_to_stage: Option<u64>,
    pub manifest_bytes_staged: Option<u64>,
    pub attempt_start_downloaded_bytes: Option<u64>,
    pub attempt_download_total_bytes: Option<u64>,
    pub attempt_start_staged_bytes: Option<u64>,
    pub attempt_stage_total_bytes: Option<u64>,
    pub observed_network_bytes: Option<u64>,
    pub latest_rate_mbps: Option<f64>,
    pub selected_depots: Vec<u32>,
    pub installed_depots: Vec<u32>,
    pub recent_connection_failures: usize,
    pub latest_observed_at: Option<String>,
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

#[derive(Default)]
struct LogEvidence {
    install_uri_seen: bool,
    update_started: bool,
    selected_depots: Vec<u32>,
    attempt_start_downloaded_bytes: Option<u64>,
    attempt_download_total_bytes: Option<u64>,
    attempt_start_staged_bytes: Option<u64>,
    attempt_stage_total_bytes: Option<u64>,
    observed_network_bytes: Option<u64>,
    latest_rate_mbps: Option<f64>,
    recent_connection_failures: usize,
    latest_observed_at: Option<String>,
    explicit_failure: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EulaPromptEvidence {
    None,
    Recent,
    Historical,
}

pub fn inspect(app_id: u32, data_root: &Path) -> Result<InstallStatus, Error> {
    let steam_root = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix/drive_c/Program Files (x86)/Steam");
    inspect_steam_root(app_id, &steam_root)
}

pub fn inspect_steam_root(app_id: u32, steam_root: &Path) -> Result<InstallStatus, Error> {
    if !steam_root.is_dir() {
        return Err(Error {
            message: format!(
                "Windows Steam root for AppID {app_id} does not exist: {}",
                steam_root.display()
            ),
        });
    }
    let manifest_path = steam_root
        .join("steamapps")
        .join(format!("appmanifest_{app_id}.acf"));
    let content_log_path = steam_root.join("logs/content_log.txt");
    let log = read_log_evidence(app_id, &content_log_path)?;
    let eula_prompt = eula_prompt_evidence(app_id, &steam_root.join("logs/webhelper_js.txt"))?;
    let content_log_is_fresh = fs::metadata(&content_log_path)
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
        .is_ok_and(|age| age <= Duration::from_secs(10 * 60));

    let Some(manifest) = read_manifest(&manifest_path)? else {
        return Ok(InstallStatus {
            app_id,
            steam_root: steam_root.to_path_buf(),
            manifest_path,
            content_log_path,
            // Steam may begin downloading before it atomically creates the
            // AppState manifest.  Preserve the content-log evidence here so
            // callers do not misclassify an in-flight install as
            // `not-requested` and stop/restart it unnecessarily.
            state: state_without_manifest(&log, content_log_is_fresh, eula_prompt),
            name: None,
            installed_build_id: None,
            target_build_id: None,
            manifest_size_on_disk: None,
            installed_payload_directory_present: false,
            installed_payload_directory_is_symlink: false,
            installed_payload_entry_observed: false,
            bytes_to_download: None,
            manifest_bytes_downloaded: None,
            bytes_to_stage: None,
            manifest_bytes_staged: None,
            attempt_start_downloaded_bytes: log.attempt_start_downloaded_bytes,
            attempt_download_total_bytes: log.attempt_download_total_bytes,
            attempt_start_staged_bytes: log.attempt_start_staged_bytes,
            attempt_stage_total_bytes: log.attempt_stage_total_bytes,
            observed_network_bytes: log.observed_network_bytes,
            latest_rate_mbps: log.latest_rate_mbps,
            selected_depots: log.selected_depots,
            installed_depots: Vec::new(),
            recent_connection_failures: log.recent_connection_failures,
            latest_observed_at: log.latest_observed_at,
        });
    };

    let app_state = manifest.get("AppState").ok_or_else(|| Error {
        message: format!(
            "Steam app manifest has no AppState object: {}",
            manifest_path.display()
        ),
    })?;
    let installed_build_id = parse_optional_u64(app_state.string("buildid"));
    // Older Steam manifests use a literal zero when there is no pending target.
    // Treating that sentinel as a real build makes a fully installed legacy game
    // look queued even when its byte checkpoints and installed depots are complete.
    let target_build_id =
        parse_optional_u64(app_state.string("TargetBuildID")).filter(|target| *target != 0);
    let manifest_size_on_disk = parse_optional_u64(app_state.string("SizeOnDisk"));
    let installed_payload_directory = app_state
        .string("installdir")
        .filter(|install_dir| safe_install_dir_name(install_dir))
        .map(|install_dir| steam_root.join("steamapps/common").join(install_dir));
    let installed_payload_directory_present = installed_payload_directory
        .as_deref()
        .is_some_and(Path::is_dir);
    let installed_payload_directory_is_symlink = installed_payload_directory
        .as_deref()
        .and_then(|path| fs::symlink_metadata(path).ok())
        .is_some_and(|metadata| metadata.file_type().is_symlink());
    let installed_payload_directory_is_macroton_view = installed_payload_directory
        .as_deref()
        .and_then(|path| fs::read_link(path).ok())
        .is_some_and(|target| {
            let marker = format!("games/{app_id}/payload-views");
            target.to_string_lossy().contains(&marker)
        });
    let installed_payload_entry_observed = installed_payload_directory
        .as_deref()
        .and_then(|path| fs::read_dir(path).ok())
        .is_some_and(|mut entries| entries.next().is_some());
    let bytes_to_download = parse_optional_u64(app_state.string("BytesToDownload"));
    let manifest_bytes_downloaded = parse_optional_u64(app_state.string("BytesDownloaded"));
    let bytes_to_stage = parse_optional_u64(app_state.string("BytesToStage"));
    let manifest_bytes_staged = parse_optional_u64(app_state.string("BytesStaged"));
    let installed_depots = numeric_object_keys(app_state.get("InstalledDepots"));
    let update_result = parse_optional_u64(app_state.string("UpdateResult")).unwrap_or(0);
    let manifest_claims_complete = installed_build_id.is_some_and(|build| build != 0)
        && target_build_id.is_none_or(|target| target == installed_build_id.unwrap_or(0))
        && bytes_to_download
            .zip(manifest_bytes_downloaded)
            .is_none_or(|(total, downloaded)| downloaded >= total);
    let is_installed = manifest_claims_complete
        && manifest_size_on_disk.is_some_and(|size| size > 0)
        && installed_payload_directory_present
        && (!installed_payload_directory_is_symlink
            || installed_payload_directory_is_macroton_view)
        && installed_payload_entry_observed;
    let state = if is_installed {
        InstallState::Installed
    } else if update_result != 0 || log.explicit_failure {
        InstallState::Failed
    } else if log.recent_connection_failures >= 3
        && log.latest_rate_mbps.is_some_and(|rate| rate <= 0.000_5)
    {
        InstallState::Stalled
    } else if log.update_started && log.latest_rate_mbps.is_some_and(|rate| rate > 0.000_5) {
        InstallState::Downloading
    } else if manifest_claims_complete {
        InstallState::ManifestOnly
    } else {
        InstallState::Queued
    };

    Ok(InstallStatus {
        app_id,
        steam_root: steam_root.to_path_buf(),
        manifest_path,
        content_log_path,
        state,
        name: app_state.string("name").map(str::to_owned),
        installed_build_id,
        target_build_id,
        manifest_size_on_disk,
        installed_payload_directory_present,
        installed_payload_directory_is_symlink,
        installed_payload_entry_observed,
        bytes_to_download,
        manifest_bytes_downloaded,
        bytes_to_stage,
        manifest_bytes_staged,
        attempt_start_downloaded_bytes: log.attempt_start_downloaded_bytes,
        attempt_download_total_bytes: log.attempt_download_total_bytes,
        attempt_start_staged_bytes: log.attempt_start_staged_bytes,
        attempt_stage_total_bytes: log.attempt_stage_total_bytes,
        observed_network_bytes: log.observed_network_bytes,
        latest_rate_mbps: log.latest_rate_mbps,
        selected_depots: log.selected_depots,
        installed_depots,
        recent_connection_failures: log.recent_connection_failures,
        latest_observed_at: log.latest_observed_at,
    })
}

fn state_without_manifest(
    log: &LogEvidence,
    content_log_is_fresh: bool,
    eula_prompt: EulaPromptEvidence,
) -> InstallState {
    if content_log_is_fresh && log.recent_connection_failures >= 3
        && log.latest_rate_mbps.is_some_and(|rate| rate <= 0.000_5)
    {
        InstallState::Stalled
    } else if content_log_is_fresh
        && log.update_started
        && log.latest_rate_mbps.is_some_and(|rate| rate > 0.000_5)
    {
        InstallState::Downloading
    } else if content_log_is_fresh && log.update_started {
        InstallState::Queued
    } else if eula_prompt == EulaPromptEvidence::Recent {
        InstallState::AwaitingEula
    } else if content_log_is_fresh && log.install_uri_seen {
        InstallState::AwaitingConfirmation
    } else if eula_prompt == EulaPromptEvidence::Historical {
        InstallState::EulaPromptUnconfirmed
    } else {
        InstallState::NotRequested
    }
}

fn eula_prompt_evidence(app_id: u32, path: &Path) -> Result<EulaPromptEvidence, Error> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => return Ok(EulaPromptEvidence::None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(EulaPromptEvidence::None),
        Err(error) => {
            return Err(Error {
                message: format!("cannot inspect Steam UI log {}: {error}", path.display()),
            });
        }
    };
    let recent = metadata
        .modified()
        .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
        .is_ok_and(|age| age <= Duration::from_secs(10 * 60));
    let contents = fs::read_to_string(path).map_err(|error| Error {
        message: format!("cannot read Steam UI log {}: {error}", path.display()),
    })?;
    // Only the terminal UI event is usable here: an old EULA prompt in this
    // append-only log must not reappear as a current prompt after a later UI
    // action or client restart.
    Ok(classify_eula_prompt(app_id, &contents, recent))
}

fn classify_eula_prompt(app_id: u32, contents: &str, recent: bool) -> EulaPromptEvidence {
    let terminal_prompt = contents
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| line.contains(&format!("prompt for eula {app_id}_eula_")));
    match (terminal_prompt, recent) {
        (true, true) => EulaPromptEvidence::Recent,
        (true, false) => EulaPromptEvidence::Historical,
        (false, _) => EulaPromptEvidence::None,
    }
}

fn safe_install_dir_name(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}

fn read_manifest(path: &Path) -> Result<Option<keyvalues::Value>, Error> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Error {
                message: format!("cannot read Steam app manifest {}: {error}", path.display()),
            });
        }
    };
    keyvalues::parse(&contents)
        .map(Some)
        .map_err(|error| Error {
            message: format!(
                "cannot parse Steam app manifest {}: {error}",
                path.display()
            ),
        })
}

fn read_log_evidence(app_id: u32, path: &Path) -> Result<LogEvidence, Error> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => {
            return Err(Error {
                message: format!("cannot read Steam content log {}: {error}", path.display()),
            });
        }
    };
    Ok(parse_log_evidence(app_id, &contents))
}

fn parse_log_evidence(app_id: u32, contents: &str) -> LogEvidence {
    let app_marker = format!("AppID {app_id}");
    let mut evidence = LogEvidence::default();
    let mut active_attempt = false;
    for line in contents.lines() {
        if line.contains(&format!("ExecuteSteamURL: \"steam://install/{app_id}\"")) {
            evidence.install_uri_seen = true;
        }
        if line.contains(&app_marker)
            && line.contains("user config changed : added depots ")
            && let Some(list) = line.split("added depots ").nth(1)
        {
            evidence.selected_depots = parse_depot_list(list);
        }
        if line.contains(&app_marker) && line.contains(" update started :") {
            active_attempt = true;
            evidence.update_started = true;
            let (downloaded, download_total) = parse_update_progress(line, "download");
            let (staged, stage_total) = parse_update_progress(line, "stage");
            evidence.attempt_start_downloaded_bytes = downloaded;
            evidence.attempt_download_total_bytes = download_total;
            evidence.attempt_start_staged_bytes = staged;
            evidence.attempt_stage_total_bytes = stage_total;
            evidence.observed_network_bytes = None;
            evidence.latest_rate_mbps = None;
            evidence.recent_connection_failures = 0;
            evidence.explicit_failure = false;
            evidence.latest_observed_at = timestamp(line);
            continue;
        }
        if !active_attempt {
            continue;
        }
        if let Some(bytes) = parse_invalid_stats_bytes(line) {
            evidence.observed_network_bytes = Some(bytes);
            evidence.latest_observed_at = timestamp(line);
        }
        if let Some(rate) = parse_download_rate(line) {
            evidence.latest_rate_mbps = Some(rate);
            evidence.latest_observed_at = timestamp(line);
        }
        if line.contains("Failed to get list of download sources")
            || line.contains("ERROR: Failed to get CDN auth token from Steam")
            || line.contains("Result No Connection")
        {
            evidence.recent_connection_failures += 1;
            evidence.latest_observed_at = timestamp(line);
        }
        if line.contains(&app_marker)
            && (line.contains("result ") && !line.contains("result No Error")
                || line.contains("update failed"))
        {
            evidence.explicit_failure = true;
            evidence.latest_observed_at = timestamp(line);
        }
        if line.contains(&app_marker)
            && line.contains("scheduler finished")
            && line.contains("result No Error")
        {
            active_attempt = false;
        }
    }
    evidence
}

fn parse_update_progress(line: &str, label: &str) -> (Option<u64>, Option<u64>) {
    let marker = format!("{label} ");
    let Some(value) = line.split(&marker).nth(1) else {
        return (None, None);
    };
    let Some(progress) = value.split([',', ' ']).next() else {
        return (None, None);
    };
    let mut parts = progress.split('/');
    let current = parts.next().and_then(|value| value.parse().ok());
    let total = parts.next().and_then(|value| value.parse().ok());
    (current, total)
}

fn parse_depot_list(value: &str) -> Vec<u32> {
    let mut depots = value
        .split(',')
        .filter_map(|value| value.trim().parse().ok())
        .collect::<Vec<_>>();
    depots.sort_unstable();
    depots.dedup();
    depots
}

fn parse_invalid_stats_bytes(line: &str) -> Option<u64> {
    let value = line.split("stats: (Invalid, 0) : ").nth(1)?;
    value.split(" Bytes").next()?.trim().parse().ok()
}

fn parse_download_rate(line: &str) -> Option<f64> {
    let value = line.split("Current download rate: ").nth(1)?;
    value.split(" Mbps").next()?.trim().parse().ok()
}

fn timestamp(line: &str) -> Option<String> {
    line.strip_prefix('[')
        .and_then(|line| line.split(']').next())
        .map(str::to_owned)
}

fn parse_optional_u64(value: Option<&str>) -> Option<u64> {
    value.and_then(|value| value.parse().ok())
}

fn numeric_object_keys(value: Option<&keyvalues::Value>) -> Vec<u32> {
    let mut keys = value
        .and_then(keyvalues::Value::entries)
        .unwrap_or_default()
        .iter()
        .filter_map(|(key, _)| key.parse().ok())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_steam(label: &str) -> PathBuf {
        let sequence = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let steam = std::env::temp_dir().join(format!(
            "macroton-install-status-{label}-{}-{sequence}/Steam",
            std::process::id()
        ));
        fs::create_dir_all(steam.join("steamapps")).unwrap();
        fs::create_dir_all(steam.join("logs")).unwrap();
        steam
    }

    fn write_manifest(steam: &Path, app_id: u32, body: &str) {
        fs::write(
            steam
                .join("steamapps")
                .join(format!("appmanifest_{app_id}.acf")),
            body,
        )
        .unwrap();
    }

    #[test]
    fn reports_a_stalled_download_without_exposing_log_tokens() {
        let steam = temporary_steam("stalled");
        write_manifest(
            &steam,
            42,
            r#""AppState" { "appid" "42" "name" "Fixture" "buildid" "0" "TargetBuildID" "7" "BytesToDownload" "1000" "BytesDownloaded" "0" "BytesToStage" "1200" "BytesStaged" "0" "UpdateResult" "0" "InstalledDepots" { } }"#,
        );
        fs::write(
            steam.join("logs/content_log.txt"),
            "[2026-01-01 10:00:00] AppID 42 user config changed : added depots 9,8\n[2026-01-01 10:00:01] AppID 42 update started : download 0/1000\n[2026-01-01 10:00:02] stats: (Invalid, 0) : 400 Bytes\n[2026-01-01 10:00:03] Current download rate: 0.000 Mbps\n[2026-01-01 10:00:04] Failed to get list of download sources\n[2026-01-01 10:00:05] Result No Connection\n[2026-01-01 10:00:06] ERROR: Failed to get CDN auth token from Steam\n",
        )
        .unwrap();

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, InstallState::Stalled);
        assert_eq!(status.selected_depots, vec![8, 9]);
        assert_eq!(status.attempt_start_downloaded_bytes, Some(0));
        assert_eq!(status.attempt_download_total_bytes, Some(1000));
        assert_eq!(status.observed_network_bytes, Some(400));
        assert_eq!(status.recent_connection_failures, 3);
        fs::remove_dir_all(steam.parent().unwrap()).unwrap();
    }

    #[test]
    fn reports_a_complete_build_and_installed_depots() {
        let steam = temporary_steam("installed");
        fs::create_dir_all(steam.join("steamapps/common/Fixture")).unwrap();
        fs::write(steam.join("steamapps/common/Fixture/game.exe"), b"fixture").unwrap();
        write_manifest(
            &steam,
            42,
            r#""AppState" { "appid" "42" "name" "Fixture" "installdir" "Fixture" "SizeOnDisk" "2200" "buildid" "7" "TargetBuildID" "7" "BytesToDownload" "1000" "BytesDownloaded" "1000" "BytesToStage" "1200" "BytesStaged" "1200" "UpdateResult" "0" "InstalledDepots" { "8" { "manifest" "88" } "9" { "manifest" "99" } } }"#,
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, InstallState::Installed);
        assert_eq!(status.manifest_size_on_disk, Some(2200));
        assert!(status.installed_payload_directory_present);
        assert!(!status.installed_payload_directory_is_symlink);
        assert!(status.installed_payload_entry_observed);
        assert_eq!(status.installed_depots, vec![8, 9]);
        fs::remove_dir_all(steam.parent().unwrap()).unwrap();
    }

    #[test]
    fn zero_target_build_is_no_pending_update_for_legacy_manifest() {
        let steam = temporary_steam("legacy-zero-target");
        fs::create_dir_all(steam.join("steamapps/common/Legacy Fixture")).unwrap();
        fs::write(
            steam.join("steamapps/common/Legacy Fixture/game.exe"),
            b"fixture",
        )
        .unwrap();
        write_manifest(
            &steam,
            42,
            r#""AppState" { "appid" "42" "name" "Legacy Fixture" "installdir" "Legacy Fixture" "SizeOnDisk" "2200" "StateFlags" "4" "buildid" "7" "TargetBuildID" "0" "BytesToDownload" "1000" "BytesDownloaded" "1000" "BytesToStage" "1200" "BytesStaged" "1200" "UpdateResult" "0" "InstalledDepots" { "8" { "manifest" "88" } } }"#,
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, InstallState::Installed);
        assert_eq!(status.target_build_id, None);
        assert_eq!(status.installed_depots, vec![8]);
        fs::remove_dir_all(steam.parent().unwrap()).unwrap();
    }

    #[test]
    fn complete_manifest_without_payload_is_manifest_only() {
        let steam = temporary_steam("manifest-only");
        write_manifest(
            &steam,
            42,
            r#""AppState" { "appid" "42" "name" "Fixture" "installdir" "Fixture" "SizeOnDisk" "0" "buildid" "7" "TargetBuildID" "0" "BytesToDownload" "0" "BytesDownloaded" "0" "UpdateResult" "0" "InstalledDepots" { "8" { "manifest" "88" } } }"#,
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, InstallState::ManifestOnly);
        assert_eq!(status.manifest_size_on_disk, Some(0));
        assert!(!status.installed_payload_directory_present);
        assert!(!status.installed_payload_directory_is_symlink);
        assert!(!status.installed_payload_entry_observed);
        assert_eq!(status.installed_depots, vec![8]);
        fs::remove_dir_all(steam.parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn no_copy_payload_symlink_is_never_a_committed_install() {
        use std::os::unix::fs::symlink;

        let steam = temporary_steam("no-copy-symlink");
        let external_payload = steam.parent().unwrap().join("owned-depot");
        fs::create_dir_all(&external_payload).unwrap();
        fs::write(external_payload.join("game.exe"), b"fixture").unwrap();
        fs::create_dir_all(steam.join("steamapps/common")).unwrap();
        symlink(&external_payload, steam.join("steamapps/common/Fixture")).unwrap();
        write_manifest(
            &steam,
            42,
            r#""AppState" { "appid" "42" "name" "Fixture" "installdir" "Fixture" "SizeOnDisk" "2200" "buildid" "7" "TargetBuildID" "0" "UpdateResult" "0" "InstalledDepots" { "8" { "manifest" "88" } } }"#,
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, InstallState::ManifestOnly);
        assert!(status.installed_payload_directory_present);
        assert!(status.installed_payload_directory_is_symlink);
        assert!(status.installed_payload_entry_observed);
        fs::remove_dir_all(steam.parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn macroton_payload_view_symlink_counts_as_installed() {
        use std::os::unix::fs::symlink;

        let steam = temporary_steam("macroton-payload-view");
        let root = steam.parent().unwrap();
        let private_view = root.join("games/42/payload-views/profile");
        fs::create_dir_all(&private_view).unwrap();
        fs::write(private_view.join("game.exe"), b"fixture").unwrap();
        fs::create_dir_all(steam.join("steamapps/common")).unwrap();
        symlink(&private_view, steam.join("steamapps/common/Fixture")).unwrap();
        write_manifest(
            &steam,
            42,
            r#""AppState" { "appid" "42" "name" "Fixture" "installdir" "Fixture" "SizeOnDisk" "2200" "buildid" "7" "TargetBuildID" "7" "BytesToDownload" "1000" "BytesDownloaded" "1000" "UpdateResult" "0" "InstalledDepots" { "8" { "manifest" "88" } } }"#,
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, InstallState::Installed);
        assert!(status.installed_payload_directory_is_symlink);
        assert!(status.installed_payload_entry_observed);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_manifest_is_not_a_requested_install() {
        let steam = temporary_steam("missing");
        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, InstallState::NotRequested);
        fs::remove_dir_all(steam.parent().unwrap()).unwrap();
    }

    #[test]
    fn recent_terminal_eula_prompt_is_not_misreported_as_no_request() {
        let steam = temporary_steam("eula-prompt");
        let log = steam.join("logs/webhelper_js.txt");
        fs::write(&log, "[time] SteamUI: INFO: prompt for eula 42_eula_0\n").unwrap();
        assert_eq!(
            inspect_steam_root(42, &steam).unwrap().state,
            InstallState::AwaitingEula
        );
        assert_eq!(
            inspect_steam_root(43, &steam).unwrap().state,
            InstallState::NotRequested
        );
        fs::write(
            &log,
            "[time] SteamUI: INFO: prompt for eula 42_eula_0\n[time] SteamUI: INFO: later UI action\n",
        )
        .unwrap();
        assert_eq!(
            inspect_steam_root(42, &steam).unwrap().state,
            InstallState::NotRequested
        );
        fs::remove_dir_all(steam.parent().unwrap()).unwrap();
    }

    #[test]
    fn historical_terminal_eula_prompt_is_explicitly_unconfirmed() {
        let prompt = "[time] SteamUI: INFO: prompt for eula 42_eula_0\n";
        assert_eq!(
            classify_eula_prompt(42, prompt, false),
            EulaPromptEvidence::Historical
        );
        assert_eq!(
            state_without_manifest(&LogEvidence::default(), false, EulaPromptEvidence::Historical),
            InstallState::EulaPromptUnconfirmed
        );
        assert_eq!(
            classify_eula_prompt(42, &format!("{prompt}[time] SteamUI: INFO: later UI action\n"), false),
            EulaPromptEvidence::None
        );
    }

    #[test]
    fn missing_manifest_with_content_log_is_in_flight_download() {
        let steam = temporary_steam("missing-in-flight");
        fs::write(
            steam.join("logs/content_log.txt"),
            "[2026-01-01 10:00:00] AppID 42 update started : download 12/100\n[2026-01-01 10:00:01] Current download rate: 4.000 Mbps\n",
        )
        .unwrap();

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, InstallState::Downloading);
        assert_eq!(status.attempt_start_downloaded_bytes, Some(12));
        assert_eq!(status.attempt_download_total_bytes, Some(100));
        fs::remove_dir_all(steam.parent().unwrap()).unwrap();
    }

    #[test]
    fn stale_log_evidence_is_not_an_active_download() {
        let log = LogEvidence {
            update_started: true,
            latest_rate_mbps: Some(4.0),
            ..LogEvidence::default()
        };
        assert_eq!(
            state_without_manifest(&log, false, EulaPromptEvidence::None),
            InstallState::NotRequested
        );
    }

    #[test]
    fn install_uri_without_update_is_awaiting_confirmation() {
        let log = parse_log_evidence(
            42,
            "[2026-09-13 03:00:00] ExecuteSteamURL: \"steam://install/42\"\n",
        );
        assert!(log.install_uri_seen);
        assert_eq!(
            state_without_manifest(&log, true, EulaPromptEvidence::None),
            InstallState::AwaitingConfirmation
        );
    }
}
