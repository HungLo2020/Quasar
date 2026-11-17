//! One frame of voxels and meshes on a shared [`SceneTarget`].
//!
//! Voxels and render-3d meshes compose through the scene's colour and depth.
//! The order matters, and [`HybridFrame`] enforces it so games do not have
//! to rediscover it:
//!
//! 1. [`Stage::Compute`]: skinning, crowd posing and other compute work
//!    whose output later passes read.
//! 2. [`Stage::Shadows`]: mesh shadow casters
//!    ([`ShadowRenderState3D::prepare_into`]) and then voxel casters
//!    ([`VoxelRenderer::prepare_depth`] into
//!    [`ShadowTarget::layer_view`], `clear: false`).
//! 3. [`Stage::Opaque`]: opaque voxels (the first opaque pass clears the
//!    scene), then opaque meshes with [`PassOptions::load`]; use
//!    [`HybridFrame::mesh_pass_options`] to get the right options.
//! 4. [`Stage::Decals`]: [`DecalRenderer`] reads the finished opaque depth.
//! 5. [`Stage::Translucent`]: translucent voxels (water, glass), then
//!    blended meshes ([`SurfaceRenderState3D::prepare_transparent`]).
//! 6. [`Stage::Effects`]: particles and outlines.
//! 7. [`Stage::Post`]: [`PostChain`] into the output, then the HUD.
//!
//! Adding work for an earlier stage after a later one is an error, and
//! [`HybridCommands::submit`] submits everything and reports each voxel
//! submission to its renderer ([`VoxelRenderer::submitted`]), which callers
//! otherwise have to remember after every submit.
//!
//! [`ShadowRenderState3D::prepare_into`]: quasar_render_3d::ShadowRenderState3D::prepare_into
//! [`ShadowTarget::layer_view`]: quasar_render_3d::ShadowTarget::layer_view
//! [`PassOptions::load`]: quasar_render_3d::PassOptions::load
//! [`DecalRenderer`]: quasar_render_3d::DecalRenderer
//! [`SurfaceRenderState3D::prepare_transparent`]: quasar_render_3d::SurfaceRenderState3D::prepare_transparent
//! [`PostChain`]: quasar_render_3d::PostChain
//! [`SceneTarget`]: quasar_render_3d::SceneTarget
use crate::{PreparedVoxels, VoxelRenderer};
use quasar_frame::{Gpu, GraphicsResult, error};
use quasar_render_3d::{PassOptions, Prepared3D, PreparedShadows};
use vulkanic_gal::{CommandBuffer, SubmissionIndex};

/// Frame stages in submission order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Stage {
    Compute,
    Shadows,
    Opaque,
    Decals,
    Translucent,
    Effects,
    Post,
}

/// Collects one frame's command buffers in [`Stage`] order. See the module
/// docs.
#[derive(Default)]
pub struct HybridFrame {
    commands: Vec<CommandBuffer>,
    voxel_submissions: Vec<u64>,
    stage: Option<Stage>,
    opaque_drawn: bool,
}

impl HybridFrame {
    pub fn new() -> Self {
        Self::default()
    }

    /// The latest stage that received work.
    pub fn stage(&self) -> Option<Stage> {
        self.stage
    }

    fn enter(&mut self, stage: Stage) -> GraphicsResult<()> {
        if self.stage.is_some_and(|current| stage < current) {
            return Err(error(format!(
                "{stage:?} work added after {:?}; record stages in order",
                self.stage.unwrap()
            )));
        }
        self.stage = Some(stage);
        Ok(())
    }

    /// Adds any command buffer at `stage` (custom passes, particles, UI).
    pub fn push(&mut self, stage: Stage, commands: CommandBuffer) -> GraphicsResult<()> {
        self.enter(stage)?;
        self.commands.push(commands);
        Ok(())
    }

    /// Adds a voxel pass at `stage` and remembers its submission.
    pub fn voxels(&mut self, stage: Stage, prepared: PreparedVoxels) -> GraphicsResult<()> {
        self.enter(stage)?;
        if stage == Stage::Opaque {
            self.opaque_drawn = true;
        }
        self.voxel_submissions.push(prepared.submission);
        self.commands.push(prepared.commands);
        Ok(())
    }

    /// Adds a mesh pass at `stage`.
    pub fn meshes(&mut self, stage: Stage, prepared: Prepared3D) -> GraphicsResult<()> {
        self.enter(stage)?;
        if stage == Stage::Opaque {
            self.opaque_drawn = true;
        }
        self.commands.push(prepared.commands);
        Ok(())
    }

    /// Adds mesh shadow casters.
    pub fn mesh_shadows(&mut self, prepared: PreparedShadows) -> GraphicsResult<()> {
        self.enter(Stage::Shadows)?;
        self.commands.push(prepared.commands);
        Ok(())
    }

    /// Options for the next opaque pass: clear colour and depth when nothing
    /// opaque was drawn yet, otherwise load both so meshes share the voxels'
    /// depth.
    pub fn mesh_pass_options(&self, clear_color: [f32; 4]) -> PassOptions {
        if self.opaque_drawn {
            PassOptions::load()
        } else {
            PassOptions {
                clear_color: Some(clear_color),
                clear_depth: true,
            }
        }
    }

    /// Whether an opaque pass has been added (voxel passes clear only when
    /// this is false).
    pub fn opaque_drawn(&self) -> bool {
        self.opaque_drawn
    }

    pub fn finish(self) -> HybridCommands {
        HybridCommands {
            commands: self.commands,
            voxel_submissions: self.voxel_submissions,
        }
    }
}

/// Ordered commands of one frame plus the voxel submissions they contain.
pub struct HybridCommands {
    pub commands: Vec<CommandBuffer>,
    pub voxel_submissions: Vec<u64>,
}

impl HybridCommands {
    /// Submits every command buffer in order, then reports each voxel
    /// submission to `voxels`.
    pub fn submit(self, gpu: &Gpu, voxels: &mut VoxelRenderer) -> SubmissionIndex {
        let index = gpu.queue.submit(self.commands);
        for submission in self.voxel_submissions {
            voxels.submitted(gpu, submission);
        }
        index
    }

    /// For frame loops that submit the commands themselves (for example
    /// `quasar_frame::OffscreenTarget::render`): call after that submit.
    pub fn submitted(voxel_submissions: &[u64], gpu: &Gpu, voxels: &mut VoxelRenderer) {
        for submission in voxel_submissions {
            voxels.submitted(gpu, *submission);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_must_be_recorded_in_order_and_opaque_switches_mesh_options() {
        let mut frame = HybridFrame::new();
        let clear = [0., 0., 0., 1.];
        assert!(frame.mesh_pass_options(clear).clear_depth);
        frame.enter(Stage::Compute).unwrap();
        frame.enter(Stage::Shadows).unwrap();
        frame.enter(Stage::Shadows).unwrap();
        frame.enter(Stage::Opaque).unwrap();
        frame.opaque_drawn = true;
        let options = frame.mesh_pass_options(clear);
        assert!(!options.clear_depth && options.clear_color.is_none());
        frame.enter(Stage::Translucent).unwrap();
        let late = frame.enter(Stage::Decals).unwrap_err();
        assert!(late.to_string().contains("Decals"), "{late}");
        frame.enter(Stage::Post).unwrap();
        assert_eq!(frame.stage(), Some(Stage::Post));
    }
}
