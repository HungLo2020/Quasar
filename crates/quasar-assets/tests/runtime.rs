use quasar_assets::{
    Asset, AssetErrorKind, AssetLoader, AssetPath, AssetServer, AssetSource, BytesLoader,
    CacheBudget, FileSource, LoadContext, MemorySource,
};
use std::sync::Arc;

#[derive(Debug)]
struct Fixture {
    text: String,
    dependencies: Vec<AssetPath>,
    version: u32,
}
impl Asset for Fixture {
    fn dependencies(&self) -> Vec<AssetPath> {
        self.dependencies.clone()
    }
    fn schema_version(&self) -> u32 {
        self.version
    }
    fn memory_size(&self) -> usize {
        64
    }
}
struct FixtureLoader {
    dependencies: Vec<AssetPath>,
    version: u32,
}
impl AssetLoader for FixtureLoader {
    type Asset = Fixture;
    fn load(&self, context: &LoadContext<'_>) -> Result<Self::Asset, quasar_assets::AssetError> {
        Ok(Fixture {
            text: String::from_utf8_lossy(context.bytes()).into_owned(),
            dependencies: self.dependencies.clone(),
            version: self.version,
        })
    }
}

fn loader(dependencies: &[&str]) -> FixtureLoader {
    FixtureLoader {
        dependencies: dependencies
            .iter()
            .map(|path| AssetPath::parse(path).unwrap())
            .collect(),
        version: 1,
    }
}

struct BaseLoader;
impl AssetLoader for BaseLoader {
    type Asset = Fixture;
    fn load(&self, context: &LoadContext<'_>) -> Result<Self::Asset, quasar_assets::AssetError> {
        Ok(Fixture {
            text: String::from_utf8_lossy(context.bytes()).into_owned(),
            dependencies: Vec::new(),
            version: 1,
        })
    }
}

struct ChildLoader;
impl AssetLoader for ChildLoader {
    type Asset = Fixture;
    fn load(&self, context: &LoadContext<'_>) -> Result<Self::Asset, quasar_assets::AssetError> {
        Ok(Fixture {
            text: String::from_utf8_lossy(context.bytes()).into_owned(),
            dependencies: vec![AssetPath::parse("base.txt").unwrap()],
            version: 1,
        })
    }
}

fn memory() -> Arc<MemorySource> {
    let mut source = MemorySource::new("fixture");
    source.insert("a.txt", b"alpha".to_vec()).unwrap();
    source.insert("b.txt", b"beta".to_vec()).unwrap();
    source.insert("base.txt", b"base".to_vec()).unwrap();
    source.insert("child.txt", b"child".to_vec()).unwrap();
    Arc::new(source)
}

#[test]
fn virtual_paths_are_portable_validated_and_join_safely() {
    let path = AssetPath::parse("textures/hero.png").unwrap();
    assert_eq!(path.as_str(), "textures/hero.png");
    assert_eq!(path.file_name(), "hero.png");
    assert_eq!(path.extension(), Some("png"));
    assert_eq!(path.parent().unwrap().as_str(), "textures");
    assert_eq!(
        path.join("frames/idle.png").unwrap().as_str(),
        "textures/hero.png/frames/idle.png"
    );
    let prefix = AssetPath::parse("textures").unwrap();
    assert_eq!(path.strip_prefix(&prefix), Some("hero.png"));
    assert_eq!(
        AssetPath::parse("textures2/hero.png")
            .unwrap()
            .strip_prefix(&prefix),
        None
    );
    for invalid in [
        "",
        "/absolute",
        "a//b",
        "a/./b",
        "a/../b",
        "a\\b",
        "C:asset",
        "a\0b",
        "a/./",
    ] {
        assert!(
            AssetPath::parse(invalid).is_err(),
            "{invalid:?} should be rejected"
        );
    }
}

#[test]
fn mounts_use_the_longest_prefix_and_file_sources_stay_inside_root() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("shared")).unwrap();
    std::fs::write(directory.path().join("root.txt"), b"root").unwrap();
    std::fs::write(directory.path().join("shared/hero.txt"), b"base").unwrap();
    std::fs::create_dir_all(directory.path().join("override")).unwrap();
    std::fs::write(directory.path().join("override/hero.txt"), b"override").unwrap();

    let mut server = AssetServer::new();
    server
        .mount("", Arc::new(FileSource::new(directory.path()).unwrap()))
        .unwrap();
    server
        .mount(
            "shared",
            Arc::new(FileSource::new(directory.path().join("override")).unwrap()),
        )
        .unwrap();
    assert_eq!(server.read("root.txt").unwrap(), b"root");
    assert_eq!(server.read("shared/hero.txt").unwrap(), b"override");
    assert!(server.exists("shared/hero.txt"));
    assert!(!server.exists("shared/missing.txt"));
    assert!(matches!(
        server.mount("", Arc::new(MemorySource::new("again"))),
        Err(error) if error.kind() == AssetErrorKind::DuplicateMount
    ));
    assert!(server.unmount("shared"));
    assert_eq!(server.read("shared/hero.txt").unwrap(), b"base");
    assert!(!server.unmount("shared"));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc", directory.path().join("escape")).unwrap();
        let error = server.read("escape/hostname").unwrap_err();
        assert!(matches!(
            error.kind(),
            AssetErrorKind::InvalidPath | AssetErrorKind::NotFound
        ));
    }
}

#[cfg(feature = "diagnostics")]
#[test]
fn asset_loads_emit_structured_diagnostics_without_a_global_logger() {
    use quasar_diagnostics::{Diagnostics, DiagnosticsConfig, Level, MemorySink};
    let sink = Arc::new(MemorySink::new(16));
    let mut server = AssetServer::new();
    server.set_diagnostics(Some(Diagnostics::with_config(
        sink.clone(),
        DiagnosticsConfig {
            min_level: Level::Debug,
            ..DiagnosticsConfig::default()
        },
    )));
    server.register(BytesLoader);
    server.mount("", memory()).unwrap();
    let _ = server.load::<BytesLoader>("a.txt").unwrap();
    let _ = server.load::<BytesLoader>("missing.txt").unwrap_err();
    assert!(sink.records().iter().any(|record| record.name == "load"));
    assert!(sink.records().iter().any(|record| record.name == "failure"));
}

#[test]
fn built_in_bytes_loader_reads_mounted_content() {
    let mut server = AssetServer::new();
    server.register(BytesLoader);
    server.mount("", memory()).unwrap();
    let bytes = server.load::<BytesLoader>("a.txt").unwrap();
    assert_eq!(bytes.as_slice(), b"alpha");
    assert_eq!(bytes.len(), 5);
}

#[test]
fn cache_is_typed_bounded_and_handles_outlive_eviction() {
    let mut server = AssetServer::with_budget(CacheBudget::new(1024, 1));
    server.register(loader(&[]));
    server.mount("", memory()).unwrap();
    let first = server.load::<FixtureLoader>("a.txt").unwrap();
    assert_eq!(first.text, "alpha");
    assert_eq!(server.cache_stats().entries, 1);
    drop(first);
    let second = server.load::<FixtureLoader>("b.txt").unwrap();
    assert_eq!(second.text, "beta");
    assert_eq!(server.cache_stats().entries, 1);
    let cached = server.load::<FixtureLoader>("b.txt").unwrap();
    assert_eq!(cached.id(), second.id());

    let retained = server.load::<FixtureLoader>("a.txt").unwrap();
    server.clear_cache();
    assert_eq!(retained.text, "alpha");
    assert_eq!(server.cache_stats().entries, 0);
    assert!(!server.is_cached("a.txt"));
    let reloaded = server.load::<FixtureLoader>("a.txt").unwrap();
    assert_ne!(reloaded.id(), retained.id());

    assert!(matches!(
        server.unload("a.txt"),
        Err(error) if error.kind() == AssetErrorKind::Busy
    ));
    drop(reloaded);
    assert!(server.unload("a.txt").unwrap());
    assert!(!server.unload("a.txt").unwrap());
}

#[test]
fn byte_budget_evicts_unreferenced_entries() {
    let mut server = AssetServer::with_budget(CacheBudget::new(6, 10));
    server.register(loader(&[]));
    server.mount("", memory()).unwrap();
    let first = server.load::<FixtureLoader>("a.txt").unwrap();
    assert_eq!(first.text, "alpha");
    drop(first);
    let second = server.load::<FixtureLoader>("b.txt").unwrap();
    assert_eq!(second.text, "beta");
    assert_eq!(server.cache_stats().entries, 1);
    assert!(!server.is_cached("a.txt"));
    assert!(server.is_cached("b.txt"));
}

#[test]
fn dependencies_drive_order_eviction_and_cycles() {
    let mut server = AssetServer::with_budget(CacheBudget::new(1024, 10));
    server.register(BaseLoader).register(ChildLoader);
    server.mount("", memory()).unwrap();
    let base_handle = server.load::<BaseLoader>("base.txt").unwrap();
    let _child = server.load::<ChildLoader>("child.txt").unwrap();
    let base = AssetPath::parse("base.txt").unwrap();
    let child_path = AssetPath::parse("child.txt").unwrap();
    assert_eq!(server.dependencies("child.txt"), vec![base.clone()]);
    assert_eq!(server.dependents("base.txt"), vec![child_path.clone()]);
    assert_eq!(
        server.load_order("child.txt").unwrap(),
        vec![base.clone(), child_path]
    );
    drop(base_handle);
    assert!(matches!(
        server.unload("base.txt"),
        Err(error) if error.kind() == AssetErrorKind::Dependency
    ));

    // A loader whose fixed dependency is its own path creates a graph cycle.
    let mut self_cycle = AssetServer::new();
    self_cycle.register(loader(&["a.txt"]));
    self_cycle.mount("", memory()).unwrap();
    let _first = self_cycle.load::<FixtureLoader>("a.txt").unwrap();
    assert!(matches!(
        self_cycle.load_order("a.txt"),
        Err(error) if error.kind() == AssetErrorKind::Dependency
    ));
}

#[test]
fn schema_missing_content_and_loader_errors_are_explicit() {
    let mut server = AssetServer::new();
    server.mount("", memory()).unwrap();
    assert!(matches!(
        server.load::<FixtureLoader>("a.txt"),
        Err(error) if error.kind() == AssetErrorKind::Configuration
    ));
    assert!(matches!(
        server.read("missing.txt"),
        Err(error) if error.kind() == AssetErrorKind::NotFound
    ));
    server.register(FixtureLoader {
        dependencies: Vec::new(),
        version: 1,
    });
    let cached = server.load::<FixtureLoader>("a.txt").unwrap();
    drop(cached);
    server.clear_cache();
    server.register(FixtureLoader {
        dependencies: Vec::new(),
        version: 2,
    });
    assert!(matches!(
        server.load::<FixtureLoader>("a.txt"),
        Err(error) if error.kind() == AssetErrorKind::Schema
    ));

    let mut empty = AssetServer::new();
    assert!(matches!(
        empty.read("a.txt"),
        Err(error) if error.kind() == AssetErrorKind::NotMounted
    ));
    empty.register(loader(&[]));
    empty
        .mount("", Arc::new(MemorySource::new("empty")))
        .unwrap();
    assert!(matches!(
        empty.load::<FixtureLoader>("a.txt"),
        Err(error) if error.kind() == AssetErrorKind::NotFound
    ));
}

#[test]
fn memory_source_can_be_replaced_and_serde_paths_round_trip() {
    let mut source = MemorySource::new("memory");
    source.insert("one.txt", b"one".to_vec()).unwrap();
    source.insert("two.txt", b"two".to_vec()).unwrap();
    assert_eq!(source.len(), 2);
    assert_eq!(
        source.read(&AssetPath::parse("one.txt").unwrap()).unwrap(),
        b"one"
    );
    source.remove(&AssetPath::parse("one.txt").unwrap());
    assert_eq!(source.len(), 1);

    #[cfg(feature = "serde")]
    {
        let path = AssetPath::parse("textures/hero.png").unwrap();
        let json = serde_json::to_string(&path).unwrap();
        let restored: AssetPath = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, path);
        assert!(serde_json::from_str::<AssetPath>("\"../escape\"").is_err());
    }
}

#[cfg(feature = "storage")]
#[test]
fn basalt_paths_can_be_mounted_without_a_build_tool() {
    use quasar_storage::GamePaths;
    let directory = tempfile::tempdir().unwrap();
    let app = directory.path().join("app");
    std::fs::create_dir_all(app.join("assets")).unwrap();
    std::fs::write(app.join("assets/data.txt"), b"basalt").unwrap();
    let paths = GamePaths {
        root: directory.path().to_path_buf(),
        app,
        save: directory.path().join("data/save"),
        config: directory.path().join("data/config"),
        shared_config: directory.path().join("data/config/shared"),
        cache: directory.path().join("data/cache"),
    };
    let mut server = AssetServer::new();
    server.mount_basalt("", &paths, "assets").unwrap();
    assert_eq!(server.read("data.txt").unwrap(), b"basalt");
    let source = FileSource::from_basalt(&paths, "assets").unwrap();
    assert!(source.exists(&AssetPath::parse("data.txt").unwrap()));
}
