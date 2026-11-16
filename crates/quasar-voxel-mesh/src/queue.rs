//! Revision-aware background extraction on `quasar-jobs`.
use quasar_jobs::{JobError, JobHandle, JobPool, JobPriority};
use std::collections::BTreeMap;

/// Tracks extraction jobs per key (usually a chunk coordinate) and drops
/// results whose revision is no longer the latest requested. Results come
/// back in key order, so consumers stay deterministic regardless of
/// scheduling.
pub struct RemeshQueue<K: Ord + Copy, T: Send + 'static> {
    pending: BTreeMap<K, (u64, JobHandle<T>)>,
    latest: BTreeMap<K, u64>,
}

impl<K: Ord + Copy, T: Send + 'static> Default for RemeshQueue<K, T> {
    fn default() -> Self {
        Self {
            pending: BTreeMap::new(),
            latest: BTreeMap::new(),
        }
    }
}

impl<K: Ord + Copy, T: Send + 'static> RemeshQueue<K, T> {
    pub fn new() -> Self {
        Self::default()
    }
    /// Requests extraction of `key` at `revision`; a pending older job for
    /// the key is cancelled. Requests for an already-requested revision are
    /// ignored.
    pub fn request(
        &mut self,
        pool: &JobPool,
        key: K,
        revision: u64,
        priority: i32,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<bool, JobError> {
        if self.latest.get(&key) == Some(&revision) && self.pending.contains_key(&key) {
            return Ok(false);
        }
        if let Some((_, old)) = self.pending.remove(&key) {
            old.cancel();
        }
        let handle = pool.spawn_with_priority(JobPriority::new(priority), move |ctx| {
            ctx.check_cancelled()?;
            Ok::<T, JobError>(work())
        })?;
        self.latest.insert(key, revision);
        self.pending.insert(key, (revision, handle));
        Ok(true)
    }
    /// Finished, still-current results in key order.
    pub fn poll(&mut self) -> Vec<(K, u64, T)> {
        let mut done = Vec::new();
        let keys: Vec<K> = self.pending.keys().copied().collect();
        for key in keys {
            let finished = self.pending.get(&key).and_then(|(_, h)| h.try_take());
            if let Some(result) = finished {
                let (revision, _) = self.pending.remove(&key).unwrap();
                if let Ok(value) = result
                    && self.latest.get(&key) == Some(&revision)
                {
                    done.push((key, revision, value));
                }
            }
        }
        done
    }
    /// Blocks until every pending job finishes, returning current results.
    pub fn wait_all(&mut self) -> Vec<(K, u64, T)> {
        let mut done = Vec::new();
        let pending = std::mem::take(&mut self.pending);
        for (key, (revision, handle)) in pending {
            if let Ok(value) = handle.wait()
                && self.latest.get(&key) == Some(&revision)
            {
                done.push((key, revision, value));
            }
        }
        done
    }
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
    /// Forgets a key (e.g. its chunk was unloaded) and cancels its job.
    pub fn forget(&mut self, key: K) {
        if let Some((_, handle)) = self.pending.remove(&key) {
            handle.cancel();
        }
        self.latest.remove(&key);
    }
}
