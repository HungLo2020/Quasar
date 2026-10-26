//! Shader modules: GLSL or SPIR-V in, naga IR out, plus the binding
//! reflection pipelines derive their bind group layouts from.
//!
//! GLSL is compiled by the shared Shaderc toolchain (and its disk cache),
//! SPIRV-Tools splits combined image samplers, and naga parses the result.
//! The split leaves `<name>_image` and `<name>_sampler` on the GAL binding
//! `b`; wgpu bindings are renumbered so every GAL binding `b` becomes wgpu
//! binding `2b`, and the sampler half of a combined sampler `2b + 1`.
//!
//! naga's SPIR-V reader can negate `Position.y` while parsing. A vertex
//! module therefore has one variant per raster Y direction: `Up` keeps clip
//! space as written (wgpu's native convention, like the Vulkan backend's
//! flipped viewport) and `Down` lets naga negate Y.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use super::device::WgpuContext;
use crate::backends::spirv::{self, tools};
use crate::error::{GalError, GalResult};
use crate::resources::{RasterYDirection, ShaderCodeFormat, ShaderModuleDesc, ShaderStage};

/// The wgpu binding of GAL binding `binding`; `sampler_half` selects the
/// sampler of a combined texture sampler.
pub(super) const fn native_binding(binding: u32, sampler_half: bool) -> u32 {
    binding * 2 + sampler_half as u32
}

/// The GAL binding and half a wgpu binding stands for.
pub(super) const fn gal_binding(native: u32) -> (u32, bool) {
    (native / 2, native % 2 == 1)
}

/// One resource an entry point uses, in wgpu numbering.
#[derive(Clone, Debug)]
pub(super) struct ReflectedBinding {
    pub(super) group: u32,
    pub(super) binding: u32,
    pub(super) ty: ReflectedType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ReflectedType {
    UniformBuffer,
    StorageBuffer {
        read_only: bool,
    },
    /// A sampled texture; float textures may still bind unfilterable formats.
    Texture {
        sample: SampleKind,
        dimension: wgpu::TextureViewDimension,
        multisampled: bool,
    },
    StorageTexture {
        access: wgpu::StorageTextureAccess,
        format: wgpu::TextureFormat,
        dimension: wgpu::TextureViewDimension,
    },
    Sampler {
        comparison: bool,
        /// Samples a texture that cannot be filtered (integer or depth
        /// data read as a value).
        pairs_with_unfilterable: bool,
        /// Float textures it samples, `(group, binding)`; any of them bound
        /// to an unfilterable format makes this sampler non-filtering too.
        float_textures: Vec<(u32, u32)>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SampleKind {
    Float,
    Depth,
    Uint,
    Sint,
}

/// A parsed, validated and renumbered module for one Y direction.
pub(super) struct ShaderVariant {
    pub(super) module: wgpu::ShaderModule,
    pub(super) bindings: Vec<ReflectedBinding>,
    /// The entry point writes flat-interpolated outputs, so the provoking
    /// vertex is observable.
    pub(super) has_flat_outputs: bool,
}

pub(super) struct ShaderObject {
    pub(super) label: String,
    pub(super) stage: ShaderStage,
    pub(super) entry_point: String,
    /// SPIR-V after the naga compatibility passes.
    spirv: Vec<u8>,
    variants: HashMap<bool, Arc<ShaderVariant>>,
}

impl ShaderObject {
    pub(super) fn new(context: &WgpuContext, desc: &ShaderModuleDesc) -> GalResult<Self> {
        let spirv = match desc.code_format {
            ShaderCodeFormat::Glsl => {
                let source = std::str::from_utf8(&desc.code).map_err(|error| {
                    GalError::invalid_argument(format!(
                        "GLSL shader '{}' is not UTF-8: {error}",
                        desc.label
                    ))
                })?;
                spirv::compile_glsl_cached(desc.stage, source, &desc.label, &desc.entry_point)
                    .map_err(|error| {
                        GalError::backend(format!(
                            "failed to compile GLSL shader '{}' for the wgpu backend: {error}",
                            desc.label
                        ))
                    })?
            }
            ShaderCodeFormat::Spirv => desc.code.clone(),
            ShaderCodeFormat::BackendPortableIr => {
                return Err(GalError::unsupported_feature(
                    "the wgpu backend accepts GLSL or SPIR-V shader code",
                ))
            }
        };
        if spirv.len() % 4 != 0 || spirv.len() < 20 {
            return Err(GalError::backend(format!(
                "shader '{}' is not 4-byte aligned SPIR-V",
                desc.label
            )));
        }
        let words: Vec<u32> = spirv
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        let split = tools::split_combined_image_samplers(&words).map_err(|error| {
            GalError::backend(format!(
                "failed to prepare shader '{}' for naga: {error}",
                desc.label
            ))
        })?;
        let mut object = Self {
            label: desc.label.clone(),
            stage: desc.stage,
            entry_point: desc.entry_point.clone(),
            spirv: split.iter().flat_map(|word| word.to_le_bytes()).collect(),
            variants: HashMap::new(),
        };
        // Parse once now so malformed shaders fail at creation, as they do
        // on the other backends.
        object.variant(context, RasterYDirection::Up)?;
        Ok(object)
    }

    /// The module for pipelines rasterizing in `direction`; only vertex
    /// shaders differ by direction.
    pub(super) fn variant(
        &mut self,
        context: &WgpuContext,
        direction: RasterYDirection,
    ) -> GalResult<Arc<ShaderVariant>> {
        let flip = self.stage == ShaderStage::Vertex && direction == RasterYDirection::Down;
        if let Some(variant) = self.variants.get(&flip) {
            return Ok(variant.clone());
        }
        let variant = Arc::new(self.build_variant(context, flip)?);
        self.variants.insert(flip, variant.clone());
        Ok(variant)
    }

    fn build_variant(&self, context: &WgpuContext, flip_y: bool) -> GalResult<ShaderVariant> {
        let options = naga::front::spv::Options {
            adjust_coordinate_space: flip_y,
            strict_capabilities: false,
            block_ctx_dump_prefix: None,
        };
        let mut module =
            naga::front::spv::parse_u8_slice(&self.spirv, &options).map_err(|error| {
                GalError::backend(format!("naga cannot read shader '{}': {error}", self.label))
            })?;
        renumber_bindings(&mut module);
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|error| {
            GalError::backend(format!(
                "shader '{}' is not valid for wgpu: {}",
                self.label,
                error.emit_to_string("")
            ))
        })?;
        let naga_stage = match self.stage {
            ShaderStage::Vertex => naga::ShaderStage::Vertex,
            ShaderStage::Fragment => naga::ShaderStage::Fragment,
            ShaderStage::Compute => naga::ShaderStage::Compute,
            other => {
                return Err(GalError::unsupported_feature(format!(
                    "the wgpu backend does not support {other:?} shaders"
                )))
            }
        };
        let entry_index = module
            .entry_points
            .iter()
            .position(|entry| entry.stage == naga_stage && entry.name == self.entry_point)
            .ok_or_else(|| {
                GalError::backend(format!(
                    "shader '{}' has no {:?} entry point '{}'",
                    self.label, self.stage, self.entry_point
                ))
            })?;
        let bindings = reflect(&module, info.get_entry_point(entry_index), &self.label)?;
        let has_flat_outputs = naga_stage == naga::ShaderStage::Vertex
            && has_flat_outputs(&module, &module.entry_points[entry_index].function);
        let module = context.scoped("shader module creation", || {
            context
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(&self.label),
                    source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
                })
        })?;
        Ok(ShaderVariant {
            module,
            bindings,
            has_flat_outputs,
        })
    }
}

fn has_flat_outputs(module: &naga::Module, function: &naga::Function) -> bool {
    let flat = |binding: &Option<naga::Binding>| {
        matches!(
            binding,
            Some(naga::Binding::Location {
                interpolation: Some(naga::Interpolation::Flat),
                ..
            })
        )
    };
    let Some(result) = &function.result else {
        return false;
    };
    flat(&result.binding)
        || match &module.types[result.ty].inner {
            naga::TypeInner::Struct { members, .. } => {
                members.iter().any(|member| flat(&member.binding))
            }
            _ => false,
        }
}

/// Applies the `2b` / `2b + 1` numbering described in the module docs.
fn renumber_bindings(module: &mut naga::Module) {
    let mut by_slot: BTreeMap<(u32, u32), Vec<naga::Handle<naga::GlobalVariable>>> =
        BTreeMap::new();
    for (handle, global) in module.global_variables.iter() {
        if let Some(binding) = &global.binding {
            by_slot
                .entry((binding.group, binding.binding))
                .or_default()
                .push(handle);
        }
    }
    for ((_, binding), globals) in by_slot {
        let shared = globals.len() > 1;
        for handle in globals {
            let is_sampler = matches!(
                module.types[module.global_variables[handle].ty].inner,
                naga::TypeInner::Sampler { .. }
            );
            if let Some(slot) = module.global_variables[handle].binding.as_mut() {
                slot.binding = native_binding(binding, shared && is_sampler);
            }
        }
    }
}

fn texture_dimension(dim: naga::ImageDimension, arrayed: bool) -> wgpu::TextureViewDimension {
    match (dim, arrayed) {
        (naga::ImageDimension::D1, _) => wgpu::TextureViewDimension::D1,
        (naga::ImageDimension::D2, false) => wgpu::TextureViewDimension::D2,
        (naga::ImageDimension::D2, true) => wgpu::TextureViewDimension::D2Array,
        (naga::ImageDimension::D3, _) => wgpu::TextureViewDimension::D3,
        (naga::ImageDimension::Cube, false) => wgpu::TextureViewDimension::Cube,
        (naga::ImageDimension::Cube, true) => wgpu::TextureViewDimension::CubeArray,
    }
}

fn storage_access(access: naga::StorageAccess) -> wgpu::StorageTextureAccess {
    let load = access.contains(naga::StorageAccess::LOAD);
    let store = access.contains(naga::StorageAccess::STORE);
    match (load, store) {
        (true, true) => wgpu::StorageTextureAccess::ReadWrite,
        (true, false) => wgpu::StorageTextureAccess::ReadOnly,
        _ => wgpu::StorageTextureAccess::WriteOnly,
    }
}

fn storage_format(format: naga::StorageFormat) -> Option<wgpu::TextureFormat> {
    use naga::StorageFormat as S;
    use wgpu::TextureFormat as T;
    Some(match format {
        S::R8Unorm => T::R8Unorm,
        S::R8Snorm => T::R8Snorm,
        S::R8Uint => T::R8Uint,
        S::R8Sint => T::R8Sint,
        S::R16Uint => T::R16Uint,
        S::R16Sint => T::R16Sint,
        S::R16Float => T::R16Float,
        S::Rg8Unorm => T::Rg8Unorm,
        S::Rg8Snorm => T::Rg8Snorm,
        S::Rg8Uint => T::Rg8Uint,
        S::Rg8Sint => T::Rg8Sint,
        S::R32Uint => T::R32Uint,
        S::R32Sint => T::R32Sint,
        S::R32Float => T::R32Float,
        S::Rg16Uint => T::Rg16Uint,
        S::Rg16Sint => T::Rg16Sint,
        S::Rg16Float => T::Rg16Float,
        S::Rgba8Unorm => T::Rgba8Unorm,
        S::Rgba8Snorm => T::Rgba8Snorm,
        S::Rgba8Uint => T::Rgba8Uint,
        S::Rgba8Sint => T::Rgba8Sint,
        S::Bgra8Unorm => T::Bgra8Unorm,
        S::Rgb10a2Uint => T::Rgb10a2Uint,
        S::Rgb10a2Unorm => T::Rgb10a2Unorm,
        S::Rg11b10Ufloat => T::Rg11b10Ufloat,
        S::Rg32Uint => T::Rg32Uint,
        S::Rg32Sint => T::Rg32Sint,
        S::Rg32Float => T::Rg32Float,
        S::Rgba16Uint => T::Rgba16Uint,
        S::Rgba16Sint => T::Rgba16Sint,
        S::Rgba16Float => T::Rgba16Float,
        S::Rgba32Uint => T::Rgba32Uint,
        S::Rgba32Sint => T::Rgba32Sint,
        S::Rgba32Float => T::Rgba32Float,
        S::R16Unorm => T::R16Unorm,
        S::R16Snorm => T::R16Snorm,
        S::Rg16Unorm => T::Rg16Unorm,
        S::Rg16Snorm => T::Rg16Snorm,
        S::Rgba16Unorm => T::Rgba16Unorm,
        S::Rgba16Snorm => T::Rgba16Snorm,
        _ => return None,
    })
}

/// The resources one entry point uses.
fn reflect(
    module: &naga::Module,
    info: &naga::valid::FunctionInfo,
    label: &str,
) -> GalResult<Vec<ReflectedBinding>> {
    // Samplers that sample integer data, or depth data as values, cannot
    // be filtering samplers; float textures they sample decide at bind time.
    let mut unfilterable_samplers = HashSet::new();
    let mut sampler_float_textures: HashMap<_, Vec<(u32, u32)>> = HashMap::new();
    for key in &info.sampling_set {
        let image = &module.global_variables[key.image];
        let sampler = &module.global_variables[key.sampler];
        let comparison = matches!(
            module.types[sampler.ty].inner,
            naga::TypeInner::Sampler { comparison: true }
        );
        match module.types[image.ty].inner {
            naga::TypeInner::Image {
                class: naga::ImageClass::Sampled { kind, .. },
                ..
            } => {
                if kind != naga::ScalarKind::Float {
                    unfilterable_samplers.insert(key.sampler);
                } else if let Some(binding) = &image.binding {
                    sampler_float_textures
                        .entry(key.sampler)
                        .or_default()
                        .push((binding.group, binding.binding));
                }
            }
            naga::TypeInner::Image {
                class: naga::ImageClass::Depth { .. },
                ..
            } if !comparison => {
                unfilterable_samplers.insert(key.sampler);
            }
            _ => {}
        }
    }
    let mut bindings = Vec::new();
    for (handle, global) in module.global_variables.iter() {
        let Some(slot) = &global.binding else {
            continue;
        };
        if info[handle].is_empty() {
            continue;
        }
        let unsupported = |what: &str| {
            GalError::unsupported_feature(format!(
                "shader '{label}' binds {what} at set {} binding {}, which the wgpu backend does not support",
                slot.group,
                gal_binding(slot.binding).0
            ))
        };
        let ty = match global.space {
            naga::AddressSpace::Uniform => ReflectedType::UniformBuffer,
            naga::AddressSpace::Storage { access } => ReflectedType::StorageBuffer {
                read_only: !access.contains(naga::StorageAccess::STORE),
            },
            naga::AddressSpace::Handle => match module.types[global.ty].inner {
                naga::TypeInner::Image {
                    dim,
                    arrayed,
                    class,
                } => {
                    let dimension = texture_dimension(dim, arrayed);
                    match class {
                        naga::ImageClass::Sampled { kind, multi } => ReflectedType::Texture {
                            sample: match kind {
                                naga::ScalarKind::Float => SampleKind::Float,
                                naga::ScalarKind::Uint => SampleKind::Uint,
                                naga::ScalarKind::Sint => SampleKind::Sint,
                                _ => return Err(unsupported("a non-numeric texture")),
                            },
                            dimension,
                            multisampled: multi,
                        },
                        naga::ImageClass::Depth { multi } => ReflectedType::Texture {
                            sample: SampleKind::Depth,
                            dimension,
                            multisampled: multi,
                        },
                        naga::ImageClass::Storage { format, access } => {
                            ReflectedType::StorageTexture {
                                access: storage_access(access),
                                format: storage_format(format)
                                    .ok_or_else(|| unsupported("a storage image format"))?,
                                dimension,
                            }
                        }
                        _ => return Err(unsupported("an external texture")),
                    }
                }
                naga::TypeInner::Sampler { comparison } => ReflectedType::Sampler {
                    comparison,
                    pairs_with_unfilterable: unfilterable_samplers.contains(&handle),
                    float_textures: sampler_float_textures
                        .get(&handle)
                        .cloned()
                        .unwrap_or_default(),
                },
                _ => return Err(unsupported("a resource array")),
            },
            _ => continue,
        };
        bindings.push(ReflectedBinding {
            group: slot.group,
            binding: slot.binding,
            ty,
        });
    }
    Ok(bindings)
}
