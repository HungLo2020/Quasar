//! CPU mesh builders for common primitives.
//!
//! Every builder returns [`MeshData`]: indexed triangles with outward normals,
//! counter-clockwise front faces (the renderer's `FrontFace::Ccw`) and UVs in
//! `[0, 1]`. Shapes are centred on the origin with +Y up, following
//! `quasar-math`'s conventions. Upload with [`MeshData::upload`] or pass the
//! vectors to [`Mesh::upload`] directly; the data stays usable for physics,
//! picking or merging without a GPU.
use crate::{Mat4, Mesh, Vertex};
use quasar_frame::{Gpu, GraphicsResult};
use std::f32::consts::{PI, TAU};

/// Indexed triangle data ready for [`Mesh::upload`].
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

fn vertex(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Vertex {
    Vertex {
        position,
        normal,
        uv,
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 0.0 {
        [v[0] / length, v[1] / length, v[2] / length]
    } else {
        [0.0, 1.0, 0.0]
    }
}

impl MeshData {
    /// Axis-aligned box with the given half extents; 24 vertices so each
    /// face has flat normals and its own `[0, 1]` UVs.
    pub fn cuboid(half_extents: [f32; 3]) -> Self {
        let [x, y, z] = half_extents;
        // (normal, four corners counter-clockwise seen from outside)
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            (
                [0., 0., 1.],
                [[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]],
            ),
            (
                [0., 0., -1.],
                [[x, -y, -z], [-x, -y, -z], [-x, y, -z], [x, y, -z]],
            ),
            (
                [1., 0., 0.],
                [[x, -y, z], [x, -y, -z], [x, y, -z], [x, y, z]],
            ),
            (
                [-1., 0., 0.],
                [[-x, -y, -z], [-x, -y, z], [-x, y, z], [-x, y, -z]],
            ),
            (
                [0., 1., 0.],
                [[-x, y, z], [x, y, z], [x, y, -z], [-x, y, -z]],
            ),
            (
                [0., -1., 0.],
                [[-x, -y, -z], [x, -y, -z], [x, -y, z], [-x, -y, z]],
            ),
        ];
        let uvs = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
        let mut data = Self::default();
        for (normal, corners) in faces {
            let base = data.vertices.len() as u32;
            for (position, uv) in corners.into_iter().zip(uvs) {
                data.vertices.push(vertex(position, normal, uv));
            }
            data.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        data
    }

    /// Unit cube from −1 to 1, the shape Quasar's examples use.
    pub fn cube() -> Self {
        Self::cuboid([1.0; 3])
    }

    /// Horizontal plane at y = 0 facing +Y, split into `subdivisions`²
    /// quads (at least one).
    pub fn plane(half_x: f32, half_z: f32, subdivisions: u32) -> Self {
        let n = subdivisions.max(1);
        let mut data = Self::default();
        for row in 0..=n {
            for column in 0..=n {
                let u = column as f32 / n as f32;
                let v = row as f32 / n as f32;
                data.vertices.push(vertex(
                    [-half_x + 2.0 * half_x * u, 0.0, -half_z + 2.0 * half_z * v],
                    [0.0, 1.0, 0.0],
                    [u, v],
                ));
            }
        }
        let stride = n + 1;
        for row in 0..n {
            for column in 0..n {
                let a = row * stride + column;
                let b = a + stride;
                data.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        data
    }

    /// UV sphere with `rings` latitude bands (≥ 2) and `sectors` longitude
    /// slices (≥ 3).
    pub fn sphere(radius: f32, rings: u32, sectors: u32) -> Self {
        let rings = rings.max(2);
        let sectors = sectors.max(3);
        let mut data = Self::default();
        for ring in 0..=rings {
            let v = ring as f32 / rings as f32;
            let (sin_lat, cos_lat) = (PI * v).sin_cos();
            for sector in 0..=sectors {
                let u = sector as f32 / sectors as f32;
                let (sin_lon, cos_lon) = (TAU * u).sin_cos();
                let normal = [sin_lat * cos_lon, cos_lat, -sin_lat * sin_lon];
                data.vertices.push(vertex(
                    [normal[0] * radius, normal[1] * radius, normal[2] * radius],
                    normal,
                    [u, v],
                ));
            }
        }
        let row = sectors + 1;
        for ring in 0..rings {
            for sector in 0..sectors {
                let a = ring * row + sector;
                let b = a + row;
                if ring != 0 {
                    data.indices.extend([a, b, a + 1]);
                }
                if ring != rings - 1 {
                    data.indices.extend([a + 1, b, b + 1]);
                }
            }
        }
        data
    }

    /// Closed cylinder along Y from `-half_height` to `half_height`.
    pub fn cylinder(radius: f32, half_height: f32, segments: u32) -> Self {
        Self::frustum(radius, radius, half_height, segments)
    }

    /// Closed cone along Y with its base at `-height / 2` and apex at
    /// `height / 2`.
    pub fn cone(radius: f32, height: f32, segments: u32) -> Self {
        Self::frustum(radius, 0.0, height * 0.5, segments)
    }

    /// Truncated cone (cylinder when radii match, cone when `top` is 0).
    pub fn frustum(bottom: f32, top: f32, half_height: f32, segments: u32) -> Self {
        let segments = segments.max(3);
        let mut data = Self::default();
        let slope = (bottom - top) / (2.0 * half_height).max(f32::EPSILON);
        // Side: duplicate the seam column so UVs wrap cleanly.
        for (radius, y, v) in [(bottom, -half_height, 1.0), (top, half_height, 0.0)] {
            for segment in 0..=segments {
                let u = segment as f32 / segments as f32;
                let (sin, cos) = (TAU * u).sin_cos();
                let normal = normalize([cos, slope, -sin]);
                data.vertices
                    .push(vertex([cos * radius, y, -sin * radius], normal, [u, v]));
            }
        }
        let row = segments + 1;
        for segment in 0..segments {
            let a = segment;
            let b = a + row;
            if bottom > 0.0 {
                data.indices.extend([a, a + 1, b]);
            }
            if top > 0.0 {
                data.indices.extend([a + 1, b + 1, b]);
            }
        }
        for (radius, y, ny) in [(bottom, -half_height, -1.0f32), (top, half_height, 1.0)] {
            if radius <= 0.0 {
                continue;
            }
            let center = data.vertices.len() as u32;
            data.vertices
                .push(vertex([0.0, y, 0.0], [0.0, ny, 0.0], [0.5, 0.5]));
            for segment in 0..=segments {
                let (sin, cos) = (TAU * segment as f32 / segments as f32).sin_cos();
                data.vertices.push(vertex(
                    [cos * radius, y, -sin * radius],
                    [0.0, ny, 0.0],
                    [0.5 + cos * 0.5, 0.5 + sin * 0.5],
                ));
            }
            for segment in 0..segments {
                let a = center + 1 + segment;
                if ny > 0.0 {
                    data.indices.extend([center, a, a + 1]);
                } else {
                    data.indices.extend([center, a + 1, a]);
                }
            }
        }
        data
    }

    /// Applies `matrix` to positions and its inverse transpose to normals.
    /// Mirroring matrices flip winding so faces stay counter-clockwise.
    pub fn transformed(mut self, matrix: Mat4) -> Self {
        let glam = matrix.to_glam();
        let normal_matrix = glam.inverse().transpose();
        for v in &mut self.vertices {
            v.position = glam
                .transform_point3(quasar_math::Vec3::from(v.position))
                .to_array();
            v.normal = normal_matrix
                .transform_vector3(quasar_math::Vec3::from(v.normal))
                .normalize_or(quasar_math::Vec3::Y)
                .to_array();
        }
        if glam.determinant() < 0.0 {
            for triangle in self.indices.chunks_exact_mut(3) {
                triangle.swap(1, 2);
            }
        }
        self
    }

    /// Appends another mesh, offsetting its indices.
    pub fn append(&mut self, other: &MeshData) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&other.vertices);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }

    /// Axis-aligned bounds `(min, max)`, or `None` when empty.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = self.vertices.first()?.position;
        Some(self.vertices.iter().fold((first, first), |(lo, hi), v| {
            (
                std::array::from_fn(|a| lo[a].min(v.position[a])),
                std::array::from_fn(|a| hi[a].max(v.position[a])),
            )
        }))
    }

    pub fn upload(&self, gpu: &Gpu) -> GraphicsResult<Mesh> {
        Mesh::upload(gpu, &self.vertices, &self.indices)
    }
}

impl From<MeshData> for (Vec<Vertex>, Vec<u32>) {
    fn from(data: MeshData) -> Self {
        (data.vertices, data.indices)
    }
}
