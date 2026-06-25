use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use tracing_subscriber::{EnvFilter, fmt::writer::MakeWriter};

pub type SharedLogBuffer = Arc<Mutex<VecDeque<String>>>;

const MAX_LOG_LINES: usize = 2_000;

#[derive(Clone)]
struct GuiLogWriterFactory {
    file: Arc<Mutex<File>>,
    lines: SharedLogBuffer,
}

struct GuiLogWriter {
    file: Arc<Mutex<File>>,
    lines: SharedLogBuffer,
    pending: Vec<u8>,
}

impl<'a> MakeWriter<'a> for GuiLogWriterFactory {
    type Writer = GuiLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        GuiLogWriter {
            file: self.file.clone(),
            lines: self.lines.clone(),
            pending: Vec::new(),
        }
    }
}

impl Write for GuiLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(mut file) = self.file.lock() {
            file.write_all(buf)?;
            file.flush()?;
        }
        let _ = io::stderr().write_all(buf);
        let _ = io::stderr().flush();

        self.pending.extend_from_slice(buf);
        while let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=index).collect::<Vec<_>>();
            let line = String::from_utf8_lossy(&line).trim().to_owned();
            if line.is_empty() {
                continue;
            }
            if let Ok(mut lines) = self.lines.lock() {
                lines.push_back(line);
                while lines.len() > MAX_LOG_LINES {
                    lines.pop_front();
                }
            }
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Ok(mut file) = self.file.lock() {
            file.flush()?;
        }
        io::stderr().flush()
    }
}

pub fn install_tracing(cache_root: &Path, filter: &str) -> Result<SharedLogBuffer> {
    let log_dir = cache_root.join("logs");
    fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("starbyte-egui.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let lines = Arc::new(Mutex::new(VecDeque::new()));
    let writer = GuiLogWriterFactory {
        file: Arc::new(Mutex::new(file)),
        lines: lines.clone(),
    };

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter.to_owned()))
        .with_thread_ids(true)
        .with_target(true)
        .with_ansi(false)
        .with_writer(writer)
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing subscriber: {error}"))?;

    Ok(lines)
}
