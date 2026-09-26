use crate::cloud_status::{self, ComparisonState, SyncState, TransferEvent};
use crate::profile::{CloudLocalRoot, CompatibilityProfile};
use crate::stop;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CURRENT_MARKER: &str = "cloud-roundtrip.current";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Baseline,
    Uploaded,
    Quarantined,
    Complete,
    Restored,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Uploaded => "uploaded",
            Self::Quarantined => "quarantined",
            Self::Complete => "complete",
            Self::Restored => "restored",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "baseline" => Some(Self::Baseline),
            "uploaded" => Some(Self::Uploaded),
            "quarantined" => Some(Self::Quarantined),
            "complete" => Some(Self::Complete),
            "restored" => Some(Self::Restored),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub app_id: u32,
    pub profile_sha256: String,
    pub transaction_id: u128,
    pub phase: Phase,
    pub cloud_log_offset: u64,
    pub account_id: String,
    pub baseline_sha1: Vec<String>,
    pub uploaded_sha1: Vec<String>,
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

pub fn begin(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
) -> Result<Transaction, Error> {
    validate_profile(app_id, profile)?;
    let game_root = game_root(app_id, data_root);
    if let Ok(current) = read_transaction(&game_root) {
        if !matches!(current.phase, Phase::Complete | Phase::Restored) {
            return Err(Error {
                message: format!(
                    "Cloud roundtrip transaction {} is still {}; finish or restore it first",
                    current.transaction_id,
                    current.phase.as_str()
                ),
            });
        }
    } else if game_root.join(CURRENT_MARKER).exists() {
        return Err(Error {
            message: "existing Cloud roundtrip marker is invalid; refusing to replace it"
                .to_owned(),
        });
    }
    require_idle(app_id, data_root)?;
    let (status, hashes) = synced_hashes(app_id, profile, data_root)?;
    let transaction_id = unix_nanos()?;
    let transaction = Transaction {
        app_id,
        profile_sha256: profile.fingerprint(),
        transaction_id,
        phase: Phase::Baseline,
        cloud_log_offset: cloud_status::log_offset(&status.log_path).map_err(convert_cloud)?,
        account_id: status.accounts[0].account_id.clone(),
        baseline_sha1: hashes,
        uploaded_sha1: Vec::new(),
    };
    persist(&game_root, &transaction)?;
    Ok(transaction)
}

pub fn status(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
) -> Result<Transaction, Error> {
    validate_profile(app_id, profile)?;
    let transaction = read_transaction(&game_root(app_id, data_root))?;
    if transaction.app_id != app_id
        || transaction.profile_sha256 != profile.fingerprint()
        || transaction.account_id.is_empty()
        || !transaction
            .account_id
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || transaction.baseline_sha1.len() != profile.cloud_files.len()
        || (!transaction.uploaded_sha1.is_empty()
            && transaction.uploaded_sha1.len() != profile.cloud_files.len())
    {
        return Err(Error {
            message: format!(
                "Cloud roundtrip transaction {} (phase {}) does not match the selected AppID/profile; finish or restore that transaction, then run `cloud-roundtrip begin` for the current profile",
                transaction.transaction_id,
                transaction.phase.as_str()
            ),
        });
    }
    Ok(transaction)
}

pub fn confirm_upload(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
) -> Result<Transaction, Error> {
    validate_profile(app_id, profile)?;
    require_idle(app_id, data_root)?;
    let game_root = game_root(app_id, data_root);
    let mut transaction = read_transaction(&game_root)?;
    validate_transaction(&transaction, app_id, profile, Phase::Baseline)?;
    let (status, hashes) = synced_hashes(app_id, profile, data_root)?;
    if hashes == transaction.baseline_sha1 {
        return Err(Error {
            message: "declared save content did not change after the baseline".to_owned(),
        });
    }
    if !cloud_status::has_transfer_event_since(
        &status.log_path,
        transaction.cloud_log_offset,
        app_id,
        TransferEvent::UploadComplete,
    )
    .map_err(convert_cloud)?
    {
        return Err(Error {
            message: "no successful Steam Cloud upload occurred after the baseline".to_owned(),
        });
    }
    transaction.phase = Phase::Uploaded;
    transaction.cloud_log_offset =
        cloud_status::log_offset(&status.log_path).map_err(convert_cloud)?;
    transaction.uploaded_sha1 = hashes;
    persist(&game_root, &transaction)?;
    Ok(transaction)
}

pub fn quarantine(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
) -> Result<Transaction, Error> {
    validate_profile(app_id, profile)?;
    require_idle(app_id, data_root)?;
    let game_root = game_root(app_id, data_root);
    let mut transaction = read_transaction(&game_root)?;
    validate_transaction(&transaction, app_id, profile, Phase::Uploaded)?;
    let (status, hashes) = synced_hashes(app_id, profile, data_root)?;
    if hashes != transaction.uploaded_sha1 {
        return Err(Error {
            message: "local save content drifted after the confirmed upload".to_owned(),
        });
    }
    let comparisons = unique_comparisons(&status, profile.cloud_files.len())?;
    if status.accounts[0].account_id != transaction.account_id {
        return Err(Error {
            message: "Steam account cache changed after the confirmed upload".to_owned(),
        });
    }
    let backup = transaction_root(&game_root, transaction.transaction_id).join("quarantine");
    fs::create_dir_all(&backup).map_err(|error| Error {
        message: format!(
            "cannot create Cloud quarantine {}: {error}",
            backup.display()
        ),
    })?;
    let destinations = (0..comparisons.len())
        .map(|index| backup.join(format!("{index:04}.save")))
        .collect::<Vec<_>>();
    if destinations.iter().any(|path| path.exists()) {
        return Err(Error {
            message: "Cloud quarantine already contains save data; refusing to overwrite"
                .to_owned(),
        });
    }
    let mut moved = Vec::new();
    for ((comparison, destination), spec) in comparisons
        .iter()
        .zip(&destinations)
        .zip(&profile.cloud_files)
    {
        if spec.local_root == CloudLocalRoot::SteamRemote {
            continue;
        }
        if let Err(error) = fs::rename(&comparison.local_path, destination) {
            for (source, destination) in moved.iter().rev() {
                let _ = fs::rename(destination, source);
            }
            return Err(Error {
                message: format!(
                    "cannot quarantine Cloud file {}: {error}; prior moves were rolled back",
                    comparison.local_path.display()
                ),
            });
        }
        moved.push((comparison.local_path.clone(), destination.clone()));
    }
    let cache_source = status.accounts[0]
        .path
        .parent()
        .ok_or_else(|| Error {
            message: "Steam account cache has no AppID directory".to_owned(),
        })?
        .to_path_buf();
    let cache_destination = backup.join("steam-cache");
    if let Err(error) = fs::rename(&cache_source, &cache_destination) {
        for (source, destination) in moved.iter().rev() {
            let _ = fs::rename(destination, source);
        }
        return Err(Error {
            message: format!(
                "cannot quarantine local Steam Cloud cache {}: {error}; save moves were rolled back",
                cache_source.display()
            ),
        });
    }
    transaction.phase = Phase::Quarantined;
    transaction.cloud_log_offset =
        cloud_status::log_offset(&status.log_path).map_err(convert_cloud)?;
    if let Err(error) = persist(&game_root, &transaction) {
        let _ = fs::rename(&cache_destination, &cache_source);
        for (source, destination) in moved.iter().rev() {
            let _ = fs::rename(destination, source);
        }
        return Err(error);
    }
    Ok(transaction)
}

pub fn verify_download(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
) -> Result<Transaction, Error> {
    validate_profile(app_id, profile)?;
    require_idle(app_id, data_root)?;
    let game_root = game_root(app_id, data_root);
    let mut transaction = read_transaction(&game_root)?;
    validate_transaction(&transaction, app_id, profile, Phase::Quarantined)?;
    let (status, hashes) = synced_hashes(app_id, profile, data_root)?;
    if status.accounts[0].account_id != transaction.account_id {
        return Err(Error {
            message: "downloaded Steam account cache belongs to a different local account"
                .to_owned(),
        });
    }
    if hashes != transaction.uploaded_sha1 {
        return Err(Error {
            message: "downloaded save content does not match the confirmed uploaded content"
                .to_owned(),
        });
    }
    if !cloud_status::has_transfer_event_since(
        &status.log_path,
        transaction.cloud_log_offset,
        app_id,
        TransferEvent::DownloadComplete,
    )
    .map_err(convert_cloud)?
    {
        return Err(Error {
            message: "no successful Steam Cloud download occurred after quarantine".to_owned(),
        });
    }
    transaction.phase = Phase::Complete;
    transaction.cloud_log_offset =
        cloud_status::log_offset(&status.log_path).map_err(convert_cloud)?;
    persist(&game_root, &transaction)?;
    Ok(transaction)
}

pub fn restore(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
) -> Result<Transaction, Error> {
    validate_profile(app_id, profile)?;
    require_idle(app_id, data_root)?;
    let game_root = game_root(app_id, data_root);
    let mut transaction = read_transaction(&game_root)?;
    validate_transaction(&transaction, app_id, profile, Phase::Quarantined)?;
    let prefix = game_root.join("prefix");
    let local_paths = profile_local_paths(&prefix, profile, Some(&transaction.account_id))?;
    let backup = transaction_root(&game_root, transaction.transaction_id).join("quarantine");
    let sources = (0..local_paths.len())
        .map(|index| backup.join(format!("{index:04}.save")))
        .collect::<Vec<_>>();
    if local_paths
        .iter()
        .zip(&profile.cloud_files)
        .any(|(path, spec)| spec.local_root != CloudLocalRoot::SteamRemote && path.exists())
    {
        return Err(Error {
            message: "a local save already exists; refusing to overwrite it during restore"
                .to_owned(),
        });
    }
    if sources
        .iter()
        .zip(&profile.cloud_files)
        .any(|(path, spec)| spec.local_root != CloudLocalRoot::SteamRemote && !path.is_file())
    {
        return Err(Error {
            message: "Cloud quarantine backup is incomplete; no files were restored".to_owned(),
        });
    }
    let cache_source = backup.join("steam-cache");
    let cache_destination = prefix
        .join("drive_c/Program Files (x86)/Steam/userdata")
        .join(&transaction.account_id)
        .join(app_id.to_string());
    if !cache_source.is_dir() || cache_destination.exists() {
        return Err(Error {
            message: "Cloud cache cannot be restored without overwriting current Steam data"
                .to_owned(),
        });
    }
    if let Some(parent) = cache_destination.parent() {
        fs::create_dir_all(parent).map_err(|error| Error {
            message: format!(
                "cannot recreate Steam Cloud account directory {}: {error}",
                parent.display()
            ),
        })?;
    }
    fs::rename(&cache_source, &cache_destination).map_err(|error| Error {
        message: format!("cannot restore quarantined Steam Cloud cache: {error}"),
    })?;
    let mut moved = Vec::new();
    for ((source, local_path), spec) in sources.iter().zip(&local_paths).zip(&profile.cloud_files) {
        if spec.local_root == CloudLocalRoot::SteamRemote {
            if !local_path.is_file() {
                for (source, destination) in moved.iter().rev() {
                    let _ = fs::rename(destination, source);
                }
                let _ = fs::rename(&cache_destination, &cache_source);
                return Err(Error {
                    message: format!(
                        "restored Steam Remote Storage file is missing from cache: {}",
                        local_path.display()
                    ),
                });
            }
            continue;
        }
        if let Some(parent) = local_path.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            for (source, destination) in moved.iter().rev() {
                let _ = fs::rename(destination, source);
            }
            let _ = fs::rename(&cache_destination, &cache_source);
            return Err(Error {
                message: format!(
                    "cannot recreate save directory {}: {error}",
                    parent.display()
                ),
            });
        }
        if let Err(error) = fs::rename(source, local_path) {
            for (source, destination) in moved.iter().rev() {
                let _ = fs::rename(destination, source);
            }
            let _ = fs::rename(&cache_destination, &cache_source);
            return Err(Error {
                message: format!(
                    "cannot restore Cloud file {}: {error}; prior moves were rolled back",
                    local_path.display()
                ),
            });
        }
        moved.push((source.clone(), local_path.clone()));
    }
    transaction.phase = Phase::Restored;
    persist(&game_root, &transaction)?;
    Ok(transaction)
}

/// Move an obsolete baseline transaction out of the active marker without
/// deleting its evidence. This is intentionally profile-independent: a
/// profile may have been revised after the transaction was created, while the
/// transaction's own AppID/phase remain the authoritative safety checks.
pub fn abandon(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
) -> Result<Transaction, Error> {
    validate_profile(app_id, profile)?;
    require_idle(app_id, data_root)?;
    let game_root = game_root(app_id, data_root);
    let transaction = read_transaction(&game_root)?;
    if transaction.app_id != app_id || transaction.phase != Phase::Baseline {
        return Err(Error {
            message: format!(
                "only a baseline transaction for AppID {app_id} can be abandoned; current phase is {}",
                transaction.phase.as_str()
            ),
        });
    }
    let marker = game_root.join(CURRENT_MARKER);
    let abandoned =
        transaction_root(&game_root, transaction.transaction_id).join("transaction.abandoned");
    fs::rename(&marker, &abandoned).map_err(|error| Error {
        message: format!(
            "cannot move Cloud transaction marker to {}: {error}",
            abandoned.display()
        ),
    })?;
    Ok(transaction)
}

fn synced_hashes(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
) -> Result<(cloud_status::CloudStatus, Vec<String>), Error> {
    let mut status = cloud_status::inspect(app_id, data_root).map_err(convert_cloud)?;
    cloud_status::compare_profile_files(&mut status, profile).map_err(convert_cloud)?;
    if !roundtrip_baseline_ready(status.state, status.pending_remote_operations.len()) {
        return Err(Error {
            message: format!(
                "Steam Cloud must be content-verified synced and launch-ready; found {}, pending client operations {}",
                status.state.as_str(),
                status.pending_remote_operations.len()
            ),
        });
    }
    let comparisons = unique_comparisons(&status, profile.cloud_files.len())?;
    let mut hashes = Vec::new();
    for comparison in comparisons {
        if comparison.state != ComparisonState::Match {
            return Err(Error {
                message: format!(
                    "Cloud file {:?} is {}; all declared files must match",
                    comparison.remote_path,
                    comparison.state.as_str()
                ),
            });
        }
        hashes.push(comparison.local_sha1.clone().ok_or_else(|| Error {
            message: format!(
                "Cloud file {:?} has no local SHA-1 evidence",
                comparison.remote_path
            ),
        })?);
    }
    Ok((status, hashes))
}

fn roundtrip_baseline_ready(state: SyncState, pending_operations: usize) -> bool {
    state == SyncState::Synced && pending_operations == 0
}

fn unique_comparisons(
    status: &cloud_status::CloudStatus,
    expected: usize,
) -> Result<Vec<&cloud_status::CloudFileComparison>, Error> {
    if status.accounts.len() != 1 || status.comparisons.len() != expected {
        return Err(Error {
            message: "Cloud roundtrip requires exactly one Steam account cache and one comparison per declared file".to_owned(),
        });
    }
    Ok(status.comparisons.iter().collect())
}

fn require_idle(app_id: u32, data_root: &Path) -> Result<(), Error> {
    let resources = stop::inspect_resources(app_id, data_root).map_err(|error| Error {
        message: format!("cannot inspect prefix resources: {error}"),
    })?;
    if !resources.pids.is_empty() {
        return Err(Error {
            message: format!(
                "Cloud roundtrip mutation requires an idle AppID prefix; {} process(es) remain",
                resources.pids.len()
            ),
        });
    }
    Ok(())
}

fn profile_local_paths(
    prefix: &Path,
    profile: &CompatibilityProfile,
    account_id: Option<&str>,
) -> Result<Vec<PathBuf>, Error> {
    let needs_user = profile
        .cloud_files
        .iter()
        .any(|spec| spec.local_root == CloudLocalRoot::WineUser);
    let user = if needs_user {
        let users_root = prefix.join("drive_c/users");
        let mut users = fs::read_dir(&users_root)
            .map_err(|error| Error {
                message: format!("cannot read Wine users {}: {error}", users_root.display()),
            })?
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter(|entry| !matches!(entry.file_name().to_str(), Some("Public" | "Default")))
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        users.sort();
        if users.len() != 1 {
            return Err(Error {
                message: "Cloud roundtrip requires exactly one Wine user directory".to_owned(),
            });
        }
        Some(users.remove(0))
    } else {
        None
    };

    profile
        .cloud_files
        .iter()
        .map(|spec| match spec.local_root {
            CloudLocalRoot::WineUser => Ok(user
                .as_ref()
                .expect("Wine user was resolved for a user-root Cloud file")
                .join(&spec.relative_path)),
            CloudLocalRoot::SteamInstall => {
                let install_dir = profile.steam_install_dir.as_ref().ok_or_else(|| Error {
                    message:
                        "steam-install Cloud mapping requires steam_install_dir in the profile"
                            .to_owned(),
                })?;
                Ok(prefix
                    .join("drive_c/Program Files (x86)/Steam/steamapps/common")
                    .join(install_dir)
                    .join(&spec.relative_path))
            }
            CloudLocalRoot::SteamRemote => {
                let account_id = account_id.ok_or_else(|| Error {
                    message: "steam-remote Cloud mapping requires a selected Steam account"
                        .to_owned(),
                })?;
                Ok(prefix
                    .join("drive_c/Program Files (x86)/Steam/userdata")
                    .join(account_id)
                    .join(profile.app_id.to_string())
                    .join("remote")
                    .join(&spec.relative_path))
            }
        })
        .collect()
}

fn validate_profile(app_id: u32, profile: &CompatibilityProfile) -> Result<(), Error> {
    if profile.app_id != app_id {
        return Err(Error {
            message: format!(
                "profile AppID {} does not match requested AppID {app_id}",
                profile.app_id
            ),
        });
    }
    if profile.cloud_files.is_empty() {
        return Err(Error {
            message: "Cloud roundtrip requires declared cloud_files".to_owned(),
        });
    }
    Ok(())
}

fn validate_transaction(
    transaction: &Transaction,
    app_id: u32,
    profile: &CompatibilityProfile,
    expected_phase: Phase,
) -> Result<(), Error> {
    if transaction.app_id != app_id
        || transaction.profile_sha256 != profile.fingerprint()
        || transaction.phase != expected_phase
        || transaction.account_id.is_empty()
        || !transaction
            .account_id
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || transaction.baseline_sha1.len() != profile.cloud_files.len()
        || (!transaction.uploaded_sha1.is_empty()
            && transaction.uploaded_sha1.len() != profile.cloud_files.len())
    {
        return Err(Error {
            message: format!(
                "Cloud roundtrip transaction does not match AppID/profile/phase {}",
                expected_phase.as_str()
            ),
        });
    }
    Ok(())
}

fn persist(game_root: &Path, transaction: &Transaction) -> Result<(), Error> {
    fs::create_dir_all(game_root).map_err(|error| Error {
        message: format!(
            "cannot create game evidence root {}: {error}",
            game_root.display()
        ),
    })?;
    let transaction_root = transaction_root(game_root, transaction.transaction_id);
    fs::create_dir_all(&transaction_root).map_err(|error| Error {
        message: format!(
            "cannot create Cloud transaction root {}: {error}",
            transaction_root.display()
        ),
    })?;
    let contents = serialize(transaction);
    write_atomic(&transaction_root.join("transaction.state"), &contents)?;
    write_atomic(&game_root.join(CURRENT_MARKER), &contents)
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), Error> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, contents).map_err(|error| Error {
        message: format!(
            "cannot write Cloud transaction state {}: {error}",
            temporary.display()
        ),
    })?;
    fs::rename(&temporary, path).map_err(|error| Error {
        message: format!(
            "cannot commit Cloud transaction state {}: {error}",
            path.display()
        ),
    })
}

fn read_transaction(game_root: &Path) -> Result<Transaction, Error> {
    let marker = game_root.join(CURRENT_MARKER);
    let contents = fs::read_to_string(&marker).map_err(|error| Error {
        message: format!(
            "cannot read Cloud transaction state {}: {error}",
            marker.display()
        ),
    })?;
    parse(&contents)
}

fn serialize(transaction: &Transaction) -> String {
    format!(
        concat!(
            "schema=1\n",
            "appid={}\n",
            "profile_sha256={}\n",
            "transaction_id={}\n",
            "phase={}\n",
            "cloud_log_offset={}\n",
            "account_id={}\n",
            "baseline_sha1={}\n",
            "uploaded_sha1={}\n"
        ),
        transaction.app_id,
        transaction.profile_sha256,
        transaction.transaction_id,
        transaction.phase.as_str(),
        transaction.cloud_log_offset,
        transaction.account_id,
        transaction.baseline_sha1.join(","),
        transaction.uploaded_sha1.join(","),
    )
}

fn parse(contents: &str) -> Result<Transaction, Error> {
    let allowed = [
        "schema",
        "appid",
        "profile_sha256",
        "transaction_id",
        "phase",
        "cloud_log_offset",
        "account_id",
        "baseline_sha1",
        "uploaded_sha1",
    ];
    let mut fields = BTreeMap::new();
    for line in contents.lines() {
        let (key, value) = line.split_once('=').ok_or_else(|| Error {
            message: "Cloud transaction contains a malformed line".to_owned(),
        })?;
        if !allowed.contains(&key) || fields.insert(key, value).is_some() {
            return Err(Error {
                message: format!("Cloud transaction contains unknown/duplicate field {key:?}"),
            });
        }
    }
    if fields.len() != allowed.len() || fields.get("schema") != Some(&"1") {
        return Err(Error {
            message: "Cloud transaction has missing fields or unsupported schema".to_owned(),
        });
    }
    let hashes = |value: &str| -> Result<Vec<String>, Error> {
        if value.is_empty() {
            return Ok(Vec::new());
        }
        let values = value.split(',').map(str::to_owned).collect::<Vec<_>>();
        if values
            .iter()
            .any(|value| value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err(Error {
                message: "Cloud transaction contains an invalid SHA-1 list".to_owned(),
            });
        }
        Ok(values)
    };
    let profile_sha256 = fields["profile_sha256"].to_owned();
    if profile_sha256.len() != 64 || !profile_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error {
            message: "Cloud transaction contains an invalid profile SHA-256".to_owned(),
        });
    }
    Ok(Transaction {
        app_id: fields["appid"].parse().map_err(|_| Error {
            message: "Cloud transaction contains an invalid AppID".to_owned(),
        })?,
        profile_sha256,
        transaction_id: fields["transaction_id"].parse().map_err(|_| Error {
            message: "Cloud transaction contains an invalid transaction ID".to_owned(),
        })?,
        phase: Phase::parse(fields["phase"]).ok_or_else(|| Error {
            message: "Cloud transaction contains an invalid phase".to_owned(),
        })?,
        cloud_log_offset: fields["cloud_log_offset"].parse().map_err(|_| Error {
            message: "Cloud transaction contains an invalid log offset".to_owned(),
        })?,
        account_id: fields["account_id"].to_owned(),
        baseline_sha1: hashes(fields["baseline_sha1"])?,
        uploaded_sha1: hashes(fields["uploaded_sha1"])?,
    })
}

fn game_root(app_id: u32, data_root: &Path) -> PathBuf {
    data_root.join("games").join(app_id.to_string())
}

fn transaction_root(game_root: &Path, transaction_id: u128) -> PathBuf {
    game_root
        .join("cloud-roundtrip")
        .join(transaction_id.to_string())
}

fn unix_nanos() -> Result<u128, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| Error {
            message: format!("system clock is before the Unix epoch: {error}"),
        })
}

fn convert_cloud(error: cloud_status::Error) -> Error {
    Error {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha1;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn transaction_state_is_strict_and_roundtrips() {
        let transaction = Transaction {
            app_id: 42,
            profile_sha256: "ab".repeat(32),
            transaction_id: 123,
            phase: Phase::Uploaded,
            cloud_log_offset: 456,
            account_id: "123".to_owned(),
            baseline_sha1: vec!["cd".repeat(20)],
            uploaded_sha1: vec!["ef".repeat(20)],
        };
        assert_eq!(parse(&serialize(&transaction)).unwrap(), transaction);
        assert!(parse(&(serialize(&transaction) + "unknown=value\n")).is_err());
        assert!(
            parse(&serialize(&transaction).replace("baseline_sha1=cd", "baseline_sha1=zz"))
                .is_err()
        );
    }

    #[test]
    fn roundtrip_baseline_rejects_pending_client_launch_operations() {
        assert!(roundtrip_baseline_ready(SyncState::Synced, 0));
        assert!(!roundtrip_baseline_ready(SyncState::Synced, 1));
        assert!(!roundtrip_baseline_ready(SyncState::Syncing, 0));
    }

    #[test]
    fn abandon_moves_only_an_obsolete_baseline_marker() {
        let root = std::env::temp_dir().join(format!(
            "macroton-cloud-abandon-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let app_id = 42;
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=slot.sav|AppData/Roaming/Game/slot.sav",
        )
        .unwrap();
        let transaction = Transaction {
            app_id,
            profile_sha256: profile.fingerprint(),
            transaction_id: 987,
            phase: Phase::Baseline,
            cloud_log_offset: 12,
            account_id: "123".to_owned(),
            baseline_sha1: vec!["ab".repeat(20)],
            uploaded_sha1: Vec::new(),
        };
        let game = game_root(app_id, &root);
        fs::create_dir_all(game.join("prefix")).unwrap();
        persist(&game, &transaction).unwrap();

        let abandoned = abandon(app_id, &profile, &root).unwrap();
        assert_eq!(abandoned, transaction);
        assert!(!game.join("cloud-roundtrip.current").exists());
        let marker =
            transaction_root(&game, transaction.transaction_id).join("transaction.abandoned");
        assert_eq!(
            parse(&fs::read_to_string(marker).unwrap()).unwrap(),
            transaction
        );
        assert!(game.join("cloud-roundtrip/987/transaction.state").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn install_root_cloud_paths_do_not_require_a_wine_user_directory() {
        let root = std::env::temp_dir().join(format!(
            "macroton-cloud-install-root-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let prefix = root.join("games/42/prefix");
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nsteam_install_dir=Game\nexecutable=game.exe\ncloud_files=Profiles/Player.nfp|steam-install:Profiles/Player.nfp",
        )
        .unwrap();
        let paths = profile_local_paths(&prefix, &profile, None).unwrap();
        assert_eq!(
            paths,
            vec![prefix.join(
                "drive_c/Program Files (x86)/Steam/steamapps/common/Game/Profiles/Player.nfp"
            )]
        );

        let missing_install = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=Profiles/Player.nfp|steam-install:Profiles/Player.nfp",
        )
        .unwrap_err();
        assert!(
            missing_install
                .message
                .contains("requires steam_install_dir")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn real_files_require_new_upload_and_download_events() {
        let root = std::env::temp_dir().join(format!(
            "macroton-cloud-roundtrip-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let app_id = 42;
        let steam = root.join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let account = steam.join("userdata/123/42");
        let save = root.join("games/42/prefix/drive_c/users/player/AppData/Roaming/Game/slot.sav");
        fs::create_dir_all(save.parent().unwrap()).unwrap();
        fs::create_dir_all(steam.join("logs")).unwrap();
        fs::create_dir_all(&account).unwrap();
        let write_cache = |bytes: &[u8]| {
            let digest = sha1::to_hex(&sha1::digest_bytes(bytes));
            fs::write(
                account.join("remotecache.vdf"),
                format!(
                    "\"42\" {{ \"slot.sav\" {{ \"size\" \"{}\" \"sha\" \"{digest}\" \"syncstate\" \"1\" }} }}",
                    bytes.len()
                ),
            )
            .unwrap();
        };
        let baseline = b"baseline";
        fs::write(&save, baseline).unwrap();
        write_cache(baseline);
        let cloud_log = steam.join("logs/cloud_log.txt");
        fs::write(&cloud_log, "[old] [AppID 42] Successfully synced\n").unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=slot.sav|AppData/Roaming/Game/slot.sav",
        )
        .unwrap();

        let transaction = begin(app_id, &profile, &root).unwrap();
        assert_eq!(transaction.phase, Phase::Baseline);
        assert!(
            confirm_upload(app_id, &profile, &root)
                .unwrap_err()
                .message
                .contains("did not change")
        );

        let uploaded = b"new save from gameplay";
        fs::write(&save, uploaded).unwrap();
        write_cache(uploaded);
        let mut log = fs::OpenOptions::new()
            .append(true)
            .open(&cloud_log)
            .unwrap();
        writeln!(log, "[new] [AppID 42] Upload complete, result OK").unwrap();
        drop(log);
        assert_eq!(
            confirm_upload(app_id, &profile, &root).unwrap().phase,
            Phase::Uploaded
        );
        let quarantined = quarantine(app_id, &profile, &root).unwrap();
        assert_eq!(quarantined.phase, Phase::Quarantined);
        assert!(!save.exists());
        assert!(!account.exists());
        assert!(
            verify_download(app_id, &profile, &root)
                .unwrap_err()
                .message
                .contains("exactly one Steam account cache")
        );

        fs::create_dir_all(&account).unwrap();
        write_cache(uploaded);
        fs::write(&save, uploaded).unwrap();
        let mut log = fs::OpenOptions::new()
            .append(true)
            .open(&cloud_log)
            .unwrap();
        writeln!(log, "[newer] [AppID 42] Download complete, result OK").unwrap();
        drop(log);
        assert_eq!(
            verify_download(app_id, &profile, &root).unwrap().phase,
            Phase::Complete
        );
        assert_eq!(fs::read(&save).unwrap(), uploaded);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn steam_remote_storage_is_quarantined_and_restored_with_its_cache() {
        let root = std::env::temp_dir().join(format!(
            "macroton-cloud-remote-storage-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let app_id = 42;
        let steam = root.join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let account = steam.join("userdata/123/42");
        let remote = account.join("remote/savegame.aws");
        fs::create_dir_all(remote.parent().unwrap()).unwrap();
        fs::create_dir_all(steam.join("logs")).unwrap();
        let write_cache = |bytes: &[u8]| {
            let digest = sha1::to_hex(&sha1::digest_bytes(bytes));
            fs::write(
                account.join("remotecache.vdf"),
                format!(
                    "\"42\" {{ \"savegame.aws\" {{ \"size\" \"{}\" \"sha\" \"{digest}\" \"syncstate\" \"1\" }} }}",
                    bytes.len()
                ),
            )
            .unwrap();
        };
        let baseline = b"api baseline";
        fs::write(&remote, baseline).unwrap();
        write_cache(baseline);
        let cloud_log = steam.join("logs/cloud_log.txt");
        fs::write(&cloud_log, "[old] [AppID 42] Successfully synced\n").unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=savegame.aws|steam-remote:savegame.aws",
        )
        .unwrap();

        begin(app_id, &profile, &root).unwrap();
        let uploaded = b"api save from gameplay";
        fs::write(&remote, uploaded).unwrap();
        write_cache(uploaded);
        let mut log = fs::OpenOptions::new()
            .append(true)
            .open(&cloud_log)
            .unwrap();
        writeln!(log, "[new] [AppID 42] Upload complete, result OK").unwrap();
        drop(log);
        confirm_upload(app_id, &profile, &root).unwrap();
        let transaction = quarantine(app_id, &profile, &root).unwrap();
        assert!(!account.exists());
        let quarantine = transaction_root(&game_root(app_id, &root), transaction.transaction_id)
            .join("quarantine");
        assert!(!quarantine.join("0000.save").exists());
        assert_eq!(
            fs::read(quarantine.join("steam-cache/remote/savegame.aws")).unwrap(),
            uploaded
        );

        let restored = restore(app_id, &profile, &root).unwrap();
        assert_eq!(restored.phase, Phase::Restored);
        assert_eq!(fs::read(&remote).unwrap(), uploaded);
        assert!(account.join("remotecache.vdf").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn restore_recovers_quarantined_save_and_cloud_cache_without_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "macroton-cloud-restore-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let game_root = root.join("games/42");
        let prefix = game_root.join("prefix");
        let user = prefix.join("drive_c/users/player");
        fs::create_dir_all(&user).unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=slot.sav|AppData/Roaming/Game/slot.sav",
        )
        .unwrap();
        let transaction = Transaction {
            app_id: 42,
            profile_sha256: profile.fingerprint(),
            transaction_id: 123,
            phase: Phase::Quarantined,
            cloud_log_offset: 0,
            account_id: "789".to_owned(),
            baseline_sha1: vec!["ab".repeat(20)],
            uploaded_sha1: vec!["cd".repeat(20)],
        };
        let quarantine = transaction_root(&game_root, 123).join("quarantine");
        fs::create_dir_all(quarantine.join("steam-cache")).unwrap();
        fs::write(quarantine.join("0000.save"), b"recover me").unwrap();
        fs::write(quarantine.join("steam-cache/remotecache.vdf"), b"cache").unwrap();
        persist(&game_root, &transaction).unwrap();

        let restored = restore(42, &profile, &root).unwrap();
        assert_eq!(restored.phase, Phase::Restored);
        assert_eq!(
            fs::read(user.join("AppData/Roaming/Game/slot.sav")).unwrap(),
            b"recover me"
        );
        assert_eq!(
            fs::read(
                prefix.join("drive_c/Program Files (x86)/Steam/userdata/789/42/remotecache.vdf")
            )
            .unwrap(),
            b"cache"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
