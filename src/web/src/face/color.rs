//! Colour for the portrait renderer.
//!
//! [`Rgb`] is a colour as written — `#rrggbb`, sRGB, 0..255 — and is what
//! the palettes and the club records hand over. [`Linear`] is the same
//! colour as light: every mix and every shade is worked out in it, because
//! mixing and multiplying encoded sRGB values muddies every colour it
//! touches.

use std::ops::{Add, AddAssign, Mul};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const fn new(r: f32, g: f32, b: f32) -> Rgb {
        Rgb { r, g, b }
    }

    /// Parses `#rrggbb`; anything malformed reads as mid grey rather than
    /// failing, because a wrong colour on one player beats a panic on all.
    pub fn hex(hex: &str) -> Rgb {
        let h = hex.trim_start_matches('#');
        let ch = |i: usize| {
            h.get(i..i + 2)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .unwrap_or(128) as f32
        };
        Rgb::new(ch(0), ch(2), ch(4))
    }

    pub fn css(self) -> String {
        let [r, g, b] = self.bytes();
        format!("#{r:02X}{g:02X}{b:02X}")
    }

    pub fn bytes(self) -> [u8; 3] {
        [
            self.r.round().clamp(0.0, 255.0) as u8,
            self.g.round().clamp(0.0, 255.0) as u8,
            self.b.round().clamp(0.0, 255.0) as u8,
        ]
    }

    pub fn linear(self) -> Linear {
        Linear::new(
            Self::decode(self.r / 255.0),
            Self::decode(self.g / 255.0),
            Self::decode(self.b / 255.0),
        )
    }

    fn decode(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
}

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.css())
    }
}

/// Linear-light RGB.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Linear {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Linear {
    pub const BLACK: Linear = Linear::new(0.0, 0.0, 0.0);

    pub const fn new(r: f32, g: f32, b: f32) -> Linear {
        Linear { r, g, b }
    }

    pub const fn gray(v: f32) -> Linear {
        Linear::new(v, v, v)
    }

    pub fn hex(hex: &str) -> Linear {
        Rgb::hex(hex).linear()
    }

    /// Linear mix toward `other`; `t` = 0 is self, 1 is other.
    pub fn mix(self, other: Linear, t: f32) -> Linear {
        self * (1.0 - t) + other * t
    }

    pub fn luma(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    /// Pushes saturation away from (t > 0) or toward (t < 0) its own grey.
    pub fn saturate(self, t: f32) -> Linear {
        let l = Linear::gray(self.luma());
        (l + (self + l * -1.0) * (1.0 + t)).max0()
    }

    /// Raises each channel to a power — the way a pigment deepens as it is
    /// layered, which darkens and saturates together.
    pub fn deepen(self, k: f32) -> Linear {
        Linear::new(self.r.powf(k), self.g.powf(k), self.b.powf(k))
    }

    pub fn max0(self) -> Linear {
        Linear::new(self.r.max(0.0), self.g.max(0.0), self.b.max(0.0))
    }

    /// Back to 8-bit sRGB after the tone curve.
    pub fn encode(self) -> [u8; 3] {
        [
            Self::encode_channel(self.r),
            Self::encode_channel(self.g),
            Self::encode_channel(self.b),
        ]
    }

    fn encode_channel(c: f32) -> u8 {
        let c = c.clamp(0.0, 1.0);
        let s = if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0 + 0.5) as u8
    }
}

impl Add for Linear {
    type Output = Linear;
    fn add(self, o: Linear) -> Linear {
        Linear::new(self.r + o.r, self.g + o.g, self.b + o.b)
    }
}

impl AddAssign for Linear {
    fn add_assign(&mut self, o: Linear) {
        self.r += o.r;
        self.g += o.g;
        self.b += o.b;
    }
}

impl Mul for Linear {
    type Output = Linear;
    fn mul(self, o: Linear) -> Linear {
        Linear::new(self.r * o.r, self.g * o.g, self.b * o.b)
    }
}

impl Mul<f32> for Linear {
    type Output = Linear;
    fn mul(self, k: f32) -> Linear {
        Linear::new(self.r * k, self.g * k, self.b * k)
    }
}
