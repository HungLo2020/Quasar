//! Policy-driven per-block model extraction (the parity-capable path).
//!
//! Every output-deciding rule is a trait so a game can reproduce a reference
//! mesher exactly:
//!
//! * [`ModelSource`]: the quads a block emits (in order) for an orientation.
//! * [`FacePolicy`]: whether a quad with a cull face is hidden by its neighbour.
//! * [`ShadeSource`]: per-vertex ambient occlusion and light.
//! * [`LayerRouter`]: which render layer receives each quad.
//! * [`IterationOrder`]: the cell visit order.
//!
//! Quasar's defaults (driven by a [`BlockCatalog`] and [`ModelLibrary`]) give
//! standard voxel-game results with 4-level AO and smooth light.
use crate::quad::{MeshSink, Quad};
use quasar_math::{IVec3, Vec2, Vec3};
use quasar_voxel::{AIR, BlockCatalog, Face, FaceShape, Layer, Orientation, Snapshot};

/// One quad of a block model in block space (`0..1` per axis).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelQuad {
    pub positions: [Vec3; 4],
    pub uv: [Vec2; 4],
    /// Face whose neighbour may hide this quad (`None` = always drawn).
    pub cull: Option<Face>,
    /// Axis face this quad lies on, for AO and compact packing.
    pub face: Option<Face>,
    pub material: u32,
    /// Apply ambient occlusion and smooth light.
    pub shade: bool,
    pub tint: u32,
}

impl ModelQuad {
    pub fn normal(&self) -> Vec3 {
        let p = &self.positions;
        (p[1] - p[0]).cross(p[3] - p[0]).normalize_or_zero()
    }
    /// The quad rotated about the block centre.
    pub fn rotated(&self, orientation: Orientation) -> Self {
        if orientation == Orientation::IDENTITY {
            return *self;
        }
        let m = orientation.matrix();
        let rotate = |p: Vec3| {
            let c = p - Vec3::splat(0.5);
            Vec3::new(
                m[0][0] as f32 * c.x + m[0][1] as f32 * c.y + m[0][2] as f32 * c.z,
                m[1][0] as f32 * c.x + m[1][1] as f32 * c.y + m[1][2] as f32 * c.z,
                m[2][0] as f32 * c.x + m[2][1] as f32 * c.y + m[2][2] as f32 * c.z,
            ) + Vec3::splat(0.5)
        };
        Self {
            positions: self.positions.map(rotate),
            cull: self.cull.map(|f| orientation.rotate_face(f)),
            face: self.face.map(|f| orientation.rotate_face(f)),
            ..*self
        }
    }
}

/// The four corners of a unit-cube face, counter-clockwise from outside.
pub fn cube_face(face: Face) -> [Vec3; 4] {
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    match face {
        Face::NegX => [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)],
        Face::PosX => [v(1., 0., 1.), v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.)],
        Face::NegY => [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
        Face::PosY => [v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.), v(0., 1., 0.)],
        Face::NegZ => [v(1., 0., 0.), v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)],
        Face::PosZ => [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
    }
}

const FACE_UV: [Vec2; 4] = [
    Vec2::new(0., 1.),
    Vec2::new(1., 1.),
    Vec2::new(1., 0.),
    Vec2::new(0., 0.),
];

/// A unit-cube face as a model quad.
pub fn cube_quad(face: Face, material: u32) -> ModelQuad {
    ModelQuad {
        positions: cube_face(face),
        uv: FACE_UV,
        cull: Some(face),
        face: Some(face),
        material,
        shade: true,
        tint: u32::MAX,
    }
}

/// A box (`min..max` in block space) as model quads, culled on faces that
/// touch the block boundary.
pub fn box_quads(min: Vec3, max: Vec3, materials: [u32; 6]) -> Vec<ModelQuad> {
    Face::ALL
        .into_iter()
        .map(|face| {
            let mut q = cube_quad(face, materials[face.index()]);
            q.positions = q.positions.map(|p| min + p * (max - min));
            let on_boundary = if face.is_positive() {
                max[face.axis()] >= 1.0
            } else {
                min[face.axis()] <= 0.0
            };
            q.cull = on_boundary.then_some(face);
            q
        })
        .collect()
}

/// Custom block models referenced by [`quasar_voxel::BlockDef::model`].
#[derive(Clone, Debug, Default)]
pub struct ModelLibrary {
    models: Vec<Vec<ModelQuad>>,
}

impl ModelLibrary {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, quads: Vec<ModelQuad>) -> u32 {
        self.models.push(quads);
        (self.models.len() - 1) as u32
    }
    pub fn get(&self, model: u32) -> Option<&[ModelQuad]> {
        self.models.get(model as usize).map(Vec::as_slice)
    }
}

/// Quads emitted by a block.
pub trait ModelSource {
    /// Appends `block`'s quads for `orientation` (already rotated) to `out`.
    fn quads(&self, block: u16, orientation: Orientation, out: &mut Vec<ModelQuad>);
}

/// Default models: catalog cubes (with per-face materials) or library models.
pub struct CatalogModels<'a> {
    pub catalog: &'a BlockCatalog,
    pub library: &'a ModelLibrary,
}

impl ModelSource for CatalogModels<'_> {
    fn quads(&self, block: u16, orientation: Orientation, out: &mut Vec<ModelQuad>) {
        let Some(def) = self.catalog.get(block) else {
            return;
        };
        if block == AIR {
            return;
        }
        match self.library.get(def.model) {
            Some(quads) => out.extend(quads.iter().map(|q| q.rotated(orientation))),
            None => {
                for face in Face::ALL {
                    // Material of the local face that ends up facing `face`.
                    let local = orientation.local_face(face);
                    let mut q = cube_quad(face, def.materials[local.index()]);
                    q.tint = u32::MAX;
                    out.push(q);
                }
            }
        }
    }
}

/// A cell seen by policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellInfo {
    pub block: u16,
    pub orientation: Orientation,
}

/// Decides whether a quad culled against `face` is hidden by `neighbour`.
pub trait FacePolicy {
    fn hidden(&self, own: CellInfo, neighbour: CellInfo, face: Face) -> bool;
}

/// Default face culling from catalog face shapes and layers:
/// opaque full faces hide what they touch; matching shaped faces of opaque
/// blocks hide each other; identical non-opaque blocks (glass next to glass)
/// hide their shared face; cutout blocks never hide neighbours.
pub struct CatalogFaces<'a>(pub &'a BlockCatalog);

impl FacePolicy for CatalogFaces<'_> {
    fn hidden(&self, own: CellInfo, neighbour: CellInfo, face: Face) -> bool {
        if neighbour.block == AIR {
            return false;
        }
        let (Some(own_def), Some(n_def)) = (self.0.get(own.block), self.0.get(neighbour.block))
        else {
            return false;
        };
        let own_shape = own_def.faces[own.orientation.local_face(face).index()];
        let n_shape = n_def.faces[neighbour.orientation.local_face(face.opposite()).index()];
        match (n_def.layer, n_shape) {
            (Layer::Opaque, FaceShape::Full) => true,
            (Layer::Opaque, shape) if shape != FaceShape::None && shape == own_shape => true,
            (Layer::Cutout, _) => false,
            (_, FaceShape::Full) => own.block == neighbour.block,
            _ => false,
        }
    }
}

/// Inputs of the shading policy for one cell.
pub struct ShadeInput<'a> {
    pub blocks: &'a Snapshot<u16>,
    pub block_light: Option<&'a Snapshot<u8>>,
    pub sky_light: Option<&'a Snapshot<u8>>,
    /// Per block ID: occludes ambient light (AO) and light sampling.
    pub opaque: &'a [bool],
}

impl ShadeInput<'_> {
    #[inline]
    fn opaque_at(&self, p: IVec3) -> bool {
        let b = self.blocks.at(p.x, p.y, p.z) as usize;
        self.opaque.get(b).copied().unwrap_or(false)
    }
    #[inline]
    fn light_at(&self, p: IVec3) -> (u8, u8) {
        (
            self.block_light.map_or(0, |s| s.at(p.x, p.y, p.z)),
            self.sky_light.map_or(0, |s| s.at(p.x, p.y, p.z)),
        )
    }
}

/// Fills a quad's AO, light and triangulation.
pub trait ShadeSource {
    fn shade(&self, input: &ShadeInput<'_>, cell: IVec3, model: &ModelQuad, quad: &mut Quad);
}

/// Default shading: 4-level vertex AO (side, side, corner) with the
/// anisotropy-correcting diagonal flip, and smooth light averaged over the
/// non-opaque cells around each vertex in front of the face.
#[derive(Clone, Copy, Debug)]
pub struct VertexShade {
    pub ao: bool,
    pub smooth_light: bool,
}

impl Default for VertexShade {
    fn default() -> Self {
        Self {
            ao: true,
            smooth_light: true,
        }
    }
}

/// Tangent axes `(u, v)` of a face axis.
pub(crate) fn tangents(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    }
}

impl ShadeSource for VertexShade {
    fn shade(&self, input: &ShadeInput<'_>, cell: IVec3, model: &ModelQuad, quad: &mut Quad) {
        let Some(face) = model.face.filter(|_| model.shade) else {
            let (b, s) = input.light_at(cell);
            quad.light = [b | (s << 4); 4];
            quad.ao = [3; 4];
            return;
        };
        let n = face.normal();
        // Quads flush with the cell boundary sample the neighbour layer;
        // inset quads sample their own cell.
        let flush = model.positions.iter().all(|p| {
            let c = p[face.axis()];
            if face.is_positive() {
                c >= 0.999
            } else {
                c <= 0.001
            }
        });
        let front = if flush { cell + n } else { cell };
        let (ua, va) = tangents(face.axis());
        for (corner, p) in model.positions.iter().enumerate() {
            let du = if p[ua] >= 0.5 { 1 } else { -1 };
            let dv = if p[va] >= 0.5 { 1 } else { -1 };
            let mut side_u = IVec3::ZERO;
            side_u[ua] = du;
            let mut side_v = IVec3::ZERO;
            side_v[va] = dv;
            let s1 = input.opaque_at(front + side_u);
            let s2 = input.opaque_at(front + side_v);
            let c = input.opaque_at(front + side_u + side_v);
            quad.ao[corner] = if !self.ao {
                3
            } else if s1 && s2 {
                0
            } else {
                3 - (u8::from(s1) + u8::from(s2) + u8::from(c))
            };
            let (b, s) = if self.smooth_light {
                let mut sum = (0u32, 0u32);
                let mut count = 0u32;
                for (offset, blocked) in [
                    (IVec3::ZERO, false),
                    (side_u, s1),
                    (side_v, s2),
                    (side_u + side_v, c || (s1 && s2)),
                ] {
                    if !blocked {
                        let (b, s) = input.light_at(front + offset);
                        sum.0 += u32::from(b);
                        sum.1 += u32::from(s);
                        count += 1;
                    }
                }
                ((sum.0 / count.max(1)) as u8, (sum.1 / count.max(1)) as u8)
            } else {
                input.light_at(front)
            };
            quad.light[corner] = (b & 15) | ((s & 15) << 4);
        }
        quad.flipped = u16::from(quad.ao[0]) + u16::from(quad.ao[2])
            < u16::from(quad.ao[1]) + u16::from(quad.ao[3]);
    }
}

/// Picks the render layer of each quad.
pub trait LayerRouter {
    fn layer(&self, block: u16, quad: &ModelQuad) -> Layer;
}

/// Layers from catalog definitions.
pub struct CatalogLayers<'a>(pub &'a BlockCatalog);

impl LayerRouter for CatalogLayers<'_> {
    fn layer(&self, block: u16, _quad: &ModelQuad) -> Layer {
        self.0.get(block).map_or(Layer::Opaque, |d| d.layer)
    }
}

/// Cell visit order: axes from slowest to fastest. Default `Y, Z, X`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IterationOrder(pub [usize; 3]);

impl Default for IterationOrder {
    fn default() -> Self {
        Self([1, 2, 0])
    }
}

/// Snapshots read by the extractor (all with the same padding, ≥ 1).
#[derive(Clone, Copy)]
pub struct CulledInput<'a> {
    pub blocks: &'a Snapshot<u16>,
    pub orientation: Option<&'a Snapshot<u8>>,
    pub block_light: Option<&'a Snapshot<u8>>,
    pub sky_light: Option<&'a Snapshot<u8>>,
    /// Per block ID: occludes AO/light (see [`ShadeInput::opaque`]).
    pub opaque: &'a [bool],
}

/// Counts from one extraction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExtractStats {
    pub quads: usize,
    pub culled: usize,
    pub cells: usize,
}

/// Opacity table from a catalog (index = block ID).
pub fn opaque_table(catalog: &BlockCatalog) -> Vec<bool> {
    catalog.defs().iter().map(|d| d.opaque).collect()
}

/// Per-block model extractor. Reusable; keeps scratch between calls.
#[derive(Debug, Default)]
pub struct CulledModelExtractor {
    pub order: IterationOrder,
    scratch: Vec<ModelQuad>,
}

impl CulledModelExtractor {
    pub fn new(order: IterationOrder) -> Self {
        Self {
            order,
            scratch: Vec::new(),
        }
    }

    /// Extracts every cell for which `include(block)` holds.
    #[allow(clippy::too_many_arguments)]
    pub fn extract<M: ModelSource, F: FacePolicy, S: ShadeSource, R: LayerRouter>(
        &mut self,
        input: CulledInput<'_>,
        models: &M,
        faces: &F,
        shade: &S,
        layers: &R,
        include: impl Fn(u16) -> bool,
        sink: &mut dyn MeshSink,
    ) -> ExtractStats {
        let pad = input.blocks.pad() as i32;
        assert!(pad >= 1, "culled extraction needs a padded snapshot");
        let n = input.blocks.edge() as i32 - 2 * pad;
        let shade_input = ShadeInput {
            blocks: input.blocks,
            block_light: input.block_light,
            sky_light: input.sky_light,
            opaque: input.opaque,
        };
        let mut stats = ExtractStats::default();
        let [slow, mid, fast] = self.order.0;
        let orientation_at =
            |p: IVec3| Orientation(input.orientation.map_or(0, |s| s.at(p.x, p.y, p.z)));
        for a in 0..n {
            for b in 0..n {
                for c in 0..n {
                    let mut cell = IVec3::ZERO;
                    cell[slow] = a;
                    cell[mid] = b;
                    cell[fast] = c;
                    let block = input.blocks.at(cell.x, cell.y, cell.z);
                    if block == AIR || !include(block) {
                        continue;
                    }
                    stats.cells += 1;
                    let own = CellInfo {
                        block,
                        orientation: orientation_at(cell),
                    };
                    self.scratch.clear();
                    models.quads(block, own.orientation, &mut self.scratch);
                    for model in &self.scratch {
                        if let Some(face) = model.cull {
                            let np = cell + face.normal();
                            let neighbour = CellInfo {
                                block: input.blocks.at(np.x, np.y, np.z),
                                orientation: orientation_at(np),
                            };
                            if faces.hidden(own, neighbour, face) {
                                stats.culled += 1;
                                continue;
                            }
                        }
                        let offset = cell.as_vec3();
                        let mut quad = Quad {
                            positions: model.positions.map(|p| p + offset),
                            normal: model.normal(),
                            face: model.face,
                            uv: model.uv,
                            material: model.material,
                            ao: [3; 4],
                            light: [0; 4],
                            tint: model.tint,
                            flipped: false,
                            size: [1, 1],
                        };
                        shade.shade(&shade_input, cell, model, &mut quad);
                        sink.quad(layers.layer(block, model), &quad);
                        stats.quads += 1;
                    }
                }
            }
        }
        stats
    }
}
