//! Environment switches. Every `VULKANIC_*` variable is read here so the
//! names that MattMC used before VulkanicGAL moved into Quasar keep working:
//! each legacy `MATTMC_*` name is a deprecated fallback, reported once.

use std::ffi::OsString;
use std::sync::Once;

/// Current names and the deprecated names they replace.
const LEGACY_NAMES: &[(&str, &str)] = &[
    (
        "VULKANIC_GAL_TRACE_PIPELINE_LABEL",
        "MATTMC_GAL_TRACE_PIPELINE_LABEL",
    ),
    (
        "VULKANIC_GAL_TRACE_SAMPLER_LABEL",
        "MATTMC_GAL_TRACE_SAMPLER_LABEL",
    ),
    ("VULKANIC_GAL_VALIDATION", "MATTMC_GAL_VALIDATION"),
    ("VULKANIC_OPENGL_STRICT", "MATTMC_OPENGL_STRICT"),
    ("VULKANIC_RENDERDOC_BACKEND", "MATTMC_RENDERDOC_BACKEND"),
    ("VULKANIC_RENDERDOC_CAPTURE", "MATTMC_RENDERDOC_CAPTURE"),
    (
        "VULKANIC_RENDERDOC_CAPTURE_TEMPLATE",
        "MATTMC_RENDERDOC_CAPTURE_TEMPLATE",
    ),
    (
        "VULKANIC_RENDERDOC_LIBRARY_PATH",
        "MATTMC_RENDERDOC_LIBRARY_PATH",
    ),
    (
        "VULKANIC_RUN_WINDOWED_CONFORMANCE",
        "MATTMC_RUN_WINDOWED_CONFORMANCE",
    ),
    (
        "VULKANIC_GAL_GL_DRAW_STATE",
        "MATTMC_RUST_GAL_GL_DRAW_STATE",
    ),
    (
        "VULKANIC_GAL_TRACE_GL_DRAWS",
        "MATTMC_RUST_GAL_TRACE_GL_DRAWS",
    ),
    ("VULKANIC_SHADER_DUMP_DIR", "MATTMC_RUST_SHADER_DUMP_DIR"),
    ("VULKANIC_TRACY", "MATTMC_RUST_TRACY"),
    (
        "VULKANIC_VULKAN_GPU_TIMESTAMPS",
        "MATTMC_RUST_VULKAN_GPU_TIMESTAMPS",
    ),
    (
        "VULKANIC_VULKAN_PIPELINE_PASS_STATS",
        "MATTMC_RUST_VULKAN_PIPELINE_PASS_STATS",
    ),
    (
        "VULKANIC_VULKAN_PIPELINE_STATS",
        "MATTMC_RUST_VULKAN_PIPELINE_STATS",
    ),
    (
        "VULKANIC_VULKAN_SCOPE_TIMESTAMPS",
        "MATTMC_RUST_VULKAN_SCOPE_TIMESTAMPS",
    ),
    ("VULKANIC_TRACE_BARRIER_LABEL", "MATTMC_TRACE_BARRIER_LABEL"),
    (
        "VULKANIC_TRACE_DESCRIPTOR_REALIZATION",
        "MATTMC_TRACE_DESCRIPTOR_REALIZATION",
    ),
    (
        "VULKANIC_TRACE_GLIBC_ALLOCATOR",
        "MATTMC_TRACE_GLIBC_ALLOCATOR",
    ),
    (
        "VULKANIC_TRACE_GLIBC_ALLOCATOR_VERBOSE",
        "MATTMC_TRACE_GLIBC_ALLOCATOR_VERBOSE",
    ),
    ("VULKANIC_TRACE_GPU_OVERLAP", "MATTMC_TRACE_GPU_OVERLAP"),
    (
        "VULKANIC_TRACE_PIPELINE_PASS_COMPAT",
        "MATTMC_TRACE_PIPELINE_PASS_COMPAT",
    ),
    (
        "VULKANIC_TRACE_RESOURCE_BINDING_OPS",
        "MATTMC_TRACE_RESOURCE_BINDING_OPS",
    ),
    ("VULKANIC_TRACE_SUBMISSIONS", "MATTMC_TRACE_SUBMISSIONS"),
    (
        "VULKANIC_TRACE_VK_NATIVE_PIPELINE_CACHE",
        "MATTMC_TRACE_VK_NATIVE_PIPELINE_CACHE",
    ),
    (
        "VULKANIC_TRACE_VK_SHADER_COMPILE",
        "MATTMC_TRACE_VK_SHADER_COMPILE",
    ),
    (
        "VULKANIC_TRACE_VULKAN_BUFFER_OBJECTS",
        "MATTMC_TRACE_VULKAN_BUFFER_OBJECTS",
    ),
    (
        "VULKANIC_TRACE_VULKAN_BUFFER_PAGES",
        "MATTMC_TRACE_VULKAN_BUFFER_PAGES",
    ),
    (
        "VULKANIC_TRACE_VULKAN_IMAGE_ALLOCATIONS",
        "MATTMC_TRACE_VULKAN_IMAGE_ALLOCATIONS",
    ),
    (
        "VULKANIC_TRACE_VULKAN_RESIDENCY",
        "MATTMC_TRACE_VULKAN_RESIDENCY",
    ),
    ("VULKANIC_TRACE_WHOLE_FRAME", "MATTMC_TRACE_WHOLE_FRAME"),
    ("VULKANIC_VULKAN_DEBUG_LABELS", "MATTMC_VULKAN_DEBUG_LABELS"),
    (
        "VULKANIC_VULKAN_RUNTIME_ALLOCATOR_TRIM",
        "MATTMC_VULKAN_RUNTIME_ALLOCATOR_TRIM",
    ),
    ("VULKANIC_VULKAN_VALIDATE", "MATTMC_VULKAN_VALIDATE"),
];

fn legacy(name: &str) -> Option<&'static str> {
    LEGACY_NAMES
        .iter()
        .find(|(current, _)| *current == name)
        .map(|(_, legacy)| *legacy)
}

fn warn_legacy(legacy: &str, current: &str) {
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        crate::diagnostics::stderr(format_args!(
            "vulkanic-gal: {legacy} is deprecated; set {current} instead (further deprecated names are read silently)"
        ));
    });
}

/// `std::env::var(name)`, falling back to the variable's legacy name.
pub(crate) fn var(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(name) {
        Err(std::env::VarError::NotPresent) => match legacy(name) {
            Some(old) => {
                let value = std::env::var(old);
                if value.is_ok() {
                    warn_legacy(old, name);
                }
                value
            }
            None => Err(std::env::VarError::NotPresent),
        },
        result => result,
    }
}

/// `std::env::var_os(name)`, falling back to the variable's legacy name.
pub(crate) fn var_os(name: &str) -> Option<OsString> {
    std::env::var_os(name).or_else(|| {
        let old = legacy(name)?;
        let value = std::env::var_os(old);
        if value.is_some() {
            warn_legacy(old, name);
        }
        value
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_legacy_name_maps_to_a_vulkanic_name() {
        for (current, legacy) in super::LEGACY_NAMES {
            assert!(current.starts_with("VULKANIC_"), "{current}");
            assert!(legacy.starts_with("MATTMC_"), "{legacy}");
        }
    }
}
