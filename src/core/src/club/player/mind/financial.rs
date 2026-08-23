//! The financial mind — what he thinks he is worth.
//!
//! The faculty that is almost never about the money in absolute terms.
//! A player on a fortune who finds out a teammate earns more is unhappy;
//! a player on a modest wage who believes he is treated fairly is not.
//! So this faculty holds a *ratio* — his own sense of his worth against
//! what he is actually paid — and an envy term that reads the dressing
//! room rather than the bank.
//!
//! It also holds the second lever a career runs on: security. A man with
//! eighteen months left and no conversation about a new deal is in a
//! different state from one with four years, whatever either is earning.

use super::organs::MindOrgans;
use super::organs::goals::{GoalDomain, GoalEvidence, GoalKind, GoalOrigin};
use super::organs::memory::{ActorRef, EpisodeKind, FactClaim, MindEpisode};
use super::submind::{MindView, MoodContribution, SubMind};

/// His sense of what he is owed.
#[derive(Debug, Clone, Copy, Default)]
pub struct FinancialMind {
    /// −100..=100. How fairly he believes he is paid: negative when he
    /// thinks he is worth more than he gets.
    fairness_pct: i8,
    /// −100..=100. What he thinks of what other people are on.
    envy_pct: i8,
    /// Times the club has said no to him.
    pub refusals: u8,
}

impl FinancialMind {
    pub const SHIFT: f32 = 0.16;

    /// Fairness below which he actively wants it put right.
    pub const AGGRIEVED: f32 = -0.25;

    /// Contract pressure above which security starts to matter more than
    /// the number. Roughly the last eighteen months.
    pub const INSECURE: f32 = 0.25;

    #[inline]
    pub fn fairness(&self) -> f32 {
        self.fairness_pct as f32 / 100.0
    }

    #[inline]
    pub fn envy(&self) -> f32 {
        self.envy_pct as f32 / 100.0
    }

    fn shift_fairness(&mut self, delta: f32) {
        let value = (self.fairness() + delta).clamp(-1.0, 1.0);
        self.fairness_pct = (value * 100.0).round() as i8;
    }

    fn shift_envy(&mut self, delta: f32) {
        let value = (self.envy() + delta).clamp(-1.0, 1.0);
        self.envy_pct = (value * 100.0).round() as i8;
    }

    /// How aggrieved he is overall, 0..1 — what he thinks he is owed,
    /// sharpened by what he thinks other people are getting.
    pub fn grievance(&self) -> f32 {
        let owed = (-self.fairness()).clamp(0.0, 1.0);
        let envy = self.envy().clamp(0.0, 1.0);
        (owed * 0.7 + envy * 0.3).clamp(0.0, 1.0)
    }

    /// A new deal settles the account.
    pub fn on_new_contract(&mut self) {
        self.fairness_pct = 0;
        self.envy_pct = 0;
        self.refusals = 0;
    }
}

impl SubMind for FinancialMind {
    fn domain(&self) -> GoalDomain {
        GoalDomain::Financial
    }

    fn observe(&mut self, episode: &MindEpisode, _organs: &mut MindOrgans) {
        match episode.kind {
            EpisodeKind::BigPayRise => {
                self.shift_fairness(Self::SHIFT * 2.0);
                self.shift_envy(-Self::SHIFT);
            }
            EpisodeKind::ContractRenewed => self.on_new_contract(),
            EpisodeKind::WageBelowPeers => {
                self.shift_fairness(-Self::SHIFT);
                self.shift_envy(Self::SHIFT * 1.5);
            }
            EpisodeKind::ClubRefusedTerms => {
                self.shift_fairness(-Self::SHIFT);
                self.refusals = self.refusals.saturating_add(1);
            }
            EpisodeKind::ClubBrokeWagePromise => {
                self.shift_fairness(-Self::SHIFT * 2.0);
                self.refusals = self.refusals.saturating_add(1);
            }
            // Standing feeds worth. A man who has just won the league or
            // been made captain revises what he thinks he is owed, and
            // this is where most real wage disputes actually start.
            EpisodeKind::WonLeagueTitle
            | EpisodeKind::WonContinentalTrophy
            | EpisodeKind::CaptaincyAwarded => self.shift_fairness(-Self::SHIFT * 0.5),
            _ => {}
        }
    }

    fn reflect(&mut self, view: &MindView<'_>, organs: &mut MindOrgans) {
        let s = view.situation;
        let today = view.today();

        if !s.is_settled() {
            return;
        }

        let contract_pressure = s.contract_pressure();

        // ── Being paid what he thinks he is worth ───────────────
        if self.fairness() < Self::AGGRIEVED {
            let mut evidence = GoalEvidence::of(&[GoalEvidence::PAID_BELOW_HIS_PEERS]);
            if self.refusals > 0 {
                evidence.insert(GoalEvidence::TERMS_REFUSED);
            }
            if contract_pressure > Self::INSECURE {
                evidence.insert(GoalEvidence::CONTRACT_RUNNING_DOWN);
            }

            organs.goals.pursue(
                GoalKind::BePaidWhatImWorth,
                GoalOrigin::Grievance,
                evidence,
                self.grievance(),
                today,
            );
            // The leverage is in the last year of a deal, and he knows it.
            organs
                .goals
                .set_urgency(GoalKind::BePaidWhatImWorth, contract_pressure);
        } else if self.fairness() > 0.1 {
            organs.goals.advance(GoalKind::BePaidWhatImWorth, 0.3);
        }

        // ── Security ────────────────────────────────────────────
        //
        // A deal running down is its own want, separate from the number
        // on it, and it sharpens as the runway shortens: a 33-year-old
        // with a year left is in a different position from a 24-year-old
        // in the same situation.
        if contract_pressure > Self::INSECURE {
            let exposure = contract_pressure * (0.5 + 0.5 * s.career_spent());
            organs.goals.pursue(
                GoalKind::SecureMyFuture,
                GoalOrigin::Survival,
                GoalEvidence::of(&[GoalEvidence::CONTRACT_RUNNING_DOWN]),
                exposure,
                today,
            );
            organs
                .goals
                .set_urgency(GoalKind::SecureMyFuture, contract_pressure);
        }

        // ── Repeatedly told no ──────────────────────────────────
        //
        // Being refused is not the same as being underpaid. A club that
        // keeps saying no, to a player who believes a conviction has
        // formed about it, is a club he stops trying with.
        let here = ActorRef::club(view.tick.club_id);
        let broke_its_word = organs.memory.believes(FactClaim::ClubBrokeItsWord, here);
        if self.refusals >= 3 || broke_its_word > 0.4 {
            organs.goals.pursue(
                GoalKind::BeAllowedToLeave,
                GoalOrigin::Grievance,
                GoalEvidence::of(&[GoalEvidence::TERMS_REFUSED]),
                (self.refusals as f32 / 5.0).clamp(0.2, 1.0),
                today,
            );
        }
    }

    fn appraise(&self, _organs: &MindOrgans) -> MoodContribution {
        // Fairness is the axis; envy adds to the sting but never makes a
        // fairly-paid man unhappy on its own.
        let value = self.fairness() * 5.0 - self.envy().max(0.0) * 3.0;
        let confidence = if self.fairness_pct == 0 && self.envy_pct == 0 {
            0.25
        } else {
            0.85
        };
        MoodContribution::new(GoalDomain::Financial, value, confidence)
    }
}

#[cfg(test)]
mod tests {
    use super::super::MindTickContext;
    use super::super::situation::MindSituation;
    use super::*;
    use crate::club::person::PersonAttributes;
    use chrono::NaiveDate;

    const CLUB: u32 = 7;

    fn attrs() -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition: 12.0,
            controversy: 5.0,
            loyalty: 12.0,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 10.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn reflect(mind: &mut FinancialMind, situation: &MindSituation, organs: &mut MindOrgans) {
        let tick = MindTickContext::new(
            NaiveDate::from_ymd_opt(2030, 6, 1).unwrap(),
            CLUB,
            &attrs(),
            50.0,
        );
        let view = MindView {
            tick: &tick,
            situation,
        };
        mind.reflect(&view, organs);
    }

    fn episode(kind: EpisodeKind) -> MindEpisode {
        MindEpisode::new(
            kind,
            ActorRef::club(CLUB),
            CLUB,
            100,
            kind.spec().valence,
            0.8,
        )
    }

    fn settled() -> MindSituation {
        MindSituation {
            days_at_club: 500,
            ..MindSituation::neutral()
        }
    }

    #[test]
    fn a_fairly_paid_player_wants_nothing() {
        let mut mind = FinancialMind::default();
        let mut organs = MindOrgans::new();
        reflect(&mut mind, &settled(), &mut organs);
        assert!(organs.goals.is_empty());
    }

    #[test]
    fn finding_out_what_a_teammate_earns_is_what_starts_it() {
        let mut organs = MindOrgans::new();
        let mut mind = FinancialMind::default();
        for _ in 0..3 {
            mind.observe(&episode(EpisodeKind::WageBelowPeers), &mut organs);
        }
        assert!(mind.envy() > 0.0);

        reflect(&mut mind, &settled(), &mut organs);
        assert!(organs.goals.pressure_of(GoalKind::BePaidWhatImWorth) > 0.0);
    }

    #[test]
    fn winning_things_makes_a_man_revise_what_he_is_owed() {
        let mut organs = MindOrgans::new();
        let mut mind = FinancialMind::default();
        assert_eq!(mind.fairness(), 0.0);

        mind.observe(&episode(EpisodeKind::WonLeagueTitle), &mut organs);
        mind.observe(&episode(EpisodeKind::CaptaincyAwarded), &mut organs);
        assert!(
            mind.fairness() < 0.0,
            "this is where most real wage disputes start"
        );
    }

    #[test]
    fn a_new_deal_settles_the_account() {
        let mut organs = MindOrgans::new();
        let mut mind = FinancialMind::default();
        for _ in 0..4 {
            mind.observe(&episode(EpisodeKind::ClubRefusedTerms), &mut organs);
        }
        assert!(mind.grievance() > 0.0);
        assert!(mind.refusals > 0);

        mind.observe(&episode(EpisodeKind::ContractRenewed), &mut organs);
        assert_eq!(mind.grievance(), 0.0);
        assert_eq!(mind.refusals, 0);
    }

    #[test]
    fn the_leverage_is_in_the_last_year_and_he_knows_it() {
        let build = |days_left: u16| {
            let mut organs = MindOrgans::new();
            let mut mind = FinancialMind::default();
            for _ in 0..3 {
                mind.observe(&episode(EpisodeKind::WageBelowPeers), &mut organs);
            }
            reflect(
                &mut mind,
                &MindSituation {
                    contract_days_left: days_left,
                    ..settled()
                },
                &mut organs,
            );
            organs
                .goals
                .get(GoalKind::BePaidWhatImWorth)
                .map(|g| g.urgency())
                .unwrap_or(0.0)
        };

        assert!(build(120) > build(700));
    }

    #[test]
    fn a_deal_running_down_is_a_want_of_its_own() {
        let mut mind = FinancialMind::default();
        let mut organs = MindOrgans::new();
        reflect(
            &mut mind,
            &MindSituation {
                contract_days_left: 200,
                ..settled()
            },
            &mut organs,
        );
        assert!(organs.goals.pressure_of(GoalKind::SecureMyFuture) > 0.0);
        assert_eq!(
            organs.goals.pressure_of(GoalKind::BePaidWhatImWorth),
            0.0,
            "security is not the same as money"
        );
    }

    #[test]
    fn security_matters_more_to_a_man_running_out_of_career() {
        let build = |age: u8| {
            let mut mind = FinancialMind::default();
            let mut organs = MindOrgans::new();
            reflect(
                &mut mind,
                &MindSituation {
                    age,
                    contract_days_left: 200,
                    ..settled()
                },
                &mut organs,
            );
            organs.goals.pressure_of(GoalKind::SecureMyFuture)
        };
        assert!(build(33) > build(24));
    }

    #[test]
    fn being_told_no_repeatedly_is_a_different_grievance_from_being_underpaid() {
        let mut organs = MindOrgans::new();
        let mut mind = FinancialMind::default();
        for _ in 0..3 {
            mind.observe(&episode(EpisodeKind::ClubRefusedTerms), &mut organs);
        }
        reflect(&mut mind, &settled(), &mut organs);
        assert!(organs.goals.pressure_of(GoalKind::BeAllowedToLeave) > 0.0);
    }

    #[test]
    fn a_new_signing_is_not_yet_arguing_about_money() {
        let mut organs = MindOrgans::new();
        let mut mind = FinancialMind::default();
        for _ in 0..4 {
            mind.observe(&episode(EpisodeKind::WageBelowPeers), &mut organs);
        }
        reflect(
            &mut mind,
            &MindSituation {
                days_at_club: 30,
                ..settled()
            },
            &mut organs,
        );
        assert!(organs.goals.is_empty());
    }

    #[test]
    fn envy_alone_never_makes_a_fairly_paid_man_miserable() {
        let mut mind = FinancialMind::default();
        let organs = MindOrgans::new();
        mind.shift_fairness(0.4);
        mind.shift_envy(0.3);
        assert!(
            mind.appraise(&organs).weighted() > -1.0,
            "he is well paid and knows it, whatever anyone else is on"
        );
    }

    #[test]
    fn a_broken_wage_promise_cuts_deeper_than_a_refusal() {
        let mut organs = MindOrgans::new();

        let mut refused = FinancialMind::default();
        refused.observe(&episode(EpisodeKind::ClubRefusedTerms), &mut organs);

        let mut betrayed = FinancialMind::default();
        betrayed.observe(&episode(EpisodeKind::ClubBrokeWagePromise), &mut organs);

        assert!(betrayed.grievance() > refused.grievance());
    }
}
