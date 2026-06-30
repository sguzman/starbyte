//! Audio processing unit bootstrap boundary.

pub mod spc700;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::error::{Error, Result};

use self::spc700::Spc700;

/// Size of the user-supplied SPC700 IPL ROM.
pub const SPC700_IPL_ROM_LEN: usize = 64;

/// Buffered audio samples returned to a frontend.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AudioFrame {
    /// Interleaved stereo 16-bit samples.
    pub samples: Vec<i16>,
}

/// Snapshot of APU bootstrap status surfaced to the emulator and CLI layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApuStatus {
    /// Whether a user-supplied IPL ROM is currently loaded.
    pub has_ipl_rom: bool,
    /// Configured firmware path if any.
    pub configured_ipl_path: Option<PathBuf>,
    /// Total SPC700 bootstrap steps executed through the APU boundary.
    pub spc700_steps: u64,
}

/// Minimal APU wrapper used to establish timing and communication boundaries early.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Apu {
    /// SPC700 core state.
    pub spc700: Spc700,
    cpu_to_apu_ports: [u8; 4],
    apu_to_cpu_ports: [u8; 4],
    ipl_rom: Option<Vec<u8>>,
    configured_ipl_path: Option<PathBuf>,
    spc700_steps: u64,
}

impl Default for Apu {
    fn default() -> Self {
        Self {
            spc700: Spc700::default(),
            cpu_to_apu_ports: [0; 4],
            apu_to_cpu_ports: [0; 4],
            ipl_rom: None,
            configured_ipl_path: None,
            spc700_steps: 0,
        }
    }
}

impl Apu {
    /// Create an APU boundary configured with an optional firmware path.
    #[must_use]
    pub fn with_ipl_path(path: Option<PathBuf>) -> Self {
        Self {
            configured_ipl_path: path,
            ..Self::default()
        }
    }

    /// Reset APU-visible state while preserving configured firmware.
    pub fn reset(&mut self) {
        self.spc700.reset();
        self.cpu_to_apu_ports = [0; 4];
        self.apu_to_cpu_ports = if self.ipl_rom.is_some() {
            [0xAA, 0xBB, 0x00, 0x00]
        } else {
            [0; 4]
        };
        self.spc700_steps = 0;
    }

    /// Configure or replace the path to a user-supplied IPL ROM.
    pub fn set_ipl_path(&mut self, path: Option<PathBuf>) {
        self.configured_ipl_path = path;
        self.ipl_rom = None;
    }

    /// Load the configured user-supplied IPL ROM if a path is present.
    #[instrument(skip_all)]
    pub fn load_configured_ipl_rom(&mut self) -> Result<bool> {
        let Some(path) = self.configured_ipl_path.clone() else {
            self.ipl_rom = None;
            return Ok(false);
        };

        let data = std::fs::read(&path).map_err(|source| Error::io(&path, source))?;
        self.install_ipl_rom_bytes(data, Some(path))?;
        Ok(true)
    }

    /// Install a user-supplied IPL ROM from owned bytes.
    pub fn install_ipl_rom_bytes(&mut self, data: Vec<u8>, source: Option<PathBuf>) -> Result<()> {
        if data.len() != SPC700_IPL_ROM_LEN {
            return Err(Error::InvalidFirmware {
                name: "SPC700 IPL ROM",
                details: format!(
                    "expected {SPC700_IPL_ROM_LEN} bytes but received {} bytes",
                    data.len()
                ),
            });
        }

        debug!(path = ?source, "loaded user-supplied SPC700 IPL ROM");
        self.ipl_rom = Some(data);
        if source.is_some() {
            self.configured_ipl_path = source;
        }
        Ok(())
    }

    /// Step the SPC700 core once through the APU boundary.
    pub fn step_spc700(&mut self) {
        self.spc700.step();
        self.spc700_steps = self.spc700_steps.saturating_add(1);
    }

    /// Advance placeholder APU work for a number of master cycles.
    pub fn step_master_cycles(&mut self, master_cycles: u64) {
        // The exact divider will be replaced when full system timing is modeled.
        for _ in 0..(master_cycles / 6) {
            self.step_spc700();
        }
        self.advance_bootstrap_handshake();
    }

    /// Write one CPU-to-APU communication port byte.
    pub fn write_cpu_port(&mut self, port: usize, value: u8) -> Result<()> {
        let Some(slot) = self.cpu_to_apu_ports.get_mut(port) else {
            return Err(Error::Unimplemented("APU port index out of range"));
        };
        *slot = value;
        Ok(())
    }

    /// Read one CPU-to-APU communication port byte.
    pub fn read_cpu_port(&self, port: usize) -> Result<u8> {
        self.cpu_to_apu_ports
            .get(port)
            .copied()
            .ok_or(Error::Unimplemented("APU port index out of range"))
    }

    /// Write one APU-to-CPU communication port byte.
    pub fn write_apu_port(&mut self, port: usize, value: u8) -> Result<()> {
        let Some(slot) = self.apu_to_cpu_ports.get_mut(port) else {
            return Err(Error::Unimplemented("APU port index out of range"));
        };
        *slot = value;
        Ok(())
    }

    /// Read one APU-to-CPU communication port byte.
    pub fn read_apu_port(&self, port: usize) -> Result<u8> {
        self.apu_to_cpu_ports
            .get(port)
            .copied()
            .ok_or(Error::Unimplemented("APU port index out of range"))
    }

    /// Borrow the current loaded IPL ROM if any.
    #[must_use]
    pub fn ipl_rom(&self) -> Option<&[u8]> {
        self.ipl_rom.as_deref()
    }

    /// Return a high-level status snapshot.
    #[must_use]
    pub fn status(&self) -> ApuStatus {
        ApuStatus {
            has_ipl_rom: self.ipl_rom.is_some(),
            configured_ipl_path: self.configured_ipl_path.clone(),
            spc700_steps: self.spc700_steps,
        }
    }

    fn advance_bootstrap_handshake(&mut self) {
        if self.ipl_rom.is_none() {
            return;
        }

        if self.apu_to_cpu_ports[0] == 0xAA && self.apu_to_cpu_ports[1] == 0xBB {
            if self.cpu_to_apu_ports[0] == 0xCC {
                self.apu_to_cpu_ports[0] = 0xCC;
                self.apu_to_cpu_ports[1] = self.cpu_to_apu_ports[1];
                self.apu_to_cpu_ports[2] = self.cpu_to_apu_ports[2];
                self.apu_to_cpu_ports[3] = self.cpu_to_apu_ports[3];
            }
            return;
        }

        // Keep the bootstrap upload handshake moving even while the full SPC700 IPL program
        // is not yet modeled. The CPU-side upload loop expects port acknowledgements to
        // advance with the values it writes.
        self.apu_to_cpu_ports = self.cpu_to_apu_ports;
    }
}

#[cfg(test)]
mod tests {
    use super::{Apu, SPC700_IPL_ROM_LEN};

    #[test]
    fn installs_valid_ipl_rom() {
        let mut apu = Apu::default();
        apu.install_ipl_rom_bytes(vec![0xAA; SPC700_IPL_ROM_LEN], None)
            .unwrap();
        assert!(apu.status().has_ipl_rom);
        assert_eq!(apu.ipl_rom().unwrap()[0], 0xAA);
    }

    #[test]
    fn rejects_invalid_ipl_rom_size() {
        let mut apu = Apu::default();
        let error = apu.install_ipl_rom_bytes(vec![0xAA; SPC700_IPL_ROM_LEN - 1], None);
        assert!(error.is_err());
    }

    #[test]
    fn port_roundtrip_and_step_accounting_work() {
        let mut apu = Apu::default();
        apu.write_cpu_port(0, 0x12).unwrap();
        apu.write_apu_port(3, 0x34).unwrap();
        apu.step_master_cycles(12);

        assert_eq!(apu.read_cpu_port(0).unwrap(), 0x12);
        assert_eq!(apu.read_apu_port(3).unwrap(), 0x34);
        assert_eq!(apu.status().spc700_steps, 2);
    }
}
