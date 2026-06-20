//! 65816 CPU scaffolding.

pub mod registers;

use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bus::Address;

/// Minimal bootstrap CPU core state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Cpu65816 {
    /// Architectural register file.
    pub registers: registers::Registers,
    cycles: u64,
}

impl Cpu65816 {
    /// Reset to a known power-on-like placeholder state.
    pub fn reset(&mut self) {
        self.registers = registers::Registers::default();
        self.cycles = 0;
    }

    /// Load a register snapshot and reset cycle accounting for compliance work.
    pub fn load_registers(&mut self, registers: registers::Registers) {
        self.registers = registers;
        self.cycles = 0;
    }

    /// Execute one placeholder instruction step.
    pub fn step(&mut self) {
        trace!(
            pc = self.registers.pc,
            cycles = self.cycles,
            "stepping 65816 placeholder"
        );
        self.registers.pc = self.registers.pc.wrapping_add(1);
        self.cycles = self.cycles.saturating_add(1);
    }

    /// Current program counter expressed as a bus address.
    #[must_use]
    pub fn program_address(&self) -> Address {
        (u32::from(self.registers.pbr) << 16) | u32::from(self.registers.pc)
    }

    /// Total executed cycles in the placeholder model.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cycles
    }
}
