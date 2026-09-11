//! The press runs.
//!
//! Two papers go out on different clocks and from different altitudes.
//! [`ClubNewsroomTick`] prints every branded side's own edition on a
//! Monday morning, written from inside a dressing room and taking a
//! side. [`LeagueNewsroomTick`] prints the divisions' own papers on the
//! first of the month, written from above the whole division with no
//! side to take.
//!
//! The desks, stories and issue types they file into belong to the
//! clubs and leagues that keep them — [`crate::club::news`] and
//! [`crate::league::news`]. What lives here is only the world-level
//! gathering: both runs are laid out gather-then-write, so all the
//! reading happens in parallel over an immutable world and only the
//! finished editions are applied under `&mut`.

mod club;
mod league;

pub(crate) use club::ClubNewsroomTick;
pub(crate) use league::LeagueNewsroomTick;
