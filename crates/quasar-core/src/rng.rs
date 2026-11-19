//! Seeded, splittable, serialisable random numbers for deterministic games.
//!
//! [`Rng`] is xoshiro256** (Blackman and Vigna) seeded through SplitMix64.
//! Its output sequence is part of Quasar's compatibility contract: the same
//! seed yields the same values on every platform, build profile and Quasar
//! release, so it can drive replays, lockstep simulations and procedural
//! content. Unlike the `rand` crate, whose value streams may change between
//! versions, this generator has no dependencies and is covered by golden
//! tests against the published reference outputs.
//!
//! ```
//! use quasar_core::Rng;
//!
//! let mut rng = Rng::seed_from_u64(42);
//! let roll = rng.range_u32(1..7);
//! assert!((1..7).contains(&roll));
//! let mut deck = [1, 2, 3, 4, 5];
//! rng.shuffle(&mut deck);
//! let mut enemies = rng.split(); // independent stream for one subsystem
//! let _ = enemies.chance(0.25);
//!
//! // The full state round-trips, so it can live in snapshots.
//! let restored = Rng::from_state(rng.state()).unwrap();
//! assert_eq!(restored, rng);
//! ```
use std::ops::Range;

/// SplitMix64 step: used to expand seeds and derive split streams.
#[inline]
pub fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic xoshiro256** generator. See the module docs.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "[u64; 4]", into = "[u64; 4]"))]
pub struct Rng {
    s: [u64; 4],
}

/// Returned by [`Rng::from_state`] for the all-zero state, which xoshiro
/// cannot leave.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ZeroStateError;

impl std::fmt::Display for ZeroStateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("xoshiro256** state must not be all zero")
    }
}
impl std::error::Error for ZeroStateError {}

impl TryFrom<[u64; 4]> for Rng {
    type Error = ZeroStateError;
    fn try_from(state: [u64; 4]) -> Result<Self, Self::Error> {
        Self::from_state(state)
    }
}

impl From<Rng> for [u64; 4] {
    fn from(rng: Rng) -> Self {
        rng.s
    }
}

impl Rng {
    /// Expands a 64-bit seed with SplitMix64. Every seed, including 0, is valid.
    pub fn seed_from_u64(seed: u64) -> Self {
        let mut state = seed;
        let s = [
            splitmix64(&mut state),
            splitmix64(&mut state),
            splitmix64(&mut state),
            splitmix64(&mut state),
        ];
        // SplitMix64 is a bijection on its counter, so four consecutive
        // outputs are never all zero; keep the invariant explicit anyway.
        Self::from_state(s).unwrap_or(Self { s: [1, 0, 0, 0] })
    }

    /// Seeds from a string label combined with a numeric seed, for named
    /// streams such as `Rng::seed_from_label(world_seed, "loot")`.
    pub fn seed_from_label(seed: u64, label: &str) -> Self {
        // FNV-1a keeps the label hash stable and dependency-free.
        let mut hash = 0xCBF2_9CE4_8422_2325u64;
        for byte in label.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
        Self::seed_from_u64(seed ^ hash.rotate_left(17))
    }

    /// Restores a generator from [`Self::state`].
    pub fn from_state(state: [u64; 4]) -> Result<Self, ZeroStateError> {
        if state == [0; 4] {
            return Err(ZeroStateError);
        }
        Ok(Self { s: state })
    }

    /// Complete generator state, suitable for snapshots and state hashes.
    pub fn state(&self) -> [u64; 4] {
        self.s
    }

    /// Next raw 64-bit output.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Upper 32 bits of the next output.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform `f64` in `[0, 1)` with 53 bits of precision.
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform `f32` in `[0, 1)` with 24 bits of precision.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    /// Fills `bytes` with random data.
    pub fn fill_bytes(&mut self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(8) {
            let value = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&value[..chunk.len()]);
        }
    }

    /// Unbiased value in `0..bound` (Lemire's multiply-shift with rejection).
    /// Returns 0 when `bound` is 0.
    pub fn below_u64(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let product = u128::from(self.next_u64()) * u128::from(bound);
            if (product as u64) >= threshold {
                return (product >> 64) as u64;
            }
        }
    }

    /// Unbiased value in `range`. An empty range returns `range.start`.
    pub fn range_u64(&mut self, range: Range<u64>) -> u64 {
        if range.end <= range.start {
            return range.start;
        }
        range.start + self.below_u64(range.end - range.start)
    }

    /// Unbiased value in `range`. An empty range returns `range.start`.
    pub fn range_u32(&mut self, range: Range<u32>) -> u32 {
        self.range_u64(u64::from(range.start)..u64::from(range.end)) as u32
    }

    /// Unbiased value in `range`. An empty range returns `range.start`.
    pub fn range_usize(&mut self, range: Range<usize>) -> usize {
        self.range_u64(range.start as u64..range.end as u64) as usize
    }

    /// Unbiased value in `range`. An empty range returns `range.start`.
    pub fn range_i64(&mut self, range: Range<i64>) -> i64 {
        if range.end <= range.start {
            return range.start;
        }
        let span = range.end.wrapping_sub(range.start) as u64;
        range.start.wrapping_add(self.below_u64(span) as i64)
    }

    /// Unbiased value in `range`. An empty range returns `range.start`.
    pub fn range_i32(&mut self, range: Range<i32>) -> i32 {
        self.range_i64(i64::from(range.start)..i64::from(range.end)) as i32
    }

    /// Uniform value in `[start, end)`. Empty or non-finite ranges return
    /// `start`.
    pub fn range_f32(&mut self, range: Range<f32>) -> f32 {
        if !(range.start.is_finite() && range.end.is_finite()) || range.end <= range.start {
            return range.start;
        }
        let value = range.start + (range.end - range.start) * self.next_f32();
        // Rounding can land exactly on `end`; keep the interval half-open.
        if value >= range.end {
            range.start.max(f32::from_bits(range.end.to_bits() - 1))
        } else {
            value
        }
    }

    /// Uniform value in `[start, end)`. Empty or non-finite ranges return
    /// `start`.
    pub fn range_f64(&mut self, range: Range<f64>) -> f64 {
        if !(range.start.is_finite() && range.end.is_finite()) || range.end <= range.start {
            return range.start;
        }
        let value = range.start + (range.end - range.start) * self.next_f64();
        if value >= range.end {
            range.start.max(f64::from_bits(range.end.to_bits() - 1))
        } else {
            value
        }
    }

    /// `true` with probability `probability` (clamped to `[0, 1]`; NaN is 0).
    pub fn chance(&mut self, probability: f64) -> bool {
        if probability.is_nan() || probability <= 0.0 {
            // Still advance so call sites stay in lockstep regardless of p.
            self.next_u64();
            return false;
        }
        self.next_f64() < probability.min(1.0) || probability >= 1.0
    }

    /// Index chosen with probability proportional to `weights`. Negative and
    /// non-finite weights count as 0; returns `None` when no weight is
    /// positive.
    pub fn weighted_index(&mut self, weights: &[f64]) -> Option<usize> {
        let weight = |w: f64| if w.is_finite() && w > 0.0 { w } else { 0.0 };
        let total: f64 = weights.iter().copied().map(weight).sum();
        if !(total.is_finite() && total > 0.0) {
            return None;
        }
        let mut target = self.next_f64() * total;
        let mut last = None;
        for (index, w) in weights.iter().copied().map(weight).enumerate() {
            if w <= 0.0 {
                continue;
            }
            last = Some(index);
            if target < w {
                return Some(index);
            }
            target -= w;
        }
        last
    }

    /// Index chosen with probability proportional to integer `weights`,
    /// using exactly one [`Self::range_u64`] draw over the weight total (so
    /// for totals below 2^32 it matches `range_u32(0..total)`). Zero weights
    /// are never picked; returns `None` without drawing when every weight
    /// is zero. Integer weights avoid floating-point rounding, so content
    /// tables give the same picks everywhere.
    pub fn weighted_index_u32(&mut self, weights: &[u32]) -> Option<usize> {
        let total: u64 = weights.iter().map(|&w| u64::from(w)).sum();
        if total == 0 {
            return None;
        }
        let mut roll = self.range_u64(0..total);
        for (index, &weight) in weights.iter().enumerate() {
            let weight = u64::from(weight);
            if roll < weight {
                return Some(index);
            }
            roll -= weight;
        }
        None
    }

    /// Item chosen with probability proportional to its integer weight; see
    /// [`Self::weighted_index_u32`].
    pub fn weighted_u32<'a, T>(
        &mut self,
        items: &'a [T],
        weight: impl Fn(&T) -> u32,
    ) -> Option<&'a T> {
        let total: u64 = items.iter().map(|item| u64::from(weight(item))).sum();
        if total == 0 {
            return None;
        }
        let mut roll = self.range_u64(0..total);
        for item in items {
            let w = u64::from(weight(item));
            if roll < w {
                return Some(item);
            }
            roll -= w;
        }
        None
    }

    /// Reference to an item chosen with probability proportional to
    /// `weight(item)`.
    pub fn weighted<'a, T>(&mut self, items: &'a [T], weight: impl Fn(&T) -> f64) -> Option<&'a T> {
        let weights: Vec<f64> = items.iter().map(weight).collect();
        self.weighted_index(&weights).map(|index| &items[index])
    }

    /// Uniformly chosen item, or `None` for an empty slice.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.below_u64(items.len() as u64) as usize])
        }
    }

    /// Fisher-Yates shuffle in place.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let other = self.below_u64(index as u64 + 1) as usize;
            items.swap(index, other);
        }
    }

    /// Derives an independent child generator and advances this one.
    ///
    /// The child is seeded from this generator's next output through
    /// SplitMix64, so splitting is deterministic: the same parent state
    /// always produces the same child, and parent and child sequences do not
    /// overlap in practice. Use it to give subsystems (loot, AI, particles)
    /// their own streams so adding a draw in one does not shift the others.
    pub fn split(&mut self) -> Self {
        Self::seed_from_u64(self.next_u64() ^ 0x6A09_E667_F3BC_C908)
    }

    /// Advances the state by 2^128 steps: the reference xoshiro256** jump.
    /// Calling it `n` times yields `n` non-overlapping subsequences.
    pub fn jump(&mut self) {
        const JUMP: [u64; 4] = [
            0x180E_C6D3_3CFD_0ABA,
            0xD5A6_1266_F0C9_392C,
            0xA958_6E3C_BAA4_3E5E,
            0x39AB_DC45_29B1_661C,
        ];
        let mut next = [0u64; 4];
        for word in JUMP {
            for bit in 0..64 {
                if word & (1u64 << bit) != 0 {
                    for (acc, value) in next.iter_mut().zip(self.s) {
                        *acc ^= value;
                    }
                }
                self.next_u64();
            }
        }
        self.s = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_reference_xoshiro256starstar_and_splitmix64_outputs() {
        // Reference outputs for state {1, 2, 3, 4} from the published C code.
        let mut rng = Rng::from_state([1, 2, 3, 4]).unwrap();
        assert_eq!(
            [
                rng.next_u64(),
                rng.next_u64(),
                rng.next_u64(),
                rng.next_u64()
            ],
            [11520, 0, 1_509_978_240, 1_215_971_899_390_074_240]
        );
        let mut state = 0u64;
        assert_eq!(splitmix64(&mut state), 0xE220_A839_7B1D_CDAF);
        assert_eq!(splitmix64(&mut state), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn seeded_streams_are_stable_golden_values() {
        // Golden values guard the compatibility contract across releases.
        let mut rng = Rng::seed_from_u64(0);
        let first: Vec<u64> = (0..3).map(|_| rng.next_u64()).collect();
        let mut again = Rng::seed_from_u64(0);
        assert_eq!(first, (0..3).map(|_| again.next_u64()).collect::<Vec<_>>());
        assert_eq!(first, GOLDEN_SEED_0);
        assert_ne!(Rng::seed_from_u64(1), Rng::seed_from_u64(0));
        assert_ne!(
            Rng::seed_from_label(7, "loot"),
            Rng::seed_from_label(7, "ai")
        );
    }

    const GOLDEN_SEED_0: [u64; 3] = [
        0x99EC_5F36_CB75_F2B4,
        0xBF6E_1F78_4956_452A,
        0x1A5F_849D_4933_E6E0,
    ];

    #[test]
    fn ranges_are_bounded_and_cover_their_span() {
        let mut rng = Rng::seed_from_u64(9);
        let mut seen = [false; 6];
        for _ in 0..10_000 {
            let value = rng.range_u32(10..16);
            assert!((10..16).contains(&value));
            seen[(value - 10) as usize] = true;
            let signed = rng.range_i32(-3..3);
            assert!((-3..3).contains(&signed));
            let wide = rng.range_i64(i64::MIN..i64::MAX);
            assert!(wide < i64::MAX);
            let float = rng.range_f32(-1.0..1.0);
            assert!((-1.0..1.0).contains(&float));
            let unit = rng.next_f64();
            assert!((0.0..1.0).contains(&unit));
        }
        assert!(seen.iter().all(|seen| *seen));
        assert_eq!(rng.range_u32(5..5), 5);
        assert_eq!(rng.range_f32(2.0..1.0), 2.0);
        assert_eq!(rng.range_f64(f64::NAN..1.0).to_bits(), f64::NAN.to_bits());
        assert_eq!(rng.below_u64(0), 0);
    }

    #[test]
    fn uniform_ranges_are_unbiased_within_tolerance() {
        let mut rng = Rng::seed_from_u64(1234);
        let mut counts = [0u32; 3];
        let samples = 300_000;
        for _ in 0..samples {
            counts[rng.below_u64(3) as usize] += 1;
        }
        for count in counts {
            let share = f64::from(count) / f64::from(samples);
            assert!((share - 1.0 / 3.0).abs() < 0.005, "{counts:?}");
        }
    }

    #[test]
    fn chance_weighted_pick_and_shuffle_behave() {
        let mut rng = Rng::seed_from_u64(77);
        assert!((0..100).all(|_| !rng.chance(0.0)));
        assert!((0..100).all(|_| rng.chance(1.0)));
        assert!(!rng.chance(f64::NAN));
        let hits = (0..100_000).filter(|_| rng.chance(0.25)).count();
        assert!((hits as f64 / 100_000.0 - 0.25).abs() < 0.01);

        assert_eq!(rng.weighted_index(&[]), None);
        assert_eq!(rng.weighted_index(&[0.0, -1.0, f64::NAN]), None);
        assert_eq!(rng.weighted_index(&[0.0, 5.0, 0.0]), Some(1));
        let mut counts = [0u32; 2];
        for _ in 0..100_000 {
            counts[rng.weighted_index(&[1.0, 3.0]).unwrap()] += 1;
        }
        assert!((f64::from(counts[1]) / 100_000.0 - 0.75).abs() < 0.01);
        let items = ["common", "rare"];
        let picked = rng.weighted(&items, |item| if *item == "rare" { 0.0 } else { 1.0 });
        assert_eq!(picked, Some(&"common"));
        assert_eq!(rng.pick::<u8>(&[]), None);
        assert!(items.contains(rng.pick(&items).unwrap()));

        let mut values: Vec<u32> = (0..50).collect();
        rng.shuffle(&mut values);
        assert_ne!(values, (0..50).collect::<Vec<_>>());
        values.sort_unstable();
        assert_eq!(values, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn split_and_jump_are_deterministic_and_independent() {
        let mut a = Rng::seed_from_u64(5);
        let mut b = Rng::seed_from_u64(5);
        let mut child_a = a.split();
        let mut child_b = b.split();
        assert_eq!(child_a, child_b);
        assert_eq!(a, b);
        assert_ne!(child_a.next_u64(), a.next_u64());
        child_b.next_u64();
        b.next_u64();
        let mut jumped = b.clone();
        jumped.jump();
        assert_ne!(jumped, b);
        let mut jumped_again = b.clone();
        jumped_again.jump();
        assert_eq!(jumped, jumped_again);
    }

    #[test]
    fn state_round_trips_and_rejects_zero() {
        let mut rng = Rng::seed_from_u64(3);
        rng.next_u64();
        let restored = Rng::from_state(rng.state()).unwrap();
        assert_eq!(restored, rng);
        assert_eq!(Rng::from_state([0; 4]), Err(ZeroStateError));
        let mut bytes = [0u8; 13];
        rng.fill_bytes(&mut bytes);
        assert!(bytes.iter().any(|byte| *byte != 0));
    }
}

#[cfg(test)]
mod integer_weight_tests {
    use super::*;

    #[test]
    fn integer_weights_use_one_range_u32_draw() {
        // Reference implementation games already ship (one range_u32 draw,
        // subtract weights in order); the engine version must match it.
        fn reference(rng: &mut Rng, weights: &[u32]) -> Option<usize> {
            let total: u32 = weights.iter().sum();
            if total == 0 {
                return None;
            }
            let mut roll = rng.range_u32(0..total);
            for (i, &w) in weights.iter().enumerate() {
                if roll < w {
                    return Some(i);
                }
                roll -= w;
            }
            None
        }
        let weights = [3, 0, 7, 1, 12];
        let mut a = Rng::seed_from_u64(99);
        let mut b = Rng::seed_from_u64(99);
        for _ in 0..2000 {
            assert_eq!(a.weighted_index_u32(&weights), reference(&mut b, &weights));
        }
        assert_eq!(a, b);
        let items = [("a", 1u32), ("b", 0), ("c", 3)];
        let picks: Vec<&str> = (0..400)
            .filter_map(|_| a.weighted_u32(&items, |i| i.1).map(|i| i.0))
            .collect();
        assert!(!picks.contains(&"b"));
        let before = a.clone();
        assert_eq!(a.weighted_index_u32(&[0, 0]), None);
        assert_eq!(a, before, "all-zero weights do not draw");
        assert_eq!(
            a.weighted_index_u32(&[u32::MAX, u32::MAX]).map(|i| i < 2),
            Some(true)
        );
    }
}
