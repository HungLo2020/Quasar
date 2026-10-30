# VulkanicGAL provenance

The current crate re-exports **external, unmodified crates.io wgpu 30.0.1**.
Cargo.lock records its registry checksum. No upstream source is copied, vendored,
patched or wrapped in Quasar resource types. wgpu and its transitive dependencies
retain their own upstream licenses and attribution.

Before the native migration, Quasar carried an independent extraction of MattMC's
graphics engine from `HungLo2020/MattMC` commit
`9e36077987f7a124ec78ba5dd4d7d7b0c38bf567`, originally distributed under MIT.
That historical implementation and its private backends have been removed.
The provenance is retained here for repository history. Quasar's MIT license
continues to cover its own renderer/frame code.
