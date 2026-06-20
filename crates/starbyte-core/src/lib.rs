//! Core library for the Starbyte SNES emulator.

pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod cpu_65816;
pub mod dma;
pub mod emulator;
pub mod error;
pub mod frontend;
pub mod input;
pub mod manifest;
pub mod ppu;
pub mod testing;
pub mod timing;

pub use crate::apu::{Apu, ApuStatus, AudioFrame, SPC700_IPL_ROM_LEN};
pub use crate::emulator::{Emulator, EmulatorBuilder};
pub use crate::error::{Error, Result};
