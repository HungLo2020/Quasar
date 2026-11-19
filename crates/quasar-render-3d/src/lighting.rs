//! Opt-in forward lighting. All inputs are ordinary caller-owned values/slices.
use super::*;

/// Default lit fragment source, also a starting point for custom shader variants.
pub const DEFAULT_LIT_FRAGMENT_SHADER: &str = concat!(
    "diagnostic(off, derivative_uniformity);\n",
    include_str!("lighting.wgsl"),
    include_str!("surface_body.frag.wgsl")
);

/// Shared WGSL lighting module: `Light`/`Lights`/`Shadow`/`Shadows`
/// records at their reserved group-0 bindings (3 lights, 5 shadow depth
/// array, 6 shadow records, 21 comparison sampler), `shadow_visibility`,
/// `lit_color`, `safe_normal`, sRGB helpers and `output_srgb`. Other
/// renderers include it (after any `diagnostic` directive) and bind
/// [`encode_lighting`] / [`ShadowBinding::records_bytes`] data so they light
/// identically to render-3d meshes.
pub const LIGHTING_WGSL: &str = include_str!("lighting.wgsl");
/// Group-0 binding of the lights storage buffer in [`LIGHTING_WGSL`].
pub const LIGHTS_BINDING: u32 = 3;
/// Group-0 binding of the shadow depth array.
pub const SHADOW_TEXTURE_BINDING: u32 = 5;
/// Group-0 binding of the shadow record storage buffer.
pub const SHADOW_RECORDS_BINDING: u32 = 6;
/// Group-0 binding of the shadow comparison sampler.
pub const SHADOW_SAMPLER_BINDING: u32 = 21;

/// Fog shared by meshes and voxels: distance fog towards the eye, optional
/// exponential height fog, and optional light scattering. Colours are
/// linear; distances are metres from the camera position the lighting was
/// encoded with.
///
/// - Distance fog blends from `start` to `end` metres (`end` infinite turns
///   it off, for height-only fog).
/// - [`HeightFog`] adds haze whose density falls off with world height, so
///   it pools on floors and thins over high ground. It is integrated
///   analytically along each view ray, so it shows from a top-down camera
///   where distance fog cannot.
/// - `scattering` (0 = off) adds light scattered by the fog: point and
///   spot lights glow through it (their cones and cookies included) and
///   directional lights brighten it. It is single scattering with an
///   isotropic phase function, proportional to the local fog density, so 1
///   is physically plausible and larger values stylise. `shafts` > 0
///   ray-marches that many shadow-map samples along each view ray for
///   directional lights, so light through openings forms volumetric
///   shafts. Positional lights are taken from the fragment's light cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    /// Linear RGB.
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
    pub height: Option<HeightFog>,
    pub scattering: f32,
    pub shafts: u32,
}

/// Exponential height fog: `density` (per metre) at world height `base`,
/// multiplied by `exp(-falloff * metres above base)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightFog {
    pub base: f32,
    pub density: f32,
    pub falloff: f32,
}

impl Fog {
    /// Plain distance fog.
    pub const fn distance(color: [f32; 3], start: f32, end: f32) -> Self {
        Self {
            color,
            start,
            end,
            height: None,
            scattering: 0.,
            shafts: 0,
        }
    }
    /// Height fog only (no distance fog).
    pub const fn height(color: [f32; 3], height: HeightFog) -> Self {
        Self {
            color,
            start: 0.,
            end: f32::INFINITY,
            height: Some(height),
            scattering: 0.,
            shafts: 0,
        }
    }
    pub const fn with_height(mut self, height: HeightFog) -> Self {
        self.height = Some(height);
        self
    }
    /// Light scattering strength (1 = physically plausible).
    pub const fn with_scattering(mut self, scattering: f32) -> Self {
        self.scattering = scattering;
        self
    }
    /// Shadow-map samples per view ray for directional-light shafts
    /// (8 to 24 is typical; 0 = off). Needs scattering.
    pub const fn with_shafts(mut self, steps: u32) -> Self {
        self.shafts = steps;
        self
    }
    /// Rebased for camera-relative positions (for example voxel rendering
    /// around `origin`): the height base moves with the camera height.
    pub fn rebased(mut self, origin_height: f32) -> Self {
        if let Some(height) = &mut self.height {
            height.base -= origin_height;
        }
        self
    }
    fn valid(&self) -> bool {
        let height_ok = self.height.is_none_or(|h| {
            h.base.is_finite()
                && h.density.is_finite()
                && h.density >= 0.
                && h.falloff.is_finite()
                && h.falloff >= 0.
        });
        positive_rgb(self.color)
            && self.start.is_finite()
            && !self.end.is_nan()
            && self.start >= 0.
            && self.end > self.start
            && height_ok
            && self.scattering.is_finite()
            && self.scattering >= 0.
            && self.shafts <= 64
    }
}

/// Toon (cel) shading settings for [`ShadingModel::Toon`] materials.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToonSettings {
    /// Light bands (2 to 4 is typical).
    pub bands: u32,
    /// Band edge softness, 0 (hard) to 1.
    pub softness: f32,
    /// Rim light strength at silhouettes (0 = off).
    pub rim: f32,
}

impl Default for ToonSettings {
    fn default() -> Self {
        Self {
            bands: 3,
            softness: 0.08,
            rim: 0.25,
        }
    }
}

impl ToonSettings {
    fn valid(&self) -> bool {
        (1..=16).contains(&self.bands)
            && (0. ..=1.).contains(&self.softness)
            && self.rim.is_finite()
            && self.rim >= 0.
    }
}

/// World-space light clustering. Space around the camera is split into a
/// grid of cubic cells; every positional (point or spot) light is listed in
/// the cells its range touches, and each fragment shades only its cell's
/// lights plus every directional light. Fragments outside the grid shade
/// every light, so clustering never changes the image, only its cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightClusters {
    /// Positional lights needed before clustering is used; with fewer,
    /// every fragment loops over all lights (cheaper for small counts).
    pub min_lights: usize,
    /// Cell edge length in metres.
    pub cell_size: f32,
    /// Cells per axis (X, Y, Z), centred on the camera.
    pub dims: [u32; 3],
}

impl Default for LightClusters {
    fn default() -> Self {
        Self {
            min_lights: 12,
            cell_size: 6.,
            dims: [24, 12, 24],
        }
    }
}

impl LightClusters {
    /// Never cluster: every fragment shades every light.
    pub const DISABLED: Self = Self {
        min_lights: usize::MAX,
        cell_size: 6.,
        dims: [1, 1, 1],
    };
    /// Upper bound on cells (`dims` product).
    pub const MAX_CELLS: u64 = 1 << 16;

    fn valid(&self) -> bool {
        self.cell_size.is_finite()
            && self.cell_size > 0.
            && self.dims.iter().all(|d| (1..=256).contains(d))
            && self.dims.iter().map(|&d| u64::from(d)).product::<u64>() <= Self::MAX_CELLS
    }
}

/// Per-pass options of the shared lighting encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightingOptions {
    pub fog: Option<Fog>,
    pub clusters: LightClusters,
    /// Band settings for [`ShadingModel::Toon`] materials in this pass.
    pub toon: ToonSettings,
}

/// CPU light grid built by [`cluster_lights`]: the exact lists the shader
/// reads, for tests, tools and debug views.
#[derive(Clone, Debug, PartialEq)]
pub struct LightGrid {
    /// Minimum corner of the grid.
    pub origin: [f32; 3],
    pub cell_size: f32,
    pub dims: [u32; 3],
    /// Directional lights, shaded everywhere.
    pub globals: Vec<u32>,
    /// `(start, count)` into `indices` per cell, X fastest then Y then Z.
    pub cells: Vec<(u32, u32)>,
    pub indices: Vec<u32>,
}

impl LightGrid {
    /// The cell holding `point`, or `None` outside the grid.
    pub fn cell_of(&self, point: [f32; 3]) -> Option<usize> {
        let mut index = 0usize;
        let mut stride = 1usize;
        for ((p, origin), dim) in point.iter().zip(self.origin).zip(self.dims) {
            let cell = ((p - origin) / self.cell_size).floor();
            if !(cell >= 0. && cell < dim as f32) {
                return None;
            }
            index += cell as usize * stride;
            stride *= dim as usize;
        }
        Some(index)
    }
    /// Positional lights listed for `point` (directional lights excluded),
    /// or `None` outside the grid.
    pub fn lights_at(&self, point: [f32; 3]) -> Option<&[u32]> {
        let (start, count) = self.cells[self.cell_of(point)?];
        Some(&self.indices[start as usize..(start + count) as usize])
    }
}

/// Builds the world-space light grid centred on `center`, or `None` when
/// clustering is disabled for this light count. Light indices refer to
/// `lights` (shadow records keep referencing the same indices).
pub fn cluster_lights(
    lights: &[Light],
    center: [f32; 3],
    clusters: &LightClusters,
) -> GraphicsResult<Option<LightGrid>> {
    if !clusters.valid() || !center.iter().all(|v| v.is_finite()) {
        return Err(error("invalid light cluster settings"));
    }
    let positional = lights
        .iter()
        .filter(|l| !matches!(l, Light::Directional { .. }))
        .count();
    if positional < clusters.min_lights.max(1) {
        return Ok(None);
    }
    let size = clusters.cell_size;
    let dims = clusters.dims;
    let origin: [f32; 3] =
        std::array::from_fn(|a| (center[a] / size).floor() * size - (dims[a] / 2) as f32 * size);
    let cell_count = dims.iter().map(|&d| d as usize).product::<usize>();
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); cell_count];
    let mut globals = Vec::new();
    for (index, light) in lights.iter().enumerate() {
        let (position, range) = match *light {
            Light::Directional { .. } => {
                globals.push(index as u32);
                continue;
            }
            Light::Point {
                position, range, ..
            }
            | Light::Spot {
                position, range, ..
            } => (position, range),
        };
        let mut lo = [0usize; 3];
        let mut hi = [0usize; 3];
        let mut outside = false;
        for a in 0..3 {
            let min = ((position[a] - range - origin[a]) / size).floor();
            let max = ((position[a] + range - origin[a]) / size).floor();
            if max < 0. || min >= dims[a] as f32 {
                outside = true;
                break;
            }
            lo[a] = min.max(0.) as usize;
            hi[a] = (max as usize).min(dims[a] as usize - 1);
        }
        if outside {
            continue;
        }
        let range_squared = range * range;
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    // Sphere-box test: distance from the light to the cell.
                    let cell = [x, y, z];
                    let distance_squared: f32 = (0..3)
                        .map(|a| {
                            let min = origin[a] + cell[a] as f32 * size;
                            let d = (min - position[a]).max(position[a] - (min + size)).max(0.);
                            d * d
                        })
                        .sum();
                    if distance_squared < range_squared {
                        let flat = x + dims[0] as usize * (y + dims[1] as usize * z);
                        lists[flat].push(index as u32);
                    }
                }
            }
        }
    }
    let mut cells = Vec::with_capacity(cell_count);
    let mut indices = Vec::new();
    for list in lists {
        cells.push((indices.len() as u32, list.len() as u32));
        indices.extend(list);
    }
    Ok(Some(LightGrid {
        origin,
        cell_size: size,
        dims,
        globals,
        cells,
        indices,
    }))
}

/// Encodes the lights storage buffer read by [`LIGHTING_WGSL`] with default
/// [`LightingOptions`] (no fog, automatic clustering).
pub fn encode_lighting(
    camera: LitCamera3D,
    lighting: Lighting<'_>,
    max_lights: usize,
) -> GraphicsResult<Vec<u8>> {
    encode_lighting_with(camera, lighting, max_lights, &LightingOptions::default())
}

/// Floats in the lights header before the light array.
const LIGHT_HEADER_FLOATS: usize = 36;

/// [`encode_lighting`] with explicit fog and clustering options.
pub fn encode_lighting_with(
    camera: LitCamera3D,
    lighting: Lighting<'_>,
    max_lights: usize,
    options: &LightingOptions,
) -> GraphicsResult<Vec<u8>> {
    if !camera.position.iter().all(|x| x.is_finite())
        || !positive_rgb(lighting.ambient)
        || lighting.lights.len() > max_lights
    {
        return Err(error("invalid lighting/camera or light budget exceeded"));
    }
    if options.fog.is_some_and(|f| !f.valid()) {
        return Err(error(
            "fog needs a non-negative colour, 0 <= start < end, non-negative height density/falloff and scattering, and at most 64 shaft steps",
        ));
    }
    if !options.toon.valid() {
        return Err(error(
            "toon shading needs 1..=16 bands, softness 0..=1 and rim >= 0",
        ));
    }
    let grid = cluster_lights(lighting.lights, camera.position, &options.clusters)?;
    let fog = options.fog;
    let fog_color = fog.map_or([0.; 3], |f| f.color);
    let mut values = Vec::with_capacity(LIGHT_HEADER_FLOATS + 16 * (lighting.lights.len() + 1));
    values.extend([
        lighting.ambient[0],
        lighting.ambient[1],
        lighting.ambient[2],
        0.,
        camera.position[0],
        camera.position[1],
        camera.position[2],
        0.,
        lighting.lights.len() as f32,
        f32::from(lighting.output_color_space == ColorSpace::Srgb),
        f32::from(grid.is_some()),
        0.,
        fog_color[0],
        fog_color[1],
        fog_color[2],
        f32::from(fog.is_some()),
        fog.map_or(0., |f| f.start),
        fog.map_or(1., |f| {
            if f.end.is_finite() {
                f.end
            } else {
                f.start + 1.
            }
        }),
        f32::from(fog.is_some_and(|f| f.end.is_finite())),
        0.,
    ]);
    let height = fog.and_then(|f| f.height);
    values.extend([
        height.map_or(0., |h| h.base),
        height.map_or(0., |h| h.density),
        height.map_or(0., |h| h.falloff),
        fog.map_or(0., |f| f.scattering),
        options.toon.bands as f32,
        options.toon.softness,
        options.toon.rim,
        fog.map_or(0., |f| {
            if f.scattering > 0. {
                f.shafts as f32
            } else {
                0.
            }
        }),
    ]);
    match &grid {
        Some(grid) => values.extend([
            grid.origin[0],
            grid.origin[1],
            grid.origin[2],
            grid.cell_size,
            grid.dims[0] as f32,
            grid.dims[1] as f32,
            grid.dims[2] as f32,
            lighting.lights.len() as f32,
        ]),
        None => values.extend([0., 0., 0., 1., 0., 0., 0., 0.]),
    }
    for light in lighting.lights {
        values.extend(light.encoded()?);
    }
    if let Some(grid) = grid {
        let globals = grid.globals.len() as u32;
        let lists = 1 + globals + 2 * grid.cells.len() as u32;
        let mut words: Vec<u32> = Vec::with_capacity((lists as usize) + grid.indices.len());
        words.push(globals);
        words.extend(&grid.globals);
        for (start, count) in &grid.cells {
            words.push(lists + start);
            words.push(*count);
        }
        words.extend(&grid.indices);
        words.resize(words.len().next_multiple_of(16), 0);
        values.extend(words.into_iter().map(f32::from_bits));
    }
    if values.len() == LIGHT_HEADER_FLOATS {
        values.resize(LIGHT_HEADER_FLOATS + 16, 0.);
    }
    Ok(bytes(values))
}

/// Lights buffer for a pass without lights (the shader still needs one
/// record to bind).
pub fn empty_lighting_bytes() -> Vec<u8> {
    vec![0; (LIGHT_HEADER_FLOATS + 16) * 4]
}

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
    /// Cel shading: light quantised into [`ToonSettings::bands`] with a
    /// hard specular highlight and an optional rim.
    Toon,
}

impl ShadingModel {
    /// The value lit shaders read (0 unlit, 1 diffuse, 2 Blinn-Phong,
    /// 3 toon).
    pub const fn code(self) -> f32 {
        match self {
            Self::Unlit => 0.,
            Self::Diffuse => 1.,
            Self::BlinnPhong => 2.,
            Self::Toon => 3.,
        }
    }
}

/// How positional light falls off towards its range.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Attenuation {
    /// `(1 - d²/range²)²`.
    #[default]
    Smooth,
    /// Physically based `1/d²`, windowed to the range.
    InverseSquare,
    /// `(1 - d/range)^exponent`: 1 is linear, larger values hug the light
    /// (a flare), smaller ones spread an even pool (a lantern).
    Curve { exponent: f32 },
}

/// A procedural pattern projected by a spot light (a "cookie"), for
/// headlamps with rings, grates, star flares and dappled light. Animate it
/// by changing `rotation` or the light's intensity each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightCookie {
    pub pattern: CookiePattern,
    /// Rings, bars or spokes across the cone (1 to 255), or the dapple
    /// noise frequency.
    pub count: u8,
    /// 0 (no pattern) to 1 (pattern fully dark between features).
    pub contrast: f32,
    /// Pattern rotation about the spot axis, in radians.
    pub rotation: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CookiePattern {
    #[default]
    None,
    /// Concentric rings (a reflector headlamp).
    Rings,
    /// Parallel bars (a grate or window).
    Bars,
    /// Radial spokes (a star flare).
    Spokes,
    /// Smooth noise (light through leaves).
    Dapple,
}

impl LightCookie {
    pub const NONE: Self = Self {
        pattern: CookiePattern::None,
        count: 0,
        contrast: 0.,
        rotation: 0.,
    };
    pub const fn new(pattern: CookiePattern, count: u8, contrast: f32) -> Self {
        Self {
            pattern,
            count,
            contrast,
            rotation: 0.,
        }
    }
    pub const fn with_rotation(mut self, rotation: f32) -> Self {
        self.rotation = rotation;
        self
    }
    fn valid(&self) -> bool {
        self.contrast.is_finite()
            && (0. ..=1.).contains(&self.contrast)
            && self.rotation.is_finite()
    }
    /// Packed into the light record: pattern (4 bits), count (8), contrast
    /// (8), rotation (12).
    fn packed(&self) -> u32 {
        let pattern = match self.pattern {
            CookiePattern::None => return 0,
            CookiePattern::Rings => 1,
            CookiePattern::Bars => 2,
            CookiePattern::Spokes => 3,
            CookiePattern::Dapple => 4,
        };
        if self.contrast <= 0. {
            return 0;
        }
        let contrast = (self.contrast * 255.).round() as u32;
        let turns = self.rotation.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
        let rotation = ((turns * 4096.) as u32).min(4095);
        pattern | u32::from(self.count) << 4 | contrast << 12 | rotation << 20
    }
    /// The cookie's light multiplier at `uv` (the cone's cross-section,
    /// `[-1, 1]` from axis to edge), matching the shader.
    pub fn sample(&self, uv: [f32; 2]) -> f32 {
        let packed = self.packed();
        if packed == 0 {
            return 1.;
        }
        let count = f32::from(self.count);
        let contrast = ((packed >> 12) & 255) as f32 / 255.;
        let angle = (packed >> 20) as f32 / 4096. * std::f32::consts::TAU;
        let (s, c) = angle.sin_cos();
        let uv = [uv[0] * c - uv[1] * s, uv[0] * s + uv[1] * c];
        let tau = std::f32::consts::TAU;
        let pattern = match packed & 15 {
            1 => 0.5 + 0.5 * ((uv[0] * uv[0] + uv[1] * uv[1]).sqrt() * count * tau).cos(),
            2 => 0.5 + 0.5 * (uv[0] * count * std::f32::consts::PI).cos(),
            3 => 0.5 + 0.5 * (uv[1].atan2(uv[0]) * count).cos(),
            _ => dapple([uv[0] * count, uv[1] * count]),
        };
        1. - contrast + contrast * pattern
    }
}

fn dapple(p: [f32; 2]) -> f32 {
    let hash = |x: f32, y: f32| {
        let h = (x * 127.1 + y * 311.7).sin() * 43_758.547;
        h - h.floor()
    };
    let (ix, iy) = (p[0].floor(), p[1].floor());
    let (fx, fy) = (p[0] - ix, p[1] - iy);
    let (sx, sy) = (fx * fx * (3. - 2. * fx), fy * fy * (3. - 2. * fy));
    let a = hash(ix, iy);
    let b = hash(ix + 1., iy);
    let c = hash(ix, iy + 1.);
    let d = hash(ix + 1., iy + 1.);
    let top = a + (b - a) * sx;
    top + (c + (d - c) * sx - top) * sy
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
        /// Projected pattern; [`LightCookie::NONE`] for a plain cone.
        cookie: LightCookie,
    },
}
impl Light {
    pub(crate) fn encoded(self) -> GraphicsResult<[f32; 16]> {
        let mut cookie = LightCookie::NONE;
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
                    cookie: spot_cookie,
                } => {
                    if !spot_cookie.valid() {
                        return Err(error("spot cookie needs 0 <= contrast <= 1"));
                    }
                    cookie = spot_cookie;
                    if !inner_angle.is_finite()
                        || !outer_angle.is_finite()
                        || inner_angle < 0.
                        || outer_angle <= inner_angle
                        || outer_angle >= std::f32::consts::FRAC_PI_2
                    {
                        return Err(error("spot half-angles require 0 <= inner < outer < pi/2"));
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
            return Err(error(
                "invalid light position/direction/color/intensity/range",
            ));
        }
        if let Attenuation::Curve { exponent } = attenuation
            && !(exponent.is_finite() && (0.01..=64.).contains(&exponent))
        {
            return Err(error("attenuation curve exponent must be in 0.01..=64"));
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
            match attenuation {
                Attenuation::Smooth => 0.,
                Attenuation::InverseSquare => 1.,
                Attenuation::Curve { exponent } => 2. + exponent,
            },
            f32::from_bits(cookie.packed()),
        ])
    }
}
#[derive(Clone, Copy, Debug)]
pub struct LitCamera3D {
    pub view_projection: Mat4,
    pub position: [f32; 3],
}
impl LitCamera3D {
    /// See [`crate::Camera3D::project`].
    pub fn project(&self, world: [f32; 3], viewport: quasar_math::Viewport) -> Option<[f32; 3]> {
        self.view_projection.project(world, viewport)
    }
    /// See [`crate::Camera3D::unproject`].
    pub fn unproject(&self, screen: [f32; 3], viewport: quasar_math::Viewport) -> Option<[f32; 3]> {
        self.view_projection.unproject(screen, viewport)
    }
    /// See [`crate::Camera3D::pick_ray`].
    pub fn pick_ray(
        &self,
        screen: [f32; 2],
        viewport: quasar_math::Viewport,
    ) -> Option<([f32; 3], [f32; 3])> {
        self.view_projection.screen_ray(screen, viewport)
    }
    /// See [`crate::Camera3D::screen_ray`].
    #[cfg(feature = "visibility")]
    pub fn screen_ray(
        &self,
        screen: [f32; 2],
        viewport: quasar_math::Viewport,
    ) -> Option<quasar_spatial::Ray3> {
        crate::screen_ray_spatial(self.view_projection, screen, viewport)
    }
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
#[derive(Clone, Debug)]
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
    pub fn textured(texture_view: &TextureView) -> Self {
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
    pub(crate) fn parameters(&self) -> [f32; 12] {
        [
            self.specular[0],
            self.specular[1],
            self.specular[2],
            self.shininess,
            self.emissive[0],
            self.emissive[1],
            self.emissive[2],
            self.alpha_cutoff,
            self.shading.code(),
            if self.texture_color_space == ColorSpace::Srgb {
                1.
            } else {
                0.
            },
            0.,
            0.,
        ]
    }
    pub(crate) fn valid(&self) -> bool {
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

pub(crate) fn positive_rgb(rgb: [f32; 3]) -> bool {
    rgb.iter().all(|x| x.is_finite() && *x >= 0.)
}
pub(crate) fn invertible_affine(m: Mat4) -> bool {
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
                attenuation: Attenuation::Smooth,
                cookie: LightCookie::NONE,
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

/// Native pass load/clear choices; wgpu owns attachment synchronization.
#[derive(Clone, Copy, Debug, Default)]
pub struct LitPassOptions {
    pub pass: PassOptions,
}
impl From<PassOptions> for LitPassOptions {
    fn from(pass: PassOptions) -> Self {
        Self { pass }
    }
}
pub struct LitRenderState3D {
    pub(crate) core: Core3D,
    pub(crate) max_lights: usize,
    pub(crate) options: LightingOptions,
}
impl LitRenderState3D {
    pub fn new(gpu: &Gpu, max_lights: usize) -> GraphicsResult<Self> {
        Self::with_submission(gpu, max_lights, SubmissionConfig::default())
    }
    pub fn with_submission(
        gpu: &Gpu,
        max_lights: usize,
        submission: SubmissionConfig,
    ) -> GraphicsResult<Self> {
        if max_lights > 4096 {
            return Err(error("light budget exceeds 4096"));
        }
        Ok(Self {
            core: Core3D::new(gpu, true, submission, None)?,
            max_lights,
            options: LightingOptions::default(),
        })
    }
    /// A WGSL fragment entry point using the documented native surface binding layout.
    pub fn with_fragment_shader(
        gpu: &Gpu,
        max_lights: usize,
        fragment: &str,
    ) -> GraphicsResult<Self> {
        if max_lights > 4096 {
            return Err(error("light budget exceeds 4096"));
        }
        Ok(Self {
            core: Core3D::new(gpu, true, SubmissionConfig::default(), Some(fragment))?,
            max_lights,
            options: LightingOptions::default(),
        })
    }
    pub(crate) fn light_bytes(
        &self,
        camera: LitCamera3D,
        lighting: Lighting<'_>,
    ) -> GraphicsResult<Vec<u8>> {
        encode_lighting_with(camera, lighting, self.max_lights, &self.options)
    }
    /// Fog for every later pass of this state (opaque and transparent):
    /// distance, height and scattering (see [`Fog`]). Matches the voxel
    /// renderer's fog so meshes fade into the same haze as blocks.
    pub fn set_fog(&mut self, fog: Option<Fog>) -> GraphicsResult<()> {
        if fog.is_some_and(|f| !f.valid()) {
            return Err(error(
                "fog needs a non-negative colour, 0 <= start < end, valid height fog and scattering",
            ));
        }
        self.options.fog = fog;
        Ok(())
    }
    /// Band settings for [`ShadingModel::Toon`] materials.
    pub fn set_toon(&mut self, toon: ToonSettings) -> GraphicsResult<()> {
        if !toon.valid() {
            return Err(error(
                "toon shading needs 1..=16 bands, softness 0..=1 and rim >= 0",
            ));
        }
        self.options.toon = toon;
        Ok(())
    }
    pub fn toon(&self) -> ToonSettings {
        self.options.toon
    }
    pub fn fog(&self) -> Option<Fog> {
        self.options.fog
    }
    /// Light clustering used once a pass has enough positional lights.
    pub fn set_light_clusters(&mut self, clusters: LightClusters) -> GraphicsResult<()> {
        if !clusters.valid() {
            return Err(error("invalid light cluster settings"));
        }
        self.options.clusters = clusters;
        Ok(())
    }
    pub fn light_clusters(&self) -> LightClusters {
        self.options.clusters
    }
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        target: &RenderTarget,
        camera: LitCamera3D,
        lighting: Lighting<'_>,
        draws: &[LitDraw3D<'_>],
        options: impl Into<LitPassOptions>,
    ) -> GraphicsResult<Prepared3D> {
        if draws.iter().any(|d| !d.material.valid()) {
            return Err(error("invalid lit material"));
        }
        if lighting.output_color_space == ColorSpace::Srgb
            && (target.format.is_srgb() || draws.iter().any(|d| d.material.base.transparent))
        {
            return Err(error(
                "use a linear target for blending; encode sRGB only once",
            ));
        }
        let data = self.light_bytes(camera, lighting)?;
        let input: Vec<_> = draws
            .iter()
            .map(|d| Input {
                mesh: d.mesh,
                model: d.model,
                base: &d.material.base,
                lit: Some(&d.material),
                normal: None,
                emissive: None,
                receives_shadows: false,
                blend: None,
            })
            .collect();
        self.core.prepare(
            gpu,
            target,
            Camera3D {
                view_projection: camera.view_projection,
                light_direction: [0., 1., 0.],
            },
            &input,
            options.into().pass,
            Some(&data),
            None,
        )
    }
    pub fn set_texture_sampler(
        &mut self,
        _gpu: &Gpu,
        view: &TextureView,
        sampler: &Sampler,
    ) -> GraphicsResult<()> {
        self.core.samplers.insert(view.clone(), sampler.clone());
        Ok(())
    }
    pub fn work_stats(&self) -> RenderWork {
        self.core.work
    }
    #[cfg(feature = "diagnostics")]
    pub fn set_diagnostics(&mut self, d: quasar_diagnostics::Diagnostics) {
        self.core.diagnostics = d;
    }
    pub fn clear_bindings(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.core.groups.clear();
        self.core.samplers.clear();
        self.core.normal_samplers.clear();
        Ok(())
    }
    pub fn release_targets(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.core.depths.clear();
        Ok(())
    }
    pub fn release(&mut self, _gpu: &Gpu) -> GraphicsResult<()> {
        self.core.release();
        Ok(())
    }
}
