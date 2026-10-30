//! Optional Rapier integration for Quasar.
//!
//! Quasar does not implement a rigid-body solver. This crate wraps Rapier 2D
//! and 3D behind small caller-owned worlds:
//!
//! - `dim2::CollisionWorld` / `dim3::CollisionWorld` expose collider-only
//!   overlap, ray and shape queries and never integrate bodies by themselves.
//! - `dim2::PhysicsWorld` / `dim3::PhysicsWorld` add fixed-step dynamics,
//!   joints, character-controller-ready raw access, collision events and
//!   optional `quasar-spatial` index construction.
//!
//! Both wrappers [`std::ops::Deref`] to Rapier's `PhysicsWorld`, so every
//! advanced set, pipeline and query remains directly available. Rapier types
//! are re-exported as `rapier2d` and `rapier3d`.
//!
//! # Features
//!
//! - `dim2` / `dim3`: select one or both dimensions. Disable defaults and pick
//!   one to avoid compiling the other.
//! - `dynamics` (default): compile the fixed-step `dim2::PhysicsWorld` and
//!   `dim3::PhysicsWorld` wrappers. Without it only collision worlds remain.
//! - `events` (default): collect Rapier collision/contact-force events during
//!   stepping through `std::sync::mpsc` channels.
//! - `model-assets`: optional decoded-model bounds and explicit cylinder fitting (3D).
//! - `spatial`: opt-in conversions and collider index construction using
//!   `quasar-spatial`. No dependency exists in the other direction.
//! - `enhanced-determinism`: forwards Rapier's feature of the same name.
//!
//! # Determinism
//!
//! Rapier is deterministic for the same build, feature set, platform and
//! sequence of fixed steps. Cross-platform bitwise equality is not promised;
//! use `quasar-sim` with fixed ticks and quantized hashes for replay checks.
//! Always prefer `dim2::PhysicsWorld::step_fixed` over variable-delta
//! `advance` when replay matters.
mod common;

#[cfg(feature = "dim3")]
mod character3;
#[cfg(feature = "dim2")]
pub mod dim2;
#[cfg(feature = "dim3")]
pub mod dim3;
#[cfg(feature = "dim3")]
mod shapes3;

#[cfg(feature = "dim2")]
pub use rapier2d;
#[cfg(feature = "dim3")]
pub use rapier3d;

pub use common::{MAX_FIXED_DT, PhysicsError, PhysicsEvents, StepConfig, StepReport, WorldConfig};
