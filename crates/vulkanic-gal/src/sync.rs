//! Submission ids and the deferred-retirement queue.

use std::collections::VecDeque;

use super::handles::Handle;

/// Identifies one accepted submission. Ids increase monotonically, so
/// "completed through id N" covers every earlier submission.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubmissionId(pub u64);

/// Returned by a submission; names it for completion queries.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SyncToken {
    /// The submission this token names.
    pub submission: SubmissionId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RetiredResource {
    pub handle: Handle,
    pub submission: SubmissionId,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RetirementQueue {
    /// In deferral order; submissions are not monotonic in it (a resource
    /// deferred later may have last been used earlier).
    pending: VecDeque<RetiredResource>,
    /// The oldest pending submission, so a poll before it completes is free.
    oldest: Option<SubmissionId>,
}

impl RetirementQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn defer(&mut self, handle: Handle, submission: SubmissionId) {
        self.pending
            .push_back(RetiredResource { handle, submission });
        self.oldest = Some(
            self.oldest
                .map_or(submission, |oldest| oldest.min(submission)),
        );
    }

    /// Removes and returns the resources whose submission completed, in
    /// deferral order.
    pub fn drain_completed(&mut self, completed: SubmissionId) -> Vec<RetiredResource> {
        if self.oldest.is_none_or(|oldest| oldest > completed) {
            return Vec::new();
        }
        let mut retired = Vec::new();
        self.pending.retain(|resource| {
            let done = resource.submission <= completed;
            if done {
                retired.push(*resource);
            }
            !done
        });
        self.oldest = self
            .pending
            .iter()
            .map(|resource| resource.submission)
            .min();
        retired
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}
