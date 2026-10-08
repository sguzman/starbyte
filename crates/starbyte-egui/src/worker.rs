use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use starbyte_core::manifest::{AssetConfig, RuntimeConfig};
use starbyte_frontend::{
    LibraryEntry, LibraryFilter, LibraryService, LibrarySnapshot, LibraryTarget,
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
        rom_path: PathBuf,
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
        let tx = if matches!(command.kind, WorkerCommandKind::MaterializeRom { .. }) {
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
            service.save_config()?;
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
            service.save_config()?;
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
            service.save_config()?;
            WorkerEvent::SnapshotReady {
                job_id,
                snapshot: service.snapshot(filter)?,
                config: service.config().clone(),
                status: format!("Refreshed covers ({count} file(s))."),
            }
        }
        WorkerCommandKind::RefreshCheats { target } => {
            let count = service.refresh_cheats(&target)?;
            service.save_config()?;
            WorkerEvent::SnapshotReady {
                job_id,
                snapshot: service.snapshot(filter)?,
                config: service.config().clone(),
                status: format!("Refreshed cheats ({count} record(s))."),
            }
        }
        WorkerCommandKind::RefreshAll => {
            let summary = service.refresh_all(&LibraryTarget::default())?;
            service.save_config()?;
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
            WorkerEvent::RomReady {
                job_id,
                entry,
                rom_path,
            }
        }
    };

    let _ = event_tx.send(event);
    Ok(())
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
    }
}

/// Route-level regression: the local ROM queue must remain independent of
/// network metadata and box-art work.
#[cfg(test)]
mod routing_tests {
    use super::{AppWorker, WorkerCommand, WorkerCommandKind};
    use starbyte_core::manifest::RuntimeConfig;
    use starbyte_frontend::{InstalledStatus, LibraryEntry, LibraryFilter};
    use std::sync::mpsc;

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
