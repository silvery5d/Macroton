use crate::keyvalues::{self, Value};
use crate::profile::{CloudLocalRoot, CompatibilityProfile};
use crate::sha1;
use std::fmt;
use std::fs;
use std::io::Read;
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    NoEvidence,
    Syncing,
    Synced,
    LocalDrift,
    IncompleteLocal,
    Failed,
}

impl SyncState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoEvidence => "no-evidence",
            Self::Syncing => "syncing",
            Self::Synced => "synced",
            Self::LocalDrift => "local-drift",
            Self::IncompleteLocal => "incomplete-local",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountCache {
    pub account_id: String,
    pub path: PathBuf,
    pub tracked_files: usize,
    pub synced_files: usize,
    pub pending_files: usize,
    pub platform_skipped_files: usize,
    pub files: Vec<CachedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedFile {
    pub relative_path: String,
    pub size: Option<u64>,
    pub sha1: Option<String>,
    pub sync_state: Option<u32>,
    pub platform_skipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudStatus {
    pub app_id: u32,
    pub prefix: PathBuf,
    pub log_path: PathBuf,
    pub state: SyncState,
    pub latest_event: Option<String>,
    /// Steam can report a synchronized cache while still holding a pending
    /// launch operation. Keep that client-side signal separate from the file
    /// cache state so callers do not mistake `synced` for launch-ready.
    pub pending_remote_operations: Vec<String>,
    pub accounts: Vec<AccountCache>,
    pub comparisons: Vec<CloudFileComparison>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonState {
    Match,
    LocalDrift,
    MissingLocal,
    MissingCache,
    MissingRemoteCopy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteCopyEncoding {
    Raw,
    SteamZlib,
}

impl RemoteCopyEncoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::SteamZlib => "steam-zlib",
        }
    }
}

impl ComparisonState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::LocalDrift => "local-drift",
            Self::MissingLocal => "missing-local",
            Self::MissingCache => "missing-cache",
            Self::MissingRemoteCopy => "missing-remote-copy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudFileComparison {
    pub account_id: String,
    pub remote_path: String,
    pub local_path: PathBuf,
    pub remote_copy_path: PathBuf,
    pub state: ComparisonState,
    pub cached_size: Option<u64>,
    pub local_size: Option<u64>,
    pub remote_copy_size: Option<u64>,
    pub remote_content_size: Option<u64>,
    pub cached_sha1: Option<String>,
    pub local_sha1: Option<String>,
    pub remote_copy_sha1: Option<String>,
    pub remote_content_sha1: Option<String>,
    pub remote_copy_encoding: Option<RemoteCopyEncoding>,
}

#[derive(Debug)]
struct RemoteCopy {
    physical_size: u64,
    physical_sha1: String,
    content_size: u64,
    content_sha1: String,
    encoding: RemoteCopyEncoding,
}

#[derive(Debug)]
pub struct Error {
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferEvent {
    UploadComplete,
    DownloadComplete,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub fn inspect(app_id: u32, data_root: &Path) -> Result<CloudStatus, Error> {
    let prefix = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix");
    let steam_root = prefix
        .join("drive_c")
        .join("Program Files (x86)")
        .join("Steam");
    let mut status = inspect_steam_root(app_id, &steam_root)?;
    status.prefix = prefix;
    Ok(status)
}

pub fn inspect_steam_root(app_id: u32, steam_root: &Path) -> Result<CloudStatus, Error> {
    if !steam_root.is_dir() {
        return Err(Error {
            message: format!(
                "Windows Steam root for AppID {app_id} does not exist: {}",
                steam_root.display()
            ),
        });
    }

    let accounts = inspect_account_caches(app_id, &steam_root.join("userdata"))?;
    let log_path = steam_root.join("logs/cloud_log.txt");
    let (state, latest_event) = inspect_log(app_id, &log_path)?;
    let pending_remote_operations = inspect_pending_remote_operations(app_id, &log_path)?;
    Ok(CloudStatus {
        app_id,
        prefix: wine_prefix_for_steam_root(steam_root).unwrap_or_else(|| steam_root.to_path_buf()),
        log_path,
        state,
        latest_event,
        pending_remote_operations,
        accounts,
        comparisons: Vec::new(),
    })
}

/// Return the most recent Steam client-side pending launch operations for an
/// AppID. These are not equivalent to pending files in `remotecache.vdf`:
/// Steam may leave an operation pending even after the cache is synchronized.
pub fn inspect_pending_remote_operations(app_id: u32, path: &Path) -> Result<Vec<String>, Error> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(path).map_err(|error| Error {
        message: format!("cannot read Steam Cloud log {}: {error}", path.display()),
    })?;
    let marker = format!("[AppID {app_id}]");
    let mut latest = Vec::new();
    let mut collecting = false;
    for line in contents.lines() {
        // Steam may retain the original pending-operation diagnostic in the
        // append-only log even after a later AC Launch has successfully
        // synchronized and begun watching the app. Once that same AppID has
        // reached the launch watcher, the older operation is no longer a
        // launch blocker; only a newer pending block should be reported.
        if line.contains(&marker)
            && line
                .to_ascii_lowercase()
                .contains("running autocloud on launch")
        {
            latest.clear();
            collecting = false;
            continue;
        }
        if line.contains(&marker)
            && line
                .to_ascii_lowercase()
                .contains("pending remote operations")
        {
            latest.clear();
            collecting = true;
            continue;
        }
        if collecting {
            if line.contains("Operation '") {
                latest.push(line.trim().to_owned());
            } else if !line.trim().is_empty() {
                collecting = false;
            }
        }
    }
    Ok(latest)
}

pub fn compare_profile_files(
    status: &mut CloudStatus,
    profile: &CompatibilityProfile,
) -> Result<(), Error> {
    let users_root = status.prefix.join("drive_c/users");
    let user = if profile
        .cloud_files
        .iter()
        .any(|spec| spec.local_root == CloudLocalRoot::WineUser)
    {
        Some(default_wine_user(&users_root).ok_or_else(|| Error {
            message: format!("cannot identify a Wine user under {}", users_root.display()),
        })?)
    } else {
        None
    };
    let mut comparisons = Vec::new();
    for account in &status.accounts {
        let remote_root = account.path.parent().ok_or_else(|| Error {
            message: format!("Steam cache path has no parent: {}", account.path.display()),
        })?;
        for spec in &profile.cloud_files {
            let local_path = match spec.local_root {
                CloudLocalRoot::WineUser => user
                    .as_ref()
                    .expect("Wine user was resolved for a user-root Cloud file")
                    .join(&spec.relative_path),
                CloudLocalRoot::SteamInstall => {
                    let install_dir = profile.steam_install_dir.as_ref().ok_or_else(|| Error {
                        message:
                            "steam-install Cloud mapping requires steam_install_dir in the profile"
                                .to_owned(),
                    })?;
                    status
                        .prefix
                        .join("drive_c/Program Files (x86)/Steam/steamapps/common")
                        .join(install_dir)
                        .join(&spec.relative_path)
                }
                CloudLocalRoot::SteamRemote => remote_root.join("remote").join(&spec.relative_path),
            };
            let remote_copy_path = remote_root.join("remote").join(&spec.remote_path);
            let cached = account
                .files
                .iter()
                .find(|file| file.relative_path == spec.remote_path);
            let local_size = file_size(&local_path)?;
            let remote_copy = inspect_remote_copy(&remote_copy_path)?;
            let remote_copy_size = remote_copy.as_ref().map(|copy| copy.physical_size);
            let remote_content_size = remote_copy.as_ref().map(|copy| copy.content_size);
            let local_sha1 = file_sha1(&local_path)?;
            let remote_copy_sha1 = remote_copy.as_ref().map(|copy| copy.physical_sha1.clone());
            let remote_content_sha1 = remote_copy.as_ref().map(|copy| copy.content_sha1.clone());
            let remote_copy_encoding = remote_copy.as_ref().map(|copy| copy.encoding);
            let state = match cached {
                None => ComparisonState::MissingCache,
                Some(_) if local_size.is_none() => ComparisonState::MissingLocal,
                Some(cached) if remote_copy_size.is_none() => {
                    if cached.sync_state == Some(1)
                        && cached.size.is_some()
                        && cached.sha1.is_some()
                        && cached.size == local_size
                        && cached.sha1 == local_sha1
                    {
                        ComparisonState::Match
                    } else if cached.size.is_some()
                        && cached.sha1.is_some()
                        && local_size.is_some()
                        && local_sha1.is_some()
                    {
                        ComparisonState::LocalDrift
                    } else {
                        ComparisonState::MissingRemoteCopy
                    }
                }
                Some(cached)
                    if cached.size == remote_copy_size && cached.sha1 == remote_copy_sha1 =>
                {
                    if local_size == remote_content_size && local_sha1 == remote_content_sha1 {
                        ComparisonState::Match
                    } else {
                        ComparisonState::LocalDrift
                    }
                }
                Some(_) => ComparisonState::LocalDrift,
            };
            comparisons.push(CloudFileComparison {
                account_id: account.account_id.clone(),
                remote_path: spec.remote_path.clone(),
                local_path,
                remote_copy_path,
                state,
                cached_size: cached.and_then(|file| file.size),
                local_size,
                remote_copy_size,
                remote_content_size,
                cached_sha1: cached.and_then(|file| file.sha1.clone()),
                local_sha1,
                remote_copy_sha1,
                remote_content_sha1,
                remote_copy_encoding,
            });
        }
    }
    status.comparisons = comparisons;
    if status
        .comparisons
        .iter()
        .any(|comparison| comparison.state == ComparisonState::LocalDrift)
    {
        status.state = SyncState::LocalDrift;
    } else if status.comparisons.iter().any(|comparison| {
        matches!(
            comparison.state,
            ComparisonState::MissingLocal
                | ComparisonState::MissingCache
                | ComparisonState::MissingRemoteCopy
        )
    }) {
        status.state = SyncState::IncompleteLocal;
    }
    Ok(())
}

pub fn log_offset(path: &Path) -> Result<u64, Error> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(metadata.len()),
        Ok(_) => Ok(0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(Error {
            message: format!("cannot inspect Steam Cloud log {}: {error}", path.display()),
        }),
    }
}

pub fn has_transfer_event_since(
    path: &Path,
    offset: u64,
    app_id: u32,
    event: TransferEvent,
) -> Result<bool, Error> {
    let length = log_offset(path)?;
    if length < offset {
        return Err(Error {
            message: "Steam Cloud log was truncated after the transaction checkpoint".to_owned(),
        });
    }
    if length == offset {
        return Ok(false);
    }
    let mut file = fs::File::open(path).map_err(|error| Error {
        message: format!("cannot open Steam Cloud log {}: {error}", path.display()),
    })?;
    file.seek(SeekFrom::Start(offset)).map_err(|error| Error {
        message: format!("cannot seek Steam Cloud log {}: {error}", path.display()),
    })?;
    let mut suffix = String::new();
    file.read_to_string(&mut suffix).map_err(|error| Error {
        message: format!(
            "cannot read Steam Cloud log suffix {}: {error}",
            path.display()
        ),
    })?;
    let app_marker = format!("[AppID {app_id}]");
    Ok(suffix.lines().any(|line| {
        let lower = line.to_ascii_lowercase();
        line.contains(&app_marker)
            && match event {
                TransferEvent::UploadComplete => {
                    lower.contains("upload complete, result ok")
                        || lower.contains("upload complete, result 1")
                }
                TransferEvent::DownloadComplete => {
                    lower.contains("download complete, result ok")
                        || lower.contains("download complete, result 1")
                }
            }
    }))
}

fn inspect_remote_copy(path: &Path) -> Result<Option<RemoteCopy>, Error> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Error {
                message: format!("cannot read Steam remote copy {}: {error}", path.display()),
            });
        }
    };
    let physical_size = bytes.len() as u64;
    let physical_sha1 = sha1::to_hex(&sha1::digest_bytes(&bytes));
    let decoded = decode_steam_zlib(&bytes);
    let (content, encoding) = decoded
        .as_deref()
        .map(|content| (content, RemoteCopyEncoding::SteamZlib))
        .unwrap_or((bytes.as_slice(), RemoteCopyEncoding::Raw));
    Ok(Some(RemoteCopy {
        physical_size,
        physical_sha1,
        content_size: content.len() as u64,
        content_sha1: sha1::to_hex(&sha1::digest_bytes(content)),
        encoding,
    }))
}

fn decode_steam_zlib(bytes: &[u8]) -> Option<Vec<u8>> {
    const MAX_CLOUD_FILE_SIZE: u64 = 1024 * 1024 * 1024;
    if bytes.len() < 6 {
        return None;
    }
    let expected = u32::from_le_bytes(bytes[..4].try_into().ok()?) as u64;
    let cmf = bytes[4];
    let flg = bytes[5];
    let zlib_header = cmf & 0x0f == 8 && (u16::from(cmf) * 256 + u16::from(flg)) % 31 == 0;
    if !zlib_header || expected > MAX_CLOUD_FILE_SIZE {
        return None;
    }
    let mut decoded = Vec::with_capacity(expected.min(1024 * 1024) as usize);
    let mut limited = ZlibDecoder::new(&bytes[4..]).take(expected + 1);
    limited.read_to_end(&mut decoded).ok()?;
    (decoded.len() as u64 == expected).then_some(decoded)
}

fn default_wine_user(users_root: &Path) -> Option<PathBuf> {
    let mut candidates = fs::read_dir(users_root)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| !matches!(entry.file_name().to_str(), Some("Public" | "Default")))
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    candidates.sort();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

fn file_size(path: &Path) -> Result<Option<u64>, Error> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(metadata.len())),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error {
            message: format!("cannot inspect Cloud file {}: {error}", path.display()),
        }),
    }
}

fn file_sha1(path: &Path) -> Result<Option<String>, Error> {
    if !path.is_file() {
        return Ok(None);
    }
    sha1::digest_file(path)
        .map(|digest| Some(sha1::to_hex(&digest)))
        .map_err(|error| Error {
            message: format!("cannot hash Cloud file {}: {error}", path.display()),
        })
}

fn wine_prefix_for_steam_root(steam_root: &Path) -> Option<PathBuf> {
    steam_root
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "drive_c"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn inspect_account_caches(app_id: u32, userdata: &Path) -> Result<Vec<AccountCache>, Error> {
    if !userdata.is_dir() {
        return Ok(Vec::new());
    }
    let mut accounts = Vec::new();
    let entries = fs::read_dir(userdata).map_err(|error| Error {
        message: format!("cannot read Steam userdata {}: {error}", userdata.display()),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| Error {
            message: format!("cannot read an entry in {}: {error}", userdata.display()),
        })?;
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let account_id = entry.file_name().to_string_lossy().into_owned();
        if !account_id.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let cache = entry
            .path()
            .join(app_id.to_string())
            .join("remotecache.vdf");
        if cache.is_file() {
            accounts.push(parse_cache(app_id, account_id, cache)?);
        }
    }
    accounts.sort_by(|left, right| left.account_id.cmp(&right.account_id));
    Ok(accounts)
}

fn parse_cache(app_id: u32, account_id: String, path: PathBuf) -> Result<AccountCache, Error> {
    let contents = fs::read_to_string(&path).map_err(|error| Error {
        message: format!("cannot read Steam Cloud cache {}: {error}", path.display()),
    })?;
    let parsed = keyvalues::parse(&contents).map_err(|error| Error {
        message: format!("cannot parse Steam Cloud cache {}: {error}", path.display()),
    })?;
    let root = parsed.get(&app_id.to_string()).ok_or_else(|| Error {
        message: format!(
            "Steam Cloud cache {} does not contain AppID {app_id}",
            path.display()
        ),
    })?;
    let mut tracked_files = 0;
    let mut synced_files = 0;
    let mut platform_skipped_files = 0;
    let mut files = Vec::new();
    for (relative_path, value) in root.entries().unwrap_or_default() {
        let Value::Object(_) = value else {
            continue;
        };
        tracked_files += 1;
        let sync_state = value
            .string("syncstate")
            .and_then(|state| state.parse().ok());
        let platform_skipped = value
            .string("platformstosync2")
            .is_some_and(|platforms| platforms != "1" && platforms != "-1");
        if sync_state == Some(1) {
            synced_files += 1;
        } else if platform_skipped {
            platform_skipped_files += 1;
        }
        files.push(CachedFile {
            relative_path: relative_path.clone(),
            size: value.string("size").and_then(|size| size.parse().ok()),
            sha1: value.string("sha").map(str::to_owned),
            sync_state,
            platform_skipped,
        });
    }
    Ok(AccountCache {
        account_id,
        path,
        tracked_files,
        synced_files,
        pending_files: tracked_files.saturating_sub(synced_files + platform_skipped_files),
        platform_skipped_files,
        files,
    })
}

fn inspect_log(app_id: u32, path: &Path) -> Result<(SyncState, Option<String>), Error> {
    if !path.is_file() {
        return Ok((SyncState::NoEvidence, None));
    }
    let contents = fs::read_to_string(path).map_err(|error| Error {
        message: format!("cannot read Steam Cloud log {}: {error}", path.display()),
    })?;
    let marker = format!("[AppID {app_id}]");
    let mut state = SyncState::NoEvidence;
    let mut latest = None;
    for line in contents.lines().filter(|line| line.contains(&marker)) {
        let lower = line.to_ascii_lowercase();
        let next =
            if lower.contains("not running autocloud") && lower.contains("no config data found") {
                // Some Windows depots emit a generic "Starting sync" checkpoint
                // even though the app has no AutoCloud configuration. Treat that
                // later authoritative event as no Cloud evidence, not as an
                // indefinitely syncing session.
                Some(SyncState::NoEvidence)
            } else if lower.contains("starting sync") {
                Some(SyncState::Syncing)
            } else if lower.contains("upload complete, result ok")
                || lower.contains("download complete, result ok")
                || lower.contains("successfully synced")
                || lower.contains("currently already synced")
            {
                Some(SyncState::Synced)
            } else if lower.contains("sync failed")
                || lower.contains("upload failed")
                || lower.contains("download failed")
            {
                Some(SyncState::Failed)
            } else {
                None
            };
        if let Some(next) = next {
            state = next;
            latest = Some(line.to_owned());
        }
    }
    Ok((state, latest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "macroton-cloud-status-test-{}-{sequence}",
                std::process::id()
            ));
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
    fn reports_cache_counts_and_completed_upload() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let cache = steam.join("userdata/123/42/remotecache.vdf");
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(
            &cache,
            r#""42" { "save/a.sav" { "syncstate" "1" } "save/b.sav" { "syncstate" "2" } "linux.cfg" { "syncstate" "2" "platformstosync2" "8" } }"#,
        )
        .unwrap();
        fs::create_dir_all(steam.join("logs")).unwrap();
        fs::write(
            steam.join("logs/cloud_log.txt"),
            "[time] [AppID 42] Starting sync (up,AC Exit,)\n[time] [AppID 42] Upload complete, result OK\n",
        )
        .unwrap();

        let status = inspect(42, &temp.0).unwrap();
        assert_eq!(status.state, SyncState::Synced);
        assert_eq!(status.accounts.len(), 1);
        assert_eq!(status.accounts[0].tracked_files, 3);
        assert_eq!(status.accounts[0].synced_files, 1);
        assert_eq!(status.accounts[0].pending_files, 1);
        assert_eq!(status.accounts[0].platform_skipped_files, 1);
        assert_eq!(status.accounts[0].files.len(), 3);
        assert_eq!(status.accounts[0].files[0].relative_path, "save/a.sav");
        assert!(status.comparisons.is_empty());
        assert!(status.pending_remote_operations.is_empty());
    }

    #[test]
    fn preserves_pending_client_launch_operations_separately_from_synced_cache() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/26800/prefix/drive_c/Program Files (x86)/Steam");
        let log = steam.join("logs/cloud_log.txt");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(
            &log,
            "[time] [AppID 26800] Currently already synced to global change number '6'\n\
[time] [AppID 26800] BYieldingAppLaunchIntent returned 1 pending remote operations:\n\
\tOperation '3' from machine ''\n",
        )
        .unwrap();
        let pending = inspect_pending_remote_operations(26800, &log).unwrap();
        assert_eq!(pending, vec!["Operation '3' from machine ''"]);
    }

    #[test]
    fn repeated_ac_launch_pending_operation_is_not_treated_as_resolved() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/606680/prefix/drive_c/Program Files (x86)/Steam");
        let log = steam.join("logs/cloud_log.txt");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(
            &log,
            "[time] [AppID 606680] Starting sync (AC Launch,down,)\n\
[time] [AppID 606680] Successfully synced to ChangeNumber 5\n\
[time] [AppID 606680] BYieldingAppLaunchIntent returned 1 pending remote operations:\n\
\tOperation '3' from machine ''\n",
        )
        .unwrap();
        assert_eq!(
            inspect_pending_remote_operations(606680, &log).unwrap(),
            vec!["Operation '3' from machine ''"]
        );
    }

    #[test]
    fn global_all_synced_does_not_clear_unresolved_launch_operation() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/606680/prefix/drive_c/Program Files (x86)/Steam");
        let log = steam.join("logs/cloud_log.txt");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(
            &log,
            r#"[time] [AppID 606680] BYieldingAppLaunchIntent returned 1 pending remote operations:
	Operation '3' from machine ''
[time] [Roaming Config] PerformSyncCloud - all sync'd up
"#,
        )
        .unwrap();
        assert_eq!(
            inspect_pending_remote_operations(606680, &log).unwrap(),
            vec!["Operation '3' from machine ''"]
        );
    }

    #[test]
    fn clears_stale_pending_operation_after_later_autocloud_launch() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/26800/prefix/drive_c/Program Files (x86)/Steam");
        let log = steam.join("logs/cloud_log.txt");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(
            &log,
            r#"[time] [AppID 26800] BYieldingAppLaunchIntent returned 1 pending remote operations:
	Operation '3' from machine ''
[time] [AppID 26800] Starting sync (AC Launch,down,)
[time] [AppID 26800] Successfully synced to ChangeNumber 0
[time] [AppID 26800] Running AutoCloud on launch. Looking for existing files to watch:
"#,
        )
        .unwrap();
        assert!(
            inspect_pending_remote_operations(26800, &log)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_new_sync_after_completion_remains_in_progress() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/9/prefix/drive_c/Program Files (x86)/Steam/logs");
        fs::create_dir_all(&steam).unwrap();
        fs::write(
            steam.join("cloud_log.txt"),
            "[AppID 9] Upload complete, result OK\n[AppID 9] Starting sync (down,)\n",
        )
        .unwrap();

        let status = inspect(9, &temp.0).unwrap();
        assert_eq!(status.state, SyncState::Syncing);
    }

    #[test]
    fn autocloud_missing_configuration_is_not_reported_as_syncing() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/105450/prefix/drive_c/Program Files (x86)/Steam/logs");
        fs::create_dir_all(&steam).unwrap();
        fs::write(
            steam.join("cloud_log.txt"),
            "[AppID 105450] Starting sync (init,)\n[AppID 105450] Not running AutoCloud. No config data found\n",
        )
        .unwrap();

        let status = inspect(105450, &temp.0).unwrap();
        assert_eq!(status.state, SyncState::NoEvidence);
        assert!(
            status
                .latest_event
                .as_deref()
                .unwrap()
                .contains("No config data")
        );
    }

    #[test]
    fn transfer_event_must_occur_after_the_checkpoint() {
        let temp = TempDir::new();
        let log = temp.0.join("cloud_log.txt");
        fs::write(&log, "[old] [AppID 42] Upload complete, result OK\n").unwrap();
        let offset = log_offset(&log).unwrap();
        assert!(
            !has_transfer_event_since(&log, offset, 42, TransferEvent::UploadComplete).unwrap()
        );
        let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(file, "[new] [AppID 42] Download complete, result OK").unwrap();
        assert!(
            has_transfer_event_since(&log, offset, 42, TransferEvent::DownloadComplete).unwrap()
        );
        assert!(
            !has_transfer_event_since(&log, offset, 42, TransferEvent::UploadComplete).unwrap()
        );
    }

    #[test]
    fn detects_when_a_synced_cache_is_older_than_the_game_file() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let cache = steam.join("userdata/123/42/remotecache.vdf");
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(
            &cache,
            r#""42" { "slot.sav" { "size" "3" "sha" "abc" "syncstate" "1" } }"#,
        )
        .unwrap();
        fs::create_dir_all(cache.parent().unwrap().join("remote")).unwrap();
        fs::write(cache.parent().unwrap().join("remote/slot.sav"), b"old").unwrap();
        let local = temp
            .0
            .join("games/42/prefix/drive_c/users/player/AppData/Roaming/Game");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("slot.sav"), b"newer save").unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=slot.sav|AppData/Roaming/Game/slot.sav",
        )
        .unwrap();

        let mut status = inspect(42, &temp.0).unwrap();
        compare_profile_files(&mut status, &profile).unwrap();
        assert_eq!(status.state, SyncState::LocalDrift);
        assert_eq!(status.accounts[0].synced_files, 1);
        assert_eq!(status.comparisons[0].state, ComparisonState::LocalDrift);
        assert_eq!(status.comparisons[0].cached_size, Some(3));
        assert_eq!(status.comparisons[0].local_size, Some(10));
        assert_ne!(
            status.comparisons[0].local_sha1,
            status.comparisons[0].cached_sha1
        );
    }

    #[test]
    fn matches_a_steam_install_root_cloud_file_without_a_wine_user() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let cache = steam.join("userdata/123/42/remotecache.vdf");
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        let bytes = b"install-root save";
        let digest = sha1::to_hex(&sha1::digest_bytes(bytes));
        fs::write(
            &cache,
            format!(
                "\"42\" {{ \"Profiles/Player.nfp\" {{ \"size\" \"{}\" \"sha\" \"{digest}\" \"syncstate\" \"1\" }} }}",
                bytes.len()
            ),
        )
        .unwrap();
        let save = steam.join("steamapps/common/Game/Profiles/Player.nfp");
        fs::create_dir_all(save.parent().unwrap()).unwrap();
        fs::write(&save, bytes).unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nsteam_install_dir=Game\nexecutable=game.exe\ncloud_files=Profiles/Player.nfp|steam-install:Profiles/Player.nfp",
        )
        .unwrap();

        let mut status = inspect(42, &temp.0).unwrap();
        compare_profile_files(&mut status, &profile).unwrap();
        assert_eq!(status.comparisons.len(), 1);
        assert_eq!(status.comparisons[0].local_path, save);
        assert_eq!(status.comparisons[0].state, ComparisonState::Match);
    }

    #[test]
    fn matches_a_raw_steam_remote_storage_file() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let account = steam.join("userdata/123/42");
        fs::create_dir_all(account.join("remote")).unwrap();
        let bytes = b"api-backed save";
        let digest = sha1::to_hex(&sha1::digest_bytes(bytes));
        fs::write(
            account.join("remotecache.vdf"),
            format!(
                "\"42\" {{ \"savegame.aws\" {{ \"size\" \"{}\" \"sha\" \"{digest}\" \"syncstate\" \"1\" }} }}",
                bytes.len()
            ),
        )
        .unwrap();
        let remote = account.join("remote/savegame.aws");
        fs::write(&remote, bytes).unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=savegame.aws|steam-remote:savegame.aws",
        )
        .unwrap();

        let mut status = inspect(42, &temp.0).unwrap();
        compare_profile_files(&mut status, &profile).unwrap();
        assert_eq!(status.comparisons.len(), 1);
        assert_eq!(status.comparisons[0].local_path, remote);
        assert_eq!(
            status.comparisons[0].remote_copy_encoding,
            Some(RemoteCopyEncoding::Raw)
        );
        assert_eq!(status.comparisons[0].state, ComparisonState::Match);
    }

    #[test]
    fn detects_equal_size_content_drift() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let cache = steam.join("userdata/123/42/remotecache.vdf");
        fs::create_dir_all(cache.parent().unwrap().join("remote")).unwrap();
        let old_sha = sha1::to_hex(&sha1::digest_bytes(b"old"));
        fs::write(
            &cache,
            format!(
                "\"42\" {{ \"slot.sav\" {{ \"size\" \"3\" \"sha\" \"{old_sha}\" \"syncstate\" \"1\" }} }}"
            ),
        )
        .unwrap();
        fs::write(cache.parent().unwrap().join("remote/slot.sav"), b"old").unwrap();
        let local = temp
            .0
            .join("games/42/prefix/drive_c/users/player/AppData/Roaming/Game");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("slot.sav"), b"new").unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=slot.sav|AppData/Roaming/Game/slot.sav",
        )
        .unwrap();

        let mut status = inspect(42, &temp.0).unwrap();
        compare_profile_files(&mut status, &profile).unwrap();
        assert_eq!(status.comparisons[0].state, ComparisonState::LocalDrift);
        assert_eq!(status.comparisons[0].local_size, Some(3));
        assert_ne!(status.comparisons[0].local_sha1, Some(old_sha));
    }

    #[test]
    fn matches_synced_raw_cache_when_steam_removed_the_staging_copy() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let cache = steam.join("userdata/123/42/remotecache.vdf");
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        let bytes = b"uncompressed save";
        let digest = sha1::to_hex(&sha1::digest_bytes(bytes));
        fs::write(
            &cache,
            format!(
                "\"42\" {{ \"slot.sav\" {{ \"size\" \"{}\" \"sha\" \"{digest}\" \"syncstate\" \"1\" }} }}",
                bytes.len()
            ),
        )
        .unwrap();
        let local = temp
            .0
            .join("games/42/prefix/drive_c/users/player/AppData/Roaming/Game");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("slot.sav"), bytes).unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=slot.sav|AppData/Roaming/Game/slot.sav",
        )
        .unwrap();

        let mut status = inspect(42, &temp.0).unwrap();
        status.state = SyncState::Synced;
        compare_profile_files(&mut status, &profile).unwrap();
        assert_eq!(status.state, SyncState::Synced);
        assert_eq!(status.comparisons[0].state, ComparisonState::Match);
        assert_eq!(status.comparisons[0].remote_copy_size, None);
        assert_eq!(
            status.comparisons[0].cached_sha1.as_deref(),
            Some(digest.as_str())
        );
        assert_eq!(
            status.comparisons[0].local_sha1.as_deref(),
            Some(digest.as_str())
        );
    }

    #[test]
    fn matches_a_steam_zlib_remote_copy_to_its_uncompressed_game_file() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let cache = steam.join("userdata/123/42/remotecache.vdf");
        fs::create_dir_all(cache.parent().unwrap().join("remote")).unwrap();

        let local_bytes = b"a save payload that Steam stores compressed";
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(local_bytes).unwrap();
        let mut remote_bytes = (local_bytes.len() as u32).to_le_bytes().to_vec();
        remote_bytes.extend(encoder.finish().unwrap());
        let remote_sha = sha1::to_hex(&sha1::digest_bytes(&remote_bytes));
        fs::write(
            &cache,
            format!(
                "\"42\" {{ \"slot.sav\" {{ \"size\" \"{}\" \"sha\" \"{remote_sha}\" \"syncstate\" \"1\" }} }}",
                remote_bytes.len()
            ),
        )
        .unwrap();
        fs::write(
            cache.parent().unwrap().join("remote/slot.sav"),
            remote_bytes,
        )
        .unwrap();

        let local = temp
            .0
            .join("games/42/prefix/drive_c/users/player/AppData/Roaming/Game");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("slot.sav"), local_bytes).unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=slot.sav|AppData/Roaming/Game/slot.sav",
        )
        .unwrap();

        let mut status = inspect(42, &temp.0).unwrap();
        compare_profile_files(&mut status, &profile).unwrap();
        let comparison = &status.comparisons[0];
        assert_eq!(comparison.state, ComparisonState::Match);
        assert_eq!(
            comparison.remote_copy_encoding,
            Some(RemoteCopyEncoding::SteamZlib)
        );
        assert_eq!(
            comparison.remote_content_size,
            Some(local_bytes.len() as u64)
        );
        assert_eq!(comparison.remote_content_sha1, comparison.local_sha1);
    }

    #[test]
    fn still_matches_an_uncompressed_remote_copy() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let cache = steam.join("userdata/123/42/remotecache.vdf");
        fs::create_dir_all(cache.parent().unwrap().join("remote")).unwrap();
        let bytes = b"raw save";
        let digest = sha1::to_hex(&sha1::digest_bytes(bytes));
        fs::write(
            &cache,
            format!(
                "\"42\" {{ \"slot.sav\" {{ \"size\" \"{}\" \"sha\" \"{digest}\" \"syncstate\" \"1\" }} }}",
                bytes.len()
            ),
        )
        .unwrap();
        fs::write(cache.parent().unwrap().join("remote/slot.sav"), bytes).unwrap();
        let local = temp
            .0
            .join("games/42/prefix/drive_c/users/player/AppData/Roaming/Game");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("slot.sav"), bytes).unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=slot.sav|AppData/Roaming/Game/slot.sav",
        )
        .unwrap();

        let mut status = inspect(42, &temp.0).unwrap();
        compare_profile_files(&mut status, &profile).unwrap();
        assert_eq!(status.comparisons[0].state, ComparisonState::Match);
        assert_eq!(
            status.comparisons[0].remote_copy_encoding,
            Some(RemoteCopyEncoding::Raw)
        );
    }

    #[test]
    fn a_corrupt_steam_zlib_candidate_cannot_match_local_content() {
        let temp = TempDir::new();
        let steam = temp
            .0
            .join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        let cache = steam.join("userdata/123/42/remotecache.vdf");
        fs::create_dir_all(cache.parent().unwrap().join("remote")).unwrap();
        let remote_bytes = [3, 0, 0, 0, 0x78, 0x9c, 0xff];
        let digest = sha1::to_hex(&sha1::digest_bytes(&remote_bytes));
        fs::write(
            &cache,
            format!(
                "\"42\" {{ \"slot.sav\" {{ \"size\" \"{}\" \"sha\" \"{digest}\" \"syncstate\" \"1\" }} }}",
                remote_bytes.len()
            ),
        )
        .unwrap();
        fs::write(
            cache.parent().unwrap().join("remote/slot.sav"),
            remote_bytes,
        )
        .unwrap();
        let local = temp
            .0
            .join("games/42/prefix/drive_c/users/player/AppData/Roaming/Game");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join("slot.sav"), b"abc").unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\ncloud_files=slot.sav|AppData/Roaming/Game/slot.sav",
        )
        .unwrap();

        let mut status = inspect(42, &temp.0).unwrap();
        compare_profile_files(&mut status, &profile).unwrap();
        assert_eq!(status.comparisons[0].state, ComparisonState::LocalDrift);
    }
}
