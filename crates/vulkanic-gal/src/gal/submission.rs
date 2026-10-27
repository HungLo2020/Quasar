//! Submitting command batches, tracking in-flight resources, completion and retirement.

use super::*;

/// Whole-frame submission tracing (`VULKANIC_TRACE_WHOLE_FRAME`, read once).
fn submission_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| crate::env::var_os("VULKANIC_TRACE_WHOLE_FRAME").is_some())
}

/// Traces a submission stage; the message is built only when tracing is on.
pub(super) fn submission_trace(message: impl FnOnce() -> String) {
    if submission_trace_enabled() {
        crate::diagnostics::stderr(format_args!("{}", message()));
    }
}

/// A stage timer that reads the clock only when someone looks at it.
#[derive(Clone, Copy)]
struct StageTimer(Option<std::time::Instant>);

impl StageTimer {
    fn start(timed: bool) -> Self {
        Self(timed.then(std::time::Instant::now))
    }

    fn nanos(self) -> u64 {
        self.0.map_or(0, elapsed_nanos_u64)
    }
}

impl VulkanicGal {
    /// Validates a batch of command lists (ops, handles, hazards: overlapping
    /// conflicting accesses need an explicit barrier), removes redundant
    /// state changes, and submits it. The returned token names the submission.
    pub fn submit(&mut self, batch: SubmissionBatch) -> GalResult<SyncToken> {
        self.submit_inner(batch, None)
    }

    /// `submit`, also accumulating what the GAL measured into `profile`.
    pub fn submit_profiled(
        &mut self,
        batch: SubmissionBatch,
        profile: &mut SubmitProfile,
    ) -> GalResult<SyncToken> {
        self.submit_inner(batch, Some(profile))
    }

    pub(super) fn submit_inner(
        &mut self,
        mut batch: SubmissionBatch,
        mut profile: Option<&mut SubmitProfile>,
    ) -> GalResult<SyncToken> {
        // Timers and backend metric snapshots only for profiling or tracing.
        let timed = profile.is_some() || submission_trace_enabled();
        let submit_started = StageTimer::start(timed);
        let creates_before = self.metrics.resource_creates;
        let destroys_before = self.metrics.resource_destroys;
        let backend_metrics_before = profile.is_some().then(|| self.backend.runtime_metrics());
        if batch.command_lists.is_empty() {
            return self.validation_error(GalError::submission(
                StatusCode::InvalidArgument,
                "submission batch must contain command lists",
            ));
        }
        let capabilities = self.capabilities();
        if batch.command_lists.len() > capabilities.limits.max_command_lists_per_submission as usize
        {
            return self.unsupported(format!(
                "submission '{}' command list count {} exceeds backend '{}' limit {}",
                batch.label,
                batch.command_lists.len(),
                capabilities.name,
                capabilities.limits.max_command_lists_per_submission
            ));
        }
        // Command, handle and hazard checks guard against producer bugs. Normal
        // play checks only list structure (see `full_validation`); upload
        // capture needs the hazard walk to observe watched writes.
        let validate = self.full_validation || self.buffer_upload_capture.is_watching();
        for list in &batch.command_lists {
            let validate_ops_started = StageTimer::start(timed);
            if !validate {
                command_structure::validate_command_structure(&list.label, &list.operations)
                    .or_else(|error| self.validation_error(error))?;
                if let Some(profile) = profile.as_deref_mut() {
                    profile.gal_validate_ops_nanos = profile
                        .gal_validate_ops_nanos
                        .saturating_add(validate_ops_started.nanos());
                }
                continue;
            }
            let validate_ops_started = StageTimer::start(timed);
            self.validate_command_ops(&list.label, &list.operations)?;
            if let Some(profile) = profile.as_deref_mut() {
                profile.gal_validate_ops_nanos = profile
                    .gal_validate_ops_nanos
                    .saturating_add(validate_ops_started.nanos());
            }
        }
        // Normalization keeps sets bound across a pipeline switch only when
        // both pipelines name the same layout; an unknown handle counts as
        // incompatible.
        let graphics_pipelines = &self.graphics_pipelines;
        let compute_pipelines = &self.compute_pipelines;
        // Normalization also strips submission receipts (CPU reservation
        // bookkeeping backends never see; they stay alive until accepted)
        // and collects the handles the kept ops name.
        let mut extracted = NormalizedExtracts::default();
        let normalization = normalize_submission_batch_with(
            &mut batch,
            &mut extracted,
            |pipeline| {
                graphics_pipelines
                    .get(pipeline)
                    .ok()
                    .map(|record| record.desc.layout)
            },
            |pipeline| {
                compute_pipelines
                    .get(pipeline)
                    .ok()
                    .map(|record| record.desc.layout)
            },
        );
        if let Some(profile) = profile.as_deref_mut() {
            profile.gal_command_ops_before_normalize = profile
                .gal_command_ops_before_normalize
                .saturating_add(normalization.ops_before);
            profile.gal_command_ops_after_normalize = profile
                .gal_command_ops_after_normalize
                .saturating_add(normalization.ops_after);
            profile.gal_redundant_pipeline_binds_removed = profile
                .gal_redundant_pipeline_binds_removed
                .saturating_add(normalization.pipeline_binds_removed);
            profile.gal_redundant_resource_set_binds_removed = profile
                .gal_redundant_resource_set_binds_removed
                .saturating_add(normalization.resource_set_binds_removed);
            profile.gal_redundant_vertex_buffer_binds_removed = profile
                .gal_redundant_vertex_buffer_binds_removed
                .saturating_add(normalization.vertex_buffer_binds_removed);
            profile.gal_redundant_index_buffer_binds_removed = profile
                .gal_redundant_index_buffer_binds_removed
                .saturating_add(normalization.index_buffer_binds_removed);
        }
        // Gather host buffer writes at the start of their lists so the GPU
        // does not drain between passes for each one.
        for list in &mut batch.command_lists {
            host_write_hoist::hoist_host_writes(list, |set, buffer| {
                self.resource_sets.get(set).map_or(true, |record| {
                    record
                        .desc
                        .bindings
                        .iter()
                        .any(|binding| binding.resource == buffer)
                })
            });
        }
        // Limits apply to what backends encode.
        for list in &batch.command_lists {
            if list.operations.len() > capabilities.limits.max_commands_per_list as usize {
                return self.unsupported(format!(
                    "command list '{}' operation count {} exceeds backend '{}' limit {}",
                    list.label,
                    list.operations.len(),
                    capabilities.name,
                    capabilities.limits.max_commands_per_list
                ));
            }
        }
        let NormalizedExtracts {
            usages: submission_usages,
            handles: referenced,
        } = extracted;
        let validate_handles_started = StageTimer::start(timed);
        if validate {
            for handle in &referenced {
                self.validate_any_resource(*handle)?;
            }
        }
        if let Some(profile) = profile.as_deref_mut() {
            profile.gal_validate_handles_nanos = profile
                .gal_validate_handles_nanos
                .saturating_add(validate_handles_started.nanos());
        }
        let hazards_started = StageTimer::start(timed);
        if validate {
            self.validate_submission_hazards(&batch, profile.as_deref_mut())?;
        }
        if let Some(profile) = profile.as_deref_mut() {
            profile.gal_hazard_analysis_nanos = profile
                .gal_hazard_analysis_nanos
                .saturating_add(hazards_started.nanos());
            add_command_profile(profile, &batch);
        }
        let validated = ValidatedSubmissionBatch::from(batch);
        let id = SubmissionId(self.next_submission);
        self.next_submission += 1;
        submission_trace(|| {
            format!(
                "gal.submit.encode.begin id={} label={} pre_encode_nanos={}",
                id.0,
                validated.label,
                submit_started.nanos()
            )
        });
        let backend_encode_started = StageTimer::start(timed);
        self.backend.encode_passes(&validated)?;
        submission_trace(|| {
            format!(
                "gal.submit.encode.end id={} elapsed_nanos={}",
                id.0,
                backend_encode_started.nanos()
            )
        });
        if let Some(profile) = profile.as_deref_mut() {
            profile.backend_encode_nanos = profile
                .backend_encode_nanos
                .saturating_add(backend_encode_started.nanos());
        }
        let backend_submit_started = StageTimer::start(timed);
        submission_trace(|| format!("gal.submit.queue.begin id={}", id.0));
        self.backend.submit(id, &validated)?;
        self.latest_accepted_submission = id;
        self.buffer_upload_capture.accept(id);
        for usage in &submission_usages {
            usage.accept(id);
        }
        submission_trace(|| {
            format!(
                "gal.submit.queue.end id={} elapsed_nanos={}",
                id.0,
                backend_submit_started.nanos()
            )
        });
        if let Some(profile) = profile.as_deref_mut() {
            profile.backend_submit_nanos = profile
                .backend_submit_nanos
                .saturating_add(backend_submit_started.nanos());
        }
        self.metrics.submissions += 1;
        for handle in referenced {
            self.mark_in_flight(handle, id)?;
        }
        if let Some(profile) = profile {
            profile.gal_submit_total_nanos = profile
                .gal_submit_total_nanos
                .saturating_add(submit_started.nanos());
            profile.resource_creates_delta =
                self.metrics.resource_creates.saturating_sub(creates_before);
            profile.resource_destroys_delta = self
                .metrics
                .resource_destroys
                .saturating_sub(destroys_before);
            if let Some(before) = backend_metrics_before {
                let after = self.backend.runtime_metrics();
                add_backend_metric_deltas(profile, before, after);
            }
        }
        Ok(SyncToken { submission: id })
    }

    /// Polls completion and destroys every deferred resource whose last
    /// submission has completed. Returns the handles destroyed.
    pub fn retire_completed(&mut self) -> GalResult<Vec<Handle>> {
        self.poll_completed();
        self.backend.retire(self.completed_submission)?;
        let mut retired = Vec::new();
        for entry in self.retirement.drain_completed(self.completed_submission) {
            if let Some(pending) = self.pending_destroys.remove(&entry.handle) {
                self.backend
                    .destroy(entry.handle, pending.kind, pending.token)?;
                self.metrics.resource_destroys += 1;
                retired.push(entry.handle);
            }
        }
        Ok(retired)
    }

    /// The newest submission the backend reports complete, without waiting.
    pub fn poll_completed(&mut self) -> SubmissionId {
        let completed = self.backend.completed_submission();
        if completed > self.completed_submission {
            self.completed_submission = completed;
        }
        self.completed_submission
    }

    /// The newest submission the GAL has accepted.
    pub fn latest_submission_id(&self) -> SubmissionId {
        self.latest_accepted_submission
    }

    /// The last submission that used a render pass, if any.
    pub fn render_pass_last_submission(&self, pass: Handle) -> GalResult<Option<SubmissionId>> {
        Ok(self.render_passes.get(pass)?.last_submission)
    }

    /// Exposes the id that the immediately following submission will receive.
    /// Frontends use this to protect transient stream ranges until that
    /// submission completes; taking this value must be followed by one submit.
    pub fn next_submission_id(&self) -> SubmissionId {
        SubmissionId(self.next_submission)
    }

    /// Waits for submissions up to `id` to complete, then destroys the
    /// deferred resources they retire. Fails for an id not yet submitted.
    pub fn retire_through(&mut self, id: SubmissionId) -> GalResult<Vec<Handle>> {
        if id > self.latest_submission_id() {
            return Err(GalError::invalid_argument(
                "cannot wait for an unsubmitted completion id",
            ));
        }
        self.backend.retire(id)?;
        if id > self.completed_submission {
            self.completed_submission = id;
        }
        let mut retired = Vec::new();
        for entry in self.retirement.drain_completed(self.completed_submission) {
            if let Some(pending) = self.pending_destroys.remove(&entry.handle) {
                self.backend
                    .destroy(entry.handle, pending.kind, pending.token)?;
                self.metrics.resource_destroys += 1;
                retired.push(entry.handle);
            }
        }
        Ok(retired)
    }

    /// Buffer bytes read back to the host by completed submissions.
    pub fn completed_host_reads(&self) -> Vec<CompletedHostRead> {
        self.backend.completed_host_reads()
    }

    /// Buffer bytes read back by completed submissions after `after`; a
    /// frame loop polling for new reads clones only those.
    pub fn completed_host_reads_since(&self, after: SubmissionId) -> Vec<CompletedHostRead> {
        self.backend.completed_host_reads_since(after)
    }

    /// Records `id` as the last submission using `handle` and everything it
    /// depends on. A command names resource sets and pipelines, while their
    /// dependency edges own the samplers, views, textures and shaders they
    /// contain; the whole closure retires with the submission (marking only
    /// the set let a replaced sampler be destroyed while its descriptor set
    /// still executed). A record already stamped with `id` has its closure
    /// marked, so shared dependencies are visited once.
    pub(super) fn mark_in_flight(&mut self, handle: Handle, id: SubmissionId) -> GalResult<()> {
        if *self.last_submission_mut(handle)? == Some(id) {
            return Ok(());
        }
        let dependencies: smallvec::SmallVec<[Handle; 8]> = self
            .reverse_dependencies
            .get(&handle)
            .map(|dependencies| dependencies.iter().copied().collect())
            .unwrap_or_default();
        for dependency in dependencies {
            self.mark_in_flight(dependency, id)?;
        }
        *self.last_submission_mut(handle)? = Some(id);
        Ok(())
    }

    fn last_submission_mut(&mut self, handle: Handle) -> GalResult<&mut Option<SubmissionId>> {
        Ok(match handle.kind() {
            Some(HandleKind::Buffer) => &mut self.buffers.get_mut_record(handle)?.last_submission,
            Some(HandleKind::Texture) => &mut self.textures.get_mut_record(handle)?.last_submission,
            Some(HandleKind::TextureView) => {
                &mut self.texture_views.get_mut_record(handle)?.last_submission
            }
            Some(HandleKind::Sampler) => &mut self.samplers.get_mut_record(handle)?.last_submission,
            Some(HandleKind::CombinedTextureSampler) => {
                &mut self
                    .combined_texture_samplers
                    .get_mut_record(handle)?
                    .last_submission
            }
            Some(HandleKind::ShaderModule) => {
                &mut self.shaders.get_mut_record(handle)?.last_submission
            }
            Some(HandleKind::ResourceLayout) => {
                &mut self
                    .resource_layouts
                    .get_mut_record(handle)?
                    .last_submission
            }
            Some(HandleKind::ResourceSet) => {
                &mut self.resource_sets.get_mut_record(handle)?.last_submission
            }
            Some(HandleKind::PipelineLayout) => {
                &mut self
                    .pipeline_layouts
                    .get_mut_record(handle)?
                    .last_submission
            }
            Some(HandleKind::GraphicsPipeline) => {
                &mut self
                    .graphics_pipelines
                    .get_mut_record(handle)?
                    .last_submission
            }
            Some(HandleKind::ComputePipeline) => {
                &mut self
                    .compute_pipelines
                    .get_mut_record(handle)?
                    .last_submission
            }
            Some(HandleKind::RenderTarget) => {
                &mut self.render_targets.get_mut_record(handle)?.last_submission
            }
            Some(HandleKind::FrameTarget) => {
                &mut self.frame_targets.get_mut_record(handle)?.last_submission
            }
            Some(HandleKind::RenderPass) => {
                &mut self.render_passes.get_mut_record(handle)?.last_submission
            }
            None => {
                return Err(GalError::handle(
                    StatusCode::WrongHandleType,
                    "unknown handle kind",
                ))
            }
        })
    }
}

/// Whether submissions are validated every frame (command ops, handles and
/// hazards). Always in tests and debug builds; in release builds only with
/// `VULKANIC_GAL_VALIDATION=1` (captures and validation runs set it). Normal
/// play trusts its producers, as Vulkan itself does without validation layers.
pub(crate) fn per_frame_validation() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        cfg!(test)
            || cfg!(debug_assertions)
            || matches!(
                crate::env::var("VULKANIC_GAL_VALIDATION").as_deref(),
                Ok("1" | "true" | "on")
            )
    })
}

/// Calls `visit` with every handle `op` names directly.
pub(super) fn op_handles(op: &CommandOp, mut visit: impl FnMut(Handle)) {
    let handles = &mut visit;
    {
        {
            match op {
                CommandOp::BeginPass {
                    pass,
                    target,
                    colors,
                    depth_stencil,
                } => {
                    handles(*pass);
                    handles(*target);
                    for color in colors {
                        handles(color.view);
                    }
                    if let Some(depth) = depth_stencil {
                        handles(depth.view);
                    }
                }
                CommandOp::BindGraphicsPipeline(handle)
                | CommandOp::BindComputePipeline(handle)
                | CommandOp::DrawIndirect { buffer: handle, .. }
                | CommandOp::DrawIndexedIndirect { buffer: handle, .. }
                | CommandOp::DispatchIndirect { buffer: handle, .. }
                | CommandOp::SetIndexBuffer { buffer: handle, .. }
                | CommandOp::HostWriteBuffer { buffer: handle, .. }
                | CommandOp::HostReadBuffer { buffer: handle, .. }
                | CommandOp::Barrier(crate::commands::ResourceBarrier {
                    resource: handle, ..
                }) => {
                    handles(*handle);
                }
                CommandOp::CopyBufferToTexture(region) | CommandOp::CopyTextureToBuffer(region) => {
                    handles(region.buffer);
                    handles(region.texture);
                }
                CommandOp::CopyTexture(region) => {
                    handles(region.src_texture);
                    handles(region.dst_texture);
                }
                CommandOp::CopyFrameTargetToTexture { src, dst, .. } => {
                    handles(*src);
                    handles(*dst);
                }
                CommandOp::CopyTextureToFrameTarget { src, dst, .. } => {
                    handles(*src);
                    handles(*dst);
                }
                CommandOp::GenerateMipmaps { texture, .. } => {
                    handles(*texture);
                }
                CommandOp::Present {
                    texture: handle, ..
                } => {
                    handles(*handle);
                }
                CommandOp::BindResourceSet {
                    pipeline_layout,
                    set,
                    ..
                } => {
                    handles(*pipeline_layout);
                    handles(*set);
                }
                CommandOp::SetVertexBuffer { buffer, .. } => {
                    handles(*buffer);
                }
                CommandOp::CopyBuffer { src, dst, .. }
                | CommandOp::CopyBufferRegion { src, dst, .. } => {
                    handles(*src);
                    handles(*dst);
                }
                CommandOp::Draw { .. }
                | CommandOp::DrawIndexed { .. }
                | CommandOp::Dispatch { .. }
                | CommandOp::TrackSubmission(_)
                | CommandOp::EndPass => {}
            }
        }
    }
}

/// Diagnostic tracing selected by label: the environment variable holds a
/// case-insensitive label substring; unset or empty traces nothing.
pub(super) fn trace_label_matches(variable: &str, label: &str) -> bool {
    crate::env::var(variable).is_ok_and(|filter| {
        !filter.is_empty()
            && label
                .to_ascii_lowercase()
                .contains(&filter.to_ascii_lowercase())
    })
}
