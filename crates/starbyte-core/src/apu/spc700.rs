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
            0x0D => self.execute_push_psw(&mut read, &mut write, &mut trace),
            0x01 | 0x11 | 0x21 | 0x31 | 0x41 | 0x51 | 0x61 | 0x71 | 0x81 | 0x91 | 0xA1 | 0xB1
            | 0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                self.execute_tcall(opcode, &mut read, &mut write, &mut trace)
            }
            0x10 => self.execute_bpl(&mut read, &mut trace),
            0x1F => self.execute_jmp_abs_x_indirect(&mut read, &mut trace),
            0x20 => self.execute_clrp(&mut read, &mut trace),
            0x2F => self.execute_bra(&mut read, &mut trace),
            0x2D => self.execute_push_a(&mut read, &mut write, &mut trace),
            0x30 => self.execute_bmi(&mut read, &mut trace),
            0x3F => self.execute_call_abs(&mut read, &mut write, &mut trace),
            0x4D => self.execute_push_x(&mut read, &mut write, &mut trace),
            0x50 => self.execute_bvc(&mut read, &mut trace),
            0x5D => self.execute_mov_x_a(&mut read, &mut trace),
            0x5F => self.execute_jmp_abs(&mut read, &mut trace),
            0xE8 => self.execute_mov_a_imm(&mut read, &mut trace),
            0x40 => self.execute_setp(&mut read, &mut trace),
            0x70 => self.execute_bvs(&mut read, &mut trace),
            0x6D => self.execute_push_y(&mut read, &mut write, &mut trace),
            0x6F => self.execute_ret(&mut read, &mut trace),
            0x7D => self.execute_mov_a_x(&mut read, &mut trace),
            0xA0 => self.execute_ei(&mut read, &mut trace),
            0x8E => self.execute_pop_psw(&mut read, &mut trace),
            0x90 => self.execute_bcc(&mut read, &mut trace),
            0x9C => self.execute_dec_a(&mut read, &mut trace),
            0xB0 => self.execute_bcs(&mut read, &mut trace),
            0xBC => self.execute_inc_a(&mut read, &mut trace),
            0xC0 => self.execute_di(&mut read, &mut trace),
            0xCD => self.execute_mov_x_imm(&mut read, &mut trace),
            0xCE => self.execute_pop_x(&mut read, &mut trace),
            0xD0 => self.execute_bne(&mut read, &mut trace),
            0xDC => self.execute_dec_y(&mut read, &mut trace),
            0xDD => self.execute_mov_a_y(&mut read, &mut trace),
            0xED => self.execute_notc(&mut read, &mut trace),
            0xEE => self.execute_pop_y(&mut read, &mut trace),
            0x8D => self.execute_mov_y_imm(&mut read, &mut trace),
            0xF0 => self.execute_beq(&mut read, &mut trace),
            0xFC => self.execute_inc_y(&mut read, &mut trace),
            0xFD => self.execute_mov_y_a(&mut read, &mut trace),
            0x7F => self.execute_ret1(&mut read, &mut trace),
            0x60 => self.execute_clrc(&mut read, &mut trace),
            0x80 => self.execute_setc(&mut read, &mut trace),
            0x1D => self.execute_dec_x(&mut read, &mut trace),
            0x3D => self.execute_inc_x(&mut read, &mut trace),
            0xAE => self.execute_pop_a(&mut read, &mut trace),
            _ => Err(Error::UnsupportedOpcode {
                cpu: "SPC700",
                opcode,
            }),
        }?;

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
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_clrp<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.psw &= !0x20;
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_bpl<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.execute_branch_relative(read, trace, self.psw & 0x80 == 0)
    }

    fn execute_bra<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.execute_branch_relative(read, trace, true)
    }

    fn execute_tcall<FRead, FWrite>(
        &mut self,
        opcode: u8,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);

        let return_pc = self.pc.wrapping_add(1);
        self.push_stack(write, trace, (return_pc >> 8) as u8);
        self.push_stack(write, trace, (return_pc & 0x00FF) as u8);

        self.push_wait_trace(trace);

        let vector_base = 0xFFDEu16.wrapping_sub(u16::from(opcode >> 4) * 2);
        let low = self.push_read_trace(read, trace, vector_base);
        let high = self.push_read_trace(read, trace, vector_base.wrapping_add(1));
        self.pc = u16::from_le_bytes([low, high]);
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
        let operand = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.a = operand;
        self.update_nz_flags(self.a);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_mov_x_imm<FRead>(
        &mut self,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let operand = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.x = operand;
        self.update_nz_flags(self.x);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_mov_y_imm<FRead>(
        &mut self,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let operand = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.y = operand;
        self.update_nz_flags(self.y);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_mov_x_a<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.x = self.a;
        self.update_nz_flags(self.x);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_mov_a_x<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.a = self.x;
        self.update_nz_flags(self.a);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_mov_a_y<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.a = self.y;
        self.update_nz_flags(self.a);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_mov_y_a<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.y = self.a;
        self.update_nz_flags(self.y);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_clrc<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.psw &= !0x01;
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_setc<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.psw |= 0x01;
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_setp<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.psw |= 0x20;
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_bmi<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.execute_branch_relative(read, trace, self.psw & 0x80 != 0)
    }

    fn execute_jmp_abs_x_indirect<FRead>(
        &mut self,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let low = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let high = self.push_read_trace(read, trace, self.pc.wrapping_add(2));
        self.push_wait_trace(trace);

        let pointer = u16::from_le_bytes([low, high]).wrapping_add(u16::from(self.x));
        let target_low = self.push_read_trace(read, trace, pointer);
        let target_high = self.push_read_trace(read, trace, pointer.wrapping_add(1));
        self.pc = u16::from_le_bytes([target_low, target_high]);
        Ok(())
    }

    fn execute_bvc<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.execute_branch_relative(read, trace, self.psw & 0x40 == 0)
    }

    fn execute_bvs<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.execute_branch_relative(read, trace, self.psw & 0x40 != 0)
    }

    fn execute_bcc<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.execute_branch_relative(read, trace, self.psw & 0x01 == 0)
    }

    fn execute_bcs<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.execute_branch_relative(read, trace, self.psw & 0x01 != 0)
    }

    fn execute_dec_a<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.a = self.a.wrapping_sub(1);
        self.update_nz_flags(self.a);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_inc_a<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.a = self.a.wrapping_add(1);
        self.update_nz_flags(self.a);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_dec_y<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.y = self.y.wrapping_sub(1);
        self.update_nz_flags(self.y);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_inc_y<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.y = self.y.wrapping_add(1);
        self.update_nz_flags(self.y);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_push_a<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_stack(write, trace, self.a);
        self.push_wait_trace(trace);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_push_psw<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_stack(write, trace, self.psw);
        self.push_wait_trace(trace);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_push_x<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_stack(write, trace, self.x);
        self.push_wait_trace(trace);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_push_y<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_stack(write, trace, self.y);
        self.push_wait_trace(trace);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_ret<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);
        let low = self.pop_stack(read, trace);
        let high = self.pop_stack(read, trace);
        self.pc = u16::from_le_bytes([low, high]);
        Ok(())
    }

    fn execute_ret1<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);
        self.psw = self.pop_stack(read, trace);
        let low = self.pop_stack(read, trace);
        let high = self.pop_stack(read, trace);
        self.pc = u16::from_le_bytes([low, high]);
        Ok(())
    }

    fn execute_ei<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);
        self.psw |= 0x04;
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_di<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);
        self.psw &= !0x04;
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_pop_psw<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);
        self.psw = self.pop_stack(read, trace);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_pop_a<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);
        self.a = self.pop_stack(read, trace);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_pop_x<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);
        self.x = self.pop_stack(read, trace);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_pop_y<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);
        self.y = self.pop_stack(read, trace);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_call_abs<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        let low = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let high = self.push_read_trace(read, trace, self.pc.wrapping_add(2));
        self.push_wait_trace(trace);

        let return_pc = self.pc.wrapping_add(3);
        self.push_stack(write, trace, (return_pc >> 8) as u8);
        self.push_stack(write, trace, (return_pc & 0x00FF) as u8);
        self.push_wait_trace(trace);
        self.push_wait_trace(trace);

        self.pc = u16::from_le_bytes([low, high]);
        Ok(())
    }

    fn execute_jmp_abs<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let low = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let high = self.push_read_trace(read, trace, self.pc.wrapping_add(2));
        self.pc = u16::from_le_bytes([low, high]);
        Ok(())
    }

    fn execute_dec_x<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.x = self.x.wrapping_sub(1);
        self.update_nz_flags(self.x);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_inc_x<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.x = self.x.wrapping_add(1);
        self.update_nz_flags(self.x);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_notc<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.push_wait_trace(trace);
        self.psw ^= 0x01;
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_bne<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.execute_branch_relative(read, trace, self.psw & 0x02 == 0)
    }

    fn execute_beq<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.execute_branch_relative(read, trace, self.psw & 0x02 != 0)
    }

    fn push_read_trace<FRead>(
        &self,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
        address: u16,
    ) -> u8
    where
        FRead: FnMut(u16) -> u8,
    {
        let value = read(address);
        trace.push(BusEvent {
            address: u32::from(address),
            value,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });
        value
    }

    fn push_wait_trace(&self, trace: &mut Vec<BusEvent>) {
        trace.push(BusEvent {
            address: 0,
            value: 0,
            access: AccessKind::Wait,
            cycle: trace.len() as u64,
        });
    }

    fn push_write_trace<FWrite>(
        &self,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
        address: u16,
        value: u8,
    ) where
        FWrite: FnMut(u16, u8),
    {
        write(address, value);
        trace.push(BusEvent {
            address: u32::from(address),
            value,
            access: AccessKind::Write,
            cycle: trace.len() as u64,
        });
    }

    fn push_stack<FWrite>(&mut self, write: &mut FWrite, trace: &mut Vec<BusEvent>, value: u8)
    where
        FWrite: FnMut(u16, u8),
    {
        let address = 0x0100 | u16::from(self.sp);
        self.push_write_trace(write, trace, address, value);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pop_stack<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> u8
    where
        FRead: FnMut(u16) -> u8,
    {
        self.sp = self.sp.wrapping_add(1);
        let address = 0x0100 | u16::from(self.sp);
        self.push_read_trace(read, trace, address)
    }

    fn execute_branch_relative<FRead>(
        &mut self,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
        condition: bool,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let displacement = self.push_read_trace(read, trace, self.pc.wrapping_add(1)) as i8;
        let next_pc = self.pc.wrapping_add(2);
        if condition {
            self.push_wait_trace(trace);
            self.push_wait_trace(trace);
            self.pc = next_pc.wrapping_add_signed(i16::from(displacement));
        } else {
            self.pc = next_pc;
        }
        Ok(())
    }

    fn update_nz_flags(&mut self, value: u8) {
        self.psw &= !(0x80 | 0x02);
        if value & 0x80 != 0 {
            self.psw |= 0x80;
        }
        if value == 0 {
            self.psw |= 0x02;
        }
    }
}
