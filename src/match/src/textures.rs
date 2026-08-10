use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// The two round textures the viewer draws on the turf, generated rather than
/// shipped: a contact shadow and the team ring under a player's boots.
///
/// Both are white with a shaped alpha channel, so the material's base colour
/// decides what they end up looking like and one image serves every player.
pub struct Textures;

impl Textures {
    const SIZE: u32 = 96;

    /// A soft round shadow. Real shadow maps are the single most expensive
    /// thing this scene could ask a WebGL2 context for, and twenty-two figures
    /// standing on nothing look like they are hovering.
    pub fn blob(images: &mut Assets<Image>) -> Handle<Image> {
        images.add(Self::radial(|distance| {
            // Dense under the player, gone by the edge of the quad.
            Self::smooth(((1.0 - distance) / 0.75).clamp(0.0, 1.0))
        }))
    }

    /// The ring drawn round a player's feet in their team's colour.
    pub fn ring(images: &mut Assets<Image>) -> Handle<Image> {
        images.add(Self::radial(|distance| {
            const RADIUS: f32 = 0.80;
            const WIDTH: f32 = 0.17;
            Self::smooth(1.0 - ((distance - RADIUS) / WIDTH).abs().min(1.0))
        }))
    }

    /// A shirt number, white on transparent, for the panel across a player's
    /// back.
    ///
    /// Deliberately small. The number lands twenty-odd pixels wide on screen,
    /// and there are no mipmaps here — a high-resolution glyph would crawl and
    /// sparkle as the player moved. At this size the texture is close to 1:1
    /// with the pixels it covers, so it stays still. The glyphs come from a
    /// 5×7 grid supersampled 4×4 per texel, which is what softens their edges.
    pub fn number(images: &mut Assets<Image>, number: u8) -> Handle<Image> {
        const WIDTH: u32 = 64;
        const HEIGHT: u32 = 44;
        const SAMPLES: u32 = 4;

        let digits: Vec<usize> = if number >= 10 {
            vec![(number / 10) as usize, (number % 10) as usize]
        } else {
            vec![number as usize]
        };
        // Glyphs are 5 grid columns with one column of air between them.
        let span = digits.len() as f32 * 6.0 - 1.0;
        let cell = (WIDTH as f32 / (span + 1.2)).min(HEIGHT as f32 / 8.2);
        let left = (WIDTH as f32 - span * cell) * 0.5;
        let top = (HEIGHT as f32 - 7.0 * cell) * 0.5;

        let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let mut hits = 0u32;
                for sub_y in 0..SAMPLES {
                    for sub_x in 0..SAMPLES {
                        let sample = Vec2::new(
                            x as f32 + (sub_x as f32 + 0.5) / SAMPLES as f32,
                            y as f32 + (sub_y as f32 + 0.5) / SAMPLES as f32,
                        );
                        let column = (sample.x - left) / cell;
                        let row = (sample.y - top) / cell;
                        if Self::glyph_covers(&digits, column, row) {
                            hits += 1;
                        }
                    }
                }
                let alpha = (hits * 255 / (SAMPLES * SAMPLES)) as u8;
                data.extend_from_slice(&[255, 255, 255, alpha]);
            }
        }

        images.add(Self::image(WIDTH, HEIGHT, data))
    }

    /// Whether the glyph grid is inked at this point, in grid units with the
    /// origin at the top-left of the first digit.
    fn glyph_covers(digits: &[usize], column: f32, row: f32) -> bool {
        if row < 0.0 || row >= 7.0 || column < 0.0 {
            return false;
        }
        let index = (column / 6.0) as usize;
        let Some(digit) = digits.get(index) else {
            return false;
        };
        let local = column - index as f32 * 6.0;
        if local >= 5.0 {
            return false;
        }
        let bits = Self::DIGITS[*digit][row as usize];
        bits & (1 << (4 - local as u8)) != 0
    }

    /// 5×7 glyphs, one `u8` per row, high bit leftmost.
    const DIGITS: [[u8; 7]; 10] = [
        [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
    ];

    /// The classic black-and-white football, as an equirectangular map for a
    /// UV sphere.
    ///
    /// A real Telstar is a truncated icosahedron — twelve black pentagons and
    /// twenty white hexagons — and unwrapping that properly is a great deal of
    /// work for something that lands a handful of pixels across. What the eye
    /// actually reads at any distance is the twelve dark patches, so those are
    /// what this draws: one at each vertex of an icosahedron, which is exactly
    /// where the pentagons sit.
    ///
    /// Patch radius is kept to 18°, giving a ball that is about 70% white. Go
    /// much wider and the patches merge under minification and the ball turns
    /// grey instead of white, which is the opposite of what a football looks
    /// like from the far side of a pitch.
    pub fn football(images: &mut Assets<Image>) -> Handle<Image> {
        const WIDTH: u32 = 128;
        const HEIGHT: u32 = 64;
        /// Angular radius of a patch, in radians.
        const PATCH: f32 = 0.314;
        /// Width of the soft edge, which is all the anti-aliasing a shape this
        /// size needs.
        const EDGE: f32 = 0.055;

        // The twelve vertices of an icosahedron: (0, ±1, ±φ) and its cyclic
        // permutations, normalised onto the unit sphere.
        let phi = (1.0 + 5.0_f32.sqrt()) * 0.5;
        let mut vertices = Vec::with_capacity(12);
        for a in [-1.0_f32, 1.0] {
            for b in [-phi, phi] {
                vertices.push(Vec3::new(0.0, a, b).normalize());
                vertices.push(Vec3::new(a, b, 0.0).normalize());
                vertices.push(Vec3::new(b, 0.0, a).normalize());
            }
        }

        let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            // v runs top (+Y) to bottom (−Y), the way a UV sphere is wound.
            let polar = (y as f32 + 0.5) / HEIGHT as f32 * std::f32::consts::PI;
            for x in 0..WIDTH {
                let azimuth = (x as f32 + 0.5) / WIDTH as f32 * std::f32::consts::TAU;
                let point = Vec3::new(
                    polar.sin() * azimuth.cos(),
                    polar.cos(),
                    polar.sin() * azimuth.sin(),
                );

                // Nearest patch centre, as an angle.
                let nearest = vertices
                    .iter()
                    .map(|vertex| vertex.dot(point).clamp(-1.0, 1.0).acos())
                    .fold(f32::MAX, f32::min);
                let dark = Self::smooth(((PATCH - nearest) / EDGE).clamp(0.0, 1.0));

                // Leather white and panel black, neither of them absolute:
                // a pure-white ball blows out under the stadium light and a
                // pure-black patch reads as a hole in it.
                let light = 0.94;
                let shade = 0.09;
                let value = ((light + (shade - light) * dark) * 255.0) as u8;
                data.extend_from_slice(&[value, value, value, 255]);
            }
        }

        images.add(Self::image(WIDTH, HEIGHT, data))
    }

    /// A run of empty seats, for the face of a row of terracing.
    ///
    /// Seats have to be a texture, not geometry: a real bank holds thousands
    /// of them and they land two or three pixels wide from a camera a hundred
    /// metres away. What matters at that size is not the shape of a seat but
    /// the RHYTHM of them — a regular vertical beat, broken up just enough
    /// that it does not read as a printed grid.
    ///
    /// Sized so the texels come out slightly LARGER than the pixels they
    /// cover. The far bank is ~144 m of stand carrying about 256 seats, drawn
    /// roughly 2200 px wide, so 1024 texels across is a mild magnification —
    /// which stays still under a panning camera. Going finer would invert
    /// that and the whole stand would crawl and sparkle, for the same reason
    /// the shirt numbers are kept deliberately small (see [`Self::number`]):
    /// there are no mipmaps here.
    ///
    /// One row of seats, reused for every row of every stand. That is not a
    /// shortcut — seats in a real stand line up in columns.
    pub fn seats(images: &mut Assets<Image>) -> Handle<Image> {
        const WIDTH: u32 = 1024;
        const HEIGHT: u32 = 24;
        const SEATS: u32 = 256;
        /// Share of each seat's slot taken by the seat itself; the rest is the
        /// dark gap between one and the next.
        const FILL: f32 = 0.74;

        let slot = WIDTH as f32 / SEATS as f32;
        let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            // 0 at the top of the row's face, 1 at the bottom. The seat back
            // occupies the upper part; beneath it is the shadowed gap through
            // to the step below.
            let down = y as f32 / (HEIGHT - 1) as f32;
            let vertical = Self::smooth((1.0 - (down - 0.30).max(0.0) / 0.62).clamp(0.0, 1.0));
            for x in 0..WIDTH {
                let index = (x as f32 / slot).floor();
                let across = x as f32 / slot - index;
                // 1 in the middle of a seat, falling to 0 across the gap.
                let horizontal =
                    Self::smooth((1.0 - (across - 0.5).abs() / (FILL * 0.5)).clamp(0.0, 1.0));

                // Seats are not identical: moulding, wear and the light all
                // vary a little down a row, and a perfectly even run is the
                // one thing that gives a generated texture away.
                let jitter = 0.88 + 0.24 * Self::hash01(index as u32);
                let lit = (horizontal * vertical * jitter).clamp(0.0, 1.0);

                // Between the seat colour and the shadow behind them.
                //
                // Cyan: a real ground's seats are a team colour and a bright
                // one, and this is the single element that tells you you are
                // looking at a stadium rather than at a grey embankment. They
                // were a near-black blue-grey, which is how an unlit stand
                // photographs and not how one looks with the lights on.
                let seat = Vec3::new(0.160, 0.600, 0.720);
                let shade = Vec3::new(0.050, 0.190, 0.230);
                let colour = shade + (seat - shade) * lit;
                data.extend_from_slice(&[
                    (colour.x * 255.0) as u8,
                    (colour.y * 255.0) as u8,
                    (colour.z * 255.0) as u8,
                    255,
                ]);
            }
        }

        images.add(Self::image(WIDTH, HEIGHT, data))
    }

    /// Cheap deterministic 0..1 from an index — consecutive seats have to land
    /// on unrelated shades.
    fn hash01(index: u32) -> f32 {
        let mut hash = index.wrapping_mul(2_654_435_761);
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(2_246_822_519);
        ((hash ^ (hash >> 13)) % 1024) as f32 / 1023.0
    }

    /// White throughout, with `alpha` sampled on the distance from the centre —
    /// 0 at the middle of the image, 1 at the edge of the inscribed circle.
    fn radial(alpha: impl Fn(f32) -> f32) -> Image {
        let mut data = Vec::with_capacity((Self::SIZE * Self::SIZE * 4) as usize);
        let centre = (Self::SIZE as f32 - 1.0) * 0.5;
        for row in 0..Self::SIZE {
            for column in 0..Self::SIZE {
                let offset = Vec2::new(column as f32 - centre, row as f32 - centre) / centre;
                let value = (alpha(offset.length()).clamp(0.0, 1.0) * 255.0) as u8;
                data.extend_from_slice(&[255, 255, 255, value]);
            }
        }

        Self::image(Self::SIZE, Self::SIZE, data)
    }

    fn image(width: u32, height: u32, data: Vec<u8>) -> Image {
        let mut image = Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        // Everything here is a gradient or a glyph a few centimetres across on
        // screen — point sampling would draw them as a staircase.
        image.sampler = ImageSampler::linear();
        image
    }

    fn smooth(value: f32) -> f32 {
        value * value * (3.0 - 2.0 * value)
    }
}
