//! Optional native-wgpu setup, presentation and readback helpers.
//! Renderers and applications can use the same upstream types without these helpers.
use std::{
    fmt,
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::Instant,
};
use vulkanic_gal::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphicsError(pub String);
impl GraphicsError {
    pub fn new(message: impl fmt::Display) -> Self {
        Self(message.to_string())
    }
}
impl fmt::Display for GraphicsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for GraphicsError {}
pub type GraphicsResult<T> = Result<T, GraphicsError>;
pub fn error(message: impl fmt::Display) -> GraphicsError {
    GraphicsError::new(message)
}
pub fn nanos(duration: std::time::Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

/// Native startup helper. It parks instead of spinning while asynchronous initialization wakes it.
pub fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// Convenience owner only. Fields are unmodified upstream objects and may be shared directly.
pub struct Gpu {
    pub instance: Instance,
    pub adapter: Adapter,
    pub device: Device,
    pub queue: Queue,
    timestamps: std::sync::atomic::AtomicBool,
}
impl Gpu {
    pub fn headless() -> GraphicsResult<Self> {
        let instance = Instance::new(InstanceDescriptor::new_without_display_handle_from_env());
        Self::request(instance, None)
    }
    pub fn request(instance: Instance, surface: Option<&Surface<'_>>) -> GraphicsResult<Self> {
        let adapter = block_on(util::initialize_adapter_from_env_or_default(
            &instance, surface,
        ))
        .map_err(error)?;
        // Optional capabilities are requested only when the adapter has them;
        // renderers check `device.features()` and fall back otherwise.
        let optional = Features::TIMESTAMP_QUERY
            | Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
            | Features::INDIRECT_FIRST_INSTANCE
            | Features::MULTI_DRAW_INDIRECT_COUNT;
        let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("Quasar shared device"),
            required_features: adapter.features() & optional,
            required_limits: adapter.limits(),
            experimental_features: ExperimentalFeatures::disabled(),
            memory_hints: MemoryHints::Performance,
            trace: Trace::Off,
        }))
        .map_err(|e| {
            error(format!(
                "requesting device on {} ({:?}): {e}",
                adapter.get_info().name,
                adapter.get_info().backend
            ))
        })?;
        Ok(Self::from_parts(instance, adapter, device, queue))
    }
    /// Adopt caller-created native objects without creating or translating a device.
    /// All four objects must belong to the same upstream instance/adapter/device.
    pub fn from_parts(instance: Instance, adapter: Adapter, device: Device, queue: Queue) -> Self {
        Self {
            instance,
            adapter,
            device,
            queue,
            timestamps: std::sync::atomic::AtomicBool::new(false),
        }
    }
    /// Enable optional per-frame native timestamp queries. Disabled for
    /// ordinary rendering. When on (and the adapter supports timestamp
    /// queries inside encoders, see [`Self::gpu_timestamps_supported`]),
    /// [`FrameReport::gpu_frame_nanos`] is filled for every frame, including
    /// headless [`OffscreenTarget::render`] benchmarks.
    pub fn set_gpu_timestamps(&self, enabled: bool) {
        self.timestamps
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }
    /// Whether the device can time frames (see [`Self::set_gpu_timestamps`]).
    pub fn gpu_timestamps_supported(&self) -> bool {
        self.device
            .features()
            .contains(Features::TIMESTAMP_QUERY | Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
    }
    /// Whether frames are currently timed.
    pub fn gpu_timestamps_enabled(&self) -> bool {
        self.timestamps.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn wait(&self) -> GraphicsResult<()> {
        self.device
            .poll(PollType::wait_indefinitely())
            .map(|_| ())
            .map_err(error)
    }
    pub fn encoder(&self, label: &str) -> CommandEncoder {
        self.device
            .create_command_encoder(&CommandEncoderDescriptor { label: Some(label) })
    }
}

/// Return upstream validation, internal and allocation errors through native scopes.
pub fn validation_scope<T>(gpu: &Gpu, operation: impl FnOnce() -> T) -> GraphicsResult<T> {
    let validation = gpu.device.push_error_scope(ErrorFilter::Validation);
    let internal = gpu.device.push_error_scope(ErrorFilter::Internal);
    let allocation = gpu.device.push_error_scope(ErrorFilter::OutOfMemory);
    let value = operation();
    let allocation_error = block_on(allocation.pop());
    let internal_error = block_on(internal.pop());
    let validation_error = block_on(validation.pop());
    if let Some(e) = allocation_error.or(internal_error).or(validation_error) {
        Err(error(e))
    } else {
        Ok(value)
    }
}

/// Grow a native COPY_DST buffer without invalidating references held by older commands.
/// Returns true when bindings must be recreated. Callers encode their writes in command order.
pub fn grow_buffer(
    gpu: &Gpu,
    slot: &mut Option<Buffer>,
    label: &str,
    size: u64,
    usage: BufferUsages,
) -> GraphicsResult<bool> {
    let limit = if usage.contains(BufferUsages::STORAGE) {
        gpu.device
            .limits()
            .max_storage_buffer_binding_size
            .min(gpu.device.limits().max_buffer_size)
    } else {
        gpu.device.limits().max_buffer_size
    };
    if size == 0 || !size.is_multiple_of(COPY_BUFFER_ALIGNMENT) || size > limit {
        return Err(error("buffer size exceeds native limits or copy alignment"));
    }
    let usage = usage | BufferUsages::COPY_DST;
    if slot
        .as_ref()
        .is_some_and(|b| b.size() >= size && b.usage() == usage)
    {
        return Ok(false);
    }
    let aligned_limit = limit / COPY_BUFFER_ALIGNMENT * COPY_BUFFER_ALIGNMENT;
    let capacity = size
        .checked_next_power_of_two()
        .unwrap_or(size)
        .min(aligned_limit);
    *slot = Some(gpu.device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size: capacity,
        usage,
        mapped_at_creation: false,
    }));
    Ok(true)
}
/// Immutable staging data retained by native command buffers. No submission receipts.
pub fn staging_buffer(gpu: &Gpu, label: &str, data: &[u8]) -> Buffer {
    use vulkanic_gal::util::DeviceExt;
    gpu.device.create_buffer_init(&util::BufferInitDescriptor {
        label: Some(label),
        contents: data,
        usage: BufferUsages::MAP_WRITE | BufferUsages::COPY_SRC,
    })
}

/// Encode an RGBA8 upload in command order, including WebGPU row alignment.
/// Unlike Queue::write_texture this transfer is inside caller timestamp intervals.
pub fn upload_rgba8(
    gpu: &Gpu,
    encoder: &mut CommandEncoder,
    texture: &Texture,
    pixels: &[u8],
) -> GraphicsResult<()> {
    let row = texture
        .width()
        .checked_mul(4)
        .ok_or_else(|| error("upload size overflow"))?;
    if texture.dimension() != TextureDimension::D2
        || texture.depth_or_array_layers() != 1
        || texture.sample_count() != 1
        || texture.mip_level_count() != 1
        || !matches!(
            texture.format(),
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
        )
        || !texture.usage().contains(TextureUsages::COPY_DST)
        || pixels.len() as u64 != u64::from(row) * u64::from(texture.height())
    {
        return Err(error(
            "upload requires matching RGBA8 pixels and a single-layer COPY_DST texture",
        ));
    }
    let stride = row.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT;
    let size = u64::from(stride) * u64::from(texture.height());
    if size > gpu.device.limits().max_buffer_size || size > 256 * 1024 * 1024 {
        return Err(error(
            "texture staging upload exceeds native limits or 256 MiB budget",
        ));
    }
    let padded;
    let bytes = if stride == row {
        pixels
    } else {
        padded = pixels
            .chunks_exact(row as usize)
            .flat_map(|r| {
                r.iter()
                    .copied()
                    .chain(std::iter::repeat_n(0, (stride - row) as usize))
            })
            .collect::<Vec<_>>();
        &padded
    };
    let source = staging_buffer(gpu, "RGBA8 upload", bytes);
    encoder.copy_buffer_to_texture(
        TexelCopyBufferInfo {
            buffer: &source,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(texture.height()),
            },
        },
        texture.as_image_copy(),
        texture.size(),
    );
    Ok(())
}

/// Explicit attachment views and dimensions. No registry or synthetic resource identity.
#[derive(Debug, Clone)]
pub struct RenderTarget {
    pub color: TextureView,
    pub depth: Option<TextureView>,
    pub format: TextureFormat,
    pub width: u32,
    pub height: u32,
}
impl RenderTarget {
    pub fn from_texture(texture: &Texture) -> Self {
        Self {
            color: texture.create_view(&TextureViewDescriptor::default()),
            depth: None,
            format: texture.format(),
            width: texture.width(),
            height: texture.height(),
        }
    }
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
pub fn texture_2d(
    device: &Device,
    label: &str,
    width: u32,
    height: u32,
    format: TextureFormat,
    usage: TextureUsages,
) -> GraphicsResult<Texture> {
    let limit = device.limits().max_texture_dimension_2d;
    if width == 0 || height == 0 || width > limit || height > limit {
        return Err(error("texture dimensions exceed device limits"));
    }
    Ok(device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    }))
}
/// Resolves asynchronously when the caller polls the native device. It belongs to one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTiming(std::sync::Arc<std::sync::OnceLock<Option<u64>>>);
impl GpuTiming {
    pub fn nanos(&self) -> Option<u64> {
        self.0.get().copied().flatten()
    }
}
struct TimestampReadback {
    queries: QuerySet,
    resolve: Buffer,
    read: Buffer,
    timing: GpuTiming,
}
impl TimestampReadback {
    fn begin(gpu: &Gpu, commands: &mut Vec<CommandBuffer>) -> Option<Self> {
        if !gpu.timestamps.load(std::sync::atomic::Ordering::Relaxed)
            || !gpu
                .device
                .features()
                .contains(Features::TIMESTAMP_QUERY | Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
        {
            return None;
        }
        let queries = gpu.device.create_query_set(&QuerySetDescriptor {
            label: Some("frame timestamps"),
            ty: QueryType::Timestamp,
            count: 2,
        });
        let resolve = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("timestamp resolve"),
            size: 16,
            usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("timestamp readback"),
            size: 16,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut first = gpu.encoder("frame timestamp start");
        first.write_timestamp(&queries, 0);
        commands.insert(0, first.finish());
        let mut last = gpu.encoder("frame timestamp end");
        last.write_timestamp(&queries, 1);
        last.resolve_query_set(&queries, 0..2, &resolve, 0);
        last.copy_buffer_to_buffer(&resolve, 0, &read, 0, 16);
        commands.push(last.finish());
        Some(Self {
            queries,
            resolve,
            read,
            timing: GpuTiming(Default::default()),
        })
    }
    fn map(self, gpu: &Gpu) -> GpuTiming {
        let timing = self.timing.clone();
        let result = self.timing.0.clone();
        let buffer = self.read.clone();
        let period = gpu.queue.get_timestamp_period() as f64;
        self.read.map_async(MapMode::Read, .., move |mapped| {
            let nanos = mapped.ok().and_then(|()| {
                let bytes = buffer.get_mapped_range(..).ok()?;
                let start = u64::from_le_bytes(bytes[..8].try_into().ok()?);
                let end = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
                let elapsed = end.wrapping_sub(start) as f64 * period;
                drop(bytes);
                buffer.unmap();
                Some(elapsed.min(u64::MAX as f64) as u64)
            });
            let _ = result.set(nanos);
        });
        // Native submitted work retains the query and resolve resources.
        drop((self.queries, self.resolve));
        timing
    }
}
#[derive(Debug, Default, Clone)]
pub struct FrameReport {
    pub prepare_nanos: u64,
    pub submit_nanos: u64,
    pub acquire_nanos: u64,
    pub present_nanos: u64,
    pub wait_nanos: u64,
    pub cpu_frame_nanos: u64,
    pub blocking_waits: u64,
    /// GPU time of the frame's command buffers. `None` unless timestamp
    /// queries are on: call [`Gpu::set_gpu_timestamps`]`(true)` (off by
    /// default, so ordinary rendering pays nothing) on a GPU that supports
    /// `TIMESTAMP_QUERY` and `TIMESTAMP_QUERY_INSIDE_ENCODERS`. Applies to
    /// surface frames and [`OffscreenTarget::render`] alike.
    pub gpu_frame_nanos: Option<u64>,
    /// Raw timestamps behind [`Self::gpu_frame_nanos`].
    pub gpu_timing: Option<GpuTiming>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionPolicy {
    Wait,
    Poll,
}
pub fn record_and_submit(
    gpu: &Gpu,
    prepare: impl FnOnce(&Gpu) -> GraphicsResult<Vec<CommandBuffer>>,
) -> GraphicsResult<(SubmissionIndex, FrameReport)> {
    let started = Instant::now();
    let mut commands = prepare(gpu)?;
    let timestamps = TimestampReadback::begin(gpu, &mut commands);
    let prepare_nanos = nanos(started.elapsed());
    let submit = Instant::now();
    let index = gpu.queue.submit(commands);
    let gpu_timing = timestamps.map(|t| t.map(gpu));
    Ok((
        index,
        FrameReport {
            gpu_timing,
            prepare_nanos,
            submit_nanos: nanos(submit.elapsed()),
            cpu_frame_nanos: nanos(started.elapsed()),
            ..Default::default()
        },
    ))
}

/// A CPU-readable owned color target, with a depth attachment for 3D composition.
///
/// [`Self::render`] reuses one readback buffer across frames. Its report
/// carries GPU time when [`Gpu::set_gpu_timestamps`] is on.
pub struct OffscreenTarget {
    pub texture: Texture,
    pub target: RenderTarget,
    _depth: Texture,
    readback: Buffer,
}
pub struct OffscreenReport {
    pub rgba: Vec<u8>,
    pub profile: FrameReport,
    pub cpu_nanos: u64,
}
impl OffscreenTarget {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> GraphicsResult<Self> {
        let texture = texture_2d(
            &gpu.device,
            "offscreen color",
            width,
            height,
            TextureFormat::Rgba8Unorm,
            TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST,
        )?;
        let depth = texture_2d(
            &gpu.device,
            "offscreen depth",
            width,
            height,
            TextureFormat::Depth32Float,
            TextureUsages::RENDER_ATTACHMENT,
        )?;
        let mut target = RenderTarget::from_texture(&texture);
        target.depth = Some(depth.create_view(&Default::default()));
        let readback = readback_buffer(gpu, &texture)?;
        Ok(Self {
            texture,
            target,
            _depth: depth,
            readback,
        })
    }
    pub fn release(self, _gpu: &Gpu) -> GraphicsResult<()> {
        Ok(())
    }
    pub fn target(&self) -> &RenderTarget {
        &self.target
    }
    pub fn size(&self) -> (u32, u32) {
        self.target.size()
    }
    /// Renders one frame, waits for it and reads the pixels back.
    /// `profile.gpu_frame_nanos` is set when [`Gpu::set_gpu_timestamps`] is
    /// on and supported.
    pub fn render(
        &self,
        gpu: &Gpu,
        prepare: impl FnOnce(&Gpu, &RenderTarget) -> GraphicsResult<Vec<CommandBuffer>>,
    ) -> GraphicsResult<OffscreenReport> {
        let start = Instant::now();
        let (_, mut profile) = record_and_submit(gpu, |gpu| prepare(gpu, &self.target))?;
        let rgba = read_rgba_with(gpu, &self.texture, &self.readback)?;
        profile.gpu_frame_nanos = profile.gpu_timing.as_ref().and_then(GpuTiming::nanos);
        profile.cpu_frame_nanos = nanos(start.elapsed());
        Ok(OffscreenReport {
            rgba,
            cpu_nanos: profile.cpu_frame_nanos,
            profile,
        })
    }
}
/// Tightly packed RGBA8 pixels read back from an offscreen render.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Row-major, top row first, 4 bytes per pixel.
    pub rgba: Vec<u8>,
}

impl Image {
    /// Wraps packed RGBA8 data, checking its length.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> GraphicsResult<Self> {
        if u64::from(width) * u64::from(height) * 4 != rgba.len() as u64 {
            return Err(error("RGBA length does not match image dimensions"));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
    /// The pixel at `(x, y)`, or `None` outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let start = (y as usize * self.width as usize + x as usize) * 4;
        self.rgba
            .get(start..start + 4)
            .map(|p| [p[0], p[1], p[2], p[3]])
    }
    /// Encodes the image as a PNG.
    #[cfg(feature = "png")]
    pub fn encode_png(&self) -> GraphicsResult<Vec<u8>> {
        if self.width == 0 || self.height == 0 {
            return Err(error("PNG dimensions must be nonzero"));
        }
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&self.rgba))
            .map_err(|e| error(format!("PNG encode: {e}")))?;
        Ok(bytes)
    }
    /// Writes the image as a PNG file.
    #[cfg(feature = "png")]
    pub fn write_png(&self, path: impl AsRef<std::path::Path>) -> GraphicsResult<()> {
        let bytes = self.encode_png()?;
        std::fs::write(path.as_ref(), bytes)
            .map_err(|e| error(format!("{}: {e}", path.as_ref().display())))
    }
}

/// Renders one frame into a temporary RGBA8 + depth target and reads it
/// back: the screenshot/cover-render/golden-image boilerplate in one call.
///
/// `prepare` records command buffers against the target exactly as a live
/// frame would (2D canvas passes, 3D passes, raw GAL work); they are
/// submitted, waited on and read back before the target is released.
///
/// ```no_run
/// # use quasar_frame::{Gpu, render_offscreen};
/// let gpu = Gpu::headless()?;
/// let image = render_offscreen(&gpu, 320, 180, |gpu, target| {
///     // record passes that draw into `target` and return their buffers
///     Ok(Vec::new())
/// })?;
/// assert_eq!(image.rgba.len(), 320 * 180 * 4);
/// # Ok::<(), quasar_frame::GraphicsError>(())
/// ```
pub fn render_offscreen(
    gpu: &Gpu,
    width: u32,
    height: u32,
    prepare: impl FnOnce(&Gpu, &RenderTarget) -> GraphicsResult<Vec<CommandBuffer>>,
) -> GraphicsResult<Image> {
    let target = OffscreenTarget::new(gpu, width, height)?;
    let report = target.render(gpu, prepare)?;
    target.release(gpu)?;
    Image::new(width, height, report.rgba)
}

/// Readback is explicit and padded for WebGPU copy alignment. Live rendering never calls this.
pub fn read_rgba(gpu: &Gpu, texture: &Texture) -> GraphicsResult<Vec<u8>> {
    let buffer = readback_buffer(gpu, texture)?;
    read_rgba_with(gpu, texture, &buffer)
}

fn readback_layout(texture: &Texture) -> GraphicsResult<(u32, u32, u64)> {
    if !matches!(
        texture.format(),
        TextureFormat::Rgba8Unorm
            | TextureFormat::Rgba8UnormSrgb
            | TextureFormat::Bgra8Unorm
            | TextureFormat::Bgra8UnormSrgb
    ) || !texture.usage().contains(TextureUsages::COPY_SRC)
    {
        return Err(error("readback requires a COPY_SRC RGBA8/BGRA8 texture"));
    }
    let row = texture
        .width()
        .checked_mul(4)
        .ok_or_else(|| error("readback size overflow"))?;
    let stride = row.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT;
    let size = u64::from(stride) * u64::from(texture.height());
    if size > 256 * 1024 * 1024 {
        return Err(error("readback exceeds 256 MiB budget"));
    }
    Ok((row, stride, size))
}

fn readback_buffer(gpu: &Gpu, texture: &Texture) -> GraphicsResult<Buffer> {
    let (_, _, size) = readback_layout(texture)?;
    Ok(gpu.device.create_buffer(&BufferDescriptor {
        label: Some("RGBA readback"),
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    }))
}

/// [`read_rgba`] into a caller-owned readback buffer (at least the padded
/// texture size, `COPY_DST | MAP_READ`), so repeated reads allocate nothing
/// on the GPU. Rows are copied whole and BGRA is swizzled four bytes at a
/// time.
fn read_rgba_with(gpu: &Gpu, texture: &Texture, buffer: &Buffer) -> GraphicsResult<Vec<u8>> {
    let (row, stride, size) = readback_layout(texture)?;
    if buffer.size() < size {
        return Err(error("readback buffer is too small"));
    }
    let mut encoder = gpu.encoder("RGBA readback");
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        TexelCopyBufferInfo {
            buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(texture.height()),
            },
        },
        texture.size(),
    );
    let index = gpu.queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::sync_channel(1);
    let slice = buffer.slice(..size);
    slice.map_async(MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    gpu.device
        .poll(PollType::Wait {
            submission_index: Some(index),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .map_err(error)?;
    recv.recv_timeout(std::time::Duration::from_secs(30))
        .map_err(error)?
        .map_err(error)?;
    let mapped = slice.get_mapped_range().map_err(error)?;
    let (row, stride) = (row as usize, stride as usize);
    let mut rgba: Vec<u8> = Vec::with_capacity(row * texture.height() as usize);
    if row == stride {
        rgba.extend_from_slice(&mapped[..row * texture.height() as usize]);
    } else {
        for line in mapped.chunks_exact(stride) {
            rgba.extend_from_slice(&line[..row]);
        }
    }
    drop(mapped);
    buffer.unmap();
    if matches!(
        texture.format(),
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    ) {
        swap_red_blue(&mut rgba);
    }
    Ok(rgba)
}

/// BGRA8 <-> RGBA8 in place, one `u32` per pixel (vectorises).
fn swap_red_blue(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        let value = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
        let swapped = (value & 0xff00_ff00) | ((value >> 16) & 0xff) | ((value & 0xff) << 16);
        pixel.copy_from_slice(&swapped.to_le_bytes());
    }
}

pub struct SurfaceFrame {
    pub target: RenderTarget,
    texture: SurfaceTexture,
    started: Instant,
    acquire_nanos: u64,
    _lease: Arc<()>,
}
type SurfaceFactory = Box<dyn Fn(&Instance) -> GraphicsResult<Surface<'static>>>;
/// Native surface lifecycle with safe window retention and explicit recovery.
pub struct FrameSurface {
    pub surface: Surface<'static>,
    pub config: SurfaceConfiguration,
    recreate: SurfaceFactory,
    size: (u32, u32),
    dirty: bool,
    depth: Option<Texture>,
    lease: std::sync::Weak<()>,
}
impl FrameSurface {
    pub fn for_window<W>(window: Arc<W>, width: u32, height: u32) -> GraphicsResult<(Gpu, Self)>
    where
        W: DisplayAndWindowHandle + fmt::Debug + Send + Sync + 'static,
    {
        Self::for_window_with_descriptor(
            window,
            width,
            height,
            InstanceDescriptor::new_without_display_handle_from_env(),
        )
    }
    pub fn for_window_with_descriptor<W>(
        window: Arc<W>,
        width: u32,
        height: u32,
        mut descriptor: InstanceDescriptor,
    ) -> GraphicsResult<(Gpu, Self)>
    where
        W: DisplayAndWindowHandle + fmt::Debug + Send + Sync + 'static,
    {
        descriptor.display = Some(Box::new(window.clone()));
        let instance = Instance::new(descriptor);
        let surface = instance.create_surface(window.clone()).map_err(error)?;
        let gpu = Gpu::request(instance, Some(&surface))?;
        let mut config = surface
            .get_default_config(&gpu.adapter, width.max(1), height.max(1))
            .ok_or_else(|| error("surface has no supported configuration"))?;
        let caps = surface.get_capabilities(&gpu.adapter);
        config.format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, TextureFormat::Bgra8Unorm | TextureFormat::Rgba8Unorm))
            .unwrap_or(config.format);
        config.present_mode = PresentMode::AutoVsync;
        config.desired_maximum_frame_latency = 3;
        let mut result = Self {
            surface,
            config,
            recreate: Box::new(move |instance| {
                instance.create_surface(window.clone()).map_err(error)
            }),
            size: (width, height),
            dirty: true,
            depth: None,
            lease: std::sync::Weak::new(),
        };
        result.configure(&gpu)?;
        Ok((gpu, result))
    }
    fn configure(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        if self.lease.strong_count() != 0 {
            return Err(error(
                "drop or present the acquired frame before reconfiguring",
            ));
        }
        if self.size.0 == 0 || self.size.1 == 0 {
            return Ok(());
        }
        self.config.width = self.size.0;
        self.config.height = self.size.1;
        self.surface.configure(&gpu.device, &self.config);
        self.depth = Some(texture_2d(
            &gpu.device,
            "surface depth",
            self.size.0,
            self.size.1,
            TextureFormat::Depth32Float,
            TextureUsages::RENDER_ATTACHMENT,
        )?);
        self.dirty = false;
        Ok(())
    }
    pub fn reconfigure(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        self.dirty = true;
        self.configure(gpu)
    }
    pub fn render(
        &mut self,
        gpu: &Gpu,
        policy: CompletionPolicy,
        prepare: impl FnOnce(&Gpu, &RenderTarget) -> GraphicsResult<Vec<CommandBuffer>>,
    ) -> GraphicsResult<Option<FrameReport>> {
        let Some(frame) = self.acquire(gpu)? else {
            return Ok(None);
        };
        let start = Instant::now();
        let commands = prepare(gpu, &frame.target)?;
        let prepare_nanos = nanos(start.elapsed());
        let mut report = self.present(gpu, frame, commands, policy)?;
        report.prepare_nanos = prepare_nanos;
        Ok(Some(report))
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.size != (width, height) {
            self.size = (width, height);
            self.dirty = true;
        }
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    pub fn needs_resize(&self) -> bool {
        self.dirty
    }
    pub fn acquire(&mut self, gpu: &Gpu) -> GraphicsResult<Option<SurfaceFrame>> {
        if self.size.0 == 0 || self.size.1 == 0 {
            return Ok(None);
        }
        if self.lease.strong_count() != 0 {
            return Err(error("a surface frame is already acquired"));
        }
        if self.dirty {
            self.configure(gpu)?;
        }
        let start = Instant::now();
        let texture = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(t) => t,
            CurrentSurfaceTexture::Suboptimal(t) => {
                self.dirty = true;
                t
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => return Ok(None),
            CurrentSurfaceTexture::Outdated => {
                self.dirty = true;
                return Ok(None);
            }
            CurrentSurfaceTexture::Lost => {
                self.surface = (self.recreate)(&gpu.instance)?;
                self.dirty = true;
                return Ok(None);
            }
            CurrentSurfaceTexture::Validation => {
                return Err(error("surface acquisition validation failure"));
            }
        };
        let mut target = RenderTarget::from_texture(&texture.texture);
        target.depth = self
            .depth
            .as_ref()
            .map(|t| t.create_view(&Default::default()));
        let lease = Arc::new(());
        self.lease = Arc::downgrade(&lease);
        Ok(Some(SurfaceFrame {
            target,
            texture,
            started: start,
            acquire_nanos: nanos(start.elapsed()),
            _lease: lease,
        }))
    }
    pub fn present(
        &mut self,
        gpu: &Gpu,
        frame: SurfaceFrame,
        mut commands: Vec<CommandBuffer>,
        policy: CompletionPolicy,
    ) -> GraphicsResult<FrameReport> {
        let timestamps = TimestampReadback::begin(gpu, &mut commands);
        let started = Instant::now();
        let index = gpu.queue.submit(commands);
        let submit_nanos = nanos(started.elapsed());
        let gpu_timing = timestamps.map(|t| t.map(gpu));
        let present = Instant::now();
        gpu.queue.present(frame.texture);
        let present_nanos = nanos(present.elapsed());
        let wait = Instant::now();
        match policy {
            CompletionPolicy::Wait => {
                gpu.device
                    .poll(PollType::Wait {
                        submission_index: Some(index),
                        timeout: Some(std::time::Duration::from_secs(30)),
                    })
                    .map_err(error)?;
            }
            CompletionPolicy::Poll => {
                gpu.device.poll(PollType::Poll).map_err(error)?;
            }
        }
        let gpu_frame_nanos = gpu_timing.as_ref().and_then(GpuTiming::nanos);
        Ok(FrameReport {
            gpu_timing,
            gpu_frame_nanos,
            blocking_waits: u64::from(policy == CompletionPolicy::Wait),
            submit_nanos,
            present_nanos,
            acquire_nanos: frame.acquire_nanos,
            wait_nanos: nanos(wait.elapsed()),
            cpu_frame_nanos: nanos(frame.started.elapsed()),
            ..Default::default()
        })
    }
    pub fn cancel(&mut self, frame: SurfaceFrame) {
        drop(frame);
    }
    pub fn finish(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        gpu.wait()
    }
}
