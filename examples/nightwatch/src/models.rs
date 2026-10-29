//! Game-owned model selection and placements; decoding/upload are reusable library APIs.
use quasar_assets::{AssetServer, CacheBudget, GltfLoader, Handle as AssetHandle, model::Model3D};
use quasar_render_3d::*;
use std::path::{Path, PathBuf};
use vulkanic_gal::*;
pub(crate) const FILES: [&str; 7] = [
    "blaster-kit/blaster-d.glb",
    "blaster-kit/blaster-f.glb",
    "blaster-kit/crate-medium.glb",
    "blaster-kit/crate-wide.glb",
    "survival-kit/barrel.glb",
    "survival-kit/workbench.glb",
    "survival-kit/metal-panel.glb",
];
pub(crate) struct Models {
    server: AssetServer,
    handles: Vec<AssetHandle<Model3D>>,
    pub gpu: Vec<UploadedModel>,
    pub root: PathBuf,
}
fn failure(e: impl std::fmt::Display) -> GalError {
    GalError::invalid_argument(format!("Nightwatch models: {e}"))
}
impl Models {
    pub fn default_root() -> PathBuf {
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
            && dir.join("assets/blaster-kit").is_dir()
        {
            return dir.join("assets");
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
    }
    pub fn load(gal: &mut VulkanicGal, root: &Path) -> GalResult<Self> {
        let mut server = AssetServer::with_budget(CacheBudget::new(64 * 1024 * 1024, 32));
        server.mount_file("", root).map_err(failure)?;
        server.register(GltfLoader {
            allow_animation_default_pose: true,
            ..GltfLoader::default()
        });
        let mut result = Self {
            server,
            handles: Vec::new(),
            gpu: Vec::new(),
            root: root.into(),
        };
        result.reload(gal)?;
        Ok(result)
    }
    pub fn set_diagnostics(&mut self, diagnostics: quasar_diagnostics::Diagnostics) {
        use quasar_diagnostics::{Field, Level};
        diagnostics.event_with(
            Level::Info,
            "nightwatch",
            "models_loaded",
            [
                Field::new("models", self.gpu.len() as u64),
                Field::new(
                    "texture_bytes",
                    self.gpu
                        .iter()
                        .map(|m| m.texture_upload_bytes as u64)
                        .sum::<u64>(),
                ),
                Field::new("material_conversion", "classic approximation"),
                Field::new("source", "Kenney CC0"),
                Field::new(
                    "default_pose_models",
                    self.handles
                        .iter()
                        .filter(|m| m.warnings.iter().any(|w| w.contains("animation clips")))
                        .count() as u64,
                ),
            ],
        );
        self.server.set_diagnostics(Some(diagnostics));
    }
    pub fn reload(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        if self.gpu.iter().any(UploadedModel::has_pending_commands) {
            return Err(GalError::submission(
                StatusCode::InFlight,
                "Nightwatch model commands are retained",
            ));
        }
        for path in FILES {
            self.server.invalidate(path, true).map_err(failure)?;
        }
        let mut next = Vec::new();
        let mut handles = Vec::new();
        let result = (|| {
            for path in FILES {
                let model = self.server.load::<GltfLoader>(path).map_err(failure)?;
                let gpu = UploadedModel::upload(
                    gal,
                    &model,
                    ModelUploadOptions {
                        material_conversion: MaterialConversion::ClassicApproximation,
                        ..ModelUploadOptions::default()
                    },
                )?;
                next.push(gpu);
                handles.push(model);
            }
            Ok(())
        })();
        if let Err(e) = result {
            for m in &mut next {
                let _ = m.release(gal);
            }
            return Err(e);
        }
        let mut old = std::mem::replace(&mut self.gpu, next);
        self.handles = handles;
        for model in &mut old {
            model.release(gal)?;
        }
        Ok(())
    }
    pub fn bind(
        &self,
        gal: &mut VulkanicGal,
        world: &mut SurfaceRenderState3D,
        weapon: &mut SurfaceRenderState3D,
        shadows: &mut ShadowRenderState3D,
    ) -> GalResult<()> {
        for (index, m) in self.gpu.iter().enumerate() {
            m.bind_samplers(gal, if index < 2 { &mut *weapon } else { &mut *world })?;
            if index >= 2 {
                m.bind_shadow_samplers(gal, shadows)?;
            }
        }
        Ok(())
    }
    pub fn fit_uniform(&self, index: usize, center: [f32; 3], half: [f32; 3]) -> Mat4 {
        let (min, max) = self.gpu[index].bounds().unwrap();
        let scale = (0..3)
            .map(|i| half[i] * 2. / (max[i] - min[i]).max(0.001))
            .fold(f32::INFINITY, f32::min);
        Mat4::translation(center[0], center[1], center[2])
            * Mat4::scale(scale, scale, scale)
            * Mat4::translation(
                -(min[0] + max[0]) / 2.,
                -(min[1] + max[1]) / 2.,
                -(min[2] + max[2]) / 2.,
            )
    }
    pub fn fit(&self, index: usize, center: [f32; 3], half: [f32; 3]) -> Mat4 {
        let (min, max) = self.gpu[index].bounds().expect("validated nonempty model");
        let scale = std::array::from_fn::<_, 3, _>(|i| half[i] * 2. / (max[i] - min[i]).max(0.001));
        Mat4::translation(center[0], center[1], center[2])
            * Mat4::scale(scale[0], scale[1], scale[2])
            * Mat4::translation(
                -(min[0] + max[0]) / 2.,
                -(min[1] + max[1]) / 2.,
                -(min[2] + max[2]) / 2.,
            )
    }
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        for m in &mut self.gpu {
            m.release(gal)?;
        }
        self.gpu.clear();
        Ok(())
    }
}
