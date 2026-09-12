//! Turning a working relationship into a record of one.
//!
//! Everything the coach was carrying about a player while they worked
//! together — the form averages, the trust axes, the standing, the
//! impression, the plan entry — is about picking him on Saturday, and once
//! there is no Saturday it is not an opinion any more, it is a leak. So it
//! is read once, consolidated into a [`PlayerDossier`], and dropped.
//!
//! What the closer writes is deliberately *not* a summary of the numbers.
//! It is the four things a manager actually retains about a player he no
//! longer has:
//!
//! * how long they were together and what he made of him,
//! * what the man did to him and for him,
//! * how it ended,
//! * and whether, on balance, he would have him again.
//!
//! The last of those is [`PlayerDossier::warmth`], and everything a reunion
//! does turns on it.

use super::record::{MedalFlags, PlayerDossier, ScarFlags, SeparationCause};
use super::store::Dossiers;
use super::tuning::DossierTuning;
use crate::club::mind::organs::memory::{ActorRef, EpisodeKind, FactClaim, MindClock};
use crate::club::staff::coach::memory::{CoachMemory, CoachMemoryFlags};
use crate::club::staff::coach::plan::PlannedRole;
use crate::club::staff::coach::standing::{GrievanceFlags, StandingRung};
use crate::club::staff::model::staff::Staff;
use crate::club::staff::perception::CoachProfile;
use chrono::NaiveDate;

/// What the closer needs to know that it cannot read off the coach.
///
/// Gathered by the caller from the player, because the closer runs at the
/// moment a player is being taken out of a squad and the two of them cannot
/// both be borrowed.
#[derive(Debug, Clone, Copy)]
pub struct PartingReport {
    pub player_id: u32,
    pub club_id: u32,
    pub age: u8,
    /// How the player's own relations read the coach, 0..1, before the
    /// coach's guess is blurred into it.
    pub his_regard: f32,
    /// Promises this coach made him and kept, and made and broke.
    pub promises_kept: u8,
    pub promises_broken: u8,
    /// Observable level now, 0..1 — what the coach can see he became.
    pub level_now: f32,
    /// Whether he wore the armband under this coach for long enough to
    /// count.
    pub was_my_captain: bool,
    /// Observable level when the spell opened, 0..1, where the coach has a
    /// judgement that remembers it.
    pub level_at_start: f32,
    /// Serious injuries and days lost during the spell.
    pub injury_days: u16,
    /// Red cards and disciplinary events during the spell.
    pub red_cards: u8,
    pub discipline_events: u8,
    /// Mistakes that led to goals during the spell.
    pub errors: u8,
}

impl PartingReport {
    /// A report with nothing in it but the identity — for the callers that
    /// genuinely know nothing more, and so every field has an honest
    /// default rather than a plausible-looking guess.
    pub fn bare(player_id: u32, club_id: u32, age: u8) -> Self {
        PartingReport {
            player_id,
            club_id,
            age,
            his_regard: 0.5,
            promises_kept: 0,
            promises_broken: 0,
            level_now: 0.5,
            was_my_captain: false,
            level_at_start: 0.5,
            injury_days: 0,
            red_cards: 0,
            discipline_events: 0,
            errors: 0,
        }
    }
}

/// Closes a spell and writes what is left of it.
pub struct SpellCloser;

impl SpellCloser {
    /// Injury days inside one spell at which a coach stops counting on a
    /// player being available.
    const FRAGILE_DAYS: u16 = 120;
    /// Mistakes that make a reputation.
    const ERROR_PRONE_COUNT: u8 = 3;
    const ERROR_PRONE_WITHIN_MATCHES: u16 = 30;
    /// Cards and rows that make a different one.
    const INDISCIPLINE_COUNT: u8 = 2;
    /// Matches together, with nothing held against him, that make a man a
    /// coach would vouch for.
    const NEVER_LET_ME_DOWN_MATCHES: u16 = 30;
    const NEVER_LET_ME_DOWN_STANDING: f32 = 0.30;
    /// Observable points of improvement under one coach that make him say
    /// he made the player.
    const MADE_UNDER_ME_GAIN: f32 = 0.12;
    const MADE_UNDER_ME_AGE: u8 = 24;
    /// Confidence in a conviction asserted at parting.
    const CONVICTION_STRENGTH: f32 = 0.55;

    /// Close the spell and consolidate everything into the dossier.
    ///
    /// Returns the episode the coach should record about it, if the parting
    /// is one he would remember at all — most are not.
    pub fn close(
        staff: &mut Staff,
        report: &PartingReport,
        cause: SeparationCause,
        today: NaiveDate,
    ) -> Option<EpisodeKind> {
        // A loan is not a parting. He is still the manager's player, the
        // manager is simply not watching him this season, and the reports
        // will be read when he comes back. Suspending rather than closing
        // is what makes a season out on loan a chapter of one working
        // relationship instead of the end of one.
        if cause == SeparationCause::LoanedOutByMe {
            Self::suspend(staff, report.player_id);
            return None;
        }
        if !Dossiers::is_working_with(&staff.dossiers, report.player_id) {
            // Nothing open: either they never worked together, or the store
            // was full when they started. Either way there is nothing to
            // close, and the hot state is cleared anyway so a departed
            // player cannot linger in a plan.
            Self::discard_hot_state(staff, report.player_id);
            return None;
        }

        let profile = CoachProfile::from_staff(staff);
        let temperament = staff.attributes.temperament;
        let loyalty = staff.attributes.loyalty;

        let memory = staff.coach_memory.get(report.player_id).copied_view();
        let plan_role = staff.squad_plan.role_of(report.player_id);
        let judgement = staff
            .mind
            .judgement_of(ActorRef::player(report.player_id))
            .map(|view| (view.level(), view.ceiling()));

        let Some(record) = Dossiers::of_mut(&mut staff.dossiers, report.player_id) else {
            Self::discard_hot_state(staff, report.player_id);
            return None;
        };

        // ── What he concluded the man was ──
        let long_form = memory.long_form.max(0.0);
        let (level, ceiling) = judgement.unwrap_or_else(|| {
            // No judgement on file: read the level off his own baseline,
            // where 5.0 is a passenger and 8.0 is a very good player. The
            // same mapping the post-match dispatch uses, and CA-blind.
            let read = ((long_form - 5.0) / 3.0).clamp(0.0, 1.0);
            (read, read)
        });
        record.set_reads(
            level,
            ceiling,
            long_form,
            memory.tactical_trust,
            memory.big_match_trust,
            memory.training_trust,
            memory.professionalism,
        );
        record.set_standing(memory.standing_score);
        record.add_matches(memory.matches_this_spell);
        if let Some(role) = plan_role {
            record.note_role(role);
        }
        record.promises_kept = record.promises_kept.saturating_add(report.promises_kept);
        record.promises_broken = record
            .promises_broken
            .saturating_add(report.promises_broken);

        // ── What he did to me, and for me ──
        Self::mark_scars(record, &memory, report, cause);
        Self::mark_medals(record, &memory, report, plan_role);
        record.refresh_scar_strength(temperament);

        // ── How I read that he felt about me ──
        // A guess, blurred by how well this coach reads a dressing room,
        // and deterministic: the same coach reading the same player on the
        // same day always guesses the same way.
        let noise = profile.perception_noise(report.player_id, 0xD05_5E1)
            * DossierTuning::HIS_STANCE_NOISE
            * (1.0 - profile.man_management.clamp(0.0, 1.0));
        let stance = ((report.his_regard.clamp(0.0, 1.0) - 0.5) * 2.0 + noise).clamp(-1.0, 1.0);
        record.set_his_stance(stance);

        // ── And whether I would have him again ──
        let warmth = Self::warmth(record, &memory, cause, stance, loyalty, temperament);
        record.set_warmth(warmth);

        record.close(cause, report.club_id, report.age, MindClock::day(today));

        let scars = record.scars;
        let medals = record.medals;
        let standing = memory.standing_score;
        let matches = record.matches_together;

        Self::conclude(staff, report, scars, medals, level, today);
        Self::discard_hot_state(staff, report.player_id);
        Self::parting_episode(cause, standing, scars, medals, matches)
    }

    /// Scars are what a coach still says about a player years later, so the
    /// bar for each is a body of evidence rather than one bad afternoon.
    fn mark_scars(
        record: &mut PlayerDossier,
        memory: &MemoryView,
        report: &PartingReport,
        cause: SeparationCause,
    ) {
        if memory.grievance.contains(GrievanceFlags::BIG_MATCH_UNTRUSTED)
            || memory.flags.contains(CoachMemoryFlags::BIG_MATCH_FAILED)
        {
            record.scars.insert(ScarFlags::BIG_MATCH_FLOP);
        }
        if memory.grievance.contains(GrievanceFlags::COST_THE_OCCASION) {
            record.scars.insert(ScarFlags::COST_US_THE_OCCASION);
        }
        if report.errors >= Self::ERROR_PRONE_COUNT
            && memory.matches_this_spell <= Self::ERROR_PRONE_WITHIN_MATCHES
        {
            record.scars.insert(ScarFlags::ERROR_PRONE);
        }
        if report.red_cards >= Self::INDISCIPLINE_COUNT
            || report.discipline_events >= Self::INDISCIPLINE_COUNT
            || memory.grievance.contains(GrievanceFlags::INDISCIPLINE)
        {
            record.scars.insert(ScarFlags::INDISCIPLINE);
        }
        if memory.grievance.contains(GrievanceFlags::REFUSED) {
            record.scars.insert(ScarFlags::REFUSED_TO_PLAY);
        }
        if memory.grievance.contains(GrievanceFlags::WENT_PUBLIC) {
            record.scars.insert(ScarFlags::WENT_PUBLIC);
        }
        // Asking to leave is only a grievance when the coach wanted him.
        if (memory.grievance.contains(GrievanceFlags::WANTS_OUT) || cause.is_his_choice())
            && memory.standing_score >= DossierTuning::WALKED_OUT_STANDING
        {
            record.scars.insert(ScarFlags::WANTED_OUT);
        }
        if report.injury_days >= Self::FRAGILE_DAYS {
            record.scars.insert(ScarFlags::FRAGILE);
        }
        if memory.training_trust < 0.35 {
            record.scars.insert(ScarFlags::TRAINING_SLACKER);
        }
        if memory.rung == StandingRung::FrozenOut {
            record.scars.insert(ScarFlags::I_FROZE_HIM_OUT);
        }
    }

    fn mark_medals(
        record: &mut PlayerDossier,
        memory: &MemoryView,
        report: &PartingReport,
        plan_role: Option<PlannedRole>,
    ) {
        if memory.flags.contains(CoachMemoryFlags::BIG_MATCH_PROVEN) {
            record.medals.insert(MedalFlags::BIG_MATCH_PLAYER);
        }
        if report.was_my_captain {
            record.medals.insert(MedalFlags::MY_CAPTAIN);
        }
        if record.peak_role() == Some(PlannedRole::Cornerstone)
            || plan_role == Some(PlannedRole::Cornerstone)
        {
            record.medals.insert(MedalFlags::CORNERSTONE);
        }
        if report.level_now - report.level_at_start >= Self::MADE_UNDER_ME_GAIN
            && report.age <= Self::MADE_UNDER_ME_AGE + 4
        {
            record.medals.insert(MedalFlags::MADE_UNDER_ME);
        }
        if memory.matches_this_spell >= Self::NEVER_LET_ME_DOWN_MATCHES
            && record.scars.is_empty()
            && memory.standing_score >= Self::NEVER_LET_ME_DOWN_STANDING
        {
            record.medals.insert(MedalFlags::NEVER_LET_ME_DOWN);
        }
        if memory.bounced_back {
            record.medals.insert(MedalFlags::BOUNCED_BACK);
        }
        if memory.repaid_faith {
            record.medals.insert(MedalFlags::REPAID_FAITH);
        }
    }

    /// Whether, on balance, he would have him again.
    fn warmth(
        record: &PlayerDossier,
        memory: &MemoryView,
        cause: SeparationCause,
        his_stance: f32,
        loyalty: f32,
        temperament: f32,
    ) -> f32 {
        let medals = (record.medals.count() as f32 / DossierTuning::WARMTH_MEDALS_FULL).min(1.0);
        let scars = (record.scars.weight() * DossierTuning::temperament_scale(temperament)).min(1.0);

        let mut parting = cause.warmth_term(memory.standing_score);
        if parting < 0.0 {
            parting *= DossierTuning::loyalty_scale(loyalty);
        }

        (memory.standing_score * DossierTuning::WARMTH_W_STANDING
            + (memory.professionalism - 0.5) * 2.0 * DossierTuning::WARMTH_W_PROFESSIONALISM
            + medals * DossierTuning::WARMTH_W_MEDALS
            - scars * DossierTuning::WARMTH_W_SCARS
            + his_stance * DossierTuning::WARMTH_W_HIS_STANCE
            + parting)
            .clamp(-1.0, 1.0)
    }

    /// The one or two things about the man he would still assert years
    /// later, filed where every other conviction lives.
    fn conclude(
        staff: &mut Staff,
        report: &PartingReport,
        scars: ScarFlags,
        medals: MedalFlags,
        level: f32,
        today: NaiveDate,
    ) {
        let player = ActorRef::player(report.player_id);
        if medals.contains(MedalFlags::CORNERSTONE | MedalFlags::NEVER_LET_ME_DOWN) && level >= 0.70
        {
            staff
                .mind
                .conclude(FactClaim::HeIsWorthBuildingAround, player, Self::CONVICTION_STRENGTH, today);
        }
        if medals.contains(MedalFlags::REPAID_FAITH) {
            staff
                .mind
                .conclude(FactClaim::HeRepaidMyFaith, player, Self::CONVICTION_STRENGTH, today);
        }
        if scars.contains(ScarFlags::POISONOUS) || scars.contains(ScarFlags::WANTED_OUT) {
            staff
                .mind
                .conclude(FactClaim::HeLetMeDown, player, Self::CONVICTION_STRENGTH, today);
        }
    }

    /// Whether this parting is one he would remember, and as what.
    fn parting_episode(
        cause: SeparationCause,
        standing: f32,
        scars: ScarFlags,
        medals: MedalFlags,
        matches: u16,
    ) -> Option<EpisodeKind> {
        match cause {
            // The board cashing in a man he was building around is a fact
            // about the board, and it is the one this episode exists for.
            SeparationCause::SoldByBoard
                if standing >= DossierTuning::WALKED_OUT_STANDING
                    && medals.contains(MedalFlags::CORNERSTONE | MedalFlags::NEVER_LET_ME_DOWN) =>
            {
                Some(EpisodeKind::BoardSoldMyBestPlayer)
            }
            SeparationCause::HeRequestedOut | SeparationCause::HeRanDownHisContract
                if scars.contains(ScarFlags::WANTED_OUT) =>
            {
                Some(EpisodeKind::PlayerWalkedOutOnMe)
            }
            // A long, clean spell that ends with both of them content is
            // worth remembering too, and it is what makes a coach ring him
            // when he needs somebody he can rely on.
            SeparationCause::SoldOnMyCall
            | SeparationCause::ReleasedOnMyCall
            | SeparationCause::HeRetired
                if scars.is_empty() && matches >= Self::NEVER_LET_ME_DOWN_MATCHES =>
            {
                Some(EpisodeKind::PlayerLeftWithMyBlessing)
            }
            _ => None,
        }
    }

    /// He has gone out on loan. The record stays, the plan does not — a
    /// plan is about a squad he is picking from, and this season the
    /// player is not in it.
    fn suspend(staff: &mut Staff, player_id: u32) {
        if let Some(memory) = staff.coach_memory.get_mut(player_id) {
            memory.flags.insert(CoachMemoryFlags::AWAY_ON_LOAN);
        }
        staff.squad_plan.remove(player_id);
    }

    /// He is back, with a season of football behind him that the coach did
    /// not watch.
    ///
    /// A manager reads the reports; he does not see the games. So the loan
    /// enters his record as evidence at a fraction of the weight of a match
    /// he was at — enough to move a read of a young player materially over
    /// thirty appearances, and not enough to pretend he saw them.
    pub fn resume_from_loan(
        staff: &mut Staff,
        player_id: u32,
        appearances: u16,
        average_rating: f32,
        today: NaiveDate,
    ) {
        let Some(memory) = staff.coach_memory.get_mut(player_id) else {
            return;
        };
        memory.flags.remove(CoachMemoryFlags::AWAY_ON_LOAN);
        if appearances == 0 || average_rating <= 0.0 {
            return;
        }
        let weight = DossierTuning::LOAN_REPORT_WEIGHT;
        let credited = ((appearances as f32) * weight).round().max(1.0) as u16;
        // Move the long-form baseline toward what he actually did, at the
        // rate a body of second-hand evidence deserves.
        let pull = (credited as f32 / (credited as f32 + 8.0)).clamp(0.0, 0.6);
        memory.long_form_rating =
            memory.long_form_rating * (1.0 - pull) + average_rating.clamp(0.0, 10.0) * pull;
        memory.recent_rating_ema = memory.long_form_rating;
        memory.matches_observed = memory.matches_observed.saturating_add(credited);
        memory.last_observed_date = Some(today);
    }

    /// Drop everything that was about picking him on Saturday.
    fn discard_hot_state(staff: &mut Staff, player_id: u32) {
        staff.coach_memory.forget(player_id);
        staff.decision_state.impressions.remove(&player_id);
        staff.squad_plan.remove(player_id);
    }
}

/// A flattened read of the hot record, taken before it is dropped.
///
/// Exists so the closer can borrow the dossier mutably without still
/// holding the memory: `Staff` owns both, and they are read in one order
/// and written in another.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryView {
    pub long_form: f32,
    pub tactical_trust: f32,
    pub big_match_trust: f32,
    pub training_trust: f32,
    pub professionalism: f32,
    pub standing_score: f32,
    pub rung: StandingRung,
    pub grievance: GrievanceFlags,
    pub flags: CoachMemoryFlags,
    pub matches_this_spell: u16,
    /// He was out of the side under this coach and played his way back in.
    pub bounced_back: bool,
    /// The coach picked him against the evidence and he answered.
    pub repaid_faith: bool,
}

/// Lifts the handful of fields the closer needs out of a borrowed record.
trait CoachMemoryView {
    fn copied_view(self) -> MemoryView;
}

impl CoachMemoryView for Option<&CoachMemory> {
    fn copied_view(self) -> MemoryView {
        let Some(memory) = self else {
            return MemoryView {
                tactical_trust: 0.5,
                big_match_trust: 0.5,
                training_trust: 0.5,
                professionalism: 0.5,
                ..MemoryView::default()
            };
        };
        MemoryView {
            long_form: memory.long_form_rating,
            tactical_trust: memory.tactical_trust,
            big_match_trust: memory.big_match_trust,
            training_trust: memory.training_trust,
            professionalism: memory.professionalism_read,
            standing_score: memory.standing.score,
            rung: memory.standing.rung,
            grievance: memory.standing.grievance,
            flags: memory.flags,
            matches_this_spell: memory.matches_this_spell(),
            bounced_back: memory.standing.bounced_back,
            // The protection a coach extends after being repaid is the
            // only durable trace of it on the hot record.
            repaid_faith: memory.standing.protected_until.is_some(),
        }
    }
}
