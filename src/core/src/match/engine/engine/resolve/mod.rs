//! **What the tick does to the ball between subsystem passes.**
//!
//! The ball layer integrates one ball and knows nothing about the other
//! twenty-one players; the player layer moves one man at a time and may
//! not rewrite the ball. Everything in this group is work that needs both
//! at once, so the ball stages an intent and the engine — the only place
//! that holds `&mut MatchField` — finishes it a phase later.
//!
//! * [`set_piece`] — the pending restart teleport, and the corner shape's
//!   station lifecycle from arming to sweep.
//! * [`corner`] — the discrete corner aerial contest.
//! * [`cross`] — the same for an open-play cross.
//! * [`delivery`] — the arm both contests finish through: flying the ball
//!   to the man who won it, or hooking it behind, plus the apex / drop
//!   constants that separate a corner's trajectory from a cross's.
//! * [`save_credit`] — the keeper / shooter stat pair the physics save
//!   left behind.
//!
//! Every number in here is calibration-critical: the corner win rate, the
//! cross completion rate and the save credit are all tracked run to run.

pub mod corner;
pub mod cross;
pub mod delivery;
pub mod save_credit;
pub mod set_piece;
