# Controller input, hotplugging and multiplayer mapping

Implements issue #7 in `quasar-input` without adding a mandatory native
dependency or changing the crate's independent, headless use.

## Audit and decisions

Before this change `quasar-input` had keyboard/mouse `Button` sources, named
actions, digital axes/chords and serde bindings. `quasar-window::input` fed winit
events into one `InputState`; there was no controller state, hotplug or player
routing. The existing action model already handles alternate sources and
press/release edges, so the controller layer extends it rather than introducing a
second action system.

Decisions:

- Keep the core pure Rust. `GamepadEvent`, `GamepadHub`, `GamepadSet`,
  `GamepadState`, `PlayerInput`, `GamepadConfig` and all bindings work with no
  native library.
- Add `gilrs` only behind the optional `gilrs` feature. `gilrs` covers Linux
  evdev, Windows Gaming Input/XInput and macOS Game Controller hotplug.
- One `PlayerInput` owns one `InputState`. This makes keyboard + gamepad actions
  compose, keeps multiplayer state isolated, and avoids global device state.
- `GamepadHub` allocates session ids and routes events to the assigned player.
  Player preferences use fingerprints (UUID, vendor/product, name), not session
  ids, so they survive reconnect and can be persisted.
- Analog axes are separate from digital `Button` values. Sticks are radial
  deadzoned; triggers have configurable rest/pressed ranges. Binding a
  `GamepadAxis` uses the processed value.

## Public API

| Type | Purpose |
|---|---|
| `GamepadButton` | Face/shoulder/trigger/stick/d-pad/menu buttons |
| `GamepadAxis` | Left/right stick X/Y and analog triggers |
| `Button::Gamepad` | Digital gamepad source usable by `Button`, `Axis`, `Chord` bindings |
| `Binding::gamepad_button` / `gamepad_axis` | Digital or analog named-action sources |
| `GamepadId` | Stable session identity; not reused for another device |
| `PlayerId` | Game-assigned local player slot |
| `GamepadFingerprint` | Persisted identity for reconnect matching |
| `GamepadConfig` | Stick/trigger deadzones, calibration, inversion, trigger threshold |
| `GamepadState` | Raw buttons/axes and processed stick/trigger values |
| `GamepadSet` | Connected-device registry and id allocation |
| `GamepadPreferences` | Serializable player-to-fingerprint preferences |
| `PlayerInput` | Player input state, assigned device and gamepad state |
| `GamepadHub` | Hotplug, assignment, reconnect and event routing |
| `GamepadEvent` | Backend-neutral connected/disconnected/button/axis event |
| `GilrsGamepads` | Optional native hotplug/event adapter |

`InputState::gamepad_button`, `gamepad_axis` and `gamepad_axes` provide the
per-player bridge. `release_gamepad` releases only controller-derived input;
`release_all` still handles focus loss. Existing `Bindings` JSON remains valid;
the new variant is additive.

## Lifecycle

1. Create a `GamepadHub`, add players, and install each player's `Bindings`.
   A game using `GameWindow` can either keep its existing keyboard-only input or
   route the same winit events into `player.input` with
   `quasar_window::input::process_window_event`; the latter makes the player's
   keyboard and gamepad sources share one action state.
2. Poll `GilrsGamepads::poll` once per frame (or feed `GamepadEvent` yourself).
3. Assign a device (`assign`) or set a preference (`prefer`); reconnects matching
   the fingerprint are assigned automatically.
4. Run the simulation from `hub.player(id).input`, then call `consume_edges`.
5. Persist `hub.preferences()` and reapply it on the next launch.

Disconnect clears the active assignment and releases controller input while
retaining the preference. Assigning a new device to a player releases the old
device's input first. Ids allocated by `GamepadSet` are monotonic and skipped if
already present; the gilrs adapter remembers its backend-to-Quasar map so a
reconnected controller keeps the same `GamepadId`.

## Verification

```sh
cargo test -p quasar-input
cargo test -p quasar-input --features gilrs
cargo clippy -p quasar-input --all-targets -- -D warnings
cargo clippy -p quasar-input --all-targets --features gilrs -- -D warnings
cargo run -p quasar-input --example gamepad_probe --features gilrs
```

Headless tests cover combined keyboard/gamepad actions, alternate-source press
edges, deadzone/calibration/trigger math, per-player isolation, preferences
serialization, disconnect release, explicit and reconnecting ids, assignment
errors, invalid configuration and binding serialization.

The hardware probe opened the `gilrs` backend successfully, but this environment
had no gamepad visible to evdev (`devices=0`), so no physical input was recorded.
The hardware smoke requirement is therefore partially unverified. The adapter
and probe are ready for a machine with a controller; no performance/force-feedback
claims are made.
