#![cfg(feature = "native")]

use quasar_audio::{AudioEngine, AudioError, Clip, EngineConfig, PlaybackConfig, VoiceStatus};
use std::time::{Duration, Instant};

#[test]
fn engine_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AudioEngine>();
}

#[test]
fn invalid_engine_config_is_rejected_before_opening_a_device() {
    let error = match AudioEngine::open(
        None,
        EngineConfig {
            command_capacity: 0,
            ..Default::default()
        },
    ) {
        Ok(_) => panic!("zero-capacity command queue must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(error, AudioError::InvalidArgument(_)));

    let error = match AudioEngine::open(
        None,
        EngineConfig {
            channels: Some(6),
            ..Default::default()
        },
    ) {
        Ok(_) => panic!("more than two channels must be rejected"),
        Err(error) => error,
    };
    assert!(matches!(error, AudioError::Unsupported(_)));
}

#[test]
#[ignore = "requires a native audio host to enumerate devices"]
fn missing_device_is_reported_without_fabricating_output() {
    let error = match AudioEngine::open(
        Some("__quasar_audio_missing_device__"),
        EngineConfig::default(),
    ) {
        Ok(_) => panic!("a missing device must not open"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        AudioError::NotFound | AudioError::NoDevice | AudioError::Device(_)
    ));
}

#[test]
#[ignore = "requires a native audio output device"]
fn native_engine_plays_a_short_clip_and_reports_diagnostics() {
    let engine = AudioEngine::open(None, EngineConfig::default()).unwrap();
    let bus = engine.create_bus("native-test", 0.5, 2).unwrap();
    assert!(engine.bus_exists(bus));
    assert!(engine.bus_exists(quasar_audio::BusId::MASTER));
    assert!(matches!(
        engine.remove_bus(quasar_audio::BusId::MASTER),
        Err(AudioError::InvalidArgument(_))
    ));
    assert!(matches!(
        engine.play(
            Clip::from_samples(vec![0.1; 8], 1, 48_000, "bad bus").unwrap(),
            PlaybackConfig {
                bus: quasar_audio::BusId(999),
                ..Default::default()
            }
        ),
        Err(AudioError::NotFound)
    ));
    let clip = Clip::from_samples(vec![0.1; 4800], 1, 48_000, "native tone").unwrap();
    let handle = engine
        .play(
            clip,
            PlaybackConfig {
                bus,
                ..Default::default()
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && handle.status() != VoiceStatus::Finished {
        std::thread::sleep(Duration::from_millis(10));
    }
    let diagnostics = engine.diagnostics();
    assert!(diagnostics.mixer.voices_started >= 1);
    assert!(!diagnostics.device_lost);
    engine.stop_all().unwrap();
}
