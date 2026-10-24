//! Reusable typed configuration and common CLI arguments for Quasar games and
//! standalone Rust applications.
//!
//! The crate owns no global configuration, does not write user files, and does
//! not require a particular project format. Typical precedence is:
//!
//! `defaults < JSON file < environment < CLI --set < programmatic`.
//!
//! Games define their own typed settings on top of [`Config`] and may use
//! [`CommonArgs`] for portable `--headless`, `--resolution`, audio, logging and
//! benchmark flags. Unknown game flags are returned untouched.
//!
//! ```
//! use quasar_config::{CommonArgs, Config};
//!
//! let mut config = Config::new();
//! config.set_default("window.width", 960_u32)?;
//! config.set_file("window.width", 1280_u32)?;
//! let args = CommonArgs::parse_from(["--set", "window.width=1600"])?;
//! args.apply_overrides(&mut config)?;
//! assert_eq!(config.get_u64("window.width")?, 1600);
//! assert_eq!(config.provenance("window.width").unwrap(), quasar_config::Source::Cli);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::Path;
use std::{collections::BTreeMap, error::Error, fmt, path::PathBuf, str::FromStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Source {
    Default,
    File,
    Environment,
    Cli,
    Programmatic,
}
impl Source {
    pub fn rank(self) -> u8 {
        match self {
            Self::Default => 0,
            Self::File => 1,
            Self::Environment => 2,
            Self::Cli => 3,
            Self::Programmatic => 4,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::File => "file",
            Self::Environment => "environment",
            Self::Cli => "cli",
            Self::Programmatic => "programmatic",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    String(String),
    /// JSON/TOML array or table retained as serialized text for complex values.
    Json(String),
}
impl ConfigValue {
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("true") {
            Self::Bool(true)
        } else if value.eq_ignore_ascii_case("false") {
            Self::Bool(false)
        } else if let Ok(value) = value.parse::<i64>() {
            Self::Integer(value)
        } else if let Ok(value) = value.parse::<u64>() {
            Self::Unsigned(value)
        } else if let Ok(value) = value.parse::<f64>() {
            if value.is_finite() {
                Self::Float(value)
            } else {
                Self::String(value.to_string())
            }
        } else {
            Self::String(value.to_owned())
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Integer(value) => match value {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
            Self::Unsigned(value) => match value {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
            Self::String(value) => match value.as_str() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                _ => None,
            },
            Self::Json(_) | Self::Float(_) => None,
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Unsigned(value) => Some(*value),
            Self::Integer(value) => u64::try_from(*value).ok(),
            Self::String(value) => value.parse().ok(),
            Self::Bool(value) => Some(u64::from(*value)),
            Self::Json(_) | Self::Float(_) => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        let value = match self {
            Self::Integer(value) => *value as f64,
            Self::Unsigned(value) => *value as f64,
            Self::Float(value) => *value,
            Self::String(value) => value.parse().ok()?,
            Self::Json(_) | Self::Bool(_) => return None,
        };
        value.is_finite().then_some(value)
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) | Self::Json(value) => Some(value),
            _ => None,
        }
    }
}
impl From<bool> for ConfigValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}
impl From<i64> for ConfigValue {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}
impl From<u64> for ConfigValue {
    fn from(value: u64) -> Self {
        Self::Unsigned(value)
    }
}
impl From<u32> for ConfigValue {
    fn from(value: u32) -> Self {
        Self::Unsigned(u64::from(value))
    }
}
impl From<usize> for ConfigValue {
    fn from(value: usize) -> Self {
        Self::Unsigned(value as u64)
    }
}
impl From<f32> for ConfigValue {
    fn from(value: f32) -> Self {
        Self::Float(f64::from(value))
    }
}
impl From<f64> for ConfigValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}
impl From<&str> for ConfigValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}
impl From<String> for ConfigValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}
impl fmt::Display for ConfigValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => value.fmt(formatter),
            Self::Integer(value) => value.fmt(formatter),
            Self::Unsigned(value) => value.fmt(formatter),
            Self::Float(value) => value.fmt(formatter),
            Self::String(value) | Self::Json(value) => formatter.write_str(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigErrorKind {
    InvalidKey,
    Missing,
    Parse,
    Validation,
    Io,
    Json,
    Toml,
    Migration,
    NewerSchema,
    Cli,
}
#[derive(Debug)]
pub struct ConfigError {
    kind: ConfigErrorKind,
    key: Option<String>,
    source: Option<Source>,
    message: String,
}
impl ConfigError {
    pub fn new(kind: ConfigErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            key: None,
            source: None,
            message: message.into(),
        }
    }
    pub fn at(mut self, key: impl Into<String>, source: Option<Source>) -> Self {
        self.key = Some(key.into());
        self.source = source;
        self
    }
    pub fn kind(&self) -> ConfigErrorKind {
        self.kind
    }
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }
    pub fn source(&self) -> Option<Source> {
        self.source
    }
}
impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(key) = &self.key {
            write!(formatter, "{key}: ")?;
        }
        write!(formatter, "{}", self.message)?;
        if let Some(source) = self.source {
            write!(formatter, " ({})", source.as_str())?;
        }
        Ok(())
    }
}
impl Error for ConfigError {}

#[derive(Clone, Debug)]
struct Entry {
    value: ConfigValue,
    source: Source,
}

/// Ordered key/value configuration with provenance. Keys use dotted paths.
#[derive(Clone, Debug, Default)]
pub struct Config {
    values: BTreeMap<String, Entry>,
}
impl Config {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_default(
        &mut self,
        key: impl Into<String>,
        value: impl Into<ConfigValue>,
    ) -> Result<(), ConfigError> {
        self.set(key, value.into(), Source::Default)
    }
    pub fn set_file(
        &mut self,
        key: impl Into<String>,
        value: impl Into<ConfigValue>,
    ) -> Result<(), ConfigError> {
        self.set(key, value.into(), Source::File)
    }
    pub fn set_cli(
        &mut self,
        key: impl Into<String>,
        value: impl Into<ConfigValue>,
    ) -> Result<(), ConfigError> {
        self.set(key, value.into(), Source::Cli)
    }
    pub fn set_programmatic(
        &mut self,
        key: impl Into<String>,
        value: impl Into<ConfigValue>,
    ) -> Result<(), ConfigError> {
        self.set(key, value.into(), Source::Programmatic)
    }
    /// Applies a value only when the source at least matches the existing source.
    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: ConfigValue,
        source: Source,
    ) -> Result<(), ConfigError> {
        let key = key.into();
        if key.trim().is_empty() {
            return Err(ConfigError::new(
                ConfigErrorKind::InvalidKey,
                "configuration keys must not be empty",
            ));
        }
        if matches!(value, ConfigValue::Float(value) if !value.is_finite()) {
            return Err(ConfigError::new(
                ConfigErrorKind::Validation,
                "configuration floats must be finite",
            )
            .at(key, Some(source)));
        }
        if self
            .values
            .get(&key)
            .is_some_and(|entry| entry.source.rank() > source.rank())
        {
            return Ok(());
        }
        self.values.insert(key, Entry { value, source });
        Ok(())
    }
    pub fn get(&self, key: &str) -> Option<&ConfigValue> {
        self.values.get(key).map(|entry| &entry.value)
    }
    pub fn provenance(&self, key: &str) -> Option<Source> {
        self.values.get(key).map(|entry| entry.source)
    }
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.values.keys().map(String::as_str)
    }
    pub fn entries(&self) -> impl Iterator<Item = (&str, &ConfigValue, Source)> {
        self.values
            .iter()
            .map(|(key, entry)| (key.as_str(), &entry.value, entry.source))
    }
    pub fn require(&self, key: &str) -> Result<&ConfigValue, ConfigError> {
        self.get(key).ok_or_else(|| {
            ConfigError::new(
                ConfigErrorKind::Missing,
                "required configuration key is missing",
            )
            .at(key, None)
        })
    }
    pub fn get_string(&self, key: &str) -> Result<String, ConfigError> {
        let entry = self.entry(key)?;
        Ok(entry.value.to_string())
    }
    pub fn get_bool(&self, key: &str) -> Result<bool, ConfigError> {
        let entry = self.entry(key)?;
        entry.value.as_bool().ok_or_else(|| {
            ConfigError::new(ConfigErrorKind::Parse, "expected a boolean")
                .at(key, Some(entry.source))
        })
    }
    pub fn get_i64(&self, key: &str) -> Result<i64, ConfigError> {
        let entry = self.entry(key)?;
        match &entry.value {
            ConfigValue::Integer(value) => Ok(*value),
            ConfigValue::Unsigned(value) => i64::try_from(*value).map_err(|_| {
                ConfigError::new(ConfigErrorKind::Parse, "integer is out of range")
                    .at(key, Some(entry.source))
            }),
            ConfigValue::String(value) => value.parse::<i64>().map_err(|_| {
                ConfigError::new(ConfigErrorKind::Parse, "expected an integer")
                    .at(key, Some(entry.source))
            }),
            _ => Err(
                ConfigError::new(ConfigErrorKind::Parse, "expected an integer")
                    .at(key, Some(entry.source)),
            ),
        }
    }
    pub fn get_u64(&self, key: &str) -> Result<u64, ConfigError> {
        let entry = self.entry(key)?;
        entry.value.as_u64().ok_or_else(|| {
            ConfigError::new(ConfigErrorKind::Parse, "expected a non-negative integer")
                .at(key, Some(entry.source))
        })
    }
    pub fn get_f32(&self, key: &str) -> Result<f32, ConfigError> {
        Ok(self.get_f64(key)? as f32)
    }
    pub fn get_f64(&self, key: &str) -> Result<f64, ConfigError> {
        let entry = self.entry(key)?;
        entry.value.as_f64().ok_or_else(|| {
            ConfigError::new(ConfigErrorKind::Parse, "expected a finite number")
                .at(key, Some(entry.source))
        })
    }
    pub fn get_resolution(&self, key: &str) -> Result<Resolution, ConfigError> {
        let entry = self.entry(key)?;
        Resolution::parse(&entry.value.to_string()).map_err(|error| {
            ConfigError::new(ConfigErrorKind::Parse, error.to_string()).at(key, Some(entry.source))
        })
    }
    pub fn get_or<T: FromStr>(&self, key: &str, default: T) -> Result<T, ConfigError> {
        match self.get(key) {
            Some(value) => value.to_string().parse::<T>().map_err(|_| {
                ConfigError::new(ConfigErrorKind::Parse, "value has the wrong type").at(key, None)
            }),
            None => Ok(default),
        }
    }
    pub fn validate_u64_range(&self, key: &str, min: u64, max: u64) -> Result<(), ConfigError> {
        let value = self.get_u64(key)?;
        if (min..=max).contains(&value) {
            Ok(())
        } else {
            Err(ConfigError::new(
                ConfigErrorKind::Validation,
                format!("must be in {min}..={max}"),
            )
            .at(key, self.provenance(key)))
        }
    }
    pub fn validate_f64_range(&self, key: &str, min: f64, max: f64) -> Result<(), ConfigError> {
        let value = self.get_f64(key)?;
        if value.is_finite() && (min..=max).contains(&value) {
            Ok(())
        } else {
            Err(ConfigError::new(
                ConfigErrorKind::Validation,
                format!("must be in {min}..={max}"),
            )
            .at(key, self.provenance(key)))
        }
    }
    pub fn validate_non_empty(&self, key: &str) -> Result<(), ConfigError> {
        if self.get_string(key)?.trim().is_empty() {
            Err(
                ConfigError::new(ConfigErrorKind::Validation, "must not be empty")
                    .at(key, self.provenance(key)),
            )
        } else {
            Ok(())
        }
    }
    /// Applies explicit environment variable mappings for game-defined keys.
    pub fn apply_env(&mut self, mappings: &[(&str, &str)]) -> Result<usize, ConfigError> {
        self.apply_env_with(mappings, |variable| std::env::var(variable).ok())
    }
    /// Deterministic environment adapter for tests and embedded hosts.
    pub fn apply_env_with(
        &mut self,
        mappings: &[(&str, &str)],
        mut get: impl FnMut(&str) -> Option<String>,
    ) -> Result<usize, ConfigError> {
        let mut applied = 0;
        for (key, variable) in mappings {
            if let Some(value) = get(variable) {
                self.set(*key, ConfigValue::parse(&value), Source::Environment)?;
                applied += 1;
            }
        }
        Ok(applied)
    }
    #[cfg(feature = "json")]
    pub fn load_json_str(&mut self, text: &str) -> Result<usize, ConfigError> {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|error| ConfigError::new(ConfigErrorKind::Json, error.to_string()))?;
        let serde_json::Value::Object(object) = value else {
            return Err(ConfigError::new(
                ConfigErrorKind::Json,
                "configuration root must be a JSON object",
            ));
        };
        let mut applied = 0;
        flatten_json("", &serde_json::Value::Object(object), &mut |key, value| {
            self.set(key, value, Source::File)?;
            applied += 1;
            Ok(())
        })?;
        Ok(applied)
    }
    #[cfg(feature = "json")]
    pub fn load_json_file(&mut self, path: impl AsRef<Path>) -> Result<usize, ConfigError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|error| {
            ConfigError::new(ConfigErrorKind::Io, format!("{}: {error}", path.display()))
        })?;
        self.load_json_str(&text)
    }
    /// Missing files are not errors; malformed existing files are.
    #[cfg(feature = "json")]
    pub fn load_json_file_if_exists(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<usize, ConfigError> {
        let path = path.as_ref();
        match std::fs::read_to_string(path) {
            Ok(text) => self.load_json_str(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(ConfigError::new(
                ConfigErrorKind::Io,
                format!("{}: {error}", path.display()),
            )),
        }
    }
    /// Loads a TOML document. TOML comments are accepted and nested tables are
    /// flattened into dotted keys.
    #[cfg(feature = "toml")]
    pub fn load_toml_str(&mut self, text: &str) -> Result<usize, ConfigError> {
        let value: toml::Value = toml::from_str(text)
            .map_err(|error| ConfigError::new(ConfigErrorKind::Toml, error.to_string()))?;
        let toml::Value::Table(table) = value else {
            return Err(ConfigError::new(
                ConfigErrorKind::Toml,
                "configuration root must be a TOML table",
            ));
        };
        let mut applied = 0;
        flatten_toml("", &toml::Value::Table(table), &mut |key, value| {
            self.set(key, value, Source::File)?;
            applied += 1;
            Ok(())
        })?;
        Ok(applied)
    }
    #[cfg(feature = "toml")]
    pub fn load_toml_file(&mut self, path: impl AsRef<Path>) -> Result<usize, ConfigError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|error| {
            ConfigError::new(ConfigErrorKind::Io, format!("{}: {error}", path.display()))
        })?;
        self.load_toml_str(&text)
    }
    /// Loads a config file selected explicitly by extension. TOML is the
    /// preferred human-editable format; JSON remains supported.
    pub fn load_file(&mut self, path: impl AsRef<Path>) -> Result<usize, ConfigError> {
        let path = path.as_ref();
        let format = FileFormat::from_path(path).ok_or_else(|| {
            ConfigError::new(
                ConfigErrorKind::Io,
                format!(
                    "{}: configuration format must be .toml or .json",
                    path.display()
                ),
            )
        })?;
        let text = std::fs::read_to_string(path).map_err(|error| {
            ConfigError::new(ConfigErrorKind::Io, format!("{}: {error}", path.display()))
        })?;
        self.load_str(&text, format)
    }
    pub fn load_str(&mut self, _text: &str, format: FileFormat) -> Result<usize, ConfigError> {
        match format {
            #[cfg(feature = "toml")]
            FileFormat::Toml => self.load_toml_str(_text),
            #[cfg(feature = "json")]
            FileFormat::Json => self.load_json_str(_text),
            #[allow(unreachable_patterns)]
            _ => Err(ConfigError::new(
                ConfigErrorKind::Io,
                "requested configuration format support is disabled",
            )),
        }
    }
    pub fn load_file_if_exists(&mut self, path: impl AsRef<Path>) -> Result<usize, ConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(0);
        }
        self.load_file(path)
    }
    /// Deserializes the merged, precedence-resolved scalar map into a strongly
    /// typed Serde structure. Complex arrays should use [`load_file_typed`].
    #[cfg(feature = "json")]
    pub fn deserialize<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
        serde_json::from_value(self.to_json_value())
            .map_err(|error| ConfigError::new(ConfigErrorKind::Json, error.to_string()))
    }
    /// Converts dotted scalar keys into a nested JSON value for Serde.
    #[cfg(feature = "json")]
    pub fn to_json_value(&self) -> serde_json::Value {
        fn insert(
            object: &mut serde_json::Map<String, serde_json::Value>,
            parts: &[&str],
            value: serde_json::Value,
        ) {
            if parts.len() == 1 {
                object.insert(parts[0].to_owned(), value);
                return;
            }
            let entry = object
                .entry(parts[0].to_owned())
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if !entry.is_object() {
                *entry = serde_json::Value::Object(serde_json::Map::new());
            }
            insert(entry.as_object_mut().unwrap(), &parts[1..], value);
        }
        let mut root = serde_json::Map::new();
        for (key, entry) in &self.values {
            let value = match &entry.value {
                ConfigValue::Bool(value) => serde_json::Value::Bool(*value),
                ConfigValue::Integer(value) => serde_json::Value::from(*value),
                ConfigValue::Unsigned(value) => serde_json::Value::from(*value),
                ConfigValue::Float(value) => serde_json::Value::from(*value),
                ConfigValue::String(value) => serde_json::Value::String(value.clone()),
                ConfigValue::Json(value) => serde_json::from_str(value)
                    .unwrap_or_else(|_| serde_json::Value::String(value.clone())),
            };
            let parts: Vec<&str> = key.split('.').collect();
            insert(&mut root, &parts, value);
        }
        serde_json::Value::Object(root)
    }
    #[cfg(feature = "storage")]
    pub fn load_json_basalt_shared(
        &mut self,
        paths: &quasar_storage::GamePaths,
        name: impl AsRef<Path>,
    ) -> Result<usize, ConfigError> {
        self.load_json_file_if_exists(paths.shared_config.join(name))
    }
    pub fn schema_version(&self) -> Result<u64, ConfigError> {
        self.get("version")
            .map_or(Ok(0), |_| self.get_u64("version"))
    }
    /// Runs consecutive migrations. Unknown/newer keys are retained; a version
    /// newer than the target is rejected rather than silently downgraded.
    pub fn migrate(&mut self, target: u64, migrations: &[Migration]) -> Result<(), ConfigError> {
        let mut current = self.schema_version()?;
        if current > target {
            return Err(ConfigError::new(
                ConfigErrorKind::NewerSchema,
                format!("configuration version {current} is newer than supported {target}"),
            ));
        }
        while current < target {
            let migration = migrations
                .iter()
                .find(|migration| migration.from == current)
                .ok_or_else(|| {
                    ConfigError::new(
                        ConfigErrorKind::Migration,
                        format!("no migration from schema version {current}"),
                    )
                })?;
            if migration.to <= current || migration.to > target {
                return Err(ConfigError::new(
                    ConfigErrorKind::Migration,
                    format!("invalid migration {} -> {}", migration.from, migration.to),
                ));
            }
            (migration.apply)(self)?;
            current = migration.to;
            self.set_programmatic("version", current)?;
        }
        Ok(())
    }
    fn entry(&self, key: &str) -> Result<&Entry, ConfigError> {
        self.values.get(key).ok_or_else(|| {
            ConfigError::new(ConfigErrorKind::Missing, "configuration key is missing").at(key, None)
        })
    }
}

pub struct Migration {
    pub from: u64,
    pub to: u64,
    pub apply: fn(&mut Config) -> Result<(), ConfigError>,
}
impl Migration {
    pub fn new(from: u64, to: u64, apply: fn(&mut Config) -> Result<(), ConfigError>) -> Self {
        Self { from, to, apply }
    }
}

#[cfg(feature = "toml")]
fn flatten_toml(
    prefix: &str,
    value: &toml::Value,
    sink: &mut impl FnMut(&str, ConfigValue) -> Result<(), ConfigError>,
) -> Result<(), ConfigError> {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                let key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_toml(&key, value, sink)?;
            }
            Ok(())
        }
        toml::Value::Array(_) => sink(prefix, ConfigValue::Json(value.to_string())),
        toml::Value::String(value) => sink(prefix, ConfigValue::String(value.clone())),
        toml::Value::Integer(value) => sink(prefix, ConfigValue::Integer(*value)),
        toml::Value::Float(value) if value.is_finite() => sink(prefix, ConfigValue::Float(*value)),
        toml::Value::Boolean(value) => sink(prefix, ConfigValue::Bool(*value)),
        toml::Value::Datetime(value) => sink(prefix, ConfigValue::String(value.to_string())),
        toml::Value::Float(_) => Err(ConfigError::new(
            ConfigErrorKind::Validation,
            "TOML floats must be finite",
        )
        .at(prefix, Some(Source::File))),
    }
}
#[cfg(feature = "json")]
fn flatten_json(
    prefix: &str,
    value: &serde_json::Value,
    sink: &mut impl FnMut(&str, ConfigValue) -> Result<(), ConfigError>,
) -> Result<(), ConfigError> {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                let key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_json(&key, value, sink)?;
            }
            Ok(())
        }
        serde_json::Value::Array(_) => sink(prefix, ConfigValue::Json(value.to_string())),
        serde_json::Value::Bool(value) => sink(prefix, ConfigValue::Bool(*value)),
        serde_json::Value::Number(value) => {
            let converted = value
                .as_i64()
                .map(ConfigValue::Integer)
                .or_else(|| value.as_u64().map(ConfigValue::Unsigned))
                .or_else(|| value.as_f64().map(ConfigValue::Float))
                .ok_or_else(|| {
                    ConfigError::new(ConfigErrorKind::Json, "unsupported JSON number")
                })?;
            sink(prefix, converted)
        }
        serde_json::Value::String(value) => sink(prefix, ConfigValue::String(value.clone())),
        serde_json::Value::Null => Err(ConfigError::new(
            ConfigErrorKind::Json,
            "null configuration values are not supported",
        )
        .at(prefix, None)),
    }
}

/// Explicit persisted configuration format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileFormat {
    Toml,
    Json,
}
impl FileFormat {
    /// Detects format by extension. Unknown/absent extensions are rejected
    /// rather than guessed.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "toml" => Some(Self::Toml),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Strongly typed, format-independent file loader. The file extension selects
/// TOML or JSON; no implicit conversion or rewrite occurs.
#[cfg(any(feature = "toml", feature = "json"))]
pub fn load_file_typed<T: serde::de::DeserializeOwned>(
    path: impl AsRef<Path>,
) -> Result<T, ConfigError> {
    let path = path.as_ref();
    let format = FileFormat::from_path(path).ok_or_else(|| {
        ConfigError::new(
            ConfigErrorKind::Io,
            format!(
                "{}: configuration format must be .toml or .json",
                path.display()
            ),
        )
    })?;
    let text = std::fs::read_to_string(path).map_err(|error| {
        ConfigError::new(ConfigErrorKind::Io, format!("{}: {error}", path.display()))
    })?;
    load_str_typed(&text, format)
}
#[cfg(feature = "toml")]
pub fn load_toml_typed<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, ConfigError> {
    toml::from_str(text).map_err(|error| ConfigError::new(ConfigErrorKind::Json, error.to_string()))
}
#[cfg(feature = "json")]
pub fn load_json_typed<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, ConfigError> {
    serde_json::from_str(text)
        .map_err(|error| ConfigError::new(ConfigErrorKind::Json, error.to_string()))
}
#[cfg(any(feature = "toml", feature = "json"))]
pub fn load_str_typed<T: serde::de::DeserializeOwned>(
    text: &str,
    format: FileFormat,
) -> Result<T, ConfigError> {
    match format {
        #[cfg(feature = "toml")]
        FileFormat::Toml => load_toml_typed(text),
        #[cfg(feature = "json")]
        FileFormat::Json => load_json_typed(text),
        #[allow(unreachable_patterns)]
        _ => Err(ConfigError::new(
            ConfigErrorKind::Io,
            "requested configuration format support is disabled",
        )),
    }
}

/// Strongly typed settings loaded through the precedence-aware [`Config`] map.
#[cfg(feature = "json")]
pub struct TypedConfig<T> {
    value: T,
    config: Config,
}
#[cfg(feature = "json")]
impl<T: serde::de::DeserializeOwned> TypedConfig<T> {
    pub fn from_config(config: Config) -> Result<Self, ConfigError> {
        let value = config.deserialize()?;
        Ok(Self { value, config })
    }
    pub fn load_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let mut config = Config::new();
        config.load_file(path)?;
        Self::from_config(config)
    }
    pub fn value(&self) -> &T {
        &self.value
    }
    pub fn into_value(self) -> T {
        self.value
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
}

/// Portable window resolution `WIDTHxHEIGHT`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}
impl Resolution {
    pub fn new(width: u32, height: u32) -> Result<Self, CliError> {
        if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
            return Err(CliError::new(
                format!("{width}x{height}"),
                "resolution dimensions must be 1..=16384",
            ));
        }
        Ok(Self { width, height })
    }
    pub fn parse(value: &str) -> Result<Self, CliError> {
        let (width, height) = value
            .split_once(['x', 'X'])
            .ok_or_else(|| CliError::new(value, "expected WIDTHxHEIGHT"))?;
        if width.is_empty() || height.is_empty() {
            return Err(CliError::new(value, "expected WIDTHxHEIGHT"));
        }
        let width = width
            .parse()
            .map_err(|_| CliError::new(value, "invalid width"))?;
        let height = height
            .parse()
            .map_err(|_| CliError::new(value, "invalid height"))?;
        Self::new(width, height)
    }
}
impl fmt::Display for Resolution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}x{}", self.width, self.height)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliError {
    pub argument: String,
    pub message: String,
}
impl CliError {
    pub fn new(argument: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            argument: argument.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.argument, self.message)
    }
}
impl Error for CliError {}

/// Common options owned by Quasar. Game-defined arguments remain in `remaining`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommonArgs {
    pub headless: Option<bool>,
    pub fullscreen: Option<bool>,
    pub resolution: Option<Resolution>,
    pub no_audio: Option<bool>,
    pub audio_device: Option<String>,
    pub benchmark: Option<bool>,
    pub benchmark_file: Option<PathBuf>,
    pub log_level: Option<String>,
    pub config: Option<PathBuf>,
    pub overrides: Vec<(String, String)>,
    pub help: bool,
    pub version: bool,
    pub remaining: Vec<String>,
}
impl CommonArgs {
    pub fn parse_from<I, S>(args: I) -> Result<Self, CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut parsed = Self::default();
        parsed.parse_into(args)?;
        Ok(parsed)
    }
    pub fn from_env(prefix: &str) -> Result<Self, ConfigError> {
        Self::from_env_with(prefix, |variable| std::env::var(variable).ok())
    }
    pub fn from_env_with(
        prefix: &str,
        mut get: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let prefix = prefix.trim_end_matches('_');
        let mut read = |suffix: &str| get(&format!("{prefix}_{suffix}"));
        let mut args = Self::default();
        if let Some(value) = read("HEADLESS") {
            args.headless = Some(parse_bool("HEADLESS", value)?);
        }
        if let Some(value) = read("FULLSCREEN") {
            args.fullscreen = Some(parse_bool("FULLSCREEN", value)?);
        }
        if let Some(value) = read("RESOLUTION") {
            args.resolution = Some(Resolution::parse(&value).map_err(|error| {
                ConfigError::new(ConfigErrorKind::Parse, error.to_string())
                    .at("RESOLUTION", Some(Source::Environment))
            })?);
        }
        if let Some(value) = read("NO_AUDIO") {
            args.no_audio = Some(parse_bool("NO_AUDIO", value)?);
        }
        args.audio_device = read("AUDIO_DEVICE").filter(|value| !value.is_empty());
        if let Some(value) = read("BENCHMARK") {
            args.benchmark = Some(parse_bool("BENCHMARK", value)?);
        }
        args.benchmark_file = read("BENCHMARK_FILE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        args.log_level = read("LOG_LEVEL").filter(|value| !value.is_empty());
        args.config = read("CONFIG")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Ok(args)
    }
    pub fn parse_into<I, S>(&mut self, args: I) -> Result<(), CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut args = args.into_iter().map(Into::into);
        while let Some(argument) = args.next() {
            if argument == "--" {
                self.remaining.extend(args);
                break;
            }
            let (flag, inline) = argument
                .split_once('=')
                .map_or((argument.as_str(), None), |(flag, value)| {
                    (flag, Some(value))
                });
            match flag {
                "--help" | "-h" => self.help = true,
                "--version" | "-V" => self.version = true,
                "--headless" => self.headless = Some(true),
                "--fullscreen" => self.fullscreen = Some(true),
                "--windowed" => self.fullscreen = Some(false),
                "--no-audio" => self.no_audio = Some(true),
                "--benchmark" => self.benchmark = Some(true),
                "--resolution" => {
                    let value = value(&mut args, inline, flag)?;
                    self.resolution = Some(Resolution::parse(&value)?);
                }
                "--audio-device" => {
                    self.audio_device = Some(value(&mut args, inline, flag)?);
                }
                "--benchmark-file" => {
                    self.benchmark_file = Some(PathBuf::from(value(&mut args, inline, flag)?));
                }
                "--log-level" => {
                    self.log_level = Some(value(&mut args, inline, flag)?);
                }
                "--config" => {
                    self.config = Some(PathBuf::from(value(&mut args, inline, flag)?));
                }
                "--set" => {
                    let value = value(&mut args, inline, flag)?;
                    let (key, value) = value
                        .split_once('=')
                        .ok_or_else(|| CliError::new(flag, "expected KEY=VALUE"))?;
                    if key.trim().is_empty() {
                        return Err(CliError::new(flag, "configuration key must not be empty"));
                    }
                    self.overrides.push((key.to_owned(), value.to_owned()));
                }
                _ => {
                    let has_inline = inline.is_some();
                    self.remaining.push(argument.clone());
                    if has_inline {
                        // `--game-flag=value` remains one token for the game parser.
                        continue;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn is_headless(&self) -> bool {
        self.headless.unwrap_or(false)
    }
    /// Applies common options at CLI precedence. Only explicitly supplied values
    /// are written; file/env defaults for unspecified options are preserved.
    pub fn apply_to_config(&self, config: &mut Config) -> Result<(), ConfigError> {
        if let Some(value) = self.headless {
            config.set_cli("common.headless", value)?;
        }
        if let Some(value) = self.fullscreen {
            config.set_cli("common.fullscreen", value)?;
        }
        if let Some(value) = self.resolution {
            config.set_cli(
                "common.resolution",
                format!("{}x{}", value.width, value.height),
            )?;
        }
        if let Some(value) = self.no_audio {
            config.set_cli("common.no_audio", value)?;
        }
        if let Some(value) = &self.audio_device {
            config.set_cli("common.audio_device", value.clone())?;
        }
        if let Some(value) = self.benchmark {
            config.set_cli("common.benchmark", value)?;
        }
        if let Some(value) = &self.benchmark_file {
            config.set_cli(
                "common.benchmark_file",
                value.to_string_lossy().into_owned(),
            )?;
        }
        if let Some(value) = &self.log_level {
            config.set_cli("common.log_level", value.clone())?;
        }
        if let Some(value) = &self.config {
            config.set_cli("common.config", value.to_string_lossy().into_owned())?;
        }
        Ok(())
    }
    /// Applies repeatable `--set KEY=VALUE` options last.
    pub fn apply_overrides(&self, config: &mut Config) -> Result<(), ConfigError> {
        for (key, value) in &self.overrides {
            config.set_cli(key.clone(), ConfigValue::parse(value))?;
        }
        Ok(())
    }
    pub fn help(&self, program: &str, extra: &str) -> String {
        format!(
            "{program}\n\nCommon options:\n  --headless             Run without a window\n  --fullscreen           Start fullscreen\n  --windowed             Start windowed\n  --resolution WxH       Window or offscreen resolution\n  --no-audio             Disable audio output\n  --audio-device NAME    Select an audio device\n  --benchmark            Enable benchmark mode\n  --benchmark-file FILE  Machine-readable benchmark output\n  --log-level LEVEL      Diagnostics level\n  --config FILE          Read a JSON configuration file\n  --set KEY=VALUE        Override a configuration value (repeatable)\n  --help, -h             Show help\n  --version, -V          Show version\n\n{extra}"
        )
    }
}

fn value(
    args: &mut impl Iterator<Item = String>,
    inline: Option<&str>,
    flag: &str,
) -> Result<String, CliError> {
    let value = match inline {
        Some(value) => value.to_owned(),
        None => args
            .next()
            .ok_or_else(|| CliError::new(flag, "missing value"))?,
    };
    if value.is_empty() {
        Err(CliError::new(flag, "missing value"))
    } else {
        Ok(value)
    }
}
fn parse_bool(name: &str, value: String) -> Result<bool, ConfigError> {
    match ConfigValue::parse(&value).as_bool() {
        Some(value) => Ok(value),
        None => Err(ConfigError::new(
            ConfigErrorKind::Parse,
            format!("{name} must be true/false/1/0"),
        )
        .at(name, Some(Source::Environment))),
    }
}

#[cfg(test)]
mod tests;
