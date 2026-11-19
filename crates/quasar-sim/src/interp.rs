//! Render interpolation between fixed simulation ticks.
//!
//! A fixed-step simulation advances in whole ticks (say 60 Hz) while the
//! display refreshes faster, so drawing only the latest tick makes moving
//! things step while a smoothed camera glides. The fix is to draw each
//! object between its previous and current tick by the clock's blend factor
//! ([`FixedStep::interpolation`](quasar_core::FixedStep::interpolation)).
//!
//! [`Interpolation`] records the previous tick's values per key (entity id)
//! with [`Interpolation::record`] just before each tick, and
//! [`Interpolation::blend`] returns the in-between value at draw time. Keys
//! that appeared this tick, stale snapshots (more than one tick old) and
//! teleports (see [`Interpolation::with_snap`]) draw the current value. The
//! simulation itself is never modified, so determinism is untouched; which
//! fields to blend stays the game's choice through the [`Blend`] trait.
//!
//! ```
//! use quasar_sim::interp::Interpolation;
//!
//! // Entity id -> position.
//! let mut positions = Interpolation::<u32, [f32; 2]>::new()
//!     .with_snap(|a, b| (a[0] - b[0]).hypot(a[1] - b[1]) > 4.0);
//! let mut world = vec![(1u32, [0.0f32, 0.0])];
//!
//! positions.record(7, world.iter().copied()); // before tick 7 runs
//! world[0].1 = [1.0, 0.0];                    // tick 7 moves the entity
//!
//! // Drawing at alpha 0.25 after tick 7 (tick counter now 8):
//! assert_eq!(positions.blend(8, &1, &world[0].1, 0.25), [0.25, 0.0]);
//! ```

/// Values that can be drawn part of the way between two ticks.
pub trait Blend: Clone {
    /// The value `t` (0..1) of the way from `self` to `to`.
    fn blend(&self, to: &Self, t: f32) -> Self;
}

impl Blend for f32 {
    fn blend(&self, to: &Self, t: f32) -> Self {
        self + (to - self) * t
    }
}
impl Blend for f64 {
    fn blend(&self, to: &Self, t: f32) -> Self {
        self + (to - self) * f64::from(t)
    }
}
impl<const N: usize> Blend for [f32; N] {
    fn blend(&self, to: &Self, t: f32) -> Self {
        std::array::from_fn(|i| self[i] + (to[i] - self[i]) * t)
    }
}
impl<A: Blend, B: Blend> Blend for (A, B) {
    fn blend(&self, to: &Self, t: f32) -> Self {
        (self.0.blend(&to.0, t), self.1.blend(&to.1, t))
    }
}
impl<A: Blend, B: Blend, C: Blend> Blend for (A, B, C) {
    fn blend(&self, to: &Self, t: f32) -> Self {
        (
            self.0.blend(&to.0, t),
            self.1.blend(&to.1, t),
            self.2.blend(&to.2, t),
        )
    }
}

/// Shortest-arc blend of an angle in radians, for yaw and headings.
pub fn blend_angle(from: f32, to: f32, t: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let delta = (to - from).rem_euclid(tau);
    let delta = if delta > std::f32::consts::PI {
        delta - tau
    } else {
        delta
    };
    from + delta * t
}

/// Decides that a change is a teleport (draw the current value).
type SnapFn<V> = Box<dyn Fn(&V, &V) -> bool + Send + Sync>;

/// Previous-tick values per key. See the module docs.
pub struct Interpolation<K, V> {
    tick: Option<u64>,
    previous: Vec<(K, V)>,
    snap: Option<SnapFn<V>>,
}

impl<K: Ord + Clone, V: Blend> Default for Interpolation<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord + Clone, V: Blend> Interpolation<K, V> {
    pub fn new() -> Self {
        Self {
            tick: None,
            previous: Vec::new(),
            snap: None,
        }
    }
    /// Draws the current value whenever `snap(previous, current)` is true,
    /// for teleports, respawns and blinks that should not slide.
    pub fn with_snap(mut self, snap: impl Fn(&V, &V) -> bool + Send + Sync + 'static) -> Self {
        self.snap = Some(Box::new(snap));
        self
    }
    /// Records the values before simulation tick `tick` runs. Reuses its
    /// buffer; keys may arrive in any order.
    pub fn record(&mut self, tick: u64, values: impl IntoIterator<Item = (K, V)>) {
        self.previous.clear();
        self.previous.extend(values);
        self.previous.sort_by(|a, b| a.0.cmp(&b.0));
        self.tick = Some(tick);
    }
    /// The tick the snapshot was taken before.
    pub fn recorded_tick(&self) -> Option<u64> {
        self.tick
    }
    /// The previous value of `key`, if it was recorded.
    pub fn previous(&self, key: &K) -> Option<&V> {
        self.previous
            .binary_search_by(|(k, _)| k.cmp(key))
            .ok()
            .map(|index| &self.previous[index].1)
    }
    /// Whether a draw after `now_tick` ticks can use the snapshot: exactly
    /// one tick ran since it was recorded.
    pub fn is_current(&self, now_tick: u64) -> bool {
        self.tick.is_some_and(|tick| tick + 1 == now_tick)
    }
    /// `current` drawn `alpha` of the way from its previous value, or
    /// `current` itself when there is no usable previous value.
    pub fn blend(&self, now_tick: u64, key: &K, current: &V, alpha: f32) -> V {
        if !self.is_current(now_tick) {
            return current.clone();
        }
        match self.previous(key) {
            Some(previous)
                if !self
                    .snap
                    .as_ref()
                    .is_some_and(|snap| snap(previous, current)) =>
            {
                previous.blend(current, alpha.clamp(0.0, 1.0))
            }
            _ => current.clone(),
        }
    }
    /// Forgets the snapshot (scene change, load).
    pub fn clear(&mut self) {
        self.tick = None;
        self.previous.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blends_known_keys_and_draws_new_or_stale_ones_as_is() {
        let mut interp = Interpolation::<u32, (f32, [f32; 2])>::new()
            .with_snap(|a, b| (a.1[0] - b.1[0]).abs() > 4.0);
        interp.record(10, [(2, (0.0, [0.0, 0.0])), (1, (1.0, [5.0, 5.0]))]);
        assert_eq!(interp.recorded_tick(), Some(10));
        // Tick 10 ran.
        assert_eq!(
            interp.blend(11, &1, &(3.0, [6.0, 5.0]), 0.5),
            (2.0, [5.5, 5.0])
        );
        assert_eq!(
            interp.blend(11, &3, &(9.0, [9.0, 9.0]), 0.5),
            (9.0, [9.0, 9.0])
        );
        // Teleport snaps.
        assert_eq!(
            interp.blend(11, &2, &(0.0, [20.0, 0.0]), 0.5),
            (0.0, [20.0, 0.0])
        );
        // Two ticks ran: stale.
        assert_eq!(
            interp.blend(12, &1, &(3.0, [6.0, 5.0]), 0.5),
            (3.0, [6.0, 5.0])
        );
        // Alpha is clamped.
        assert_eq!(
            interp.blend(11, &1, &(3.0, [6.0, 5.0]), 7.0),
            (3.0, [6.0, 5.0])
        );
        interp.clear();
        assert!(!interp.is_current(11));
    }

    #[test]
    fn angles_blend_the_short_way() {
        let pi = std::f32::consts::PI;
        let a = blend_angle(pi - 0.1, -pi + 0.1, 0.5);
        assert!((a.abs() - pi).abs() < 1e-5, "{a}");
        assert!((blend_angle(0.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
    }
}
