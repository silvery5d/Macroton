use macroton::binary::{Architecture, BinaryKind, PeArchitecture};
use macroton::cloud_roundtrip;
use macroton::cloud_status;
use macroton::execute;
use macroton::launch_status;
use macroton::macos_session;
use macroton::plan;
use macroton::prerequisite_status;
use macroton::profile::{CompatibilityProfile, SteamCloudPolicy, WindowsPayloadRoot};
use macroton::runtime::{self, InstallOutcome, RuntimeManifest};
use macroton::sha256;
use macroton::steam;
use macroton::steam_appinfo;
use macroton::steam_bootstrap;
use macroton::steam_client_status;
use macroton::steam_connection_status;
use macroton::steam_install_status;
use macroton::steam_library_status;
use macroton::steam_prefix_status;
use macroton::steam_session;
use macroton::steamcmd;
use macroton::stop;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

struct IdleSleepGuard {
    child: Option<Child>,
}

impl IdleSleepGuard {
    fn acquire(operation: &str) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let child = Command::new("/usr/bin/caffeinate")
                .args(["-i", "-w", &std::process::id().to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| {
                    format!("cannot inhibit idle system sleep during {operation}: {error}")
                })?;
            Ok(Self { child: Some(child) })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = operation;
            Ok(Self { child: None })
        }
    }
}

impl Drop for IdleSleepGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        print_help();
        return Ok(());
    };
    match command {
        "scan" => scan_command(&args[1..]),
        "inspect" => inspect_command(&args[1..]),
        "inspect-pe" => inspect_pe_command(&args[1..]),
        "profile" => profile_command(&args[1..]),
        "evidence" => evidence_command(&args[1..]),
        "plan" => plan_command(&args[1..]),
        "launch" => launch_command(&args[1..]),
        "play" => play_command(&args[1..]),
        "shortcut" => shortcut_command(&args[1..]),
        "stop" => stop_command(&args[1..]),
        "resource" => resource_command(&args[1..]),
        "runtime" => runtime_command(&args[1..]),
        "steam" => steam_command(&args[1..]),
        "steamcmd" => steamcmd_command(&args[1..]),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => Err(format!("unknown command {other:?}; run `macroton help`")),
    }
}

struct EvidenceSnapshot {
    captured_unix: u64,
    app_id: u32,
    profile_sha256: String,
    runtime_id: String,
    runtime_fingerprint_sha256: Option<String>,
    profile_runtime_match: bool,
    profile_session_match: bool,
    steam_session_sha256: Option<String>,
    renderer: String,
    graphics_api: String,
    steam_client_mode: String,
    phase: String,
    console_state: String,
    prefix_readiness: String,
    bootstrap_valid: bool,
    stored_login_records: usize,
    launch_state: String,
    tracked_game_pid: Option<u32>,
    tracked_game_pid_live: bool,
    exit_code: Option<i32>,
    total_rss_kib: u64,
    process_count: usize,
    steam_cloud_policy: String,
    cloud_declared_files: usize,
    cloud_state: String,
    cloud_matching_files: usize,
}

impl EvidenceSnapshot {
    fn to_json(&self) -> String {
        let tracked_game_pid = self
            .tracked_game_pid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_owned());
        let exit_code = self
            .exit_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_owned());
        let runtime_fingerprint_sha256 = self
            .runtime_fingerprint_sha256
            .as_ref()
            .map(|value| format!("\"{}\"", json_escape(value)))
            .unwrap_or_else(|| "null".to_owned());
        let steam_session_sha256 = self
            .steam_session_sha256
            .as_ref()
            .map(|value| format!("\"{}\"", json_escape(value)))
            .unwrap_or_else(|| "null".to_owned());
        format!(
            concat!(
                "{{\n",
                "  \"schema_version\": 1,\n",
                "  \"captured_unix\": {},\n",
                "  \"appid\": {},\n",
                "  \"profile_sha256\": \"{}\",\n",
                "  \"runtime_id\": \"{}\",\n",
                "  \"runtime_fingerprint_sha256\": {},\n",
                "  \"profile_runtime_match\": {},\n",
                "  \"profile_session_match\": {},\n",
                "  \"steam_session_sha256\": {},\n",
                "  \"renderer\": \"{}\",\n",
                "  \"graphics_api\": \"{}\",\n",
                "  \"steam_client_mode\": \"{}\",\n",
                "  \"phase\": \"{}\",\n",
                "  \"console_state\": \"{}\",\n",
                "  \"prefix_readiness\": \"{}\",\n",
                "  \"bootstrap_valid\": {},\n",
                "  \"stored_login_records\": {},\n",
                "  \"launch_state\": \"{}\",\n",
                "  \"tracked_game_pid\": {},\n",
                "  \"tracked_game_pid_live\": {},\n",
                "  \"exit_code\": {},\n",
                "  \"total_rss_kib\": {},\n",
                "  \"process_count\": {},\n",
                "  \"steam_cloud_policy\": \"{}\",\n",
                "  \"cloud_declared_files\": {},\n",
                "  \"cloud_state\": \"{}\",\n",
                "  \"cloud_matching_files\": {},\n",
                "  \"automated_scope\": [\"profile\",\"session\",\"prefix\",\"steam-lifecycle\",\"exact-prefix-resources\",\"cloud-cache-content\"],\n",
                "  \"not_inferred\": [\"presentation\",\"input\",\"audible-quality\",\"save-reload\",\"fresh-cloud-roundtrip\",\"stability\",\"normal-exit\"]\n",
                "}}\n"
            ),
            self.captured_unix,
            self.app_id,
            json_escape(&self.profile_sha256),
            json_escape(&self.runtime_id),
            runtime_fingerprint_sha256,
            self.profile_runtime_match,
            self.profile_session_match,
            steam_session_sha256,
            json_escape(&self.renderer),
            json_escape(&self.graphics_api),
            json_escape(&self.steam_client_mode),
            json_escape(&self.phase),
            json_escape(&self.console_state),
            json_escape(&self.prefix_readiness),
            self.bootstrap_valid,
            self.stored_login_records,
            json_escape(&self.launch_state),
            tracked_game_pid,
            self.tracked_game_pid_live,
            exit_code,
            self.total_rss_kib,
            self.process_count,
            json_escape(&self.steam_cloud_policy),
            self.cloud_declared_files,
            json_escape(&self.cloud_state),
            self.cloud_matching_files,
        )
    }
}

fn evidence_command(args: &[String]) -> Result<(), String> {
    if args.first().map(String::as_str) == Some("record-failure") {
        return evidence_record_failure_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("failure-status") {
        return evidence_failure_status_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("record-review") {
        return evidence_record_review_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("acceptance-status") {
        return evidence_acceptance_status_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("bundle-status") {
        return evidence_bundle_status_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("matrix-status") {
        return evidence_matrix_status_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("bundle") {
        return evidence_bundle_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("verify-audio") {
        return evidence_verify_audio_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("verify-focus-stream") {
        return evidence_verify_focus_stream_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("verify-motion-rate") {
        return evidence_verify_motion_rate_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("verify-gpu-capture") {
        return evidence_verify_gpu_capture_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("verify-interaction") {
        return evidence_verify_interaction_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("verify-independence") {
        return evidence_verify_independence_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("watch-exit") {
        return evidence_watch_exit_command(&args[1..]);
    }
    if args.first().map(String::as_str) != Some("snapshot") {
        return Err(
            "usage: macroton evidence <snapshot|bundle|bundle-status|matrix-status|record-failure|failure-status|record-review|acceptance-status|verify-audio|verify-focus-stream|verify-motion-rate|verify-gpu-capture|verify-interaction|verify-independence|watch-exit> APPID [options]"
                .to_owned(),
        );
    }
    let app_id: u32 = args
        .get(1)
        .ok_or_else(|| "evidence snapshot requires APPID".to_owned())?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut output = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 2;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown evidence snapshot option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !output.is_absolute() {
        return Err("--output must be an absolute path".to_owned());
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let session = macos_session::inspect().map_err(|error| error.to_string())?;
    let prefix =
        steam_prefix_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    let installed_runtime = runtime::resolve_installed(&runtime_root, &runtime_id).ok();
    let runtime_fingerprint_sha256 = installed_runtime
        .as_ref()
        .map(|installed| installed.fingerprint_sha256.clone());
    let profile_runtime_match = installed_runtime.as_ref().is_some_and(|installed| {
        steam_prefix_status::validate_expected_runtime(
            app_id,
            &prefix.prefix,
            &runtime_id,
            &installed.fingerprint_sha256,
        )
        .is_ok()
    });
    let profile_session_match = installed_runtime.as_ref().is_some_and(|installed| {
        steam_session::validate(
            &prefix.prefix,
            &steam_session::Identity {
                app_id,
                profile_sha256: profile.fingerprint(),
                runtime_id: runtime_id.clone(),
                runtime_fingerprint_sha256: installed.fingerprint_sha256.clone(),
                steam_client_mode: profile.steam_client_mode.as_str().to_owned(),
                renderer: profile.renderer.clone(),
                graphics_api: profile.graphics_api.as_str().to_owned(),
            },
        )
        .is_ok()
    });
    let steam_session_sha256 = if profile_session_match {
        Some(sha256::to_hex(
            &sha256::digest_file(&prefix.prefix.join(steam_session::MARKER))
                .map_err(|error| format!("cannot hash Steam session provenance: {error}"))?,
        ))
    } else {
        None
    };
    let launch = launch_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let resources =
        stop::inspect_resources(app_id, &data_root).map_err(|error| error.to_string())?;
    let tracked_game_pid_live = launch
        .process_id
        .is_some_and(|pid| resources.pids.contains(&pid));
    let primary_steam_client_live = resources
        .processes
        .iter()
        .any(is_primary_steam_client_process);
    let client_update =
        steam_client_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let update_fresh = client_update.update_progress_is_fresh(current_unix_seconds()?, 300);
    let phase = lifecycle_phase(
        session,
        prefix.readiness,
        launch.state,
        tracked_game_pid_live,
        primary_steam_client_live,
        update_fresh,
        client_update.state.update_active() && !update_fresh,
    );
    let (cloud_state, cloud_matching_files) = if profile.cloud_files.is_empty() {
        ("not-declared".to_owned(), 0)
    } else {
        let mut cloud =
            cloud_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
        cloud_status::compare_profile_files(&mut cloud, &profile)
            .map_err(|error| error.to_string())?;
        (
            cloud.state.as_str().to_owned(),
            cloud
                .comparisons
                .iter()
                .filter(|comparison| comparison.state == cloud_status::ComparisonState::Match)
                .count(),
        )
    };
    let snapshot = EvidenceSnapshot {
        captured_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
            .as_secs(),
        app_id,
        profile_sha256: profile.fingerprint(),
        runtime_id,
        runtime_fingerprint_sha256,
        profile_runtime_match,
        profile_session_match,
        steam_session_sha256,
        renderer: profile.renderer.clone(),
        graphics_api: profile.graphics_api.as_str().to_owned(),
        steam_client_mode: profile.steam_client_mode.as_str().to_owned(),
        phase: phase.to_owned(),
        console_state: session.as_str().to_owned(),
        prefix_readiness: prefix.readiness.as_str().to_owned(),
        bootstrap_valid: prefix.bootstrap_recorded,
        stored_login_records: prefix.stored_login_records,
        launch_state: launch.state.as_str().to_owned(),
        tracked_game_pid: launch.process_id,
        tracked_game_pid_live,
        exit_code: launch.exit_code,
        total_rss_kib: resources.total_rss_kib,
        process_count: resources.processes.len(),
        steam_cloud_policy: profile
            .steam_cloud
            .effective(!profile.cloud_files.is_empty())
            .as_str()
            .to_owned(),
        cloud_declared_files: profile.cloud_files.len(),
        cloud_state,
        cloud_matching_files,
    };
    write_new_evidence(&output, snapshot.to_json().as_bytes())?;
    println!("evidence snapshot: {}", output.display());
    println!("phase: {phase}; profile: {}", snapshot.profile_sha256);
    println!(
        "boundary: runtime presentation/input/audio/save/roundtrip/stability are not inferred"
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HumanGateFailure {
    app_id: u32,
    profile_sha256: String,
    source_sha256: String,
    gate: String,
    code: String,
}

fn evidence_record_failure_command(args: &[String]) -> Result<(), String> {
    let usage = "usage: macroton evidence record-failure APPID --profile PATH --source ABSOLUTE_PATH --gate GATE --code CODE --output ABSOLUTE_PATH";
    let app_id: u32 = args
        .first()
        .ok_or_else(|| usage.to_owned())?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut source_path = None;
    let mut gate = None;
    let mut code = None;
    let mut output = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--source" => source_path = Some(PathBuf::from(value)),
            "--gate" => gate = Some(value.clone()),
            "--code" => code = Some(value.clone()),
            "--output" => output = Some(PathBuf::from(value)),
            other => return Err(format!("unknown evidence record-failure option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let source_path = source_path.ok_or_else(|| "--source ABSOLUTE_PATH is required".to_owned())?;
    let gate = gate.ok_or_else(|| "--gate GATE is required".to_owned())?;
    let code = code.ok_or_else(|| "--code CODE is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    validate_failure_gate(&gate)?;
    validate_failure_code(&code)?;
    if !source_path.is_absolute() || !output.is_absolute() {
        return Err("--source and --output must be absolute paths".to_owned());
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let source_sha256 = hash_regular_evidence_source(&source_path)?;
    let captured_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_secs();
    let record = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"kind\": \"human-gate-failure\",\n",
            "  \"captured_unix\": {},\n",
            "  \"appid\": {},\n",
            "  \"profile_sha256\": \"{}\",\n",
            "  \"source_sha256\": \"{}\",\n",
            "  \"gate\": \"{}\",\n",
            "  \"code\": \"{}\",\n",
            "  \"evidence_strength\": \"human-observation-content-addressed\",\n",
            "  \"status\": \"human-gate-failure-recorded\",\n",
            "  \"not_inferred\": [\"automated-metric\",\"exact-runtime-session-identity\",\"other-scenes\",\"future-runtime-regressions\"]\n",
            "}}\n"
        ),
        captured_unix,
        app_id,
        profile.fingerprint(),
        source_sha256,
        gate,
        code,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!("human gate failure: {}", output.display());
    println!("gate: {gate}; code: {code}");
    Ok(())
}

fn evidence_failure_status_command(args: &[String]) -> Result<(), String> {
    let usage = "usage: macroton evidence failure-status APPID --profile PATH --failure ABSOLUTE_PATH --source ABSOLUTE_PATH";
    let app_id: u32 = args
        .first()
        .ok_or_else(|| usage.to_owned())?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut failure_path = None;
    let mut source_path = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--failure" => failure_path = Some(PathBuf::from(value)),
            "--source" => source_path = Some(PathBuf::from(value)),
            other => return Err(format!("unknown evidence failure-status option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let failure_path =
        failure_path.ok_or_else(|| "--failure ABSOLUTE_PATH is required".to_owned())?;
    let source_path = source_path.ok_or_else(|| "--source ABSOLUTE_PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    let failure = read_human_gate_failure(&failure_path)?;
    let source_sha256 = hash_regular_evidence_source(&source_path)?;
    if profile.app_id != app_id
        || failure.app_id != app_id
        || failure.profile_sha256 != profile.fingerprint()
        || failure.source_sha256 != source_sha256
    {
        return Err(
            "failure evidence does not bind to the selected AppID/profile/source".to_owned(),
        );
    }
    println!("{{");
    println!("  \"schema_version\": 1,");
    println!("  \"appid\": {app_id},");
    println!("  \"profile_sha256\": \"{}\",", failure.profile_sha256);
    println!("  \"source_sha256\": \"{}\",", failure.source_sha256);
    println!("  \"gate\": \"{}\",", failure.gate);
    println!("  \"code\": \"{}\",", failure.code);
    println!("  \"failure_evidence_valid\": true,");
    println!("  \"gate_pass\": false");
    println!("}}");
    Ok(())
}

fn validate_failure_gate(gate: &str) -> Result<(), String> {
    if [
        "visual-correctness",
        "audible-quality",
        "semantic-input",
        "gameplay-correctness",
        "save-reload-inside-game",
        "sustained-playability",
        "normal-exit",
    ]
    .contains(&gate)
    {
        Ok(())
    } else {
        Err(format!("unsupported human failure gate {gate:?}"))
    }
}

fn validate_failure_code(code: &str) -> Result<(), String> {
    if (3..=80).contains(&code.len())
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !code.starts_with('-')
        && !code.ends_with('-')
        && !code.contains("--")
    {
        Ok(())
    } else {
        Err(
            "failure code must be 3-80 lowercase ASCII letters, digits, or single hyphens"
                .to_owned(),
        )
    }
}

fn hash_regular_evidence_source(path: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("evidence source path must be absolute".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err("evidence source must be a regular, non-symlink file".to_owned());
    }
    sha256::digest_file(path)
        .map(|digest| sha256::to_hex(&digest))
        .map_err(|error| format!("cannot hash {}: {error}", path.display()))
}

fn read_human_gate_failure(path: &Path) -> Result<HumanGateFailure, String> {
    let (fields, _) = read_hashed_canonical_evidence(path)?;
    let field = |name: &str| {
        fields
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("missing field {name:?}"))
    };
    if field("schema_version")? != "1"
        || parse_json_string(field("kind")?, "kind")? != "human-gate-failure"
        || parse_json_string(field("status")?, "status")? != "human-gate-failure-recorded"
        || parse_json_string(field("evidence_strength")?, "evidence_strength")?
            != "human-observation-content-addressed"
    {
        return Err("expected a recorded schema-1 human gate failure".to_owned());
    }
    let gate = parse_json_string(field("gate")?, "gate")?;
    let code = parse_json_string(field("code")?, "code")?;
    validate_failure_gate(&gate)?;
    validate_failure_code(&code)?;
    Ok(HumanGateFailure {
        app_id: parse_json_u64(field("appid")?, "appid")?
            .try_into()
            .map_err(|_| "appid is out of range".to_owned())?,
        profile_sha256: parse_sha256_json(field("profile_sha256")?, "profile_sha256")?,
        source_sha256: parse_sha256_json(field("source_sha256")?, "source_sha256")?,
        gate,
        code,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AutomatedBundleReference {
    app_id: u32,
    profile_sha256: String,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HumanGameplayReview {
    app_id: u32,
    profile_sha256: String,
    automated_bundle_sha256: String,
    visual_correct: bool,
    audio_acceptable: bool,
    semantic_input_correct: bool,
    gameplay_correct: bool,
    save_reload_pass: bool,
    sustained_playability: bool,
    sha256: Option<String>,
}

impl HumanGameplayReview {
    fn all_pass(&self) -> bool {
        self.visual_correct
            && self.audio_acceptable
            && self.semantic_input_correct
            && self.gameplay_correct
            && self.save_reload_pass
            && self.sustained_playability
    }

    fn failed_gates(&self) -> Vec<&'static str> {
        [
            ("visual-correctness", self.visual_correct),
            ("audible-quality", self.audio_acceptable),
            ("semantic-input", self.semantic_input_correct),
            ("gameplay-correctness", self.gameplay_correct),
            ("save-reload-inside-game", self.save_reload_pass),
            ("sustained-playability", self.sustained_playability),
        ]
        .into_iter()
        .filter_map(|(label, pass)| (!pass).then_some(label))
        .collect()
    }
}

fn evidence_record_review_command(args: &[String]) -> Result<(), String> {
    let usage = "usage: macroton evidence record-review APPID --profile PATH --bundle ABSOLUTE_PATH --visual pass|fail --audio pass|fail --input pass|fail --gameplay pass|fail --save-reload pass|fail --sustained-playability pass|fail --output ABSOLUTE_PATH";
    let app_id: u32 = args
        .first()
        .ok_or_else(|| usage.to_owned())?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut bundle_path = None;
    let mut output = None;
    let mut gates: BTreeMap<&str, bool> = BTreeMap::new();
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--bundle" => bundle_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--visual"
            | "--audio"
            | "--input"
            | "--gameplay"
            | "--save-reload"
            | "--sustained-playability" => {
                if gates.insert(option, parse_review_verdict(value)?).is_some() {
                    return Err(format!("duplicate review gate {option}"));
                }
            }
            other => return Err(format!("unknown evidence record-review option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let bundle_path = bundle_path.ok_or_else(|| "--bundle ABSOLUTE_PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !bundle_path.is_absolute() || !output.is_absolute() {
        return Err("--bundle and --output must be absolute paths".to_owned());
    }
    let gate = |name: &'static str| {
        gates
            .get(name)
            .copied()
            .ok_or_else(|| format!("{name} pass|fail is required"))
    };
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let bundle = read_automated_bundle(&bundle_path)?;
    if bundle.app_id != app_id || bundle.profile_sha256 != profile.fingerprint() {
        return Err("automated bundle does not match the selected AppID/profile".to_owned());
    }
    let review = HumanGameplayReview {
        app_id,
        profile_sha256: profile.fingerprint(),
        automated_bundle_sha256: bundle.sha256,
        visual_correct: gate("--visual")?,
        audio_acceptable: gate("--audio")?,
        semantic_input_correct: gate("--input")?,
        gameplay_correct: gate("--gameplay")?,
        save_reload_pass: gate("--save-reload")?,
        sustained_playability: gate("--sustained-playability")?,
        sha256: None,
    };
    let captured_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_secs();
    let status = if review.all_pass() {
        "human-gameplay-review-pass"
    } else {
        "human-gameplay-review-fail"
    };
    let record = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"kind\": \"human-gameplay-review\",\n",
            "  \"captured_unix\": {},\n",
            "  \"appid\": {},\n",
            "  \"profile_sha256\": \"{}\",\n",
            "  \"automated_bundle_sha256\": \"{}\",\n",
            "  \"visual_correct\": {},\n",
            "  \"audio_acceptable\": {},\n",
            "  \"semantic_input_correct\": {},\n",
            "  \"gameplay_correct\": {},\n",
            "  \"save_reload_pass\": {},\n",
            "  \"sustained_playability\": {},\n",
            "  \"status\": \"{}\",\n",
            "  \"not_inferred\": [\"other-hardware\",\"future-runtime-regressions\",\"unreviewed-scenes\"]\n",
            "}}\n"
        ),
        captured_unix,
        app_id,
        review.profile_sha256,
        review.automated_bundle_sha256,
        review.visual_correct,
        review.audio_acceptable,
        review.semantic_input_correct,
        review.gameplay_correct,
        review.save_reload_pass,
        review.sustained_playability,
        status,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!("human gameplay review: {}", output.display());
    println!("status: {status}");
    if !review.all_pass() {
        println!("failed gates: {}", review.failed_gates().join(","));
    }
    Ok(())
}

fn evidence_acceptance_status_command(args: &[String]) -> Result<(), String> {
    let usage = "usage: macroton evidence acceptance-status APPID --profile PATH --bundle ABSOLUTE_PATH --review ABSOLUTE_PATH";
    let app_id: u32 = args
        .first()
        .ok_or_else(|| usage.to_owned())?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut bundle_path = None;
    let mut review_path = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--bundle" => bundle_path = Some(PathBuf::from(value)),
            "--review" => review_path = Some(PathBuf::from(value)),
            other => {
                return Err(format!(
                    "unknown evidence acceptance-status option {other:?}"
                ));
            }
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let bundle_path = bundle_path.ok_or_else(|| "--bundle ABSOLUTE_PATH is required".to_owned())?;
    let review_path = review_path.ok_or_else(|| "--review ABSOLUTE_PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let bundle = read_automated_bundle(&bundle_path)?;
    let review = read_human_gameplay_review(&review_path)?;
    let profile_sha256 = profile.fingerprint();
    if bundle.app_id != app_id
        || bundle.profile_sha256 != profile_sha256
        || review.app_id != app_id
        || review.profile_sha256 != profile_sha256
        || review.automated_bundle_sha256 != bundle.sha256
    {
        return Err(
            "acceptance evidence does not bind to the selected AppID/profile/bundle".to_owned(),
        );
    }
    let ready = review.all_pass();
    let failed_gates = review
        .failed_gates()
        .into_iter()
        .map(|gate| format!("\"{gate}\""))
        .collect::<Vec<_>>()
        .join(",");
    println!("{{");
    println!("  \"schema_version\": 1,");
    println!("  \"appid\": {app_id},");
    println!("  \"profile_sha256\": \"{profile_sha256}\",");
    println!("  \"automated_bundle_sha256\": \"{}\",", bundle.sha256);
    println!(
        "  \"human_review_sha256\": \"{}\",",
        review.sha256.as_deref().unwrap_or("")
    );
    println!("  \"automated_bundle_valid\": true,");
    println!("  \"human_review_all_pass\": {ready},");
    println!("  \"failed_human_gates\": [{failed_gates}],");
    println!("  \"gameplay_acceptance_ready\": {ready},");
    println!(
        "  \"evidence_boundary\": \"release redistribution, other hardware, and future runtime regressions remain separate\""
    );
    println!("}}");
    Ok(())
}

fn parse_review_verdict(value: &str) -> Result<bool, String> {
    match value {
        "pass" => Ok(true),
        "fail" => Ok(false),
        _ => Err(format!(
            "review verdict must be pass or fail, got {value:?}"
        )),
    }
}

fn read_automated_bundle(path: &Path) -> Result<AutomatedBundleReference, String> {
    let (fields, sha256) = read_hashed_canonical_evidence(path)?;
    let field = |name: &str| {
        fields
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("missing field {name:?}"))
    };
    if field("schema_version")? != "1"
        || parse_json_string(field("kind")?, "kind")? != "automated-runtime-evidence-bundle"
        || parse_json_string(field("status")?, "status")?
            != "automated-runtime-evidence-bundle-pass"
    {
        return Err("expected a passing schema-1 automated runtime evidence bundle".to_owned());
    }
    for name in [
        "runtime_fingerprint_sha256",
        "steam_session_sha256",
        "focus_sha256",
        "motion_sha256",
        "interaction_sha256",
        "audio_sha256",
        "independence_sha256",
        "exit_sha256",
    ] {
        parse_sha256_json(field(name)?, name)?;
    }
    for name in [
        "runtime_id",
        "steam_client_mode",
        "renderer",
        "graphics_api",
    ] {
        if parse_json_string(field(name)?, name)?.trim().is_empty() {
            return Err(format!("automated bundle field {name:?} cannot be empty"));
        }
    }
    let steam_cloud_policy = fields
        .get("steam_cloud_policy")
        .map(|value| parse_json_string(value, "steam_cloud_policy"))
        .transpose()?
        .unwrap_or_else(|| "required".to_owned());
    let cloud_transaction_id =
        parse_json_u64(field("cloud_transaction_id")?, "cloud_transaction_id")?;
    let cloud_phase = parse_json_string(field("cloud_phase")?, "cloud_phase")?;
    let cloud_valid = match steam_cloud_policy.as_str() {
        "required" => cloud_transaction_id > 0 && cloud_phase == "complete",
        "unsupported" => cloud_transaction_id == 0 && cloud_phase == "not-applicable-unsupported",
        _ => false,
    };
    if parse_json_u64(field("tracked_game_pid")?, "tracked_game_pid")? == 0
        || !cloud_valid
        || parse_json_u64(
            field("exact_prefix_residual_processes")?,
            "exact_prefix_residual_processes",
        )? != 0
        || parse_json_u64(
            field("exact_prefix_residual_rss_kib")?,
            "exact_prefix_residual_rss_kib",
        )? != 0
    {
        return Err(
            "automated bundle does not prove a live session, its declared Cloud policy, and zero residuals"
                .to_owned(),
        );
    }
    Ok(AutomatedBundleReference {
        app_id: parse_json_u64(field("appid")?, "appid")?
            .try_into()
            .map_err(|_| "appid is out of range".to_owned())?,
        profile_sha256: parse_sha256_json(field("profile_sha256")?, "profile_sha256")?,
        sha256,
    })
}

fn read_human_gameplay_review(path: &Path) -> Result<HumanGameplayReview, String> {
    let (fields, sha256) = read_hashed_canonical_evidence(path)?;
    let field = |name: &str| {
        fields
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("missing field {name:?}"))
    };
    if field("schema_version")? != "1"
        || parse_json_string(field("kind")?, "kind")? != "human-gameplay-review"
    {
        return Err("expected schema-1 human gameplay review evidence".to_owned());
    }
    let review = HumanGameplayReview {
        app_id: parse_json_u64(field("appid")?, "appid")?
            .try_into()
            .map_err(|_| "appid is out of range".to_owned())?,
        profile_sha256: parse_sha256_json(field("profile_sha256")?, "profile_sha256")?,
        automated_bundle_sha256: parse_sha256_json(
            field("automated_bundle_sha256")?,
            "automated_bundle_sha256",
        )?,
        visual_correct: parse_json_bool(field("visual_correct")?, "visual_correct")?,
        audio_acceptable: parse_json_bool(field("audio_acceptable")?, "audio_acceptable")?,
        semantic_input_correct: parse_json_bool(
            field("semantic_input_correct")?,
            "semantic_input_correct",
        )?,
        gameplay_correct: parse_json_bool(field("gameplay_correct")?, "gameplay_correct")?,
        save_reload_pass: parse_json_bool(field("save_reload_pass")?, "save_reload_pass")?,
        sustained_playability: parse_json_bool(
            field("sustained_playability")?,
            "sustained_playability",
        )?,
        sha256: Some(sha256),
    };
    let expected_status = if review.all_pass() {
        "human-gameplay-review-pass"
    } else {
        "human-gameplay-review-fail"
    };
    if parse_json_string(field("status")?, "status")? != expected_status {
        return Err("human review status contradicts its gate values".to_owned());
    }
    Ok(review)
}

fn read_hashed_canonical_evidence(
    path: &Path,
) -> Result<(BTreeMap<String, String>, String), String> {
    if !path.is_absolute() {
        return Err("evidence path must be absolute".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err("evidence path must be a regular, non-symlink file".to_owned());
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let fields = parse_flat_evidence_fields(&contents)?;
    let hash = sha256::to_hex(
        &sha256::digest_file(path)
            .map_err(|error| format!("cannot hash {}: {error}", path.display()))?,
    );
    Ok((fields, hash))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundleArtifact {
    app_id: u32,
    profile_sha256: String,
    runtime_id: String,
    runtime_fingerprint_sha256: String,
    steam_session_sha256: String,
    steam_client_mode: String,
    renderer: String,
    graphics_api: String,
    tracked_game_pid: u32,
    sha256: String,
}

const BUNDLE_ARTIFACT_SPECS: [(&str, &str, &str); 6] = [
    (
        "focus",
        "runtime-focused-temporal-sample",
        "focused-temporal-sample-pass",
    ),
    (
        "motion",
        "runtime-motion-presentation-proxy",
        "motion-presentation-proxy-pass",
    ),
    (
        "interaction",
        "runtime-input-associated-frame-change",
        "input-associated-frame-change-pass",
    ),
    ("audio", "runtime-audio-presence", "pcm-presence-pass"),
    (
        "independence",
        "runtime-process-independence",
        "managed-runtime-no-crossover-path-pass",
    ),
    ("exit", "runtime-normal-game-exit", "normal-game-exit-pass"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundleStatusSummary {
    app_id: u32,
    profile_sha256: String,
    runtime_state: String,
    runtime_ready: bool,
    artifact_statuses: Vec<(&'static str, &'static str)>,
    artifacts_consistent: bool,
    steam_cloud_policy: String,
    cloud_phase: String,
    exit_zero: bool,
    zero_residuals: bool,
    automated_bundle_ready: bool,
    blocking_gate: String,
    next_action: String,
    blocking_artifact: Option<(String, String)>,
}

fn evidence_bundle_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton evidence bundle-status APPID --profile PATH --evidence-dir PATH [--data-root PATH] [--runtime-root PATH]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut evidence_dir = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--evidence-dir" => evidence_dir = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown evidence bundle-status option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let evidence_dir = evidence_dir.ok_or_else(|| "--evidence-dir PATH is required".to_owned())?;
    if !evidence_dir.is_absolute() {
        return Err("--evidence-dir must be an absolute path".to_owned());
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let status = inspect_bundle_status(app_id, &profile, &evidence_dir, &data_root, &runtime_root);
    print_bundle_status(&status);
    Ok(())
}

fn inspect_bundle_status(
    app_id: u32,
    profile: &CompatibilityProfile,
    evidence_dir: &Path,
    data_root: &Path,
    runtime_root: &Path,
) -> BundleStatusSummary {
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    let installed = runtime::resolve_installed(runtime_root, &runtime_id).ok();
    let prefix = steam_prefix_status::inspect(app_id, data_root).ok();
    let runtime_state = match (&installed, &prefix) {
        (None, _) => "runtime-unresolved",
        (Some(_), None) => "prefix-unavailable",
        (Some(installed), Some(prefix)) => {
            if steam_prefix_status::validate_expected_runtime(
                app_id,
                &prefix.prefix,
                &runtime_id,
                &installed.fingerprint_sha256,
            )
            .is_ok()
            {
                "ready"
            } else {
                "prefix-runtime-provenance-mismatch"
            }
        }
    };
    let runtime_ready = runtime_state == "ready";
    let mut statuses = Vec::new();
    let mut artifacts = Vec::new();
    for (label, kind, status) in BUNDLE_ARTIFACT_SPECS {
        let path = evidence_dir.join(format!("{label}.json"));
        if !path.exists() {
            statuses.push((label, "missing"));
            continue;
        }
        match read_bundle_artifact(&path, kind, status) {
            Ok(artifact) => {
                statuses.push((label, "valid"));
                artifacts.push((label, artifact));
            }
            Err(_) => statuses.push((label, "invalid")),
        }
    }
    let artifact_refs = artifacts
        .iter()
        .map(|(label, artifact)| (*label, artifact))
        .collect::<Vec<_>>();
    let artifacts_consistent = installed.as_ref().is_some_and(|installed| {
        artifacts.len() == BUNDLE_ARTIFACT_SPECS.len()
            && validate_bundle_artifact_set(
                app_id,
                &profile.fingerprint(),
                &runtime_id,
                &installed.fingerprint_sha256,
                profile.steam_client_mode.as_str(),
                &profile.renderer,
                profile.graphics_api.as_str(),
                &artifact_refs,
            )
            .is_ok()
    });
    let steam_cloud_policy = profile
        .steam_cloud
        .effective(!profile.cloud_files.is_empty());
    let cloud_phase = if steam_cloud_policy == SteamCloudPolicy::Unsupported {
        "not-applicable-unsupported".to_owned()
    } else {
        cloud_roundtrip::status(app_id, profile, data_root)
            .map(|transaction| transaction.phase.as_str().to_owned())
            .unwrap_or_else(|_| "unavailable".to_owned())
    };
    let cloud_gate_ready = cloud_phase == "complete"
        || (steam_cloud_policy == SteamCloudPolicy::Unsupported
            && cloud_phase == "not-applicable-unsupported");
    let launch = launch_status::inspect(app_id, data_root).ok();
    let exit_zero = launch.as_ref().is_some_and(|launch| {
        launch.state == launch_status::LaunchState::Exited && launch.exit_code == Some(0)
    });
    let resources = stop::inspect_resources(app_id, data_root).ok();
    let zero_residuals = resources
        .as_ref()
        .is_some_and(|resources| resources.processes.is_empty() && resources.total_rss_kib == 0);
    let ready =
        runtime_ready && artifacts_consistent && cloud_gate_ready && exit_zero && zero_residuals;
    let (blocking_gate, next_action, blocking_artifact) = bundle_status_next_action(
        runtime_state,
        &statuses,
        artifacts_consistent,
        &cloud_phase,
        exit_zero,
        zero_residuals,
    );
    BundleStatusSummary {
        app_id,
        profile_sha256: profile.fingerprint(),
        runtime_state: runtime_state.to_owned(),
        runtime_ready,
        artifact_statuses: statuses,
        artifacts_consistent,
        steam_cloud_policy: steam_cloud_policy.as_str().to_owned(),
        cloud_phase,
        exit_zero,
        zero_residuals,
        automated_bundle_ready: ready,
        blocking_gate: blocking_gate.to_owned(),
        next_action,
        blocking_artifact,
    }
}

fn print_bundle_status(status: &BundleStatusSummary) {
    println!("{{");
    println!("  \"schema_version\": 2,");
    println!("  \"appid\": {},", status.app_id);
    println!("  \"profile_sha256\": \"{}\",", status.profile_sha256);
    println!("  \"runtime_state\": \"{}\",", status.runtime_state);
    println!("  \"runtime_ready\": {},", status.runtime_ready);
    println!("  \"artifacts\": {{");
    for (position, (label, artifact_status)) in status.artifact_statuses.iter().enumerate() {
        println!(
            "    \"{}\": \"{}\"{}",
            json_escape(label),
            json_escape(artifact_status),
            if position + 1 == status.artifact_statuses.len() {
                ""
            } else {
                ","
            }
        );
    }
    println!("  }},");
    println!(
        "  \"artifacts_consistent\": {},",
        status.artifacts_consistent
    );
    println!(
        "  \"steam_cloud_policy\": \"{}\",",
        json_escape(&status.steam_cloud_policy)
    );
    println!(
        "  \"cloud_phase\": \"{}\",",
        json_escape(&status.cloud_phase)
    );
    println!("  \"exit_zero\": {},", status.exit_zero);
    println!("  \"zero_residuals\": {},", status.zero_residuals);
    println!(
        "  \"automated_bundle_ready\": {},",
        status.automated_bundle_ready
    );
    println!("  \"blocking_gate\": \"{}\",", status.blocking_gate);
    println!("  \"next_action\": \"{}\",", status.next_action);
    if let Some((label, state)) = &status.blocking_artifact {
        println!("  \"blocking_artifact\": \"{}\",", json_escape(label));
        println!("  \"blocking_artifact_state\": \"{}\"", json_escape(state));
    } else {
        println!("  \"blocking_artifact\": null,");
        println!("  \"blocking_artifact_state\": null");
    }
    println!("}}");
}

fn evidence_matrix_status_command(args: &[String]) -> Result<(), String> {
    let usage = "usage: macroton evidence matrix-status --profiles-dir ABSOLUTE_PATH --evidence-root ABSOLUTE_PATH [--failure-dir ABSOLUTE_PATH --failure-source APPID=ABSOLUTE_PATH] [--data-root PATH] [--app-data-root APPID=PATH] [--runtime-root PATH]";
    let mut profiles_dir = None;
    let mut evidence_root = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut app_data_roots = BTreeMap::new();
    let mut failure_dir = None;
    let mut failure_sources = BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profiles-dir" => profiles_dir = Some(PathBuf::from(value)),
            "--evidence-root" => evidence_root = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--failure-dir" => failure_dir = Some(PathBuf::from(value)),
            "--failure-source" => {
                let (app_id, path) = value
                    .split_once('=')
                    .ok_or_else(|| "--failure-source must be APPID=ABSOLUTE_PATH".to_owned())?;
                let app_id: u32 = app_id
                    .parse()
                    .map_err(|_| "--failure-source APPID must be an unsigned integer".to_owned())?;
                let path = PathBuf::from(path);
                if !path.is_absolute() || failure_sources.insert(app_id, path).is_some() {
                    return Err(format!(
                        "--failure-source for AppID {app_id} must be absolute and unique"
                    ));
                }
            }
            "--app-data-root" => {
                let (app_id, path) = value
                    .split_once('=')
                    .ok_or_else(|| "--app-data-root must be APPID=PATH".to_owned())?;
                let app_id: u32 = app_id
                    .parse()
                    .map_err(|_| "--app-data-root APPID must be an unsigned integer".to_owned())?;
                if path.is_empty() || app_data_roots.insert(app_id, PathBuf::from(path)).is_some() {
                    return Err(format!(
                        "--app-data-root for AppID {app_id} is empty or duplicated"
                    ));
                }
            }
            other => return Err(format!("unknown evidence matrix-status option {other:?}")),
        }
        index += 1;
    }
    let profiles_dir = profiles_dir.ok_or_else(|| usage.to_owned())?;
    let evidence_root = evidence_root.ok_or_else(|| usage.to_owned())?;
    if !profiles_dir.is_absolute() || !evidence_root.is_absolute() {
        return Err("--profiles-dir and --evidence-root must be absolute paths".to_owned());
    }
    if let Some(directory) = &failure_dir {
        if !directory.is_absolute() {
            return Err("--failure-dir must be absolute".to_owned());
        }
        let metadata = fs::symlink_metadata(directory).map_err(|error| {
            format!(
                "cannot inspect failure directory {}: {error}",
                directory.display()
            )
        })?;
        if !metadata.file_type().is_dir() {
            return Err("failure directory must be a real directory".to_owned());
        }
    } else if !failure_sources.is_empty() {
        return Err("--failure-source requires --failure-dir".to_owned());
    }
    match fs::symlink_metadata(&evidence_root) {
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err("evidence root must be a real directory when it exists".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect evidence root {}: {error}",
                evidence_root.display()
            ));
        }
    }
    let metadata = fs::symlink_metadata(&profiles_dir).map_err(|error| {
        format!(
            "cannot inspect formal profile directory {}: {error}",
            profiles_dir.display()
        )
    })?;
    if !metadata.file_type().is_dir() {
        return Err("formal profile directory must be a real directory".to_owned());
    }
    let mut profile_paths = Vec::new();
    for entry in fs::read_dir(&profiles_dir)
        .map_err(|error| format!("cannot read formal profile directory: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("cannot read formal profile entry: {error}"))?
            .path();
        if path.extension().and_then(|value| value.to_str()) == Some("conf")
            && !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".example.conf"))
        {
            profile_paths.push(path);
        }
    }
    profile_paths.sort();
    if profile_paths.is_empty() {
        return Err("formal profile directory contains no release profiles".to_owned());
    }

    let console_state = macos_session::inspect().unwrap_or(macos_session::State::Unknown);
    let mut records = Vec::new();
    let mut observed_app_ids = BTreeSet::new();
    for path in profile_paths {
        let file_metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!("cannot inspect formal profile {}: {error}", path.display())
        })?;
        if !file_metadata.file_type().is_file() {
            return Err(format!(
                "formal profile must be a regular, non-symlink file: {}",
                path.display()
            ));
        }
        let profile = CompatibilityProfile::load(&path).map_err(|error| error.to_string())?;
        if !observed_app_ids.insert(profile.app_id) {
            return Err(format!(
                "formal profile directory contains duplicate AppID {}",
                profile.app_id
            ));
        }
        let expected_name = format!("{}.conf", profile.app_id);
        if path.file_name().and_then(|value| value.to_str()) != Some(expected_name.as_str()) {
            return Err(format!(
                "formal profile filename must match AppID: expected {expected_name}"
            ));
        }
        let app_data_root = app_data_roots.get(&profile.app_id).unwrap_or(&data_root);
        let status = inspect_bundle_status(
            profile.app_id,
            &profile,
            &evidence_root.join(profile.app_id.to_string()),
            app_data_root,
            &runtime_root,
        );
        let prefix_readiness = steam_prefix_status::inspect(profile.app_id, app_data_root)
            .ok()
            .map(|prefix| prefix.readiness);
        let (operational_gate, operational_next_action) = matrix_operational_next_action(
            console_state,
            prefix_readiness,
            &status.runtime_state,
            &status.blocking_gate,
            &status.next_action,
        );
        let failures = failure_dir
            .as_deref()
            .map(|directory| {
                inspect_matrix_human_failures(
                    profile.app_id,
                    &profile.fingerprint(),
                    directory,
                    failure_sources.get(&profile.app_id).map(PathBuf::as_path),
                )
            })
            .transpose()?;
        records.push((
            profile,
            status,
            prefix_readiness,
            operational_gate,
            operational_next_action,
            failures,
        ));
    }
    if let Some(unused) = app_data_roots
        .keys()
        .find(|app_id| !observed_app_ids.contains(app_id))
    {
        return Err(format!(
            "--app-data-root AppID {unused} has no formal profile"
        ));
    }
    if let Some(unused) = failure_sources
        .keys()
        .find(|app_id| !observed_app_ids.contains(app_id))
    {
        return Err(format!(
            "--failure-source AppID {unused} has no formal profile"
        ));
    }
    let ready_count = records
        .iter()
        .filter(|(_, status, _, _, _, _)| status.automated_bundle_ready)
        .count();
    println!("{{");
    println!("  \"schema_version\": 2,");
    println!("  \"console_state\": \"{}\",", console_state.as_str());
    println!("  \"formal_profile_count\": {},", records.len());
    println!("  \"automated_bundle_ready_count\": {ready_count},");
    println!("  \"human_failures_inspected\": {},", failure_dir.is_some());
    println!("  \"entries\": [");
    for (
        position,
        (profile, status, readiness, operational_gate, operational_next_action, failures),
    ) in records.iter().enumerate()
    {
        println!("    {{");
        println!("      \"appid\": {},", profile.app_id);
        println!("      \"profile_sha256\": \"{}\",", status.profile_sha256);
        println!(
            "      \"renderer\": \"{}\",",
            json_escape(&profile.renderer)
        );
        println!(
            "      \"graphics_api\": \"{}\",",
            profile.graphics_api.as_str()
        );
        println!(
            "      \"steam_cloud_policy\": \"{}\",",
            profile
                .steam_cloud
                .effective(!profile.cloud_files.is_empty())
                .as_str()
        );
        println!(
            "      \"steam_cloud_evidence\": \"{}\",",
            steam_cloud_evidence_mode(profile)
        );
        println!("      \"runtime_state\": \"{}\",", status.runtime_state);
        println!(
            "      \"prefix_readiness\": {},",
            json_optional_string(readiness.map(|readiness| readiness.as_str()))
        );
        println!(
            "      \"automated_bundle_ready\": {},",
            status.automated_bundle_ready
        );
        println!(
            "      \"acceptance_state\": \"{}\",",
            if failures
                .as_ref()
                .is_some_and(|failures| !failures.is_empty())
            {
                "known-human-failure"
            } else if status.automated_bundle_ready {
                "automated-ready-needs-human-review"
            } else {
                "pending"
            }
        );
        println!(
            "      \"human_failure_count\": {},",
            failures.as_ref().map(Vec::len).unwrap_or(0)
        );
        println!(
            "      \"human_failures\": [{}],",
            matrix_failures_json(failures.as_deref())
        );
        println!(
            "      \"evidence_blocking_gate\": \"{}\",",
            status.blocking_gate
        );
        println!(
            "      \"evidence_next_action\": \"{}\",",
            status.next_action
        );
        println!("      \"operational_gate\": \"{operational_gate}\",");
        println!("      \"operational_next_action\": \"{operational_next_action}\"");
        println!(
            "    }}{}",
            if position + 1 == records.len() {
                ""
            } else {
                ","
            }
        );
    }
    println!("  ],");
    println!(
        "  \"evidence_boundary\": \"operational actions order runtime, console and Steam authentication before evidence capture; source-rehashed human failures remain failures until a new profile-bound acceptance pass supersedes them\""
    );
    println!("}}");
    Ok(())
}

fn matrix_operational_next_action(
    console_state: macos_session::State,
    prefix_readiness: Option<steam_prefix_status::Readiness>,
    runtime_state: &str,
    evidence_gate: &str,
    evidence_next_action: &str,
) -> (&'static str, String) {
    if runtime_state != "ready" {
        return ("runtime-provenance", format!("repair-{runtime_state}"));
    }
    if console_state != macos_session::State::Unlocked {
        return (
            "interactive-console",
            if console_state == macos_session::State::Locked {
                "unlock-console".to_owned()
            } else {
                "verify-console-state".to_owned()
            },
        );
    }
    match prefix_readiness {
        None | Some(steam_prefix_status::Readiness::NotBootstrapped) => {
            return ("steam-prefix", "bootstrap-windows-steam".to_owned());
        }
        Some(steam_prefix_status::Readiness::SignInRequired) => {
            return ("steam-authentication", "sign-in-windows-steam".to_owned());
        }
        Some(steam_prefix_status::Readiness::StoredLoginPresent) => {}
    }
    (
        matrix_evidence_operational_gate(evidence_gate),
        evidence_next_action.to_owned(),
    )
}

fn inspect_matrix_human_failures(
    app_id: u32,
    profile_sha256: &str,
    failure_dir: &Path,
    source: Option<&Path>,
) -> Result<Vec<HumanGateFailure>, String> {
    let prefix = format!("{app_id}-failure-");
    let mut paths = Vec::new();
    for entry in fs::read_dir(failure_dir)
        .map_err(|error| format!("cannot read failure directory: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("cannot read failure directory entry: {error}"))?
            .path();
        let matches = path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".json"));
        if matches {
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() {
        if source.is_some() {
            return Err(format!(
                "--failure-source AppID {app_id} has no matching failure evidence"
            ));
        }
        return Ok(Vec::new());
    }
    let source = source.ok_or_else(|| {
        format!(
            "failure evidence exists for AppID {app_id}; pass --failure-source {app_id}=ABSOLUTE_PATH"
        )
    })?;
    let source_sha256 = hash_regular_evidence_source(source)?;
    let mut failures = Vec::new();
    let mut gates = BTreeSet::new();
    for path in paths {
        let failure = read_human_gate_failure(&path)?;
        if failure.app_id != app_id
            || failure.profile_sha256 != profile_sha256
            || failure.source_sha256 != source_sha256
        {
            return Err(format!(
                "failure evidence {} does not bind to matrix AppID/profile/source",
                path.display()
            ));
        }
        if !gates.insert(failure.gate.clone()) {
            return Err(format!(
                "failure directory contains duplicate gate {:?} for AppID {app_id}",
                failure.gate
            ));
        }
        failures.push(failure);
    }
    Ok(failures)
}

fn matrix_failures_json(failures: Option<&[HumanGateFailure]>) -> String {
    failures
        .unwrap_or(&[])
        .iter()
        .map(|failure| {
            format!(
                "{{\"gate\":\"{}\",\"code\":\"{}\"}}",
                json_escape(&failure.gate),
                json_escape(&failure.code)
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn matrix_evidence_operational_gate(evidence_gate: &str) -> &'static str {
    match evidence_gate {
        "none" => "human-review",
        "live-session-artifacts" | "single-session-consistency" => "live-session-evidence",
        "fresh-cloud-roundtrip" => "steam-cloud",
        "normal-exit" => "normal-exit",
        "exact-prefix-residuals" => "resource-cleanup",
        _ => "evidence-status",
    }
}

fn bundle_status_next_action(
    runtime_state: &str,
    artifact_statuses: &[(&str, &str)],
    artifacts_consistent: bool,
    cloud_phase: &str,
    exit_zero: bool,
    zero_residuals: bool,
) -> (&'static str, String, Option<(String, String)>) {
    if runtime_state != "ready" {
        return (
            "runtime-provenance",
            format!("repair-{runtime_state}"),
            None,
        );
    }
    if let Some((label, status)) = artifact_statuses
        .iter()
        .find(|(_, status)| *status != "valid")
    {
        return (
            "live-session-artifacts",
            if *status == "invalid" {
                "recapture-one-live-session".to_owned()
            } else {
                "capture-one-live-session".to_owned()
            },
            Some(((*label).to_owned(), (*status).to_owned())),
        );
    }
    if !artifacts_consistent {
        return (
            "single-session-consistency",
            "recapture-one-consistent-live-session".to_owned(),
            None,
        );
    }
    if cloud_phase != "complete" && cloud_phase != "not-applicable-unsupported" {
        return (
            "fresh-cloud-roundtrip",
            "complete-fresh-cloud-roundtrip".to_owned(),
            None,
        );
    }
    if !exit_zero {
        return (
            "normal-exit",
            "capture-normal-exit-code-zero".to_owned(),
            None,
        );
    }
    if !zero_residuals {
        return (
            "exact-prefix-residuals",
            "stop-exact-prefix".to_owned(),
            None,
        );
    }
    (
        "none",
        "record-human-review-and-run-acceptance-status".to_owned(),
        None,
    )
}

fn evidence_bundle_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton evidence bundle APPID --profile PATH --focus PATH --motion PATH --interaction PATH --audio PATH --independence PATH --exit PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut focus_path = None;
    let mut motion_path = None;
    let mut interaction_path = None;
    let mut audio_path = None;
    let mut independence_path = None;
    let mut exit_path = None;
    let mut output = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--focus" => focus_path = Some(PathBuf::from(value)),
            "--motion" => motion_path = Some(PathBuf::from(value)),
            "--interaction" => interaction_path = Some(PathBuf::from(value)),
            "--audio" => audio_path = Some(PathBuf::from(value)),
            "--independence" => independence_path = Some(PathBuf::from(value)),
            "--exit" => exit_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown evidence bundle option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !output.is_absolute() {
        return Err("--output must be an absolute path".to_owned());
    }
    let required = |value: Option<PathBuf>, option: &str| {
        value.ok_or_else(|| format!("{option} PATH is required"))
    };
    let artifact_specs = [
        ("focus", required(focus_path, "--focus")?),
        ("motion", required(motion_path, "--motion")?),
        ("interaction", required(interaction_path, "--interaction")?),
        ("audio", required(audio_path, "--audio")?),
        (
            "independence",
            required(independence_path, "--independence")?,
        ),
        ("exit", required(exit_path, "--exit")?),
    ];
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let steam_cloud_policy = profile
        .steam_cloud
        .effective(!profile.cloud_files.is_empty());
    if steam_cloud_policy == SteamCloudPolicy::Unspecified {
        return Err(
            "evidence bundle requires cloud_files or an explicit steam_cloud=unsupported policy"
                .to_owned(),
        );
    }
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    let installed = runtime::resolve_installed(&runtime_root, &runtime_id)
        .map_err(|error| format!("cannot resolve bundle runtime: {error}"))?;
    let prefix =
        steam_prefix_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    steam_prefix_status::validate_expected_runtime(
        app_id,
        &prefix.prefix,
        &runtime_id,
        &installed.fingerprint_sha256,
    )
    .map_err(|error| format!("bundle runtime provenance mismatch: {error}"))?;
    let mut artifacts = Vec::new();
    for ((label, path), (_, kind, status)) in artifact_specs.iter().zip(BUNDLE_ARTIFACT_SPECS) {
        let artifact = read_bundle_artifact(path, kind, status)
            .map_err(|error| format!("invalid {label} evidence: {error}"))?;
        artifacts.push((*label, path, artifact));
    }
    let first = &artifacts[0].2;
    let artifact_refs = artifacts
        .iter()
        .map(|(label, _, artifact)| (*label, artifact))
        .collect::<Vec<_>>();
    validate_bundle_artifact_set(
        app_id,
        &profile.fingerprint(),
        &runtime_id,
        &installed.fingerprint_sha256,
        profile.steam_client_mode.as_str(),
        &profile.renderer,
        profile.graphics_api.as_str(),
        &artifact_refs,
    )?;
    let launch = launch_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    if launch.state != launch_status::LaunchState::Exited || launch.exit_code != Some(0) {
        return Err(
            "evidence bundle requires the current Steam lifecycle to end with exit code 0"
                .to_owned(),
        );
    }
    let resources =
        stop::inspect_resources(app_id, &data_root).map_err(|error| error.to_string())?;
    if !resources.processes.is_empty() || resources.total_rss_kib != 0 {
        return Err(format!(
            "evidence bundle requires zero exact-prefix residuals; found {} process(es), {} KiB",
            resources.processes.len(),
            resources.total_rss_kib
        ));
    }
    let (cloud_transaction_id, cloud_phase) = if steam_cloud_policy == SteamCloudPolicy::Unsupported
    {
        (0, "not-applicable-unsupported")
    } else {
        let cloud = cloud_roundtrip::status(app_id, &profile, &data_root)
            .map_err(|error| format!("Cloud roundtrip evidence is unavailable: {error}"))?;
        if cloud.phase != cloud_roundtrip::Phase::Complete {
            return Err(format!(
                "Cloud roundtrip is {}, expected complete",
                cloud.phase.as_str()
            ));
        }
        (cloud.transaction_id, cloud.phase.as_str())
    };
    let captured_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_secs();
    let hashes = artifacts
        .iter()
        .map(|(label, _, artifact)| format!("  \"{label}_sha256\": \"{}\",\n", artifact.sha256))
        .collect::<String>();
    let record = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"kind\": \"automated-runtime-evidence-bundle\",\n",
            "  \"captured_unix\": {},\n",
            "  \"appid\": {},\n",
            "  \"profile_sha256\": \"{}\",\n",
            "  \"runtime_id\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"steam_session_sha256\": \"{}\",\n",
            "  \"steam_client_mode\": \"{}\",\n",
            "  \"renderer\": \"{}\",\n",
            "  \"graphics_api\": \"{}\",\n",
            "  \"steam_cloud_policy\": \"{}\",\n",
            "  \"tracked_game_pid\": {},\n",
            "{}",
            "  \"cloud_transaction_id\": {},\n",
            "  \"cloud_phase\": \"{}\",\n",
            "  \"exact_prefix_residual_processes\": 0,\n",
            "  \"exact_prefix_residual_rss_kib\": 0,\n",
            "  \"status\": \"automated-runtime-evidence-bundle-pass\",\n",
            "  \"not_inferred\": [\"human-visual-pass\",\"human-listening-pass\",\"semantic-input-correctness\",\"gameplay-correctness\",\"save-reload-inside-game\",\"future-session-stability\"]\n",
            "}}\n"
        ),
        captured_unix,
        app_id,
        profile.fingerprint(),
        json_escape(&runtime_id),
        installed.fingerprint_sha256,
        first.steam_session_sha256,
        profile.steam_client_mode.as_str(),
        json_escape(&profile.renderer),
        profile.graphics_api.as_str(),
        steam_cloud_policy.as_str(),
        first.tracked_game_pid,
        hashes,
        cloud_transaction_id,
        cloud_phase,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!("automated evidence bundle: {}", output.display());
    println!("status: automated-runtime-evidence-bundle-pass");
    println!(
        "boundary: human visual/audio/input/gameplay and in-game reload review remain required"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_bundle_artifact_set(
    app_id: u32,
    profile_sha256: &str,
    runtime_id: &str,
    runtime_fingerprint_sha256: &str,
    steam_client_mode: &str,
    renderer: &str,
    graphics_api: &str,
    artifacts: &[(&str, &BundleArtifact)],
) -> Result<(), String> {
    let (_, first) = artifacts
        .first()
        .ok_or_else(|| "evidence bundle has no artifacts".to_owned())?;
    for (label, artifact) in artifacts {
        if artifact.app_id != app_id
            || artifact.profile_sha256 != profile_sha256
            || artifact.runtime_id != runtime_id
            || artifact.runtime_fingerprint_sha256 != runtime_fingerprint_sha256
            || artifact.steam_client_mode != steam_client_mode
            || artifact.renderer != renderer
            || artifact.graphics_api != graphics_api
        {
            return Err(format!(
                "{label} evidence does not match the selected AppID/profile/runtime"
            ));
        }
        if artifact.steam_session_sha256 != first.steam_session_sha256
            || artifact.tracked_game_pid != first.tracked_game_pid
        {
            return Err(format!(
                "{label} evidence does not belong to the same Steam session and game PID"
            ));
        }
    }
    Ok(())
}

fn read_bundle_artifact(
    path: &Path,
    expected_kind: &str,
    expected_status: &str,
) -> Result<BundleArtifact, String> {
    if !path.is_absolute() {
        return Err("evidence path must be absolute".to_owned());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err("evidence path must be a regular, non-symlink file".to_owned());
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let fields = parse_flat_evidence_fields(&contents)?;
    let value = |key: &str| {
        fields
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing field {key:?}"))
    };
    let schema = fields
        .get("schema_version")
        .or_else(|| fields.get("schema"))
        .ok_or_else(|| "missing evidence schema".to_owned())?;
    if schema != "1" {
        return Err("evidence must use schema 1".to_owned());
    }
    if parse_json_string(value("kind")?, "kind")? != expected_kind
        || parse_json_string(value("status")?, "status")? != expected_status
    {
        return Err(format!(
            "expected {expected_kind}/{expected_status} evidence"
        ));
    }
    let app_id = parse_json_u64(value("appid")?, "appid")?
        .try_into()
        .map_err(|_| "appid is out of range".to_owned())?;
    let tracked_game_pid = parse_json_u64(value("tracked_game_pid")?, "tracked_game_pid")?
        .try_into()
        .ok()
        .filter(|pid: &u32| *pid > 0)
        .ok_or_else(|| "tracked_game_pid is invalid".to_owned())?;
    let profile_sha256 = parse_sha256_json(value("profile_sha256")?, "profile_sha256")?;
    let runtime_fingerprint_sha256 = parse_sha256_json(
        value("runtime_fingerprint_sha256")?,
        "runtime_fingerprint_sha256",
    )?;
    let steam_session_sha256 =
        parse_sha256_json(value("steam_session_sha256")?, "steam_session_sha256")?;
    let sha256 = sha256::to_hex(
        &sha256::digest_file(path)
            .map_err(|error| format!("cannot hash {}: {error}", path.display()))?,
    );
    Ok(BundleArtifact {
        app_id,
        profile_sha256,
        runtime_id: parse_json_string(value("runtime_id")?, "runtime_id")?,
        runtime_fingerprint_sha256,
        steam_session_sha256,
        steam_client_mode: parse_json_string(value("steam_client_mode")?, "steam_client_mode")?,
        renderer: parse_json_string(value("renderer")?, "renderer")?,
        graphics_api: parse_json_string(value("graphics_api")?, "graphics_api")?,
        tracked_game_pid,
        sha256,
    })
}

fn parse_flat_evidence_fields(contents: &str) -> Result<BTreeMap<String, String>, String> {
    if contents.lines().next().map(str::trim) != Some("{")
        || contents.lines().last().map(str::trim) != Some("}")
    {
        return Err("evidence is not a canonical JSON object".to_owned());
    }
    let mut fields = BTreeMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if !line.starts_with('"') {
            continue;
        }
        let (raw_key, raw_value) = line
            .split_once(':')
            .ok_or_else(|| "malformed top-level evidence field".to_owned())?;
        let key = raw_key.trim().trim_matches('"');
        let value = raw_value.trim().trim_end_matches(',').trim().to_owned();
        if fields.insert(key.to_owned(), value).is_some() {
            return Err(format!("duplicate field {key:?}"));
        }
    }
    Ok(fields)
}

fn parse_sha256_json(value: &str, field: &str) -> Result<String, String> {
    let value = parse_json_string(value, field)?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid {field}"));
    }
    Ok(value.to_ascii_lowercase())
}

#[derive(Debug, PartialEq)]
struct AudioProbeEvidence {
    window_id: u32,
    pid: u32,
    duration_seconds: f64,
    sample_rate: f64,
    channels: u32,
    buffers: u64,
    rms_dbfs: f64,
    peak_dbfs: f64,
    non_silent_buffers: u64,
    non_silent_ratio: f64,
    invalid_buffers: u64,
    format_changes: u64,
    owner_match: bool,
}

#[derive(Debug, PartialEq)]
struct FocusStreamEvidence {
    window_id: u32,
    pid: u32,
    frames: usize,
    duration_ms: f64,
    max_gap_ms: f64,
    minimum_luma: f64,
    black_frames: usize,
    unique_sampled_frames: usize,
    changed_transitions: usize,
    delivered_rate_fps: f64,
    changed_rate_fps: f64,
    max_unchanged_ms: f64,
    p95_change_interval_ms: f64,
}

#[derive(Debug, PartialEq)]
struct InteractionProbeEvidence {
    window_id: u32,
    pid: u32,
    action_kind: String,
    action_value: String,
    delay_ms: u32,
    width: u32,
    height: u32,
    before_png_sha256: String,
    after_png_sha256: String,
    before_sampled_sha256: String,
    after_sampled_sha256: String,
    before_luma: f64,
    after_luma: f64,
    sampled_pixels: u64,
    changed_pixels: u64,
    changed_ratio: f64,
}

struct RuntimeSessionEvidence {
    prefix: PathBuf,
    runtime_id: String,
    runtime_fingerprint_sha256: String,
    runtime_entrypoint: PathBuf,
    steam_session_sha256: String,
    game_pid: u32,
}

fn validate_live_runtime_session(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
    runtime_root: &Path,
    evidence_kind: &str,
) -> Result<RuntimeSessionEvidence, String> {
    if macos_session::inspect().map_err(|error| error.to_string())?
        != macos_session::State::Unlocked
    {
        return Err(format!(
            "{evidence_kind} evidence requires an unlocked macOS console"
        ));
    }
    let prefix =
        steam_prefix_status::inspect(app_id, data_root).map_err(|error| error.to_string())?;
    if !prefix.bootstrap_recorded {
        return Err(format!(
            "{evidence_kind} evidence requires valid Macroton bootstrap provenance"
        ));
    }
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    let installed_runtime =
        runtime::resolve_installed(runtime_root, &runtime_id).map_err(|error| error.to_string())?;
    steam_prefix_status::validate_expected_runtime(
        app_id,
        &prefix.prefix,
        &runtime_id,
        &installed_runtime.fingerprint_sha256,
    )
    .map_err(|error| format!("{evidence_kind} evidence runtime provenance mismatch: {error}"))?;
    steam_session::validate(
        &prefix.prefix,
        &steam_session::Identity {
            app_id,
            profile_sha256: profile.fingerprint(),
            runtime_id: runtime_id.clone(),
            runtime_fingerprint_sha256: installed_runtime.fingerprint_sha256.clone(),
            steam_client_mode: profile.steam_client_mode.as_str().to_owned(),
            renderer: profile.renderer.clone(),
            graphics_api: profile.graphics_api.as_str().to_owned(),
        },
    )
    .map_err(|error| {
        format!("{evidence_kind} evidence Steam session provenance mismatch: {error}")
    })?;
    let steam_session_sha256 = sha256::to_hex(
        &sha256::digest_file(&prefix.prefix.join(steam_session::MARKER))
            .map_err(|error| format!("cannot hash Steam session provenance: {error}"))?,
    );
    let launch = launch_status::inspect(app_id, data_root).map_err(|error| error.to_string())?;
    if launch.state != launch_status::LaunchState::RunningObserved {
        return Err(format!(
            "{evidence_kind} evidence requires running-observed Steam lifecycle; found {}",
            launch.state.as_str()
        ));
    }
    let game_pid = launch
        .process_id
        .ok_or_else(|| "running Steam lifecycle has no tracked game PID".to_owned())?;
    let resources =
        stop::inspect_resources(app_id, data_root).map_err(|error| error.to_string())?;
    if !resources.pids.contains(&game_pid) {
        return Err("Steam's tracked game PID is not live in the exact AppID prefix".to_owned());
    }
    Ok(RuntimeSessionEvidence {
        prefix: prefix.prefix,
        runtime_id,
        runtime_fingerprint_sha256: installed_runtime.fingerprint_sha256,
        runtime_entrypoint: installed_runtime.entrypoint,
        steam_session_sha256,
        game_pid,
    })
}

fn evidence_verify_interaction_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton evidence verify-interaction APPID --profile PATH --probe PATH --capture-dir PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--min-changed-ratio N]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut probe_path = None;
    let mut capture_dir = None;
    let mut output = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut min_changed_ratio = 0.01_f64;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--probe" => probe_path = Some(PathBuf::from(value)),
            "--capture-dir" => capture_dir = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--min-changed-ratio" => {
                min_changed_ratio = value
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
                    .ok_or_else(|| "--min-changed-ratio must be between 0 and 1".to_owned())?
            }
            other => {
                return Err(format!(
                    "unknown evidence verify-interaction option {other:?}"
                ));
            }
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let probe_path = probe_path.ok_or_else(|| "--probe PATH is required".to_owned())?;
    let capture_dir = capture_dir.ok_or_else(|| "--capture-dir PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !output.is_absolute() {
        return Err("--output must be an absolute path".to_owned());
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let live =
        validate_live_runtime_session(app_id, &profile, &data_root, &runtime_root, "interaction")?;
    let probe_contents = fs::read_to_string(&probe_path)
        .map_err(|error| format!("cannot read interaction probe result: {error}"))?;
    let probe = parse_interaction_probe(&probe_contents)?;
    if probe.pid != live.game_pid {
        return Err(format!(
            "interaction probe PID {} does not match Steam's live game PID {}",
            probe.pid, live.game_pid
        ));
    }
    if probe.changed_ratio < min_changed_ratio {
        return Err(format!(
            "interaction frame change ratio {} is below requested minimum {min_changed_ratio}",
            probe.changed_ratio
        ));
    }
    let before_path = capture_dir.join("before.png");
    let after_path = capture_dir.join("after.png");
    let before_sha256 = sha256::to_hex(
        &sha256::digest_file(&before_path)
            .map_err(|error| format!("cannot hash interaction before image: {error}"))?,
    );
    let after_sha256 = sha256::to_hex(
        &sha256::digest_file(&after_path)
            .map_err(|error| format!("cannot hash interaction after image: {error}"))?,
    );
    if before_sha256 != probe.before_png_sha256 || after_sha256 != probe.after_png_sha256 {
        return Err(
            "interaction before/after PNG content does not match the probe record".to_owned(),
        );
    }
    let probe_sha256 = sha256::to_hex(
        &sha256::digest_file(&probe_path)
            .map_err(|error| format!("cannot hash interaction probe result: {error}"))?,
    );
    let captured_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_secs();
    let record = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"kind\": \"runtime-input-associated-frame-change\",\n",
            "  \"captured_unix\": {},\n",
            "  \"appid\": {},\n",
            "  \"profile_sha256\": \"{}\",\n",
            "  \"runtime_id\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"steam_session_sha256\": \"{}\",\n",
            "  \"steam_client_mode\": \"{}\",\n",
            "  \"renderer\": \"{}\",\n",
            "  \"graphics_api\": \"{}\",\n",
            "  \"probe_sha256\": \"{}\",\n",
            "  \"tracked_game_pid\": {},\n",
            "  \"window_id\": {},\n",
            "  \"action_kind\": \"{}\",\n",
            "  \"action_value\": \"{}\",\n",
            "  \"delay_ms\": {},\n",
            "  \"width\": {},\n",
            "  \"height\": {},\n",
            "  \"before_png_sha256\": \"{}\",\n",
            "  \"after_png_sha256\": \"{}\",\n",
            "  \"before_sampled_sha256\": \"{}\",\n",
            "  \"after_sampled_sha256\": \"{}\",\n",
            "  \"before_luma\": {},\n",
            "  \"after_luma\": {},\n",
            "  \"sampled_pixels\": {},\n",
            "  \"changed_pixels\": {},\n",
            "  \"changed_ratio\": {},\n",
            "  \"minimum_changed_ratio\": {},\n",
            "  \"status\": \"input-associated-frame-change-pass\",\n",
            "  \"not_inferred\": [\"semantic-action-correctness\",\"causality-in-animated-scene\",\"control-remapping-correctness\",\"sustained-input\",\"human-input-pass\"]\n",
            "}}\n"
        ),
        captured_unix,
        app_id,
        profile.fingerprint(),
        live.runtime_id,
        live.runtime_fingerprint_sha256,
        live.steam_session_sha256,
        profile.steam_client_mode.as_str(),
        profile.renderer,
        profile.graphics_api.as_str(),
        probe_sha256,
        live.game_pid,
        probe.window_id,
        probe.action_kind,
        probe.action_value,
        probe.delay_ms,
        probe.width,
        probe.height,
        probe.before_png_sha256,
        probe.after_png_sha256,
        probe.before_sampled_sha256,
        probe.after_sampled_sha256,
        probe.before_luma,
        probe.after_luma,
        probe.sampled_pixels,
        probe.changed_pixels,
        probe.changed_ratio,
        min_changed_ratio,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!("verified interaction evidence: {}", output.display());
    println!(
        "status: input-associated-frame-change-pass for live AppID {app_id} PID {}",
        live.game_pid
    );
    println!(
        "boundary: semantic correctness and causality in animated scenes require human review"
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimePathAudit {
    absolute_paths: Vec<String>,
    runtime_paths: usize,
    prefix_paths: usize,
    forbidden_paths: Vec<String>,
}

fn evidence_verify_independence_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton evidence verify-independence APPID --profile PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut output = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => {
                return Err(format!(
                    "unknown evidence verify-independence option {other:?}"
                ));
            }
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !output.is_absolute() {
        return Err("--output must be an absolute path".to_owned());
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let live = validate_live_runtime_session(
        app_id,
        &profile,
        &data_root,
        &runtime_root,
        "runtime-independence",
    )?;
    let lsof = Command::new("lsof")
        .args(["-n", "-p", &live.game_pid.to_string(), "-Fn"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot inspect live game file mappings with lsof: {error}"))?;
    if !lsof.status.success() {
        return Err(format!(
            "lsof could not inspect live game PID {}: {}",
            live.game_pid, lsof.status
        ));
    }
    let runtime_bin = live
        .runtime_entrypoint
        .parent()
        .ok_or_else(|| "managed runtime entrypoint has no parent directory".to_owned())?
        .canonicalize()
        .map_err(|error| format!("cannot resolve managed runtime directory: {error}"))?;
    // WineHQ macOS bundles place the native executable at
    // `Contents/Resources/wine/bin/wine`, while archive runtimes commonly use
    // `runtime/bin/wine`.  The executable's parent alone therefore misses the
    // bundle's `lib/` mappings.  Include the managed runtime payload root as a
    // second ownership root without widening the audit to the user's system.
    let runtime_payload = runtime_bin
        .parent()
        .ok_or_else(|| "managed runtime bin directory has no payload parent".to_owned())?
        .canonicalize()
        .map_err(|error| format!("cannot resolve managed runtime payload: {error}"))?;
    let prefix = live
        .prefix
        .canonicalize()
        .map_err(|error| format!("cannot resolve managed prefix: {error}"))?;
    let audit = parse_runtime_path_audit(&lsof.stdout, &runtime_bin, &runtime_payload, &prefix)?;
    if !audit.forbidden_paths.is_empty() {
        return Err(format!(
            "live game PID has {} CrossOver/CodeWeavers path(s); Macroton independence failed",
            audit.forbidden_paths.len()
        ));
    }
    if audit.runtime_paths == 0 || audit.prefix_paths == 0 {
        return Err(format!(
            "live mapping snapshot lacks required managed ownership: {} runtime path(s), {} prefix path(s)",
            audit.runtime_paths, audit.prefix_paths
        ));
    }
    let path_set_sha256 = sha256::to_hex(&sha256::digest_bytes(
        audit.absolute_paths.join("\n").as_bytes(),
    ));
    let captured_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_secs();
    let record = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"kind\": \"runtime-process-independence\",\n",
            "  \"captured_unix\": {},\n",
            "  \"appid\": {},\n",
            "  \"profile_sha256\": \"{}\",\n",
            "  \"runtime_id\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"steam_session_sha256\": \"{}\",\n",
            "  \"steam_client_mode\": \"{}\",\n",
            "  \"renderer\": \"{}\",\n",
            "  \"graphics_api\": \"{}\",\n",
            "  \"tracked_game_pid\": {},\n",
            "  \"absolute_path_records\": {},\n",
            "  \"managed_runtime_path_records\": {},\n",
            "  \"managed_prefix_path_records\": {},\n",
            "  \"forbidden_crossover_codeweavers_path_records\": 0,\n",
            "  \"redacted_path_set_sha256\": \"{}\",\n",
            "  \"status\": \"managed-runtime-no-crossover-path-pass\",\n",
            "  \"not_inferred\": [\"historical-process-mappings\",\"unobserved-child-processes\",\"source-code-provenance\",\"license-compliance\",\"future-session-independence\"]\n",
            "}}\n"
        ),
        captured_unix,
        app_id,
        profile.fingerprint(),
        live.runtime_id,
        live.runtime_fingerprint_sha256,
        live.steam_session_sha256,
        profile.steam_client_mode.as_str(),
        profile.renderer,
        profile.graphics_api.as_str(),
        live.game_pid,
        audit.absolute_paths.len(),
        audit.runtime_paths,
        audit.prefix_paths,
        path_set_sha256,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!(
        "verified runtime independence evidence: {}",
        output.display()
    );
    println!(
        "status: managed-runtime-no-crossover-path-pass for live AppID {app_id} PID {}",
        live.game_pid
    );
    println!(
        "boundary: this is a live mapping snapshot, not a claim about prior or future sessions"
    );
    Ok(())
}

fn parse_runtime_path_audit(
    lsof_output: &[u8],
    runtime_bin: &Path,
    runtime_payload: &Path,
    prefix: &Path,
) -> Result<RuntimePathAudit, String> {
    let mut paths = lsof_output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| line.strip_prefix(b"n"))
        .map(|line| {
            std::str::from_utf8(line)
                .map(str::to_owned)
                .map_err(|_| "lsof emitted a non-UTF-8 path".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|path| path.starts_with('/'))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let forbidden_paths = paths
        .iter()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.contains("crossover") || lower.contains("codeweavers")
        })
        .cloned()
        .collect::<Vec<_>>();
    let runtime_paths = paths
        .iter()
        .filter(|path| {
            let path = Path::new(path);
            path.starts_with(runtime_bin) || path.starts_with(runtime_payload)
        })
        .count();
    let prefix_paths = paths
        .iter()
        .filter(|path| Path::new(path).starts_with(prefix))
        .count();
    Ok(RuntimePathAudit {
        absolute_paths: paths,
        runtime_paths,
        prefix_paths,
        forbidden_paths,
    })
}

fn parse_interaction_probe(contents: &str) -> Result<InteractionProbeEvidence, String> {
    if contents.lines().next().map(str::trim) != Some("{")
        || contents.lines().last().map(str::trim) != Some("}")
    {
        return Err("interaction probe is not a flat JSON object".to_owned());
    }
    let scalar = |key: &str| flat_json_scalar_for(contents, key, "interaction probe");
    let parse_u32 = |key: &str| {
        scalar(key)?
            .parse::<u32>()
            .map_err(|_| format!("interaction probe {key} is not an unsigned integer"))
    };
    let parse_u64 = |key: &str| {
        scalar(key)?
            .parse::<u64>()
            .map_err(|_| format!("interaction probe {key} is not an unsigned integer"))
    };
    let parse_f64 = |key: &str| {
        scalar(key)?
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("interaction probe {key} is not finite numeric evidence"))
    };
    let parse_string = |key: &str| {
        let value = scalar(key)?;
        value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .filter(|value| {
                !value.is_empty()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
                    })
            })
            .map(str::to_owned)
            .ok_or_else(|| format!("interaction probe {key} is not a safe string"))
    };
    let parse_bool = |key: &str| match scalar(key)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("interaction probe {key} is not boolean")),
    };
    if parse_u32("schema")? != 1 {
        return Err("interaction probe schema is unsupported".to_owned());
    }
    let action_kind = parse_string("action_kind")?;
    let action_value = parse_string("action_value")?;
    if !matches!(action_kind.as_str(), "key" | "click") {
        return Err("interaction probe action_kind is unsupported".to_owned());
    }
    let evidence = InteractionProbeEvidence {
        window_id: parse_u32("window_id")?,
        pid: parse_u32("pid")?,
        action_kind,
        action_value,
        delay_ms: parse_u32("delay_ms")?,
        width: parse_u32("width")?,
        height: parse_u32("height")?,
        before_png_sha256: parse_sha256_scalar(&scalar("before_png_sha256")?)?,
        after_png_sha256: parse_sha256_scalar(&scalar("after_png_sha256")?)?,
        before_sampled_sha256: parse_sha256_scalar(&scalar("before_sampled_sha256")?)?,
        after_sampled_sha256: parse_sha256_scalar(&scalar("after_sampled_sha256")?)?,
        before_luma: parse_f64("before_luma")?,
        after_luma: parse_f64("after_luma")?,
        sampled_pixels: parse_u64("sampled_pixels")?,
        changed_pixels: parse_u64("changed_pixels")?,
        changed_ratio: parse_f64("changed_ratio")?,
    };
    if evidence.window_id == 0
        || evidence.pid == 0
        || evidence.delay_ms < 100
        || evidence.width == 0
        || evidence.height == 0
        || evidence.before_png_sha256 == evidence.after_png_sha256
        || evidence.before_sampled_sha256 == evidence.after_sampled_sha256
        || evidence.sampled_pixels == 0
        || evidence.changed_pixels == 0
        || evidence.changed_pixels > evidence.sampled_pixels
        || !(0.0..=1.0).contains(&evidence.changed_ratio)
        || !(0.0..=255.0).contains(&evidence.before_luma)
        || !(0.0..=255.0).contains(&evidence.after_luma)
        || !parse_bool("frontmost_before")?
        || !parse_bool("frontmost_after")?
        || !parse_bool("owner_match_before")?
        || !parse_bool("owner_match_after")?
    {
        return Err("interaction probe did not pass identity/frame-change invariants".to_owned());
    }
    let calculated_ratio = evidence.changed_pixels as f64 / evidence.sampled_pixels as f64;
    if (calculated_ratio - evidence.changed_ratio).abs() > 0.000_001 {
        return Err("interaction probe changed_ratio is inconsistent".to_owned());
    }
    Ok(evidence)
}

fn parse_sha256_scalar(value: &str) -> Result<String, String> {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|value| value.to_ascii_lowercase())
        .ok_or_else(|| "probe SHA-256 is invalid".to_owned())
}

fn evidence_verify_focus_stream_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton evidence verify-focus-stream APPID --profile PATH --probe PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--min-frames N] [--min-duration-seconds N] [--max-gap-ms N] [--black-luma N] [--max-black-frames N]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut probe_path = None;
    let mut output = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut min_frames = 120_usize;
    let mut min_duration_seconds = 5.0_f64;
    let mut max_gap_ms = 100.0_f64;
    let mut black_luma = 1.0_f64;
    let mut max_black_frames = 0_usize;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--probe" => probe_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--min-frames" => {
                min_frames = value
                    .parse()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or_else(|| "--min-frames must be a positive integer".to_owned())?
            }
            "--min-duration-seconds" => min_duration_seconds = parse_positive_f64(value, option)?,
            "--max-gap-ms" => max_gap_ms = parse_positive_f64(value, option)?,
            "--black-luma" => {
                black_luma = value
                    .parse::<f64>()
                    .ok()
                    .filter(|number| number.is_finite() && *number >= 0.0)
                    .ok_or_else(|| "--black-luma must be a non-negative number".to_owned())?
            }
            "--max-black-frames" => {
                max_black_frames = value
                    .parse()
                    .map_err(|_| "--max-black-frames must be an unsigned integer".to_owned())?
            }
            other => {
                return Err(format!(
                    "unknown evidence verify-focus-stream option {other:?}"
                ));
            }
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let probe_path = probe_path.ok_or_else(|| "--probe PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !output.is_absolute() {
        return Err("--output must be an absolute path".to_owned());
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let live =
        validate_live_runtime_session(app_id, &profile, &data_root, &runtime_root, "focus-stream")?;
    let probe_contents = fs::read_to_string(&probe_path)
        .map_err(|error| format!("cannot read focus stream result: {error}"))?;
    let probe = parse_focus_stream(&probe_contents, black_luma)?;
    if probe.pid != live.game_pid {
        return Err(format!(
            "focus stream PID {} does not match Steam's live game PID {}",
            probe.pid, live.game_pid
        ));
    }
    if probe.frames < min_frames
        || probe.duration_ms < min_duration_seconds * 1000.0
        || probe.max_gap_ms > max_gap_ms
        || probe.black_frames > max_black_frames
    {
        return Err(format!(
            "focus stream does not pass requested bounds: frames {} (min {min_frames}), duration {:.3}s (min {min_duration_seconds}s), max gap {:.3}ms (limit {max_gap_ms}ms), black frames {} (limit {max_black_frames})",
            probe.frames,
            probe.duration_ms / 1000.0,
            probe.max_gap_ms,
            probe.black_frames
        ));
    }
    let probe_sha256 = sha256::to_hex(
        &sha256::digest_file(&probe_path)
            .map_err(|error| format!("cannot hash focus stream result: {error}"))?,
    );
    let captured_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_secs();
    let record = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"kind\": \"runtime-focused-temporal-sample\",\n",
            "  \"captured_unix\": {},\n",
            "  \"appid\": {},\n",
            "  \"profile_sha256\": \"{}\",\n",
            "  \"runtime_id\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"steam_session_sha256\": \"{}\",\n",
            "  \"steam_client_mode\": \"{}\",\n",
            "  \"renderer\": \"{}\",\n",
            "  \"graphics_api\": \"{}\",\n",
            "  \"probe_sha256\": \"{}\",\n",
            "  \"tracked_game_pid\": {},\n",
            "  \"window_id\": {},\n",
            "  \"frames\": {},\n",
            "  \"duration_ms\": {},\n",
            "  \"max_gap_ms\": {},\n",
            "  \"minimum_luma\": {},\n",
            "  \"black_luma_threshold\": {},\n",
            "  \"black_frames\": {},\n",
            "  \"unique_sampled_frames\": {},\n",
            "  \"status\": \"focused-temporal-sample-pass\",\n",
            "  \"not_inferred\": [\"correct-color\",\"localized-flicker-free\",\"input-response\",\"scene-correctness\",\"playable-frame-rate\",\"human-visual-pass\"]\n",
            "}}\n"
        ),
        captured_unix,
        app_id,
        profile.fingerprint(),
        live.runtime_id,
        live.runtime_fingerprint_sha256,
        live.steam_session_sha256,
        profile.steam_client_mode.as_str(),
        profile.renderer,
        profile.graphics_api.as_str(),
        probe_sha256,
        live.game_pid,
        probe.window_id,
        probe.frames,
        probe.duration_ms,
        probe.max_gap_ms,
        probe.minimum_luma,
        black_luma,
        probe.black_frames,
        probe.unique_sampled_frames,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!("verified focus stream evidence: {}", output.display());
    println!(
        "status: focused-temporal-sample-pass for live AppID {app_id} PID {}",
        live.game_pid
    );
    println!(
        "boundary: color, localized artifacts, input, scene correctness and playability are not inferred"
    );
    Ok(())
}

fn evidence_verify_motion_rate_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton evidence verify-motion-rate APPID --profile PATH --probe PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--min-duration-seconds N] [--min-changed-rate N] [--max-unchanged-ms N]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut probe_path = None;
    let mut output = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut min_duration_seconds = 10.0_f64;
    let mut min_changed_rate = 20.0_f64;
    let mut max_unchanged_ms = 200.0_f64;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--probe" => probe_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--min-duration-seconds" => min_duration_seconds = parse_positive_f64(value, option)?,
            "--min-changed-rate" => min_changed_rate = parse_positive_f64(value, option)?,
            "--max-unchanged-ms" => max_unchanged_ms = parse_positive_f64(value, option)?,
            other => {
                return Err(format!(
                    "unknown evidence verify-motion-rate option {other:?}"
                ));
            }
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let probe_path = probe_path.ok_or_else(|| "--probe PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !output.is_absolute() {
        return Err("--output must be an absolute path".to_owned());
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let live =
        validate_live_runtime_session(app_id, &profile, &data_root, &runtime_root, "motion-rate")?;
    let probe_contents = fs::read_to_string(&probe_path)
        .map_err(|error| format!("cannot read motion-rate focus stream: {error}"))?;
    let probe = parse_focus_stream(&probe_contents, 0.0)?;
    if probe.pid != live.game_pid {
        return Err(format!(
            "motion-rate PID {} does not match Steam's live game PID {}",
            probe.pid, live.game_pid
        ));
    }
    validate_motion_rate_bounds(
        &probe,
        min_duration_seconds,
        min_changed_rate,
        max_unchanged_ms,
    )?;
    let probe_sha256 = sha256::to_hex(
        &sha256::digest_file(&probe_path)
            .map_err(|error| format!("cannot hash motion-rate focus stream: {error}"))?,
    );
    let captured_unix_ms = unix_time_ms()?;
    let record = format!(
        concat!(
            "{{\n  \"schema\": 1,\n  \"kind\": \"runtime-motion-presentation-proxy\",\n",
            "  \"captured_unix_ms\": {captured_unix_ms},\n  \"appid\": {app_id},\n",
            "  \"profile_sha256\": \"{}\",\n  \"runtime_id\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n  \"steam_session_sha256\": \"{}\",\n",
            "  \"steam_client_mode\": \"{}\",\n  \"renderer\": \"{}\",\n  \"graphics_api\": \"{}\",\n",
            "  \"probe_sha256\": \"{probe_sha256}\",\n  \"tracked_game_pid\": {},\n  \"window_id\": {},\n",
            "  \"frames\": {},\n  \"duration_ms\": {},\n  \"delivered_rate_fps\": {},\n",
            "  \"changed_transitions\": {},\n  \"changed_rate_fps\": {},\n",
            "  \"max_unchanged_ms\": {},\n  \"p95_change_interval_ms\": {},\n",
            "  \"min_changed_rate\": {min_changed_rate},\n  \"max_unchanged_limit_ms\": {max_unchanged_ms},\n",
            "  \"status\": \"motion-presentation-proxy-pass\",\n",
            "  \"not_inferred\": [\"engine-fps\",\"simulation-rate\",\"input-latency\",\"scene-correctness\",\"localized-artifacts\",\"human-playability\"]\n}}\n"
        ),
        profile.fingerprint(),
        live.runtime_id,
        live.runtime_fingerprint_sha256,
        live.steam_session_sha256,
        profile.steam_client_mode.as_str(),
        profile.renderer,
        profile.graphics_api.as_str(),
        live.game_pid,
        probe.window_id,
        probe.frames,
        probe.duration_ms,
        probe.delivered_rate_fps,
        probe.changed_transitions,
        probe.changed_rate_fps,
        probe.max_unchanged_ms,
        probe.p95_change_interval_ms,
        captured_unix_ms = captured_unix_ms,
        app_id = app_id,
        probe_sha256 = probe_sha256,
        min_changed_rate = min_changed_rate,
        max_unchanged_ms = max_unchanged_ms,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!("verified motion presentation proxy: {}", output.display());
    println!(
        "changed-rate proxy: {:.3}/s; max unchanged: {:.3}ms",
        probe.changed_rate_fps, probe.max_unchanged_ms
    );
    println!("boundary: this is not engine FPS or a human playability result");
    Ok(())
}

fn validate_motion_rate_bounds(
    probe: &FocusStreamEvidence,
    min_duration_seconds: f64,
    min_changed_rate: f64,
    max_unchanged_ms: f64,
) -> Result<(), String> {
    if probe.duration_ms < min_duration_seconds * 1000.0
        || probe.changed_rate_fps < min_changed_rate
        || probe.max_unchanged_ms > max_unchanged_ms
    {
        return Err(format!(
            "motion-rate proxy does not pass requested bounds: duration {:.3}s (min {min_duration_seconds}s), changed rate {:.3}/s (min {min_changed_rate}/s), max unchanged {:.3}ms (limit {max_unchanged_ms}ms)",
            probe.duration_ms / 1000.0,
            probe.changed_rate_fps,
            probe.max_unchanged_ms,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GpuCaptureDigest {
    sha256: String,
    file_count: usize,
    total_bytes: u64,
}

fn evidence_verify_gpu_capture_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton evidence verify-gpu-capture APPID --profile PATH --capture ABSOLUTE_PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--stability-ms N]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut capture_path = None;
    let mut output = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut stability_ms = 1000_u64;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--capture" => capture_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--stability-ms" => {
                stability_ms = parse_positive_u64(value, option)?;
                if stability_ms > 10_000 {
                    return Err("--stability-ms must not exceed 10000".to_owned());
                }
            }
            other => {
                return Err(format!(
                    "unknown evidence verify-gpu-capture option {other:?}"
                ));
            }
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let capture_path =
        capture_path.ok_or_else(|| "--capture ABSOLUTE_PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !capture_path.is_absolute() || !output.is_absolute() {
        return Err("--capture and --output must be absolute paths".to_owned());
    }
    if capture_path == output {
        return Err("GPU capture and evidence output must be different paths".to_owned());
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let live =
        validate_live_runtime_session(app_id, &profile, &data_root, &runtime_root, "gpu-capture")?;
    let first = digest_gpu_capture(&capture_path)?;
    thread::sleep(Duration::from_millis(stability_ms));
    let stable = digest_gpu_capture(&capture_path)?;
    if first != stable {
        return Err("GPU capture changed during the stability interval".to_owned());
    }
    let captured_unix_ms = unix_time_ms()?;
    let record = format!(
        concat!(
            "{{\n  \"schema\": 1,\n  \"kind\": \"runtime-gpu-command-capture\",\n",
            "  \"captured_unix_ms\": {captured_unix_ms},\n  \"appid\": {app_id},\n",
            "  \"profile_sha256\": \"{}\",\n  \"runtime_id\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n  \"steam_session_sha256\": \"{}\",\n",
            "  \"steam_client_mode\": \"{}\",\n  \"renderer\": \"{}\",\n  \"graphics_api\": \"{}\",\n",
            "  \"tracked_game_pid\": {},\n  \"capture_sha256\": \"{}\",\n",
            "  \"capture_file_count\": {},\n  \"capture_total_bytes\": {},\n",
            "  \"stability_ms\": {stability_ms},\n  \"status\": \"gpu-command-capture-pass\",\n",
            "  \"not_inferred\": [\"non-black-presentation\",\"visible-window\",\"correct-rendering\",\"engine-fps\",\"input-response\",\"playability\"]\n}}\n"
        ),
        profile.fingerprint(),
        live.runtime_id,
        live.runtime_fingerprint_sha256,
        live.steam_session_sha256,
        profile.steam_client_mode.as_str(),
        profile.renderer,
        profile.graphics_api.as_str(),
        live.game_pid,
        stable.sha256,
        stable.file_count,
        stable.total_bytes,
        captured_unix_ms = captured_unix_ms,
        app_id = app_id,
        stability_ms = stability_ms,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!("verified GPU command capture: {}", output.display());
    println!(
        "capture: {} file(s), {} byte(s), SHA-256 {}",
        stable.file_count, stable.total_bytes, stable.sha256
    );
    println!("boundary: GPU commands do not prove visible or correct presentation");
    Ok(())
}

fn digest_gpu_capture(path: &Path) -> Result<GpuCaptureDigest, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect GPU capture {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err("GPU capture root must not be a symbolic link".to_owned());
    }
    let mut files = Vec::<(PathBuf, PathBuf)>::new();
    if metadata.is_file() {
        files.push((PathBuf::from("capture"), path.to_owned()));
    } else if metadata.is_dir() {
        collect_gpu_capture_files(path, path, &mut files)?;
    } else {
        return Err("GPU capture must be a regular file or directory".to_owned());
    }
    if files.is_empty() {
        return Err("GPU capture contains no regular files".to_owned());
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut manifest = Vec::new();
    let mut total_bytes = 0_u64;
    for (relative, file) in &files {
        let metadata = fs::metadata(file)
            .map_err(|error| format!("cannot inspect GPU capture member: {error}"))?;
        if metadata.len() == 0 {
            return Err("GPU capture contains an empty file".to_owned());
        }
        total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| "GPU capture total size overflow".to_owned())?;
        let digest = sha256::to_hex(
            &sha256::digest_file(file)
                .map_err(|error| format!("cannot hash GPU capture member: {error}"))?,
        );
        manifest.extend_from_slice(relative.to_string_lossy().as_bytes());
        manifest.push(0);
        manifest.extend_from_slice(metadata.len().to_string().as_bytes());
        manifest.push(0);
        manifest.extend_from_slice(digest.as_bytes());
        manifest.push(b'\n');
    }
    Ok(GpuCaptureDigest {
        sha256: sha256::to_hex(&sha256::digest_bytes(&manifest)),
        file_count: files.len(),
        total_bytes,
    })
}

fn collect_gpu_capture_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    if files.len() > 10_000 {
        return Err("GPU capture contains more than 10000 files".to_owned());
    }
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("cannot read GPU capture directory: {error}"))?
    {
        let entry = entry.map_err(|error| format!("cannot read GPU capture entry: {error}"))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect GPU capture entry: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("GPU capture contains a symbolic link".to_owned());
        }
        if metadata.is_dir() {
            collect_gpu_capture_files(root, &path, files)?;
        } else if metadata.is_file() {
            if files.len() >= 10_000 {
                return Err("GPU capture contains more than 10000 files".to_owned());
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "GPU capture member escapes its root".to_owned())?
                .to_owned();
            files.push((relative, path));
        } else {
            return Err("GPU capture contains a non-regular entry".to_owned());
        }
    }
    Ok(())
}

fn parse_focus_stream(contents: &str, black_luma: f64) -> Result<FocusStreamEvidence, String> {
    let mut lines = contents.lines();
    if lines.next() != Some("frame,elapsed_ms,frontmost,average_luma,sampled_sha256,window_id,pid")
    {
        return Err("focus stream CSV header is unsupported".to_owned());
    }
    let mut expected_frame = 0_usize;
    let mut prior_elapsed = None;
    let mut first_elapsed = None;
    let mut max_gap_ms = 0.0_f64;
    let mut minimum_luma = f64::INFINITY;
    let mut black_frames = 0_usize;
    let mut hashes = BTreeSet::new();
    let mut prior_hash: Option<String> = None;
    let mut last_change_elapsed = None;
    let mut change_intervals_ms = Vec::new();
    let mut changed_transitions = 0_usize;
    let mut max_unchanged_ms = 0.0_f64;
    let mut window_id = None;
    let mut pid = None;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split(',').collect::<Vec<_>>();
        if fields.len() != 7 {
            return Err("focus stream CSV row does not have seven fields".to_owned());
        }
        let frame: usize = fields[0]
            .parse()
            .map_err(|_| "focus stream frame is invalid".to_owned())?;
        if frame != expected_frame {
            return Err("focus stream frames are missing, duplicated, or reordered".to_owned());
        }
        expected_frame += 1;
        let elapsed: f64 = fields[1]
            .parse()
            .ok()
            .filter(|value: &f64| value.is_finite() && *value >= 0.0)
            .ok_or_else(|| "focus stream elapsed time is invalid".to_owned())?;
        if fields[2] != "true" {
            return Err("focus stream contains an unfocused frame".to_owned());
        }
        let luma: f64 = fields[3]
            .parse()
            .ok()
            .filter(|value: &f64| value.is_finite() && (0.0..=255.0).contains(value))
            .ok_or_else(|| "focus stream luma is invalid".to_owned())?;
        let hash = fields[4];
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("focus stream sampled frame hash is invalid".to_owned());
        }
        let row_window_id: u32 = fields[5]
            .parse()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| "focus stream window ID is invalid".to_owned())?;
        let row_pid: u32 = fields[6]
            .parse()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| "focus stream PID is invalid".to_owned())?;
        if window_id.is_some_and(|value| value != row_window_id)
            || pid.is_some_and(|value| value != row_pid)
        {
            return Err("focus stream window/PID identity changed".to_owned());
        }
        window_id = Some(row_window_id);
        pid = Some(row_pid);
        if let Some(prior) = prior_elapsed {
            if elapsed <= prior {
                return Err("focus stream timestamps are not strictly increasing".to_owned());
            }
            max_gap_ms = max_gap_ms.max(elapsed - prior);
        }
        first_elapsed.get_or_insert(elapsed);
        if let Some(prior_hash) = prior_hash.as_deref() {
            let since_change = elapsed - last_change_elapsed.unwrap_or(elapsed);
            max_unchanged_ms = max_unchanged_ms.max(since_change);
            if !hash.eq_ignore_ascii_case(prior_hash) {
                changed_transitions += 1;
                change_intervals_ms.push(since_change);
                last_change_elapsed = Some(elapsed);
            }
        } else {
            last_change_elapsed = Some(elapsed);
        }
        prior_hash = Some(hash.to_ascii_lowercase());
        prior_elapsed = Some(elapsed);
        minimum_luma = minimum_luma.min(luma);
        if luma <= black_luma {
            black_frames += 1;
        }
        hashes.insert(hash.to_ascii_lowercase());
    }
    if expected_frame < 2 {
        return Err("focus stream requires at least two valid frames".to_owned());
    }
    let duration_ms = prior_elapsed.unwrap_or_default() - first_elapsed.unwrap_or_default();
    let duration_seconds = duration_ms / 1000.0;
    change_intervals_ms.sort_by(f64::total_cmp);
    let p95_change_interval_ms = if change_intervals_ms.is_empty() {
        duration_ms
    } else {
        let rank = ((change_intervals_ms.len() as f64) * 0.95).ceil() as usize;
        change_intervals_ms[rank.saturating_sub(1).min(change_intervals_ms.len() - 1)]
    };
    Ok(FocusStreamEvidence {
        window_id: window_id.unwrap_or_default(),
        pid: pid.unwrap_or_default(),
        frames: expected_frame,
        duration_ms,
        max_gap_ms,
        minimum_luma,
        black_frames,
        unique_sampled_frames: hashes.len(),
        changed_transitions,
        delivered_rate_fps: (expected_frame.saturating_sub(1) as f64) / duration_seconds,
        changed_rate_fps: changed_transitions as f64 / duration_seconds,
        max_unchanged_ms,
        p95_change_interval_ms,
    })
}

fn parse_positive_f64(value: &str, option: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite() && *number > 0.0)
        .ok_or_else(|| format!("{option} must be a positive number"))
}

fn evidence_watch_exit_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton evidence watch-exit APPID --profile PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--min-runtime-seconds N] [--timeout-seconds N] [--interval-seconds N]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut output = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut min_runtime_seconds = 600_u64;
    let mut timeout_seconds = 4 * 60 * 60_u64;
    let mut interval_seconds = 1_u64;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--min-runtime-seconds" => min_runtime_seconds = parse_positive_u64(value, option)?,
            "--timeout-seconds" => timeout_seconds = parse_positive_u64(value, option)?,
            "--interval-seconds" => interval_seconds = parse_positive_u64(value, option)?,
            other => return Err(format!("unknown evidence watch-exit option {other:?}")),
        }
        index += 1;
    }
    if timeout_seconds < min_runtime_seconds {
        return Err("--timeout-seconds must be at least --min-runtime-seconds".to_owned());
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !output.is_absolute() {
        return Err("--output must be an absolute path".to_owned());
    }
    if output.exists() {
        return Err(format!(
            "refusing to replace existing evidence: {}",
            output.display()
        ));
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let live =
        validate_live_runtime_session(app_id, &profile, &data_root, &runtime_root, "exit-watch")?;
    let _idle_sleep_guard = IdleSleepGuard::acquire("normal-exit evidence watch")?;
    let started = Instant::now();
    let mut samples = 0_u64;
    let mut live_samples = 0_u64;
    let mut peak_rss_kib = 0_u64;
    let mut disappearance_started = None;
    let mut last_live_elapsed = Duration::ZERO;
    let (exit_code, remaining_rss_kib, remaining_process_count) = loop {
        let resources =
            stop::inspect_resources(app_id, &data_root).map_err(|error| error.to_string())?;
        samples += 1;
        peak_rss_kib = peak_rss_kib.max(resources.total_rss_kib);
        if resources.pids.contains(&live.game_pid) {
            if disappearance_started.is_some() {
                return Err(
                    "tracked game PID disappeared and later returned; continuity is unproven"
                        .to_owned(),
                );
            }
            live_samples += 1;
            last_live_elapsed = started.elapsed();
        } else {
            disappearance_started.get_or_insert_with(Instant::now);
            let launch =
                launch_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
            if launch.state == launch_status::LaunchState::Exited {
                let exit_code = launch.exit_code.ok_or_else(|| {
                    "Steam observed game removal but provided no exit code".to_owned()
                })?;
                break (
                    exit_code,
                    resources.total_rss_kib,
                    resources.processes.len(),
                );
            }
            if launch.state == launch_status::LaunchState::Failed {
                return Err("Steam recorded launch failure while watching game exit".to_owned());
            }
            if disappearance_started
                .is_some_and(|started| started.elapsed() >= Duration::from_secs(30))
            {
                return Err(
                    "tracked game PID disappeared for 30s without a Steam exit record".to_owned(),
                );
            }
        }
        if started.elapsed() >= Duration::from_secs(timeout_seconds) {
            return Err(format!(
                "game did not produce a Steam-tracked exit within {timeout_seconds}s"
            ));
        }
        thread::sleep(Duration::from_secs(interval_seconds));
    };
    let observed_runtime_seconds = last_live_elapsed.as_secs();
    if exit_code != 0 {
        return Err(format!(
            "Steam tracked game exit code {exit_code}, expected 0"
        ));
    }
    if observed_runtime_seconds < min_runtime_seconds {
        return Err(format!(
            "normal exit occurred after {observed_runtime_seconds}s, below the requested {min_runtime_seconds}s stability interval"
        ));
    }
    let captured_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_secs();
    let record = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"kind\": \"runtime-normal-game-exit\",\n",
            "  \"captured_unix\": {},\n",
            "  \"appid\": {},\n",
            "  \"profile_sha256\": \"{}\",\n",
            "  \"runtime_id\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"steam_session_sha256\": \"{}\",\n",
            "  \"steam_client_mode\": \"{}\",\n",
            "  \"renderer\": \"{}\",\n",
            "  \"graphics_api\": \"{}\",\n",
            "  \"tracked_game_pid\": {},\n",
            "  \"observed_runtime_seconds\": {},\n",
            "  \"minimum_runtime_seconds\": {},\n",
            "  \"samples\": {},\n",
            "  \"live_pid_samples\": {},\n",
            "  \"peak_exact_prefix_rss_kib\": {},\n",
            "  \"exit_code\": {},\n",
            "  \"remaining_exact_prefix_rss_kib\": {},\n",
            "  \"remaining_exact_prefix_process_count\": {},\n",
            "  \"status\": \"normal-game-exit-pass\",\n",
            "  \"not_inferred\": [\"prefix-fully-stopped\",\"steam-client-exited\",\"crash-free-beyond-observed-duration\",\"gameplay-correctness\",\"human-stability-pass\"]\n",
            "}}\n"
        ),
        captured_unix,
        app_id,
        profile.fingerprint(),
        live.runtime_id,
        live.runtime_fingerprint_sha256,
        live.steam_session_sha256,
        profile.steam_client_mode.as_str(),
        profile.renderer,
        profile.graphics_api.as_str(),
        live.game_pid,
        observed_runtime_seconds,
        min_runtime_seconds,
        samples,
        live_samples,
        peak_rss_kib,
        exit_code,
        remaining_rss_kib,
        remaining_process_count,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!("verified normal-exit evidence: {}", output.display());
    println!(
        "status: normal-game-exit-pass after {observed_runtime_seconds}s; exact-prefix Steam/helper residuals remain a separate stop gate"
    );
    Ok(())
}

fn evidence_verify_audio_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton evidence verify-audio APPID --profile PATH --probe PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut probe_path = None;
    let mut output = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--probe" => probe_path = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown evidence verify-audio option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let probe_path = probe_path.ok_or_else(|| "--probe PATH is required".to_owned())?;
    let output = output.ok_or_else(|| "--output ABSOLUTE_PATH is required".to_owned())?;
    if !output.is_absolute() {
        return Err("--output must be an absolute path".to_owned());
    }
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let live = validate_live_runtime_session(app_id, &profile, &data_root, &runtime_root, "audio")?;
    let probe_contents = fs::read_to_string(&probe_path)
        .map_err(|error| format!("cannot read audio probe result: {error}"))?;
    let probe = parse_audio_probe(&probe_contents)?;
    validate_audio_probe(&probe)?;
    if probe.pid != live.game_pid {
        return Err(format!(
            "audio probe PID {} does not match Steam's live game PID {}",
            probe.pid, live.game_pid
        ));
    }
    let probe_sha256 = sha256::to_hex(
        &sha256::digest_file(&probe_path)
            .map_err(|error| format!("cannot hash audio probe result: {error}"))?,
    );
    let captured_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_secs();
    let record = format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"kind\": \"runtime-audio-presence\",\n",
            "  \"captured_unix\": {},\n",
            "  \"appid\": {},\n",
            "  \"profile_sha256\": \"{}\",\n",
            "  \"runtime_id\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"steam_session_sha256\": \"{}\",\n",
            "  \"steam_client_mode\": \"{}\",\n",
            "  \"renderer\": \"{}\",\n",
            "  \"graphics_api\": \"{}\",\n",
            "  \"probe_sha256\": \"{}\",\n",
            "  \"tracked_game_pid\": {},\n",
            "  \"window_id\": {},\n",
            "  \"duration_seconds\": {},\n",
            "  \"sample_rate\": {},\n",
            "  \"channels\": {},\n",
            "  \"buffers\": {},\n",
            "  \"rms_dbfs\": {},\n",
            "  \"peak_dbfs\": {},\n",
            "  \"non_silent_buffers\": {},\n",
            "  \"non_silent_ratio\": {},\n",
            "  \"status\": \"pcm-presence-pass\",\n",
            "  \"not_inferred\": [\"audible-content\",\"audible-quality\",\"volume-control\",\"dropout-free-playback\",\"human-listening-pass\"]\n",
            "}}\n"
        ),
        captured_unix,
        app_id,
        profile.fingerprint(),
        live.runtime_id,
        live.runtime_fingerprint_sha256,
        live.steam_session_sha256,
        profile.steam_client_mode.as_str(),
        profile.renderer,
        profile.graphics_api.as_str(),
        probe_sha256,
        live.game_pid,
        probe.window_id,
        probe.duration_seconds,
        probe.sample_rate,
        probe.channels,
        probe.buffers,
        probe.rms_dbfs,
        probe.peak_dbfs,
        probe.non_silent_buffers,
        probe.non_silent_ratio,
    );
    write_new_evidence(&output, record.as_bytes())?;
    println!("verified audio evidence: {}", output.display());
    println!(
        "status: pcm-presence-pass for live AppID {app_id} PID {}",
        live.game_pid
    );
    println!("boundary: content, quality, controls and human audibility are not inferred");
    Ok(())
}

fn parse_audio_probe(contents: &str) -> Result<AudioProbeEvidence, String> {
    if contents.lines().next().map(str::trim) != Some("{")
        || contents.lines().last().map(str::trim) != Some("}")
    {
        return Err("audio probe is not a flat JSON object".to_owned());
    }
    let scalar = |key: &str| flat_json_scalar(contents, key);
    let parse_u32 = |key: &str| {
        scalar(key)?
            .parse::<u32>()
            .map_err(|_| format!("audio probe {key} is not an unsigned integer"))
    };
    let parse_u64 = |key: &str| {
        scalar(key)?
            .parse::<u64>()
            .map_err(|_| format!("audio probe {key} is not an unsigned integer"))
    };
    let parse_f64 = |key: &str| {
        scalar(key)?
            .parse::<f64>()
            .map_err(|_| format!("audio probe {key} is not finite numeric evidence"))
            .and_then(|value| {
                value
                    .is_finite()
                    .then_some(value)
                    .ok_or_else(|| format!("audio probe {key} is not finite numeric evidence"))
            })
    };
    if parse_u32("schema")? != 1 {
        return Err("audio probe schema is unsupported".to_owned());
    }
    Ok(AudioProbeEvidence {
        window_id: parse_u32("window_id")?,
        pid: parse_u32("pid")?,
        duration_seconds: parse_f64("duration_seconds")?,
        sample_rate: parse_f64("sample_rate")?,
        channels: parse_u32("channels")?,
        buffers: parse_u64("buffers")?,
        rms_dbfs: parse_f64("rms_dbfs")?,
        peak_dbfs: parse_f64("peak_dbfs")?,
        non_silent_buffers: parse_u64("non_silent_buffers")?,
        non_silent_ratio: parse_f64("non_silent_ratio")?,
        invalid_buffers: parse_u64("invalid_buffers")?,
        format_changes: parse_u64("format_changes")?,
        owner_match: match scalar("owner_match")?.as_str() {
            "true" => true,
            "false" => false,
            _ => return Err("audio probe owner_match is not boolean".to_owned()),
        },
    })
}

fn flat_json_scalar(contents: &str, key: &str) -> Result<String, String> {
    flat_json_scalar_for(contents, key, "audio probe")
}

fn flat_json_scalar_for(contents: &str, key: &str, kind: &str) -> Result<String, String> {
    let key_prefix = format!("\"{key}\"");
    let mut values = contents.lines().filter_map(|line| {
        let value = line.trim().strip_prefix(&key_prefix)?.trim_start();
        let value = value.strip_prefix(':')?;
        Some(value.trim().trim_end_matches(',').trim().to_owned())
    });
    let value = values
        .next()
        .ok_or_else(|| format!("{kind} is missing {key}"))?;
    if values.next().is_some() {
        return Err(format!("{kind} contains duplicate {key}"));
    }
    Ok(value)
}

fn validate_audio_probe(probe: &AudioProbeEvidence) -> Result<(), String> {
    if probe.window_id == 0 || probe.pid == 0 {
        return Err("audio probe window/PID identity is invalid".to_owned());
    }
    if probe.duration_seconds < 0.5
        || probe.sample_rate <= 0.0
        || probe.channels == 0
        || probe.buffers == 0
        || probe.non_silent_buffers == 0
        || probe.non_silent_buffers > probe.buffers
        || !(0.0..=1.0).contains(&probe.non_silent_ratio)
        || probe.invalid_buffers != 0
        || probe.format_changes != 0
        || !probe.owner_match
    {
        return Err("audio probe did not pass bounded PCM presence invariants".to_owned());
    }
    Ok(())
}

fn write_new_evidence(destination: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = destination
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| "evidence output parent directory does not exist".to_owned())?;
    if destination.exists() {
        return Err(format!(
            "refusing to replace existing evidence: {}",
            destination.display()
        ));
    }
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "evidence output requires a UTF-8 file name".to_owned())?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("cannot create evidence temporary file: {error}"))?;
        file.write_all(contents)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("cannot write evidence snapshot: {error}"))?;
        fs::hard_link(&temporary, destination)
            .map_err(|error| format!("cannot commit new evidence snapshot: {error}"))?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn steamcmd_command(args: &[String]) -> Result<(), String> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(
            "usage: macroton steamcmd <login|acquire|status|validate|plan|launch> APPID [options]"
                .to_owned(),
        );
    };
    if action == "status" {
        return steamcmd_status_command(&args[1..]);
    }
    if action == "plan" {
        return steamcmd_plan_command(&args[1..]);
    }
    if action == "validate" {
        return steamcmd_validate_command(&args[1..]);
    }
    if action == "launch" {
        return steamcmd_launch_command(&args[1..]);
    }
    if !matches!(action, "login" | "acquire") {
        return Err(format!("unknown steamcmd action {action:?}"));
    }
    if !std::io::stdin().is_terminal() {
        return Err(
            "interactive SteamCMD requires a real TTY; open a visible Terminal or use scripts/open-steamcmd-login-terminal.sh --print-command"
                .to_owned(),
        );
    }
    let app_id: u32 = args
        .get(1)
        .ok_or_else(|| {
            "usage: macroton steamcmd login APPID --tool-root PATH --runtime-id ID [options]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut tool_root = None;
    let mut runtime_id = None;
    let mut profile_path = None;
    let mut runtime_root = default_runtime_root()?;
    let mut data_root = default_data_root()?;
    let mut max_rss_mib = 500_u64;
    let mut account = None;
    let mut index = 2;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--tool-root" => tool_root = Some(PathBuf::from(value)),
            "--runtime-id" => runtime_id = Some(value.to_owned()),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--data-root" => data_root = PathBuf::from(value),
            "--max-rss-mib" => max_rss_mib = parse_positive_u64(value, option)?,
            "--account" => account = Some(value.to_owned()),
            other => return Err(format!("unknown steamcmd {action} option {other:?}")),
        }
        index += 1;
    }
    let tool_root = tool_root.ok_or_else(|| "--tool-root PATH is required".to_owned())?;
    let runtime_id = runtime_id.ok_or_else(|| "--runtime-id ID is required".to_owned())?;
    let acquire_profile = match action {
        "login" => {
            if profile_path.is_some() {
                return Err("--profile is only valid for steamcmd acquire".to_owned());
            }
            if account.is_some() {
                return Err("--account is only valid for steamcmd acquire".to_owned());
            }
            None
        }
        "acquire" => {
            let profile_path = profile_path
                .as_ref()
                .ok_or_else(|| "steamcmd acquire requires --profile PATH".to_owned())?;
            let profile =
                CompatibilityProfile::load(profile_path).map_err(|error| error.to_string())?;
            if profile.app_id != app_id {
                return Err(format!(
                    "profile AppID {} does not match requested AppID {app_id}",
                    profile.app_id
                ));
            }
            Some(profile)
        }
        _ => unreachable!(),
    };
    if let Some(profile) = &acquire_profile {
        let profile_runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
        if runtime_id != profile_runtime_id {
            return Err(format!(
                "steamcmd acquire runtime {runtime_id} does not match profile runtime {profile_runtime_id}"
            ));
        }
    }
    steamcmd::validate_tool_root(&tool_root).map_err(|error| error.to_string())?;
    runtime::resolve_installed(&runtime_root, &runtime_id).map_err(|error| error.to_string())?;
    let max_runtime_seconds = (action == "login").then_some(30 * 60);
    let (guard_pid, guard_log) = spawn_resource_guard(
        app_id,
        &data_root,
        &runtime_root,
        &runtime_id,
        max_rss_mib,
        None,
        max_runtime_seconds,
        None,
        None,
    )?;
    println!("SteamCMD interactive {action} session for AppID {app_id}");
    println!(
        "resource guard: PID {guard_pid}, limit {max_rss_mib} MiB, log {}",
        guard_log.display()
    );
    if action == "acquire" && account.is_some() {
        println!(
            "SteamCMD will queue login, app_update and quit; enter the password only at its hidden prompt."
        );
    } else {
        println!("At the Steam> prompt, type: login ACCOUNT_NAME");
    }
    println!(
        "Enter the password only at SteamCMD's hidden prompt and complete Steam Guard if requested."
    );
    println!("Macroton does not accept or place a password on the process command line.");
    match action {
        "login" => {
            println!("After Login Logged in OK, type: quit");
            let result = steamcmd::interactive_login(
                app_id,
                &tool_root,
                &data_root,
                &runtime_root,
                &runtime_id,
            )
            .map_err(|error| error.to_string())?;
            println!(
                "SteamCMD exited with {}; authenticated state, if Valve accepted it, remains in the private tool/prefix roots",
                result
                    .exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".to_owned())
            );
            match result.exit_code {
                Some(0) => Ok(()),
                Some(code) => Err(format!("SteamCMD login exited with status {code}")),
                None => Err("SteamCMD login terminated by signal".to_owned()),
            }
        }
        "acquire" => {
            let profile = acquire_profile.as_ref().expect("profile validated above");
            let (expected_build_id, expected_depots) = profile_manifest_expectations(profile)?;
            println!("After Login Logged in OK, type: app_update {app_id} validate");
            println!("Wait for Success! App '{app_id}' fully installed, then type: quit");
            let profile_fingerprint = profile.fingerprint();
            let result = steamcmd::interactive_acquire(&steamcmd::AcquireOptions {
                app_id,
                relative_executable: &profile.executable,
                profile_fingerprint: &profile_fingerprint,
                expected_build_id,
                expected_depots: &expected_depots,
                tool_root: &tool_root,
                data_root: &data_root,
                runtime_root: &runtime_root,
                runtime_id: &runtime_id,
                account: account.as_deref(),
            })
            .map_err(|error| error.to_string())?;
            println!("Acquired:    {}", result.executable.display());
            println!("Architecture: {:?}", result.architecture);
            println!("SHA-256:     {}", result.executable_sha256);
            println!(
                "BuildID:      {}",
                result
                    .build_id
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "missing".to_owned())
            );
            println!(
                "Depots:       {}",
                result
                    .installed_depots
                    .iter()
                    .map(|(id, manifest)| format!("{id}:{manifest}"))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            println!(
                "Graphics:    {}",
                if result.graphics.apis.is_empty() {
                    "none detected".to_owned()
                } else {
                    result
                        .graphics
                        .apis
                        .iter()
                        .map(|api| api.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                }
            );
            println!(
                "Input:       {}",
                display_values(result.compatibility.input.iter().map(|api| api.as_str()))
            );
            println!(
                "Audio:       {}",
                display_values(result.compatibility.audio.iter().map(|api| api.as_str()))
            );
            println!(
                "Prereqs:     {}",
                display_values(
                    result
                        .compatibility
                        .prerequisites
                        .iter()
                        .map(|item| item.as_str())
                )
            );
            println!("Evidence:    {}", result.marker.display());
            if result.manifest_matches_profile {
                Ok(())
            } else {
                Err("SteamCMD installed manifest does not exactly match the profile BuildID and depot manifests; keep the profile experimental and record the observed set".to_owned())
            }
        }
        _ => unreachable!(),
    }
}

fn steamcmd_plan_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steamcmd plan APPID --profile PATH [--data-root PATH] [--runtime-root PATH]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown steamcmd plan option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let launch_plan = build_steamcmd_launch_plan(app_id, &profile, &data_root, &runtime_root)?;
    print_launch_plan(&launch_plan, "macroton steamcmd launch");
    Ok(())
}

fn steamcmd_launch_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steamcmd launch APPID --profile PATH [--data-root PATH] [--runtime-root PATH] [--max-rss-mib N]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut max_rss_mib = 1600_u64;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--max-rss-mib" => max_rss_mib = parse_positive_u64(value, option)?,
            other => return Err(format!("unknown steamcmd launch option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let launch_plan = build_steamcmd_launch_plan(app_id, &profile, &data_root, &runtime_root)?;
    if !launch_plan.is_ready() {
        return Err(format!(
            "SteamCMD launch is blocked: {}",
            launch_plan.blockers.join("; ")
        ));
    }
    let result = execute::execute(&launch_plan, &data_root).map_err(|error| error.to_string())?;
    println!("SteamCMD-acquired game launch completed/submitted for AppID {app_id}");
    println!("Result:         {}", result.result_path.display());
    if result.launch_mode == macroton::profile::LaunchMode::Steam {
        let (guard_pid, guard_log) = spawn_resource_guard(
            app_id,
            &data_root,
            &runtime_root,
            &result.runtime_id,
            max_rss_mib,
            None,
            None,
            Some(&resource_guard_identity(
                &launch_plan,
                &result.runtime_fingerprint_sha256,
            )),
            profile.executable.to_str(),
        )?;
        println!(
            "Resource guard: PID {guard_pid}, limit {max_rss_mib} MiB, log {}",
            guard_log.display()
        );
    }
    Ok(())
}

fn build_steamcmd_launch_plan(
    app_id: u32,
    profile: &CompatibilityProfile,
    data_root: &Path,
    runtime_root: &Path,
) -> Result<plan::LaunchPlan, String> {
    let (expected_build_id, expected_depots) = profile_manifest_expectations(profile)?;
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    let installed_runtime = runtime::resolve_installed(runtime_root, &runtime_id).ok();
    let status = steamcmd::inspect_acquisition(
        app_id,
        &profile.executable,
        &profile.fingerprint(),
        expected_build_id,
        &expected_depots,
        steamcmd::AcquisitionProvenance {
            runtime_id: &runtime_id,
            runtime_fingerprint_sha256: installed_runtime
                .as_ref()
                .map(|runtime| runtime.fingerprint_sha256.as_str()),
        },
        data_root,
    )
    .map_err(|error| error.to_string())?;
    let game = steamcmd::acquired_game(&status, profile).map_err(|error| error.to_string())?;
    let mut launch_plan = plan::build(&game, profile, runtime_root);
    if profile.launch_mode == macroton::profile::LaunchMode::Steam {
        let prefix =
            steam_prefix_status::inspect(app_id, data_root).map_err(|error| error.to_string())?;
        if prefix.readiness == steam_prefix_status::Readiness::NotBootstrapped {
            launch_plan.blockers.push(
                "Windows Steam launch/Steamworks client is not safely bootstrapped in this AppID prefix"
                    .to_owned(),
            );
        }
    }
    Ok(launch_plan)
}

fn steamcmd_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steamcmd status APPID --profile PATH [--data-root PATH] [--runtime-root PATH] [--json]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown steamcmd status option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let (expected_build_id, expected_depots) = profile_manifest_expectations(&profile)?;
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    let installed_runtime = runtime::resolve_installed(&runtime_root, &runtime_id).ok();
    let status = steamcmd::inspect_acquisition(
        app_id,
        &profile.executable,
        &profile.fingerprint(),
        expected_build_id,
        &expected_depots,
        steamcmd::AcquisitionProvenance {
            runtime_id: &runtime_id,
            runtime_fingerprint_sha256: installed_runtime
                .as_ref()
                .map(|runtime| runtime.fingerprint_sha256.as_str()),
        },
        &data_root,
    )
    .map_err(|error| error.to_string())?;
    let architecture = status
        .architecture
        .map(|architecture| format!("{architecture:?}").to_ascii_lowercase());
    let graphics = status
        .graphics
        .as_ref()
        .map(|evidence| {
            evidence
                .apis
                .iter()
                .map(|api| api.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let graphics_display = graphics.join(",");
    let input = status
        .compatibility
        .as_ref()
        .map(|evidence| {
            evidence
                .input
                .iter()
                .map(|api| api.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let audio = status
        .compatibility
        .as_ref()
        .map(|evidence| {
            evidence
                .audio
                .iter()
                .map(|api| api.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let prerequisites = status
        .compatibility
        .as_ref()
        .map(|evidence| {
            evidence
                .prerequisites
                .iter()
                .map(|item| item.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if json {
        println!("{{");
        println!("  \"schema\": 1,");
        println!("  \"appid\": {app_id},");
        println!("  \"state\": \"{}\",", status.state.as_str());
        match architecture.as_deref() {
            Some(value) => println!("  \"architecture\": \"{}\",", json_escape(value)),
            None => println!("  \"architecture\": null,"),
        }
        println!(
            "  \"graphics_apis\": [{}],",
            graphics
                .iter()
                .map(|api| format!("\"{}\"", json_escape(api)))
                .collect::<Vec<_>>()
                .join(",")
        );
        println!("  \"input_apis\": [{}],", json_string_array(&input));
        println!("  \"audio_apis\": [{}],", json_string_array(&audio));
        println!(
            "  \"prerequisites\": [{}],",
            json_string_array(&prerequisites)
        );
        match status.executable_sha256.as_deref() {
            Some(value) => println!("  \"executable_sha256\": \"{}\",", json_escape(value)),
            None => println!("  \"executable_sha256\": null,"),
        }
        println!(
            "  \"drift_reasons\": [{}],",
            status
                .drift_reasons
                .iter()
                .map(|reason| format!("\"{}\"", json_escape(reason)))
                .collect::<Vec<_>>()
                .join(",")
        );
        println!("  \"detail\": \"{}\"", json_escape(&status.detail));
        println!("}}");
    } else {
        println!("AppID:       {app_id}");
        println!("State:       {}", status.state.as_str());
        println!("Executable:  {}", status.executable.display());
        println!(
            "Architecture: {}",
            architecture.as_deref().unwrap_or("unknown")
        );
        println!(
            "Graphics:    {}",
            if graphics_display.is_empty() {
                "none detected"
            } else {
                &graphics_display
            }
        );
        println!("Input:       {}", display_values(input.iter().copied()));
        println!("Audio:       {}", display_values(audio.iter().copied()));
        println!(
            "Prereqs:     {}",
            display_values(prerequisites.iter().copied())
        );
        println!(
            "Drift:       {}",
            if status.drift_reasons.is_empty() {
                "none".to_owned()
            } else {
                status.drift_reasons.join(",")
            }
        );
        println!("Detail:      {}", status.detail);
    }
    Ok(())
}

fn steamcmd_validate_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steamcmd validate APPID --profile PATH [--data-root PATH] [--runtime-root PATH] [--json]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown steamcmd validate option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    let (expected_build_id, expected_depots) = profile_manifest_expectations(&profile)?;
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    let installed_runtime = runtime::resolve_installed(&runtime_root, &runtime_id).ok();
    let acquisition = steamcmd::inspect_acquisition(
        app_id,
        &profile.executable,
        &profile.fingerprint(),
        expected_build_id,
        &expected_depots,
        steamcmd::AcquisitionProvenance {
            runtime_id: &runtime_id,
            runtime_fingerprint_sha256: installed_runtime
                .as_ref()
                .map(|runtime| runtime.fingerprint_sha256.as_str()),
        },
        &data_root,
    )
    .map_err(|error| error.to_string())?;
    let prefix =
        steam_prefix_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let prerequisites =
        prerequisite_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let steam_client_required = profile.launch_mode == macroton::profile::LaunchMode::Steam;
    let steam_client_bootstrapped =
        prefix.readiness != steam_prefix_status::Readiness::NotBootstrapped;
    let state = steamcmd_validation_state(
        acquisition.state,
        steam_client_required,
        prefix.readiness,
        prerequisites.state,
    );
    let ready_for_launch = matches!(
        state,
        "ready-for-first-launch"
            | "prerequisite-installing"
            | "prerequisite-passed"
            | "no-prerequisite-required"
    );
    if json {
        println!("{{");
        println!("  \"schema\": 1,");
        println!("  \"appid\": {app_id},");
        println!("  \"state\": \"{state}\",");
        println!("  \"ready_for_launch\": {ready_for_launch},");
        println!("  \"acquisition\": \"{}\",", acquisition.state.as_str());
        println!("  \"steam_client_required\": {steam_client_required},");
        println!("  \"steam_client_bootstrapped\": {steam_client_bootstrapped},");
        println!("  \"steam_prefix\": \"{}\",", prefix.readiness.as_str());
        println!("  \"prerequisites\": \"{}\",", prerequisites.state.as_str());
        println!(
            "  \"game_process_observed\": {}",
            prerequisites.state == prerequisite_status::State::PassedToGame
        );
        println!("}}");
    } else {
        println!("AppID:            {app_id}");
        println!("State:            {state}");
        println!("Ready for launch: {ready_for_launch}");
        println!("Acquisition:      {}", acquisition.state.as_str());
        println!("Steam prefix:     {}", prefix.readiness.as_str());
        println!("Prerequisites:    {}", prerequisites.state.as_str());
        println!(
            "Boundary:         stored login is local metadata only; this report does not prove online Steamworks, gameplay, audio, input, save, or Cloud"
        );
    }
    Ok(())
}

fn steamcmd_validation_state(
    acquisition: steamcmd::AcquisitionState,
    steam_client_required: bool,
    prefix: steam_prefix_status::Readiness,
    prerequisites: prerequisite_status::State,
) -> &'static str {
    if acquisition != steamcmd::AcquisitionState::Ready {
        return "blocked-acquisition";
    }
    if steam_client_required && prefix == steam_prefix_status::Readiness::NotBootstrapped {
        return "blocked-steam-client";
    }
    match prerequisites {
        prerequisite_status::State::SignatureMissing => "blocked-prerequisite-integrity",
        prerequisite_status::State::Pending => "ready-for-first-launch",
        prerequisite_status::State::Installing => "prerequisite-installing",
        prerequisite_status::State::PassedToGame => "prerequisite-passed",
        prerequisite_status::State::NoScript => "no-prerequisite-required",
    }
}

fn display_values<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let values = values.collect::<Vec<_>>();
    if values.is_empty() {
        "none observed".to_owned()
    } else {
        values.join(",")
    }
}

fn json_string_array(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| format!("\"{}\"", json_escape(value)))
        .collect::<Vec<_>>()
        .join(",")
}

fn profile_manifest_expectations(
    profile: &CompatibilityProfile,
) -> Result<(Option<u64>, BTreeMap<u32, u64>), String> {
    let build_id = profile
        .windows_build_id
        .as_deref()
        .map(|value| {
            value
                .parse()
                .map_err(|_| "profile windows_build_id is not an unsigned integer".to_owned())
        })
        .transpose()?;
    let mut depots = BTreeMap::new();
    for depot in &profile.windows_depots {
        let manifest = depot.manifest_id.ok_or_else(|| {
            format!(
                "steamcmd acquisition requires a pinned manifest for depot {}",
                depot.id
            )
        })?;
        depots.insert(depot.id, manifest);
    }
    if depots.is_empty() {
        return Err("steamcmd acquisition requires profile windows_depots".to_owned());
    }
    Ok((build_id, depots))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResourcePeakRecord {
    app_id: u32,
    observed_milliseconds: u128,
    samples: u64,
    observed_steam_process: bool,
    observed_game_process: bool,
    max_consecutive_steam_samples: u64,
    max_consecutive_game_samples: u64,
    interval_seconds: u64,
    reason: String,
    evidence_scope: String,
    profile_sha256: String,
    runtime_id: String,
    runtime_fingerprint_sha256: String,
    steam_client_mode: String,
    renderer: String,
    graphics_api: String,
    steam_session_sha256: String,
    peak_rss_kib: u64,
    limit_rss_kib: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceComparePhase {
    Client,
    Game,
}

impl ResourceComparePhase {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "client" => Ok(Self::Client),
            "game" => Ok(Self::Game),
            _ => Err("--phase must be client or game".to_owned()),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Game => "game",
        }
    }

    fn default_min_seconds(self) -> u64 {
        match self {
            Self::Client => 60,
            Self::Game => 300,
        }
    }
}

fn resource_compare_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton resource compare APPID --standard PATH --low-memory PATH --standard-profile PATH --low-memory-profile PATH --phase <client|game> [--min-seconds N] [--output PATH]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut standard_path = None;
    let mut low_memory_path = None;
    let mut phase = None;
    let mut standard_profile_path = None;
    let mut low_memory_profile_path = None;
    let mut min_seconds = None;
    let mut output = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--standard" => standard_path = Some(PathBuf::from(value)),
            "--low-memory" => low_memory_path = Some(PathBuf::from(value)),
            "--standard-profile" => standard_profile_path = Some(PathBuf::from(value)),
            "--low-memory-profile" => low_memory_profile_path = Some(PathBuf::from(value)),
            "--phase" => phase = Some(ResourceComparePhase::parse(value)?),
            "--min-seconds" => min_seconds = Some(parse_positive_u64(value, option)?),
            "--output" => output = Some(PathBuf::from(value)),
            other => return Err(format!("unknown resource compare option {other:?}")),
        }
        index += 1;
    }
    let standard_path = standard_path.ok_or_else(|| "--standard PATH is required".to_owned())?;
    let low_memory_path =
        low_memory_path.ok_or_else(|| "--low-memory PATH is required".to_owned())?;
    let standard_profile_path =
        standard_profile_path.ok_or_else(|| "--standard-profile PATH is required".to_owned())?;
    let low_memory_profile_path = low_memory_profile_path
        .ok_or_else(|| "--low-memory-profile PATH is required".to_owned())?;
    if standard_path == low_memory_path {
        return Err("resource comparison requires two different evidence files".to_owned());
    }
    let phase = phase.ok_or_else(|| "--phase <client|game> is required".to_owned())?;
    let min_seconds = min_seconds.unwrap_or_else(|| phase.default_min_seconds());
    let standard = read_resource_peak_record(&standard_path)?;
    let low_memory = read_resource_peak_record(&low_memory_path)?;
    let standard_profile =
        CompatibilityProfile::load(&standard_profile_path).map_err(|error| error.to_string())?;
    let low_memory_profile =
        CompatibilityProfile::load(&low_memory_profile_path).map_err(|error| error.to_string())?;
    validate_resource_comparison(app_id, phase, min_seconds, &standard, &low_memory)?;
    validate_resource_comparison_profiles(
        app_id,
        &standard_profile,
        &low_memory_profile,
        &standard,
        &low_memory,
    )?;
    let standard_sha256 = sha256::to_hex(
        &sha256::digest_file(&standard_path)
            .map_err(|error| format!("cannot hash Standard evidence: {error}"))?,
    );
    let low_memory_sha256 = sha256::to_hex(
        &sha256::digest_file(&low_memory_path)
            .map_err(|error| format!("cannot hash LowMemory evidence: {error}"))?,
    );
    let delta_kib = i128::from(low_memory.peak_rss_kib) - i128::from(standard.peak_rss_kib);
    let change_percent = if standard.peak_rss_kib == 0 {
        return Err("Standard evidence has zero peak RSS".to_owned());
    } else {
        (delta_kib as f64 / standard.peak_rss_kib as f64) * 100.0
    };
    let verdict = if delta_kib < 0 {
        "low-memory-lower"
    } else if delta_kib > 0 {
        "low-memory-higher"
    } else {
        "equal"
    };
    let captured_unix_ms = unix_time_ms()?;
    let record = format!(
        concat!(
            "{{\n  \"schema\": 1,\n  \"captured_unix_ms\": {captured_unix_ms},\n",
            "  \"appid\": {app_id},\n  \"phase\": \"{}\",\n  \"min_seconds\": {min_seconds},\n",
            "  \"runtime_id\": \"{}\",\n  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"renderer\": \"{}\",\n  \"graphics_api\": \"{}\",\n",
            "  \"standard_profile_sha256\": \"{}\",\n  \"standard_session_sha256\": \"{}\",\n",
            "  \"standard_evidence_sha256\": \"{standard_sha256}\",\n  \"standard_peak_rss_kib\": {},\n",
            "  \"low_memory_profile_sha256\": \"{}\",\n  \"low_memory_session_sha256\": \"{}\",\n",
            "  \"low_memory_evidence_sha256\": \"{low_memory_sha256}\",\n  \"low_memory_peak_rss_kib\": {},\n",
            "  \"delta_rss_kib\": {delta_kib},\n  \"change_percent\": {change_percent:.3},\n",
            "  \"verdict\": \"{verdict}\"\n}}\n"
        ),
        phase.as_str(),
        json_escape(&standard.runtime_id),
        standard.runtime_fingerprint_sha256,
        json_escape(&standard.renderer),
        json_escape(&standard.graphics_api),
        standard.profile_sha256,
        standard.steam_session_sha256,
        standard.peak_rss_kib,
        low_memory.profile_sha256,
        low_memory.steam_session_sha256,
        low_memory.peak_rss_kib,
        captured_unix_ms = captured_unix_ms,
        app_id = app_id,
        min_seconds = min_seconds,
        standard_sha256 = standard_sha256,
        low_memory_sha256 = low_memory_sha256,
        delta_kib = delta_kib,
        change_percent = change_percent,
        verdict = verdict,
    );
    if let Some(output) = output {
        write_new_evidence(&output, record.as_bytes())?;
        println!("resource comparison evidence: {}", output.display());
    }
    print!("{record}");
    Ok(())
}

fn read_resource_peak_record(path: &Path) -> Result<ResourcePeakRecord, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read resource evidence {}: {error}", path.display()))?;
    parse_resource_peak_record(&contents)
        .map_err(|error| format!("invalid resource evidence {}: {error}", path.display()))
}

fn parse_resource_peak_record(contents: &str) -> Result<ResourcePeakRecord, String> {
    let allowed = [
        "schema",
        "captured_unix_ms",
        "started_unix_ms",
        "observed_milliseconds",
        "samples",
        "observed_steam_process",
        "observed_game_process",
        "max_consecutive_steam_samples",
        "max_consecutive_game_samples",
        "appid",
        "interval_seconds",
        "reason",
        "evidence_scope",
        "profile_sha256",
        "runtime_id",
        "runtime_fingerprint_sha256",
        "steam_client_mode",
        "renderer",
        "graphics_api",
        "steam_session_sha256",
        "peak_rss_kib",
        "limit_rss_kib",
        "categories",
        "processes",
    ];
    let mut fields = BTreeMap::<String, String>::new();
    for line in contents.lines() {
        let line = line.trim();
        if !line.starts_with('"') {
            continue;
        }
        let (raw_key, raw_value) = line
            .split_once(':')
            .ok_or_else(|| "malformed top-level field".to_owned())?;
        let key = raw_key.trim().trim_matches('"');
        if !allowed.contains(&key) {
            return Err(format!("unknown field {key:?}"));
        }
        if matches!(key, "categories" | "processes") {
            continue;
        }
        let value = raw_value.trim().trim_end_matches(',').trim().to_owned();
        if fields.insert(key.to_owned(), value).is_some() {
            return Err(format!("duplicate field {key:?}"));
        }
    }
    let value = |key: &str| {
        fields
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| format!("missing field {key:?}"))
    };
    if value("schema")? != "3" {
        return Err("resource evidence must use schema 3".to_owned());
    }
    Ok(ResourcePeakRecord {
        app_id: parse_json_u64(value("appid")?, "appid")?
            .try_into()
            .map_err(|_| "appid is out of range".to_owned())?,
        observed_milliseconds: value("observed_milliseconds")?
            .parse()
            .map_err(|_| "invalid observed_milliseconds".to_owned())?,
        samples: parse_json_u64(value("samples")?, "samples")?,
        observed_steam_process: parse_json_bool(
            value("observed_steam_process")?,
            "observed_steam_process",
        )?,
        observed_game_process: parse_json_bool(
            value("observed_game_process")?,
            "observed_game_process",
        )?,
        max_consecutive_steam_samples: parse_json_u64(
            value("max_consecutive_steam_samples")?,
            "max_consecutive_steam_samples",
        )?,
        max_consecutive_game_samples: parse_json_u64(
            value("max_consecutive_game_samples")?,
            "max_consecutive_game_samples",
        )?,
        interval_seconds: parse_json_u64(value("interval_seconds")?, "interval_seconds")?,
        reason: parse_json_string(value("reason")?, "reason")?,
        evidence_scope: parse_json_string(value("evidence_scope")?, "evidence_scope")?,
        profile_sha256: parse_json_nullable_string(value("profile_sha256")?, "profile_sha256")?,
        runtime_id: parse_json_nullable_string(value("runtime_id")?, "runtime_id")?,
        runtime_fingerprint_sha256: parse_json_nullable_string(
            value("runtime_fingerprint_sha256")?,
            "runtime_fingerprint_sha256",
        )?,
        steam_client_mode: parse_json_nullable_string(
            value("steam_client_mode")?,
            "steam_client_mode",
        )?,
        renderer: parse_json_nullable_string(value("renderer")?, "renderer")?,
        graphics_api: parse_json_nullable_string(value("graphics_api")?, "graphics_api")?,
        steam_session_sha256: parse_json_nullable_string(
            value("steam_session_sha256")?,
            "steam_session_sha256",
        )?,
        peak_rss_kib: parse_json_u64(value("peak_rss_kib")?, "peak_rss_kib")?,
        limit_rss_kib: parse_json_u64(value("limit_rss_kib")?, "limit_rss_kib")?,
    })
}

fn parse_json_string(value: &str, field: &str) -> Result<String, String> {
    let stripped = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| format!("{field} must be a JSON string"))?;
    if stripped.contains(['\\', '"']) {
        return Err(format!("{field} contains unsupported escaping"));
    }
    Ok(stripped.to_owned())
}

fn parse_json_nullable_string(value: &str, field: &str) -> Result<String, String> {
    if value == "null" {
        Ok(String::new())
    } else {
        parse_json_string(value, field)
    }
}

fn parse_json_u64(value: &str, field: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|_| format!("{field} must be an unsigned integer"))
}

fn parse_json_bool(value: &str, field: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{field} must be a boolean")),
    }
}

fn validate_resource_comparison(
    app_id: u32,
    phase: ResourceComparePhase,
    min_seconds: u64,
    standard: &ResourcePeakRecord,
    low_memory: &ResourcePeakRecord,
) -> Result<(), String> {
    for (label, record, expected_mode) in [
        ("Standard", standard, "standard"),
        ("LowMemory", low_memory, "low-memory-experimental"),
    ] {
        if record.app_id != app_id {
            return Err(format!("{label} evidence AppID does not match {app_id}"));
        }
        if record.evidence_scope != "steam-profile-session" {
            return Err(format!(
                "{label} evidence is not bound to a Steam profile session"
            ));
        }
        if record.steam_client_mode != expected_mode {
            return Err(format!("{label} evidence has the wrong Steam client mode"));
        }
        if !record.observed_steam_process {
            return Err(format!("{label} evidence never observed Windows Steam"));
        }
        if phase == ResourceComparePhase::Game && !record.observed_game_process {
            return Err(format!(
                "{label} evidence never observed a Windows game process"
            ));
        }
        let consecutive_samples = match phase {
            ResourceComparePhase::Client => record.max_consecutive_steam_samples,
            ResourceComparePhase::Game => record.max_consecutive_game_samples,
        };
        let consecutive_seconds = consecutive_samples
            .saturating_sub(1)
            .saturating_mul(record.interval_seconds);
        if consecutive_seconds < min_seconds {
            return Err(format!(
                "{label} evidence has only {consecutive_seconds} continuous {phase_name} seconds",
                phase_name = phase.as_str()
            ));
        }
        if record.observed_milliseconds < u128::from(min_seconds) * 1000 {
            return Err(format!(
                "{label} evidence is shorter than {min_seconds} seconds"
            ));
        }
        if !matches!(record.reason.as_str(), "prefix-idle" | "time-limit") {
            return Err(format!(
                "{label} evidence ended with non-comparable reason {:?}",
                record.reason
            ));
        }
        if record.samples < 2 || record.peak_rss_kib == 0 || record.interval_seconds == 0 {
            return Err(format!(
                "{label} evidence has insufficient resource samples"
            ));
        }
    }
    if standard.profile_sha256 == low_memory.profile_sha256 {
        return Err("Standard and LowMemory evidence must use distinct profiles".to_owned());
    }
    if standard.steam_session_sha256 == low_memory.steam_session_sha256 {
        return Err("Standard and LowMemory evidence must use distinct Steam sessions".to_owned());
    }
    for (name, left, right) in [
        (
            "runtime ID",
            standard.runtime_id.as_str(),
            low_memory.runtime_id.as_str(),
        ),
        (
            "runtime fingerprint",
            standard.runtime_fingerprint_sha256.as_str(),
            low_memory.runtime_fingerprint_sha256.as_str(),
        ),
        (
            "renderer",
            standard.renderer.as_str(),
            low_memory.renderer.as_str(),
        ),
        (
            "graphics API",
            standard.graphics_api.as_str(),
            low_memory.graphics_api.as_str(),
        ),
    ] {
        if left != right {
            return Err(format!("resource comparison changed {name}"));
        }
    }
    if standard.limit_rss_kib != low_memory.limit_rss_kib
        || standard.interval_seconds != low_memory.interval_seconds
        || standard.reason != low_memory.reason
    {
        return Err(
            "resource comparison changed guard limits, cadence, or terminal reason".to_owned(),
        );
    }
    let duration_delta = standard
        .observed_milliseconds
        .abs_diff(low_memory.observed_milliseconds);
    let duration_allowance =
        (standard.observed_milliseconds / 10).max(u128::from(standard.interval_seconds) * 2000);
    if duration_delta > duration_allowance {
        return Err("resource comparison observation durations differ by more than 10%".to_owned());
    }
    let sample_delta = standard.samples.abs_diff(low_memory.samples);
    let sample_allowance = (standard.samples / 10).max(2);
    if sample_delta > sample_allowance {
        return Err("resource comparison sample counts differ by more than 10%".to_owned());
    }
    Ok(())
}

fn validate_resource_comparison_profiles(
    app_id: u32,
    standard_profile: &CompatibilityProfile,
    low_memory_profile: &CompatibilityProfile,
    standard: &ResourcePeakRecord,
    low_memory: &ResourcePeakRecord,
) -> Result<(), String> {
    if standard_profile.app_id != app_id || low_memory_profile.app_id != app_id {
        return Err("resource comparison profile AppID mismatch".to_owned());
    }
    if standard_profile.steam_client_mode.as_str() != "standard"
        || low_memory_profile.steam_client_mode.as_str() != "low-memory-experimental"
    {
        return Err("resource comparison profiles have the wrong client modes".to_owned());
    }
    if standard_profile.fingerprint() != standard.profile_sha256 {
        return Err("Standard evidence does not match the supplied profile fingerprint".to_owned());
    }
    if low_memory_profile.fingerprint() != low_memory.profile_sha256 {
        return Err(
            "LowMemory evidence does not match the supplied profile fingerprint".to_owned(),
        );
    }
    let mut normalized_low_memory = low_memory_profile.clone();
    normalized_low_memory.steam_client_mode = standard_profile.steam_client_mode;
    if normalized_low_memory != *standard_profile {
        return Err("resource comparison profiles differ beyond steam_client_mode".to_owned());
    }
    Ok(())
}

fn resource_command(args: &[String]) -> Result<(), String> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(
            "usage: macroton resource <status|status-all|guard|compare> [APPID] [options]"
                .to_owned(),
        );
    };
    if action == "compare" {
        return resource_compare_command(&args[1..]);
    }
    if action == "status-all" {
        return resource_status_all_command(&args[1..]);
    }
    let app_id: u32 = args
        .get(1)
        .ok_or_else(|| {
            "usage: macroton resource <status|guard|compare> APPID [options]".to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut max_rss_mib = 1600_u64;
    let mut hard_rss_mib = None;
    let mut interval_seconds = 5_u64;
    let mut startup_grace_seconds = 30_u64;
    let mut max_runtime_seconds = None;
    let mut runtime_id = None;
    let mut profile_sha256 = None;
    let mut runtime_fingerprint_sha256 = None;
    let mut steam_client_mode = None;
    let mut renderer = None;
    let mut graphics_api = None;
    let mut steam_session_sha256 = None;
    let mut game_executable = None;
    let mut keep_idle = false;
    let mut json = false;
    let mut index = 2;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        if option == "--keep-idle" {
            keep_idle = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--runtime-id" => runtime_id = Some(value.to_owned()),
            "--profile-sha256" => profile_sha256 = Some(value.to_owned()),
            "--runtime-fingerprint-sha256" => runtime_fingerprint_sha256 = Some(value.to_owned()),
            "--steam-client-mode" => steam_client_mode = Some(value.to_owned()),
            "--renderer" => renderer = Some(value.to_owned()),
            "--graphics-api" => graphics_api = Some(value.to_owned()),
            "--steam-session-sha256" => steam_session_sha256 = Some(value.to_owned()),
            "--game-executable" => game_executable = Some(value.to_owned()),
            "--max-rss-mib" => {
                max_rss_mib = parse_positive_u64(value, option)?;
            }
            "--hard-rss-mib" => {
                hard_rss_mib = Some(parse_positive_u64(value, option)?);
            }
            "--interval-seconds" => {
                interval_seconds = parse_positive_u64(value, option)?;
            }
            "--startup-grace-seconds" => {
                startup_grace_seconds = parse_positive_u64(value, option)?;
            }
            "--max-runtime-seconds" => {
                max_runtime_seconds = Some(parse_positive_u64(value, option)?);
            }
            other => return Err(format!("unknown resource option {other:?}")),
        }
        index += 1;
    }

    match action {
        "status" => {
            if hard_rss_mib.is_some() {
                return Err("--hard-rss-mib is only valid for resource guard".to_owned());
            }
            let usage =
                stop::inspect_resources(app_id, &data_root).map_err(|error| error.to_string())?;
            if json {
                print_resource_usage_json(app_id, &usage, max_rss_mib);
            } else {
                print_resource_usage(app_id, &usage, max_rss_mib);
                for process in &usage.processes {
                    println!(
                        "  PID {:>6} {:>8.1} MiB {:<13} {}",
                        process.pid,
                        process.rss_kib as f64 / 1024.0,
                        process.category,
                        process.command
                    );
                }
            }
            Ok(())
        }
        "guard" => {
            if json {
                return Err("--json is only valid for resource status".to_owned());
            }
            if hard_rss_mib.is_some_and(|hard| hard < max_rss_mib) {
                return Err("--hard-rss-mib must be at least --max-rss-mib".to_owned());
            }
            let provenance_values = [
                profile_sha256.as_ref(),
                runtime_fingerprint_sha256.as_ref(),
                steam_client_mode.as_ref(),
                renderer.as_ref(),
                graphics_api.as_ref(),
                steam_session_sha256.as_ref(),
            ];
            let provenance = if provenance_values.iter().all(|value| value.is_none()) {
                None
            } else if provenance_values.iter().any(|value| value.is_none()) {
                return Err(
                    "resource guard Steam provenance options must be supplied together".to_owned(),
                );
            } else {
                let runtime_id = runtime_id
                    .as_ref()
                    .ok_or_else(|| "attributed resource guard requires --runtime-id".to_owned())?;
                Some(ResourceGuardProvenance {
                    identity: steam_session::Identity {
                        app_id,
                        profile_sha256: profile_sha256.unwrap(),
                        runtime_id: runtime_id.clone(),
                        runtime_fingerprint_sha256: runtime_fingerprint_sha256.unwrap(),
                        steam_client_mode: steam_client_mode.unwrap(),
                        renderer: renderer.unwrap(),
                        graphics_api: graphics_api.unwrap(),
                    },
                    steam_session_sha256: steam_session_sha256.unwrap(),
                })
            };
            resource_guard(&ResourceGuardOptions {
                app_id,
                data_root: &data_root,
                runtime_root: &runtime_root,
                max_rss_mib,
                hard_rss_mib,
                interval_seconds,
                startup_grace_seconds,
                max_runtime_seconds,
                runtime_id: runtime_id.as_deref(),
                provenance: provenance.as_ref(),
                game_executable: game_executable.as_deref(),
                keep_idle,
            })
        }
        other => Err(format!("unknown resource action {other:?}")),
    }
}

fn resource_status_all_command(args: &[String]) -> Result<(), String> {
    let mut data_root = default_data_root()?;
    let mut max_total_rss_mib = 4096_u64;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--max-total-rss-mib" => {
                max_total_rss_mib = parse_positive_u64(value, option)?;
            }
            other => return Err(format!("unknown resource status-all option {other:?}")),
        }
        index += 1;
    }
    let games_root = data_root.join("games");
    let directories = fs::read_dir(&games_root).map_err(|error| {
        format!(
            "cannot inspect Macroton games root {}: {error}",
            games_root.display()
        )
    })?;
    let mut app_ids = Vec::new();
    for directory in directories {
        let directory = directory.map_err(|error| {
            format!(
                "cannot inspect Macroton games root {}: {error}",
                games_root.display()
            )
        })?;
        if !directory
            .file_type()
            .map(|kind| kind.is_dir())
            .unwrap_or(false)
        {
            continue;
        }
        if let Ok(app_id) = directory.file_name().to_string_lossy().parse::<u32>() {
            app_ids.push(app_id);
        }
    }
    app_ids.sort_unstable();
    app_ids.dedup();
    let usages = app_ids
        .into_iter()
        .map(|app_id| {
            stop::inspect_resources(app_id, &data_root)
                .map(|usage| (app_id, usage))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_unique_resource_pids(&usages)?;
    if json {
        print!("{}", resource_status_all_json(&usages, max_total_rss_mib));
    } else {
        let total_rss_kib = usages
            .iter()
            .map(|(_, usage)| usage.total_rss_kib)
            .sum::<u64>();
        let process_count = usages
            .iter()
            .map(|(_, usage)| usage.processes.len())
            .sum::<usize>();
        println!(
            "Macroton prefixes: {:.1} MiB RSS across {} process(es) and {} known prefix(es); total limit {} MiB",
            total_rss_kib as f64 / 1024.0,
            process_count,
            usages.len(),
            max_total_rss_mib
        );
        for (app_id, usage) in usages
            .iter()
            .filter(|(_, usage)| !usage.processes.is_empty())
        {
            println!(
                "  AppID {app_id}: {:.1} MiB across {} process(es)",
                usage.total_rss_kib as f64 / 1024.0,
                usage.processes.len()
            );
        }
    }
    Ok(())
}

fn validate_unique_resource_pids(
    usages: &[(u32, stop::PrefixResourceUsage)],
) -> Result<(), String> {
    let mut owners = BTreeMap::new();
    for (app_id, usage) in usages {
        for process in &usage.processes {
            if let Some(other_app_id) = owners.insert(process.pid, *app_id) {
                return Err(format!(
                    "PID {} was attributed to both AppID {} and AppID {}; aggregate RSS is ambiguous",
                    process.pid, other_app_id, app_id
                ));
            }
        }
    }
    Ok(())
}

fn resource_status_all_json(
    usages: &[(u32, stop::PrefixResourceUsage)],
    max_total_rss_mib: u64,
) -> String {
    let total_rss_kib = usages
        .iter()
        .map(|(_, usage)| usage.total_rss_kib)
        .sum::<u64>();
    let process_count = usages
        .iter()
        .map(|(_, usage)| usage.processes.len())
        .sum::<usize>();
    let active_prefix_count = usages
        .iter()
        .filter(|(_, usage)| !usage.processes.is_empty())
        .count();
    let apps = usages
        .iter()
        .map(|(app_id, usage)| {
            format!(
                "    {{\"appid\": {app_id}, \"total_rss_kib\": {}, \"process_count\": {}}}",
                usage.total_rss_kib,
                usage.processes.len()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let limit_rss_kib = max_total_rss_mib.saturating_mul(1024);
    format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"scope\": \"all-numeric-appid-directories-under-data-root\",\n",
            "  \"total_rss_kib\": {},\n",
            "  \"limit_rss_kib\": {},\n",
            "  \"over_limit\": {},\n",
            "  \"process_count\": {},\n",
            "  \"known_prefix_count\": {},\n",
            "  \"active_prefix_count\": {},\n",
            "  \"apps\": [\n",
            "{}\n",
            "  ],\n",
            "  \"action\": \"read-only\"\n",
            "}}\n"
        ),
        total_rss_kib,
        limit_rss_kib,
        total_rss_kib > limit_rss_kib,
        process_count,
        usages.len(),
        active_prefix_count,
        apps,
    )
}

struct ResourceGuardOptions<'a> {
    app_id: u32,
    data_root: &'a Path,
    runtime_root: &'a Path,
    max_rss_mib: u64,
    hard_rss_mib: Option<u64>,
    interval_seconds: u64,
    startup_grace_seconds: u64,
    max_runtime_seconds: Option<u64>,
    runtime_id: Option<&'a str>,
    provenance: Option<&'a ResourceGuardProvenance>,
    game_executable: Option<&'a str>,
    keep_idle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResourceGuardProvenance {
    identity: steam_session::Identity,
    steam_session_sha256: String,
}

const RESOURCE_GUARD_OVER_LIMIT_SAMPLES: u64 = 5;
const RESOURCE_GUARD_REPORT_CHANGE_KIB: u64 = 16 * 1024;
const RESOURCE_GUARD_HEARTBEAT_SECONDS: u64 = 5 * 60;
/// Bound a Steam launch handoff so a logged-in CEF client cannot remain
/// resident forever when Steam never creates the configured game process.
///
/// Steam may spend well over a minute updating/restarting its CEF client before
/// it submits a delayed `-applaunch` request.  A 90-second bound was observed
/// to race that handoff and stop the prefix just before the game appeared.
/// Keep the safety bound, but give the client a realistic five-minute window.
const STEAM_GAME_HANDOFF_TIMEOUT_SECONDS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceLimitAction {
    WithinLimit,
    StartupBurst,
    StopSustained,
    StopHard,
}

fn resource_limit_action(
    total_rss_kib: u64,
    limit_rss_kib: u64,
    hard_limit_rss_kib: u64,
    consecutive_over_limit_samples: &mut u64,
) -> ResourceLimitAction {
    if total_rss_kib > hard_limit_rss_kib {
        *consecutive_over_limit_samples += 1;
        return ResourceLimitAction::StopHard;
    }
    if total_rss_kib > limit_rss_kib {
        *consecutive_over_limit_samples += 1;
        if *consecutive_over_limit_samples >= RESOURCE_GUARD_OVER_LIMIT_SAMPLES {
            ResourceLimitAction::StopSustained
        } else {
            ResourceLimitAction::StartupBurst
        }
    } else {
        *consecutive_over_limit_samples = 0;
        ResourceLimitAction::WithinLimit
    }
}

fn resource_guard_idle_is_terminal(
    observed_process: bool,
    observed_game_process: bool,
    elapsed: Duration,
    startup_grace: Duration,
) -> bool {
    observed_process && (observed_game_process || elapsed >= startup_grace)
}

fn steam_game_handoff_timed_out(
    observed_steam_process: bool,
    game_present: bool,
    elapsed: Duration,
    game_executable_configured: bool,
) -> bool {
    game_executable_configured
        && observed_steam_process
        && !game_present
        && elapsed >= Duration::from_secs(STEAM_GAME_HANDOFF_TIMEOUT_SECONDS)
}

fn resource_guard_should_report(
    sample: u64,
    interval_seconds: u64,
    total_rss_kib: u64,
    process_count: usize,
    last_reported: Option<(u64, usize)>,
) -> bool {
    let Some((previous_rss_kib, previous_process_count)) = last_reported else {
        return true;
    };
    let heartbeat_samples = RESOURCE_GUARD_HEARTBEAT_SECONDS
        .div_ceil(interval_seconds.max(1))
        .max(1);
    previous_process_count != process_count
        || previous_rss_kib.abs_diff(total_rss_kib) >= RESOURCE_GUARD_REPORT_CHANGE_KIB
        || sample.is_multiple_of(heartbeat_samples)
}

fn parse_lsof_cwd(output: &str) -> Option<&str> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix('n'))
        .find(|path| path.starts_with('/'))
}

fn observed_game_cwd(pid: u32) -> Result<Option<String>, String> {
    let output = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .map_err(|error| format!("cannot run lsof for verified game PID {pid}: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_lsof_cwd(&text).map(str::to_owned))
}

fn resource_guard_cwd_candidates(
    app_id: u32,
    data_root: &Path,
    game_executable: Option<&str>,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(expected) = game_executable else {
        return names;
    };
    names.insert(
        expected
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(expected)
            .to_ascii_lowercase(),
    );
    if let Ok(metadata) = steam_appinfo::inspect(app_id, data_root) {
        for launch in metadata.launches {
            if launch
                .os_list
                .as_deref()
                .is_some_and(|platforms| !platforms.split(',').any(|item| item.trim() == "windows"))
            {
                continue;
            }
            if let Some(basename) = launch.executable.rsplit(['/', '\\']).next() {
                names.insert(basename.to_ascii_lowercase());
            }
        }
    }
    names
}

fn resource_guard(options: &ResourceGuardOptions<'_>) -> Result<(), String> {
    let ResourceGuardOptions {
        app_id,
        data_root,
        runtime_root,
        max_rss_mib,
        hard_rss_mib,
        interval_seconds,
        startup_grace_seconds,
        max_runtime_seconds,
        runtime_id,
        provenance,
        game_executable,
        keep_idle,
    } = options;
    validate_resource_guard_provenance(*app_id, data_root, *provenance)?;
    let limit_kib = max_rss_mib.saturating_mul(1024);
    let hard_limit_kib = hard_rss_mib
        .unwrap_or(max_rss_mib.saturating_mul(3) / 2)
        .saturating_mul(1024);
    let started = Instant::now();
    let started_unix_ms = unix_time_ms()?;
    let mut observed_process = false;
    let mut observed_steam_process = false;
    let mut observed_game_process = false;
    let mut consecutive_steam_samples = 0_u64;
    let mut consecutive_game_samples = 0_u64;
    let mut max_consecutive_steam_samples = 0_u64;
    let mut max_consecutive_game_samples = 0_u64;
    let mut consecutive_over_limit_samples = 0_u64;
    let mut peak = None;
    let mut samples = 0_u64;
    let mut last_reported_usage = None;
    let mut cwd_probed_game_pids = BTreeSet::new();
    let cwd_candidate_names = resource_guard_cwd_candidates(*app_id, data_root, *game_executable);
    loop {
        let usage =
            stop::inspect_resources(*app_id, data_root).map_err(|error| error.to_string())?;
        samples += 1;
        if !usage.pids.is_empty() {
            observed_process = true;
        }
        let steam_present = usage
            .processes
            .iter()
            .any(|process| is_windows_steam_resource_category(process.category));
        let exact_game_present = usage.processes.iter().any(|process| {
            process.category == "windows-game"
                && game_executable
                    .map(|expected| {
                        process.command.eq_ignore_ascii_case(
                            expected.rsplit(['/', '\\']).next().unwrap_or(expected),
                        )
                    })
                    .unwrap_or(true)
        });
        // Steam games may expose a short-lived configuration/launcher EXE
        // before handing off to their real game process (for example, Heroes
        // III's HOMM3Launcher.exe -> HOMM3 2.0.exe). Both are classified as
        // windows-game inside this exact prefix. Treating any such process as
        // a completed handoff prevents the guard from killing a game while a
        // user is still interacting with its launcher, while the exact name
        // remains the primary evidence when it is present.
        let launcher_or_game_present = game_executable.is_some()
            && usage
                .processes
                .iter()
                .any(|process| process.category == "windows-game");
        let game_present = exact_game_present || launcher_or_game_present;
        observed_steam_process |= steam_present;
        observed_game_process |= game_present;
        consecutive_steam_samples = if steam_present {
            consecutive_steam_samples + 1
        } else {
            0
        };
        consecutive_game_samples = if game_present {
            consecutive_game_samples + 1
        } else {
            0
        };
        max_consecutive_steam_samples =
            max_consecutive_steam_samples.max(consecutive_steam_samples);
        max_consecutive_game_samples = max_consecutive_game_samples.max(consecutive_game_samples);
        if peak
            .as_ref()
            .map(|current: &stop::PrefixResourceUsage| usage.total_rss_kib > current.total_rss_kib)
            .unwrap_or(true)
        {
            peak = Some(usage.clone());
        }
        if resource_guard_should_report(
            samples,
            *interval_seconds,
            usage.total_rss_kib,
            usage.processes.len(),
            last_reported_usage,
        ) {
            print_resource_usage(*app_id, &usage, *max_rss_mib);
            last_reported_usage = Some((usage.total_rss_kib, usage.processes.len()));
        }
        let limit_action = resource_limit_action(
            usage.total_rss_kib,
            limit_kib,
            hard_limit_kib,
            &mut consecutive_over_limit_samples,
        );
        if limit_action == ResourceLimitAction::StartupBurst {
            println!(
                "resource guard: temporary RSS excess sample {}/{}; allowing startup to settle; hard limit {:.1} MiB",
                consecutive_over_limit_samples,
                RESOURCE_GUARD_OVER_LIMIT_SAMPLES,
                hard_limit_kib as f64 / 1024.0
            );
        }
        if matches!(
            limit_action,
            ResourceLimitAction::StopSustained | ResourceLimitAction::StopHard
        ) {
            print_resource_peak(*app_id, peak.as_ref());
            report_resource_peak_evidence(
                *app_id,
                data_root,
                &ResourcePeakObservation {
                    peak: peak.as_ref(),
                    reason: if limit_action == ResourceLimitAction::StopHard {
                        "rss-hard-limit"
                    } else {
                        "rss-sustained-limit"
                    },
                    limit_rss_kib: limit_kib,
                    provenance: *provenance,
                    started_unix_ms,
                    observed: started.elapsed(),
                    samples,
                    observed_steam_process,
                    observed_game_process,
                    max_consecutive_steam_samples,
                    max_consecutive_game_samples,
                    interval_seconds: *interval_seconds,
                },
            );
            println!(
                "resource guard: {} RSS limit exceeded; stopping only AppID {app_id}'s managed prefix",
                if limit_action == ResourceLimitAction::StopHard {
                    "hard"
                } else {
                    "sustained"
                }
            );
            if let Err(error) = execute::record_stop_request(*app_id, data_root) {
                eprintln!("resource guard: cannot cancel in-flight launch before RSS containment: {error}");
            }
            let result = stop::stop_with_runtime_id(*app_id, data_root, runtime_root, *runtime_id)
                .map_err(|error| error.to_string())?;
            println!(
                "resource guard: contained AppID {app_id}; {} TERM, {} KILL",
                result.terminated_orphans.len(),
                result.killed_orphans.len()
            );
            return Ok(());
        }
        // Probe only PIDs already verified as belonging to this exact Wine
        // prefix. Launcher-to-game handoffs often last only a few seconds;
        // one bounded cwd sample per PID distinguishes a wrong inherited
        // Steam directory from a game-content or API initialization failure.
        // Do this after RSS containment so lsof cannot postpone a hard stop.
        for process in usage.processes.iter().filter(|process| {
            process.category == "windows-game"
                && cwd_candidate_names.contains(&process.command.to_ascii_lowercase())
        }) {
            if cwd_probed_game_pids.len() >= 4 || !cwd_probed_game_pids.insert(process.pid) {
                continue;
            }
            match observed_game_cwd(process.pid) {
                Ok(Some(cwd)) => println!(
                    "resource guard: verified Windows game PID {} cwd {}",
                    process.pid, cwd
                ),
                Ok(None) => println!(
                    "resource guard: verified Windows game PID {} cwd unavailable",
                    process.pid
                ),
                Err(error) => eprintln!("resource guard: {error}"),
            }
        }
        if max_runtime_seconds.is_some_and(|limit| started.elapsed() >= Duration::from_secs(limit))
            && !usage.pids.is_empty()
        {
            print_resource_peak(*app_id, peak.as_ref());
            report_resource_peak_evidence(
                *app_id,
                data_root,
                &ResourcePeakObservation {
                    peak: peak.as_ref(),
                    reason: "time-limit",
                    limit_rss_kib: limit_kib,
                    provenance: *provenance,
                    started_unix_ms,
                    observed: started.elapsed(),
                    samples,
                    observed_steam_process,
                    observed_game_process,
                    max_consecutive_steam_samples,
                    max_consecutive_game_samples,
                    interval_seconds: *interval_seconds,
                },
            );
            println!(
                "resource guard: session time limit reached; stopping only AppID {app_id}'s managed prefix"
            );
            if let Err(error) = execute::record_stop_request(*app_id, data_root) {
                eprintln!("resource guard: cannot cancel in-flight launch before time containment: {error}");
            }
            let result = stop::stop_with_runtime_id(*app_id, data_root, runtime_root, *runtime_id)
                .map_err(|error| error.to_string())?;
            println!(
                "resource guard: contained timed-out AppID {app_id}; {} TERM, {} KILL",
                result.terminated_orphans.len(),
                result.killed_orphans.len()
            );
            return Ok(());
        }
        if steam_game_handoff_timed_out(
            observed_steam_process,
            game_present,
            started.elapsed(),
            game_executable.is_some(),
        ) {
            print_resource_peak(*app_id, peak.as_ref());
            report_resource_peak_evidence(
                *app_id,
                data_root,
                &ResourcePeakObservation {
                    peak: peak.as_ref(),
                    reason: "steam-client-no-game",
                    limit_rss_kib: limit_kib,
                    provenance: *provenance,
                    started_unix_ms,
                    observed: started.elapsed(),
                    samples,
                    observed_steam_process,
                    observed_game_process,
                    max_consecutive_steam_samples,
                    max_consecutive_game_samples,
                    interval_seconds: *interval_seconds,
                },
            );
            println!(
                "resource guard: Steam client remained live for {} seconds without creating {}; stopping only AppID {}'s managed prefix",
                STEAM_GAME_HANDOFF_TIMEOUT_SECONDS,
                game_executable.unwrap_or("the configured game"),
                app_id
            );
            if let Err(error) = execute::record_stop_request(*app_id, data_root) {
                eprintln!("resource guard: cannot cancel in-flight launch before handoff containment: {error}");
            }
            let result = stop::stop_with_runtime_id(*app_id, data_root, runtime_root, *runtime_id)
                .map_err(|error| error.to_string())?;
            println!(
                "resource guard: contained failed Steam handoff for AppID {app_id}; {} TERM, {} KILL",
                result.terminated_orphans.len(),
                result.killed_orphans.len()
            );
            return Ok(());
        }
        if !keep_idle
            && usage.pids.is_empty()
            && resource_guard_idle_is_terminal(
                observed_process,
                observed_game_process,
                started.elapsed(),
                Duration::from_secs(*startup_grace_seconds),
            )
        {
            print_resource_peak(*app_id, peak.as_ref());
            report_resource_peak_evidence(
                *app_id,
                data_root,
                &ResourcePeakObservation {
                    peak: peak.as_ref(),
                    reason: "prefix-idle",
                    limit_rss_kib: limit_kib,
                    provenance: *provenance,
                    started_unix_ms,
                    observed: started.elapsed(),
                    samples,
                    observed_steam_process,
                    observed_game_process,
                    max_consecutive_steam_samples,
                    max_consecutive_game_samples,
                    interval_seconds: *interval_seconds,
                },
            );
            println!("resource guard: managed prefix is idle");
            return Ok(());
        }
        if !observed_process && started.elapsed() >= Duration::from_secs(*startup_grace_seconds) {
            report_resource_peak_evidence(
                *app_id,
                data_root,
                &ResourcePeakObservation {
                    peak: peak.as_ref(),
                    reason: "startup-no-process",
                    limit_rss_kib: limit_kib,
                    provenance: *provenance,
                    started_unix_ms,
                    observed: started.elapsed(),
                    samples,
                    observed_steam_process,
                    observed_game_process,
                    max_consecutive_steam_samples,
                    max_consecutive_game_samples,
                    interval_seconds: *interval_seconds,
                },
            );
            println!("resource guard: no managed process observed during startup grace period");
            return Ok(());
        }
        thread::sleep(Duration::from_secs(*interval_seconds));
    }
}

struct ResourcePeakObservation<'a> {
    peak: Option<&'a stop::PrefixResourceUsage>,
    reason: &'a str,
    limit_rss_kib: u64,
    provenance: Option<&'a ResourceGuardProvenance>,
    started_unix_ms: u128,
    observed: Duration,
    samples: u64,
    observed_steam_process: bool,
    observed_game_process: bool,
    max_consecutive_steam_samples: u64,
    max_consecutive_game_samples: u64,
    interval_seconds: u64,
}

fn report_resource_peak_evidence(
    app_id: u32,
    data_root: &Path,
    observation: &ResourcePeakObservation<'_>,
) {
    if let Err(error) = persist_resource_peak(app_id, data_root, observation) {
        eprintln!("resource guard evidence warning: {error}");
    }
}

fn persist_resource_peak(
    app_id: u32,
    data_root: &Path,
    observation: &ResourcePeakObservation<'_>,
) -> Result<PathBuf, String> {
    let evidence_root = data_root
        .join("games")
        .join(app_id.to_string())
        .join("resource-guard-evidence");
    fs::create_dir_all(&evidence_root)
        .map_err(|error| format!("cannot create resource evidence directory: {error}"))?;
    let captured_unix_ms = unix_time_ms()?;
    let destination = evidence_root.join(format!("{captured_unix_ms}-{}.json", std::process::id()));
    let contents = resource_peak_json(app_id, observation, captured_unix_ms);
    write_new_evidence(&destination, contents.as_bytes())?;
    println!("resource guard evidence: {}", destination.display());
    Ok(destination)
}

fn resource_peak_json(
    app_id: u32,
    observation: &ResourcePeakObservation<'_>,
    captured_unix_ms: u128,
) -> String {
    let ResourcePeakObservation {
        peak,
        reason,
        limit_rss_kib,
        provenance,
        started_unix_ms,
        observed,
        samples,
        observed_steam_process,
        observed_game_process,
        max_consecutive_steam_samples,
        max_consecutive_game_samples,
        interval_seconds,
    } = observation;
    let total_rss_kib = peak.map(|usage| usage.total_rss_kib).unwrap_or(0);
    let processes = peak.map(|usage| usage.processes.as_slice()).unwrap_or(&[]);
    let mut categories = BTreeMap::<&str, u64>::new();
    for process in processes {
        *categories.entry(process.category).or_default() += process.rss_kib;
    }
    let category_json = categories
        .iter()
        .map(|(category, rss_kib)| format!("\"{}\":{}", json_escape(category), rss_kib))
        .collect::<Vec<_>>()
        .join(",");
    let process_json = processes
        .iter()
        .map(|process| {
            format!(
                "{{\"pid\":{},\"rss_kib\":{},\"category\":\"{}\",\"command\":\"{}\"}}",
                process.pid,
                process.rss_kib,
                process.category,
                json_escape(&process.command)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let (
        scope,
        profile,
        runtime,
        runtime_fingerprint,
        client_mode,
        renderer,
        graphics_api,
        session,
    ) = match provenance {
        Some(provenance) => (
            "steam-profile-session",
            json_value(Some(&provenance.identity.profile_sha256)),
            json_value(Some(&provenance.identity.runtime_id)),
            json_value(Some(&provenance.identity.runtime_fingerprint_sha256)),
            json_value(Some(&provenance.identity.steam_client_mode)),
            json_value(Some(&provenance.identity.renderer)),
            json_value(Some(&provenance.identity.graphics_api)),
            json_value(Some(&provenance.steam_session_sha256)),
        ),
        None => (
            "unattributed-guard",
            "null".to_owned(),
            "null".to_owned(),
            "null".to_owned(),
            "null".to_owned(),
            "null".to_owned(),
            "null".to_owned(),
            "null".to_owned(),
        ),
    };
    format!(
        concat!(
            "{{\n  \"schema\": 3,\n  \"captured_unix_ms\": {captured_unix_ms},\n",
            "  \"started_unix_ms\": {started_unix_ms},\n  \"observed_milliseconds\": {},\n",
            "  \"samples\": {samples},\n  \"observed_steam_process\": {observed_steam_process},\n",
            "  \"observed_game_process\": {observed_game_process},\n  \"appid\": {app_id},\n",
            "  \"max_consecutive_steam_samples\": {max_consecutive_steam_samples},\n",
            "  \"max_consecutive_game_samples\": {max_consecutive_game_samples},\n",
            "  \"interval_seconds\": {interval_seconds},\n",
            "  \"reason\": \"{}\",\n",
            "  \"evidence_scope\": \"{scope}\",\n  \"profile_sha256\": {profile},\n",
            "  \"runtime_id\": {runtime},\n  \"runtime_fingerprint_sha256\": {runtime_fingerprint},\n",
            "  \"steam_client_mode\": {client_mode},\n  \"renderer\": {renderer},\n",
            "  \"graphics_api\": {graphics_api},\n  \"steam_session_sha256\": {session},\n",
            "  \"peak_rss_kib\": {total_rss_kib},\n  \"limit_rss_kib\": {limit_rss_kib},\n",
            "  \"categories\": {{{category_json}}},\n  \"processes\": [{process_json}]\n}}\n"
        ),
        observed.as_millis(),
        json_escape(reason),
        captured_unix_ms = captured_unix_ms,
        started_unix_ms = started_unix_ms,
        samples = samples,
        observed_steam_process = observed_steam_process,
        observed_game_process = observed_game_process,
        max_consecutive_steam_samples = max_consecutive_steam_samples,
        max_consecutive_game_samples = max_consecutive_game_samples,
        interval_seconds = interval_seconds,
        app_id = app_id,
        scope = scope,
        profile = profile,
        runtime = runtime,
        runtime_fingerprint = runtime_fingerprint,
        client_mode = client_mode,
        renderer = renderer,
        graphics_api = graphics_api,
        session = session,
        total_rss_kib = total_rss_kib,
        limit_rss_kib = limit_rss_kib,
        category_json = category_json,
        process_json = process_json,
    )
}

fn json_value(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", json_escape(value)))
        .unwrap_or_else(|| "null".to_owned())
}

fn unix_time_ms() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))
}

fn current_unix_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))
}

fn validate_resource_guard_provenance(
    app_id: u32,
    data_root: &Path,
    provenance: Option<&ResourceGuardProvenance>,
) -> Result<(), String> {
    let Some(provenance) = provenance else {
        return Ok(());
    };
    if provenance.identity.app_id != app_id {
        return Err("resource guard provenance AppID mismatch".to_owned());
    }
    if provenance.steam_session_sha256.len() != 64
        || !provenance
            .steam_session_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("resource guard provenance has invalid Steam session SHA-256".to_owned());
    }
    let prefix = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix");
    steam_session::validate(&prefix, &provenance.identity)
        .map_err(|error| format!("resource guard Steam session validation failed: {error}"))?;
    let actual_sha256 = sha256::to_hex(
        &sha256::digest_file(&prefix.join(steam_session::MARKER))
            .map_err(|error| format!("cannot hash resource guard Steam session marker: {error}"))?,
    );
    if actual_sha256 != provenance.steam_session_sha256 {
        return Err("resource guard Steam session marker changed before observation".to_owned());
    }
    Ok(())
}

fn print_resource_peak(app_id: u32, peak: Option<&stop::PrefixResourceUsage>) {
    let Some(peak) = peak.filter(|usage| usage.total_rss_kib > 0) else {
        return;
    };
    let mut categories = BTreeMap::<&str, u64>::new();
    for process in &peak.processes {
        *categories.entry(process.category).or_default() += process.rss_kib;
    }
    println!(
        "resource guard peak: AppID {app_id} {:.1} MiB; {}",
        peak.total_rss_kib as f64 / 1024.0,
        categories
            .iter()
            .map(|(category, rss_kib)| format!("{category}={:.1} MiB", *rss_kib as f64 / 1024.0))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for process in peak.processes.iter().take(10) {
        println!(
            "resource guard peak process: PID {} {:.1} MiB {} {}",
            process.pid,
            process.rss_kib as f64 / 1024.0,
            process.category,
            process.command
        );
    }
}

fn is_windows_steam_resource_category(category: &str) -> bool {
    matches!(category, "steam-client" | "steam-cef" | "steamcmd")
}

fn is_primary_steam_client_process(process: &stop::ProcessResourceUsage) -> bool {
    process.category == "steam-client" && process.command.eq_ignore_ascii_case("steam.exe")
}

fn print_resource_usage(app_id: u32, usage: &stop::PrefixResourceUsage, max_rss_mib: u64) {
    println!(
        "AppID {app_id}: {:.1} MiB RSS across {} exact-prefix process(es); limit {} MiB",
        usage.total_rss_kib as f64 / 1024.0,
        usage.pids.len(),
        max_rss_mib
    );
}

fn print_resource_usage_json(app_id: u32, usage: &stop::PrefixResourceUsage, max_rss_mib: u64) {
    let mut categories = BTreeMap::<&str, u64>::new();
    for process in &usage.processes {
        *categories.entry(process.category).or_default() += process.rss_kib;
    }
    println!("{{");
    println!("  \"schema\": 1,");
    println!("  \"appid\": {app_id},");
    println!("  \"total_rss_kib\": {},", usage.total_rss_kib);
    println!("  \"limit_rss_kib\": {},", max_rss_mib.saturating_mul(1024));
    println!("  \"process_count\": {},", usage.processes.len());
    println!("  \"categories\": {{");
    for (index, (category, rss_kib)) in categories.iter().enumerate() {
        println!(
            "    \"{}\": {}{}",
            json_escape(category),
            rss_kib,
            if index + 1 == categories.len() {
                ""
            } else {
                ","
            }
        );
    }
    println!("  }},");
    println!("  \"processes\": [");
    for (index, process) in usage.processes.iter().enumerate() {
        println!(
            "    {{\"pid\": {}, \"rss_kib\": {}, \"category\": \"{}\", \"command\": \"{}\"}}{}",
            process.pid,
            process.rss_kib,
            process.category,
            json_escape(&process.command),
            if index + 1 == usage.processes.len() {
                ""
            } else {
                ","
            }
        );
    }
    println!("  ]");
    println!("}}");
}

fn parse_positive_u64(value: &str, option: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{option} must be a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{option} must be a positive integer"));
    }
    Ok(parsed)
}

fn stop_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton stop APPID [--runtime-root PATH] [--data-root PATH] [--preserve-session]".to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut runtime_root = default_runtime_root()?;
    let mut data_root = default_data_root()?;
    let mut preserve_session = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--preserve-session" {
            preserve_session = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a path"))?;
        match option {
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--data-root" => data_root = PathBuf::from(value),
            other => return Err(format!("unknown stop option {other:?}")),
        }
        index += 1;
    }
    if let Some((owner, _)) = read_shared_client_marker(app_id, &data_root)? {
        return Err(format!("AppID {app_id} is running in AppID {owner}'s shared Steam prefix; exit the game normally and use `macroton steam shared-client-finish {owner} {app_id}` for Cloud sync, or stop AppID {owner} explicitly to interrupt the whole client"));
    }
    // Cancel an in-flight Steam warmup/retry before stopping its current
    // processes; otherwise the same launch command can recreate the client.
    execute::record_stop_request(app_id, &data_root).map_err(|error| error.to_string())?;
    let result = if preserve_session {
        stop::stop_processes_preserve_session(app_id, &data_root, &runtime_root)
    } else {
        stop::stop(app_id, &data_root, &runtime_root)
    }
    .map_err(|error| error.to_string())?;
    // A stopped prefix must not continue reporting a historical
    // `running-observed` event on the next status poll.  Advance the same
    // launch-log boundaries used before a new Steam request; this preserves
    // the logs for evidence while making lifecycle state reflect the stop.
    launch_status::record_launch_boundary(app_id, &data_root).map_err(|error| {
        format!("stopped prefix but could not advance launch boundary: {error}")
    })?;
    println!("stopped AppID {app_id} prefix: {}", result.prefix.display());
    println!("runtime: {}", result.runtime_id);
    println!(
        "orphan cleanup: {} TERM, {} KILL",
        result.terminated_orphans.len(),
        result.killed_orphans.len()
    );
    Ok(())
}

fn steam_command(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(
            "usage: macroton steam <appinfo|bootstrap|client-status|client-wait|cloud-roundtrip|cloud-status|cloud-wait|connection-status|install|install-status|install-wait|inventory|launch-status|library-status|lifecycle-status|login|login-wait|prefix-status|prerequisite-status|rebind-runtime|session-status|shared-client-abandon|shared-client-finish|shared-client-install|shared-client-launch|shared-client-status|target-status|trim-client> ..."
                .to_owned(),
        );
    };
    match command {
        "appinfo" => steam_appinfo_command(&args[1..]),
        "bootstrap" => steam_bootstrap_command(&args[1..]),
        "client-status" => steam_client_status_command(&args[1..]),
        "client-wait" => steam_client_wait_command(&args[1..]),
        "cloud-status" => steam_cloud_status_command(&args[1..]),
        "cloud-wait" => steam_cloud_wait_command(&args[1..]),
        "cloud-roundtrip" => steam_cloud_roundtrip_command(&args[1..]),
        "connection-status" => steam_connection_status_command(&args[1..]),
        "install" => steam_install_command(&args[1..]),
        "install-status" => steam_install_status_command(&args[1..]),
        "install-wait" => steam_install_wait_command(&args[1..]),
        "inventory" => steam_inventory_command(&args[1..]),
        "launch-status" => steam_launch_status_command(&args[1..]),
        "library-status" => steam_library_status_command(&args[1..]),
        "lifecycle-status" => steam_lifecycle_status_command(&args[1..]),
        "login" => steam_login_command(&args[1..]),
        "login-wait" => steam_login_wait_command(&args[1..]),
        "prefix-status" => steam_prefix_status_command(&args[1..]),
        "prerequisite-status" => steam_prerequisite_status_command(&args[1..]),
        "rebind-runtime" => steam_rebind_runtime_command(&args[1..]),
        "session-status" => steam_session_status_command(&args[1..]),
        "shared-client-status" => steam_shared_client_status_command(&args[1..]),
        "shared-client-install" => steam_shared_client_install_command(&args[1..]),
        "shared-client-launch" => steam_shared_client_launch_command(&args[1..]),
        "shared-client-finish" => steam_shared_client_finish_command(&args[1..]),
        "shared-client-abandon" => steam_shared_client_abandon_command(&args[1..]),
        "target-status" => steam_target_status_command(&args[1..]),
        "trim-client" => steam_trim_client_command(&args[1..]),
        other => Err(format!("unknown steam command {other:?}")),
    }
}

fn steam_cloud_roundtrip_command(args: &[String]) -> Result<(), String> {
    let action = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam cloud-roundtrip <status|begin|confirm-upload|quarantine|verify-download|restore|abandon> APPID --profile PATH [--data-root PATH] [--confirm]"
                .to_owned()
        })?
        .as_str();
    let app_id: u32 = args
        .get(1)
        .ok_or_else(|| "cloud-roundtrip action requires APPID".to_owned())?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut data_root = default_data_root()?;
    let mut confirm = false;
    let mut index = 2;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--confirm" {
            confirm = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            other => return Err(format!("unknown steam cloud-roundtrip option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if action == "abandon" {
        if !confirm {
            return Err(
                "abandon requires --confirm; it moves only the active baseline marker to recoverable evidence"
                    .to_owned(),
            );
        }
        let transaction = cloud_roundtrip::abandon(app_id, &profile, &data_root)
            .map_err(|error| error.to_string())?;
        println!(
            "Steam Cloud roundtrip AppID {} transaction {} abandoned; evidence moved to recoverable transaction storage",
            transaction.app_id, transaction.transaction_id
        );
        return Ok(());
    }
    let transaction = match action {
        "status" => cloud_roundtrip::status(app_id, &profile, &data_root),
        "begin" => cloud_roundtrip::begin(app_id, &profile, &data_root),
        "confirm-upload" => cloud_roundtrip::confirm_upload(app_id, &profile, &data_root),
        "quarantine" => cloud_roundtrip::quarantine(app_id, &profile, &data_root),
        "verify-download" => cloud_roundtrip::verify_download(app_id, &profile, &data_root),
        "restore" => cloud_roundtrip::restore(app_id, &profile, &data_root),
        other => {
            return Err(format!(
                "unknown cloud-roundtrip action {other:?}; expected status, begin, confirm-upload, quarantine, verify-download, restore, or abandon"
            ));
        }
    }
    .map_err(|error| error.to_string())?;
    println!(
        "Steam Cloud roundtrip AppID {} transaction {}: {}",
        transaction.app_id,
        transaction.transaction_id,
        transaction.phase.as_str()
    );
    match transaction.phase {
        cloud_roundtrip::Phase::Baseline => println!(
            "Next: create a new in-game save, exit normally, wait for Steam upload, stop the prefix, then run confirm-upload."
        ),
        cloud_roundtrip::Phase::Uploaded => println!(
            "Upload content and post-baseline event verified. Next: run quarantine while the prefix remains idle."
        ),
        cloud_roundtrip::Phase::Quarantined => println!(
            "Local saves moved to the recoverable quarantine. Restart authenticated Steam to download, stop the prefix, then run verify-download; use restore only if recovery is needed."
        ),
        cloud_roundtrip::Phase::Complete => println!(
            "Fresh upload, local quarantine, post-quarantine download and byte-identical restoration are verified."
        ),
        cloud_roundtrip::Phase::Restored => println!(
            "Quarantined local saves were restored without overwriting downloaded content."
        ),
    }
    Ok(())
}

fn steam_lifecycle_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam lifecycle-status APPID [--data-root PATH] [--json]".to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                json = true;
                index += 1;
            }
            "--data-root" => {
                index += 1;
                data_root = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--data-root requires a path".to_owned())?,
                );
                index += 1;
            }
            other => return Err(format!("unknown steam lifecycle-status option {other:?}")),
        }
    }
    let session = macos_session::inspect().map_err(|error| error.to_string())?;
    let prefix =
        steam_prefix_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let launch = launch_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let resources =
        stop::inspect_resources(app_id, &data_root).map_err(|error| error.to_string())?;
    let tracked_game_pid_live = launch
        .process_id
        .is_some_and(|pid| resources.pids.contains(&pid));
    let primary_steam_client_live = resources
        .processes
        .iter()
        .any(is_primary_steam_client_process);
    let client_update =
        steam_client_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let update_now = current_unix_seconds()?;
    let update_fresh = client_update.update_progress_is_fresh(update_now, 300);
    let update_stalled = client_update.state.update_active() && !update_fresh;
    let phase = lifecycle_phase(
        session,
        prefix.readiness,
        launch.state,
        tracked_game_pid_live,
        primary_steam_client_live,
        update_fresh,
        update_stalled,
    );
    if json {
        println!("{{");
        println!("  \"schema_version\": 2,");
        println!("  \"appid\": {app_id},");
        println!("  \"phase\": {:?},", phase);
        println!("  \"console_state\": {:?},", session.as_str());
        println!("  \"prefix_readiness\": {:?},", prefix.readiness.as_str());
        println!("  \"launch_state\": {:?},", launch.state.as_str());
        println!("  \"tracked_game_pid_live\": {tracked_game_pid_live},");
        println!("  \"primary_steam_client_live\": {primary_steam_client_live},");
        println!(
            "  \"steam_client_update_state\": {:?},",
            client_update.state.as_str()
        );
        println!("  \"steam_client_update_progress_fresh\": {update_fresh},");
        println!(
            "  \"steam_client_update_downloaded_kib\": {},",
            json_optional_u64(client_update.downloaded_kib)
        );
        println!(
            "  \"steam_client_update_total_kib\": {},",
            json_optional_u64(client_update.total_kib)
        );
        println!(
            "  \"steam_client_update_rate_kib_per_minute\": {},",
            json_optional_u64(client_update.recent_rate_kib_per_minute)
        );
        println!(
            "  \"steam_client_update_eta_seconds\": {},",
            json_optional_u64(client_update.estimated_seconds_remaining)
        );
        println!("  \"total_rss_kib\": {},", resources.total_rss_kib);
        println!("  \"process_count\": {},", resources.processes.len());
        // A prior launch record can remain `stale-running-event` after both
        // the game and Steam have exited.  Keep that phase visible for audit,
        // but do not deadlock the next launch when the exact prefix is empty.
        let no_live_prefix_processes = !tracked_game_pid_live && !primary_steam_client_live;
        let retryable_terminal_phase = matches!(
            phase,
            "stale-running-event" | "stale-waiting-user" | "exited" | "failed"
        );
        let safe_to_start = session == macos_session::State::Unlocked
            && no_live_prefix_processes
            && (phase == "ready-to-launch" || retryable_terminal_phase);
        println!("  \"safe_to_start_interactive\": {safe_to_start}");
        println!("}}");
    } else {
        println!("Steam lifecycle status for AppID {app_id}");
        println!("Phase:     {phase}");
        println!("Console:   {}", session.as_str());
        println!("Prefix:    {}", prefix.readiness.as_str());
        println!("Launch:    {}", launch.state.as_str());
        if phase == "waiting-cloud" {
            println!(
                "Action:    resolve the Steam Cloud pending session, then retry the Macroton launch"
            );
        }
        println!("Game live: {tracked_game_pid_live}");
        println!("Steam live: {primary_steam_client_live}");
        println!("Update:    {}", client_update.state.as_str());
        if let (Some(downloaded), Some(total)) =
            (client_update.downloaded_kib, client_update.total_kib)
        {
            if primary_steam_client_live && update_fresh {
                println!(
                    "Progress:  {downloaded}/{total} KiB ({:.1}%)",
                    downloaded as f64 * 100.0 / total as f64
                );
                if let Some(rate) = client_update.recent_rate_kib_per_minute {
                    println!("Rate:      {rate} KiB/min");
                }
                if let Some(remaining) = client_update.estimated_seconds_remaining {
                    println!("ETA:       {remaining} seconds");
                }
            } else {
                println!(
                    "Progress:  stale historical log ({downloaded}/{total} KiB); no live Steam update evidence"
                );
            }
        }
        println!(
            "Resources: {:.1} MiB across {} exact-prefix process(es)",
            resources.total_rss_kib as f64 / 1024.0,
            resources.processes.len()
        );
    }
    Ok(())
}

fn lifecycle_phase(
    session: macos_session::State,
    prefix: steam_prefix_status::Readiness,
    launch: launch_status::LaunchState,
    tracked_game_pid_live: bool,
    primary_steam_client_live: bool,
    client_update_active: bool,
    client_update_stalled: bool,
) -> &'static str {
    if prefix == steam_prefix_status::Readiness::NotBootstrapped {
        return "needs-bootstrap";
    }
    if launch == launch_status::LaunchState::RunningObserved {
        return if tracked_game_pid_live {
            "running"
        } else {
            "stale-running-event"
        };
    }
    if matches!(
        launch,
        launch_status::LaunchState::Submitted
            | launch_status::LaunchState::InstallingPrerequisites
            | launch_status::LaunchState::WaitingEula
    ) {
        return launch.as_str();
    }
    if launch == launch_status::LaunchState::WaitingUser {
        return if primary_steam_client_live {
            "waiting-user"
        } else {
            "stale-waiting-user"
        };
    }
    if launch == launch_status::LaunchState::WaitingCloud {
        return if primary_steam_client_live {
            "waiting-cloud"
        } else {
            "stale-waiting-cloud"
        };
    }
    if launch == launch_status::LaunchState::Exited {
        return "exited";
    }
    if launch == launch_status::LaunchState::Failed {
        return "failed";
    }
    if prefix == steam_prefix_status::Readiness::SignInRequired {
        return if primary_steam_client_live && client_update_active {
            "steam-client-updating"
        } else if primary_steam_client_live && client_update_stalled {
            "steam-client-update-stalled"
        } else if primary_steam_client_live {
            "login-ui-active"
        } else {
            "needs-login"
        };
    }
    if session != macos_session::State::Unlocked {
        "interactive-session-blocked"
    } else {
        "ready-to-launch"
    }
}

fn steam_client_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam client-status APPID [--data-root PATH] [--json]".to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                json = true;
                index += 1;
            }
            "--data-root" => {
                index += 1;
                data_root = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--data-root requires a path".to_owned())?,
                );
                index += 1;
            }
            other => return Err(format!("unknown steam client-status option {other:?}")),
        }
    }
    let status =
        steam_client_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let now = current_unix_seconds()?;
    let progress_age = status.progress_age_seconds(now);
    let progress_fresh = status.update_progress_is_fresh(now, 300);
    if json {
        println!("{{");
        println!("  \"schema_version\": 1,");
        println!("  \"appid\": {app_id},");
        println!("  \"scope\": \"latest-observed-update-attempt\",");
        println!("  \"state\": {:?},", status.state.as_str());
        println!(
            "  \"downloaded_kib\": {},",
            json_optional_u64(status.downloaded_kib)
        );
        println!("  \"total_kib\": {},", json_optional_u64(status.total_kib));
        println!(
            "  \"recent_rate_kib_per_minute\": {},",
            json_optional_u64(status.recent_rate_kib_per_minute)
        );
        println!(
            "  \"estimated_seconds_remaining\": {},",
            json_optional_u64(status.estimated_seconds_remaining)
        );
        println!(
            "  \"log_modified_unix\": {},",
            json_optional_u64(status.log_modified_unix)
        );
        println!(
            "  \"progress_age_seconds\": {},",
            json_optional_u64(progress_age)
        );
        println!("  \"progress_fresh\": {progress_fresh}");
        println!("}}");
        return Ok(());
    }
    println!("Steam client status for AppID {app_id}");
    println!("State:    {}", status.state.as_str());
    if let (Some(downloaded), Some(total)) = (status.downloaded_kib, status.total_kib) {
        println!(
            "Progress: {downloaded}/{total} KiB ({:.1}%)",
            downloaded as f64 * 100.0 / total as f64
        );
    }
    if let Some(rate) = status.recent_rate_kib_per_minute {
        println!("Rate:     {rate} KiB/min");
    }
    if let Some(remaining) = status.estimated_seconds_remaining {
        println!("ETA:      {remaining} seconds");
    }
    println!("Fresh:    {}", if progress_fresh { "yes" } else { "no" });
    println!("Log:      {}", status.log_path.display());
    Ok(())
}

fn steam_client_wait_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam client-wait APPID [--data-root PATH] [--timeout-seconds N] [--interval-seconds N]".to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut timeout_seconds = 4 * 60 * 60_u64;
    let mut interval_seconds = 5_u64;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--timeout-seconds" => timeout_seconds = parse_positive_u64(value, option)?,
            "--interval-seconds" => interval_seconds = parse_positive_u64(value, option)?,
            other => return Err(format!("unknown steam client-wait option {other:?}")),
        }
        index += 1;
    }
    let _idle_sleep_guard = IdleSleepGuard::acquire("Steam client update")?;
    let started = Instant::now();
    let mut last_progress = None;
    loop {
        let status =
            steam_client_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
        let now = current_unix_seconds()?;
        match status.state {
            steam_client_status::State::Current => {
                println!(
                    "Steam client update attempt completed for AppID {app_id}; this does not prove that a subsequent client launch will not start another platform/update stage"
                );
                return Ok(());
            }
            steam_client_status::State::Failed => {
                return Err(format!("Steam client update failed for AppID {app_id}"));
            }
            steam_client_status::State::Downloading | steam_client_status::State::Applying => {
                if !status.update_progress_is_fresh(now, 300) {
                    return Err(format!(
                        "Steam client update for AppID {app_id} has no fresh progress for more than 300 seconds"
                    ));
                }
            }
            steam_client_status::State::NoEvidence | steam_client_status::State::Unknown => {
                return Err(format!(
                    "Steam client update state for AppID {app_id} is {}",
                    status.state.as_str()
                ));
            }
        }
        if status.downloaded_kib != last_progress {
            if let (Some(downloaded), Some(total)) = (status.downloaded_kib, status.total_kib) {
                println!(
                    "Steam client update AppID {app_id}: {downloaded}/{total} KiB ({:.1}%); rate {}; ETA {} seconds",
                    downloaded as f64 * 100.0 / total as f64,
                    status
                        .recent_rate_kib_per_minute
                        .map_or_else(|| "unknown".to_owned(), |rate| format!("{rate} KiB/min")),
                    status
                        .estimated_seconds_remaining
                        .map_or_else(|| "unknown".to_owned(), |remaining| remaining.to_string())
                );
            } else {
                println!(
                    "Steam client update AppID {app_id}: {}",
                    status.state.as_str()
                );
            }
            last_progress = status.downloaded_kib;
        }
        if started.elapsed() >= Duration::from_secs(timeout_seconds) {
            return Err(format!(
                "Steam client update wait timed out after {timeout_seconds} seconds for AppID {app_id}"
            ));
        }
        thread::sleep(Duration::from_secs(interval_seconds));
    }
}

fn steam_session_status_command(args: &[String]) -> Result<(), String> {
    let json = match args {
        [] => false,
        [option] if option == "--json" => true,
        _ => return Err("usage: macroton steam session-status [--json]".to_owned()),
    };
    let state = macos_session::inspect().map_err(|error| error.to_string())?;
    let interactive_steam_allowed = state == macos_session::State::Unlocked;
    if json {
        println!("{{");
        println!("  \"schema_version\": 1,");
        println!("  \"console_state\": {:?},", state.as_str());
        println!(
            "  \"interactive_steam_allowed\": {},",
            interactive_steam_allowed
        );
        println!(
            "  \"action\": {:?}",
            if interactive_steam_allowed {
                "start-or-inspect-interactive-steam"
            } else {
                "unlock-macos-desktop"
            }
        );
        println!("}}");
    } else {
        println!("macOS console:             {}", state.as_str());
        println!(
            "interactive Windows Steam: {}",
            if interactive_steam_allowed {
                "allowed"
            } else {
                "blocked"
            }
        );
        if !interactive_steam_allowed {
            println!(
                "Action:                    unlock the macOS desktop before starting Windows Steam"
            );
        }
    }
    Ok(())
}

fn require_unlocked_console_session() -> Result<(), String> {
    match macos_session::inspect().map_err(|error| error.to_string())? {
        macos_session::State::Unlocked => Ok(()),
        macos_session::State::Locked => Err(
            "macOS console is locked; unlock the desktop before starting interactive Windows Steam"
                .to_owned(),
        ),
        macos_session::State::Unknown => Err(
            "cannot prove that the macOS console is unlocked; refusing interactive Windows Steam"
                .to_owned(),
        ),
    }
}

fn steam_login_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam login APPID --profile PATH [--runtime-root PATH] [--data-root PATH] [--max-rss-mib N] [--force] [--allow-ephemeral-data-root]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut runtime_root = default_runtime_root()?;
    let mut data_root = default_data_root()?;
    let mut profile_path = None;
    let mut max_rss_mib = 1000_u64;
    let mut force_login = false;
    let mut allow_ephemeral_data_root = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--allow-ephemeral-data-root" {
            allow_ephemeral_data_root = true;
            continue;
        }
        if option == "--force" {
            force_login = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--data-root" => data_root = PathBuf::from(value),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--max-rss-mib" => max_rss_mib = parse_positive_u64(value, option)?,
            other => return Err(format!("unknown steam login option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    require_persistent_data_root(&data_root, allow_ephemeral_data_root, "Windows Steam login")?;
    let prefix =
        steam_prefix_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    if prefix.readiness == steam_prefix_status::Readiness::NotBootstrapped {
        return Err("Windows Steam prefix has no valid Macroton bootstrap record".to_owned());
    }
    let reuse_persistent_login =
        prefix.readiness == steam_prefix_status::Readiness::StoredLoginPresent && !force_login;
    require_unlocked_console_session()?;
    if let Some(warning) = other_live_steam_prefix_warning(app_id, &data_root)? {
        eprintln!("Steam compatibility warning: {warning}");
    }
    let plan = plan::build_steam_client(&profile, &runtime_root);
    if !plan.is_ready() {
        return Err(format!(
            "Steam login is blocked by launch prerequisites: {}",
            plan.blockers.join("; ")
        ));
    }
    let request =
        execute::launch_steam_client(&plan, &data_root).map_err(|error| error.to_string())?;
    let (guard_pid, guard_log) = spawn_resource_guard_with_idle(
        app_id,
        &data_root,
        &runtime_root,
        &request.runtime_id,
        max_rss_mib,
        None,
        Some(4 * 60 * 60),
        Some(&resource_guard_identity(
            &plan,
            plan.runtime_fingerprint_sha256
                .as_deref()
                .ok_or_else(|| "ready Steam plan is missing its runtime fingerprint".to_owned())?,
        )),
        None,
        true,
        None,
    )?;
    let mut consecutive_live_samples = 0_usize;
    let mut observed_stable_steam_client = false;
    let needs_login_ui = prefix.readiness == steam_prefix_status::Readiness::SignInRequired
        && !reuse_persistent_login;
    let mut observed_login_ui = false;
    let update_log = request
        .client
        .parent()
        .map(|root| root.join("logs/bootstrap_log.txt"));
    let mut update_log_len = update_log
        .as_deref()
        .and_then(|path| fs::metadata(path).ok())
        .map_or(0, |metadata| metadata.len());
    let mut observed_update_progress = false;
    // A fresh Wine prefix may spend more than ten seconds starting services
    // before Steam.exe becomes visible, may restart once after bootstrapping,
    // or may first download a large client update without creating a login
    // window. Require ten continuous seconds of the real client. A growing
    // bootstrap log is reported distinctly as an updater, never as a login UI.
    for _ in 0..180 {
        let resources =
            stop::inspect_resources(app_id, &data_root).map_err(|error| error.to_string())?;
        let live = resources
            .processes
            .iter()
            .any(is_primary_steam_client_process);
        observed_login_ui = steam_login_ui_observed(&request.ui_log, request.ui_log_offset)?;
        if let Some(path) = update_log.as_deref() {
            let current_len = fs::metadata(path).map_or(0, |metadata| metadata.len());
            observed_update_progress |=
                steam_update_progress_observed(path, &mut update_log_len, current_len);
        }
        if steam_login_liveness_observed(&mut consecutive_live_samples, live)
            && (!needs_login_ui || observed_login_ui || observed_update_progress)
        {
            observed_stable_steam_client = true;
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    if !observed_stable_steam_client {
        let transport_hint = steam_connection_status::inspect(app_id, &data_root)
            .ok()
            .map(|status| {
                format!(
                    "transport={}, authenticated-context={}, ui-login={}, auth-poll-transport-error={}",
                    status.state.as_str(),
                    status.authenticated_context,
                    status.ui_login_observed,
                    status.auth_poll_transport_error
                )
            })
            .unwrap_or_else(|| "transport=no-observation".to_owned());
        let _ = stop::stop_with_runtime_id(
            app_id,
            &data_root,
            &runtime_root,
            Some(&request.runtime_id),
        );
        return Err(format!(
            "Windows Steam did not produce a usable login UI or a progressing client update and remain live for ten continuous seconds for AppID {app_id} ({transport_hint}); the prefix was stopped; inspect {}, {} and {}",
            request.log.display(),
            request.ui_log.display(),
            guard_log.display()
        ));
    }
    println!(
        "Windows Steam {} for AppID {app_id}",
        if reuse_persistent_login {
            "persistent session started"
        } else {
            "login requested"
        }
    );
    println!("PID:       {}", request.pid);
    println!("Steam UI:  {}", request.client_mode.as_str());
    println!("Profile:   {}", request.profile_sha256);
    println!("Client:    {}", request.client.display());
    println!(
        "Login UI:  {}",
        if reuse_persistent_login {
            "persistent-login-client-live"
        } else if needs_login_ui && observed_login_ui {
            "window-created-observed; visual-content-unverified"
        } else if needs_login_ui && observed_update_progress {
            "client-update-progress-observed; login-window-not-yet-created"
        } else {
            "stored-login-client-live"
        }
    );
    println!("Log:       {}", request.log.display());
    if request.client_mode == macroton::profile::SteamClientMode::CefDebugExperimental {
        let webhelper_log = data_root
            .join("games")
            .join(app_id.to_string())
            .join("prefix/drive_c/Program Files (x86)/Steam/logs/webhelper.txt");
        println!(
            "Cloud UI:  node scripts/resolve-steam-cloud-dialog.js --webhelper-log \"{}\" (read-only)",
            webhelper_log.display()
        );
    }
    println!(
        "Guard:     PID {guard_pid}, {max_rss_mib} MiB, {}",
        guard_log.display()
    );
    if reuse_persistent_login {
        println!("Reused the persistent Steam session; no credentials were requested.");
    } else {
        println!(
            "Enter credentials only in Valve's Windows Steam UI; Macroton does not read them."
        );
    }
    Ok(())
}

fn steam_login_wait_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam login-wait APPID [--data-root PATH] [--timeout-seconds N] [--interval-seconds N]".to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut timeout_seconds = 10 * 60_u64;
    let mut interval_seconds = 2_u64;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--timeout-seconds" => timeout_seconds = parse_positive_u64(value, option)?,
            "--interval-seconds" => interval_seconds = parse_positive_u64(value, option)?,
            other => return Err(format!("unknown steam login-wait option {other:?}")),
        }
        index += 1;
    }

    let client =
        steam_client_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    if client.state != steam_client_status::State::Current {
        return Err(format!(
            "Steam login verification requires a current client; updater state is {}",
            client.state.as_str()
        ));
    }

    let _idle_sleep_guard = IdleSleepGuard::acquire("Steam login verification")?;
    let started = Instant::now();
    let mut last_summary = None;
    loop {
        let prefix =
            steam_prefix_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
        let resources =
            stop::inspect_resources(app_id, &data_root).map_err(|error| error.to_string())?;
        let steam_live = resources
            .processes
            .iter()
            .any(is_primary_steam_client_process);
        let connection = steam_connection_status::inspect(app_id, &data_root).ok();
        let connected = connection.as_ref().is_some_and(|status| {
            status.state == steam_connection_status::TransportState::Connected
        });
        let authenticated_context = connection
            .as_ref()
            .is_some_and(|status| status.authenticated_context);
        let ui_login_observed = connection
            .as_ref()
            .is_some_and(|status| status.ui_login_observed);
        let auth_poll_transport_error = connection
            .as_ref()
            .is_some_and(|status| status.auth_poll_transport_error);
        let summary = format!(
            "prefix={}, stored-logins={}, userdata={}, steam-live={}, connected={}, authenticated-context={}, ui-login={}, auth-poll-transport-error={}",
            prefix.readiness.as_str(),
            prefix.stored_login_records,
            prefix.userdata_directories,
            steam_live,
            connected,
            authenticated_context,
            ui_login_observed,
            auth_poll_transport_error
        );
        if last_summary.as_deref() != Some(summary.as_str()) {
            println!("Steam login verification AppID {app_id}: {summary}");
            last_summary = Some(summary);
        }
        if steam_login_acceptance_observed(
            prefix.readiness,
            prefix.userdata_directories,
            connected,
            authenticated_context,
            steam_live,
        ) {
            println!(
                "Steam login verified for AppID {app_id}: persistent login, account userdata, authenticated connected transport, and live client observed"
            );
            return Ok(());
        }
        if !steam_live {
            if steam_persisted_login_acceptance_observed(
                prefix.readiness,
                prefix.userdata_directories,
                connected,
                authenticated_context,
                ui_login_observed,
            ) {
                println!(
                    "Steam login verified for AppID {app_id}: persistent login, account userdata, authenticated connected transport, and logged-in UI state persisted after client exit"
                );
                return Ok(());
            }
            return Err(format!(
                "Windows Steam exited before login verification completed for AppID {app_id}"
            ));
        }
        if started.elapsed() >= Duration::from_secs(timeout_seconds) {
            return Err(format!(
                "Steam login verification timed out after {timeout_seconds} seconds for AppID {app_id}; last observation: {}",
                last_summary.as_deref().unwrap_or("none")
            ));
        }
        thread::sleep(Duration::from_secs(interval_seconds));
    }
}

fn steam_persisted_login_acceptance_observed(
    readiness: steam_prefix_status::Readiness,
    userdata_directories: usize,
    connected: bool,
    authenticated_context: bool,
    ui_login_observed: bool,
) -> bool {
    readiness == steam_prefix_status::Readiness::StoredLoginPresent
        && userdata_directories > 0
        && connected
        && authenticated_context
        && ui_login_observed
}

fn steam_login_acceptance_observed(
    readiness: steam_prefix_status::Readiness,
    userdata_directories: usize,
    connected: bool,
    authenticated_context: bool,
    steam_live: bool,
) -> bool {
    readiness == steam_prefix_status::Readiness::StoredLoginPresent
        && userdata_directories > 0
        && connected
        && authenticated_context
        && steam_live
}

#[cfg(test)]
fn live_steam_authentication_observed(
    connection: &steam_connection_status::ConnectionStatus,
    resources: &stop::PrefixResourceUsage,
) -> bool {
    connection.state == steam_connection_status::TransportState::Connected
        && connection.authenticated_context
        && resources
            .processes
            .iter()
            .any(is_primary_steam_client_process)
}

const STEAM_LOGIN_STABLE_SAMPLES: usize = 40;

fn steam_login_liveness_observed(consecutive_live_samples: &mut usize, live: bool) -> bool {
    if live {
        *consecutive_live_samples += 1;
    } else {
        *consecutive_live_samples = 0;
    }
    *consecutive_live_samples >= STEAM_LOGIN_STABLE_SAMPLES
}

fn steam_update_progress_observed(path: &Path, previous_len: &mut u64, current_len: u64) -> bool {
    if current_len <= *previous_len {
        return false;
    }
    let appended = fs::read(path)
        .ok()
        .and_then(|bytes| {
            usize::try_from(*previous_len)
                .ok()
                .and_then(|offset| bytes.get(offset..).map(|slice| slice.to_vec()))
        })
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default();
    *previous_len = current_len;
    [
        "Downloading update (",
        "Download complete.",
        "Installing update...",
        "Extracting package...",
        "Update complete, launching Steam...",
    ]
    .iter()
    .any(|marker| appended.contains(marker))
}

fn steam_login_ui_observed(path: &Path, offset: u64) -> Result<bool, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "cannot inspect Steam login UI log {}: {error}",
                path.display()
            ));
        }
    };
    Ok(steam_login_ui_marker_observed(&bytes, offset))
}

fn steam_login_ui_marker_observed(bytes: &[u8], offset: u64) -> bool {
    let Ok(offset) = usize::try_from(offset) else {
        return false;
    };
    let Some(appended) = bytes.get(offset..) else {
        return false;
    };
    let appended = String::from_utf8_lossy(appended);
    appended.contains("SP DesktopLoginWindow_") && appended.contains("WasHidden 0")
}

fn steam_cloud_wait_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam cloud-wait APPID --profile PATH [--data-root PATH | --steam-root PATH] [--timeout-seconds N] [--interval-seconds N]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut data_root = default_data_root()?;
    let mut data_root_explicit = false;
    let mut steam_root = None;
    let mut timeout_seconds = 300_u64;
    let mut interval_seconds = 2_u64;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--data-root" => {
                data_root = PathBuf::from(value);
                data_root_explicit = true;
            }
            "--steam-root" => steam_root = Some(PathBuf::from(value)),
            "--timeout-seconds" => timeout_seconds = parse_positive_u64(value, option)?,
            "--interval-seconds" => interval_seconds = parse_positive_u64(value, option)?,
            other => return Err(format!("unknown steam cloud-wait option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    if profile.cloud_files.is_empty() {
        return Err(
            "cloud-wait requires profile cloud_files so cached metadata is checked against real local and remote-copy content"
                .to_owned(),
        );
    }
    if data_root_explicit && steam_root.is_some() {
        return Err("cloud-wait accepts either --data-root or --steam-root, not both".to_owned());
    }
    let _idle_sleep_guard = IdleSleepGuard::acquire("Steam Cloud reconciliation")?;
    let started = Instant::now();
    let mut last_state = None;
    let mut last_pending_operations = None;
    let mut last_remote_freshness = None;
    loop {
        let mut status = if let Some(steam_root) = steam_root.as_deref() {
            cloud_status::inspect_steam_root(app_id, steam_root)
        } else {
            cloud_status::inspect(app_id, &data_root)
        }
        .map_err(|error| error.to_string())?;
        cloud_status::compare_profile_files(&mut status, &profile)
            .map_err(|error| error.to_string())?;
        if last_state != Some(status.state) {
            println!(
                "Steam Cloud AppID {app_id}: {} after {}s",
                status.state.as_str(),
                started.elapsed().as_secs()
            );
            last_state = Some(status.state);
        }
        if last_remote_freshness != Some(status.remote_freshness) {
            println!("Steam Cloud AppID {app_id}: remote freshness {} (local cache match cannot establish latest other-device data)", status.remote_freshness.as_str());
            last_remote_freshness = Some(status.remote_freshness);
        }
        if last_pending_operations != Some(status.pending_remote_operations.len()) {
            if !status.pending_remote_operations.is_empty() {
                println!(
                    "Steam Cloud AppID {app_id}: {} client-side pending launch operation(s); waiting for Steam to resolve them",
                    status.pending_remote_operations.len()
                );
            }
            last_pending_operations = Some(status.pending_remote_operations.len());
        }
        if cloud_launch_ready(status.state, status.pending_remote_operations.len(), status.remote_freshness) {
            let remote_copy_matches = status
                .comparisons
                .iter()
                .filter(|comparison| comparison.remote_copy_size.is_some())
                .count();
            let cache_direct_matches = status.comparisons.len() - remote_copy_matches;
            println!(
                "Local Cloud cache gate: passed; {} declared file(s) match ({} remote-copy content, {} synced-cache direct). Latest data on other devices remains unverified",
                status.comparisons.len(),
                remote_copy_matches,
                cache_direct_matches
            );
            return Ok(());
        }
        if started.elapsed() >= Duration::from_secs(timeout_seconds) {
            return Err(format!(
                "Steam Cloud did not reach content-verified synced and launch-ready within {timeout_seconds}s; final cache state {}, remote freshness {}, pending client operations {}",
                status.state.as_str(),
                status.remote_freshness.as_str(),
                status.pending_remote_operations.len()
            ));
        }
        thread::sleep(Duration::from_secs(interval_seconds));
    }
}

fn cloud_launch_ready(state: cloud_status::SyncState, pending_operations: usize, remote_freshness: cloud_status::RemoteFreshness) -> bool {
    state == cloud_status::SyncState::Synced && pending_operations == 0
        && remote_freshness == cloud_status::RemoteFreshness::Unverified
}

fn steam_trim_client_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam trim-client APPID --profile PATH [--data-root PATH] [--runtime-root PATH]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut profile_path = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown steam trim-client option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    if profile.launch_mode != macroton::profile::LaunchMode::Steam {
        return Err("steam trim-client requires a Steam launch profile".to_owned());
    }
    let launch = launch_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    let before = stop::inspect_resources(app_id, &data_root).map_err(|error| error.to_string())?;
    let game_pid = trim_client_game_pid(&launch, &before.processes)?;
    if !before
        .processes
        .iter()
        .any(|process| is_windows_steam_resource_category(process.category))
    {
        return Err(
            "refusing trim-client because no exact-prefix Windows Steam process is currently resident"
                .to_owned(),
        );
    }
    let prefix = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix")
        .canonicalize()
        .map_err(|error| format!("cannot resolve AppID prefix: {error}"))?;
    let steam = prefix.join("drive_c/Program Files (x86)/Steam/Steam.exe");
    if !steam.is_file() {
        return Err(format!(
            "Windows Steam client is missing: {}",
            steam.display()
        ));
    }
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    let installed = runtime::resolve_installed(&runtime_root, &runtime_id)
        .map_err(|error| error.to_string())?;
    steam_session::validate(
        &prefix,
        &steam_session::Identity {
            app_id,
            profile_sha256: profile.fingerprint(),
            runtime_id: runtime_id.clone(),
            runtime_fingerprint_sha256: installed.fingerprint_sha256.clone(),
            steam_client_mode: profile.steam_client_mode.as_str().to_owned(),
            renderer: profile.renderer.clone(),
            graphics_api: profile.graphics_api.as_str().to_owned(),
        },
    )
    .map_err(|error| format!("trim-client active Steam session mismatch: {error}"))?;
    let steam_session_sha256 = sha256::to_hex(
        &sha256::digest_file(&prefix.join(steam_session::MARKER))
            .map_err(|error| format!("cannot hash trim-client Steam session marker: {error}"))?,
    );
    Command::new(&installed.entrypoint)
        .arg(&steam)
        .arg("-shutdown")
        .env("WINEPREFIX", &prefix)
        .current_dir(steam.parent().expect("Steam.exe has a parent"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot submit official Steam shutdown: {error}"))?;

    for _ in 0..60 {
        thread::sleep(Duration::from_millis(500));
        let current =
            stop::inspect_resources(app_id, &data_root).map_err(|error| error.to_string())?;
        if !current.pids.contains(&game_pid) {
            return Err(
                "game process exited while stopping Steam; launch-only client lifetime is rejected for this run"
                    .to_owned(),
            );
        }
        let steam_resident = current
            .processes
            .iter()
            .any(|process| is_windows_steam_resource_category(process.category));
        if !steam_resident {
            let evidence = persist_trim_client_evidence(
                app_id,
                &data_root,
                &TrimClientEvidenceObservation {
                    profile: &profile,
                    runtime_fingerprint_sha256: &installed.fingerprint_sha256,
                    steam_session_sha256: &steam_session_sha256,
                    game_pid,
                    before: &before,
                    after: &current,
                },
            )?;
            println!("AppID:       {app_id}");
            println!("Game PID:    {game_pid} (still exact-prefix resident)");
            println!(
                "RSS:         {:.1} -> {:.1} MiB",
                before.total_rss_kib as f64 / 1024.0,
                current.total_rss_kib as f64 / 1024.0
            );
            println!("Steam:       stopped after verified game start");
            println!("Evidence:    {}", evidence.display());
            println!(
                "Boundary:    experimental; restart official Steam after game exit and require Cloud synced before promotion"
            );
            return Ok(());
        }
    }
    Err(
        "Windows Steam/CEF remained resident 30 seconds after official -shutdown request"
            .to_owned(),
    )
}

struct TrimClientEvidenceObservation<'a> {
    profile: &'a CompatibilityProfile,
    runtime_fingerprint_sha256: &'a str,
    steam_session_sha256: &'a str,
    game_pid: u32,
    before: &'a stop::PrefixResourceUsage,
    after: &'a stop::PrefixResourceUsage,
}

fn persist_trim_client_evidence(
    app_id: u32,
    data_root: &Path,
    observation: &TrimClientEvidenceObservation<'_>,
) -> Result<PathBuf, String> {
    let captured_unix_ms = unix_time_ms()?;
    let evidence_root = data_root
        .join("games")
        .join(app_id.to_string())
        .join("trim-client-evidence");
    fs::create_dir_all(&evidence_root)
        .map_err(|error| format!("cannot create trim-client evidence directory: {error}"))?;
    let destination = evidence_root.join(format!("{captured_unix_ms}-{}.json", std::process::id()));
    let record = trim_client_evidence_json(app_id, captured_unix_ms, observation);
    write_new_evidence(&destination, record.as_bytes())?;
    Ok(destination)
}

fn trim_client_evidence_json(
    app_id: u32,
    captured_unix_ms: u128,
    observation: &TrimClientEvidenceObservation<'_>,
) -> String {
    let TrimClientEvidenceObservation {
        profile,
        runtime_fingerprint_sha256,
        steam_session_sha256,
        game_pid,
        before,
        after,
    } = observation;
    let rss_reduction_kib = before.total_rss_kib.saturating_sub(after.total_rss_kib);
    format!(
        concat!(
            "{{\n  \"schema\": 1,\n  \"captured_unix_ms\": {captured_unix_ms},\n",
            "  \"appid\": {app_id},\n  \"profile_sha256\": \"{}\",\n",
            "  \"runtime_id\": \"{}-{}\",\n  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"steam_client_mode\": \"{}\",\n  \"steam_session_sha256\": \"{}\",\n",
            "  \"game_pid\": {game_pid},\n  \"game_still_resident\": {},\n",
            "  \"before_rss_kib\": {},\n  \"after_rss_kib\": {},\n",
            "  \"rss_reduction_kib\": {rss_reduction_kib},\n  \"steam_resident_after\": {}\n}}\n"
        ),
        profile.fingerprint(),
        json_escape(&profile.runtime),
        json_escape(&profile.runtime_version),
        runtime_fingerprint_sha256,
        profile.steam_client_mode.as_str(),
        steam_session_sha256,
        after.pids.contains(game_pid),
        before.total_rss_kib,
        after.total_rss_kib,
        after
            .processes
            .iter()
            .any(|process| is_windows_steam_resource_category(process.category)),
        captured_unix_ms = captured_unix_ms,
        app_id = app_id,
        game_pid = game_pid,
        rss_reduction_kib = rss_reduction_kib,
    )
}

fn trim_client_game_pid(
    launch: &launch_status::LaunchStatus,
    prefix_processes: &[stop::ProcessResourceUsage],
) -> Result<u32, String> {
    if launch.state != launch_status::LaunchState::RunningObserved {
        return Err(format!(
            "refusing to stop Steam before a real tracked game process is running; observed {}",
            launch.state.as_str()
        ));
    }
    let game_pid = launch
        .process_id
        .ok_or_else(|| "running launch evidence has no game PID".to_owned())?;
    let Some(process) = prefix_processes
        .iter()
        .find(|process| process.pid == game_pid)
    else {
        return Err(format!(
            "Steam's latest game PID {game_pid} is not currently owned by the exact AppID prefix"
        ));
    };
    if process.category != "windows-game" {
        return Err(format!(
            "Steam's latest game PID {game_pid} is classified as {} instead of windows-game",
            process.category
        ));
    }
    Ok(game_pid)
}

fn steam_prerequisite_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam prerequisite-status APPID [--data-root PATH] [--json]".to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                json = true;
                index += 1;
            }
            "--data-root" => {
                index += 1;
                data_root = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--data-root requires a path".to_owned())?,
                );
                index += 1;
            }
            other => return Err(format!("unknown prerequisite-status option {other:?}")),
        }
    }
    let status =
        prerequisite_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    if json {
        println!("{{");
        println!("  \"schema\": 1,");
        println!("  \"appid\": {app_id},");
        println!("  \"state\": \"{}\",", status.state.as_str());
        println!("  \"signature_present\": {},", status.signature_present);
        println!("  \"requirements\": [");
        for (index, requirement) in status.requirements.iter().enumerate() {
            println!(
                "    {{\"id\": \"{}\", \"description\": {}}}{}",
                json_escape(&requirement.id),
                requirement
                    .description
                    .as_deref()
                    .map(|value| format!("\"{}\"", json_escape(value)))
                    .unwrap_or_else(|| "null".to_owned()),
                if index + 1 == status.requirements.len() {
                    ""
                } else {
                    ","
                }
            );
        }
        println!("  ]");
        println!("}}");
    } else {
        println!("AppID:      {app_id}");
        println!("State:      {}", status.state.as_str());
        println!(
            "Signature:  {}",
            if status.signature_present {
                "present"
            } else {
                "missing"
            }
        );
        for requirement in &status.requirements {
            println!(
                "Requirement: {}{}",
                requirement.id,
                requirement
                    .description
                    .as_deref()
                    .map(|description| format!(" ({description})"))
                    .unwrap_or_default()
            );
        }
        if let Some(event) = status.latest_event {
            println!("Latest:     {event}");
        }
    }
    Ok(())
}

fn steam_install_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam install APPID --profile PATH [--runtime-root PATH] [--data-root PATH] [--max-rss-mib N] [--allow-ephemeral-data-root]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut runtime_root = default_runtime_root()?;
    let mut data_root = default_data_root()?;
    let mut profile_path = None;
    let mut max_rss_mib = 1000_u64;
    let mut allow_ephemeral_data_root = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--allow-ephemeral-data-root" {
            allow_ephemeral_data_root = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--data-root" => data_root = PathBuf::from(value),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--max-rss-mib" => max_rss_mib = parse_positive_u64(value, option)?,
            other => return Err(format!("unknown steam install option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!(
            "profile AppID {} does not match requested AppID {app_id}",
            profile.app_id
        ));
    }
    require_persistent_data_root(
        &data_root,
        allow_ephemeral_data_root,
        "Windows Steam install",
    )?;
    require_unlocked_console_session()?;
    let prefix =
        steam_prefix_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    if prefix.readiness != steam_prefix_status::Readiness::StoredLoginPresent {
        return Err(format!(
            "Windows Steam install requires stored-login-present; prefix is {}",
            prefix.readiness.as_str()
        ));
    }
    let client =
        steam_client_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    if client.state != steam_client_status::State::Current {
        return Err(format!(
            "Windows Steam install requires a current client; updater state is {}",
            client.state.as_str()
        ));
    }
    // Do not require a separately live client here.  Starting a second
    // Steam.exe just to dispatch steam://install can replace an existing
    // session before it has finished reconnecting.  The persistent-login
    // prefix is the authority for this bounded request; Steam itself remains
    // responsible for authenticating the account and accepting the install
    // URI after startup.  install-status/login-wait provide the stricter
    // post-submission gates.
    let plan = plan::build_steam_client(&profile, &runtime_root);
    if !plan.is_ready() {
        return Err(format!(
            "Steam install is blocked by client prerequisites: {}",
            plan.blockers.join("; ")
        ));
    }
    // Steam's single-instance client drops install URIs submitted during its
    // CEF/bootstrap window. Warm the exact-prefix client first, then dispatch
    // the URI through that authenticated instance. If a client is already
    // resident, reuse it and avoid creating another CEF tree.
    if !execute::steam_session_processes_present(app_id, &data_root) {
        let warmup =
            execute::launch_steam_client(&plan, &data_root).map_err(|error| error.to_string())?;
        execute::wait_for_steam_client_ready(&warmup, Duration::from_secs(90));
    }
    let request =
        execute::request_steam_install(&plan, &data_root).map_err(|error| error.to_string())?;
    let (guard_pid, guard_log) = spawn_resource_guard(
        app_id,
        &data_root,
        &runtime_root,
        &request.runtime_id,
        max_rss_mib,
        None,
        Some(4 * 60 * 60),
        Some(&resource_guard_identity(
            &plan,
            plan.runtime_fingerprint_sha256
                .as_deref()
                .ok_or_else(|| "ready Steam plan is missing its runtime fingerprint".to_owned())?,
        )),
        None,
    )?;
    println!("Windows Steam install requested for AppID {app_id}");
    println!("PID:       {}", request.pid);
    println!("Profile:   {}", request.profile_sha256);
    println!("Runtime:   {}", request.runtime_id);
    println!("Client:    {}", request.client.display());
    println!("Log:       {}", request.log.display());
    println!(
        "Guard:     PID {guard_pid}, {max_rss_mib} MiB, {}",
        guard_log.display()
    );
    println!("Progress:  macroton steam install-status {app_id}");
    Ok(())
}

fn steam_install_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam install-status APPID [--data-root PATH | --steam-root PATH] [--json]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut steam_root = None;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--steam-root" => steam_root = Some(PathBuf::from(value)),
            other => return Err(format!("unknown steam install-status option {other:?}")),
        }
        index += 1;
    }
    let status = if let Some(steam_root) = steam_root {
        steam_install_status::inspect_steam_root(app_id, &steam_root)
    } else if let Some((owner, _)) = read_shared_client_marker(app_id, &data_root)? {
        let steam_root = data_root.join("games").join(owner.to_string())
            .join("prefix/drive_c/Program Files (x86)/Steam");
        steam_install_status::inspect_steam_root(app_id, &steam_root)
    } else {
        steam_install_status::inspect(app_id, &data_root)
    }
    .map_err(|error| error.to_string())?;

    if json {
        println!("{}", install_status_json(&status));
        return Ok(());
    }

    println!("Steam install status for AppID {}", status.app_id);
    println!("State:       {}", status.state.as_str());
    println!(
        "Name:        {}",
        status.name.as_deref().unwrap_or("unknown")
    );
    println!(
        "Build:       {} -> {}",
        optional_u64(status.installed_build_id),
        optional_u64(status.target_build_id)
    );
    println!(
        "Payload:     {} manifest bytes; directory {}; symlink {}; entries {}",
        optional_u64(status.manifest_size_on_disk),
        if status.installed_payload_directory_present {
            "present"
        } else {
            "missing"
        },
        status.installed_payload_directory_is_symlink,
        status.installed_payload_entry_observed
    );
    println!(
        "Download:    {} / {} manifest-checkpoint bytes",
        optional_u64(status.manifest_bytes_downloaded),
        optional_u64(status.bytes_to_download)
    );
    println!(
        "Stage:       {} / {} manifest-checkpoint bytes",
        optional_u64(status.manifest_bytes_staged),
        optional_u64(status.bytes_to_stage)
    );
    println!(
        "Resume:      {} / {} download; {} / {} staged bytes at latest attempt start",
        optional_u64(status.attempt_start_downloaded_bytes),
        optional_u64(status.attempt_download_total_bytes),
        optional_u64(status.attempt_start_staged_bytes),
        optional_u64(status.attempt_stage_total_bytes)
    );
    println!(
        "Network:     {} bytes observed in latest content-log attempt",
        optional_u64(status.observed_network_bytes)
    );
    println!(
        "Rate:        {}",
        status
            .latest_rate_mbps
            .map(|rate| format!("{rate:.3} Mbps"))
            .unwrap_or_else(|| "not observed".to_owned())
    );
    println!("Failures:    {} recent", status.recent_connection_failures);
    println!("Selected:    {}", format_u32_list(&status.selected_depots));
    println!("Installed:   {}", format_u32_list(&status.installed_depots));
    println!(
        "Observed:    {}",
        status
            .latest_observed_at
            .as_deref()
            .unwrap_or("not observed")
    );
    println!("Manifest:    {}", status.manifest_path.display());
    println!("Content log: {}", status.content_log_path.display());
    println!(
        "Evidence:    installed requires nonzero SizeOnDisk plus a nonempty payload; Macroton payload-views symlinks are accepted, external no-copy links are not"
    );
    Ok(())
}

fn steam_install_wait_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam install-wait APPID [--data-root PATH] [--timeout-seconds N] [--request-timeout-seconds N] [--interval-seconds N]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut timeout_seconds = 4 * 60 * 60_u64;
    let mut request_timeout_seconds = 60_u64;
    let mut interval_seconds = 5_u64;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--timeout-seconds" => timeout_seconds = parse_positive_u64(value, option)?,
            "--request-timeout-seconds" => {
                request_timeout_seconds = parse_positive_u64(value, option)?
            }
            "--interval-seconds" => interval_seconds = parse_positive_u64(value, option)?,
            other => return Err(format!("unknown steam install-wait option {other:?}")),
        }
        index += 1;
    }
    let _idle_sleep_guard = IdleSleepGuard::acquire("Steam game installation")?;
    let started = Instant::now();
    let mut last_summary = None;
    let mut reported_confirmation = false;
    let mut reported_eula = false;
    loop {
        let status =
            steam_install_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
        match status.state {
            steam_install_status::InstallState::Installed => {
                println!(
                    "Windows Steam installation completed for AppID {app_id}: build {}, {} installed depot(s)",
                    optional_u64(status.installed_build_id),
                    status.installed_depots.len()
                );
                return Ok(());
            }
            steam_install_status::InstallState::Failed => {
                return Err(format!(
                    "Windows Steam installation failed for AppID {app_id}"
                ));
            }
            steam_install_status::InstallState::Stalled => {
                return Err(format!(
                    "Windows Steam installation stalled for AppID {app_id} after {} recent connection failure(s)",
                    status.recent_connection_failures
                ));
            }
            steam_install_status::InstallState::EulaPromptUnconfirmed => {
                return Err(format!(
                    "Windows Steam previously prompted for the AppID {app_id} EULA, but the prompt is no longer fresh; inspect the visible client before retrying, and leave the legal choice to the account owner"
                ));
            }
            steam_install_status::InstallState::NotRequested
                if started.elapsed() >= Duration::from_secs(request_timeout_seconds) =>
            {
                return Err(format!(
                    "Windows Steam did not record an installation request for AppID {app_id} within {request_timeout_seconds} seconds; open the visible Wine Steam client and confirm Install there, then rerun install-wait"
                ));
            }
            steam_install_status::InstallState::NotRequested
            | steam_install_status::InstallState::AwaitingConfirmation
            | steam_install_status::InstallState::AwaitingEula
            | steam_install_status::InstallState::Queued
            | steam_install_status::InstallState::Downloading
            | steam_install_status::InstallState::ManifestOnly => {}
        }
        let summary = format!(
            "{}:{}:{}:{}",
            status.state.as_str(),
            optional_u64(status.manifest_bytes_downloaded),
            optional_u64(status.bytes_to_download),
            optional_u64(status.observed_network_bytes)
        );
        if status.state == steam_install_status::InstallState::AwaitingConfirmation
            && !reported_confirmation
        {
            println!(
                "Windows Steam is waiting for Install confirmation for AppID {app_id}; confirm Install in the visible Wine Steam Library UI"
            );
            reported_confirmation = true;
        }
        if status.state == steam_install_status::InstallState::AwaitingEula
            && !reported_eula
        {
            println!(
                "Windows Steam is waiting for the account owner to read and decide the EULA for AppID {app_id}; Macroton will not accept it"
            );
            reported_eula = true;
        }
        if last_summary.as_deref() != Some(summary.as_str()) {
            println!(
                "Windows Steam install AppID {app_id}: {}; manifest {} / {} bytes; observed network {} bytes",
                status.state.as_str(),
                optional_u64(status.manifest_bytes_downloaded),
                optional_u64(status.bytes_to_download),
                optional_u64(status.observed_network_bytes)
            );
            last_summary = Some(summary);
        }
        if started.elapsed() >= Duration::from_secs(timeout_seconds) {
            return Err(format!(
                "Windows Steam install wait timed out after {timeout_seconds} seconds for AppID {app_id}"
            ));
        }
        thread::sleep(Duration::from_secs(interval_seconds));
    }
}

fn install_status_json(status: &steam_install_status::InstallStatus) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"schema\": 2,\n",
            "  \"appid\": {},\n",
            "  \"state\": \"{}\",\n",
            "  \"name\": {},\n",
            "  \"installed_build_id\": {},\n",
            "  \"target_build_id\": {},\n",
            "  \"manifest_size_on_disk\": {},\n",
            "  \"installed_payload_directory_present\": {},\n",
            "  \"installed_payload_directory_is_symlink\": {},\n",
            "  \"installed_payload_entry_observed\": {},\n",
            "  \"bytes_to_download\": {},\n",
            "  \"manifest_bytes_downloaded\": {},\n",
            "  \"bytes_to_stage\": {},\n",
            "  \"manifest_bytes_staged\": {},\n",
            "  \"attempt_start_downloaded_bytes\": {},\n",
            "  \"attempt_download_total_bytes\": {},\n",
            "  \"attempt_start_staged_bytes\": {},\n",
            "  \"attempt_stage_total_bytes\": {},\n",
            "  \"observed_network_bytes\": {},\n",
            "  \"latest_rate_mbps\": {},\n",
            "  \"recent_connection_failures\": {},\n",
            "  \"selected_depots\": {},\n",
            "  \"installed_depots\": {},\n",
            "  \"latest_observed_at\": {},\n",
            "  \"evidence\": \"installed requires nonzero SizeOnDisk plus a nonempty payload; Macroton payload-views symlinks are accepted, external no-copy links remain plan-only\"\n",
            "}}"
        ),
        status.app_id,
        status.state.as_str(),
        json_optional_string(status.name.as_deref()),
        json_optional_u64(status.installed_build_id),
        json_optional_u64(status.target_build_id),
        json_optional_u64(status.manifest_size_on_disk),
        status.installed_payload_directory_present,
        status.installed_payload_directory_is_symlink,
        status.installed_payload_entry_observed,
        json_optional_u64(status.bytes_to_download),
        json_optional_u64(status.manifest_bytes_downloaded),
        json_optional_u64(status.bytes_to_stage),
        json_optional_u64(status.manifest_bytes_staged),
        json_optional_u64(status.attempt_start_downloaded_bytes),
        json_optional_u64(status.attempt_download_total_bytes),
        json_optional_u64(status.attempt_start_staged_bytes),
        json_optional_u64(status.attempt_stage_total_bytes),
        json_optional_u64(status.observed_network_bytes),
        status
            .latest_rate_mbps
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_owned()),
        status.recent_connection_failures,
        json_u32_array(&status.selected_depots),
        json_u32_array(&status.installed_depots),
        json_optional_string(status.latest_observed_at.as_deref()),
    )
}

fn json_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_owned())
}

fn json_optional_string(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", json_escape(value)))
        .unwrap_or_else(|| "null".to_owned())
}

fn json_u32_array(values: &[u32]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn format_u32_list(values: &[u32]) -> String {
    if values.is_empty() {
        "none".to_owned()
    } else {
        values
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn inventory_selection_label(
    candidates_only: bool,
    verified_win32_only: bool,
    two_d_only: bool,
) -> &'static str {
    if two_d_only && verified_win32_only {
        "verified-win32-2d-only"
    } else if two_d_only {
        "2d-only"
    } else if verified_win32_only {
        "verified-win32-only"
    } else if candidates_only {
        "candidates-only"
    } else {
        "all"
    }
}

fn inventory_installed_evidence(
    steam_root: &Path,
    data_root: Option<&Path>,
    app_ids: impl Iterator<Item = u32>,
) -> Result<BTreeMap<u32, InstalledTargetEvidence>, String> {
    let mut installed = steam::scan(steam_root)
        .unwrap_or_default()
        .into_iter()
        .map(|game| (game.app_id, installed_target_evidence(game)))
        .collect::<BTreeMap<_, _>>();
    let Some(data_root) = data_root else {
        return Ok(installed);
    };
    for app_id in app_ids {
        // Only an AppID's own isolated prefix may contribute its PE evidence.
        // A different prefix's appmanifest must never promote this title.
        if installed.contains_key(&app_id) {
            continue;
        }
        let root = data_root
            .join("games")
            .join(app_id.to_string())
            .join("prefix/drive_c/Program Files (x86)/Steam");
        let manifest = root.join("steamapps").join(format!("appmanifest_{app_id}.acf"));
        match fs::symlink_metadata(&manifest) {
            Ok(metadata) if metadata.file_type().is_file() => {}
            Ok(_) => return Err(format!("exact-prefix Steam manifest is not a regular file: {}", manifest.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("cannot inspect exact-prefix Steam manifest {}: {error}", manifest.display())),
        }
        let install = steam_install_status::inspect_steam_root(app_id, &root)
            .map_err(|error| format!("cannot inspect AppID {app_id} exact-prefix install: {error}"))?;
        if install.state != steam_install_status::InstallState::Installed {
            continue;
        }
        let game = steam::scan(&root)
            .map_err(|error| format!("cannot scan AppID {app_id} exact-prefix payload: {error}"))?
            .into_iter()
            .find(|game| game.app_id == app_id);
        if let Some(game) = game {
            installed.insert(app_id, installed_target_evidence(game));
        }
    }
    Ok(installed)
}

fn installed_target_evidence(game: steam::SteamGame) -> InstalledTargetEvidence {
    InstalledTargetEvidence {
        classification: game.classification,
        pe_binaries: game
            .binaries
            .into_iter()
            .filter_map(|binary| Some((binary.relative_path, binary.pe_architecture?)))
            .collect(),
    }
}

fn steam_inventory_command(args: &[String]) -> Result<(), String> {
    let mut steam_root = None;
    let mut data_root = None;
    let mut eligibility_dir = None;
    let mut min_playtime = 1_u64;
    let mut limit = 50_usize;
    let mut with_appinfo = false;
    let mut candidates_only = false;
    let mut verified_win32_only = false;
    let mut two_d_only = false;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--with-appinfo" {
            with_appinfo = true;
            continue;
        }
        if option == "--candidates-only" {
            candidates_only = true;
            with_appinfo = true;
            continue;
        }
        if option == "--verified-win32-only" {
            verified_win32_only = true;
            with_appinfo = true;
            continue;
        }
        if option == "--2d-only" {
            two_d_only = true;
            candidates_only = true;
            with_appinfo = true;
            continue;
        }
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--steam-root" => steam_root = Some(PathBuf::from(value)),
            "--data-root" => data_root = Some(PathBuf::from(value)),
            "--eligibility-dir" => {
                eligibility_dir = Some(PathBuf::from(value));
                with_appinfo = true;
            }
            "--min-playtime" => {
                min_playtime = value
                    .parse()
                    .map_err(|_| "--min-playtime must be an unsigned integer".to_owned())?;
            }
            "--limit" => {
                limit = value
                    .parse::<usize>()
                    .map_err(|_| "--limit must be an unsigned integer".to_owned())?;
                if limit == 0 {
                    return Err("--limit must be greater than zero".to_owned());
                }
            }
            other => return Err(format!("unknown steam inventory option {other:?}")),
        }
        index += 1;
    }
    let steam_root = steam_root.ok_or_else(|| {
        "usage: macroton steam inventory --steam-root PATH [--data-root PATH] [--eligibility-dir PATH] [--min-playtime MINUTES] [--limit COUNT] [--with-appinfo] [--candidates-only] [--verified-win32-only] [--2d-only] [--json]"
            .to_owned()
    })?;
    let mut inventory =
        steam_library_status::scan_steam_root(&steam_root).map_err(|error| error.to_string())?;
    inventory.entries.retain(|entry| {
        entry.max_playtime_minutes.unwrap_or(0) >= min_playtime || entry.cached_app_tickets > 0
    });
    inventory.entries.sort_by(|first, second| {
        second
            .max_playtime_minutes
            .unwrap_or(0)
            .cmp(&first.max_playtime_minutes.unwrap_or(0))
            .then_with(|| {
                second
                    .latest_last_played_unix
                    .unwrap_or(0)
                    .cmp(&first.latest_last_played_unix.unwrap_or(0))
            })
            .then_with(|| first.app_id.cmp(&second.app_id))
    });
    let installed_evidence = inventory_installed_evidence(
        &steam_root,
        data_root.as_deref(),
        inventory.entries.iter().map(|entry| entry.app_id),
    )?;
    let eligibility_reviews = eligibility_dir
        .as_deref()
        .map(|directory| {
            load_inventory_eligibility_reviews(
                directory,
                inventory.entries.iter().map(|entry| entry.app_id),
            )
        })
        .transpose()?
        .unwrap_or_default();
    if candidates_only || verified_win32_only {
        inventory.entries.retain(|entry| {
            steam_appinfo::inspect_steam_root(entry.app_id, &steam_root)
                .map(|metadata| {
                    let gate = inventory_target_gate(
                        &metadata,
                        installed_evidence.get(&entry.app_id),
                        eligibility_reviews.get(&entry.app_id),
                    );
                    if two_d_only && !review_scope_allows_2d(eligibility_reviews.get(&entry.app_id))
                    {
                        return false;
                    }
                    if verified_win32_only {
                        strict_verified_inventory_candidate(
                            &metadata,
                            &gate,
                            eligibility_reviews.get(&entry.app_id),
                        )
                    } else {
                        is_candidate_gate(&gate)
                    }
                })
                .unwrap_or(false)
        });
    }
    if json {
        let records = inventory
            .entries
            .iter()
            .take(limit)
            .map(|entry| {
                let metadata = if with_appinfo {
                    Some(steam_appinfo::inspect_steam_root(entry.app_id, &steam_root))
                } else {
                    None
                };
                let review = eligibility_reviews.get(&entry.app_id);
                let (name, platforms, gate) = match metadata {
                    Some(Ok(metadata)) => (
                        metadata.name.clone(),
                        Some(cached_app_platforms(&metadata)),
                        Some(inventory_target_gate(
                            &metadata,
                            installed_evidence.get(&entry.app_id),
                            review,
                        )),
                    ),
                    Some(Err(_)) => (None, None, Some("metadata-incomplete".to_owned())),
                    None => (None, None, None),
                };
                inventory_entry_json(
                    entry,
                    name.as_deref(),
                    platforms.as_deref(),
                    gate.as_deref(),
                    review,
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        println!(
            concat!(
                "{{\n",
                "  \"schema_version\": 2,\n",
                "  \"account_identifiers_redacted\": true,\n",
                "  \"selection\": \"{}\",\n",
                "  \"entries\": [\n",
                "{}\n",
                "  ],\n",
                "  \"evidence\": \"local history only; native viability, Win32 architecture and current entitlement require their explicit gates\"\n",
                "}}"
            ),
            inventory_selection_label(candidates_only, verified_win32_only, two_d_only),
            records
        );
        return Ok(());
    }
    println!("Steam local inventory history (account identifiers redacted)");
    println!("Steam: {}", inventory.steam_root.display());
    for entry in inventory.entries.iter().take(limit) {
        let base = format!(
            "AppID {}\tplaytime {}\tlast-played {}\trecords {}\ttickets {}",
            entry.app_id,
            entry.max_playtime_minutes.unwrap_or(0),
            entry.latest_last_played_unix.unwrap_or(0),
            entry.account_records,
            entry.cached_app_tickets
        );
        if with_appinfo {
            match steam_appinfo::inspect_steam_root(entry.app_id, &steam_root) {
                Ok(metadata) => println!(
                    "{}\tname {}\tcached-launch-platforms {}\ttarget-gate {}\treview {}",
                    base,
                    single_line(metadata.name.as_deref().unwrap_or("unknown")),
                    cached_app_platforms(&metadata),
                    inventory_target_gate(
                        &metadata,
                        installed_evidence.get(&entry.app_id),
                        eligibility_reviews.get(&entry.app_id),
                    ),
                    eligibility_reviews
                        .get(&entry.app_id)
                        .map(|review| review.sha256.as_str())
                        .unwrap_or("none")
                ),
                Err(_) => println!(
                    "{base}\tname unknown\tcached-launch-platforms unknown\ttarget-gate metadata-incomplete"
                ),
            }
        } else {
            println!("{base}");
        }
    }
    println!(
        "Evidence: local history only; native viability and entitlement remain separate gates"
    );
    Ok(())
}

fn steam_target_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam target-status APPID --native-steam-root PATH --windows-steam-root PATH [--eligibility-review PATH] [--json]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut native_steam_root = None;
    let mut windows_steam_root = None;
    let mut eligibility_review_path = None;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--native-steam-root" => native_steam_root = Some(PathBuf::from(value)),
            "--windows-steam-root" => windows_steam_root = Some(PathBuf::from(value)),
            "--eligibility-review" => eligibility_review_path = Some(PathBuf::from(value)),
            other => return Err(format!("unknown steam target-status option {other:?}")),
        }
        index += 1;
    }
    let native_steam_root =
        native_steam_root.ok_or_else(|| "--native-steam-root PATH is required".to_owned())?;
    let windows_steam_root =
        windows_steam_root.ok_or_else(|| "--windows-steam-root PATH is required".to_owned())?;
    let observed_native_gate = target_gate_for_root(app_id, &native_steam_root)?;
    let windows_gate = target_gate_for_root(app_id, &windows_steam_root)?;
    let eligibility_review = eligibility_review_path
        .as_deref()
        .map(|path| load_target_eligibility_review(path, app_id))
        .transpose()?;
    let native_gate = eligibility_review
        .as_ref()
        .map(|review| reviewed_native_gate(&observed_native_gate, review.native_status.as_str()))
        .unwrap_or_else(|| observed_native_gate.clone());
    let outcome = combined_target_outcome(&native_gate, &windows_gate);
    if json {
        let review_sha256 = eligibility_review
            .as_ref()
            .map(|review| format!("\"{}\"", review.sha256))
            .unwrap_or_else(|| "null".to_owned());
        let review_source = eligibility_review
            .as_ref()
            .map(|review| format!("\"{}\"", json_escape(&review.source_url)))
            .unwrap_or_else(|| "null".to_owned());
        println!(
            concat!(
                "{{\n",
                "  \"schema_version\": 3,\n",
                "  \"appid\": {},\n",
                "  \"observed_native_target_gate\": \"{}\",\n",
                "  \"native_target_gate\": \"{}\",\n",
                "  \"windows_target_gate\": \"{}\",\n",
                "  \"eligibility_review_sha256\": {},\n",
                "  \"eligibility_review_source\": {},\n",
                "  \"decision\": \"{}\",\n",
                "  \"stage\": \"{}\",\n",
                "  \"eligible\": {},\n",
                "  \"reason\": \"{}\",\n",
                "  \"next_action\": \"{}\",\n",
                "  \"evidence_boundary\": \"target eligibility is not gameplay compatibility; native viability and the path-bound Windows entrypoint are independent gates\"\n",
                "}}"
            ),
            app_id,
            observed_native_gate,
            native_gate,
            windows_gate,
            review_sha256,
            review_source,
            outcome.decision,
            outcome.stage,
            outcome.stage == "eligible",
            outcome.reason,
            outcome.next_action,
        );
    } else {
        println!("Macroton target status for AppID {app_id}");
        println!("Observed native gate: {observed_native_gate}");
        println!("Native gate:  {native_gate}");
        println!("Windows gate: {windows_gate}");
        if let Some(review) = eligibility_review {
            println!("Review:       {} ({})", review.native_status, review.sha256);
            println!("Source:       {}", review.source_url);
        }
        println!("Decision:     {}", outcome.decision);
        println!("Stage:        {}", outcome.stage);
        println!("Reason:       {}", outcome.reason);
        println!("Next action:  {}", outcome.next_action);
        println!("Boundary:     Windows PE32 proof never overrides a working native Mac build");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TargetEligibilityReview {
    native_status: String,
    source_url: String,
    sha256: String,
    scope: Option<String>,
}

fn review_scope_allows_2d(review: Option<&TargetEligibilityReview>) -> bool {
    review.and_then(|item| item.scope.as_deref()) == Some("2d")
}

fn load_target_eligibility_review(
    path: &Path,
    expected_app_id: u32,
) -> Result<TargetEligibilityReview, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect eligibility review {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() {
        return Err("eligibility review must be a regular, non-symlink file".to_owned());
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read eligibility review {}: {error}", path.display()))?;
    let mut fields = BTreeMap::new();
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| "eligibility review line must be key=value".to_owned())?;
        let key = key.trim();
        let value = value.trim();
        if !matches!(
            key,
            "schema"
                | "appid"
                | "native_status"
                | "source_url"
                | "reviewed_date"
                | "evidence"
                | "scope"
        ) {
            return Err(format!("unknown eligibility review field {key:?}"));
        }
        if value.is_empty() || fields.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(format!(
                "empty or duplicate eligibility review field {key:?}"
            ));
        }
    }
    let field = |name: &str| {
        fields
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("missing eligibility review field {name:?}"))
    };
    if field("schema")? != "1" {
        return Err("eligibility review schema must be 1".to_owned());
    }
    let app_id: u32 = field("appid")?
        .parse()
        .map_err(|_| "eligibility review appid is invalid".to_owned())?;
    if app_id != expected_app_id {
        return Err(format!(
            "eligibility review AppID {app_id} does not match requested AppID {expected_app_id}"
        ));
    }
    let native_status = field("native_status")?;
    if !matches!(
        native_status,
        "working" | "legacy-unusable" | "windows-only"
    ) {
        return Err(format!(
            "unsupported eligibility native_status {native_status:?}"
        ));
    }
    let source_url = field("source_url")?;
    let expected_prefix = format!("https://store.steampowered.com/app/{app_id}/");
    if !source_url.starts_with(&expected_prefix)
        || source_url.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(
            "eligibility source must be the matching official Steam store AppID URL".to_owned(),
        );
    }
    let reviewed_date = field("reviewed_date")?;
    if reviewed_date.len() != 10
        || reviewed_date.as_bytes()[4] != b'-'
        || reviewed_date.as_bytes()[7] != b'-'
        || !reviewed_date
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return Err("eligibility reviewed_date must be YYYY-MM-DD".to_owned());
    }
    validate_failure_code(field("evidence")?)?;
    let scope = fields.get("scope").cloned();
    if let Some(scope) = scope.as_deref()
        && !matches!(scope, "2d" | "3d" | "unknown")
    {
        return Err(format!("unsupported eligibility scope {scope:?}"));
    }
    let sha256 = sha256::to_hex(
        &sha256::digest_file(path)
            .map_err(|error| format!("cannot hash eligibility review: {error}"))?,
    );
    Ok(TargetEligibilityReview {
        native_status: native_status.to_owned(),
        source_url: source_url.to_owned(),
        sha256,
        scope,
    })
}

fn load_inventory_eligibility_reviews(
    directory: &Path,
    app_ids: impl Iterator<Item = u32>,
) -> Result<BTreeMap<u32, TargetEligibilityReview>, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        format!(
            "cannot inspect eligibility review directory {}: {error}",
            directory.display()
        )
    })?;
    if !metadata.file_type().is_dir() {
        return Err("eligibility review directory must be a real directory".to_owned());
    }
    let mut reviews = BTreeMap::new();
    for app_id in app_ids {
        let path = directory.join(format!("{app_id}.conf"));
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                reviews.insert(app_id, load_target_eligibility_review(&path, app_id)?);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect eligibility review {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(reviews)
}

fn inventory_target_gate(
    metadata: &steam_appinfo::AppMetadata,
    installed: Option<&InstalledTargetEvidence>,
    review: Option<&TargetEligibilityReview>,
) -> String {
    let observed = target_eligibility(metadata, installed);
    let Some(review) = review else {
        return observed.to_owned();
    };
    if observed == "exclude-working-native-mac" || review.native_status == "working" {
        return "exclude-working-native-mac".to_owned();
    }
    if is_verified_win32_gate(observed) {
        return observed.to_owned();
    }
    reviewed_native_gate(observed, &review.native_status)
}

fn reviewed_native_gate(observed_gate: &str, native_status: &str) -> String {
    if observed_gate == "exclude-working-native-mac" || native_status == "working" {
        return "exclude-working-native-mac".to_owned();
    }
    match native_status {
        "legacy-unusable" => "candidate-legacy-mac-fallback".to_owned(),
        "windows-only" => "candidate-windows-only".to_owned(),
        _ => observed_gate.to_owned(),
    }
}

fn target_gate_for_root(app_id: u32, steam_root: &Path) -> Result<String, String> {
    let metadata =
        steam_appinfo::inspect_steam_root(app_id, steam_root).map_err(|error| error.to_string())?;
    let installed = steam::scan(steam_root)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|game| game.app_id == app_id)
        .map(|game| InstalledTargetEvidence {
            classification: game.classification,
            pe_binaries: game
                .binaries
                .into_iter()
                .filter_map(|binary| Some((binary.relative_path, binary.pe_architecture?)))
                .collect(),
        });
    Ok(target_eligibility(&metadata, installed.as_ref()).to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CombinedTargetOutcome {
    decision: &'static str,
    stage: &'static str,
    reason: &'static str,
    next_action: &'static str,
}

fn combined_target_outcome(native_gate: &str, windows_gate: &str) -> CombinedTargetOutcome {
    if native_gate == "exclude-working-native-mac" {
        return CombinedTargetOutcome {
            decision: "exclude-working-native-mac",
            stage: "excluded",
            reason: "an official Mac build is already usable on this host",
            next_action: "use-official-native-build",
        };
    }
    if native_gate == "exclude-no-windows-launch" {
        return CombinedTargetOutcome {
            decision: "exclude-no-windows-build",
            stage: "excluded",
            reason: "Steam metadata exposes no Windows launch path for this title",
            next_action: "outside-scope-no-windows-build",
        };
    }
    if windows_gate == "exclude-windows-64-installed" {
        return match native_gate {
            "candidate-legacy-mac-fallback" | "candidate-windows-only" => CombinedTargetOutcome {
                decision: "experimental-verified-win64-2d",
                stage: "experimental",
                reason: "the installed Windows entrypoint is PE32+ x86-64; it may be tested in the separate Win64 2D lane but cannot count as Win32 coverage",
                next_action: "begin-win64-experimental-validation",
            },
            _ => CombinedTargetOutcome {
                decision: "review-native-mac-viability",
                stage: "review",
                reason: "the Windows entrypoint is PE32+ x86-64, but current native Mac viability is unresolved",
                next_action: "verify-official-mac-build-on-target-host",
            },
        };
    }
    let win32_verified = matches!(
        windows_gate,
        "candidate-verified-win32-installed" | "candidate-verified-win32-fallback"
    );
    if !win32_verified {
        let reason = if windows_gate == "exclude-windows-64-only" {
            "Steam metadata declares a 64-bit Windows launch, but no installed path-bound game entrypoint has been verified"
        } else {
            "no exact path-bound PE32/i386 Windows game entrypoint has been verified"
        };
        return CombinedTargetOutcome {
            decision: "blocked-no-path-bound-win32-entrypoint",
            stage: "blocked",
            reason,
            next_action: "verify-authenticated-windows-pe32-entrypoint",
        };
    }
    match native_gate {
        "candidate-legacy-mac-fallback" | "candidate-verified-win32-fallback" => {
            CombinedTargetOutcome {
                decision: "eligible-verified-win32-legacy-mac",
                stage: "eligible",
                reason: "the official Mac payload is legacy-only and an exact PE32 Windows entrypoint is installed",
                next_action: "begin-runtime-validation",
            }
        }
        "candidate-windows-only" | "candidate-verified-win32-installed" => CombinedTargetOutcome {
            decision: "eligible-verified-win32-windows-only",
            stage: "eligible",
            reason: "the title is Windows-only and an exact PE32 Windows entrypoint is installed",
            next_action: "begin-runtime-validation",
        },
        _ => CombinedTargetOutcome {
            decision: "review-native-mac-viability",
            stage: "review",
            reason: "the Windows PE32 entrypoint is verified but current native Mac viability is unresolved",
            next_action: "verify-official-mac-build-on-target-host",
        },
    }
}

fn inventory_entry_json(
    entry: &steam_library_status::LibraryStatus,
    name: Option<&str>,
    cached_launch_platforms: Option<&str>,
    target_gate: Option<&str>,
    review: Option<&TargetEligibilityReview>,
) -> String {
    format!(
        concat!(
            "    {{\n",
            "      \"appid\": {},\n",
            "      \"max_playtime_minutes\": {},\n",
            "      \"latest_last_played_unix\": {},\n",
            "      \"account_records\": {},\n",
            "      \"cached_app_tickets\": {},\n",
            "      \"name\": {},\n",
            "      \"cached_launch_platforms\": {},\n",
            "      \"target_gate\": {},\n",
            "      \"eligibility_review_sha256\": {},\n",
            "      \"eligibility_review_source\": {},\n",
            "      \"scope\": {}\n",
            "    }}"
        ),
        entry.app_id,
        json_optional_u64(entry.max_playtime_minutes),
        json_optional_u64(entry.latest_last_played_unix),
        entry.account_records,
        entry.cached_app_tickets,
        json_optional_string(name),
        json_optional_string(cached_launch_platforms),
        json_optional_string(target_gate),
        json_optional_string(review.map(|review| review.sha256.as_str())),
        json_optional_string(review.map(|review| review.source_url.as_str())),
        json_optional_string(review.and_then(|review| review.scope.as_deref())),
    )
}

fn is_candidate_gate(gate: &str) -> bool {
    gate.starts_with("candidate-")
}

fn is_verified_win32_gate(gate: &str) -> bool {
    matches!(
        gate,
        "candidate-verified-win32-installed" | "candidate-verified-win32-fallback"
    )
}

fn strict_verified_inventory_candidate(
    metadata: &steam_appinfo::AppMetadata,
    gate: &str,
    review: Option<&TargetEligibilityReview>,
) -> bool {
    if !is_verified_win32_gate(gate)
        || review.is_some_and(|review| review.native_status == "working")
    {
        return false;
    }
    if metadata_has_platform(metadata, "macos") {
        return review.is_some_and(|review| {
            matches!(
                review.native_status.as_str(),
                "legacy-unusable" | "windows-only"
            )
        });
    }
    true
}

fn metadata_has_platform(metadata: &steam_appinfo::AppMetadata, wanted: &str) -> bool {
    metadata.launches.iter().any(|launch| {
        launch
            .os_list
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .any(|platform| platform == wanted)
    }) || metadata
        .app_os_list
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .any(|platform| platform == wanted)
}

#[derive(Clone)]
struct InstalledTargetEvidence {
    classification: steam::Classification,
    pe_binaries: Vec<(PathBuf, PeArchitecture)>,
}

fn target_eligibility(
    metadata: &steam_appinfo::AppMetadata,
    installed: Option<&InstalledTargetEvidence>,
) -> &'static str {
    let launches = &metadata.launches;
    let windows = metadata_has_platform(metadata, "windows");
    let macos = metadata_has_platform(metadata, "macos");
    if installed.is_some_and(|evidence| evidence.classification == steam::Classification::NativeMac)
    {
        return "exclude-working-native-mac";
    }
    if let Some(evidence) = installed
        && evidence.classification == steam::Classification::WindowsPayload
    {
        let entrypoints = metadata_entrypoints(metadata);
        let mut has_i386_entrypoint = false;
        let mut has_x86_64_entrypoint = false;
        for (path, architecture) in &evidence.pe_binaries {
            if !entrypoints.contains(&normalized_windows_path(&path.to_string_lossy())) {
                continue;
            }
            has_i386_entrypoint |= *architecture == PeArchitecture::I386;
            has_x86_64_entrypoint |= *architecture == PeArchitecture::X86_64;
        }
        if has_i386_entrypoint {
            return "candidate-verified-win32-installed";
        }
        if has_x86_64_entrypoint {
            return "exclude-windows-64-installed";
        }
        return "candidate-windows-architecture-unverified";
    }
    let mut windows_launch_count = 0_usize;
    let mut every_windows_launch_is_explicit_64 = true;
    for launch in launches.iter().filter(|launch| {
        let launch_platforms = launch
            .os_list
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .collect::<Vec<_>>();
        launch_platforms.contains(&"windows") || (launch.os_list.is_none() && windows && !macos)
    }) {
        windows_launch_count += 1;
        if launch.os_arch.as_deref().map(str::trim) != Some("64") {
            every_windows_launch_is_explicit_64 = false;
        }
    }
    if windows_launch_count > 0 && every_windows_launch_is_explicit_64 {
        return "exclude-windows-64-only";
    }
    match installed.map(|evidence| evidence.classification) {
        Some(steam::Classification::LegacyMac32)
            if windows
                && installed.is_some_and(|evidence| {
                    let entrypoints = metadata_entrypoints(metadata);
                    evidence.pe_binaries.iter().any(|(path, arch)| {
                        *arch == PeArchitecture::I386
                            && entrypoints
                                .contains(&normalized_windows_path(&path.to_string_lossy()))
                    })
                }) =>
        {
            "candidate-verified-win32-fallback"
        }
        Some(steam::Classification::LegacyMac32) if windows => "candidate-legacy-mac-fallback",
        _ if windows && !macos => "candidate-windows-only",
        _ if windows && macos => "review-native-mac-viability",
        _ if !windows && macos => "exclude-no-windows-launch",
        _ => "metadata-incomplete",
    }
}

fn metadata_entrypoints(metadata: &steam_appinfo::AppMetadata) -> BTreeSet<String> {
    metadata
        .launches
        .iter()
        .map(|launch| normalized_windows_path(&launch.executable))
        .chain(
            metadata
                .executable_references
                .iter()
                .filter(|reference| reference.metadata_path.starts_with("config/checkguid"))
                .map(|reference| normalized_windows_path(&reference.executable)),
        )
        .collect()
}

fn normalized_windows_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase()
}

fn cached_app_platforms(metadata: &steam_appinfo::AppMetadata) -> String {
    let mut platforms: BTreeSet<&str> = metadata
        .launches
        .iter()
        .flat_map(|launch| launch.os_list.as_deref().unwrap_or("").split(','))
        .map(str::trim)
        .filter(|platform| !platform.is_empty())
        .collect();
    platforms.extend(
        metadata
            .app_os_list
            .as_deref()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|platform| !platform.is_empty()),
    );
    if platforms.is_empty() {
        "unknown".to_owned()
    } else {
        platforms.into_iter().collect::<Vec<_>>().join(",")
    }
}

fn single_line(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if matches!(character, '\n' | '\r' | '\t') {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn steam_library_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam library-status APPID [--data-root PATH | --steam-root PATH]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut steam_root = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a path"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--steam-root" => steam_root = Some(PathBuf::from(value)),
            other => return Err(format!("unknown steam library-status option {other:?}")),
        }
        index += 1;
    }
    let status = if let Some(steam_root) = steam_root {
        steam_library_status::inspect_steam_root(app_id, &steam_root)
    } else {
        steam_library_status::inspect(app_id, &data_root)
    }
    .map_err(|error| error.to_string())?;
    println!("Steam local library status for AppID {}", status.app_id);
    println!("Records:   {} account record(s)", status.account_records);
    println!(
        "Playtime:  {} minute(s) maximum observed",
        status
            .max_playtime_minutes
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
    );
    println!(
        "Last play: {} (Unix timestamp)",
        status
            .latest_last_played_unix
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
    );
    println!(
        "Tickets:   {} cached ticket record(s)",
        status.cached_app_tickets
    );
    println!("Steam:     {}", status.steam_root.display());
    println!("Evidence:  local history only; current entitlement is not inferred");
    Ok(())
}

fn steam_appinfo_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam appinfo APPID [--profile PATH] [--data-root PATH | --steam-root PATH]".to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut steam_root = None;
    let mut profile_path = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a path"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--steam-root" => steam_root = Some(PathBuf::from(value)),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            other => return Err(format!("unknown steam appinfo option {other:?}")),
        }
        index += 1;
    }
    let metadata = if let Some(steam_root) = steam_root {
        steam_appinfo::inspect_steam_root(app_id, &steam_root)
    } else {
        steam_appinfo::inspect(app_id, &data_root)
    }
    .map_err(|error| error.to_string())?;
    println!("Steam cached app metadata for AppID {}", metadata.app_id);
    println!(
        "Name:      {}",
        metadata.name.as_deref().unwrap_or("unknown")
    );
    println!(
        "App OS:    {}",
        metadata.app_os_list.as_deref().unwrap_or("unspecified")
    );
    println!(
        "Install:   {}",
        metadata.install_dir.as_deref().unwrap_or("unknown")
    );
    println!(
        "BuildID:   {}",
        metadata.public_build_id.as_deref().unwrap_or("unknown")
    );
    for launch in &metadata.launches {
        println!(
            "Launch:    {} (os {})",
            launch.executable,
            launch.os_list.as_deref().unwrap_or("unspecified")
        );
        if let Some(working_dir) = &launch.working_dir {
            println!("Workdir:   {working_dir}");
        }
    }
    for reference in &metadata.executable_references {
        if !metadata
            .launches
            .iter()
            .any(|launch| launch.executable == reference.executable)
        {
            println!(
                "Exe ref:   {} ({})",
                reference.executable, reference.metadata_path
            );
        }
    }
    for depot in &metadata.windows_public_depots {
        println!(
            "Depot:     {} manifest {} size {} language {}{}",
            depot.depot_id,
            depot.manifest_id,
            depot
                .size
                .map(|size| size.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            depot.language.as_deref().unwrap_or("neutral"),
            depot
                .dlc_app_id
                .map(|app_id| format!(" dlc {app_id}"))
                .unwrap_or_default()
        );
    }
    for rule in &metadata.cloud_save_rules {
        println!(
            "Cloud rule: {} root {} path {} pattern {} recursive {} platforms {}",
            rule.rule_id,
            rule.root.as_deref().unwrap_or("unknown"),
            rule.path.as_deref().unwrap_or("."),
            rule.pattern.as_deref().unwrap_or("*"),
            rule.recursive
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            rule.platforms.as_deref().unwrap_or("unspecified")
        );
    }
    for rule in &metadata.cloud_root_overrides {
        println!(
            "Cloud root: {} root {} os {} path {}",
            rule.rule_id,
            rule.root.as_deref().unwrap_or("unknown"),
            rule.os.as_deref().unwrap_or("unspecified"),
            rule.path.as_deref().unwrap_or(".")
        );
    }
    if let Some(profile_path) = profile_path {
        let profile =
            CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
        let mismatches = steam_appinfo::profile_mismatches(&metadata, &profile);
        if !mismatches.is_empty() {
            return Err(format!(
                "cached app metadata does not match profile:\n- {}",
                mismatches.join("\n- ")
            ));
        }
        println!(
            "Profile:   cached metadata match ({})",
            profile_path.display()
        );
    }
    println!("Cache:     {}", metadata.cache_path.display());
    println!("Evidence:  metadata only; not entitlement or installed-payload proof");
    Ok(())
}

fn steam_connection_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam connection-status APPID [--data-root PATH | --steam-root PATH] [--json]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut steam_root = None;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a path"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--steam-root" => steam_root = Some(PathBuf::from(value)),
            other => return Err(format!("unknown steam connection-status option {other:?}")),
        }
        index += 1;
    }
    let status = if let Some(steam_root) = steam_root {
        steam_connection_status::inspect_steam_root(app_id, &steam_root)
    } else {
        steam_connection_status::inspect(app_id, &data_root)
    }
    .map_err(|error| error.to_string())?;
    if json {
        println!(
            "{{\"schema\":1,\"appid\":{},\"state\":\"{}\",\"synthetic_fake_ip\":{},\"authenticated_context\":{},\"ui_login_observed\":{},\"auth_poll_transport_error\":{}}}",
            status.app_id,
            status.state.as_str(),
            status
                .synthetic_fake_ip
                .map_or_else(|| "null".to_owned(), |value| value.to_string()),
            status.authenticated_context,
            status.ui_login_observed,
            status.auth_poll_transport_error
        );
        return Ok(());
    }
    println!("Steam transport status for AppID {}", status.app_id);
    println!("State:     {}", status.state.as_str());
    println!("Steam:     {}", status.steam_root.display());
    println!(
        "Route:     {}",
        match status.synthetic_fake_ip {
            Some(true) => "benchmark-range-fake-ip",
            Some(false) => "public-or-unclassified",
            None => "no-completed-endpoint",
        }
    );
    if let Some(timestamp) = &status.latest_timestamp {
        println!("Observed:  {timestamp}");
    }
    println!("Auth:      not inferred from transport state");
    println!(
        "UI login:  {}",
        if status.ui_login_observed {
            "post-login state observed in Steam UI log"
        } else {
            "not observed"
        }
    );
    println!(
        "Auth poll: {}",
        if status.auth_poll_transport_error {
            "transport errors observed while polling login session"
        } else {
            "no transport polling error observed"
        }
    );
    println!(
        "Context:   {}",
        if status.authenticated_context {
            "authenticated-account-marker-observed; account-id-redacted"
        } else {
            "no-authenticated-account-marker"
        }
    );
    println!("Log:       {}", status.log_path.display());
    Ok(())
}

fn steam_prefix_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam prefix-status APPID [--data-root PATH] [--json]".to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a path"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            other => return Err(format!("unknown steam prefix-status option {other:?}")),
        }
        index += 1;
    }
    let status =
        steam_prefix_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    if json {
        println!("{{");
        println!("  \"schema_version\": 1,");
        println!("  \"appid\": {},", status.app_id);
        println!("  \"readiness\": {:?},", status.readiness.as_str());
        println!("  \"client_installed\": {},", status.client_installed);
        println!(
            "  \"bootstrap_marker_present\": {},",
            status.bootstrap_marker_present
        );
        println!("  \"bootstrap_valid\": {},", status.bootstrap_recorded);
        println!(
            "  \"runtime_identity_recorded\": {},",
            status.runtime_identity_recorded
        );
        println!(
            "  \"stored_login_records\": {},",
            status.stored_login_records
        );
        println!(
            "  \"userdata_directories\": {},",
            status.userdata_directories
        );
        println!("  \"live_auth_verified\": false");
        println!("}}");
        return Ok(());
    }
    println!("Windows Steam prefix status for AppID {}", status.app_id);
    println!("State:     {}", status.readiness.as_str());
    println!("Prefix:    {}", status.prefix.display());
    println!(
        "Client:    {}",
        if status.client_installed {
            "installed"
        } else {
            "missing"
        }
    );
    println!(
        "Bootstrap: {}",
        if status.bootstrap_recorded {
            "valid"
        } else if status.bootstrap_marker_present {
            "invalid"
        } else {
            "missing"
        }
    );
    println!("Evidence:  {}", status.bootstrap_detail);
    println!(
        "Runtime:   {}",
        if status.runtime_identity_recorded {
            "recorded"
        } else {
            "missing"
        }
    );
    println!(
        "Logins:    {} stored record(s)",
        status.stored_login_records
    );
    println!(
        "Userdata:  {} account directories",
        status.userdata_directories
    );
    println!("Live auth: not verified by this read-only command");
    Ok(())
}

fn steam_launch_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam launch-status APPID [--data-root PATH | --steam-root PATH] [--json]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut steam_root = None;
    let mut json = false;
    let mut uses_managed_data_root = true;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--steam-root" => {
                steam_root = Some(PathBuf::from(value));
                uses_managed_data_root = false;
            }
            other => return Err(format!("unknown steam launch-status option {other:?}")),
        }
        index += 1;
    }
    let status = if let Some(steam_root) = steam_root {
        launch_status::inspect_steam_root(app_id, &steam_root)
    } else if let Some((owner, _)) = read_shared_client_marker(app_id, &data_root)? {
        launch_status::inspect_in_client(app_id, owner, &data_root)
    } else {
        launch_status::inspect(app_id, &data_root)
    }
    .map_err(|error| error.to_string())?;
    if json {
        print!("{}", launch_status_json(&status));
        return Ok(());
    }
    println!("Steam launch status for AppID {}", status.app_id);
    println!("State:    {}", status.state.as_str());
    println!("Source:   Steam console-log lifecycle (not a host liveness probe)");
    if status.state == launch_status::LaunchState::RunningObserved
        && uses_managed_data_root
        && status.process_id.is_none()
    {
        println!(
            "Notice:   historical running-observed event has no live game PID; treat it as stale and retry"
        );
    }
    if status.state == launch_status::LaunchState::WaitingCloud {
        if let Ok(cloud) = cloud_status::inspect_steam_root(app_id, &status.steam_root)
            && !cloud.pending_remote_operations.is_empty()
        {
            println!(
                "Pending:  {} client-side Cloud launch operation(s)",
                cloud.pending_remote_operations.len()
            );
            for operation in cloud.pending_remote_operations {
                println!("          {operation}");
            }
        }
        if uses_managed_data_root {
            let steam_live = stop::inspect_resources(app_id, &data_root)
                .map(|resources| {
                    resources
                        .processes
                        .iter()
                        .any(is_primary_steam_client_process)
                })
                .unwrap_or(false);
            if steam_live {
                println!(
                    "Action:   resolve the Steam Cloud pending session before retrying the game"
                );
            } else {
                println!(
                    "Notice:   this is a stale Cloud event; no Steam client is currently live"
                );
            }
        } else {
            println!(
                "Notice:   external Steam root supplied; host process liveness was not checked"
            );
        }
    }
    if status.state == launch_status::LaunchState::WaitingUser {
        if uses_managed_data_root {
            let steam_live = stop::inspect_resources(app_id, &data_root)
                .map(|resources| {
                    resources
                        .processes
                        .iter()
                        .any(is_primary_steam_client_process)
                })
                .unwrap_or(false);
            if steam_live {
                println!(
                    "Action:   open the visible Windows Steam UI for this prefix and complete the pending confirmation/interstitial"
                );
            } else {
                println!(
                    "Notice:   Steam is no longer live; this user-response event is stale and the prefix can be retried"
                );
            }
        } else {
            println!(
                "Notice:   external Steam root supplied; open its visible Windows Steam UI to complete the pending user response"
            );
        }
    }
    println!("Steam:    {}", status.steam_root.display());
    if let Some(process_id) = status.process_id {
        println!("Process:  {process_id}");
    } else {
        println!("Process:  none observed");
    }
    if let Some(exit_code) = status.exit_code {
        println!("Exit code: {exit_code}");
    }
    if let Some(event) = &status.latest_event {
        println!("Latest:   {event}");
    } else {
        println!("Latest:   no AppID-specific launch event found");
    }
    println!("Log:      {}", status.log_path.display());
    Ok(())
}

fn launch_status_json(status: &launch_status::LaunchStatus) -> String {
    let process_id = status
        .process_id
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_owned());
    let exit_code = status
        .exit_code
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_owned());
    let steam_process_id = status
        .steam_process_id
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_owned());
    format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 2,\n",
            "  \"appid\": {},\n",
            "  \"state\": \"{}\",\n",
            "  \"process_id\": {},\n",
            "  \"steam_process_id\": {},\n",
            "  \"game_process_live\": {},\n",
            "  \"exit_code\": {},\n",
            "  \"event_present\": {},\n",
            "  \"source\": \"steam-console-log-lifecycle\"\n",
            "}}\n"
        ),
        status.app_id,
        status.state.as_str(),
        process_id,
        steam_process_id,
        status.process_id.is_some(),
        exit_code,
        status.latest_event.is_some()
    )
}

fn steam_cloud_status_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton steam cloud-status APPID [--profile PATH] [--data-root PATH | --steam-root PATH] [--json]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut data_root = default_data_root()?;
    let mut steam_root = None;
    let mut profile_path = None;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--steam-root" => steam_root = Some(PathBuf::from(value)),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            other => return Err(format!("unknown steam cloud-status option {other:?}")),
        }
        index += 1;
    }
    let mut status = if let Some(steam_root) = steam_root {
        cloud_status::inspect_steam_root(app_id, &steam_root)
    } else if let Some((owner, _)) = read_shared_client_marker(app_id, &data_root)? {
        cloud_status::inspect_in_client(app_id, owner, &data_root)
    } else {
        cloud_status::inspect(app_id, &data_root)
    }
    .map_err(|error| error.to_string())?;
    if let Some(profile_path) = profile_path {
        let profile =
            CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
        if profile.app_id != app_id {
            return Err(format!(
                "profile AppID {} does not match requested AppID {app_id}",
                profile.app_id
            ));
        }
        if let Some((_, fingerprint)) = read_shared_client_marker(app_id, &data_root)?
            && fingerprint != profile.fingerprint()
        {
            return Err("Cloud comparison profile differs from the active shared-client launch".to_owned());
        }
        cloud_status::compare_profile_files(&mut status, &profile)
            .map_err(|error| error.to_string())?;
    }
    if json {
        print!("{}", cloud_status_json(&status));
        return Ok(());
    }
    println!("Steam Cloud status for AppID {}", status.app_id);
    println!("Cache:    {}", status.state.as_str());
    println!("Remote:   {} (local cache does not prove latest data on other devices)", status.remote_freshness.as_str());
    println!("Prefix:   {}", status.prefix.display());
    if status.accounts.is_empty() {
        println!("Cache:    no per-account remotecache.vdf found");
    } else {
        for account in &status.accounts {
            println!(
                "Account:  {} ({} tracked, {} synced, {} pending, {} other-platform)",
                account.account_id,
                account.tracked_files,
                account.synced_files,
                account.pending_files,
                account.platform_skipped_files
            );
            println!("Cache:    {}", account.path.display());
            for file in &account.files {
                println!(
                    "File:     {} (size {}, SHA-1 {}, syncstate {}{})",
                    file.relative_path,
                    file.size
                        .map(|size| size.to_string())
                        .unwrap_or_else(|| "unknown".to_owned()),
                    file.sha1.as_deref().unwrap_or("unknown"),
                    file.sync_state
                        .map(|state| state.to_string())
                        .unwrap_or_else(|| "unknown".to_owned()),
                    if file.platform_skipped {
                        ", other-platform"
                    } else {
                        ""
                    }
                );
            }
        }
    }
    if let Some(event) = &status.latest_event {
        println!("Latest:   {event}");
    } else {
        println!("Latest:   no AppID-specific sync event found");
    }
    if status.pending_remote_operations.is_empty() {
        println!("Pending:  no client-side pending remote launch operations observed");
    } else {
        println!(
            "Pending:  {} client-side remote launch operation(s); cache sync alone is not launch-ready",
            status.pending_remote_operations.len()
        );
        for operation in &status.pending_remote_operations {
            println!("          {operation}");
        }
    }
    for comparison in &status.comparisons {
        println!(
            "Compare:  {} [{}] (cache {}, local {}, remote copy {}, remote content {}, encoding {})",
            comparison.remote_path,
            comparison.state.as_str(),
            comparison
                .cached_size
                .map(|size| size.to_string())
                .unwrap_or_else(|| "missing".to_owned()),
            comparison
                .local_size
                .map(|size| size.to_string())
                .unwrap_or_else(|| "missing".to_owned()),
            comparison
                .remote_copy_size
                .map(|size| size.to_string())
                .unwrap_or_else(|| "missing".to_owned()),
            comparison
                .remote_content_size
                .map(|size| size.to_string())
                .unwrap_or_else(|| "missing".to_owned()),
            comparison
                .remote_copy_encoding
                .map(|encoding| encoding.as_str())
                .unwrap_or("missing"),
        );
        println!("Local:    {}", comparison.local_path.display());
        println!("Remote:   {}", comparison.remote_copy_path.display());
        println!(
            "Hashes:   cache {}, local {}, remote copy {}, remote content {}",
            comparison.cached_sha1.as_deref().unwrap_or("missing"),
            comparison.local_sha1.as_deref().unwrap_or("missing"),
            comparison.remote_copy_sha1.as_deref().unwrap_or("missing"),
            comparison
                .remote_content_sha1
                .as_deref()
                .unwrap_or("missing")
        );
    }
    println!("Log:      {}", status.log_path.display());
    Ok(())
}

fn cloud_status_json(status: &cloud_status::CloudStatus) -> String {
    let pending = status
        .pending_remote_operations
        .iter()
        .map(|operation| format!("\"{}\"", json_escape(operation)))
        .collect::<Vec<_>>()
        .join(", ");
    let comparisons = status
        .comparisons
        .iter()
        .map(|comparison| {
            format!(
                "{{\"remote_path\":\"{}\",\"state\":\"{}\"}}",
                json_escape(&comparison.remote_path),
                comparison.state.as_str()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{{\"schema_version\":2,\"appid\":{},\"state\":\"{}\",\"remote_freshness\":\"{}\",\"latest_event\":{},\"launch_ready\":{},\"pending_remote_operations\":[{}],\"comparisons\":[{}]}}\n",
        status.app_id,
        status.state.as_str(),
        status.remote_freshness.as_str(),
        json_optional_string(status.latest_event.as_deref()),
        cloud_launch_ready(status.state, status.pending_remote_operations.len(), status.remote_freshness),
        pending,
        comparisons
    )
}

fn steam_bootstrap_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| "usage: macroton steam bootstrap APPID --profile PATH --installer PATH --installer-sha256 SHA256 [--runtime-root PATH] [--data-root PATH]".to_owned())?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut runtime_root = default_runtime_root()?;
    let mut data_root = default_data_root()?;
    let mut profile_path = None;
    let mut installer = None;
    let mut installer_sha256 = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--installer" => installer = Some(PathBuf::from(value)),
            "--installer-sha256" => installer_sha256 = Some(value.clone()),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--data-root" => data_root = PathBuf::from(value),
            other => return Err(format!("unknown steam bootstrap option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let installer = installer.ok_or_else(|| "--installer PATH is required".to_owned())?;
    let installer_sha256 =
        installer_sha256.ok_or_else(|| "--installer-sha256 SHA256 is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    let outcome = steam_bootstrap::bootstrap(&steam_bootstrap::Options {
        app_id,
        profile: &profile,
        installer: &installer,
        installer_sha256: &installer_sha256,
        runtime_root: &runtime_root,
        data_root: &data_root,
    })
    .map_err(|error| error.to_string())?;
    let verb = match outcome.kind {
        steam_bootstrap::OutcomeKind::Installed => "installed",
        steam_bootstrap::OutcomeKind::AlreadyInstalled => "already installed",
        steam_bootstrap::OutcomeKind::RepairedProvenance => "provenance repaired",
    };
    println!("Windows Steam {verb} for AppID {app_id}");
    println!("Runtime:  {}", outcome.runtime_id);
    println!("Prefix:   {}", outcome.prefix.display());
    println!("Client:   {}", outcome.steam_executable.display());
    println!("Evidence: {}", outcome.marker.display());
    Ok(())
}

fn steam_rebind_runtime_command(args: &[String]) -> Result<(), String> {
    let usage = "usage: macroton steam rebind-runtime APPID --profile PATH --from-runtime-fingerprint SHA256 [--runtime-root PATH] [--data-root PATH]";
    let app_id: u32 = args
        .first()
        .ok_or_else(|| usage.to_owned())?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut runtime_root = default_runtime_root()?;
    let mut data_root = default_data_root()?;
    let mut profile_path = None;
    let mut old_fingerprint = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--from-runtime-fingerprint" => old_fingerprint = Some(value.clone()),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--data-root" => data_root = PathBuf::from(value),
            other => return Err(format!("unknown steam rebind-runtime option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let old_fingerprint = old_fingerprint
        .ok_or_else(|| "--from-runtime-fingerprint SHA256 is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    let outcome = steam_bootstrap::rebind_runtime(&steam_bootstrap::RebindOptions {
        app_id,
        profile: &profile,
        expected_old_fingerprint: &old_fingerprint,
        runtime_root: &runtime_root,
        data_root: &data_root,
    })
    .map_err(|error| error.to_string())?;
    println!("rebound Windows Steam runtime for AppID {app_id}");
    println!(
        "Previous: {} ({})",
        outcome.old_runtime_id, outcome.old_runtime_fingerprint_sha256
    );
    println!(
        "Current:  {} ({})",
        outcome.runtime_id, outcome.runtime_fingerprint_sha256
    );
    println!("Prefix:   {}", outcome.prefix.display());
    println!("Evidence: {}", outcome.marker.display());
    Ok(())
}

fn runtime_command(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(
            "usage: macroton runtime <discover|audit|release-audit|profile-release-audit|probe-win32|register-external|verify|install|list> [arguments]"
                .to_owned(),
        );
    };
    match command {
        "discover" => {
            if args.len() != 1 {
                return Err("usage: macroton runtime discover".to_owned());
            }
            let candidates = runtime::discover_external_candidates();
            if candidates.is_empty() {
                println!("no supported external Wine installations discovered");
            } else {
                for candidate in candidates {
                    println!(
                        "{}\t{}\t{}",
                        candidate.provider,
                        candidate.version.as_deref().unwrap_or("version-unknown"),
                        candidate.entrypoint.display()
                    );
                }
            }
            Ok(())
        }
        "probe-win32" => {
            if args.len() != 2 {
                return Err("usage: macroton runtime probe-win32 ENTRYPOINT".to_owned());
            }
            let result = runtime::probe_win32(Path::new(&args[1]), Path::new("/private/tmp"))
                .map_err(|error| error.to_string())?;
            println!("Win32 probe passed");
            println!("Runtime: {}", result.entrypoint.display());
            println!("Host:    {}", result.host_execution.as_str());
            println!("Payload: {}", result.payload.display());
            println!("Result:  {}", result.reported_windows_version);
            Ok(())
        }
        "audit" => {
            if args.len() != 2 {
                return Err("usage: macroton runtime audit ENTRYPOINT".to_owned());
            }
            let result = runtime::audit_runtime(Path::new(&args[1]), Path::new("/private/tmp"))
                .map_err(|error| error.to_string())?;
            println!("Runtime audit");
            println!("Runtime:   {}", result.entrypoint.display());
            println!("Host:      {}", result.host_execution.as_str());
            println!("Readiness: {}", result.readiness.as_str());
            print_payload_probe("Native", &result.native_payload);
            print_payload_probe("Win32", &result.win32_payload);
            Ok(())
        }
        "release-audit" => {
            let manifest_path = args.get(1).ok_or_else(|| {
                "usage: macroton runtime release-audit MANIFEST --runtime-root PATH [--json]"
                    .to_owned()
            })?;
            let mut runtime_root = None;
            let mut json = false;
            let mut index = 2;
            while index < args.len() {
                let option = args[index].as_str();
                index += 1;
                if option == "--json" {
                    json = true;
                    continue;
                }
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("{option} requires a value"))?;
                match option {
                    "--runtime-root" => runtime_root = Some(PathBuf::from(value)),
                    other => {
                        return Err(format!("unknown runtime release-audit option {other:?}"));
                    }
                }
                index += 1;
            }
            let runtime_root =
                runtime_root.ok_or_else(|| "--runtime-root PATH is required".to_owned())?;
            let manifest = RuntimeManifest::load(Path::new(manifest_path))
                .map_err(|error| error.to_string())?;
            let result = runtime::audit_release_tree(&manifest, &runtime_root)
                .map_err(|error| error.to_string())?;
            if json {
                print!("{}", release_tree_audit_json(&result));
                return Ok(());
            }
            println!("Release runtime audit passed");
            println!("Runtime:  {}", result.runtime_id);
            println!("SHA-256: {}", result.fingerprint_sha256);
            println!("Entries:  {}", result.entries_checked);
            println!("Symlinks: {}", result.symbolic_links_checked);
            println!(
                "Entrypoint dependencies: {}",
                result.entrypoint_dependencies_checked
            );
            println!(
                "Policy:   managed HTTPS LGPL runtime tree contains no CrossOver, CodeWeavers, or D3DMetal path/link identifiers"
            );
            println!("Boundary: file contents and live process mappings remain separate gates");
            Ok(())
        }
        "profile-release-audit" => runtime_profile_release_audit_command(&args[1..]),
        "register-external" => runtime_register_external_command(&args[1..]),
        "verify" => {
            if args.len() != 3 {
                return Err("usage: macroton runtime verify MANIFEST ARTIFACT".to_owned());
            }
            let manifest =
                RuntimeManifest::load(Path::new(&args[1])).map_err(|error| error.to_string())?;
            runtime::verify_artifact(&manifest, Path::new(&args[2]))
                .map_err(|error| error.to_string())?;
            println!("verified {} ({})", manifest.id, manifest.sha256);
            Ok(())
        }
        "install" => runtime_install_command(&args[1..]),
        "list" => runtime_list_command(&args[1..]),
        other => Err(format!("unknown runtime command {other:?}")),
    }
}

fn runtime_profile_release_audit_command(args: &[String]) -> Result<(), String> {
    let profile_path = args.first().ok_or_else(|| {
        "usage: macroton runtime profile-release-audit PROFILE MANIFEST --runtime-root PATH --windows-steam-root PATH [--json]"
            .to_owned()
    })?;
    let manifest_path = args.get(1).ok_or_else(|| {
        "usage: macroton runtime profile-release-audit PROFILE MANIFEST --runtime-root PATH --windows-steam-root PATH [--json]"
            .to_owned()
    })?;
    let mut runtime_root = None;
    let mut windows_steam_root = None;
    let mut json = false;
    let mut index = 2;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--json" {
            json = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--runtime-root" => runtime_root = Some(PathBuf::from(value)),
            "--windows-steam-root" => windows_steam_root = Some(PathBuf::from(value)),
            other => return Err(format!("unknown profile release-audit option {other:?}")),
        }
        index += 1;
    }
    let runtime_root = runtime_root.ok_or_else(|| "--runtime-root PATH is required".to_owned())?;
    let windows_steam_root =
        windows_steam_root.ok_or_else(|| "--windows-steam-root PATH is required".to_owned())?;
    let profile =
        CompatibilityProfile::load(Path::new(profile_path)).map_err(|error| error.to_string())?;
    let manifest =
        RuntimeManifest::load(Path::new(manifest_path)).map_err(|error| error.to_string())?;
    validate_release_profile(&profile, &manifest)?;
    let win32_evidence = verified_win32_release_gate(&profile, &windows_steam_root, &runtime_root)?;
    let runtime_audit =
        runtime::audit_release_tree(&manifest, &runtime_root).map_err(|error| error.to_string())?;
    if json {
        println!(
            concat!(
                "{{\n",
                "  \"schema_version\": 2,\n",
                "  \"appid\": {},\n",
                "  \"profile_sha256\": \"{}\",\n",
                "  \"runtime_id\": \"{}\",\n",
                "  \"runtime_fingerprint_sha256\": \"{}\",\n",
                "  \"renderer\": \"{}\",\n",
                "  \"graphics_api\": \"{}\",\n",
                "  \"steam_cloud_policy\": \"{}\",\n",
                "  \"steam_cloud_evidence\": \"{}\",\n",
                "  \"windows_build_id\": \"{}\",\n",
                "  \"pinned_depot_count\": {},\n",
                "  \"windows_payload_root\": \"{}\",\n",
                "  \"win32_target_gate\": \"{}\",\n",
                "  \"entrypoint_architecture\": \"i386\",\n",
                "  \"entrypoint_sha256\": \"{}\",\n",
                "  \"steam_client_mode\": \"standard\",\n",
                "  \"forbidden_profile_identifiers\": 0,\n",
                "  \"status\": \"profile-release-audit-pass\",\n",
                "  \"not_inferred\": [\"gameplay\",\"input\",\"presentation\",\"audio\",\"save-reload\",\"fresh-cloud-roundtrip\",\"stability\"]\n",
                "}}"
            ),
            profile.app_id,
            profile.fingerprint(),
            manifest.id,
            runtime_audit.fingerprint_sha256,
            json_escape(&profile.renderer),
            profile.graphics_api.as_str(),
            profile
                .steam_cloud
                .effective(!profile.cloud_files.is_empty())
                .as_str(),
            steam_cloud_evidence_mode(&profile),
            profile.windows_build_id.as_deref().unwrap_or(""),
            profile.windows_depots.len(),
            profile.windows_payload_root.as_str(),
            win32_evidence.gate,
            win32_evidence.entrypoint_sha256,
        );
    } else {
        println!("Profile release audit passed for AppID {}", profile.app_id);
        println!("Profile: {}", profile.fingerprint());
        println!(
            "Runtime: {} ({})",
            manifest.id, runtime_audit.fingerprint_sha256
        );
        println!(
            "Renderer/API: {} / {}",
            profile.renderer,
            profile.graphics_api.as_str()
        );
        println!("Pinned depots: {}", profile.windows_depots.len());
        println!("Payload root: {}", profile.windows_payload_root.as_str());
        println!("Win32 gate: {}", win32_evidence.gate);
        println!("Entrypoint: i386 / {}", win32_evidence.entrypoint_sha256);
        println!("Boundary: runtime/gameplay acceptance remains separately evidenced");
    }
    Ok(())
}

struct ReleaseWin32Evidence {
    gate: &'static str,
    entrypoint_sha256: String,
}

fn verified_win32_release_gate(
    profile: &CompatibilityProfile,
    steam_root: &Path,
    runtime_root: &Path,
) -> Result<ReleaseWin32Evidence, String> {
    let game = steam::scan(steam_root)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|game| game.app_id == profile.app_id)
        .ok_or_else(|| {
            format!(
                "release profile AppID {} is absent from the supplied Steam root",
                profile.app_id
            )
        })?;
    let release_plan = plan::build(&game, profile, runtime_root);
    let architecture = macroton::binary::inspect_pe_architecture(&release_plan.executable)
        .map_err(|error| format!("cannot inspect release entrypoint architecture: {error}"))?;
    let gate = release_win32_gate(
        profile.windows_payload_root,
        &release_plan.blockers,
        architecture,
    )?;
    let entrypoint_sha256 = sha256::to_hex(
        &sha256::digest_file(&release_plan.executable)
            .map_err(|error| format!("cannot hash release entrypoint: {error}"))?,
    );
    Ok(ReleaseWin32Evidence {
        gate,
        entrypoint_sha256,
    })
}

fn release_win32_gate(
    payload_root: WindowsPayloadRoot,
    blockers: &[String],
    architecture: Option<PeArchitecture>,
) -> Result<&'static str, String> {
    if !blockers.is_empty() {
        return Err(format!(
            "release profile launch plan is blocked: {}",
            blockers.join("; ")
        ));
    }
    if architecture != Some(PeArchitecture::I386) {
        return Err(format!(
            "release profile requires an exact PE32/i386 game entrypoint; observed {}",
            architecture
                .map(|value| format!("{value:?}").to_ascii_lowercase())
                .unwrap_or_else(|| "unknown".to_owned())
        ));
    }
    Ok(match payload_root {
        WindowsPayloadRoot::NativeDepotContent => "candidate-verified-win32-depot-content",
        WindowsPayloadRoot::SteamInstalledTree => "candidate-verified-win32-installed",
    })
}

fn validate_release_profile(
    profile: &CompatibilityProfile,
    manifest: &RuntimeManifest,
) -> Result<(), String> {
    let runtime_id = format!("{}-{}", profile.runtime, profile.runtime_version);
    if runtime_id != manifest.id {
        return Err(format!(
            "profile runtime {runtime_id} does not match release manifest {}",
            manifest.id
        ));
    }
    if profile.launch_mode != macroton::profile::LaunchMode::Steam {
        return Err("release profile must use launch=steam".to_owned());
    }
    if profile.steam_client_mode != macroton::profile::SteamClientMode::Standard {
        return Err("release profile must use the Standard Steam client mode".to_owned());
    }
    if profile.renderer == "auto" {
        return Err("release profile must pin an explicit renderer".to_owned());
    }
    if profile.graphics_api == macroton::profile::GraphicsApi::Unknown {
        return Err("release profile must declare a known graphics_api".to_owned());
    }
    if profile.windows_build_id.is_none() {
        return Err("release profile must pin windows_build_id".to_owned());
    }
    if profile
        .steam_cloud
        .effective(!profile.cloud_files.is_empty())
        == SteamCloudPolicy::Unspecified
    {
        return Err(
            "release profile must declare cloud_files or steam_cloud=unsupported".to_owned(),
        );
    }
    if profile.windows_depots.is_empty()
        || profile
            .windows_depots
            .iter()
            .any(|depot| depot.manifest_id.is_none())
    {
        return Err("release profile must pin every Windows depot manifest".to_owned());
    }
    let mut values = vec![
        profile.runtime.as_str(),
        profile.runtime_version.as_str(),
        profile.renderer.as_str(),
    ];
    values.extend(profile.arguments.iter().map(String::as_str));
    values.extend(profile.environment.keys().map(String::as_str));
    values.extend(profile.environment.values().map(String::as_str));
    if values.iter().any(|value| {
        let normalized = value.to_ascii_lowercase();
        ["crossover", "codeweavers", "d3dmetal"]
            .iter()
            .any(|forbidden| normalized.contains(forbidden))
    }) {
        return Err(
            "release profile contains a forbidden commercial-runtime identifier".to_owned(),
        );
    }
    Ok(())
}

fn steam_cloud_evidence_mode(profile: &CompatibilityProfile) -> &'static str {
    match profile
        .steam_cloud
        .effective(!profile.cloud_files.is_empty())
    {
        SteamCloudPolicy::Required => "profile-file-mappings",
        SteamCloudPolicy::Unsupported => "official-store-review",
        SteamCloudPolicy::Unspecified => "unresolved",
    }
}

fn release_tree_audit_json(audit: &runtime::ReleaseTreeAudit) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"schema_version\": 1,\n",
            "  \"runtime_id\": \"{}\",\n",
            "  \"runtime_fingerprint_sha256\": \"{}\",\n",
            "  \"entries_checked\": {},\n",
            "  \"symbolic_links_checked\": {},\n",
            "  \"entrypoint_dependencies_checked\": {},\n",
            "  \"managed_archive\": true,\n",
            "  \"https_source\": true,\n",
            "  \"lgpl_declared\": true,\n",
            "  \"forbidden_path_or_dependency_identifiers\": 0,\n",
            "  \"status\": \"release-tree-audit-pass\",\n",
            "  \"not_inferred\": [\"full-source-provenance\",\"live-process-independence\",\"game-compatibility\"]\n",
            "}}\n"
        ),
        json_escape(&audit.runtime_id),
        audit.fingerprint_sha256,
        audit.entries_checked,
        audit.symbolic_links_checked,
        audit.entrypoint_dependencies_checked,
    )
}

fn print_payload_probe(label: &str, probe: &runtime::PayloadProbe) {
    println!(
        "{label}:    {} ({})",
        if probe.passed { "passed" } else { "failed" },
        probe
            .payload
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "no payload".to_owned())
    );
    println!("{label} detail: {}", probe.detail);
}

fn runtime_register_external_command(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err(
            "usage: macroton runtime register-external ID ENTRYPOINT [--runtime-root PATH]"
                .to_owned(),
        );
    }
    let runtime_id = &args[0];
    let entrypoint = Path::new(&args[1]);
    let mut runtime_root = default_runtime_root()?;
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--runtime-root" => {
                index += 1;
                runtime_root = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--runtime-root requires a path".to_owned())?,
                );
                index += 1;
            }
            other => return Err(format!("unknown register-external option {other:?}")),
        }
    }
    match runtime::register_external(runtime_id, entrypoint, &runtime_root)
        .map_err(|error| error.to_string())?
    {
        InstallOutcome::Installed => println!(
            "registered external runtime {runtime_id} at {}",
            runtime_root.display()
        ),
        InstallOutcome::AlreadyInstalled => {
            println!("external runtime already registered: {runtime_id}")
        }
    }
    Ok(())
}

fn runtime_install_command(args: &[String]) -> Result<(), String> {
    if args.len() < 2 {
        return Err(
            "usage: macroton runtime install MANIFEST ARTIFACT [--runtime-root PATH]".to_owned(),
        );
    }
    let manifest = RuntimeManifest::load(Path::new(&args[0])).map_err(|error| error.to_string())?;
    let artifact = Path::new(&args[1]);
    let mut runtime_root = default_runtime_root()?;
    let mut index = 2;
    while index < args.len() {
        if args[index] != "--runtime-root" {
            return Err(format!("unknown runtime install option {:?}", args[index]));
        }
        index += 1;
        runtime_root = PathBuf::from(
            args.get(index)
                .ok_or_else(|| "--runtime-root requires a path".to_owned())?,
        );
        index += 1;
    }
    match runtime::install(&manifest, artifact, &runtime_root).map_err(|error| error.to_string())? {
        InstallOutcome::Installed => {
            println!("installed {} at {}", manifest.id, runtime_root.display())
        }
        InstallOutcome::AlreadyInstalled => {
            println!("already installed: {}", manifest.id)
        }
    }
    Ok(())
}

fn runtime_list_command(args: &[String]) -> Result<(), String> {
    let mut runtime_root = default_runtime_root()?;
    match args {
        [] => {}
        [option, path] if option == "--runtime-root" => runtime_root = PathBuf::from(path),
        _ => return Err("usage: macroton runtime list [--runtime-root PATH]".to_owned()),
    }
    let ids = runtime::installed_runtime_ids(&runtime_root).map_err(|error| error.to_string())?;
    if ids.is_empty() {
        println!(
            "no managed runtimes installed in {}",
            runtime_root.display()
        );
    } else {
        for id in ids {
            println!("{id}");
        }
    }
    Ok(())
}

fn scan_command(args: &[String]) -> Result<(), String> {
    let mut steam_root = steam::default_steam_root()
        .ok_or_else(|| "HOME is not set; pass --steam-root PATH".to_owned())?;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--steam-root" => {
                index += 1;
                steam_root = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--steam-root requires a path".to_owned())?,
                );
            }
            "--json" => json = true,
            other => return Err(format!("unknown scan option {other:?}")),
        }
        index += 1;
    }
    let games = steam::scan(&steam_root).map_err(|error| error.to_string())?;
    if json {
        print_games_json(&games);
    } else {
        println!("{:<10}  {:<16}  NAME", "APPID", "LOCAL EVIDENCE");
        for game in &games {
            println!(
                "{:<10}  {:<16}  {}",
                game.app_id,
                game.classification.as_str(),
                game.name
            );
            if game.inspection_truncated {
                println!("            warning: file inspection stopped at safety limit");
            }
        }
        println!(
            "\n{} installed app(s); remote Windows depot availability was not queried.",
            games.len()
        );
    }
    Ok(())
}

fn inspect_command(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or_else(|| "usage: macroton inspect PATH".to_owned())?;
    if args.len() != 1 {
        return Err("usage: macroton inspect PATH".to_owned());
    }
    let kind = macroton::binary::inspect(Path::new(path)).map_err(|error| error.to_string())?;
    println!("{}", format_binary_kind(&kind));
    Ok(())
}

fn profile_command(args: &[String]) -> Result<(), String> {
    if args.len() != 2 || args[0] != "validate" {
        return Err("usage: macroton profile validate PATH".to_owned());
    }
    let path = Path::new(&args[1]);
    let profile = CompatibilityProfile::load(path).map_err(|error| error.to_string())?;
    let expected_name = format!("{}.conf", profile.app_id);
    if path.file_name().and_then(|name| name.to_str()) != Some(expected_name.as_str()) {
        return Err(format!(
            "profile filename must be {expected_name} for AppID {}",
            profile.app_id
        ));
    }
    println!("valid profile: AppID {} ({})", profile.app_id, path.display());
    Ok(())
}

fn inspect_pe_command(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .ok_or_else(|| "usage: macroton inspect-pe PATH [--json]".to_owned())?;
    let json = match &args[1..] {
        [] => false,
        [option] if option == "--json" => true,
        _ => return Err("usage: macroton inspect-pe PATH [--json]".to_owned()),
    };
    let path = Path::new(path);
    let architecture = macroton::binary::inspect_pe_architecture(path)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("not a valid Windows PE executable: {}", path.display()))?;
    let evidence = macroton::binary::inspect_pe_compatibility(path)
        .map_err(|error| format!("cannot inspect PE compatibility evidence: {error}"))?;
    let architecture = format!("{architecture:?}").to_ascii_lowercase();
    let graphics = evidence
        .graphics
        .iter()
        .map(|api| api.as_str())
        .collect::<Vec<_>>();
    let input = evidence
        .input
        .iter()
        .map(|api| api.as_str())
        .collect::<Vec<_>>();
    let audio = evidence
        .audio
        .iter()
        .map(|api| api.as_str())
        .collect::<Vec<_>>();
    let prerequisites = evidence
        .prerequisites
        .iter()
        .map(|prerequisite| prerequisite.as_str())
        .collect::<Vec<_>>();
    if json {
        let array = |values: &[&str]| {
            values
                .iter()
                .map(|value| format!("\"{}\"", json_escape(value)))
                .collect::<Vec<_>>()
                .join(",")
        };
        println!("{{");
        println!("  \"schema\": 1,");
        println!("  \"architecture\": \"{}\",", json_escape(&architecture));
        println!("  \"graphics_apis\": [{}],", array(&graphics));
        println!("  \"input_apis\": [{}],", array(&input));
        println!("  \"audio_apis\": [{}],", array(&audio));
        println!("  \"prerequisites\": [{}]", array(&prerequisites));
        println!("}}");
    } else {
        let display = |values: &[&str]| {
            if values.is_empty() {
                "none observed".to_owned()
            } else {
                values.join(",")
            }
        };
        println!("Architecture:  {architecture}");
        println!("Graphics APIs: {}", display(&graphics));
        println!("Input APIs:    {}", display(&input));
        println!("Audio APIs:    {}", display(&audio));
        println!("Prerequisites: {}", display(&prerequisites));
    }
    Ok(())
}

fn plan_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton plan APPID --profile PATH [--steam-root PATH] [--runtime-root PATH]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut steam_root = steam::default_steam_root()
        .ok_or_else(|| "HOME is not set; pass --steam-root PATH".to_owned())?;
    let mut runtime_root = default_runtime_root()?;
    let mut profile_path = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a path"))?;
        match option {
            "--steam-root" => steam_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            other => return Err(format!("unknown plan option {other:?}")),
        }
        index += 1;
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    let games = steam::scan(&steam_root).map_err(|error| error.to_string())?;
    let game = games
        .iter()
        .find(|game| game.app_id == app_id)
        .ok_or_else(|| format!("AppID {app_id} is not installed in the discovered libraries"))?;
    let plan = plan::build(game, &profile, &runtime_root);
    print_launch_plan(&plan, "macroton launch");
    Ok(())
}

fn print_launch_plan(plan: &plan::LaunchPlan, launch_command: &str) {
    println!("AppID:      {}", plan.app_id);
    println!("Game:       {}", plan.game_name);
    println!(
        "Depots:     {}",
        if plan.windows_depots.is_empty() {
            "local-install".to_owned()
        } else {
            plan.windows_depots
                .iter()
                .map(|depot| match depot.manifest_id {
                    Some(manifest) => format!("{}:{}", depot.id, manifest),
                    None => depot.id.to_string(),
                })
                .collect::<Vec<_>>()
                .join(",")
        }
    );
    println!(
        "Payloads:   {}",
        plan.payload_roots
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("Runtime:    {}", plan.runtime_id);
    println!(
        "Components: {}",
        if plan.runtime_components.is_empty() {
            "none".to_owned()
        } else {
            plan.runtime_components.join(",")
        }
    );
    println!("Profile:    {}", plan.profile_sha256);
    println!("Renderer:   {}", plan.renderer);
    println!("Graphics:   {}", plan.graphics_api.as_str());
    println!(
        "PE evidence: {}",
        if plan.graphics_evidence.is_empty() {
            "none observed".to_owned()
        } else {
            plan.graphics_evidence.join(",")
        }
    );
    println!("Launch:     {}", plan.launch_mode.as_str());
    if plan.launch_mode == macroton::profile::LaunchMode::Steam {
        println!("Steam UI:   {}", plan.steam_client_mode.as_str());
    }
    println!("Executable: {}", plan.executable.display());
    println!(
        "Arguments:  {}",
        if plan.arguments.is_empty() {
            "(none)".to_owned()
        } else {
            plan.arguments.join(" ")
        }
    );
    if plan.is_ready() {
        println!("Status:     ready (use `{launch_command}` to execute this plan)");
    } else {
        println!("Status:     blocked");
        for blocker in &plan.blockers {
            println!("  - {blocker}");
        }
    }
}

fn steam_port_preflight_warning() -> Option<String> {
    match TcpListener::bind(("0.0.0.0", 27036)) {
        Ok(listener) => {
            drop(listener);
            None
        }
        Err(error) => Some(format_steam_port_warning(&error)),
    }
}

fn format_steam_port_warning(error: &io::Error) -> String {
    format!(
        "TCP 27036 is already occupied or unavailable ({error}); native Steam or another Steam-compatible client may conflict with Wine Steam. Macroton will not terminate the other process; close the conflicting client before retrying if the Wine client cannot connect"
    )
}

fn format_steam_port_warning_for_scope(warning: &str, exact_prefix_has_processes: bool) -> String {
    if exact_prefix_has_processes {
        format!(
            "{warning}; the listener may belong to this AppID's existing Wine prefix. Macroton will reuse only its exact-prefix session and will not start a second client"
        )
    } else {
        warning.to_owned()
    }
}

fn other_live_steam_prefix_warning(app_id: u32, data_root: &Path) -> Result<Option<String>, String> {
    let games_root = data_root.join("games");
    let entries = match fs::read_dir(&games_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot inspect Steam prefixes in {}: {error}", games_root.display())),
    };
    let mut live = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot inspect Steam prefix entry: {error}"))?;
        let Some(other_id) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        if other_id == app_id || !entry.path().is_dir() {
            continue;
        }
        let resources = stop::inspect_resources(other_id, data_root)
            .map_err(|error| format!("cannot inspect AppID {other_id} resources: {error}"))?;
        if resources.processes.iter().any(is_primary_steam_client_process) {
            live.push(other_id);
        }
    }
    live.sort_unstable();
    Ok((!live.is_empty()).then(|| format!(
        "another Macroton Wine Steam client is live for AppID(s) {}; starting this client may replace its account session and leave the old window showing NO CONNECTION. Macroton will not stop it automatically",
        live.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
    )))
}

fn play_profile(app_id: u32, explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let name = format!("{app_id}.conf");
    let mut candidates = Vec::new();
    if let Some(directory) = env::var_os("MACROTON_PROFILES_DIR") {
        candidates.push(PathBuf::from(directory).join(&name));
    }
    if let Ok(executable) = env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("compatibility").join(&name));
            if let Some(parent) = directory.parent() {
                candidates.push(parent.join("compatibility").join(&name));
                if let Some(grandparent) = parent.parent() {
                    candidates.push(grandparent.join("compatibility").join(&name));
                }
            }
        }
    }
    candidates.push(PathBuf::from("compatibility").join(&name));
    candidates.into_iter().find(|path| path.is_file()).ok_or_else(|| {
        format!("no compatibility profile for AppID {app_id}; pass --profile PATH or set MACROTON_PROFILES_DIR")
    })
}

const CLOUD_HANDOFF_PENDING: &str = ".macroton-cloud-handoff-pending";
const SHARED_CLIENT_MARKER: &str = ".macroton-shared-client";
const SHARED_CLIENT_PROFILE_PATH: &str = ".macroton-shared-client-profile-path";
const SHARED_CLIENT_LOCK: &str = ".macroton-shared-client-operation";

struct SharedClientLock(PathBuf);

impl SharedClientLock {
    fn acquire(_client_app_id: u32, data_root: &Path) -> Result<Self, String> {
        let root = data_root.join("games");
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let path = root.join(SHARED_CLIENT_LOCK);
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path)
            .map_err(|error| format!("shared-client operation is already active or its lock needs review at {}: {error}", path.display()))?;
        if let Err(error) = file.write_all(format!("pid={}\n", std::process::id()).as_bytes()) {
            let _ = fs::remove_file(&path);
            return Err(format!("cannot write shared-client lock {}: {error}", path.display()));
        }
        Ok(Self(path))
    }
}

impl Drop for SharedClientLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn shared_client_marker(app_id: u32, data_root: &Path) -> PathBuf {
    data_root.join("games").join(app_id.to_string()).join(SHARED_CLIENT_MARKER)
}

fn shared_client_profile_path_marker(app_id: u32, data_root: &Path) -> PathBuf {
    data_root.join("games").join(app_id.to_string()).join(SHARED_CLIENT_PROFILE_PATH)
}

fn read_shared_client_marker(app_id: u32, data_root: &Path) -> Result<Option<(u32, String)>, String> {
    let path = shared_client_marker(app_id, data_root);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read shared-client marker {}: {error}", path.display())),
    };
    let mut lines = contents.lines();
    let valid = lines.next() == Some("schema=1")
        && lines.next() == Some(format!("appid={app_id}").as_str());
    let owner = lines.next().and_then(|line| line.strip_prefix("client_appid="))
        .and_then(|value| value.parse::<u32>().ok());
    let fingerprint = lines.next().and_then(|line| line.strip_prefix("profile_sha256="))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if !valid || owner.is_none_or(|owner| owner == app_id) || fingerprint.is_none() || lines.next().is_some() {
        return Err(format!("invalid shared-client marker {}", path.display()));
    }
    Ok(Some((owner.unwrap(), fingerprint.unwrap().to_owned())))
}

fn record_shared_client_marker(app_id: u32, client_app_id: u32, data_root: &Path, profile: &CompatibilityProfile, profile_path: &Path) -> Result<(), String> {
    if app_id == client_app_id || profile.app_id != app_id {
        return Err("shared-client marker target/profile mismatch".to_owned());
    }
    let marker = shared_client_marker(app_id, data_root);
    fs::create_dir_all(marker.parent().ok_or("shared-client marker has no parent")?)
        .map_err(|error| error.to_string())?;
    if marker.exists() {
        return Err(format!("shared-client marker already exists: {}", marker.display()));
    }
    let canonical_profile = profile_path.canonicalize().map_err(|error| error.to_string())?;
    let profile_text = canonical_profile.to_str()
        .filter(|value| !value.contains(['\n', '\r']))
        .ok_or("shared-client profile path must be valid single-line UTF-8")?;
    fs::write(shared_client_profile_path_marker(app_id, data_root), profile_text)
        .map_err(|error| format!("cannot save shared-client profile path: {error}"))?;
    let temporary = marker.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, format!("schema=1\nappid={app_id}\nclient_appid={client_app_id}\nprofile_sha256={}\n", profile.fingerprint()))
        .map_err(|error| format!("cannot write shared-client marker {}: {error}", temporary.display()))?;
    let result = fs::hard_link(&temporary, &marker)
        .map_err(|error| format!("cannot commit shared-client marker {}: {error}", marker.display()));
    let _ = fs::remove_file(&temporary);
    result
}

fn shared_client_markers(data_root: &Path) -> Result<Vec<(u32, u32, String)>, String> {
    let games_root = data_root.join("games");
    let entries = match fs::read_dir(&games_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot scan shared-client markers: {error}")),
    };
    let mut markers = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let Some(app_id) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else { continue };
        if let Some((owner, fingerprint)) = read_shared_client_marker(app_id, data_root)? {
            markers.push((app_id, owner, fingerprint));
        }
    }
    markers.sort_by_key(|(app_id, _, _)| *app_id);
    Ok(markers)
}

struct SharedClientAssessment {
    resident_id: u32,
    target_id: u32,
    resident: CompatibilityProfile,
    target: CompatibilityProfile,
    target_path: PathBuf,
    resident_plan: plan::LaunchPlan,
    data_root: PathBuf,
    runtime_root: PathBuf,
    steam_root: PathBuf,
    blockers: Vec<String>,
}

fn inspect_shared_client_candidate(args: &[String], require_installed: bool) -> Result<SharedClientAssessment, String> {
    let usage = "usage: macroton steam shared-client-<status|install|launch> RESIDENT_APPID TARGET_APPID [--resident-profile PATH] [--target-profile PATH] [--data-root PATH] [--runtime-root PATH]";
    let resident_id: u32 = args.first().ok_or(usage)?.parse()
        .map_err(|_| "RESIDENT_APPID must be an unsigned integer")?;
    let target_id: u32 = args.get(1).ok_or(usage)?.parse()
        .map_err(|_| "TARGET_APPID must be an unsigned integer")?;
    let mut resident_path = None;
    let mut target_path = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 2;
    while index < args.len() {
        let option = args[index].as_str();
        let value = args.get(index + 1).ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--resident-profile" => resident_path = Some(PathBuf::from(value)),
            "--target-profile" => target_path = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown shared-client-status option {other:?}")),
        }
        index += 2;
    }
    let resident_path = play_profile(resident_id, resident_path)?;
    let target_path = play_profile(target_id, target_path)?;
    let resident = CompatibilityProfile::load(&resident_path).map_err(|error| error.to_string())?;
    let target = CompatibilityProfile::load(&target_path).map_err(|error| error.to_string())?;
    if resident.app_id != resident_id || target.app_id != target_id {
        return Err("a selected profile AppID does not match its requested AppID".to_owned());
    }
    let resident_plan = plan::build_steam_client(&resident, &runtime_root);
    let target_plan = plan::build_steam_client(&target, &runtime_root);
    let mut blockers = macroton::shared_client::profile_blockers(
        &resident, &target, &resident_plan, &target_plan);
    let prefix = data_root.join("games").join(resident_id.to_string()).join("prefix");
    let steam_root = prefix.join("drive_c/Program Files (x86)/Steam");
    match resident_plan.runtime_fingerprint_sha256.as_deref() {
        Some(fingerprint) => {
            if let Err(error) = steam_prefix_status::validate_expected_runtime(
                resident_id, &prefix, &resident_plan.runtime_id, fingerprint) {
                blockers.push(format!("resident prefix provenance: {error}"));
            }
        }
        None => blockers.push("resident runtime fingerprint is unavailable".to_owned()),
    }
    match steam_prefix_status::inspect(resident_id, &data_root) {
        Ok(status) if status.readiness == steam_prefix_status::Readiness::StoredLoginPresent => (),
        Ok(status) => blockers.push(format!("resident Steam login state is {}", status.readiness.as_str())),
        Err(error) => blockers.push(format!("resident Steam prefix status: {error}")),
    }
    match steam_client_status::inspect(resident_id, &data_root) {
        Ok(status) if status.state == steam_client_status::State::Current => (),
        Ok(status) => blockers.push(format!("resident Steam updater state is {}", status.state.as_str())),
        Err(error) => blockers.push(format!("resident Steam client status: {error}")),
    }
    let resident_live = match stop::inspect_resources(resident_id, &data_root) {
        Ok(resources) => {
            if resources.processes.iter().any(|process| process.category == "windows-game") {
                blockers.push("resident prefix still has a running Windows game".to_owned());
            }
            resources.processes.iter().any(is_primary_steam_client_process)
        }
        Err(error) => {
            blockers.push(format!("resident process inspection unavailable: {error}"));
            false
        }
    };
    if resident_live {
        match steam_session::read(&prefix) {
            Ok(session) => {
                if session.app_id != resident_id
                    || session.profile_sha256 != resident.fingerprint()
                    || session.runtime_id != resident_plan.runtime_id
                    || Some(session.runtime_fingerprint_sha256.as_str())
                        != resident_plan.runtime_fingerprint_sha256.as_deref()
                {
                    blockers.push("resident session provenance does not match its selected profile/runtime".to_owned());
                }
            }
            Err(error) => blockers.push(format!("resident session provenance: {error}")),
        }
    } else if !blockers.iter().any(|item| item.starts_with("resident process inspection unavailable")) {
        blockers.push("resident Windows Steam client is not running".to_owned());
    }
    if require_installed {
        match steam_install_status::inspect_steam_root(target_id, &steam_root) {
            Ok(status) if status.state == steam_install_status::InstallState::Installed => (),
            Ok(status) => blockers.push(format!(
                "target is not installed in the resident Steam library (state {}); ownership remains unconfirmed",
                status.state.as_str())),
            Err(error) => blockers.push(format!("target install in resident Steam library: {error}")),
        }
    }
    Ok(SharedClientAssessment { resident_id, target_id, resident, target, target_path, resident_plan, data_root, runtime_root, steam_root, blockers })
}

fn steam_shared_client_status_command(args: &[String]) -> Result<(), String> {
    let candidate = inspect_shared_client_candidate(args, true)?;
    println!("Shared Steam client candidate: AppID {} -> AppID {}", candidate.resident_id, candidate.target_id);
    println!("Resident Steam root: {}", candidate.steam_root.display());
    println!("Profile/prefix eligibility: {}", if candidate.blockers.is_empty() { "candidate" } else { "blocked" });
    for blocker in &candidate.blockers {
        println!("  blocker: {blocker}");
    }
    if let Some((owner, fingerprint)) = read_shared_client_marker(candidate.target_id, &candidate.data_root)? {
        if owner != candidate.resident_id || fingerprint != candidate.target.fingerprint() {
            println!("Shared launch: marker belongs to another client/profile; do not reuse this session");
        } else {
            let launch = launch_status::inspect_in_client(candidate.target_id, owner, &candidate.data_root)
                .map_err(|error| error.to_string())?;
            let cloud = cloud_status::inspect_in_client(candidate.target_id, owner, &candidate.data_root)
                .map_err(|error| error.to_string())?;
            println!("Shared launch: {}; Cloud local cache: {}; remote freshness: {}",
                launch.state.as_str(), cloud.state.as_str(), cloud.remote_freshness.as_str());
        }
    } else {
        println!("Shared launch: not submitted in this client");
    }
    Ok(())
}

fn finish_shared_client_game(app_id: u32, client_app_id: u32, data_root: &Path) -> Result<(), String> {
    let (owner, fingerprint) = read_shared_client_marker(app_id, data_root)?
        .ok_or_else(|| format!("AppID {app_id} has no shared-client launch marker"))?;
    if owner != client_app_id {
        return Err(format!("AppID {app_id} shared-client marker belongs to AppID {owner}, not {client_app_id}"));
    }
    let profile_path = handoff_profile(app_id, data_root, Some(&fingerprint))?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    let launch = launch_status::inspect_in_client(app_id, client_app_id, data_root)
        .map_err(|error| error.to_string())?;
    if launch.state != launch_status::LaunchState::Exited {
        return Err(format!("AppID {app_id} shared game state is {}; wait for its exit", launch.state.as_str()));
    }
    match profile.steam_cloud.effective(!profile.cloud_files.is_empty()) {
        SteamCloudPolicy::Required => wait_for_handoff_cloud_in_client(app_id, client_app_id, data_root,
            &profile, launch.latest_event.as_deref().ok_or("shared game exit event is missing")?)?,
        SteamCloudPolicy::Unsupported => (),
        SteamCloudPolicy::Unspecified => return Err(format!("AppID {app_id} Cloud policy is unreviewed")),
    }
    let resources = stop::inspect_resources(client_app_id, data_root)
        .map_err(|error| error.to_string())?;
    if resources.processes.iter().any(|process| process.category == "windows-game") {
        return Err("a game started in the shared prefix during Cloud handoff".to_owned());
    }
    fs::remove_file(shared_client_marker(app_id, data_root))
        .map_err(|error| format!("cannot clear completed shared-client marker: {error}"))?;
    let _ = fs::remove_file(shared_client_profile_path_marker(app_id, data_root));
    println!("AppID {app_id} shared-client session finished; Cloud handoff is complete");
    Ok(())
}

fn steam_shared_client_finish_command(args: &[String]) -> Result<(), String> {
    let usage = "usage: macroton steam shared-client-finish RESIDENT_APPID TARGET_APPID [--data-root PATH]";
    let owner: u32 = args.first().ok_or(usage)?.parse().map_err(|_| usage)?;
    let target: u32 = args.get(1).ok_or(usage)?.parse().map_err(|_| usage)?;
    let mut data_root = default_data_root()?;
    if args.len() == 4 && args[2] == "--data-root" {
        data_root = PathBuf::from(&args[3]);
    } else if args.len() != 2 {
        return Err(usage.to_owned());
    }
    let _lock = SharedClientLock::acquire(owner, &data_root)?;
    finish_shared_client_game(target, owner, &data_root)
}

fn steam_shared_client_abandon_command(args: &[String]) -> Result<(), String> {
    let usage = "usage: macroton steam shared-client-abandon RESIDENT_APPID TARGET_APPID [--data-root PATH] [--runtime-root PATH]";
    let owner: u32 = args.first().ok_or(usage)?.parse().map_err(|_| usage)?;
    let target: u32 = args.get(1).ok_or(usage)?.parse().map_err(|_| usage)?;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 2;
    while index < args.len() {
        let option = args[index].as_str();
        let value = args.get(index + 1).ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown shared-client-abandon option {other:?}")),
        }
        index += 2;
    }
    let _lock = SharedClientLock::acquire(owner, &data_root)?;
    let marker_owner = read_shared_client_marker(target, &data_root)?
        .ok_or_else(|| format!("AppID {target} has no shared-client launch marker"))?.0;
    if marker_owner != owner {
        return Err(format!("AppID {target} marker belongs to AppID {marker_owner}, not {owner}"));
    }
    let launch = launch_status::inspect_in_client(target, owner, &data_root)
        .map_err(|error| error.to_string())?;
    if !matches!(launch.state, launch_status::LaunchState::NoEvidence
        | launch_status::LaunchState::Submitted | launch_status::LaunchState::Failed)
    {
        return Err(format!("AppID {target} launch state is {}; cannot abandon a game or user/Cloud prompt", launch.state.as_str()));
    }
    if launch_status::game_process_ever_observed_in_client(target, owner, &data_root)
        .map_err(|error| error.to_string())?
    {
        return Err("Steam observed a target game process after this shared request; finish its Cloud handoff instead".to_owned());
    }
    let cloud = cloud_status::inspect_in_client(target, owner, &data_root)
        .map_err(|error| error.to_string())?;
    if !cloud.pending_remote_operations.is_empty()
        || cloud.remote_freshness != cloud_status::RemoteFreshness::Unverified
        || cloud.state == cloud_status::SyncState::Syncing
    {
        return Err("target Steam Cloud has a pending or unresolved operation; cannot abandon it".to_owned());
    }
    let resources = stop::inspect_resources(owner, &data_root).map_err(|error| error.to_string())?;
    if resources.processes.iter().any(|process| process.category == "windows-game") {
        return Err("a Windows game is still running in the shared prefix".to_owned());
    }
    if !resources.pids.is_empty() {
        stop::stop_processes_preserve_session(owner, &data_root, &runtime_root)
            .map_err(|error| error.to_string())?;
    }
    fs::remove_file(shared_client_marker(target, &data_root))
        .map_err(|error| format!("cannot clear abandoned shared-client marker: {error}"))?;
    let _ = fs::remove_file(shared_client_profile_path_marker(target, &data_root));
    println!("AppID {target} unstarted shared request abandoned after stopping AppID {owner}'s Steam client; login state preserved");
    Ok(())
}

fn steam_shared_client_launch_command(args: &[String]) -> Result<(), String> {
    let candidate = inspect_shared_client_candidate(args, true)?;
    let _lock = SharedClientLock::acquire(candidate.resident_id, &candidate.data_root)?;
    if !candidate.blockers.is_empty() {
        return Err(format!("shared Steam launch is blocked: {}", candidate.blockers.join("; ")));
    }
    if !candidate.target.arguments.is_empty() || candidate.target.steam_launcher_bypass.is_some() {
        return Err("shared Steam launch currently requires a target profile without game-specific Steam arguments or launcher bypass".to_owned());
    }
    for (app_id, owner, _) in shared_client_markers(&candidate.data_root)? {
        if owner != candidate.resident_id {
            return Err(format!("AppID {app_id} still uses another shared Steam client {owner}; finish it first"));
        }
        finish_shared_client_game(app_id, owner, &candidate.data_root)?;
    }
    if let Some(fingerprint) = pending_cloud_handoff_fingerprint(candidate.resident_id, &candidate.data_root)? {
        if fingerprint != candidate.resident.fingerprint() {
            return Err("resident Cloud handoff belongs to a different profile".to_owned());
        }
        let launch = launch_status::inspect(candidate.resident_id, &candidate.data_root)
            .map_err(|error| error.to_string())?;
        if launch.state != launch_status::LaunchState::Exited {
            return Err(format!("resident game state is {}; wait for its exit", launch.state.as_str()));
        }
        if candidate.resident.steam_cloud.effective(!candidate.resident.cloud_files.is_empty()) == SteamCloudPolicy::Required {
            wait_for_handoff_cloud(candidate.resident_id, &candidate.data_root, &candidate.resident,
                launch.latest_event.as_deref().ok_or("resident game exit event is missing")?)?;
        }
    }
    let installed = steam::scan(&candidate.steam_root).map_err(|error| error.to_string())?;
    let game = installed.iter().find(|game| game.app_id == candidate.target_id)
        .ok_or_else(|| format!("target AppID {} is absent from the resident Steam library", candidate.target_id))?;
    let target_plan = plan::build(game, &candidate.target, &candidate.runtime_root);
    if !target_plan.is_ready() {
        return Err(format!("target launch plan is blocked: {}", target_plan.blockers.join("; ")));
    }
    let prefix = candidate.data_root.join("games").join(candidate.resident_id.to_string()).join("prefix");
    require_persistent_data_root(&candidate.data_root, false, "shared Windows Steam launch")?;
    require_unlocked_console_session()?;
    let resources = stop::inspect_resources(candidate.resident_id, &candidate.data_root)
        .map_err(|error| error.to_string())?;
    if resources.processes.iter().any(|process| process.category == "windows-game")
        || !resources.processes.iter().any(is_primary_steam_client_process) {
        return Err("resident Steam client changed or a game started during Cloud handoff".to_owned());
    }
    execute::prepare_steam_launch(&target_plan, &prefix).map_err(|error| error.to_string())?;
    launch_status::record_launch_boundary_in_client(candidate.target_id, candidate.resident_id, &candidate.data_root)
        .map_err(|error| error.to_string())?;
    record_shared_client_marker(candidate.target_id, candidate.resident_id, &candidate.data_root, &candidate.target, &candidate.target_path)?;
    let request = execute::request_steam_game_in_client(&candidate.resident_plan, candidate.target_id, &candidate.data_root)
        .map_err(|error| format!("shared launch request failed; the pending marker remains for safe recovery: {error}"))?;
    let fingerprint = candidate.resident_plan.runtime_fingerprint_sha256.as_deref()
        .ok_or("resident runtime fingerprint disappeared before resource guard")?;
    let guard = spawn_resource_guard(
        candidate.resident_id, &candidate.data_root, &candidate.runtime_root,
        &candidate.resident_plan.runtime_id, 4096, None, None,
        Some(&resource_guard_identity(&candidate.resident_plan, fingerprint)),
        candidate.target.executable.to_str(),
    ).map_err(|error| format!("shared request was submitted and its pending marker remains, but resource guard failed: {error}"))?;
    println!("AppID {} launch request submitted to AppID {}'s resident Steam client (PID {})",
        candidate.target_id, candidate.resident_id, request.pid);
    println!("Resource guard: PID {}, log {}", guard.0, guard.1.display());
    println!("Submission is not proof of a game window or remote Cloud freshness; inspect `macroton steam shared-client-status {} {}`",
        candidate.resident_id, candidate.target_id);
    Ok(())
}

fn steam_shared_client_install_command(args: &[String]) -> Result<(), String> {
    let candidate = inspect_shared_client_candidate(args, false)?;
    let _lock = SharedClientLock::acquire(candidate.resident_id, &candidate.data_root)?;
    if !candidate.blockers.is_empty() {
        return Err(format!("shared Steam install is blocked: {}", candidate.blockers.join("; ")));
    }
    for (app_id, owner, _) in shared_client_markers(&candidate.data_root)? {
        if owner != candidate.resident_id {
            return Err(format!("AppID {app_id} still uses another shared Steam client {owner}; finish it first"));
        }
        finish_shared_client_game(app_id, owner, &candidate.data_root)?;
    }
    let install = steam_install_status::inspect_steam_root(candidate.target_id, &candidate.steam_root)
        .map_err(|error| error.to_string())?;
    match install.state {
        steam_install_status::InstallState::Installed => {
            println!("AppID {} is already installed in AppID {}'s Steam library", candidate.target_id, candidate.resident_id);
            return Ok(());
        }
        steam_install_status::InstallState::NotRequested => (),
        state => {
            println!("AppID {} installation state in the shared Steam library: {}; ownership remains unconfirmed until Steam decides", candidate.target_id, state.as_str());
            return Ok(());
        }
    }
    let pending = pending_cloud_handoff_fingerprint(candidate.resident_id, &candidate.data_root)?;
    if let Some(fingerprint) = pending {
        if fingerprint != candidate.resident.fingerprint() {
            return Err("resident Cloud handoff belongs to a different profile; cannot install another game yet".to_owned());
        }
        let launch = launch_status::inspect(candidate.resident_id, &candidate.data_root)
            .map_err(|error| error.to_string())?;
        if launch.state != launch_status::LaunchState::Exited {
            return Err(format!("resident game state is {}; wait for its exit before shared install", launch.state.as_str()));
        }
        if candidate.resident.steam_cloud.effective(!candidate.resident.cloud_files.is_empty())
            == SteamCloudPolicy::Required
        {
            wait_for_handoff_cloud(candidate.resident_id, &candidate.data_root, &candidate.resident,
                launch.latest_event.as_deref().ok_or("resident exit event is missing")?)?;
        }
    }
    require_persistent_data_root(&candidate.data_root, false, "shared Windows Steam install")?;
    require_unlocked_console_session()?;
    let resources = stop::inspect_resources(candidate.resident_id, &candidate.data_root)
        .map_err(|error| error.to_string())?;
    if resources.processes.iter().any(|process| process.category == "windows-game")
        || !resources.processes.iter().any(is_primary_steam_client_process)
    {
        return Err("resident Steam client changed or a game started during Cloud handoff; retry after it is idle".to_owned());
    }
    let request = execute::request_steam_install_in_client(
        &candidate.resident_plan, candidate.target_id, &candidate.data_root)
        .map_err(|error| error.to_string())?;
    println!("Requested AppID {} installation in AppID {}'s live Steam client", candidate.target_id, candidate.resident_id);
    println!("Steam may request sign-in, purchase, EULA, or installation confirmation; ownership is unconfirmed");
    println!("Request PID: {}; Steam root: {}", request.pid, candidate.steam_root.display());
    println!("Progress: macroton steam install-status {} --steam-root {}",
        candidate.target_id, shell_quote(&candidate.steam_root.to_string_lossy()));
    Ok(())
}

fn cloud_handoff_marker(app_id: u32, data_root: &Path) -> PathBuf {
    data_root.join("games").join(app_id.to_string()).join(CLOUD_HANDOFF_PENDING)
}

fn record_cloud_handoff(app_id: u32, data_root: &Path, profile: &CompatibilityProfile) -> Result<(), String> {
    let marker = cloud_handoff_marker(app_id, data_root);
    let temporary = marker.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, format!("schema=1\nappid={app_id}\nprofile_sha256={}\n", profile.fingerprint()))
        .map_err(|error| format!("cannot write Cloud handoff marker {}: {error}", temporary.display()))?;
    fs::rename(&temporary, &marker)
        .map_err(|error| format!("cannot commit Cloud handoff marker {}: {error}", marker.display()))
}

fn pending_cloud_handoff_fingerprint(app_id: u32, data_root: &Path) -> Result<Option<String>, String> {
    let marker = cloud_handoff_marker(app_id, data_root);
    let contents = match fs::read_to_string(&marker) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read Cloud handoff marker {}: {error}", marker.display())),
    };
    let expected_prefix = format!("schema=1\nappid={app_id}\nprofile_sha256=");
    let fingerprint = contents.strip_prefix(&expected_prefix)
        .and_then(|value| value.strip_suffix('\n'))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| format!("invalid Cloud handoff marker {}", marker.display()))?;
    Ok(Some(fingerprint.to_owned()))
}

fn handoff_profile(app_id: u32, data_root: &Path, pending_fingerprint: Option<&str>) -> Result<PathBuf, String> {
    let prefix = data_root.join("games").join(app_id.to_string()).join("prefix");
    let fingerprint = match pending_fingerprint {
        Some(fingerprint) => fingerprint.to_owned(),
        None => steam_session::read(&prefix).map_err(|error| error.to_string())?.profile_sha256,
    };
    if let Ok(path_text) = fs::read_to_string(shared_client_profile_path_marker(app_id, data_root)) {
        let path = PathBuf::from(path_text);
        if let Ok(profile) = CompatibilityProfile::load(&path)
            && profile.app_id == app_id && profile.fingerprint() == fingerprint
        {
            return Ok(path);
        }
    }
    let mut roots = Vec::new();
    if let Some(root) = env::var_os("MACROTON_PROFILES_DIR") {
        roots.push(PathBuf::from(root));
    }
    if let Ok(executable) = env::current_exe() {
        for ancestor in executable.ancestors().take(4) {
            roots.push(ancestor.join("compatibility"));
        }
    }
    roots.push(PathBuf::from("compatibility"));
    for root in roots {
        for directory in [root.clone(), root.join("experiments")] {
            let Ok(entries) = fs::read_dir(&directory) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|extension| extension != "conf") {
                    continue;
                }
                let Ok(profile) = CompatibilityProfile::load(&path) else { continue };
                if profile.app_id == app_id && profile.fingerprint() == fingerprint {
                    return Ok(path);
                }
            }
        }
    }
    Err(format!(
        "cannot find the active profile for AppID {app_id} (fingerprint {}); keep its Steam client running until Cloud sync is verified, or place the exact profile in compatibility/ or compatibility/experiments/",
        fingerprint
    ))
}

fn steam_event_timestamp(event: &str) -> Option<&str> {
    let timestamp = event.strip_prefix('[')?.get(..19)?;
    (timestamp.as_bytes().get(4) == Some(&b'-')
        && timestamp.as_bytes().get(7) == Some(&b'-')
        && timestamp.as_bytes().get(10) == Some(&b' ')
        && timestamp.as_bytes().get(13) == Some(&b':')
        && timestamp.as_bytes().get(16) == Some(&b':'))
        .then_some(timestamp)
}

fn cloud_handoff_ready(status: &cloud_status::CloudStatus, exit_event: &str) -> bool {
    let Some(exit_time) = steam_event_timestamp(exit_event) else { return false };
    cloud_launch_ready(status.state, status.pending_remote_operations.len(), status.remote_freshness)
        && status.latest_event.as_deref().and_then(steam_event_timestamp)
            .is_some_and(|time| time >= exit_time)
}

fn wait_for_handoff_cloud(app_id: u32, data_root: &Path, profile: &CompatibilityProfile, exit_event: &str) -> Result<(), String> {
    wait_for_handoff_cloud_in_client(app_id, app_id, data_root, profile, exit_event)
}

fn wait_for_handoff_cloud_in_client(app_id: u32, client_app_id: u32, data_root: &Path, profile: &CompatibilityProfile, exit_event: &str) -> Result<(), String> {
    let exit_time = steam_event_timestamp(exit_event)
        .ok_or_else(|| format!("AppID {app_id} game exit has no timestamp; cannot verify post-exit Cloud sync"))?;
    let _idle_sleep_guard = IdleSleepGuard::acquire("cross-game Steam Cloud handoff")?;
    let started = Instant::now();
    loop {
        let mut status = cloud_status::inspect_in_client(app_id, client_app_id, data_root).map_err(|error| error.to_string())?;
        cloud_status::compare_profile_files(&mut status, profile).map_err(|error| error.to_string())?;
        let cloud_after_exit = status.latest_event.as_deref()
            .and_then(steam_event_timestamp)
            .is_some_and(|time| time >= exit_time);
        if cloud_handoff_ready(&status, exit_event) {
            println!("AppID {app_id} Steam Cloud reconciled after game exit; {} declared local file(s) match its cache", status.comparisons.len());
            return Ok(());
        }
        if started.elapsed() >= Duration::from_secs(300) {
            return Err(format!(
                "AppID {app_id} Steam Cloud was not verified after game exit within 300s (cache {}, remote {}, post-exit event {}); keeping its Steam client running and refusing the switch",
                status.state.as_str(), status.remote_freshness.as_str(), cloud_after_exit
            ));
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn handoff_other_steam_clients(app_id: u32, data_root: &Path, runtime_root: &Path) -> Result<(), String> {
    let games_root = data_root.join("games");
    if games_root.join(SHARED_CLIENT_LOCK).exists() {
        return Err("a shared Steam-client operation is in progress or its lock needs review; refusing a separate-client switch".to_owned());
    }
    if let Some((target, owner, _)) = shared_client_markers(data_root)?.first() {
        return Err(format!("AppID {target} still has a shared Steam session in AppID {owner}'s client; finish it after Cloud sync, or abandon it if no game process was ever observed"));
    }
    let entries = match fs::read_dir(&games_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot inspect Steam prefixes in {}: {error}", games_root.display())),
    };
    let mut others = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot inspect Steam prefix entry: {error}"))?;
        let Some(other_id) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else { continue };
        if other_id == app_id || !entry.path().is_dir() { continue; }
        let resources = stop::inspect_resources(other_id, data_root).map_err(|error| error.to_string())?;
        let live_client = resources.processes.iter().any(is_primary_steam_client_process);
        let pending = pending_cloud_handoff_fingerprint(other_id, data_root)?;
        if live_client || pending.is_some() {
            others.push((other_id, live_client, pending));
        }
    }
    others.sort_by_key(|(other_id, _, _)| *other_id);
    for (other_id, live_client, pending) in others {
        let launch = launch_status::inspect(other_id, data_root).map_err(|error| error.to_string())?;
        if launch.state != launch_status::LaunchState::Exited {
            return Err(format!(
                "AppID {other_id} Steam is still active (game state {}); finish that game and wait for its exit before opening AppID {app_id}",
                launch.state.as_str()
            ));
        }
        let profile_path = handoff_profile(other_id, data_root, pending.as_deref())?;
        let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
        match profile.steam_cloud.effective(!profile.cloud_files.is_empty()) {
            SteamCloudPolicy::Unsupported => println!("AppID {other_id}: reviewed Steam Cloud unsupported; no upload gate required"),
            SteamCloudPolicy::Required => {
                println!("Waiting for AppID {other_id} Steam Cloud before opening AppID {app_id}");
                wait_for_handoff_cloud(other_id, data_root, &profile,
                    launch.latest_event.as_deref().ok_or_else(|| format!("AppID {other_id} exit event is missing"))?)?;
            }
            SteamCloudPolicy::Unspecified => return Err(format!(
                "AppID {other_id} has no reviewed Steam Cloud policy or declared save paths; refusing to switch clients before its upload is known"
            )),
        }
        let launch = launch_status::inspect(other_id, data_root).map_err(|error| error.to_string())?;
        let resources = stop::inspect_resources(other_id, data_root).map_err(|error| error.to_string())?;
        if launch.state != launch_status::LaunchState::Exited
            || resources.processes.iter().any(|process| process.category == "windows-game")
        {
            return Err(format!("AppID {other_id} started a game during Cloud handoff; refusing to stop its Steam client"));
        }
        if live_client {
            stop_command(&[
                other_id.to_string(), "--preserve-session".into(),
                "--data-root".into(), data_root.to_string_lossy().into_owned(),
                "--runtime-root".into(), runtime_root.to_string_lossy().into_owned(),
            ])?;
        }
        let marker = cloud_handoff_marker(other_id, data_root);
        match fs::remove_file(&marker) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(format!("cannot clear Cloud handoff marker {}: {error}", marker.display())),
        }
        println!("AppID {other_id} Cloud handoff completed; AppID {app_id} may now start");
    }
    Ok(())
}

fn play_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args.first().ok_or("usage: macroton play APPID [--profile PATH] [--data-root PATH] [--runtime-root PATH]")?
        .parse().map_err(|_| "APPID must be an unsigned integer")?;
    let mut profile_path = None;
    let mut data_root = default_data_root()?;
    let mut runtime_root = default_runtime_root()?;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args.get(index).ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--data-root" => data_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            other => return Err(format!("unknown play option {other:?}")),
        }
        index += 1;
    }
    let profile_path = play_profile(app_id, profile_path)?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    if profile.app_id != app_id {
        return Err(format!("profile AppID {} does not match requested AppID {app_id}", profile.app_id));
    }
    if try_play_shared_client(app_id, &profile_path, &data_root, &runtime_root)? {
        return Ok(());
    }
    let prefix = steam_prefix_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    if prefix.readiness == steam_prefix_status::Readiness::NotBootstrapped {
        return Err(format!("Steam client setup is required for AppID {app_id}; use `macroton steam bootstrap {app_id} --profile {}` with a verified Steam installer", profile_path.display()));
    }
    if prefix.readiness == steam_prefix_status::Readiness::SignInRequired {
        handoff_other_steam_clients(app_id, &data_root, &runtime_root)?;
        println!("Steam sign-in is required for AppID {app_id}. Opening the existing login flow.");
        return steam_login_command(&[app_id.to_string(), "--profile".into(), profile_path.to_string_lossy().into_owned(), "--data-root".into(), data_root.to_string_lossy().into_owned(), "--runtime-root".into(), runtime_root.to_string_lossy().into_owned()]);
    }
    let install = steam_install_status::inspect(app_id, &data_root).map_err(|error| error.to_string())?;
    match install.state {
        steam_install_status::InstallState::Installed => {
            launch_command(&[app_id.to_string(), "--profile".into(), profile_path.to_string_lossy().into_owned(), "--data-root".into(), data_root.to_string_lossy().into_owned(), "--runtime-root".into(), runtime_root.to_string_lossy().into_owned()])
        }
        steam_install_status::InstallState::NotRequested => {
            handoff_other_steam_clients(app_id, &data_root, &runtime_root)?;
            println!("Requesting Steam installation for AppID {app_id}. Steam may ask for sign-in, purchase, EULA, or installation confirmation; ownership is not yet determined.");
            steam_install_command(&[app_id.to_string(), "--profile".into(), profile_path.to_string_lossy().into_owned(), "--data-root".into(), data_root.to_string_lossy().into_owned(), "--runtime-root".into(), runtime_root.to_string_lossy().into_owned()])
        }
        state => {
            println!("AppID {app_id} installation state: {}. Continue in Steam, then run `macroton play {app_id}` again. Ownership is unconfirmed by this state.", state.as_str());
            Ok(())
        }
    }
}

fn try_play_shared_client(app_id: u32, profile_path: &Path, data_root: &Path, runtime_root: &Path) -> Result<bool, String> {
    let games_root = data_root.join("games");
    let entries = match fs::read_dir(&games_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("cannot inspect existing Steam clients: {error}")),
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let Some(owner) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else { continue };
        if owner == app_id || !entry.path().is_dir() { continue; }
        let resources = stop::inspect_resources(owner, data_root).map_err(|error| error.to_string())?;
        if !resources.processes.iter().any(is_primary_steam_client_process) { continue; }
        let Ok(owner_profile) = handoff_profile(owner, data_root, None) else { continue };
        let request = vec![
            owner.to_string(), app_id.to_string(),
            "--resident-profile".into(), owner_profile.to_string_lossy().into_owned(),
            "--target-profile".into(), profile_path.to_string_lossy().into_owned(),
            "--data-root".into(), data_root.to_string_lossy().into_owned(),
            "--runtime-root".into(), runtime_root.to_string_lossy().into_owned(),
        ];
        let candidate = inspect_shared_client_candidate(&request, false)?;
        if candidate.blockers.is_empty() {
            candidates.push((candidate, request));
        }
    }
    if candidates.len() > 1 {
        return Err(format!("AppID {app_id} matches multiple live Steam clients; select one explicitly with `macroton steam shared-client-launch RESIDENT_APPID {app_id}`"));
    }
    if let Some((candidate, request)) = candidates.pop() {
        let install = steam_install_status::inspect_steam_root(app_id, &candidate.steam_root)
            .map_err(|error| error.to_string())?;
        if install.state == steam_install_status::InstallState::Installed {
            steam_shared_client_launch_command(&request)?;
        } else {
            steam_shared_client_install_command(&request)?;
        }
        return Ok(true);
    }
    if let Some((target, owner, _)) = shared_client_markers(data_root)?.first() {
        if *target == app_id {
            let launch = launch_status::inspect_in_client(app_id, *owner, data_root)
                .map_err(|error| error.to_string())?;
            if matches!(launch.state,
                launch_status::LaunchState::Submitted
                    | launch_status::LaunchState::InstallingPrerequisites
                    | launch_status::LaunchState::WaitingEula
                    | launch_status::LaunchState::WaitingCloud
                    | launch_status::LaunchState::WaitingUser
                    | launch_status::LaunchState::RunningObserved)
            {
                println!("AppID {app_id} already has a shared Steam launch state: {}; use that session or resolve its Steam prompt", launch.state.as_str());
                return Ok(true);
            }
        }
        return Err(format!("AppID {target} still has a shared Steam session in AppID {owner}'s client; finish it after Cloud sync, or abandon it if no game process was ever observed"));
    }
    Ok(false)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shortcut_game_name(app_id: u32, profile_path: &Path, fallback: Option<&str>) -> Result<String, String> {
    let catalog = profile_path.parent().and_then(Path::parent)
        .map(|root| root.join("shortcuts/catalog.tsv"));
    if let Some(catalog) = catalog.filter(|path| path.is_file()) {
        let contents = fs::read_to_string(&catalog).map_err(|error| format!("cannot read {}: {error}", catalog.display()))?;
        for line in contents.lines() {
            if line.is_empty() || line.starts_with('#') { continue; }
            let (id, name) = line.split_once('\t').ok_or_else(|| format!("invalid shortcut catalog row in {}", catalog.display()))?;
            if id.parse::<u32>().ok() == Some(app_id) {
                if name.trim() != name || name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '\0']) {
                    return Err(format!("unsafe shortcut name for AppID {app_id}"));
                }
                return Ok(name.to_owned());
            }
        }
    }
    fallback.map(str::to_owned).ok_or_else(|| format!("no shortcut name for AppID {app_id}"))
}

fn shortcut_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args.first().ok_or("usage: macroton shortcut APPID [--output PATH.command] [--profile PATH]")?
        .parse().map_err(|_| "APPID must be an unsigned integer")?;
    let mut output = None;
    let mut profile = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args.get(index).ok_or_else(|| format!("{option} requires a value"))?;
        match option {
            "--output" => output = Some(PathBuf::from(value)),
            "--profile" => profile = Some(PathBuf::from(value)),
            other => return Err(format!("unknown shortcut option {other:?}")),
        }
        index += 1;
    }
    let profile = play_profile(app_id, profile)?;
    let loaded = CompatibilityProfile::load(&profile).map_err(|error| error.to_string())?;
    if loaded.app_id != app_id {
        return Err(format!("profile AppID {} does not match requested AppID {app_id}", loaded.app_id));
    }
    let game_name = shortcut_game_name(app_id, &profile, loaded.steam_install_dir.as_deref())?;
    let output = match output {
        Some(path) => path,
        None => env::current_dir().map_err(|error| error.to_string())?
            .join("shortcuts").join(format!("{game_name}.command")),
    };
    if !output.is_absolute() || output.extension().is_none_or(|ext| ext != "command") {
        return Err("shortcut output must be an absolute .command path".into());
    }
    if output.exists() {
        return Err(format!("shortcut already exists: {}", output.display()));
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("cannot create shortcut directory {}: {error}", parent.display()))?;
    }
    let executable = env::current_exe().map_err(|error| error.to_string())?;
    let body = format!("#!/bin/sh\nexec {} play {} --profile {}\n", shell_quote(&executable.to_string_lossy()), app_id, shell_quote(&profile.canonicalize().map_err(|error| error.to_string())?.to_string_lossy()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&output)
        .map_err(|error| format!("cannot create shortcut {}: {error}", output.display()))?;
    file.write_all(body.as_bytes()).map_err(|error| error.to_string())?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&output, fs::Permissions::from_mode(0o755)).map_err(|error| error.to_string())?;
    }
    println!("Created AppID {app_id} shortcut: {}", output.display());
    Ok(())
}

fn launch_command(args: &[String]) -> Result<(), String> {
    let app_id: u32 = args
        .first()
        .ok_or_else(|| {
            "usage: macroton launch APPID --profile PATH [--steam-root PATH] [--runtime-root PATH] [--data-root PATH] [--max-rss-mib N] [--hard-rss-mib N | --no-resource-guard] [--allow-ephemeral-data-root]"
                .to_owned()
        })?
        .parse()
        .map_err(|_| "APPID must be an unsigned integer".to_owned())?;
    let mut steam_root = steam::default_steam_root()
        .ok_or_else(|| "HOME is not set; pass --steam-root PATH".to_owned())?;
    let mut runtime_root = default_runtime_root()?;
    let mut data_root = default_data_root()?;
    let mut profile_path = None;
    // Wine Steam's CEF client routinely peaks around 2.8--3.4 GiB on
    // Apple Silicon before the game process appears. Keep a bounded default
    // that does not kill the client during bootstrap; callers can still set a
    // lower explicit limit when testing a constrained host.
    let mut max_rss_mib = Some(4096_u64);
    let mut hard_rss_mib = None;
    let mut allow_ephemeral_data_root = false;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        if option == "--no-resource-guard" {
            max_rss_mib = None;
            continue;
        }
        if option == "--allow-ephemeral-data-root" {
            allow_ephemeral_data_root = true;
            continue;
        }
        let value = args
            .get(index)
            .ok_or_else(|| format!("{option} requires a path"))?;
        match option {
            "--steam-root" => steam_root = PathBuf::from(value),
            "--runtime-root" => runtime_root = PathBuf::from(value),
            "--data-root" => data_root = PathBuf::from(value),
            "--profile" => profile_path = Some(PathBuf::from(value)),
            "--max-rss-mib" => max_rss_mib = Some(parse_positive_u64(value, option)?),
            "--hard-rss-mib" => hard_rss_mib = Some(parse_positive_u64(value, option)?),
            other => return Err(format!("unknown launch option {other:?}")),
        }
        index += 1;
    }
    if hard_rss_mib.is_some_and(|hard| max_rss_mib.is_none_or(|soft| hard < soft)) {
        return Err("--hard-rss-mib requires an active resource guard and must be at least --max-rss-mib".to_owned());
    }
    let profile_path = profile_path.ok_or_else(|| "--profile PATH is required".to_owned())?;
    let profile = CompatibilityProfile::load(&profile_path).map_err(|error| error.to_string())?;
    let games = steam::scan(&steam_root).map_err(|error| error.to_string())?;
    // GUI Steam acquisition is isolated per AppID. Use its Windows Steam
    // library when the host macOS library has no copy of this title.
    let prefix_steam_root = data_root
        .join("games")
        .join(app_id.to_string())
        .join("prefix/drive_c/Program Files (x86)/Steam");
    let host_game = games.iter().find(|game| game.app_id == app_id).cloned();
    let mut plan = host_game
        .as_ref()
        .map(|game| plan::build(game, &profile, &runtime_root));
    // A stale host-library AppState for the same AppID must not win over a
    // complete isolated Windows Steam install. For the explicit installed-
    // tree source, prefer the prefix's manifest whenever it exists.
    if profile.windows_payload_root == macroton::profile::WindowsPayloadRoot::SteamInstalledTree
        && let Some(prefix_game) = steam::scan(&prefix_steam_root)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|game| game.app_id == app_id)
    {
        let prefix_plan = plan::build(&prefix_game, &profile, &runtime_root);
        plan = Some(prefix_plan);
    }
    let plan =
        plan.ok_or_else(|| format!("AppID {app_id} is not installed in the discovered libraries"))?;
    if !plan.is_ready() {
        return Err(format!("launch is blocked: {}", plan.blockers.join("; ")));
    }
    if plan.launch_mode == macroton::profile::LaunchMode::Steam {
        let fingerprint = plan.runtime_fingerprint_sha256.as_deref()
            .ok_or("ready Steam launch plan is missing its runtime fingerprint")?;
        steam_prefix_status::validate_expected_runtime(app_id,
            &data_root.join("games").join(app_id.to_string()).join("prefix"),
            &plan.runtime_id, fingerprint)
            .map_err(|error| format!("selected profile is incompatible with the existing AppID {app_id} Steam prefix: {error}. Check the profile and registered runtime before launching"))?;
        require_persistent_data_root(&data_root, allow_ephemeral_data_root, "Steam game launch")?;
        require_unlocked_console_session()?;
        handoff_other_steam_clients(app_id, &data_root, &runtime_root)?;
        if let Some(warning) = other_live_steam_prefix_warning(app_id, &data_root)? {
            eprintln!("Steam compatibility warning: {warning}");
        }
        if let Some(warning) = steam_port_preflight_warning() {
            let owns_port_candidate = stop::inspect_resources(app_id, &data_root)
                .map(|resources| {
                    resources.processes.iter().any(|process| {
                        matches!(
                            process.category,
                            "steam-client" | "steam-cef" | "wine-service"
                        )
                    })
                })
                .unwrap_or(false);
            println!(
                "Steam compatibility warning: {}",
                format_steam_port_warning_for_scope(&warning, owns_port_candidate)
            );
        }
        // Establish the new request boundary before inspecting stale client
        // state. Otherwise an old `Game process removed` event can make a
        // still-warming client look terminal and trigger an unnecessary cold
        // restart on every retry.
        launch_status::record_launch_boundary(app_id, &data_root)
            .map_err(|error| error.to_string())?;

        // A historical pending launch intent cannot be resolved by Macroton.
        // Still submit the normal Steam request: Steam may present the owner
        // with a Cloud choice, or may have resolved an old intent since the
        // append-only log entry. Never choose a save version or fall back to
        // a direct game executable here.
        if !profile.cloud_files.is_empty()
            && let Ok(cloud) = cloud_status::inspect(app_id, &data_root)
            && cloud.remote_freshness != cloud_status::RemoteFreshness::Unverified
        {
            eprintln!(
                "warning: Steam Cloud remote freshness for AppID {app_id}: {}. Local cache sync does not prove the latest save from another device. Resolve any Steam Cloud choice in Steam; Macroton will not choose a save version.",
                cloud.remote_freshness.as_str()
            );
        }
        if !profile.cloud_files.is_empty()
            && let Ok(cloud) = cloud_status::inspect(app_id, &data_root)
            && !cloud.pending_remote_operations.is_empty()
        {
            eprintln!(
                "warning: Steam Cloud has {} historical pending launch operation(s) for AppID {app_id}; submitting a normal Steam launch so Steam can show any required owner confirmation. Do not choose a save version automatically. Read-only CEF check: node scripts/resolve-steam-cloud-dialog.js --webhelper-log \"{}\"",
                cloud.pending_remote_operations.len(),
                data_root
                    .join("games")
                    .join(app_id.to_string())
                    .join("prefix/drive_c/Program Files (x86)/Steam/logs/webhelper.txt")
                    .display()
            );
        }

        // Steam can leave a Wine client/CEF process alive after its game
        // launch event, while the actual game process has already vanished.
        // Treat that state as recoverable: stop only this AppID's exact
        // prefix-owned processes, retain the authenticated session marker,
        // then let execute() submit a fresh Steam launch request.
        let launch_state = launch_status::inspect(app_id, &data_root)
            .ok()
            .map(|status| status.state);
        if let Ok(resources) = stop::inspect_resources(app_id, &data_root) {
            let has_client = resources
                .processes
                .iter()
                .any(|process| matches!(process.category, "steam-client" | "steam-cef"));
            let has_game = resources
                .processes
                .iter()
                .any(|process| process.category == "windows-game");
            if has_game {
                let game = existing_windows_game_command(&resources)
                    .unwrap_or("the existing Windows game process");
                return Err(format!(
                    "AppID {app_id} already has a running Windows game process ({game}); use that session or stop the exact prefix before launching again"
                ));
            }
            // A resident client with no game is often still warming up. Only
            // restart it after an explicit lifecycle terminal event; otherwise
            // reuse the live client so a retry can submit `-applaunch` instead
            // of repeatedly returning to a cold CEF startup.
            if has_client
                && !has_game
                && matches!(
                    launch_state,
                    Some(macroton::launch_status::LaunchState::Exited)
                        | Some(macroton::launch_status::LaunchState::Failed)
                )
            {
                println!(
                    "no game process in AppID {app_id} prefix; restarting its Wine Steam client"
                );
                stop::stop_processes_preserve_session(app_id, &data_root, &runtime_root)
                    .map_err(|error| error.to_string())?;
            }
        }
    } else {
        eprintln!(
            "warning: this profile uses direct EXE launch and bypasses Steam; Steam-protected games may show `Steam is required`. Use a launch=steam profile for the supported path."
        );
    }
    // Protect cold Steam/CEF startup as well as the post-handoff game phase.
    // execute() may spend up to 90 seconds warming the client before it
    // returns, so starting the guard only afterward leaves the largest
    // memory spike uncontained. The prelaunch guard is unattributed evidence;
    // the normal attributed guard replaces it after a successful handoff.
    let prelaunch_guard = if plan.launch_mode == macroton::profile::LaunchMode::Steam {
        max_rss_mib
            .map(|limit| {
                spawn_resource_guard_with_idle(
                    app_id,
                    &data_root,
                    &runtime_root,
                    &plan.runtime_id,
                    limit,
                    hard_rss_mib,
                    Some(300),
                    None,
                    profile
                        .steam_launcher_bypass
                        .as_deref()
                        .and_then(|path| path.to_str())
                        .or_else(|| profile.executable.to_str()),
                    false,
                    Some(120),
                )
            })
            .transpose()?
    } else {
        None
    };
    let result = execute::execute(&plan, &data_root).map_err(|error| {
        if let Some((pid, log)) = &prelaunch_guard {
            format!("{error}; prelaunch resource guard PID {pid}, log {}", log.display())
        } else {
            error.to_string()
        }
    })?;
    let resource_guard = if result.launch_mode == macroton::profile::LaunchMode::Steam {
        max_rss_mib
            .map(|limit| {
                spawn_resource_guard(
                    app_id,
                    &data_root,
                    &runtime_root,
                    &result.runtime_id,
                    limit,
                    hard_rss_mib,
                    None,
                    Some(&resource_guard_identity(
                        &plan,
                        &result.runtime_fingerprint_sha256,
                    )),
                    // Launcher-bypass profiles execute the configured game
                    // image under the bypass filename. Pass that effective
                    // basename to the guard so its exact-game evidence and
                    // handoff timeout match the process Steam actually
                    // creates (for example HOMM3Launcher.exe).
                    profile
                        .steam_launcher_bypass
                        .as_deref()
                        .and_then(|path| path.to_str())
                        .or_else(|| profile.executable.to_str()),
                )
            })
            .transpose()?
    } else {
        None
    };
    match result.launch_mode {
        macroton::profile::LaunchMode::Direct => println!(
            "game process exited with {}; result: {}",
            result
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_owned()),
            result.result_path.display()
        ),
        macroton::profile::LaunchMode::Steam => {
            println!(
                "Steam launch request submitted; runtime exit is not awaited; result: {}",
                result.result_path.display()
            );
            // Steam may create a Cloud launch transaction just after the
            // handoff returns. Surface an already-visible waiting state so a
            // user does not mistake a resident Steam client for a game start;
            // the authoritative follow-up remains `steam launch-status`.
            if let Ok(status) = launch_status::inspect(app_id, &data_root) {
                if status.state == launch_status::LaunchState::WaitingCloud {
                    println!(
                        "Steam Cloud: waiting for a pending launch confirmation; resolve it in the isolated Windows Steam UI, then retry"
                    );
                } else if status.state == launch_status::LaunchState::WaitingUser {
                    println!(
                        "Steam: waiting for user confirmation in the isolated Windows Steam UI; complete it before retrying"
                    );
                }
            }
            if let Some((pid, log)) = resource_guard {
                println!(
                    "resource guard: PID {pid}, limit {} MiB, log {}",
                    max_rss_mib.unwrap_or_default(),
                    log.display()
                );
            } else {
                println!("resource guard: disabled by explicit launch option");
            }
            println!(
                "handoff request log: {}",
                data_root
                    .join("games")
                    .join(app_id.to_string())
                    .join("steam-handoff.log")
                    .display()
            );
        }
    }
    if result.launch_mode == macroton::profile::LaunchMode::Steam {
        record_cloud_handoff(app_id, &data_root, &profile)?;
    }
    Ok(())
}

fn existing_windows_game_command(resources: &stop::PrefixResourceUsage) -> Option<&str> {
    resources
        .processes
        .iter()
        .find(|process| process.category == "windows-game")
        .map(|process| process.command.as_str())
}

#[allow(clippy::too_many_arguments)]
fn spawn_resource_guard(
    app_id: u32,
    data_root: &Path,
    runtime_root: &Path,
    runtime_id: &str,
    max_rss_mib: u64,
    hard_rss_mib: Option<u64>,
    max_runtime_seconds: Option<u64>,
    provenance: Option<&steam_session::Identity>,
    game_executable: Option<&str>,
) -> Result<(u32, PathBuf), String> {
    spawn_resource_guard_with_idle(
        app_id,
        data_root,
        runtime_root,
        runtime_id,
        max_rss_mib,
        hard_rss_mib,
        max_runtime_seconds,
        provenance,
        game_executable,
        false,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_resource_guard_with_idle(
    app_id: u32,
    data_root: &Path,
    runtime_root: &Path,
    runtime_id: &str,
    max_rss_mib: u64,
    hard_rss_mib: Option<u64>,
    max_runtime_seconds: Option<u64>,
    provenance: Option<&steam_session::Identity>,
    game_executable: Option<&str>,
    keep_idle: bool,
    startup_grace_seconds: Option<u64>,
) -> Result<(u32, PathBuf), String> {
    let executable = env::current_exe().map_err(|error| {
        format!("cannot resolve Macroton executable for resource guard: {error}")
    })?;
    let log = data_root
        .join("games")
        .join(app_id.to_string())
        .join("resource-guard.log");
    if let Some(parent) = log.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create resource guard directory {}: {error}",
                parent.display()
            )
        })?;
    }

    // A guard is tied to a bounded session. Reusing one left behind by a
    // previous launch can make the new session inherit the old guard's
    // elapsed-time deadline and get terminated immediately. Replace every
    // matching guard before starting the new session monitor.
    for pid in existing_resource_guard_pids(app_id, data_root, runtime_id) {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|error| format!("cannot open resource guard log {}: {error}", log.display()))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("cannot clone resource guard log handle: {error}"))?;
    let mut command = Command::new(executable);
    command
        .args(["resource", "guard", &app_id.to_string()])
        .arg("--data-root")
        .arg(data_root)
        .arg("--runtime-root")
        .arg(runtime_root)
        .args(["--runtime-id", runtime_id])
        .args(["--max-rss-mib", &max_rss_mib.to_string()]);
    if let Some(hard) = hard_rss_mib {
        command
            .args(["--hard-rss-mib", &hard.to_string()])
            .args(["--interval-seconds", "1"]);
    }
    if let Some(seconds) = startup_grace_seconds {
        command.args(["--startup-grace-seconds", &seconds.to_string()]);
    }
    if keep_idle {
        command.arg("--keep-idle");
    }
    if let Some(game_executable) = game_executable {
        command.args(["--game-executable", game_executable]);
    }
    if let Some(seconds) = max_runtime_seconds {
        command.args(["--max-runtime-seconds", &seconds.to_string()]);
    }
    if let Some(identity) = provenance {
        let prefix = data_root
            .join("games")
            .join(app_id.to_string())
            .join("prefix");
        steam_session::validate(&prefix, identity).map_err(|error| {
            format!("cannot bind resource guard to active Steam session: {error}")
        })?;
        let marker_sha256 = sha256::to_hex(
            &sha256::digest_file(&prefix.join(steam_session::MARKER)).map_err(|error| {
                format!("cannot hash active Steam session marker for resource guard: {error}")
            })?,
        );
        command
            .args(["--profile-sha256", &identity.profile_sha256])
            .args([
                "--runtime-fingerprint-sha256",
                &identity.runtime_fingerprint_sha256,
            ])
            .args(["--steam-client-mode", &identity.steam_client_mode])
            .args(["--renderer", &identity.renderer])
            .args(["--graphics-api", &identity.graphics_api])
            .args(["--steam-session-sha256", &marker_sha256]);
    }
    #[cfg(unix)]
    command.process_group(0);
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| format!("cannot start prefix resource guard: {error}"))?;
    Ok((child.id(), log))
}

fn existing_resource_guard_pids(app_id: u32, data_root: &Path, runtime_id: &str) -> Vec<u32> {
    let output = match Command::new("ps")
        .args(["-axo", "pid=,command="])
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    let app_token = format!("resource guard {app_id} ");
    let data_token = data_root.to_string_lossy();
    let runtime_token = format!("--runtime-id {runtime_id}");
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let (pid, command) = trimmed.split_once(' ')?;
            let pid = pid.trim().parse::<u32>().ok()?;
            if command.contains("macroton resource guard")
                && command.contains(&app_token)
                && command.contains(data_token.as_ref())
                && command.contains(&runtime_token)
            {
                Some(pid)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
fn find_existing_resource_guard_pid(
    output: &[u8],
    app_id: u32,
    data_root: &Path,
    runtime_id: &str,
    profile_sha256: Option<&str>,
) -> Option<u32> {
    let app_token = format!("resource guard {app_id} ");
    let data_token = data_root.to_string_lossy();
    let runtime_token = format!("--runtime-id {runtime_id}");
    let profile_token = profile_sha256.map(|sha| format!("--profile-sha256 {sha}"));
    output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let (pid, command) = trimmed.split_once(' ')?;
            let pid = pid.trim().parse::<u32>().ok()?;
            if !command.contains("macroton resource guard")
                || !command.contains(&app_token)
                || !command.contains(data_token.as_ref())
                || !command.contains(&runtime_token)
                || profile_token
                    .as_deref()
                    .is_some_and(|token| !command.contains(token))
            {
                return None;
            }
            Some(pid)
        })
        .next()
}

fn resource_guard_identity(
    plan: &plan::LaunchPlan,
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

fn default_runtime_root() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or_else(|| {
        "HOME is not set; pass --runtime-root PATH when planning a launch".to_owned()
    })?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Macroton")
        .join("runtimes"))
}

fn default_data_root() -> Result<PathBuf, String> {
    let home = env::var_os("HOME")
        .ok_or_else(|| "HOME is not set; pass --data-root PATH when launching".to_owned())?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Macroton"))
}

fn require_persistent_data_root(
    data_root: &Path,
    allow_ephemeral: bool,
    operation: &str,
) -> Result<(), String> {
    if allow_ephemeral {
        return Ok(());
    }
    if !data_root.is_absolute() {
        return Err(format!(
            "{operation} requires an absolute persistent --data-root; temporary test roots require --allow-ephemeral-data-root"
        ));
    }
    let known_ephemeral_roots = [
        Path::new("/tmp"),
        Path::new("/private/tmp"),
        Path::new("/var/tmp"),
        Path::new("/private/var/tmp"),
        Path::new("/var/folders"),
        Path::new("/private/var/folders"),
    ];
    let configured_temp = env::temp_dir();
    if known_ephemeral_roots
        .iter()
        .any(|root| data_root.starts_with(root))
        || data_root.starts_with(&configured_temp)
    {
        return Err(format!(
            "refusing {operation} with ephemeral data root {}; Steam login, userdata, Cloud cache and per-AppID prefixes would be lost when the temporary directory is cleaned; use the default Application Support root or pass --allow-ephemeral-data-root only for disposable diagnostics",
            data_root.display()
        ));
    }
    Ok(())
}

fn format_binary_kind(kind: &BinaryKind) -> String {
    match kind {
        BinaryKind::PortableExecutable => "pe".to_owned(),
        BinaryKind::Unknown => "unknown".to_owned(),
        BinaryKind::MachO(architectures) => format!(
            "mach-o:{}",
            architectures
                .iter()
                .map(|architecture| match architecture {
                    Architecture::I386 => "i386".to_owned(),
                    Architecture::X86_64 => "x86_64".to_owned(),
                    Architecture::Arm64 => "arm64".to_owned(),
                    Architecture::Other(cpu) => format!("cpu-{cpu:#x}"),
                })
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn print_games_json(games: &[steam::SteamGame]) {
    println!("[");
    for (index, game) in games.iter().enumerate() {
        println!("  {{");
        println!("    \"appid\": {},", game.app_id);
        println!("    \"name\": \"{}\",", json_escape(&game.name));
        println!(
            "    \"classification\": \"{}\",",
            game.classification.as_str()
        );
        println!(
            "    \"install_dir\": \"{}\",",
            json_escape(&game.install_dir.to_string_lossy())
        );
        println!("    \"observed_binaries\": {}", game.binaries.len());
        println!("  }}{}", if index + 1 == games.len() { "" } else { "," });
    }
    println!("]");
}

fn json_escape(input: &str) -> String {
    let mut output = String::new();
    for character in input.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => output.push(character),
        }
    }
    output
}

fn print_help() {
    println!(
        "Macroton 0.1\n\
         \n\
         Usage:\n\
           macroton scan [--steam-root PATH] [--json]\n\
           macroton inspect PATH\n\
           macroton inspect-pe PATH [--json]\n\
           macroton profile validate PATH\n\
           macroton evidence snapshot APPID --profile PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH]\n\
           macroton evidence bundle APPID --profile PATH --focus PATH --motion PATH --interaction PATH --audio PATH --independence PATH --exit PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH]\n\
           macroton evidence bundle-status APPID --profile PATH --evidence-dir PATH [--data-root PATH] [--runtime-root PATH]\n\
           macroton evidence matrix-status --profiles-dir ABSOLUTE_PATH --evidence-root ABSOLUTE_PATH [--failure-dir ABSOLUTE_PATH --failure-source APPID=ABSOLUTE_PATH] [--data-root PATH] [--app-data-root APPID=PATH] [--runtime-root PATH]\n\
           macroton evidence record-failure APPID --profile PATH --source ABSOLUTE_PATH --gate GATE --code CODE --output ABSOLUTE_PATH\n\
           macroton evidence failure-status APPID --profile PATH --failure ABSOLUTE_PATH --source ABSOLUTE_PATH\n\
           macroton evidence record-review APPID --profile PATH --bundle ABSOLUTE_PATH --visual pass|fail --audio pass|fail --input pass|fail --gameplay pass|fail --save-reload pass|fail --sustained-playability pass|fail --output ABSOLUTE_PATH\n\
           macroton evidence acceptance-status APPID --profile PATH --bundle ABSOLUTE_PATH --review ABSOLUTE_PATH\n\
           macroton evidence verify-audio APPID --profile PATH --probe PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH]\n\
           macroton evidence verify-focus-stream APPID --profile PATH --probe PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--min-frames N] [--min-duration-seconds N] [--max-gap-ms N] [--black-luma N] [--max-black-frames N]\n\
           macroton evidence verify-motion-rate APPID --profile PATH --probe PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--min-duration-seconds N] [--min-changed-rate N] [--max-unchanged-ms N]\n\
           macroton evidence verify-gpu-capture APPID --profile PATH --capture ABSOLUTE_PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--stability-ms N]\n\
           macroton evidence verify-interaction APPID --profile PATH --probe PATH --capture-dir PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--min-changed-ratio N]\n\
           macroton evidence verify-independence APPID --profile PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH]\n\
           macroton evidence watch-exit APPID --profile PATH --output ABSOLUTE_PATH [--data-root PATH] [--runtime-root PATH] [--min-runtime-seconds N] [--timeout-seconds N] [--interval-seconds N]\n\
           macroton plan APPID --profile PATH [--steam-root PATH] [--runtime-root PATH]\n\
           macroton play APPID [--profile PATH] [--data-root PATH] [--runtime-root PATH]\n\
           macroton shortcut APPID [--output ABSOLUTE_PATH.command] [--profile PATH]\n\
           macroton launch APPID --profile PATH [--steam-root PATH] [--runtime-root PATH] [--data-root PATH] [--max-rss-mib N] [--hard-rss-mib N | --no-resource-guard] [--allow-ephemeral-data-root]\n\
           macroton stop APPID [--runtime-root PATH] [--data-root PATH] [--preserve-session]\n\
           macroton resource status APPID [--data-root PATH] [--max-rss-mib N] [--json]\n\
           macroton resource status-all [--data-root PATH] [--max-total-rss-mib N] [--json]\n\
           macroton resource guard APPID [--data-root PATH] [--runtime-root PATH] [--runtime-id ID] [--max-rss-mib N] [--hard-rss-mib N] [--max-runtime-seconds N] [--keep-idle]\n\
           macroton resource compare APPID --standard PATH --low-memory PATH --standard-profile PATH --low-memory-profile PATH --phase <client|game> [--min-seconds N] [--output PATH]\n\
           macroton steamcmd login APPID --tool-root PATH --runtime-id ID [--runtime-root PATH] [--data-root PATH] [--max-rss-mib N]\n\
           macroton steamcmd acquire APPID --profile PATH --tool-root PATH --runtime-id ID [--account ACCOUNT_NAME] [--runtime-root PATH] [--data-root PATH] [--max-rss-mib N]\n\
           macroton steamcmd status APPID --profile PATH [--data-root PATH] [--json]\n\
           macroton steamcmd validate APPID --profile PATH [--data-root PATH] [--json]\n\
           macroton steamcmd plan APPID --profile PATH [--data-root PATH] [--runtime-root PATH]\n\
           macroton steamcmd launch APPID --profile PATH [--data-root PATH] [--runtime-root PATH] [--max-rss-mib N]\n\
           macroton steam bootstrap APPID --profile PATH --installer PATH --installer-sha256 SHA256 [--runtime-root PATH] [--data-root PATH]\n\
           macroton steam appinfo APPID [--profile PATH] [--data-root PATH | --steam-root PATH]\n\
           macroton steam client-status APPID [--data-root PATH] [--json]\n\
           macroton steam client-wait APPID [--data-root PATH] [--timeout-seconds N] [--interval-seconds N]\n\
           macroton steam cloud-status APPID [--profile PATH] [--data-root PATH | --steam-root PATH] [--json]\n\
           macroton steam cloud-wait APPID --profile PATH [--data-root PATH | --steam-root PATH] [--timeout-seconds N] [--interval-seconds N]\n\
           macroton steam cloud-roundtrip <status|begin|confirm-upload|quarantine|verify-download|restore|abandon> APPID --profile PATH [--data-root PATH] [--confirm]\n\
           macroton steam connection-status APPID [--data-root PATH | --steam-root PATH] [--json]\n\
           macroton steam install APPID --profile PATH [--runtime-root PATH] [--data-root PATH] [--max-rss-mib N] [--allow-ephemeral-data-root]\n\
           macroton steam install-status APPID [--data-root PATH | --steam-root PATH] [--json]\n\
           macroton steam install-wait APPID [--data-root PATH] [--timeout-seconds N] [--request-timeout-seconds N] [--interval-seconds N]\n\
           macroton steam inventory --steam-root PATH [--data-root PATH] [--eligibility-dir PATH] [--min-playtime MINUTES] [--limit COUNT] [--with-appinfo] [--candidates-only] [--verified-win32-only] [--2d-only] [--json]\n\
           macroton steam target-status APPID --native-steam-root PATH --windows-steam-root PATH [--eligibility-review PATH] [--json]\n\
           macroton steam launch-status APPID [--data-root PATH | --steam-root PATH] [--json]\n\
           macroton steam library-status APPID [--data-root PATH | --steam-root PATH]\n\
           macroton steam lifecycle-status APPID [--data-root PATH] [--json]\n\
           macroton steam login APPID --profile PATH [--runtime-root PATH] [--data-root PATH] [--max-rss-mib N] [--force] [--allow-ephemeral-data-root]\n\
           macroton steam login-wait APPID [--data-root PATH] [--timeout-seconds N] [--interval-seconds N]\n\
           macroton steam prefix-status APPID [--data-root PATH] [--json]\n\
           macroton steam prerequisite-status APPID [--data-root PATH] [--json]\n\
           macroton steam rebind-runtime APPID --profile PATH --from-runtime-fingerprint SHA256 [--runtime-root PATH] [--data-root PATH]\n\
           macroton steam session-status [--json]\n\
           macroton steam shared-client-status RESIDENT_APPID TARGET_APPID [--resident-profile PATH] [--target-profile PATH]\n\
           macroton steam shared-client-install RESIDENT_APPID TARGET_APPID [--resident-profile PATH] [--target-profile PATH]\n\
           macroton steam shared-client-launch RESIDENT_APPID TARGET_APPID [--resident-profile PATH] [--target-profile PATH]\n\
           macroton steam shared-client-finish RESIDENT_APPID TARGET_APPID [--data-root PATH]\n\
           macroton steam shared-client-abandon RESIDENT_APPID TARGET_APPID [--data-root PATH] [--runtime-root PATH]\n\
           macroton steam trim-client APPID --profile PATH [--data-root PATH] [--runtime-root PATH]\n\
           macroton runtime discover\n\
           macroton runtime audit ENTRYPOINT\n\
           macroton runtime release-audit MANIFEST --runtime-root PATH [--json]\n\
           macroton runtime profile-release-audit PROFILE MANIFEST --runtime-root PATH --windows-steam-root PATH [--json]\n\
           macroton runtime probe-win32 ENTRYPOINT\n\
           macroton runtime register-external ID ENTRYPOINT [--runtime-root PATH]\n\
           macroton runtime verify MANIFEST ARTIFACT\n\
           macroton runtime install MANIFEST ARTIFACT [--runtime-root PATH]\n\
           macroton runtime list [--runtime-root PATH]\n\
         \n\
         Runtime installation and game launch are mutating commands."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contributed_shortcut_catalog_supplies_portable_display_name() {
        assert_eq!(shortcut_game_name(400, Path::new("compatibility/400.conf"), Some("Wrong Fallback")).unwrap(), "Portal");
    }

    #[test]
    fn inventory_selection_label_preserves_combined_win32_and_2d_gate() {
        assert_eq!(inventory_selection_label(true, true, true), "verified-win32-2d-only");
        assert_eq!(inventory_selection_label(true, false, true), "2d-only");
        assert_eq!(inventory_selection_label(false, true, false), "verified-win32-only");
    }

    #[test]
    fn inventory_finds_only_matching_exact_appid_prefix_pe_evidence() {
        let root = std::env::temp_dir().join(format!(
            "macroton-inventory-prefix-{}-{}",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        let primary = root.join("primary/Steam");
        fs::create_dir_all(primary.join("steamapps")).unwrap();
        let data = root.join("data");
        for app_id in [42, 43, 44] {
            let steam = data
                .join("games")
                .join(app_id.to_string())
                .join("prefix/drive_c/Program Files (x86)/Steam");
            fs::create_dir_all(steam.join("steamapps/common/Fixture")).unwrap();
            // AppID 43's folder contains a mismatched manifest. AppID 44 has
            // a PE file but no completed install checkpoint. Neither may
            // lend its Windows binary to a verified-installed inventory entry.
            let manifest = if app_id == 44 {
                r#""AppState" { "appid" "44" "name" "Fixture" "installdir" "Fixture" "buildid" "1" "SizeOnDisk" "0" }"#
            } else {
                r#""AppState" { "appid" "42" "name" "Fixture" "installdir" "Fixture" "buildid" "1" "TargetBuildID" "0" "SizeOnDisk" "100" "BytesToDownload" "0" "BytesDownloaded" "0" }"#
            };
            fs::write(
                steam.join("steamapps").join(format!("appmanifest_{app_id}.acf")),
                manifest,
            )
            .unwrap();
            let mut pe32 = vec![0_u8; 0x80];
            pe32[0..2].copy_from_slice(b"MZ");
            pe32[0x3c..0x40].copy_from_slice(&0x40_u32.to_le_bytes());
            pe32[0x40..0x44].copy_from_slice(b"PE\0\0");
            pe32[0x44..0x46].copy_from_slice(&0x014c_u16.to_le_bytes());
            fs::write(steam.join("steamapps/common/Fixture/game.exe"), pe32).unwrap();
        }
        let installed = inventory_installed_evidence(&primary, Some(&data), [42, 43, 44].into_iter())
            .unwrap();
        assert_eq!(installed.len(), 1);
        let game = installed.get(&42).unwrap();
        assert_eq!(game.classification, steam::Classification::WindowsPayload);
        assert_eq!(game.pe_binaries[0].1, PeArchitecture::I386);
        assert!(!installed.contains_key(&43));
        assert!(!installed.contains_key(&44));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_artifact_parser_binds_identity_and_rejects_wrong_kind() {
        let root = std::env::temp_dir().join(format!(
            "macroton-bundle-artifact-{}-{}",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("focus.json");
        let digest = "ab".repeat(32);
        fs::write(
            &path,
            format!(
                "{{\n  \"schema_version\": 1,\n  \"kind\": \"runtime-focused-temporal-sample\",\n  \"appid\": 42,\n  \"profile_sha256\": \"{digest}\",\n  \"runtime_id\": \"wine-1\",\n  \"runtime_fingerprint_sha256\": \"{digest}\",\n  \"steam_session_sha256\": \"{digest}\",\n  \"steam_client_mode\": \"standard\",\n  \"renderer\": \"wine-default\",\n  \"graphics_api\": \"d3d9\",\n  \"tracked_game_pid\": 9,\n  \"status\": \"focused-temporal-sample-pass\"\n}}\n"
            ),
        )
        .unwrap();
        let artifact = read_bundle_artifact(
            &path,
            "runtime-focused-temporal-sample",
            "focused-temporal-sample-pass",
        )
        .unwrap();
        assert_eq!(artifact.app_id, 42);
        assert_eq!(artifact.tracked_game_pid, 9);
        assert_eq!(artifact.profile_sha256, digest);
        assert!(
            read_bundle_artifact(&path, "runtime-audio-presence", "pcm-presence-pass")
                .unwrap_err()
                .contains("expected")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gameplay_review_binds_to_automated_bundle_and_rejects_contradictory_status() {
        let root = std::env::temp_dir().join(format!(
            "macroton-gameplay-review-{}-{}",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let profile_sha256 = "ab".repeat(32);
        let bundle_path = root.join("bundle.json");
        fs::write(
            &bundle_path,
            format!(
                "{{\n  \"schema_version\": 1,\n  \"kind\": \"automated-runtime-evidence-bundle\",\n  \"appid\": 42,\n  \"profile_sha256\": \"{profile_sha256}\",\n  \"runtime_id\": \"wine-1\",\n  \"runtime_fingerprint_sha256\": \"{profile_sha256}\",\n  \"steam_session_sha256\": \"{profile_sha256}\",\n  \"steam_client_mode\": \"standard\",\n  \"renderer\": \"wined3d-opengl\",\n  \"graphics_api\": \"opengl\",\n  \"tracked_game_pid\": 9,\n  \"focus_sha256\": \"{profile_sha256}\",\n  \"motion_sha256\": \"{profile_sha256}\",\n  \"interaction_sha256\": \"{profile_sha256}\",\n  \"audio_sha256\": \"{profile_sha256}\",\n  \"independence_sha256\": \"{profile_sha256}\",\n  \"exit_sha256\": \"{profile_sha256}\",\n  \"cloud_transaction_id\": 1,\n  \"cloud_phase\": \"complete\",\n  \"exact_prefix_residual_processes\": 0,\n  \"exact_prefix_residual_rss_kib\": 0,\n  \"status\": \"automated-runtime-evidence-bundle-pass\"\n}}\n"
            ),
        )
        .unwrap();
        let bundle = read_automated_bundle(&bundle_path).unwrap();
        assert_eq!(bundle.app_id, 42);
        assert_eq!(bundle.profile_sha256, profile_sha256);

        let review_path = root.join("review.json");
        let review_json = |audio: bool, status: &str| {
            format!(
                "{{\n  \"schema_version\": 1,\n  \"kind\": \"human-gameplay-review\",\n  \"appid\": 42,\n  \"profile_sha256\": \"{}\",\n  \"automated_bundle_sha256\": \"{}\",\n  \"visual_correct\": true,\n  \"audio_acceptable\": {audio},\n  \"semantic_input_correct\": true,\n  \"gameplay_correct\": true,\n  \"save_reload_pass\": true,\n  \"sustained_playability\": true,\n  \"status\": \"{status}\"\n}}\n",
                bundle.profile_sha256, bundle.sha256
            )
        };
        fs::write(
            &review_path,
            review_json(false, "human-gameplay-review-fail"),
        )
        .unwrap();
        let review = read_human_gameplay_review(&review_path).unwrap();
        assert!(!review.all_pass());
        assert_eq!(review.failed_gates(), vec!["audible-quality"]);

        fs::write(
            &review_path,
            review_json(false, "human-gameplay-review-pass"),
        )
        .unwrap();
        assert!(
            read_human_gameplay_review(&review_path)
                .unwrap_err()
                .contains("contradicts")
        );
        assert_eq!(parse_review_verdict("pass"), Ok(true));
        assert_eq!(parse_review_verdict("fail"), Ok(false));
        assert!(parse_review_verdict("maybe").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automated_bundle_accepts_only_coherent_unsupported_cloud_policy() {
        let root = std::env::temp_dir().join(format!(
            "macroton-unsupported-cloud-bundle-{}-{}",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("bundle.json");
        let digest = "ab".repeat(32);
        let record = |transaction_id: u64, phase: &str| {
            format!(
                "{{\n  \"schema_version\": 1,\n  \"kind\": \"automated-runtime-evidence-bundle\",\n  \"appid\": 42,\n  \"profile_sha256\": \"{digest}\",\n  \"runtime_id\": \"wine-1\",\n  \"runtime_fingerprint_sha256\": \"{digest}\",\n  \"steam_session_sha256\": \"{digest}\",\n  \"steam_client_mode\": \"standard\",\n  \"renderer\": \"wine-default\",\n  \"graphics_api\": \"d3d9\",\n  \"steam_cloud_policy\": \"unsupported\",\n  \"tracked_game_pid\": 9,\n  \"focus_sha256\": \"{digest}\",\n  \"motion_sha256\": \"{digest}\",\n  \"interaction_sha256\": \"{digest}\",\n  \"audio_sha256\": \"{digest}\",\n  \"independence_sha256\": \"{digest}\",\n  \"exit_sha256\": \"{digest}\",\n  \"cloud_transaction_id\": {transaction_id},\n  \"cloud_phase\": \"{phase}\",\n  \"exact_prefix_residual_processes\": 0,\n  \"exact_prefix_residual_rss_kib\": 0,\n  \"status\": \"automated-runtime-evidence-bundle-pass\"\n}}\n"
            )
        };
        fs::write(&path, record(0, "not-applicable-unsupported")).unwrap();
        read_automated_bundle(&path).unwrap();

        fs::write(&path, record(1, "not-applicable-unsupported")).unwrap();
        assert!(read_automated_bundle(&path).is_err());
        fs::write(&path, record(0, "complete")).unwrap();
        assert!(read_automated_bundle(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gameplay_review_commands_require_exact_bundle_binding_and_all_human_gates() {
        let root = std::env::temp_dir().join(format!(
            "macroton-gameplay-review-command-{}-{}",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let profile_path = Path::new("compatibility/606680.conf");
        let profile = CompatibilityProfile::load(profile_path).unwrap();
        let bundle_path = root.join("bundle.json");
        fs::write(
            &bundle_path,
            format!(
                "{{\n  \"schema_version\": 1,\n  \"kind\": \"automated-runtime-evidence-bundle\",\n  \"appid\": 606680,\n  \"profile_sha256\": \"{digest}\",\n  \"runtime_id\": \"wine-1\",\n  \"runtime_fingerprint_sha256\": \"{digest}\",\n  \"steam_session_sha256\": \"{digest}\",\n  \"steam_client_mode\": \"standard\",\n  \"renderer\": \"wined3d-opengl\",\n  \"graphics_api\": \"opengl\",\n  \"tracked_game_pid\": 9,\n  \"focus_sha256\": \"{digest}\",\n  \"motion_sha256\": \"{digest}\",\n  \"interaction_sha256\": \"{digest}\",\n  \"audio_sha256\": \"{digest}\",\n  \"independence_sha256\": \"{digest}\",\n  \"exit_sha256\": \"{digest}\",\n  \"cloud_transaction_id\": 1,\n  \"cloud_phase\": \"complete\",\n  \"exact_prefix_residual_processes\": 0,\n  \"exact_prefix_residual_rss_kib\": 0,\n  \"status\": \"automated-runtime-evidence-bundle-pass\"\n}}\n",
                digest = profile.fingerprint()
            ),
        )
        .unwrap();
        let review_path = root.join("review.json");
        let review_args = vec![
            "606680".to_owned(),
            "--profile".to_owned(),
            profile_path.display().to_string(),
            "--bundle".to_owned(),
            bundle_path.display().to_string(),
            "--visual".to_owned(),
            "pass".to_owned(),
            "--audio".to_owned(),
            "pass".to_owned(),
            "--input".to_owned(),
            "pass".to_owned(),
            "--gameplay".to_owned(),
            "pass".to_owned(),
            "--save-reload".to_owned(),
            "pass".to_owned(),
            "--sustained-playability".to_owned(),
            "pass".to_owned(),
            "--output".to_owned(),
            review_path.display().to_string(),
        ];
        evidence_record_review_command(&review_args).unwrap();
        assert!(read_human_gameplay_review(&review_path).unwrap().all_pass());

        let acceptance_args = vec![
            "606680".to_owned(),
            "--profile".to_owned(),
            profile_path.display().to_string(),
            "--bundle".to_owned(),
            bundle_path.display().to_string(),
            "--review".to_owned(),
            review_path.display().to_string(),
        ];
        evidence_acceptance_status_command(&acceptance_args).unwrap();

        fs::write(
            &bundle_path,
            format!(
                "{{\n  \"schema_version\": 1,\n  \"kind\": \"automated-runtime-evidence-bundle\",\n  \"appid\": 606680,\n  \"profile_sha256\": \"{digest}\",\n  \"runtime_id\": \"wine-1\",\n  \"runtime_fingerprint_sha256\": \"{digest}\",\n  \"steam_session_sha256\": \"{digest}\",\n  \"steam_client_mode\": \"standard\",\n  \"renderer\": \"wined3d-opengl\",\n  \"graphics_api\": \"opengl\",\n  \"tracked_game_pid\": 9,\n  \"focus_sha256\": \"{digest}\",\n  \"motion_sha256\": \"{digest}\",\n  \"interaction_sha256\": \"{digest}\",\n  \"audio_sha256\": \"{digest}\",\n  \"independence_sha256\": \"{digest}\",\n  \"exit_sha256\": \"{digest}\",\n  \"cloud_transaction_id\": 1,\n  \"cloud_phase\": \"complete\",\n  \"exact_prefix_residual_processes\": 0,\n  \"exact_prefix_residual_rss_kib\": 0,\n  \"status\": \"automated-runtime-evidence-bundle-pass\",\n  \"changed\": true\n}}\n",
                digest = profile.fingerprint()
            ),
        )
        .unwrap();
        assert!(
            evidence_acceptance_status_command(&acceptance_args)
                .unwrap_err()
                .contains("does not bind")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn human_failure_record_is_profile_and_source_bound() {
        let root = std::env::temp_dir().join(format!(
            "macroton-human-failure-{}-{}",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("session-notes.md");
        fs::write(&source, "Observed moving 3D scene at unacceptable speed.\n").unwrap();
        let output = root.join("failure.json");
        let profile_path = Path::new("compatibility/108710.conf");
        let record_args = vec![
            "108710".to_owned(),
            "--profile".to_owned(),
            profile_path.display().to_string(),
            "--source".to_owned(),
            source.display().to_string(),
            "--gate".to_owned(),
            "sustained-playability".to_owned(),
            "--code".to_owned(),
            "unacceptable-3d-frame-rate".to_owned(),
            "--output".to_owned(),
            output.display().to_string(),
        ];
        evidence_record_failure_command(&record_args).unwrap();
        let failure = read_human_gate_failure(&output).unwrap();
        assert_eq!(failure.app_id, 108710);
        assert_eq!(failure.gate, "sustained-playability");
        assert_eq!(failure.code, "unacceptable-3d-frame-rate");

        let status_args = vec![
            "108710".to_owned(),
            "--profile".to_owned(),
            profile_path.display().to_string(),
            "--failure".to_owned(),
            output.display().to_string(),
            "--source".to_owned(),
            source.display().to_string(),
        ];
        evidence_failure_status_command(&status_args).unwrap();
        fs::write(&source, "Changed after failure recording.\n").unwrap();
        assert!(
            evidence_failure_status_command(&status_args)
                .unwrap_err()
                .contains("does not bind")
        );
        assert!(validate_failure_gate("unknown").is_err());
        assert!(validate_failure_code("Invalid Code").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_artifact_set_rejects_cross_session_and_profile_composition() {
        let digest = "ab".repeat(32);
        let artifact = BundleArtifact {
            app_id: 42,
            profile_sha256: digest.clone(),
            runtime_id: "wine-1".to_owned(),
            runtime_fingerprint_sha256: digest.clone(),
            steam_session_sha256: "cd".repeat(32),
            steam_client_mode: "standard".to_owned(),
            renderer: "wine-default".to_owned(),
            graphics_api: "d3d9".to_owned(),
            tracked_game_pid: 9,
            sha256: "ef".repeat(32),
        };
        assert!(
            validate_bundle_artifact_set(
                42,
                &digest,
                "wine-1",
                &digest,
                "standard",
                "wine-default",
                "d3d9",
                &[("focus", &artifact), ("audio", &artifact)],
            )
            .is_ok()
        );
        let mut other_session = artifact.clone();
        other_session.steam_session_sha256 = "01".repeat(32);
        assert!(
            validate_bundle_artifact_set(
                42,
                &digest,
                "wine-1",
                &digest,
                "standard",
                "wine-default",
                "d3d9",
                &[("focus", &artifact), ("audio", &other_session)],
            )
            .unwrap_err()
            .contains("same Steam session")
        );
        assert!(
            validate_bundle_artifact_set(
                42,
                &"02".repeat(32),
                "wine-1",
                &digest,
                "standard",
                "wine-default",
                "d3d9",
                &[("focus", &artifact)],
            )
            .unwrap_err()
            .contains("selected AppID/profile/runtime")
        );
    }

    #[test]
    fn bundle_resume_action_is_fail_closed_and_gate_ordered() {
        let all_valid = [
            ("focus", "valid"),
            ("motion", "valid"),
            ("interaction", "valid"),
            ("audio", "valid"),
            ("independence", "valid"),
            ("exit", "valid"),
        ];
        assert_eq!(
            bundle_status_next_action(
                "runtime-unresolved",
                &all_valid,
                true,
                "complete",
                true,
                true,
            ),
            (
                "runtime-provenance",
                "repair-runtime-unresolved".to_owned(),
                None
            )
        );
        let missing = [("focus", "missing"), ("motion", "invalid")];
        assert_eq!(
            bundle_status_next_action("ready", &missing, false, "unavailable", false, true),
            (
                "live-session-artifacts",
                "capture-one-live-session".to_owned(),
                Some(("focus".to_owned(), "missing".to_owned()))
            )
        );
        let invalid = [("focus", "valid"), ("motion", "invalid")];
        assert_eq!(
            bundle_status_next_action("ready", &invalid, false, "unavailable", false, true).1,
            "recapture-one-live-session"
        );
        assert_eq!(
            bundle_status_next_action("ready", &all_valid, false, "complete", true, true).0,
            "single-session-consistency"
        );
        assert_eq!(
            bundle_status_next_action("ready", &all_valid, true, "syncing", true, true).0,
            "fresh-cloud-roundtrip"
        );
        assert_eq!(
            bundle_status_next_action("ready", &all_valid, true, "complete", false, true).0,
            "normal-exit"
        );
        assert_eq!(
            bundle_status_next_action("ready", &all_valid, true, "complete", true, false).0,
            "exact-prefix-residuals"
        );
        assert_eq!(
            bundle_status_next_action("ready", &all_valid, true, "complete", true, true),
            (
                "none",
                "record-human-review-and-run-acceptance-status".to_owned(),
                None
            )
        );
        assert_eq!(
            bundle_status_next_action(
                "ready",
                &all_valid,
                true,
                "not-applicable-unsupported",
                true,
                true,
            ),
            (
                "none",
                "record-human-review-and-run-acceptance-status".to_owned(),
                None
            )
        );
    }

    #[test]
    fn matrix_actions_order_runtime_console_login_and_live_evidence() {
        assert_eq!(
            matrix_operational_next_action(
                macos_session::State::Locked,
                Some(steam_prefix_status::Readiness::SignInRequired),
                "prefix-runtime-provenance-mismatch",
                "live-session-artifacts",
                "capture-one-live-session",
            ),
            (
                "runtime-provenance",
                "repair-prefix-runtime-provenance-mismatch".to_owned()
            )
        );
        assert_eq!(
            matrix_operational_next_action(
                macos_session::State::Locked,
                Some(steam_prefix_status::Readiness::SignInRequired),
                "ready",
                "live-session-artifacts",
                "capture-one-live-session",
            ),
            ("interactive-console", "unlock-console".to_owned())
        );
        assert_eq!(
            matrix_operational_next_action(
                macos_session::State::Unlocked,
                Some(steam_prefix_status::Readiness::SignInRequired),
                "ready",
                "live-session-artifacts",
                "capture-one-live-session",
            ),
            ("steam-authentication", "sign-in-windows-steam".to_owned())
        );
        assert_eq!(
            matrix_operational_next_action(
                macos_session::State::Unlocked,
                Some(steam_prefix_status::Readiness::StoredLoginPresent),
                "ready",
                "live-session-artifacts",
                "capture-one-live-session",
            ),
            (
                "live-session-evidence",
                "capture-one-live-session".to_owned()
            )
        );


    }

    #[test]
    fn cached_platforms_are_sorted_and_deduplicated() {
        let launches = vec![
            steam_appinfo::LaunchEntry {
                executable: "game.exe".to_owned(),
                working_dir: None,
                description: None,
                os_list: Some("windows,macos".to_owned()),
                os_arch: None,
            },
            steam_appinfo::LaunchEntry {
                executable: "tool.exe".to_owned(),
                working_dir: None,
                description: None,
                os_list: Some("windows".to_owned()),
                os_arch: None,
            },
        ];
        let metadata = steam_appinfo::AppMetadata {
            app_id: 42,
            cache_path: PathBuf::from("appinfo.vdf"),
            name: Some("Fixture".to_owned()),
            app_os_list: Some("linux,windows".to_owned()),
            install_dir: None,
            public_build_id: None,
            launches,
            executable_references: Vec::new(),
            windows_public_depots: Vec::new(),
            cloud_save_rules: Vec::new(),
            cloud_root_overrides: Vec::new(),
        };
        assert_eq!(cached_app_platforms(&metadata), "linux,macos,windows");
    }

    #[test]
    fn target_gate_excludes_proven_native_mac_and_keeps_windows_only_games() {
        fn metadata(launches: Vec<steam_appinfo::LaunchEntry>) -> steam_appinfo::AppMetadata {
            steam_appinfo::AppMetadata {
                app_id: 42,
                cache_path: PathBuf::from("appinfo.vdf"),
                name: Some("Fixture".to_owned()),
                app_os_list: None,
                install_dir: Some("Fixture".to_owned()),
                public_build_id: Some("1".to_owned()),
                launches,
                executable_references: Vec::new(),
                windows_public_depots: Vec::new(),
                cloud_save_rules: Vec::new(),
                cloud_root_overrides: Vec::new(),
            }
        }
        let dual_platform = vec![steam_appinfo::LaunchEntry {
            executable: "game".to_owned(),
            working_dir: None,
            description: None,
            os_list: Some("macos,windows".to_owned()),
            os_arch: None,
        }];
        let windows_only = vec![steam_appinfo::LaunchEntry {
            executable: "game.exe".to_owned(),
            working_dir: None,
            description: None,
            os_list: Some("windows".to_owned()),
            os_arch: None,
        }];
        assert_eq!(
            target_eligibility(
                &metadata(dual_platform.clone()),
                Some(&InstalledTargetEvidence {
                    classification: steam::Classification::NativeMac,
                    pe_binaries: Vec::new(),
                })
            ),
            "exclude-working-native-mac"
        );
        assert_eq!(
            target_eligibility(
                &metadata(dual_platform.clone()),
                Some(&InstalledTargetEvidence {
                    classification: steam::Classification::LegacyMac32,
                    pe_binaries: Vec::new(),
                })
            ),
            "candidate-legacy-mac-fallback"
        );
        assert_eq!(
            target_eligibility(&metadata(windows_only.clone()), None),
            "candidate-windows-only"
        );
        assert_eq!(
            target_eligibility(&metadata(dual_platform), None),
            "review-native-mac-viability"
        );
        let mut app_level_windows = metadata(vec![steam_appinfo::LaunchEntry {
            executable: "game.exe".to_owned(),
            working_dir: None,
            description: None,
            os_list: None,
            os_arch: None,
        }]);
        app_level_windows.app_os_list = Some("windows".to_owned());
        assert_eq!(cached_app_platforms(&app_level_windows), "windows");
        assert_eq!(
            target_eligibility(&app_level_windows, None),
            "candidate-windows-only"
        );
        let windows_64_only = vec![steam_appinfo::LaunchEntry {
            executable: "modern.exe".to_owned(),
            working_dir: None,
            description: None,
            os_list: Some("windows".to_owned()),
            os_arch: Some("64".to_owned()),
        }];
        assert_eq!(
            target_eligibility(&metadata(windows_64_only), None),
            "exclude-windows-64-only"
        );
        assert!(is_candidate_gate("candidate-windows-only"));
        assert!(is_candidate_gate("candidate-legacy-mac-fallback"));
        assert!(!is_candidate_gate("review-native-mac-viability"));
        assert!(!is_candidate_gate("exclude-windows-64-only"));
        assert_eq!(
            target_eligibility(
                &metadata(windows_only.clone()),
                Some(&InstalledTargetEvidence {
                    classification: steam::Classification::WindowsPayload,
                    pe_binaries: vec![(PathBuf::from("game.exe"), PeArchitecture::I386)],
                })
            ),
            "candidate-verified-win32-installed"
        );
        assert_eq!(
            target_eligibility(
                &metadata(windows_only),
                Some(&InstalledTargetEvidence {
                    classification: steam::Classification::WindowsPayload,
                    pe_binaries: vec![(PathBuf::from("game.exe"), PeArchitecture::X86_64)],
                })
            ),
            "exclude-windows-64-installed"
        );
        let windows_launch = vec![steam_appinfo::LaunchEntry {
            executable: "bin/game.exe".to_owned(),
            working_dir: None,
            description: None,
            os_list: Some("windows".to_owned()),
            os_arch: None,
        }];
        assert_eq!(
            target_eligibility(
                &metadata(windows_launch),
                Some(&InstalledTargetEvidence {
                    classification: steam::Classification::WindowsPayload,
                    pe_binaries: vec![
                        (
                            PathBuf::from("redist/vcredist_x86.exe"),
                            PeArchitecture::I386
                        ),
                        (PathBuf::from("bin/game.exe"), PeArchitecture::X86_64),
                    ],
                })
            ),
            "exclude-windows-64-installed"
        );
    }

    #[test]
    fn combined_target_outcome_never_lets_win32_override_native_viability() {
        assert_eq!(
            combined_target_outcome(
                "exclude-working-native-mac",
                "candidate-verified-win32-installed"
            )
            .decision,
            "exclude-working-native-mac"
        );
        let no_windows = combined_target_outcome(
            "exclude-no-windows-launch",
            "candidate-verified-win32-installed",
        );
        assert_eq!(no_windows.decision, "exclude-no-windows-build");
        assert_eq!(no_windows.stage, "excluded");
        assert_eq!(no_windows.next_action, "outside-scope-no-windows-build");
        assert_eq!(
            combined_target_outcome(
                "candidate-legacy-mac-fallback",
                "candidate-verified-win32-installed"
            )
            .decision,
            "eligible-verified-win32-legacy-mac"
        );
        assert_eq!(
            combined_target_outcome(
                "candidate-windows-only",
                "candidate-verified-win32-installed"
            )
            .decision,
            "eligible-verified-win32-windows-only"
        );
        assert_eq!(
            combined_target_outcome("metadata-incomplete", "candidate-verified-win32-installed")
                .decision,
            "review-native-mac-viability"
        );
        let win64 = combined_target_outcome(
            "candidate-legacy-mac-fallback",
            "exclude-windows-64-installed",
        );
        assert_eq!(win64.decision, "experimental-verified-win64-2d");
        assert_eq!(win64.stage, "experimental");
        assert_eq!(win64.next_action, "begin-win64-experimental-validation");
        assert_eq!(
            combined_target_outcome("candidate-windows-only", "exclude-windows-64-installed")
                .stage,
            "experimental"
        );
        assert_eq!(
            combined_target_outcome("exclude-working-native-mac", "exclude-windows-64-installed")
                .decision,
            "exclude-working-native-mac"
        );
        assert_eq!(
            combined_target_outcome("metadata-incomplete", "exclude-windows-64-installed")
                .stage,
            "review"
        );
        assert_eq!(
            combined_target_outcome("candidate-windows-only", "exclude-windows-64-only")
                .stage,
            "blocked"
        );

        let eligible = combined_target_outcome(
            "candidate-windows-only",
            "candidate-verified-win32-installed",
        );
        assert_eq!(eligible.stage, "eligible");
        assert_eq!(eligible.next_action, "begin-runtime-validation");

        let review = combined_target_outcome(
            "review-native-mac-viability",
            "candidate-verified-win32-installed",
        );
        assert_eq!(review.stage, "review");
        assert_eq!(
            review.next_action,
            "verify-official-mac-build-on-target-host"
        );
    }

    #[test]
    fn two_d_scope_gate_fails_closed_without_explicit_review() {
        assert!(!review_scope_allows_2d(None));
        let mut review = TargetEligibilityReview {
            native_status: "windows-only".to_owned(),
            source_url: "https://store.steampowered.com/app/42/Fixture/".to_owned(),
            sha256: "a".repeat(64),
            scope: None,
        };
        assert!(!review_scope_allows_2d(Some(&review)));
        review.scope = Some("3d".to_owned());
        assert!(!review_scope_allows_2d(Some(&review)));
        review.scope = Some("2d".to_owned());
        assert!(review_scope_allows_2d(Some(&review)));
    }

    #[test]
    fn reviewed_inventory_preserves_win32_proof_but_working_native_wins() {
        let metadata = steam_appinfo::AppMetadata {
            app_id: 42,
            cache_path: PathBuf::from("appinfo.vdf"),
            name: Some("Fixture".to_owned()),
            app_os_list: Some("macos,windows".to_owned()),
            install_dir: Some("Fixture".to_owned()),
            public_build_id: Some("1".to_owned()),
            launches: vec![steam_appinfo::LaunchEntry {
                executable: "game.exe".to_owned(),
                working_dir: None,
                description: None,
                os_list: Some("windows".to_owned()),
                os_arch: None,
            }],
            executable_references: Vec::new(),
            windows_public_depots: Vec::new(),
            cloud_save_rules: Vec::new(),
            cloud_root_overrides: Vec::new(),
        };
        let installed = InstalledTargetEvidence {
            classification: steam::Classification::WindowsPayload,
            pe_binaries: vec![(PathBuf::from("game.exe"), PeArchitecture::I386)],
        };
        let mut review = TargetEligibilityReview {
            native_status: "legacy-unusable".to_owned(),
            source_url: "https://store.steampowered.com/app/42/Fixture/".to_owned(),
            sha256: "a".repeat(64),
            scope: None,
        };
        assert_eq!(
            inventory_target_gate(&metadata, Some(&installed), Some(&review)),
            "candidate-verified-win32-installed"
        );
        assert!(is_verified_win32_gate("candidate-verified-win32-installed"));
        assert!(!is_verified_win32_gate("candidate-windows-only"));
        assert!(strict_verified_inventory_candidate(
            &metadata,
            "candidate-verified-win32-installed",
            Some(&review),
        ));
        assert!(!strict_verified_inventory_candidate(
            &metadata,
            "candidate-verified-win32-installed",
            None,
        ));

        review.native_status = "working".to_owned();
        assert_eq!(
            inventory_target_gate(&metadata, Some(&installed), Some(&review)),
            "exclude-working-native-mac"
        );
        assert!(!strict_verified_inventory_candidate(
            &metadata,
            "candidate-verified-win32-installed",
            Some(&review),
        ));

        let mut windows_only = metadata;
        windows_only.app_os_list = Some("windows".to_owned());
        assert!(strict_verified_inventory_candidate(
            &windows_only,
            "candidate-verified-win32-installed",
            None,
        ));
    }

    #[test]
    fn official_store_review_resolves_stale_metadata_but_never_overrides_working_native() {
        let braid =
            load_target_eligibility_review(Path::new("eligibility/26800.conf"), 26800).unwrap();
        assert_eq!(braid.native_status, "working");
        assert!(
            braid
                .source_url
                .starts_with("https://store.steampowered.com/app/26800/")
        );
        assert_eq!(
            reviewed_native_gate("review-native-mac-viability", &braid.native_status),
            "exclude-working-native-mac"
        );

        let mass_effect =
            load_target_eligibility_review(Path::new("eligibility/17460.conf"), 17460).unwrap();
        assert_eq!(mass_effect.native_status, "windows-only");
        assert_eq!(
            reviewed_native_gate("metadata-incomplete", &mass_effect.native_status),
            "candidate-windows-only"
        );
        assert_eq!(
            reviewed_native_gate("exclude-working-native-mac", &mass_effect.native_status),
            "exclude-working-native-mac"
        );
        assert!(
            load_target_eligibility_review(Path::new("eligibility/17460.conf"), 26800)
                .unwrap_err()
                .contains("does not match")
        );
        let reviews = load_inventory_eligibility_reviews(
            Path::new("eligibility"),
            [400, 17460, 205100, 26800, 606680, 999_999].into_iter(),
        )
        .unwrap();
        assert_eq!(reviews.len(), 5);
        assert_eq!(reviews[&400].native_status, "legacy-unusable");
        assert_eq!(reviews[&205100].native_status, "windows-only");
        assert_eq!(reviews[&606680].native_status, "legacy-unusable");
        assert_eq!(reviews[&606680].scope.as_deref(), Some("3d"));
        assert!(!review_scope_allows_2d(reviews.get(&606680)));
        assert!(!reviews.contains_key(&999_999));
    }

    #[test]
    fn launch_status_json_is_machine_readable_and_omits_raw_log_lines() {
        let status = launch_status::LaunchStatus {
            app_id: 42,
            steam_root: PathBuf::from("/private/account/Steam"),
            log_path: PathBuf::from("/private/account/Steam/logs/console_log.txt"),
            state: launch_status::LaunchState::RunningObserved,
            latest_event: Some(
                "account-secret game process added : AppID 42, ProcID 99".to_owned(),
            ),
            process_id: Some(99),
            steam_process_id: Some(1234),
            exit_code: None,
        };
        let json = launch_status_json(&status);
        assert!(json.contains("\"state\": \"running-observed\""));
        assert!(json.contains("\"schema_version\": 2"));
        assert!(json.contains("\"process_id\": 99"));
        assert!(json.contains("\"steam_process_id\": 1234"));
        assert!(json.contains("\"game_process_live\": true"));
        assert!(json.contains("\"event_present\": true"));
        assert!(!json.contains("account-secret"));
        assert!(!json.contains("/private/account"));
    }

    #[test]
    fn lifecycle_phase_never_treats_a_stale_steam_event_as_live_gameplay() {
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::StoredLoginPresent,
                launch_status::LaunchState::RunningObserved,
                false,
                false,
                false,
                false
            ),
            "stale-running-event"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::StoredLoginPresent,
                launch_status::LaunchState::RunningObserved,
                true,
                true,
                false,
                true
            ),
            "running"
        );
    }

    #[test]
    fn lifecycle_phase_orders_bootstrap_login_and_session_gates() {
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Locked,
                steam_prefix_status::Readiness::NotBootstrapped,
                launch_status::LaunchState::NoEvidence,
                false,
                false,
                false,
                false
            ),
            "needs-bootstrap"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Locked,
                steam_prefix_status::Readiness::SignInRequired,
                launch_status::LaunchState::NoEvidence,
                false,
                false,
                false,
                false
            ),
            "needs-login"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Locked,
                steam_prefix_status::Readiness::StoredLoginPresent,
                launch_status::LaunchState::NoEvidence,
                false,
                false,
                false,
                false
            ),
            "interactive-session-blocked"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::StoredLoginPresent,
                launch_status::LaunchState::NoEvidence,
                false,
                false,
                false,
                false
            ),
            "ready-to-launch"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::StoredLoginPresent,
                launch_status::LaunchState::WaitingCloud,
                false,
                true,
                false,
                false
            ),
            "waiting-cloud"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::StoredLoginPresent,
                launch_status::LaunchState::WaitingCloud,
                false,
                false,
                false,
                false
            ),
            "stale-waiting-cloud"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::StoredLoginPresent,
                launch_status::LaunchState::WaitingUser,
                false,
                false,
                false,
                false
            ),
            "stale-waiting-user"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::StoredLoginPresent,
                launch_status::LaunchState::WaitingUser,
                false,
                true,
                false,
                false
            ),
            "waiting-user"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::SignInRequired,
                launch_status::LaunchState::NoEvidence,
                false,
                true,
                false,
                false
            ),
            "login-ui-active"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::SignInRequired,
                launch_status::LaunchState::NoEvidence,
                false,
                true,
                true,
                false
            ),
            "steam-client-updating"
        );
        assert_eq!(
            lifecycle_phase(
                macos_session::State::Unlocked,
                steam_prefix_status::Readiness::SignInRequired,
                launch_status::LaunchState::NoEvidence,
                false,
                true,
                false,
                true
            ),
            "steam-client-update-stalled"
        );
    }

    #[test]
    fn evidence_snapshot_is_redacted_and_marks_non_inferred_runtime_gates() {
        let snapshot = EvidenceSnapshot {
            captured_unix: 1,
            app_id: 42,
            profile_sha256: "ab".repeat(32),
            runtime_id: "wine-test".to_owned(),
            runtime_fingerprint_sha256: Some("cd".repeat(32)),
            profile_runtime_match: true,
            profile_session_match: true,
            steam_session_sha256: Some("ef".repeat(32)),
            renderer: "wined3d-opengl".to_owned(),
            graphics_api: "d3d9".to_owned(),
            steam_client_mode: "standard".to_owned(),
            phase: "running".to_owned(),
            console_state: "unlocked".to_owned(),
            prefix_readiness: "stored-login-present".to_owned(),
            bootstrap_valid: true,
            stored_login_records: 1,
            launch_state: "running-observed".to_owned(),
            tracked_game_pid: Some(99),
            tracked_game_pid_live: true,
            exit_code: None,
            total_rss_kib: 1024,
            process_count: 1,
            steam_cloud_policy: "required".to_owned(),
            cloud_declared_files: 1,
            cloud_state: "synced".to_owned(),
            cloud_matching_files: 1,
        };
        let json = snapshot.to_json();
        assert!(json.contains("\"tracked_game_pid_live\": true"));
        assert!(json.contains("\"profile_runtime_match\": true"));
        assert!(json.contains("\"profile_session_match\": true"));
        assert!(json.contains(&format!(
            "\"steam_session_sha256\": \"{}\"",
            "ef".repeat(32)
        )));
        assert!(json.contains(&format!(
            "\"runtime_fingerprint_sha256\": \"{}\"",
            "cd".repeat(32)
        )));
        assert!(json.contains("\"fresh-cloud-roundtrip\""));
        assert!(json.contains("\"normal-exit\""));
        assert!(!json.contains("/Users/"));
        assert!(!json.contains("account"));
    }

    #[test]
    fn evidence_snapshot_fails_closed_when_runtime_is_unresolved() {
        let snapshot = EvidenceSnapshot {
            captured_unix: 1,
            app_id: 42,
            profile_sha256: "ab".repeat(32),
            runtime_id: "wine-missing".to_owned(),
            runtime_fingerprint_sha256: None,
            profile_runtime_match: false,
            profile_session_match: false,
            steam_session_sha256: None,
            renderer: "wined3d-opengl".to_owned(),
            graphics_api: "d3d9".to_owned(),
            steam_client_mode: "standard".to_owned(),
            phase: "needs-bootstrap".to_owned(),
            console_state: "locked".to_owned(),
            prefix_readiness: "not-bootstrapped".to_owned(),
            bootstrap_valid: false,
            stored_login_records: 0,
            launch_state: "no-evidence".to_owned(),
            tracked_game_pid: None,
            tracked_game_pid_live: false,
            exit_code: None,
            total_rss_kib: 0,
            process_count: 0,
            steam_cloud_policy: "unspecified".to_owned(),
            cloud_declared_files: 0,
            cloud_state: "not-declared".to_owned(),
            cloud_matching_files: 0,
        };

        let json = snapshot.to_json();
        assert!(json.contains("\"runtime_fingerprint_sha256\": null"));
        assert!(json.contains("\"profile_runtime_match\": false"));
        assert!(json.contains("\"profile_session_match\": false"));
        assert!(json.contains("\"steam_session_sha256\": null"));
    }

    #[test]
    fn evidence_writer_refuses_to_replace_an_existing_record() {
        let directory = std::env::temp_dir().join(format!(
            "macroton-evidence-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("snapshot.json");
        write_new_evidence(&destination, b"first").unwrap();
        let error = write_new_evidence(&destination, b"second").unwrap_err();
        assert!(error.contains("refusing to replace"));
        assert_eq!(fs::read(&destination).unwrap(), b"first");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn audio_probe_parser_accepts_only_passing_pcm_invariants() {
        let fixture = r#"{
  "schema": 1,
  "window_id": 7,
  "pid": 99,
  "duration_seconds": 10.0,
  "sample_rate": 48000.0,
  "channels": 2,
  "buffers": 100,
  "frames": 480000,
  "rms_dbfs": -20.0,
  "peak_dbfs": -3.0,
  "silence_threshold_dbfs": -50.0,
  "non_silent_buffers": 90,
  "non_silent_ratio": 0.9,
  "invalid_buffers": 0,
  "format_changes": 0,
  "owner_match": true
}"#;
        let parsed = parse_audio_probe(fixture).unwrap();
        assert_eq!(parsed.pid, 99);
        validate_audio_probe(&parsed).unwrap();

        let silent = fixture.replace("\"non_silent_buffers\": 90", "\"non_silent_buffers\": 0");
        assert!(validate_audio_probe(&parse_audio_probe(&silent).unwrap()).is_err());
        let duplicate = fixture.replace("\"pid\": 99,", "\"pid\": 99,\n  \"pid\": 100,");
        assert!(parse_audio_probe(&duplicate).is_err());
    }

    #[test]
    fn focus_stream_parser_requires_focused_stable_identity_and_order() {
        let fixture = concat!(
            "frame,elapsed_ms,frontmost,average_luma,sampled_sha256,window_id,pid\n",
            "0,0.000,true,10.0,aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,7,99\n",
            "1,16.667,true,11.0,bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,7,99\n",
            "2,33.334,true,0.5,cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc,7,99\n"
        );
        let evidence = parse_focus_stream(fixture, 1.0).unwrap();
        assert_eq!(evidence.window_id, 7);
        assert_eq!(evidence.pid, 99);
        assert_eq!(evidence.frames, 3);
        assert_eq!(evidence.black_frames, 1);
        assert_eq!(evidence.unique_sampled_frames, 3);
        assert_eq!(evidence.changed_transitions, 2);
        assert!((evidence.delivered_rate_fps - 59.999).abs() < 0.01);
        assert!((evidence.changed_rate_fps - 59.999).abs() < 0.01);
        assert!((evidence.max_unchanged_ms - 16.667).abs() < 0.001);
        assert!((evidence.p95_change_interval_ms - 16.667).abs() < 0.001);
        assert!((evidence.max_gap_ms - 16.667).abs() < 0.001);
        validate_motion_rate_bounds(&evidence, 0.03, 50.0, 20.0).unwrap();
        assert!(validate_motion_rate_bounds(&evidence, 0.04, 50.0, 20.0).is_err());
        assert!(validate_motion_rate_bounds(&evidence, 0.03, 61.0, 20.0).is_err());
        assert!(validate_motion_rate_bounds(&evidence, 0.03, 50.0, 10.0).is_err());
        assert!(parse_focus_stream(&fixture.replace(",true,", ",false,"), 1.0).is_err());
        assert!(parse_focus_stream(&fixture.replace(",7,99\n2", ",8,99\n2"), 1.0).is_err());
        assert!(parse_focus_stream(&fixture.replace("1,16.667", "4,16.667"), 1.0).is_err());

        let frozen = fixture.replace(
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );
        let frozen = parse_focus_stream(&frozen, 1.0).unwrap();
        assert_eq!(frozen.changed_transitions, 1);
        assert!((frozen.max_unchanged_ms - 16.667).abs() < 0.001);
    }

    #[test]
    fn gpu_capture_digest_is_stable_content_addressed_and_rejects_empty_files() {
        let root = std::env::temp_dir().join(format!(
            "macroton-gpu-capture-{}-{}",
            std::process::id(),
            unix_time_ms().unwrap()
        ));
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("z.bin"), b"frame-z").unwrap();
        fs::write(root.join("nested/a.bin"), b"frame-a").unwrap();
        let first = digest_gpu_capture(&root).unwrap();
        let second = digest_gpu_capture(&root).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.file_count, 2);
        assert_eq!(first.total_bytes, 14);

        fs::write(root.join("nested/a.bin"), b"changed").unwrap();
        assert_ne!(digest_gpu_capture(&root).unwrap().sha256, first.sha256);
        fs::write(root.join("empty.bin"), b"").unwrap();
        assert!(
            digest_gpu_capture(&root)
                .unwrap_err()
                .contains("empty file")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interaction_probe_parser_requires_identity_focus_and_real_frame_change() {
        let fixture = include_str!("../tests/fixtures/interaction_probe_pass.json");
        let evidence = parse_interaction_probe(fixture).unwrap();
        assert_eq!(evidence.window_id, 44_774);
        assert_eq!(evidence.pid, 4_242);
        assert_eq!(evidence.action_kind, "key");
        assert_eq!(evidence.action_value, "escape");
        assert_eq!(evidence.changed_ratio, 0.25);
        let spaced = fixture.replace("\":", "\" :");
        assert_eq!(parse_interaction_probe(&spaced).unwrap().pid, 4_242);
        assert!(
            parse_interaction_probe(
                &fixture.replace("\"frontmost_after\": true", "\"frontmost_after\": false")
            )
            .is_err()
        );
        assert!(
            parse_interaction_probe(
                &fixture.replace("\"changed_pixels\": 250", "\"changed_pixels\": 0")
            )
            .is_err()
        );
        assert!(
            parse_interaction_probe(
                &fixture.replace("\"pid\": 4242,", "\"pid\": 4242,\n  \"pid\": 7,")
            )
            .is_err()
        );
    }

    #[test]
    fn runtime_path_audit_requires_managed_paths_and_detects_commercial_runtime() {
        let output = concat!(
            "p42\n",
            "n/private/tmp/runtime/bin/wine64-preloader\n",
            "n/private/tmp/data/games/42/prefix/drive_c/game.exe\n",
            "n/usr/lib/libSystem.B.dylib\n",
            "n/private/tmp/runtime/bin/wine64-preloader\n"
        );
        let audit = parse_runtime_path_audit(
            output.as_bytes(),
            Path::new("/private/tmp/runtime/bin"),
            Path::new("/private/tmp/runtime"),
            Path::new("/private/tmp/data/games/42/prefix"),
        )
        .unwrap();
        assert_eq!(audit.absolute_paths.len(), 3);
        assert_eq!(audit.runtime_paths, 1);
        assert_eq!(audit.prefix_paths, 1);
        assert!(audit.forbidden_paths.is_empty());

        let forbidden = parse_runtime_path_audit(
            b"n/Applications/CrossOver.app/Contents/SharedSupport/lib/wine.dylib\n",
            Path::new("/private/tmp/runtime/bin"),
            Path::new("/private/tmp/runtime"),
            Path::new("/private/tmp/prefix"),
        )
        .unwrap();
        assert_eq!(forbidden.forbidden_paths.len(), 1);
    }

    #[test]
    fn runtime_path_audit_counts_bundle_payload_libs() {
        let output = concat!(
            "n/private/tmp/Wine Devel.app/Contents/Resources/wine/bin/wine\n",
            "n/private/tmp/Wine Devel.app/Contents/Resources/wine/lib/wine/d3d.dll.so\n",
            "n/private/tmp/data/games/42/prefix/drive_c/game.exe\n"
        );
        let audit = parse_runtime_path_audit(
            output.as_bytes(),
            Path::new("/private/tmp/Wine Devel.app/Contents/Resources/wine/bin"),
            Path::new("/private/tmp/Wine Devel.app/Contents/Resources/wine"),
            Path::new("/private/tmp/data/games/42/prefix"),
        )
        .unwrap();
        assert_eq!(audit.runtime_paths, 2);
        assert_eq!(audit.prefix_paths, 1);
    }

    #[test]
    fn inventory_names_cannot_inject_output_rows() {
        assert_eq!(single_line("one\ttwo\nthree\rfour"), "one two three four");
        let entry = steam_library_status::LibraryStatus {
            app_id: 42,
            steam_root: PathBuf::from("redacted"),
            account_records: 1,
            max_playtime_minutes: Some(7),
            latest_last_played_unix: None,
            cached_app_tickets: 0,
        };
        let json = inventory_entry_json(
            &entry,
            Some("one\ntwo\"three"),
            Some("windows"),
            Some("candidate-windows-only"),
            Some(&TargetEligibilityReview {
                native_status: "windows-only".to_owned(),
                source_url: "https://store.steampowered.com/app/42/Fixture/".to_owned(),
                sha256: "a".repeat(64),
                scope: None,
            }),
        );
        assert!(json.contains("\"name\": \"one\\ntwo\\\"three\""));
        assert!(json.contains("\"latest_last_played_unix\": null"));
        assert!(json.contains("\"eligibility_review_sha256\": \"aaaaaaaa"));
        assert!(json.contains("https://store.steampowered.com/app/42/Fixture/"));
        assert!(!json.contains("redacted"));
    }

    #[test]
    fn global_resource_status_sums_prefixes_and_rejects_duplicate_pid_ownership() {
        let usage = |prefix: &str, pid: u32, rss_kib: u64| stop::PrefixResourceUsage {
            prefix: PathBuf::from(prefix),
            pids: vec![pid],
            total_rss_kib: rss_kib,
            processes: vec![stop::ProcessResourceUsage {
                pid,
                rss_kib,
                command: format!("{prefix}/game.exe"),
                category: "windows-game",
            }],
        };
        let usages = vec![
            (42, usage("/secret/42", 100, 1024)),
            (43, usage("/secret/43", 101, 2048)),
        ];
        validate_unique_resource_pids(&usages).unwrap();
        let json = resource_status_all_json(&usages, 2);
        assert!(json.contains("\"total_rss_kib\": 3072"));
        assert!(json.contains("\"over_limit\": true"));
        assert!(json.contains("\"active_prefix_count\": 2"));
        assert!(!json.contains("/secret"));

        let duplicate = vec![(42, usage("a", 100, 1)), (43, usage("b", 100, 2))];
        assert!(
            validate_unique_resource_pids(&duplicate)
                .unwrap_err()
                .contains("both AppID 42 and AppID 43")
        );
    }

    #[test]
    fn release_profile_requires_pinned_public_runtime_and_rejects_commercial_fields() {
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nwindows_depot_id=43\nwindows_depot_manifest_id=44\nruntime=macroton-winehq-macos\nruntime_version=11.14\nrenderer=wined3d-opengl\ngraphics_api=d3d9\nlaunch=steam\nsteam_install_dir=Fixture\nwindows_build_id=45\nexecutable=game.exe\nsteam_cloud=unsupported\nsteam_cloud_source=https://store.steampowered.com/app/42/Fixture/\nsteam_cloud_reviewed_date=2026-08-13\n",
        )
        .unwrap();
        let manifest = RuntimeManifest::parse(
            "schema=1\nid=macroton-winehq-macos-11.14\nfamily=macroton-winehq-macos\nversion=11.14\nsource=https://example.invalid/runtime.tar\nlicense=LGPL-2.1-or-later\nlicense_url=https://example.invalid/license\nsha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\narchive=tar\nentrypoint=bin/wine\n",
        )
        .unwrap();
        validate_release_profile(&profile, &manifest).unwrap();

        let mut unresolved_cloud = profile.clone();
        unresolved_cloud.steam_cloud = SteamCloudPolicy::Unspecified;
        assert!(
            validate_release_profile(&unresolved_cloud, &manifest)
                .unwrap_err()
                .contains("cloud_files or steam_cloud=unsupported")
        );

        let mut experimental = profile.clone();
        experimental.steam_client_mode = macroton::profile::SteamClientMode::LowMemoryExperimental;
        assert!(
            validate_release_profile(&experimental, &manifest)
                .unwrap_err()
                .contains("Standard")
        );

        let mut forbidden = profile.clone();
        forbidden.environment.insert(
            "DYLD_FALLBACK_LIBRARY_PATH".to_owned(),
            "/Applications/CrossOver.app/Contents/SharedSupport".to_owned(),
        );
        assert!(
            validate_release_profile(&forbidden, &manifest)
                .unwrap_err()
                .contains("forbidden commercial-runtime")
        );

        let mut unpinned = profile;
        unpinned.windows_depots[0].manifest_id = None;
        assert!(
            validate_release_profile(&unpinned, &manifest)
                .unwrap_err()
                .contains("every Windows depot manifest")
        );
    }

    #[test]
    fn release_win32_gate_accepts_exact_no_copy_depot_and_rejects_x64() {
        assert_eq!(
            release_win32_gate(
                WindowsPayloadRoot::NativeDepotContent,
                &[],
                Some(PeArchitecture::I386),
            )
            .unwrap(),
            "candidate-verified-win32-depot-content"
        );
        assert!(
            release_win32_gate(
                WindowsPayloadRoot::NativeDepotContent,
                &[],
                Some(PeArchitecture::X86_64),
            )
            .unwrap_err()
            .contains("PE32/i386")
        );
        assert!(
            release_win32_gate(
                WindowsPayloadRoot::NativeDepotContent,
                &["missing pinned depot".to_owned()],
                Some(PeArchitecture::I386),
            )
            .unwrap_err()
            .contains("missing pinned depot")
        );
    }

    #[test]
    fn steamcmd_validation_fails_closed_in_gate_order() {
        assert_eq!(
            steamcmd_validation_state(
                steamcmd::AcquisitionState::Missing,
                true,
                steam_prefix_status::Readiness::StoredLoginPresent,
                prerequisite_status::State::PassedToGame,
            ),
            "blocked-acquisition"
        );
        assert_eq!(
            steamcmd_validation_state(
                steamcmd::AcquisitionState::Ready,
                true,
                steam_prefix_status::Readiness::NotBootstrapped,
                prerequisite_status::State::PassedToGame,
            ),
            "blocked-steam-client"
        );
        assert_eq!(
            steamcmd_validation_state(
                steamcmd::AcquisitionState::Ready,
                true,
                steam_prefix_status::Readiness::StoredLoginPresent,
                prerequisite_status::State::SignatureMissing,
            ),
            "blocked-prerequisite-integrity"
        );
    }

    #[test]
    fn steamcmd_validation_distinguishes_first_launch_from_runtime_evidence() {
        assert_eq!(
            steamcmd_validation_state(
                steamcmd::AcquisitionState::Ready,
                true,
                steam_prefix_status::Readiness::SignInRequired,
                prerequisite_status::State::Pending,
            ),
            "ready-for-first-launch"
        );
        assert_eq!(
            steamcmd_validation_state(
                steamcmd::AcquisitionState::Ready,
                false,
                steam_prefix_status::Readiness::NotBootstrapped,
                prerequisite_status::State::PassedToGame,
            ),
            "prerequisite-passed"
        );
    }

    #[test]
    fn trim_client_requires_current_exact_prefix_game_pid() {
        let mut launch = launch_status::LaunchStatus {
            app_id: 42,
            steam_root: PathBuf::from("/fixture/Steam"),
            log_path: PathBuf::from("/fixture/Steam/logs/console_log.txt"),
            state: launch_status::LaunchState::Submitted,
            latest_event: None,
            process_id: None,
            steam_process_id: None,
            exit_code: None,
        };
        assert!(trim_client_game_pid(&launch, &[]).is_err());
        launch.state = launch_status::LaunchState::RunningObserved;
        launch.process_id = Some(99);
        let process = |pid, category| stop::ProcessResourceUsage {
            pid,
            rss_kib: 10,
            command: "fixture.exe".to_owned(),
            category,
        };
        assert!(trim_client_game_pid(&launch, &[process(98, "windows-game")]).is_err());
        assert!(trim_client_game_pid(&launch, &[process(99, "steam-client")]).is_err());
        assert_eq!(
            trim_client_game_pid(
                &launch,
                &[process(98, "steam-client"), process(99, "windows-game")]
            )
            .unwrap(),
            99
        );
    }

    #[test]
    fn resource_guard_and_trim_share_real_steam_categories() {
        for category in ["steam-client", "steam-cef", "steamcmd"] {
            assert!(is_windows_steam_resource_category(category));
        }
        for category in ["windows-game", "wine-service", "windows-steam"] {
            assert!(!is_windows_steam_resource_category(category));
        }
        let process = |command: &str| stop::ProcessResourceUsage {
            pid: 1,
            rss_kib: 1,
            command: command.to_owned(),
            category: "steam-client",
        };
        assert!(is_primary_steam_client_process(&process("Steam.exe")));
        assert!(!is_primary_steam_client_process(&process(
            "steamservice.exe"
        )));
    }

    #[test]
    fn resource_guard_reuse_requires_same_app_root_runtime_and_profile() {
        let listing = b" 101 /tmp/macroton resource guard 26800 --data-root /tmp/macroton --runtime-id wine-1 --profile-sha256 abc\n 202 /tmp/macroton resource guard 400 --data-root /tmp/macroton --runtime-id wine-1 --profile-sha256 abc\n";
        assert_eq!(
            find_existing_resource_guard_pid(
                listing,
                26800,
                Path::new("/tmp/macroton"),
                "wine-1",
                Some("abc"),
            ),
            Some(101)
        );
        assert_eq!(
            find_existing_resource_guard_pid(
                listing,
                26800,
                Path::new("/tmp/macroton"),
                "wine-1",
                Some("different"),
            ),
            None
        );
    }

    #[test]
    fn resource_guard_allows_a_bounded_startup_burst_but_stops_sustained_excess() {
        let limit = 1_600 * 1024;
        let hard = limit * 3 / 2;
        let mut consecutive = 0;
        for expected in 1..RESOURCE_GUARD_OVER_LIMIT_SAMPLES {
            assert_eq!(
                resource_limit_action(1_900 * 1024, limit, hard, &mut consecutive),
                ResourceLimitAction::StartupBurst
            );
            assert_eq!(consecutive, expected);
        }
        assert_eq!(
            resource_limit_action(1_900 * 1024, limit, hard, &mut consecutive),
            ResourceLimitAction::StopSustained
        );
    }

    #[test]
    fn resource_guard_resets_burst_counter_and_keeps_a_hard_ceiling() {
        let limit = 1_600 * 1024;
        let hard = limit * 3 / 2;
        let mut consecutive = 0;
        assert_eq!(
            resource_limit_action(1_900 * 1024, limit, hard, &mut consecutive),
            ResourceLimitAction::StartupBurst
        );
        assert_eq!(
            resource_limit_action(1_500 * 1024, limit, hard, &mut consecutive),
            ResourceLimitAction::WithinLimit
        );
        assert_eq!(consecutive, 0);
        assert_eq!(
            resource_limit_action(2_401 * 1024, limit, hard, &mut consecutive),
            ResourceLimitAction::StopHard
        );
    }

    #[test]
    fn resource_guard_honors_an_explicit_hard_ceiling_during_startup() {
        let mut consecutive = 0;
        assert_eq!(
            resource_limit_action(1_900 * 1024, 1_800 * 1024, 2_000 * 1024, &mut consecutive),
            ResourceLimitAction::StartupBurst
        );
        assert_eq!(
            resource_limit_action(2_001 * 1024, 1_800 * 1024, 2_000 * 1024, &mut consecutive),
            ResourceLimitAction::StopHard
        );
    }

    #[test]
    fn lsof_cwd_parser_accepts_only_an_absolute_cwd_record() {
        assert_eq!(
            parse_lsof_cwd("p123\nfcwd\nnC:/not-a-host-path\nn/private/tmp/game\n"),
            Some("/private/tmp/game")
        );
        assert_eq!(
            parse_lsof_cwd("p123\nfcwd\nn/private/tmp/game\n"),
            Some("/private/tmp/game")
        );
        assert_eq!(parse_lsof_cwd("p123\nfcwd\n"), None);
    }

    #[test]
    fn cwd_probe_ignores_generic_windows_helpers_without_app_metadata() {
        let root = std::env::temp_dir().join(format!(
            "macroton-missing-appinfo-cwd-test-{}",
            std::process::id()
        ));
        let candidates = resource_guard_cwd_candidates(297000, &root, Some("dir\\HOMM3 2.0.exe"));
        assert_eq!(candidates.len(), 1);
        assert!(candidates.contains("homm3 2.0.exe"));
        assert!(!candidates.contains("rundll32.exe"));
        assert!(resource_guard_cwd_candidates(297000, &root, None).is_empty());
    }

    #[test]
    fn resource_guard_keeps_watching_across_a_steam_startup_restart_gap() {
        let grace = Duration::from_secs(30);
        assert!(!resource_guard_idle_is_terminal(
            true,
            false,
            Duration::from_secs(5),
            grace
        ));
        assert!(resource_guard_idle_is_terminal(
            true,
            false,
            Duration::from_secs(30),
            grace
        ));
        assert!(resource_guard_idle_is_terminal(
            true,
            true,
            Duration::from_secs(5),
            grace
        ));
    }

    #[test]
    fn steam_game_handoff_timeout_requires_steam_without_a_game() {
        let timeout = Duration::from_secs(STEAM_GAME_HANDOFF_TIMEOUT_SECONDS);
        assert!(!steam_game_handoff_timed_out(false, false, timeout, true));
        assert!(!steam_game_handoff_timed_out(true, true, timeout, true));
        assert!(!steam_game_handoff_timed_out(true, false, timeout, false));
        assert!(!steam_game_handoff_timed_out(
            true,
            false,
            timeout - Duration::from_secs(1),
            true
        ));
        assert!(steam_game_handoff_timed_out(true, false, timeout, true));
    }

    #[test]
    fn resource_guard_reports_changes_and_bounded_heartbeats() {
        assert!(resource_guard_should_report(1, 5, 100_000, 10, None));
        assert!(!resource_guard_should_report(
            2,
            5,
            100_100,
            10,
            Some((100_000, 10))
        ));
        assert!(resource_guard_should_report(
            3,
            5,
            100_100,
            11,
            Some((100_000, 10))
        ));
        assert!(resource_guard_should_report(
            4,
            5,
            116_384,
            10,
            Some((100_000, 10))
        ));
        assert!(resource_guard_should_report(
            60,
            5,
            100_100,
            10,
            Some((100_000, 10))
        ));
        assert!(resource_guard_should_report(
            1,
            600,
            100_100,
            10,
            Some((100_000, 10))
        ));
    }

    #[test]
    fn steam_login_requires_ten_continuous_seconds_of_client_liveness() {
        let mut consecutive = 0;
        for _ in 0..STEAM_LOGIN_STABLE_SAMPLES - 1 {
            assert!(!steam_login_liveness_observed(&mut consecutive, true));
        }
        assert_eq!(consecutive, STEAM_LOGIN_STABLE_SAMPLES - 1);
        assert!(steam_login_liveness_observed(&mut consecutive, true));
    }

    #[test]
    fn steam_login_liveness_resets_after_a_bootstrap_restart() {
        let mut consecutive = 0;
        for _ in 0..STEAM_LOGIN_STABLE_SAMPLES - 1 {
            assert!(!steam_login_liveness_observed(&mut consecutive, true));
        }
        assert!(!steam_login_liveness_observed(&mut consecutive, false));
        assert_eq!(consecutive, 0);
        for _ in 0..STEAM_LOGIN_STABLE_SAMPLES - 1 {
            assert!(!steam_login_liveness_observed(&mut consecutive, true));
        }
        assert!(steam_login_liveness_observed(&mut consecutive, true));
    }

    #[test]
    fn steam_client_update_requires_explicit_progress_marker() {
        let path = std::env::temp_dir().join(format!(
            "macroton-steam-update-test-{}.log",
            std::process::id()
        ));
        fs::write(&path, "[2026-09-04] Verification complete\n").unwrap();
        let mut length = fs::metadata(&path).unwrap().len();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"[2026-09-04] Nothing to do\n")
            .unwrap();
        let current_len = fs::metadata(&path).unwrap().len();
        assert!(!steam_update_progress_observed(
            &path,
            &mut length,
            current_len
        ));
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"[2026-09-04] Downloading update (1 of 2 KB)...\n")
            .unwrap();
        let current_len = fs::metadata(&path).unwrap().len();
        assert!(steam_update_progress_observed(
            &path,
            &mut length,
            current_len
        ));
        assert_eq!(length, current_len);
        assert!(!steam_update_progress_observed(
            &path,
            &mut length,
            current_len
        ));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn steam_login_ui_requires_a_visible_window_marker_after_launch() {
        let old = b"SP DesktopLoginWindow_old: WasHidden 0\r\n";
        let mut log = old.to_vec();
        log.extend_from_slice(
            b"SP DesktopLoginWindow_new: Created window\r\nSP DesktopLoginWindow_new: WasHidden 0\r\n",
        );
        assert!(!steam_login_ui_marker_observed(old, old.len() as u64));
        assert!(steam_login_ui_marker_observed(&log, old.len() as u64));
        assert!(!steam_login_ui_marker_observed(&log, log.len() as u64 + 1));
    }

    #[test]
    fn trim_client_evidence_is_profile_bound_redacted_and_quantified() {
        let profile = CompatibilityProfile::load(Path::new("compatibility/606680.conf")).unwrap();
        let process = |pid, rss_kib, command: &str, category| stop::ProcessResourceUsage {
            pid,
            rss_kib,
            command: command.to_owned(),
            category,
        };
        let before = stop::PrefixResourceUsage {
            prefix: PathBuf::from("/secret/prefix"),
            pids: vec![9, 10],
            total_rss_kib: 900_000,
            processes: vec![
                process(9, 300_000, "silver.exe", "windows-game"),
                process(10, 600_000, "steamwebhelper.exe", "steam-cef"),
            ],
        };
        let after = stop::PrefixResourceUsage {
            prefix: PathBuf::from("/secret/prefix"),
            pids: vec![9],
            total_rss_kib: 300_000,
            processes: vec![process(9, 300_000, "silver.exe", "windows-game")],
        };
        let output = trim_client_evidence_json(
            606680,
            20,
            &TrimClientEvidenceObservation {
                profile: &profile,
                runtime_fingerprint_sha256: &"cd".repeat(32),
                steam_session_sha256: &"ef".repeat(32),
                game_pid: 9,
                before: &before,
                after: &after,
            },
        );
        assert!(output.contains("\"game_still_resident\": true"));
        assert!(output.contains("\"rss_reduction_kib\": 600000"));
        assert!(output.contains("\"steam_resident_after\": false"));
        assert!(!output.contains("secret"));
        assert!(!output.contains("steamwebhelper"));
    }

    #[test]
    fn resource_peak_json_is_machine_readable_and_omits_prefix_paths() {
        let usage = stop::PrefixResourceUsage {
            prefix: PathBuf::from("/secret/prefix"),
            pids: vec![9],
            total_rss_kib: 2048,
            processes: vec![stop::ProcessResourceUsage {
                pid: 9,
                rss_kib: 2048,
                command: "game.exe".to_owned(),
                category: "windows-game",
            }],
        };
        let provenance = ResourceGuardProvenance {
            identity: steam_session::Identity {
                app_id: 42,
                profile_sha256: "ab".repeat(32),
                runtime_id: "wine-test".to_owned(),
                runtime_fingerprint_sha256: "cd".repeat(32),
                steam_client_mode: "standard".to_owned(),
                renderer: "wined3d-opengl".to_owned(),
                graphics_api: "d3d9".to_owned(),
            },
            steam_session_sha256: "ef".repeat(32),
        };
        let observation = ResourcePeakObservation {
            peak: Some(&usage),
            reason: "prefix-idle",
            limit_rss_kib: 4096,
            provenance: Some(&provenance),
            started_unix_ms: 10,
            observed: Duration::from_millis(10),
            samples: 3,
            observed_steam_process: true,
            observed_game_process: true,
            max_consecutive_steam_samples: 3,
            max_consecutive_game_samples: 3,
            interval_seconds: 5,
        };
        let output = resource_peak_json(42, &observation, 20);
        assert!(output.contains("\"schema\": 3"));
        assert!(output.contains("\"observed_steam_process\": true"));
        assert!(output.contains("\"observed_game_process\": true"));
        assert!(output.contains("\"evidence_scope\": \"steam-profile-session\""));
        assert!(output.contains("\"steam_client_mode\": \"standard\""));
        assert!(output.contains(&format!(
            "\"steam_session_sha256\": \"{}\"",
            "ef".repeat(32)
        )));
        assert!(output.contains("\"peak_rss_kib\": 2048"));
        assert!(output.contains("\"windows-game\":2048"));
        assert!(output.contains("\"command\":\"game.exe\""));
        assert!(!output.contains("secret"));
    }

    #[test]
    fn unattributed_resource_peak_cannot_be_mistaken_for_profile_ab_evidence() {
        let observation = ResourcePeakObservation {
            peak: None,
            reason: "startup-no-process",
            limit_rss_kib: 4096,
            provenance: None,
            started_unix_ms: 10,
            observed: Duration::from_millis(10),
            samples: 1,
            observed_steam_process: false,
            observed_game_process: false,
            max_consecutive_steam_samples: 0,
            max_consecutive_game_samples: 0,
            interval_seconds: 5,
        };
        let output = resource_peak_json(42, &observation, 20);
        assert!(output.contains("\"evidence_scope\": \"unattributed-guard\""));
        assert!(output.contains("\"profile_sha256\": null"));
        assert!(output.contains("\"steam_session_sha256\": null"));
        let parsed = parse_resource_peak_record(&output).unwrap();
        assert_eq!(parsed.evidence_scope, "unattributed-guard");
        assert!(parsed.profile_sha256.is_empty());
    }

    fn attributed_resource_record(mode: &str, profile_byte: &str, session_byte: &str) -> String {
        let usage = stop::PrefixResourceUsage {
            prefix: PathBuf::from("/not-emitted"),
            pids: vec![9],
            total_rss_kib: if mode == "standard" { 900_000 } else { 700_000 },
            processes: vec![stop::ProcessResourceUsage {
                pid: 9,
                rss_kib: if mode == "standard" { 900_000 } else { 700_000 },
                command: "game.exe".to_owned(),
                category: "windows-game",
            }],
        };
        let provenance = ResourceGuardProvenance {
            identity: steam_session::Identity {
                app_id: 42,
                profile_sha256: profile_byte.repeat(32),
                runtime_id: "wine-test".to_owned(),
                runtime_fingerprint_sha256: "cd".repeat(32),
                steam_client_mode: mode.to_owned(),
                renderer: "wined3d-opengl".to_owned(),
                graphics_api: "d3d9".to_owned(),
            },
            steam_session_sha256: session_byte.repeat(32),
        };
        let observation = ResourcePeakObservation {
            peak: Some(&usage),
            reason: "time-limit",
            limit_rss_kib: 1_638_400,
            provenance: Some(&provenance),
            started_unix_ms: 10,
            observed: Duration::from_secs(300),
            samples: 61,
            observed_steam_process: true,
            observed_game_process: true,
            max_consecutive_steam_samples: 61,
            max_consecutive_game_samples: 61,
            interval_seconds: 5,
        };
        resource_peak_json(42, &observation, 300_010)
    }

    #[test]
    fn resource_comparison_requires_matched_real_session_game_observations() {
        let standard =
            parse_resource_peak_record(&attributed_resource_record("standard", "ab", "12"))
                .unwrap();
        let low_memory = parse_resource_peak_record(&attributed_resource_record(
            "low-memory-experimental",
            "ef",
            "34",
        ))
        .unwrap();
        validate_resource_comparison(42, ResourceComparePhase::Game, 300, &standard, &low_memory)
            .unwrap();

        let mut no_game = low_memory.clone();
        no_game.observed_game_process = false;
        assert!(
            validate_resource_comparison(42, ResourceComparePhase::Game, 300, &standard, &no_game,)
                .unwrap_err()
                .contains("never observed a Windows game")
        );

        let mut flashed_game = low_memory.clone();
        flashed_game.max_consecutive_game_samples = 1;
        assert!(
            validate_resource_comparison(
                42,
                ResourceComparePhase::Game,
                300,
                &standard,
                &flashed_game,
            )
            .unwrap_err()
            .contains("continuous game seconds")
        );

        let mut drifted = low_memory;
        drifted.renderer = "dxvk-d3d9-moltenvk".to_owned();
        assert!(
            validate_resource_comparison(42, ResourceComparePhase::Game, 300, &standard, &drifted,)
                .unwrap_err()
                .contains("changed renderer")
        );
    }

    #[test]
    fn silver_resource_ab_profiles_differ_only_in_client_mode() {
        let standard_profile =
            CompatibilityProfile::load(Path::new("compatibility/606680.conf")).unwrap();
        let low_memory_profile = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/606680-low-memory.conf",
        ))
        .unwrap();
        let mut standard =
            parse_resource_peak_record(&attributed_resource_record("standard", "ab", "12"))
                .unwrap();
        let mut low_memory = parse_resource_peak_record(&attributed_resource_record(
            "low-memory-experimental",
            "ef",
            "34",
        ))
        .unwrap();
        standard.profile_sha256 = standard_profile.fingerprint();
        low_memory.profile_sha256 = low_memory_profile.fingerprint();
        validate_resource_comparison_profiles(
            606680,
            &standard_profile,
            &low_memory_profile,
            &standard,
            &low_memory,
        )
        .unwrap();

        let mut drifted_profile = low_memory_profile;
        drifted_profile.arguments.push("-different".to_owned());
        assert!(
            validate_resource_comparison_profiles(
                606680,
                &standard_profile,
                &drifted_profile,
                &standard,
                &low_memory,
            )
            .unwrap_err()
            .contains("profile fingerprint")
        );
    }

    #[test]
    fn portal_low_memory_experiment_differs_only_in_client_mode() {
        let standard = CompatibilityProfile::load(Path::new("compatibility/400.conf")).unwrap();
        let mut low_memory =
            CompatibilityProfile::load(Path::new("compatibility/experiments/400-low-memory.conf"))
                .unwrap();
        assert_eq!(
            low_memory.steam_client_mode,
            macroton::profile::SteamClientMode::LowMemoryExperimental
        );
        low_memory.steam_client_mode = standard.steam_client_mode;
        assert_eq!(low_memory, standard);
    }

    #[test]
    fn portal_rendered_login_experiment_differs_only_in_client_mode() {
        let standard = CompatibilityProfile::load(Path::new("compatibility/400.conf")).unwrap();
        let mut rendered = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/400-rendered-login.conf",
        ))
        .unwrap();
        assert_eq!(
            rendered.steam_client_mode,
            macroton::profile::SteamClientMode::RenderedLoginExperimental
        );
        rendered.steam_client_mode = standard.steam_client_mode;
        assert_eq!(rendered, standard);
    }

    #[test]
    fn portal_opaque_software_experiment_differs_only_in_client_mode() {
        let standard = CompatibilityProfile::load(Path::new("compatibility/400.conf")).unwrap();
        let mut opaque = CompatibilityProfile::load(Path::new(
            "compatibility/experiments/400-opaque-software.conf",
        ))
        .unwrap();
        assert_eq!(
            opaque.steam_client_mode,
            macroton::profile::SteamClientMode::OpaqueSoftwareExperimental
        );
        opaque.steam_client_mode = standard.steam_client_mode;
        assert_eq!(opaque, standard);
    }

    #[test]
    fn resource_peak_parser_rejects_old_unknown_and_duplicate_schemas() {
        let valid = attributed_resource_record("standard", "ab", "12");
        assert!(
            parse_resource_peak_record(&valid.replace("\"schema\": 3", "\"schema\": 2"))
                .unwrap_err()
                .contains("schema 3")
        );
        assert!(
            parse_resource_peak_record(&valid.replace(
                "\"captured_unix_ms\": 300010,",
                "\"unknown\": 1,\n  \"captured_unix_ms\": 300010,"
            ))
            .unwrap_err()
            .contains("unknown field")
        );
        assert!(
            parse_resource_peak_record(
                &valid.replace("\"samples\": 61,", "\"samples\": 61,\n  \"samples\": 61,")
            )
            .unwrap_err()
            .contains("duplicate field")
        );
    }

    #[test]
    fn install_status_json_is_machine_readable_and_escapes_names() {
        let status = steam_install_status::InstallStatus {
            app_id: 42,
            steam_root: PathBuf::from("/not-emitted"),
            manifest_path: PathBuf::from("/not-emitted/manifest"),
            content_log_path: PathBuf::from("/not-emitted/log"),
            state: steam_install_status::InstallState::Stalled,
            name: Some("Fixture \"Game\"".to_owned()),
            installed_build_id: Some(0),
            target_build_id: Some(7),
            manifest_size_on_disk: Some(0),
            installed_payload_directory_present: false,
            installed_payload_directory_is_symlink: false,
            installed_payload_entry_observed: false,
            bytes_to_download: Some(100),
            manifest_bytes_downloaded: Some(0),
            bytes_to_stage: Some(120),
            manifest_bytes_staged: Some(0),
            attempt_start_downloaded_bytes: Some(40),
            attempt_download_total_bytes: Some(100),
            attempt_start_staged_bytes: Some(48),
            attempt_stage_total_bytes: Some(120),
            observed_network_bytes: Some(40),
            latest_rate_mbps: Some(0.0),
            selected_depots: vec![8, 9],
            installed_depots: Vec::new(),
            recent_connection_failures: 3,
            latest_observed_at: Some("2026-01-01 10:00:00".to_owned()),
        };

        let output = install_status_json(&status);
        assert!(output.contains("\"state\": \"stalled\""));
        assert!(output.contains("\"schema\": 2"));
        assert!(output.contains("\"manifest_size_on_disk\": 0"));
        assert!(output.contains("\"installed_payload_directory_present\": false"));
        assert!(output.contains("\"installed_payload_directory_is_symlink\": false"));
        assert!(output.contains("\"installed_payload_entry_observed\": false"));
        assert!(output.contains("\"name\": \"Fixture \\\"Game\\\"\""));
        assert!(output.contains("\"selected_depots\": [8,9]"));
        assert!(output.contains("\"attempt_start_downloaded_bytes\": 40"));
        assert!(!output.contains("not-emitted"));
    }

    #[test]
    fn interactive_steam_rejects_ephemeral_account_roots_unless_explicit() {
        for root in [
            "/tmp/macroton",
            "/private/tmp/macroton",
            "/var/folders/fixture/T/macroton",
            "/private/var/folders/fixture/T/macroton",
        ] {
            let error =
                require_persistent_data_root(Path::new(root), false, "fixture login").unwrap_err();
            assert!(error.contains("ephemeral data root"));
            require_persistent_data_root(Path::new(root), true, "fixture login").unwrap();
        }
        require_persistent_data_root(
            Path::new("/Users/fixture/Library/Application Support/Macroton"),
            false,
            "fixture login",
        )
        .unwrap();
        assert!(
            require_persistent_data_root(Path::new("relative-data"), false, "fixture login")
                .unwrap_err()
                .contains("absolute persistent")
        );
    }

    #[test]
    fn login_acceptance_requires_all_independent_signals() {
        use steam_prefix_status::Readiness;

        assert!(steam_login_acceptance_observed(
            Readiness::StoredLoginPresent,
            1,
            true,
            true,
            true
        ));
        for observation in [
            (Readiness::SignInRequired, 1, true, true, true),
            (Readiness::StoredLoginPresent, 0, true, true, true),
            (Readiness::StoredLoginPresent, 1, false, true, true),
            (Readiness::StoredLoginPresent, 1, true, false, true),
            (Readiness::StoredLoginPresent, 1, true, true, false),
        ] {
            assert!(!steam_login_acceptance_observed(
                observation.0,
                observation.1,
                observation.2,
                observation.3,
                observation.4
            ));
        }
    }

    #[test]
    fn persisted_login_acceptance_allows_client_exit_only_with_ui_evidence() {
        use steam_prefix_status::Readiness;

        assert!(steam_persisted_login_acceptance_observed(
            Readiness::StoredLoginPresent,
            1,
            true,
            true,
            true
        ));
        for observation in [
            (Readiness::SignInRequired, 1, true, true, true),
            (Readiness::StoredLoginPresent, 0, true, true, true),
            (Readiness::StoredLoginPresent, 1, false, true, true),
            (Readiness::StoredLoginPresent, 1, true, false, true),
            (Readiness::StoredLoginPresent, 1, true, true, false),
        ] {
            assert!(!steam_persisted_login_acceptance_observed(
                observation.0,
                observation.1,
                observation.2,
                observation.3,
                observation.4
            ));
        }
    }

    #[test]
    fn install_auth_gate_requires_connected_authenticated_live_primary_client() {
        let mut connection = steam_connection_status::ConnectionStatus {
            app_id: 42,
            steam_root: PathBuf::from("/not-emitted"),
            log_path: PathBuf::from("/not-emitted/connection_log.txt"),
            state: steam_connection_status::TransportState::Connected,
            latest_timestamp: Some("2026-08-13 10:00:00".to_owned()),
            synthetic_fake_ip: Some(true),
            authenticated_context: true,
            ui_login_observed: true,
            auth_poll_transport_error: false,
        };
        let mut resources = stop::PrefixResourceUsage {
            prefix: PathBuf::from("/not-emitted"),
            pids: vec![9],
            total_rss_kib: 100,
            processes: vec![stop::ProcessResourceUsage {
                pid: 9,
                rss_kib: 100,
                command: "Steam.exe".to_owned(),
                category: "steam-client",
            }],
        };
        assert!(live_steam_authentication_observed(&connection, &resources));

        connection.authenticated_context = false;
        assert!(!live_steam_authentication_observed(&connection, &resources));
        connection.authenticated_context = true;
        connection.state = steam_connection_status::TransportState::Disconnected;
        assert!(!live_steam_authentication_observed(&connection, &resources));
        connection.state = steam_connection_status::TransportState::Connected;
        resources.processes[0].command = "steamwebhelper.exe".to_owned();
        resources.processes[0].category = "steam-cef";
        assert!(!live_steam_authentication_observed(&connection, &resources));
    }

    #[test]
    fn cloud_launch_gate_requires_no_pending_client_operations() {
        use cloud_status::RemoteFreshness as R;
        assert!(cloud_launch_ready(cloud_status::SyncState::Synced, 0, R::Unverified));
        assert!(!cloud_launch_ready(cloud_status::SyncState::Synced, 1, R::Unverified));
        assert!(!cloud_launch_ready(cloud_status::SyncState::Syncing, 0, R::Unverified));
        assert!(!cloud_launch_ready(cloud_status::SyncState::Synced, 0, R::PendingOtherSession));
        assert!(!cloud_launch_ready(cloud_status::SyncState::Synced, 0, R::BypassedPendingSession));
    }

    #[test]
    fn cross_game_handoff_requires_a_post_exit_cloud_event() {
        let mut status = cloud_status::CloudStatus {
            app_id: 400,
            prefix: PathBuf::new(),
            log_path: PathBuf::new(),
            state: cloud_status::SyncState::Synced,
            latest_event: Some("[2026-10-06 09:58:50] [AppID 400] Successfully synced".into()),
            pending_remote_operations: Vec::new(),
            remote_freshness: cloud_status::RemoteFreshness::Unverified,
            accounts: Vec::new(),
            comparisons: Vec::new(),
        };
        let exit = "[2026-10-06 10:06:39] AppID 400 no longer tracking PID 956, exit code 0";
        assert!(!cloud_handoff_ready(&status, exit));
        status.latest_event = Some("[2026-10-06 10:06:41] [AppID 400] Upload complete, result OK".into());
        assert!(cloud_handoff_ready(&status, exit));
        status.remote_freshness = cloud_status::RemoteFreshness::BypassedPendingSession;
        assert!(!cloud_handoff_ready(&status, exit));
    }

    #[test]
    fn cloud_handoff_marker_identifies_the_exact_launched_profile() {
        let root = std::env::temp_dir().join(format!(
            "macroton-cloud-handoff-marker-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(root.join("games/42")).unwrap();
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=test\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\n"
        ).unwrap();
        record_cloud_handoff(42, &root, &profile).unwrap();
        assert_eq!(pending_cloud_handoff_fingerprint(42, &root).unwrap(), Some(profile.fingerprint()));
        assert_eq!(pending_cloud_handoff_fingerprint(43, &root).unwrap(), None);
        fs::write(cloud_handoff_marker(42, &root), "schema=1\nappid=43\nprofile_sha256=bad\n").unwrap();
        assert!(pending_cloud_handoff_fingerprint(42, &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shared_game_marker_blocks_separate_client_switch_until_cloud_finish() {
        let root = std::env::temp_dir().join(format!(
            "macroton-shared-marker-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let profile = CompatibilityProfile::parse(
            "schema=1\nappid=42\nruntime=test\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\n"
        ).unwrap();
        fs::create_dir_all(&root).unwrap();
        let profile_path = root.join("42.conf");
        fs::write(&profile_path, "schema=1\nappid=42\nruntime=test\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\n").unwrap();
        record_shared_client_marker(42, 100, &root, &profile, &profile_path).unwrap();
        assert_eq!(read_shared_client_marker(42, &root).unwrap(), Some((100, profile.fingerprint())));
        assert_eq!(handoff_profile(42, &root, Some(&profile.fingerprint())).unwrap(), profile_path.canonicalize().unwrap());
        assert!(record_shared_client_marker(42, 101, &root, &profile, &profile_path).is_err());
        assert!(handoff_other_steam_clients(43, &root, Path::new("/unused"))
            .unwrap_err().contains("shared Steam session"));
        let lock = SharedClientLock::acquire(100, &root).unwrap();
        assert!(SharedClientLock::acquire(101, &root).is_err());
        drop(lock);
        assert!(SharedClientLock::acquire(101, &root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cloud_status_json_preserves_pending_operations_and_escapes_paths() {
        let status = cloud_status::CloudStatus {
            app_id: 26800,
            prefix: PathBuf::from("/tmp/prefix"),
            log_path: PathBuf::from("/tmp/cloud_log.txt"),
            state: cloud_status::SyncState::Synced,
            latest_event: None,
            pending_remote_operations: vec!["Operation '3' from machine \"deck\"".to_owned()],
            remote_freshness: cloud_status::RemoteFreshness::BypassedPendingSession,
            accounts: Vec::new(),
            comparisons: vec![cloud_status::CloudFileComparison {
                account_id: "123".to_owned(),
                remote_path: "save/\\\"slot.sav".to_owned(),
                local_path: PathBuf::from("/tmp/save"),
                remote_copy_path: PathBuf::from("/tmp/remote"),
                state: cloud_status::ComparisonState::Match,
                cached_size: None,
                local_size: None,
                remote_copy_size: None,
                remote_content_size: None,
                cached_sha1: None,
                local_sha1: None,
                remote_copy_sha1: None,
                remote_content_sha1: None,
                remote_copy_encoding: None,
            }],
        };
        let json = cloud_status_json(&status);
        assert!(json.contains("\"latest_event\":null"));
        assert!(json.contains("\"launch_ready\":false"));
        assert!(json.contains(
            "\"pending_remote_operations\":[\"Operation '3' from machine \\\"deck\\\"\"]"
        ));
        assert!(json.contains("\"remote_path\":\"save/\\\\\\\"slot.sav\""));

        let mut status = status;
        status.latest_event = Some("[2026-09-12 15:04:37] [AppID 26800] Currently already synced".to_owned());
        let json = cloud_status_json(&status);
        assert!(json.contains("\"latest_event\":\"[2026-09-12 15:04:37] [AppID 26800] Currently already synced\""));
    }

    #[test]
    fn duplicate_launch_guard_identifies_existing_windows_game() {
        let resources = stop::PrefixResourceUsage {
            prefix: PathBuf::from("/tmp/macroton-prefix"),
            pids: vec![23192],
            total_rss_kib: 1024,
            processes: vec![stop::ProcessResourceUsage {
                pid: 23192,
                rss_kib: 1024,
                command: "silver.exe".to_owned(),
                category: "windows-game",
            }],
        };
        assert_eq!(
            existing_windows_game_command(&resources),
            Some("silver.exe")
        );
    }

    #[test]
    fn steam_port_warning_is_advisory_and_does_not_offer_process_cleanup() {
        let error = io::Error::new(io::ErrorKind::AddrInUse, "address already in use");
        let warning = format_steam_port_warning(&error);
        assert!(warning.contains("TCP 27036"));
        assert!(warning.contains("Macroton will not terminate"));
        assert!(warning.contains("close the conflicting client"));
    }

    #[test]
    fn steam_port_warning_labels_exact_prefix_ownership() {
        let warning = "TCP 27036 is already occupied";
        let scoped = format_steam_port_warning_for_scope(warning, true);
        assert!(scoped.contains("this AppID's existing Wine prefix"));
        assert!(scoped.contains("will not start a second client"));
        assert_eq!(format_steam_port_warning_for_scope(warning, false), warning);
    }

    #[test]
    fn profile_validation_checks_content_and_appid_filename() {
        let root = std::env::temp_dir().join(format!("macroton-profile-validation-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let valid = root.join("42.conf");
        fs::write(&valid, "schema=1\nappid=42\nruntime=wine\nruntime_version=1\nrenderer=auto\nexecutable=game.exe\n").unwrap();
        assert!(profile_command(&["validate".to_owned(), valid.display().to_string()]).is_ok());
        let wrong_name = root.join("43.conf");
        fs::copy(&valid, &wrong_name).unwrap();
        assert!(profile_command(&["validate".to_owned(), wrong_name.display().to_string()]).unwrap_err().contains("filename"));
        fs::write(&valid, "schema=1\nappid=42\nexecutable=../game.exe\n").unwrap();
        assert!(profile_command(&["validate".to_owned(), valid.display().to_string()]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
