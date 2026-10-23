//! Standalone GAL boundaries; MattMC renderer-specific checks stay upstream.
use std::{
    fs,
    path::{Path, PathBuf},
};
fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out
}
#[test]
fn backend_implementations_remain_private() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")).unwrap();
    assert!(source.lines().any(|line| line == "mod backends;"));
    assert!(!source.contains("pub mod backends"));
    assert!(source.contains("#![warn(missing_docs)]"));
}
#[test]
fn native_graphics_dependencies_stay_in_their_backend() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in files(&root) {
        if path
            .file_name()
            .is_some_and(|name| name == "architecture_boundary.rs")
        {
            continue;
        }
        let relative = path.strip_prefix(&root).unwrap().to_string_lossy();
        let source = fs::read_to_string(&path).unwrap();
        for token in ["ash::", "ash_window::", "shaderc::", "glow::"] {
            if source.contains(token) {
                let backend = if token == "glow::" {
                    "backends/opengl/"
                } else {
                    "backends/vulkan/"
                };
                assert!(
                    relative.starts_with(backend),
                    "{relative} exposes {token} outside {backend}"
                );
            }
        }
        assert!(
            !source.contains("crate::render::"),
            "{relative} still depends on MattMC renderers"
        );
        assert!(
            !source.contains("crate::core::"),
            "{relative} still depends on MattMC core"
        );
    }
}
