use crate::{AssetError, AssetErrorKind, AssetPath};
use std::ops::Deref;

/// Raw bytes loaded through a mount. Useful for small embedded assets and as a
/// baseline loader; games normally add format-specific typed loaders.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bytes {
    bytes: Vec<u8>,
}
impl Bytes {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_vec(self) -> Vec<u8> {
        self.bytes
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}
impl Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}
impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl Asset for Bytes {
    fn memory_size(&self) -> usize {
        self.bytes.len()
    }
}

/// Loader for [`Bytes`].
#[derive(Clone, Copy, Debug, Default)]
pub struct BytesLoader;
impl AssetLoader for BytesLoader {
    type Asset = Bytes;
    fn load(&self, context: &LoadContext<'_>) -> Result<Self::Asset, AssetError> {
        Ok(Bytes::new(context.bytes().to_vec()))
    }
}

/// A loaded value suitable for the asset cache.
///
/// Implementations keep dependency discovery cheap; the server calls
/// [`Self::dependencies`] once after loading. `schema_version` lets a loader
/// reject incompatible on-disk data and is compared with
/// [`AssetLoader::schema_version`].
pub trait Asset: Send + Sync + 'static {
    fn dependencies(&self) -> Vec<AssetPath> {
        Vec::new()
    }
    fn schema_version(&self) -> u32 {
        1
    }
    /// Approximate retained memory. The server also accounts for source bytes;
    /// zero means "use the source size".
    fn memory_size(&self) -> usize {
        0
    }
}

type DependencyResolver<'a> = dyn Fn(&AssetPath, usize) -> Result<Vec<u8>, AssetError> + 'a;

/// Context passed to a typed loader.
pub struct LoadContext<'a> {
    path: &'a AssetPath,
    bytes: &'a [u8],
    source: &'a str,
    schema_version: u32,
    resolver: Option<&'a DependencyResolver<'a>>,
    reads: std::cell::RefCell<std::collections::BTreeSet<AssetPath>>,
}
impl<'a> LoadContext<'a> {
    pub(crate) fn new(
        path: &'a AssetPath,
        bytes: &'a [u8],
        source: &'a str,
        schema_version: u32,
    ) -> Self {
        Self {
            path,
            bytes,
            source,
            schema_version,
            resolver: None,
            reads: Default::default(),
        }
    }
    pub(crate) fn with_resolver(mut self, resolver: &'a DependencyResolver<'a>) -> Self {
        self.resolver = Some(resolver);
        self
    }
    /// Read through mounted sources and automatically register successful dependency reads.
    pub fn read_relative(
        &self,
        relative: &str,
        max_bytes: usize,
    ) -> Result<(AssetPath, Vec<u8>), AssetError> {
        let path = self.path.resolve_relative(relative)?;
        let resolver = self.resolver.ok_or_else(|| {
            AssetError::new(
                AssetErrorKind::Configuration,
                "dependency resolver is unavailable",
            )
        })?;
        let bytes = resolver(&path, max_bytes)?;
        self.reads.borrow_mut().insert(path.clone());
        Ok((path, bytes))
    }
    pub(crate) fn read_dependencies(&self) -> Vec<AssetPath> {
        self.reads.borrow().iter().cloned().collect()
    }
    pub fn path(&self) -> &AssetPath {
        self.path
    }
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }
    pub fn source(&self) -> &str {
        self.source
    }
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }
    /// Loaders that read a version field from serialized data should call this
    /// with the found value. It produces a normal [`AssetErrorKind::Schema`]
    /// error with the asset path attached.
    pub fn require_schema_version(&self, found: u32) -> Result<(), AssetError> {
        if found == self.schema_version {
            Ok(())
        } else {
            Err(AssetError::at(
                AssetErrorKind::Schema,
                self.path,
                format!(
                    "schema version {found} does not match loader version {}",
                    self.schema_version
                ),
            ))
        }
    }
}

/// Typed synchronous loader. Registration is explicit; the server never guesses
/// a decoder from a file extension.
pub trait AssetLoader: Send + Sync + 'static {
    type Asset: Asset;
    /// Version of the format this loader implements. The loaded asset's
    /// [`Asset::schema_version`] must match.
    fn schema_version(&self) -> u32 {
        1
    }
    fn max_source_bytes(&self) -> usize {
        usize::MAX
    }
    fn load(&self, context: &LoadContext<'_>) -> Result<Self::Asset, AssetError>;
}

pub(crate) struct ErasedAsset {
    pub value: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    pub dependencies: Vec<AssetPath>,
    pub schema_version: u32,
    pub memory_size: usize,
}

pub(crate) trait ErasedLoader: Send + Sync {
    fn load(&self, context: &LoadContext<'_>) -> Result<ErasedAsset, AssetError>;
    fn schema_version(&self) -> u32;
    fn max_source_bytes(&self) -> usize;
}

impl<L: AssetLoader> ErasedLoader for L {
    fn load(&self, context: &LoadContext<'_>) -> Result<ErasedAsset, AssetError> {
        let asset = AssetLoader::load(self, context)?;
        let schema_version = asset.schema_version();
        let dependencies = asset.dependencies();
        let memory_size = asset.memory_size();
        Ok(ErasedAsset {
            value: std::sync::Arc::new(asset),
            dependencies,
            schema_version,
            memory_size,
        })
    }
    fn schema_version(&self) -> u32 {
        AssetLoader::schema_version(self)
    }
    fn max_source_bytes(&self) -> usize {
        AssetLoader::max_source_bytes(self)
    }
}
