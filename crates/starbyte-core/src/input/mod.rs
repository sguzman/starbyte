//! Input types shared by frontends.

use serde::{Deserialize, Serialize};

/// SNES controller state snapshot.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerState {
    /// B button.
    pub b: bool,
    /// Y button.
    pub y: bool,
    /// Select button.
    pub select: bool,
    /// Start button.
    pub start: bool,
    /// Up direction.
    pub up: bool,
    /// Down direction.
    pub down: bool,
    /// Left direction.
    pub left: bool,
    /// Right direction.
    pub right: bool,
    /// A button.
    pub a: bool,
    /// X button.
    pub x: bool,
    /// L shoulder.
    pub l: bool,
    /// R shoulder.
    pub r: bool,
}

impl ControllerState {
    /// Encode the controller into the standard SNES serial bit order.
    #[must_use]
    pub const fn to_bits(self) -> u16 {
        (self.b as u16)
            | ((self.y as u16) << 1)
            | ((self.select as u16) << 2)
            | ((self.start as u16) << 3)
            | ((self.up as u16) << 4)
            | ((self.down as u16) << 5)
            | ((self.left as u16) << 6)
            | ((self.right as u16) << 7)
            | ((self.a as u16) << 8)
            | ((self.x as u16) << 9)
            | ((self.l as u16) << 10)
            | ((self.r as u16) << 11)
    }
}
