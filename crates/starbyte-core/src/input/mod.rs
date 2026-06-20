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
