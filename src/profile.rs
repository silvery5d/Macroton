use crate::sha256;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityProfile {
    pub schema: u32,
    pub app_id: u32,
    pub windows_depot_id: Option<u32>,
    pub runtime: String,
    pub runtime_version: String,
    pub renderer: String,
    pub graphics_api: GraphicsApi,
    pub launch_mode: LaunchMode,
    pub steam_client_mode: SteamClientMode,
    pub steam_install_dir: Option<String>,
    pub steam_launcher_bypass: Option<PathBuf>,
    pub windows_build_id: Option<String>,
    pub windows_depot_manifest_id: Option<u64>,
    pub windows_depots: Vec<DepotSpec>,
    pub windows_payload_root: WindowsPayloadRoot,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub steam_cloud: SteamCloudPolicy,
    pub steam_cloud_source: Option<String>,
    pub steam_cloud_reviewed_date: Option<String>,
    pub cloud_files: Vec<CloudFileSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamCloudPolicy {
    Unspecified,
    Required,
    Unsupported,
}

impl SteamCloudPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::Required => "required",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn effective(self, has_files: bool) -> Self {
        if self == Self::Unspecified && has_files {
            Self::Required
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPayloadRoot {
    NativeDepotContent,
    SteamInstalledTree,
}

impl WindowsPayloadRoot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NativeDepotContent => "native-depot-content",
            Self::SteamInstalledTree => "steam-installed-tree",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepotSpec {
    pub id: u32,
    pub manifest_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudFileSpec {
    pub remote_path: String,
    pub local_root: CloudLocalRoot,
    pub relative_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudLocalRoot {
    WineUser,
    SteamInstall,
    SteamRemote,
}

impl CloudLocalRoot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WineUser => "user",
            Self::SteamInstall => "steam-install",
            Self::SteamRemote => "steam-remote",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Direct,
    Steam,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteamClientMode {
    Standard,
    RenderedLoginExperimental,
    OpaqueSoftwareExperimental,
    CefDebugExperimental,
    LowMemoryExperimental,
}

impl SteamClientMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::RenderedLoginExperimental => "rendered-login-experimental",
            Self::OpaqueSoftwareExperimental => "opaque-software-experimental",
            Self::CefDebugExperimental => "cef-debug-experimental",
            Self::LowMemoryExperimental => "low-memory-experimental",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsApi {
    Unknown,
    D3d9,
    D3d10,
    D3d11,
    OpenGl,
}

impl GraphicsApi {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::D3d9 => "d3d9",
            Self::D3d10 => "d3d10",
            Self::D3d11 => "d3d11",
            Self::OpenGl => "opengl",
        }
    }
}

impl LaunchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Steam => "steam",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(formatter, "profile line {line}: {}", self.message),
            None => write!(formatter, "profile: {}", self.message),
        }
    }
}

impl std::error::Error for Error {}

impl CompatibilityProfile {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let contents = fs::read_to_string(path).map_err(|error| Error {
            line: None,
            message: format!("cannot read {}: {error}", path.display()),
        })?;
        Self::parse(&contents)
    }

    pub fn parse(contents: &str) -> Result<Self, Error> {
        let mut values = BTreeMap::<String, (usize, String)>::new();
        let mut environment = BTreeMap::new();
        for (index, raw_line) in contents.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or_else(|| Error {
                line: Some(line_number),
                message: "expected key=value".to_owned(),
            })?;
            let key = key.trim();
            let value = value.trim().to_owned();
            if let Some(name) = key.strip_prefix("env.") {
                if name.is_empty() {
                    return Err(Error {
                        line: Some(line_number),
                        message: "environment variable name is empty".to_owned(),
                    });
                }
                if is_reserved_environment(name) {
                    return Err(Error {
                        line: Some(line_number),
                        message: format!(
                            "environment variable {name} is controlled by the launcher"
                        ),
                    });
                }
                if environment.insert(name.to_owned(), value).is_some() {
                    return Err(Error {
                        line: Some(line_number),
                        message: format!("duplicate environment variable {name}"),
                    });
                }
                continue;
            }
            if !matches!(
                key,
                "schema"
                    | "appid"
                    | "windows_depot_id"
                    | "runtime"
                    | "runtime_version"
                    | "renderer"
                    | "graphics_api"
                    | "launch"
                    | "steam_client_mode"
                    | "steam_install_dir"
                    | "steam_launcher_bypass"
                    | "trim_after_executable"
                    | "windows_build_id"
                    | "windows_depot_manifest_id"
                    | "windows_depots"
                    | "windows_payload_root"
                    | "executable"
                    | "arguments"
                    | "steam_cloud"
                    | "steam_cloud_source"
                    | "steam_cloud_reviewed_date"
                    | "cloud_files"
            ) {
                return Err(Error {
                    line: Some(line_number),
                    message: format!("unknown key {key}"),
                });
            }
            if values
                .insert(key.to_owned(), (line_number, value))
                .is_some()
            {
                return Err(Error {
                    line: Some(line_number),
                    message: format!("duplicate key {key}"),
                });
            }
        }

        let required = |key: &str| -> Result<&str, Error> {
            values
                .get(key)
                .map(|(_, value)| value.as_str())
                .ok_or_else(|| Error {
                    line: None,
                    message: format!("missing required key {key}"),
                })
        };
        let parse_u32 = |key: &str| -> Result<u32, Error> {
            let (line, value) = values.get(key).ok_or_else(|| Error {
                line: None,
                message: format!("missing required key {key}"),
            })?;
            value.parse().map_err(|_| Error {
                line: Some(*line),
                message: format!("{key} is not an unsigned integer"),
            })
        };

        let schema = parse_u32("schema")?;
        if schema != 1 {
            return Err(Error {
                line: values.get("schema").map(|(line, _)| *line),
                message: format!("unsupported schema {schema}"),
            });
        }
        let app_id = parse_u32("appid")?;
        let executable = PathBuf::from(required("executable")?);
        validate_relative_path(&executable)?;
        let renderer = required("renderer")?.to_owned();
        if !matches!(
            renderer.as_str(),
            "auto"
                | "wine-default"
                | "wined3d-opengl"
                | "wined3d-opengl-serialized"
                | "wined3d-vulkan"
                | "wined3d-vulkan-serialized"
                | "dxvk-d3d9-moltenvk"
                | "dxmt"
        ) {
            return Err(Error {
                line: values.get("renderer").map(|(line, _)| *line),
                message: format!(
                    "renderer {renderer:?} is not implemented; use auto, wine-default, wined3d-opengl, wined3d-opengl-serialized, wined3d-vulkan, wined3d-vulkan-serialized, dxvk-d3d9-moltenvk, or dxmt"
                ),
            });
        }
        let graphics_api = match values.get("graphics_api").map(|(_, value)| value.as_str()) {
            None | Some("unknown") => GraphicsApi::Unknown,
            Some("d3d9") => GraphicsApi::D3d9,
            Some("d3d10") => GraphicsApi::D3d10,
            Some("d3d11") => GraphicsApi::D3d11,
            Some("opengl") => GraphicsApi::OpenGl,
            Some(value) => {
                return Err(Error {
                    line: values.get("graphics_api").map(|(line, _)| *line),
                    message: format!("unsupported graphics_api {value:?}"),
                });
            }
        };
        let arguments = values
            .get("arguments")
            .map(|(_, value)| split_arguments(value))
            .transpose()?
            .unwrap_or_default();

        let launch_mode = match values.get("launch").map(|(_, value)| value.as_str()) {
            None | Some("direct") => LaunchMode::Direct,
            Some("steam") => LaunchMode::Steam,
            Some(value) => {
                return Err(Error {
                    line: values.get("launch").map(|(line, _)| *line),
                    message: format!("unsupported launch mode {value:?}"),
                });
            }
        };
        let steam_client_mode = match values
            .get("steam_client_mode")
            .map(|(_, value)| value.as_str())
        {
            None | Some("standard") => SteamClientMode::Standard,
            Some("rendered-login-experimental") => SteamClientMode::RenderedLoginExperimental,
            Some("opaque-software-experimental") => SteamClientMode::OpaqueSoftwareExperimental,
            Some("cef-debug-experimental") => SteamClientMode::CefDebugExperimental,
            Some("low-memory-experimental") => SteamClientMode::LowMemoryExperimental,
            Some(value) => {
                return Err(Error {
                    line: values.get("steam_client_mode").map(|(line, _)| *line),
                    message: format!("unsupported steam_client_mode {value:?}"),
                });
            }
        };
        if launch_mode != LaunchMode::Steam && steam_client_mode != SteamClientMode::Standard {
            return Err(Error {
                line: values.get("steam_client_mode").map(|(line, _)| *line),
                message: "steam_client_mode requires launch=steam".to_owned(),
            });
        }
        let steam_install_dir = values
            .get("steam_install_dir")
            .map(|(_, value)| value.clone());
        if let Some(install_dir) = &steam_install_dir {
            validate_steam_install_dir(install_dir)?;
        }
        if launch_mode == LaunchMode::Steam && steam_install_dir.is_none() {
            return Err(Error {
                line: None,
                message: "steam launch mode requires steam_install_dir".to_owned(),
            });
        }
        let steam_launcher_bypass = values
            .get("steam_launcher_bypass")
            .map(|(_, value)| PathBuf::from(value));
        if let Some(path) = &steam_launcher_bypass {
            validate_safe_relative_path(path, "steam_launcher_bypass")?;
            if launch_mode != LaunchMode::Steam {
                return Err(Error {
                    line: values.get("steam_launcher_bypass").map(|(line, _)| *line),
                    message: "steam_launcher_bypass requires launch=steam".to_owned(),
                });
            }
            if path == &executable {
                return Err(Error {
                    line: values.get("steam_launcher_bypass").map(|(line, _)| *line),
                    message: "steam_launcher_bypass must differ from executable".to_owned(),
                });
            }
        }
        let windows_build_id = values
            .get("windows_build_id")
            .map(|(line, value)| {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    Err(Error {
                        line: Some(*line),
                        message: "windows_build_id must contain decimal digits".to_owned(),
                    })
                } else {
                    Ok(value.clone())
                }
            })
            .transpose()?;
        let windows_depot_manifest_id = values
            .get("windows_depot_manifest_id")
            .map(|(line, value)| {
                value.parse().map_err(|_| Error {
                    line: Some(*line),
                    message: "windows_depot_manifest_id is not an unsigned integer".to_owned(),
                })
            })
            .transpose()?;
        let legacy_depot_id = values
            .get("windows_depot_id")
            .map(|(line, value)| {
                value.parse().map_err(|_| Error {
                    line: Some(*line),
                    message: "windows_depot_id is not an unsigned integer".to_owned(),
                })
            })
            .transpose()?;
        if values.contains_key("windows_depots") && legacy_depot_id.is_some() {
            return Err(Error {
                line: values.get("windows_depots").map(|(line, _)| *line),
                message: "use either windows_depots or legacy windows_depot_id fields, not both"
                    .to_owned(),
            });
        }
        if windows_depot_manifest_id.is_some() && legacy_depot_id.is_none() {
            return Err(Error {
                line: values
                    .get("windows_depot_manifest_id")
                    .map(|(line, _)| *line),
                message: "windows_depot_manifest_id requires windows_depot_id".to_owned(),
            });
        }
        let windows_depots = if let Some((line, value)) = values.get("windows_depots") {
            parse_depot_specs(value, *line)?
        } else {
            legacy_depot_id
                .map(|id| {
                    vec![DepotSpec {
                        id,
                        manifest_id: windows_depot_manifest_id,
                    }]
                })
                .unwrap_or_default()
        };
        let windows_payload_root = match values
            .get("windows_payload_root")
            .map(|(_, value)| value.as_str())
        {
            None | Some("native-depot-content") => WindowsPayloadRoot::NativeDepotContent,
            Some("steam-installed-tree") => WindowsPayloadRoot::SteamInstalledTree,
            Some(value) => {
                return Err(Error {
                    line: values.get("windows_payload_root").map(|(line, _)| *line),
                    message: format!("unsupported windows_payload_root {value:?}"),
                });
            }
        };
        let cloud_files = values
            .get("cloud_files")
            .map(|(line, value)| parse_cloud_files(value, *line))
            .transpose()?
            .unwrap_or_default();
        let steam_cloud = match values.get("steam_cloud").map(|(_, value)| value.as_str()) {
            None => SteamCloudPolicy::Unspecified,
            Some("required") => SteamCloudPolicy::Required,
            Some("unsupported") => SteamCloudPolicy::Unsupported,
            Some(value) => {
                return Err(Error {
                    line: values.get("steam_cloud").map(|(line, _)| *line),
                    message: format!("unsupported steam_cloud policy {value:?}"),
                });
            }
        };
        let steam_cloud_source = values
            .get("steam_cloud_source")
            .map(|(_, value)| value.clone());
        let steam_cloud_reviewed_date = values
            .get("steam_cloud_reviewed_date")
            .map(|(_, value)| value.clone());
        if steam_cloud == SteamCloudPolicy::Required && cloud_files.is_empty() {
            return Err(Error {
                line: values.get("steam_cloud").map(|(line, _)| *line),
                message: "steam_cloud=required requires cloud_files".to_owned(),
            });
        }
        if steam_cloud == SteamCloudPolicy::Unsupported && !cloud_files.is_empty() {
            return Err(Error {
                line: values.get("steam_cloud").map(|(line, _)| *line),
                message: "steam_cloud=unsupported cannot declare cloud_files".to_owned(),
            });
        }
        if steam_cloud == SteamCloudPolicy::Unsupported {
            let source = steam_cloud_source.as_deref().ok_or_else(|| Error {
                line: None,
                message: "steam_cloud=unsupported requires steam_cloud_source".to_owned(),
            })?;
            let expected_prefix = format!("https://store.steampowered.com/app/{app_id}/");
            if !source.starts_with(&expected_prefix)
                || source.bytes().any(|byte| byte.is_ascii_whitespace())
            {
                return Err(Error {
                    line: values.get("steam_cloud_source").map(|(line, _)| *line),
                    message: format!(
                        "steam_cloud_source must be an official Steam Store URL for AppID {app_id}"
                    ),
                });
            }
            let reviewed_date = steam_cloud_reviewed_date.as_deref().ok_or_else(|| Error {
                line: None,
                message: "steam_cloud=unsupported requires steam_cloud_reviewed_date".to_owned(),
            })?;
            if !valid_reviewed_date(reviewed_date) {
                return Err(Error {
                    line: values
                        .get("steam_cloud_reviewed_date")
                        .map(|(line, _)| *line),
                    message: "steam_cloud_reviewed_date must be YYYY-MM-DD".to_owned(),
                });
            }
        } else if steam_cloud_source.is_some() || steam_cloud_reviewed_date.is_some() {
            return Err(Error {
                line: values
                    .get("steam_cloud_source")
                    .or_else(|| values.get("steam_cloud_reviewed_date"))
                    .map(|(line, _)| *line),
                message: "steam_cloud source fields require steam_cloud=unsupported".to_owned(),
            });
        }
        if steam_install_dir.is_none()
            && cloud_files
                .iter()
                .any(|file| file.local_root == CloudLocalRoot::SteamInstall)
        {
            return Err(Error {
                line: values.get("cloud_files").map(|(line, _)| *line),
                message: "steam-install Cloud mapping requires steam_install_dir".to_owned(),
            });
        }

        Ok(Self {
            schema,
            app_id,
            windows_depot_id: legacy_depot_id,
            runtime: required("runtime")?.to_owned(),
            runtime_version: required("runtime_version")?.to_owned(),
            renderer,
            graphics_api,
            launch_mode,
            steam_client_mode,
            steam_install_dir,
            steam_launcher_bypass,
            windows_build_id,
            windows_depot_manifest_id,
            windows_depots,
            windows_payload_root,
            executable,
            arguments,
            environment,
            steam_cloud,
            steam_cloud_source,
            steam_cloud_reviewed_date,
            cloud_files,
        })
    }

    pub fn fingerprint(&self) -> String {
        let mut canonical = Vec::new();
        append_field(&mut canonical, "schema", &self.schema.to_string());
        append_field(&mut canonical, "appid", &self.app_id.to_string());
        if let Some(depot_id) = self.windows_depot_id {
            append_field(&mut canonical, "windows_depot_id", &depot_id.to_string());
        }
        append_field(&mut canonical, "runtime", &self.runtime);
        append_field(&mut canonical, "runtime_version", &self.runtime_version);
        append_field(&mut canonical, "renderer", &self.renderer);
        append_field(&mut canonical, "graphics_api", self.graphics_api.as_str());
        append_field(&mut canonical, "launch", self.launch_mode.as_str());
        append_field(
            &mut canonical,
            "steam_client_mode",
            self.steam_client_mode.as_str(),
        );
        if let Some(install_dir) = &self.steam_install_dir {
            append_field(&mut canonical, "steam_install_dir", install_dir);
        }
        if let Some(path) = &self.steam_launcher_bypass {
            append_field(
                &mut canonical,
                "steam_launcher_bypass",
                &path.to_string_lossy(),
            );
        }
        if let Some(build_id) = &self.windows_build_id {
            append_field(&mut canonical, "windows_build_id", build_id);
        }
        if let Some(manifest_id) = self.windows_depot_manifest_id {
            append_field(
                &mut canonical,
                "windows_depot_manifest_id",
                &manifest_id.to_string(),
            );
        }
        for depot in &self.windows_depots {
            append_field(&mut canonical, "depot.id", &depot.id.to_string());
            if let Some(manifest_id) = depot.manifest_id {
                append_field(
                    &mut canonical,
                    "depot.manifest_id",
                    &manifest_id.to_string(),
                );
            }
        }
        append_field(
            &mut canonical,
            "windows_payload_root",
            self.windows_payload_root.as_str(),
        );
        append_field(
            &mut canonical,
            "executable",
            &self.executable.to_string_lossy(),
        );
        for argument in &self.arguments {
            append_field(&mut canonical, "argument", argument);
        }
        for (key, value) in &self.environment {
            append_field(&mut canonical, &format!("env.{key}"), value);
        }
        if self.steam_cloud != SteamCloudPolicy::Unspecified {
            append_field(&mut canonical, "steam_cloud", self.steam_cloud.as_str());
        }
        if let Some(source) = &self.steam_cloud_source {
            append_field(&mut canonical, "steam_cloud_source", source);
        }
        if let Some(reviewed_date) = &self.steam_cloud_reviewed_date {
            append_field(&mut canonical, "steam_cloud_reviewed_date", reviewed_date);
        }
        for file in &self.cloud_files {
            append_field(&mut canonical, "cloud.remote_path", &file.remote_path);
            let key = match file.local_root {
                CloudLocalRoot::WineUser => "cloud.user_relative_path",
                CloudLocalRoot::SteamInstall => "cloud.steam_install_relative_path",
                CloudLocalRoot::SteamRemote => "cloud.steam_remote_relative_path",
            };
            append_field(&mut canonical, key, &file.relative_path.to_string_lossy());
        }
        sha256::to_hex(&sha256::digest_bytes(&canonical))
    }
}

fn parse_cloud_files(value: &str, line: usize) -> Result<Vec<CloudFileSpec>, Error> {
    let mut files = Vec::new();
    for raw in value.split(';') {
        let item = raw.trim();
        let (remote_path, local_path) = item.split_once('|').ok_or_else(|| Error {
            line: Some(line),
            message:
                "cloud_files entries must be REMOTE_PATH|[user:|steam-install:|steam-remote:]RELATIVE_PATH"
                    .to_owned(),
        })?;
        let remote_path = remote_path.trim();
        let local_path = local_path.trim();
        let (local_root, relative_path) = if let Some(path) = local_path.strip_prefix("user:") {
            (CloudLocalRoot::WineUser, PathBuf::from(path))
        } else if let Some(path) = local_path.strip_prefix("steam-install:") {
            (CloudLocalRoot::SteamInstall, PathBuf::from(path))
        } else if let Some(path) = local_path.strip_prefix("steam-remote:") {
            (CloudLocalRoot::SteamRemote, PathBuf::from(path))
        } else {
            // Schema 1 profiles historically treated an unqualified path as
            // relative to the single Wine user directory.
            (CloudLocalRoot::WineUser, PathBuf::from(local_path))
        };
        if remote_path.is_empty()
            || remote_path.contains('\\')
            || remote_path.starts_with('/')
            || remote_path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(Error {
                line: Some(line),
                message: "cloud remote path must be a safe slash-separated relative path"
                    .to_owned(),
            });
        }
        validate_safe_relative_path(&relative_path, "cloud local path")?;
        if files
            .iter()
            .any(|file: &CloudFileSpec| file.remote_path == remote_path)
        {
            return Err(Error {
                line: Some(line),
                message: format!("cloud_files contains duplicate remote path {remote_path:?}"),
            });
        }
        if files.iter().any(|file: &CloudFileSpec| {
            file.local_root == local_root && file.relative_path == relative_path
        }) {
            return Err(Error {
                line: Some(line),
                message: format!(
                    "cloud_files contains duplicate {} local path {:?}",
                    local_root.as_str(),
                    relative_path
                ),
            });
        }
        files.push(CloudFileSpec {
            remote_path: remote_path.to_owned(),
            local_root,
            relative_path,
        });
    }
    if files.is_empty() {
        return Err(Error {
            line: Some(line),
            message: "cloud_files cannot be empty".to_owned(),
        });
    }
    Ok(files)
}

fn valid_reviewed_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return false;
    }
    let month = value[5..7].parse::<u8>().ok();
    let day = value[8..10].parse::<u8>().ok();
    matches!(month, Some(1..=12)) && matches!(day, Some(1..=31))
}

fn parse_depot_specs(value: &str, line: usize) -> Result<Vec<DepotSpec>, Error> {
    let mut depots = Vec::new();
    for raw in value.split(',') {
        let item = raw.trim();
        let (id, manifest_id) = item.split_once(':').ok_or_else(|| Error {
            line: Some(line),
            message: "windows_depots entries must be DEPOT_ID:MANIFEST_ID".to_owned(),
        })?;
        let id = id.parse().map_err(|_| Error {
            line: Some(line),
            message: "windows_depots contains an invalid depot ID".to_owned(),
        })?;
        let manifest_id = manifest_id.parse().map_err(|_| Error {
            line: Some(line),
            message: "windows_depots contains an invalid manifest ID".to_owned(),
        })?;
        if depots.iter().any(|depot: &DepotSpec| depot.id == id) {
            return Err(Error {
                line: Some(line),
                message: format!("windows_depots contains duplicate depot {id}"),
            });
        }
        depots.push(DepotSpec {
            id,
            manifest_id: Some(manifest_id),
        });
    }
    if depots.is_empty() {
        return Err(Error {
            line: Some(line),
            message: "windows_depots cannot be empty".to_owned(),
        });
    }
    Ok(depots)
}

fn validate_steam_install_dir(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(Error {
            line: None,
            message: "steam_install_dir must be one safe directory name".to_owned(),
        });
    }
    Ok(())
}

fn append_field(output: &mut Vec<u8>, key: &str, value: &str) {
    output.extend_from_slice(key.len().to_string().as_bytes());
    output.push(b':');
    output.extend_from_slice(key.as_bytes());
    output.extend_from_slice(value.len().to_string().as_bytes());
    output.push(b':');
    output.extend_from_slice(value.as_bytes());
}

fn is_reserved_environment(name: &str) -> bool {
    let uppercase = name.to_ascii_uppercase();
    matches!(
        uppercase.as_str(),
        "HOME" | "PATH" | "WINEPREFIX" | "WINELOADER" | "WINEARCH" | "WINE_D3D_CONFIG"
    ) || uppercase.starts_with("DYLD_")
        || uppercase.starts_with("LD_")
}

pub fn renderer_environment(renderer: &str) -> Option<(&'static str, &'static str)> {
    match renderer {
        "wined3d-opengl" => Some(("WINE_D3D_CONFIG", "renderer=gl")),
        "wined3d-vulkan" => Some(("WINE_D3D_CONFIG", "renderer=vulkan")),
        "wined3d-vulkan-serialized" => Some(("WINE_D3D_CONFIG", "renderer=vulkan;csmt=3")),
        // Wine defines csmt as a bit-field. Bit 0 enables the threaded command
        // stream and bit 1 serializes multiple command streams in one process.
        // Keep this as an explicit renderer so profiles cannot silently combine
        // an unreviewed Direct3D synchronization policy with another backend.
        "wined3d-opengl-serialized" => Some(("WINE_D3D_CONFIG", "renderer=gl;csmt=3")),
        // DXMT is delivered by a dedicated runtime containing Wine builtin
        // DLLs and its Unix Metal bridge. Its selection is expressed through
        // builtin load order, not WineD3D's renderer environment.
        "dxvk-d3d9-moltenvk" | "dxmt" => None,
        _ => None,
    }
}

fn validate_relative_path(path: &Path) -> Result<(), Error> {
    validate_safe_relative_path(path, "executable")
}

fn validate_safe_relative_path(path: &Path, label: &str) -> Result<(), Error> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(Error {
            line: None,
            message: format!("{label} must be a non-empty relative path"),
        });
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(Error {
            line: None,
            message: format!("{label} cannot escape its root"),
        });
    }
    Ok(())
}

fn split_arguments(input: &str) -> Result<Vec<String>, Error> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        if matches!(character, '"' | '\'') {
            quote = Some(character);
        } else if character.is_whitespace() {
            if !current.is_empty() {
                output.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if escaped || quote.is_some() {
        return Err(Error {
            line: None,
            message: "arguments contain an unfinished escape or quote".to_owned(),
        });
    }
    if !current.is_empty() {
        output.push(current);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_profile() {
        let profile = CompatibilityProfile::parse(
            r#"
            schema=1
            appid=42
            runtime=wine-wow64
            runtime_version=10.0
            renderer=auto
            executable=Game/game.exe
            arguments=-windowed "two words"
            env.TEST=value
            "#,
        )
        .unwrap();
        assert_eq!(profile.app_id, 42);
        assert_eq!(profile.arguments, ["-windowed", "two words"]);
        assert_eq!(profile.environment["TEST"], "value");
        assert_eq!(profile.launch_mode, LaunchMode::Direct);
    }

    #[test]
    fn parses_steam_launch_metadata() {
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depot_id=43\nruntime=wine\nruntime_version=11.14\nrenderer=wine-default\nlaunch=steam\nsteam_install_dir=Example Game\nwindows_build_id=1234\nwindows_depot_manifest_id=1882163069931705787\nexecutable=game.exe",
        )
        .unwrap();
        assert_eq!(profile.launch_mode, LaunchMode::Steam);
        assert_eq!(profile.steam_install_dir.as_deref(), Some("Example Game"));
        assert_eq!(profile.windows_build_id.as_deref(), Some("1234"));
        assert_eq!(
            profile.windows_depot_manifest_id,
            Some(1_882_163_069_931_705_787)
        );
        assert_eq!(
            profile.windows_depots,
            [DepotSpec {
                id: 43,
                manifest_id: Some(1_882_163_069_931_705_787)
            }]
        );
    }

    #[test]
    fn parses_ordered_multi_depot_metadata() {
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:100, 44:200\nruntime=wine\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Game\nexecutable=bin/game.exe",
        )
        .unwrap();
        assert_eq!(
            profile.windows_depots,
            [
                DepotSpec {
                    id: 43,
                    manifest_id: Some(100)
                },
                DepotSpec {
                    id: 44,
                    manifest_id: Some(200)
                }
            ]
        );
        assert_eq!(profile.windows_depot_id, None);
        assert_eq!(
            profile.windows_payload_root,
            WindowsPayloadRoot::NativeDepotContent
        );
    }

    #[test]
    fn parses_authenticated_steam_installed_payload_root() {
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:100,44:200\nwindows_payload_root=steam-installed-tree\nruntime=wine\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Game\nexecutable=bin/game.exe",
        )
        .unwrap();
        assert_eq!(
            profile.windows_payload_root,
            WindowsPayloadRoot::SteamInstalledTree
        );
    }

    #[test]
    fn rejects_ambiguous_or_duplicate_depot_metadata() {
        let mixed = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depot_id=43\nwindows_depots=43:100,44:200\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe",
        )
        .unwrap_err();
        assert!(mixed.message.contains("either windows_depots"));

        let duplicate = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:100,43:200\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe",
        )
        .unwrap_err();
        assert!(duplicate.message.contains("duplicate depot 43"));
    }

    #[test]
    fn steam_launch_requires_a_safe_install_directory() {
        let missing = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nlaunch=steam\nexecutable=x.exe",
        )
        .unwrap_err();
        assert!(missing.message.contains("requires steam_install_dir"));

        let traversal = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=../escape\nexecutable=x.exe",
        )
        .unwrap_err();
        assert!(traversal.message.contains("safe directory"));
    }

    #[test]
    fn launcher_bypass_requires_steam_and_distinct_safe_paths() {
        let direct = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\nsteam_launcher_bypass=Launcher.exe",
        )
        .unwrap_err();
        assert!(direct.message.contains("requires launch=steam"));

        let same = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Game\nexecutable=game.exe\nsteam_launcher_bypass=game.exe",
        )
        .unwrap_err();
        assert!(same.message.contains("must differ"));

        let escaping = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Game\nexecutable=game.exe\nsteam_launcher_bypass=../Launcher.exe",
        )
        .unwrap_err();
        assert!(escaping.message.contains("cannot escape"));
    }

    #[test]
    fn rejects_path_traversal_and_unknown_keys() {
        let traversal = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=../x.exe",
        )
        .unwrap_err();
        assert!(traversal.message.contains("escape"));

        let unknown = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nrenderre=dxvk",
        )
        .unwrap_err();
        assert!(unknown.message.contains("unknown key"));
    }

    #[test]
    fn rejects_launcher_controlled_environment() {
        let error = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nenv.WINEPREFIX=/tmp/escape",
        )
        .unwrap_err();
        assert!(error.message.contains("controlled by the launcher"));

        let renderer_error = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nenv.WINE_D3D_CONFIG=renderer=gl",
        )
        .unwrap_err();
        assert!(
            renderer_error
                .message
                .contains("controlled by the launcher")
        );
    }

    #[test]
    fn maps_the_opengl_renderer_to_launcher_environment() {
        assert_eq!(
            renderer_environment("wined3d-opengl"),
            Some(("WINE_D3D_CONFIG", "renderer=gl"))
        );
        assert_eq!(renderer_environment("wine-default"), None);
        assert_eq!(
            renderer_environment("wined3d-opengl-serialized"),
            Some(("WINE_D3D_CONFIG", "renderer=gl;csmt=3"))
        );
        assert_eq!(
            renderer_environment("wined3d-vulkan"),
            Some(("WINE_D3D_CONFIG", "renderer=vulkan"))
        );
        assert_eq!(
            renderer_environment("wined3d-vulkan-serialized"),
            Some(("WINE_D3D_CONFIG", "renderer=vulkan;csmt=3"))
        );
        assert_eq!(renderer_environment("dxvk-d3d9-moltenvk"), None);
        assert_eq!(renderer_environment("dxmt"), None);
    }

    #[test]
    fn braid_profile_pins_the_color_correct_renderer_and_effect_compiler() {
        let profile = CompatibilityProfile::parse(include_str!("../compatibility/26800.conf"))
            .expect("Braid profile should parse");
        assert_eq!(profile.app_id, 26_800);
        assert_eq!(profile.renderer, "wined3d-opengl");
        assert_eq!(profile.cloud_files.len(), 1);
        assert_eq!(profile.cloud_files[0].remote_path, "slot_0.braid_campaign");
        assert_eq!(
            profile.environment["WINEDLLOVERRIDES"],
            "d3dx9_43=n,b;d3dcompiler_43=n,b"
        );
        assert!(
            !profile
                .arguments
                .iter()
                .any(|argument| argument == "-no_vsync")
        );
    }

    #[test]
    fn rejects_cloud_paths_that_escape_or_duplicate() {
        let escaping = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\ncloud_files=save.sav|../outside.sav",
        )
        .unwrap_err();
        assert!(escaping.message.contains("cannot escape"));

        let duplicate = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\ncloud_files=save.sav|one.sav;save.sav|two.sav",
        )
        .unwrap_err();
        assert!(duplicate.message.contains("duplicate remote path"));

        let duplicate_local = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nsteam_install_dir=Game\nexecutable=x.exe\ncloud_files=one.sav|steam-install:Profiles/Player.nfp;two.sav|steam-install:Profiles/Player.nfp",
        )
        .unwrap_err();
        assert!(
            duplicate_local
                .message
                .contains("duplicate steam-install local path")
        );
    }

    #[test]
    fn cloud_files_support_explicit_user_and_steam_install_roots() {
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nsteam_install_dir=Game\nexecutable=x.exe\ncloud_files=legacy.sav|AppData/Roaming/Game/legacy.sav;explicit.sav|user:Documents/Game/explicit.sav;Profiles/Player.nfp|steam-install:Profiles/Player.nfp;savegame.aws|steam-remote:savegame.aws",
        )
        .unwrap();
        assert_eq!(profile.cloud_files.len(), 4);
        assert_eq!(profile.cloud_files[0].local_root, CloudLocalRoot::WineUser);
        assert_eq!(
            profile.cloud_files[0].relative_path,
            PathBuf::from("AppData/Roaming/Game/legacy.sav")
        );
        assert_eq!(profile.cloud_files[1].local_root, CloudLocalRoot::WineUser);
        assert_eq!(
            profile.cloud_files[2].local_root,
            CloudLocalRoot::SteamInstall
        );
        assert_eq!(
            profile.cloud_files[2].relative_path,
            PathBuf::from("Profiles/Player.nfp")
        );
        assert_eq!(
            profile.cloud_files[3].local_root,
            CloudLocalRoot::SteamRemote
        );
        assert_eq!(
            profile.cloud_files[3].relative_path,
            PathBuf::from("savegame.aws")
        );

        let escaping = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nsteam_install_dir=Game\nexecutable=x.exe\ncloud_files=save.sav|steam-install:../outside.sav",
        )
        .unwrap_err();
        assert!(escaping.message.contains("escape"));
    }

    #[test]
    fn mass_effect_profile_pins_the_authenticated_two_depot_build() {
        let profile = CompatibilityProfile::parse(include_str!("../compatibility/17460.conf"))
            .expect("Mass Effect profile should parse");
        assert_eq!(profile.app_id, 17_460);
        assert_eq!(
            profile.windows_payload_root,
            WindowsPayloadRoot::SteamInstalledTree
        );
        assert_eq!(
            profile.windows_depots,
            [
                DepotSpec {
                    id: 17_461,
                    manifest_id: Some(3_291_123_364_866_001_389),
                },
                DepotSpec {
                    id: 17_462,
                    manifest_id: Some(1_494_229_070_932_282_596),
                },
            ]
        );
        assert_eq!(profile.windows_build_id.as_deref(), Some("2794"));
        assert_eq!(profile.graphics_api, GraphicsApi::D3d9);
        assert_eq!(profile.executable, PathBuf::from("Binaries/MassEffect.exe"));
        assert_eq!(profile.launch_mode, LaunchMode::Steam);
    }

    #[test]
    fn portal_profile_pins_the_authenticated_multi_depot_build() {
        let profile = CompatibilityProfile::parse(include_str!("../compatibility/400.conf"))
            .expect("Portal profile should parse");
        assert_eq!(profile.app_id, 400);
        assert_eq!(profile.graphics_api, GraphicsApi::D3d9);
        assert_eq!(profile.renderer, "wined3d-opengl");
        assert_eq!(
            profile.windows_payload_root,
            WindowsPayloadRoot::SteamInstalledTree
        );
        assert_eq!(profile.windows_depots.len(), 4);
        assert_eq!(profile.windows_depots[0].id, 402);
        assert_eq!(profile.windows_depots[3].id, 401);
        assert_eq!(profile.windows_build_id.as_deref(), Some("19017868"));
        assert_eq!(profile.executable, PathBuf::from("hl2.exe"));
        assert_eq!(profile.launch_mode, LaunchMode::Steam);
        assert_eq!(profile.cloud_files.len(), 13);
        assert!(
            profile
                .cloud_files
                .iter()
                .all(|file| file.local_root == CloudLocalRoot::SteamInstall)
        );
        assert_eq!(profile.cloud_files[0].remote_path, "cfg/config.cfg");
        assert_eq!(
            profile.cloud_files[0].relative_path,
            PathBuf::from("portal/cfg/config.cfg")
        );
    }

    #[test]
    fn alan_wake_profile_pins_the_authenticated_six_depot_build() {
        let profile = CompatibilityProfile::parse(include_str!("../compatibility/108710.conf"))
            .expect("Alan Wake profile should parse");
        assert_eq!(profile.app_id, 108_710);
        assert_eq!(profile.graphics_api, GraphicsApi::D3d9);
        assert_eq!(
            profile.windows_payload_root,
            WindowsPayloadRoot::SteamInstalledTree
        );
        assert_eq!(profile.windows_depots.len(), 6);
        assert_eq!(
            profile.windows_depots.last(),
            Some(&DepotSpec {
                id: 108_711,
                manifest_id: Some(3_695_390_701_422_947_803),
            })
        );
        assert_eq!(profile.windows_build_id.as_deref(), Some("14596864"));
        assert_eq!(profile.executable, PathBuf::from("AlanWake.exe"));
        assert_eq!(profile.launch_mode, LaunchMode::Steam);
        assert_eq!(profile.cloud_files.len(), 3);
        assert!(
            profile
                .cloud_files
                .iter()
                .all(|file| file.local_root == CloudLocalRoot::SteamRemote)
        );
        assert_eq!(profile.cloud_files[2].remote_path, "savegame.aws");
    }

    #[test]
    fn silver_profile_pins_all_observed_windows_cloud_files() {
        let profile = CompatibilityProfile::parse(include_str!("../compatibility/606680.conf"))
            .expect("Silver profile should parse");
        assert_eq!(profile.app_id, 606_680);
        assert_eq!(profile.graphics_api, GraphicsApi::OpenGl);
        assert_eq!(profile.runtime, "macroton-winehq-vulkan-shadowalias");
        assert_eq!(profile.cloud_files.len(), 13);
        assert_eq!(
            profile
                .cloud_files
                .first()
                .map(|file| file.remote_path.as_str()),
            Some("THQNordic/Silver/00000000.sav")
        );
        assert_eq!(
            profile
                .cloud_files
                .last()
                .map(|file| file.remote_path.as_str()),
            Some("THQNordic/Silver/silver.opt")
        );
    }

    #[test]
    fn silver_low_memory_client_mode_is_explicit_and_fingerprinted() {
        let standard = CompatibilityProfile::parse(include_str!("../compatibility/606680.conf"))
            .expect("standard Silver profile should parse");
        let low_memory = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/606680-low-memory.conf"
        ))
        .expect("experimental Silver profile should parse");
        assert_eq!(standard.steam_client_mode, SteamClientMode::Standard);
        assert_eq!(
            low_memory.steam_client_mode,
            SteamClientMode::LowMemoryExperimental
        );
        assert_ne!(standard.fingerprint(), low_memory.fingerprint());
    }

    #[test]
    fn silver_wined3d_capture_profile_keeps_cloud_mapping_and_opengl_renderer() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/606680-shadow-wined3d-gl.conf"
        ))
        .expect("Silver WineD3D capture profile should parse");
        assert_eq!(profile.app_id, 606_680);
        assert_eq!(profile.renderer, "wined3d-opengl");
        assert_eq!(profile.graphics_api, GraphicsApi::OpenGl);
        assert_eq!(
            profile.steam_client_mode,
            SteamClientMode::LowMemoryExperimental
        );
        assert_eq!(profile.cloud_files.len(), 13);
        assert_eq!(
            profile.cloud_files.last().unwrap().relative_path,
            PathBuf::from("AppData/Roaming/THQNordic/Silver/silver.opt")
        );
    }

    #[test]
    fn silver_serialized_wined3d_capture_profile_is_distinct_and_cloud_bound() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/606680-shadow-wined3d-gl-serialized.conf"
        ))
        .expect("Silver serialized WineD3D capture profile should parse");
        assert_eq!(profile.app_id, 606_680);
        assert_eq!(profile.renderer, "wined3d-opengl-serialized");
        assert_eq!(profile.graphics_api, GraphicsApi::OpenGl);
        assert_eq!(profile.cloud_files.len(), 13);
        let normal = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/606680-shadow-wined3d-gl.conf"
        ))
        .expect("Silver WineD3D profile should parse");
        assert_ne!(profile.fingerprint(), normal.fingerprint());
    }

    #[test]
    fn dishonored_preflight_profile_pins_cached_non_dlc_english_metadata() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/205100-preflight.conf"
        ))
        .expect("Dishonored preflight profile should parse");
        assert_eq!(profile.app_id, 205_100);
        assert_eq!(profile.graphics_api, GraphicsApi::D3d9);
        assert_eq!(
            profile.windows_payload_root,
            WindowsPayloadRoot::SteamInstalledTree
        );
        assert_eq!(
            profile.windows_depots,
            [
                DepotSpec {
                    id: 205_101,
                    manifest_id: Some(3_682_254_645_125_751_955),
                },
                DepotSpec {
                    id: 205_102,
                    manifest_id: Some(955_788_823_064_539_748),
                },
                DepotSpec {
                    id: 205_107,
                    manifest_id: Some(3_090_641_581_836_335_173),
                },
            ]
        );
        assert_eq!(profile.windows_build_id.as_deref(), Some("8335061"));
        assert_eq!(
            profile.executable,
            PathBuf::from("Binaries/Win32/Dishonored.exe")
        );
        assert_eq!(profile.launch_mode, LaunchMode::Steam);
        assert_eq!(profile.cloud_files.len(), 11);
        assert!(
            profile
                .cloud_files
                .iter()
                .all(|file| file.local_root == CloudLocalRoot::SteamRemote)
        );
        assert_eq!(profile.cloud_files[10].remote_path, "OPTIONS.sav");
    }

    #[test]
    fn dishonored_cef_login_profile_only_changes_the_client_mode() {
        let gameplay = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/205100-preflight.conf"
        ))
        .expect("Dishonored preflight profile should parse");
        let login = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/205100-cef-login.conf"
        ))
        .expect("Dishonored CEF login profile should parse");
        assert_eq!(
            login.steam_client_mode,
            SteamClientMode::CefDebugExperimental
        );
        let mut normalized = login;
        normalized.steam_client_mode = gameplay.steam_client_mode;
        assert_eq!(normalized, gameplay);
    }

    #[test]
    fn dishonored_low_memory_login_profile_is_distinct_from_cef_diagnostics() {
        let low_memory = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/205100-low-memory-login.conf"
        ))
        .expect("Dishonored low-memory login profile should parse");
        let cef = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/205100-cef-login.conf"
        ))
        .expect("Dishonored CEF login profile should parse");
        assert_eq!(
            low_memory.steam_client_mode,
            SteamClientMode::LowMemoryExperimental
        );
        assert_eq!(cef.steam_client_mode, SteamClientMode::CefDebugExperimental);
        assert_ne!(low_memory.fingerprint(), cef.fingerprint());
    }

    #[test]
    fn dishonored_opaque_login_profile_selects_opaque_cef_fallback() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/205100-opaque-login.conf"
        ))
        .expect("Dishonored opaque login profile should parse");
        assert_eq!(
            profile.steam_client_mode,
            SteamClientMode::OpaqueSoftwareExperimental
        );
        assert_eq!(profile.app_id, 205_100);
        assert_eq!(profile.launch_mode, LaunchMode::Steam);
    }

    #[test]
    fn mass_effect_inventory_candidate_profile_targets_win32_entrypoint() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/17460-preflight.conf"
        ))
        .expect("Mass Effect preflight profile should parse");
        assert_eq!(profile.app_id, 17_460);
        assert_eq!(profile.windows_build_id.as_deref(), Some("2794"));
        assert_eq!(profile.executable, PathBuf::from("Binaries/MassEffect.exe"));
        assert_eq!(profile.windows_depots.len(), 2);
        assert_eq!(profile.graphics_api, GraphicsApi::D3d9);
    }

    #[test]
    fn mass_effect_rendered_login_profile_is_cef_debuggable_and_distinct() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/17460-rendered-opaque-login.conf"
        ))
        .expect("Mass Effect rendered login profile should parse");
        assert_eq!(profile.app_id, 17_460);
        assert_eq!(
            profile.steam_client_mode,
            SteamClientMode::RenderedLoginExperimental
        );
        assert_eq!(profile.launch_mode, LaunchMode::Steam);
        assert_eq!(profile.graphics_api, GraphicsApi::D3d9);
        assert_ne!(
            profile.fingerprint(),
            CompatibilityProfile::parse(include_str!("../compatibility/17460.conf"))
                .expect("Mass Effect standard profile should parse")
                .fingerprint()
        );
    }

    #[test]
    fn silver_direct_local_profile_is_not_a_steam_login_profile() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/606680-direct-local.conf"
        ))
        .expect("Silver direct local profile should parse");
        assert_eq!(profile.app_id, 606_680);
        assert_eq!(profile.launch_mode, LaunchMode::Direct);
        assert_eq!(profile.steam_client_mode, SteamClientMode::Standard);
        assert_eq!(profile.executable, PathBuf::from("silver.exe"));
        assert_eq!(profile.graphics_api, GraphicsApi::OpenGl);
        assert_eq!(
            profile.windows_payload_root,
            WindowsPayloadRoot::SteamInstalledTree
        );
    }

    #[test]
    fn heroes_iii_bypass_profiles_use_the_working_v20_handoff() {
        for source in [
            include_str!("../compatibility/experiments/297000-bypass-pure-forceangle.conf"),
            include_str!(
                "../compatibility/experiments/297000-bypass-pure-forceangle-standard.conf"
            ),
        ] {
            let profile = CompatibilityProfile::parse(source)
                .expect("Heroes III bypass profile should parse");
            assert_eq!(profile.app_id, 297_000);
            assert_eq!(
                profile.arguments,
                vec![
                    "--fromLauncher".to_owned(),
                    "--version".to_owned(),
                    "2.0".to_owned()
                ]
            );
            assert_eq!(
                profile.steam_launcher_bypass,
                Some(PathBuf::from("HOMM3Launcher.exe"))
            );
        }
    }

    #[test]
    fn heroes_iii_v118_low_memory_profile_preserves_2d_handoff() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/297000-bypass-pure-v118-low-memory.conf"
        ))
        .expect("Heroes III low-memory profile should parse");
        assert_eq!(profile.app_id, 297_000);
        assert_eq!(
            profile.steam_client_mode,
            SteamClientMode::LowMemoryExperimental
        );
        assert_eq!(profile.arguments[0], "--fromLauncher");
        assert_eq!(profile.arguments[1], "--version");
        assert_eq!(
            profile.steam_launcher_bypass,
            Some(PathBuf::from("HOMM3Launcher.exe"))
        );
    }

    #[test]
    fn age2hd_preflight_profile_targets_the_game_not_only_the_launcher() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/221380-preflight.conf"
        ))
        .expect("Age of Empires II (2013) preflight profile should parse");
        assert_eq!(profile.app_id, 221_380);
        assert_eq!(profile.graphics_api, GraphicsApi::D3d9);
        assert_eq!(
            profile.windows_payload_root,
            WindowsPayloadRoot::SteamInstalledTree
        );
        assert_eq!(profile.windows_depots.len(), 7);
        assert_eq!(
            profile.windows_depots.first(),
            Some(&DepotSpec {
                id: 221_381,
                manifest_id: Some(1_823_527_930_179_156_287),
            })
        );
        assert_eq!(
            profile.windows_depots.last(),
            Some(&DepotSpec {
                id: 355_950,
                manifest_id: Some(495_816_626_342_536_223),
            })
        );
        assert_eq!(profile.windows_build_id.as_deref(), Some("3062235"));
        assert_eq!(profile.executable, PathBuf::from("aok hd.exe"));
        assert_ne!(profile.executable, PathBuf::from("Launcher.exe"));
        assert_eq!(
            profile.steam_launcher_bypass,
            Some(PathBuf::from("Launcher.exe"))
        );
        assert_eq!(profile.arguments, ["NoStartUp"]);
        assert_eq!(profile.launch_mode, LaunchMode::Steam);
        assert_eq!(profile.cloud_files.len(), 3);
        assert!(
            profile
                .cloud_files
                .iter()
                .all(|file| file.local_root == CloudLocalRoot::SteamInstall)
        );
        assert_eq!(
            profile.cloud_files[0].relative_path,
            PathBuf::from("Profiles/Player.nfp")
        );
        assert_eq!(
            profile.cloud_files[2].relative_path,
            PathBuf::from("SaveGame/Macroton-Acceptance.aoe2spgame")
        );
        assert_eq!(
            profile.cloud_files[2].remote_path,
            "SaveGame/Macroton-Acceptance.aoe2spgame"
        );
    }

    #[test]
    fn age2hd_dxvk_capture_profile_keeps_the_shadow_alias_runtime_experimental() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/221380-dxvk-shadowalias.conf"
        ))
        .expect("Age of Empires II DXVK capture profile should parse");
        assert_eq!(profile.app_id, 221_380);
        assert_eq!(profile.renderer, "dxvk-d3d9-moltenvk");
        assert_eq!(profile.runtime, "macroton-winehq-vulkan-shadowalias");
        assert_eq!(
            profile.steam_launcher_bypass,
            Some(PathBuf::from("Launcher.exe"))
        );
        assert_eq!(profile.arguments, ["NoStartUp"]);
        assert_eq!(profile.windows_depots.len(), 7);
        assert_eq!(profile.cloud_files.len(), 3);
        assert!(
            profile
                .cloud_files
                .iter()
                .all(|file| file.local_root == CloudLocalRoot::SteamInstall)
        );
        assert_eq!(
            profile.cloud_files[2].relative_path,
            PathBuf::from("SaveGame/Macroton-Acceptance.aoe2spgame")
        );
    }

    #[test]
    fn portal_dxvk_profile_keeps_the_independent_metal_path_experimental() {
        let profile = CompatibilityProfile::parse(include_str!(
            "../compatibility/experiments/400-dxvk-d3d9-moltenvk.conf"
        ))
        .unwrap();
        assert_eq!(profile.app_id, 400);
        assert_eq!(profile.runtime, "macroton-winehq-vulkan-shadowalias");
        assert_eq!(profile.runtime_version, "11.14");
        assert_eq!(profile.renderer, "dxvk-d3d9-moltenvk");
        assert_eq!(profile.graphics_api, GraphicsApi::D3d9);
        assert_eq!(profile.launch_mode, LaunchMode::Steam);
        assert_eq!(
            profile.steam_client_mode,
            SteamClientMode::LowMemoryExperimental
        );
        assert_eq!(profile.executable, PathBuf::from("hl2.exe"));
        assert_eq!(
            profile.arguments,
            ["-novid", "-windowed", "-w", "1280", "-h", "720"]
        );
    }

    #[test]
    fn portal_dxvk_cef_login_differs_only_in_client_mode() {
        let game = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/400-dxvk-d3d9-moltenvk.conf",
        ))
        .unwrap();
        let login = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/400-dxvk-cef-debug.conf",
        ))
        .unwrap();
        assert_eq!(
            game.steam_client_mode,
            SteamClientMode::LowMemoryExperimental
        );
        assert_eq!(
            login.steam_client_mode,
            SteamClientMode::CefDebugExperimental
        );
        let mut normalized = login;
        normalized.steam_client_mode = game.steam_client_mode;
        assert_eq!(normalized, game);
    }

    #[test]
    fn chrono_trigger_profile_pins_the_windows_only_2d_candidate() {
        let profile = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/613830-pure-wined3d-standard.conf",
        ))
        .unwrap();
        assert_eq!(profile.app_id, 613830);
        assert_eq!(profile.runtime, "macroton-winehq-pure");
        assert_eq!(profile.renderer, "wined3d-opengl");
        assert_eq!(profile.graphics_api, GraphicsApi::OpenGl);
        assert_eq!(profile.windows_build_id.as_deref(), Some("11890051"));
        assert_eq!(profile.windows_depot_id, Some(613831));
        assert_eq!(profile.windows_depot_manifest_id, Some(6725455135839501871));
        assert_eq!(profile.executable, PathBuf::from("Chrono Trigger.exe"));
        assert_eq!(profile.steam_client_mode, SteamClientMode::Standard);
    }

    #[test]
    fn chrono_trigger_wgl_control_changes_only_the_wine_runtime() {
        let pure = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/613830-pure-wined3d-standard.conf",
        ))
        .unwrap();
        let staging = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/613830-wine-staging-wgl-2d.conf",
        ))
        .unwrap();
        assert_eq!(staging.runtime, "macroton-wine-staging-macos");
        assert_eq!(staging.runtime_version, "11.14");
        let mut normalized = staging;
        normalized.runtime = pure.runtime.clone();
        assert_eq!(normalized, pure);
    }

    #[test]
    fn chrono_trigger_seh_diagnostic_changes_only_the_debug_channel() {
        let baseline = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/613830-wine-staging-wgl-2d.conf",
        ))
        .unwrap();
        let diagnostic = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/613830-wine-staging-seh-only.conf",
        ))
        .unwrap();
        assert_eq!(diagnostic.environment.get("WINEDEBUG").map(String::as_str), Some("+seh"));
        let mut normalized = diagnostic;
        normalized.environment.insert("WINEDEBUG".to_owned(), "-all".to_owned());
        assert_eq!(normalized, baseline);
    }

    #[test]
    fn parses_and_fingerprints_graphics_api() {
        let d3d9 = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\ngraphics_api=d3d9\nexecutable=x.exe",
        )
        .unwrap();
        let d3d10 = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\ngraphics_api=d3d10\nexecutable=x.exe",
        )
        .unwrap();
        assert_eq!(d3d9.graphics_api, GraphicsApi::D3d9);
        assert_eq!(d3d10.graphics_api, GraphicsApi::D3d10);
        assert_ne!(d3d9.fingerprint(), d3d10.fingerprint());
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive_to_configuration() {
        let first = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nenv.TEST=one",
        )
        .unwrap();
        let second = CompatibilityProfile::parse(
            "env.TEST=one\nexecutable=x.exe\nrenderer=auto\nruntime_version=1\nruntime=x\nappid=1\nschema=1",
        )
        .unwrap();
        let changed = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nenv.TEST=two",
        )
        .unwrap();
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_ne!(first.fingerprint(), changed.fingerprint());
    }

    #[test]
    fn steam_cloud_policy_is_explicit_and_fail_closed() {
        let unsupported = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nsteam_cloud=unsupported\nsteam_cloud_source=https://store.steampowered.com/app/1/Fixture/\nsteam_cloud_reviewed_date=2026-08-13",
        )
        .unwrap();
        assert_eq!(unsupported.steam_cloud, SteamCloudPolicy::Unsupported);
        assert_eq!(
            unsupported
                .steam_cloud
                .effective(!unsupported.cloud_files.is_empty()),
            SteamCloudPolicy::Unsupported
        );

        let legacy_required = CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\ncloud_files=save.sav|save.sav",
        )
        .unwrap();
        assert_eq!(legacy_required.steam_cloud, SteamCloudPolicy::Unspecified);
        assert_eq!(
            legacy_required
                .steam_cloud
                .effective(!legacy_required.cloud_files.is_empty()),
            SteamCloudPolicy::Required
        );

        assert!(CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nsteam_cloud=required",
        )
        .unwrap_err()
        .message
        .contains("requires cloud_files"));
        assert!(CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nsteam_cloud=unsupported\ncloud_files=save.sav|save.sav",
        )
        .unwrap_err()
        .message
        .contains("cannot declare cloud_files"));
        assert!(CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nsteam_cloud=unsupported",
        )
        .unwrap_err()
        .message
        .contains("requires steam_cloud_source"));
        assert!(CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nsteam_cloud=unsupported\nsteam_cloud_source=https://store.steampowered.com/app/2/Wrong/\nsteam_cloud_reviewed_date=2026-08-13",
        )
        .unwrap_err()
        .message
        .contains("official Steam Store URL"));
        assert!(CompatibilityProfile::parse(
            "schema=1\nappid=1\nruntime=x\nruntime_version=1\nrenderer=auto\nexecutable=x.exe\nsteam_cloud=unsupported\nsteam_cloud_source=https://store.steampowered.com/app/1/Fixture/\nsteam_cloud_reviewed_date=2026-99-13",
        )
        .unwrap_err()
        .message
        .contains("YYYY-MM-DD"));
    }
}
