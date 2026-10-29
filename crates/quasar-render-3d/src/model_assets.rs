//! Optional GPU adapter for renderer-neutral quasar-assets model data.
use super::*;
use quasar_assets::model::*;
use vulkanic_gal::commands::BufferImageCopyRegion;

pub struct UploadedTexture {
    texture: Handle,
    view: Handle,
    sampler: Handle,
    usage: SubmissionUsage,
    pub upload_bytes: usize,
    pub mip_levels: u32,
}
impl UploadedTexture {
    pub fn upload(
        gal: &mut VulkanicGal,
        image: &ModelImage,
        sampler: ModelSampler,
        color: MipColor,
        max_bytes: usize,
    ) -> GalResult<Self> {
        let levels = image
            .mip_chain(color, sampler.mip.is_some(), max_bytes)
            .map_err(|e| GalError::invalid_argument(e.to_string()))?;
        let bytes = levels.iter().map(|l| l.rgba.len()).sum::<usize>();
        let mut owned = Vec::new();
        let result = (|| {
            let texture = gal.create_texture(TextureDesc {
                label: "model-texture".into(),
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                extent: Extent3d {
                    width: image.width,
                    height: image.height,
                    depth: 1,
                },
                mip_levels: levels.len() as u32,
                array_layers: 1,
                usages: vec![TextureUsage::Sampled, TextureUsage::TransferDst],
            })?;
            owned.push(texture);
            let view = gal.create_texture_view(TextureViewDesc {
                label: "model-texture-view".into(),
                texture,
                format: TextureFormat::Rgba8Unorm,
                base_mip: 0,
                mip_count: levels.len() as u32,
                base_layer: 0,
                layer_count: 1,
            })?;
            owned.push(view);
            let filter = |v| match v {
                TextureFilter::Nearest => SamplerFilter::Nearest,
                TextureFilter::Linear => SamplerFilter::Linear,
            };
            let wrap = |v| match v {
                TextureWrap::Clamp => SamplerAddressMode::ClampToEdge,
                TextureWrap::Repeat => SamplerAddressMode::Repeat,
                TextureWrap::Mirror => SamplerAddressMode::MirroredRepeat,
            };
            let sampler = gal.create_sampler(SamplerDesc {
                label: "model-texture-sampler".into(),
                min_filter: filter(sampler.min),
                mag_filter: filter(sampler.mag),
                mip_filter: filter(sampler.mip.unwrap_or(TextureFilter::Nearest)),
                address_u: wrap(sampler.wrap_u),
                address_v: wrap(sampler.wrap_v),
                address_w: SamplerAddressMode::ClampToEdge,
                comparison: None,
            })?;
            owned.push(sampler);
            let upload = buffer(gal, "model-texture-upload", bytes, BufferUsage::TransferSrc)?;
            owned.push(upload);
            let mut data = Vec::with_capacity(bytes);
            let mut operations = vec![barrier(
                texture,
                TextureUsageState::Undefined,
                TextureUsageState::TransferDst,
            )];
            let mut offset = 0;
            for (mip, level) in levels.iter().enumerate() {
                data.extend_from_slice(&level.rgba);
                operations.push(CommandOp::CopyBufferToTexture(BufferImageCopyRegion {
                    buffer: upload,
                    buffer_offset: offset,
                    bytes_per_row: level.width * 4,
                    rows_per_image: level.height,
                    texture,
                    texture_mip: mip as u32,
                    texture_layer: 0,
                    texture_origin: TextureOrigin3d { x: 0, y: 0, z: 0 },
                    extent: Extent3d {
                        width: level.width,
                        height: level.height,
                        depth: 1,
                    },
                }));
                offset += level.rgba.len() as u64;
            }
            operations.splice(
                0..0,
                [
                    CommandOp::HostWriteBuffer {
                        buffer: upload,
                        offset: 0,
                        data,
                    },
                    barrier(
                        upload,
                        TextureUsageState::TransferDst,
                        TextureUsageState::TransferSrc,
                    ),
                ],
            );
            operations.push(barrier(
                texture,
                TextureUsageState::TransferDst,
                TextureUsageState::ShaderRead,
            ));
            gal.submit(SubmissionBatch {
                label: "model-texture-upload".into(),
                command_lists: vec![CommandList::from(CommandListDesc {
                    label: "model-texture-upload".into(),
                    operations,
                })],
            })?;
            gal.retire(upload)?;
            owned.pop();
            Ok(Self {
                texture,
                view,
                sampler,
                usage: SubmissionUsage::default(),
                upload_bytes: bytes,
                mip_levels: levels.len() as u32,
            })
        })();
        if result.is_err() {
            for h in owned.into_iter().rev() {
                let _ = gal.retire(h);
            }
        }
        result
    }
    pub fn view(&self) -> Handle {
        self.view
    }
    pub fn sampler(&self) -> Handle {
        self.sampler
    }
    /// Carry this receipt in direct GAL consumers that may outlive the texture owner.
    pub fn submission_usage(&self) -> SubmissionUsage {
        self.usage.clone()
    }
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.usage.has_pending_commands() {
            return Err(GalError::submission(
                StatusCode::InFlight,
                "texture consumers still retained",
            ));
        }
        if !self.view.is_null() {
            for h in [self.view, self.sampler, self.texture] {
                gal.retire(h)?;
            }
            self.view = Handle::NULL;
            self.texture = Handle::NULL;
            self.sampler = Handle::NULL;
        }
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
    pub fn upload(
        gal: &mut VulkanicGal,
        model: &Model3D,
        options: ModelUploadOptions,
    ) -> GalResult<Self> {
        if model.primitives.is_empty() || model.materials.is_empty() {
            return Err(GalError::invalid_argument(
                "model has no geometry/materials",
            ));
        }
        let mut mesh_bytes = 0usize;
        for p in &model.primitives {
            mesh_bytes = mesh_bytes
                .checked_add(
                    p.vertices
                        .len()
                        .checked_mul(48)
                        .and_then(|n| n.checked_add(p.indices.len().checked_mul(4)?))
                        .ok_or_else(|| GalError::invalid_argument("mesh byte overflow"))?,
                )
                .ok_or_else(|| GalError::invalid_argument("mesh byte overflow"))?;
            if p.material >= model.materials.len() {
                return Err(GalError::invalid_argument("invalid material index"));
            }
        }
        if mesh_bytes > options.max_mesh_bytes {
            return Err(GalError::invalid_argument("model GPU mesh budget exceeded"));
        }
        let used: std::collections::BTreeSet<_> =
            model.primitives.iter().map(|p| p.material).collect();
        for (index, m) in model.materials.iter().enumerate() {
            if !used.contains(&index) {
                continue;
            }
            m.validate()
                .map_err(|e| GalError::invalid_argument(e.to_string()))?;
            if m.normal_texture.is_some() && m.normal_scale > 8. {
                return Err(GalError::unsupported_feature(
                    "normal-map strength exceeds the surface shader's limit",
                ));
            }
            if options.material_conversion == MaterialConversion::RejectPbr && !m.unlit {
                return Err(GalError::unsupported_feature(
                    "choose ClassicApproximation explicitly to render PBR model materials",
                ));
            }
            if m.metallic_roughness_texture.is_some()
                || m.emissive_texture.is_some()
                || m.occlusion_texture.is_some()
            {
                return Err(GalError::unsupported_feature(
                    "classic materials do not support PBR/emissive/occlusion texture maps",
                ));
            }
            if m.normal_texture.is_some() && m.normal_uv != m.base_uv {
                return Err(GalError::unsupported_feature(
                    "base and normal texture UV transforms must agree",
                ));
            }
            for t in [m.base_texture, m.normal_texture].into_iter().flatten() {
                if t >= model.textures.len() || model.textures[t].image >= model.images.len() {
                    return Err(GalError::invalid_argument(
                        "invalid model texture/image index",
                    ));
                }
            }
        }
        if model
            .instances
            .iter()
            .any(|i| i.primitive >= model.primitives.len() || !Mat4(i.transform).finite())
        {
            return Err(GalError::invalid_argument("invalid model instance"));
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
            material: p.material,
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
        gal: &mut VulkanicGal,
        renderer: &mut SurfaceRenderState3D,
    ) -> GalResult<()> {
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
    pub fn has_pending_commands(&self) -> bool {
        self.primitives
            .iter()
            .any(|p| p.mesh.usage.has_pending_commands())
            || self.textures.iter().any(|t| t.usage.has_pending_commands())
    }
    pub fn bind_shadow_samplers(
        &self,
        gal: &mut VulkanicGal,
        renderer: &mut ShadowRenderState3D,
    ) -> GalResult<()> {
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
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self
            .primitives
            .iter()
            .any(|p| p.mesh.usage.has_pending_commands())
            || self.textures.iter().any(|t| t.usage.has_pending_commands())
        {
            return Err(GalError::submission(
                StatusCode::InFlight,
                "model commands still retain this generation",
            ));
        }
        for p in &mut self.primitives {
            p.mesh.release(gal)?;
        }
        for texture in &mut self.textures {
            texture.release(gal)?;
        }
        self.primitives.clear();
        self.instances.clear();
        self.normal_textures.clear();
        self.textures.clear();
        Ok(())
    }
}
