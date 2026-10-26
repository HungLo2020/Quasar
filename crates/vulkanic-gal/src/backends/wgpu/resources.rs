//! wgpu objects behind GAL handles.
//!
//! The GAL's resource layouts name only binding kinds, while wgpu bind group
//! layouts also need texture sample types, view dimensions, storage formats
//! and sampler filtering. Pipelines therefore derive their bind group
//! layouts from shader reflection, and bind groups are created lazily per
//! (resource set, bind group layout) pair. A float texture binding bound to
//! an unfilterable format (depth sampled as a value, or `R32Float` without
//! `FLOAT32_FILTERABLE`) needs an unfilterable layout and a non-filtering
//! sampler; such pipelines get a layout variant keyed by those bindings.

use std::collections::{BTreeMap, HashMap};
use std::num::NonZeroU64;
use std::sync::Arc;

use super::convert;
use super::device::WgpuContext;
use super::shaders::{gal_binding, ReflectedBinding, ReflectedType, SampleKind, ShaderObject};
use crate::backends::{BackendCreateDesc, BackendToken};
use crate::error::{GalError, GalResult};
use crate::frame::FrameRenderTargetId;
use crate::handles::{Handle, HandleKind};
use crate::resources::*;

pub(super) struct BufferObject {
    pub(super) buffer: wgpu::Buffer,
    /// The GAL size; the native size is rounded up to a multiple of four.
    pub(super) size: u64,
}

pub(super) struct TextureObject {
    pub(super) texture: wgpu::Texture,
    pub(super) desc: TextureDesc,
    pub(super) format: wgpu::TextureFormat,
}

pub(super) struct TextureViewObject {
    texture: wgpu::Texture,
    pub(super) desc: TextureViewDesc,
    pub(super) format: wgpu::TextureFormat,
    texture_dimension: TextureDimension,
    views: HashMap<(wgpu::TextureViewDimension, wgpu::TextureAspect), wgpu::TextureView>,
}

impl TextureViewObject {
    fn view(
        &mut self,
        dimension: wgpu::TextureViewDimension,
        aspect: wgpu::TextureAspect,
    ) -> wgpu::TextureView {
        let desc = &self.desc;
        let texture = &self.texture;
        let three_d = self.texture_dimension == TextureDimension::D3;
        self.views
            .entry((dimension, aspect))
            .or_insert_with(|| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&desc.label),
                    format: None,
                    dimension: Some(dimension),
                    usage: None,
                    aspect,
                    base_mip_level: desc.base_mip,
                    mip_level_count: Some(desc.mip_count),
                    base_array_layer: if three_d { 0 } else { desc.base_layer },
                    array_layer_count: Some(if three_d { 1 } else { desc.layer_count }),
                })
            })
            .clone()
    }

    /// The view a shader samples or stores through.
    pub(super) fn binding_view(
        &mut self,
        dimension: wgpu::TextureViewDimension,
    ) -> wgpu::TextureView {
        let aspect = if self.format == wgpu::TextureFormat::Depth24PlusStencil8 {
            wgpu::TextureAspect::DepthOnly
        } else {
            wgpu::TextureAspect::All
        };
        self.view(dimension, aspect)
    }

    /// The view a render pass attaches: one mip, one layer.
    pub(super) fn attachment_view(&mut self) -> wgpu::TextureView {
        let dimension = if self.texture_dimension == TextureDimension::D3 {
            wgpu::TextureViewDimension::D3
        } else {
            wgpu::TextureViewDimension::D2
        };
        self.view(dimension, wgpu::TextureAspect::All)
    }
}

pub(super) struct SamplerObject {
    pub(super) sampler: wgpu::Sampler,
}

pub(super) struct ResourceSetObject {
    desc: ResourceSetDesc,
    layout: ResourceLayoutDesc,
    /// Default dynamic offsets by GAL binding.
    default_offsets: BTreeMap<u32, u64>,
    /// GAL bindings whose texture is stored in an unfilterable format.
    unfilterable: Vec<u32>,
    bind_groups: HashMap<u64, wgpu::BindGroup>,
}

impl ResourceSetObject {
    /// Whether the texture at wgpu `binding` is stored in an unfilterable
    /// format.
    pub(super) fn is_unfilterable(&self, binding: u32) -> bool {
        self.unfilterable.contains(&gal_binding(binding).0)
    }

    /// Buffers the set lets shaders write.
    pub(super) fn written_buffers(&self) -> impl Iterator<Item = Handle> + '_ {
        self.desc
            .bindings
            .iter()
            .filter(|binding| {
                binding.kind == ResourceBindingKind::StorageBuffer && binding.access.writes()
            })
            .map(|binding| binding.resource)
    }

    /// The dynamic offsets `layout` takes, from explicit offsets in GAL
    /// binding order or else the set's defaults.
    pub(super) fn dynamic_offsets(
        &self,
        layout: &BindGroupLayoutInfo,
        explicit: &[u64],
    ) -> GalResult<Vec<u32>> {
        let mut dynamic: Vec<u32> = self
            .layout
            .bindings
            .iter()
            .filter(|binding| binding.dynamic_offset_count > 0)
            .map(|binding| binding.binding)
            .collect();
        dynamic.sort_unstable();
        let by_binding: BTreeMap<u32, u64> = if explicit.is_empty() {
            self.default_offsets.clone()
        } else {
            dynamic
                .iter()
                .copied()
                .zip(explicit.iter().copied())
                .collect()
        };
        layout
            .dynamic_bindings
            .iter()
            .map(|binding| {
                let offset = by_binding.get(binding).copied().unwrap_or(0);
                convert::checked_u32(offset, "dynamic buffer offset")
            })
            .collect()
    }
}

pub(super) struct BindGroupLayoutInfo {
    pub(super) id: u64,
    pub(super) layout: wgpu::BindGroupLayout,
    entries: Vec<wgpu::BindGroupLayoutEntry>,
    /// GAL bindings of the dynamic entries, in wgpu offset order.
    dynamic_bindings: Vec<u32>,
}

/// One concrete pipeline for a set of unfilterable float bindings.
pub(super) struct RenderVariant {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) groups: Vec<Arc<BindGroupLayoutInfo>>,
}

pub(super) struct ComputeVariant {
    pub(super) pipeline: wgpu::ComputePipeline,
    pub(super) groups: Vec<Arc<BindGroupLayoutInfo>>,
}

/// Bindings both kinds of pipeline derive layouts from.
struct PipelineBindings {
    /// Per group: wgpu binding to (type, stages).
    groups: Vec<BTreeMap<u32, (ReflectedType, wgpu::ShaderStages)>>,
    /// Float texture bindings `(group, binding)`; bit `i` of a variant key
    /// marks `float_textures[i]` as bound to an unfilterable format.
    float_textures: Vec<(u32, u32)>,
    resource_layouts: Vec<ResourceLayoutDesc>,
}

impl PipelineBindings {
    fn new(
        label: &str,
        resource_layouts: Vec<ResourceLayoutDesc>,
        stages: &[(&[ReflectedBinding], wgpu::ShaderStages)],
    ) -> GalResult<Self> {
        let mut groups: Vec<BTreeMap<u32, (ReflectedType, wgpu::ShaderStages)>> = Vec::new();
        for (bindings, stage) in stages {
            for binding in *bindings {
                let group = binding.group as usize;
                let Some(layout) = resource_layouts.get(group) else {
                    return Err(GalError::invalid_argument(format!(
                        "pipeline '{label}' shaders use set {group}, but its layout has {} sets",
                        resource_layouts.len()
                    )));
                };
                let (gal, _) = gal_binding(binding.binding);
                if !layout
                    .bindings
                    .iter()
                    .any(|candidate| candidate.binding == gal)
                {
                    return Err(GalError::invalid_argument(format!(
                        "pipeline '{label}' shaders use set {group} binding {gal}, which its resource layout '{}' does not declare",
                        layout.label
                    )));
                }
                if groups.len() <= group {
                    groups.resize_with(group + 1, BTreeMap::new);
                }
                match groups[group].get_mut(&binding.binding) {
                    Some((existing, stages)) => {
                        *stages |= *stage;
                        merge_reflected(existing, &binding.ty).map_err(|what| {
                            GalError::invalid_argument(format!(
                                "pipeline '{label}' stages disagree on set {group} binding {gal}: {what}"
                            ))
                        })?;
                    }
                    None => {
                        groups[group].insert(binding.binding, (binding.ty.clone(), *stage));
                    }
                }
            }
        }
        let mut float_textures = Vec::new();
        for (group, bindings) in groups.iter().enumerate() {
            for (binding, (ty, _)) in bindings {
                if matches!(
                    ty,
                    ReflectedType::Texture {
                        sample: SampleKind::Float,
                        ..
                    }
                ) {
                    float_textures.push((group as u32, *binding));
                }
            }
        }
        Ok(Self {
            groups,
            float_textures,
            resource_layouts,
        })
    }

    fn unfilterable(&self, key: u64, group: u32, binding: u32) -> bool {
        self.float_textures
            .iter()
            .position(|texture| *texture == (group, binding))
            .is_some_and(|index| index < 64 && key & (1 << index) != 0)
    }

    /// Layout entries of every group for variant `key`.
    fn entries(&self, key: u64) -> Vec<Vec<wgpu::BindGroupLayoutEntry>> {
        self.groups
            .iter()
            .enumerate()
            .map(|(group, bindings)| {
                let group = group as u32;
                let layout = &self.resource_layouts[group as usize];
                bindings
                    .iter()
                    .map(|(binding, (ty, visibility))| {
                        let (gal, _) = gal_binding(*binding);
                        let dynamic = layout.bindings.iter().any(|candidate| {
                            candidate.binding == gal && candidate.dynamic_offset_count > 0
                        });
                        let ty = match ty {
                            ReflectedType::UniformBuffer => wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: dynamic,
                                min_binding_size: None,
                            },
                            ReflectedType::StorageBuffer { read_only } => {
                                wgpu::BindingType::Buffer {
                                    ty: wgpu::BufferBindingType::Storage {
                                        read_only: *read_only,
                                    },
                                    has_dynamic_offset: dynamic,
                                    min_binding_size: None,
                                }
                            }
                            ReflectedType::Texture {
                                sample,
                                dimension,
                                multisampled,
                            } => wgpu::BindingType::Texture {
                                sample_type: match sample {
                                    SampleKind::Float => wgpu::TextureSampleType::Float {
                                        filterable: !self.unfilterable(key, group, *binding),
                                    },
                                    SampleKind::Depth => wgpu::TextureSampleType::Depth,
                                    SampleKind::Uint => wgpu::TextureSampleType::Uint,
                                    SampleKind::Sint => wgpu::TextureSampleType::Sint,
                                },
                                view_dimension: *dimension,
                                multisampled: *multisampled,
                            },
                            ReflectedType::StorageTexture {
                                access,
                                format,
                                dimension,
                            } => wgpu::BindingType::StorageTexture {
                                access: *access,
                                format: *format,
                                view_dimension: *dimension,
                            },
                            ReflectedType::Sampler {
                                comparison,
                                pairs_with_unfilterable,
                                float_textures,
                            } => wgpu::BindingType::Sampler(if *comparison {
                                wgpu::SamplerBindingType::Comparison
                            } else if *pairs_with_unfilterable
                                || float_textures.iter().any(|(group, binding)| {
                                    self.unfilterable(key, *group, *binding)
                                })
                            {
                                wgpu::SamplerBindingType::NonFiltering
                            } else {
                                wgpu::SamplerBindingType::Filtering
                            }),
                        };
                        wgpu::BindGroupLayoutEntry {
                            binding: *binding,
                            visibility: *visibility,
                            ty,
                            count: None,
                        }
                    })
                    .collect()
            })
            .collect()
    }
}

fn merge_reflected(
    existing: &mut ReflectedType,
    other: &ReflectedType,
) -> Result<(), &'static str> {
    match (existing, other) {
        (ReflectedType::UniformBuffer, ReflectedType::UniformBuffer) => Ok(()),
        (
            ReflectedType::StorageBuffer { read_only },
            ReflectedType::StorageBuffer {
                read_only: other_read_only,
            },
        ) => {
            *read_only &= *other_read_only;
            Ok(())
        }
        (
            ReflectedType::Sampler {
                comparison,
                pairs_with_unfilterable,
                float_textures,
            },
            ReflectedType::Sampler {
                comparison: other_comparison,
                pairs_with_unfilterable: other_unfilterable,
                float_textures: other_textures,
            },
        ) => {
            if comparison != other_comparison {
                return Err("comparison and plain sampler");
            }
            *pairs_with_unfilterable |= *other_unfilterable;
            for texture in other_textures {
                if !float_textures.contains(texture) {
                    float_textures.push(*texture);
                }
            }
            Ok(())
        }
        (existing, other) if *existing == *other => Ok(()),
        _ => Err("different resource types"),
    }
}

pub(super) struct GraphicsPipelineObject {
    desc: GraphicsPipelineDesc,
    bindings: PipelineBindings,
    vertex: wgpu::ShaderModule,
    fragment: wgpu::ShaderModule,
    vertex_entry: String,
    fragment_entry: String,
    variants: HashMap<u64, Arc<RenderVariant>>,
    pub(super) topology: PrimitiveTopology,
    /// wgpu flat-shades from the first vertex. A `Last` pipeline whose
    /// vertex shader has flat outputs rotates each primitive's vertices so
    /// the last comes first (rotation keeps winding).
    pub(super) provoking_last: bool,
    pub(super) stencil_reference: Option<u32>,
}

impl GraphicsPipelineObject {
    pub(super) fn float_textures(&self) -> &[(u32, u32)] {
        &self.bindings.float_textures
    }
}

pub(super) struct ComputePipelineObject {
    label: String,
    bindings: PipelineBindings,
    shader: wgpu::ShaderModule,
    entry: String,
    variants: HashMap<u64, Arc<ComputeVariant>>,
}

impl ComputePipelineObject {
    pub(super) fn float_textures(&self) -> &[(u32, u32)] {
        &self.bindings.float_textures
    }
}

pub(super) struct FrameTargetObject {
    pub(super) render_target: FrameRenderTargetId,
}

/// Placeholders for optional bindings a set leaves empty.
#[derive(Default)]
struct Dummies {
    buffer: Option<wgpu::Buffer>,
    textures: HashMap<(wgpu::TextureViewDimension, u8), wgpu::TextureView>,
    storage: HashMap<(wgpu::TextureViewDimension, wgpu::TextureFormat), wgpu::TextureView>,
    samplers: HashMap<u8, wgpu::Sampler>,
    empty_groups: HashMap<u64, wgpu::BindGroup>,
}

enum Bound {
    Buffer(wgpu::Buffer, u64, Option<NonZeroU64>),
    View(wgpu::TextureView),
    Sampler(wgpu::Sampler),
}

#[derive(Default)]
pub(super) struct WgpuObjects {
    next_token: u64,
    pub(super) buffers: HashMap<Handle, BufferObject>,
    pub(super) textures: HashMap<Handle, TextureObject>,
    pub(super) views: HashMap<Handle, TextureViewObject>,
    pub(super) samplers: HashMap<Handle, SamplerObject>,
    combined: HashMap<Handle, (Handle, Handle)>,
    shaders: HashMap<Handle, ShaderObject>,
    resource_layouts: HashMap<Handle, ResourceLayoutDesc>,
    pub(super) sets: HashMap<Handle, ResourceSetObject>,
    pipeline_layouts: HashMap<Handle, Vec<Handle>>,
    pub(super) graphics_pipelines: HashMap<Handle, GraphicsPipelineObject>,
    pub(super) compute_pipelines: HashMap<Handle, ComputePipelineObject>,
    pub(super) render_targets: HashMap<Handle, RenderTargetDesc>,
    pub(super) frame_targets: HashMap<Handle, FrameTargetObject>,
    render_passes: HashMap<Handle, RenderPassDesc>,
    layouts: HashMap<Vec<wgpu::BindGroupLayoutEntry>, Arc<BindGroupLayoutInfo>>,
    next_layout_id: u64,
    dummies: Dummies,
}

fn missing(kind: &str, handle: Handle) -> GalError {
    GalError::backend(format!("wgpu backend has no {kind} for handle {handle:?}"))
}

impl WgpuObjects {
    pub(super) fn create(
        &mut self,
        context: &WgpuContext,
        handle: Handle,
        desc: BackendCreateDesc<'_>,
    ) -> GalResult<BackendToken> {
        match desc {
            BackendCreateDesc::Buffer(desc) => {
                let size = desc.size.max(1).next_multiple_of(4);
                let buffer = context.scoped("buffer creation", || {
                    context.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(&desc.label),
                        size,
                        usage: convert::buffer_usages(&desc.usages),
                        mapped_at_creation: false,
                    })
                })?;
                self.buffers.insert(
                    handle,
                    BufferObject {
                        buffer,
                        size: desc.size,
                    },
                );
            }
            BackendCreateDesc::Texture(desc) => {
                let texture = create_texture(context, desc)?;
                self.textures.insert(handle, texture);
            }
            BackendCreateDesc::TextureView(desc) => {
                let texture = self
                    .textures
                    .get(&desc.texture)
                    .ok_or_else(|| missing("texture", desc.texture))?;
                self.views.insert(
                    handle,
                    TextureViewObject {
                        texture: texture.texture.clone(),
                        desc: desc.clone(),
                        format: texture.format,
                        texture_dimension: texture.desc.dimension,
                        views: HashMap::new(),
                    },
                );
            }
            BackendCreateDesc::Sampler(desc) => {
                let sampler = context.scoped("sampler creation", || {
                    context
                        .device
                        .create_sampler(&convert::sampler_descriptor(desc))
                })?;
                self.samplers.insert(handle, SamplerObject { sampler });
            }
            BackendCreateDesc::CombinedTextureSampler(desc) => {
                self.combined
                    .insert(handle, (desc.texture_view, desc.sampler));
            }
            BackendCreateDesc::ShaderModule(desc) => {
                let shader = ShaderObject::new(context, desc)?;
                self.shaders.insert(handle, shader);
            }
            BackendCreateDesc::ResourceLayout(desc) => {
                if let Some(binding) = desc.bindings.iter().find(|binding| binding.array_count > 1)
                {
                    return Err(GalError::unsupported_feature(format!(
                        "resource layout '{}' binding {} is an array; the wgpu backend binds single resources",
                        desc.label, binding.binding
                    )));
                }
                self.resource_layouts.insert(handle, desc.clone());
            }
            BackendCreateDesc::ResourceSet(desc) => {
                let set = self.create_resource_set(context, desc)?;
                self.sets.insert(handle, set);
            }
            BackendCreateDesc::PipelineLayout(desc) => {
                self.pipeline_layouts
                    .insert(handle, desc.resource_layouts.clone());
            }
            BackendCreateDesc::GraphicsPipeline(desc) => {
                let pipeline = self.create_graphics_pipeline(context, desc)?;
                self.graphics_pipelines.insert(handle, pipeline);
            }
            BackendCreateDesc::ComputePipeline(desc) => {
                let pipeline = self.create_compute_pipeline(context, desc)?;
                self.compute_pipelines.insert(handle, pipeline);
            }
            BackendCreateDesc::RenderTarget(desc) => {
                self.render_targets.insert(handle, desc.clone());
            }
            BackendCreateDesc::FrameTarget(desc) => {
                self.frame_targets.insert(
                    handle,
                    FrameTargetObject {
                        render_target: desc.render_target,
                    },
                );
            }
            BackendCreateDesc::RenderPass(desc) => {
                self.render_passes.insert(handle, desc.clone());
            }
        }
        self.next_token += 1;
        Ok(BackendToken(self.next_token))
    }

    pub(super) fn destroy(&mut self, handle: Handle, kind: HandleKind) -> GalResult<()> {
        let removed = match kind {
            HandleKind::Buffer => self
                .buffers
                .remove(&handle)
                .map(|buffer| buffer.buffer.destroy())
                .is_some(),
            HandleKind::Texture => self
                .textures
                .remove(&handle)
                .map(|texture| texture.texture.destroy())
                .is_some(),
            HandleKind::TextureView => self.views.remove(&handle).is_some(),
            HandleKind::Sampler => self.samplers.remove(&handle).is_some(),
            HandleKind::CombinedTextureSampler => self.combined.remove(&handle).is_some(),
            HandleKind::ShaderModule => self.shaders.remove(&handle).is_some(),
            HandleKind::ResourceLayout => self.resource_layouts.remove(&handle).is_some(),
            HandleKind::ResourceSet => self.sets.remove(&handle).is_some(),
            HandleKind::PipelineLayout => self.pipeline_layouts.remove(&handle).is_some(),
            HandleKind::GraphicsPipeline => self.graphics_pipelines.remove(&handle).is_some(),
            HandleKind::ComputePipeline => self.compute_pipelines.remove(&handle).is_some(),
            HandleKind::RenderTarget => self.render_targets.remove(&handle).is_some(),
            HandleKind::FrameTarget => self.frame_targets.remove(&handle).is_some(),
            HandleKind::RenderPass => self.render_passes.remove(&handle).is_some(),
        };
        if removed {
            Ok(())
        } else {
            Err(missing(&format!("{kind:?}"), handle))
        }
    }

    pub(super) fn buffer(&self, handle: Handle) -> GalResult<&BufferObject> {
        self.buffers
            .get(&handle)
            .ok_or_else(|| missing("buffer", handle))
    }

    pub(super) fn texture(&self, handle: Handle) -> GalResult<&TextureObject> {
        self.textures
            .get(&handle)
            .ok_or_else(|| missing("texture", handle))
    }

    pub(super) fn view_mut(&mut self, handle: Handle) -> GalResult<&mut TextureViewObject> {
        self.views
            .get_mut(&handle)
            .ok_or_else(|| missing("texture view", handle))
    }

    fn create_resource_set(
        &self,
        context: &WgpuContext,
        desc: &ResourceSetDesc,
    ) -> GalResult<ResourceSetObject> {
        let layout = self
            .resource_layouts
            .get(&desc.layout)
            .ok_or_else(|| missing("resource layout", desc.layout))?
            .clone();
        let mut default_offsets = BTreeMap::new();
        let mut unfilterable = Vec::new();
        for binding in &desc.bindings {
            if let Some(offset) = binding.dynamic_offsets.first() {
                default_offsets.insert(binding.binding, *offset);
            }
            let view = match binding.kind {
                ResourceBindingKind::SampledTexture => Some(binding.resource),
                ResourceBindingKind::CombinedTextureSampler => {
                    self.combined.get(&binding.resource).map(|(view, _)| *view)
                }
                _ => None,
            };
            if let Some(view) = view.and_then(|view| self.views.get(&view)) {
                if convert::unfilterable_float(view.format, context.features) {
                    unfilterable.push(binding.binding);
                }
            }
        }
        Ok(ResourceSetObject {
            desc: desc.clone(),
            layout,
            default_offsets,
            unfilterable,
            bind_groups: HashMap::new(),
        })
    }

    fn pipeline_resource_layouts(&self, layout: Handle) -> GalResult<Vec<ResourceLayoutDesc>> {
        self.pipeline_layouts
            .get(&layout)
            .ok_or_else(|| missing("pipeline layout", layout))?
            .iter()
            .map(|handle| {
                self.resource_layouts
                    .get(handle)
                    .cloned()
                    .ok_or_else(|| missing("resource layout", *handle))
            })
            .collect()
    }

    fn create_graphics_pipeline(
        &mut self,
        context: &WgpuContext,
        desc: &GraphicsPipelineDesc,
    ) -> GalResult<GraphicsPipelineObject> {
        let stencil_reference = match desc.stencil {
            Some(stencil) => {
                if stencil.front.reference != stencil.back.reference
                    || stencil.front.read_mask != stencil.back.read_mask
                    || stencil.front.write_mask != stencil.back.write_mask
                {
                    return Err(GalError::unsupported_feature(
                        "the wgpu backend shares one stencil reference and mask pair between faces",
                    ));
                }
                Some(stencil.front.reference)
            }
            None => None,
        };
        let resource_layouts = self.pipeline_resource_layouts(desc.layout)?;
        let vertex_shader = self
            .shaders
            .get_mut(&desc.vertex_shader)
            .ok_or_else(|| missing("vertex shader", desc.vertex_shader))?;
        let vertex = vertex_shader.variant(context, desc.raster_y_direction)?;
        let vertex_entry = vertex_shader.entry_point.clone();
        let fragment_shader = self
            .shaders
            .get_mut(&desc.fragment_shader)
            .ok_or_else(|| missing("fragment shader", desc.fragment_shader))?;
        let fragment = fragment_shader.variant(context, desc.raster_y_direction)?;
        let fragment_entry = fragment_shader.entry_point.clone();
        let bindings = PipelineBindings::new(
            &desc.label,
            resource_layouts,
            &[
                (&vertex.bindings, wgpu::ShaderStages::VERTEX),
                (&fragment.bindings, wgpu::ShaderStages::FRAGMENT),
            ],
        )?;
        let mut pipeline = GraphicsPipelineObject {
            desc: desc.clone(),
            bindings,
            vertex: vertex.module.clone(),
            fragment: fragment.module.clone(),
            vertex_entry,
            fragment_entry,
            variants: HashMap::new(),
            topology: desc.topology,
            provoking_last: desc.provoking_vertex == ProvokingVertex::Last
                && vertex.has_flat_outputs
                && desc.topology != PrimitiveTopology::Points,
            stencil_reference,
        };
        // Build the common variant now so invalid pipelines fail at creation.
        self.graphics_variant(context, &mut pipeline, 0)?;
        Ok(pipeline)
    }

    fn create_compute_pipeline(
        &mut self,
        context: &WgpuContext,
        desc: &ComputePipelineDesc,
    ) -> GalResult<ComputePipelineObject> {
        let resource_layouts = self.pipeline_resource_layouts(desc.layout)?;
        let shader = self
            .shaders
            .get_mut(&desc.shader)
            .ok_or_else(|| missing("compute shader", desc.shader))?;
        let entry = shader.entry_point.clone();
        let variant = shader.variant(context, RasterYDirection::Up)?;
        let bindings = PipelineBindings::new(
            &desc.label,
            resource_layouts,
            &[(&variant.bindings, wgpu::ShaderStages::COMPUTE)],
        )?;
        let mut pipeline = ComputePipelineObject {
            label: desc.label.clone(),
            bindings,
            shader: variant.module.clone(),
            entry,
            variants: HashMap::new(),
        };
        self.compute_variant(context, &mut pipeline, 0)?;
        Ok(pipeline)
    }

    fn group_layouts(
        &mut self,
        context: &WgpuContext,
        bindings: &PipelineBindings,
        key: u64,
    ) -> GalResult<Vec<Arc<BindGroupLayoutInfo>>> {
        bindings
            .entries(key)
            .into_iter()
            .map(|entries| self.bind_group_layout(context, entries))
            .collect()
    }

    fn bind_group_layout(
        &mut self,
        context: &WgpuContext,
        entries: Vec<wgpu::BindGroupLayoutEntry>,
    ) -> GalResult<Arc<BindGroupLayoutInfo>> {
        if let Some(layout) = self.layouts.get(&entries) {
            return Ok(layout.clone());
        }
        let layout = context.scoped("bind group layout creation", || {
            context
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: None,
                    entries: &entries,
                })
        })?;
        let dynamic_bindings = entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.ty,
                    wgpu::BindingType::Buffer {
                        has_dynamic_offset: true,
                        ..
                    }
                )
            })
            .map(|entry| gal_binding(entry.binding).0)
            .collect();
        self.next_layout_id += 1;
        let info = Arc::new(BindGroupLayoutInfo {
            id: self.next_layout_id,
            layout,
            entries: entries.clone(),
            dynamic_bindings,
        });
        self.layouts.insert(entries, info.clone());
        Ok(info)
    }

    pub(super) fn graphics_variant(
        &mut self,
        context: &WgpuContext,
        pipeline: &mut GraphicsPipelineObject,
        key: u64,
    ) -> GalResult<Arc<RenderVariant>> {
        if let Some(variant) = pipeline.variants.get(&key) {
            return Ok(variant.clone());
        }
        let groups = self.group_layouts(context, &pipeline.bindings, key)?;
        let desc = &pipeline.desc;
        let group_refs: Vec<Option<&wgpu::BindGroupLayout>> =
            groups.iter().map(|group| Some(&group.layout)).collect();
        let targets: Vec<Option<wgpu::ColorTargetState>> = desc
            .color_formats
            .iter()
            .enumerate()
            .map(|(index, format)| {
                let (blend, write_mask) = convert::color_blend(desc.blend, index);
                Some(wgpu::ColorTargetState {
                    format: convert::texture_format(*format),
                    blend,
                    write_mask,
                })
            })
            .collect();
        let depth_stencil = desc.depth_format.map(|format| {
            let stencil = desc
                .stencil
                .map_or_else(wgpu::StencilState::default, |stencil| wgpu::StencilState {
                    front: convert::stencil_face(&stencil.front),
                    back: convert::stencil_face(&stencil.back),
                    read_mask: stencil.front.read_mask,
                    write_mask: stencil.front.write_mask,
                });
            let bias = desc
                .depth_bias
                .map_or_else(wgpu::DepthBiasState::default, |bias| wgpu::DepthBiasState {
                    constant: bias.constant_factor.round() as i32,
                    slope_scale: bias.slope_factor,
                    clamp: 0.0,
                });
            wgpu::DepthStencilState {
                format: convert::texture_format(format),
                // A disabled depth test never writes, as on the other backends.
                depth_write_enabled: Some(desc.depth_write && desc.depth_compare.is_some()),
                depth_compare: Some(
                    desc.depth_compare
                        .map_or(wgpu::CompareFunction::Always, convert::compare),
                ),
                stencil,
                bias,
            }
        });
        let pipeline_desc = desc;
        let render = context.scoped("render pipeline creation", || {
            let layout = context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(&pipeline_desc.label),
                    bind_group_layouts: &group_refs,
                    immediate_size: 0,
                });
            context
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&pipeline_desc.label),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &pipeline.vertex,
                        entry_point: Some(&pipeline.vertex_entry),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: convert::topology(pipeline_desc.topology),
                        strip_index_format: None,
                        front_face: convert::front_face(
                            pipeline_desc.front_face,
                            pipeline_desc.raster_y_direction,
                        ),
                        cull_mode: convert::cull_mode(pipeline_desc.cull_mode),
                        unclipped_depth: false,
                        polygon_mode: wgpu::PolygonMode::Fill,
                        conservative: false,
                    },
                    depth_stencil: depth_stencil.clone(),
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &pipeline.fragment,
                        entry_point: Some(&pipeline.fragment_entry),
                        compilation_options: Default::default(),
                        targets: &targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })?;
        let variant = Arc::new(RenderVariant {
            pipeline: render,
            groups,
        });
        pipeline.variants.insert(key, variant.clone());
        Ok(variant)
    }

    pub(super) fn compute_variant(
        &mut self,
        context: &WgpuContext,
        pipeline: &mut ComputePipelineObject,
        key: u64,
    ) -> GalResult<Arc<ComputeVariant>> {
        if let Some(variant) = pipeline.variants.get(&key) {
            return Ok(variant.clone());
        }
        let groups = self.group_layouts(context, &pipeline.bindings, key)?;
        let group_refs: Vec<Option<&wgpu::BindGroupLayout>> =
            groups.iter().map(|group| Some(&group.layout)).collect();
        let compute = context.scoped("compute pipeline creation", || {
            let layout = context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(&pipeline.label),
                    bind_group_layouts: &group_refs,
                    immediate_size: 0,
                });
            context
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(&pipeline.label),
                    layout: Some(&layout),
                    module: &pipeline.shader,
                    entry_point: Some(&pipeline.entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        })?;
        let variant = Arc::new(ComputeVariant {
            pipeline: compute,
            groups,
        });
        pipeline.variants.insert(key, variant.clone());
        Ok(variant)
    }

    /// The bind group of `set` for `layout`, created on first use.
    pub(super) fn bind_group(
        &mut self,
        context: &WgpuContext,
        set: Handle,
        layout: &BindGroupLayoutInfo,
    ) -> GalResult<wgpu::BindGroup> {
        if let Some(group) = self
            .sets
            .get(&set)
            .ok_or_else(|| missing("resource set", set))?
            .bind_groups
            .get(&layout.id)
        {
            return Ok(group.clone());
        }
        let desc = self.sets[&set].desc.clone();
        let mut bound = Vec::with_capacity(layout.entries.len());
        for entry in &layout.entries {
            let (binding, sampler_half) = gal_binding(entry.binding);
            let resource = desc
                .bindings
                .iter()
                .find(|candidate| candidate.binding == binding && candidate.array_index == 0);
            let resolved = match resource {
                Some(resource) => {
                    self.resolve_binding(context, resource, sampler_half, entry, &desc.label)?
                }
                None => self.dummy(context, entry)?,
            };
            bound.push(resolved);
        }
        let entries: Vec<wgpu::BindGroupEntry<'_>> = layout
            .entries
            .iter()
            .zip(&bound)
            .map(|(entry, bound)| wgpu::BindGroupEntry {
                binding: entry.binding,
                resource: match bound {
                    Bound::Buffer(buffer, offset, size) => {
                        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: *offset,
                            size: *size,
                        })
                    }
                    Bound::View(view) => wgpu::BindingResource::TextureView(view),
                    Bound::Sampler(sampler) => wgpu::BindingResource::Sampler(sampler),
                },
            })
            .collect();
        let group = context.scoped("bind group creation", || {
            context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&desc.label),
                    layout: &layout.layout,
                    entries: &entries,
                })
        })?;
        drop(entries);
        if let Some(object) = self.sets.get_mut(&set) {
            object.bind_groups.insert(layout.id, group.clone());
        }
        Ok(group)
    }

    /// The bind group for a group the pipeline declares but no set fills.
    pub(super) fn empty_bind_group(
        &mut self,
        context: &WgpuContext,
        layout: &BindGroupLayoutInfo,
    ) -> GalResult<Option<wgpu::BindGroup>> {
        if !layout.entries.is_empty() {
            return Ok(None);
        }
        if let Some(group) = self.dummies.empty_groups.get(&layout.id) {
            return Ok(Some(group.clone()));
        }
        let group = context.scoped("bind group creation", || {
            context
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("vulkanic.empty-set"),
                    layout: &layout.layout,
                    entries: &[],
                })
        })?;
        self.dummies.empty_groups.insert(layout.id, group.clone());
        Ok(Some(group))
    }

    fn resolve_binding(
        &mut self,
        context: &WgpuContext,
        resource: &ResourceBinding,
        sampler_half: bool,
        entry: &wgpu::BindGroupLayoutEntry,
        set_label: &str,
    ) -> GalResult<Bound> {
        let mismatch = || {
            GalError::invalid_argument(format!(
                "resource set '{set_label}' binding {} holds {:?}, which does not match its shader use",
                resource.binding, resource.kind
            ))
        };
        let view_dimension = match entry.ty {
            wgpu::BindingType::Texture { view_dimension, .. }
            | wgpu::BindingType::StorageTexture { view_dimension, .. } => Some(view_dimension),
            _ => None,
        };
        match (resource.kind, entry.ty) {
            (
                ResourceBindingKind::UniformBuffer | ResourceBindingKind::StorageBuffer,
                wgpu::BindingType::Buffer { ty, .. },
            ) => {
                let buffer = self.buffer(resource.resource)?;
                let range = resource.buffer_range.unwrap_or_else(|| {
                    let max_default = resource.dynamic_offsets.iter().copied().max().unwrap_or(0);
                    buffer.size.saturating_sub(max_default)
                });
                let native_size = buffer.buffer.size();
                let limit = match ty {
                    wgpu::BufferBindingType::Uniform => {
                        context.limits.max_uniform_buffer_binding_size
                    }
                    wgpu::BufferBindingType::Storage { .. } => {
                        context.limits.max_storage_buffer_binding_size
                    }
                };
                // Storage bindings are sized in whole words.
                let size = range.next_multiple_of(4).min(native_size).min(limit).max(4);
                Ok(Bound::Buffer(
                    buffer.buffer.clone(),
                    0,
                    NonZeroU64::new(size),
                ))
            }
            (
                ResourceBindingKind::SampledTexture | ResourceBindingKind::StorageTexture,
                wgpu::BindingType::Texture { .. } | wgpu::BindingType::StorageTexture { .. },
            ) => {
                let dimension = view_dimension.ok_or_else(mismatch)?;
                Ok(Bound::View(
                    self.view_mut(resource.resource)?.binding_view(dimension),
                ))
            }
            (ResourceBindingKind::Sampler, wgpu::BindingType::Sampler(_)) => Ok(Bound::Sampler(
                self.samplers
                    .get(&resource.resource)
                    .ok_or_else(|| missing("sampler", resource.resource))?
                    .sampler
                    .clone(),
            )),
            (ResourceBindingKind::CombinedTextureSampler, wgpu::BindingType::Texture { .. })
                if !sampler_half =>
            {
                let (view, _) = *self
                    .combined
                    .get(&resource.resource)
                    .ok_or_else(|| missing("combined texture sampler", resource.resource))?;
                let dimension = view_dimension.ok_or_else(mismatch)?;
                Ok(Bound::View(self.view_mut(view)?.binding_view(dimension)))
            }
            (ResourceBindingKind::CombinedTextureSampler, wgpu::BindingType::Sampler(_))
                if sampler_half =>
            {
                let (_, sampler) = *self
                    .combined
                    .get(&resource.resource)
                    .ok_or_else(|| missing("combined texture sampler", resource.resource))?;
                Ok(Bound::Sampler(
                    self.samplers
                        .get(&sampler)
                        .ok_or_else(|| missing("sampler", sampler))?
                        .sampler
                        .clone(),
                ))
            }
            _ => Err(mismatch()),
        }
    }

    fn dummy(
        &mut self,
        context: &WgpuContext,
        entry: &wgpu::BindGroupLayoutEntry,
    ) -> GalResult<Bound> {
        let device = &context.device;
        match entry.ty {
            wgpu::BindingType::Buffer { .. } => {
                let buffer = self.dummies.buffer.get_or_insert_with(|| {
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("vulkanic.empty-buffer"),
                        size: 65_536,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE,
                        mapped_at_creation: false,
                    })
                });
                Ok(Bound::Buffer(buffer.clone(), 0, None))
            }
            wgpu::BindingType::Texture {
                sample_type,
                view_dimension,
                multisampled: false,
            } => {
                let (kind, format) = match sample_type {
                    wgpu::TextureSampleType::Float { .. } => (0, wgpu::TextureFormat::Rgba8Unorm),
                    wgpu::TextureSampleType::Depth => (1, wgpu::TextureFormat::Depth32Float),
                    wgpu::TextureSampleType::Uint => (2, wgpu::TextureFormat::R32Uint),
                    wgpu::TextureSampleType::Sint => (3, wgpu::TextureFormat::R32Sint),
                };
                let view = self
                    .dummies
                    .textures
                    .entry((view_dimension, kind))
                    .or_insert_with(|| {
                        dummy_view(
                            device,
                            format,
                            view_dimension,
                            wgpu::TextureUsages::TEXTURE_BINDING,
                        )
                    });
                Ok(Bound::View(view.clone()))
            }
            wgpu::BindingType::StorageTexture {
                format,
                view_dimension,
                ..
            } => {
                let view = self
                    .dummies
                    .storage
                    .entry((view_dimension, format))
                    .or_insert_with(|| {
                        dummy_view(
                            device,
                            format,
                            view_dimension,
                            wgpu::TextureUsages::STORAGE_BINDING,
                        )
                    });
                Ok(Bound::View(view.clone()))
            }
            wgpu::BindingType::Sampler(kind) => {
                let key = kind as u8;
                let sampler = self.dummies.samplers.entry(key).or_insert_with(|| {
                    device.create_sampler(&wgpu::SamplerDescriptor {
                        label: Some("vulkanic.empty-sampler"),
                        compare: (kind == wgpu::SamplerBindingType::Comparison)
                            .then_some(wgpu::CompareFunction::Always),
                        ..Default::default()
                    })
                });
                Ok(Bound::Sampler(sampler.clone()))
            }
            _ => Err(GalError::unsupported_feature(
                "the wgpu backend cannot leave this binding empty",
            )),
        }
    }
}

fn dummy_view(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    dimension: wgpu::TextureViewDimension,
    usage: wgpu::TextureUsages,
) -> wgpu::TextureView {
    let (texture_dimension, layers) = match dimension {
        wgpu::TextureViewDimension::D1 => (wgpu::TextureDimension::D1, 1),
        wgpu::TextureViewDimension::D3 => (wgpu::TextureDimension::D3, 1),
        wgpu::TextureViewDimension::Cube | wgpu::TextureViewDimension::CubeArray => {
            (wgpu::TextureDimension::D2, 6)
        }
        _ => (wgpu::TextureDimension::D2, 1),
    };
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("vulkanic.empty-texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: texture_dimension,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(dimension),
            ..Default::default()
        })
}

fn create_texture(context: &WgpuContext, desc: &TextureDesc) -> GalResult<TextureObject> {
    let format = convert::texture_format(desc.format);
    let requested = convert::texture_usages(&desc.usages);
    let allowed = context
        .adapter
        .get_texture_format_features(format)
        .allowed_usages;
    if !allowed.contains(requested) {
        return Err(GalError::unsupported_feature(format!(
            "texture '{}' format {:?} does not support {:?} on this wgpu adapter",
            desc.label,
            desc.format,
            requested - allowed
        )));
    }
    // Copies, row reversal and mip generation work through these usages.
    let mut extra = wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST;
    if desc.mip_levels > 1 {
        extra |= wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    }
    let usage = requested | (extra & allowed);
    let (dimension, depth_or_array_layers) = match desc.dimension {
        TextureDimension::D3 => (wgpu::TextureDimension::D3, desc.extent.depth),
        _ => (wgpu::TextureDimension::D2, desc.array_layers),
    };
    let texture = context.scoped("texture creation", || {
        context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&desc.label),
            size: wgpu::Extent3d {
                width: desc.extent.width,
                height: desc.extent.height,
                depth_or_array_layers,
            },
            mip_level_count: desc.mip_levels,
            sample_count: 1,
            dimension,
            format,
            usage,
            view_formats: &[],
        })
    })?;
    Ok(TextureObject {
        texture,
        desc: desc.clone(),
        format,
    })
}
