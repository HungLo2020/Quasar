//! Compile-time proof that the public API has upstream type identity.
#[test]
fn types_are_upstream_without_conversion() {
    fn native_device(device: &wgpu::Device) -> &vulkanic_gal::Device {
        device
    }
    fn native_queue(queue: &vulkanic_gal::Queue) -> &wgpu::Queue {
        queue
    }
    fn native_view(view: &wgpu::TextureView) -> &vulkanic_gal::TextureView {
        view
    }
    let _ = (native_device, native_queue, native_view);
    let descriptor: vulkanic_gal::BufferDescriptor<'_> = wgpu::BufferDescriptor {
        label: None,
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    };
    assert_eq!(descriptor.size, 16);
}
