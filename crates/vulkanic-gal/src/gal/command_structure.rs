//! The command-list checks that always run, with full validation off.
//!
//! Full validation (`command_validation`) looks every handle up and checks
//! ranges, formats and compatibility; release builds skip it per frame (see
//! `per_frame_validation`). Backends still rely on a list's shape: passes
//! open and close in order, draws happen inside a pass with a graphics
//! pipeline bound, and dispatches and copies outside one. This walk checks
//! only that shape, from op discriminants, so it costs a few nanoseconds per
//! op.

use super::*;

pub(super) fn validate_command_structure(label: &str, ops: &[CommandOp]) -> GalResult<()> {
    let mut in_pass = false;
    let mut graphics = false;
    let mut compute = false;
    let mut pipeline = false;
    let fail = |index: usize, message: &str| -> GalResult<()> {
        Err(GalError::command(
            StatusCode::InvalidArgument,
            format!("command list '{label}' op {index}: {message}"),
        ))
    };
    for (index, op) in ops.iter().enumerate() {
        match op {
            CommandOp::BeginPass { .. } => {
                if in_pass {
                    return fail(index, "nested passes are invalid");
                }
                in_pass = true;
            }
            CommandOp::EndPass => {
                if !in_pass {
                    return fail(index, "EndPass without BeginPass");
                }
                in_pass = false;
                graphics = false;
                pipeline = false;
            }
            CommandOp::BindGraphicsPipeline(_) => {
                if !in_pass {
                    return fail(
                        index,
                        "graphics pipeline must be bound inside a render pass",
                    );
                }
                graphics = true;
                compute = false;
                pipeline = true;
            }
            CommandOp::BindComputePipeline(_) => {
                if in_pass {
                    return fail(
                        index,
                        "compute pipeline cannot be bound inside a render pass",
                    );
                }
                compute = true;
                graphics = false;
                pipeline = true;
            }
            CommandOp::BindResourceSet { .. } => {
                if !pipeline {
                    return fail(index, "resource set bound without a pipeline");
                }
            }
            CommandOp::Draw { .. }
            | CommandOp::DrawIndexed { .. }
            | CommandOp::DrawIndirect { .. }
            | CommandOp::DrawIndexedIndirect { .. } => {
                if !in_pass || !graphics {
                    return fail(index, "draw requires an active pass and graphics pipeline");
                }
            }
            CommandOp::Dispatch { .. } | CommandOp::DispatchIndirect { .. } => {
                if in_pass || !compute {
                    return fail(index, "dispatch requires a compute pipeline outside passes");
                }
            }
            CommandOp::Barrier(_)
            | CommandOp::CopyBuffer { .. }
            | CommandOp::CopyBufferRegion { .. }
            | CommandOp::CopyBufferToTexture(_)
            | CommandOp::CopyTextureToBuffer(_)
            | CommandOp::CopyTexture(_)
            | CommandOp::CopyFrameTargetToTexture { .. }
            | CommandOp::CopyTextureToFrameTarget { .. }
            | CommandOp::GenerateMipmaps { .. } => {
                if in_pass {
                    return fail(
                        index,
                        "barriers and copies are invalid inside a render pass",
                    );
                }
            }
            CommandOp::SetVertexBuffer { .. }
            | CommandOp::SetIndexBuffer { .. }
            | CommandOp::HostWriteBuffer { .. }
            | CommandOp::HostReadBuffer { .. }
            | CommandOp::Present { .. }
            | CommandOp::TrackSubmission(_) => {}
        }
    }
    if in_pass {
        return fail(ops.len(), "command list ended inside a pass");
    }
    Ok(())
}
