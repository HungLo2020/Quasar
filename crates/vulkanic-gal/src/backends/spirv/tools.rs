//! SPIRV-Tools optimizer passes, through the C API that the static Shaderc
//! build already links (`libshaderc_combined` bundles SPIRV-Tools-opt), so
//! no further native dependency is needed.
//!
//! The only use is reshaping Vulkan-flavoured SPIR-V for naga, which reads
//! a narrower subset: combined image samplers must become separate images
//! and samplers, and those split resources must not flow through function
//! parameters (naga loses a sampler's comparison type across calls).

use std::ffi::{c_char, CStr};

#[repr(C)]
struct SpvBinary {
    code: *mut u32,
    word_count: usize,
}

#[repr(C)]
struct SpvOptimizer {
    _private: [u8; 0],
}

#[repr(C)]
struct SpvOptimizerOptions {
    _private: [u8; 0],
}

/// `spv_target_env::SPV_ENV_VULKAN_1_1`, the environment the naga path
/// compiles for (SPIR-V 1.3).
const SPV_ENV_VULKAN_1_1: i32 = 18;
const SPV_SUCCESS: i32 = 0;

extern "C" {
    fn spvOptimizerCreate(env: i32) -> *mut SpvOptimizer;
    fn spvOptimizerDestroy(optimizer: *mut SpvOptimizer);
    fn spvOptimizerRegisterPassFromFlag(optimizer: *mut SpvOptimizer, flag: *const c_char) -> bool;
    fn spvOptimizerRun(
        optimizer: *mut SpvOptimizer,
        binary: *const u32,
        word_count: usize,
        optimized: *mut *mut SpvBinary,
        options: *const SpvOptimizerOptions,
    ) -> i32;
    fn spvOptimizerOptionsCreate() -> *mut SpvOptimizerOptions;
    fn spvOptimizerOptionsDestroy(options: *mut SpvOptimizerOptions);
    fn spvOptimizerOptionsSetRunValidator(options: *mut SpvOptimizerOptions, run: bool);
    fn spvBinaryDestroy(binary: *mut SpvBinary);
}

/// Passes that make Vulkan GLSL output readable by naga, in order. Inlining
/// first removes calls that take images or samplers, so splitting never has
/// to retype a function parameter.
const NAGA_COMPATIBILITY_PASSES: &[&CStr] = &[
    c"--inline-entry-points-exhaustive",
    c"--eliminate-dead-functions",
    c"--split-combined-image-sampler",
];

/// Rewrites `spirv` so every combined image sampler becomes an image and a
/// sampler variable (`<name>_image`, `<name>_sampler`) sharing the original
/// set and binding, with all resource-taking calls inlined. Callers assign
/// the sampler its own binding afterwards.
pub(in crate::backends) fn split_combined_image_samplers(
    spirv: &[u32],
) -> Result<Vec<u32>, String> {
    run_passes(spirv, NAGA_COMPATIBILITY_PASSES)
}

fn run_passes(spirv: &[u32], passes: &[&CStr]) -> Result<Vec<u32>, String> {
    // SAFETY: every pointer passed below is either created by the matching
    // SPIRV-Tools constructor in this function or borrowed from `spirv` for
    // the duration of the call, and each object is destroyed exactly once.
    unsafe {
        let optimizer = spvOptimizerCreate(SPV_ENV_VULKAN_1_1);
        if optimizer.is_null() {
            return Err("SPIRV-Tools optimizer creation failed".into());
        }
        let options = spvOptimizerOptionsCreate();
        if options.is_null() {
            spvOptimizerDestroy(optimizer);
            return Err("SPIRV-Tools optimizer options creation failed".into());
        }
        // Shaderc validated its own output; the passes keep it valid.
        spvOptimizerOptionsSetRunValidator(options, false);
        let result = (|| {
            for pass in passes {
                if !spvOptimizerRegisterPassFromFlag(optimizer, pass.as_ptr()) {
                    return Err(format!(
                        "SPIRV-Tools does not provide pass {}",
                        pass.to_string_lossy()
                    ));
                }
            }
            let mut optimized: *mut SpvBinary = std::ptr::null_mut();
            let status = spvOptimizerRun(
                optimizer,
                spirv.as_ptr(),
                spirv.len(),
                &mut optimized,
                options,
            );
            if status != SPV_SUCCESS || optimized.is_null() {
                if !optimized.is_null() {
                    spvBinaryDestroy(optimized);
                }
                return Err(format!("SPIRV-Tools passes failed with status {status}"));
            }
            let words =
                std::slice::from_raw_parts((*optimized).code, (*optimized).word_count).to_vec();
            spvBinaryDestroy(optimized);
            Ok(words)
        })();
        spvOptimizerOptionsDestroy(options);
        spvOptimizerDestroy(optimizer);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(bytes: &[u8]) -> Vec<u32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|chunk| u32::from_le_bytes(*chunk))
            .collect()
    }

    fn naga_accepts(words: &[u32]) -> Result<(), String> {
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let module = naga::front::spv::parse_u8_slice(&bytes, &Default::default())
            .map_err(|error| format!("{error:?}"))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
    }

    #[test]
    fn splits_combined_samplers_passed_through_functions() {
        let source = "#version 450\n\
            layout(set=0,binding=0) uniform sampler2DShadow shadow;\n\
            layout(set=0,binding=1) uniform usampler2D mask;\n\
            layout(location=0) in vec2 uv;\n\
            layout(location=0) out vec4 color;\n\
            float pcf(sampler2DShadow s, vec3 p) { return texture(s, p); }\n\
            void main() {\n\
                color = vec4(pcf(shadow, vec3(uv, 0.5)) * float(texelFetch(mask, ivec2(uv), 0).r));\n\
            }\n";
        let spirv = crate::backends::spirv::compile_glsl_cached(
            crate::resources::ShaderStage::Fragment,
            source,
            "split",
            "main",
        )
        .expect("shader compiles");
        let input = words(&spirv);
        assert!(
            naga_accepts(&input).is_err(),
            "naga rejects combined samplers"
        );
        let split = split_combined_image_samplers(&input).expect("passes run");
        // The split leaves image and sampler on one binding; give the
        // samplers their own before validating, as the wgpu backend does.
        let bytes: Vec<u8> = split.iter().flat_map(|word| word.to_le_bytes()).collect();
        let mut module = naga::front::spv::parse_u8_slice(&bytes, &Default::default())
            .expect("naga parses split SPIR-V");
        let samplers: Vec<_> = module
            .global_variables
            .iter()
            .filter(|(_, global)| {
                matches!(
                    module.types[global.ty].inner,
                    naga::TypeInner::Sampler { .. }
                )
            })
            .map(|(handle, _)| handle)
            .collect();
        assert_eq!(samplers.len(), 2);
        for handle in samplers {
            if let Some(binding) = module.global_variables[handle].binding.as_mut() {
                binding.binding += 100;
            }
        }
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("split module validates");
    }

    #[test]
    fn struct_copies_from_buffers_reach_naga_without_copy_logical() {
        // The 2D quad shader's pattern: SPIR-V 1.4+ would copy with
        // OpCopyLogical, which naga cannot read.
        let source = "#version 450\n\
            struct Quad { vec4 a; vec4 b; };\n\
            layout(set=0,binding=1,std430) readonly buffer Geometry { Quad quads[]; } geometry;\n\
            void main() { Quad q = geometry.quads[gl_InstanceIndex]; gl_Position = q.a + q.b; }\n";
        let spirv = crate::backends::spirv::compile_glsl_cached(
            crate::resources::ShaderStage::Vertex,
            source,
            "struct-copy",
            "main",
        )
        .expect("shader compiles");
        let split = split_combined_image_samplers(&words(&spirv)).expect("passes run");
        naga_accepts(&split).expect("naga reads the module");
    }
}
