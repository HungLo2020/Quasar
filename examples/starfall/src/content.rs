//! Asset and procedural-content loading. This module is headless.

use crate::world::{GameError, GameEvent, Level};
use quasar_assets::{Asset, AssetError, AssetErrorKind, AssetLoader, AssetServer, LoadContext};
use quasar_audio::Clip;
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
                .chunks_exact(3)
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
                .collect(),
            png::ColorType::Grayscale => buffer
                .into_iter()
                .flat_map(|value| [value, value, value, 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => buffer
                .chunks_exact(2)
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

#[derive(Clone)]
pub struct SoundBank {
    pub pickup: Clip,
    pub shot: Clip,
    pub hit: Clip,
    pub win: Clip,
    pub lose: Clip,
    pub portal: Clip,
}
impl SoundBank {
    pub fn generate(sample_rate: u32) -> Result<Self, GameError> {
        Ok(Self {
            pickup: tone(sample_rate, 880., 0.11, 0.55)?,
            shot: tone(sample_rate, 220., 0.08, 0.35)?,
            hit: tone(sample_rate, 110., 0.16, 0.6)?,
            win: arpeggio(sample_rate, &[523.25, 659.25, 783.99, 1046.5], 0.18, 0.5)?,
            lose: tone(sample_rate, 82., 0.55, 0.5)?,
            portal: arpeggio(sample_rate, &[392., 523.25, 659.25], 0.12, 0.35)?,
        })
    }
    pub fn for_event(&self, event: GameEvent) -> Option<&Clip> {
        match event {
            GameEvent::Crystal => Some(&self.pickup),
            GameEvent::Shot => Some(&self.shot),
            GameEvent::DroneDestroyed | GameEvent::Damage => Some(&self.hit),
            GameEvent::PortalReady => Some(&self.portal),
            GameEvent::Won => Some(&self.win),
            GameEvent::Lost => Some(&self.lose),
            GameEvent::Restart => None,
        }
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
    let sounds = SoundBank::generate(sample_rate)?;
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
