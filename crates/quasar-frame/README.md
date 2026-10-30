# quasar-frame

Optional setup, presentation, frame timing and explicit GPU readback helpers
using native wgpu types imported through VulkanicGAL. `Gpu` owns public native
`instance`, `adapter`, `device` and `queue` fields. `Gpu::headless()` uses upstream
adapter selection and `WGPU_BACKEND`/`WGPU_ADAPTER_NAME`; `Gpu::request` accepts a
caller-created native instance and optional compatible surface. `Gpu::from_parts`
adopts an existing native instance, adapter, device and queue, preserving any
features requested by the application. Default setup requests only optional
timestamp features beyond ordinary device functionality. No global device
lock exists. Multiple devices/instances can coexist.

`RenderTarget` contains native color/depth views, format and dimensions. Both
renderers accept the same target and device. Prepare native command buffers,
order them with application-created buffers, and submit with `gpu.queue.submit`.
`record_and_submit` is optional convenience; encoding can also be fully manual.

`FrameSurface::for_window(Arc<Window>, width, height)` returns `(Gpu, FrameSurface)`.
`for_window_with_descriptor` additionally accepts native instance options.
The owned window stays alive through surface recreation. `render` acquires,
prepares and presents; `acquire`/`present` exposes each step. Drop an unpresented
`SurfaceFrame` to abandon it. Resize/reconfiguration rejects a still-acquired
frame. Zero size and timeout/occlusion defer presentation; outdated/lost surfaces
recover on a later frame. Suboptimal images are presented before reconfiguration.

`CompletionPolicy::Poll` submits without waiting for GPU completion.
`CompletionPolicy::Wait` waits for the specific submitted work, with a 30 second
timeout. The native queue and reference-counted resources govern lifetime.
`finish` waits for outstanding work; dropping owners releases their references.
Native command buffers are single-use and are not cloned or reused.

`OffscreenTarget::new` owns an RGBA8 color texture and a Depth32Float attachment.
`render` invokes the same renderer APIs and returns real GPU pixels. `read_rgba`
handles 256-byte copy-row alignment and BGRA conversion, bounds readback to
256 MiB, and waits explicitly. Live presentation never reads pixels back.

`FrameReport` separates CPU preparation, submit, acquire, present and poll/wait
costs. `blocking_waits` counts actual requested waits, not poll calls. GPU queries
are opt-in with `gpu.set_gpu_timestamps(true)` and require upstream timestamp
features. The `gpu_timing` handle stays associated with its frame and resolves
when the native device is polled; synchronous reports also expose
`gpu_frame_nanos`. `quasar-profile` resolves asynchronous handles when computing
summaries. The interval covers the submitted command buffers, including explicit
staging copies; implicit `Queue::write_*` transfers precede it and are excluded.
Unsupported timings remain absent. Backend-private validation and
allocator counters are not invented; native allocator reports are optional.
