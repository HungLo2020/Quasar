//! Renderer-neutral model data and bounded texture preparation. No scene identity or GAL.
use crate::{Asset, AssetError, AssetErrorKind, AssetPath};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
#[derive(Clone, Debug)]
pub struct ModelPrimitive {
    pub name: String,
    pub vertices: Vec<ModelVertex>,
    pub indices: Vec<u32>,
    pub material: usize,
}
#[derive(Clone, Debug)]
pub struct ModelInstance {
    pub node: usize,
    pub name: String,
    pub primitive: usize,
    pub transform: [[f32; 4]; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFilter {
    Nearest,
    Linear,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureWrap {
    Clamp,
    Repeat,
    Mirror,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSampler {
    pub min: TextureFilter,
    pub mag: TextureFilter,
    pub mip: Option<TextureFilter>,
    pub wrap_u: TextureWrap,
    pub wrap_v: TextureWrap,
}
impl Default for ModelSampler {
    fn default() -> Self {
        Self {
            min: TextureFilter::Linear,
            mag: TextureFilter::Linear,
            mip: Some(TextureFilter::Linear),
            wrap_u: TextureWrap::Repeat,
            wrap_v: TextureWrap::Repeat,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ModelImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct ModelTexture {
    pub image: usize,
    pub sampler: ModelSampler,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelAlpha {
    Opaque,
    Mask,
    Blend,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTransform {
    pub offset: [f32; 2],
    pub scale: [f32; 2],
    pub rotation: f32,
}
impl Default for UvTransform {
    fn default() -> Self {
        Self {
            offset: [0.; 2],
            scale: [1.; 2],
            rotation: 0.,
        }
    }
}
impl UvTransform {
    pub fn apply(self, uv: [f32; 2]) -> [f32; 2] {
        let (s, c) = self.rotation.sin_cos();
        [
            self.offset[0] + c * uv[0] * self.scale[0] - s * uv[1] * self.scale[1],
            self.offset[1] + s * uv[0] * self.scale[0] + c * uv[1] * self.scale[1],
        ]
    }
}
#[derive(Clone, Debug)]
pub struct ModelMaterial {
    pub base_uv: UvTransform,
    pub normal_uv: UvTransform,
    pub metallic_roughness_texture: Option<usize>,
    pub emissive_texture: Option<usize>,
    pub occlusion_texture: Option<usize>,
    pub name: String,
    pub base_color: [f32; 4],
    pub base_texture: Option<usize>,
    pub normal_texture: Option<usize>,
    pub normal_scale: f32,
    pub emissive: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub alpha: ModelAlpha,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    pub unlit: bool,
}
impl ModelMaterial {
    pub fn validate(&self) -> Result<(), AssetError> {
        let unit = |v: f32| v.is_finite() && (0. ..=1.).contains(&v);
        if !self.base_color.iter().copied().all(unit)
            || !self.emissive.iter().all(|v| v.is_finite() && *v >= 0.)
            || !unit(self.metallic)
            || !unit(self.roughness)
            || !unit(self.alpha_cutoff)
            || !self.normal_scale.is_finite()
            || self.normal_scale < 0.
            || !self
                .base_uv
                .offset
                .iter()
                .chain(&self.base_uv.scale)
                .chain(&self.normal_uv.offset)
                .chain(&self.normal_uv.scale)
                .chain([&self.base_uv.rotation, &self.normal_uv.rotation])
                .all(|v| v.is_finite())
        {
            return Err(AssetError::new(
                AssetErrorKind::Loader,
                "invalid model material factors or UV transform",
            ));
        }
        Ok(())
    }
}
impl Default for ModelMaterial {
    fn default() -> Self {
        Self {
            base_uv: UvTransform::default(),
            normal_uv: UvTransform::default(),
            metallic_roughness_texture: None,
            emissive_texture: None,
            occlusion_texture: None,
            name: "default".into(),
            base_color: [1.; 4],
            base_texture: None,
            normal_texture: None,
            normal_scale: 1.,
            emissive: [0.; 3],
            metallic: 1.,
            roughness: 1.,
            alpha: ModelAlpha::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            unlit: false,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Model3D {
    pub primitives: Vec<ModelPrimitive>,
    pub instances: Vec<ModelInstance>,
    pub materials: Vec<ModelMaterial>,
    pub images: Vec<ModelImage>,
    pub textures: Vec<ModelTexture>,
    pub warnings: Vec<String>,
    pub dependencies: Vec<AssetPath>,
}
impl Asset for Model3D {
    fn dependencies(&self) -> Vec<AssetPath> {
        self.dependencies.clone()
    }
    fn memory_size(&self) -> usize {
        self.images.iter().map(|i| i.rgba.len()).sum::<usize>()
            + self
                .primitives
                .iter()
                .map(|p| {
                    p.vertices.len() * std::mem::size_of::<ModelVertex>()
                        + p.indices.len() * 4
                        + p.name.len()
                })
                .sum::<usize>()
            + self.instances.len() * std::mem::size_of::<ModelInstance>()
            + self.materials.len() * std::mem::size_of::<ModelMaterial>()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ModelLimits {
    pub max_source_bytes: usize,
    pub max_total_buffer_bytes: usize,
    pub max_vertices: usize,
    pub max_indices: usize,
    pub max_primitives: usize,
    pub max_instances: usize,
    pub max_materials: usize,
    pub max_images: usize,
    pub max_image_dimension: u32,
    pub max_decoded_image_bytes: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
}
impl Default for ModelLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 32 * 1024 * 1024,
            max_total_buffer_bytes: 64 * 1024 * 1024,
            max_vertices: 1_000_000,
            max_indices: 3_000_000,
            max_primitives: 1024,
            max_instances: 4096,
            max_materials: 256,
            max_images: 64,
            max_image_dimension: 4096,
            max_decoded_image_bytes: 128 * 1024 * 1024,
            max_nodes: 4096,
            max_depth: 128,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MipColor {
    Srgb,
    Linear,
    Normal,
}
impl ModelImage {
    /// CPU-generated mips with explicit color/normal semantics; every backend uploads the same bytes.
    pub fn mip_chain(
        &self,
        color: MipColor,
        generate: bool,
        max_bytes: usize,
    ) -> Result<Vec<Self>, AssetError> {
        let len = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| AssetError::new(AssetErrorKind::Budget, "image size overflow"))?;
        if self.width == 0 || self.height == 0 || len != self.rgba.len() {
            return Err(AssetError::new(
                AssetErrorKind::Loader,
                "invalid RGBA image",
            ));
        }
        if len > max_bytes {
            return Err(AssetError::new(
                AssetErrorKind::Budget,
                "mipmap budget exceeded",
            ));
        }
        let mut levels = vec![self.clone()];
        let mut used = len;
        while generate && (levels.last().unwrap().width > 1 || levels.last().unwrap().height > 1) {
            let prev = levels.last().unwrap();
            let width = (prev.width / 2).max(1);
            let height = (prev.height / 2).max(1);
            let bytes = width as usize * height as usize * 4;
            used = used
                .checked_add(bytes)
                .ok_or_else(|| AssetError::new(AssetErrorKind::Budget, "mipmap budget overflow"))?;
            if used > max_bytes {
                return Err(AssetError::new(
                    AssetErrorKind::Budget,
                    "mipmap budget exceeded",
                ));
            }
            let mut rgba = Vec::with_capacity(bytes);
            for y in 0..height {
                for x in 0..width {
                    // Include every source texel for odd dimensions rather than dropping the final row/column.
                    let mut sum = [0f32; 4];
                    let mut count = 0.;
                    for sy in y * prev.height / height..(y + 1) * prev.height / height {
                        for sx in x * prev.width / width..(x + 1) * prev.width / width {
                            for (c, out) in sum.iter_mut().enumerate() {
                                let value = prev.rgba[((sy * prev.width + sx) * 4) as usize + c]
                                    as f32
                                    / 255.;
                                *out += if c < 3 {
                                    match color {
                                        MipColor::Srgb => decode(value),
                                        MipColor::Normal => value * 2. - 1.,
                                        MipColor::Linear => value,
                                    }
                                } else {
                                    value
                                };
                            }
                            count += 1.;
                        }
                    }
                    for v in &mut sum {
                        *v /= count;
                    }
                    if color == MipColor::Normal {
                        let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
                        if length > 1e-6 {
                            for v in &mut sum[..3] {
                                *v /= length;
                            }
                        } else {
                            sum[..3].copy_from_slice(&[0., 0., 1.]);
                        }
                    }
                    for (c, v) in sum.into_iter().enumerate() {
                        let v = if c < 3 {
                            match color {
                                MipColor::Srgb => encode(v),
                                MipColor::Normal => v * 0.5 + 0.5,
                                MipColor::Linear => v,
                            }
                        } else {
                            v
                        };
                        rgba.push((v.clamp(0., 1.) * 255.).round() as u8);
                    }
                }
            }
            levels.push(Self {
                width,
                height,
                rgba,
            });
        }
        Ok(levels)
    }
}
fn decode(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}
fn encode(x: f32) -> f32 {
    if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1. / 2.4) - 0.055
    }
}
