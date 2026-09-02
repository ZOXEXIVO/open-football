use crate::app::bill::{Held, MemoryBill};
use crate::art::typeface::Stencil;
use crate::players::body::Grain;
use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use shared::Palette;
use std::f32::consts::{PI, TAU};

/// Where the features of a face land on the head mesh that carries it.
///
/// Handed over by whoever built the skull (see
/// [`crate::players::body::BodyParts::face_layout`]) rather than guessed here,
/// because an eye painted at a height the mesh does not have there ends up
/// somewhere on a cheekbone and nothing downstream can tell.
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
/// [`crate::players::portrait`], which is the only thing that builds one.
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
    /// picture as it arrives (see [`crate::players::portrait`]). The first cut
    /// of this assumed the framing instead — one set of constants for the
    /// whole photo library — and the library is not framed to one standard. A
    /// head shot cropped closer than the rest came out of it stretched half as
    /// wide again, because a face measured at 70 pixels was being told it was
    /// 50.
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
///
/// A resource as well as a return value: the playing surface and the ground
/// beyond the touchlines are laid on consecutive frames (see
/// [`crate::app::bringup`]) and the second of them has to find the sheets the
/// first generated rather than generate a second pair.
#[derive(Resource)]
pub struct Turf {
    /// What the grass looks like: blades in the shade it was asked for.
    pub albedo: Handle<Image>,
    /// Which way each leaf is facing, as a tangent-space normal map.
    pub relief: Handle<Image>,
}

/// One spectator's head, and the arithmetic that paints it.
///
/// **A whole head unwrapped, not a face stuck on the front of one.** `u` runs
/// right round the skull — the nose at the middle of the tile and the back of
/// the head at both its edges — and `v` from the crown down to the collar, so
/// the hair, the ears, the jaw and the neck are painted in the same pass as
/// the eyes and every one of them lands where the geometry under it actually
/// is. Before this the tile was a portrait pasted on the one face of a boxed
/// head that looked at the pitch, with the sides and the crown taking a flat
/// skin colour. That is fine at a hundred metres and it is a ground full of
/// masks the moment a lens is walked into a stand, which is the whole
/// complaint this answers.
///
/// **The tile is not spent evenly round the head.** [`CrowdPalette::head_uv`]
/// runs `u` as the square root of the angle, which hands the front ninety
/// degrees half the width of the tile: a wrap that paid the back of a skull
/// the same texels as a pair of eyes would spend three quarters of itself on
/// hair.
///
/// **Every measurement below is a FRACTION of the tile rather than a texel
/// count** — the same discipline [`Painter`] keeps for the players' heads,
/// where the features are given in metres. A face written in texels is a face
/// that has to be redrawn by hand the day the tile changes size, and one that
/// silently stops being a face if nobody does.
///
/// It is still read at four pixels by most of the ground, and the DARK MARKS
/// — a cap of hair, two eyes, a mouth — are still what survives down there.
/// The difference is that they are no longer all there is: the shading, the
/// nose and the ears cost nothing at that distance, because the mip chain has
/// already averaged them into the one brown the far end is drawn in (see
/// [`Textures::crowd`]), and they are the entire picture at three metres.
struct Face {
    skin: Vec3,
    hair: Vec3,
    iris: Vec3,
    /// No hair on top. One in five, which is about the rate in a stand that is
    /// mostly men over thirty, and it is the cheapest variety there is: a bald
    /// head reads as a different person at a distance where no feature
    /// resolves. The sides and the back stay — a bald man is not a hairless
    /// one — so the silhouette does not change with it.
    bald: bool,
    /// How much of a beard, `0` clean-shaven through stubble to `1` full.
    beard: f32,
    /// A hat, where he brought one. Painted over the hair rather than modelled,
    /// so it changes the colour of the silhouette without changing its shape.
    hat: Option<Vec3>,
}

impl Face {
    /// **Down the tile**: `0` at the crown of the skull, `1` under the collar.
    ///
    /// Laid out on the canons rather than by eye, because they are what a face
    /// IS: the eyes halfway between the crown and the chin, and the three
    /// thirds — hairline to brow, brow to the base of the nose, nose to chin —
    /// each the same height. Placed by eye they came out a tenth of a head too
    /// low, which is the difference between a man and a puppet.
    ///
    /// Tied to the rings the head is turned on as well:
    /// [`Crowd::SKULL`](crate::scene::crowd::Crowd) puts the widest ring of
    /// the lathe at 0.43, which is where [`Self::EYES`] is — because the
    /// widest part of a skull is the cheekbone and the cheekbone is at eye
    /// level.
    const HAIRLINE: f32 = 0.245;
    const BROW: f32 = 0.372;
    const EYES: f32 = 0.430;
    const NOSTRILS: f32 = 0.585;
    const MOUTH: f32 = 0.672;
    const CHIN: f32 = 0.790;
    /// Where the neck goes into the collar of whatever he came in.
    const COLLAR: f32 = 0.900;
    /// How far down the hair carries at the temples, before it stops being a
    /// hairline at all and becomes the back of his head.
    const TEMPLE: f32 = 0.44;

    /// **Round the tile**: `0` at the nose, `1` at the back of the skull.
    ///
    /// Not an angle, and that is the trap. `u` is the SQUARE ROOT of the angle
    /// (see [`CrowdPalette::head_uv`]), so a tile fraction `t` is `PI·t²`
    /// radians round from the nose and `0.099·sin` of that across a face — the
    /// numbers below are all checked back through that, against the same
    /// millimetres [`Painter`] gives the players. An eye written where it
    /// looked right on the sheet came out five centimetres off the mid-line,
    /// which is a lemur.
    ///
    /// Past [`Self::FRONT`] the surface has turned edge-on to anybody looking
    /// at it, and a feature smeared round there reads as a smudge on the side
    /// of a skull. Everything belonging to the FACE stops there; hair, beards
    /// and ears, which genuinely go round a head, do not.
    const FRONT: f32 = 0.60;
    /// The centre of each eye from the mid-line, and the half-width and
    /// half-height of the opening.
    const APART: f32 = 0.309;
    const OPEN_WIDE: f32 = 0.095;
    const OPEN_TALL: f32 = 0.030;
    /// The iris, and the pupil in it: round the head, then down it.
    const IRIS: (f32, f32) = (0.036, 0.023);
    const PUPIL: (f32, f32) = (0.016, 0.010);
    /// How far the brow runs, from the bridge of the nose outward; how thick
    /// it is; and how far the outer end rides above the inner one — a brow
    /// without that angle reads as a fringe.
    const BROW_INNER: f32 = 0.170;
    const BROW_OUTER: f32 = 0.424;
    const BROW_THICK: f32 = 0.019;
    const BROW_RISE: f32 = 0.029;
    /// Half-width of the nose at the nostrils, and where the hollows either
    /// side of it fall.
    const NOSE_WIDE: f32 = 0.227;
    const BESIDE_NOSE: f32 = 0.250;
    const MOUTH_WIDE: f32 = 0.289;
    /// The ear: how far round it sits, and how far it reaches either way.
    const EAR: (f32, f32) = (0.700, 0.082);
    /// Never a pure white and never a pure black. The two together are the
    /// highest-contrast pair in the whole scene, and twenty thousand of them
    /// read as a wall of dots rather than as people.
    const SCLERA: Vec3 = Vec3::new(0.74, 0.72, 0.69);
    const LASH: Vec3 = Vec3::new(0.10, 0.08, 0.07);
    const INK: Vec3 = Vec3::new(0.05, 0.04, 0.04);

    /// The colour of one texel of the tile.
    fn at(&self, x: u32, y: u32, tile: u32) -> Vec3 {
        // Texel CENTRES, so a band given as 0.40..0.46 covers the texels whose
        // middles fall inside it rather than the ones it clips a corner off.
        let down = (y as f32 + 0.5) / tile as f32;
        let across = (x as f32 + 0.5) / tile as f32 - 0.5;
        // How far round the head this texel is, and which side of the nose.
        let turn = (across * 2.0).abs();

        let mut paint = self.skin * self.shading(down, turn);
        if turn < Self::FRONT {
            paint = self.brows(self.eyes(paint, down, turn), down, turn);
            paint = self.nose(paint, down, turn);
            paint = self.mouth(paint, down, turn);
        }
        paint = self.ear(paint, down, turn);
        paint = self.whiskers(paint, down, turn);
        paint = self.scalp(paint, down, across, turn, tile);

        // The collar. A neck has to end in something or it ends in a ring of
        // bare skin sitting on a coat, which is the one join in a figure that
        // a lens can be walked right up to.
        paint = Textures::shade(paint, 0.55 * Self::hard(down - Self::COLLAR, tile));
        // …and a shave off one side of him, so the two halves of a head are
        // not a mirror image of each other. What gives a generated face away
        // fastest is perfect symmetry.
        Textures::shade(
            paint,
            0.03 * (1.0 + across.signum()) * Textures::smooth((turn - 0.20) / 0.60),
        )
    }

    /// An edge ramped over about a texel rather than stepped. At sixteen
    /// texels a step was all that could be afforded and every feature was a
    /// rectangle; at sixty-four a step is a staircase, and a staircase is what
    /// says "drawn by a machine".
    fn hard(inside: f32, tile: u32) -> f32 {
        Textures::smooth(inside * tile as f32 / 1.5 + 0.5)
    }

    /// A soft elliptical blob, `1` at its middle and gone by its own edge —
    /// which is most of what a face is made of at this size.
    fn blob(across: f32, down: f32, wide: f32, tall: f32) -> f32 {
        Textures::smooth(1.0 - Vec2::new(across / wide, down / tall).length())
    }

    /// **The relief a head carries before anything is drawn on it**, as a
    /// multiplier.
    ///
    /// One directional light with no shadow map lights a lathe as a lathe, so
    /// every bit of a face's own modelling — the temple falling away, the jaw
    /// in its own shade, the brow ridge catching the light, the cheekbone
    /// under the eye and the hollow below it — has to be in the paint. Without
    /// it the features sit on a flat disc. The same argument
    /// `Painter::shading` makes for the players, and mostly the same numbers:
    /// a crowd is made of the same people the teams are.
    fn shading(&self, down: f32, turn: f32) -> f32 {
        let turned = Textures::smooth(turn / 0.72);
        let mut shade = 1.0 - 0.26 * turned;
        // Under the jaw and down the neck, which is in the head's own shadow
        // for the whole match.
        shade *= 1.0 - 0.34 * Textures::smooth((down - Self::CHIN + 0.020) / 0.060);
        // The brow ridge, which catches the light…
        shade *= 1.0
            + 0.06 * Textures::smooth(1.0 - (down - Self::BROW).abs() / 0.030) * (1.0 - turned);
        // …the cheekbone under the eye, which catches it too…
        shade *= 1.0 + 0.05 * Self::blob(turn - 0.30, down - Self::EYES - 0.055, 0.22, 0.045);
        // …and the hollow of the cheek below that.
        shade * (1.0 - 0.07 * Self::blob(turn - 0.34, down - Self::NOSTRILS + 0.010, 0.20, 0.050))
    }

    fn eyes(&self, base: Vec3, down: f32, turn: f32) -> Vec3 {
        let sideways = turn - Self::APART;
        // Positive ABOVE the eye line, which is the way round every other
        // measurement of a face is written.
        let rise = Self::EYES - down;

        // The socket first: a soft shadow for the eye to sit in. Without one,
        // two white almonds on a flat plane read as buttons.
        let socket = Textures::shade(base, 0.16 * Self::blob(sideways, rise + 0.008, 0.115, 0.075));

        // The opening: a lens, wider than it is tall and pointed at both
        // corners, which is the shape that still says *eye* at ten texels
        // across. An ellipse says *bead*.
        let lid = Self::OPEN_TALL * (1.0 - (sideways / Self::OPEN_WIDE).powi(2)).max(0.0).sqrt();
        if lid <= 0.0 || rise.abs() > lid {
            return socket;
        }
        // The upper lid throws a shadow across the top of it — the one thing
        // that stops an eye reading as a hole punched in a mask.
        let mut colour = Textures::shade(Self::SCLERA, 0.34 * (rise / lid).max(0.0).powf(0.6));
        let ball = Vec2::new(sideways / Self::IRIS.0, (rise - 0.004) / Self::IRIS.1).length();
        if ball < 1.0 {
            // A dark limbal ring round a lighter iris: the rim is most of what
            // makes an eye colour legible at all once the iris itself is four
            // texels wide.
            colour = Textures::over(
                self.iris * 0.5,
                self.iris,
                Textures::smooth((1.0 - ball) / 0.40),
            );
        }
        if Vec2::new(sideways / Self::PUPIL.0, (rise - 0.004) / Self::PUPIL.1).length() < 1.0 {
            colour = Self::INK;
        }
        // The catchlight — the single cheapest thing in the whole figure that
        // makes it look alive.
        if Vec2::new((sideways + 0.013) / 0.015, (rise - 0.010) / 0.010).length() < 1.0 {
            colour = Vec3::splat(0.94);
        }
        // And the lash line along the top lid.
        if rise > lid - 0.007 {
            colour = Textures::over(colour, Self::LASH, 0.8);
        }
        colour
    }

    fn brows(&self, base: Vec3, down: f32, turn: f32) -> Vec3 {
        if !(Self::BROW_INNER..Self::BROW_OUTER).contains(&turn) {
            return base;
        }
        let along = (turn - Self::BROW_INNER) / (Self::BROW_OUTER - Self::BROW_INNER);
        // The outer end rides higher, and the arch peaks two thirds of the way
        // out rather than at the end of it.
        let line = Self::BROW - Self::BROW_RISE * along * (2.0 - along) + 0.008;
        let thick = Self::BROW_THICK * (1.0 - 0.55 * along * along);
        Textures::over(
            base,
            // A shade of the hair rather than a shadow of the skin: the brow
            // is the feature that survives longest as a head minifies, for the
            // reason `Complexion::face` gives on the pitch.
            self.hair.lerp(self.skin, 0.16),
            Textures::smooth((thick - (down - line).abs()) / 0.011),
        )
    }

    /// The nose, whole.
    ///
    /// A player's is a mesh of its own and all his face carries is what it
    /// does to the skin AROUND it; a spectator's head is a lathe with no room
    /// for one, so this has to be the shape as well: a lit ridge, the hollows
    /// either side, the wings at the nostrils, and the shadow the tip drops
    /// onto the lip — which at ten texels is most of what says there is a nose
    /// there at all.
    fn nose(&self, base: Vec3, down: f32, turn: f32) -> Vec3 {
        let along = Textures::smooth((down - Self::BROW) / 0.090)
            * (1.0 - Textures::smooth((down - Self::NOSTRILS) / 0.050));
        let mut colour = Textures::over(
            base,
            self.skin * 1.09,
            0.55 * along * Textures::smooth(1.0 - turn / (Self::NOSE_WIDE * 0.55)),
        );
        colour = Textures::shade(
            colour,
            // Tapered at BOTH ends of its run, not just the bottom: left to
            // fade only where the nose does, the hollow reaches from the brow
            // to the lip as a dark stripe under each eye, and a face with two
            // of those is a face that has been crying.
            0.10
                * along
                * Textures::smooth(along.min(1.0 - along) * 3.4)
                * Textures::smooth(1.0 - (turn - Self::BESIDE_NOSE).abs() / 0.060),
        );
        colour = Textures::shade(
            colour,
            0.16
                * Self::blob(
                    turn - Self::NOSE_WIDE * 0.86,
                    down - Self::NOSTRILS + 0.006,
                    0.075,
                    0.022,
                ),
        );
        Textures::shade(
            colour,
            0.24 * Self::blob(turn, down - Self::NOSTRILS - 0.012, Self::NOSE_WIDE * 1.15, 0.026),
        )
    }

    fn mouth(&self, base: Vec3, down: f32, turn: f32) -> Vec3 {
        // Lips are the skin with the blood closer to the surface: darker and a
        // good deal redder, and the same relation whatever the tone underneath
        // — which is why this is derived rather than picked.
        let lip = (self.skin * 0.80 + Vec3::new(0.12, 0.015, 0.02)).min(Vec3::ONE);
        let mut colour = Textures::over(
            base,
            lip,
            0.85 * Self::blob(turn, down - Self::MOUTH, Self::MOUTH_WIDE, 0.040),
        );
        // The line between them, which is the feature…
        colour = Textures::over(
            colour,
            self.skin * 0.34,
            Self::blob(turn, down - Self::MOUTH, Self::MOUTH_WIDE * 0.94, 0.015),
        );
        // …and the shadow under the lower lip, which does as much work as the
        // line does.
        Textures::shade(
            colour,
            0.16 * Self::blob(turn, down - Self::MOUTH - 0.050, Self::MOUTH_WIDE * 0.80, 0.022),
        )
    }

    /// The ear. No geometry to hang it on — a head is a lathe and an ear costs
    /// four more courses of one — so it is painted where the lathe is already
    /// turning away, which is exactly where an ear is anyway.
    fn ear(&self, base: Vec3, down: f32, turn: f32) -> Vec3 {
        let at = down - Self::EYES - 0.045;
        let colour = Textures::over(
            base,
            self.skin * 1.05,
            0.60 * Self::blob(turn - Self::EAR.0, at, Self::EAR.1, 0.090),
        );
        Textures::shade(
            colour,
            0.30 * Self::blob(turn - Self::EAR.0, at, Self::EAR.1 * 0.45, 0.048),
        )
    }

    /// A beard, where there is one.
    ///
    /// Cut along a line that hugs the jaw across the chin and then turns up
    /// the side of the face, which is an EASE and not a straight rise — a
    /// straight one puts a sharp V under the chin and has the beard up under
    /// the eye three centimetres out. The same shape `Painter::whiskers` cuts
    /// for the players.
    fn whiskers(&self, base: Vec3, down: f32, turn: f32) -> Vec3 {
        if self.beard <= 0.0 {
            return base;
        }
        let line = Self::MOUTH + 0.050 - 0.21 * Textures::smooth(turn / 0.52);
        let cover = Textures::smooth((down - line) / 0.045)
            * (1.0 - Textures::smooth((down - Self::CHIN - 0.055) / 0.045))
            * (1.0 - Textures::smooth((turn - 0.66) / 0.10));
        // The moustache is the same growth above the mouth rather than below
        // it, and head-on it is the half that says "beard".
        let tache = Textures::smooth((down - Self::MOUTH + 0.060) / 0.018)
            * (1.0 - Textures::smooth((down - Self::MOUTH + 0.014) / 0.014))
            * Textures::smooth(1.0 - turn / (Self::MOUTH_WIDE * 1.15));
        Textures::over(
            base,
            self.skin.lerp(self.hair, 0.85),
            self.beard * cover.max(tache),
        )
    }

    /// The hair, and whatever he put on over it.
    fn scalp(&self, base: Vec3, down: f32, across: f32, turn: f32, tile: u32) -> Vec3 {
        // A cap over the crown that carries down past the temples and covers
        // the whole back of the head. Bald heads keep the temples — a bald man
        // is not a hairless one — which is also what keeps the silhouette from
        // changing with it.
        let top = if self.bald { 0.0 } else { Self::HAIRLINE };
        let line = top
            + (Self::TEMPLE - top) * Textures::smooth((turn - 0.38) / 0.24)
            + (Self::COLLAR - Self::TEMPLE) * Textures::smooth((turn - 0.64) / 0.20)
            // Waved a little, because a hairline ruled straight across a
            // forehead is the one detail that reads as a wig at any distance.
            + 0.008 * (across * 29.0).sin() * (1.0 - Textures::smooth((turn - 0.50) / 0.30));
        let hair = Self::hard(line - down, tile);
        let mut colour = Textures::over(base, self.hair, hair);
        // A highlight, not a multiple of the colour, which is what a specular
        // is: black hair times anything is still black, and a whole head of it
        // reads as a hole cut in the stand.
        colour = Textures::over(
            colour,
            self.hair.lerp(Vec3::splat(0.36), 0.45),
            hair * 0.38 * Textures::smooth(((across * 26.0 + down * 7.0).sin() - 0.35) / 0.65),
        );

        let Some(hat) = self.hat else {
            return colour;
        };
        let brim = Self::HAIRLINE + 0.055;
        let under = Self::hard(brim - down, tile);
        colour = Textures::over(colour, hat, under);
        // The turn-up round the bottom of a woolly hat, and the shadow its
        // edge throws on the forehead under it.
        colour = Textures::over(
            colour,
            hat * 1.20,
            under * Textures::smooth((down - brim + 0.055) / 0.020),
        );
        Textures::shade(
            colour,
            0.26 * Textures::smooth((down - brim) / 0.012)
                * (1.0 - Textures::smooth((down - brim - 0.050) / 0.040)),
        )
    }
}

/// The colours a crowd is drawn from, and where each one sits on the strip.
///
/// Handed out together because they are one thing: the sheet is a run of flat
/// swatches whose only meaning is the index into it, so a mesh holding a `uv`
/// it worked out against a different swatch count would dress every spectator
/// in somebody else's coat. See [`Textures::crowd`].
pub struct CrowdPalette {
    pub sheet: Handle<Image>,
    /// Head tiles. They come first along the sheet.
    heads: usize,
    /// Neutral outerwear, which follows them.
    coats: usize,
    /// …the home club's own colours…
    ///
    /// A group of its own rather than a few more entries mixed into the coats,
    /// because who wears them is not a matter of chance: it depends on which
    /// bank a man is sitting in. See
    /// [`Stature::allegiance`](crate::scene::crowd::Stature::allegiance).
    club: usize,
    /// …and the visitors', last. Worn in exactly one place — the block of the
    /// far end their supporters were given.
    visiting: usize,
    /// Texels across one tile — what a half-texel inset is measured in.
    tile: f32,
}

impl CrowdPalette {
    /// The middle of the `pick`th head's FLAT tile, wrapped: his skin as one
    /// colour, which is what the back of his hand is painted with.
    ///
    /// The exact texel centre, which is what makes a palette safe to sample
    /// with a linear filter: at the centre of a solid tile the four taps of
    /// the bilinear fetch land on the same colour and it comes back unmixed.
    /// Every corner of the hand shares this, so the fetch has no gradient
    /// across it either and the hardware reads it at mip zero.
    pub fn head(&self, pick: u32) -> Vec2 {
        self.flat(pick as usize % self.heads)
    }

    /// …of the `pick`th NEUTRAL coat, which is what most of a ground wears.
    pub fn coat(&self, pick: u32) -> Vec2 {
        self.flat(self.heads + pick as usize % self.coats)
    }

    /// …of the `pick`th tile in the HOME club's colours, for the ones who came
    /// dressed for it.
    pub fn colours(&self, pick: u32) -> Vec2 {
        self.flat(self.heads + self.coats + pick as usize % self.club)
    }

    /// …and of the `pick`th tile in the VISITORS'.
    pub fn visitors(&self, pick: u32) -> Vec2 {
        self.flat(self.heads + self.coats + self.club + pick as usize % self.visiting)
    }

    /// How many tiles there are altogether, which is what a `u` is measured
    /// against.
    fn tiles(&self) -> f32 {
        (self.heads + self.coats + self.club + self.visiting) as f32
    }

    /// **Where one point on the `pick`th head lands on his own tile.**
    ///
    /// `turn` is how far round the skull the point is, `-1` and `1` at the
    /// back of the head and `0` at the nose; `down` is how far down the tile
    /// it belongs, `0` at the crown and `1` under the collar — read off the
    /// ring that carries it, not off its height, so the face gets the share of
    /// the tile a face deserves and the neck does not.
    ///
    /// **`u` is the SQUARE ROOT of the turn**, and that is the whole of what
    /// makes a wrapped head worth doing at this size. Spread evenly, sixty-four
    /// texels round a skull leaves the front ninety degrees — which is all a
    /// lens is ever pointed at — with sixteen of them, and an eye two texels
    /// wide is a smudge again. The root gives that quarter of the head half of
    /// the tile: thirty-two texels across a face, an eye four across, a pupil
    /// two. The cost is at the back, where the hair is one colour and could
    /// have been drawn at any resolution at all.
    ///
    /// This is also the one place in the crowd where a `uv` varies across a
    /// triangle, and that too is deliberate: it is what lets the hardware pick
    /// a mip level for a head, so a face six pixels wide resolves to its own
    /// average instead of to whichever texel the sample point happened to land
    /// on. See [`Textures::crowd`], which builds the chain that answers it.
    ///
    /// Inset half a texel all round, because the tiles share a sheet: sampled
    /// right to the edge the filter reaches into the neighbour, and every face
    /// wears a sliver of the next man's hair.
    pub fn head_uv(&self, pick: u32, turn: f32, down: f32) -> Vec2 {
        let column = pick as usize % self.heads;
        let span = 1.0 / self.tiles();
        let inset = span * 0.5 / self.tile;
        let (left, right) = (column as f32 * span + inset, (column + 1) as f32 * span - inset);
        let across = 0.5 + 0.5 * turn.signum() * turn.abs().clamp(0.0, 1.0).sqrt();

        // The drawn heads are the TOP half of the sheet and the flat colours
        // the bottom, so `v` runs over the first row only.
        let inset = 0.25 / self.tile;
        Vec2::new(
            left + (right - left) * across,
            (down * 0.5).clamp(inset, 0.5 - inset),
        )
    }

    /// The centre of a tile in the sheet's bottom row, which is where the flat
    /// colours live.
    fn flat(&self, tile: usize) -> Vec2 {
        Vec2::new((tile as f32 + 0.5) / self.tiles(), 0.75)
    }
}

#[cfg(test)]
impl CrowdPalette {
    /// A palette with nothing behind it, for the parts of the crowd that only
    /// ever ask it where a tile is.
    pub fn of_swatches(heads: usize, coats: usize, club: usize, visiting: usize) -> Self {
        CrowdPalette {
            sheet: Handle::default(),
            heads,
            coats,
            club,
            visiting,
            tile: 64.0,
        }
    }
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

    /// And the two arrows, marking a change.
    ///
    /// One pointing on, one pointing off, stacked — the sign the fourth
    /// official holds up, which is the only glyph in football that means
    /// substitution and nothing else. Drawn horizontally rather than
    /// vertically because the pin is a circle: two long shapes side by side
    /// waste the round corners, two stacked ones fill them.
    ///
    /// Both arrows are the same shape and one is turned through half a turn,
    /// so the pair reads as a swap rather than as two unrelated marks. The
    /// head is deliberately more than twice the width of the shaft: at a
    /// dozen pixels an arrow whose head only just outgrows its stem reads as
    /// a plain bar, which is the same mistake [`Self::chance_icon`] documents
    /// at its own size.
    pub fn substitution_icon(images: &mut Assets<Image>) -> Handle<Image> {
        /// Where the two arrows sit, as a fraction down the square, and how
        /// thick the shafts are.
        const ROW: f32 = 0.29;
        const SHAFT: f32 = 0.075;
        /// The span each arrow covers across the square, and where its head
        /// begins.
        const TAIL: f32 = 0.16;
        const NOSE: f32 = 0.84;
        const HEAD_AT: f32 = 0.52;
        const HEAD_HALF: f32 = 0.20;

        images.add(Self::mask(|x, y| {
            // Fold the two arrows onto one: the lower one is the upper one
            // rotated half a turn about the centre of the square.
            let (along, across) = if y < 0.5 {
                (x, y - ROW)
            } else {
                (1.0 - x, (1.0 - y) - ROW)
            };
            let across = across.abs();
            if !(TAIL..=NOSE).contains(&along) {
                return false;
            }
            if along <= HEAD_AT {
                return across <= SHAFT;
            }
            // The head: a triangle closing from `HEAD_HALF` at its base to
            // nothing at the point.
            let into_head = (along - HEAD_AT) / (NOSE - HEAD_AT);
            across <= HEAD_HALF * (1.0 - into_head)
        }))
    }

    /// The ring the picture comes up through when the replay cuts from one clip
    /// to the next — see [`crate::broadcast::cut`], which is the only thing
    /// that draws it.
    ///
    /// White throughout with all the weight in the ALPHA, like [`Self::mask`]
    /// above it and for a second reason as well as the first: `ImageNode::color`
    /// then carries BOTH the colour of the dip and how far through it we are,
    /// so a fade is one float written per frame rather than a texture rebuilt
    /// per frame.
    ///
    /// Three numbers, and the shape of a lens rather than of a border:
    ///
    /// - [`CORE`] is how much of the picture survives in the middle at the
    ///   instant of the cut. Not zero — a full dip to black hides the cut
    ///   instead of announcing it, and the point is to see the new episode
    ///   arrive.
    /// - [`REACH`] is where the ring closes to solid, in half-widths. Past
    ///   one, so the mid-edges are dark and the corners are darker still,
    ///   which is what makes this read as a lens rather than as a curtain.
    /// - [`EYE`] puts the middle of it above the middle of the frame, where
    ///   the eye already thinks the centre of a picture is. It is the same 45%
    ///   the match page's own loading ground uses.
    ///
    /// [`CORE`]: Self::vignette
    /// [`REACH`]: Self::vignette
    /// [`EYE`]: Self::vignette
    pub fn vignette(images: &mut Assets<Image>) -> Handle<Image> {
        /// Big enough that a gradient this soft is not stretched into visible
        /// steps across a 4K canvas, small enough to be a quarter of a
        /// megabyte. There is no detail in here to lose: it is one ramp.
        const SIZE: u32 = 256;
        const CORE: f32 = 0.50;
        const REACH: f32 = 1.6;
        const EYE: f32 = 0.45;

        let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for row in 0..SIZE {
            for column in 0..SIZE {
                let across = (column as f32 + 0.5) / SIZE as f32 - 0.5;
                let down = (row as f32 + 0.5) / SIZE as f32 - EYE;
                let reach = (across * across + down * down).sqrt() / (0.5 * REACH);
                let weight = CORE + (1.0 - CORE) * Self::smooth(reach);
                data.extend_from_slice(&[255, 255, 255, (weight * 255.0).round() as u8]);
            }
        }
        images.add(Self::image(SIZE, SIZE, data))
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

        let printed = Self::as_printed(name)?;
        Some(Self::lettering(images, &printed, WIDTH, HEIGHT, 0.94, 0.88))
    }

    /// **The walk-out name, set on a plate rather than straight onto the
    /// cloth**: a rounded rectangle in the kit's print colour with the surname
    /// knocked out of it in the shirt's own colour.
    ///
    /// The back of the shirt is lettering and nothing else, because that is
    /// what a shirt is. The FRONT carries this instead (2026-08-30, maintainer:
    /// *"on red-white team i want see white rectangle with rounded corners and
    /// red text"*), and the inversion is the point: the ceremony's pass comes
    /// down the line at four metres with eleven men in the same strip, and a
    /// white-on-blue name is one more thing on a blue chest while a white plate
    /// is a shape the eye finds before it reads anything. Every accreditation
    /// badge, every squad-number patch and every warm-up top on earth does the
    /// same.
    ///
    /// # Why the colours are baked in and not left to the material
    ///
    /// [`Self::lettering`] paints white and lets [`Wardrobe::printed`] tint the
    /// whole panel off the strip, which works exactly as long as a print is ONE
    /// colour. This one is two, and the second is behind the first. So the
    /// texels carry the colours and the material is left white; the panel is
    /// per-player anyway, since the name is.
    ///
    /// [`Wardrobe::printed`]: crate::players::kit::Wardrobe
    pub fn name_plate(
        images: &mut Assets<Image>,
        name: &str,
        plate: Color,
        ink: Color,
    ) -> Option<Handle<Image>> {
        /// Half again as tall as the back panel for the same lettering: the
        /// air above and below the letters is the plate. Matches the shape of
        /// [`BodyParts::NAME_FRONT_ARC`] × `NAME_FRONT_HEIGHT` — 25 cm by 7.6
        /// — or the whole thing prints stretched.
        ///
        /// [`BodyParts::NAME_FRONT_ARC`]: crate::players::body::BodyParts
        const WIDTH: u32 = 176;
        const HEIGHT: u32 = 54;
        /// Letters the same size on the plate as off it: the back print sets
        /// 0.88 of 40 texels — 5.1 cm of cloth — and this is that same 5.1 cm
        /// out of the 7.6 the taller panel covers.
        const CAP: f32 = 0.67;
        /// …and narrower than the back print's 0.94, because the plate has to
        /// get round the outside of them.
        const MARGIN: f32 = 0.80;
        /// How far the plate stands off the lettering, in texels: out to the
        /// sides, where it is clamped by the panel's own edge, and top and
        /// bottom, where the plate is a fixed band so that eleven men in a row
        /// wear the same one whether or not their names have a descender.
        const PAD_X: f32 = 13.0;
        const BLEED: f32 = 2.0;
        /// The narrowest a plate may be, in texels, however short the name on
        /// it. LI padded by [`PAD_X`] alone comes out very nearly square, which
        /// does not read as a name badge; this holds it to about three halves
        /// of its own height, which does.
        ///
        /// [`PAD_X`]: Self::name_plate
        const NARROWEST: f32 = 76.0;
        /// The corner radius, as a share of the plate's height. A patch, not a
        /// pill: enough that the corner is visibly turned at four metres and
        /// not so much that the shape stops being a rectangle.
        const CORNER: f32 = 0.24;
        /// Coverage below this is a glyph's antialiased skirt rather than the
        /// glyph, and including it in the ink box would grow the plate by a
        /// texel or two of nothing.
        const INKED: u8 = 24;

        let printed = Self::as_printed(name)?;
        let glyphs = Stencil::mask(&printed, WIDTH, HEIGHT, MARGIN, CAP);

        // Where the lettering actually reaches across the panel. Fitting is
        // done to the WIDEST of the two constraints, so a short name is held
        // off the margin by the cap and its plate must not be a full-width
        // band; measuring is the only way to know which happened.
        let mut from = WIDTH as f32;
        let mut to = 0.0f32;
        for (index, coverage) in glyphs.iter().enumerate() {
            if *coverage >= INKED {
                let column = (index as u32 % WIDTH) as f32;
                from = from.min(column);
                to = to.max(column + 1.0);
            }
        }
        if from >= to {
            return None;
        }
        let left = (from - PAD_X).max(BLEED);
        let right = (to + PAD_X).min(WIDTH as f32 - BLEED);
        // Grown about its own middle rather than about the panel's, so a plate
        // that had to be pushed off one edge does not walk back onto it. The
        // lettering is centred, so on everything but a clamped plate the two
        // are the same point anyway.
        let centre = Vec2::new(
            ((left + right) * 0.5).clamp(
                BLEED + NARROWEST * 0.5,
                WIDTH as f32 - BLEED - NARROWEST * 0.5,
            ),
            HEIGHT as f32 * 0.5,
        );
        let half = Vec2::new(
            (right - left).max(NARROWEST) * 0.5,
            (HEIGHT as f32 - BLEED * 2.0) * 0.5,
        );

        // The plate's own mask, painted by the same signed-distance routine the
        // perimeter boards use — so a shirt and a hoarding round the same corner
        // the same way, and there is one rounded rectangle in the crate.
        let mut cover = vec![0u8; (WIDTH * HEIGHT) as usize];
        Self::rounded(
            &mut cover,
            WIDTH,
            HEIGHT,
            centre,
            half,
            half.y * 2.0 * CORNER,
        );

        let plate = plate.to_srgba();
        let ink = ink.to_srgba();
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;

        let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for texel in 0..(WIDTH * HEIGHT) as usize {
            // The glyph decides the colour and the plate decides whether there
            // is anything there at all — the lettering is a hole in the plate,
            // so it can never reach outside one.
            let letter = glyphs[texel] as f32 / 255.0;
            let mix = |plate: f32, ink: f32| channel(plate + (ink - plate) * letter);
            data.extend_from_slice(&[
                mix(plate.red, ink.red),
                mix(plate.green, ink.green),
                mix(plate.blue, ink.blue),
                cover[texel],
            ]);
        }
        Some(images.add(Self::image(WIDTH, HEIGHT, data)))
    }

    /// The surname as any panel on the shirt will set it: folded to what the
    /// face can draw, and cut to a length a shirt printer would accept.
    ///
    /// `None` when nothing survives, and the shirt then goes out with its
    /// number alone rather than with a row of blanks.
    fn as_printed(name: &str) -> Option<String> {
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
        Some(printed)
    }

    /// White glyphs on transparent, centred and sized to fill the image.
    ///
    /// `margin` is the share of the width they may use and `cap` the share of
    /// the height, which is what separates a shirt number — as tall as the
    /// panel will take — from a name, which is set smaller with air above and
    /// below it.
    ///
    /// The outlines come from [`Stencil`], which is to say from the same face
    /// the player's own label is set in. Everything on this player is drawn
    /// with one typeface — and so, since [`Self::hoarding`] followed the shirts
    /// off the 5×7 grid, is everything behind him.
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
    /// ⚠ **Nobody takes the field wearing one of these.** A face on the pitch
    /// is a picture of the man or it is nothing — see
    /// [`crate::players::portrait`] — so the only caller left is the dump that
    /// puts a drawn head beside a photographed one to compare them. The paint
    /// itself is very much alive: [`Self::photographed_face`] lays the picture
    /// over this same sheet, and everything the camera never saw is still what
    /// this draws.
    #[cfg(test)]
    pub fn face(images: &mut Assets<Image>, layout: &FaceLayout, look: &FaceLook) -> Handle<Image> {
        images.add(Self::face_sheet(layout, look, None, Grain::FULL))
    }

    /// The same head with a real picture of it laid over the front — his
    /// photograph, or the portrait his profile page draws when there is none.
    ///
    /// Painted first and covered second, rather than replaced: the picture is
    /// a flat frontal of a face and a head is a head all the way round, so
    /// everything the camera never saw — the sides, the back, the underside of
    /// the jaw — is still the generated face, and the two meet on a soft edge
    /// rather than a cut.
    pub fn photographed_face(
        layout: &FaceLayout,
        look: &FaceLook,
        portrait: &Portrait,
        grain: Grain,
    ) -> Image {
        Self::face_sheet(layout, look, Some(portrait), grain)
    }

    fn face_sheet(
        layout: &FaceLayout,
        look: &FaceLook,
        portrait: Option<&Portrait>,
        grain: Grain,
    ) -> Image {
        // One of these per man, and a photographed one carries a mip chain —
        // 341 KiB apiece at `PICTURE`'s full size. Charged apart from the
        // scenery because the two grow for entirely different reasons: the
        // stadium is built once, and this is built again every time somebody's
        // photograph lands.
        let _charge = MemoryBill::charge(Held::Faces);
        /// Wide enough that an eye — three centimetres of a fifty-six
        /// centimetre circumference — lands about nine texels across, which is
        /// the least an iris and a pupil can be told apart in.
        const WIDTH: u32 = 128;
        const HEIGHT: u32 = 96;

        // …and what a PICTURE gets, which is more than five times as many
        // texels.
        //
        // The sheet above is sized to the rule the rest of this file works
        // to: one texel on about one pixel at the range a face is looked at
        // from. That rule is right for a painted face, whose features are
        // drawn to be legible at exactly that size — and wrong for a
        // photograph, which is not a diagram of a face but a picture of one.
        // Squeezed onto the painted sheet a man's own head shot comes out as
        // a tinted smudge: the fifty texels across the front of his face are
        // enough to say "a face" and nowhere near enough to say WHOSE.
        //
        // SQUARE rather than the 4:3 the painted one is, because the two
        // axes are not the same problem. Across, the sheet has always had
        // more texels than a head shot has pixels to fill them. Down, at 192,
        // it had 580 to the metre against the photograph's 700 — so the last
        // sixth of the detail in every face was being thrown away at the one
        // step that had it to spare. 256 puts the sheet ahead of the picture
        // on both axes, which is where the limit belongs.
        //
        // The cost of breaking the rule is minification crawl, and the answer
        // to that is the mip chain below rather than a smaller sheet.
        //
        // ⚠ **And it is the grain's to say now, not this constant's.** The
        // argument above is a picture argument and it is right — on a machine
        // with the memory to hold it. There is one of these per man, with a
        // full mip chain, which is 341 KiB each and the largest per-player
        // allocation a squad has; on a phone that is nine megabytes across a
        // team sheet, on a device whose tab is killed for what it holds.
        // [`Grain::SPARE`] halves the side, which quarters the sheet, and at
        // the range a handheld watches a match from the picture is
        // indistinguishable — the head is a few dozen pixels across and the
        // sheet was several times what the screen could show either way.
        let (width, height) = if portrait.is_some() {
            (grain.face(), grain.face())
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

    /// Lays a rounded rectangle into an alpha mask: the shape of the logo tile
    /// on a perimeter board, and — with a radius of half its own thickness —
    /// of the pill that divides the lockup.
    ///
    /// Signed distance rather than the supersampling the glyph grid used to do
    /// here: the shape is analytic, so the exact distance to its edge is both
    /// cheaper than sixteen inside/outside tests a texel and a better edge than
    /// they gave. Feathered over one texel, which is all antialiasing a hard
    /// edge means at this size.
    ///
    /// Ink already in the mask survives where it is darker, so a tile and the
    /// letters over it can share one buffer.
    fn rounded(
        coverage: &mut [u8],
        width: u32,
        height: u32,
        centre: Vec2,
        half: Vec2,
        radius: f32,
    ) {
        // A radius past either half-extent is not a rounder rectangle, it is a
        // shape that turns inside out.
        let radius = radius.min(half.x).min(half.y);
        // The box the corners are struck from: the full half-extent pulled in
        // by the radius on both axes.
        let core = (half - Vec2::splat(radius)).max(Vec2::ZERO);
        for y in 0..height {
            for x in 0..width {
                let offset = (Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - centre).abs() - core;
                // Outside the core box on both axes this is the distance to a
                // corner, on one axis the distance to a side, and on neither a
                // negative depth into the middle.
                let distance =
                    offset.max(Vec2::ZERO).length() + offset.x.max(offset.y).min(0.0) - radius;
                let alpha = 1.0 - Self::smooth(distance + 0.5);
                if alpha > 0.0 {
                    let target = &mut coverage[(y * width + x) as usize];
                    *target = (*target).max((alpha * 255.0) as u8);
                }
            }
        }
    }

    /// A panel of flat colour — the whole of what a board carries when the face
    /// behind its lockup cannot be read. See [`Self::advert`].
    fn plain(width: u32, height: u32, colour: Vec3) -> Vec<u8> {
        [
            (colour.x * 255.0) as u8,
            (colour.y * 255.0) as u8,
            (colour.z * 255.0) as u8,
            255,
        ]
        .repeat((width * height) as usize)
    }

    /// One advertising panel for the perimeter boards, to be tiled along them:
    /// the project's mark, its name and its address, set as one lockup on the
    /// board's own dark tone.
    ///
    /// # The face
    ///
    /// The lettering used to come off a hand-rolled 5×7 grid — the last of the
    /// viewer's pixel type, left here long after the shirts moved to real
    /// outlines. A ground where the squad is set in Outfit and the boards
    /// around them are set on graph paper reads as two different games, which
    /// is the same argument [`Self::lettering`] makes about a number next to a
    /// name. So a board now goes through [`Stencil`] as well, and there is one
    /// typeface in the scene.
    ///
    /// # Sizing
    ///
    /// **The height is the constraint, not the width.** A board is 0.95 m tall
    /// and lands about 60 px on screen from the near touchline and 22 px from
    /// the far one. The panel used to be 64 texels tall for exactly that — one
    /// texel on one pixel where it matters — because a sharper sheet with no
    /// mip chain under it would crawl and sparkle along the touchline every
    /// time the camera panned, which is the one thing that would make
    /// advertising look worse than no advertising.
    ///
    /// There is a chain under it now, and anisotropy over it, so the sheet can
    /// be sampled for the camera that walks up to a board rather than for the
    /// one on the halfway line: 1024 × 128 for the same 7.6 m of hoarding.
    /// Both axes doubled together, so a texel stays square and the lockup is
    /// neither stretched nor squashed.
    pub fn hoarding(
        images: &mut Assets<Image>,
        mark: &str,
        name: &str,
        address: &str,
    ) -> Handle<Image> {
        const WIDTH: u32 = 1024;
        const HEIGHT: u32 = 128;

        // Chained for the same reason the seating is: these ring the pitch and
        // are seen almost edge-on from a low rig, where a repeated wordmark
        // undersampled along its own length is a row of sparkling confetti.
        let mut image = Self::mipped(
            WIDTH,
            HEIGHT,
            Self::advert(WIDTH, HEIGHT, mark, name, address),
        );
        // Tiled along boards of two different lengths, so the repeat lives in
        // the sampler and the count in each board's `uv_transform`.
        if let ImageSampler::Descriptor(descriptor) = &mut image.sampler {
            descriptor.address_mode_u = ImageAddressMode::Repeat;
            // A perimeter board is the most foreshortened surface in the scene:
            // it runs straight away from a camera standing at the touchline,
            // and plain trilinear filtering picks a mip level from the WIDER of
            // the two directions a texel is stretched over. So the far half of
            // every board was read out of a level that had already averaged the
            // lettering away, and no amount of drawing the panel better would
            // have shown up down there. Same trade — and the same "free on
            // essentially every device" — as [`Self::tiled`] makes for the
            // turf.
            descriptor.anisotropy_clamp = 16;
        }
        images.add(image)
    }

    /// The panel's texels, apart from the image they end up in — so a board can
    /// be dumped and LOOKED at without building 28 MB of wasm, which is the
    /// only way to review a lockup at all. See `textures::tests::dump_hoarding`.
    fn advert(width: u32, height: u32, mark: &str, name: &str, address: &str) -> Vec<u8> {
        /// Share of the panel width the lockup may use. What is left is the air
        /// between one repeat and the next, and it has to be clearly wider than
        /// any gap inside the lockup or the boards read as one run-on sentence
        /// instead of as a panel that repeats.
        const MARGIN: f32 = 0.88;
        /// The logo tile and its corner radius, as shares of the board's height
        /// and of the tile's own side. The radius is `favicon.svg`'s, which is
        /// 10 on a 64 box.
        const TILE: f32 = 0.72;
        const CORNER: f32 = 0.16;
        /// Cap height of the initials inside the tile, as a share of its side.
        /// A mark is mostly air; crowd the letters to the edge and it stops
        /// reading as a logo and starts reading as a third word.
        const MARK: f32 = 0.44;
        /// The two text sizes, as shares of the board's height — and they are
        /// ink ABOVE the baseline, not boxes to fit into. The name and the
        /// address sit on ONE baseline, so what makes the address secondary is
        /// how far up it reaches.
        ///
        /// These are nominal, and the fit below scales the whole lockup down to
        /// the panel — so raising one of them does not simply enlarge it, it
        /// buys that piece a bigger share of a fixed width. A first cut asked
        /// for 0.40 and 0.24 and printed a 33 cm wordmark on a 95 cm board,
        /// which is a caption; a perimeter sponsor sets its name at 40 cm and
        /// up, and the address is what gives way for it.
        const NAME: f32 = 0.50;
        const ADDRESS: f32 = 0.22;
        /// Air between the mark, the name, the divider and the address.
        const GAP: f32 = 0.26;
        /// The divider: a thin pill, about as tall as the wordmark's own ink.
        const RULE: f32 = 0.05;
        const RULE_HEIGHT: f32 = 0.46;
        /// Tracking for each of the three lines — see [`Stencil::set`] for why
        /// this is not one number. A shirt's 0.08 is loose on purpose; a
        /// wordmark wants to hold together as a word, and an address set at
        /// half its size wants the air back.
        const MARK_TRACKING: f32 = 0.04;
        const NAME_TRACKING: f32 = 0.02;
        const ADDRESS_TRACKING: f32 = 0.06;

        // Exactly the board's own colour, so the seam between an advertised
        // panel and the plain structure behind it never shows.
        let ground = Vec3::new(0.200, 0.230, 0.300);
        let letters = Vec3::new(0.95, 0.96, 0.99);
        // The mark's own teal — `favicon.svg`'s #0e637f — lifted for a dark
        // ground. At its web value the tile is barely a shade off the board it
        // sits on, and the logo reads as two letters floating in the middle of
        // nothing; lifted, it is a tile with a mark in it.
        let brand = Vec3::new(0.075, 0.470, 0.590);
        // One accent for the divider and the address, out of the same family as
        // the tile and the seats. Pale rather than saturated: this is the
        // smallest lettering on the board and it has to survive being 22 px
        // away.
        let accent = Vec3::new(0.42, 0.78, 0.88);

        let (Some(mark), Some(name), Some(address)) = (
            Stencil::set(mark, MARK_TRACKING),
            Stencil::set(name, NAME_TRACKING),
            Stencil::set(address, ADDRESS_TRACKING),
        ) else {
            // Nothing the compiled-in face can draw, which would mean no text
            // anywhere in the viewer rather than a problem with this panel. A
            // plain board is the right way to lose it: the perimeter is still
            // there, it simply carries no advertising.
            return Self::plain(width, height, ground);
        };

        // Everything at its nominal size first, and then the whole lockup
        // scaled by however much of the panel it turned out to want. Fitting
        // the pieces one at a time instead would let the wordmark and the
        // address disagree about how hard each had been squeezed, which is
        // exactly what makes a lockup look assembled rather than drawn.
        let board = height as f32;
        let tile = TILE * board;
        let gap = GAP * board;
        let rule = RULE * board;
        let name_scale = NAME * board / name.rise();
        let address_scale = ADDRESS * board / address.rise();
        let run = tile
            + gap
            + name.span() * name_scale
            + gap
            + rule
            + gap
            + address.span() * address_scale;
        let fit = (width as f32 * MARGIN / run).min(1.0);

        let tile = tile * fit;
        let gap = gap * fit;
        let rule = rule * fit;
        let name_scale = name_scale * fit;
        let address_scale = address_scale * fit;

        // One baseline under the name and the address both, with the ink of the
        // pair of them — the taller line's rise, the deeper line's drop —
        // centred on the board. A centred ink box's own middle is the middle of
        // the panel, which is what the tile and the divider centre on too.
        let rise = name.rise() * name_scale;
        let drop = (name.drop() * name_scale).max(address.drop() * address_scale);
        let baseline = (board + rise - drop) * 0.5;
        let middle = board * 0.5;

        // Three layers, composited in this order at the end: the tile under
        // everything, the accent over it, the white lettering over both. They
        // only actually overlap where the initials cross the tile, which is the
        // one place the order has to be right.
        let mut tiles = vec![0u8; (width * height) as usize];
        let mut accents = vec![0u8; (width * height) as usize];
        let mut ink = vec![0u8; (width * height) as usize];

        let mut left = (width as f32 - run * fit) * 0.5;
        Self::rounded(
            &mut tiles,
            width,
            height,
            Vec2::new(left + tile * 0.5, middle),
            Vec2::splat(tile * 0.5),
            tile * CORNER,
        );
        // The initials centre in the tile rather than sitting on the lockup's
        // baseline: a mark is a picture of two letters, not the first word of
        // the line.
        let mark_scale = MARK * tile / mark.rise();
        mark.draw(
            &mut ink,
            width,
            height,
            left + (tile - mark.span() * mark_scale) * 0.5,
            middle + (mark.rise() - mark.drop()) * mark_scale * 0.5,
            mark_scale,
        );
        left += tile + gap;

        name.draw(&mut ink, width, height, left, baseline, name_scale);
        left += name.span() * name_scale + gap;

        // A pill rather than a bar: at this thickness a square end is two stray
        // texels and reads as a nick in the rule.
        Self::rounded(
            &mut accents,
            width,
            height,
            Vec2::new(left + rule * 0.5, middle),
            Vec2::new(rule * 0.5, RULE_HEIGHT * board * fit * 0.5),
            rule * 0.5,
        );
        left += rule + gap;

        address.draw(&mut accents, width, height, left, baseline, address_scale);

        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for index in 0..(width * height) as usize {
            let mut colour = Self::over(ground, brand, tiles[index] as f32 / 255.0);
            colour = Self::over(colour, accent, accents[index] as f32 / 255.0);
            colour = Self::over(colour, letters, ink[index] as f32 / 255.0);
            data.extend_from_slice(&[
                (colour.x * 255.0) as u8,
                (colour.y * 255.0) as u8,
                (colour.z * 255.0) as u8,
                255,
            ]);
        }
        data
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
    /// authored across a whole ground instead of inside a two-metre square. See
    /// [`crate::scene::pitch::Sward`], which carries it, the mow and the wear
    /// of a played match in the vertices of the pitch.
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

        /// …and how far the RELIEF is asked to survive, which is not the same
        /// number and should never have been.
        ///
        /// The note above is an argument about the albedo: blade detail that
        /// washes out to the mean is a pitch that has gone back to being a
        /// green rectangle, and only a long anisotropic footprint reaches the
        /// far touchline with any of it left. None of that transfers. A normal
        /// map is not a picture, it is three signed numbers per texel that the
        /// shader turns into a lighting term, and beyond a few metres the sward
        /// it describes is finer than a pixel — so what sixteen taps recover
        /// there is per-leaf shading at sub-pixel scale, which does not read as
        /// grass. It reads as sparkle, and sparkle that re-rolls as the camera
        /// pans is precisely the crawling this is supposed to prevent.
        ///
        /// Four taps is where the relief stops being undersampled and starts
        /// being oversampled, and the saving is not small: this is the second
        /// of TWO anisotropic fetches on a surface that covers most of the
        /// frame, so the pitch's sampler cost falls by about a third. On a
        /// discrete card that is a rounding error. On an integrated part,
        /// where the texture units and the memory bus are shared with
        /// everything else in the machine, it is one of the two largest costs
        /// in the frame — see `quality`, which handles the other one.
        const ACROSS_A_LEAF: u16 = 4;

        // ⚠ **The order of these four lines is a memory decision.**
        //
        // This is the first course of the bring-up and the largest transient
        // in it. Four buffers want to exist here — the two height fields, the
        // albedo's texels and the relief's — at four megabytes each, and each
        // of the last two then grows into a mip chain half as big again. Left
        // to fall out naturally they overlapped: `dry` was still alive while
        // the relief was rasterised, and both chains were built with both
        // fields still standing, which measured 25–30 MiB of peak on the heap.
        //
        // On wasm32 a peak is not a peak, it is a floor: linear memory only
        // grows, so every byte held at the worst instant of the load is a byte
        // the browser counts against the tab for the rest of the session — and
        // on iOS crossing that ceiling is not a slow frame, it is the renderer
        // being killed. See [`MemoryBill`](crate::app::bill::MemoryBill).
        //
        // So each buffer is dropped the moment nothing needs it again: `dry`
        // once the colour is written, `lit` once the relief is taken off it,
        // and each set of texels as its chain consumes it. Peak ~12 MiB.
        drop(dry);
        let relief = Self::relief(SIZE, &lit);
        drop(lit);

        Turf {
            albedo: images.add(Self::tiled(Self::mipped(SIZE, SIZE, data), ALONG_THE_PITCH)),
            relief: images.add(Self::tiled(
                Self::mipped_linear(SIZE, SIZE, relief),
                ACROSS_A_LEAF,
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
                // Tangent space here is the one [`crate::scene::pitch::Sward`]
                // writes out: U on +X, V on +Z, and the green channel positive
                // along V, which is the convention Bevy's own normal maps use.
                let normal = Vec3::new(-across * RELIEF, -down * RELIEF, 1.0).normalize_or(Vec3::Z);
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

    /// A chain for a sheet of TILES, stopped where one texel is one tile.
    ///
    /// The stopping point is the whole of it. A tile is a power of two across
    /// and sits on a multiple of its own width, so every box filter down to
    /// that level takes its four taps from inside a single tile and two tiles
    /// never average together — the chain is a per-tile chain that happens to
    /// be packed as one image. One level further and a texel would straddle
    /// two of them, which for a palette is the end of it being a palette.
    ///
    /// So the coarsest level is exactly one texel per tile, holding that
    /// tile's own average, and a spectator's head at the far side of the
    /// ground resolves to his own skin rather than to the mean of the sheet.
    /// See [`Self::crowd`], which is the only caller and carries the argument.
    fn mipped_tiles(width: u32, height: u32, base: Vec<u8>, tile: u32) -> Image {
        Self::mipped_capped(
            width,
            height,
            base,
            TextureFormat::Rgba8UnormSrgb,
            tile.trailing_zeros() + 1,
        )
    }

    fn mipped_as(width: u32, height: u32, base: Vec<u8>, format: TextureFormat) -> Image {
        Self::mipped_capped(width, height, base, format, u32::MAX)
    }

    /// ⚠ **The chain is built IN PLACE, in one buffer, and that is a memory
    /// decision rather than a tidiness one.**
    ///
    /// It used to accumulate a `Vec` per level and then concatenate the lot
    /// into a `Vec::new()`. Three costs, all of them paid at once on the
    /// browser's only thread during the first course of the bring-up: every
    /// level alive at the same time as the copy of it, the copy itself, and
    /// the copy's buffer DOUBLING its way up from nothing while all of that
    /// stood. For the pitch's two 1024-square sheets that came to some 17 MiB
    /// of transient per sheet against 5.6 MiB of chain.
    ///
    /// On a desktop it would be nothing; on wasm32 it is permanent. Linear
    /// memory only ever grows — there is no `memory.shrink` — so a peak the
    /// allocator later reuses is still a page the browser counts against the
    /// tab forever. See [`MemoryBill`](crate::app::bill::MemoryBill).
    ///
    /// So: the total is worked out first, the base buffer is grown to it once,
    /// and every level is filtered out of the bytes already in the buffer and
    /// pushed onto its own end. Reading level *n* while writing level *n+1* is
    /// safe by index because the two never overlap — a level starts where its
    /// parent ended — and no reference is held across the push.
    fn mipped_capped(
        width: u32,
        height: u32,
        base: Vec<u8>,
        format: TextureFormat,
        cap: u32,
    ) -> Image {
        // How many levels there are and what they come to, before a byte
        // moves. Every format this is called with is four bytes a texel.
        let mut mip_level_count = 1u32;
        let (mut across, mut down) = (width, height);
        let mut total = (width * height * 4) as usize;
        while mip_level_count < cap && (across > 1 || down > 1) {
            across = (across / 2).max(1);
            down = (down / 2).max(1);
            total += (across * down * 4) as usize;
            mip_level_count += 1;
        }

        let mut data = base;
        data.reserve_exact(total.saturating_sub(data.len()));
        let (mut across, mut down) = (width, height);
        // Where the level being READ starts. Level 0 is at the front.
        let mut start = 0usize;
        for _ in 1..mip_level_count {
            let (half_across, half_down) = ((across / 2).max(1), (down / 2).max(1));
            let next = data.len();
            // Row and column offsets hoisted out of the sample loop. The
            // obvious way to write this recomputes `(y * width + x) * 4 +
            // channel` for all four taps of all four channels of every texel,
            // which is sixteen multiplications where two would do — and this
            // is the hot end of the whole load: a 1024-square sheet and its
            // chain is some twenty million taps, and the pitch now builds two
            // of them. Same arithmetic, same clamping at the far edge, an
            // eighth of the address maths.
            for y in 0..half_down {
                let top = start + ((y * 2).min(down - 1) * across) as usize * 4;
                let bottom = start + ((y * 2 + 1).min(down - 1) * across) as usize * 4;
                for x in 0..half_across {
                    let left = (x * 2).min(across - 1) as usize * 4;
                    let right = (x * 2 + 1).min(across - 1) as usize * 4;
                    for channel in 0..4 {
                        let sum = data[top + left + channel] as u32
                            + data[top + right + channel] as u32
                            + data[bottom + left + channel] as u32
                            + data[bottom + right + channel] as u32;
                        data.push((sum / 4) as u8);
                    }
                }
            }
            start = next;
            across = half_across;
            down = half_down;
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
        // Told here because here is where the bytes are made, and a moment
        // from now the only copy of them is in the driver. Charged to whatever
        // the caller said it was drawing — see [`MemoryBill::charge`].
        MemoryBill::sheet(&image);
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

        // Mip-chained, for the reason [`Self::mipped_netting`] is: the ball is
        // the one object on the pitch the eye tracks, and it is also the
        // SMALLEST — 22 cm across at up to a hundred and fifty metres, which is
        // a couple of pixels wrapped in a sheet a hundred and twenty-eight
        // texels wide. Undersampled that badly, a panel either lands under the
        // sample point or it does not, so the ball flickers between white and
        // black from frame to frame as it flies. The chain converges to the
        // leather and the panels averaged in their true proportion, which is a
        // light grey — and a light grey is exactly what a football looks like
        // from the far end of a ground.
        images.add(Self::mipped(WIDTH, HEIGHT, data))
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

        // Chained. Two hundred and fifty-six seats across a bank two hundred
        // metres wide is a texture that is at about one texel to the pixel from
        // the broadcast gantry and nowhere else: the rig flies (`CameraFlight`
        // lets it to 130 m) and orbits to behind a goal, and from either the
        // far bank is minified several times over. Unchained, the seats there
        // do not blur — they SWIM, resampling to a different set of them on
        // every frame the camera moves, and the banks are the largest thing in
        // frame after the pitch. Which is the whole of what "the camera does
        // not move smoothly" is made of, as much as any frame rate.
        images.add(Self::mipped(WIDTH, HEIGHT, data))
    }


    /// Every colour a spectator is painted in, as one strip of flat swatches.
    ///
    /// **A palette rather than a set of materials, and a texture rather than
    /// vertex colours**, for two separate reasons and both of them measured.
    ///
    /// A material each would put a draw call behind every shade in the ground,
    /// and this viewer's frame is spent per ENTITY and not per pixel — see
    /// [`crate::app::perf`], where the same scene costs the same 3.9 ms at
    /// 720p and at 4K. The crowd is one mesh per bank precisely so it does
    /// not, so a spectator's colour cannot live on his material.
    ///
    /// Vertex colours would carry it for nothing — except that a mesh with
    /// `ATTRIBUTE_COLOR` on it is a different vertex layout, and a different
    /// vertex layout is a different render pipeline: a FIFTH PBR shader for
    /// the browser to link, four to six seconds of frozen tab on the first
    /// open of a session (see [`crate::app::bringup`], which has the trace).
    /// A `uv` into a palette costs nothing at all — the stands already carry
    /// one — and the crowd draws through the same program the terracing does.
    ///
    /// **Two rows of tiles.** The bottom is the flat colours — one solid
    /// block each, which is what a torso, a sleeve and the back of a hand are
    /// painted with. The top is the same heads UNWRAPPED: hair, ears, brows, a
    /// pair of eyes, a nose, a mouth, a beard on some and a hat on some, laid
    /// out right round the skull. [`CrowdPalette::head_uv`] is what reads it.
    ///
    /// A face is worth drawing at all because of where a lens ends up. The
    /// near terrace is thirty metres from a pitch-side camera and a head there
    /// is twenty pixels across, which is plenty for two eyes to read as a
    /// person rather than as a bead — and the free camera flies, so the same
    /// head is two hundred pixels across the moment anybody walks it into a
    /// stand. **Sixty-four texels** is set by the second of those: with `u`
    /// stretched toward the front it puts about thirty texels across a face,
    /// an eye four across and a pupil two, which is the coarsest a head can be
    /// drawn at and still hold up at arm's length. It was sixteen, set by the
    /// first alone, and at sixteen a face at three metres is nine enormous
    /// squares.
    ///
    /// **Mip-chained, but only down to one texel per TILE.** The chain is what
    /// makes a drawn face safe: a head six pixels wide against a sixty-four
    /// texel tile is undersampled ten times over, and point-sampled it does
    /// not blur, it SWIMS — resampling to a different pair of eyes every frame
    /// the camera moves, which is the exact failure [`Self::seats`] documents
    /// for the seats behind them. But a chain run to the bottom would average
    /// neighbouring TILES together, and two levels past that the whole sheet
    /// is one brown — a crowd whose colour converges to a single average at
    /// distance is a car park.
    ///
    /// Both are had by stopping the chain where a texel is exactly one tile.
    /// Tiles are a power of two wide and aligned to it, so every box filter
    /// down to that level falls INSIDE a tile and no two tiles ever meet; the
    /// coarsest level is each tile's own average — a face resolving to skin
    /// darkened a little by its own eyes, which is what a face at a hundred
    /// metres is. See [`Self::mipped_tiles`].
    ///
    /// Heads first, then clothing. `home` is the shirt the ground's own
    /// support turns up in, so it is dealt several tiles rather than one: a
    /// real stand is mostly coats, with the club's colour running through it
    /// in enough quantity to tell you whose ground you are at.
    pub fn crowd(
        images: &mut Assets<Image>,
        home: Color,
        trim: Color,
        visitor: Color,
        visitor_trim: Color,
    ) -> CrowdPalette {
        /// Texels across one tile, and so down one row. See the note above for
        /// why sixty-four and not sixteen.
        const TILE: u32 = 64;
        /// **What a crowd wears over whatever else it is wearing**: winter
        /// coats, in the greys, greens, dark blues and browns people actually
        /// own. This is most of a ground and all of the main stands.
        ///
        /// Deliberately low in saturation. The club's colours below have to be
        /// the only bright thing in a stand or the eye reads the crowd as
        /// bunting — and, more to the point, a fan end only reads as one
        /// because the seats either side of it do NOT.
        ///
        /// Sixteen of them, and the variety that pays is in the VALUE: what
        /// tells one spectator from his neighbour at a hundred metres is light
        /// against dark, and a coat that is a third grey rather than a quarter
        /// grey is a coat nobody will ever resolve. So the run goes from
        /// near-black to pale and only wanders off the greys where a real
        /// wardrobe does — olive, navy, brown, burgundy.
        ///
        /// It was ten, which was enough while a spectator was a rectangle and
        /// the whole of him was one of these. He is a body, two sleeves and a
        /// pair of hands now, all cut from the same tile, so the coat carries
        /// more of the figure than it used to and a repeat is correspondingly
        /// easier to see.
        const COATS: [Color; 16] = [
            Color::srgb(0.145, 0.160, 0.205),
            Color::srgb(0.235, 0.235, 0.245),
            Color::srgb(0.355, 0.345, 0.330),
            Color::srgb(0.185, 0.150, 0.125),
            Color::srgb(0.300, 0.245, 0.195),
            Color::srgb(0.115, 0.130, 0.155),
            Color::srgb(0.430, 0.415, 0.385),
            Color::srgb(0.170, 0.220, 0.165),
            Color::srgb(0.560, 0.545, 0.520),
            Color::srgb(0.135, 0.175, 0.245),
            Color::srgb(0.085, 0.090, 0.100),
            Color::srgb(0.265, 0.180, 0.180),
            Color::srgb(0.215, 0.240, 0.255),
            Color::srgb(0.400, 0.330, 0.245),
            Color::srgb(0.495, 0.500, 0.535),
            Color::srgb(0.230, 0.205, 0.150),
        ];
        /// How many different people are in the stand.
        ///
        /// Twenty-four, against twelve complexions, ten hair colours and eight
        /// irises, so the pairings do not repeat before any of the ramps does.
        /// It is a small number for a crowd of twenty thousand and it does not
        /// need to be a big one: what tells two spectators apart at any
        /// distance this is seen from is the coat, and there are sixteen of
        /// those over the top of these — the same reasoning as [`Beard`] on
        /// the pitch, where four beards do the work of a hundred faces.
        ///
        /// It was fourteen. What bought the other ten was the tile: at sixteen
        /// texels the heads differed by a complexion and a cap of hair and a
        /// fifteenth would have been a repeat of the second, and at sixty-four
        /// there is a beard, a hat, a bald crown and an iris in each of them
        /// as well — enough combinations that the run is now short of the
        /// variety rather than past it.
        const HEADS: usize = 24;
        /// What a share of them came in, over the hair: a woollen hat in the
        /// colours or in whatever else, which is most of what a winter crowd
        /// has on its head.
        const HATS: [Color; 4] = [
            Color::srgb(0.120, 0.130, 0.150),
            Color::srgb(0.330, 0.150, 0.140),
            Color::srgb(0.480, 0.455, 0.420),
            Color::srgb(0.150, 0.230, 0.330),
        ];

        // Complexions and hair off the SHARED ramps — the same tables every
        // player on the pitch is painted from, so a crowd is made of the same
        // people the teams are rather than out of a second set of colours that
        // could drift away from the first.
        let coat = |colour: Color| {
            let entry = colour.to_srgba();
            Vec3::new(entry.red, entry.green, entry.blue)
        };
        let ramp = |table: &[&str], index: usize, fallback: Vec3| -> Vec3 {
            table
                .get(index)
                .and_then(|hex| Srgba::hex(hex).ok())
                .map(|entry| Vec3::new(entry.red, entry.green, entry.blue))
                .unwrap_or(fallback)
        };
        let faces: Vec<Face> = (0..HEADS)
            .map(|person| Face {
                // Spread across the whole complexion ramp rather than taken
                // off the front of it: a stand drawn from the first six
                // entries is a stand of one ethnicity.
                skin: ramp(
                    &Palette::SKIN,
                    person * Palette::SKIN.len() / HEADS,
                    Vec3::new(0.78, 0.62, 0.48),
                ),
                // A different stride through each of the other two ramps, so
                // complexion, hair and eyes are not one draw wearing three
                // hats — the trap `Complexion::face` documents on the pitch,
                // where a correlated pair gives a whole stand one look.
                hair: ramp(
                    &Palette::HAIR,
                    person * 7 % Palette::HAIR.len(),
                    Vec3::new(0.06, 0.06, 0.06),
                ),
                iris: ramp(
                    &Palette::EYES,
                    person * 5 % Palette::EYES.len(),
                    Vec3::new(0.24, 0.19, 0.14),
                ),
                bald: person % 5 == 0,
                // Clean-shaven, stubble, or the full thing. Stubble is the
                // most common of the three in a real stand and it is also the
                // one that survives minification best — it moves the average
                // of the whole jaw rather than drawing a shape on it.
                beard: match person % 4 {
                    0 => 0.45,
                    3 => 1.0,
                    _ => 0.0,
                },
                hat: (person % 3 == 1)
                    .then(|| coat(HATS[person / 3 % HATS.len()])),
            })
            .collect();

        // A head's tile in the flat row is his HANDS, and nothing else: the
        // head itself is skinned off the drawn row the whole way round now, so
        // this is only ever read by the disc that closes the end of a sleeve.
        // Taken down a tenth, because that is where it is — a hand at the end
        // of an arm hanging at somebody's side is in the shade of his own
        // body, and left at full strength it is the brightest thing in the
        // stand and reads as a badge sewn to his coat.
        let mut flat: Vec<Vec3> = faces.iter().map(|face| face.skin * 0.88).collect();
        let heads = faces.len();
        flat.extend(COATS.iter().copied().map(coat));
        let coats = flat.len() - heads;

        // **A side's colours**, as six tiles rather than one.
        //
        // An end painted in a single colour is a flat sheet of it, which is
        // the one thing an end is not: the same red is a replica two seasons
        // old, a scarf, a bobble hat and a coat somebody bought in the shop
        // last week, and what carries that at a hundred metres is the VALUE
        // running through it. So the shirt is dealt at three strengths — as
        // printed, worn-in, and caught by the light — and the trim is what an
        // end's stripes and scarves are.
        //
        // **Weighted rather than one tile each.** The mesh picks a tile evenly,
        // so the run has to carry the weights itself: three parts the shirt as
        // it comes, one each of the two shades, one of the trim. Dealt evenly,
        // a quarter of an away end turned up in the SECOND colour — a block of
        // white where a block of green belonged, which is not what a travelling
        // support looks like from any distance.
        let shirts = |kit: Color, trim: Color| {
            let kit = coat(kit);
            [
                kit,
                kit,
                kit,
                kit * 0.72,
                kit.lerp(Vec3::ONE, 0.22),
                coat(trim),
            ]
        };
        flat.extend_from_slice(&shirts(home, trim));
        let club = flat.len() - heads - coats;

        // …and the same for the side that travelled. They get a block of one
        // end and nothing else in the ground, which is what an away allocation
        // is — see [`Stature::away_section`](crate::scene::crowd::Stature).
        flat.extend_from_slice(&shirts(visitor, visitor_trim));
        let visiting = flat.len() - heads - coats - club;

        let columns = flat.len() as u32;
        let (width, height) = (columns * TILE, TILE * 2);
        let mut data = vec![0u8; (width * height * 4) as usize];
        let mut put = |x: u32, y: u32, colour: Vec3| {
            let at = ((y * width + x) * 4) as usize;
            data[at] = (colour.x.clamp(0.0, 1.0) * 255.0) as u8;
            data[at + 1] = (colour.y.clamp(0.0, 1.0) * 255.0) as u8;
            data[at + 2] = (colour.z.clamp(0.0, 1.0) * 255.0) as u8;
            data[at + 3] = 255;
        };

        for (column, block) in flat.iter().enumerate() {
            let left = column as u32 * TILE;
            // The drawn face on top, where there is one; the flat colour
            // repeated up there for a coat, which nothing ever samples but
            // which must not be a hole in the mip chain.
            let face = faces.get(column);
            for y in 0..TILE {
                for x in 0..TILE {
                    let painted = match face {
                        Some(face) => face.at(x, y, TILE),
                        None => *block,
                    };
                    put(left + x, y, painted);
                    put(left + x, TILE + y, *block);
                }
            }
        }

        CrowdPalette {
            sheet: images.add(Self::mipped_tiles(width, height, data, TILE)),
            heads,
            coats,
            club,
            visiting,
            tile: TILE as f32,
        }
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
        MemoryBill::sheet(&image);
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
        // one. What moves to meet the picture is the REST of him: his neck, his
        // arms and the cap of hair on his head are all repainted from what this
        // picture says he looks like (see `crate::players::portrait`).
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
    use crate::app::config::PlayerInfo;
    use crate::players::body::BodyParts;
    use crate::players::kit::Complexion;

    /// The crowd's drawn faces share one sheet with the flat colours every
    /// other surface of a spectator is painted in, so the one thing that can
    /// go wrong is a `uv` reaching out of its own tile — and it would not look
    /// like a bug. It would look like every man in the ground wearing a sliver
    /// of the next one's hair, which is exactly the kind of thing that gets
    /// explained away as "the crowd looks a bit muddy".
    #[test]
    fn a_spectators_face_stays_inside_his_own_tile() {
        const HEADS: usize = 24;
        const COATS: usize = 16;
        const CLUB: usize = 6;
        const AWAY: usize = 6;
        let palette = CrowdPalette::of_swatches(HEADS, COATS, CLUB, AWAY);
        let tiles = (HEADS + COATS + CLUB + AWAY) as f32;

        for person in 0..HEADS as u32 {
            let (left, right) = (person as f32 / tiles, (person + 1) as f32 / tiles);
            // Right round the head and from the crown to under the collar —
            // the whole wrap, not just the part a face is drawn on.
            let corners = [-1.0f32, -0.5, 0.0, 0.5, 1.0]
                .into_iter()
                .flat_map(|turn| {
                    [0.0f32, 0.5, 1.0].map(|down| palette.head_uv(person, turn, down))
                });
            for corner in corners {
                assert!(
                    corner.x > left && corner.x < right,
                    "face {person} reaches to {} outside {left}..{right}",
                    corner.x
                );
                // …and stays in the TOP row of the sheet, which is the half
                // the faces are drawn in. The bottom half is flat colour, and
                // a face that dipped into it would be half a portrait and half
                // a paint chip.
                assert!(corner.y > 0.0 && corner.y < 0.5, "face {person} at v {}", corner.y);
            }
            // The nose is the middle of the tile and the back of the head is
            // both its edges — the wrap's whole contract, and the one thing
            // that would put a man's ear where his eye belongs.
            let nose = palette.head_uv(person, 0.0, 0.5).x;
            assert!(
                (nose - (left + right) * 0.5).abs() < 1e-5,
                "face {person} has its nose at {nose}, not at {}",
                (left + right) * 0.5
            );
            let (back_left, back_right) = (
                palette.head_uv(person, -1.0, 0.5).x,
                palette.head_uv(person, 1.0, 0.5).x,
            );
            // Within a texel of each edge — which is the inset, and the whole
            // of what may separate them from it.
            let texel = 1.0 / tiles / 64.0;
            assert!(
                back_left - left < texel && right - back_right < texel,
                "the back of head {person} is at {back_left}..{back_right}, not at {left}..{right}"
            );

            // His flat skin is the SAME tile in the bottom row, sampled dead
            // centre — it is what the back of his hand is painted with, and a
            // texel centre is what keeps the tile unmixed under a linear
            // filter.
            let flat = palette.head(person);
            assert!(flat.y > 0.5, "flat head {person} at v {}", flat.y);
            assert!(
                (flat.x * tiles - (person as f32 + 0.5)).abs() < 1e-4,
                "flat head {person} is not on a texel centre: {}",
                flat.x * tiles
            );
        }

        // Coats follow the heads along the sheet, and the club's colours
        // follow the coats. The three groups are picked from separately — an
        // overlap would put a man in the wrong end's clothes.
        for coat in 0..COATS as u32 {
            let at = palette.coat(coat).x * tiles;
            assert!(
                at > HEADS as f32 && at < (HEADS + COATS) as f32,
                "coat {coat} landed outside the coats at {at}"
            );
        }
        for colours in 0..CLUB as u32 {
            let at = palette.colours(colours).x * tiles;
            assert!(
                at > (HEADS + COATS) as f32,
                "club colour {colours} landed among the coats at {at}"
            );
        }
    }

    /// **The chain has to stop where a texel is one tile.** One level further
    /// and a texel straddles two tiles, the palette stops being a palette, and
    /// a couple of levels past that the whole crowd is one brown.
    ///
    /// It is a silent failure in both directions — too few levels and the
    /// faces swim, too many and the colour drains out — and neither shows up
    /// anywhere except on screen at a distance, so it is pinned here.
    #[test]
    fn the_crowd_sheet_stops_mipping_where_a_texel_is_a_tile() {
        const TILE: u32 = 16;
        const COLUMNS: u32 = 29;
        let (width, height) = (COLUMNS * TILE, TILE * 2);
        let sheet = Textures::mipped_tiles(
            width,
            height,
            vec![128; (width * height * 4) as usize],
            TILE,
        );

        // 464x32 → 232x16 → 116x8 → 58x4 → 29x2, which is one texel per tile.
        assert_eq!(sheet.texture_descriptor.mip_level_count, 5);

        // …and the buffer holds exactly those levels, or wgpu rejects the
        // upload and the whole crowd is untextured.
        let wanted: usize = (0..5)
            .map(|level| {
                let (across, down) = (
                    (width >> level).max(1) as usize,
                    (height >> level).max(1) as usize,
                );
                across * down * 4
            })
            .sum();
        assert_eq!(sheet.data.as_ref().map(Vec::len), Some(wanted));
    }

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

    /// The ring a cut is faded in through has to be a LENS: thin enough in the
    /// middle to see the new episode arrive through it, solid by the corners,
    /// and one smooth ramp between the two. Each of the three is a way of
    /// getting it wrong that looks fine in the source — a flat sheet is a
    /// curtain and reads as a dropped frame, a hard edge reads as a mask, and
    /// a ring that turns back on itself draws a halo.
    #[test]
    fn the_vignette_thins_towards_the_middle_of_the_frame() {
        let mut images = Assets::default();
        let handle = Textures::vignette(&mut images);
        let image = images.get(&handle).expect("the vignette was just added");
        let size = image.width();
        let pixels = image.data.as_ref().expect("pixels");
        let alpha = |x: u32, y: u32| pixels[((y * size + x) * 4 + 3) as usize];

        // The middle of the ring sits above the middle of the frame — see
        // `vignette`'s `EYE` — so that is where the thin part is measured.
        let eye = (size as f32 * 0.45) as u32;
        let middle = alpha(size / 2, eye);
        let side = alpha(size - 1, eye);
        let corner = alpha(size - 1, size - 1);

        assert!(
            (110..=145).contains(&middle),
            "the middle of the dip is {middle}/255: {}",
            if middle < 110 {
                "the cut will not be seen through it"
            } else {
                "the new episode arrives behind a curtain"
            }
        );
        assert!(side > 190, "the edge of the frame is only {side}/255 dark");
        assert!(
            corner > side,
            "the corner ({corner}) is no darker than the edge ({side}) — this is a \
             band, not a lens"
        );

        // One ramp outward from the eye, on both axes, with no step and no
        // turn anywhere in it.
        let mut previous = middle;
        for x in size / 2..size {
            let here = alpha(x, eye);
            assert!(here >= previous, "the ramp turns back at column {x}");
            assert!(here - previous < 8, "the ramp steps at column {x}");
            previous = here;
        }
        let mut previous = middle;
        for y in eye..size {
            let here = alpha(size / 2, y);
            assert!(here >= previous, "the ramp turns back at row {y}");
            previous = here;
        }

        // White throughout: the colour of the dip belongs to the node, which
        // is what lets one float a frame carry the whole fade.
        assert!(
            pixels.chunks(4).all(|texel| texel[..3] == [255, 255, 255]),
            "the vignette carries a colour of its own"
        );
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
                    starting: true,
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

    /// One printed perimeter panel, addressed by column and row.
    struct Board {
        panel: Vec<u8>,
    }

    impl Board {
        /// The panel `Textures::hoarding` builds, at the size it builds it and
        /// carrying the strings the ground actually carries.
        const WIDTH: u32 = 1024;
        const HEIGHT: u32 = 128;

        fn print() -> Self {
            Self {
                panel: Textures::advert(
                    Self::WIDTH,
                    Self::HEIGHT,
                    "OF",
                    "OpenFootball",
                    "open-football.org",
                ),
            }
        }

        fn texel(&self, x: u32, y: u32) -> [u8; 3] {
            let at = ((y * Self::WIDTH + x) * 4) as usize;
            [self.panel[at], self.panel[at + 1], self.panel[at + 2]]
        }

        /// The first and last column this colour is printed in, or `None` if it
        /// is nowhere on the board. Matched with a little slack: the interior
        /// of a shape is exactly its layer's colour, but only after a round
        /// trip through `over` and back to a byte.
        fn columns(&self, colour: [u8; 3]) -> Option<(u32, u32)> {
            let mut found: Option<(u32, u32)> = None;
            for x in 0..Self::WIDTH {
                for y in 0..Self::HEIGHT {
                    let texel = self.texel(x, y);
                    if (0..3).all(|channel| texel[channel].abs_diff(colour[channel]) <= 2) {
                        found = Some(match found {
                            Some((first, _)) => (first, x),
                            None => (x, x),
                        });
                        break;
                    }
                }
            }
            found
        }
    }

    /// The lockup has to fit INSIDE its panel with air to spare. A board is one
    /// texture repeated along a hundred metres of touchline, so a lockup that
    /// runs to the edge is not clipped — it runs straight into the next copy of
    /// itself, and the whole perimeter reads as one unbroken sentence instead
    /// of as a panel that repeats. The check is the margin band: the outermost
    /// columns must be the board's own colour and nothing else.
    #[test]
    fn the_advert_leaves_air_between_one_panel_and_the_next() {
        /// Comfortably inside `advert`'s own margin, so this pins the air
        /// rather than restating the constant.
        const BAND: u32 = Board::WIDTH / 24;

        let board = Board::print();
        let ground = board.texel(0, 0);
        for x in 0..BAND {
            for y in 0..Board::HEIGHT {
                assert_eq!(
                    board.texel(x, y),
                    ground,
                    "the lockup reaches the left edge of the panel at {x},{y}"
                );
                let mirrored = Board::WIDTH - 1 - x;
                assert_eq!(
                    board.texel(mirrored, y),
                    ground,
                    "the lockup reaches the right edge of the panel at {mirrored},{y}"
                );
            }
        }
    }

    /// **The walk-out plate is the strip inverted, and it is a plate.**
    ///
    /// Three things can go wrong with it and none of them would look like a
    /// bug from four metres: the two colours could come out the same way round
    /// as the back print, in which case the plate is invisible on the shirt;
    /// the rounded rectangle could fail to fill, in which case the ceremony
    /// draws bare lettering again and nobody notices which of the two panels
    /// they are looking at; and it could grow past the panel it is printed on,
    /// which cuts the plate off with a hard edge along the seam. So the test
    /// takes the picture apart and asks for all three.
    #[test]
    fn the_walk_out_name_is_set_on_a_plate_of_the_print_colour() {
        let mut images = Assets::<Image>::default();
        let shirt = Color::srgb(0.78, 0.08, 0.10);
        let print = Color::WHITE;
        let handle = Textures::name_plate(&mut images, "Petrov", print, shirt)
            .expect("a printable name is printable");
        let plate = images.get(&handle).expect("the plate was added");
        let (width, height) = (plate.width(), plate.height());
        let texels = plate.data.as_ref().expect("the plate carries pixels");
        let texel = |x: u32, y: u32| {
            let at = ((y * width + x) * 4) as usize;
            [texels[at], texels[at + 1], texels[at + 2], texels[at + 3]]
        };

        // Every corner of the panel is off the plate — the corners are turned,
        // and the plate does not reach the edge in any case.
        for (x, y) in [
            (0, 0),
            (width - 1, 0),
            (0, height - 1),
            (width - 1, height - 1),
        ] {
            assert_eq!(texel(x, y)[3], 0, "the plate reaches the corner at {x},{y}");
        }
        // The middle of it is opaque, and it is the PRINT colour rather than
        // the shirt's — which is the inversion, and the whole point.
        let middle = texel(width / 2, 2 + (height - 4) / 8);
        assert_eq!(middle[3], 255, "the plate has a hole in the middle of it");
        assert!(
            middle[0] > 200 && middle[1] > 200 && middle[2] > 200,
            "the plate is not the print colour: {middle:?}"
        );
        // And somewhere inside it there is lettering in the shirt's own colour.
        let inked = (0..width * height)
            .map(|index| texel(index % width, index / width))
            .filter(|texel| texel[3] > 128 && texel[0] > 150 && texel[1] < 80 && texel[2] < 80)
            .count();
        assert!(
            inked > 40,
            "only {inked} texels of the plate are set in the shirt colour"
        );
    }

    /// A plate has to fit the panel it is printed on whatever is written on it:
    /// a name long enough to fill the row still has to leave the rounded corner
    /// somewhere the shirt can show it, and a name of two letters must not come
    /// out as a square badge.
    #[test]
    fn a_plate_fits_its_panel_whatever_the_name() {
        let mut images = Assets::<Image>::default();
        for name in ["Li", "Petrov", "Vandenbroucke", "Papastathopoulos"] {
            let handle =
                Textures::name_plate(&mut images, name, Color::WHITE, Color::BLACK).unwrap();
            let plate = images.get(&handle).unwrap();
            let (width, height) = (plate.width(), plate.height());
            let texels = plate.data.as_ref().unwrap();
            let (mut left, mut right) = (width as i64, -1i64);
            for index in 0..width * height {
                if texels[(index * 4 + 3) as usize] > 128 {
                    left = left.min((index % width) as i64);
                    right = right.max((index % width) as i64);
                }
            }
            assert!(left >= 1, "{name}'s plate runs off the left of the panel");
            assert!(
                right <= width as i64 - 2,
                "{name}'s plate runs off the right of the panel"
            );
            // Wider than it is tall, and by enough to read as a badge rather
            // than as a box.
            let across = (right - left + 1) as f32;
            assert!(
                across > height as f32 * 1.4,
                "{name}'s plate is {across} by {height}, which is a square"
            );
        }
        // Nothing to print, nothing printed — the walk-out then shows the same
        // bare chest the back of the shirt shows.
        assert!(Textures::name_plate(&mut images, "のぞみ", Color::WHITE, Color::BLACK).is_none());
    }

    /// And all three pieces have to be on it, in the order the lockup is
    /// written in: mark, name, address. Each is drawn into a layer of its own
    /// and composited by colour, so a piece that quietly fails to set — a face
    /// that will not load, a scale that comes out zero, a baseline off the
    /// bottom of the panel — leaves a board that still looks like a board and
    /// is missing a third of itself.
    #[test]
    fn the_advert_carries_a_mark_a_name_and_an_address() {
        // The three layer colours, as `advert` mixes them at full coverage.
        let tile = [19, 119, 150];
        let letters = [242, 244, 252];
        let accent = [107, 198, 224];

        let board = Board::print();
        let (mark_from, _) = board
            .columns(tile)
            .expect("the logo tile is not on the board");
        let (name_from, name_to) = board
            .columns(letters)
            .expect("nothing is printed in white on the board");
        let (_, address_to) = board
            .columns(accent)
            .expect("neither the divider nor the address is on the board");

        assert!(
            mark_from < name_from,
            "the mark starts at {mark_from} and the lettering at {name_from}: the logo is not \
             leading the lockup"
        );
        assert!(
            address_to > name_to,
            "the white lettering ends at {name_to} and the accent at {address_to}: the address \
             is not closing the lockup"
        );
    }

    /// A perimeter board is a piece of graphic design, and the only questions
    /// worth asking about one — does the lockup hold together, is the address
    /// still readable at the size the wordmark leaves it — are questions for an
    /// eye. The two tests above pin the arithmetic; this is how the picture
    /// gets looked at.
    ///
    /// ```text
    /// MATCH_AD_DUMP=<dir> cargo test --lib dump_hoarding -- --ignored --nocapture
    /// ```
    ///
    /// Writes `hoarding.rgba` — three panels side by side, which is the only
    /// way to see whether the air between one repeat and the next is doing its
    /// job — with its dimensions on stdout, for whatever turns raw pixels into
    /// a picture.
    #[test]
    #[ignore = "writes a file; run by hand when the board changes"]
    fn dump_hoarding() {
        const REPEATS: usize = 3;

        let Ok(directory) = std::env::var("MATCH_AD_DUMP") else {
            panic!("set MATCH_AD_DUMP to a directory");
        };

        let board = Board::print();
        let across = Board::WIDTH as usize * REPEATS;
        let mut sheet = vec![0u8; across * Board::HEIGHT as usize * 4];
        for repeat in 0..REPEATS {
            for row in 0..Board::HEIGHT as usize {
                let from = row * Board::WIDTH as usize * 4;
                let to = (row * across + repeat * Board::WIDTH as usize) * 4;
                sheet[to..to + Board::WIDTH as usize * 4]
                    .copy_from_slice(&board.panel[from..from + Board::WIDTH as usize * 4]);
            }
        }

        let path = std::path::Path::new(&directory).join("hoarding.rgba");
        std::fs::write(&path, &sheet).expect("wrote the sheet");
        println!("{}x{} at {}", across, Board::HEIGHT, path.display());
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
        let shade = crate::scene::pitch::Pitch::MOWN;
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

