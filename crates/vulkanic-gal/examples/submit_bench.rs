//! Native encoder/queue overhead. Use WGPU_BACKEND / WGPU_ADAPTER_NAME to select a device.
//! Renderer workloads and native presentation are benchmarked by quasar-composition.
use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::Instant,
};
use vulkanic_gal::*;
fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct Thread(std::thread::Thread);
    impl Wake for Thread {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Thread(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let instance = Instance::new(InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = block_on(util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))?;
    let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor::default()))?;
    println!(
        "device={} backend={:?}",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    let source = device.create_buffer(&BufferDescriptor {
        label: Some("copy source"),
        size: 16384,
        usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let target = device.create_buffer(&BufferDescriptor {
        label: Some("copy destination"),
        size: 16384,
        usage: BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    for count in [0, 1, 64, 1000] {
        let mut record = Vec::new();
        let mut submit = Vec::new();
        for frame in 0..364 {
            let start = Instant::now();
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
            for _ in 0..count {
                encoder.copy_buffer_to_buffer(&source, 0, &target, 0, 16384);
            }
            let command = encoder.finish();
            let recording = start.elapsed();
            let start = Instant::now();
            queue.submit([command]);
            let submission = start.elapsed();
            device.poll(PollType::wait_indefinitely())?;
            if frame >= 64 {
                record.push(recording.as_nanos());
                submit.push(submission.as_nanos());
            }
        }
        record.sort_unstable();
        submit.sort_unstable();
        println!(
            "copies={count} record_p50_us={:.3} submit_p50_us={:.3}",
            record[record.len() / 2] as f64 / 1000.,
            submit[submit.len() / 2] as f64 / 1000.
        );
    }
    Ok(())
}
