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
        if bus.poll_nmi() {
            let mut trace = Vec::new();
            self.service_interrupt(bus, &mut trace, InterruptKind::Nmi)?;
            self.cycles = trace.len() as u64;
            return Ok(trace);
        }

        if self.irq_enabled() && bus.poll_irq() {
            let mut trace = Vec::new();
            self.service_interrupt(bus, &mut trace, InterruptKind::Irq)?;
            self.cycles = trace.len() as u64;
            return Ok(trace);
        }

        let opcode_address = self.program_address();
        let opcode = bus.read(opcode_address);
        let mut trace = vec![BusEvent {
            address: opcode_address,
            value: opcode,
            access: AccessKind::Read,
            cycle: 0,
        }];

        match opcode {
            0x01 => self.execute_ora_direct_page_indexed_indirect_x(bus, &mut trace),
            0x02 => self.execute_cop(bus, &mut trace),
            0x03 => self.execute_ora_stack_relative(bus, &mut trace),
            0x04 => self.execute_tsb_direct_page(bus, &mut trace),
            0x05 => self.execute_ora_direct_page(bus, &mut trace),
            0x06 => self.execute_asl_direct_page(bus, &mut trace),
            0x07 => self.execute_ora_direct_page_indirect_long(bus, &mut trace),
            0x08 => self.execute_php(bus, &mut trace),
            0x0A => self.execute_asl_a(bus, &mut trace),
            0x0B => self.execute_phd(bus, &mut trace),
            0x09 => self.execute_ora_immediate(bus, &mut trace),
            0x0D => self.execute_ora_absolute(bus, &mut trace),
            0x0E => self.execute_asl_absolute(bus, &mut trace),
            0x0F => self.execute_ora_long(bus, &mut trace),
            0x10 => self.execute_bpl(bus, &mut trace),
            0x11 => self.execute_ora_direct_page_indirect_y(bus, &mut trace),
            0x12 => self.execute_ora_direct_page_indirect(bus, &mut trace),
            0x13 => self.execute_ora_stack_relative_indirect_y(bus, &mut trace),
            0x15 => self.execute_ora_direct_page_x(bus, &mut trace),
            0x16 => self.execute_asl_direct_page_x(bus, &mut trace),
            0x17 => self.execute_ora_direct_page_indirect_long_y(bus, &mut trace),
            0x19 => self.execute_ora_absolute_y(bus, &mut trace),
            0xEA => self.execute_nop(bus, &mut trace),
            0x00 => self.execute_brk(bus, &mut trace),
            0xE6 => self.execute_inc_direct_page(bus, &mut trace),
            0x1B => self.execute_tcs(bus, &mut trace),
            0x18 => self.execute_clc(bus, &mut trace),
            0x1A => self.execute_inc_a(bus, &mut trace),
            0x1D => self.execute_ora_absolute_x(bus, &mut trace),
            0x1E => self.execute_asl_absolute_x(bus, &mut trace),
            0x1F => self.execute_ora_long_x(bus, &mut trace),
            0x20 => self.execute_jsr_absolute(bus, &mut trace),
            0x22 => self.execute_jsl_long(bus, &mut trace),
            0x21 => self.execute_and_direct_page_indexed_indirect_x(bus, &mut trace),
            0x28 => self.execute_plp(bus, &mut trace),
            0x29 => self.execute_and_immediate(bus, &mut trace),
            0x2A => self.execute_rol_a(bus, &mut trace),
            0x2B => self.execute_pld(bus, &mut trace),
            0x25 => self.execute_and_direct_page(bus, &mut trace),
            0x26 => self.execute_rol_direct_page(bus, &mut trace),
            0x27 => self.execute_and_direct_page_indirect_long(bus, &mut trace),
            0x2D => self.execute_and_absolute(bus, &mut trace),
            0x2E => self.execute_rol_absolute(bus, &mut trace),
            0x2C => self.execute_bit_absolute(bus, &mut trace),
            0x30 => self.execute_bmi(bus, &mut trace),
            0x31 => self.execute_and_direct_page_indirect_y(bus, &mut trace),
            0x32 => self.execute_and_direct_page_indirect(bus, &mut trace),
            0x35 => self.execute_and_direct_page_x(bus, &mut trace),
            0x36 => self.execute_rol_direct_page_x(bus, &mut trace),
            0x37 => self.execute_and_direct_page_indirect_long_y(bus, &mut trace),
            0x38 => self.execute_sec(bus, &mut trace),
            0x39 => self.execute_and_absolute_y(bus, &mut trace),
            0x3A => self.execute_dec_a(bus, &mut trace),
            0x3B => self.execute_tsc(bus, &mut trace),
            0x3D => self.execute_and_absolute_x(bus, &mut trace),
            0x3E => self.execute_rol_absolute_x(bus, &mut trace),
            0x3F => self.execute_and_long_x(bus, &mut trace),
            0x40 => self.execute_rti(bus, &mut trace),
            0x48 => self.execute_pha(bus, &mut trace),
            0x49 => self.execute_eor_immediate(bus, &mut trace),
            0x4A => self.execute_lsr_a(bus, &mut trace),
            0x4C => self.execute_jmp_absolute(bus, &mut trace),
            0x4B => self.execute_phk(bus, &mut trace),
            0x4D => self.execute_eor_absolute(bus, &mut trace),
            0x46 => self.execute_lsr_direct_page(bus, &mut trace),
            0x4E => self.execute_lsr_absolute(bus, &mut trace),
            0x50 => self.execute_bvc(bus, &mut trace),
            0x5A => self.execute_phy(bus, &mut trace),
            0x58 => self.execute_cli(bus, &mut trace),
            0x56 => self.execute_lsr_direct_page_x(bus, &mut trace),
            0x5E => self.execute_lsr_absolute_x(bus, &mut trace),
            0x60 => self.execute_rts(bus, &mut trace),
            0x63 => self.execute_adc_stack_relative(bus, &mut trace),
            0x64 => self.execute_stz_direct_page(bus, &mut trace),
            0x65 => self.execute_adc_direct_page(bus, &mut trace),
            0x66 => self.execute_ror_direct_page(bus, &mut trace),
            0x68 => self.execute_pla(bus, &mut trace),
            0x69 => self.execute_adc_immediate(bus, &mut trace),
            0x6D => self.execute_adc_absolute(bus, &mut trace),
            0x6A => self.execute_ror_a(bus, &mut trace),
            0x6E => self.execute_ror_absolute(bus, &mut trace),
            0x6F => self.execute_adc_long(bus, &mut trace),
            0x70 => self.execute_bvs(bus, &mut trace),
            0x73 => self.execute_adc_stack_relative_indirect_y(bus, &mut trace),
            0x79 => self.execute_adc_absolute_y(bus, &mut trace),
            0x7A => self.execute_ply(bus, &mut trace),
            0x7D => self.execute_adc_absolute_x(bus, &mut trace),
            0x76 => self.execute_ror_direct_page_x(bus, &mut trace),
            0x7E => self.execute_ror_absolute_x(bus, &mut trace),
            0x7F => self.execute_adc_long_x(bus, &mut trace),
            0x74 => self.execute_stz_direct_page_x(bus, &mut trace),
            0x6B => self.execute_rtl(bus, &mut trace),
            0x5B => self.execute_tcd(bus, &mut trace),
            0x78 => self.execute_sei(bus, &mut trace),
            0x80 => self.execute_bra(bus, &mut trace),
            0x81 => self.execute_sta_direct_page_indexed_indirect_x(bus, &mut trace),
            0x82 => self.execute_brl(bus, &mut trace),
            0x83 => self.execute_sta_stack_relative(bus, &mut trace),
            0x7B => self.execute_tdc(bus, &mut trace),
            0x84 => self.execute_sty_direct_page(bus, &mut trace),
            0x85 => self.execute_sta_direct_page(bus, &mut trace),
            0x86 => self.execute_stx_direct_page(bus, &mut trace),
            0x87 => self.execute_sta_direct_page_indirect_long(bus, &mut trace),
            0x88 => self.execute_dey(bus, &mut trace),
            0x90 => self.execute_bcc(bus, &mut trace),
            0x91 => self.execute_sta_direct_page_indirect_y(bus, &mut trace),
            0x92 => self.execute_sta_direct_page_indirect(bus, &mut trace),
            0x93 => self.execute_sta_stack_relative_indirect_y(bus, &mut trace),
            0x8B => self.execute_phb(bus, &mut trace),
            0x94 => self.execute_sty_direct_page_x(bus, &mut trace),
            0x95 => self.execute_sta_direct_page_x(bus, &mut trace),
            0x96 => self.execute_stx_direct_page_y(bus, &mut trace),
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
            0xA1 => self.execute_lda_direct_page_indexed_indirect_x(bus, &mut trace),
            0xA7 => self.execute_lda_direct_page_indirect_long(bus, &mut trace),
            0xA9 => self.execute_lda_immediate(bus, &mut trace),
            0xAE => self.execute_ldx_absolute(bus, &mut trace),
            0xAC => self.execute_ldy_absolute(bus, &mut trace),
            0xAB => self.execute_plb(bus, &mut trace),
            0x98 => self.execute_tya(bus, &mut trace),
            0xAD => self.execute_lda_absolute(bus, &mut trace),
            0xAF => self.execute_lda_long(bus, &mut trace),
            0x9A => self.execute_txs(bus, &mut trace),
            0xB8 => self.execute_clv(bus, &mut trace),
            0xB0 => self.execute_bcs(bus, &mut trace),
            0xB4 => self.execute_ldy_direct_page_x(bus, &mut trace),
            0xB1 => self.execute_lda_direct_page_indirect_y(bus, &mut trace),
            0xB2 => self.execute_lda_direct_page_indirect(bus, &mut trace),
            0xB5 => self.execute_lda_direct_page_x(bus, &mut trace),
            0xB6 => self.execute_ldx_direct_page_y(bus, &mut trace),
            0xB7 => self.execute_lda_direct_page_indirect_long_y(bus, &mut trace),
            0xB9 => self.execute_lda_absolute_y(bus, &mut trace),
            0xBC => self.execute_ldy_absolute_x(bus, &mut trace),
            0xBD => self.execute_lda_absolute_x(bus, &mut trace),
            0xBE => self.execute_ldx_absolute_y(bus, &mut trace),
            0xBF => self.execute_lda_long_x(bus, &mut trace),
            0xBB => self.execute_tyx(bus, &mut trace),
            0xC8 => self.execute_iny(bus, &mut trace),
            0xCA => self.execute_dex(bus, &mut trace),
            0xC5 => self.execute_cmp_direct_page(bus, &mut trace),
            0xC6 => self.execute_dec_direct_page(bus, &mut trace),
            0xC9 => self.execute_cmp_immediate(bus, &mut trace),
            0xCC => self.execute_cpy_absolute(bus, &mut trace),
            0xCD => self.execute_cmp_absolute(bus, &mut trace),
            0xCE => self.execute_dec_absolute(bus, &mut trace),
            0xDC => self.execute_jmp_absolute_indirect_long(bus, &mut trace),
            0xDA => self.execute_phx(bus, &mut trace),
            0xBA => self.execute_tsx(bus, &mut trace),
            0xC2 => self.execute_rep(bus, &mut trace),
            0xC0 => self.execute_cpy_immediate(bus, &mut trace),
            0xC4 => self.execute_cpy_direct_page(bus, &mut trace),
            0xD0 => self.execute_bne(bus, &mut trace),
            0xD5 => self.execute_cmp_direct_page_x(bus, &mut trace),
            0xD6 => self.execute_dec_direct_page_x(bus, &mut trace),
            0xD9 => self.execute_cmp_absolute_y(bus, &mut trace),
            0xD8 => self.execute_cld(bus, &mut trace),
            0xDD => self.execute_cmp_absolute_x(bus, &mut trace),
            0xDE => self.execute_dec_absolute_x(bus, &mut trace),
            0xE0 => self.execute_cpx_immediate(bus, &mut trace),
            0xE4 => self.execute_cpx_direct_page(bus, &mut trace),
            0xEC => self.execute_cpx_absolute(bus, &mut trace),
            0xE8 => self.execute_inx(bus, &mut trace),
            0xE9 => self.execute_sbc_immediate(bus, &mut trace),
            0xEB => self.execute_xba(bus, &mut trace),
            0xE2 => self.execute_sep(bus, &mut trace),
            0xEE => self.execute_inc_absolute(bus, &mut trace),
            0xF0 => self.execute_beq(bus, &mut trace),
            0xF6 => self.execute_inc_direct_page_x(bus, &mut trace),
            0xFA => self.execute_plx(bus, &mut trace),
            0xFB => self.execute_xce(bus, &mut trace),
            0xFC => self.execute_jsr_absolute_indexed_indirect_x(bus, &mut trace),
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

    fn irq_enabled(&self) -> bool {
        self.registers.p & 0x04 == 0
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

    fn execute_brl<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let offset = self.fetch_operand_u16(bus, trace) as i16;
        let next = self.registers.pc.wrapping_add(3);
        self.registers.pc = next.wrapping_add_signed(offset);
        Ok(())
    }

    fn execute_asl_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.asl_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_asl_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.asl_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_asl_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.asl_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_asl_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.asl_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_rol_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.rol_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_rol_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.rol_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_rol_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.rol_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_rol_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.rol_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_bvs<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x40 != 0)
    }

    fn execute_bvc<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x40 == 0)
    }

    fn execute_bcc<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x01 == 0)
    }

    fn execute_bcs<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.execute_branch_relative(bus, trace, self.registers.p & 0x01 != 0)
    }

    fn execute_ora_stack_relative<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.stack_relative_address(operand);
        if self.accumulator_is_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.or_accumulator_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.or_accumulator_16(rhs);
        }
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ora_stack_relative_indirect_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.stack_relative_indirect_y_address(bus, trace, operand);
        if self.accumulator_is_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.or_accumulator_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.or_accumulator_16(rhs);
        }
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
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
        self.registers.s = if self.registers.emulation {
            0x0100 | (self.registers.x & 0x00FF)
        } else {
            self.registers.x
        };
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_tcs<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        self.registers.s = if self.registers.emulation {
            0x0100 | (self.registers.a & 0x00FF)
        } else {
            self.registers.a
        };
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

    fn execute_dec_a<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            let value = (self.registers.a as u8).wrapping_sub(1);
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(value);
            self.update_nz_8(value);
        } else {
            self.registers.a = self.registers.a.wrapping_sub(1);
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

    // PHD and PLD operate on the full 16-bit direct-page register,
    // independently of the accumulator's M flag.
    fn execute_phd<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        let [low, high] = self.registers.d.to_le_bytes();
        self.push_stack(bus, trace, high)?;
        self.push_stack(bus, trace, low)?;
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_pld<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        let low = self.pull_stack(bus, trace);
        let high = self.pull_stack(bus, trace);
        self.registers.d = u16::from_le_bytes([low, high]);
        self.update_nz_16(self.registers.d);
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

    fn execute_jsr_absolute_indexed_indirect_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let pointer = self
            .fetch_operand_u16(bus, trace)
            .wrapping_add(self.registers.x);
        let low = self.read_u8_trace(bus, trace, u32::from(pointer));
        let high = self.read_u8_trace(bus, trace, u32::from(pointer.wrapping_add(1)));
        let target = u16::from_le_bytes([low, high]);
        let return_pc = self.registers.pc.wrapping_add(2);
        self.push_stack(bus, trace, (return_pc >> 8) as u8)?;
        self.push_stack(bus, trace, (return_pc & 0x00FF) as u8)?;
        self.registers.pc = target;
        Ok(())
    }

    fn execute_jmp_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
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

    fn execute_rti<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        let mut status = self.pull_stack(bus, trace);
        if self.registers.emulation {
            status |= 0x30;
        }
        self.registers.p = status;
        let low = self.pull_stack(bus, trace);
        let high = self.pull_stack(bus, trace);
        self.registers.pc = u16::from_le_bytes([low, high]);
        if !self.registers.emulation {
            self.registers.pbr = self.pull_stack(bus, trace);
        }
        if self.index_registers_are_8_bit() {
            self.registers.x &= 0x00FF;
            self.registers.y &= 0x00FF;
        }
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

    fn execute_lda_direct_page_indirect<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let address = self.absolute_address(u16::from_le_bytes([low, high]));
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_lda_direct_page_indirect_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let base = u16::from_le_bytes([low, high]);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_lda_direct_page_indexed_indirect_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_indexed_x_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let address = self.absolute_address(u16::from_le_bytes([low, high]));
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

    fn execute_lda_long<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        // 24-bit absolute addressing ignores DBR and carries across banks.
        let address = self.fetch_operand_u24(bus, trace);
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(4);
        Ok(())
    }

    fn execute_lda_long_x<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        // Unlike absolute,X, long,X can cross a 64 KiB bank boundary.
        let address = self
            .fetch_operand_u24(bus, trace)
            .wrapping_add(u32::from(self.registers.x))
            & 0x00FF_FFFF;
        self.load_accumulator_from_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(4);
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

    fn execute_ldy_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.load_index_y_from_address(bus, trace, address);
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

    fn execute_sty_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
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

    fn execute_sta_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_sta_direct_page_indexed_indirect_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_indexed_x_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let address = self.absolute_address(u16::from_le_bytes([low, high]));
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_sta_stack_relative<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.stack_relative_address(operand);
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_sta_direct_page_indirect_long<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_indirect_long_address(bus, trace, operand);
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
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

    fn execute_stx_direct_page_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.y & 0x00FF));
        self.store_index_x_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
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

    fn execute_sta_direct_page_indirect_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let base = u16::from_le_bytes([low, high]);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_sta_direct_page_indirect<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let address = self.absolute_address(u16::from_le_bytes([low, high]));
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_sta_stack_relative_indirect_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.stack_relative_indirect_y_address(bus, trace, operand);
        self.store_accumulator_to_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
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
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ora_immediate<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        if self.accumulator_is_8_bit() {
            let rhs = self.push_read_trace(bus, trace, self.fetch_address(1));
            self.or_accumulator_8(rhs);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let rhs = self.fetch_operand_u16(bus, trace);
            self.or_accumulator_16(rhs);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_ora_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_indexed_x_address(operand);
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ora_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_ora_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_ora_absolute_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_ora_long<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let address = self.fetch_operand_u24(bus, trace);
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(4);
        Ok(())
    }

    fn execute_ora_long_x<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let address = self
            .fetch_operand_u24(bus, trace)
            .wrapping_add(u32::from(self.registers.x));
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(4);
        Ok(())
    }

    fn execute_ora_direct_page_indirect<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let address = self.absolute_address(u16::from_le_bytes([low, high]));
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ora_direct_page_indirect_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let base = u16::from_le_bytes([low, high]);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ora_direct_page_indexed_indirect_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_indexed_x_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let address = self.absolute_address(u16::from_le_bytes([low, high]));
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ora_direct_page_indirect_long<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_indirect_long_address(bus, trace, operand);
        self.or_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ora_direct_page_indirect_long_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_indirect_long_address(bus, trace, operand)
            .wrapping_add(u32::from(self.registers.y));
        self.or_accumulator_with_address(bus, trace, address);
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
            self.and_accumulator_8(operand);
            self.registers.pc = self.registers.pc.wrapping_add(2);
        } else {
            let operand = self.fetch_operand_u16(bus, trace);
            self.and_accumulator_16(operand);
            self.registers.pc = self.registers.pc.wrapping_add(3);
        }
        Ok(())
    }

    fn execute_and_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_and_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_indexed_x_address(operand);
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_and_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_and_absolute_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_and_long_x<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let address = self
            .fetch_operand_u24(bus, trace)
            .wrapping_add(u32::from(self.registers.x));
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(4);
        Ok(())
    }

    fn execute_and_direct_page_indirect<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let address = self.absolute_address(u16::from_le_bytes([low, high]));
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_and_direct_page_indirect_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let base = u16::from_le_bytes([low, high]);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_and_direct_page_indexed_indirect_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let pointer = self.direct_page_indexed_x_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let address = self.absolute_address(u16::from_le_bytes([low, high]));
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_and_direct_page_indirect_long<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_indirect_long_address(bus, trace, operand);
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_and_direct_page_indirect_long_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_indirect_long_address(bus, trace, operand)
            .wrapping_add(u32::from(self.registers.y));
        self.and_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
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
        self.and_accumulator_with_address(bus, trace, address);
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

    fn execute_adc_stack_relative<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.stack_relative_address(operand);
        self.add_accumulator_with_address(bus, trace, address);
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

    fn execute_adc_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.add_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_adc_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.add_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_adc_absolute_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.y));
        self.add_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_adc_long<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let address = self.fetch_operand_u24(bus, trace);
        self.add_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(4);
        Ok(())
    }

    fn execute_adc_long_x<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        let base = self.fetch_operand_u24(bus, trace);
        let address = base.wrapping_add(u32::from(self.registers.x));
        self.add_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(4);
        Ok(())
    }

    fn execute_adc_stack_relative_indirect_y<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.stack_relative_indirect_y_address(bus, trace, operand);
        self.add_accumulator_with_address(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
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

    fn execute_cpx_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        if self.index_registers_are_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.compare_index_x_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.compare_index_x_16(rhs);
        }
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_cpy_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        if self.index_registers_are_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.compare_index_y_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.compare_index_y_16(rhs);
        }
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_cpx_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        if self.index_registers_are_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.compare_index_x_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.compare_index_x_16(rhs);
        }
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_cpy_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        if self.index_registers_are_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.compare_index_y_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.compare_index_y_16(rhs);
        }
        self.registers.pc = self.registers.pc.wrapping_add(3);
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

    fn execute_lsr_a<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            let value = self.registers.a as u8;
            self.set_carry(value & 0x01 != 0);
            let result = value >> 1;
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
            self.update_nz_8(result);
        } else {
            let value = self.registers.a;
            self.set_carry(value & 0x0001 != 0);
            let result = value >> 1;
            self.registers.a = result;
            self.update_nz_16(result);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_ror_a<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
        self.push_read_trace(bus, trace, self.fetch_address(1));
        if self.accumulator_is_8_bit() {
            let value = self.registers.a as u8;
            let incoming = u8::from(self.registers.p & 0x01 != 0) << 7;
            self.set_carry(value & 0x01 != 0);
            let result = (value >> 1) | incoming;
            self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
            self.update_nz_8(result);
        } else {
            let value = self.registers.a;
            let incoming = u16::from(self.registers.p & 0x01 != 0) << 15;
            self.set_carry(value & 0x0001 != 0);
            let result = (value >> 1) | incoming;
            self.registers.a = result;
            self.update_nz_16(result);
        }
        self.registers.pc = self.registers.pc.wrapping_add(1);
        Ok(())
    }

    fn execute_lsr_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.lsr_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_lsr_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.lsr_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_lsr_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.lsr_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_lsr_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.lsr_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_ror_direct_page<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self.direct_page_address(operand);
        self.ror_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ror_direct_page_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let operand = self.push_read_trace(bus, trace, self.fetch_address(1));
        let address = self
            .direct_page_address(operand)
            .wrapping_add(u32::from(self.registers.x & 0x00FF));
        self.ror_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(2);
        Ok(())
    }

    fn execute_ror_absolute<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let address = self.absolute_address(self.fetch_operand_u16(bus, trace));
        self.ror_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
        Ok(())
    }

    fn execute_ror_absolute_x<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
    ) -> Result<()> {
        let base = self.fetch_operand_u16(bus, trace);
        let address = self.absolute_address(base.wrapping_add(self.registers.x));
        self.ror_memory(bus, trace, address);
        self.registers.pc = self.registers.pc.wrapping_add(3);
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
        self.finish_interrupt_vector_load(bus, trace, vector_base);
        Ok(())
    }

    fn execute_cop<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> Result<()> {
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
            0x00FFF4
        } else {
            0x00FFE4
        };
        self.finish_interrupt_vector_load(bus, trace, vector_base);
        Ok(())
    }

    fn service_interrupt<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        kind: InterruptKind,
    ) -> Result<()> {
        if !self.registers.emulation {
            self.push_stack(bus, trace, self.registers.pbr)?;
        }
        self.push_stack(bus, trace, (self.registers.pc >> 8) as u8)?;
        self.push_stack(bus, trace, (self.registers.pc & 0x00FF) as u8)?;
        // In emulation mode, the pushed status has B=0 for hardware IRQ/NMI.
        // In native mode that same bit is X (index width), not B: clearing it
        // silently changes 8-bit index registers to 16-bit after RTI.
        let stacked_status = if self.registers.emulation {
            self.registers.p & !0x10
        } else {
            self.registers.p
        };
        self.push_stack(bus, trace, stacked_status)?;
        self.finish_interrupt_vector_load(bus, trace, kind.vector_base(self.registers.emulation));
        Ok(())
    }

    fn finish_interrupt_vector_load<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        vector_base: Address,
    ) {
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
        // Interrupt vectors always target bank zero in either CPU mode.
        self.registers.pbr = 0;
        self.registers.p = (self.registers.p | 0x04) & !0x08;
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
        if self.registers.emulation {
            self.registers.s = 0x0100 | (self.registers.s & 0x00FF);
        }
        Ok(())
    }

    fn pull_stack<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>) -> u8 {
        self.registers.s = self.registers.s.wrapping_add(1);
        if self.registers.emulation {
            self.registers.s = 0x0100 | (self.registers.s & 0x00FF);
        }
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

    fn or_accumulator_with_address<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.accumulator_is_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.or_accumulator_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.or_accumulator_16(rhs);
        }
    }

    fn and_accumulator_with_address<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.accumulator_is_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.and_accumulator_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.and_accumulator_16(rhs);
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

    fn add_accumulator_8(&mut self, rhs: u8) {
        let lhs = self.registers.a as u8;
        let carry = u8::from(self.registers.p & 0x01 != 0);
        let (tmp, carry1) = lhs.overflowing_add(rhs);
        let (result, carry2) = tmp.overflowing_add(carry);
        self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
        self.set_carry(carry1 || carry2);
        self.set_overflow((!(lhs ^ rhs) & (lhs ^ result) & 0x80) != 0);
        self.update_nz_8(result);
    }

    fn add_accumulator_16(&mut self, rhs: u16) {
        let lhs = self.registers.a;
        let carry = u16::from(self.registers.p & 0x01 != 0);
        let (tmp, carry1) = lhs.overflowing_add(rhs);
        let (result, carry2) = tmp.overflowing_add(carry);
        self.registers.a = result;
        self.set_carry(carry1 || carry2);
        self.set_overflow((!(lhs ^ rhs) & (lhs ^ result) & 0x8000) != 0);
        self.update_nz_16(result);
    }

    fn add_accumulator_with_address<B: Bus>(
        &mut self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        address: Address,
    ) {
        if self.accumulator_is_8_bit() {
            let rhs = self.read_u8_trace(bus, trace, address);
            self.add_accumulator_8(rhs);
        } else {
            let rhs = self.read_u16_trace(bus, trace, address);
            self.add_accumulator_16(rhs);
        }
    }

    fn asl_memory<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>, address: Address) {
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            self.set_carry(value & 0x80 != 0);
            let result = value << 1;
            self.write_u8_trace(bus, trace, address, result);
            self.update_nz_8(result);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            self.set_carry(value & 0x8000 != 0);
            let result = value << 1;
            self.write_u16_trace(bus, trace, address, result);
            self.update_nz_16(result);
        }
    }

    fn lsr_memory<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>, address: Address) {
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            self.set_carry(value & 0x01 != 0);
            let result = value >> 1;
            self.write_u8_trace(bus, trace, address, result);
            self.update_nz_8(result);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            self.set_carry(value & 0x0001 != 0);
            let result = value >> 1;
            self.write_u16_trace(bus, trace, address, result);
            self.update_nz_16(result);
        }
    }

    fn rol_memory<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>, address: Address) {
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            let incoming = u8::from(self.registers.p & 0x01 != 0);
            self.set_carry(value & 0x80 != 0);
            let result = (value << 1) | incoming;
            self.write_u8_trace(bus, trace, address, result);
            self.update_nz_8(result);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            let incoming = u16::from(self.registers.p & 0x01 != 0);
            self.set_carry(value & 0x8000 != 0);
            let result = (value << 1) | incoming;
            self.write_u16_trace(bus, trace, address, result);
            self.update_nz_16(result);
        }
    }

    fn ror_memory<B: Bus>(&mut self, bus: &mut B, trace: &mut Vec<BusEvent>, address: Address) {
        if self.accumulator_is_8_bit() {
            let value = self.read_u8_trace(bus, trace, address);
            let incoming = u8::from(self.registers.p & 0x01 != 0) << 7;
            self.set_carry(value & 0x01 != 0);
            let result = (value >> 1) | incoming;
            self.write_u8_trace(bus, trace, address, result);
            self.update_nz_8(result);
        } else {
            let value = self.read_u16_trace(bus, trace, address);
            let incoming = u16::from(self.registers.p & 0x01 != 0) << 15;
            self.set_carry(value & 0x0001 != 0);
            let result = (value >> 1) | incoming;
            self.write_u16_trace(bus, trace, address, result);
            self.update_nz_16(result);
        }
    }

    fn or_accumulator_8(&mut self, rhs: u8) {
        let result = (self.registers.a as u8) | rhs;
        self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
        self.update_nz_8(result);
    }

    fn or_accumulator_16(&mut self, rhs: u16) {
        self.registers.a |= rhs;
        self.update_nz_16(self.registers.a);
    }

    fn and_accumulator_8(&mut self, rhs: u8) {
        let result = (self.registers.a as u8) & rhs;
        self.registers.a = (self.registers.a & 0xFF00) | u16::from(result);
        self.update_nz_8(result);
    }

    fn and_accumulator_16(&mut self, rhs: u16) {
        self.registers.a &= rhs;
        self.update_nz_16(self.registers.a);
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

    fn direct_page_indirect_long_address<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        operand: u8,
    ) -> Address {
        let base = self.direct_page_address(operand);
        let low = self.read_u8_trace(bus, trace, base);
        let high = self.read_u8_trace(bus, trace, base.wrapping_add(1));
        let bank = self.read_u8_trace(bus, trace, base.wrapping_add(2));
        u32::from(low) | (u32::from(high) << 8) | (u32::from(bank) << 16)
    }

    fn stack_relative_address(&self, operand: u8) -> Address {
        u32::from(self.stack_address().wrapping_add(u16::from(operand)))
    }

    fn stack_relative_indirect_y_address<B: Bus>(
        &self,
        bus: &mut B,
        trace: &mut Vec<BusEvent>,
        operand: u8,
    ) -> Address {
        let pointer = self.stack_relative_address(operand);
        let low = self.read_u8_trace(bus, trace, pointer);
        let high = self.read_u8_trace(bus, trace, pointer.wrapping_add(1));
        let base = u16::from_le_bytes([low, high]);
        self.absolute_address(base.wrapping_add(self.registers.y))
    }
}

#[derive(Clone, Copy)]
enum InterruptKind {
    Nmi,
    Irq,
}

impl InterruptKind {
    fn vector_base(self, emulation: bool) -> Address {
        match (self, emulation) {
            (Self::Nmi, true) => 0x00FFFA,
            (Self::Nmi, false) => 0x00FFEA,
            (Self::Irq, true) => 0x00FFFE,
            (Self::Irq, false) => 0x00FFEE,
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
        pending_nmi: bool,
        pending_irq: bool,
    }

    impl TestBus {
        fn with_bytes(bytes: &[(Address, u8)]) -> Self {
            let mut map = HashMap::new();
            for (address, value) in bytes {
                map.insert(*address, *value);
            }
            Self {
                bytes: map,
                pending_nmi: false,
                pending_irq: false,
            }
        }
    }

    impl Bus for TestBus {
        fn read(&mut self, address: Address) -> u8 {
            self.bytes.get(&address).copied().unwrap_or(0)
        }

        fn write(&mut self, address: Address, value: u8) {
            self.bytes.insert(address, value);
        }

        fn poll_nmi(&mut self) -> bool {
            let pending = self.pending_nmi;
            self.pending_nmi = false;
            pending
        }

        fn poll_irq(&mut self) -> bool {
            let pending = self.pending_irq;
            self.pending_irq = false;
            pending
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

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x64), (0x008001, 0x22), (0x000032, 0xFE)]);

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

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x8C), (0x008001, 0x78), (0x008002, 0x56)]);

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

        let mut bus = TestBus::with_bytes(&[(0x008000, 0xE6), (0x008001, 0x10), (0x000030, 0x7F)]);

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

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x7A), (0x0001FE, 0x34), (0x0001FF, 0x12)]);

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

        let mut bus = TestBus::with_bytes(&[(0x008000, 0xFA), (0x0001FE, 0x78), (0x0001FF, 0x56)]);

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

        let mut bus = TestBus::with_bytes(&[(0x008000, 0xB6), (0x008001, 0x20), (0x000033, 0xA5)]);

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
    fn ora_direct_page_indirect_long_updates_accumulator() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.a = 0x0003;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x07),
            (0x008001, 0x10),
            (0x000030, 0x78),
            (0x000031, 0x56),
            (0x000032, 0x7E),
            (0x7E5678, 0x0C),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(cpu.registers.a & 0x00FF, 0x0F);
    }

    #[test]
    fn ora_direct_page_indirect_long_y_uses_long_pointer_plus_y() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.y = 0x0004;
        cpu.registers.a = 0x0001;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x17),
            (0x008001, 0x10),
            (0x000030, 0x78),
            (0x000031, 0x56),
            (0x000032, 0x7E),
            (0x7E567C, 0x06),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(cpu.registers.a & 0x00FF, 0x07);
    }

    #[test]
    fn lda_direct_page_indirect_loads_accumulator_from_pointer() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xB2),
            (0x008001, 0x10),
            (0x000030, 0x78),
            (0x000031, 0x56),
            (0x005678, 0xA5),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_eq!(cpu.registers.a & 0x00FF, 0xA5);
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

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x4C), (0x008001, 0x56), (0x008002, 0x34)]);

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
    fn and_direct_page_updates_accumulator() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.a = 0x00F0;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x25), (0x008001, 0x10), (0x000030, 0xAA)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
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
    fn ldy_absolute_loads_index_y() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xAC),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x001234, 0x7F),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8003);
        assert_eq!(cpu.registers.y, 0x007F);
        assert_eq!(cpu.registers.p & 0x80, 0);
        assert_eq!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn cpy_absolute_updates_flags() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.y = 0x0050;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xCC),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x001234, 0x40),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8003);
        assert_ne!(cpu.registers.p & 0x01, 0);
        assert_eq!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn cpx_absolute_updates_flags() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.x = 0x0040;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xEC),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x001234, 0x40),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8003);
        assert_ne!(cpu.registers.p & 0x01, 0);
        assert_ne!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn cpx_direct_page_updates_flags() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.x = 0x0040;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0xE4), (0x008001, 0x10), (0x000030, 0x40)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8002);
        assert_ne!(cpu.registers.p & 0x01, 0);
        assert_ne!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn brl_applies_signed_16_bit_offset() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x82), (0x008001, 0xFC), (0x008002, 0xFF)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x7FFF);
    }

    #[test]
    fn bvc_branches_when_overflow_clear() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.p = 0x00;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x50), (0x008001, 0x02)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x8004);
    }

    #[test]
    fn jsr_absolute_indexed_indirect_uses_indexed_pointer() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.x = 0x0004;
        cpu.registers.s = 0x01FF;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0xFC),
            (0x008001, 0x00),
            (0x008002, 0x20),
            (0x00002004, 0x34),
            (0x00002005, 0x12),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x1234);
        assert_eq!(bus.read(0x0001FF), 0x80);
        assert_eq!(bus.read(0x0001FE), 0x02);
    }

    #[test]
    fn sty_direct_page_x_uses_indexed_target() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.x = 0x0004;
        cpu.registers.y = 0x005A;
        cpu.registers.p = 0x10;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x94), (0x008001, 0x10)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x000034), 0x5A);
    }

    #[test]
    fn sta_direct_page_indexed_indirect_x_uses_indexed_pointer_target() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.x = 0x0004;
        cpu.registers.a = 0x005A;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x81),
            (0x008001, 0x10),
            (0x000034, 0x78),
            (0x000035, 0x56),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x005678), 0x5A);
    }

    #[test]
    fn sta_stack_relative_stores_to_stack_page_offset() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01F0;
        cpu.registers.a = 0x005A;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x83), (0x008001, 0x10)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x000200), 0x5A);
    }

    #[test]
    fn sta_direct_page_indirect_stores_to_pointer_target() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.a = 0x005A;
        cpu.registers.p = 0x20;
        cpu.registers.dbr = 0x7E;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x92),
            (0x008001, 0x10),
            (0x000030, 0x34),
            (0x000031, 0x12),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x7E1234), 0x5A);
    }

    #[test]
    fn sta_direct_page_indirect_y_stores_to_indexed_pointer_target() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.y = 0x0002;
        cpu.registers.a = 0x005A;
        cpu.registers.p = 0x20;
        cpu.registers.dbr = 0x7E;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x91),
            (0x008001, 0x10),
            (0x000030, 0x34),
            (0x000031, 0x12),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x7E1236), 0x5A);
    }

    #[test]
    fn sta_stack_relative_indirect_y_stores_to_stack_pointer_target() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01F0;
        cpu.registers.y = 0x0002;
        cpu.registers.a = 0x005A;
        cpu.registers.p = 0x20;
        cpu.registers.dbr = 0x7E;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x93),
            (0x008001, 0x10),
            (0x000200, 0x34),
            (0x000201, 0x12),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x7E1236), 0x5A);
    }

    #[test]
    fn sta_direct_page_indirect_long_stores_to_long_pointer_target() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.a = 0x005A;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x87),
            (0x008001, 0x10),
            (0x000030, 0x34),
            (0x000031, 0x12),
            (0x000032, 0x7E),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x7E1234), 0x5A);
    }

    #[test]
    fn ora_stack_relative_reads_from_stack_page() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01F0;
        cpu.registers.a = 0x000F;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x03), (0x008001, 0x10), (0x000200, 0xF0)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0xFF);
    }

    #[test]
    fn ora_stack_relative_indirect_y_uses_dbr_indexed_target() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01F0;
        cpu.registers.y = 0x0002;
        cpu.registers.dbr = 0x7E;
        cpu.registers.a = 0x0001;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x13),
            (0x008001, 0x10),
            (0x000200, 0x34),
            (0x000201, 0x12),
            (0x7E1236, 0x80),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x81);
    }

    #[test]
    fn asl_direct_page_shifts_memory_and_sets_carry() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x06), (0x008001, 0x10), (0x000030, 0x81)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x000030), 0x02);
        assert_ne!(cpu.registers.p & 0x01, 0);
    }

    #[test]
    fn rol_absolute_rotates_carry_into_memory() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.p = 0x21;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x2E),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x001234, 0x40),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x001234), 0x81);
        assert_eq!(cpu.registers.p & 0x01, 0);
    }

    #[test]
    fn lsr_absolute_shifts_memory_right() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x4E),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x001234, 0x03),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x001234), 0x01);
        assert_ne!(cpu.registers.p & 0x01, 0);
    }

    #[test]
    fn ror_direct_page_x_rotates_memory_right_through_carry() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.x = 0x0004;
        cpu.registers.p = 0x21;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x76), (0x008001, 0x10), (0x000034, 0x02)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(bus.read(0x000034), 0x81);
        assert_eq!(cpu.registers.p & 0x01, 0);
    }

    #[test]
    fn dec_direct_page_decrements_memory_and_updates_flags() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.d = 0x0020;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0xC6), (0x008001, 0x10), (0x000030, 0x01)]);

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
    fn adc_absolute_adds_memory_operand() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x0005;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x6D),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x001234, 0x07),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x0C);
        assert_eq!(cpu.registers.pc, 0x8003);
    }

    #[test]
    fn adc_long_x_adds_long_addressed_operand() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x0005;
        cpu.registers.x = 0x0002;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x7F),
            (0x008001, 0x34),
            (0x008002, 0x12),
            (0x008003, 0x7E),
            (0x7E1236, 0x07),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x0C);
        assert_eq!(cpu.registers.pc, 0x8004);
    }

    #[test]
    fn adc_stack_relative_adds_stack_page_operand() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01F0;
        cpu.registers.a = 0x0005;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x63), (0x008001, 0x10), (0x000200, 0x07)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x0C);
    }

    #[test]
    fn adc_stack_relative_indirect_y_adds_indexed_indirect_operand() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01F0;
        cpu.registers.y = 0x0002;
        cpu.registers.dbr = 0x7E;
        cpu.registers.a = 0x0005;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x73),
            (0x008001, 0x10),
            (0x000200, 0x34),
            (0x000201, 0x12),
            (0x7E1236, 0x07),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x0C);
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

    #[test]
    fn lsr_a_shifts_right_and_sets_carry() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x0003;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x4A)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x01);
        assert_ne!(cpu.registers.p & 0x01, 0);
        assert_eq!(cpu.registers.pc, 0x8001);
    }

    #[test]
    fn ror_a_rotates_in_carry() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x0002;
        cpu.registers.p = 0x21;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x6A)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x81);
        assert_eq!(cpu.registers.p & 0x01, 0);
        assert_eq!(cpu.registers.pc, 0x8001);
    }

    #[test]
    fn dec_a_decrements_accumulator() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0x0001;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x008000, 0x3A)]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.a & 0x00FF, 0x00);
        assert_ne!(cpu.registers.p & 0x02, 0);
        assert_eq!(cpu.registers.pc, 0x8001);
    }

    #[test]
    fn nmi_is_serviced_before_next_opcode_fetch() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.p = 0x20;
        cpu.registers.emulation = false;

        let mut bus = TestBus::with_bytes(&[(0x00FFEA, 0x34), (0x00FFEB, 0x12)]);
        bus.pending_nmi = true;

        let trace = cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x1234);
        assert_eq!(cpu.registers.pbr, 0x00);
        assert_ne!(cpu.registers.p & 0x04, 0);
        assert!(trace.iter().any(|event| event.address == 0x00FFEA));
        assert!(trace.iter().any(|event| event.address == 0x00FFEB));
    }

    #[test]
    fn native_nmi_preserves_index_width_through_stack_and_rti() {
        let mut cpu = Cpu65816::default();
        cpu.registers.emulation = false;
        cpu.registers.pbr = 0x80;
        cpu.registers.pc = 0x9456;
        cpu.registers.p = 0xB0; // M=1, X=1: essential for 8-bit PLY.
        cpu.registers.s = 0x01FF;
        let mut bus = TestBus::with_bytes(&[
            (0x00FFEA, 0x00),
            (0x00FFEB, 0x81),
            (0x008100, 0x40), // RTI
        ]);
        bus.pending_nmi = true;

        cpu.step_with_bus(&mut bus).unwrap();
        assert_eq!(bus.read(0x0001FF), 0x80); // PBR
        assert_eq!(bus.read(0x0001FE), 0x94); // PC high
        assert_eq!(bus.read(0x0001FD), 0x56); // PC low
        assert_eq!(bus.read(0x0001FC), 0xB0); // Native X bit preserved.
        assert_eq!(cpu.registers.s, 0x01FB);
        assert_eq!(cpu.registers.pc, 0x8100);
        assert_eq!(cpu.registers.pbr, 0);

        cpu.step_with_bus(&mut bus).unwrap();
        assert_eq!(cpu.registers.p, 0xB0);
        assert_eq!(cpu.registers.s, 0x01FF);
        assert_eq!(cpu.registers.pc, 0x9456);
        assert_eq!(cpu.registers.pbr, 0x80);
    }

    #[test]
    fn native_irq_preserves_index_width_through_stack_and_rti() {
        let mut cpu = Cpu65816::default();
        cpu.registers.emulation = false;
        cpu.registers.pc = 0x8123;
        cpu.registers.p = 0x30; // IRQ enabled, 8-bit A and XY.
        cpu.registers.s = 0x01FF;
        let mut bus = TestBus::with_bytes(&[
            (0x00FFEE, 0x00),
            (0x00FFEF, 0x82),
            (0x008200, 0x40), // RTI
        ]);
        bus.pending_irq = true;

        cpu.step_with_bus(&mut bus).unwrap();
        assert_eq!(bus.read(0x0001FC), 0x30);
        assert_eq!(cpu.registers.pc, 0x8200);

        cpu.step_with_bus(&mut bus).unwrap();
        assert_eq!(cpu.registers.p, 0x30);
        assert_eq!(cpu.registers.pc, 0x8123);
        assert_eq!(cpu.registers.s, 0x01FF);
    }

    #[test]
    fn emulation_mode_nmi_pushes_break_flag_clear() {
        let mut cpu = Cpu65816::default();
        cpu.reset();
        cpu.registers.pc = 0x8000;
        cpu.registers.p = 0x30;
        cpu.registers.s = 0x01FF;
        let mut bus = TestBus::with_bytes(&[(0x00FFFA, 0x00), (0x00FFFB, 0x81), (0x008100, 0x40)]);
        bus.pending_nmi = true;

        cpu.step_with_bus(&mut bus).unwrap();
        assert_eq!(bus.read(0x0001FD), 0x20);
        assert_eq!(cpu.registers.s, 0x01FC);
        cpu.step_with_bus(&mut bus).unwrap();
        assert_eq!(cpu.registers.p, 0x30);
        assert_eq!(cpu.registers.pc, 0x8000);
        assert_eq!(cpu.registers.s, 0x01FF);
    }

    #[test]
    fn rti_restores_status_and_program_counter() {
        let mut cpu = Cpu65816::default();
        cpu.reset(); // RTI here is exercised in 65816 emulation mode.
        cpu.registers.pc = 0x1234;
        cpu.registers.p = 0x04;
        cpu.registers.s = 0x01FB;

        let mut bus = TestBus::with_bytes(&[
            (0x001234, 0x40),
            (0x001235, 0x00),
            (0x0001FC, 0x22),
            (0x0001FD, 0x78),
            (0x0001FE, 0x56),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.p, 0x32);
        assert_eq!(cpu.registers.pc, 0x5678);
        assert_eq!(cpu.registers.s, 0x01FE);
    }

    #[test]
    fn phd_and_pld_roundtrip_full_direct_page_in_eight_bit_accumulator_mode() {
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x8000;
        cpu.registers.p = 0x20; // M=1; PHD/PLD still operate on sixteen bits.
        cpu.registers.emulation = false;
        cpu.registers.s = 0x01FF;
        cpu.registers.d = 0xBEEF;
        let mut bus = TestBus::with_bytes(&[(0x008000, 0x0B), (0x008001, 0x2B)]);

        cpu.step_with_bus(&mut bus).unwrap();
        assert_eq!(bus.read(0x0001FF), 0xBE);
        assert_eq!(bus.read(0x0001FE), 0xEF);
        assert_eq!(cpu.registers.s, 0x01FD);
        assert_eq!(cpu.registers.pc, 0x8001);

        cpu.registers.d = 0;
        cpu.step_with_bus(&mut bus).unwrap();
        assert_eq!(cpu.registers.d, 0xBEEF);
        assert_eq!(cpu.registers.s, 0x01FF);
        assert_eq!(cpu.registers.pc, 0x8002);
        assert_ne!(cpu.registers.p & 0x80, 0);
        assert_eq!(cpu.registers.p & 0x02, 0);
    }

    #[test]
    fn emulation_mode_stack_push_and_pull_wrap_within_page_one() {
        let mut cpu = Cpu65816::default();
        cpu.reset();
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x0100;
        let mut bus = TestBus::with_bytes(&[(0x008000, 0x08), (0x008001, 0x28)]);

        cpu.step_with_bus(&mut bus).unwrap(); // PHP
        assert_eq!(cpu.registers.s, 0x01FF);
        assert_eq!(bus.read(0x000100), 0x34);
        cpu.step_with_bus(&mut bus).unwrap(); // PLP
        assert_eq!(cpu.registers.s, 0x0100);
        assert_eq!(cpu.registers.pc, 0x8002);
    }

    #[test]
    fn emulation_mode_stack_transfers_keep_page_one_high_byte() {
        let mut cpu = Cpu65816::default();
        cpu.reset();
        cpu.registers.pc = 0x8000;
        cpu.registers.a = 0xABCD;
        cpu.registers.x = 0x007E;
        let mut bus = TestBus::with_bytes(&[(0x008000, 0x1B), (0x008001, 0x9A)]);

        cpu.step_with_bus(&mut bus).unwrap(); // TCS
        assert_eq!(cpu.registers.s, 0x01CD);
        cpu.step_with_bus(&mut bus).unwrap(); // TXS
        assert_eq!(cpu.registers.s, 0x017E);
    }

    #[test]
    fn emulation_mode_nmi_from_nonzero_program_bank_enters_bank_zero() {
        let mut cpu = Cpu65816::default();
        cpu.reset();
        cpu.registers.pbr = 0x82;
        cpu.registers.pc = 0x9456;
        cpu.registers.s = 0x01FF;
        let mut bus = TestBus::with_bytes(&[(0x00FFFA, 0xCD), (0x00FFFB, 0xAB)]);
        bus.pending_nmi = true;

        cpu.step_with_bus(&mut bus).unwrap();
        assert_eq!(cpu.registers.pc, 0xABCD);
        assert_eq!(cpu.registers.pbr, 0x00);
        assert_eq!(cpu.registers.s, 0x01FC);
        assert_eq!(bus.read(0x0001FF), 0x94);
        assert_eq!(bus.read(0x0001FE), 0x56);
        assert_ne!(cpu.registers.p & 0x04, 0);
    }

    #[test]
    fn eight_bit_index_dispatcher_can_unpack_long_call_return_pointer() {
        // Synthetic version of a common 65816 jump-table convention:
        // a long call enters with X=1, PLY removes the *low* return byte,
        // then REP #$30 and PLA consume its remaining high byte and bank.
        // PLY must not consume both return bytes in eight-bit index mode.
        let mut cpu = Cpu65816::default();
        cpu.registers.pc = 0x9325;
        cpu.registers.p = 0x30; // Native M=1, X=1.
        cpu.registers.emulation = false;
        cpu.registers.s = 0x01FD; // Outer JSR return is already on stack.
        cpu.registers.a = 0x0001; // Choose table entry one.
        cpu.registers.y = 0x0004; // Preserve caller Y.

        let mut bus = TestBus::with_bytes(&[
            (0x009325, 0x22),
            (0x009326, 0xDF),
            (0x009327, 0x86),
            (0x009328, 0x00),
            (0x009329, 0x00),
            (0x00932A, 0x94),
            (0x00932B, 0x20),
            (0x00932C, 0x94),
            (0x0086DF, 0x84),
            (0x0086E0, 0x03),
            (0x0086E1, 0x7A),
            (0x0086E2, 0x84),
            (0x0086E3, 0x00),
            (0x0086E4, 0xC2),
            (0x0086E5, 0x30),
            (0x0086E6, 0x29),
            (0x0086E7, 0xFF),
            (0x0086E8, 0x00),
            (0x0086E9, 0x0A),
            (0x0086EA, 0xA8),
            (0x0086EB, 0x68),
            (0x0086EC, 0x85),
            (0x0086ED, 0x01),
            (0x0086EE, 0xC8),
            (0x0086EF, 0xB7),
            (0x0086F0, 0x00),
            (0x0086F1, 0x85),
            (0x0086F2, 0x00),
            (0x0086F3, 0xE2),
            (0x0086F4, 0x30),
            (0x0086F5, 0xA4),
            (0x0086F6, 0x03),
            (0x0086F7, 0xDC),
            (0x0086F8, 0x00),
            (0x0086F9, 0x00),
            (0x0001FE, 0x74),
            (0x0001FF, 0x80),
        ]);
        for _ in 0..16 {
            cpu.step_with_bus(&mut bus).unwrap();
        }
        assert_eq!(cpu.registers.pbr, 0);
        assert_eq!(cpu.registers.pc, 0x9420);
        assert_eq!(cpu.registers.s, 0x01FD);
        assert_eq!(cpu.registers.p & 0x30, 0x30);
        assert_eq!(cpu.registers.y, 4);
        assert_eq!(bus.read(0x000000), 0x20);
        assert_eq!(bus.read(0x000001), 0x94);
        assert_eq!(bus.read(0x000002), 0x00);
    }

    #[test]
    fn cop_uses_cop_vector() {
        let mut cpu = Cpu65816::default();
        cpu.reset(); // The $FFF4 COP vector is the emulation-mode vector.
        cpu.registers.pc = 0x8000;
        cpu.registers.s = 0x01FF;

        let mut bus = TestBus::with_bytes(&[
            (0x008000, 0x02),
            (0x008001, 0x99),
            (0x00FFF4, 0x34),
            (0x00FFF5, 0x12),
        ]);

        cpu.step_with_bus(&mut bus).unwrap();

        assert_eq!(cpu.registers.pc, 0x1234);
        assert_eq!(bus.read(0x0001FF), 0x80);
        assert_eq!(bus.read(0x0001FE), 0x02);
    }
}
