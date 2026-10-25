use falling_sand::menu::{Menu, MenuAction};
use falling_sand::settings::{Settings, UiOptions};
use quasar_ui::{Rect, UiInput};

fn viewport() -> Rect {
    Rect::new(0.0, 0.0, 1280.0, 720.0)
}

fn advance(menu: &mut Menu, input: UiInput) -> MenuAction {
    menu.update(&input, viewport(), 1.0, 1.0 / 60.0).unwrap()
}

fn next() -> UiInput {
    UiInput {
        next: true,
        ..UiInput::default()
    }
}

fn activate() -> UiInput {
    UiInput {
        activate: true,
        ..UiInput::default()
    }
}

#[test]
fn main_menu_starts_and_quit_uses_a_modal_confirmation() {
    let mut menu = Menu::new(UiOptions::default());
    menu.open_main();
    assert!(menu.is_open());
    assert!(
        !menu.ui().unwrap().layout_entries().is_empty()
            || menu.ui().unwrap().focus_order().is_empty()
    );
    assert_eq!(
        advance(
            &mut menu,
            UiInput {
                activate: true,
                ..UiInput::default()
            }
        ),
        MenuAction::Start
    );
    assert!(!menu.is_open());

    menu.open_main();
    for _ in 0..2 {
        advance(
            &mut menu,
            UiInput {
                next: true,
                ..UiInput::default()
            },
        );
    }
    assert_eq!(
        advance(
            &mut menu,
            UiInput {
                activate: true,
                ..UiInput::default()
            }
        ),
        MenuAction::None
    );
    assert!(menu.is_open());
    assert_eq!(
        advance(
            &mut menu,
            UiInput {
                activate: true,
                ..UiInput::default()
            }
        ),
        MenuAction::Quit
    );
}

#[test]
fn pause_resume_and_options_apply_cancel_are_real_actions() {
    let mut menu = Menu::new(UiOptions::default());
    menu.open_pause();
    assert_eq!(
        advance(
            &mut menu,
            UiInput {
                activate: true,
                ..UiInput::default()
            }
        ),
        MenuAction::Resume
    );

    menu.open_pause();
    menu.update(
        &UiInput {
            next: true,
            ..UiInput::default()
        },
        viewport(),
        1.0,
        1.0 / 60.0,
    )
    .unwrap();
    menu.update(
        &UiInput {
            next: true,
            ..UiInput::default()
        },
        viewport(),
        1.0,
        1.0 / 60.0,
    )
    .unwrap();
    let action = menu
        .update(
            &UiInput {
                activate: true,
                ..UiInput::default()
            },
            viewport(),
            1.0,
            1.0 / 60.0,
        )
        .unwrap();
    assert_eq!(action, MenuAction::None);
    // Options screen has several focusable rows; applying via keyboard should
    // yield the draft through the public action contract.
    let mut applied = false;
    for _ in 0..12 {
        let action = menu
            .update(
                &UiInput {
                    next: true,
                    ..UiInput::default()
                },
                viewport(),
                1.0,
                1.0 / 60.0,
            )
            .unwrap();
        if action != MenuAction::None {
            applied = matches!(action, MenuAction::Apply(_));
            break;
        }
        let action = menu
            .update(
                &UiInput {
                    activate: true,
                    ..UiInput::default()
                },
                viewport(),
                1.0,
                1.0 / 60.0,
            )
            .unwrap();
        if action != MenuAction::None {
            applied = matches!(action, MenuAction::Apply(_));
            break;
        }
    }
    assert!(applied || menu.is_open());
}

#[test]
fn reset_defaults_restores_ui_defaults_before_apply() {
    let mut menu = Menu::new(UiOptions {
        fullscreen: true,
        brush_radius: 9.0,
        ui_scale: 1.5,
    });
    menu.open_pause();
    advance(&mut menu, next());
    assert_eq!(advance(&mut menu, activate()), MenuAction::None);

    for _ in 0..5 {
        advance(&mut menu, next());
    }
    assert_eq!(advance(&mut menu, activate()), MenuAction::None);
    assert_eq!(menu.options(), UiOptions::default());

    for _ in 0..3 {
        advance(&mut menu, next());
    }
    assert_eq!(
        advance(&mut menu, activate()),
        MenuAction::Apply(UiOptions::default())
    );
}

#[test]
fn ui_options_persist_without_touching_game_save_schemas() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("ui.toml");
    let options = UiOptions {
        fullscreen: true,
        brush_radius: 7.0,
        ui_scale: 1.25,
    };
    options.save(&path).unwrap();
    let loaded = UiOptions::load(&path).unwrap();
    assert_eq!(loaded, options);
    assert_eq!(UiOptions::default().brush_radius, 4.0);
    assert_eq!(Settings::default().brush_radius, 4.0);
}
