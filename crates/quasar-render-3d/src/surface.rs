//! Normal-mapped materials on the same native forward-lighting pipeline.
use super::*;
pub const DEFAULT_SURFACE_FRAGMENT_SHADER: &str = include_str!("surface.frag.wgsl");
#[derive(Clone, Debug)]
pub struct NormalMap {
    pub texture_view: TextureView,
    pub strength: f32,
    pub flip_y: bool,
    pub uv_scale: [f32; 2],
}
impl NormalMap {
    pub fn new(texture_view: &TextureView) -> Self {
        Self {
            texture_view: texture_view.clone(),
            strength: 1.,
            flip_y: false,
            uv_scale: [1.; 2],
        }
    }
}
#[derive(Clone, Debug)]
pub struct SurfaceMaterial {
    pub lit: LitMaterial,
    pub normal_map: Option<NormalMap>,
    pub receives_shadows: bool,
}
impl From<LitMaterial> for SurfaceMaterial {
    fn from(lit: LitMaterial) -> Self {
        Self {
            lit,
            normal_map: None,
            receives_shadows: true,
        }
    }
}
pub struct SurfaceDraw3D<'a> {
    pub mesh: &'a Mesh,
    pub model: Mat4,
    pub material: SurfaceMaterial,
}
impl<'a> SurfaceDraw3D<'a> {
    pub fn shadow_caster(&self) -> ShadowCaster<'a> {
        ShadowCaster {
            mesh: self.mesh,
            model: self.model,
            texture_view: self.material.lit.base.texture_view.clone(),
            alpha_cutoff: self.material.lit.alpha_cutoff,
            opacity: self.material.lit.base.tint[3],
        }
    }
}
pub struct SurfaceRenderState3D {
    lit: LitRenderState3D,
    max_shadows: usize,
}
impl SurfaceRenderState3D {
    pub fn new(gpu: &Gpu, max_lights: usize, max_shadows: usize) -> GraphicsResult<Self> {
        Self::with_submission(gpu, max_lights, max_shadows, SubmissionConfig::default())
    }
    pub fn with_submission(
        gpu: &Gpu,
        max_lights: usize,
        max_shadows: usize,
        submission: SubmissionConfig,
    ) -> GraphicsResult<Self> {
        if max_shadows > 16 {
            return Err(error("shadow budget exceeds 16"));
        }
        Ok(Self {
            lit: LitRenderState3D::with_submission(gpu, max_lights, submission)?,
            max_shadows,
        })
    }
    pub fn with_fragment_shader(
        gpu: &Gpu,
        max_lights: usize,
        max_shadows: usize,
        fragment: &str,
    ) -> GraphicsResult<Self> {
        if max_shadows > 16 {
            return Err(error("shadow budget exceeds 16"));
        }
        Ok(Self {
            lit: LitRenderState3D::with_fragment_shader(gpu, max_lights, fragment)?,
            max_shadows,
        })
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Explicit caller-owned pass inputs preserve independent composition."
    )]
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        camera: LitCamera3D,
        lighting: Lighting<'_>,
        draws: &[SurfaceDraw3D<'_>],
        shadows: Option<&ShadowBinding>,
        options: impl Into<LitPassOptions>,
    ) -> GraphicsResult<Prepared3D> {
        if shadows.is_some_and(|s| s.records.len() > self.max_shadows)
            || draws.iter().any(|d| {
                !d.material.lit.valid()
                    || d.material.normal_map.as_ref().is_some_and(|n| {
                        !n.strength.is_finite()
                            || !(0. ..=8.).contains(&n.strength)
                            || n.uv_scale
                                .iter()
                                .any(|x| !x.is_finite() || x.abs() < 1e-6 || x.abs() > 10000.)
                    })
            })
        {
            return Err(error(
                "invalid normal map/material or shadow budget exceeded",
            ));
        }
        if lighting.output_color_space == ColorSpace::Srgb
            && (target.format.is_srgb() || draws.iter().any(|d| d.material.lit.base.transparent))
        {
            return Err(error(
                "use a linear target for blending; encode sRGB only once",
            ));
        }
        if let Some(binding) = shadows {
            for record in &binding.records {
                if !matches!(
                    lighting.lights.get(record[16] as usize),
                    Some(Light::Directional { .. } | Light::Spot { .. })
                ) {
                    return Err(error(
                        "shadow map must reference a supplied directional or spot light",
                    ));
                }
            }
        }
        let data = self.lit.light_bytes(camera, lighting)?;
        let input: Vec<_> = draws
            .iter()
            .map(|d| Input {
                mesh: d.mesh,
                model: d.model,
                base: &d.material.lit.base,
                lit: Some(&d.material.lit),
                normal: d.material.normal_map.as_ref(),
                receives_shadows: d.material.receives_shadows,
            })
            .collect();
        self.lit.core.prepare(
            gpu,
            target,
            Camera3D {
                view_projection: camera.view_projection,
                light_direction: [0., 1., 0.],
            },
            &input,
            options.into().pass,
            Some(&data),
            shadows,
        )
    }
    pub fn set_texture_sampler(
        &mut self,
        _gpu: &Gpu,
        view: &TextureView,
        sampler: &Sampler,
        normal: bool,
    ) -> GraphicsResult<()> {
        if normal {
            &mut self.lit.core.normal_samplers
        } else {
            &mut self.lit.core.samplers
        }
        .insert(view.clone(), sampler.clone());
        Ok(())
    }
    pub fn work_stats(&self) -> RenderWork {
        self.lit.work_stats()
    }
    #[cfg(feature = "diagnostics")]
    pub fn set_diagnostics(&mut self, d: quasar_diagnostics::Diagnostics) {
        self.lit.set_diagnostics(d)
    }
    pub fn clear_bindings(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        self.lit.clear_bindings(gpu)
    }
    pub fn release_targets(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        self.lit.release_targets(gpu)
    }
    pub fn release(&mut self, gpu: &Gpu) -> GraphicsResult<()> {
        self.lit.release(gpu)
    }
}
