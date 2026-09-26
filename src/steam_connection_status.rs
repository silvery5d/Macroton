use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    NoObservation,
    Connecting,
    Connected,
    Disconnected,
}

impl TransportState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoObservation => "no-observation",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionStatus {
    pub app_id: u32,
    pub steam_root: PathBuf,
    pub log_path: PathBuf,
    pub state: TransportState,
    pub latest_timestamp: Option<String>,
    pub synthetic_fake_ip: Option<bool>,
    pub authenticated_context: bool,
    /// Evidence that Steam's rendered UI reached its logged-in state. This is
    /// intentionally separate from transport connectivity and is not treated
    /// as proof that an AppID install request was accepted.
    pub ui_login_observed: bool,
    /// Steam UI reached the login page but its QR/auth-session polling hit a
    /// transport error. This is diagnostic only and never implies auth.
    pub auth_poll_transport_error: bool,
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

pub fn inspect(app_id: u32, data_root: &Path) -> Result<ConnectionStatus, Error> {
    let steam_root = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix/drive_c/Program Files (x86)/Steam");
    inspect_steam_root(app_id, &steam_root)
}

pub fn inspect_steam_root(app_id: u32, steam_root: &Path) -> Result<ConnectionStatus, Error> {
    let log_path = steam_root.join("logs/connection_log.txt");
    let contents = fs::read_to_string(&log_path).map_err(|error| Error {
        message: format!(
            "cannot read Steam connection log {}: {error}",
            log_path.display()
        ),
    })?;
    let observation = parse_connection_log(&contents);
    let webhelper =
        fs::read_to_string(steam_root.join("logs/webhelper_js.txt")).unwrap_or_default();
    let ui_login_observed = parse_webhelper_login_state(&webhelper);
    let auth_poll_transport_error = parse_auth_poll_transport_error(&webhelper);
    Ok(ConnectionStatus {
        app_id,
        steam_root: steam_root.to_path_buf(),
        log_path,
        state: observation.state,
        latest_timestamp: observation.latest_timestamp,
        synthetic_fake_ip: observation.synthetic_fake_ip,
        authenticated_context: observation.authenticated_context,
        ui_login_observed,
        auth_poll_transport_error,
    })
}

fn parse_webhelper_login_state(contents: &str) -> bool {
    let mut latest_state = None;
    for line in contents.lines() {
        let Some(state) = line.split("OnLoginStateChange").nth(1) else {
            continue;
        };
        // Steam UI state 5 is the post-login state. Keep this parser narrow:
        // merely seeing a username or an intermediate state is insufficient.
        latest_state = state.split_whitespace().nth(1).map(str::to_owned);
    }
    latest_state.as_deref() == Some("5")
}

fn parse_auth_poll_transport_error(contents: &str) -> bool {
    contents.lines().any(|line| {
        line.contains("Login: Failed to poll auth session") && line.contains("Transport Error")
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Observation {
    state: TransportState,
    latest_timestamp: Option<String>,
    synthetic_fake_ip: Option<bool>,
    authenticated_context: bool,
}

fn parse_connection_log(contents: &str) -> Observation {
    let mut observation = Observation {
        state: TransportState::NoObservation,
        latest_timestamp: None,
        synthetic_fake_ip: None,
        authenticated_context: false,
    };
    for line in contents.lines() {
        let next_state = if line.contains("OnDisconnected")
            || line.contains("ConnectionDisconnected")
            || line.contains("] [Disconnected,")
        {
            Some(TransportState::Disconnected)
        } else if line.contains("ConnectionCompleted()") || line.contains("] [Connected,") {
            Some(TransportState::Connected)
        } else if line.contains("] [Connecting,") {
            Some(TransportState::Connecting)
        } else {
            None
        };
        if let Some(state) = next_state {
            observation.state = state;
            observation.latest_timestamp = timestamp(line).map(str::to_owned);
            observation.authenticated_context = authenticated_context(line);
        }
        if line.contains("ConnectionCompleted()") {
            observation.synthetic_fake_ip = completed_endpoint(line).map(is_benchmark_fake_ip);
        }
    }
    observation
}

fn authenticated_context(line: &str) -> bool {
    let Some(account) = line.split("[U:1:").nth(1) else {
        return false;
    };
    account
        .split_once(']')
        .and_then(|(value, _)| value.parse::<u64>().ok())
        .is_some_and(|value| value != 0)
}

fn timestamp(line: &str) -> Option<&str> {
    line.strip_prefix('[')?
        .split_once(']')
        .map(|(value, _)| value)
}

fn completed_endpoint(line: &str) -> Option<&str> {
    let endpoint = line.split_once("ConnectionCompleted() (")?.1;
    let host_and_port = endpoint
        .split_once(", ")
        .map_or(endpoint, |(value, _)| value);
    if host_and_port.starts_with('[') {
        return None;
    }
    host_and_port.rsplit_once(':').map(|(host, _)| host)
}

fn is_benchmark_fake_ip(host: &str) -> bool {
    host.strip_prefix("198.")
        .and_then(|tail| tail.split('.').next())
        .and_then(|octet| octet.parse::<u8>().ok())
        .is_some_and(|octet| matches!(octet, 18 | 19))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_connected_fake_ip_without_preserving_account_fields() {
        let log = concat!(
            "[2026-08-12 10:00:00] [Connecting, 0, 1] [U:1:123] start\n",
            "[2026-08-12 10:00:01] [Connecting, 0, 1] [U:1:123] ",
            "ConnectionCompleted() (198.18.3.217:27021, WebSocket) local address (198.18.0.1:1)\n",
            "[2026-08-12 10:00:02] [Connected, 0, 1] [U:1:123] stable\n"
        );
        let status = parse_connection_log(log);
        assert_eq!(status.state, TransportState::Connected);
        assert_eq!(status.synthetic_fake_ip, Some(true));
        assert!(status.authenticated_context);
        assert_eq!(
            status.latest_timestamp.as_deref(),
            Some("2026-08-12 10:00:02")
        );
    }

    #[test]
    fn recognizes_only_the_post_login_ui_state() {
        assert!(!parse_webhelper_login_state(
            "[x] Login: OnLoginStateChange user 4 1 0 0\n"
        ));
        assert!(parse_webhelper_login_state(
            "[x] Login: OnLoginStateChange user 5 1 0 0\n"
        ));
    }

    #[test]
    fn recognizes_login_poll_transport_errors_without_inferring_auth() {
        assert!(parse_auth_poll_transport_error(
            "[x] Login: Failed to poll auth session. Result 2. Transport Error: 2\n"
        ));
        assert!(!parse_auth_poll_transport_error(
            "[x] Login: Failed to poll auth session. Result 2. Pending\n"
        ));
    }

    #[test]
    fn a_later_login_state_supersedes_an_earlier_success() {
        let log = concat!(
            "[x] Login: OnLoginStateChange user 5 1 0 0\n",
            "[x] Login: OnLoginStateChange user 0 1 0 0\n"
        );
        assert!(!parse_webhelper_login_state(log));
    }

    #[test]
    fn a_later_disconnect_supersedes_a_connection() {
        let log = concat!(
            "[2026-08-12 10:00:01] [Connecting, 0, 1] ConnectionCompleted() (1.2.3.4:443, WebSocket)\n",
            "[2026-08-12 10:00:03] [Disconnected, 0, 1] OnDisconnected reason\n"
        );
        let status = parse_connection_log(log);
        assert_eq!(status.state, TransportState::Disconnected);
        assert_eq!(status.synthetic_fake_ip, Some(false));
        assert!(!status.authenticated_context);
        assert_eq!(
            status.latest_timestamp.as_deref(),
            Some("2026-08-12 10:00:03")
        );
    }

    #[test]
    fn ignores_ping_failures_as_transport_state() {
        let status = parse_connection_log(
            "[2026-08-12 10:00:00] PingWebSocketCM() failed talking to cm (Invalid)\n",
        );
        assert_eq!(status.state, TransportState::NoObservation);
        assert_eq!(status.synthetic_fake_ip, None);
        assert!(!status.authenticated_context);
    }

    #[test]
    fn latest_connection_state_controls_redacted_auth_context() {
        let status = parse_connection_log(concat!(
            "[2026-08-12 10:00:01] [Connected, 0, 1] [U:1:123] live\n",
            "[2026-08-12 10:00:02] [Disconnected, 0, 1] [U:1:0] bye\n"
        ));
        assert_eq!(status.state, TransportState::Disconnected);
        assert!(!status.authenticated_context);
    }
}
