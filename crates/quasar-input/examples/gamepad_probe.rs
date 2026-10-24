//! Native gamepad smoke probe. Build with `--features gilrs`.
//!
//! Prints hotplug, button, stick and trigger changes for a few seconds. This is
//! intentionally headless and useful on CI/hardware where no window is needed.
use quasar_input::{
    Binding, Bindings, GamepadAxis, GamepadButton, GamepadHub, GilrsGamepads, PlayerId,
};
use std::{
    collections::HashSet,
    env,
    time::{Duration, Instant},
};

fn main() {
    let seconds = env::var("QUASAR_GAMEPAD_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5.0);
    let mut backend = GilrsGamepads::new().expect("open native gamepad backend");
    let player = PlayerId::new(0);
    let mut hub = GamepadHub::new();
    hub.add_player(player).expect("add player");
    let mut bindings = Bindings::default();
    bindings
        .bind("south", Binding::gamepad_button(GamepadButton::South))
        .bind("left_x", Binding::gamepad_axis(GamepadAxis::LeftStickX))
        .bind("left_y", Binding::gamepad_axis(GamepadAxis::LeftStickY))
        .bind(
            "left_trigger",
            Binding::gamepad_axis(GamepadAxis::LeftTrigger),
        );
    hub.player_mut(player)
        .expect("player")
        .input
        .set_bindings(bindings)
        .expect("valid bindings");

    println!(
        "GAMEPAD_PROBE backend=gilrs seconds={seconds} device={:?}",
        env::var("QUASAR_GAMEPAD_DEVICE").ok()
    );
    let mut seen = HashSet::new();
    let mut last_state = String::new();
    let deadline = Instant::now() + Duration::from_secs_f32(seconds);
    while Instant::now() < deadline {
        backend.poll(&mut hub).expect("poll gamepads");
        let new_devices: Vec<_> = hub
            .devices()
            .iter()
            .filter(|info| !seen.contains(&info.id))
            .map(|info| (info.id, info.fingerprint.clone()))
            .collect();
        for (id, fingerprint) in new_devices {
            seen.insert(id);
            println!(
                "CONNECTED id={} name={:?} vendor={:?} product={:?}",
                id.value(),
                fingerprint.name,
                fingerprint.vendor_id,
                fingerprint.product_id
            );
            if hub.assignment(player).is_none() {
                let _ = hub.assign(player, id);
            }
        }
        if let Some(input) = hub.player(player) {
            let state = format!(
                "device={:?} south={} left=({:.3},{:.3}) trigger={:.3} south_action={}",
                input.gamepad().map(|id| id.value()),
                input.gamepad_state().button(GamepadButton::South),
                input.gamepad_state().left_stick().0,
                input.gamepad_state().left_stick().1,
                input.gamepad_state().left_trigger(),
                input.input.action("south").held(),
            );
            if state != last_state {
                println!("STATE {state}");
                last_state = state;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    println!(
        "GAMEPAD_PROBE_DONE devices={} assigned={:?}",
        hub.devices().len(),
        hub.assignment(player)
    );
}
