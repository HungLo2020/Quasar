//! Optional submission policies, independent of world storage and frame ownership.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmissionConfig {
    pub instancing: bool,
    pub frustum_culling: bool,
    /// Input instances per receiver pass / total across shadow maps.
    pub max_instances: usize,
}
impl Default for SubmissionConfig {
    fn default() -> Self {
        Self {
            instancing: false,
            frustum_culling: false,
            max_instances: 4096,
        }
    }
}
impl SubmissionConfig {
    pub(crate) fn validate(self, _gpu: &Gpu) -> GraphicsResult<()> {
        if !(1..=4096).contains(&self.max_instances) {
            return Err(error("instance budget must be 1..=4096"));
        }
        if self.frustum_culling && !cfg!(feature = "visibility") {
            return Err(error("enable render3d visibility for frustum culling"));
        }
        Ok(())
    }
    #[cfg(feature = "config")]
    pub fn register_defaults(
        config: &mut quasar_config::Config,
        prefix: &str,
    ) -> Result<(), quasar_config::ConfigError> {
        let d = Self::default();
        config.set_default(format!("{prefix}.instancing"), d.instancing)?;
        config.set_default(format!("{prefix}.frustum_culling"), d.frustum_culling)?;
        config.set_default(format!("{prefix}.max_instances"), d.max_instances)
    }
    #[cfg(feature = "config")]
    pub fn from_config(
        config: &quasar_config::Config,
        prefix: &str,
    ) -> Result<Self, quasar_config::ConfigError> {
        let result = Self {
            instancing: config.get_bool(&format!("{prefix}.instancing"))?,
            frustum_culling: config.get_bool(&format!("{prefix}.frustum_culling"))?,
            max_instances: usize::try_from(config.get_u64(&format!("{prefix}.max_instances"))?)
                .unwrap_or(usize::MAX),
        };
        if !(1..=4096).contains(&result.max_instances) {
            return Err(quasar_config::ConfigError::new(
                quasar_config::ConfigErrorKind::Validation,
                "instance budget must be 1..=4096",
            ));
        }
        Ok(result)
    }
}
/// Work from the most recent preparation. Instances and physical draws are distinct.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderWork {
    pub candidates: usize,
    pub visible: usize,
    pub culled: usize,
    pub draws: usize,
    pub instance_upload_bytes: usize,
}
impl RenderWork {
    #[cfg(feature = "diagnostics")]
    pub fn report(&self, diagnostics: &quasar_diagnostics::Diagnostics) {
        use quasar_diagnostics::{Field, Level};
        if diagnostics.is_enabled(Level::Debug) {
            diagnostics.event_with(
                Level::Debug,
                "quasar.render3d",
                "submission_work",
                [
                    Field::new("candidates", self.candidates as u64),
                    Field::new("visible", self.visible as u64),
                    Field::new("culled", self.culled as u64),
                    Field::new("draws", self.draws as u64),
                    Field::new("instance_upload_bytes", self.instance_upload_bytes as u64),
                ],
            );
        }
    }
}
impl std::ops::AddAssign for RenderWork {
    fn add_assign(&mut self, rhs: Self) {
        self.candidates += rhs.candidates;
        self.visible += rhs.visible;
        self.culled += rhs.culled;
        self.draws += rhs.draws;
        self.instance_upload_bytes += rhs.instance_upload_bytes;
    }
}

pub(crate) struct Visibility {
    #[cfg(feature = "visibility")]
    frustum: Option<quasar_spatial::Frustum<3>>,
}
impl Visibility {
    pub(crate) fn new(enabled: bool, view_projection: Mat4) -> Self {
        #[cfg(feature = "visibility")]
        {
            Self {
                frustum: enabled
                    .then(|| {
                        quasar_spatial::Frustum::from_view_projection(
                            view_projection.0.map(|c| c.map(f64::from)),
                            quasar_spatial::ClipDepth::ZeroToOne,
                        )
                        .ok()
                    })
                    .flatten(),
            }
        }
        #[cfg(not(feature = "visibility"))]
        {
            let _ = (enabled, view_projection);
            Self {}
        }
    }
    pub(crate) fn intersects(&self, mesh: &Mesh, model: Mat4) -> bool {
        #[cfg(feature = "visibility")]
        if let Some(frustum) = &self.frustum {
            // Invalid/projective transforms fail open, preserving legacy drawing.
            if let Ok(bounds) = quasar_spatial::Aabb::new(
                mesh.bounds.0.map(f64::from),
                mesh.bounds.1.map(f64::from),
            )
            .and_then(|b| b.transformed_affine(model.0.map(|c| c.map(f64::from))))
            {
                let magnitude = bounds
                    .min
                    .iter()
                    .chain(&bounds.max)
                    .fold(1f64, |m, v| m.max(v.abs()));
                let padded = bounds.expanded(magnitude * 64. * f64::from(f32::EPSILON));
                if let Ok(bounds) = padded {
                    return frustum.intersects_aabb(&bounds);
                }
            }
        }
        let _ = (mesh, model);
        true
    }
}

#[cfg(all(test, feature = "config"))]
mod tests {
    use super::*;
    #[test]
    fn reusable_submission_config_respects_sources_and_validates_budget() {
        let mut c = quasar_config::Config::new();
        SubmissionConfig::register_defaults(&mut c, "renderer").unwrap();
        c.set_file("renderer.instancing", true).unwrap();
        c.set_cli("renderer.max_instances", 128u64).unwrap();
        c.set_file("renderer.max_instances", 4u64).unwrap();
        let settings = SubmissionConfig::from_config(&c, "renderer").unwrap();
        assert!(settings.instancing);
        assert!(!settings.frustum_culling);
        assert_eq!(settings.max_instances, 128);
        c.set_cli("renderer.max_instances", 4097u64).unwrap();
        assert!(SubmissionConfig::from_config(&c, "renderer").is_err());
    }
}
