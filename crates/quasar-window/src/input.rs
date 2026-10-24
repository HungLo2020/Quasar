//! Native event adapter; state and action mapping belong to quasar-input.
use quasar_input::{InputState, Key, MouseButton};
use winit::{
    event::{DeviceEvent, ElementState, Ime, MouseScrollDelta, WindowEvent},
    keyboard::{KeyCode, PhysicalKey},
};

/// Feed one native event without consuming accumulated action or device edges.
pub fn process_window_event(input: &mut InputState, event: &WindowEvent) {
    match event {
        WindowEvent::KeyboardInput {
            event,
            is_synthetic: false,
            ..
        } => process_key_event(
            input,
            event.physical_key,
            event.state,
            event.repeat,
            event.text.as_deref(),
        ),
        WindowEvent::MouseInput { state, button, .. } => {
            input.mouse_button(mouse_button(*button), state.is_pressed())
        }
        // IME commits are the authoritative text entry path on Wayland and
        // composed-input platforms. Preedit is intentionally not committed.
        WindowEvent::Ime(Ime::Commit(text)) => input.text.push_str(text),
        WindowEvent::CursorMoved { position, .. } => {
            input.cursor_physical = Some((position.x, position.y))
        }
        WindowEvent::CursorLeft { .. } => input.cursor_physical = None,
        WindowEvent::MouseWheel { delta, .. } => match delta {
            MouseScrollDelta::LineDelta(x, y) => {
                input.scroll_lines.0 += f64::from(*x);
                input.scroll_lines.1 += f64::from(*y);
            }
            MouseScrollDelta::PixelDelta(p) => {
                input.scroll_pixels.0 += p.x;
                input.scroll_pixels.1 += p.y;
            }
        },
        WindowEvent::Focused(false) => input.release_all(),
        _ => {}
    }
}

/// Applies a native key event.
///
/// OS auto-repeat never creates a physical or action press edge: a held key
/// remains held until its real release. Repeated printable text is still
/// appended to [`InputState::text`] so chat, console and UI text fields retain
/// normal key-repeat editing behavior. `text` is separate from bindings and is
/// cleared with the other edges by [`InputState::consume_edges`].
pub fn process_key_event(
    input: &mut InputState,
    physical: PhysicalKey,
    state: ElementState,
    repeat: bool,
    text: Option<&str>,
) {
    if !repeat
        && let PhysicalKey::Code(code) = physical
        && let Some(key) = physical_key(code)
    {
        input.key(key, state == ElementState::Pressed);
    }
    if state == ElementState::Pressed
        && let Some(text) = text
    {
        input.text.push_str(text);
    }
}

/// Relative device motion is accepted only for the focused window.
pub fn process_device_event(input: &mut InputState, event: &DeviceEvent, focused: bool) {
    if focused && let DeviceEvent::MouseMotion { delta } = event {
        input.relative_motion.0 += delta.0;
        input.relative_motion.1 += delta.1;
    }
}

fn mouse_button(button: winit::event::MouseButton) -> MouseButton {
    match button {
        winit::event::MouseButton::Left => MouseButton::Left,
        winit::event::MouseButton::Right => MouseButton::Right,
        winit::event::MouseButton::Middle => MouseButton::Middle,
        winit::event::MouseButton::Back => MouseButton::Back,
        winit::event::MouseButton::Forward => MouseButton::Forward,
        winit::event::MouseButton::Other(value) => MouseButton::Other(value),
    }
}

/// Maps supported physical codes. Future winit codes are ignored until mapped.
pub fn physical_key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::Backquote => Key::Backquote,
        KeyCode::Backslash => Key::Backslash,
        KeyCode::BracketLeft => Key::BracketLeft,
        KeyCode::BracketRight => Key::BracketRight,
        KeyCode::Comma => Key::Comma,
        KeyCode::Digit0 => Key::Digit0,
        KeyCode::Digit1 => Key::Digit1,
        KeyCode::Digit2 => Key::Digit2,
        KeyCode::Digit3 => Key::Digit3,
        KeyCode::Digit4 => Key::Digit4,
        KeyCode::Digit5 => Key::Digit5,
        KeyCode::Digit6 => Key::Digit6,
        KeyCode::Digit7 => Key::Digit7,
        KeyCode::Digit8 => Key::Digit8,
        KeyCode::Digit9 => Key::Digit9,
        KeyCode::Equal => Key::Equal,
        KeyCode::IntlBackslash => Key::IntlBackslash,
        KeyCode::IntlRo => Key::IntlRo,
        KeyCode::IntlYen => Key::IntlYen,
        KeyCode::KeyA => Key::KeyA,
        KeyCode::KeyB => Key::KeyB,
        KeyCode::KeyC => Key::KeyC,
        KeyCode::KeyD => Key::KeyD,
        KeyCode::KeyE => Key::KeyE,
        KeyCode::KeyF => Key::KeyF,
        KeyCode::KeyG => Key::KeyG,
        KeyCode::KeyH => Key::KeyH,
        KeyCode::KeyI => Key::KeyI,
        KeyCode::KeyJ => Key::KeyJ,
        KeyCode::KeyK => Key::KeyK,
        KeyCode::KeyL => Key::KeyL,
        KeyCode::KeyM => Key::KeyM,
        KeyCode::KeyN => Key::KeyN,
        KeyCode::KeyO => Key::KeyO,
        KeyCode::KeyP => Key::KeyP,
        KeyCode::KeyQ => Key::KeyQ,
        KeyCode::KeyR => Key::KeyR,
        KeyCode::KeyS => Key::KeyS,
        KeyCode::KeyT => Key::KeyT,
        KeyCode::KeyU => Key::KeyU,
        KeyCode::KeyV => Key::KeyV,
        KeyCode::KeyW => Key::KeyW,
        KeyCode::KeyX => Key::KeyX,
        KeyCode::KeyY => Key::KeyY,
        KeyCode::KeyZ => Key::KeyZ,
        KeyCode::Minus => Key::Minus,
        KeyCode::Period => Key::Period,
        KeyCode::Quote => Key::Quote,
        KeyCode::Semicolon => Key::Semicolon,
        KeyCode::Slash => Key::Slash,
        KeyCode::AltLeft => Key::AltLeft,
        KeyCode::AltRight => Key::AltRight,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::CapsLock => Key::CapsLock,
        KeyCode::ContextMenu => Key::ContextMenu,
        KeyCode::ControlLeft => Key::ControlLeft,
        KeyCode::ControlRight => Key::ControlRight,
        KeyCode::Enter => Key::Enter,
        KeyCode::SuperLeft => Key::SuperLeft,
        KeyCode::SuperRight => Key::SuperRight,
        KeyCode::ShiftLeft => Key::ShiftLeft,
        KeyCode::ShiftRight => Key::ShiftRight,
        KeyCode::Space => Key::Space,
        KeyCode::Tab => Key::Tab,
        KeyCode::Convert => Key::Convert,
        KeyCode::KanaMode => Key::KanaMode,
        KeyCode::Lang1 => Key::Lang1,
        KeyCode::Lang2 => Key::Lang2,
        KeyCode::Lang3 => Key::Lang3,
        KeyCode::Lang4 => Key::Lang4,
        KeyCode::Lang5 => Key::Lang5,
        KeyCode::NonConvert => Key::NonConvert,
        KeyCode::Delete => Key::Delete,
        KeyCode::End => Key::End,
        KeyCode::Help => Key::Help,
        KeyCode::Home => Key::Home,
        KeyCode::Insert => Key::Insert,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::ArrowDown => Key::ArrowDown,
        KeyCode::ArrowLeft => Key::ArrowLeft,
        KeyCode::ArrowRight => Key::ArrowRight,
        KeyCode::ArrowUp => Key::ArrowUp,
        KeyCode::NumLock => Key::NumLock,
        KeyCode::Numpad0 => Key::Numpad0,
        KeyCode::Numpad1 => Key::Numpad1,
        KeyCode::Numpad2 => Key::Numpad2,
        KeyCode::Numpad3 => Key::Numpad3,
        KeyCode::Numpad4 => Key::Numpad4,
        KeyCode::Numpad5 => Key::Numpad5,
        KeyCode::Numpad6 => Key::Numpad6,
        KeyCode::Numpad7 => Key::Numpad7,
        KeyCode::Numpad8 => Key::Numpad8,
        KeyCode::Numpad9 => Key::Numpad9,
        KeyCode::NumpadAdd => Key::NumpadAdd,
        KeyCode::NumpadBackspace => Key::NumpadBackspace,
        KeyCode::NumpadClear => Key::NumpadClear,
        KeyCode::NumpadClearEntry => Key::NumpadClearEntry,
        KeyCode::NumpadComma => Key::NumpadComma,
        KeyCode::NumpadDecimal => Key::NumpadDecimal,
        KeyCode::NumpadDivide => Key::NumpadDivide,
        KeyCode::NumpadEnter => Key::NumpadEnter,
        KeyCode::NumpadEqual => Key::NumpadEqual,
        KeyCode::NumpadHash => Key::NumpadHash,
        KeyCode::NumpadMemoryAdd => Key::NumpadMemoryAdd,
        KeyCode::NumpadMemoryClear => Key::NumpadMemoryClear,
        KeyCode::NumpadMemoryRecall => Key::NumpadMemoryRecall,
        KeyCode::NumpadMemoryStore => Key::NumpadMemoryStore,
        KeyCode::NumpadMemorySubtract => Key::NumpadMemorySubtract,
        KeyCode::NumpadMultiply => Key::NumpadMultiply,
        KeyCode::NumpadParenLeft => Key::NumpadParenLeft,
        KeyCode::NumpadParenRight => Key::NumpadParenRight,
        KeyCode::NumpadStar => Key::NumpadStar,
        KeyCode::NumpadSubtract => Key::NumpadSubtract,
        KeyCode::Escape => Key::Escape,
        KeyCode::Fn => Key::Fn,
        KeyCode::FnLock => Key::FnLock,
        KeyCode::PrintScreen => Key::PrintScreen,
        KeyCode::ScrollLock => Key::ScrollLock,
        KeyCode::Pause => Key::Pause,
        KeyCode::BrowserBack => Key::BrowserBack,
        KeyCode::BrowserFavorites => Key::BrowserFavorites,
        KeyCode::BrowserForward => Key::BrowserForward,
        KeyCode::BrowserHome => Key::BrowserHome,
        KeyCode::BrowserRefresh => Key::BrowserRefresh,
        KeyCode::BrowserSearch => Key::BrowserSearch,
        KeyCode::BrowserStop => Key::BrowserStop,
        KeyCode::Eject => Key::Eject,
        KeyCode::LaunchApp1 => Key::LaunchApp1,
        KeyCode::LaunchApp2 => Key::LaunchApp2,
        KeyCode::LaunchMail => Key::LaunchMail,
        KeyCode::MediaPlayPause => Key::MediaPlayPause,
        KeyCode::MediaSelect => Key::MediaSelect,
        KeyCode::MediaStop => Key::MediaStop,
        KeyCode::MediaTrackNext => Key::MediaTrackNext,
        KeyCode::MediaTrackPrevious => Key::MediaTrackPrevious,
        KeyCode::Power => Key::Power,
        KeyCode::Sleep => Key::Sleep,
        KeyCode::AudioVolumeDown => Key::AudioVolumeDown,
        KeyCode::AudioVolumeMute => Key::AudioVolumeMute,
        KeyCode::AudioVolumeUp => Key::AudioVolumeUp,
        KeyCode::WakeUp => Key::WakeUp,
        KeyCode::Meta => Key::Meta,
        KeyCode::Hyper => Key::Hyper,
        KeyCode::Turbo => Key::Turbo,
        KeyCode::Abort => Key::Abort,
        KeyCode::Resume => Key::Resume,
        KeyCode::Suspend => Key::Suspend,
        KeyCode::Again => Key::Again,
        KeyCode::Copy => Key::Copy,
        KeyCode::Cut => Key::Cut,
        KeyCode::Find => Key::Find,
        KeyCode::Open => Key::Open,
        KeyCode::Paste => Key::Paste,
        KeyCode::Props => Key::Props,
        KeyCode::Select => Key::Select,
        KeyCode::Undo => Key::Undo,
        KeyCode::Hiragana => Key::Hiragana,
        KeyCode::Katakana => Key::Katakana,
        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,
        KeyCode::F13 => Key::F13,
        KeyCode::F14 => Key::F14,
        KeyCode::F15 => Key::F15,
        KeyCode::F16 => Key::F16,
        KeyCode::F17 => Key::F17,
        KeyCode::F18 => Key::F18,
        KeyCode::F19 => Key::F19,
        KeyCode::F20 => Key::F20,
        KeyCode::F21 => Key::F21,
        KeyCode::F22 => Key::F22,
        KeyCode::F23 => Key::F23,
        KeyCode::F24 => Key::F24,
        KeyCode::F25 => Key::F25,
        KeyCode::F26 => Key::F26,
        KeyCode::F27 => Key::F27,
        KeyCode::F28 => Key::F28,
        KeyCode::F29 => Key::F29,
        KeyCode::F30 => Key::F30,
        KeyCode::F31 => Key::F31,
        KeyCode::F32 => Key::F32,
        KeyCode::F33 => Key::F33,
        KeyCode::F34 => Key::F34,
        KeyCode::F35 => Key::F35,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quasar_input::{Binding, Bindings};
    #[test]
    fn keyboard_auto_repeat_preserves_text_but_not_gameplay_edges() {
        let mut input = InputState::default();
        let mut bindings = Bindings::default();
        bindings.bind("fire", Binding::key(Key::KeyW));
        input.set_bindings(bindings).unwrap();
        process_key_event(
            &mut input,
            PhysicalKey::Code(KeyCode::KeyW),
            ElementState::Pressed,
            false,
            Some("w"),
        );
        assert!(input.action("fire").pressed);
        assert_eq!(input.text, "w");
        input.consume_edges();
        for _ in 0..3 {
            process_key_event(
                &mut input,
                PhysicalKey::Code(KeyCode::KeyW),
                ElementState::Pressed,
                true,
                Some("w"),
            );
        }
        // Repeat text reaches chat/console fields, but the physical press and
        // action edge remain consumed while the key is held.
        assert_eq!(input.text, "www");
        assert!(input.action("fire").held());
        assert!(!input.action("fire").pressed);
        assert!(!input.pressed(Key::KeyW));
        process_key_event(
            &mut input,
            PhysicalKey::Code(KeyCode::KeyW),
            ElementState::Released,
            false,
            Some("ignored"),
        );
        assert!(!input.action("fire").held());
        assert!(input.action("fire").released);
        assert_eq!(input.text, "www");
        input.consume_edges();
        assert!(input.text.is_empty());
    }

    #[test]
    fn ime_commit_updates_text_without_creating_a_key_edge() {
        let mut input = InputState::default();
        process_window_event(&mut input, &WindowEvent::Ime(Ime::Commit("chat".into())));
        assert_eq!(input.text, "chat");
        input.consume_edges();
        assert!(input.text.is_empty());
    }

    #[test]
    fn adapter_preserves_focus_release_and_separates_wheel_units() {
        let mut input = InputState::default();
        let mut bindings = Bindings::default();
        bindings.bind("move", Binding::key(Key::KeyW));
        input.set_bindings(bindings).unwrap();
        input.key(Key::KeyW, true);
        process_device_event(
            &mut input,
            &DeviceEvent::MouseMotion { delta: (3.0, 4.0) },
            false,
        );
        assert_eq!(input.relative_motion, (0.0, 0.0));
        process_device_event(
            &mut input,
            &DeviceEvent::MouseMotion { delta: (3.0, 4.0) },
            true,
        );
        assert_eq!(input.relative_motion, (3.0, 4.0));
        let scroll = |delta| WindowEvent::MouseWheel {
            device_id: winit::event::DeviceId::dummy(),
            delta,
            phase: winit::event::TouchPhase::Moved,
        };
        process_window_event(&mut input, &scroll(MouseScrollDelta::LineDelta(1.0, 2.0)));
        process_window_event(
            &mut input,
            &scroll(MouseScrollDelta::PixelDelta(
                winit::dpi::PhysicalPosition::new(5.0, 6.0),
            )),
        );
        assert_eq!(input.scroll_lines, (1.0, 2.0));
        assert_eq!(input.scroll_pixels, (5.0, 6.0));
        process_window_event(&mut input, &WindowEvent::Focused(false));
        assert!(!input.action("move").held());
        assert!(!input.action("move").pressed);
        assert!(input.action("move").released);
        assert_eq!(physical_key(KeyCode::KeyW), Some(Key::KeyW));
    }
}
