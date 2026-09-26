use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Unlocked,
    Locked,
    Unknown,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unlocked => "unlocked",
            Self::Locked => "locked",
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

#[cfg(target_os = "macos")]
pub fn inspect() -> Result<State, Error> {
    let output = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-n", "Root", "-d1"])
        .output()
        .map_err(|error| Error {
            message: format!("cannot inspect macOS console lock state: {error}"),
        })?;
    if !output.status.success() {
        return Err(Error {
            message: format!(
                "macOS console lock-state query exited with {}",
                output.status
            ),
        });
    }
    Ok(parse_ioreg(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(not(target_os = "macos"))]
pub fn inspect() -> Result<State, Error> {
    Ok(State::Unknown)
}

fn parse_ioreg(output: &str) -> State {
    output
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let value = line.strip_prefix("\"IOConsoleLocked\" = ")?;
            match value.trim() {
                "Yes" => Some(State::Locked),
                "No" => Some(State::Unlocked),
                _ => Some(State::Unknown),
            }
        })
        .unwrap_or(State::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_locked_and_unlocked_console_properties() {
        assert_eq!(
            parse_ioreg("{\n  \"IOConsoleLocked\" = Yes\n}"),
            State::Locked
        );
        assert_eq!(
            parse_ioreg("{\n  \"IOConsoleLocked\" = No\n}"),
            State::Unlocked
        );
    }

    #[test]
    fn missing_or_ambiguous_lock_state_fails_to_unknown() {
        assert_eq!(parse_ioreg("{}"), State::Unknown);
        assert_eq!(parse_ioreg("\"IOConsoleLocked\" = Maybe"), State::Unknown);
    }
}
