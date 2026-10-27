//! Internal pipelines that implement GAL operations wgpu lacks:
//!
//! - `byte_copy`: copies a 3D block of bytes between buffers with arbitrary
//!   offsets and row/image strides. wgpu copies need four-byte offsets and
//!   sizes, and texture copies need 256-byte rows; GAL host writes, buffer
//!   copies and tightly packed texture uploads need neither. Each invocation
//!   owns one destination word, so rows sharing a word never race.
//! - `mip_blit`: renders each mip level from the previous one with a
//!   linear filter, for `GenerateMipmaps`.

use std::collections::HashMap;

use super::device::WgpuContext;
use super::resources::TextureObject;
use crate::error::{GalError, GalResult};
use crate::resources::TextureSubresourceRange;

const BYTE_COPY_SHADER: &str = r#"
struct Params {
    src_offset: u32,
    src_row_stride: u32,
    src_image_stride: u32,
    dst_offset: u32,
    dst_row_stride: u32,
    dst_image_stride: u32,
    row_bytes: u32,
    rows: u32,
    images: u32,
    first_word: u32,
    word_count: u32,
    groups_x: u32,
}

@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
@group(0) @binding(2) var<uniform> p: Params;

fn src_byte(index: u32) -> u32 {
    return (src[index >> 2u] >> ((index & 3u) * 8u)) & 0xffu;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let local = id.y * p.groups_x * 64u + id.x;
    if (local >= p.word_count) {
        return;
    }
    let word = p.first_word + local;
    var value = dst[word];
    for (var k = 0u; k < 4u; k = k + 1u) {
        let byte = word * 4u + k;
        if (byte < p.dst_offset) {
            continue;
        }
        let relative = byte - p.dst_offset;
        let image = relative / p.dst_image_stride;
        let in_image = relative % p.dst_image_stride;
        let row = in_image / p.dst_row_stride;
        let column = in_image % p.dst_row_stride;
        if (image < p.images && row < p.rows && column < p.row_bytes) {
            let source = p.src_offset + image * p.src_image_stride + row * p.src_row_stride + column;
            let shift = k * 8u;
            value = (value & ~(0xffu << shift)) | (src_byte(source) << shift);
        }
    }
    dst[word] = value;
}
"#;

const INDEX_ROTATION_SHADER: &str = r#"
struct Params {
    element_bytes: u32,
    elements: u32,
    lines: u32,
    groups_x: u32,
    phase: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
@group(0) @binding(2) var<uniform> p: Params;

// The element that moves to position `k`: each primitive's last vertex
// first, keeping cyclic (winding) order. Primitives start at element
// `phase` (where the bound offset puts the first one); elements before it
// and incomplete trailing primitives keep their positions.
fn source_element(k: u32) -> u32 {
    if (k < p.phase) {
        return k;
    }
    let j = k - p.phase;
    var s = j ^ 1u;
    if (p.lines == 0u) {
        let corner = j % 3u;
        s = j - corner + select(corner - 1u, 2u, corner == 0u);
    }
    s = s + p.phase;
    return select(k, s, s < p.elements);
}

fn load(k: u32) -> u32 {
    if (p.element_bytes == 4u) {
        return src[k];
    }
    return (src[k >> 1u] >> ((k & 1u) * 16u)) & 0xffffu;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let word = id.y * p.groups_x * 64u + id.x;
    if (p.element_bytes == 4u) {
        if (word < p.elements) {
            dst[word] = load(source_element(word));
        }
        return;
    }
    let first = word * 2u;
    if (first >= p.elements) {
        return;
    }
    var value = load(source_element(first));
    if (first + 1u < p.elements) {
        value = value | (load(source_element(first + 1u)) << 16u);
    }
    dst[word] = value;
}
"#;

const MIP_BLIT_SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) index: u32) -> Varyings {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Varyings;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs(in: Varyings) -> @location(0) vec4<f32> {
    return textureSampleLevel(source, source_sampler, in.uv, 0.0);
}
"#;

/// A byte block copy, in bytes. Strides apply between rows and images of
/// both sides; `images` and `rows` count blocks of `row_bytes`.
#[derive(Clone, Copy, Debug)]
pub(super) struct ByteCopy {
    pub(super) src_offset: u64,
    pub(super) src_row_stride: u64,
    pub(super) src_image_stride: u64,
    pub(super) dst_offset: u64,
    pub(super) dst_row_stride: u64,
    pub(super) dst_image_stride: u64,
    pub(super) row_bytes: u64,
    pub(super) rows: u64,
    pub(super) images: u64,
}

impl ByteCopy {
    pub(super) fn linear(src_offset: u64, dst_offset: u64, size: u64) -> Self {
        Self {
            src_offset,
            src_row_stride: size,
            src_image_stride: size,
            dst_offset,
            dst_row_stride: size,
            dst_image_stride: size,
            row_bytes: size,
            rows: 1,
            images: 1,
        }
    }

    fn src_range(&self) -> (u64, u64) {
        let end = self.src_offset
            + (self.images - 1) * self.src_image_stride
            + (self.rows - 1) * self.src_row_stride
            + self.row_bytes;
        (self.src_offset, end)
    }

    fn dst_range(&self) -> (u64, u64) {
        let end = self.dst_offset
            + (self.images - 1) * self.dst_image_stride
            + (self.rows - 1) * self.dst_row_stride
            + self.row_bytes;
        (self.dst_offset, end)
    }
}

/// A byte copy ready to record: bound windows `(offset, size)`, the
/// parameter block and the dispatch size.
pub(super) struct ByteCopyPlan {
    params: [u32; 12],
    src_window: (u64, u64),
    dst_window: (u64, u64),
    groups: (u32, u32),
}

impl ByteCopyPlan {
    pub(super) fn param_bytes(&self) -> Vec<u8> {
        self.params
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }
}

/// A whole-buffer index rotation ready to record.
pub(super) struct IndexRotationPlan {
    params: [u32; 8],
    groups: (u32, u32),
}

impl IndexRotationPlan {
    pub(super) fn param_bytes(&self) -> Vec<u8> {
        self.params
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }
}

pub(super) struct Utility {
    byte_copy: Option<(wgpu::ComputePipeline, wgpu::BindGroupLayout)>,
    index_rotation: Option<(wgpu::ComputePipeline, wgpu::BindGroupLayout)>,
    mip_module: Option<wgpu::ShaderModule>,
    mip_layouts: HashMap<bool, (wgpu::BindGroupLayout, wgpu::PipelineLayout)>,
    mip_pipelines: HashMap<(wgpu::TextureFormat, bool), wgpu::RenderPipeline>,
    mip_samplers: HashMap<bool, wgpu::Sampler>,
}

impl Utility {
    pub(super) fn new() -> Self {
        Self {
            byte_copy: None,
            index_rotation: None,
            mip_module: None,
            mip_layouts: HashMap::new(),
            mip_pipelines: HashMap::new(),
            mip_samplers: HashMap::new(),
        }
    }

    fn byte_copy_pipeline(
        &mut self,
        context: &WgpuContext,
    ) -> GalResult<(wgpu::ComputePipeline, wgpu::BindGroupLayout)> {
        if let Some(pipeline) = &self.byte_copy {
            return Ok(pipeline.clone());
        }
        let created = copy_pipeline(context, "vulkanic.byte-copy", BYTE_COPY_SHADER)?;
        self.byte_copy = Some(created.clone());
        Ok(created)
    }

    fn index_rotation_pipeline(
        &mut self,
        context: &WgpuContext,
    ) -> GalResult<(wgpu::ComputePipeline, wgpu::BindGroupLayout)> {
        if let Some(pipeline) = &self.index_rotation {
            return Ok(pipeline.clone());
        }
        let created = copy_pipeline(context, "vulkanic.index-rotation", INDEX_ROTATION_SHADER)?;
        self.index_rotation = Some(created.clone());
        Ok(created)
    }

    /// Plans rotating every primitive of the index buffer `src`, with
    /// primitives starting at element `phase`.
    pub(super) fn plan_index_rotation(
        context: &WgpuContext,
        src: &wgpu::Buffer,
        element_bytes: u32,
        lines: bool,
        phase: u32,
    ) -> GalResult<IndexRotationPlan> {
        if src.size() > context.limits.max_storage_buffer_binding_size {
            return Err(GalError::unsupported_feature(
                "index buffer exceeds the largest storage binding for last-vertex rotation",
            ));
        }
        let elements = u32::try_from(src.size() / u64::from(element_bytes))
            .map_err(|_| GalError::unsupported_feature("index buffer exceeds 4 GiB"))?;
        let words = elements.div_ceil(4 / element_bytes);
        let groups = words.div_ceil(64).max(1);
        let groups_x = groups.min(context.limits.max_compute_workgroups_per_dimension);
        Ok(IndexRotationPlan {
            params: [
                element_bytes,
                elements,
                u32::from(lines),
                groups_x,
                phase,
                0,
                0,
                0,
            ],
            groups: (groups_x, groups.div_ceil(groups_x)),
        })
    }

    pub(super) fn record_index_rotation(
        &mut self,
        context: &WgpuContext,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::Buffer,
        dst: &wgpu::Buffer,
        plan: &IndexRotationPlan,
        params: &wgpu::Buffer,
    ) -> GalResult<()> {
        let (pipeline, layout) = self.index_rotation_pipeline(context)?;
        let group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vulkanic.index-rotation"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: src.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: dst.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("vulkanic.index-rotation"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(plan.groups.0, plan.groups.1, 1);
        Ok(())
    }

    /// Plans `copy` from `src` to `dst` (distinct buffers): the storage
    /// windows to bind and the parameter block, or `None` for an empty copy.
    pub(super) fn plan_byte_copy(
        context: &WgpuContext,
        src: &wgpu::Buffer,
        dst: &wgpu::Buffer,
        copy: ByteCopy,
    ) -> GalResult<Option<ByteCopyPlan>> {
        if copy.row_bytes == 0 || copy.rows == 0 || copy.images == 0 {
            return Ok(None);
        }
        let alignment = u64::from(context.limits.min_storage_buffer_offset_alignment).max(4);
        let max_binding = context.limits.max_storage_buffer_binding_size;
        let window = |buffer: &wgpu::Buffer, (start, end): (u64, u64)| -> GalResult<(u64, u64)> {
            let base = start / alignment * alignment;
            let size = (buffer.size() - base).min(max_binding) / 4 * 4;
            if end - base > size {
                return Err(GalError::unsupported_feature(
                    "wgpu byte copy exceeds the largest storage buffer binding",
                ));
            }
            Ok((base, size))
        };
        let (src_base, src_size) = window(src, copy.src_range())?;
        let (dst_start, dst_end) = copy.dst_range();
        let (dst_base, dst_size) = window(dst, (dst_start, dst_end))?;
        let to_u32 = |value: u64| {
            u32::try_from(value)
                .map_err(|_| GalError::unsupported_feature("wgpu byte copy exceeds 4 GiB"))
        };
        let first_word = (dst_start - dst_base) / 4;
        let word_count = (dst_end - dst_base).div_ceil(4) - first_word;
        let groups = word_count.div_ceil(64);
        let groups_x = groups.min(u64::from(
            context.limits.max_compute_workgroups_per_dimension,
        ));
        let groups_y = groups.div_ceil(groups_x);
        Ok(Some(ByteCopyPlan {
            params: [
                to_u32(copy.src_offset - src_base)?,
                to_u32(copy.src_row_stride.max(1))?,
                to_u32(copy.src_image_stride.max(1))?,
                to_u32(copy.dst_offset - dst_base)?,
                to_u32(copy.dst_row_stride.max(1))?,
                to_u32(copy.dst_image_stride.max(1))?,
                to_u32(copy.row_bytes)?,
                to_u32(copy.rows)?,
                to_u32(copy.images)?,
                to_u32(first_word)?,
                to_u32(word_count)?,
                to_u32(groups_x)?,
            ],
            src_window: (src_base, src_size),
            dst_window: (dst_base, dst_size),
            groups: (to_u32(groups_x)?, to_u32(groups_y)?),
        }))
    }

    /// Records a planned byte copy; `params` holds `plan.param_bytes()`.
    pub(super) fn record_byte_copy(
        &mut self,
        context: &WgpuContext,
        encoder: &mut wgpu::CommandEncoder,
        src: &wgpu::Buffer,
        dst: &wgpu::Buffer,
        plan: &ByteCopyPlan,
        params: &wgpu::Buffer,
    ) -> GalResult<()> {
        let (pipeline, layout) = self.byte_copy_pipeline(context)?;
        let group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vulkanic.byte-copy"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: src,
                            offset: plan.src_window.0,
                            size: std::num::NonZeroU64::new(plan.src_window.1),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: dst,
                            offset: plan.dst_window.0,
                            size: std::num::NonZeroU64::new(plan.dst_window.1),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("vulkanic.byte-copy"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(plan.groups.0, plan.groups.1, 1);
        Ok(())
    }

    /// Renders mips `base + 1 .. base + count` of each layer from the level
    /// above. `filterable` selects linear filtering.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn generate_mipmaps(
        &mut self,
        context: &WgpuContext,
        encoder: &mut wgpu::CommandEncoder,
        texture: &mut TextureObject,
        filterable: bool,
        range: TextureSubresourceRange,
    ) -> GalResult<()> {
        let device = &context.device;
        let format = texture.format;
        let module = match &self.mip_module {
            Some(module) => module.clone(),
            None => {
                let module = context.scoped("mip shader creation", || {
                    device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("vulkanic.mip-blit"),
                        source: wgpu::ShaderSource::Wgsl(MIP_BLIT_SHADER.into()),
                    })
                })?;
                self.mip_module = Some(module.clone());
                module
            }
        };
        let (layout, pipeline_layout) = match self.mip_layouts.get(&filterable) {
            Some(layouts) => layouts.clone(),
            None => {
                let layouts = context.scoped("mip pipeline layout creation", || {
                    let layout =
                        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                            label: Some("vulkanic.mip-blit"),
                            entries: &[
                                wgpu::BindGroupLayoutEntry {
                                    binding: 0,
                                    visibility: wgpu::ShaderStages::FRAGMENT,
                                    ty: wgpu::BindingType::Texture {
                                        sample_type: wgpu::TextureSampleType::Float { filterable },
                                        view_dimension: wgpu::TextureViewDimension::D2,
                                        multisampled: false,
                                    },
                                    count: None,
                                },
                                wgpu::BindGroupLayoutEntry {
                                    binding: 1,
                                    visibility: wgpu::ShaderStages::FRAGMENT,
                                    ty: wgpu::BindingType::Sampler(if filterable {
                                        wgpu::SamplerBindingType::Filtering
                                    } else {
                                        wgpu::SamplerBindingType::NonFiltering
                                    }),
                                    count: None,
                                },
                            ],
                        });
                    let pipeline_layout =
                        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some("vulkanic.mip-blit"),
                            bind_group_layouts: &[Some(&layout)],
                            immediate_size: 0,
                        });
                    (layout, pipeline_layout)
                })?;
                self.mip_layouts.insert(filterable, layouts.clone());
                layouts
            }
        };
        let pipeline = match self.mip_pipelines.get(&(format, filterable)) {
            Some(pipeline) => pipeline.clone(),
            None => {
                let pipeline = context.scoped("mip pipeline creation", || {
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("vulkanic.mip-blit"),
                        layout: Some(&pipeline_layout),
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some("vs"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some("fs"),
                            compilation_options: Default::default(),
                            targets: &[Some(format.into())],
                        }),
                        multiview_mask: None,
                        cache: None,
                    })
                })?;
                self.mip_pipelines
                    .insert((format, filterable), pipeline.clone());
                pipeline
            }
        };
        let sampler = self
            .mip_samplers
            .entry(filterable)
            .or_insert_with(|| {
                let filter = if filterable {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                };
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("vulkanic.mip-blit"),
                    mag_filter: filter,
                    min_filter: filter,
                    ..Default::default()
                })
            })
            .clone();
        for layer in range.base_layer..range.base_layer + range.layer_count {
            for mip in range.base_mip + 1..range.base_mip + range.mip_count {
                let native = &texture.texture;
                let (group, target) = texture
                    .mip_blits
                    .entry((filterable, layer, mip))
                    .or_insert_with(|| {
                        let view = |level| {
                            native.create_view(&wgpu::TextureViewDescriptor {
                                label: Some("vulkanic.mip-blit"),
                                dimension: Some(wgpu::TextureViewDimension::D2),
                                base_mip_level: level,
                                mip_level_count: Some(1),
                                base_array_layer: layer,
                                array_layer_count: Some(1),
                                ..Default::default()
                            })
                        };
                        let source = view(mip - 1);
                        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("vulkanic.mip-blit"),
                            layout: &layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::TextureView(&source),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::Sampler(&sampler),
                                },
                            ],
                        });
                        (group, view(mip))
                    });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("vulkanic.mip-blit"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &*group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        Ok(())
    }
}

/// A compute pipeline over (read-only storage, read-write storage, uniform)
/// bindings, the shape of every internal copy shader.
fn copy_pipeline(
    context: &WgpuContext,
    label: &'static str,
    shader: &'static str,
) -> GalResult<(wgpu::ComputePipeline, wgpu::BindGroupLayout)> {
    let device = &context.device;
    context.scoped("internal copy pipeline creation", || {
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[
                storage(0, true),
                storage(1, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        (pipeline, layout)
    })
}
