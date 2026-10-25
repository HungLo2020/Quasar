use quasar_ui::{
    Edges, Layer, LayoutItem, Length, MonospaceMeasure, PanelLayout, Rect, Theme, Ui, UiInput,
    Widget,
};

#[cfg(feature = "input")]
#[test]
fn quasar_input_adapter_maps_navigation_mouse_and_text() {
    use quasar_input::{Binding, Bindings, InputState, Key, MouseButton};
    use quasar_ui::{UiBindings, from_input_state};

    let mut bindings = Bindings::default();
    bindings
        .bind("ui_down", Binding::key(Key::ArrowDown))
        .bind("ui_activate", Binding::key(Key::Enter));
    let mut input = InputState::default();
    input.set_bindings(bindings).unwrap();
    input.key(Key::ArrowDown, true);
    input.key(Key::Enter, true);
    input.mouse_button(MouseButton::Left, true);
    input.cursor_physical = Some((12.0, 34.0));
    input.text = "hello".into();
    input.scroll_lines = (1.0, 2.0);
    let mapped = from_input_state(&input, &UiBindings::default());
    assert!(mapped.down);
    assert!(mapped.activate);
    assert!(mapped.primary_pressed);
    assert_eq!(mapped.pointer, Some(quasar_ui::Vec2::new(12.0, 34.0)));
    assert_eq!(mapped.text, "hello");
    assert_eq!(mapped.scroll, quasar_ui::Vec2::new(1.0, 2.0));
}

#[cfg(feature = "config")]
#[test]
fn config_values_convert_to_ui_values() {
    use quasar_config::ConfigValue;
    use quasar_ui::{UiValue, config_to_value, value_to_config};

    assert_eq!(
        config_to_value(&ConfigValue::Bool(true)),
        UiValue::Bool(true)
    );
    assert_eq!(
        config_to_value(&ConfigValue::Float(1.5)),
        UiValue::Float(1.5)
    );
    assert_eq!(
        config_to_value(&ConfigValue::String("x".into())),
        UiValue::Text("x".into())
    );
    assert_eq!(value_to_config(&UiValue::UInt(7)), ConfigValue::Unsigned(7));
}

#[test]
fn layout_and_interaction_sequences_are_finite_and_bounded() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Stack {
                direction: quasar_ui::StackDirection::Vertical,
                gap: 4.0,
            },
            padding: Edges::all(8.0),
            clip: true,
            scrollable: true,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    for index in 0..40 {
        ui.add_child(
            root,
            Widget::Button {
                text: quasar_ui::UiText::literal(format!("B{index}")),
                action: Some(quasar_ui::ActionId::new(index)),
            },
            LayoutItem::size(Length::Fill, Length::fixed(20.0)),
        )
        .unwrap();
    }
    let mut seed = 0x1234u64;
    for _ in 0..500 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let x = (seed >> 32) as f32 / 100.0;
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let y = (seed >> 32) as f32 / 100.0;
        let input = UiInput {
            pointer: Some(quasar_ui::Vec2::new(x, y)),
            primary_pressed: seed.is_multiple_of(13),
            primary_down: seed % 13 < 3,
            scroll: quasar_ui::Vec2::new(0.0, if seed.is_multiple_of(17) { 1.0 } else { 0.0 }),
            next: seed.is_multiple_of(19),
            previous: seed.is_multiple_of(23),
            activate: seed.is_multiple_of(29),
            ..UiInput::default()
        };
        let frame = ui.update(
            &input,
            Rect::new(0.0, 0.0, 640.0, 360.0),
            1.0,
            Edges::ZERO,
            &MonospaceMeasure,
            1.0 / 60.0,
        );
        assert!(frame.events.len() < 100);
        for entry in ui.layout_entries() {
            assert!(entry.rect.valid());
            assert!(entry.clip.valid());
        }
    }
    assert!(!ui.focus_order().is_empty());
}
