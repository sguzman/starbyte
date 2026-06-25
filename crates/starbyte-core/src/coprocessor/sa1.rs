use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::bus::Address;
use crate::cartridge::{Cartridge, Mapper};

const SA1_STATUS_RUNNING: u8 = 0x01;
const SA1_STATUS_IRQ_PENDING: u8 = 0x40;
const SA1_STATUS_BOOT_COMPLETE: u8 = 0x80;

/// Bounded SA-1 subsystem with MMIO, shared RAM, and deterministic boot signaling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sa1Coprocessor {
    control: u8,
    status: u8,
    reset_vector: u16,
    irq_vector: u16,
    timer_reload: u16,
    timer_counter: u16,
    irq_enable: u8,
    irq_pending: u8,
    cpu_message: u8,
    sa1_message: u8,
    boot_token: u8,
    pc: u16,
    accumulated_cycles: u64,
    iram: Vec<u8>,
    bwram: Vec<u8>,
}

impl Sa1Coprocessor {
    pub(crate) fn new(cartridge: &Cartridge) -> Self {
        let reset_vector = cartridge.reset_vector().unwrap_or(0x8000);
        trace!(
            target: "starbyte_core::coprocessor::sa1",
            title = %cartridge.header().title,
            reset_vector,
            "initializing SA-1 coprocessor"
        );
        Self {
            control: 0,
            status: 0,
            reset_vector,
            irq_vector: reset_vector,
            timer_reload: 32,
            timer_counter: 32,
            irq_enable: 0,
            irq_pending: 0,
            cpu_message: 0,
            sa1_message: 0,
            boot_token: 0,
            pc: reset_vector,
            accumulated_cycles: 0,
            iram: vec![0; 0x800],
            bwram: vec![0; 0x2000],
        }
    }

    pub(crate) fn reset(&mut self) {
        trace!(target: "starbyte_core::coprocessor::sa1", "resetting SA-1 coprocessor");
        self.control = 0;
        self.status = 0;
        self.timer_counter = self.timer_reload.max(1);
        self.irq_pending = 0;
        self.cpu_message = 0;
        self.sa1_message = 0;
        self.boot_token = 0;
        self.pc = self.reset_vector;
        self.accumulated_cycles = 0;
        self.iram.fill(0);
        self.bwram.fill(0);
    }

    pub(crate) fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        match decode_sa1(mapper, address)? {
            Sa1Target::Mmio(offset) => Some(self.read_mmio(offset)),
            Sa1Target::Iram(index) => Some(self.iram[index]),
            Sa1Target::Bwram(index) => Some(self.bwram[index]),
        }
    }

    pub(crate) fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        let Some(target) = decode_sa1(mapper, address) else {
            return false;
        };
        match target {
            Sa1Target::Mmio(offset) => self.write_mmio(offset, value),
            Sa1Target::Iram(index) => self.iram[index] = value,
            Sa1Target::Bwram(index) => self.bwram[index] = value,
        }
        true
    }

    pub(crate) fn step(&mut self, clocks: u64) {
        if self.status & SA1_STATUS_RUNNING == 0 {
            return;
        }

        self.accumulated_cycles = self.accumulated_cycles.saturating_add(clocks);
        let countdown = u64::from(self.timer_counter.max(1));
        if self.accumulated_cycles < countdown {
            return;
        }

        self.accumulated_cycles -= countdown;
        self.status &= !SA1_STATUS_RUNNING;
        self.status |= SA1_STATUS_BOOT_COMPLETE;
        self.pc = self.reset_vector;
        self.sa1_message = self.cpu_message.wrapping_add(0x33);
        self.boot_token = self.cpu_message ^ 0xA5 ^ (self.reset_vector as u8);
        self.timer_counter = self.timer_reload.max(1);
        if self.irq_enable & 0x01 != 0 {
            self.irq_pending |= 0x01;
            self.status |= SA1_STATUS_IRQ_PENDING;
        }
    }

    fn read_mmio(&self, offset: u16) -> u8 {
        match offset {
            0x00 => self.control,
            0x01 => self.status,
            0x02 => self.reset_vector as u8,
            0x03 => (self.reset_vector >> 8) as u8,
            0x04 => self.irq_vector as u8,
            0x05 => (self.irq_vector >> 8) as u8,
            0x06 => self.timer_reload as u8,
            0x07 => (self.timer_reload >> 8) as u8,
            0x08 => self.cpu_message,
            0x09 => self.sa1_message,
            0x0A => self.irq_enable,
            0x0B => self.irq_pending,
            0x0C => self.pc as u8,
            0x0D => (self.pc >> 8) as u8,
            0x0E => self.boot_token,
            0x0F => self.timer_counter as u8,
            _ => 0,
        }
    }

    fn write_mmio(&mut self, offset: u16, value: u8) {
        match offset {
            0x00 => {
                self.control = value;
                if value & 0x01 != 0 {
                    self.status = 0;
                    self.irq_pending = 0;
                    self.pc = self.reset_vector;
                    self.timer_counter = self.timer_reload.max(1);
                    self.boot_token = 0;
                }
                if value & 0x80 != 0 {
                    self.status |= SA1_STATUS_RUNNING;
                }
            }
            0x02 => self.reset_vector = (self.reset_vector & 0xFF00) | u16::from(value),
            0x03 => self.reset_vector = (self.reset_vector & 0x00FF) | (u16::from(value) << 8),
            0x04 => self.irq_vector = (self.irq_vector & 0xFF00) | u16::from(value),
            0x05 => self.irq_vector = (self.irq_vector & 0x00FF) | (u16::from(value) << 8),
            0x06 => {
                self.timer_reload = (self.timer_reload & 0xFF00) | u16::from(value);
                self.timer_counter = self.timer_reload.max(1);
            }
            0x07 => {
                self.timer_reload = (self.timer_reload & 0x00FF) | (u16::from(value) << 8);
                self.timer_counter = self.timer_reload.max(1);
            }
            0x08 => self.cpu_message = value,
            0x0A => self.irq_enable = value,
            0x0B => {
                self.irq_pending &= !value;
                if self.irq_pending == 0 {
                    self.status &= !SA1_STATUS_IRQ_PENDING;
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sa1Target {
    Mmio(u16),
    Iram(usize),
    Bwram(usize),
}

fn decode_sa1(mapper: Mapper, address: Address) -> Option<Sa1Target> {
    if mapper != Mapper::LoRom {
        return None;
    }

    let bank = ((address >> 16) & 0xFF) as u8 & 0x7F;
    let offset = (address & 0xFFFF) as u16;

    if (0x00..=0x3F).contains(&bank) {
        return match offset {
            0x2200..=0x23FF => Some(Sa1Target::Mmio(offset - 0x2200)),
            0x3000..=0x37FF => Some(Sa1Target::Iram(usize::from(offset - 0x3000))),
            _ => None,
        };
    }

    if (0x40..=0x43).contains(&bank) {
        return Some(Sa1Target::Bwram(
            ((usize::from(bank - 0x40) * 0x10000) + usize::from(offset)) % 0x2000,
        ));
    }

    None
}

#[cfg(test)]
mod tests {
    use crate::cartridge::{Cartridge, Mapper};

    use super::Sa1Coprocessor;

    fn cart() -> Cartridge {
        let mut rom = vec![0_u8; 0x10000];
        let base = 0x7FC0;
        rom[base..base + 21].copy_from_slice(b"STARBYTE SA-1 TEST   ");
        rom[base + 0x15] = 0x20;
        rom[base + 0x16] = 0x34;
        rom[base + 0x17] = 0x09;
        rom[base + 0x18] = 0x01;
        rom[base + 0x19] = 0x01;
        rom[base + 0x1C] = 0x00;
        rom[base + 0x1D] = 0xFF;
        rom[base + 0x1E] = 0xFF;
        rom[base + 0x1F] = 0x00;
        rom[0x7FFC] = 0x34;
        rom[0x7FFD] = 0x12;
        Cartridge::from_bytes(rom, None).unwrap()
    }

    #[test]
    fn mmio_and_shared_memory_roundtrip() {
        let mut sa1 = Sa1Coprocessor::new(&cart());
        assert!(sa1.write(Mapper::LoRom, 0x003000, 0x44));
        assert!(sa1.write(Mapper::LoRom, 0x406000, 0x99));
        assert_eq!(sa1.read(Mapper::LoRom, 0x003000), Some(0x44));
        assert_eq!(sa1.read(Mapper::LoRom, 0x406000), Some(0x99));
    }

    #[test]
    fn stepping_completes_boot_and_raises_irq() {
        let mut sa1 = Sa1Coprocessor::new(&cart());
        sa1.write(Mapper::LoRom, 0x002202, 0x78);
        sa1.write(Mapper::LoRom, 0x002203, 0x56);
        sa1.write(Mapper::LoRom, 0x002206, 0x08);
        sa1.write(Mapper::LoRom, 0x002207, 0x00);
        sa1.write(Mapper::LoRom, 0x002208, 0x55);
        sa1.write(Mapper::LoRom, 0x00220A, 0x01);
        sa1.write(Mapper::LoRom, 0x002200, 0x80);
        sa1.step(8);

        assert_eq!(sa1.read(Mapper::LoRom, 0x002201), Some(0xC0));
        assert_eq!(sa1.read(Mapper::LoRom, 0x002209), Some(0x88));
        assert_eq!(sa1.read(Mapper::LoRom, 0x00220C), Some(0x78));
        assert_eq!(sa1.read(Mapper::LoRom, 0x00220D), Some(0x56));
    }
}
