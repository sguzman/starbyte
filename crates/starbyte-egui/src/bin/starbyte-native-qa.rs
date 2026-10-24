//! Unattended native-Hyprland adjuncts for the opt-in original-window worker.
//!
//! This runner uses isolated demo/config/cache artifacts on the *local* host.
//! Passing these synthetic probes is not physical speaker, gamepad, workspace
//! switching or production-default acceptance.

use std::{
    env,
    error::Error,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Probe {
    name: &'static str,
    args: &'static [&'static str],
    markers: &'static [&'static str],
}

const PROBES: &[Probe] = &[
    Probe {
        name: "worker-window",
        args: &["--exit-after-frames", "30"],
        markers: &[
            "STARBYTE_FULL_LIBRARY_WORKER_SMOKE_OK frame=",
            "STARBYTE_FULL_LIBRARY_SINGLE_OWNER_SMOKE_OK legacy_session_absent=true",
        ],
    },
    Probe {
        name: "save-and-restore",
        args: &["--exit-after-full-worker-saves"],
        markers: &["STARBYTE_FULL_LIBRARY_SAVE_SMOKE_OK frame="],
    },
    Probe {
        name: "no-repaint-pacing",
        args: &["--exit-after-full-worker-no-redraw"],
        markers: &[
            "STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=",
            "STARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_OK advanced_frames=",
        ],
    },
    Probe {
        name: "focus-pause",
        args: &["--exit-after-full-worker-focus-pause"],
        markers: &["STARBYTE_FULL_LIBRARY_FOCUS_PAUSE_SMOKE_OK paused_frames="],
    },
    Probe {
        name: "worker-exit",
        args: &["--exit-after-full-worker-panic"],
        markers: &["STARBYTE_FULL_LIBRARY_WORKER_EXIT_SMOKE_OK playback_stopped=true"],
    },
    Probe {
        name: "sram-shutdown",
        args: &["--exit-after-full-worker-sram"],
        markers: &[
            "STARBYTE_FULL_LIBRARY_SRAM_SMOKE_OK persisted_byte=5A",
            "STARBYTE_FULL_LIBRARY_OWNER_SHUTDOWN_SMOKE_OK finalized=true audio_released=true",
        ],
    },
    Probe {
        name: "rom-replacement",
        args: &["--exit-after-full-worker-replacement"],
        markers: &[
            "STARBYTE_FULL_LIBRARY_REPLACEMENT_SMOKE_OK retained_rom=true advanced_frames=",
            "STARBYTE_FULL_LIBRARY_RECEIPT_RACE_SMOKE_OK retained_rom=true screenshot_written=true step_uploaded=true pending_step_cleared=true",
            "STARBYTE_FULL_LIBRARY_SUPERSEDED_INSTALL_SMOKE_OK active_owner_synced=true newer_failure_reported=true",
        ],
    },
    Probe {
        name: "original-window-shortcuts",
        args: &["--exit-after-full-worker-shortcuts"],
        markers: &[
            "STARBYTE_FULL_LIBRARY_SHORTCUT_SMOKE_OK f5=true f8=true f9=true escape=true",
            "STARBYTE_FULL_LIBRARY_SHELL_SMOKE_OK menu=true recent_cards=true cover_texture=true library=true status_metrics=true view_toggles=true",
        ],
    },
    Probe {
        name: "sound-mode-slots",
        args: &["--exit-after-full-worker-modes"],
        markers: &[
            "STARBYTE_FULL_LIBRARY_MODE_SMOKE_OK persistent_slot=true compatibility_slot=true separate_slots=true",
        ],
    },
];

const GAP_MARKER: &str = "STARBYTE_FULL_LIBRARY_NO_REPAINT_GAP_BEGIN frame=";
const PHYSICAL_WORKSPACE_PROBE: Probe = Probe {
    name: "real-workspace-progress",
    args: &["--exit-after-full-worker-no-redraw"],
    markers: &[
        "STARBYTE_FULL_LIBRARY_NO_REPAINT_GAP_BEGIN frame=",
        "STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=",
        "STARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_OK advanced_frames=",
    ],
};

fn missing_markers<'a>(text: &str, markers: &'a [&'a str]) -> Vec<&'a str> {
    markers
        .iter()
        .copied()
        .filter(|marker| !text.contains(marker))
        .collect()
}

fn require_hyprland() -> Result<(), Box<dyn Error>> {
    for key in ["WAYLAND_DISPLAY", "HYPRLAND_INSTANCE_SIGNATURE"] {
        if env::var(key).map_or(true, |value| value.is_empty()) {
            return Err(format!("{key} is missing; a real Hyprland session is required").into());
        }
    }
    if !env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland") {
        return Err("XDG_SESSION_TYPE must be wayland; XWayland is not acceptance".into());
    }
    Ok(())
}

fn capture_host(root: &Path) -> Result<(), Box<dyn Error>> {
    for (name, args) in [
        ("monitors", &["-j", "monitors"][..]),
        ("workspaces", &["-j", "workspaces"][..]),
        ("activeworkspace", &["-j", "activeworkspace"][..]),
        ("version", &["version"][..]),
    ] {
        let output = Command::new("hyprctl").args(args).output()?;
        if !output.status.success() || output.stdout.is_empty() {
            return Err(format!("hyprctl {name} failed on the active compositor").into());
        }
        fs::write(root.join(format!("hyprctl-{name}.txt")), output.stdout)?;
    }
    let mut context = File::create(root.join("context.txt"))?;
    writeln!(context, "probe_kind=synthetic_native_adjunct_only")?;
    writeln!(context, "accepted_physical_tasks=none")?;
    for key in [
        "XDG_SESSION_TYPE",
        "WAYLAND_DISPLAY",
        "HYPRLAND_INSTANCE_SIGNATURE",
    ] {
        writeln!(context, "{key}={}", env::var(key).unwrap_or_default())?;
    }
    if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output()
        && output.status.success()
    {
        writeln!(
            context,
            "git_commit_of_working_directory={}",
            String::from_utf8_lossy(&output.stdout).trim()
        )?;
    }
    Ok(())
}

fn isolated_probe_command(
    binary: &Path,
    probe_dir: &Path,
    probe: &Probe,
) -> std::io::Result<Command> {
    let cache = probe_dir.join("cache");
    let xdg_config = probe_dir.join("xdg-config");
    let xdg_cache = probe_dir.join("xdg-cache");
    let xdg_data = probe_dir.join("xdg-data");
    // Startup searches HOME even when --config and --cache-dir are passed.
    // Do not enumerate the real user's game library in a synthetic QA run.
    let isolated_home = probe_dir.join("home");
    for dir in [&cache, &xdg_config, &xdg_cache, &xdg_data, &isolated_home] {
        fs::create_dir_all(dir)?;
    }
    let mut command = Command::new(binary);
    command
        .args(["--demo", "--native-worker-preview"])
        .args(probe.args)
        .arg("--config")
        .arg(probe_dir.join("settings.toml"))
        .arg("--cache-dir")
        .arg(&cache)
        // Disallow an accidental XWayland/X11 fallback. The launcher
        // inherits the compositor's real Wayland socket but no X display.
        .env_remove("DISPLAY")
        .env("WINIT_UNIX_BACKEND", "wayland")
        .env("HOME", &isolated_home)
        .env("XDG_CONFIG_HOME", &xdg_config)
        .env("XDG_CACHE_HOME", &xdg_cache)
        .env("XDG_DATA_HOME", &xdg_data);
    Ok(command)
}

/// Detect the actual timestamp published by the original window when its
/// callback gap started. A stale marker must never authorize a physical
/// workspace switch or a falsely attributed pacing receipt.
fn gap_age_at(log: &[u8], now: SystemTime) -> Option<Duration> {
    let line = String::from_utf8_lossy(log)
        .lines()
        .find(|line| line.contains(GAP_MARKER))?
        .to_owned();
    let millis = line
        .split_whitespace()
        .find_map(|word| word.strip_prefix("started_unix_ms="))?
        .parse::<u64>()
        .ok()?;
    let began = UNIX_EPOCH.checked_add(Duration::from_millis(millis))?;
    now.duration_since(began).ok()
}

fn hyprland_json(command: &str) -> Result<serde_json::Value, Box<dyn Error>> {
    let output = Command::new("hyprctl").args(["-j", command]).output()?;
    if !output.status.success() {
        return Err(format!("hyprctl {command} failed").into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// Avoid interpolating any user-controlled workspace name into Lua source.
/// Both the QA workspace and the original numeric workspace are generated here.
fn dispatch_workspace(target: &str) -> Result<(), Box<dyn Error>> {
    if target.is_empty()
        || !target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':')
    {
        return Err("unsafe workspace selector".into());
    }
    let lua = format!("hl.dsp.focus({{ workspace = \"{target}\" }})");
    let output = Command::new("hyprctl").args(["dispatch", &lua]).output()?;
    if !output.status.success() {
        return Err(format!(
            "Hyprland workspace dispatcher failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

fn workspace_id(value: &serde_json::Value) -> Option<i64> {
    value.get("id")?.as_i64().filter(|id| *id > 0)
}

fn native_client_on_workspace(clients: &serde_json::Value, pid: u32, workspace: i64) -> bool {
    clients.as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry.get("pid").and_then(serde_json::Value::as_u64) == Some(u64::from(pid))
                && entry.get("xwayland").and_then(serde_json::Value::as_bool) == Some(false)
                && entry
                    .get("workspace")
                    .and_then(|ws| ws.get("id"))
                    .and_then(serde_json::Value::as_i64)
                    == Some(workspace)
        })
    })
}

struct RestoreWorkspace(i64);

impl Drop for RestoreWorkspace {
    fn drop(&mut self) {
        // Workspace restoration must occur even after a failed probe, not
        // leave the QA operator's actual desktop on the empty test workspace.
        if let Err(error) = dispatch_workspace(&self.0.to_string()) {
            eprintln!("Could not restore previous Hyprland workspace: {error}");
        }
    }
}

fn exercise_real_workspace(
    pid: u32,
    probe_dir: &Path,
    gap_started: SystemTime,
) -> Result<(), Box<dyn Error>> {
    let original = workspace_id(&hyprland_json("activeworkspace")?)
        .ok_or("native workspace QA needs a regular positive numeric workspace")?;
    if !native_client_on_workspace(&hyprland_json("clients")?, pid, original) {
        return Err("test window is not a native Wayland client on the active workspace".into());
    }
    let name = format!("starbyte-qa-{pid}");
    let target = format!("name:{name}");
    let restore = RestoreWorkspace(original);
    dispatch_workspace(&target)?;
    let active = hyprland_json("activeworkspace")?;
    let observed_name = active.get("name").and_then(serde_json::Value::as_str);
    // Some Hyprland versions return named workspaces with a prefix.
    let switched = (observed_name == Some(name.as_str()) || observed_name == Some(target.as_str()))
        && active.get("id").and_then(serde_json::Value::as_i64) != Some(original);
    if !switched {
        return Err("Hyprland did not switch to the isolated QA workspace".into());
    }
    // A process observed late could already have finished its two-second
    // callback gap before the physical switch actually happened.
    if SystemTime::now()
        .duration_since(gap_started)
        .map_or(true, |age| age > Duration::from_millis(1_200))
    {
        return Err("workspace switch began too late for the measured callback gap".into());
    }
    // Sample the compositor's *actual* active workspace throughout the
    // interval; one successful switch followed by an early return is not
    // evidence of sustained off-workspace scheduling.
    let off_workspace_at = Instant::now();
    let mut samples = 0_u32;
    while off_workspace_at.elapsed() < Duration::from_millis(2_250) {
        let observed = hyprland_json("activeworkspace")?;
        let name_now = observed.get("name").and_then(serde_json::Value::as_str);
        if (name_now != Some(name.as_str()) && name_now != Some(target.as_str()))
            || observed.get("id").and_then(serde_json::Value::as_i64) == Some(original)
        {
            return Err("Hyprland left the QA workspace during the timed gap".into());
        }
        samples += 1;
        thread::sleep(Duration::from_millis(250));
    }
    if samples < 5 {
        return Err("too few compositor samples to substantiate off-workspace duration".into());
    }
    let observed_ms = off_workspace_at.elapsed().as_millis();
    // A compositor rule could move the game window along with the newly
    // focused workspace. That would be a false off-workspace success.
    if !native_client_on_workspace(&hyprland_json("clients")?, pid, original) {
        return Err("Starbyte did not remain on its original workspace".into());
    }
    drop(restore);
    if workspace_id(&hyprland_json("activeworkspace")?) != Some(original) {
        return Err("Hyprland did not return to the original workspace".into());
    }
    // The local QA operator owns this evidence; do not expose other clients'
    // titles, personal file paths, serial numbers, or full compositor state.
    let receipt = serde_json::json!({
        "kind": "physical_workspace_switch_observation",
        "original_workspace": original,
        "temporary_workspace": name,
        "starbyte_process_id": pid,
        "starbyte_native_wayland_client": true,
        "left_original_workspace": true,
        "starbyte_remained_on_original_workspace": true,
        "returned_to_original_workspace": true,
        "off_workspace_observed_ms": observed_ms,
        "workspace_observation_samples": samples,
        "speaker_audibility": "not_tested",
        "gamepad_input": "not_tested",
        "production_cutover": "not_tested"
    });
    fs::write(
        probe_dir.join("workspace-transition.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(())
}

fn run_probe(
    binary: &Path,
    root: &Path,
    probe: &Probe,
    physical_workspace: bool,
) -> Result<(bool, u128, Vec<&'static str>), Box<dyn Error>> {
    let probe_dir = root.join(probe.name);
    fs::create_dir(&probe_dir)?;
    let log_path = probe_dir.join("starbyte.log");
    let log = File::create(&log_path)?;
    let mut command = isolated_probe_command(binary, &probe_dir, probe)?;
    let mut child = command
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    let started = Instant::now();
    let timeout = Duration::from_secs(90);
    let mut switched_workspace = false;
    let exited = loop {
        if let Some(status) = child.try_wait()? {
            break status.success();
        }
        // The process must still be alive when the physical switch starts.
        // Avoid a false pass when a successful marker was printed *after*
        // the child had already exited without ever being off-workspace.
        if physical_workspace && !switched_workspace {
            let log_bytes = fs::read(&log_path)?;
            if String::from_utf8_lossy(&log_bytes).contains(GAP_MARKER) {
                let now = SystemTime::now();
                let gap_age = gap_age_at(&log_bytes, now);
                let fresh_gap = gap_age.is_some_and(|age| age <= Duration::from_millis(900));
                let result = if fresh_gap {
                    exercise_real_workspace(
                        child.id(),
                        &probe_dir,
                        now - gap_age.unwrap_or_default(),
                    )
                } else {
                    Err("no valid fresh timestamp on the original-window gap marker".into())
                };
                if let Err(error) = result {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
                switched_workspace = true;
            }
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            child.wait()?;
            break false;
        }
        thread::sleep(Duration::from_millis(100));
    };
    let output = fs::read_to_string(log_path)?;
    let missing = missing_markers(&output, probe.markers);
    Ok((
        exited && missing.is_empty() && (!physical_workspace || switched_workspace),
        started.elapsed().as_millis(),
        missing,
    ))
}

fn run() -> Result<(), Box<dyn Error>> {
    let executable = env::current_exe()?;
    let default_binary = executable.with_file_name(if cfg!(windows) {
        "starbyte-egui.exe"
    } else {
        "starbyte-egui"
    });
    let mut binary = default_binary;
    let mut args = env::args().skip(1);
    let mut physical_workspace = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--binary" => binary = PathBuf::from(args.next().ok_or("--binary needs a path")?),
            "--physical-workspace" => physical_workspace = true,
            "--help" | "-h" => {
                println!("Usage: starbyte-native-qa [--binary PATH] [--physical-workspace]");
                println!("Run isolated demo smoke probes on native Hyprland.");
                println!("--physical-workspace also switches to a temporary real workspace");
                println!("and restores the original workspace after measuring worker pacing.");
                return Ok(());
            }
            _ => return Err(format!("unrecognized option: {arg}").into()),
        }
    }
    require_hyprland()?;
    if !binary.is_file() {
        return Err(format!(
            "Starbyte binary missing at {}; the QA operator must build --bins first",
            binary.display()
        )
        .into());
    }
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let root = env::temp_dir().join(format!(
        "starbyte-hyprland-qa-{}-{nonce}",
        std::process::id()
    ));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&root)?;
    }
    #[cfg(not(unix))]
    fs::create_dir(&root)?;
    capture_host(&root)?;
    let mut report = File::create(root.join("results.tsv"))?;
    writeln!(report, "probe\tresult\telapsed_ms\tmissing_markers")?;
    let mut passed = 0;
    let mut total = 0;
    let supplemental = physical_workspace.then_some(&PHYSICAL_WORKSPACE_PROBE);
    for probe in PROBES.iter().chain(supplemental) {
        total += 1;
        let result = run_probe(
            &binary,
            &root,
            probe,
            physical_workspace && probe.name == PHYSICAL_WORKSPACE_PROBE.name,
        );
        let (ok, elapsed, missing) = match result {
            Ok((ok, elapsed, missing)) => (ok, elapsed, missing.join(" | ")),
            Err(error) => (false, 0, format!("probe error: {error}")),
        };
        writeln!(
            report,
            "{}\t{}\t{}\t{}",
            probe.name,
            if ok { "pass" } else { "fail" },
            elapsed,
            missing
        )?;
        if ok {
            passed += 1;
        }
    }
    report.flush()?;
    println!(
        "Native Hyprland smoke probes: {passed}/{total} (real workspace opt-in: {physical_workspace})."
    );
    println!("Local-only logs and context: {}", root.display());
    if physical_workspace {
        println!("Real workspace switch was requested and checked; review local receipt.");
    }
    println!("Physical speaker, gamepad, device recovery and production acceptance: NOT TESTED.");
    if passed != total {
        return Err("one or more native probes failed; inspect local logs".into());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("starbyte-native-qa: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{PROBES, gap_age_at, missing_markers, native_client_on_workspace, workspace_id};

    #[test]
    fn only_explicit_success_markers_pass_a_probe() {
        let markers = &["STARBYTE_FIRST_OK", "STARBYTE_SECOND_OK"];
        assert_eq!(
            missing_markers("STARBYTE_FIRST_OK", markers),
            vec!["STARBYTE_SECOND_OK"]
        );
        assert!(missing_markers("STARBYTE_FIRST_OK\nSTARBYTE_SECOND_OK", markers).is_empty());
        assert_eq!(PROBES.len(), 9);
        assert!(PROBES.iter().all(|p| !p.markers.is_empty()));
    }

    #[test]
    fn native_runner_isolates_home_as_well_as_xdg_directories() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("synthetic");
        let command =
            super::isolated_probe_command(std::path::Path::new("starbyte-egui"), &dir, &PROBES[0])
                .unwrap();
        let configured: std::collections::HashMap<_, _> = command
            .get_envs()
            .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
            .collect();
        for (key, path) in [
            ("HOME", dir.join("home")),
            ("XDG_CONFIG_HOME", dir.join("xdg-config")),
            ("XDG_CACHE_HOME", dir.join("xdg-cache")),
            ("XDG_DATA_HOME", dir.join("xdg-data")),
        ] {
            assert_eq!(configured.get(key).copied(), path.to_str());
            assert!(path.is_dir(), "{key} needs an isolated directory");
        }
        assert_eq!(configured.get("WINIT_UNIX_BACKEND"), Some(&"wayland"));
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "DISPLAY" && value.is_none()),
            "a native Wayland smoke must not fall back to X11/XWayland"
        );
    }

    #[test]
    fn physical_gap_marker_is_fresh_only_with_a_valid_timestamp() {
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_millis(4_000);
        let line = b"STARBYTE_FULL_LIBRARY_NO_REPAINT_GAP_BEGIN frame=42 started_unix_ms=3500";
        assert_eq!(
            gap_age_at(line, now),
            Some(std::time::Duration::from_millis(500))
        );
        assert!(gap_age_at(line, std::time::UNIX_EPOCH).is_none());
        assert!(gap_age_at(b"STARBYTE_FULL_LIBRARY_NO_REPAINT_GAP_BEGIN frame=42", now).is_none());
        assert!(gap_age_at(b"STARBYTE_FULL_LIBRARY_NO_REPAINT_GAP_BEGIN frame=42 started_unix_ms=bad", now).is_none());
    }

    #[test]
    fn physical_workspace_receipts_require_a_native_client_and_positive_workspace() {
        assert_eq!(
            workspace_id(&serde_json::json!({"id": 6, "name": "6"})),
            Some(6)
        );
        assert_eq!(workspace_id(&serde_json::json!({"id": -99})), None);
        let clients = serde_json::json!([
            {"pid": 4321, "xwayland": false, "workspace": {"id": 6}},
            {"pid": 5432, "xwayland": true, "workspace": {"id": 6}}
        ]);
        assert!(native_client_on_workspace(&clients, 4321, 6));
        assert!(!native_client_on_workspace(&clients, 5432, 6));
        assert!(!native_client_on_workspace(&clients, 4321, 5));
    }

    #[test]
    fn native_probes_require_real_ci_smoke_contract_markers() {
        let ci = std::fs::read_to_string(format!(
            "{}/../../.github/workflows/ci.yml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        for probe in PROBES
            .iter()
            .chain(std::iter::once(&super::PHYSICAL_WORKSPACE_PROBE))
        {
            for marker in probe.markers {
                assert!(
                    ci.contains(marker),
                    "{} marker is not in the original-window CI contract: {marker}",
                    probe.name
                );
            }
        }
    }
}
