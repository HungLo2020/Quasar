//! GPU tests for the voxel renderer (`--include-ignored` on a Vulkan device).
use quasar_frame::{Gpu, OffscreenTarget};
use quasar_math::{DQuat, DVec3, IVec3, Vec3};
use quasar_render_3d::{Light, Lighting, ShadingModel};
use quasar_render_voxel::*;
use quasar_voxel::{
    AIR, BlockCatalog, BlockDef, Brush, BrushOp, BrushShape, Cube16, EditBatch, EditOptions,
    GridId, GridTransform, Layer, Schema, Snapshot, StorageKind, Volume,
};
use quasar_voxel_mesh::*;

const SIZE: u32 = 64;
const GRID: GridId = GridId(1);

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

struct Blocks {
    volume: Volume<Cube16>,
    block: quasar_voxel::ChannelId<u16>,
    catalog: BlockCatalog,
}

fn blocks() -> Blocks {
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, AIR);
    let mut catalog = BlockCatalog::new();
    catalog.add(BlockDef::cube("red", 0));
    catalog.add(BlockDef::cube("glass", 1).with_layer(Layer::Translucent));
    catalog.add(BlockDef::cube("green", 2));
    Blocks {
        volume: Volume::new(schema),
        block,
        catalog,
    }
}

fn greedy_mesh(b: &Blocks, coord: IVec3) -> MeshBuffers {
    let mut snap = Snapshot::new();
    b.volume.snapshot(b.block, coord, 1, &mut snap);
    let mut out = MeshBuffers::new();
    GreedyExtractor::new(GreedyOptions {
        ao: false,
        light: false,
    })
    .extract(
        &snap,
        None,
        None,
        None,
        &opaque_table(&b.catalog),
        &CatalogGreedy::new(&b.catalog),
        &mut out,
    );
    out
}

fn materials(gpu: &Gpu) -> VoxelMaterials {
    let mut glass = MaterialDef::layer(1);
    glass.tint = [1.0, 1.0, 1.0, 0.5];
    VoxelMaterials::new(
        gpu,
        4,
        &[
            [255, 0, 0, 255].repeat(16),
            [255, 255, 255, 255].repeat(16),
            [0, 255, 0, 255].repeat(16),
        ],
        &[MaterialDef::layer(0), glass, MaterialDef::layer(2)],
        MaterialFilter::Nearest,
    )
    .unwrap()
}

fn camera(position: DVec3, target: DVec3) -> VoxelCamera {
    let projection = quasar_math::perspective(1.0, 1.0, 0.1, 500.0).unwrap();
    VoxelCamera::looking(position, (target - position).as_vec3(), Vec3::Y, projection).unwrap()
}

fn unlit() -> VoxelShading {
    VoxelShading {
        model: ShadingModel::Unlit,
        ao_strength: 0.0,
        ..VoxelShading::default()
    }
}

fn render(
    gpu: &Gpu,
    renderer: &mut VoxelRenderer,
    cam: &VoxelCamera,
    shading: &VoxelShading,
    options: &VoxelPassOptions,
    lights: &[Light],
) -> (Vec<u8>, VoxelStats) {
    let target = OffscreenTarget::new(gpu, SIZE, SIZE).unwrap();
    let mut stats = VoxelStats::default();
    let image = target
        .render(gpu, |gpu, target| {
            let prepared = renderer.prepare(
                gpu,
                target,
                cam,
                Lighting {
                    ambient: [1.0; 3],
                    lights,
                    ..Lighting::default()
                },
                None,
                shading,
                options,
            )?;
            stats = prepared.stats;
            Ok(vec![prepared.commands])
        })
        .unwrap();
    renderer.wait_and_retire(gpu).unwrap();
    (image.rgba, stats)
}

#[test]
#[ignore = "requires Vulkan"]
fn gpu_and_cpu_culling_render_identical_images_and_cull_offscreen_chunks() {
    let gpu = Gpu::headless().unwrap();
    let mut b = blocks();
    for x in 4..8 {
        for y in 4..8 {
            for z in 4..8 {
                b.volume.set(b.block, IVec3::new(x, y, z), 1);
            }
        }
    }
    // A second chunk far behind the camera (it looks down +Z).
    b.volume.set(b.block, IVec3::new(4, 4, -100), 3);
    let cam = camera(DVec3::new(6.0, 6.0, -10.0), DVec3::new(6.0, 6.0, 6.0));
    let mut images = Vec::new();
    for cull in [CullMode::Auto, CullMode::Cpu] {
        let mut renderer = VoxelRenderer::new(
            &gpu,
            materials(&gpu),
            VoxelRendererConfig {
                cull,
                ..Default::default()
            },
        )
        .unwrap();
        renderer.set_grid(GRID, GridTransform::default(), 1.0);
        for coord in [IVec3::ZERO, IVec3::new(0, 0, -7)] {
            let mesh = greedy_mesh(&b, coord);
            renderer
                .upload(
                    &gpu,
                    ChunkKey::new(GRID, coord),
                    ChunkPlacement::new(coord * 16),
                    &mesh,
                )
                .unwrap();
        }
        let (rgba, stats) = render(
            &gpu,
            &mut renderer,
            &cam,
            &unlit(),
            &VoxelPassOptions::default(),
            &[],
        );
        assert_eq!(
            pixel(&rgba, 32, 32),
            [255, 0, 0, 255],
            "{cull:?}: red cube at the centre"
        );
        assert_eq!(
            pixel(&rgba, 2, 2),
            [0, 0, 0, 255],
            "{cull:?}: clear colour at the corner"
        );
        assert_eq!(stats.records, 2);
        if cull == CullMode::Cpu || !renderer.uses_gpu_culling() {
            assert_eq!(
                stats.cpu_culled, 1,
                "the chunk behind the camera is rejected"
            );
        } else {
            assert!(stats.indirect_batches > 0, "multi-draw indirect path");
        }
        images.push(rgba);
    }
    assert_eq!(
        images[0], images[1],
        "GPU and CPU culling agree pixel for pixel"
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn translucent_chunks_blend_and_resort_when_the_camera_crosses_planes() {
    let gpu = Gpu::headless().unwrap();
    let mut b = blocks();
    for x in 2..14 {
        for y in 2..14 {
            b.volume.set(b.block, IVec3::new(x, y, 10), 1);
            b.volume.set(b.block, IVec3::new(x, y, 5), 2);
        }
    }
    let mut renderer =
        VoxelRenderer::new(&gpu, materials(&gpu), VoxelRendererConfig::default()).unwrap();
    renderer.set_grid(GRID, GridTransform::default(), 1.0);
    renderer
        .upload(
            &gpu,
            ChunkKey::new(GRID, IVec3::ZERO),
            ChunkPlacement::new(IVec3::ZERO),
            &greedy_mesh(&b, IVec3::ZERO),
        )
        .unwrap();
    let cam = camera(DVec3::new(8.0, 8.0, -12.0), DVec3::new(8.0, 8.0, 8.0));
    let (rgba, stats) = render(
        &gpu,
        &mut renderer,
        &cam,
        &unlit(),
        &VoxelPassOptions::default(),
        &[],
    );
    let c = pixel(&rgba, 32, 32);
    // Linear 0.5 glass over red, written to a UNORM target as linear values.
    // Linear 0.5 white glass (front face only) over red: (1, 0.5, 0.5).
    assert!(
        c[0] == 255 && (120..=135).contains(&c[1]) && c[1] == c[2],
        "glass blends over red: {c:?}"
    );
    assert_eq!(stats.resorted_chunks, 1, "first frame sorts");
    let (_, again) = render(
        &gpu,
        &mut renderer,
        &cam,
        &unlit(),
        &VoxelPassOptions::default(),
        &[],
    );
    assert_eq!(again.resorted_chunks, 0, "no plane crossed");
    let behind = camera(DVec3::new(8.0, 8.0, 7.5), DVec3::new(8.0, 8.0, 30.0));
    let (_, moved) = render(
        &gpu,
        &mut renderer,
        &behind,
        &unlit(),
        &VoxelPassOptions::default(),
        &[],
    );
    assert_eq!(
        moved.resorted_chunks, 1,
        "crossing the glass planes re-sorts"
    );
}

#[test]
#[ignore = "requires Vulkan"]
fn smooth_surfaces_light_by_normal_and_overlays_draw_lines() {
    let gpu = Gpu::headless().unwrap();
    let mut schema = Schema::new();
    let density = schema.channel("density", StorageKind::Dense, -127i8);
    let material = schema.channel("material", StorageKind::Paletted, 0u8);
    let mut volume = Volume::<Cube16>::new(schema);
    let mut batch = EditBatch::new();
    batch.brush(
        Brush::new(
            BrushShape::Sphere {
                center: Vec3::splat(8.0),
                radius: 6.0,
            },
            BrushOp::Add,
            density,
        )
        .with_material(material, 2u8),
    );
    volume.apply(&batch, EditOptions::default());
    let mut d = Snapshot::new();
    volume.snapshot(density, IVec3::ZERO, 1, &mut d);
    let mut m = Snapshot::new();
    volume.snapshot(material, IVec3::ZERO, 1, &mut m);
    let mut mesh = MeshBuffers::new();
    SurfaceNetsExtractor::new().extract(&d, Some(&m), SmoothOptions::default(), &mut mesh);
    let mut renderer =
        VoxelRenderer::new(&gpu, materials(&gpu), VoxelRendererConfig::default()).unwrap();
    renderer.set_grid(GRID, GridTransform::default(), 1.0);
    let stats = renderer
        .upload(
            &gpu,
            ChunkKey::new(GRID, IVec3::ZERO),
            ChunkPlacement::new(IVec3::ZERO),
            &mesh,
        )
        .unwrap();
    assert!(stats.smooth_triangles > 100);
    let cam = camera(DVec3::new(8.0, 8.0, -14.0), DVec3::new(8.0, 8.0, 8.0));
    let sun = [Light::Directional {
        direction: [-1.0, 0.0, 0.0],
        color: [1.0; 3],
        intensity: 1.0,
    }];
    let shading = VoxelShading {
        model: ShadingModel::Diffuse,
        ao_strength: 0.0,
        ..Default::default()
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let image = target
        .render(&gpu, |gpu, target| {
            Ok(vec![
                renderer
                    .prepare(
                        gpu,
                        target,
                        &cam,
                        Lighting {
                            ambient: [0.05; 3],
                            lights: &sun,
                            ..Lighting::default()
                        },
                        None,
                        &shading,
                        &VoxelPassOptions::default(),
                    )?
                    .commands,
            ])
        })
        .unwrap();
    // Light travels along -X: the +X side of the sphere (screen left, since
    // the camera looks down +Z) is lit, the -X side is dark.
    let lit = pixel(&image.rgba, 22, 32)[1];
    let dark = pixel(&image.rgba, 42, 32)[1];
    assert!(
        lit > 100 && dark < 30 && lit > dark * 4,
        "lit {lit} dark {dark}"
    );
    let options = VoxelPassOptions {
        overlays: vec![OverlayBox {
            grid: GRID,
            min: Vec3::splat(-2.0),
            max: Vec3::splat(18.0),
            color: [1.0, 1.0, 0.0, 1.0],
        }],
        ..Default::default()
    };
    let (rgba, _) = render(&gpu, &mut renderer, &cam, &unlit(), &options, &[]);
    let yellow = (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .filter(|(x, y)| {
            let p = pixel(&rgba, *x, *y);
            p[0] > 200 && p[1] > 200 && p[2] < 50
        })
        .count();
    assert!(yellow > 40, "outline pixels: {yellow}");
}

#[test]
#[ignore = "requires Vulkan"]
fn far_from_origin_rendering_is_exact_and_grids_move() {
    let gpu = Gpu::headless().unwrap();
    let mut b = blocks();
    for x in 6..10 {
        for y in 6..10 {
            for z in 6..10 {
                b.volume.set(b.block, IVec3::new(x, y, z), 3);
            }
        }
    }
    let mesh = greedy_mesh(&b, IVec3::ZERO);
    let mut images = Vec::new();
    for offset in [DVec3::ZERO, DVec3::new(1.0e9, -2.0e8, 7.0e8)] {
        let mut renderer =
            VoxelRenderer::new(&gpu, materials(&gpu), VoxelRendererConfig::default()).unwrap();
        renderer.set_grid(GRID, GridTransform::new(offset, DQuat::IDENTITY), 1.0);
        renderer
            .upload(
                &gpu,
                ChunkKey::new(GRID, IVec3::ZERO),
                ChunkPlacement::new(IVec3::ZERO),
                &mesh,
            )
            .unwrap();
        let cam = camera(
            offset + DVec3::new(8.0, 8.0, -6.0),
            offset + DVec3::splat(8.0),
        );
        images.push(
            render(
                &gpu,
                &mut renderer,
                &cam,
                &unlit(),
                &VoxelPassOptions::default(),
                &[],
            )
            .0,
        );
        if offset == DVec3::ZERO {
            // Moving the grid moves the image.
            renderer.set_grid(
                GRID,
                GridTransform::new(DVec3::new(40.0, 0.0, 0.0), DQuat::IDENTITY),
                1.0,
            );
            let (moved, _) = render(
                &gpu,
                &mut renderer,
                &cam,
                &unlit(),
                &VoxelPassOptions::default(),
                &[],
            );
            assert_eq!(pixel(&moved, 32, 32), [0, 0, 0, 255]);
            renderer.remove_grid(GRID);
            assert_eq!(renderer.chunk_count(), 0);
        }
    }
    assert_eq!(pixel(&images[0], 32, 32), [0, 255, 0, 255]);
    assert_eq!(images[0], images[1], "identical at a billion metres");
}

#[test]
#[ignore = "requires Vulkan"]
fn voxels_cast_into_render_3d_shadow_maps_and_memory_is_reused() {
    use quasar_render_3d::*;
    let gpu = Gpu::headless().unwrap();
    let mut b = blocks();
    for x in 6..10 {
        for z in 6..10 {
            b.volume.set(b.block, IVec3::new(x, 8, z), 1);
        }
    }
    let mesh = greedy_mesh(&b, IVec3::ZERO);
    let mut renderer =
        VoxelRenderer::new(&gpu, materials(&gpu), VoxelRendererConfig::default()).unwrap();
    renderer.set_grid(GRID, GridTransform::default(), 1.0);
    renderer
        .upload(
            &gpu,
            ChunkKey::new(GRID, IVec3::ZERO),
            ChunkPlacement::new(IVec3::ZERO),
            &mesh,
        )
        .unwrap();
    let config = ShadowConfig {
        resolution: 128,
        max_maps: 1,
        ..ShadowConfig::default()
    };
    let mut shadows = ShadowRenderState3D::new(&gpu, config).unwrap();
    let shadow_target = ShadowTarget::new(&gpu, config).unwrap();
    let light_vp = Mat4::orthographic(-12., 12., -12., 12., 0.1, 40.).unwrap()
        * Mat4::look_at([8., 20., 8.], [8., 0., 8.], [0., 0., 1.]).unwrap();
    let floor_mesh = MeshData::plane(8.0, 8.0, 1).upload(&gpu).unwrap();
    let (_white_texture, white) = {
        let texture = quasar_frame::texture_2d(
            &gpu.device,
            "white",
            1,
            1,
            vulkanic_gal::TextureFormat::Rgba8Unorm,
            vulkanic_gal::TextureUsages::TEXTURE_BINDING | vulkanic_gal::TextureUsages::COPY_DST,
        )
        .unwrap();
        gpu.queue.write_texture(
            texture.as_image_copy(),
            &[255; 4],
            vulkanic_gal::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            texture.size(),
        );
        let view = texture.create_view(&Default::default());
        (texture, view)
    };
    let mut lit = LitMaterial::textured(&white);
    lit.shading = ShadingModel::Diffuse;
    let mut receiver = SurfaceRenderState3D::new(&gpu, 1, 1).unwrap();
    let lights = [Light::Directional {
        direction: [0., -1., 0.],
        color: [1.; 3],
        intensity: 1.,
    }];
    let camera = LitCamera3D {
        view_projection: Mat4::orthographic(-10., 10., -10., 10., 0.1, 50.).unwrap()
            * Mat4::look_at([8., 30., 8.], [8., 0., 8.], [0., 0., 1.]).unwrap(),
        position: [8., 30., 8.],
    };
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let render_floor = |shadows: &mut ShadowRenderState3D,
                        receiver: &mut SurfaceRenderState3D,
                        renderer: &mut VoxelRenderer,
                        cast: bool| {
        target
            .render(&gpu, |gpu, target| {
                let maps = shadows.prepare_into(
                    gpu,
                    &[ShadowView {
                        light_index: 0,
                        view_projection: light_vp,
                        settings: ShadowSettings {
                            strength: 1.0,
                            normal_bias: 0.0,
                            filter: ShadowFilter::Hard,
                            ..ShadowSettings::default()
                        },
                        casters: &[],
                    }],
                    &shadow_target,
                )?;
                let binding = maps.binding().clone();
                let mut commands = vec![maps.commands];
                if cast {
                    commands.push(
                        renderer
                            .prepare_depth(
                                gpu,
                                shadow_target.layer_view(0).unwrap(),
                                light_vp.to_glam().as_dmat4(),
                                DVec3::ZERO,
                                false,
                            )?
                            .commands,
                    );
                }
                let draws = [SurfaceDraw3D {
                    mesh: &floor_mesh,
                    model: Mat4::translation(8., 0., 8.),
                    material: lit.clone().into(),
                }];
                commands.push(
                    receiver
                        .prepare(
                            gpu,
                            target,
                            camera,
                            Lighting {
                                ambient: [0.0; 3],
                                lights: &lights,
                                ..Lighting::default()
                            },
                            &draws,
                            Some(&binding),
                            PassOptions::default(),
                        )?
                        .commands,
                );
                Ok(commands)
            })
            .unwrap()
            .rgba
    };
    let open = render_floor(&mut shadows, &mut receiver, &mut renderer, false);
    let shaded = render_floor(&mut shadows, &mut receiver, &mut renderer, true);
    assert!(pixel(&open, 32, 32)[0] > 200, "unshadowed floor is lit");
    assert!(
        pixel(&shaded, 32, 32)[0] < 40,
        "voxel roof shadows the floor: {:?}",
        pixel(&shaded, 32, 32)
    );
    assert!(
        pixel(&shaded, 14, 14)[0] > 200,
        "floor outside the roof stays lit: {:?}",
        pixel(&shaded, 14, 14)
    );
    // Memory: removal frees after retirement and is reused.
    let used = renderer.geometry_bytes();
    assert!(used > 0);
    renderer.remove(ChunkKey::new(GRID, IVec3::ZERO));
    renderer.wait_and_retire(&gpu).unwrap();
    assert_eq!(renderer.geometry_bytes(), 0);
    renderer
        .upload(
            &gpu,
            ChunkKey::new(GRID, IVec3::ZERO),
            ChunkPlacement::new(IVec3::ZERO),
            &mesh,
        )
        .unwrap();
    assert_eq!(renderer.page_count(), 1, "the freed page is reused");
}

#[test]
#[ignore = "requires Vulkan"]
fn completed_submissions_retire_frees_without_blocking() {
    let gpu = Gpu::headless().unwrap();
    let mut b = blocks();
    b.volume.set(b.block, IVec3::new(2, 2, 2), 1);
    let mut renderer =
        VoxelRenderer::new(&gpu, materials(&gpu), VoxelRendererConfig::default()).unwrap();
    renderer.set_grid(GRID, GridTransform::default(), 1.0);
    let key = ChunkKey::new(GRID, IVec3::ZERO);
    renderer
        .upload(
            &gpu,
            key,
            ChunkPlacement::new(IVec3::ZERO),
            &greedy_mesh(&b, IVec3::ZERO),
        )
        .unwrap();
    let cam = camera(DVec3::new(2.5, 2.5, -6.0), DVec3::new(2.5, 2.5, 2.5));
    let target = OffscreenTarget::new(&gpu, SIZE, SIZE).unwrap();
    let mut submission = 0;
    target
        .render(&gpu, |gpu, target| {
            let prepared = renderer.prepare(
                gpu,
                target,
                &cam,
                Lighting::default(),
                None,
                &unlit(),
                &VoxelPassOptions::default(),
            )?;
            submission = prepared.submission;
            Ok(vec![prepared.commands])
        })
        .unwrap();
    renderer.submitted(&gpu, submission);
    // Removed after the frame that used it: freed once that frame completes.
    assert!(renderer.remove(key));
    assert!(renderer.geometry_bytes() > 0);
    gpu.wait().unwrap();
    renderer.retire_completed(&gpu);
    assert_eq!(renderer.geometry_bytes(), 0);
}

#[test]
#[ignore = "requires Vulkan"]
fn visibility_filters_skip_occluded_chunks() {
    let gpu = Gpu::headless().unwrap();
    let mut b = blocks();
    b.volume.set(b.block, IVec3::new(6, 6, 6), 1);
    let mut renderer =
        VoxelRenderer::new(&gpu, materials(&gpu), VoxelRendererConfig::default()).unwrap();
    renderer.set_grid(GRID, GridTransform::default(), 1.0);
    renderer
        .upload(
            &gpu,
            ChunkKey::new(GRID, IVec3::ZERO),
            ChunkPlacement::new(IVec3::ZERO),
            &greedy_mesh(&b, IVec3::ZERO),
        )
        .unwrap();
    let cam = camera(DVec3::new(6.5, 6.5, -4.0), DVec3::new(6.5, 6.5, 6.5));
    let (rgba, stats) = render(
        &gpu,
        &mut renderer,
        &cam,
        &unlit(),
        &VoxelPassOptions::default(),
        &[],
    );
    assert_eq!(pixel(&rgba, 32, 32), [255, 0, 0, 255]);
    assert_eq!(stats.occlusion_culled, 0);
    renderer.set_visible_chunks(GRID, Some(&[IVec3::ONE]));
    let (rgba, stats) = render(
        &gpu,
        &mut renderer,
        &cam,
        &unlit(),
        &VoxelPassOptions::default(),
        &[],
    );
    assert_eq!(pixel(&rgba, 32, 32), [0, 0, 0, 255], "filtered out");
    assert_eq!((stats.occlusion_culled, stats.records), (1, 0));
    renderer.set_visible_chunks(GRID, None);
    let (rgba, _) = render(
        &gpu,
        &mut renderer,
        &cam,
        &unlit(),
        &VoxelPassOptions::default(),
        &[],
    );
    assert_eq!(pixel(&rgba, 32, 32), [255, 0, 0, 255]);
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stream {
    Packed,
    Model,
    Smooth,
}

/// The wall slab of `wall_blocks` re-expressed in one geometry stream and
/// layer: lattice quads pack, quads nudged off the lattice need the model
/// stream, and smooth geometry uses the same faces as indexed triangles.
fn wall_in(b: &Blocks, stream: Stream, layer: Layer) -> MeshBuffers {
    let wall = greedy_mesh(b, IVec3::ZERO);
    let quads: Vec<_> = wall
        .layers
        .iter()
        .flat_map(|(_, l)| l.quads.clone())
        .collect();
    let mut buffers = quasar_voxel_mesh::LayerBuffers::default();
    match stream {
        Stream::Packed => buffers.quads = quads,
        Stream::Model => {
            buffers.quads = quads;
            for quad in &mut buffers.quads {
                for p in &mut quad.positions {
                    p.z -= 0.25;
                }
            }
        }
        Stream::Smooth => {
            for quad in &quads {
                let base = buffers.vertices.len() as u32;
                for p in quad.positions {
                    buffers.vertices.push(SmoothVertex {
                        position: p,
                        normal: quad.normal,
                        materials: [0, 0],
                        blend: 0,
                    });
                }
                buffers
                    .indices
                    .extend(quad.triangles().iter().map(|&i| base + i as u32));
            }
        }
    }
    MeshBuffers {
        layers: vec![(layer, buffers)],
    }
}

/// `mesh` repeated until it holds about `bytes` of GPU geometry (a filler
/// that shifts later uploads and, three at a time, spills into a new page).
fn padded(mesh: &MeshBuffers, stream: Stream, bytes: usize) -> MeshBuffers {
    let (layer, one) = &mesh.layers[0];
    let per_quad = match stream {
        Stream::Packed => 12,
        Stream::Model => 80,
        Stream::Smooth => 4 * 16 + 6 * 4,
    };
    let quads = one.quads.len().max(one.indices.len() / 6);
    let mut out = quasar_voxel_mesh::LayerBuffers::default();
    for _ in 0..(bytes / (per_quad * quads)).max(1) {
        out.quads.extend_from_slice(&one.quads);
        let base = out.vertices.len() as u32;
        out.vertices.extend_from_slice(&one.vertices);
        out.indices.extend(one.indices.iter().map(|i| base + i));
    }
    MeshBuffers {
        layers: vec![(*layer, out)],
    }
}

/// Writes depth with [`VoxelRenderer::prepare_depth`], then draws the colour
/// pass over it without clearing depth: geometry the depth pass drew hides
/// whatever the colour pass draws behind it.
fn render_over_depth(gpu: &Gpu, renderer: &mut VoxelRenderer, cam: &VoxelCamera) -> Vec<u8> {
    let depth = quasar_frame::texture_2d(
        &gpu.device,
        "test depth",
        SIZE,
        SIZE,
        vulkanic_gal::TextureFormat::Depth32Float,
        vulkanic_gal::TextureUsages::RENDER_ATTACHMENT,
    )
    .unwrap();
    let depth = depth.create_view(&Default::default());
    let target = OffscreenTarget::new(gpu, SIZE, SIZE).unwrap();
    let image = target
        .render(gpu, |gpu, target| {
            let mut target = target.clone();
            target.depth = Some(depth.clone());
            let vp = cam.projection.as_dmat4() * cam.view;
            let first = renderer.prepare_depth(gpu, &depth, vp, cam.position, true)?;
            let second = renderer.prepare(
                gpu,
                &target,
                cam,
                Lighting {
                    ambient: [1.0; 3],
                    ..Lighting::default()
                },
                None,
                &unlit(),
                &VoxelPassOptions {
                    clear_depth: false,
                    ..VoxelPassOptions::default()
                },
            )?;
            Ok(vec![first.commands, second.commands])
        })
        .unwrap();
    renderer.wait_and_retire(gpu).unwrap();
    image.rgba
}

#[test]
#[ignore = "requires Vulkan"]
fn every_stream_layer_and_cull_path_draws_at_any_arena_offset() {
    // The renderer's address arithmetic differs per stream, its draw path per
    // layer and cull mode, and both depend on where geometry lands in the
    // paged arenas: check every combination at fresh, shifted, later-page and
    // replaced-in-place offsets, then that removal clears it. Minimum-size
    // pages hold two ~24 KB fillers, so a third spills into a new page.
    let gpu = Gpu::headless().unwrap();
    let mut b = blocks();
    for x in 4..8 {
        for y in 4..8 {
            b.volume.set(b.block, IVec3::new(x, y, 6), 1);
        }
    }
    // A small green backdrop behind the wall, in the next chunk.
    for x in 5..7 {
        for y in 5..7 {
            b.volume.set(b.block, IVec3::new(x, y, 22), 3);
        }
    }
    let backdrop = ChunkKey::new(GRID, IVec3::new(0, 0, 1));
    let backdrop_mesh = greedy_mesh(&b, backdrop.chunk);
    let cam = camera(DVec3::new(6.0, 6.0, -10.0), DVec3::new(6.0, 6.0, 6.0));
    let target = ChunkKey::new(GRID, IVec3::ZERO);
    let mut combinations = 0;
    for cull in [CullMode::Auto, CullMode::Cpu] {
        let mut renderer = VoxelRenderer::new(
            &gpu,
            materials(&gpu),
            VoxelRendererConfig {
                cull,
                page_bytes: 64 << 10,
                ..Default::default()
            },
        )
        .unwrap();
        renderer.set_grid(GRID, GridTransform::default(), 1.0);
        for stream in [Stream::Packed, Stream::Model, Stream::Smooth] {
            for layer in [Layer::Opaque, Layer::Cutout, Layer::Translucent] {
                let mesh = wall_in(&b, stream, layer);
                // Translucent quads always take the sortable model stream.
                let used = match stream {
                    Stream::Packed if layer == Layer::Translucent => Stream::Model,
                    s => s,
                };
                let filler = padded(&mesh, used, 24 << 10);
                let indirect = renderer.uses_gpu_culling()
                    && layer != Layer::Translucent
                    && used != Stream::Smooth;
                for (fillers, replace) in [(0, false), (1, false), (3, true)] {
                    let case = format!(
                        "{cull:?} {stream:?} {layer:?} fillers={fillers} replace={replace}"
                    );
                    // Off-screen chunks of the same kind shift the wall's offset.
                    let filler_keys: Vec<ChunkKey> = (0..fillers)
                        .map(|i| ChunkKey::new(GRID, IVec3::new(i, 0, -7)))
                        .collect();
                    for key in &filler_keys {
                        renderer
                            .upload(&gpu, *key, ChunkPlacement::new(key.chunk * 16), &filler)
                            .unwrap();
                    }
                    for _ in 0..=usize::from(replace) {
                        let stats = renderer
                            .upload(&gpu, target, ChunkPlacement::new(IVec3::ZERO), &mesh)
                            .unwrap();
                        let streams = (
                            stats.packed_quads > 0,
                            stats.model_quads > 0,
                            stats.smooth_triangles > 0,
                        );
                        let expected = (
                            used == Stream::Packed,
                            used == Stream::Model,
                            used == Stream::Smooth,
                        );
                        assert_eq!(streams, expected, "{case}: stream used");
                    }
                    let (rgba, stats) = render(
                        &gpu,
                        &mut renderer,
                        &cam,
                        &unlit(),
                        &VoxelPassOptions::default(),
                        &[],
                    );
                    assert_eq!(pixel(&rgba, 32, 32), [255, 0, 0, 255], "{case}: wall");
                    assert_eq!(pixel(&rgba, 2, 2), [0, 0, 0, 255], "{case}: background");
                    assert_eq!(
                        (stats.indirect_batches > 0, stats.direct_draws > 0),
                        (indirect, !indirect),
                        "{case}: draw path"
                    );
                    // Depth pass: the wall (unless translucent, which casts
                    // no depth) must hide the backdrop the colour pass draws.
                    renderer
                        .upload(
                            &gpu,
                            backdrop,
                            ChunkPlacement::new(backdrop.chunk * 16),
                            &backdrop_mesh,
                        )
                        .unwrap();
                    renderer.set_visible_chunks(GRID, Some(&[backdrop.chunk]));
                    let rgba = render_over_depth(&gpu, &mut renderer, &cam);
                    let expected = if layer == Layer::Translucent {
                        [0, 255, 0, 255]
                    } else {
                        [0, 0, 0, 255]
                    };
                    assert_eq!(pixel(&rgba, 32, 32), expected, "{case}: depth pass");
                    renderer.set_visible_chunks(GRID, None);
                    for key in filler_keys.iter().chain([&target, &backdrop]) {
                        assert!(renderer.remove(*key), "{case}");
                    }
                    let (rgba, _) = render(
                        &gpu,
                        &mut renderer,
                        &cam,
                        &unlit(),
                        &VoxelPassOptions::default(),
                        &[],
                    );
                    assert_eq!(pixel(&rgba, 32, 32), [0, 0, 0, 255], "{case}: removed");
                    combinations += 1;
                }
            }
        }
        assert!(renderer.page_count() > 3, "later pages were exercised");
    }
    assert_eq!(combinations, 54);
}

#[test]
#[ignore = "requires Vulkan"]
fn emissive_masks_glow_per_texel_and_height_fog_follows_world_height() {
    use quasar_render_3d::{Fog, HeightFog};
    let gpu = Gpu::headless().unwrap();
    // Layer 3: mask lit on its left half. Layer 4: black.
    let mask: Vec<u8> = (0..16)
        .flat_map(|i| {
            if i % 4 < 2 {
                [255u8; 4]
            } else {
                [0, 0, 0, 255]
            }
        })
        .collect();
    let ore = MaterialDef::layer(4).with_emissive_mask([0.0, 1.0, 0.0], 3);
    let materials = VoxelMaterials::new(
        &gpu,
        4,
        &[
            [255, 0, 0, 255].repeat(16),
            [255, 255, 255, 255].repeat(16),
            [0, 255, 0, 255].repeat(16),
            mask,
            [0, 0, 0, 255].repeat(16),
        ],
        &[
            MaterialDef::layer(0),
            MaterialDef::layer(1),
            MaterialDef::layer(2),
            ore,
        ],
        MaterialFilter::Nearest,
    )
    .unwrap();
    assert!(
        VoxelMaterials::new(
            &gpu,
            4,
            &[[0u8; 64].to_vec()],
            &[MaterialDef::layer(0).with_emissive_mask([1.0; 3], 7)],
            MaterialFilter::Nearest,
        )
        .is_err(),
        "mask layers must exist"
    );
    let mut b = blocks();
    b.catalog.add(BlockDef::cube("ore", 3));
    // A 4x4x4 ore cube far from the origin, to exercise camera rebasing.
    let base = IVec3::new(4, 10_004, 4);
    for x in 0..4 {
        for y in 0..4 {
            for z in 0..4 {
                b.volume.set(b.block, base + IVec3::new(x, y, z), 4);
            }
        }
    }
    let chunk = IVec3::new(0, 10_004 / 16, 0);
    let mut renderer = VoxelRenderer::new(&gpu, materials, VoxelRendererConfig::default()).unwrap();
    renderer.set_grid(GRID, GridTransform::default(), 1.0);
    let mesh = greedy_mesh(&b, chunk);
    renderer
        .upload(
            &gpu,
            ChunkKey::new(GRID, chunk),
            ChunkPlacement::new(chunk * 16),
            &mesh,
        )
        .unwrap();
    // Looking straight down at the cube's top face.
    let top = DVec3::new(6.0, 10_008.0, 6.0);
    let cam = camera(top + DVec3::new(0.0, 8.0, 0.001), top);
    let row = |rgba: &[u8]| -> Vec<[u8; 4]> { (20..44).map(|x| pixel(rgba, x, 32)).collect() };
    let (rgba, _) = render(
        &gpu,
        &mut renderer,
        &cam,
        &unlit(),
        &VoxelPassOptions::default(),
        &[],
    );
    let row0 = row(&rgba);
    assert!(
        row0.iter().any(|p| p[1] > 200 && p[0] < 20),
        "glowing texels: {row0:?}"
    );
    assert!(row0.iter().any(|p| p[1] < 20), "dark texels: {row0:?}");
    // Height fog whose base sits at the cube top: the top face is in thick
    // haze. Moved 30 m lower, the top face is clear again.
    let haze = |base_height: f32| VoxelShading {
        fog: Some(Fog::height(
            [0.0, 0.0, 1.0],
            HeightFog {
                base: base_height,
                density: 0.8,
                falloff: 0.5,
            },
        )),
        ..unlit()
    };
    let (fogged, _) = render(
        &gpu,
        &mut renderer,
        &cam,
        &haze(10_008.0),
        &VoxelPassOptions::default(),
        &[],
    );
    let (clear, _) = render(
        &gpu,
        &mut renderer,
        &cam,
        &haze(9_978.0),
        &VoxelPassOptions::default(),
        &[],
    );
    let blue = |rgba: &[u8]| row(rgba).iter().map(|p| u32::from(p[2])).sum::<u32>() / 24;
    assert!(blue(&fogged) > 150, "in the haze: {}", blue(&fogged));
    assert!(blue(&clear) < 10, "above the haze: {}", blue(&clear));
}
