//! A scene "look": the sun, ambient light, fog, toon bands, colour grade,
//! bloom, vignette, god rays and ink edges of one place or time of day, as
//! one value that can be stored per level or planet and blended.
//!
//! ```
//! use quasar_render_3d::{Fog, SceneLook};
//!
//! let day = SceneLook::default();
//! let dusk = SceneLook {
//!     sun_color: [1.0, 0.55, 0.3],
//!     sun_intensity: 0.6,
//!     ambient: [0.05, 0.04, 0.08],
//!     fog: Some(Fog::distance([0.3, 0.2, 0.25], 20.0, 120.0)),
//!     ..SceneLook::default()
//! };
//! let evening = day.lerp(&dusk, 0.5);
//! let lights = evening.lights_with_sun(&[]); // sun first, then your lights
//! let lighting = evening.lighting(&lights);
//! let post = evening.post_settings();
//! assert_eq!(lighting.lights.len(), 1);
//! assert!(post.validate().is_ok());
//! ```
use super::*;

/// See the module docs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLook {
    /// Direction the sun's rays travel.
    pub sun_direction: [f32; 3],
    /// Linear RGB.
    pub sun_color: [f32; 3],
    /// 0 turns the sun off (caves, night).
    pub sun_intensity: f32,
    pub ambient: [f32; 3],
    pub fog: Option<Fog>,
    pub toon: ToonSettings,
    pub exposure: f32,
    pub tonemap: Tonemap,
    pub bloom: BloomSettings,
    pub grade: ColorGrade,
    pub vignette: VignetteSettings,
    /// Screen-space shafts from the sun; the source follows `sun_direction`.
    pub god_rays: Option<GodRays>,
    pub edges: Option<EdgeSettings>,
}

impl Default for SceneLook {
    fn default() -> Self {
        let post = PostSettings::default();
        Self {
            sun_direction: [-0.3, -1.0, -0.4],
            sun_color: [1.0, 0.96, 0.9],
            sun_intensity: 1.0,
            ambient: [0.08, 0.09, 0.11],
            fog: None,
            toon: ToonSettings::default(),
            exposure: 1.0,
            tonemap: Tonemap::Aces,
            bloom: post.bloom,
            grade: post.grade,
            vignette: post.vignette,
            god_rays: None,
            edges: None,
        }
    }
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| mix(a[i], b[i], t))
}
fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| mix(a[i], b[i], t))
}
/// Discrete settings switch half way.
fn pick<T: Copy>(a: T, b: T, t: f32) -> T {
    if t < 0.5 { a } else { b }
}

fn mix_fog(a: Option<Fog>, b: Option<Fog>, t: f32) -> Option<Fog> {
    // Fading to or from no fog thins the fog rather than snapping.
    let clear = |f: Fog| Fog {
        height: f.height.map(|h| HeightFog { density: 0., ..h }),
        scattering: 0.,
        start: f.start.max(1e4),
        end: if f.end.is_finite() {
            f.end.max(1e4 + 1.)
        } else {
            f.end
        },
        ..f
    };
    let (a, b) = match (a, b) {
        (None, None) => return None,
        (Some(a), None) => (a, clear(a)),
        (None, Some(b)) => (clear(b), b),
        (Some(a), Some(b)) => (a, b),
    };
    let height = match (a.height, b.height) {
        (Some(x), Some(y)) => Some(HeightFog {
            base: mix(x.base, y.base, t),
            density: mix(x.density, y.density, t),
            falloff: mix(x.falloff, y.falloff, t),
        }),
        (Some(x), None) => Some(HeightFog {
            density: x.density * (1. - t),
            ..x
        }),
        (None, Some(y)) => Some(HeightFog {
            density: y.density * t,
            ..y
        }),
        (None, None) => None,
    };
    let end = match (a.end.is_finite(), b.end.is_finite()) {
        (true, true) => mix(a.end, b.end, t),
        _ => pick(a.end, b.end, t),
    };
    let start = mix(a.start, b.start, t);
    Some(Fog {
        color: mix3(a.color, b.color, t),
        start: if end > start { start } else { end - 1. },
        end,
        height,
        scattering: mix(a.scattering, b.scattering, t),
        shafts: pick(a.shafts, b.shafts, t),
    })
}

impl SceneLook {
    /// The sun as a directional light (`None` when its intensity is 0).
    pub fn sun(&self) -> Option<Light> {
        (self.sun_intensity > 0.).then_some(Light::Directional {
            direction: self.sun_direction,
            color: self.sun_color,
            intensity: self.sun_intensity,
        })
    }
    /// The sun (if any) followed by `lights`, ready for [`Self::lighting`].
    /// Shadow records referencing `lights` shift by one when the sun is on.
    pub fn lights_with_sun(&self, lights: &[Light]) -> Vec<Light> {
        self.sun()
            .into_iter()
            .chain(lights.iter().copied())
            .collect()
    }
    /// Per-pass lighting with this look's ambient.
    pub fn lighting<'a>(&self, lights: &'a [Light]) -> Lighting<'a> {
        Lighting {
            ambient: self.ambient,
            lights,
            output_color_space: ColorSpace::Linear,
        }
    }
    /// Post-chain settings for this look.
    pub fn post_settings(&self) -> PostSettings {
        PostSettings {
            exposure: self.exposure,
            tonemap: self.tonemap,
            bloom: self.bloom,
            vignette: self.vignette,
            grade: self.grade,
            ..PostSettings::default()
        }
    }
    /// God rays aimed along the sun.
    pub fn sun_rays(&self) -> Option<GodRays> {
        self.god_rays.map(|rays| GodRays {
            source: GodRaySource::Direction(self.sun_direction),
            ..rays
        })
    }
    /// Sets this look's fog and toon bands on a surface renderer.
    pub fn apply(&self, state: &mut SurfaceRenderState3D) -> GraphicsResult<()> {
        state.set_fog(self.fog)?;
        state.set_toon(self.toon)
    }
    /// The look `t` (0..1) of the way to `other`. Colours, intensities and
    /// distances blend; discrete choices (tonemapper, bands, shaft steps,
    /// bloom levels) switch half way.
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        // Exact endpoints, so a finished transition is exactly the target.
        if t.is_nan() || t <= 0. {
            return *self;
        }
        if t >= 1. {
            return *other;
        }
        let a = self;
        let b = other;
        let direction = mix3(a.sun_direction, b.sun_direction, t);
        let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
        Self {
            sun_direction: if length > 1e-4 {
                direction
            } else {
                b.sun_direction
            },
            sun_color: mix3(a.sun_color, b.sun_color, t),
            sun_intensity: mix(a.sun_intensity, b.sun_intensity, t),
            ambient: mix3(a.ambient, b.ambient, t),
            fog: mix_fog(a.fog, b.fog, t),
            toon: ToonSettings {
                bands: pick(a.toon.bands, b.toon.bands, t),
                softness: mix(a.toon.softness, b.toon.softness, t),
                rim: mix(a.toon.rim, b.toon.rim, t),
            },
            exposure: mix(a.exposure, b.exposure, t),
            tonemap: pick(a.tonemap, b.tonemap, t),
            bloom: BloomSettings {
                enabled: pick(a.bloom.enabled, b.bloom.enabled, t),
                threshold: mix(a.bloom.threshold, b.bloom.threshold, t),
                knee: mix(a.bloom.knee, b.bloom.knee, t),
                intensity: mix(a.bloom.intensity, b.bloom.intensity, t),
                levels: pick(a.bloom.levels, b.bloom.levels, t),
            },
            grade: ColorGrade {
                saturation: mix(a.grade.saturation, b.grade.saturation, t),
                contrast: mix(a.grade.contrast, b.grade.contrast, t),
                gain: mix3(a.grade.gain, b.grade.gain, t),
                lift: mix3(a.grade.lift, b.grade.lift, t),
            },
            vignette: VignetteSettings {
                strength: mix(a.vignette.strength, b.vignette.strength, t),
                radius: mix(a.vignette.radius, b.vignette.radius, t),
                softness: mix(a.vignette.softness, b.vignette.softness, t),
            },
            god_rays: match (a.god_rays, b.god_rays) {
                (Some(x), Some(y)) => Some(GodRays {
                    source: pick(x.source, y.source, t),
                    color: mix3(x.color, y.color, t),
                    intensity: mix(x.intensity, y.intensity, t),
                    decay: mix(x.decay, y.decay, t),
                    density: mix(x.density, y.density, t),
                    samples: pick(x.samples, y.samples, t),
                }),
                (Some(x), None) => Some(GodRays {
                    intensity: x.intensity * (1. - t),
                    ..x
                }),
                (None, Some(y)) => Some(GodRays {
                    intensity: y.intensity * t,
                    ..y
                }),
                (None, None) => None,
            },
            edges: match (a.edges, b.edges) {
                (Some(x), Some(y)) => Some(EdgeSettings {
                    color: mix4(x.color, y.color, t),
                    width: mix(x.width, y.width, t),
                    depth_threshold: mix(x.depth_threshold, y.depth_threshold, t),
                    crease_threshold: mix(x.crease_threshold, y.crease_threshold, t),
                    fade_distance: mix(x.fade_distance, y.fade_distance, t),
                }),
                (Some(x), None) => Some(EdgeSettings {
                    color: [x.color[0], x.color[1], x.color[2], x.color[3] * (1. - t)],
                    ..x
                }),
                (None, Some(y)) => Some(EdgeSettings {
                    color: [y.color[0], y.color[1], y.color[2], y.color[3] * t],
                    ..y
                }),
                (None, None) => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_blend_and_stay_valid() {
        let cave = SceneLook {
            sun_intensity: 0.,
            ambient: [0.02; 3],
            fog: Some(Fog::height(
                [0.2, 0.25, 0.3],
                HeightFog {
                    base: 0.,
                    density: 0.4,
                    falloff: 0.8,
                },
            )),
            edges: Some(EdgeSettings::default()),
            ..SceneLook::default()
        };
        let surface = SceneLook {
            fog: Some(Fog::distance([0.6, 0.7, 0.9], 30., 200.)),
            god_rays: Some(GodRays::default()),
            ..SceneLook::default()
        };
        assert!(cave.sun().is_none());
        assert_eq!(surface.lights_with_sun(&[]).len(), 1);
        for i in 0..=10 {
            let t = i as f32 / 10.;
            let look = cave.lerp(&surface, t);
            look.post_settings().validate().unwrap();
            let lights = look.lights_with_sun(&[]);
            encode_lighting_with(
                LitCamera3D {
                    view_projection: Mat4::IDENTITY,
                    position: [0.; 3],
                },
                look.lighting(&lights),
                4,
                &LightingOptions {
                    fog: look.fog,
                    toon: look.toon,
                    ..LightingOptions::default()
                },
            )
            .unwrap();
            if let Some(rays) = look.sun_rays() {
                assert_eq!(rays.source, GodRaySource::Direction(look.sun_direction));
            }
        }
        assert_eq!(cave.lerp(&surface, 0.), cave);
        assert_eq!(cave.lerp(&surface, 1.), surface);
        let half = cave.lerp(&surface, 0.5);
        assert!((half.ambient[0] - 0.05).abs() < 1e-6);
        assert!(half.edges.unwrap().color[3] < 1.);
    }
}
