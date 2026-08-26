//! Pixels and glyphs the crate makes for itself.
//!
//! Nothing is fetched from the asset server: a replay that had to download its
//! own images would show a bare pitch for as long as they took. So [`textures`]
//! paints every one of them at startup — faces, turf, rings, contact shadows,
//! the chips on the bar — and [`typeface`] carries the two faces every label is
//! drawn with, along with the stencil the painter draws text through.

pub(crate) mod textures;
pub(crate) mod typeface;
