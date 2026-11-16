//! Chunk shapes, in-chunk index orders and faces.
//!
//! Chunks are cubes with a power-of-two edge fixed at compile time
//! (`Cube<LOG2>`, `LOG2` in 3..=6). Indexing is monomorphised per layout so
//! hot loops never branch on the chunk size or order.
use quasar_math::{IVec3, UVec3};
use std::marker::PhantomData;

/// One of the six axis-aligned cell faces. The discriminant order
/// (`-X, +X, -Y, +Y, -Z, +Z`) is stable and used by every mask and table.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum Face {
    NegX = 0,
    PosX = 1,
    NegY = 2,
    PosY = 3,
    NegZ = 4,
    PosZ = 5,
}

impl Face {
    pub const ALL: [Face; 6] = [
        Face::NegX,
        Face::PosX,
        Face::NegY,
        Face::PosY,
        Face::NegZ,
        Face::PosZ,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::NegX),
            1 => Some(Self::PosX),
            2 => Some(Self::NegY),
            3 => Some(Self::PosY),
            4 => Some(Self::NegZ),
            5 => Some(Self::PosZ),
            _ => None,
        }
    }
    /// 0 = X, 1 = Y, 2 = Z.
    pub const fn axis(self) -> usize {
        self as usize / 2
    }
    pub const fn is_positive(self) -> bool {
        self as usize & 1 == 1
    }
    pub const fn opposite(self) -> Self {
        match self {
            Self::NegX => Self::PosX,
            Self::PosX => Self::NegX,
            Self::NegY => Self::PosY,
            Self::PosY => Self::NegY,
            Self::NegZ => Self::PosZ,
            Self::PosZ => Self::NegZ,
        }
    }
    pub const fn normal(self) -> IVec3 {
        match self {
            Self::NegX => IVec3::new(-1, 0, 0),
            Self::PosX => IVec3::new(1, 0, 0),
            Self::NegY => IVec3::new(0, -1, 0),
            Self::PosY => IVec3::new(0, 1, 0),
            Self::NegZ => IVec3::new(0, 0, -1),
            Self::PosZ => IVec3::new(0, 0, 1),
        }
    }
    /// Face whose normal is `normal` (must be a unit axis vector).
    pub fn from_normal(normal: IVec3) -> Option<Self> {
        Face::ALL.into_iter().find(|face| face.normal() == normal)
    }
    /// Bit for this face in a 6-bit mask.
    pub const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// Order of cells inside a chunk's flat arrays. Names list axes from the
/// fastest-varying to the slowest: `Xzy` is `x + z * edge + y * edge²`.
pub trait IndexOrder: Copy + Default + Send + Sync + 'static {
    /// Axis permutation `[fastest, middle, slowest]` (0 = X, 1 = Y, 2 = Z).
    const AXES: [usize; 3];
}

/// `x` fastest, then `z`, then `y` (column-major heights; Minecraft-style).
#[derive(Clone, Copy, Debug, Default)]
pub struct Xzy;
/// `x` fastest, then `y`, then `z`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Xyz;
/// `y` fastest, then `z`, then `x` (vertical runs contiguous).
#[derive(Clone, Copy, Debug, Default)]
pub struct Yzx;

impl IndexOrder for Xzy {
    const AXES: [usize; 3] = [0, 2, 1];
}
impl IndexOrder for Xyz {
    const AXES: [usize; 3] = [0, 1, 2];
}
impl IndexOrder for Yzx {
    const AXES: [usize; 3] = [1, 2, 0];
}

/// A compile-time chunk shape and in-chunk order.
pub trait ChunkLayout: Copy + Default + Send + Sync + 'static {
    const LOG2: u32;
    const EDGE: usize = 1 << Self::LOG2;
    const AREA: usize = Self::EDGE * Self::EDGE;
    const VOLUME: usize = Self::AREA * Self::EDGE;
    const MASK: i32 = (1 << Self::LOG2) - 1;
    type Order: IndexOrder;

    /// Flat index of a local cell (each component `< EDGE`).
    #[inline(always)]
    fn index(x: u32, y: u32, z: u32) -> usize {
        let c = [x as usize, y as usize, z as usize];
        let [a, b, s] = Self::Order::AXES;
        c[a] | (c[b] << Self::LOG2) | (c[s] << (2 * Self::LOG2))
    }
    /// Local cell of a flat index.
    #[inline(always)]
    fn local(index: usize) -> UVec3 {
        let mask = Self::EDGE - 1;
        let [a, b, s] = Self::Order::AXES;
        let mut c = [0u32; 3];
        c[a] = (index & mask) as u32;
        c[b] = ((index >> Self::LOG2) & mask) as u32;
        c[s] = (index >> (2 * Self::LOG2)) as u32;
        UVec3::new(c[0], c[1], c[2])
    }
    #[inline(always)]
    fn index_of(local: UVec3) -> usize {
        Self::index(local.x, local.y, local.z)
    }
    /// Chunk coordinate containing a grid-local voxel.
    #[inline(always)]
    fn chunk_of(voxel: IVec3) -> IVec3 {
        IVec3::new(
            voxel.x >> Self::LOG2,
            voxel.y >> Self::LOG2,
            voxel.z >> Self::LOG2,
        )
    }
    /// Position of a voxel inside its chunk.
    #[inline(always)]
    fn local_of(voxel: IVec3) -> UVec3 {
        UVec3::new(
            (voxel.x & Self::MASK) as u32,
            (voxel.y & Self::MASK) as u32,
            (voxel.z & Self::MASK) as u32,
        )
    }
    /// First voxel of a chunk.
    #[inline(always)]
    fn origin(chunk: IVec3) -> IVec3 {
        chunk << Self::LOG2 as i32
    }
}

/// Cubic chunk with edge `2^LOG2` (8, 16, 32 or 64) and index order `O`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cube<const LOG2: u32, O = Xzy>(PhantomData<O>);

impl<const LOG2: u32, O: IndexOrder> ChunkLayout for Cube<LOG2, O> {
    const LOG2: u32 = {
        assert!(LOG2 >= 3 && LOG2 <= 6, "chunk edge must be 8, 16, 32 or 64");
        LOG2
    };
    type Order = O;
}

/// 16³ chunks, `x`-`z`-`y` order: the Minecraft-style section default.
pub type Cube16 = Cube<4, Xzy>;
/// 32³ chunks, `x`-`z`-`y` order: the default for terrain, ships and density maps.
pub type Cube32 = Cube<5, Xzy>;
/// 8³ chunks.
pub type Cube8 = Cube<3, Xzy>;
/// 64³ chunks (storage/queries; some extractors reject this size).
pub type Cube64 = Cube<6, Xzy>;

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    fn round_trip<L: ChunkLayout>() {
        let mut seen = vec![false; L::VOLUME];
        for i in 0..L::VOLUME {
            let local = L::local(i);
            assert_eq!(L::index_of(local), i);
            seen[i] = true;
        }
        assert!(seen.into_iter().all(|v| v));
    }

    #[test]
    fn indices_round_trip_for_every_supported_layout() {
        round_trip::<Cube8>();
        round_trip::<Cube16>();
        round_trip::<Cube32>();
        round_trip::<Cube<4, Xyz>>();
        round_trip::<Cube<4, Yzx>>();
        assert_eq!(Cube16::index(1, 0, 0), 1);
        assert_eq!(Cube16::index(0, 0, 1), 16);
        assert_eq!(Cube16::index(0, 1, 0), 256, "Xzy keeps y slowest");
        assert_eq!(Cube::<4, Yzx>::index(0, 1, 0), 1);
    }

    #[test]
    fn chunk_and_local_coordinates_handle_negatives() {
        let voxel = IVec3::new(-1, 17, -33);
        assert_eq!(Cube16::chunk_of(voxel), IVec3::new(-1, 1, -3));
        assert_eq!(Cube16::local_of(voxel), UVec3::new(15, 1, 15));
        let chunk = Cube16::chunk_of(voxel);
        assert_eq!(
            Cube16::origin(chunk) + Cube16::local_of(voxel).as_ivec3(),
            voxel
        );
    }

    #[test]
    fn faces_are_consistent() {
        for face in Face::ALL {
            assert_eq!(face.opposite().opposite(), face);
            assert_eq!(face.normal(), -face.opposite().normal());
            assert_eq!(Face::from_normal(face.normal()), Some(face));
            assert_eq!(Face::from_index(face.index()), Some(face));
        }
        assert_eq!(Face::PosY.axis(), 1);
        assert!(Face::PosZ.is_positive() && !Face::NegZ.is_positive());
    }
}
