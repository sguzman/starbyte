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
        self.registers = registers::Registers {
            a: 0,
            x: 0,
            y: 0,
            s: 0x01FF,
            d: 0,
            pc: 0,
            pbr: 0,
            dbr: 0,
            p: 0x34,
            emulation: true,
        };
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
            0x04 => self.execute_tsb_direct_page(bus, &mut trace),
            0x05 => self.execute_ora_direct_page(bus, &mut trace),
            0x08 => self.execute_php(bus, &mut trace),
            0x10 => self.execute_bpl(bus, &mut trace),
            0xEA => self.execute_nop(bus, &mut trace),
            0x00 => self.execute_brk(bus, &mut trace),
            0x1B => self.execute_tcs(bus, &mut trace),
            0x18 => self.execute_clc(bus, &mut trace),
            0x1A => self.execute_inc_a(bus, &mut trace),
            0x20 => self.execute_jsr_absolute(bus, &mut trace),
            0x22 => self.execute_jsl_long(bus, &mut trace),
            0x28 => self.execute_plp(bus, &mut trace),
            0x29 => self.execute_and_immediate(bus, &mut trace),
            0x2A => self.execute_rol_a(bus, &mut trace),
            0x2C => self.execute_bit_absolute(bus, &mut trace),
            0x30 => self.execute_bmi(bus, &mut trace),
            0x38 => self.execute_sec(bus, &mut trace),
            0x3B => self.execute_tsc(bus, &mut trace),
            0x48 => self.execute_pha(bus, &mut trace),
            0x49 => self.execute_eor_immediate(bus, &mut trace),
            0x4B => self.execute_phk(bus, &mut trace),
            0x58 => self.execute_cli(bus, &mut trace),
            0x60 => self.execute_rts(bus, &mut trace),
            0x65 => self.execute_adc_direct_page(bus, &mut trace),
            0x68 => self.execute_pla(bus, &mut trace),
            0x69 => self.execute_adc_immediate(bus, &mut trace),
            0x70 => self.execute_bvs(bus, &mut trace),
            0x74 => self.execute_stz_direct_page_x(bus, &mut trace),
            0x6B => self.execute_rtl(bus, &mut trace),
            0x5B => self.execute_tcd(bus, &mut trace),
            0x78 => self.execute_sei(bus, &mut trace),
            0x80 => self.execute_bra(bus, &mut trace),
            0x7B => self.execute_tdc(bus, &mut trace),
            0x84 => self.execute_sty_direct_page(bus, &mut trace),
            0x85 => self.execute_sta_direct_page(bus, &mut trace),
            0x86 => self.execute_stx_direct_page(bus, &mut trace),
            0x88 => self.execute_dey(bus, &mut trace),
            0x8B => self.execute_phb(bus, &mut trace),
            0xA8 => self.execute_tay(bus, &mut trace),
            0x8A => self.execute_txa(bus, &mut trace),
            0x8D => self.execute_sta_absolute(bus, &mut trace),
            0x8E => self.execute_stx_absolute(bus, &mut trace),
            0x8F => self.execute_sta_long(bus, &mut trace),
            0x9B => self.execute_txy(bus, &mut trace),
            0x99 => self.execute_sta_absolute_y(bus, &mut trace),
            0x9C => self.execute_stz_absolute(bus, &mut trace),
            0x9E => self.execute_stz_absolute_x(bus, &mut trace),
            0x9F => self.execute_sta_long_x(bus, &mut trace),
            0xAA => self.execute_tax(bus, &mut trace),
            0xA0 => self.execute_ldy_immediate(bus, &mut trace),
            0xA2 => self.execute_ldx_immediate(bus, &mut trace),
            0xA5 => self.execute_lda_direct_page(bus, &mut trace),
            0xA9 => self.execute_lda_immediate(bus, &mut trace),
            0xAB => self.execute_plb(bus, &mut trace),
            0x98 => self.execute_tya(bus, &mut trace),
            0xAD => self.execute_lda_absolute(bus, &mut trace),
            0x9A => self.execute_txs(bus, &mut trace),
            0xB8 => self.execute_clv(bus, &mut trace),
            0xB5 => self.execute_lda_direct_page_x(bus, &mut trace),
            0xB7 => self.execute_lda_direct_page_indirect_long_y(bus, &mut trace),
            0xB9 => self.execute_lda_absolute_y(bus, &mut trace),
            0xBD => self.execute_lda_absolute_x(bus, &mut trace),
            0xBB => self.execute_tyx(bus, &mut trace),
            0xC8 => self.execute_iny(bus, &mut trace),
            0xCA => self.execute_dex(bus, &mut trace),
            0xCD => self.execute_cmp_absolute(bus, &mut trace),
            0xBA => self.execute_tsx(bus, &mut trace),
            0xC2 => self.execute_rep(bus, &mut trace),
            0xD0 => self.execute_bne(bus, &mut trace),
            0xD8 => self.execute_cld(bus, &mut trace),
            0xE0 => self.execute_cpx_immediate(bus, &mut trace),
            0xE8 => self.execute_inx(bus, &mut trace),
            0xE9 => self.execute_sbc_immediate(bus, &mut trace),
            0xE2 => self.execute_sep(bus, &mut trace),
            0xF0 => self.execute_beq(bus, &mut trace),
            0xFB => self.execute_xce(bus, &mut trace),
            0xF8 => self.execute_sed(bus, &mut trace),
            _ => Err(Error::UnsupportedOpcode {
                cpu: "65816",
                opcode,
                address: opcode_address,
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
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_clc<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.p &= !0x01;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_sec<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.p |= 0x01;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_cli<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.p &= !0x04;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_sei<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.p |= 0x04;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_bpl<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x80 == 0)
    }

    fn execute_bmi<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x80 != 0)
    }

    fn execute_bne<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x02 == 0)
    }

    fn execute_beq<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x02 != 0)
    }

    fn execute_bra<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, true)
    }

    fn execute_bvs<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x40 != 0)
    }

    fn execute_tay<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            self.registers.y = self.registers.a & 0x00FF;
            self.update_nz_8(self.registers.y as u8);
        } else {
            self.registers.y = self.registers.a;
            self.update_nz_16(self.registers.y);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_txa<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            self.registers.a = (self.registers.a & 0xFF00) | (self.registers.x & 0x00FF);
            self.update_nz_8(self.registers.a as u8);
        } else if self.index_registers_are_8_bit() {
            self.registers.a = self.registers.x & 0x00FF;
            self.update_nz_16(self.registers.a);
        } else {
            self.registers.a = self.registers.x;
            self.update_nz_16(self.registers.a);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tax<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            self.registers.x = self.registers.a & 0x00FF;
            self.update_nz_8(self.registers.x as u8);
        } else {
            self.registers.x = self.registers.a;
            self.update_nz_16(self.registers.x);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tya<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            self.registers.a = (self.registers.a & 0xFF00) | (self.registers.y & 0x00FF);
            self.update_nz_8(self.registers.a as u8);
        } else if self.index_registers_are_8_bit() {
            self.registers.a = self.registers.y & 0x00FF;
            self.update_nz_16(self.registers.a);
        } else {
            self.registers.a = self.registers.y;
            self.update_nz_16(self.registers.a);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_txy<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            self.registers.y = self.registers.x & 0x00FF;
            self.update_nz_8(self.registers.y as u8);
        } else {
            self.registers.y = self.registers.x;
            self.update_nz_16(self.registers.y);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tyx<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            self.registers.x = self.registers.y & 0x00FF;
            self.update_nz_8(self.registers.x as u8);
        } else {
            self.registers.x = self.registers.y;
            self.update_nz_16(self.registers.x);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_txs<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.s = if self.index_registers_are_8_bit() {
            self.registers.x & 0x00FF
        } else {
            self.registers.x
        };
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tcs<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.s = self.registers.a;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tsc<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.a = self.registers.s;
        self.update_nz_16(self.registers.a);
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tcd<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.d = self.registers.a;
        self.update_nz_16(self.registers.d);
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tdc<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.a = self.registers.d;
        self.update_nz_16(self.registers.a);
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tsx<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            self.registers.x = self.registers.s & 0x00FF;
            self.update_nz_8(self.registers.x as u8);
        } else {
            self.registers.x = self.registers.s;
            self.update_nz_16(self.registers.x);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_clv<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.p &= !0x40;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_rep<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let operand_address = self.fetch_address(1);
        let operand = self.push_read_trace(bus, trace, operand_address);
        self.push_read_trace(bus, trace, operand_address);
        self.registers.p &= !operand;
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_cld<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.p &= !0x08;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_sed<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.p |= 0x08;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_sep<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let operand_address = self.fetch_address(1);
        let operand = self.push_read_trace(bus, trace, operand_address);
        self.push_read_trace(bus, trace, operand_address);
        self.registers.p |= operand;
        if operand & 0x10 != 0 {
            self.registers.x &= 0x00FF;
            self.registers.y &= 0x00FF;
        }
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_dey<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            let value = (self.registers.y as u8).wrapping_sub(1);
            self.registers.y = value as u16;
            self.update_nz_8(value);
        } else {
            self.registers.y = self.registers.y.wrapping_sub(1);
            self.update_nz_16(self.registers.y);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_iny<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            let value = (self.registers.y as u8).wrapping_add(1);
            self.registers.y = value as u16;
            self.update_nz_8(value);
        } else {
            self.registers.y = self.registers.y.wrapping_add(1);
            self.update_nz_16(self.registers.y);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_dex<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            let value = (self.registers.x as u8).wrapping_sub(1);
            self.registers.x = value as u16;
            self.update_nz_8(value);
        } else {
            self.registers.x = self.registers.x.wrapping_sub(1);
            self.update_nz_16(self.registers.x);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_inx<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            let value = (self.registers.x as u8).wrapping_add(1);
            self.registers.x = value as u16;
            self.update_nz_8(value);
        } else {
            self.registers.x = self.registers.x.wrapping_add(1);
            self.update_nz_16(self.registers.x);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_inc_a<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            let value = (self.registers.a as u8).wrapping_add(1);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz_8(value);
        } else {
            self.registers.a = self.registers.a.wrapping_add(1);
            self.update_nz_16(self.registers.a);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_php<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.push_stack(bus, trace, self.registers.p)?;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_phb<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.push_stack(bus, trace, self.registers.dbr)?;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_phk<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.push_stack(bus, trace, self.registers.pbr)?;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_plb<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        let value = self.pull_stack(bus, trace);
        self.registers.dbr = value;
        self.update_nz_8(value);
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_jsr_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let target = self.fetch_operand_u16(bus, trace);
        let return_pc = self.registers.pc.wrapping_add(2);
        self.push_stack(bus, trace, (return_pc >> 8) as u8)?;
        self.push_stack(bus, trace, (return_pc & 0x00FF) as u8)?;
        self.registers.pc = target;
        Ok(())
    }

    fn execute_jsl_long<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let target = self.fetch_operand_u24(bus, trace);
        let return_pc = self.registers.pc.wrapping_add(3);
        self.push_stack(bus, trace, self.registers.pbr)?;
        self.push_stack(bus, trace, (return_pc >> 8) as u8)?;
        self.push_stack(bus, trace, (return_pc & 0x00FF) as u8)?;
        self.registers.pc = target as u16;
        self.registers.pbr = (target >> 16) as u8;
        Ok(())
    }

    fn execute_rts<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        let low = self.pull_stack(bus, trace);
        let high = self.pull_stack(bus, trace);
        self.registers.pc = u16::from_le_bytes([low, high]).wrapping_add(1);
        Ok(())
    }

    fn execute_rtl<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        let low = self.pull_stack(bus, trace);
        let high = self.pull_stack(bus, trace);
        let bank = self.pull_stack(bus, trace);
        self.registers.pc = u16::from_le_bytes([low, high]).wrapping_add(1);
        self.registers.pbr = bank;
        Ok(())
    }

    fn execute_plp<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        let mut value = self.pull_stack(bus, trace);
        if self.registers.emulation {
            value |= 0x30;
        }
        self.registers.p = value;
        if self.index_registers_are_8_bit() {
            self.registers.x &= 0x00FF;
            self.registers.y &= 0x00FF;
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_pla<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            let value = self.pull_stack(bus, trace);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz_8(value);
        } else {
            let low = self.pull_stack(bus, trace);
            let high = self.pull_stack(bus, trace);
            self.registers.a = u16::from_le_bytes([low, high]);
            self.update_nz_16(self.registers.a);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_pha<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            self.push_stack(bus, trace, self.registers.a as u8)?;
        } else {
            let [low, high] = self.registers.a.to_le_bytes();
            self.push_stack(bus, trace, high)?;
            self.push_stack(bus, trace, low)?;
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_lda_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.accumulator_is_8_bit() {
            let value = self.push_read_trace(bus, trace, self.fetch_address(1));
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz_8(value);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let value = self.fetch_operand_u16(bus, trace);
            self.registers.a = value;
            self.update_nz_16(value);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_ldx_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.index_registers_are_8_bit() {
            let value = self.push_read_trace(bus, trace, self.fetch_address(1));
            self.registers.x = u16::from(value);
            self.update_nz_8(value);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let value = self.fetch_operand_u16(bus, trace);
            self.registers.x = value;
            self.update_nz_16(value);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_ldy_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.index_registers_are_8_bit() {
            let value = self.push_read_trace(bus, trace, self.fetch_address(1));
            self.registers.y = u16::from(value);
            self.update_nz_8(value);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let value = self.fetch_operand_u16(bus, trace);
            self.registers.y = value;
            self.update_nz_16(value);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_lda_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_lda_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_lda_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_lda_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_lda_absolute_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_lda_direct_page_indirect_long_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let base = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, base);
        let high = self.read_u8_trace(bus, trace, base.wrapping_add(1));
        let bank = self.read_u8_trace(bus, trace, base.wrapping_add(2));
        let address = (u32::from(low) | (u32::from(high) << 8) | (u32::from(bank) << 16))
            .wrapping_add(u32::from(self.registers.y));
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_sta_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_stx_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.store_index_x_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_sty_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.store_index_y_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_sta_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_stx_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.store_index_x_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_sta_absolute_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_sta_long<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let address = self.fetch_operand_u24(bus, trace);
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(4);
        Ok(())
    }

    fn execute_sta_long_x<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let address = self
            .fetch_operand_u24(bus, trace)
            .wrapping_add(u32::from(self.registers.x));
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(4);
        Ok(())
    }

    fn execute_stz_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.store_zero_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_stz_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_indexed_x_address(operand);
        self.store_zero_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_stz_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.store_zero_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_ora_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            let result = (self.registers.a as u8) | value;
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
            self.update_nz_8(result);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            self.registers.a |= value;
            self.update_nz_16(self.registers.a);
        }
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_and_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.accumulator_is_8_bit() {
            let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
            let value = (self.registers.a as u8) & operand;
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz_8(value);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let operand = self.fetch_operand_u16(bus, trace);
            self.registers.a &= operand;
            self.update_nz_16(self.registers.a);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_eor_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.accumulator_is_8_bit() {
            let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
            let value = (self.registers.a as u8) ^ operand;
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz_8(value);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let operand = self.fetch_operand_u16(bus, trace);
            self.registers.a ^= operand;
            self.update_nz_16(self.registers.a);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_adc_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        if self.accumulator_is_8_bit() {
            let lhs = self.registers.a as u8;
            let rhs = self.read_u8_trace(bus, trace, address);
            let carry = u8::from(self.registers.p & 0x01 != 0);
            let (tmp, carry1) = lhs.overflowing_add(rhs);
            let (result, carry2) = tmp.overflowing_add(carry);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
            self.set_carry(carry1 || carry2);
            self.update_nz_8(result);
        } else {
            let lhs = self.registers.a;
            let rhs = self.read_u16_trace(bus, trace, address);
            let carry = u16::from(self.registers.p & 0x01 != 0);
            let (tmp, carry1) = lhs.overflowing_add(rhs);
            let (result, carry2) = tmp.overflowing_add(carry);
            self.registers.a = result;
            self.set_carry(carry1 || carry2);
            self.update_nz_16(result);
        }
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_adc_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.accumulator_is_8_bit() {
            let rhs = self.push_read_trace(bus, trace, self.fetch_address(1));
            let lhs = self.registers.a as u8;
            let carry = u8::from(self.registers.p & 0x01 != 0);
            let (tmp, carry1) = lhs.overflowing_add(rhs);
            let (result, carry2) = tmp.overflowing_add(carry);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
            self.set_carry(carry1 || carry2);
            self.update_nz_8(result);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let rhs = self.fetch_operand_u16(bus, trace);
            let lhs = self.registers.a;
            let carry = u16::from(self.registers.p & 0x01 != 0);
            let (tmp, carry1) = lhs.overflowing_add(rhs);
            let (result, carry2) = tmp.overflowing_add(carry);
            self.registers.a = result;
            self.set_carry(carry1 || carry2);
            self.update_nz_16(result);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_cpx_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.index_registers_are_8_bit() {
            let rhs = self.push_read_trace(bus, trace, self.fetch_address(1));
            let lhs = self.registers.x as u8;
            let result = lhs.wrapping_sub(rhs);
            self.set_carry(lhs >= rhs);
            self.update_nz_8(result);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let rhs = self.fetch_operand_u16(bus, trace);
            let lhs = self.registers.x;
            let result = lhs.wrapping_sub(rhs);
            self.set_carry(lhs >= rhs);
            self.update_nz_16(result);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_sbc_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.accumulator_is_8_bit() {
            let rhs = self.push_read_trace(bus, trace, self.fetch_address(1));
            let lhs = self.registers.a as u8;
            let borrow = u8::from(self.registers.p & 0x01 == 0);
            let (tmp, borrow1) = lhs.overflowing_sub(rhs);
            let (result, borrow2) = tmp.overflowing_sub(borrow);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
            self.set_carry(!(borrow1 || borrow2));
            self.update_nz_8(result);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let rhs = self.fetch_operand_u16(bus, trace);
            let lhs = self.registers.a;
            let borrow = u16::from(self.registers.p & 0x01 == 0);
            let (tmp, borrow1) = lhs.overflowing_sub(rhs);
            let (result, borrow2) = tmp.overflowing_sub(borrow);
            self.registers.a = result;
            self.set_carry(!(borrow1 || borrow2));
            self.update_nz_16(result);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_xce<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        let carry = (self.registers.p & 0x01) != 0;
        self.set_carry(self.registers.emulation);
        self.registers.emulation = carry;
        if self.registers.emulation {
            self.registers.p |= 0x30;
            self.registers.x &= 0x00FF;
            self.registers.y &= 0x00FF;
            self.registers.s = 0x0100 | (self.registers.s & 0x00FF);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tsb_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            self.set_zero((value & self.registers.a as u8) == 0);
            self.write_u8_trace(bus, trace, address, value | self.registers.a as u8);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            self.set_zero((value & self.registers.a) == 0);
            self.write_u16_trace(bus, trace, address, value | self.registers.a);
        }
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_bit_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            self.set_zero((value & self.registers.a as u8) == 0);
            self.set_negative(value & 0x80 != 0);
            self.set_overflow(value & 0x40 != 0);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            self.set_zero((value & self.registers.a) == 0);
            self.set_negative(value & 0x8000 != 0);
            self.set_overflow(value & 0x4000 != 0);
        }
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_rol_a<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            let carry_in = u8::from(self.registers.p & 0x01 != 0);
            let value = self.registers.a as u8;
            self.set_carry(value & 0x80 != 0);
            let result = (value << 1) | carry_in;
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
            self.update_nz_8(result);
        } else {
            let carry_in = u16::from(self.registers.p & 0x01 != 0);
            let value = self.registers.a;
            self.set_carry(value & 0x8000 != 0);
            let result = (value << 1) | carry_in;
            self.registers.a = result;
            self.update_nz_16(result);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_cmp_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        if self.accumulator_is_8_bit() {
            let lhs = self.registers.a as u8;
            let rhs = self.read_u8_trace(bus, trace, address);
            let result = lhs.wrapping_sub(rhs);
            self.set_carry(lhs >= rhs);
            self.update_nz_8(result);
        } else {
            let lhs = self.registers.a;
            let rhs = self.read_u16_trace(bus, trace, address);
            let result = lhs.wrapping_sub(rhs);
            self.set_carry(lhs >= rhs);
            self.update_nz_16(result);
        }
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_brk<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let signature_address = self.fetch_address(1);
        let signature = bus.read(signature_address);
        trace.push(BusEvent {
            address: signature_address,
            value: signature,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });

        let return_pc = self.registers.pc.wrapping_add(2);
        if !self.registers.emulation {
            self.push_stack(bus, trace, self.registers.pbr)?;
        }
        self.push_stack(bus, trace, (return_pc >> 8) as u8)?;
        self.push_stack(bus, trace, (return_pc & 0x00FF) as u8)?;
        self.push_stack(
            bus,
            trace,
            if self.registers.emulation {
                self.registers.p | 0x10
            } else {
                self.registers.p
            },
        )?;

        let vector_base = if self.registers.emulation {
            0x00FFFE
        } else {
            0x00FFE6
        };
        let vector_low = bus.read(vector_base);
        trace.push(BusEvent {
            address: vector_base,
            value: vector_low,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });
        let vector_high = bus.read(vector_base + 1);
        trace.push(BusEvent {
            address: vector_base + 1,
            value: vector_high,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });

        self.registers.pc = u16::from_le_bytes([vector_low, vector_high]);
        if !self.registers.emulation {
            self.registers.pbr = 0;
        }
        self.registers.p = (self.registers.p | 0x04) & !0x08;
        Ok(())
    }

    fn push_stack<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        value: u8,
    ) -> Result<()> {
        let address = u32::from(self.stack_address());
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

    fn pull_stack<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> u8 {
        self.registers.s = self.registers.s.wrapping_add(1);
        let address = u32::from(self.stack_address());
        let value = bus.read(address);
        trace.push(BusEvent {
            address,
            value,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });
        value
    }

    fn push_read_trace<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) -> u8 {
        let value = bus.read(address);
        trace.push(BusEvent {
            address,
            value,
            access: AccessKind::Read,
            cycle: trace.len() as u64,
        });
        value
    }

    fn fetch_address(&self, offset: u16) -> Address {
        (u32::from(self.registers.pbr) << 16) | u32::from(self.registers.pc.wrapping_add(offset))
    }

    fn fetch_operand_u16<B: Bus>(&self, bus: &mut B, trace: &mut Vec<BusEvent>) -> u16 {
        let low = self.push_read_trace(bus, trace, self.fetch_address(1));
        let high = self.push_read_trace(bus, trace, self.fetch_address(2));
        u16::from_le_bytes([low, high])
    }

    fn fetch_operand_u24<B: Bus>(&self, bus: &mut B, trace: &mut Vec<BusEvent>) -> u32 {
        let low = self.push_read_trace(bus, trace, self.fetch_address(1));
        let high = self.push_read_trace(bus, trace, self.fetch_address(2));
        let bank = self.push_read_trace(bus, trace, self.fetch_address(3));
        u32::from(low) | (u32::from(high) << 8) | (u32::from(bank) << 16)
    }

    fn direct_page_address(&self, operand: u8) -> Address {
        u32::from(self.registers.d.wrapping_add(u16::from(operand)))
    }

    fn direct_page_indexed_x_address(&self, operand: u8) -> Address {
        self.direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x))
    }

    fn absolute_address(&self, operand: u16) -> Address {
        (u32::from(self.registers.dbr) << 16) | u32::from(operand)
    }

    fn stack_address(&self) -> u16 {
        if self.registers.emulation {
            0x0100 | (self.registers.s & 0x00FF)
        } else {
            self.registers.s
        }
    }

    fn execute_branch_relative<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        condition: bool,
    ) -> Result<()> {
        let offset = self.push_read_trace(bus, trace, self.fetch_address(1)) as i8;
        let next = self.registers.pc.wrapping_add(2);
        self.registers.pc = if condition {
            next.wrapping_add_signed(i16::from(offset))
        } else {
            next
        };
        Ok(())
    }

    fn read_u8_trace<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) -> u8 {
        self.push_read_trace(bus, trace, address)
    }

    fn read_u16_trace<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) -> u16 {
        let low = self.read_u8_trace(bus, trace, address);
        let high = self.read_u8_trace(bus, trace, address.wrapping_add(1));
        u16::from_le_bytes([low, high])
    }

    fn write_u8_trace<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
        value: u8,
    ) {
        bus.write(address, value);
        trace.push(BusEvent {
            address,
            value,
            access: AccessKind::Write,
            cycle: trace.len() as u64,
        });
    }

    fn write_u16_trace<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
        value: u16,
    ) {
        let [low, high] = value.to_le_bytes();
        self.write_u8_trace(bus, trace, address, low);
        self.write_u8_trace(bus, trace, address.wrapping_add(1), high);
    }

    fn load_accumulator_from_address<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz_8(value);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            self.registers.a = value;
            self.update_nz_16(value);
        }
    }

    fn store_accumulator_to_address<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.accumulator_is_8_bit() {
            self.write_u8_trace(bus, trace, address, self.registers.a as u8);
        } else {
            self.write_u16_trace(bus, trace, address, self.registers.a);
        }
    }

    fn store_index_x_to_address<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.index_registers_are_8_bit() {
            self.write_u8_trace(bus, trace, address, self.registers.x as u8);
        } else {
            self.write_u16_trace(bus, trace, address, self.registers.x);
        }
    }

    fn store_index_y_to_address<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.index_registers_are_8_bit() {
            self.write_u8_trace(bus, trace, address, self.registers.y as u8);
        } else {
            self.write_u16_trace(bus, trace, address, self.registers.y);
        }
    }

    fn store_zero_to_address<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.accumulator_is_8_bit() {
            self.write_u8_trace(bus, trace, address, 0);
        } else {
            self.write_u16_trace(bus, trace, address, 0);
        }
    }

    fn accumulator_is_8_bit(&self) -> bool {
        self.registers.emulation || (self.registers.p & 0x20) != 0
    }

    fn index_registers_are_8_bit(&self) -> bool {
        self.registers.emulation || (self.registers.p & 0x10) != 0
    }

    fn update_nz_8(&mut self, value: u8) {
        self.registers.p &= !(0x80 | 0x02);
        if value & 0x80 != 0 {
            self.registers.p |= 0x80;
        }
        if value == 0 {
            self.registers.p |= 0x02;
        }
    }

    fn update_nz_16(&mut self, value: u16) {
        self.registers.p &= !(0x80 | 0x02);
        if value & 0x8000 != 0 {
            self.registers.p |= 0x80;
        }
        if value == 0 {
            self.registers.p |= 0x02;
        }
    }

    fn set_carry(&mut self, enabled: bool) {
        if enabled {
            self.registers.p |= 0x01;
        } else {
            self.registers.p &= !0x01;
        }
    }

    fn set_zero(&mut self, enabled: bool) {
        if enabled {
            self.registers.p |= 0x02;
        } else {
            self.registers.p &= !0x02;
        }
    }

    fn set_overflow(&mut self, enabled: bool) {
        if enabled {
            self.registers.p |= 0x40;
        } else {
            self.registers.p &= !0x40;
        }
    }

    fn set_negative(&mut self, enabled: bool) {
        if enabled {
            self.registers.p |= 0x80;
        } else {
            self.registers.p &= !0x80;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::bus::{AccessKind, Address, Bus};

    use super::Cpu65816;

    #[derive(Default)]
    struct TestBus {
        bytes: HashMap<Address, u8>,
    }

    impl TestBus {
        fn with_bytes(bytes: &[(Address, u8)]) -> Self {
            let mut map = HashMap::new();
            for (address, value) in bytes {
                map.insert(*address, *value);
            }
            Self { bytes: map }
        }
    }

    impl Bus for TestBus {
        fn read(&mut self, address: Address) -> u8 {
            self.bytes.get(&address).copied().unwrap_or(0)
        }

        fn write(&mut self, address: Address, value: u8) {
            self.bytes.insert(address, value);
        }
    }

    #[test]
    fn stz_direct_page_x_stores_zero_without_opcode_failure() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0010;
        cpu.registers.x = 0x0004;
        cpu.registers.p = 0x00;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x74),
            (0x008001, 0x20),
            (0x000034, 0xAA),
            (0x000035, 0xBB),
        ]);

        let trace = cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(bus.read(0x000034), 0x00);
        assert_eq!(bus.read(0x000035), 0x00);
        assert!(trace.iter().any(|event| {
            event.address == 0x000034 && event.access == AccessKind::Write && event.value == 0x00
        }));
    }

    #[test]
    fn stz_absolute_x_uses_indexed_target_address() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.x = 0x0003;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x9E),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x001237, 0xFE),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8003);
        assert_eq!(bus.read(0x001237), 0x00);
    }
}
