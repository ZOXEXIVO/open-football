//! The club's development pathway, player by player.
//!
//! Every other squad pass in this folder answers a question about one
//! moment — is he surplus today, is he idle today, is the keeper room
//! balanced today. None of them holds an intention, so a club could send
//! the same boy out three seasons running and never once ask what the
//! loans were FOR, and a returnee's spell was read by the news desk and by
//! nobody who decides anything.
//!
//! [`PathwayReview`] is that intention. One monthly pass sets a
//! [`PathwayStage`] on every contracted player and a date it is looked at
//! again; [`Club::on_loanee_returned`] is the other entry point, because a
//! spell ending is a decision point whatever the calendar says. The stages
//! are read downstream as a factor — the loan agreement's parent
//! willingness, the asset ledger's sale motive, the squad page — never as
//! a gate.

use chrono::NaiveDate;

use crate::club::academy::AcademyDevelopmentIdentity;
use crate::club::board::vision::VisionYouthFocus;
use crate::club::player::statistics::StuckCareerScan;
use crate::club::staff::perception::{AbilityEstimator, PotentialEstimator};
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::deal::offer::PromisedSquadStatus;
use crate::transfers::pipeline::{
    LoanDestinationPreference, LoanOutCandidate, LoanOutReason, LoanOutStatus,
};
use crate::transfers::squad::LevelBand;
use crate::transfers::value::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{
    Club, ClubLevelAnchor, ClubPhilosophy, LoanSpellVerdict, PathwayStage, Person, Player,
    PlayerFieldPositionGroup, PlayerPlan, PlayerPlanRole, ReputationLevel, Team, TeamType,
    TransferItem,
};

use super::depth::PromotionBar;

/// What the club can see about one player when it reviews his pathway.
///
/// Every field is a visible signal: the coach-observable level, the
/// observable ceiling, the minutes he has had, the deal he is on. Hidden
/// potential is never read here — the pathway is a decision a staff makes,
/// on the evidence a staff has.
#[derive(Debug, Clone, Copy)]
pub(in crate::club::core) struct PathwaySignals {
    pub band: f32,
    pub upside_gap: i16,
    pub runway: f32,
    pub starter_share: f32,
    pub contract_days: i64,
    /// His observable level clears the first team's promotion floor.
    pub clears_promotion_bar: bool,
    pub rank_in_group: u8,
    pub gap_to_best: i16,
}

/// The monthly pathway pass, and the verdict tables the two event-driven
/// entry points share with it.
pub(in crate::club::core) struct PathwayReview;

impl PathwayReview {
    /// Band at or above which the club reads him as one of the men the
    /// team is built around.
    const CORE_BAND: f32 = 1.15;
    /// … a starter …
    const STARTER_BAND: f32 = 0.85;
    /// … and squad depth that plays. Below it he is not first-team
    /// quality for this club at all.
    const ROTATION_BAND: f32 = 0.35;
    /// Share of starts at which the club reads him as actually in the
    /// side, rather than merely good enough to be.
    const PLAYING_SHARE: f32 = 0.45;
    /// Observable points of believed growth that make a below-band player
    /// a prospect rather than surplus.
    const PROSPECT_UPSIDE: i16 = 8;
    /// Runway at or above which a man below the club's level is still
    /// worth developing. Continuous everywhere else; this is the point
    /// where developing and moving on cross.
    const DEVELOP_RUNWAY: f32 = 0.45;
    /// Contract days inside which an unresolved pathway becomes a sale
    /// rather than a plan — the club is out of time to be patient.
    const SHORT_CONTRACT_DAYS: i64 = 400;
    /// Age share of a career at which a `DevelopAndSell` club starts
    /// reading a good player as a fee.
    const PEAK_SALE_SPENT: f32 = 0.45;
    /// Group rank, and observable gap to the man ahead, at which a
    /// standout returnee is plainly behind better men for good.
    const BLOCKED_RANK: u8 = 3;
    const BLOCKED_GAP: i16 = 8;
    /// Loans a club will arrange before a player's answer is the market.
    const MAX_LOANS: u8 = 2;

    /// Read a player as the club sees him.
    pub(in crate::club::core) fn signals(
        player: &Player,
        date: NaiveDate,
        anchor: &ClubLevelAnchor,
        bar: &PromotionBar,
        rank_in_group: u8,
        best_in_group: u8,
    ) -> PathwaySignals {
        let group = player.position().position_group();
        let level = AbilityEstimator::observable_level(player);
        let ceiling = PotentialEstimator::observable_ceiling(player, date);
        let age = player.age(date);
        PathwaySignals {
            band: LevelBand::of(level, group, anchor),
            upside_gap: ceiling as i16 - level as i16,
            runway: ((34.0 - age as f32) / 12.0).clamp(0.0, 1.0),
            starter_share: player.happiness.starter_ratio,
            contract_days: player
                .contract
                .as_ref()
                .map(|c| (c.expiration - date).num_days())
                .unwrap_or(0),
            clears_promotion_bar: level >= bar.floor(group),
            rank_in_group,
            gap_to_best: best_in_group as i16 - level as i16,
        }
    }

    /// The stage a player the club has never written a pathway for starts
    /// on. Derived from what it already believes about him, so nothing is
    /// invented on the first pass over an existing squad.
    pub(in crate::club::core) fn opening_stage(
        signals: &PathwaySignals,
        is_youth_squad: bool,
    ) -> PathwayStage {
        if is_youth_squad && !signals.clears_promotion_bar {
            return PathwayStage::Academy;
        }
        Self::standing_stage(signals)
    }

    /// Where a player's own level and minutes put him, with nothing else
    /// in the picture. The spine of every table below.
    fn standing_stage(signals: &PathwaySignals) -> PathwayStage {
        if signals.band >= Self::CORE_BAND && signals.starter_share >= Self::PLAYING_SHARE {
            PathwayStage::Core
        } else if signals.band >= Self::STARTER_BAND {
            PathwayStage::Starter
        } else if signals.band >= Self::ROTATION_BAND {
            PathwayStage::Rotation
        } else if signals.runway >= Self::DEVELOP_RUNWAY
            && signals.upside_gap >= Self::PROSPECT_UPSIDE
        {
            PathwayStage::Prospect
        } else {
            PathwayStage::MoveOn
        }
    }

    /// The monthly verdict for one player already on a pathway.
    ///
    /// Reads the same continuous signals at every stage rather than
    /// branching on which one he is at: where his level puts him, what the
    /// club is for, and how much time either side has left.
    pub(in crate::club::core) fn next_stage(
        current: PathwayStage,
        signals: &PathwaySignals,
        plan: &PlayerPlan,
        philosophy: &ClubPhilosophy,
    ) -> PathwayStage {
        // A staged loan is a decision the club has already taken and has
        // not yet been able to act on. It resolves when the spell ends or
        // the purpose is cleared, not on a calendar month.
        if current == PathwayStage::LoanOut {
            return PathwayStage::LoanOut;
        }

        let standing = Self::standing_stage(signals);

        // Out of contract runway with nothing settled: the club is past
        // being patient, whatever it once meant him to become.
        if signals.contract_days > 0
            && signals.contract_days < Self::SHORT_CONTRACT_DAYS
            && !standing.is_first_team()
        {
            return PathwayStage::MoveOn;
        }

        // A man who is not going to play here, young enough for a season
        // elsewhere to be a career step and with loans left in him, goes
        // out rather than sitting. This is the club's half of the same
        // sentence the player's `ProveOnLoan` arc says.
        if matches!(standing, PathwayStage::Prospect)
            && plan.loans_used < Self::MAX_LOANS
            && signals.starter_share < Self::PLAYING_SHARE
        {
            return PathwayStage::LoanOut;
        }

        // Selling at peak is a philosophy applied to a good player with
        // his best years behind the horizon the club is trading against —
        // never a category of player, and never a club that does not
        // trade.
        if standing.is_first_team()
            && Self::sells_at_peak(philosophy)
            && 1.0 - signals.runway >= Self::PEAK_SALE_SPENT
        {
            return PathwayStage::SellAtPeak;
        }

        standing
    }

    /// The verdict table a finished spell resolves to — the club's read of
    /// what the loan was worth, and what it does next.
    ///
    /// Returns the stage, the purpose of a second loan when that is the
    /// answer, and the shirt the club promises if it is promoting him.
    pub(in crate::club::core) fn after_loan(
        verdict: LoanSpellVerdict,
        signals: &PathwaySignals,
        plan: &PlayerPlan,
        philosophy: &ClubPhilosophy,
    ) -> LoanReturnVerdict {
        let blocked =
            signals.rank_in_group >= Self::BLOCKED_RANK && signals.gap_to_best >= Self::BLOCKED_GAP;
        match verdict {
            LoanSpellVerdict::Standout => {
                if blocked && Self::sells_at_peak(philosophy) {
                    return LoanReturnVerdict::sell(PathwayStage::SellAtPeak, 1.4);
                }
                let stage = if signals.clears_promotion_bar || signals.band >= Self::STARTER_BAND {
                    PathwayStage::Starter
                } else {
                    PathwayStage::Rotation
                };
                LoanReturnVerdict::promote(
                    stage,
                    PromisedSquadStatus::FirstTeamSquadRotation,
                    PlayerPlan::PROMISE_REVIEW_DAYS,
                )
            }
            LoanSpellVerdict::Successful => LoanReturnVerdict::promote(
                PathwayStage::Rotation,
                PromisedSquadStatus::HotProspectForTheFuture,
                120,
            ),
            LoanSpellVerdict::Steady => {
                if signals.runway >= 0.6 {
                    LoanReturnVerdict::loan_again(LoanOutReason::NeedsFirstTeamMinutes)
                } else {
                    LoanReturnVerdict::sell(PathwayStage::MoveOn, 0.9)
                }
            }
            LoanSpellVerdict::Peripheral | LoanSpellVerdict::Struggled => {
                if signals.runway >= 0.7
                    && plan.loans_used < Self::MAX_LOANS
                    && signals.upside_gap >= Self::BLOCKED_GAP
                {
                    LoanReturnVerdict::loan_again(LoanOutReason::BlockedByDepth)
                } else {
                    LoanReturnVerdict::sell(PathwayStage::MoveOn, 0.7)
                }
            }
            LoanSpellVerdict::Inconclusive => LoanReturnVerdict::hold(),
        }
    }

    /// Does this club trade players as a matter of policy?
    fn sells_at_peak(philosophy: &ClubPhilosophy) -> bool {
        matches!(philosophy, ClubPhilosophy::DevelopAndSell)
    }
}

/// What the club decided about a returning loanee.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::club::core) struct LoanReturnVerdict {
    /// `None` holds the stage he was on.
    pub stage: Option<PathwayStage>,
    pub promise: Option<PromisedSquadStatus>,
    pub loan_purpose: Option<LoanOutReason>,
    /// Multiple of his value the club would take for him, when the
    /// decision is a sale.
    pub asking_multiple: Option<f32>,
    pub review_in_days: i64,
}

impl LoanReturnVerdict {
    fn promote(stage: PathwayStage, promise: PromisedSquadStatus, review_in_days: i64) -> Self {
        LoanReturnVerdict {
            stage: Some(stage),
            promise: Some(promise),
            loan_purpose: None,
            asking_multiple: None,
            review_in_days,
        }
    }

    fn loan_again(purpose: LoanOutReason) -> Self {
        LoanReturnVerdict {
            stage: Some(PathwayStage::Reassess),
            promise: None,
            loan_purpose: Some(purpose),
            asking_multiple: None,
            review_in_days: PlayerPlan::SHORT_REVIEW_DAYS,
        }
    }

    fn sell(stage: PathwayStage, asking_multiple: f32) -> Self {
        LoanReturnVerdict {
            stage: Some(stage),
            promise: None,
            loan_purpose: None,
            asking_multiple: Some(asking_multiple),
            review_in_days: PlayerPlan::SHORT_REVIEW_DAYS,
        }
    }

    fn hold() -> Self {
        LoanReturnVerdict {
            stage: None,
            promise: None,
            loan_purpose: None,
            asking_multiple: None,
            review_in_days: PlayerPlan::SHORT_REVIEW_DAYS,
        }
    }
}

impl ClubPhilosophy {
    /// What the club is FOR, derived from the brief its board actually
    /// wrote and the academy it actually runs.
    ///
    /// Reputation alone mapped three of the four onto a ladder and never
    /// produced the fourth at all, so no club in the world traded players
    /// as a policy. Youth focus, academy standard and the academy's own
    /// identity are the signals that separate a Benfica from a Chelsea at
    /// the same reputation.
    pub fn derive(
        youth_focus: VisionYouthFocus,
        academy_tier: u8,
        identity: AcademyDevelopmentIdentity,
        rep_level: ReputationLevel,
    ) -> Self {
        /// Academy standard at which a club can actually supply its own
        /// first team, and therefore has something to sell.
        const TRADING_ACADEMY_TIER: u8 = 6;

        let trades = matches!(identity, AcademyDevelopmentIdentity::PlayerTrading)
            || (matches!(youth_focus, VisionYouthFocus::DevelopYouth)
                && academy_tier >= TRADING_ACADEMY_TIER
                && matches!(
                    rep_level,
                    ReputationLevel::National | ReputationLevel::Continental
                ));
        if trades {
            return ClubPhilosophy::DevelopAndSell;
        }
        match rep_level {
            ReputationLevel::Elite if matches!(youth_focus, VisionYouthFocus::SignExperienced) => {
                ClubPhilosophy::SignToCompete
            }
            ReputationLevel::Elite | ReputationLevel::Continental | ReputationLevel::National => {
                ClubPhilosophy::Balanced
            }
            _ => ClubPhilosophy::LoanFocused,
        }
    }
}

impl Club {
    /// Re-derive what the club is for. The board writes its brief on the
    /// first tick and rewrites it at every takeover, so the philosophy is
    /// read from it rather than frozen at world load.
    pub(in crate::club::core) fn review_philosophy(&mut self) {
        let rep_level = self
            .teams
            .main()
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur);
        self.philosophy = ClubPhilosophy::derive(
            self.board.vision.youth_focus,
            self.academy.tier().value(),
            self.academy.development_identity,
            rep_level,
        );
    }

    /// The monthly look at every contracted player: is he still on the
    /// pathway the club put him on, and what is the next rung?
    pub(in crate::club::core) fn review_pathways(&mut self, date: NaiveDate) {
        let Some(main_idx) = self.teams.main_index() else {
            return;
        };
        let anchor =
            ClubLevelAnchor::for_reputation(self.teams.teams[main_idx].reputation.overall_score());
        let bar = PromotionBar::snapshot(&self.teams.teams[main_idx]);
        let philosophy = self.philosophy;
        let club_id = self.id;

        for team_idx in 0..self.teams.teams.len() {
            let is_youth_squad = self.teams.teams[team_idx].team_type.is_youth();
            let ranks = Self::group_standing(&self.teams.teams[team_idx]);
            for player in self.teams.teams[team_idx].players.players.iter_mut() {
                if player.contract.is_none() || player.is_on_loan() {
                    continue;
                }
                let (rank, best) = ranks
                    .iter()
                    .find(|(g, _, _)| *g == player.position().position_group())
                    .map(|(_, best, levels)| {
                        let level = AbilityEstimator::observable_level(player);
                        let ahead = levels.iter().filter(|&&l| l > level).count() as u8;
                        (ahead, *best)
                    })
                    .unwrap_or((0, 0));
                let signals = PathwayReview::signals(player, date, &anchor, &bar, rank, best);

                let Some(plan) = player.plan.as_ref() else {
                    let role = Self::role_for_existing(&signals);
                    let stage = PathwayReview::opening_stage(&signals, is_youth_squad);
                    let mut fresh = PlayerPlan::from_existing(role, stage, date);
                    fresh.loans_used = StuckCareerScan::of(player, date)
                        .map(|scan| if scan.serial_loanee { 2 } else { 0 })
                        .unwrap_or(0);
                    player.assign_pathway(club_id, fresh, date);
                    continue;
                };
                if !plan.review_due(date) {
                    continue;
                }
                let next = PathwayReview::next_stage(plan.stage, &signals, plan, &philosophy);
                let from = plan.stage;
                let review_days = if next == from {
                    PlayerPlan::REVIEW_DAYS
                } else {
                    PlayerPlan::SHORT_REVIEW_DAYS
                };
                if let Some(plan) = player.plan.as_mut() {
                    plan.move_to(next, date, review_days);
                }
                if next != from {
                    player.on_pathway_stage_changed(club_id, Some(from), next, date);
                }
            }
        }
    }

    /// A loan has ended and the player is back. The parent reads the
    /// spell and says what he is now — which is the decision the blanket
    /// move to the reserves used to make for it.
    pub fn on_loanee_returned(
        &mut self,
        player_id: u32,
        verdict: LoanSpellVerdict,
        date: NaiveDate,
    ) {
        let Some(main_idx) = self.teams.main_index() else {
            return;
        };
        let anchor =
            ClubLevelAnchor::for_reputation(self.teams.teams[main_idx].reputation.overall_score());
        let bar = PromotionBar::snapshot(&self.teams.teams[main_idx]);
        let philosophy = self.philosophy;
        let club_id = self.id;
        // The same seller context the valuation and every other sell-side
        // reading resolve, so a pathway listing quotes the market's own
        // number rather than a second one.
        let (seller_league_rep, seller_club_rep) =
            PlayerValuationCalculator::seller_context_from_club(self);

        let Some(team_idx) = self
            .teams
            .teams
            .iter()
            .position(|t| t.players.iter().any(|p| p.id == player_id))
        else {
            return;
        };
        let ranks = Self::group_standing(&self.teams.teams[team_idx]);
        let decision = {
            let Some(player) = self.teams.teams[team_idx].players.find(player_id) else {
                return;
            };
            let (rank, best) = ranks
                .iter()
                .find(|(g, _, _)| *g == player.position().position_group())
                .map(|(_, best, levels)| {
                    let level = AbilityEstimator::observable_level(player);
                    (levels.iter().filter(|&&l| l > level).count() as u8, *best)
                })
                .unwrap_or((0, 0));
            let signals = PathwayReview::signals(player, date, &anchor, &bar, rank, best);
            let owned;
            let plan = match player.plan.as_ref() {
                Some(plan) => plan,
                None => {
                    owned = PlayerPlan::from_existing(
                        PlayerPlanRole::Development,
                        PathwayStage::Reassess,
                        date,
                    );
                    &owned
                }
            };
            PathwayReview::after_loan(verdict, &signals, plan, &philosophy)
        };

        let Some(player) = self.teams.teams[team_idx].players.find_mut(player_id) else {
            return;
        };
        let from = player.plan.as_ref().map(|p| p.stage);
        if player.plan.is_none() {
            player.plan = Some(PlayerPlan::from_existing(
                PlayerPlanRole::Development,
                PathwayStage::Reassess,
                date,
            ));
        }
        if let Some(plan) = player.plan.as_mut() {
            plan.last_verdict = Some(verdict);
            plan.loans_used = plan.loans_used.saturating_add(1);
            match decision.stage {
                Some(stage) => {
                    plan.move_to(stage, date, decision.review_in_days);
                    plan.loan_purpose = decision.loan_purpose;
                }
                None => plan.defer_review(date, decision.review_in_days),
            }
        }
        if let (Some(stage), true) = (decision.stage, decision.stage != from) {
            player.on_pathway_stage_changed(club_id, from, stage, date);
        }
        if let Some(promise) = decision.promise {
            player.on_squad_role_promised(promise, date, decision.review_in_days);
        }
        let asking = decision.asking_multiple.map(|multiple| {
            player.on_listed_by_club("dec_reason_pathway_exhausted", date);
            player.value(date, seller_league_rep, seller_club_rep) * multiple as f64
        });

        // The price the verdict named. A listing with no number on it
        // is a badge, and the market reads rows.
        if let Some(asking) = asking {
            self.teams.teams[main_idx]
                .transfer_list
                .add(TransferItem::new(
                    player_id,
                    CurrencyValue::new(FormattingUtils::round_fee(asking), Currency::Usd),
                ));
        }

        // A second loan is the club's own decision, so it is staged as one
        // rather than waiting for a sweep to rediscover him.
        if let Some(purpose) = decision.loan_purpose {
            self.on_pathway_loan_staged(player_id, purpose, date);
        }
        // Whether he stays on the first-team roster is the consequence of
        // the verdict, not of the calendar.
        let keep_on_main = decision.stage.map(|s| s.is_first_team()).unwrap_or(false);
        if !keep_on_main && team_idx == main_idx {
            self.demote_to_reserve(player_id, date);
        }
    }

    /// The club has decided to send this man out, and why. The single
    /// producer of a [`LoanOutCandidate`] on the club side, so the purpose
    /// survives all the way to the borrower instead of collapsing to "lack
    /// of playing time".
    pub fn on_pathway_loan_staged(
        &mut self,
        player_id: u32,
        purpose: LoanOutReason,
        date: NaiveDate,
    ) {
        let club_id = self.id;
        let Some(team_idx) = self
            .teams
            .teams
            .iter()
            .position(|t| t.players.iter().any(|p| p.id == player_id))
        else {
            return;
        };
        if let Some(player) = self.teams.teams[team_idx].players.find_mut(player_id) {
            let from = player.plan.as_ref().map(|p| p.stage);
            match player.plan.as_mut() {
                Some(plan) => {
                    plan.move_to(PathwayStage::LoanOut, date, PlayerPlan::REVIEW_DAYS);
                    plan.loan_purpose = Some(purpose);
                }
                None => {
                    let mut plan = PlayerPlan::from_existing(
                        PlayerPlanRole::Development,
                        PathwayStage::LoanOut,
                        date,
                    );
                    plan.loan_purpose = Some(purpose);
                    player.plan = Some(plan);
                }
            }
            if from != Some(PathwayStage::LoanOut) {
                player.on_pathway_stage_changed(club_id, from, PathwayStage::LoanOut, date);
            }
        }

        if let Some(existing) = self
            .transfer_plan
            .loan_out_candidates
            .iter_mut()
            .find(|c| c.player_id == player_id)
        {
            existing.reason = purpose;
            return;
        }
        let preference = self.teams.teams[team_idx]
            .players
            .find(player_id)
            .map(Player::loan_destination_preference)
            .unwrap_or(LoanDestinationPreference::Any);
        self.transfer_plan
            .loan_out_candidates
            .push(LoanOutCandidate {
                player_id,
                reason: purpose,
                status: LoanOutStatus::Identified,
                loan_fee: 0.0,
                preferred_destination: preference,
            });
    }

    /// A player has been sold. The man behind him moves up a rung — the
    /// succession the club was holding him for.
    pub fn on_asset_sold(&mut self, player_id: u32, date: NaiveDate) {
        let club_id = self.id;
        let Some(main_idx) = self.teams.main_index() else {
            return;
        };
        let Some(group) = self
            .teams
            .iter()
            .flat_map(|t| t.players.iter())
            .find(|p| p.id == player_id)
            .map(|p| p.position().position_group())
        else {
            return;
        };
        // The best man at the position the club already reads as a
        // successor — a rung below the first team, not another starter.
        let heir = self
            .teams
            .teams
            .iter()
            .flat_map(|t| t.players.iter())
            .filter(|p| {
                p.id != player_id
                    && p.contract.is_some()
                    && !p.is_on_loan()
                    && p.position().position_group() == group
                    && !p.pathway_stage().is_terminal()
                    && !matches!(
                        p.pathway_stage(),
                        PathwayStage::Starter | PathwayStage::Core
                    )
            })
            .max_by_key(|p| AbilityEstimator::observable_level(p))
            .map(|p| p.id);
        let Some(heir) = heir else {
            return;
        };
        let _ = main_idx;
        for team in self.teams.teams.iter_mut() {
            if let Some(player) = team.players.find_mut(heir) {
                let from = player.plan.as_ref().map(|p| p.stage);
                let to = from.unwrap_or(PathwayStage::Prospect).promoted();
                match player.plan.as_mut() {
                    Some(plan) => {
                        plan.move_to(to, date, PlayerPlan::SHORT_REVIEW_DAYS);
                    }
                    None => {
                        player.plan = Some(PlayerPlan::from_existing(
                            PlayerPlanRole::CompeteForStarting,
                            to,
                            date,
                        ));
                    }
                }
                if from != Some(to) {
                    player.on_pathway_stage_changed(club_id, from, to, date);
                }
                break;
            }
        }
    }

    /// Move one player off the first-team roster onto the reserve side.
    /// The consequence of a pathway decision, never a weekly sweep.
    fn demote_to_reserve(&mut self, player_id: u32, date: NaiveDate) {
        let Some(main_idx) = self.teams.main_index() else {
            return;
        };
        let Some(reserve_idx) = self
            .teams
            .index_of_type(TeamType::Reserve)
            .or_else(|| self.teams.index_of_type(TeamType::B))
            .or_else(|| self.teams.index_of_type(TeamType::Second))
        else {
            return;
        };
        let from_info = self.teams.teams[main_idx].history_info();
        let to_info = self.teams.teams[reserve_idx].history_info();
        let from_senior = self.teams.teams[main_idx].team_type.is_own_team();
        let to_senior = self.teams.teams[reserve_idx].team_type.is_own_team();
        if let Some(mut player) = self.teams.teams[main_idx].players.take_player(&player_id) {
            if player.is_force_match_selection {
                self.teams.teams[main_idx].players.add(player);
                return;
            }
            player.on_intra_club_move(&from_info, &to_info, from_senior, to_senior, date);
            self.teams.teams[reserve_idx].players.add(player);
        }
    }

    /// Per-group `(group, best observable level, every level)` for one
    /// squad — the depth chart the pathway reads standing off.
    fn group_standing(team: &Team) -> Vec<(PlayerFieldPositionGroup, u8, Vec<u8>)> {
        PlayerFieldPositionGroup::ALL
            .iter()
            .map(|&group| {
                let levels: Vec<u8> = team
                    .players
                    .iter()
                    .filter(|p| p.position().position_group() == group && !p.is_on_loan())
                    .map(AbilityEstimator::observable_level)
                    .collect();
                let best = levels.iter().copied().max().unwrap_or(0);
                (group, best, levels)
            })
            .collect()
    }

    /// The signing role the club would have written for a man it already
    /// has — for the one pass that backfills a pathway onto an existing
    /// squad.
    fn role_for_existing(signals: &PathwaySignals) -> PlayerPlanRole {
        if signals.band >= PathwayReview::STARTER_BAND {
            PlayerPlanRole::ImmediateStarter
        } else if signals.band >= PathwayReview::ROTATION_BAND {
            PlayerPlanRole::DepthRotation
        } else if signals.runway >= PathwayReview::DEVELOP_RUNWAY {
            PlayerPlanRole::Development
        } else {
            PlayerPlanRole::DepthRotation
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, PlayerSquadStatus, StaffCollection, TeamBuilder, TeamCollection,
        TeamReputation, TrainingSchedule,
    };
    use chrono::NaiveTime;

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
        }

        fn signals(band: f32, runway: f32) -> PathwaySignals {
            PathwaySignals {
                band,
                upside_gap: 12,
                runway,
                starter_share: 0.2,
                contract_days: 900,
                clears_promotion_bar: band >= 0.85,
                rank_in_group: 1,
                gap_to_best: 4,
            }
        }

        fn plan(loans_used: u8) -> PlayerPlan {
            let mut plan = PlayerPlan::from_existing(
                PlayerPlanRole::Development,
                PathwayStage::LoanOut,
                Self::date(),
            );
            plan.loans_used = loans_used;
            plan
        }

        fn player(id: u32, ca: u8, age: u8) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = ca.saturating_add(20);
            attrs.condition = 10_000;
            let mut contract =
                PlayerClubContract::new(20_000, NaiveDate::from_ymd_opt(2031, 6, 30).unwrap());
            contract.squad_status = PlayerSquadStatus::FirstTeamSquadRotation;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("P".into(), format!("{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2026 - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ca))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        fn club(players: Vec<Player>) -> Club {
            let team = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("t".into())
                .slug("t".into())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            )
        }
    }

    #[test]
    fn a_standout_spell_brings_him_back_into_the_reckoning() {
        let verdict = PathwayReview::after_loan(
            LoanSpellVerdict::Standout,
            &Fx::signals(0.95, 0.8),
            &Fx::plan(1),
            &ClubPhilosophy::Balanced,
        );
        assert_eq!(verdict.stage, Some(PathwayStage::Starter));
        assert!(
            verdict.promise.is_some(),
            "a standout returnee is promised a shirt"
        );
    }

    #[test]
    fn a_standout_nobody_can_get_past_is_cashed_in_by_a_trading_club() {
        let mut signals = Fx::signals(0.95, 0.8);
        signals.rank_in_group = 3;
        signals.gap_to_best = 10;
        let verdict = PathwayReview::after_loan(
            LoanSpellVerdict::Standout,
            &signals,
            &Fx::plan(1),
            &ClubPhilosophy::DevelopAndSell,
        );
        assert_eq!(verdict.stage, Some(PathwayStage::SellAtPeak));
    }

    #[test]
    fn a_steady_spell_buys_another_season_away_while_there_is_time_for_one() {
        let young = PathwayReview::after_loan(
            LoanSpellVerdict::Steady,
            &Fx::signals(0.4, 0.8),
            &Fx::plan(1),
            &ClubPhilosophy::Balanced,
        );
        assert_eq!(
            young.loan_purpose,
            Some(LoanOutReason::NeedsFirstTeamMinutes)
        );

        let older = PathwayReview::after_loan(
            LoanSpellVerdict::Steady,
            &Fx::signals(0.4, 0.3),
            &Fx::plan(1),
            &ClubPhilosophy::Balanced,
        );
        assert_eq!(older.stage, Some(PathwayStage::MoveOn));
    }

    #[test]
    fn a_failed_spell_is_a_second_chance_once_and_the_market_after_that() {
        let first = PathwayReview::after_loan(
            LoanSpellVerdict::Struggled,
            &Fx::signals(0.3, 0.9),
            &Fx::plan(0),
            &ClubPhilosophy::Balanced,
        );
        assert_eq!(first.loan_purpose, Some(LoanOutReason::BlockedByDepth));

        let exhausted = PathwayReview::after_loan(
            LoanSpellVerdict::Struggled,
            &Fx::signals(0.3, 0.9),
            &Fx::plan(2),
            &ClubPhilosophy::Balanced,
        );
        assert_eq!(exhausted.stage, Some(PathwayStage::MoveOn));
    }

    #[test]
    fn a_spell_too_short_to_read_changes_nothing() {
        let verdict = PathwayReview::after_loan(
            LoanSpellVerdict::Inconclusive,
            &Fx::signals(0.5, 0.8),
            &Fx::plan(1),
            &ClubPhilosophy::Balanced,
        );
        assert_eq!(verdict.stage, None);
    }

    #[test]
    fn every_philosophy_is_reachable_from_a_real_brief() {
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::DevelopYouth,
                8,
                AcademyDevelopmentIdentity::Balanced,
                ReputationLevel::National,
            ),
            ClubPhilosophy::DevelopAndSell
        );
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::Balanced,
                3,
                AcademyDevelopmentIdentity::PlayerTrading,
                ReputationLevel::Regional,
            ),
            ClubPhilosophy::DevelopAndSell,
            "a trading academy is a trading club at any standing"
        );
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::SignExperienced,
                4,
                AcademyDevelopmentIdentity::Balanced,
                ReputationLevel::Elite,
            ),
            ClubPhilosophy::SignToCompete
        );
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::Balanced,
                4,
                AcademyDevelopmentIdentity::Balanced,
                ReputationLevel::Continental,
            ),
            ClubPhilosophy::Balanced
        );
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::Balanced,
                2,
                AcademyDevelopmentIdentity::Balanced,
                ReputationLevel::Local,
            ),
            ClubPhilosophy::LoanFocused
        );
    }

    #[test]
    fn the_purpose_the_club_had_in_mind_reaches_the_candidate() {
        let mut club = Fx::club(vec![Fx::player(1, 110, 20)]);
        club.on_pathway_loan_staged(1, LoanOutReason::UnsettledAbroad, Fx::date());

        let candidate = club
            .transfer_plan
            .loan_out_candidates
            .iter()
            .find(|c| c.player_id == 1)
            .expect("the club staged him");
        assert_eq!(candidate.reason, LoanOutReason::UnsettledAbroad);
        let player = club.teams.teams[0].players.find(1).unwrap();
        assert_eq!(player.pathway_stage(), PathwayStage::LoanOut);
        assert_eq!(
            player.plan.as_ref().unwrap().loan_purpose,
            Some(LoanOutReason::UnsettledAbroad),
            "the purpose is held on the pathway, not re-derived downstream"
        );
    }

    #[test]
    fn the_monthly_review_writes_a_pathway_for_a_squad_that_had_none() {
        let mut club = Fx::club(vec![Fx::player(1, 140, 26), Fx::player(2, 70, 20)]);
        assert!(club.teams.teams[0].players.find(1).unwrap().plan.is_none());

        club.review_pathways(Fx::date());

        for id in [1, 2] {
            assert!(
                club.teams.teams[0].players.find(id).unwrap().plan.is_some(),
                "every contracted player carries the intention the club has for him"
            );
        }
    }

    #[test]
    fn selling_a_starter_moves_his_successor_up_a_rung() {
        // The heir is squad depth at this club's own level, not another
        // man of the same standing — that is what makes him the heir
        // rather than a second starter.
        let mut club = Fx::club(vec![Fx::player(1, 140, 28), Fx::player(2, 30, 22)]);
        club.review_pathways(Fx::date());
        let heir_before = club.teams.teams[0].players.find(2).unwrap().pathway_stage();
        assert_eq!(heir_before, PathwayStage::Rotation, "precondition");

        club.on_asset_sold(1, Fx::date());

        let heir_after = club.teams.teams[0].players.find(2).unwrap().pathway_stage();
        assert_ne!(
            heir_after, heir_before,
            "the man behind him moves up when the shirt comes free"
        );
        assert_eq!(heir_after, heir_before.promoted());
    }

    #[test]
    fn a_sale_with_nobody_behind_him_promotes_nobody() {
        let mut club = Fx::club(vec![Fx::player(1, 140, 28)]);
        club.review_pathways(Fx::date());
        club.on_asset_sold(1, Fx::date());
        assert!(club.teams.teams[0].players.find(1).is_some());
    }
}
