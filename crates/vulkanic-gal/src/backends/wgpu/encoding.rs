//! Lowers validated GAL command lists into one wgpu command buffer.
//!
//! - Barriers are validated by the GAL and dropped: wgpu tracks usage and
//!   synchronizes itself.
//! - Pipelines and resource sets are applied lazily before each draw or
//!   dispatch, so a pipeline switch can pick its unfilterable-binding
//!   variant and rebind sets whose wgpu layout changed.
//! - Host writes keep their position in the command stream: bytes are
//!   staged in a `StagingBelt` and copied at the op, never written ahead
//!   of the whole submission. Unaligned writes and copies go through the
//!   byte-copy pipeline.
//! - Host reads copy into mappable buffers after every list, matching the
//!   Vulkan backend, which reads the buffer once the submission completes.
//! - Triangle fans draw generated triangle-list indices.

use std::collections::HashMap;
use std::sync::Arc;

use super::convert;
use super::device::WgpuContext;
use super::resources::{ComputeVariant, RenderVariant, WgpuObjects};
use super::utility::{ByteCopy, Utility};
use crate::commands::{
    AttachmentLoadOp, BufferImageCopyRegion, CommandOp, TextureImageCopyRegion, TextureRowOrder,
    ValidatedSubmissionBatch,
};
use crate::error::{GalError, GalResult};
use crate::frame::FrameRenderTargetId;
use crate::handles::{Handle, HandleKind};
use crate::resources::{IndexType, PrimitiveTopology, TextureDimension, TextureFormat};

/// How generated or rotated indices reorder a primitive's vertices.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum IndexPattern {
    /// Triangle fan as a list: `(i + 1, i + 2, 0)`, Vulkan's fan order.
    Fan,
    /// Fan with its last vertex first: `(i + 2, 0, i + 1)`.
    FanLast,
    /// Triangle list with its last vertex first: `(3t + 2, 3t, 3t + 1)`.
    TrianglesLast,
    /// Line list with its last vertex first: `(2l + 1, 2l)`.
    LinesLast,
}

impl IndexPattern {
    fn for_draw(topology: PrimitiveTopology, provoking_last: bool) -> Option<Self> {
        match (topology, provoking_last) {
            (PrimitiveTopology::TriangleFan, false) => Some(Self::Fan),
            (PrimitiveTopology::TriangleFan, true) => Some(Self::FanLast),
            (PrimitiveTopology::Triangles, true) => Some(Self::TrianglesLast),
            (PrimitiveTopology::Lines, true) => Some(Self::LinesLast),
            _ => None,
        }
    }

    /// Indices drawn for `vertices` input vertices.
    fn index_count(self, vertices: u32) -> u32 {
        match self {
            Self::Fan | Self::FanLast => 3 * vertices.saturating_sub(2),
            Self::TrianglesLast => vertices - vertices % 3,
            Self::LinesLast => vertices - vertices % 2,
        }
    }

    fn indices(self, vertices: u32) -> Vec<u32> {
        match self {
            Self::Fan => (0..vertices.saturating_sub(2))
                .flat_map(|i| [i + 1, i + 2, 0])
                .collect(),
            Self::FanLast => (0..vertices.saturating_sub(2))
                .flat_map(|i| [i + 2, 0, i + 1])
                .collect(),
            Self::TrianglesLast => (0..vertices / 3)
                .flat_map(|t| [3 * t + 2, 3 * t, 3 * t + 1])
                .collect(),
            Self::LinesLast => (0..vertices / 2).flat_map(|l| [2 * l + 1, 2 * l]).collect(),
        }
    }
}

/// How a rotated copy of an index buffer reorders it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum Rotation {
    Triangles,
    Lines,
}

/// Generated index lists and rotated index-buffer copies, kept by the
/// backend across submissions. A rotated copy is dropped as soon as its
/// source buffer is written.
#[derive(Default)]
pub(super) struct IndexCache {
    patterns: HashMap<IndexPattern, (wgpu::Buffer, u32)>,
    rotated: HashMap<(Handle, wgpu::IndexFormat, Rotation), wgpu::Buffer>,
}

impl IndexCache {
    pub(super) fn forget(&mut self, buffer: Handle) {
        if !self.rotated.is_empty() {
            self.rotated.retain(|(source, _, _), _| *source != buffer);
        }
    }
}

/// The draw-relevant shape of the bound graphics pipeline.
#[derive(Clone, Copy)]
struct DrawShape {
    topology: PrimitiveTopology,
    provoking_last: bool,
}

impl DrawShape {
    fn rotation(self) -> Option<Rotation> {
        match (self.provoking_last, self.topology) {
            (true, PrimitiveTopology::Triangles) => Some(Rotation::Triangles),
            (true, PrimitiveTopology::Lines) => Some(Rotation::Lines),
            _ => None,
        }
    }
}

/// A host read waiting for its submission.
pub(super) struct PendingRead {
    pub(super) buffer: Handle,
    pub(super) offset: u64,
    pub(super) size: u64,
    pub(super) staging: wgpu::Buffer,
    /// Where `offset` falls in `staging` (the copy is word aligned).
    pub(super) staging_offset: u64,
}

pub(super) struct Encoded {
    pub(super) command_buffer: wgpu::CommandBuffer,
    pub(super) host_reads: Vec<PendingRead>,
    pub(super) ops: u64,
}

/// The acquired surface texture frame targets render to.
pub(super) struct AcquiredSurface<'a> {
    pub(super) render_target: FrameRenderTargetId,
    pub(super) texture: &'a wgpu::Texture,
    pub(super) view: &'a wgpu::TextureView,
}

#[derive(Clone, PartialEq)]
struct BoundSet {
    set: Handle,
    offsets: Vec<u64>,
}

/// What one wgpu group slot holds: layout id, set (none for an empty
/// group) and dynamic offsets.
type AppliedGroup = (u64, Option<Handle>, Vec<u32>);

#[derive(Default)]
struct BindState {
    sets: Vec<Option<BoundSet>>,
    applied: Vec<Option<AppliedGroup>>,
}

impl BindState {
    fn bind(&mut self, index: u32, set: BoundSet) {
        let index = index as usize;
        if self.sets.len() <= index {
            self.sets.resize(index + 1, None);
        }
        self.sets[index] = Some(set);
    }

    fn unfilterable_key(&self, objects: &WgpuObjects, float_textures: &[(u32, u32)]) -> u64 {
        let mut key = 0;
        for (index, (group, binding)) in float_textures.iter().enumerate().take(64) {
            let unfilterable = self
                .sets
                .get(*group as usize)
                .and_then(Option::as_ref)
                .and_then(|bound| objects.sets.get(&bound.set))
                .is_some_and(|set| set.is_unfilterable(*binding));
            if unfilterable {
                key |= 1 << index;
            }
        }
        key
    }
}

struct Lowering<'a> {
    context: &'a WgpuContext,
    objects: &'a mut WgpuObjects,
    utility: &'a mut Utility,
    belt: &'a mut wgpu::util::StagingBelt,
    indices: &'a mut IndexCache,
    surface: Option<AcquiredSurface<'a>>,
    encoder: wgpu::CommandEncoder,
    render: Option<wgpu::RenderPass<'static>>,
    compute: Option<wgpu::ComputePass<'static>>,
    graphics_pipeline: Option<Handle>,
    applied_graphics: Option<(Handle, u64, Arc<RenderVariant>)>,
    compute_pipeline: Option<Handle>,
    applied_compute: Option<(Handle, u64, Arc<ComputeVariant>)>,
    graphics_sets: BindState,
    compute_sets: BindState,
    index_buffer: Option<(Handle, u64, wgpu::IndexFormat)>,
    /// The index buffer the pass holds: source, offset, format, rotation.
    applied_index: Option<(Handle, u64, wgpu::IndexFormat, Option<Rotation>)>,
    host_reads: Vec<(Handle, u64, u64)>,
}

pub(super) struct EncodeInputs<'a> {
    pub(super) context: &'a WgpuContext,
    pub(super) objects: &'a mut WgpuObjects,
    pub(super) utility: &'a mut Utility,
    pub(super) belt: &'a mut wgpu::util::StagingBelt,
    pub(super) indices: &'a mut IndexCache,
    pub(super) surface: Option<AcquiredSurface<'a>>,
}

pub(super) fn encode(
    inputs: EncodeInputs<'_>,
    batch: &ValidatedSubmissionBatch,
) -> GalResult<Encoded> {
    let encoder = inputs
        .context
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some(&batch.label),
        });
    let mut lowering = Lowering {
        context: inputs.context,
        objects: inputs.objects,
        utility: inputs.utility,
        belt: inputs.belt,
        indices: inputs.indices,
        surface: inputs.surface,
        encoder,
        render: None,
        compute: None,
        graphics_pipeline: None,
        applied_graphics: None,
        compute_pipeline: None,
        applied_compute: None,
        graphics_sets: BindState::default(),
        compute_sets: BindState::default(),
        index_buffer: None,
        applied_index: None,
        host_reads: Vec::new(),
    };
    let mut ops = 0;
    for list in &batch.command_lists {
        lowering.encoder.push_debug_group(&list.label);
        for (index, op) in list.operations.iter().enumerate() {
            lowering
                .op(op, &list.operations[index + 1..])
                .map_err(|error| {
                    GalError::backend(format!(
                        "wgpu lowering of command list '{}' failed: {error}",
                        list.label
                    ))
                })?;
            ops += 1;
        }
        lowering.end_passes();
        lowering.encoder.pop_debug_group();
        // Like a native command buffer boundary, state does not carry over.
        lowering.compute_pipeline = None;
        lowering.applied_compute = None;
        lowering.compute_sets = BindState::default();
    }
    lowering.end_passes();
    let host_reads = lowering.finish_host_reads()?;
    let Lowering { encoder, belt, .. } = lowering;
    belt.finish();
    Ok(Encoded {
        command_buffer: encoder.finish(),
        host_reads,
        ops,
    })
}

fn index_format(index_type: IndexType) -> wgpu::IndexFormat {
    match index_type {
        IndexType::U16 => wgpu::IndexFormat::Uint16,
        IndexType::U32 => wgpu::IndexFormat::Uint32,
    }
}

impl Lowering<'_> {
    fn end_compute(&mut self) {
        self.compute = None;
    }

    fn end_passes(&mut self) {
        self.render = None;
        self.compute = None;
        self.graphics_pipeline = None;
        self.applied_graphics = None;
        self.graphics_sets = BindState::default();
        self.index_buffer = None;
        self.applied_index = None;
    }

    /// Lowers `op`; `rest` is the remainder of its command list.
    fn op(&mut self, op: &CommandOp, rest: &[CommandOp]) -> GalResult<()> {
        match op {
            CommandOp::TrackSubmission(_) | CommandOp::Barrier(_) => {}
            CommandOp::BeginPass {
                target,
                colors,
                depth_stencil,
                ..
            } => {
                self.prepare_rotated_indices(rest)?;
                self.begin_pass(*target, colors, depth_stencil.as_ref())?
            }
            CommandOp::EndPass => {
                self.render = None;
                self.graphics_pipeline = None;
                self.applied_graphics = None;
                self.graphics_sets = BindState::default();
                self.index_buffer = None;
                self.applied_index = None;
            }
            CommandOp::BindGraphicsPipeline(pipeline) => self.graphics_pipeline = Some(*pipeline),
            CommandOp::BindComputePipeline(pipeline) => self.compute_pipeline = Some(*pipeline),
            CommandOp::BindResourceSet {
                set_index,
                set,
                dynamic_offsets,
                ..
            } => {
                let bound = BoundSet {
                    set: *set,
                    offsets: dynamic_offsets.clone(),
                };
                if self.render.is_some() {
                    self.graphics_sets.bind(*set_index, bound);
                } else {
                    self.compute_sets.bind(*set_index, bound);
                }
            }
            // Pipelines declare no vertex attributes; shaders pull vertices
            // from storage buffers.
            CommandOp::SetVertexBuffer { .. } => {}
            CommandOp::SetIndexBuffer {
                buffer,
                offset,
                index_type,
            } => {
                self.index_buffer = Some((*buffer, *offset, index_format(*index_type)));
                self.applied_index = None;
            }
            CommandOp::Draw {
                vertices,
                instances,
            } => {
                let shape = self.flush_graphics()?;
                match IndexPattern::for_draw(shape.topology, shape.provoking_last) {
                    None => self.render_pass()?.draw(0..*vertices, 0..*instances),
                    Some(pattern) => {
                        let count = pattern.index_count(*vertices);
                        if count == 0 {
                            return Ok(());
                        }
                        let indices = self.pattern_buffer(pattern, *vertices)?;
                        let pass = self.render_pass()?;
                        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(0..count, 0, 0..*instances);
                        self.applied_index = None;
                    }
                }
            }
            CommandOp::DrawIndexed { indices, instances } => {
                self.flush_indexed()?;
                self.render_pass()?
                    .draw_indexed(0..*indices, 0, 0..*instances);
            }
            CommandOp::DrawIndirect {
                buffer,
                offset,
                draw_count,
            } => {
                let shape = self.flush_graphics()?;
                if IndexPattern::for_draw(shape.topology, shape.provoking_last).is_some() {
                    return Err(GalError::unsupported_feature(
                        "the wgpu backend draws triangle fans and last-vertex flat shading only with direct or indexed draws",
                    ));
                }
                let buffer = self.objects.buffer(*buffer)?.buffer.clone();
                let pass = self.render_pass()?;
                if *draw_count == 1 {
                    pass.draw_indirect(&buffer, *offset);
                } else {
                    pass.multi_draw_indirect(&buffer, *offset, *draw_count);
                }
            }
            CommandOp::DrawIndexedIndirect {
                buffer,
                offset,
                draw_count,
            } => {
                self.flush_indexed()?;
                let buffer = self.objects.buffer(*buffer)?.buffer.clone();
                let pass = self.render_pass()?;
                if *draw_count == 1 {
                    pass.draw_indexed_indirect(&buffer, *offset);
                } else {
                    pass.multi_draw_indexed_indirect(&buffer, *offset, *draw_count);
                }
            }
            CommandOp::Dispatch {
                groups_x,
                groups_y,
                groups_z,
            } => {
                self.flush_compute()?;
                self.compute
                    .as_mut()
                    .expect("flush opened a compute pass")
                    .dispatch_workgroups(*groups_x, *groups_y, *groups_z);
            }
            CommandOp::DispatchIndirect { buffer, offset } => {
                self.flush_compute()?;
                let buffer = self.objects.buffer(*buffer)?.buffer.clone();
                self.compute
                    .as_mut()
                    .expect("flush opened a compute pass")
                    .dispatch_workgroups_indirect(&buffer, *offset);
            }
            CommandOp::CopyBuffer { src, dst, size } => {
                self.copy_buffer(*src, 0, *dst, 0, *size)?
            }
            CommandOp::CopyBufferRegion {
                src,
                src_offset,
                dst,
                dst_offset,
                size,
            } => self.copy_buffer(*src, *src_offset, *dst, *dst_offset, *size)?,
            CommandOp::CopyBufferToTexture(region) => self.copy_buffer_to_texture(region)?,
            CommandOp::CopyTextureToBuffer(region) => self.copy_texture_to_buffer(region)?,
            CommandOp::CopyTexture(region) => self.copy_texture(region)?,
            CommandOp::CopyFrameTargetToTexture { src, dst, extent } => {
                self.end_compute();
                let surface = self.surface_for(*src)?;
                let surface_texture = surface.texture.clone();
                let texture = self.objects.texture(*dst)?.texture.clone();
                self.encoder.copy_texture_to_texture(
                    surface_texture.as_image_copy(),
                    texture.as_image_copy(),
                    wgpu::Extent3d {
                        width: extent.width,
                        height: extent.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            CommandOp::CopyTextureToFrameTarget { src, dst, extent } => {
                self.end_compute();
                let surface = self.surface_for(*dst)?;
                let surface_texture = surface.texture.clone();
                let texture = self.objects.texture(*src)?.texture.clone();
                self.encoder.copy_texture_to_texture(
                    texture.as_image_copy(),
                    surface_texture.as_image_copy(),
                    wgpu::Extent3d {
                        width: extent.width,
                        height: extent.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            CommandOp::GenerateMipmaps {
                texture,
                subresources,
            } => {
                self.end_compute();
                let object = self.objects.texture(*texture)?;
                if object.desc.dimension == TextureDimension::D3 {
                    return Err(GalError::unsupported_feature(
                        "the wgpu backend generates mipmaps for 2D textures only",
                    ));
                }
                let filterable = !convert::unfilterable_float(object.format, self.context.features);
                let (native, format) = (object.texture.clone(), object.format);
                self.utility.generate_mipmaps(
                    self.context,
                    &mut self.encoder,
                    &native,
                    format,
                    filterable,
                    subresources.base_mip,
                    subresources.mip_count,
                    subresources.base_layer,
                    subresources.layer_count,
                )?;
            }
            CommandOp::HostWriteBuffer {
                buffer,
                offset,
                data,
            } => self.host_write(*buffer, *offset, data)?,
            CommandOp::HostReadBuffer {
                buffer,
                offset,
                size,
            } => self.host_reads.push((*buffer, *offset, *size)),
            CommandOp::Present { .. } => {
                return Err(GalError::unsupported_feature(
                    "presentation commands are lowered by the frame coordinator",
                ))
            }
        }
        Ok(())
    }

    fn render_pass(&mut self) -> GalResult<&mut wgpu::RenderPass<'static>> {
        self.render
            .as_mut()
            .ok_or_else(|| GalError::backend("draw recorded outside a render pass"))
    }

    fn surface_for(&self, frame_target: Handle) -> GalResult<&AcquiredSurface<'_>> {
        let target = self
            .objects
            .frame_targets
            .get(&frame_target)
            .ok_or_else(|| GalError::backend("unknown frame target"))?;
        self.surface
            .as_ref()
            .filter(|surface| surface.render_target == target.render_target)
            .ok_or_else(|| {
                GalError::backend("frame target was used without its acquired surface texture")
            })
    }

    fn attachment_view(&mut self, handle: Handle) -> GalResult<wgpu::TextureView> {
        if handle.kind() == Some(HandleKind::FrameTarget) {
            return Ok(self.surface_for(handle)?.view.clone());
        }
        Ok(self.objects.view_mut(handle)?.attachment_view())
    }

    fn begin_pass(
        &mut self,
        target: Handle,
        colors: &[crate::commands::PassAttachment],
        depth_stencil: Option<&crate::commands::PassAttachment>,
    ) -> GalResult<()> {
        self.end_compute();
        // Frame target passes attach the surface even if the list names the
        // target itself as its color view.
        let color_views = colors
            .iter()
            .map(|attachment| {
                let handle = if target.kind() == Some(HandleKind::FrameTarget)
                    && attachment.view.kind() != Some(HandleKind::TextureView)
                {
                    target
                } else {
                    attachment.view
                };
                self.attachment_view(handle)
            })
            .collect::<GalResult<Vec<_>>>()?;
        let depth = match depth_stencil {
            Some(attachment) => {
                let format = self.objects.view_mut(attachment.view)?.format;
                Some((self.attachment_view(attachment.view)?, format, attachment))
            }
            None => None,
        };
        let color_attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = colors
            .iter()
            .zip(&color_views)
            .map(|(attachment, view)| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: convert::color_load(attachment.load_op, attachment.clear_color),
                        store: convert::store(attachment.store_op),
                    },
                })
            })
            .collect();
        let depth_stencil_attachment = depth.as_ref().map(|(view, format, attachment)| {
            wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: match attachment.load_op {
                        AttachmentLoadOp::Load => wgpu::LoadOp::Load,
                        AttachmentLoadOp::Clear | AttachmentLoadOp::DontCare => {
                            wgpu::LoadOp::Clear(1.0)
                        }
                    },
                    store: convert::store(attachment.store_op),
                }),
                stencil_ops: format.has_stencil_aspect().then(|| wgpu::Operations {
                    load: match attachment.load_op {
                        AttachmentLoadOp::Load => wgpu::LoadOp::Load,
                        AttachmentLoadOp::Clear | AttachmentLoadOp::DontCare => {
                            wgpu::LoadOp::Clear(0)
                        }
                    },
                    store: convert::store(attachment.store_op),
                }),
            }
        });
        let pass = self
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &color_attachments,
                depth_stencil_attachment,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        self.render = Some(pass);
        self.graphics_pipeline = None;
        self.applied_graphics = None;
        self.graphics_sets = BindState::default();
        self.index_buffer = None;
        self.applied_index = None;
        Ok(())
    }

    /// Applies the bound graphics pipeline and sets; returns the
    /// pipeline's draw shape.
    fn flush_graphics(&mut self) -> GalResult<DrawShape> {
        let handle = self
            .graphics_pipeline
            .ok_or_else(|| GalError::backend("draw without a bound graphics pipeline"))?;
        let (shape, stencil_reference, key) = {
            let pipeline = self
                .objects
                .graphics_pipelines
                .get(&handle)
                .ok_or_else(|| GalError::backend("unknown graphics pipeline"))?;
            let key = self
                .graphics_sets
                .unfilterable_key(self.objects, pipeline.float_textures());
            let shape = DrawShape {
                topology: pipeline.topology,
                provoking_last: pipeline.provoking_last,
            };
            (shape, pipeline.stencil_reference, key)
        };
        let current = self
            .applied_graphics
            .as_ref()
            .filter(|(applied, applied_key, _)| *applied == handle && *applied_key == key)
            .map(|(_, _, variant)| variant.clone());
        let variant = match current {
            Some(variant) => variant,
            None => {
                let mut pipeline = self
                    .objects
                    .graphics_pipelines
                    .remove(&handle)
                    .expect("looked up above");
                let variant = self
                    .objects
                    .graphics_variant(self.context, &mut pipeline, key);
                self.objects.graphics_pipelines.insert(handle, pipeline);
                let variant = variant?;
                let pass = self.render_pass()?;
                pass.set_pipeline(&variant.pipeline);
                if let Some(reference) = stencil_reference {
                    pass.set_stencil_reference(reference);
                }
                self.applied_graphics = Some((handle, key, variant.clone()));
                variant
            }
        };
        let mut bind = std::mem::take(&mut self.graphics_sets);
        let result = self.apply_sets(&mut bind, &variant.groups, true);
        self.forget_rotations_written_by(&bind);
        self.graphics_sets = bind;
        result?;
        Ok(shape)
    }

    fn flush_indexed(&mut self) -> GalResult<()> {
        let shape = self.flush_graphics()?;
        if shape.topology == PrimitiveTopology::TriangleFan {
            return Err(GalError::unsupported_feature(
                "the wgpu backend draws non-indexed triangle fans only",
            ));
        }
        let (buffer, offset, format) = self
            .index_buffer
            .ok_or_else(|| GalError::backend("indexed draw without an index buffer"))?;
        let rotation = shape.rotation();
        let wanted = (buffer, offset, format, rotation);
        if self.applied_index != Some(wanted) {
            let native = match rotation {
                None => self.objects.buffer(buffer)?.buffer.clone(),
                Some(rotation) => self
                    .indices
                    .rotated
                    .get(&(buffer, format, rotation))
                    .cloned()
                    .ok_or_else(|| {
                        GalError::backend(
                            "last-vertex index buffer was written inside the pass that draws with it",
                        )
                    })?,
            };
            self.render_pass()?
                .set_index_buffer(native.slice(offset..), format);
            self.applied_index = Some(wanted);
        }
        Ok(())
    }

    /// Before a render pass begins, builds the rotated index buffers its
    /// last-vertex indexed draws read (compute cannot run inside the pass).
    fn prepare_rotated_indices(&mut self, rest: &[CommandOp]) -> GalResult<()> {
        let mut pipeline = None;
        let mut index = None;
        let mut wanted = Vec::new();
        for op in rest {
            match op {
                CommandOp::EndPass => break,
                CommandOp::BindGraphicsPipeline(handle) => pipeline = Some(*handle),
                CommandOp::SetIndexBuffer {
                    buffer, index_type, ..
                } => index = Some((*buffer, index_format(*index_type))),
                CommandOp::DrawIndexed { .. } | CommandOp::DrawIndexedIndirect { .. } => {
                    let (Some(pipeline), Some((buffer, format))) = (pipeline, index) else {
                        continue;
                    };
                    let rotation =
                        self.objects
                            .graphics_pipelines
                            .get(&pipeline)
                            .and_then(|pipeline| {
                                DrawShape {
                                    topology: pipeline.topology,
                                    provoking_last: pipeline.provoking_last,
                                }
                                .rotation()
                            });
                    if let Some(rotation) = rotation {
                        if !wanted.contains(&(buffer, format, rotation)) {
                            wanted.push((buffer, format, rotation));
                        }
                    }
                }
                _ => {}
            }
        }
        for (buffer, format, rotation) in wanted {
            if self
                .indices
                .rotated
                .contains_key(&(buffer, format, rotation))
            {
                continue;
            }
            let source = self.objects.buffer(buffer)?.buffer.clone();
            let rotated = self.context.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vulkanic.last-vertex-indices"),
                size: source.size(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDEX,
                mapped_at_creation: false,
            });
            let element_bytes = match format {
                wgpu::IndexFormat::Uint16 => 2,
                wgpu::IndexFormat::Uint32 => 4,
            };
            let plan = Utility::plan_index_rotation(
                self.context,
                &source,
                element_bytes,
                rotation == Rotation::Lines,
            )?;
            let params = self.uniform_params(&plan.param_bytes());
            self.utility.record_index_rotation(
                self.context,
                &mut self.encoder,
                &source,
                &rotated,
                &plan,
                &params,
            )?;
            self.indices
                .rotated
                .insert((buffer, format, rotation), rotated);
        }
        Ok(())
    }

    /// Drops rotated copies of index buffers the bound sets may write.
    fn forget_rotations_written_by(&mut self, bind: &BindState) {
        if self.indices.rotated.is_empty() {
            return;
        }
        for bound in bind.sets.iter().flatten() {
            if let Some(set) = self.objects.sets.get(&bound.set) {
                for buffer in set.written_buffers() {
                    self.indices.forget(buffer);
                }
            }
        }
    }

    fn flush_compute(&mut self) -> GalResult<()> {
        let handle = self
            .compute_pipeline
            .ok_or_else(|| GalError::backend("dispatch without a bound compute pipeline"))?;
        if self.compute.is_none() {
            self.compute = Some(
                self.encoder
                    .begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: None,
                        timestamp_writes: None,
                    })
                    .forget_lifetime(),
            );
            self.applied_compute = None;
            self.compute_sets.applied.fill(None);
        }
        let key = {
            let pipeline = self
                .objects
                .compute_pipelines
                .get(&handle)
                .ok_or_else(|| GalError::backend("unknown compute pipeline"))?;
            self.compute_sets
                .unfilterable_key(self.objects, pipeline.float_textures())
        };
        let current = self
            .applied_compute
            .as_ref()
            .filter(|(applied, applied_key, _)| *applied == handle && *applied_key == key)
            .map(|(_, _, variant)| variant.clone());
        let variant = match current {
            Some(variant) => variant,
            None => {
                let mut pipeline = self
                    .objects
                    .compute_pipelines
                    .remove(&handle)
                    .expect("looked up above");
                let variant = self
                    .objects
                    .compute_variant(self.context, &mut pipeline, key);
                self.objects.compute_pipelines.insert(handle, pipeline);
                let variant = variant?;
                self.compute
                    .as_mut()
                    .expect("opened above")
                    .set_pipeline(&variant.pipeline);
                self.applied_compute = Some((handle, key, variant.clone()));
                variant
            }
        };
        let mut bind = std::mem::take(&mut self.compute_sets);
        let result = self.apply_sets(&mut bind, &variant.groups, false);
        self.forget_rotations_written_by(&bind);
        self.compute_sets = bind;
        result
    }

    fn apply_sets(
        &mut self,
        bind: &mut BindState,
        groups: &[Arc<super::resources::BindGroupLayoutInfo>],
        render: bool,
    ) -> GalResult<()> {
        if bind.applied.len() < groups.len() {
            bind.applied.resize(groups.len(), None);
        }
        for (index, layout) in groups.iter().enumerate() {
            let bound = bind.sets.get(index).cloned().flatten();
            let (group, set, offsets) = match &bound {
                Some(bound) => {
                    let set = self
                        .objects
                        .sets
                        .get(&bound.set)
                        .ok_or_else(|| GalError::backend("unknown resource set"))?;
                    let offsets = set.dynamic_offsets(layout, &bound.offsets)?;
                    if bind.applied[index].as_ref().is_some_and(
                        |(id, applied_set, applied_offsets)| {
                            *id == layout.id
                                && *applied_set == Some(bound.set)
                                && *applied_offsets == offsets
                        },
                    ) {
                        continue;
                    }
                    (
                        self.objects.bind_group(self.context, bound.set, layout)?,
                        Some(bound.set),
                        offsets,
                    )
                }
                None => {
                    if bind.applied[index]
                        .as_ref()
                        .is_some_and(|(id, applied_set, _)| {
                            *id == layout.id && applied_set.is_none()
                        })
                    {
                        continue;
                    }
                    match self.objects.empty_bind_group(self.context, layout)? {
                        Some(group) => (group, None, Vec::new()),
                        None => {
                            return Err(GalError::invalid_argument(format!(
                                "the bound pipeline reads resource set {index}, but no set is bound there"
                            )))
                        }
                    }
                }
            };
            if render {
                self.render_pass()?
                    .set_bind_group(index as u32, &group, &offsets);
            } else {
                self.compute
                    .as_mut()
                    .expect("compute pass is open")
                    .set_bind_group(index as u32, &group, &offsets);
            }
            bind.applied[index] = Some((layout.id, set, offsets));
        }
        Ok(())
    }

    fn pattern_buffer(&mut self, pattern: IndexPattern, vertices: u32) -> GalResult<wgpu::Buffer> {
        if let Some((buffer, capacity)) = self.indices.patterns.get(&pattern) {
            if *capacity >= vertices {
                return Ok(buffer.clone());
            }
        }
        // Lists for fewer vertices are prefixes of longer ones.
        let capacity = vertices.next_power_of_two().max(64);
        let indices: Vec<u8> = pattern
            .indices(capacity)
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let buffer = self.context.scoped("index pattern buffer creation", || {
            self.context.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vulkanic.generated-indices"),
                size: indices.len() as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })?;
        // Contents never change, so writing ahead of the submission is safe.
        self.context.queue.write_buffer(&buffer, 0, &indices);
        self.indices
            .patterns
            .insert(pattern, (buffer.clone(), capacity));
        Ok(buffer)
    }

    /// A transient storage buffer holding `bytes`, staged in order.
    fn staged_storage(&mut self, bytes: &[u8]) -> GalResult<wgpu::Buffer> {
        let size = (bytes.len() as u64).next_multiple_of(4).max(4);
        let buffer = self.context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vulkanic.staged-bytes"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut padded = bytes.to_vec();
        padded.resize(size as usize, 0);
        let mut view = self.belt.write_buffer(
            &mut self.encoder,
            &buffer,
            0,
            std::num::NonZeroU64::new(size).expect("non-zero"),
        );
        view.copy_from_slice(&padded);
        drop(view);
        Ok(buffer)
    }

    fn uniform_params(&mut self, bytes: &[u8]) -> wgpu::Buffer {
        let size = (bytes.len() as u64).next_multiple_of(16);
        let buffer = self.context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vulkanic.byte-copy-params"),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut padded = bytes.to_vec();
        padded.resize(size as usize, 0);
        let mut view = self.belt.write_buffer(
            &mut self.encoder,
            &buffer,
            0,
            std::num::NonZeroU64::new(size).expect("non-zero"),
        );
        view.copy_from_slice(&padded);
        drop(view);
        buffer
    }

    fn byte_copy(
        &mut self,
        src: &wgpu::Buffer,
        dst: &wgpu::Buffer,
        copy: ByteCopy,
    ) -> GalResult<()> {
        self.end_compute();
        let Some(plan) = Utility::plan_byte_copy(self.context, src, dst, copy)? else {
            return Ok(());
        };
        let params = self.uniform_params(&plan.param_bytes());
        self.utility
            .record_byte_copy(self.context, &mut self.encoder, src, dst, &plan, &params)
    }

    fn host_write(&mut self, buffer: Handle, offset: u64, data: &[u8]) -> GalResult<()> {
        self.end_compute();
        self.indices.forget(buffer);
        let target = self.objects.buffer(buffer)?.buffer.clone();
        let size = data.len() as u64;
        if offset.is_multiple_of(4) && size.is_multiple_of(4) {
            let mut view = self.belt.write_buffer(
                &mut self.encoder,
                &target,
                offset,
                std::num::NonZeroU64::new(size).expect("GAL rejects empty host writes"),
            );
            view.copy_from_slice(data);
            return Ok(());
        }
        let staged = self.staged_storage(data)?;
        self.byte_copy(&staged, &target, ByteCopy::linear(0, offset, size))
    }

    fn copy_buffer(
        &mut self,
        src: Handle,
        src_offset: u64,
        dst: Handle,
        dst_offset: u64,
        size: u64,
    ) -> GalResult<()> {
        self.end_compute();
        self.indices.forget(dst);
        let source = self.objects.buffer(src)?.buffer.clone();
        let target = self.objects.buffer(dst)?.buffer.clone();
        let aligned =
            src_offset.is_multiple_of(4) && dst_offset.is_multiple_of(4) && size.is_multiple_of(4);
        if src != dst && aligned {
            self.encoder
                .copy_buffer_to_buffer(&source, src_offset, &target, dst_offset, size);
            return Ok(());
        }
        // Through a scratch copy: same-buffer copies conflict in one pass,
        // and unaligned ones need the byte copy.
        let window_start = src_offset / 4 * 4;
        let window_size = (src_offset + size).next_multiple_of(4) - window_start;
        let scratch = self.context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vulkanic.copy-scratch"),
            size: window_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        self.encoder
            .copy_buffer_to_buffer(&source, window_start, &scratch, 0, window_size);
        if aligned {
            self.encoder.copy_buffer_to_buffer(
                &scratch,
                src_offset - window_start,
                &target,
                dst_offset,
                size,
            );
            return Ok(());
        }
        self.byte_copy(
            &scratch,
            &target,
            ByteCopy::linear(src_offset - window_start, dst_offset, size),
        )
    }

    fn texel_size(&self, texture: Handle) -> GalResult<(wgpu::Texture, u64, TextureDimension)> {
        let object = self.objects.texture(texture)?;
        if object.desc.format == TextureFormat::Rgb16Float {
            return Err(GalError::unsupported_feature(
                "the wgpu backend stores Rgb16Float as Rgba16Float and cannot copy it to or from buffers",
            ));
        }
        let texel = object.format.block_copy_size(None).ok_or_else(|| {
            GalError::unsupported_feature("texture format has no buffer copy layout")
        })?;
        Ok((
            object.texture.clone(),
            u64::from(texel),
            object.desc.dimension,
        ))
    }

    fn image_copy<'t>(
        texture: &'t wgpu::Texture,
        dimension: TextureDimension,
        mip: u32,
        layer: u32,
        origin: crate::commands::TextureOrigin3d,
    ) -> wgpu::TexelCopyTextureInfo<'t> {
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin: wgpu::Origin3d {
                x: origin.x,
                y: origin.y,
                z: if dimension == TextureDimension::D3 {
                    origin.z
                } else {
                    layer
                },
            },
            aspect: wgpu::TextureAspect::All,
        }
    }

    fn copy_buffer_to_texture(&mut self, region: &BufferImageCopyRegion) -> GalResult<()> {
        self.end_compute();
        let (texture, texel, dimension) = self.texel_size(region.texture)?;
        let buffer = self.objects.buffer(region.buffer)?.buffer.clone();
        let extent = wgpu::Extent3d {
            width: region.extent.width,
            height: region.extent.height,
            depth_or_array_layers: region.extent.depth,
        };
        let destination = Self::image_copy(
            &texture,
            dimension,
            region.texture_mip,
            region.texture_layer,
            region.texture_origin,
        );
        let row_bytes = u64::from(region.extent.width) * texel;
        let bytes_per_row = u64::from(region.bytes_per_row);
        if bytes_per_row % 256 == 0 && region.buffer_offset.is_multiple_of(texel) {
            self.encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: region.buffer_offset,
                        bytes_per_row: Some(region.bytes_per_row),
                        rows_per_image: Some(region.rows_per_image),
                    },
                },
                destination,
                extent,
            );
            return Ok(());
        }
        // Repack the tightly packed rows into 256-byte rows first.
        let aligned_row = row_bytes.next_multiple_of(256);
        let rows = u64::from(region.extent.height);
        let images = u64::from(region.extent.depth.max(1));
        let scratch = self.context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vulkanic.upload-repack"),
            size: aligned_row * rows * images,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        self.byte_copy(
            &buffer,
            &scratch,
            ByteCopy {
                src_offset: region.buffer_offset,
                src_row_stride: bytes_per_row,
                src_image_stride: bytes_per_row * u64::from(region.rows_per_image),
                dst_offset: 0,
                dst_row_stride: aligned_row,
                dst_image_stride: aligned_row * rows,
                row_bytes,
                rows,
                images,
            },
        )?;
        let destination = Self::image_copy(
            &texture,
            dimension,
            region.texture_mip,
            region.texture_layer,
            region.texture_origin,
        );
        self.encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &scratch,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(convert::checked_u32(aligned_row, "texture row")?),
                    rows_per_image: Some(region.extent.height),
                },
            },
            destination,
            extent,
        );
        Ok(())
    }

    fn copy_texture_to_buffer(&mut self, region: &BufferImageCopyRegion) -> GalResult<()> {
        self.end_compute();
        self.indices.forget(region.buffer);
        let (texture, texel, dimension) = self.texel_size(region.texture)?;
        let buffer = self.objects.buffer(region.buffer)?.buffer.clone();
        let extent = wgpu::Extent3d {
            width: region.extent.width,
            height: region.extent.height,
            depth_or_array_layers: region.extent.depth,
        };
        let source = Self::image_copy(
            &texture,
            dimension,
            region.texture_mip,
            region.texture_layer,
            region.texture_origin,
        );
        let row_bytes = u64::from(region.extent.width) * texel;
        let bytes_per_row = u64::from(region.bytes_per_row);
        if bytes_per_row % 256 == 0 && region.buffer_offset.is_multiple_of(texel) {
            self.encoder.copy_texture_to_buffer(
                source,
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: region.buffer_offset,
                        bytes_per_row: Some(region.bytes_per_row),
                        rows_per_image: Some(region.rows_per_image),
                    },
                },
                extent,
            );
            return Ok(());
        }
        let aligned_row = row_bytes.next_multiple_of(256);
        let rows = u64::from(region.extent.height);
        let images = u64::from(region.extent.depth.max(1));
        let scratch = self.context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vulkanic.readback-repack"),
            size: aligned_row * rows * images,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.encoder.copy_texture_to_buffer(
            source,
            wgpu::TexelCopyBufferInfo {
                buffer: &scratch,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(convert::checked_u32(aligned_row, "texture row")?),
                    rows_per_image: Some(region.extent.height),
                },
            },
            extent,
        );
        self.byte_copy(
            &scratch,
            &buffer,
            ByteCopy {
                src_offset: 0,
                src_row_stride: aligned_row,
                src_image_stride: aligned_row * rows,
                dst_offset: region.buffer_offset,
                dst_row_stride: bytes_per_row,
                dst_image_stride: bytes_per_row * u64::from(region.rows_per_image),
                row_bytes,
                rows,
                images,
            },
        )
    }

    fn copy_texture(&mut self, region: &TextureImageCopyRegion) -> GalResult<()> {
        self.end_compute();
        let source = self.objects.texture(region.src_texture)?;
        let (source_texture, source_dimension) = (source.texture.clone(), source.desc.dimension);
        let target = self.objects.texture(region.dst_texture)?;
        let (target_texture, target_dimension) = (target.texture.clone(), target.desc.dimension);
        match region.row_order {
            TextureRowOrder::Preserve => {
                self.encoder.copy_texture_to_texture(
                    Self::image_copy(
                        &source_texture,
                        source_dimension,
                        region.src_mip,
                        region.src_layer,
                        region.src_origin,
                    ),
                    Self::image_copy(
                        &target_texture,
                        target_dimension,
                        region.dst_mip,
                        region.dst_layer,
                        region.dst_origin,
                    ),
                    wgpu::Extent3d {
                        width: region.extent.width,
                        height: region.extent.height,
                        depth_or_array_layers: region.extent.depth,
                    },
                );
            }
            TextureRowOrder::Reverse => {
                // wgpu has no flipped copy; copy one row at a time.
                for row in 0..region.extent.height {
                    let mut src_origin = region.src_origin;
                    src_origin.y += row;
                    let mut dst_origin = region.dst_origin;
                    dst_origin.y += region.extent.height - 1 - row;
                    self.encoder.copy_texture_to_texture(
                        Self::image_copy(
                            &source_texture,
                            source_dimension,
                            region.src_mip,
                            region.src_layer,
                            src_origin,
                        ),
                        Self::image_copy(
                            &target_texture,
                            target_dimension,
                            region.dst_mip,
                            region.dst_layer,
                            dst_origin,
                        ),
                        wgpu::Extent3d {
                            width: region.extent.width,
                            height: 1,
                            depth_or_array_layers: region.extent.depth,
                        },
                    );
                }
            }
        }
        Ok(())
    }

    fn finish_host_reads(&mut self) -> GalResult<Vec<PendingRead>> {
        let mut reads = Vec::with_capacity(self.host_reads.len());
        for (buffer, offset, size) in std::mem::take(&mut self.host_reads) {
            let source = self.objects.buffer(buffer)?.buffer.clone();
            let start = offset / 4 * 4;
            let end = (offset + size).next_multiple_of(4).min(source.size());
            let staging = self.context.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("vulkanic.host-read"),
                size: end - start,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.encoder
                .copy_buffer_to_buffer(&source, start, &staging, 0, end - start);
            reads.push(PendingRead {
                buffer,
                offset,
                size,
                staging,
                staging_offset: offset - start,
            });
        }
        Ok(reads)
    }
}
