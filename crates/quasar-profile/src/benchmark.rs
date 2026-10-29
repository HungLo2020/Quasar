//! Bounded, configurable live benchmark sessions shared by any application.
use super::*;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct BenchmarkConfig {
    pub warmup_frames: u64,
    pub sample_capacity: usize,
    pub thresholds: ProfileThresholds,
}
impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            warmup_frames: 8,
            sample_capacity: 600,
            thresholds: ProfileThresholds::default(),
        }
    }
}
impl BenchmarkConfig {
    pub fn validate(self) -> Result<(), BenchmarkError> {
        if !(1..=100_000).contains(&self.sample_capacity) || self.warmup_frames > 1_000_000 {
            return Err(BenchmarkError::InvalidConfig(
                "benchmark capacity must be 1..=100000 and warmup <=1000000",
            ));
        }
        Ok(())
    }
    #[cfg(feature = "config")]
    pub fn register_defaults(
        config: &mut quasar_config::Config,
        prefix: &str,
    ) -> Result<(), quasar_config::ConfigError> {
        config.set_default(
            format!("{prefix}.warmup_frames"),
            Self::default().warmup_frames,
        )?;
        config.set_default(
            format!("{prefix}.sample_capacity"),
            Self::default().sample_capacity,
        )?;
        for name in [
            "cpu_frame",
            "gpu_frame",
            "acquire",
            "present",
            "draws",
            "resource_creates",
            "host_write_bytes",
        ] {
            config.set_default(format!("{prefix}.max_{name}_p99"), 0u64)?;
        }
        Ok(())
    }
    /// Zero threshold values mean unbounded; time limits are in nanoseconds.
    #[cfg(feature = "config")]
    pub fn from_config(
        config: &quasar_config::Config,
        prefix: &str,
    ) -> Result<Self, quasar_config::ConfigError> {
        let limit = |name: &str| {
            config
                .get_u64(&format!("{prefix}.max_{name}_p99"))
                .map(|n| (n != 0).then_some(n))
        };
        let result = Self {
            warmup_frames: config.get_u64(&format!("{prefix}.warmup_frames"))?,
            sample_capacity: usize::try_from(config.get_u64(&format!("{prefix}.sample_capacity"))?)
                .unwrap_or(usize::MAX),
            thresholds: ProfileThresholds {
                max_cpu_frame_p99_nanos: limit("cpu_frame")?,
                max_gpu_frame_p99_nanos: limit("gpu_frame")?,
                max_acquire_p99_nanos: limit("acquire")?,
                max_present_p99_nanos: limit("present")?,
                max_draws_p99: limit("draws")?,
                max_resource_creates_p99: limit("resource_creates")?,
                max_host_write_bytes_p99: limit("host_write_bytes")?,
            },
        };
        result.validate().map_err(|e| {
            quasar_config::ConfigError::new(
                quasar_config::ConfigErrorKind::Validation,
                e.to_string(),
            )
        })?;
        Ok(result)
    }
}
#[derive(Clone, Debug)]
pub struct BenchmarkSession {
    config: BenchmarkConfig,
    seen: u64,
    profiler: FrameProfiler,
    metadata: BTreeMap<String, String>,
}
impl BenchmarkSession {
    pub fn new(config: BenchmarkConfig) -> Result<Self, BenchmarkError> {
        config.validate()?;
        Ok(Self {
            config,
            seen: 0,
            profiler: FrameProfiler::new(config.sample_capacity),
            metadata: BTreeMap::new(),
        })
    }
    /// Caller-supplied context only. Bounded to 64 entries / 256 bytes per string.
    pub fn set_metadata(&mut self, key: &str, value: &str) -> Result<(), BenchmarkError> {
        if key.is_empty()
            || key.len() > 256
            || value.len() > 256
            || (!self.metadata.contains_key(key) && self.metadata.len() >= 64)
        {
            return Err(BenchmarkError::InvalidConfig(
                "benchmark metadata budget exceeded",
            ));
        }
        self.metadata.insert(key.into(), value.into());
        Ok(())
    }
    pub fn record(&mut self, sample: FrameSample) -> bool {
        self.seen = self.seen.saturating_add(1);
        if self.seen <= self.config.warmup_frames {
            return false;
        }
        self.profiler.record(sample);
        true
    }
    pub fn summary(&self) -> ProfileSummary {
        self.profiler.summary()
    }
    pub fn check(&self) -> Result<(), BenchmarkError> {
        let summary = self.summary();
        if summary.window_samples == 0 {
            return Err(BenchmarkError::NoSamples);
        }
        if self.config.thresholds.max_gpu_frame_p99_nanos.is_some()
            && summary.gpu_frame.samples < summary.window_samples as u64
        {
            return Err(BenchmarkError::MissingGpu);
        }
        self.config
            .thresholds
            .check(&summary)
            .map_err(BenchmarkError::Threshold)
    }
    #[cfg(feature = "json")]
    pub fn write_json(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        // Preserve existing top-level summary keys for consumers and old fixtures.
        let mut value = serde_json::to_value(self.summary()).map_err(std::io::Error::other)?;
        value.as_object_mut().unwrap().insert("benchmark".into(),serde_json::json!({"config":self.config,"metadata":self.metadata,"seen_frames":self.seen}));
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?,
        )
    }
    #[cfg(feature = "diagnostics")]
    pub fn report(
        &self,
        diagnostics: &quasar_diagnostics::Diagnostics,
    ) -> Result<(), BenchmarkError> {
        use quasar_diagnostics::{Field, Level};
        if let Err(error) = self.check() {
            diagnostics.failure("quasar.profile", "benchmark_failed", &error);
            return Err(error);
        }
        if diagnostics.is_enabled(Level::Info) {
            let s = self.summary();
            diagnostics.event_with(
                Level::Info,
                "quasar.profile",
                "benchmark_complete",
                [
                    Field::new("samples", s.window_samples as u64),
                    Field::new("cpu_p99_ns", s.cpu_frame.p99),
                    Field::new("submit_p99_ns", s.submit.p99),
                    Field::new("gpu_samples", s.gpu_frame.samples),
                    Field::new("gpu_p99_ns", s.gpu_frame.p99),
                    Field::new("draws_p99", s.draws.p99),
                    Field::new("culled_instances_p99", s.culled_instances.p99),
                ],
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BenchmarkError {
    InvalidConfig(&'static str),
    NoSamples,
    MissingGpu,
    Threshold(ProfileViolation),
}
impl std::fmt::Display for BenchmarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(s) => f.write_str(s),
            Self::NoSamples => f.write_str("benchmark has no samples after warmup"),
            Self::MissingGpu => {
                f.write_str("GPU threshold requires valid timestamps for every measured frame")
            }
            Self::Threshold(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for BenchmarkError {}
