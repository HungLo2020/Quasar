//! Back-to-front ordering for translucent quads.
use crate::quad::Quad;
use quasar_math::Vec3;

/// Produces a draw order (indices into the quads) for a camera position in
/// the same space as the quads. Returns whether the order changed.
pub trait TranslucentSorter {
    fn sort(&mut self, quads: &[Quad], camera: Vec3, order: &mut Vec<u32>) -> bool;
}

/// Sorts by centroid distance, farthest first; ties keep emission order.
#[derive(Clone, Debug, Default)]
pub struct CentroidSorter {
    keyed: Vec<(f32, u32)>,
}

impl TranslucentSorter for CentroidSorter {
    fn sort(&mut self, quads: &[Quad], camera: Vec3, order: &mut Vec<u32>) -> bool {
        self.keyed.clear();
        self.keyed.extend(
            quads
                .iter()
                .enumerate()
                .map(|(i, q)| ((q.centroid() - camera).length_squared(), i as u32)),
        );
        self.keyed
            .sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let changed =
            order.len() != quads.len() || order.iter().zip(&self.keyed).any(|(o, k)| *o != k.1);
        order.clear();
        order.extend(self.keyed.iter().map(|k| k.1));
        changed
    }
}

/// Re-sorts only when the camera crosses the plane of some quad (which is
/// when a correct order can change for planar quads); otherwise keeps the
/// previous order. Cheaper per frame than sorting.
#[derive(Clone, Debug, Default)]
pub struct PlaneSorter {
    sides: Vec<bool>,
    inner: CentroidSorter,
    quad_count: usize,
}

impl TranslucentSorter for PlaneSorter {
    fn sort(&mut self, quads: &[Quad], camera: Vec3, order: &mut Vec<u32>) -> bool {
        let mut crossed = self.quad_count != quads.len() || order.len() != quads.len();
        if self.sides.len() != quads.len() {
            self.sides.resize(quads.len(), false);
            crossed = true;
        }
        for (side, quad) in self.sides.iter_mut().zip(quads) {
            let front = (camera - quad.positions[0]).dot(quad.normal) >= 0.0;
            if *side != front {
                *side = front;
                crossed = true;
            }
        }
        self.quad_count = quads.len();
        if crossed {
            self.inner.sort(quads, camera, order)
        } else {
            false
        }
    }
}
