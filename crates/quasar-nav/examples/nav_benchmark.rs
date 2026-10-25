//! Local grid/pathfinding workload probe.
//!
//! ```sh
//! cargo run --release -p quasar-nav --example nav_benchmark
//! ```
use quasar_nav::{Grid, GridCell, SearchOptions, SearchOutcome};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

struct CountingAllocator;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(live, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn reset_allocations() {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    LIVE.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
}

fn allocation_report(name: &str, grid: &Grid<2>) {
    reset_allocations();
    let outcome = grid.find_path(
        GridCell::new2(2, 2),
        GridCell::new2(253, 253),
        SearchOptions::default(),
    );
    println!(
        "alloc {name:11} allocations={} bytes={} peak_live={} grid_bytes={}",
        ALLOCATIONS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
        grid.len() * (std::mem::size_of::<bool>() + std::mem::size_of::<f32>()),
    );
    black_box(outcome);
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
}

fn percentile(samples: &mut [Duration], fraction: f64) -> Duration {
    samples.sort_unstable();
    samples[((samples.len() - 1) as f64 * fraction).round() as usize]
}

fn run_case(name: &str, grid: &Grid<2>, start: GridCell<2>, goal: GridCell<2>, paths: usize) {
    let mut times = Vec::with_capacity(paths);
    let mut found = 0usize;
    let mut visited = 0usize;
    let mut nodes = 0usize;
    let mut checksum = 0.0f64;
    for _ in 0..paths {
        let started = Instant::now();
        let outcome = grid.find_path(start, goal, SearchOptions::default());
        times.push(started.elapsed());
        match outcome {
            SearchOutcome::Found(path) => {
                found += 1;
                nodes += path.nodes.len();
                visited += path.visited;
                checksum += path.cost;
            }
            SearchOutcome::NotFound { visited: count }
            | SearchOutcome::Cancelled { visited: count }
            | SearchOutcome::LimitReached { visited: count } => visited += count,
        }
    }
    println!(
        "{name:11} found={found}/{paths} mean_ms={:.3} p50_ms={:.3} p99_ms={:.3} mean_visited={:.1} mean_nodes={:.1} checksum={checksum:.1}",
        times
            .iter()
            .map(|time| time.as_secs_f64() * 1000.0)
            .sum::<f64>()
            / paths as f64,
        percentile(&mut times, 0.50).as_secs_f64() * 1000.0,
        percentile(&mut times, 0.99).as_secs_f64() * 1000.0,
        visited as f64 / paths as f64,
        if found == 0 {
            0.0
        } else {
            nodes as f64 / found as f64
        },
    );
    black_box(checksum);
}

fn random_grid(seed: u64, density_percent: u64) -> Grid<2> {
    let width = 256u32;
    let height = 256u32;
    let mut rng = Rng(seed);
    let mut grid = Grid::<2>::new([width, height]).unwrap();
    for y in 0..height {
        for x in 0..width {
            if rng.next() % 100 < density_percent {
                grid.set_blocked(GridCell::new2(x as i32, y as i32), true)
                    .unwrap();
            }
        }
    }
    grid
}

fn main() {
    let start = GridCell::new2(2, 2);
    let goal = GridCell::new2(253, 253);

    let easy = Grid::<2>::new([256, 256]).unwrap();
    run_case("easy", &easy, start, goal, 20);
    allocation_report("easy", &easy);
    let typical = random_grid(0x5eed, 12);
    run_case("typical", &typical, start, goal, 20);
    allocation_report("typical", &typical);
    let difficult = random_grid(0xbeef, 28);
    run_case("difficult", &difficult, start, goal, 20);

    let mut unreachable = Grid::<2>::new([256, 256]).unwrap();
    for y in 0..256 {
        unreachable
            .set_blocked(GridCell::new2(128, y), true)
            .unwrap();
    }
    run_case("unreachable", &unreachable, start, goal, 5);

    let started = Instant::now();
    let dijkstra = typical.find_path_dijkstra(start, goal, SearchOptions::default());
    let elapsed = started.elapsed();
    println!(
        "dijkstra   elapsed_ms={:.3} visited={:?} cost={:?}",
        elapsed.as_secs_f64() * 1000.0,
        match &dijkstra {
            SearchOutcome::Found(path) => Some(path.visited),
            _ => None,
        },
        dijkstra.path().map(|path| path.cost)
    );
}
