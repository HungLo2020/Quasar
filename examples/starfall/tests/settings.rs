use quasar_config::CommonArgs;
use starfall::settings::{GameCli, GameSettings};
use std::fs;

#[test]
fn file_common_cli_and_set_precedence_reach_typed_settings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("starfall.json");
    fs::write(
        &path,
        r#"{
            "window": {"width": 800, "height": 600, "title": "Configured"},
            "game": {"seed": 11, "player_speed": 4.0},
            "controls": {"forward": "KeyI", "back": "KeyK", "left": "KeyJ", "right": "KeyL", "turn_left": "KeyU", "turn_right": "KeyO", "shoot": "KeyF", "restart": "KeyY", "quit": "KeyP"}
        }"#,
    )
    .unwrap();
    let mut common = CommonArgs {
        config: Some(path),
        headless: Some(true),
        resolution: Some(quasar_config::Resolution::new(1024, 768).unwrap()),
        ..CommonArgs::default()
    };
    common.overrides.push(("game.seed".into(), "99".into()));
    common
        .overrides
        .push(("controls.forward".into(), "KeyZ".into()));
    let settings = GameSettings::load(&common, &GameCli::default()).unwrap();
    assert_eq!(settings.window_width, 1024);
    assert_eq!(settings.window_height, 768);
    assert_eq!(settings.seed, 99);
    assert_eq!(settings.player_speed, 4.0);
    assert_eq!(settings.controls.forward.name(), "KeyZ");
    assert_eq!(settings.controls.quit.name(), "KeyP");
    assert!(settings.headless);
    assert!(settings.autoplay);
}

#[test]
fn invalid_configuration_is_rejected_without_hiding_the_cause() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("broken.json");
    fs::write(&path, r#"{"game":{"player_speed":-1.0}}"#).unwrap();
    let common = CommonArgs {
        config: Some(path),
        ..CommonArgs::default()
    };
    let error = GameSettings::load(&common, &GameCli::default()).unwrap_err();
    assert!(error.to_string().contains("player_speed"));
    assert!(
        fs::read_to_string(directory.path().join("broken.json"))
            .unwrap()
            .contains("player_speed")
    );
}

#[test]
fn an_explicit_missing_config_file_is_an_error() {
    let common = CommonArgs {
        config: Some(std::path::PathBuf::from(
            "/definitely/missing/starfall.json",
        )),
        ..CommonArgs::default()
    };
    let error = GameSettings::load(&common, &GameCli::default()).unwrap_err();
    assert!(error.to_string().contains("does not exist"));
}

#[test]
fn controls_and_resolution_conflicts_are_explicit() {
    let mut common = CommonArgs::default();
    common
        .overrides
        .push(("controls.shoot".into(), "NotAKey".into()));
    assert!(GameSettings::load(&common, &GameCli::default()).is_err());

    let common = CommonArgs {
        resolution: Some(quasar_config::Resolution::new(10, 10).unwrap()),
        ..CommonArgs::default()
    };
    assert!(GameSettings::load(&common, &GameCli::default()).is_err());
}

#[test]
fn help_text_contains_game_and_common_options() {
    let common = CommonArgs::default();
    let text = common.help("starfall", GameCli::help());
    for option in ["--headless", "--resolution", "--set", "--seed", "--record"] {
        assert!(text.contains(option), "{option}");
    }
}
