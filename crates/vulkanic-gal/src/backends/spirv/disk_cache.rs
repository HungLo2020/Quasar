//! Persistent GLSL→SPIR-V results for the SPIR-V backends.
//!
//! Shaderc takes 50–80 ms per optimized module, and every launch compiled the
//! same built-in and pack shaders again, stalling the first frame that needed
//! each pipeline. Results are keyed by everything that affects the output and
//! validated on load; a missing or invalid entry simply recompiles. Frozen's
//! OpenGL driver keeps an equivalent on-disk shader cache.

use std::path::{Path, PathBuf};

/// Bump when compile options or key material change. (The name predates
/// VulkanicGAL's move into Quasar; renaming it would discard every cache.)
const SCHEMA: &str = "mattmc-spirv-cache-v1;shaderc-0.10.1;vulkan1.3;auto-bind;auto-map";
const MAX_ENTRIES: usize = 8192;
const SPIRV_MAGIC: u32 = 0x0723_0203;

/// Cache directory explicitly supplied by the application through
/// `VULKANIC_SPIRV_CACHE_DIR`. Unset or `off` disables disk caching. Source dumps
/// also disable caching so a cache hit cannot bypass the requested output.
pub(in crate::backends) fn cache_dir() -> Option<PathBuf> {
    // Source dumps happen inside the compiler; never skip them with a hit.
    if crate::env::var_os("VULKANIC_SHADER_DUMP_DIR").is_some() {
        return None;
    }
    // A library must not pick a game's storage location or write into HOME;
    // the application may name one (`set_default_spirv_cache_dir`).
    match crate::env::var_os("VULKANIC_SPIRV_CACHE_DIR") {
        Some(value) if value == "off" => None,
        Some(value) => Some(PathBuf::from(value)),
        None => crate::create::default_spirv_cache_dir(),
    }
}

#[cfg(test)]
fn key(stage: shaderc::ShaderKind, entry: &str, source: &str) -> u128 {
    variant_key("", stage, entry, source)
}

/// The cache key; a non-empty `variant` separates other compile options
/// while leaving the original keys unchanged.
fn variant_key(variant: &str, stage: shaderc::ShaderKind, entry: &str, source: &str) -> u128 {
    let mut material = Vec::with_capacity(SCHEMA.len() + entry.len() + source.len() + 32);
    if !variant.is_empty() {
        material.extend_from_slice(&(variant.len() as u64).to_le_bytes());
        material.extend_from_slice(variant.as_bytes());
    }
    for part in [
        SCHEMA.as_bytes(),
        &[u8::from(cfg!(debug_assertions)), u8::from(cfg!(test))],
        format!("{stage:?}").as_bytes(),
        entry.as_bytes(),
        source.as_bytes(),
    ] {
        material.extend_from_slice(&(part.len() as u64).to_le_bytes());
        material.extend_from_slice(part);
    }
    xxhash_rust::xxh3::xxh3_128(&material)
}

fn valid_spirv(bytes: &[u8]) -> bool {
    bytes.len() >= 20
        && bytes.len().is_multiple_of(4)
        && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == SPIRV_MAGIC
}

/// Entries in the cache directory: counted once per process, then kept up
/// to date by this process's stores (another process's stores may let the
/// directory exceed the bound slightly, which only costs disk).
static ENTRIES: std::sync::OnceLock<std::sync::atomic::AtomicUsize> = std::sync::OnceLock::new();

fn store(dir: &Path, path: &Path, spirv: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::Ordering;
    std::fs::create_dir_all(dir)?;
    let entries = ENTRIES.get_or_init(|| {
        let count = std::fs::read_dir(dir)
            .map(|entries| entries.take(MAX_ENTRIES + 1).count())
            .unwrap_or(0);
        std::sync::atomic::AtomicUsize::new(count)
    });
    if entries.load(Ordering::Relaxed) > MAX_ENTRIES {
        return Ok(());
    }
    let temporary = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("spirv"),
        std::process::id()
    ));
    std::fs::write(&temporary, spirv)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })?;
    entries.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

/// Returns cached SPIR-V for this exact input, or compiles and stores it.
/// `compile` must be the deterministic backend compiler for these inputs.
#[cfg(feature = "vulkan")]
pub(in crate::backends) fn compile_with_disk_cache(
    dir: Option<&Path>,
    stage: shaderc::ShaderKind,
    entry: &str,
    source: &str,
    compile: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<(Vec<u8>, bool), String> {
    compile_variant_with_disk_cache(dir, "", stage, entry, source, compile)
}

/// `compile_with_disk_cache` for output compiled with other options, named
/// by `variant`.
pub(in crate::backends) fn compile_variant_with_disk_cache(
    dir: Option<&Path>,
    variant: &str,
    stage: shaderc::ShaderKind,
    entry: &str,
    source: &str,
    compile: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<(Vec<u8>, bool), String> {
    let Some(dir) = dir else {
        return compile().map(|spirv| (spirv, false));
    };
    let path = dir.join(format!(
        "{:032x}.spv",
        variant_key(variant, stage, entry, source)
    ));
    if let Ok(bytes) = std::fs::read(&path) {
        if valid_spirv(&bytes) {
            return Ok((bytes, true));
        }
    }
    let spirv = compile()?;
    if valid_spirv(&spirv) {
        // A cache write failure never affects the compiled result.
        let _ = store(dir, &path, &spirv);
    }
    Ok((spirv, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default (Vulkan) cache key space, available in every build.
    fn compile_with_disk_cache_for_test(
        dir: Option<&Path>,
        stage: shaderc::ShaderKind,
        entry: &str,
        source: &str,
        compile: impl FnOnce() -> Result<Vec<u8>, String>,
    ) -> Result<(Vec<u8>, bool), String> {
        compile_variant_with_disk_cache(dir, "", stage, entry, source, compile)
    }

    fn fake_spirv(tag: u8) -> Vec<u8> {
        let mut bytes = SPIRV_MAGIC.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[tag; 20]);
        bytes
    }

    #[test]
    fn reuses_exact_inputs_and_separates_any_difference() {
        let dir =
            std::env::temp_dir().join(format!("mattmc-spirv-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let compile = |tag| move || Ok(fake_spirv(tag));
        let vertex = shaderc::ShaderKind::Vertex;
        let (first, hit) =
            compile_with_disk_cache_for_test(Some(&dir), vertex, "main", "src", compile(1))
                .unwrap();
        assert!(!hit);
        let (again, hit) =
            compile_with_disk_cache_for_test(Some(&dir), vertex, "main", "src", compile(9))
                .unwrap();
        assert!(hit);
        assert_eq!(first, again);
        for (stage, entry, source) in [
            (shaderc::ShaderKind::Fragment, "main", "src"),
            (vertex, "other", "src"),
            (vertex, "main", "src "),
        ] {
            let (_, hit) =
                compile_with_disk_cache_for_test(Some(&dir), stage, entry, source, compile(2))
                    .unwrap();
            assert!(!hit, "{stage:?} {entry} {source:?} must not alias");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_entries_recompile_and_failures_are_not_cached() {
        let dir =
            std::env::temp_dir().join(format!("mattmc-spirv-cache-invalid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let stage = shaderc::ShaderKind::Fragment;
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{:032x}.spv", key(stage, "main", "x")));
        std::fs::write(&path, b"not spirv").unwrap();
        let (bytes, hit) =
            compile_with_disk_cache_for_test(Some(&dir), stage, "main", "x", || Ok(fake_spirv(3)))
                .unwrap();
        assert!(!hit);
        assert_eq!(bytes, fake_spirv(3));
        assert_eq!(std::fs::read(&path).unwrap(), fake_spirv(3));
        let error = compile_with_disk_cache_for_test(Some(&dir), stage, "main", "y", || {
            Err("bad".to_string())
        });
        assert!(error.is_err());
        assert!(!dir
            .join(format!("{:032x}.spv", key(stage, "main", "y")))
            .exists());
        assert!(
            compile_with_disk_cache_for_test(None, stage, "main", "x", || Ok(fake_spirv(4)))
                .unwrap()
                .0
                == fake_spirv(4)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
