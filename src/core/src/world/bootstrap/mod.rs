//! Bringing a freshly-loaded world up to a state the tick can run on,
//! and catching up anything created since.
//!
//! Three passes, each on its own type: [`ClubIdentity`] answers "which
//! badge does this squad's career history hang under", [`WorldSeeder`]
//! writes the rows that identity implies (league tables, career
//! histories, id sequences), and [`PassportOffice`] stamps the
//! nationality fields every market gate reads.

mod identity;
mod passports;
mod seeder;

pub use identity::ClubIdentity;
