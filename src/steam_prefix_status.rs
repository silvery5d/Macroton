use crate::keyvalues;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const BOOTSTRAP_MARKER: &str = ".macroton-steam-bootstrap.json";
const RUNTIME_IDENTITY: &str = ".macroton-runtime-identity";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    NotBootstrapped,
    SignInRequired,
    StoredLoginPresent,
}

impl Readiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotBootstrapped => "not-bootstrapped",
            Self::SignInRequired => "sign-in-required",
            Self::StoredLoginPresent => "stored-login-present",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixStatus {
    pub app_id: u32,
    pub prefix: PathBuf,
    pub steam_root: PathBuf,
    pub client_installed: bool,
    pub bootstrap_marker_present: bool,
    pub bootstrap_recorded: bool,
    pub bootstrap_detail: String,
    pub runtime_identity_recorded: bool,
    pub stored_login_records: usize,
    pub userdata_directories: usize,
    pub readiness: Readiness,
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

pub fn inspect(app_id: u32, data_root: &Path) -> Result<PrefixStatus, Error> {
    let prefix = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix");
    inspect_prefix(app_id, &prefix)
}

pub fn inspect_prefix(app_id: u32, prefix: &Path) -> Result<PrefixStatus, Error> {
    let steam_root = prefix.join("drive_c/Program Files (x86)/Steam");
    let client_installed = steam_root.join("Steam.exe").is_file();
    let marker = prefix.join(BOOTSTRAP_MARKER);
    let bootstrap_marker_present = marker.is_file();
    let runtime_identity_recorded = prefix.join(RUNTIME_IDENTITY).is_file();
    let (bootstrap_recorded, bootstrap_detail, _, _) =
        validate_bootstrap(app_id, prefix, &marker, client_installed)?;
    let stored_login_records = count_stored_logins(&steam_root.join("config/loginusers.vdf"))?;
    let userdata_directories = count_userdata_directories(&steam_root.join("userdata"))?;
    let readiness = if !client_installed || !bootstrap_recorded {
        Readiness::NotBootstrapped
    } else if stored_login_records == 0 {
        Readiness::SignInRequired
    } else {
        Readiness::StoredLoginPresent
    };

    Ok(PrefixStatus {
        app_id,
        prefix: prefix.to_path_buf(),
        steam_root,
        client_installed,
        bootstrap_marker_present,
        bootstrap_recorded,
        bootstrap_detail,
        runtime_identity_recorded,
        stored_login_records,
        userdata_directories,
        readiness,
    })
}

fn validate_bootstrap(
    app_id: u32,
    prefix: &Path,
    marker: &Path,
    client_installed: bool,
) -> Result<(bool, String, Option<String>, Option<String>), Error> {
    if !client_installed {
        return Ok((false, "Steam.exe is missing".to_owned(), None, None));
    }
    let contents = match fs::read_to_string(marker) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((false, "bootstrap marker is missing".to_owned(), None, None));
        }
        Err(error) => {
            return Err(Error {
                message: format!("cannot read bootstrap marker {}: {error}", marker.display()),
            });
        }
    };
    let invalid = |detail: &str| Ok((false, detail.to_owned(), None, None));
    let Some(fields) = parse_flat_json_object(&contents) else {
        return invalid("bootstrap marker is not canonical flat JSON");
    };
    let expected_keys = [
        "schema",
        "appid",
        "runtime_id",
        "runtime_kind",
        "runtime_fingerprint_sha256",
        "installer_sha256",
    ];
    if fields.len() != expected_keys.len()
        || expected_keys.iter().any(|key| !fields.contains_key(*key))
    {
        return invalid("bootstrap marker fields are incomplete or unknown");
    }
    if fields.get("schema").map(String::as_str) != Some("1") {
        return invalid("bootstrap schema is missing or unsupported");
    }
    if fields
        .get("appid")
        .and_then(|value| value.parse::<u32>().ok())
        != Some(app_id)
    {
        return invalid("bootstrap AppID does not match the prefix");
    }
    let Some(runtime_id) = fields.get("runtime_id").map(String::as_str) else {
        return invalid("bootstrap runtime_id is missing");
    };
    let Some(runtime_kind) = fields.get("runtime_kind").map(String::as_str) else {
        return invalid("bootstrap runtime_kind is missing");
    };
    if !matches!(runtime_kind, "managed-archive" | "external-executable") {
        return invalid("bootstrap runtime_kind is unsupported");
    }
    let Some(runtime_fingerprint) = fields.get("runtime_fingerprint_sha256").map(String::as_str)
    else {
        return invalid("bootstrap runtime fingerprint is missing");
    };
    if !is_sha256(runtime_fingerprint) {
        return invalid("bootstrap runtime fingerprint is invalid");
    }
    let Some(installer_fingerprint) = fields.get("installer_sha256").map(String::as_str) else {
        return invalid("bootstrap installer fingerprint is missing");
    };
    if !is_sha256(installer_fingerprint) {
        return invalid("bootstrap installer fingerprint is invalid");
    }
    let identity_path = prefix.join(RUNTIME_IDENTITY);
    let identity = match fs::read_to_string(&identity_path) {
        Ok(identity) => identity,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return invalid("runtime identity is missing");
        }
        Err(error) => {
            return Err(Error {
                message: format!(
                    "cannot read runtime identity {}: {error}",
                    identity_path.display()
                ),
            });
        }
    };
    if identity_value(&identity, "schema") != Some("1")
        || identity_value(&identity, "runtime_id") != Some(runtime_id)
        || identity_value(&identity, "runtime_fingerprint_sha256") != Some(runtime_fingerprint)
    {
        return invalid("runtime identity does not match bootstrap provenance");
    }
    Ok((
        true,
        "installer and runtime fingerprints match the prefix identity".to_owned(),
        Some(runtime_id.to_owned()),
        Some(runtime_fingerprint.to_owned()),
    ))
}

fn parse_flat_json_object(contents: &str) -> Option<BTreeMap<String, String>> {
    let mut lines = contents.lines();
    if lines.next()?.trim() != "{" {
        return None;
    }
    let mut fields = BTreeMap::new();
    let mut closed = false;
    for line in lines {
        let line = line.trim();
        if line == "}" {
            if closed {
                return None;
            }
            closed = true;
            continue;
        }
        if closed || line.is_empty() {
            return None;
        }
        let line = line.strip_suffix(',').unwrap_or(line);
        let (key, value) = line.split_once(':')?;
        let key = key.trim().strip_prefix('"')?.strip_suffix('"')?;
        if key.is_empty() || key.contains(['"', '\\']) {
            return None;
        }
        let value = value.trim();
        let value = if value.starts_with('"') {
            value.strip_prefix('"')?.strip_suffix('"')?.to_owned()
        } else if value.bytes().all(|byte| byte.is_ascii_digit()) && !value.is_empty() {
            value.to_owned()
        } else {
            return None;
        };
        if value.contains(['"', '\\']) || fields.insert(key.to_owned(), value).is_some() {
            return None;
        }
    }
    closed.then_some(fields)
}

pub fn validate_expected_runtime(
    app_id: u32,
    prefix: &Path,
    expected_runtime_id: &str,
    expected_runtime_fingerprint: &str,
) -> Result<(), Error> {
    let marker = prefix.join(BOOTSTRAP_MARKER);
    let steam = prefix.join("drive_c/Program Files (x86)/Steam/Steam.exe");
    let (valid, detail, runtime_id, runtime_fingerprint) =
        validate_bootstrap(app_id, prefix, &marker, steam.is_file())?;
    if !valid {
        return Err(Error { message: detail });
    }
    if runtime_id.as_deref() != Some(expected_runtime_id)
        || runtime_fingerprint.as_deref() != Some(expected_runtime_fingerprint)
    {
        return Err(Error {
            message: "bootstrap runtime provenance does not match the current launch plan"
                .to_owned(),
        });
    }
    Ok(())
}

fn identity_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn count_stored_logins(path: &Path) -> Result<usize, Error> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(Error {
                message: format!(
                    "cannot read Steam stored-login metadata {}: {error}",
                    path.display()
                ),
            });
        }
    };
    let parsed = keyvalues::parse(&contents).map_err(|error| Error {
        message: format!(
            "cannot parse Steam stored-login metadata {}: {error}",
            path.display()
        ),
    })?;
    let users = parsed.get("users").ok_or_else(|| Error {
        message: format!(
            "Steam stored-login metadata has no users object: {}",
            path.display()
        ),
    })?;
    Ok(users
        .entries()
        .unwrap_or_default()
        .iter()
        .filter(|(key, value)| {
            key.bytes().all(|byte| byte.is_ascii_digit()) && value.entries().is_some()
        })
        .count())
}

fn count_userdata_directories(path: &Path) -> Result<usize, Error> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(Error {
                message: format!("cannot inspect Steam userdata {}: {error}", path.display()),
            });
        }
    };
    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(|error| Error {
            message: format!("cannot inspect Steam userdata {}: {error}", path.display()),
        })?;
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
            && entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|byte| byte.is_ascii_digit())
        {
            count += 1;
        }
    }
    Ok(count)
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

    fn write_valid_provenance(prefix: &Path, app_id: u32) {
        let fingerprint = "ab".repeat(32);
        let installer = "cd".repeat(32);
        fs::write(
            prefix.join(BOOTSTRAP_MARKER),
            format!(
                "{{\n  \"schema\": 1,\n  \"appid\": {app_id},\n  \"runtime_id\": \"wine-test\",\n  \"runtime_kind\": \"managed-archive\",\n  \"runtime_fingerprint_sha256\": \"{fingerprint}\",\n  \"installer_sha256\": \"{installer}\"\n}}\n"
            ),
        )
        .unwrap();
        fs::write(
            prefix.join(RUNTIME_IDENTITY),
            format!(
                "schema=1\nruntime_id=wine-test\nruntime_fingerprint_sha256={fingerprint}\ncomponents=\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn reports_a_fresh_bootstrap_as_requiring_sign_in() {
        let prefix = temporary_root("prefix-status-fresh");
        let steam = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&steam).unwrap();
        fs::write(steam.join("Steam.exe"), b"MZ").unwrap();
        write_valid_provenance(&prefix, 42);

        let status = inspect_prefix(42, &prefix).unwrap();
        assert_eq!(status.readiness, Readiness::SignInRequired);
        assert_eq!(status.stored_login_records, 0);
        assert_eq!(status.userdata_directories, 0);
        fs::remove_dir_all(prefix).unwrap();
    }

    #[test]
    fn counts_records_without_exposing_account_identifiers() {
        let prefix = temporary_root("prefix-status-login");
        let steam = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(steam.join("config")).unwrap();
        fs::create_dir_all(steam.join("userdata/12345")).unwrap();
        fs::create_dir_all(steam.join("userdata/not-an-account")).unwrap();
        fs::write(steam.join("Steam.exe"), b"MZ").unwrap();
        write_valid_provenance(&prefix, 42);
        fs::write(
            steam.join("config/loginusers.vdf"),
            r#""users" { "12345" { "AccountName" "private" } }"#,
        )
        .unwrap();

        let status = inspect_prefix(42, &prefix).unwrap();
        assert_eq!(status.readiness, Readiness::StoredLoginPresent);
        assert_eq!(status.stored_login_records, 1);
        assert_eq!(status.userdata_directories, 1);
        fs::remove_dir_all(prefix).unwrap();
    }

    #[test]
    fn does_not_treat_an_unmanaged_client_as_bootstrapped() {
        let prefix = temporary_root("prefix-status-unmanaged");
        let steam = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&steam).unwrap();
        fs::write(steam.join("Steam.exe"), b"MZ").unwrap();

        let status = inspect_prefix(42, &prefix).unwrap();
        assert_eq!(status.readiness, Readiness::NotBootstrapped);
        fs::remove_dir_all(prefix).unwrap();
    }

    #[test]
    fn rejects_a_present_but_forged_or_mismatched_bootstrap_marker() {
        let prefix = temporary_root("prefix-status-forged");
        let steam = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&steam).unwrap();
        fs::write(steam.join("Steam.exe"), b"MZ").unwrap();
        fs::write(prefix.join(BOOTSTRAP_MARKER), b"{}").unwrap();

        let status = inspect_prefix(42, &prefix).unwrap();
        assert!(status.bootstrap_marker_present);
        assert!(!status.bootstrap_recorded);
        assert_eq!(status.readiness, Readiness::NotBootstrapped);
        assert!(status.bootstrap_detail.contains("canonical flat JSON"));
        fs::remove_dir_all(prefix).unwrap();
    }

    #[test]
    fn expected_runtime_must_match_both_bootstrap_and_identity() {
        let prefix = temporary_root("prefix-status-runtime-drift");
        let steam = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&steam).unwrap();
        fs::write(steam.join("Steam.exe"), b"MZ").unwrap();
        write_valid_provenance(&prefix, 42);

        validate_expected_runtime(42, &prefix, "wine-test", &"ab".repeat(32)).unwrap();
        let error =
            validate_expected_runtime(42, &prefix, "wine-new", &"ab".repeat(32)).unwrap_err();
        assert!(error.message.contains("current launch plan"));
        fs::remove_dir_all(prefix).unwrap();
    }

    #[test]
    fn flat_marker_parser_rejects_duplicate_and_unknown_fields() {
        assert!(parse_flat_json_object("{\n\"schema\": 1,\n\"schema\": 1\n}").is_none());
        let prefix = temporary_root("prefix-status-unknown-field");
        let steam = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&steam).unwrap();
        fs::write(steam.join("Steam.exe"), b"MZ").unwrap();
        write_valid_provenance(&prefix, 42);
        let marker = prefix.join(BOOTSTRAP_MARKER);
        let contents = fs::read_to_string(&marker).unwrap();
        fs::write(
            &marker,
            contents.replacen("{\n", "{\n  \"extra\": \"x\",\n", 1),
        )
        .unwrap();
        let status = inspect_prefix(42, &prefix).unwrap();
        assert!(!status.bootstrap_recorded);
        assert!(status.bootstrap_detail.contains("unknown"));
        fs::remove_dir_all(prefix).unwrap();
    }
}
