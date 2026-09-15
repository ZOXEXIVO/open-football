//! **The white plate the viewer's cards are cut from.**
//!
//! Three pieces of furniture wear it now — the score in the corner
//! ([`Scoreboard`](crate::ui::scoreboard::Scoreboard)), the team sheet the
//! ceremony opens on ([`TeamSheet`](crate::ui::teamsheet::TeamSheet)) and the
//! card the replay ends with ([`FullTime`](crate::ui::scoreboard::FullTime)) —
//! and they have to be ONE plate rather than three that nearly agree. Three
//! copies of a palette is three places to miss when the plate is asked to be a
//! shade lighter, and the plate has already been asked twice.
//!
//! Only what is genuinely common lives here. How opaque a given plate is and
//! how far its corners are rounded are its own business: the score bug is a
//! small hard chip in the corner of a moving picture and the cards are large
//! panels laid over a stilled one, and they are not the same object at two
//! sizes.

use bevy::prelude::*;

/// The ink and the rules every card on the picture is set with.
pub struct Plate;

impl Plate {
    /// What a club is named in, and the darkest thing on any plate.
    ///
    /// Near-black rather than black, which is what a broadcast caption is set
    /// in and what keeps a white panel from reading as a hole punched in the
    /// picture.
    pub const INK: Color = Color::srgb(0.063, 0.086, 0.114);
    /// A step back: the men, where the clubs are the headings.
    pub const INK_SOFT: Color = Color::srgb(0.29, 0.33, 0.376);
    /// A step back again: headings, shirt numbers, and anything that is there
    /// to be understood rather than read.
    pub const INK_MUTED: Color = Color::srgb(0.42, 0.46, 0.50);
    /// What separates one part of a plate from the next.
    pub const HAIRLINE: Color = Color::srgba(0.063, 0.086, 0.114, 0.10);
    /// The edge round anything drawn in a club's own colour.
    ///
    /// A club can play in white, and a white chip on a white plate is a hole in
    /// it rather than a score — so every one of them is drawn with this, for
    /// the same reason the pins on the seek rail wear theirs: the shape
    /// survives whatever the kit is.
    pub const EDGE: Color = Color::srgba(0.063, 0.086, 0.114, 0.14);
    /// The colour band across the top of a card, one half per side. The crest
    /// this crate does not have: it says whose card this is in the two colours
    /// everything else in the viewer says it in.
    pub const BAND: f32 = 3.0;
    /// How far a card's corners are rounded — the big panels, not the bug.
    pub const CORNER: f32 = 14.0;
}
