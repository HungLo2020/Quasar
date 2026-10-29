use crate::{
    Canvas, DisplayOptions, Filter, PixelState, Rect, Sprite, TextureId, Textures, Viewport,
};
use quasar_frame::{CompletionPolicy, FrameSurface};
#[cfg(any(feature = "vulkan", feature = "wgpu"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(any(feature = "vulkan", feature = "wgpu"))]
use std::sync::Arc;
use vulkanic_gal::{commands::SubmissionUsage, metrics::BackendRuntimeMetrics, resources::*, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    Presented,
    Deferred,
}
/// A persistent, dynamically updated RGBA8 texture owned by a `RenderState2D`.
/// The GPU texture, view, samplers and resource bindings are created once; only
/// the pixel contents are uploaded per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynamicTexture {
    id: TextureId,
    size: (u32, u32),
}
impl DynamicTexture {
    /// Texture-store identity used by `Canvas`/`Sprite` draw commands.
    pub fn id(self) -> TextureId {
        self.id
    }
    /// Pixel dimensions of the persistent texture.
    pub fn size(self) -> (u32, u32) {
        self.size
    }
}
/// Whether a 2D pass clears its target or preserves the existing image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOp {
    Clear,
    Load,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderStats {
    pub quads: usize,
    pub batches: usize,
    pub geometry_upload_bytes: usize,
    pub texture_upload_bytes: usize,
}
/// Owned GAL commands. Keeping this value (or a command-list clone) reserves
/// its buffers. Dropping unsubmitted commands abandons the reservation.
#[derive(Debug, Clone)]
pub struct Prepared2D {
    pub commands: CommandList,
    pub stats: RenderStats,
}
struct TextureGpu {
    texture: Option<Handle>,
    view: Handle,
    samplers: [Handle; 2],
    combined: [Handle; 2],
    dynamic: bool,
    initialized: bool,
}
struct Slot {
    geometry: Handle,
    uniform: Handle,
    upload: Handle,
    geometry_capacity: usize,
    uniform_capacity: usize,
    upload_capacity: usize,
    sets: BTreeMap<(TextureId, TextureId), [Handle; 2]>,
    mask_targets: BTreeMap<Handle, (Handle, Handle, Handle)>,
    passes: BTreeMap<Handle, Handle>,
    usage: SubmissionUsage,
}
struct WindowSurface {
    frame: FrameSurface,
    options: DisplayOptions,
}
fn window_surface_desc(size: (u32, u32)) -> FrameSurfaceDesc {
    FrameSurfaceDesc {
        label: "2d-surface".into(),
        extent: extent(size),
        color_format: TextureFormat::Bgra8Unorm,
        present_mode: PresentMode::AutoVsync,
        max_frames_in_flight: 3,
    }
}
struct Batch {
    texture: TextureId,
    palette: TextureId,
    pixels: PixelState,
    filter: Filter,
    first: u32,
    count: u32,
}
/// Standalone convenience owner. Existing games keep their original API.
pub struct Renderer2D {
    gal: VulkanicGal,
    state: RenderState2D,
    surface: Option<WindowSurface>,
    frame_overlap: bool,
}
impl Renderer2D {
    /// Creates a headless renderer on the default backend: native Vulkan
    /// first, then wgpu, then OpenGL, among those compiled in.
    /// `VULKANIC_BACKEND` overrides the choice.
    pub fn headless() -> GalResult<Self> {
        Self::headless_with_options(GalOptions::default())
    }
    /// Creates a headless renderer on the backend `options` selects.
    pub fn headless_with_options(options: GalOptions) -> GalResult<Self> {
        Self::from_gal(VulkanicGal::create_with(options, "Quasar 2D offscreen")?)
    }
    /// Creates a headless renderer on an explicitly requested backend.
    pub fn headless_with(backend: BackendChoice) -> GalResult<Self> {
        Self::from_gal(VulkanicGal::create(backend, "Quasar 2D offscreen")?)
    }
    pub fn from_gal(mut gal: VulkanicGal) -> GalResult<Self> {
        let state = RenderState2D::new(&mut gal)?;
        Ok(Self {
            gal,
            state,
            surface: None,
            frame_overlap: false,
        })
    }
    /// Enables completion polling and bounded multi-frame resource reuse.
    /// The default retains synchronous native retirement for existing clients.
    /// Readback always waits regardless of this presentation policy.
    pub fn set_frame_overlap(&mut self, enabled: bool) {
        self.frame_overlap = enabled;
    }
    /// Wraps a caller-created, already-windowed GAL with a 2D surface.
    ///
    /// This is the backend-neutral windowed constructor. The caller owns
    /// context creation and presentation (including OpenGL buffer swaps).
    pub fn from_windowed_gal(
        mut gal: VulkanicGal,
        size: (u32, u32),
        options: DisplayOptions,
    ) -> GalResult<Self> {
        options.validate().map_err(GalError::invalid_argument)?;
        gal.configure_frame_surface(window_surface_desc(size))?;
        Self::with_window_surface(gal, size, options)
    }
    /// Creates a renderer on a window on the default backend: native Vulkan
    /// when available, otherwise wgpu. `VULKANIC_BACKEND` overrides the
    /// choice.
    #[cfg(any(feature = "vulkan", feature = "wgpu"))]
    pub fn for_window<W: HasDisplayHandle + HasWindowHandle + 'static>(
        window: Arc<W>,
        size: (u32, u32),
        options: DisplayOptions,
    ) -> GalResult<Self> {
        Self::for_window_with(window, size, options, GalOptions::default())
    }
    /// Creates a renderer on a window on the backend `backend` selects, for
    /// games that let players choose (see `GraphicsBackend::selectable`).
    #[cfg(any(feature = "vulkan", feature = "wgpu"))]
    pub fn for_window_with<W: HasDisplayHandle + HasWindowHandle + 'static>(
        window: Arc<W>,
        size: (u32, u32),
        options: DisplayOptions,
        backend: GalOptions,
    ) -> GalResult<Self> {
        options.validate().map_err(GalError::invalid_argument)?;
        let gal = VulkanicGal::create_windowed_with(
            backend,
            "Quasar 2D",
            window,
            window_surface_desc(size),
        )?;
        Self::with_window_surface(gal, size, options)
    }
    fn with_window_surface(
        gal: VulkanicGal,
        size: (u32, u32),
        options: DisplayOptions,
    ) -> GalResult<Self> {
        let mut renderer = Self::from_gal(gal)?;
        renderer.surface = Some(WindowSurface {
            frame: FrameSurface::new(size.0, size.1),
            options,
        });
        Ok(renderer)
    }
    pub fn options(&self) -> DisplayOptions {
        self.surface.as_ref().map(|s| s.options).unwrap_or_default()
    }
    pub fn set_options(&mut self, options: DisplayOptions) -> GalResult<()> {
        options.validate().map_err(GalError::invalid_argument)?;
        if let Some(surface) = &mut self.surface {
            surface.options = options;
        }
        Ok(())
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(surface) = &mut self.surface {
            surface.frame.resize(width, height);
        }
    }
    pub fn stats(&self) -> RenderStats {
        self.state.stats
    }
    pub fn dynamic_texture_count(&self) -> usize {
        self.state.dynamic_texture_count()
    }
    /// Creates a persistent RGBA8 texture updated through `update_dynamic_texture`.
    pub fn create_dynamic_texture(
        &mut self,
        textures: &mut Textures,
        width: u32,
        height: u32,
    ) -> GalResult<DynamicTexture> {
        self.state
            .create_dynamic_texture(&mut self.gal, textures, width, height)
    }
    /// Stages pixels for the next `present`/`prepare` that draws the texture.
    pub fn update_dynamic_texture(
        &mut self,
        texture: DynamicTexture,
        rgba: &[u8],
    ) -> GalResult<()> {
        self.state.update_dynamic_texture(texture, rgba)
    }
    /// Stages owned pixels without an additional copy.
    pub fn update_dynamic_texture_owned(
        &mut self,
        texture: DynamicTexture,
        rgba: Vec<u8>,
    ) -> GalResult<()> {
        self.state.update_dynamic_texture_owned(texture, rgba)
    }
    /// Destroys a dynamic texture and its bindings.
    pub fn destroy_dynamic_texture(
        &mut self,
        textures: &mut Textures,
        texture: DynamicTexture,
    ) -> GalResult<()> {
        self.state
            .destroy_dynamic_texture(&mut self.gal, textures, texture)
    }
    /// GPU resource and submission counters for diagnostics/benchmarks.
    pub fn gal_metrics(&self) -> Metrics {
        self.gal.metrics().clone()
    }
    /// Backend or adapter name reported by VulkanicGAL.
    pub fn device_name(&self) -> &str {
        self.gal.capabilities().name
    }
    /// What the renderer runs on: requested and selected backend, device and
    /// driver (see `BackendInfo`'s one-line `Display` for debug screens).
    pub fn backend_info(&self) -> BackendInfo {
        self.gal.backend_info()
    }
    /// Backend timing, wait, and churn counters for diagnostics/benchmarks.
    pub fn backend_metrics(&self) -> BackendRuntimeMetrics {
        self.gal.backend_runtime_metrics()
    }
    /// Submission-to-observed-completion latencies accumulated by the surface.
    pub fn take_completion_latencies(&mut self) -> Vec<(SubmissionId, u64)> {
        self.surface
            .as_mut()
            .map(|surface| surface.frame.take_completion_latencies())
            .unwrap_or_default()
    }
    /// Engine-frame-to-observed-completion latencies accumulated by the surface.
    pub fn take_frame_completion_latencies(&mut self) -> Vec<(SubmissionId, u64)> {
        self.surface
            .as_mut()
            .map(|surface| surface.frame.take_frame_completion_latencies())
            .unwrap_or_default()
    }
    pub fn viewport(&self, canvas: &Canvas) -> Option<Viewport> {
        let s = self.surface.as_ref()?;
        let size = s.frame.size();
        Viewport::calculate((canvas.width, canvas.height), size, s.options)
    }
    pub fn present(&mut self, canvas: &Canvas, textures: &Textures) -> GalResult<PresentOutcome> {
        canvas
            .validate(textures)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        let Self {
            gal,
            state,
            surface,
            frame_overlap,
        } = self;
        let surface = surface
            .as_mut()
            .ok_or_else(|| GalError::invalid_argument("renderer has no window surface"))?;
        let options = surface.options;
        let policy = if *frame_overlap {
            CompletionPolicy::Poll
        } else {
            CompletionPolicy::Wait
        };
        if surface.frame.needs_resize() {
            state.release_targets(gal)?;
        }
        let _report = match surface.frame.render(gal, policy, |gal, target| {
            let prepared = state.prepare(gal, canvas, textures, target, options, LoadOp::Clear)?;
            Ok(vec![prepared.commands])
        }) {
            Ok(Some(report)) => report,
            Ok(None) => {
                // The surface helper deferred (zero size, minimized, or resized).
                let _ = state.release_targets(gal);
                return Ok(PresentOutcome::Deferred);
            }
            Err(error) => {
                // The surface helper already cancelled the acquired frame and
                // released its targets; drop cached passes that point at them.
                let _ = state.release_targets(gal);
                return Err(error);
            }
        };
        let presented = matches!(
            surface.frame.last_present_status(),
            Some(FramePresentStatus::Presented | FramePresentStatus::Suboptimal)
        );
        Ok(if presented {
            PresentOutcome::Presented
        } else {
            PresentOutcome::Deferred
        })
    }
    /// Real GPU rendering/readback using the same quad shaders as native frames.
    /// Returns tightly packed top-to-bottom RGBA8, without creating a window.
    pub fn render_to_image(&mut self, canvas: &Canvas, textures: &Textures) -> GalResult<Vec<u8>> {
        canvas
            .validate(textures)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        let size = extent((canvas.width, canvas.height));
        let texture = self.gal.create_texture(TextureDesc {
            label: "2d-offscreen".into(),
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            extent: size,
            mip_levels: 1,
            array_layers: 1,
            usages: vec![TextureUsage::ColorAttachment, TextureUsage::TransferSrc],
        })?;
        let view = self.gal.create_texture_view(TextureViewDesc {
            label: "2d-offscreen-view".into(),
            texture,
            format: TextureFormat::Rgba8Unorm,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        })?;
        let target = self.gal.create_render_target(RenderTargetDesc {
            label: "2d-offscreen-target".into(),
            color_views: vec![view],
            depth_stencil_view: None,
            extent: size,
        })?;
        let readback = self.gal.create_buffer(BufferDesc {
            label: "2d-readback".into(),
            size: u64::from(size.width) * u64::from(size.height) * 4,
            memory: MemoryDomain::Readback,
            usages: vec![BufferUsage::TransferDst, BufferUsage::HostRead],
        })?;
        let result = (|| {
            let prepared = self.state.prepare(
                &mut self.gal,
                canvas,
                textures,
                target,
                DisplayOptions::default(),
                LoadOp::Clear,
            )?;
            let mut operations = prepared.commands.operations;
            operations.extend([
                barrier(
                    texture,
                    TextureUsageState::ColorAttachment,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::CopyTextureToBuffer(copy(readback, texture, size)),
                barrier(
                    readback,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ),
                CommandOp::HostReadBuffer {
                    buffer: readback,
                    offset: 0,
                    size: u64::from(size.width) * u64::from(size.height) * 4,
                },
            ]);
            let token = self.gal.submit(SubmissionBatch {
                label: "2d-readback".into(),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: "2d-readback".into(),
                    operations,
                })],
            })?;
            self.gal.retire_through(token.submission)?;
            let read = self
                .gal
                .completed_host_reads()
                .into_iter()
                .find(|read| read.buffer == readback)
                .ok_or_else(|| GalError::backend("2D readback did not complete"))?;
            Ok(read.bytes)
        })();
        self.state.release_targets(&mut self.gal)?;
        self.gal.destroy(readback)?;
        self.gal.destroy(target)?;
        self.gal.destroy(view)?;
        self.gal.destroy(texture)?;
        result
    }
}
/// Ready-made software-framebuffer presenter. Owns a `Renderer2D`, a persistent
/// RGBA8 dynamic texture, and a one-quad canvas; each `present` uploads pixels
/// into the existing texture and uses the same scaling/letterbox/surface path as
/// normal 2D rendering. For shared-device composition use `RenderState2D`
/// directly with `create_dynamic_texture`/`update_dynamic_texture`.
pub struct FramebufferPresenter {
    renderer: Renderer2D,
    source: (u32, u32),
    textures: Textures,
    dynamic: DynamicTexture,
    canvas: Canvas,
}
impl FramebufferPresenter {
    /// Creates a presenter for a native window on the default windowed
    /// backend (see `Renderer2D::for_window`).
    #[cfg(any(feature = "vulkan", feature = "wgpu"))]
    pub fn new<W: HasDisplayHandle + HasWindowHandle + 'static>(
        window: Arc<W>,
        window_size: (u32, u32),
        source: (u32, u32),
        options: DisplayOptions,
    ) -> GalResult<Self> {
        validate_framebuffer_size(source)?;
        let renderer = Renderer2D::for_window(window, window_size, options)?;
        Self::from_renderer(renderer, source)
    }
    /// Wraps a caller-created windowed renderer, regardless of backend.
    pub fn from_renderer(mut renderer: Renderer2D, source: (u32, u32)) -> GalResult<Self> {
        validate_framebuffer_size(source)?;
        let mut textures = Textures::default();
        let dynamic = renderer.state.create_dynamic_texture(
            &mut renderer.gal,
            &mut textures,
            source.0,
            source.1,
        )?;
        let mut canvas = Canvas::new(source.0, source.1)
            .map_err(|error| GalError::invalid_argument(error.to_string()))?;
        canvas.sprite(Sprite::new(
            dynamic.id(),
            Rect::new(0.0, 0.0, source.0 as f32, source.1 as f32),
        ));
        Ok(Self {
            renderer,
            source,
            textures,
            dynamic,
            canvas,
        })
    }
    /// Presents one tightly packed, top-to-bottom RGBA8 framebuffer.
    pub fn present(&mut self, rgba: &[u8]) -> GalResult<PresentOutcome> {
        self.renderer
            .state
            .update_dynamic_texture(self.dynamic, rgba)?;
        self.renderer.present(&self.canvas, &self.textures)
    }
    /// Presents owned pixels without an additional staging copy.
    pub fn present_owned(&mut self, rgba: Vec<u8>) -> GalResult<PresentOutcome> {
        self.renderer
            .state
            .update_dynamic_texture_owned(self.dynamic, rgba)?;
        self.renderer.present(&self.canvas, &self.textures)
    }
    /// Renders the framebuffer through the same 2D pipeline to a headless image.
    pub fn render_to_image(&mut self, rgba: &[u8]) -> GalResult<Vec<u8>> {
        self.renderer
            .state
            .update_dynamic_texture(self.dynamic, rgba)?;
        self.renderer.render_to_image(&self.canvas, &self.textures)
    }
    pub fn source_size(&self) -> (u32, u32) {
        self.source
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
    }
    pub fn options(&self) -> DisplayOptions {
        self.renderer.options()
    }
    pub fn set_options(&mut self, options: DisplayOptions) -> GalResult<()> {
        self.renderer.set_options(options)
    }
    pub fn viewport(&self) -> Option<Viewport> {
        self.renderer.viewport(&self.canvas)
    }
    pub fn set_frame_overlap(&mut self, enabled: bool) {
        self.renderer.set_frame_overlap(enabled);
    }
    pub fn stats(&self) -> RenderStats {
        self.renderer.stats()
    }
    pub fn gal_metrics(&self) -> Metrics {
        self.renderer.gal_metrics()
    }
    pub fn device_name(&self) -> &str {
        self.renderer.device_name()
    }
    /// See `Renderer2D::backend_info`.
    pub fn backend_info(&self) -> BackendInfo {
        self.renderer.backend_info()
    }
    pub fn backend_metrics(&self) -> BackendRuntimeMetrics {
        self.renderer.backend_metrics()
    }
    pub fn take_completion_latencies(&mut self) -> Vec<(SubmissionId, u64)> {
        self.renderer.take_completion_latencies()
    }
    pub fn take_frame_completion_latencies(&mut self) -> Vec<(SubmissionId, u64)> {
        self.renderer.take_frame_completion_latencies()
    }
}
impl Drop for Renderer2D {
    fn drop(&mut self) {
        let _ = self.gal.retire_through(self.gal.latest_submission_id());
        let _ = self.state.release(&mut self.gal);
        if let Some(surface) = &mut self.surface {
            let _ = surface.frame.release_targets(&mut self.gal);
        }
        let _ = self.gal.shutdown_frame_surface();
    }
}
/// Reusable rendering state on a caller-owned GAL. No window, submission or
/// presentation ownership. All methods must use the same GAL as `new`.
/// Call `release` before dropping to return cached resources to GAL.
pub struct RenderState2D {
    layout: Handle,
    resource_layout: Handle,
    vertex: Handle,
    fragment: Handle,
    pipelines: BTreeMap<(TextureFormat, Option<TextureFormat>, PixelState), Handle>,
    slots: Vec<Slot>,
    textures: BTreeMap<TextureId, TextureGpu>,
    pending_dynamic: BTreeMap<TextureId, Vec<u8>>,
    uniform_stride: usize,
    stats: RenderStats,
}
impl RenderState2D {
    pub fn new(gal: &mut VulkanicGal) -> GalResult<Self> {
        let mut created = Vec::new();
        let result = (|| {
            if gal.capabilities().shader_conventions.glsl_dialect != GlslDialect::ExplicitBindings {
                return Err(GalError::unsupported_feature(
                    "2D shaders require explicit-binding GLSL conventions",
                ));
            }
            let stride = (gal.capabilities().limits.uniform_buffer_offset_alignment as usize)
                .max(32)
                .next_power_of_two();
            let resource_layout = gal.create_resource_layout(ResourceLayoutDesc {
                label: "2d-resources".into(),
                bindings: vec![
                    desc(0, ResourceBindingKind::CombinedTextureSampler, 0),
                    desc(1, ResourceBindingKind::StorageBuffer, 0),
                    desc(2, ResourceBindingKind::UniformBuffer, 1),
                    desc(3, ResourceBindingKind::CombinedTextureSampler, 0),
                ],
            })?;
            created.push(resource_layout);
            let layout = gal.create_pipeline_layout(PipelineLayoutDesc {
                label: "2d-layout".into(),
                resource_layouts: vec![resource_layout],
            })?;
            created.push(layout);
            let vertex = gal.create_shader_module(shader(
                "2d-vertex",
                ShaderStage::Vertex,
                include_str!("quad.vert"),
            ))?;
            created.push(vertex);
            let fragment = gal.create_shader_module(shader(
                "2d-fragment",
                ShaderStage::Fragment,
                include_str!("quad.frag"),
            ))?;
            created.push(fragment);
            Ok(Self {
                layout,
                resource_layout,
                vertex,
                fragment,
                pipelines: BTreeMap::new(),
                slots: Vec::new(),
                textures: BTreeMap::new(),
                pending_dynamic: BTreeMap::new(),
                uniform_stride: stride,
                stats: RenderStats::default(),
            })
        })();
        if result.is_err() {
            for h in created.into_iter().rev() {
                let _ = gal.retire(h);
            }
        }
        result
    }

    pub fn stats(&self) -> RenderStats {
        self.stats
    }
    /// Number of live persistent dynamic textures owned by this state.
    pub fn dynamic_texture_count(&self) -> usize {
        self.textures
            .values()
            .filter(|texture| texture.dynamic)
            .count()
    }
    /// Prepares a pass without acquiring, presenting, or waiting for this pass.
    /// First-use CPU images are uploaded on this GAL queue before their draws.
    /// Up to three independent command lists may reserve transient slots.
    pub fn prepare(
        &mut self,
        gal: &mut VulkanicGal,
        canvas: &Canvas,
        assets: &Textures,
        target: Handle,
        options: DisplayOptions,
        load: LoadOp,
    ) -> GalResult<Prepared2D> {
        if self.layout.is_null() {
            return Err(GalError::invalid_argument("2D state is released"));
        }
        canvas
            .validate(assets)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        options.validate().map_err(GalError::invalid_argument)?;
        let format = gal.pass_target_color_format(target)?;
        let size = gal.pass_target_extent(target)?;
        let depth = None;
        gal.begin_command_recording()?;
        let result = self.record(
            gal, canvas, assets, target, format, size, depth, options, load,
        );
        let finish = gal.finish_command_recording();
        result.and_then(|prepared| finish.map(|()| prepared))
    }
    /// Registers a private view of another renderer's sampled image. The source
    /// owner keeps the texture; this renderer owns only its view and bindings.
    /// The caller orders writes -> ShaderRead before recording consumers.
    pub fn register_texture(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &mut Textures,
        source_view: Handle,
    ) -> GalResult<TextureId> {
        if self.layout.is_null() {
            return Err(GalError::invalid_argument("2D state is released"));
        }
        let info = gal.texture_view_info(source_view)?;
        if info.dimension != TextureDimension::D2
            || info.extent.width > 16384
            || info.extent.height > 16384
            || info.range.base_mip != 0
            || info.range.mip_count != 1
            || info.range.layer_count != 1
            || info.extent.depth != 1
            || !info.usages.contains(&TextureUsage::Sampled)
            || !matches!(
                info.format,
                TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm
            )
        {
            return Err(GalError::invalid_argument(
                "shared image must be a sampled 2D RGBA8/BGRA8 single-mip view",
            ));
        }
        let view = gal.create_texture_view(TextureViewDesc {
            label: "2d-shared-view".into(),
            texture: info.texture,
            format: info.format,
            base_mip: 0,
            mip_count: 1,
            base_layer: info.range.base_layer,
            layer_count: 1,
        })?;
        let id = assets
            .declare_external(info.extent.width, info.extent.height)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        if let Err(error) = self.install_texture(gal, id, None, view, false) {
            assets.remove(id);
            let _ = gal.retire(view);
            return Err(error);
        }
        Ok(id)
    }
    /// Creates a persistent RGBA8 texture whose contents are uploaded per frame.
    /// The texture, view, samplers and bindings are created once. Call
    /// [`Self::update_dynamic_texture`] before each `prepare` that draws it.
    pub fn create_dynamic_texture(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &mut Textures,
        width: u32,
        height: u32,
    ) -> GalResult<DynamicTexture> {
        if self.layout.is_null() {
            return Err(GalError::invalid_argument("2D state is released"));
        }
        validate_framebuffer_size((width, height))?;
        let id = assets
            .declare_external(width, height)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        let size = extent((width, height));
        let mut created = Vec::new();
        let result = (|| {
            let texture = gal.create_texture(TextureDesc {
                label: "2d-dynamic-texture".into(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: size,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
            })?;
            created.push(texture);
            let view = gal.create_texture_view(TextureViewDesc {
                label: "2d-dynamic-view".into(),
                texture,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(view);
            self.install_texture(gal, id, Some(texture), view, true)?;
            Ok(DynamicTexture {
                id,
                size: (width, height),
            })
        })();
        if result.is_err() {
            assets.remove(id);
            for h in created.into_iter().rev() {
                let _ = gal.retire(h);
            }
        }
        result
    }
    /// Stages a full RGBA8 frame for the next `prepare` that draws `texture`.
    /// The caller supplies tightly packed, top-to-bottom pixels. Staging reuses
    /// its CPU buffer while possible; the prepared command list owns its snapshot,
    /// so frames submitted out of preparation order keep their own pixels.
    pub fn update_dynamic_texture(
        &mut self,
        texture: DynamicTexture,
        rgba: &[u8],
    ) -> GalResult<()> {
        let expected = self.validate_dynamic_texture(texture, rgba.len())?;
        match self.pending_dynamic.get_mut(&texture.id) {
            Some(buffer) if buffer.len() == expected => buffer.copy_from_slice(rgba),
            Some(buffer) => *buffer = rgba.to_vec(),
            None => {
                self.pending_dynamic.insert(texture.id, rgba.to_vec());
            }
        }
        Ok(())
    }
    /// Stages pixels without copying when the caller already owns a `Vec`.
    pub fn update_dynamic_texture_owned(
        &mut self,
        texture: DynamicTexture,
        rgba: Vec<u8>,
    ) -> GalResult<()> {
        self.validate_dynamic_texture(texture, rgba.len())?;
        self.pending_dynamic.insert(texture.id, rgba);
        Ok(())
    }
    fn validate_dynamic_texture(&self, texture: DynamicTexture, length: usize) -> GalResult<usize> {
        let expected = validate_framebuffer_size(texture.size)? as usize;
        if length != expected {
            return Err(GalError::invalid_argument(format!(
                "dynamic RGBA length {} does not match {}x{}",
                length, texture.size.0, texture.size.1
            )));
        }
        if !self
            .textures
            .get(&texture.id)
            .is_some_and(|texture| texture.dynamic)
        {
            return Err(GalError::invalid_argument(
                "dynamic texture is not owned by this render state",
            ));
        }
        Ok(expected)
    }
    /// Destroys a dynamic texture and its bindings. Pending pixels are dropped.
    /// The matching `Textures` entry is removed as well.
    pub fn destroy_dynamic_texture(
        &mut self,
        gal: &mut VulkanicGal,
        assets: &mut Textures,
        texture: DynamicTexture,
    ) -> GalResult<()> {
        if !self
            .textures
            .get(&texture.id)
            .is_some_and(|texture| texture.dynamic)
        {
            return Err(GalError::invalid_argument(
                "dynamic texture is not owned by this render state",
            ));
        }
        self.pending_dynamic.remove(&texture.id);
        assets.remove(texture.id);
        let owned = self.textures.remove(&texture.id).unwrap();
        for h in owned.combined {
            gal.retire(h)?;
        }
        for h in owned.samplers {
            gal.retire(h)?;
        }
        gal.retire(owned.view)?;
        if let Some(h) = owned.texture {
            gal.retire(h)?;
        }
        Ok(())
    }
    fn install_texture(
        &mut self,
        gal: &mut VulkanicGal,
        id: TextureId,
        texture: Option<Handle>,
        view: Handle,
        dynamic: bool,
    ) -> GalResult<()> {
        let mut samplers = Vec::new();
        let mut combined = Vec::new();
        let result = (|| {
            for filter in [SamplerFilter::Nearest, SamplerFilter::Linear] {
                let sampler = gal.create_sampler(SamplerDesc {
                    label: "2d-sampler".into(),
                    min_filter: filter,
                    mag_filter: filter,
                    mip_filter: SamplerFilter::Nearest,
                    address_u: SamplerAddressMode::ClampToEdge,
                    address_v: SamplerAddressMode::ClampToEdge,
                    address_w: SamplerAddressMode::ClampToEdge,
                    comparison: None,
                })?;
                samplers.push(sampler);
                combined.push(
                    gal.create_combined_texture_sampler(CombinedTextureSamplerDesc {
                        label: "2d-image".into(),
                        texture_view: view,
                        sampler,
                    })?,
                );
            }
            Ok(())
        })();
        if let Err(error) = result {
            for h in combined.into_iter().rev() {
                let _ = gal.retire(h);
            }
            for h in samplers.into_iter().rev() {
                let _ = gal.retire(h);
            }
            return Err(error);
        }
        self.textures.insert(
            id,
            TextureGpu {
                texture,
                view,
                samplers: [samplers[0], samplers[1]],
                combined: [combined[0], combined[1]],
                dynamic,
                initialized: !dynamic,
            },
        );
        Ok(())
    }
    fn collect_removed(&mut self, gal: &mut VulkanicGal, assets: &Textures) -> GalResult<()> {
        let dynamic: BTreeSet<TextureId> = self
            .textures
            .iter()
            .filter(|(_, texture)| texture.dynamic)
            .map(|(id, _)| *id)
            .collect();
        for slot in &mut self.slots {
            if slot.usage.has_pending_commands() {
                continue;
            }
            let removed: Vec<_> = slot
                .sets
                .keys()
                .filter(|(a, b)| {
                    [a, b]
                        .into_iter()
                        .any(|id| !dynamic.contains(id) && assets.dimensions(*id).is_none())
                })
                .copied()
                .collect();
            for id in removed {
                for h in slot.sets.remove(&id).unwrap() {
                    gal.retire(h)?;
                }
            }
        }
        let removed: Vec<_> = self
            .textures
            .iter()
            .filter(|(id, texture)| !texture.dynamic && assets.dimensions(**id).is_none())
            .map(|(id, _)| *id)
            .collect();
        for id in removed {
            let t = self.textures.remove(&id).unwrap();
            for h in t.combined {
                gal.retire(h)?;
            }
            for h in t.samplers {
                gal.retire(h)?;
            }
            gal.retire(t.view)?;
            if let Some(h) = t.texture {
                gal.retire(h)?;
            }
        }
        Ok(())
    }
    fn ensure_set(
        &mut self,
        gal: &mut VulkanicGal,
        index: usize,
        id: (TextureId, TextureId),
    ) -> GalResult<()> {
        let slot = &mut self.slots[index];
        if slot.sets.contains_key(&id) {
            return Ok(());
        }
        let texture = &self.textures[&id.0];
        let palette = self.textures[&id.1].combined[0];
        let mut sets = Vec::new();
        let result = (|| {
            for pair in texture.combined {
                sets.push(gal.create_resource_set(ResourceSetDesc {
                    label: "2d-set".into(),
                    layout: self.resource_layout,
                    bindings: vec![
                        binding(
                            3,
                            palette,
                            ResourceBindingKind::CombinedTextureSampler,
                            None,
                            vec![],
                        ),
                        binding(
                            0,
                            pair,
                            ResourceBindingKind::CombinedTextureSampler,
                            None,
                            vec![],
                        ),
                        binding(
                            1,
                            slot.geometry,
                            ResourceBindingKind::StorageBuffer,
                            None,
                            vec![],
                        ),
                        binding(
                            2,
                            slot.uniform,
                            ResourceBindingKind::UniformBuffer,
                            Some(32),
                            vec![0],
                        ),
                    ],
                })?);
            }
            Ok(())
        })();
        if let Err(error) = result {
            for h in sets {
                let _ = gal.retire(h);
            }
            return Err(error);
        }
        slot.sets.insert(id, [sets[0], sets[1]]);
        Ok(())
    }
    fn mask_target(
        &mut self,
        gal: &mut VulkanicGal,
        index: usize,
        original: Handle,
        size: Extent3d,
    ) -> GalResult<(Handle, Handle, Handle)> {
        let slot = &mut self.slots[index];
        if let Some(&target) = slot.mask_targets.get(&original) {
            return Ok(target);
        }
        if slot.mask_targets.len() >= 8 {
            for h in std::mem::take(&mut slot.passes).into_values() {
                gal.retire(h)?;
            }
            for (target, texture, view) in std::mem::take(&mut slot.mask_targets).into_values() {
                for h in [target, view, texture] {
                    gal.retire(h)?;
                }
            }
        }
        let mut created = Vec::new();
        let result = (|| {
            let texture = gal.create_texture(TextureDesc {
                label: "2d-mask".into(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth24Stencil8,
                extent: size,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::DepthStencilAttachment],
            })?;
            created.push(texture);
            let view = gal.create_texture_view(TextureViewDesc {
                label: "2d-mask-view".into(),
                texture,
                format: TextureFormat::Depth24Stencil8,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(view);
            let color = gal.pass_target_color_attachment(original)?;
            let target = gal.create_render_target(RenderTargetDesc {
                label: "2d-masked-target".into(),
                color_views: vec![color],
                depth_stencil_view: Some(view),
                extent: size,
            })?;
            created.push(target);
            Ok((target, texture, view))
        })();
        match result {
            Ok(value) => {
                slot.mask_targets.insert(original, value);
                Ok(value)
            }
            Err(error) => {
                for h in created.into_iter().rev() {
                    let _ = gal.retire(h);
                }
                Err(error)
            }
        }
    }
    fn acquire_slot(
        &mut self,
        gal: &mut VulkanicGal,
        geometry: usize,
        uniform: usize,
        upload: usize,
    ) -> GalResult<usize> {
        let completed = gal.poll_completed();
        let mut index = self.slots.iter().position(|s| {
            !s.usage.has_pending_commands() && s.usage.last_submission() <= completed
        });
        if index.is_none() && self.slots.len() < 3 {
            let g = geometry.next_power_of_two();
            let u = uniform.next_power_of_two();
            let up = upload.next_power_of_two().max(1);
            self.slots.push(Slot {
                geometry: buffer(gal, "2d-geometry", g, BufferUsage::Storage)?,
                uniform: buffer(gal, "2d-frame", u, BufferUsage::Uniform)?,
                upload: buffer(gal, "2d-framebuffer-upload", up, BufferUsage::TransferSrc)?,
                geometry_capacity: g,
                uniform_capacity: u,
                upload_capacity: up,
                sets: BTreeMap::new(),
                mask_targets: BTreeMap::new(),
                passes: BTreeMap::new(),
                usage: SubmissionUsage::default(),
            });
            index = Some(self.slots.len() - 1);
        }
        if index.is_none() {
            let (i, submission) = self
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.usage.has_pending_commands())
                .map(|(i, s)| (i, s.usage.last_submission()))
                .min_by_key(|(_, id)| *id)
                .ok_or_else(|| {
                    GalError::submission(
                        StatusCode::InFlight,
                        "all 2D slots are reserved by unsubmitted commands",
                    )
                })?;
            gal.retire_through(submission)?;
            index = Some(i);
        }
        let i = index.unwrap();
        let s = &mut self.slots[i];
        if geometry > s.geometry_capacity || uniform > s.uniform_capacity {
            for sets in std::mem::take(&mut s.sets).into_values() {
                for h in sets {
                    gal.retire(h)?;
                }
            }
            if geometry > s.geometry_capacity {
                gal.retire(s.geometry)?;
                s.geometry_capacity = geometry.next_power_of_two();
                s.geometry = buffer(
                    gal,
                    "2d-geometry",
                    s.geometry_capacity,
                    BufferUsage::Storage,
                )?;
            }
            if uniform > s.uniform_capacity {
                gal.retire(s.uniform)?;
                s.uniform_capacity = uniform.next_power_of_two();
                s.uniform = buffer(gal, "2d-frame", s.uniform_capacity, BufferUsage::Uniform)?;
            }
        }
        if upload > s.upload_capacity {
            gal.retire(s.upload)?;
            s.upload_capacity = upload.next_power_of_two();
            s.upload = buffer(
                gal,
                "2d-framebuffer-upload",
                s.upload_capacity,
                BufferUsage::TransferSrc,
            )?;
        }
        Ok(i)
    }
    pub fn release_targets(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.require_no_pending()?;
        for slot in &mut self.slots {
            for h in std::mem::take(&mut slot.passes).into_values() {
                gal.retire(h)?;
            }
            for (target, texture, view) in std::mem::take(&mut slot.mask_targets).into_values() {
                for h in [target, view, texture] {
                    gal.retire(h)?;
                }
            }
        }
        Ok(())
    }
    fn require_no_pending(&self) -> GalResult<()> {
        if self.slots.iter().any(|s| s.usage.has_pending_commands()) {
            return Err(GalError::submission(
                StatusCode::InFlight,
                "2D commands still retain resources",
            ));
        }
        Ok(())
    }
    /// Explicit cleanup; rejected without mutation while prepared commands live.
    /// Submitted work uses GAL's deferred retirement, without a device idle wait.
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.layout.is_null() {
            return Ok(());
        }
        self.require_no_pending()?;
        self.release_targets(gal)?;
        self.pending_dynamic.clear();
        for slot in self.slots.drain(..) {
            for sets in slot.sets.into_values() {
                for h in sets {
                    gal.retire(h)?;
                }
            }
            gal.retire(slot.geometry)?;
            gal.retire(slot.uniform)?;
            gal.retire(slot.upload)?;
        }
        for t in std::mem::take(&mut self.textures).into_values() {
            for h in t.combined {
                gal.retire(h)?;
            }
            for h in t.samplers {
                gal.retire(h)?;
            }
            gal.retire(t.view)?;
            if let Some(h) = t.texture {
                gal.retire(h)?;
            }
        }
        for h in std::mem::take(&mut self.pipelines).into_values() {
            gal.retire(h)?;
        }
        for h in [
            self.layout,
            self.vertex,
            self.fragment,
            self.resource_layout,
        ] {
            gal.retire(h)?;
        }
        self.layout = Handle::NULL;
        Ok(())
    }
    fn upload(&mut self, gal: &mut VulkanicGal, id: TextureId, assets: &Textures) -> GalResult<()> {
        if self.textures.contains_key(&id) {
            return Ok(());
        }
        let image = assets.get(id).ok_or_else(|| {
            GalError::invalid_argument("GPU-only image is not registered on this renderer")
        })?;
        let size = extent((image.width, image.height));
        let mut created = Vec::new();
        let result = (|| {
            let texture = gal.create_texture(TextureDesc {
                label: "2d-texture".into(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: size,
                mip_levels: 1,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
            })?;
            created.push(texture);
            let upload = buffer(
                gal,
                "2d-texture-upload",
                image.rgba.len(),
                BufferUsage::TransferSrc,
            )?;
            created.push(upload);
            let operations = vec![
                CommandOp::HostWriteBuffer {
                    buffer: upload,
                    offset: 0,
                    data: image.rgba.to_vec(),
                },
                barrier(
                    upload,
                    TextureUsageState::TransferDst,
                    TextureUsageState::TransferSrc,
                ),
                barrier(
                    texture,
                    TextureUsageState::Undefined,
                    TextureUsageState::TransferDst,
                ),
                CommandOp::CopyBufferToTexture(copy(upload, texture, size)),
                barrier(
                    texture,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
            ];
            gal.submit(SubmissionBatch {
                label: "2d-initial-upload".into(),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: "2d-initial-upload".into(),
                    operations,
                })],
            })?;
            gal.destroy(upload)?;
            self.stats.texture_upload_bytes += image.rgba.len();
            let view = gal.create_texture_view(TextureViewDesc {
                label: "2d-texture-view".into(),
                texture,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: 1,
                base_layer: 0,
                layer_count: 1,
            })?;
            created.push(view);
            self.install_texture(gal, id, Some(texture), view, false)?;
            Ok(())
        })();
        if result.is_err() {
            for h in created.into_iter().rev() {
                let _ = gal.retire(h);
            }
        }
        result
    }
    fn pipeline(
        &mut self,
        gal: &mut VulkanicGal,
        format: TextureFormat,
        depth: Option<TextureFormat>,
        pixels: PixelState,
    ) -> GalResult<Handle> {
        let pixels = PixelState {
            alpha_min: 0,
            exclude_index: None,
            ..pixels
        };
        if let Some(pipeline) = self.pipelines.get(&(format, depth, pixels)) {
            return Ok(*pipeline);
        }
        let pipeline = gal.create_graphics_pipeline(GraphicsPipelineDesc {
            label: "2d-quads".into(),
            layout: self.layout,
            vertex_shader: self.vertex,
            fragment_shader: self.fragment,
            topology: PrimitiveTopology::Triangles,
            cull_mode: CullMode::None,
            front_face: FrontFace::CounterClockwise,
            provoking_vertex: ProvokingVertex::First,
            raster_y_direction: RasterYDirection::Down,
            blend: if pixels.color_write {
                BlendMode::Alpha
            } else {
                BlendMode::DepthMask
            },
            depth_compare: None,
            depth_write: false,
            depth_bias: None,
            color_formats: vec![format],
            depth_format: depth,
            stencil: (pixels.read_mask != 0 || pixels.write_mask != 0).then(|| {
                let face = StencilFaceState {
                    compare: if pixels.read_mask == 0 {
                        CompareOp::Always
                    } else {
                        CompareOp::Equal
                    },
                    reference: pixels.reference as u32,
                    read_mask: if pixels.read_mask == 0 {
                        255
                    } else {
                        pixels.read_mask as u32
                    },
                    write_mask: pixels.write_mask as u32,
                    fail_op: StencilOp::Keep,
                    depth_fail_op: StencilOp::Keep,
                    pass_op: StencilOp::Replace,
                };
                StencilState {
                    front: face,
                    back: face,
                }
            }),
        })?;
        self.pipelines.insert((format, depth, pixels), pipeline);
        Ok(pipeline)
    }
    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        gal: &mut VulkanicGal,
        canvas: &Canvas,
        assets: &Textures,
        target: Handle,
        format: TextureFormat,
        size: Extent3d,
        depth: Option<(Handle, Handle, TextureFormat)>,
        options: DisplayOptions,
        load: LoadOp,
    ) -> GalResult<Prepared2D> {
        self.collect_removed(gal, assets)?;
        self.stats = RenderStats {
            quads: canvas.quads.len(),
            ..Default::default()
        };
        let mut data = Vec::with_capacity(canvas.quads.len() * 80);
        let mut batches: Vec<Batch> = Vec::new();
        for (index, q) in canvas.quads.iter().enumerate() {
            let image = assets.dimensions(q.texture).unwrap();
            let source =
                q.source
                    .unwrap_or(crate::Rect::new(0.0, 0.0, image.0 as f32, image.1 as f32));
            // Preserve the 80-byte quad ABI: axis padding holds raw u32 words
            // (indexed flag, transparent/excluded index + 1; alpha threshold).
            let values = [
                q.origin[0],
                q.origin[1],
                q.x[0],
                q.x[1],
                q.y[0],
                q.y[1],
                f32::from_bits(
                    u32::from(q.palette.is_some())
                        | (q.palette
                            .and_then(|p| p.transparent_index)
                            .map_or(0, |i| u32::from(i) + 1)
                            << 1)
                        | (q.pixels.exclude_index.map_or(0, |i| u32::from(i) + 1) << 10),
                ),
                f32::from_bits(u32::from(q.pixels.alpha_min)),
                source.x / image.0 as f32,
                source.y / image.1 as f32,
                (source.x + source.width) / image.0 as f32,
                (source.y + source.height) / image.1 as f32,
                q.color.0[0] as f32 / 255.0,
                q.color.0[1] as f32 / 255.0,
                q.color.0[2] as f32 / 255.0,
                q.color.0[3] as f32 / 255.0,
                q.clip.x,
                q.clip.y,
                q.clip.x + q.clip.width,
                q.clip.y + q.clip.height,
            ];
            data.extend(values.into_iter().flat_map(f32::to_ne_bytes));
            let filter = if q.palette.is_some() {
                Filter::Nearest
            } else {
                q.filter.unwrap_or(options.filter)
            };
            let palette = q.palette.map_or(q.texture, |p| p.texture);
            // Identical stencil replacements preserve the comparison result,
            // including where read/write masks overlap; draw order stays intact.
            if let Some(batch) = batches.last_mut()
                && batch.texture == q.texture
                && batch.filter == filter
                && batch.palette == palette
                && batch.pixels == q.pixels
            {
                batch.count += 1;
            } else {
                batches.push(Batch {
                    texture: q.texture,
                    palette,
                    pixels: q.pixels,
                    filter,
                    first: index as u32,
                    count: 1,
                });
            }
        }
        self.stats.batches = batches.len();
        self.stats.geometry_upload_bytes = data.len();
        let mut dynamic_uploads: Vec<(TextureId, Vec<u8>, (u32, u32))> = Vec::new();
        let mut seen_dynamic = BTreeSet::new();
        for id in batches.iter().flat_map(|b| [b.texture, b.palette]) {
            if !seen_dynamic.insert(id) {
                continue;
            }
            let Some(texture) = self.textures.get(&id) else {
                continue;
            };
            if !texture.dynamic {
                continue;
            }
            let Some(pixels) = self.pending_dynamic.remove(&id) else {
                // A dynamic texture that already has GPU contents can be reused
                // without re-uploading unchanged pixels.
                if texture.initialized {
                    continue;
                }
                return Err(GalError::invalid_argument(
                    "dynamic texture must be updated before it is prepared for drawing",
                ));
            };
            let size = assets.dimensions(id).ok_or_else(|| {
                GalError::invalid_argument("dynamic texture is missing from the texture store")
            })?;
            let expected =
                usize::try_from(u64::from(size.0) * u64::from(size.1) * 4).map_err(|_| {
                    GalError::invalid_argument("dynamic texture size is not addressable")
                })?;
            if pixels.len() != expected {
                return Err(GalError::invalid_argument(
                    "dynamic texture pixel length changed after staging",
                ));
            }
            dynamic_uploads.push((id, pixels, size));
        }
        for (id, _, _) in &dynamic_uploads {
            if let Some(texture) = self.textures.get_mut(id) {
                texture.initialized = true;
            }
        }
        let mut upload_offsets = BTreeMap::new();
        let mut upload_bytes = 0usize;
        for (id, pixels, _) in &dynamic_uploads {
            upload_bytes = align_up(upload_bytes, 4);
            upload_offsets.insert(*id, upload_bytes);
            upload_bytes = upload_bytes.checked_add(pixels.len()).ok_or_else(|| {
                GalError::invalid_argument("dynamic framebuffer upload exceeds addressable size")
            })?;
        }
        let index = self.acquire_slot(
            gal,
            data.len().max(80),
            batches.len().max(1) * self.uniform_stride,
            upload_bytes,
        )?;
        for batch in &batches {
            self.upload(gal, batch.texture, assets)?;
            self.upload(gal, batch.palette, assets)?;
            self.ensure_set(gal, index, (batch.texture, batch.palette))?;
        }
        let viewport = Viewport::calculate(
            (canvas.width, canvas.height),
            (size.width, size.height),
            options,
        )
        .ok_or_else(|| GalError::invalid_argument("invalid 2D viewport"))?;
        let mut uniforms = vec![0; batches.len().max(1) * self.uniform_stride];
        for (i, batch) in batches.iter().enumerate() {
            let values = [
                canvas.width as f32,
                canvas.height as f32,
                batch.first as f32,
                0.0,
                viewport.x as f32 / size.width as f32,
                viewport.y as f32 / size.height as f32,
                viewport.width as f32 / size.width as f32,
                viewport.height as f32 / size.height as f32,
            ];
            let bytes: Vec<_> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
            uniforms[i * self.uniform_stride..i * self.uniform_stride + 32].copy_from_slice(&bytes);
        }
        let (target, depth) = if canvas.uses_pixel_masks() {
            let (target, texture, view) = self.mask_target(gal, index, target, size)?;
            (
                target,
                Some((texture, view, TextureFormat::Depth24Stencil8)),
            )
        } else {
            (target, depth)
        };
        let pipelines: Vec<_> = batches
            .iter()
            .map(|b| self.pipeline(gal, format, depth.map(|(_, _, f)| f), b.pixels))
            .collect::<GalResult<_>>()?;
        let mut dynamic_commands = Vec::with_capacity(dynamic_uploads.len());
        for (id, pixels, size) in dynamic_uploads {
            let texture = self.textures[&id]
                .texture
                .ok_or_else(|| GalError::invalid_argument("dynamic texture has no GPU texture"))?;
            dynamic_commands.push((upload_offsets[&id] as u64, texture, size, pixels));
        }
        let slot = &mut self.slots[index];
        if !slot.passes.contains_key(&target) {
            if slot.passes.len() >= 8 {
                for h in std::mem::take(&mut slot.passes).into_values() {
                    gal.retire(h)?;
                }
            }
            let pass = gal.create_render_pass(RenderPassDesc {
                label: "2d-pass".into(),
                target,
                color_formats: vec![format],
                depth_format: depth.map(|(_, _, f)| f),
            })?;
            slot.passes.insert(target, pass);
        }
        let pass = slot.passes[&target];
        let attachment = gal.pass_target_color_attachment(target)?;
        let clear = canvas.clear.0;
        let mut operations = Vec::new();
        if let Some((texture, _, _)) = depth {
            operations.push(barrier(
                texture,
                TextureUsageState::Undefined,
                TextureUsageState::DepthStencilAttachment,
            ));
        }
        if !dynamic_commands.is_empty() {
            let mut copy_regions = Vec::with_capacity(dynamic_commands.len());
            operations.push(barrier(
                slot.upload,
                TextureUsageState::TransferSrc,
                TextureUsageState::TransferDst,
            ));
            for (offset, texture, size, pixels) in dynamic_commands {
                operations.push(CommandOp::HostWriteBuffer {
                    buffer: slot.upload,
                    offset,
                    data: pixels,
                });
                copy_regions.push((offset, texture, size));
            }
            operations.push(barrier(
                slot.upload,
                TextureUsageState::TransferDst,
                TextureUsageState::TransferSrc,
            ));
            for (offset, texture, size) in copy_regions {
                operations.extend([
                    // The full image is overwritten, so discarding the previous
                    // contents is valid and avoids tracking per-frame generations.
                    barrier(
                        texture,
                        TextureUsageState::Undefined,
                        TextureUsageState::TransferDst,
                    ),
                    CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                        buffer: slot.upload,
                        buffer_offset: offset,
                        bytes_per_row: size.0 * 4,
                        rows_per_image: size.1,
                        texture,
                        texture_mip: 0,
                        texture_layer: 0,
                        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                        extent: extent(size),
                    }),
                    barrier(
                        texture,
                        TextureUsageState::TransferDst,
                        TextureUsageState::ShaderRead,
                    ),
                ]);
            }
            self.stats.texture_upload_bytes += upload_bytes;
        }
        if !data.is_empty() {
            operations.extend([
                barrier(
                    slot.geometry,
                    TextureUsageState::ShaderRead,
                    TextureUsageState::TransferDst,
                ),
                CommandOp::HostWriteBuffer {
                    buffer: slot.geometry,
                    offset: 0,
                    data,
                },
                barrier(
                    slot.geometry,
                    TextureUsageState::TransferDst,
                    TextureUsageState::ShaderRead,
                ),
            ]);
        }
        operations.extend([
            barrier(
                slot.uniform,
                TextureUsageState::ShaderRead,
                TextureUsageState::TransferDst,
            ),
            CommandOp::HostWriteBuffer {
                buffer: slot.uniform,
                offset: 0,
                data: uniforms,
            },
            barrier(
                slot.uniform,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ),
            CommandOp::BeginPass {
                pass,
                target,
                colors: vec![PassAttachment {
                    view: attachment,
                    load_op: if load == LoadOp::Clear {
                        AttachmentLoadOp::Clear
                    } else {
                        AttachmentLoadOp::Load
                    },
                    store_op: AttachmentStoreOp::Store,
                    clear_color: (load == LoadOp::Clear).then_some(ClearColor {
                        r: clear[0] as f32 / 255.0,
                        g: clear[1] as f32 / 255.0,
                        b: clear[2] as f32 / 255.0,
                        a: clear[3] as f32 / 255.0,
                    }),
                }],
                depth_stencil: depth.map(|(_, view, _)| PassAttachment {
                    view,
                    load_op: AttachmentLoadOp::Clear,
                    store_op: AttachmentStoreOp::Store,
                    clear_color: None,
                }),
            },
        ]);
        for (i, batch) in batches.iter().enumerate() {
            let set = slot.sets[&(batch.texture, batch.palette)]
                [usize::from(batch.filter == Filter::Linear)];
            operations.extend([
                CommandOp::BindGraphicsPipeline(pipelines[i]),
                CommandOp::BindResourceSet {
                    pipeline_layout: self.layout,
                    set_index: 0,
                    set,
                    dynamic_offsets: [(i * self.uniform_stride) as u64].into(),
                },
                CommandOp::Draw {
                    vertices: 6,
                    instances: batch.count,
                },
            ]);
        }
        operations.push(CommandOp::EndPass);
        operations.push(CommandOp::TrackSubmission(slot.usage.clone()));
        let commands = gal.create_command_list(CommandListDesc {
            label: "2d-prepared".into(),
            operations,
        })?;
        Ok(Prepared2D {
            commands,
            stats: self.stats,
        })
    }
}
fn extent(size: (u32, u32)) -> Extent3d {
    Extent3d {
        width: size.0,
        height: size.1,
        depth: 1,
    }
}
fn validate_framebuffer_size(size: (u32, u32)) -> GalResult<u64> {
    if size.0 == 0 || size.1 == 0 || size.0 > 16384 || size.1 > 16384 {
        return Err(GalError::invalid_argument(
            "framebuffer dimensions must be 1..16384",
        ));
    }
    Ok(u64::from(size.0) * u64::from(size.1) * 4)
}
fn align_up(value: usize, alignment: usize) -> usize {
    debug_assert!(alignment.is_power_of_two());
    (value + alignment - 1) & !(alignment - 1)
}
fn buffer(
    gal: &mut VulkanicGal,
    label: &str,
    size: usize,
    usage: BufferUsage,
) -> GalResult<Handle> {
    gal.create_buffer(BufferDesc {
        label: label.into(),
        size: size as u64,
        memory: MemoryDomain::Upload,
        usages: vec![usage, BufferUsage::HostWrite],
    })
}
fn desc(binding: u32, kind: ResourceBindingKind, dynamic_offset_count: u32) -> ResourceBindingDesc {
    ResourceBindingDesc {
        binding,
        kind,
        stages: PipelineStageFlags::DRAW,
        array_count: 1,
        optional: false,
        dynamic_offset_count,
    }
}
fn binding(
    binding: u32,
    resource: Handle,
    kind: ResourceBindingKind,
    buffer_range: Option<u64>,
    dynamic_offsets: Vec<u64>,
) -> ResourceBinding {
    ResourceBinding {
        binding,
        resource,
        kind,
        array_index: 0,
        access: AccessFlags::READ,
        dynamic_offsets,
        buffer_range,
    }
}
fn shader(label: &str, stage: ShaderStage, source: &str) -> ShaderModuleDesc {
    ShaderModuleDesc {
        label: label.into(),
        stage,
        code_format: ShaderCodeFormat::Glsl,
        code: source.as_bytes().to_vec(),
        entry_point: "main".into(),
    }
}
fn barrier(resource: Handle, before: TextureUsageState, after: TextureUsageState) -> CommandOp {
    CommandOp::Barrier(ResourceBarrier {
        resource,
        subresources: None,
        before,
        after,
        src_queue: QueueClass::Graphics,
        dst_queue: QueueClass::Graphics,
    })
}
fn copy(buffer: Handle, texture: Handle, extent: Extent3d) -> BufferImageCopyRegion {
    BufferImageCopyRegion {
        buffer,
        buffer_offset: 0,
        bytes_per_row: extent.width * 4,
        rows_per_image: extent.height,
        texture,
        texture_mip: 0,
        texture_layer: 0,
        texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
        extent,
    }
}
