//! Reproducible graphics profiling and performance-regression summaries.
//!
//! The crate samples caller-owned frame measurements, computes distributions
//! (including p99), checks configured thresholds and optionally writes JSON.
//! It does not create a GAL, renderer, window or global profiler.
//!
//! ```
//! use quasar_profile::{FrameProfiler, FrameSample};
//!
//! let mut profiler = FrameProfiler::new(120);
//! profiler.record(FrameSample {
//!     frame: 1,
//!     cpu_frame_nanos: 1_000_000,
//!     ..FrameSample::default()
//! });
//! let summary = profiler.summary();
//! assert_eq!(summary.frames, 1);
//! assert_eq!(summary.cpu_frame.p99, 1_000_000);
//! ```

#[cfg(feature = "frame")]
use quasar_frame::FrameReport;
use std::collections::VecDeque;
mod benchmark;
pub use benchmark::{BenchmarkConfig, BenchmarkError, BenchmarkSession};

#[cfg(feature = "json")]
use serde::{Deserialize, Serialize};

/// Renderer-produced counters for one frame. Callers map their own renderer
/// stats into this neutral shape.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct RendererStats {
    pub draws: u64,
    pub geometry_upload_bytes: u64,
    pub texture_upload_bytes: u64,
    pub uniform_upload_bytes: u64,
}

/// One frame of CPU, GPU, resource, upload and synchronization measurements.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct FrameSample {
    /// Native frame reports cannot expose backend-private counters. Such fields
    /// produce empty distributions rather than zero-valued measurements.
    #[cfg_attr(feature = "json", serde(default))]
    pub backend_details_unavailable: bool,
    pub frame: u64,
    pub cpu_frame_nanos: u64,
    pub acquire_nanos: u64,
    pub present_nanos: u64,
    pub submit_nanos: u64,
    #[cfg_attr(feature = "json", serde(default))]
    pub instance_upload_bytes: u64,
    #[cfg_attr(feature = "json", serde(default))]
    pub culled_instances: u64,
    #[cfg_attr(feature = "json", serde(default))]
    pub visible_instances: u64,
    #[cfg_attr(feature = "json", serde(default))]
    pub backend_submit_nanos: u64,
    #[cfg_attr(feature = "json", serde(default))]
    pub backend_encode_nanos: u64,
    #[cfg_attr(feature = "json", serde(default))]
    pub hazard_analysis_nanos: u64,
    #[cfg_attr(feature = "json", serde(default))]
    pub validation_nanos: u64,
    #[cfg_attr(feature = "json", serde(default))]
    pub renderer_prepare_nanos: u64,
    pub gpu_frame_nanos: Option<u64>,
    #[cfg(feature = "frame")]
    #[cfg_attr(feature = "json", serde(skip))]
    pub gpu_timing: Option<quasar_frame::GpuTiming>,
    pub gpu_scope_nanos: Vec<u64>,
    pub draws: u64,
    pub geometry_upload_bytes: u64,
    pub texture_upload_bytes: u64,
    pub uniform_upload_bytes: u64,
    pub host_write_bytes: u64,
    pub host_write_ops: u64,
    pub resource_creates: u64,
    pub resource_destroys: u64,
    pub blocking_waits: u64,
    pub device_idle_waits: u64,
}
impl FrameSample {
    pub fn with_renderer_stats(&mut self, stats: RendererStats) -> &mut Self {
        self.draws = stats.draws;
        self.geometry_upload_bytes = stats.geometry_upload_bytes;
        self.texture_upload_bytes = stats.texture_upload_bytes;
        self.uniform_upload_bytes = stats.uniform_upload_bytes;
        self
    }
    #[cfg(feature = "frame")]
    pub fn from_frame_report(frame: u64, report: &FrameReport) -> Self {
        Self {
            frame,
            backend_details_unavailable: true,
            cpu_frame_nanos: report.cpu_frame_nanos,
            acquire_nanos: report.acquire_nanos,
            present_nanos: report.present_nanos,
            submit_nanos: report.submit_nanos,
            renderer_prepare_nanos: report.prepare_nanos,
            gpu_frame_nanos: report.gpu_frame_nanos,
            gpu_timing: report.gpu_timing.clone(),
            blocking_waits: report.blocking_waits,
            ..Self::default()
        }
    }
}

/// Summary of one metric over a bounded sample window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct Distribution {
    pub samples: u64,
    pub min: u64,
    pub mean: f64,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
}
impl Distribution {
    pub fn from_values(values: impl IntoIterator<Item = u64>) -> Self {
        let mut values: Vec<u64> = values.into_iter().collect();
        if values.is_empty() {
            return Self::default();
        }
        values.sort_unstable();
        let samples = values.len() as u64;
        let mean = values.iter().map(|value| *value as f64).sum::<f64>() / samples as f64;
        let percentile = |percent: usize| {
            let index = (values.len() * percent)
                .div_ceil(100)
                .saturating_sub(1)
                .min(values.len() - 1);
            values[index]
        };
        Self {
            samples,
            min: values[0],
            mean,
            p50: percentile(50),
            p95: percentile(95),
            p99: percentile(99),
            max: *values.last().unwrap(),
        }
    }
}

/// Aggregated measurements over the profiler's retained window.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct ProfileSummary {
    pub frames: u64,
    pub window_samples: usize,
    pub cpu_frame: Distribution,
    pub acquire: Distribution,
    pub present: Distribution,
    pub submit: Distribution,
    #[cfg_attr(feature = "json", serde(default))]
    pub instance_upload_bytes: Distribution,
    #[cfg_attr(feature = "json", serde(default))]
    pub culled_instances: Distribution,
    #[cfg_attr(feature = "json", serde(default))]
    pub visible_instances: Distribution,
    #[cfg_attr(feature = "json", serde(default))]
    pub backend_submit: Distribution,
    #[cfg_attr(feature = "json", serde(default))]
    pub backend_encode: Distribution,
    #[cfg_attr(feature = "json", serde(default))]
    pub hazard_analysis: Distribution,
    #[cfg_attr(feature = "json", serde(default))]
    pub validation: Distribution,
    #[cfg_attr(feature = "json", serde(default))]
    pub renderer_prepare: Distribution,
    pub gpu_frame: Distribution,
    pub gpu_scopes: Vec<Distribution>,
    pub draws: Distribution,
    pub geometry_upload_bytes: Distribution,
    pub texture_upload_bytes: Distribution,
    pub uniform_upload_bytes: Distribution,
    pub host_write_bytes: Distribution,
    pub host_write_ops: Distribution,
    pub resource_creates: Distribution,
    pub resource_destroys: Distribution,
    pub blocking_waits: Distribution,
    pub device_idle_waits: Distribution,
}

/// Bounded ring of frame measurements.
#[derive(Clone, Debug)]
pub struct FrameProfiler {
    capacity: usize,
    samples: VecDeque<FrameSample>,
    frames: u64,
}
impl FrameProfiler {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            samples: VecDeque::with_capacity(capacity.max(1)),
            frames: 0,
        }
    }
    pub fn record(&mut self, sample: FrameSample) {
        self.frames = self.frames.saturating_add(1);
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }
    #[cfg(feature = "frame")]
    pub fn record_report(&mut self, frame: u64, report: &FrameReport) -> &mut Self {
        self.record(FrameSample::from_frame_report(frame, report));
        self
    }
    pub fn samples(&self) -> &VecDeque<FrameSample> {
        &self.samples
    }
    pub fn len(&self) -> usize {
        self.samples.len()
    }
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
    pub fn total_frames(&self) -> u64 {
        self.frames
    }
    pub fn clear(&mut self) {
        self.samples.clear();
        self.frames = 0;
    }
    pub fn summary(&self) -> ProfileSummary {
        let gpu_scope_count = self
            .samples
            .iter()
            .map(|sample| sample.gpu_scope_nanos.len())
            .max()
            .unwrap_or(0);
        ProfileSummary {
            frames: self.frames,
            window_samples: self.samples.len(),
            cpu_frame: values(&self.samples, |sample| sample.cpu_frame_nanos),
            acquire: values(&self.samples, |sample| sample.acquire_nanos),
            present: values(&self.samples, |sample| sample.present_nanos),
            submit: values(&self.samples, |sample| sample.submit_nanos),
            instance_upload_bytes: values(&self.samples, |sample| sample.instance_upload_bytes),
            culled_instances: values(&self.samples, |sample| sample.culled_instances),
            visible_instances: values(&self.samples, |sample| sample.visible_instances),
            backend_submit: values_optional(&self.samples, |sample| {
                (!sample.backend_details_unavailable).then_some(sample.backend_submit_nanos)
            }),
            backend_encode: values_optional(&self.samples, |sample| {
                (!sample.backend_details_unavailable).then_some(sample.backend_encode_nanos)
            }),
            hazard_analysis: values_optional(&self.samples, |sample| {
                (!sample.backend_details_unavailable).then_some(sample.hazard_analysis_nanos)
            }),
            validation: values_optional(&self.samples, |sample| {
                (!sample.backend_details_unavailable).then_some(sample.validation_nanos)
            }),
            renderer_prepare: values(&self.samples, |sample| sample.renderer_prepare_nanos),
            gpu_frame: values_optional(&self.samples, |sample| {
                #[cfg(feature = "frame")]
                {
                    sample.gpu_frame_nanos.or_else(|| {
                        sample
                            .gpu_timing
                            .as_ref()
                            .and_then(quasar_frame::GpuTiming::nanos)
                    })
                }
                #[cfg(not(feature = "frame"))]
                {
                    sample.gpu_frame_nanos
                }
            }),
            gpu_scopes: (0..gpu_scope_count)
                .map(|index| {
                    Distribution::from_values(
                        self.samples
                            .iter()
                            .filter_map(|sample| sample.gpu_scope_nanos.get(index).copied()),
                    )
                })
                .collect(),
            draws: values(&self.samples, |sample| sample.draws),
            geometry_upload_bytes: values(&self.samples, |sample| sample.geometry_upload_bytes),
            texture_upload_bytes: values(&self.samples, |sample| sample.texture_upload_bytes),
            uniform_upload_bytes: values(&self.samples, |sample| sample.uniform_upload_bytes),
            host_write_bytes: values_optional(&self.samples, |sample| {
                (!sample.backend_details_unavailable).then_some(sample.host_write_bytes)
            }),
            host_write_ops: values_optional(&self.samples, |sample| {
                (!sample.backend_details_unavailable).then_some(sample.host_write_ops)
            }),
            resource_creates: values_optional(&self.samples, |sample| {
                (!sample.backend_details_unavailable).then_some(sample.resource_creates)
            }),
            resource_destroys: values_optional(&self.samples, |sample| {
                (!sample.backend_details_unavailable).then_some(sample.resource_destroys)
            }),
            blocking_waits: values(&self.samples, |sample| sample.blocking_waits),
            device_idle_waits: values_optional(&self.samples, |sample| {
                (!sample.backend_details_unavailable).then_some(sample.device_idle_waits)
            }),
        }
    }
    #[cfg(feature = "json")]
    pub fn write_json(&self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.summary()).map_err(std::io::Error::other)?;
        std::fs::write(path, bytes)
    }
    #[cfg(feature = "json")]
    pub fn summary_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.summary())
    }
}

/// Optional p99 regression limits. Unspecified limits are not checked.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "json", derive(Serialize, Deserialize))]
pub struct ProfileThresholds {
    pub max_cpu_frame_p99_nanos: Option<u64>,
    pub max_gpu_frame_p99_nanos: Option<u64>,
    pub max_acquire_p99_nanos: Option<u64>,
    pub max_present_p99_nanos: Option<u64>,
    pub max_draws_p99: Option<u64>,
    pub max_resource_creates_p99: Option<u64>,
    pub max_host_write_bytes_p99: Option<u64>,
}
impl ProfileThresholds {
    pub fn check(&self, summary: &ProfileSummary) -> Result<(), ProfileViolation> {
        check(
            "cpu_frame.p99",
            summary.cpu_frame.p99,
            self.max_cpu_frame_p99_nanos,
        )?;
        check(
            "gpu_frame.p99",
            summary.gpu_frame.p99,
            self.max_gpu_frame_p99_nanos,
        )?;
        check(
            "acquire.p99",
            summary.acquire.p99,
            self.max_acquire_p99_nanos,
        )?;
        check(
            "present.p99",
            summary.present.p99,
            self.max_present_p99_nanos,
        )?;
        check("draws.p99", summary.draws.p99, self.max_draws_p99)?;
        check(
            "resource_creates.p99",
            summary.resource_creates.p99,
            self.max_resource_creates_p99,
        )?;
        check(
            "host_write_bytes.p99",
            summary.host_write_bytes.p99,
            self.max_host_write_bytes_p99,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProfileViolation {
    pub metric: &'static str,
    pub observed: u64,
    pub limit: u64,
}
impl std::fmt::Display for ProfileViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "profiling threshold exceeded: {}={} > {}",
            self.metric, self.observed, self.limit
        )
    }
}
impl std::error::Error for ProfileViolation {}

fn check(metric: &'static str, observed: u64, limit: Option<u64>) -> Result<(), ProfileViolation> {
    match limit {
        Some(limit) if observed > limit => Err(ProfileViolation {
            metric,
            observed,
            limit,
        }),
        _ => Ok(()),
    }
}
fn values(samples: &VecDeque<FrameSample>, get: impl Fn(&FrameSample) -> u64) -> Distribution {
    Distribution::from_values(samples.iter().map(get))
}
fn values_optional(
    samples: &VecDeque<FrameSample>,
    get: impl Fn(&FrameSample) -> Option<u64>,
) -> Distribution {
    Distribution::from_values(samples.iter().filter_map(get))
}
