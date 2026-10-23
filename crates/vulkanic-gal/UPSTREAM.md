# VulkanicGAL provenance

Copied from https://github.com/HungLo2020/MattMC at `9e36077987f7a124ec78ba5dd4d7d7b0c38bf567`
(`src/main/rust/render/vulkanic`). Retrieved from GitHub, not the local checkout.

Standalone adaptations: crate paths and logging; an independent Cargo manifest;
architecture checks rooted at this crate; explicit opt-in shader cache location
(`VULKANIC_SPIRV_CACHE_DIR`) instead of MattMC/HOME defaults; owned native-window
surface creation for independent applications; Vulkan portability enumeration;
raw-handle constructor safety contract; mechanical formatting/lint fixes.

The shaderpack/world-renderer integration suites (backend `conformance.rs`,
`conformance_matrix.rs`, and `terrain_interpolation_observation.rs`) remain in
MattMC; they require game code. GAL unit tests and storage conformance are retained.
Driver-required tests are opt-in (`--include-ignored`). The OpenGL state restoration
test creates a stencil attachment instead of assuming the default target has
stencil storage. Test helpers and diagnostics retained for upstream compatibility
may be unused by this standalone crate.

The imported source retains its upstream licensing; no new license is asserted.
