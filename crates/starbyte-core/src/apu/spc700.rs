//! SPC700 bootstrap state.

use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bus::{AccessKind, BusEvent};
use crate::error::{Error, Result};

/// Minimal SPC700 state for harness scaffolding.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spc700 {
    /// Program counter.
    pub pc: u16,
    /// Accumulator.
    pub a: u8,
    /// X register.
    pub x: u8,
    /// Y register.
    pub y: u8,
    /// Stack pointer.
    pub sp: u8,
    /// Status register.
    pub psw: u8,
    cycles: u64,
}

impl Spc700 {
    /// Reset placeholder state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Execute one placeholder step.
    pub fn step(&mut self) {
        trace!(
            pc = self.pc,
            cycles = self.cycles,
            "stepping spc700 placeholder"
        );
        self.pc = self.pc.wrapping_add(1);
        self.cycles = self.cycles.saturating_add(1);
    }

    /// Execute one instruction against a 64 KiB memory callback and return the trace.
    pub fn step_with_memory<FRead, FWrite>(
        &mut self,
        mut read: FRead,
        mut write: FWrite,
    ) -> Result<Vec<BusEvent>>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        let opcode_address = self.pc;
        let opcode = read(opcode_address);
        let mut trace = vec![BusEvent {
            address: u32::from(opcode_address),
            value: opcode,
            access: AccessKind::Read,
            cycle: 0,
        }];

        match opcode {
            0x00 => self.execute_nop(&mut read, &mut trace),
            0xE8 => self.execute_mov_a_imm(&mut read, &mut trace),
            _ => Err(Error::UnsupportedOpcode {
                cpu: "SPC700",
                opcode,
            }),
        }?;

        let _ = &mut write;
        self.cycles = trace.len() as u64;
        Ok(trace)
    }

    /// Load a register snapshot and reset cycle accounting for compliance work.
    pub fn load_state(&mut self, pc: u16, a: u8, x: u8, y: u8, sp: u8, psw: u8) {
        self.pc = pc;
        self.a = a;
        self.x = x;
        self.y = y;
        self.sp = sp;
        self.psw = psw;
        self.cycles = 0;
    }

    /// Total executed cycles in the placeholder model.
    #[must_use]
    pub const fn cycles(&self) -> u64 {
        self.cycles
    }

    fn execute_nop<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let next_address = self.pc.wrapping_add(1);
        let value = read(next_address);
        trace.push(BusEvent {
            address: u32::from(next_address),
            value,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_mov_a_imm<FRead>(
        &mut self,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let operand_address = self.pc.wrapping_add(1);
        let operand = read(operand_address);
        trace.push(BusEvent {
            address: u32::from(operand_address),
            value: operand,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });
        self.a = operand;
        self.psw &= !(0x80 | 0x02);
        if self.a & 0x80 != 0 {
            self.psw |= 0x80;
        }
        if self.a == 0 {
            self.psw |= 0x02;
        }
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }
}
