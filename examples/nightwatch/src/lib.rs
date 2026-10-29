//! A scene/ECS-free FPS: game-owned state, navigation, and optional shared-GAL rendering.
#[cfg(feature = "graphics")]
mod geometry;
#[cfg(feature = "graphics")]
mod models;
#[cfg(feature = "graphics")]
pub mod render;
pub mod settings;
pub mod world;
#[derive(Debug)]
pub struct GameError(String);
impl std::fmt::Display for GameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for GameError {}
impl From<String> for GameError {
    fn from(s: String) -> Self {
        Self(s)
    }
}
impl From<&str> for GameError {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}
macro_rules! error_from {
    ($ty:ty) => {
        impl From<$ty> for GameError {
            fn from(e: $ty) -> Self {
                Self(e.to_string())
            }
        }
    };
}
error_from!(quasar_config::CliError);
error_from!(std::num::ParseIntError);
error_from!(std::io::Error);
#[cfg(feature = "graphics")]
error_from!(vulkanic_gal::GalError);
#[cfg(feature = "graphics")]
error_from!(quasar_render_2d::Error);
pub type Result<T> = std::result::Result<T, GameError>;

error_from!(quasar_config::ConfigError);
error_from!(quasar_profile::BenchmarkError);
error_from!(quasar_diagnostics::ParseLevelError);
error_from!(std::num::TryFromIntError);
