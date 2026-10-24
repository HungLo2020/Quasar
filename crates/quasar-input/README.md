# quasar-input

Portable keyboard, mouse and gamepad state plus named actions. The core has no
windowing, graphics, file I/O or compulsory native device dependency. Games own
action names, player count and configuration versions; input can come from a
window adapter, another host, a synthetic test, or the optional native backend.

```rust
use quasar_input::{Binding, Bindings, InputState, Key};

let mut bindings = Bindings::default();
bindings.bind("move", Binding::axis(Key::KeyW, Key::KeyS))
    .bind("serve", Binding::key(Key::Space))
    .bind("fullscreen", Binding::chord([Key::AltLeft, Key::Enter]));
let mut input = InputState::default();
input.set_bindings(bindings).unwrap();
input.key(Key::KeyW, true);
assert_eq!(input.action("move").value, -1.0);
input.consume_edges();
```

`Key` identifies physical locations rather than characters in a particular
layout. Text input is separate. `Button` supports physical keys and mouse
buttons. Buttons contribute +1; an opposing-button axis contributes -1/0/+1;
a chord requires every button to be held. Alternative bindings for an action
are summed and clamped to [-1,1]. Empty lists intentionally disable actions.
Mouse motion and line/pixel wheel deltas remain separately available; they
are not currently bindable axes.

## Controllers

Gamepad types and routing are always available; the native `gilrs` backend is
optional:

```toml
quasar-input = { path = "../Quasar/crates/quasar-input", features = ["gilrs"] }
```

The core never depends on `gilrs`, SDL, winit or Qt. Feed synthetic
`GamepadEvent` values from a test or another platform API, or use
`GilrsGamepads` for evdev/XInput/GameController discovery:

```rust
use quasar_input::{
    Binding, Bindings, GamepadAxis, GamepadButton, GamepadHub, GilrsGamepads,
    PlayerId,
};

let mut hub = GamepadHub::new();
let player = PlayerId::new(0);
hub.add_player(player)?;
let mut bindings = Bindings::default();
bindings
    .bind("fire", Binding::gamepad_button(GamepadButton::South))
    .bind("fire", Binding::key(quasar_input::Key::Space))
    .bind("move", Binding::gamepad_axis(GamepadAxis::LeftStickX))
    .bind("move", Binding::axis(quasar_input::Key::KeyA, quasar_input::Key::KeyD));
hub.player_mut(player).unwrap().input.set_bindings(bindings)?;

let mut gamepads = GilrsGamepads::new()?;
loop {
    gamepads.poll(&mut hub)?;
    let input = &hub.player(player).unwrap().input;
    let _fire = input.action("fire").pressed;
    let _move = input.action("move").value;
    // Run one simulation tick, then:
    hub.player_mut(player).unwrap().input.consume_edges();
}
```

The bundled adapter registers already-connected controllers, consumes
hotplug/button/axis events, preserves a Quasar `GamepadId` when `gilrs` reuses
an id after reconnect, and maps standard face/shoulder/trigger/stick/d-pad
controls. One `PlayerInput` owns one `InputState`: keyboard and gamepad sources
therefore combine into the same named actions, and multiple players are
independent states routed by `GamepadHub`. In a winit host, feed the same window
events to `quasar_window::input::process_window_event(&mut player.input, event)`
when the player's keyboard and controller must share action state; `GameWindow`'s
built-in input can remain keyboard-only if that is simpler.

Each player has a `GamepadConfig`:

- radial left/right stick deadzones and outer range;
- per-axis center/min/max calibration;
- optional X/Y inversion;
- per-trigger rest/pressed range and deadzone;
- a digital trigger-button threshold.

Triggers are available as analog `GamepadAxis` values and as digital
`GamepadButton` values. Bind one or the other for an action unless summing both
is intentional. `GamepadHub::preferences`/`apply_preferences` persist player to
fingerprint mappings; `Bindings` and `GamepadPreferences` both serialize through
serde. Preferences survive disconnect/reconnect and are matched by UUID plus
vendor/product/name.

With the optional `diagnostics` feature, `GamepadHub::set_diagnostics` emits
hotplug/assignment records to an explicit `quasar_diagnostics::Sink`. No global
logger is installed and the hook is absent from default builds.

Platform notes:

- Linux uses evdev and requires read access to `/dev/input/event*` plus
  `libudev-dev` at build time when the `gilrs` feature is enabled.
- Windows defaults to Windows Gaming Input, which needs an in-focus window;
  `gilrs` can be configured for XInput by downstream dependency settings.
- macOS uses the Game Controller framework. Force feedback is not exposed by
  this crate, and no platform parity is claimed beyond mapped buttons/axes.
- Hardware smoke:
  `cargo run -p quasar-input --example gamepad_probe --features gilrs`.
  This environment exposed no readable gamepad, so the synthetic tests are
  complete and the hardware run remains an explicit limitation.

Actions are evaluated on each actual button change. `pressed`/`released` track
transitions into/out of a nonzero value and accumulate until `consume_edges`.
A complete press/release between ticks therefore survives. Auto-repeat and an
additional alternate binding do not repeat an already-active action's press.
The native window adapter suppresses repeated key edges but still forwards
repeated printable text to `InputState::text` for chat/console/UI entry. Use
`PlayerInput::release_all`/`GamepadHub::release_all` on focus loss to clear
keyboard and controller state while retaining device assignments.

Consume once after a simulation tick, retaining held values for catch-up ticks.
Immediate commands can use `take_pressed(name)` to consume only their own edge.
Call `release_all` on focus loss: held devices clear, releases remain observable,
and pending presses/motion/text are cancelled. Replacing bindings initializes
values from currently held devices without inventing new action presses;
invalid replacements leave the previous configuration intact.

`Bindings` serializes through serde and validates empty action names, empty or
duplicate chords, and identical opposing buttons. Unknown physical key names
are rejected on deserialization. Games can perform further validation of their
own action names. Different actions may intentionally share a button.

`quasar-window::input` adapts winit events into this crate. It can be used with
`GameWindow` or called directly by a custom host. The core does not depend on
that adapter. See [Pong's controls](../../examples/pong/src/controls.rs) for
configuration loading and named gameplay/window commands.

External applications can consume this crate using a pinned Git dependency
(or a registry version after publication), without a sibling Quasar checkout:

```toml
quasar-input = { git = "https://github.com/HungLo2020/Quasar", rev = "<commit-containing-this-crate>" }
```

Cargo fetches the repository into its managed cache. Paths in this repository's
workspace manifests are internal workspace wiring, not a requirement imposed
on external applications. Replace the revision placeholder with a commit containing this crate.
