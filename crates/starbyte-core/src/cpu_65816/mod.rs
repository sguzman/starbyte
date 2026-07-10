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
            0x0A => self.execute_asl_a(bus, &mut trace),
            0x10 => self.execute_bpl(bus, &mut trace),
            0xEA => self.execute_nop(bus, &mut trace),
            0x00 => self.execute_brk(bus, &mut trace),
            0xE6 => self.execute_inc_direct_page(bus, &mut trace),
            0x1B => self.execute_tcs(bus, &mut trace),
            0x18 => self.execute_clc(bus, &mut trace),
            0x1A => self.execute_inc_a(bus, &mut trace),
            0x20 => self.execute_jsr_absolute(bus, &mut trace),
            0x22 => self.execute_jsl_long(bus, &mut trace),
            0x28 => self.execute_plp(bus, &mut trace),
            0x29 => self.execute_and_immediate(bus, &mut trace),
            0x2A => self.execute_rol_a(bus, &mut trace),
            0x2D => self.execute_and_absolute(bus, &mut trace),
            0x2C => self.execute_bit_absolute(bus, &mut trace),
            0x30 => self.execute_bmi(bus, &mut trace),
            0x38 => self.execute_sec(bus, &mut trace),
            0x3B => self.execute_tsc(bus, &mut trace),
            0x48 => self.execute_pha(bus, &mut trace),
            0x49 => self.execute_eor_immediate(bus, &mut trace),
            0x4C => self.execute_jmp_absolute(bus, &mut trace),
            0x4B => self.execute_phk(bus, &mut trace),
            0x4D => self.execute_eor_absolute(bus, &mut trace),
            0x5A => self.execute_phy(bus, &mut trace),
            0x58 => self.execute_cli(bus, &mut trace),
            0x60 => self.execute_rts(bus, &mut trace),
            0x64 => self.execute_stz_direct_page(bus, &mut trace),
            0x65 => self.execute_adc_direct_page(bus, &mut trace),
            0x68 => self.execute_pla(bus, &mut trace),
            0x69 => self.execute_adc_immediate(bus, &mut trace),
            0x70 => self.execute_bvs(bus, &mut trace),
            0x7A => self.execute_ply(bus, &mut trace),
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
            0x90 => self.execute_bcc(bus, &mut trace),
            0x8B => self.execute_phb(bus, &mut trace),
            0xA8 => self.execute_tay(bus, &mut trace),
            0x8A => self.execute_txa(bus, &mut trace),
            0x8D => self.execute_sta_absolute(bus, &mut trace),
            0x8C => self.execute_sty_absolute(bus, &mut trace),
            0x8E => self.execute_stx_absolute(bus, &mut trace),
            0x8F => self.execute_sta_long(bus, &mut trace),
            0x97 => self.execute_sta_direct_page_indirect_long_y(bus, &mut trace),
            0x9B => self.execute_txy(bus, &mut trace),
            0x99 => self.execute_sta_absolute_y(bus, &mut trace),
            0x9D => self.execute_sta_absolute_x(bus, &mut trace),
            0x9C => self.execute_stz_absolute(bus, &mut trace),
            0x9E => self.execute_stz_absolute_x(bus, &mut trace),
            0x9F => self.execute_sta_long_x(bus, &mut trace),
            0xAA => self.execute_tax(bus, &mut trace),
            0xA0 => self.execute_ldy_immediate(bus, &mut trace),
            0xA4 => self.execute_ldy_direct_page(bus, &mut trace),
            0xA2 => self.execute_ldx_immediate(bus, &mut trace),
            0xA6 => self.execute_ldx_direct_page(bus, &mut trace),
            0xA5 => self.execute_lda_direct_page(bus, &mut trace),
            0xA7 => self.execute_lda_direct_page_indirect_long(bus, &mut trace),
            0xA9 => self.execute_lda_immediate(bus, &mut trace),
            0xAE => self.execute_ldx_absolute(bus, &mut trace),
            0xAB => self.execute_plb(bus, &mut trace),
            0x98 => self.execute_tya(bus, &mut trace),
            0xAD => self.execute_lda_absolute(bus, &mut trace),
            0x9A => self.execute_txs(bus, &mut trace),
            0xB8 => self.execute_clv(bus, &mut trace),
            0xB0 => self.execute_bcs(bus, &mut trace),
            0xB4 => self.execute_ldy_direct_page_x(bus, &mut trace),
            0xB5 => self.execute_lda_direct_page_x(bus, &mut trace),
            0xB6 => self.execute_ldx_direct_page_y(bus, &mut trace),
            0xB7 => self.execute_lda_direct_page_indirect_long_y(bus, &mut trace),
            0xB9 => self.execute_lda_absolute_y(bus, &mut trace),
            0xBC => self.execute_ldy_absolute_x(bus, &mut trace),
            0xBD => self.execute_lda_absolute_x(bus, &mut trace),
            0xBE => self.execute_ldx_absolute_y(bus, &mut trace),
            0xBB => self.execute_tyx(bus, &mut trace),
            0xC8 => self.execute_iny(bus, &mut trace),
            0xCA => self.execute_dex(bus, &mut trace),
            0xC5 => self.execute_cmp_direct_page(bus, &mut trace),
            0xC6 => self.execute_dec_direct_page(bus, &mut trace),
            0xC9 => self.execute_cmp_immediate(bus, &mut trace),
            0xCD => self.execute_cmp_absolute(bus, &mut trace),
            0xCE => self.execute_dec_absolute(bus, &mut trace),
            0xDC => self.execute_jmp_absolute_indirect_long(bus, &mut trace),
            0xDA => self.execute_phx(bus, &mut trace),
            0xBA => self.execute_tsx(bus, &mut trace),
            0xC2 => self.execute_rep(bus, &mut trace),
            0xC0 => self.execute_cpy_immediate(bus, &mut trace),
            0xD0 => self.execute_bne(bus, &mut trace),
            0xD5 => self.execute_cmp_direct_page_x(bus, &mut trace),
            0xD6 => self.execute_dec_direct_page_x(bus, &mut trace),
            0xD9 => self.execute_cmp_absolute_y(bus, &mut trace),
            0xD8 => self.execute_cld(bus, &mut trace),
            0xDD => self.execute_cmp_absolute_x(bus, &mut trace),
            0xDE => self.execute_dec_absolute_x(bus, &mut trace),
            0xE0 => self.execute_cpx_immediate(bus, &mut trace),
            0xE8 => self.execute_inx(bus, &mut trace),
            0xE9 => self.execute_sbc_immediate(bus, &mut trace),
            0xEB => self.execute_xba(bus, &mut trace),
            0xE2 => self.execute_sep(bus, &mut trace),
            0xEE => self.execute_inc_absolute(bus, &mut trace),
            0xF0 => self.execute_beq(bus, &mut trace),
            0xF6 => self.execute_inc_direct_page_x(bus, &mut trace),
            0xFA => self.execute_plx(bus, &mut trace),
            0xFB => self.execute_xce(bus, &mut trace),
            0xFE => self.execute_inc_absolute_x(bus, &mut trace),
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

    fn execute_bcc<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x01 == 0)
    }

    fn execute_bcs<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x01 != 0)
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

    fn execute_inc_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.increment_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_inc_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.increment_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_inc_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.increment_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_inc_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.increment_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_dec_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.decrement_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_dec_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.decrement_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_dec_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.decrement_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_dec_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.decrement_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
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

    fn execute_jmp_absolute<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.registers.pc = self.fetch_operand_u16(bus, trace);
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

    fn execute_ply<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            let value = self.pull_stack(bus, trace);
            self.registers.y = u16::from(value);
            self.update_nz_8(value);
        } else {
            let low = self.pull_stack(bus, trace);
            let high = self.pull_stack(bus, trace);
            self.registers.y = u16::from_le_bytes([low, high]);
            self.update_nz_16(self.registers.y);
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

    fn execute_phy<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            self.push_stack(bus, trace, self.registers.y as u8)?;
        } else {
            let [low, high] = self.registers.y.to_le_bytes();
            self.push_stack(bus, trace, high)?;
            self.push_stack(bus, trace, low)?;
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_phx<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            self.push_stack(bus, trace, self.registers.x as u8)?;
        } else {
            let [low, high] = self.registers.x.to_le_bytes();
            self.push_stack(bus, trace, high)?;
            self.push_stack(bus, trace, low)?;
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_plx<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.index_registers_are_8_bit() {
            let value = self.pull_stack(bus, trace);
            self.registers.x = u16::from(value);
            self.update_nz_8(value);
        } else {
            let low = self.pull_stack(bus, trace);
            let high = self.pull_stack(bus, trace);
            self.registers.x = u16::from_le_bytes([low, high]);
            self.update_nz_16(self.registers.x);
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

    fn execute_ldy_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.load_index_y_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ldx_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.load_index_x_from_address(bus, trace, address);
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

    fn execute_ldy_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.load_index_y_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ldx_direct_page_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.y & 0x00FF));
        self.load_index_x_from_address(bus, trace, address);
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

    fn execute_ldx_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.load_index_x_from_address(bus, trace, address);
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

    fn execute_ldy_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.load_index_y_from_address(bus, trace, address);
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

    fn execute_ldx_absolute_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.load_index_x_from_address(bus, trace, address);
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

    fn execute_lda_direct_page_indirect_long<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let base = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, base);
        let high = self.read_u8_trace(bus, trace, base.wrapping_add(1));
        let bank = self.read_u8_trace(bus, trace, base.wrapping_add(2));
        let address = u32::from(low) | (u32::from(high) << 8) | (u32::from(bank) << 16);
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

    fn execute_sty_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.store_index_y_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
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

    fn execute_sta_direct_page_indirect_long_y<B: Bus>(
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
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_sta_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
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

    fn execute_stz_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.store_zero_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
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

    fn execute_eor_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        if self.accumulator_is_8_bit() {
            let value = (self.registers.a as u8) ^ self.read_u8_trace(bus, trace, address);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz_8(value);
        } else {
            self.registers.a ^= self.read_u16_trace(bus, trace, address);
            self.update_nz_16(self.registers.a);
        }
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_and_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        if self.accumulator_is_8_bit() {
            let value = (self.registers.a as u8) & self.read_u8_trace(bus, trace, address);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz_8(value);
        } else {
            self.registers.a &= self.read_u16_trace(bus, trace, address);
            self.update_nz_16(self.registers.a);
        }
        self.registers.pc = self.registers.pc.wrapping_add(3);
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
            self.set_overflow((!(lhs ^ rhs) & (lhs ^ result) & 0x80) != 0);
            self.update_nz_8(result);
        } else {
            let lhs = self.registers.a;
            let rhs = self.read_u16_trace(bus, trace, address);
            let carry = u16::from(self.registers.p & 0x01 != 0);
            let (tmp, carry1) = lhs.overflowing_add(rhs);
            let (result, carry2) = tmp.overflowing_add(carry);
            self.registers.a = result;
            self.set_carry(carry1 || carry2);
            self.set_overflow((!(lhs ^ rhs) & (lhs ^ result) & 0x8000) != 0);
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
            self.set_overflow((!(lhs ^ rhs) & (lhs ^ result) & 0x80) != 0);
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
            self.set_overflow((!(lhs ^ rhs) & (lhs ^ result) & 0x8000) != 0);
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
            self.compare_index_x_8(rhs);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let rhs = self.fetch_operand_u16(bus, trace);
            self.compare_index_x_16(rhs);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_cpy_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.index_registers_are_8_bit() {
            let rhs = self.push_read_trace(bus, trace, self.fetch_address(1));
            self.compare_index_y_8(rhs);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let rhs = self.fetch_operand_u16(bus, trace);
            self.compare_index_y_16(rhs);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_cmp_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.accumulator_is_8_bit() {
            let rhs = self.push_read_trace(bus, trace, self.fetch_address(1));
            self.compare_accumulator_8(rhs);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let rhs = self.fetch_operand_u16(bus, trace);
            self.compare_accumulator_16(rhs);
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
            self.set_overflow(((lhs ^ rhs) & (lhs ^ result) & 0x80) != 0);
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
            self.set_overflow(((lhs ^ rhs) & (lhs ^ result) & 0x8000) != 0);
            self.update_nz_16(result);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_xba<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.a = self.registers.a.rotate_left(8);
        self.update_nz_8((self.registers.a & 0x00FF) as u8);
        self.registers.pc = self.registers.pc.wrapping_add(1);
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

    fn execute_asl_a<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            let value = self.registers.a as u8;
            self.set_carry(value & 0x80 != 0);
            let result = value << 1;
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
            self.update_nz_8(result);
        } else {
            let value = self.registers.a;
            self.set_carry(value & 0x8000 != 0);
            let result = value << 1;
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
        self.compare_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_cmp_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.compare_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_cmp_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.compare_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_cmp_absolute_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.compare_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_cmp_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.compare_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn compare_accumulator_with_address<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.accumulator_is_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.compare_accumulator_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.compare_accumulator_16(rhs);
        }
    }

    fn execute_jmp_absolute_indirect_long<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let pointer = self.fetch_operand_u16(bus, trace);
        let base = u32::from(pointer);
        let low = self.read_u8_trace(bus, trace, base);
        let high = self.read_u8_trace(bus, trace, base.wrapping_add(1));
        let bank = self.read_u8_trace(bus, trace, base.wrapping_add(2));
        self.registers.pc = u16::from_le_bytes([low, high]);
        self.registers.pbr = bank;
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

    fn load_index_x_from_address<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.index_registers_are_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            self.registers.x = u16::from(value);
            self.update_nz_8(value);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            self.registers.x = value;
            self.update_nz_16(value);
        }
    }

    fn load_index_y_from_address<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.index_registers_are_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            self.registers.y = u16::from(value);
            self.update_nz_8(value);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            self.registers.y = value;
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

    fn compare_accumulator_8(&mut self, rhs: u8) {
        let lhs = self.registers.a as u8;
        let result = lhs.wrapping_sub(rhs);
        self.set_carry(lhs >= rhs);
        self.update_nz_8(result);
    }

    fn compare_accumulator_16(&mut self, rhs: u16) {
        let lhs = self.registers.a;
        let result = lhs.wrapping_sub(rhs);
        self.set_carry(lhs >= rhs);
        self.update_nz_16(result);
    }

    fn compare_index_x_8(&mut self, rhs: u8) {
        let lhs = self.registers.x as u8;
        let result = lhs.wrapping_sub(rhs);
        self.set_carry(lhs >= rhs);
        self.update_nz_8(result);
    }

    fn compare_index_x_16(&mut self, rhs: u16) {
        let lhs = self.registers.x;
        let result = lhs.wrapping_sub(rhs);
        self.set_carry(lhs >= rhs);
        self.update_nz_16(result);
    }

    fn compare_index_y_8(&mut self, rhs: u8) {
        let lhs = self.registers.y as u8;
        let result = lhs.wrapping_sub(rhs);
        self.set_carry(lhs >= rhs);
        self.update_nz_8(result);
    }

    fn compare_index_y_16(&mut self, rhs: u16) {
        let lhs = self.registers.y;
        let result = lhs.wrapping_sub(rhs);
        self.set_carry(lhs >= rhs);
        self.update_nz_16(result);
    }

    fn increment_memory<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address).wrapping_add(1);
            self.write_u8_trace(bus, trace, address, value);
            self.update_nz_8(value);
        } else {
            let value = self.read_u16_trace(bus, trace, address).wrapping_add(1);
            self.write_u16_trace(bus, trace, address, value);
            self.update_nz_16(value);
        }
    }

    fn decrement_memory<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address).wrapping_sub(1);
            self.write_u8_trace(bus, trace, address, value);
            self.update_nz_8(value);
        } else {
            let value = self.read_u16_trace(bus, trace, address).wrapping_sub(1);
            self.write_u16_trace(bus, trace, address, value);
            self.update_nz_16(value);
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

    #[test]
    fn stz_direct_page_stores_zero() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0010;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x64),
            (0x008001, 0x22),
            (0x000032, 0xFE),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(bus.read(0x000032), 0x00);
    }

    #[test]
    fn sta_absolute_x_uses_indexed_target_address() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x00A5;
        cpu.registers.x = 0x0004;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x9D),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x001238, 0x00),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8003);
        assert_eq!(bus.read(0x001238), 0xA5);
    }

    #[test]
    fn sty_absolute_stores_index_register_y() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.y = 0x1234;
        cpu.registers.p = 0x00;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x8C),
            (0x008001, 0x78),
            (0x008002, 0x56),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8003);
        assert_eq!(bus.read(0x005678), 0x34);
        assert_eq!(bus.read(0x005679), 0x12);
    }

    #[test]
    fn inc_direct_page_increments_memory_and_updates_flags() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xE6),
            (0x008001, 0x10),
            (0x000030, 0x7F),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(bus.read(0x000030), 0x80);
        assert_ne!(cpu.registers.p & 0x80, 0);
        assert_eq!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn ply_restores_16_bit_index_register_from_stack() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01FD;
        cpu.registers.p = 0x00;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x7A),
            (0x0001FE, 0x34),
            (0x0001FF, 0x12),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8001);
        assert_eq!(cpu.registers.y, 0x1234);
        assert_eq!(cpu.registers.s, 0x01FF);
    }

    #[test]
    fn phy_pushes_16_bit_index_register_y_to_stack() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01FF;
        cpu.registers.y = 0x1234;
        cpu.registers.p = 0x00;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x5A)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8001);
        assert_eq!(cpu.registers.s, 0x01FD);
        assert_eq!(bus.read(0x0001FF), 0x12);
        assert_eq!(bus.read(0x0001FE), 0x34);
    }

    #[test]
    fn plx_restores_16_bit_index_register_x_from_stack() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01FD;
        cpu.registers.p = 0x00;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xFA),
            (0x0001FE, 0x78),
            (0x0001FF, 0x56),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8001);
        assert_eq!(cpu.registers.x, 0x5678);
        assert_eq!(cpu.registers.s, 0x01FF);
    }

    #[test]
    fn asl_a_shifts_left_and_updates_carry() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x0081;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x0A)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8001);
        assert_eq!(cpu.registers.a & 0x00FF, 0x02);
        assert_ne!(cpu.registers.p & 0x01, 0);
        assert_eq!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn ldy_direct_page_loads_16_bit_value() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0040;
        cpu.registers.p = 0x00;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xA4),
            (0x008001, 0x20),
            (0x000060, 0x78),
            (0x000061, 0x56),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(cpu.registers.y, 0x5678);
        assert_eq!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn ldx_direct_page_loads_16_bit_value() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0040;
        cpu.registers.p = 0x00;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xA6),
            (0x008001, 0x20),
            (0x000060, 0x34),
            (0x000061, 0x12),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(cpu.registers.x, 0x1234);
    }

    #[test]
    fn ldx_direct_page_y_uses_indexed_target_address() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0010;
        cpu.registers.y = 0x0003;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xB6),
            (0x008001, 0x20),
            (0x000033, 0xA5),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(cpu.registers.x, 0x00A5);
    }

    #[test]
    fn ldx_absolute_y_uses_indexed_target_address() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.y = 0x0002;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xBE),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x001236, 0x5A),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8003);
        assert_eq!(cpu.registers.x, 0x005A);
    }

    #[test]
    fn sta_direct_page_indirect_long_y_uses_long_pointer_plus_y() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.y = 0x0004;
        cpu.registers.a = 0x00A5;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x97),
            (0x008001, 0x10),
            (0x000030, 0x78),
            (0x000031, 0x56),
            (0x000032, 0x7E),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(bus.read(0x7E567C), 0xA5);
    }

    #[test]
    fn lda_direct_page_indirect_long_loads_16_bit_value() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.p = 0x00;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xA7),
            (0x008001, 0x10),
            (0x000030, 0x78),
            (0x000031, 0x56),
            (0x000032, 0x7E),
            (0x7E5678, 0x34),
            (0x7E5679, 0x12),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(cpu.registers.a, 0x1234);
    }

    #[test]
    fn jmp_absolute_indirect_long_loads_pc_and_bank_from_pointer() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.pbr = 0x12;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x128000, 0xDC),
            (0x128001, 0x20),
            (0x128002, 0x00),
            (0x000020, 0x56),
            (0x000021, 0x34),
            (0x000022, 0x7E),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x3456);
        assert_eq!(cpu.registers.pbr, 0x7E);
    }

    #[test]
    fn jmp_absolute_loads_target_pc() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x4C),
            (0x008001, 0x56),
            (0x008002, 0x34),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x3456);
    }

    #[test]
    fn eor_absolute_updates_accumulator() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x00F0;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x4D),
            (0x008001, 0x00),
            (0x008002, 0x20),
            (0x002000, 0xAA),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x5A);
    }

    #[test]
    fn and_absolute_updates_accumulator() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x00F0;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x2D),
            (0x008001, 0x00),
            (0x008002, 0x20),
            (0x002000, 0xAA),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0xA0);
    }

    #[test]
    fn cmp_immediate_updates_carry_and_zero_flags() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x0034;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0xC9), (0x008001, 0x34)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_ne!(cpu.registers.p & 0x01, 0);
        assert_ne!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn dec_direct_page_decrements_memory_and_updates_flags() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xC6),
            (0x008001, 0x10),
            (0x000030, 0x01),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(bus.read(0x000030), 0x00);
        assert_ne!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn cpy_immediate_updates_carry_and_zero_flags() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.y = 0x0034;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0xC0), (0x008001, 0x34)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8003);
        assert_ne!(cpu.registers.p & 0x01, 0);
        assert_ne!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn adc_immediate_sets_overflow_for_signed_wrap() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x007F;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x69), (0x008001, 0x01)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x80);
        assert_ne!(cpu.registers.p & 0x40, 0);
    }

    #[test]
    fn xba_swaps_accumulator_bytes_and_updates_low_byte_flags() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x1234;
        cpu.registers.p = 0x00;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0xEB)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a, 0x3412);
        assert_eq!(cpu.registers.pc, 0x8001);
        assert_eq!(cpu.registers.p & 0x02, 0);
    }
}
