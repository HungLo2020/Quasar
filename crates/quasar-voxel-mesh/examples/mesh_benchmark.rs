//! Extraction benchmark against the voxel spec's targets.
//!
//!     cargo run --release -p quasar-voxel-mesh --example mesh_benchmark -- --check
//!
//! Prints JSON. With `--check`, exits non-zero when a target is missed
//! (meaningful only in release builds).
use quasar_math::{IVec3, Vec3};
use quasar_voxel::*;
use quasar_voxel_mesh::*;
use std::time::Instant;

fn terrain<L: ChunkLayout>(seed: u64) -> (Volume<L>, ChannelId<u16>, BlockCatalog) {
    terrain_with::<L>(seed, true)
}

/// `rough`: per-column noise, scattered glass and caves (worst case);
/// otherwise smooth rolling hills (typical terrain).
fn terrain_with<L: ChunkLayout>(
    seed: u64,
    rough: bool,
) -> (Volume<L>, ChannelId<u16>, BlockCatalog) {
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, AIR);
    let mut catalog = BlockCatalog::new();
    let stone = catalog.add(BlockDef::cube("stone", 1));
    let dirt = catalog.add(BlockDef::cube("dirt", 2));
    let glass = catalog.add(BlockDef::cube("glass", 3).with_layer(Layer::Translucent));
    let mut volume = Volume::new(schema);
    let mut rng = quasar_core::Rng::seed_from_u64(seed);
    let n = L::EDGE as i32;
    let mut batch = EditBatch::new();
    for x in -1..=n {
        for z in -1..=n {
            let noise = if rough { rng.range_i32(0..3) } else { 0 };
            let h = n / 3
                + ((x as f32 * 0.11).sin() * 3.0 + (z as f32 * 0.09).cos() * 3.0) as i32
                + noise;
            for y in -1..h.min(n + 1) {
                let b = if y + 1 == h {
                    dirt
                } else if rough && rng.chance(0.02) {
                    glass
                } else {
                    stone
                };
                batch.set(block, IVec3::new(x, y, z), b);
            }
            // Caves.
            if rough && rng.chance(0.05) {
                batch.fill(block, IVec3::new(x, 2, z), IVec3::new(x, 4, z), AIR);
            }
        }
    }
    volume.apply(
        &batch,
        EditOptions {
            record_changes: false,
            ..Default::default()
        },
    );
    (volume, block, catalog)
}

fn time<T>(iterations: usize, mut f: impl FnMut() -> T) -> f64 {
    let _ = f(); // warm scratch buffers
    let start = Instant::now();
    for _ in 0..iterations {
        std::hint::black_box(f());
    }
    start.elapsed().as_secs_f64() * 1e6 / iterations as f64
}

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let iterations = 200;
    // Greedy, 32³ terrain.
    let (v32, block32, catalog32) = terrain::<Cube32>(1);
    let mut snap32 = Snapshot::new();
    v32.snapshot(block32, IVec3::ZERO, 1, &mut snap32);
    let opaque32 = opaque_table(&catalog32);
    let greedy_materials = CatalogGreedy::new(&catalog32);
    let mut greedy = GreedyExtractor::new(GreedyOptions {
        ao: true,
        light: false,
    });
    let mut sink = MeshBuffers::new();
    let greedy_us = time(iterations, || {
        sink.clear();
        greedy.extract(
            &snap32,
            None,
            None,
            None,
            &opaque32,
            &greedy_materials,
            &mut sink,
        )
    });
    let greedy_quads = sink.quad_count();
    let mut flat_greedy = GreedyExtractor::new(GreedyOptions {
        ao: false,
        light: false,
    });
    let greedy_noao_us = time(iterations, || {
        sink.clear();
        flat_greedy.extract(
            &snap32,
            None,
            None,
            None,
            &opaque32,
            &greedy_materials,
            &mut sink,
        )
    });
    let (hills, hills_block, hills_catalog) = terrain_with::<Cube32>(4, false);
    let mut hills_snap = Snapshot::new();
    hills.snapshot(hills_block, IVec3::ZERO, 1, &mut hills_snap);
    let hills_opaque = opaque_table(&hills_catalog);
    let hills_materials = CatalogGreedy::new(&hills_catalog);
    let greedy_hills_us = time(iterations, || {
        sink.clear();
        greedy.extract(
            &hills_snap,
            None,
            None,
            None,
            &hills_opaque,
            &hills_materials,
            &mut sink,
        )
    });
    let greedy_hills_quads = sink.quad_count();
    // Culled models, 16³ terrain.
    let (v16, block16, catalog16) = terrain::<Cube16>(2);
    let mut snap16 = Snapshot::new();
    v16.snapshot(block16, IVec3::ZERO, 1, &mut snap16);
    let opaque16 = opaque_table(&catalog16);
    let library = ModelLibrary::new();
    let models = CatalogModels {
        catalog: &catalog16,
        library: &library,
    };
    let faces = CatalogFaces(&catalog16);
    let layers = CatalogLayers(&catalog16);
    let mut culled = CulledModelExtractor::default();
    let mut sink16 = MeshBuffers::new();
    let culled_us = time(iterations, || {
        sink16.clear();
        culled.extract(
            CulledInput {
                blocks: &snap16,
                orientation: None,
                block_light: None,
                sky_light: None,
                opaque: &opaque16,
            },
            &models,
            &faces,
            &VertexShade::default(),
            &layers,
            |_| true,
            &mut sink16,
        )
    });
    let culled_quads = sink16.quad_count();
    // Snapshot fill (16³, pad 1).
    let snapshot_us = time(iterations * 5, || {
        v16.snapshot(block16, IVec3::ZERO, 1, &mut snap16);
        snap16.edge()
    });
    // Surface Nets, 32³ node.
    let mut schema = Schema::new();
    let density = schema.channel("density", StorageKind::Dense, -127i8);
    let material = schema.channel("material", StorageKind::Paletted, 0u8);
    let mut smooth = Volume::<Cube32>::new(schema);
    let mut batch = EditBatch::new();
    batch.brush(
        Brush::new(
            BrushShape::Sphere {
                center: Vec3::splat(16.0),
                radius: 13.0,
            },
            BrushOp::Add,
            density,
        )
        .with_material(material, 1u8),
    );
    for i in 0..12 {
        let a = i as f32 * 0.7;
        batch.brush(Brush::new(
            BrushShape::Sphere {
                center: Vec3::new(
                    16.0 + 12.0 * a.cos(),
                    16.0 + 6.0 * (a * 1.3).sin(),
                    16.0 + 12.0 * a.sin(),
                ),
                radius: 4.0,
            },
            BrushOp::Subtract,
            density,
        ));
    }
    smooth.apply(
        &batch,
        EditOptions {
            record_changes: false,
            ..Default::default()
        },
    );
    let mut d = Snapshot::new();
    smooth.snapshot(density, IVec3::ZERO, 1, &mut d);
    let mut m = Snapshot::new();
    smooth.snapshot(material, IVec3::ZERO, 1, &mut m);
    let mut nets = SurfaceNetsExtractor::new();
    let mut smooth_sink = MeshBuffers::new();
    let nets_us = time(iterations, || {
        smooth_sink.clear();
        nets.extract(&d, Some(&m), SmoothOptions::default(), &mut smooth_sink)
    });
    let smooth_triangles = smooth_sink.triangle_count();
    // Spec §11 targets (single thread, release build).
    let targets = [
        ("greedy_32_hills_us", greedy_hills_us, 150.0),
        ("greedy_32_us", greedy_us, 500.0),
        ("culled_16_us", culled_us, 300.0),
        ("surface_nets_32_us", nets_us, 450.0),
        ("snapshot_16_us", snapshot_us, 50.0),
    ];
    println!("{{");
    println!("  \"greedy_32_us\": {greedy_us:.2},");
    println!("  \"greedy_32_quads\": {greedy_quads},");
    println!("  \"greedy_32_no_ao_us\": {greedy_noao_us:.2},");
    println!("  \"greedy_32_hills_us\": {greedy_hills_us:.2},");
    println!("  \"greedy_32_hills_quads\": {greedy_hills_quads},");
    println!("  \"culled_16_us\": {culled_us:.2},");
    println!("  \"culled_16_quads\": {culled_quads},");
    println!("  \"snapshot_16_us\": {snapshot_us:.2},");
    println!("  \"surface_nets_32_us\": {nets_us:.2},");
    println!("  \"surface_nets_32_triangles\": {smooth_triangles},");
    println!("  \"release\": {}", !cfg!(debug_assertions));
    println!("}}");
    if check {
        let missed: Vec<_> = targets.iter().filter(|(_, v, t)| v > t).collect();
        for (name, value, target) in &missed {
            eprintln!("target missed: {name} = {value:.1} > {target}");
        }
        if !missed.is_empty() {
            std::process::exit(1);
        }
    }
}
