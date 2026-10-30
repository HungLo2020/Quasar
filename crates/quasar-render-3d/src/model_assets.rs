//! Native GPU generations for renderer-neutral asset data.
use super::*;
use quasar_assets::model::*;
pub struct UploadedTexture {
    texture: Texture,
    view: TextureView,
    sampler: Sampler,
    pub upload_bytes: usize,
    pub mip_levels: u32,
}
impl UploadedTexture {
    pub fn upload(
        gpu: &Gpu,
        image: &ModelImage,
        sampler: ModelSampler,
        color: MipColor,
        max_bytes: usize,
    ) -> GraphicsResult<Self> {
        let levels = image
            .mip_chain(color, sampler.mip.is_some(), max_bytes)
            .map_err(error)?;
        if image.width > gpu.device.limits().max_texture_dimension_2d
            || image.height > gpu.device.limits().max_texture_dimension_2d
        {
            return Err(error("image exceeds device dimension limit"));
        }
        let texture = gpu.device.create_texture(&TextureDescriptor {
            label: Some("model texture"),
            size: Extent3d {
                width: image.width,
                height: image.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: levels.len() as u32,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (mip, level) in levels.iter().enumerate() {
            gpu.queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip as u32,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                &level.rgba,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(level.width * 4),
                    rows_per_image: Some(level.height),
                },
                Extent3d {
                    width: level.width,
                    height: level.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        let filter = |f| match f {
            TextureFilter::Nearest => FilterMode::Nearest,
            TextureFilter::Linear => FilterMode::Linear,
        };
        let wrap = |w| match w {
            TextureWrap::Clamp => AddressMode::ClampToEdge,
            TextureWrap::Repeat => AddressMode::Repeat,
            TextureWrap::Mirror => AddressMode::MirrorRepeat,
        };
        let native_sampler = gpu.device.create_sampler(&SamplerDescriptor {
            label: Some("model sampler"),
            min_filter: filter(sampler.min),
            mag_filter: filter(sampler.mag),
            mipmap_filter: match sampler.mip {
                Some(TextureFilter::Linear) => MipmapFilterMode::Linear,
                _ => MipmapFilterMode::Nearest,
            },
            lod_max_clamp: if sampler.mip.is_some() { 32. } else { 0. },
            address_mode_u: wrap(sampler.wrap_u),
            address_mode_v: wrap(sampler.wrap_v),
            ..Default::default()
        });
        Ok(Self {
            view: texture.create_view(&Default::default()),
            texture,
            sampler: native_sampler,
            upload_bytes: levels.iter().map(|l| l.rgba.len()).sum(),
            mip_levels: levels.len() as u32,
        })
    }
    pub fn view(&self) -> &TextureView {
        &self.view
    }
    pub fn sampler(&self) -> &Sampler {
        &self.sampler
    }
    pub fn texture(&self) -> &Texture {
        &self.texture
    }
    pub fn release(self, _gpu: &Gpu) -> GraphicsResult<()> {
        Ok(())
    }
}
/// Classic shading is an explicit approximation of glTF metallic/roughness PBR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialConversion {
    RejectPbr,
    ClassicApproximation,
}
#[derive(Clone, Copy, Debug)]
pub struct ModelUploadOptions {
    pub max_texture_bytes: usize,
    pub max_mesh_bytes: usize,
    pub material_conversion: MaterialConversion,
}
impl Default for ModelUploadOptions {
    fn default() -> Self {
        Self {
            max_texture_bytes: 128 * 1024 * 1024,
            max_mesh_bytes: 128 * 1024 * 1024,
            material_conversion: MaterialConversion::RejectPbr,
        }
    }
}
struct Primitive {
    mesh: Mesh,
    material: SurfaceMaterial,
}
/// One independent GPU generation. Upload a replacement first, then swap owners;
/// keep/release the old generation according to its command reservations.
pub struct UploadedModel {
    primitives: Vec<Primitive>,
    textures: Vec<UploadedTexture>,
    normal_textures: Vec<usize>,
    instances: Vec<ModelInstance>,
    pub texture_upload_bytes: usize,
    pub mesh_upload_bytes: usize,
}
impl UploadedModel {
    pub fn upload(gal: &Gpu, model: &Model3D, options: ModelUploadOptions) -> GraphicsResult<Self> {
        if model.primitives.is_empty() || model.materials.is_empty() {
            return Err(error("model has no geometry/materials"));
        }
        let mut mesh_bytes = 0usize;
        for p in &model.primitives {
            mesh_bytes = mesh_bytes
                .checked_add(
                    p.vertices
                        .len()
                        .checked_mul(48)
                        .and_then(|n| n.checked_add(p.indices.len().checked_mul(4)?))
                        .ok_or_else(|| error("mesh byte overflow"))?,
                )
                .ok_or_else(|| error("mesh byte overflow"))?;
            if p.material >= model.materials.len() {
                return Err(error("invalid material index"));
            }
        }
        if mesh_bytes > options.max_mesh_bytes {
            return Err(error("model GPU mesh budget exceeded"));
        }
        let used: std::collections::BTreeSet<_> =
            model.primitives.iter().map(|p| p.material).collect();
        for (index, m) in model.materials.iter().enumerate() {
            if !used.contains(&index) {
                continue;
            }
            m.validate().map_err(|e| error(e.to_string()))?;
            if m.normal_texture.is_some() && m.normal_scale > 8. {
                return Err(error(
                    "normal-map strength exceeds the surface shader's limit",
                ));
            }
            if options.material_conversion == MaterialConversion::RejectPbr && !m.unlit {
                return Err(error(
                    "choose ClassicApproximation explicitly to render PBR model materials",
                ));
            }
            if m.metallic_roughness_texture.is_some()
                || m.emissive_texture.is_some()
                || m.occlusion_texture.is_some()
            {
                return Err(error(
                    "classic materials do not support PBR/emissive/occlusion texture maps",
                ));
            }
            if m.normal_texture.is_some() && m.normal_uv != m.base_uv {
                return Err(error("base and normal texture UV transforms must agree"));
            }
            for t in [m.base_texture, m.normal_texture].into_iter().flatten() {
                if t >= model.textures.len() || model.textures[t].image >= model.images.len() {
                    return Err(error("invalid model texture/image index"));
                }
            }
        }
        if model
            .instances
            .iter()
            .any(|i| i.primitive >= model.primitives.len() || !Mat4(i.transform).finite())
        {
            return Err(error("invalid model instance"));
        }
        let mut result = Self {
            primitives: Vec::new(),
            textures: Vec::new(),
            normal_textures: Vec::new(),
            instances: model.instances.clone(),
            texture_upload_bytes: 0,
            mesh_upload_bytes: mesh_bytes,
        };
        let uploaded = (|| {
            let mut images: BTreeMap<(usize, bool), usize> = BTreeMap::new();
            for (index, m) in model.materials.iter().enumerate() {
                if !used.contains(&index) {
                    continue;
                }
                for (t, normal) in [(m.base_texture, false), (m.normal_texture, true)] {
                    if let Some(t) = t
                        && !images.contains_key(&(t, normal))
                    {
                        let texture = &model.textures[t];
                        let gpu = UploadedTexture::upload(
                            gal,
                            &model.images[texture.image],
                            texture.sampler,
                            if normal {
                                MipColor::Normal
                            } else {
                                MipColor::Srgb
                            },
                            options
                                .max_texture_bytes
                                .saturating_sub(result.texture_upload_bytes),
                        )?;
                        result.texture_upload_bytes += gpu.upload_bytes;
                        images.insert((t, normal), result.textures.len());
                        if normal {
                            result.normal_textures.push(result.textures.len());
                        }
                        result.textures.push(gpu);
                    }
                }
            }
            let white = UploadedTexture::upload(
                gal,
                &ModelImage {
                    width: 1,
                    height: 1,
                    rgba: vec![255; 4],
                },
                ModelSampler {
                    mip: None,
                    ..ModelSampler::default()
                },
                MipColor::Linear,
                options
                    .max_texture_bytes
                    .saturating_sub(result.texture_upload_bytes),
            )?;
            let white_index = result.textures.len();
            result.texture_upload_bytes += white.upload_bytes;
            result.textures.push(white);
            for p in &model.primitives {
                let m = &model.materials[p.material];
                let view = m
                    .base_texture
                    .map_or(result.textures[white_index].view(), |t| {
                        result.textures[images[&(t, false)]].view()
                    });
                let mut lit = LitMaterial::textured(view);
                lit.base.tint = m.base_color;
                if m.alpha == ModelAlpha::Opaque {
                    lit.base.tint[3] = 1.;
                }
                lit.base.transparent = m.alpha == ModelAlpha::Blend;
                lit.base.depth_write = m.alpha != ModelAlpha::Blend;
                lit.alpha_cutoff = if m.alpha == ModelAlpha::Mask {
                    m.alpha_cutoff
                } else {
                    0.
                };
                lit.emissive = m.emissive;
                lit.shading = if m.unlit {
                    ShadingModel::Unlit
                } else {
                    ShadingModel::BlinnPhong
                };
                lit.texture_color_space = ColorSpace::Srgb;
                lit.specular = std::array::from_fn(|i| {
                    0.04 * (1. - m.metallic) + m.base_color[i] * m.metallic
                });
                lit.shininess = (2. / m.roughness.max(0.05).powi(4) - 2.).clamp(1., 8192.);
                let mut material = SurfaceMaterial::from(lit);
                material.normal_map = m.normal_texture.map(|t| NormalMap {
                    strength: m.normal_scale,
                    ..NormalMap::new(result.textures[images[&(t, true)]].view())
                });
                let vertices: Vec<_> = p
                    .vertices
                    .iter()
                    .map(|v| Vertex {
                        position: v.position,
                        normal: v.normal,
                        uv: m.base_uv.apply(v.uv),
                    })
                    .collect();
                let mut mesh = Mesh::upload(gal, &vertices, &p.indices)?;
                mesh.set_opaque_alpha(m.alpha == ModelAlpha::Opaque);
                mesh.set_face_policy(if m.double_sided {
                    MeshFacePolicy::DoubleSided
                } else {
                    MeshFacePolicy::SingleSided
                });
                result.primitives.push(Primitive { mesh, material });
            }
            Ok(())
        })();
        if let Err(e) = uploaded {
            let _ = result.release(gal);
            return Err(e);
        }
        Ok(result)
    }
    pub fn instances(&self) -> &[ModelInstance] {
        &self.instances
    }
    pub fn primitive_count(&self) -> usize {
        self.primitives.len()
    }
    /// Draw geometry directly, including models imported without a default scene.
    pub fn primitive_draw(&self, index: usize, model: Mat4) -> Option<SurfaceDraw3D<'_>> {
        let p = self.primitives.get(index)?;
        Some(SurfaceDraw3D {
            mesh: &p.mesh,
            model,
            material: p.material.clone(),
        })
    }
    pub fn draw(&self, index: usize, root: Mat4) -> Option<SurfaceDraw3D<'_>> {
        let instance = self.instances.get(index)?;
        self.primitive_draw(instance.primitive, root * Mat4(instance.transform))
    }
    pub fn draws(&self, root: Mat4) -> Vec<SurfaceDraw3D<'_>> {
        (0..self.instances.len())
            .filter_map(|i| self.draw(i, root))
            .collect()
    }
    /// Install caller-owned samplers without giving the renderer ownership of this generation.
    pub fn bind_samplers(
        &self,
        gal: &Gpu,
        renderer: &mut SurfaceRenderState3D,
    ) -> GraphicsResult<()> {
        for (i, t) in self.textures.iter().enumerate() {
            renderer.set_texture_sampler(
                gal,
                t.view(),
                t.sampler(),
                self.normal_textures.contains(&i),
            )?;
        }
        Ok(())
    }
    pub fn bind_shadow_samplers(
        &self,
        gal: &Gpu,
        renderer: &mut ShadowRenderState3D,
    ) -> GraphicsResult<()> {
        for (i, t) in self.textures.iter().enumerate() {
            if !self.normal_textures.contains(&i) {
                renderer.set_texture_sampler(gal, t.view(), t.sampler())?;
            }
        }
        Ok(())
    }
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for i in &self.instances {
            let (a, b) = self.primitives[i.primitive].mesh.local_bounds();
            for bits in 0..8 {
                let p = Mat4(i.transform).transform([
                    if bits & 1 == 0 { a[0] } else { b[0] },
                    if bits & 2 == 0 { a[1] } else { b[1] },
                    if bits & 4 == 0 { a[2] } else { b[2] },
                    1.,
                ]);
                for axis in 0..3 {
                    min[axis] = min[axis].min(p[axis]);
                    max[axis] = max[axis].max(p[axis]);
                }
            }
        }
        min.iter()
            .chain(&max)
            .all(|v| v.is_finite())
            .then_some((min, max))
    }
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.primitives.clear();
        self.instances.clear();
        self.normal_textures.clear();
        self.textures.clear();
        Ok(())
    }
}
