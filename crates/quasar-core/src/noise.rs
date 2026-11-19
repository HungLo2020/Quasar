//! Stateless coordinate hashes and seeded coherent noise.
//!
//! Everything here is a pure function of its inputs: the same seed and
//! coordinates give the same value on every platform and build, so hashes
//! and noise can drive world generation, replays and per-block visual
//! variation without storing anything. Like [`Rng`](crate::Rng), the output
//! is part of Quasar's compatibility contract and covered by golden tests.
//!
//! - [`hash_u32`], [`hash2`], [`hash3`]: integer mixing for "a random value
//!   per cell" (block tint, decoration choice, ore placement).
//! - [`unit_f32`] / [`signed_f32`]: map a hash to `[0, 1)` / `[-1, 1)`.
//! - [`Noise`]: smooth value and gradient (Perlin) noise in 2D and 3D, plus
//!   fractal sums ([`Fractal`]) for caves, terrain and texture detail.
//!
//! ```
//! use quasar_core::noise::{Fractal, Noise, hash2, unit_f32};
//!
//! let tint = unit_f32(hash2(7, 12, -3)); // per-block variation in [0, 1)
//! assert!((0.0..1.0).contains(&tint));
//!
//! let noise = Noise::new(42);
//! let height = noise.fbm2(10.5, 3.25, Fractal::new(0.05, 4));
//! assert!((-1.0..=1.0).contains(&height));
//! ```

/// Mixes 32 bits into 32 well-distributed bits (Wellons' `lowbias32`).
#[inline]
pub const fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x21f0_aaad);
    x ^= x >> 15;
    x = x.wrapping_mul(0x735a_2d97);
    x ^= x >> 15;
    x
}

/// Mixes 64 bits (the SplitMix64 finaliser).
#[inline]
pub const fn hash_u64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Hash of a 2D integer coordinate under `seed`.
#[inline]
pub const fn hash2(seed: u32, x: i32, y: i32) -> u32 {
    hash_u32(
        seed ^ hash_u32(
            (x as u32).wrapping_mul(0x8da6_b343)
                ^ (y as u32).wrapping_mul(0xd816_3841)
                ^ 0x27d4_eb2f,
        ),
    )
}

/// Hash of a 3D integer coordinate under `seed`.
#[inline]
pub const fn hash3(seed: u32, x: i32, y: i32, z: i32) -> u32 {
    hash_u32(
        seed ^ hash_u32(
            (x as u32).wrapping_mul(0x8da6_b343)
                ^ (y as u32).wrapping_mul(0xd816_3841)
                ^ (z as u32).wrapping_mul(0xcb1a_b31f)
                ^ 0x1656_67b1,
        ),
    )
}

/// Maps a hash to `[0, 1)` using its top 24 bits.
#[inline]
pub fn unit_f32(hash: u32) -> f32 {
    (hash >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Maps a hash to `[-1, 1)`.
#[inline]
pub fn signed_f32(hash: u32) -> f32 {
    unit_f32(hash) * 2.0 - 1.0
}

/// Fractal (fBm) settings: `octaves` layers starting at `frequency` (cycles
/// per unit), each `lacunarity` times finer and `gain` times weaker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fractal {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
}

impl Fractal {
    /// `octaves` at `frequency` with lacunarity 2 and gain 0.5.
    pub const fn new(frequency: f32, octaves: u32) -> Self {
        Self {
            frequency,
            octaves,
            lacunarity: 2.0,
            gain: 0.5,
        }
    }
    pub const fn with_lacunarity(mut self, lacunarity: f32) -> Self {
        self.lacunarity = lacunarity;
        self
    }
    pub const fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }
}

impl Default for Fractal {
    fn default() -> Self {
        Self::new(1.0, 4)
    }
}

/// Seeded coherent noise. Cheap to copy; holds only the seed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Noise {
    seed: u32,
}

#[inline]
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
#[inline]
fn quintic(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
#[inline]
fn cell(v: f32) -> (i32, f32) {
    let floor = v.floor();
    (floor as i32, v - floor)
}

#[inline]
fn grad2(hash: u32, x: f32, y: f32) -> f32 {
    // Eight directions: axes and diagonals (diagonals scaled to unit length
    // so the range stays near [-1, 1]).
    const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
    match hash & 7 {
        0 => x,
        1 => -x,
        2 => y,
        3 => -y,
        4 => (x + y) * D,
        5 => (x - y) * D,
        6 => (-x + y) * D,
        _ => (-x - y) * D,
    }
}

#[inline]
fn grad3(hash: u32, x: f32, y: f32, z: f32) -> f32 {
    // Perlin's twelve cube-edge directions (with four repeats for 16).
    match hash & 15 {
        0 | 12 => x + y,
        1 | 14 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 | 13 => -y + z,
        10 => y - z,
        _ => -y - z,
    }
}

impl Noise {
    /// Noise for `seed`. Different seeds give unrelated fields.
    pub const fn new(seed: u64) -> Self {
        Self {
            seed: hash_u64(seed) as u32,
        }
    }
    /// A field derived from this one (for example one per octave or per
    /// feature), unrelated to it.
    pub const fn derive(self, salt: u32) -> Self {
        Self {
            seed: hash_u32(self.seed ^ salt.wrapping_mul(0x9e37_79b9)),
        }
    }
    pub const fn seed(self) -> u32 {
        self.seed
    }

    /// Smooth value noise in `[0, 1]`.
    pub fn value2(self, x: f32, y: f32) -> f32 {
        let (ix, fx) = cell(x);
        let (iy, fy) = cell(y);
        let v = |dx, dy| unit_f32(hash2(self.seed, ix.wrapping_add(dx), iy.wrapping_add(dy)));
        let (sx, sy) = (smooth(fx), smooth(fy));
        lerp(lerp(v(0, 0), v(1, 0), sx), lerp(v(0, 1), v(1, 1), sx), sy)
    }

    /// Smooth value noise in `[0, 1]`.
    pub fn value3(self, x: f32, y: f32, z: f32) -> f32 {
        let (ix, fx) = cell(x);
        let (iy, fy) = cell(y);
        let (iz, fz) = cell(z);
        let v = |dx, dy, dz| {
            unit_f32(hash3(
                self.seed,
                ix.wrapping_add(dx),
                iy.wrapping_add(dy),
                iz.wrapping_add(dz),
            ))
        };
        let (sx, sy, sz) = (smooth(fx), smooth(fy), smooth(fz));
        let plane = |dz| {
            lerp(
                lerp(v(0, 0, dz), v(1, 0, dz), sx),
                lerp(v(0, 1, dz), v(1, 1, dz), sx),
                sy,
            )
        };
        lerp(plane(0), plane(1), sz)
    }

    /// Gradient (Perlin) noise, about `[-1, 1]`, zero at integer points.
    pub fn perlin2(self, x: f32, y: f32) -> f32 {
        let (ix, fx) = cell(x);
        let (iy, fy) = cell(y);
        let g = |dx: i32, dy: i32| {
            grad2(
                hash2(self.seed, ix.wrapping_add(dx), iy.wrapping_add(dy)),
                fx - dx as f32,
                fy - dy as f32,
            )
        };
        let (u, v) = (quintic(fx), quintic(fy));
        (lerp(lerp(g(0, 0), g(1, 0), u), lerp(g(0, 1), g(1, 1), u), v) * 1.414).clamp(-1.0, 1.0)
    }

    /// Gradient (Perlin) noise, about `[-1, 1]`, zero at integer points.
    pub fn perlin3(self, x: f32, y: f32, z: f32) -> f32 {
        let (ix, fx) = cell(x);
        let (iy, fy) = cell(y);
        let (iz, fz) = cell(z);
        let g = |dx: i32, dy: i32, dz: i32| {
            grad3(
                hash3(
                    self.seed,
                    ix.wrapping_add(dx),
                    iy.wrapping_add(dy),
                    iz.wrapping_add(dz),
                ),
                fx - dx as f32,
                fy - dy as f32,
                fz - dz as f32,
            )
        };
        let (u, v, w) = (quintic(fx), quintic(fy), quintic(fz));
        let plane = |dz| {
            lerp(
                lerp(g(0, 0, dz), g(1, 0, dz), u),
                lerp(g(0, 1, dz), g(1, 1, dz), u),
                v,
            )
        };
        (lerp(plane(0), plane(1), w) * 0.97).clamp(-1.0, 1.0)
    }

    fn fractal(self, fractal: Fractal, mut sample: impl FnMut(Noise, f32) -> f32) -> f32 {
        let (mut sum, mut amplitude, mut norm, mut frequency) = (0.0, 1.0, 0.0, fractal.frequency);
        for octave in 0..fractal.octaves.max(1) {
            sum += sample(self.derive(octave), frequency) * amplitude;
            norm += amplitude;
            amplitude *= fractal.gain;
            frequency *= fractal.lacunarity;
        }
        if norm > 0.0 { sum / norm } else { 0.0 }
    }

    /// Fractal Perlin noise in `[-1, 1]`.
    pub fn fbm2(self, x: f32, y: f32, fractal: Fractal) -> f32 {
        self.fractal(fractal, |n, f| n.perlin2(x * f, y * f))
    }
    /// Fractal Perlin noise in `[-1, 1]`.
    pub fn fbm3(self, x: f32, y: f32, z: f32, fractal: Fractal) -> f32 {
        self.fractal(fractal, |n, f| n.perlin3(x * f, y * f, z * f))
    }
    /// Fractal value noise in `[0, 1]`.
    pub fn value_fbm2(self, x: f32, y: f32, fractal: Fractal) -> f32 {
        self.fractal(fractal, |n, f| n.value2(x * f, y * f))
    }
    /// Fractal value noise in `[0, 1]`.
    pub fn value_fbm3(self, x: f32, y: f32, z: f32, fractal: Fractal) -> f32 {
        self.fractal(fractal, |n, f| n.value3(x * f, y * f, z * f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_hashes_are_stable() {
        assert_eq!(hash_u32(0), 0);
        // Pin real values so an accidental change to the mixers is caught.
        let pinned = [hash2(0, 0, 0), hash2(7, 12, -3), hash3(1, 2, 3, 4)];
        assert_eq!(pinned, GOLDEN);
    }
    const GOLDEN: [u32; 3] = [4_056_896_858, 167_102_238, 4_105_787_967];

    #[test]
    fn hashes_spread_neighbouring_cells() {
        let mut buckets = [0u32; 16];
        for y in 0..64 {
            for x in 0..64 {
                buckets[(hash2(3, x, y) >> 28) as usize] += 1;
            }
        }
        for count in buckets {
            assert!((200..=320).contains(&count), "{buckets:?}");
        }
        assert_ne!(hash2(1, 0, 1), hash2(1, 1, 0));
        assert_ne!(hash3(1, 0, 0, 1), hash3(1, 0, 1, 0));
        assert_ne!(hash2(1, 5, 5), hash2(2, 5, 5));
    }

    #[test]
    fn noise_is_continuous_and_in_range() {
        let noise = Noise::new(9);
        let mut prev = noise.perlin2(0.0, 0.3);
        for i in 1..2000 {
            let x = i as f32 * 0.01;
            let p = noise.perlin2(x, 0.3);
            assert!((-1.0..=1.0).contains(&p));
            assert!((p - prev).abs() < 0.08, "jump at {x}");
            prev = p;
            let v = noise.value3(x, 1.7, -2.2);
            assert!((0.0..=1.0).contains(&v));
            let q = noise.perlin3(x, -0.4, 3.3);
            assert!((-1.0..=1.0).contains(&q));
        }
        assert_eq!(noise.perlin2(3.0, -4.0), 0.0);
        assert_eq!(noise.perlin3(3.0, -4.0, 1.0), 0.0);
    }

    #[test]
    fn noise_is_deterministic_and_seeded() {
        let a = Noise::new(1);
        let b = Noise::new(2);
        let f = Fractal::new(0.1, 5);
        assert_eq!(a.fbm2(4.2, 9.1, f), Noise::new(1).fbm2(4.2, 9.1, f));
        let differs =
            (0..32).any(|i| a.perlin2(i as f32 * 0.37, 0.5) != b.perlin2(i as f32 * 0.37, 0.5));
        assert!(differs);
        let v = a.value_fbm3(1.0, 2.0, 3.0, f);
        assert!((0.0..=1.0).contains(&v));
        let fb = a.fbm3(1.5, 2.5, 3.5, f);
        assert!((-1.0..=1.0).contains(&fb));
    }

    #[test]
    fn perlin_uses_the_whole_range() {
        let noise = Noise::new(5);
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        for y in 0..200 {
            for x in 0..200 {
                let p = noise.perlin2(x as f32 * 0.093, y as f32 * 0.071);
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        assert!(lo < -0.6 && hi > 0.6, "{lo} {hi}");
    }
}
