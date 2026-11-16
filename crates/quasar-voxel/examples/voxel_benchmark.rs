//! Core voxel benchmark: storage, snapshots, edits, raycasts, propagation
//! and codecs.
//!
//!     cargo run --release -p quasar-voxel --example voxel_benchmark -- --check
//!
//! Prints JSON. With `--check`, exits non-zero when a spec §11 target is
//! missed (meaningful only in release builds). Timing and the gate come from
//! `quasar_profile::micro` (best-of-batch timings, retried passes and
//! slowdown normalisation, so a busy desktop does not fail the gate).
use quasar_math::{IVec3, Vec3};
use quasar_profile::micro::{Metric, MicroGate, best_batch_micros};
use quasar_voxel::*;
use std::time::Instant;

fn time<T>(iterations: usize, f: impl FnMut() -> T) -> f64 {
    best_batch_micros(iterations, 10, f)
}

struct BlockLight {
    block: ChannelId<u16>,
}

impl PropagationRules<Cube16> for BlockLight {
    fn cost(&self, volume: &Volume<Cube16>, cursor: &mut Cursor, to: IVec3, _: Face) -> Option<u8> {
        (volume.get_with(cursor, self.block, to) != 1).then_some(1)
    }
    fn emission(&self, volume: &Volume<Cube16>, cursor: &mut Cursor, at: IVec3) -> u8 {
        if volume.get_with(cursor, self.block, at) == 2 {
            14
        } else {
            0
        }
    }
}

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let report = MicroGate::default().run(check, measure);
    let release = !cfg!(debug_assertions);
    println!("{}", report.to_json(&[("release", release.to_string())]));
    if check && !report.report_misses() {
        std::process::exit(1);
    }
}

/// One measurement pass over freshly generated content.
fn measure() -> Vec<Metric> {
    let mut schema = Schema::new();
    let block = schema.channel("block", StorageKind::Paletted, AIR);
    let light = schema.derived_channel("light", StorageKind::Nibble, 0u8);
    let mut volume = Volume::<Cube16>::new(schema);
    // 3×3×3 chunks of terrain-like content.
    let mut rng = quasar_core::Rng::seed_from_u64(7);
    let mut batch = EditBatch::new();
    for x in -16..32 {
        for z in -16..32 {
            let h = 4 + ((x as f32 * 0.2).sin() * 3.0 + (z as f32 * 0.17).cos() * 3.0) as i32;
            batch.fill(block, IVec3::new(x, -16, z), IVec3::new(x, h, z), 1);
        }
    }
    let apply_us = {
        let start = Instant::now();
        volume.apply(
            &batch,
            EditOptions {
                record_changes: false,
                ..Default::default()
            },
        );
        start.elapsed().as_secs_f64() * 1e6
    };
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                volume.ensure_chunk(IVec3::new(x, y, z));
            }
        }
    }
    // Uniform chunk overhead (spec: 32 B header + 32 B per channel).
    volume.ensure_chunk(IVec3::new(0, 40, 0));
    let uniform_bytes = volume
        .chunk(IVec3::new(0, 40, 0))
        .map_or(0, Chunk::memory_bytes);
    let uniform_budget = 32 + 32 * volume.schema().len();

    // Random point writes and reads.
    let points: Vec<IVec3> = (0..4096)
        .map(|_| {
            IVec3::new(
                rng.range_i32(-16..32),
                rng.range_i32(-16..32),
                rng.range_i32(-16..32),
            )
        })
        .collect();
    let mut flip = 0u16;
    let set_ns = time(50, || {
        flip ^= 1;
        for p in &points {
            volume.set(block, *p, 3 + flip);
        }
    }) * 1000.0
        / points.len() as f64;
    let get_ns = time(200, || {
        points
            .iter()
            .map(|p| u32::from(volume.get(block, *p)))
            .sum::<u32>()
    }) * 1000.0
        / points.len() as f64;

    // Snapshot of a mixed chunk.
    let mut snap = Snapshot::new();
    let snapshot_us = time(1000, || {
        volume.snapshot(block, IVec3::ZERO, 1, &mut snap);
        snap.edge()
    });

    // Raycasts across the loaded region.
    let rays: Vec<(Vec3, Vec3)> = (0..256)
        .map(|i| {
            let a = i as f32 * 0.37;
            (
                Vec3::new(8.0, 30.0, 8.0),
                Vec3::new(a.cos(), -0.6, a.sin()).normalize(),
            )
        })
        .collect();
    let raycast_us = time(200, || {
        rays.iter()
            .filter(|(o, d)| {
                volume
                    .raycast(block, *o, *d, 64.0, TieBreak::default(), |_, v| v != 0)
                    .is_some()
            })
            .count()
    }) / rays.len() as f64;

    // Edit batch with a density-free sphere fill.
    let sphere_us = time(50, || {
        let mut b = EditBatch::new();
        b.fill(block, IVec3::new(2, 2, 2), IVec3::new(10, 10, 10), 0);
        volume
            .apply(
                &b,
                EditOptions {
                    record_changes: true,
                    ..Default::default()
                },
            )
            .dirty
            .len()
    });

    // Block light: place a lamp in a dug cave and propagate, then remove it.
    let rules = BlockLight { block };
    let mut engine = Propagator::new(QueueDiscipline::BucketFifo);
    let lamp = IVec3::new(6, 6, 6);
    let light_us = time(50, || {
        volume.set(block, lamp, 2);
        let add = engine.update(&mut volume, light, &[lamp], &rules).changed;
        volume.set(block, lamp, 0);
        add + engine.update(&mut volume, light, &[lamp], &rules).changed
    });

    // Codec round trip of the mixed chunk.
    let schema = volume.schema().clone();
    let chunk = volume.chunk(IVec3::ZERO).unwrap().clone();
    let bytes = codec::encode_chunk(&schema, &chunk, false);
    let encode_us = time(500, || codec::encode_chunk(&schema, &chunk, false).len());
    let decode_us = time(500, || {
        codec::decode_chunk::<Cube16>(&schema, &bytes).is_ok()
    });

    vec![
        Metric::timed("apply_terrain_us", apply_us, None),
        Metric::exact(
            "uniform_chunk_bytes",
            uniform_bytes as f64,
            Some(uniform_budget as f64),
        ),
        Metric::timed("set_ns", set_ns, None),
        Metric::timed("get_ns", get_ns, None),
        Metric::timed("snapshot_16_us", snapshot_us, Some(50.0)),
        Metric::timed("raycast_us", raycast_us, None),
        Metric::timed("fill_9x9x9_us", sphere_us, None),
        Metric::timed("light_place_remove_us", light_us, None),
        Metric::timed("encode_chunk_us", encode_us, None),
        Metric::timed("decode_chunk_us", decode_us, None),
        Metric::exact("encoded_chunk_bytes", bytes.len() as f64, None),
    ]
}
