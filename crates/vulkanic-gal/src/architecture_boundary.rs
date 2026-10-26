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
/// Where each native graphics crate may be named. Shaderc and SPIRV-Tools
/// are shared by the SPIR-V backends through `backends/spirv`.
const NATIVE_CRATE_HOMES: &[(&str, &[&str])] = &[
    ("ash::", &["backends/vulkan/"]),
    ("ash_window::", &["backends/vulkan/"]),
    ("glow::", &["backends/opengl/"]),
    ("shaderc::", &["backends/vulkan/", "backends/spirv/"]),
    ("wgpu::", &["backends/wgpu/"]),
    ("naga::", &["backends/wgpu/", "backends/spirv/"]),
];

/// Whether `source` names the crate `token` starts, rather than a local
/// module of the same name (`self::wgpu::`, `backends::wgpu::`) or a longer
/// identifier (`hash::`).
fn names_crate(source: &str, token: &str) -> bool {
    source.match_indices(token).any(|(index, _)| {
        let before = &source[..index];
        !before
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !["self::", "super::", "backends::"]
                .iter()
                .any(|local| before.ends_with(local))
    })
}

/// The backend implementation modules under `backends/`.
const BACKENDS: &[&str] = &["vulkan", "opengl", "wgpu"];

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
        for (token, homes) in NATIVE_CRATE_HOMES {
            if names_crate(&source, token) {
                assert!(
                    homes.iter().any(|home| relative.starts_with(home)),
                    "{relative} exposes {token} outside {homes:?}"
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

#[test]
fn backends_do_not_reference_each_other() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backends");
    for backend in BACKENDS {
        for path in files(&root.join(backend)) {
            let source = fs::read_to_string(&path).unwrap();
            for other in BACKENDS.iter().filter(|other| *other != backend) {
                for reference in [
                    format!("backends::{other}"),
                    format!("super::{other}::"),
                    format!("super::super::{other}"),
                ] {
                    assert!(
                        !source.contains(&reference),
                        "{} references the {other} backend ({reference}); share code below backends/ instead",
                        path.display()
                    );
                }
            }
        }
    }
}
