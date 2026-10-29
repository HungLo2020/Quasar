use crate::AssetError;
use std::{
    fmt,
    ops::Deref,
    path::{Path, PathBuf},
    str::FromStr,
};

const MAX_PATH: usize = 4096;

/// Validated, portable asset path.
///
/// Paths are UTF-8, relative, `/`-separated and contain no empty, `.` or `..`
/// segments. Backslashes, drive/ADS colons, NULs and control characters are
/// rejected so the same string is meaningful on every target. Construction is
/// the only validated entry point; all mount/read/load APIs accept this type or
/// parse a string through [`AssetPath::parse`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "String"))]
pub struct AssetPath(String);
impl AssetPath {
    pub fn parse(input: impl AsRef<str>) -> Result<Self, AssetError> {
        let input = input.as_ref();
        if input.is_empty() {
            return Err(AssetError::invalid("asset path must not be empty"));
        }
        if input.len() > MAX_PATH {
            return Err(AssetError::invalid(format!(
                "asset path exceeds {MAX_PATH} bytes"
            )));
        }
        if input.starts_with('/') {
            return Err(AssetError::invalid("asset path must be relative"));
        }
        if input.contains('\\') || input.contains(':') || input.contains('\0') {
            return Err(AssetError::invalid(
                "asset path must not contain '\\', ':' or NUL",
            ));
        }
        for segment in input.split('/') {
            if segment.is_empty() {
                return Err(AssetError::invalid(
                    "asset path must not contain empty segments",
                ));
            }
            if segment == "." || segment == ".." {
                return Err(AssetError::invalid(
                    "asset path must not contain '.' or '..' segments",
                ));
            }
            if segment.chars().any(char::is_control) {
                return Err(AssetError::invalid(
                    "asset path must not contain control characters",
                ));
            }
        }
        Ok(Self(input.to_owned()))
    }
    /// Resolve a file-relative dependency, normalizing dot segments without escaping the virtual root.
    pub fn resolve_relative(&self, relative: &str) -> Result<Self, AssetError> {
        if relative.starts_with('/') || relative.contains(['\\', ':', '?', '#']) {
            return Err(AssetError::invalid(
                "dependency must be a relative asset path",
            ));
        }
        let mut parts: Vec<_> = self.0.split('/').collect();
        parts.pop();
        for part in relative.split('/') {
            match part {
                "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(AssetError::invalid("dependency escapes virtual asset root"));
                    }
                }
                "" => return Err(AssetError::invalid("empty dependency path segment")),
                p => parts.push(p),
            }
        }
        Self::parse(parts.join("/"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn join(&self, relative: impl AsRef<str>) -> Result<Self, AssetError> {
        let relative = relative.as_ref();
        if relative.is_empty() {
            return Ok(self.clone());
        }
        Self::parse(format!("{}/{}", self.0, relative))
    }
    pub fn parent(&self) -> Option<Self> {
        self.0
            .rsplit_once('/')
            .and_then(|(parent, _)| (parent != self.0.as_str()).then(|| Self(parent.to_owned())))
    }
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }
    pub fn extension(&self) -> Option<&str> {
        self.file_name()
            .rsplit_once('.')
            .and_then(|(_, extension)| (!extension.is_empty()).then_some(extension))
    }
    pub fn starts_with(&self, prefix: &Self) -> bool {
        self.0 == prefix.0
            || self
                .0
                .strip_prefix(&prefix.0)
                .is_some_and(|rest| rest.starts_with('/'))
    }
    /// Removes a validated mount prefix. Returns an empty string when the paths
    /// are identical; callers that need a file should reject that case.
    pub fn strip_prefix(&self, prefix: &Self) -> Option<&str> {
        if self == prefix {
            return Some("");
        }
        self.0
            .strip_prefix(&prefix.0)
            .and_then(|rest| rest.strip_prefix('/'))
    }
    /// Converts to a platform path. The value is relative and validated, so it
    /// cannot escape a trusted source root by itself.
    pub fn to_path_buf(&self) -> PathBuf {
        Path::new(&self.0).to_path_buf()
    }
}
impl Deref for AssetPath {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl AsRef<str> for AssetPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for AssetPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl FromStr for AssetPath {
    type Err = AssetError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}
#[cfg(feature = "serde")]
impl TryFrom<String> for AssetPath {
    type Error = AssetError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
