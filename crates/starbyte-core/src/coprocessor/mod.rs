//! Cartridge coprocessor metadata and runtime models.

mod cx4;
mod dsp;
mod sa1;
mod secondary;
mod superfx;

use serde::{Deserialize, Serialize};

use crate::bus::Address;
use crate::cartridge::{Cartridge, CartridgeHeader, Mapper};
use crate::ppu::FrameBuffer;

pub use self::cx4::Cx4Coprocessor;
pub use self::dsp::{DspCoprocessor, DspVariant};
pub use self::sa1::Sa1Coprocessor;
pub use self::secondary::{Obc1Coprocessor, SRtcCoprocessor, Sdd1Coprocessor};
pub use self::superfx::{SuperFxCoprocessor, SuperFxMap};

/// Coarse coprocessor family derived from the cartridge header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoprocessorKind {
    /// NEC uPD77C25-based DSP family (`DSP-1/2/3/4`).
    Dsp,
    /// GSU / SuperFX family.
    SuperFx,
    /// Capcom Cx4 math coprocessor.
    Cx4,
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

        let title_upper = header.title.to_ascii_uppercase();
        if chipset == 0xF3
            || title_upper.contains(" CX4")
            || title_upper.contains("MEGA MAN X2")
            || title_upper.contains("MEGA MAN X3")
            || title_upper.contains("ROCKMAN X2")
            || title_upper.contains("ROCKMAN X3")
        {
            return Some(Self::Cx4);
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
            Self::Cx4 => f.write_str("Cx4"),
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
    /// Bounded SA-1 subsystem model.
    Sa1(Sa1Coprocessor),
    /// Bounded Cx4 math-command model.
    Cx4(Cx4Coprocessor),
    /// Bounded S-DD1 decompression model.
    Sdd1(Sdd1Coprocessor),
    /// OBC1 object attribute controller model.
    Obc1(Obc1Coprocessor),
    /// Deterministic S-RTC model.
    SRtc(SRtcCoprocessor),
}

impl Coprocessor {
    /// Build the runtime coprocessor state for a cartridge, if any.
    #[must_use]
    pub fn for_cartridge(cartridge: &Cartridge) -> Option<Self> {
        match CoprocessorKind::detect(cartridge.header())? {
            CoprocessorKind::Dsp => Some(Self::Dsp(DspCoprocessor::new(cartridge.header()))),
            CoprocessorKind::SuperFx => Some(Self::SuperFx(SuperFxCoprocessor::new(cartridge))),
            CoprocessorKind::Sa1 => Some(Self::Sa1(Sa1Coprocessor::new(cartridge))),
            CoprocessorKind::Cx4 => Some(Self::Cx4(Cx4Coprocessor::new(cartridge.header()))),
            CoprocessorKind::Sdd1 => Some(Self::Sdd1(Sdd1Coprocessor::new())),
            CoprocessorKind::Obc1 => Some(Self::Obc1(Obc1Coprocessor::new())),
            CoprocessorKind::SRtc => Some(Self::SRtc(SRtcCoprocessor::new())),
            CoprocessorKind::Custom(_) => None,
        }
    }

    /// Return the coprocessor family.
    #[must_use]
    pub const fn kind(&self) -> CoprocessorKind {
        match self {
            Self::Dsp(_) => CoprocessorKind::Dsp,
            Self::SuperFx(_) => CoprocessorKind::SuperFx,
            Self::Sa1(_) => CoprocessorKind::Sa1,
            Self::Cx4(_) => CoprocessorKind::Cx4,
            Self::Sdd1(_) => CoprocessorKind::Sdd1,
            Self::Obc1(_) => CoprocessorKind::Obc1,
            Self::SRtc(_) => CoprocessorKind::SRtc,
        }
    }

    /// Reset transient runtime state.
    pub fn reset(&mut self) {
        match self {
            Self::Dsp(dsp) => dsp.reset(),
            Self::SuperFx(superfx) => superfx.reset(),
            Self::Sa1(sa1) => sa1.reset(),
            Self::Cx4(cx4) => cx4.reset(),
            Self::Sdd1(sdd1) => sdd1.reset(),
            Self::Obc1(obc1) => obc1.reset(),
            Self::SRtc(srtc) => srtc.reset(),
        }
    }

    /// Attempt to service a CPU read from a coprocessor-mapped address.
    #[must_use]
    pub fn read(&mut self, mapper: Mapper, address: Address) -> Option<u8> {
        match self {
            Self::Dsp(dsp) => dsp.read(mapper, address),
            Self::SuperFx(superfx) => superfx.read(mapper, address),
            Self::Sa1(sa1) => sa1.read(mapper, address),
            Self::Cx4(cx4) => cx4.read(mapper, address),
            Self::Sdd1(sdd1) => sdd1.read(mapper, address),
            Self::Obc1(obc1) => obc1.read(mapper, address),
            Self::SRtc(srtc) => srtc.read(mapper, address),
        }
    }

    /// Attempt to service a CPU write to a coprocessor-mapped address.
    pub fn write(&mut self, mapper: Mapper, address: Address, value: u8) -> bool {
        match self {
            Self::Dsp(dsp) => dsp.write(mapper, address, value),
            Self::SuperFx(superfx) => superfx.write(mapper, address, value),
            Self::Sa1(sa1) => sa1.write(mapper, address, value),
            Self::Cx4(cx4) => cx4.write(mapper, address, value),
            Self::Sdd1(sdd1) => sdd1.write(mapper, address, value),
            Self::Obc1(obc1) => obc1.write(mapper, address, value),
            Self::SRtc(srtc) => srtc.write(mapper, address, value),
        }
    }

    /// Advance coprocessor-internal timing, if the active chip model uses it.
    pub fn step_master_cycles(&mut self, clocks: u64) {
        match self {
            Self::Dsp(dsp) => dsp.step_master_cycles(clocks),
            Self::SuperFx(superfx) => superfx.step(clocks),
            Self::Sa1(sa1) => sa1.step(clocks),
            Self::Cx4(cx4) => cx4.step(clocks),
            Self::Sdd1(sdd1) => sdd1.step(clocks),
            Self::Obc1(_) | Self::SRtc(_) => {}
        }
    }

    /// Render any coprocessor-owned overlay content on top of the main framebuffer.
    pub fn render_overlay(&self, framebuffer: &mut FrameBuffer) {
        match self {
            Self::Dsp(_)
            | Self::Sa1(_)
            | Self::Cx4(_)
            | Self::Sdd1(_)
            | Self::Obc1(_)
            | Self::SRtc(_) => {}
            Self::SuperFx(superfx) => superfx.render_overlay(framebuffer),
        }
    }
}
