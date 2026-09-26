use crate::binary::{self, PeArchitecture};
use crate::execute;
use crate::profile::{CompatibilityProfile, LaunchMode, renderer_environment};
use crate::runtime::{self, RuntimeKind};
use crate::sha256;
use crate::steam_prefix_status;
use crate::stop;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const MARKER_FILE: &str = ".macroton-steam-bootstrap.json";
const STEAM_OVERLAY_DLL_OVERRIDES: &str = "gameoverlayrenderer=d;gameoverlayrenderer64=d";

#[derive(Debug, Clone)]
pub struct Options<'a> {
    pub app_id: u32,
    pub profile: &'a CompatibilityProfile,
    pub installer: &'a Path,
    pub installer_sha256: &'a str,
    pub runtime_root: &'a Path,
    pub data_root: &'a Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    Installed,
    AlreadyInstalled,
    RepairedProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub kind: OutcomeKind,
    pub runtime_id: String,
    pub runtime_kind: RuntimeKind,
    pub runtime_fingerprint_sha256: String,
    pub installer_sha256: String,
    pub prefix: PathBuf,
    pub steam_executable: PathBuf,
    pub marker: PathBuf,
}

pub struct RebindOptions<'a> {
    pub app_id: u32,
    pub profile: &'a CompatibilityProfile,
    pub expected_old_fingerprint: &'a str,
    pub runtime_root: &'a Path,
    pub data_root: &'a Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebindOutcome {
    pub old_runtime_id: String,
    pub old_runtime_fingerprint_sha256: String,
    pub runtime_id: String,
    pub runtime_kind: RuntimeKind,
    pub runtime_fingerprint_sha256: String,
    pub installer_sha256: String,
    pub prefix: PathBuf,
    pub marker: PathBuf,
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

pub fn bootstrap(options: &Options<'_>) -> Result<Outcome, Error> {
    validate_options(options)?;
    let expected_installer_sha256 = options.installer_sha256.to_ascii_lowercase();
    let actual_installer_sha256 = sha256::to_hex(&sha256::digest_file(options.installer).map_err(
        |error| Error {
            message: format!(
                "cannot hash Steam installer {}: {error}",
                options.installer.display()
            ),
        },
    )?);
    if actual_installer_sha256 != expected_installer_sha256 {
        return Err(Error {
            message: format!(
                "Steam installer checksum mismatch: expected {expected_installer_sha256}, got {actual_installer_sha256}"
            ),
        });
    }
    let installer_architecture =
        binary::inspect_pe_architecture(options.installer).map_err(|error| Error {
            message: format!("cannot inspect Steam installer PE header: {error}"),
        })?;
    if !matches!(
        installer_architecture,
        Some(PeArchitecture::I386 | PeArchitecture::X86_64)
    ) {
        return Err(Error {
            message: format!(
                "Steam installer is not a Windows PE executable: {}",
                options.installer.display()
            ),
        });
    }

    let runtime_id = format!(
        "{}-{}",
        options.profile.runtime, options.profile.runtime_version
    );
    let installed =
        runtime::resolve_installed(options.runtime_root, &runtime_id).map_err(|error| Error {
            message: error.to_string(),
        })?;
    let prefix = options
        .data_root
        .join("games")
        .join(options.app_id.to_string())
        .join("prefix");
    let steam_executable = prefix
        .join("drive_c")
        .join("Program Files (x86)")
        .join("Steam")
        .join("Steam.exe");
    let marker = prefix.join(MARKER_FILE);
    let marker_contents = marker_contents(
        options.app_id,
        &runtime_id,
        installed.kind,
        &installed.fingerprint_sha256,
        &expected_installer_sha256,
    );

    if prefix.exists() {
        match (steam_executable.is_file(), fs::read_to_string(&marker)) {
            (true, Ok(existing)) if existing == marker_contents => {
                ensure_runtime_identity(&prefix, &installed, &runtime_id)?;
                return Ok(Outcome {
                    kind: OutcomeKind::AlreadyInstalled,
                    runtime_id,
                    runtime_kind: installed.kind,
                    runtime_fingerprint_sha256: installed.fingerprint_sha256,
                    installer_sha256: expected_installer_sha256,
                    prefix,
                    steam_executable,
                    marker,
                });
            }
            (true, Ok(existing))
                if repairable_stale_marker(
                    options,
                    &prefix,
                    &existing,
                    &runtime_id,
                    &installed,
                    &expected_installer_sha256,
                )? =>
            {
                write_marker(&marker, &marker_contents)?;
                return Ok(Outcome {
                    kind: OutcomeKind::RepairedProvenance,
                    runtime_id,
                    runtime_kind: installed.kind,
                    runtime_fingerprint_sha256: installed.fingerprint_sha256,
                    installer_sha256: expected_installer_sha256,
                    prefix,
                    steam_executable,
                    marker,
                });
            }
            (false, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => {
                return Err(Error {
                    message: format!(
                        "refusing to modify an existing Steam prefix without a matching Macroton bootstrap record: {}",
                        prefix.display()
                    ),
                });
            }
        }
    }

    fs::create_dir_all(&prefix).map_err(|error| Error {
        message: format!(
            "cannot create isolated prefix {}: {error}",
            prefix.display()
        ),
    })?;
    // Write runtime ownership before invoking the installer. Even if setup
    // fails or leaves Steam running, `macroton stop` can still resolve the
    // exact wineserver for this one prefix.
    ensure_runtime_identity(&prefix, &installed, &runtime_id)?;
    // Install the same fail-closed post-mortem debugger policy used by normal
    // launches before executing the installer. Setup failures are otherwise
    // able to recursively spawn winedbg processes and exhaust host resources.
    execute::ensure_crash_debugger_guard(
        &prefix,
        &installed.entrypoint,
        &installed.fingerprint_sha256,
    )
    .map_err(|error| Error {
        message: format!("cannot configure bootstrap crash guard: {error}"),
    })?;
    let mut command = Command::new(&installed.entrypoint);
    command
        .arg(options.installer)
        .arg("/S")
        .envs(&options.profile.environment)
        .env("WINEPREFIX", &prefix)
        .env("WINEDEBUG", "-all")
        .env(
            "WINEDLLOVERRIDES",
            bootstrap_dll_overrides(options.profile.environment.get("WINEDLLOVERRIDES")),
        )
        .env("MVK_CONFIG_LOG_LEVEL", "0");
    if let Some((name, value)) = renderer_environment(&options.profile.renderer) {
        command.env(name, value);
    }
    let status = command.status().map_err(|error| Error {
        message: format!(
            "cannot start Steam installer with managed runtime {}: {error}",
            installed.entrypoint.display()
        ),
    })?;
    if !status.success() {
        return Err(Error {
            message: format!("Steam installer exited with {status}"),
        });
    }
    if !steam_executable.is_file() {
        return Err(Error {
            message: format!(
                "Steam installer reported success but Steam.exe was not created at {}",
                steam_executable.display()
            ),
        });
    }
    write_marker(&marker, &marker_contents)?;

    Ok(Outcome {
        kind: OutcomeKind::Installed,
        runtime_id,
        runtime_kind: installed.kind,
        runtime_fingerprint_sha256: installed.fingerprint_sha256,
        installer_sha256: expected_installer_sha256,
        prefix,
        steam_executable,
        marker,
    })
}

/// Explicitly moves an existing, authenticated Steam prefix to another
/// checksum-verified runtime without copying or rewriting Steam account data.
/// The caller must name the exact old fingerprint, and the prefix must be idle.
pub fn rebind_runtime(options: &RebindOptions<'_>) -> Result<RebindOutcome, Error> {
    if options.app_id != options.profile.app_id {
        return Err(Error {
            message: format!(
                "profile AppID {} does not match requested AppID {}",
                options.profile.app_id, options.app_id
            ),
        });
    }
    if options.profile.launch_mode != LaunchMode::Steam {
        return Err(Error {
            message: "Steam runtime rebinding requires a profile with launch=steam".to_owned(),
        });
    }
    if options.expected_old_fingerprint.len() != 64
        || !options
            .expected_old_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error {
            message: "--from-runtime-fingerprint must contain exactly 64 hexadecimal digits"
                .to_owned(),
        });
    }

    let prefix = options
        .data_root
        .join("games")
        .join(options.app_id.to_string())
        .join("prefix");
    let steam_executable = prefix.join("drive_c/Program Files (x86)/Steam/Steam.exe");
    if !steam_executable.is_file() {
        return Err(Error {
            message: format!(
                "cannot rebind a prefix without Windows Steam: {}",
                prefix.display()
            ),
        });
    }
    let marker = prefix.join(MARKER_FILE);
    let old_marker = fs::read_to_string(&marker).map_err(|error| Error {
        message: format!("cannot read bootstrap record {}: {error}", marker.display()),
    })?;
    let Some((old_kind, old_fingerprint)) = canonical_marker_runtime(&old_marker) else {
        return Err(Error {
            message: "existing Steam bootstrap record is not canonical".to_owned(),
        });
    };
    let old_runtime_id = marker_quoted_value(&old_marker, "runtime_id", true)
        .ok_or_else(|| Error {
            message: "existing Steam bootstrap record has no canonical runtime_id".to_owned(),
        })?
        .to_owned();
    let installer_sha256 = marker_quoted_value(&old_marker, "installer_sha256", false)
        .ok_or_else(|| Error {
            message: "existing Steam bootstrap record has no canonical installer_sha256".to_owned(),
        })?
        .to_owned();
    if old_marker
        != marker_contents(
            options.app_id,
            &old_runtime_id,
            old_kind,
            &old_fingerprint,
            &installer_sha256,
        )
    {
        return Err(Error {
            message: "existing Steam bootstrap record contains mismatched provenance".to_owned(),
        });
    }
    if old_fingerprint != options.expected_old_fingerprint.to_ascii_lowercase() {
        return Err(Error {
            message: format!(
                "prefix runtime fingerprint changed: expected {}, found {}",
                options.expected_old_fingerprint, old_fingerprint
            ),
        });
    }
    let old_identity =
        fs::read_to_string(prefix.join(".macroton-runtime-identity")).map_err(|error| Error {
            message: format!("cannot read existing prefix runtime identity: {error}"),
        })?;
    if !canonical_runtime_identity(&old_identity, &old_runtime_id, &old_fingerprint) {
        return Err(Error {
            message: "existing prefix runtime identity does not match bootstrap provenance"
                .to_owned(),
        });
    }
    let prefix_status =
        steam_prefix_status::inspect_prefix(options.app_id, &prefix).map_err(|error| Error {
            message: format!("cannot validate existing Steam prefix: {error}"),
        })?;
    if !prefix_status.bootstrap_recorded {
        return Err(Error {
            message: format!(
                "existing Steam prefix provenance is invalid: {}",
                prefix_status.bootstrap_detail
            ),
        });
    }
    let resources =
        stop::inspect_resources(options.app_id, options.data_root).map_err(|error| Error {
            message: format!("cannot audit Steam prefix resources before rebinding: {error}"),
        })?;
    if !resources.pids.is_empty() {
        return Err(Error {
            message: format!(
                "refusing to rebind an active Steam prefix; stop AppID {} first",
                options.app_id
            ),
        });
    }

    let runtime_id = format!(
        "{}-{}",
        options.profile.runtime, options.profile.runtime_version
    );
    let installed =
        runtime::resolve_installed(options.runtime_root, &runtime_id).map_err(|error| Error {
            message: error.to_string(),
        })?;
    if runtime_id == old_runtime_id && installed.fingerprint_sha256 == old_fingerprint {
        return Err(Error {
            message: "requested runtime is already bound to this prefix".to_owned(),
        });
    }

    ensure_runtime_identity(&prefix, &installed, &runtime_id)?;
    let new_marker = marker_contents(
        options.app_id,
        &runtime_id,
        installed.kind,
        &installed.fingerprint_sha256,
        &installer_sha256,
    );
    if let Err(error) = write_marker(&marker, &new_marker) {
        let identity_path = prefix.join(".macroton-runtime-identity");
        let rollback = fs::write(&identity_path, &old_identity);
        return Err(Error {
            message: match rollback {
                Ok(()) => format!("{error}; restored previous runtime identity"),
                Err(rollback_error) => format!(
                    "{error}; failed to restore previous runtime identity: {rollback_error}"
                ),
            },
        });
    }

    Ok(RebindOutcome {
        old_runtime_id,
        old_runtime_fingerprint_sha256: old_fingerprint,
        runtime_id,
        runtime_kind: installed.kind,
        runtime_fingerprint_sha256: installed.fingerprint_sha256,
        installer_sha256,
        prefix,
        marker,
    })
}

fn repairable_stale_marker(
    options: &Options<'_>,
    prefix: &Path,
    existing_marker: &str,
    runtime_id: &str,
    installed: &runtime::InstalledRuntime,
    installer_sha256: &str,
) -> Result<bool, Error> {
    let Some((old_kind, old_fingerprint)) = canonical_marker_runtime(existing_marker) else {
        return Ok(false);
    };
    if old_fingerprint == installed.fingerprint_sha256
        || existing_marker
            != marker_contents(
                options.app_id,
                runtime_id,
                old_kind,
                &old_fingerprint,
                installer_sha256,
            )
    {
        return Ok(false);
    }

    let desired_identity = format!(
        "schema=1\nruntime_id={}\nruntime_fingerprint_sha256={}\ncomponents={}\n",
        runtime_id,
        installed.fingerprint_sha256,
        installed.components.join(",")
    );
    let identity_path = prefix.join(".macroton-runtime-identity");
    let current_identity = fs::read_to_string(&identity_path).ok();
    let identity_is_desired = current_identity.as_deref() == Some(desired_identity.as_str());
    let identity_matches_old_marker = current_identity
        .as_deref()
        .is_some_and(|identity| canonical_runtime_identity(identity, runtime_id, &old_fingerprint));
    if !identity_is_desired && !identity_matches_old_marker {
        return Ok(false);
    }

    let status =
        steam_prefix_status::inspect_prefix(options.app_id, prefix).map_err(|error| Error {
            message: format!("cannot audit stale Steam bootstrap record: {error}"),
        })?;
    if status.stored_login_records != 0 || status.userdata_directories != 0 {
        return Ok(false);
    }
    let resources =
        stop::inspect_resources(options.app_id, options.data_root).map_err(|error| Error {
            message: format!("cannot audit stale Steam prefix resources: {error}"),
        })?;
    if !resources.pids.is_empty() {
        return Ok(false);
    }
    if !identity_is_desired {
        ensure_runtime_identity(prefix, installed, runtime_id)?;
    }
    Ok(true)
}

fn canonical_runtime_identity(contents: &str, runtime_id: &str, fingerprint: &str) -> bool {
    let lines = contents.lines().collect::<Vec<_>>();
    lines.len() == 4
        && lines[0] == "schema=1"
        && lines[1] == format!("runtime_id={runtime_id}")
        && lines[2] == format!("runtime_fingerprint_sha256={fingerprint}")
        && lines[3]
            .strip_prefix("components=")
            .is_some_and(|components| {
                components.split(',').all(|component| {
                    component.is_empty()
                        || component.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                        })
                })
            })
}

fn canonical_marker_runtime(contents: &str) -> Option<(RuntimeKind, String)> {
    let kind = match marker_quoted_value(contents, "runtime_kind", true)? {
        "managed-archive" => RuntimeKind::ManagedArchive,
        "external-executable" => RuntimeKind::ExternalExecutable,
        _ => return None,
    };
    let fingerprint = marker_quoted_value(contents, "runtime_fingerprint_sha256", true)?;
    if fingerprint.len() != 64 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some((kind, fingerprint.to_ascii_lowercase()))
}

fn marker_quoted_value<'a>(contents: &'a str, key: &str, comma: bool) -> Option<&'a str> {
    let prefix = format!("  \"{key}\": \"");
    let suffix = if comma { "\"," } else { "\"" };
    contents
        .lines()
        .find(|line| line.starts_with(&prefix))?
        .strip_prefix(&prefix)?
        .strip_suffix(suffix)
}

fn ensure_runtime_identity(
    prefix: &Path,
    installed: &runtime::InstalledRuntime,
    runtime_id: &str,
) -> Result<(), Error> {
    execute::ensure_prefix_runtime_identity(
        prefix,
        &installed.entrypoint,
        runtime_id,
        &installed.fingerprint_sha256,
        &installed.components,
    )
    .map_err(|error| Error {
        message: format!("cannot record bootstrap runtime ownership: {error}"),
    })
}

fn bootstrap_dll_overrides(configured: Option<&String>) -> String {
    let baseline = "mscoree,mshtml=";
    match configured
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        Some(configured) => {
            format!("{baseline};{configured};{STEAM_OVERLAY_DLL_OVERRIDES}")
        }
        None => format!("{baseline};{STEAM_OVERLAY_DLL_OVERRIDES}"),
    }
}

fn validate_options(options: &Options<'_>) -> Result<(), Error> {
    if options.app_id != options.profile.app_id {
        return Err(Error {
            message: format!(
                "profile AppID {} does not match requested AppID {}",
                options.profile.app_id, options.app_id
            ),
        });
    }
    if options.profile.launch_mode != LaunchMode::Steam {
        return Err(Error {
            message: "Steam bootstrap requires a profile with launch=steam".to_owned(),
        });
    }
    if options.installer_sha256.len() != 64
        || !options
            .installer_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error {
            message: "--installer-sha256 must contain exactly 64 hexadecimal digits".to_owned(),
        });
    }
    Ok(())
}

fn marker_contents(
    app_id: u32,
    runtime_id: &str,
    runtime_kind: RuntimeKind,
    runtime_fingerprint_sha256: &str,
    installer_sha256: &str,
) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"schema\": 1,\n",
            "  \"appid\": {},\n",
            "  \"runtime_id\": \"{}\",\n",
            "  \"runtime_kind\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"installer_sha256\": \"{}\"\n",
            "}}\n"
        ),
        app_id,
        json_escape(runtime_id),
        runtime_kind.as_str(),
        runtime_fingerprint_sha256,
        installer_sha256
    )
}

fn write_marker(marker: &Path, contents: &str) -> Result<(), Error> {
    let temporary = marker.with_extension(format!("tmp-{}", unix_ms()));
    fs::write(&temporary, contents).map_err(|error| Error {
        message: format!(
            "cannot write bootstrap record {}: {error}",
            temporary.display()
        ),
    })?;
    fs::rename(&temporary, marker).map_err(|error| Error {
        message: format!(
            "cannot commit bootstrap record {}: {error}",
            marker.display()
        ),
    })
}

fn json_escape(input: &str) -> String {
    input
        .chars()
        .flat_map(|character| match character {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect(),
            character => vec![character],
        })
        .collect()
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::register_external;
    use std::os::unix::fs::PermissionsExt;

    fn profile() -> CompatibilityProfile {
        CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=test-wine\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Game\nexecutable=game.exe\n",
        )
        .unwrap()
    }

    fn write_i386_pe(path: &Path) {
        let mut bytes = vec![0_u8; 0x86];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x014c_u16.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn installs_and_then_reports_idempotent_result() {
        let root = std::env::temp_dir().join(format!("macroton-bootstrap-test-{}", unix_ms()));
        let runtime_root = root.join("runtimes");
        let data_root = root.join("data");
        let fake_runtime = root.join("fake-wine");
        let installer = root.join("SteamSetup.exe");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &fake_runtime,
            "#!/bin/sh\nmkdir -p \"$WINEPREFIX/drive_c/Program Files (x86)/Steam\"\nprintf MZ > \"$WINEPREFIX/drive_c/Program Files (x86)/Steam/Steam.exe\"\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake_runtime).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_runtime, permissions).unwrap();
        write_i386_pe(&installer);
        register_external("test-wine-1", &fake_runtime, &runtime_root).unwrap();
        let digest = sha256::to_hex(&sha256::digest_file(&installer).unwrap());
        let profile = profile();
        let options = Options {
            app_id: 42,
            profile: &profile,
            installer: &installer,
            installer_sha256: &digest,
            runtime_root: &runtime_root,
            data_root: &data_root,
        };

        let first = bootstrap(&options).unwrap();
        assert_eq!(first.kind, OutcomeKind::Installed);
        assert!(first.steam_executable.is_file());
        let identity_path = data_root.join("games/42/prefix/.macroton-runtime-identity");
        let identity = fs::read_to_string(&identity_path).unwrap();
        assert!(identity.contains("runtime_id=test-wine-1"));
        assert!(
            data_root
                .join("games/42/prefix/.macroton-crash-guard")
                .is_file()
        );
        fs::remove_file(&identity_path).unwrap();
        let second = bootstrap(&options).unwrap();
        assert_eq!(second.kind, OutcomeKind::AlreadyInstalled);
        assert_eq!(first.marker, second.marker);
        assert!(identity_path.is_file());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repairs_only_an_idle_unauthenticated_canonical_fingerprint_migration() {
        let root = std::env::temp_dir().join(format!("macroton-bootstrap-repair-{}", unix_ms()));
        let runtime_root_a = root.join("runtimes-a");
        let runtime_root_b = root.join("runtimes-b");
        let runtime_root_c = root.join("runtimes-c");
        let data_root = root.join("data");
        let fake_runtime = root.join("fake-wine");
        let installer = root.join("SteamSetup.exe");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &fake_runtime,
            "#!/bin/sh\nmkdir -p \"$WINEPREFIX/drive_c/Program Files (x86)/Steam\"\nprintf MZ > \"$WINEPREFIX/drive_c/Program Files (x86)/Steam/Steam.exe\"\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&fake_runtime).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_runtime, permissions).unwrap();
        write_i386_pe(&installer);
        let digest = sha256::to_hex(&sha256::digest_file(&installer).unwrap());
        let profile = profile();
        register_external("test-wine-1", &fake_runtime, &runtime_root_a).unwrap();
        let first = bootstrap(&Options {
            app_id: 42,
            profile: &profile,
            installer: &installer,
            installer_sha256: &digest,
            runtime_root: &runtime_root_a,
            data_root: &data_root,
        })
        .unwrap();

        fs::write(
            &fake_runtime,
            "#!/bin/sh\n# migrated runtime artifact\nexit 99\n",
        )
        .unwrap();
        register_external("test-wine-1", &fake_runtime, &runtime_root_b).unwrap();
        let migrated = runtime::resolve_installed(&runtime_root_b, "test-wine-1").unwrap();
        let repaired = bootstrap(&Options {
            app_id: 42,
            profile: &profile,
            installer: &installer,
            installer_sha256: &digest,
            runtime_root: &runtime_root_b,
            data_root: &data_root,
        })
        .unwrap();
        assert_eq!(repaired.kind, OutcomeKind::RepairedProvenance);
        assert!(
            fs::read_to_string(&repaired.marker)
                .unwrap()
                .contains(&migrated.fingerprint_sha256)
        );
        assert!(
            fs::read_to_string(first.prefix.join(".macroton-runtime-identity"))
                .unwrap()
                .contains(&migrated.fingerprint_sha256)
        );

        fs::write(&fake_runtime, "#!/bin/sh\n# third artifact\nexit 98\n").unwrap();
        register_external("test-wine-1", &fake_runtime, &runtime_root_c).unwrap();
        let third = runtime::resolve_installed(&runtime_root_c, "test-wine-1").unwrap();
        fs::create_dir_all(
            first
                .prefix
                .join("drive_c/Program Files (x86)/Steam/userdata/123"),
        )
        .unwrap();
        let error = bootstrap(&Options {
            app_id: 42,
            profile: &profile,
            installer: &installer,
            installer_sha256: &digest,
            runtime_root: &runtime_root_c,
            data_root: &data_root,
        })
        .unwrap_err();
        assert!(
            fs::read_to_string(first.prefix.join(".macroton-runtime-identity"))
                .unwrap()
                .contains(&migrated.fingerprint_sha256)
        );
        assert!(
            !fs::read_to_string(&first.marker)
                .unwrap()
                .contains(&third.fingerprint_sha256)
        );
        assert!(error.message.contains("refusing to modify"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicitly_rebinds_an_idle_authenticated_prefix_without_copying_account_data() {
        let root = std::env::temp_dir().join(format!("macroton-bootstrap-rebind-{}", unix_ms()));
        let old_runtime_root = root.join("old-runtimes");
        let new_runtime_root = root.join("new-runtimes");
        let data_root = root.join("data");
        let old_runtime = root.join("old-wine");
        let new_runtime = root.join("new-wine");
        let installer = root.join("SteamSetup.exe");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &old_runtime,
            "#!/bin/sh\nmkdir -p \"$WINEPREFIX/drive_c/Program Files (x86)/Steam\"\nprintf MZ > \"$WINEPREFIX/drive_c/Program Files (x86)/Steam/Steam.exe\"\n",
        )
        .unwrap();
        fs::write(&new_runtime, "#!/bin/sh\nexit 0\n").unwrap();
        for runtime in [&old_runtime, &new_runtime] {
            let mut permissions = fs::metadata(runtime).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(runtime, permissions).unwrap();
        }
        write_i386_pe(&installer);
        register_external("test-wine-1", &old_runtime, &old_runtime_root).unwrap();
        register_external("new-wine-1", &new_runtime, &new_runtime_root).unwrap();
        let digest = sha256::to_hex(&sha256::digest_file(&installer).unwrap());
        let old_profile = profile();
        let installed = bootstrap(&Options {
            app_id: 42,
            profile: &old_profile,
            installer: &installer,
            installer_sha256: &digest,
            runtime_root: &old_runtime_root,
            data_root: &data_root,
        })
        .unwrap();
        let steam_root = installed.prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(steam_root.join("config")).unwrap();
        fs::create_dir_all(steam_root.join("userdata/76561190000000000")).unwrap();
        let login = "\"users\"\n{\n\"76561190000000000\"\n{\n\"AccountName\" \"fixture\"\n}\n}\n";
        fs::write(steam_root.join("config/loginusers.vdf"), login).unwrap();
        let new_profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=new-wine\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Game\nexecutable=game.exe\n",
        )
        .unwrap();

        let wrong = rebind_runtime(&RebindOptions {
            app_id: 42,
            profile: &new_profile,
            expected_old_fingerprint: &"00".repeat(32),
            runtime_root: &new_runtime_root,
            data_root: &data_root,
        })
        .unwrap_err();
        assert!(wrong.message.contains("fingerprint changed"));

        let rebound = rebind_runtime(&RebindOptions {
            app_id: 42,
            profile: &new_profile,
            expected_old_fingerprint: &installed.runtime_fingerprint_sha256,
            runtime_root: &new_runtime_root,
            data_root: &data_root,
        })
        .unwrap();
        assert_eq!(rebound.old_runtime_id, "test-wine-1");
        assert_eq!(rebound.runtime_id, "new-wine-1");
        assert_eq!(
            fs::read_to_string(steam_root.join("config/loginusers.vdf")).unwrap(),
            login
        );
        assert!(steam_root.join("userdata/76561190000000000").is_dir());
        assert!(
            fs::read_to_string(&rebound.marker)
                .unwrap()
                .contains("\"runtime_id\": \"new-wine-1\"")
        );
        assert!(
            fs::read_to_string(installed.prefix.join(".macroton-runtime-identity"))
                .unwrap()
                .contains("runtime_id=new-wine-1")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_checksum_mismatch_before_running_runtime() {
        let root = std::env::temp_dir().join(format!("macroton-bootstrap-hash-{}", unix_ms()));
        let runtime_root = root.join("runtimes");
        let fake_runtime = root.join("fake-wine");
        let installer = root.join("SteamSetup.exe");
        fs::create_dir_all(&root).unwrap();
        fs::write(&fake_runtime, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&fake_runtime).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_runtime, permissions).unwrap();
        write_i386_pe(&installer);
        register_external("test-wine-1", &fake_runtime, &runtime_root).unwrap();
        let profile = profile();

        let error = bootstrap(&Options {
            app_id: 42,
            profile: &profile,
            installer: &installer,
            installer_sha256: &"0".repeat(64),
            runtime_root: &runtime_root,
            data_root: &root.join("data"),
        })
        .unwrap_err();
        assert!(error.message.contains("checksum mismatch"));
        assert!(!root.join("data").exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_an_mz_marker_without_a_valid_pe_header() {
        let root = std::env::temp_dir().join(format!("macroton-bootstrap-pe-{}", unix_ms()));
        let installer = root.join("SteamSetup.exe");
        fs::create_dir_all(&root).unwrap();
        fs::write(&installer, b"MZnot-a-pe").unwrap();
        let digest = sha256::to_hex(&sha256::digest_file(&installer).unwrap());
        let profile = profile();

        let error = bootstrap(&Options {
            app_id: 42,
            profile: &profile,
            installer: &installer,
            installer_sha256: &digest,
            runtime_root: &root.join("runtimes"),
            data_root: &root.join("data"),
        })
        .unwrap_err();
        assert!(error.message.contains("not a Windows PE executable"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bootstrap_keeps_setup_defaults_and_disables_overlay() {
        assert_eq!(
            bootstrap_dll_overrides(None),
            "mscoree,mshtml=;gameoverlayrenderer=d;gameoverlayrenderer64=d"
        );
        assert_eq!(
            bootstrap_dll_overrides(Some(&"xinput1_3=n,b".to_owned())),
            "mscoree,mshtml=;xinput1_3=n,b;gameoverlayrenderer=d;gameoverlayrenderer64=d"
        );
    }

    #[test]
    fn canonical_stale_marker_accepts_a_supported_runtime_kind_migration() {
        let fingerprint = "ab".repeat(32);
        let marker = marker_contents(
            42,
            "wine-1",
            RuntimeKind::ExternalExecutable,
            &fingerprint,
            &"cd".repeat(32),
        );
        assert_eq!(
            canonical_marker_runtime(&marker),
            Some((RuntimeKind::ExternalExecutable, fingerprint.clone()))
        );
        assert!(
            canonical_marker_runtime(
                &marker.replace("\"external-executable\"", "\"unsupported-runtime\"")
            )
            .is_none()
        );
        let identity = format!(
            "schema=1\nruntime_id=wine-1\nruntime_fingerprint_sha256={fingerprint}\ncomponents=dxvk-1,wine-1\n"
        );
        assert!(canonical_runtime_identity(
            &identity,
            "wine-1",
            &fingerprint
        ));
        assert!(!canonical_runtime_identity(
            &format!("{identity}unknown=value\n"),
            "wine-1",
            &fingerprint
        ));
        assert!(!canonical_runtime_identity(
            &identity.replace("runtime_id=wine-1", "runtime_id=other"),
            "wine-1",
            &fingerprint
        ));
    }
}
