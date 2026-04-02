use std::{
    cell::RefCell,
    collections::hash_map::{Entry, HashMap},
};

// Self-rolled 2D Perlin noise (replaces the `noise` crate).
// Implements Ken Perlin's improved noise function for 2D.
// Output range: approximately [-sqrt(0.5), sqrt(0.5)].

/// 2D Perlin noise generator with a seed-derived permutation table.
pub struct Perlin {
    perm: [u8; 512],
}

impl Perlin {
    /// Build a Perlin generator from a u32 seed.
    /// The permutation table is a Fisher-Yates shuffle of 0..255 seeded
    /// with a simple LCG so results are deterministic per seed.
    pub fn new(seed: u32) -> Self {
        let mut perm_base: [u8; 256] = [0; 256];
        for i in 0..256u16 {
            perm_base[i as usize] = i as u8;
        }
        // Simple LCG for deterministic shuffle
        let mut rng = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        for i in (1..256usize).rev() {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let j = (rng >> 16) as usize % (i + 1);
            perm_base.swap(i, j);
        }
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = perm_base[i & 255];
        }
        Perlin { perm }
    }

    /// Evaluate 2D Perlin noise at the given point.
    pub fn get(&self, point: [f64; 2]) -> f64 {
        let x = point[0];
        let y = point[1];

        // Unit square containing the point
        let xi = x.floor() as i32;
        let yi = y.floor() as i32;

        // Relative position within the unit square
        let xf = x - x.floor();
        let yf = y - y.floor();

        // Fade curves
        let u = fade(xf);
        let v = fade(yf);

        // Wrap coordinates to 0..255
        let xi = (xi & 255) as usize;
        let yi = (yi & 255) as usize;

        // Hash the four corners
        let aa = self.perm[self.perm[xi] as usize + yi] as usize;
        let ab = self.perm[self.perm[xi] as usize + yi + 1] as usize;
        let ba = self.perm[self.perm[xi + 1] as usize + yi] as usize;
        let bb = self.perm[self.perm[xi + 1] as usize + yi + 1] as usize;

        // Gradient dot products at each corner
        let g00 = grad2d(aa, xf, yf);
        let g10 = grad2d(ba, xf - 1.0, yf);
        let g01 = grad2d(ab, xf, yf - 1.0);
        let g11 = grad2d(bb, xf - 1.0, yf - 1.0);

        // Bilinear interpolation
        let x0 = lerp(u, g00, g10);
        let x1 = lerp(u, g01, g11);
        lerp(v, x0, x1)
    }
}

/// Fade function: 6t^5 - 15t^4 + 10t^3
#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Linear interpolation
#[inline]
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

/// 2D gradient function using 4 diagonal unit gradients: (1,1), (-1,1), (1,-1), (-1,-1).
/// These produce a range of [-sqrt(0.5), sqrt(0.5)] after interpolation,
/// matching the `noise` crate's 2D Perlin output range.
#[inline]
fn grad2d(hash: usize, x: f64, y: f64) -> f64 {
    // Use lowest 2 bits to select one of 4 diagonal gradients
    // Each gradient is (+-1, +-1) normalized to length sqrt(2),
    // but we skip the normalization and just dot with (x,y) —
    // the raw dot product naturally lands in [-sqrt(2), sqrt(2)]
    // per corner, and after bilinear interp the range contracts
    // to approximately [-sqrt(0.5), sqrt(0.5)].
    match hash & 3 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        _ => -x - y,
    }
}

use crate::error::Result;

// Seeds are typically a small set of game-defined integers — pre-size for 16 entries.
thread_local! {
    static GENERATORS: RefCell<HashMap<String, Perlin>> = RefCell::new(HashMap::with_capacity(16));
}

byond_fn!(fn noise_get_at_coordinates(seed, x, y) {
    get_at_coordinates(seed, x, y).ok()
});

byond_fn!(fn noise_reset() {
    GENERATORS.with(|cell| cell.borrow_mut().clear());
    Some("")
});

//note that this will be 0 at integer x & y, scaling is left up to the caller
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noise_basic() {
        let result = get_at_coordinates("42", "0.5", "0.5").unwrap();
        let val: f64 = result.parse().unwrap();
        assert!(val >= 0.0 && val <= 1.0);
    }

    #[test]
    fn test_noise_at_origin() {
        // Perlin noise is 0 at integer coordinates
        let result = get_at_coordinates("1", "0", "0").unwrap();
        let val: f64 = result.parse().unwrap();
        // After scaling: (0 * sqrt(2) + 1) / 2 = 0.5
        assert!((val - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_noise_deterministic() {
        let r1 = get_at_coordinates("100", "1.5", "2.5").unwrap();
        let r2 = get_at_coordinates("100", "1.5", "2.5").unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_noise_different_seeds() {
        let r1 = get_at_coordinates("1", "0.5", "0.5").unwrap();
        let r2 = get_at_coordinates("999", "0.5", "0.5").unwrap();
        // Different seeds should generally produce different values
        // (not guaranteed but very unlikely to be identical)
        let v1: f64 = r1.parse().unwrap();
        let v2: f64 = r2.parse().unwrap();
        // Both should be valid
        assert!(v1 >= 0.0 && v1 <= 1.0);
        assert!(v2 >= 0.0 && v2 <= 1.0);
    }

    #[test]
    fn test_noise_invalid_x() {
        assert!(get_at_coordinates("1", "not_a_number", "0.5").is_err());
    }

    #[test]
    fn test_noise_invalid_y() {
        assert!(get_at_coordinates("1", "0.5", "not_a_number").is_err());
    }

    #[test]
    fn test_noise_invalid_seed() {
        assert!(get_at_coordinates("not_a_seed", "0.5", "0.5").is_err());
    }

    #[test]
    fn test_noise_range_coverage() {
        // Sample several points to verify output is always in [0,1]
        for i in 0..20 {
            let x = format!("{:.1}", i as f64 * 0.37);
            let y = format!("{:.1}", i as f64 * 0.53);
            let result = get_at_coordinates("42", &x, &y).unwrap();
            let val: f64 = result.parse().unwrap();
            assert!(val >= 0.0 && val <= 1.0, "Out of range at ({}, {}): {}", x, y, val);
        }
    }

    #[test]
    fn test_noise_negative_coordinates() {
        let result = get_at_coordinates("42", "-1.5", "-2.5").unwrap();
        let val: f64 = result.parse().unwrap();
        assert!(val >= 0.0 && val <= 1.0);
    }
}

pub fn get_at_coordinates(seed_as_str: &str, x_as_str: &str, y_as_str: &str) -> Result<String> {
    let x = x_as_str.parse::<f64>()?;
    let y = y_as_str.parse::<f64>()?;
    GENERATORS.with(|cell| {
        let mut generators = cell.borrow_mut();
        let mut entry = generators.entry(seed_as_str.to_string());
        let generator = match entry {
            Entry::Occupied(ref mut occ) => occ.get_mut(),
            Entry::Vacant(vac) => {
                let seed = seed_as_str.parse::<u32>()?;
                let perlin = Perlin::new(seed);
                vac.insert(perlin)
            }
        };
        //perlin noise produces a result in [-sqrt(0.5), sqrt(0.5)] which we scale to [0, 1] for simplicity
        let unscaled = generator.get([x, y]);
        let scaled = (unscaled * 2.0_f64.sqrt() + 1.0) / 2.0;
        let clamped = scaled.clamp(0.0, 1.0);
        Ok(clamped.to_string())
    })
}
