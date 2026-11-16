//! The 24 axis-aligned rotations of a cube.
use crate::layout::Face;
use quasar_math::IVec3;
use std::sync::OnceLock;

/// One of the 24 proper rotations mapping axes to axes. `Orientation(0)` is
/// the identity. Indices are stable: rotations are enumerated by the faces
/// they send local `+Y` (up) and `+Z` (forward) to, in [`Face`] order.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Orientation(pub u8);

struct Table {
    matrices: [[[i32; 3]; 3]; 24],
    compose: [[u8; 24]; 24],
    inverse: [u8; 24],
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut list: Vec<[[i32; 3]; 3]> = Vec::with_capacity(24);
        // Identity first, then in (up, forward) face order.
        let identity = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
        list.push(identity);
        for up in Face::ALL {
            for forward in Face::ALL {
                if up.axis() == forward.axis() {
                    continue;
                }
                let u = up.normal();
                let f = forward.normal();
                let r = u.cross(f); // right = up × forward (right-handed)
                // Columns are the images of +X, +Y, +Z.
                let m = [[r.x, u.x, f.x], [r.y, u.y, f.y], [r.z, u.z, f.z]];
                if m != identity {
                    list.push(m);
                }
            }
        }
        assert_eq!(list.len(), 24);
        let mut matrices = [[[0; 3]; 3]; 24];
        matrices.copy_from_slice(&list);
        let mul = |a: &[[i32; 3]; 3], b: &[[i32; 3]; 3]| {
            let mut out = [[0; 3]; 3];
            for (r, row) in out.iter_mut().enumerate() {
                for (c, cell) in row.iter_mut().enumerate() {
                    *cell = (0..3).map(|k| a[r][k] * b[k][c]).sum();
                }
            }
            out
        };
        let find = |m: &[[i32; 3]; 3]| matrices.iter().position(|x| x == m).unwrap() as u8;
        let mut compose = [[0u8; 24]; 24];
        let mut inverse = [0u8; 24];
        for a in 0..24 {
            for b in 0..24 {
                compose[a][b] = find(&mul(&matrices[a], &matrices[b]));
                if compose[a][b] == 0 {
                    inverse[a] = b as u8;
                }
            }
        }
        Table {
            matrices,
            compose,
            inverse,
        }
    })
}

impl Orientation {
    pub const IDENTITY: Self = Self(0);
    pub const COUNT: usize = 24;

    pub fn all() -> impl Iterator<Item = Self> {
        (0..24).map(Self)
    }
    pub fn is_valid(self) -> bool {
        (self.0 as usize) < Self::COUNT
    }
    /// Row-major integer rotation matrix.
    pub fn matrix(self) -> [[i32; 3]; 3] {
        table().matrices[self.0 as usize % 24]
    }
    /// Rotation sending local up/forward to the given faces, if they are
    /// perpendicular.
    pub fn from_up_forward(up: Face, forward: Face) -> Option<Self> {
        Self::all()
            .find(|o| o.rotate_face(Face::PosY) == up && o.rotate_face(Face::PosZ) == forward)
    }
    #[inline]
    pub fn rotate(self, v: IVec3) -> IVec3 {
        if self.0 == 0 {
            return v;
        }
        let m = self.matrix();
        IVec3::new(
            m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
            m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
            m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
        )
    }
    #[inline]
    pub fn rotate_face(self, face: Face) -> Face {
        if self.0 == 0 {
            return face;
        }
        Face::from_normal(self.rotate(face.normal())).expect("rotations map faces to faces")
    }
    /// The local face that ends up facing world direction `face`.
    #[inline]
    pub fn local_face(self, world: Face) -> Face {
        if self.0 == 0 {
            return world;
        }
        self.inverse().rotate_face(world)
    }
    pub fn inverse(self) -> Self {
        Self(table().inverse[self.0 as usize % 24])
    }
    /// `self ∘ then`: apply `then` first, then `self`.
    pub fn compose(self, then: Self) -> Self {
        Self(table().compose[self.0 as usize % 24][then.0 as usize % 24])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_group_is_closed_and_has_inverses() {
        let mut seen = std::collections::BTreeSet::new();
        for o in Orientation::all() {
            let m = o.matrix();
            let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
            assert_eq!(det, 1, "proper rotations only");
            assert!(seen.insert(m));
            assert_eq!(o.compose(o.inverse()), Orientation::IDENTITY);
            for v in [IVec3::new(1, 2, 3), IVec3::new(-4, 0, 7)] {
                assert_eq!(o.inverse().rotate(o.rotate(v)), v);
            }
            for f in Face::ALL {
                assert_eq!(o.local_face(o.rotate_face(f)), f);
            }
            let b = Orientation(7);
            let v = IVec3::new(1, 5, -2);
            assert_eq!(o.compose(b).rotate(v), o.rotate(b.rotate(v)));
        }
        assert_eq!(Orientation::IDENTITY.rotate(IVec3::X), IVec3::X);
        let o = Orientation::from_up_forward(Face::PosX, Face::PosY).unwrap();
        assert_eq!(o.rotate_face(Face::PosY), Face::PosX);
        assert!(Orientation::from_up_forward(Face::PosX, Face::NegX).is_none());
    }
}
