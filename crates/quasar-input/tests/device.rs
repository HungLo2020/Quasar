//! Active-device tracking, button-pair axes and prompt names.
use quasar_input::{
    ACTIVE_AXIS_THRESHOLD, Binding, Bindings, Button, DeviceFamily, GamepadAxis, GamepadButton,
    GamepadFingerprint, InputDevice, InputState, Key, MouseButton,
};

#[test]
fn last_active_device_follows_presses_axes_and_reported_pointer_use() {
    let mut input = InputState::default();
    assert_eq!(input.last_active_device(), None);
    input.key(Key::KeyW, true);
    assert_eq!(input.last_active_device(), Some(InputDevice::KeyboardMouse));
    input.gamepad_button(GamepadButton::South, true);
    assert_eq!(input.last_active_device(), Some(InputDevice::Gamepad));
    // Releasing a key does not switch back.
    input.key(Key::KeyW, false);
    assert_eq!(input.last_active_device(), Some(InputDevice::Gamepad));
    input.mouse_button(MouseButton::Left, true);
    assert_eq!(input.last_active_device(), Some(InputDevice::KeyboardMouse));
    // Small stick drift does not steal prompts; deliberate movement does.
    input.gamepad_axis(GamepadAxis::LeftStickX, ACTIVE_AXIS_THRESHOLD * 0.5);
    assert_eq!(input.last_active_device(), Some(InputDevice::KeyboardMouse));
    input.gamepad_axis(GamepadAxis::LeftStickX, -0.9);
    assert_eq!(input.last_active_device(), Some(InputDevice::Gamepad));
    input.note_device(InputDevice::KeyboardMouse);
    assert_eq!(input.last_active_device(), Some(InputDevice::KeyboardMouse));
    // Focus loss keeps the last device; held keys repeating do not re-trigger.
    input.release_all();
    assert_eq!(input.last_active_device(), Some(InputDevice::KeyboardMouse));
}

#[test]
fn frame_snapshots_report_device_changes_once() {
    let mut input = InputState::default();
    let mut frame = input.frame(0);
    assert!(!frame.active_device_changed());
    input.key(Key::Space, true);
    frame.capture(&input, 1);
    assert_eq!(frame.last_active_device(), Some(InputDevice::KeyboardMouse));
    assert!(frame.active_device_changed());
    frame.capture(&input, 2);
    assert!(!frame.active_device_changed());
    input.gamepad_button(GamepadButton::Start, true);
    frame.capture(&input, 3);
    assert!(frame.active_device_changed());
    assert_eq!(frame.last_active_device(), Some(InputDevice::Gamepad));
}

#[test]
fn gamepad_button_pairs_drive_axis_actions() {
    let mut bindings = Bindings::default();
    bindings
        .bind(
            "move_x",
            Binding::gamepad_button_axis(GamepadButton::DPadLeft, GamepadButton::DPadRight),
        )
        .bind("move_x", Binding::axis(Key::KeyA, Key::KeyD));
    bindings.validate().unwrap();
    let mut input = InputState::default();
    input.set_bindings(bindings).unwrap();
    input.gamepad_button(GamepadButton::DPadRight, true);
    assert_eq!(input.action("move_x").value, 1.0);
    assert!(input.action("move_x").pressed);
    input.gamepad_button(GamepadButton::DPadLeft, true);
    assert_eq!(input.action("move_x").value, 0.0, "opposites cancel");
    input.gamepad_button(GamepadButton::DPadRight, false);
    assert_eq!(input.action("move_x").value, -1.0);

    let same = Binding::gamepad_button_axis(GamepadButton::South, GamepadButton::South);
    let mut invalid = Bindings::default();
    invalid.bind("bad", same);
    assert!(invalid.validate().is_err());
    let json = serde_json::to_string(&Binding::button_axis(
        Button::Gamepad(GamepadButton::LeftBumper),
        Button::Key(Key::KeyQ),
    ))
    .unwrap();
    assert!(json.contains("\"type\":\"axis\""));
}

#[test]
fn display_names_follow_device_family_conventions() {
    let interact = Binding::gamepad_button(GamepadButton::South);
    assert_eq!(interact.display_name(DeviceFamily::Xbox), "A");
    assert_eq!(interact.display_name(DeviceFamily::PlayStation), "Cross");
    assert_eq!(interact.display_name(DeviceFamily::Nintendo), "B");
    assert_eq!(interact.display_name(DeviceFamily::GenericGamepad), "South");
    assert_eq!(
        Binding::gamepad_axis(GamepadAxis::RightTrigger).display_name(DeviceFamily::PlayStation),
        "R2"
    );
    assert_eq!(
        Binding::key(Key::KeyE).display_name(DeviceFamily::KeyboardMouse),
        "E"
    );
    assert_eq!(
        Binding::key(Key::Digit3).display_name(DeviceFamily::KeyboardMouse),
        "3"
    );
    assert_eq!(
        Binding::key(Key::ShiftLeft).display_name(DeviceFamily::KeyboardMouse),
        "Left Shift"
    );
    assert_eq!(
        Binding::key(Key::ArrowUp).display_name(DeviceFamily::KeyboardMouse),
        "Up"
    );
    assert_eq!(
        Binding::key(Key::Escape).display_name(DeviceFamily::KeyboardMouse),
        "Esc"
    );
    assert_eq!(
        Binding::axis(Key::KeyA, Key::KeyD).display_name(DeviceFamily::KeyboardMouse),
        "A / D"
    );
    assert_eq!(
        Binding::chord([Key::ControlLeft, Key::KeyS]).display_name(DeviceFamily::KeyboardMouse),
        "Left Ctrl + S"
    );
    assert_eq!(
        Button::Mouse(MouseButton::Right).display_name(DeviceFamily::KeyboardMouse),
        "Right Mouse"
    );
    assert_eq!(
        DeviceFamily::from_vendor_id(Some(0x054C)),
        DeviceFamily::PlayStation
    );
    assert_eq!(
        DeviceFamily::from_vendor_id(None),
        DeviceFamily::GenericGamepad
    );
    let pad = GamepadFingerprint::new("Pad").with_ids(Some(0x045E), Some(1));
    assert_eq!(pad.family(), DeviceFamily::Xbox);
    assert_eq!(
        DeviceFamily::for_device(InputDevice::Gamepad, Some(&pad)),
        DeviceFamily::Xbox
    );
    assert_eq!(
        DeviceFamily::for_device(InputDevice::KeyboardMouse, Some(&pad)),
        DeviceFamily::KeyboardMouse
    );
}

#[test]
fn prompts_pick_the_binding_for_the_active_family() {
    let mut bindings = Bindings::default();
    bindings
        .bind("interact", Binding::key(Key::KeyE))
        .bind("interact", Binding::gamepad_button(GamepadButton::West))
        .bind("jump", Binding::key(Key::Space));
    assert_eq!(
        bindings.display_names("interact", DeviceFamily::Xbox),
        vec!["X".to_owned()]
    );
    assert_eq!(
        bindings.format_prompt("Press {interact} to open", DeviceFamily::KeyboardMouse),
        "Press E to open"
    );
    assert_eq!(
        bindings.format_prompt("Press {interact} to open", DeviceFamily::PlayStation),
        "Press Square to open"
    );
    // Missing gamepad binding and unknown actions stay visible.
    assert_eq!(
        bindings.format_prompt("{jump} / {missing} {{literal}}", DeviceFamily::Nintendo),
        "{jump} / {missing} {literal}"
    );
    assert_eq!(
        bindings.format_prompt("open {brace", DeviceFamily::Xbox),
        "open {brace"
    );
    assert_eq!(
        bindings
            .display_name("jump", DeviceFamily::KeyboardMouse)
            .as_deref(),
        Some("Space")
    );
}
