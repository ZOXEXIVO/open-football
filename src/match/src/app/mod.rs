//! The layer between the browser tab and the replay.
//!
//! Nothing in here is part of the match. [`config`] is the document the page
//! hands over and the only thing the viewer is told from outside; [`bringup`]
//! spreads the first frames out so the tab stays answerable while the browser
//! compiles shaders; [`quality`] and [`stage`] are the two halves of the same
//! question — how much the machine on the other end can afford to draw, in
//! samples per pixel and in pixels; and [`perf`] is the measurement the other
//! two are steered by, which is what turns "laggy" into a number.

pub(crate) mod bringup;
pub(crate) mod config;
pub(crate) mod perf;
pub(crate) mod quality;
pub(crate) mod stage;
