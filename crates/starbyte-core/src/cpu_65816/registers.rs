//! 65816 register model.

use serde::{Deserialize, Serialize};

/// Minimal 65816 register file representation.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registers {
    /// Accumulator.
    pub a: u16,
    /// Index X.
    pub x: u16,
    /// Index Y.
    pub y: u16,
    /// Stack pointer.
    pub s: u16,
    /// Direct page register.
    pub d: u16,
    /// Program counter.
    pub pc: u16,
    /// Program bank.
    pub pbr: u8,
    /// Data bank.
    pub dbr: u8,
    /// Processor status.
    pub p: u8,
    /// Emulation mode flag.
    pub emulation: bool,
}
