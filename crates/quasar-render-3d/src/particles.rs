//! GPU particles: CPU-described bursts, compute integration and billboards.
//!
//! A [`ParticleSystem`] owns a fixed ring of particle slots on the GPU.
//! [`ParticleSystem::emit`] turns a [`ParticleBurst`] into particles with a
//! seeded generator (the same bursts always produce the same particles) and
//! queues them; [`ParticleSystem::prepare`] uploads the queued slots, runs
//! one compute step over every slot and draws the live ones as camera-facing
//! quads in one instanced draw. When the ring is full, new particles replace
//! the oldest ones.
//!
//! Draw particles into a linear target after the opaque pass, normally the
//! [`SceneTarget`]: they depth-test against its depth without writing it.
//! Additive particles (sparks, glows) are order-independent; alpha-blended
//! particles are drawn in slot order, which suits small soft particles such
//! as dust and smoke puffs.
use super::*;

/// Bytes of one particle slot (five `vec4<f32>`).
const PARTICLE_BYTES: u64 = 80;

/// One spawn request: `count` particles from a point with randomised
/// direction, speed, lifetime and placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleBurst {
    pub position: [f32; 3],
    /// Random offset radius around `position`.
    pub position_jitter: f32,
    pub count: u32,
    /// Main emission direction (normalised internally).
    pub direction: [f32; 3],
    /// Cone half-angle around `direction` in radians; π emits in all
    /// directions.
    pub spread: f32,
    /// Speed range in units per second.
    pub speed: [f32; 2],
    /// Lifetime range in seconds.
    pub life: [f32; 2],
    /// Full quad width at birth and at death.
    pub size: [f32; 2],
    /// Linear RGBA at birth and at death; alpha fades the particle.
    pub color_start: [f32; 4],
    pub color_end: [f32; 4],
}

impl Default for ParticleBurst {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            position_jitter: 0.,
            count: 16,
            direction: [0., 1., 0.],
            spread: std::f32::consts::PI,
            speed: [1., 2.],
            life: [0.5, 1.],
            size: [0.1, 0.02],
            color_start: [1., 0.8, 0.4, 1.],
            color_end: [1., 0.2, 0., 0.],
        }
    }
}

impl ParticleBurst {
    fn valid(&self) -> bool {
        let finite = self
            .position
            .iter()
            .chain(&self.direction)
            .chain(&self.speed)
            .chain(&self.life)
            .chain(&self.size)
            .chain(&self.color_start)
            .chain(&self.color_end)
            .chain([&self.position_jitter, &self.spread])
            .all(|v| v.is_finite());
        finite
            && self.direction.iter().map(|v| v * v).sum::<f32>() > 1e-12
            && self.position_jitter >= 0.
            && (0. ..=std::f32::consts::PI).contains(&self.spread)
            && self.speed[0] >= 0.
            && self.speed[0] <= self.speed[1]
            && self.life[0] > 0.
            && self.life[0] <= self.life[1]
            && self.size.iter().all(|s| *s >= 0.)
    }
}

/// Forces applied to every particle of a system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleSettings {
    /// Acceleration in units per second squared.
    pub gravity: [f32; 3],
    /// Fraction of velocity lost per second (0 = none).
    pub drag: f32,
    pub blend: BlendMode,
}

impl Default for ParticleSettings {
    fn default() -> Self {
        Self {
            gravity: [0., -9.81, 0.],
            drag: 0.5,
            blend: BlendMode::Additive,
        }
    }
}

/// Camera basis for billboards.
#[derive(Clone, Copy, Debug)]
pub struct ParticleCamera {
    pub view_projection: Mat4,
    /// World-space camera right and up (quads face the camera).
    pub right: [f32; 3],
    pub up: [f32; 3],
}

impl ParticleCamera {
    /// From a world-to-view matrix and a projection.
    pub fn from_view(view: Mat4, projection: Mat4) -> Self {
        let m = view.0;
        Self {
            view_projection: projection * view,
            right: [m[0][0], m[1][0], m[2][0]],
            up: [m[0][1], m[1][1], m[2][1]],
        }
    }
}

/// One particle slot as stored on the GPU (for tools and tests).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Particle {
    pub position: [f32; 3],
    pub age: f32,
    pub velocity: [f32; 3],
    /// Lifetime in seconds; zero for a slot never used.
    pub life: f32,
    pub color_start: [f32; 4],
    pub color_end: [f32; 4],
    pub size: [f32; 2],
}

impl Particle {
    pub fn alive(&self) -> bool {
        self.life > 0. && self.age < self.life
    }
    fn encode(&self) -> [f32; 20] {
        let p = self;
        [
            p.position[0],
            p.position[1],
            p.position[2],
            p.age,
            p.velocity[0],
            p.velocity[1],
            p.velocity[2],
            p.life,
            p.color_start[0],
            p.color_start[1],
            p.color_start[2],
            p.color_start[3],
            p.color_end[0],
            p.color_end[1],
            p.color_end[2],
            p.color_end[3],
            p.size[0],
            p.size[1],
            0.,
            0.,
        ]
    }
    fn decode(v: &[f32]) -> Self {
        Self {
            position: [v[0], v[1], v[2]],
            age: v[3],
            velocity: [v[4], v[5], v[6]],
            life: v[7],
            color_start: [v[8], v[9], v[10], v[11]],
            color_end: [v[12], v[13], v[14], v[15]],
            size: [v[16], v[17]],
        }
    }
    /// One integration step with the GPU's maths (semi-implicit Euler with
    /// linear drag). Dead particles do not move.
    pub fn step(&mut self, settings: &ParticleSettings, dt: f32) {
        if !self.alive() {
            return;
        }
        let damping = (1. - settings.drag * dt).max(0.);
        for a in 0..3 {
            self.velocity[a] = (self.velocity[a] + settings.gravity[a] * dt) * damping;
            self.position[a] += self.velocity[a] * dt;
        }
        self.age += dt;
    }
}

/// Small deterministic generator for burst randomisation.
#[derive(Clone, Debug)]
struct Random(u64);
impl Random {
    fn next(&mut self) -> f32 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, [lo, hi]: [f32; 2]) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.map(|x| x / length)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// A ring of GPU particle slots with its compute and draw pipelines.
pub struct ParticleSystem {
    capacity: u32,
    settings: ParticleSettings,
    particles: Buffer,
    step: Buffer,
    view: Buffer,
    random: Random,
    cursor: u32,
    /// Slots written since the last prepare, with their encoded particle.
    pending: Vec<(u32, [f32; 20])>,
    /// CPU mirror of each slot's death time on this system's clock.
    deaths: Vec<f64>,
    clock: f64,
    /// Zero every slot before the next upload ([`ParticleSystem::clear`]).
    clear_pending: bool,
    simulate: ComputePipeline,
    simulate_group: BindGroup,
    draw_layout: BindGroupLayout,
    draw_pipeline_layout: PipelineLayout,
    draw_module: ShaderModule,
    pipelines: HashMap<(TextureFormat, bool), RenderPipeline>,
    sprite: Option<TextureView>,
    white: TextureView,
    sampler: Sampler,
    draw_group: Option<BindGroup>,
    no_depth: TextureView,
    /// Fade distance against scene depth (0 = hard intersections).
    soft_distance: f32,
    /// The depth view `draw_group` was built with.
    group_depth: Option<TextureView>,
}

/// View uniform: view-projection, its inverse, right, up, soft settings.
const VIEW_BYTES: u64 = 176;

impl ParticleSystem {
    /// A system with `capacity` slots (1..=1,048,576) and a seed for burst
    /// randomisation.
    pub fn new(
        gpu: &Gpu,
        capacity: u32,
        settings: ParticleSettings,
        seed: u64,
    ) -> GraphicsResult<Self> {
        let size = u64::from(capacity) * PARTICLE_BYTES;
        if !(1..=1 << 20).contains(&capacity)
            || size > gpu.device.limits().max_storage_buffer_binding_size
        {
            return Err(error(
                "particle capacity must be 1..=1048576 and fit storage",
            ));
        }
        Self::check_settings(&settings)?;
        let source = include_str!("particles.wgsl");
        let (common, rest) = source
            .split_once("// --- simulate ---")
            .expect("particle shader sections");
        let (simulate_src, draw_src) = rest.split_once("// --- draw ---").expect("draw section");
        let particles = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("particles"),
            size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let uniform = |label, size| {
            gpu.device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let step = uniform("particle step", 32);
        let view = uniform("particle view", VIEW_BYTES);
        let simulate_layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("particle simulate"),
                entries: &[
                    BindGroupLayoutEntry {
                        binding: 0,
                        visibility: ShaderStages::COMPUTE,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    BindGroupLayoutEntry {
                        binding: 1,
                        visibility: ShaderStages::COMPUTE,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let simulate = checked(gpu, || {
            let module = gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("particle simulate"),
                source: ShaderSource::Wgsl(format!("{common}{simulate_src}").into()),
            });
            let layout = gpu
                .device
                .create_pipeline_layout(&PipelineLayoutDescriptor {
                    label: Some("particle simulate"),
                    bind_group_layouts: &[Some(&simulate_layout)],
                    immediate_size: 0,
                });
            gpu.device
                .create_compute_pipeline(&ComputePipelineDescriptor {
                    label: Some("particle simulate"),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some("simulate"),
                    compilation_options: Default::default(),
                    cache: None,
                })
        })?;
        let simulate_group = gpu.device.create_bind_group(&BindGroupDescriptor {
            label: Some("particle simulate"),
            layout: &simulate_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: particles.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: step.as_entire_binding(),
                },
            ],
        });
        let draw_layout = gpu
            .device
            .create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("particle draw"),
                entries: &[
                    storage_entry(0, ShaderStages::VERTEX),
                    BindGroupLayoutEntry {
                        binding: 2,
                        visibility: ShaderStages::VERTEX_FRAGMENT,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    image_entry(3, TextureViewDimension::D2, false),
                    sampler_entry(4, false),
                    image_entry(5, TextureViewDimension::D2, true),
                ],
            });
        let draw_pipeline_layout = gpu
            .device
            .create_pipeline_layout(&PipelineLayoutDescriptor {
                label: Some("particle draw"),
                bind_group_layouts: &[Some(&draw_layout)],
                immediate_size: 0,
            });
        let draw_module = checked(gpu, || {
            gpu.device.create_shader_module(ShaderModuleDescriptor {
                label: Some("particle draw"),
                source: ShaderSource::Wgsl(format!("{common}{draw_src}").into()),
            })
        })?;
        let white = texture_2d(
            &gpu.device,
            "particle white",
            1,
            1,
            TextureFormat::Rgba8Unorm,
            TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
        )?;
        gpu.queue.write_texture(
            white.as_image_copy(),
            &[255; 4],
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            white.size(),
        );
        let sampler = gpu.device.create_sampler(&SamplerDescriptor {
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        // Bound when there is no scene depth to fade against.
        let no_depth = texture_2d(
            &gpu.device,
            "particle no depth",
            1,
            1,
            TextureFormat::Depth32Float,
            TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        )?;
        Ok(Self {
            capacity,
            settings,
            particles,
            step,
            view,
            random: Random(seed),
            cursor: 0,
            pending: Vec::new(),
            deaths: vec![f64::NEG_INFINITY; capacity as usize],
            clear_pending: false,
            clock: 0.,
            simulate,
            simulate_group,
            draw_layout,
            draw_pipeline_layout,
            draw_module,
            pipelines: HashMap::new(),
            sprite: None,
            white: white.create_view(&Default::default()),
            sampler,
            draw_group: None,
            no_depth: no_depth.create_view(&Default::default()),
            soft_distance: 0.,
            group_depth: None,
        })
    }

    fn check_settings(settings: &ParticleSettings) -> GraphicsResult<()> {
        if !settings.gravity.iter().all(|g| g.is_finite())
            || !settings.drag.is_finite()
            || settings.drag < 0.
        {
            return Err(error(
                "particle gravity must be finite and drag non-negative",
            ));
        }
        Ok(())
    }

    pub fn settings(&self) -> ParticleSettings {
        self.settings
    }
    pub fn set_settings(&mut self, settings: ParticleSettings) -> GraphicsResult<()> {
        Self::check_settings(&settings)?;
        self.settings = settings;
        Ok(())
    }
    pub fn capacity(&self) -> u32 {
        self.capacity
    }
    /// Uses the alpha channel of `sprite` as the particle shape instead of
    /// the default soft disc; `None` restores the disc.
    pub fn set_sprite(&mut self, sprite: Option<&TextureView>) {
        self.sprite = sprite.cloned();
        self.draw_group = None;
    }
    /// Removes every particle, live and queued (for example on a scene
    /// change or when a save loads). Takes effect at the next
    /// [`Self::prepare`] or [`Self::step`]; bursts emitted after the call
    /// still appear.
    pub fn clear(&mut self) {
        self.pending.clear();
        self.deaths.fill(f64::NEG_INFINITY);
        self.cursor = 0;
        self.clear_pending = true;
    }
    /// Soft particles: fade each particle out over `distance` metres where
    /// it meets scene geometry (read from the target's depth), so dust and
    /// mist sit in the air without hard intersection lines. 0 (the default)
    /// keeps hard edges. Needs a target with a sampleable depth, such as a
    /// [`SceneTarget`]'s.
    pub fn set_soft_distance(&mut self, distance: f32) -> GraphicsResult<()> {
        if !distance.is_finite() || distance < 0. {
            return Err(error("soft particle distance must be finite and >= 0"));
        }
        self.soft_distance = distance;
        Ok(())
    }
    pub fn soft_distance(&self) -> f32 {
        self.soft_distance
    }
    /// Particles still alive on this system's clock.
    pub fn live_count(&self) -> usize {
        self.deaths.iter().filter(|d| **d > self.clock).count()
    }

    /// Queues the particles of one burst. They appear at the next
    /// [`Self::prepare`]; a full ring replaces its oldest slots.
    pub fn emit(&mut self, burst: &ParticleBurst) -> GraphicsResult<()> {
        if !burst.valid() {
            return Err(error("invalid particle burst"));
        }
        let axis = normalize(burst.direction);
        let helper = if axis[1].abs() < 0.9 {
            [0., 1., 0.]
        } else {
            [1., 0., 0.]
        };
        let tangent = normalize(cross(axis, helper));
        let bitangent = cross(axis, tangent);
        let spread_cos = burst.spread.cos();
        for _ in 0..burst.count {
            let r = &mut self.random;
            // Uniform direction in the cone around `axis`.
            let cos = 1. - r.next() * (1. - spread_cos);
            let sin = (1. - cos * cos).max(0.).sqrt();
            let angle = r.next() * std::f32::consts::TAU;
            let direction: [f32; 3] = std::array::from_fn(|a| {
                axis[a] * cos + (tangent[a] * angle.cos() + bitangent[a] * angle.sin()) * sin
            });
            let speed = r.range(burst.speed);
            let jitter = [r.next() - 0.5, r.next() - 0.5, r.next() - 0.5];
            let life = r.range(burst.life);
            let particle = Particle {
                position: std::array::from_fn(|a| {
                    burst.position[a] + jitter[a] * 2. * burst.position_jitter
                }),
                age: 0.,
                velocity: direction.map(|d| d * speed),
                life,
                color_start: burst.color_start,
                color_end: burst.color_end,
                size: burst.size,
            };
            let slot = self.cursor;
            self.cursor = (self.cursor + 1) % self.capacity;
            self.deaths[slot as usize] = self.clock + f64::from(life);
            self.pending.push((slot, particle.encode()));
        }
        Ok(())
    }

    /// Uploads queued particles, advances every slot by `dt` seconds and
    /// draws the live particles into `target` (depth-tested against
    /// `target.depth` when present).
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        camera: ParticleCamera,
        dt: f32,
    ) -> GraphicsResult<CommandBuffer> {
        if !dt.is_finite() || !(0. ..=1.).contains(&dt) || !camera.view_projection.finite() {
            return Err(error(
                "particle step needs 0 <= dt <= 1 and a finite camera",
            ));
        }
        checked(gpu, || {
            let mut encoder = gpu.encoder("particles");
            self.record_uploads(gpu, &mut encoder);
            self.record_step(gpu, &mut encoder, dt);
            self.record_draw(gpu, &mut encoder, target, camera)?;
            Ok(encoder.finish())
        })?
    }

    /// Uploads queued particles and advances the simulation without drawing
    /// (for headless simulation and tests).
    pub fn step(&mut self, gpu: &Gpu, dt: f32) -> GraphicsResult<CommandBuffer> {
        if !dt.is_finite() || !(0. ..=1.).contains(&dt) {
            return Err(error("particle step needs 0 <= dt <= 1"));
        }
        let mut encoder = gpu.encoder("particle step");
        self.record_uploads(gpu, &mut encoder);
        self.record_step(gpu, &mut encoder, dt);
        Ok(encoder.finish())
    }

    fn record_uploads(&mut self, gpu: &Gpu, encoder: &mut CommandEncoder) {
        if std::mem::take(&mut self.clear_pending) {
            // An all-zero slot has lifetime 0, which the shaders treat as empty.
            encoder.clear_buffer(&self.particles, 0, None);
        }
        if self.pending.is_empty() {
            return;
        }
        // A burst larger than the ring only keeps its newest particles.
        let mut latest: BTreeMap<u32, [f32; 20]> = BTreeMap::new();
        for (slot, data) in self.pending.drain(..) {
            latest.insert(slot, data);
        }
        let mut runs: Vec<(u32, Vec<u8>)> = Vec::new();
        for (slot, data) in latest {
            match runs.last_mut() {
                Some((start, bytes))
                    if *start + (bytes.len() as u64 / PARTICLE_BYTES) as u32 == slot =>
                {
                    bytes.extend(data.iter().flat_map(|v| v.to_ne_bytes()));
                }
                _ => runs.push((slot, data.iter().flat_map(|v| v.to_ne_bytes()).collect())),
            }
        }
        let mut all = Vec::new();
        let mut copies = Vec::new();
        for (slot, bytes) in runs {
            copies.push((
                all.len() as u64,
                u64::from(slot) * PARTICLE_BYTES,
                bytes.len() as u64,
            ));
            all.extend(bytes);
        }
        let upload = quasar_frame::staging_buffer(gpu, "particle spawns", &all);
        for (source, destination, size) in copies {
            encoder.copy_buffer_to_buffer(&upload, source, &self.particles, destination, size);
        }
    }

    fn record_step(&mut self, gpu: &Gpu, encoder: &mut CommandEncoder, dt: f32) {
        let s = &self.settings;
        let data = bytes([
            s.gravity[0],
            s.gravity[1],
            s.gravity[2],
            dt,
            s.drag,
            self.capacity as f32,
            0.,
            0.,
        ]);
        let upload = quasar_frame::staging_buffer(gpu, "particle step upload", &data);
        encoder.copy_buffer_to_buffer(&upload, 0, &self.step, 0, 32);
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("particle simulate"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.simulate);
        pass.set_bind_group(0, &self.simulate_group, &[]);
        pass.dispatch_workgroups(self.capacity.div_ceil(64), 1, 1);
        drop(pass);
        self.clock += f64::from(dt);
    }

    fn record_draw(
        &mut self,
        gpu: &Gpu,
        encoder: &mut CommandEncoder,
        target: &RenderTarget,
        camera: ParticleCamera,
    ) -> GraphicsResult<()> {
        let key = (target.format, target.depth.is_some());
        if !self.pipelines.contains_key(&key) {
            let pipeline = gpu
                .device
                .create_render_pipeline(&RenderPipelineDescriptor {
                    label: Some("particles"),
                    layout: Some(&self.draw_pipeline_layout),
                    vertex: VertexState {
                        module: &self.draw_module,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: PrimitiveState::default(),
                    depth_stencil: key.1.then(|| DepthStencilState {
                        format: TextureFormat::Depth32Float,
                        depth_write_enabled: Some(false),
                        depth_compare: Some(CompareFunction::LessEqual),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(FragmentState {
                        module: &self.draw_module,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(ColorTargetState {
                            format: target.format,
                            blend: Some(self.settings.blend.state()),
                            write_mask: ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
            self.pipelines.insert(key, pipeline);
        }
        let soft = self.soft_distance > 0. && target.depth.is_some();
        let inverse = if soft {
            camera
                .view_projection
                .inverse()
                .filter(|m| m.finite())
                .ok_or_else(|| error("soft particles need an invertible camera"))?
        } else {
            Mat4::IDENTITY
        };
        let mut values: Vec<f32> = camera.view_projection.0.iter().flatten().copied().collect();
        values.extend([camera.right[0], camera.right[1], camera.right[2], 0.]);
        values.extend([
            camera.up[0],
            camera.up[1],
            camera.up[2],
            f32::from(self.sprite.is_some()),
        ]);
        values.extend(inverse.0.iter().flatten().copied());
        let (width, height) = target.size();
        values.extend([
            if soft { self.soft_distance } else { 0. },
            width as f32,
            height as f32,
            0.,
        ]);
        let data = bytes(values);
        let upload = quasar_frame::staging_buffer(gpu, "particle view upload", &data);
        encoder.copy_buffer_to_buffer(&upload, 0, &self.view, 0, VIEW_BYTES);
        let depth = if soft { target.depth.as_ref() } else { None };
        if self.group_depth.as_ref() != depth {
            self.draw_group = None;
            self.group_depth = depth.cloned();
        }
        if self.draw_group.is_none() {
            let depth = self.group_depth.as_ref().unwrap_or(&self.no_depth);
            let sprite = self.sprite.as_ref().unwrap_or(&self.white);
            self.draw_group = Some(gpu.device.create_bind_group(&BindGroupDescriptor {
                label: Some("particle draw"),
                layout: &self.draw_layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: self.particles.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: self.view.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::TextureView(sprite),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: BindingResource::TextureView(depth),
                    },
                ],
            }));
        }
        let colors = [Some(RenderPassColorAttachment {
            view: &target.color,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("particles"),
            color_attachments: &colors,
            depth_stencil_attachment: target.depth.as_ref().map(|view| {
                RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: None,
                    stencil_ops: None,
                }
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipelines[&key]);
        pass.set_bind_group(0, self.draw_group.as_ref(), &[]);
        pass.draw(0..6, 0..self.capacity);
        Ok(())
    }

    /// Reads every slot back (tests and tools; never needed to render).
    pub fn read_particles(&self, gpu: &Gpu) -> GraphicsResult<Vec<Particle>> {
        let size = u64::from(self.capacity) * PARTICLE_BYTES;
        let staging = gpu.device.create_buffer(&BufferDescriptor {
            label: Some("particle readback"),
            size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.encoder("particle readback");
        encoder.copy_buffer_to_buffer(&self.particles, 0, &staging, 0, size);
        let index = gpu.queue.submit([encoder.finish()]);
        let (send, recv) = std::sync::mpsc::sync_channel(1);
        staging.map_async(MapMode::Read, .., move |result| {
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
        let mapped = staging.get_mapped_range(..).map_err(error)?;
        let floats: Vec<f32> = mapped
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        drop(mapped);
        staging.unmap();
        Ok(floats.chunks_exact(20).map(Particle::decode).collect())
    }

    /// Drops this owner's GPU references.
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.pipelines.clear();
        self.draw_group = None;
        self.pending.clear();
        Ok(())
    }
}
