use super::*;

fn input_with(bindings: Bindings) -> InputState {
    let mut input = InputState::default();
    input.set_bindings(bindings).unwrap();
    input
}
#[test]
fn short_taps_and_repeats_preserve_action_edges_until_consumed() {
    let mut bindings = Bindings::default();
    bindings.bind("serve", Binding::key(Key::Space));
    let mut input = input_with(bindings);
    input.key(Key::Space, true);
    input.key(Key::Space, false);
    assert_eq!(
        input.action("serve"),
        ActionState {
            value: 0.0,
            pressed: true,
            released: true
        }
    );
    input.consume_edges();
    assert_eq!(input.action("serve"), ActionState::default());
    input.key(Key::Space, true);
    input.consume_edges();
    input.key(Key::Space, true);
    assert!(input.action("serve").held());
    assert!(!input.action("serve").pressed);
}
#[test]
fn alternate_bindings_release_only_when_the_action_becomes_inactive() {
    let mut bindings = Bindings::default();
    bindings.bind("fire", Binding::key(Key::Space)).bind(
        "fire",
        Binding::Button {
            button: Button::Mouse(MouseButton::Left),
        },
    );
    let mut input = input_with(bindings);
    input.key(Key::Space, true);
    input.consume_edges();
    input.mouse_button(MouseButton::Left, true);
    assert!(!input.action("fire").pressed);
    input.key(Key::Space, false);
    assert!(input.action("fire").held());
    assert!(!input.action("fire").released);
    input.mouse_button(MouseButton::Left, false);
    assert!(input.action("fire").released);
}
#[test]
fn axes_cancel_opposing_keys_and_focus_loss_cancels_pending_commands() {
    let mut bindings = Bindings::default();
    bindings.bind("move", Binding::axis(Key::KeyW, Key::KeyS));
    let mut input = input_with(bindings);
    input.key(Key::KeyW, true);
    assert_eq!(input.action("move").value, -1.0);
    input.key(Key::KeyS, true);
    assert_eq!(input.action("move").value, 0.0);
    input.key(Key::KeyW, false);
    assert_eq!(input.action("move").value, 1.0);
    input.text.push_str("unsent chat");
    input.release_all();
    assert!(input.text.is_empty());
    assert_eq!(
        input.action("move"),
        ActionState {
            value: 0.0,
            pressed: false,
            released: true
        }
    );
    assert!(!input.held(Key::KeyS));
    assert!(input.released(Key::KeyS));
    input.consume_edges();
    assert_eq!(input.action("move"), ActionState::default());
    // A key pressed after focus returns must produce a fresh edge, not inherit
    // stale state cleared by release_all.
    input.key(Key::KeyS, true);
    assert!(input.action("move").pressed);
    assert!(input.action("move").held());
}
#[test]
fn chord_edges_are_immediate_and_consumed_without_affecting_other_actions() {
    let mut bindings = Bindings::default();
    bindings
        .bind("fullscreen", Binding::chord([Key::AltLeft, Key::Enter]))
        .bind("serve", Binding::key(Key::Space));
    let mut input = input_with(bindings);
    input.key(Key::Enter, true);
    assert!(!input.take_pressed("fullscreen"));
    input.key(Key::AltLeft, true);
    assert!(input.take_pressed("fullscreen"));
    input.key(Key::Space, true);
    assert!(!input.take_pressed("fullscreen"));
    assert!(input.action("serve").pressed);
    input.key(Key::AltLeft, false);
    assert!(input.action("fullscreen").released);
}
#[test]
fn rebinding_has_no_spurious_press_and_invalid_replacement_keeps_old_bindings() {
    let mut input = InputState::default();
    input.key(Key::KeyW, true);
    let mut bindings = Bindings::default();
    bindings.bind("fire", Binding::key(Key::KeyW));
    input.set_bindings(bindings.clone()).unwrap();
    assert!(input.action("fire").held());
    assert!(!input.action("fire").pressed);
    assert!(!input.button_pressed(Button::Key(Key::KeyW)));
    let mut invalid = bindings.clone();
    invalid.bind("bad", Binding::chord([]));
    assert!(input.set_bindings(invalid).is_err());
    assert_eq!(input.bindings(), &bindings);
    input.key(Key::KeyW, false);
    assert!(input.action("fire").released);
}
#[test]
fn duplicate_keyboard_and_gamepad_events_create_no_duplicate_edges() {
    let mut bindings = Bindings::default();
    bindings
        .bind("fire", Binding::key(Key::Space))
        .bind("fire", Binding::gamepad_button(GamepadButton::South));
    let mut input = input_with(bindings);
    input.key(Key::Space, true);
    input.key(Key::Space, true);
    assert!(input.action("fire").pressed);
    input.consume_edges();
    input.key(Key::Space, true);
    assert!(!input.action("fire").pressed);
    input.key(Key::Space, false);
    assert!(input.action("fire").released);
    input.consume_edges();
    input.key(Key::Space, false);
    assert!(!input.action("fire").released);

    input.gamepad_button(GamepadButton::South, true);
    input.gamepad_button(GamepadButton::South, true);
    assert!(input.action("fire").pressed);
    input.consume_edges();
    input.gamepad_button(GamepadButton::South, false);
    input.gamepad_button(GamepadButton::South, false);
    assert!(input.action("fire").released);
}

#[test]
fn simultaneous_keyboard_and_gamepad_sources_release_only_at_zero() {
    let mut bindings = Bindings::default();
    bindings
        .bind("right", Binding::key(Key::ArrowRight))
        .bind("right", Binding::gamepad_axis(GamepadAxis::LeftStickX));
    let mut input = input_with(bindings);
    input.key(Key::ArrowRight, true);
    input.gamepad_axis(GamepadAxis::LeftStickX, 0.6);
    assert_eq!(input.action("right").value, 1.0);
    input.key(Key::ArrowRight, false);
    assert!(input.action("right").held());
    assert!(!input.action("right").released);
    input.gamepad_axis(GamepadAxis::LeftStickX, 0.0);
    assert!(!input.action("right").held());
    assert!(input.action("right").released);
}

#[test]
fn hub_release_all_clears_inputs_but_keeps_assignments() {
    let mut bindings = Bindings::default();
    bindings
        .bind("fire", Binding::gamepad_button(GamepadButton::South))
        .bind("walk", Binding::key(Key::KeyW));
    let mut hub = GamepadHub::new();
    hub.add_player(PlayerId::new(0)).unwrap();
    hub.player_mut(PlayerId::new(0))
        .unwrap()
        .input
        .set_bindings(bindings)
        .unwrap();
    let device = hub.connect(GamepadFingerprint::new("Focus Pad"));
    hub.assign(PlayerId::new(0), device).unwrap();
    hub.button(device, GamepadButton::South, true);
    hub.player_mut(PlayerId::new(0))
        .unwrap()
        .input
        .key(Key::KeyW, true);
    hub.release_all();
    let player = hub.player(PlayerId::new(0)).unwrap();
    assert!(!player.input.action("fire").held());
    assert!(!player.input.action("walk").held());
    assert!(player.input.action("fire").released);
    assert_eq!(hub.assignment(PlayerId::new(0)), Some(device));
}

#[test]
fn gamepad_buttons_axes_and_keyboard_combine_into_named_actions() {
    let mut bindings = Bindings::default();
    bindings
        .bind("fire", Binding::key(Key::Space))
        .bind("fire", Binding::gamepad_button(GamepadButton::South))
        .bind("move", Binding::axis(Key::KeyA, Key::KeyD))
        .bind("move", Binding::gamepad_axis(GamepadAxis::LeftStickX));
    let mut input = input_with(bindings);
    input.gamepad_button(GamepadButton::South, true);
    assert!(input.action("fire").held());
    assert!(input.action("fire").pressed);
    input.consume_edges();
    input.key(Key::Space, true);
    assert!(!input.action("fire").pressed);
    input.gamepad_axes([
        (GamepadAxis::LeftStickX, 0.8),
        (GamepadAxis::LeftStickY, 0.0),
    ]);
    assert!((input.action("move").value - 0.8).abs() < 1e-6);
    input.key(Key::KeyA, true);
    assert!((input.action("move").value + 0.2).abs() < 1e-6);
    input.gamepad_button(GamepadButton::South, false);
    assert!(input.action("fire").held());
    assert!(!input.action("fire").released);
    input.key(Key::Space, false);
    assert!(!input.action("fire").held());
    assert!(input.action("fire").released);
    input.release_all();
    assert_eq!(input.gamepad_axis_value(GamepadAxis::LeftStickX), 0.0);
    assert!(!input.button_held(Button::Gamepad(GamepadButton::South)));
}

#[test]
fn stick_deadzones_calibration_and_trigger_thresholds_are_deterministic() {
    let mut state = GamepadState::default();
    state.set_axis(GamepadAxis::LeftStickX, 0.1);
    assert_eq!(state.left_stick(), (0.0, 0.0));
    state.set_axis(GamepadAxis::LeftStickX, 0.5);
    let (x, y) = state.left_stick();
    assert!((x - 0.35 / 0.85).abs() < 1e-6);
    assert_eq!(y, 0.0);

    let mut config = GamepadConfig::default();
    config.left_stick.deadzone = 0.0;
    config.left_stick.outer = 0.0;
    assert!(config.validate().is_err());

    config = GamepadConfig::default();
    config.left_trigger = TriggerConfig {
        rest: 0.0,
        pressed: 1.0,
        deadzone: 0.1,
    };
    let mut state = GamepadState::new(config).unwrap();
    state.set_axis(GamepadAxis::LeftTrigger, 0.05);
    assert_eq!(state.left_trigger(), 0.0);
    state.set_axis(GamepadAxis::LeftTrigger, 0.55);
    assert!((state.left_trigger() - 0.5).abs() < 1e-6);

    config.left_stick.invert_x = true;
    config.left_stick.deadzone = 0.0;
    let mut state = GamepadState::new(config).unwrap();
    state.set_axis(GamepadAxis::LeftStickX, 0.5);
    assert!(state.left_stick().0 < 0.0);
}

#[test]
fn player_input_releases_only_gamepad_state_when_unassigned() {
    let mut bindings = Bindings::default();
    bindings
        .bind("move", Binding::gamepad_axis(GamepadAxis::LeftStickY))
        .bind("walk", Binding::key(Key::KeyW));
    let mut player = PlayerInput::new(PlayerId::new(0));
    player.input.set_bindings(bindings).unwrap();
    player.assign(Some(GamepadId::new(7)));
    player.input.key(Key::KeyW, true);
    player.axis(GamepadAxis::LeftStickY, -0.9);
    assert!((player.input.action("move").value + (0.9 - 0.15) / 0.85).abs() < 1e-6);
    assert_eq!(player.input.action("walk").value, 1.0);
    player.assign(None);
    assert_eq!(player.input.action("move").value, 0.0);
    assert!(player.input.action("move").released);
    assert_eq!(player.input.action("walk").value, 1.0);
    assert_eq!(
        player.gamepad_state().raw_axis(GamepadAxis::LeftStickY),
        0.0
    );
}

#[test]
fn hub_routes_multiple_players_and_restores_preferences_after_reconnect() {
    let fingerprint = GamepadFingerprint::new("Test Pad")
        .with_ids(Some(0x1234), Some(0x5678))
        .with_uuid(Some([7; 16]));
    let mut bindings = Bindings::default();
    bindings
        .bind("fire", Binding::gamepad_button(GamepadButton::South))
        .bind("move", Binding::gamepad_axis(GamepadAxis::LeftStickX));
    let mut hub = GamepadHub::new();
    hub.add_player(PlayerId::new(0)).unwrap();
    hub.add_player(PlayerId::new(1)).unwrap();
    hub.player_mut(PlayerId::new(0))
        .unwrap()
        .input
        .set_bindings(bindings.clone())
        .unwrap();
    hub.player_mut(PlayerId::new(1))
        .unwrap()
        .input
        .set_bindings(bindings)
        .unwrap();
    let device = hub.connect(fingerprint.clone());
    hub.assign(PlayerId::new(0), device).unwrap();
    let second = hub.connect(GamepadFingerprint::new("Second Pad"));
    hub.assign(PlayerId::new(1), second).unwrap();
    assert!(hub.button(device, GamepadButton::South, true));
    assert!(
        hub.player(PlayerId::new(0))
            .unwrap()
            .input
            .action("fire")
            .held()
    );
    assert!(
        !hub.player(PlayerId::new(1))
            .unwrap()
            .input
            .action("fire")
            .held()
    );
    assert!(hub.button(second, GamepadButton::South, true));
    assert!(
        hub.player(PlayerId::new(1))
            .unwrap()
            .input
            .action("fire")
            .held()
    );
    hub.disconnect(device);
    assert!(
        !hub.player(PlayerId::new(0))
            .unwrap()
            .input
            .action("fire")
            .held()
    );
    assert!(
        hub.player(PlayerId::new(0))
            .unwrap()
            .input
            .action("fire")
            .released
    );
    // The other player's controller is untouched by the first disconnect.
    assert!(
        hub.player(PlayerId::new(1))
            .unwrap()
            .input
            .action("fire")
            .held()
    );
    let preferences = hub.preferences();
    let encoded = serde_json::to_string(&preferences).unwrap();
    let decoded: GamepadPreferences = serde_json::from_str(&encoded).unwrap();
    let reconnected = hub.connect(fingerprint);
    assert_eq!(hub.assignment(PlayerId::new(0)), Some(reconnected));
    hub.apply_preferences(decoded).unwrap();
    assert_eq!(hub.assignment(PlayerId::new(0)), Some(reconnected));
}

#[test]
fn hub_preserves_ids_for_explicit_reconnect_and_exposes_events() {
    let fingerprint = GamepadFingerprint::new("Reconnect Pad");
    let mut hub = GamepadHub::new();
    hub.add_player(PlayerId::new(0)).unwrap();
    let id = hub.connect(fingerprint.clone());
    hub.assign(PlayerId::new(0), id).unwrap();
    hub.player_mut(PlayerId::new(0))
        .unwrap()
        .input
        .set_bindings({
            let mut bindings = Bindings::default();
            bindings.bind("east", Binding::gamepad_button(GamepadButton::East));
            bindings
        })
        .unwrap();
    hub.handle_event(&GamepadEvent::button(id, GamepadButton::East, true))
        .unwrap();
    assert!(
        hub.player(PlayerId::new(0))
            .unwrap()
            .input
            .action("east")
            .held()
    );
    hub.disconnect(id);
    hub.connect_with_id(id, fingerprint).unwrap();
    assert_eq!(hub.assignment(PlayerId::new(0)), Some(id));
    hub.handle_event(&GamepadEvent::disconnected(id)).unwrap();
    assert_eq!(hub.assignment(PlayerId::new(0)), None);
}

#[cfg(feature = "diagnostics")]
#[test]
fn gamepad_hub_records_hotplug_and_assignment_through_an_explicit_sink() {
    use quasar_diagnostics::{Diagnostics, MemorySink};
    use std::sync::Arc;
    let sink = Arc::new(MemorySink::new(16));
    let mut hub = GamepadHub::new();
    hub.set_diagnostics(Some(Diagnostics::new(sink.clone())));
    hub.add_player(PlayerId::new(0)).unwrap();
    let id = hub.connect(GamepadFingerprint::new("Diagnostic Pad"));
    hub.assign(PlayerId::new(0), id).unwrap();
    hub.disconnect(id);
    let records = sink.records();
    assert!(
        records
            .iter()
            .any(|record| record.name == "gamepad_connected")
    );
    assert!(
        records
            .iter()
            .any(|record| record.name == "gamepad_assigned")
    );
    assert!(
        records
            .iter()
            .any(|record| record.name == "gamepad_disconnected")
    );
}

#[test]
fn invalid_gamepad_config_assignment_and_bindings_are_rejected() {
    let config = GamepadConfig {
        trigger_button_threshold: f32::NAN,
        ..GamepadConfig::default()
    };
    assert!(config.validate().is_err());
    assert!(GamepadState::new(config).is_err());
    let mut hub = GamepadHub::new();
    hub.add_player(PlayerId::new(0)).unwrap();
    assert!(matches!(
        hub.assign(PlayerId::new(0), GamepadId::new(99)),
        Err(GamepadError::UnknownGamepad(_))
    ));
    assert!(matches!(
        hub.prefer(PlayerId::new(9), GamepadFingerprint::new("x")),
        Err(GamepadError::UnknownPlayer(_))
    ));
    let mut bindings = Bindings::default();
    bindings.bind(
        "bad",
        Binding::gamepad_axis_scaled(GamepadAxis::LeftStickX, f32::NAN),
    );
    assert!(bindings.validate().is_err());

    let mut gamepad = Bindings::default();
    gamepad
        .bind("fire", Binding::gamepad_button(GamepadButton::South))
        .bind("move", Binding::gamepad_axis(GamepadAxis::LeftStickX));
    let encoded = serde_json::to_string(&gamepad).unwrap();
    assert!(encoded.contains("gamepad"));
    let restored: Bindings = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored, gamepad);
    assert!(
        serde_json::from_str::<Bindings>(
            r#"{"actions":{"x":[{"type":"gamepad_axis","axis":"not_an_axis","scale":1.0}]}}"#
        )
        .is_err()
    );
}

#[test]
fn bindings_serialize_and_reject_typographical_errors_and_ambiguous_sources() {
    let mut bindings = Bindings::default();
    bindings
        .bind("move", Binding::axis(Key::KeyW, Key::KeyS))
        .bind("pause", Binding::chord([Key::ControlLeft, Key::KeyP]));
    let json = serde_json::to_string(&bindings).unwrap();
    let restored: Bindings = serde_json::from_str(&json).unwrap();
    restored.validate().unwrap();
    assert_eq!(bindings, restored);
    assert!(serde_json::from_str::<Bindings>(&json.replace("KeyW", "NotAKey")).is_err());
    assert!(serde_json::from_str::<Bindings>(r#"{"actionz":{}}"#).is_err());
    for binding in [
        Binding::axis(Key::KeyW, Key::KeyW),
        Binding::chord([Key::AltLeft, Key::AltLeft]),
    ] {
        let mut invalid = Bindings::default();
        invalid.bind("bad", binding);
        assert!(invalid.validate().is_err());
    }
    let mut invalid = Bindings::default();
    invalid.bind(" ", Binding::key(Key::Space));
    assert!(invalid.validate().is_err());
}
