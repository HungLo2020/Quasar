//! wgpu voxel renderer for Quasar's voxel stack.
//!
//! [`VoxelRenderer`] draws `quasar-voxel-mesh` output on a caller-owned
//! VulkanicGAL device and frame:
//!
//! * Geometry lives in three paged, deferred-free arenas
//!   (`quasar-voxel-view::PagedArena`): 12-byte packed quads, 20-byte model
//!   vertices and 16-byte smooth vertices with their indices. Vertex shaders
//!   pull from storage buffers; there are no vertex buffers.
//! * Per-chunk instances reference a per-grid table of camera-relative
//!   transforms (computed in `f64`), so worlds far from the origin render
//!   without jitter and moving grids only update one matrix.
//! * Opaque and cutout draws are culled on the GPU by a compute pass that
//!   writes indirect arguments for `multi_draw_indirect` when the device
//!   supports `INDIRECT_FIRST_INSTANCE`; otherwise (or with
//!   [`CullMode::Cpu`]) the CPU culls and issues direct draws.
//! * Translucent chunks draw back to front and re-sort their quads when the
//!   camera crosses a quad plane.
//! * Lighting and shadows use render-3d's shared [`quasar_render_3d::LIGHTING_WGSL`]
//!   module, so voxels and meshes light identically. Voxels can also cast into
//!   render-3d shadow maps through [`VoxelRenderer::prepare_depth`].
pub mod hybrid;
mod materials;

pub use hybrid::{HybridCommands, HybridFrame, Stage};
pub use materials::{MaterialDef, MaterialFilter, VoxelMaterials};

use quasar_frame::{
    Gpu, GraphicsResult, RenderTarget, error, texture_2d, validation_scope as checked,
};
use quasar_math::{DMat4, DVec3, IVec3, Mat4, Vec3};
use quasar_render_3d::{
    ColorSpace, Light, Lighting, LightingOptions, LitCamera3D, ShadingModel, ShadowBinding,
    ToonSettings,
};
use quasar_voxel::{GridId, GridTransform, Layer};
use quasar_voxel_mesh::{
    MeshBuffers, ModelVertex, PackedQuad, PackedSmoothVertex, PlaneSorter, Quad, TranslucentSorter,
};
use quasar_voxel_view::{DrawKey, DrawList, DrawRecord, PageAllocation, PagedArena};
use std::collections::{BTreeMap, HashMap};
use vulkanic_gal::*;

const STREAM_PACKED: u8 = 0;
const STREAM_MODEL: u8 = 1;
const STREAM_SMOOTH: u8 = 2;
/// Bytes per addressable element: a packed quad, a model quad (four
/// 20-byte vertices) and a smooth vertex. Draw ranges index these elements.
const ELEMENT_BYTES: [u64; 3] = [12, 80, 16];
const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;
/// `Frame` uniform: view-projection plus four `vec4`s.
const FRAME_BYTES: u64 = 128;

/// How opaque/cutout draws are culled.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CullMode {
    /// GPU culling with multi-draw indirect when supported, else CPU.
    #[default]
    Auto,
    /// Always CPU frustum culling and direct draws.
    Cpu,
}

/// Renderer limits.
#[derive(Clone, Copy, Debug)]
pub struct VoxelRendererConfig {
    /// Bytes per geometry page (one buffer each; a chunk must fit one page).
    pub page_bytes: u64,
    pub max_lights: usize,
    pub cull: CullMode,
}

impl Default for VoxelRendererConfig {
    fn default() -> Self {
        Self {
            page_bytes: 32 << 20,
            max_lights: 64,
            cull: CullMode::Auto,
        }
    }
}

/// Identifies uploaded geometry: a chunk (level 0) or LOD node of a grid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ChunkKey {
    pub grid: GridId,
    pub chunk: IVec3,
    pub level: u32,
}

impl ChunkKey {
    pub fn new(grid: GridId, chunk: IVec3) -> Self {
        Self {
            grid,
            chunk,
            level: 0,
        }
    }
    fn order(&self) -> (GridId, (i32, i32, i32), u32) {
        (
            self.grid,
            (self.chunk.x, self.chunk.y, self.chunk.z),
            self.level,
        )
    }
}
impl Ord for ChunkKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.order().cmp(&other.order())
    }
}
impl PartialOrd for ChunkKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Where uploaded chunk-local geometry sits in its grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChunkPlacement {
    /// Grid voxel coordinate of the chunk's local origin.
    pub origin: IVec3,
    /// Voxels per local unit (2^level for LOD nodes).
    pub scale: f32,
}

impl ChunkPlacement {
    pub fn new(origin: IVec3) -> Self {
        Self { origin, scale: 1.0 }
    }
}

/// Camera with an `f64` world position.
#[derive(Clone, Copy, Debug)]
pub struct VoxelCamera {
    pub position: DVec3,
    /// World-to-view (right-handed, `quasar-math` conventions).
    pub view: DMat4,
    /// `[0, 1]` depth projection.
    pub projection: Mat4,
}

impl VoxelCamera {
    /// Camera at `position` looking along `direction` (`up` must not be
    /// parallel). The view is a rotation plus an `f64` translation, so the
    /// camera-relative matrix is exact at any distance from the origin.
    pub fn looking(position: DVec3, direction: Vec3, up: Vec3, projection: Mat4) -> Option<Self> {
        let rotation = quasar_math::look_to(Vec3::ZERO, direction, up)?;
        Some(Self {
            position,
            view: rotation.as_dmat4() * DMat4::from_translation(-position),
            projection,
        })
    }
    /// View-projection for camera-relative world positions.
    pub fn relative_view_projection(&self) -> Mat4 {
        let relative = self.view * DMat4::from_translation(self.position);
        self.projection * relative.as_mat4()
    }
}

/// Distance fog in camera-relative metres. The same type configures mesh fog
/// ([`quasar_render_3d::SurfaceRenderState3D::set_fog`]), so voxels and
/// meshes fade identically.
pub use quasar_render_3d::Fog;

/// Voxel shading controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelShading {
    /// Darkening at fully occluded vertices (0..=1).
    pub ao_strength: f32,
    /// Multiply by baked voxel light (block and sky levels in meshes).
    pub voxel_light: bool,
    /// Sky light multiplier (time of day).
    pub sky_brightness: f32,
    /// Brightness at light level 0.
    pub min_light: f32,
    /// Response to scene lights (unlit ignores them; ambient comes from `Lighting`).
    pub model: ShadingModel,
    pub specular: [f32; 3],
    pub shininess: f32,
    /// Distance, height and scattering fog (shared with meshes).
    pub fog: Option<Fog>,
    /// Bands for [`ShadingModel::Toon`].
    pub toon: ToonSettings,
}

impl Default for VoxelShading {
    fn default() -> Self {
        Self {
            ao_strength: 0.45,
            voxel_light: false,
            sky_brightness: 1.0,
            min_light: 0.06,
            model: ShadingModel::Diffuse,
            specular: [0.0; 3],
            shininess: 16.0,
            fog: None,
            toon: ToonSettings::default(),
        }
    }
}

/// An outline box (voxel space of `grid`), e.g. the picked block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayBox {
    pub grid: GridId,
    pub min: Vec3,
    pub max: Vec3,
    /// Linear RGBA.
    pub color: [f32; 4],
}

/// Pass controls.
#[derive(Clone, Debug)]
pub struct VoxelPassOptions {
    pub clear_color: Option<[f32; 4]>,
    pub clear_depth: bool,
    pub opaque: bool,
    pub cutout: bool,
    pub translucent: bool,
    pub overlays: Vec<OverlayBox>,
}

impl Default for VoxelPassOptions {
    fn default() -> Self {
        Self {
            clear_color: Some([0.0, 0.0, 0.0, 1.0]),
            clear_depth: true,
            opaque: true,
            cutout: true,
            translucent: true,
            overlays: Vec::new(),
        }
    }
}

impl VoxelPassOptions {
    /// Keep existing colour and depth (draw after other passes).
    pub fn load() -> Self {
        Self {
            clear_color: None,
            clear_depth: false,
            ..Self::default()
        }
    }
}

/// Counts from one pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoxelStats {
    /// Draw records submitted (before GPU culling).
    pub records: usize,
    /// Records rejected on the CPU.
    pub cpu_culled: usize,
    /// Records culled on the GPU (`multi_draw_indirect` path).
    pub gpu_culled_records: usize,
    pub triangles: usize,
    pub indirect_batches: usize,
    pub direct_draws: usize,
    pub resorted_chunks: usize,
    /// Records skipped by a [`VoxelRenderer::set_visible_chunks`] filter.
    pub occlusion_culled: usize,
}

/// Commands to submit in order with the caller's other passes.
pub struct PreparedVoxels {
    pub commands: CommandBuffer,
    pub stats: VoxelStats,
    /// Frame number; pass it to [`VoxelRenderer::retire`] once complete.
    pub submission: u64,
}

/// Geometry counts from one upload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStats {
    pub packed_quads: usize,
    pub model_quads: usize,
    pub smooth_triangles: usize,
    pub bytes: usize,
}

struct Stream {
    arena: PagedArena,
    buffers: Vec<Buffer>,
}

struct Translucent {
    quads: Vec<Quad>,
    sorter: PlaneSorter,
    order: Vec<u32>,
    allocation: PageAllocation,
}

struct ChunkData {
    placement: ChunkPlacement,
    instance: u32,
    allocations: Vec<(DrawKey, u8, PageAllocation)>,
    translucent: Vec<Translucent>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PassKind {
    Color { format: TextureFormat, layer: u8 },
    Depth,
}

/// See the crate docs.
pub struct VoxelRenderer {
    config: VoxelRendererConfig,
    gpu_cull: bool,
    streams: [Stream; 3],
    draws: DrawList,
    chunks: BTreeMap<ChunkKey, ChunkData>,
    grid_slots: Vec<Option<(GridId, GridTransform, f64)>>,
    grid_index: HashMap<GridId, u32>,
    instances: Vec<[u32; 8]>,
    free_instances: Vec<u32>,
    instances_dirty: bool,
    frame: u64,
    layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    module: ShaderModule,
    pipelines: HashMap<(u8, PassKind), RenderPipeline>,
    overlay_layout: BindGroupLayout,
    overlay_pipelines: HashMap<TextureFormat, RenderPipeline>,
    cull_layout: BindGroupLayout,
    cull_pipeline: ComputePipeline,
    instance_buffer: Option<Buffer>,
    grid_buffer: Option<Buffer>,
    lights_buffer: Option<Buffer>,
    shadow_buffer: Option<Buffer>,
    frame_buffer: Buffer,
    cull_records: Option<Buffer>,
    cull_args: Option<Buffer>,
    cull_planes: Buffer,
    overlay_buffer: Option<Buffer>,
    empty_shadow: TextureView,
    comparison: Sampler,
    materials: VoxelMaterials,
    groups: HashMap<(u8, u32, Option<TextureView>), BindGroup>,
    depths: HashMap<TextureView, Texture>,
    resources_generation: u64,
    groups_generation: u64,
    completions: std::collections::VecDeque<(u64, std::sync::Arc<std::sync::atomic::AtomicBool>)>,
    upload_scratch: Vec<u8>,
    visible_filters: HashMap<GridId, std::collections::HashSet<IVec3>>,
    /// Bumped whenever draw records or visibility filters change.
    draw_version: u64,
    cull_cache: Option<CullCache>,
    cull_group: Option<(u64, BindGroup)>,
    /// Chunks with translucent layers (re-sorted per frame).
    translucent_keys: std::collections::BTreeSet<ChunkKey>,
}

/// Per-frame draw inputs reused while geometry and filters are unchanged.
#[derive(Default)]
struct CullCache {
    version: u64,
    layers: Vec<u8>,
    records: usize,
    occlusion_culled: usize,
    /// (layer, stream, page, byte offset, count, triangles)
    indirect: Vec<(u8, u8, u32, u64, u32, usize)>,
    /// Records culled on the CPU each frame: (layer, record).
    cpu: Vec<(u8, DrawRecord)>,
    cull_count: u32,
}

/// Bindings >= 100 are compute-only (cull layout), renumbered by 100.
fn entry(binding: u32, ty: BindingType) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding: binding % 100,
        visibility: if binding >= 100 {
            ShaderStages::COMPUTE
        } else {
            ShaderStages::VERTEX_FRAGMENT
        },
        ty,
        count: None,
    }
}
fn storage(binding: u32, read_only: bool) -> BindGroupLayoutEntry {
    entry(
        binding,
        BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}
fn uniform(binding: u32) -> BindGroupLayoutEntry {
    entry(
        binding,
        BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

fn f32_bytes(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_ne_bytes).collect()
}

/// Six inward frustum planes `(n, d)` from a `[0, 1]`-depth matrix.
fn frustum_planes(m: Mat4) -> [[f32; 4]; 6] {
    let r = |i: usize| m.row(i);
    let planes = [
        r(3) + r(0),
        r(3) - r(0),
        r(3) + r(1),
        r(3) - r(1),
        r(2),
        r(3) - r(2),
    ];
    planes.map(|p| {
        let len = p.truncate().length().max(1e-12);
        (p / len).to_array()
    })
}

fn box_visible(planes: &[[f32; 4]; 6], model: &Mat4, min: Vec3, max: Vec3) -> bool {
    let corners: [Vec3; 8] = std::array::from_fn(|c| {
        Vec3::new(
            if c & 1 == 0 { min.x } else { max.x },
            if c & 2 == 0 { min.y } else { max.y },
            if c & 4 == 0 { min.z } else { max.z },
        )
    });
    let world: [Vec3; 8] = corners.map(|p| model.transform_point3(p));
    planes.iter().all(|p| {
        let n = Vec3::new(p[0], p[1], p[2]);
        world.iter().any(|w| n.dot(*w) + p[3] >= 0.0)
    })
}

impl VoxelRenderer {
    pub fn new(
        gpu: &Gpu,
        materials: VoxelMaterials,
        config: VoxelRendererConfig,
    ) -> GraphicsResult<Self> {
        checked(gpu, || Self::create(gpu, materials, config))?
    }

    fn create(
        gpu: &Gpu,
        materials: VoxelMaterials,
        config: VoxelRendererConfig,
    ) -> GraphicsResult<Self> {
        if config.page_bytes < 1 << 16
            || config.page_bytes > gpu.device.limits().max_storage_buffer_binding_size
        {
            return Err(error(
                "voxel page size must be 64 KiB..=max storage binding size",
            ));
        }
        let source = format!(
            "diagnostic(off, derivative_uniformity);\n{}\n{}\n{}",
            quasar_render_3d::LIGHTING_WGSL,
            quasar_voxel_mesh::WGSL_FORMATS,
            include_str!("voxel.wgsl")
        );
        let module = gpu.device.create_shader_module(ShaderModuleDescriptor {
            label: Some("quasar voxels"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("voxel draw"),
                entries: &[
                    entry(
                        0,
                        BindingType::Texture {
                            sample_type: TextureSampleType::Float { filterable: true },
                            view_dimension: TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                    ),
                    entry(1, BindingType::Sampler(SamplerBindingType::Filtering)),
                    storage(2, true),
                    storage(3, true),
                    entry(
                        5,
                        BindingType::Texture {
                            sample_type: TextureSampleType::Depth,
                            view_dimension: TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                    ),
                    storage(6, true),
                    storage(7, true),
                    storage(8, true),
                    uniform(9),
                    storage(10, true),
                    entry(21, BindingType::Sampler(SamplerBindingType::Comparison)),
                ],
            });
        let overlay_layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("voxel overlay"),
                entries: &[storage(0, true)],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("voxel draw"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let cull_layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("voxel cull"),
                entries: &[
                    storage(100, true),
                    storage(101, true),
                    uniform(102),
                    storage(103, false),
                ],
            });
        let cull_module = gpu.device.create_shader_module(ShaderModuleDescriptor {
            label: Some("voxel cull"),
            source: ShaderSource::Wgsl(include_str!("cull.wgsl").into()),
        });
        let cull_pipeline = gpu
            .device
            .create_compute_pipeline(&ComputePipelineDescriptor {
                label: Some("voxel cull"),
                layout: Some(
                    &gpu.device
                        .create_pipeline_layout(&PipelineLayoutDescriptor {
                            label: Some("voxel cull"),
                            bind_group_layouts: &[Some(&cull_layout)],
                            immediate_size: 0,
                        }),
                ),
                module: &cull_module,
                entry_point: Some("cull"),
                compilation_options: Default::default(),
                cache: None,
            });
        let empty_shadow = texture_2d(
            &gpu.device,
            "voxel empty shadow",
            1,
            1,
            DEPTH_FORMAT,
            TextureUsages::TEXTURE_BINDING,
        )?;
        // A 2-layer array view is required by the binding type.
        let empty_shadow = gpu
            .device
            .create_texture(&TextureDescriptor {
                label: Some("voxel empty shadow array"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 2,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: empty_shadow.format(),
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..Default::default()
            });
        let comparison = gpu.device.create_sampler(&SamplerDescriptor {
            label: Some("voxel shadow comparison"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            compare: Some(CompareFunction::LessEqual),
            ..Default::default()
        });
        let features = gpu.device.features();
        let downlevel = gpu.adapter.get_downlevel_capabilities().flags;
        let gpu_cull = config.cull == CullMode::Auto
            && features.contains(Features::INDIRECT_FIRST_INSTANCE)
            && downlevel.contains(DownlevelFlags::INDIRECT_EXECUTION)
            && downlevel.contains(DownlevelFlags::COMPUTE_SHADERS);
        let stream = || Stream {
            arena: PagedArena::new(config.page_bytes),
            buffers: Vec::new(),
        };
        Ok(Self {
            config,
            gpu_cull,
            streams: [stream(), stream(), stream()],
            draws: DrawList::new(),
            chunks: BTreeMap::new(),
            grid_slots: Vec::new(),
            grid_index: HashMap::new(),
            instances: Vec::new(),
            free_instances: Vec::new(),
            instances_dirty: true,
            frame: 0,
            layout,
            pipeline_layout,
            module,
            pipelines: HashMap::new(),
            overlay_layout,
            overlay_pipelines: HashMap::new(),
            cull_layout,
            cull_pipeline,
            instance_buffer: None,
            grid_buffer: None,
            lights_buffer: None,
            shadow_buffer: None,
            frame_buffer: gpu.device.create_buffer(&BufferDescriptor {
                label: Some("voxel frame"),
                size: FRAME_BYTES,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            cull_records: None,
            cull_args: None,
            cull_planes: gpu.device.create_buffer(&BufferDescriptor {
                label: Some("voxel cull planes"),
                size: 112,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            overlay_buffer: None,
            empty_shadow,
            comparison,
            materials,
            groups: HashMap::new(),
            depths: HashMap::new(),
            resources_generation: 1,
            groups_generation: 0,
            completions: std::collections::VecDeque::new(),
            upload_scratch: Vec::new(),
            visible_filters: HashMap::new(),
            draw_version: 0,
            cull_cache: None,
            cull_group: None,
            translucent_keys: std::collections::BTreeSet::new(),
        })
    }

    /// Whether opaque/cutout draws use GPU culling + multi-draw indirect.
    pub fn uses_gpu_culling(&self) -> bool {
        self.gpu_cull
    }
    pub fn set_materials(&mut self, materials: VoxelMaterials) {
        self.materials = materials;
        self.resources_generation += 1;
    }
    pub fn materials(&self) -> &VoxelMaterials {
        &self.materials
    }
    /// Places (or moves) a grid.
    pub fn set_grid(&mut self, grid: GridId, transform: GridTransform, cell_size: f64) {
        match self.grid_index.get(&grid) {
            Some(&slot) => self.grid_slots[slot as usize] = Some((grid, transform, cell_size)),
            None => {
                let slot = match self.grid_slots.iter().position(Option::is_none) {
                    Some(free) => {
                        self.grid_slots[free] = Some((grid, transform, cell_size));
                        free
                    }
                    None => {
                        self.grid_slots.push(Some((grid, transform, cell_size)));
                        self.grid_slots.len() - 1
                    }
                };
                self.grid_index.insert(grid, slot as u32);
            }
        }
    }
    /// Removes a grid and all its geometry.
    pub fn remove_grid(&mut self, grid: GridId) {
        let keys: Vec<ChunkKey> = self
            .chunks
            .keys()
            .filter(|k| k.grid == grid)
            .copied()
            .collect();
        for key in keys {
            self.remove(key);
        }
        if let Some(slot) = self.grid_index.remove(&grid) {
            self.grid_slots[slot as usize] = None;
        }
    }
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
    pub fn contains(&self, key: ChunkKey) -> bool {
        self.chunks.contains_key(&key)
    }
    /// Restricts colour passes for `grid` to `chunks` (for example the
    /// result of a `quasar_voxel_view::VisibilityGraph` traversal), or draws
    /// every chunk again with `None`. Shadow passes ignore the filter.
    pub fn set_visible_chunks(&mut self, grid: GridId, chunks: Option<&[IVec3]>) {
        match chunks {
            Some(chunks) => {
                let set = self.visible_filters.entry(grid).or_default();
                if set.len() == chunks.len() && chunks.iter().all(|c| set.contains(c)) {
                    return;
                }
                set.clear();
                set.extend(chunks.iter().copied());
            }
            None => {
                if self.visible_filters.remove(&grid).is_none() {
                    return;
                }
            }
        }
        self.draw_version += 1;
    }

    /// Bytes of geometry currently allocated.
    pub fn geometry_bytes(&self) -> u64 {
        self.streams.iter().map(|s| s.arena.used()).sum()
    }
    pub fn page_count(&self) -> usize {
        self.streams.iter().map(|s| s.arena.page_count()).sum()
    }

    fn allocate(&mut self, gpu: &Gpu, stream: u8, bytes: &[u8]) -> GraphicsResult<PageAllocation> {
        let s = &mut self.streams[stream as usize];
        let (allocation, new_page) = s
            .arena
            // Ranges are addressed by element index, so they must start on an
            // element boundary (12, 80 and 16 bytes are all 4-byte multiples,
            // as buffer writes require).
            .allocate(bytes.len() as u64, ELEMENT_BYTES[stream as usize])
            .ok_or_else(|| error("voxel geometry larger than a page"))?;
        if new_page {
            let mut usage = BufferUsages::STORAGE | BufferUsages::COPY_DST;
            if stream == STREAM_SMOOTH {
                usage |= BufferUsages::INDEX;
            }
            s.buffers.push(gpu.device.create_buffer(&BufferDescriptor {
                label: Some("voxel geometry page"),
                size: self.config.page_bytes,
                usage,
                mapped_at_creation: false,
            }));
        }
        gpu.queue.write_buffer(
            &s.buffers[allocation.page as usize],
            allocation.range.offset,
            bytes,
        );
        Ok(allocation)
    }

    fn bounds(points: impl Iterator<Item = Vec3>) -> (Vec3, Vec3) {
        points.fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(lo, hi), p| (lo.min(p), hi.max(p)),
        )
    }

    /// Uploads (replacing) a chunk's geometry. Translucent layers keep a CPU
    /// copy for re-sorting. Returns what was uploaded.
    pub fn upload(
        &mut self,
        gpu: &Gpu,
        key: ChunkKey,
        placement: ChunkPlacement,
        mesh: &MeshBuffers,
    ) -> GraphicsResult<UploadStats> {
        let Some(&grid_slot) = self.grid_index.get(&key.grid) else {
            return Err(error(
                "voxel upload for an unknown grid; call set_grid first",
            ));
        };
        if !(placement.scale > 0.0 && placement.scale.is_finite()) {
            return Err(error("chunk placement scale must be positive"));
        }
        self.remove(key);
        let instance = match self.free_instances.pop() {
            Some(slot) => slot,
            None => {
                self.instances.push([0; 8]);
                (self.instances.len() - 1) as u32
            }
        };
        let origin = placement.origin.as_vec3();
        self.instances[instance as usize] = [
            origin.x.to_bits(),
            origin.y.to_bits(),
            origin.z.to_bits(),
            placement.scale.to_bits(),
            grid_slot,
            0,
            0,
            0,
        ];
        self.instances_dirty = true;
        let mut data = ChunkData {
            placement,
            instance,
            allocations: Vec::new(),
            translucent: Vec::new(),
        };
        let mut stats = UploadStats::default();
        let to_grid = |p: Vec3| origin + p * placement.scale;
        // Reused staging bytes: steady-state uploads do not allocate (except
        // the CPU copy translucent layers keep for re-sorting).
        let mut bytes = std::mem::take(&mut self.upload_scratch);
        for (layer, buffers) in &mesh.layers {
            let draw_key = |stream: u8| DrawKey {
                grid: key.grid,
                chunk: key.chunk,
                level: key.level,
                layer: *layer,
                stream,
            };
            let packable = |q: &Quad| *layer != Layer::Translucent && PackedQuad::encode(q).is_ok();
            bytes.clear();
            let mut packed = 0usize;
            let mut model = 0usize;
            let mut bounds = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for quad in &buffers.quads {
                match (*layer != Layer::Translucent).then(|| PackedQuad::encode(quad)) {
                    Some(Ok(p)) => {
                        bytes.extend(p.0.iter().flat_map(|w| w.to_ne_bytes()));
                        packed += 1;
                        for v in quad.positions {
                            let g = to_grid(v);
                            bounds = (bounds.0.min(g), bounds.1.max(g));
                        }
                    }
                    _ => model += 1,
                }
            }
            if packed > 0 {
                let allocation = self.allocate(gpu, STREAM_PACKED, &bytes)?;
                let k = draw_key(STREAM_PACKED);
                self.draws.insert(DrawRecord {
                    key: k,
                    page: allocation.page,
                    first: (allocation.range.offset / ELEMENT_BYTES[STREAM_PACKED as usize]) as u32,
                    count: packed as u32,
                    base_vertex: 0,
                    min: bounds.0,
                    max: bounds.1,
                    instance,
                });
                data.allocations.push((k, STREAM_PACKED, allocation));
                stats.packed_quads += packed;
                stats.bytes += bytes.len();
            }
            if model > 0 {
                bytes.clear();
                let mut bounds = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                for quad in buffers.quads.iter().filter(|q| !packable(q)) {
                    bytes.extend(
                        ModelVertex::quad(quad)
                            .iter()
                            .flat_map(|v| v.0)
                            .flat_map(u32::to_ne_bytes),
                    );
                    for v in quad.positions {
                        let g = to_grid(v);
                        bounds = (bounds.0.min(g), bounds.1.max(g));
                    }
                }
                let allocation = self.allocate(gpu, STREAM_MODEL, &bytes)?;
                let k = draw_key(STREAM_MODEL);
                self.draws.insert(DrawRecord {
                    key: k,
                    page: allocation.page,
                    first: (allocation.range.offset / ELEMENT_BYTES[STREAM_MODEL as usize]) as u32,
                    count: model as u32,
                    base_vertex: 0,
                    min: bounds.0,
                    max: bounds.1,
                    instance,
                });
                data.allocations.push((k, STREAM_MODEL, allocation));
                stats.model_quads += model;
                stats.bytes += bytes.len();
                if *layer == Layer::Translucent {
                    data.translucent.push(Translucent {
                        quads: buffers
                            .quads
                            .iter()
                            .filter(|q| !packable(q))
                            .copied()
                            .collect(),
                        sorter: PlaneSorter::default(),
                        order: Vec::new(),
                        allocation,
                    });
                }
            }
            if !buffers.indices.is_empty() {
                bytes.clear();
                bytes.extend(
                    buffers
                        .vertices
                        .iter()
                        .flat_map(|v| PackedSmoothVertex::encode(v).0)
                        .flat_map(u32::to_ne_bytes),
                );
                let vertex_bytes = bytes.len() as u64;
                bytes.extend(buffers.indices.iter().flat_map(|i| i.to_ne_bytes()));
                let allocation = self.allocate(gpu, STREAM_SMOOTH, &bytes)?;
                let (min, max) = Self::bounds(buffers.vertices.iter().map(|v| to_grid(v.position)));
                let k = draw_key(STREAM_SMOOTH);
                self.draws.insert(DrawRecord {
                    key: k,
                    page: allocation.page,
                    first: ((allocation.range.offset + vertex_bytes) / 4) as u32,
                    count: buffers.indices.len() as u32,
                    base_vertex: (allocation.range.offset / 16) as i32,
                    min,
                    max,
                    instance,
                });
                data.allocations.push((k, STREAM_SMOOTH, allocation));
                stats.smooth_triangles += buffers.indices.len() / 3;
                stats.bytes += bytes.len();
            }
        }
        self.upload_scratch = bytes;
        self.draw_version += 1;
        if !data.translucent.is_empty() {
            self.translucent_keys.insert(key);
        }
        self.chunks.insert(key, data);
        Ok(stats)
    }

    /// Removes a chunk's geometry; its memory is reused after the current
    /// frame is retired.
    pub fn remove(&mut self, key: ChunkKey) -> bool {
        let Some(data) = self.chunks.remove(&key) else {
            return false;
        };
        self.translucent_keys.remove(&key);
        self.draw_version += 1;
        for (draw_key, stream, allocation) in data.allocations {
            self.draws.remove(&draw_key);
            self.streams[stream as usize]
                .arena
                .free_after(allocation, self.frame);
        }
        self.free_instances.push(data.instance);
        true
    }

    /// Releases memory freed up to and including frame `completed`.
    pub fn retire(&mut self, completed: u64) -> usize {
        self.streams
            .iter_mut()
            .map(|s| s.arena.retire(completed))
            .sum()
    }

    /// Records that the commands of `submission` (a [`PreparedVoxels`]
    /// number) were submitted; [`Self::retire_completed`] releases its frees
    /// once the GPU finishes them. The live-loop alternative to [`Self::retire`].
    pub fn submitted(&mut self, gpu: &Gpu, submission: u64) {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        gpu.queue
            .on_submitted_work_done(move || flag.store(true, std::sync::atomic::Ordering::Release));
        self.completions.push_back((submission, done));
    }

    /// Polls the device and retires every submission reported complete
    /// through [`Self::submitted`]. Never blocks.
    pub fn retire_completed(&mut self, gpu: &Gpu) -> usize {
        let _ = gpu.device.poll(PollType::Poll);
        let mut completed = None;
        while let Some((submission, done)) = self.completions.front() {
            if !done.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            completed = Some(*submission);
            self.completions.pop_front();
        }
        completed.map_or(0, |c| self.retire(c))
    }

    /// Waits for the GPU and retires every frame (headless tools, tests).
    pub fn wait_and_retire(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        gpu.wait()?;
        self.completions.clear();
        self.retire(u64::MAX);
        Ok(())
    }

    fn pipeline(&mut self, gpu: &Gpu, stream: u8, kind: PassKind) -> GraphicsResult<()> {
        if self.pipelines.contains_key(&(stream, kind)) {
            return Ok(());
        }
        let entry_point = match stream {
            STREAM_PACKED => "vs_packed",
            STREAM_MODEL => "vs_model",
            _ => "vs_smooth",
        };
        let layer_constant = [(
            "LAYER",
            match kind {
                PassKind::Color { layer, .. } => f64::from(layer),
                PassKind::Depth => 0.0,
            },
        )];
        let (fragment_targets, depth_write, cull, bias) = match kind {
            PassKind::Color { format, layer } => (
                Some([Some(ColorTargetState {
                    format,
                    blend: (layer == 2).then_some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })]),
                layer != 2,
                // Cutout foliage is double-sided; solid and glass cubes cull back faces.
                if layer == 1 { None } else { Some(Face::Back) },
                DepthBiasState::default(),
            ),
            PassKind::Depth => (
                None,
                true,
                None,
                DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            ),
        };
        let pipeline = checked(gpu, || {
            gpu.device
                .create_render_pipeline(&RenderPipelineDescriptor {
                    label: Some("voxel pipeline"),
                    layout: Some(&self.pipeline_layout),
                    vertex: VertexState {
                        module: &self.module,
                        entry_point: Some(entry_point),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: PrimitiveState {
                        cull_mode: cull,
                        front_face: FrontFace::Ccw,
                        ..Default::default()
                    },
                    depth_stencil: Some(DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(depth_write),
                        depth_compare: Some(CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias,
                    }),
                    multisample: Default::default(),
                    fragment: fragment_targets.as_ref().map(|targets| FragmentState {
                        module: &self.module,
                        entry_point: Some("fs_main"),
                        compilation_options: PipelineCompilationOptions {
                            constants: &layer_constant,
                            ..Default::default()
                        },
                        targets,
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })?;
        self.pipelines.insert((stream, kind), pipeline);
        Ok(())
    }

    fn overlay_pipeline(&mut self, gpu: &Gpu, format: TextureFormat) -> GraphicsResult<()> {
        if self.overlay_pipelines.contains_key(&format) {
            return Ok(());
        }
        let layout = gpu
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("voxel overlay"),
                bind_group_layouts: &[Some(&self.layout), Some(&self.overlay_layout)],
                immediate_size: 0,
            });
        let pipeline = checked(gpu, || {
            gpu.device
                .create_render_pipeline(&RenderPipelineDescriptor {
                    label: Some("voxel overlay"),
                    layout: Some(&layout),
                    vertex: VertexState {
                        module: &self.module,
                        entry_point: Some("vs_overlay"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: PrimitiveState {
                        topology: PrimitiveTopology::LineList,
                        ..Default::default()
                    },
                    depth_stencil: Some(DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: DepthBiasState::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(FragmentState {
                        module: &self.module,
                        entry_point: Some("fs_overlay"),
                        compilation_options: Default::default(),
                        targets: &[Some(ColorTargetState {
                            format,
                            blend: Some(BlendState::ALPHA_BLENDING),
                            write_mask: ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        })?;
        self.overlay_pipelines.insert(format, pipeline);
        Ok(())
    }

    fn grow(
        gpu: &Gpu,
        slot: &mut Option<Buffer>,
        label: &str,
        size: u64,
        usage: BufferUsages,
    ) -> GraphicsResult<bool> {
        quasar_frame::grow_buffer(
            gpu,
            slot,
            label,
            size.max(64),
            usage | BufferUsages::COPY_DST,
        )
    }

    /// Uploads per-frame tables through staging copies in `encoder` so every
    /// pass of a frame sees the values it was prepared with.
    #[allow(clippy::too_many_arguments)]
    fn write_frame(
        &mut self,
        gpu: &Gpu,
        encoder: &mut CommandEncoder,
        origin: DVec3,
        view_projection: Mat4,
        lighting: Option<(Lighting<'_>, Option<&ShadowBinding>)>,
        shading: &VoxelShading,
    ) -> GraphicsResult<()> {
        let mut changed = false;
        // Grid table (camera-relative, computed in f64).
        let grids: Vec<f32> = self
            .grid_slots
            .iter()
            .flat_map(|slot| match slot {
                Some((_, transform, cell)) => {
                    transform.camera_relative(*cell, origin).to_cols_array()
                }
                None => Mat4::IDENTITY.to_cols_array(),
            })
            .collect();
        let grid_bytes = f32_bytes(grids);
        changed |= Self::grow(
            gpu,
            &mut self.grid_buffer,
            "voxel grids",
            grid_bytes.len() as u64,
            BufferUsages::STORAGE,
        )?;
        if !grid_bytes.is_empty() {
            let staging = quasar_frame::staging_buffer(gpu, "voxel grids", &grid_bytes);
            encoder.copy_buffer_to_buffer(
                &staging,
                0,
                self.grid_buffer.as_ref().unwrap(),
                0,
                grid_bytes.len() as u64,
            );
        }
        let instance_len = (self.instances.len() * 32) as u64;
        let grew = Self::grow(
            gpu,
            &mut self.instance_buffer,
            "voxel instances",
            instance_len,
            BufferUsages::STORAGE,
        )?;
        changed |= grew;
        if (self.instances_dirty || grew) && instance_len > 0 {
            let instance_bytes: Vec<u8> = self
                .instances
                .iter()
                .flatten()
                .flat_map(|v| v.to_ne_bytes())
                .collect();
            let staging = quasar_frame::staging_buffer(gpu, "voxel instances", &instance_bytes);
            encoder.copy_buffer_to_buffer(
                &staging,
                0,
                self.instance_buffer.as_ref().unwrap(),
                0,
                instance_bytes.len() as u64,
            );
            self.instances_dirty = false;
        }
        // Lights rebased to the camera so positions stay precise; fog height
        // moves with it (positions in the shader are camera-relative).
        let camera = LitCamera3D {
            view_projection: quasar_render_3d::Mat4::from_glam(view_projection),
            position: [0.0; 3],
        };
        let options = LightingOptions {
            fog: shading.fog.map(|f| f.rebased(origin.y as f32)),
            toon: shading.toon,
            ..LightingOptions::default()
        };
        let (light_bytes, shadow_bytes) = match lighting {
            Some((lighting, shadow)) => {
                let rebased: Vec<Light> = lighting
                    .lights
                    .iter()
                    .map(|light| match *light {
                        Light::Point {
                            position,
                            color,
                            intensity,
                            range,
                            attenuation,
                        } => Light::Point {
                            position: (DVec3::from(position.map(f64::from)) - origin)
                                .as_vec3()
                                .to_array(),
                            color,
                            intensity,
                            range,
                            attenuation,
                        },
                        Light::Spot {
                            position,
                            direction,
                            color,
                            intensity,
                            range,
                            inner_angle,
                            outer_angle,
                            attenuation,
                            cookie,
                        } => Light::Spot {
                            position: (DVec3::from(position.map(f64::from)) - origin)
                                .as_vec3()
                                .to_array(),
                            direction,
                            color,
                            intensity,
                            range,
                            inner_angle,
                            outer_angle,
                            attenuation,
                            cookie,
                        },
                        other => other,
                    })
                    .collect();
                let lights = quasar_render_3d::encode_lighting_with(
                    camera,
                    Lighting {
                        lights: &rebased,
                        ..lighting
                    },
                    self.config.max_lights,
                    &options,
                )?;
                let shadows = shadow.map_or_else(
                    || f32_bytes([0.0f32; 28]),
                    |s| s.records_bytes_rebased(origin.to_array()),
                );
                (lights, shadows)
            }
            // No scene lights: the header still carries fog and toon bands.
            None => (
                quasar_render_3d::encode_lighting_with(
                    camera,
                    Lighting {
                        ambient: [0.0; 3],
                        lights: &[],
                        output_color_space: ColorSpace::Linear,
                    },
                    self.config.max_lights,
                    &options,
                )?,
                f32_bytes([0.0f32; 28]),
            ),
        };
        changed |= Self::grow(
            gpu,
            &mut self.lights_buffer,
            "voxel lights",
            light_bytes.len() as u64,
            BufferUsages::STORAGE,
        )?;
        changed |= Self::grow(
            gpu,
            &mut self.shadow_buffer,
            "voxel shadows",
            shadow_bytes.len() as u64,
            BufferUsages::STORAGE,
        )?;
        let staging = quasar_frame::staging_buffer(gpu, "voxel lights", &light_bytes);
        encoder.copy_buffer_to_buffer(
            &staging,
            0,
            self.lights_buffer.as_ref().unwrap(),
            0,
            light_bytes.len() as u64,
        );
        let staging = quasar_frame::staging_buffer(gpu, "voxel shadows", &shadow_bytes);
        encoder.copy_buffer_to_buffer(
            &staging,
            0,
            self.shadow_buffer.as_ref().unwrap(),
            0,
            shadow_bytes.len() as u64,
        );
        let model = shading.model.code();
        let fog = shading.fog;
        let mut frame: Vec<f32> = view_projection.to_cols_array().to_vec();
        frame.extend([
            shading.ao_strength,
            f32::from(shading.voxel_light),
            shading.sky_brightness,
            model,
        ]);
        frame.extend([
            shading.specular[0],
            shading.specular[1],
            shading.specular[2],
            shading.shininess,
        ]);
        let fog_color = fog.map_or([0.0; 3], |f| f.color);
        frame.extend([
            fog_color[0],
            fog_color[1],
            fog_color[2],
            f32::from(fog.is_some()),
        ]);
        frame.extend([
            fog.map_or(0.0, |f| f.start),
            fog.map_or(1.0, |f| f.end),
            shading.min_light,
            0.0,
        ]);
        let frame = f32_bytes(frame);
        debug_assert_eq!(frame.len() as u64, FRAME_BYTES);
        let staging = quasar_frame::staging_buffer(gpu, "voxel frame", &frame);
        encoder.copy_buffer_to_buffer(&staging, 0, &self.frame_buffer, 0, FRAME_BYTES);
        if changed {
            self.resources_generation += 1;
        }
        Ok(())
    }

    fn group(
        &mut self,
        gpu: &Gpu,
        stream: u8,
        page: u32,
        shadow: Option<&TextureView>,
    ) -> BindGroup {
        let generation = self.resources_generation ^ (self.materials.generation << 32);
        if generation != self.groups_generation || self.groups.len() > 512 {
            self.groups.clear();
            self.groups_generation = generation;
        }
        let key = (stream, page, shadow.cloned());
        if let Some(group) = self.groups.get(&key) {
            return group.clone();
        }
        let shadow_view = shadow.unwrap_or(&self.empty_shadow);
        let group = gpu.device.create_bind_group(&BindGroupDescriptor {
            label: Some("voxel draw"),
            layout: &self.layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&self.materials.view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&self.materials.sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: self.materials.table.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: self.lights_buffer.as_ref().unwrap().as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::TextureView(shadow_view),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: self.shadow_buffer.as_ref().unwrap().as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: self.instance_buffer.as_ref().unwrap().as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 8,
                    resource: self.streams[stream as usize].buffers[page as usize]
                        .as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 9,
                    resource: self.frame_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 10,
                    resource: self.grid_buffer.as_ref().unwrap().as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 21,
                    resource: BindingResource::Sampler(&self.comparison),
                },
            ],
        });
        self.groups.insert(key, group.clone());
        group
    }

    fn grid_matrix(&self, slot: u32, origin: DVec3) -> Mat4 {
        match self.grid_slots.get(slot as usize).and_then(Option::as_ref) {
            Some((_, t, cell)) => t.camera_relative(*cell, origin),
            None => Mat4::IDENTITY,
        }
    }

    fn instance_grid(&self, instance: u32) -> u32 {
        self.instances[instance as usize][4]
    }

    /// Re-sorts translucent chunks whose camera side changed.
    fn resort(&mut self, gpu: &Gpu, camera: DVec3) -> usize {
        let mut resorted = 0;
        let mut bytes = std::mem::take(&mut self.upload_scratch);
        for &key in &self.translucent_keys {
            let Some(&slot) = self.grid_index.get(&key.grid) else {
                continue;
            };
            let Some((_, transform, cell)) = self.grid_slots[slot as usize] else {
                continue;
            };
            let data = self.chunks.get_mut(&key).unwrap();
            if data.translucent.is_empty() {
                continue;
            }
            let voxel = transform.rotation.inverse() * (camera - transform.position) / cell;
            let local = ((voxel - data.placement.origin.as_dvec3())
                / f64::from(data.placement.scale))
            .as_vec3();
            for t in &mut data.translucent {
                if t.sorter.sort(&t.quads, local, &mut t.order) {
                    bytes.clear();
                    bytes.extend(
                        t.order
                            .iter()
                            .flat_map(|i| ModelVertex::quad(&t.quads[*i as usize]))
                            .flat_map(|v| v.0)
                            .flat_map(u32::to_ne_bytes),
                    );
                    let buffer =
                        &self.streams[STREAM_MODEL as usize].buffers[t.allocation.page as usize];
                    gpu.queue
                        .write_buffer(buffer, t.allocation.range.offset, &bytes);
                    resorted += 1;
                }
            }
        }
        self.upload_scratch = bytes;
        resorted
    }

    fn depth_view(&mut self, gpu: &Gpu, target: &RenderTarget) -> GraphicsResult<TextureView> {
        if let Some(depth) = &target.depth {
            return Ok(depth.clone());
        }
        if !self.depths.contains_key(&target.color) {
            if self.depths.len() >= 8 {
                self.depths.clear();
            }
            self.depths.insert(
                target.color.clone(),
                texture_2d(
                    &gpu.device,
                    "voxel depth",
                    target.width,
                    target.height,
                    DEPTH_FORMAT,
                    TextureUsages::RENDER_ATTACHMENT,
                )?,
            );
        }
        Ok(self.depths[&target.color].create_view(&Default::default()))
    }

    /// Records the colour pass (opaque, cutout, translucent, overlays) into
    /// one command buffer. Submit it after passes it must draw over, and call
    /// [`Self::retire`] with `submission` once that work is complete.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        camera: &VoxelCamera,
        lighting: Lighting<'_>,
        shadow: Option<&ShadowBinding>,
        shading: &VoxelShading,
        options: &VoxelPassOptions,
    ) -> GraphicsResult<PreparedVoxels> {
        checked(gpu, || {
            self.prepare_inner(gpu, target, camera, lighting, shadow, shading, options)
        })?
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_inner(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        camera: &VoxelCamera,
        lighting: Lighting<'_>,
        shadow: Option<&ShadowBinding>,
        shading: &VoxelShading,
        options: &VoxelPassOptions,
    ) -> GraphicsResult<PreparedVoxels> {
        self.frame += 1;
        let origin = camera.position;
        let vp = camera.relative_view_projection();
        if !vp.is_finite() {
            return Err(error("invalid voxel camera"));
        }
        let planes = frustum_planes(vp);
        let mut stats = VoxelStats {
            resorted_chunks: if options.translucent {
                self.resort(gpu, camera.position)
            } else {
                0
            },
            ..VoxelStats::default()
        };
        let depth = self.depth_view(gpu, target)?;
        let shadow_view = shadow.map(|s| s.texture_view().clone());
        let mut encoder = gpu.encoder("voxels");
        self.write_frame(
            gpu,
            &mut encoder,
            origin,
            vp,
            Some((lighting, shadow)),
            shading,
        )?;
        let layers: Vec<(u8, Layer)> = [
            (0u8, Layer::Opaque, options.opaque),
            (1, Layer::Cutout, options.cutout),
            (2, Layer::Translucent, options.translucent),
        ]
        .into_iter()
        .filter(|(_, _, on)| *on)
        .map(|(i, l, _)| (i, l))
        .collect();
        // Record lists and GPU cull input only change when geometry or the
        // visibility filter does: cache them so a static scene costs one
        // plane upload and a dispatch per frame.
        let layer_ids: Vec<u8> = layers.iter().map(|(i, _)| *i).collect();
        let fresh = matches!(&self.cull_cache, Some(c) if c.version == self.draw_version && c.layers == layer_ids);
        let mut upload_records: Option<Vec<u8>> = None;
        if !fresh {
            let mut cache = CullCache {
                version: self.draw_version,
                layers: layer_ids.clone(),
                ..CullCache::default()
            };
            let mut cull_bytes: Vec<u8> = Vec::new();
            let filters = &self.visible_filters;
            let mut grouped: BTreeMap<(u8, u8, u32), Vec<DrawRecord>> = BTreeMap::new();
            for r in self.draws.records() {
                let Some(&(layer_index, _)) = layers.iter().find(|(_, l)| *l == r.key.layer) else {
                    continue;
                };
                if !filters
                    .get(&r.key.grid)
                    .is_none_or(|set| set.contains(&r.key.chunk))
                {
                    cache.occlusion_culled += 1;
                    continue;
                }
                cache.records += 1;
                if self.gpu_cull && layer_index < 2 && r.key.stream != STREAM_SMOOTH {
                    grouped
                        .entry((layer_index, r.key.stream, r.page))
                        .or_default()
                        .push(*r);
                } else {
                    cache.cpu.push((layer_index, *r));
                }
            }
            for ((layer_index, stream, page), group) in grouped {
                let start = cache.cull_count;
                let mut triangles = 0;
                for r in &group {
                    let grid = self.instance_grid(r.instance);
                    let values: [u32; 12] = [
                        r.min.x.to_bits(),
                        r.min.y.to_bits(),
                        r.min.z.to_bits(),
                        0,
                        r.max.x.to_bits(),
                        r.max.y.to_bits(),
                        r.max.z.to_bits(),
                        0,
                        r.first * 6,
                        r.count * 6,
                        r.instance,
                        grid,
                    ];
                    cull_bytes.extend(values.iter().flat_map(|v| v.to_ne_bytes()));
                    triangles += r.count as usize * 2;
                    cache.cull_count += 1;
                }
                cache.indirect.push((
                    layer_index,
                    stream,
                    page,
                    u64::from(start) * 16,
                    cache.cull_count - start,
                    triangles,
                ));
            }
            upload_records = Some(cull_bytes);
            self.cull_cache = Some(cache);
        }
        let cache = self.cull_cache.take().expect("cull cache");
        stats.records = cache.records;
        stats.occlusion_culled = cache.occlusion_culled;
        let indirect = &cache.indirect;
        let cull_count = cache.cull_count;
        // (layer, record) in draw order
        let mut direct: Vec<(u8, DrawRecord)> = Vec::new();
        for &(layer_index, layer) in &layers {
            let mut visible: Vec<DrawRecord> = Vec::new();
            for (l, r) in &cache.cpu {
                if *l != layer_index {
                    continue;
                }
                let model = self.grid_matrix(self.instance_grid(r.instance), origin);
                if box_visible(&planes, &model, r.min, r.max) {
                    visible.push(*r);
                } else {
                    stats.cpu_culled += 1;
                }
            }
            if layer == Layer::Translucent {
                let mut keyed: Vec<(f32, DrawRecord)> = visible
                    .into_iter()
                    .map(|r| {
                        let model = self.grid_matrix(self.instance_grid(r.instance), origin);
                        (
                            model
                                .transform_point3((r.min + r.max) * 0.5)
                                .length_squared(),
                            r,
                        )
                    })
                    .collect();
                keyed.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.key.cmp(&b.1.key)));
                visible = keyed.into_iter().map(|(_, r)| r).collect();
            }
            direct.extend(visible.into_iter().map(|r| (layer_index, r)));
        }
        if cull_count > 0 {
            if let Some(cull_bytes) = &upload_records {
                let grew_records = Self::grow(
                    gpu,
                    &mut self.cull_records,
                    "voxel cull records",
                    cull_bytes.len() as u64,
                    BufferUsages::STORAGE,
                )?;
                let grew_args = Self::grow(
                    gpu,
                    &mut self.cull_args,
                    "voxel indirect args",
                    u64::from(cull_count) * 16,
                    BufferUsages::STORAGE | BufferUsages::INDIRECT,
                )?;
                if grew_records || grew_args {
                    self.resources_generation += 1;
                    self.cull_group = None;
                }
                let staging = quasar_frame::staging_buffer(gpu, "voxel cull records", cull_bytes);
                encoder.copy_buffer_to_buffer(
                    &staging,
                    0,
                    self.cull_records.as_ref().unwrap(),
                    0,
                    cull_bytes.len() as u64,
                );
            }
            let mut plane_bytes = f32_bytes(planes.iter().flatten().copied());
            plane_bytes.extend([cull_count, 6, 0, 0].iter().flat_map(|v| v.to_ne_bytes()));
            let staging = quasar_frame::staging_buffer(gpu, "voxel cull planes", &plane_bytes);
            encoder.copy_buffer_to_buffer(&staging, 0, &self.cull_planes, 0, 112);
            if self
                .cull_group
                .as_ref()
                .is_none_or(|(generation, _)| *generation != self.resources_generation)
            {
                let group = gpu.device.create_bind_group(&BindGroupDescriptor {
                    label: Some("voxel cull"),
                    layout: &self.cull_layout,
                    entries: &[
                        BindGroupEntry {
                            binding: 0,
                            resource: self.cull_records.as_ref().unwrap().as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: self.grid_buffer.as_ref().unwrap().as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: self.cull_planes.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: self.cull_args.as_ref().unwrap().as_entire_binding(),
                        },
                    ],
                });
                self.cull_group = Some((self.resources_generation, group));
            }
            let group = self
                .cull_group
                .as_ref()
                .map(|(_, g)| g.clone())
                .expect("cull group");
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("voxel cull"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.cull_pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(cull_count.div_ceil(64), 1, 1);
        }
        for &(layer_index, _) in &layers {
            for stream in [STREAM_PACKED, STREAM_MODEL, STREAM_SMOOTH] {
                self.pipeline(
                    gpu,
                    stream,
                    PassKind::Color {
                        format: target.format,
                        layer: layer_index,
                    },
                )?;
            }
        }
        let mut groups: Vec<(u8, u32, BindGroup)> = Vec::new();
        let needed: Vec<(u8, u32)> = indirect
            .iter()
            .map(|(_, s, p, _, _, _)| (*s, *p))
            .chain(direct.iter().map(|(_, r)| (r.key.stream, r.page)))
            .collect();
        for (stream, page) in needed {
            if !groups.iter().any(|(s, p, _)| *s == stream && *p == page) {
                let group = self.group(gpu, stream, page, shadow_view.as_ref());
                groups.push((stream, page, group));
            }
        }
        if !options.overlays.is_empty() {
            self.overlay_pipeline(gpu, target.format)?;
        }
        let overlay =
            self.overlay_resources(gpu, &mut encoder, &options.overlays, shadow_view.as_ref())?;
        {
            let colors = [Some(RenderPassColorAttachment {
                view: &target.color,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: match options.clear_color {
                        Some(c) => LoadOp::Clear(Color {
                            r: c[0] as f64,
                            g: c[1] as f64,
                            b: c[2] as f64,
                            a: c[3] as f64,
                        }),
                        None => LoadOp::Load,
                    },
                    store: StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("voxels"),
                color_attachments: &colors,
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(Operations {
                        load: if options.clear_depth {
                            LoadOp::Clear(1.0)
                        } else {
                            LoadOp::Load
                        },
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let group_of = |stream: u8, page: u32| {
                &groups
                    .iter()
                    .find(|(s, p, _)| *s == stream && *p == page)
                    .unwrap()
                    .2
            };
            for &(layer_index, _) in &layers {
                let kind = PassKind::Color {
                    format: target.format,
                    layer: layer_index,
                };
                for (l, stream, page, offset, count, triangles) in indirect.iter() {
                    if *l != layer_index {
                        continue;
                    }
                    pass.set_pipeline(&self.pipelines[&(*stream, kind)]);
                    pass.set_bind_group(0, group_of(*stream, *page), &[]);
                    pass.multi_draw_indirect(self.cull_args.as_ref().unwrap(), *offset, *count);
                    stats.indirect_batches += 1;
                    stats.triangles += triangles;
                }
                for (l, r) in &direct {
                    if *l != layer_index {
                        continue;
                    }
                    pass.set_pipeline(&self.pipelines[&(r.key.stream, kind)]);
                    pass.set_bind_group(0, group_of(r.key.stream, r.page), &[]);
                    if r.key.stream == STREAM_SMOOTH {
                        let buffer = &self.streams[STREAM_SMOOTH as usize].buffers[r.page as usize];
                        pass.set_index_buffer(buffer.slice(..), IndexFormat::Uint32);
                        pass.draw_indexed(
                            r.first..r.first + r.count,
                            r.base_vertex,
                            r.instance..r.instance + 1,
                        );
                        stats.triangles += r.count as usize / 3;
                    } else {
                        pass.draw(
                            r.first * 6..(r.first + r.count) * 6,
                            r.instance..r.instance + 1,
                        );
                        stats.triangles += r.count as usize * 2;
                    }
                    stats.direct_draws += 1;
                }
            }
            if let Some((group0, overlay_group)) = &overlay {
                pass.set_pipeline(&self.overlay_pipelines[&target.format]);
                pass.set_bind_group(0, group0, &[]);
                pass.set_bind_group(1, overlay_group, &[]);
                pass.draw(0..options.overlays.len() as u32 * 24, 0..1);
            }
        }
        self.cull_cache = Some(cache);
        Ok(PreparedVoxels {
            commands: encoder.finish(),
            stats,
            submission: self.frame,
        })
    }

    fn overlay_resources(
        &mut self,
        gpu: &Gpu,
        encoder: &mut CommandEncoder,
        overlays: &[OverlayBox],
        shadow: Option<&TextureView>,
    ) -> GraphicsResult<Option<(BindGroup, BindGroup)>> {
        if overlays.is_empty() {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        for b in overlays {
            let grid = self.grid_index.get(&b.grid).copied().unwrap_or(0);
            bytes.extend(f32_bytes([
                b.min.x, b.min.y, b.min.z, 0.0, b.max.x, b.max.y, b.max.z, 0.0,
            ]));
            bytes.extend(f32_bytes(b.color));
            bytes.extend([grid, 0, 0, 0].iter().flat_map(|v| v.to_ne_bytes()));
        }
        if Self::grow(
            gpu,
            &mut self.overlay_buffer,
            "voxel overlays",
            bytes.len() as u64,
            BufferUsages::STORAGE,
        )? {
            self.resources_generation += 1;
        }
        let staging = quasar_frame::staging_buffer(gpu, "voxel overlays", &bytes);
        encoder.copy_buffer_to_buffer(
            &staging,
            0,
            self.overlay_buffer.as_ref().unwrap(),
            0,
            bytes.len() as u64,
        );
        // Group 0 needs some geometry page; overlays only read frame and grids.
        if self.streams.iter().all(|s| s.buffers.is_empty()) {
            let allocation = self.allocate(gpu, STREAM_PACKED, &[0u8; 16])?;
            self.streams[0].arena.free(allocation);
        }
        let stream = (0..3u8)
            .find(|s| !self.streams[*s as usize].buffers.is_empty())
            .unwrap();
        let group0 = self.group(gpu, stream, 0, shadow);
        let overlay_group = gpu.device.create_bind_group(&BindGroupDescriptor {
            label: Some("voxel overlay"),
            layout: &self.overlay_layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: self.overlay_buffer.as_ref().unwrap().as_entire_binding(),
            }],
        });
        Ok(Some((group0, overlay_group)))
    }

    /// Depth-only pass of opaque and cutout geometry into `depth` (e.g. a
    /// render-3d [`quasar_render_3d::ShadowTarget::layer_view`]), with a world-space
    /// light `view_projection` and the frame's camera `origin`.
    pub fn prepare_depth(
        &mut self,
        gpu: &Gpu,
        depth: &TextureView,
        view_projection: DMat4,
        origin: DVec3,
        clear: bool,
    ) -> GraphicsResult<PreparedVoxels> {
        checked(gpu, || {
            self.frame += 1;
            let vp = (view_projection * DMat4::from_translation(origin)).as_mat4();
            let planes = frustum_planes(vp);
            let mut encoder = gpu.encoder("voxel depth");
            self.write_frame(
                gpu,
                &mut encoder,
                origin,
                vp,
                None,
                &VoxelShading::default(),
            )?;
            let mut stats = VoxelStats::default();
            let mut visible = Vec::new();
            for r in self.draws.records() {
                if r.key.layer == Layer::Translucent {
                    continue;
                }
                stats.records += 1;
                let model = self.grid_matrix(self.instance_grid(r.instance), origin);
                if box_visible(&planes, &model, r.min, r.max) {
                    visible.push(*r);
                } else {
                    stats.cpu_culled += 1;
                }
            }
            for stream in [STREAM_PACKED, STREAM_MODEL, STREAM_SMOOTH] {
                self.pipeline(gpu, stream, PassKind::Depth)?;
            }
            let mut groups = Vec::new();
            for r in &visible {
                if !groups
                    .iter()
                    .any(|(s, p, _): &(u8, u32, BindGroup)| *s == r.key.stream && *p == r.page)
                {
                    let g = self.group(gpu, r.key.stream, r.page, None);
                    groups.push((r.key.stream, r.page, g));
                }
            }
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("voxel depth"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: depth,
                        depth_ops: Some(Operations {
                            load: if clear {
                                LoadOp::Clear(1.0)
                            } else {
                                LoadOp::Load
                            },
                            store: StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                for r in &visible {
                    let group = &groups
                        .iter()
                        .find(|(s, p, _)| *s == r.key.stream && *p == r.page)
                        .unwrap()
                        .2;
                    pass.set_pipeline(&self.pipelines[&(r.key.stream, PassKind::Depth)]);
                    pass.set_bind_group(0, group, &[]);
                    if r.key.stream == STREAM_SMOOTH {
                        let buffer = &self.streams[STREAM_SMOOTH as usize].buffers[r.page as usize];
                        pass.set_index_buffer(buffer.slice(..), IndexFormat::Uint32);
                        pass.draw_indexed(
                            r.first..r.first + r.count,
                            r.base_vertex,
                            r.instance..r.instance + 1,
                        );
                        stats.triangles += r.count as usize / 3;
                    } else {
                        pass.draw(
                            r.first * 6..(r.first + r.count) * 6,
                            r.instance..r.instance + 1,
                        );
                        stats.triangles += r.count as usize * 2;
                    }
                    stats.direct_draws += 1;
                }
            }
            Ok(PreparedVoxels {
                commands: encoder.finish(),
                stats,
                submission: self.frame,
            })
        })?
    }
}
