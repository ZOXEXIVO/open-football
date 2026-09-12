//! Starting a working relationship that has run before.
//!
//! Everybody else in the dressing room gets a clean slate when a manager
//! walks in, and that is the point of a clean slate: nobody knows what the
//! new man thinks, so everybody's chances have been reset. For a player he
//! has coached before, none of that is true. The coach knows what he thinks.
//! He has thought it for years.
//!
//! So the hot record is not built from nothing. It is seeded from the
//! dossier, scaled by [`ReunionPrior`] — and the seeding is deliberately
//! partial in a specific way:
//!
//! * **the reads start part of the way to where they were**, not at them,
//! * **the matches watched are credited**, so he is entitled to an opinion
//!   from day one instead of spending six weeks earning one,
//! * **and the re-rating runs slower**, because a man updating a view he
//!   has held for three years is not a man forming a first impression.
//!
//! The last is the one that makes a reunion feel different rather than
//! merely start differently: trust buys a returning player a run of
//! matches, and then it runs out, exactly as it does in a real dressing
//! room.

use super::prior::ReunionPrior;
use super::record::{MedalFlags, PlayerDossier, ScarFlags};
use super::store::Dossiers;
use super::tuning::DossierTuning;
use crate::club::mind::organs::memory::{ActorRef, MindClock};
use crate::club::staff::coach::memory::CoachMemoryFlags;
use crate::club::staff::coach::plan::PlannedRole;
use crate::club::staff::coach::standing::{CoachStanding, GrievanceFlags};
use crate::club::staff::mind::Judgements;
use crate::club::staff::model::staff::Staff;
use crate::club::staff::perception::{CoachProfile, PlayerImpression};
use chrono::NaiveDate;

/// What a reunion did, for the caller that has to tell everybody about it.
#[derive(Debug, Clone, Copy)]
pub struct ReunionSeed {
    /// How much of the old view carried, 0..1.
    pub prior: f32,
    /// How warmly he remembers him, faded to today, −1..=1.
    pub warmth: f32,
    /// What the old grievances still weigh, 0..1.
    pub scar: f32,
    /// Where the seeded standing put him.
    pub standing: f32,
    /// The role his history entitles him to while the coach looks again,
    /// where there is one.
    pub plan_floor: Option<PlannedRole>,
    /// And the role it holds him under, where his history is against him.
    pub plan_ceiling: Option<PlannedRole>,
}

/// Builds a hot record out of an old one.
pub struct ReunionSeeder;

impl ReunionSeeder {
    /// Seed everything the coach carries about a player he has just started
    /// working with again.
    ///
    /// Returns `None` when there is nothing to seed from — which is the
    /// ordinary case, and the reason every caller can run this
    /// unconditionally.
    pub fn seed(staff: &mut Staff, player_id: u32, age: u8, today: NaiveDate) -> Option<ReunionSeed> {
        let day = MindClock::day(today);
        let profile = CoachProfile::from_staff(staff);

        let record = *Dossiers::of(&staff.dossiers, player_id)?;
        if record.spells <= 1 {
            // A first spell has nothing behind it.
            return None;
        }

        let prior = ReunionPrior::compute(&record, age, &profile, day);
        let warmth = record.warmth_now(day);
        let scar = record.scar_now(day);

        Self::seed_memory(staff, &record, player_id, prior, warmth, scar, today);
        Self::seed_impression(staff, &record, player_id, prior, scar, today);
        Self::seed_judgement(staff, &record, player_id, prior, day);
        let (floor, ceiling) = Self::plan_bounds(&record, age, warmth, scar, &profile);
        if let Some(role) = floor.or(ceiling) {
            staff.squad_plan.force_role(player_id, role, today);
        }

        Some(ReunionSeed {
            prior,
            warmth,
            scar,
            standing: staff
                .coach_memory
                .standing_of(player_id)
                .map(|standing| standing.score)
                .unwrap_or(0.0),
            plan_floor: floor,
            plan_ceiling: ceiling,
        })
    }

    /// The form and trust record: part of the way back to where it was.
    fn seed_memory(
        staff: &mut Staff,
        record: &PlayerDossier,
        player_id: u32,
        prior: f32,
        warmth: f32,
        scar: f32,
        today: NaiveDate,
    ) {
        let toward = |axis: f32| 0.5 + (axis - 0.5) * prior;
        // Character reads stick harder than ability reads: whether a man is
        // a professional is not a question a manager re-opens.
        let sticky = prior.max(DossierTuning::SEED_PROFESSIONALISM_MIN_PRIOR);

        let long_form = if record.long_form() > 0.0 {
            record.long_form()
        } else {
            6.7
        };
        let observed = (DossierTuning::SEED_OBSERVATIONS * prior).round() as u16;

        let mut flags = CoachMemoryFlags::default();
        flags.insert(CoachMemoryFlags::KNOWN_QUANTITY);
        if record.medals.contains(MedalFlags::BIG_MATCH_PLAYER) && prior >= 0.4 {
            flags.insert(CoachMemoryFlags::BIG_MATCH_PROVEN);
        }
        if record.scars.contains(ScarFlags::BIG_MATCH_FLOP) && scar >= 0.3 {
            flags.insert(CoachMemoryFlags::BIG_MATCH_FAILED);
        }
        if record
            .medals
            .contains(MedalFlags::CORNERSTONE | MedalFlags::NEVER_LET_ME_DOWN)
            && prior >= 0.5
        {
            flags.insert(CoachMemoryFlags::TRUSTED_CORE);
        }

        // Where he stands before a ball is kicked: what the record said,
        // discounted for the years, plus a little for simply being liked.
        // A grievance he never got past is re-armed — a second chance, and
        // a watched one.
        let mut grievance = GrievanceFlags::default();
        if record.scars.contains(ScarFlags::REFUSED_TO_PLAY)
            && scar >= DossierTuning::SCAR_REARM
        {
            grievance.insert(GrievanceFlags::REFUSED);
        }
        let score = record.standing() * prior + warmth * DossierTuning::SEED_STANDING_WARMTH_BONUS;

        staff.coach_memory.seed_reunion(
            player_id,
            observed,
            prior,
            long_form,
            (
                toward(record.tactical_trust()),
                toward(record.big_match_trust()),
                toward(record.training_trust()),
            ),
            0.5 + (record.professionalism() - 0.5) * sticky,
            flags,
            CoachStanding::seeded(score, grievance, today),
            today,
        );
    }

    /// The week-to-week impression: anchored where it was, so a coach who
    /// rated him does not have to be talked round again.
    fn seed_impression(
        staff: &mut Staff,
        record: &PlayerDossier,
        player_id: u32,
        prior: f32,
        scar: f32,
        today: NaiveDate,
    ) {
        if !staff.decision_state.is_bound() {
            return;
        }
        let mut impression = PlayerImpression::new(player_id, today);
        // `perceived_quality` is on the same 0..20 band the lens produces.
        let quality = record.level() * 20.0;
        impression.perceived_quality = quality;
        impression.potential_impression = record.ceiling() * 20.0;
        impression.bias.first_impression = quality;
        impression.bias.anchored = true;
        if record.medals.contains(MedalFlags::MY_SIGNING) {
            impression.bias.sunk_cost = 2.0 * prior;
        }
        if scar >= 0.4 {
            impression.bias.disappointments = record.scars.count().min(2) as u8;
        }
        staff
            .decision_state
            .impressions
            .insert(player_id, impression);
    }

    /// And the ability read, if the judgement organ has let it go. A view
    /// evicted from a 48-slot store is still a view he holds — the dossier
    /// is the longer-lived record of the two.
    fn seed_judgement(
        staff: &mut Staff,
        record: &PlayerDossier,
        player_id: u32,
        prior: f32,
        day: u16,
    ) {
        let player = ActorRef::player(player_id);
        if staff.mind.judgement_of(player).is_some() {
            return;
        }
        Judgements::form(
            &mut staff.mind.organs.judgements,
            player,
            record.level(),
            record.ceiling(),
            day,
        );
        if let Some(view) = Judgements::of_mut(&mut staff.mind.organs.judgements, player) {
            view.rehydrate(prior);
        }
    }

    /// What his history entitles him to while the coach looks again.
    ///
    /// A floor, not a place: it holds for ninety days and the evidence of
    /// this spell takes over. Which is the honest model — a manager gives
    /// a man he trusts a run, and then the run ends.
    fn plan_bounds(
        record: &PlayerDossier,
        age: u8,
        warmth: f32,
        scar: f32,
        profile: &CoachProfile,
    ) -> (Option<PlannedRole>, Option<PlannedRole>) {
        // Against him: a poisonous history, and a coach stubborn enough to
        // hold it.
        if record.scars.contains(ScarFlags::WANTED_OUT | ScarFlags::REFUSED_TO_PLAY)
            && scar >= 0.3
            && profile.stubbornness >= DossierTuning::REUNION_STUBBORN
        {
            return (None, Some(PlannedRole::Cover));
        }
        // For him: warmth, a real role last time, and legs.
        if warmth >= DossierTuning::REUNION_PLAN_WARMTH
            && age <= DossierTuning::REUNION_PLAN_MAX_AGE
        {
            if let Some(last) = record.last_role() {
                if last.is_at_least(PlannedRole::Rotation) {
                    return (Some(last.demoted()), None);
                }
            }
        }
        (None, None)
    }
}
