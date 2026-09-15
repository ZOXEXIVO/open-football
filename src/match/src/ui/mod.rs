//! The controls drawn over the picture, and the one thing up there that is not
//! a control.
//!
//! [`timeline`] is the transport bar along the bottom — the scrub with the
//! recording's holes marked on it, the clock, the speed and the debug chips.
//! [`touch`] is the same controls for a device with no mouse to give them.
//! [`scoreboard`] is the score as the playhead has reached it, which is the
//! only thing on the screen that says a goal has been given. [`watermark`] is
//! the one thing up there that is about the project rather than the match.

pub(crate) mod plate;
pub(crate) mod scoreboard;
pub(crate) mod teamsheet;
pub(crate) mod timeline;
pub(crate) mod touch;
pub(crate) mod watermark;
