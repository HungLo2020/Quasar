//! Bounded glTF 2.0/GLB decoding. All I/O passes through LoadContext mounts.
use crate::{AssetError, AssetErrorKind, AssetLoader, LoadContext, model::*};
use base64::Engine;
use std::io::Cursor;
#[derive(Clone, Debug, Default)]
pub struct GltfLoader {
    pub limits: ModelLimits,
    pub scene: Option<usize>,
    /// Explicitly import authored default transforms while leaving animation clips unevaluated.
    pub allow_animation_default_pose: bool,
}
fn error(message: impl Into<String>) -> AssetError {
    AssetError::new(AssetErrorKind::Loader, message)
}
fn budget(message: &str) -> AssetError {
    AssetError::new(AssetErrorKind::Budget, message)
}
impl AssetLoader for GltfLoader {
    type Asset = Model3D;
    fn max_source_bytes(&self) -> usize {
        self.limits.max_source_bytes
    }
    fn load(&self, context: &LoadContext<'_>) -> Result<Model3D, AssetError> {
        self.decode(context)
            .map_err(|e| AssetError::at(e.kind(), context.path(), e.message()))
    }
}
impl GltfLoader {
    fn decode(&self, c: &LoadContext<'_>) -> Result<Model3D, AssetError> {
        let l = self.limits;
        if c.bytes().len() > l.max_source_bytes {
            return Err(budget("model source exceeds byte budget"));
        }
        if l.max_source_bytes == 0
            || l.max_nodes == 0
            || l.max_depth == 0
            || l.max_image_dimension == 0
        {
            return Err(budget("model limits must be positive"));
        }
        let file =
            gltf::Gltf::from_slice(c.bytes()).map_err(|e| error(format!("invalid glTF: {e}")))?;
        for extension in file.extensions_required() {
            if !["KHR_materials_unlit", "KHR_texture_transform"].contains(&extension) {
                return Err(error(format!("unsupported required extension {extension}")));
            }
        }
        if file.nodes().len() > l.max_nodes
            || file.materials().len().saturating_add(1) > l.max_materials
            || file.images().len() > l.max_images
            || file.textures().len() > l.max_images.saturating_mul(4)
        {
            return Err(budget("model node/material/image budget exceeded"));
        }
        if file.skins().len() != 0
            || (file.animations().len() != 0 && !self.allow_animation_default_pose)
        {
            return Err(error(
                "this static-model loader does not support skins or animation clips",
            ));
        }
        let mut parents = vec![0usize; file.nodes().len()];
        for node in file.nodes() {
            for child in node.children() {
                parents[child.index()] += 1;
                if parents[child.index()] > 1 {
                    return Err(error("node has multiple parents"));
                }
            }
        }
        let mut finished = vec![false; file.nodes().len()];
        for node in file.nodes() {
            let mut check = vec![(node, Vec::<usize>::new())];
            while let Some((node, mut ancestors)) = check.pop() {
                if ancestors.contains(&node.index()) || ancestors.len() >= l.max_depth {
                    return Err(error("node graph is cyclic or exceeds depth budget"));
                }
                if finished[node.index()] {
                    continue;
                }
                finished[node.index()] = true;
                ancestors.push(node.index());
                for child in node.children() {
                    check.push((child, ancestors.clone()));
                }
            }
        }
        let mut buffers = Vec::new();
        let mut total = 0usize;
        for buffer in file.buffers() {
            let remaining = l.max_total_buffer_bytes.saturating_sub(total);
            if buffer.length() > remaining {
                return Err(budget("model buffer byte budget exceeded"));
            }
            let bytes = match buffer.source() {
                gltf::buffer::Source::Bin => file
                    .blob
                    .as_ref()
                    .ok_or_else(|| error("missing GLB binary chunk"))?
                    .clone(),
                gltf::buffer::Source::Uri(uri) => read_uri(c, uri, remaining)?,
            };
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| budget("buffer byte overflow"))?;
            if total > l.max_total_buffer_bytes || bytes.len() < buffer.length() {
                return Err(error("buffer is truncated or exceeds budget"));
            }
            buffers.push(bytes);
        }
        // Validate ranges before the gltf utility readers see any bytes. Sparse
        // accessors are explicitly rejected rather than relying on unchecked reads.
        for view in file.views() {
            let end = view
                .offset()
                .checked_add(view.length())
                .ok_or_else(|| error("buffer view overflow"))?;
            if end > file.buffers().nth(view.buffer().index()).unwrap().length()
                || end > buffers[view.buffer().index()].len()
            {
                return Err(error("buffer view is outside its buffer"));
            }
        }
        for accessor in file.accessors() {
            if accessor.sparse().is_some() {
                return Err(error(
                    "sparse accessors are not supported by the static loader",
                ));
            }
            let view = accessor
                .view()
                .ok_or_else(|| error("accessor has no buffer view"))?;
            let stride = view.stride().unwrap_or(accessor.size());
            if stride < accessor.size() {
                return Err(error("accessor stride is smaller than an element"));
            }
            let end = accessor
                .count()
                .saturating_sub(1)
                .checked_mul(stride)
                .and_then(|n| n.checked_add(accessor.offset()))
                .and_then(|n| {
                    n.checked_add(if accessor.count() == 0 {
                        0
                    } else {
                        accessor.size()
                    })
                })
                .ok_or_else(|| error("accessor range overflow"))?;
            if end > view.length() {
                return Err(error("accessor is outside its buffer view"));
            }
        }
        let mut model = Model3D::default();
        if file.animations().len() != 0 {
            model.warnings.push(format!(
                "{} animation clips not evaluated: default-pose import explicitly enabled",
                file.animations().len()
            ));
        }
        let mut image_bytes = 0usize;
        for image in file.images() {
            let bytes = match image.source() {
                gltf::image::Source::Uri { uri, .. } => read_uri(c, uri, l.max_source_bytes)?,
                gltf::image::Source::View { view, .. } => buffers[view.buffer().index()]
                    [view.offset()..view.offset() + view.length()]
                    .to_vec(),
            };
            let mut reader = image::ImageReader::new(Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|e| error(e.to_string()))?;
            if !matches!(
                reader.format(),
                Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
            ) {
                return Err(error("model images must be PNG or JPEG"));
            }
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(l.max_image_dimension);
            limits.max_image_height = Some(l.max_image_dimension);
            limits.max_alloc = Some(l.max_decoded_image_bytes.saturating_sub(image_bytes) as u64);
            reader.limits(limits);
            let decoder = reader.into_decoder().map_err(|e| error(e.to_string()))?;
            use image::ImageDecoder;
            let (width, height) = decoder.dimensions();
            let required = (width as usize)
                .checked_mul(height as usize)
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| budget("image byte overflow"))?;
            image_bytes = image_bytes
                .checked_add(required)
                .ok_or_else(|| budget("image byte overflow"))?;
            if image_bytes > l.max_decoded_image_bytes {
                return Err(budget("decoded image byte budget exceeded"));
            }
            let decoded = image::DynamicImage::from_decoder(decoder)
                .map_err(|e| error(e.to_string()))?
                .into_rgba8();
            model.images.push(ModelImage {
                width,
                height,
                rgba: decoded.into_raw(),
            });
        }
        for texture in file.textures() {
            use gltf::texture::{MagFilter, MinFilter, WrappingMode};
            let sampler = texture.sampler();
            let (min, mip) = match sampler
                .min_filter()
                .unwrap_or(MinFilter::LinearMipmapLinear)
            {
                MinFilter::Nearest => (TextureFilter::Nearest, None),
                MinFilter::Linear => (TextureFilter::Linear, None),
                MinFilter::NearestMipmapNearest => {
                    (TextureFilter::Nearest, Some(TextureFilter::Nearest))
                }
                MinFilter::LinearMipmapNearest => {
                    (TextureFilter::Linear, Some(TextureFilter::Nearest))
                }
                MinFilter::NearestMipmapLinear => {
                    (TextureFilter::Nearest, Some(TextureFilter::Linear))
                }
                MinFilter::LinearMipmapLinear => {
                    (TextureFilter::Linear, Some(TextureFilter::Linear))
                }
            };
            let wrap = |v| match v {
                WrappingMode::ClampToEdge => TextureWrap::Clamp,
                WrappingMode::Repeat => TextureWrap::Repeat,
                WrappingMode::MirroredRepeat => TextureWrap::Mirror,
            };
            model.textures.push(ModelTexture {
                image: texture.source().index(),
                sampler: ModelSampler {
                    min,
                    mag: if sampler.mag_filter() == Some(MagFilter::Nearest) {
                        TextureFilter::Nearest
                    } else {
                        TextureFilter::Linear
                    },
                    mip,
                    wrap_u: wrap(sampler.wrap_s()),
                    wrap_v: wrap(sampler.wrap_t()),
                },
            });
        }
        for material in file.materials() {
            let pbr = material.pbr_metallic_roughness();
            let base = pbr.base_color_texture();
            let normal = material.normal_texture();
            if base.as_ref().is_some_and(|t| t.tex_coord() != 0)
                || normal.as_ref().is_some_and(|t| t.tex_coord() != 0)
            {
                return Err(error("only TEXCOORD_0 is supported"));
            }
            let base_uv = if let Some(t) = base.as_ref().and_then(|t| t.texture_transform()) {
                if t.tex_coord().is_some_and(|set| set != 0) {
                    return Err(error("texture transform requires TEXCOORD_0"));
                }
                UvTransform {
                    offset: t.offset(),
                    scale: t.scale(),
                    rotation: t.rotation(),
                }
            } else {
                UvTransform::default()
            };
            if normal
                .as_ref()
                .is_some_and(|t| t.extension_value("KHR_texture_transform").is_some())
            {
                return Err(error("normal texture transforms are not supported"));
            }
            let m = ModelMaterial {
                name: material.name().unwrap_or("material").into(),
                base_color: pbr.base_color_factor(),
                base_texture: base.map(|t| t.texture().index()),
                normal_texture: normal.as_ref().map(|t| t.texture().index()),
                normal_scale: normal.map_or(1., |t| t.scale()),
                emissive: material.emissive_factor(),
                metallic: pbr.metallic_factor(),
                roughness: pbr.roughness_factor(),
                alpha: match material.alpha_mode() {
                    gltf::material::AlphaMode::Opaque => ModelAlpha::Opaque,
                    gltf::material::AlphaMode::Mask => ModelAlpha::Mask,
                    gltf::material::AlphaMode::Blend => ModelAlpha::Blend,
                },
                alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
                double_sided: material.double_sided(),
                unlit: material.unlit(),
                base_uv,
                normal_uv: UvTransform::default(),
                metallic_roughness_texture: pbr
                    .metallic_roughness_texture()
                    .map(|t| t.texture().index()),
                emissive_texture: material.emissive_texture().map(|t| t.texture().index()),
                occlusion_texture: material.occlusion_texture().map(|t| t.texture().index()),
            };
            m.validate()?;
            model.materials.push(m);
        }
        let default_material = model.materials.len();
        model.materials.push(ModelMaterial::default());
        let mut meshes = Vec::new();
        let mut vertices = 0usize;
        let mut indices = 0usize;
        for mesh in file.meshes() {
            let mut mesh_parts = Vec::new();
            for primitive in mesh.primitives() {
                if model.primitives.len() >= l.max_primitives {
                    return Err(budget("primitive budget exceeded"));
                }
                if primitive.mode() != gltf::mesh::Mode::Triangles
                    || primitive.morph_targets().next().is_some()
                {
                    return Err(error("only static triangle primitives are supported"));
                }
                if primitive.get(&gltf::Semantic::Colors(0)).is_some() {
                    return Err(error("vertex colors require an unsupported vertex layout"));
                }
                let count = primitive
                    .get(&gltf::Semantic::Positions)
                    .ok_or_else(|| error("primitive has no POSITION"))?
                    .count();
                if count == 0 || count > l.max_vertices {
                    return Err(budget("vertex count exceeds budget"));
                }
                use gltf::accessor::{DataType, Dimensions};
                let position = primitive.get(&gltf::Semantic::Positions).unwrap();
                if position.data_type() != DataType::F32
                    || position.dimensions() != Dimensions::Vec3
                    || position.normalized()
                {
                    return Err(error("POSITION must be unnormalized float VEC3"));
                }
                if let Some(a) = primitive.get(&gltf::Semantic::Normals)
                    && (a.count() != count
                        || a.data_type() != DataType::F32
                        || a.dimensions() != Dimensions::Vec3
                        || a.normalized())
                {
                    return Err(error("NORMAL must match POSITION as float VEC3"));
                }
                if let Some(a) = primitive.get(&gltf::Semantic::TexCoords(0))
                    && (a.count() != count
                        || a.dimensions() != Dimensions::Vec2
                        || !matches!(a.data_type(), DataType::F32 | DataType::U8 | DataType::U16)
                        || (a.data_type() != DataType::F32 && !a.normalized()))
                {
                    return Err(error("TEXCOORD_0 has unsupported type or count"));
                }
                if let Some(a) = primitive.indices()
                    && (a.dimensions() != Dimensions::Scalar
                        || a.normalized()
                        || !matches!(a.data_type(), DataType::U8 | DataType::U16 | DataType::U32))
                {
                    return Err(error("indices must be unsigned scalar integers"));
                }
                let index_count = primitive.indices().map_or(count, |i| i.count());
                if index_count == 0
                    || !index_count.is_multiple_of(3)
                    || index_count > l.max_indices.saturating_sub(indices)
                {
                    return Err(budget("triangle/index budget exceeded"));
                }
                let reader = primitive.reader(|b| Some(buffers[b.index()].as_slice()));
                let positions: Vec<_> = reader
                    .read_positions()
                    .ok_or_else(|| error("unsupported POSITION component type"))?
                    .collect();
                let inds: Vec<u32> = reader
                    .read_indices()
                    .map_or_else(|| (0..count as u32).collect(), |i| i.into_u32().collect());
                if positions.len() != count
                    || inds.iter().any(|i| *i as usize >= count)
                    || positions.iter().flatten().any(|v| !v.is_finite())
                {
                    return Err(error("invalid positions or indices"));
                }
                let normals = reader.read_normals().map(|n| n.collect::<Vec<_>>());
                let uv = reader
                    .read_tex_coords(0)
                    .map(|v| v.into_f32().collect::<Vec<_>>());
                if normals
                    .as_ref()
                    .is_some_and(|n| n.len() != count || n.iter().flatten().any(|v| !v.is_finite()))
                    || uv.as_ref().is_some_and(|n| {
                        n.len() != count || n.iter().flatten().any(|v| !v.is_finite())
                    })
                {
                    return Err(error("invalid normals/UVs"));
                }
                let material = primitive.material().index().unwrap_or(default_material);
                if uv.is_none()
                    && (model.materials[material].base_texture.is_some()
                        || model.materials[material].normal_texture.is_some())
                {
                    return Err(error("textured primitive has no TEXCOORD_0"));
                }
                let final_count = if normals.is_some() {
                    count
                } else {
                    index_count
                };
                if final_count > l.max_vertices.saturating_sub(vertices) {
                    return Err(budget("decoded vertex budget exceeded"));
                }
                let (verts, inds) = if let Some(normals) = normals {
                    (
                        positions
                            .iter()
                            .enumerate()
                            .map(|(i, p)| ModelVertex {
                                position: *p,
                                normal: normals[i],
                                uv: uv.as_ref().map_or([0.; 2], |u| u[i]),
                            })
                            .collect(),
                        inds,
                    )
                } else {
                    let mut verts = Vec::with_capacity(final_count);
                    for face in inds.chunks_exact(3) {
                        let a = positions[face[0] as usize];
                        let b = positions[face[1] as usize];
                        let c = positions[face[2] as usize];
                        let ab =
                            std::array::from_fn::<_, 3, _>(|i| f64::from(b[i]) - f64::from(a[i]));
                        let ac =
                            std::array::from_fn::<_, 3, _>(|i| f64::from(c[i]) - f64::from(a[i]));
                        let n = [
                            ab[1] * ac[2] - ab[2] * ac[1],
                            ab[2] * ac[0] - ab[0] * ac[2],
                            ab[0] * ac[1] - ab[1] * ac[0],
                        ];
                        let length = n.iter().map(|v| v * v).sum::<f64>().sqrt();
                        if length == 0. || !length.is_finite() {
                            return Err(error(
                                "cannot generate a normal for a degenerate triangle",
                            ));
                        }
                        for i in face {
                            verts.push(ModelVertex {
                                position: positions[*i as usize],
                                normal: n.map(|v| (v / length) as f32),
                                uv: uv.as_ref().map_or([0.; 2], |u| u[*i as usize]),
                            });
                        }
                    }
                    (verts, (0..final_count as u32).collect())
                };
                vertices += final_count;
                indices += index_count;
                mesh_parts.push(model.primitives.len());
                model.primitives.push(ModelPrimitive {
                    name: mesh.name().unwrap_or("mesh").into(),
                    vertices: verts,
                    indices: inds,
                    material,
                });
            }
            meshes.push(mesh_parts);
        }
        let scene = if let Some(index) = self.scene {
            Some(
                file.scenes()
                    .nth(index)
                    .ok_or_else(|| error("selected scene is missing"))?,
            )
        } else {
            file.default_scene().or_else(|| file.scenes().next())
        };
        if let Some(scene) = scene {
            let mut stack: Vec<_> = scene
                .nodes()
                .map(|n| (n, identity(), Vec::<usize>::new()))
                .collect();
            // Reverse roots/children so extracted draws retain the authored order.
            stack.reverse();
            while let Some((node, parent, mut ancestors)) = stack.pop() {
                if ancestors.len() >= l.max_depth || ancestors.contains(&node.index()) {
                    return Err(error("node graph is cyclic or exceeds depth budget"));
                }
                let transform = multiply(parent, node.transform().matrix());
                if transform.iter().flatten().any(|v| !v.is_finite())
                    || transform[0][3] != 0.
                    || transform[1][3] != 0.
                    || transform[2][3] != 0.
                    || transform[3][3] != 1.
                {
                    return Err(error("invalid node transform"));
                }
                if let Some(mesh) = node.mesh() {
                    for primitive in &meshes[mesh.index()] {
                        if model.instances.len() >= l.max_instances {
                            return Err(budget("model instance budget exceeded"));
                        }
                        model.instances.push(ModelInstance {
                            node: node.index(),
                            name: node.name().unwrap_or("node").into(),
                            primitive: *primitive,
                            transform,
                        });
                    }
                }
                ancestors.push(node.index());
                let children: Vec<_> = node.children().collect();
                for child in children.into_iter().rev() {
                    stack.push((child, transform, ancestors.clone()));
                }
            }
        }
        if model.primitives.is_empty() {
            return Err(error("model has no triangle geometry"));
        }
        model.dependencies = c.read_dependencies();
        model.warnings.push(
            "PBR data is retained; classic rendering requires an explicit approximation policy"
                .into(),
        );
        Ok(model)
    }
}
fn read_uri(c: &LoadContext<'_>, uri: &str, max: usize) -> Result<Vec<u8>, AssetError> {
    if let Some(data) = uri.strip_prefix("data:") {
        let (kind, encoded) = data
            .split_once(',')
            .ok_or_else(|| error("invalid data URI"))?;
        if !kind.ends_with(";base64") {
            return Err(error("only base64 data URIs are supported"));
        }
        if encoded.len()
            > max
                .saturating_add(2)
                .saturating_div(3)
                .saturating_mul(4)
                .saturating_add(4)
        {
            return Err(budget("data URI exceeds byte budget"));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| error(e.to_string()))?;
        if bytes.len() > max {
            return Err(budget("data URI exceeds byte budget"));
        }
        Ok(bytes)
    } else {
        let path = percent_encoding::percent_decode_str(uri)
            .decode_utf8()
            .map_err(|e| error(e.to_string()))?;
        c.read_relative(&path, max).map(|(_, b)| b)
    }
}
fn identity() -> [[f32; 4]; 4] {
    [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]
}
fn multiply(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|c| std::array::from_fn(|r| (0..4).map(|i| a[i][r] * b[c][i]).sum()))
}
