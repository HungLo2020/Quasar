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
    input.release_all();
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
    let mut invalid = bindings.clone();
    invalid.bind("bad", Binding::chord([]));
    assert!(input.set_bindings(invalid).is_err());
    assert_eq!(input.bindings(), &bindings);
    input.key(Key::KeyW, false);
    assert!(input.action("fire").released);
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
