//! CPU cost of preparing a voxel frame with many resident chunks.
//!
//!     cargo run --release -p quasar-render-voxel --example render_benchmark -- --check
//!
//! Uploads a 65×5×65 chunk world (a 32-chunk view radius, ~21k chunks with
//! geometry in every column), then times `VoxelRenderer::prepare` over many
//! frames. Prints JSON; `--check` fails when the median exceeds the spec's
//! 1 ms draw-submission target (release builds, GPU culling available). The
//! gate is `quasar_profile::micro`: misses re-measure the frame loop and the
//! median is normalised by a reference workload's slowdown, so a busy
//! desktop does not fail it while a regression still does.
use quasar_frame::{Gpu, OffscreenTarget};
use quasar_math::{DVec3, IVec3, Vec3};
use quasar_profile::micro::{Metric, MicroGate};
use quasar_render_3d::Lighting;
use quasar_render_voxel::*;
use quasar_voxel::{
    AIR, BlockCatalog, BlockDef, Cube16, EditBatch, EditOptions, GridId, GridTransform, Schema,
    Snapshot, StorageKind, Volume,
};
use quasar_voxel_mesh::*;
use std::time::Instant;

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let radius = 32;
    let gpu = Gpu::headless().expect("GPU");
    // One rolling-hills chunk mesh, placed at every chunk of the world.
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, AIR);
    let mut catalog = BlockCatalog::new();
    let stone = catalog.add(BlockDef::cube("stone", 0));
    let mut volume = Volume::<Cube16>::new(schema);
    let mut batch = EditBatch::new();
    for x in -1..=16 {
        for z in -1..=16 {
            let h = 6 + ((x as f32 * 0.4).sin() * 2.0 + (z as f32 * 0.3).cos() * 2.0) as i32;
            batch.fill(block, IVec3::new(x, -1, z), IVec3::new(x, h, z), stone);
        }
    }
    volume.apply(
        &batch,
        EditOptions {
            record_changes: false,
            ..Default::default()
        },
    );
    let mut snap = Snapshot::new();
    volume.snapshot(block, IVec3::ZERO, 1, &mut snap);
    let mut mesh = MeshBuffers::new();
    GreedyExtractor::new(GreedyOptions {
        ao: true,
        light: false,
    })
    .extract(
        &snap,
        None,
        None,
        None,
        &opaque_table(&catalog),
        &CatalogGreedy::new(&catalog),
        &mut mesh,
    );
    let materials = VoxelMaterials::solid_colors(&gpu, &[[120, 120, 120, 255]]).expect("materials");
    let mut renderer =
        VoxelRenderer::new(&gpu, materials, VoxelRendererConfig::default()).expect("renderer");
    let grid = GridId(1);
    renderer.set_grid(grid, GridTransform::default(), 1.0);
    let upload = Instant::now();
    let mut chunks = 0;
    for z in -radius..=radius {
        for x in -radius..=radius {
            for y in 0..5 {
                let c = IVec3::new(x, y, z);
                renderer
                    .upload(
                        &gpu,
                        ChunkKey::new(grid, c),
                        ChunkPlacement::new(c * 16),
                        &mesh,
                    )
                    .expect("upload");
                chunks += 1;
            }
        }
    }
    let upload_ms = upload.elapsed().as_secs_f64() * 1e3;
    let target = OffscreenTarget::new(&gpu, 640, 360).expect("target");
    let projection = quasar_math::perspective(1.2, 16.0 / 9.0, 0.1, 1200.0).unwrap();
    let gpu_culling = renderer.uses_gpu_culling();
    let quads_per_chunk = mesh.quad_count();
    let mut measure = || {
        let mut samples = Vec::new();
        let mut stats = VoxelStats::default();
        for frame in 0..60 {
            let yaw = frame as f32 * 0.05;
            let camera = VoxelCamera::looking(
                DVec3::new(8.0, 60.0, 8.0),
                Vec3::new(yaw.sin(), -0.3, -yaw.cos()),
                Vec3::Y,
                projection,
            )
            .unwrap();
            target
                .render(&gpu, |gpu, target| {
                    let started = Instant::now();
                    let prepared = renderer.prepare(
                        gpu,
                        target,
                        &camera,
                        Lighting::default(),
                        None,
                        &VoxelShading::default(),
                        &VoxelPassOptions::default(),
                    )?;
                    if frame >= 10 {
                        samples.push(started.elapsed().as_secs_f64() * 1e6);
                    }
                    stats = prepared.stats;
                    Ok(vec![prepared.commands])
                })
                .expect("frame");
            renderer.retire(u64::MAX);
        }
        samples.sort_by(f64::total_cmp);
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let p95 = samples[(samples.len() as f64 * 0.95) as usize - 1];
        let median = samples[samples.len() / 2];
        vec![
            Metric::exact("chunks", chunks as f64, None),
            Metric::exact("records", stats.records as f64, None),
            Metric::exact("quads_per_chunk", quads_per_chunk as f64, None),
            Metric::timed("prepare_mean_us", mean, None),
            // The median resists scheduling noise on shared machines.
            Metric::timed("prepare_median_us", median, gpu_culling.then_some(1000.0)),
            Metric::timed("prepare_p95_us", p95, None),
            Metric::exact("indirect_batches", stats.indirect_batches as f64, None),
        ]
    };
    let report = MicroGate::default().run(check, &mut measure);
    println!(
        "{}",
        report.to_json(&[
            ("upload_ms", format!("{upload_ms:.1}")),
            ("gpu_culling", gpu_culling.to_string()),
            ("release", (!cfg!(debug_assertions)).to_string()),
        ])
    );
    if check && !report.report_misses() {
        std::process::exit(1);
    }
}
