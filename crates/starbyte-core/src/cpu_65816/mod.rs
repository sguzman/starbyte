//! 65816 CPU scaffolding.

pub mod registers;

use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bus::{AccessKind, Address, Bus, BusEvent};
use crate::error::{Error, Result};

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

    /// Execute one instruction against a bus and return the captured bus trace.
    pub fn step_with_bus<B: Bus>(&mut self, bus: &mut B) -> Result<Vec<BusEvent>> {
        let opcode_address = self.program_address();
        let opcode = bus.read(opcode_address);
        let mut trace = vec![BusEvent {
            address: opcode_address,
            value: opcode,
            access: AccessKind::Read,
            cycle: 0,
        }];

        match opcode {
            0xEA => self.execute_nop(bus, &mut trace),
            0x00 => self.execute_brk(bus, &mut trace),
            _ => Err(Error::UnsupportedOpcode {
                cpu: "65816",
                opcode,
            }),
        }?;

        self.cycles = trace.len() as u64;
        Ok(trace)
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

    fn execute_nop<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let next_address = self.program_address().wrapping_add(1);
        let value = bus.read(next_address);
        trace.push(BusEvent {
            address: next_address,
            value,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_brk<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        if self.registers.emulation {
            return Err(Error::Unimplemented("65816 BRK emulation mode"));
        }

        let signature_address = self.program_address().wrapping_add(1);
        let signature = bus.read(signature_address);
        trace.push(BusEvent {
            address: signature_address,
            value: signature,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });

        let return_pc = self.registers.pc.wrapping_add(2);
        self.push_stack(bus, trace, self.registers.pbr)?;
        self.push_stack(bus, trace, (return_pc >> 8) as u8)?;
        self.push_stack(bus, trace, (return_pc & 0x00FF) as u8)?;
        self.push_stack(bus, trace, self.registers.p)?;

        let vector_low = bus.read(0x00FFE6);
        trace.push(BusEvent {
            address: 0x00FFE6,
            value: vector_low,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });
        let vector_high = bus.read(0x00FFE7);
        trace.push(BusEvent {
            address: 0x00FFE7,
            value: vector_high,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });

        self.registers.pc = u16::from_le_bytes([vector_low, vector_high]);
        self.registers.pbr = 0;
        self.registers.p = (self.registers.p | 0x04) & !0x08;
        Ok(())
    }

    fn push_stack<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        value: u8,
    ) -> Result<()> {
        let address = u32::from(self.registers.s);
        bus.write(address, value);
        trace.push(BusEvent {
            address,
            value,
            access: AccessKind::Write,
            cycle: trace.len() as u64,
        });
        self.registers.s = self.registers.s.wrapping_sub(1);
        Ok(())
    }
}
