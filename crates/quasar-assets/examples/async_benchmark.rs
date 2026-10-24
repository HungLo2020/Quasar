//! Small local sync/async load probe; not a statistically rigorous benchmark.
use quasar_assets::{AssetServer, AsyncAssetServer, AsyncConfig, BytesLoader, MemorySource};
use std::{sync::Arc, time::Instant};

const ASSETS: usize = 2_000;

fn server() -> AssetServer {
    let mut source = MemorySource::new("bench");
    for index in 0..ASSETS {
        source
            .insert(format!("asset-{index}.bin"), vec![0_u8; 256])
            .unwrap();
    }
    let mut server = AssetServer::new();
    server.register(BytesLoader);
    server.mount("", Arc::new(source)).unwrap();
    server
}

fn main() {
    let mut sync_server = server();
    let start = Instant::now();
    for index in 0..ASSETS {
        let handle = sync_server
            .load::<BytesLoader>(format!("asset-{index}.bin"))
            .unwrap();
        std::hint::black_box(handle.len());
    }
    let sync = start.elapsed();

    let async_server = AsyncAssetServer::new(
        server(),
        AsyncConfig {
            queue_capacity: ASSETS + 1,
        },
    )
    .unwrap();
    let start = Instant::now();
    let mut requests = Vec::with_capacity(ASSETS);
    for index in 0..ASSETS {
        requests.push(
            async_server
                .load::<BytesLoader>(format!("asset-{index}.bin"))
                .unwrap(),
        );
    }
    for request in &mut requests {
        let loaded = request
            .wait(std::time::Duration::from_secs(30))
            .expect("asset")
            .expect("load");
        std::hint::black_box(loaded.handle.len());
    }
    let async_time = start.elapsed();
    println!(
        "assets={ASSETS} sync_ms={:.3} async_ms={:.3} sync_us_per_asset={:.3} async_us_per_asset={:.3}",
        sync.as_secs_f64() * 1000.0,
        async_time.as_secs_f64() * 1000.0,
        sync.as_secs_f64() * 1_000_000.0 / ASSETS as f64,
        async_time.as_secs_f64() * 1_000_000.0 / ASSETS as f64
    );
}
