//! The GLSL→SPIR-V toolchain shared by backends that consume SPIR-V: Vulkan
//! natively, and wgpu through naga. Shaderc and SPIRV-Tools stay here, below
//! the backends; this is not a backend and names none.
//!
//! - `compiler`: Shaderc GLSL→SPIR-V (and the legacy C entry points).
//! - `disk_cache`: opt-in persistent cache of compiled SPIR-V.
//! - `tools`: SPIRV-Tools passes that rewrite modules for consumers with a
//!   narrower SPIR-V subset (separate images and samplers, no calls taking
//!   resources).

pub(super) mod compiler;
pub(super) mod disk_cache;
#[cfg(feature = "wgpu")]
pub(super) mod tools;

/// Compiles GLSL for `stage` to SPIR-V 1.3 (the version naga reads) through
/// Shaderc and the opt-in disk cache.
#[cfg(feature = "wgpu")]
pub(super) fn compile_glsl_cached(
    stage: crate::resources::ShaderStage,
    source: &str,
    label: &str,
    entry_point: &str,
) -> Result<Vec<u8>, String> {
    use crate::resources::ShaderStage;
    let kind = match stage {
        ShaderStage::Vertex => shaderc::ShaderKind::Vertex,
        ShaderStage::Fragment => shaderc::ShaderKind::Fragment,
        ShaderStage::Compute => shaderc::ShaderKind::Compute,
        ShaderStage::Geometry => shaderc::ShaderKind::Geometry,
        ShaderStage::TessControl => shaderc::ShaderKind::TessControl,
        ShaderStage::TessEvaluation => shaderc::ShaderKind::TessEvaluation,
    };
    disk_cache::compile_variant_with_disk_cache(
        disk_cache::cache_dir().as_deref(),
        "vulkan1.1",
        kind,
        entry_point,
        source,
        || {
            compiler::compile_glsl_for_target(
                compiler::SpirvTarget::Vulkan1_1,
                kind,
                source,
                label,
                entry_point,
            )
        },
    )
    .map(|(spirv, _hit)| spirv)
}
