//! Cartridge coprocessor metadata and runtime models.

mod dsp;
mod superfx;

use serde::{Deserialize, Serialize};

use crate::bus::Address;
use crate::cartridge::{Cartridge, CartridgeHeader, Mapper};
use crate::ppu::FrameBuffer;

pub use self::dsp::{DspCoprocessor, DspVariant};
pub use self::superfx::{SuperFxCoprocessor, SuperFxMap};

/// Coarse coprocessor family derived from the cartridge header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoprocessorKind {
    /// NEC uPD77C25-based DSP family (`DSP-1/2/3/4`).
    Dsp,
    /// GSU / SuperFX family.
    SuperFx,
    /// OBC1 object controller.
    Obc1,
    /// SA-1 companion CPU.
    Sa1,
    /// S-DD1 decompression chip.
    Sdd1,
    /// S-RTC real-time clock.
    SRtc,
    /// Custom coprocessor identified by the expanded header.
    Custom(u8),
}

impl CoprocessorKind {
    /// Detect coprocessor class from the cartridge header chipset field.
    #[must_use]
    pub fn detect(header: &CartridgeHeader) -> Option<Self> {
        let chipset = header.rom_type;
        if chipset < 0x03 {
            return None;
        }

        match (chipset >> 4) & 0x0F {
            0x0 => Some(Self::Dsp),
            0x1 => Some(Self::SuperFx),
            0x2 => Some(Self::Obc1),
            0x3 => Some(Self::Sa1),
            0x4 => Some(Self::Sdd1),
            0x5 => Some(Self::SRtc),
            0xF => Some(Self::Custom(0)),
            _ => None,
        }
    }
}

impl std::fmt::Display for CoprocessorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dsp => f.write_str("DSP"),
            Self::SuperFx => f.write_str("SuperFX"),
            Self::Obc1 => f.write_str("OBC1"),
            Self::Sa1 => f.write_str("SA-1"),
            Self::Sdd1 => f.write_str("S-DD1"),
            Self::SRtc => f.write_str("S-RTC"),
            Self::Custom(value) => write!(f, "Custom(0x{value:02X})"),
        }
    }
}

/// Runtime coprocessor slot attached to the system bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Coprocessor {
    /// DSP-family runtime model.
    Dsp(DspCoprocessor),
    /// Bounded SuperFX register and execution model.
    SuperFx(SuperFxCoprocessor),
}

impl Coprocessor {
    /// Build the runtime coprocessor state for a cartridge, if any.
    #[must_use]
    pub fn for_cartridge(cartridge: &Cartridge) -> Option<Self> {
        match CoprocessorKind::detect(cartridge.header())? {
            CoprocessorKind::Dsp => Some(Self::Dsp(DspCoprocessor::new(cartridge.header()))),
            CoprocessorKind::SuperFx => {
                Some(Self::SuperFx(SuperFxCoprocessor::new(cartridge)))
            }
            _ => None,
        }
    }

    /// Return the coprocessor family.
    #[must_use]
    pub const fn kind(&self) -> CoprocessorKind {
        match self {
            Self::Dsp(_) => CoprocessorKind::Dsp,
            Self::SuperFx(_) => CoprocessorKind::SuperFx,
        }
    }

    /// Reset transient runtime state.
    pub fn reset(&mut self) {
        match self {
            Self::Dsp(dsp) => dsp.reset(),
            Self::SuperFx(superfx) => superfx.reset(),
        }
    }

    /// Attempt to service a CPU read from a coprocessor-mapped address.
    #[must_use]
    pub fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        match self {
            Self::Dsp(dsp) => dsp.read(mapper, address),
            Self::SuperFx(superfx) => superfx.read(mapper, address),
        }
    }

    /// Attempt to service a CPU write to a coprocessor-mapped address.
    pub fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        match self {
            Self::Dsp(dsp) => dsp.write(mapper, address, value),
            Self::SuperFx(superfx) => superfx.write(mapper, address, value),
        }
    }

    /// Advance coprocessor-internal timing, if the active chip model uses it.
    pub fn step_master_cycles(&mut self, clocks: u64) {
        match self {
            Self::Dsp(dsp) => dsp.step_master_cycles(clocks),
            Self::SuperFx(superfx) => superfx.step(clocks),
        }
    }

    /// Render any coprocessor-owned overlay content on top of the main framebuffer.
    pub fn render_overlay(&self, framebuffer: &mut FrameBuffer) {
        match self {
            Self::Dsp(_) => {}
            Self::SuperFx(superfx) => superfx.render_overlay(framebuffer),
        }
    }
}
