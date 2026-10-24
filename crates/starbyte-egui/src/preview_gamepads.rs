//! GUI-side gamepad input for the opt-in worker-backed player.
//! The emulation thread only receives packed SNES button state, never a
//! Gilrs handle or a blocking device operation.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    time::Instant,
};

use gilrs::{Axis, EventType, GamepadId, Gilrs};
use starbyte_core::input::ControllerState;

use crate::{
    app::{discard_gamepad_backlog, input_actions, set_controller_flag},
    session_engine::controller_from_bits,
};

#[derive(Default, Clone, Copy)]
struct Stick {
    x: f32,
    y: f32,
}

impl Stick {
    fn virtual_dpad(self) -> BTreeSet<String> {
        let mut active = BTreeSet::new();
        if self.x < -0.5 {
            active.insert("DPadLeft".to_owned());
        } else if self.x > 0.5 {
            active.insert("DPadRight".to_owned());
        }
        if self.y > 0.5 {
            active.insert("DPadUp".to_owned());
        } else if self.y < -0.5 {
            active.insert("DPadDown".to_owned());
        }
        active
    }
}

#[derive(Default, Clone, Copy)]
struct ControllerAxes {
    left: Stick,
    dpad: Stick,
}

impl ControllerAxes {
    fn virtual_dpad(self) -> BTreeSet<String> {
        self.left
            .virtual_dpad()
            .union(&self.dpad.virtual_dpad())
            .cloned()
            .collect()
    }
}

/// Multiple gamepads are supported; pressed edges survive a short tap between
/// two egui update callbacks. Buttons are combined without picking an
/// arbitrary first controller.
pub struct PreviewGamepads {
    gilrs: Option<Gilrs>,
    held_by_id: HashMap<GamepadId, BTreeSet<String>>,
    virtual_by_id: HashMap<GamepadId, BTreeSet<String>>,
    sticks_by_id: HashMap<GamepadId, ControllerAxes>,
    bindings: BTreeMap<String, String>,
    last_poll: Instant,
}

impl PreviewGamepads {
    pub fn new(bindings: BTreeMap<String, String>) -> Self {
        Self {
            gilrs: Gilrs::new().ok(),
            held_by_id: HashMap::new(),
            virtual_by_id: HashMap::new(),
            sticks_by_id: HashMap::new(),
            bindings,
            last_poll: Instant::now(),
        }
    }

    fn clear_held_controls(&mut self) {
        self.held_by_id.clear();
        self.virtual_by_id.clear();
        self.sticks_by_id.clear();
    }

    pub fn poll(&mut self, focused: bool) -> (ControllerState, ControllerState) {
        let now = Instant::now();
        let discard_backlog = discard_gamepad_backlog(focused, self.last_poll, now);
        if discard_backlog {
            // A compositor may skip egui updates without delivering a blur
            // event. Old held state and queued events must both be discarded:
            // resetting held state alone still replays stale button taps.
            self.clear_held_controls();
        }
        self.last_poll = now;
        let mut newly_pressed = BTreeSet::new();
        if let Some(gilrs) = &mut self.gilrs {
            while let Some(event) = gilrs.next_event() {
                if discard_backlog {
                    continue;
                }
                match event.event {
                    EventType::ButtonPressed(button, _) if focused => {
                        let name = format!("{button:?}");
                        self.held_by_id
                            .entry(event.id)
                            .or_default()
                            .insert(name.clone());
                        if focused {
                            newly_pressed.insert(name);
                        }
                    }
                    EventType::ButtonReleased(button, _) => {
                        self.held_by_id
                            .entry(event.id)
                            .or_default()
                            .remove(&format!("{button:?}"));
                    }
                    EventType::AxisChanged(axis, value, _) if focused => {
                        let axes = self.sticks_by_id.entry(event.id).or_default();
                        match axis {
                            Axis::LeftStickX => axes.left.x = value,
                            Axis::LeftStickY => axes.left.y = value,
                            Axis::DPadX => axes.dpad.x = value,
                            Axis::DPadY => axes.dpad.y = value,
                            _ => continue,
                        }
                        let next = axes.virtual_dpad();
                        let previous = self
                            .virtual_by_id
                            .insert(event.id, next.clone())
                            .unwrap_or_default();
                        if focused {
                            for name in next.difference(&previous) {
                                newly_pressed.insert(name.clone());
                            }
                        }
                    }
                    EventType::Disconnected => {
                        self.held_by_id.remove(&event.id);
                        self.virtual_by_id.remove(&event.id);
                        self.sticks_by_id.remove(&event.id);
                    }
                    _ => {}
                }
            }
        }

        if !focused {
            return (ControllerState::default(), ControllerState::default());
        }
        let held_buttons = self
            .held_by_id
            .values()
            .chain(self.virtual_by_id.values())
            .flat_map(|set| set.iter().cloned())
            .collect::<BTreeSet<_>>();
        let mut held = ControllerState::default();
        let mut edges = ControllerState::default();
        for &action in input_actions() {
            if let Some(binding) = self.bindings.get(action) {
                set_controller_flag(&mut held, action, held_buttons.contains(binding));
                set_controller_flag(&mut edges, action, newly_pressed.contains(binding));
            }
        }
        (held, edges)
    }
}

/// Merge independent keyboard and gamepad inputs in SNES hardware bit order.
pub fn merge_sources(
    keyboard: (ControllerState, ControllerState),
    gamepad: (ControllerState, ControllerState),
) -> (ControllerState, ControllerState) {
    (
        controller_from_bits(keyboard.0.to_bits() | gamepad.0.to_bits()),
        controller_from_bits(keyboard.1.to_bits() | gamepad.1.to_bits()),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn left_stick_virtual_dpad_has_expected_y_orientation() {
        let up = Stick { x: 0.0, y: 0.9 }.virtual_dpad();
        assert!(up.contains("DPadUp"));
        assert!(!up.contains("DPadDown"));
        let down_left = Stick { x: -0.9, y: -0.9 }.virtual_dpad();
        assert!(down_left.contains("DPadLeft"));
        assert!(down_left.contains("DPadDown"));
    }

    #[test]
    fn left_stick_and_dpad_axes_do_not_overwrite_each_other() {
        let axes = ControllerAxes {
            left: Stick { x: -0.95, y: 0.0 },
            dpad: Stick { x: 0.0, y: 0.91 },
        };
        let active = axes.virtual_dpad();
        assert!(active.contains("DPadLeft"));
        assert!(active.contains("DPadUp"));
    }

    #[test]
    fn stale_events_are_not_replayed_after_background_or_long_callback_gap() {
        let now = Instant::now();
        assert!(discard_gamepad_backlog(false, now, now));
        assert!(discard_gamepad_backlog(
            true,
            now,
            now + Duration::from_secs(2)
        ));
        assert!(!discard_gamepad_backlog(
            true,
            now,
            now + Duration::from_millis(16)
        ));
    }

    #[test]
    fn keyboard_and_pad_merge_without_overwriting_a_short_press() {
        let pressed = ControllerState {
            start: true,
            ..ControllerState::default()
        };
        let held = ControllerState {
            a: true,
            ..ControllerState::default()
        };
        let (merged, edges) = merge_sources(
            (ControllerState::default(), pressed),
            (held, ControllerState::default()),
        );
        assert!(merged.a);
        assert!(!merged.start);
        assert!(edges.start);
    }
}
