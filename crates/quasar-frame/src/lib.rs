//! Optional caller-owned frame transactions, extracted from the explicit
//! composition example. No renderer, window, scene graph, runtime or scheduler.
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};
use vulkanic_gal::{metrics::SubmitProfile, resources::FrameTargetDesc, *};

/// RAII boundary over GAL's existing nestable recording scopes. Error and
/// panic unwinding close the scope; it does not implement another lifetime system.
struct RecordingScope<'a> {
    gal: &'a mut VulkanicGal,
    active: bool,
}
impl<'a> RecordingScope<'a> {
    fn new(gal: &'a mut VulkanicGal) -> GalResult<Self> {
        gal.begin_command_recording()?;
        Ok(Self { gal, active: true })
    }
}
impl Drop for RecordingScope<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.gal.finish_command_recording();
        }
    }
}
/// Synchronous reference policy, or bounded frontend slots + completion polling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionPolicy {
    Wait,
    Poll,
}
/// Opens GAL's existing recording lifetime, runs preparation and submits one
/// ordered batch. Always closes the recording scope, including on rejection.
/// A closure must not retain commands whose resources it explicitly destroys
/// within this transaction. Renderer-produced commands retain their own slots.
pub fn record_and_submit(
    gal: &mut VulkanicGal,
    prepare: impl FnOnce(&mut VulkanicGal) -> GalResult<Vec<CommandList>>,
) -> GalResult<(SyncToken, SubmitProfile)> {
    let creates = gal.metrics().resource_creates;
    let destroys = gal.metrics().resource_destroys;
    let mut scope = RecordingScope::new(gal)?;
    let gal = &mut *scope.gal;
    let mut profile = SubmitProfile::default();
    let started = Instant::now();
    let result = (|| {
        let commands = prepare(gal)?;
        profile.gal_command_generation_nanos = nanos(started.elapsed());
        gal.submit_profiled(
            SubmissionBatch {
                label: "quasar-frame".into(),
                command_lists: commands,
            },
            &mut profile,
        )
    })();
    let finish_started = Instant::now();
    let deferred = gal.command_recording_deferred_destroy_count();
    let finish = gal.finish_command_recording();
    scope.active = false;
    let gal = &mut *scope.gal;
    profile.gal_command_recording_finish_nanos = nanos(finish_started.elapsed());
    profile.gal_command_recording_deferred_destroys = deferred as u64;
    profile.resource_creates_delta = gal.metrics().resource_creates.saturating_sub(creates);
    profile.resource_destroys_delta = gal.metrics().resource_destroys.saturating_sub(destroys);
    result.and_then(|token| finish.map(|()| (token, profile)))
}
#[derive(Debug, Clone)]
pub struct SurfaceFrame {
    pub acquired: AcquiredFrame,
    pub target: Handle,
    started: Instant,
}
#[derive(Debug, Clone, Copy)]
pub struct FrameReport {
    pub submission: SubmissionId,
    pub profile: SubmitProfile,
    pub acquire_nanos: u64,
    pub present_nanos: u64,
    pub cpu_frame_nanos: u64,
}
/// Tracks one already configured GAL surface. The caller owns its GAL/window.
/// Cached target wrappers are retired through GAL when resize/cancel invalidates
/// the swapchain. Renderers independently retire their dependent pass caches.
pub struct FrameSurface {
    size: (u32, u32),
    dirty: bool,
    correlation: u64,
    targets: BTreeMap<FrameRenderTargetId, Handle>,
    pending: VecDeque<(SubmissionId, Instant, Instant)>,
    completed: VecDeque<(SubmissionId, u64)>,
    frame_completed: VecDeque<(SubmissionId, u64)>,
    last_present_status: Option<FramePresentStatus>,
}
impl FrameSurface {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            size: (width, height),
            dirty: false,
            correlation: 0,
            targets: BTreeMap::new(),
            pending: VecDeque::new(),
            completed: VecDeque::new(),
            frame_completed: VecDeque::new(),
            last_present_status: None,
        }
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        self.dirty = true;
    }
    /// Current logical surface size.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    /// Whether the next acquire will resize the surface and retire cached targets.
    pub fn needs_resize(&self) -> bool {
        self.dirty
    }
    pub fn acquire(&mut self, gal: &mut VulkanicGal) -> GalResult<Option<SurfaceFrame>> {
        let started = Instant::now();
        if self.size.0 == 0 || self.size.1 == 0 {
            return Ok(None);
        }
        self.correlation = self
            .correlation
            .checked_add(1)
            .ok_or_else(|| GalError::invalid_argument("frame correlation exhausted"))?;
        let correlation_id = FrameCorrelationId(self.correlation);
        let extent = vulkanic_gal::resources::Extent3d {
            width: self.size.0,
            height: self.size.1,
            depth: 1,
        };
        if self.dirty {
            self.release_targets(gal)?;
            gal.resize_frame_surface(FrameResizeDesc {
                correlation_id,
                extent,
            })?;
            self.dirty = false;
        }
        let acquired = gal.acquire_frame(FrameAcquireDesc {
            correlation_id,
            expected_extent: extent,
        })?;
        if matches!(
            acquired.status,
            FrameAcquireStatus::Resized | FrameAcquireStatus::Minimized
        ) {
            self.release_targets(gal)?;
            return Ok(None);
        }
        let target = if let Some(h) = self.targets.get(&acquired.render_target) {
            *h
        } else {
            match gal.create_frame_target(FrameTargetDesc {
                label: "quasar-frame-target".into(),
                frame_id: acquired.frame.0,
                render_target: acquired.render_target,
                extent: acquired.extent,
                color_format: acquired.color_format,
            }) {
                Ok(h) => {
                    self.targets.insert(acquired.render_target, h);
                    h
                }
                Err(e) => {
                    let _ = gal.cancel_frame(acquired.frame);
                    let _ = self.release_targets(gal);
                    return Err(e);
                }
            }
        };
        Ok(Some(SurfaceFrame {
            acquired,
            target,
            started,
        }))
    }
    /// Present once. Poll mode does not wait for this frame on the CPU; a
    /// backend may still apply acquire/queue/backpressure waits when required.
    pub fn present(
        &mut self,
        gal: &mut VulkanicGal,
        frame: &SurfaceFrame,
        token: SyncToken,
        policy: CompletionPolicy,
    ) -> GalResult<()> {
        self.pending
            .push_back((token.submission, Instant::now(), frame.started));
        let presented = gal.present_frame(PresentFrameDesc {
            frame: frame.acquired.frame,
            correlation_id: frame.acquired.correlation_id,
            wait_for: token.submission,
        })?;
        self.last_present_status = Some(presented.status);
        if frame.acquired.status == FrameAcquireStatus::Suboptimal
            || matches!(
                presented.status,
                FramePresentStatus::Suboptimal | FramePresentStatus::OutOfDate
            )
        {
            self.dirty = true;
        }
        match policy {
            CompletionPolicy::Wait => {
                gal.retire_through(token.submission)?;
            }
            CompletionPolicy::Poll => {
                gal.retire_completed()?;
            }
        }
        self.observe_completion(gal);
        Ok(())
    }
    /// Cancels an acquired frame after failed preparation/submission/present.
    pub fn cancel(&mut self, gal: &mut VulkanicGal, frame: &SurfaceFrame) -> GalResult<()> {
        gal.cancel_frame(frame.acquired.frame)?;
        self.release_targets(gal)
    }
    /// Status returned by the most recent successful `present`, if any.
    pub fn last_present_status(&self) -> Option<FramePresentStatus> {
        self.last_present_status
    }
    pub fn render(
        &mut self,
        gal: &mut VulkanicGal,
        policy: CompletionPolicy,
        prepare: impl FnOnce(&mut VulkanicGal, Handle) -> GalResult<Vec<CommandList>>,
    ) -> GalResult<Option<FrameReport>> {
        let start = Instant::now();
        let Some(frame) = self.acquire(gal)? else {
            return Ok(None);
        };
        let acquire_nanos = nanos(start.elapsed());
        let result = (|| {
            let (token, profile) = record_and_submit(gal, |gal| prepare(gal, frame.target))?;
            let present_started = Instant::now();
            self.present(gal, &frame, token, policy)?;
            Ok(FrameReport {
                submission: token.submission,
                profile,
                acquire_nanos,
                present_nanos: nanos(present_started.elapsed()),
                cpu_frame_nanos: nanos(start.elapsed()),
            })
        })();
        if result.is_err() {
            let _ = self.cancel(gal, &frame);
        }
        result.map(Some)
    }
    fn observe_completion(&mut self, gal: &mut VulkanicGal) {
        let completed = gal.poll_completed();
        let now = Instant::now();
        while self
            .pending
            .front()
            .is_some_and(|(id, _, _)| *id <= completed)
        {
            let (id, start, frame_start) = self.pending.pop_front().unwrap();
            if self.completed.len() == 256 {
                self.completed.pop_front();
            }
            self.completed.push_back((id, nanos(now - start)));
            if self.frame_completed.len() == 256 {
                self.frame_completed.pop_front();
            }
            self.frame_completed
                .push_back((id, nanos(now - frame_start)));
        }
    }
    /// Submission-to-observed-GPU-completion latency, not input-to-photon or
    /// compositor/display latency. Polling cadence affects the observation.
    pub fn take_completion_latencies(&mut self) -> Vec<(SubmissionId, u64)> {
        self.completed.drain(..).collect()
    }
    /// End-to-end engine frame latency: acquire entry through host-observed GPU
    /// completion, including preparation/submission/acquire stalls. This still
    /// excludes event dispatch, compositor and physical display latency.
    pub fn take_frame_completion_latencies(&mut self) -> Vec<(SubmissionId, u64)> {
        self.frame_completed.drain(..).collect()
    }
    pub fn finish(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        gal.retire_through(gal.latest_submission_id())?;
        self.observe_completion(gal);
        Ok(())
    }
    pub fn release_targets(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        for h in std::mem::take(&mut self.targets).into_values() {
            gal.retire(h)?;
        }
        Ok(())
    }
}
fn nanos(d: Duration) -> u64 {
    d.as_nanos().min(u64::MAX as u128) as u64
}
