//! Core bus primitives.

use serde::{Deserialize, Serialize};

/// Canonical SNES address type.
pub type Address = u32;

/// Memory access direction for traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccessKind {
    /// Read access.
    Read,
    /// Write access.
    Write,
    /// Idle or wait cycle with no meaningful bus transfer.
    Wait,
}

/// A single bus event captured for correctness work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusEvent {
    /// Absolute address.
    pub address: Address,
    /// Value observed on the bus.
    pub value: u8,
    /// Read or write.
    pub access: AccessKind,
    /// Master clock tick associated with the event.
    pub cycle: u64,
}

/// Minimal bus interface used by the bootstrap emulator skeleton.
pub trait Bus {
    /// Read one byte.
    fn read(&mut self, address: Address) -> u8;

    /// Write one byte.
    fn write(&mut self, address: Address, value: u8);
}
