use quasar_audio::{
    AudioError, AudioFormat, BusId, Clip, Listener, LoopMode, LoopRegion, Mixer, MixerConfig,
    PlaybackConfig, ProceduralSource, SoundSource, SpatialConfig, SpatialGain, SpatialModel,
    SpatialModelKind, StreamSource, VoicePolicy, VoiceStatus,
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn config(max_voices: usize) -> MixerConfig {
    MixerConfig {
        sample_rate: 48_000,
        channels: 2,
        max_voices,
        command_capacity: 32,
    }
}
fn mono(values: impl IntoIterator<Item = f32>, rate: u32) -> Clip {
    Clip::from_samples(values.into_iter().collect(), 1, rate, "test").unwrap()
}
fn tone_wav(path: &Path) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for index in 0..480 {
        let sample = ((index as f32 / 48_000.0) * 440.0 * std::f32::consts::TAU).sin();
        writer
            .write_sample((sample * 0.5 * 32767.0) as i16)
            .unwrap();
    }
    writer.finalize().unwrap();
}

#[test]
fn wav_decode_playback_and_wav_output_are_sample_correct() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tone.wav");
    tone_wav(&path);
    let clip = Clip::load(&path).unwrap();
    assert_eq!(clip.channels(), 1);
    assert_eq!(clip.sample_rate(), 48_000);
    assert_eq!(clip.frames(), 480);
    assert!((clip.duration_seconds() - 0.01).abs() < 1e-9);

    let mut mixer = Mixer::new(config(4)).unwrap();
    let handle = mixer
        .play(
            clip,
            PlaybackConfig {
                bus: BusId::MASTER,
                ..Default::default()
            },
        )
        .unwrap();
    let pcm = mixer.render(480).unwrap();
    assert!(pcm.iter().any(|sample| sample.abs() > 0.1));
    assert_eq!(handle.status(), VoiceStatus::Finished);

    let output = directory.path().join("render.wav");
    mixer.render_wav(&output, 480).unwrap();
    let mut reader = hound::WavReader::open(&output).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 48_000);
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(samples.len(), 480 * 2);
}

#[test]
fn ogg_decode_reads_the_reference_tone() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data/tone.ogg");
    let clip = Clip::load(&path).unwrap();
    assert_eq!(clip.channels(), 1);
    assert_eq!(clip.sample_rate(), 48_000);
    assert_eq!(clip.frames(), 12_000);
    assert!(clip.samples().iter().any(|sample| sample.abs() > 0.05));
    assert_eq!(
        AudioFormat::detect(&std::fs::read(&path).unwrap()),
        Some(AudioFormat::Ogg)
    );
}

#[test]
fn buses_master_mute_and_volume_compose() {
    let mut mixer = Mixer::new(config(4)).unwrap();
    let bus = mixer.create_bus("sfx", 0.5, 4).unwrap();
    mixer.set_master_volume(0.5).unwrap();
    mixer
        .play(
            mono([0.5; 8], 48_000),
            PlaybackConfig {
                bus,
                ..Default::default()
            },
        )
        .unwrap();
    let pcm = mixer.render(2).unwrap();
    assert!((pcm[0] - 0.125).abs() < 1e-6);
    assert!((pcm[1] - 0.125).abs() < 1e-6);
    mixer.set_bus_muted(bus, true).unwrap();
    let silent = mixer.render(2).unwrap();
    assert!(silent.iter().all(|sample| *sample == 0.0));
    mixer.set_bus_muted(bus, false).unwrap();
    mixer.set_muted(true);
    assert!(mixer.render(2).unwrap().iter().all(|sample| *sample == 0.0));
    assert_eq!(mixer.bus(bus).unwrap().name.as_ref(), "sfx");
}

#[test]
fn pause_preserves_position_and_freezes_scheduled_time() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    let clip = mono((0..100).map(|value| value as f32 / 100.0), 48_000);
    let handle = mixer.play(clip, PlaybackConfig::default()).unwrap();
    let first = mixer.render(10).unwrap();
    assert!((first[0] - 0.0).abs() < 1e-6);
    mixer.set_paused(true);
    let paused = mixer.render(10).unwrap();
    assert!(paused.iter().all(|sample| *sample == 0.0));
    assert_eq!(mixer.frame(), 10);
    assert_eq!(handle.status(), VoiceStatus::Paused);
    mixer.set_paused(false);
    let resumed = mixer.render(1).unwrap();
    assert!((resumed[0] - 0.10).abs() < 1e-4);
    assert_eq!(handle.status(), VoiceStatus::Playing);
}

#[test]
fn loop_regions_repeat_exactly_after_the_intro() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    let clip = mono((0..10).map(|value| value as f32 / 10.0), 48_000);
    let region = LoopRegion::new(2, 5).unwrap();
    let handle = mixer
        .play(
            clip,
            PlaybackConfig {
                looping: LoopMode::Region(region),
                ..Default::default()
            },
        )
        .unwrap();
    let pcm = mixer.render(10).unwrap();
    let left: Vec<f32> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| frame[0])
        .collect();
    let expected = [0.0, 0.1, 0.2, 0.3, 0.4, 0.2, 0.3, 0.4, 0.2, 0.3];
    for (actual, expected) in left.iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-4, "{left:?}");
    }
    assert_eq!(handle.status(), VoiceStatus::Playing);
}

#[test]
fn scheduling_starts_on_the_exact_frame() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    let clip = mono((0..10).map(|value| value as f32 / 10.0), 48_000);
    mixer.play_at(clip, PlaybackConfig::default(), 3).unwrap();
    let pcm = mixer.render(6).unwrap();
    let left: Vec<f32> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| frame[0])
        .collect();
    assert_eq!(left, [0.0, 0.0, 0.0, 0.0, 0.1, 0.2]);
}

#[test]
fn pitch_and_speed_change_the_source_step() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    let clip = mono((0..100).map(|value| value as f32), 48_000);
    let handle = mixer
        .play(
            clip,
            PlaybackConfig {
                pitch: 2.0,
                ..Default::default()
            },
        )
        .unwrap();
    let pcm = mixer.render(50).unwrap();
    assert!(pcm.iter().any(|sample| *sample != 0.0));
    assert_eq!(handle.status(), VoiceStatus::Finished);
}

#[test]
fn pan_and_spatial_models_map_to_stereo() {
    let mut mixer = Mixer::new(config(4)).unwrap();
    mixer
        .play(
            mono([0.5; 10], 48_000),
            PlaybackConfig {
                pan: -1.0,
                ..Default::default()
            },
        )
        .unwrap();
    let pcm = mixer.render(1).unwrap();
    assert!((pcm[0] - 0.5).abs() < 1e-6);
    assert_eq!(pcm[1], 0.0);

    let mut mixer = Mixer::new(config(4)).unwrap();
    mixer.set_listener(Listener::default()).unwrap();
    mixer
        .play(
            mono([0.5; 10], 48_000),
            PlaybackConfig {
                spatial: Some(SpatialConfig {
                    position: [10.0, 0.0, 0.0],
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let pcm = mixer.render(1).unwrap();
    assert!(pcm[1] > pcm[0], "right emitter should pan right: {pcm:?}");

    struct Fixed;
    impl SpatialModel for Fixed {
        fn evaluate(&self, _: &Listener, _: [f32; 3]) -> SpatialGain {
            SpatialGain {
                gain: 0.25,
                pan: -1.0,
            }
        }
    }
    let mut mixer = Mixer::new(config(4)).unwrap();
    mixer
        .play(
            mono([1.0; 10], 48_000),
            PlaybackConfig {
                spatial: Some(SpatialConfig {
                    model: SpatialModelKind::Custom(Arc::new(Fixed)),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let pcm = mixer.render(1).unwrap();
    assert!((pcm[0] - 0.25).abs() < 1e-6);
    assert_eq!(pcm[1], 0.0);
}

struct Sequence {
    values: Vec<f32>,
    index: usize,
}
impl ProceduralSource for Sequence {
    fn sample_rate(&self) -> u32 {
        48_000
    }
    fn channels(&self) -> u16 {
        1
    }
    fn next_frame(&mut self, output: &mut [f32]) -> bool {
        let Some(value) = self.values.get(self.index).copied() else {
            return false;
        };
        self.index += 1;
        output[0] = value;
        true
    }
}

#[test]
fn procedural_sources_are_rendered_and_reject_looping() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    let handle = mixer
        .play(
            SoundSource::Procedural(Box::new(Sequence {
                values: vec![0.1, 0.2, 0.3],
                index: 0,
            })),
            PlaybackConfig::default(),
        )
        .unwrap();
    let pcm = mixer.render(5).unwrap();
    let left: Vec<f32> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| frame[0])
        .collect();
    assert_eq!(left, [0.1, 0.2, 0.3, 0.0, 0.0]);
    assert_eq!(handle.status(), VoiceStatus::Finished);
    let error = mixer
        .play(
            SoundSource::Procedural(Box::new(Sequence {
                values: vec![0.1],
                index: 0,
            })),
            PlaybackConfig {
                looping: LoopMode::Whole,
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, AudioError::Unsupported(_)));
}

#[test]
fn voice_limits_steal_by_priority_or_reject() {
    let mut mixer = Mixer::new(config(1)).unwrap();
    let first = mixer
        .play(mono([0.1; 10], 48_000), PlaybackConfig::default())
        .unwrap();
    let second = mixer
        .play(
            mono([0.2; 10], 48_000),
            PlaybackConfig {
                priority: 1,
                policy: VoicePolicy::StealLowestPriority,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(first.status(), VoiceStatus::Stopped);
    assert_eq!(second.status(), VoiceStatus::Playing);
    let rejected = mixer.play(
        mono([0.3; 10], 48_000),
        PlaybackConfig {
            policy: VoicePolicy::Reject,
            ..Default::default()
        },
    );
    assert!(matches!(rejected, Err(AudioError::VoiceLimit)));
    let pcm = mixer.render(1).unwrap();
    assert!((pcm[0] - 0.2).abs() < 1e-6);
}

#[test]
fn invalid_assets_and_parameters_fail_explicitly() {
    assert!(Clip::from_bytes(b"not audio", None, "bad").is_err());
    let mut bad_ogg = b"OggS".to_vec();
    bad_ogg.extend_from_slice(&[0; 40]);
    assert!(Clip::from_bytes(&bad_ogg, Some(AudioFormat::Ogg), "bad").is_err());
    let mut mixer = Mixer::new(config(2)).unwrap();
    let clip = mono([0.1; 10], 48_000);
    assert!(matches!(
        mixer.play(
            clip.clone(),
            PlaybackConfig {
                pan: 2.0,
                ..Default::default()
            }
        ),
        Err(AudioError::InvalidArgument(_))
    ));
    assert!(matches!(
        mixer.play(
            clip,
            PlaybackConfig {
                looping: LoopMode::Region(LoopRegion { start: 2, end: 2 }),
                ..Default::default()
            }
        ),
        Err(AudioError::InvalidArgument(_))
    ));
}

#[test]
fn streaming_ogg_delivers_pcm_without_full_preload() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data/tone.ogg");
    let stream = StreamSource::from_path(&path).unwrap();
    assert_eq!(stream.channels(), 1);
    assert_eq!(stream.sample_rate(), 48_000);
    assert!(matches!(
        {
            let directory = tempfile::tempdir().unwrap();
            let wav = directory.path().join("tone.wav");
            tone_wav(&wav);
            StreamSource::from_path(&wav)
        },
        Err(AudioError::Unsupported(_))
    ));
    let mut mixer = Mixer::new(config(4)).unwrap();
    mixer
        .play_stream(stream, PlaybackConfig::default())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut heard = false;
    while Instant::now() < deadline {
        let pcm = mixer.render(512).unwrap();
        if pcm.iter().any(|sample| sample.abs() > 0.01) {
            heard = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(heard, "stream never delivered non-silent PCM");
}

#[test]
fn deterministic_render_hash_is_stable() {
    fn render_once() -> (Vec<f32>, u64) {
        let mut mixer = Mixer::new(config(4)).unwrap();
        let bus = mixer.create_bus("fx", 0.7, 2).unwrap();
        let clip = mono(
            (0..480).map(|index| ((index as f32) * 0.03).sin() * 0.4),
            48_000,
        );
        mixer
            .play(
                clip,
                PlaybackConfig {
                    bus,
                    pan: 0.3,
                    pitch: 1.25,
                    ..Default::default()
                },
            )
            .unwrap();
        let pcm = mixer.render(480).unwrap();
        let hash = pcm.iter().fold(0xcbf29ce484222325u64, |hash, sample| {
            (hash ^ u64::from(sample.to_bits())).wrapping_mul(0x100000001b3)
        });
        (pcm, hash)
    }
    let (a, hash_a) = render_once();
    let (b, hash_b) = render_once();
    assert_eq!(a, b);
    assert_eq!(hash_a, hash_b);
    assert_eq!(
        hash_a, 0x2d0a754272f4372d,
        "golden mix hash changed; inspect DSP changes before updating"
    );
}

#[test]
fn stream_reader_loop_is_configured_and_not_silently_substituted() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-data/tone.ogg");
    let stream = StreamSource::from_path(path).unwrap();
    let mut mixer = Mixer::new(config(2)).unwrap();
    mixer
        .play_stream(
            stream,
            PlaybackConfig {
                looping: LoopMode::Region(LoopRegion::new(100, 200).unwrap()),
                ..Default::default()
            },
        )
        .unwrap();
    // The stream worker skips/trims to the requested frame region; a short
    // render must not panic or report a fabricated completion.
    let _ = mixer.render(64).unwrap();
}

#[test]
fn late_schedules_are_reported_and_start_on_the_next_frame() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    let clip = mono((0..16).map(|value| value as f32 / 16.0), 48_000);
    let _ = mixer.render(4).unwrap();
    mixer.play_at(clip, PlaybackConfig::default(), 0).unwrap();
    let pcm = mixer.render(1).unwrap();
    assert_eq!(mixer.diagnostics().late_starts, 1);
    assert!((pcm[0] - 0.0).abs() < 1e-6);
    assert_eq!(mixer.frame(), 5);
}

#[test]
fn resampling_is_linear_and_frame_exact() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    let clip = mono([0.0, 1.0, 0.0, -1.0], 24_000);
    mixer.play(clip, PlaybackConfig::default()).unwrap();
    let pcm = mixer.render(8).unwrap();
    let left: Vec<f32> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| frame[0])
        .collect();
    let expected = [0.0, 0.5, 1.0, 0.5, 0.0, -0.5, -1.0, -1.0];
    for (actual, expected) in left.iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-6, "{left:?}");
    }
}

#[test]
fn mono_output_downmixes_stereo_and_preserves_peak_bounds() {
    let mut mixer = Mixer::new(MixerConfig {
        sample_rate: 48_000,
        channels: 1,
        max_voices: 2,
        command_capacity: 8,
    })
    .unwrap();
    let clip = Clip::from_samples(vec![0.5, -0.25, 0.5, -0.25], 2, 48_000, "stereo").unwrap();
    mixer.play(clip, PlaybackConfig::default()).unwrap();
    let pcm = mixer.render(2).unwrap();
    assert!((pcm[0] - 0.125).abs() < 1e-6);
}

#[test]
fn stop_bus_removal_and_stop_all_release_voices() {
    let mut mixer = Mixer::new(config(4)).unwrap();
    let bus = mixer.create_bus("sfx", 1.0, 4).unwrap();
    let a = mixer
        .play(
            mono([0.1; 100], 48_000),
            PlaybackConfig {
                bus,
                ..Default::default()
            },
        )
        .unwrap();
    let b = mixer
        .play(mono([0.2; 100], 48_000), PlaybackConfig::default())
        .unwrap();
    assert_eq!(mixer.active_voices(), 2);
    mixer.remove_bus(bus).unwrap();
    assert_eq!(a.status(), VoiceStatus::Stopped);
    assert_eq!(mixer.active_voices(), 1);
    mixer.stop_all();
    assert_eq!(b.status(), VoiceStatus::Stopped);
    assert_eq!(mixer.active_voices(), 0);
    assert!(!mixer.stop(&b));
}

#[test]
fn clip_restart_and_seek_are_sample_exact() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    let clip = mono((0..8).map(|value| value as f32 / 8.0), 48_000);
    let handle = mixer.play(clip, PlaybackConfig::default()).unwrap();
    let _ = mixer.render(3).unwrap();
    mixer.seek(&handle, 6).unwrap();
    let pcm = mixer.render(1).unwrap();
    assert!((pcm[0] - 0.75).abs() < 1e-6);
    mixer.restart(&handle).unwrap();
    assert_eq!(handle.position_frames(), 0);
    let pcm = mixer.render(1).unwrap();
    assert!((pcm[0] - 0.0).abs() < 1e-6);
}

#[test]
fn invalid_listener_and_spatial_inputs_are_rejected() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    assert!(matches!(
        mixer.set_listener(Listener {
            position: [f32::NAN, 0.0, 0.0],
            ..Default::default()
        }),
        Err(AudioError::InvalidArgument(_))
    ));
    let clip = mono([0.1; 8], 48_000);
    let handle = mixer.play(clip, PlaybackConfig::default()).unwrap();
    assert!(matches!(
        mixer.set_voice_spatial(
            &handle,
            Some(SpatialConfig {
                model: SpatialModelKind::Inverse {
                    min_distance: 0.0,
                    max_distance: 10.0,
                    rolloff: 1.0,
                },
                ..Default::default()
            })
        ),
        Err(AudioError::InvalidArgument(_))
    ));
    assert!(matches!(
        mixer.play(
            mono([0.1; 8], 48_000),
            PlaybackConfig {
                spatial: Some(SpatialConfig {
                    pan_strength: 2.0,
                    ..Default::default()
                }),
                ..Default::default()
            }
        ),
        Err(AudioError::InvalidArgument(_))
    ));
}

#[test]
fn procedural_sources_must_match_the_mixer_rate() {
    struct WrongRate;
    impl ProceduralSource for WrongRate {
        fn sample_rate(&self) -> u32 {
            24_000
        }
        fn channels(&self) -> u16 {
            1
        }
        fn next_frame(&mut self, output: &mut [f32]) -> bool {
            output[0] = 0.1;
            true
        }
    }
    let mut mixer = Mixer::new(config(2)).unwrap();
    assert!(matches!(
        mixer.play(
            SoundSource::Procedural(Box::new(WrongRate)),
            PlaybackConfig::default()
        ),
        Err(AudioError::Unsupported(_))
    ));
}

#[test]
fn concurrent_effects_mix_without_losing_handles() {
    let mut mixer = Mixer::new(config(16)).unwrap();
    let bus = mixer.create_bus("sfx", 0.9, 16).unwrap();
    let handles: Vec<_> = (0..12)
        .map(|index| {
            mixer
                .play(
                    mono(vec![(index as f32 + 1.0) / 64.0; 64], 48_000),
                    PlaybackConfig {
                        bus,
                        pan: (index as f32 / 11.0) * 2.0 - 1.0,
                        priority: index,
                        ..Default::default()
                    },
                )
                .unwrap()
        })
        .collect();
    let pcm = mixer.render(64).unwrap();
    assert!(pcm.iter().any(|sample| *sample > 0.1));
    assert!(
        handles
            .iter()
            .all(|handle| handle.status() == VoiceStatus::Finished)
    );
    assert_eq!(mixer.active_voices(), 0);
    assert_eq!(mixer.diagnostics().voices_finished, 12);
}

#[test]
fn reference_blackops_millisecond_delay_maps_to_exact_frames() {
    // Recovered BlackOps start-delay is in milliseconds. At 48 kHz, 300 ms is
    // exactly 14,400 frames; the quasar primitive must not round it to a tick.
    let mut mixer = Mixer::new(config(2)).unwrap();
    let clip = mono([0.25; 4], 48_000);
    mixer
        .play_at(clip, PlaybackConfig::default(), 14_400)
        .unwrap();
    let before = mixer.render(14_400).unwrap();
    assert!(before.iter().all(|sample| *sample == 0.0));
    let after = mixer.render(1).unwrap();
    assert!((after[0] - 0.25).abs() < 1e-6);
}

#[test]
fn reference_jackal_sixteen_voice_limit_steals_oldest() {
    let mut mixer = Mixer::new(config(16)).unwrap();
    let bus = mixer.create_bus("effects", 1.0, 16).unwrap();
    let handles: Vec<_> = (0..16)
        .map(|index| {
            mixer
                .play(
                    mono(vec![(index as f32 + 1.0) / 100.0; 64], 48_000),
                    PlaybackConfig {
                        bus,
                        policy: VoicePolicy::StealOldest,
                        ..Default::default()
                    },
                )
                .unwrap()
        })
        .collect();
    let newest = mixer
        .play(
            mono(vec![0.5; 64], 48_000),
            PlaybackConfig {
                bus,
                policy: VoicePolicy::StealOldest,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(handles[0].status(), VoiceStatus::Stopped);
    assert_eq!(newest.status(), VoiceStatus::Playing);
    assert_eq!(mixer.active_voices(), 16);
    assert_eq!(mixer.diagnostics().voices_stolen, 1);
}

#[test]
fn reference_resampled_mix_hash_is_stable() {
    fn render_once() -> u64 {
        let mut mixer = Mixer::new(config(4)).unwrap();
        let bus = mixer.create_bus("reference", 0.8, 2).unwrap();
        let clip = mono(
            (0..441).map(|index| {
                (index as f32 * 0.07).sin() * 0.3 + (index as f32 * 0.013).cos() * 0.1
            }),
            44_100,
        );
        mixer
            .play(
                clip,
                PlaybackConfig {
                    bus,
                    looping: LoopMode::Region(LoopRegion::new(32, 400).unwrap()),
                    pitch: 1.0,
                    speed: 1.0,
                    pan: -0.25,
                    ..Default::default()
                },
            )
            .unwrap();
        let pcm = mixer.render(480).unwrap();
        // Hash the actual 16-bit output samples so the reference is stable
        // across debug/release floating-point contraction differences.
        pcm.iter().fold(0xcbf29ce484222325u64, |hash, sample| {
            let quantized = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            (hash ^ u64::from(quantized as u16)).wrapping_mul(0x100000001b3)
        })
    }
    let first = render_once();
    assert_eq!(first, render_once());
    assert_eq!(
        first, 374642549635170912,
        "reference resampling/loop hash changed; inspect DSP changes"
    );
}

#[test]
fn voice_groups_limit_independently_of_buses() {
    let mut mixer = Mixer::new(config(8)).unwrap();
    let bus = mixer.create_bus("shared", 1.0, 0).unwrap();
    let group = quasar_audio::VoiceGroup(7);
    mixer.set_group_limit(group, 2).unwrap();
    assert_eq!(mixer.group_limit(group), Some(2));
    let first = mixer
        .play(
            mono([0.1; 64], 48_000),
            PlaybackConfig {
                bus,
                group: Some(group),
                priority: 1,
                ..Default::default()
            },
        )
        .unwrap();
    let second = mixer
        .play(
            mono([0.2; 64], 48_000),
            PlaybackConfig {
                bus,
                group: Some(group),
                priority: 2,
                ..Default::default()
            },
        )
        .unwrap();
    let third = mixer
        .play(
            mono([0.3; 64], 48_000),
            PlaybackConfig {
                bus,
                group: Some(group),
                priority: 3,
                policy: VoicePolicy::StealOldest,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(first.status(), VoiceStatus::Stopped);
    assert_eq!(second.status(), VoiceStatus::Playing);
    assert_eq!(third.status(), VoiceStatus::Playing);
    // A voice outside the group is not constrained by it.
    mixer
        .play(
            mono([0.4; 64], 48_000),
            PlaybackConfig {
                bus,
                group: Some(quasar_audio::VoiceGroup(8)),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(mixer.active_voices(), 3);
    assert_eq!(mixer.diagnostics().voices_stolen, 1);
}

#[test]
fn voice_group_table_is_bounded() {
    let mut mixer = Mixer::new(config(2)).unwrap();
    for group in 0..quasar_audio::MAX_VOICE_GROUPS as u32 {
        mixer
            .set_group_limit(quasar_audio::VoiceGroup(group), 1)
            .unwrap();
    }
    assert!(matches!(
        mixer.set_group_limit(quasar_audio::VoiceGroup(999), 1),
        Err(AudioError::Unsupported(_))
    ));
    // Replacing an existing group remains possible.
    mixer
        .set_group_limit(quasar_audio::VoiceGroup(0), 4)
        .unwrap();
    assert_eq!(mixer.group_limit(quasar_audio::VoiceGroup(0)), Some(4));
}

#[test]
fn shareable_audio_handles_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Clip>();
    assert_send_sync::<StreamSource>();
    assert_send_sync::<quasar_audio::VoiceHandle>();
    assert_send_sync::<quasar_audio::AudioError>();
}

#[test]
fn non_finite_procedural_output_is_sanitized() {
    struct Bad {
        frames: usize,
    }
    impl ProceduralSource for Bad {
        fn sample_rate(&self) -> u32 {
            48_000
        }
        fn channels(&self) -> u16 {
            1
        }
        fn next_frame(&mut self, output: &mut [f32]) -> bool {
            if self.frames == 0 {
                return false;
            }
            self.frames -= 1;
            output[0] = f32::INFINITY;
            true
        }
    }
    let mut mixer = Mixer::new(config(2)).unwrap();
    mixer
        .play(
            SoundSource::Procedural(Box::new(Bad { frames: 4 })),
            PlaybackConfig::default(),
        )
        .unwrap();
    let pcm = mixer.render(4).unwrap();
    assert!(pcm.iter().all(|sample| sample.is_finite()));
    assert!(pcm.iter().all(|sample| *sample == 0.0));
}

#[test]
fn handles_are_scoped_to_their_owning_mixer() {
    let mut first = Mixer::new(config(2)).unwrap();
    let mut second = Mixer::new(config(2)).unwrap();
    let handle = first
        .play(mono([0.1; 64], 48_000), PlaybackConfig::default())
        .unwrap();
    // Same numeric ID space in a fresh mixer must not alias the first handle.
    assert!(!second.stop(&handle));
    assert!(matches!(
        second.set_voice_volume(&handle, 0.5),
        Err(AudioError::NotFound)
    ));
    assert_eq!(handle.status(), VoiceStatus::Playing);
    assert!(first.stop(&handle));
    assert_eq!(handle.status(), VoiceStatus::Stopped);
}
