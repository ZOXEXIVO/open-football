//! What a coach carries between clubs about the players he has worked with.
//!
//! The coach layer already had four per-player stores and every one of them
//! was about *this* Saturday: [`CoachMemory`] (form and trust), the
//! impression, the squad plan, and — the exception — the judgement organ,
//! which holds an ability read that survives the job.
//!
//! None of them answered the question a manager answers instantly about
//! anyone he has ever coached: *what is he to me?* Two seasons or two
//! matches, what he did on the night that counted, how it ended, and whether
//! I would have him again. That is what lives here.
//!
//! ```text
//! spell opens     →  hot record (CoachMemory + CoachStanding)
//!                       ↓ the spell ends
//! SpellCloser     →  PlayerDossier + convictions + an episode
//!                       ↓ years pass, and then they meet again
//! ReunionPrior    →  how much of it he still trusts
//! ReunionSeeder   →  a fresh hot record, started from what he knew
//! ```
//!
//! Three properties the existing stores did not have, and each is the point
//! of a module here:
//!
//! * **It is bounded.** [`CoachDossierStore`] holds
//!   [`DossierTuning::CAPACITY`] records and evicts the one he would miss
//!   least. `CoachMemory` grew without limit and never forgot anybody,
//!   which is not memory, it is a log.
//! * **It records the *shape* of a working relationship**, not a rating:
//!   how long, how warm, what he did to me, what he did for me, and how we
//!   parted.
//! * **It is re-usable.** A record is not a souvenir — it seeds the next
//!   spell, weighted by how long ago it was and how much of the man the
//!   coach actually saw.
//!
//! [`CoachMemory`]: crate::club::staff::coach::CoachMemory

pub mod closer;
pub mod prior;
pub mod record;
pub mod seeder;
pub mod store;
pub mod tuning;

#[cfg(test)]
mod tests;

pub use closer::{PartingReport, SpellCloser};
pub use prior::ReunionPrior;
pub use seeder::{ReunionSeed, ReunionSeeder};
pub use record::{MedalFlags, PlayerDossier, ScarFlags, SeparationCause};
pub use store::{CoachDossierStore, DossierCensus, Dossiers, SpellOpening};
pub use tuning::DossierTuning;
