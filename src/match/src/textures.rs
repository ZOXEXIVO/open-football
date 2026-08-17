use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::f32::consts::{PI, TAU};

/// Where the features of a face land on the head mesh that carries it.
///
/// Handed over by whoever built the skull (see
/// [`crate::body::BodyParts::face_layout`]) rather than guessed here, because
/// an eye painted at a height the mesh does not have there ends up somewhere
/// on a cheekbone and nothing downstream can tell.
pub struct FaceLayout {
    /// Bottom of the head's lathe and how much of it there is, in metres of
    /// the head's own space. The texture's `v` runs over exactly this.
    pub foot: f32,
    pub span: f32,
    /// Half-width of the skull at eye level. Turns an angle round the head
    /// into a distance ACROSS the face, which is what keeps an eye an eye
    /// rather than an ellipse that stretches with the mesh.
    pub cheek: f32,
    /// Metric heights, in the same space.
    pub eyes: f32,
    pub brow: f32,
    pub nostrils: f32,
    pub mouth: f32,
    pub chin: f32,
    pub hairline: f32,
}

/// How much of a footballer's face is hair.
///
/// Four of them rather than a single on/off, because it is the cheapest kind
/// of variety there is: two players who differ only in the shape of a beard
/// are told apart instantly at a distance where no other feature resolves at
/// all.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Beard {
    Clean,
    Stubble,
    Goatee,
    Full,
}

/// Everything about one player's face that is his rather than his club's.
pub struct FaceLook {
    pub skin: Color,
    pub hair: Color,
    pub eyes: Color,
    /// How heavy the brows are, 0..1.
    pub brow: f32,
    pub beard: Beard,
    /// True when no hair cap is hung over the skull, so the scalp has to carry
    /// its own shadow. A bare head with nothing on it reads as a mannequin;
    /// a shaved one reads as a footballer.
    pub shaved: bool,
}

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

    /// The play triangle, in the unit square the transport icons are drawn in.
    ///
    /// The left margin is wider than the right on purpose. A triangle centred
    /// on its bounding box reads as though it has slipped left, because all
    /// its mass is down that end and the eye weighs the shape rather than the
    /// box; every media control in the world nudges it back toward the apex.
    /// Slightly taller than it is wide (0.90 against 0.74) for the same
    /// reason — a tapering shape needs the height to hold its own beside a
    /// pair of rectangles.
    const PLAY_LEFT: f32 = 0.16;
    const PLAY_APEX: f32 = 0.90;
    const PLAY_HALF: f32 = 0.45;

    /// The pause bars: two uprights, each a shade narrower than the gap
    /// between them, and shorter than the triangle is tall. Sized by weight
    /// rather than by extent — two solid rectangles cover far more of the
    /// square than a triangle does, and matching their bounding boxes would
    /// make the button visibly thicken every time playback started.
    const PAUSE_BAR: f32 = 0.24;
    const PAUSE_GAP: f32 = 0.20;
    const PAUSE_TOP: f32 = 0.08;

    /// Resolution the transport icons are rasterised at.
    ///
    /// Small on purpose, for the reason [`Self::number`] gives at more length:
    /// there are no mipmaps here, and these land about thirty device pixels
    /// wide. Drawing them at 32 keeps the texture close to 1:1 with the pixels
    /// it covers instead of asking the sampler to throw three quarters of it
    /// away.
    const ICON_SIZE: u32 = 32;

    /// The play triangle, white on transparent.
    ///
    /// Drawn rather than typed. The only font shipped with the viewer is
    /// Bevy's built-in one, which carries ASCII and nothing else — so the
    /// nearest available glyph is `>`, a chevron, which means "next" on every
    /// control surface ever built and is what the bar used to show. Sixteen
    /// lines of coverage sampling buys the real thing.
    pub fn play_icon(images: &mut Assets<Image>) -> Handle<Image> {
        images.add(Self::mask(|x, y| {
            if x < Self::PLAY_LEFT || x > Self::PLAY_APEX {
                return false;
            }
            // How far across the triangle we are, and therefore how far the
            // two sloping edges have closed on the centre line.
            let across = (x - Self::PLAY_LEFT) / (Self::PLAY_APEX - Self::PLAY_LEFT);
            (y - 0.5).abs() <= Self::PLAY_HALF * (1.0 - across)
        }))
    }

    /// The pause bars, white on transparent.
    pub fn pause_icon(images: &mut Assets<Image>) -> Handle<Image> {
        let first = (1.0 - Self::PAUSE_BAR * 2.0 - Self::PAUSE_GAP) * 0.5;
        let second = first + Self::PAUSE_BAR + Self::PAUSE_GAP;
        images.add(Self::mask(|x, y| {
            if y < Self::PAUSE_TOP || y > 1.0 - Self::PAUSE_TOP {
                return false;
            }
            (x >= first && x <= first + Self::PAUSE_BAR)
                || (x >= second && x <= second + Self::PAUSE_BAR)
        }))
    }

    /// White throughout, with alpha from how much of each texel the shape
    /// covers — sampled on a 4×4 grid, which is what gives a hard-edged glyph
    /// a soft enough edge to sit still at this size. `inside` is asked about
    /// points in the unit square, origin top-left.
    fn mask(inside: impl Fn(f32, f32) -> bool) -> Image {
        const SUB: u32 = 4;
        let size = Self::ICON_SIZE;
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for row in 0..size {
            for column in 0..size {
                let mut covered = 0u32;
                for sub_y in 0..SUB {
                    for sub_x in 0..SUB {
                        let x = (column as f32 + (sub_x as f32 + 0.5) / SUB as f32) / size as f32;
                        let y = (row as f32 + (sub_y as f32 + 0.5) / SUB as f32) / size as f32;
                        if inside(x, y) {
                            covered += 1;
                        }
                    }
                }
                data.extend_from_slice(&[255, 255, 255, (covered * 255 / (SUB * SUB)) as u8]);
            }
        }
        Self::image(size, size, data)
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
    ///
    /// The shape is the panel's, not a free choice: the print goes on a curved
    /// sheet 23.3 cm round the shirt by 19 tall (`BodyParts::NUMBER_*`), and a
    /// texture of a different aspect stretches every digit by the difference.
    pub fn number(images: &mut Assets<Image>, number: u8) -> Handle<Image> {
        const WIDTH: u32 = 64;
        const HEIGHT: u32 = 52;

        Self::lettering(images, &number.to_string(), WIDTH, HEIGHT, 0.94, 1.0)
    }

    /// And the player's name, across the shoulders above it.
    ///
    /// Folded to the plain ASCII capitals the 5×7 grid can draw, because that
    /// is what a shirt carries: real kits print names unaccented for exactly
    /// the same reason, which is that the lettering is a stencil. `None` when
    /// nothing survives the fold, and the shirt then goes out with its number
    /// alone rather than with a row of blanks.
    pub fn name(images: &mut Assets<Image>, name: &str) -> Option<Handle<Image>> {
        /// Wide and short: the panel is 25 cm round the shoulders by 5.8 tall,
        /// and this is that shape.
        const WIDTH: u32 = 176;
        const HEIGHT: u32 = 40;
        /// Past this a name is set so small it is a smudge either way, and
        /// real shirts abbreviate rather than shrink indefinitely.
        const LETTERS: usize = 16;

        let mut printed = Self::fold(name);
        if printed.chars().count() > LETTERS {
            printed = printed.chars().take(LETTERS).collect();
        }
        if printed.trim().is_empty() {
            return None;
        }
        Some(Self::lettering(images, &printed, WIDTH, HEIGHT, 0.94, 0.88))
    }

    /// White glyphs on transparent, centred and sized to fill the image.
    ///
    /// `margin` is the share of the width they may use and `cap` the share of
    /// the height, which is what separates a shirt number — as tall as the
    /// panel will take — from a name, which is set smaller with air above and
    /// below it.
    fn lettering(
        images: &mut Assets<Image>,
        text: &str,
        width: u32,
        height: u32,
        margin: f32,
        cap: f32,
    ) -> Handle<Image> {
        const SAMPLES: u32 = 4;

        let glyphs = Self::glyphs(text);
        let span = Self::span(&glyphs);
        let cell = (width as f32 * margin / span.max(1.0)).min(height as f32 * cap / 7.4);
        let left = (width as f32 - span * cell) * 0.5;
        let top = (height as f32 - 7.0 * cell) * 0.5;

        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let ink = Self::ink(&glyphs, x, y, left, top, cell, SAMPLES);
                data.extend_from_slice(&[255, 255, 255, (ink * 255.0) as u8]);
            }
        }

        images.add(Self::image(width, height, data))
    }

    /// Everything the 5×7 grid can draw, and nothing else: capitals, digits
    /// and the two marks that turn up inside surnames.
    ///
    /// The accented letters are folded onto their base rather than dropped —
    /// losing the acute off an O is a shirt printer's compromise, losing the O
    /// is a bug. Anything with no Latin base at all is dropped, and a name
    /// made entirely of those comes back empty for the caller to notice.
    fn fold(name: &str) -> String {
        let mut printed = String::with_capacity(name.len());
        for character in name.chars() {
            let folded: &str = match character {
                'a'..='z' | 'A'..='Z' | '0'..='9' => {
                    printed.push(character.to_ascii_uppercase());
                    continue;
                }
                // No leading, doubled or trailing separators: a stencil cannot
                // show what a dropped character was standing in for, so a name
                // that folds to nothing between two hyphens must not print as
                // a pair of hyphens.
                ' ' | '-' | '\'' | '.' => {
                    if !printed.is_empty() && !printed.ends_with(Self::SEPARATORS) {
                        printed.push(character);
                    }
                    continue;
                }
                'à'..='å' | 'ā' | 'ă' | 'ą' | 'À'..='Å' | 'Ā' | 'Ă' | 'Ą' => "A",
                'æ' | 'Æ' => "AE",
                'ç' | 'ć' | 'č' | 'ĉ' | 'ċ' | 'Ç' | 'Ć' | 'Č' | 'Ĉ' | 'Ċ' => "C",
                'ď' | 'đ' | 'ð' | 'Ď' | 'Đ' | 'Ð' => "D",
                'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'È'..='Ë' | 'Ē' | 'Ė' | 'Ę' | 'Ě' => {
                    "E"
                }
                'ĝ' | 'ğ' | 'ġ' | 'ģ' | 'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => "G",
                'ì'..='ï' | 'ī' | 'į' | 'ı' | 'Ì'..='Ï' | 'Ī' | 'Į' | 'İ' => "I",
                'ķ' | 'Ķ' => "K",
                'ĺ' | 'ļ' | 'ľ' | 'ł' | 'Ĺ' | 'Ļ' | 'Ľ' | 'Ł' => "L",
                'ñ' | 'ń' | 'ņ' | 'ň' | 'Ñ' | 'Ń' | 'Ņ' | 'Ň' => "N",
                'ò'..='ö' | 'ø' | 'ō' | 'ő' | 'Ò'..='Ö' | 'Ø' | 'Ō' | 'Ő' => "O",
                'ŕ' | 'ř' | 'Ŕ' | 'Ř' => "R",
                'ś' | 'ş' | 'š' | 'ŝ' | 'Ś' | 'Ş' | 'Š' | 'Ŝ' => "S",
                'ß' => "SS",
                'ţ' | 'ť' | 'ŧ' | 'Ţ' | 'Ť' | 'Ŧ' => "T",
                'þ' | 'Þ' => "TH",
                'ù'..='ü' | 'ū' | 'ů' | 'ű' | 'ų' | 'Ù'..='Ü' | 'Ū' | 'Ů' | 'Ű' | 'Ų' => {
                    "U"
                }
                'ý' | 'ÿ' | 'ŷ' | 'Ý' | 'Ŷ' | 'Ÿ' => "Y",
                'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => "Z",
                _ => "",
            };
            printed.push_str(folded);
        }
        while printed.ends_with(Self::SEPARATORS) {
            printed.pop();
        }
        printed
    }

    /// The marks a surname may carry that are not letters. Held apart because
    /// they are also the marks that must not start or end one.
    const SEPARATORS: [char; 4] = [' ', '-', '\'', '.'];

    /// One player's face, as the texture that wraps the head lathe.
    ///
    /// A footballer's head used to be a skin-coloured egg with a cap of hair
    /// on it, which at any range closer than the halfway line is the single
    /// most obviously wrong thing in the scene: the eye goes to a face before
    /// it goes to anything else, and finding none reads as *mannequin* however
    /// good the running is.
    ///
    /// Painted in METRES on the surface of the skull rather than in texels,
    /// which is what keeps an eye the same shape whatever the mesh is doing
    /// around it — the head narrows by a third between the cheekbone and the
    /// chin, so a feature laid out in `uv` is a different size at every height
    /// it could sit at. See [`FaceLayout`] for the crossing between the two.
    pub fn face(images: &mut Assets<Image>, layout: &FaceLayout, look: &FaceLook) -> Handle<Image> {
        /// Wide enough that an eye — three centimetres of a fifty-six
        /// centimetre circumference — lands about nine texels across, which is
        /// the least an iris and a pupil can be told apart in.
        const WIDTH: u32 = 128;
        const HEIGHT: u32 = 96;

        images.add(Self::image(
            WIDTH,
            HEIGHT,
            Self::face_pixels(layout, look, WIDTH, HEIGHT),
        ))
    }

    /// The face's texels, apart from the image they end up in — so a whole
    /// squad's worth of them can be dumped and LOOKED at without a browser,
    /// which is the only way to review a generated face at all. See
    /// `textures::tests::dump_faces`.
    fn face_pixels(layout: &FaceLayout, look: &FaceLook, width: u32, height: u32) -> Vec<u8> {
        let painter = Painter::new(layout, look);
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            // Row 0 is the BOTTOM of the head — the base of the neck — and the
            // last row is the crown.
            //
            // Upside down to look at, and it has to be. A texture's `v` runs
            // DOWN from the top of the image (`Mesh::ATTRIBUTE_UV_0`: "[0,0]
            // is mapped to the top left"), while the lathe writes
            // `v = (y - foot) / span`, which climbs WITH the model. So the
            // crown samples the last row and the neck samples the first, and a
            // face painted the way it reads comes out mirrored about the
            // middle of the head: the eyes land on the jaw, under a mouth that
            // has gone up to the cheekbone. Which is exactly how it was
            // reported — "eyes on mouth".
            let up = (row as f32 + 0.5) / height as f32;
            let level = layout.foot + up * layout.span;
            for column in 0..width {
                // Radians round from the FRONT of the head, which the lathe
                // puts a quarter of the way along `u`.
                let angle = Self::wrap(((column as f32 + 0.5) / width as f32 - 0.25) * TAU);
                let colour = painter.texel(level, angle * layout.cheek, angle);
                data.extend_from_slice(&[
                    (colour.x.clamp(0.0, 1.0) * 255.0) as u8,
                    (colour.y.clamp(0.0, 1.0) * 255.0) as u8,
                    (colour.z.clamp(0.0, 1.0) * 255.0) as u8,
                    255,
                ]);
            }
        }
        data
    }

    /// An angle brought back into −π..π.
    fn wrap(angle: f32) -> f32 {
        (angle + PI).rem_euclid(TAU) - PI
    }

    /// A colour as the three numbers this file paints in. Everything here is
    /// written straight into an sRGB image, so these are the same values a
    /// `Color::srgb` was built from and no conversion belongs in between.
    fn tone(color: Color) -> Vec3 {
        let rgb = color.to_srgba();
        Vec3::new(rgb.red, rgb.green, rgb.blue)
    }

    fn over(base: Vec3, ink: Vec3, alpha: f32) -> Vec3 {
        base + (ink - base) * alpha.clamp(0.0, 1.0)
    }

    fn shade(base: Vec3, amount: f32) -> Vec3 {
        base * (1.0 - amount.clamp(0.0, 1.0))
    }

    /// How much of this texel the text covers, 0..1, supersampled `samples`
    /// squared. `left`/`top` place the first glyph's top-left corner and
    /// `cell` is the size of one grid unit, both in texels.
    fn ink(
        glyphs: &[[u8; 7]],
        x: u32,
        y: u32,
        left: f32,
        top: f32,
        cell: f32,
        samples: u32,
    ) -> f32 {
        let mut hits = 0u32;
        for sub_y in 0..samples {
            for sub_x in 0..samples {
                let sample = Vec2::new(
                    x as f32 + (sub_x as f32 + 0.5) / samples as f32,
                    y as f32 + (sub_y as f32 + 0.5) / samples as f32,
                );
                if Self::glyph_covers(glyphs, (sample.x - left) / cell, (sample.y - top) / cell) {
                    hits += 1;
                }
            }
        }
        hits as f32 / (samples * samples) as f32
    }

    /// Width of a run of glyphs in grid units — 5 columns each with one column
    /// of air between them, and no trailing air.
    fn span(glyphs: &[[u8; 7]]) -> f32 {
        (glyphs.len() as f32 * 6.0 - 1.0).max(0.0)
    }

    /// Whether the glyph grid is inked at this point, in grid units with the
    /// origin at the top-left of the first glyph.
    fn glyph_covers(glyphs: &[[u8; 7]], column: f32, row: f32) -> bool {
        if !(0.0..7.0).contains(&row) || column < 0.0 {
            return false;
        }
        let index = (column / 6.0) as usize;
        let Some(glyph) = glyphs.get(index) else {
            return false;
        };
        let local = column - index as f32 * 6.0;
        if local >= 5.0 {
            return false;
        }
        glyph[row as usize] & (1 << (4 - local as u8)) != 0
    }

    /// One 5×7 pattern per character. Unknown characters come back blank,
    /// which prints as a space rather than as a missing-glyph box — a
    /// hoarding is not a place to report a typo.
    fn glyphs(text: &str) -> Vec<[u8; 7]> {
        text.chars().map(Self::glyph).collect()
    }

    fn glyph(character: char) -> [u8; 7] {
        const BLANK: [u8; 7] = [0; 7];
        match character.to_ascii_uppercase() {
            digit @ '0'..='9' => Self::DIGITS[digit as usize - '0' as usize],
            letter @ 'A'..='Z' => Self::LETTERS[letter as usize - 'A' as usize],
            '-' => [0, 0, 0, 0b01110, 0, 0, 0],
            '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
            ':' => [0, 0b01100, 0b01100, 0, 0b01100, 0b01100, 0],
            '/' => [
                0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000,
            ],
            _ => BLANK,
        }
    }

    /// 5×7 glyphs, one `u8` per row, high bit leftmost.
    const DIGITS: [[u8; 7]; 10] = [
        [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
        ],
        [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
    ];

    /// 5×7 capitals, sharing the row format and the renderer with the digits
    /// above. `A` first; index with `letter - 'A'`.
    const LETTERS: [[u8; 7]; 26] = [
        [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ], // A
        [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ], // B
        [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ], // C
        [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ], // D
        [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ], // E
        [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ], // F
        [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ], // G
        [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ], // H
        [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ], // I
        [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ], // J
        [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ], // K
        [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ], // L
        [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ], // M
        [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ], // N
        [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ], // O
        [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ], // P
        [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ], // Q
        [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ], // R
        [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ], // S
        [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ], // T
        [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ], // U
        [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ], // V
        [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ], // W
        [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ], // X
        [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ], // Y
        [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ], // Z
    ];

    /// One advertising panel for the perimeter boards, to be tiled along
    /// them: the wordmark in lit white on the board's own dark tone, with an
    /// accent rule under it.
    ///
    /// **The height is the constraint, not the width.** A board is 0.95 m
    /// tall and lands about 60 px on screen from the near touchline and 22 px
    /// from the far one, so 64 texels tall is roughly 1:1 where it matters
    /// and a mild minification where it does not. There are no mipmaps in
    /// this scene — see [`Self::seats`] and [`Self::number`], which are sized
    /// against the same problem — and a sharper board would crawl and sparkle
    /// along the touchline every time the camera panned, which is the one
    /// thing that would make advertising look worse than no advertising.
    ///
    /// The panel is therefore 512 × 64 for roughly 7.6 m of hoarding, which
    /// puts the cap height at half the board and the text at a size a
    /// broadcast camera can actually read.
    pub fn hoarding(images: &mut Assets<Image>, text: &str) -> Handle<Image> {
        const WIDTH: u32 = 512;
        const HEIGHT: u32 = 64;
        const SAMPLES: u32 = 4;
        /// Share of the panel width the wordmark is allowed, so consecutive
        /// panels do not run into each other where they tile.
        const MARGIN: f32 = 0.88;
        /// Cap height as a fraction of the board — a real perimeter wordmark
        /// runs about half of one.
        const CAP: f32 = 0.46;
        /// Air between the baseline and the accent rule, and the rule's own
        /// thickness, both in grid units.
        const RULE_GAP: f32 = 0.9;
        const RULE_THICKNESS: f32 = 0.5;

        // Exactly the board's own colour, so the seam between an advertised
        // panel and the plain structure behind it never shows.
        let ground = Vec3::new(0.200, 0.230, 0.300);
        let letters = Vec3::new(0.95, 0.96, 0.99);
        // The seats' cyan. One accent runs through this ground rather than
        // three unrelated brand colours.
        let accent = Vec3::new(0.160, 0.600, 0.720);

        let glyphs = Self::glyphs(text);
        let span = Self::span(&glyphs);
        let cell = ((WIDTH as f32 * MARGIN) / span.max(1.0)).min(HEIGHT as f32 * CAP / 7.0);
        let left = (WIDTH as f32 - span * cell) * 0.5;
        // Wordmark and rule centred as one block, so the panel does not sit
        // high on the board with a band of empty ground under it.
        let rule = (cell * RULE_THICKNESS).max(1.5);
        let block = 7.0 * cell + RULE_GAP * cell + rule;
        let top = (HEIGHT as f32 - block) * 0.5;

        // A rule the width of the wordmark, a little under it.
        let rule_top = top + (7.0 + RULE_GAP) * cell;
        let rule_bottom = rule_top + rule;

        let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for y in 0..HEIGHT {
            let on_rule = (y as f32 + 0.5) >= rule_top && (y as f32 + 0.5) < rule_bottom;
            for x in 0..WIDTH {
                let within = (x as f32 + 0.5) >= left && (x as f32 + 0.5) < left + span * cell;
                let ink = Self::ink(&glyphs, x, y, left, top, cell, SAMPLES);
                let mut colour = ground + (letters - ground) * ink;
                if on_rule && within {
                    colour = accent;
                }
                data.extend_from_slice(&[
                    (colour.x * 255.0) as u8,
                    (colour.y * 255.0) as u8,
                    (colour.z * 255.0) as u8,
                    255,
                ]);
            }
        }

        let mut image = Self::image(WIDTH, HEIGHT, data);
        // Tiled along boards of two different lengths, so the repeat lives in
        // the sampler and the count in each board's `uv_transform`.
        if let ImageSampler::Descriptor(descriptor) = &mut image.sampler {
            descriptor.address_mode_u = ImageAddressMode::Repeat;
        }
        images.add(image)
    }

    /// Goal netting: a grid of cords on a transparent ground.
    ///
    /// # Why the net looked stationary
    ///
    /// The netting used to be drawn as a flat `srgba(1, 1, 1, 0.11)` sheet
    /// with no texture at all, and **an untextured surface has no visible
    /// motion**. `net.rs` was already bagging the mesh half a metre behind a
    /// goal — the deformation was there and correct — but an 11%-opaque
    /// featureless plane offers the eye nothing to track it by, so what the
    /// user saw was a net that did not move. Cords are not decoration here;
    /// they are the whole of how a net reads as a net and how its movement
    /// reads at all.
    ///
    /// # Sizing
    ///
    /// A goal net's mesh is about 12 cm across and its cord a few
    /// millimetres. Drawn honestly at broadcast distance that is a cord
    /// under a pixel wide, which sparkles and crawls as the camera pans —
    /// the same problem [`Self::hoarding`] and [`Self::seats`] are sized
    /// against, and there are no mipmaps in this scene. So the cord is
    /// deliberately drawn FAT relative to life (about a fifth of the mesh
    /// rather than a thirtieth) and softened at its edges: at range that
    /// integrates to an even haze of the right density instead of a moiré,
    /// and close up it reads as netting.
    ///
    /// Tiled by each panel's `uv_transform`, so one cell here is one mesh
    /// square on the goal and the count is a property of the panel's real
    /// size.
    pub fn netting(images: &mut Assets<Image>) -> Handle<Image> {
        /// Texels per mesh square, which is what sets how much of the panel is
        /// cord: a cord cannot be drawn thinner than the two texels either
        /// side of the line it runs along, so a bigger cell means a finer net.
        ///
        /// **Sized against how the net reads at RANGE, not up close.** The
        /// broadcast rig sits a fixed distance from the centre spot, so a goal
        /// is about two hundred pixels wide when one is scored — three pixels
        /// to a mesh square, well past where the mip chain has flattened the
        /// sheet into a haze. The density of that haze is the whole of what
        /// the eye gets, and it is `OPACITY × coverage`. At 32 texels a cell
        /// the coverage came to a sixth and the net screenshotted as faint as
        /// the untextured sheet it replaced; a quarter is what reads as
        /// netting from the halfway line while still being seen through.
        /// Same trade the perimeter boards make in [`Self::hoarding`].
        const CELL: u32 = 16;
        /// Cells in the sheet — four, so the base is 64 and the mip chain
        /// halves cleanly all the way to 1×1.
        const CELLS: u32 = 4;
        const SIZE: u32 = CELL * CELLS;
        /// Half-width of a cord, in texels, and the width of its soft edge.
        const CORD: f32 = 0.7;
        const FEATHER: f32 = 1.0;
        /// Opacity of a cord at its centre. Well short of 1: a goal net is
        /// twine, it is lit from in front, and it has to stay possible to see
        /// the ball through the back of it.
        const OPACITY: f32 = 0.85;

        // Nets are white or off-white; a faint warm grey keeps them from
        // glowing brighter than the lines painted on the grass.
        let cord = Vec3::new(0.93, 0.94, 0.95);

        let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                // Distance to the nearest cord centre on each axis. The cords
                // run along the cell boundaries, so the centre of a cell is
                // the middle of a hole.
                let to_line = |v: u32| -> f32 {
                    let within = (v as f32 + 0.5) % CELL as f32;
                    within.min(CELL as f32 - within)
                };
                let coverage = |d: f32| -> f32 { 1.0 - Self::smooth((d - CORD) / FEATHER) };
                // Either cord covers the texel — union, not sum, so the knots
                // where two cross do not come out twice as bright.
                let alpha = coverage(to_line(x)).max(coverage(to_line(y))) * OPACITY;
                data.extend_from_slice(&[
                    (cord.x * 255.0) as u8,
                    (cord.y * 255.0) as u8,
                    (cord.z * 255.0) as u8,
                    (alpha * 255.0) as u8,
                ]);
            }
        }

        images.add(Self::mipped_netting(SIZE, data))
    }

    /// The netting sheet, with a mip chain.
    ///
    /// **This is the one texture in the crate that cannot do without
    /// mipmaps.** Everything else here is stretched over a single quad or a
    /// lathe and is sized so that one texel lands on about one pixel — which
    /// is exactly why the other generators can be sharp and why the comments
    /// on [`Self::hoarding`] and [`Self::seats`] warn against making them
    /// sharper. A goal net is different in kind: the panel repeats the sheet
    /// **sixty times across**, so at broadcast distance a single pixel covers
    /// dozens of texels. Screenshotted without mipmaps the cords came out as
    /// *dotted lines* — the sampler taking one texel per pixel and hitting
    /// cord or hole more or less at random — which reads as a broken net
    /// rather than a fine one, and would crawl the moment the camera panned.
    /// No amount of tuning the cord width fixes that; it is undersampling,
    /// and the answer to undersampling is to pre-filter.
    ///
    /// A box filter is right here because the sheet's RGB is uniform — only
    /// the alpha varies — so averaging cannot bleed one colour into another,
    /// and the chain converges to a flat haze of exactly the net's own
    /// density. Which is what a goal net looks like from the far touchline.
    fn mipped_netting(size: u32, base: Vec<u8>) -> Image {
        let mut levels: Vec<(u32, Vec<u8>)> = vec![(size, base)];
        while levels.last().map(|(w, _)| *w).unwrap_or(1) > 1 {
            let (width, source) = levels.last().expect("seeded above");
            let (width, half) = (*width, (*width / 2).max(1));
            let mut next = Vec::with_capacity((half * half * 4) as usize);
            for y in 0..half {
                for x in 0..half {
                    for channel in 0..4 {
                        let mut sum = 0u32;
                        for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                            let sy = (y * 2 + dy).min(width - 1);
                            let sx = (x * 2 + dx).min(width - 1);
                            sum += source[((sy * width + sx) * 4 + channel) as usize] as u32;
                        }
                        next.push((sum / 4) as u8);
                    }
                }
            }
            levels.push((half, next));
        }

        let mip_level_count = levels.len() as u32;
        let mut data = Vec::new();
        for (_, level) in &levels {
            data.extend_from_slice(level);
        }

        // `Image::new` asserts that the buffer is exactly one mip level, so the
        // chain goes on through `new_uninit`.
        let mut image = Image::new_uninit(
            Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        // `TextureDataOrder` defaults to layer-major, which for one layer is
        // simply mip 0 then mip 1 then mip 2 — the order built above.
        image.texture_descriptor.mip_level_count = mip_level_count;
        image.data = Some(data);
        let mut sampler = ImageSamplerDescriptor::linear();
        // Repeat on BOTH axes: a panel is many mesh squares across and many
        // up, and the panel's own UVs say how many.
        sampler.address_mode_u = ImageAddressMode::Repeat;
        sampler.address_mode_v = ImageAddressMode::Repeat;
        image.sampler = ImageSampler::Descriptor(sampler);
        image
    }

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
            let polar = (y as f32 + 0.5) / HEIGHT as f32 * PI;
            for x in 0..WIDTH {
                let azimuth = (x as f32 + 0.5) / WIDTH as f32 * TAU;
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

    /// Smoothstep, clamped at both ends.
    ///
    /// The clamp is not decoration. `t²(3−2t)` turns back on itself outside
    /// 0..1 — it is 2.25 at t = 1.5 and 0 again at t = 1.5 exactly — so an
    /// unclamped argument does not merely saturate, it produces a mask that
    /// re-appears somewhere it has no business being. Painted onto a face
    /// that is a shadow beside the nose turning up on the cheekbone.
    fn smooth(value: f32) -> f32 {
        let value = value.clamp(0.0, 1.0);
        value * value * (3.0 - 2.0 * value)
    }
}

/// One face, painted a texel at a time.
///
/// Every measurement below is in METRES on the surface of the head, not in
/// texels: `height` up the skull in its own space and `across` the distance
/// round it from the centre-line of the face. That is the only way the
/// features stay the right size and shape as the mesh narrows under them.
struct Painter<'a> {
    layout: &'a FaceLayout,
    look: &'a FaceLook,
    skin: Vec3,
    hair: Vec3,
    iris: Vec3,
    lip: Vec3,
}

impl Painter<'_> {
    /// Past this many radians round from the front, the surface has turned far
    /// enough away that anything drawn on it is being seen edge-on and is
    /// smeared round the side of the skull. Everything that belongs to the
    /// FACE stops here; hair and beards, which genuinely go round, do not.
    const FRONT: f32 = 1.30;
    /// Centre of each eye from the mid-line. Real pupils sit about 63 mm
    /// apart, and a face is roughly five eyes wide.
    const APART: f32 = 0.032;
    /// Half-width and half-height of the opening.
    const OPEN_WIDE: f32 = 0.0165;
    const OPEN_TALL: f32 = 0.0080;
    const IRIS: f32 = 0.0064;
    const PUPIL: f32 = 0.0028;
    /// How far along the brow runs, from the bridge of the nose outward, and
    /// how far the outer end rides above the inner one — a brow without that
    /// angle reads as a fringe.
    const BROW_INNER: f32 = 0.009;
    const BROW_OUTER: f32 = 0.056;
    const BROW_THICK: f32 = 0.0058;
    const BROW_RISE: f32 = 0.009;
    /// Half-width of the nose at the nostrils, and how far out from the
    /// mid-line the hollows either side of it are painted — outside the nose
    /// mesh's own silhouette, or they are drawn on the back of it.
    const NOSE_WIDE: f32 = 0.016;
    const BESIDE_NOSE: f32 = 0.027;
    const MOUTH_WIDE: f32 = 0.026;
    /// Where a beard is cut: how far above the mouth the line sits in the
    /// middle of the face, how far it has climbed by the ear, and how far out
    /// that is.
    ///
    /// Eased rather than climbed in a straight line. A straight rise puts a
    /// sharp V under the chin and has the beard up under the eye three
    /// centimetres out; the real line hugs the jaw across the chin and then
    /// turns up the side of the face, which is what an ease does.
    const JAW_RISE: f32 = 0.016;
    const JAW_CLIMB: f32 = 0.075;
    const JAW_REACH: f32 = 0.090;
    const SCLERA: Vec3 = Vec3::new(0.80, 0.78, 0.75);
    const LASH: Vec3 = Vec3::new(0.10, 0.08, 0.07);
    const INK: Vec3 = Vec3::new(0.05, 0.04, 0.04);

    fn new<'a>(layout: &'a FaceLayout, look: &'a FaceLook) -> Painter<'a> {
        let skin = Textures::tone(look.skin);
        Painter {
            layout,
            look,
            skin,
            hair: Textures::tone(look.hair),
            iris: Textures::tone(look.eyes),
            // Lips are the skin with the blood closer to the surface: darker
            // and a good deal redder, and the same relation whatever the tone
            // underneath — which is why this is derived rather than picked.
            lip: (skin * 0.74 + Vec3::new(0.15, 0.02, 0.03)).min(Vec3::ONE),
        }
    }

    fn texel(&self, height: f32, across: f32, angle: f32) -> Vec3 {
        let mut colour = self.modelled(height, angle);
        if angle.abs() < Self::FRONT {
            colour = self.brows(self.eyes(colour, height, across), height, across);
            colour = self.nose(colour, height, across);
            colour = self.mouth(colour, height, across);
        }
        self.scalp(self.whiskers(colour, height, across, angle), height, angle)
    }

    /// The shading a head carries before anything is drawn on it.
    ///
    /// One directional light with no shadow maps lights a sphere as a sphere,
    /// so every bit of relief a face has — the temples falling away, the jaw
    /// in its own shade, the brow ridge catching the light — has to be in the
    /// paint. Without it the features sit on a flat disc.
    fn modelled(&self, height: f32, angle: f32) -> Vec3 {
        let turned = Textures::smooth((angle.abs() / 1.9).min(1.0));
        let mut colour = self.skin * (1.0 - 0.16 * turned);
        // Under the jaw and down the neck, which is in the head's own shadow
        // for the whole match.
        let under = Textures::smooth(((self.layout.chin + 0.012 - height) / 0.055).clamp(0.0, 1.0));
        colour *= 1.0 - 0.30 * under;
        // And the brow ridge, which catches it.
        let ridge =
            Textures::smooth((1.0 - (height - self.layout.brow).abs() / 0.020).clamp(0.0, 1.0));
        colour * (1.0 + 0.055 * ridge * (1.0 - turned))
    }

    fn eyes(&self, base: Vec3, height: f32, across: f32) -> Vec3 {
        let mut colour = base;
        for side in [-1.0f32, 1.0] {
            let sideways = across - side * Self::APART;
            let rise = height - self.layout.eyes;

            // The socket first: a soft shadow for the eye to sit in. Without
            // one, two white almonds on a flat plane read as buttons.
            let socket = Vec2::new(sideways / 0.036, rise / 0.028).length();
            colour = Textures::shade(
                colour,
                0.20 * Textures::smooth((1.0 - socket).clamp(0.0, 1.0)),
            );

            // The opening: a lens, wider than it is tall and pointed at both
            // corners, which is the shape that still says *eye* at nine texels
            // across. An ellipse says *bead*.
            let lid =
                Self::OPEN_TALL * (1.0 - (sideways / Self::OPEN_WIDE).powi(2)).max(0.0).sqrt();
            if lid <= 0.0 || rise.abs() > lid {
                continue;
            }
            colour = Self::SCLERA;
            // The upper lid throws a shadow across the top of it — the one
            // thing that stops an eye reading as a hole punched in a mask.
            colour = Textures::shade(colour, 0.34 * ((rise / lid).max(0.0)).powf(0.6));

            let ball = Vec2::new(sideways, rise - 0.0008).length();
            if ball < Self::IRIS {
                let inside = Textures::smooth(((Self::IRIS - ball) / 0.0018).clamp(0.0, 1.0));
                // A dark limbal ring round a lighter iris: the rim is most of
                // what makes an eye colour legible at all once the iris itself
                // is four texels wide.
                colour = Textures::over(self.iris * 0.5, self.iris, inside);
            }
            if ball < Self::PUPIL {
                colour = Self::INK;
            }
            // The catchlight. Two texels of white, and the single cheapest
            // thing in the whole figure that makes it look alive.
            if Vec2::new(sideways + 0.0024, rise - 0.0026).length() < 0.0015 {
                colour = Vec3::splat(0.94);
            }
            // And the lash line along the top lid.
            if rise > lid - 0.0018 {
                colour = Textures::over(colour, Self::LASH, 0.8);
            }
        }
        colour
    }

    fn brows(&self, base: Vec3, height: f32, across: f32) -> Vec3 {
        let reach = across.abs();
        if reach < Self::BROW_INNER || reach > Self::BROW_OUTER {
            return base;
        }
        let along = (reach - Self::BROW_INNER) / (Self::BROW_OUTER - Self::BROW_INNER);
        // The outer end rides higher, and the arch peaks two-thirds of the way
        // out rather than at the end of it.
        let line = self.layout.brow + Self::BROW_RISE * along * (2.0 - along) - 0.003;
        let thick = Self::BROW_THICK * (1.0 - 0.55 * along * along);
        let inside = ((thick - (height - line).abs()) / 0.0024).clamp(0.0, 1.0);
        Textures::over(
            base,
            self.hair * 0.82,
            Textures::smooth(inside) * self.look.brow,
        )
    }

    /// What is painted AROUND the nose, which is a mesh of its own.
    ///
    /// Everything inside the nose's own silhouette is wasted paint — it ends
    /// up behind a piece of geometry standing a centimetre and a half off the
    /// face. That is where the nostrils and the shading down the bridge used
    /// to go, and at four millimetres a texel the two flank shadows met in the
    /// middle and drew a dark bar down the middle of every face on the pitch.
    ///
    /// So this draws the two things a nose does to the face rather than to
    /// itself: the hollows either side of it, out where they can be seen, and
    /// the shadow it drops onto the lip.
    fn nose(&self, base: Vec3, height: f32, across: f32) -> Vec3 {
        // Beside it, from the inner corner of the eye down to the nostril.
        let along = ((height - (self.layout.nostrils - 0.004))
            / (self.layout.eyes + 0.006 - self.layout.nostrils))
            .clamp(0.0, 1.0);
        let hollow = Textures::smooth(1.0 - (across.abs() - Self::BESIDE_NOSE).abs() / 0.012)
            * Textures::smooth(along.min(1.0 - along) * 3.0);
        let mut colour = Textures::shade(base, 0.13 * hollow);
        // And the shadow the tip drops onto the lip, which is under the mesh
        // and therefore the one part of it anybody sees.
        let below =
            ((0.010 - (height - (self.layout.nostrils - 0.009)).abs()) / 0.010).clamp(0.0, 1.0);
        let width = ((Self::NOSE_WIDE + 0.006 - across.abs()) / 0.010).clamp(0.0, 1.0);
        colour = Textures::shade(
            colour,
            0.20 * Textures::smooth(below) * Textures::smooth(width),
        );
        colour
    }

    fn mouth(&self, base: Vec3, height: f32, across: f32) -> Vec3 {
        let span = 1.0 - (across / Self::MOUTH_WIDE).powi(2);
        if span <= 0.0 {
            return base;
        }
        let span = span.sqrt();
        let mut colour = base;
        // The lower lip catches the light and the upper one is in its own
        // shade, which is the only reason a mouth has a direction to it.
        let lower = ((0.0070 * span - (height - (self.layout.mouth - 0.0055)).abs()) / 0.0050)
            .clamp(0.0, 1.0);
        colour = Textures::over(colour, self.lip, 0.60 * Textures::smooth(lower));
        let upper = ((0.0050 * span - (height - (self.layout.mouth + 0.0040)).abs()) / 0.0042)
            .clamp(0.0, 1.0);
        colour = Textures::over(colour, self.lip * 0.82, 0.55 * Textures::smooth(upper));
        // And the line between them, which is the part that survives when
        // everything else has minified away.
        let seam = ((0.0026 * span - (height - self.layout.mouth).abs()) / 0.0016).clamp(0.0, 1.0);
        Textures::shade(colour, 0.55 * Textures::smooth(seam))
    }

    /// The beard, if he has one.
    ///
    /// Cheap variety and a great deal of it: two players who differ only in
    /// the shape of a beard are told apart instantly at a range where no other
    /// feature on them resolves at all.
    fn whiskers(&self, base: Vec3, height: f32, across: f32, angle: f32) -> Vec3 {
        let weight = match self.look.beard {
            Beard::Clean => return base,
            Beard::Stubble => 0.26,
            Beard::Goatee => 0.78,
            // Not opaque. A beard is hair over skin, and painted solid it
            // reads as a mask — which a grey one on a dark complexion very
            // much does.
            Beard::Full => 0.76,
        };

        // The moustache, which all three have: a band across the top lip.
        let mut mask = Textures::smooth(
            ((0.0075 - (height - (self.layout.mouth + 0.0125)).abs()) / 0.0060).clamp(0.0, 1.0),
        ) * Textures::smooth(((0.022 - across.abs()) / 0.010).clamp(0.0, 1.0));

        if self.look.beard == Beard::Goatee {
            // A patch on the chin, and nothing else.
            let patch = Vec2::new(
                across / 0.026,
                (height - (self.layout.mouth - 0.021)) / 0.023,
            )
            .length();
            mask = mask.max(Textures::smooth((1.0 - patch).clamp(0.0, 1.0)));
        } else {
            // Cut along the JAW, which is not a level line: it climbs toward
            // the ear, so a beard sits just under the lip in the middle and up
            // on the cheekbone at the sides. Cut level — as this was — the
            // result is a rectangle stuck across a face, and a grey one on an
            // older player reads as a surgical mask.
            let line = self.layout.mouth
                + Self::JAW_RISE
                + Self::JAW_CLIMB * Textures::smooth(across.abs() / Self::JAW_REACH);
            let below = Textures::smooth((line - height) / 0.018);
            // Rounded off round the sides rather than stopping at a wall,
            // where it meets the hair.
            let round = Textures::smooth(((1.40 - angle.abs()) / 0.45).clamp(0.0, 1.0));
            let throat =
                Textures::smooth(((height - (self.layout.chin - 0.016)) / 0.020).clamp(0.0, 1.0));
            mask = mask.max(below * round * throat);
        }

        // Lips show through all of it — a beard grows round a mouth, not over
        // one, and the mouth is the feature that survives longest as the head
        // minifies.
        let lips = Textures::smooth(
            ((0.009 - (height - self.layout.mouth).abs()) / 0.006).clamp(0.0, 1.0),
        ) * Textures::smooth(((0.024 - across.abs()) / 0.008).clamp(0.0, 1.0));
        mask *= 1.0 - 0.85 * lips;

        Textures::over(base, self.hair * 0.7, mask * weight)
    }

    /// The hairline, which is not a level line: it runs across the forehead
    /// and then drops away at the temples toward the top of the ear.
    ///
    /// The same curve serves both the shaved head and the shading under a cap
    /// of hair, because it is the same line — one has stubble above it and the
    /// other has a mesh.
    fn hairline(&self, angle: f32) -> f32 {
        self.layout.hairline - 0.050 * Textures::smooth((angle.abs() - 0.28) / 1.05)
    }

    /// What happens where the hair is — or where it is not.
    fn scalp(&self, base: Vec3, height: f32, angle: f32) -> Vec3 {
        let line = self.hairline(angle);
        if self.look.shaved {
            // A shaved head is not a bare scalp. Left bare it reads as a
            // mannequin; what it should read as is the wash of colour a
            // number-one leaves, which is what most of the players who have
            // one actually look like.
            let mask = Textures::smooth((height - line) / 0.022);
            return Textures::over(base, self.hair * 0.55, mask * 0.55);
        }
        // Under a cap of hair the skin goes into shadow as it disappears, so
        // the mesh's own edge lands on shade rather than on lit skin — which
        // is the difference between hair growing out of a head and a wig
        // resting on one.
        //
        // A NARROW band. Ramped over three centimetres, as it was, the shadow
        // covered the entire four centimetres of forehead there is between the
        // eyebrows and the hair, and every player took the field looking like
        // he was wearing the shade of his own fringe.
        Textures::shade(
            base,
            0.26 * Textures::smooth((height - (line - 0.014)) / 0.014),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::BodyParts;
    use crate::config::PlayerInfo;
    use crate::kit::Complexion;

    fn look(beard: Beard, shaved: bool) -> FaceLook {
        FaceLook {
            skin: Color::srgb(0.85, 0.66, 0.51),
            hair: Color::srgb(0.10, 0.08, 0.07),
            eyes: Color::srgb(0.38, 0.50, 0.58),
            brow: 1.0,
            beard,
            shaved,
        }
    }

    /// Brightness, which is all these assertions need: every feature on a face
    /// is either darker or lighter than the skin it sits in.
    fn value(colour: Vec3) -> f32 {
        colour.x * 0.2126 + colour.y * 0.7152 + colour.z * 0.0722
    }

    /// A shirt is printed with a stencil, and the stencil has capitals and
    /// digits in it. Everything else either folds onto a base letter or is
    /// dropped — losing the acute off an O is what a shirt printer does;
    /// losing the O is a bug.
    #[test]
    fn a_name_is_folded_onto_the_stencil() {
        assert_eq!(Textures::fold("Müller"), "MULLER");
        assert_eq!(Textures::fold("Nuñez"), "NUNEZ");
        assert_eq!(Textures::fold("Sørensen"), "SORENSEN");
        assert_eq!(Textures::fold("Šeško"), "SESKO");
        assert_eq!(Textures::fold("Weiß"), "WEISS");
        assert_eq!(Textures::fold("Łukasz"), "LUKASZ");
        assert_eq!(Textures::fold("Åkerman"), "AKERMAN");
        // Real surnames carry these, and they are in the 5×7 grid.
        assert_eq!(Textures::fold("O'Neill"), "O'NEILL");
        assert_eq!(Textures::fold("Van der Sar"), "VAN DER SAR");
        assert_eq!(Textures::fold("Alves-Silva"), "ALVES-SILVA");
        // Tidied rather than printed as found: no leading, trailing or
        // doubled punctuation, because a stencil cannot show what it is
        // standing in for.
        assert_eq!(Textures::fold("  de  Jong "), "DE JONG");
        assert_eq!(Textures::fold("-Smith-"), "SMITH");
        // And a name with nothing Latin in it comes back empty for the caller
        // to notice, rather than as a row of blanks on a shirt.
        assert_eq!(Textures::fold("日本"), "");
        assert!(Textures::name(&mut Assets::default(), "日本").is_none());
        assert!(Textures::name(&mut Assets::default(), "Kane").is_some());
    }

    /// The face goes onto the head the right way up.
    ///
    /// The one thing every other test here cannot see. They all ask the
    /// `Painter` where it puts a feature, and it puts them all exactly where
    /// it should — but a texture's `v` runs DOWN from the top of the image
    /// while the lathe's runs UP from the foot of the model, so the two
    /// disagree by a mirror, and the whole face went onto the head upside
    /// down. The eyes came out on the jaw with the mouth above them.
    ///
    /// So this one goes through the mesh's own `v` instead: the row the eyes
    /// are painted on must be the row the head samples at eye level.
    #[test]
    fn the_face_goes_on_the_head_the_right_way_up() {
        const WIDTH: u32 = 128;
        const HEIGHT: u32 = 96;

        let layout = BodyParts::face_layout();
        let pixels = Textures::face_pixels(&layout, &look(Beard::Clean, false), WIDTH, HEIGHT);
        // Exactly the `v` the lathe writes for a ring at this height, as an
        // image row.
        let row_of = |height: f32| {
            ((((height - layout.foot) / layout.span) * HEIGHT as f32) as u32).min(HEIGHT - 1)
        };
        let at = |column: u32, row: u32| {
            let index = ((row * WIDTH + column) * 4) as usize;
            Vec3::new(
                pixels[index] as f32,
                pixels[index + 1] as f32,
                pixels[index + 2] as f32,
            ) / 255.0
        };
        // The middle of the face is a quarter of the way round the lathe; one
        // eye sits `APART` metres of surface to the side of it.
        let front = WIDTH / 4;
        let eye = front + ((Painter::APART / layout.cheek / TAU) * WIDTH as f32) as u32;
        // Measured on the PUPIL rather than the white of the eye: the sclera
        // is only a tenth brighter than a light complexion, where the pupil is
        // near black and nothing else on a clean-shaven face comes close.
        let darkest = |row: u32| {
            (eye.saturating_sub(4)..eye + 4)
                .map(|column| value(at(column, row)))
                .fold(1.0f32, f32::min)
        };
        assert!(
            darkest(row_of(layout.eyes)) < 0.15,
            "no eye on the eye line: darkest there is {}",
            darkest(row_of(layout.eyes))
        );
        assert!(
            darkest(row_of(layout.mouth)) > 0.20,
            "an eye down on the mouth: {} — the face is upside down",
            darkest(row_of(layout.mouth))
        );

        // And the same mirror seen from the other end: the hair's shadow is up
        // under the hairline and the jaw's is down under the jaw.
        let cheek = value(at(front, row_of(layout.eyes - 0.030)));
        assert!(
            value(at(front, row_of(layout.hairline + 0.010))) < cheek,
            "the hairline's shade is not under the hair"
        );
        assert!(
            value(at(front, row_of(layout.chin - 0.040))) < cheek * 0.9,
            "the throat is not in the head's shadow"
        );
    }

    /// There is an eye where the layout says there is an eye — and only there.
    #[test]
    fn the_eyes_land_on_the_eye_line() {
        let layout = BodyParts::face_layout();
        let look = look(Beard::Clean, false);
        let painter = Painter::new(&layout, &look);
        let at = |height: f32, across: f32| painter.texel(height, across, across / layout.cheek);

        let cheek = at(layout.eyes, 0.068);
        for side in [-1.0f32, 1.0] {
            // The white of the eye, sampled inside the opening and outside the
            // iris — which is the only part of it that is white.
            let white = at(layout.eyes, side * (Painter::APART + 0.011));
            assert!(
                value(white) > value(cheek) + 0.10,
                "no sclera on the {side} side: {white:?} against cheek {cheek:?}"
            );
            // The pupil is the darkest thing on the head.
            let pupil = at(layout.eyes + 0.0008, side * Painter::APART);
            assert!(value(pupil) < 0.10, "no pupil: {pupil:?}");
            // And there is a brow over it somewhere in the centimetre above
            // the eye line — the arch decides exactly where, and that is the
            // arch's business rather than this test's.
            let darkest = (0..24)
                .map(|step| value(at(layout.brow - 0.006 + step as f32 * 0.0006, side * 0.034)))
                .fold(f32::MAX, f32::min);
            assert!(darkest < value(cheek) * 0.5, "no brow: {darkest}");
        }
        // Symmetric, because a face is.
        assert!(
            (value(at(layout.eyes, -Painter::APART)) - value(at(layout.eyes, Painter::APART)))
                .abs()
                < 1e-5
        );
        // And nothing on the back of his head.
        let behind = at(layout.eyes, PI * layout.cheek);
        assert!(value(behind) < value(cheek), "the skull is lit like a face");
        assert!(value(behind) > 0.05, "the back of his head is a hole");
    }

    /// The mouth, the nose and the shading under the jaw are all there, and
    /// all of them darker than the skin they sit in.
    #[test]
    fn the_lower_face_is_modelled() {
        let layout = BodyParts::face_layout();
        let look = look(Beard::Clean, false);
        let painter = Painter::new(&layout, &look);
        let at = |height: f32, across: f32| painter.texel(height, across, across / layout.cheek);

        let cheek = at(layout.eyes - 0.030, 0.062);
        assert!(
            value(at(layout.mouth, 0.0)) < value(cheek) * 0.8,
            "no mouth line"
        );
        // The shadow the nose drops onto the lip, which is the one part of a
        // painted nose that is not hidden behind the nose mesh.
        assert!(
            value(at(layout.nostrils - 0.009, 0.0)) < value(cheek) * 0.9,
            "the nose casts nothing"
        );
        // And the hollow beside it, which is painted out where it can be seen
        // rather than down the middle where it cannot.
        assert!(
            value(at(layout.nostrils + 0.020, Painter::BESIDE_NOSE)) < value(cheek) * 0.95,
            "no hollow beside the nose"
        );
        // Under the jaw and down the neck is in the head's own shade for the
        // whole match, which is what stops a chin reading as a chin-shaped
        // patch of the same flat colour as everything under it.
        assert!(
            value(at(layout.chin - 0.035, 0.0)) < value(cheek) * 0.85,
            "no shade under the jaw"
        );
    }

    /// A beard covers a jaw and leaves a forehead, and a shaved head is the
    /// other way round.
    #[test]
    fn hair_goes_where_hair_grows() {
        let layout = BodyParts::face_layout();
        let clean = look(Beard::Clean, false);
        let bearded = look(Beard::Full, false);
        let shaved = look(Beard::Clean, true);
        let jaw = (layout.chin + 0.014, 0.030);
        let scalp = (layout.hairline + 0.020, 0.0);

        let plain = Painter::new(&layout, &clean);
        let whiskers = Painter::new(&layout, &bearded);
        let bald = Painter::new(&layout, &shaved);
        let at = |painter: &Painter, (height, across): (f32, f32)| {
            painter.texel(height, across, across / layout.cheek)
        };

        assert!(
            value(at(&whiskers, jaw)) < value(at(&plain, jaw)) * 0.75,
            "the beard is not on the jaw"
        );
        // And it stops at the cheekbone rather than climbing to the eyes.
        let under_eye = (layout.eyes - 0.008, 0.030);
        assert!(
            (value(at(&whiskers, under_eye)) - value(at(&plain, under_eye))).abs() < 0.02,
            "the beard has reached his eyes"
        );
        assert!(
            value(at(&bald, scalp)) < value(at(&plain, scalp)),
            "a shaved head is a bare scalp"
        );
    }

    /// Writes a sheet of generated faces out as raw RGBA so they can be
    /// looked at.
    ///
    /// A face is the one thing in this crate that cannot be reviewed by
    /// assertion. The tests above pin that an eye is where an eye goes and
    /// that a beard is on the jaw; whether the result reads as a footballer
    /// is a question only an eye can answer, and building 28 MB of wasm to
    /// find out is not a loop anybody will run twice. Off by default:
    ///
    /// ```text
    /// MATCH_FACE_DUMP=<dir> cargo test --lib dump_faces -- --ignored
    /// ```
    ///
    /// Writes `faces.rgba` — a strip of them side by side, 128 texels each —
    /// plus its dimensions on stdout, for whatever turns raw pixels into a
    /// picture.
    #[test]
    #[ignore = "writes a file; run by hand when the face generator changes"]
    fn dump_faces() {
        const WIDTH: u32 = 128;
        const HEIGHT: u32 = 96;

        let Ok(directory) = std::env::var("MATCH_FACE_DUMP") else {
            panic!("set MATCH_FACE_DUMP to a directory");
        };
        let layout = BodyParts::face_layout();
        // One face per entry of the shared skin ramp, over consecutive ids —
        // the whole span the server can ask for, and the case the hash behind
        // the other features has to break up.
        let looks: Vec<FaceLook> = (0..12)
            .map(|index| {
                Complexion::face(&PlayerInfo {
                    id: 1000 + index as u32,
                    shirt_number: 1 + index as u8,
                    last_name: String::new(),
                    position: "ST".to_string(),
                    is_home: true,
                    skin: index as u8,
                    hair: (index as u8 * 3) % 10,
                    eyes: (index as u8 * 5) % 8,
                })
            })
            .collect();

        let across = WIDTH as usize * looks.len();
        let mut sheet = vec![0u8; across * HEIGHT as usize * 4];
        for (column, look) in looks.iter().enumerate() {
            let face = Textures::face_pixels(&layout, look, WIDTH, HEIGHT);
            for row in 0..HEIGHT as usize {
                // Turned over on the way out. The texture is stored with the
                // crown at the BOTTOM, because that is the end a lathe's `v`
                // arrives at — see [`Textures::face_pixels`] — and a sheet of
                // upside-down faces is no use to anybody trying to look at
                // them.
                let from = (HEIGHT as usize - 1 - row) * WIDTH as usize * 4;
                let to = (row * across + column * WIDTH as usize) * 4;
                sheet[to..to + WIDTH as usize * 4]
                    .copy_from_slice(&face[from..from + WIDTH as usize * 4]);
            }
        }

        let path = std::path::Path::new(&directory).join("faces.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", across, HEIGHT, path.display());
    }
}
