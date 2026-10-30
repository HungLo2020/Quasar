//! VulkanicGAL is the sole dependency entry point for external upstream wgpu
//! and its transitive driver/shader dependencies. It contains no private backend.
//! No other workspace crate may depend on them, directly or through a
//! third-party crate; renderers and games reach the GPU only through
//! VulkanicGAL's public API.
//!
//! `Cargo.lock` records every dependency edge for every feature and target,
//! so the graph check also covers optional and platform-specific
//! dependencies. The source scan catches backend types leaking through a
//! re-export.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The one workspace crate allowed to depend on native graphics crates.
const GAL: &str = "vulkanic-gal";

/// Native graphics API and shader toolchain crates.
const NATIVE_GRAPHICS_CRATES: &[&str] = &[
    "ash",
    "ash-window",
    "glow",
    "naga",
    "shaderc",
    "shaderc-sys",
    "wgpu",
];

/// Crate-name prefixes of the same families (`wgpu-core`, `wgpu-hal`, ...).
const NATIVE_GRAPHICS_PREFIXES: &[&str] = &["wgpu-"];

/// Path tokens that name those crates in Rust source.
const NATIVE_GRAPHICS_TOKENS: &[&str] = &[
    "ash::",
    "ash_window::",
    "glow::",
    "naga::",
    "shaderc::",
    "wgpu::",
    "wgpu_core::",
    "wgpu_hal::",
    "wgpu_types::",
];

fn is_native_graphics_crate(name: &str) -> bool {
    NATIVE_GRAPHICS_CRATES.contains(&name)
        || NATIVE_GRAPHICS_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives at <workspace>/crates/<name>")
        .to_path_buf()
}

#[derive(Debug, Default)]
struct Package {
    name: String,
    version: String,
    /// Path packages (workspace members) have no source.
    has_source: bool,
    dependencies: Vec<String>,
}

impl Package {
    fn id(&self) -> String {
        format!("{} {}", self.name, self.version)
    }
}

fn quoted(value: &str) -> Option<&str> {
    let value = value.trim().trim_end_matches(',');
    value.strip_prefix('"')?.strip_suffix('"')
}

/// Parses the `[[package]]` tables of a version 3/4 lockfile.
fn parse_lockfile(text: &str) -> Vec<Package> {
    let mut packages = Vec::new();
    let mut current: Option<Package> = None;
    let mut in_dependencies = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            packages.extend(current.take());
            current = Some(Package::default());
            in_dependencies = false;
            continue;
        }
        if line.starts_with('[') && !line.starts_with("[[package]]") && !in_dependencies {
            // Another table (for example [metadata]) ends the package list.
            packages.extend(current.take());
            continue;
        }
        let Some(package) = current.as_mut() else {
            continue;
        };
        if in_dependencies {
            if line == "]" {
                in_dependencies = false;
            } else if let Some(dependency) = quoted(line) {
                package.dependencies.push(dependency.to_owned());
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("name = ") {
            package.name = quoted(value).unwrap_or_default().to_owned();
        } else if let Some(value) = line.strip_prefix("version = ") {
            package.version = quoted(value).unwrap_or_default().to_owned();
        } else if line.starts_with("source = ") {
            package.has_source = true;
        } else if let Some(value) = line.strip_prefix("dependencies = [") {
            match value.strip_suffix(']') {
                Some(inline) => package
                    .dependencies
                    .extend(inline.split(',').filter_map(quoted).map(str::to_owned)),
                None => in_dependencies = true,
            }
        }
    }
    packages.extend(current);
    packages
}

struct Graph {
    packages: BTreeMap<String, Package>,
    /// Package ids by crate name, for dependency strings without a version.
    by_name: BTreeMap<String, Vec<String>>,
}

impl Graph {
    fn new(packages: Vec<Package>) -> Self {
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let packages: BTreeMap<_, _> = packages
            .into_iter()
            .map(|package| {
                by_name
                    .entry(package.name.clone())
                    .or_default()
                    .push(package.id());
                (package.id(), package)
            })
            .collect();
        Self { packages, by_name }
    }

    /// Resolves a lockfile dependency string: `name`, `name version`, or
    /// `name version (source)`.
    fn resolve(&self, dependency: &str) -> Vec<String> {
        let mut parts = dependency.split_whitespace();
        let name = parts.next().unwrap_or_default();
        match parts.next() {
            Some(version) => vec![format!("{name} {version}")],
            None => self.by_name.get(name).cloned().unwrap_or_default(),
        }
    }

    /// Every package reachable from the workspace members other than the GAL
    /// without passing through the GAL, with the path that first reached it.
    fn reachable_outside_gal(&self) -> BTreeMap<String, Vec<String>> {
        let mut paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut stack: Vec<String> = self
            .packages
            .values()
            .filter(|package| !package.has_source && package.name != GAL)
            .map(Package::id)
            .collect();
        for root in &stack {
            paths.insert(root.clone(), vec![root.clone()]);
        }
        while let Some(id) = stack.pop() {
            let Some(package) = self.packages.get(&id) else {
                continue;
            };
            let path = paths[&id].clone();
            for dependency in &package.dependencies {
                for next in self.resolve(dependency) {
                    let next_name = next.split(' ').next().unwrap_or_default();
                    if next_name == GAL || paths.contains_key(&next) {
                        continue;
                    }
                    let mut next_path = path.clone();
                    next_path.push(next.clone());
                    paths.insert(next.clone(), next_path);
                    stack.push(next);
                }
            }
        }
        paths
    }
}

fn workspace_graph() -> Graph {
    let lockfile = workspace_root().join("Cargo.lock");
    let text = fs::read_to_string(&lockfile)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", lockfile.display()));
    let packages = parse_lockfile(&text);
    assert!(
        packages.iter().any(|package| package.name == GAL),
        "{} does not list {GAL}; the lockfile parser is out of date",
        lockfile.display()
    );
    Graph::new(packages)
}

#[test]
fn native_graphics_crates_are_reachable_only_through_vulkanic_gal() {
    let graph = workspace_graph();
    let violations: Vec<String> = graph
        .reachable_outside_gal()
        .into_iter()
        .filter(|(id, _)| is_native_graphics_crate(id.split(' ').next().unwrap_or_default()))
        .map(|(_, path)| path.join(" -> "))
        .collect();
    assert!(
        violations.is_empty(),
        "native graphics crates must be used only by {GAL}'s upstream dependency; \
         these dependency paths bypass it:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn vulkanic_gal_remains_the_owner_of_its_backend_dependencies() {
    // Guards the check above against vacuously passing: the GAL itself must
    // still be where the native crates enter the graph.
    let graph = workspace_graph();
    let gal = graph
        .packages
        .values()
        .find(|package| package.name == GAL && !package.has_source)
        .expect("vulkanic-gal is a workspace member");
    assert!(
        gal.dependencies
            .iter()
            .any(|dependency| dependency.split(' ').next() == Some("wgpu")),
        "{GAL} no longer depends on upstream wgpu; update this check's expectations"
    );
}

/// Whether `source` names `token` as a path root, so `ash::` matches
/// `ash::vk` but not `std::hash::Hash`.
fn names_path(source: &str, token: &str) -> bool {
    source.match_indices(token).any(|(index, _)| {
        !source[..index]
            .chars()
            .next_back()
            .is_some_and(|before| before.is_alphanumeric() || before == '_')
    })
}

fn rust_sources(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_source_outside_vulkanic_gal_names_a_native_graphics_crate() {
    let root = workspace_root();
    let gal = root.join("crates").join(GAL);
    let this_crate = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = Vec::new();
    for dir in ["crates", "examples"] {
        rust_sources(&root.join(dir), &mut sources);
    }
    let mut violations = Vec::new();
    for path in sources {
        if path.starts_with(&gal) || path.starts_with(this_crate) {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap_or_default();
        for token in NATIVE_GRAPHICS_TOKENS {
            if names_path(&source, token) {
                violations.push(format!(
                    "{} names {token}",
                    path.strip_prefix(&root).unwrap_or(&path).display()
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "native graphics crates must stay inside {GAL}'s upstream dependency:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn lockfile_parser_reads_inline_and_multiline_dependency_lists() {
    let packages = parse_lockfile(
        r#"
version = 4

[[package]]
name = "game"
version = "0.1.0"
dependencies = [
 "helper",
 "vulkanic-gal",
]

[[package]]
name = "helper"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["wgpu 30.0.1 (registry+https://github.com/rust-lang/crates.io-index)"]

[[package]]
name = "vulkanic-gal"
version = "0.1.0"
dependencies = [
 "ash",
]

[[package]]
name = "ash"
version = "0.38.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "wgpu"
version = "30.0.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
    );
    let graph = Graph::new(packages);
    let reachable = graph.reachable_outside_gal();
    // A third-party crate that pulls in wgpu is caught, with its path...
    assert_eq!(
        reachable["wgpu 30.0.1"],
        ["game 0.1.0", "helper 1.0.0", "wgpu 30.0.1"]
    );
    // ...while the GAL's own dependencies are not reached around it.
    assert!(!reachable.contains_key("ash 0.38.0"));
    assert!(names_path("use ash::vk;", "ash::"));
    assert!(names_path("x(wgpu::Device)", "wgpu::"));
    assert!(!names_path("std::hash::Hash + StateHash::new()", "ash::"));
    assert!(is_native_graphics_crate("wgpu-hal"));
    assert!(!is_native_graphics_crate("raw-window-handle"));
    let expected: BTreeSet<&str> = ["game 0.1.0", "helper 1.0.0", "wgpu 30.0.1"].into();
    assert_eq!(
        reachable
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        expected
    );
}

#[test]
fn gal_is_an_unmodified_upstream_reexport() {
    let source =
        fs::read_to_string(workspace_root().join("crates/vulkanic-gal/src/lib.rs")).unwrap();
    let code: Vec<_> = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect();
    assert_eq!(code, ["pub use wgpu;", "pub use wgpu::*;"]);
    let graph = workspace_graph();
    let package = graph.packages.values().find(|p| p.name == GAL).unwrap();
    assert_eq!(package.dependencies.len(), 1);
    assert_eq!(
        package.dependencies[0].split_whitespace().next(),
        Some("wgpu")
    );
}

#[test]
fn dx12_allocator_and_backend_share_windows_api_types() {
    let graph = workspace_graph();
    let windows_api = |name: &str| {
        let package = graph.packages.values().find(|p| p.name == name).unwrap();
        let dependencies: Vec<_> = package
            .dependencies
            .iter()
            .filter(|d| d.split_whitespace().next() == Some("windows"))
            .flat_map(|d| graph.resolve(d))
            .collect();
        assert_eq!(dependencies.len(), 1, "{name} must select one Windows API");
        dependencies
    };
    // Both libraries pass native COM types across their boundary. Other consumers
    // such as CPAL may legitimately retain an older, independent Windows API.
    assert_eq!(
        windows_api("gpu-allocator"),
        windows_api("wgpu-hal"),
        "the DX12 allocator and backend must resolve the same Windows API types"
    );
}
