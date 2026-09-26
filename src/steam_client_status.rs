use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    NoEvidence,
    Downloading,
    Applying,
    Current,
    Failed,
    Unknown,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoEvidence => "no-evidence",
            Self::Downloading => "downloading",
            Self::Applying => "applying",
            Self::Current => "current",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
        }
    }

    pub fn update_active(self) -> bool {
        matches!(self, Self::Downloading | Self::Applying)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Status {
    pub app_id: u32,
    pub state: State,
    pub downloaded_kib: Option<u64>,
    pub total_kib: Option<u64>,
    pub recent_rate_kib_per_minute: Option<u64>,
    pub estimated_seconds_remaining: Option<u64>,
    pub log_modified_unix: Option<u64>,
    pub log_path: PathBuf,
}

impl Status {
    pub fn progress_age_seconds(&self, now_unix: u64) -> Option<u64> {
        self.state.update_active().then(|| {
            self.log_modified_unix
                .map(|modified| now_unix.saturating_sub(modified))
        })?
    }

    pub fn update_progress_is_fresh(&self, now_unix: u64, max_age_seconds: u64) -> bool {
        self.state.update_active()
            && self
                .progress_age_seconds(now_unix)
                .is_some_and(|age| age <= max_age_seconds)
    }
}

#[derive(Debug)]
pub struct Error {
    message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub fn inspect(app_id: u32, data_root: &Path) -> Result<Status, Error> {
    let steam_root = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix/drive_c/Program Files (x86)/Steam");
    inspect_steam_root(app_id, &steam_root)
}

pub fn inspect_steam_root(app_id: u32, steam_root: &Path) -> Result<Status, Error> {
    let log_path = steam_root.join("logs/bootstrap_log.txt");
    let contents = match fs::read_to_string(&log_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Status {
                app_id,
                state: State::NoEvidence,
                downloaded_kib: None,
                total_kib: None,
                recent_rate_kib_per_minute: None,
                estimated_seconds_remaining: None,
                log_modified_unix: None,
                log_path,
            });
        }
        Err(error) => {
            return Err(Error {
                message: format!("cannot read Steam client update log: {error}"),
            });
        }
    };
    let modified = fs::metadata(&log_path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs());
    // bootstrap_log.txt is append-only across Steam launches. Progress from a
    // previous launch must not be presented as the current update attempt.
    let attempt = latest_attempt(&contents);
    let (state, downloaded_kib, total_kib) = parse(attempt);
    let recent_rate_kib_per_minute = recent_download_rate_kib_per_minute(attempt);
    let estimated_seconds_remaining = downloaded_kib
        .zip(total_kib)
        .zip(recent_rate_kib_per_minute)
        .and_then(|((downloaded, total), rate)| {
            (rate > 0).then_some(total.saturating_sub(downloaded).saturating_mul(60) / rate)
        });
    Ok(Status {
        app_id,
        state,
        downloaded_kib,
        total_kib,
        recent_rate_kib_per_minute,
        estimated_seconds_remaining,
        log_modified_unix: modified,
        log_path,
    })
}

fn latest_attempt(contents: &str) -> &str {
    contents
        .rfind("Startup - updater built")
        .map(|start| &contents[start..])
        .unwrap_or(contents)
}

fn recent_download_rate_kib_per_minute(contents: &str) -> Option<u64> {
    let samples = contents
        .lines()
        .filter_map(parse_timed_download_sample)
        .collect::<Vec<_>>();
    let &(last_date, last_second, last_current, last_total) = samples.last()?;
    let mut baseline = None;
    for &(date, second, current, total) in samples.iter().rev().skip(1) {
        if date != last_date
            || total != last_total
            || current > last_current
            || second > last_second
        {
            break;
        }
        let elapsed = last_second - second;
        if elapsed > 120 {
            break;
        }
        if elapsed >= 15 && current < last_current {
            baseline = Some((elapsed, current));
        }
    }
    let (elapsed, current) = baseline?;
    let transferred = last_current - current;
    Some(transferred.saturating_mul(60) / u64::from(elapsed))
}

fn parse_timed_download_sample(line: &str) -> Option<(&str, u32, u64, u64)> {
    let timestamp = line.strip_prefix('[')?.get(..19)?;
    let date = timestamp.get(..10)?;
    let time = timestamp.get(11..19)?;
    let mut parts = time.split(':');
    let hour = parts.next()?.parse::<u32>().ok()?;
    let minute = parts.next()?.parse::<u32>().ok()?;
    let second = parts.next()?.parse::<u32>().ok()?;
    if hour >= 24 || minute >= 60 || second >= 60 {
        return None;
    }
    let (current, total) = parse_download_progress(line)?;
    Some((date, hour * 3600 + minute * 60 + second, current, total))
}

fn parse(contents: &str) -> (State, Option<u64>, Option<u64>) {
    let mut state = State::Unknown;
    let mut downloaded = None;
    let mut total = None;
    for line in contents.lines() {
        if let Some((current, target)) = parse_download_progress(line) {
            state = State::Downloading;
            downloaded = Some(current);
            total = Some(target);
        } else if line.contains("Verifying installation")
            || line.contains("Extracting package")
            || line.contains("Installing update")
            || line.contains("Applying update")
        {
            state = State::Applying;
        } else if line.contains("Update complete")
            || line.contains("up-to-date")
            || line.contains("Nothing to do")
            || line.contains("Verification complete")
            || line.contains("Suppressing Steam update")
        {
            state = State::Current;
        } else if line.contains("Failed to download")
            || line.contains("Download failed")
            || line.contains("Update failed")
        {
            state = State::Failed;
        }
    }
    (state, downloaded, total)
}

fn parse_download_progress(line: &str) -> Option<(u64, u64)> {
    let value = line.split_once("Downloading update (")?.1;
    let value = value.split_once(" KB)")?.0;
    let (current, total) = value.split_once(" of ")?;
    let parse = |value: &str| value.replace(',', "").parse::<u64>().ok();
    let current = parse(current)?;
    let total = parse(total)?;
    (total > 0 && current <= total).then_some((current, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_latest_downloading_progress_with_grouped_numbers() {
        let input = "[time] Update complete\n[time] Downloading update (9,582 of 336,229 KB)...\n";
        assert_eq!(
            parse(input),
            (State::Downloading, Some(9_582), Some(336_229))
        );
    }

    #[test]
    fn later_apply_or_complete_event_supersedes_download() {
        let downloading = "Downloading update (10 of 20 KB)...\nExtracting package...\n";
        assert_eq!(parse(downloading).0, State::Applying);
        let complete = format!("{downloading}Update complete, launching Steam...\n");
        assert_eq!(parse(&complete).0, State::Current);
        assert_eq!(
            parse("Verifying installation...\nVerification complete\n").0,
            State::Current
        );
    }

    #[test]
    fn rejects_malformed_or_impossible_progress() {
        assert_eq!(
            parse_download_progress("Downloading update (30 of 20 KB)..."),
            None
        );
        assert_eq!(
            parse_download_progress("Downloading update (x of 20 KB)..."),
            None
        );
    }

    #[test]
    fn update_freshness_is_bounded_and_clock_skew_safe() {
        let status = Status {
            app_id: 42,
            state: State::Downloading,
            downloaded_kib: Some(1),
            total_kib: Some(2),
            recent_rate_kib_per_minute: Some(1),
            estimated_seconds_remaining: Some(60),
            log_modified_unix: Some(100),
            log_path: PathBuf::from("bootstrap_log.txt"),
        };
        assert!(status.update_progress_is_fresh(100, 300));
        assert!(status.update_progress_is_fresh(400, 300));
        assert!(!status.update_progress_is_fresh(401, 300));
        assert!(status.update_progress_is_fresh(99, 300));
    }

    #[test]
    fn estimates_recent_rate_without_crossing_attempt_or_day_boundaries() {
        let input = "[2026-08-13 19:48:00] Downloading update (40,000 of 336,229 KB)...\n\
[2026-08-13 19:49:00] Downloading update (43,000 of 336,229 KB)...\n\
[2026-08-13 19:49:30] Downloading update (44,500 of 336,229 KB)...\n";
        assert_eq!(recent_download_rate_kib_per_minute(input), Some(3_000));
        let restarted =
            format!("{input}[2026-08-13 19:49:40] Downloading update (10 of 336,229 KB)...\n");
        assert_eq!(recent_download_rate_kib_per_minute(&restarted), None);
        let next_day =
            format!("{input}[2026-08-14 00:00:10] Downloading update (45,000 of 336,229 KB)...\n");
        assert_eq!(recent_download_rate_kib_per_minute(&next_day), None);
    }

    #[test]
    fn latest_suppressed_launch_does_not_inherit_prior_download_progress() {
        let log = "[2026-09-26 16:00:00] Startup - updater built Sep 2 2026\n\
[2026-09-26 16:00:30] Downloading update (10 of 20 KB)...\n\
[2026-09-26 16:01:00] Downloading update (15 of 20 KB)...\n\
[2026-09-26 17:00:00] Startup - updater built Sep 2 2026\n\
[2026-09-26 17:00:01] Suppressing Steam update\n";
        let attempt = latest_attempt(log);
        assert_eq!(parse(attempt), (State::Current, None, None));
        assert_eq!(recent_download_rate_kib_per_minute(attempt), None);
    }

    #[test]
    fn no_active_progress_has_no_progress_age() {
        let status = Status {
            app_id: 42,
            state: State::Current,
            downloaded_kib: None,
            total_kib: None,
            recent_rate_kib_per_minute: None,
            estimated_seconds_remaining: None,
            log_modified_unix: Some(100),
            log_path: PathBuf::from("bootstrap_log.txt"),
        };
        assert_eq!(status.progress_age_seconds(400), None);
        assert!(!status.update_progress_is_fresh(400, 300));
    }
}
