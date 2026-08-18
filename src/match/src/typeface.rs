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

include!("../fonts/coverage.rs");

/// Installs [`Faces`].
pub struct Typeface;

impl Typeface {
    /// Compiled in rather than fetched: the viewer is one WASM artefact the
    /// page loads and runs, and a face arriving a round trip later would draw
    /// the first frames of the replay with no names on it.
    const OUTFIT: &'static [u8] = include_bytes!("../fonts/Outfit-subset.ttf");
    const MANROPE: &'static [u8] = include_bytes!("../fonts/Manrope-subset.ttf");
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

    fn in_outfit(character: char) -> bool {
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
