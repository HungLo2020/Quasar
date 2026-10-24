# quasar-frame

Small optional frame helpers extracted after the explicit cube/HUD composition
worked. This crate depends only on VulkanicGAL. Renderers remain usable without
it. It owns no GAL, window, game loop, scene, renderer registry or render graph.
These APIs were introduced in Git tag `2.1`. The current release is `v0.2.7`,
with Cargo package versions set to `0.2.7`. This crate does not select a backend
by default; enable its `vulkan` or `opengl` forwarding feature when the same
dependency needs a backend, or select one through the renderer crate.

`record_and_submit(&mut gal, closure)` opens GAL's existing nestable command
recording scope. The closure prepares an ordered `Vec<CommandList>` on that GAL.
The helper submits it, closes the scope on success/error (also on panic unwind),
and returns the submission token plus CPU/GPU submission profiling fields.
Creation/destruction deltas cover preparation through scope closure. It does
not implement another resource lifetime system. Use renderer-provided lifetime
receipts when retaining prepared commands. Explicitly destroyed custom resources
must not escape the transaction through retained command copies.

The optional `diagnostics` feature lets an explicit `quasar_diagnostics` handle
receive resize/presentation lifecycle records; it is absent from default builds.

`FrameSurface` tracks an already configured GAL presentation surface. It caches
frame-target wrappers by acquired-image identity and retires them on resize or
cancellation. Pass caches remain with the renderer that created them. Forward
physical pixel sizes with `resize`; zero sizes defer until restoration.

`render(&mut gal, policy, |gal, target| ...)` acquires once, records/submits ordered
passes, presents once, then retires according to `CompletionPolicy::Wait` or
`Poll`. Failed preparation/submission/presentation cancels the acquired frame.
The individual `acquire`, `present` and `cancel` operations are also public for
applications that need explicit orchestration.

Poll mode reclaims already completed work. Backends can still block for image
acquisition or bounded pressure; resize/cancel/shutdown may quiesce the device.
Frontend allocators must protect their pending and submitted ranges. Polling
alone cannot make arbitrary buffer reuse safe. Configuring frames in flight
remains the caller's responsibility when constructing/configuring GAL.

Call `finish` when draining a session, release renderer caches, then
`release_targets`; shut down the surface through the owning GAL. Every operation
must use the same GAL. Surface access is exclusive; this is not a multithreaded
GAL-sharing wrapper.

`take_completion_latencies` returns submission-to-host-observed-GPU-completion
samples. `take_frame_completion_latencies` includes the engine path from
acquire entry through observed GPU completion. Poll cadence influences both. They exclude input, compositor and display
latency. Uncollected completed samples retain only the newest 256 entries.
See [the example and measurements](../../examples/composition/README.md).
