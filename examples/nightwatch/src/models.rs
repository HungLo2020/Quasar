//! Game-owned model selection and placements; decoding/upload are reusable library APIs.
use crate::props::ModelId;
use quasar_assets::{AssetServer, CacheBudget, GltfLoader, Handle as AssetHandle, model::Model3D};
use quasar_frame::{Gpu, GraphicsError, GraphicsResult, error};
use quasar_physics::dim3::{Bounds3, RigidPose3, model_bounds, model_bounds_selected};
use quasar_render_3d::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub(crate) struct Models {
    server: AssetServer,
    handles: BTreeMap<ModelId, AssetHandle<Model3D>>,
    pub gpu: BTreeMap<ModelId, UploadedModel>,
    bounds: BTreeMap<ModelId, Bounds3>,
    pub root: PathBuf,
}
fn failure(e: impl std::fmt::Display) -> GraphicsError {
    error(format!("Nightwatch models: {e}"))
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
    pub fn load(gal: &Gpu, root: &Path) -> GraphicsResult<Self> {
        let mut server = AssetServer::with_budget(CacheBudget::new(64 * 1024 * 1024, 32));
        server.mount_file("", root).map_err(failure)?;
        server.register(GltfLoader {
            allow_animation_default_pose: true,
            ..GltfLoader::default()
        });
        let mut result = Self {
            server,
            handles: BTreeMap::new(),
            gpu: BTreeMap::new(),
            bounds: BTreeMap::new(),
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
                        .values()
                        .map(|m| m.texture_upload_bytes as u64)
                        .sum::<u64>(),
                ),
                Field::new("material_conversion", "classic approximation"),
                Field::new("source", "Kenney CC0"),
                Field::new(
                    "default_pose_models",
                    self.handles
                        .values()
                        .filter(|m| m.warnings.iter().any(|w| w.contains("animation clips")))
                        .count() as u64,
                ),
            ],
        );
        self.server.set_diagnostics(Some(diagnostics));
    }
    pub fn reload(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        for id in ModelId::ALL {
            self.server.invalidate(id.file(), true).map_err(failure)?;
        }
        let mut next = BTreeMap::new();
        let mut handles = BTreeMap::new();
        let mut bounds = BTreeMap::new();
        let result = (|| {
            for id in ModelId::ALL {
                let model = self.server.load::<GltfLoader>(id.file()).map_err(failure)?;
                let fitted = if id == ModelId::Workbench {
                    model_bounds_selected(&model, |i| i.name == "workbench")
                } else {
                    model_bounds(&model)
                }
                .map_err(failure)?;
                bounds.insert(id, fitted);
                let gpu = UploadedModel::upload(
                    gal,
                    &model,
                    ModelUploadOptions {
                        material_conversion: MaterialConversion::ClassicApproximation,
                        ..ModelUploadOptions::default()
                    },
                )?;
                next.insert(id, gpu);
                handles.insert(id, model);
            }
            Ok(())
        })();
        if let Err(e) = result {
            for m in next.values_mut() {
                let _ = m.release(gal);
            }
            return Err(e);
        }
        let mut old = std::mem::replace(&mut self.gpu, next);
        self.handles = handles;
        self.bounds = bounds;
        for model in old.values_mut() {
            model.release(gal)?;
        }
        Ok(())
    }
    pub fn bind(
        &self,
        gal: &Gpu,
        world: &mut SurfaceRenderState3D,
        weapon: &mut SurfaceRenderState3D,
        shadows: &mut ShadowRenderState3D,
    ) -> GraphicsResult<()> {
        for (&id, m) in &self.gpu {
            m.bind_samplers(gal, world)?;
            if id.is_weapon() {
                m.bind_samplers(gal, weapon)?;
            }
            m.bind_shadow_samplers(gal, shadows)?;
        }
        Ok(())
    }
    pub fn fit(&self, id: ModelId, pose: RigidPose3, half: [f32; 3]) -> Mat4 {
        Mat4(
            self.bounds[&id]
                .fit_matrix(pose, half)
                .expect("validated prop/model fit"),
        )
    }
    pub fn release(&mut self, gal: &Gpu) -> GraphicsResult<()> {
        for m in self.gpu.values_mut() {
            m.release(gal)?;
        }
        self.gpu.clear();
        Ok(())
    }
}
