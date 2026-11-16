//! Extractor output: quads, smooth vertices and sinks.
use quasar_math::{Vec2, Vec3};
use quasar_voxel::{Face, Layer};

/// One emitted quad in chunk-local voxel units (voxel `v` spans `[v, v+1)`).
/// Vertices are counter-clockwise seen from the front; triangulate as
/// `0-1-2, 0-2-3`, or `1-2-3, 1-3-0` when [`Self::flipped`] (the AO-correct
/// diagonal).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub positions: [Vec3; 4],
    pub normal: Vec3,
    /// Axis face for axis-aligned quads (enables compact packing).
    pub face: Option<Face>,
    /// Texture coordinates in material-tile units (repeat across merged quads).
    pub uv: [Vec2; 4],
    pub material: u32,
    /// Ambient occlusion per vertex, 0 (darkest) ..= 3 (open).
    pub ao: [u8; 4],
    /// Light per vertex: block light in the low nibble, sky light in the high.
    pub light: [u8; 4],
    /// RGBA8 tint (`0xffffffff` = none).
    pub tint: u32,
    pub flipped: bool,
    /// Size in cells along the quad's two axes (1×1 for per-block faces).
    pub size: [u8; 2],
}

impl Quad {
    pub fn centroid(&self) -> Vec3 {
        (self.positions[0] + self.positions[1] + self.positions[2] + self.positions[3]) * 0.25
    }
    /// Triangle vertex order honouring [`Self::flipped`].
    pub fn triangles(&self) -> [usize; 6] {
        if self.flipped {
            [1, 2, 3, 1, 3, 0]
        } else {
            [0, 1, 2, 0, 2, 3]
        }
    }
}

/// One vertex of a smooth (density) mesh in node-local voxel units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmoothVertex {
    pub position: Vec3,
    pub normal: Vec3,
    /// Dominant and secondary material.
    pub materials: [u16; 2],
    /// Weight of the secondary material, 0..=255.
    pub blend: u8,
}

/// Receives extractor output in deterministic order.
pub trait MeshSink {
    fn quad(&mut self, layer: Layer, quad: &Quad);
    /// Smooth geometry: `indices` index into `vertices` (triangles).
    fn smooth(&mut self, layer: Layer, vertices: &[SmoothVertex], indices: &[u32]) {
        let _ = (layer, vertices, indices);
    }
}

/// Collects output per layer, in emission order.
#[derive(Clone, Debug, Default)]
pub struct MeshBuffers {
    pub layers: Vec<(Layer, LayerBuffers)>,
}

/// Output of one layer.
#[derive(Clone, Debug, Default)]
pub struct LayerBuffers {
    pub quads: Vec<Quad>,
    pub vertices: Vec<SmoothVertex>,
    pub indices: Vec<u32>,
}

impl MeshBuffers {
    pub fn new() -> Self {
        Self::default()
    }
    /// Empties every layer but keeps allocations.
    pub fn clear(&mut self) {
        for (_, layer) in &mut self.layers {
            layer.quads.clear();
            layer.vertices.clear();
            layer.indices.clear();
        }
    }
    pub fn layer_mut(&mut self, layer: Layer) -> &mut LayerBuffers {
        let index = match self.layers.iter().position(|(l, _)| *l == layer) {
            Some(i) => i,
            None => {
                self.layers.push((layer, LayerBuffers::default()));
                self.layers.sort_by_key(|(l, _)| *l);
                self.layers.iter().position(|(l, _)| *l == layer).unwrap()
            }
        };
        &mut self.layers[index].1
    }
    pub fn layer(&self, layer: Layer) -> Option<&LayerBuffers> {
        self.layers
            .iter()
            .find(|(l, _)| *l == layer)
            .map(|(_, b)| b)
    }
    pub fn quad_count(&self) -> usize {
        self.layers.iter().map(|(_, b)| b.quads.len()).sum()
    }
    pub fn triangle_count(&self) -> usize {
        self.layers
            .iter()
            .map(|(_, b)| b.quads.len() * 2 + b.indices.len() / 3)
            .sum()
    }
    pub fn is_empty(&self) -> bool {
        self.layers
            .iter()
            .all(|(_, b)| b.quads.is_empty() && b.indices.is_empty())
    }
}

impl MeshSink for MeshBuffers {
    fn quad(&mut self, layer: Layer, quad: &Quad) {
        self.layer_mut(layer).quads.push(*quad);
    }
    fn smooth(&mut self, layer: Layer, vertices: &[SmoothVertex], indices: &[u32]) {
        let buffer = self.layer_mut(layer);
        let base = buffer.vertices.len() as u32;
        buffer.vertices.extend_from_slice(vertices);
        buffer.indices.extend(indices.iter().map(|i| i + base));
    }
}

/// Counts output without storing it (benchmarks).
#[derive(Clone, Copy, Debug, Default)]
pub struct CountingSink {
    pub quads: usize,
    pub triangles: usize,
}

impl MeshSink for CountingSink {
    fn quad(&mut self, _: Layer, _: &Quad) {
        self.quads += 1;
        self.triangles += 2;
    }
    fn smooth(&mut self, _: Layer, _: &[SmoothVertex], indices: &[u32]) {
        self.triangles += indices.len() / 3;
    }
}

/// Deterministic bytes of extractor output in emission order (layers in
/// order; every quad field; smooth vertices and indices) for the
/// `quasar_voxel::conformance` harness.
pub fn record_mesh(mesh: &MeshBuffers) -> Vec<u8> {
    let mut out = Vec::new();
    let layer_tag = |layer: &Layer| -> u16 {
        match layer {
            Layer::Opaque => 0,
            Layer::Cutout => 1,
            Layer::Translucent => 2,
            Layer::Custom(c) => 3 + u16::from(*c),
        }
    };
    for (layer, buffers) in &mesh.layers {
        out.extend(layer_tag(layer).to_le_bytes());
        out.extend((buffers.quads.len() as u32).to_le_bytes());
        for q in &buffers.quads {
            for p in q.positions {
                out.extend(p.to_array().iter().flat_map(|v| v.to_bits().to_le_bytes()));
            }
            out.extend(
                q.normal
                    .to_array()
                    .iter()
                    .flat_map(|v| v.to_bits().to_le_bytes()),
            );
            out.push(q.face.map_or(255, |f| f as u8));
            for uv in q.uv {
                out.extend(uv.to_array().iter().flat_map(|v| v.to_bits().to_le_bytes()));
            }
            out.extend(q.material.to_le_bytes());
            out.extend(q.ao);
            out.extend(q.light);
            out.extend(q.tint.to_le_bytes());
            out.push(u8::from(q.flipped));
            out.extend(q.size);
        }
        out.extend((buffers.vertices.len() as u32).to_le_bytes());
        for v in &buffers.vertices {
            out.extend(
                v.position
                    .to_array()
                    .iter()
                    .chain(&v.normal.to_array())
                    .flat_map(|f| f.to_bits().to_le_bytes()),
            );
            out.extend(v.materials[0].to_le_bytes());
            out.extend(v.materials[1].to_le_bytes());
            out.push(v.blend);
        }
        out.extend((buffers.indices.len() as u32).to_le_bytes());
        out.extend(buffers.indices.iter().flat_map(|i| i.to_le_bytes()));
    }
    out
}
