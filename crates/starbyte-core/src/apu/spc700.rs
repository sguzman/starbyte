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

    /// Advance the legacy non-executing placeholder without an O(steps) loop.
    ///
    /// This changes neither guest memory nor the real interpreter. The
    /// production frame loop still uses the placeholder for compatibility
    /// while authentic sound driver execution is under development.
    pub fn advance_placeholder_steps(&mut self, steps: u64) {
        if steps == 0 {
            return;
        }
        trace!(pc = self.pc, steps, "advancing placeholder spc700");
        self.pc = self.pc.wrapping_add(steps as u16);
        self.cycles = self.cycles.saturating_add(steps);
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
            // Standard SPC700 accumulator ALU: immediate, direct-page,
            // and absolute memory addressing. Separate from word ALU.
            0x08 | 0x28 | 0x48 | 0x68 | 0x88 | 0xA8 | 0x04 | 0x24 | 0x44 | 0x64 | 0x84 | 0xA4
            | 0x05 | 0x25 | 0x45 | 0x65 | 0x85 | 0xA5 => {
                self.execute_accumulator_alu(opcode, &mut read, &mut trace)
            }
            0x78 => self.execute_cmp_dp_imm(&mut read, &mut trace),
            0x7E => self.execute_cmp_y_dp(&mut read, &mut trace),
            0x50 => self.execute_bvc(&mut read, &mut trace),
            0x5C => self.execute_lsr_a(&mut read, &mut trace),
            0x5D => self.execute_mov_x_a(&mut read, &mut trace),
            0x5F => self.execute_jmp_abs(&mut read, &mut trace),
            0xE8 => self.execute_mov_a_imm(&mut read, &mut trace),
            0x40 => self.execute_setp(&mut read, &mut trace),
            0x70 => self.execute_bvs(&mut read, &mut trace),
            0x6D => self.execute_push_y(&mut read, &mut write, &mut trace),
            0x6F => self.execute_ret(&mut read, &mut trace),
            0x7C => self.execute_ror_a(&mut read, &mut trace),
            0x7D => self.execute_mov_a_x(&mut read, &mut trace),
            0xA0 => self.execute_ei(&mut read, &mut trace),
            0x8E => self.execute_pop_psw(&mut read, &mut trace),
            0x90 => self.execute_bcc(&mut read, &mut trace),
            0x9C => self.execute_dec_a(&mut read, &mut trace),
            0x9D => self.execute_mov_x_sp(&mut read, &mut trace),
            0xB0 => self.execute_bcs(&mut read, &mut trace),
            0xBC => self.execute_inc_a(&mut read, &mut trace),
            0xBD => self.execute_mov_sp_x(&mut read, &mut trace),
            0xC0 => self.execute_di(&mut read, &mut trace),
            0xCD => self.execute_mov_x_imm(&mut read, &mut trace),
            0xCE => self.execute_pop_x(&mut read, &mut trace),
            0xD0 => self.execute_bne(&mut read, &mut trace),
            0xDC => self.execute_dec_y(&mut read, &mut trace),
            0xDD => self.execute_mov_a_y(&mut read, &mut trace),
            0xED => self.execute_notc(&mut read, &mut trace),
            0xEE => self.execute_pop_y(&mut read, &mut trace),
            0x8D => self.execute_mov_y_imm(&mut read, &mut trace),
            0x8F => self.execute_mov_dp_imm(&mut read, &mut write, &mut trace),
            0xAB => self.execute_inc_dp(&mut read, &mut write, &mut trace),
            0xBA => self.execute_movw_ya_dp(&mut read, &mut trace),
            0xC6 => self.execute_mov_x_indirect_a(&mut read, &mut write, &mut trace),
            0xCB => self.execute_mov_dp_y(&mut read, &mut write, &mut trace),
            0xD7 => self.execute_mov_indirect_y_a(&mut read, &mut write, &mut trace),
            0xDA => self.execute_movw_dp_ya(&mut read, &mut write, &mut trace),
            0xEB => self.execute_mov_y_dp(&mut read, &mut trace),
            0xC4 => self.execute_mov_dp_a(&mut read, &mut write, &mut trace),
            0xE4 => self.execute_mov_a_dp(&mut read, &mut trace),
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
                address: u32::from(opcode_address),
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

    fn execute_accumulator_alu<FRead>(
        &mut self,
        opcode: u8,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let address_mode = opcode & 0x1f;
        let (rhs, instruction_len) = match address_mode {
            0x08 => (
                self.push_read_trace(read, trace, self.pc.wrapping_add(1)),
                2,
            ),
            0x04 => {
                let offset = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                let rhs = self.push_read_trace(read, trace, self.direct_page_address(offset));
                (rhs, 2)
            }
            0x05 => {
                let low = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                let high = self.push_read_trace(read, trace, self.pc.wrapping_add(2));
                let rhs = self.push_read_trace(read, trace, u16::from_le_bytes([low, high]));
                (rhs, 3)
            }
            _ => unreachable!("not an implemented SPC accumulator ALU mode"),
        };
        let lhs = self.a;
        let family = opcode & 0xe0;
        match family {
            0x00 => {
                self.a |= rhs;
                self.update_nz_flags(self.a);
            }
            0x20 => {
                self.a &= rhs;
                self.update_nz_flags(self.a);
            }
            0x40 => {
                self.a ^= rhs;
                self.update_nz_flags(self.a);
            }
            0x60 => self.update_cmp_flags(lhs, rhs),
            0x80 => {
                let carry = u16::from(self.psw & 0x01 != 0);
                let sum = u16::from(lhs) + u16::from(rhs) + carry;
                let result = sum as u8;
                self.a = result;
                self.psw &= !(0x01 | 0x08 | 0x40);
                if sum > 0xff {
                    self.psw |= 0x01;
                }
                if (u16::from(lhs & 0x0f) + u16::from(rhs & 0x0f) + carry) > 0x0f {
                    self.psw |= 0x08;
                }
                if (!(lhs ^ rhs) & (lhs ^ result) & 0x80) != 0 {
                    self.psw |= 0x40;
                }
                self.update_nz_flags(result);
            }
            0xA0 => {
                let borrow = u16::from(self.psw & 0x01 == 0);
                let subtrahend = u16::from(rhs) + borrow;
                let result = lhs.wrapping_sub(subtrahend as u8);
                self.a = result;
                self.psw &= !(0x01 | 0x08 | 0x40);
                if u16::from(lhs) >= subtrahend {
                    self.psw |= 0x01;
                }
                if u16::from(lhs & 0x0f) >= u16::from(rhs & 0x0f) + borrow {
                    self.psw |= 0x08;
                }
                if ((lhs ^ rhs) & (lhs ^ result) & 0x80) != 0 {
                    self.psw |= 0x40;
                }
                self.update_nz_flags(result);
            }
            _ => unreachable!("unknown SPC accumulator ALU family"),
        }
        self.pc = self.pc.wrapping_add(instruction_len);
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

    // MOV dp,#imm ($8F): the immediate precedes the direct-page address.
    // The PSW direct-page bit selects page $0000 or $0100.
    fn execute_mov_dp_imm<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        let immediate = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(2));
        let address = u16::from(dp) | if self.psw & 0x20 != 0 { 0x0100 } else { 0 };
        self.push_read_trace(read, trace, address);
        self.push_write_trace(write, trace, address, immediate);
        self.pc = self.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_mov_dp_a<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let address = u16::from(dp) | if self.psw & 0x20 != 0 { 0x0100 } else { 0 };
        self.push_read_trace(read, trace, address);
        self.push_write_trace(write, trace, address, self.a);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_mov_a_dp<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let address = u16::from(dp) | if self.psw & 0x20 != 0 { 0x0100 } else { 0 };
        self.a = self.push_read_trace(read, trace, address);
        self.update_nz_flags(self.a);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    /// MOV (X),A: write A to the active zero page at X.
    fn execute_mov_x_indirect_a<FRead, FWrite>(
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
        let address = self.direct_page_address(self.x);
        self.push_read_trace(read, trace, address);
        self.push_write_trace(write, trace, address, self.a);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    /// MOV Y,dp: direct-page read, updating N/Z.
    fn execute_mov_y_dp<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.y = self.push_read_trace(read, trace, self.direct_page_address(dp));
        self.update_nz_flags(self.y);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    /// MOV dp,Y: direct-page write without altering flags.
    fn execute_mov_dp_y<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let address = self.direct_page_address(dp);
        self.push_read_trace(read, trace, address);
        self.push_write_trace(write, trace, address, self.y);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    /// CMP dp,#imm: compare the memory value to the immediate operand.
    fn execute_cmp_dp_imm<FRead>(
        &mut self,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let imm = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(2));
        let value = self.push_read_trace(read, trace, self.direct_page_address(dp));
        self.update_cmp_flags(value, imm);
        self.pc = self.pc.wrapping_add(3);
        Ok(())
    }

    /// CMP Y,dp: compare the Y register to a direct-page value.
    fn execute_cmp_y_dp<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let value = self.push_read_trace(read, trace, self.direct_page_address(dp));
        self.update_cmp_flags(self.y, value);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_inc_dp<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let address = self.direct_page_address(dp);
        let previous = self.push_read_trace(read, trace, address);
        let value = previous.wrapping_add(1);
        self.push_write_trace(write, trace, address, value);
        self.update_nz_flags(value);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    /// MOVW YA,dp: 16-bit direct-page load with page-wrapped high byte.
    fn execute_movw_ya_dp<FRead>(
        &mut self,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.a = self.push_read_trace(read, trace, self.direct_page_address(dp));
        self.y = self.push_read_trace(read, trace, self.direct_page_address(dp.wrapping_add(1)));
        self.update_nz_word_flags(u16::from_le_bytes([self.a, self.y]));
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    /// MOVW dp,YA: 16-bit direct-page store with page-wrapped high byte.
    fn execute_movw_dp_ya<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let low_address = self.direct_page_address(dp);
        let high_address = self.direct_page_address(dp.wrapping_add(1));
        self.push_write_trace(write, trace, low_address, self.a);
        self.push_write_trace(write, trace, high_address, self.y);
        self.pc = self.pc.wrapping_add(2);
        Ok(())
    }

    /// MOV [dp]+Y,A: dereference a little-endian zero-page pointer.
    fn execute_mov_indirect_y_a<FRead, FWrite>(
        &mut self,
        read: &mut FRead,
        write: &mut FWrite,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
        FWrite: FnMut(u16, u8),
    {
        let dp = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let low = self.push_read_trace(read, trace, self.direct_page_address(dp));
        let high = self.push_read_trace(read, trace, self.direct_page_address(dp.wrapping_add(1)));
        let destination = u16::from_le_bytes([low, high]).wrapping_add(u16::from(self.y));
        self.push_write_trace(write, trace, destination, self.a);
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

    fn execute_mov_x_sp<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.x = self.sp;
        self.update_nz_flags(self.x);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_mov_sp_x<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        self.sp = self.x;
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_lsr_a<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let carry = self.a & 0x01 != 0;
        self.a >>= 1;
        self.psw = (self.psw & !0x01) | u8::from(carry);
        self.update_nz_flags(self.a);
        self.pc = self.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_ror_a<FRead>(&mut self, read: &mut FRead, trace: &mut Vec<BusEvent>) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let carry_in = (self.psw & 0x01) != 0;
        let carry_out = self.a & 0x01 != 0;
        self.a = (self.a >> 1) | (u8::from(carry_in) << 7);
        self.psw = (self.psw & !0x01) | u8::from(carry_out);
        self.update_nz_flags(self.a);
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

    fn direct_page_address(&self, offset: u8) -> u16 {
        u16::from(offset) | if self.psw & 0x20 != 0 { 0x0100 } else { 0 }
    }

    fn update_cmp_flags(&mut self, lhs: u8, rhs: u8) {
        self.update_nz_flags(lhs.wrapping_sub(rhs));
        if lhs >= rhs {
            self.psw |= 0x01;
        } else {
            self.psw &= !0x01;
        }
    }

    fn update_nz_word_flags(&mut self, value: u16) {
        self.psw &= !(0x80 | 0x02);
        if value & 0x8000 != 0 {
            self.psw |= 0x80;
        }
        if value == 0 {
            self.psw |= 0x02;
        }
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

#[cfg(test)]
mod accumulator_alu_tests {
    use std::cell::RefCell;

    use super::Spc700;
    use crate::bus::AccessKind;

    fn run(
        opcode: u8,
        operands: &[u8],
        a: u8,
        psw: u8,
        memory_data: &[(u16, u8)],
    ) -> (Spc700, Vec<crate::bus::BusEvent>, Vec<u8>) {
        let mut cpu = Spc700::default();
        cpu.load_state(0x8000, a, 0, 0, 0xef, psw);
        let mut bytes = vec![0_u8; 65_536];
        bytes[0x8000] = opcode;
        for (index, &byte) in operands.iter().enumerate() {
            bytes[0x8001 + index] = byte;
        }
        for &(address, value) in memory_data {
            bytes[usize::from(address)] = value;
        }
        let bus = RefCell::new(bytes);
        let trace = cpu
            .step_with_memory(
                |addr| bus.borrow()[usize::from(addr)],
                |addr, value| bus.borrow_mut()[usize::from(addr)] = value,
            )
            .unwrap();
        (cpu, trace, bus.into_inner())
    }

    #[test]
    fn immediate_alu_updates_flags_for_carry_halfcarry_and_signed_overflow() {
        for (opcode, a, rhs, initial_psw, expected_a, expected_nzcvh) in [
            (0x08, 0xF0, 0x0F, 0x00, 0xFF, 0x80),
            (0x28, 0xF0, 0x0F, 0x01, 0x00, 0x03),
            (0x48, 0x80, 0xFF, 0x00, 0x7F, 0x00),
            (0x68, 0x10, 0x11, 0x00, 0x10, 0x80),
            (0x68, 0x11, 0x11, 0x00, 0x11, 0x03),
            (0x88, 0x7F, 0x01, 0x00, 0x80, 0xC8),
            (0x88, 0xFF, 0x01, 0x01, 0x01, 0x09),
            (0xA8, 0x00, 0x01, 0x01, 0xFF, 0x80),
            (0xA8, 0x80, 0x01, 0x01, 0x7F, 0x41),
            (0xA8, 0x01, 0x01, 0x01, 0x00, 0x0B),
        ] {
            let (cpu, trace, _) = run(opcode, &[rhs], a, initial_psw, &[]);
            assert_eq!(cpu.a, expected_a, "opcode {opcode:02X}, A={a:02X}");
            assert_eq!(
                cpu.psw & 0xCB,
                expected_nzcvh,
                "flags for opcode {opcode:02X}, A={a:02X}"
            );
            assert_eq!(cpu.pc, 0x8002);
            assert_eq!(trace.len(), 2, "immediate opcode {opcode:02X}");
            assert!(trace.iter().all(|event| event.access == AccessKind::Read));
        }
    }

    #[test]
    fn direct_page_and_absolute_alu_addressing_cover_all_six_families() {
        for (dp_opcode, abs_opcode, a, data, carry, expected) in [
            (0x04, 0x05, 0xF0, 0x0F, 0, 0xFF), // OR
            (0x24, 0x25, 0xF0, 0x0F, 0, 0x00), // AND
            (0x44, 0x45, 0xF0, 0x0F, 0, 0xFF), // XOR
            (0x64, 0x65, 0x10, 0x0F, 0, 0x10), // CMP
            (0x84, 0x85, 0x10, 0x0F, 0, 0x1F), // ADC
            (0xA4, 0xA5, 0x10, 0x0F, 1, 0x01), // SBC
        ] {
            let (cpu, trace, bytes) = run(dp_opcode, &[0x42], a, 0x20 | carry, &[(0x0142, data)]);
            assert_eq!(cpu.a, expected, "direct-page opcode {dp_opcode:02X}");
            assert_eq!(cpu.pc, 0x8002);
            assert_eq!(trace.len(), 3);
            assert_eq!(trace[2].address, 0x0142);
            assert_eq!(bytes[0x0142], data, "ALU must not mutate source");
            let (cpu, trace, bytes) = run(abs_opcode, &[0x34, 0x92], a, carry, &[(0x9234, data)]);
            assert_eq!(cpu.a, expected, "absolute opcode {abs_opcode:02X}");
            assert_eq!(cpu.pc, 0x8003);
            assert_eq!(trace.len(), 4);
            assert_eq!(trace[3].address, 0x9234);
            assert_eq!(bytes[0x9234], data);
        }
    }

    #[test]
    fn adc_and_sbc_preserve_unrelated_direct_page_interrupt_flags() {
        let flags = 0x34; // PSW P, B and I; C is clear.
        let (cpu, _, _) = run(0x88, &[0x10], 0x10, flags, &[]);
        assert_eq!(cpu.psw & 0x34, flags);
        let (cpu, _, _) = run(0xA8, &[0x10], 0x10, flags, &[]);
        assert_eq!(cpu.psw & 0x34, flags);
    }
}

#[cfg(test)]
mod placeholder_tests {
    use super::Spc700;

    #[test]
    fn bulk_dummy_clock_matches_individual_steps_across_pc_wrap() {
        let mut bulk = Spc700::default();
        bulk.load_state(0xfffe, 1, 2, 3, 0xef, 0x02);
        let mut single = bulk.clone();
        bulk.advance_placeholder_steps(70_000);
        for _ in 0..70_000 {
            single.step();
        }
        assert_eq!(bulk, single);
        bulk.advance_placeholder_steps(0);
        assert_eq!(bulk, single);
        assert_eq!(bulk.cycles(), 70_000);
    }
}
