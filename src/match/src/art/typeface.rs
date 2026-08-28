//! The faces every label in the viewer is drawn with.
//!
//! Bevy's `default_font` feature compiles in `FiraMono-subset.ttf` and hands it
//! to every `TextFont` that does not name a font of its own — which, in this
//! crate, was all of them. That subset is close to ASCII-only, and a squad list
//! is not an ASCII data set: `Conceição` came out of it as `Concei□□o`, because
//! `ç` and `ã` are not in there and nothing downstream substitutes. `fontique`
//! would normally fall back to a system font, but a browser tab running WASM
//! has no font database to fall back to, so an absent glyph is a box on screen.
//!
//! So the viewer carries the same two faces the web UI already ships. `style.css`
//! declares both under `font-family: "Outfit"` — Outfit for the Latin ranges,
//! Manrope for Cyrillic — and a browser picks between them per character. There
//! is no per-character machinery here, so the choice is made per label instead,
//! off a table `fonts/subset.mjs` reads out of the Outfit subset itself. Guessing
//! it from the script would not work: Outfit is missing 29 characters scattered
//! through Latin Extended-A, so "is this Latin?" is the wrong question.
//!
//! See `fonts/README.md` for where the faces came from.

use bevy::asset::AssetId;
use bevy::prelude::*;
use bevy::text::Font;
use swash::FontRef;
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::{Format, Vector};

include!("../../fonts/coverage.rs");

/// Installs [`Faces`].
pub struct Typeface;

impl Typeface {
    /// Compiled in rather than fetched: the viewer is one WASM artefact the
    /// page loads and runs, and a face arriving a round trip later would draw
    /// the first frames of the replay with no names on it.
    pub const OUTFIT: &'static [u8] = include_bytes!("../../fonts/Outfit-subset.ttf");
    pub const MANROPE: &'static [u8] = include_bytes!("../../fonts/Manrope-subset.ttf");
}

impl Plugin for Typeface {
    fn build(&self, app: &mut App) {
        let mut fonts = app.world_mut().resource_mut::<Assets<Font>>();
        // `AssetId::default()` is the handle behind `FontSource::Handle`'s own
        // default, which is what `TextFont { ..default() }` resolves to. Putting
        // Outfit there means a label that never asks for a face still gets the
        // right one, so only the labels that can carry a name — or a translated
        // string — have to think about it at all.
        //
        // Bevy writes the same slot when `default_font` is on; the feature is
        // off in `Cargo.toml` precisely so this is the only write to it, and so
        // a face that cannot spell the squad list is not shipped alongside two
        // that can.
        fonts
            .insert(AssetId::default(), Font::from_bytes(Self::OUTFIT.to_vec()))
            .expect("the default font slot is a fixed id and is always writable");
        let faces = Faces {
            latin: Handle::default(),
            wide: fonts.add(Font::from_bytes(Self::MANROPE.to_vec())),
        };
        app.insert_resource(faces);
    }
}

/// Which face a given label needs.
#[derive(Resource)]
pub struct Faces {
    /// Outfit — the face the rest of the app is set in, and the one nearly
    /// every label ends up using.
    latin: Handle<Font>,
    /// Manrope — reached for by the labels Outfit cannot spell: the Russian
    /// half-time and loading strings, and the handful of names carrying a
    /// Greek, Cyrillic or Extended-A character Outfit has no glyph for.
    wide: Handle<Font>,
}

impl Faces {
    /// The face `text` can be drawn in without losing a character.
    ///
    /// Outfit unless something in the string is outside it, in which case the
    /// whole label goes to Manrope — one label set in a face that is not quite
    /// the other reads better than one character missing from a name.
    pub fn face_for(&self, text: &str) -> Handle<Font> {
        if text.chars().all(Self::in_outfit) {
            self.latin.clone()
        } else {
            self.wide.clone()
        }
    }

    /// The same question for a label whose text changes under it — the clock,
    /// which swaps between the two half names as the replay runs. Picking the
    /// face per frame would rebuild the atlas every time the second ticks.
    pub fn face_for_all<'a>(&self, texts: impl IntoIterator<Item = &'a str>) -> Handle<Font> {
        if texts
            .into_iter()
            .all(|text| text.chars().all(Self::in_outfit))
        {
            self.latin.clone()
        } else {
            self.wide.clone()
        }
    }

    pub fn in_outfit(character: char) -> bool {
        let code = character as u32;
        OUTFIT_COVERAGE
            .binary_search_by(|(first, last)| {
                if code < *first {
                    core::cmp::Ordering::Greater
                } else if code > *last {
                    core::cmp::Ordering::Less
                } else {
                    core::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }
}

/// Lettering painted into a texture rather than laid out by the text engine.
///
/// The print on the back of a shirt is not a label: it is part of the kit,
/// baked into the material a player wears (see [`crate::players::kit`]), and
/// Bevy's text stack cannot draw into a texture. So it is rasterised here, out
/// of the same two faces every label in the viewer is set in — which is the
/// whole point. The lettering used to come from a hand-rolled 5×7 grid, and a
/// shirt number drawn on graph paper next to a name set in Outfit read as two
/// different games; one face for both is what makes the squad look like one
/// squad.
///
/// `swash` does the outlines. It is not a new dependency in any meaningful
/// sense — `bevy_text` already carries it, and it is what rasterises the labels
/// too, so the glyph on the shirt is the same glyph by the same rasteriser.
pub struct Stencil;

impl Stencil {
    /// Reference size the outlines are measured at before anything is fitted.
    /// Any value works — everything below is a ratio of it — and a round
    /// hundred keeps the intermediate numbers readable in a debugger.
    const REFERENCE: f32 = 100.0;
    /// Air between letters, as a share of the size. Shirt lettering is tracked
    /// out well past a paragraph's, and without it a name at this size closes
    /// up into a single dark bar at the distance the camera actually watches
    /// from.
    const TRACKING: f32 = 0.08;

    /// Draws `text` into a `width` × `height` alpha mask, centred, as large as
    /// the box allows.
    ///
    /// `margin` is the share of the width the letters may use and `cap` the
    /// share of the height — which is what separates a shirt number, as tall as
    /// the panel will take, from a name set smaller with air above and below.
    ///
    /// What is fitted is the INK: the tallest and deepest the actual outlines
    /// in this line reach, measured rather than looked up. The face's own
    /// ascent is far too tall — it leaves room for accents on every line
    /// whether or not there are any, and printed the numbers visibly small —
    /// while its cap height is too short the moment there IS one: MÜLLER put
    /// the diaeresis a texel off the top edge of the panel, because a
    /// diaeresis is by definition the part of the letter that is above the cap
    /// height. Measuring means a name without accents is set exactly as large
    /// as cap-height fitting made it, and only the ones that need the room give
    /// any up.
    ///
    /// Comes back empty (all zero) if the face cannot be read at all, which the
    /// caller sees as an unprinted shirt rather than as a panic.
    pub fn mask(text: &str, width: u32, height: u32, margin: f32, cap: f32) -> Vec<u8> {
        let mut coverage = vec![0u8; (width * height) as usize];
        let Some(line) = Self::set(text, Self::TRACKING) else {
            return coverage;
        };

        let scale = (width as f32 * margin / line.span())
            .min(height as f32 * cap / (line.rise() + line.drop()));
        let left = (width as f32 - line.span() * scale) * 0.5;
        // The ink box is what is centred, and the baseline is not in the middle
        // of it: it sits `drop` up from the bottom of that box.
        let baseline = (height as f32 + (line.rise() - line.drop()) * scale) * 0.5;
        line.draw(&mut coverage, width, height, left, baseline, scale);

        coverage
    }

    /// Sets one line of `text` — picks its face, lays its glyphs out along a
    /// baseline and measures how far the ink reaches — without deciding how
    /// big it is or where it goes.
    ///
    /// `tracking` is the air between letters as a share of the size. It is a
    /// parameter rather than [`Self::TRACKING`] because the three places this
    /// is set want three different values: a shirt is tracked out well past a
    /// paragraph's so a name does not close into a dark bar at the distance the
    /// camera watches from, a wordmark on a perimeter board wants rather less
    /// than that, and an address set at half the wordmark's size wants more
    /// again — small type needs air, not less of it.
    ///
    /// `None` when the face cannot be read at all, or when nothing in `text`
    /// has an outline to draw. Callers print an unlettered shirt or an unlit
    /// board rather than panicking.
    pub fn set(text: &str, tracking: f32) -> Option<Stencilled> {
        let face = Self::face_for(text);
        let font = FontRef::from_index(face, 0)?;

        let charmap = font.charmap();
        let advances = font.glyph_metrics(&[]).scale(Self::REFERENCE);
        let tracking = Self::REFERENCE * tracking;

        let mut context = ScaleContext::new();
        // Unhinted on purpose, here and again in [`Stencilled::draw`]: the
        // panel is stretched over a curved sheet of cloth, or hung along a
        // touchline and looked at from every angle there is, so there is no
        // pixel grid for hinting to snap to and its only effect would be to
        // make the letters uneven.
        let mut measure = context
            .builder(font)
            .size(Self::REFERENCE)
            .hint(false)
            .build();

        // Where each glyph starts along the line, how long the line is, and how
        // far the ink reaches above and below the baseline — all still at the
        // reference size.
        let mut pen = 0.0f32;
        let mut rise = 0.0f32;
        let mut drop = 0.0f32;
        let mut run = Vec::with_capacity(text.chars().count());
        for character in text.chars() {
            let glyph = charmap.map(character);
            // Glyph zero is `.notdef` — the box. It should not be reachable
            // (the caller folds away anything the face cannot spell) but the
            // face is picked for the whole line while the fold asks per
            // character, so a name that pulls the line over to Manrope could in
            // principle carry a letter Manrope has no glyph for. Dropping it is
            // the same call the fold makes: a missing letter, never a box.
            if glyph == 0 {
                continue;
            }
            run.push((glyph, pen));
            pen += advances.advance_width(glyph) + tracking;
            // A space has no outline and no extent; it still takes its advance.
            if let Some(outline) = measure.scale_outline(glyph) {
                let bounds = outline.bounds();
                rise = rise.max(bounds.max.y);
                drop = drop.max(-bounds.min.y);
            }
        }
        // The last letter's tracking is air off the end of the line, not part
        // of it: left in, every line would sit that far left of centre.
        let span = (pen - tracking).max(1.0);
        if run.is_empty() || rise + drop <= 0.0 {
            return None;
        }

        Some(Stencilled {
            face,
            run,
            span,
            rise,
            drop,
        })
    }

    /// Whether the face this would be set in can draw every character of it.
    ///
    /// The caller uses it to decide what to fold: a letter the shirt can print
    /// should be printed, and only the ones neither face has a glyph for get
    /// flattened onto a Latin base.
    pub fn can_print(character: char) -> bool {
        let face = if Faces::in_outfit(character) {
            Typeface::OUTFIT
        } else {
            Typeface::MANROPE
        };
        FontRef::from_index(face, 0).is_some_and(|font| font.charmap().map(character) != 0)
    }

    /// Outfit unless something in the string is outside it, in which case the
    /// whole line goes to Manrope — the same rule [`Faces::face_for`] applies
    /// to a label, so a name and the shirt under it never disagree about which
    /// of the two faces it belongs in.
    fn face_for(text: &str) -> &'static [u8] {
        if text.chars().all(Faces::in_outfit) {
            Typeface::OUTFIT
        } else {
            Typeface::MANROPE
        }
    }
}

/// One line of text with its outlines already measured, ready to be drawn at
/// whatever size and wherever in a mask the caller decides.
///
/// [`Stencil::mask`] fits a line to a box on its own, which is all a shirt ever
/// asks for. A perimeter board is a LOCKUP — a mark, a wordmark and an address
/// that have to share a baseline and add up to a width — and lining those up
/// means knowing how big each of them is BEFORE any of them is drawn. So
/// measuring and drawing are two steps, and `mask` is now the small one built
/// on top of them.
///
/// Every measurement here is in units of [`Stencil::REFERENCE`]; multiply by
/// the scale the caller settles on to get texels.
pub struct Stencilled {
    /// The face the whole line is set in, picked once for the reason
    /// [`Faces::face_for`] gives.
    face: &'static [u8],
    /// Each glyph and where along the line it starts.
    run: Vec<(u16, f32)>,
    span: f32,
    rise: f32,
    drop: f32,
}

impl Stencilled {
    /// How long the line is: the start of the first letter to the end of the
    /// last, with the trailing tracking left off.
    pub fn span(&self) -> f32 {
        self.span
    }

    /// How far the ink reaches above the baseline — measured, not looked up.
    /// See [`Stencil::mask`] for why the face's own ascent is the wrong number
    /// to fit against.
    pub fn rise(&self) -> f32 {
        self.rise
    }

    /// And below it: a descender, or zero for a line that has none.
    pub fn drop(&self) -> f32 {
        self.drop
    }

    /// Paints the line into `coverage`, a `width` × `height` alpha mask, with
    /// the start of the line at `left` and its baseline at `baseline` — both in
    /// texels — at `scale` texels to the reference unit.
    ///
    /// Ink already in the mask survives where it is darker, so several lines,
    /// or a line over a mark, can go into one buffer.
    pub fn draw(
        &self,
        coverage: &mut [u8],
        width: u32,
        height: u32,
        left: f32,
        baseline: f32,
        scale: f32,
    ) {
        let Some(font) = FontRef::from_index(self.face, 0) else {
            return;
        };
        let mut context = ScaleContext::new();
        let mut scaler = context
            .builder(font)
            .size(Stencil::REFERENCE * scale)
            .hint(false)
            .build();
        let mut render = Render::new(&[Source::Outline]);
        render.format(Format::Alpha);

        for (glyph, offset) in &self.run {
            let x = left + offset * scale;
            // The fractional part goes to the rasteriser rather than being
            // rounded away, so letters land where the spacing put them instead
            // of snapping to whole texels and printing unevenly.
            let whole = x.floor();
            render.offset(Vector::new(x - whole, 0.0));
            let Some(image) = render.render(&mut scaler, *glyph) else {
                continue;
            };
            let origin_x = whole as i32 + image.placement.left;
            let origin_y = baseline as i32 - image.placement.top;
            for row in 0..image.placement.height as i32 {
                let y = origin_y + row;
                if y < 0 || y >= height as i32 {
                    continue;
                }
                for column in 0..image.placement.width as i32 {
                    let x = origin_x + column;
                    if x < 0 || x >= width as i32 {
                        continue;
                    }
                    let ink = image.data[(row * image.placement.width as i32 + column) as usize];
                    let target = &mut coverage[(y as u32 * width + x as u32) as usize];
                    // Letters can overlap at this tracking on a tight panel;
                    // the darker of the two wins rather than the later one.
                    *target = (*target).max(ink);
                }
            }
        }
    }
}
