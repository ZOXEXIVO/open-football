//! A loan's option to buy, put to the borrower's board as the purchase it is.

use chrono::NaiveDate;

use crate::club::board::mandate::{MandatePurpose, SigningMandate, TargetBelief};
use crate::club::board::{BoardTransferDecision, BoardTransferProposal};
use crate::club::player::contract::contract::ClubLevelAnchor;
use crate::club::staff::perception::{AbilityEstimator, PotentialEstimator};
use crate::transfers::pipeline::TransferNeedPriority;
use crate::transfers::squad::plan::{BriefTier, MoneySlack};
use crate::transfers::value::upgrade::UpgradeMath;
use crate::{Club, Person, Player};

impl Club {
    /// Appearances at which the board is as sure of a loanee as watching
    /// him can make it.
    const SEASON_WATCHED_APPS: f32 = 10.0;

    /// Hear a loanee's option to buy the way a negotiated signing is heard.
    ///
    /// The purpose is what the club would be buying him for, read from the
    /// role he held here. The fee is weighed against the board's own price
    /// for that purpose, measured against the best of its own men in his
    /// position with loanees left out, and against what it can afford. The
    /// loan season is the evidence: the more of it the club watched, the
    /// less it discounts what it saw. No brief set money aside for the
    /// shirt, so the walk-away is the sporting value alone.
    ///
    /// Returns the proposal heard, whose mandate is the purpose the plan
    /// carries if he signs, and the board's answer.
    pub fn hear_option_to_buy(
        &self,
        player: &Player,
        fee: f64,
        date: NaiveDate,
    ) -> (BoardTransferProposal, BoardTransferDecision) {
        let main = self.teams.main();
        let club_reputation = main.map(|t| t.reputation.world).unwrap_or(0);
        let rep_score = main.map(|t| t.reputation.overall_score()).unwrap_or(0.0);
        let group = player.position().position_group();
        let age = player.age(date);
        let level = AbilityEstimator::observable_level(player);

        let mandate = SigningMandate::unnegotiated(player, club_reputation, date);
        let belief = TargetBelief {
            group,
            tier: BriefTier::B,
            believed_level: level as f32,
            incumbent_level: UpgradeMath::incumbent_level(self, group),
            replacement_level: ClubLevelAnchor::for_reputation(rep_score).rotation_floor(group)
                as f32,
            believed_ceiling: PotentialEstimator::observable_ceiling(player, date) as f32,
            league_gap: 0.0,
            confidence: (player.loan_season_appearances() as f32 / Self::SEASON_WATCHED_APPS)
                .min(1.0),
            age,
            annual_wage: player
                .contract
                .as_ref()
                .map(|c| c.salary as f64)
                .unwrap_or(0.0),
        };
        let envelope = self.board.fee_envelope(
            &mandate,
            &belief,
            &MoneySlack::of(self, date, 0.0, 0.0),
            0.0,
            1.0,
        );

        let proposal = BoardTransferProposal {
            fee,
            allocated_budget: fee,
            remaining_transfer_budget: self
                .finance
                .transfer_budget
                .as_ref()
                .map(|b| b.amount)
                .unwrap_or(self.transfer_plan.total_budget),
            priority: Self::option_priority(mandate.purpose, player.wants_loan_made_permanent()),
            reason: mandate.purpose.request_reason(),
            mandate,
            envelope,
            player_age: Some(age),
            player_ability: Some(level),
            squad_avg_ability: main.map(|t| t.players.current_ability_avg()).unwrap_or(0),
            shortlist_score: 0.0,
            dossier: None,
            economics: None,
        };
        let decision = self.board.hear(&proposal);
        (proposal, decision)
    }

    /// A starter is a shirt the club needs filled; anything less it can do
    /// without. A man keen to stay earns the deal one step more of the
    /// board's stretch.
    fn option_priority(purpose: MandatePurpose, keen_to_stay: bool) -> TransferNeedPriority {
        match (purpose, keen_to_stay) {
            (MandatePurpose::Starter, true) => TransferNeedPriority::Critical,
            (MandatePurpose::Starter, false) | (_, true) => TransferNeedPriority::Important,
            (_, false) => TransferNeedPriority::Optional,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::board::BoardTransferConcern;
    use crate::club::board::mandate::MandateStretch;
    use crate::shared::{Currency, CurrencyValue};
    use crate::transfers::tests::kit::{TestClub, TestDate, TestPlayer};
    use crate::{
        ClubFinancialBalance, HappinessEventType, PlayerClubContract, PlayerPositionType,
        PlayerSquadStatus,
    };
    use chrono::Duration;

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            TestDate::on(2027, 5, 28)
        }

        fn midfielder(id: u32, level: u8, role: PlayerSquadStatus) -> Player {
            TestPlayer::new(id)
                .on(Self::date())
                .age(25)
                .position(PlayerPositionType::MidfielderCenter)
                .position_level(20)
                .ability(level)
                .skills_match_ability()
                .contract_until(30_000, TestDate::on(2029, 6, 30))
                .squad_status(role)
                .build()
        }

        /// A borrower whose own midfielders sit at `own_level`, with a year
        /// of income behind it and room in its transfer budget.
        fn borrower(own_level: u8) -> Club {
            let own = (10..13)
                .map(|id| Self::midfielder(id, own_level, PlayerSquadStatus::FirstTeamRegular))
                .collect();
            let mut club = TestClub::new(200)
                .reputation(5_000)
                .balance(5_000_000)
                .players(own)
                .build();
            let mut month = ClubFinancialBalance::new(0);
            month.income = 3_000_000;
            club.finance
                .history
                .add(Self::date() - Duration::days(30), month);
            club.finance.transfer_budget = Some(CurrencyValue::new(200_000_000.0, Currency::Usd));
            club
        }

        /// A level-140 loanee who held `role` here over `apps` league games.
        fn loanee(role: PlayerSquadStatus, apps: u16) -> Player {
            let mut player = Self::midfielder(55, 140, role);
            player.statistics.played = apps;
            player.statistics.rating_points = 6.9 * apps as f32;
            player.statistics.rating_weight = apps as f32;
            player.contract_loan = Some(PlayerClubContract::new_loan(
                30_000,
                Self::date(),
                100,
                10,
                200,
            ));
            player
        }
    }

    #[test]
    fn a_body_for_the_bench_is_not_bought_above_its_cover_price() {
        let club = Fx::borrower(120);
        let loanee = Fx::loanee(PlayerSquadStatus::MainBackupPlayer, 12);
        let (priced, _) = club.hear_option_to_buy(&loanee, 0.0, Fx::date());
        assert_eq!(priced.mandate.purpose, MandatePurpose::Cover);

        // Past the most any board stretches over its own cover price.
        let fee = priced.envelope.walk_away * MandateStretch::MAX + 500_000.0;
        let (_, decision) = club.hear_option_to_buy(&loanee, fee, Fx::date());
        assert_eq!(
            decision,
            BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
        );
    }

    #[test]
    fn a_starter_the_board_cannot_replace_is_bought() {
        // Its own best midfielder is forty points short of the loanee.
        let club = Fx::borrower(100);
        let loanee = Fx::loanee(PlayerSquadStatus::FirstTeamRegular, 25);
        let (priced, _) = club.hear_option_to_buy(&loanee, 0.0, Fx::date());
        assert_eq!(priced.mandate.purpose, MandatePurpose::Starter);
        assert!(priced.envelope.walk_away > 0.0);

        let fee = priced.envelope.walk_away * 0.5;
        let (_, decision) = club.hear_option_to_buy(&loanee, fee, Fx::date());
        assert!(decision.is_approved(), "{decision:?}");
    }

    #[test]
    fn a_season_watched_is_worth_no_less_than_a_glimpse() {
        let club = Fx::borrower(100);
        let (glimpse, _) = club.hear_option_to_buy(
            &Fx::loanee(PlayerSquadStatus::FirstTeamRegular, 8),
            2_000_000.0,
            Fx::date(),
        );
        let (season, _) = club.hear_option_to_buy(
            &Fx::loanee(PlayerSquadStatus::FirstTeamRegular, 25),
            2_000_000.0,
            Fx::date(),
        );
        assert!(season.envelope.walk_away > 0.0);
        assert!(
            glimpse.envelope.walk_away <= season.envelope.walk_away,
            "eight games {} against twenty-five {}",
            glimpse.envelope.walk_away,
            season.envelope.walk_away
        );
    }

    #[test]
    fn a_player_keen_to_stay_earns_one_step_of_urgency() {
        let club = Fx::borrower(100);
        let mut starter = Fx::loanee(PlayerSquadStatus::FirstTeamRegular, 25);
        let mut cover = Fx::loanee(PlayerSquadStatus::MainBackupPlayer, 25);
        let priority =
            |player: &Player| club.hear_option_to_buy(player, 0.0, Fx::date()).0.priority;
        assert_eq!(priority(&starter), TransferNeedPriority::Important);
        assert_eq!(priority(&cover), TransferNeedPriority::Optional);

        starter
            .happiness
            .add_event(HappinessEventType::WantsLoanMadePermanent, 1.0);
        cover
            .happiness
            .add_event(HappinessEventType::WantsLoanMadePermanent, 1.0);
        assert_eq!(priority(&starter), TransferNeedPriority::Critical);
        assert_eq!(priority(&cover), TransferNeedPriority::Important);
    }
}
