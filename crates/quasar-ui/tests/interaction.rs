use quasar_ui::{
    ActionId, Edges, Layer, LayoutItem, Length, MonospaceMeasure, NodeId, PanelLayout, Rect,
    StackDirection, Theme, Ui, UiEvent, UiFrame, UiInput, UiText, UiValue, Vec2, Widget,
};

fn update(ui: &mut Ui, input: UiInput) -> UiFrame {
    ui.update(
        &input,
        Rect::new(0.0, 0.0, 400.0, 300.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        1.0 / 60.0,
    )
}

fn menu() -> (Ui, NodeId, NodeId) {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Stack {
                direction: StackDirection::Vertical,
                gap: 4.0,
            },
            padding: Edges::ZERO,
            clip: true,
            scrollable: false,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    let mut button = |name, action| {
        ui.add_child(
            root,
            Widget::Button {
                text: UiText::literal(name),
                action: Some(ActionId(action)),
            },
            LayoutItem::size(Length::fixed(100.0), Length::fixed(30.0)),
        )
        .unwrap()
    };
    let a = button("A", 1);
    let b = button("B", 2);
    update(&mut ui, UiInput::default());
    (ui, a, b)
}

fn center(ui: &Ui, id: NodeId) -> Vec2 {
    ui.layout_entries()
        .iter()
        .find(|entry| entry.id == id)
        .unwrap()
        .rect
        .center()
}

#[test]
fn selection_switches_immediately_and_stationary_mouse_does_not_override_keyboard() {
    let (mut ui, a, b) = menu();
    let pointer = Some(center(&ui, a));
    update(
        &mut ui,
        UiInput {
            pointer,
            ..UiInput::default()
        },
    );
    update(
        &mut ui,
        UiInput {
            pointer,
            down: true,
            ..UiInput::default()
        },
    );
    assert_eq!(ui.focused(), Some(b));
    assert_eq!(ui.node(a).unwrap().state.focus_t, 0.0);
    assert_eq!(ui.node(b).unwrap().state.focus_t, 1.0);
    assert!(!ui.node(a).unwrap().state.hovered);
    for _ in 0..10 {
        update(
            &mut ui,
            UiInput {
                pointer,
                ..UiInput::default()
            },
        );
        assert_eq!(ui.focused(), Some(b));
        assert!(!ui.node(a).unwrap().state.hovered);
    }
    let pos = center(&ui, a);
    let moved = Some(Vec2::new(pos.x + 1.0, pos.y));
    update(
        &mut ui,
        UiInput {
            pointer: moved,
            ..UiInput::default()
        },
    );
    assert_eq!(ui.focused(), Some(a));
    assert_eq!(ui.node(b).unwrap().state.focus_t, 0.0);
    assert!(ui.node(a).unwrap().state.hovered);
}

#[test]
fn button_release_requires_the_original_target_and_can_be_cancelled() {
    let (mut ui, a, b) = menu();
    let pointer = Some(center(&ui, a));
    let press = update(
        &mut ui,
        UiInput {
            pointer,
            primary_pressed: true,
            primary_down: true,
            ..UiInput::default()
        },
    );
    assert!(!press.events.contains(&UiEvent::Pressed(ActionId(1))));
    assert!(ui.node(a).unwrap().state.pressed);
    let other = Some(center(&ui, b));
    let drag = update(
        &mut ui,
        UiInput {
            pointer: other,
            primary_down: true,
            ..UiInput::default()
        },
    );
    assert!(drag.consumed_pointer);
    assert!(!ui.node(a).unwrap().state.pressed);
    let release = update(
        &mut ui,
        UiInput {
            pointer: other,
            primary_released: true,
            ..UiInput::default()
        },
    );
    assert!(release.consumed_pointer);
    assert!(
        !release
            .events
            .iter()
            .any(|event| matches!(event, UiEvent::Pressed(_)))
    );
    // A release without a preceding press cannot activate a button.
    let release = update(
        &mut ui,
        UiInput {
            pointer,
            primary_released: true,
            ..UiInput::default()
        },
    );
    assert!(!release.events.contains(&UiEvent::Pressed(ActionId(1))));
    // Returning inside before release succeeds once.
    update(
        &mut ui,
        UiInput {
            pointer,
            primary_pressed: true,
            primary_down: true,
            ..UiInput::default()
        },
    );
    update(
        &mut ui,
        UiInput {
            pointer: None,
            primary_down: true,
            ..UiInput::default()
        },
    );
    let release = update(
        &mut ui,
        UiInput {
            pointer,
            primary_released: true,
            ..UiInput::default()
        },
    );
    assert_eq!(
        release
            .events
            .iter()
            .filter(|event| **event == UiEvent::Pressed(ActionId(1)))
            .count(),
        1
    );
    assert!(!ui.node(a).unwrap().state.pressed);
}

#[test]
fn disabling_or_hiding_a_pressed_button_cancels_activation() {
    for hide in [false, true] {
        let (mut ui, a, _) = menu();
        let pointer = Some(center(&ui, a));
        update(
            &mut ui,
            UiInput {
                pointer,
                primary_pressed: true,
                primary_down: true,
                ..UiInput::default()
            },
        );
        if hide {
            ui.set_visible(a, false).unwrap();
        } else {
            ui.set_enabled(a, false).unwrap();
        }
        let release = update(
            &mut ui,
            UiInput {
                pointer,
                primary_released: true,
                ..UiInput::default()
            },
        );
        assert!(!release.events.contains(&UiEvent::Pressed(ActionId(1))));
    }
}

#[test]
fn clicking_a_tab_selects_its_position_instead_of_cycling() {
    let mut ui = Ui::new(Theme::modern());
    let tabs = ui.add_root(
        Widget::Tabs {
            tabs: ["A", "B", "C"].map(UiText::literal).to_vec(),
            selected: 0,
            action: Some(ActionId(7)),
        },
        LayoutItem::size(Length::fixed(300.0), Length::fixed(30.0)),
        Layer::Normal,
        false,
    );
    update(&mut ui, UiInput::default());
    for (x, expected) in [(250.0, 2), (50.0, 0), (150.0, 1)] {
        let frame = update(
            &mut ui,
            UiInput {
                pointer: Some(Vec2::new(x, 15.0)),
                primary_pressed: true,
                primary_down: true,
                ..UiInput::default()
            },
        );
        assert!(frame.events.contains(&UiEvent::Changed {
            id: tabs,
            value: UiValue::UInt(expected)
        }));
        assert!(frame.events.contains(&UiEvent::Pressed(ActionId(7))));
        let repeat = update(
            &mut ui,
            UiInput {
                pointer: Some(Vec2::new(x, 15.0)),
                primary_pressed: true,
                primary_down: true,
                ..UiInput::default()
            },
        );
        assert!(
            !repeat
                .events
                .iter()
                .any(|event| matches!(event, UiEvent::Changed { .. }))
        );
    }
}

#[test]
fn hovering_menu_choices_does_not_steal_text_editing_focus() {
    let (mut ui, a, _) = menu();
    let text = ui.add_root(
        Widget::TextInput {
            text: String::new(),
            placeholder: UiText::literal("Name"),
            cursor: 0,
            anchor: 0,
            max_len: 32,
            password: false,
            action: None,
        },
        LayoutItem::size(Length::fixed(100.0), Length::fixed(25.0))
            .with_offset(Vec2::new(200.0, 100.0)),
        Layer::Normal,
        false,
    );
    ui.focus(Some(text));
    let pointer = Some(center(&ui, a));
    update(
        &mut ui,
        UiInput {
            pointer,
            text: "hello".into(),
            ..UiInput::default()
        },
    );
    update(
        &mut ui,
        UiInput {
            pointer: Some(Vec2::new(40.0, 15.0)),
            ..UiInput::default()
        },
    );
    assert_eq!(ui.focused(), Some(text));
    assert!(
        matches!(&ui.node(text).unwrap().widget, Widget::TextInput { text, .. } if text == "hello")
    );
}

#[cfg(feature = "input")]
#[test]
fn consumed_menu_frames_keep_stationary_clicks_and_slider_drags_working() {
    use quasar_input::{InputState, MouseButton};
    use quasar_ui::{UiBindings, from_frame_input};
    let (mut ui, a, _) = menu();
    let mut state = InputState::default();
    let pos = center(&ui, a);
    state.cursor_physical = Some((pos.x as f64, pos.y as f64));
    let mut step = |state: &mut InputState| {
        let mut input = state.frame(1);
        let frame = update(&mut ui, from_frame_input(&input, &UiBindings::default()));
        input.consume_all();
        // The runner applies consumption after frame_update and before simulation.
        assert_eq!(input.cursor_physical(), None);
        assert!(!input.mouse_held(MouseButton::Left));
        input.apply_consumption(state);
        frame
    };
    for _ in 0..5 {
        step(&mut state);
    }
    state.mouse_button(MouseButton::Left, true);
    assert!(
        !step(&mut state)
            .events
            .contains(&UiEvent::Pressed(ActionId(1)))
    );
    for _ in 0..5 {
        step(&mut state);
    }
    state.mouse_button(MouseButton::Left, false);
    assert!(
        step(&mut state)
            .events
            .contains(&UiEvent::Pressed(ActionId(1)))
    );

    let mut ui = Ui::new(Theme::retro());
    let slider = ui.add_root(
        Widget::Slider {
            label: UiText::literal("Volume"),
            value: 0.0,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            action: None,
        },
        LayoutItem::size(Length::fixed(200.0), Length::fixed(30.0)),
        Layer::Normal,
        false,
    );
    state.cursor_physical = Some((20.0, 15.0));
    state.mouse_button(MouseButton::Left, true);
    for x in [20.0, 60.0, 100.0, 180.0, 250.0] {
        state.cursor_physical = Some((x, 15.0));
        let mut input = state.frame(1);
        let frame = update(&mut ui, from_frame_input(&input, &UiBindings::default()));
        assert!(frame.consumed_pointer);
        input.consume_all();
        input.apply_consumption(&mut state);
    }
    assert!(
        matches!(&ui.node(slider).unwrap().widget, Widget::Slider { value, .. } if *value == 1.0)
    );
    state.mouse_button(MouseButton::Left, false);
    let input = state.frame(1);
    let release = update(&mut ui, from_frame_input(&input, &UiBindings::default()));
    assert!(release.consumed_pointer);
    assert!(!ui.node(slider).unwrap().state.pressed);
}
