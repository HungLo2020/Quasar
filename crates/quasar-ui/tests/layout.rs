use quasar_ui::{
    ActionId, Align, Edges, Layer, LayoutItem, Length, MonospaceMeasure, PanelLayout, Rect,
    StackDirection, Theme, Ui, UiEvent, UiInput, UiText, Widget,
};

fn panel() -> Widget {
    Widget::Panel {
        layout: PanelLayout::Stack {
            direction: StackDirection::Vertical,
            gap: 4.0,
        },
        padding: Edges::all(8.0),
        clip: true,
        scrollable: false,
    }
}

#[test]
fn stack_layout_places_children_and_respects_dpi_and_safe_area() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(panel(), LayoutItem::fill(), Layer::Normal, false);
    let label = ui
        .add_child(
            root,
            Widget::Label {
                text: UiText::literal("Hello"),
                style: quasar_ui::TextStyle::Body,
            },
            LayoutItem::auto(),
        )
        .unwrap();
    let button = ui
        .add_child(
            root,
            Widget::Button {
                text: UiText::literal("Play"),
                action: Some(ActionId::new(1)),
            },
            LayoutItem::size(Length::Fill, Length::fixed(30.0)),
        )
        .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 400.0, 300.0),
        2.0,
        Edges::new(10.0, 20.0, 10.0, 20.0),
        &MonospaceMeasure,
    );
    let root_rect = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == root)
        .unwrap()
        .rect;
    assert_eq!(root_rect.x, 20.0);
    assert_eq!(root_rect.y, 40.0);
    let label_rect = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == label)
        .unwrap()
        .rect;
    let button_rect = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == button)
        .unwrap()
        .rect;
    assert_eq!(button_rect.height, 60.0);
    assert_eq!(button_rect.width, root_rect.width - 32.0);
    assert!(button_rect.y >= label_rect.y + label_rect.height);
}

#[test]
fn grid_and_absolute_layouts_are_bounded_and_aligned() {
    let mut ui = Ui::new(Theme::modern());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Grid {
                columns: 2,
                gap: 10.0,
            },
            padding: Edges::all(10.0),
            clip: true,
            scrollable: false,
        },
        LayoutItem::fill(),
        Layer::Normal,
        false,
    );
    for index in 0..4 {
        ui.add_child(
            root,
            Widget::Button {
                text: UiText::literal(format!("B{index}")),
                action: None,
            },
            LayoutItem::size(Length::Fill, Length::fixed(24.0)).with_align(Align::Center),
        )
        .unwrap();
    }
    ui.layout(
        Rect::new(0.0, 0.0, 420.0, 240.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    let entries: Vec<_> = ui
        .layout_entries()
        .iter()
        .filter(|entry| entry.id != root)
        .collect();
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().all(|entry| entry.rect.x >= 10.0));
    assert!(entries.iter().all(|entry| entry.rect.width > 0.0));
    assert!(entries[0].rect.x < entries[1].rect.x);
    assert_eq!(entries[0].rect.y, entries[1].rect.y);
}

#[test]
fn scrollable_panels_clamp_offsets_and_clip_children() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(
        Widget::Panel {
            layout: PanelLayout::Stack {
                direction: StackDirection::Vertical,
                gap: 2.0,
            },
            padding: Edges::all(4.0),
            clip: true,
            scrollable: true,
        },
        LayoutItem::size(Length::fixed(200.0), Length::fixed(100.0)),
        Layer::Normal,
        false,
    );
    for index in 0..20 {
        ui.add_child(
            root,
            Widget::Label {
                text: UiText::literal(format!("Row {index}")),
                style: quasar_ui::TextStyle::Small,
            },
            LayoutItem::size(Length::Fill, Length::fixed(20.0)),
        )
        .unwrap();
    }
    ui.layout(
        Rect::new(0.0, 0.0, 320.0, 240.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    ui.set_scroll(root, quasar_ui::Vec2::new(0.0, 500.0))
        .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 320.0, 240.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    let root_entry = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == root)
        .unwrap();
    assert!(root_entry.clip.height <= 100.0);
    let scrolled = ui.node(root).unwrap().scroll.y;
    assert!(scrolled > 0.0);
}

#[test]
fn buttons_emit_actions_from_keyboard_and_mouse() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(panel(), LayoutItem::fill(), Layer::Normal, false);
    let button = ui
        .add_child(
            root,
            Widget::Button {
                text: UiText::literal("Go"),
                action: Some(ActionId::new(9)),
            },
            LayoutItem::size(Length::fixed(100.0), Length::fixed(30.0)),
        )
        .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 300.0, 200.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    let mut input = UiInput {
        next: true,
        ..UiInput::default()
    };
    let frame = ui.update(
        &input,
        Rect::new(0.0, 0.0, 300.0, 200.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        0.016,
    );
    assert_eq!(ui.focused(), Some(button));
    assert!(!frame.events.is_empty() || frame.consumed);
    input = UiInput {
        activate: true,
        ..UiInput::default()
    };
    let frame = ui.update(
        &input,
        Rect::new(0.0, 0.0, 300.0, 200.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        0.016,
    );
    assert!(
        frame
            .events
            .iter()
            .any(|event| *event == UiEvent::Pressed(ActionId::new(9)))
    );

    let rect = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == button)
        .unwrap()
        .rect;
    let frame = ui.update(
        &UiInput {
            pointer: Some(quasar_ui::Vec2::new(rect.center().x, rect.center().y)),
            primary_pressed: true,
            primary_down: true,
            ..UiInput::default()
        },
        Rect::new(0.0, 0.0, 300.0, 200.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        0.016,
    );
    assert!(frame.consumed_pointer);
    assert!(!frame.events.contains(&UiEvent::Pressed(ActionId::new(9))));
    let frame = ui.update(
        &UiInput {
            pointer: Some(rect.center()),
            primary_released: true,
            ..UiInput::default()
        },
        Rect::new(0.0, 0.0, 300.0, 200.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        0.016,
    );
    assert!(frame.consumed_pointer);
    assert!(frame.events.contains(&UiEvent::Pressed(ActionId::new(9))));
}

#[test]
fn sliders_checkboxes_lists_and_text_fields_interact() {
    let mut ui = Ui::new(Theme::retro());
    let root = ui.add_root(panel(), LayoutItem::fill(), Layer::Normal, false);
    let slider = ui
        .add_child(
            root,
            Widget::Slider {
                label: UiText::literal("Volume"),
                value: 0.0,
                min: 0.0,
                max: 10.0,
                step: 1.0,
                action: Some(ActionId::new(1)),
            },
            LayoutItem::size(Length::Fill, Length::fixed(24.0)),
        )
        .unwrap();
    let checkbox = ui
        .add_child(
            root,
            Widget::Checkbox {
                label: UiText::literal("Enabled"),
                checked: false,
                action: Some(ActionId::new(2)),
            },
            LayoutItem::size(Length::Fill, Length::fixed(24.0)),
        )
        .unwrap();
    let text = ui
        .add_child(
            root,
            Widget::TextInput {
                text: String::new(),
                placeholder: UiText::literal("Name"),
                cursor: 0,
                anchor: 0,
                max_len: 12,
                password: false,
                action: None,
            },
            LayoutItem::size(Length::Fill, Length::fixed(24.0)),
        )
        .unwrap();
    ui.layout(
        Rect::new(0.0, 0.0, 320.0, 300.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    let slider_rect = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == slider)
        .unwrap()
        .rect;
    ui.update(
        &UiInput {
            pointer: Some(quasar_ui::Vec2::new(
                slider_rect.right() * 0.0 + slider_rect.x + slider_rect.width * 0.8,
                slider_rect.center().y,
            )),
            primary_pressed: true,
            primary_down: true,
            ..UiInput::default()
        },
        Rect::new(0.0, 0.0, 320.0, 300.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        0.016,
    );
    let value = match &ui.node(slider).unwrap().widget {
        Widget::Slider { value, .. } => *value,
        _ => panic!(),
    };
    assert!(value > 5.0);

    let check_rect = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == checkbox)
        .unwrap()
        .rect;
    ui.update(
        &UiInput {
            pointer: Some(check_rect.center()),
            primary_pressed: true,
            primary_down: true,
            ..UiInput::default()
        },
        Rect::new(0.0, 0.0, 320.0, 300.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        0.016,
    );
    assert!(matches!(
        ui.node(checkbox).unwrap().widget,
        Widget::Checkbox { checked: true, .. }
    ));

    let text_rect = ui
        .layout_entries()
        .iter()
        .find(|entry| entry.id == text)
        .unwrap()
        .rect;
    ui.update(
        &UiInput {
            pointer: Some(text_rect.center()),
            primary_pressed: true,
            primary_down: true,
            ..UiInput::default()
        },
        Rect::new(0.0, 0.0, 320.0, 300.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        0.016,
    );
    ui.update(
        &UiInput {
            text: "abc".into(),
            ..UiInput::default()
        },
        Rect::new(0.0, 0.0, 320.0, 300.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        0.016,
    );
    assert!(matches!(
        &ui.node(text).unwrap().widget,
        Widget::TextInput { text, cursor: 3, .. } if text == "abc"
    ));
}

#[test]
fn modal_scope_traps_focus_and_rebinding_consumes_input() {
    let mut ui = Ui::new(Theme::retro());
    let normal = ui.add_root(panel(), LayoutItem::fill(), Layer::Normal, false);
    ui.add_child(
        normal,
        Widget::Button {
            text: UiText::literal("Background"),
            action: Some(ActionId::new(1)),
        },
        LayoutItem::auto(),
    )
    .unwrap();
    let modal = ui.add_root(panel(), LayoutItem::fill(), Layer::Overlay, true);
    let binding = ui
        .add_child(
            modal,
            Widget::Binding {
                label: UiText::literal("Jump"),
                action: "jump".into(),
                value: "Space".into(),
            },
            LayoutItem::auto(),
        )
        .unwrap();
    ui.set_modal(Some(modal));
    ui.layout(
        Rect::new(0.0, 0.0, 300.0, 200.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
    );
    assert_eq!(ui.focused(), Some(binding));
    let frame = ui.update(
        &UiInput {
            activate: true,
            ..UiInput::default()
        },
        Rect::new(0.0, 0.0, 300.0, 200.0),
        1.0,
        Edges::ZERO,
        &MonospaceMeasure,
        0.016,
    );
    assert!(frame.capturing || ui.is_capturing());
    assert!(
        frame
            .events
            .iter()
            .any(|event| matches!(event, UiEvent::RebindRequested { .. }))
    );
    assert!(ui.is_capturing());
    ui.finish_rebind("F").unwrap();
    assert!(!ui.is_capturing());
}

#[test]
fn localization_and_theme_contrast_are_explicit() {
    let mut ui = Ui::new(Theme::modern());
    ui.set_localized("menu.start", "Start Game");
    let text = UiText::keyed("menu.start", "fallback");
    assert_eq!(ui.resolve_text(&text), "Start Game");
    let theme = ui.theme();
    assert!(Theme::contrast_ratio(theme.text, theme.panel) > 4.5);
    assert!(Theme::contrast_ratio(theme.focus, theme.button) > 3.0);
}
