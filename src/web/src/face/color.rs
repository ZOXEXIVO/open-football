//! Colour arithmetic for the portrait painter.
//!
//! Everything is an sRGB triple. Blending in gamma space is deliberate: the
//! painter mixes colours the way a browser composites them, so a tone worked
//! out here lands on screen the same as it would if it were painted by hand.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const WHITE: Rgb = Rgb::new(255.0, 255.0, 255.0);
    pub const BLACK: Rgb = Rgb::new(0.0, 0.0, 0.0);

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

    /// From hue (degrees), saturation and lightness in 0..1 — for the
    /// fallback jersey hue of a player without a club.
    pub fn hsl(h: f32, s: f32, l: f32) -> Rgb {
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let hp = (h.rem_euclid(360.0)) / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r, g, b) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        Rgb::new((r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0)
    }

    pub fn css(self) -> String {
        format!(
            "#{:02X}{:02X}{:02X}",
            self.r.round().clamp(0.0, 255.0) as u8,
            self.g.round().clamp(0.0, 255.0) as u8,
            self.b.round().clamp(0.0, 255.0) as u8
        )
    }

    /// Multiplies every channel: below 1 darkens, above 1 brightens.
    pub fn shade(self, f: f32) -> Rgb {
        Rgb::new(self.r * f, self.g * f, self.b * f).clamped()
    }

    /// Linear mix toward `other`; `t` = 0 is self, 1 is other.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        Rgb::new(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
        )
    }

    /// Mix toward a hex string — most tints are written as literals.
    pub fn toward(self, hex: &str, t: f32) -> Rgb {
        self.mix(Rgb::hex(hex), t)
    }

    /// Screen blend toward white: lifts the tone the way light does, so a
    /// highlight on dark skin stays a highlight of THAT skin instead of a
    /// grey smear.
    pub fn lift(self, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        Rgb::new(
            255.0 - (255.0 - self.r) * (1.0 - t),
            255.0 - (255.0 - self.g) * (1.0 - t),
            255.0 - (255.0 - self.b) * (1.0 - t),
        )
    }

    /// Relative luminance in 0..1, for deciding how strong a tint should be
    /// on a given complexion.
    pub fn luma(self) -> f32 {
        (0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b) / 255.0
    }

    /// Pulls saturation toward grey by `t`.
    pub fn desaturate(self, t: f32) -> Rgb {
        let l = self.luma() * 255.0;
        self.mix(Rgb::new(l, l, l), t)
    }

    /// Pushes saturation away from grey by `t` (a negative desaturate,
    /// clamped to the gamut).
    pub fn saturate(self, t: f32) -> Rgb {
        let l = self.luma() * 255.0;
        Rgb::new(
            l + (self.r - l) * (1.0 + t),
            l + (self.g - l) * (1.0 + t),
            l + (self.b - l) * (1.0 + t),
        )
        .clamped()
    }

    fn clamped(self) -> Rgb {
        Rgb::new(
            self.r.clamp(0.0, 255.0),
            self.g.clamp(0.0, 255.0),
            self.b.clamp(0.0, 255.0),
        )
    }
}

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.css())
    }
}
