//! Asset and procedural-content loading. This module is headless.

use crate::world::{GameError, GameEvent, Level};
use quasar_assets::{Asset, AssetError, AssetErrorKind, AssetLoader, AssetServer, LoadContext};
use quasar_audio::Clip;
use quasar_config::content::{
    ContentFormat, ContentRecord, ContentRegistry, ContentSnapshot, Validator,
};
use quasar_diagnostics::{Diagnostics, Field, Level as LogLevel};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl Asset for TextureData {}
impl Asset for Level {}

pub struct LevelLoader;
impl AssetLoader for LevelLoader {
    type Asset = Level;
    fn load(&self, context: &LoadContext<'_>) -> Result<Level, AssetError> {
        let level: Level = serde_json::from_slice(context.bytes()).map_err(|error| {
            AssetError::new(AssetErrorKind::Loader, format!("level JSON: {error}"))
        })?;
        level
            .validate()
            .map_err(|error| AssetError::new(AssetErrorKind::Schema, error.to_string()))?;
        Ok(level)
    }
}

pub struct TextureLoader;
impl AssetLoader for TextureLoader {
    type Asset = TextureData;
    fn load(&self, context: &LoadContext<'_>) -> Result<TextureData, AssetError> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(context.bytes()));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|error| {
            AssetError::new(AssetErrorKind::Loader, format!("PNG header: {error}"))
        })?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).map_err(|error| {
            AssetError::new(AssetErrorKind::Loader, format!("PNG data: {error}"))
        })?;
        if info.width == 0 || info.height == 0 || info.width > 4096 || info.height > 4096 {
            return Err(AssetError::new(
                AssetErrorKind::Schema,
                "texture dimensions must be 1..=4096",
            ));
        }
        let rgba = match info.color_type {
            png::ColorType::Rgba => buffer,
            png::ColorType::Rgb => buffer
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
                .collect(),
            png::ColorType::Grayscale => buffer
                .into_iter()
                .flat_map(|value| [value, value, value, 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => buffer
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
                .collect(),
            other => {
                return Err(AssetError::new(
                    AssetErrorKind::Schema,
                    format!("unsupported PNG color type {other:?}"),
                ));
            }
        };
        let expected = info.width as usize * info.height as usize * 4;
        if rgba.len() != expected {
            return Err(AssetError::new(
                AssetErrorKind::Schema,
                "decoded PNG is not RGBA8",
            ));
        }
        Ok(TextureData {
            width: info.width,
            height: info.height,
            rgba,
        })
    }
}

/// One procedural sound (`sounds.toml`).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundDef {
    pub id: String,
    pub notes: Vec<f32>,
    pub note_seconds: f32,
    pub gain: f32,
}

impl ContentRecord for SoundDef {
    fn id(&self) -> &str {
        &self.id
    }
    fn validate(&self, v: &mut Validator) {
        v.range("notes", self.notes.len(), 1, 8)
            .check(
                self.notes.iter().all(|n| (20.0..=20_000.0).contains(n)),
                "notes",
                "frequencies must be 20..=20000 Hz",
            )
            .range("note_seconds", f64::from(self.note_seconds), 0.01, 2.0)
            .range("gain", f64::from(self.gain), 0.0, 1.0);
    }
}

/// Which sound a game event plays (`cues.toml`).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CueDef {
    /// Event name (`crystal`, `shot`, ...).
    pub id: String,
    pub sound: String,
}

impl ContentRecord for CueDef {
    fn id(&self) -> &str {
        &self.id
    }
    fn validate(&self, v: &mut Validator) {
        v.check(event_of(&self.id).is_some(), "id", "unknown game event")
            .reference("sound", "sounds", &self.sound);
    }
}

fn event_of(name: &str) -> Option<GameEvent> {
    Some(match name {
        "crystal" => GameEvent::Crystal,
        "shot" => GameEvent::Shot,
        "drone_destroyed" => GameEvent::DroneDestroyed,
        "damage" => GameEvent::Damage,
        "portal_ready" => GameEvent::PortalReady,
        "won" => GameEvent::Won,
        "lost" => GameEvent::Lost,
        _ => return None,
    })
}

/// The registry holding Starfall's sound tables; `reload_paths` swaps in
/// edited files after validating both tables together.
pub fn sound_registry() -> ContentRegistry {
    let mut registry = ContentRegistry::new();
    registry
        .register::<SoundDef>("sounds")
        .register::<CueDef>("cues");
    registry
}

#[derive(Clone)]
pub struct SoundBank {
    clips: Vec<(GameEvent, Clip)>,
}
impl SoundBank {
    /// Synthesises every cue of a loaded registry snapshot.
    pub fn from_content(content: &ContentSnapshot, sample_rate: u32) -> Result<Self, GameError> {
        let missing = || GameError::invalid("sound tables are not loaded");
        let sounds = content.table::<SoundDef>("sounds").ok_or_else(missing)?;
        let cues = content.table::<CueDef>("cues").ok_or_else(missing)?;
        let mut clips = Vec::new();
        for cue in cues.records() {
            let sound = sounds.get(&cue.sound).ok_or_else(missing)?;
            let event = event_of(&cue.id).ok_or_else(missing)?;
            clips.push((
                event,
                arpeggio(sample_rate, &sound.notes, sound.note_seconds, sound.gain)?,
            ));
        }
        Ok(Self { clips })
    }
    pub fn for_event(&self, event: GameEvent) -> Option<&Clip> {
        self.clips.iter().find(|(e, _)| *e == event).map(|(_, c)| c)
    }
}

#[derive(Clone)]
pub struct Content {
    pub level: Level,
    pub texture: TextureData,
    pub sounds: SoundBank,
}

pub fn load_content(
    root: PathBuf,
    level_path: String,
    sample_rate: u32,
    diagnostics: Diagnostics,
) -> Result<Content, GameError> {
    let mut server = AssetServer::new();
    server.set_diagnostics(Some(diagnostics.clone()));
    server
        .mount_file("", &root)
        .map_err(|error| GameError::invalid(format!("mount assets: {error}")))?;
    server.register(LevelLoader);
    server.register(TextureLoader);
    let level = server
        .load::<LevelLoader>(&level_path)
        .map_err(|error| GameError::invalid(format!("load level: {error}")))?
        .as_ref()
        .clone();
    let texture = server
        .load::<TextureLoader>("crystal.png")
        .map_err(|error| GameError::invalid(format!("load texture: {error}")))?
        .as_ref()
        .clone();
    let registry = sound_registry();
    registry
        .load_all(&[
            (
                "sounds",
                &read_text(&root, "sounds.toml")?,
                ContentFormat::Toml,
            ),
            ("cues", &read_text(&root, "cues.toml")?, ContentFormat::Toml),
        ])
        .map_err(|error| GameError::invalid(format!("sound tables:\n{error}")))?;
    let sounds = SoundBank::from_content(&registry.snapshot(), sample_rate)?;
    diagnostics.event_with(
        LogLevel::Info,
        "starfall.assets",
        "content_loaded",
        [
            Field::new("level", level_path),
            Field::new("crystals", level.crystals.len()),
            Field::new("drones", level.drones.len()),
            Field::new("texture_width", u64::from(texture.width)),
        ],
    );
    Ok(Content {
        level,
        texture,
        sounds,
    })
}

fn read_text(root: &std::path::Path, file: &str) -> Result<String, GameError> {
    std::fs::read_to_string(root.join(file))
        .map_err(|error| GameError::invalid(format!("read {file}: {error}")))
}

fn tone(sample_rate: u32, frequency: f32, duration: f32, gain: f32) -> Result<Clip, GameError> {
    let frames = (sample_rate as f32 * duration).round().max(1.) as usize;
    let mut samples = Vec::with_capacity(frames);
    for frame in 0..frames {
        let time = frame as f32 / sample_rate as f32;
        let envelope = (1.0 - frame as f32 / frames as f32).powi(2);
        let value = (std::f32::consts::TAU * frequency * time).sin()
            + 0.25 * (std::f32::consts::TAU * frequency * 2. * time).sin();
        samples.push(value * envelope * gain);
    }
    Clip::from_samples(samples, 1, sample_rate, "starfall-tone")
        .map_err(|error| GameError::invalid(error.to_string()))
}
fn arpeggio(
    sample_rate: u32,
    notes: &[f32],
    note_duration: f32,
    gain: f32,
) -> Result<Clip, GameError> {
    let mut samples = Vec::new();
    for note in notes {
        let clip = tone(sample_rate, *note, note_duration, gain)?;
        samples.extend(clip.samples().iter().copied());
    }
    Clip::from_samples(samples, 1, sample_rate, "starfall-arpeggio")
        .map_err(|error| GameError::invalid(error.to_string()))
}
