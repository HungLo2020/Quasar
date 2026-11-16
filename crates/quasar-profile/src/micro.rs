//! CPU microbenchmark timing and target gates that tolerate a busy machine.
//!
//! Two kinds of noise inflate a timing on a shared desktop:
//!
//! * **Bursts** (another process takes the core for a while): timings are
//!   the best per-iteration mean over several batches, so a preempted batch
//!   is discarded ([`best_batch_micros`]).
//! * **Sustained load** (a busy sibling hyper-thread, a lower all-core
//!   turbo): every pass also times a fixed reference workload
//!   ([`calibration_micros`]); how much slower it runs than on the reference
//!   machine is the pass's *slowdown*, and gated timings are divided by it.
//!   A regression slows the benchmark but not the reference workload, so it
//!   still fails. Slowdown is never below 1: a faster or idle machine does
//!   not tighten targets.
//!
//! [`MicroGate::run`] repeats measurement passes (up to `max_passes`, only
//! while a target is missed) and keeps each metric's best normalised value.
//!
//! ```
//! use quasar_profile::micro::{Metric, MicroGate};
//!
//! let gate = MicroGate::default();
//! let report = gate.run_with(true, || 1.0, || vec![Metric::timed("work_us", 5.0, Some(10.0))]);
//! assert!(report.missed().is_empty());
//! assert_eq!(report.passes, 1);
//! ```
use std::time::Instant;

/// Best per-iteration mean (µs) of `f` over `batches` batches of
/// `iterations / batches` calls each, after one warm-up call.
pub fn best_batch_micros<T>(iterations: usize, batches: usize, mut f: impl FnMut() -> T) -> f64 {
    let _ = std::hint::black_box(f());
    let batches = batches.max(1);
    let per_batch = (iterations / batches).max(1);
    (0..batches)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..per_batch {
                std::hint::black_box(f());
            }
            start.elapsed().as_secs_f64() * 1e6 / per_batch as f64
        })
        .fold(f64::INFINITY, f64::min)
}

/// Fixed reference workload: integer mixing, data-dependent branches and
/// table reads/writes over a 64 KiB L1/L2-resident table, like a mesher or
/// flood fill's inner loop. Returns a checksum so it cannot be optimised out.
fn reference_workload(table: &mut [u16]) -> u64 {
    let mask = table.len() - 1;
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut acc = 0u64;
    for _ in 0..40_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let i = (x as usize) & mask;
        let v = table[i].wrapping_add(x as u16);
        table[i] = v;
        if v & 1 == 0 {
            acc = acc.wrapping_add(u64::from(v));
        } else {
            acc ^= u64::from(table[(i + 1) & mask]);
        }
    }
    acc
}

/// Best time (µs) of the reference workload on this machine right now.
pub fn calibration_micros() -> f64 {
    let mut table = vec![0u16; 32 << 10];
    best_batch_micros(40, 10, || reference_workload(&mut table))
}

/// Reference-workload time on the machine the voxel spec targets are checked
/// on (release build, idle). Slowdown is measured against this.
pub const CALIBRATION_REFERENCE_MICROS: f64 = 137.0;

/// Above this slowdown on every pass, a missed target says more about the
/// machine than the code: [`GateReport::too_busy`].
pub const BUSY_SLOWDOWN: f64 = 1.5;

/// One reported benchmark value.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    pub name: &'static str,
    pub value: f64,
    /// Upper bound checked by the gate.
    pub target: Option<f64>,
    /// Timings merge by minimum and are normalised by slowdown; exact values
    /// (counts, byte sizes) are compared as measured.
    pub timed: bool,
}

impl Metric {
    pub fn timed(name: &'static str, micros: f64, target: Option<f64>) -> Self {
        Self {
            name,
            value: micros,
            target,
            timed: true,
        }
    }
    pub fn exact(name: &'static str, value: f64, target: Option<f64>) -> Self {
        Self {
            name,
            value,
            target,
            timed: false,
        }
    }
}

/// Retry and calibration policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MicroGate {
    /// Passes when a target is missed (1 = no retries).
    pub max_passes: u32,
    pub reference_micros: f64,
}

impl Default for MicroGate {
    fn default() -> Self {
        Self {
            max_passes: 3,
            reference_micros: CALIBRATION_REFERENCE_MICROS,
        }
    }
}

/// Result of [`MicroGate::run`].
#[derive(Clone, Debug, PartialEq)]
pub struct GateReport {
    /// Best raw value per metric (timings) or last measured (exact values).
    pub metrics: Vec<Metric>,
    /// Best slowdown-normalised value per metric (equal to `value` for exact
    /// metrics), checked against `target`.
    pub normalized: Vec<f64>,
    /// Slowdown of each pass (≥ 1).
    pub slowdowns: Vec<f64>,
    pub passes: u32,
}

impl GateReport {
    /// Metrics whose normalised value still exceeds the target.
    pub fn missed(&self) -> Vec<(&Metric, f64)> {
        self.metrics
            .iter()
            .zip(&self.normalized)
            .filter(|(m, n)| m.target.is_some_and(|t| **n > t))
            .map(|(m, n)| (m, *n))
            .collect()
    }

    /// The report as a flat JSON object: each metric's raw value, the
    /// normalised value of gated timings (`<name>_normalized`), passes and
    /// slowdowns, then `extra` pairs (already JSON-encoded values).
    pub fn to_json(&self, extra: &[(&str, String)]) -> String {
        let mut fields: Vec<(String, String)> = Vec::new();
        for (m, n) in self.metrics.iter().zip(&self.normalized) {
            if m.timed {
                fields.push((m.name.into(), format!("{:.2}", m.value)));
                if m.target.is_some() {
                    fields.push((format!("{}_normalized", m.name), format!("{n:.2}")));
                }
            } else {
                fields.push((m.name.into(), format!("{}", m.value)));
            }
        }
        fields.push(("passes".into(), self.passes.to_string()));
        let slowdowns: Vec<String> = self.slowdowns.iter().map(|s| format!("{s:.3}")).collect();
        fields.push(("slowdowns".into(), format!("[{}]", slowdowns.join(", "))));
        fields.extend(extra.iter().map(|(k, v)| ((*k).into(), v.clone())));
        let body: Vec<String> = fields
            .iter()
            .map(|(k, v)| format!("  \"{k}\": {v}"))
            .collect();
        format!("{{\n{}\n}}", body.join(",\n"))
    }

    /// Targets were missed but every pass ran on a heavily loaded machine,
    /// where calibration under-corrects: rerun on an idle machine.
    pub fn too_busy(&self) -> bool {
        !self.missed().is_empty() && self.slowdowns.iter().all(|s| *s > BUSY_SLOWDOWN)
    }

    /// Prints misses to stderr; returns whether every target was met.
    pub fn report_misses(&self) -> bool {
        let missed = self.missed();
        for (m, n) in &missed {
            eprintln!(
                "target missed after {} pass(es): {} = {:.1} (raw {:.1}, slowdown-normalised) > {}",
                self.passes,
                m.name,
                n,
                m.value,
                m.target.unwrap_or_default()
            );
        }
        if self.too_busy() {
            eprintln!(
                "machine too busy to judge (slowdowns {:?} > {BUSY_SLOWDOWN}): rerun when idle",
                self.slowdowns
            );
        }
        missed.is_empty()
    }
}

impl MicroGate {
    /// Measures once; with `check`, re-measures while a target is missed.
    pub fn run(&self, check: bool, measure: impl FnMut() -> Vec<Metric>) -> GateReport {
        self.run_with(check, calibration_micros, measure)
    }

    /// [`Self::run`] with an explicit calibration (µs of the reference
    /// workload), timed before and after each pass and averaged.
    pub fn run_with(
        &self,
        check: bool,
        mut calibrate: impl FnMut() -> f64,
        mut measure: impl FnMut() -> Vec<Metric>,
    ) -> GateReport {
        let mut report = GateReport {
            metrics: Vec::new(),
            normalized: Vec::new(),
            slowdowns: Vec::new(),
            passes: 0,
        };
        loop {
            let before = calibrate();
            let metrics = measure();
            let after = calibrate();
            let slowdown = ((before + after) * 0.5 / self.reference_micros).max(1.0);
            report.passes += 1;
            report.slowdowns.push(slowdown);
            if report.metrics.is_empty() {
                report.normalized = metrics
                    .iter()
                    .map(|m| if m.timed { m.value / slowdown } else { m.value })
                    .collect();
                report.metrics = metrics;
            } else {
                assert_eq!(
                    metrics.len(),
                    report.metrics.len(),
                    "every pass reports the same metrics"
                );
                for ((best, normalized), next) in report
                    .metrics
                    .iter_mut()
                    .zip(&mut report.normalized)
                    .zip(metrics)
                {
                    if best.timed {
                        best.value = best.value.min(next.value);
                        *normalized = normalized.min(next.value / slowdown);
                    } else {
                        best.value = next.value;
                        *normalized = next.value;
                    }
                }
            }
            if !check || report.passes >= self.max_passes.max(1) || report.missed().is_empty() {
                return report;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_only_while_missing_and_keeps_each_best() {
        let gate = MicroGate {
            max_passes: 3,
            reference_micros: 100.0,
        };
        let mut runs = [12.0, 9.0, 4.0].into_iter();
        let report = gate.run_with(
            true,
            || 100.0,
            || {
                let v = runs.next().unwrap();
                vec![
                    Metric::timed("a_us", v, Some(10.0)),
                    Metric::timed("b_us", 20.0 - v, None),
                ]
            },
        );
        assert_eq!(report.passes, 2, "stops once every target is met");
        assert_eq!(report.metrics[0].value, 9.0);
        assert_eq!(report.metrics[1].value, 8.0, "untargeted timings merge too");
        assert!(report.missed().is_empty());
    }

    #[test]
    fn sustained_slowdown_is_forgiven_but_regressions_are_not() {
        let gate = MicroGate {
            max_passes: 2,
            reference_micros: 100.0,
        };
        // Machine 30% slower: workload and reference both slow down.
        let loaded = gate.run_with(
            true,
            || 130.0,
            || vec![Metric::timed("w_us", 12.0, Some(10.0))],
        );
        assert!(loaded.missed().is_empty(), "{loaded:?}");
        assert!((loaded.normalized[0] - 12.0 / 1.3).abs() < 1e-9);
        // Idle machine, slower workload: a real regression.
        let regressed = gate.run_with(
            true,
            || 100.0,
            || vec![Metric::timed("w_us", 12.0, Some(10.0))],
        );
        assert_eq!(regressed.passes, 2);
        assert_eq!(regressed.missed().len(), 1);
        // A faster machine never tightens targets.
        let fast = gate.run_with(
            true,
            || 50.0,
            || vec![Metric::timed("w_us", 9.0, Some(10.0))],
        );
        assert_eq!(fast.slowdowns, vec![1.0]);
        assert!(fast.missed().is_empty());
        // Extreme load: still a miss, flagged as inconclusive.
        let busy = gate.run_with(
            true,
            || 200.0,
            || vec![Metric::timed("w_us", 30.0, Some(10.0))],
        );
        assert!(busy.too_busy() && !regressed.too_busy());
    }

    #[test]
    fn exact_metrics_are_not_normalised_and_no_check_measures_once() {
        let gate = MicroGate::default();
        let report = gate.run_with(
            false,
            || 1e9,
            || vec![Metric::exact("bytes", 96.0, Some(64.0))],
        );
        assert_eq!(report.passes, 1);
        assert_eq!(report.normalized, vec![96.0]);
        assert_eq!(report.missed().len(), 1);
        let json = report.to_json(&[("release", "true".into())]);
        assert!(json.contains("\"bytes\": 96") && json.contains("\"release\": true"));
    }

    #[test]
    fn best_batch_timing_and_calibration_are_positive() {
        assert!(best_batch_micros(20, 4, || 1 + 1) >= 0.0);
        assert!(calibration_micros() > 0.0);
    }
}
