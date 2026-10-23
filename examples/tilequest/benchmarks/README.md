# Local sample measurements

These JSON files are one release-build run on Linux x86-64, Intel Core i7-10750H,
NVIDIA RTX 2070, Rust 1.94.0. They are observations on a desktop, not performance
thresholds. Regenerate with the commands in the example README.

The CPU comparison uses a fully occupied single layer and a moving 480×320 view.
Increasing stored cells from 10,000 to 1,000,000 still visits/emits at most 620 cells
per frame. Median culling/draw-list time was 25.8µs and 28.0µs respectively; parsing
and index construction increased from 1.1ms to 65.8ms. Decoding and GPU work are
excluded. Load cost still scales with stored content.

The native run presents 364 frames and measures the final 300. Autoplay advances
one simulation tick per attempted frame. It reports roughly 0.159ms median CPU
frame work and 0.036ms GPU frame work, five draw batches, and zero texture upload
bytes on the final frame. The measured interval had one resource creation and
one timeline wait, with no device-idle waits. Resource reuse can grow buffers as
visible content changes; this is not a claim of zero allocations. Native teardown
separately verified 57 resources created and 57 destroyed in this benchmark run.

Present mode is AutoNoVsync; completion polling permits overlap. These timings do
not measure input latency or the simulation, nor compare against another engine.
The CPU benchmark process and native benchmark ran sequentially. Driver scheduling,
clock speeds, compositors and background work can change these numbers.
