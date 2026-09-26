use crate::plan::LaunchPlan;
use crate::profile::renderer_environment;
use crate::profile::{GraphicsApi, LaunchMode, SteamClientMode};
use crate::runtime::RuntimeKind;
use crate::sha256;
use crate::steam_prefix_status;
use crate::steam_session;
use crate::stop;
use std::env;
use std::fmt;
use std::fs;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Steam's optional overlay renderer currently blocks some legacy games before
/// their first window on macOS Wine. Keep Steamworks authentication enabled,
/// but prevent the two overlay DLLs from being injected into game processes.
const STEAM_OVERLAY_DLL_OVERRIDES: &str = "gameoverlayrenderer=d;gameoverlayrenderer64=d";
// Steam's Windows client can accept -applaunch before its CEF/login session
// has finished initializing. Legacy Win32 games are sensitive to that race;
// measured Wine CEF cold starts on this host commonly take 69–82 seconds.
// Warm the authenticated client first, then submit through the single-instance
// handoff. Keep the value overrideable for faster local experiments.
const STEAM_GAME_CLIENT_WARMUP: Duration = Duration::from_secs(90);
const STEAM_HANDOFF_RETRY_DELAY: Duration = Duration::from_secs(8);
// Wine's macOS WoW64 exception path can enter an RtlUnwindEx loop when the
// host stack is still at the macOS default.  Give legacy 32-bit games enough
// headroom for the unwind handler to run.  This is an inherited process limit;
// it does not allocate 64 MiB up front.
#[cfg(target_os = "macos")]
const LEGACY_WINE_STACK_LIMIT: u64 = 64 * 1024 * 1024;
const PREFIX_RUNTIME_IDENTITY: &str = ".macroton-runtime-identity";
const PREFIX_CRASH_GUARD: &str = ".macroton-crash-guard";
// This opt-in repair is activated only after the reversible CEF wrapper has
// been installed in a prefix. Steam updates replace steamwebhelper.exe, so a
// later launch must restore the wrapper around the *new* Valve executable.
const STEAM_CEF_WRAPPER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/steamwebhelper-wrapper.exe"));
// `-silent` hides Steam's desktop window. During `-applaunch`, Steam can then
// wait forever at ShowInterstitials/CreatingProcess with no surface that can
// receive confirmation input. Keep the game-launch client visible; login has
// its own argument set below and remains unchanged.
// Steam's Wine client can surface its CWorkThreadPool shutdown assertion as
// a modal Windows dialog. Keep the assertion in the client logs, but suppress
// the modal box so it cannot cover a game's own window during handoff.
const STEAM_STANDARD_ARGUMENTS: [&str; 3] = ["-tcp", "-nocrashdialog", "-nofriendsui"];
const STEAM_RENDERED_LOGIN_ARGUMENTS: [&str; 2] = ["-tcp", "-nofriendsui"];
const STEAM_OPAQUE_SOFTWARE_ARGUMENTS: [&str; 5] = [
    "-tcp",
    "-nofriendsui",
    "-cef-enable-debugging",
    "-cef-disable-gpu",
    "-cef-force-opaque-backgrounds",
];
// Keep the localhost CEF endpoint available for the owner-assisted login QR
// capture, but avoid the GPU process fan-out that can push Steamwebhelper over
// a GiB on Apple Silicon Wine.
const STEAM_CEF_DEBUG_ARGUMENTS: [&str; 3] = ["-tcp", "-cef-enable-debugging", "-cef-disable-gpu"];
// Wine's CEF compositor leaves Steam's transparent popup surface black.
// Force an opaque surface in the default low-memory path so the actual Steam
// window is visible instead of relying on the CDP screenshot fallback.
const STEAM_LOW_MEMORY_ARGUMENTS: [&str; 5] = [
    "-tcp",
    "-nofriendsui",
    "-nochatui",
    "-cef-disable-gpu",
    "-cef-force-opaque-backgrounds",
];
const STEAM_LOGIN_STANDARD_ARGUMENTS: [&str; 1] = ["-tcp"];
// Keep CEF GPU compositing enabled for this diagnostic path; disabling it is
// the source of a fully black native Wine surface on some macOS displays.
const STEAM_LOGIN_RENDERED_ARGUMENTS: [&str; 4] = [
    "-tcp",
    "-nofriendsui",
    "-cef-enable-debugging",
    "-cef-force-opaque-backgrounds",
];
const STEAM_LOGIN_OPAQUE_SOFTWARE_ARGUMENTS: [&str; 5] = [
    "-tcp",
    "-nofriendsui",
    "-cef-enable-debugging",
    "-cef-disable-gpu",
    "-cef-force-opaque-backgrounds",
];
const STEAM_LOGIN_CEF_DEBUG_ARGUMENTS: [&str; 3] =
    ["-tcp", "-cef-enable-debugging", "-cef-disable-gpu"];
// Keep a localhost DevTools endpoint in the low-memory login path so a black
// Wine-composited surface still has a deterministic QR/CEF fallback.
const STEAM_LOGIN_LOW_MEMORY_ARGUMENTS: [&str; 5] = [
    "-tcp",
    "-nofriendsui",
    "-cef-enable-debugging",
    "-cef-disable-gpu",
    "-cef-force-opaque-backgrounds",
];
const DISABLED_DEBUGGER_COMMAND: &str =
    r"C:\windows\system32\macroton-disabled-debugger.exe %ld %ld";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionResult {
    pub app_id: u32,
    pub steam_build_id: Option<String>,
    pub windows_depot_id: Option<u32>,
    pub windows_depots: Vec<crate::profile::DepotSpec>,
    pub profile_sha256: String,
    pub runtime_id: String,
    pub runtime_kind: RuntimeKind,
    pub runtime_fingerprint_sha256: String,
    pub runtime_components: Vec<String>,
    pub renderer: String,
    pub graphics_api: GraphicsApi,
    pub graphics_evidence: Vec<String>,
    pub launch_mode: LaunchMode,
    pub exit_code: Option<i32>,
    pub started_unix_ms: u128,
    pub finished_unix_ms: u128,
    pub result_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamLaunch {
    pub client: PathBuf,
    pub appmanifest: PathBuf,
    pub library_link: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamClientRequest {
    pub pid: u32,
    pub client: PathBuf,
    pub runtime_id: String,
    pub profile_sha256: String,
    pub client_mode: SteamClientMode,
    pub log: PathBuf,
    pub ui_log: PathBuf,
    pub ui_log_offset: u64,
    pub console_log: PathBuf,
    pub console_log_offset: u64,
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

pub fn execute(plan: &LaunchPlan, data_root: &Path) -> Result<ExecutionResult, Error> {
    execute_with_client_warmup(plan, data_root, steam_game_client_warmup())
}

const STOP_REQUEST_MARKER: &str = ".macroton-stop-request";

pub fn record_stop_request(app_id: u32, data_root: &Path) -> Result<(), Error> {
    let game_root = data_root.join("games").join(app_id.to_string());
    fs::create_dir_all(&game_root).map_err(|error| Error {
        message: format!("cannot create stop request directory: {error}"),
    })?;
    let marker = game_root.join(STOP_REQUEST_MARKER);
    let token = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );
    fs::write(&marker, token).map_err(|error| Error {
        message: format!("cannot record stop request {}: {error}", marker.display()),
    })
}

fn stop_marker_snapshot(game_root: &Path) -> Result<Option<Vec<u8>>, Error> {
    match fs::read(game_root.join(STOP_REQUEST_MARKER)) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error {
            message: format!("cannot inspect stop request marker: {error}"),
        }),
    }
}

fn ensure_launch_not_stopped(game_root: &Path, starting_stop_marker: &Option<Vec<u8>>) -> Result<(), Error> {
    let current = stop_marker_snapshot(game_root)?;
    if &current != starting_stop_marker {
        return Err(Error {
            message: "launch cancelled by an explicit stop request for this AppID".to_owned(),
        });
    }
    Ok(())
}

fn execute_with_client_warmup(
    plan: &LaunchPlan,
    data_root: &Path,
    client_warmup: Duration,
) -> Result<ExecutionResult, Error> {
    if !plan.is_ready() {
        return Err(Error {
            message: format!("launch plan is blocked: {}", plan.blockers.join("; ")),
        });
    }
    let runtime_entrypoint = plan.runtime_entrypoint.as_ref().ok_or_else(|| Error {
        message: "ready plan is missing its runtime entrypoint".to_owned(),
    })?;
    let runtime_kind = plan.runtime_kind.ok_or_else(|| Error {
        message: "ready plan is missing its runtime kind".to_owned(),
    })?;
    let runtime_fingerprint_sha256 =
        plan.runtime_fingerprint_sha256
            .clone()
            .ok_or_else(|| Error {
                message: "ready plan is missing its runtime fingerprint".to_owned(),
            })?;
    let game_root = data_root.join("games").join(plan.app_id.to_string());
    let starting_stop_marker = stop_marker_snapshot(&game_root)?;
    let prefix = game_root.join("prefix");
    if plan.launch_mode == LaunchMode::Steam {
        require_valid_steam_bootstrap(plan, &prefix)?;
    }
    let results = game_root.join("results");
    fs::create_dir_all(&prefix).map_err(|error| Error {
        message: format!(
            "cannot create isolated prefix {}: {error}",
            prefix.display()
        ),
    })?;
    fs::create_dir_all(&results).map_err(|error| Error {
        message: format!(
            "cannot create result directory {}: {error}",
            results.display()
        ),
    })?;
    // A direct diagnostic may intentionally use a different Wine build than
    // the authenticated Steam prefix. Do not rewrite the prefix identity in
    // that case: doing so would leave the bootstrap marker and identity out of
    // sync and make the next legitimate Steam launch fail provenance checks.
    // Fresh direct prefixes still receive an identity so they remain auditable.
    let bootstrap_marker = prefix.join(".macroton-steam-bootstrap.json");
    if plan.launch_mode == LaunchMode::Steam || !bootstrap_marker.is_file() {
        ensure_prefix_runtime_identity(
            &prefix,
            runtime_entrypoint,
            &plan.runtime_id,
            &runtime_fingerprint_sha256,
            &plan.runtime_components,
        )?;
    }
    ensure_crash_debugger_guard(&prefix, runtime_entrypoint, &runtime_fingerprint_sha256)?;
    ensure_game_compatibility_aliases(plan.app_id, &prefix)?;
    ensure_profile_startup_config(&prefix, plan.environment.get("MACROTON_STARTUP_CFG"))?;
    ensure_aoe3_graphics_profile(&prefix, plan.environment.get("MACROTON_AOE3_PROFILE_XML"))?;
    if plan.launch_mode == LaunchMode::Steam {
        require_compatible_active_steam_session(
            plan,
            data_root,
            &prefix,
            &runtime_fingerprint_sha256,
        )?;
        repair_opted_in_steam_cef_wrapper(plan.app_id, data_root, &prefix)?;
    }

    let started_unix_ms = unix_ms();
    let mut command = Command::new(runtime_entrypoint);
    match plan.launch_mode {
        LaunchMode::Direct => {
            // Inventory plans resolve the payload from the host Steam depot,
            // but a direct run still executes inside the AppID's Wine prefix.
            // Reuse the same private Steam-library view used by Steam runs;
            // otherwise a diagnostic/direct profile silently points Wine at
            // a macOS host path and never reaches the Windows game.
            let executable = resolve_prefix_game_executable(plan, &prefix);
            command.arg(&executable).args(&plan.arguments).current_dir(
                executable.parent().ok_or_else(|| Error {
                    message: "game executable has no parent directory".to_owned(),
                })?,
            );
        }
        LaunchMode::Steam => {
            // Start the authenticated Steam client as its own phase.  The
            // second Steam.exe invocation below is only a protocol handoff;
            // this mirrors a user clicking Play after the client is visible
            // and avoids launching a game during the client's first CEF boot.
            // If the authenticated client is already alive for this exact
            // prefix/profile, reuse it instead of spawning another CEF tree.
            // This is the normal post-login path and prevents Steam's web UI
            // from multiplying every time a game is launched.
            let steam_already_live = steam_session_processes_present(plan.app_id, data_root);
            if !steam_already_live {
                ensure_launch_not_stopped(&game_root, &starting_stop_marker)?;
                let request = launch_steam_client_action(plan, data_root, None, true, None)?;
                wait_for_steam_client_ready_or_stop(
                    &request,
                    client_warmup,
                    &game_root,
                    &starting_stop_marker,
                )?;
            }
            ensure_launch_not_stopped(&game_root, &starting_stop_marker)?;
            let steam = prepare_steam_launch(plan, &prefix)?;
            command.arg(&steam.client);
            append_steam_client_arguments(&mut command, plan.steam_client_mode);
            // Steam normally launches a game from its own install directory,
            // but a Macroton launcher-bypass view is a private symlink tree
            // that Steam does not know about.  Games such as Heroes III use
            // the process working directory to resolve relative `data/`
            // assets; inheriting Steam's directory makes bitmapPAK/LOD loads
            // fail even though the payload files are present.  Keep the
            // normal Steam root for ordinary games and use the private game
            // view only for explicit launcher-bypass profiles.
            let working_directory = if plan.steam_launcher_bypass.is_some() {
                steam.library_link.as_path()
            } else {
                steam.client.parent().ok_or_else(|| Error {
                    message: "Steam client has no parent directory".to_owned(),
                })?
            };
            let launch_request =
                steam_launch_request(plan.app_id, steam_already_live, plan.arguments.is_empty());
            if launch_request.len() == 1 {
                // A second Steam.exe invocation commonly reaches the existing
                // Wine single-instance client without forwarding -applaunch.
                // Use the protocol action as the primary request when the
                // authenticated client is already resident and this is an
                // ordinary launch with no game-specific arguments.  A
                // launcher-bypass profile carries required handoff arguments
                // (for example Heroes III's --fromLauncher); steam:// loses
                // those arguments, so keep -applaunch for that case.
                command.arg(launch_request[0].as_str());
            } else {
                command.args(&launch_request);
            }
            command.args(&plan.arguments).current_dir(working_directory);
        }
    }
    command.envs(&plan.environment).env("WINEPREFIX", &prefix);
    // Some external Wine bundles keep MoltenVK beside the runtime's Unix
    // Wine libraries rather than embedding an absolute install name.  Make
    // that dependency discoverable for Vulkan profiles while preserving any
    // caller-provided search path after the verified runtime paths.
    if plan.renderer.contains("vulkan")
        && let Some(library_path) = runtime_dyld_library_path(runtime_entrypoint)
    {
        command.env("DYLD_LIBRARY_PATH", library_path);
    }
    if let Some((name, value)) = renderer_environment(&plan.renderer) {
        command.env(name, value);
    }
    if plan.launch_mode == LaunchMode::Steam {
        // Steam's shader-precache helper (`bin\\gldriverquery.exe`) is a
        // separate Win32 process.  It imports SDL2.dll, but Steam's Windows
        // install does not always ship that DLL beside the helper.  Legacy
        // games commonly bundle the matching SDL2.dll in their install root;
        // expose that root through Wine's DLL search path so the helper can
        // start without copying or mutating either the authenticated Steam
        // tree or the game payload.
        if let Some(install_dir) = plan.steam_install_dir.as_deref() {
            let game_root = format!(r"C:\Program Files (x86)\Steam\steamapps\common\{install_dir}");
            let winepath = append_winepath(plan.environment.get("WINEPATH"), &game_root);
            command.env("WINEPATH", winepath);
        }
        // Steam normally provides these identifiers to a game launched from
        // its client.  Keep them explicit at the Wine process boundary too:
        // older Win32 titles often use SteamAppId/SteamGameId while locating
        // steam_api.dll or deciding whether SteamAPI_Init may proceed.  This
        // does not bypass ownership or Steamworks IPC; the real Steam client
        // still owns the -applaunch request and the game's API calls.
        command
            .env("SteamAppId", plan.app_id.to_string())
            .env("SteamGameId", plan.app_id.to_string());
        command.env(
            "WINEDLLOVERRIDES",
            launch_dll_overrides(&plan.renderer, plan.environment.get("WINEDLLOVERRIDES")),
        );
    }
    apply_legacy_wine_stack_limit(&mut command);
    let exit_code = match plan.launch_mode {
        LaunchMode::Direct => command
            .status()
            .map_err(|error| Error {
                message: format!(
                    "cannot start managed runtime {}: {error}",
                    runtime_entrypoint.display()
                ),
            })?
            .code(),
        LaunchMode::Steam => {
            ensure_launch_not_stopped(&game_root, &starting_stop_marker)?;
            // Steam is the long-lived owner of the game process. Detach it
            // from Macroton's short-lived terminal process group just as the
            // dedicated login path does; otherwise a PTY hangup can terminate
            // Wine after the launch request is submitted but before Steam
            // creates the game process.
            #[cfg(unix)]
            command.process_group(0);
            // Keep an auditable request boundary even when Wine's single
            // instance Steam client consumes the second invocation without
            // emitting a corresponding gameprocess event. This distinguishes
            // "Macroton submitted a request" from "Steam accepted it".
            record_steam_handoff_request(
                &game_root,
                plan.app_id,
                &prefix.join("drive_c/Program Files (x86)/Steam/Steam.exe"),
                &plan.arguments,
                plan.steam_client_mode,
            )?;
            command.spawn().map_err(|error| Error {
                message: format!(
                    "cannot submit Steam launch through managed runtime {}: {error}",
                    runtime_entrypoint.display()
                ),
            })?;
            steam_session::record(
                &prefix,
                &steam_session_identity(plan, &runtime_fingerprint_sha256),
            )
            .map_err(|error| Error {
                message: format!("cannot bind submitted Steam launch to its profile: {error}"),
            })?;
            // Wine Steam may accept -applaunch before its single-instance IPC
            // endpoint is ready. Give the client a bounded series of protocol
            // retries instead of assuming one fixed delay is enough. This is
            // still fail-closed to the exact AppID prefix and never falls
            // back to a direct executable launch.
            const MAX_PROTOCOL_RETRIES: usize = 5;
            // A protocol retry is safe only for ordinary launches.  A
            // launcher-bypass profile carries required game arguments; the
            // steam:// URI cannot carry them and would silently relaunch the
            // public launcher without --fromLauncher.  Keep the original
            // -applaunch request authoritative for those profiles and let
            // launch-status report a failed handoff instead of changing the
            // command line on retry.
            if plan.arguments.is_empty() {
                let retry = format!("steam://rungameid/{}", plan.app_id);
                for _ in 0..MAX_PROTOCOL_RETRIES {
                    wait_for_retry_or_stop(
                        steam_handoff_retry_delay(),
                        &game_root,
                        &starting_stop_marker,
                    )?;
                    if steam_game_process_present(plan.app_id, data_root) {
                        break;
                    }
                    let _ = launch_steam_client_action(
                        plan,
                        data_root,
                        Some(retry.clone()),
                        false,
                        None,
                    );
                }
            } else {
                // Any profile with game-specific switches must retain them on
                // retry.  This includes launcher-bypass profiles (for example
                // Heroes III's --version/--forceANGLE), but also the original
                // Steam launcher path: a steam:// URI cannot carry arguments
                // and would silently relaunch the public launcher instead.
                // Repeat the exact -applaunch request only while no real game
                // process has appeared, keeping the handoff fail-closed.
                const MAX_ARGUMENT_HANDOFF_RETRIES: usize = 3;
                for _ in 0..MAX_ARGUMENT_HANDOFF_RETRIES {
                    wait_for_retry_or_stop(
                        steam_handoff_retry_delay(),
                        &game_root,
                        &starting_stop_marker,
                    )?;
                    if steam_game_process_present(plan.app_id, data_root) {
                        break;
                    }
                    let _ = launch_steam_client_action(
                        plan,
                        data_root,
                        None,
                        false,
                        Some(&plan.arguments),
                    );
                }
            }
            None
        }
    };
    let finished_unix_ms = unix_ms();
    let result_path = results.join(format!("{started_unix_ms}.json"));
    let result = ExecutionResult {
        app_id: plan.app_id,
        steam_build_id: plan.steam_build_id.clone(),
        windows_depot_id: plan.windows_depot_id,
        windows_depots: plan.windows_depots.clone(),
        profile_sha256: plan.profile_sha256.clone(),
        runtime_id: plan.runtime_id.clone(),
        runtime_kind,
        runtime_fingerprint_sha256,
        runtime_components: plan.runtime_components.clone(),
        renderer: plan.renderer.clone(),
        graphics_api: plan.graphics_api,
        graphics_evidence: plan.graphics_evidence.clone(),
        launch_mode: plan.launch_mode,
        exit_code,
        started_unix_ms,
        finished_unix_ms,
        result_path: result_path.clone(),
    };
    write_result(&result)?;
    Ok(result)
}

fn steam_launch_request(
    app_id: u32,
    steam_already_live: bool,
    no_game_arguments: bool,
) -> Vec<String> {
    if steam_already_live && no_game_arguments {
        vec![format!("steam://rungameid/{app_id}")]
    } else {
        vec!["-applaunch".to_owned(), app_id.to_string()]
    }
}

fn record_steam_handoff_request(
    game_root: &Path,
    app_id: u32,
    steam_client: &Path,
    arguments: &[String],
    mode: SteamClientMode,
) -> Result<(), Error> {
    let path = game_root.join("steam-handoff.log");
    let mut line = format!(
        "timestamp_unix_ms={} appid={} mode={} executable={} args=-applaunch {}",
        unix_ms(),
        app_id,
        mode.as_str(),
        steam_client.display(),
        app_id,
    );
    for argument in arguments {
        line.push(' ');
        line.push_str(argument);
    }
    line.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))
        .map_err(|error| Error {
            message: format!(
                "cannot record Steam handoff request {}: {error}",
                path.display()
            ),
        })
}

/// Some legacy Windows games read startup switches from a file under the
/// Wine user's "My Documents" tree rather than from Steam's command line.
/// Profiles opt in with `env.MACROTON_STARTUP_CFG`, using comma/semicolon
/// separated lines. Keep this scoped to the isolated AppID prefix and replace
/// only Macroton-managed AoE3 startup directives so ordinary user settings are
/// preserved and switching profiles remains reproducible.
fn ensure_profile_startup_config(prefix: &Path, config: Option<&String>) -> Result<(), Error> {
    let Some(config) = config else {
        return Ok(());
    };
    let directives: Vec<&str> = config
        .split([',', ';'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if directives.is_empty() {
        return Ok(());
    }
    let users = prefix.join("drive_c/users");
    let entries = fs::read_dir(&users).map_err(|error| Error {
        message: format!("cannot inspect Wine users for startup config: {error}"),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| Error {
            message: format!("cannot inspect Wine user for startup config: {error}"),
        })?;
        if entry.file_name() == "Public" || !entry.path().is_dir() {
            continue;
        }
        let startup = entry
            .path()
            .join("My Documents/My Games/Age of Empires 3/Startup");
        fs::create_dir_all(&startup).map_err(|error| Error {
            message: format!(
                "cannot create AoE3 Startup directory {}: {error}",
                startup.display()
            ),
        })?;
        let user_cfg = startup.join("user.cfg");
        let existing = fs::read_to_string(&user_cfg).unwrap_or_default();
        let managed = [
            "nostartup",
            "nointrocinematics",
            "window",
            "dontcheckunsupported",
        ];
        let mut contents = existing
            .lines()
            .filter(|line| {
                !managed
                    .iter()
                    .any(|directive| line.trim().eq_ignore_ascii_case(directive))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !contents.is_empty() {
            contents.push('\n');
        }
        for directive in &directives {
            contents.push_str(directive);
            contents.push('\n');
        }
        fs::write(&user_cfg, contents).map_err(|error| Error {
            message: format!(
                "cannot write AoE3 startup config {}: {error}",
                user_cfg.display()
            ),
        })?;
    }
    Ok(())
}

/// Seed the legacy AoE3 user profile only for an explicitly opted-in
/// experiment.  AoE3 reads its graphics mode from Users/NewProfile.xml (or
/// NewProfile3.xml for the expansion), not from the Startup file used by the
/// other startup switches.  Keeping this behind an environment key makes the
/// workaround auditable and prevents it from changing unrelated prefixes.
fn ensure_aoe3_graphics_profile(prefix: &Path, mode: Option<&String>) -> Result<(), Error> {
    if mode.is_none_or(|value| !value.eq_ignore_ascii_case("window")) {
        return Ok(());
    }
    let contents = r#"<?xml version="1.0" encoding="UTF-8"?>
<Profile Version="10" GameID="1">
<UserInformation><Name>Macroton</Name><ESOName></ESOName><OnlineName></OnlineName></UserInformation>
<GameSettings Name="GameOptions" Def="cDefGameOptions">
<Settings Version="53">
<Setting Name="optiongrfxres">1024 x 768</Setting>
<Setting Name="optiongrfxbitdepth">32</Setting>
<Setting Name="optiongrfxwindowmode">true</Setting>
<Setting Name="optionmusiconoff">true</Setting>
<Setting Name="optionsoundonoff">true</Setting>
</Settings>
</GameSettings>
</Profile>
"#;
    let users = prefix.join("drive_c/users");
    for entry in fs::read_dir(&users).map_err(|error| Error {
        message: format!("cannot inspect Wine users for AoE3 profile: {error}"),
    })? {
        let entry = entry.map_err(|error| Error {
            message: format!("cannot inspect Wine user for AoE3 profile: {error}"),
        })?;
        if entry.file_name() == "Public" || !entry.path().is_dir() {
            continue;
        }
        let users_root = entry
            .path()
            .join("My Documents/My Games/Age of Empires 3/Users");
        fs::create_dir_all(&users_root).map_err(|error| Error {
            message: format!(
                "cannot create AoE3 profile directory {}: {error}",
                users_root.display()
            ),
        })?;
        let profile = users_root.join("NewProfile.xml");
        if !profile.is_file() {
            fs::write(&profile, contents).map_err(|error| Error {
                message: format!(
                    "cannot write AoE3 graphics profile {}: {error}",
                    profile.display()
                ),
            })?;
        }
    }
    Ok(())
}

fn resolve_prefix_game_executable(plan: &LaunchPlan, prefix: &Path) -> PathBuf {
    let Some(install_dir) = plan.steam_install_dir.as_deref() else {
        return plan.executable.clone();
    };
    let Ok(relative) = plan.executable.strip_prefix(&plan.payload_root) else {
        return plan.executable.clone();
    };
    let candidate = prefix
        .join("drive_c/Program Files (x86)/Steam/steamapps/common")
        .join(install_dir)
        .join(relative);
    if candidate.is_file() {
        candidate
    } else {
        plan.executable.clone()
    }
}

pub fn launch_steam_client(
    plan: &LaunchPlan,
    data_root: &Path,
) -> Result<SteamClientRequest, Error> {
    launch_steam_client_action(plan, data_root, None, false, None)
}

fn repair_opted_in_steam_cef_wrapper(
    app_id: u32,
    data_root: &Path,
    prefix: &Path,
) -> Result<(), Error> {
    let cef = prefix.join("drive_c/Program Files (x86)/Steam/bin/cef/cef.win64");
    let helper = cef.join("steamwebhelper.exe");
    let real = cef.join("steamwebhelper_real.exe");
    // No wrapper installation means no opt-in and no Steam binary mutation.
    if !real.exists() {
        return Ok(());
    }
    for path in [&helper, &real] {
        let metadata = fs::symlink_metadata(path).map_err(|error| Error {
            message: format!("cannot inspect opted-in Steam CEF helper {}: {error}", path.display()),
        })?;
        if !metadata.file_type().is_file() {
            return Err(Error {
                message: format!("Steam CEF helper must be a regular file: {}", path.display()),
            });
        }
    }
    if sha256::digest_file(&helper).map_err(|error| Error {
        message: format!("cannot hash Steam CEF helper {}: {error}", helper.display()),
    })? == sha256::digest_bytes(STEAM_CEF_WRAPPER)
    {
        return Ok(());
    }
    let resources = stop::inspect_resources(app_id, data_root).map_err(|error| Error {
        message: format!("cannot check whether Steam CEF repair is safe: {error}"),
    })?;
    if !resources.pids.is_empty() {
        return Err(Error {
            message: "Steam updated the opted-in CEF wrapper while this exact prefix is live; stop it with --preserve-session before restarting so Macroton can repair the UI".to_owned(),
        });
    }
    let replacement = cef.join("steamwebhelper.macroton-new.exe");
    if replacement.exists() {
        return Err(Error {
            message: format!("CEF repair staging path already exists: {}", replacement.display()),
        });
    }
    fs::write(&replacement, STEAM_CEF_WRAPPER).map_err(|error| Error {
        message: format!("cannot stage Steam CEF wrapper {}: {error}", replacement.display()),
    })?;
    let previous = cef.join(format!("steamwebhelper_real.previous.{}.exe", unix_ms()));
    fs::rename(&real, &previous).map_err(|error| Error {
        message: format!("cannot preserve previous Valve CEF helper {}: {error}", previous.display()),
    })?;
    fs::rename(&helper, &real).map_err(|error| Error {
        message: format!("cannot promote updated Valve CEF helper {}: {error}", real.display()),
    })?;
    fs::rename(&replacement, &helper).map_err(|error| Error {
        message: format!("cannot restore Macroton CEF wrapper {}: {error}", helper.display()),
    })?;
    Ok(())
}

/// Submit an installation request to the Windows Steam client bound to this
/// AppID's isolated prefix. The URI is constructed exclusively from the
/// numeric AppID so callers cannot inject arbitrary Steam protocol actions.
pub fn request_steam_install(
    plan: &LaunchPlan,
    data_root: &Path,
) -> Result<SteamClientRequest, Error> {
    launch_steam_client_action(
        plan,
        data_root,
        Some(steam_install_uri(plan.app_id)),
        false,
        None,
    )
}

fn steam_install_uri(app_id: u32) -> String {
    format!("steam://install/{app_id}")
}

fn launch_steam_client_action(
    plan: &LaunchPlan,
    data_root: &Path,
    steam_uri: Option<String>,
    game_warmup: bool,
    game_arguments: Option<&[String]>,
) -> Result<SteamClientRequest, Error> {
    if !plan.is_ready() || plan.launch_mode != LaunchMode::Steam {
        return Err(Error {
            message: "Steam login requires a ready launch=steam plan".to_owned(),
        });
    }
    let runtime_entrypoint = plan.runtime_entrypoint.as_ref().ok_or_else(|| Error {
        message: "ready plan is missing its runtime entrypoint".to_owned(),
    })?;
    let runtime_fingerprint = plan
        .runtime_fingerprint_sha256
        .as_deref()
        .ok_or_else(|| Error {
            message: "ready plan is missing its runtime fingerprint".to_owned(),
        })?;
    let prefix = data_root
        .join("games")
        .join(plan.app_id.to_string())
        .join("prefix");
    require_valid_steam_bootstrap(plan, &prefix)?;
    let client = prefix.join("drive_c/Program Files (x86)/Steam/Steam.exe");
    if !client.is_file() {
        return Err(Error {
            message: format!("Windows Steam is not installed at {}", client.display()),
        });
    }
    ensure_prefix_runtime_identity(
        &prefix,
        runtime_entrypoint,
        &plan.runtime_id,
        runtime_fingerprint,
        &plan.runtime_components,
    )?;
    ensure_crash_debugger_guard(&prefix, runtime_entrypoint, runtime_fingerprint)?;
    require_compatible_active_steam_session(plan, data_root, &prefix, runtime_fingerprint)?;
    repair_opted_in_steam_cef_wrapper(plan.app_id, data_root, &prefix)?;

    let client_parent = client.parent().ok_or_else(|| Error {
        message: "Steam client has no parent directory".to_owned(),
    })?;
    let log = data_root
        .join("games")
        .join(plan.app_id.to_string())
        .join("steam-client.log");
    let ui_log = client_parent.join("logs/webhelper.txt");
    let ui_log_offset = fs::metadata(&ui_log)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let console_log = client_parent.join("logs/console_log.txt");
    let console_log_offset = fs::metadata(&console_log)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|error| Error {
            message: format!("cannot open Steam client log {}: {error}", log.display()),
        })?;
    let stderr = stdout.try_clone().map_err(|error| Error {
        message: format!("cannot clone Steam client log handle: {error}"),
    })?;
    let mut command = Command::new(runtime_entrypoint);
    if steam_uri.is_some() || game_warmup {
        configure_steam_action_command(
            &mut command,
            &client,
            client_parent,
            &prefix,
            plan.steam_client_mode,
        );
    } else {
        configure_steam_login_command(
            &mut command,
            &client,
            client_parent,
            &prefix,
            plan.steam_client_mode,
        );
    }
    // Steam runs shader-precache helpers (including gldriverquery.exe) from
    // this client process.  Give those helpers the same game-directory DLL
    // search path as the later -applaunch process, so SDL2-backed probes do
    // not fail before the game is created.
    if let Some(install_dir) = plan.steam_install_dir.as_deref() {
        let game_root = format!(r"C:\Program Files (x86)\Steam\steamapps\common\{install_dir}");
        let winepath = append_winepath(plan.environment.get("WINEPATH"), &game_root);
        command.env("WINEPATH", winepath);
    }
    if let Some(uri) = steam_uri {
        command.arg(uri);
    }
    if let Some(arguments) = game_arguments {
        command.args(steam_game_handoff_arguments(plan.app_id, arguments));
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    apply_legacy_wine_stack_limit(&mut command);
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().map_err(|error| Error {
        message: format!(
            "cannot start Windows Steam through managed runtime {}: {error}",
            runtime_entrypoint.display()
        ),
    })?;
    steam_session::record(&prefix, &steam_session_identity(plan, runtime_fingerprint)).map_err(
        |error| Error {
            message: format!("cannot bind Steam client session to its profile: {error}"),
        },
    )?;
    Ok(SteamClientRequest {
        pid: child.id(),
        client,
        runtime_id: plan.runtime_id.clone(),
        profile_sha256: plan.profile_sha256.clone(),
        client_mode: plan.steam_client_mode,
        log,
        ui_log,
        ui_log_offset,
        console_log,
        console_log_offset,
    })
}

fn steam_game_handoff_arguments(app_id: u32, arguments: &[String]) -> Vec<String> {
    let mut request = vec!["-applaunch".to_owned(), app_id.to_string()];
    request.extend(arguments.iter().cloned());
    request
}

/// Raise the host stack limit before entering Wine.  macOS WoW64 runs the
/// 32-bit guest through a 64-bit host thunk; older games that exercise deep
/// SEH unwinding can otherwise exhaust the default stack and re-enter the
/// same unwind frame indefinitely.  Keep this narrowly scoped to managed
/// Wine children and leave non-macOS builds unchanged.
fn apply_legacy_wine_stack_limit(command: &mut Command) {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: pre_exec runs in the just-forked child before any other
        // managed code, and the closure only calls the async-signal-safe
        // setrlimit syscall wrapper.
        unsafe {
            command.pre_exec(|| {
                let limit = Rlimit {
                    current: LEGACY_WINE_STACK_LIMIT,
                    maximum: LEGACY_WINE_STACK_LIMIT,
                };
                // Some restricted test sandboxes refuse to raise the child
                // limit even though the launch itself is otherwise valid.
                // Treat that as a best-effort optimization; the runtime
                // remains usable with the inherited limit.
                let _ = setrlimit(RLIMIT_STACK, &limit);
                Ok(())
            });
        }
    }
}

#[cfg(target_os = "macos")]
const RLIMIT_STACK: u32 = 3;

#[cfg(target_os = "macos")]
#[repr(C)]
struct Rlimit {
    current: u64,
    maximum: u64,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn setrlimit(resource: u32, limit: *const Rlimit) -> i32;
}

/// Silver's Windows build reads saves from `%APPDATA%\\THQNordic\\Silver\\SAVEGAME`,
/// while Steam's UFS rule watches the parent directory (`Silver\\*.*`). Keep one
/// physical save set so the game and Steam Cloud observe the same bytes. The
/// directory-level alias is retained until a stable in-game Load result proves
/// that a copied or file-linked directory is safe for this old Win32 build.
fn ensure_game_compatibility_aliases(app_id: u32, prefix: &Path) -> Result<(), Error> {
    if app_id != 606680 {
        return Ok(());
    }
    let users = prefix.join("drive_c/users");
    let entries = fs::read_dir(&users).map_err(|error| Error {
        message: format!("cannot inspect Wine users for Silver save alias: {error}"),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| Error {
            message: format!("cannot inspect Wine user for Silver save alias: {error}"),
        })?;
        let user_root = entry.path();
        if !user_root.is_dir() || entry.file_name() == "Public" {
            continue;
        }
        let save_root = user_root.join("AppData/Roaming/THQNordic/Silver");
        if !save_root.is_dir() {
            continue;
        }
        let alias = save_root.join("SAVEGAME");
        match fs::symlink_metadata(&alias) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&alias).map_err(|error| Error {
                    message: format!(
                        "cannot inspect Silver SAVEGAME alias {}: {error}",
                        alias.display()
                    ),
                })?;
                if target != Path::new(".") {
                    return Err(Error {
                        message: format!(
                            "Silver SAVEGAME alias {} points to unexpected target {}",
                            alias.display(),
                            target.display()
                        ),
                    });
                }
            }
            Ok(metadata) if metadata.is_dir() => {
                return Ok(());
            }
            Ok(_) => {
                return Err(Error {
                    message: format!(
                        "Silver SAVEGAME path exists but is not a directory: {}",
                        alias.display()
                    ),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(".", &alias).map_err(|error| Error {
                    message: format!(
                        "cannot create Silver SAVEGAME alias {}: {error}",
                        alias.display()
                    ),
                })?;
                #[cfg(not(unix))]
                return Err(Error {
                    message: "Silver SAVEGAME alias requires a Unix-compatible Wine host"
                        .to_owned(),
                });
            }
            Err(error) => {
                return Err(Error {
                    message: format!(
                        "cannot inspect Silver SAVEGAME path {}: {error}",
                        alias.display()
                    ),
                });
            }
        }
    }
    Ok(())
}

fn runtime_dyld_library_path(entrypoint: &Path) -> Option<String> {
    let runtime_root = entrypoint.parent()?.parent()?;
    let mut paths = [
        runtime_root.join("lib"),
        runtime_root.join("lib/wine/x86_64-unix"),
    ]
    .into_iter()
    .filter(|path| path.is_dir())
    .map(|path| path.display().to_string())
    .collect::<Vec<_>>();
    if let Ok(existing) = env::var("DYLD_LIBRARY_PATH")
        && !existing.is_empty()
    {
        paths.push(existing);
    }
    (!paths.is_empty()).then(|| paths.join(":"))
}

fn steam_session_identity(
    plan: &LaunchPlan,
    runtime_fingerprint_sha256: &str,
) -> steam_session::Identity {
    steam_session::Identity {
        app_id: plan.app_id,
        profile_sha256: plan.profile_sha256.clone(),
        runtime_id: plan.runtime_id.clone(),
        runtime_fingerprint_sha256: runtime_fingerprint_sha256.to_owned(),
        steam_client_mode: plan.steam_client_mode.as_str().to_owned(),
        renderer: plan.renderer.clone(),
        graphics_api: plan.graphics_api.as_str().to_owned(),
    }
}

fn require_compatible_active_steam_session(
    plan: &LaunchPlan,
    data_root: &Path,
    prefix: &Path,
    runtime_fingerprint_sha256: &str,
) -> Result<(), Error> {
    let resources = stop::inspect_resources(plan.app_id, data_root).map_err(|error| Error {
        message: format!("cannot inspect active Steam session ownership: {error}"),
    })?;
    let has_session_process = resources.processes.iter().any(|process| {
        matches!(
            process.category,
            "steam-client" | "steam-cef" | "windows-game"
        )
    });
    if !has_session_process {
        return Ok(());
    }
    steam_session::validate(
        prefix,
        &steam_session_identity(plan, runtime_fingerprint_sha256),
    )
    .map_err(|error| Error {
        message: format!(
            "managed prefix already has an incompatible Steam/game session; stop AppID {} before switching profile: {error}",
            plan.app_id
        ),
    })
}

pub fn steam_session_processes_present(app_id: u32, data_root: &Path) -> bool {
    stop::inspect_resources(app_id, data_root)
        .map(|resources| {
            resources.processes.iter().any(|process| {
                matches!(
                    process.category,
                    "steam-client" | "steam-cef" | "windows-game"
                )
            })
        })
        .unwrap_or(false)
}

fn steam_game_process_present(app_id: u32, data_root: &Path) -> bool {
    stop::inspect_resources(app_id, data_root)
        .map(|resources| {
            resources
                .processes
                .iter()
                .any(|process| process.category == "windows-game")
        })
        .unwrap_or(false)
}

fn steam_game_client_warmup() -> Duration {
    let seconds = parse_steam_game_warmup_seconds(
        std::env::var("MACROTON_STEAM_GAME_WARMUP_SECONDS")
            .ok()
            .as_deref(),
    );
    Duration::from_secs(seconds)
}

fn steam_handoff_retry_delay() -> Duration {
    let seconds = parse_steam_handoff_retry_seconds(
        std::env::var("MACROTON_STEAM_HANDOFF_RETRY_SECONDS")
            .ok()
            .as_deref(),
    );
    Duration::from_secs(seconds)
}

fn parse_steam_handoff_retry_seconds(value: Option<&str>) -> u64 {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| (1..=30).contains(seconds))
        .unwrap_or(STEAM_HANDOFF_RETRY_DELAY.as_secs())
}

/// Wait for a real Steam CEF surface before submitting `-applaunch`.
///
/// A fixed sleep is not sufficient for Wine on Apple Silicon: the client may
/// have a live Steam.exe process while its webhelper is still creating the
/// friends/game-overview surfaces.  The launch request is still submitted
/// after the bounded wait if the marker is absent, preserving the old
/// fail-open behavior for clients that do not emit this log format.
pub fn wait_for_steam_client_ready(request: &SteamClientRequest, timeout: Duration) {
    let started = Instant::now();
    loop {
        if steam_client_ui_ready(&request.ui_log, request.ui_log_offset)
            || steam_client_console_ready(&request.console_log, request.console_log_offset)
        {
            return;
        }
        if started.elapsed() >= timeout {
            return;
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn wait_for_steam_client_ready_or_stop(
    request: &SteamClientRequest,
    timeout: Duration,
    game_root: &Path,
    starting_stop_marker: &Option<Vec<u8>>,
) -> Result<(), Error> {
    let started = Instant::now();
    loop {
        ensure_launch_not_stopped(game_root, starting_stop_marker)?;
        if steam_client_ui_ready(&request.ui_log, request.ui_log_offset)
            || steam_client_console_ready(&request.console_log, request.console_log_offset)
            || started.elapsed() >= timeout
        {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1).min(timeout.saturating_sub(started.elapsed())));
    }
}

fn wait_for_retry_or_stop(
    delay: Duration,
    game_root: &Path,
    starting_stop_marker: &Option<Vec<u8>>,
) -> Result<(), Error> {
    let started = Instant::now();
    loop {
        ensure_launch_not_stopped(game_root, starting_stop_marker)?;
        let remaining = delay.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1).min(remaining));
    }
}

fn steam_client_ui_ready(path: &Path, offset: u64) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let start = usize::try_from(offset)
        .ok()
        .filter(|start| *start <= bytes.len())
        .unwrap_or(0);
    let appended = String::from_utf8_lossy(&bytes[start..]);
    ["SteamBrowser-", "Friends Li", "Game Overview"]
        .iter()
        .any(|marker| appended.contains(marker))
}

fn steam_client_console_ready(path: &Path, offset: u64) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let start = usize::try_from(offset)
        .ok()
        .filter(|start| *start <= bytes.len())
        .unwrap_or(0);
    String::from_utf8_lossy(&bytes[start..]).contains("System startup time:")
}

fn parse_steam_game_warmup_seconds(value: Option<&str>) -> u64 {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| (1..=120).contains(seconds))
        .unwrap_or(STEAM_GAME_CLIENT_WARMUP.as_secs())
}

fn require_valid_steam_bootstrap(plan: &LaunchPlan, prefix: &Path) -> Result<(), Error> {
    let expected_fingerprint = plan
        .runtime_fingerprint_sha256
        .as_deref()
        .ok_or_else(|| Error {
            message: "ready plan is missing its runtime fingerprint".to_owned(),
        })?;
    steam_prefix_status::validate_expected_runtime(
        plan.app_id,
        prefix,
        &plan.runtime_id,
        expected_fingerprint,
    )
    .map_err(|error| Error {
        message: format!(
            "Windows Steam prefix has no valid Macroton bootstrap provenance: {error}"
        ),
    })
}

fn append_steam_client_arguments(command: &mut Command, mode: SteamClientMode) {
    match mode {
        SteamClientMode::Standard => command.args(STEAM_STANDARD_ARGUMENTS),
        SteamClientMode::RenderedLoginExperimental => command.args(STEAM_RENDERED_LOGIN_ARGUMENTS),
        SteamClientMode::OpaqueSoftwareExperimental => {
            command.args(STEAM_OPAQUE_SOFTWARE_ARGUMENTS)
        }
        SteamClientMode::CefDebugExperimental => command.args(STEAM_CEF_DEBUG_ARGUMENTS),
        SteamClientMode::LowMemoryExperimental => command.args(STEAM_LOW_MEMORY_ARGUMENTS),
    };
    append_configured_cef_port(command, mode);
}

fn append_steam_login_arguments(command: &mut Command, mode: SteamClientMode) {
    match mode {
        SteamClientMode::Standard => command.args(STEAM_LOGIN_STANDARD_ARGUMENTS),
        SteamClientMode::RenderedLoginExperimental => command.args(STEAM_LOGIN_RENDERED_ARGUMENTS),
        SteamClientMode::OpaqueSoftwareExperimental => {
            command.args(STEAM_LOGIN_OPAQUE_SOFTWARE_ARGUMENTS)
        }
        SteamClientMode::CefDebugExperimental => command.args(STEAM_LOGIN_CEF_DEBUG_ARGUMENTS),
        SteamClientMode::LowMemoryExperimental => command.args(STEAM_LOGIN_LOW_MEMORY_ARGUMENTS),
    };
    append_configured_cef_port(command, mode);
}

fn append_configured_cef_port(command: &mut Command, mode: SteamClientMode) {
    let uses_cef_debugging = matches!(
        mode,
        SteamClientMode::RenderedLoginExperimental
            | SteamClientMode::OpaqueSoftwareExperimental
            | SteamClientMode::CefDebugExperimental
            | SteamClientMode::LowMemoryExperimental
    );
    if !uses_cef_debugging {
        return;
    }
    let Ok(port) = std::env::var("MACROTON_STEAM_CEF_PORT") else {
        return;
    };
    if port.parse::<u16>().is_ok() && port != "0" {
        command.arg(format!("-remote-debugging-port={port}"));
    }
}

fn configure_steam_login_command(
    command: &mut Command,
    client: &Path,
    client_parent: &Path,
    prefix: &Path,
    mode: SteamClientMode,
) {
    command.arg(client);
    append_steam_login_arguments(command, mode);
    // Login/update belongs to the Steam client, not the game. Do not leak
    // renderer selection, native game DLL overrides, overlay suppression, or
    // other profile environment into Steam.exe. The exact prefix and client
    // mode are the only per-session inputs it needs.
    command
        .env("WINEPREFIX", prefix)
        .env("WINEDEBUG", "-all")
        .current_dir(client_parent);
}

fn configure_steam_action_command(
    command: &mut Command,
    client: &Path,
    client_parent: &Path,
    prefix: &Path,
    mode: SteamClientMode,
) {
    command.arg(client);
    // Protocol actions (install, validate, run) are handed to Steam's
    // existing single-instance client. Use the normal client arguments so
    // the handoff is quiet and does not recreate the owner-login surface.
    append_steam_client_arguments(command, mode);
    command
        .env("WINEPREFIX", prefix)
        .env("WINEDEBUG", "-all")
        .current_dir(client_parent);
}

/// Wine installs `winedbg --auto` as the post-mortem debugger in both registry
/// views. If the debugger itself cannot attach (for example after a graphics
/// stack failure), every crashing helper can recursively create another
/// long-lived winedbg process. Macroton is a launcher, not a debugger, so make
/// the isolated prefix fail closed without spawning a post-mortem debugger.
///
/// The deliberately absent executable makes Wine's CreateProcess call fail
/// immediately. The original process then follows its normal unhandled-
/// exception termination path. We use Wine's registry tool instead of editing
/// system.reg because a Windows Steam wineserver may already own the prefix.
pub(crate) fn ensure_crash_debugger_guard(
    prefix: &Path,
    runtime_entrypoint: &Path,
    runtime_fingerprint_sha256: &str,
) -> Result<(), Error> {
    let marker = prefix.join(PREFIX_CRASH_GUARD);
    let desired_marker = format!(
        "schema=1\nruntime_fingerprint_sha256={runtime_fingerprint_sha256}\ndebugger={}\n",
        DISABLED_DEBUGGER_COMMAND
    );
    match fs::read_to_string(&marker) {
        Ok(current) if current == desired_marker => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Error {
                message: format!(
                    "cannot read crash guard marker {}: {error}",
                    marker.display()
                ),
            });
        }
    }

    const AEDEBUG_KEY: &str = r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\AeDebug";
    for view in ["32", "64"] {
        for (name, kind, value) in [
            ("Debugger", "REG_SZ", DISABLED_DEBUGGER_COMMAND),
            ("Auto", "REG_SZ", "1"),
        ] {
            let status = Command::new(runtime_entrypoint)
                .args([
                    "reg.exe",
                    "add",
                    AEDEBUG_KEY,
                    "/v",
                    name,
                    "/t",
                    kind,
                    "/d",
                    value,
                    "/f",
                    if view == "32" { "/reg:32" } else { "/reg:64" },
                ])
                .env("WINEPREFIX", prefix)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| Error {
                    message: format!(
                        "cannot configure {view}-bit Wine crash guard with {}: {error}",
                        runtime_entrypoint.display()
                    ),
                })?;
            if !status.success() {
                return Err(Error {
                    message: format!(
                        "cannot configure {view}-bit Wine crash guard value {name}: runtime exited with {status}"
                    ),
                });
            }
        }
    }

    let temporary = marker.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, desired_marker).map_err(|error| Error {
        message: format!(
            "cannot write temporary crash guard marker {}: {error}",
            temporary.display()
        ),
    })?;
    fs::rename(&temporary, &marker).map_err(|error| Error {
        message: format!(
            "cannot commit crash guard marker {}: {error}",
            marker.display()
        ),
    })?;
    Ok(())
}

pub(crate) fn ensure_prefix_runtime_identity(
    prefix: &Path,
    runtime_entrypoint: &Path,
    runtime_id: &str,
    runtime_fingerprint_sha256: &str,
    runtime_components: &[String],
) -> Result<(), Error> {
    let marker = prefix.join(PREFIX_RUNTIME_IDENTITY);
    let desired = format!(
        "schema=1\nruntime_id={}\nruntime_fingerprint_sha256={}\ncomponents={}\n",
        runtime_id,
        runtime_fingerprint_sha256,
        runtime_components.join(",")
    );
    let current = match fs::read_to_string(&marker) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(Error {
                message: format!(
                    "cannot read prefix runtime identity {}: {error}",
                    marker.display()
                ),
            });
        }
    };
    if current.as_deref() == Some(desired.as_str()) {
        return Ok(());
    }

    if prefix_has_active_wineserver(prefix, runtime_entrypoint)? {
        let owner = current
            .as_deref()
            .and_then(|value| {
                value
                    .lines()
                    .find_map(|line| line.strip_prefix("runtime_id="))
            })
            .unwrap_or("unowned");
        return Err(Error {
            message: format!(
                "prefix {} has an active wineserver owned by runtime {}; shut down that prefix before switching to {}",
                prefix.display(),
                owner,
                runtime_id
            ),
        });
    }

    let temporary = marker.with_extension("tmp");
    fs::write(&temporary, desired).map_err(|error| Error {
        message: format!(
            "cannot write temporary prefix runtime identity {}: {error}",
            temporary.display()
        ),
    })?;
    fs::rename(&temporary, &marker).map_err(|error| Error {
        message: format!(
            "cannot commit prefix runtime identity {}: {error}",
            marker.display()
        ),
    })
}

fn prefix_has_active_wineserver(prefix: &Path, runtime_entrypoint: &Path) -> Result<bool, Error> {
    let Some(runtime_bin) = runtime_entrypoint.parent() else {
        return Ok(false);
    };
    let wineserver = runtime_bin.join("wineserver");
    if !wineserver.is_file() {
        return Ok(false);
    }
    let mut waiter = Command::new(&wineserver)
        .arg("-w")
        .env("WINEPREFIX", prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Error {
            message: format!(
                "cannot query wineserver state with {}: {error}",
                wineserver.display()
            ),
        })?;
    // Wine startup, Rosetta translation, and concurrent process scheduling can
    // delay an otherwise immediate no-server result beyond one second. Three
    // seconds remains a bounded launch-time guard while avoiding false active
    // classifications observed under a full parallel test load.
    for _ in 0..120 {
        if waiter
            .try_wait()
            .map_err(|error| Error {
                message: format!("cannot query wineserver state: {error}"),
            })?
            .is_some()
        {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(25));
    }
    waiter.kill().map_err(|error| Error {
        message: format!("cannot stop wineserver state query: {error}"),
    })?;
    let _ = waiter.wait();
    Ok(true)
}

fn write_result(result: &ExecutionResult) -> Result<(), Error> {
    let temporary = result.result_path.with_extension("json.tmp");
    let contents = format!(
        concat!(
            "{{\n",
            "  \"schema\": 1,\n",
            "  \"appid\": {},\n",
            "  \"steam_build_id\": {},\n",
            "  \"windows_depot_id\": {},\n",
            "  \"windows_depots\": {},\n",
            "  \"profile_sha256\": \"{}\",\n",
            "  \"runtime_id\": \"{}\",\n",
            "  \"runtime_kind\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"runtime_components\": {},\n",
            "  \"renderer\": \"{}\",\n",
            "  \"graphics_api\": \"{}\",\n",
            "  \"graphics_evidence\": {},\n",
            "  \"launch_mode\": \"{}\",\n",
            "  \"exit_code\": {},\n",
            "  \"started_unix_ms\": {},\n",
            "  \"finished_unix_ms\": {}\n",
            "}}\n"
        ),
        result.app_id,
        result
            .steam_build_id
            .as_ref()
            .map(|build_id| format!("\"{}\"", json_escape(build_id)))
            .unwrap_or_else(|| "null".to_owned()),
        result
            .windows_depot_id
            .map(|depot_id| depot_id.to_string())
            .unwrap_or_else(|| "null".to_owned()),
        depots_json(&result.windows_depots),
        result.profile_sha256,
        json_escape(&result.runtime_id),
        result.runtime_kind.as_str(),
        result.runtime_fingerprint_sha256,
        strings_json(&result.runtime_components),
        json_escape(&result.renderer),
        result.graphics_api.as_str(),
        strings_json(&result.graphics_evidence),
        result.launch_mode.as_str(),
        result
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "null".to_owned()),
        result.started_unix_ms,
        result.finished_unix_ms
    );
    fs::write(&temporary, contents).map_err(|error| Error {
        message: format!(
            "cannot write temporary result record {}: {error}",
            temporary.display()
        ),
    })?;
    fs::rename(&temporary, &result.result_path).map_err(|error| Error {
        message: format!(
            "cannot commit result record {}: {error}",
            result.result_path.display()
        ),
    })
}

fn strings_json(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| format!("\"{}\"", json_escape(value)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub fn prepare_steam_launch(plan: &LaunchPlan, prefix: &Path) -> Result<SteamLaunch, Error> {
    if plan.launch_mode != LaunchMode::Steam {
        return Err(Error {
            message: "Steam preparation requires launch=steam".to_owned(),
        });
    }
    let install_dir = plan.steam_install_dir.as_deref().ok_or_else(|| Error {
        message: "Steam launch plan is missing steam_install_dir".to_owned(),
    })?;
    let steam_root = prefix
        .join("drive_c")
        .join("Program Files (x86)")
        .join("Steam");
    let client = steam_root.join("Steam.exe");
    if !client.is_file() {
        return Err(Error {
            message: format!(
                "Windows Steam is not installed in the isolated prefix: {}",
                client.display()
            ),
        });
    }
    let steamapps = steam_root.join("steamapps");
    let common = steamapps.join("common");
    fs::create_dir_all(&common).map_err(|error| Error {
        message: format!("cannot create Steam library {}: {error}", common.display()),
    })?;
    let library_link = common.join(install_dir);
    recover_interrupted_install_tree(&common, install_dir, &library_link)?;
    let needs_writable_renderer_view = plan.renderer == "dxvk-d3d9-moltenvk";
    // WineD3D profiles also need a profile-addressed view when a prior DXVK
    // experiment left a native d3d9.dll beside the executable.  Keeping the
    // view private lets us detach that override without mutating Steam's
    // authenticated install tree.
    let needs_private_renderer_view =
        needs_writable_renderer_view || plan.renderer.starts_with("wined3d-");
    let needs_launcher_bypass_view = plan.steam_launcher_bypass.is_some();
    // The GUI Steam acquisition path installs directly into this isolated
    // prefix.  Prefer that already-installed tree when it contains the
    // profiled executable; the inventory may still point at the host Steam
    // depot cache, which must not be copied into or overwrite the prefix.
    let prefix_payload = plan
        .executable
        .strip_prefix(&plan.payload_root)
        .ok()
        .map(|relative| library_link.join(relative))
        .filter(|candidate| candidate.is_file())
        .map(|_| library_link.clone());
    // Renderer and launcher-bypass transforms must never mutate Steam's
    // authenticated install tree; force a profile-addressed private view.
    let mapped_payload = if !needs_launcher_bypass_view
        && !needs_private_renderer_view
        && let Some(prefix_payload) = prefix_payload
    {
        prefix_payload
    } else if plan.payload_roots.len() > 1
        || needs_private_renderer_view
        || needs_launcher_bypass_view
    {
        let game_root = prefix.parent().ok_or_else(|| Error {
            message: format!("prefix has no game root: {}", prefix.display()),
        })?;
        // An authenticated Steam install is a real directory.  For a
        // profile-addressed transform, preserve it under a stable hidden
        // source name and expose the transformed view at Steam's library
        // path.  The view uses absolute links to that preserved source.
        let mut source_roots = plan.payload_roots.clone();
        let executable_relative = plan.executable.strip_prefix(&plan.payload_root).ok();
        // A previous profile may have rotated the hidden source suffix. If
        // the plan points at that older suffix, recover the current matching
        // source before materializing the view.
        if needs_launcher_bypass_view
            && source_roots.iter().all(|root| !root.is_dir())
            && let Ok(entries) = fs::read_dir(&common)
        {
            let prefix = format!(".{install_dir}.macroton-source-");
            if let Some(current_source) = entries.flatten().find_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?;
                (name.starts_with(&prefix)
                    && executable_relative.is_some_and(|relative| path.join(relative).is_file()))
                .then_some(path)
            }) {
                source_roots = vec![current_source];
            }
        }
        if needs_launcher_bypass_view
            && library_link.is_symlink()
            && let Ok(current) = fs::read_link(&library_link)
            && current
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".macroton-source-"))
        {
            source_roots = source_roots
                .into_iter()
                .map(|root| {
                    root.strip_prefix(&library_link)
                        .ok()
                        .map(|relative| current.join(relative))
                        .unwrap_or(root)
                })
                .collect();
        }
        if (needs_launcher_bypass_view || needs_private_renderer_view)
            && library_link.is_dir()
            && !library_link.is_symlink()
        {
            // An authenticated Steam install may contain files that are not
            // present in the inventory/depot metadata root (and it is the
            // authoritative source for a renderer or launcher transform).
            // Preserve that tree before creating the private view; using only
            // external roots can reject a valid launcher or mutate Steam's
            // installed files through a view symlink.
            let installed_tree_has_executable = plan
                .executable
                .strip_prefix(&plan.payload_root)
                .ok()
                .is_some_and(|relative| library_link.join(relative).is_file());
            let source_root = common.join(format!(
                ".{}.macroton-source-{}",
                install_dir,
                &plan.profile_sha256[..16]
            ));
            if source_root.exists() {
                if !source_root.is_dir() {
                    return Err(Error {
                        message: format!(
                            "Macroton source path is not a directory: {}",
                            source_root.display()
                        ),
                    });
                }
            } else {
                fs::rename(&library_link, &source_root).map_err(|error| Error {
                    message: format!(
                        "cannot preserve Steam install tree {}: {error}",
                        library_link.display()
                    ),
                })?;
            }
            source_roots = if installed_tree_has_executable {
                vec![source_root]
            } else {
                source_roots
                    .into_iter()
                    .map(|root| {
                        root.strip_prefix(&library_link)
                            .ok()
                            .map(|relative| source_root.join(relative))
                            .unwrap_or(root)
                    })
                    .collect()
            };
        }
        materialize_payload_view(
            &source_roots,
            &game_root.join("payload-views").join(&plan.profile_sha256),
        )?
    } else {
        plan.payload_root.clone()
    };
    if needs_launcher_bypass_view {
        deploy_steam_launcher_bypass(plan, &mapped_payload)?;
    }
    if needs_writable_renderer_view {
        deploy_dxvk_d3d9(plan, &mapped_payload)?;
    }
    if plan.renderer.starts_with("wined3d-") {
        detach_private_wined3d_d3d9(plan, &mapped_payload)?;
    }
    ensure_payload_link(&library_link, &mapped_payload)?;

    let appmanifest = steamapps.join(format!("appmanifest_{}.acf", plan.app_id));
    // Do not replace a manifest produced by the authenticated Steam client.
    // Its non-zero SizeOnDisk/installed-depot fields are the authoritative
    // acquisition record; rewriting it with Macroton's synthetic zero-size
    // launch manifest makes the next install-status report look unfinished.
    if fs::read_to_string(&appmanifest)
        .ok()
        .is_some_and(|contents| {
            manifest_size_on_disk_positive(&contents)
                && library_link.is_dir()
                && fs::read_dir(&library_link)
                    .ok()
                    .is_some_and(|mut entries| entries.next().is_some())
        })
    {
        return Ok(SteamLaunch {
            client,
            appmanifest,
            library_link,
        });
    }
    let temporary = appmanifest.with_extension("acf.tmp");
    let depot = installed_depots_vdf(&plan.windows_depots);
    let contents = format!(
        concat!(
            "\"AppState\"\n",
            "{{\n",
            "\t\"appid\"\t\t\"{}\"\n",
            "\t\"Universe\"\t\t\"1\"\n",
            "\t\"name\"\t\t\"{}\"\n",
            "\t\"StateFlags\"\t\t\"4\"\n",
            "\t\"installdir\"\t\t\"{}\"\n",
            "\t\"LastUpdated\"\t\t\"0\"\n",
            "\t\"SizeOnDisk\"\t\t\"0\"\n",
            "\t\"buildid\"\t\t\"{}\"\n",
            "{}",
            "}}\n"
        ),
        plan.app_id,
        vdf_escape(&plan.game_name),
        vdf_escape(install_dir),
        plan.windows_build_id.as_deref().unwrap_or("0"),
        depot
    );
    fs::write(&temporary, contents).map_err(|error| Error {
        message: format!(
            "cannot write temporary Steam manifest {}: {error}",
            temporary.display()
        ),
    })?;
    fs::rename(&temporary, &appmanifest).map_err(|error| Error {
        message: format!(
            "cannot commit Steam manifest {}: {error}",
            appmanifest.display()
        ),
    })?;
    Ok(SteamLaunch {
        client,
        appmanifest,
        library_link,
    })
}

fn manifest_size_on_disk_positive(contents: &str) -> bool {
    contents.lines().any(|line| {
        if !line.contains("\"SizeOnDisk\"") {
            return false;
        }
        line.split('"')
            .filter_map(|part| part.trim().parse::<u64>().ok())
            .any(|size| size > 0)
    })
}

/// Some retired games expose a web launcher that fails before their real
/// executable under Wine. Steam must still create the launch context, so a
/// profile may replace that public launcher path with a byte-for-byte copy of
/// the configured game executable. This operates only in Macroton's private,
/// fingerprinted payload view; the Steam-owned tree is never changed.
fn deploy_steam_launcher_bypass(plan: &LaunchPlan, payload_view: &Path) -> Result<(), Error> {
    let launcher_relative = plan.steam_launcher_bypass.as_ref().ok_or_else(|| Error {
        message: "launcher-bypass deployment requires a configured path".to_owned(),
    })?;
    let executable_relative = plan
        .payload_roots
        .iter()
        .find_map(|root| plan.executable.strip_prefix(root).ok())
        .ok_or_else(|| Error {
            message: format!(
                "game executable {} is outside all payload roots",
                plan.executable.display()
            ),
        })?;
    if executable_relative == launcher_relative {
        return Err(Error {
            message: "launcher bypass path must differ from the game executable".to_owned(),
        });
    }

    let source = payload_view.join(executable_relative);
    let destination = payload_view.join(launcher_relative);
    if !source.is_file() {
        return Err(Error {
            message: format!("launcher bypass source is absent: {}", source.display()),
        });
    }
    let source_digest = sha256::digest_file(&source).map_err(|error| Error {
        message: format!(
            "cannot hash launcher bypass source {}: {error}",
            source.display()
        ),
    })?;

    if destination.is_symlink() {
        let observed = fs::canonicalize(&destination).map_err(|error| Error {
            message: format!(
                "cannot resolve staged launcher {}: {error}",
                destination.display()
            ),
        })?;
        let expected = plan
            .payload_roots
            .iter()
            .map(|root| root.join(launcher_relative))
            .find(|path| path.is_file())
            // An authenticated install may have been preserved under a
            // profile-addressed hidden source before this private view was
            // materialized. In that case the staged link's trusted source
            // is the materialized view itself, not the original inventory
            // root (which may no longer exist at its public path).
            .or_else(|| {
                let candidate = payload_view.join(launcher_relative);
                candidate.is_file().then_some(candidate)
            })
            .ok_or_else(|| Error {
                message: format!(
                    "configured Steam launcher {} is absent from all payload roots",
                    launcher_relative.display()
                ),
            })?;
        let expected = fs::canonicalize(&expected).map_err(|error| Error {
            message: format!(
                "cannot resolve expected Steam launcher {}: {error}",
                expected.display()
            ),
        })?;
        if observed != expected {
            return Err(Error {
                message: format!(
                    "staged launcher {} does not map to the expected Steam payload; refusing replacement",
                    destination.display()
                ),
            });
        }
        fs::remove_file(&destination).map_err(|error| Error {
            message: format!(
                "cannot detach staged launcher link {}: {error}",
                destination.display()
            ),
        })?;
    } else if destination.is_file() {
        let destination_digest = sha256::digest_file(&destination).map_err(|error| Error {
            message: format!(
                "cannot hash staged launcher {}: {error}",
                destination.display()
            ),
        })?;
        if destination_digest == source_digest {
            return Ok(());
        }
        let backup = destination.with_file_name(format!(
            ".{}.macroton-original",
            destination
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("launcher.exe")
        ));
        if !backup.exists() {
            fs::rename(&destination, &backup).map_err(|error| Error {
                message: format!(
                    "cannot preserve original Steam launcher {}: {error}",
                    destination.display()
                ),
            })?;
        } else {
            let backup_digest = sha256::digest_file(&backup).map_err(|error| Error {
                message: format!(
                    "cannot verify preserved Steam launcher {}: {error}",
                    backup.display()
                ),
            })?;
            if backup_digest != destination_digest {
                return Err(Error {
                    message: format!(
                        "launcher bypass backup conflict at {}; refusing replacement",
                        backup.display()
                    ),
                });
            }
            fs::remove_file(&destination).map_err(|error| Error {
                message: format!(
                    "cannot remove prior launcher bypass {}: {error}",
                    destination.display()
                ),
            })?;
        }
    } else {
        return Err(Error {
            message: format!(
                "configured Steam launcher is absent from staged payload: {}",
                destination.display()
            ),
        });
    }

    let temporary = destination.with_extension(format!("exe.tmp-{}", std::process::id()));
    fs::copy(&source, &temporary).map_err(|error| Error {
        message: format!(
            "cannot stage launcher bypass {} at {}: {error}",
            source.display(),
            temporary.display()
        ),
    })?;
    let staged_digest = sha256::digest_file(&temporary).map_err(|error| Error {
        message: format!(
            "cannot verify staged launcher bypass {}: {error}",
            temporary.display()
        ),
    })?;
    if staged_digest != source_digest {
        let _ = fs::remove_file(&temporary);
        return Err(Error {
            message: "staged launcher bypass checksum differs from game executable".to_owned(),
        });
    }
    fs::rename(&temporary, &destination).map_err(|error| Error {
        message: format!(
            "cannot commit staged launcher bypass at {}: {error}",
            destination.display()
        ),
    })
}

/// DXVK is a native Windows DLL, so Wine's `d3d9=n` load order only works when
/// the DLL is in a Windows search location. Keeping it under `lib/wine` makes
/// the component verifiable, but does not make it native-loadable. Deploy the
/// verified artifact beside the game's entrypoint in Macroton's per-profile
/// payload view; never alter Steam's owned depot tree or the shared prefix.
fn deploy_dxvk_d3d9(plan: &LaunchPlan, payload_view: &Path) -> Result<(), Error> {
    let runtime_entrypoint = plan.runtime_entrypoint.as_ref().ok_or_else(|| Error {
        message: "DXVK deployment requires a resolved runtime entrypoint".to_owned(),
    })?;
    let runtime_root = runtime_entrypoint
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| Error {
            message: format!(
                "cannot resolve runtime root from {}",
                runtime_entrypoint.display()
            ),
        })?;
    let source = runtime_root.join("lib/wine/i386-windows/d3d9.dll");
    if !source.is_file() {
        return Err(Error {
            message: format!(
                "verified DXVK component has no PE32 d3d9.dll at {}",
                source.display()
            ),
        });
    }
    let executable_relative = plan
        .payload_roots
        .iter()
        .find_map(|root| plan.executable.strip_prefix(root).ok())
        .ok_or_else(|| Error {
            message: format!(
                "game executable {} is outside all payload roots",
                plan.executable.display()
            ),
        })?;
    let executable_parent = executable_relative
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let destination = payload_view.join(executable_parent).join("d3d9.dll");
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| Error {
            message: format!(
                "cannot create renderer deployment directory {}: {error}",
                parent.display()
            ),
        })?;
    }
    let source_digest = sha256::digest_file(&source).map_err(|error| Error {
        message: format!("cannot hash DXVK component {}: {error}", source.display()),
    })?;
    // Payload views normally contain symlinks back to the authenticated Steam
    // tree.  Renderer deployment is intentionally writable, so detach an
    // existing d3d9.dll link before staging the private native DLL; otherwise
    // copying through the link would mutate Steam's source installation.
    if fs::symlink_metadata(&destination)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        fs::remove_file(&destination).map_err(|error| Error {
            message: format!(
                "cannot detach source renderer link {}: {error}",
                destination.display()
            ),
        })?;
    }
    if destination.exists() {
        let destination_digest = sha256::digest_file(&destination).map_err(|error| Error {
            message: format!(
                "cannot hash deployed renderer {}: {error}",
                destination.display()
            ),
        })?;
        if destination_digest == source_digest {
            return Ok(());
        }
        return Err(Error {
            message: format!(
                "renderer deployment conflict at {}; refusing to replace a different d3d9.dll",
                destination.display()
            ),
        });
    }
    let temporary = destination.with_extension(format!("dll.tmp-{}", std::process::id()));
    fs::copy(&source, &temporary).map_err(|error| Error {
        message: format!(
            "cannot stage DXVK component {} at {}: {error}",
            source.display(),
            temporary.display()
        ),
    })?;
    let staged_digest = sha256::digest_file(&temporary).map_err(|error| Error {
        message: format!(
            "cannot verify staged DXVK component {}: {error}",
            temporary.display()
        ),
    })?;
    if staged_digest != source_digest {
        let _ = fs::remove_file(&temporary);
        return Err(Error {
            message: format!(
                "staged DXVK component checksum differs from {}",
                source.display()
            ),
        });
    }
    fs::rename(&temporary, &destination).map_err(|error| Error {
        message: format!(
            "cannot commit DXVK component at {}: {error}",
            destination.display()
        ),
    })
}

/// Remove a prior native DXVK override from a private WineD3D payload view.
/// The file is removed only from the profile view (normally a symlink), never
/// from the preserved Steam source tree.
fn detach_private_wined3d_d3d9(plan: &LaunchPlan, payload_view: &Path) -> Result<(), Error> {
    let executable_relative = plan
        .executable
        .strip_prefix(&plan.payload_root)
        .map_err(|_| Error {
            message: format!(
                "game executable {} is outside the payload root {}",
                plan.executable.display(),
                plan.payload_root.display()
            ),
        })?;
    let destination = payload_view
        .join(
            executable_relative
                .parent()
                .unwrap_or_else(|| Path::new("")),
        )
        .join("d3d9.dll");
    match fs::symlink_metadata(&destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            fs::remove_file(&destination).map_err(|error| Error {
                message: format!(
                    "cannot detach native d3d9 override from WineD3D view {}: {error}",
                    destination.display()
                ),
            })?;
        }
        Ok(_) => {
            return Err(Error {
                message: format!(
                    "WineD3D d3d9 override path is not a regular file or symlink: {}",
                    destination.display()
                ),
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Error {
                message: format!(
                    "cannot inspect native d3d9 override {}: {error}",
                    destination.display()
                ),
            });
        }
    }
    Ok(())
}

fn installed_depots_vdf(depots: &[crate::profile::DepotSpec]) -> String {
    let entries = depots
        .iter()
        .filter_map(|depot| {
            depot.manifest_id.map(|manifest_id| {
                format!(
                    concat!(
                        "\t\t\"{}\"\n",
                        "\t\t{{\n",
                        "\t\t\t\"manifest\"\t\t\"{}\"\n",
                        "\t\t}}\n"
                    ),
                    depot.id, manifest_id
                )
            })
        })
        .collect::<String>();
    if entries.is_empty() {
        String::new()
    } else {
        format!("\t\"InstalledDepots\"\n\t{{\n{entries}\t}}\n")
    }
}

fn recover_interrupted_install_tree(
    common: &Path,
    install_dir: &str,
    library_link: &Path,
) -> Result<(), Error> {
    if fs::symlink_metadata(library_link).is_ok() {
        return Ok(());
    }
    let prefix = format!(".{install_dir}.macroton-source-");
    let orphaned = fs::read_dir(common)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && path.is_dir())
        })
        .collect::<Vec<_>>();
    if orphaned.len() == 1 {
        fs::rename(&orphaned[0], library_link).map_err(|error| Error {
            message: format!(
                "cannot recover interrupted Steam install tree {}: {error}",
                orphaned[0].display()
            ),
        })?;
    } else if orphaned.len() > 1 {
        return Err(Error {
            message: format!(
                "multiple interrupted Steam install trees found for {}",
                library_link.display()
            ),
        });
    }
    Ok(())
}

fn materialize_payload_view(roots: &[PathBuf], destination: &Path) -> Result<PathBuf, Error> {
    if destination.is_dir() {
        if !payload_view_has_broken_symlink(destination) {
            return Ok(destination.to_path_buf());
        }
        // The view is Macroton-owned and content-addressed. Rebuild only
        // when a prior source rotation left broken absolute links behind.
        fs::remove_dir_all(destination).map_err(|error| Error {
            message: format!(
                "cannot rebuild broken payload view {}: {error}",
                destination.display()
            ),
        })?;
    }
    if destination.exists() {
        return Err(Error {
            message: format!(
                "payload view exists but is not a directory: {}",
                destination.display()
            ),
        });
    }
    let parent = destination.parent().ok_or_else(|| Error {
        message: format!("payload view has no parent: {}", destination.display()),
    })?;
    fs::create_dir_all(parent).map_err(|error| Error {
        message: format!(
            "cannot create payload-view root {}: {error}",
            parent.display()
        ),
    })?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("payload"),
        std::process::id()
    ));
    if temporary.exists() {
        return Err(Error {
            message: format!("stale payload-view transaction: {}", temporary.display()),
        });
    }
    fs::create_dir(&temporary).map_err(|error| Error {
        message: format!(
            "cannot create payload view {}: {error}",
            temporary.display()
        ),
    })?;
    for root in roots {
        if let Err(error) = merge_payload_directory(root, root, &temporary) {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error);
        }
    }
    fs::rename(&temporary, destination).map_err(|error| Error {
        message: format!(
            "cannot commit payload view {}: {error}",
            destination.display()
        ),
    })?;
    Ok(destination.to_path_buf())
}

fn payload_view_has_broken_symlink(path: &Path) -> bool {
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let child = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&child) else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            fs::metadata(&child).is_err()
        } else if metadata.is_dir() {
            payload_view_has_broken_symlink(&child)
        } else {
            false
        }
    })
}

fn depots_json(depots: &[crate::profile::DepotSpec]) -> String {
    let entries = depots
        .iter()
        .map(|depot| {
            format!(
                "{{\"id\":{},\"manifest_id\":{}}}",
                depot.id,
                depot
                    .manifest_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "null".to_owned())
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{entries}]")
}

fn merge_payload_directory(root: &Path, directory: &Path, view: &Path) -> Result<(), Error> {
    let entries = fs::read_dir(directory).map_err(|error| Error {
        message: format!(
            "cannot read depot directory {}: {error}",
            directory.display()
        ),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| Error {
            message: format!(
                "cannot read depot entry in {}: {error}",
                directory.display()
            ),
        })?;
        let source = entry.path();
        let relative = source.strip_prefix(root).map_err(|error| Error {
            message: format!("cannot contain depot path {}: {error}", source.display()),
        })?;
        let target = view.join(relative);
        let file_type = entry.file_type().map_err(|error| Error {
            message: format!("cannot inspect depot entry {}: {error}", source.display()),
        })?;
        if file_type.is_symlink() {
            return Err(Error {
                message: format!(
                    "refusing symlink inside depot payload: {}",
                    source.display()
                ),
            });
        }
        if file_type.is_dir() {
            if target.exists() && !target.is_dir() {
                return Err(payload_conflict(&source, &target));
            }
            fs::create_dir_all(&target).map_err(|error| Error {
                message: format!(
                    "cannot create merged directory {}: {error}",
                    target.display()
                ),
            })?;
            merge_payload_directory(root, &source, view)?;
        } else if file_type.is_file() {
            if target.exists() {
                if target.is_file() && files_are_identical(&source, &target)? {
                    continue;
                }
                return Err(payload_conflict(&source, &target));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| Error {
                    message: format!(
                        "cannot create merged directory {}: {error}",
                        parent.display()
                    ),
                })?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&source, &target).map_err(|error| Error {
                message: format!(
                    "cannot map depot file {} into {}: {error}",
                    source.display(),
                    target.display()
                ),
            })?;
        }
    }
    Ok(())
}

fn files_are_identical(left: &Path, right: &Path) -> Result<bool, Error> {
    let right_source = fs::canonicalize(right).map_err(|error| Error {
        message: format!(
            "cannot resolve merged payload file {}: {error}",
            right.display()
        ),
    })?;
    let left_metadata = fs::metadata(left).map_err(|error| Error {
        message: format!("cannot inspect payload file {}: {error}", left.display()),
    })?;
    let right_metadata = fs::metadata(&right_source).map_err(|error| Error {
        message: format!(
            "cannot inspect payload file {}: {error}",
            right_source.display()
        ),
    })?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }
    let left_digest = sha256::digest_file(left).map_err(|error| Error {
        message: format!("cannot hash payload file {}: {error}", left.display()),
    })?;
    let right_digest = sha256::digest_file(&right_source).map_err(|error| Error {
        message: format!(
            "cannot hash payload file {}: {error}",
            right_source.display()
        ),
    })?;
    Ok(left_digest == right_digest)
}

fn payload_conflict(source: &Path, target: &Path) -> Error {
    Error {
        message: format!(
            "conflicting multi-depot payload at {} from {}; refusing implicit overwrite",
            target.display(),
            source.display()
        ),
    }
}

fn ensure_payload_link(link: &Path, payload_root: &Path) -> Result<(), Error> {
    // A depot acquired by the isolated Windows Steam client already lives at
    // the exact library path.  In that mode there is nothing to link and the
    // real directory must be reused; only externally staged payloads need a
    // symlink here.
    if link == payload_root {
        return Ok(());
    }
    if let Ok(metadata) = fs::symlink_metadata(link) {
        if !metadata.file_type().is_symlink() {
            return Err(Error {
                message: format!(
                    "refusing to replace existing Steam library entry {}",
                    link.display()
                ),
            });
        }
        let current = fs::read_link(link).map_err(|error| Error {
            message: format!(
                "cannot inspect Steam library link {}: {error}",
                link.display()
            ),
        })?;
        if current == payload_root {
            return Ok(());
        }
        // A prior launcher-bypass transaction may have left the authenticated
        // source tree behind a stable Macroton link.  It is safe to advance
        // that link to the new private view; the source directory remains
        // preserved and is never overwritten.
        let current_is_macroton_view = current
            .components()
            .any(|component| component.as_os_str() == "payload-views")
            || current
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".macroton-source-"));
        if current_is_macroton_view
            && payload_root
                .components()
                .any(|component| component.as_os_str() == "payload-views")
        {
            fs::remove_file(link).map_err(|error| Error {
                message: format!(
                    "cannot advance Macroton Steam library link {}: {error}",
                    link.display()
                ),
            })?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(payload_root, link).map_err(|error| Error {
                message: format!(
                    "cannot map transformed payload {} into Steam library {}: {error}",
                    payload_root.display(),
                    link.display()
                ),
            })?;
            return Ok(());
        }
        return Err(Error {
            message: format!(
                "Steam library link {} already points to {}",
                link.display(),
                current.display()
            ),
        });
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(payload_root, link).map_err(|error| Error {
        message: format!(
            "cannot map Windows payload {} into Steam library {}: {error}",
            payload_root.display(),
            link.display()
        ),
    })?;
    Ok(())
}

fn vdf_escape(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

fn launch_dll_overrides(renderer: &str, configured: Option<&String>) -> String {
    let renderer_overrides = match renderer {
        "dxvk-d3d9-moltenvk" => Some("d3d9=n"),
        "dxmt" => Some("dxgi,d3d11,d3d10core=b"),
        _ => None,
    };
    [
        configured.map(String::as_str),
        renderer_overrides,
        Some(STEAM_OVERLAY_DLL_OVERRIDES),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join(";")
}

fn append_winepath(configured: Option<&String>, required: &str) -> String {
    match configured {
        Some(existing)
            if !existing.is_empty() && existing.split(';').any(|part| part == required) =>
        {
            existing.clone()
        }
        Some(existing) if !existing.is_empty() => format!("{required};{existing}"),
        _ => required.to_owned(),
    }
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn json_escape(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn explicit_stop_cancels_only_the_in_flight_launch_generation() {
        let temp = TempDir::new();
        let game_root = temp.0.join("games/297000");
        let original = stop_marker_snapshot(&game_root).unwrap();
        ensure_launch_not_stopped(&game_root, &original).unwrap();

        record_stop_request(297000, &temp.0).unwrap();
        assert!(ensure_launch_not_stopped(&game_root, &original)
            .unwrap_err()
            .to_string()
            .contains("cancelled"));

        let next_launch = stop_marker_snapshot(&game_root).unwrap();
        ensure_launch_not_stopped(&game_root, &next_launch).unwrap();
        record_stop_request(297000, &temp.0).unwrap();
        assert_ne!(next_launch, stop_marker_snapshot(&game_root).unwrap());
        assert!(ensure_launch_not_stopped(&game_root, &next_launch).is_err());
    }

    #[test]
    fn explicit_stop_interrupts_a_protocol_retry_wait() {
        let temp = TempDir::new();
        let root = temp.0.clone();
        let game_root = root.join("games/297000");
        let original = stop_marker_snapshot(&game_root).unwrap();
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            record_stop_request(297000, &root).unwrap();
        });
        let started = Instant::now();
        let result = wait_for_retry_or_stop(Duration::from_secs(3), &game_root, &original);
        writer.join().unwrap();
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn explicit_stop_interrupts_steam_client_warmup() {
        let temp = TempDir::new();
        let root = temp.0.clone();
        let game_root = root.join("games/297000");
        let original = stop_marker_snapshot(&game_root).unwrap();
        let request = SteamClientRequest {
            pid: 0,
            client: root.join("Steam.exe"),
            runtime_id: "test".to_owned(),
            profile_sha256: "test".to_owned(),
            client_mode: SteamClientMode::Standard,
            log: root.join("missing.log"),
            ui_log: root.join("missing-ui.log"),
            ui_log_offset: 0,
            console_log: root.join("missing-console.log"),
            console_log_offset: 0,
        };
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            record_stop_request(297000, &root).unwrap();
        });
        let started = Instant::now();
        let result = wait_for_steam_client_ready_or_stop(
            &request,
            Duration::from_secs(3),
            &game_root,
            &original,
        );
        writer.join().unwrap();
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn steam_client_ui_ready_only_accepts_markers_after_recorded_offset() {
        let temp = TempDir::new();
        let log = temp.0.join("webhelper.txt");
        fs::write(&log, "old SteamBrowser- stale\n").unwrap();
        let offset = fs::metadata(&log).unwrap().len();
        assert!(!steam_client_ui_ready(&log, offset));
        fs::write(&log, "old SteamBrowser- stale\nFriends List created\n").unwrap();
        assert!(steam_client_ui_ready(&log, offset));
    }

    #[test]
    fn steam_client_ui_ready_handles_rotated_log() {
        let temp = TempDir::new();
        let log = temp.0.join("webhelper.txt");
        fs::write(&log, "SteamBrowser- ready\n").unwrap();
        assert!(steam_client_ui_ready(&log, 999_999));
    }

    #[test]
    fn steam_client_console_ready_accepts_startup_marker_after_offset() {
        let temp = TempDir::new();
        let log = temp.0.join("console_log.txt");
        fs::write(&log, "old System startup time: 9.0 seconds\n").unwrap();
        let offset = fs::metadata(&log).unwrap().len();
        assert!(!steam_client_console_ready(&log, offset));
        fs::write(
            &log,
            "old System startup time: 9.0 seconds\nSystem startup time: 21.5 seconds\n",
        )
        .unwrap();
        assert!(steam_client_console_ready(&log, offset));
    }

    #[test]
    fn runtime_dyld_library_path_includes_verified_vulkan_runtime_dirs() {
        let root = std::env::temp_dir().join(format!(
            "macroton-dyld-path-{}-{}",
            std::process::id(),
            unix_ms()
        ));
        let entrypoint = root.join("bin/wine");
        fs::create_dir_all(root.join("lib/wine/x86_64-unix")).unwrap();
        fs::create_dir_all(entrypoint.parent().unwrap()).unwrap();
        let value = runtime_dyld_library_path(&entrypoint).unwrap();
        assert!(value.starts_with(&root.join("lib").display().to_string()));
        assert!(value.contains("lib/wine/x86_64-unix"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn steam_handoff_retry_delay_is_bounded_and_overrideable() {
        assert_eq!(parse_steam_game_warmup_seconds(Some("0")), 90);
        assert_eq!(parse_steam_game_warmup_seconds(Some("999")), 90);
        assert_eq!(parse_steam_handoff_retry_seconds(Some("1")), 1);
        assert_eq!(parse_steam_handoff_retry_seconds(Some("31")), 8);
        assert_eq!(parse_steam_handoff_retry_seconds(None), 8);
    }

    #[test]
    fn runtime_dyld_library_path_is_none_for_missing_runtime_dirs() {
        let entrypoint =
            std::env::temp_dir().join(format!("macroton-missing-runtime-{}/bin/wine", unix_ms()));
        assert_eq!(runtime_dyld_library_path(&entrypoint), None);
    }

    #[test]
    fn authenticated_steam_manifest_is_detected_as_installed() {
        assert!(manifest_size_on_disk_positive(
            "\"SizeOnDisk\"\t\t\"1889540215\"\n"
        ));
        assert!(!manifest_size_on_disk_positive("\"SizeOnDisk\"\t\t\"0\"\n"));
        assert!(!manifest_size_on_disk_positive("\"appid\"\t\t\"606680\"\n"));
    }

    #[test]
    fn silver_savegame_alias_is_created_and_idempotent() {
        let temp = TempDir::new();
        let save_root = temp
            .0
            .join("prefix/drive_c/users/player/AppData/Roaming/THQNordic/Silver");
        fs::create_dir_all(&save_root).unwrap();
        fs::write(save_root.join("00000000.sav"), b"save").unwrap();

        ensure_game_compatibility_aliases(606680, &temp.0.join("prefix")).unwrap();
        let alias = save_root.join("SAVEGAME");
        assert!(
            fs::symlink_metadata(&alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&alias).unwrap(), Path::new("."));
        assert_eq!(fs::read(alias.join("00000000.sav")).unwrap(), b"save");

        ensure_game_compatibility_aliases(606680, &temp.0.join("prefix")).unwrap();
        assert_eq!(fs::read_link(alias).unwrap(), Path::new("."));
    }

    #[test]
    fn startup_config_replaces_macroton_directives_without_touching_user_lines() {
        let temp = TempDir::new();
        let startup = temp
            .0
            .join("prefix/drive_c/users/player/My Documents/My Games/Age of Empires 3/Startup");
        fs::create_dir_all(&startup).unwrap();
        fs::write(
            startup.join("user.cfg"),
            "customsetting=true\nNoStartUp\nwindow\nDontCheckUnsupported\n",
        )
        .unwrap();

        ensure_profile_startup_config(
            &temp.0.join("prefix"),
            Some(&"nointrocinematics,dontcheckunsupported".to_owned()),
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(startup.join("user.cfg")).unwrap(),
            "customsetting=true\nnointrocinematics\ndontcheckunsupported\n"
        );
    }

    #[test]
    fn aoe3_window_profile_is_seeded_for_each_wine_user() {
        let temp = TempDir::new();
        let prefix = temp.0.join("prefix");
        let user_root = prefix.join("drive_c/users/player");
        fs::create_dir_all(&user_root).unwrap();
        ensure_aoe3_graphics_profile(&prefix, Some(&"window".to_owned())).unwrap();
        let profile = user_root.join("My Documents/My Games/Age of Empires 3/Users/NewProfile.xml");
        let contents = fs::read_to_string(profile).unwrap();
        assert!(contents.contains("<Setting Name=\"optiongrfxwindowmode\">true</Setting>"));
    }

    #[test]
    fn non_silver_profiles_do_not_create_game_specific_save_aliases() {
        let temp = TempDir::new();
        ensure_game_compatibility_aliases(26800, &temp.0.join("prefix")).unwrap();
        assert!(!temp.0.join("prefix/drive_c/users").exists());
    }

    #[test]
    fn silver_savegame_alias_rejects_a_real_file() {
        let temp = TempDir::new();
        let save_root = temp
            .0
            .join("prefix/drive_c/users/player/AppData/Roaming/THQNordic/Silver");
        fs::create_dir_all(&save_root).unwrap();
        fs::write(save_root.join("SAVEGAME"), b"not a directory").unwrap();

        let error = ensure_game_compatibility_aliases(606680, &temp.0.join("prefix"))
            .expect_err("a real SAVEGAME file must not be silently reused");
        assert!(error.message.contains("not a directory"));
    }

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "macroton-execute-test-{}-{sequence}",
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
    fn steam_cef_repair_is_opt_in_and_tracks_valve_updates() {
        let temp = TempDir::new();
        let prefix = temp.0.join("games/42/prefix");
        let cef = prefix.join("drive_c/Program Files (x86)/Steam/bin/cef/cef.win64");
        fs::create_dir_all(&cef).unwrap();
        let helper = cef.join("steamwebhelper.exe");
        let real = cef.join("steamwebhelper_real.exe");
        fs::write(&helper, b"Valve v1").unwrap();
        repair_opted_in_steam_cef_wrapper(42, &temp.0, &prefix).unwrap();
        assert_eq!(fs::read(&helper).unwrap(), b"Valve v1");

        fs::write(&real, b"Valve v1").unwrap();
        fs::write(&helper, STEAM_CEF_WRAPPER).unwrap();
        repair_opted_in_steam_cef_wrapper(42, &temp.0, &prefix).unwrap();
        assert_eq!(fs::read(&real).unwrap(), b"Valve v1");

        fs::write(&helper, b"Valve v2").unwrap();
        repair_opted_in_steam_cef_wrapper(42, &temp.0, &prefix).unwrap();
        assert_eq!(fs::read(&real).unwrap(), b"Valve v2");
        assert_eq!(fs::read(&helper).unwrap(), STEAM_CEF_WRAPPER);
        assert_eq!(fs::read_dir(&cef).unwrap().count(), 3);
    }

    #[test]
    fn executes_without_a_shell_and_records_the_result() {
        let temp = TempDir::new();
        let runtime = temp.0.join("fake-wine");
        let game = temp.0.join("game");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&game).unwrap();
        let entrypoint = runtime.join("wine");
        fs::write(
            &entrypoint,
            "#!/bin/sh\n[ \"$1\" = reg.exe ] && exit 0\nexit 7\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&entrypoint).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&entrypoint, permissions).unwrap();
        let executable = game.join("game.exe");
        fs::write(&executable, b"MZ").unwrap();
        let plan = LaunchPlan {
            app_id: 42,
            game_name: "Test".to_owned(),
            steam_build_id: Some("1234".to_owned()),
            windows_depot_id: Some(43),
            windows_depots: vec![crate::profile::DepotSpec {
                id: 43,
                manifest_id: None,
            }],
            payload_root: game.clone(),
            payload_roots: vec![game.clone()],
            executable,
            profile_sha256: "cd".repeat(32),
            runtime_id: "fake-wine".to_owned(),
            runtime_kind: Some(RuntimeKind::ExternalExecutable),
            runtime_entrypoint: Some(entrypoint.clone()),
            runtime_fingerprint_sha256: Some("ab".repeat(32)),
            runtime_components: Vec::new(),
            renderer: "auto".to_owned(),
            graphics_api: crate::profile::GraphicsApi::Unknown,
            graphics_evidence: Vec::new(),
            launch_mode: LaunchMode::Direct,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: None,
            steam_launcher_bypass: None,
            windows_build_id: None,
            windows_depot_manifest_id: None,
            arguments: vec!["argument with spaces".to_owned()],
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };

        let result = execute(&plan, &temp.0.join("data")).unwrap();
        assert_eq!(result.exit_code, Some(7));
        assert!(result.result_path.is_file());
        assert!(temp.0.join("data/games/42/prefix").is_dir());
        let record = fs::read_to_string(result.result_path).unwrap();
        assert!(record.contains("\"exit_code\": 7"));
        assert!(record.contains(&"ab".repeat(32)));
        assert!(record.contains(&"cd".repeat(32)));
        assert!(record.contains("\"steam_build_id\": \"1234\""));
        assert!(record.contains("\"renderer\": \"auto\""));
        assert!(record.contains("\"graphics_api\": \"unknown\""));
        assert!(record.contains("\"graphics_evidence\": []"));
        let identity = fs::read_to_string(
            temp.0
                .join("data/games/42/prefix")
                .join(PREFIX_RUNTIME_IDENTITY),
        )
        .unwrap();
        assert!(identity.contains("runtime_id=fake-wine"));
        assert!(identity.contains(&format!("runtime_fingerprint_sha256={}", "ab".repeat(32))));
    }

    #[test]
    fn direct_launch_maps_host_depot_executable_into_prefix_library() {
        let temp = TempDir::new();
        let payload = temp.0.join("host-steam-depot");
        let prefix = temp.0.join("games/42/prefix");
        let executable = payload.join("silver.exe");
        let staged =
            prefix.join("drive_c/Program Files (x86)/Steam/steamapps/common/Silver/silver.exe");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::create_dir_all(staged.parent().unwrap()).unwrap();
        fs::write(&executable, b"host").unwrap();
        fs::write(&staged, b"prefix").unwrap();
        let plan = LaunchPlan {
            app_id: 42,
            game_name: "Silver".to_owned(),
            steam_build_id: None,
            windows_depot_id: None,
            windows_depots: Vec::new(),
            payload_root: payload.clone(),
            payload_roots: vec![payload],
            executable,
            profile_sha256: "00".repeat(32),
            runtime_id: "wine".to_owned(),
            runtime_kind: None,
            runtime_entrypoint: None,
            runtime_fingerprint_sha256: None,
            runtime_components: Vec::new(),
            renderer: "auto".to_owned(),
            graphics_api: GraphicsApi::Unknown,
            graphics_evidence: Vec::new(),
            launch_mode: LaunchMode::Direct,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: Some("Silver".to_owned()),
            steam_launcher_bypass: None,
            windows_build_id: None,
            windows_depot_manifest_id: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };
        assert_eq!(resolve_prefix_game_executable(&plan, &prefix), staged);
    }

    #[test]
    fn refuses_runtime_switch_while_prefix_wineserver_is_active() {
        let temp = TempDir::new();
        let runtime = temp.0.join("runtime/bin");
        let prefix = temp.0.join("prefix");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&prefix).unwrap();
        let wine = runtime.join("wine");
        let wineserver = runtime.join("wineserver");
        fs::write(&wine, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(&wineserver, "#!/bin/sh\nsleep 10\n").unwrap();
        for executable in [&wine, &wineserver] {
            let mut permissions = fs::metadata(executable).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(executable, permissions).unwrap();
        }
        fs::write(
            prefix.join(PREFIX_RUNTIME_IDENTITY),
            "schema=1\nruntime_id=old\nruntime_fingerprint_sha256=old-hash\ncomponents=\n",
        )
        .unwrap();

        let error = ensure_prefix_runtime_identity(&prefix, &wine, "new", &"ab".repeat(32), &[])
            .unwrap_err();
        assert!(error.message.contains("active wineserver"));
        assert!(error.message.contains("owned by runtime old"));
        assert!(error.message.contains("switching to new"));
    }

    #[test]
    fn permits_runtime_switch_when_prefix_is_idle() {
        let temp = TempDir::new();
        let runtime = temp.0.join("runtime/bin");
        let prefix = temp.0.join("prefix");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir_all(&prefix).unwrap();
        let wine = runtime.join("wine");
        let wineserver = runtime.join("wineserver");
        fs::write(&wine, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(&wineserver, "#!/bin/sh\nexit 0\n").unwrap();
        for executable in [&wine, &wineserver] {
            let mut permissions = fs::metadata(executable).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(executable, permissions).unwrap();
        }

        ensure_prefix_runtime_identity(
            &prefix,
            &wine,
            "new",
            &"ab".repeat(32),
            &["component-1".to_owned()],
        )
        .unwrap();
        let identity = fs::read_to_string(prefix.join(PREFIX_RUNTIME_IDENTITY)).unwrap();
        assert!(identity.contains("runtime_id=new"));
        assert!(identity.contains("components=component-1"));
    }

    #[test]
    fn steam_launch_submits_without_waiting_for_the_long_lived_client() {
        let temp = TempDir::new();
        let entrypoint = temp.0.join("fake-wine");
        fs::write(
            &entrypoint,
            "#!/bin/sh\ncd /\n[ \"$1\" = reg.exe ] && exit 0\n: > \"$0.done\"\nexit 0\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&entrypoint).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&entrypoint, permissions).unwrap();

        let data_root = temp.0.join("data");
        let payload = temp.0.join("payload");
        let steam_root = data_root.join("games/42/prefix/drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&payload).unwrap();
        fs::create_dir_all(&steam_root).unwrap();
        fs::write(payload.join("game.exe"), b"MZ").unwrap();
        fs::write(steam_root.join("Steam.exe"), b"MZ").unwrap();
        let prefix = data_root.join("games/42/prefix");
        fs::write(
            prefix.join(".macroton-steam-bootstrap.json"),
            format!(
                "{{\n  \"schema\": 1,\n  \"appid\": 42,\n  \"runtime_id\": \"fake-wine\",\n  \"runtime_kind\": \"external-executable\",\n  \"runtime_fingerprint_sha256\": \"{}\",\n  \"installer_sha256\": \"{}\"\n}}\n",
                "ab".repeat(32),
                "ef".repeat(32)
            ),
        )
        .unwrap();
        fs::write(
            prefix.join(PREFIX_RUNTIME_IDENTITY),
            format!(
                "schema=1\nruntime_id=fake-wine\nruntime_fingerprint_sha256={}\ncomponents=\n",
                "ab".repeat(32)
            ),
        )
        .unwrap();
        let plan = LaunchPlan {
            app_id: 42,
            game_name: "Test".to_owned(),
            steam_build_id: None,
            windows_depot_id: Some(43),
            windows_depots: vec![crate::profile::DepotSpec {
                id: 43,
                manifest_id: None,
            }],
            payload_root: payload.clone(),
            payload_roots: vec![payload.clone()],
            executable: payload.join("game.exe"),
            profile_sha256: "cd".repeat(32),
            runtime_id: "fake-wine".to_owned(),
            runtime_kind: Some(RuntimeKind::ExternalExecutable),
            runtime_entrypoint: Some(entrypoint.clone()),
            runtime_fingerprint_sha256: Some("ab".repeat(32)),
            runtime_components: Vec::new(),
            renderer: "auto".to_owned(),
            graphics_api: crate::profile::GraphicsApi::Unknown,
            graphics_evidence: Vec::new(),
            launch_mode: LaunchMode::Steam,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: Some("Test".to_owned()),
            steam_launcher_bypass: None,
            windows_build_id: None,
            windows_depot_manifest_id: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };

        let result = execute_with_client_warmup(&plan, &data_root, Duration::ZERO).unwrap();
        assert_eq!(result.exit_code, None);
        let record = fs::read_to_string(result.result_path).unwrap();
        assert!(record.contains("\"launch_mode\": \"steam\""));
        assert!(record.contains("\"exit_code\": null"));
        let session = steam_session::read(&prefix).unwrap();
        assert_eq!(session.app_id, 42);
        assert_eq!(session.profile_sha256, "cd".repeat(32));
        assert_eq!(session.runtime_fingerprint_sha256, "ab".repeat(32));
        assert_eq!(session.steam_client_mode, "standard");
        let completed = entrypoint.with_file_name("fake-wine.done");
        for _ in 0..100 {
            if completed.is_file() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(completed.is_file(), "spawned fake runtime did not complete");
    }

    #[test]
    fn steam_launch_request_preserves_arguments_when_client_is_live() {
        assert_eq!(
            steam_launch_request(297000, true, false),
            vec!["-applaunch", "297000"]
        );
        assert_eq!(
            steam_launch_request(297000, true, true),
            vec!["steam://rungameid/297000"]
        );
        assert_eq!(
            steam_launch_request(297000, false, false),
            vec!["-applaunch", "297000"]
        );
    }

    #[test]
    fn steam_game_handoff_arguments_keep_launcher_switches() {
        let arguments = vec![
            "--version".to_owned(),
            "v1.18".to_owned(),
            "--forceANGLE".to_owned(),
        ];
        assert_eq!(
            steam_game_handoff_arguments(297000, &arguments),
            vec!["-applaunch", "297000", "--version", "v1.18", "--forceANGLE"]
        );
    }

    #[test]
    fn crash_guard_disables_post_mortem_debugger_in_both_registry_views() {
        let temp = TempDir::new();
        let prefix = temp.0.join("prefix");
        let runtime = temp.0.join("fake-wine");
        fs::create_dir_all(&prefix).unwrap();
        fs::write(
            &runtime,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$0.args\"\nexit 0\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&runtime).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&runtime, permissions).unwrap();

        let fingerprint = "ab".repeat(32);
        ensure_crash_debugger_guard(&prefix, &runtime, &fingerprint).unwrap();
        let calls = fs::read_to_string(runtime.with_extension("args")).unwrap();
        assert_eq!(calls.lines().count(), 4);
        assert_eq!(calls.matches("/reg:32").count(), 2);
        assert_eq!(calls.matches("/reg:64").count(), 2);
        assert_eq!(calls.matches(DISABLED_DEBUGGER_COMMAND).count(), 2);
        assert!(calls.contains("/v Auto /t REG_SZ /d 1 /f"));

        // An unchanged runtime fingerprint must not add launch latency by
        // rewriting the same registry values on every game start.
        ensure_crash_debugger_guard(&prefix, &runtime, &fingerprint).unwrap();
        let unchanged = fs::read_to_string(runtime.with_extension("args")).unwrap();
        assert_eq!(unchanged, calls);
        let marker = fs::read_to_string(prefix.join(PREFIX_CRASH_GUARD)).unwrap();
        assert!(marker.contains(&fingerprint));
        assert!(marker.contains("macroton-disabled-debugger.exe"));
    }

    #[test]
    fn crash_guard_fails_closed_when_registry_write_fails() {
        let temp = TempDir::new();
        let prefix = temp.0.join("prefix");
        let runtime = temp.0.join("fake-wine");
        fs::create_dir_all(&prefix).unwrap();
        fs::write(&runtime, "#!/bin/sh\nexit 9\n").unwrap();
        let mut permissions = fs::metadata(&runtime).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&runtime, permissions).unwrap();

        let error = ensure_crash_debugger_guard(&prefix, &runtime, &"cd".repeat(32)).unwrap_err();
        assert!(
            error
                .message
                .contains("32-bit Wine crash guard value Debugger")
        );
        assert!(!prefix.join(PREFIX_CRASH_GUARD).exists());
    }

    #[test]
    fn prepares_an_isolated_steam_library_without_copying_the_payload() {
        let temp = TempDir::new();
        let payload = temp.0.join("payload");
        let prefix = temp.0.join("prefix");
        let steam_root = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&payload).unwrap();
        fs::create_dir_all(&steam_root).unwrap();
        fs::write(payload.join("game.exe"), b"MZ").unwrap();
        fs::write(steam_root.join("Steam.exe"), b"MZ").unwrap();
        let plan = LaunchPlan {
            app_id: 42,
            game_name: "Quoted \"Game\"".to_owned(),
            steam_build_id: None,
            windows_depot_id: Some(43),
            windows_depots: vec![crate::profile::DepotSpec {
                id: 43,
                manifest_id: Some(5678),
            }],
            payload_root: payload.clone(),
            payload_roots: vec![payload.clone()],
            executable: payload.join("game.exe"),
            profile_sha256: "cd".repeat(32),
            runtime_id: "fake-wine".to_owned(),
            runtime_kind: Some(RuntimeKind::ExternalExecutable),
            runtime_entrypoint: Some(temp.0.join("wine")),
            runtime_fingerprint_sha256: Some("ab".repeat(32)),
            runtime_components: Vec::new(),
            renderer: "auto".to_owned(),
            graphics_api: crate::profile::GraphicsApi::Unknown,
            graphics_evidence: Vec::new(),
            launch_mode: LaunchMode::Steam,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: Some("Example Game".to_owned()),
            steam_launcher_bypass: None,
            windows_build_id: Some("1234".to_owned()),
            windows_depot_manifest_id: Some(5678),
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };

        let prepared = prepare_steam_launch(&plan, &prefix).unwrap();
        assert_eq!(fs::read_link(&prepared.library_link).unwrap(), payload);
        let manifest = fs::read_to_string(prepared.appmanifest).unwrap();
        assert!(manifest.contains("\"appid\"\t\t\"42\""));
        assert!(manifest.contains("\"manifest\"\t\t\"5678\""));
        assert!(manifest.contains("Quoted \\\"Game\\\""));
    }

    #[test]
    fn steam_launcher_bypass_replaces_only_the_private_payload_view() {
        let temp = TempDir::new();
        let payload = temp.0.join("payload");
        let prefix = temp.0.join("game/prefix");
        let steam_root = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&payload).unwrap();
        fs::create_dir_all(&steam_root).unwrap();
        fs::write(payload.join("Launcher.exe"), b"MZ-retired-launcher").unwrap();
        fs::write(payload.join("game.exe"), b"MZ-real-game").unwrap();
        fs::write(payload.join("content.bin"), b"content").unwrap();
        fs::write(steam_root.join("Steam.exe"), b"MZ").unwrap();
        let plan = LaunchPlan {
            app_id: 42,
            game_name: "Launcher Bypass".to_owned(),
            steam_build_id: Some("1".to_owned()),
            windows_depot_id: Some(43),
            windows_depots: vec![crate::profile::DepotSpec {
                id: 43,
                manifest_id: Some(100),
            }],
            payload_root: payload.clone(),
            payload_roots: vec![payload.clone()],
            executable: payload.join("game.exe"),
            profile_sha256: "12".repeat(32),
            runtime_id: "fake".to_owned(),
            runtime_kind: Some(RuntimeKind::ExternalExecutable),
            runtime_entrypoint: Some(temp.0.join("wine")),
            runtime_fingerprint_sha256: Some("ab".repeat(32)),
            runtime_components: Vec::new(),
            renderer: "auto".to_owned(),
            graphics_api: crate::profile::GraphicsApi::Unknown,
            graphics_evidence: Vec::new(),
            launch_mode: LaunchMode::Steam,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: Some("Bypassed".to_owned()),
            steam_launcher_bypass: Some(PathBuf::from("Launcher.exe")),
            windows_build_id: Some("1".to_owned()),
            windows_depot_manifest_id: Some(100),
            arguments: vec!["NoStartUp".to_owned()],
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };

        let prepared = prepare_steam_launch(&plan, &prefix).unwrap();
        let view = fs::read_link(&prepared.library_link).unwrap();
        assert_eq!(
            fs::read(view.join("Launcher.exe")).unwrap(),
            b"MZ-real-game"
        );
        assert!(
            !fs::symlink_metadata(view.join("Launcher.exe"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(view.join("content.bin"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read(payload.join("Launcher.exe")).unwrap(),
            b"MZ-retired-launcher"
        );

        // Re-preparing the same fingerprint is idempotent.
        prepare_steam_launch(&plan, &prefix).unwrap();
        assert_eq!(
            fs::read(view.join("Launcher.exe")).unwrap(),
            b"MZ-real-game"
        );
    }

    #[test]
    fn refuses_to_replace_an_existing_steam_library_directory() {
        let temp = TempDir::new();
        let payload = temp.0.join("payload");
        let prefix = temp.0.join("prefix");
        let steam_root = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(&payload).unwrap();
        fs::create_dir_all(steam_root.join("steamapps/common/Example")).unwrap();
        fs::write(steam_root.join("Steam.exe"), b"MZ").unwrap();
        let mut plan = LaunchPlan {
            app_id: 42,
            game_name: "Example".to_owned(),
            steam_build_id: None,
            windows_depot_id: Some(43),
            windows_depots: vec![crate::profile::DepotSpec {
                id: 43,
                manifest_id: None,
            }],
            payload_root: payload.clone(),
            payload_roots: vec![payload.clone()],
            executable: payload.join("game.exe"),
            profile_sha256: "cd".repeat(32),
            runtime_id: "fake-wine".to_owned(),
            runtime_kind: Some(RuntimeKind::ExternalExecutable),
            runtime_entrypoint: Some(temp.0.join("wine")),
            runtime_fingerprint_sha256: Some("ab".repeat(32)),
            runtime_components: Vec::new(),
            renderer: "auto".to_owned(),
            graphics_api: crate::profile::GraphicsApi::Unknown,
            graphics_evidence: Vec::new(),
            launch_mode: LaunchMode::Steam,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: Some("Example".to_owned()),
            steam_launcher_bypass: None,
            windows_build_id: None,
            windows_depot_manifest_id: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };
        plan.executable = payload.join("game.exe");
        let error = prepare_steam_launch(&plan, &prefix).unwrap_err();
        assert!(error.message.contains("refusing to replace"));
    }

    #[test]
    fn steam_launch_always_disables_optional_overlay_injection() {
        assert_eq!(
            launch_dll_overrides("auto", None),
            "gameoverlayrenderer=d;gameoverlayrenderer64=d"
        );
        assert_eq!(
            launch_dll_overrides("auto", Some(&"xinput1_3=n,b".to_owned())),
            "xinput1_3=n,b;gameoverlayrenderer=d;gameoverlayrenderer64=d"
        );
        assert_eq!(
            launch_dll_overrides("auto", Some(&"pidgen=n;pidgenx=n".to_owned())),
            "pidgen=n;pidgenx=n;gameoverlayrenderer=d;gameoverlayrenderer64=d"
        );
        assert_eq!(
            launch_dll_overrides("dxvk-d3d9-moltenvk", None),
            "d3d9=n;gameoverlayrenderer=d;gameoverlayrenderer64=d"
        );
        assert_eq!(
            launch_dll_overrides("dxmt", None),
            "dxgi,d3d11,d3d10core=b;gameoverlayrenderer=d;gameoverlayrenderer64=d"
        );
    }

    #[test]
    fn steam_helper_winepath_exposes_game_root_without_duplicates() {
        let required = r"C:\Program Files (x86)\Steam\steamapps\common\Braid";
        assert_eq!(append_winepath(None, required), required);
        assert_eq!(
            append_winepath(Some(&"C:\\custom".to_owned()), required),
            format!("{required};C:\\custom")
        );
        assert_eq!(
            append_winepath(Some(&format!("{required};C:\\custom")), required),
            format!("{required};C:\\custom")
        );
    }

    #[test]
    fn steam_game_warmup_accepts_bounded_override_and_rejects_invalid_values() {
        assert_eq!(parse_steam_game_warmup_seconds(None), 90);
        assert_eq!(parse_steam_game_warmup_seconds(Some("15")), 15);
        assert_eq!(parse_steam_game_warmup_seconds(Some("1")), 1);
        assert_eq!(parse_steam_game_warmup_seconds(Some("0")), 90);
        assert_eq!(parse_steam_game_warmup_seconds(Some("121")), 90);
        assert_eq!(parse_steam_game_warmup_seconds(Some("slow")), 90);
    }

    #[test]
    fn steam_launch_uses_only_verified_low_overhead_client_switches() {
        assert_eq!(STEAM_RENDERED_LOGIN_ARGUMENTS, ["-tcp", "-nofriendsui"]);
        assert_eq!(
            STEAM_LOW_MEMORY_ARGUMENTS,
            [
                "-tcp",
                "-nofriendsui",
                "-nochatui",
                "-cef-disable-gpu",
                "-cef-force-opaque-backgrounds"
            ]
        );
        assert_eq!(
            STEAM_OPAQUE_SOFTWARE_ARGUMENTS,
            [
                "-tcp",
                "-nofriendsui",
                "-cef-enable-debugging",
                "-cef-disable-gpu",
                "-cef-force-opaque-backgrounds"
            ]
        );
        assert_eq!(
            STEAM_STANDARD_ARGUMENTS,
            ["-tcp", "-nocrashdialog", "-nofriendsui"]
        );
        assert_eq!(
            STEAM_CEF_DEBUG_ARGUMENTS,
            ["-tcp", "-cef-enable-debugging", "-cef-disable-gpu"]
        );
        assert!(!STEAM_LOW_MEMORY_ARGUMENTS.contains(&"-no-browser"));
        assert!(!STEAM_LOW_MEMORY_ARGUMENTS.contains(&"-cef-single-process"));
        assert_eq!(STEAM_LOGIN_STANDARD_ARGUMENTS, ["-tcp"]);
        assert_eq!(steam_install_uri(400), "steam://install/400");
        assert_eq!(
            STEAM_LOGIN_RENDERED_ARGUMENTS,
            [
                "-tcp",
                "-nofriendsui",
                "-cef-enable-debugging",
                "-cef-force-opaque-backgrounds"
            ]
        );
        assert_eq!(
            STEAM_LOGIN_OPAQUE_SOFTWARE_ARGUMENTS,
            [
                "-tcp",
                "-nofriendsui",
                "-cef-enable-debugging",
                "-cef-disable-gpu",
                "-cef-force-opaque-backgrounds"
            ]
        );
        assert_eq!(
            STEAM_LOGIN_LOW_MEMORY_ARGUMENTS,
            [
                "-tcp",
                "-nofriendsui",
                "-cef-enable-debugging",
                "-cef-disable-gpu",
                "-cef-force-opaque-backgrounds"
            ]
        );
        assert_eq!(
            STEAM_LOGIN_CEF_DEBUG_ARGUMENTS,
            ["-tcp", "-cef-enable-debugging", "-cef-disable-gpu"]
        );

        let mut standard = Command::new("wine");
        append_steam_client_arguments(&mut standard, SteamClientMode::Standard);
        assert_eq!(
            standard.get_args().collect::<Vec<_>>(),
            ["-tcp", "-nocrashdialog", "-nofriendsui"]
        );
        let mut low_memory = Command::new("wine");
        append_steam_client_arguments(&mut low_memory, SteamClientMode::LowMemoryExperimental);
        assert_eq!(
            low_memory.get_args().collect::<Vec<_>>(),
            [
                "-tcp",
                "-nofriendsui",
                "-nochatui",
                "-cef-disable-gpu",
                "-cef-force-opaque-backgrounds"
            ]
        );
        let mut rendered = Command::new("wine");
        append_steam_client_arguments(&mut rendered, SteamClientMode::RenderedLoginExperimental);
        assert_eq!(
            rendered.get_args().collect::<Vec<_>>(),
            ["-tcp", "-nofriendsui"]
        );
        let mut opaque = Command::new("wine");
        append_steam_client_arguments(&mut opaque, SteamClientMode::OpaqueSoftwareExperimental);
        assert_eq!(
            opaque.get_args().collect::<Vec<_>>(),
            [
                "-tcp",
                "-nofriendsui",
                "-cef-enable-debugging",
                "-cef-disable-gpu",
                "-cef-force-opaque-backgrounds"
            ]
        );
        let mut cef_debug = Command::new("wine");
        append_steam_client_arguments(&mut cef_debug, SteamClientMode::CefDebugExperimental);
        assert_eq!(
            cef_debug.get_args().collect::<Vec<_>>(),
            ["-tcp", "-cef-enable-debugging", "-cef-disable-gpu"]
        );
        let mut login = Command::new("wine");
        append_steam_login_arguments(&mut login, SteamClientMode::Standard);
        assert_eq!(login.get_args().collect::<Vec<_>>(), ["-tcp"]);
        let mut rendered_login = Command::new("wine");
        append_steam_login_arguments(
            &mut rendered_login,
            SteamClientMode::RenderedLoginExperimental,
        );
        assert_eq!(
            rendered_login.get_args().collect::<Vec<_>>(),
            [
                "-tcp",
                "-nofriendsui",
                "-cef-enable-debugging",
                "-cef-force-opaque-backgrounds"
            ]
        );
        let mut opaque_login = Command::new("wine");
        append_steam_login_arguments(
            &mut opaque_login,
            SteamClientMode::OpaqueSoftwareExperimental,
        );
        assert_eq!(
            opaque_login.get_args().collect::<Vec<_>>(),
            [
                "-tcp",
                "-nofriendsui",
                "-cef-enable-debugging",
                "-cef-disable-gpu",
                "-cef-force-opaque-backgrounds"
            ]
        );
        let mut cef_debug_login = Command::new("wine");
        append_steam_login_arguments(&mut cef_debug_login, SteamClientMode::CefDebugExperimental);
        assert_eq!(
            cef_debug_login.get_args().collect::<Vec<_>>(),
            ["-tcp", "-cef-enable-debugging", "-cef-disable-gpu"]
        );

        let mut configured_login = Command::new("wine");
        configure_steam_login_command(
            &mut configured_login,
            Path::new("Steam.exe"),
            Path::new("Steam"),
            Path::new("prefix"),
            SteamClientMode::Standard,
        );
        assert_eq!(
            configured_login.get_args().collect::<Vec<_>>(),
            ["Steam.exe", "-tcp"]
        );
        assert_eq!(
            configured_login
                .get_envs()
                .map(|(key, value)| (key, value.unwrap()))
                .collect::<Vec<_>>(),
            [
                (
                    std::ffi::OsStr::new("WINEDEBUG"),
                    std::ffi::OsStr::new("-all")
                ),
                (
                    std::ffi::OsStr::new("WINEPREFIX"),
                    std::ffi::OsStr::new("prefix")
                )
            ]
        );

        let mut configured_action = Command::new("wine");
        configure_steam_action_command(
            &mut configured_action,
            Path::new("Steam.exe"),
            Path::new("Steam"),
            Path::new("prefix"),
            SteamClientMode::Standard,
        );
        assert_eq!(
            configured_action.get_args().collect::<Vec<_>>(),
            ["Steam.exe", "-tcp", "-nocrashdialog", "-nofriendsui"]
        );
        assert_eq!(
            configured_action
                .get_envs()
                .map(|(key, value)| (key, value.unwrap()))
                .collect::<Vec<_>>(),
            [
                (
                    std::ffi::OsStr::new("WINEDEBUG"),
                    std::ffi::OsStr::new("-all")
                ),
                (
                    std::ffi::OsStr::new("WINEPREFIX"),
                    std::ffi::OsStr::new("prefix")
                )
            ]
        );
    }

    #[test]
    fn steam_execution_refuses_a_client_without_bootstrap_provenance() {
        let temp = TempDir::new();
        let prefix = temp.0.join("prefix");
        fs::create_dir_all(prefix.join("drive_c/Program Files (x86)/Steam")).unwrap();
        fs::write(
            prefix.join("drive_c/Program Files (x86)/Steam/Steam.exe"),
            b"MZ",
        )
        .unwrap();
        let error = steam_prefix_status::validate_expected_runtime(
            42,
            &prefix,
            "wine-test",
            &"ab".repeat(32),
        )
        .unwrap_err();
        assert!(error.message.contains("bootstrap marker is missing"));
    }

    #[test]
    fn multi_depot_view_maps_files_without_copying_and_writes_all_manifests() {
        let temp = TempDir::new();
        let first = temp.0.join("depot-1");
        let second = temp.0.join("depot-2");
        let prefix = temp.0.join("game/prefix");
        let steam_root = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(first.join("Binaries")).unwrap();
        fs::create_dir_all(second.join("BioGame")).unwrap();
        fs::create_dir_all(&steam_root).unwrap();
        fs::write(first.join("Binaries/game.exe"), b"MZgame").unwrap();
        fs::write(second.join("BioGame/content.bin"), b"content").unwrap();
        fs::write(steam_root.join("Steam.exe"), b"MZ").unwrap();
        let plan = LaunchPlan {
            app_id: 42,
            game_name: "Multi Depot".to_owned(),
            steam_build_id: Some("9".to_owned()),
            windows_depot_id: None,
            windows_depots: vec![
                crate::profile::DepotSpec {
                    id: 43,
                    manifest_id: Some(100),
                },
                crate::profile::DepotSpec {
                    id: 44,
                    manifest_id: Some(200),
                },
            ],
            payload_root: first.clone(),
            payload_roots: vec![first.clone(), second.clone()],
            executable: first.join("Binaries/game.exe"),
            profile_sha256: "ef".repeat(32),
            runtime_id: "fake".to_owned(),
            runtime_kind: Some(RuntimeKind::ExternalExecutable),
            runtime_entrypoint: Some(temp.0.join("wine")),
            runtime_fingerprint_sha256: Some("ab".repeat(32)),
            runtime_components: Vec::new(),
            renderer: "auto".to_owned(),
            graphics_api: crate::profile::GraphicsApi::Unknown,
            graphics_evidence: Vec::new(),
            launch_mode: LaunchMode::Steam,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: Some("Multi".to_owned()),
            steam_launcher_bypass: None,
            windows_build_id: Some("9".to_owned()),
            windows_depot_manifest_id: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };

        let prepared = prepare_steam_launch(&plan, &prefix).unwrap();
        let view = fs::read_link(&prepared.library_link).unwrap();
        assert!(
            fs::symlink_metadata(view.join("Binaries/game.exe"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(view.join("BioGame/content.bin"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(view.join("Binaries/game.exe")).unwrap(), b"MZgame");
        let manifest = fs::read_to_string(prepared.appmanifest).unwrap();
        assert!(manifest.contains("\"43\""));
        assert!(manifest.contains("\"manifest\"\t\t\"100\""));
        assert!(manifest.contains("\"44\""));
        assert!(manifest.contains("\"manifest\"\t\t\"200\""));
    }

    #[test]
    fn multi_depot_view_refuses_conflicting_files() {
        let temp = TempDir::new();
        let first = temp.0.join("depot-1");
        let second = temp.0.join("depot-2");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("same.bin"), b"one").unwrap();
        fs::write(second.join("same.bin"), b"two").unwrap();
        let error =
            materialize_payload_view(&[first, second], &temp.0.join("views/profile")).unwrap_err();
        assert!(error.message.contains("refusing implicit overwrite"));
    }

    #[test]
    fn payload_view_rebuilds_broken_absolute_links() {
        let temp = TempDir::new();
        let source = temp.0.join("source");
        let destination = temp.0.join("views/profile");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("game.exe"), b"MZ-new").unwrap();
        fs::create_dir_all(&destination).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            temp.0.join("rotated-source/game.exe"),
            destination.join("game.exe"),
        )
        .unwrap();

        let view = materialize_payload_view(std::slice::from_ref(&source), &destination).unwrap();
        assert_eq!(fs::read(view.join("game.exe")).unwrap(), b"MZ-new");
        assert!(
            fs::symlink_metadata(view.join("game.exe"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::canonicalize(view.join("game.exe")).unwrap(),
            fs::canonicalize(source.join("game.exe")).unwrap()
        );
    }

    #[test]
    fn interrupted_install_tree_is_recovered_once() {
        let temp = TempDir::new();
        let common = temp.0.join("common");
        let library = common.join("Legacy Game");
        let source = common.join(".Legacy Game.macroton-source-deadbeef");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("game.exe"), b"MZ").unwrap();

        recover_interrupted_install_tree(&common, "Legacy Game", &library).unwrap();
        assert!(library.is_dir());
        assert!(!source.exists());

        // A second preparation is idempotent and does not move the tree again.
        recover_interrupted_install_tree(&common, "Legacy Game", &library).unwrap();
        assert!(library.join("game.exe").is_file());
    }

    #[test]
    #[cfg(unix)]
    fn advances_between_macroton_payload_views() {
        let temp = TempDir::new();
        let common = temp.0.join("common");
        let link = common.join("Legacy Game");
        let old_view = temp.0.join("games/old/payload-views/old");
        let new_view = temp.0.join("games/new/payload-views/new");
        fs::create_dir_all(&old_view).unwrap();
        fs::create_dir_all(&new_view).unwrap();
        fs::create_dir_all(&common).unwrap();
        std::os::unix::fs::symlink(&old_view, &link).unwrap();

        ensure_payload_link(&link, &new_view).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), new_view);
    }

    #[test]
    fn dxvk_renderer_deploys_verified_d3d9_beside_game_in_private_view() {
        let temp = TempDir::new();
        let payload = temp.0.join("payload");
        let prefix = temp.0.join("game/prefix");
        let runtime = temp.0.join("runtime");
        let steam_root = prefix.join("drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(payload.join("bin")).unwrap();
        fs::create_dir_all(runtime.join("bin")).unwrap();
        fs::create_dir_all(runtime.join("lib/wine/i386-windows")).unwrap();
        fs::create_dir_all(&steam_root).unwrap();
        fs::write(payload.join("bin/game.exe"), b"MZgame").unwrap();
        fs::write(runtime.join("bin/wine"), b"runtime").unwrap();
        fs::write(runtime.join("lib/wine/i386-windows/d3d9.dll"), b"MZdxvk").unwrap();
        fs::write(steam_root.join("Steam.exe"), b"MZ").unwrap();
        let plan = LaunchPlan {
            app_id: 42,
            game_name: "DXVK Game".to_owned(),
            steam_build_id: None,
            windows_depot_id: Some(43),
            windows_depots: vec![crate::profile::DepotSpec {
                id: 43,
                manifest_id: None,
            }],
            payload_root: payload.clone(),
            payload_roots: vec![payload.clone()],
            executable: payload.join("bin/game.exe"),
            profile_sha256: "12".repeat(32),
            runtime_id: "wine-dxvk".to_owned(),
            runtime_kind: Some(RuntimeKind::ExternalExecutable),
            runtime_entrypoint: Some(runtime.join("bin/wine")),
            runtime_fingerprint_sha256: Some("ab".repeat(32)),
            runtime_components: vec!["dxvk-d3d9-moltenvk-test".to_owned()],
            renderer: "dxvk-d3d9-moltenvk".to_owned(),
            graphics_api: crate::profile::GraphicsApi::D3d9,
            graphics_evidence: vec!["d3d9".to_owned()],
            launch_mode: LaunchMode::Steam,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: Some("DXVK Game".to_owned()),
            steam_launcher_bypass: None,
            windows_build_id: None,
            windows_depot_manifest_id: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };

        let prepared = prepare_steam_launch(&plan, &prefix).unwrap();
        let view = fs::read_link(&prepared.library_link).unwrap();
        assert_ne!(view, payload);
        assert_eq!(fs::read(view.join("bin/d3d9.dll")).unwrap(), b"MZdxvk");
        assert!(!payload.join("bin/d3d9.dll").exists());
        assert!(
            fs::symlink_metadata(view.join("bin/game.exe"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn dxvk_renderer_refuses_a_different_existing_game_local_dll() {
        let temp = TempDir::new();
        let payload = temp.0.join("payload");
        let view = temp.0.join("view");
        let runtime = temp.0.join("runtime");
        fs::create_dir_all(&payload).unwrap();
        fs::create_dir_all(&view).unwrap();
        fs::create_dir_all(runtime.join("bin")).unwrap();
        fs::create_dir_all(runtime.join("lib/wine/i386-windows")).unwrap();
        fs::write(payload.join("game.exe"), b"MZgame").unwrap();
        fs::write(view.join("d3d9.dll"), b"MZother").unwrap();
        fs::write(runtime.join("lib/wine/i386-windows/d3d9.dll"), b"MZdxvk").unwrap();
        let plan = LaunchPlan {
            app_id: 42,
            game_name: "Conflict".to_owned(),
            steam_build_id: None,
            windows_depot_id: None,
            windows_depots: Vec::new(),
            payload_root: payload.clone(),
            payload_roots: vec![payload.clone()],
            executable: payload.join("game.exe"),
            profile_sha256: "34".repeat(32),
            runtime_id: "wine-dxvk".to_owned(),
            runtime_kind: Some(RuntimeKind::ExternalExecutable),
            runtime_entrypoint: Some(runtime.join("bin/wine")),
            runtime_fingerprint_sha256: Some("ab".repeat(32)),
            runtime_components: vec!["dxvk-d3d9-moltenvk-test".to_owned()],
            renderer: "dxvk-d3d9-moltenvk".to_owned(),
            graphics_api: crate::profile::GraphicsApi::D3d9,
            graphics_evidence: vec!["d3d9".to_owned()],
            launch_mode: LaunchMode::Steam,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: Some("Conflict".to_owned()),
            steam_launcher_bypass: None,
            windows_build_id: None,
            windows_depot_manifest_id: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };

        let error = deploy_dxvk_d3d9(&plan, &view).unwrap_err();
        assert!(error.message.contains("refusing to replace"));
        assert_eq!(fs::read(view.join("d3d9.dll")).unwrap(), b"MZother");
    }

    #[test]
    fn dxvk_renderer_detaches_source_symlink_before_writing_private_dll() {
        let temp = TempDir::new();
        let payload = temp.0.join("payload");
        let view = temp.0.join("view");
        let runtime = temp.0.join("runtime");
        fs::create_dir_all(&payload).unwrap();
        fs::create_dir_all(&view).unwrap();
        fs::create_dir_all(runtime.join("lib/wine/i386-windows")).unwrap();
        fs::write(payload.join("game.exe"), b"MZgame").unwrap();
        fs::write(payload.join("d3d9.dll"), b"MZsource").unwrap();
        fs::write(runtime.join("lib/wine/i386-windows/d3d9.dll"), b"MZdxvk").unwrap();
        std::os::unix::fs::symlink(payload.join("d3d9.dll"), view.join("d3d9.dll")).unwrap();
        let plan = LaunchPlan {
            app_id: 42,
            game_name: "Symlink Source".to_owned(),
            steam_build_id: None,
            windows_depot_id: None,
            windows_depots: Vec::new(),
            payload_root: payload.clone(),
            payload_roots: vec![payload.clone()],
            executable: payload.join("game.exe"),
            profile_sha256: "56".repeat(32),
            runtime_id: "wine-dxvk".to_owned(),
            runtime_kind: Some(RuntimeKind::ExternalExecutable),
            runtime_entrypoint: Some(runtime.join("bin/wine")),
            runtime_fingerprint_sha256: Some("cd".repeat(32)),
            runtime_components: vec!["dxvk-d3d9-moltenvk-test".to_owned()],
            renderer: "dxvk-d3d9-moltenvk".to_owned(),
            graphics_api: crate::profile::GraphicsApi::D3d9,
            graphics_evidence: vec!["d3d9".to_owned()],
            launch_mode: LaunchMode::Steam,
            steam_client_mode: SteamClientMode::Standard,
            steam_install_dir: Some("Symlink Source".to_owned()),
            steam_launcher_bypass: None,
            windows_build_id: None,
            windows_depot_manifest_id: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            blockers: Vec::new(),
        };

        deploy_dxvk_d3d9(&plan, &view).unwrap();
        assert_eq!(fs::read(payload.join("d3d9.dll")).unwrap(), b"MZsource");
        assert_eq!(fs::read(view.join("d3d9.dll")).unwrap(), b"MZdxvk");
        assert!(
            !fs::symlink_metadata(view.join("d3d9.dll"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
