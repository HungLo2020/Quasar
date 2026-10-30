use quasar_frame::*;
use vulkanic_gal::*;
fn clear(gpu: &Gpu, view: &TextureView, color: Color) -> CommandBuffer {
    let mut encoder = gpu.encoder("native clear");
    {
        let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(color),
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    encoder.finish()
}
#[test]
#[ignore = "native GPU aligned readback and independent devices"]
fn native_devices_coexist_and_readback_handles_unaligned_rows_and_bgra() {
    let native = Gpu::headless().unwrap();
    let a = Gpu::from_parts(native.instance, native.adapter, native.device, native.queue);
    let b = Gpu::headless().unwrap();
    for gpu in [&a, &b] {
        for format in [TextureFormat::Rgba8Unorm, TextureFormat::Bgra8Unorm] {
            let texture = texture_2d(
                &gpu.device,
                "odd-width target",
                3,
                5,
                format,
                TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            )
            .unwrap();
            let view = texture.create_view(&Default::default());
            gpu.queue.submit([clear(
                gpu,
                &view,
                Color {
                    r: 1.,
                    g: 0.5,
                    b: 0.,
                    a: 1.,
                },
            )]);
            let rgba = read_rgba(gpu, &texture).unwrap();
            for pixel in rgba.chunks_exact(4) {
                assert_eq!([pixel[0], pixel[2], pixel[3]], [255, 0, 255]);
                assert!(
                    pixel[1].abs_diff(128) <= 1,
                    "UNORM clear rounding: {pixel:?}"
                );
            }
        }
    }
    assert!(OffscreenTarget::new(&a, 0, 8).is_err());
    assert!(OffscreenTarget::new(&a, u32::MAX, 8).is_err());
    let unsampled = texture_2d(
        &a.device,
        "not readable",
        4,
        4,
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING,
    )
    .unwrap();
    assert!(read_rgba(&a, &unsampled).is_err());
}
#[test]
#[ignore = "native GPU timestamp query and validation errors"]
fn timestamps_remain_frame_correlated_without_polling_waits() {
    let gpu = Gpu::headless().unwrap();
    gpu.set_gpu_timestamps(true);
    let target = OffscreenTarget::new(&gpu, 16, 16).unwrap();
    let mut pending = Vec::new();
    for _ in 0..8 {
        let (_, report) = record_and_submit(&gpu, |gpu| {
            Ok(vec![clear(gpu, &target.target.color, Color::BLUE)])
        })
        .unwrap();
        assert_eq!(report.blocking_waits, 0);
        pending.push(report);
    }
    gpu.wait().unwrap();
    let supported = gpu
        .device
        .features()
        .contains(Features::TIMESTAMP_QUERY | Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
    for report in pending {
        assert_eq!(report.gpu_timing.is_some(), supported);
        if let Some(timing) = report.gpu_timing {
            assert!(timing.nanos().is_some());
        }
    }
    let error = validation_scope(&gpu, || {
        gpu.device.create_buffer(&BufferDescriptor {
            label: Some("invalid usage"),
            size: 4,
            usage: BufferUsages::empty(),
            mapped_at_creation: false,
        })
    });
    assert!(error.is_err());
    // A scoped validation failure leaves the native device usable.
    assert_eq!(
        target
            .render(&gpu, |gpu, target| Ok(vec![clear(
                gpu,
                &target.color,
                Color::BLUE
            )]))
            .unwrap()
            .rgba,
        [0, 0, 255, 255].repeat(256)
    );
}
