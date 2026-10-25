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
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Probe {
    name: &'static str,
    args: &'static [&'static str],
    markers: &'static [&'static str],
}

/// Child processes are not killed by std::process::Child::drop. A physical
/// compositor inspection error must not leak a running emulator into the
/// owner's actual Hyprland workspace.
struct KillOnDrop(Child);

impl Deref for KillOnDrop {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for KillOnDrop {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
const PAUSE_GAP_MARKER: &str = "STARBYTE_FULL_LIBRARY_REAL_PAUSE_BEGIN frame=";
const PHYSICAL_WORKSPACE_PROBE: Probe = Probe {
    name: "real-workspace-progress",
    args: &["--exit-after-full-worker-no-redraw"],
    markers: &[
        "STARBYTE_FULL_LIBRARY_NO_REPAINT_GAP_BEGIN frame=",
        "STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK advanced_frames=",
        "STARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_OK advanced_frames=",
    ],
};

const PHYSICAL_FOCUSED_PROBE: Probe = Probe {
    name: "real-focused-clock",
    args: &["--exit-after-full-worker-focused-clock"],
    markers: &[
        "STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_BEGIN frame=",
        "STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_OK fps=",
    ],
};

const PHYSICAL_PAUSE_PROBE: Probe = Probe {
    name: "real-background-pause",
    args: &["--exit-after-full-worker-physical-pause"],
    markers: &[
        "STARBYTE_FULL_LIBRARY_REAL_PAUSE_BEGIN frame=",
        "STARBYTE_FULL_LIBRARY_REAL_PAUSE_SMOKE_OK advanced_away=",
    ],
};

fn missing_markers<'a>(text: &str, markers: &'a [&'a str]) -> Vec<&'a str> {
    markers
        .iter()
        .copied()
        .filter(|marker| !text.contains(marker))
        .collect()
}

/// Derive a bounded, reviewable focused-window receipt from the original
/// application's actual owner frame and texture counters, never from a
/// synthetic frame estimate or the display refresh rate.
fn focused_clock_receipt(
    log: &str,
    compositor_samples: u32,
    pid: u32,
) -> Option<serde_json::Value> {
    let line = log
        .lines()
        .find(|line| line.starts_with("STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_OK "))?;
    let get = |prefix: &str| {
        line.split_whitespace()
            .find_map(|word| word.strip_prefix(prefix))
    };
    let fps = get("fps=")?.parse::<f64>().ok()?;
    let callbacks = get("callbacks=")?.parse::<u64>().ok()?;
    let presented = get("presented=")?.parse::<u64>().ok()?;
    let frames = get("frames=")?.parse::<u64>().ok()?;
    let wall_ms = get("wall_ms=")?.parse::<u64>().ok()?;
    // The application prints its rounded FPS alongside the *actual* frame
    // delta and monotonic duration. Never accept a physically focused receipt
    // that claims ~60 FPS while reporting missing or contradictory frames.
    let derived_fps = frames as f64 * 1_000.0 / wall_ms.max(1) as f64;
    if !fps.is_finite()
        || !(55.0..=65.0).contains(&fps)
        || (fps - derived_fps).abs() > 0.5
        || compositor_samples < 10
        || callbacks < 30
        || presented < 90
        || frames < 90
        || wall_ms < 3_000
    {
        return None;
    }
    Some(serde_json::json!({
        "kind": "physical_focused_clock_observation",
        "starbyte_process_id": pid,
        "native_wayland_client": true,
        "compositor_active_window_observation_samples": compositor_samples,
        "window_retained_keyboard_focus": true,
        "emulator_worker_fps": fps,
        "egui_callback_count": callbacks,
        "presented_texture_frame_delta": presented,
        "completed_worker_frames": frames,
        "wall_clock_ms": wall_ms,
        "speaker_audibility": "not_tested",
        "gamepad_input": "not_tested",
        "production_cutover": "not_tested"
    }))
}

/// Extract only numeric values from the exact original-window success marker.
/// Never publish arbitrary app log lines or compositor client metadata.
fn marker_metric<T: std::str::FromStr>(log: &str, marker: &str, key: &str) -> Option<T> {
    let line = log.lines().find(|line| line.starts_with(marker))?;
    line.split_whitespace()
        .find_map(|field| field.strip_prefix(key))?
        .parse()
        .ok()
}

/// A deliberately narrow public result. Full physical evidence, including
/// local window names, display IDs, device paths and raw logs, remains under
/// the private output root. This only contains aggregate timing and proof
/// counters sufficient for an agent to review the clock/pause gates.
fn sanitized_physical_receipt(root: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
    let focused: serde_json::Value = serde_json::from_slice(&fs::read(
        root.join("real-focused-clock/focused-clock-observation.json"),
    )?)?;
    let background: serde_json::Value = serde_json::from_slice(&fs::read(
        root.join("real-workspace-progress/workspace-transition.json"),
    )?)?;
    let paused: serde_json::Value = serde_json::from_slice(&fs::read(
        root.join("real-background-pause/workspace-transition.json"),
    )?)?;
    let bg_log = fs::read_to_string(root.join("real-workspace-progress/starbyte.log"))?;
    let pause_log = fs::read_to_string(root.join("real-background-pause/starbyte.log"))?;
    let bg_frames: u64 = marker_metric(
        &bg_log,
        "STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK ",
        "advanced_frames=",
    )
    .ok_or("no physical off-workspace progress receipt")?;
    let bg_uploaded_frame: u64 = marker_metric(
        &bg_log,
        "STARBYTE_FULL_LIBRARY_NO_REPAINT_SMOKE_OK ",
        "caught_up_frame=",
    )
    .ok_or("no post-workspace texture catchup receipt")?;
    let bg_fps: f64 = marker_metric(
        &bg_log,
        "STARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_OK ",
        "fps=",
    )
    .ok_or("no off-workspace owner-clock FPS receipt")?;
    if !bg_fps.is_finite() || !(48.0..=72.0).contains(&bg_fps) {
        return Err("invalid off-workspace frame cadence".into());
    }
    let pause_metric = |field| {
        marker_metric::<u64>(
            &pause_log,
            "STARBYTE_FULL_LIBRARY_REAL_PAUSE_SMOKE_OK ",
            field,
        )
    };
    let advanced_away = pause_metric("advanced_away=").ok_or("no paused frame count")?;
    let suspension_count = pause_metric("suspend_events=").ok_or("no pause event count")?;
    let resumed_count = pause_metric("resumed_events=").ok_or("no resume event count")?;
    let resumed_frames = pause_metric("resumed_frames=").ok_or("no resumed frames count")?;
    let resume_ms = pause_metric("resume_elapsed_ms=").ok_or("no resume elapsed time")?;
    if suspension_count == 0 || resumed_count == 0 || resumed_frames < 10 || resume_ms < 80 {
        return Err("insufficient physical pause/resume measurements".into());
    }
    for receipt in [&background, &paused] {
        if receipt["starbyte_native_wayland_client"] != true
            || receipt["returned_to_original_workspace"] != true
            || receipt["starbyte_remained_on_original_workspace"] != true
        {
            return Err("physical workspace transition evidence incomplete".into());
        }
    }
    if focused["native_wayland_client"] != true || focused["window_retained_keyboard_focus"] != true
    {
        return Err("focused window was not confirmed by Hyprland".into());
    }
    Ok(serde_json::json!({
        "schema_version": 1,
        "kind": "sanitized_physical_hyprland_clock_evidence",
        "original_window_native_wayland": true,
        "focused": {
            "worker_fps": focused["emulator_worker_fps"],
            "completed_worker_frames": focused["completed_worker_frames"],
            "texture_frames": focused["presented_texture_frame_delta"],
            "egui_callbacks": focused["egui_callback_count"],
            "wall_ms": focused["wall_clock_ms"]
        },
        "off_workspace": {
            "observed_ms": background["off_workspace_observed_ms"],
            "compositor_samples": background["workspace_observation_samples"],
            "completed_worker_frames": bg_frames,
            "owner_fps": bg_fps,
            "post_return_texture_frame": bg_uploaded_frame
        },
        "pause_on_blur": {
            "observed_ms": paused["off_workspace_observed_ms"],
            "compositor_samples": paused["workspace_observation_samples"],
            "advanced_while_away": advanced_away,
            "suspend_events": suspension_count,
            "resume_events": resumed_count,
            "resumed_frames": resumed_frames,
            "resume_elapsed_ms": resume_ms
        },
        "speaker_audibility": "not_tested",
        "physical_gamepad": "not_tested",
        "production_cutover": "not_tested",
        "accepted_gate_updates": "requires_evidence_review"
    }))
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
fn gap_age_at(log: &[u8], now: SystemTime, marker: &str) -> Option<Duration> {
    let line = String::from_utf8_lossy(log)
        .lines()
        .find(|line| line.contains(marker))?
        .to_owned();
    let millis = line
        .split_whitespace()
        .find_map(|word| word.strip_prefix("started_unix_ms="))?
        .parse::<u64>()
        .ok()?;
    let began = UNIX_EPOCH.checked_add(Duration::from_millis(millis))?;
    now.duration_since(began).ok()
}

fn fresh_gap_start_at(log: &[u8], now: SystemTime, marker: &str) -> Option<SystemTime> {
    let age = gap_age_at(log, now, marker)?;
    (age <= Duration::from_millis(900)).then(|| now - age)
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

/// If the wrapper reserved a dedicated QA workspace, never open another
/// test window after the user has moved focus elsewhere. Failing a probe is
/// preferable to interrupting the working desktop with a new window.
fn qa_workspace_matches(expected: &str, active: &serde_json::Value) -> bool {
    expected
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .is_some_and(|id| workspace_id(active) == Some(id))
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
        "probe": probe_dir.file_name().and_then(|name| name.to_str()).unwrap_or("unknown"),
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
    if let Ok(expected) = env::var("STARBYTE_QA_WORKSPACE_ID") {
        let active = hyprland_json("activeworkspace")?;
        if !qa_workspace_matches(&expected, &active) {
            return Err(
                "dedicated QA workspace no longer active; refusing to spawn a window".into(),
            );
        }
    }
    let log_path = probe_dir.join("starbyte.log");
    let log = File::create(&log_path)?;
    let mut command = isolated_probe_command(binary, &probe_dir, probe)?;
    let mut child = KillOnDrop(
        command
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .spawn()?,
    );
    let started = Instant::now();
    let timeout = Duration::from_secs(90);
    let mut switched_workspace = false;
    let mut focused_samples = 0_u32;
    let exited = loop {
        if let Some(status) = child.try_wait()? {
            break status.success();
        }
        // The foreground probe must continuously observe the actual
        // process on the native compositor's active workspace while its
        // original-window egui callbacks measure frames and texture updates.
        if probe.name == PHYSICAL_FOCUSED_PROBE.name {
            let log_bytes = fs::read(&log_path)?;
            if String::from_utf8_lossy(&log_bytes).contains(PHYSICAL_FOCUSED_PROBE.markers[0]) {
                let active = workspace_id(&hyprland_json("activeworkspace")?)
                    .ok_or("focused-clock QA requires a regular active workspace")?;
                let foreground_pid = hyprland_json("activewindow")?
                    .get("pid")
                    .and_then(serde_json::Value::as_u64);
                if foreground_pid != Some(u64::from(child.id()))
                    || !native_client_on_workspace(&hyprland_json("clients")?, child.id(), active)
                {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(
                        "focused QA window lost the native Hyprland foreground workspace".into(),
                    );
                }
                focused_samples += 1;
            }
        }
        // The process must still be alive when the physical switch starts.
        // Avoid a false pass when a successful marker was printed *after*
        // the child had already exited without ever being off-workspace.
        if physical_workspace && !switched_workspace {
            let log_bytes = fs::read(&log_path)?;
            let marker = if probe.name == PHYSICAL_PAUSE_PROBE.name {
                PAUSE_GAP_MARKER
            } else {
                GAP_MARKER
            };
            if String::from_utf8_lossy(&log_bytes).contains(marker) {
                let result = match fresh_gap_start_at(&log_bytes, SystemTime::now(), marker) {
                    Some(began) => exercise_real_workspace(child.id(), &probe_dir, began),
                    None => {
                        Err("no valid fresh timestamp on the original-window gap marker".into())
                    }
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
    let focused_observed = if probe.name == PHYSICAL_FOCUSED_PROBE.name {
        if let Some(receipt) = focused_clock_receipt(&output, focused_samples, child.id()) {
            fs::write(
                probe_dir.join("focused-clock-observation.json"),
                serde_json::to_vec_pretty(&receipt)?,
            )?;
            true
        } else {
            false
        }
    } else {
        true
    };
    Ok((
        exited
            && missing.is_empty()
            && (!physical_workspace || switched_workspace)
            && focused_observed,
        started.elapsed().as_millis(),
        missing,
    ))
}

// Fail closed until a truly independent compositor/seat can be proved.
// This is intentionally not controlled by an environment variable or by
// Codex setting an override on the owner's active desktop.
fn gui_probes_allowed() -> bool {
    false
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
    let mut only_probe: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--binary" => binary = PathBuf::from(args.next().ok_or("--binary needs a path")?),
            "--physical-workspace" => physical_workspace = true,
            "--only" => only_probe = Some(args.next().ok_or("--only needs a probe name")?),
            "--help" | "-h" => {
                println!(
                    "Usage: starbyte-native-qa [--binary PATH] [--physical-workspace] [--only PROBE]"
                );
                println!("Run isolated demo smoke probes on native Hyprland.");
                println!("--physical-workspace additionally tests real workspace progress");
                println!("and pause-on-blur before restoring the previous workspace.");
                return Ok(());
            }
            _ => return Err(format!("unrecognized option: {arg}").into()),
        }
    }
    // The owner's first physical retests showed that a different workspace
    // still hijacks the single active keyboard focus. Until a genuinely
    // independent QA compositor/seat has been implemented and confirmed, no
    // invocation of this collector may spawn a real GUI test window on the
    // owner's live desktop, including its former non-physical nine-probe mode.
    if !gui_probes_allowed() {
        return Err(
            "desktop-focus safety gate: native GUI probes require an independently isolated compositor and seat; run bash scripts/qa-native-hyprland.sh --check-only instead"
                .into(),
        );
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
    let supplemental = if physical_workspace {
        Some([
            &PHYSICAL_FOCUSED_PROBE,
            &PHYSICAL_WORKSPACE_PROBE,
            &PHYSICAL_PAUSE_PROBE,
        ])
    } else {
        None
    };
    for probe in PROBES.iter().chain(supplemental.into_iter().flatten()) {
        if only_probe.as_deref().is_some_and(|name| name != probe.name) {
            continue;
        }
        total += 1;
        let result = run_probe(
            &binary,
            &root,
            probe,
            physical_workspace
                && (probe.name == PHYSICAL_WORKSPACE_PROBE.name
                    || probe.name == PHYSICAL_PAUSE_PROBE.name),
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
    if total == 0 {
        return Err("no probe matched --only; physical probes require --physical-workspace".into());
    }
    // Emit aggregate evidence only after *all* native and physical probes
    // passed. A local agent can review these metrics through Actions job logs
    // without revealing physical workspace names, PIDs or personal paths.
    if physical_workspace && only_probe.is_none() && passed == total {
        let public = sanitized_physical_receipt(&root)?;
        fs::write(
            root.join("public-evidence.json"),
            serde_json::to_vec_pretty(&public)?,
        )?;
        println!(
            "STARBYTE_NATIVE_PHYSICAL_EVIDENCE={}",
            serde_json::to_string(&public)?
        );
    }
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
    use super::{
        PROBES, fresh_gap_start_at, gap_age_at, missing_markers, native_client_on_workspace,
        workspace_id,
    };

    #[cfg(unix)]
    #[test]
    fn unsuccessful_probe_scope_reaps_child_instead_of_leaving_emulator_running() {
        use std::process::{Command, Stdio};

        // A real compositor read can fail after the child has launched. The
        // watchdog owns the process even when the caller returns an error.
        let child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut guarded = super::KillOnDrop(child);
        let pid = guarded.id();
        assert!(guarded.try_wait().unwrap().is_none());
        drop(guarded);
        // A reaped child is no longer waitable: Linux /proc must not retain
        // a live process with this PID after the guard has been dropped.
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }

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
            gap_age_at(line, now, super::GAP_MARKER),
            Some(std::time::Duration::from_millis(500))
        );
        assert!(gap_age_at(line, std::time::UNIX_EPOCH, super::GAP_MARKER).is_none());
        assert!(
            gap_age_at(
                b"STARBYTE_FULL_LIBRARY_NO_REPAINT_GAP_BEGIN frame=42",
                now,
                super::GAP_MARKER
            )
            .is_none()
        );
        assert!(
            gap_age_at(
                b"STARBYTE_FULL_LIBRARY_NO_REPAINT_GAP_BEGIN frame=42 started_unix_ms=bad",
                now,
                super::GAP_MARKER
            )
            .is_none()
        );
        assert_eq!(
            fresh_gap_start_at(line, now, super::GAP_MARKER),
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_millis(3_500))
        );
        assert!(
            fresh_gap_start_at(
                b"STARBYTE_FULL_LIBRARY_NO_REPAINT_GAP_BEGIN frame=42 started_unix_ms=2700",
                now,
                super::GAP_MARKER
            )
            .is_none()
        );
        assert_eq!(
            fresh_gap_start_at(
                b"STARBYTE_FULL_LIBRARY_REAL_PAUSE_BEGIN frame=20 started_unix_ms=3500",
                now,
                super::PAUSE_GAP_MARKER
            ),
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_millis(3_500))
        );
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
    fn physical_collector_fails_closed_on_owner_desktop() {
        assert!(!super::gui_probes_allowed());
    }

    #[test]
    fn wrapper_blocks_gui_modes_before_external_commands() {
        let wrapper = include_str!("../../../../scripts/qa-native-hyprland.sh");
        let case_start = wrapper
            .find("case \"${1:-}\" in")
            .expect("wrapper must validate mode before running commands");
        let cases = &wrapper[case_start..];
        let focused = cases
            .split("  \"\"|--focused-only)")
            .nth(1)
            .expect("focused-only mode must have an explicit safety branch")
            .split(";;")
            .next()
            .expect("focused-only safety branch must terminate");

        assert!(focused.contains("fail 'owner_desktop_focus_must_not_be_stolen"));
        assert!(!focused.contains("cargo"));
        assert!(!focused.contains("git"));
        assert!(!focused.contains("hyprctl"));
        assert!(!focused.contains("starbyte-native-qa"));
        assert!(
            wrapper[case_start..]
                .find("for command_name in cargo git")
                .is_some_and(|commands| commands > 0)
        );
    }

    #[test]
    fn isolated_wayland_harness_hides_host_display_and_devices() {
        let harness = include_str!("../../../../scripts/qa-isolated-wayland.sh");
        for required in [
            "--unshare-all --die-with-parent --new-session",
            "--tmpfs /run",
            "--tmpfs /tmp",
            "--dev /dev",
            "--clearenv",
            "--unsetenv DISPLAY",
            "--unsetenv WAYLAND_SOCKET",
            "test ! -e /dev/input",
            "test ! -e /dev/dri",
            "test ! -e \"$STARBYTE_HOST_WAYLAND_SOCKET\"",
            "test ! -e \"$STARBYTE_HOST_RUNTIME_DIR/hypr\"",
            "test ! -e \"$STARBYTE_HOST_CONFIG_HOME\"",
            "test ! -e \"$STARBYTE_HOST_DBUS_SOCKET\"",
            "test ! -e /run/user/qa/bus",
            "test ! -e /run/dbus/system_bus_socket",
            "test -z \"${DBUS_SESSION_BUS_ADDRESS-}\"",
            "test -z \"${HYPRLAND_INSTANCE_SIGNATURE-}\"",
            "test -z \"${XDG_SESSION_ID-}\"",
            "test -z \"${SYSTEMD_EXEC_PID-}\"",
            "test ! -e /tmp/.X11-unix",
            "--hyprland-headless-smoke",
            "--hyprland-nested-smoke",
            "nested_hyprland_requires_private_drm_allocator",
            "headless_hyprland_requires_private_drm_allocator",
            "HYPRLAND_INSTANCE_SIGNATURE",
            "--backend=headless --renderer=pixman",
            "--shell=kiosk-shell.so",
            "timeout --signal=TERM --kill-after=1s 2s",
            "timeout --signal=TERM --kill-after=1s 4s \"$STARBYTE_WAYLAND_INFO\"",
            "timeout --signal=TERM --kill-after=2s 20s",
            "timeout --signal=TERM --kill-after=2s 10s bwrap",
            "bubblewrap_isolation_check_failed_or_timed_out",
            "weston_smoke_failed_or_timed_out",
            "weston-simple-shm",
            "wayland-info",
            "PRIVATE_WESTON_CLIENT_CONNECTED",
            "trap cleanup EXIT",
            "trap \"exit 143\" TERM",
            "PRIVATE_WESTON_SOCKET_STILL_PRESENT",
            "kill -KILL \"$weston_pid\"",
            "STARBYTE_WESTON_PACKAGE_ROOT",
            "--ro-bind \"$package_root\" /run/user/qa/package",
            "WESTON_MODULE_MAP",
            "WESTON_DATA_DIR",
            "PRIVATE_WESTON_TORN_DOWN",
        ] {
            assert!(
                harness.contains(required),
                "missing isolation guard: {required}"
            );
        }
        assert!(!harness.contains("STARBYTE_HYPRCTL_BIN"));
        assert!(!harness.contains("XDG_SESSION_BUS_ADDRESS"));
    }

    #[test]
    fn hosted_virtual_device_probe_has_no_host_display_or_passthrough() {
        let probe = include_str!("../../../../scripts/qa-virtual-devices.sh");
        for expected in [
            "github_hosted_runner_only",
            "graphical_session_present",
            "desktop_session_present",
            "interactive_session_present",
            "q35,accel=tcg",
            "-nodefaults -display none -vga none -net none",
            "-monitor none -serial none -parallel none",
            "-device virtio-gpu-pci,id=sb_gpu",
            "-device virtio-keyboard-pci,id=sb_keyboard",
            "qmp_capabilities",
            "query-pci",
            "virtual_gpu_or_keyboard_missing_from_pci",
            "guest_booted=false",
            "trap cleanup EXIT",
        ] {
            assert!(
                probe.contains(expected),
                "missing QMP hardware-model safety rule: {expected}"
            );
        }
        for forbidden in ["-enable-kvm", "-drive ", "usb-host", "vfio-pci"] {
            assert!(
                !probe.contains(forbidden),
                "forbidden host passthrough: {forbidden}"
            );
        }
    }

    #[test]
    fn hosted_guest_probe_forbids_live_desktop_and_hardware_passthrough() {
        let script = include_str!("../../../../scripts/qa-guest-linux.sh");
        for guard in [
            "github_hosted_runner_only",
            "graphical_session_present",
            "interactive_session_present",
            "q35,accel=tcg",
            "-nodefaults -display none -vga none -net none",
            "-monitor none -parallel none -serial",
            "-device virtio-gpu-pci,id=sb_gpu",
            "-device virtio-keyboard-pci,id=sb_keyboard",
            "STARBYTE_GUEST_LINUX_BOOT_OK",
            "STARBYTE_GUEST_PCI_RESULT",
            "drm_render_node=not_tested",
            "STARBYTE_GUEST_DRM_INPUT_OK",
            "modloop-virt",
            "modloop_module_tree_missing",
            "guest_driver_or_device_nodes_missing",
        ] {
            assert!(script.contains(guard), "guest smoke guard missing: {guard}");
        }
    }

    #[test]
    fn hosted_gbm_probe_requires_guest_render_buffer_and_no_host_device() {
        let launcher = include_str!("../../../../scripts/qa-guest-linux.sh");
        let rust = include_str!("../../../../scripts/qa-guest-gbm.rs");
        for requirement in [
            "--gbm-smoke",
            "gbm_probe_compilation_failed",
            "gbm_dependency_copy_failed",
            "hosted_kms_swrast_backend_missing",
            "MESA_LOADER_DRIVER_OVERRIDE=kms_swrast",
            "LIBGL_ALWAYS_SOFTWARE=1",
            "STARBYTE_GUEST_GBM_RESULT node=render device=1 buffer=1",
            "STARBYTE_GUEST_GBM_RESULT node=card device=1 buffer=1",
            "STARBYTE_GUEST_GBM_OK node=card device=initialized buffer=rendering guest_only=true",
            "guest_gbm_allocator_unavailable",
            "dri_gbm.so",
            "hosted_mesa_gbm_backend_missing",
            "GBM_BACKENDS_PATH=/usr/lib/x86_64-linux-gnu/gbm",
        ] {
            assert!(launcher.contains(requirement));
        }
        assert!(rust.contains("gbm_create_device"));
        assert!(rust.contains("gbm_bo_create"));
        assert!(rust.contains("/dev/dri/renderD"));
        assert!(rust.contains("/dev/dri/card"));
        assert!(!rust.contains("/dev/input"));
        assert!(!rust.contains("WAYLAND_DISPLAY"));
    }

    #[test]
    fn hosted_input_probe_routes_qmp_to_guest_evdev_only() {
        let harness = include_str!("../../../../scripts/qa-guest-linux.sh");
        let reader = include_str!("../../../../scripts/qa-guest-input.rs");
        for required in [
            "--input-smoke",
            "qmp_args=(-qmp",
            "guest_input_injector",
            "input-send-event",
            "guest_private_qmp_injection_failed",
            "STARBYTE_GUEST_INPUT_QMP_ACK key_a=sent",
            "guest_virtual_keypress_missing",
            "STARBYTE_GUEST_INPUT_OK key_a=received source=private_qmp evdev=guest_only",
        ] {
            assert!(
                harness.contains(required),
                "missing private input rule: {required}"
            );
        }
        assert!(reader.contains("STARBYTE_GUEST_INPUT_READY evdev=present"));
        assert!(reader.contains("STARBYTE_GUEST_INPUT_RESULT key_a=1"));
        assert!(reader.contains("/dev/input"));
        assert!(!reader.contains("WAYLAND_DISPLAY"));
        assert!(!reader.contains("/dev/dri"));
    }

    #[test]
    fn nested_hyprland_probe_refuses_without_render_allocator_before_any_launch() {
        let harness = include_str!("../../../../scripts/qa-isolated-wayland.sh");
        let guard = harness
            .find("if [[ \"$mode\" == --hyprland-nested-smoke ]]; then")
            .expect("explicit nested block required");
        let reason = harness
            .find("fail 'nested_hyprland_requires_private_drm_allocator'")
            .expect("must report why the pinned Aquamarine backend cannot start");
        let first_external_probe = harness
            .find("command -v bwrap")
            .expect("must not resolve external launchers before refusing");
        let headless_guard = harness
            .find("if [[ \"$mode\" == --hyprland-headless-smoke ]]; then")
            .expect("headless backend cannot start without DRM either");
        let headless_reason = harness
            .find("fail 'headless_hyprland_requires_private_drm_allocator'")
            .expect("must classify the headless allocator blocker too");
        assert!(guard < reason && reason < headless_guard);
        assert!(headless_guard < headless_reason && headless_reason < first_external_probe);
        assert!(!harness.contains("nested_supervisor"));
        assert!(!harness.contains("STARBYTE_HYPRLAND_BIN"));
        assert!(!harness.contains("STARBYTE_ISOLATED_NESTED_HYPRLAND_SMOKE_OK"));
    }

    #[test]
    fn isolated_qa_workspace_guard_rejects_external_focus_and_invalid_ids() {
        let active = serde_json::json!({"id": 812345, "name": "812345"});
        assert!(super::qa_workspace_matches("812345", &active));
        assert!(!super::qa_workspace_matches("812346", &active));
        assert!(!super::qa_workspace_matches("0", &active));
        assert!(!super::qa_workspace_matches("-1", &active));
        assert!(!super::qa_workspace_matches("invalid", &active));
        assert!(!super::qa_workspace_matches(
            "812345",
            &serde_json::json!({})
        ));
    }

    #[test]
    fn native_focused_clock_evidence_rejects_synthetic_or_incomplete_rates() {
        let source = "STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_OK fps=60.090 callbacks=180 presented=180 frames=180 wall_ms=3000";
        let receipt = super::focused_clock_receipt(source, 25, 5432).unwrap();
        assert_eq!(receipt["emulator_worker_fps"], 60.09);
        assert_eq!(receipt["completed_worker_frames"], 180);
        assert_eq!(receipt["starbyte_process_id"], 5432);
        assert!(super::focused_clock_receipt(source, 3, 5432).is_none());
        assert!(super::focused_clock_receipt(
            "STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_OK fps=60.0 callbacks=180 presented=180 frames=0 wall_ms=3000",
            25,
            5432
        )
        .is_none());
        assert!(super::focused_clock_receipt(
            "STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_OK fps=60.0 callbacks=180 presented=180 frames=120 wall_ms=3000",
            25,
            5432
        )
        .is_none());
        // The application rounds its measured FPS to three decimals and
        // elapsed time to whole milliseconds; normal rounding still passes.
        assert!(super::focused_clock_receipt(
            "STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_OK fps=60.098 callbacks=180 presented=180 frames=181 wall_ms=3012",
            25,
            5432
        )
        .is_some());

        assert!(super::focused_clock_receipt(
            "STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_OK fps=100.0 callbacks=180 presented=180 frames=300 wall_ms=3000",
            25,
            5432
        )
        .is_none());
        assert!(
            super::focused_clock_receipt(
                "STARBYTE_FULL_LIBRARY_FOCUSED_CLOCK_BEGIN frame=30",
                25,
                5432
            )
            .is_none()
        );
    }

    #[test]
    fn opt_in_physical_pause_receipts_match_real_original_window_messages() {
        let app = include_str!("../app.rs");
        for marker in super::PHYSICAL_PAUSE_PROBE
            .markers
            .iter()
            .chain(super::PHYSICAL_FOCUSED_PROBE.markers.iter())
        {
            assert!(
                app.contains(marker),
                "physical pause marker is absent from original egui window: {marker}"
            );
        }
    }

    #[test]
    fn sanitized_physical_metrics_only_read_explicit_numeric_success_fields() {
        let background = "window paths: /home/owner/private-game.sfc\nSTARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_OK advanced_frames=120 fps=60.12 wall_ms=2000\n";
        let fps: Option<f64> = super::marker_metric(
            background,
            "STARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_OK ",
            "fps=",
        );
        assert_eq!(fps, Some(60.12));
        let invalid: Option<u64> = super::marker_metric(
            background,
            "STARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_OK ",
            "private-game.sfc=",
        );
        assert_eq!(invalid, None);
        let not_ok: Option<f64> = super::marker_metric(
            "STARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_FAILED fps=60.12",
            "STARBYTE_FULL_LIBRARY_CLOCK_PACING_SMOKE_OK ",
            "fps=",
        );
        assert_eq!(not_ok, None);
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
