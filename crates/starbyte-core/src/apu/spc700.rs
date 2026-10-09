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
            | 0x05 | 0x25 | 0x45 | 0x65 | 0x85 | 0xA5 | 0x06 | 0x26 | 0x46 | 0x66 | 0x86 | 0xA6
            | 0x07 | 0x27 | 0x47 | 0x67 | 0x87 | 0xA7 | 0x14 | 0x34 | 0x54 | 0x74 | 0x94 | 0xB4
            | 0x15 | 0x35 | 0x55 | 0x75 | 0x95 | 0xB5 | 0x16 | 0x36 | 0x56 | 0x76 | 0x96 | 0xB6
            | 0x17 | 0x37 | 0x57 | 0x77 | 0x97 | 0xB7 => {
                self.execute_accumulator_alu(opcode, &mut read, &mut trace)
            }
            0x18 | 0x38 | 0x58 | 0x78 | 0x98 | 0xB8 | 0x09 | 0x29 | 0x49 | 0x69 | 0x89 | 0xA9
            | 0x19 | 0x39 | 0x59 | 0x79 | 0x99 | 0xB9 => {
                self.execute_memory_alu(opcode, &mut read, &mut write, &mut trace)
            }
            0x7E => self.execute_cmp_y_dp(&mut read, &mut trace),
            0x50 => self.execute_bvc(&mut read, &mut trace),
            0x5C => self.execute_lsr_a(&mut read, &mut trace),
            0x5D => self.execute_mov_x_a(&mut read, &mut trace),
            0x5F => self.execute_jmp_abs(&mut read, &mut trace),
            0xE8 => self.execute_mov_a_imm(&mut read, &mut trace),
            0xE5 | 0xE6 | 0xE7 | 0xF4 | 0xF5 | 0xF6 | 0xF7 => {
                self.execute_mov_a_addressed(opcode, &mut read, &mut trace)
            }
            0xBF => self.execute_mov_a_x_increment(&mut read, &mut trace),
            0xE9 | 0xEC | 0xF8 | 0xF9 | 0xFB => {
                self.execute_mov_index_load(opcode, &mut read, &mut trace)
            }
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

    /// Read an SPC700 accumulator operand with its addressing bus events.
    /// Shared by arithmetic and MOV A modes to keep DP pointer wrapping
    /// and indexed addressing consistent across instruction families.
    fn read_accumulator_operand<FRead>(
        &self,
        opcode: u8,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> (u8, u16)
    where
        FRead: FnMut(u16) -> u8,
    {
        let address_mode = opcode & 0x1f;
        match address_mode {
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
            0x06 => {
                // (X): one-byte opcode with a dummy bus read before DP access.
                self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                let rhs = self.push_read_trace(read, trace, self.direct_page_address(self.x));
                (rhs, 1)
            }
            0x07 => {
                // [dp+X]: add X within the selected direct page before
                // fetching the little-endian pointer. Both pointer bytes
                // wrap at 0xFF without escaping the active direct-page bank.
                let offset = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                self.push_wait_trace(trace);
                let index = offset.wrapping_add(self.x);
                let low = self.push_read_trace(read, trace, self.direct_page_address(index));
                let high = self.push_read_trace(
                    read,
                    trace,
                    self.direct_page_address(index.wrapping_add(1)),
                );
                let rhs = self.push_read_trace(read, trace, u16::from_le_bytes([low, high]));
                (rhs, 2)
            }
            0x14 => {
                let offset = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                self.push_wait_trace(trace);
                let rhs = self.push_read_trace(
                    read,
                    trace,
                    self.direct_page_address(offset.wrapping_add(self.x)),
                );
                (rhs, 2)
            }
            0x15 | 0x16 => {
                let low = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                let high = self.push_read_trace(read, trace, self.pc.wrapping_add(2));
                self.push_wait_trace(trace);
                let index = if address_mode == 0x15 { self.x } else { self.y };
                let effective = u16::from_le_bytes([low, high]).wrapping_add(u16::from(index));
                let rhs = self.push_read_trace(read, trace, effective);
                (rhs, 3)
            }
            0x17 => {
                // [dp]+Y: indirect pointer comes from unindexed direct
                // page, then Y indexes the 16-bit effective address.
                let offset = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                let low = self.push_read_trace(read, trace, self.direct_page_address(offset));
                let high = self.push_read_trace(
                    read,
                    trace,
                    self.direct_page_address(offset.wrapping_add(1)),
                );
                self.push_wait_trace(trace);
                let effective = u16::from_le_bytes([low, high]).wrapping_add(u16::from(self.y));
                let rhs = self.push_read_trace(read, trace, effective);
                (rhs, 2)
            }
            _ => unreachable!("not an implemented SPC accumulator ALU mode"),
        }
    }

    /// Apply one SPC700 8-bit arithmetic/logical operator to an arbitrary
    /// destination byte. The accumulator and direct-page memory forms use
    /// identical N/V/H/Z/C semantics; CMP only changes flags.
    fn calculate_alu(&mut self, opcode: u8, lhs: u8, rhs: u8) -> u8 {
        match opcode & 0xe0 {
            0x00 => {
                let result = lhs | rhs;
                self.update_nz_flags(result);
                result
            }
            0x20 => {
                let result = lhs & rhs;
                self.update_nz_flags(result);
                result
            }
            0x40 => {
                let result = lhs ^ rhs;
                self.update_nz_flags(result);
                result
            }
            0x60 => {
                self.update_cmp_flags(lhs, rhs);
                lhs
            }
            0x80 => {
                let carry = u16::from(self.psw & 0x01 != 0);
                let sum = u16::from(lhs) + u16::from(rhs) + carry;
                let result = sum as u8;
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
                result
            }
            0xA0 => {
                let borrow = u16::from(self.psw & 0x01 == 0);
                let subtrahend = u16::from(rhs) + borrow;
                let result = lhs.wrapping_sub(subtrahend as u8);
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
                result
            }
            _ => unreachable!("unsupported SPC700 ALU family"),
        }
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
        let (rhs, instruction_len) = self.read_accumulator_operand(opcode, read, trace);
        self.a = self.calculate_alu(opcode, self.a, rhs);
        self.pc = self.pc.wrapping_add(instruction_len);
        Ok(())
    }

    fn execute_mov_a_addressed<FRead>(
        &mut self,
        opcode: u8,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let (value, instruction_len) = self.read_accumulator_operand(opcode, read, trace);
        self.a = value;
        self.update_nz_flags(value);
        self.pc = self.pc.wrapping_add(instruction_len);
        Ok(())
    }

    fn execute_mov_index_load<FRead>(
        &mut self,
        opcode: u8,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        let (value, len) = match opcode {
            0xF8 => self.read_accumulator_operand(0xE4, read, trace), // MOV X,dp
            0xF9 => {
                // MOV X,dp+Y, including wrap within the active direct page.
                let offset = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                self.push_wait_trace(trace);
                let value = self.push_read_trace(
                    read,
                    trace,
                    self.direct_page_address(offset.wrapping_add(self.y)),
                );
                (value, 2)
            }
            0xE9 | 0xEC => self.read_accumulator_operand(0xE5, read, trace), // MOV X/Y,abs
            0xFB => self.read_accumulator_operand(0xF4, read, trace),        // MOV Y,dp+X
            _ => unreachable!("not a supported SPC index-register load"),
        };
        if matches!(opcode, 0xF8 | 0xF9 | 0xE9) {
            self.x = value;
        } else {
            self.y = value;
        }
        self.update_nz_flags(value);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    fn execute_mov_a_x_increment<FRead>(
        &mut self,
        read: &mut FRead,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()>
    where
        FRead: FnMut(u16) -> u8,
    {
        // MOV A,(X)+ reads the current direct-page X address and advances
        // X modulo 256 without crossing to the other direct-page bank.
        self.push_read_trace(read, trace, self.pc.wrapping_add(1));
        let value = self.push_read_trace(read, trace, self.direct_page_address(self.x));
        self.push_wait_trace(trace);
        self.a = value;
        self.x = self.x.wrapping_add(1);
        self.update_nz_flags(value);
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
    /// SPC700 memory-destination ALU instructions. Instruction operands
    /// are encoded source first, destination last (the reverse of assembly
    /// syntax); (X),(Y) treats X as destination, Y as source.
    fn execute_memory_alu<FRead, FWrite>(
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
        let (address, lhs, rhs, len) = match opcode & 0x1f {
            0x18 => {
                let immediate = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                let dest = self.push_read_trace(read, trace, self.pc.wrapping_add(2));
                let address = self.direct_page_address(dest);
                let lhs = self.push_read_trace(read, trace, address);
                (address, lhs, immediate, 3)
            }
            0x09 => {
                let source = self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                let dest = self.push_read_trace(read, trace, self.pc.wrapping_add(2));
                let rhs = self.push_read_trace(read, trace, self.direct_page_address(source));
                let address = self.direct_page_address(dest);
                let lhs = self.push_read_trace(read, trace, address);
                (address, lhs, rhs, 3)
            }
            0x19 => {
                self.push_read_trace(read, trace, self.pc.wrapping_add(1));
                let rhs = self.push_read_trace(read, trace, self.direct_page_address(self.y));
                let address = self.direct_page_address(self.x);
                let lhs = self.push_read_trace(read, trace, address);
                (address, lhs, rhs, 1)
            }
            _ => unreachable!("unsupported SPC700 memory-destination ALU mode"),
        };
        let result = self.calculate_alu(opcode, lhs, rhs);
        if opcode & 0xe0 == 0x60 {
            // CMP never mutates the destination. Its final bus slot is idle.
            self.push_wait_trace(trace);
        } else {
            self.push_write_trace(write, trace, address, result);
        }
        self.pc = self.pc.wrapping_add(len);
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
        run_indexed(opcode, operands, a, psw, 0, 0, memory_data)
    }

    fn run_indexed(
        opcode: u8,
        operands: &[u8],
        a: u8,
        psw: u8,
        x: u8,
        y: u8,
        memory_data: &[(u16, u8)],
    ) -> (Spc700, Vec<crate::bus::BusEvent>, Vec<u8>) {
        let mut cpu = Spc700::default();
        cpu.load_state(0x8000, a, x, y, 0xef, psw);
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
    fn indexed_alu_modes_apply_spc_direct_page_and_absolute_wrap_rules() {
        let x = 3;
        let y = 5;
        for (base, a, carry, expected) in [
            (0x00_u8, 0x10, 0, 0x1F), // OR
            (0x20, 0x10, 0, 0x00),    // AND
            (0x40, 0x10, 0, 0x1F),    // EOR
            (0x60, 0x10, 0, 0x10),    // CMP
            (0x80, 0x10, 0, 0x1F),    // ADC
            (0xA0, 0x10, 1, 0x01),    // SBC
        ] {
            for mode in [0x06_u8, 0x07, 0x14, 0x15, 0x16, 0x17] {
                let opcode = base | mode;
                let (operands, mut memory, expected_address, expected_len, expected_cycles) =
                    match mode {
                        0x06 => (vec![], vec![], 0x0103, 1, 3),
                        0x07 => (
                            vec![0xfc],
                            vec![(0x01ff, 0x30), (0x0100, 0x40)],
                            0x4030,
                            2,
                            6,
                        ),
                        0x14 => (vec![0xfe], vec![], 0x0101, 2, 4),
                        0x15 => (vec![0xfe, 0xff], vec![], 0x0001, 3, 5),
                        0x16 => (vec![0xfe, 0xff], vec![], 0x0003, 3, 5),
                        0x17 => (
                            vec![0xff],
                            vec![(0x01ff, 0x30), (0x0100, 0x40)],
                            0x4035,
                            2,
                            6,
                        ),
                        _ => unreachable!(),
                    };
                memory.push((expected_address, 0x0f));
                let (cpu, trace, bytes) =
                    run_indexed(opcode, &operands, a, 0x20 | carry, x, y, &memory);
                assert_eq!(cpu.a, expected, "opcode {opcode:02X}");
                assert_eq!(cpu.pc, 0x8000 + expected_len, "opcode {opcode:02X}");
                assert_eq!(trace.len(), expected_cycles, "opcode {opcode:02X}");
                assert_eq!(
                    trace.last().unwrap().address,
                    u32::from(expected_address),
                    "opcode {opcode:02X} did not access expected target"
                );
                assert_eq!(
                    bytes[usize::from(expected_address)],
                    0x0f,
                    "operand must be unchanged"
                );
                assert!(trace.iter().any(|event| event.access == AccessKind::Read));
            }
        }
    }

    #[test]
    fn mov_a_indexed_forms_share_alu_addressing_and_update_only_nz() {
        for (opcode, operands, initial_data, expected_address, expected_len, cycles) in [
            (0xE6_u8, vec![], vec![], 0x0103, 1, 3),
            (
                0xE7,
                vec![0xfc],
                vec![(0x01ff, 0x30), (0x0100, 0x40)],
                0x4030,
                2,
                6,
            ),
            (0xE5, vec![0xfe, 0xff], vec![], 0xfffe, 3, 4),
            (0xF4, vec![0xfe], vec![], 0x0101, 2, 4),
            (0xF5, vec![0xfe, 0xff], vec![], 0x0001, 3, 5),
            (0xF6, vec![0xfe, 0xff], vec![], 0x0003, 3, 5),
            (
                0xF7,
                vec![0xff],
                vec![(0x01ff, 0x30), (0x0100, 0x40)],
                0x4035,
                2,
                6,
            ),
        ] {
            let mut initial_data = initial_data;
            initial_data.push((expected_address, 0x80));
            let (cpu, trace, bytes) =
                run_indexed(opcode, &operands, 0xAA, 0x23, 3, 5, &initial_data);
            assert_eq!(cpu.a, 0x80, "opcode {opcode:02X}");
            assert_eq!(cpu.x, 3, "opcode {opcode:02X}");
            assert_eq!(cpu.y, 5, "opcode {opcode:02X}");
            assert_eq!(cpu.pc, 0x8000 + expected_len, "opcode {opcode:02X}");
            assert_eq!(cpu.psw & 0x83, 0x81, "opcode {opcode:02X} N/Z/C");
            assert_eq!(trace.len(), cycles, "opcode {opcode:02X}");
            assert_eq!(trace.last().unwrap().address, u32::from(expected_address));
            assert_eq!(bytes[usize::from(expected_address)], 0x80);
        }
    }

    #[test]
    fn mov_x_y_indexed_loads_preserve_other_register_and_wrap_direct_page() {
        for (opcode, operands, target, len, cycles, writes_x) in [
            (0xF8_u8, vec![0xff], 0x01ff_u16, 2, 3, true),
            (0xF9, vec![0xfc], 0x0101, 2, 4, true),
            (0xE9, vec![0x34, 0x12], 0x1234, 3, 4, true),
            (0xEC, vec![0x34, 0x12], 0x1234, 3, 4, false),
            (0xFB, vec![0xfe], 0x0101, 2, 4, false),
        ] {
            let (cpu, trace, bytes) =
                run_indexed(opcode, &operands, 0x55, 0x22, 3, 5, &[(target, 0x80)]);
            assert_eq!(cpu.a, 0x55, "opcode {opcode:02X} must preserve A");
            assert_eq!(
                (cpu.x, cpu.y),
                if writes_x { (0x80, 5) } else { (3, 0x80) },
                "opcode {opcode:02X}"
            );
            assert_eq!(cpu.pc, 0x8000 + len);
            assert_eq!(cpu.psw & 0x83, 0x80, "N/Z/C");
            assert_eq!(trace.len(), cycles);
            assert_eq!(trace.last().unwrap().address, u32::from(target));
            assert_eq!(bytes[usize::from(target)], 0x80);
        }
    }

    #[test]
    fn mov_x_from_zero_sets_z_without_clearing_carry() {
        let (cpu, trace, _) = run_indexed(0xF9, &[0xfc], 0xaa, 0x21, 3, 5, &[(0x0101, 0)]);
        assert_eq!(cpu.x, 0);
        assert_eq!(cpu.y, 5);
        assert_eq!(cpu.psw & 0x83, 0x03);
        assert_eq!(trace.len(), 4);
    }

    #[test]
    fn mov_a_x_postincrement_wraps_without_leaving_selected_direct_page() {
        let (cpu, trace, bytes) = run_indexed(0xBF, &[], 0, 0x22, 0xff, 0, &[(0x01ff, 0x7f)]);
        assert_eq!(cpu.a, 0x7f);
        assert_eq!(cpu.x, 0);
        assert_eq!(cpu.pc, 0x8001);
        assert_eq!(cpu.psw & 0x83, 0, "N/Z cleared, C unchanged");
        assert_eq!(trace.len(), 4);
        assert_eq!(trace[2].address, 0x01ff);
        assert_eq!(bytes[0x01ff], 0x7f);
    }

    #[test]
    fn memory_destination_alu_modes_respect_source_destination_order_and_cycles() {
        for (family, carry, expected) in [
            (0x00_u8, 0, 0x1f), // OR
            (0x20, 0, 0x00),    // AND
            (0x40, 0, 0x1f),    // EOR
            (0x60, 0, 0x10),    // CMP leaves destination unchanged
            (0x80, 0, 0x1f),    // ADC
            (0xa0, 1, 0x01),    // SBC, no incoming borrow
        ] {
            for (mode, operands, data, destination, source, cycles, len) in [
                (
                    0x18_u8,
                    vec![0x0f, 0x42],
                    vec![(0x0142_u16, 0x10)],
                    0x0142_u16,
                    None,
                    5,
                    3,
                ),
                (
                    0x09,
                    vec![0x43, 0x42],
                    vec![(0x0143, 0x0f), (0x0142, 0x10)],
                    0x0142,
                    Some(0x0143_u16),
                    6,
                    3,
                ),
                (
                    0x19,
                    vec![],
                    vec![(0x0134, 0x0f), (0x0112, 0x10)],
                    0x0112,
                    Some(0x0134),
                    5,
                    1,
                ),
            ] {
                let opcode = family | mode;
                let mut cpu = Spc700::default();
                cpu.load_state(0x8000, 0x5a, 0x12, 0x34, 0xef, 0x20 | carry);
                let mut bytes = vec![0_u8; 65_536];
                bytes[0x8000] = opcode;
                for (i, &byte) in operands.iter().enumerate() {
                    bytes[0x8001 + i] = byte;
                }
                for (address, value) in data {
                    bytes[usize::from(address)] = value;
                }
                let bus = RefCell::new(bytes);
                let trace = cpu
                    .step_with_memory(
                        |address| bus.borrow()[usize::from(address)],
                        |address, value| bus.borrow_mut()[usize::from(address)] = value,
                    )
                    .unwrap();
                let bytes = bus.into_inner();

                assert_eq!(cpu.pc, 0x8000 + len, "opcode {opcode:02X}");
                assert_eq!(cpu.a, 0x5a, "memory ALU must not overwrite A");
                assert_eq!(trace.len(), cycles, "opcode {opcode:02X}");
                assert!(
                    trace
                        .iter()
                        .any(|event| event.address == u32::from(destination)),
                    "opcode {opcode:02X} must read destination"
                );
                if let Some(address) = source {
                    assert!(
                        trace
                            .iter()
                            .any(|event| event.address == u32::from(address))
                    );
                    assert_eq!(bytes[usize::from(address)], 0x0f, "source unchanged");
                }
                if family == 0x60 {
                    assert_eq!(bytes[usize::from(destination)], 0x10);
                    assert_eq!(trace.last().unwrap().access, AccessKind::Wait);
                    assert_ne!(cpu.psw & 0x01, 0, "CMP carry indicates lhs >= rhs");
                } else {
                    assert_eq!(bytes[usize::from(destination)], expected);
                    assert_eq!(trace.last().unwrap().access, AccessKind::Write);
                    assert_eq!(trace.last().unwrap().address, u32::from(destination));
                }
            }
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
