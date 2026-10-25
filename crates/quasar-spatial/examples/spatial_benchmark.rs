//! Local spatial-index workload probe. Not a portable benchmark.
//!
//! ```sh
//! cargo run --release -p quasar-spatial --example spatial_benchmark
//! ```
use quasar_spatial::{Aabb, GridConfig, Ray, SpatialGrid};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }
    fn range(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.next()
    }
}

fn percentile(samples: &mut [Duration], fraction: f64) -> Duration {
    samples.sort_unstable();
    let index = ((samples.len() - 1) as f64 * fraction).round() as usize;
    samples[index]
}

fn micros(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

fn main() {
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let mut grid = SpatialGrid::<2, u64>::new(GridConfig::new(8.0, 256, 100_000).unwrap()).unwrap();
    let mut boxes = Vec::new();
    let start = Instant::now();
    for id in 0..20_000u64 {
        let center = [rng.range(-2_000.0, 2_000.0), rng.range(-2_000.0, 2_000.0)];
        let half = [rng.range(0.25, 6.0), rng.range(0.25, 6.0)];
        let bounds = Aabb::from_center_half_extents(center, half).unwrap();
        grid.insert(id, bounds);
        boxes.push((id, bounds));
    }
    let build = start.elapsed();
    println!(
        "build 2d items={} occupied_cells={} references={} time_ms={:.3}",
        grid.len(),
        grid.occupied_cells(),
        grid.cell_references(),
        build.as_secs_f64() * 1_000.0
    );

    let mut query_times = Vec::with_capacity(2_000);
    let mut ray_times = Vec::with_capacity(2_000);
    let mut total_hits = 0usize;
    let start = Instant::now();
    for _ in 0..2_000 {
        let center = [rng.range(-2_000.0, 2_000.0), rng.range(-2_000.0, 2_000.0)];
        let region = Aabb::from_center_half_extents(center, [40.0, 30.0]).unwrap();
        let query_start = Instant::now();
        let hits = grid.query(region);
        query_times.push(query_start.elapsed());
        total_hits += hits.len();

        let direction = [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)];
        if let Ok(ray) = Ray::new(center, direction) {
            let ray_start = Instant::now();
            let hit = grid.first_ray(&ray, 500.0);
            ray_times.push(ray_start.elapsed());
            black_box(hit);
        }
    }
    let queries = start.elapsed();
    let brute_start = Instant::now();
    let mut brute_hits = 0usize;
    for _ in 0..200 {
        let center = [rng.range(-2_000.0, 2_000.0), rng.range(-2_000.0, 2_000.0)];
        let region = Aabb::from_center_half_extents(center, [40.0, 30.0]).unwrap();
        brute_hits += boxes
            .iter()
            .filter(|(_, bounds)| bounds.intersects(&region))
            .count();
    }
    let brute = brute_start.elapsed();
    let mut updates = Vec::with_capacity(5_000);
    let update_start = Instant::now();
    for _ in 0..5_000 {
        let index = (rng.next() * boxes.len() as f64) as usize % boxes.len();
        let (id, bounds) = boxes[index];
        let moved = Aabb::new(
            [bounds.min[0] + 0.25, bounds.min[1] - 0.25],
            [bounds.max[0] + 0.25, bounds.max[1] - 0.25],
        )
        .unwrap();
        let step_start = Instant::now();
        grid.update(id, moved).unwrap();
        updates.push(step_start.elapsed());
        boxes[index].1 = moved;
    }
    let update = update_start.elapsed();

    let p50 = percentile(&mut query_times, 0.50);
    let p99 = percentile(&mut query_times, 0.99);
    let max = *query_times.last().unwrap();
    println!(
        "query 2d n=2000 total_ms={:.3} mean_us={:.3} p50_us={:.3} p99_us={:.3} max_us={:.3} avg_hits={:.1}",
        queries.as_secs_f64() * 1_000.0,
        micros(queries) / 2_000.0,
        micros(p50),
        micros(p99),
        micros(max),
        total_hits as f64 / 2_000.0
    );
    println!(
        "ray 2d n={} mean_us={:.3} p50_us={:.3} p99_us={:.3} max_us={:.3}",
        ray_times.len(),
        ray_times.iter().map(|d| micros(*d)).sum::<f64>() / ray_times.len() as f64,
        micros(percentile(&mut ray_times, 0.50)),
        micros(percentile(&mut ray_times, 0.99)),
        micros(*ray_times.last().unwrap())
    );
    println!(
        "bruteforce 2d n=200 total_ms={:.3} mean_us={:.3} hits={brute_hits}",
        brute.as_secs_f64() * 1_000.0,
        micros(brute) / 200.0
    );
    println!(
        "update 2d n=5000 total_ms={:.3} mean_us={:.3} p50_us={:.3} p99_us={:.3}",
        update.as_secs_f64() * 1_000.0,
        micros(update) / 5_000.0,
        micros(percentile(&mut updates, 0.50)),
        micros(percentile(&mut updates, 0.99))
    );
    println!("memory 2d {:?}", grid.memory());
}
