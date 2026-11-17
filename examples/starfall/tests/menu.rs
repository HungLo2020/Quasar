use quasar_ui::{MonospaceMeasure, Rect, UiInput};
use starfall::menu::{Menu, MenuAction, Options};

fn viewport() -> Rect {
    Rect::new(0.0, 0.0, 1280.0, 720.0)
}

fn advance(menu: &mut Menu, input: UiInput) -> MenuAction {
    menu.update(&input, viewport(), 1.0, &MonospaceMeasure, 1.0 / 60.0)
        .unwrap()
}

fn activate() -> UiInput {
    UiInput {
        activate: true,
        ..UiInput::default()
    }
}

fn next() -> UiInput {
    UiInput {
        next: true,
        ..UiInput::default()
    }
}

#[test]
fn main_menu_starts_and_modal_quit_requires_confirmation() {
    let mut menu = Menu::new(Options::default());
    menu.open_main();
    assert!(menu.is_open());
    assert_eq!(advance(&mut menu, activate()), MenuAction::Start);
    assert!(!menu.is_open());

    menu.open_main();
    advance(&mut menu, next());
    advance(&mut menu, next());
    assert_eq!(advance(&mut menu, activate()), MenuAction::None);
    assert!(menu.is_open());
    // The confirmation starts focused on YES.
    assert_eq!(advance(&mut menu, activate()), MenuAction::Quit);
}

#[test]
fn pause_escape_resumes_and_options_cancel_restores_applied_values() {
    let mut menu = Menu::new(Options::default());
    menu.open_pause();
    assert_eq!(
        advance(
            &mut menu,
            UiInput {
                cancel: true,
                ..UiInput::default()
            }
        ),
        MenuAction::Resume
    );

    menu.open_main();
    advance(&mut menu, next());
    assert_eq!(advance(&mut menu, activate()), MenuAction::None);
    // Options starts on FULLSCREEN; move to MASTER VOLUME and change it.
    advance(&mut menu, next());
    let before = menu.options().volume;
    advance(
        &mut menu,
        UiInput {
            right: true,
            ..UiInput::default()
        },
    );
    assert!(menu.options().volume > before);
    assert_eq!(
        advance(
            &mut menu,
            UiInput {
                cancel: true,
                ..UiInput::default()
            }
        ),
        MenuAction::None
    );
    assert_eq!(menu.options().volume, before);
}

#[test]
fn options_apply_returns_a_snapshot_and_resets_screen() {
    let mut menu = Menu::new(Options::default());
    menu.open_main();
    advance(&mut menu, next());
    advance(&mut menu, activate());
    advance(&mut menu, next());
    advance(
        &mut menu,
        UiInput {
            right: true,
            ..UiInput::default()
        },
    );
    advance(&mut menu, next());
    advance(&mut menu, next());
    advance(&mut menu, next());
    let action = advance(&mut menu, activate());
    match action {
        MenuAction::Apply(options) => assert!(options.volume > Options::default().volume),
        other => panic!("expected apply action, got {other:?}"),
    }
    assert!(menu.is_open());
}

#[test]
fn reset_defaults_restores_the_default_snapshot_before_apply() {
    let mut menu = Menu::new(Options {
        fullscreen: true,
        volume: 0.2,
        sensitivity: 0.009,
        player_name: "Test Pilot".into(),
        language: "en".into(),
    });
    menu.open_main();
    advance(&mut menu, next());
    advance(&mut menu, activate());

    for _ in 0..6 {
        advance(&mut menu, next());
    }
    assert_eq!(advance(&mut menu, activate()), MenuAction::None);
    assert_eq!(menu.options(), Options::default());

    for _ in 0..4 {
        advance(&mut menu, next());
    }
    assert_eq!(
        advance(&mut menu, activate()),
        MenuAction::Apply(Options::default())
    );
}

#[test]
fn callsign_text_entry_tracks_unicode_and_apply_snapshot() {
    let mut menu = Menu::new(Options::default());
    menu.open_pause();
    advance(&mut menu, next());
    advance(&mut menu, activate());
    // FULLSCREEN -> MASTER VOLUME -> MOUSE SENSITIVITY -> CALLSIGN.
    for _ in 0..3 {
        advance(&mut menu, next());
    }
    advance(
        &mut menu,
        UiInput {
            text: "Команда नमस्ते".into(),
            ..UiInput::default()
        },
    );
    assert_eq!(menu.options().player_name, "PilotКоманда नमस्ते");
    advance(&mut menu, next());
    match advance(&mut menu, activate()) {
        MenuAction::Apply(options) => {
            assert_eq!(options.player_name, "PilotКоманда नमस्ते");
        }
        other => panic!("expected apply, got {other:?}"),
    }
}

#[test]
fn preferences_round_trip_and_clamp_invalid_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("starfall-ui.toml");
    let options = Options {
        fullscreen: true,
        volume: 0.25,
        sensitivity: 0.004,
        player_name: "Pilot".into(),
        language: "en".into(),
    };
    options.save(&path).unwrap();
    assert_eq!(Options::load(&path).unwrap(), options);
    assert_eq!(Options::default().volume, 0.6);

    std::fs::write(
        &path,
        "fullscreen = false\nvolume = 9.0\nsensitivity = 0.0\n",
    )
    .unwrap();
    let loaded = Options::load(&path).unwrap();
    assert_eq!(loaded.volume, 1.0);
    assert_eq!(loaded.sensitivity, 0.0001);
}

fn open_options(menu: &mut Menu) {
    menu.open_main();
    advance(menu, next());
    assert_eq!(advance(menu, activate()), MenuAction::None);
}

fn resolved_buttons(menu: &mut Menu) -> Vec<String> {
    // Lay out the freshly built screen.
    advance(menu, UiInput::default());
    let ui = menu.ui().unwrap();
    ui.layout_entries()
        .iter()
        .filter_map(|e| match &ui.node(e.id)?.widget {
            quasar_ui::Widget::Button { text, .. } => Some(ui.resolve_text(text).to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_language_option_localises_the_menu_and_applies() {
    let mut menu = Menu::new(Options::default());
    open_options(&mut menu);
    assert!(resolved_buttons(&mut menu).contains(&"APPLY".to_owned()));
    // FULLSCREEN … RESET DEFAULTS, CONTROLS, then LANGUAGE.
    for _ in 0..8 {
        advance(&mut menu, next());
    }
    advance(
        &mut menu,
        UiInput {
            right: true,
            ..UiInput::default()
        },
    );
    assert_eq!(menu.options().language, "es");
    let buttons = resolved_buttons(&mut menu);
    assert!(buttons.contains(&"APLICAR".to_owned()), "{buttons:?}");
    assert!(
        menu.ui().unwrap().missing_keys().is_empty(),
        "every menu key is translated"
    );
    // APPLY is four rows up from LANGUAGE.
    for _ in 0..5 {
        advance(&mut menu, next());
    }
    match advance(&mut menu, activate()) {
        MenuAction::Apply(options) => assert_eq!(options.language, "es"),
        other => panic!("expected apply, got {other:?}"),
    }
    // The main menu stays Spanish after applying.
    assert!(resolved_buttons(&mut menu).iter().any(|b| b == "OPCIONES"));
    // Every English key has a Spanish translation.
    let en = quasar_ui::StringTable::parse("en", include_str!("../lang/en.lang")).unwrap();
    let es = quasar_ui::StringTable::parse("es", include_str!("../lang/es.lang")).unwrap();
    assert!(es.missing_from(&en).is_empty());
}

#[test]
fn the_controls_screen_rebinds_axes_and_reports_new_bindings() {
    use quasar_input::{Binding, Button, Key};
    let defaults = starfall::settings::ControlKeys::default()
        .bindings()
        .unwrap();
    let mut menu = Menu::new(Options::default());
    menu.set_bindings(defaults.clone(), defaults.clone());
    open_options(&mut menu);
    for _ in 0..7 {
        advance(&mut menu, next());
    }
    assert_eq!(advance(&mut menu, activate()), MenuAction::None);
    // Focus starts on THRUST; activating it starts a capture.
    advance(&mut menu, activate());
    assert!(menu.is_capturing());
    assert_eq!(
        menu.capture(&[Button::Key(Key::KeyK)]).unwrap(),
        MenuAction::None
    );
    let action = menu.capture(&[Button::Key(Key::KeyI)]).unwrap();
    let MenuAction::ApplyBindings(bindings) = action else {
        panic!("expected new bindings, got {action:?}");
    };
    assert!(bindings.actions["forward"].contains(&Binding::axis(Key::KeyK, Key::KeyI)));
    assert_eq!(menu.bindings(), &bindings);
    assert!(!menu.is_capturing());
    // Escape leaves the controls screen for the options screen.
    advance(
        &mut menu,
        UiInput {
            cancel: true,
            ..UiInput::default()
        },
    );
    assert!(resolved_buttons(&mut menu).contains(&"CONTROLS".to_owned()));
}
