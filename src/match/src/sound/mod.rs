//! What the ball sounds like.
//!
//! Deliberately nothing else. There is no crowd, no whistle and no
//! commentary: the one thing a replay gains from being audible is knowing
//! **when the ball was touched and how hard**, which is the fact the picture
//! is worst at carrying — a pass and a shot look much the same from a
//! broadcast camera for the first few frames, and sound tells them apart
//! instantly.
//!
//! A crowd was tried and taken out again. Synthesised stadium noise is
//! filtered noise, filtered noise is weather, and a bed of it sat on top of
//! the very transients this exists to deliver.
//!
//! Two halves, and the split is the same one the rest of the crate makes
//! between the browser and the replay:
//!
//! - [`mixer`] is the instrument — an audio graph built out of the browser's
//!   own oscillators and filters, which knows how to make the sound of
//!   something being struck and nothing whatever about football.
//! - [`matchday`] is the player. It reads the same facts everything else in
//!   the viewer is drawn from — where the ball is, when it is about to be hit,
//!   how fast it leaves — and turns them into calls on the instrument.
//!
//! # Why nothing is downloaded
//!
//! For the reason [`crate::art`] paints its own textures: a replay that had to
//! fetch a set of foley samples would either play silently for the first ten
//! seconds or hold the page until they landed, and the recordings themselves
//! are already a megabyte and a half per five minutes of match. Everything
//! here is synthesised in the browser out of a few seconds of noise the module
//! generates for itself, so the whole soundtrack costs no bytes over the wire
//! and nothing in the wasm artefact but the code that builds it.
//!
//! It also means the crate stays free of `bevy_audio`, which is deliberately
//! absent from the feature list in `Cargo.toml` — it would drag in a decoder
//! stack and an asset pipeline to play files that do not exist.
//!
//! # The browser has the last word
//!
//! No page may make a noise before somebody has interacted with it, so the
//! [`AudioContext`](web_sys::AudioContext) is not opened until the replay is
//! actually running, and is resumed on every frame it is found suspended. A
//! viewer who never presses play never creates one; a browser that refuses
//! outright leaves the replay silent and unharmed.

pub(crate) mod matchday;
pub(crate) mod mixer;
