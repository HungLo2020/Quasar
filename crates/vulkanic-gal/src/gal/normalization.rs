//! Command normalization: removing redundant binds and fusing identical adjacent passes.

use super::*;
use crate::commands::DynamicOffsets;

#[derive(Default)]
pub(crate) struct CommandNormalizationStats {
    pub(crate) ops_before: u64,
    pub(crate) ops_after: u64,
    pub(crate) pipeline_binds_removed: u64,
    pub(crate) resource_set_binds_removed: u64,
    pub(crate) vertex_buffer_binds_removed: u64,
    pub(crate) index_buffer_binds_removed: u64,
}

/// The binds a list has made since the last state boundary.
#[derive(Default)]
struct CommandStateTracker {
    graphics_pipeline: Option<Handle>,
    compute_pipeline: Option<Handle>,
    pipeline_layout: Option<Handle>,
    /// By set index: the layout it was bound against, the set, offsets.
    resource_sets: Vec<Option<(Handle, Handle, DynamicOffsets)>>,
    /// By slot: buffer and offset.
    vertex_buffers: Vec<Option<(Handle, u64)>>,
    index_buffer: Option<(Handle, u64, IndexType)>,
}

impl CommandStateTracker {
    fn invalidate(&mut self) {
        self.graphics_pipeline = None;
        self.compute_pipeline = None;
        self.pipeline_layout = None;
        self.resource_sets.clear();
        self.vertex_buffers.clear();
        self.index_buffer = None;
    }

    /// Records a pipeline switch to one with `layout`. Sets are bound
    /// against a layout: if the new one is unknown or different, every set
    /// is replayed (this keeps a four-binding set from being reused by a
    /// two-binding pipeline).
    fn switch_layout(&mut self, layout: Option<Handle>) {
        if layout.is_none() || self.pipeline_layout != layout {
            self.resource_sets.clear();
        }
        self.pipeline_layout = layout;
    }
}

/// Whether a pass begun with `begin` keeps its attachments' contents, so a
/// following identical pass can continue it.
fn preserves_contents(begin: &CommandOp) -> bool {
    match begin {
        CommandOp::BeginPass {
            colors,
            depth_stencil,
            ..
        } => colors.iter().chain(depth_stencil.iter()).all(|attachment| {
            attachment.load_op == AttachmentLoadOp::Load
                && attachment.store_op == AttachmentStoreOp::Store
        }),
        _ => false,
    }
}

/// What normalization takes out of a batch besides redundant binds.
#[derive(Default)]
pub(crate) struct NormalizedExtracts {
    /// The submission receipts, removed from the lists.
    pub(crate) usages: Vec<crate::commands::SubmissionUsage>,
    /// Every handle the kept ops name, with repeats.
    pub(crate) handles: Vec<Handle>,
}

#[cfg(test)]
pub(crate) fn normalize_submission_batch(batch: &mut SubmissionBatch) -> CommandNormalizationStats {
    normalize_submission_batch_with(
        batch,
        &mut NormalizedExtracts::default(),
        |_| None,
        |_| None,
    )
}

#[cfg(test)]
pub(crate) fn normalize_submission_batch_with_pipeline_layouts(
    batch: &mut SubmissionBatch,
    graphics_pipeline_layouts: &BTreeMap<Handle, Handle>,
    compute_pipeline_layouts: &BTreeMap<Handle, Handle>,
) -> CommandNormalizationStats {
    normalize_submission_batch_with(
        batch,
        &mut NormalizedExtracts::default(),
        |pipeline| graphics_pipeline_layouts.get(&pipeline).copied(),
        |pipeline| compute_pipeline_layouts.get(&pipeline).copied(),
    )
}

/// Removes redundant binds and submission receipts and fuses adjacent
/// identical passes, compacting each list in place; `extracts` receives the
/// receipts and the handles the kept ops name. `graphics_layout` and
/// `compute_layout` name a pipeline's layout (`None` when unknown, which
/// never keeps sets bound across the switch).
pub(crate) fn normalize_submission_batch_with(
    batch: &mut SubmissionBatch,
    extracts: &mut NormalizedExtracts,
    graphics_layout: impl Fn(Handle) -> Option<Handle>,
    compute_layout: impl Fn(Handle) -> Option<Handle>,
) -> CommandNormalizationStats {
    let mut stats = CommandNormalizationStats::default();
    let mut state = CommandStateTracker::default();
    for list in &mut batch.command_lists {
        let ops = &mut list.operations;
        stats.ops_before = stats.ops_before.saturating_add(ops.len() as u64);
        state.invalidate();
        // Kept ops are moved down to `write`; `active_begin` is the kept
        // BeginPass of the open pass.
        let mut write = 0;
        let mut active_begin: Option<usize> = None;
        let mut read = 0;
        while read < ops.len() {
            let keep = match &ops[read] {
                CommandOp::TrackSubmission(usage) => {
                    extracts.usages.push(usage.clone());
                    false
                }
                CommandOp::BeginPass { .. } => {
                    state.invalidate();
                    active_begin = Some(write);
                    true
                }
                CommandOp::EndPass => {
                    let continues = active_begin.is_some_and(|begin| {
                        preserves_contents(&ops[begin])
                            && ops.get(read + 1).is_some_and(|next| *next == ops[begin])
                    });
                    if continues {
                        // The next pass has the same target, attachments and
                        // load/store contract: keep this one open so ordered
                        // draws do not get a pass boundary. Barriers, uploads
                        // and receipts between passes prevent the adjacency.
                        read += 2;
                        continue;
                    }
                    active_begin = None;
                    state.invalidate();
                    true
                }
                CommandOp::Barrier(_) => {
                    state.invalidate();
                    true
                }
                CommandOp::BindGraphicsPipeline(handle) => {
                    if state.graphics_pipeline == Some(*handle) {
                        stats.pipeline_binds_removed += 1;
                        false
                    } else {
                        state.graphics_pipeline = Some(*handle);
                        state.compute_pipeline = None;
                        state.switch_layout(graphics_layout(*handle));
                        true
                    }
                }
                CommandOp::BindComputePipeline(handle) => {
                    if state.compute_pipeline == Some(*handle) {
                        stats.pipeline_binds_removed += 1;
                        false
                    } else {
                        state.compute_pipeline = Some(*handle);
                        state.graphics_pipeline = None;
                        state.switch_layout(compute_layout(*handle));
                        true
                    }
                }
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set_index,
                    set,
                    dynamic_offsets,
                } => {
                    let index = *set_index as usize;
                    if state.resource_sets.len() <= index {
                        state.resource_sets.resize_with(index + 1, || None);
                    }
                    match &mut state.resource_sets[index] {
                        Some((layout, bound, offsets))
                            if layout == pipeline_layout
                                && bound == set
                                && offsets == dynamic_offsets =>
                        {
                            stats.resource_set_binds_removed += 1;
                            false
                        }
                        Some((layout, bound, offsets)) => {
                            *layout = *pipeline_layout;
                            *bound = *set;
                            offsets.set_from_slice(dynamic_offsets);
                            true
                        }
                        empty => {
                            *empty = Some((*pipeline_layout, *set, dynamic_offsets.clone()));
                            true
                        }
                    }
                }
                CommandOp::SetVertexBuffer {
                    slot,
                    buffer,
                    offset,
                } => {
                    let index = *slot as usize;
                    if state.vertex_buffers.len() <= index {
                        state.vertex_buffers.resize(index + 1, None);
                    }
                    let value = Some((*buffer, *offset));
                    if state.vertex_buffers[index] == value {
                        stats.vertex_buffer_binds_removed += 1;
                        false
                    } else {
                        state.vertex_buffers[index] = value;
                        true
                    }
                }
                CommandOp::SetIndexBuffer {
                    buffer,
                    offset,
                    index_type,
                } => {
                    let value = Some((*buffer, *offset, *index_type));
                    if state.index_buffer == value {
                        stats.index_buffer_binds_removed += 1;
                        false
                    } else {
                        state.index_buffer = value;
                        true
                    }
                }
                CommandOp::CopyBuffer { .. }
                | CommandOp::CopyBufferRegion { .. }
                | CommandOp::CopyBufferToTexture(_)
                | CommandOp::CopyTextureToBuffer(_)
                | CommandOp::CopyTexture(_)
                | CommandOp::CopyFrameTargetToTexture { .. }
                | CommandOp::CopyTextureToFrameTarget { .. }
                | CommandOp::GenerateMipmaps { .. }
                | CommandOp::HostWriteBuffer { .. }
                | CommandOp::HostReadBuffer { .. }
                | CommandOp::Present { .. } => {
                    state.invalidate();
                    true
                }
                CommandOp::Draw { .. }
                | CommandOp::DrawIndexed { .. }
                | CommandOp::DrawIndirect { .. }
                | CommandOp::DrawIndexedIndirect { .. }
                | CommandOp::Dispatch { .. }
                | CommandOp::DispatchIndirect { .. } => true,
            };
            if keep {
                super::submission::op_handles(&ops[read], |handle| extracts.handles.push(handle));
                ops.swap(write, read);
                write += 1;
            }
            read += 1;
        }
        ops.truncate(write);
        stats.ops_after = stats.ops_after.saturating_add(ops.len() as u64);
    }
    stats
}
