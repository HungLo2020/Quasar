use crate::{
    AssetError, AssetErrorKind, AssetLoader, AssetPath, AssetSource, ErasedLoader, LoadContext,
    SharedSource,
};
#[cfg(feature = "diagnostics")]
use quasar_diagnostics::{Diagnostics, Field, Level};
#[cfg(feature = "hot-reload")]
use std::path::PathBuf;
use std::{
    any::{Any, TypeId, type_name},
    collections::{HashMap, HashSet},
    fmt,
    ops::Deref,
    sync::Arc,
};

/// Monotonic cache identity. Handles remain valid after cache eviction because
/// they own an `Arc` to the loaded value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct AssetId(u64);
impl AssetId {
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// A typed strong reference to a loaded asset.
///
/// Cloning is cheap. The value stays valid even if the cache later evicts it;
/// the cache only removes entries whose `Arc` has no external handles.
pub struct Handle<T> {
    id: AssetId,
    path: AssetPath,
    schema_version: u32,
    revision: u64,
    value: Arc<T>,
}
impl<T> Handle<T> {
    pub fn id(&self) -> AssetId {
        self.id
    }
    pub fn path(&self) -> &AssetPath {
        &self.path
    }
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }
    /// Server revision when this value was loaded. A later invalidation makes
    /// the handle old but does not invalidate the memory it owns.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            path: self.path.clone(),
            schema_version: self.schema_version,
            revision: self.revision,
            value: Arc::clone(&self.value),
        }
    }
}
impl<T> Deref for Handle<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
impl<T> AsRef<T> for Handle<T> {
    fn as_ref(&self) -> &T {
        &self.value
    }
}
impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Handle")
            .field("id", &self.id)
            .field("path", &self.path)
            .field("schema_version", &self.schema_version)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.path == other.path
    }
}
impl<T> Eq for Handle<T> {}

/// Cache limits. Budgets are soft while handles are alive; eviction never
/// invalidates a live [`Handle`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheBudget {
    pub max_bytes: usize,
    pub max_entries: usize,
}
impl Default for CacheBudget {
    fn default() -> Self {
        Self {
            max_bytes: usize::MAX,
            max_entries: usize::MAX,
        }
    }
}
impl CacheBudget {
    pub fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            max_bytes,
            max_entries,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub max_entries: usize,
    pub max_bytes: usize,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct AssetKey {
    path: AssetPath,
    type_id: TypeId,
}

struct CacheEntry {
    id: AssetId,
    value: Arc<dyn Any + Send + Sync>,
    schema_version: u32,
    revision: u64,
    dependencies: Vec<AssetPath>,
    cost: usize,
    last_used: u64,
}

/// Result of invalidating one path and optionally its dependents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidationReport {
    pub paths: Vec<AssetPath>,
    pub revision: u64,
}

struct Mount {
    prefix: Option<AssetPath>,
    source: SharedSource,
}

/// Mounts, typed loaders, dependency metadata and an LRU-style asset cache.
///
/// The server is synchronous and headless. It does not watch files, create
/// threads, or require an editor/build tool.
pub struct AssetServer {
    mounts: Vec<Mount>,
    loaders: HashMap<TypeId, Arc<dyn ErasedLoader>>,
    cache: HashMap<AssetKey, CacheEntry>,
    next_id: u64,
    clock: u64,
    revision: u64,
    budget: CacheBudget,
    #[cfg(feature = "diagnostics")]
    diagnostics: Option<Diagnostics>,
}
impl Default for AssetServer {
    fn default() -> Self {
        Self::new()
    }
}
impl AssetServer {
    pub fn new() -> Self {
        Self {
            mounts: Vec::new(),
            loaders: HashMap::new(),
            cache: HashMap::new(),
            next_id: 0,
            clock: 0,
            revision: 0,
            budget: CacheBudget::default(),
            #[cfg(feature = "diagnostics")]
            diagnostics: None,
        }
    }
    /// Attaches an explicit diagnostics handle. No global logger or runtime is
    /// required; pass `None` to disable asset instrumentation.
    #[cfg(feature = "diagnostics")]
    pub fn set_diagnostics(&mut self, diagnostics: Option<Diagnostics>) {
        self.diagnostics = diagnostics;
    }
    #[cfg(feature = "diagnostics")]
    pub fn diagnostics(&self) -> Option<&Diagnostics> {
        self.diagnostics.as_ref()
    }
    pub fn with_budget(budget: CacheBudget) -> Self {
        let mut server = Self::new();
        server.set_budget(budget);
        server
    }
    pub fn set_budget(&mut self, budget: CacheBudget) {
        self.budget = budget;
        self.trim_cache();
    }
    pub fn budget(&self) -> CacheBudget {
        self.budget
    }
    /// Cache revision. Incremented by invalidation, unload and clear.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    fn bump_revision(&mut self) -> u64 {
        self.revision = self.revision.wrapping_add(1).max(1);
        self.revision
    }
    /// Registers the loader used by [`Self::load`]. A second registration
    /// for the same Rust type replaces the previous instance; use distinct
    /// loader types when two configurations of the same format are needed.
    pub fn register<L: AssetLoader>(&mut self, loader: L) -> &mut Self {
        self.loaders.insert(TypeId::of::<L>(), Arc::new(loader));
        self
    }
    /// Mounts a source at a prefix. An empty prefix is the root mount. Longer
    /// prefixes win; duplicate prefixes are rejected.
    pub fn mount(
        &mut self,
        prefix: impl AsRef<str>,
        source: Arc<dyn AssetSource>,
    ) -> Result<(), AssetError> {
        let prefix = if prefix.as_ref().is_empty() {
            None
        } else {
            Some(AssetPath::parse(prefix)?)
        };
        if self.mounts.iter().any(|mount| mount.prefix == prefix) {
            return Err(AssetError::new(
                AssetErrorKind::DuplicateMount,
                format!(
                    "mount prefix already exists: {}",
                    prefix.as_ref().map_or("", AssetPath::as_str)
                ),
            ));
        }
        self.mounts.push(Mount { prefix, source });
        self.mounts
            .sort_by_key(|mount| std::cmp::Reverse(mount.prefix.as_ref().map_or(0, |p| p.len())));
        Ok(())
    }
    pub fn mount_file(
        &mut self,
        prefix: impl AsRef<str>,
        root: impl AsRef<std::path::Path>,
    ) -> Result<(), AssetError> {
        self.mount(prefix, Arc::new(crate::FileSource::new(root)?))
    }
    #[cfg(feature = "storage")]
    pub fn mount_basalt(
        &mut self,
        prefix: impl AsRef<str>,
        paths: &quasar_storage::GamePaths,
        relative: impl AsRef<std::path::Path>,
    ) -> Result<(), AssetError> {
        self.mount(
            prefix,
            Arc::new(crate::FileSource::from_basalt(paths, relative)?),
        )
    }
    pub fn unmount(&mut self, prefix: impl AsRef<str>) -> bool {
        let prefix = prefix.as_ref();
        let parsed = if prefix.is_empty() {
            None
        } else {
            match AssetPath::parse(prefix) {
                Ok(prefix) => Some(prefix),
                Err(_) => return false,
            }
        };
        let position = self.mounts.iter().position(|mount| mount.prefix == parsed);
        position.is_some_and(|position| {
            self.mounts.remove(position);
            true
        })
    }
    pub fn sources(&self) -> impl Iterator<Item = (&str, &str)> {
        self.mounts.iter().map(|mount| {
            (
                mount.prefix.as_ref().map_or("", AssetPath::as_str),
                mount.source.name(),
            )
        })
    }
    /// Filesystem roots exposed by mounted sources, paired with their virtual
    /// prefixes. Used to construct hot-reload watchers; memory/archive sources
    /// intentionally do not report a root.
    #[cfg(feature = "hot-reload")]
    pub fn watch_roots(&self) -> Vec<(String, PathBuf)> {
        self.mounts
            .iter()
            .filter_map(|mount| {
                mount.source.watch_root().map(|root| {
                    (
                        mount
                            .prefix
                            .as_ref()
                            .map_or(String::new(), |prefix| prefix.to_string()),
                        root.to_path_buf(),
                    )
                })
            })
            .collect()
    }
    pub fn read_limited(
        &self,
        path: impl AsRef<str>,
        max_bytes: usize,
    ) -> Result<Vec<u8>, AssetError> {
        let path = AssetPath::parse(path)?;
        let (source, relative) = self.resolve(&path)?;
        source.read_limited(&AssetPath::parse(relative)?, max_bytes)
    }
    pub fn read(&self, path: impl AsRef<str>) -> Result<Vec<u8>, AssetError> {
        let path = AssetPath::parse(path)?;
        let (source, relative) = self.resolve(&path)?;
        let relative = AssetPath::parse(relative).map_err(|_| {
            AssetError::at(
                AssetErrorKind::NotFound,
                &path,
                "mount prefix is not a file",
            )
        })?;
        source.read(&relative)
    }
    pub fn exists(&self, path: impl AsRef<str>) -> bool {
        let Ok(path) = AssetPath::parse(path) else {
            return false;
        };
        self.resolve(&path)
            .ok()
            .and_then(|(source, relative)| {
                let relative = AssetPath::parse(relative).ok()?;
                Some(source.exists(&relative))
            })
            .unwrap_or(false)
    }
    /// Loads or returns a cached typed asset. Loaders must be registered first.
    pub fn load<L: AssetLoader>(
        &mut self,
        path: impl AsRef<str>,
    ) -> Result<Handle<L::Asset>, AssetError> {
        let path = AssetPath::parse(path)?;
        let result = self.load_inner::<L>(&path);
        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = &self.diagnostics {
            match &result {
                Ok(handle) => {
                    if diagnostics.is_enabled(Level::Debug) {
                        diagnostics.event_with(
                            Level::Debug,
                            "quasar.assets",
                            "load",
                            [
                                Field::new("path", path.as_str()),
                                Field::new("asset_id", handle.id().value()),
                                Field::new("schema", u64::from(handle.schema_version())),
                            ],
                        );
                    }
                }
                Err(error) => diagnostics.failure_with_fields(
                    "quasar.assets",
                    "load",
                    error,
                    [Field::new("path", path.as_str())],
                ),
            }
        }
        result
    }
    fn load_inner<L: AssetLoader>(
        &mut self,
        path: &AssetPath,
    ) -> Result<Handle<L::Asset>, AssetError> {
        let path = path.clone();
        let loader = self
            .loaders
            .get(&TypeId::of::<L>())
            .cloned()
            .ok_or_else(|| {
                AssetError::at(
                    AssetErrorKind::Configuration,
                    &path,
                    format!("loader {} is not registered", type_name::<L>()),
                )
            })?;
        let key = AssetKey {
            path: path.clone(),
            type_id: TypeId::of::<L>(),
        };
        self.clock = self.clock.wrapping_add(1);
        if self
            .cache
            .get(&key)
            .is_some_and(|entry| entry.schema_version != loader.schema_version())
        {
            // A loader of the same Rust type was re-registered with a different
            // schema. Do not serve the old value.
            self.cache.remove(&key);
        }
        if let Some(entry) = self.cache.get_mut(&key) {
            entry.last_used = self.clock;
            let value = entry.value.clone().downcast::<L::Asset>().map_err(|_| {
                AssetError::at(
                    AssetErrorKind::WrongType,
                    &path,
                    "cached value has an unexpected type",
                )
            })?;
            let handle = Handle {
                id: entry.id,
                path,
                schema_version: entry.schema_version,
                revision: entry.revision,
                value,
            };
            #[cfg(feature = "diagnostics")]
            if let Some(diagnostics) = &self.diagnostics
                && diagnostics.is_enabled(Level::Trace)
            {
                diagnostics.event_with(
                    Level::Trace,
                    "quasar.assets",
                    "cache_hit",
                    [
                        Field::new("path", handle.path.as_str()),
                        Field::new("asset_id", handle.id.value()),
                    ],
                );
            }
            return Ok(handle);
        }
        let (source, relative) = self.resolve(&path)?;
        let source_name = source.name().to_owned();
        let relative = AssetPath::parse(relative).map_err(|_| {
            AssetError::at(
                AssetErrorKind::NotFound,
                &path,
                "mount prefix is not a file",
            )
        })?;
        let bytes = source.read_limited(&relative, loader.max_source_bytes())?;
        let resolver = |path: &AssetPath, limit: usize| self.read_limited(path, limit);
        let context = LoadContext::new(&path, &bytes, &source_name, loader.schema_version())
            .with_resolver(&resolver);
        let mut loaded = loader.load(&context)?;
        loaded.dependencies.extend(context.read_dependencies());
        loaded.dependencies.sort();
        loaded.dependencies.dedup();
        if loaded.schema_version != loader.schema_version() {
            return Err(AssetError::at(
                AssetErrorKind::Schema,
                &path,
                format!(
                    "asset reports schema {}, loader expects {}",
                    loaded.schema_version,
                    loader.schema_version()
                ),
            ));
        }
        let any: Arc<dyn Any + Send + Sync> = loaded.value;
        let typed = any.clone().downcast::<L::Asset>().map_err(|_| {
            AssetError::at(
                AssetErrorKind::WrongType,
                &path,
                "loader returned a different asset type",
            )
        })?;
        let cost = bytes.len().max(loaded.memory_size);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = AssetId(self.next_id);
        self.cache.insert(
            key,
            CacheEntry {
                id,
                value: any,
                schema_version: loaded.schema_version,
                revision: self.revision,
                dependencies: loaded.dependencies,
                cost,
                last_used: self.clock,
            },
        );
        self.trim_cache();
        Ok(Handle {
            id,
            path,
            schema_version: loaded.schema_version,
            revision: self.revision,
            value: typed,
        })
    }
    pub fn is_cached(&self, path: impl AsRef<str>) -> bool {
        let Ok(path) = AssetPath::parse(path) else {
            return false;
        };
        self.cache.keys().any(|key| key.path == path)
    }
    /// Removes a cache entry only when no external handle and no cached asset
    /// depends on it. Returns false when the path was not cached.
    pub fn unload(&mut self, path: impl AsRef<str>) -> Result<bool, AssetError> {
        let path = AssetPath::parse(path)?;
        let keys: Vec<AssetKey> = self
            .cache
            .keys()
            .filter(|key| key.path == path)
            .cloned()
            .collect();
        if keys.is_empty() {
            return Ok(false);
        }
        for key in &keys {
            let entry = &self.cache[key];
            if Arc::strong_count(&entry.value) > 1 {
                return Err(AssetError::at(
                    AssetErrorKind::Busy,
                    &path,
                    "live handles still reference this asset",
                ));
            }
            if self
                .cache
                .iter()
                .any(|(other, entry)| other.path != path && entry.dependencies.contains(&path))
            {
                return Err(AssetError::at(
                    AssetErrorKind::Dependency,
                    &path,
                    "a cached asset depends on this entry",
                ));
            }
        }
        for key in keys {
            self.cache.remove(&key);
        }
        self.bump_revision();
        Ok(true)
    }
    /// Invalidates cached values for a path and, optionally, every cached asset
    /// that depends on it. Live handles remain valid but report an old
    /// [`Handle::revision`]. Returns the deterministic affected-path set.
    pub fn invalidate(
        &mut self,
        path: impl AsRef<str>,
        cascade: bool,
    ) -> Result<InvalidationReport, AssetError> {
        let path = AssetPath::parse(path)?;
        let mut affected = vec![path.clone()];
        if cascade {
            loop {
                let dependents: Vec<AssetPath> = self
                    .cache
                    .iter()
                    .filter(|(key, entry)| {
                        !affected.contains(&key.path)
                            && entry
                                .dependencies
                                .iter()
                                .any(|dependency| affected.contains(dependency))
                    })
                    .map(|(key, _)| key.path.clone())
                    .collect();
                if dependents.is_empty() {
                    break;
                }
                affected.extend(dependents);
            }
        }
        affected.sort();
        affected.dedup();
        self.cache.retain(|key, _| !affected.contains(&key.path));
        let revision = self.bump_revision();
        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = &self.diagnostics
            && diagnostics.is_enabled(Level::Info)
        {
            diagnostics.event_with(
                Level::Info,
                "quasar.assets",
                "invalidated",
                [
                    Field::new("path", path.as_str()),
                    Field::new("affected", affected.len()),
                    Field::new("revision", revision),
                ],
            );
        }
        Ok(InvalidationReport {
            paths: affected,
            revision,
        })
    }
    /// Drops all cache entries. Existing handles remain valid because they own
    /// their values; the cache simply reloads later.
    pub fn clear_cache(&mut self) {
        self.cache.clear();
        self.bump_revision();
    }
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            entries: self.cache.len(),
            bytes: self.cache.values().map(|entry| entry.cost).sum(),
            max_entries: self.budget.max_entries,
            max_bytes: self.budget.max_bytes,
        }
    }
    pub fn dependencies(&self, path: impl AsRef<str>) -> Vec<AssetPath> {
        let Ok(path) = AssetPath::parse(path) else {
            return Vec::new();
        };
        let mut dependencies = Vec::new();
        for key in self.cache.keys().filter(|key| key.path == path) {
            for dependency in &self.cache[key].dependencies {
                if !dependencies.contains(dependency) {
                    dependencies.push(dependency.clone());
                }
            }
        }
        dependencies
    }
    pub fn dependents(&self, path: impl AsRef<str>) -> Vec<AssetPath> {
        let Ok(path) = AssetPath::parse(path) else {
            return Vec::new();
        };
        let mut dependents = Vec::new();
        for (key, entry) in &self.cache {
            if key.path != path
                && entry.dependencies.contains(&path)
                && !dependents.contains(&key.path)
            {
                dependents.push(key.path.clone());
            }
        }
        dependents
    }
    /// Dependency-first order for a cached path. Missing dependencies are
    /// included as leaves and the requested path is last; cycles produce an
    /// explicit error.
    pub fn load_order(&self, path: impl AsRef<str>) -> Result<Vec<AssetPath>, AssetError> {
        let path = AssetPath::parse(path)?;
        if !self.is_cached(&path) {
            return Err(AssetError::at(
                AssetErrorKind::NotFound,
                &path,
                "asset is not in the cache",
            ));
        }
        let mut order = Vec::new();
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        self.visit_dependencies(&path, &mut visiting, &mut visited, &mut order)?;
        Ok(order)
    }
    fn visit_dependencies(
        &self,
        path: &AssetPath,
        visiting: &mut HashSet<AssetPath>,
        visited: &mut HashSet<AssetPath>,
        order: &mut Vec<AssetPath>,
    ) -> Result<(), AssetError> {
        if visited.contains(path) {
            return Ok(());
        }
        if !visiting.insert(path.clone()) {
            return Err(AssetError::at(
                AssetErrorKind::Dependency,
                path,
                "asset dependency cycle",
            ));
        }
        for key in self.cache.keys().filter(|key| &key.path == path) {
            for dependency in &self.cache[key].dependencies {
                self.visit_dependencies(dependency, visiting, visited, order)?;
            }
        }
        visiting.remove(path);
        visited.insert(path.clone());
        order.push(path.clone());
        Ok(())
    }
    fn resolve(&self, path: &AssetPath) -> Result<(SharedSource, String), AssetError> {
        for mount in &self.mounts {
            let Some(relative) = mount
                .prefix
                .as_ref()
                .map_or(Some(path.as_str()), |prefix| path.strip_prefix(prefix))
            else {
                continue;
            };
            if relative.is_empty() {
                continue;
            }
            return Ok((Arc::clone(&mount.source), relative.to_owned()));
        }
        Err(AssetError::at(
            AssetErrorKind::NotMounted,
            path,
            "no mount covers this path",
        ))
    }
    fn trim_cache(&mut self) {
        loop {
            let over_entries = self.cache.len() > self.budget.max_entries;
            let bytes: usize = self.cache.values().map(|entry| entry.cost).sum();
            let over_bytes = bytes > self.budget.max_bytes;
            if !over_entries && !over_bytes {
                break;
            }
            let mut path_counts: HashMap<AssetPath, usize> = HashMap::new();
            for entry in self.cache.values() {
                for dependency in &entry.dependencies {
                    *path_counts.entry(dependency.clone()).or_insert(0) += 1;
                }
            }
            let candidate = self
                .cache
                .iter()
                .filter(|(key, entry)| {
                    Arc::strong_count(&entry.value) == 1
                        && path_counts.get(&key.path).copied().unwrap_or(0) == 0
                })
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone());
            let Some(candidate) = candidate else {
                break;
            };
            self.cache.remove(&candidate);
        }
    }
}
