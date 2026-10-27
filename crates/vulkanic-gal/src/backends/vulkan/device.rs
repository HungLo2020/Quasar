use std::ffi::{CStr, CString};
use std::sync::Arc;

use ash::vk;

use crate::error::{GalError, GalResult};

use super::swapchain::SurfaceOwner;

pub(super) struct VulkanContext {
    /// Keeps the Vulkan loader library loaded while the instance lives.
    #[allow(dead_code)]
    pub(super) entry: ash::Entry,
    pub(super) instance: ash::Instance,
    pub(super) physical_device: vk::PhysicalDevice,
    pub(super) device: ash::Device,
    pub(super) debug_utils: Option<ash::ext::debug_utils::Device>,
    /// Per-frame command labels cost a format, an allocation and a driver
    /// call each; emit them only for debugging tools and validation runs.
    pub(super) command_labels: bool,
    pub(super) queue: vk::Queue,
    pub(super) memory_properties: vk::PhysicalDeviceMemoryProperties,
    pub(super) timestamp_period: f32,
    pub(super) timestamp_valid_bits: u32,
    pub(super) command_pool: vk::CommandPool,
    pub(super) timeline: vk::Semaphore,
    /// Native pipeline compilation results, shared by every pipeline the
    /// device creates and saved to `pipeline_cache_path` on drop.
    pub(super) pipeline_cache: vk::PipelineCache,
    pipeline_cache_path: Option<std::path::PathBuf>,
    /// Whether device creation enabled Vulkan's core multiDrawIndirect feature.
    /// Packed GAL indirect lists still work without it because lowering emits
    /// one native draw per record on that fallback path.
    pub(super) multi_draw_indirect: bool,
    /// Whether the optional pipeline-statistics query feature was enabled.
    /// The lowerer creates and reads these queries only when explicitly
    /// requested by the diagnostics environment variable.
    pub(super) pipeline_statistics_query: bool,
    /// Whether the device was created with Vulkan's core independent MRT
    /// blending feature. Source translucent passes rely on distinct primary
    /// and auxiliary attachment blend semantics; callers must keep that
    /// capability unavailable when the physical device cannot provide it.
    pub(super) independent_blend: bool,
    pub(super) provoking_vertex_last: bool,
    pub(super) provoking_vertex_per_pipeline: bool,
    /// DRAW visibility currently includes both vertex and fragment stages.
    /// A writable graphics storage binding requires both corresponding features.
    pub(super) graphics_storage_writes: bool,
    pub(super) surface_loader: Option<ash::khr::surface::Instance>,
    pub(super) surface: Option<vk::SurfaceKHR>,
    pub(super) swapchain_loader: Option<ash::khr::swapchain::Device>,
}

/// Text of a fixed-size, NUL-terminated Vulkan string field.
fn vk_string(raw: &[std::ffi::c_char]) -> String {
    let bytes: Vec<u8> = raw
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

impl VulkanContext {
    /// The physical device, its driver and the Vulkan version it supports.
    pub(super) fn device_info(&self) -> crate::create::DeviceInfo {
        use crate::create::{DeviceInfo, DeviceKind};
        let mut driver = vk::PhysicalDeviceDriverProperties::default();
        let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut driver);
        unsafe {
            self.instance
                .get_physical_device_properties2(self.physical_device, &mut properties);
        }
        let device = properties.properties;
        let version = device.api_version;
        let driver_name = vk_string(&driver.driver_name);
        let driver_info = vk_string(&driver.driver_info);
        DeviceInfo {
            name: vk_string(&device.device_name),
            driver: match (driver_name.is_empty(), driver_info.is_empty()) {
                (false, false) => format!("{driver_name} {driver_info}"),
                (false, true) => driver_name,
                _ => driver_info,
            },
            api: format!(
                "Vulkan {}.{}.{}",
                vk::api_version_major(version),
                vk::api_version_minor(version),
                vk::api_version_patch(version)
            ),
            kind: match device.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => DeviceKind::Discrete,
                vk::PhysicalDeviceType::INTEGRATED_GPU => DeviceKind::Integrated,
                vk::PhysicalDeviceType::VIRTUAL_GPU => DeviceKind::Virtual,
                vk::PhysicalDeviceType::CPU => DeviceKind::Cpu,
                _ => DeviceKind::Other,
            },
        }
    }

    pub(super) fn new(label: &str, validation: ValidationMode) -> GalResult<Arc<Self>> {
        Self::new_with_surface(label, validation, None)
    }

    pub(super) fn new_with_surface(
        label: &str,
        validation: ValidationMode,
        surface_owner: Option<&dyn SurfaceOwner>,
    ) -> GalResult<Arc<Self>> {
        let entry = unsafe { ash::Entry::load() }
            .map_err(|error| GalError::backend(format!("failed to load Vulkan entry: {error}")))?;
        let app_name = CString::new(label)
            .map_err(|_| GalError::backend("Vulkan application label contains NUL"))?;
        let engine_name = CString::new("VulkanicGAL").expect("static Vulkan engine name is valid");

        let available_layers = unsafe { entry.enumerate_instance_layer_properties() }
            .map_err(|error| GalError::backend(format!("failed to enumerate layers: {error:?}")))?;
        let available_extensions = unsafe { entry.enumerate_instance_extension_properties(None) }
            .map_err(|error| {
            GalError::backend(format!("failed to enumerate extensions: {error:?}"))
        })?;
        let enable_validation = validation != ValidationMode::Off
            && has_layer(&available_layers, "VK_LAYER_KHRONOS_validation");
        let validation_layer =
            CString::new("VK_LAYER_KHRONOS_validation").expect("static layer name is valid");
        let layer_names = if enable_validation {
            vec![validation_layer.as_ptr()]
        } else {
            Vec::new()
        };

        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(1)
            .engine_name(&engine_name)
            .engine_version(1)
            .api_version(vk::make_api_version(0, 1, 3, 0));
        let mut extension_names = Vec::new();
        let enable_debug_utils_extension =
            has_extension(&available_extensions, ash::ext::debug_utils::NAME);
        if enable_debug_utils_extension {
            extension_names.push(ash::ext::debug_utils::NAME.as_ptr());
        }
        if let Some(surface_owner) = surface_owner {
            for extension in surface_owner.required_instance_extensions() {
                if !has_extension(&available_extensions, extension) {
                    return Err(GalError::backend(format!(
                        "Vulkan presentation requires unavailable instance extension {}",
                        extension.to_string_lossy()
                    )));
                }
                extension_names.push(extension.as_ptr());
            }
        }
        let portability = has_extension(
            &available_extensions,
            ash::khr::portability_enumeration::NAME,
        );
        if portability {
            extension_names.push(ash::khr::portability_enumeration::NAME.as_ptr());
        }
        let validation_feature_enables = match validation {
            ValidationMode::Off => Vec::new(),
            ValidationMode::Routine => vec![
                vk::ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION,
                vk::ValidationFeatureEnableEXT::BEST_PRACTICES,
            ],
            ValidationMode::Deep => vec![
                vk::ValidationFeatureEnableEXT::GPU_ASSISTED,
                vk::ValidationFeatureEnableEXT::GPU_ASSISTED_RESERVE_BINDING_SLOT,
                vk::ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION,
                vk::ValidationFeatureEnableEXT::BEST_PRACTICES,
            ],
        };
        let mut validation_features = vk::ValidationFeaturesEXT::default()
            .enabled_validation_features(&validation_feature_enables);
        let mut instance_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_layer_names(&layer_names)
            .enabled_extension_names(&extension_names);
        if portability {
            instance_info = instance_info.flags(vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR);
        }
        if enable_validation && !validation_feature_enables.is_empty() {
            instance_info = instance_info.push_next(&mut validation_features);
        }
        let instance = unsafe { entry.create_instance(&instance_info, None) }.map_err(|error| {
            GalError::backend(format!("failed to create Vulkan instance: {error:?}"))
        })?;
        // From here on, a failure destroys what was created.
        let mut guard = CreationGuard::new(instance.clone());

        let surface_loader =
            surface_owner.map(|_| ash::khr::surface::Instance::new(&entry, &instance));
        let surface = match (surface_owner, surface_loader.as_ref()) {
            (Some(owner), Some(_)) => Some(owner.create_surface(&entry, &instance)?),
            _ => None,
        };
        if let (Some(loader), Some(surface)) = (&surface_loader, surface) {
            guard.surface = Some((loader.clone(), surface));
        }

        let physical_devices =
            unsafe { instance.enumerate_physical_devices() }.map_err(|error| {
                GalError::backend(format!(
                    "failed to enumerate Vulkan physical devices: {error:?}"
                ))
            })?;
        let (physical_device, queue_family_index) = select_graphics_device(
            &instance,
            &physical_devices,
            surface_loader.as_ref(),
            surface,
        )?;
        let memory_properties =
            unsafe { instance.get_physical_device_memory_properties(physical_device) };
        let physical_properties =
            unsafe { instance.get_physical_device_properties(physical_device) };

        let priority = [1.0_f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priority);
        let mut dynamic_rendering =
            vk::PhysicalDeviceDynamicRenderingFeatures::default().dynamic_rendering(true);
        let mut synchronization2 =
            vk::PhysicalDeviceSynchronization2Features::default().synchronization2(true);
        let mut timeline_features =
            vk::PhysicalDeviceTimelineSemaphoreFeatures::default().timeline_semaphore(true);
        // The shader compiler targets Vulkan 1.3 and may emit
        // DemoteToHelperInvocation for discard-compatible fragment paths. Query
        // and enable the promoted feature explicitly so validation and device
        // execution agree about that SPIR-V capability.
        let mut supported_demote =
            vk::PhysicalDeviceShaderDemoteToHelperInvocationFeatures::default();
        let extensions = unsafe { instance.enumerate_device_extension_properties(physical_device) }
            .map_err(|error| {
                GalError::backend(format!("failed to query device extensions: {error:?}"))
            })?;
        let provoking_extension = extensions.iter().any(|extension| unsafe {
            CStr::from_ptr(extension.extension_name.as_ptr()) == ash::ext::provoking_vertex::NAME
        });
        let mut provoking_properties = vk::PhysicalDeviceProvokingVertexPropertiesEXT::default();
        if provoking_extension {
            let mut properties =
                vk::PhysicalDeviceProperties2::default().push_next(&mut provoking_properties);
            unsafe {
                instance.get_physical_device_properties2(physical_device, &mut properties);
            }
        }
        let mut provoking_features = vk::PhysicalDeviceProvokingVertexFeaturesEXT::default();
        let mut supported_features =
            vk::PhysicalDeviceFeatures2::default().push_next(&mut supported_demote);
        if provoking_extension {
            supported_features = supported_features.push_next(&mut provoking_features);
        }
        unsafe {
            instance.get_physical_device_features2(physical_device, &mut supported_features);
        }
        // Terrain-translucent source passes use an explicit MRT blend
        // contract: the primary color target is alpha-blended while the
        // auxiliary targets are written without blending. Vulkan requires
        // `independentBlend` whenever color-attachment blend state differs,
        // so negotiate that core feature instead of silently creating an
        // invalid pipeline and relying on validation to catch it later.
        let independent_blend_supported = supported_features.features.independent_blend == vk::TRUE;
        let vertex_storage_writes = supported_features
            .features
            .vertex_pipeline_stores_and_atomics
            == vk::TRUE;
        let fragment_storage_writes =
            supported_features.features.fragment_stores_and_atomics == vk::TRUE;
        let multi_draw_indirect_supported =
            supported_features.features.multi_draw_indirect == vk::TRUE;
        // Multi-draw records address per-draw data with `firstInstance`
        // (terrain instance streams, DH draw records); Vulkan requires this
        // feature for a non-zero first instance in indirect commands.
        let draw_indirect_first_instance_supported =
            supported_features.features.draw_indirect_first_instance == vk::TRUE;
        let pipeline_statistics_supported =
            supported_features.features.pipeline_statistics_query == vk::TRUE;
        let provoking_vertex_last =
            provoking_extension && provoking_features.provoking_vertex_last == vk::TRUE;
        let core_features = vk::PhysicalDeviceFeatures::default()
            .independent_blend(independent_blend_supported)
            .multi_draw_indirect(multi_draw_indirect_supported)
            .draw_indirect_first_instance(draw_indirect_first_instance_supported)
            .pipeline_statistics_query(pipeline_statistics_supported)
            .vertex_pipeline_stores_and_atomics(vertex_storage_writes)
            .fragment_stores_and_atomics(fragment_storage_writes);
        if supported_demote.shader_demote_to_helper_invocation == vk::TRUE {
            supported_demote = supported_demote.shader_demote_to_helper_invocation(true);
        }
        let queue_infos = [queue_info];
        let mut device_extension_names = if surface.is_some() {
            vec![ash::khr::swapchain::NAME.as_ptr()]
        } else {
            Vec::new()
        };
        if has_extension(&extensions, ash::khr::portability_subset::NAME) {
            device_extension_names.push(ash::khr::portability_subset::NAME.as_ptr());
        }
        if provoking_vertex_last {
            device_extension_names.push(ash::ext::provoking_vertex::NAME.as_ptr());
        }
        let mut device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&device_extension_names)
            .enabled_features(&core_features)
            .push_next(&mut dynamic_rendering)
            .push_next(&mut synchronization2)
            .push_next(&mut timeline_features);
        if supported_demote.shader_demote_to_helper_invocation == vk::TRUE {
            device_info = device_info.push_next(&mut supported_demote);
        }
        let mut enabled_provoking =
            vk::PhysicalDeviceProvokingVertexFeaturesEXT::default().provoking_vertex_last(true);
        if provoking_vertex_last {
            device_info = device_info.push_next(&mut enabled_provoking);
        }
        let device = unsafe { instance.create_device(physical_device, &device_info, None) }
            .map_err(|error| {
                GalError::backend(format!("failed to create Vulkan device: {error:?}"))
            })?;
        guard.device = Some(device.clone());
        let debug_utils = if enable_debug_utils_extension {
            Some(ash::ext::debug_utils::Device::new(&instance, &device))
        } else {
            None
        };
        let swapchain_loader =
            surface.map(|_| ash::khr::swapchain::Device::new(&instance, &device));
        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
        let command_pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let command_pool = unsafe { device.create_command_pool(&command_pool_info, None) }
            .map_err(|error| {
                GalError::backend(format!("failed to create command pool: {error:?}"))
            })?;
        guard.command_pool = command_pool;
        let mut timeline_info = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let semaphore_info = vk::SemaphoreCreateInfo::default().push_next(&mut timeline_info);
        let timeline =
            unsafe { device.create_semaphore(&semaphore_info, None) }.map_err(|error| {
                GalError::backend(format!("failed to create timeline semaphore: {error:?}"))
            })?;
        guard.timeline = timeline;
        let pipeline_cache_path = pipeline_cache_path(&physical_properties);
        let pipeline_cache = create_pipeline_cache(&device, pipeline_cache_path.as_deref())?;
        let timestamp_valid_bits =
            queue_timestamp_valid_bits(&instance, physical_device, queue_family_index);

        let context = Arc::new(Self {
            entry,
            instance,
            physical_device,
            device,
            command_labels: debug_utils.is_some() && command_labels_requested(),
            debug_utils,
            queue,
            memory_properties,
            timestamp_period: physical_properties.limits.timestamp_period,
            timestamp_valid_bits,
            command_pool,
            timeline,
            pipeline_cache,
            pipeline_cache_path,
            multi_draw_indirect: multi_draw_indirect_supported,
            pipeline_statistics_query: pipeline_statistics_supported,
            independent_blend: independent_blend_supported,
            provoking_vertex_last,
            provoking_vertex_per_pipeline: provoking_properties.provoking_vertex_mode_per_pipeline
                == vk::TRUE,
            graphics_storage_writes: vertex_storage_writes && fragment_storage_writes,
            surface_loader,
            surface,
            swapchain_loader,
        });
        guard.disarm();
        context.set_object_name(context.timeline, "gal.timeline.graphics");
        Ok(context)
    }

    pub(super) fn allocate_memory(
        &self,
        requirements: vk::MemoryRequirements,
        properties: vk::MemoryPropertyFlags,
    ) -> GalResult<vk::DeviceMemory> {
        let memory_type_index = self.find_memory_type(requirements.memory_type_bits, properties)?;
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index);
        unsafe { self.device.allocate_memory(&alloc_info, None) }.map_err(|error| {
            GalError::backend(format!("failed to allocate Vulkan memory: {error:?}"))
        })
    }

    /// The first memory type in `type_bits` with every `required` flag.
    /// `HOST_CACHED` is a preference: without a cached type, an uncached
    /// one with the other flags is chosen.
    pub(super) fn find_memory_type(
        &self,
        type_bits: u32,
        required: vk::MemoryPropertyFlags,
    ) -> GalResult<u32> {
        let cached = vk::MemoryPropertyFlags::HOST_CACHED;
        if required.contains(cached) {
            return self
                .find_exact_memory_type(type_bits, required)
                .or_else(|_| self.find_exact_memory_type(type_bits, required & !cached));
        }
        self.find_exact_memory_type(type_bits, required)
    }

    fn find_exact_memory_type(
        &self,
        type_bits: u32,
        required: vk::MemoryPropertyFlags,
    ) -> GalResult<u32> {
        for index in 0..self.memory_properties.memory_type_count {
            let mask = 1_u32 << index;
            let properties = self.memory_properties.memory_types[index as usize].property_flags;
            if type_bits & mask != 0 && properties.contains(required) {
                return Ok(index);
            }
        }
        Err(GalError::backend(format!(
            "no Vulkan memory type matches bits=0x{type_bits:x} required=0x{:x}",
            required.as_raw()
        )))
    }

    pub(super) fn wait_idle(&self) -> GalResult<()> {
        unsafe { self.device.device_wait_idle() }.map_err(|error| {
            GalError::backend(format!("Vulkan device wait idle failed: {error:?}"))
        })
    }

    pub(super) fn read_mapped_memory(
        &self,
        memory: vk::DeviceMemory,
        offset: u64,
        size: u64,
    ) -> GalResult<Vec<u8>> {
        let offset = usize::try_from(offset)
            .map_err(|_| GalError::backend("Vulkan readback offset does not fit usize"))?;
        let size = usize::try_from(size)
            .map_err(|_| GalError::backend("Vulkan readback size does not fit usize"))?;
        if size == 0 {
            return Ok(Vec::new());
        }
        // Map only the bytes read: the page holds other buffers too.
        let ptr = unsafe {
            self.device.map_memory(
                memory,
                offset as u64,
                size as u64,
                vk::MemoryMapFlags::empty(),
            )
        }
        .map_err(|error| {
            GalError::backend(format!("failed to map Vulkan readback memory: {error:?}"))
        })?;
        // SAFETY: the mapping covers exactly `size` bytes from `ptr`, and the
        // memory stays mapped until the copy below completes.
        let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), size).to_vec() };
        unsafe { self.device.unmap_memory(memory) };
        Ok(bytes)
    }

    pub(super) fn set_object_name<T: vk::Handle>(&self, object: T, name: &str) {
        let Some(debug_utils) = &self.debug_utils else {
            return;
        };
        let Ok(name) = CString::new(name) else {
            return;
        };
        let info = vk::DebugUtilsObjectNameInfoEXT::default()
            .object_handle(object)
            .object_name(&name);
        let _ = unsafe { debug_utils.set_debug_utils_object_name(&info) };
    }

    pub(super) unsafe fn begin_label(&self, command_buffer: vk::CommandBuffer, name: &str) {
        let Some(debug_utils) = self.debug_utils.as_ref().filter(|_| self.command_labels) else {
            return;
        };
        let Ok(name) = CString::new(name) else {
            return;
        };
        let label = vk::DebugUtilsLabelEXT::default()
            .label_name(&name)
            .color([0.15, 0.45, 0.95, 1.0]);
        unsafe { debug_utils.cmd_begin_debug_utils_label(command_buffer, &label) };
    }

    pub(super) unsafe fn end_label(&self, command_buffer: vk::CommandBuffer) {
        if let Some(debug_utils) = self.debug_utils.as_ref().filter(|_| self.command_labels) {
            unsafe { debug_utils.cmd_end_debug_utils_label(command_buffer) };
        }
    }
}

/// Command labels are for RenderDoc/Nsight captures and validation runs:
/// `VULKANIC_VULKAN_DEBUG_LABELS=1`, RenderDoc capture (ours or RenderDoc's
/// own launcher variable) or GAL validation enables them.
fn command_labels_requested() -> bool {
    let set = |name: &str| {
        crate::env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0")
    };
    set("VULKANIC_VULKAN_DEBUG_LABELS")
        || set("VULKANIC_RENDERDOC_CAPTURE")
        || set("ENABLE_VULKAN_RENDERDOC_CAPTURE")
        || crate::gal::per_frame_validation()
}

fn queue_timestamp_valid_bits(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    queue_family_index: u32,
) -> u32 {
    unsafe { instance.get_physical_device_queue_family_properties(physical_device) }
        .get(queue_family_index as usize)
        .map(|family| family.timestamp_valid_bits)
        .unwrap_or(0)
}

/// Destroys the objects of a partly created context when creation fails.
struct CreationGuard {
    instance: Option<ash::Instance>,
    surface: Option<(ash::khr::surface::Instance, vk::SurfaceKHR)>,
    device: Option<ash::Device>,
    command_pool: vk::CommandPool,
    timeline: vk::Semaphore,
}

impl CreationGuard {
    fn new(instance: ash::Instance) -> Self {
        Self {
            instance: Some(instance),
            surface: None,
            device: None,
            command_pool: vk::CommandPool::null(),
            timeline: vk::Semaphore::null(),
        }
    }

    /// Creation succeeded; the context owns everything now.
    fn disarm(mut self) {
        self.instance = None;
        self.surface = None;
        self.device = None;
    }
}

impl Drop for CreationGuard {
    fn drop(&mut self) {
        // SAFETY: each object was created by this context's creation, is
        // unused (creation failed before anything was submitted), and is
        // destroyed before the object that created it.
        unsafe {
            if let Some(device) = &self.device {
                if self.timeline != vk::Semaphore::null() {
                    device.destroy_semaphore(self.timeline, None);
                }
                if self.command_pool != vk::CommandPool::null() {
                    device.destroy_command_pool(self.command_pool, None);
                }
                device.destroy_device(None);
            }
            if let Some((loader, surface)) = &self.surface {
                loader.destroy_surface(*surface, None);
            }
            if let Some(instance) = &self.instance {
                instance.destroy_instance(None);
            }
        }
    }
}

/// Where the pipeline cache persists: beside the SPIR-V cache, per device
/// (the driver also rejects data from another device or driver version).
fn pipeline_cache_path(properties: &vk::PhysicalDeviceProperties) -> Option<std::path::PathBuf> {
    let dir = crate::backends::spirv::disk_cache::cache_dir()?;
    let uuid: String = properties
        .pipeline_cache_uuid
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(dir.join(format!(
        "vulkan-pipelines-{:04x}-{:04x}-{uuid}.bin",
        properties.vendor_id, properties.device_id
    )))
}

fn create_pipeline_cache(
    device: &ash::Device,
    path: Option<&std::path::Path>,
) -> GalResult<vk::PipelineCache> {
    let initial = path
        .and_then(|path| std::fs::read(path).ok())
        .unwrap_or_default();
    let create = |data: &[u8]| unsafe {
        device.create_pipeline_cache(
            &vk::PipelineCacheCreateInfo::default().initial_data(data),
            None,
        )
    };
    // A driver may refuse stale or corrupt data instead of ignoring it.
    create(&initial)
        .or_else(|_| create(&[]))
        .map_err(|error| GalError::backend(format!("failed to create pipeline cache: {error:?}")))
}

impl Drop for VulkanContext {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            if let Some(path) = &self.pipeline_cache_path {
                if let Ok(data) = self.device.get_pipeline_cache_data(self.pipeline_cache) {
                    // Write then rename, so a crash never leaves a torn file.
                    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
                    if std::fs::write(&temporary, &data).is_ok() {
                        let _ = std::fs::rename(&temporary, path);
                    }
                }
            }
            self.device
                .destroy_pipeline_cache(self.pipeline_cache, None);
            self.device.destroy_semaphore(self.timeline, None);
            self.device.destroy_command_pool(self.command_pool, None);
            self.device.destroy_device(None);
            if let (Some(surface_loader), Some(surface)) = (&self.surface_loader, self.surface) {
                surface_loader.destroy_surface(surface, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ValidationMode {
    Off,
    Routine,
    Deep,
}

impl ValidationMode {
    pub(super) fn from_env() -> Self {
        match crate::env::var("VULKANIC_VULKAN_VALIDATE") {
            Ok(value) if value.eq_ignore_ascii_case("deep") => Self::Deep,
            Ok(value)
                if value.eq_ignore_ascii_case("1")
                    || value.eq_ignore_ascii_case("true")
                    || value.eq_ignore_ascii_case("routine") =>
            {
                Self::Routine
            }
            _ => Self::Off,
        }
    }
}

fn has_layer(layers: &[vk::LayerProperties], wanted: &str) -> bool {
    layers.iter().any(|layer| {
        let name = unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) };
        name.to_string_lossy() == wanted
    })
}

fn has_extension(extensions: &[vk::ExtensionProperties], wanted: &CStr) -> bool {
    extensions.iter().any(|extension| {
        let name = unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) };
        name == wanted
    })
}

fn select_graphics_device(
    instance: &ash::Instance,
    physical_devices: &[vk::PhysicalDevice],
    surface_loader: Option<&ash::khr::surface::Instance>,
    surface: Option<vk::SurfaceKHR>,
) -> GalResult<(vk::PhysicalDevice, u32)> {
    let mut best = None;
    // One line per windowed device selection, so a recording or log shows
    // which GPU rendered and why (e.g. a discrete GPU that cannot present).
    let mut candidates = Vec::new();
    for physical_device in physical_devices {
        let properties = unsafe { instance.get_physical_device_properties(*physical_device) };
        let name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let queue_families =
            unsafe { instance.get_physical_device_queue_family_properties(*physical_device) };
        let Some((queue_family_index, _)) =
            queue_families.iter().enumerate().find(|(index, family)| {
                if !family.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                    return false;
                }
                let (Some(surface_loader), Some(surface)) = (surface_loader, surface) else {
                    return true;
                };
                unsafe {
                    surface_loader
                        .get_physical_device_surface_support(
                            *physical_device,
                            *index as u32,
                            surface,
                        )
                        .unwrap_or(false)
                }
            })
        else {
            candidates.push((*physical_device, name, properties.device_type, false, -1));
            continue;
        };
        let score = match properties.device_type {
            vk::PhysicalDeviceType::DISCRETE_GPU => 3,
            vk::PhysicalDeviceType::INTEGRATED_GPU => 2,
            vk::PhysicalDeviceType::CPU => 1,
            _ => 0,
        };
        candidates.push((*physical_device, name, properties.device_type, true, score));
        if best
            .map(|(_, _, best_score)| score > best_score)
            .unwrap_or(true)
        {
            best = Some((*physical_device, queue_family_index as u32, score));
        }
    }
    if surface.is_some() {
        let selected = best.map(|(device, _, _)| device);
        let listed: Vec<String> = candidates
            .iter()
            .map(|(device, name, kind, usable, score)| {
                let kind = match *kind {
                    vk::PhysicalDeviceType::DISCRETE_GPU => "discrete",
                    vk::PhysicalDeviceType::INTEGRATED_GPU => "integrated",
                    vk::PhysicalDeviceType::VIRTUAL_GPU => "virtual",
                    vk::PhysicalDeviceType::CPU => "cpu",
                    _ => "other",
                };
                format!(
                    "[{name} type={kind} graphics_present={usable} score={score}{}]",
                    if Some(*device) == selected {
                        " SELECTED"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        eprintln!("VulkanicGAL device selection: {}", listed.join(" "));
    }
    best.map(|(device, queue, _)| (device, queue))
        .ok_or_else(|| {
            if surface.is_some() {
                GalError::backend("no Vulkan physical device exposes a graphics+present queue")
            } else {
                GalError::backend("no Vulkan physical device exposes a graphics queue")
            }
        })
}
