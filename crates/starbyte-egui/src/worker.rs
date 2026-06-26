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
    event_rx: Receiver<WorkerEvent>,
}

impl AppWorker {
    pub fn spawn(assets: AssetConfig) -> Self {
        let (command_tx, command_rx) = mpsc::channel::<WorkerCommand>();
        let (event_tx, event_rx) = mpsc::channel::<WorkerEvent>();
        thread::Builder::new()
            .name("starbyte-library-worker".to_owned())
            .spawn(move || worker_loop(assets, command_rx, event_tx))
            .expect("failed to spawn starbyte library worker");
        Self {
            command_tx,
            event_rx,
        }
    }

    pub fn submit(&self, command: WorkerCommand) {
        let _ = self.command_tx.send(command);
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
        WorkerCommandKind::RefreshSnapshot => WorkerEvent::SnapshotReady {
            job_id,
            snapshot: service.snapshot(filter)?,
            config: service.config().clone(),
            status: "Library scan complete.".to_owned(),
        },
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
        WorkerCommandKind::RefreshCovers { .. } => "Refresh Covers",
        WorkerCommandKind::RefreshCheats { .. } => "Refresh Cheats",
        WorkerCommandKind::RefreshAll => "Refresh All",
        WorkerCommandKind::MaterializeRom { .. } => "Load Game",
    }
}
