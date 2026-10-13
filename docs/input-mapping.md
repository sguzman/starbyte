# Native gamepad input mapping

Starbyte accepts generic USB gamepads through **gilrs**, including devices that identify themselves as a Nintendo Pro Controller. The native frontend does not require a browser gamepad tester, keyboard emulation, or Bluetooth pairing when the controller already works over USB.

## Map the SNES controls

Open **Settings → Input** and select **Gamepad** or **Automatic (Keyboard + Gamepad)**. The **Connected Gamepads** section shows controller names and live physical button labels and axis values. Under **Gamepad Bindings**, choose **Rebind Pad** next to one of the twelve SNES actions (Up, Down, Left, Right, Start, Select, A, B, X, Y, L, R), then press the desired physical button or push a stick direction deliberately. The setting saves as soon as a capture succeeds.

- Ordinary face, shoulder, menu, and D-pad buttons can all be captured as named gilrs button inputs.
- The directional pad can arrive as separate button events **or** D-pad X/Y hat-axis events. Both are supported.
- Left and right analog sticks can also be assigned explicitly as positive/negative X/Y directions. Stick capture requires crossing a stronger threshold to reject small calibration drift.
- The left stick additionally acts as a virtual SNES D-pad by default, with a neutral deadzone. The physical D-pad does not depend on stick emulation.
- A captured button or stick direction is held out of gameplay until released or returned to neutral, so the control used for rebinding does not also trigger an immediate in-game action.
- **Cancel Rebind** aborts capture. **Unbind** removes an individual SNES action's physical gamepad assignment. **Reset Pad Defaults** restores the standard layout without changing keyboard bindings. **Save Input** persists the active device selection.
- The settings page includes a live **Mapped SNES buttons** preview, so you can confirm that pressing a physical control maps to the intended SNES action without loading a commercial ROM.

Bindings are saved in the normal Starbyte user config under `[input.gamepad_bindings]`, not in the ROM or Cheatarium. Rebinding the same physical button to more than one SNES action is currently allowed; such a button can trigger multiple actions, so inspect each mapping if inputs appear coupled.

## Input lifecycle

Short physical button and directional axis taps are buffered until the next emulation sample. Disconnected controllers have their held states and queued taps cleared independently. If Starbyte loses native window focus, gameplay is not sent fresh input and transient queued gamepad events are discarded. Returning to the window permits new input again.

## Troubleshooting an unresponsive game

With a controller connected, Starbyte enables the **Input Diagnostics** overlay in Play View. It shows the raw physical inputs seen by gilrs, the mapped SNES controller buttons, and the current emulator host/latched joypad bitfields. This distinguishes three very different cases: the controller is not delivering an event, a saved mapping does not produce the expected SNES action, or the game is not consuming a correctly latched input. A lost Wayland window focus is called out directly. The input overlay can be toggled in the Play View toolbar.

Asynchronous library refreshes are not permitted to overwrite the live input or Cheatarium config with an old worker snapshot. Gamepad changes persist even when background library and cheat jobs complete. These are tested invariants, not physical controller acceptance results.

## Verification status

CI can test button state merging, direction thresholds, event buffering, configuration persistence, per-device disconnect behavior and rebind-gesture suppression. It **cannot** establish that a particular real controller works under the player's Hyprland session. Native acceptance must still check the reference PXN P50L over USB, actual SNES input during gameplay, start/select/face-button orientation, sustained holds, short presses, simultaneous buttons and focus/reconnect behavior. Track the remaining work in [Issue #1](https://github.com/sguzman/starbyte/issues/1). Do not close the issue based only on these synthetic tests.
