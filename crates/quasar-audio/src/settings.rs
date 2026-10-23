//! Optional Basalt-compatible settings adapter.
//!
//! Enable the `settings` feature to persist master/bus volumes, mute state and a
//! preferred device name as schema-versioned JSON. The file path is supplied by
//! the caller, typically from `quasar-storage::GamePaths`, so this module does
//! not hardcode a game directory or depend on Basalt being present.
//!
//! The adapter intentionally does not know game sound names, alias tables or
//! event mappings. It only writes the reusable mixer/device settings that
//! `quasar-audio` owns.

use crate::error::{AudioError, AudioResult};
use crate::mixer::{BusId, Mixer};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Current settings schema version.
pub const SCHEMA_VERSION: u32 = 1;

fn schema_version() -> u32 {
    SCHEMA_VERSION
}
fn default_volume() -> f32 {
    1.0
}

/// Settings for one named bus.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BusSettings {
    /// Linear bus gain in `0..=4`.
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// Whether the bus is muted.
    #[serde(default)]
    pub muted: bool,
    /// Per-bus voice limit; `0` uses the mixer limit.
    #[serde(default)]
    pub max_voices: usize,
}
impl Default for BusSettings {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
            max_voices: 0,
        }
    }
}

/// Persistent, backend-independent audio settings.
///
/// `buses` is keyed by the game's own bus name (`"music"`, `"sfx"`, ...), not a
/// Minecraft category enum or another title-specific identifier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AudioSettings {
    /// Schema version; currently [`SCHEMA_VERSION`].
    #[serde(default = "schema_version")]
    pub schema_version: u32,
    /// Master linear gain in `0..=4`.
    #[serde(default = "default_volume")]
    pub master_volume: f32,
    /// Global mute state.
    #[serde(default)]
    pub muted: bool,
    /// Preferred output device name, if the game supports device selection.
    #[serde(default)]
    pub device: Option<String>,
    /// Named bus settings.
    #[serde(default)]
    pub buses: BTreeMap<String, BusSettings>,
}
impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            master_volume: 1.0,
            muted: false,
            device: None,
            buses: BTreeMap::new(),
        }
    }
}
impl AudioSettings {
    /// Loads settings from `path`, or `None` when the file does not exist.
    ///
    /// Malformed JSON, unknown fields, an unsupported schema version and
    /// out-of-range gains are errors. Callers must not silently overwrite
    /// corrupt settings with defaults.
    pub fn load(path: impl AsRef<Path>) -> AudioResult<Option<Self>> {
        let settings = quasar_storage::read_json::<Self>(path.as_ref())?;
        if let Some(settings) = &settings {
            settings.validate()?;
        }
        Ok(settings)
    }
    /// Atomically writes settings as pretty JSON.
    pub fn save(&self, path: impl AsRef<Path>) -> AudioResult<()> {
        self.validate()?;
        quasar_storage::write_json(path.as_ref(), self)?;
        Ok(())
    }
    /// Validates the schema version, gains and bus names.
    pub fn validate(&self) -> AudioResult<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(AudioError::Unsupported(format!(
                "audio settings schema {}, expected {SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        validate_gain("master_volume", self.master_volume)?;
        if let Some(device) = &self.device
            && device.trim().is_empty()
        {
            return Err(AudioError::InvalidArgument(
                "settings device name must not be empty".into(),
            ));
        }
        for (name, bus) in &self.buses {
            if name.trim().is_empty() {
                return Err(AudioError::InvalidArgument(
                    "settings bus name must not be empty".into(),
                ));
            }
            validate_gain(&format!("bus {name:?} volume"), bus.volume)?;
        }
        Ok(())
    }
    /// Applies these settings to a mixer, creating missing named buses.
    ///
    /// Returns the resolved bus identities in name order so a game can keep its
    /// own mapping. The master bus is not created or renamed.
    pub fn apply_mixer(&self, mixer: &mut Mixer) -> AudioResult<Vec<(String, BusId)>> {
        self.validate()?;
        mixer.set_master_volume(self.master_volume)?;
        mixer.set_muted(self.muted);
        let mut resolved = Vec::with_capacity(self.buses.len());
        for (name, bus) in &self.buses {
            let id = match mixer.bus_id(name) {
                Some(id) => id,
                None => mixer.create_bus(name.as_str(), bus.volume, bus.max_voices)?,
            };
            mixer.set_bus_volume(id, bus.volume)?;
            mixer.set_bus_muted(id, bus.muted)?;
            mixer.set_bus_max_voices(id, bus.max_voices)?;
            resolved.push((name.clone(), id));
        }
        Ok(resolved)
    }
    /// Captures mixer settings for persistence.
    pub fn from_mixer(mixer: &Mixer) -> Self {
        let mut settings = Self {
            master_volume: mixer.master_volume(),
            muted: mixer.muted(),
            ..Self::default()
        };
        for (id, bus) in mixer.buses() {
            if id == BusId::MASTER {
                continue;
            }
            settings.buses.insert(
                bus.name.to_string(),
                BusSettings {
                    volume: bus.volume,
                    muted: bus.muted,
                    max_voices: bus.max_voices,
                },
            );
        }
        settings
    }
    /// Path helper for callers that already resolved a Basalt configuration
    /// directory. The returned path is not created until [`Self::save`].
    pub fn config_path(config_dir: impl AsRef<Path>) -> PathBuf {
        config_dir.as_ref().join("audio.json")
    }
}

fn validate_gain(label: &str, value: f32) -> AudioResult<()> {
    if !value.is_finite() || !(0.0..=4.0).contains(&value) {
        return Err(AudioError::InvalidArgument(format!(
            "{label} must be finite in [0, 4]"
        )));
    }
    Ok(())
}
