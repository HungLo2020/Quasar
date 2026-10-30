# Native wgpu comparison — 2026-10-30

Baseline: exact Quasar `7a82ff3` / 0.2.19, rebuilt from its source archive.
Native: 0.2.19 migration working tree. Linux 7.0, Rust 1.94.0, RTX 2070,
580.178.04 driver. Release/thin LTO, X11/Xvfb 1600×1000 without compositor,
GPU timestamps enabled. Driver debug layers disabled in both; upstream native
core validation unchanged. No neighboring repositories were involved.

Three alternating pairs per case. Nightwatch: 664 frames, 64 warmup, 600 measured,
960×540, seed 42, autoplay, 1024² shadows, normals/instancing/culling on, no audio.
Composition: 364 frames, 64 warmup, 300 measured. Framebuffer: 365 frames,
65 actual warmup, 300 measured (its historical reporting field says 64).

The following values are medians of the three per-run summaries, not percentiles
pooled across frames. [Summary](summary.json) and the adjacent 30 raw reports
retain means, p50/p95/p99, sample counts and allocation measurements.

| Workload / policy | Baseline CPU p50 ms | Native CPU p50 ms | Baseline CPU p99 ms | Native CPU p99 ms |
|---|---:|---:|---:|---:|
| Nightwatch / poll | 0.810 | 0.713 | 2.239 | 2.197 |
| Composition / wait | 14.715 | 9.434 | 35.545 | 11.101 |
| Composition / poll | 8.856 | 9.117 | 12.118 | 11.736 |
| Framebuffer / wait | 12.096 | 12.331 | 15.512 | 20.049 |
| Framebuffer / poll | 9.853 | 10.251 | 12.085 | 13.328 |

Nightwatch's measured CPU frame p50 improved about 12%, with identical hash
`0b4f2a0cb5a69ca3` in all six runs. Its prepare-plus-submit mean increased from
0.442 to 0.583 ms; the old/new boundaries include different frontend work, so a
lower overall frame time does not mean every stage improved. Draw counts agree.

Presentation dominates the small composition/framebuffer demos. Run-to-run
variation is large (composition wait p50 ranged 8.96–17.23 ms across both builds,
framebuffer wait 10.93–24.53 ms). These data do not establish a universal display
performance improvement. Polling composition/framebuffer p50 increased roughly
3%/4% in this run. GPU timings are command intervals, not directly comparable
total transfer times: native explicit framebuffer/staging copies are included,
while implicit queue writes can fall outside them. Do not interpret the lower
native GPU values as an equivalent end-to-end transfer speedup.

| Whole-process Rust allocations per measured frame | Baseline | Native |
|---|---:|---:|
| Composition, either policy | 77.23 | 270.57 |
| Framebuffer, either policy | 55.31 | 227.75 |

Upstream command construction, immutable staging snapshots and timestamp
instrumentation increase allocation count. This is a remaining cost of this
implementation. Framebuffer allocated bytes fall from about 1,034,814 to 31,770
per frame because the presenter borrows source pixels into native mapped staging
instead of copying a complete CPU framebuffer Vec. Composition allocated bytes
increase from about 22,444 to 49,741. These are Rust allocator observations,
not GPU allocation counts or equivalent private backend counters.

The first native port allocated destination buffers, bindings and shadow atlases
per frame. Its Nightwatch CPU p50 was 1.253 ms in an earlier run. The final native
path caches destination buffers/bindings, uses immutable inline copies and reuses
an explicit whole-frame shadow atlas. Comparison above uses that cached path.

## Reproduction

Build the baseline from an archive of the named Quasar revision into a separate
temporary directory; retain its release `nightwatch`, `quasar-composition` and
`framebuffer` binaries. Build the same three current binaries with
`cargo build --locked --release -p nightwatch -p quasar-composition -p framebuffer`.
Run the adjacent [comparison script](compare.py) with `QUASAR_BASELINE_BIN` pointing
to the baseline binaries and `XVFB` optionally pointing to Xvfb. It writes fresh
raw reports and a summary to a temporary directory. Run without competing Cargo
builds; this is a shared desktop measurement, not an isolated benchmark machine.
The adjacent [pixel comparison script](compare_pixels.py) uses the same
`QUASAR_BASELINE_BIN` setting; it verifies exact screenshots across six cases
and writes [pixel parity results](pixel-parity.json). It requires Python Pillow.
Native resource cleanup and correctness are tested separately in the
[validation report](../../native-wgpu-validation.md).
