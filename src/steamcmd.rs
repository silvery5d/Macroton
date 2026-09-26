use crate::binary::{
    self, BinaryKind, PeArchitecture, PeCompatibilityEvidence, PeGraphicsEvidence,
};
use crate::profile::CompatibilityProfile;
use crate::runtime;
use crate::sha256;
use crate::steam;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginResult {
    pub app_id: u32,
    pub prefix: PathBuf,
    pub executable: PathBuf,
    pub runtime_id: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquireResult {
    pub app_id: u32,
    pub prefix: PathBuf,
    pub payload: PathBuf,
    pub executable: PathBuf,
    pub architecture: PeArchitecture,
    pub graphics: PeGraphicsEvidence,
    pub compatibility: PeCompatibilityEvidence,
    pub executable_sha256: String,
    pub marker: PathBuf,
    pub manifest: PathBuf,
    pub build_id: Option<u64>,
    pub installed_depots: BTreeMap<u32, u64>,
    pub manifest_matches_profile: bool,
    pub runtime_id: String,
    pub exit_code: Option<i32>,
}

pub struct AcquireOptions<'a> {
    pub app_id: u32,
    pub relative_executable: &'a Path,
    pub profile_fingerprint: &'a str,
    pub expected_build_id: Option<u64>,
    pub expected_depots: &'a BTreeMap<u32, u64>,
    pub tool_root: &'a Path,
    pub data_root: &'a Path,
    pub runtime_root: &'a Path,
    pub runtime_id: &'a str,
    pub account: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcquisitionProvenance<'a> {
    pub runtime_id: &'a str,
    pub runtime_fingerprint_sha256: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionState {
    Missing,
    Partial,
    Unrecorded,
    Ready,
    Drifted,
    Invalid,
}

impl AcquisitionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Partial => "partial",
            Self::Unrecorded => "unrecorded",
            Self::Ready => "ready",
            Self::Drifted => "drifted",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquisitionStatus {
    pub app_id: u32,
    pub state: AcquisitionState,
    pub executable: PathBuf,
    pub marker: PathBuf,
    pub architecture: Option<PeArchitecture>,
    pub graphics: Option<PeGraphicsEvidence>,
    pub compatibility: Option<PeCompatibilityEvidence>,
    pub executable_sha256: Option<String>,
    pub drift_reasons: Vec<String>,
    pub detail: String,
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

pub fn validate_tool_root(tool_root: &Path) -> Result<PathBuf, Error> {
    let executable = tool_root.join("steamcmd.exe");
    if !executable.is_file() {
        return Err(Error {
            message: format!(
                "SteamCMD executable is missing: {}; extract Valve's Windows SteamCMD archive into --tool-root",
                executable.display()
            ),
        });
    }
    let kind = binary::inspect(&executable).map_err(|error| Error {
        message: format!("cannot inspect {}: {error}", executable.display()),
    })?;
    if kind != BinaryKind::PortableExecutable {
        return Err(Error {
            message: format!(
                "SteamCMD is not a Windows PE executable: {}",
                executable.display()
            ),
        });
    }
    let architecture = binary::inspect_pe_architecture(&executable).map_err(|error| Error {
        message: format!("cannot inspect SteamCMD PE header: {error}"),
    })?;
    if !matches!(
        architecture,
        Some(PeArchitecture::I386 | PeArchitecture::X86_64)
    ) {
        return Err(Error {
            message: format!(
                "SteamCMD must be a Valve Windows i386 or x86_64 PE executable; observed {architecture:?}: {}",
                executable.display()
            ),
        });
    }
    Ok(executable)
}

pub fn interactive_login(
    app_id: u32,
    tool_root: &Path,
    data_root: &Path,
    runtime_root: &Path,
    runtime_id: &str,
) -> Result<LoginResult, Error> {
    let executable = validate_tool_root(tool_root)?;
    let installed =
        runtime::resolve_installed(runtime_root, runtime_id).map_err(|error| Error {
            message: error.to_string(),
        })?;
    let prefix = prefix_path(app_id, data_root);
    let status = run_interactive(&installed.entrypoint, &executable, tool_root, &prefix, &[])?;

    Ok(LoginResult {
        app_id,
        prefix,
        executable,
        runtime_id: installed.id,
        exit_code: status.code(),
    })
}

pub fn interactive_acquire(options: &AcquireOptions<'_>) -> Result<AcquireResult, Error> {
    let AcquireOptions {
        app_id,
        relative_executable,
        profile_fingerprint,
        expected_build_id,
        expected_depots,
        tool_root,
        data_root,
        runtime_root,
        runtime_id,
        account,
    } = options;
    if relative_executable.is_absolute()
        || relative_executable
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(Error {
            message: "acquisition executable must be a safe relative path".to_owned(),
        });
    }
    let executable = validate_tool_root(tool_root)?;
    let installed =
        runtime::resolve_installed(runtime_root, runtime_id).map_err(|error| Error {
            message: error.to_string(),
        })?;
    let prefix = prefix_path(*app_id, data_root);
    let payload = data_root
        .join("games")
        .join(app_id.to_string())
        .join("steamcmd-payload");
    fs::create_dir_all(&payload).map_err(|error| Error {
        message: format!(
            "cannot create SteamCMD payload directory {}: {error}",
            payload.display()
        ),
    })?;
    let payload = payload.canonicalize().map_err(|error| Error {
        message: format!(
            "cannot canonicalize SteamCMD payload directory {}: {error}",
            payload.display()
        ),
    })?;
    let windows_payload = wine_z_path(&payload)?;
    let arguments = acquire_arguments(*app_id, &windows_payload, *account)?;
    let status = run_interactive(
        &installed.entrypoint,
        &executable,
        tool_root,
        &prefix,
        &arguments,
    )?;
    if !status.success() {
        return Err(Error {
            message: match status.code() {
                Some(code) => format!("SteamCMD acquisition exited with status {code}"),
                None => "SteamCMD acquisition terminated by signal".to_owned(),
            },
        });
    }
    let game_executable = payload.join(relative_executable);
    if !game_executable.is_file() {
        return Err(Error {
            message: format!(
                "SteamCMD exited without the expected game executable {}; the authenticated app_update did not complete",
                game_executable.display()
            ),
        });
    }
    if binary::inspect(&game_executable).map_err(|error| Error {
        message: format!("cannot inspect acquired game executable: {error}"),
    })? != BinaryKind::PortableExecutable
    {
        return Err(Error {
            message: format!(
                "acquired game entrypoint is not a Windows PE executable: {}",
                game_executable.display()
            ),
        });
    }
    let architecture = binary::inspect_pe_architecture(&game_executable)
        .map_err(|error| Error {
            message: format!("cannot inspect acquired game PE architecture: {error}"),
        })?
        .ok_or_else(|| Error {
            message: "acquired game executable has no valid PE architecture".to_owned(),
        })?;
    let graphics = binary::inspect_pe_graphics(&game_executable).map_err(|error| Error {
        message: format!("cannot inspect acquired game graphics imports: {error}"),
    })?;
    let compatibility =
        binary::inspect_pe_compatibility(&game_executable).map_err(|error| Error {
            message: format!("cannot inspect acquired game compatibility imports: {error}"),
        })?;
    let executable_sha256 = sha256::to_hex(&sha256::digest_file(&game_executable).map_err(
        |error| Error {
            message: format!("cannot hash acquired game executable: {error}"),
        },
    )?);
    let steamcmd_sha256 =
        sha256::to_hex(&sha256::digest_file(&executable).map_err(|error| Error {
            message: format!("cannot hash SteamCMD executable: {error}"),
        })?);
    let source_manifest = locate_app_manifest(tool_root, &payload, *app_id).ok_or_else(|| Error {
        message: format!(
            "SteamCMD produced a game executable but no appmanifest_{app_id}.acf in a supported location"
        ),
    })?;
    let manifest_evidence =
        steam::inspect_app_manifest(&source_manifest).map_err(|error| Error {
            message: format!("cannot inspect SteamCMD app manifest: {error}"),
        })?;
    if manifest_evidence.app_id != *app_id {
        return Err(Error {
            message: format!(
                "SteamCMD manifest AppID {} does not match requested AppID {app_id}",
                manifest_evidence.app_id
            ),
        });
    }
    let game_root = data_root.join("games").join(app_id.to_string());
    let manifest = game_root.join(format!("appmanifest_{app_id}.acf"));
    write_manifest_snapshot(&source_manifest, &manifest)?;
    let manifest_sha256 =
        sha256::to_hex(&sha256::digest_file(&manifest).map_err(|error| Error {
            message: format!("cannot hash SteamCMD app manifest snapshot: {error}"),
        })?);
    let manifest_matches_profile = manifest_evidence.build_id == *expected_build_id
        && manifest_evidence.installed_depots == **expected_depots;
    let marker = game_root.join("steamcmd-acquisition.json");
    let marker_contents = acquisition_marker(
        *app_id,
        relative_executable,
        architecture,
        &graphics,
        &compatibility,
        &executable_sha256,
        profile_fingerprint,
        &installed.id,
        &installed.fingerprint_sha256,
        &steamcmd_sha256,
        &manifest_sha256,
        manifest_evidence.build_id,
        &manifest_evidence.installed_depots,
    );
    write_marker(&marker, &marker_contents)?;
    Ok(AcquireResult {
        app_id: *app_id,
        prefix,
        payload,
        executable: game_executable,
        architecture,
        graphics,
        compatibility,
        executable_sha256,
        marker,
        manifest,
        build_id: manifest_evidence.build_id,
        installed_depots: manifest_evidence.installed_depots,
        manifest_matches_profile,
        runtime_id: installed.id,
        exit_code: status.code(),
    })
}

fn acquire_arguments(
    app_id: u32,
    windows_payload: &str,
    account: Option<&str>,
) -> Result<Vec<String>, Error> {
    let mut arguments = vec![
        "+@sSteamCmdForcePlatformType".to_owned(),
        "windows".to_owned(),
        "+force_install_dir".to_owned(),
        windows_payload.to_owned(),
    ];
    if let Some(account) = account {
        if account.is_empty() || account.chars().any(|character| character.is_whitespace()) {
            return Err(Error {
                message: "Steam account name must be non-empty and contain no whitespace"
                    .to_owned(),
            });
        }
        arguments.extend([
            "+login".to_owned(),
            account.to_owned(),
            "+app_update".to_owned(),
            app_id.to_string(),
            "validate".to_owned(),
            "+quit".to_owned(),
        ]);
    }
    Ok(arguments)
}

pub fn inspect_acquisition(
    app_id: u32,
    relative_executable: &Path,
    profile_fingerprint: &str,
    expected_build_id: Option<u64>,
    expected_depots: &BTreeMap<u32, u64>,
    expected_provenance: AcquisitionProvenance<'_>,
    data_root: &Path,
) -> Result<AcquisitionStatus, Error> {
    let game_root = data_root.join("games").join(app_id.to_string());
    let payload = game_root.join("steamcmd-payload");
    let executable = payload.join(relative_executable);
    let marker = game_root.join("steamcmd-acquisition.json");
    if !payload.exists() {
        return Ok(acquisition_status(
            app_id,
            AcquisitionState::Missing,
            executable,
            marker,
            None,
            None,
            None,
            None,
            "SteamCMD payload directory is absent",
        ));
    }
    if !executable.is_file() {
        return Ok(acquisition_status(
            app_id,
            AcquisitionState::Partial,
            executable,
            marker,
            None,
            None,
            None,
            None,
            "payload directory exists but the profile entrypoint is absent",
        ));
    }
    if binary::inspect(&executable).map_err(|error| Error {
        message: format!("cannot inspect acquired executable: {error}"),
    })? != BinaryKind::PortableExecutable
    {
        return Ok(acquisition_status(
            app_id,
            AcquisitionState::Invalid,
            executable,
            marker,
            None,
            None,
            None,
            None,
            "profile entrypoint is not a Windows PE executable",
        ));
    }
    let architecture = binary::inspect_pe_architecture(&executable).map_err(|error| Error {
        message: format!("cannot inspect acquired executable architecture: {error}"),
    })?;
    let graphics = binary::inspect_pe_graphics(&executable).map_err(|error| Error {
        message: format!("cannot inspect acquired executable graphics imports: {error}"),
    })?;
    let compatibility = binary::inspect_pe_compatibility(&executable).map_err(|error| Error {
        message: format!("cannot inspect acquired executable compatibility imports: {error}"),
    })?;
    let executable_sha256 =
        sha256::to_hex(&sha256::digest_file(&executable).map_err(|error| Error {
            message: format!("cannot hash acquired executable: {error}"),
        })?);
    if !marker.is_file() {
        return Ok(acquisition_status(
            app_id,
            AcquisitionState::Unrecorded,
            executable,
            marker,
            architecture,
            Some(graphics),
            Some(compatibility),
            Some(executable_sha256),
            "real PE entrypoint exists but no successful acquisition marker is recorded",
        ));
    }
    let marker_contents = fs::read_to_string(&marker).map_err(|error| Error {
        message: format!(
            "cannot read acquisition marker {}: {error}",
            marker.display()
        ),
    })?;
    let manifest = game_root.join(format!("appmanifest_{app_id}.acf"));
    let manifest_evidence = manifest
        .is_file()
        .then(|| steam::inspect_app_manifest(&manifest))
        .transpose()
        .map_err(|error| Error {
            message: format!("cannot inspect SteamCMD app manifest snapshot: {error}"),
        })?;
    let manifest_sha256 = manifest
        .is_file()
        .then(|| sha256::digest_file(&manifest))
        .transpose()
        .map_err(|error| Error {
            message: format!("cannot hash SteamCMD app manifest snapshot: {error}"),
        })?
        .map(|digest| sha256::to_hex(&digest));
    let expected_architecture = architecture.map(pe_architecture_name);
    let expected_graphics = graphics
        .apis
        .iter()
        .map(|api| format!("\"{}\"", api.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let expected_app_id = app_id.to_string();
    let expected_profile = format!("\"{}\"", json_escape(profile_fingerprint));
    let expected_executable = format!(
        "\"{}\"",
        json_escape(&relative_executable.to_string_lossy())
    );
    let expected_executable_sha256 = format!("\"{executable_sha256}\"");
    let expected_graphics = format!("[{expected_graphics}]");
    let expected_input = marker_api_array(compatibility.input.iter().map(|api| api.as_str()));
    let expected_audio = marker_api_array(compatibility.audio.iter().map(|api| api.as_str()));
    let expected_prerequisites = marker_api_array(
        compatibility
            .prerequisites
            .iter()
            .map(|prerequisite| prerequisite.as_str()),
    );
    let manifest_matches = manifest_evidence.as_ref().is_some_and(|manifest| {
        manifest.app_id == app_id
            && manifest.build_id == expected_build_id
            && manifest.installed_depots == *expected_depots
    });
    let expected_build_id = expected_build_id
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_owned());
    let expected_depots = expected_depots
        .iter()
        .map(|(id, manifest)| format!("\"{id}:{manifest}\""))
        .collect::<Vec<_>>()
        .join(",");
    let expected_depots = format!("[{expected_depots}]");
    let expected_runtime_id = format!("\"{}\"", json_escape(expected_provenance.runtime_id));
    let expected_runtime_sha256 = expected_provenance
        .runtime_fingerprint_sha256
        .map(|value| format!("\"{}\"", json_escape(value)));
    let expected_manifest_sha256 = manifest_sha256.as_ref().map(|value| format!("\"{value}\""));
    let marker_schema_matches = marker_field(&marker_contents, "schema").as_deref() == Some("2");
    let profile_matches = marker_field(&marker_contents, "appid").as_deref()
        == Some(expected_app_id.as_str())
        && marker_field(&marker_contents, "profile_sha256").as_deref()
            == Some(expected_profile.as_str());
    let executable_matches = marker_field(&marker_contents, "executable").as_deref()
        == Some(expected_executable.as_str())
        && marker_field(&marker_contents, "executable_sha256").as_deref()
            == Some(expected_executable_sha256.as_str())
        && expected_architecture.is_some_and(|value| {
            let expected = format!("\"{value}\"");
            marker_field(&marker_contents, "architecture").as_deref() == Some(expected.as_str())
        })
        && marker_field(&marker_contents, "graphics_apis").as_deref()
            == Some(expected_graphics.as_str())
        && marker_field(&marker_contents, "input_apis").as_deref() == Some(expected_input.as_str())
        && marker_field(&marker_contents, "audio_apis").as_deref() == Some(expected_audio.as_str())
        && marker_field(&marker_contents, "prerequisites").as_deref()
            == Some(expected_prerequisites.as_str());
    let runtime_matches = marker_field(&marker_contents, "runtime_id").as_deref()
        == Some(expected_runtime_id.as_str())
        && expected_runtime_sha256.as_deref().is_some_and(|expected| {
            marker_field(&marker_contents, "runtime_sha256").as_deref() == Some(expected)
        });
    let manifest_marker_matches = marker_field(&marker_contents, "build_id").as_deref()
        == Some(expected_build_id.as_str())
        && marker_field(&marker_contents, "installed_depots").as_deref()
            == Some(expected_depots.as_str())
        && expected_manifest_sha256.as_deref().is_some_and(|expected| {
            marker_field(&marker_contents, "manifest_sha256").as_deref() == Some(expected)
        });
    let mut drift_reasons = Vec::new();
    if !marker_schema_matches {
        drift_reasons.push("marker-schema".to_owned());
    }
    if !profile_matches {
        drift_reasons.push("profile".to_owned());
    }
    if !executable_matches {
        drift_reasons.push("executable".to_owned());
    }
    if !runtime_matches {
        drift_reasons.push("runtime".to_owned());
    }
    if !manifest_matches || !manifest_marker_matches {
        drift_reasons.push("manifest".to_owned());
    }
    let (state, detail) = if drift_reasons.is_empty() {
        (
            AcquisitionState::Ready,
            "payload, runtime and manifest snapshot match the recorded acquisition evidence"
                .to_owned(),
        )
    } else {
        (
            AcquisitionState::Drifted,
            format!(
                "recorded acquisition evidence differs in: {}",
                drift_reasons.join(",")
            ),
        )
    };
    let mut status = acquisition_status(
        app_id,
        state,
        executable,
        marker,
        architecture,
        Some(graphics),
        Some(compatibility),
        Some(executable_sha256),
        &detail,
    );
    status.drift_reasons = drift_reasons;
    Ok(status)
}

pub fn acquired_game(
    status: &AcquisitionStatus,
    profile: &CompatibilityProfile,
) -> Result<steam::SteamGame, Error> {
    if status.app_id != profile.app_id {
        return Err(Error {
            message: format!(
                "acquisition AppID {} does not match profile AppID {}",
                status.app_id, profile.app_id
            ),
        });
    }
    if status.state != AcquisitionState::Ready {
        return Err(Error {
            message: format!(
                "SteamCMD acquisition is {}; only ready evidence can become a launch plan",
                status.state.as_str()
            ),
        });
    }
    let installed_depots = profile
        .windows_depots
        .iter()
        .map(|depot| {
            depot
                .manifest_id
                .map(|manifest| (depot.id, manifest))
                .ok_or_else(|| Error {
                    message: format!("profile depot {} has no pinned manifest", depot.id),
                })
        })
        .collect::<Result<BTreeMap<_, _>, Error>>()?;
    let payload = status
        .marker
        .parent()
        .ok_or_else(|| Error {
            message: "acquisition marker has no game root".to_owned(),
        })?
        .join("steamcmd-payload");
    Ok(steam::SteamGame {
        app_id: status.app_id,
        name: profile
            .steam_install_dir
            .clone()
            .unwrap_or_else(|| format!("Steam App {}", status.app_id)),
        install_dir: payload,
        manifest_path: status.marker.clone(),
        state_flags: Some(4),
        build_id: profile.windows_build_id.clone(),
        installed_depots,
        classification: steam::Classification::WindowsPayload,
        binaries: vec![steam::ObservedBinary {
            relative_path: profile.executable.clone(),
            kind: BinaryKind::PortableExecutable,
            executable: true,
            pe_architecture: status.architecture,
        }],
        inspection_truncated: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn acquisition_status(
    app_id: u32,
    state: AcquisitionState,
    executable: PathBuf,
    marker: PathBuf,
    architecture: Option<PeArchitecture>,
    graphics: Option<PeGraphicsEvidence>,
    compatibility: Option<PeCompatibilityEvidence>,
    executable_sha256: Option<String>,
    detail: &str,
) -> AcquisitionStatus {
    AcquisitionStatus {
        app_id,
        state,
        executable,
        marker,
        architecture,
        graphics,
        compatibility,
        executable_sha256,
        drift_reasons: Vec::new(),
        detail: detail.to_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
fn acquisition_marker(
    app_id: u32,
    relative_executable: &Path,
    architecture: PeArchitecture,
    graphics: &PeGraphicsEvidence,
    compatibility: &PeCompatibilityEvidence,
    executable_sha256: &str,
    profile_fingerprint: &str,
    runtime_id: &str,
    runtime_fingerprint: &str,
    steamcmd_sha256: &str,
    manifest_sha256: &str,
    build_id: Option<u64>,
    installed_depots: &BTreeMap<u32, u64>,
) -> String {
    let architecture = pe_architecture_name(architecture);
    let graphics = graphics
        .apis
        .iter()
        .map(|api| format!("\"{}\"", api.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let build_id = build_id
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_owned());
    let installed_depots = installed_depots
        .iter()
        .map(|(id, manifest)| format!("\"{id}:{manifest}\""))
        .collect::<Vec<_>>()
        .join(",");
    let input = marker_api_array(compatibility.input.iter().map(|api| api.as_str()));
    let audio = marker_api_array(compatibility.audio.iter().map(|api| api.as_str()));
    let prerequisites = marker_api_array(
        compatibility
            .prerequisites
            .iter()
            .map(|prerequisite| prerequisite.as_str()),
    );
    format!(
        "{{\n  \"schema\": 2,\n  \"appid\": {app_id},\n  \"profile_sha256\": \"{}\",\n  \"runtime_id\": \"{}\",\n  \"runtime_sha256\": \"{}\",\n  \"steamcmd_sha256\": \"{}\",\n  \"manifest_sha256\": \"{}\",\n  \"build_id\": {build_id},\n  \"installed_depots\": [{installed_depots}],\n  \"executable\": \"{}\",\n  \"executable_sha256\": \"{}\",\n  \"architecture\": \"{}\",\n  \"graphics_apis\": [{}],\n  \"input_apis\": {input},\n  \"audio_apis\": {audio},\n  \"prerequisites\": {prerequisites}\n}}\n",
        json_escape(profile_fingerprint),
        json_escape(runtime_id),
        json_escape(runtime_fingerprint),
        json_escape(steamcmd_sha256),
        json_escape(manifest_sha256),
        json_escape(&relative_executable.to_string_lossy()),
        json_escape(executable_sha256),
        architecture,
        graphics,
    )
}

fn marker_api_array<'a>(values: impl Iterator<Item = &'a str>) -> String {
    format!(
        "[{}]",
        values
            .map(|value| format!("\"{}\"", json_escape(value)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn pe_architecture_name(architecture: PeArchitecture) -> String {
    match architecture {
        PeArchitecture::I386 => "i386".to_owned(),
        PeArchitecture::X86_64 => "x86_64".to_owned(),
        PeArchitecture::Arm64 => "arm64".to_owned(),
        PeArchitecture::Other(machine) => format!("other-{machine:#06x}"),
    }
}

fn write_marker(path: &Path, contents: &str) -> Result<(), Error> {
    let parent = path.parent().ok_or_else(|| Error {
        message: format!("acquisition marker has no parent: {}", path.display()),
    })?;
    fs::create_dir_all(parent).map_err(|error| Error {
        message: format!("cannot create acquisition marker directory: {error}"),
    })?;
    let temporary = parent.join(format!(".steamcmd-acquisition-{}.tmp", std::process::id()));
    fs::write(&temporary, contents).map_err(|error| Error {
        message: format!("cannot write acquisition marker: {error}"),
    })?;
    fs::rename(&temporary, path).map_err(|error| Error {
        message: format!("cannot commit acquisition marker: {error}"),
    })
}

fn write_manifest_snapshot(source: &Path, destination: &Path) -> Result<(), Error> {
    let contents = fs::read(source).map_err(|error| Error {
        message: format!("cannot read SteamCMD app manifest for snapshot: {error}"),
    })?;
    let parent = destination.parent().ok_or_else(|| Error {
        message: "SteamCMD app manifest snapshot has no parent".to_owned(),
    })?;
    fs::create_dir_all(parent).map_err(|error| Error {
        message: format!("cannot create SteamCMD manifest snapshot directory: {error}"),
    })?;
    let temporary = parent.join(format!(".appmanifest-{}.tmp", std::process::id()));
    fs::write(&temporary, contents).map_err(|error| Error {
        message: format!("cannot write SteamCMD app manifest snapshot: {error}"),
    })?;
    fs::rename(&temporary, destination).map_err(|error| Error {
        message: format!("cannot commit SteamCMD app manifest snapshot: {error}"),
    })
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn marker_field(contents: &str, key: &str) -> Option<String> {
    let prefix = format!("\"{key}\":");
    let mut values = contents.lines().filter_map(|line| {
        let line = line.trim();
        line.strip_prefix(&prefix)
            .map(|value| value.trim().trim_end_matches(',').to_owned())
    });
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

fn prefix_path(app_id: u32, data_root: &Path) -> PathBuf {
    data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix")
}

fn locate_app_manifest(tool_root: &Path, payload: &Path, app_id: u32) -> Option<PathBuf> {
    let file_name = format!("appmanifest_{app_id}.acf");
    let mut candidates = vec![
        tool_root.join("steamapps").join(&file_name),
        payload.join("steamapps").join(&file_name),
    ];
    if let Some(parent) = payload.parent() {
        candidates.push(parent.join("steamapps").join(&file_name));
    }
    candidates.into_iter().find(|path| path.is_file())
}

fn run_interactive(
    runtime_entrypoint: &Path,
    executable: &Path,
    tool_root: &Path,
    prefix: &Path,
    arguments: &[String],
) -> Result<std::process::ExitStatus, Error> {
    if !std::io::stdin().is_terminal() {
        return Err(Error {
            message: "interactive SteamCMD requires a real TTY; open a visible Terminal or use scripts/open-steamcmd-login-terminal.sh --print-command".to_owned(),
        });
    }
    fs::create_dir_all(prefix).map_err(|error| Error {
        message: format!(
            "cannot create SteamCMD prefix {}: {error}",
            prefix.display()
        ),
    })?;
    Command::new(runtime_entrypoint)
        .arg(executable)
        .args(arguments)
        .env("WINEPREFIX", prefix)
        .env("WINEDEBUG", "-all")
        .current_dir(tool_root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| Error {
            message: format!("cannot start interactive SteamCMD session: {error}"),
        })
}

fn wine_z_path(path: &Path) -> Result<String, Error> {
    if !path.is_absolute() {
        return Err(Error {
            message: "SteamCMD payload directory must be absolute".to_owned(),
        });
    }
    let host = path.to_string_lossy();
    if host
        .chars()
        .any(|character| character.is_control() || character == '\\')
    {
        return Err(Error {
            message: "SteamCMD payload directory contains an unsupported character".to_owned(),
        });
    }
    Ok(format!("Z:{}", host.replace('/', "\\")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "macroton-steamcmd-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn rejects_missing_executable() {
        let root = test_root("missing");
        fs::create_dir_all(&root).unwrap();
        let error = validate_tool_root(&root).unwrap_err();
        assert!(error.to_string().contains("is missing"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn queued_acquire_commands_never_contain_a_password() {
        let arguments = acquire_arguments(17460, r"Z:\payload", Some("silvery5d")).unwrap();
        assert_eq!(
            arguments,
            vec![
                "+@sSteamCmdForcePlatformType",
                "windows",
                "+force_install_dir",
                r"Z:\payload",
                "+login",
                "silvery5d",
                "+app_update",
                "17460",
                "validate",
                "+quit",
            ]
        );
        assert!(!arguments.iter().any(|argument| argument == "password"));
    }

    #[test]
    fn queued_acquire_commands_reject_whitespace_account_names() {
        let error = acquire_arguments(17460, r"Z:\payload", Some("bad name")).unwrap_err();
        assert!(error.message.contains("no whitespace"));
    }

    #[test]
    fn rejects_non_pe_executable() {
        let root = test_root("not-pe");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("steamcmd.exe"), b"not a PE file").unwrap();
        let error = validate_tool_root(&root).unwrap_err();
        assert!(error.to_string().contains("not a Windows PE"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_pe32_i386_executable() {
        let root = test_root("pe32");
        fs::create_dir_all(&root).unwrap();
        let mut bytes = vec![0_u8; 0x200];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x014c_u16.to_le_bytes());
        bytes[0x94..0x96].copy_from_slice(&0xe0_u16.to_le_bytes());
        bytes[0x98..0x9a].copy_from_slice(&0x10b_u16.to_le_bytes());
        fs::write(root.join("steamcmd.exe"), bytes).unwrap();
        assert_eq!(
            validate_tool_root(&root).unwrap(),
            root.join("steamcmd.exe")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_self_updated_x86_64_executable() {
        let root = test_root("pe64");
        fs::create_dir_all(&root).unwrap();
        let mut bytes = vec![0_u8; 0x86];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x8664_u16.to_le_bytes());
        fs::write(root.join("steamcmd.exe"), bytes).unwrap();
        assert_eq!(
            validate_tool_root(&root).unwrap(),
            root.join("steamcmd.exe")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn converts_absolute_host_path_to_wine_z_drive() {
        assert_eq!(
            wine_z_path(Path::new("/private/tmp/Game Files")).unwrap(),
            r"Z:\private\tmp\Game Files"
        );
        assert!(wine_z_path(Path::new("relative/path")).is_err());
    }

    #[test]
    fn acquisition_marker_is_redacted_and_reproducible() {
        let mut apis = std::collections::BTreeSet::new();
        apis.insert(crate::binary::PeGraphicsApi::D3d9);
        let graphics = PeGraphicsEvidence {
            apis,
            inspected_files: vec!["game.exe".to_owned()],
            imported_libraries: vec!["d3d9.dll".to_owned()],
        };
        let compatibility = PeCompatibilityEvidence {
            graphics: graphics.apis.clone(),
            input: std::collections::BTreeSet::new(),
            audio: std::collections::BTreeSet::new(),
            prerequisites: std::collections::BTreeSet::new(),
            imported_libraries: graphics.imported_libraries.clone(),
            local_libraries: Vec::new(),
        };
        let marker = acquisition_marker(
            42,
            Path::new("bin/game.exe"),
            PeArchitecture::I386,
            &graphics,
            &compatibility,
            "exe-hash",
            "profile-hash",
            "wine-id",
            "wine-hash",
            "steamcmd-hash",
            "manifest-hash",
            Some(9),
            &BTreeMap::from([(43, 100)]),
        );
        assert!(marker.contains("\"appid\": 42"));
        assert!(marker.contains("\"schema\": 2"));
        assert!(marker.contains("\"architecture\": \"i386\""));
        assert!(marker.contains("\"graphics_apis\": [\"d3d9\"]"));
        assert!(marker.contains("\"installed_depots\": [\"43:100\"]"));
        assert!(!marker.contains("account"));
    }

    #[test]
    fn acquisition_status_distinguishes_missing_and_partial_payloads() {
        let root = test_root("status");
        let expected_depots = BTreeMap::from([(43, 100)]);
        let missing = inspect_acquisition(
            42,
            Path::new("bin/game.exe"),
            "profile",
            Some(9),
            &expected_depots,
            AcquisitionProvenance {
                runtime_id: "wine-id",
                runtime_fingerprint_sha256: None,
            },
            &root,
        )
        .unwrap();
        assert_eq!(missing.state, AcquisitionState::Missing);
        assert!(missing.drift_reasons.is_empty());
        fs::create_dir_all(root.join("games/42/steamcmd-payload")).unwrap();
        let partial = inspect_acquisition(
            42,
            Path::new("bin/game.exe"),
            "profile",
            Some(9),
            &expected_depots,
            AcquisitionProvenance {
                runtime_id: "wine-id",
                runtime_fingerprint_sha256: Some("wine-hash"),
            },
            &root,
        )
        .unwrap();
        assert_eq!(partial.state, AcquisitionState::Partial);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn acquisition_status_detects_ready_and_profile_drift() {
        let root = test_root("status-ready");
        let game_root = root.join("games/42");
        let executable = game_root.join("steamcmd-payload/bin/game.exe");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        let mut bytes = vec![0_u8; 0x200];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x014c_u16.to_le_bytes());
        bytes[0x94..0x96].copy_from_slice(&0xe0_u16.to_le_bytes());
        bytes[0x98..0x9a].copy_from_slice(&0x10b_u16.to_le_bytes());
        fs::write(&executable, bytes).unwrap();
        let graphics = binary::inspect_pe_graphics(&executable).unwrap();
        let compatibility = binary::inspect_pe_compatibility(&executable).unwrap();
        let executable_sha256 = sha256::to_hex(&sha256::digest_file(&executable).unwrap());
        let marker = acquisition_marker(
            42,
            Path::new("bin/game.exe"),
            PeArchitecture::I386,
            &graphics,
            &compatibility,
            &executable_sha256,
            "profile",
            "wine-id",
            "wine-hash",
            "steamcmd-hash",
            "manifest-hash",
            Some(9),
            &BTreeMap::from([(43, 100)]),
        );
        fs::write(game_root.join("steamcmd-acquisition.json"), marker).unwrap();
        fs::write(
            game_root.join("appmanifest_42.acf"),
            r#""AppState" { "appid" "42" "name" "Fixture" "installdir" "Fixture" "buildid" "9" "InstalledDepots" { "43" { "manifest" "100" } } }"#,
        )
        .unwrap();
        let manifest_hash =
            sha256::to_hex(&sha256::digest_file(&game_root.join("appmanifest_42.acf")).unwrap());
        let marker_path = game_root.join("steamcmd-acquisition.json");
        let marker = fs::read_to_string(&marker_path)
            .unwrap()
            .replace("manifest-hash", &manifest_hash);
        fs::write(&marker_path, marker).unwrap();

        let expected_depots = BTreeMap::from([(43, 100)]);
        let ready = inspect_acquisition(
            42,
            Path::new("bin/game.exe"),
            "profile",
            Some(9),
            &expected_depots,
            AcquisitionProvenance {
                runtime_id: "wine-id",
                runtime_fingerprint_sha256: Some("wine-hash"),
            },
            &root,
        )
        .unwrap();
        assert_eq!(ready.state, AcquisitionState::Ready);
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:100\nwindows_payload_root=steam-installed-tree\nruntime=wine\nruntime_version=1\nrenderer=wine-default\ngraphics_api=d3d9\nlaunch=steam\nsteam_install_dir=Fixture\nwindows_build_id=9\nexecutable=bin/game.exe",
        )
        .unwrap();
        let game = acquired_game(&ready, &profile).unwrap();
        assert_eq!(game.classification, steam::Classification::WindowsPayload);
        assert_eq!(game.build_id.as_deref(), Some("9"));
        assert_eq!(game.installed_depots, expected_depots);
        assert_eq!(game.install_dir, root.join("games/42/steamcmd-payload"));
        let runtime_drifted = inspect_acquisition(
            42,
            Path::new("bin/game.exe"),
            "profile",
            Some(9),
            &expected_depots,
            AcquisitionProvenance {
                runtime_id: "wine-id",
                runtime_fingerprint_sha256: Some("changed-runtime-hash"),
            },
            &root,
        )
        .unwrap();
        assert_eq!(runtime_drifted.state, AcquisitionState::Drifted);
        assert_eq!(runtime_drifted.drift_reasons, ["runtime"]);
        let drifted = inspect_acquisition(
            42,
            Path::new("bin/game.exe"),
            "changed",
            Some(9),
            &expected_depots,
            AcquisitionProvenance {
                runtime_id: "wine-id",
                runtime_fingerprint_sha256: Some("wine-hash"),
            },
            &root,
        )
        .unwrap();
        assert_eq!(drifted.state, AcquisitionState::Drifted);
        assert_eq!(drifted.drift_reasons, ["profile"]);
        fs::write(
            game_root.join("appmanifest_42.acf"),
            r#""AppState" { "appid" "42" "name" "Fixture" "installdir" "Fixture" "buildid" "10" "InstalledDepots" { "43" { "manifest" "100" } } }"#,
        )
        .unwrap();
        let manifest_drifted = inspect_acquisition(
            42,
            Path::new("bin/game.exe"),
            "profile",
            Some(9),
            &expected_depots,
            AcquisitionProvenance {
                runtime_id: "wine-id",
                runtime_fingerprint_sha256: Some("wine-hash"),
            },
            &root,
        )
        .unwrap();
        assert_eq!(manifest_drifted.state, AcquisitionState::Drifted);
        assert_eq!(manifest_drifted.drift_reasons, ["manifest"]);
        fs::remove_dir_all(root).unwrap();
    }
}
