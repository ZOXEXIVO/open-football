//! The stadium, which is there before the teams are.
//!
//! [`field`] is the one place the engine's grid and the world's metres meet,
//! and everything else here is built off it: the [`pitch`] and the stands
//! around it, the [`net`] the ball deforms, and the [`sky`] the whole thing
//! sits under.

pub(crate) mod field;
pub(crate) mod net;
pub(crate) mod pitch;
pub(crate) mod sky;
