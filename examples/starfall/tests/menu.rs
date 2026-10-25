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
