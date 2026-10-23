#![cfg(feature = "settings")]

use quasar_audio::{AudioSettings, BusSettings, Mixer, MixerConfig, SCHEMA_VERSION};
use std::fs;

fn mixer() -> Mixer {
    Mixer::new(MixerConfig {
        sample_rate: 48_000,
        channels: 2,
        max_voices: 8,
        command_capacity: 8,
    })
    .unwrap()
}

#[test]
fn missing_settings_return_none_and_saved_settings_round_trip() {
    let directory = tempfile::tempdir().unwrap();
    let path = AudioSettings::config_path(directory.path());
    assert!(AudioSettings::load(&path).unwrap().is_none());

    let mut settings = AudioSettings {
        master_volume: 0.6,
        muted: true,
        device: Some("Speakers".into()),
        ..Default::default()
    };
    settings.buses.insert(
        "music".into(),
        BusSettings {
            volume: 0.4,
            muted: false,
            max_voices: 2,
        },
    );
    settings.save(&path).unwrap();
    let loaded = AudioSettings::load(&path).unwrap().unwrap();
    assert_eq!(loaded, settings);
    assert_eq!(loaded.schema_version, SCHEMA_VERSION);
}

#[test]
fn settings_apply_and_capture_mixer_buses() {
    let mut mixer = mixer();
    let mut settings = AudioSettings {
        master_volume: 0.5,
        muted: false,
        ..Default::default()
    };
    settings.buses.insert(
        "music".into(),
        BusSettings {
            volume: 0.25,
            muted: true,
            max_voices: 3,
        },
    );
    settings.buses.insert(
        "sfx".into(),
        BusSettings {
            volume: 0.8,
            muted: false,
            max_voices: 12,
        },
    );
    let resolved = settings.apply_mixer(&mut mixer).unwrap();
    assert_eq!(resolved.len(), 2);
    assert_eq!(mixer.master_volume(), 0.5);
    let music = mixer.bus_id("music").unwrap();
    assert_eq!(mixer.bus(music).unwrap().volume, 0.25);
    assert!(mixer.bus(music).unwrap().muted);
    assert_eq!(mixer.bus(music).unwrap().max_voices, 3);

    // Applying twice resolves the existing buses instead of duplicating them.
    settings.apply_mixer(&mut mixer).unwrap();
    assert_eq!(mixer.buses().count(), 3);

    let captured = AudioSettings::from_mixer(&mixer);
    assert_eq!(captured.master_volume, 0.5);
    assert_eq!(captured.buses.len(), 2);
    assert_eq!(captured.buses["sfx"].volume, 0.8);
    assert_eq!(captured.buses["sfx"].max_voices, 12);
}

#[test]
fn corrupt_or_unsupported_settings_are_rejected_not_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let path = AudioSettings::config_path(directory.path());
    fs::create_dir_all(directory.path()).unwrap();
    fs::write(&path, "{not json").unwrap();
    assert!(AudioSettings::load(&path).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{not json");

    fs::write(
        &path,
        r#"{"schema_version":99,"master_volume":1.0,"muted":false,"buses":{}}"#,
    )
    .unwrap();
    assert!(AudioSettings::load(&path).is_err());

    fs::write(
        &path,
        r#"{"schema_version":1,"master_volume":9.0,"muted":false,"buses":{}}"#,
    )
    .unwrap();
    assert!(AudioSettings::load(&path).is_err());

    fs::write(
        &path,
        r#"{"schema_version":1,"master_volume":1.0,"muted":false,"buses":{},"unexpected":true}"#,
    )
    .unwrap();
    assert!(AudioSettings::load(&path).is_err());
}
