//! Gradient noise: the pores, the blotches, the clumps of hair and the weave
//! of a shirt. Seeded per player, so the texture belongs to the man and not
//! to the render.

pub struct Noise {
    perm: [u8; 512],
}

impl Noise {
    pub fn new(seed: u64) -> Noise {
        let mut table: [u8; 256] = std::array::from_fn(|i| i as u8);
        let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
        for i in (1..256).rev() {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            table.swap(i, (z % (i as u64 + 1)) as usize);
        }
        Noise {
            perm: std::array::from_fn(|i| table[i & 255]),
        }
    }

    /// Perlin gradient noise, about −1..1.
    pub fn at(&self, x: f32, y: f32) -> f32 {
        let (fx, fy) = (x.floor(), y.floor());
        let (dx, dy) = (x - fx, y - fy);
        let xi = (fx as i32 & 255) as usize;
        let yi = (fy as i32 & 255) as usize;
        let p = &self.perm;
        let a = p[xi] as usize + yi;
        let b = p[xi + 1] as usize + yi;
        let (u, v) = (Self::fade(dx), Self::fade(dy));
        let n00 = Self::grad(p[a], dx, dy);
        let n10 = Self::grad(p[b], dx - 1.0, dy);
        let n01 = Self::grad(p[a + 1], dx, dy - 1.0);
        let n11 = Self::grad(p[b + 1], dx - 1.0, dy - 1.0);
        let top = n00 + (n10 - n00) * u;
        let bottom = n01 + (n11 - n01) * u;
        (top + (bottom - top) * v) * 1.41
    }

    /// Octaves of [`Self::at`], normalised back to about −1..1.
    pub fn fbm(&self, x: f32, y: f32, octaves: u32) -> f32 {
        let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
        for o in 0..octaves {
            // Each octave is shifted so their lattices never line up
            let shift = o as f32 * 17.31;
            sum += amp * self.at(x * freq + shift, y * freq - shift);
            norm += amp;
            amp *= 0.5;
            freq *= 2.03;
        }
        sum / norm
    }

    /// One-dimensional noise along a line — strands and lashes are indexed
    /// by a single coordinate across them.
    pub fn line(&self, t: f32) -> f32 {
        self.at(t, 0.37)
    }

    /// Cellular noise: distance to the nearest of a jittered lattice of
    /// points, one per unit cell, and a 0..1 value of that point's own —
    /// pores, follicles, anything scattered rather than flowing.
    pub fn cells(&self, x: f32, y: f32) -> (f32, f32) {
        let (fx, fy) = (x.floor(), y.floor());
        let mut best = (f32::MAX, 0.0);
        for dj in -1..=1 {
            for di in -1..=1 {
                let (ci, cj) = (fx as i32 + di, fy as i32 + dj);
                let h = self.perm
                    [((ci & 255) as usize + self.perm[(cj & 255) as usize] as usize) & 511];
                let h2 = self.perm[(h as usize + 97) & 511];
                let px = ci as f32 + h as f32 / 255.0;
                let py = cj as f32 + h2 as f32 / 255.0;
                let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
                if d < best.0 {
                    best = (d, self.perm[(h2 as usize + 31) & 511] as f32 / 255.0);
                }
            }
        }
        best
    }

    fn fade(t: f32) -> f32 {
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    fn grad(hash: u8, x: f32, y: f32) -> f32 {
        match hash & 7 {
            0 => x + y,
            1 => x - y,
            2 => -x + y,
            3 => -x - y,
            4 => x,
            5 => -x,
            6 => y,
            _ => -y,
        }
    }
}
