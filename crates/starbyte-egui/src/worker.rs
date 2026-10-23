use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use starbyte_core::manifest::{AssetConfig, RuntimeConfig};
use starbyte_frontend::{
    LibraryEntry, LibraryFilter, LibraryService, LibrarySnapshot, LibraryTarget,
    SessionLaunchRequest, SoundSessionMode,
};
use tracing::{error, info};

#[derive(Debug, Clone)]
pub enum WorkerCommandKind {
    RefreshSnapshot,
    RefreshMetadata,
    RefreshArtwork,
    RefreshCovers { target: LibraryTarget },
    RefreshCheats { target: LibraryTarget },
    RefreshAll,
    MaterializeRom { entry: LibraryEntry },
    MaterializePath {
        path: PathBuf,
        matching: Option<LibraryEntry>,
    },
}

#[derive(Debug, Clone)]
pub struct WorkerCommand {
    pub job_id: u64,
    pub config: RuntimeConfig,
    pub filter: LibraryFilter,
    pub kind: WorkerCommandKind,
}

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    JobStarted {
        job_id: u64,
        label: String,
    },
    SnapshotReady {
        job_id: u64,
        snapshot: LibrarySnapshot,
        config: RuntimeConfig,
        status: String,
    },
    RomReady {
        job_id: u64,
        entry: LibraryEntry,
        request: SessionLaunchRequest,
    },
    DirectRomReady {
        job_id: u64,
        original_path: PathBuf,
        game_id: Option<String>,
        request: SessionLaunchRequest,
    },
    JobFailed {
        job_id: u64,
        label: String,
        error: String,
    },
}

#[derive(Debug)]
pub struct AppWorker {
    command_tx: Sender<WorkerCommand>,
    rom_tx: Sender<WorkerCommand>,
    event_rx: Receiver<WorkerEvent>,
}

impl AppWorker {
    pub fn spawn(assets: AssetConfig) -> Self {
        let (command_tx, command_rx) = mpsc::channel::<WorkerCommand>();
        let (rom_tx, rom_rx) = mpsc::channel::<WorkerCommand>();
        let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>();
        let rom_assets = assets.clone();
        let rom_events = event_tx.clone();
        // ROM extraction is local and should not wait for a potentially
        // minutes-long cover-download job on the library/network worker.
        thread::Builder::new()
            .name("starbyte-rom-worker".to_owned())
            .spawn(move || worker_loop(rom_assets, rom_rx, rom_events))
            .expect("failed to spawn starbyte ROM worker");
        thread::Builder::new()
            .name("starbyte-library-worker".to_owned())
            .spawn(move || worker_loop(assets, command_rx, event_tx))
            .expect("failed to spawn starbyte library worker");
        Self {
            command_tx,
            rom_tx,
            event_rx,
        }
    }

    pub fn submit(&self, command: WorkerCommand) {
        // Separate background queues keep gameplay loading responsive while
        // Libretro artwork or other network refreshes are in progress.
        let tx = if matches!(
            command.kind,
            WorkerCommandKind::MaterializeRom { .. } | WorkerCommandKind::MaterializePath { .. }
        ) {
            &self.rom_tx
        } else {
            &self.command_tx
        };
        let _ = tx.send(command);
    }

    pub fn try_recv(&self) -> Option<WorkerEvent> {
        self.event_rx.try_recv().ok()
    }
}

fn worker_loop(
    assets: AssetConfig,
    command_rx: Receiver<WorkerCommand>,
    event_tx: Sender<WorkerEvent>,
) {
    for command in command_rx {
        let label = label_for_kind(&command.kind);
        let job_id = command.job_id;
        info!(job_id = command.job_id, label, "worker job started");
        let _ = event_tx.send(WorkerEvent::JobStarted {
            job_id: command.job_id,
            label: label.to_owned(),
        });

        if let Err(error) = handle_command(command, &assets, &event_tx, label) {
            error!(label, "{error}");
            let _ = event_tx.send(WorkerEvent::JobFailed {
                job_id,
                label: label.to_owned(),
                error: error.to_string(),
            });
        }
    }
}

fn handle_command(
    command: WorkerCommand,
    assets: &AssetConfig,
    event_tx: &Sender<WorkerEvent>,
    _label: &str,
) -> anyhow::Result<()> {
    let WorkerCommand {
        job_id,
        config,
        filter,
        kind,
    } = command;
    let mut service = LibraryService::new(config, assets.clone())?;

    let event = match kind {
        WorkerCommandKind::RefreshSnapshot => {
            let snapshot = service.snapshot(filter)?;
            let report = service.scan_report()?;
            WorkerEvent::SnapshotReady {
                job_id,
                snapshot,
                config: service.config().clone(),
                status: format!(
                    "Library: {} indexed from {} ROM candidates; {} skipped. See Scan Report in Settings.",
                    report.discovered,
                    report.candidates,
                    report.skipped.len()
                ),
            }
        }
        WorkerCommandKind::RefreshMetadata => {
            let count = service.refresh_metadata_index()?;
            // The UI owns mutable settings. Workers only publish cache results.
            WorkerEvent::SnapshotReady {
                job_id,
                snapshot: service.snapshot(filter)?,
                config: service.config().clone(),
                status: format!("Refreshed metadata index ({count} records)."),
            }
        }
        WorkerCommandKind::RefreshArtwork => {
            anyhow::ensure!(
                service.config().advanced.providers.enable_network,
                "Network artwork is disabled. Enable network providers in Settings first."
            );
            let metadata_count = service.refresh_metadata_index()?;
            // Do not download covers for catalog titles the user does not own.
            let written = service.refresh_covers(&LibraryTarget {
                installed_only: true,
                ..LibraryTarget::default()
            })?;
            // The UI owns mutable settings. Workers only publish cache results.
            WorkerEvent::SnapshotReady {
                job_id,
                // Artwork refresh is about the user's owned/local collection.
                // Do not suddenly populate the player grid with thousands of
                // metadata-only online catalog entries after fetching images.
                snapshot: service.snapshot(LibraryFilter {
                    installed_only: true,
                    ..filter
                })?,
                config: service.config().clone(),
                status: format!(
                    "Artwork: indexed {metadata_count} reference titles; downloaded {written} new covers."
                ),
            }
        }
        WorkerCommandKind::RefreshCovers { target } => {
            let count = service.refresh_covers(&target)?;
            // The UI owns mutable settings. Workers only publish cache results.
            WorkerEvent::SnapshotReady {
                job_id,
                snapshot: service.snapshot(filter)?,
                config: service.config().clone(),
                status: format!("Refreshed covers ({count} file(s))."),
            }
        }
        WorkerCommandKind::RefreshCheats { target } => {
            let count = service.refresh_cheats(&target)?;
            // The UI owns mutable settings. Workers only publish cache results.
            WorkerEvent::SnapshotReady {
                job_id,
                snapshot: service.snapshot(filter)?,
                config: service.config().clone(),
                status: format!("Refreshed cheats ({count} record(s))."),
            }
        }
        WorkerCommandKind::RefreshAll => {
            let summary = service.refresh_all(&LibraryTarget::default())?;
            // The UI owns mutable settings. Workers only publish cache results.
            WorkerEvent::SnapshotReady {
                job_id,
                snapshot: service.snapshot(filter)?,
                config: service.config().clone(),
                status: format!(
                    "Refreshed metadata {}, covers {}, cheats {}.",
                    summary.metadata_records, summary.covers_written, summary.cheat_records
                ),
            }
        }
        WorkerCommandKind::MaterializeRom { entry } => {
            let Some(local) = entry.local.as_ref() else {
                anyhow::bail!("Selected game is not installed locally.");
            };
            let rom_path = service.materialize_rom(local)?;
            let request = build_launch_request(&entry, rom_path);
            WorkerEvent::RomReady {
                job_id,
                entry,
                request,
            }
        }
        WorkerCommandKind::MaterializePath { path, matching } => {
            // ZIP extraction may involve substantial disk work. Keep it on
            // the dedicated ROM lane, never in eframe::App::update.
            let resolved = if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
            {
                service.materialize_single_rom_zip(&path)?
            } else {
                path.clone()
            };
            let request = direct_launch_request(matching.as_ref(), resolved);
            WorkerEvent::DirectRomReady {
                job_id,
                original_path: path,
                game_id: matching.map(|entry| entry.game_id),
                request,
            }
        }
    };

    let _ = event_tx.send(event);
    Ok(())
}

/// Prepare the complete cartridge transaction on the library side, where
/// the selected entry and matching cheats are still available. The emulation
/// owner can consume this request without inferring a game's prior settings.
fn build_launch_request(entry: &LibraryEntry, rom_path: PathBuf) -> SessionLaunchRequest {
    SessionLaunchRequest {
        rom_path,
        sound_mode: SoundSessionMode::Persistent,
        cheats: entry.cheats.clone(),
    }
}

/// Directly opened files use the same transaction shape as library entries.
/// Preserve the selected entry's cheats, if already indexed, and never
/// inherit cheats or sound/save policy from the previously running ROM.
fn direct_launch_request(
    matching: Option<&LibraryEntry>,
    resolved: PathBuf,
) -> SessionLaunchRequest {
    SessionLaunchRequest {
        rom_path: resolved,
        sound_mode: SoundSessionMode::Persistent,
        cheats: matching.map_or_else(Vec::new, |entry| entry.cheats.clone()),
    }
}

fn label_for_kind(kind: &WorkerCommandKind) -> &'static str {
    match kind {
        WorkerCommandKind::RefreshSnapshot => "Scan Library",
        WorkerCommandKind::RefreshMetadata => "Refresh Metadata",
        WorkerCommandKind::RefreshArtwork => "Get Artwork",
        WorkerCommandKind::RefreshCovers { .. } => "Refresh Covers",
        WorkerCommandKind::RefreshCheats { .. } => "Refresh Cheats",
        WorkerCommandKind::RefreshAll => "Refresh All",
        WorkerCommandKind::MaterializeRom { .. } => "Load Game",
        WorkerCommandKind::MaterializePath { .. } => "Open ROM",
    }
}

/// Route-level regression: the local ROM queue must remain independent of
/// network metadata and box-art work.
#[cfg(test)]
mod routing_tests {
    use super::{
        AppWorker, WorkerCommand, WorkerCommandKind, WorkerEvent, build_launch_request,
        direct_launch_request, handle_command,
    };
    use starbyte_core::manifest::{AssetConfig, RuntimeConfig};
    use starbyte_frontend::{
        CheatEntry, InstalledStatus, LibraryEntry, LibraryFilter, SoundSessionMode,
    };
    use std::{path::PathBuf, sync::mpsc};

    #[test]
    fn prepared_rom_launch_binds_selected_cheats_and_sound_namespace() {
        let path = std::path::PathBuf::from("resolved-game.sfc");
        let cheat = CheatEntry {
            id: "selected-cheat".to_owned(),
            game_id: "sample".to_owned(),
            name: "Selected patch".to_owned(),
            code: "7E0000:01".to_owned(),
            source: "local".to_owned(),
            kind: "raw".to_owned(),
            enabled: true,
        };
        let entry = LibraryEntry {
            game_id: "sample".to_owned(),
            display_title: "Sample".to_owned(),
            installed_status: InstalledStatus::Installed,
            local: None,
            metadata: None,
            cover: None,
            cheats: vec![cheat],
        };
        let request = build_launch_request(&entry, path.clone());
        assert_eq!(request.rom_path, path);
        assert_eq!(request.sound_mode, SoundSessionMode::Persistent);
        assert_eq!(request.cheats.len(), 1);
        assert_eq!(request.cheats[0].id, "selected-cheat");
    }

    #[test]
    fn direct_file_open_prepares_worker_event_without_advancing_emulation() {
        let (events, receiver) = mpsc::channel();
        let rom = PathBuf::from("prepared-game.sfc");
        let command = WorkerCommand {
            job_id: 17,
            config: RuntimeConfig::default(),
            filter: LibraryFilter::default(),
            kind: WorkerCommandKind::MaterializePath {
                path: rom.clone(),
                matching: None,
            },
        };
        handle_command(command, &AssetConfig::default(), &events, "Open ROM").unwrap();
        match receiver.recv().unwrap() {
            WorkerEvent::DirectRomReady {
                job_id,
                original_path,
                game_id,
                request,
            } => {
                assert_eq!(job_id, 17);
                assert_eq!(original_path, rom);
                assert_eq!(request.rom_path, rom);
                assert_eq!(game_id, None);
                assert!(request.cheats.is_empty());
                assert_eq!(request.sound_mode, SoundSessionMode::Persistent);
            }
            other => panic!("unexpected ROM event: {other:?}"),
        }
    }

    #[test]
    fn direct_file_launch_does_not_inherit_another_games_cheats() {
        let request = direct_launch_request(None, PathBuf::from("new-game.sfc"));
        assert!(request.cheats.is_empty());
        assert_eq!(request.sound_mode, SoundSessionMode::Persistent);
    }

    #[test]
    fn game_launch_bypasses_blocked_artwork_queue() {
        let (library_tx, library_rx) = mpsc::channel();
        let (rom_tx, rom_rx) = mpsc::channel();
        let (_events_tx, events_rx) = mpsc::channel();
        let worker = AppWorker {
            command_tx: library_tx,
            rom_tx,
            event_rx: events_rx,
        };
        let config = RuntimeConfig::default();
        let filter = LibraryFilter::default();
        worker.submit(WorkerCommand {
            job_id: 1,
            config: config.clone(),
            filter: filter.clone(),
            kind: WorkerCommandKind::RefreshArtwork,
        });
        worker.submit(WorkerCommand {
            job_id: 2,
            config,
            filter,
            kind: WorkerCommandKind::MaterializeRom {
                entry: LibraryEntry {
                    game_id: "sample".to_owned(),
                    display_title: "Sample".to_owned(),
                    installed_status: InstalledStatus::Installed,
                    local: None,
                    metadata: None,
                    cover: None,
                    cheats: Vec::new(),
                },
            },
        });
        assert_eq!(library_rx.try_recv().unwrap().job_id, 1);
        assert_eq!(rom_rx.try_recv().unwrap().job_id, 2);
    }
}
