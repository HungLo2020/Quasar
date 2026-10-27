//! GLSL to SPIR-V through Shaderc, shared by the SPIR-V backends.

use std::cell::RefCell;

/// Keeps shader compiler failures actionable without retaining or writing a
/// full generated pack source. Shaderc uses one-based source lines in its
/// diagnostics; include a small numbered window when that line is present.
fn shader_error_context(source: &str, diagnostic: &str) -> Option<String> {
    let line = diagnostic
        .split(':')
        .find_map(|part| part.trim().parse::<usize>().ok())?;
    let first = line.saturating_sub(3).max(1);
    let last = line.saturating_add(2);
    let context = source
        .lines()
        .enumerate()
        .filter_map(|(index, text)| {
            let number = index + 1;
            (first..=last)
                .contains(&number)
                .then(|| format!("{number}: {text}"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!context.is_empty()).then_some(context)
}

/// The SPIR-V environment a module is compiled for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::backends) enum SpirvTarget {
    /// SPIR-V 1.6 for the native Vulkan 1.3 context.
    #[cfg_attr(not(any(feature = "vulkan", test)), allow(dead_code))]
    Vulkan1_3,
    /// SPIR-V 1.3, which naga reads: glslang emits member-wise struct copies
    /// instead of SPIR-V 1.4's `OpCopyLogical`.
    #[cfg_attr(not(feature = "wgpu"), allow(dead_code))]
    Vulkan1_1,
}

thread_local! {
    /// Creating a Shaderc compiler sets up glslang's process state; each
    /// thread keeps one instead of creating one per module.
    static COMPILER: RefCell<Option<shaderc::Compiler>> = const { RefCell::new(None) };
}

fn stage_name(stage: shaderc::ShaderKind) -> &'static str {
    match stage {
        shaderc::ShaderKind::Vertex => "vertex",
        shaderc::ShaderKind::Fragment => "fragment",
        shaderc::ShaderKind::Compute => "compute",
        shaderc::ShaderKind::Geometry => "geometry",
        shaderc::ShaderKind::TessControl => "tess-control",
        shaderc::ShaderKind::TessEvaluation => "tess-evaluation",
        _ => "other",
    }
}

#[cfg(feature = "vulkan")]
pub(in crate::backends) fn compile_glsl_for_backend(
    stage: shaderc::ShaderKind,
    source: &str,
    source_name: &str,
    entry: &str,
) -> Result<Vec<u8>, String> {
    compile_glsl_for_target(SpirvTarget::Vulkan1_3, stage, source, source_name, entry)
}

pub(in crate::backends) fn compile_glsl_for_target(
    target: SpirvTarget,
    stage: shaderc::ShaderKind,
    source: &str,
    source_name: &str,
    entry_point: &str,
) -> Result<Vec<u8>, String> {
    if source.trim().is_empty() {
        return Err(format!("cannot compile blank shader source: {source_name}"));
    }
    if entry_point.trim().is_empty() {
        return Err(format!("shader entry point is blank for {source_name}"));
    }
    let stage_label = stage_name(stage);
    if let Ok(dir) = crate::env::var("VULKANIC_SHADER_DUMP_DIR") {
        let safe_name = source_name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>();
        let path = std::path::Path::new(&dir).join(format!("{stage_label}_{safe_name}.glsl"));
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(path, source);
        }
    }

    let mut options = shaderc::CompileOptions::new()
        .map_err(|error| format!("failed to create Shaderc compile options: {error}"))?;
    options.set_source_language(shaderc::SourceLanguage::GLSL);
    options.set_target_env(
        shaderc::TargetEnv::Vulkan,
        // The native Vulkan context is created at 1.3. Emit modules against
        // that same contract so shader features are not silently lowered
        // through a stale Vulkan 1.0 capability model.
        match target {
            SpirvTarget::Vulkan1_3 => shaderc::EnvVersion::Vulkan1_3 as u32,
            SpirvTarget::Vulkan1_1 => shaderc::EnvVersion::Vulkan1_1 as u32,
        },
    );
    options.set_auto_bind_uniforms(true);
    options.set_auto_map_locations(true);
    // Debug builds and the conformance tests keep unoptimized modules, which
    // keep inactive std140 members so their declared offsets can be
    // inspected; release modules get Shaderc's semantics-preserving cleanup
    // and dead-code elimination.
    if !cfg!(debug_assertions) && !cfg!(test) {
        options.set_optimization_level(shaderc::OptimizationLevel::Performance);
    }

    let artifact = COMPILER.with(|compiler| {
        let mut compiler = compiler.borrow_mut();
        if compiler.is_none() {
            *compiler = Some(
                shaderc::Compiler::new()
                    .map_err(|error| format!("failed to create Shaderc compiler: {error}"))?,
            );
        }
        compiler
            .as_ref()
            .expect("created above")
            .compile_into_spirv(source, stage, source_name, entry_point, Some(&options))
            .map_err(|error| {
                let diagnostic = error.to_string();
                let context = shader_error_context(source, &diagnostic)
                    .map(|context| format!("\nsource context:\n{context}"))
                    .unwrap_or_default();
                format!(
                    "SPIR-V compilation failed for '{source_name}' (stage={stage_label}, entryPoint={entry_point}) using Shaderc: {diagnostic}{context}"
                )
            })
    })?;

    let spirv = artifact.as_binary_u8().to_vec();
    if spirv.is_empty() {
        return Err(format!(
            "Shaderc produced empty SPIR-V for '{source_name}' (stage={stage_label}, entryPoint={entry_point})"
        ));
    }
    Ok(spirv)
}

#[cfg(all(test, feature = "vulkan"))]
pub(crate) fn compile_glsl_for_backend_test(
    stage: shaderc::ShaderKind,
    source: &str,
    source_name: &str,
) -> Result<Vec<u8>, String> {
    compile_glsl_for_backend(stage, source, source_name, "main")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_error_context_extracts_a_bounded_one_based_window() {
        let source = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
        let context = shader_error_context(source, "source:5: error: malformed")
            .expect("a Shaderc-style source line must produce context");
        assert_eq!(
            "2: two\n3: three\n4: four\n5: five\n6: six\n7: seven",
            context
        );
    }

    #[test]
    fn shader_error_context_ignores_diagnostics_without_a_source_line() {
        assert!(shader_error_context("one", "error: malformed").is_none());
    }

    #[test]
    fn shaderc_compiles_minimal_vertex_shader() {
        let source = "#version 450\nvoid main(){gl_Position=vec4(0.0);}";
        let name = "test:minimal_vertex";
        let entry = "main";

        let spirv = compile_glsl_for_target(
            SpirvTarget::Vulkan1_3,
            shaderc::ShaderKind::Vertex,
            source,
            name,
            entry,
        )
        .expect("Shaderc should compile a minimal vertex shader");
        // The thread's compiler is reused for the next module.
        compile_glsl_for_target(
            SpirvTarget::Vulkan1_3,
            shaderc::ShaderKind::Vertex,
            source,
            name,
            entry,
        )
        .expect("a reused compiler compiles again");

        assert!(spirv.len() >= 4);
        assert_eq!([0x03, 0x02, 0x23, 0x07], spirv[0..4]);
    }
}
