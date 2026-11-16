//! Compact GPU formats and their WGSL decoders.
//!
//! | Format | Size | Use |
//! |---|---|---|
//! | [`PackedQuad`] | 12 bytes per quad | axis-aligned quads on the voxel lattice (greedy and cube faces) |
//! | [`ModelVertex`] | 20 bytes per vertex | arbitrary model quads |
//! | [`PackedSmoothVertex`] | 16 bytes per vertex | density meshes |
//!
//! Positions are chunk/node-local, so `f32` precision never depends on world
//! position. [`WGSL_FORMATS`] holds matching decode functions for renderers
//! that pull vertices from storage buffers; external renderers can include it.
use crate::culled::tangents;
use crate::quad::{LayerBuffers, Quad, SmoothVertex};
use quasar_math::{Vec2, Vec3};
use quasar_voxel::Face;

/// Fixed-point units per voxel for model and smooth positions.
pub const POSITION_SCALE: f32 = 1024.0;
/// Bias (voxels) so positions in `-16..48` fit `u16`.
pub const POSITION_BIAS: f32 = 16.0;
/// Fixed-point units per material tile for model UVs (`0..16` tiles).
pub const UV_SCALE: f32 = 4096.0;

/// Why a quad cannot use [`PackedQuad`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackError {
    NotAxisAligned,
    OffLattice,
    TooLarge,
    MaterialTooLarge,
}

/// 12-byte axis-aligned quad, expanded to four vertices in the shader.
///
/// * word 0: `x:6 y:6 z:6 face:3 (w-1):5 (h-1):5 flip:1` (min corner on the lattice, `0..=32`)
/// * word 1: `material:16 ao:8 (4 × 2 bits) reserved:8`
/// * word 2: light per corner (`block | sky << 4`), corners in canonical order
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[repr(C)]
pub struct PackedQuad(pub [u32; 3]);

/// Canonical corners of an axis quad: `(u0,v0) (u1,v0) (u1,v1) (u0,v1)` with
/// corners 1 and 3 swapped on faces where that order would wind clockwise.
pub fn canonical_corners(face: Face, origin: Vec3, w: f32, h: f32) -> [Vec3; 4] {
    let (ua, va) = tangents(face.axis());
    let corner = |du: f32, dv: f32| {
        let mut p = origin;
        p[ua] += du;
        p[va] += dv;
        p
    };
    let mut c = [
        corner(0.0, 0.0),
        corner(w, 0.0),
        corner(w, h),
        corner(0.0, h),
    ];
    if canonical_swaps(face) {
        c.swap(1, 3);
    }
    c
}

/// Texture coordinates of a packed-quad corner `(du, dv)` along the face's
/// tangent axes (quad size `w × h`). Matches [`crate::cube_quad`]: the image's
/// top row sits at the top of side faces and no face is mirrored when seen
/// from outside, so packed and model paths texture identically.
pub fn packed_uv(face: Face, du: f32, dv: f32, w: f32, h: f32) -> Vec2 {
    match face {
        Face::NegX => Vec2::new(dv, w - du),
        Face::PosX => Vec2::new(h - dv, w - du),
        Face::NegY => Vec2::new(du, h - dv),
        Face::PosY => Vec2::new(du, dv),
        Face::NegZ => Vec2::new(w - du, h - dv),
        Face::PosZ => Vec2::new(du, h - dv),
    }
}

/// Whether the canonical order swaps corners 1 and 3 for `face`.
pub fn canonical_swaps(face: Face) -> bool {
    let (ua, va) = tangents(face.axis());
    let mut u = Vec3::ZERO;
    u[ua] = 1.0;
    let mut v = Vec3::ZERO;
    v[va] = 1.0;
    u.cross(v).dot(face.normal().as_vec3()) < 0.0
}

impl PackedQuad {
    /// Packs an axis-aligned lattice quad, reordering per-corner AO and light
    /// into canonical order.
    pub fn encode(quad: &Quad) -> Result<Self, PackError> {
        let face = quad.face.ok_or(PackError::NotAxisAligned)?;
        let min = quad
            .positions
            .iter()
            .fold(Vec3::splat(f32::INFINITY), |a, p| a.min(*p));
        let max = quad
            .positions
            .iter()
            .fold(Vec3::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
        // Integer round trips (cheaper than `round`, a libm call on
        // baseline x86-64).
        let lattice = |v: Vec3| {
            v.x == v.x as i32 as f32 && v.y == v.y as i32 as f32 && v.z == v.z as i32 as f32
        };
        if !lattice(min) || !lattice(max) {
            return Err(PackError::OffLattice);
        }
        let extent = max - min;
        let axis = face.axis();
        if extent[axis] != 0.0 {
            return Err(PackError::NotAxisAligned);
        }
        let (ua, va) = tangents(axis);
        let (w, h) = (extent[ua], extent[va]);
        if w < 1.0
            || h < 1.0
            || w > 32.0
            || h > 32.0
            || min.min_element() < 0.0
            || min.max_element() > 63.0
        {
            return Err(PackError::TooLarge);
        }
        if quad.material > 0xffff {
            return Err(PackError::MaterialTooLarge);
        }
        let corners = canonical_corners(face, min, w, h);
        let mut ao = [3u8; 4];
        let mut light = [0u8; 4];
        for (i, c) in corners.iter().enumerate() {
            let source = quad
                .positions
                .iter()
                .position(|p| (*p - *c).abs().max_element() < 1e-4)
                .ok_or(PackError::OffLattice)?;
            ao[i] = quad.ao[source] & 3;
            light[i] = quad.light[source];
        }
        let flip = u16::from(ao[0]) + u16::from(ao[2]) < u16::from(ao[1]) + u16::from(ao[3]);
        let w0 = min.x as u32
            | (min.y as u32) << 6
            | (min.z as u32) << 12
            | (face as u32) << 18
            | (w as u32 - 1) << 21
            | (h as u32 - 1) << 26
            | u32::from(flip) << 31;
        let ao_bits = u32::from(ao[0])
            | u32::from(ao[1]) << 2
            | u32::from(ao[2]) << 4
            | u32::from(ao[3]) << 6;
        let w1 = quad.material | ao_bits << 16;
        let w2 = u32::from_le_bytes(light);
        Ok(Self([w0, w1, w2]))
    }
    /// Expands back to a quad (canonical corner order).
    pub fn decode(self) -> Quad {
        let [w0, w1, w2] = self.0;
        let min = Vec3::new(
            (w0 & 63) as f32,
            ((w0 >> 6) & 63) as f32,
            ((w0 >> 12) & 63) as f32,
        );
        let face = Face::from_index(((w0 >> 18) & 7) as usize).unwrap_or(Face::PosY);
        let w = ((w0 >> 21) & 31) as f32 + 1.0;
        let h = ((w0 >> 26) & 31) as f32 + 1.0;
        let positions = canonical_corners(face, min, w, h);
        let (ua, va) = tangents(face.axis());
        let uv = positions.map(|p| packed_uv(face, p[ua] - min[ua], p[va] - min[va], w, h));
        let ao = std::array::from_fn(|i| ((w1 >> (16 + i * 2)) & 3) as u8);
        Quad {
            positions,
            normal: face.normal().as_vec3(),
            face: Some(face),
            uv,
            material: w1 & 0xffff,
            ao,
            light: w2.to_le_bytes(),
            tint: u32::MAX,
            flipped: w0 >> 31 != 0,
            size: [w as u8, h as u8],
        }
    }
}

/// Octahedral encoding of a unit vector into two bytes.
pub fn oct_encode(n: Vec3) -> [u8; 2] {
    let n = n / (n.x.abs() + n.y.abs() + n.z.abs()).max(1e-12);
    let (mut x, mut y) = (n.x, n.y);
    if n.z < 0.0 {
        let (ox, oy) = (x, y);
        x = (1.0 - oy.abs()) * ox.signum();
        y = (1.0 - ox.abs()) * oy.signum();
    }
    let q = |v: f32| ((v * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
    [q(x), q(y)]
}

pub fn oct_decode(e: [u8; 2]) -> Vec3 {
    let x = f32::from(e[0]) / 255.0 * 2.0 - 1.0;
    let y = f32::from(e[1]) / 255.0 * 2.0 - 1.0;
    let z = 1.0 - x.abs() - y.abs();
    let t = (-z).max(0.0);
    Vec3::new(x - t * x.signum(), y - t * y.signum(), z).normalize_or(Vec3::Z)
}

fn fixed(v: f32) -> u32 {
    ((v + POSITION_BIAS) * POSITION_SCALE)
        .round()
        .clamp(0.0, 65535.0) as u32
}
fn unfixed(v: u32) -> f32 {
    v as f32 / POSITION_SCALE - POSITION_BIAS
}

/// 20-byte general model vertex.
///
/// * words 0–1: position `x, y, z` as `u16` fixed point ([`POSITION_SCALE`],
///   [`POSITION_BIAS`]), then an octahedral normal (2 × `u8`)
/// * word 2: UV as `u16` fixed point ([`UV_SCALE`])
/// * word 3: `material:16 ao:8 light:8`
/// * word 4: RGBA8 tint
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[repr(C)]
pub struct ModelVertex(pub [u32; 5]);

impl ModelVertex {
    pub fn encode(
        position: Vec3,
        normal: Vec3,
        uv: Vec2,
        material: u32,
        ao: u8,
        light: u8,
        tint: u32,
    ) -> Self {
        let n = oct_encode(normal);
        let uvq = |v: f32| (v * UV_SCALE).round().clamp(0.0, 65535.0) as u32;
        Self([
            fixed(position.x) | fixed(position.y) << 16,
            fixed(position.z) | u32::from(n[0]) << 16 | u32::from(n[1]) << 24,
            uvq(uv.x) | uvq(uv.y) << 16,
            (material & 0xffff) | u32::from(ao) << 16 | u32::from(light) << 24,
            tint,
        ])
    }
    /// `(position, normal, uv, material, ao, light, tint)`.
    pub fn decode(self) -> (Vec3, Vec3, Vec2, u32, u8, u8, u32) {
        let [a, b, c, d, e] = self.0;
        (
            Vec3::new(unfixed(a & 0xffff), unfixed(a >> 16), unfixed(b & 0xffff)),
            oct_decode([(b >> 16) as u8, (b >> 24) as u8]),
            Vec2::new((c & 0xffff) as f32 / UV_SCALE, (c >> 16) as f32 / UV_SCALE),
            d & 0xffff,
            (d >> 16) as u8,
            (d >> 24) as u8,
            e,
        )
    }
    /// Four vertices of a quad in its own corner order.
    pub fn quad(quad: &Quad) -> [Self; 4] {
        std::array::from_fn(|i| {
            Self::encode(
                quad.positions[i],
                quad.normal,
                quad.uv[i],
                quad.material,
                quad.ao[i],
                quad.light[i],
                quad.tint,
            )
        })
    }
}

/// 16-byte smooth vertex: position (3 × `u16`), octahedral normal, two
/// materials and a blend weight.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[repr(C)]
pub struct PackedSmoothVertex(pub [u32; 4]);

impl PackedSmoothVertex {
    pub fn encode(v: &SmoothVertex) -> Self {
        let n = oct_encode(v.normal);
        Self([
            fixed(v.position.x) | fixed(v.position.y) << 16,
            fixed(v.position.z) | u32::from(n[0]) << 16 | u32::from(n[1]) << 24,
            u32::from(v.materials[0]) | u32::from(v.materials[1]) << 16,
            u32::from(v.blend),
        ])
    }
    pub fn decode(self) -> SmoothVertex {
        let [a, b, c, d] = self.0;
        SmoothVertex {
            position: Vec3::new(unfixed(a & 0xffff), unfixed(a >> 16), unfixed(b & 0xffff)),
            normal: oct_decode([(b >> 16) as u8, (b >> 24) as u8]),
            materials: [(c & 0xffff) as u16, (c >> 16) as u16],
            blend: d as u8,
        }
    }
}

/// One layer encoded for upload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GpuLayer {
    pub packed: Vec<PackedQuad>,
    /// Four vertices per model quad.
    pub model: Vec<ModelVertex>,
    pub smooth: Vec<PackedSmoothVertex>,
    pub smooth_indices: Vec<u32>,
}

impl GpuLayer {
    /// Encodes quads (packed where possible, model vertices otherwise) and
    /// smooth geometry.
    pub fn encode(layer: &LayerBuffers) -> Self {
        let mut out = Self::default();
        out.encode_into(layer);
        out
    }
    pub fn encode_into(&mut self, layer: &LayerBuffers) {
        self.packed.clear();
        self.model.clear();
        self.smooth.clear();
        self.smooth_indices.clear();
        for quad in &layer.quads {
            match PackedQuad::encode(quad) {
                Ok(packed) => self.packed.push(packed),
                Err(_) => self.model.extend(ModelVertex::quad(quad)),
            }
        }
        self.smooth
            .extend(layer.vertices.iter().map(PackedSmoothVertex::encode));
        self.smooth_indices.extend_from_slice(&layer.indices);
    }
    pub fn byte_size(&self) -> usize {
        self.packed.len() * 12
            + self.model.len() * 20
            + self.smooth.len() * 16
            + self.smooth_indices.len() * 4
    }
    pub fn is_empty(&self) -> bool {
        self.packed.is_empty() && self.model.is_empty() && self.smooth_indices.is_empty()
    }
}

/// WGSL decode helpers matching the Rust encoders (include in vertex shaders).
pub const WGSL_FORMATS: &str = r#"
const QV_POSITION_SCALE: f32 = 1024.0;
const QV_POSITION_BIAS: f32 = 16.0;
const QV_UV_SCALE: f32 = 4096.0;

fn qv_oct_decode(e: vec2<u32>) -> vec3<f32> {
    let f = vec2<f32>(e) / 255.0 * 2.0 - 1.0;
    var n = vec3<f32>(f.x, f.y, 1.0 - abs(f.x) - abs(f.y));
    let t = max(-n.z, 0.0);
    n.x = n.x - t * sign(n.x);
    n.y = n.y - t * sign(n.y);
    return normalize(n);
}

fn qv_face_normal(face: u32) -> vec3<f32> {
    var n = vec3<f32>(0.0);
    let axis = face / 2u;
    let s = select(-1.0, 1.0, (face & 1u) == 1u);
    if (axis == 0u) { n.x = s; } else if (axis == 1u) { n.y = s; } else { n.z = s; }
    return n;
}

fn qv_tangents(axis: u32) -> vec2<u32> {
    if (axis == 0u) { return vec2<u32>(1u, 2u); }
    if (axis == 1u) { return vec2<u32>(0u, 2u); }
    return vec2<u32>(0u, 1u);
}

/// Mirrors `packed_uv` in Rust: image top at the top of side faces, never
/// mirrored from outside.
fn qv_packed_uv(face: u32, du: f32, dv: f32, w: f32, h: f32) -> vec2<f32> {
    switch face {
        case 0u: { return vec2<f32>(dv, w - du); }
        case 1u: { return vec2<f32>(h - dv, w - du); }
        case 2u: { return vec2<f32>(du, h - dv); }
        case 3u: { return vec2<f32>(du, dv); }
        case 4u: { return vec2<f32>(w - du, h - dv); }
        default: { return vec2<f32>(du, h - dv); }
    }
}

struct QvQuadVertex {
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    material: u32,
    ao: f32,
    light: vec2<f32>,
};

/// Corner `corner` (0..4) of a packed quad, honouring the canonical order.
fn qv_packed_quad_vertex(w0: u32, w1: u32, w2: u32, corner_in: u32) -> QvQuadVertex {
    let origin = vec3<f32>(f32(w0 & 63u), f32((w0 >> 6u) & 63u), f32((w0 >> 12u) & 63u));
    let face = (w0 >> 18u) & 7u;
    let w = f32(((w0 >> 21u) & 31u) + 1u);
    let h = f32(((w0 >> 26u) & 31u) + 1u);
    let axis = face / 2u;
    let t = qv_tangents(axis);
    var u_dir = vec3<f32>(0.0);
    var v_dir = vec3<f32>(0.0);
    u_dir[t.x] = 1.0;
    v_dir[t.y] = 1.0;
    let normal = qv_face_normal(face);
    let swap = dot(cross(u_dir, v_dir), normal) < 0.0;
    var corner = corner_in;
    if (swap && (corner == 1u || corner == 3u)) { corner = 4u - corner; }
    let du = select(0.0, w, corner == 1u || corner == 2u);
    let dv = select(0.0, h, corner == 2u || corner == 3u);
    var out: QvQuadVertex;
    out.position = origin + u_dir * du + v_dir * dv;
    out.normal = normal;
    out.uv = qv_packed_uv(face, du, dv, w, h);
    out.material = w1 & 0xffffu;
    out.ao = f32((w1 >> (16u + corner_in * 2u)) & 3u) / 3.0;
    let l = (w2 >> (corner_in * 8u)) & 255u;
    out.light = vec2<f32>(f32(l & 15u), f32(l >> 4u)) / 15.0;
    return out;
}

/// Index of the corner for `vertex_index` (6 per quad), honouring the flip bit.
fn qv_quad_corner(w0: u32, vertex: u32) -> u32 {
    let normal_order = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let flipped_order = array<u32, 6>(1u, 2u, 3u, 1u, 3u, 0u);
    if ((w0 >> 31u) != 0u) { return flipped_order[vertex]; }
    return normal_order[vertex];
}

fn qv_unfix(v: u32) -> f32 { return f32(v) / QV_POSITION_SCALE - QV_POSITION_BIAS; }

fn qv_model_vertex(a: u32, b: u32, c: u32, d: u32, e: u32) -> QvQuadVertex {
    var out: QvQuadVertex;
    out.position = vec3<f32>(qv_unfix(a & 0xffffu), qv_unfix(a >> 16u), qv_unfix(b & 0xffffu));
    out.normal = qv_oct_decode(vec2<u32>((b >> 16u) & 255u, b >> 24u));
    out.uv = vec2<f32>(f32(c & 0xffffu), f32(c >> 16u)) / QV_UV_SCALE;
    out.material = d & 0xffffu;
    out.ao = f32((d >> 16u) & 255u) / 3.0;
    let l = d >> 24u;
    out.light = vec2<f32>(f32(l & 15u), f32(l >> 4u)) / 15.0;
    return out;
}

struct QvSmoothVertex {
    position: vec3<f32>,
    normal: vec3<f32>,
    material0: u32,
    material1: u32,
    blend: f32,
};

fn qv_smooth_vertex(a: u32, b: u32, c: u32, d: u32) -> QvSmoothVertex {
    var out: QvSmoothVertex;
    out.position = vec3<f32>(qv_unfix(a & 0xffffu), qv_unfix(a >> 16u), qv_unfix(b & 0xffffu));
    out.normal = qv_oct_decode(vec2<u32>((b >> 16u) & 255u, b >> 24u));
    out.material0 = c & 0xffffu;
    out.material1 = c >> 16u;
    out.blend = f32(d & 255u) / 255.0;
    return out;
}
"#;
