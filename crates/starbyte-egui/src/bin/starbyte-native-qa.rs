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

fn run_probe(
    binary: &Path,
    root: &Path,
    probe: &Probe,
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
    let exited = loop {
        if let Some(status) = child.try_wait()? {
            break status.success();
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
        exited && missing.is_empty(),
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
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--binary" => binary = PathBuf::from(args.next().ok_or("--binary needs a path")?),
            "--help" | "-h" => {
                println!("Usage: starbyte-native-qa [--binary PATH]");
                println!("Run local, isolated demo smoke probes on native Hyprland.");
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
    for probe in PROBES {
        let result = run_probe(&binary, &root, probe);
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
        "Native Hyprland synthetic probes: {passed}/{}.",
        PROBES.len()
    );
    println!("Local-only logs and context: {}", root.display());
    println!("Physical hardware and real workspace acceptance: NOT TESTED.");
    if passed != PROBES.len() {
        return Err("one or more native synthetic probes failed; inspect local logs".into());
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
    use super::{PROBES, missing_markers};

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
    fn native_probes_require_real_ci_smoke_contract_markers() {
        let ci = std::fs::read_to_string(format!(
            "{}/../../.github/workflows/ci.yml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        for probe in PROBES {
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
