//! Voxel materials: one RGBA8 texture array plus a material table.
use quasar_frame::{Gpu, GraphicsResult, error};
use vulkanic_gal::util::DeviceExt;
use vulkanic_gal::*;

/// One material. Material IDs in meshes index the table passed to
/// [`VoxelMaterials::new`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialDef {
    /// Texture array layer.
    pub layer: u32,
    /// Linear multiplier (alpha included).
    pub tint: [f32; 4],
    /// Linear emitted colour added after lighting.
    pub emissive: [f32; 3],
    /// Optional emissive mask: a texture array layer whose colour scales
    /// `emissive` per texel, so ore veins, lava cracks and lit windows glow
    /// on their own while the rest of the block does not. Sampled with the
    /// same UVs (or triplanar mapping) as `layer`.
    pub emissive_layer: Option<u32>,
    /// Discard fragments with alpha below `cutoff` (cutout layers).
    pub alpha_test: bool,
    pub cutoff: f32,
    /// Texture repeats per voxel for triplanar (smooth) surfaces.
    pub triplanar_scale: f32,
}

impl MaterialDef {
    pub fn layer(layer: u32) -> Self {
        Self {
            layer,
            tint: [1.0; 4],
            emissive: [0.0; 3],
            emissive_layer: None,
            alpha_test: false,
            cutoff: 0.5,
            triplanar_scale: 0.25,
        }
    }
    pub fn with_tint(mut self, tint: [f32; 4]) -> Self {
        self.tint = tint;
        self
    }
    pub fn with_emissive(mut self, emissive: [f32; 3]) -> Self {
        self.emissive = emissive;
        self
    }
    /// Glows `emissive` scaled by the colour of texture layer `layer`.
    pub fn with_emissive_mask(mut self, emissive: [f32; 3], layer: u32) -> Self {
        self.emissive = emissive;
        self.emissive_layer = Some(layer);
        self
    }
    pub fn with_alpha_test(mut self, cutoff: f32) -> Self {
        self.alpha_test = true;
        self.cutoff = cutoff;
        self
    }
}

/// Texture filtering for material layers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MaterialFilter {
    /// Crisp texels (block games).
    #[default]
    Nearest,
    Linear,
}

/// Uploaded material layers and table.
pub struct VoxelMaterials {
    _texture: Texture,
    pub(crate) view: TextureView,
    pub(crate) sampler: Sampler,
    pub(crate) table: Buffer,
    count: u32,
    pub(crate) generation: u64,
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl VoxelMaterials {
    /// `layers` are `size × size` sRGB RGBA8 images.
    pub fn new(
        gpu: &Gpu,
        size: u32,
        layers: &[Vec<u8>],
        defs: &[MaterialDef],
        filter: MaterialFilter,
    ) -> GraphicsResult<Self> {
        let expected = size as usize * size as usize * 4;
        if size == 0 || layers.is_empty() || layers.iter().any(|l| l.len() != expected) {
            return Err(error(
                "material layers must be non-empty size×size RGBA8 images",
            ));
        }
        if defs.is_empty()
            || defs.iter().any(|d| {
                d.layer as usize >= layers.len()
                    || d.emissive_layer.is_some_and(|l| l as usize >= layers.len())
            })
        {
            return Err(error("materials must reference existing texture layers"));
        }
        let texture = gpu.device.create_texture(&TextureDescriptor {
            label: Some("voxel material layers"),
            size: Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (layer, pixels) in layers.iter().enumerate() {
            gpu.queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: TextureAspect::All,
                },
                pixels,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: Some(size),
                },
                Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..Default::default()
        });
        let mode = match filter {
            MaterialFilter::Nearest => FilterMode::Nearest,
            MaterialFilter::Linear => FilterMode::Linear,
        };
        let sampler = gpu.device.create_sampler(&SamplerDescriptor {
            label: Some("voxel material sampler"),
            address_mode_u: AddressMode::Repeat,
            address_mode_v: AddressMode::Repeat,
            mag_filter: mode,
            min_filter: mode,
            ..Default::default()
        });
        let mut bytes = Vec::with_capacity(defs.len() * 48);
        for d in defs {
            let values: [u32; 12] = [
                d.layer,
                u32::from(d.alpha_test),
                d.cutoff.to_bits(),
                d.triplanar_scale.to_bits(),
                d.tint[0].to_bits(),
                d.tint[1].to_bits(),
                d.tint[2].to_bits(),
                d.tint[3].to_bits(),
                d.emissive[0].to_bits(),
                d.emissive[1].to_bits(),
                d.emissive[2].to_bits(),
                // Emissive mask layer + 1 (0 = none), read as a float.
                d.emissive_layer
                    .map_or(0.0f32, |l| (l + 1) as f32)
                    .to_bits(),
            ];
            bytes.extend(values.iter().flat_map(|v| v.to_ne_bytes()));
        }
        let table = gpu.device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("voxel material table"),
            contents: &bytes,
            usage: BufferUsages::STORAGE,
        });
        Ok(Self {
            _texture: texture,
            view,
            sampler,
            table,
            count: defs.len() as u32,
            generation: GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        })
    }
    /// One flat-coloured layer and material per colour (prototypes, tests).
    pub fn solid_colors(gpu: &Gpu, colors: &[[u8; 4]]) -> GraphicsResult<Self> {
        let layers: Vec<Vec<u8>> = colors.iter().map(|c| c.repeat(4 * 4)).collect();
        let defs: Vec<MaterialDef> = (0..colors.len() as u32).map(MaterialDef::layer).collect();
        Self::new(gpu, 4, &layers, &defs, MaterialFilter::Nearest)
    }
    pub fn count(&self) -> u32 {
        self.count
    }
}
