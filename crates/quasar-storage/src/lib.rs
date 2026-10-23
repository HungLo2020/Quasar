//! Basalt v1 portable data locations and atomic JSON persistence.
use serde::{Serialize, de::DeserializeOwned};
use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct GamePaths {
    pub root: PathBuf,
    pub app: PathBuf,
    pub save: PathBuf,
    pub config: PathBuf,
    pub shared_config: PathBuf,
    pub cache: PathBuf,
}

impl GamePaths {
    /// Basalt overrides win. Packaged `app/bin/<game>` resolves relative to the
    /// executable, regardless of working directory. An optional development root
    /// lets checkouts keep their data outside Cargo's build directory.
    pub fn discover(development_root: Option<&Path>) -> io::Result<Self> {
        Self::resolve(&std::env::current_exe()?, development_root, |key| {
            std::env::var_os(key)
        })
    }

    fn resolve(
        executable: &Path,
        development_root: Option<&Path>,
        env: impl Fn(&str) -> Option<OsString>,
    ) -> io::Result<Self> {
        let bin = executable
            .parent()
            .ok_or_else(|| io::Error::other("executable has no parent"))?;
        let packaged_root = bin
            .parent()
            .filter(|app| {
                bin.file_name().is_some_and(|n| n == "bin")
                    && app.file_name().is_some_and(|n| n == "app")
            })
            .and_then(Path::parent);
        let root = env("BASALT_GAME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                packaged_root
                    .or(development_root)
                    .unwrap_or(bin)
                    .to_path_buf()
            });
        let location =
            |key: &str, fallback: PathBuf| env(key).map(PathBuf::from).unwrap_or(fallback);
        let config = location("BASALT_CONFIG_DIR", root.join("data/config"));
        Ok(Self {
            app: location("BASALT_APP_DIR", root.join("app")),
            save: location("BASALT_SAVE_DIR", root.join("data/save")),
            shared_config: location("BASALT_SHARED_CONFIG_DIR", config.join("shared")),
            cache: location("BASALT_CACHE_DIR", root.join("data/cache")),
            config,
            root,
        })
    }
}

/// Missing files return None; corrupt/unsupported data must be handled by the
/// caller, rather than silently overwritten with defaults.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut file, value).map_err(io::Error::other)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaged_paths_and_overrides_do_not_depend_on_cwd() {
        let paths = GamePaths::resolve(
            Path::new("/games/Pong/app/bin/pong"),
            Some(Path::new("/dev")),
            |_| None,
        )
        .unwrap();
        assert_eq!(paths.save, Path::new("/games/Pong/data/save"));
        let paths = GamePaths::resolve(
            Path::new("/dev/target/debug/pong"),
            Some(Path::new("/dev")),
            |key| match key {
                "BASALT_GAME_DIR" => Some("/moved/Pong".into()),
                "BASALT_CONFIG_DIR" => Some("/override/config".into()),
                _ => None,
            },
        )
        .unwrap();
        assert_eq!(paths.shared_config, Path::new("/override/config/shared"));
        assert_eq!(paths.save, Path::new("/moved/Pong/data/save"));
    }

    #[test]
    fn atomic_replacement_and_corrupt_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data/save/test.json");
        assert!(read_json::<Vec<u32>>(&path).unwrap().is_none());
        write_json(&path, &vec![1, 2]).unwrap();
        write_json(&path, &vec![3]).unwrap();
        assert_eq!(read_json::<Vec<u32>>(&path).unwrap(), Some(vec![3]));
        fs::write(&path, "broken").unwrap();
        assert!(read_json::<Vec<u32>>(&path).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "broken");
    }
}
