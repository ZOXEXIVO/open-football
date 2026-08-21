use crate::typeface::Stencil;
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

/// A picture of one player's head, cut out and ready to go onto the front of
/// his skull: his photograph where the game has one, and the drawn portrait
/// off his profile page where it does not.
///
/// Pixels rather than a URL, because by the time one of these exists the
/// browser has already decoded, cropped and scaled the picture for us — see
/// [`crate::portrait`], which is the only thing that builds one.
///
/// Straight RGBA, row-major from the TOP, and the alpha is load-bearing: a
/// photograph arrives with its studio background keyed out and a drawn
/// portrait with nothing behind it at all, so the transparent parts are
/// exactly where the head is NOT. Every one of those texels keeps the face
/// this crate painted, which is what makes a picture that is too narrow, too
/// short or simply missing degrade into a generated face rather than into a
/// hole.
pub struct Portrait {
    pub width: u32,
    pub height: u32,
    /// Square pixels, in the sense that matters: a step across is the same
    /// distance on the man as a step down. Everything below assumes it.
    pub pixels: Vec<u8>,
    /// Where his eyes are — the mid-line, the eye line, and how far apart the
    /// pupils sit IN PIXELS.
    ///
    /// Three numbers and not one of them a guess: they are measured off each
    /// picture as it arrives (see [`crate::portrait`]). The first cut of this
    /// assumed the framing instead — one set of constants for the whole photo
    /// library — and the library is not framed to one standard. A head shot
    /// cropped closer than the rest came out of it stretched half as wide
    /// again, because a face measured at 70 pixels was being told it was 50.
    pub centre: f32,
    pub eyes: f32,
    pub pupils: f32,
}

impl Portrait {
    /// The colour and coverage at a point on the skull: `height` metres up the
    /// head in its own space, `angle` radians round from the front.
    ///
    /// This is the whole crossing between a flat picture and a round head, and
    /// it is an orthographic projection — the picture is a slide and the head
    /// is what it is being projected onto. So the horizontal is `sin(angle)`
    /// and not the angle itself: a point 60° round the skull is `sin 60°` of
    /// the way across the face as the camera saw it, not two thirds of the
    /// way. Getting that wrong squeezes the eyes toward the nose and drags the
    /// corners of the mouth round the cheeks.
    ///
    /// The vertical is fitted on two landmarks rather than scaled: eye line to
    /// eye line, chin to chin. Which is what matters, because the head carries
    /// a NOSE as geometry — a picture whose nose lands anywhere but on it is a
    /// face with two noses.
    fn at(&self, layout: &FaceLayout, height: f32, angle: f32) -> Option<(Vec3, f32)> {
        let scale = self.scale();
        // Sideways is the ORTHOGRAPHIC offset and not the angle: a point 60°
        // round the skull is `sin 60°` of the way across the face as a camera
        // saw it, not two thirds of the way. The other version squeezes the
        // eyes toward the nose and drags the mouth round the cheeks.
        let u = self.centre + (angle.sin() * layout.cheek) * scale / self.width as f32;
        let v = self.eyes + (layout.eyes - height) * scale / self.height as f32;
        self.sample(u, v)
    }

    /// Pixels of picture per metre of head — ONE number for both axes, which
    /// is the whole point of it.
    ///
    /// Taken off the pupils, because the distance between a man's pupils is
    /// the one measurement on a face that is the same on everybody: 63 mm,
    /// give or take a couple. The head this is going onto has its own eyes
    /// [`Painter::APART`] from the mid-line and 64 mm apart, so matching the
    /// two puts a photographed eye exactly where a painted one would have
    /// gone — and fixes the scale of everything else at the same time.
    ///
    /// It fixes the scale of BOTH axes deliberately. Fitting each axis to the
    /// head separately is what stretches a face: this skull is nearly twice as
    /// wide as it is deep from eye line to chin, where a real head is about
    /// two thirds of that, so a picture told to fill it comes out a third too
    /// wide. Filling the head is not worth distorting the man — the sides of
    /// the skull, past where the picture reaches, are painted in the
    /// complexion the picture itself was measured for.
    fn scale(&self) -> f32 {
        /// Interpupillary distance, in metres. `Painter::APART` doubled: the
        /// two have to agree, or a photographed face and the painted face
        /// under it would be different sizes.
        const PUPILS: f32 = 0.064;

        self.pupils / PUPILS
    }

    /// Bilinear, because the picture arrives at roughly the same resolution as
    /// the face sheet and a nearest-neighbour lift at that ratio drops whole
    /// rows of an eyelid. Outside the picture there is nothing to sample and
    /// the generated face stands.
    fn sample(&self, u: f32, v: f32) -> Option<(Vec3, f32)> {
        if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
            return None;
        }
        let x = (u * self.width as f32 - 0.5).max(0.0);
        let y = (v * self.height as f32 - 0.5).max(0.0);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);

        let mut colour = Vec3::ZERO;
        let mut alpha = 0.0;
        for (px, py, weight) in [
            (x0, y0, (1.0 - fx) * (1.0 - fy)),
            (x1, y0, fx * (1.0 - fy)),
            (x0, y1, (1.0 - fx) * fy),
            (x1, y1, fx * fy),
        ] {
            let at = ((py * self.width + px) * 4) as usize;
            let texel = Vec3::new(
                self.pixels[at] as f32,
                self.pixels[at + 1] as f32,
                self.pixels[at + 2] as f32,
            ) / 255.0;
            let opacity = self.pixels[at + 3] as f32 / 255.0;
            // Weighted by opacity as well: a texel on the cut edge is half
            // background, and averaging its colour in unweighted drags a
            // rim of studio grey round the whole silhouette.
            colour += texel * (weight * opacity);
            alpha += weight * opacity;
        }
        if alpha <= 0.004 {
            return None;
        }
        Some((colour / alpha, alpha))
    }

    /// The mean of the cheeks and the bridge of the nose: the widest patch of
    /// plain lit skin a head shot has, taken between the eyes and the mouth
    /// and inside the width of the nose so that hair, beard and background
    /// stay out of it.
    ///
    /// Nothing is done to the picture with this. It is read so the REST of the
    /// player can be moved onto it — his neck, his arms and his legs wear a
    /// tone off the shared ramp, and the ramp entry nearest this one is the
    /// one that makes the man on the pitch the man in the photograph. The
    /// picture itself is laid down exactly as it arrived.
    pub fn cheek_tone(&self) -> Option<Vec3> {
        // In pupils: the cheeks run from half a pupil-width below the eyes to
        // a pupil-width and a half below, and reach a pupil-width either side
        // of the mid-line. Everything on a face scales with that distance, so
        // the band lands on cheek whatever the picture's framing.
        let (top, bottom) = self.band(0.5, 1.5);
        let (left, right) = self.column(1.0);

        let mut total = Vec3::ZERO;
        let mut count = 0.0;
        for row in 0..12 {
            for column in 0..12 {
                let v = top + (bottom - top) * (row as f32 + 0.5) / 12.0;
                let u = left + (right - left) * (column as f32 + 0.5) / 12.0;
                // Only the opaque ones: a narrow face leaves the outer
                // columns on keyed-out background, and averaging those in
                // would tell us the studio was the colour of his cheek.
                if let Some((colour, alpha)) = self.sample(u, v)
                    && alpha > 0.9
                {
                    total += colour;
                    count += 1.0;
                }
            }
        }
        (count > 8.0).then(|| total / count)
    }

    /// A band across the picture, measured DOWN from the eye line in pupil
    /// widths — the only ruler a face carries that means the same thing on
    /// every picture of every man.
    fn band(&self, from: f32, to: f32) -> (f32, f32) {
        let pupil = self.pupils / self.height as f32;
        (self.eyes + pupil * from, self.eyes + pupil * to)
    }

    /// …and the same measure across it, either side of the mid-line.
    fn column(&self, reach: f32) -> (f32, f32) {
        let pupil = self.pupils / self.width as f32;
        (self.centre - pupil * reach, self.centre + pupil * reach)
    }

    /// …and the same for the top of his head.
    ///
    /// Read for the same reason and used the same way: the cap of hair on the
    /// model is a MESH in a flat colour off the shared ramp, and it sits over
    /// the top of the picture. Left as it was, a photograph of a blond man
    /// gets a black cap and the head stops being his.
    ///
    /// The band is over the crown rather than at the hairline, and the crown
    /// is where the cap actually is. A bald man's crown reads as skin, which
    /// puts the nearest ramp entry on the palest hair there is — the right
    /// answer for a cap he should not be wearing at all.
    pub fn hair_tone(&self) -> Option<Vec3> {
        // Just above the hairline, which on any face sits about a pupil width
        // and a bit over the eyes. Higher than this and the band walks off
        // the top of the picture and finds nothing at all.
        let (top, bottom) = self.band(-1.55, -1.15);
        let (left, right) = self.column(0.7);

        let mut total = Vec3::ZERO;
        let mut count = 0.0;
        for row in 0..8 {
            for column in 0..8 {
                let v = top + (bottom - top) * (row as f32 + 0.5) / 8.0;
                let u = left + (right - left) * (column as f32 + 0.5) / 8.0;
                if let Some((colour, alpha)) = self.sample(u, v)
                    && alpha > 0.9
                {
                    total += colour;
                    count += 1.0;
                }
            }
        }
        (count > 6.0).then(|| total / count)
    }
}

/// The round texture the viewer draws on the turf, generated rather than
/// shipped: the contact shadow under a player and under the ball.
///
/// White with a shaped alpha channel, so the material's base colour decides
/// what it ends up looking like and one image serves every player. It had a
/// team-coloured ring beside it, drawn round each player's boots; that went
/// once the footballers themselves were legible enough to tell apart by their
/// kit, which is how you tell them apart in a broadcast.
/// The playing surface, as the two sheets that describe it.
///
/// One sheet, two questions: what colour the grass is, and which way it is
/// facing. They have to be generated together — the relief is taken off the
/// same rasterised blades the colour is — and they have to be handed out
/// together, since a material carrying one without the other is either a flat
/// picture of grass or grass with no colour in it.
pub struct Turf {
    /// What the grass looks like: blades in the shade it was asked for.
    pub albedo: Handle<Image>,
    /// Which way each leaf is facing, as a tangent-space normal map.
    pub relief: Handle<Image>,
}

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

    /// The altitude arrows on the touch controls: the same triangle stood on
    /// end. Centred rather than nudged toward its apex, which is the one place
    /// the play button's optical correction (see [`Self::PLAY_LEFT`]) would do
    /// harm — that shape sits alone in a wide box, where these two sit one
    /// directly above the other and any nudge reads as them being out of line
    /// with each other rather than as balance.
    const LIFT_BASE: f32 = 0.22;
    const LIFT_APEX: f32 = 0.78;
    const LIFT_HALF: f32 = 0.34;

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

    /// The altitude arrows, white on transparent: up on one button, down on
    /// the other.
    ///
    /// Drawn rather than typed for the reason [`Self::play_icon`] gives at
    /// length — the only font here carries ASCII, where the nearest thing to an
    /// arrow is a caret.
    pub fn lift_icon(images: &mut Assets<Image>, up: bool) -> Handle<Image> {
        images.add(Self::mask(move |x, y| {
            // The axis the triangle tapers along, running from its base to its
            // point. `y` runs DOWN the square, so an up arrow measures from the
            // bottom.
            let along = if up { 1.0 - y } else { y };
            if along < Self::LIFT_BASE || along > Self::LIFT_APEX {
                return false;
            }
            // How far the two sloping edges have closed on the centre line.
            let across = (along - Self::LIFT_BASE) / (Self::LIFT_APEX - Self::LIFT_BASE);
            (x - 0.5).abs() <= Self::LIFT_HALF * (1.0 - across)
        }))
    }

    /// The ball on the timeline, marking a goal.
    ///
    /// A disc with the panels taken OUT of it rather than drawn on: a mask
    /// carries one colour and an alpha, and cutting the dark panels away lets
    /// whatever the marker is filled with show through them. Two tones for
    /// free, and both of them the club's.
    ///
    /// What survives of the pattern at a dozen pixels is the pentagons: a big
    /// one in the middle and five smaller ones around it, sitting off its EDGES
    /// the way a real ball's do — the panels next to a pentagon are the ones
    /// across the hexagons from it, never the ones off its points. That
    /// distinction is not pedantry at this size, it is the whole picture (see
    /// below). A plain circle will not do: the playhead is one, and the two sit
    /// ten pixels apart on the same rail.
    ///
    /// Three earlier drafts are worth not repeating, because each looks right
    /// in the source and wrong on screen:
    ///
    /// - Round panels cut out of the RIM scallop the silhouette. A circle with
    ///   five bites out of it is a cog.
    /// - The same panels shrunk to fit inside the rim leave six dots of equal
    ///   weight. That is a button.
    /// - A big hub with seams running out of its VERTICES merges with them into
    ///   five points. That is a star — and it is what putting the satellites on
    ///   the vertex angles does too, since the hub already reaches its full
    ///   radius there and the shapes touch.
    ///
    /// ⚠ NOTHING REACHES THE EDGE. `RIM` is a band of solid colour that keeps
    /// the outline whole; all the pattern lives inside it.
    pub fn goal_icon(images: &mut Assets<Image>) -> Handle<Image> {
        const RADIUS: f32 = 0.47;
        const RIM: f32 = 0.405;
        const HUB: f32 = 0.20;
        const PANEL_AT: f32 = 0.30;
        const PANEL: f32 = 0.105;

        images.add(Self::mask(|x, y| {
            let (dx, dy) = (x - 0.5, y - 0.5);
            let distance_sq = dx * dx + dy * dy;
            if distance_sq > RADIUS * RADIUS {
                return false;
            }
            if distance_sq > RIM * RIM {
                return true;
            }
            // `y` runs down the square, so -π/2 is straight up: the hub's first
            // vertex points there, and the first satellite sits off the edge
            // clockwise of it.
            if Self::pentagon(dx, dy, HUB, 0.0) {
                return false;
            }
            (0..5).all(|panel| {
                let angle = -std::f32::consts::FRAC_PI_2
                    + std::f32::consts::PI / 5.0
                    + panel as f32 * std::f32::consts::TAU / 5.0;
                let (cx, cy) = (PANEL_AT * angle.cos(), PANEL_AT * angle.sin());
                // Turned so its own point faces outward, which is what stops
                // five little pentagons reading as five little circles.
                !Self::pentagon(dx - cx, dy - cy, PANEL, angle + std::f32::consts::FRAC_PI_2)
            })
        }))
    }

    /// And the exclamation, marking a chance that stayed out.
    ///
    /// Tapered rather than a plain bar, and the taper is the whole reason it
    /// can be drawn this small: at twelve pixels a stroke of even width beside
    /// a dot reads as a colon stood on end. The stem has to visibly come to a
    /// point for the eye to finish it.
    pub fn chance_icon(images: &mut Assets<Image>) -> Handle<Image> {
        const STEM_TOP: f32 = 0.14;
        const STEM_FOOT: f32 = 0.62;
        const STEM_HALF_TOP: f32 = 0.125;
        const STEM_HALF_FOOT: f32 = 0.075;
        const DOT_AT: f32 = 0.81;
        const DOT: f32 = 0.115;

        images.add(Self::mask(|x, y| {
            let across = (x - 0.5).abs();
            if (STEM_TOP..=STEM_FOOT).contains(&y) {
                let down = (y - STEM_TOP) / (STEM_FOOT - STEM_TOP);
                return across <= STEM_HALF_TOP + (STEM_HALF_FOOT - STEM_HALF_TOP) * down;
            }
            let below = y - DOT_AT;
            across * across + below * below <= DOT * DOT
        }))
    }

    /// Is this point inside a regular pentagon of `radius` centred on the
    /// origin, turned `rotation` radians from vertex-straight-up?
    ///
    /// Five half-plane tests, one per edge: an edge normal points at
    /// `-π/2 + π/5 + k·2π/5`, and every edge stands its apothem away from the
    /// centre along it.
    fn pentagon(dx: f32, dy: f32, radius: f32, rotation: f32) -> bool {
        let apothem = radius * (std::f32::consts::PI / 5.0).cos();
        (0..5).all(|edge| {
            let angle = -std::f32::consts::FRAC_PI_2
                + std::f32::consts::PI / 5.0
                + rotation
                + edge as f32 * std::f32::consts::TAU / 5.0;
            dx * angle.cos() + dy * angle.sin() <= apothem
        })
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

    /// A shirt number, white on transparent, for the panel across a player's
    /// back.
    ///
    /// Deliberately small. The number lands twenty-odd pixels wide on screen,
    /// and there are no mipmaps here — a high-resolution glyph would crawl and
    /// sparkle as the player moved. At this size the texture is close to 1:1
    /// with the pixels it covers, so it stays still.
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
    /// Set in capitals, which is what a shirt carries, and otherwise printed as
    /// spelled: the accents used to be flattened as well — CONCEICAO for
    /// Conceição — because the 5×7 grid this was drawn on had no glyph for them.
    /// The face does, and it is the same face the label over his head is set in,
    /// so a letter that can be printed is printed. [`Self::fold`] is now only a
    /// backstop for the characters neither face carries. `None` when nothing
    /// survives it, and the shirt then goes out with its number alone rather
    /// than with a row of blanks.
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
    ///
    /// The outlines come from [`Stencil`], which is to say from the same face
    /// the player's own label is set in. Everything on this player is now drawn
    /// with one typeface; what is left of the 5×7 grid below belongs to the
    /// hoardings, where a pixel letterform is the point rather than a limit.
    fn lettering(
        images: &mut Assets<Image>,
        text: &str,
        width: u32,
        height: u32,
        margin: f32,
        cap: f32,
    ) -> Handle<Image> {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for ink in Stencil::mask(text, width, height, margin, cap) {
            data.extend_from_slice(&[255, 255, 255, ink]);
        }

        images.add(Self::image(width, height, data))
    }

    /// The name as the shirt will carry it: capitals, and every character the
    /// face has a glyph for kept as it is.
    ///
    /// What is left of the old behaviour is the backstop. Letters the face
    /// cannot draw are folded onto their Latin base rather than dropped —
    /// losing the acute off an O is a shirt printer's compromise, losing the O
    /// is a bug. Anything with no Latin base at all is dropped, and a name made
    /// entirely of those comes back empty for the caller to notice.
    ///
    /// Uppercasing runs through `to_uppercase` rather than `to_ascii_uppercase`,
    /// now that there is more than ASCII to print — and it is taken WHOLE. A
    /// capital is not always one character: `ß` has none of its own and upper
    /// cases to `SS`, so taking the first character of the result prints
    /// WEIS.
    fn fold(name: &str) -> String {
        let mut printed = String::with_capacity(name.len());
        for character in name.chars() {
            // The separators are decided before anything else, because they are
            // the ones that must NOT simply be copied through: no leading,
            // doubled or trailing punctuation. A stencil cannot show what a
            // dropped character was standing in for, so a name that folds to
            // nothing between two hyphens must not print as a pair of hyphens.
            if Self::SEPARATORS.contains(&character) {
                if !printed.is_empty() && !printed.ends_with(Self::SEPARATORS) {
                    printed.push(character);
                }
                continue;
            }
            // Then: set it in capitals, and if the face can draw every
            // character of that, print it as it is spelled. Everything below
            // this line is the fold, which only the characters the face has no
            // glyph for ever reach.
            let capital: String = character.to_uppercase().collect();
            if capital.chars().all(Stencil::can_print) {
                printed.push_str(&capital);
                continue;
            }
            // Matched on the character as it was WRITTEN rather than on its
            // capital, which is why the table below lists both cases.
            let folded: &str = match character {
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
        images.add(Self::face_sheet(layout, look, None))
    }

    /// The same head with a real picture of it laid over the front — his
    /// photograph, or the portrait his profile page draws when there is none.
    ///
    /// Painted first and covered second, rather than replaced: the picture is
    /// a flat frontal of a face and a head is a head all the way round, so
    /// everything the camera never saw — the sides, the back, the underside of
    /// the jaw — is still the generated face, and the two meet on a soft edge
    /// rather than a cut.
    pub fn photographed_face(layout: &FaceLayout, look: &FaceLook, portrait: &Portrait) -> Image {
        Self::face_sheet(layout, look, Some(portrait))
    }

    fn face_sheet(layout: &FaceLayout, look: &FaceLook, portrait: Option<&Portrait>) -> Image {
        /// Wide enough that an eye — three centimetres of a fifty-six
        /// centimetre circumference — lands about nine texels across, which is
        /// the least an iris and a pupil can be told apart in.
        const WIDTH: u32 = 128;
        const HEIGHT: u32 = 96;
        /// …and what a PICTURE gets, which is more than five times as many
        /// texels.
        ///
        /// The sheet above is sized to the rule the rest of this file works
        /// to: one texel on about one pixel at the range a face is looked at
        /// from. That rule is right for a painted face, whose features are
        /// drawn to be legible at exactly that size — and wrong for a
        /// photograph, which is not a diagram of a face but a picture of one.
        /// Squeezed onto the painted sheet a man's own head shot comes out as
        /// a tinted smudge: the fifty texels across the front of his face are
        /// enough to say "a face" and nowhere near enough to say WHOSE.
        ///
        /// SQUARE rather than the 4:3 the painted one is, because the two
        /// axes are not the same problem. Across, the sheet has always had
        /// more texels than a head shot has pixels to fill them. Down, at 192,
        /// it had 580 to the metre against the photograph's 700 — so the last
        /// sixth of the detail in every face was being thrown away at the one
        /// step that had it to spare. 256 puts the sheet ahead of the picture
        /// on both axes, which is where the limit belongs.
        ///
        /// The cost of breaking the rule is minification crawl, and the answer
        /// to that is the mip chain below rather than a smaller sheet.
        const PICTURE: (u32, u32) = (256, 256);

        let (width, height) = if portrait.is_some() {
            PICTURE
        } else {
            (WIDTH, HEIGHT)
        };
        let pixels = Self::face_pixels(layout, look, width, height, portrait);
        if portrait.is_some() {
            Self::mipped(width, height, pixels)
        } else {
            Self::image(width, height, pixels)
        }
    }

    /// The face's texels, apart from the image they end up in — so a whole
    /// squad's worth of them can be dumped and LOOKED at without a browser,
    /// which is the only way to review a generated face at all. See
    /// `textures::tests::dump_faces`.
    fn face_pixels(
        layout: &FaceLayout,
        look: &FaceLook,
        width: u32,
        height: u32,
        portrait: Option<&Portrait>,
    ) -> Vec<u8> {
        let painter = Painter::new(layout, look, portrait);
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
        // Repeat on BOTH axes: a panel is many mesh squares across and many
        // up, and the panel's own UVs say how many.
        //
        // Plain trilinear, deliberately. The chain converging to a flat haze
        // of the net's own density is what this function is FOR — see the note
        // above — and sharpening the far netting back up would undo it.
        Self::tiled(Self::mipped(size, size, base), 1)
    }

    /// The same image, repeated past its edges rather than clamped at them,
    /// and sampled along the axis it is squashed on.
    ///
    /// `anisotropy` is how many samples the hardware may take across a
    /// foreshortened footprint. One is plain trilinear filtering, which picks a
    /// mip level from the WIDER of the two directions a texel is stretched
    /// over — so a surface seen edge-on is filtered as though it were blurred
    /// in both directions and not just the one, and its detail is gone long
    /// before the geometry is. That is the difference between a pitch with
    /// grass on it and a green rectangle, and it is what the mip note in
    /// [`Self::turf`] was describing when it said everything past twenty
    /// metres resolves to the mean.
    ///
    /// Free to ask for: `EXT_texture_filter_anisotropic` is on essentially
    /// every WebGL2 device, and wgpu quietly clamps this back to one where it
    /// is not — so the worst case is exactly the picture we had before, with
    /// no error and nothing to handle. See `wgpu_core`'s `create_sampler`,
    /// which sets it unconditionally to 1 without the downlevel flag.
    fn tiled(mut image: Image, anisotropy: u16) -> Image {
        let mut sampler = ImageSamplerDescriptor::linear();
        sampler.address_mode_u = ImageAddressMode::Repeat;
        sampler.address_mode_v = ImageAddressMode::Repeat;
        sampler.anisotropy_clamp = anisotropy;
        image.sampler = ImageSampler::Descriptor(sampler);
        image
    }

    /// The playing surface, close enough to see the blades.
    ///
    /// Drawn rather than photographed, for the reason everything in this file
    /// is drawn: a photograph of turf is a photograph of ONE patch of turf, lit
    /// from wherever the photographer stood. Tiled over a hundred metres it
    /// prints that patch's own vignette onto the ground as a lattice, and it
    /// cannot be shipped at all without putting a bitmap inside a wasm binary
    /// built at `opt-level = "z"` precisely so as not to carry one.
    ///
    /// Four things it has to be, every one of them something the flat green it
    /// replaces met for free:
    ///
    /// **Seamless**, because it repeats fifty times down the pitch. Blades are
    /// rasterised through wrapping indices, and the slow variation underneath
    /// them is built from sinusoids at whole-numbered frequencies over the
    /// tile, so there is no edge anywhere to line up.
    ///
    /// **Free of anything larger than a leaf.** This is the one that is easy to
    /// get wrong, and the reason there is nothing here that varies slowly. A
    /// first cut had a gentle sinusoidal drift under the blades, on the
    /// reasoning that real turf is not laid evenly and a sward with no
    /// variation above blade scale looks printed. At five per cent it was
    /// invisible in one tile and unmistakable in sixteen: a two-metre grid of
    /// blotches, because the eye finds a grid at any contrast once it repeats.
    /// The unevenness a real pitch has is at the scale of a penalty area, which
    /// is not something a tile can hold. So the only structure in here is the
    /// size of a blade, and what keeps it from looking mechanical is the spread
    /// between one leaf and the next.
    ///
    /// The unevenness itself did not go away — it went where it can be
    /// authored across a whole ground instead of inside a two-metre square.
    /// See [`crate::pitch::Sward`], which carries it, the mow and the wear of a
    /// played match in the vertices of the pitch.
    ///
    /// **Mip-chained, and sampled anisotropically.** Turf is seen at every
    /// angle down to flat-on at the far touchline, where one pixel covers
    /// hundreds of texels along the line of sight and a handful across it.
    /// Undersampled blade detail there does not read as grass, it crawls — the
    /// same failure [`Self::mipped_netting`] was written for — so the chain is
    /// needed either way.
    ///
    /// It is not sufficient on its own, though, and this note used to say it
    /// was: trilinear filtering picks its level off the WIDER of the two
    /// directions, so a surface seen edge-on was blurred across as hard as it
    /// was blurred along, and everything past about twenty metres resolved to
    /// the mean — which is to say, back to the flat green the sheet replaced.
    /// That was recorded here as intended behaviour. It was a filtering
    /// setting. See [`Self::tiled`], which asks for sixteen samples along the
    /// squashed axis and costs nothing measurable: this scene is bound per
    /// entity, and renders in the same time at 720p and at 4K.
    ///
    /// **Exactly the colour it was asked for.** The blades are a MULTIPLIER on
    /// `shade`, normalised at the end so the tile averages to one. Which is
    /// what lets density, length and scatter be tuned without moving the colour
    /// of the pitch — the only reason the two are safe to work on separately —
    /// and it puts the 1x1 end of the mip chain on `shade` itself.
    ///
    /// Costs about 85 ms of a release build to draw — both sheets, both mip
    /// chains — on the browser's main thread before the first frame. That is
    /// the budget every constant below is set against, and `dump_turf` prints
    /// it. It was 55 ms for the colour alone; [`Self::relief`] would have put
    /// it past 120 had the box filter in [`Self::mipped_as`] not been tightened
    /// at the same time.
    pub fn turf(images: &mut Assets<Image>, shade: Color) -> Turf {
        /// Texels on a side. Against the two-metre tile in `Pitch::TURF_TILE`
        /// this is 512 to the metre, which puts a 4 mm blade at two texels
        /// across — the floor for anything that has to survive minification,
        /// and the reason the tile is not any larger than it is.
        const SIZE: u32 = 1024;
        /// Blades on the tile — about a tenth of the real count, a square metre
        /// of sward carrying tens of thousands of shoots.
        ///
        /// Set by the two failures either side of it. Too few and it is tussock
        /// rather than turf: the gaps win, and a pitch is a MOWN surface where
        /// what you mostly see is leaf. Too many and the mat closes up and the
        /// whole thing goes back to being flat green with noise on it, which is
        /// the thing being fixed.
        const BLADES: u32 = 46_000;
        /// One mown blade in texels: 24 mm long, 4 mm across. A pitch is cut to
        /// around 25 mm and it is cut OFTEN, which is why this is short — long
        /// blades at this density read as a lawn that has got away from
        /// somebody.
        const LENGTH: f32 = 12.0;
        const BREADTH: f32 = 2.0;
        /// How far off the mow line a blade may lie, in radians. Nothing is a
        /// carpet and a half-turn is a meadow. Two thirds of a radian — some
        /// 38° either side — is a sward that has been rolled one way and has
        /// then had a fortnight and twenty-two players to recover from it.
        ///
        /// It is also what carries the mowing stripes at range. The two stripe
        /// materials differ only in tint (see `Pitch::spawn_turf`); the GRAIN
        /// that makes a real stripe is in here, and tightening this too far
        /// leaves the pitch combed like corduroy while loosening it past about
        /// a radian throws the mow line away altogether.
        const SCATTER: f32 = 0.62;

        let count = (SIZE * SIZE) as usize;
        // How bright this texel is against the mown shade, and how far the leaf
        // covering it leans from green toward the yellow of a dead one. Two
        // buffers rather than three channels, because the blades write both
        // together and neither is a colour until the very last pass.
        let mut lit = vec![0.0f32; count];
        let mut dry = vec![0.0f32; count];

        // The floor: thatch and soil, seen down between the leaves. Dark, and
        // deliberately NOT green — what lies under a sward is dead stem, and a
        // green floor is most of what makes drawn grass read as a carpet with
        // scratches on it.
        //
        // Not as dark as it wants to be, either. At a third of the mown shade
        // the gaps went to near-black and the tile came out as tussock lit from
        // below; the normalisation then has to drive the leaves up to compensate
        // and the contrast runs away. What is down there is shaded grass, not a
        // hole.
        for index in 0..count {
            lit[index] = 0.44 + 0.08 * Self::hash01(index as u32);
            dry[index] = 0.78;
        }

        for blade in 0..BLADES {
            let seed = blade * 8;
            let root = Vec2::new(
                Self::hash01(seed) * SIZE as f32,
                Self::hash01(seed + 1) * SIZE as f32,
            );
            // Along the mow line, give or take — and half of them lying the
            // other way along it, because a roller leaves a BIAS and not a
            // parting. Blades that all point one way comb into corduroy at
            // arm's length.
            let off_line = (Self::hash01(seed + 2) - 0.5) * 2.0 * SCATTER;
            let backward = f32::from(Self::hash01(seed + 3) < 0.5) * PI;
            let (sin, cos) = (off_line + backward).sin_cos();
            // `v` runs across the pitch, which is the way the mower went — see
            // the stripes in `Pitch::spawn_turf`, which are bands ACROSS it.
            let along = Vec2::new(sin, cos);
            let across = Vec2::new(cos, -sin);

            let length = LENGTH * (0.65 + 0.7 * Self::hash01(seed + 4));
            let breadth = BREADTH * (0.75 + 0.5 * Self::hash01(seed + 5));
            // Leaf to leaf: one catches the light, the next is in the shadow of
            // the one above it, a third is last week's growth going over. This
            // spread is most of what separates turf from a green surface with
            // scratches on it.
            let leaf = 0.76 + 0.50 * Self::hash01(seed + 6);
            let sear = Self::hash01(seed + 7);

            let steps = length.ceil().max(1.0) as u32;
            for step in 0..=steps {
                let t = step as f32 / steps as f32;
                let spine = root + along * (length * t);
                // A blade tapers, and its tip is the part the light finds.
                let half = breadth * 0.5 * (1.0 - 0.45 * t);
                let bright = leaf * (0.86 + 0.28 * t);
                let reach = half.ceil() as i32 + 1;
                for offset in -reach..=reach {
                    // Soft at the edge. A blade two texels wide with hard sides
                    // is a dotted line the moment the mip chain gets hold of it
                    // — [`Self::netting`] learned that one the hard way.
                    let cover = Self::smooth(half + 0.5 - (offset as f32).abs());
                    if cover <= 0.0 {
                        continue;
                    }
                    let point = spine + across * offset as f32;
                    let x = (point.x.round() as i32).rem_euclid(SIZE as i32) as u32;
                    let y = (point.y.round() as i32).rem_euclid(SIZE as i32) as u32;
                    let index = (y * SIZE + x) as usize;
                    // Whichever leaf is on top wins, rather than the two being
                    // mixed. Looking down into a sward you see the nearest
                    // blade and the shadow beside it; averaging the blades that
                    // overlap is what fills that shadow in and takes the depth
                    // straight back out.
                    let value = lit[index] + (bright - lit[index]) * cover;
                    if value > lit[index] {
                        lit[index] = value;
                        dry[index] += (sear - dry[index]) * cover;
                    }
                }
            }
        }

        // Green toward the yellow of a dead leaf, as a multiplier per channel.
        // Red up and blue DOWN with green almost still, because that is what a
        // leaf does as it dries: it does not get brighter, it loses its blue.
        // Centred, so the axis runs both ways off `shade` — new growth is the
        // bluer green and there is always some of it.
        let cast = |sear: f32| -> Vec3 {
            let axis = sear - 0.5;
            Vec3::new(1.0 + 0.30 * axis, 1.0 + 0.05 * axis, 1.0 - 0.34 * axis)
        };

        // Normalise, per channel. Whatever the blades did to the average, the
        // tile has to come off the end of the mip chain as `shade` — see the
        // note above, which is the whole reason this is a multiplier rather
        // than a colour.
        let mut mean = Vec3::ZERO;
        for index in 0..count {
            mean += cast(dry[index]) * lit[index];
        }
        mean /= count as f32;
        let gain = Vec3::new(1.0 / mean.x, 1.0 / mean.y, 1.0 / mean.z);

        let base = Self::tone(shade);
        let mut data = Vec::with_capacity(count * 4);
        for index in 0..count {
            let colour = base * cast(dry[index]) * lit[index] * gain;
            data.extend_from_slice(&[
                (colour.x.clamp(0.0, 1.0) * 255.0) as u8,
                (colour.y.clamp(0.0, 1.0) * 255.0) as u8,
                (colour.z.clamp(0.0, 1.0) * 255.0) as u8,
                255,
            ]);
        }

        /// How far down the pitch the blades are asked to survive.
        ///
        /// Sixteen is the ceiling every implementation that offers this at all
        /// offers, and the pitch is the one surface in the scene that earns
        /// it: it runs a hundred metres away from the lens on a long lens, so
        /// the far touchline is compressed by well over an order of magnitude
        /// and nothing less would reach it.
        const ALONG_THE_PITCH: u16 = 16;

        Turf {
            albedo: images.add(Self::tiled(
                Self::mipped(SIZE, SIZE, data),
                ALONG_THE_PITCH,
            )),
            relief: images.add(Self::tiled(
                Self::mipped_linear(SIZE, SIZE, Self::relief(SIZE, &lit)),
                ALONG_THE_PITCH,
            )),
        }
    }

    /// The same sward as a normal map, taken off the brightness the blades
    /// were rasterised into.
    ///
    /// Without this the pitch is a Lambertian plane with a picture of grass
    /// printed on it — and a plane lit from one direction returns exactly one
    /// value, which is what "flat" means. A sward is not flat: every leaf
    /// faces somewhere of its own, catches the light on one side and shades
    /// its neighbour on the other, and that is most of what the eye reads as
    /// depth in grass at any range where it can still see leaves.
    ///
    /// `lit` is used as a height field, which is not an approximation but the
    /// thing itself: the rasteriser resolves overlapping blades by keeping the
    /// brightest, so whichever leaf is ON TOP is the one that wrote each texel
    /// — see the note there. A depth buffer arrived at from the other end.
    ///
    /// Gradients wrap, exactly as the blades did, so the relief is seamless
    /// wherever the albedo is.
    fn relief(size: u32, lit: &[f32]) -> Vec<u8> {
        /// How steep to make it. Not a physical figure — `lit` is a
        /// brightness, and the conversion from "this leaf is a fifth brighter
        /// than its neighbour" to "this leaf stands a millimetre proud of it"
        /// has no honest constant in it. Judged on a render from a metre off
        /// the deck, which is the only place the full-resolution mip is ever
        /// seen: below about one it does not read as grass at all, and past
        /// about four the sward turns to gravel.
        const RELIEF: f32 = 2.4;

        let at = |x: u32, y: u32| lit[(y * size + x) as usize];
        let wrap = |value: i64| value.rem_euclid(size as i64) as u32;

        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let across = at(wrap(x as i64 + 1), y) - at(wrap(x as i64 - 1), y);
                let down = at(x, wrap(y as i64 + 1)) - at(x, wrap(y as i64 - 1));
                // A height field's normal is (-dh/du, -dh/dv, 1) normalised.
                // Tangent space here is the one [`crate::pitch::Sward`] writes
                // out: U on +X, V on +Z, and the green channel positive along
                // V, which is the convention Bevy's own normal maps use.
                let normal =
                    Vec3::new(-across * RELIEF, -down * RELIEF, 1.0).normalize_or(Vec3::Z);
                data.extend_from_slice(&[
                    ((normal.x * 0.5 + 0.5) * 255.0) as u8,
                    ((normal.y * 0.5 + 0.5) * 255.0) as u8,
                    ((normal.z * 0.5 + 0.5) * 255.0) as u8,
                    255,
                ]);
            }
        }
        data
    }

    /// Any image, with a box-filtered mip chain down to a single texel.
    ///
    /// Two textures in the scene need one and they need it for the same
    /// reason: more texels than the screen has pixels to put them on. The net
    /// gets there by repeating sixty times across a panel; a photographed face
    /// gets there by being a PHOTOGRAPH — see [`Self::face_sheet`], which
    /// hands a head four times the sheet it draws for itself, because that is
    /// what it takes for a picture of a man to still look like him. Neither
    /// can be sampled at one texel per pixel, and undersampling is answered by
    /// pre-filtering or not at all.
    fn mipped(width: u32, height: u32, base: Vec<u8>) -> Image {
        Self::mipped_as(width, height, base, TextureFormat::Rgba8UnormSrgb)
    }

    /// The same for a sheet that is data rather than a picture.
    ///
    /// A normal map holds three signed numbers packed into a byte each, and
    /// putting them through the sRGB transfer curve would bend every one of
    /// them — a normal map read as a colour points the wrong way everywhere
    /// except straight up.
    ///
    /// Box-filtering normals and NOT renormalising them is deliberate too: a
    /// pair of leaves facing opposite ways average to something short, which
    /// the shader normalises back to straight up. That is exactly right —
    /// grass seen from far enough away that two blades share a pixel has no
    /// relief left to show, and the chain arriving at flat is the same
    /// resolution to the mean the albedo makes.
    fn mipped_linear(width: u32, height: u32, base: Vec<u8>) -> Image {
        Self::mipped_as(width, height, base, TextureFormat::Rgba8Unorm)
    }

    fn mipped_as(width: u32, height: u32, base: Vec<u8>, format: TextureFormat) -> Image {
        let mut levels: Vec<(u32, u32, Vec<u8>)> = vec![(width, height, base)];
        while levels
            .last()
            .is_some_and(|(across, down, _)| *across > 1 || *down > 1)
        {
            let (across, down, source) = levels.last().expect("seeded above");
            let (across, down) = (*across, *down);
            let (half_across, half_down) = ((across / 2).max(1), (down / 2).max(1));
            let mut next = Vec::with_capacity((half_across * half_down * 4) as usize);
            // Row and column offsets hoisted out of the sample loop. The
            // obvious way to write this recomputes `(y * width + x) * 4 +
            // channel` for all four taps of all four channels of every texel,
            // which is sixteen multiplications where two would do — and this
            // is the hot end of the whole load: a 1024-square sheet and its
            // chain is some twenty million taps, and the pitch now builds two
            // of them. Same arithmetic, same clamping at the far edge, an
            // eighth of the address maths.
            for y in 0..half_down {
                let top = ((y * 2).min(down - 1) * across) as usize * 4;
                let bottom = ((y * 2 + 1).min(down - 1) * across) as usize * 4;
                for x in 0..half_across {
                    let left = (x * 2).min(across - 1) as usize * 4;
                    let right = (x * 2 + 1).min(across - 1) as usize * 4;
                    for channel in 0..4 {
                        let sum = source[top + left + channel] as u32
                            + source[top + right + channel] as u32
                            + source[bottom + left + channel] as u32
                            + source[bottom + right + channel] as u32;
                        next.push((sum / 4) as u8);
                    }
                }
            }
            levels.push((half_across, half_down, next));
        }

        let mip_level_count = levels.len() as u32;
        let mut data = Vec::new();
        for (_, _, level) in &levels {
            data.extend_from_slice(level);
        }

        // `Image::new` asserts that the buffer is exactly one mip level, so the
        // chain goes on through `new_uninit`.
        let mut image = Image::new_uninit(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            format,
            RenderAssetUsages::RENDER_WORLD,
        );
        // `TextureDataOrder` defaults to layer-major, which for one layer is
        // simply mip 0 then mip 1 then mip 2 — the order built above.
        image.texture_descriptor.mip_level_count = mip_level_count;
        image.data = Some(data);
        image.sampler = ImageSampler::linear();
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

    /// The gradient the sky dome is skinned with, read top to bottom: zenith at
    /// v = 0, horizon at v = 0.5, ground at v = 1.
    ///
    /// One column would do — nothing varies across it — but a handful keeps the
    /// texture from being a degenerate strip on the way to the GPU.
    ///
    /// The horizon stop is exactly the camera's haze colour, and that is the
    /// whole trick: distant geometry already tends toward the haze, so a sky
    /// that meets the ground in the same value gives the far stands nothing to
    /// stand out against. Above it the value falls away to a deep blue rather
    /// than the flat black this scene used to end in; below it the gradient
    /// keeps going down a little, so the far edge of the surround — which is
    /// well inside the haze by then — does not sit on a seam.
    pub fn sky(images: &mut Assets<Image>) -> Handle<Image> {
        const WIDTH: u32 = 4;
        const HEIGHT: u32 = 512;
        /// `(v, colour)`, in order. Between two stops the mix is smoothstepped
        /// rather than linear: a straight ramp puts a visible crease at every
        /// stop, and on something that fills a third of the frame a crease
        /// reads as a band of cloud that is not there.
        /// The stops are not spread evenly over the dome because the shot is
        /// not: a broadcast lens spends its life within about 25° of the
        /// horizon, which is `v` 0.36 to 0.50 — an eighth of the sphere doing
        /// nearly all the work. Spaced evenly, the visible band came out as one
        /// flat grey and the gradient was only really there overhead, where
        /// nothing ever looks.
        ///
        /// The blues are stronger than they look here. Everything on screen has
        /// been through the tonemapper by the time it is seen, and it pulls a
        /// dark saturated colour a long way toward grey — authored at the value
        /// it should end up, the sky came out as slate.
        const STOPS: [(f32, Vec3); 8] = [
            (0.000, Vec3::new(0.020, 0.055, 0.190)),
            (0.250, Vec3::new(0.035, 0.080, 0.230)),
            (0.380, Vec3::new(0.060, 0.115, 0.265)),
            (0.455, Vec3::new(0.110, 0.165, 0.295)),
            (0.500, Vec3::new(0.200, 0.230, 0.280)),
            (0.560, Vec3::new(0.150, 0.172, 0.205)),
            (0.720, Vec3::new(0.098, 0.112, 0.132)),
            (1.000, Vec3::new(0.072, 0.082, 0.096)),
        ];

        let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for row in 0..HEIGHT {
            let down = row as f32 / (HEIGHT - 1) as f32;
            let colour = STOPS
                .windows(2)
                .find(|pair| down <= pair[1].0)
                .map(|pair| {
                    let (from, below) = pair[0];
                    let (to, above) = pair[1];
                    let span = (to - from).max(f32::EPSILON);
                    below.lerp(above, Self::smooth((down - from) / span))
                })
                .unwrap_or(STOPS[STOPS.len() - 1].1);
            let texel = [
                (colour.x * 255.0) as u8,
                (colour.y * 255.0) as u8,
                (colour.z * 255.0) as u8,
                255,
            ];
            for _ in 0..WIDTH {
                data.extend_from_slice(&texel);
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
    /// The real picture of this man's head, once one has arrived.
    portrait: Option<&'a Portrait>,
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

    fn new<'a>(
        layout: &'a FaceLayout,
        look: &'a FaceLook,
        portrait: Option<&'a Portrait>,
    ) -> Painter<'a> {
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
            portrait,
        }
    }

    fn texel(&self, height: f32, across: f32, angle: f32) -> Vec3 {
        let mut colour = self.modelled(height, angle);
        if angle.abs() < Self::FRONT {
            colour = self.brows(self.eyes(colour, height, across), height, across);
            colour = self.nose(colour, height, across);
            colour = self.mouth(colour, height, across);
        }
        colour = self.whiskers(colour, height, across, angle);
        // Between the whiskers and the scalp on purpose. A real beard on the
        // picture replaces the painted one under it; the cap of hair, which
        // is MESH, still has to lay its shadow over whatever ends up on the
        // forehead, or it reads as a wig resting on a photograph.
        colour = self.photographed(colour, height, angle);
        self.scalp(colour, height, angle)
    }

    /// Where a picture starts giving way to the painted head, and where it is
    /// gone. Past the second of these the surface has turned so far that a
    /// flat frontal laid on it is being seen edge-on and every pixel is
    /// smeared along the side of the skull — the same argument that stops the
    /// painted features at [`Self::FRONT`], arrived at from the other side.
    ///
    /// Both were a quarter of a radian tighter, and what that left was a hard
    /// vertical seam down the temple of every photographed player with the
    /// man's own cheek on one side of it and flat paint on the other. The
    /// picture goes nearly to the silhouette now, which is also where his own
    /// EARS are: they are not on this model any more, so the only ears a head
    /// has are the ones in the photograph, and cutting the picture off before
    /// them would leave him without any. It runs out on its own account at
    /// the silhouette — the studio was keyed off the back of it, so the
    /// outline of his head is exactly where its alpha stops — which is a
    /// softer edge than any angle this could name.
    ///
    /// It does not go PAST the silhouette, which was tried: the projection is
    /// `sin(angle)`, so beyond a quarter turn it folds back and starts laying
    /// the same texels down a second time, and one row of them drags
    /// backwards across the side of the head as a streak.
    const PICTURE_FRONT: f32 = 1.26;
    const PICTURE_EDGE: f32 = 1.56;

    /// The picture of this man's head, laid over the face just painted.
    ///
    /// Everything here is about the EDGES of it, because the middle takes care
    /// of itself: what decides whether this reads as a footballer's face or as
    /// a photograph stuck to a ball is how it stops. So it stops four ways —
    /// round the sides as the head turns away, under the jaw, over the crown,
    /// and wherever the picture itself is transparent, which after the studio
    /// background has been keyed out is exactly the outline of his head.
    fn photographed(&self, painted: Vec3, height: f32, angle: f32) -> Vec3 {
        let Some(picture) = self.portrait else {
            return painted;
        };
        let cover = self.pictured(height, angle);
        if cover <= 0.0 {
            return painted;
        }
        let Some((colour, alpha)) = picture.at(self.layout, height, angle) else {
            return painted;
        };

        // Straight down, exactly as it arrived: the photograph's own colour,
        // the photograph's own light. Nothing is re-toned to meet the palette
        // and no modelled shading is laid over the top — the picture already
        // has a face's worth of light and shadow in it, and both corrections
        // together turned a recognisable man into a tinted approximation of
        // one. What moves to meet the picture is the REST of him: his neck,
        // his arms and the cap of hair on his head are all repainted from
        // what this picture says he looks like (see `crate::portrait`).
        Textures::over(painted, colour.min(Vec3::ONE), cover * alpha)
    }

    /// How much of this texel belongs to the picture rather than to the paint,
    /// before the picture's own transparency is taken into account.
    fn pictured(&self, height: f32, angle: f32) -> f32 {
        let crown = self.layout.foot + self.layout.span;
        // Round the sides.
        let front = 1.0
            - Textures::smooth(
                (angle.abs() - Self::PICTURE_FRONT) / (Self::PICTURE_EDGE - Self::PICTURE_FRONT),
            );
        // Under the jaw, where a head shot has a throat, a collar and the top
        // of a shirt, and this head has the top of a neck that belongs to the
        // body's own skin.
        //
        // The picture is scaled off the man's pupils rather than stretched to
        // fit, so his chin lands a little BELOW this model's — which put a
        // sliver of club shirt across the jaw of every player who had a
        // photograph. So the picture stops at the jaw itself and is gone a
        // centimetre under it.
        let above_jaw = Textures::smooth((height - (self.layout.chin - 0.004)) / 0.016);
        // And over the crown, where the surface has turned to face the sky and
        // a flat frontal laid on it is being seen edge-on — the same argument
        // as `front`, arrived at from above.
        //
        // It used to start three centimetres down and take the top of every
        // head with it. That was written when the skull stood 125 mm above
        // the eye line and no photograph reached the top of it anyway, so
        // there was nothing up there to lose; what it painted instead was a
        // flat dome in the colour of his hair, standing over the man's own
        // hairline like a bald patch he does not have. The skull now ends
        // where a photographed head ends (see `BodyParts::SKULL`), so the
        // picture is carried to within a centimetre of the crown and it is
        // his own hair that goes over the top.
        let below_crown = 1.0 - Textures::smooth((height - (crown - 0.011)) / 0.010);
        front * above_jaw * below_crown
    }

    /// The shading a head carries before anything is drawn on it.
    ///
    /// One directional light with no shadow maps lights a sphere as a sphere,
    /// so every bit of relief a face has — the temples falling away, the jaw
    /// in its own shade, the brow ridge catching the light — has to be in the
    /// paint. Without it the features sit on a flat disc.
    fn modelled(&self, height: f32, angle: f32) -> Vec3 {
        self.skin * self.shading(height, angle)
    }

    /// That relief on its own, as a multiplier — nothing here tints, it only
    /// darkens and lightens, so the same numbers serve bare skin and a
    /// photograph of it equally. Held apart from [`Self::modelled`] for the
    /// second caller: see [`Self::photographed`].
    fn shading(&self, height: f32, angle: f32) -> f32 {
        let turned = Textures::smooth((angle.abs() / 1.9).min(1.0));
        // How much the flank falls away. Deeper than it was, and what set the
        // depth is a PHOTOGRAPH: where a picture gives out at the temple the
        // paint has to carry on from the tone the picture had there, and a
        // sixth off the front tone was nowhere near a studio-lit cheek turning
        // into shadow. The step showed as a seam down the side of every
        // photographed face, and on a painted one it left the head reading as
        // a disc.
        let mut shade = 1.0 - 0.26 * turned;
        // Under the jaw and down the neck, which is in the head's own shadow
        // for the whole match.
        let under = Textures::smooth(((self.layout.chin + 0.012 - height) / 0.055).clamp(0.0, 1.0));
        shade *= 1.0 - 0.30 * under;
        // And the brow ridge, which catches it.
        let ridge =
            Textures::smooth((1.0 - (height - self.layout.brow).abs() / 0.020).clamp(0.0, 1.0));
        shade * (1.0 + 0.055 * ridge * (1.0 - turned))
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
            //
            // Except on a man wearing a PICTURE, where this is not stubble at
            // all: he has no cap of hair on him — his own is in the picture —
            // so above the hairline this wash is the only hair he has, and it
            // is laid on as hair rather than as a shadow of it. The colour was
            // read off the top of his own head; see `Portrait::hair_tone`.
            let (ink, weight) = if self.portrait.is_some() {
                (0.85, 0.95)
            } else {
                (0.55, 0.55)
            };
            let mask = Textures::smooth((height - line) / 0.022);
            return Textures::over(base, self.hair * ink, mask * weight);
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

    /// A shirt is printed in capitals, and in the same face the player's own
    /// label is set in — so a letter that face can draw is printed as it is
    /// spelled, accent and all. Only what it cannot draw folds onto a base
    /// letter or is dropped: losing the acute off an O is what a shirt printer
    /// does when he has no acute; losing the O is a bug.
    #[test]
    fn a_name_is_printed_in_the_face_that_can_spell_it() {
        // Outfit carries the whole of Latin-1 and most of Extended-A, so these
        // reach the shirt the way the squad list spells them. They all folded
        // to bare ASCII when the lettering came off a 5×7 grid.
        assert_eq!(Textures::fold("Müller"), "MÜLLER");
        assert_eq!(Textures::fold("Nuñez"), "NUÑEZ");
        assert_eq!(Textures::fold("Sørensen"), "SØRENSEN");
        assert_eq!(Textures::fold("Šeško"), "ŠEŠKO");
        assert_eq!(Textures::fold("Åkerman"), "ÅKERMAN");
        // Uppercasing is what a shirt does, and it is done properly: the
        // sharp S has no capital of its own and becomes a pair.
        assert_eq!(Textures::fold("Weiß"), "WEISS");
        // And the contract that matters, which does not depend on which
        // characters either subset happens to carry: whatever survives the fold
        // can be drawn. A box on the back of a shirt is the one outcome there
        // is no excuse for, and the fold is the only thing standing between the
        // squad list and one.
        for spelled in [
            "Müller",
            "Nuñez",
            "Sørensen",
            "Šeško",
            "Weiß",
            "Łukasz",
            "Åkerman",
            "Ǧorǧe",
            "Đorđević",
            "Håland",
            "Ćaleta-Car",
            "O'Shea",
            "Ægir",
            "Þórsson",
        ] {
            let printed = Textures::fold(spelled);
            assert!(!printed.is_empty(), "{spelled} printed nothing");
            assert!(
                printed.chars().all(Stencil::can_print),
                "{spelled} printed {printed}, which the face cannot draw"
            );
        }
        // Real surnames carry these.
        assert_eq!(Textures::fold("O'Neill"), "O'NEILL");
        assert_eq!(Textures::fold("Van der Sar"), "VAN DER SAR");
        assert_eq!(Textures::fold("Alves-Silva"), "ALVES-SILVA");
        // Tidied rather than printed as found: no leading, trailing or
        // doubled punctuation, because a stencil cannot show what it is
        // standing in for. The face can draw all four marks, so this has to be
        // decided ahead of "can the face draw it" rather than after.
        assert_eq!(Textures::fold("  de  Jong "), "DE JONG");
        assert_eq!(Textures::fold("-Smith-"), "SMITH");
        // And a name neither face can print comes back empty for the caller to
        // notice, rather than as a row of blanks on a shirt.
        assert_eq!(Textures::fold("日本"), "");
        assert!(Textures::name(&mut Assets::default(), "日本").is_none());
        assert!(Textures::name(&mut Assets::default(), "Kane").is_some());
    }

    /// The print is the same shape the panel is, and it is actually inked.
    ///
    /// The mask is what ends up stretched over the curved sheet of cloth on a
    /// player's back, so what matters is that the letters land inside it and
    /// fill it: a stencil that misses its panel prints a blank shirt, and one
    /// that overflows prints a clipped name.
    #[test]
    fn the_print_fills_its_panel() {
        const WIDTH: u32 = 176;
        const HEIGHT: u32 = 40;

        let mask = Stencil::mask("RONALDO", WIDTH, HEIGHT, 0.94, 0.88);
        assert_eq!(mask.len(), (WIDTH * HEIGHT) as usize);

        let inked = |x: u32, y: u32| mask[(y * WIDTH + x) as usize] > 0;
        let columns: Vec<u32> = (0..WIDTH)
            .filter(|x| (0..HEIGHT).any(|y| inked(*x, y)))
            .collect();
        let rows: Vec<u32> = (0..HEIGHT)
            .filter(|y| (0..WIDTH).any(|x| inked(x, *y)))
            .collect();
        let (first, last) = (columns[0], columns[columns.len() - 1]);
        let (top, bottom) = (rows[0], rows[rows.len() - 1]);

        // Inside the panel on every side...
        assert!(
            first > 0 && last < WIDTH - 1,
            "{first}..{last} across {WIDTH}"
        );
        assert!(
            top > 0 && bottom < HEIGHT - 1,
            "{top}..{bottom} down {HEIGHT}"
        );
        // ...using most of the width it is allowed...
        assert!(
            last - first > WIDTH * 3 / 4,
            "only {} of {WIDTH} used",
            last - first
        );
        // ...and centred, which is what a shirt is.
        let slack = WIDTH - 1 - last;
        assert!(
            first.abs_diff(slack) <= 2,
            "left {first} against right {slack}"
        );

        // A number is set to the full height of its own panel, where a name is
        // set smaller with air above and below — that is the whole difference
        // between the two calls, and it has to survive.
        let digits = Stencil::mask("9", 64, 52, 0.94, 1.0);
        let tall = (0..52)
            .filter(|y| (0..64).any(|x| digits[(y * 64 + x) as usize] > 0))
            .count();
        assert!(tall > 44, "a number filled only {tall} of 52 rows");
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
        let pixels =
            Textures::face_pixels(&layout, &look(Beard::Clean, false), WIDTH, HEIGHT, None);
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

    /// A picture the size of the patch the browser hands over, painted in
    /// bands so that where each band ends up on the skull can be read off the
    /// sheet: green across the eye line, blue across the chin, and a red
    /// stripe down one cheek at a known angle round the head.
    ///
    /// Deliberately not a face. What is being tested is the crossing between
    /// a flat picture and a round head, and a test that needs a face to see it
    /// is a test that can only be read by eye.
    fn banded_picture() -> Portrait {
        const SIZE: u32 = 96;
        // A picture measured as: eyes across the middle, pupils a quarter of
        // the frame apart. Everything else is placed off those.
        let (centre, eyes, pupils) = (0.5, 0.42, SIZE as f32 * 0.25);
        // A metre of head is this many pixels of picture, which is what turns
        // the bands below into distances on a skull.
        let scale = pupils / 0.064;
        // Green on the eye line, blue 90 mm under it — just above where the
        // model's own jaw cuts the picture off — and a red stripe 40 mm out.
        let chin = eyes + 0.090 * scale / SIZE as f32;
        let stripe = centre + 0.040 * scale / SIZE as f32;

        let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for row in 0..SIZE {
            for column in 0..SIZE {
                let v = (row as f32 + 0.5) / SIZE as f32;
                let u = (column as f32 + 0.5) / SIZE as f32;
                let band = if (v - eyes).abs() < 0.012 {
                    [40, 220, 40]
                } else if (v - chin).abs() < 0.012 {
                    [40, 40, 220]
                } else if (u - stripe).abs() < 0.008 {
                    [220, 40, 40]
                } else {
                    [190, 150, 130]
                };
                pixels.extend_from_slice(&[band[0], band[1], band[2], 255]);
            }
        }
        Portrait {
            width: SIZE,
            height: SIZE,
            pixels,
            centre,
            eyes,
            pupils,
        }
    }

    /// A picture goes onto the head where the head's own landmarks are.
    ///
    /// The one thing that decides whether a photograph reads as this player's
    /// face or as a poster of somebody stuck to a ball: the eye line has to
    /// land on the eye line and the chin on the chin, because the skull under
    /// it carries a nose as GEOMETRY and a picture hung a centimetre low is a
    /// face with two of them. Sideways matters as much and is easier to get
    /// wrong — the head is round and the picture is flat, so a point two
    /// thirds of the way across the face belongs at `asin(2/3)` round the
    /// skull and nowhere else.
    #[test]
    fn a_picture_lands_on_the_landmarks_of_the_head_it_goes_onto() {
        const WIDTH: u32 = 128;
        const HEIGHT: u32 = 96;

        let layout = BodyParts::face_layout();
        let picture = banded_picture();
        let pixels = Textures::face_pixels(
            &layout,
            &look(Beard::Clean, false),
            WIDTH,
            HEIGHT,
            Some(&picture),
        );
        let at = |column: u32, row: u32| {
            let index = ((row * WIDTH + column) * 4) as usize;
            Vec3::new(
                pixels[index] as f32,
                pixels[index + 1] as f32,
                pixels[index + 2] as f32,
            ) / 255.0
        };
        let row_of = |height: f32| {
            ((((height - layout.foot) / layout.span) * HEIGHT as f32) as u32).min(HEIGHT - 1)
        };
        // The front of the head is a quarter of the way round the lathe.
        let column_of = |angle: f32| (((angle / TAU + 0.25) * WIDTH as f32) as u32).min(WIDTH - 1);
        let front = column_of(0.0);

        // Green on the eye line and blue on the chin, both dead ahead. Read as
        // "greener than the skin it replaced" rather than against a number:
        // the picture is shaded by the head's own relief and pulled toward the
        // complexion of the neck it sits on, and neither of those is a colour
        // this test should have to know.
        let eye_line = at(front, row_of(layout.eyes));
        assert!(
            eye_line.y > eye_line.x && eye_line.y > eye_line.z,
            "the eye line of the picture is not on the eye line of the head: {eye_line:?}"
        );
        // The blue band is 90 mm under the picture's eye line, and that is a
        // DISTANCE rather than a fraction of a frame. So it has to land 90 mm
        // under the head's eye line: the picture keeps its own proportions
        // and the head is whatever shape it is.
        let low = at(front, row_of(layout.eyes - 0.090));
        assert!(
            low.z > low.x && low.z > low.y,
            "the lower band of the picture is not 90 mm under its eyes: {low:?}"
        );
        // …and nothing green between them, which is what a picture hung at the
        // wrong scale would leave: the eye line has to be at ONE height.
        let midway = at(front, row_of(layout.eyes - 0.050));
        assert!(
            midway.x > midway.y,
            "the eye line is smeared down the face: {midway:?}"
        );

        // Sideways, and by the same ruler: the stripe is 40 mm out from the
        // mid-line, so it belongs where the skull is 40 mm across —
        // `asin(40/90)`, 26° — and NOT at 40/90 of the way round to 90°, which
        // is where mapping the angle straight across would put it.
        let redness = |column: u32| {
            let texel = at(column, row_of(layout.eyes - 0.020));
            texel.x - (texel.y + texel.z) * 0.5
        };
        let projected = column_of((0.040f32 / layout.cheek).asin());
        let flat = column_of(std::f32::consts::FRAC_PI_2 * 0.040 / layout.cheek);
        assert!(
            redness(projected) > 0.10,
            "the stripe is not where the projection puts it: {}",
            redness(projected)
        );
        assert!(
            redness(projected) > redness(flat) + 0.05,
            "the picture is wrapped round the head rather than projected onto it"
        );

        // And round the back there is no picture at all — a flat frontal seen
        // edge-on is a smear, so it has to have given way to paint by then.
        let behind = at(column_of(2.6), row_of(layout.eyes));
        assert!(
            behind.x > behind.y && behind.x > behind.z,
            "the picture has been carried round the back of the head: {behind:?}"
        );
    }

    /// There is an eye where the layout says there is an eye — and only there.
    #[test]
    fn the_eyes_land_on_the_eye_line() {
        let layout = BodyParts::face_layout();
        let look = look(Beard::Clean, false);
        let painter = Painter::new(&layout, &look, None);
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
        let painter = Painter::new(&layout, &look, None);
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

        let plain = Painter::new(&layout, &clean, None);
        let whiskers = Painter::new(&layout, &bearded, None);
        let bald = Painter::new(&layout, &shaved, None);
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
                    photo: None,
                    face: None,
                })
            })
            .collect();

        let across = WIDTH as usize * looks.len();
        let mut sheet = vec![0u8; across * HEIGHT as usize * 4];
        for (column, look) in looks.iter().enumerate() {
            let face = Textures::face_pixels(&layout, look, WIDTH, HEIGHT, None);
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

    /// One tile of turf, and the same tile four across so the seam has
    /// somewhere to show up if there is one.
    ///
    /// Blade density, length, scatter and the spread between one leaf and the
    /// next are all judged by eye and there is no other way to judge them:
    /// [`Textures::turf`] guarantees the AVERAGE (see its note on
    /// normalisation), so every number in it moves the texture without moving
    /// the colour, and the only question left is whether it reads as grass.
    ///
    /// ```text
    /// MATCH_TURF_DUMP=<dir> cargo test --lib dump_turf -- --ignored --nocapture
    /// ```
    ///
    /// Writes `turf.rgba` and `turf-tiled.rgba` with their dimensions on
    /// stdout, for whatever turns raw pixels into a picture. Also prints the
    /// mean the tile actually lands on against the shade it was handed, which
    /// is the one thing here that is not a matter of taste.
    #[test]
    #[ignore = "writes files; run by hand when the turf generator changes"]
    fn dump_turf() {
        let Ok(directory) = std::env::var("MATCH_TURF_DUMP") else {
            panic!("set MATCH_TURF_DUMP to a directory");
        };
        let directory = std::path::Path::new(&directory);

        // The pitch's own shade rather than a stand-in. Restating it here is
        // exactly the drift the generator is built to avoid — the first cut of
        // this test did restate it, the pitch was darkened, and the assertion
        // below went on passing against a colour the pitch no longer used.
        let shade = crate::pitch::Pitch::MOWN;
        let mut images = Assets::<Image>::default();
        let started = std::time::Instant::now();
        let sheets = Textures::turf(&mut images, shade);
        // Load-time cost, and the reason `BLADES` is not simply turned up until
        // it stops looking better: this runs once on a browser's main thread
        // before the first frame, so it is measured rather than assumed.
        println!("built in {:?}", started.elapsed());
        let image = images.get(&sheets.albedo).expect("the tile was just added");
        let size = image.texture_descriptor.size.width as usize;
        // Mip 0 only: the chain is appended after it and is not a picture.
        let tile = &image.data.as_ref().expect("pixels")[..size * size * 4];

        std::fs::write(directory.join("turf.rgba"), tile).expect("wrote the tile");
        println!("{size}x{size} at {}", directory.join("turf.rgba").display());

        // And the relief beside it, so the two can be looked at together — a
        // normal map is readable by eye as a lilac sheet with the blades
        // embossed in it, and a wrong sign shows up as the whole sward lit
        // from underneath.
        let relief = images.get(&sheets.relief).expect("built alongside");
        std::fs::write(
            directory.join("turf-relief.rgba"),
            &relief.data.as_ref().expect("pixels")[..size * size * 4],
        )
        .expect("wrote the relief");
        println!(
            "{size}x{size} at {}",
            directory.join("turf-relief.rgba").display()
        );

        // Four across and four down. A tile that does not wrap prints a cross
        // through the middle of this and nothing else will show it.
        const REPEAT: usize = 4;
        let across = size * REPEAT;
        let mut sheet = vec![0u8; across * across * 4];
        for row in 0..across {
            for column in 0..across {
                let from = ((row % size) * size + (column % size)) * 4;
                let to = (row * across + column) * 4;
                sheet[to..to + 4].copy_from_slice(&tile[from..from + 4]);
            }
        }
        std::fs::write(directory.join("turf-tiled.rgba"), &sheet).expect("wrote the sheet");
        println!(
            "{across}x{across} at {}",
            directory.join("turf-tiled.rgba").display()
        );

        let mut mean = Vec3::ZERO;
        for texel in tile.chunks_exact(4) {
            mean += Vec3::new(texel[0] as f32, texel[1] as f32, texel[2] as f32) / 255.0;
        }
        mean /= (size * size) as f32;
        let wanted = Textures::tone(shade);
        println!("mean {mean:?}\nwant {wanted:?}");
        // The normalisation is the contract the rest of the tuning rests on:
        // whatever the blades do, the far end of the mip chain is `shade`. A
        // quantisation step is 1/255, so half a per cent is comfortably more
        // slack than rounding needs and far less than a visible drift.
        for channel in 0..3 {
            assert!(
                (mean[channel] - wanted[channel]).abs() < 0.005,
                "channel {channel} averaged {} against a shade of {}",
                mean[channel],
                wanted[channel]
            );
        }
    }

    /// The shirt print, straight out of the stencil and onto the terminal.
    ///
    /// The panels are 40 and 52 texels tall and end up stretched over a curved
    /// sheet of cloth twenty metres from the lens, which is not a place you can
    /// read a letterform. This is: run it when the face, the tracking or the
    /// fitting changes and look at the shapes.
    ///
    ///     cargo test --lib dump_print -- --ignored --nocapture
    #[test]
    #[ignore = "prints to the terminal; run by hand when the lettering changes"]
    fn dump_print() {
        // Half the rows, because a terminal cell is about twice as tall as it
        // is wide and the panel would otherwise come out stretched.
        const RAMP: [char; 5] = [' ', '.', ':', '#', '@'];

        for (text, width, height, margin, cap) in [
            ("9", 64, 52, 0.94, 1.0),
            ("70", 64, 52, 0.94, 1.0),
            ("RONALDO", 176, 40, 0.94, 0.88),
            ("MÜLLER", 176, 40, 0.94, 0.88),
            ("ALVES-SILVA", 176, 40, 0.94, 0.88),
        ] {
            println!("\n{text}  ({width}x{height})");
            let mask = Stencil::mask(text, width, height, margin, cap);
            for y in (0..height).step_by(2) {
                let row: String = (0..width)
                    .map(|x| RAMP[(mask[(y * width + x) as usize] as usize * 4) / 255])
                    .collect();
                println!("|{row}|");
            }
        }
    }
}
