//! Where a player stands with his manager.
//!
//! The coach layer could already read a player's form, his trust, his role
//! fit and his risk, and fold all of it into a selection score. What it
//! could not do was *hold a position*. Every week's team was re-derived from
//! the current numbers, so a manager's displeasure lasted exactly as long as
//! the moving average that caused it, and a player could be dropped on a
//! Tuesday and restored on a Saturday without anything having happened.
//!
//! Real managers decide, remember having decided, and want more evidence to
//! reverse a decision than they needed to make it.
//!
//! ```text
//! StandingEvidence  a match, a card, a talk, a run in training → an impulse
//!        ↓                                      scaled by who the coach is
//! CoachStanding     a continuous score, and the decision it has hardened into
//!        ↓                                    with a minimum stay on each rung
//! StandingRead      what selection and the squad plan do about it
//! ```
//!
//! Undroppable · Trusted · InFavour · Neutral · UnderReview · OutOfFavour ·
//! FrozenOut. Entering a rung is easier than leaving it, in both directions,
//! and the gap between the two thresholds is where a manager's mind is made
//! up.
//!
//! The standing lives inside [`CoachMemory`] and dies with it. It is a
//! reading of a live working relationship; when that ends it is consolidated
//! into a [`PlayerDossier`] rather than carried around.
//!
//! [`CoachMemory`]: crate::club::staff::coach::CoachMemory
//! [`PlayerDossier`]: crate::club::staff::coach::PlayerDossier

pub mod evidence;
pub mod ladder;
pub mod outcome;
pub mod tuning;

pub use evidence::{EvidenceLens, StandingEvidence};
pub use ladder::{CoachStanding, GrievanceFlags, LadderContext, StandingLadder, StandingRung};
pub use outcome::{StandingOutcome, StandingRead};
pub use tuning::StandingTuning;
