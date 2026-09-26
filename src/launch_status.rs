use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::stop;

const LAUNCH_BOUNDARY_FILE: &str = ".macroton-launch-console-offset";
const LAUNCH_DIAGNOSTIC_BOUNDARY_FILE: &str = ".macroton-launch-diagnostic-offset";
const LAUNCH_GAMEPROCESS_BOUNDARY_FILE: &str = ".macroton-launch-gameprocess-offset";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchState {
    NoEvidence,
    Submitted,
    InstallingPrerequisites,
    WaitingEula,
    WaitingCloud,
    WaitingUser,
    RunningObserved,
    Exited,
    Failed,
}

impl LaunchState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoEvidence => "no-evidence",
            Self::Submitted => "submitted",
            Self::InstallingPrerequisites => "installing-prerequisites",
            Self::WaitingEula => "waiting-eula",
            Self::WaitingCloud => "waiting-cloud",
            Self::WaitingUser => "waiting-user",
            Self::RunningObserved => "running-observed",
            Self::Exited => "exited",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchStatus {
    pub app_id: u32,
    pub steam_root: PathBuf,
    pub log_path: PathBuf,
    pub state: LaunchState,
    pub latest_event: Option<String>,
    /// Host macOS PID when the exact AppID prefix owns one game process.
    pub process_id: Option<u32>,
    /// PID reported by Windows Steam inside Wine.
    pub steam_process_id: Option<u32>,
    pub exit_code: Option<i32>,
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

pub fn inspect(app_id: u32, data_root: &Path) -> Result<LaunchStatus, Error> {
    let app_root = data_root.join("games").join(app_id.to_string());
    let steam_root = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix/drive_c/Program Files (x86)/Steam");
    let boundary = fs::read_to_string(app_root.join(LAUNCH_BOUNDARY_FILE))
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok());
    let mut status = inspect_steam_root_with_offset(app_id, &steam_root, boundary)?;
    // Wine can create the host process before its Steam console log flushes
    // the corresponding `Game process added` line.  When Macroton has a fresh
    // launch boundary, use the exact-prefix resource inventory as a temporary
    // host-side observation rather than reporting a live game as absent.  A
    // unique executable is required; ambiguity remains fail-closed.
    if status.state != LaunchState::Exited && boundary.is_some() {
        let resources = stop::inspect_resources(app_id, data_root).map_err(|error| Error {
            message: format!("cannot map Steam's Windows PID to the exact prefix: {error}"),
        })?;
        let expected_executable = status
            .latest_event
            .as_deref()
            .and_then(parse_executable_basename);
        let host_game_pids = resources
            .processes
            .iter()
            .filter(|process| {
                process.category == "windows-game"
                    && expected_executable
                        .as_deref()
                        .is_none_or(|expected| process.command.eq_ignore_ascii_case(expected))
            })
            .map(|process| process.pid)
            .collect::<Vec<_>>();
        status.process_id = match host_game_pids.as_slice() {
            [pid] => Some(*pid),
            _ => None,
        };
        // A Steam interstitial/auth event can be flushed after the game has
        // already been handed off.  A unique exact-prefix game process is
        // stronger host-side evidence than that stale client event, so do not
        // leave the lifecycle stuck at `waiting-user`/`submitted` while the
        // actual executable is alive.  This still says only
        // `running-observed`; it does not imply a visible or playable frame.
        if status.process_id.is_some()
            && matches!(
                status.state,
                LaunchState::Submitted
                    | LaunchState::WaitingUser
                    | LaunchState::WaitingEula
                    | LaunchState::WaitingCloud
                    | LaunchState::InstallingPrerequisites
            )
        {
            status.state = LaunchState::RunningObserved;
        }
    }
    Ok(status)
}

pub fn inspect_steam_root(app_id: u32, steam_root: &Path) -> Result<LaunchStatus, Error> {
    inspect_steam_root_with_offset(app_id, steam_root, None)
}

/// Record the console-log byte offset immediately before a new Steam game
/// request. Managed `launch-status` reads only events appended after this
/// boundary, so a prior run cannot masquerade as the current result.
pub fn record_launch_boundary(app_id: u32, data_root: &Path) -> Result<(), Error> {
    let app_root = data_root.join("games").join(app_id.to_string());
    fs::create_dir_all(&app_root).map_err(|error| Error {
        message: format!("cannot create launch boundary directory: {error}"),
    })?;
    let console_log =
        app_root.join("prefix/drive_c/Program Files (x86)/Steam/logs/console_log.txt");
    let diagnostic_log = app_root.join("steam-client.log");
    for (boundary_file, log_path) in [
        (LAUNCH_BOUNDARY_FILE, console_log),
        (LAUNCH_DIAGNOSTIC_BOUNDARY_FILE, diagnostic_log),
        (
            LAUNCH_GAMEPROCESS_BOUNDARY_FILE,
            app_root.join("prefix/drive_c/Program Files (x86)/Steam/logs/gameprocess_log.txt"),
        ),
    ] {
        let offset = fs::metadata(&log_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        fs::write(app_root.join(boundary_file), offset.to_string()).map_err(|error| Error {
            message: format!("cannot write launch boundary: {error}"),
        })?;
    }
    Ok(())
}

fn inspect_steam_root_with_offset(
    app_id: u32,
    steam_root: &Path,
    boundary: Option<u64>,
) -> Result<LaunchStatus, Error> {
    if !steam_root.is_dir() {
        return Err(Error {
            message: format!(
                "Windows Steam root for AppID {app_id} does not exist: {}",
                steam_root.display()
            ),
        });
    }
    let log_path = steam_root.join("logs/console_log.txt");
    if !log_path.is_file() {
        return Ok(LaunchStatus {
            app_id,
            steam_root: steam_root.to_path_buf(),
            log_path,
            state: LaunchState::NoEvidence,
            latest_event: None,
            process_id: None,
            steam_process_id: None,
            exit_code: None,
        });
    }
    let mut contents = fs::read(&log_path).map_err(|error| Error {
        message: format!(
            "cannot read Steam console log {}: {error}",
            log_path.display()
        ),
    })?;
    if let Some(offset) = boundary {
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        // Steam may truncate/recreate console_log.txt when its client restarts.
        // An old boundary can then be larger than the new file; treating that
        // as an empty suffix loses the current ShowInterstitials/game events.
        // Restart the scan at byte zero when truncation is detected.
        contents = contents
            .get(offset..)
            .unwrap_or(contents.as_slice())
            .to_vec();
    }
    let contents = String::from_utf8_lossy(&contents);
    let app_marker = format!("AppID {app_id}");
    let launch_marker = format!("-applaunch {app_id}");
    let mut state = LaunchState::NoEvidence;
    let mut latest_event = None;
    let mut process_id = None;
    let mut steam_process_id = None;
    let mut exit_code = None;
    for line in contents.lines() {
        let lower = line.to_ascii_lowercase();
        let next = if line.contains(&launch_marker) {
            // Every ExecCommandLine starts a new lifecycle boundary.  Keeping
            // the prior PID here makes a dead process look live forever and
            // blocks the next launch as `stale-running-event`; the following
            // Game process added event will restore RunningObserved for the
            // new request.
            process_id = None;
            steam_process_id = None;
            exit_code = None;
            Some(LaunchState::Submitted)
        } else if line.contains(&app_marker) && lower.contains("showeula") {
            Some(LaunchState::WaitingEula)
        } else if line.contains(&app_marker) && lower.contains("runninginstallscript") {
            Some(LaunchState::InstallingPrerequisites)
        } else if line.contains(&app_marker)
            && lower.contains("waiting for user response to synchronizingcloud")
            && lower.contains("pendingcloudsessions")
        {
            Some(LaunchState::WaitingCloud)
        } else if line.contains(&app_marker) && lower.contains("waiting for user response") {
            Some(LaunchState::WaitingUser)
        } else if line.contains(&app_marker) && lower.contains("game process added") {
            process_id = parse_process_id(line);
            steam_process_id = process_id;
            exit_code = None;
            Some(LaunchState::RunningObserved)
        } else if line.contains(&app_marker) && lower.contains("game process removed") {
            process_id = None;
            Some(LaunchState::Exited)
        } else if line.contains(&app_marker)
            && (lower.contains("launch failed") || lower.contains("failed to start"))
        {
            process_id = None;
            Some(LaunchState::Failed)
        } else if lower.contains("logonfailure") || lower.contains("auth error") {
            // These lines are not AppID-tagged, but the inspection suffix is
            // bounded by a fresh launch boundary. Report the actionable
            // Steam session failure instead of the opaque no-evidence state.
            process_id = None;
            Some(LaunchState::Failed)
        } else {
            None
        };
        if let Some(next) = next {
            state = next;
            latest_event = Some(line.to_owned());
        }
    }
    // Some Steam builds emit the authoritative AppID handoff only in
    // gameprocess_log.txt, not console_log.txt.  Read only the suffix after
    // Macroton's launch boundary so a historical game cannot masquerade as a
    // current launch.  This is especially important for short-lived 2D games
    // whose host process exits before the lifecycle poll runs.
    if let Some(offset) = boundary {
        let gameprocess_path = steam_root.join("logs/gameprocess_log.txt");
        if let Ok(mut gameprocess) = fs::read(&gameprocess_path) {
            let boundary_path = steam_root
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .and_then(Path::parent)
                .map(|root| root.join(LAUNCH_GAMEPROCESS_BOUNDARY_FILE));
            let gameprocess_offset = boundary_path
                .and_then(|path| fs::read_to_string(path).ok())
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(usize::try_from(offset).unwrap_or(usize::MAX));
            gameprocess = gameprocess
                .get(gameprocess_offset..)
                .unwrap_or(gameprocess.as_slice())
                .to_vec();
            for line in String::from_utf8_lossy(&gameprocess).lines() {
                let lower = line.to_ascii_lowercase();
                if line.contains(&format!("AppID {app_id}"))
                    && lower.contains("adding pid")
                    && lower.contains("tracked process")
                {
                    steam_process_id = parse_gameprocess_pid(line);
                    process_id = None;
                    exit_code = None;
                    state = LaunchState::RunningObserved;
                    latest_event = Some(line.to_owned());
                } else if line.contains(&format!("AppID {app_id}"))
                    && lower.contains("no longer tracking pid")
                {
                    process_id = None;
                    state = LaunchState::Exited;
                    exit_code = parse_exit_code(line);
                    latest_event = Some(line.to_owned());
                }
            }
        }
    }
    if state == LaunchState::Exited {
        exit_code = latest_gameprocess_exit_code(app_id, steam_root)?;
    }
    Ok(LaunchStatus {
        app_id,
        steam_root: steam_root.to_path_buf(),
        log_path,
        state,
        latest_event,
        process_id,
        steam_process_id,
        exit_code,
    })
}

fn latest_gameprocess_exit_code(app_id: u32, steam_root: &Path) -> Result<Option<i32>, Error> {
    let path = steam_root.join("logs/gameprocess_log.txt");
    if !path.is_file() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path).map_err(|error| Error {
        message: format!(
            "cannot read Steam game process log {}: {error}",
            path.display()
        ),
    })?;
    let marker = format!("AppID {app_id} no longer tracking PID ");
    Ok(contents
        .lines()
        .filter(|line| line.contains(&marker))
        .filter_map(parse_exit_code)
        .next_back())
}

fn parse_exit_code(line: &str) -> Option<i32> {
    line.split("exit code ").nth(1)?.trim().parse().ok()
}

fn parse_process_id(line: &str) -> Option<u32> {
    let value = line.split("ProcID ").nth(1)?.split([',', ' ']).next()?;
    value.parse().ok()
}

fn parse_gameprocess_pid(line: &str) -> Option<u32> {
    line.split("adding PID ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn parse_executable_basename(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let end = lower.find(".exe")? + 4;
    let start = line[..end]
        .rfind(['\\', '/', '"', ' '])
        .map_or(0, |index| index + 1);
    let name = &line[start..end];
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "macroton-launch-status-test-{}-{sequence}",
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

    fn write_log(temp: &TempDir, contents: &str) -> PathBuf {
        let steam = temp.0.join("Steam");
        fs::create_dir_all(steam.join("logs")).unwrap();
        fs::write(steam.join("logs/console_log.txt"), contents).unwrap();
        steam
    }

    fn write_gameprocess_log(steam: &Path, contents: &str) {
        fs::write(steam.join("logs/gameprocess_log.txt"), contents).unwrap();
    }

    #[test]
    fn managed_inspection_ignores_events_before_recorded_launch_boundary() {
        let temp = TempDir::new();
        let app_root = temp.0.join("games/42");
        let steam = app_root.join("prefix/drive_c/Program Files (x86)/Steam");
        fs::create_dir_all(steam.join("logs")).unwrap();
        fs::write(
            steam.join("logs/console_log.txt"),
            "Game process added : AppID 42 old.exe, ProcID 1, IP 0.0.0.0:0\n",
        )
        .unwrap();
        record_launch_boundary(42, &temp.0).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(steam.join("logs/console_log.txt"))
            .unwrap()
            .write_all(b"GameAction [AppID 42] : LaunchApp waiting for user response to ShowEula\n")
            .unwrap();

        let status = inspect(42, &temp.0).unwrap();
        assert_eq!(status.state, LaunchState::WaitingEula);
        assert!(status.latest_event.unwrap().contains("ShowEula"));
    }

    #[test]
    fn a_truncated_console_log_restarts_scan_from_the_new_file_start() {
        let temp = TempDir::new();
        let steam = write_log(
            &temp,
            "GameAction [AppID 42] : LaunchApp waiting for user response to ShowInterstitials\n",
        );
        let status = inspect_steam_root_with_offset(42, &steam, Some(10_000)).unwrap();
        assert_eq!(status.state, LaunchState::WaitingUser);
        assert!(status.latest_event.unwrap().contains("ShowInterstitials"));
    }

    #[test]
    fn reports_eula_as_a_user_gate_instead_of_a_launch_pass() {
        let temp = TempDir::new();
        let steam = write_log(
            &temp,
            "ExecCommandLine: Steam.exe -applaunch 17460\nGameAction [AppID 17460] : LaunchApp waiting for user response to ShowEula\n",
        );

        let status = inspect_steam_root(17_460, &steam).unwrap();
        assert_eq!(status.state, LaunchState::WaitingEula);
        assert_eq!(status.process_id, None);
        assert_eq!(status.exit_code, None);
    }

    #[test]
    fn reports_pending_cloud_session_as_a_distinct_user_gate() {
        let temp = TempDir::new();
        let steam = write_log(
            &temp,
            "ExecCommandLine: Steam.exe -applaunch 26800\nGameAction [AppID 26800] : LaunchApp waiting for user response to SynchronizingCloud \"pendingcloudsessions\"\n",
        );

        let status = inspect_steam_root(26_800, &steam).unwrap();
        assert_eq!(status.state, LaunchState::WaitingCloud);
        assert_eq!(status.process_id, None);
        assert_eq!(status.exit_code, None);
    }

    #[test]
    fn reports_fresh_steam_auth_failure_as_failed() {
        let temp = TempDir::new();
        let steam = write_log(&temp, "LogonFailure No Connection\n");
        let status = inspect_steam_root_with_offset(42, &steam, Some(0)).unwrap();
        assert_eq!(status.state, LaunchState::Failed);
        assert!(status.latest_event.unwrap().contains("LogonFailure"));
    }

    #[test]
    fn a_real_process_supersedes_the_prior_user_gate() {
        let temp = TempDir::new();
        let steam = write_log(
            &temp,
            "ExecCommandLine: Steam.exe -applaunch 42\nGameAction [AppID 42] : LaunchApp waiting for user response to ShowEula\nGame process added : AppID 42 game.exe, ProcID 1234, IP 0.0.0.0:0\n",
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, LaunchState::RunningObserved);
        assert_eq!(status.process_id, Some(1234));
        assert_eq!(status.steam_process_id, Some(1234));
        assert_eq!(status.exit_code, None);
    }

    #[test]
    fn a_new_submission_supersedes_an_old_exit() {
        let temp = TempDir::new();
        let steam = write_log(
            &temp,
            "Game process added : AppID 42 game.exe, ProcID 12, IP 0.0.0.0:0\nGame process removed: AppID 42 game.exe, ProcID 12\nExecCommandLine: Steam.exe -applaunch 42\n",
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, LaunchState::Submitted);
        assert_eq!(status.process_id, None);
        assert_eq!(status.steam_process_id, None);
        assert_eq!(status.exit_code, None);
    }

    #[test]
    fn a_late_submission_marker_starts_a_new_lifecycle_boundary() {
        let temp = TempDir::new();
        let steam = write_log(
            &temp,
            "ExecCommandLine: Steam.exe -applaunch 42\nGame process added : AppID 42 game.exe, ProcID 1234, IP 0.0.0.0:0\nExecCommandLine: Steam.exe -applaunch 42\n",
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, LaunchState::Submitted);
        assert_eq!(status.process_id, None);
        assert_eq!(status.steam_process_id, None);
        assert_eq!(status.exit_code, None);
    }

    #[test]
    fn reports_signed_prerequisite_installation_separately() {
        let temp = TempDir::new();
        let steam = write_log(
            &temp,
            "ExecCommandLine: Steam.exe -applaunch 42\nGameAction [AppID 42] : LaunchApp changed task to RunningInstallScript\n",
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, LaunchState::InstallingPrerequisites);
        assert_eq!(status.process_id, None);
        assert_eq!(status.exit_code, None);
    }

    #[test]
    fn reports_the_latest_steam_tracked_exit_code() {
        let temp = TempDir::new();
        let steam = write_log(
            &temp,
            "Game process added : AppID 42 game.exe, ProcID 12, IP 0.0.0.0:0\nGame process removed: AppID 42 game.exe, ProcID 12\n",
        );
        write_gameprocess_log(
            &steam,
            "AppID 42 no longer tracking PID 9, exit code -1\nAppID 42 no longer tracking PID 12, exit code 0\n",
        );

        let status = inspect_steam_root(42, &steam).unwrap();
        assert_eq!(status.state, LaunchState::Exited);
        assert_eq!(status.exit_code, Some(0));
    }

    #[test]
    fn extracts_the_game_executable_instead_of_a_steam_helper() {
        let line = r#"Game process added : AppID 400 ""C:\Program Files (x86)\Steam\steamapps\common\Portal\hl2.exe" -game portal", ProcID 1232"#;
        assert_eq!(parse_executable_basename(line).as_deref(), Some("hl2.exe"));
    }

    #[test]
    fn parses_gameprocess_log_pid_format() {
        let line = r#"[2026-09-07 19:28:54] AppID 606680 adding PID 1352 as a tracked process ""C:\Program Files (x86)\Steam\steamapps\common\Silver\silver.exe"""#;
        assert_eq!(parse_gameprocess_pid(line), Some(1352));
    }
}
