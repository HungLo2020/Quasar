//! Opt-in forward lighting. All inputs are ordinary caller-owned values/slices.
use super::*;

/// Default lit fragment source, also a starting point for custom shader variants.
pub const DEFAULT_LIT_FRAGMENT_SHADER: &str = include_str!("lit.frag");

/// Color textures are decoded explicitly; GAL UNORM targets do not perform sRGB conversion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorSpace {
    #[default]
    Linear,
    Srgb,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShadingModel {
    Unlit,
    Diffuse,
    #[default]
    BlinnPhong,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Attenuation {
    #[default]
    Smooth,
    InverseSquare,
}

/// Directions point along emitted light rays. Colors/intensities are linear.
#[derive(Clone, Copy, Debug)]
pub enum Light {
    Directional {
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
    },
    Point {
        position: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        range: f32,
        attenuation: Attenuation,
    },
    Spot {
        position: [f32; 3],
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        range: f32,
        inner_angle: f32,
        outer_angle: f32,
        attenuation: Attenuation,
    },
}
impl Light {
    fn encoded(self) -> GalResult<[f32; 16]> {
        let (position, kind, direction, range, color, intensity, inner, outer, attenuation) =
            match self {
                Self::Directional {
                    direction,
                    color,
                    intensity,
                } => (
                    [0.; 3],
                    0.,
                    direction,
                    1.,
                    color,
                    intensity,
                    0.,
                    0.,
                    Attenuation::Smooth,
                ),
                Self::Point {
                    position,
                    color,
                    intensity,
                    range,
                    attenuation,
                } => (
                    position,
                    1.,
                    [0., -1., 0.],
                    range,
                    color,
                    intensity,
                    0.,
                    0.,
                    attenuation,
                ),
                Self::Spot {
                    position,
                    direction,
                    color,
                    intensity,
                    range,
                    inner_angle,
                    outer_angle,
                    attenuation,
                } => {
                    if !inner_angle.is_finite()
                        || !outer_angle.is_finite()
                        || inner_angle < 0.
                        || outer_angle <= inner_angle
                        || outer_angle >= std::f32::consts::FRAC_PI_2
                    {
                        return Err(GalError::invalid_argument(
                            "spot half-angles require 0 <= inner < outer < pi/2",
                        ));
                    }
                    (
                        position,
                        2.,
                        direction,
                        range,
                        color,
                        intensity,
                        inner_angle.cos(),
                        outer_angle.cos(),
                        attenuation,
                    )
                }
            };
        let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
        if !position.iter().chain(&direction).all(|x| x.is_finite())
            || !length.is_finite()
            || length < 1e-6
            || !positive_rgb(color)
            || !intensity.is_finite()
            || intensity < 0.
            || !range.is_finite()
            || range <= 0.
        {
            return Err(GalError::invalid_argument(
                "invalid light position/direction/color/intensity/range",
            ));
        }
        Ok([
            position[0],
            position[1],
            position[2],
            kind,
            direction[0] / length,
            direction[1] / length,
            direction[2] / length,
            range,
            color[0],
            color[1],
            color[2],
            intensity,
            inner,
            outer,
            if attenuation == Attenuation::Smooth {
                0.
            } else {
                1.
            },
            0.,
        ])
    }
}
#[derive(Clone, Copy, Debug)]
pub struct LitCamera3D {
    pub view_projection: Mat4,
    pub position: [f32; 3],
}
/// Borrowed per-pass light data; never tied to entities, nodes or a global world.
#[derive(Clone, Copy, Debug)]
pub struct Lighting<'a> {
    pub ambient: [f32; 3],
    pub lights: &'a [Light],
    /// Use Linear for HDR/composition targets; encode only the final display pass.
    pub output_color_space: ColorSpace,
}
impl Default for Lighting<'_> {
    fn default() -> Self {
        Self {
            ambient: [0.03; 3],
            lights: &[],
            output_color_space: ColorSpace::Linear,
        }
    }
}
/// Classic material parameters. Base tint, emissive and specular colors are linear.
/// Texture decoding is explicit; alpha is never gamma-corrected.
#[derive(Clone, Copy, Debug)]
pub struct LitMaterial {
    pub base: Material,
    pub shading: ShadingModel,
    pub specular: [f32; 3],
    pub shininess: f32,
    pub emissive: [f32; 3],
    pub alpha_cutoff: f32,
    pub texture_color_space: ColorSpace,
}
impl LitMaterial {
    pub fn textured(texture_view: Handle) -> Self {
        Self {
            base: Material::textured(texture_view),
            shading: ShadingModel::BlinnPhong,
            specular: [0.2; 3],
            shininess: 32.,
            emissive: [0.; 3],
            alpha_cutoff: 0.,
            texture_color_space: ColorSpace::Linear,
        }
    }
    pub(crate) fn parameters(self) -> [f32; 12] {
        [
            self.specular[0],
            self.specular[1],
            self.specular[2],
            self.shininess,
            self.emissive[0],
            self.emissive[1],
            self.emissive[2],
            self.alpha_cutoff,
            match self.shading {
                ShadingModel::Unlit => 0.,
                ShadingModel::Diffuse => 1.,
                ShadingModel::BlinnPhong => 2.,
            },
            if self.texture_color_space == ColorSpace::Srgb {
                1.
            } else {
                0.
            },
            0.,
            0.,
        ]
    }
    fn valid(self) -> bool {
        positive_rgb(self.specular)
            && positive_rgb([self.base.tint[0], self.base.tint[1], self.base.tint[2]])
            && (0. ..=1.).contains(&self.base.tint[3])
            && positive_rgb(self.emissive)
            && self.shininess.is_finite()
            && (1. ..=8192.).contains(&self.shininess)
            && self.alpha_cutoff.is_finite()
            && (0. ..=1.).contains(&self.alpha_cutoff)
    }
}
pub struct LitDraw3D<'a> {
    pub mesh: &'a Mesh,
    pub model: Mat4,
    pub material: LitMaterial,
}

/// Pass load/clear policy plus the caller-declared prior depth attachment state.
/// This permits depth clearing between world and weapon passes without discarding
/// synchronization with earlier depth writes in the same submission.
#[derive(Clone, Copy, Debug)]
pub struct LitPassOptions {
    pub pass: PassOptions,
    pub depth_before: TextureUsageState,
}
impl Default for LitPassOptions {
    fn default() -> Self {
        PassOptions::default().into()
    }
}
impl From<PassOptions> for LitPassOptions {
    fn from(pass: PassOptions) -> Self {
        Self {
            depth_before: if pass.clear_depth {
                TextureUsageState::Undefined
            } else {
                TextureUsageState::DepthStencilAttachment
            },
            pass,
        }
    }
}
/// Opt-in renderer with the same GAL command/lifetime contract as RenderState3D.
/// No window, scene, ECS, frame coordinator, or implicit submission is created.
pub struct LitRenderState3D {
    core: RenderState3D,
    max_lights: usize,
}
pub(crate) struct LightingFrame<'a> {
    pub bytes: Vec<u8>,
    pub materials: &'a [LitMaterial],
    pub depth_before: TextureUsageState,
}
impl LitRenderState3D {
    /// A fixed per-slot light budget, 0..=4096. Overflow is an explicit error.
    pub fn new(gal: &mut VulkanicGal, max_lights: usize) -> GalResult<Self> {
        Self::with_fragment_shader(gal, max_lights, DEFAULT_LIT_FRAGMENT_SHADER)
    }
    /// Custom GLSL fragment shader using the documented lit shader interface.
    /// Allows game-defined BRDF/style without modifying GAL or adopting scene data.
    pub fn with_fragment_shader(
        gal: &mut VulkanicGal,
        max_lights: usize,
        fragment: &str,
    ) -> GalResult<Self> {
        if max_lights > 4096 {
            return Err(GalError::invalid_argument("light budget exceeds 4096"));
        }
        let capabilities = gal.capabilities();
        if !capabilities.features.storage_buffers
            || !capabilities.features.uniform_buffers
            || !capabilities.features.dynamic_buffer_offsets
            || capabilities.limits.max_resource_layout_bindings < 4
            || (48 + max_lights.max(1) * 64) as u64 > capabilities.limits.max_buffer_size
        {
            return Err(GalError::unsupported_feature(
                "lit renderer needs storage/uniform buffers, dynamic offsets and four bindings within backend limits",
            ));
        }
        Ok(Self {
            core: RenderState3D::new_configured(gal, Some(fragment), 48 + max_lights.max(1) * 64)?,
            max_lights,
        })
    }
    pub fn prepare(
        &mut self,
        gal: &mut VulkanicGal,
        target: Handle,
        camera: LitCamera3D,
        lighting: Lighting<'_>,
        draws: &[LitDraw3D<'_>],
        options: impl Into<LitPassOptions>,
    ) -> GalResult<Prepared3D> {
        let options = options.into();
        if draws.len() > 4096 {
            return Err(GalError::invalid_argument("lit draw budget exceeds 4096"));
        }
        if lighting.output_color_space == ColorSpace::Srgb
            && draws.iter().any(|d| d.material.base.transparent)
        {
            return Err(GalError::invalid_argument(
                "blend transparent materials in a linear target; encode the final composite separately",
            ));
        }
        if !matches!(
            options.depth_before,
            TextureUsageState::Undefined | TextureUsageState::DepthStencilAttachment
        ) {
            return Err(GalError::invalid_argument(
                "depth_before must be Undefined or DepthStencilAttachment; transition custom prior uses explicitly",
            ));
        }
        if !camera.position.iter().all(|x| x.is_finite())
            || !positive_rgb(lighting.ambient)
            || lighting.lights.len() > self.max_lights
            || draws
                .iter()
                .any(|d| !d.material.valid() || !invertible_affine(d.model))
        {
            return Err(GalError::invalid_argument(
                "invalid lit camera/material/model or light budget exceeded",
            ));
        }
        let mut values = vec![
            lighting.ambient[0],
            lighting.ambient[1],
            lighting.ambient[2],
            0.,
            camera.position[0],
            camera.position[1],
            camera.position[2],
            0.,
            lighting.lights.len() as f32,
            if lighting.output_color_space == ColorSpace::Srgb {
                1.
            } else {
                0.
            },
            0.,
            0.,
        ];
        for light in lighting.lights {
            values.extend(light.encoded()?);
        }
        let bytes: Vec<u8> = values.into_iter().flat_map(f32::to_ne_bytes).collect();
        let materials: Vec<_> = draws.iter().map(|d| d.material).collect();
        let base: Vec<_> = draws
            .iter()
            .map(|d| Draw3D {
                mesh: d.mesh,
                model: d.model,
                material: d.material.base,
            })
            .collect();
        self.core.prepare_frame(
            gal,
            target,
            Camera3D {
                view_projection: camera.view_projection,
                light_direction: [0., 1., 0.],
            },
            &base,
            options.pass,
            Some(&LightingFrame {
                bytes,
                materials: &materials,
                depth_before: options.depth_before,
            }),
        )
    }
    pub fn clear_bindings(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.core.clear_bindings(gal)
    }
    pub fn release_targets(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.core.release_targets(gal)
    }
    pub fn release(&mut self, gal: &mut VulkanicGal) -> GalResult<()> {
        self.core.release(gal)
    }
}
fn positive_rgb(rgb: [f32; 3]) -> bool {
    rgb.iter().all(|x| x.is_finite() && *x >= 0.)
}
fn invertible_affine(m: Mat4) -> bool {
    let a = m.0;
    let determinant = a[0][0] * (a[1][1] * a[2][2] - a[2][1] * a[1][2])
        - a[1][0] * (a[0][1] * a[2][2] - a[2][1] * a[0][2])
        + a[2][0] * (a[0][1] * a[1][2] - a[1][1] * a[0][2]);
    m.finite()
        && determinant.is_finite()
        && determinant != 0.0
        && determinant.recip().is_finite()
        && a[0][3] == 0.
        && a[1][3] == 0.
        && a[2][3] == 0.
        && a[3][3] == 1.
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_validation_rejects_invalid_directions_ranges_and_cones() {
        assert!(
            Light::Directional {
                direction: [0.; 3],
                color: [1.; 3],
                intensity: 1.
            }
            .encoded()
            .is_err()
        );
        assert!(
            Light::Point {
                position: [0.; 3],
                color: [1.; 3],
                intensity: 1.,
                range: 0.,
                attenuation: Attenuation::Smooth
            }
            .encoded()
            .is_err()
        );
        assert!(
            Light::Spot {
                position: [0.; 3],
                direction: [0., 0., -1.],
                color: [1.; 3],
                intensity: 1.,
                range: 5.,
                inner_angle: 0.5,
                outer_angle: 0.4,
                attenuation: Attenuation::Smooth
            }
            .encoded()
            .is_err()
        );
    }
    #[test]
    fn normal_transform_requires_invertible_affine_model() {
        assert!(invertible_affine(Mat4::scale(2., 0.5, -3.)));
        assert!(invertible_affine(Mat4::scale(0.0001, 0.0001, 0.0001)));
        assert!(!invertible_affine(Mat4::scale(1., 0., 1.)));
        assert!(!invertible_affine(
            Mat4::perspective(1., 1., 0.1, 10.).unwrap()
        ));
    }
}
