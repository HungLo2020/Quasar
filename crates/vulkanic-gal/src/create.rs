//! Creating a GAL: the one public place a backend is chosen.
//!
//! The composition root (the game, or an engine layer such as Quasar's
//! renderers) picks a backend once and gets back a `VulkanicGal`. Backend
//! types stay private to this crate, and renderers branch on capabilities
//! (`BackendCapabilities`), never on which backend is running.
//!
//! Games choose through `GalOptions`: a `GraphicsBackend` (`Auto` when they do
//! not care, native Vulkan first), whether a failed explicit choice falls back
//! to automatic selection, and whether the `VULKANIC_BACKEND` developer
//! override may replace an explicit choice. `VulkanicGal::backend_info`
//! reports what was requested and what actually runs, for debug screens and
//! performance reports. The `BackendChoice` constructors keep their original
//! behavior, including the override.

#[cfg(feature = "opengl")]
use super::backends::create_borrowed_opengl_backend;
#[cfg(feature = "vulkan")]
use super::backends::{self, create_native_windowed_vulkan_backend};
use super::backends::{create_backend, Backend};
use super::error::{GalError, GalResult};
#[cfg(any(feature = "vulkan", feature = "wgpu"))]
use super::frame::FrameSurfaceDesc;
use super::gal::VulkanicGal;

/// The graphics API a new GAL runs on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendChoice {
    /// The Vulkan backend; requires the `vulkan` feature.
    Vulkan,
    /// The OpenGL backend; requires the `opengl` feature.
    OpenGl,
    /// The wgpu backend (Vulkan, Metal or Direct3D 12 through wgpu);
    /// requires the `wgpu` feature.
    Wgpu,
}

impl BackendChoice {
    /// The backend `VULKANIC_BACKEND` names, if it is set and not empty.
    /// wgpu variants (`wgpu-vulkan`, ...) map to `Wgpu`; use
    /// `GraphicsBackend::from_env` to keep the native API.
    pub fn from_env() -> GalResult<Option<Self>> {
        Ok(GraphicsBackend::from_env()?.and_then(GraphicsBackend::choice))
    }
}

/// The native API wgpu runs on.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum WgpuApi {
    /// wgpu's choice (`WGPU_BACKEND` applies).
    Auto,
    /// Vulkan.
    Vulkan,
    /// Metal (Apple platforms).
    Metal,
    /// Direct3D 12 (Windows).
    Dx12,
}

/// A graphics backend as games, settings files and menus name it.
///
/// `Auto` means the game does not care: native Vulkan first, then wgpu (and
/// OpenGL for headless GALs), skipping backends this build lacks. Store the
/// choice with `id()` and read it back with `parse`/`FromStr`; show it with
/// `display_name()`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GraphicsBackend {
    /// Automatic selection; native Vulkan when available.
    #[default]
    Auto,
    /// VulkanicGAL's native Vulkan backend.
    Vulkan,
    /// VulkanicGAL's native OpenGL backend (headless or borrowed contexts;
    /// it does not create window surfaces).
    OpenGl,
    /// VulkanicGAL on wgpu, on the given native API.
    Wgpu(WgpuApi),
}

impl GraphicsBackend {
    const ALL: [Self; 7] = [
        Self::Auto,
        Self::Vulkan,
        Self::OpenGl,
        Self::Wgpu(WgpuApi::Auto),
        Self::Wgpu(WgpuApi::Vulkan),
        Self::Wgpu(WgpuApi::Metal),
        Self::Wgpu(WgpuApi::Dx12),
    ];

    /// A stable id for settings files and `VULKANIC_BACKEND`.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Vulkan => "vulkan",
            Self::OpenGl => "opengl",
            Self::Wgpu(WgpuApi::Auto) => "wgpu",
            Self::Wgpu(WgpuApi::Vulkan) => "wgpu-vulkan",
            Self::Wgpu(WgpuApi::Metal) => "wgpu-metal",
            Self::Wgpu(WgpuApi::Dx12) => "wgpu-dx12",
        }
    }

    /// A short name for menus and debug screens.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Vulkan => "Vulkan",
            Self::OpenGl => "OpenGL",
            Self::Wgpu(WgpuApi::Auto) => "wgpu",
            Self::Wgpu(WgpuApi::Vulkan) => "wgpu / Vulkan",
            Self::Wgpu(WgpuApi::Metal) => "wgpu / Metal",
            Self::Wgpu(WgpuApi::Dx12) => "wgpu / Direct3D 12",
        }
    }

    /// The backend `id` names (case-insensitive; `gl` is accepted for
    /// OpenGL).
    pub fn parse(id: &str) -> Option<Self> {
        let id = id.trim().to_ascii_lowercase();
        if id == "gl" {
            return Some(Self::OpenGl);
        }
        Self::ALL.into_iter().find(|backend| backend.id() == id)
    }

    /// The backend `VULKANIC_BACKEND` names, if it is set and not empty.
    pub fn from_env() -> GalResult<Option<Self>> {
        match crate::env::var("VULKANIC_BACKEND") {
            Ok(value) if value.trim().is_empty() => Ok(None),
            Ok(value) => Self::parse(&value).map(Some).ok_or_else(|| {
                GalError::invalid_argument(format!(
                    "VULKANIC_BACKEND={value} is not one of {}",
                    Self::ALL.map(Self::id).join(", ")
                ))
            }),
            Err(_) => Ok(None),
        }
    }

    /// Whether this build (features and target platform) contains it.
    /// `Auto` is always available.
    pub const fn is_compiled(self) -> bool {
        match self {
            Self::Auto => true,
            Self::Vulkan => cfg!(feature = "vulkan"),
            Self::OpenGl => cfg!(feature = "opengl"),
            Self::Wgpu(WgpuApi::Auto) => cfg!(feature = "wgpu"),
            Self::Wgpu(WgpuApi::Vulkan) => cfg!(all(
                feature = "wgpu",
                any(
                    windows,
                    target_os = "linux",
                    target_os = "android",
                    target_os = "freebsd"
                )
            )),
            Self::Wgpu(WgpuApi::Metal) => cfg!(all(feature = "wgpu", target_vendor = "apple")),
            Self::Wgpu(WgpuApi::Dx12) => cfg!(all(feature = "wgpu", windows)),
        }
    }

    /// The choices a settings menu can offer in this build, `Auto` first.
    /// Windowed GALs exclude OpenGL, which only borrows host contexts.
    pub fn selectable(windowed: bool) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|backend| backend.is_compiled())
            .filter(|backend| !(windowed && *backend == Self::OpenGl))
            .collect()
    }

    /// The concrete backends `Auto` tries, in order.
    fn automatic_order(windowed: bool) -> Vec<Self> {
        let mut order = vec![Self::Vulkan, Self::Wgpu(WgpuApi::Auto)];
        if !windowed {
            order.push(Self::OpenGl);
        }
        order.retain(|backend| backend.is_compiled());
        order
    }

    /// The `BackendChoice` family, if this is a concrete backend.
    fn choice(self) -> Option<BackendChoice> {
        match self {
            Self::Auto => None,
            Self::Vulkan => Some(BackendChoice::Vulkan),
            Self::OpenGl => Some(BackendChoice::OpenGl),
            Self::Wgpu(_) => Some(BackendChoice::Wgpu),
        }
    }
}

impl From<BackendChoice> for GraphicsBackend {
    fn from(choice: BackendChoice) -> Self {
        match choice {
            BackendChoice::Vulkan => Self::Vulkan,
            BackendChoice::OpenGl => Self::OpenGl,
            BackendChoice::Wgpu => Self::Wgpu(WgpuApi::Auto),
        }
    }
}

impl std::str::FromStr for GraphicsBackend {
    type Err = GalError;

    fn from_str(id: &str) -> GalResult<Self> {
        Self::parse(id)
            .ok_or_else(|| GalError::invalid_argument(format!("unknown graphics backend '{id}'")))
    }
}

impl std::fmt::Display for GraphicsBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

/// How a game asks for its GAL's backend.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GalOptions {
    /// The requested backend; `Auto` (the default) when the game does not
    /// care.
    pub backend: GraphicsBackend,
    /// When an explicit backend cannot start, fall back to automatic
    /// selection instead of failing. `backend_info` reports the fallback.
    pub fallback_to_auto: bool,
    /// Let `VULKANIC_BACKEND` replace an explicit backend. It always applies
    /// to `Auto`.
    pub allow_env_override: bool,
}

impl GalOptions {
    /// Options requesting `backend`, with no fallback or override.
    pub const fn new(backend: GraphicsBackend) -> Self {
        Self {
            backend,
            fallback_to_auto: false,
            allow_env_override: false,
        }
    }

    /// These options, falling back to automatic selection on failure.
    pub const fn with_fallback(mut self) -> Self {
        self.fallback_to_auto = true;
        self
    }

    /// The `BackendChoice` constructors' behavior: the developer override
    /// applies.
    fn legacy(choice: BackendChoice) -> Self {
        Self {
            backend: choice.into(),
            fallback_to_auto: false,
            allow_env_override: true,
        }
    }
}

/// The kind of device a GAL runs on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeviceKind {
    /// A discrete GPU.
    Discrete,
    /// A GPU integrated with the CPU.
    Integrated,
    /// A virtualized GPU.
    Virtual,
    /// A software rasterizer.
    Cpu,
    /// Unknown or not reported.
    #[default]
    Other,
}

/// The device and driver a GAL runs on, as its native API reports them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceInfo {
    /// The adapter or renderer name, e.g. "NVIDIA GeForce RTX 2070".
    pub name: String,
    /// The driver name and version, when the API reports them.
    pub driver: String,
    /// The native API and its version, e.g. "Vulkan 1.4.312".
    pub api: String,
    /// The device kind.
    pub kind: DeviceKind,
}

/// Why the selected backend differs from the requested one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionNote {
    /// `VULKANIC_BACKEND` replaced the requested backend.
    EnvironmentOverride {
        /// The backend the variable named.
        backend: GraphicsBackend,
    },
    /// The requested backend failed and automatic selection was used.
    Fallback {
        /// The backend that failed.
        from: GraphicsBackend,
        /// Its creation error.
        reason: String,
    },
}

/// What a GAL runs on: the request, the backend actually selected, and the
/// device. `Display` gives one line for a debug screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendInfo {
    /// What the game asked for.
    pub requested: GraphicsBackend,
    /// The backend running, never `Auto`; for wgpu, with the native API wgpu
    /// selected.
    pub selected: GraphicsBackend,
    /// Overrides and fallbacks between request and selection, in order.
    pub notes: Vec<SelectionNote>,
    /// The device and driver.
    pub device: DeviceInfo,
}

impl std::fmt::Display for BackendInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} | {}", self.selected, self.device.name)?;
        if !self.device.driver.is_empty() {
            write!(f, " | {}", self.device.driver)?;
        }
        if !self.device.api.is_empty() {
            write!(f, " | {}", self.device.api)?;
        }
        for note in &self.notes {
            match note {
                SelectionNote::EnvironmentOverride { .. } => {
                    f.write_str(" (VULKANIC_BACKEND override)")?
                }
                SelectionNote::Fallback { from, .. } => write!(f, " (fallback from {from})")?,
            }
        }
        Ok(())
    }
}

/// How a request resolved before any backend is tried.
#[derive(Debug, Eq, PartialEq)]
struct Plan {
    candidates: Vec<GraphicsBackend>,
    /// The backend whose failure counts as a fallback, if explicit.
    explicit: Option<GraphicsBackend>,
    notes: Vec<SelectionNote>,
}

/// Resolves `options` against the `VULKANIC_BACKEND` value `env`.
fn plan(options: GalOptions, env: Option<GraphicsBackend>, windowed: bool) -> Plan {
    let mut notes = Vec::new();
    let mut requested = options.backend;
    if let Some(env) = env {
        if options.backend == GraphicsBackend::Auto || options.allow_env_override {
            if env != options.backend {
                notes.push(SelectionNote::EnvironmentOverride { backend: env });
            }
            requested = env;
        }
    }
    if requested == GraphicsBackend::Auto {
        return Plan {
            candidates: GraphicsBackend::automatic_order(windowed),
            explicit: None,
            notes,
        };
    }
    let mut candidates = vec![requested];
    if options.fallback_to_auto {
        candidates.extend(
            GraphicsBackend::automatic_order(windowed)
                .into_iter()
                .filter(|backend| *backend != requested),
        );
    }
    Plan {
        candidates,
        explicit: Some(requested),
        notes,
    }
}

/// Tries the planned candidates with `open_backend` and records the
/// selection on the GAL.
fn open(
    options: GalOptions,
    windowed: bool,
    mut open_backend: impl FnMut(GraphicsBackend) -> GalResult<Box<dyn Backend>>,
) -> GalResult<VulkanicGal> {
    let Plan {
        candidates,
        explicit,
        mut notes,
    } = plan(options, GraphicsBackend::from_env()?, windowed);
    let mut failures = Vec::new();
    for candidate in candidates {
        let result = if candidate.is_compiled() {
            open_backend(candidate)
        } else {
            Err(GalError::unsupported_feature(format!(
                "{candidate} support is not compiled into this build"
            )))
        };
        match result {
            Ok(backend) => {
                if let (Some(from), Some((_, reason))) = (explicit, failures.first()) {
                    notes.push(SelectionNote::Fallback {
                        from,
                        reason: format!("{reason}"),
                    });
                }
                let mut gal = VulkanicGal::new_with_backend(backend);
                gal.set_requested_backend(options.backend, notes);
                return Ok(gal);
            }
            Err(error) => failures.push((candidate, error)),
        }
    }
    match failures.len() {
        0 => Err(GalError::unsupported_feature(
            "no graphics backend is compiled into this build",
        )),
        1 => Err(failures.remove(0).1),
        _ => Err(GalError::unsupported_feature(format!(
            "no graphics backend could start: {}",
            failures
                .iter()
                .map(|(backend, error)| format!("{backend}: {error}"))
                .collect::<Vec<_>>()
                .join("; ")
        ))),
    }
}

/// A native window a Vulkan GAL presents to, as handed over by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeWindow {
    /// Host windowing platform id (X11 or Wayland) from the window request.
    pub platform: u32,
    /// Non-zero id that stays stable for the lifetime of the host window.
    pub stable_window_id: u64,
    /// The host's native display handle (for example an X11 `Display*`).
    pub native_display: u64,
    /// The host's native window handle.
    pub native_window: u64,
}

impl VulkanicGal {
    /// A GAL that owns its device and has no window of its own.
    pub fn create(backend: BackendChoice, label: &str) -> GalResult<Self> {
        Self::create_with(GalOptions::legacy(backend), label)
    }

    /// A GAL with no window of its own, on the backend `options` selects.
    pub fn create_with(options: GalOptions, label: &str) -> GalResult<Self> {
        open(options, false, |backend| create_backend(backend, label))
    }

    /// An OpenGL GAL that renders through the host's current GL context.
    #[cfg(feature = "opengl")]
    pub fn create_borrowed_opengl(label: &str, stable_window_id: u64) -> GalResult<Self> {
        let mut gal =
            Self::new_with_backend(create_borrowed_opengl_backend(label, stable_window_id)?);
        gal.set_requested_backend(GraphicsBackend::OpenGl, Vec::new());
        Ok(gal)
    }

    /// A Vulkan GAL that presents to the host's native window.
    ///
    /// # Safety
    /// The native handles must be valid for their declared platform, and the
    /// window/display must remain alive until the returned GAL is destroyed.
    /// Prefer `create_windowed_vulkan`, which retains the window owner.
    #[cfg(feature = "vulkan")]
    pub unsafe fn create_native_windowed_vulkan(
        label: &str,
        window: NativeWindow,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        unsafe { Self::create_native_windowed(BackendChoice::Vulkan, label, window, surface) }
    }

    /// A presenting GAL on `backend` for the host's native X11 or Wayland
    /// window, given as integer handles.
    ///
    /// # Safety
    /// The native handles must be valid for their declared platform, and the
    /// window/display must remain alive until the returned GAL is destroyed.
    /// Prefer `create_windowed`, which retains the window owner.
    #[cfg(any(feature = "vulkan", feature = "wgpu"))]
    pub unsafe fn create_native_windowed(
        backend: BackendChoice,
        label: &str,
        window: NativeWindow,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        unsafe {
            Self::create_native_windowed_with(GalOptions::legacy(backend), label, window, surface)
        }
    }

    /// A presenting GAL on the backend `options` selects, for the host's
    /// native X11 or Wayland window given as integer handles.
    ///
    /// # Safety
    /// As `create_native_windowed`.
    #[cfg(any(feature = "vulkan", feature = "wgpu"))]
    pub unsafe fn create_native_windowed_with(
        options: GalOptions,
        label: &str,
        window: NativeWindow,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        open(options, true, |backend| match backend {
            #[cfg(feature = "vulkan")]
            GraphicsBackend::Vulkan => create_native_windowed_vulkan_backend(
                label,
                window.platform,
                window.stable_window_id,
                window.native_display,
                window.native_window,
                surface.clone(),
            ),
            #[cfg(feature = "wgpu")]
            GraphicsBackend::Wgpu(api) => unsafe {
                super::backends::create_native_windowed_wgpu_backend(
                    label,
                    api,
                    window.platform,
                    window.native_display,
                    window.native_window,
                    surface.clone(),
                )
            },
            other => Err(windowless(other)),
        })
    }
}

/// Turns GPU frame timestamps on or off for every backend that records them.
pub fn set_gpu_timestamps_requested(requested: bool) {
    #[cfg(feature = "vulkan")]
    backends::set_gpu_timestamps_requested(requested);
    #[cfg(not(feature = "vulkan"))]
    let _ = requested;
}

static DEFAULT_SPIRV_CACHE_DIR: std::sync::RwLock<Option<std::path::PathBuf>> =
    std::sync::RwLock::new(None);

/// Sets the application's directory for the persistent GLSL→SPIR-V cache.
/// `VULKANIC_SPIRV_CACHE_DIR` still overrides it (`off` disables the cache);
/// with neither set, shaders are compiled on every run. The GAL never picks a
/// location itself. Call before creating GALs.
pub fn set_default_spirv_cache_dir(dir: Option<std::path::PathBuf>) {
    *DEFAULT_SPIRV_CACHE_DIR
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = dir;
}

/// The directory set by `set_default_spirv_cache_dir`.
#[cfg(any(feature = "vulkan", feature = "wgpu"))]
pub(crate) fn default_spirv_cache_dir() -> Option<std::path::PathBuf> {
    DEFAULT_SPIRV_CACHE_DIR
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The error for a backend that cannot create a window surface.
#[cfg(any(feature = "vulkan", feature = "wgpu"))]
fn windowless(backend: GraphicsBackend) -> GalError {
    match backend {
        GraphicsBackend::OpenGl => GalError::unsupported_feature(
            "OpenGL GALs do not create window surfaces; use create_borrowed_opengl",
        ),
        other => GalError::unsupported_feature(format!(
            "{other} support is not compiled into this build"
        )),
    }
}

#[cfg(any(feature = "vulkan", feature = "wgpu"))]
impl VulkanicGal {
    /// Creates a presenting GAL on `backend`, retaining ownership of the
    /// window until the GAL has destroyed its surface and device.
    ///
    /// Vulkan and wgpu create surfaces; OpenGL GALs borrow a host context
    /// instead (`create_borrowed_opengl`). Call on the window's owning thread.
    pub fn create_windowed<
        W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + 'static,
    >(
        backend: BackendChoice,
        label: &str,
        window: std::sync::Arc<W>,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        Self::create_windowed_with(GalOptions::legacy(backend), label, window, surface)
    }

    /// Creates a presenting GAL on the backend `options` selects, retaining
    /// ownership of the window. See `create_windowed`.
    pub fn create_windowed_with<
        W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + 'static,
    >(
        options: GalOptions,
        label: &str,
        window: std::sync::Arc<W>,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        open(options, true, |backend| match backend {
            #[cfg(feature = "vulkan")]
            GraphicsBackend::Vulkan => backends::create_owned_windowed_vulkan_backend(
                label,
                window.clone(),
                surface.clone(),
            ),
            #[cfg(feature = "wgpu")]
            GraphicsBackend::Wgpu(api) => super::backends::create_owned_windowed_wgpu_backend(
                label,
                api,
                window.clone(),
                surface.clone(),
            ),
            other => Err(windowless(other)),
        })
    }

    /// Creates a Vulkan device and surface, retaining ownership of the window.
    ///
    /// Supports the native platforms supported by `ash-window`, including X11,
    /// Wayland, Win32 and Metal (macOS requires a Vulkan portability driver).
    /// Call on the window's owning thread. The window stays alive until the GAL
    /// has destroyed its surface and device.
    #[cfg(feature = "vulkan")]
    pub fn create_windowed_vulkan<
        W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + 'static,
    >(
        label: &str,
        window: std::sync::Arc<W>,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        Self::create_windowed(BackendChoice::Vulkan, label, window, surface)
    }

    /// Creates a wgpu device and surface (Vulkan, Metal or Direct3D 12),
    /// retaining ownership of the window. See `create_windowed`.
    #[cfg(feature = "wgpu")]
    pub fn create_windowed_wgpu<
        W: raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle + 'static,
    >(
        label: &str,
        window: std::sync::Arc<W>,
        surface: FrameSurfaceDesc,
    ) -> GalResult<Self> {
        Self::create_windowed(BackendChoice::Wgpu, label, window, surface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_name_every_backend() {
        for backend in GraphicsBackend::ALL {
            assert_eq!(GraphicsBackend::parse(backend.id()), Some(backend));
            assert_eq!(backend.id().parse::<GraphicsBackend>().ok(), Some(backend));
            assert!(!backend.display_name().is_empty());
        }
        assert_eq!(
            GraphicsBackend::parse(" WGPU-Vulkan "),
            Some(GraphicsBackend::Wgpu(WgpuApi::Vulkan))
        );
        assert_eq!(GraphicsBackend::parse("gl"), Some(GraphicsBackend::OpenGl));
        assert_eq!(GraphicsBackend::parse("metal"), None);
        assert_eq!(GraphicsBackend::default(), GraphicsBackend::Auto);
    }

    #[test]
    fn automatic_selection_prefers_native_vulkan() {
        let order = GraphicsBackend::automatic_order(true);
        if cfg!(feature = "vulkan") {
            assert_eq!(order.first(), Some(&GraphicsBackend::Vulkan));
        }
        assert!(
            !order.contains(&GraphicsBackend::OpenGl),
            "windowed GALs never pick OpenGL"
        );
        assert!(order.iter().all(|backend| backend.is_compiled()));
        let selectable = GraphicsBackend::selectable(true);
        assert_eq!(selectable.first(), Some(&GraphicsBackend::Auto));
        assert!(!selectable.contains(&GraphicsBackend::OpenGl));
    }

    #[test]
    fn explicit_choices_resist_the_environment_unless_allowed() {
        let wgpu = GraphicsBackend::Wgpu(WgpuApi::Vulkan);
        let locked = plan(GalOptions::new(GraphicsBackend::Vulkan), Some(wgpu), true);
        assert_eq!(locked.candidates, [GraphicsBackend::Vulkan]);
        assert!(locked.notes.is_empty());

        let overridable = GalOptions {
            allow_env_override: true,
            ..GalOptions::new(GraphicsBackend::Vulkan)
        };
        let overridden = plan(overridable, Some(wgpu), true);
        assert_eq!(overridden.candidates, [wgpu]);
        assert_eq!(
            overridden.notes,
            [SelectionNote::EnvironmentOverride { backend: wgpu }]
        );

        let automatic = plan(GalOptions::default(), Some(wgpu), true);
        assert_eq!(automatic.candidates, [wgpu]);
        assert_eq!(automatic.explicit, Some(wgpu));
    }

    #[test]
    fn fallback_appends_automatic_order_without_repeating() {
        let options = GalOptions::new(GraphicsBackend::Wgpu(WgpuApi::Dx12)).with_fallback();
        let planned = plan(options, None, true);
        assert_eq!(planned.candidates[0], GraphicsBackend::Wgpu(WgpuApi::Dx12));
        assert_eq!(
            &planned.candidates[1..],
            GraphicsBackend::automatic_order(true)
        );
        let strict = plan(
            GalOptions::new(GraphicsBackend::Wgpu(WgpuApi::Dx12)),
            None,
            true,
        );
        assert_eq!(strict.candidates, [GraphicsBackend::Wgpu(WgpuApi::Dx12)]);
    }

    #[test]
    fn info_formats_one_debug_line() {
        let info = BackendInfo {
            requested: GraphicsBackend::Auto,
            selected: GraphicsBackend::Wgpu(WgpuApi::Vulkan),
            notes: vec![SelectionNote::Fallback {
                from: GraphicsBackend::Vulkan,
                reason: "no device".into(),
            }],
            device: DeviceInfo {
                name: "GPU".into(),
                driver: "Driver 1".into(),
                api: "Vulkan 1.3".into(),
                kind: DeviceKind::Discrete,
            },
        };
        assert_eq!(
            info.to_string(),
            "wgpu / Vulkan | GPU | Driver 1 | Vulkan 1.3 (fallback from Vulkan)"
        );
    }

    /// Whether a device-creating test may judge `Auto`: the developer
    /// override would replace it.
    fn auto_is_unaffected() -> bool {
        GraphicsBackend::from_env().ok().flatten().is_none()
    }

    #[cfg(feature = "vulkan")]
    #[test]
    fn automatic_selection_runs_native_vulkan_and_reports_the_device() {
        if !auto_is_unaffected() {
            return;
        }
        let Ok(gal) = VulkanicGal::create_with(GalOptions::default(), "auto selection") else {
            return; // No graphics device: environment gap.
        };
        let info = gal.backend_info();
        eprintln!("backend: {info}");
        assert_eq!(info.requested, GraphicsBackend::Auto);
        assert_eq!(info.selected, GraphicsBackend::Vulkan);
        assert!(info.notes.is_empty());
        assert!(!info.device.name.is_empty());
        assert!(
            info.device.api.starts_with("Vulkan "),
            "{}",
            info.device.api
        );
        assert!(info.to_string().starts_with("Vulkan | "), "{info}");
    }

    #[cfg(all(feature = "wgpu", target_os = "linux"))]
    #[test]
    fn explicit_wgpu_vulkan_is_honored_and_reported() {
        let options = GalOptions::new(GraphicsBackend::Wgpu(WgpuApi::Vulkan));
        let Ok(gal) = VulkanicGal::create_with(options, "explicit wgpu vulkan") else {
            return; // No adapter: environment gap.
        };
        let info = gal.backend_info();
        eprintln!("backend: {info}");
        assert_eq!(info.requested, GraphicsBackend::Wgpu(WgpuApi::Vulkan));
        assert_eq!(info.selected, GraphicsBackend::Wgpu(WgpuApi::Vulkan));
        assert_eq!(info.device.api, "Vulkan through wgpu");
        assert!(!info.device.name.is_empty());
    }

    #[cfg(all(feature = "wgpu", feature = "vulkan", target_os = "linux"))]
    #[test]
    fn unavailable_choices_fail_or_fall_back_as_requested() {
        let metal = GalOptions::new(GraphicsBackend::Wgpu(WgpuApi::Metal));
        let error = VulkanicGal::create_with(metal, "strict metal")
            .err()
            .expect("Metal is not compiled for Linux");
        assert!(error.to_string().contains("not compiled"), "{error}");
        let Ok(gal) = VulkanicGal::create_with(metal.with_fallback(), "metal fallback") else {
            return; // No graphics device: environment gap.
        };
        let info = gal.backend_info();
        eprintln!("backend: {info}");
        assert_eq!(info.requested, GraphicsBackend::Wgpu(WgpuApi::Metal));
        assert!(info.selected != GraphicsBackend::Wgpu(WgpuApi::Metal));
        assert!(matches!(
            info.notes.as_slice(),
            [SelectionNote::Fallback {
                from: GraphicsBackend::Wgpu(WgpuApi::Metal),
                ..
            }]
        ));
        assert!(
            info.to_string().contains("(fallback from wgpu / Metal)"),
            "{info}"
        );
    }

    #[cfg(feature = "wgpu")]
    #[test]
    fn legacy_choices_report_their_request() {
        if !auto_is_unaffected() {
            return;
        }
        let Ok(gal) = VulkanicGal::create(BackendChoice::Wgpu, "legacy wgpu") else {
            return; // No adapter: environment gap.
        };
        let info = gal.backend_info();
        eprintln!("backend: {info}");
        assert_eq!(info.requested, GraphicsBackend::Wgpu(WgpuApi::Auto));
        assert!(matches!(info.selected, GraphicsBackend::Wgpu(api) if api != WgpuApi::Auto));
    }

    #[cfg(all(feature = "opengl", target_os = "linux"))]
    #[test]
    fn opengl_reports_its_renderer() {
        let options = GalOptions::new(GraphicsBackend::OpenGl);
        let Ok(gal) = VulkanicGal::create_with(options, "opengl report") else {
            return; // No GL context: environment gap.
        };
        let info = gal.backend_info();
        eprintln!("backend: {info}");
        assert_eq!(info.selected, GraphicsBackend::OpenGl);
        assert!(
            info.device.api.starts_with("OpenGL "),
            "{}",
            info.device.api
        );
        assert!(!info.device.name.is_empty());
    }
}
