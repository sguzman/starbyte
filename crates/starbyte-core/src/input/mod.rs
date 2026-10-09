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
    /// Pack SNES auto-joypad bits for $4218/$4219 (BYsS UDLR AXlr 0000).
    /// The manual $4016 serial reader emits these bits most-significant first.
    #[must_use]
    pub const fn to_bits(self) -> u16 {
        ((self.b as u16) << 15)
            | ((self.y as u16) << 14)
            | ((self.select as u16) << 13)
            | ((self.start as u16) << 12)
            | ((self.up as u16) << 11)
            | ((self.down as u16) << 10)
            | ((self.left as u16) << 9)
            | ((self.right as u16) << 8)
            | ((self.a as u16) << 7)
            | ((self.x as u16) << 6)
            | ((self.l as u16) << 5)
            | ((self.r as u16) << 4)
    }
}

#[cfg(test)]
mod tests {
    use super::ControllerState;

    #[test]
    fn snes_joypad_word_places_all_twelve_buttons_in_hardware_bits() {
        let cases = [
            (ControllerState { b: true, ..ControllerState::default() }, 0x8000),
            (ControllerState { y: true, ..ControllerState::default() }, 0x4000),
            (ControllerState { select: true, ..ControllerState::default() }, 0x2000),
            (ControllerState { start: true, ..ControllerState::default() }, 0x1000),
            (ControllerState { up: true, ..ControllerState::default() }, 0x0800),
            (ControllerState { down: true, ..ControllerState::default() }, 0x0400),
            (ControllerState { left: true, ..ControllerState::default() }, 0x0200),
            (ControllerState { right: true, ..ControllerState::default() }, 0x0100),
            (ControllerState { a: true, ..ControllerState::default() }, 0x0080),
            (ControllerState { x: true, ..ControllerState::default() }, 0x0040),
            (ControllerState { l: true, ..ControllerState::default() }, 0x0020),
            (ControllerState { r: true, ..ControllerState::default() }, 0x0010),
        ];
        for (state, expected) in cases {
            assert_eq!(state.to_bits(), expected);
        }
        assert_eq!(ControllerState::default().to_bits(), 0);
    }
}
