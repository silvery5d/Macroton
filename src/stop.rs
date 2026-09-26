use crate::runtime;
use crate::steam_session;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const PREFIX_RUNTIME_IDENTITY: &str = ".macroton-runtime-identity";
const STEAM_BOOTSTRAP_MARKER: &str = ".macroton-steam-bootstrap.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopResult {
    pub prefix: PathBuf,
    pub runtime_id: String,
    pub terminated_orphans: Vec<u32>,
    pub killed_orphans: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixResourceUsage {
    pub prefix: PathBuf,
    pub pids: Vec<u32>,
    pub total_rss_kib: u64,
    pub processes: Vec<ProcessResourceUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessResourceUsage {
    pub pid: u32,
    pub rss_kib: u64,
    pub command: String,
    pub category: &'static str,
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

pub fn stop(app_id: u32, data_root: &Path, runtime_root: &Path) -> Result<StopResult, Error> {
    stop_with_runtime_id(app_id, data_root, runtime_root, None)
}

/// Stop only the Wine processes owned by one exact Macroton prefix while
/// preserving the authenticated Steam-session provenance marker. This is used
/// for recovery when Steam's client is alive but has become stale; the next
/// launch can reuse the login without forcing another QR login.
pub fn stop_processes_preserve_session(
    app_id: u32,
    data_root: &Path,
    runtime_root: &Path,
) -> Result<StopResult, Error> {
    stop_with_runtime_id_policy(app_id, data_root, runtime_root, None, false)
}

pub fn stop_with_runtime_id(
    app_id: u32,
    data_root: &Path,
    runtime_root: &Path,
    runtime_id_override: Option<&str>,
) -> Result<StopResult, Error> {
    stop_with_runtime_id_policy(app_id, data_root, runtime_root, runtime_id_override, true)
}

fn stop_with_runtime_id_policy(
    app_id: u32,
    data_root: &Path,
    runtime_root: &Path,
    runtime_id_override: Option<&str>,
    clear_session: bool,
) -> Result<StopResult, Error> {
    let prefix = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix");
    if !prefix.is_dir() {
        return Err(Error {
            message: format!("managed prefix does not exist: {}", prefix.display()),
        });
    }
    let canonical_prefix = prefix.canonicalize().map_err(|error| Error {
        message: format!(
            "cannot resolve managed prefix {}: {error}",
            prefix.display()
        ),
    })?;
    let runtime_id = match runtime_id_override {
        Some(runtime_id) if !runtime_id.is_empty() => runtime_id.to_owned(),
        Some(_) => {
            return Err(Error {
                message: "explicit runtime ID must not be empty".to_owned(),
            });
        }
        None => read_prefix_runtime_id(&prefix)?,
    };
    let installed =
        runtime::resolve_installed(runtime_root, &runtime_id).map_err(|error| Error {
            message: format!("cannot resolve prefix runtime {runtime_id:?}: {error}"),
        })?;
    let wineserver = installed
        .entrypoint
        .parent()
        .map(|parent| parent.join("wineserver"))
        .filter(|path| path.is_file())
        .ok_or_else(|| Error {
            message: format!(
                "runtime {runtime_id:?} has no wineserver beside {}",
                installed.entrypoint.display()
            ),
        })?;

    // WINEPREFIX is always explicit: stopping one game must never affect another
    // Macroton prefix or an unrelated host Wine session.
    let status = Command::new(&wineserver)
        .arg("-k")
        .env("WINEPREFIX", &canonical_prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| Error {
            message: format!(
                "cannot request prefix shutdown with {}: {error}",
                wineserver.display()
            ),
        })?;
    // Wine returns 1 when no matching server exists. That is already the desired
    // state, so only abnormal status values are rejected.
    if !status.success() && status.code() != Some(1) {
        return Err(Error {
            message: format!("wineserver shutdown failed with {status}"),
        });
    }

    thread::sleep(Duration::from_millis(250));
    let mut terminated_orphans = prefix_owned_wine_pids(&canonical_prefix)?;
    let term_failures =
        signal_verified_prefix_pids(&terminated_orphans, &canonical_prefix, "-TERM")?;

    for _ in 0..20 {
        thread::sleep(Duration::from_millis(50));
        if prefix_owned_wine_pids(&canonical_prefix)?.is_empty() {
            break;
        }
    }
    let mut killed_orphans = prefix_owned_wine_pids(&canonical_prefix)?;
    let mut kill_failures =
        signal_verified_prefix_pids(&killed_orphans, &canonical_prefix, "-KILL")?;

    // Steam can finish an already queued -applaunch after wineserver -k and
    // briefly recreate the game process. Drain the exact prefix for a bounded
    // grace period instead of returning success on the first empty snapshot.
    // This preserves the containment guarantee while avoiding an unbounded
    // wait on a broken Wine process.
    let mut remaining = Vec::new();
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(100));
        remaining = prefix_owned_wine_pids(&canonical_prefix)?;
        if remaining.is_empty() {
            break;
        }
        for pid in &remaining {
            if !killed_orphans.contains(pid) {
                killed_orphans.push(*pid);
            }
        }
        kill_failures.extend(signal_verified_prefix_pids(
            &remaining,
            &canonical_prefix,
            "-KILL",
        )?);
    }
    if !remaining.is_empty() {
        let mut diagnostics = Vec::new();
        if !term_failures.is_empty() {
            diagnostics.push(format!("TERM: {}", term_failures.join(", ")));
        }
        if !kill_failures.is_empty() {
            diagnostics.push(format!("KILL: {}", kill_failures.join(", ")));
        }
        let diagnostics = if diagnostics.is_empty() {
            "processes remained after verified signals without a command error".to_owned()
        } else {
            diagnostics.join("; ")
        };
        return Err(Error {
            message: format!(
                "cannot stop all verified prefix processes; remaining PIDs: {}; {diagnostics}",
                format_pids(&remaining)
            ),
        });
    }
    let remaining_resources = prefix_owned_resource_pids(&canonical_prefix)?;
    if !remaining_resources.is_empty() {
        return Err(Error {
            message: format!(
                "managed Windows game processes remained after exact wineserver shutdown; refusing to claim containment for PIDs: {}",
                format_pids(&remaining_resources)
            ),
        });
    }
    terminated_orphans.sort_unstable();
    terminated_orphans.dedup();
    if clear_session {
        steam_session::clear(&prefix).map_err(|error| Error {
            message: format!(
                "managed prefix stopped but session provenance cleanup failed: {error}"
            ),
        })?;
    }

    Ok(StopResult {
        prefix,
        runtime_id,
        terminated_orphans,
        killed_orphans,
    })
}

/// Return the resident memory owned by known Wine/Steam processes that have an
/// open path inside one exact managed prefix. This deliberately uses the same
/// fail-closed ownership test as `stop`; unrelated host Steam and Wine sessions
/// are never included.
pub fn inspect_resources(app_id: u32, data_root: &Path) -> Result<PrefixResourceUsage, Error> {
    let prefix = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix");
    if !prefix.is_dir() {
        return Err(Error {
            message: format!("managed prefix does not exist: {}", prefix.display()),
        });
    }
    let canonical_prefix = prefix.canonicalize().map_err(|error| Error {
        message: format!(
            "cannot resolve managed prefix {}: {error}",
            prefix.display()
        ),
    })?;
    let pids = prefix_owned_resource_pids(&canonical_prefix)?;
    let processes = resources_for_pids(&pids)?;
    let total_rss_kib = processes.iter().map(|process| process.rss_kib).sum();
    Ok(PrefixResourceUsage {
        prefix,
        pids,
        total_rss_kib,
        processes,
    })
}

fn prefix_owned_resource_pids(prefix: &Path) -> Result<Vec<u32>, Error> {
    let ps = match Command::new("ps")
        .args(["-axo", "pid=,comm="])
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(_) => return inspect_all_resource_open_paths(prefix),
    };
    if !ps.status.success() {
        return inspect_all_resource_open_paths(prefix);
    }
    let candidates = parse_resource_candidate_pids(&ps.stdout);
    let mut matches = Vec::new();
    for chunk in candidates.chunks(128) {
        let pid_list = chunk
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let output = Command::new("lsof")
            .args(["-n", "-a", "-p", &pid_list, "-Fpcn"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| Error {
                message: format!("cannot inspect resource candidate open paths: {error}"),
            })?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(Error {
                message: format!("resource candidate lsof failed with {}", output.status),
            });
        }
        matches.extend(parse_prefix_owned_pids(&output.stdout, prefix, false));
    }
    matches.sort_unstable();
    matches.dedup();
    Ok(matches)
}

fn inspect_all_resource_open_paths(prefix: &Path) -> Result<Vec<u32>, Error> {
    let output = Command::new("lsof")
        .arg("-Fpcn")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| Error {
            message: format!("cannot inspect all resource process open paths with lsof: {error}"),
        })?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(Error {
            message: format!("all-process resource lsof failed with {}", output.status),
        });
    }
    Ok(parse_prefix_owned_pids(&output.stdout, prefix, false))
}

fn resources_for_pids(pids: &[u32]) -> Result<Vec<ProcessResourceUsage>, Error> {
    if pids.is_empty() {
        return Ok(Vec::new());
    }
    let pid_list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let output = Command::new("ps")
        .args(["-o", "pid=,rss=,comm=", "-p", &pid_list])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| Error {
            message: format!("cannot inspect prefix process RSS with ps: {error}"),
        })?;
    // A process can disappear after the lsof ownership snapshot. ps then exits
    // nonzero or simply omits that PID; either case represents zero current RSS
    // for the vanished process. Syntax and permission failures normally include
    // stderr and no parseable rows, so preserve that diagnostic.
    let resources = parse_ps_resources(&output.stdout, pids);
    if !output.status.success() && resources.is_empty() && !output.stderr.is_empty() {
        return Err(Error {
            message: format!(
                "ps RSS inspection failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(resources)
}

fn parse_ps_resources(output: &[u8], selected_pids: &[u32]) -> Vec<ProcessResourceUsage> {
    let mut processes = output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid: u32 = fields.next()?.parse().ok()?;
            let rss_kib: u64 = fields.next()?.parse().ok()?;
            let command = fields.collect::<Vec<_>>().join(" ");
            (!command.is_empty() && selected_pids.contains(&pid)).then(|| ProcessResourceUsage {
                pid,
                rss_kib,
                command: process_basename(&command),
                category: resource_category(&command),
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by(|left, right| {
        right
            .rss_kib
            .cmp(&left.rss_kib)
            .then_with(|| left.pid.cmp(&right.pid))
    });
    processes
}

fn process_basename(command: &str) -> String {
    command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_owned()
}

fn resource_category(command: &str) -> &'static str {
    let command = process_basename(command).to_ascii_lowercase();
    match command.as_str() {
        // Macroton's reversible CEF workaround preserves Valve's helper as
        // `steamwebhelper_real.exe`; both names are Steam UI processes, not
        // game processes. Keep the backup name covered as well so a wrapper
        // A/B cannot inflate the game RSS bucket or trigger a false launch.
        "steamwebhelper.exe"
        | "steamwebhelper_real.exe"
        | "steamwebhelper_macroton-wrapper.exe" => "steam-cef",
        "steam.exe" | "steamservice.exe" | "steamservicetmp.exe" => "steam-client",
        "steamcmd.exe" => "steamcmd",
        "services.exe" | "plugplay.exe" | "svchost.exe" | "explorer.exe" | "rpcss.exe"
        | "conhost.exe" | "winedevice.exe" | "wineserver" => "wine-service",
        // Wine may run this helper while probing the host OpenGL stack before
        // the real PE game starts. It is not a game process and must not make
        // the resource guard or Steam lifecycle gate report a false pass.
        "gldriverquery.exe"
        | "gldriverquery64.exe"
        | "steamsysinfo.exe"
        | "steamerrorreporter.exe"
        | "steamerrorreporter64.exe" => "wine-helper",
        "winedbg" => "wine-debugger",
        "wine" | "wine64" | "wine-preloader" | "wine64-preloader" | "start.exe" => "wine-runtime",
        _ if command.ends_with(".exe") => "windows-game",
        _ => "other-managed",
    }
}

fn read_prefix_runtime_id(prefix: &Path) -> Result<String, Error> {
    let identity = prefix.join(PREFIX_RUNTIME_IDENTITY);
    match fs::read_to_string(&identity) {
        Ok(contents) => runtime_id_from_identity(&contents, &identity),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            read_bootstrap_runtime_id(&prefix.join(STEAM_BOOTSTRAP_MARKER))
        }
        Err(error) => Err(Error {
            message: format!(
                "cannot read prefix runtime identity {}: {error}",
                identity.display()
            ),
        }),
    }
}

fn runtime_id_from_identity(contents: &str, marker: &Path) -> Result<String, Error> {
    contents
        .lines()
        .find_map(|line| line.strip_prefix("runtime_id="))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Error {
            message: format!(
                "prefix runtime identity is missing runtime_id: {}",
                marker.display()
            ),
        })
}

fn read_bootstrap_runtime_id(marker: &Path) -> Result<String, Error> {
    let contents = fs::read_to_string(marker).map_err(|error| Error {
        message: format!(
            "cannot read prefix runtime identity or bootstrap marker {}: {error}",
            marker.display()
        ),
    })?;
    contents
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("\"runtime_id\":"))
        .map(str::trim)
        .and_then(|value| {
            value
                .strip_suffix(',')
                .unwrap_or(value)
                .trim()
                .strip_prefix('"')
        })
        .and_then(|value| value.strip_suffix('"'))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| Error {
            message: format!(
                "Steam bootstrap marker is missing runtime_id: {}",
                marker.display()
            ),
        })
}

fn prefix_owned_wine_pids(prefix: &Path) -> Result<Vec<u32>, Error> {
    let ps = match Command::new("ps")
        .args(["-axo", "pid=,comm="])
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(_) => return inspect_all_process_open_paths(prefix),
    };
    if !ps.status.success() {
        return inspect_all_process_open_paths(prefix);
    }
    let candidates = parse_known_process_pids(&ps.stdout);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut matches = Vec::new();
    // Keep argument vectors bounded even after a process-storm incident. lsof's
    // comma-separated -p selector is an OR list; -a combines it with no path
    // restriction, allowing us to verify every open path without scanning every
    // unrelated process on the host.
    for chunk in candidates.chunks(128) {
        let pid_list = chunk
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let output = Command::new("lsof")
            .args(["-n", "-a", "-p", &pid_list, "-Fpcn"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| Error {
                message: format!("cannot inspect candidate process open paths: {error}"),
            })?;
        if !output.status.success() && output.status.code() != Some(1) {
            return Err(Error {
                message: format!("lsof process inspection failed with {}", output.status),
            });
        }
        matches.extend(parse_prefix_owned_wine_pids(&output.stdout, prefix));
    }
    matches.sort_unstable();
    matches.dedup();
    Ok(matches)
}

fn inspect_all_process_open_paths(prefix: &Path) -> Result<Vec<u32>, Error> {
    let output = Command::new("lsof")
        .arg("-Fpcn")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| Error {
            message: format!("cannot inspect process open paths with lsof: {error}"),
        })?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(Error {
            message: format!("lsof process inspection failed with {}", output.status),
        });
    }
    Ok(parse_prefix_owned_wine_pids(&output.stdout, prefix))
}

fn parse_known_process_pids(output: &[u8]) -> Vec<u32> {
    let mut pids = output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| {
            let (pid, command) = line.trim().split_once(char::is_whitespace)?;
            let pid = pid.parse().ok()?;
            is_known_wine_process(command.trim()).then_some(pid)
        })
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn parse_resource_candidate_pids(output: &[u8]) -> Vec<u32> {
    let mut pids = output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| {
            let (pid, command) = line.trim().split_once(char::is_whitespace)?;
            let pid = pid.parse().ok()?;
            let basename = process_basename(command.trim()).to_ascii_lowercase();
            (is_known_wine_process(command.trim()) || basename.ends_with(".exe")).then_some(pid)
        })
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn signal_verified_prefix_pids(
    pids: &[u32],
    prefix: &Path,
    signal: &str,
) -> Result<Vec<String>, Error> {
    let mut failures = Vec::new();
    for pid in pids {
        // Re-read the exact process immediately before signalling it. This avoids
        // acting on a PID that exited and was reused after the first lsof scan.
        let output = Command::new("lsof")
            .args(["-p", &pid.to_string(), "-Fpcn"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| Error {
                message: format!("cannot revalidate process {pid} with lsof: {error}"),
            })?;
        if !parse_prefix_owned_wine_pids(&output.stdout, prefix).contains(pid) {
            continue;
        }
        let output = Command::new("kill")
            .args([signal, &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| Error {
                message: format!("cannot signal verified prefix process {pid}: {error}"),
            })?;
        if !output.status.success() {
            // A process can exit between revalidation and kill; re-check before
            // treating the non-zero status as a failure.
            let remaining = prefix_owned_wine_pids(prefix)?;
            if remaining.contains(pid) {
                let detail = String::from_utf8_lossy(&output.stderr);
                let detail = detail.trim();
                let detail = if detail.is_empty() {
                    "the operating system denied the signal without diagnostic output"
                } else {
                    detail
                };
                failures.push(format!("PID {pid}: {} ({detail})", output.status));
            }
        }
    }
    Ok(failures)
}

fn format_pids(pids: &[u32]) -> String {
    pids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_prefix_owned_wine_pids(output: &[u8], prefix: &Path) -> Vec<u32> {
    parse_prefix_owned_pids(output, prefix, true)
}

fn parse_prefix_owned_pids(output: &[u8], prefix: &Path, require_known: bool) -> Vec<u32> {
    let prefix = prefix.to_string_lossy();
    let mut current_pid = None;
    let mut current_command = None;
    let mut matches = Vec::new();
    for line in output.split(|byte| *byte == b'\n') {
        let Some((field, value)) = line.split_first() else {
            continue;
        };
        match field {
            b'p' => {
                current_pid = std::str::from_utf8(value)
                    .ok()
                    .and_then(|value| value.parse().ok());
                current_command = None;
            }
            b'c' => current_command = std::str::from_utf8(value).ok(),
            b'n' => {
                let Some(pid) = current_pid else { continue };
                let Ok(path) = std::str::from_utf8(value) else {
                    continue;
                };
                if current_command
                    .is_some_and(|command| !require_known || is_known_wine_process(command))
                    && (path == prefix
                        || path
                            .strip_prefix(prefix.as_ref())
                            .is_some_and(|tail| tail.starts_with('/')))
                {
                    matches.push(pid);
                }
            }
            _ => {}
        }
    }
    matches.sort_unstable();
    matches.dedup();
    matches
}

fn is_known_wine_process(command: &str) -> bool {
    let command = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    matches!(
        command.as_str(),
        "wine"
            | "wine64"
            | "wine-preloader"
            | "wine64-preloader"
            | "wineserver"
            | "winedevice.exe"
            | "winedbg"
            | "services.exe"
            | "plugplay.exe"
            | "svchost.exe"
            | "explorer.exe"
            | "rpcss.exe"
            | "conhost.exe"
            | "start.exe"
            | "steam.exe"
            | "steamcmd.exe"
            | "steamservice.exe"
            | "steamservicetmp.exe"
            | "steamwebhelper.exe"
            | "steamwebhelper_real.exe"
            | "steamwebhelper_macroton-wrapper.exe"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("macroton-{name}-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn parses_only_known_processes_with_an_open_path_inside_exact_prefix() {
        let output = b"p10\ncwinedevice.exe\nfcwd\nn/tmp/game/prefix/drive_c/windows/system32\np11\ncwinedevice.exe\nfcwd\nn/tmp/game/prefix-other\np12\ncsteamwebhelper.exe\nfcwd\nn/tmp/game/prefix\np13\nczsh\nfcwd\nn/tmp/game/prefix\np14\ncservices.exe\nfcwd\nn/tmp/game/prefix/drive_c/windows/system32\np15\ncconhost.exe\nfcwd\nn/tmp/game/prefix\n";
        assert_eq!(
            parse_prefix_owned_wine_pids(output, Path::new("/tmp/game/prefix")),
            vec![10, 12, 14, 15]
        );
    }

    #[test]
    fn detects_a_known_process_with_host_cwd_but_an_open_prefix_file() {
        let output = b"p32\ncSteam.exe\nfcwd\nn/Users/test\nftxt\nn/tmp/game/prefix/drive_c/Program Files (x86)/Steam/Steam.exe\n";
        assert_eq!(
            parse_prefix_owned_wine_pids(output, Path::new("/tmp/game/prefix")),
            vec![32]
        );
    }

    #[test]
    fn recognizes_standard_wine_and_steam_service_processes() {
        for command in [
            "services.exe",
            "plugplay.exe",
            "svchost.exe",
            "explorer.exe",
            "rpcss.exe",
            "conhost.exe",
            "start.exe",
            "steamservice.exe",
            "SteamServiceTmp.exe",
            "steamcmd.exe",
            r"C:\Program Files (x86)\Steam\Steam.exe",
            "/private/tmp/runtime/bin/wineserver",
        ] {
            assert!(is_known_wine_process(command), "{command}");
        }
    }

    #[test]
    fn filters_ps_rows_to_known_wine_and_windows_steam_candidates() {
        let output = b"  12 /private/tmp/runtime/bin/wineserver\n  44 C:\\Program Files (x86)\\Steam\\Steam.exe\n  45 /Applications/Steam.app/Contents/MacOS/steam_osx\n  46 zsh\n";
        assert_eq!(parse_known_process_pids(output), vec![12, 44]);
    }

    #[test]
    fn resource_candidates_include_arbitrary_windows_game_executables() {
        let output = b"  12 /private/tmp/runtime/bin/wineserver\n  44 C:\\Games\\AlanWake.exe\n  45 /Applications/Steam.app/Contents/MacOS/steam_osx\n  46 zsh\n";
        assert_eq!(parse_resource_candidate_pids(output), vec![12, 44]);
    }

    #[test]
    fn read_only_resource_ownership_accepts_game_exe_in_exact_prefix() {
        let output = b"p44\ncAlanWake.exe\nfcwd\nn/tmp/game/prefix/drive_c/game\n";
        assert_eq!(
            parse_prefix_owned_pids(output, Path::new("/tmp/game/prefix"), false),
            vec![44]
        );
        assert!(parse_prefix_owned_wine_pids(output, Path::new("/tmp/game/prefix")).is_empty());
    }

    #[test]
    fn ignores_malformed_lsof_fields() {
        let output = b"pnot-a-pid\ncwine\nn/tmp/game/prefix\np42\ncwinedevice.exe\nnelsewhere\n";
        assert!(parse_prefix_owned_wine_pids(output, Path::new("/tmp/game/prefix")).is_empty());
    }

    #[test]
    fn formats_all_remaining_pids_for_one_actionable_error() {
        assert_eq!(format_pids(&[7, 42, 900]), "7, 42, 900");
    }

    #[test]
    fn attributes_only_selected_well_formed_ps_rows() {
        let output = b"  12 1024 /runtime/wineserver\n  44 2048 C:\\Program Files (x86)\\Steam\\steamwebhelper.exe\n  99 9999 ignored.exe\nmalformed\n  12 nope wine\n";
        let resources = parse_ps_resources(output, &[12, 44]);
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].pid, 44);
        assert_eq!(resources[0].command, "steamwebhelper.exe");
        assert_eq!(resources[0].category, "steam-cef");
        assert_eq!(resources[1].category, "wine-service");
        assert_eq!(resources.iter().map(|item| item.rss_kib).sum::<u64>(), 3072);
    }

    #[test]
    fn classifies_preserved_steam_cef_helpers_as_steam_cef() {
        let output = b"  44 2048 C:\\Program Files (x86)\\Steam\\steamwebhelper_real.exe\n  45 1024 C:\\Program Files (x86)\\Steam\\steamwebhelper_macroton-wrapper.exe\n";
        let resources = parse_ps_resources(output, &[44, 45]);
        assert_eq!(resources[0].category, "steam-cef");
        assert_eq!(resources[1].category, "steam-cef");
    }

    #[test]
    fn does_not_promote_wine_graphics_probe_to_game_process() {
        let output = b"  12 4096 C:\\windows\\system32\\gldriverquery64.exe\n  13 4096 C:\\Program Files (x86)\\Steam\\bin\\steamsysinfo.exe\n  44 8192 C:\\Games\\Silver.exe\n";
        let resources = parse_ps_resources(output, &[12, 44]);
        assert_eq!(resources[0].command, "Silver.exe");
        assert_eq!(resources[0].category, "windows-game");
        assert_eq!(resources[1].command, "gldriverquery64.exe");
        assert_eq!(resources[1].category, "wine-helper");
        let helpers = parse_ps_resources(output, &[13]);
        assert_eq!(helpers[0].category, "wine-helper");
    }

    #[test]
    fn never_selects_an_unrecognized_host_process_inside_the_prefix() {
        let output = b"p42\nczsh\nfcwd\nn/tmp/game/prefix\n";
        assert!(parse_prefix_owned_wine_pids(output, Path::new("/tmp/game/prefix")).is_empty());
    }

    #[test]
    fn reads_runtime_id_from_identity_marker() {
        let root = temporary_root("stop-marker-test");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(PREFIX_RUNTIME_IDENTITY),
            "schema=1\nruntime_id=wine-test\n",
        )
        .unwrap();
        assert_eq!(read_prefix_runtime_id(&root).unwrap(), "wine-test");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn falls_back_to_a_legacy_steam_bootstrap_marker() {
        let root = temporary_root("stop-bootstrap-marker-test");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join(STEAM_BOOTSTRAP_MARKER),
            "{\n  \"schema\": 1,\n  \"runtime_id\": \"wine-bootstrap-test\",\n  \"runtime_kind\": \"external-executable\"\n}\n",
        )
        .unwrap();
        assert_eq!(
            read_prefix_runtime_id(&root).unwrap(),
            "wine-bootstrap-test"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stops_an_idle_registered_prefix() {
        use std::os::unix::fs::PermissionsExt;

        let root = temporary_root("stop-command-test");
        let runtime_bin = root.join("runtime/bin");
        let runtime_root = root.join("runtimes");
        let data_root = root.join("data");
        let prefix = data_root.join("games/42/prefix");
        fs::create_dir_all(&runtime_bin).unwrap();
        fs::create_dir_all(&prefix).unwrap();
        let wine = runtime_bin.join("wine");
        let wineserver = runtime_bin.join("wineserver");
        fs::write(&wine, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(&wineserver, "#!/bin/sh\nexit 1\n").unwrap();
        for executable in [&wine, &wineserver] {
            let mut permissions = fs::metadata(executable).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(executable, permissions).unwrap();
        }
        runtime::register_external("wine-stop-test", &wine, &runtime_root).unwrap();
        fs::write(
            prefix.join(PREFIX_RUNTIME_IDENTITY),
            "schema=1\nruntime_id=wine-stop-test\n",
        )
        .unwrap();
        steam_session::record(
            &prefix,
            &steam_session::Identity {
                app_id: 42,
                profile_sha256: "ab".repeat(32),
                runtime_id: "wine-stop-test".to_owned(),
                runtime_fingerprint_sha256: "cd".repeat(32),
                steam_client_mode: "standard".to_owned(),
                renderer: "wine-default".to_owned(),
                graphics_api: "opengl".to_owned(),
            },
        )
        .unwrap();

        let result = stop(42, &data_root, &runtime_root).unwrap();
        assert_eq!(result.runtime_id, "wine-stop-test");
        assert!(result.terminated_orphans.is_empty());
        assert!(result.killed_orphans.is_empty());
        assert!(!prefix.join(steam_session::MARKER).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn explicit_runtime_id_stops_a_legacy_prefix_without_identity_marker() {
        use std::os::unix::fs::PermissionsExt;

        let root = temporary_root("stop-explicit-runtime-test");
        let runtime_bin = root.join("runtime/bin");
        let runtime_root = root.join("runtimes");
        let data_root = root.join("data");
        let prefix = data_root.join("games/42/prefix");
        fs::create_dir_all(&runtime_bin).unwrap();
        fs::create_dir_all(&prefix).unwrap();
        let wine = runtime_bin.join("wine");
        let wineserver = runtime_bin.join("wineserver");
        fs::write(&wine, "#!/bin/sh\nexit 0\n").unwrap();
        fs::write(&wineserver, "#!/bin/sh\nexit 1\n").unwrap();
        for executable in [&wine, &wineserver] {
            let mut permissions = fs::metadata(executable).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(executable, permissions).unwrap();
        }
        runtime::register_external("wine-explicit-test", &wine, &runtime_root).unwrap();

        let result =
            stop_with_runtime_id(42, &data_root, &runtime_root, Some("wine-explicit-test"))
                .unwrap();
        assert_eq!(result.runtime_id, "wine-explicit-test");
        fs::remove_dir_all(root).unwrap();
    }
}
