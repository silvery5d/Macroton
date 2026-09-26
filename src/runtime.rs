use crate::binary::{self, PeArchitecture};
use crate::sha256;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MARKER_FILE: &str = ".macroton-runtime";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeManifest {
    pub schema: u32,
    pub id: String,
    pub family: String,
    pub version: String,
    pub source: String,
    pub license: String,
    pub license_url: String,
    pub sha256: String,
    pub archive: ArchiveFormat,
    pub entrypoint: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Tar,
    TarGz,
    TarXz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed,
    AlreadyInstalled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    ManagedArchive,
    ExternalExecutable,
}

impl RuntimeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManagedArchive => "managed-archive",
            Self::ExternalExecutable => "external-executable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRuntime {
    pub id: String,
    pub kind: RuntimeKind,
    pub fingerprint_sha256: String,
    pub entrypoint: PathBuf,
    pub components: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCandidate {
    pub provider: String,
    pub entrypoint: PathBuf,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Win32ProbeResult {
    pub entrypoint: PathBuf,
    pub payload: PathBuf,
    pub reported_windows_version: String,
    pub host_execution: HostExecution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeReadiness {
    TranslatedBootstrap,
    NativeHostBlocked,
    NativeHostNeedsX86Emulator,
    NativeWin32Ready,
    UnsupportedHost,
}

impl RuntimeReadiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TranslatedBootstrap => "translated-bootstrap",
            Self::NativeHostBlocked => "native-host-blocked",
            Self::NativeHostNeedsX86Emulator => "native-host-needs-x86-emulator",
            Self::NativeWin32Ready => "native-win32-ready",
            Self::UnsupportedHost => "unsupported-host",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadProbe {
    pub payload: Option<PathBuf>,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAudit {
    pub entrypoint: PathBuf,
    pub host_execution: HostExecution,
    pub native_payload: PayloadProbe,
    pub win32_payload: PayloadProbe,
    pub readiness: RuntimeReadiness,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTreeAudit {
    pub runtime_id: String,
    pub fingerprint_sha256: String,
    pub entries_checked: usize,
    pub symbolic_links_checked: usize,
    pub entrypoint_dependencies_checked: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostExecution {
    Native,
    UniversalNativeSlice,
    TranslationRequired,
    Unknown,
}

impl HostExecution {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::UniversalNativeSlice => "native slice in universal binary",
            Self::TranslationRequired => "host translation required",
            Self::Unknown => "unknown",
        }
    }
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

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl RuntimeManifest {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let contents = fs::read_to_string(path).map_err(|error| Error {
            message: format!("cannot read manifest {}: {error}", path.display()),
        })?;
        Self::parse(&contents)
    }

    pub fn parse(contents: &str) -> Result<Self, Error> {
        let mut values = BTreeMap::<String, (usize, String)>::new();
        for (index, raw_line) in contents.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or_else(|| Error {
                message: format!("manifest line {line_number}: expected key=value"),
            })?;
            let key = key.trim();
            if !matches!(
                key,
                "schema"
                    | "id"
                    | "family"
                    | "version"
                    | "source"
                    | "license"
                    | "license_url"
                    | "sha256"
                    | "archive"
                    | "entrypoint"
            ) {
                return Err(Error {
                    message: format!("manifest line {line_number}: unknown key {key}"),
                });
            }
            if values
                .insert(key.to_owned(), (line_number, value.trim().to_owned()))
                .is_some()
            {
                return Err(Error {
                    message: format!("manifest line {line_number}: duplicate key {key}"),
                });
            }
        }
        let required = |key: &str| -> Result<&str, Error> {
            values
                .get(key)
                .map(|(_, value)| value.as_str())
                .ok_or_else(|| Error {
                    message: format!("manifest: missing required key {key}"),
                })
        };
        let parse_u32 = |key: &str| -> Result<u32, Error> {
            let (line, value) = values.get(key).ok_or_else(|| Error {
                message: format!("manifest: missing required key {key}"),
            })?;
            value.parse().map_err(|_| Error {
                message: format!("manifest line {line}: {key} is not an unsigned integer"),
            })
        };

        let schema = parse_u32("schema")?;
        if schema != 1 {
            return Err(Error {
                message: format!("manifest: unsupported schema {schema}"),
            });
        }
        let id = required("id")?.to_owned();
        let family = required("family")?.to_owned();
        let version = required("version")?.to_owned();
        validate_identifier("id", &id)?;
        validate_identifier("family", &family)?;
        validate_identifier("version", &version)?;
        let expected_id = format!("{family}-{version}");
        if id != expected_id {
            return Err(Error {
                message: format!("manifest: id must be {expected_id} for this family and version"),
            });
        }
        let source = required("source")?.to_owned();
        if !(source.starts_with("https://") || source.starts_with("file://")) {
            return Err(Error {
                message: "manifest: source must use https:// or file://".to_owned(),
            });
        }
        let license_url = required("license_url")?.to_owned();
        if !license_url.starts_with("https://") {
            return Err(Error {
                message: "manifest: license_url must use https://".to_owned(),
            });
        }
        let sha256 = required("sha256")?.to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error {
                message: "manifest: sha256 must contain exactly 64 hexadecimal digits".to_owned(),
            });
        }
        let archive = match required("archive")? {
            "tar" => ArchiveFormat::Tar,
            "tar.gz" => ArchiveFormat::TarGz,
            "tar.xz" => ArchiveFormat::TarXz,
            other => {
                return Err(Error {
                    message: format!("manifest: unsupported archive format {other}"),
                });
            }
        };
        let entrypoint = PathBuf::from(required("entrypoint")?);
        validate_relative_path("entrypoint", &entrypoint)?;

        let license = required("license")?.to_owned();
        if license.is_empty() {
            return Err(Error {
                message: "manifest: license cannot be empty".to_owned(),
            });
        }

        Ok(Self {
            schema,
            id,
            family,
            version,
            source,
            license,
            license_url,
            sha256,
            archive,
            entrypoint,
        })
    }
}

pub fn verify_artifact(manifest: &RuntimeManifest, artifact: &Path) -> Result<(), Error> {
    let digest = sha256::digest_file(artifact).map_err(|error| Error {
        message: format!("cannot hash artifact {}: {error}", artifact.display()),
    })?;
    let actual = sha256::to_hex(&digest);
    if actual != manifest.sha256 {
        return Err(Error {
            message: format!(
                "artifact checksum mismatch: expected {}, got {actual}",
                manifest.sha256
            ),
        });
    }
    Ok(())
}

pub fn install(
    manifest: &RuntimeManifest,
    artifact: &Path,
    runtime_root: &Path,
) -> Result<InstallOutcome, Error> {
    verify_artifact(manifest, artifact)?;
    inspect_archive(artifact)?;
    fs::create_dir_all(runtime_root).map_err(|error| Error {
        message: format!(
            "cannot create runtime root {}: {error}",
            runtime_root.display()
        ),
    })?;

    let target = runtime_root.join(&manifest.id);
    if target.exists() {
        if installation_matches(&target, manifest) {
            return Ok(InstallOutcome::AlreadyInstalled);
        }
        return Err(Error {
            message: format!(
                "runtime target already exists but does not match the manifest: {}",
                target.display()
            ),
        });
    }

    let staging = staging_path(runtime_root, &manifest.id);
    fs::create_dir(&staging).map_err(|error| Error {
        message: format!(
            "cannot create staging directory {}: {error}",
            staging.display()
        ),
    })?;
    let mut guard = StagingGuard {
        path: staging.clone(),
        committed: false,
    };

    let output = Command::new("/usr/bin/tar")
        .arg("-xf")
        .arg(artifact)
        .arg("-C")
        .arg(&staging)
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| Error {
            message: format!("cannot execute /usr/bin/tar: {error}"),
        })?;
    if !output.status.success() {
        return Err(Error {
            message: format!(
                "runtime extraction failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }

    validate_extracted_tree(&staging, &manifest.entrypoint)?;
    fs::write(staging.join(MARKER_FILE), marker_contents(manifest)).map_err(|error| Error {
        message: format!("cannot write runtime installation marker: {error}"),
    })?;
    fs::rename(&staging, &target).map_err(|error| Error {
        message: format!(
            "cannot atomically install runtime at {}: {error}",
            target.display()
        ),
    })?;
    guard.committed = true;
    Ok(InstallOutcome::Installed)
}

pub fn register_external(
    runtime_id: &str,
    entrypoint: &Path,
    runtime_root: &Path,
) -> Result<InstallOutcome, Error> {
    validate_identifier("runtime id", runtime_id)?;
    let canonical_entrypoint = entrypoint.canonicalize().map_err(|error| Error {
        message: format!(
            "cannot resolve external runtime entrypoint {}: {error}",
            entrypoint.display()
        ),
    })?;
    validate_executable(&canonical_entrypoint)?;
    let fingerprint_sha256 = sha256::to_hex(&sha256::digest_file(&canonical_entrypoint).map_err(
        |error| Error {
            message: format!(
                "cannot fingerprint external runtime {}: {error}",
                canonical_entrypoint.display()
            ),
        },
    )?);

    fs::create_dir_all(runtime_root).map_err(|error| Error {
        message: format!(
            "cannot create runtime root {}: {error}",
            runtime_root.display()
        ),
    })?;
    let target = runtime_root.join(runtime_id);
    if target.exists() {
        let existing = resolve_installed(runtime_root, runtime_id);
        if existing.as_ref().is_ok_and(|runtime| {
            runtime.kind == RuntimeKind::ExternalExecutable
                && runtime.entrypoint == canonical_entrypoint
                && runtime.fingerprint_sha256 == fingerprint_sha256
        }) {
            return Ok(InstallOutcome::AlreadyInstalled);
        }
        return Err(Error {
            message: format!(
                "runtime target already exists but does not match this external runtime: {}",
                target.display()
            ),
        });
    }

    let staging = staging_path(runtime_root, runtime_id);
    fs::create_dir(&staging).map_err(|error| Error {
        message: format!(
            "cannot create staging directory {}: {error}",
            staging.display()
        ),
    })?;
    let mut guard = StagingGuard {
        path: staging.clone(),
        committed: false,
    };
    let marker = format!(
        "schema=1\nkind={}\nid={runtime_id}\nsha256={fingerprint_sha256}\nentrypoint={}\nsource=user-installed\n",
        RuntimeKind::ExternalExecutable.as_str(),
        canonical_entrypoint.display()
    );
    fs::write(staging.join(MARKER_FILE), marker).map_err(|error| Error {
        message: format!("cannot write external runtime marker: {error}"),
    })?;
    fs::rename(&staging, &target).map_err(|error| Error {
        message: format!(
            "cannot atomically register external runtime at {}: {error}",
            target.display()
        ),
    })?;
    guard.committed = true;
    Ok(InstallOutcome::Installed)
}

pub fn discover_external_candidates() -> Vec<ExternalCandidate> {
    let mut application_roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        application_roots.push(PathBuf::from(home).join("Applications"));
    }
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for root in application_roots {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let app = entry.path();
            let Some(name) = app.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let entrypoint = if name.starts_with("Wine") && name.ends_with(".app") {
                app.join("Contents/Resources/wine/bin/wine")
            } else {
                continue;
            };
            let Ok(entrypoint) = entrypoint.canonicalize() else {
                continue;
            };
            if !seen.insert(entrypoint.clone()) || validate_executable(&entrypoint).is_err() {
                continue;
            }
            let version = Command::new(&entrypoint)
                .arg("--version")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| {
                    let stdout = String::from_utf8(output.stdout).ok()?;
                    let version = stdout.trim();
                    (!version.is_empty()).then(|| version.to_owned())
                });
            candidates.push(ExternalCandidate {
                provider: name.trim_end_matches(".app").to_owned(),
                entrypoint,
                version,
            });
        }
    }
    candidates.sort_by(|left, right| left.provider.cmp(&right.provider));
    candidates
}

pub fn probe_win32(entrypoint: &Path, probe_root: &Path) -> Result<Win32ProbeResult, Error> {
    let entrypoint = entrypoint.canonicalize().map_err(|error| Error {
        message: format!(
            "cannot resolve runtime entrypoint {}: {error}",
            entrypoint.display()
        ),
    })?;
    validate_executable(&entrypoint)?;
    let host_execution = classify_host_execution(
        &binary::inspect(&entrypoint).map_err(|error| Error {
            message: format!("cannot inspect runtime host executable: {error}"),
        })?,
        std::env::consts::ARCH,
    );
    let payload = locate_i386_cmd(&entrypoint).ok_or_else(|| Error {
        message: format!(
            "runtime does not contain a discoverable i386-windows/cmd.exe near {}",
            entrypoint.display()
        ),
    })?;
    if binary::inspect_pe_architecture(&payload).map_err(|error| Error {
        message: format!("cannot inspect Win32 probe payload: {error}"),
    })? != Some(PeArchitecture::I386)
    {
        return Err(Error {
            message: format!("probe payload is not PE32 i386: {}", payload.display()),
        });
    }

    fs::create_dir_all(probe_root).map_err(|error| Error {
        message: format!("cannot create probe root {}: {error}", probe_root.display()),
    })?;
    let session_root = staging_path(probe_root, "win32-probe");
    fs::create_dir(&session_root).map_err(|error| Error {
        message: format!(
            "cannot create private probe directory {}: {error}",
            session_root.display()
        ),
    })?;
    let prefix = session_root.join("prefix");
    let _guard = ProbeGuard { path: session_root };
    let _server_guard = ProbeServerGuard {
        entrypoint: entrypoint.clone(),
        prefix: prefix.clone(),
    };
    let mut command = Command::new(&entrypoint);
    command
        .arg(&payload)
        .args(["/c", "ver"])
        .env("WINEPREFIX", &prefix)
        .env("WINEDEBUG", "-all")
        .env("WINEDLLOVERRIDES", "mscoree,mshtml=")
        .env("MVK_CONFIG_LOG_LEVEL", "0");
    let (status, stdout, stderr) = run_bounded(command, Duration::from_secs(60))?;

    let mut shutdown = Command::new(&entrypoint);
    shutdown
        .args(["wineboot", "--shutdown"])
        .env("WINEPREFIX", &prefix)
        .env("WINEDEBUG", "-all")
        .env("WINEDLLOVERRIDES", "mscoree,mshtml=")
        .env("MVK_CONFIG_LOG_LEVEL", "0");
    let _ = run_bounded(shutdown, Duration::from_secs(10));

    if !status.success() {
        return Err(Error {
            message: format!(
                "Win32 probe exited with {status}: {}",
                truncate_output(&stderr)
            ),
        });
    }
    let reported_windows_version = stdout.trim().to_owned();
    if reported_windows_version.is_empty() {
        return Err(Error {
            message: format!(
                "Win32 probe produced no version output; stderr: {}",
                truncate_output(&stderr)
            ),
        });
    }
    Ok(Win32ProbeResult {
        entrypoint,
        payload,
        reported_windows_version,
        host_execution,
    })
}

pub fn audit_runtime(entrypoint: &Path, probe_root: &Path) -> Result<RuntimeAudit, Error> {
    let entrypoint = entrypoint.canonicalize().map_err(|error| Error {
        message: format!(
            "cannot resolve runtime entrypoint {}: {error}",
            entrypoint.display()
        ),
    })?;
    validate_executable(&entrypoint)?;
    let host_execution = classify_host_execution(
        &binary::inspect(&entrypoint).map_err(|error| Error {
            message: format!("cannot inspect runtime host executable: {error}"),
        })?,
        std::env::consts::ARCH,
    );

    fs::create_dir_all(probe_root).map_err(|error| Error {
        message: format!("cannot create probe root {}: {error}", probe_root.display()),
    })?;
    let native_payload_path = if matches!(
        host_execution,
        HostExecution::Native | HostExecution::UniversalNativeSlice
    ) && std::env::consts::ARCH == "aarch64"
    {
        locate_payload(&entrypoint, "aarch64-windows/cmd.exe")
    } else {
        None
    };
    let native_payload = match native_payload_path {
        Some(path) => probe_payload(&entrypoint, &path, probe_root, "native-arm64")?,
        None => PayloadProbe {
            payload: None,
            passed: false,
            detail: "not applicable or payload missing".to_owned(),
        },
    };
    let win32_payload = match locate_i386_cmd(&entrypoint) {
        Some(path) => probe_payload(&entrypoint, &path, probe_root, "win32-i386")?,
        None => PayloadProbe {
            payload: None,
            passed: false,
            detail: "i386-windows/cmd.exe is missing".to_owned(),
        },
    };

    let readiness =
        classify_runtime_readiness(host_execution, native_payload.passed, win32_payload.passed);
    Ok(RuntimeAudit {
        entrypoint,
        host_execution,
        native_payload,
        win32_payload,
        readiness,
    })
}

pub fn audit_release_tree(
    manifest: &RuntimeManifest,
    runtime_root: &Path,
) -> Result<ReleaseTreeAudit, Error> {
    if !manifest.source.starts_with("https://") {
        return Err(Error {
            message: "release runtime source must use https://".to_owned(),
        });
    }
    if !manifest.license.to_ascii_lowercase().starts_with("lgpl-") {
        return Err(Error {
            message: "release runtime must declare an LGPL SPDX license".to_owned(),
        });
    }
    reject_forbidden_release_text("manifest id", &manifest.id)?;
    reject_forbidden_release_text("manifest family", &manifest.family)?;
    reject_forbidden_release_text("manifest source", &manifest.source)?;
    reject_forbidden_release_text(
        "manifest entrypoint",
        &manifest.entrypoint.to_string_lossy(),
    )?;

    let installed = resolve_installed(runtime_root, &manifest.id)?;
    if installed.kind != RuntimeKind::ManagedArchive {
        return Err(Error {
            message: "release runtime must be a managed archive, not an external executable"
                .to_owned(),
        });
    }
    if installed.fingerprint_sha256 != manifest.sha256 {
        return Err(Error {
            message: "installed runtime fingerprint does not match release manifest".to_owned(),
        });
    }
    let runtime_directory = runtime_root.join(&manifest.id);
    let canonical_runtime = runtime_directory.canonicalize().map_err(|error| Error {
        message: format!("cannot resolve release runtime directory: {error}"),
    })?;
    let expected_entrypoint = runtime_directory
        .join(&manifest.entrypoint)
        .canonicalize()
        .map_err(|error| Error {
            message: format!("cannot resolve manifest entrypoint: {error}"),
        })?;
    if installed.entrypoint != expected_entrypoint {
        return Err(Error {
            message: "installed runtime entrypoint does not match release manifest".to_owned(),
        });
    }
    let entrypoint_dependencies_checked = audit_macho_dependencies(&installed.entrypoint)?;

    let mut pending = vec![runtime_directory];
    let mut entries_checked = 0_usize;
    let mut symbolic_links_checked = 0_usize;
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| Error {
            message: format!(
                "cannot inspect release runtime {}: {error}",
                directory.display()
            ),
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| Error {
                message: format!(
                    "cannot inspect release runtime {}: {error}",
                    directory.display()
                ),
            })?;
            let path = entry.path();
            let relative = path.strip_prefix(runtime_root).map_err(|_| Error {
                message: "release runtime path escaped its registry root".to_owned(),
            })?;
            reject_forbidden_release_text("runtime path", &relative.to_string_lossy())?;
            entries_checked += 1;
            let metadata = fs::symlink_metadata(&path).map_err(|error| Error {
                message: format!(
                    "cannot inspect release runtime entry {}: {error}",
                    path.display()
                ),
            })?;
            if metadata.file_type().is_symlink() {
                symbolic_links_checked += 1;
                let target = fs::read_link(&path).map_err(|error| Error {
                    message: format!(
                        "cannot read runtime symbolic link {}: {error}",
                        path.display()
                    ),
                })?;
                reject_forbidden_release_text(
                    "runtime symbolic-link target",
                    &target.to_string_lossy(),
                )?;
                let resolved = path.canonicalize().map_err(|error| Error {
                    message: format!(
                        "cannot resolve runtime symbolic link {}: {error}",
                        path.display()
                    ),
                })?;
                if !resolved.starts_with(&canonical_runtime) {
                    return Err(Error {
                        message: format!(
                            "release runtime symbolic link escapes installation: {}",
                            relative.display()
                        ),
                    });
                }
            } else if metadata.is_dir() {
                pending.push(path);
            } else if !metadata.is_file() {
                return Err(Error {
                    message: format!(
                        "release runtime contains unsupported entry: {}",
                        relative.display()
                    ),
                });
            }
        }
    }
    Ok(ReleaseTreeAudit {
        runtime_id: manifest.id.clone(),
        fingerprint_sha256: installed.fingerprint_sha256,
        entries_checked,
        symbolic_links_checked,
        entrypoint_dependencies_checked,
    })
}

fn audit_macho_dependencies(path: &Path) -> Result<usize, Error> {
    let output = Command::new("/usr/bin/otool")
        .arg("-L")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| Error {
            message: format!("cannot inspect runtime entrypoint dependencies: {error}"),
        })?;
    if !output.status.success() {
        return Err(Error {
            message: format!(
                "cannot inspect runtime entrypoint dependencies: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    let stdout = String::from_utf8(output.stdout).map_err(|_| Error {
        message: "runtime entrypoint dependency list is not UTF-8".to_owned(),
    })?;
    let mut dependencies = 0_usize;
    for line in stdout.lines().skip(1) {
        let dependency = line.trim().split(" (").next().unwrap_or("").trim();
        if dependency.is_empty() {
            continue;
        }
        reject_forbidden_release_text("runtime entrypoint dependency", dependency)?;
        dependencies += 1;
    }
    if dependencies == 0 {
        return Err(Error {
            message: "runtime entrypoint has no inspectable Mach-O dependencies".to_owned(),
        });
    }
    Ok(dependencies)
}

fn reject_forbidden_release_text(label: &str, value: &str) -> Result<(), Error> {
    let normalized = value.to_ascii_lowercase();
    if ["crossover", "codeweavers", "d3dmetal"]
        .iter()
        .any(|forbidden| normalized.contains(forbidden))
    {
        return Err(Error {
            message: format!("{label} contains a forbidden commercial-runtime identifier"),
        });
    }
    Ok(())
}

fn classify_runtime_readiness(
    host_execution: HostExecution,
    native_payload_passed: bool,
    win32_payload_passed: bool,
) -> RuntimeReadiness {
    match host_execution {
        HostExecution::TranslationRequired if win32_payload_passed => {
            RuntimeReadiness::TranslatedBootstrap
        }
        HostExecution::Native | HostExecution::UniversalNativeSlice if !native_payload_passed => {
            RuntimeReadiness::NativeHostBlocked
        }
        HostExecution::Native | HostExecution::UniversalNativeSlice if win32_payload_passed => {
            RuntimeReadiness::NativeWin32Ready
        }
        HostExecution::Native | HostExecution::UniversalNativeSlice => {
            RuntimeReadiness::NativeHostNeedsX86Emulator
        }
        _ => RuntimeReadiness::UnsupportedHost,
    }
}

fn probe_payload(
    entrypoint: &Path,
    payload: &Path,
    probe_root: &Path,
    label: &str,
) -> Result<PayloadProbe, Error> {
    let session_root = staging_path(probe_root, label);
    fs::create_dir(&session_root).map_err(|error| Error {
        message: format!(
            "cannot create private probe directory {}: {error}",
            session_root.display()
        ),
    })?;
    let prefix = session_root.join("prefix");
    let _guard = ProbeGuard { path: session_root };
    let _server_guard = ProbeServerGuard {
        entrypoint: entrypoint.to_owned(),
        prefix: prefix.clone(),
    };
    let mut command = Command::new(entrypoint);
    command
        .arg(payload)
        .args(["/c", "ver"])
        .env("WINEPREFIX", &prefix)
        .env("WINEDEBUG", "-all")
        .env("WINEDLLOVERRIDES", "mscoree,mshtml=")
        .env("MVK_CONFIG_LOG_LEVEL", "0");
    let (status, stdout, stderr) = run_bounded(command, Duration::from_secs(60))?;
    let output = stdout.trim();
    let passed = status.success() && !output.is_empty();
    let detail = if passed {
        output.to_owned()
    } else {
        format_exit_detail(status, &stderr)
    };
    Ok(PayloadProbe {
        payload: Some(payload.to_owned()),
        passed,
        detail,
    })
}

fn stop_probe_server(entrypoint: &Path, prefix: &Path) {
    let Some(bin_dir) = entrypoint.parent() else {
        return;
    };
    let wineserver = bin_dir.join("wineserver");
    if !wineserver.is_file() {
        return;
    }
    let mut command = Command::new(wineserver);
    command
        .arg("-k")
        .env("WINEPREFIX", prefix)
        .env("WINEDEBUG", "-all");
    let _ = run_bounded(command, Duration::from_secs(10));
}

fn format_exit_detail(status: ExitStatus, stderr: &str) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("terminated by signal {signal}: {}", truncate_output(stderr));
        }
    }
    format!("exited with {status}: {}", truncate_output(stderr))
}

fn classify_host_execution(kind: &binary::BinaryKind, host_arch: &str) -> HostExecution {
    let binary::BinaryKind::MachO(architectures) = kind else {
        return HostExecution::Unknown;
    };
    let native = match host_arch {
        "aarch64" => binary::Architecture::Arm64,
        "x86_64" => binary::Architecture::X86_64,
        _ => return HostExecution::Unknown,
    };
    if architectures.contains(&native) {
        if architectures.len() > 1 {
            HostExecution::UniversalNativeSlice
        } else {
            HostExecution::Native
        }
    } else if host_arch == "aarch64" && architectures.contains(&binary::Architecture::X86_64) {
        HostExecution::TranslationRequired
    } else {
        HostExecution::Unknown
    }
}

fn locate_i386_cmd(entrypoint: &Path) -> Option<PathBuf> {
    locate_payload(entrypoint, "i386-windows/cmd.exe")
}

fn locate_payload(entrypoint: &Path, payload_relative: &str) -> Option<PathBuf> {
    for ancestor in entrypoint.ancestors().take(6) {
        for candidate in [
            ancestor.join(payload_relative),
            ancestor.join("lib/wine").join(payload_relative),
        ] {
            if candidate.is_file() {
                return candidate.canonicalize().ok();
            }
        }
    }
    None
}

fn run_bounded(
    mut command: Command,
    timeout: Duration,
) -> Result<(ExitStatus, String, String), Error> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| Error {
        message: format!("cannot start runtime probe: {error}"),
    })?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| Error {
            message: format!("cannot query runtime probe status: {error}"),
        })? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error {
                message: format!(
                    "runtime probe timed out after {} seconds",
                    timeout.as_secs()
                ),
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_end(&mut stdout).map_err(|error| Error {
            message: format!("cannot read runtime probe stdout: {error}"),
        })?;
    }
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_end(&mut stderr).map_err(|error| Error {
            message: format!("cannot read runtime probe stderr: {error}"),
        })?;
    }
    Ok((
        status,
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    ))
}

fn truncate_output(output: &str) -> String {
    const LIMIT: usize = 2_000;
    if output.chars().count() <= LIMIT {
        output.trim().to_owned()
    } else {
        format!(
            "{}...",
            output.chars().take(LIMIT).collect::<String>().trim()
        )
    }
}

pub fn installed_runtime_ids(runtime_root: &Path) -> Result<Vec<String>, Error> {
    if !runtime_root.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(runtime_root).map_err(|error| Error {
        message: format!(
            "cannot read runtime root {}: {error}",
            runtime_root.display()
        ),
    })?;
    let mut ids = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || !path.join(MARKER_FILE).is_file() {
            continue;
        }
        if let Some(id) = path.file_name().and_then(|name| name.to_str())
            && !id.starts_with(".install-")
        {
            ids.push(id.to_owned());
        }
    }
    ids.sort();
    Ok(ids)
}

pub fn is_installed(runtime_root: &Path, runtime_id: &str) -> bool {
    resolve_installed(runtime_root, runtime_id).is_ok()
}

pub fn resolve_installed(runtime_root: &Path, runtime_id: &str) -> Result<InstalledRuntime, Error> {
    validate_identifier("runtime id", runtime_id)?;
    let directory = runtime_root.join(runtime_id);
    let marker_path = directory.join(MARKER_FILE);
    let marker = fs::read_to_string(&marker_path).map_err(|error| Error {
        message: format!(
            "cannot read runtime marker {}: {error}",
            marker_path.display()
        ),
    })?;
    let mut id = None;
    let mut sha256 = None;
    let mut entrypoint = None;
    let mut kind = None;
    for line in marker.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "kind" => kind = Some(value),
            "id" => id = Some(value.to_owned()),
            "sha256" => sha256 = Some(value.to_owned()),
            "entrypoint" => entrypoint = Some(PathBuf::from(value)),
            _ => {}
        }
    }
    let kind = match kind.unwrap_or("managed-archive") {
        "managed-archive" => RuntimeKind::ManagedArchive,
        "external-executable" => RuntimeKind::ExternalExecutable,
        other => {
            return Err(Error {
                message: format!("runtime marker contains unknown kind {other}"),
            });
        }
    };
    if id.as_deref() != Some(runtime_id) {
        return Err(Error {
            message: format!("runtime marker identity does not match {runtime_id}"),
        });
    }
    let fingerprint_sha256 = sha256.ok_or_else(|| Error {
        message: "runtime marker is missing sha256".to_owned(),
    })?;
    if fingerprint_sha256.len() != 64
        || !fingerprint_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(Error {
            message: "runtime marker contains an invalid sha256".to_owned(),
        });
    }
    let configured_entrypoint = entrypoint.ok_or_else(|| Error {
        message: "runtime marker is missing entrypoint".to_owned(),
    })?;
    let canonical_entrypoint = match kind {
        RuntimeKind::ManagedArchive => {
            validate_relative_path("runtime marker entrypoint", &configured_entrypoint)?;
            let entrypoint = directory.join(configured_entrypoint);
            let canonical_directory = directory.canonicalize().map_err(|error| Error {
                message: format!("cannot resolve runtime directory: {error}"),
            })?;
            let canonical_entrypoint = entrypoint.canonicalize().map_err(|error| Error {
                message: format!("cannot resolve runtime entrypoint: {error}"),
            })?;
            if !canonical_entrypoint.starts_with(&canonical_directory) {
                return Err(Error {
                    message: "runtime entrypoint escapes its installation".to_owned(),
                });
            }
            canonical_entrypoint
        }
        RuntimeKind::ExternalExecutable => {
            if !configured_entrypoint.is_absolute() {
                return Err(Error {
                    message: "external runtime entrypoint must be absolute".to_owned(),
                });
            }
            let canonical_entrypoint =
                configured_entrypoint
                    .canonicalize()
                    .map_err(|error| Error {
                        message: format!("cannot resolve external runtime entrypoint: {error}"),
                    })?;
            let actual = sha256::to_hex(&sha256::digest_file(&canonical_entrypoint).map_err(
                |error| Error {
                    message: format!("cannot fingerprint external runtime: {error}"),
                },
            )?);
            if actual != fingerprint_sha256 {
                return Err(Error {
                    message: format!(
                        "external runtime changed since registration: expected {fingerprint_sha256}, got {actual}"
                    ),
                });
            }
            canonical_entrypoint
        }
    };
    let components = verify_component_manifests(&canonical_entrypoint)?;
    validate_executable(&canonical_entrypoint)?;
    Ok(InstalledRuntime {
        id: runtime_id.to_owned(),
        kind,
        fingerprint_sha256,
        entrypoint: canonical_entrypoint,
        components,
    })
}

fn verify_component_manifests(entrypoint: &Path) -> Result<Vec<String>, Error> {
    let Some(runtime_root) = entrypoint.parent().and_then(Path::parent) else {
        return Ok(Vec::new());
    };
    let component_root = runtime_root.join("share/macroton/components");
    if !component_root.exists() {
        return Ok(Vec::new());
    }
    let canonical_runtime_root = runtime_root.canonicalize().map_err(|error| Error {
        message: format!("cannot resolve component runtime root: {error}"),
    })?;
    let mut manifests = fs::read_dir(&component_root)
        .map_err(|error| Error {
            message: format!(
                "cannot read component manifests {}: {error}",
                component_root.display()
            ),
        })?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("conf"))
        .collect::<Vec<_>>();
    manifests.sort();
    let mut component_ids = Vec::new();
    for manifest in manifests {
        let contents = fs::read_to_string(&manifest).map_err(|error| Error {
            message: format!(
                "cannot read component manifest {}: {error}",
                manifest.display()
            ),
        })?;
        let mut verified_files = 0usize;
        let mut component_id = None;
        for (index, line) in contents.lines().enumerate() {
            let Some((key, expected)) = line.split_once('=') else {
                continue;
            };
            if key == "id" {
                validate_identifier("component id", expected)?;
                component_id = Some(expected.to_owned());
                continue;
            }
            let Some(relative) = key.strip_prefix("file.") else {
                continue;
            };
            let relative = Path::new(relative);
            validate_relative_path("component file", relative)?;
            if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(Error {
                    message: format!(
                        "component manifest {} line {} has an invalid sha256",
                        manifest.display(),
                        index + 1
                    ),
                });
            }
            let component = runtime_root.join("lib/wine").join(relative);
            let canonical_component = component.canonicalize().map_err(|error| Error {
                message: format!(
                    "cannot resolve component file {}: {error}",
                    component.display()
                ),
            })?;
            if !canonical_component.starts_with(&canonical_runtime_root) {
                return Err(Error {
                    message: format!(
                        "component file escapes runtime root: {}",
                        component.display()
                    ),
                });
            }
            let actual =
                sha256::to_hex(&sha256::digest_file(&canonical_component).map_err(|error| {
                    Error {
                        message: format!(
                            "cannot fingerprint component file {}: {error}",
                            canonical_component.display()
                        ),
                    }
                })?);
            if actual != expected.to_ascii_lowercase() {
                return Err(Error {
                    message: format!(
                        "runtime component changed: {} expected {}, got {}",
                        relative.display(),
                        expected,
                        actual
                    ),
                });
            }
            verified_files += 1;
        }
        if verified_files == 0 {
            return Err(Error {
                message: format!(
                    "component manifest contains no file fingerprints: {}",
                    manifest.display()
                ),
            });
        }
        let component_id = component_id.ok_or_else(|| Error {
            message: format!("component manifest is missing id: {}", manifest.display()),
        })?;
        if component_ids.contains(&component_id) {
            return Err(Error {
                message: format!("duplicate runtime component id: {component_id}"),
            });
        }
        component_ids.push(component_id);
    }
    component_ids.sort();
    Ok(component_ids)
}

fn validate_executable(path: &Path) -> Result<(), Error> {
    if !path.is_file() {
        return Err(Error {
            message: format!("runtime entrypoint does not exist: {}", path.display()),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map_err(|error| Error {
                message: format!("cannot inspect runtime entrypoint: {error}"),
            })?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(Error {
                message: format!("runtime entrypoint is not executable: {}", path.display()),
            });
        }
    }
    Ok(())
}

fn inspect_archive(artifact: &Path) -> Result<(), Error> {
    let names = tar_output(artifact, "-tf")?;
    let verbose = tar_output(artifact, "-tvf")?;
    let paths: Vec<&str> = names.lines().collect();
    let metadata: Vec<&str> = verbose.lines().collect();
    if paths.is_empty() {
        return Err(Error {
            message: "runtime archive is empty".to_owned(),
        });
    }
    if paths.len() != metadata.len() {
        return Err(Error {
            message: "runtime archive listing is ambiguous".to_owned(),
        });
    }
    for (path, metadata) in paths.into_iter().zip(metadata) {
        validate_relative_path("archive member", Path::new(path))?;
        match metadata.as_bytes().first().copied() {
            Some(b'-' | b'd') => {}
            Some(b'l') => {
                let marker = format!("{path} -> ");
                let target = metadata
                    .find(&marker)
                    .map(|index| &metadata[index + marker.len()..])
                    .ok_or_else(|| Error {
                        message: format!(
                            "runtime archive contains an ambiguous symbolic link: {path}"
                        ),
                    })?;
                validate_archive_link(Path::new(path), Path::new(target))?;
            }
            Some(other) => {
                return Err(Error {
                    message: format!(
                        "runtime archive contains unsupported entry type {:?}: {path}",
                        other as char
                    ),
                });
            }
            None => {
                return Err(Error {
                    message: "runtime archive contains invalid metadata".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_archive_link(member: &Path, target: &Path) -> Result<(), Error> {
    if target.as_os_str().is_empty() || target.is_absolute() {
        return Err(Error {
            message: format!(
                "runtime archive symbolic link escapes its installation: {} -> {}",
                member.display(),
                target.display()
            ),
        });
    }

    let mut depth = member
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count();
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error {
                    message: format!(
                        "runtime archive symbolic link escapes its installation: {} -> {}",
                        member.display(),
                        target.display()
                    ),
                });
            }
        }
    }
    Ok(())
}

fn tar_output(artifact: &Path, mode: &str) -> Result<String, Error> {
    let output = Command::new("/usr/bin/tar")
        .arg(mode)
        .arg(artifact)
        .env("LC_ALL", "C")
        .output()
        .map_err(|error| Error {
            message: format!("cannot execute /usr/bin/tar: {error}"),
        })?;
    if !output.status.success() {
        return Err(Error {
            message: format!(
                "cannot inspect runtime archive: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    String::from_utf8(output.stdout).map_err(|_| Error {
        message: "runtime archive listing is not UTF-8".to_owned(),
    })
}

fn validate_extracted_tree(staging: &Path, entrypoint: &Path) -> Result<(), Error> {
    let staging = staging.canonicalize().map_err(|error| Error {
        message: format!("cannot resolve staging directory: {error}"),
    })?;
    validate_extracted_links(&staging, &staging)?;
    let resolved_entrypoint = staging.join(entrypoint);
    if !resolved_entrypoint.is_file() {
        return Err(Error {
            message: format!(
                "runtime entrypoint is missing after extraction: {}",
                entrypoint.display()
            ),
        });
    }
    let resolved_entrypoint = resolved_entrypoint.canonicalize().map_err(|error| Error {
        message: format!("cannot resolve runtime entrypoint: {error}"),
    })?;
    if !resolved_entrypoint.starts_with(&staging) {
        return Err(Error {
            message: "runtime entrypoint escapes the installation directory".to_owned(),
        });
    }
    Ok(())
}

fn validate_extracted_links(root: &Path, directory: &Path) -> Result<(), Error> {
    for entry in fs::read_dir(directory).map_err(|error| Error {
        message: format!("cannot inspect extracted runtime tree: {error}"),
    })? {
        let entry = entry.map_err(|error| Error {
            message: format!("cannot inspect extracted runtime entry: {error}"),
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| Error {
            message: format!("cannot inspect extracted runtime entry: {error}"),
        })?;
        if metadata.file_type().is_symlink() {
            let resolved = path.canonicalize().map_err(|error| Error {
                message: format!(
                    "cannot resolve extracted runtime symbolic link {}: {error}",
                    path.display()
                ),
            })?;
            if !resolved.starts_with(root) {
                return Err(Error {
                    message: format!(
                        "extracted runtime symbolic link escapes its installation: {}",
                        path.display()
                    ),
                });
            }
        } else if metadata.is_dir() {
            validate_extracted_links(root, &path)?;
        }
    }
    Ok(())
}

fn installation_matches(target: &Path, manifest: &RuntimeManifest) -> bool {
    fs::read_to_string(target.join(MARKER_FILE))
        .map(|contents| contents == marker_contents(manifest))
        .unwrap_or(false)
        && target.join(&manifest.entrypoint).is_file()
}

fn marker_contents(manifest: &RuntimeManifest) -> String {
    format!(
        "schema=1\nkind={}\nid={}\nsha256={}\nentrypoint={}\n",
        RuntimeKind::ManagedArchive.as_str(),
        manifest.id,
        manifest.sha256,
        manifest.entrypoint.display()
    )
}

fn staging_path(runtime_root: &Path, runtime_id: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    runtime_root.join(format!(
        ".install-{runtime_id}-{}-{nonce}",
        std::process::id()
    ))
}

fn validate_identifier(label: &str, value: &str) -> Result<(), Error> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(Error {
            message: format!(
                "manifest: {label} may only contain ASCII letters, digits, '.', '_' and '-'"
            ),
        });
    }
    Ok(())
}

fn validate_relative_path(label: &str, path: &Path) -> Result<(), Error> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(Error {
            message: format!("{label} must be a non-empty relative path"),
        });
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(Error {
            message: format!("{label} cannot escape the installation directory"),
        });
    }
    Ok(())
}

struct StagingGuard {
    path: PathBuf,
    committed: bool,
}

struct ProbeGuard {
    path: PathBuf,
}

struct ProbeServerGuard {
    entrypoint: PathBuf,
    prefix: PathBuf,
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

impl Drop for ProbeServerGuard {
    fn drop(&mut self) {
        stop_probe_server(&self.entrypoint, &self.prefix);
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn identifies_an_x86_64_only_runtime_as_translated_on_apple_silicon() {
        assert_eq!(
            classify_host_execution(
                &binary::BinaryKind::MachO(vec![binary::Architecture::X86_64]),
                "aarch64"
            ),
            HostExecution::TranslationRequired
        );
        assert_eq!(
            classify_host_execution(
                &binary::BinaryKind::MachO(vec![
                    binary::Architecture::X86_64,
                    binary::Architecture::Arm64,
                ]),
                "aarch64"
            ),
            HostExecution::UniversalNativeSlice
        );
    }

    #[test]
    fn readiness_never_promotes_an_unexecutable_native_host() {
        assert_eq!(
            classify_runtime_readiness(HostExecution::Native, false, false),
            RuntimeReadiness::NativeHostBlocked
        );
        assert_eq!(
            classify_runtime_readiness(HostExecution::Native, true, false),
            RuntimeReadiness::NativeHostNeedsX86Emulator
        );
        assert_eq!(
            classify_runtime_readiness(HostExecution::Native, true, true),
            RuntimeReadiness::NativeWin32Ready
        );
        assert_eq!(
            classify_runtime_readiness(HostExecution::TranslationRequired, false, true),
            RuntimeReadiness::TranslatedBootstrap
        );
    }
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            loop {
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos();
                let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "macroton-runtime-test-{}-{nonce}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create runtime test directory: {error}"),
                }
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn manifest(checksum: &str) -> RuntimeManifest {
        RuntimeManifest::parse(&format!(
            "schema=1\n\
             id=wine-wow64-test\n\
             family=wine-wow64\n\
             version=test\n\
             source=https://example.invalid/wine.tar\n\
             license=LGPL-2.1-or-later\n\
             license_url=https://www.winehq.org/license\n\
             sha256={checksum}\n\
             archive=tar\n\
             entrypoint=bin/wine"
        ))
        .unwrap()
    }

    #[test]
    fn rejects_invalid_manifest_identity() {
        let error = RuntimeManifest::parse(
            "schema=1\nid=wrong\nfamily=wine\nversion=1\nsource=https://example.invalid/x\nlicense=X\nlicense_url=https://example.invalid/license\nsha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\narchive=tar\nentrypoint=bin/wine",
        )
        .unwrap_err();
        assert!(error.message.contains("id must be wine-1"));
    }

    #[test]
    fn verifies_and_atomically_installs_archive() {
        let temp = TempDir::new();
        let payload = temp.0.join("payload");
        fs::create_dir_all(payload.join("bin")).unwrap();
        let entrypoint = payload.join("bin/wine");
        fs::copy("/bin/echo", &entrypoint).unwrap();
        let artifact = temp.0.join("runtime.tar");
        let status = Command::new("/usr/bin/tar")
            .args(["-cf"])
            .arg(&artifact)
            .arg("-C")
            .arg(&payload)
            .arg(".")
            .status()
            .unwrap();
        assert!(status.success());
        let checksum = sha256::to_hex(&sha256::digest_file(&artifact).unwrap());
        let manifest = manifest(&checksum);
        let runtime_root = temp.0.join("runtimes");

        assert_eq!(
            install(&manifest, &artifact, &runtime_root).unwrap(),
            InstallOutcome::Installed
        );
        assert!(runtime_root.join("wine-wow64-test/bin/wine").is_file());
        assert!(is_installed(&runtime_root, "wine-wow64-test"));
        assert_eq!(
            install(&manifest, &artifact, &runtime_root).unwrap(),
            InstallOutcome::AlreadyInstalled
        );
        assert_eq!(
            installed_runtime_ids(&runtime_root).unwrap(),
            ["wine-wow64-test"]
        );
    }

    #[test]
    fn release_tree_audit_requires_public_managed_runtime_and_rejects_commercial_paths() {
        let temp = TempDir::new();
        let payload = temp.0.join("payload");
        fs::create_dir_all(payload.join("bin")).unwrap();
        fs::create_dir_all(payload.join("lib/wine")).unwrap();
        let entrypoint = payload.join("bin/wine");
        fs::copy("/bin/echo", &entrypoint).unwrap();
        fs::write(payload.join("lib/wine/open-source.dylib"), b"fixture").unwrap();
        let artifact = temp.0.join("runtime.tar");
        let status = Command::new("/usr/bin/tar")
            .args(["-cf"])
            .arg(&artifact)
            .arg("-C")
            .arg(&payload)
            .arg(".")
            .status()
            .unwrap();
        assert!(status.success());
        let checksum = sha256::to_hex(&sha256::digest_file(&artifact).unwrap());
        let manifest = manifest(&checksum);
        let runtime_root = temp.0.join("runtimes");
        install(&manifest, &artifact, &runtime_root).unwrap();
        let audit = audit_release_tree(&manifest, &runtime_root).unwrap();
        assert_eq!(audit.runtime_id, "wine-wow64-test");
        assert!(audit.entries_checked >= 4);
        assert!(audit.entrypoint_dependencies_checked > 0);

        fs::write(
            runtime_root.join("wine-wow64-test/lib/wine/D3DMetal.dylib"),
            b"forbidden fixture",
        )
        .unwrap();
        assert!(
            audit_release_tree(&manifest, &runtime_root)
                .unwrap_err()
                .message
                .contains("forbidden commercial-runtime identifier")
        );

        let mut local = manifest.clone();
        local.source = "file:///private/tmp/runtime.tar".to_owned();
        assert!(
            audit_release_tree(&local, &runtime_root)
                .unwrap_err()
                .message
                .contains("https://")
        );
    }

    #[test]
    fn rejects_checksum_mismatch_before_installing() {
        let temp = TempDir::new();
        let artifact = temp.0.join("runtime.tar");
        fs::write(&artifact, b"not an archive").unwrap();
        let error = install(
            &manifest(&"00".repeat(32)),
            &artifact,
            &temp.0.join("runtimes"),
        )
        .unwrap_err();
        assert!(error.message.contains("checksum mismatch"));
        assert!(!temp.0.join("runtimes").exists());
    }

    #[test]
    fn rejects_escaping_archive_paths() {
        assert!(validate_relative_path("member", Path::new("../escape")).is_err());
        assert!(validate_relative_path("member", Path::new("/absolute")).is_err());
        assert!(validate_relative_path("member", Path::new("safe/bin/wine")).is_ok());
    }

    #[test]
    fn allows_internal_symbolic_links_and_rejects_escaping_links() {
        let temp = TempDir::new();
        let payload = temp.0.join("payload");
        fs::create_dir_all(payload.join("bin")).unwrap();
        fs::write(payload.join("bin/wine-real"), "#!/bin/sh\n").unwrap();
        symlink("wine-real", payload.join("bin/wine")).unwrap();
        let artifact = temp.0.join("runtime.tar");
        let status = Command::new("/usr/bin/tar")
            .args(["-cf"])
            .arg(&artifact)
            .arg("-C")
            .arg(&payload)
            .arg(".")
            .status()
            .unwrap();
        assert!(status.success());
        let checksum = sha256::to_hex(&sha256::digest_file(&artifact).unwrap());
        assert_eq!(
            install(&manifest(&checksum), &artifact, &temp.0.join("runtimes")).unwrap(),
            InstallOutcome::Installed
        );

        assert!(validate_archive_link(Path::new("bin/wine"), Path::new("wine-real")).is_ok());
        assert!(validate_archive_link(Path::new("bin/wine"), Path::new("../../escape")).is_err());
        assert!(validate_archive_link(Path::new("bin/wine"), Path::new("/tmp")).is_err());
    }

    #[test]
    fn registers_external_runtime_and_detects_binary_drift() {
        let temp = TempDir::new();
        let entrypoint = temp.0.join("wine");
        fs::write(&entrypoint, "#!/bin/sh\necho wine-test\n").unwrap();
        let mut permissions = fs::metadata(&entrypoint).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&entrypoint, permissions).unwrap();
        let runtime_root = temp.0.join("runtimes");

        assert_eq!(
            register_external("wine-test", &entrypoint, &runtime_root).unwrap(),
            InstallOutcome::Installed
        );
        let installed = resolve_installed(&runtime_root, "wine-test").unwrap();
        assert_eq!(installed.kind, RuntimeKind::ExternalExecutable);
        assert_eq!(installed.entrypoint, entrypoint.canonicalize().unwrap());
        assert_eq!(
            register_external("wine-test", &entrypoint, &runtime_root).unwrap(),
            InstallOutcome::AlreadyInstalled
        );

        fs::write(&entrypoint, "#!/bin/sh\necho changed\n").unwrap();
        let drift = resolve_installed(&runtime_root, "wine-test").unwrap_err();
        assert!(drift.message.contains("changed since registration"));
    }

    #[test]
    fn external_runtime_verifies_composed_component_fingerprints() {
        let temp = TempDir::new();
        let runtime = temp.0.join("wine-root");
        let entrypoint = runtime.join("bin/wine");
        let component = runtime.join("lib/wine/i386-windows/d3d10core.dll");
        fs::create_dir_all(entrypoint.parent().unwrap()).unwrap();
        fs::create_dir_all(component.parent().unwrap()).unwrap();
        fs::create_dir_all(runtime.join("share/macroton/components")).unwrap();
        fs::write(&entrypoint, "#!/bin/sh\n").unwrap();
        fs::write(&component, b"dxmt-component").unwrap();
        let mut permissions = fs::metadata(&entrypoint).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&entrypoint, permissions).unwrap();
        let checksum = sha256::to_hex(&sha256::digest_file(&component).unwrap());
        fs::write(
            runtime.join("share/macroton/components/dxmt-test.conf"),
            format!("schema=1\nid=dxmt-test\nfile.i386-windows/d3d10core.dll={checksum}\n"),
        )
        .unwrap();
        let runtime_root = temp.0.join("runtimes");

        register_external("wine-dxmt-test", &entrypoint, &runtime_root).unwrap();
        resolve_installed(&runtime_root, "wine-dxmt-test").unwrap();

        fs::write(&component, b"replaced-component").unwrap();
        let drift = resolve_installed(&runtime_root, "wine-dxmt-test").unwrap_err();
        assert!(drift.message.contains("runtime component changed"));
    }
}
