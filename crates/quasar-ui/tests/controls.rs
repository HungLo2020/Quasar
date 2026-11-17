//! Controls screen: listing, capture, axis rebinds, conflicts, reset and
//! persistence.
#![cfg(feature = "input")]
use quasar_input::{Binding, Bindings, Button, DeviceFamily, GamepadButton, InputDevice, Key};
use quasar_ui::{
    ActionId, ControlEntry, ControlsOutcome, ControlsScreen, Edges, Layer, LayoutItem,
    MonospaceMeasure, PanelLayout, Rect, StackDirection, Theme, Ui, UiEvent, UiText, Widget,
};

fn defaults() -> Bindings {
    let mut b = Bindings::default();
    b.bind("move_x", Binding::axis(Key::KeyA, Key::KeyD))
        .bind("jump", Binding::key(Key::Space))
        .bind("jump", Binding::gamepad_button(GamepadButton::South))
        .bind("use", Binding::key(Key::KeyE));
    b
}

fn setup(saved: &Bindings) -> (Ui, ControlsScreen, Vec<quasar_ui::NodeId>) {
    let mut ui = Ui::new(Theme::modern());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Stack {
                direction: StackDirection::Vertical,
                gap: 0.,
            },
            padding: Edges::ZERO,
            clip: false,
            scrollable: false,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    let mut screen = ControlsScreen::new(
        vec![
            ControlEntry::new("move_x", UiText::literal("Move")),
            ControlEntry::new("jump", UiText::literal("Jump")),
            ControlEntry::new("use", UiText::literal("Use")),
        ],
        saved,
        defaults(),
    );
    let panel = screen.build(&mut ui, root, ActionId::new(99)).unwrap();
    ui.layout(
        Rect::new(0., 0., 640., 480.),
        1.,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    let rows: Vec<_> = ui.node(panel).unwrap().children[..3].to_vec();
    (ui, screen, rows)
}

fn value(ui: &Ui, row: quasar_ui::NodeId) -> String {
    match &ui.node(row).unwrap().widget {
        Widget::Binding { value, .. } => value.clone(),
        _ => unreachable!(),
    }
}

fn request(ui: &mut Ui, screen: &mut ControlsScreen, row: quasar_ui::NodeId) -> ControlsOutcome {
    ui.begin_rebind(row).unwrap();
    screen
        .handle_event(
            ui,
            &UiEvent::RebindRequested {
                id: row,
                action: String::new(),
            },
        )
        .unwrap()
}

#[test]
fn rows_show_bindings_and_captures_rebind_buttons_and_axes() {
    let (mut ui, mut screen, rows) = setup(&Bindings::default());
    assert_eq!(value(&ui, rows[0]), "A / D");
    assert_eq!(value(&ui, rows[1]), "Space");
    assert!(matches!(
        request(&mut ui, &mut screen, rows[2]),
        ControlsOutcome::Capturing { .. }
    ));
    assert_eq!(value(&ui, rows[2]), "Press a button…");
    // Gamepad buttons are ignored while editing the keyboard column.
    let none = screen
        .capture(&mut ui, &[Button::Gamepad(GamepadButton::North)])
        .unwrap();
    assert_eq!(none, ControlsOutcome::None);
    let rebound = screen.capture(&mut ui, &[Button::Key(Key::KeyF)]).unwrap();
    assert_eq!(
        rebound,
        ControlsOutcome::Rebound {
            action: "use".into(),
            binding: Binding::key(Key::KeyF)
        }
    );
    assert_eq!(value(&ui, rows[2]), "F");
    assert!(!ui.is_capturing());
    // Axes take two presses: negative, then positive.
    request(&mut ui, &mut screen, rows[0]);
    let first = screen
        .capture(&mut ui, &[Button::Key(Key::ArrowLeft)])
        .unwrap();
    assert!(matches!(first, ControlsOutcome::CapturingSecond { .. }));
    screen
        .capture(&mut ui, &[Button::Key(Key::ArrowRight)])
        .unwrap();
    assert_eq!(
        screen.bindings().actions["move_x"],
        vec![Binding::Axis {
            negative: Button::Key(Key::ArrowLeft),
            positive: Button::Key(Key::ArrowRight),
        }]
    );
    // The gamepad column keeps its own binding.
    assert!(
        screen.bindings().actions["jump"].contains(&Binding::gamepad_button(GamepadButton::South))
    );
    screen
        .set_device(&mut ui, InputDevice::Gamepad, DeviceFamily::Xbox)
        .unwrap();
    assert_eq!(value(&ui, rows[1]), "A");
    assert_eq!(value(&ui, rows[2]), "—");
}

#[test]
fn conflicts_are_flagged_and_reset_or_escape_recover() {
    let (mut ui, mut screen, rows) = setup(&Bindings::default());
    request(&mut ui, &mut screen, rows[2]);
    screen.capture(&mut ui, &[Button::Key(Key::Space)]).unwrap();
    let conflicts = screen.conflicts();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].actions, vec!["jump", "use"]);
    assert!(
        ui.node(rows[1]).unwrap().style.is_some(),
        "conflicting rows are marked"
    );
    assert!(ui.node(rows[0]).unwrap().style.is_none());
    // Escape cancels a capture without changing anything.
    request(&mut ui, &mut screen, rows[1]);
    assert_eq!(
        screen
            .capture(&mut ui, &[Button::Key(Key::Escape)])
            .unwrap(),
        ControlsOutcome::Cancelled
    );
    assert_eq!(value(&ui, rows[1]), "Space");
    assert!(!ui.is_capturing());
    let reset = screen
        .handle_event(&mut ui, &UiEvent::Pressed(ActionId::new(99)))
        .unwrap();
    assert_eq!(reset, ControlsOutcome::Reset);
    assert!(screen.conflicts().is_empty());
    assert_eq!(value(&ui, rows[2]), "E");
}

#[test]
fn saved_bindings_merge_over_new_defaults() {
    let mut saved = Bindings::default();
    saved.bind("jump", Binding::key(Key::KeyW));
    saved.bind("removed_action", Binding::key(Key::KeyQ));
    let (ui, screen, rows) = setup(&saved);
    assert_eq!(value(&ui, rows[1]), "W", "saved binding wins");
    assert_eq!(value(&ui, rows[2]), "E", "new action gets its default");
    assert!(!screen.bindings().actions.contains_key("removed_action"));
}

#[cfg(feature = "controls-file")]
#[test]
fn bindings_round_trip_through_toml_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("controls.toml");
    assert_eq!(
        quasar_ui::load_bindings(&path, &defaults()).unwrap(),
        defaults()
    );
    let mut changed = defaults();
    changed
        .actions
        .insert("use".into(), vec![Binding::key(Key::KeyR)]);
    quasar_ui::save_bindings(&path, &changed).unwrap();
    assert_eq!(
        quasar_ui::load_bindings(&path, &defaults()).unwrap(),
        changed
    );
    std::fs::write(&path, "not = [valid").unwrap();
    assert!(quasar_ui::load_bindings(&path, &defaults()).is_err());
}
