use crate::keyvalues::{self, Value};
use crate::steam;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    NoScript,
    SignatureMissing,
    Pending,
    Installing,
    PassedToGame,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoScript => "no-script",
            Self::SignatureMissing => "signature-missing",
            Self::Pending => "pending",
            Self::Installing => "installing",
            Self::PassedToGame => "passed-to-game",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub id: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub app_id: u32,
    pub state: State,
    pub installscript: Option<PathBuf>,
    pub signature_present: bool,
    pub requirements: Vec<Requirement>,
    pub latest_event: Option<String>,
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

pub fn inspect(app_id: u32, data_root: &Path) -> Result<Status, Error> {
    let game_root = data_root.join("games").join(app_id.to_string());
    let acquisition = game_root.join("steamcmd-payload");
    let steam_root = game_root.join("prefix/drive_c/Program Files (x86)/Steam");
    let install_root = if acquisition.is_dir() {
        Some(acquisition)
    } else if steam_root.join("steamapps").is_dir() {
        steam::scan(&steam_root)
            .map_err(|error| Error {
                message: error.to_string(),
            })?
            .into_iter()
            .find(|game| game.app_id == app_id)
            .map(|game| game.install_dir)
    } else {
        None
    };
    let installscript = install_root.as_deref().and_then(locate_installscript);
    let Some(installscript) = installscript else {
        return Ok(Status {
            app_id,
            state: State::NoScript,
            installscript: None,
            signature_present: false,
            requirements: Vec::new(),
            latest_event: None,
        });
    };
    let contents = fs::read_to_string(&installscript).map_err(|error| Error {
        message: format!("cannot read {}: {error}", installscript.display()),
    })?;
    let document = keyvalues::parse(&contents).map_err(|error| Error {
        message: format!("cannot parse {}: {error}", installscript.display()),
    })?;
    let requirements = parse_requirements(&document);
    let signature_present = document
        .get("kvsignatures")
        .and_then(|value| value.string("installscript"))
        .is_some_and(|value| !value.is_empty());
    let (runtime_state, latest_event) =
        prerequisite_runtime_state(app_id, &steam_root.join("logs/console_log.txt"))?;
    let state = if !signature_present {
        State::SignatureMissing
    } else {
        runtime_state
    };
    Ok(Status {
        app_id,
        state,
        installscript: Some(installscript),
        signature_present,
        requirements,
        latest_event,
    })
}

fn locate_installscript(root: &Path) -> Option<PathBuf> {
    let direct = root.join("installscript.vdf");
    if direct.is_file() {
        return Some(direct);
    }
    let mut directories = vec![(root.to_path_buf(), 0_usize)];
    let mut visited = 0_usize;
    while let Some((directory, depth)) = directories.pop() {
        if depth >= 3 {
            continue;
        }
        for entry in fs::read_dir(directory).ok()?.flatten() {
            visited += 1;
            if visited > 1000 {
                return None;
            }
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("installscript.vdf"))
            {
                return Some(path);
            }
            if file_type.is_dir() {
                directories.push((path, depth + 1));
            }
        }
    }
    None
}

fn parse_requirements(document: &Value) -> Vec<Requirement> {
    let mut requirements = document
        .get("installscript")
        .and_then(|value| value.get("Run Process"))
        .and_then(Value::entries)
        .into_iter()
        .flatten()
        .map(|(id, value)| Requirement {
            id: id.to_owned(),
            description: value.string("description").map(str::to_owned),
        })
        .collect::<Vec<_>>();
    requirements.sort_by(|left, right| left.id.cmp(&right.id));
    requirements
}

fn prerequisite_runtime_state(
    app_id: u32,
    console_log: &Path,
) -> Result<(State, Option<String>), Error> {
    let contents = match fs::read_to_string(console_log) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((State::Pending, None));
        }
        Err(error) => {
            return Err(Error {
                message: format!("cannot read {}: {error}", console_log.display()),
            });
        }
    };
    let launch = format!("-applaunch {app_id}");
    let app_marker = format!("AppID {app_id}");
    let mut state = State::Pending;
    let mut latest_event = None;
    for line in contents.lines() {
        let lower = line.to_ascii_lowercase();
        if line.contains(&launch) {
            state = State::Pending;
            latest_event = Some(line.to_owned());
        } else if line.contains(&app_marker) && lower.contains("runninginstallscript") {
            state = State::Installing;
            latest_event = Some(line.to_owned());
        } else if line.contains(&app_marker) && lower.contains("game process added") {
            state = State::PassedToGame;
            latest_event = Some(line.to_owned());
        }
    }
    Ok((state, latest_event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_requirements_pass_only_after_real_game_process() {
        let root = std::env::temp_dir().join(format!(
            "macroton-prerequisite-status-{}",
            std::process::id()
        ));
        let game = root.join("games/42/steamcmd-payload");
        let logs = root.join("games/42/prefix/drive_c/Program Files (x86)/Steam/logs");
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            game.join("installscript.vdf"),
            r#""installscript" { "Run Process" { "directx" { "description" "Microsoft DirectX" } "vcredist" { "description" "Visual C++" } } } "kvsignatures" { "installscript" "signed" }"#,
        )
        .unwrap();
        fs::write(
            logs.join("console_log.txt"),
            "ExecCommandLine: Steam.exe -applaunch 42\nGameAction [AppID 42] changed task to RunningInstallScript\nGame process added : AppID 42 game.exe, ProcID 9\n",
        )
        .unwrap();
        let status = inspect(42, &root).unwrap();
        assert_eq!(status.state, State::PassedToGame);
        assert!(status.signature_present);
        assert_eq!(status.requirements.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_signature_fails_closed() {
        let root = std::env::temp_dir().join(format!(
            "macroton-prerequisite-unsigned-{}",
            std::process::id()
        ));
        let game = root.join("games/42/steamcmd-payload");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("installscript.vdf"),
            r#""installscript" { "Run Process" { "directx" { } } }"#,
        )
        .unwrap();
        let status = inspect(42, &root).unwrap();
        assert_eq!(status.state, State::SignatureMissing);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bootstrapped_client_without_steamapps_is_no_script() {
        let root = std::env::temp_dir().join(format!(
            "macroton-prerequisite-empty-client-{}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("games/42/prefix/drive_c/Program Files (x86)/Steam")).unwrap();
        let status = inspect(42, &root).unwrap();
        assert_eq!(status.state, State::NoScript);
        assert!(status.installscript.is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
