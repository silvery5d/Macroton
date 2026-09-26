use crate::binary::{self, BinaryKind, PeGraphicsApi};
use crate::profile::{
    CompatibilityProfile, DepotSpec, GraphicsApi, LaunchMode, SteamClientMode, WindowsPayloadRoot,
};
use crate::runtime::{self, RuntimeKind};
use crate::steam::SteamGame;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub app_id: u32,
    pub game_name: String,
    pub steam_build_id: Option<String>,
    pub windows_depot_id: Option<u32>,
    pub windows_depots: Vec<DepotSpec>,
    pub payload_root: PathBuf,
    pub payload_roots: Vec<PathBuf>,
    pub executable: PathBuf,
    pub profile_sha256: String,
    pub runtime_id: String,
    pub runtime_kind: Option<RuntimeKind>,
    pub runtime_entrypoint: Option<PathBuf>,
    pub runtime_fingerprint_sha256: Option<String>,
    pub runtime_components: Vec<String>,
    pub renderer: String,
    pub graphics_api: GraphicsApi,
    pub graphics_evidence: Vec<String>,
    pub launch_mode: LaunchMode,
    pub steam_client_mode: SteamClientMode,
    pub steam_install_dir: Option<String>,
    pub steam_launcher_bypass: Option<PathBuf>,
    pub windows_build_id: Option<String>,
    pub windows_depot_manifest_id: Option<u64>,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub blockers: Vec<String>,
}

impl LaunchPlan {
    pub fn is_ready(&self) -> bool {
        self.blockers.is_empty()
    }
}

pub fn build(game: &SteamGame, profile: &CompatibilityProfile, runtime_root: &Path) -> LaunchPlan {
    let mut blockers = Vec::new();
    if game.app_id != profile.app_id {
        blockers.push(format!(
            "profile AppID {} does not match game AppID {}",
            profile.app_id, game.app_id
        ));
    }
    if let Some(expected) = &profile.windows_build_id
        && game.build_id.as_deref() != Some(expected)
    {
        blockers.push(format!(
            "installed Steam BuildID {} does not match profile BuildID {expected}",
            game.build_id.as_deref().unwrap_or("unknown")
        ));
    }
    for depot in &profile.windows_depots {
        if let Some(expected_manifest) = depot.manifest_id {
            match game.installed_depots.get(&depot.id) {
                Some(observed) if *observed == expected_manifest => {}
                Some(observed) => blockers.push(format!(
                    "installed depot {} manifest {observed} does not match profile manifest {expected_manifest}",
                    depot.id
                )),
                None if profile.windows_payload_root == WindowsPayloadRoot::SteamInstalledTree => {
                    blockers.push(format!(
                        "Steam manifest does not record required depot {} manifest {expected_manifest}",
                        depot.id
                    ));
                }
                None => {}
            }
        }
    }
    let payload_roots = if profile.windows_depots.is_empty()
        || profile.windows_payload_root == WindowsPayloadRoot::SteamInstalledTree
    {
        // A previous launcher/renderer transform may have preserved the
        // authenticated install under Macroton's hidden source name while
        // the public Steam install path is temporarily a view or absent.
        // Treat that source as the installed tree for readiness checks; the
        // executor will recover/materialize the public view safely.
        let installed_root = if !game.install_dir.is_symlink()
            && game.install_dir.join(&profile.executable).is_file()
        {
            game.install_dir.clone()
        } else {
            game.install_dir
                .parent()
                .and_then(|parent| {
                    let install_name = game
                        .install_dir
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default();
                    let prefix = format!(".{install_name}.macroton-source-");
                    fs::read_dir(parent).ok()?.flatten().find_map(|entry| {
                        let path = entry.path();
                        let name = path.file_name()?.to_str()?;
                        (name.starts_with(&prefix) && path.join(&profile.executable).is_file())
                            .then_some(path)
                    })
                })
                .unwrap_or_else(|| game.install_dir.clone())
        };
        vec![installed_root]
    } else {
        profile
            .windows_depots
            .iter()
            .map(|depot| resolve_depot_root(game, depot.id))
            .collect()
    };
    let payload_root = payload_roots[0].clone();
    if profile.windows_payload_root == WindowsPayloadRoot::NativeDepotContent {
        for (depot, root) in profile.windows_depots.iter().zip(&payload_roots) {
            if !root.is_dir() {
                blockers.push(format!(
                    "Windows depot {} is not downloaded at {}",
                    depot.id,
                    root.display()
                ));
            }
        }
    }
    let executable = payload_roots
        .iter()
        .map(|root| root.join(&profile.executable))
        .find(|path| path.is_file())
        .unwrap_or_else(|| payload_root.join(&profile.executable));
    let mut graphics_evidence = Vec::new();
    if !executable.is_file() {
        if !profile.windows_depots.is_empty() {
            blockers.push(format!(
                "configured executable {} is absent from Windows depots {}",
                profile.executable.display(),
                profile
                    .windows_depots
                    .iter()
                    .map(|depot| depot.id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        } else {
            blockers.push(format!(
                "configured Windows payload is not installed: {}",
                executable.display()
            ));
        }
    } else if !matches!(
        binary::inspect(&executable),
        Ok(BinaryKind::PortableExecutable)
    ) {
        blockers.push(format!(
            "configured payload is not a Windows PE executable: {}",
            executable.display()
        ));
    } else {
        // Some launchers are packed or otherwise have import tables that
        // static inspection cannot parse. Absence of evidence is not a
        // mismatch; renderer-specific gates still apply independently.
        if let Ok(evidence) = binary::inspect_pe_graphics(&executable) {
            graphics_evidence = evidence
                .apis
                .iter()
                .map(|api| api.as_str().to_owned())
                .collect();
            if let Some(expected) = profile_api_to_pe(profile.graphics_api)
                && !evidence.apis.is_empty()
                && !evidence.apis.contains(&expected)
            {
                blockers.push(format!(
                    "profile graphics_api={} is not supported by static PE import evidence [{}] from {}",
                    profile.graphics_api.as_str(),
                    graphics_evidence.join(","),
                    evidence.inspected_files.join(",")
                ));
            }
        }
    }
    let (runtime_id, installed_runtime) =
        resolve_profile_runtime(profile, runtime_root, &mut blockers);
    LaunchPlan {
        app_id: game.app_id,
        game_name: game.name.clone(),
        steam_build_id: game.build_id.clone(),
        windows_depot_id: profile.windows_depot_id,
        windows_depots: profile.windows_depots.clone(),
        payload_root,
        payload_roots,
        executable,
        profile_sha256: profile.fingerprint(),
        runtime_id,
        runtime_kind: installed_runtime.as_ref().map(|runtime| runtime.kind),
        runtime_entrypoint: installed_runtime
            .as_ref()
            .map(|runtime| runtime.entrypoint.clone()),
        runtime_fingerprint_sha256: installed_runtime
            .as_ref()
            .map(|runtime| runtime.fingerprint_sha256.clone()),
        runtime_components: installed_runtime
            .as_ref()
            .map(|runtime| runtime.components.clone())
            .unwrap_or_default(),
        renderer: profile.renderer.clone(),
        graphics_api: profile.graphics_api,
        graphics_evidence,
        launch_mode: profile.launch_mode,
        steam_client_mode: profile.steam_client_mode,
        steam_install_dir: profile.steam_install_dir.clone(),
        steam_launcher_bypass: profile.steam_launcher_bypass.clone(),
        windows_build_id: profile.windows_build_id.clone(),
        windows_depot_manifest_id: profile.windows_depot_manifest_id,
        arguments: profile.arguments.clone(),
        environment: profile.environment.clone(),
        blockers,
    }
}

pub fn build_steam_client(profile: &CompatibilityProfile, runtime_root: &Path) -> LaunchPlan {
    let mut blockers = Vec::new();
    if profile.launch_mode != LaunchMode::Steam {
        blockers.push("Steam login requires launch=steam".to_owned());
    }
    let (runtime_id, installed_runtime) =
        resolve_profile_runtime(profile, runtime_root, &mut blockers);
    LaunchPlan {
        app_id: profile.app_id,
        game_name: format!("Steam AppID {}", profile.app_id),
        steam_build_id: None,
        windows_depot_id: profile.windows_depot_id,
        windows_depots: profile.windows_depots.clone(),
        payload_root: PathBuf::new(),
        payload_roots: Vec::new(),
        executable: profile.executable.clone(),
        profile_sha256: profile.fingerprint(),
        runtime_id,
        runtime_kind: installed_runtime.as_ref().map(|runtime| runtime.kind),
        runtime_entrypoint: installed_runtime
            .as_ref()
            .map(|runtime| runtime.entrypoint.clone()),
        runtime_fingerprint_sha256: installed_runtime
            .as_ref()
            .map(|runtime| runtime.fingerprint_sha256.clone()),
        runtime_components: installed_runtime
            .as_ref()
            .map(|runtime| runtime.components.clone())
            .unwrap_or_default(),
        renderer: profile.renderer.clone(),
        graphics_api: profile.graphics_api,
        graphics_evidence: Vec::new(),
        launch_mode: profile.launch_mode,
        steam_client_mode: profile.steam_client_mode,
        steam_install_dir: profile.steam_install_dir.clone(),
        steam_launcher_bypass: profile.steam_launcher_bypass.clone(),
        windows_build_id: profile.windows_build_id.clone(),
        windows_depot_manifest_id: profile.windows_depot_manifest_id,
        arguments: profile.arguments.clone(),
        environment: profile.environment.clone(),
        blockers,
    }
}

fn resolve_profile_runtime(
    profile: &CompatibilityProfile,
    runtime_root: &Path,
    blockers: &mut Vec<String>,
) -> (String, Option<runtime::InstalledRuntime>) {
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    let runtime_path = runtime_root.join(&runtime_id);
    let installed_runtime = match runtime::resolve_installed(runtime_root, &runtime_id) {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            blockers.push(format!(
                "pinned runtime is not usable at {}: {error}",
                runtime_path.display(),
            ));
            None
        }
    };
    if profile.renderer == "dxmt"
        && installed_runtime
            .as_ref()
            .is_some_and(|runtime| !runtime.components.iter().any(|id| id.starts_with("dxmt-")))
    {
        blockers.push(format!(
            "renderer dxmt requires a verified dxmt-* runtime component manifest in {}",
            runtime_path.display()
        ));
    }
    if profile.renderer == "dxvk-d3d9-moltenvk"
        && installed_runtime.as_ref().is_some_and(|runtime| {
            !runtime
                .components
                .iter()
                .any(|id| id.starts_with("dxvk-d3d9-moltenvk-"))
        })
    {
        blockers.push(format!(
            "renderer dxvk-d3d9-moltenvk requires a verified dxvk-d3d9-moltenvk-* runtime component manifest in {}",
            runtime_path.display()
        ));
    }
    if profile.renderer.starts_with("wined3d-")
        && profile.graphics_api == GraphicsApi::D3d9
        && installed_runtime.as_ref().is_some_and(|runtime| {
            runtime
                .components
                .iter()
                .any(|id| id.starts_with("dxvk-d3d9-moltenvk-"))
        })
    {
        blockers.push(format!(
            "renderer {} cannot use a runtime with a verified DXVK d3d9 component in {}",
            profile.renderer,
            runtime_path.display()
        ));
    }
    if profile.renderer == "dxvk-d3d9-moltenvk" && profile.graphics_api != GraphicsApi::D3d9 {
        blockers.push(format!(
            "renderer dxvk-d3d9-moltenvk supports d3d9 profiles, not {}",
            profile.graphics_api.as_str()
        ));
    }
    if profile.renderer == "dxmt"
        && !matches!(
            profile.graphics_api,
            GraphicsApi::D3d10 | GraphicsApi::D3d11
        )
    {
        blockers.push(format!(
            "renderer dxmt supports d3d10/d3d11 profiles, not {}",
            profile.graphics_api.as_str()
        ));
    }
    (runtime_id, installed_runtime)
}

fn profile_api_to_pe(api: GraphicsApi) -> Option<PeGraphicsApi> {
    match api {
        GraphicsApi::Unknown => None,
        GraphicsApi::D3d9 => Some(PeGraphicsApi::D3d9),
        GraphicsApi::D3d10 => Some(PeGraphicsApi::D3d10),
        GraphicsApi::D3d11 => Some(PeGraphicsApi::D3d11),
        GraphicsApi::OpenGl => Some(PeGraphicsApi::OpenGl),
    }
}

fn resolve_depot_root(game: &SteamGame, depot_id: u32) -> PathBuf {
    let steamapps = game
        .manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let suffix = PathBuf::from("content")
        .join(format!("app_{}", game.app_id))
        .join(format!("depot_{depot_id}"));
    let standard = steamapps.join(&suffix);
    let bundled = steamapps
        .parent()
        .map(|steam_root| {
            steam_root
                .join("Steam.AppBundle/Steam/Contents/MacOS/steamapps")
                .join(&suffix)
        })
        .unwrap_or_default();
    if standard.is_dir() {
        standard
    } else if bundled.is_dir() {
        bundled
    } else {
        standard
    }
}

pub fn resolve_depot_root_for_game(game: &SteamGame, depot_id: u32) -> PathBuf {
    resolve_depot_root(game, depot_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256;
    use crate::steam::Classification;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "macroton-plan-test-{}-{nonce}-{sequence}",
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

    fn write_pe_with_import(path: &Path, imported: &str) {
        let mut bytes = vec![0_u8; 0x500];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x014c_u16.to_le_bytes());
        bytes[0x86..0x88].copy_from_slice(&1_u16.to_le_bytes());
        bytes[0x94..0x96].copy_from_slice(&0xe0_u16.to_le_bytes());
        bytes[0x98..0x9a].copy_from_slice(&0x10b_u16.to_le_bytes());
        bytes[0x98 + 96 + 8..0x98 + 96 + 12].copy_from_slice(&0x1100_u32.to_le_bytes());
        let section = 0x98 + 0xe0;
        bytes[section..section + 6].copy_from_slice(b".rdata");
        bytes[section + 8..section + 12].copy_from_slice(&0x300_u32.to_le_bytes());
        bytes[section + 12..section + 16].copy_from_slice(&0x1000_u32.to_le_bytes());
        bytes[section + 16..section + 20].copy_from_slice(&0x300_u32.to_le_bytes());
        bytes[section + 20..section + 24].copy_from_slice(&0x200_u32.to_le_bytes());
        bytes[0x300 + 12..0x300 + 16].copy_from_slice(&0x1180_u32.to_le_bytes());
        bytes[0x380..0x380 + imported.len()].copy_from_slice(imported.as_bytes());
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn reports_missing_payload_and_runtime() {
        let game = SteamGame {
            app_id: 42,
            name: "Example".to_owned(),
            install_dir: PathBuf::from("/definitely/missing/game"),
            manifest_path: PathBuf::from("/manifest"),
            state_flags: None,
            build_id: None,
            installed_depots: BTreeMap::new(),
            classification: Classification::Unknown,
            binaries: Vec::new(),
            inspection_truncated: false,
        };
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine-wow64\nruntime_version=10\nrenderer=auto\nexecutable=game.exe",
        )
        .unwrap();
        let plan = build(&game, &profile, Path::new("/definitely/missing/runtime"));
        assert!(!plan.is_ready());
        assert_eq!(plan.blockers.len(), 2);
    }

    #[test]
    fn steam_client_plan_does_not_require_an_installed_game_payload() {
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:100,44:200\nwindows_payload_root=steam-installed-tree\nwindows_build_id=9\nruntime=wine\nruntime_version=1\nrenderer=wined3d-opengl\ngraphics_api=d3d9\nlaunch=steam\nsteam_install_dir=Example\nexecutable=game.exe",
        )
        .unwrap();
        let plan = build_steam_client(&profile, Path::new("/definitely/missing/runtime"));
        assert_eq!(plan.app_id, 42);
        assert_eq!(plan.blockers.len(), 1);
        assert!(plan.blockers[0].contains("runtime"));
        assert!(!plan.blockers[0].contains("payload"));
        assert!(!plan.blockers[0].contains("BuildID"));
        assert!(!plan.blockers[0].contains("manifest"));
    }

    #[test]
    fn resolves_steam_app_bundle_depot_layout() {
        let temp = TempDir::new();
        let steam_root = temp.0.join("Steam");
        let manifest = steam_root.join("steamapps/appmanifest_42.acf");
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        fs::write(&manifest, "fixture").unwrap();
        let depot = steam_root
            .join("Steam.AppBundle/Steam/Contents/MacOS/steamapps/content/app_42/depot_43");
        fs::create_dir_all(&depot).unwrap();
        fs::write(depot.join("game.exe"), b"MZfixture").unwrap();
        let game = SteamGame {
            app_id: 42,
            name: "Example".to_owned(),
            install_dir: steam_root.join("steamapps/common/Example"),
            manifest_path: manifest,
            state_flags: None,
            build_id: Some("1".to_owned()),
            installed_depots: BTreeMap::from([(43, 1)]),
            classification: Classification::LegacyMac32,
            binaries: Vec::new(),
            inspection_truncated: false,
        };
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depot_id=43\nruntime=wine\nruntime_version=1\nrenderer=wine-default\nexecutable=game.exe",
        )
        .unwrap();

        let plan = build(&game, &profile, &temp.0.join("runtimes"));
        assert_eq!(plan.payload_root, depot);
        assert_eq!(plan.windows_depot_id, Some(43));
        assert_eq!(plan.blockers.len(), 1);
        assert!(plan.blockers[0].contains("runtime"));
    }

    #[test]
    fn accepts_an_authenticated_multi_depot_installed_tree() {
        let temp = TempDir::new();
        let manifest = temp.0.join("steamapps/appmanifest_42.acf");
        let installed = temp.0.join("steamapps/common/Example");
        fs::create_dir_all(installed.join("bin")).unwrap();
        fs::write(&manifest, "fixture").unwrap();
        fs::write(installed.join("bin/game.exe"), b"MZfixture").unwrap();
        let game = SteamGame {
            app_id: 42,
            name: "Example".to_owned(),
            install_dir: installed.clone(),
            manifest_path: manifest,
            state_flags: None,
            build_id: Some("9".to_owned()),
            installed_depots: BTreeMap::from([(43, 100), (44, 200)]),
            classification: Classification::WindowsPayload,
            binaries: Vec::new(),
            inspection_truncated: false,
        };
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:100,44:200\nwindows_payload_root=steam-installed-tree\nruntime=wine\nruntime_version=1\nrenderer=auto\nlaunch=steam\nsteam_install_dir=Example\nexecutable=bin/game.exe",
        )
        .unwrap();
        let plan = build(&game, &profile, &temp.0.join("runtimes"));
        assert_eq!(plan.payload_roots, [installed]);
        assert_eq!(plan.windows_depots.len(), 2);
        assert_eq!(plan.blockers.len(), 1);
        assert!(plan.blockers[0].contains("runtime"));
    }

    #[cfg(unix)]
    #[test]
    fn resolves_hidden_source_behind_payload_view_symlink() {
        let temp = TempDir::new();
        let common = temp.0.join("steamapps/common");
        let public_view = common.join("Silver");
        let hidden_source = common.join(".Silver.macroton-source-test");
        fs::create_dir_all(&hidden_source).unwrap();
        fs::write(hidden_source.join("silver.exe"), b"MZfixture").unwrap();
        fs::create_dir_all(&common).unwrap();
        symlink(&hidden_source, &public_view).unwrap();
        let game = SteamGame {
            app_id: 42,
            name: "Silver".to_owned(),
            install_dir: public_view,
            manifest_path: temp.0.join("steamapps/appmanifest_42.acf"),
            state_flags: None,
            build_id: None,
            installed_depots: BTreeMap::new(),
            classification: Classification::WindowsPayload,
            binaries: Vec::new(),
            inspection_truncated: false,
        };
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_payload_root=steam-installed-tree\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=silver.exe",
        )
        .unwrap();
        let plan = build(&game, &profile, &temp.0.join("runtimes"));
        assert_eq!(plan.payload_root, hidden_source);
        assert_eq!(plan.executable, plan.payload_root.join("silver.exe"));
        assert!(!plan.blockers.iter().any(|value| value.contains("payload")));
    }

    #[test]
    fn rejects_a_drifted_installed_build_or_depot_manifest() {
        let temp = TempDir::new();
        let manifest = temp.0.join("steamapps/appmanifest_42.acf");
        let installed = temp.0.join("steamapps/common/Example");
        fs::create_dir_all(&installed).unwrap();
        fs::write(&manifest, "fixture").unwrap();
        fs::write(installed.join("game.exe"), b"MZfixture").unwrap();
        let game = SteamGame {
            app_id: 42,
            name: "Example".to_owned(),
            install_dir: installed,
            manifest_path: manifest,
            state_flags: None,
            build_id: Some("10".to_owned()),
            installed_depots: BTreeMap::from([(43, 101)]),
            classification: Classification::WindowsPayload,
            binaries: Vec::new(),
            inspection_truncated: false,
        };
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depots=43:100\nwindows_payload_root=steam-installed-tree\nwindows_build_id=9\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe",
        )
        .unwrap();

        let plan = build(&game, &profile, &temp.0.join("runtimes"));
        assert!(plan.blockers.iter().any(|value| value.contains("BuildID")));
        assert!(plan.blockers.iter().any(|value| value.contains("manifest")));
    }

    #[test]
    fn dxmt_renderer_requires_a_verified_runtime_component() {
        let temp = TempDir::new();
        let installed = temp.0.join("steamapps/common/Example");
        fs::create_dir_all(&installed).unwrap();
        fs::write(installed.join("game.exe"), b"MZfixture").unwrap();
        let game = SteamGame {
            app_id: 42,
            name: "Example".to_owned(),
            install_dir: installed,
            manifest_path: temp.0.join("steamapps/appmanifest_42.acf"),
            state_flags: None,
            build_id: None,
            installed_depots: BTreeMap::new(),
            classification: Classification::WindowsPayload,
            binaries: Vec::new(),
            inspection_truncated: false,
        };
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=dxmt\ngraphics_api=d3d10\nexecutable=game.exe",
        )
        .unwrap();
        let wine_root = temp.0.join("wine-root");
        let entrypoint = wine_root.join("bin/wine");
        fs::create_dir_all(entrypoint.parent().unwrap()).unwrap();
        fs::write(&entrypoint, "#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&entrypoint).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&entrypoint, permissions).unwrap();
        let runtime_root = temp.0.join("runtimes");
        runtime::register_external("wine-1", &entrypoint, &runtime_root).unwrap();

        let missing = build(&game, &profile, &runtime_root);
        assert!(missing.blockers.iter().any(|value| {
            value.contains("renderer dxmt requires a verified dxmt-* runtime component")
        }));

        let component = wine_root.join("lib/wine/i386-windows/d3d10core.dll");
        fs::create_dir_all(component.parent().unwrap()).unwrap();
        fs::write(&component, b"dxmt").unwrap();
        fs::create_dir_all(wine_root.join("share/macroton/components")).unwrap();
        let checksum = sha256::to_hex(&sha256::digest_file(&component).unwrap());
        fs::write(
            wine_root.join("share/macroton/components/dxmt-1.conf"),
            format!("schema=1\nid=dxmt-1\nfile.i386-windows/d3d10core.dll={checksum}\n"),
        )
        .unwrap();

        let verified = build(&game, &profile, &runtime_root);
        assert!(verified.is_ready(), "{:?}", verified.blockers);

        let mismatched = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=dxmt\ngraphics_api=d3d9\nexecutable=game.exe",
        )
        .unwrap();
        let rejected = build(&game, &mismatched, &runtime_root);
        assert!(
            rejected
                .blockers
                .iter()
                .any(|value| value.contains("not d3d9"))
        );
    }

    #[test]
    fn dxvk_d3d9_moltenvk_requires_matching_api_and_verified_component() {
        let temp = TempDir::new();
        let installed = temp.0.join("steamapps/common/Example");
        fs::create_dir_all(&installed).unwrap();
        write_pe_with_import(&installed.join("game.exe"), "d3d9.dll\0");
        let game = SteamGame {
            app_id: 42,
            name: "Example".to_owned(),
            install_dir: installed,
            manifest_path: temp.0.join("steamapps/appmanifest_42.acf"),
            state_flags: None,
            build_id: None,
            installed_depots: BTreeMap::new(),
            classification: Classification::WindowsPayload,
            binaries: Vec::new(),
            inspection_truncated: false,
        };
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=dxvk-d3d9-moltenvk\ngraphics_api=d3d9\nexecutable=game.exe",
        )
        .unwrap();
        let wine_root = temp.0.join("wine-root");
        let entrypoint = wine_root.join("bin/wine");
        fs::create_dir_all(entrypoint.parent().unwrap()).unwrap();
        fs::write(&entrypoint, "#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&entrypoint).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&entrypoint, permissions).unwrap();
        let runtime_root = temp.0.join("runtimes");
        runtime::register_external("wine-1", &entrypoint, &runtime_root).unwrap();

        let missing = build(&game, &profile, &runtime_root);
        assert!(
            missing
                .blockers
                .iter()
                .any(|value| { value.contains("requires a verified dxvk-d3d9-moltenvk-*") })
        );

        let component = wine_root.join("lib/wine/i386-windows/d3d9.dll");
        fs::create_dir_all(component.parent().unwrap()).unwrap();
        fs::write(&component, b"dxvk-d3d9").unwrap();
        fs::create_dir_all(wine_root.join("share/macroton/components")).unwrap();
        let checksum = sha256::to_hex(&sha256::digest_file(&component).unwrap());
        fs::write(
            wine_root.join("share/macroton/components/dxvk-d3d9-moltenvk-1.conf"),
            format!("schema=1\nid=dxvk-d3d9-moltenvk-1\nfile.i386-windows/d3d9.dll={checksum}\n"),
        )
        .unwrap();
        assert!(build(&game, &profile, &runtime_root).is_ready());

        let mismatch = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=dxvk-d3d9-moltenvk\ngraphics_api=d3d11\nexecutable=game.exe",
        )
        .unwrap();
        assert!(
            build(&game, &mismatch, &runtime_root)
                .blockers
                .iter()
                .any(|value| value.contains("not d3d11"))
        );

        let mislabeled = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=wined3d-opengl\ngraphics_api=d3d9\nexecutable=game.exe",
        )
        .unwrap();
        let rejected = build(&game, &mislabeled, &runtime_root);
        assert!(rejected.blockers.iter().any(|value| {
            value.contains("cannot use a runtime with a verified DXVK d3d9 component")
        }));
    }

    #[test]
    fn rejects_profile_graphics_api_that_contradicts_pe_imports() {
        let temp = TempDir::new();
        let installed = temp.0.join("steamapps/common/Example");
        fs::create_dir_all(&installed).unwrap();
        write_pe_with_import(&installed.join("game.exe"), "d3d9.dll\0");
        let game = SteamGame {
            app_id: 42,
            name: "Example".to_owned(),
            install_dir: installed,
            manifest_path: temp.0.join("steamapps/appmanifest_42.acf"),
            state_flags: None,
            build_id: None,
            installed_depots: BTreeMap::new(),
            classification: Classification::WindowsPayload,
            binaries: Vec::new(),
            inspection_truncated: false,
        };
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=wine-default\ngraphics_api=d3d11\nexecutable=game.exe",
        )
        .unwrap();
        let plan = build(&game, &profile, &temp.0.join("runtimes"));
        assert_eq!(plan.graphics_evidence, ["d3d9"]);
        assert!(plan.blockers.iter().any(|blocker| {
            blocker.contains("graphics_api=d3d11") && blocker.contains("evidence [d3d9]")
        }));
    }
}
