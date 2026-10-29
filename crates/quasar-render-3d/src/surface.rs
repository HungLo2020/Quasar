//! Additive surface shading; existing lit materials/shader ABI remain unchanged.
use super::*;
pub const DEFAULT_SURFACE_FRAGMENT_SHADER: &str = include_str!("surface.frag");
/// Linear tangent-space RGB normal map, sampled with linear/repeating filtering.
#[derive(Clone, Copy, Debug)]
pub struct NormalMap {
    pub texture_view: Handle,
    pub strength: f32,
    /// Tangent-space +Y by default; set for authored -Y normal maps.
    pub flip_y: bool,
    pub uv_scale: [f32; 2],
}
impl NormalMap {
    pub fn new(texture_view: Handle) -> Self {
        Self {
            texture_view,
            strength: 1.,
            flip_y: false,
            uv_scale: [1.; 2],
        }
    }
}
/// Additive material wrapper preserving the existing LitMaterial API.
#[derive(Clone, Copy, Debug)]
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
            texture_view: self.material.lit.base.texture_view,
            alpha_cutoff: self.material.lit.alpha_cutoff,
            opacity: self.material.lit.base.tint[3],
        }
    }
}
pub(crate) struct SurfaceFrame<'a> {
    pub materials: &'a [SurfaceMaterial],
    pub shadows: Option<&'a ShadowBinding>,
}
/// Extends the lit forward path with normal maps and explicitly prepared shadows.
pub struct SurfaceRenderState3D {
    lit: LitRenderState3D,
    max_shadows: usize,
}
impl SurfaceRenderState3D {
    pub fn with_submission(
        gal: &mut VulkanicGal,
        max_lights: usize,
        max_shadows: usize,
        submission: SubmissionConfig,
    ) -> GalResult<Self> {
        if max_lights > 4096 || max_shadows > 16 {
            return Err(GalError::invalid_argument(
                "surface light/shadow budget exceeded",
            ));
        }
        Ok(Self {
            lit: LitRenderState3D {
                core: RenderState3D::new_submission_configured(
                    gal,
                    Some(DEFAULT_SURFACE_FRAGMENT_SHADER),
                    48 + max_lights.max(1) * 64,
                    16 + max_shadows.max(1) * 96,
                    submission,
                )?,
                max_lights,
            },
            max_shadows,
        })
    }
    pub fn set_texture_sampler(
        &mut self,
        gal: &mut VulkanicGal,
        view: Handle,
        sampler: Handle,
        normal: bool,
    ) -> GalResult<()> {
        self.lit
            .core
            .set_texture_sampler(gal, view, sampler, normal)
    }
    pub fn work_stats(&self) -> RenderWork {
        self.lit.work_stats()
    }
    #[cfg(feature = "diagnostics")]
    pub fn set_diagnostics(&mut self, d: quasar_diagnostics::Diagnostics) {
        self.lit.set_diagnostics(d);
    }
    /// Zero shadow capacity supports normal-map-only use with no shadow passes.
    pub fn new(gal: &mut VulkanicGal, max_lights: usize, max_shadows: usize) -> GalResult<Self> {
        Self::with_fragment_shader(
            gal,
            max_lights,
            max_shadows,
            DEFAULT_SURFACE_FRAGMENT_SHADER,
        )
    }
    pub fn with_fragment_shader(
        gal: &mut VulkanicGal,
        max_lights: usize,
        max_shadows: usize,
        fragment: &str,
    ) -> GalResult<Self> {
        let c = gal.capabilities();
        if max_lights > 4096 || max_shadows > 16 {
            return Err(GalError::invalid_argument(
                "surface budgets: <=4096 lights, 0..=16 shadow maps",
            ));
        }
        if c.limits.max_resource_layout_bindings < 7
            || !c.features.storage_buffers
            || !c.features.uniform_buffers
            || !c.features.dynamic_buffer_offsets
            || !c.features.texture_array_layers
        {
            return Err(GalError::unsupported_feature(
                "surface shaders require seven bindings, storage/uniform buffers, dynamic offsets and texture arrays",
            ));
        }
        Ok(Self {
            lit: LitRenderState3D {
                core: RenderState3D::new_surface_configured(
                    gal,
                    Some(fragment),
                    48 + max_lights.max(1) * 64,
                    16 + max_shadows.max(1) * 96,
                )?,
                max_lights,
            },
            max_shadows,
        })
    }
    /// Record receiver commands. Pass None to omit shadow mapping entirely.
    /// A supplied binding must be produced before these commands are submitted.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        gal: &mut VulkanicGal,
        target: Handle,
        camera: LitCamera3D,
        lighting: Lighting<'_>,
        draws: &[SurfaceDraw3D<'_>],
        shadows: Option<&ShadowBinding>,
        options: impl Into<LitPassOptions>,
    ) -> GalResult<Prepared3D> {
        if shadows.is_some_and(|s| s.records.len() > self.max_shadows) {
            return Err(GalError::invalid_argument("surface shadow budget exceeded"));
        }
        for record in shadows
            .into_iter()
            .flat_map(|s| s.records.iter())
            .filter(|_| draws.iter().any(|d| d.material.receives_shadows))
        {
            if !matches!(
                lighting.lights.get(record[16] as usize),
                Some(Light::Directional { .. } | Light::Spot { .. })
            ) {
                return Err(GalError::invalid_argument(
                    "shadow map must reference an existing directional/spot light",
                ));
            }
        }
        if draws.iter().any(|d| {
            d.material.normal_map.is_some_and(|n| {
                !n.strength.is_finite()
                    || !(0. ..=8.).contains(&n.strength)
                    || n.uv_scale
                        .iter()
                        .any(|x| !x.is_finite() || x.abs() < 1e-6 || x.abs() > 10000.)
            })
        }) {
            return Err(GalError::invalid_argument(
                "invalid normal-map strength or UV scale",
            ));
        }
        let materials: Vec<_> = draws.iter().map(|d| d.material).collect();
        let lit: Vec<_> = draws
            .iter()
            .map(|d| LitDraw3D {
                mesh: d.mesh,
                model: d.model,
                material: d.material.lit,
            })
            .collect();
        self.lit.prepare_internal(
            gal,
            target,
            camera,
            lighting,
            &lit,
            options.into(),
            Some(SurfaceFrame {
                materials: &materials,
                shadows,
            }),
        )
    }
    pub fn clear_bindings(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.lit.clear_bindings(gal)
    }
    pub fn release_targets(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.lit.release_targets(gal)
    }
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.lit.release(gal)
    }
}
