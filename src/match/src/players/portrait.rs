//! Real faces on the pitch: the picture off a player's profile page, fetched
//! while the match is already running and laid over the head the viewer drew
//! for him.
//!
//! The page decides WHICH picture, exactly as it does everywhere else on the
//! site — a photograph for a real footballer, the drawn portrait for a regen —
//! and hands both URLs over in the config. This module does the rest: fetch,
//! decode, cut the background off, and repaint one face sheet.
//!
//! Nothing here is on the critical path. A player takes the field with his own
//! complexion on a bare head and this puts a face on it a few frames later; a
//! picture that 404s, that the browser refuses to read across an origin, or
//! that never comes back at all simply leaves him as he is. Which is the only
//! way to load a face over a network into a match that has already kicked off.
//!
//! **A face here is a PICTURE OF THE MAN or it is nothing.** The viewer can
//! draw a face — eyes, brows, a jaw, a beard, all off his id — and for a while
//! it did, as the thing to look at until his photograph landed. It is not a
//! lesser version of a photograph, though; it is a different thing, and a
//! squad wearing a mix of the two reads as half the men being somebody and
//! half being nobody. The page has a picture for everybody — a photograph for
//! a real footballer, the portrait it draws for a regen — so the two sources
//! below are what a head gets, in that order, and there is no third.

use crate::app::config::{PlayerInfo, ViewerConfig};
use crate::art::textures::{Portrait, Textures};
use crate::players::actors::PlayerActor;
use crate::players::body::{BodyParts, DressedFlesh, Flesh, Grain, Thatch};
use crate::players::kit::{Complexion, Wardrobe};
use bevy::prelude::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{Blob, CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement, Response, Url};

/// Where a face sits in one kind of picture, and what has to be done to it
/// before it is one.
///
/// Two of these exist, and they are measured off the two things the site
/// actually serves. Neither is guessed at run time: a head shot is a studio
/// frame with the same head in the same place every time, and the drawn
/// portrait is drawn by us, so its landmarks are known exactly.
#[derive(Clone, Copy)]
struct Framing {
    /// The part of the picture worth having, as fractions of the whole. A
    /// club head shot is mostly card and shoulders.
    crop: (f32, f32, f32, f32),
    /// The eye line and the point of the chin, down the WHOLE picture.
    eyes: f32,
    chin: f32,
    /// Half the width of the face at eye level, across the whole picture —
    /// cheekbone to cheekbone, not ear to ear.
    half_face: f32,
    /// True when the picture comes on a background that has to be cut off it.
    /// A drawn cutout arrives with real transparency and needs nothing.
    keyed: bool,
    /// The size to render at, for a picture that has no size of its own. An
    /// SVG with a `viewBox` and no `width` is one: left to itself the browser
    /// gives it the 300×150 default and the head comes out squashed.
    intrinsic: Option<(u32, u32)>,
}

impl Framing {
    /// A club head shot: 260×310, a man on a light card, shoulders in.
    /// Measured off the library the site serves — the head lands in the same
    /// place in every one of them to within a few pixels, which is what makes
    /// fixed landmarks good enough here and would not be for a bag of
    /// photographs from anywhere.
    const PHOTOGRAPH: Framing = Framing {
        crop: (0.215, 0.03, 0.57, 0.62),
        eyes: 0.310,
        chin: 0.548,
        half_face: 0.135,
        keyed: true,
        intrinsic: None,
    };

    /// The drawn portrait, asked for as a cutout: `viewBox="0 0 200 250"`,
    /// head centred on x=100, eye line at y=118 and the chin at y≈205 — see
    /// `web`'s face generator, which puts them there rather than being
    /// measured for them.
    ///
    /// The width is the exception and IS measured, off rendered cutouts: 40
    /// of the 200 units from the mid-line to the edge of the cheek, and the
    /// ears sit sixteen more out beyond that. The ears are the reason it has
    /// to be measured rather than read off `temple_w` — the projection wants
    /// the width of the SKULL, which is where the head's own silhouette ends,
    /// and a head drawn with its ears on is wider than its skull.
    const DRAWN: Framing = Framing {
        crop: (0.0, 0.0, 1.0, 1.0),
        eyes: 0.472,
        chin: 0.820,
        half_face: 0.200,
        keyed: false,
        intrinsic: Some((200, 250)),
    };

    /// How big a patch to keep, once the head is cut out of the picture.
    ///
    /// Enough that the PICTURE is what limits the face and not this: a studio
    /// head shot carries about 130 pixels across the head it is cropped to,
    /// and a patch of 160 holds all of them with a little to spare. Sized this
    /// way round on purpose — the sheet it is going onto is four times what a
    /// painted face needs (see `Textures::face_sheet`), and there is no point
    /// in a sheet with more texels than the photograph has pixels to fill it.
    const PATCH: u32 = 160;

    /// The landmarks again, now as fractions of the CROP — which is what the
    /// projection is handed, because the crop is all it ever sees.
    fn cropped(&self) -> (f32, f32, f32) {
        let (_, top, width, height) = self.crop;
        (
            (self.eyes - top) / height,
            (self.chin - top) / height,
            self.half_face / width,
        )
    }

    /// Take the head out of a decoded picture: crop to it, bring it down to
    /// the patch, and cut the studio off the back of it.
    ///
    /// The reduction is a box filter over the source rectangle each patch
    /// texel covers, rather than a lift of the nearest pixel. A studio frame
    /// comes down by about half on each axis and a nearest-neighbour reduction
    /// at that ratio drops every other row — which on a face means an eyelash
    /// here, an eyebrow there, and a different set of them for every player.
    fn cut(&self, source: &[u8], width: u32, height: u32) -> Option<Portrait> {
        if width == 0 || height == 0 || source.len() < (width * height * 4) as usize {
            return None;
        }
        let (left, top, across, down) = self.crop;
        // The patch keeps the crop's SHAPE. It used to be square whatever the
        // crop was, which quietly put a different scale on each axis — and
        // everything downstream measures a face in pupil widths and assumes a
        // step across is a step down.
        let (crop_width, crop_height) = (across * width as f32, down * height as f32);
        let tall = ((Self::PATCH as f32 * crop_height / crop_width).round() as u32).clamp(16, 512);
        let x0 = left * width as f32;
        let y0 = top * height as f32;
        let step_x = crop_width / Self::PATCH as f32;
        let step_y = crop_height / tall as f32;

        let mut pixels = Vec::with_capacity((Self::PATCH * tall * 4) as usize);
        for row in 0..tall {
            for column in 0..Self::PATCH {
                let from_x = x0 + column as f32 * step_x;
                let from_y = y0 + row as f32 * step_y;
                pixels.extend_from_slice(&Self::box_filter(
                    source,
                    width,
                    height,
                    (from_x, from_y),
                    (step_x, step_y),
                ));
            }
        }

        if self.keyed {
            Portraits::key_out_the_studio(&mut pixels, Self::PATCH, tall);
        }
        // Measured off the picture, with this framing's own numbers standing
        // in only if the measurement finds nothing it believes.
        let found = Landmarks::of(&pixels, Self::PATCH, tall);
        #[cfg(test)]
        if std::env::var("MATCH_PICTURE_DUMP").is_ok() {
            match &found {
                Some(landmarks) => println!(
                    "  measured: centre {:.3} eyes {:.3} pupils {:.1}",
                    landmarks.centre, landmarks.eyes, landmarks.pupils
                ),
                None => println!("  measured: REFUSED — falling back on the framing"),
            }
        }
        let landmarks = found.unwrap_or_else(|| self.assumed(tall));
        Some(Portrait {
            width: Self::PATCH,
            height: tall,
            pixels,
            centre: landmarks.centre,
            eyes: landmarks.eyes,
            pupils: landmarks.pupils,
        })
    }

    /// What this framing says, for a picture the measurement could not read: a
    /// head shot too dark to find eyes in, a cutout that came back blank.
    ///
    /// The old behaviour, in other words, kept as the floor rather than as the
    /// rule. It is right on average and wrong on any picture framed unlike the
    /// rest, which is exactly the case it now only has to cover when nothing
    /// better can be had.
    fn assumed(&self, tall: u32) -> Landmarks {
        let (eyes, _, half_face) = self.cropped();
        Landmarks {
            centre: 0.5,
            eyes,
            // A face is a bit over two pupil widths across, half of it a bit
            // over one — so the framing's half-face, doubled and scaled by
            // the ratio a head keeps between the two.
            pupils: half_face * Self::PATCH as f32 * 1.22,
        }
        .clamped(Self::PATCH, tall)
    }

    /// The mean of one source rectangle. Colour is weighted by opacity, so the
    /// transparent ground a drawn cutout stands on cannot darken the edge of
    /// the hair standing on it.
    fn box_filter(
        source: &[u8],
        width: u32,
        height: u32,
        (from_x, from_y): (f32, f32),
        (step_x, step_y): (f32, f32),
    ) -> [u8; 4] {
        let x0 = (from_x.floor().max(0.0) as u32).min(width - 1);
        let y0 = (from_y.floor().max(0.0) as u32).min(height - 1);
        let x1 = ((from_x + step_x).ceil().max(0.0) as u32).clamp(x0 + 1, width);
        let y1 = ((from_y + step_y).ceil().max(0.0) as u32).clamp(y0 + 1, height);

        let (mut colour, mut alpha, mut count) = ([0.0f32; 3], 0.0f32, 0.0f32);
        for y in y0..y1 {
            for x in x0..x1 {
                let at = ((y * width + x) * 4) as usize;
                let opacity = source[at + 3] as f32 / 255.0;
                for channel in 0..3 {
                    colour[channel] += source[at + channel] as f32 * opacity;
                }
                alpha += opacity;
                count += 1.0;
            }
        }
        if alpha <= 0.0 {
            return [0, 0, 0, 0];
        }
        [
            (colour[0] / alpha) as u8,
            (colour[1] / alpha) as u8,
            (colour[2] / alpha) as u8,
            (alpha / count * 255.0) as u8,
        ]
    }
}

/// Where a man's eyes are in a picture of him: the mid-line, the eye line and
/// the distance between his pupils, all in the patch's own pixels.
///
/// Three numbers, and the third is the important one. Interpupillary distance
/// is 63 mm on nearly everybody, so a picture that knows how many pixels lie
/// between a man's pupils knows how big every other feature on his face is —
/// which is what lets a photograph framed close and a photograph framed wide
/// go onto the same skull at the same size, and neither of them stretched.
struct Landmarks {
    centre: f32,
    eyes: f32,
    pupils: f32,
}

impl Landmarks {
    /// How far down from the top of the head to start looking for eyes, and
    /// how far to keep looking, as multiples of the head's WIDTH.
    ///
    /// A head is about a third again as tall as it is wide and the eyes sit
    /// near the middle of it, which puts them about two thirds of a width
    /// below the crown; the band either side of that is how much hair moves
    /// the crown. Kept as tight as the hair allows on purpose — every row of
    /// it that is really scalp is a row where a shadow in a blond man's
    /// fringe can outscore his own pale eyes, and a face pinned to a fringe
    /// is a photograph hung a hand's breadth up the skull.
    const SEARCH: (f32, f32) = (0.45, 0.95);
    /// Pupils, as a fraction of the width of the head they are in.
    ///
    /// 63 mm apart across 150 mm of head, ear to ear — the two measurements
    /// on a human head that vary least, and their ratio varies less still.
    /// This is the whole reason the silhouette is worth measuring: it turns
    /// a shape anyone can find into a scale nobody has to guess — and why the
    /// width it is handed has to be the SKULL's (see [`Silhouette::CRANIUM`])
    /// and not the outline's, which has a pair of ears on it.
    const PUPILS_OF_A_HEAD: f32 = 0.42;
    /// How much darker an eye has to be than the face round it before this
    /// is believed to have found one.
    const CONTRAST: f32 = 0.06;
    /// How far above the eye line the brow is, in pupil widths, and what a
    /// brow found there is worth. Twenty millimetres over the pupils is where
    /// one sits on nearly everybody.
    const BROW_OVER: f32 = 0.30;
    const BROWED: f32 = 0.6;
    /// Where the white of an eye is, out from the middle of it, in pupil
    /// widths.
    ///
    /// An iris is about 11 mm across and the eye round it about 30, so seven
    /// to twelve millimetres out from the middle of one is sclera on both
    /// sides whatever a man is doing with his face, short of shutting it.
    /// Three of them rather than one because the eye this is looking for may
    /// be a couple of millimetres from where the seeded pupil distance said
    /// it would be, and the brightest of the three is the white.
    const SCLERA: [f32; 3] = [0.11, 0.15, 0.19];
    /// …and the same for the iris in the middle, where the DARKEST of them is
    /// the one wanted, for the same reason.
    const IRIS: [f32; 5] = [-0.07, -0.035, 0.0, 0.035, 0.07];
    /// What that comparison is worth against the other two. Weighted up
    /// because it is the only one of the three that a BROW cannot also
    /// satisfy, and because a pale-eyed man under studio light gives it less
    /// to work with than a dark-eyed one gives the rest.
    const WHITES: f32 = 2.0;

    /// Measure a keyed patch, or admit it cannot.
    ///
    /// Two measurements and one piece of anatomy. The SILHOUETTE gives the
    /// head's width and mid-line, which is the sturdiest thing in any picture
    /// of a head — it survives shadow, beards, bad white balance and a man
    /// squinting. Anatomy turns that width into a pupil distance, because the
    /// ratio between the two barely varies from person to person: 63 mm of
    /// pupils across 150 mm of head — and that is where the search for the
    /// eyes themselves STARTS, since a window to look in is all a ratio can
    /// honestly give. The EYE LINE, which anatomy cannot give at all (hair
    /// moves the top of a head and nothing else does), is looked for in the
    /// picture from the first.
    fn of(pixels: &[u8], width: u32, height: u32) -> Option<Landmarks> {
        let head = Silhouette::of(pixels, width, height);
        #[cfg(test)]
        if std::env::var("MATCH_PICTURE_DUMP").is_ok() {
            match &head {
                Some(head) => println!(
                    "  silhouette: crown {} widest {} centre {:.1}",
                    head.crown, head.widest, head.centre
                ),
                None => println!("  silhouette: none found"),
            }
        }
        let head = head?;
        let span = head.widest as f32;
        let pupils = span * Self::PUPILS_OF_A_HEAD;

        let luminance = |x: f32, y: f32| -> Option<f32> {
            let (x, y) = (x.round(), y.round());
            if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
                return None;
            }
            let at = ((y as u32 * width + x as u32) * 4) as usize;
            (pixels[at + 3] > 128).then(|| {
                (pixels[at] as f32 * 0.2126
                    + pixels[at + 1] as f32 * 0.7152
                    + pixels[at + 2] as f32 * 0.0722)
                    / 255.0
            })
        };
        // The mean over a little block, so a single dark texel is not an eye.
        let patch = |centre_x: f32, centre_y: f32, half: f32| -> Option<f32> {
            let (mut total, mut count) = (0.0, 0.0);
            let step = (half / 2.0).max(0.5);
            let mut y = centre_y - half * 0.6;
            while y <= centre_y + half * 0.6 {
                let mut x = centre_x - half;
                while x <= centre_x + half {
                    if let Some(value) = luminance(x, y) {
                        total += value;
                        count += 1.0;
                    }
                    x += step;
                }
                y += step.max(1.0);
            }
            (count > 3.0).then(|| total / count)
        };

        // The eye line, looked for with the spacing already known — which is
        // what makes this a search down one axis instead of over two, and the
        // difference between finding a face and finding the darkest thing in
        // the picture.
        let (from, to) = Self::SEARCH;
        let window = (pupils * 0.11).max(1.0);
        // What is left of an eye once the iris is taken out of it: two wedges
        // of sclera, the brightest thing on a face, seven to twelve
        // millimetres either side of the middle. That pattern — dark, bright
        // on BOTH sides, at that spacing — is the one mark nothing else on a
        // head carries, and it is what tells an eye from the BROW over it: a
        // brow is dark for its whole length, so a sample stepped along one is
        // dark either side too and this comes out at nothing.
        //
        // Both ends are scanned rather than sampled, because where this is
        // told to look is only ever as good as the pupil distance it was
        // seeded with — a couple of millimetres out and a fixed sample for
        // the iris lands on the white of the eye instead.
        let iris = (pupils * 0.05).max(1.0);
        let whites = |row: f32, at: f32| -> Option<f32> {
            let middle = Self::IRIS
                .iter()
                .filter_map(|step| patch(at + pupils * step, row, iris))
                .fold(f32::INFINITY, f32::min);
            let side = |sign: f32| {
                Self::SCLERA
                    .iter()
                    .filter_map(|out| patch(at + sign * pupils * out, row, iris))
                    .fold(f32::NEG_INFINITY, f32::max)
            };
            let white = side(-1.0).min(side(1.0));
            (middle.is_finite() && white.is_finite()).then_some(white - middle)
        };
        let mut best: Option<(f32, f32)> = None;
        let mut row = head.crown as f32 + span * from;
        while row <= head.crown as f32 + span * to && row < height as f32 {
            let across = (head.centre - pupils * 0.5, head.centre + pupils * 0.5);
            let eyes = (patch(across.0, row, window), patch(across.1, row, window));
            let bridge = patch(head.centre, row, pupils * 0.12);
            let cheeks = (
                patch(across.0, row + pupils * 0.55, window),
                patch(across.1, row + pupils * 0.55, window),
            );
            let brows = (
                patch(across.0, row - pupils * Self::BROW_OVER, window),
                patch(across.1, row - pupils * Self::BROW_OVER, window),
            );
            if let (
                (Some(left), Some(right)),
                Some(bridge),
                (Some(under_left), Some(under_right)),
                (Some(over_left), Some(over_right)),
            ) = (eyes, bridge, cheeks, brows)
                && let (Some(white_left), Some(white_right)) =
                    (whites(row, across.0), whites(row, across.1))
            {
                // What an eye is, put as four comparisons.
                //
                // WHITES either side of it, which is the one mark nothing
                // else on a head carries. Darker than the BRIDGE of the nose
                // between the pair, which tells a face from a fringe, hair
                // being dark all the way across. Darker than the CHEEK
                // beneath it, which keeps the search off the hair at the
                // temples. And with something dark of its own just ABOVE it,
                // which is the brow — the one thing a brow does not have over
                // itself, and the term that decides between the two rows when
                // the whites cannot. It is sampled over the EYES and not down
                // the middle of the face: the middle of a forehead and the
                // middle of a brow-line are both bare skin on most men, so
                // asked there it answers the same for both rows and settles
                // nothing. Without it a heavy pair of eyebrows wins the row
                // two centimetres above the eyes, and the face goes onto the
                // skull that far up.
                let eye = left.max(right);
                let under = under_left.min(under_right);
                let over = over_left.min(over_right);
                let score =
                    white_left.min(white_right) * Self::WHITES + (bridge - eye) + (under - eye)
                        - (over - eye) * Self::BROWED;
                #[cfg(test)]
                if std::env::var("MATCH_PICTURE_ROWS").is_ok() {
                    println!(
                        "    row {row:.0} score {score:.3} whites {:.3} bridge {:.3} under {:.3} over {:.3} eye {eye:.3}",
                        white_left.min(white_right),
                        bridge - eye,
                        under - eye,
                        over - eye
                    );
                }
                if best.is_none_or(|(_, found)| score > found) {
                    best = Some((row, score));
                }
            }
            row += 1.0;
        }

        let (row, score) = best?;
        // A face has real contrast between an eye and the skin round it.
        // Anything flatter than this is a picture the measurement has no
        // business reading, and the framing's own numbers are the better bet.
        #[cfg(test)]
        if std::env::var("MATCH_PICTURE_DUMP").is_ok() {
            println!(
                "  eye band: row {row:.0} score {score:.3} (needs {:.3})",
                Self::CONTRAST
            );
        }
        if score < Self::CONTRAST {
            return None;
        }

        Some(Landmarks {
            centre: head.centre / width as f32,
            // The row itself, with nothing added to it. It used to be nudged
            // down by a tenth of a pupil width, because what the search
            // actually found half the time was the BROW-AND-EYE band and the
            // brow was as often the winner — two dark marks with a lit bridge
            // between them describes an eyebrow as exactly as it describes an
            // eye. The bias split the difference and was wrong for both. What
            // replaced it is the pair of comparisons that tell the two apart
            // outright: the WHITES either side of an iris, and the brow that
            // stands over an eye and over nothing else.
            eyes: row / height as f32,
            pupils,
        })
        .map(|found| found.clamped(width, height))
    }

    /// Nothing measured or assumed may put the face off the picture.
    fn clamped(self, width: u32, height: u32) -> Landmarks {
        Landmarks {
            centre: self.centre.clamp(0.3, 0.7),
            eyes: self.eyes.clamp(0.1, 0.8),
            pupils: self
                .pupils
                .clamp(width as f32 * 0.08, width as f32 * 0.45)
                .min(height as f32 * 0.45),
        }
    }
}

/// The outline of the head in a keyed patch: where it starts, how wide it
/// gets, and where its middle is.
struct Silhouette {
    crown: u32,
    widest: u32,
    centre: f32,
}

impl Silhouette {
    /// The band of the outline that is CRANIUM and nothing else, measured
    /// down from the crown in multiples of the head's gross width.
    ///
    /// There is a band at all because of the EARS. The widest row of a head
    /// shot is almost never the skull: a pair of them stands an eighth of the
    /// head's width out past it, and a maximum taken over the whole outline
    /// hands everything downstream a head an eighth too wide — which becomes
    /// a pupil distance an eighth too large, a picture laid down an eighth
    /// too small, and a photograph sitting in the middle of a skull like a
    /// mask on it. Ears start about six tenths of the way down a head and a
    /// head is about a third again as tall as it is wide, so everything
    /// within 0.58 of a width below the crown is skull. The top 0.18 is left
    /// out at the other end, where the outline is still curving over.
    const CRANIUM: (f32, f32) = (0.18, 0.58);

    fn of(pixels: &[u8], width: u32, height: u32) -> Option<Silhouette> {
        let mut spans: Vec<Option<(u32, u32)>> = Vec::with_capacity(height as usize);
        for y in 0..height {
            let mut span: Option<(u32, u32)> = None;
            for x in 0..width {
                if pixels[((y * width + x) * 4 + 3) as usize] > 128 {
                    span = Some(match span {
                        Some((first, _)) => (first, x),
                        None => (x, x),
                    });
                }
            }
            spans.push(span);
        }

        // The crown is the first row with real width in it — a stray texel
        // left by the key is not the top of a head.
        let crown = spans
            .iter()
            .position(|span| span.is_some_and(|(first, last)| last - first > width / 6))?
            as u32;
        // A first pass over the top two thirds of what is left — below that a
        // picture has shoulders in it, and shoulders are wider than any head.
        // This one is only ever a RULER for the band below: it takes the ears
        // in and is that much too wide, and all the band wants of it is
        // roughly how big this head is.
        let below = (crown + (height - crown) * 2 / 3).min(height);
        let gross = spans[crown as usize..below as usize]
            .iter()
            .flatten()
            .map(|(first, last)| last - first)
            .max()? as f32;
        // …and the measurement itself, over the skull alone.
        let band = |reach: f32| (crown + (gross * reach) as u32).min(height.saturating_sub(1));
        let (from, to) = (band(Self::CRANIUM.0), band(Self::CRANIUM.1).max(crown + 2));
        let cranium = spans.get(from as usize..to as usize)?;
        // A head that runs off the side of the crop has not been measured,
        // it has been cut to fit — and every number taken off it afterwards
        // is the crop's rather than the man's. It happens on the one head in
        // a squad with real volume of hair, and it hands the projection a
        // skull half again too wide, which lays his face on at two thirds the
        // size it should be. Better the framing's own numbers, which at least
        // describe a head.
        if cranium
            .iter()
            .flatten()
            .any(|(first, last)| *first == 0 || *last >= width - 1)
        {
            return None;
        }
        let widest = cranium
            .iter()
            .flatten()
            .map(|(first, last)| last - first)
            .max()?;
        // …and its mid-line, from the rows around the widest point rather
        // than from the whole picture: a shoulder in frame is rarely centred.
        let middles: Vec<f32> = cranium
            .iter()
            .flatten()
            .filter(|(first, last)| last - first > widest * 3 / 4)
            .map(|(first, last)| (*first + *last) as f32 * 0.5)
            .collect();
        if middles.is_empty() || widest < width / 4 {
            return None;
        }
        Some(Silhouette {
            crown,
            widest,
            centre: middles.iter().sum::<f32>() / middles.len() as f32,
        })
    }
}

/// The face materials, and the mailbox pictures land in.
///
/// Built empty by [`crate::players::actors::Actors::spawn`] and filled a man at
/// a time by [`crate::players::actors::Actors::take_the_field`], which is what
/// dresses him. **That order is the contract**, and it is why the send is not
/// done here for the whole squad at once: [`Self::attach`] repaints the face
/// material and then reaches for the man's `Flesh` and `Thatch` — the limbs
/// to move onto the complexion in the picture, and the cap of hair to take off
/// over it — and those are components on a body. A picture that landed before
/// the body was built would repaint a material nothing was wearing yet and
/// leave him to walk on in the wrong skin under hair he does not have. Asking
/// only once he has been assembled makes that unreachable rather than unlikely:
/// the network cannot answer sooner than the frame the request was made on.
#[derive(Resource)]
pub struct Portraits {
    /// The material each man's picture will be painted into, as he is dressed.
    /// The material rather than the texture, because repainting means handing
    /// the material a NEW sheet: the old one is only ever held by the material
    /// itself, so it goes when it is replaced.
    faces: Vec<(u32, Handle<StandardMaterial>)>,
    /// The shared skin ramp, one material per entry. A picture moves a
    /// player from one entry to another — it never gives him a material of
    /// his own, which would take him out of the batch every player on that
    /// tone is drawn in.
    complexions: Vec<Handle<StandardMaterial>>,
    /// Finished pictures, waiting for a frame to be folded in. Browser
    /// fetches resolve on the JS microtask queue, which has no access to the
    /// ECS — same arrangement as [`crate::recording::loader::ChunkLoader`].
    ///
    /// ⚠ **Bounded, at [`Self::MAILBOX`].** A `Portrait` is a decoded bitmap
    /// and this queue is drained one per frame ([`Self::A_FRAME`]), so nothing
    /// but [`Self::SPACING`] was keeping it shallow — a spacing that exists
    /// for a completely different reason and could be tuned for that reason
    /// tomorrow. See the cap for what it costs to be wrong about that.
    inbox: Arc<Mutex<Vec<(u32, Portrait)>>>,
    /// How finely this squad is cut, which decides how large a face sheet a
    /// photograph is painted onto. See
    /// [`Grain::face`](crate::players::body::Grain::face).
    grain: Grain,
    /// Men who have been dressed and not yet asked about, oldest first. See
    /// [`Self::ask`], which is what takes them off it.
    outbox: VecDeque<(u32, Vec<(String, Framing)>)>,
    /// Real seconds still to run before the next request goes out.
    cooldown: f32,
}

impl Portraits {
    /// **How many arrivals are folded in on one frame.** See [`Self::attach`],
    /// which measures what a face costs and why one is the answer: a photograph
    /// repaints a 256-square sheet and its mip chain, about six milliseconds of
    /// this thread, and a frame in this replay is two and a half.
    const A_FRAME: usize = 1;

    /// **Real seconds between one request for a photograph and the next.**
    ///
    /// ⚠ **Pacing [`Self::attach`] is not enough on its own, and this is the
    /// other half of it.** Cutting a head shot out of its card — keying the
    /// studio off it, bleeding the edge, hunting for the pupils — happens in
    /// the `async` task that fetched it, on the microtask queue, where no
    /// budget kept in the ECS can reach it. Twenty-two requests fired within a
    /// few frames of each other come back from one host within a few
    /// milliseconds of each other, and the browser then runs twenty-two of
    /// those continuations back to back with nothing between them: measured, a
    /// single **114 ms** frame in the middle of the ceremony's pass, with the
    /// folding already paced to one a frame. Answered a quarter of a second
    /// apart, the same squad cost 14–17 ms at its worst.
    ///
    /// A tenth of a second is six frames of clear air at sixty hertz between
    /// one photograph and the next — a decode and a fold are about fifteen
    /// milliseconds of this thread between them — and it puts a whole squad on
    /// the pitch inside two and a half seconds, which is well inside the
    /// walk-out and long before the pass reaches anybody.
    ///
    /// ⚠ It also keeps the pictures **out of the recording's way**. A browser
    /// opens six connections to a host and the pictures come from the same one
    /// the replay does; sent in one burst ahead of the chunks, they queue the
    /// match itself behind them. Measured against a deliberately slow library,
    /// the first chunk landed eleven seconds late.
    ///
    /// It is measured on [`Time<Real>`] rather than in frames because frames
    /// are not a unit of time here: this replay runs at four hundred a second
    /// on the machine it was written on, where twenty-two frames is fifty-five
    /// milliseconds and no spacing at all.
    const SPACING: f32 = 0.10;

    /// **The most decoded pictures the mailbox will hold**, past which the
    /// oldest is dropped and its man keeps the face this crate painted him.
    ///
    /// ⚠ **The spacing above is a MEMORY bound and was only ever documented
    /// as a timing one.** A `Portrait` is a decoded bitmap sitting in the wasm
    /// heap, arrivals are folded in at one a frame, and nothing else stood
    /// between "the network answered faster than the replay drew" and a queue
    /// of them. In the ordinary case it is never more than one or two deep —
    /// a tenth of a second between requests against a frame every few
    /// milliseconds — but the case that matters is the one where frames stop
    /// being every few milliseconds: a phone mid-bring-up, which is exactly
    /// when the whole squad's photographs are in flight and exactly when the
    /// tab is closest to being killed.
    ///
    /// Four, which is more than a healthy session ever reaches and small
    /// enough that being wrong about the pacing costs four bitmaps rather than
    /// twenty-two. Dropping the OLDEST rather than refusing the newest, so a
    /// backlog resolves toward the men currently being dressed.
    const MAILBOX: usize = 4;

    /// An empty mailbox and the ramp to move players along it.
    pub fn waiting(wardrobe: &Wardrobe, grain: Grain) -> Portraits {
        Portraits {
            faces: Vec::new(),
            complexions: wardrobe.complexions(),
            inbox: Arc::new(Mutex::new(Vec::new())),
            outbox: VecDeque::new(),
            cooldown: 0.0,
            grain,
        }
    }

    /// Put one man's picture on the list to be sent for, now that there is a
    /// body to put it on.
    ///
    /// Idempotent, because the thing calling it is a system: a second call for
    /// a player already sent for does nothing rather than starting a second
    /// fetch of the same head.
    ///
    /// ⚠ **The request does not go out here** — see [`Self::ask`], which spaces
    /// them out. A squad is dressed a few men a frame, so doing it here put
    /// twenty-two image requests on the wire inside a fifth of a second.
    pub fn send_for(&mut self, player: &PlayerInfo, face: Handle<StandardMaterial>) {
        if self.faces.iter().any(|(id, _)| *id == player.id) {
            return;
        }
        self.faces.push((player.id, face));

        // In the page's own order: the photograph of him if the game has one,
        // and the portrait it draws for him if it has not — or if the
        // photograph cannot be had, which on a machine serving the game from
        // somewhere other than the picture library is what happens to every
        // one of them.
        let sources: Vec<(String, Framing)> = [
            (player.photo.clone(), Framing::PHOTOGRAPH),
            (player.face.clone(), Framing::DRAWN),
        ]
        .into_iter()
        .filter_map(|(url, framing)| url.map(|url| (url, framing)))
        .collect();
        if sources.is_empty() {
            return;
        }
        self.outbox.push_back((player.id, sources));
    }

    /// **Asks for the next photograph, no more often than [`Self::SPACING`].**
    ///
    /// One request at a time, oldest first, so the squad is photographed in the
    /// order it walked out. Everything about why is on [`Self::SPACING`]: what
    /// this is really spacing out is not the fetch but the work that follows
    /// it, which happens off the ECS where nothing else can pace it.
    ///
    /// Idle for all but the opening seconds of a match — the outbox is filled
    /// as the squad is dressed and never again.
    pub fn ask(time: Res<Time<Real>>, portraits: Option<ResMut<Portraits>>) {
        let Some(mut portraits) = portraits else {
            return;
        };
        if portraits.outbox.is_empty() {
            return;
        }
        portraits.cooldown -= time.delta_secs();
        if portraits.cooldown > 0.0 {
            return;
        }
        portraits.cooldown = Self::SPACING;

        let Some((id, sources)) = portraits.outbox.pop_front() else {
            return;
        };
        let inbox = portraits.inbox.clone();
        spawn_local(async move {
            for (url, framing) in sources {
                if let Some(picture) = Self::picture(&url, framing).await {
                    if let Ok(mut inbox) = inbox.lock() {
                        // Oldest first, so a backlog resolves toward the men
                        // being dressed now — see [`Self::MAILBOX`].
                        while inbox.len() >= Self::MAILBOX {
                            inbox.remove(0);
                        }
                        inbox.push((id, picture));
                    }
                    return;
                }
            }
        });
    }

    /// Fold whatever has come back into the faces on the pitch.
    ///
    /// Three things happen per arrival, and the last two are what make the
    /// first one look like the man rather than like a photograph stuck to a
    /// model. His face sheet is repainted with the picture laid over it; the
    /// rest of his skin moves to the ramp entry nearest the complexion IN the
    /// picture; and the cap of hair over the top of the picture moves to the
    /// entry nearest what the picture has up there.
    ///
    /// The sheet is painted again from scratch rather than edited: the one he
    /// is wearing lives on the GPU and cannot be read back. It costs a quarter
    /// of a million texels once per player per match.
    ///
    /// ⚠ **One a frame**, and the reason is that quarter of a million texels.
    /// Twenty-two pictures of the same squad are asked for within a few frames
    /// of each other and come back over one connection to one host, so they
    /// land in a burst — and the whole burst used to be folded on the frame the
    /// last of them arrived. Measured in the browser against a local library on
    /// an RTX 3080 Ti, eighteen faces cost **105 ms in a single frame**: six
    /// dropped at sixty hertz, in a replay whose every other frame is two and a
    /// half milliseconds. It lands during the pre-match ceremony, because that
    /// is when a squad is dressed, and the ceremony's pass along the faces is a
    /// moving camera — which is precisely the shot a dropped frame shows up in.
    ///
    /// Paced, the same burst is eighteen frames of six milliseconds and the pan
    /// does not miss a refresh. Nothing is lost by the wait: the face a man is
    /// already wearing is the one the viewer drew for him, so a frame later is
    /// a frame later and not a frame of nothing. Same argument, same shape, as
    /// [`Actors::take_the_field`](crate::players::actors::Actors::take_the_field)'s
    /// dressing budget.
    pub fn attach(
        portraits: Option<Res<Portraits>>,
        config: Res<ViewerConfig>,
        actors: Query<(Entity, &PlayerActor)>,
        mut skin: Query<
            (&Flesh, &mut MeshMaterial3d<StandardMaterial>),
            Without<DressedFlesh>,
        >,
        // The merged limbs — cloth and skin on one sheet — move the same way
        // the bare parts above do, except that their destination is the
        // wardrobe's sheet for the same strip in the new tone rather than a
        // ramp entry. The `Without` filters are what let the two queries
        // borrow the same component: no part carries both markers, but only
        // the filters can say so to the scheduler.
        mut dressed: Query<
            (&DressedFlesh, &mut MeshMaterial3d<StandardMaterial>),
            Without<Flesh>,
        >,
        mut hair: Query<(&Thatch, &mut Visibility)>,
        mut wardrobe: Option<ResMut<Wardrobe>>,
        mut materials: ResMut<Assets<StandardMaterial>>,
        mut images: ResMut<Assets<Image>>,
    ) {
        let Some(portraits) = portraits else {
            return;
        };
        let arrivals: Vec<(u32, Portrait)> = match portraits.inbox.lock() {
            // Taken from the FRONT, so a squad is photographed in the order it
            // was sent for rather than backwards.
            Ok(mut inbox) => {
                let taking = Self::A_FRAME.min(inbox.len());
                inbox.drain(..taking).collect()
            }
            Err(_) => Vec::new(),
        };
        if arrivals.is_empty() {
            return;
        }

        let layout = BodyParts::face_layout();
        // How large a sheet to paint him onto — the same decision that cut the
        // rest of him, taken once when the squad was assembled. See
        // [`Grain::face`].
        let grain = portraits.grain;
        for (id, picture) in arrivals {
            let Some(player) = config.players.iter().find(|player| player.id == id) else {
                continue;
            };
            // The head under the picture is painted as a SHAVED one, in the
            // colour of the hair in the picture. Which is what it has to be,
            // because the cap of hair this player was wearing comes off a few
            // lines down: a man's own hair is in his photograph, over his own
            // forehead at his own hairline, and a moulded cap over the top of
            // that is a helmet — see the same argument, and the same outcome,
            // for the nose that used to stand on the front of this face.
            let mut look = Complexion::face(player);
            look.shaved = true;
            if let Some(tone) = picture.hair_tone() {
                look.hair = Color::srgb(tone.x, tone.y, tone.z);
            }
            // …and the SKIN, for the same reason and to the same picture.
            // Everything on this sheet the picture does not reach is painted
            // in this: the temples past the edge of it, the back of his head,
            // the underside of his jaw. Left as the nationality-drawn guess
            // it left a seam down the side of every photographed face with
            // the man's own colour on one side of it and a stranger's on the
            // other — the same argument that moved his neck and his arms onto
            // the picture, applied to the part of his head the camera never
            // saw. Not rounded to a ramp entry, unlike the body: this sheet is
            // already his alone, so nothing is batched by it.
            if let Some(tone) = picture.cheek_tone() {
                look.skin = Color::srgb(tone.x, tone.y, tone.z);
            }
            if let Some((_, material)) = portraits.faces.iter().find(|(face, _)| *face == id) {
                let sheet =
                    images.add(Textures::photographed_face(&layout, &look, &picture, grain));
                if let Some(mut material) = materials.get_mut(material) {
                    // The base colour has been carrying his complexion, which
                    // is what a head with no picture on it is painted in. A
                    // base colour MULTIPLIES the texture over it, so leaving a
                    // tan there would put the man's own skin through his own
                    // photograph twice.
                    material.base_color = Color::WHITE;
                    material.base_color_texture = Some(sheet);
                }
            }

            let Some((actor, _)) = actors.iter().find(|(_, actor)| actor.id == id) else {
                continue;
            };
            if let Some(tone) = picture.cheek_tone() {
                let entry = Complexion::nearest_skin(tone);
                if let Some(material) = portraits.complexions.get(entry) {
                    for (part, mut worn) in &mut skin {
                        if part.actor == actor {
                            worn.0 = material.clone();
                        }
                    }
                }
                // …and the merged limbs follow, onto the same strip's sheet
                // in the same nearest tone — through the wardrobe, so a
                // second player photographed into that tone lands in the
                // batch this one just created. See [`Wardrobe::limb_sheet`].
                if let Some(wardrobe) = wardrobe.as_mut() {
                    for (part, mut worn) in &mut dressed {
                        if part.actor == actor {
                            worn.0 =
                                wardrobe.limb_sheet(part.strip, entry, &mut materials, &mut images);
                        }
                    }
                }
            }
            // And the cap comes off. It is a moulded shell that sits over the
            // forehead and down past the ears, and the man in the picture is
            // already wearing his own hair inside it — cut at his hairline,
            // parted where he parts it. What is left is his skull with his
            // hair painted over the crown of it, which from any distance a
            // match is watched at is a head of hair and not a helmet.
            for (cap, mut seen) in &mut hair {
                if cap.actor == actor {
                    *seen = Visibility::Hidden;
                }
            }
        }
    }

    /// Fetch one picture and turn it into a cut-out patch, or nothing.
    ///
    /// Every step of this can fail for a reason that is not a fault: the
    /// player has no photograph (404), the picture library is on another
    /// origin and the browser will not let a canvas read it, the machine is
    /// offline. None of them is worth a word in the console — the face the
    /// viewer drew is already on his head and stays there.
    async fn picture(url: &str, framing: Framing) -> Option<Portrait> {
        let window = web_sys::window()?;
        let response: Response = JsFuture::from(window.fetch_with_str(url))
            .await
            .ok()?
            .dyn_into()
            .ok()?;
        if !response.ok() {
            return None;
        }
        let blob: Blob = JsFuture::from(response.blob().ok()?)
            .await
            .ok()?
            .dyn_into()
            .ok()?;

        // Through a blob URL rather than straight into a texture, for two
        // reasons that both matter. The browser owns decoders for PNG and for
        // SVG and this crate would otherwise have to carry one of each into a
        // WebAssembly binary that is already six megabytes; and a `blob:` URL
        // is same-origin whatever the picture came from, so the canvas it is
        // drawn on can be read back — which a cross-origin image would taint
        // beyond reading, even after `fetch` has already been allowed it.
        let source = Url::create_object_url_with_blob(&blob).ok()?;
        let decoded = Self::decoded(&source, framing).await;
        let _ = Url::revoke_object_url(&source);
        let (width, height, pixels) = decoded?;

        framing.cut(&pixels, width, height)
    }

    /// Hand the picture to the browser's own decoder and get its pixels back.
    ///
    /// This is the only step that has to happen out there. PNG and SVG both
    /// arrive as bytes this crate cannot read — carrying a decoder for each
    /// into a WebAssembly binary that is already six megabytes is not worth
    /// the megabyte, and the SVG one would be a browser. Everything done to
    /// the pixels afterwards is done here, where it can be tested.
    async fn decoded(source: &str, framing: Framing) -> Option<(u32, u32, Vec<u8>)> {
        let window = web_sys::window()?;
        let document = window.document()?;

        let image = HtmlImageElement::new().ok()?;
        if let Some((width, height)) = framing.intrinsic {
            image.set_width(width);
            image.set_height(height);
        }
        // `onload`/`onerror` want plain functions and a promise's two arms are
        // exactly that, so this needs no closures of its own.
        let decoded = js_sys::Promise::new(&mut |resolve, reject| {
            image.set_onload(Some(&resolve));
            image.set_onerror(Some(&reject));
        });
        image.set_src(source);
        JsFuture::from(decoded).await.ok()?;

        let (width, height) = framing
            .intrinsic
            .unwrap_or((image.natural_width(), image.natural_height()));
        if width == 0 || height == 0 {
            return None;
        }

        let canvas: HtmlCanvasElement = document.create_element("canvas").ok()?.dyn_into().ok()?;
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        context
            .draw_image_with_html_image_element_and_dw_and_dh(
                &image,
                0.0,
                0.0,
                width as f64,
                height as f64,
            )
            .ok()?;

        let data = context
            .get_image_data(0.0, 0.0, width as f64, height as f64)
            .ok()?;
        Some((width, height, data.data().0))
    }

    /// Cut the studio out from behind him.
    ///
    /// A club head shot is a man on a plain pale card, and everything the head
    /// does not cover has to end up transparent — otherwise a slice of that
    /// card gets projected onto his cheekbones the moment his face turns out
    /// to be narrower than the skull it is going onto, which for a picture
    /// laid on a model is most of them.
    ///
    /// A flood fill in from the border rather than a colour test on every
    /// pixel, because the two are not the same question: the card is a smooth
    /// gradient, and a threshold wide enough to hold all of it also holds the
    /// whites of his eyes and the light side of a grey shirt. Growing inward
    /// and comparing each pixel to the one it grew FROM follows a gradient as
    /// far as it goes and stops dead at the edge of a head, which is the only
    /// place the colour jumps.
    fn key_out_the_studio(pixels: &mut [u8], width: u32, height: u32) {
        /// How far one background pixel may sit from its neighbour. Wide
        /// enough for a gradient and for the grain of a JPEG re-encode,
        /// nowhere near the twenty or thirty levels between card and skin.
        const STEP: i32 = 30;
        /// A background pixel is at least this bright in every channel…
        const PALE: i32 = 150;
        /// …and this close to colourless. The one thing a studio card is that
        /// skin never is: skin is warm, and even the palest face carries
        /// forty levels between its red and its blue.
        const GREY: i32 = 30;

        let at = |x: u32, y: u32| ((y * width + x) * 4) as usize;
        let channels = |pixels: &[u8], index: usize| {
            [
                pixels[index] as i32,
                pixels[index + 1] as i32,
                pixels[index + 2] as i32,
            ]
        };
        let card = |rgb: [i32; 3]| {
            let (low, high) = (
                rgb.iter().copied().min().unwrap_or(0),
                rgb.iter().copied().max().unwrap_or(0),
            );
            low >= PALE && high - low <= GREY
        };

        let mut background = vec![false; (width * height) as usize];
        let mut queue = VecDeque::new();
        let border = (0..width)
            .flat_map(|x| [(x, 0), (x, height - 1)])
            .chain((0..height).flat_map(|y| [(0, y), (width - 1, y)]));
        for (x, y) in border {
            let index = (y * width + x) as usize;
            if !background[index] && card(channels(pixels, at(x, y))) {
                background[index] = true;
                queue.push_back((x, y));
            }
        }

        while let Some((x, y)) = queue.pop_front() {
            let from = channels(pixels, at(x, y));
            let neighbours = [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ];
            for (nx, ny) in neighbours {
                if nx >= width || ny >= height {
                    continue;
                }
                let index = (ny * width + nx) as usize;
                if background[index] {
                    continue;
                }
                let rgb = channels(pixels, at(nx, ny));
                let step = (0..3).map(|c| (rgb[c] - from[c]).abs()).max().unwrap_or(0);
                if step <= STEP && card(rgb) {
                    background[index] = true;
                    queue.push_back((nx, ny));
                }
            }
        }

        // One ring in from the edge goes too. The pixel where the card meets
        // his hair is half of each, and half of a pale card is a bright rim
        // right round the silhouette — which is exactly what a badly cut-out
        // head looks like. A pixel of his outline is cheaper than that, and
        // the outline is under the soft edge of the mask anyway.
        let mut rim = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let index = (y * width + x) as usize;
                if background[index] {
                    continue;
                }
                let touching = [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ]
                .into_iter()
                .any(|(nx, ny)| {
                    nx < width && ny < height && background[(ny * width + nx) as usize]
                });
                if touching {
                    rim.push(index);
                }
            }
        }
        let keyed: Vec<usize> = background
            .iter()
            .enumerate()
            .filter_map(|(index, keyed)| keyed.then_some(index))
            .chain(rim)
            .collect();
        for index in keyed {
            pixels[index * 4 + 3] = 0;
        }

        // A cut is not an edge. What is left at this point is a head with a
        // one-texel cliff round it, and laid onto a skull that cliff shows as
        // a line where the photograph stops and the painted face starts —
        // the join being between two complexions that are close but never
        // identical. So the outline is softened, and the colour under it is
        // pushed outward first: fading alpha over pixels that are still the
        // colour of the studio would fade the studio back in.
        Self::bleed_outward(pixels, width, height);
        Self::soften_the_outline(pixels, width, height);
    }

    /// Push the head's colour out into the transparent ground around it, so
    /// that anything sampling across the edge finds skin and hair out there
    /// rather than the card that was keyed away.
    fn bleed_outward(pixels: &mut [u8], width: u32, height: u32) {
        /// Two rings, which is one more than the blur below can reach.
        const RINGS: usize = 2;

        for _ in 0..RINGS {
            let source = pixels.to_vec();
            for y in 0..height {
                for x in 0..width {
                    let at = ((y * width + x) * 4) as usize;
                    if source[at + 3] > 0 {
                        continue;
                    }
                    let (mut colour, mut count) = ([0u32; 3], 0u32);
                    for (nx, ny) in [
                        (x.wrapping_sub(1), y),
                        (x + 1, y),
                        (x, y.wrapping_sub(1)),
                        (x, y + 1),
                    ] {
                        if nx >= width || ny >= height {
                            continue;
                        }
                        let neighbour = ((ny * width + nx) * 4) as usize;
                        if source[neighbour + 3] == 0 {
                            continue;
                        }
                        for channel in 0..3 {
                            colour[channel] += source[neighbour + channel] as u32;
                        }
                        count += 1;
                    }
                    if count > 0 {
                        for channel in 0..3 {
                            pixels[at + channel] = (colour[channel] / count) as u8;
                        }
                    }
                }
            }
        }
    }

    /// One box pass over the alpha alone: enough to turn a stepped cut into a
    /// ramp a texel or two wide, which at the size a face is sampled at is the
    /// difference between an outline and a seam.
    fn soften_the_outline(pixels: &mut [u8], width: u32, height: u32) {
        let source: Vec<u8> = pixels.iter().skip(3).step_by(4).copied().collect();
        for y in 0..height {
            for x in 0..width {
                let (mut total, mut count) = (0u32, 0u32);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                            continue;
                        }
                        total += source[(ny as u32 * width + nx as u32) as usize] as u32;
                        count += 1;
                    }
                }
                pixels[((y * width + x) * 4 + 3) as usize] = (total / count.max(1)) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::textures::{Beard, FaceLook};
    use crate::players::body::preview;

    /// A patch of studio card with a head-coloured blob sitting in the middle
    /// of it, which is what a club head shot is once it has been cropped.
    fn head_shot(width: u32, height: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                // The card is a gradient, not a flat fill — which is the whole
                // reason the key grows inward instead of testing each pixel
                // against one colour.
                let card = 208 + (y * 30 / height) as u8;
                let inside = {
                    let (dx, dy) = (
                        x as f32 - width as f32 * 0.5,
                        y as f32 - height as f32 * 0.5,
                    );
                    (dx / (width as f32 * 0.28)).powi(2) + (dy / (height as f32 * 0.36)).powi(2)
                        < 1.0
                };
                pixels.extend_from_slice(&if inside {
                    [214, 168, 138, 255]
                } else {
                    [card, card, card, 255]
                });
            }
        }
        pixels
    }

    /// The card goes and the head stays.
    ///
    /// Both halves are the test. A key that takes the card and stops at the
    /// hairline is what makes a picture of a narrow face sit on a wide skull
    /// without a slice of studio across the cheekbone; a key that carries on
    /// into the face takes the face with it.
    #[test]
    fn the_studio_is_cut_off_the_back_of_a_head_shot() {
        const SIZE: u32 = 64;
        let mut pixels = head_shot(SIZE, SIZE);
        Portraits::key_out_the_studio(&mut pixels, SIZE, SIZE);

        let alpha = |x: u32, y: u32| pixels[((y * SIZE + x) * 4 + 3) as usize];
        // Corners, edges and the run of card between the head and the frame.
        for (x, y) in [
            (0, 0),
            (SIZE - 1, 0),
            (0, SIZE - 1),
            (SIZE - 1, SIZE - 1),
            (2, 32),
            (32, 2),
        ] {
            assert_eq!(alpha(x, y), 0, "the card is still there at {x},{y}");
        }
        // And the head itself, which has to come through whole.
        for (x, y) in [(32, 32), (32, 24), (26, 32), (38, 32), (32, 40)] {
            assert_eq!(alpha(x, y), 255, "the head has been keyed away at {x},{y}");
        }
    }

    /// A face-coloured pixel on the border is not the studio, however pale the
    /// player is: the key seeds on colourlessness, and skin has colour in it.
    #[test]
    fn a_pale_face_is_not_a_pale_card() {
        const SIZE: u32 = 8;
        // The palest complexion in the shared ramp, over the whole patch.
        let mut pixels: Vec<u8> = (0..SIZE * SIZE)
            .flat_map(|_| [247, 214, 191, 255])
            .collect();
        Portraits::key_out_the_studio(&mut pixels, SIZE, SIZE);
        assert!(
            pixels.chunks(4).all(|texel| texel[3] == 255),
            "a face was keyed out as though it were the studio"
        );
    }

    /// The crop lands on the head rather than beside it.
    ///
    /// Measured the only way that means anything: the patch that comes out is
    /// asked where its face is, and the answer has to be the middle of it. If
    /// the crop rectangle ever drifts, the landmarks drift with it and every
    /// face on the pitch is hung a centimetre off its own skull.
    #[test]
    fn the_crop_keeps_the_head_in_the_middle_of_the_patch() {
        const WIDTH: u32 = 260;
        const HEIGHT: u32 = 310;
        let source = head_shot(WIDTH, HEIGHT);
        let patch = Framing::PHOTOGRAPH
            .cut(&source, WIDTH, HEIGHT)
            .expect("a picture this size crops");

        assert_eq!(patch.width, Framing::PATCH);
        // The patch keeps the crop's shape rather than being squared off — a
        // step across it has to be the same distance on the man as a step
        // down, or every measurement below it is on two different scales.
        let shape = patch.height as f32 / patch.width as f32;
        let crop = (Framing::PHOTOGRAPH.crop.3 * HEIGHT as f32)
            / (Framing::PHOTOGRAPH.crop.2 * WIDTH as f32);
        assert!(
            (shape - crop).abs() < 0.02,
            "the patch is {shape:.2} tall for every one across, the crop {crop:.2}"
        );

        // The blob is a head with no eyes in it, so the measurement should
        // refuse it and the framing's own numbers should stand.
        assert!(patch.eyes > 0.0 && patch.eyes < 1.0);
        assert!(patch.pupils > 0.0 && patch.pupils < patch.width as f32 * 0.5);
        let opaque = |u: f32, v: f32| {
            let (x, y) = (
                (u * patch.width as f32) as u32,
                (v * patch.height as f32) as u32,
            );
            patch.pixels[((y * patch.width + x) * 4 + 3) as usize] > 128
        };
        assert!(opaque(patch.centre, patch.eyes), "no head on the eye line");
    }

    /// A face is measured, not assumed.
    ///
    /// The whole reason this exists: the photograph library is not framed to
    /// one standard, and a head shot cropped closer than the rest was being
    /// told its face was the size the others' are — which stretched it half as
    /// wide again across the skull. So the same head, drawn twice at two
    /// scales, has to come back with two pupil measurements in the same ratio.
    #[test]
    fn a_face_is_measured_off_the_picture_and_not_off_its_framing() {
        const WIDTH: u32 = 260;
        const HEIGHT: u32 = 310;
        let small = Framing::PHOTOGRAPH
            .cut(&portrait_with_eyes(WIDTH, HEIGHT, 0.60), WIDTH, HEIGHT)
            .expect("crops");
        let large = Framing::PHOTOGRAPH
            .cut(&portrait_with_eyes(WIDTH, HEIGHT, 0.90), WIDTH, HEIGHT)
            .expect("crops");

        let ratio = large.pupils / small.pupils;
        assert!(
            (ratio - 1.5).abs() < 0.2,
            "a head drawn half as big again measured {ratio:.2} times the pupils"
        );
        // …and the eye line is found rather than assumed: on these it sits a
        // little above the middle of the head, and the head sits in a
        // different place in each of the two.
        for (patch, scale) in [(&small, 0.60f32), (&large, 0.90)] {
            let drawn = HEIGHT as f32 * 0.46 - HEIGHT as f32 * 0.32 * scale * 0.10;
            let drawn = (drawn - Framing::PHOTOGRAPH.crop.1 * HEIGHT as f32)
                / (Framing::PHOTOGRAPH.crop.3 * HEIGHT as f32);
            assert!(
                (patch.eyes - drawn).abs() < 0.05,
                "eyes measured at {:.3} of the patch, drawn at {drawn:.3}",
                patch.eyes
            );
        }
    }

    /// A head on a studio card with a pair of eyes in it, at a chosen size —
    /// the same man photographed from two distances.
    fn portrait_with_eyes(width: u32, height: u32, scale: f32) -> Vec<u8> {
        let centre = (width as f32 * 0.5, height as f32 * 0.46);
        let half = (width as f32 * 0.26 * scale, height as f32 * 0.32 * scale);
        // Anatomy, so the fixture is the shape the measurement expects to
        // find: pupils a little under half the head's width apart, on a line
        // just above the middle of it, with brows a touch higher — which is
        // the pair of dark marks this has to NOT mistake for the eyes.
        let pupils = half.0 * 2.0 * 0.42;
        let eyes = centre.1 - half.1 * 0.10;

        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let (fx, fy) = (x as f32, y as f32);
                let head = ((fx - centre.0) / half.0).powi(2) + ((fy - centre.1) / half.1).powi(2);
                let card = 208 + (y * 30 / height) as u8;
                let mark = |at: f32, wide: f32, tall: f32| {
                    ((fx - (centre.0 + at)) / wide).powi(2) + ((fy - eyes) / tall).powi(2) < 1.0
                };
                let brow = |at: f32| {
                    ((fx - (centre.0 + at)) / (pupils * 0.30)).powi(2)
                        + ((fy - (eyes - pupils * 0.30)) / (pupils * 0.07)).powi(2)
                        < 1.0
                };
                let texel = if head >= 1.0 {
                    [card, card, card]
                } else if mark(-pupils * 0.5, pupils * 0.16, pupils * 0.08)
                    || mark(pupils * 0.5, pupils * 0.16, pupils * 0.08)
                {
                    [28, 24, 22]
                } else if brow(-pupils * 0.5) || brow(pupils * 0.5) {
                    [64, 50, 40]
                } else {
                    [214, 168, 138]
                };
                pixels.extend_from_slice(&[texel[0], texel[1], texel[2], 255]);
            }
        }
        pixels
    }

    /// Dev-only: paint one face sheet from a real picture and write it out to
    /// be looked at, which is the only way to review a face.
    ///
    /// Takes a raw RGBA dump rather than a PNG — this crate has no decoder in
    /// it, on purpose (see [`Portraits::decoded`]). Run with:
    ///   MATCH_PICTURE=<file.rgba> MATCH_PICTURE_SIZE=260x310 \
    ///   MATCH_PICTURE_DUMP=<dir> cargo test -p match_viewer --lib \
    ///   dump_photographed_face -- --ignored
    ///
    /// `MATCH_PICTURE_ROWS` on top of that prints the eye-line search row by
    /// row with its four terms broken out, which is the only way to see WHY a
    /// face went onto a skull two centimetres up rather than merely that it
    /// did.
    #[test]
    #[ignore = "writes a file; run by hand when the projection changes"]
    fn dump_photographed_face() {
        let (Ok(picture), Ok(size), Ok(directory)) = (
            std::env::var("MATCH_PICTURE"),
            std::env::var("MATCH_PICTURE_SIZE"),
            std::env::var("MATCH_PICTURE_DUMP"),
        ) else {
            panic!("set MATCH_PICTURE, MATCH_PICTURE_SIZE and MATCH_PICTURE_DUMP");
        };
        let (width, height) = size.split_once('x').expect("WxH");
        let (width, height): (u32, u32) = (width.parse().unwrap(), height.parse().unwrap());
        let source = std::fs::read(&picture).expect("read the picture");
        // A drawn cutout comes in with its own transparency; anything else is
        // taken to be a studio head shot.
        let framing = if picture.contains("cutout") {
            Framing::DRAWN
        } else {
            Framing::PHOTOGRAPH
        };
        let patch = framing
            .cut(&source, width, height)
            .expect("cut the head out");

        let directory = std::path::Path::new(&directory);
        std::fs::write(directory.join("patch.rgba"), &patch.pixels).expect("wrote the patch");

        // What  builds for a player whose picture has
        // arrived: the crown painted in the hair the picture has on it, and
        // the head under it shaved, because the cap comes off.
        let look = FaceLook {
            // Off the picture, exactly as `Portraits::attach` does it — the
            // dump is worth nothing if the head in it is painted in a tone no
            // player would be wearing.
            skin: patch
                .cheek_tone()
                .map(|tone| Color::srgb(tone.x, tone.y, tone.z))
                .unwrap_or(Color::srgb(0.78, 0.60, 0.46)),
            hair: patch
                .hair_tone()
                .map(|tone| Color::srgb(tone.x, tone.y, tone.z))
                .unwrap_or(Color::srgb(0.10, 0.08, 0.07)),
            eyes: Color::srgb(0.38, 0.50, 0.58),
            brow: 1.0,
            beard: Beard::Stubble,
            shaved: true,
        };
        let sheet =
            Textures::photographed_face(&BodyParts::face_layout(), &look, &patch, Grain::FULL);
        // Level zero only: a pictured sheet carries a mip chain behind it, and
        // everything below is looking at the sheet itself.
        let base = (sheet.width() * sheet.height() * 4) as usize;
        let pixels = sheet.data.clone().expect("the sheet has pixels")[..base].to_vec();
        std::fs::write(directory.join("face.rgba"), &pixels).expect("wrote the sheet");

        // …and the head wearing it, which is the only view that answers the
        // question. A face sheet is a cylinder cut open and laid flat: the
        // eyes on it are ovals a third of their real width, and whether they
        // land on the eye line of a SKULL cannot be seen in it at all.
        const SHOT: usize = 320;
        let mut meshes = Assets::<Mesh>::default();
        let parts = BodyParts::tailor(&mut meshes, Grain::FULL);
        let layout = BodyParts::face_layout();
        let head = Transform::from_xyz(0.0, -layout.foot - layout.span * 0.5, 0.0);
        // The generated face beside it, on the same head from the same
        // angles: the picture is only an improvement if it beats what the
        // viewer already draws, and that is not a judgement any assertion
        // makes.
        let mut images = Assets::<Image>::default();
        let plain = Textures::face(&mut images, &layout, &look);
        let plain = images.get(&plain).expect("just added").clone();
        let plain_pixels = plain.data.clone().expect("the sheet has pixels");

        for (name, bearing) in [
            ("head_front", std::f32::consts::PI),
            ("head_quarter", std::f32::consts::PI - 0.9),
            ("head_side", std::f32::consts::FRAC_PI_2),
        ] {
            for (suffix, sheet) in [
                ("", (sheet.width(), sheet.height(), pixels.as_slice())),
                (
                    "_plain",
                    (plain.width(), plain.height(), plain_pixels.as_slice()),
                ),
            ] {
                let mut canvas = preview::Canvas::new(SHOT, SHOT);
                let lens = preview::Lens {
                    bearing,
                    bottom: -layout.span * 0.60,
                    top: layout.span * 0.60,
                };
                preview::wearing(
                    &mut canvas,
                    &lens,
                    &meshes,
                    &parts,
                    head,
                    sheet,
                    suffix == "_plain",
                );
                std::fs::write(
                    directory.join(format!("{name}{suffix}.rgba")),
                    canvas.pixels(),
                )
                .expect("wrote the head");
            }
        }
        println!(
            "patch {}x{} (centre {:.3}, eyes {:.3}, pupils {:.1}px = {:.1} source px), \
             sheet {}x{}, heads {SHOT}x{SHOT} in {}",
            patch.width,
            patch.height,
            patch.centre,
            patch.eyes,
            patch.pupils,
            patch.pupils * framing.crop.2 * width as f32 / patch.width as f32,
            sheet.width(),
            sheet.height(),
            directory.display()
        );
    }

    /// Dev-only: measure a whole FOLDER of pictures at once and print what
    /// comes out, which is the only way to tell a detector that works from
    /// one that works on the two pictures it was written against.
    ///
    /// Every file is `<name>_<W>x<H>.rgba`. Beside each measurement it writes
    /// the patch back out with the landmarks drawn on it, because a column of
    /// numbers cannot say whether the mark that was found is an eye. RED is
    /// what was measured — the eye line and the two pupils; CYAN is the top
    /// of the head; and YELLOW and GREEN are where the SKULL this is all
    /// going onto puts its own crown and chin, in this picture's pupil
    /// widths. Yellow belongs on the top of his head and green on the point
    /// of his chin, and how far off they are is how far off the skull is.
    ///   MATCH_PICTURE_DIR=<dir> cargo test -p match_viewer --lib \
    ///   measure_pictures -- --ignored --nocapture
    #[test]
    #[ignore = "reads a folder of raw pictures; run by hand"]
    fn measure_pictures() {
        let Ok(directory) = std::env::var("MATCH_PICTURE_DIR") else {
            panic!("set MATCH_PICTURE_DIR");
        };
        let directory = std::path::Path::new(&directory);
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(directory)
            .expect("read the folder")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|end| end == "rgba"))
            .filter(|path| !path.to_string_lossy().contains(".marked"))
            .collect();
        files.sort();

        // The head every one of these is going onto, so its own crown and
        // chin can be drawn on each picture in that picture's pupil widths.
        let skull = BodyParts::face_layout();
        // Interpupillary distance in metres, as `Portrait::scale` has it.
        const PUPILS: f32 = 0.064;

        let mut cranium: Vec<f32> = Vec::new();
        let mut breadth: Vec<f32> = Vec::new();
        println!(
            "{:<12} {:>7} {:>7} {:>7} {:>8} {:>8} {:>8}",
            "picture", "crown", "widest", "pupils", "eyes", "crown/ipd", "wide/ipd"
        );
        for file in &files {
            let stem = file.file_stem().unwrap().to_string_lossy().to_string();
            let Some((name, size)) = stem.rsplit_once('_') else {
                continue;
            };
            let Some((wide, tall)) = size.split_once('x') else {
                continue;
            };
            let (wide, tall): (u32, u32) = (wide.parse().unwrap(), tall.parse().unwrap());
            let source = std::fs::read(file).expect("read the picture");
            let Some(patch) = Framing::PHOTOGRAPH.cut(&source, wide, tall) else {
                println!("{name:<12} REFUSED");
                continue;
            };
            let head = Silhouette::of(&patch.pixels, patch.width, patch.height);
            let (crown, widest) = match &head {
                Some(head) => (head.crown as f32, head.widest as f32),
                None => (f32::NAN, f32::NAN),
            };
            let eyes = patch.eyes * patch.height as f32;
            let above = (eyes - crown) / patch.pupils;
            let across = widest / patch.pupils;
            cranium.push(above);
            breadth.push(across);
            println!(
                "{name:<12} {crown:>7.0} {widest:>7.0} {:>7.1} {eyes:>8.1} {above:>8.2} {across:>8.2}",
                patch.pupils
            );

            let mut marked = patch.pixels.clone();
            let mut mark = |x: i32, y: i32, ink: [u8; 3]| {
                if x >= 0 && y >= 0 && x < patch.width as i32 && y < patch.height as i32 {
                    let at = ((y as u32 * patch.width + x as u32) * 4) as usize;
                    marked[at..at + 3].copy_from_slice(&ink);
                    marked[at + 3] = 255;
                }
            };
            for x in 0..patch.width as i32 {
                mark(x, eyes as i32, [255, 0, 0]);
                mark(x, crown as i32, [0, 200, 255]);
                // Where the head the picture is going ONTO puts its crown and
                // its chin, in this picture's own pupil widths — which is the
                // whole question this whole dump exists to ask, and the only
                // way to ask it of a squad at once. Yellow should sit on the
                // top of his head and green on the point of his chin.
                let of_a_head = |part: f32| (eyes + patch.pupils * part / PUPILS) as i32;
                mark(
                    x,
                    of_a_head(skull.eyes - (skull.foot + skull.span)),
                    [255, 220, 0],
                );
                mark(x, of_a_head(skull.eyes - skull.chin), [0, 255, 120]);
            }
            for side in [-0.5f32, 0.5] {
                let column = (patch.centre * patch.width as f32 + side * patch.pupils) as i32;
                for y in 0..patch.height as i32 {
                    mark(column, y, [255, 0, 0]);
                }
            }
            std::fs::write(
                directory.join(format!(
                    "{name}.marked_{}x{}.rgba",
                    patch.width, patch.height
                )),
                &marked,
            )
            .expect("wrote the marked patch");
        }

        let median = |mut of: Vec<f32>| {
            of.retain(|value| value.is_finite());
            of.sort_by(|a, b| a.partial_cmp(b).unwrap());
            of.get(of.len() / 2).copied().unwrap_or(f32::NAN)
        };
        println!(
            "\n{} pictures: median crown {:.2} pupil-widths above the eyes, \
             head {:.2} pupil-widths across",
            files.len(),
            median(cranium),
            median(breadth)
        );
    }
}
