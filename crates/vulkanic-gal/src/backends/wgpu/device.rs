//! The wgpu instance, adapter, device and queue a backend owns, the native
//! features and limits it negotiated, and error capture: wgpu reports
//! validation failures through error scopes or an uncaptured-error handler
//! that would otherwise panic, so every fallible call is scoped and the
//! handler records instead of aborting.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use crate::create::{DeviceInfo, DeviceKind, GraphicsBackend, WgpuApi};
use crate::error::{GalError, GalResult};

/// Native features used when the adapter offers them; capabilities report
/// what was actually granted.
const OPTIONAL_FEATURES: wgpu::Features = wgpu::Features::FLOAT32_FILTERABLE
    .union(wgpu::Features::INDIRECT_FIRST_INSTANCE)
    .union(wgpu::Features::RG11B10UFLOAT_RENDERABLE)
    .union(wgpu::Features::BGRA8UNORM_STORAGE)
    .union(wgpu::Features::VERTEX_WRITABLE_STORAGE)
    .union(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);

/// Drives a wgpu future to completion. Native wgpu-core futures are ready
/// on first poll; the loop only covers backends that resolve later.
pub(super) fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}

pub(super) struct WgpuContext {
    /// Kept for the lifetime of the adapter and surfaces created from it.
    _instance: wgpu::Instance,
    pub(super) adapter: wgpu::Adapter,
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
    pub(super) features: wgpu::Features,
    pub(super) limits: wgpu::Limits,
    pub(super) downlevel: wgpu::DownlevelCapabilities,
    uncaptured: Arc<Mutex<Vec<String>>>,
}

/// An instance limited to `api`. Only `WgpuApi::Auto` lets `WGPU_BACKEND`
/// choose; `WGPU_DEBUG`, `WGPU_VALIDATION` and friends always apply.
pub(super) fn new_instance(api: WgpuApi) -> wgpu::Instance {
    let backends = match api {
        WgpuApi::Auto => wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY),
        WgpuApi::Vulkan => wgpu::Backends::VULKAN,
        WgpuApi::Metal => wgpu::Backends::METAL,
        WgpuApi::Dx12 => wgpu::Backends::DX12,
    };
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        flags: wgpu::InstanceFlags::from_build_config().with_env(),
        memory_budget_thresholds: Default::default(),
        backend_options: wgpu::BackendOptions::from_env_or_default(),
        // Only wgpu's GL backend needs it, and that backend is not enabled.
        display: None,
    })
}

impl WgpuContext {
    /// Picks an adapter (honouring `WGPU_ADAPTER_NAME` and
    /// `WGPU_POWER_PREF`), optionally compatible with `surface`, and opens
    /// its device with the optional features it supports.
    pub(super) fn new(
        label: &str,
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> GalResult<Self> {
        let adapter = block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, surface,
        ))
        .map_err(|error| GalError::backend(format!("no wgpu adapter is available: {error}")))?;
        let features = adapter.features() & OPTIONAL_FEATURES;
        let limits = adapter.limits();
        let downlevel = adapter.get_downlevel_capabilities();
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some(label),
            required_features: features,
            required_limits: limits.clone(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|error| GalError::backend(format!("wgpu device creation failed: {error}")))?;
        let uncaptured = Arc::new(Mutex::new(Vec::new()));
        let sink = uncaptured.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            if let Ok(mut errors) = sink.lock() {
                errors.push(error.to_string());
            }
        }));
        Ok(Self {
            _instance: instance,
            adapter,
            device,
            queue,
            features,
            limits,
            downlevel,
            uncaptured,
        })
    }

    /// Runs `operation` inside validation and out-of-memory error scopes and
    /// turns a reported error into a `GalError`.
    pub(super) fn scoped<T>(&self, what: &str, operation: impl FnOnce() -> T) -> GalResult<T> {
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let value = operation();
        let validation = block_on(validation.pop());
        let memory = block_on(memory.pop());
        match validation.or(memory) {
            Some(error) => Err(GalError::backend(format!("wgpu {what} failed: {error}"))),
            None => Ok(value),
        }
    }

    /// Errors reported outside any scope (for example by queue submission)
    /// since the last call.
    pub(super) fn take_uncaptured_errors(&self) -> Vec<String> {
        self.uncaptured
            .lock()
            .map(|mut errors| std::mem::take(&mut *errors))
            .unwrap_or_default()
    }

    /// The native API wgpu runs on.
    pub(super) fn selected_backend(&self) -> GraphicsBackend {
        GraphicsBackend::Wgpu(match self.adapter.get_info().backend {
            wgpu::Backend::Vulkan => WgpuApi::Vulkan,
            wgpu::Backend::Metal => WgpuApi::Metal,
            wgpu::Backend::Dx12 => WgpuApi::Dx12,
            _ => WgpuApi::Auto,
        })
    }

    /// The adapter and driver wgpu selected.
    pub(super) fn device_info(&self) -> DeviceInfo {
        let info = self.adapter.get_info();
        let api = match info.backend {
            wgpu::Backend::Vulkan => "Vulkan",
            wgpu::Backend::Metal => "Metal",
            wgpu::Backend::Dx12 => "Direct3D 12",
            wgpu::Backend::Gl => "OpenGL",
            wgpu::Backend::BrowserWebGpu => "WebGPU",
            _ => "unknown API",
        };
        DeviceInfo {
            name: info.name,
            driver: [info.driver, info.driver_info]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            api: format!("{api} through wgpu"),
            kind: match info.device_type {
                wgpu::DeviceType::DiscreteGpu => DeviceKind::Discrete,
                wgpu::DeviceType::IntegratedGpu => DeviceKind::Integrated,
                wgpu::DeviceType::VirtualGpu => DeviceKind::Virtual,
                wgpu::DeviceType::Cpu => DeviceKind::Cpu,
                wgpu::DeviceType::Other => DeviceKind::Other,
            },
        }
    }

    #[cfg(test)]
    pub(super) fn adapter_name(&self) -> String {
        let info = self.adapter.get_info();
        format!("{} ({:?})", info.name, info.backend)
    }
}
