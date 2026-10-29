use crate::{AssetError, AssetErrorKind, AssetPath};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Read-only byte source for one mount.
///
/// Implementations must treat [`AssetPath`] as relative and must not accept
/// paths outside their own root. Errors should retain enough context to identify
/// the failing file.
pub trait AssetSource: Send + Sync {
    fn name(&self) -> &str;
    fn read(&self, path: &AssetPath) -> Result<Vec<u8>, AssetError>;
    /// Read with a caller budget. Custom sources may override to enforce it before allocation.
    fn read_limited(&self, path: &AssetPath, max_bytes: usize) -> Result<Vec<u8>, AssetError> {
        let bytes = self.read(path)?;
        if bytes.len() > max_bytes {
            return Err(AssetError::at(
                AssetErrorKind::Budget,
                path,
                "dependency exceeds read budget",
            ));
        }
        Ok(bytes)
    }
    fn exists(&self, path: &AssetPath) -> bool {
        self.read(path).is_ok()
    }
    /// Optional filesystem root for the `hot-reload` watcher. Memory and
    /// archive sources return `None`.
    fn watch_root(&self) -> Option<&Path> {
        None
    }
}

/// Filesystem source rooted at a canonical directory.
#[derive(Clone, Debug)]
pub struct FileSource {
    root: PathBuf,
    name: String,
    max_file_bytes: usize,
}
impl FileSource {
    /// Canonicalizes `root` so symlinks cannot escape it later.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, AssetError> {
        let root = fs::canonicalize(root.as_ref()).map_err(|error| {
            AssetError::new(
                AssetErrorKind::Io,
                format!("open asset root {}: {error}", root.as_ref().display()),
            )
        })?;
        if !root.is_dir() {
            return Err(AssetError::new(
                AssetErrorKind::Io,
                format!("asset root {} is not a directory", root.display()),
            ));
        }
        Ok(Self {
            name: root.display().to_string(),
            root,
            max_file_bytes: 256 * 1024 * 1024,
        })
    }
    pub fn with_max_file_bytes(mut self, bytes: usize) -> Self {
        self.max_file_bytes = bytes;
        self
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Convenience for Basalt/Cargo projects: mount a directory such as
    /// `paths.app.join("assets")` or `CARGO_MANIFEST_DIR/assets`.
    pub fn from_directory(root: impl AsRef<Path>) -> Result<Self, AssetError> {
        Self::new(root)
    }
    #[cfg(feature = "storage")]
    pub fn from_basalt(
        paths: &quasar_storage::GamePaths,
        relative: impl AsRef<Path>,
    ) -> Result<Self, AssetError> {
        let relative = relative.as_ref();
        let packaged = paths.app.join(relative);
        Self::new(if packaged.is_dir() {
            packaged
        } else {
            paths.root.join(relative)
        })
    }
}
impl AssetSource for FileSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn read(&self, path: &AssetPath) -> Result<Vec<u8>, AssetError> {
        self.read_limited(path, self.max_file_bytes)
    }
    fn read_limited(&self, path: &AssetPath, max_bytes: usize) -> Result<Vec<u8>, AssetError> {
        use std::io::Read;
        let max_bytes = max_bytes.min(self.max_file_bytes);
        let joined = self.root.join(path.to_path_buf());
        let canonical = fs::canonicalize(&joined).map_err(|error| {
            let kind = match error.kind() {
                std::io::ErrorKind::NotFound => AssetErrorKind::NotFound,
                _ => AssetErrorKind::Io,
            };
            AssetError::at(kind, path, error.to_string())
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(AssetError::at(
                AssetErrorKind::InvalidPath,
                path,
                "asset escapes its mount root",
            ));
        }
        let metadata = fs::metadata(&canonical)
            .map_err(|error| AssetError::at(AssetErrorKind::Io, path, error.to_string()))?;
        if !metadata.is_file() {
            return Err(AssetError::at(
                AssetErrorKind::NotFound,
                path,
                "asset is not a regular file",
            ));
        }
        if metadata.len() > max_bytes as u64 {
            return Err(AssetError::at(
                AssetErrorKind::Budget,
                path,
                format!(
                    "file is {} bytes; source limit is {} bytes",
                    metadata.len(),
                    max_bytes
                ),
            ));
        }
        let file = fs::File::open(&canonical)
            .map_err(|error| AssetError::at(AssetErrorKind::Io, path, error.to_string()))?;
        let mut bytes = Vec::with_capacity((metadata.len() as usize).min(max_bytes));
        file.take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| AssetError::at(AssetErrorKind::Io, path, error.to_string()))?;
        if bytes.len() > max_bytes {
            return Err(AssetError::at(
                AssetErrorKind::Budget,
                path,
                "asset grew beyond read budget",
            ));
        }
        Ok(bytes)
    }
    fn exists(&self, path: &AssetPath) -> bool {
        let Ok(joined) = fs::canonicalize(self.root.join(path.to_path_buf())) else {
            return false;
        };
        joined.starts_with(&self.root) && joined.is_file()
    }
    fn watch_root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

/// In-memory source for tests, embedded assets and generated content.
#[derive(Clone, Debug, Default)]
pub struct MemorySource {
    name: String,
    files: BTreeMap<AssetPath, Vec<u8>>,
}
impl MemorySource {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            files: BTreeMap::new(),
        }
    }
    pub fn insert(
        &mut self,
        path: impl AsRef<str>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<(), AssetError> {
        let path = AssetPath::parse(path)?;
        self.files.insert(path, bytes.into());
        Ok(())
    }
    pub fn remove(&mut self, path: &AssetPath) -> Option<Vec<u8>> {
        self.files.remove(path)
    }
    pub fn len(&self) -> usize {
        self.files.len()
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}
impl AssetSource for MemorySource {
    fn name(&self) -> &str {
        &self.name
    }
    fn read(&self, path: &AssetPath) -> Result<Vec<u8>, AssetError> {
        self.files.get(path).cloned().ok_or_else(|| {
            AssetError::at(AssetErrorKind::NotFound, path, "memory asset is missing")
        })
    }
    fn read_limited(&self, path: &AssetPath, max_bytes: usize) -> Result<Vec<u8>, AssetError> {
        let bytes = self.files.get(path).ok_or_else(|| {
            AssetError::at(AssetErrorKind::NotFound, path, "memory asset is missing")
        })?;
        if bytes.len() > max_bytes {
            return Err(AssetError::at(
                AssetErrorKind::Budget,
                path,
                "dependency exceeds read budget",
            ));
        }
        Ok(bytes.clone())
    }
    fn exists(&self, path: &AssetPath) -> bool {
        self.files.contains_key(path)
    }
}

/// Type-erased source used by mounts.
pub(crate) type SharedSource = Arc<dyn AssetSource>;
