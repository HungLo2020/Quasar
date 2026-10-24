use super::*;
#[cfg(any(feature = "json", feature = "storage"))]
use std::fs;
use std::{collections::BTreeMap, path::Path};

#[cfg(all(feature = "toml", feature = "json"))]
use serde::Deserialize;

#[test]
fn precedence_is_default_file_environment_cli_programmatic() {
    let mut config = Config::new();
    config.set_default("game.speed", 1.0).unwrap();
    config.set_file("game.speed", 2.0).unwrap();
    config
        .apply_env_with(&[("game.speed", "GAME_SPEED")], |_| Some("3.0".into()))
        .unwrap();
    config.set_cli("game.speed", 4.0).unwrap();
    assert_eq!(config.get_f64("game.speed").unwrap(), 4.0);
    assert_eq!(config.provenance("game.speed"), Some(Source::Cli));
    config.set_file("game.speed", 9.0).unwrap();
    assert_eq!(config.get_f64("game.speed").unwrap(), 4.0);
    config.set_programmatic("game.speed", 5.0).unwrap();
    assert_eq!(config.get_f64("game.speed").unwrap(), 5.0);
    assert_eq!(config.provenance("game.speed"), Some(Source::Programmatic));
}

#[test]
fn typed_access_validation_and_missing_errors_are_explicit() {
    let mut config = Config::new();
    config.set_default("window.width", 1280_u32).unwrap();
    config.set_default("window.height", "720").unwrap();
    config.set_default("audio.enabled", "yes").unwrap();
    assert_eq!(config.get_u64("window.width").unwrap(), 1280);
    assert_eq!(config.get_u64("window.height").unwrap(), 720);
    assert_eq!(config.get_string("audio.enabled").unwrap(), "yes");
    assert!(config.get_bool("audio.enabled").is_err());
    let error = config
        .validate_u64_range("window.width", 320, 1024)
        .unwrap_err();
    assert_eq!(error.kind(), ConfigErrorKind::Validation);
    assert_eq!(error.key(), Some("window.width"));
    assert_eq!(error.source(), Some(Source::Default));
    assert!(config.get_u64("missing").is_err());
    assert_eq!(config.get_or("missing", 7).unwrap(), 7);
}

#[cfg(feature = "json")]
#[test]
fn json_flattens_objects_and_preserves_unknown_keys() {
    let mut config = Config::new();
    config.set_default("game.speed", 1.0).unwrap();
    let applied = config
        .load_json_str(
            r#"{
                "version": 1,
                "window": {"width": 1440, "title": "Starfall"},
                "game": {"speed": 2.5, "tags": ["a", "b"]},
                "future": {"option": true}
            }"#,
        )
        .unwrap();
    assert_eq!(applied, 6);
    assert_eq!(config.get_u64("window.width").unwrap(), 1440);
    assert_eq!(config.get_string("window.title").unwrap(), "Starfall");
    assert_eq!(config.get_f64("game.speed").unwrap(), 2.5);
    assert!(config.keys().any(|key| key == "future.option"));
    assert!(config.keys().any(|key| key == "game.tags"));
    assert_eq!(config.schema_version().unwrap(), 1);
}

#[cfg(feature = "json")]
#[test]
fn malformed_json_and_files_are_non_destructive() {
    let mut config = Config::new();
    config.set_cli("window.width", 900_u32).unwrap();
    let before = config.get("window.width").cloned();
    assert!(config.load_json_str("{not json").is_err());
    assert_eq!(config.get("window.width"), before.as_ref());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    fs::write(&path, "{broken").unwrap();
    assert!(config.load_json_file(&path).is_err());
    assert_eq!(config.get("window.width"), before.as_ref());
    assert_eq!(
        config
            .load_json_file_if_exists(directory.path().join("missing.json"))
            .unwrap(),
        0
    );
}

#[cfg(feature = "json")]
#[test]
fn json_file_and_environment_mapping_apply() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    fs::write(&path, r#"{"audio":{"volume":0.25}}"#).unwrap();
    let mut config = Config::new();
    config.load_json_file(&path).unwrap();
    assert_eq!(
        config
            .apply_env_with(&[("audio.volume", "GAME_VOLUME")], |variable| {
                (variable == "GAME_VOLUME").then(|| "0.5".into())
            })
            .unwrap(),
        1
    );
    assert_eq!(config.get_f64("audio.volume").unwrap(), 0.5);
    assert_eq!(config.provenance("audio.volume"), Some(Source::Environment));
}

#[test]
fn migration_is_consecutive_and_preserves_unknown_keys() {
    fn v1_to_v2(config: &mut Config) -> Result<(), ConfigError> {
        config.set_file("new.key", 1_u32)?;
        Ok(())
    }
    fn v2_to_v3(config: &mut Config) -> Result<(), ConfigError> {
        config.set_file("new.key", 2_u32)?;
        Ok(())
    }
    let mut config = Config::new();
    config.set_file("version", 1_u64).unwrap();
    config.set_file("unknown.future", true).unwrap();
    config
        .migrate(
            3,
            &[
                Migration::new(1, 2, v1_to_v2),
                Migration::new(2, 3, v2_to_v3),
            ],
        )
        .unwrap();
    assert_eq!(config.schema_version().unwrap(), 3);
    assert_eq!(config.get_u64("new.key").unwrap(), 2);
    assert!(config.contains("unknown.future"));
    assert!(matches!(
        config.migrate(2, &[]).unwrap_err().kind(),
        ConfigErrorKind::NewerSchema
    ));
    let mut config = Config::new();
    config.set_file("version", 1_u64).unwrap();
    assert!(matches!(
        config.migrate(2, &[]).unwrap_err().kind(),
        ConfigErrorKind::Migration
    ));
}

#[test]
fn resolution_parser_rejects_invalid_and_nonportable_sizes() {
    assert_eq!(
        Resolution::parse("1280x720").unwrap(),
        Resolution::new(1280, 720).unwrap()
    );
    for value in ["1280", "0x720", "1280x0", "x720", "1280x", "20000x10"] {
        assert!(Resolution::parse(value).is_err(), "{value}");
    }
}

#[test]
fn common_cli_parses_values_unknown_flags_and_set_overrides() {
    let args = CommonArgs::parse_from([
        "--headless",
        "--resolution=1600x900",
        "--no-audio",
        "--benchmark",
        "--benchmark-file=/tmp/report.json",
        "--log-level=debug",
        "--config",
        "/tmp/game.json",
        "--set",
        "game.speed=2.5",
        "--difficulty",
        "hard",
        "--",
        "--after",
    ])
    .unwrap();
    assert_eq!(args.headless, Some(true));
    assert_eq!(args.resolution, Resolution::new(1600, 900).ok());
    assert_eq!(args.no_audio, Some(true));
    assert_eq!(args.benchmark, Some(true));
    assert_eq!(
        args.benchmark_file.as_deref(),
        Some(Path::new("/tmp/report.json"))
    );
    assert_eq!(args.log_level.as_deref(), Some("debug"));
    assert_eq!(args.config.as_deref(), Some(Path::new("/tmp/game.json")));
    assert_eq!(args.overrides, vec![("game.speed".into(), "2.5".into())]);
    assert_eq!(args.remaining, ["--difficulty", "hard", "--after"]);
}

#[test]
fn common_cli_reports_missing_or_malformed_values() {
    for args in [
        vec!["--resolution"],
        vec!["--resolution", "bad"],
        vec!["--audio-device"],
        vec!["--set", "no_equals"],
        vec!["--set", "=value"],
    ] {
        assert!(CommonArgs::parse_from(args).is_err());
    }
}

#[test]
fn common_args_apply_values_and_overrides_with_cli_precedence() {
    let args = CommonArgs::parse_from([
        "--headless",
        "--resolution",
        "1024x768",
        "--set",
        "game.speed=3",
    ])
    .unwrap();
    let mut config = Config::new();
    config.set_file("common.headless", false).unwrap();
    config.set_file("game.speed", 1.0).unwrap();
    args.apply_to_config(&mut config).unwrap();
    args.apply_overrides(&mut config).unwrap();
    assert!(config.get_bool("common.headless").unwrap());
    assert_eq!(
        config.get_resolution("common.resolution").unwrap().width,
        1024
    );
    assert_eq!(config.get_f64("game.speed").unwrap(), 3.0);
    assert_eq!(config.provenance("game.speed"), Some(Source::Cli));
    assert!(
        args.help("starfall", "Game options:\n  --seed N")
            .contains("--seed N")
    );
}

#[test]
fn common_env_is_deterministic_and_cli_can_override_it() {
    let mut values = BTreeMap::from([
        ("STAR_HEADLESS".to_owned(), "true".to_owned()),
        ("STAR_RESOLUTION".to_owned(), "800x600".to_owned()),
        ("STAR_NO_AUDIO".to_owned(), "1".to_owned()),
        ("STAR_LOG_LEVEL".to_owned(), "trace".to_owned()),
    ]);
    let mut args = CommonArgs::from_env_with("STAR", |variable| values.remove(variable)).unwrap();
    assert!(args.is_headless());
    assert_eq!(args.resolution, Resolution::new(800, 600).ok());
    assert_eq!(args.no_audio, Some(true));
    assert_eq!(args.log_level.as_deref(), Some("trace"));
    args.parse_into(["--resolution", "1920x1080", "--windowed"])
        .unwrap();
    assert_eq!(args.resolution, Resolution::new(1920, 1080).ok());
    assert_eq!(args.fullscreen, Some(false));
}

#[test]
fn common_env_rejects_invalid_values() {
    assert!(
        CommonArgs::from_env_with("STAR", |variable| {
            (variable == "STAR_HEADLESS").then(|| "maybe".into())
        })
        .is_err()
    );
    assert!(
        CommonArgs::from_env_with("STAR", |variable| {
            (variable == "STAR_RESOLUTION").then(|| "bad".into())
        })
        .is_err()
    );
}

#[cfg(feature = "storage")]
#[test]
fn optional_basalt_shared_config_loads_without_writing() {
    let directory = tempfile::tempdir().unwrap();
    let paths = quasar_storage::GamePaths {
        root: directory.path().to_path_buf(),
        app: directory.path().join("app"),
        save: directory.path().join("save"),
        config: directory.path().join("config"),
        shared_config: directory.path().join("config/shared"),
        cache: directory.path().join("cache"),
    };
    fs::create_dir_all(&paths.shared_config).unwrap();
    let path = paths.shared_config.join("starfall.json");
    fs::write(&path, r#"{"game":{"speed":1.5}}"#).unwrap();
    let mut config = Config::new();
    config
        .load_json_basalt_shared(&paths, "starfall.json")
        .unwrap();
    assert_eq!(config.get_f64("game.speed").unwrap(), 1.5);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        r#"{"game":{"speed":1.5}}"#
    );
}

#[cfg(all(feature = "toml", feature = "json"))]
#[test]
fn toml_comments_nested_tables_and_typed_serde_work() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Window {
        width: u32,
        title: String,
    }
    #[derive(Debug, Deserialize, PartialEq)]
    struct Settings {
        version: u32,
        window: Window,
        tags: Vec<String>,
        enabled: bool,
    }
    let text = r#"
# Human-edited TOML keeps comments.
version = 2
enabled = true
tags = ["alpha", "beta"]
[window]
width = 1280 # inline comment
title = "123"
"#;
    let mut config = Config::new();
    config.load_toml_str(text).unwrap();
    assert_eq!(config.get_u64("window.width").unwrap(), 1280);
    assert_eq!(config.get_string("window.title").unwrap(), "123");
    let typed: Settings = config.deserialize().unwrap();
    assert_eq!(typed.window.width, 1280);
    assert_eq!(typed.window.title, "123");
    assert_eq!(typed.tags, ["alpha", "beta"]);
    assert!(typed.enabled);
    let direct: Settings = load_toml_typed(text).unwrap();
    assert_eq!(direct, typed);
}

#[cfg(all(feature = "toml", feature = "json"))]
#[test]
fn extension_detection_supports_toml_and_json_without_rewriting() {
    let directory = tempfile::tempdir().unwrap();
    let toml_path = directory.path().join("settings.toml");
    let json_path = directory.path().join("settings.json");
    let toml_text = "# comment\nversion = 1\n[window]\nwidth = 640\n";
    let json_text = r#"{"version":1,"window":{"width":800}}"#;
    fs::write(&toml_path, toml_text).unwrap();
    fs::write(&json_path, json_text).unwrap();
    let mut config = Config::new();
    config.load_file(&toml_path).unwrap();
    assert_eq!(config.get_u64("window.width").unwrap(), 640);
    let mut config = Config::new();
    config.load_file(&json_path).unwrap();
    assert_eq!(config.get_u64("window.width").unwrap(), 800);
    assert_eq!(FileFormat::from_path(&toml_path), Some(FileFormat::Toml));
    assert_eq!(FileFormat::from_path(&json_path), Some(FileFormat::Json));
    assert!(FileFormat::from_path(Path::new("settings.yaml")).is_none());
    assert!(Config::new().load_file("settings.yaml").is_err());
    assert_eq!(fs::read_to_string(&toml_path).unwrap(), toml_text);
    assert_eq!(fs::read_to_string(&json_path).unwrap(), json_text);
    let typed: SettingsJson = load_file_typed(&json_path).unwrap();
    assert_eq!(typed.version, 1);
    assert_eq!(typed.window.width, 800);
}

#[cfg(all(feature = "toml", feature = "json"))]
#[derive(Deserialize)]
struct SettingsJson {
    version: u32,
    window: WindowJson,
}
#[cfg(all(feature = "toml", feature = "json"))]
#[derive(Deserialize)]
struct WindowJson {
    width: u32,
}

#[cfg(all(feature = "toml", feature = "json"))]
#[test]
fn typed_config_uses_precedence_resolved_values() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Typed {
        window: Window,
        speed: f64,
    }
    #[derive(Debug, Deserialize, PartialEq)]
    struct Window {
        width: u32,
        title: String,
    }
    let mut config = Config::new();
    config.set_default("window.width", 1280_u32).unwrap();
    config.set_default("window.title", "123").unwrap();
    config.set_default("speed", 1.0).unwrap();
    config.set_file("window.width", 800_u32).unwrap();
    config.set_cli("speed", 4.0).unwrap();
    let typed = TypedConfig::<Typed>::from_config(config).unwrap();
    assert_eq!(typed.value().window.width, 800);
    assert_eq!(typed.value().window.title, "123");
    assert_eq!(typed.value().speed, 4.0);
    assert_eq!(
        typed.config().provenance("window.width"),
        Some(Source::File)
    );
}
