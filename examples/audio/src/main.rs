use quasar_audio::{
    AudioFormat, BusId, Clip, Listener, LoopMode, Mixer, MixerConfig, PlaybackConfig,
    ProceduralSource, SoundSource, SpatialConfig, SpatialModelKind, StreamSource, VoiceGroup,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    error::Error,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;

struct CountAlloc;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(size as u64, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

#[derive(Default)]
struct Options {
    offline: Option<PathBuf>,
    play: bool,
    devices: bool,
    benchmark: Option<PathBuf>,
    frames: Option<usize>,
}
impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--offline" => options.offline = Some(args.next().ok_or("missing output")?.into()),
                "--play" => options.play = true,
                "--devices" => options.devices = true,
                "--benchmark" => {
                    options.benchmark = Some(args.next().ok_or("missing output")?.into())
                }
                "--frames" => options.frames = Some(args.next().ok_or("missing frames")?.parse()?),
                "--help" | "-h" => {
                    println!(
                        "audio-example [--offline FILE] [--play] [--devices] [--benchmark FILE] [--frames N]\nBuilds music, overlapping effects, spatial emitters, buses and procedural audio. Offline mode writes WAV and a deterministic hash."
                    );
                    std::process::exit(0);
                }
                _ => return Err(format!("unknown argument {arg}").into()),
            }
        }
        if options.play && options.offline.is_some() {
            return Err("--play and --offline are mutually exclusive".into());
        }
        Ok(options)
    }
}

fn music() -> Result<Clip> {
    let seconds = 4.0;
    let mut samples = Vec::with_capacity((RATE as f32 * seconds) as usize * CHANNELS as usize);
    let notes = [220.0f32, 277.18, 329.63, 440.0];
    for frame in 0..(RATE as f32 * seconds) as usize {
        let time = frame as f32 / RATE as f32;
        let note = notes[(time * 2.0) as usize % notes.len()];
        let envelope = (1.0 - (time * 2.0).fract()).clamp(0.0, 1.0).powf(0.6);
        let value = (time * note * std::f32::consts::TAU).sin() * 0.18 * envelope;
        samples.push(value);
        samples.push(value * 0.92);
    }
    Ok(Clip::from_samples(
        samples,
        CHANNELS,
        RATE,
        "procedural music",
    )?)
}

fn effect(frequency: f32, seconds: f32) -> Result<Clip> {
    let frames = (RATE as f32 * seconds) as usize;
    let mut samples = Vec::with_capacity(frames);
    for frame in 0..frames {
        let time = frame as f32 / RATE as f32;
        let envelope = (1.0 - frame as f32 / frames as f32).powf(2.0);
        samples.push((time * frequency * std::f32::consts::TAU).sin() * 0.25 * envelope);
    }
    Ok(Clip::from_samples(samples, 1, RATE, "effect")?)
}

struct Beep {
    left: usize,
    frequency: f32,
    phase: f32,
}
impl ProceduralSource for Beep {
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn channels(&self) -> u16 {
        1
    }
    fn next_frame(&mut self, output: &mut [f32]) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= 1;
        self.phase += self.frequency * std::f32::consts::TAU / RATE as f32;
        output[0] = self.phase.sin() * 0.12;
        true
    }
}

fn write_wav(path: &PathBuf, pcm: &[f32]) -> Result {
    let spec = hound::WavSpec {
        channels: CHANNELS,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in pcm {
        writer.write_sample((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16)?;
    }
    writer.finalize()?;
    Ok(())
}

fn build_mixer() -> Result<(Mixer, BusId, BusId)> {
    let mut mixer = Mixer::new(MixerConfig {
        sample_rate: RATE,
        channels: CHANNELS,
        max_voices: 64,
        command_capacity: 64,
    })?;
    let music_bus = mixer.create_bus("music", 0.45, 4)?;
    let sfx_bus = mixer.create_bus("sfx", 0.8, 32)?;
    mixer.set_master_volume(0.8)?;
    mixer.set_listener(Listener::default())?;
    Ok((mixer, music_bus, sfx_bus))
}

fn schedule_demo(mixer: &mut Mixer, music_bus: BusId, sfx_bus: BusId) -> Result<usize> {
    let effect_group = VoiceGroup(1);
    mixer.set_group_limit(effect_group, 3)?;
    let music = music()?;
    let mut decoded_asset_bytes = std::mem::size_of_val(music.samples());
    let tone_wav = Clip::from_bytes(
        include_bytes!("../assets/tone.wav"),
        Some(AudioFormat::Wav),
        "tone.wav",
    )?;
    let tone_ogg = Clip::from_bytes(
        include_bytes!("../assets/tone.ogg"),
        Some(AudioFormat::Ogg),
        "tone.ogg",
    )?;
    decoded_asset_bytes += std::mem::size_of_val(tone_wav.samples());
    decoded_asset_bytes += std::mem::size_of_val(tone_ogg.samples());
    let _music = mixer.play(
        music,
        PlaybackConfig {
            bus: music_bus,
            looping: LoopMode::Whole,
            ..Default::default()
        },
    )?;
    for (index, frequency) in [440.0, 660.0, 880.0].into_iter().enumerate() {
        let clip = effect(frequency, 0.35)?;
        decoded_asset_bytes += std::mem::size_of_val(clip.samples());
        mixer.play_at(
            clip,
            PlaybackConfig {
                bus: sfx_bus,
                group: Some(effect_group),
                pan: index as f32 - 1.0,
                priority: index as i32,
                ..Default::default()
            },
            2_400 + index as u64 * 2_400,
        )?;
    }
    let _wav = mixer.play(
        tone_wav,
        PlaybackConfig {
            bus: sfx_bus,
            ..Default::default()
        },
    )?;
    let _ogg = mixer.play(
        tone_ogg,
        PlaybackConfig {
            bus: sfx_bus,
            ..Default::default()
        },
    )?;
    let _procedural = mixer.play(
        SoundSource::Procedural(Box::new(Beep {
            left: RATE as usize / 2,
            frequency: 1200.0,
            phase: 0.0,
        })),
        PlaybackConfig {
            bus: sfx_bus,
            ..Default::default()
        },
    )?;
    let spatial_clip = effect(330.0, 1.0)?;
    decoded_asset_bytes += std::mem::size_of_val(spatial_clip.samples());
    let _spatial = mixer.play(
        spatial_clip,
        PlaybackConfig {
            bus: sfx_bus,
            spatial: Some(SpatialConfig {
                position: [8.0, 0.0, 0.0],
                model: SpatialModelKind::Inverse {
                    min_distance: 1.0,
                    max_distance: 32.0,
                    rolloff: 1.0,
                },
                pan_strength: 1.0,
            }),
            ..Default::default()
        },
    )?;
    Ok(decoded_asset_bytes)
}

fn polyphony_benchmark() -> Result<serde_json::Value> {
    const VOICES: usize = 64;
    let mut mixer = Mixer::new(MixerConfig {
        sample_rate: RATE,
        channels: CHANNELS,
        max_voices: VOICES,
        command_capacity: 64,
    })?;
    let bus = mixer.create_bus("polyphony", 1.0, VOICES)?;
    let frames = RATE as usize;
    let samples = (0..frames)
        .map(|index| (index as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin() * 0.01)
        .collect::<Vec<_>>();
    let clip = Clip::from_samples(samples, 1, RATE, "polyphony reference")?;
    for index in 0..VOICES {
        mixer.play(
            clip.clone(),
            PlaybackConfig {
                bus,
                pan: (index as f32 / (VOICES - 1) as f32) * 2.0 - 1.0,
                priority: index as i32,
                ..Default::default()
            },
        )?;
    }
    let mut output = vec![0.0f32; frames * CHANNELS as usize];
    let allocations = ALLOCS.load(Ordering::Relaxed);
    let bytes = ALLOC_BYTES.load(Ordering::Relaxed);
    let started = Instant::now();
    mixer.render_interleaved(frames, &mut output)?;
    let elapsed = started.elapsed();
    let mix_allocations = ALLOCS.load(Ordering::Relaxed) - allocations;
    let mix_allocated_bytes = ALLOC_BYTES.load(Ordering::Relaxed) - bytes;
    let diagnostics = mixer.diagnostics();
    Ok(serde_json::json!({
        "voices": VOICES,
        "frames": frames,
        "mix_ms": elapsed.as_secs_f64() * 1000.0,
        "mix_ns_per_frame": elapsed.as_nanos() as f64 / frames as f64,
        "ns_per_voice_frame": elapsed.as_nanos() as f64 / (frames * VOICES) as f64,
        "allocations": mix_allocations,
        "allocated_bytes": mix_allocated_bytes,
        "peak": diagnostics.peak,
        "clipped_frames": diagnostics.clipped_frames,
        "voices_finished": diagnostics.voices_finished,
    }))
}

fn streaming_benchmark() -> Result<serde_json::Value> {
    let stream =
        StreamSource::from_bytes(include_bytes!("../assets/tone.ogg").to_vec(), "tone.ogg")?;
    let mut mixer = Mixer::new(MixerConfig {
        sample_rate: RATE,
        channels: CHANNELS,
        max_voices: 2,
        command_capacity: 8,
    })?;
    let allocations = ALLOCS.load(Ordering::Relaxed);
    let bytes = ALLOC_BYTES.load(Ordering::Relaxed);
    let started = Instant::now();
    mixer.play_stream(stream, PlaybackConfig::default())?;
    let chunk_frames = (RATE / 1000) as usize;
    let mut output = vec![0.0f32; chunk_frames * CHANNELS as usize];
    let mut rendered_frames = 0usize;
    let mut first_audio_ms = None;
    let deadline = started + Duration::from_millis(750);
    while Instant::now() < deadline {
        mixer.render_interleaved(chunk_frames, &mut output)?;
        rendered_frames += chunk_frames;
        if first_audio_ms.is_none() && output.iter().any(|sample| sample.abs() > 0.001) {
            first_audio_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
        // Keep the producer on a real-time schedule rather than outrunning it.
        let target = started + Duration::from_secs_f64(rendered_frames as f64 / f64::from(RATE));
        if let Some(wait) = target.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    let stream_allocations = ALLOCS.load(Ordering::Relaxed) - allocations;
    let stream_allocated_bytes = ALLOC_BYTES.load(Ordering::Relaxed) - bytes;
    let diagnostics = mixer.diagnostics();
    Ok(serde_json::json!({
        "chunk_frames": chunk_frames,
        "rendered_frames": rendered_frames,
        "first_audio_ms": first_audio_ms,
        "underruns": diagnostics.underruns,
        "decode_errors": diagnostics.decode_errors,
        "active_voices": diagnostics.active_voices,
        "allocations": stream_allocations,
        "allocated_bytes": stream_allocated_bytes,
        // STREAM_QUEUE_DEPTH (4) * STREAM_CHUNK_FRAMES (1024) * stereo * f32
        "queue_memory_bound_bytes": 4 * 1024 * 2 * 4,
    }))
}

fn offline(options: &Options) -> Result {
    let frames = options.frames.unwrap_or((RATE as usize) * 4);
    let output = options
        .offline
        .clone()
        .unwrap_or_else(|| "audio-example.wav".into());
    let (mut mixer, music_bus, sfx_bus) = build_mixer()?;
    let decoded_asset_bytes = schedule_demo(&mut mixer, music_bus, sfx_bus)?;
    let allocations = ALLOCS.load(Ordering::Relaxed);
    let bytes = ALLOC_BYTES.load(Ordering::Relaxed);
    let started = Instant::now();
    let pcm = mixer.render(frames)?;
    let elapsed = started.elapsed();
    let mix_allocations = ALLOCS.load(Ordering::Relaxed) - allocations;
    let mix_allocated_bytes = ALLOC_BYTES.load(Ordering::Relaxed) - bytes;
    let diagnostics = mixer.diagnostics();
    // Quantize before hashing so debug/release floating-point contraction
    // differences below one PCM16 step do not change the reference hash.
    let hash = pcm.iter().fold(0xcbf29ce484222325u64, |hash, sample| {
        let quantized = (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        (hash ^ u64::from(quantized as u16)).wrapping_mul(0x100000001b3)
    });
    write_wav(&output, &pcm)?;
    let summary = serde_json::json!({
        "mode": "offline",
        "frames": frames,
        "sample_rate": RATE,
        "channels": CHANNELS,
        "output": output,
        "mix_ms": elapsed.as_secs_f64() * 1000.0,
        "mix_ns_per_frame": elapsed.as_nanos() as f64 / frames.max(1) as f64,
        "hash": format!("{hash:016x}"),
        "peak": diagnostics.peak,
        "voices_started": diagnostics.voices_started,
        "voices_finished": diagnostics.voices_finished,
        "voices_stolen": diagnostics.voices_stolen,
        "underruns": diagnostics.underruns,
        "decode_errors": diagnostics.decode_errors,
        "clipped_frames": diagnostics.clipped_frames,
        "allocations": mix_allocations,
        "allocated_bytes": mix_allocated_bytes,
        "decoded_asset_bytes": decoded_asset_bytes,
        "output_bytes": pcm.len() * 2,
        "polyphony": polyphony_benchmark()?,
        "streaming": streaming_benchmark()?,
    });
    if let Some(path) = &options.benchmark {
        std::fs::write(path, serde_json::to_vec_pretty(&summary)?)?;
        println!("benchmark={}", path.display());
    }
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

#[cfg(feature = "native")]
fn native(options: &Options) -> Result {
    use quasar_audio::{AudioEngine, EngineConfig};
    if options.devices {
        for device in AudioEngine::devices()? {
            println!(
                "{}{}",
                device.name,
                if device.is_default { " (default)" } else { "" }
            );
        }
        return Ok(());
    }
    let engine = AudioEngine::open(None, EngineConfig::default())?;
    let music_bus = engine.create_bus("music", 0.45, 4)?;
    let sfx_bus = engine.create_bus("sfx", 0.8, 32)?;
    engine.set_group_limit(VoiceGroup(1), 4)?;
    engine.set_master_volume(0.8)?;
    let _ = engine.play(
        music()?,
        PlaybackConfig {
            bus: music_bus,
            looping: LoopMode::Whole,
            ..Default::default()
        },
    )?;
    engine.play_stream(
        quasar_audio::StreamSource::from_bytes(
            include_bytes!("../assets/tone.ogg").to_vec(),
            "tone.ogg",
        )?,
        PlaybackConfig {
            bus: sfx_bus,
            ..Default::default()
        },
    )?;
    engine.play(
        effect(660.0, 0.6)?,
        PlaybackConfig {
            bus: sfx_bus,
            group: Some(VoiceGroup(1)),
            spatial: Some(SpatialConfig {
                position: [6.0, 0.0, 0.0],
                ..Default::default()
            }),
            ..Default::default()
        },
    )?;
    std::thread::sleep(Duration::from_secs(
        options
            .frames
            .map_or(2, |frames| (frames as f64 / RATE as f64).ceil() as u64),
    ));
    let diagnostics = engine.diagnostics();
    println!("{:?}", diagnostics);
    engine.stop_all()?;
    Ok(())
}

#[cfg(not(feature = "native"))]
fn native(_: &Options) -> Result {
    Err("native playback requires the `native` feature".into())
}

fn main() -> Result {
    let options = Options::parse()?;
    if options.devices {
        return native(&options);
    }
    if options.play {
        return native(&options);
    }
    offline(&options)
}
