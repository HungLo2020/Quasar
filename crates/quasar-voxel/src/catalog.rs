//! Block definitions for block worlds and block grids.
//!
//! A [`BlockCatalog`] describes blocks referenced by a block channel: their
//! (possibly multi-cell) footprint, per-face shapes for culling, mount points
//! for attachment and connectivity, and render data for extractors. Blocks are
//! placed in one of the 24 [`Orientation`]s. Placement and removal produce
//! ordinary [`EditBatch`] operations, so they replicate and replay like any
//! other edit.
use crate::chunk::ChannelId;
use crate::edit::EditBatch;
use crate::layout::{ChunkLayout, Face};
use crate::orient::Orientation;
use crate::volume::Volume;
use quasar_math::IVec3;

/// Block ID 0 is always empty space.
pub const AIR: u16 = 0;

/// Shape of one block face for culling. `Full` hides the neighbour's face
/// when both touch; `Shape(n)` faces only hide `Shape(n)` faces they touch.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FaceShape {
    #[default]
    None,
    Full,
    Shape(u16),
}

/// Render layer of a block's geometry.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Layer {
    #[default]
    Opaque,
    /// Alpha-tested (leaves, grates).
    Cutout,
    /// Blended and sorted (glass, water).
    Translucent,
    /// Game-defined layer.
    Custom(u8),
}

/// One block type.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlockDef {
    pub name: String,
    /// Occupied cells relative to the anchor (always includes the origin).
    pub footprint: Vec<IVec3>,
    /// Face shapes of the block's bounding box in local orientation, indexed
    /// by [`Face`].
    pub faces: [FaceShape; 6],
    /// Faces (bit per [`Face`]) other blocks may attach to.
    pub mounts: u8,
    /// Whether the block fully blocks light and visibility.
    pub opaque: bool,
    /// Whether every cell is a full unit cube (enables greedy meshing).
    pub full_cube: bool,
    pub layer: Layer,
    /// Model ID for model-based extractors (`u32::MAX` = cube faces).
    pub model: u32,
    /// Material per local face for cube extractors.
    pub materials: [u32; 6],
    /// Light emitted (0..=15).
    pub emission: u8,
    pub collision: bool,
}

impl BlockDef {
    /// A solid unit cube with the same material on every face.
    pub fn cube(name: &str, material: u32) -> Self {
        Self {
            name: name.to_owned(),
            footprint: vec![IVec3::ZERO],
            faces: [FaceShape::Full; 6],
            mounts: 0x3f,
            opaque: true,
            full_cube: true,
            layer: Layer::Opaque,
            model: u32::MAX,
            materials: [material; 6],
            emission: 0,
            collision: true,
        }
    }
    /// The empty block.
    pub fn air() -> Self {
        Self {
            name: "air".into(),
            footprint: vec![IVec3::ZERO],
            faces: [FaceShape::None; 6],
            mounts: 0,
            opaque: false,
            full_cube: false,
            layer: Layer::Opaque,
            model: u32::MAX,
            materials: [0; 6],
            emission: 0,
            collision: false,
        }
    }
    pub fn with_layer(mut self, layer: Layer) -> Self {
        self.layer = layer;
        self.opaque = layer == Layer::Opaque && self.full_cube;
        self
    }
    pub fn with_footprint(mut self, footprint: Vec<IVec3>) -> Self {
        self.footprint = footprint;
        self.full_cube = false;
        self
    }
    pub fn with_faces(mut self, faces: [FaceShape; 6]) -> Self {
        self.faces = faces;
        self.full_cube = faces.iter().all(|f| *f == FaceShape::Full) && self.footprint.len() == 1;
        self.opaque &= self.full_cube;
        self
    }
    pub fn with_materials(mut self, materials: [u32; 6]) -> Self {
        self.materials = materials;
        self
    }
    pub fn with_emission(mut self, emission: u8) -> Self {
        self.emission = emission.min(15);
        self
    }
    pub fn with_model(mut self, model: u32) -> Self {
        self.model = model;
        self
    }
    pub fn with_mounts(mut self, mounts: u8) -> Self {
        self.mounts = mounts;
        self
    }
}

/// Channels a catalog reads and writes.
#[derive(Clone, Copy, Debug)]
pub struct BlockChannels {
    pub block: ChannelId<u16>,
    pub orientation: ChannelId<u8>,
    /// Non-anchor cells of multi-cell blocks store an encoded offset to their
    /// anchor here; 0 for anchors and single-cell blocks. Use a sparse channel.
    pub part: ChannelId<u32>,
}

/// Why a placement failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaceError {
    UnknownBlock,
    InvalidOrientation,
    /// A footprint cell is already occupied.
    Occupied(IVec3),
}

const PART_FLAG: u32 = 1 << 30;

fn encode_part(offset: IVec3) -> u32 {
    let e = |v: i32| (v + 512) as u32 & 1023;
    PART_FLAG | e(offset.x) | (e(offset.y) << 10) | (e(offset.z) << 20)
}

fn decode_part(raw: u32) -> Option<IVec3> {
    (raw & PART_FLAG != 0).then(|| {
        let d = |shift: u32| ((raw >> shift) & 1023) as i32 - 512;
        IVec3::new(d(0), d(10), d(20))
    })
}

/// Block definitions indexed by block ID.
#[derive(Clone, Debug)]
pub struct BlockCatalog {
    defs: Vec<BlockDef>,
}

impl Default for BlockCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockCatalog {
    /// A catalog containing only [`AIR`].
    pub fn new() -> Self {
        Self {
            defs: vec![BlockDef::air()],
        }
    }
    /// Adds a block and returns its ID.
    pub fn add(&mut self, def: BlockDef) -> u16 {
        assert!(
            def.footprint.contains(&IVec3::ZERO),
            "footprints include the anchor"
        );
        assert!(
            def.footprint.iter().all(|c| c.abs().max_element() < 512),
            "footprint offsets must be within ±511"
        );
        self.defs.push(def);
        (self.defs.len() - 1) as u16
    }
    pub fn get(&self, id: u16) -> Option<&BlockDef> {
        self.defs.get(id as usize)
    }
    pub fn len(&self) -> usize {
        self.defs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.defs.len() <= 1
    }
    pub fn find(&self, name: &str) -> Option<u16> {
        self.defs
            .iter()
            .position(|d| d.name == name)
            .map(|i| i as u16)
    }
    pub fn defs(&self) -> &[BlockDef] {
        &self.defs
    }
    /// World cells covered by block `id` anchored at `anchor`.
    pub fn footprint(&self, id: u16, orientation: Orientation, anchor: IVec3) -> Vec<IVec3> {
        self.get(id)
            .map(|def| {
                def.footprint
                    .iter()
                    .map(|offset| anchor + orientation.rotate(*offset))
                    .collect()
            })
            .unwrap_or_default()
    }
    /// The anchor cell of whatever block occupies `cell` (itself for
    /// single-cell blocks), or `None` for air.
    pub fn anchor_of<L: ChunkLayout>(
        &self,
        volume: &Volume<L>,
        ch: BlockChannels,
        cell: IVec3,
    ) -> Option<IVec3> {
        if volume.get(ch.block, cell) == AIR {
            return None;
        }
        Some(match decode_part(volume.get(ch.part, cell)) {
            Some(offset) => cell + offset,
            None => cell,
        })
    }
    /// Whether block `id` fits at `anchor` (every footprint cell is air).
    pub fn check_place<L: ChunkLayout>(
        &self,
        volume: &Volume<L>,
        ch: BlockChannels,
        id: u16,
        orientation: Orientation,
        anchor: IVec3,
    ) -> Result<Vec<IVec3>, PlaceError> {
        if id == AIR || self.get(id).is_none() {
            return Err(PlaceError::UnknownBlock);
        }
        if !orientation.is_valid() {
            return Err(PlaceError::InvalidOrientation);
        }
        let cells = self.footprint(id, orientation, anchor);
        if let Some(cell) = cells.iter().find(|c| volume.get(ch.block, **c) != AIR) {
            return Err(PlaceError::Occupied(*cell));
        }
        Ok(cells)
    }
    /// Appends the edits that place block `id`.
    pub fn place<L: ChunkLayout>(
        &self,
        batch: &mut EditBatch,
        volume: &Volume<L>,
        ch: BlockChannels,
        id: u16,
        orientation: Orientation,
        anchor: IVec3,
    ) -> Result<(), PlaceError> {
        let cells = self.check_place(volume, ch, id, orientation, anchor)?;
        for cell in cells {
            batch.set(ch.block, cell, id);
            batch.set(ch.orientation, cell, orientation.0);
            let part = if cell == anchor {
                0
            } else {
                encode_part(anchor - cell)
            };
            batch.set(ch.part, cell, part);
        }
        Ok(())
    }
    /// Appends the edits that remove the block occupying `cell` (all its
    /// footprint cells). Returns the removed block's anchor.
    pub fn remove<L: ChunkLayout>(
        &self,
        batch: &mut EditBatch,
        volume: &Volume<L>,
        ch: BlockChannels,
        cell: IVec3,
    ) -> Option<IVec3> {
        let anchor = self.anchor_of(volume, ch, cell)?;
        let id = volume.get(ch.block, anchor);
        let orientation = Orientation(volume.get(ch.orientation, anchor));
        for c in self.footprint(id, orientation, anchor) {
            batch.set(ch.block, c, AIR);
            batch.set(ch.orientation, c, 0);
            batch.set(ch.part, c, 0);
        }
        Some(anchor)
    }
    /// Shape of the face of `cell`'s block that faces world direction
    /// `face`. Interior faces between cells of one multi-cell block report
    /// `Full` so they cull each other.
    pub fn face_shape<L: ChunkLayout>(
        &self,
        volume: &Volume<L>,
        ch: BlockChannels,
        cell: IVec3,
        face: Face,
    ) -> FaceShape {
        let id = volume.get(ch.block, cell);
        let Some(def) = self.get(id) else {
            return FaceShape::None;
        };
        if id == AIR {
            return FaceShape::None;
        }
        let orientation = Orientation(volume.get(ch.orientation, cell));
        let local = orientation.local_face(face);
        def.faces[local.index()]
    }
    /// Whether `cell`'s block offers a mount on world face `face`.
    pub fn mounts<L: ChunkLayout>(
        &self,
        volume: &Volume<L>,
        ch: BlockChannels,
        cell: IVec3,
        face: Face,
    ) -> bool {
        let id = volume.get(ch.block, cell);
        let Some(def) = self.get(id) else {
            return false;
        };
        let orientation = Orientation(volume.get(ch.orientation, cell));
        def.mounts & orientation.local_face(face).bit() != 0
    }
    /// Whether two face-adjacent cells are structurally connected: parts of
    /// one block, or both mount on the shared face.
    pub fn connected<L: ChunkLayout>(
        &self,
        volume: &Volume<L>,
        ch: BlockChannels,
        a: IVec3,
        face: Face,
    ) -> bool {
        let b = a + face.normal();
        let (Some(anchor_a), Some(anchor_b)) =
            (self.anchor_of(volume, ch, a), self.anchor_of(volume, ch, b))
        else {
            return false;
        };
        anchor_a == anchor_b
            || (self.mounts(volume, ch, a, face) && self.mounts(volume, ch, b, face.opposite()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Schema;
    use crate::edit::EditOptions;
    use crate::layout::Cube16;
    use crate::storage::StorageKind;

    pub(crate) fn grid() -> (Volume<Cube16>, BlockChannels) {
        let mut schema = Schema::new();
        let block = schema.channel("block", StorageKind::Paletted, AIR);
        let orientation = schema.channel("orientation", StorageKind::Paletted, 0u8);
        let part = schema.channel("part", StorageKind::Sparse, 0u32);
        (
            Volume::new(schema),
            BlockChannels {
                block,
                orientation,
                part,
            },
        )
    }

    #[test]
    fn multi_cell_blocks_place_rotate_and_remove_as_a_unit() {
        let (mut volume, ch) = grid();
        let mut catalog = BlockCatalog::new();
        let armor = catalog.add(BlockDef::cube("armor", 1));
        let long = catalog.add(BlockDef::cube("thruster", 2).with_footprint(vec![
            IVec3::ZERO,
            IVec3::Z,
            IVec3::Z * 2,
        ]));
        let mut batch = EditBatch::new();
        catalog
            .place(
                &mut batch,
                &volume,
                ch,
                armor,
                Orientation::IDENTITY,
                IVec3::ZERO,
            )
            .unwrap();
        let turn = Orientation::from_up_forward(Face::PosY, Face::PosX).unwrap();
        catalog
            .place(&mut batch, &volume, ch, long, turn, IVec3::new(1, 0, 0))
            .unwrap();
        volume.apply(&batch, EditOptions::default());
        for x in 1..=3 {
            assert_eq!(
                volume.get(ch.block, IVec3::new(x, 0, 0)),
                long,
                "rotated footprint along +X"
            );
            assert_eq!(
                catalog.anchor_of(&volume, ch, IVec3::new(x, 0, 0)),
                Some(IVec3::new(1, 0, 0))
            );
        }
        assert_eq!(
            catalog.check_place(
                &volume,
                ch,
                armor,
                Orientation::IDENTITY,
                IVec3::new(2, 0, 0)
            ),
            Err(PlaceError::Occupied(IVec3::new(2, 0, 0)))
        );
        assert!(catalog.connected(&volume, ch, IVec3::ZERO, Face::PosX));
        assert!(
            catalog.connected(&volume, ch, IVec3::new(2, 0, 0), Face::PosX),
            "same block"
        );
        let mut removal = EditBatch::new();
        assert_eq!(
            catalog.remove(&mut removal, &volume, ch, IVec3::new(3, 0, 0)),
            Some(IVec3::new(1, 0, 0))
        );
        volume.apply(&removal, EditOptions::default());
        assert!((1..=3).all(|x| volume.get(ch.block, IVec3::new(x, 0, 0)) == AIR));
        assert_eq!(volume.get(ch.block, IVec3::ZERO), armor);
        assert_eq!(
            catalog.check_place(&volume, ch, 99, Orientation::IDENTITY, IVec3::ZERO),
            Err(PlaceError::UnknownBlock)
        );
    }

    #[test]
    fn face_shapes_and_mounts_follow_orientation() {
        let (mut volume, ch) = grid();
        let mut catalog = BlockCatalog::new();
        let mut faces = [FaceShape::Full; 6];
        faces[Face::PosY.index()] = FaceShape::Shape(1); // sloped top
        let slope = catalog.add(
            BlockDef::cube("slope", 1)
                .with_faces(faces)
                .with_mounts(Face::NegY.bit()),
        );
        assert!(!catalog.get(slope).unwrap().full_cube);
        let mut batch = EditBatch::new();
        let flipped = Orientation::from_up_forward(Face::PosX, Face::PosZ).unwrap();
        catalog
            .place(&mut batch, &volume, ch, slope, flipped, IVec3::ZERO)
            .unwrap();
        volume.apply(&batch, EditOptions::default());
        assert_eq!(
            catalog.face_shape(&volume, ch, IVec3::ZERO, Face::PosX),
            FaceShape::Shape(1)
        );
        assert_eq!(
            catalog.face_shape(&volume, ch, IVec3::ZERO, Face::PosY),
            FaceShape::Full
        );
        assert!(catalog.mounts(&volume, ch, IVec3::ZERO, Face::NegX));
        assert!(!catalog.mounts(&volume, ch, IVec3::ZERO, Face::NegY));
        assert_eq!(
            decode_part(encode_part(IVec3::new(-3, 500, -511))),
            Some(IVec3::new(-3, 500, -511))
        );
    }
}
