use crate::MatchTacticType;
use crate::club::board::BoardManagerMeeting;
use crate::club::board::context::FfpStatus;
use crate::club::board::decision::{BoardDecision, DecisionReason};
use crate::club::board::infrastructure::FacilityReview;
use crate::club::board::manager_market::ManagerCandidate;
use crate::club::board::ownership::{OwnershipModel, OwnershipType};
use crate::club::board::pressure::{BoardPressure, SupporterEvent};
use crate::club::board::promise::{BoardPromise, PromiseLedger, PromiseType};
use crate::club::board::relationship::ManagerRelationship;
use crate::club::board::scoring::{BoardComponentScores, SeasonPhase};
use crate::club::board::strategy::{
    InfrastructurePriority, ManagerAutonomy, ReviewFrequency, SquadProfile,
};
use crate::club::board::takeover::{TakeoverEngine, TakeoverWatch};
use crate::club::team::reputation::AchievementType;
use crate::club::{BoardContext, BoardMood, BoardMoodState, BoardResult, StaffClubContract};
use crate::context::{GlobalContext, SimulationContext};
use crate::transfers::pipeline::{TransferNeedPriority, TransferNeedReason};
use chrono::Duration;
use chrono::{Datelike, NaiveDate};
use log::debug;

/// Long-term club vision — the direction the board wants the manager to
/// take the club. Drives expectations, recruitment preferences, and
/// manager-board friction. Each item is advisory: the manager can ignore
/// it but the board will judge them against it at season's end.
#[derive(Debug, Clone, Default)]
pub struct ClubVision {
    pub playing_style: VisionPlayingStyle,
    pub youth_focus: VisionYouthFocus,
    pub signing_preference: SigningPreference,
    pub financial_stance: FinancialStance,
    pub long_term_goal: Option<LongTermGoal>,
    /// Seasons allotted for the manager to reach `long_term_goal`.
    pub long_term_horizon_seasons: u8,
    /// The kind of squad the board wants assembled. Biases transfer
    /// governance and the squad-building component score.
    pub preferred_squad_profile: SquadProfile,
    /// Where surplus capital should go — drives the yearly facility review.
    pub infrastructure_priority: InfrastructurePriority,
    /// How much football autonomy the manager is granted. Combined with
    /// ownership interference to set autonomy, DoF override, and patience.
    pub manager_autonomy: ManagerAutonomy,
    /// How often the board runs a full confidence re-evaluation.
    pub review_frequency: ReviewFrequency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisionPlayingStyle {
    #[default]
    Balanced,
    AttackingFootball,
    Possession,
    HighPressing,
    DefensiveSolid,
    CounterAttack,
    DirectPlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisionYouthFocus {
    #[default]
    Balanced,
    /// Promote youth aggressively, prefer home-grown signings.
    DevelopYouth,
    /// Proven quality only; youth serves as backup.
    SignExperienced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SigningPreference {
    #[default]
    Anyone,
    /// Prefer home-nation or home-continent signings.
    Domestic,
    /// Actively scout cheaper regions for value gems.
    ValueHunter,
    /// Top-tier names only.
    Marquee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FinancialStance {
    #[default]
    Balanced,
    /// Spend now, worry later.
    Ambitious,
    /// Live within wage budget; no loans.
    Conservative,
    /// Cost-cutting mode — sell high, minimise outgoings.
    Austerity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongTermGoal {
    WinLeague,
    WinDomesticCup,
    WinContinental,
    PromotionToTopFlight,
    EstablishTopHalf,
    Survive,
}

/// Ownership personality — a simplified chairman archetype whose traits
/// shape how the board actually exercises its powers. Two knobs, each
/// with meaningful consequences downstream of board.simulate().
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChairmanAmbition {
    #[default]
    Balanced,
    /// "We want the Champions League." Budget skew +, expectations +.
    Ambitious,
    /// Sugar daddy / oil money. Budget skew ++, expectations ++,
    /// but also trigger-happy when results slip.
    Reckless,
    /// Old-money prudent. Budget skew -, stability prized.
    Conservative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChairmanPatience {
    #[default]
    Medium,
    /// Results yesterday. Sacking threshold is one bad run away.
    Low,
    /// Long-term project builder, trusts the process.
    High,
}

#[derive(Debug, Clone, Default)]
pub struct ChairmanProfile {
    pub ambition: ChairmanAmbition,
    pub patience: ChairmanPatience,
    /// 0..100 — how personally loyal the chairman is to the current manager.
    /// Rebuilt on each hire; decays with poor form, lifts with trophies.
    pub manager_loyalty: u8,
}

impl ChairmanProfile {
    pub fn new() -> Self {
        ChairmanProfile {
            ambition: ChairmanAmbition::Balanced,
            patience: ChairmanPatience::Medium,
            manager_loyalty: 50,
        }
    }

    /// Poor-mood-month threshold before patience snaps. Lower = quicker
    /// firing. High-loyalty chairmen buy their guy some extra time.
    pub fn poor_mood_threshold(&self) -> u8 {
        let base = match self.patience {
            ChairmanPatience::Low => 3,
            ChairmanPatience::Medium => 4,
            ChairmanPatience::High => 6,
        };
        // Loyal chairmen tolerate one extra poor month before acting.
        if self.manager_loyalty >= 70 {
            base + 1
        } else if self.manager_loyalty <= 20 {
            base.saturating_sub(1).max(1)
        } else {
            base
        }
    }

    /// Multiplier applied to the baseline transfer budget. Reckless owners
    /// push spend harder; conservative ones throttle it.
    pub fn budget_multiplier(&self) -> f32 {
        match self.ambition {
            ChairmanAmbition::Reckless => 1.4,
            ChairmanAmbition::Ambitious => 1.15,
            ChairmanAmbition::Balanced => 1.0,
            ChairmanAmbition::Conservative => 0.85,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardTransferDecision {
    Approved,
    Conditional(BoardTransferConcern),
    Vetoed(BoardTransferConcern),
}

impl BoardTransferDecision {
    pub fn is_approved(self) -> bool {
        matches!(
            self,
            BoardTransferDecision::Approved | BoardTransferDecision::Conditional(_)
        )
    }

    pub fn manager_satisfaction_delta(self, priority: &TransferNeedPriority) -> f32 {
        match self {
            BoardTransferDecision::Approved => match priority {
                TransferNeedPriority::Critical => 0.8,
                TransferNeedPriority::Important => 0.4,
                TransferNeedPriority::Optional => 0.1,
            },
            BoardTransferDecision::Conditional(_) => match priority {
                TransferNeedPriority::Critical => -0.8,
                TransferNeedPriority::Important => -0.4,
                TransferNeedPriority::Optional => 0.0,
            },
            BoardTransferDecision::Vetoed(_) => match priority {
                TransferNeedPriority::Critical => -4.5,
                TransferNeedPriority::Important => -2.75,
                TransferNeedPriority::Optional => -1.0,
            },
        }
    }

    pub fn loyalty_delta(self, priority: &TransferNeedPriority) -> i16 {
        match self {
            BoardTransferDecision::Approved => match priority {
                TransferNeedPriority::Critical => 1,
                _ => 0,
            },
            BoardTransferDecision::Conditional(_) => match priority {
                TransferNeedPriority::Critical => -1,
                _ => 0,
            },
            BoardTransferDecision::Vetoed(_) => match priority {
                TransferNeedPriority::Critical => -5,
                TransferNeedPriority::Important => -3,
                TransferNeedPriority::Optional => -1,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardTransferConcern {
    ExceedsTransferBudget,
    FinancialDiscipline,
    WeakSportingCase,
    ConflictsWithVision,
}

#[derive(Debug, Clone)]
pub struct BoardTransferProposal {
    pub fee: f64,
    pub allocated_budget: f64,
    pub remaining_transfer_budget: f64,
    pub priority: TransferNeedPriority,
    pub reason: TransferNeedReason,
    pub player_age: Option<u8>,
    pub player_ability: Option<u8>,
    pub squad_avg_ability: u8,
    pub shortlist_score: f32,
    /// Optional recruitment-meeting dossier built from scout monitoring
    /// state. When present, the board uses it to relax or tighten its
    /// tolerance — strong consensus + chief scout backing earn extra
    /// rope; thin discussion or risk-heavy dossiers get less.
    /// When `None` the board falls back to the legacy decision path
    /// (preserves behaviour for non-pipeline call sites and tests).
    pub dossier: Option<BoardDossierSummary>,
    /// Optional financial/profile dossier on the deal. When present the
    /// board applies ownership-archetype governance (wage impact, resale,
    /// risk, manager priority). `None` keeps the legacy path for tests and
    /// call sites that don't build it yet.
    pub economics: Option<BoardTransferEconomics>,
}

/// Financial + profile snapshot of a proposed signing, used by the board's
/// ownership-archetype governance. Mirrors the recruitment dossier pattern
/// — present for pipeline calls, `None` elsewhere.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoardTransferEconomics {
    /// Added annual wage this signing commits the club to.
    pub wage_impact_annual: f64,
    /// Remaining annual wage-budget headroom before the deal.
    pub wage_budget_headroom: f64,
    /// Agent fee on top of the transfer fee.
    pub agent_fee: f64,
    /// Proposed contract length in years.
    pub contract_length_years: u8,
    /// Projected resale value at the end of the deal.
    pub resale_projection: f64,
    /// Off-pitch risk 0..1 (1 = serious professionalism/discipline concern).
    pub professionalism_risk: f32,
    /// True when the player counts as home-grown / domestic for the club.
    pub homegrown_fit: bool,
    /// Injury proneness 0..1.
    pub injury_risk: f32,
    /// Commercial / shirt-sales appeal 0..1.
    pub commercial_value: f32,
    /// True when this is the manager's explicit priority target.
    pub manager_priority: bool,
}

/// Compact, board-facing snapshot of the recruitment dossier. We pull
/// only the fields the board actually reasons about so the board layer
/// stays decoupled from `pipeline::recruitment`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoardDossierSummary {
    pub scout_votes: u8,
    pub chief_scout_support: bool,
    pub avg_confidence: f32,
    pub avg_role_fit: f32,
    pub risk_flag_count: u8,
    /// Sum of weighted scout votes from the latest meeting on the player.
    pub consensus_score: f32,
    pub data_support: bool,
    pub matches_watched: u16,
}

#[derive(Debug, Clone)]
pub struct SeasonTargets {
    pub transfer_budget: i32,
    pub wage_budget: i32,
    pub max_squad_size: u8,
    pub min_squad_size: u8,
    /// Expected league finish position (1-based). Board judges performance against this.
    pub expected_position: u8,
    /// Minimum acceptable position before board becomes unhappy
    pub min_acceptable_position: u8,
}

/// Board confidence in the current management (0-100).
/// Drops when results are poor, recovers when exceeding expectations.
/// At 0 — or after sustained Poor mood — the manager is sacked.
#[derive(Debug, Clone)]
pub struct BoardConfidence {
    pub level: i32,
}

impl Default for BoardConfidence {
    fn default() -> Self {
        BoardConfidence { level: 65 }
    }
}

#[derive(Debug, Clone)]
pub struct ClubBoard {
    pub mood: BoardMood,
    pub confidence: BoardConfidence,
    pub director: Option<StaffClubContract>,
    pub sport_director: Option<StaffClubContract>,
    pub season_targets: Option<SeasonTargets>,
    /// Consecutive months the board has been in Poor mood
    pub poor_mood_months: u8,
    /// The board has publicly put the manager on final warning (the
    /// crisis-meeting ultimatum). A sack — barring a total confidence
    /// collapse — requires this to have been set in an EARLIER month,
    /// so the ultimatum is a real stage the squad gets to react to,
    /// not a same-tick formality. Cleared on recovery and on sacking.
    pub manager_on_final_warning: bool,
    /// Long-term vision — the "contract" the board expects the manager
    /// to honour across multiple seasons.
    pub vision: ClubVision,
    /// Year the current vision horizon started. Populated on the first
    /// season-start tick after the vision is installed. Reset at the end
    /// of each horizon regardless of outcome.
    pub vision_start_year: Option<i32>,
    /// Set to true the first time a trophy / promotion matching the
    /// long-term goal lands in the current horizon. Tracked separately
    /// from `team.reputation` achievements because those decay after two
    /// years and horizons can extend longer.
    pub vision_goal_achieved: bool,
    /// Date the last manager was dismissed — drives the search timer.
    /// `None` when the manager seat is filled (either permanently, or
    /// an interim has been confirmed as permanent).
    pub manager_search_since: Option<NaiveDate>,
    /// Ranked free-agent (slice B) and employed-target (slice C)
    /// candidates the board is willing to appoint. Refreshed weekly
    /// while a search is open. Front of vec = top choice.
    pub manager_shortlist: Vec<ManagerCandidate>,
    /// Day the current shortlist was built. Used to decide when it's
    /// stale enough to rebuild — see `ManagerShortlist::REFRESH_DAYS`.
    pub shortlist_built_at: Option<NaiveDate>,
    /// How long the search may run before the board commits to a
    /// hire. Locked in when `manager_search_since` is set so it stays
    /// stable across the search window. Top clubs hold out longer.
    pub search_window_days: u16,
    /// Ownership archetype. Modulates budget size, sacking threshold,
    /// and long-term tolerance. Populated at club creation; stable for
    /// the lifetime of the chairman.
    pub chairman: ChairmanProfile,
    /// Richer ownership submodel layered on the chairman — wealth,
    /// interference, risk appetite, exit pressure. Derived once from the
    /// club's durable signals (reputation, finances, league) on the first
    /// simulate tick, then stable for the chairman's tenure.
    pub ownership: OwnershipModel,
    /// Slow-moving pressure gauges (supporters, media, dressing room,
    /// finances, regulatory) read as inputs to confidence and meetings.
    pub pressure: BoardPressure,
    /// Five-facet board↔manager trust relationship. Drives renewals,
    /// relationship-driven dismissal, and transfer autonomy.
    pub relationship: ManagerRelationship,
    /// Live board promises to the manager and their kept/broken record.
    pub promises: PromiseLedger,
    /// Latest component scores from the monthly review — stored so the UI
    /// and tests can inspect *why* the board feels how it does.
    pub latest_scores: BoardComponentScores,
    /// Rare ownership-change watch (takeover rumours / completion).
    pub takeover: TakeoverWatch,
    /// 0-based month index since the current season started — drives the
    /// quarterly / season-end review cadence.
    pub season_month_index: u32,
    /// One-shot guard: ownership/personality is derived from club data on
    /// the first simulate tick (which, unlike `new()`, has club context).
    pub personality_initialized: bool,
    /// Calendar year the board last approved a *funded* facility upgrade.
    /// Drives a cooldown so even a wealthy owner can't upgrade every single
    /// season — see `FacilityReview::COOLDOWN_SEASONS`.
    pub last_facility_upgrade_year: Option<i32>,
}

impl ClubBoard {
    pub fn new() -> Self {
        ClubBoard {
            mood: BoardMood::default(),
            confidence: BoardConfidence::default(),
            director: None,
            sport_director: None,
            season_targets: None,
            poor_mood_months: 0,
            manager_on_final_warning: false,
            vision: ClubVision::default(),
            vision_start_year: None,
            vision_goal_achieved: false,
            manager_search_since: None,
            manager_shortlist: Vec::new(),
            shortlist_built_at: None,
            search_window_days: 0,
            chairman: ChairmanProfile::new(),
            ownership: OwnershipModel::new(),
            pressure: BoardPressure::new(),
            relationship: ManagerRelationship::new(),
            promises: PromiseLedger::new(),
            latest_scores: BoardComponentScores::default(),
            takeover: TakeoverWatch::new(),
            season_month_index: 0,
            personality_initialized: false,
            last_facility_upgrade_year: None,
        }
    }

    /// True when the current long-term goal matches the achievement just
    /// earned. Call at trophy time to flip `vision_goal_achieved`.
    pub fn matches_long_term_goal(&self, ach: AchievementType) -> bool {
        let Some(goal) = self.vision.long_term_goal else {
            return false;
        };
        use LongTermGoal::*;
        matches!(
            (goal, ach),
            (WinLeague, AchievementType::LeagueTitle)
                | (WinDomesticCup, AchievementType::CupWin)
                | (WinContinental, AchievementType::ContinentalTrophy)
                | (PromotionToTopFlight, AchievementType::Promotion)
        )
    }

    /// Flip `vision_goal_achieved` when this achievement lands the long-term
    /// target. Returns true if the flag changed.
    pub fn on_achievement(&mut self, ach: AchievementType) -> bool {
        if !self.vision_goal_achieved && self.matches_long_term_goal(ach) {
            self.vision_goal_achieved = true;
            true
        } else {
            false
        }
    }

    /// Board/chairman review of a proposed incoming transfer. This is the
    /// football committee layer: the head coach can ask, the recruitment team
    /// can shortlist, but ownership still weighs budget, urgency, squad level,
    /// chairman temperament, and club vision before negotiations start.
    pub fn review_transfer_proposal(
        &self,
        proposal: &BoardTransferProposal,
    ) -> BoardTransferDecision {
        let allocated_budget = proposal.allocated_budget.max(1.0);
        let over_allocated = proposal.fee / allocated_budget;
        let remaining_budget = proposal.remaining_transfer_budget.max(0.0);

        if remaining_budget > 0.0 && proposal.fee > remaining_budget * 1.05 {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::ExceedsTransferBudget);
        }

        let mut tolerance: f64 = match self.vision.financial_stance {
            FinancialStance::Austerity => 0.90,
            FinancialStance::Conservative => 1.25,
            FinancialStance::Balanced => 1.75,
            FinancialStance::Ambitious => 2.35,
        };

        tolerance += match self.chairman.ambition {
            ChairmanAmbition::Reckless => 0.45,
            ChairmanAmbition::Ambitious => 0.20,
            ChairmanAmbition::Balanced => 0.0,
            ChairmanAmbition::Conservative => -0.15,
        };

        // Ownership archetype risk appetite. Neutral owners (risk 50,
        // LocalBusiness) contribute exactly 0 so legacy call sites and
        // tests are unaffected.
        tolerance += (self.ownership.risk_tolerance as f64 - 50.0) / 100.0 * 0.5;
        tolerance += match self.ownership.ownership_type {
            OwnershipType::StateBacked => 0.20,
            OwnershipType::MemberOwned => -0.10,
            OwnershipType::PrivateEquity => -0.05,
            _ => 0.0,
        };

        // Member-owned boards prize local identity: a homegrown target earns
        // extra rope, an import is viewed more coolly. Reads the economics
        // dossier's homegrown flag when one is present.
        if matches!(self.ownership.ownership_type, OwnershipType::MemberOwned) {
            if let Some(e) = proposal.economics {
                tolerance += if e.homegrown_fit { 0.20 } else { -0.10 };
            }
        }

        tolerance += match proposal.priority {
            TransferNeedPriority::Critical => 0.35,
            TransferNeedPriority::Important => 0.15,
            TransferNeedPriority::Optional => 0.0,
        };

        if self.confidence.level >= 75 {
            tolerance += 0.15;
        } else if self.confidence.level < 35 {
            tolerance -= 0.25;
        }

        // Low-autonomy boards under sliding confidence let the director of
        // football intervene and tighten tolerance on the manager's asks.
        if matches!(self.vision.manager_autonomy, ManagerAutonomy::Low)
            && self.confidence.level < self.vision.manager_autonomy.dof_override_threshold()
        {
            tolerance -= 0.20;
        }

        if is_board_urgent_reason(&proposal.reason) {
            tolerance += 0.20;
        }

        if proposal.shortlist_score >= 1.15 {
            tolerance += 0.10;
        } else if proposal.shortlist_score < 0.75 {
            tolerance -= 0.15;
        }

        // Dossier-driven tolerance shift. Strong consensus + chief
        // scout backing + plenty of confidence earn extra board rope;
        // thin or risk-heavy dossiers tighten tolerance. Done before
        // the over-allocation gate so a well-supported target can
        // survive a slightly higher fee, and a poorly-supported one
        // can fall short even if the fee is close to budget.
        if let Some(d) = proposal.dossier {
            if d.consensus_score >= 2.5 && d.chief_scout_support {
                tolerance += 0.20;
            } else if d.consensus_score >= 1.5 {
                tolerance += 0.10;
            } else if d.consensus_score <= 0.5 && d.scout_votes >= 2 {
                tolerance -= 0.15;
            }
            if d.avg_confidence >= 0.8 {
                tolerance += 0.05;
            } else if d.avg_confidence < 0.5 {
                tolerance -= 0.10;
            }
            if d.risk_flag_count >= 3 {
                tolerance -= 0.15;
            }
            if d.data_support {
                tolerance += 0.05;
            }
            if d.avg_role_fit < 0.85 {
                tolerance -= 0.10;
            }
        }

        if over_allocated > tolerance.max(0.50) {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline);
        }

        if !self.is_sporting_case_credible(proposal) {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::WeakSportingCase);
        }

        if self.transfer_conflicts_with_vision(proposal) {
            return BoardTransferDecision::Conditional(BoardTransferConcern::ConflictsWithVision);
        }

        // Dossier-driven veto: if the dossier shows a serious red flag
        // (split votes / no role fit / multiple risks) the board sends
        // it back to the recruitment team rather than approving.
        if let Some(d) = proposal.dossier {
            // "Two scouts watching, consensus near zero" = open
            // disagreement. The board doesn't sign on a flip-coin.
            if d.scout_votes >= 2 && d.consensus_score.abs() < 0.4 && d.risk_flag_count >= 2 {
                return BoardTransferDecision::Vetoed(BoardTransferConcern::WeakSportingCase);
            }
        }

        // Ownership-archetype governance: squad-profile fit + deal
        // economics (wage impact, resale, off-pitch risk). No-op for a
        // Balanced profile with no economics dossier.
        if let Some(decision) = self.review_governance(proposal) {
            return decision;
        }

        if over_allocated > 1.0 || remaining_budget <= allocated_budget * 0.25 {
            return BoardTransferDecision::Conditional(BoardTransferConcern::FinancialDiscipline);
        }

        BoardTransferDecision::Approved
    }

    /// Ownership-archetype governance layered on the base review:
    /// squad-profile fit plus deal economics (wage impact, resale, risk).
    /// Returns `Some` to override the base decision; `None` to defer to it.
    /// A `Balanced` profile with no economics dossier always returns `None`.
    fn review_governance(&self, proposal: &BoardTransferProposal) -> Option<BoardTransferDecision> {
        use BoardTransferConcern::*;

        // ── Squad profile fit ──
        if let Some(age) = proposal.player_age {
            let critical = matches!(proposal.priority, TransferNeedPriority::Critical);
            match self.vision.preferred_squad_profile {
                // Youth project: accept weaker-but-young; block ageing depth
                // outright (a hard veto, not a soft flag).
                SquadProfile::Youth if age >= 29 && !critical => {
                    return Some(BoardTransferDecision::Vetoed(ConflictsWithVision));
                }
                // Resale model: no point buying a player past resale age.
                SquadProfile::ResaleValue if age >= 30 && !critical => {
                    return Some(BoardTransferDecision::Conditional(ConflictsWithVision));
                }
                // Galáctico policy: signings must raise the bar.
                SquadProfile::Stars => {
                    if let Some(ability) = proposal.player_ability {
                        if ability + 4 < proposal.squad_avg_ability {
                            return Some(BoardTransferDecision::Vetoed(WeakSportingCase));
                        }
                    }
                }
                _ => {}
            }
        }

        // ── Deal economics ──
        let Some(e) = proposal.economics else {
            return None;
        };

        let elite_exception = matches!(
            self.chairman.ambition,
            ChairmanAmbition::Reckless | ChairmanAmbition::Ambitious
        ) && (matches!(proposal.priority, TransferNeedPriority::Critical)
            || proposal
                .player_ability
                .is_some_and(|a| a >= proposal.squad_avg_ability.saturating_add(10)));

        // Wage impact above the remaining headroom.
        if e.wage_impact_annual > e.wage_budget_headroom.max(0.0) {
            let austere = matches!(
                self.vision.financial_stance,
                FinancialStance::Conservative | FinancialStance::Austerity
            );
            if austere && !elite_exception {
                return Some(BoardTransferDecision::Vetoed(FinancialDiscipline));
            }
            if !elite_exception {
                return Some(BoardTransferDecision::Conditional(FinancialDiscipline));
            }
        }

        // Private-equity / resale owners dislike ageing players with weak
        // resale projection.
        if self.ownership.ownership_type.resale_driven() {
            let poor_resale = e.resale_projection < proposal.fee * 0.4;
            let ageing = proposal.player_age.is_some_and(|a| a >= 28);
            if poor_resale && ageing {
                return Some(BoardTransferDecision::Conditional(ConflictsWithVision));
            }
        }

        // Off-pitch risk worries prudent / fan-owned boards.
        if e.professionalism_risk >= 0.7
            && matches!(
                self.vision.financial_stance,
                FinancialStance::Conservative | FinancialStance::Austerity
            )
        {
            return Some(BoardTransferDecision::Conditional(WeakSportingCase));
        }

        None
    }

    fn is_sporting_case_credible(&self, proposal: &BoardTransferProposal) -> bool {
        if matches!(
            proposal.reason,
            TransferNeedReason::DevelopmentSigning
                | TransferNeedReason::CheapReinforcement
                | TransferNeedReason::SquadPadding
                | TransferNeedReason::InjuryCoverLoan
                | TransferNeedReason::LoanToFillSquad
        ) {
            return true;
        }

        let Some(ability) = proposal.player_ability else {
            return true;
        };

        let squad_avg = proposal.squad_avg_ability;
        ability.saturating_add(12) >= squad_avg || proposal.shortlist_score >= 0.95
    }

    fn transfer_conflicts_with_vision(&self, proposal: &BoardTransferProposal) -> bool {
        let Some(age) = proposal.player_age else {
            return false;
        };

        match self.vision.youth_focus {
            VisionYouthFocus::DevelopYouth => {
                age >= 30
                    && matches!(
                        proposal.reason,
                        TransferNeedReason::DevelopmentSigning
                            | TransferNeedReason::SuccessionPlanning
                            | TransferNeedReason::StaffRecommendation
                    )
            }
            VisionYouthFocus::SignExperienced => {
                age <= 20
                    && !matches!(
                        proposal.reason,
                        TransferNeedReason::DevelopmentSigning
                            | TransferNeedReason::SuccessionPlanning
                    )
            }
            VisionYouthFocus::Balanced => false,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> BoardResult {
        let mut result = BoardResult::new();
        result.club_id = ctx.club.as_ref().map(|c| c.id).unwrap_or(0);
        let today = ctx.simulation.date.date();

        // Derive the ownership archetype + opening vision once, the first
        // time we have club context to read. Different clubs get different
        // boards purely from durable signals — no hard-coded names.
        if !self.personality_initialized {
            if let Some(board_ctx) = &ctx.board {
                self.bootstrap_personality(board_ctx, result.club_id);
            }
        }

        if self.director.is_none() {
            self.run_director_election(&ctx.simulation);
        }

        if self.sport_director.is_none() {
            self.run_sport_director_election(&ctx.simulation);
        }

        if ctx.simulation.check_contract_expiration() {
            if self.is_director_contract_expiring(&ctx.simulation) {}
            if self.is_sport_director_contract_expiring(&ctx.simulation) {}
        }

        let season = ctx
            .country
            .as_ref()
            .map(|c| c.season_dates)
            .unwrap_or_default();
        let is_season_start = ctx.simulation.is_season_start(&season);
        let is_month_beginning = ctx.simulation.is_month_beginning();

        // ── Season start: targets, vision reckoning, facility review ──
        if is_season_start {
            if let Some(board_ctx) = &ctx.board {
                let current_year = today.year();
                self.evaluate_long_term_vision(current_year, &mut result);
                self.calculate_season_targets(board_ctx);

                // Promises whose deadline lapsed unfulfilled break now and
                // cost the manager board trust.
                let penalty = self.promises.break_overdue(today);
                if penalty != 0 {
                    self.relationship.adjust_communication(penalty);
                }
                self.promises.prune(today, 800);

                // Yearly infrastructure review → facility decisions applied
                // in `BoardResult::process`. Gated by a per-board cooldown
                // so wealthy owners can't upgrade every season.
                let facility_decisions = self.run_facility_review(board_ctx, current_year);

                // Open this season's board promises (season goal, youth
                // pathway, deferred capex). Done after the review so a
                // declined-on-affordability upgrade becomes a "we'll revisit"
                // facility promise.
                self.open_season_promises(board_ctx, today, &facility_decisions);
                result.decisions.extend(facility_decisions);

                // Renewal: a happy board moves to tie the manager down, but
                // only when the deal is genuinely running down (or its
                // length is unknown). Driven by legacy confidence/loyalty OR
                // sustained multi-facet trust.
                let contract_at_risk = board_ctx.manager_contract_months_left == 0
                    || board_ctx.manager_contract_months_left <= 18;
                if !result.manager_sacked
                    && contract_at_risk
                    && ((self.confidence.level >= 70 && self.chairman.manager_loyalty >= 55)
                        || self.relationship.merits_renewal())
                {
                    result.offer_manager_renewal = true;
                }
                self.confidence.level = 65; // Reset confidence at season start
                self.poor_mood_months = 0;
                self.season_month_index = 0;
            }
        }

        // ── Monthly review + takeover watch ──
        if is_month_beginning {
            if let Some(board_ctx) = &ctx.board {
                self.evaluate_performance(board_ctx, &mut result);
                self.tick_takeover(board_ctx, today, &mut result);
                // Resolve outstanding promises against this tick's decisions
                // and league standing (kept promises build manager trust).
                self.resolve_promises(board_ctx, today, &mut result);
            }
            if !is_season_start {
                self.season_month_index = self.season_month_index.saturating_add(1);
            }
        }

        // Manager search: once the per-club search window elapses, signal
        // the result stage to confirm a permanent appointment. The result
        // stage tries the top free-agent shortlist first (slice B) and
        // falls back to promoting the caretaker if no candidate sticks.
        // Window length scales with reputation — top clubs hunt longer
        // because they're chasing big names; smaller clubs move faster.
        if let Some(since) = self.manager_search_since {
            let today = ctx.simulation.date.date();
            let days = (today - since).num_days();
            // Defensive: a board with `manager_search_since` set but a
            // zero search window (legacy state, or first tick after a
            // hot-reload) falls back to the previous fixed value so the
            // seat doesn't sit empty forever.
            let window = if self.search_window_days == 0 {
                30
            } else {
                self.search_window_days as i64
            };
            if days >= window {
                result.confirm_new_manager = true;
            }
        }

        result
    }

    /// Check whether the long-term horizon has elapsed and reckon with the
    /// manager against the original vision goal. Fires at the START of a
    /// season — the previous season's trophies are already banked in
    /// `vision_goal_achieved`. Horizonless visions (no `long_term_goal`)
    /// don't trigger any judgment.
    fn evaluate_long_term_vision(&mut self, current_year: i32, result: &mut BoardResult) {
        if self.vision.long_term_goal.is_none() || self.vision.long_term_horizon_seasons == 0 {
            return;
        }

        let start_year = match self.vision_start_year {
            Some(y) => y,
            None => {
                // First season under this vision — start the clock and return.
                self.vision_start_year = Some(current_year);
                return;
            }
        };

        let seasons_elapsed = (current_year - start_year).max(0) as u8;
        if seasons_elapsed < self.vision.long_term_horizon_seasons {
            return;
        }

        // Horizon reached. Judge and reset regardless of outcome.
        if !self.vision_goal_achieved {
            debug!(
                "Long-term vision failed: goal {:?} not met in {} seasons — manager sacked",
                self.vision.long_term_goal, self.vision.long_term_horizon_seasons
            );
            result.manager_sacked = true;
            self.confidence.level = 20;
            self.poor_mood_months = 0;
        } else {
            // Horizon met. Small confidence bump so the next horizon starts
            // on a positive note; board keeps the manager.
            self.confidence.level = (self.confidence.level + 10).clamp(0, 100);
        }

        self.vision_start_year = Some(current_year);
        self.vision_goal_achieved = false;
    }

    fn calculate_season_targets(&mut self, board_ctx: &BoardContext) {
        let rep = board_ctx.reputation_score;

        // Revenue-based budgets: a club's transfer war chest comes from
        // the slack between projected income and projected expenses, not
        // from the cash balance. Clubs that spent the offseason hauling
        // in TV money get a meaningful budget; clubs running at a deficit
        // get nothing — even if the bank account looks healthy from a
        // recent owner injection.
        let projected_income = board_ctx.trailing_annual_income.max(0) as f64;
        let projected_expenses = board_ctx.trailing_annual_outcome.max(0) as f64;
        let projected_free_cash = (projected_income - projected_expenses).max(0.0);

        let ambition_mult = ambition_budget_multiplier(self.vision.long_term_goal);
        let chair_mult = self.chairman.budget_multiplier() as f64;
        let ffp_mult = match board_ctx.ffp_status {
            FfpStatus::Clean => 1.00,
            FfpStatus::Watchlist => 0.70,
            FfpStatus::Breach => 0.35,
        };

        // Cold-start fallback: a freshly created club with no twelve-month
        // history would have free_cash == 0 and never get a budget. Seed
        // the calculation with a reputation-scaled allowance so the first
        // season can make signings — slightly smaller than the legacy
        // cash-based budget to avoid over-spending when the club hasn't
        // earned anything yet.
        let seed_budget = if projected_income < 1.0 {
            let cash = board_ctx.balance.max(0) as f64;
            let seed_pct = if rep >= 0.8 {
                0.30
            } else if rep >= 0.6 {
                0.25
            } else if rep >= 0.4 {
                0.20
            } else {
                0.15
            };
            cash * seed_pct
        } else {
            0.0
        };

        // Ownership wealth/risk multiplier. Neutral owners resolve to 1.0
        // so the legacy budget tests are unchanged; a deep-pocketed
        // risk-taker can inflate the war chest, a cautious one throttles it.
        let owner_mult = self.ownership.budget_multiplier();
        let revenue_budget =
            projected_free_cash * ambition_mult * chair_mult * ffp_mult * owner_mult;
        let raw_budget = (revenue_budget + seed_budget).max(0.0);

        let eco = board_ctx.country_economic_factor as f64;
        let price = board_ctx.country_price_level as f64;
        let price_ceiling = price * price * 80_000_000.0;
        let eco_ceiling = eco * eco * 300_000_000.0;
        // Division tier caps the war chest: lower leagues simply don't move
        // the same money. Top flight (tier 1) is unconstrained here.
        let tier_factor = match board_ctx.league_tier {
            0 | 1 => 1.0,
            2 => 0.6,
            3 => 0.35,
            _ => 0.2,
        };
        let budget_ceiling = price_ceiling.min(eco_ceiling) * tier_factor;
        let transfer_budget = raw_budget.min(budget_ceiling) as i32;

        // Wage budget: target wage/revenue ratio. Healthy clubs run
        // 55–65% wages on revenue; distressed clubs squeeze that down to
        // 45–50%; reckless elite owners are allowed to push to 70%.
        let target_ratio = (wage_revenue_target(board_ctx.ffp_status, self.chairman.ambition, rep)
            + self.ownership.wage_ratio_bonus())
        .clamp(0.30, 0.80);
        let revenue_floor = projected_income.max(board_ctx.total_annual_wages as f64);
        let wage_budget =
            (revenue_floor * target_ratio).max(board_ctx.total_annual_wages as f64 * 0.95) as i32;

        // Squad size limits based on reputation
        let (min_squad, max_squad) = if rep >= 0.8 {
            (25u8, 50u8)
        } else if rep >= 0.6 {
            (23, 45)
        } else if rep >= 0.4 {
            (20, 38)
        } else if rep >= 0.2 {
            (18, 30)
        } else {
            (16, 25)
        };

        // Expected league position. Reputation sets the baseline, then the
        // owner's ambition and the long-term goal pull it up or down, and a
        // lower division nudges a reputable club towards the promotion mix.
        //
        // TODO: previous-season finish and a league-relative wage / squad-
        // ability rank would sharpen this. Neither is threaded into
        // `BoardContext` yet, so we fall back to the reputation baseline
        // rather than invent a rank.
        let (expected, min_acceptable) = if board_ctx.league_size > 0 {
            let league_sz = board_ctx.league_size as f32;

            // Baseline: 0.0 = champions, 1.0 = bottom of the table.
            let mut frac = 1.0 - rep;

            // Owner ambition shifts the bar; reckless owners demand more.
            frac += match self.chairman.ambition {
                ChairmanAmbition::Reckless => -0.10,
                ChairmanAmbition::Ambitious => -0.05,
                ChairmanAmbition::Balanced => 0.0,
                ChairmanAmbition::Conservative => 0.06,
            };

            // The long-term goal anchors a ceiling (or floor) on the brief.
            if let Some(goal) = self.vision.long_term_goal {
                match goal {
                    LongTermGoal::WinLeague
                    | LongTermGoal::WinContinental
                    | LongTermGoal::PromotionToTopFlight => frac = frac.min(0.10),
                    LongTermGoal::EstablishTopHalf => frac = frac.min(0.45),
                    LongTermGoal::Survive => frac = frac.max(0.72),
                    LongTermGoal::WinDomesticCup => {}
                }
            }

            // A reputable club in a lower division is expected to push up.
            if board_ctx.league_tier >= 2 {
                frac -= 0.05;
            }

            frac = frac.clamp(0.02, 0.97);
            let expected = ((frac * league_sz).round() as u8).clamp(1, board_ctx.league_size);
            // Acceptable floor sits a quarter-table below the target.
            let buffer = (league_sz * 0.25).max(2.0) as u8;
            let min_acceptable = expected.saturating_add(buffer).min(board_ctx.league_size);
            (expected, min_acceptable)
        } else {
            (1, 1)
        };

        self.season_targets = Some(SeasonTargets {
            transfer_budget,
            wage_budget,
            max_squad_size: max_squad,
            min_squad_size: min_squad,
            expected_position: expected,
            min_acceptable_position: min_acceptable,
        });
    }

    /// Monthly performance evaluation — the core of board behaviour. Scores
    /// four independent dimensions (sporting / financial / squad-building /
    /// strategy), folds in supporter & financial pressure, drifts the
    /// manager relationship, and gates meetings / sackings / budget moves.
    fn evaluate_performance(&mut self, board_ctx: &BoardContext, result: &mut BoardResult) {
        // Own a copy of the targets so we can freely mutate other board
        // fields below without fighting the borrow checker.
        let targets = match self.season_targets.clone() {
            Some(t) => t,
            None => return,
        };

        let phase = SeasonPhase::classify(board_ctx.matches_played, board_ctx.total_matches);

        // Respect the board's review cadence — quarterly / season-end
        // boards don't re-judge every month. Still surface current state.
        if !self
            .vision
            .review_frequency
            .evaluates_on_month(self.season_month_index)
        {
            result.mood = self.mood.state.clone();
            result.confidence = self.confidence.level;
            return;
        }

        // Playing-style mismatch (legacy helper retained).
        let style_drag = match board_ctx.main_tactic {
            Some(t) => style_mismatch_drag(self.vision.playing_style, t),
            None => 0,
        };

        // ── Component scores ──
        let scores = BoardComponentScores::evaluate(
            board_ctx,
            &targets,
            &self.vision,
            &self.promises,
            phase,
            style_drag,
        );
        self.latest_scores = scores;

        // ── Pressure inputs (supporters / media / finances / regulatory) ──
        self.refresh_pressure(board_ctx);
        let pressure_drag = self.pressure.confidence_drag(self.ownership.ownership_type);

        // ── Confidence: component delta minus pressure drag ──
        let confidence_change = scores.confidence_delta(phase) - pressure_drag;
        self.confidence.level = (self.confidence.level + confidence_change).clamp(0, 100);

        // ── Manager relationship drift ──
        self.relationship.update_from_scores(&scores, style_drag);
        // Keep the legacy loyalty scalar broadly in step (blend, so the
        // fast-moving transfer/achievement nudges from other systems
        // aren't wholly overwritten).
        let blended =
            ((self.chairman.manager_loyalty as i16 + self.relationship.overall_trust() as i16) / 2)
                .clamp(0, 100) as u8;
        self.chairman.manager_loyalty = blended;

        // Position-vs-expectation delta retained for backing / meetings.
        let performance_delta = if board_ctx.league_position > 0 && board_ctx.matches_played >= 5 {
            targets.expected_position as i32 - board_ctx.league_position as i32
        } else {
            0
        };

        // ── Mood from confidence ──
        let new_mood = if self.confidence.level >= 80 {
            BoardMoodState::Excellent
        } else if self.confidence.level >= 55 {
            BoardMoodState::Good
        } else if self.confidence.level >= 30 {
            BoardMoodState::Normal
        } else {
            BoardMoodState::Poor
        };
        if matches!(new_mood, BoardMoodState::Poor) {
            self.poor_mood_months += 1;
        } else {
            self.poor_mood_months = 0;
        }
        self.mood.state = new_mood;

        // ── Manager satisfaction (mood + style friction) ──
        let mood_delta = match self.mood.state {
            BoardMoodState::Excellent => 1.5,
            BoardMoodState::Good => 0.5,
            BoardMoodState::Normal => 0.0,
            BoardMoodState::Poor => -1.0 - (self.poor_mood_months as f32 * 0.5).min(3.0),
        };
        let style_friction = (style_drag as f32 * 0.35).min(1.5);
        result.manager_satisfaction_delta = mood_delta - style_friction;

        // ── Squad limits ──
        let total_squad = board_ctx.main_squad_size + board_ctx.reserve_squad_size;
        if total_squad > targets.max_squad_size as usize + 5 {
            result.squad_over_limit = true;
            result.squad_excess = total_squad.saturating_sub(targets.max_squad_size as usize);
        }
        if board_ctx.main_squad_size < targets.min_squad_size as usize {
            result.squad_under_limit = true;
        }

        // ── Underperformance alarm ──
        if board_ctx.league_position > 0
            && board_ctx.league_position > targets.min_acceptable_position
            && phase.can_judge_table()
        {
            result.underperforming = true;
        }

        result.mood = self.mood.state.clone();
        result.confidence = self.confidence.level;

        // ── Budget / FFP / owner-injection decisions. Single source of
        // truth: emits at most one cut OR one increase per tick, and sets
        // the legacy `cut_transfer_budget` / `bonus_transfer_funds` flags
        // purely for the UI — `process` no longer applies them itself. ──
        self.emit_budget_decisions(board_ctx, &targets, performance_delta, result);

        if result.underperforming || matches!(self.mood.state, BoardMoodState::Poor) {
            debug!(
                "Board unhappy at confidence {} (weakest: {}): pos {}/{} expected {}",
                self.confidence.level,
                scores.headline(),
                board_ctx.league_position,
                board_ctx.league_size,
                targets.expected_position
            );
        }

        // ── Sacking gate ──
        // Triggers: zero confidence; sustained poor mood (+ underperformance);
        // sustained poor mood regardless; or a full relationship breakdown.
        // Patience is the chairman's threshold adjusted by manager autonomy.
        // Early-season grace via `phase.can_sack_manager()`.
        let enough_data = phase.can_sack_manager();
        let zero_confidence = self.confidence.level <= 0;
        let base_threshold = self.chairman.poor_mood_threshold() as i16;
        let autonomy_adj = self.vision.manager_autonomy.patience_bonus() as i16;
        let patience_threshold = (base_threshold + autonomy_adj).clamp(1, 12) as u8;
        let sustained_poor_with_underperformance =
            self.poor_mood_months >= patience_threshold && result.underperforming;
        let sustained_poor_absolute = self.poor_mood_months >= patience_threshold + 2;
        let relationship_breakdown =
            self.relationship.relationship_breakdown() && phase.can_judge_table();

        // The ladder needs LAST month's warning state: a sack only
        // follows an ultimatum the squad has already lived with — never
        // the same meeting that issued it.
        let already_on_final_warning = self.manager_on_final_warning;
        let crisis = sustained_poor_with_underperformance
            || sustained_poor_absolute
            || relationship_breakdown;
        // Confidence collapses faster than the mood counters — a board
        // sliding toward zero goes public BEFORE it reaches the axe, so
        // the critical band is what triggers the ultimatum.
        let confidence_critical = self.confidence.level <= 20;
        let ultimatum_danger = crisis || confidence_critical;

        // Meetings + matching decisions.
        if crisis {
            result.manager_meeting = Some(BoardManagerMeeting::Crisis);
            result.decisions.push(BoardDecision::HoldCrisisMeeting);
        } else if result.underperforming
            || matches!(self.mood.state, BoardMoodState::Poor)
            || self.pressure.demands_meeting(self.ownership.ownership_type)
        {
            result.manager_meeting = Some(BoardManagerMeeting::Warning);
            result.decisions.push(BoardDecision::IssueFormalWarning);
        } else if matches!(self.mood.state, BoardMoodState::Excellent) && performance_delta >= 3 {
            result.manager_meeting = Some(BoardManagerMeeting::Backing);
            result.decisions.push(BoardDecision::IssueManagerBacking);
        }

        // First month in the danger zone = the public ultimatum. The
        // result carries the announcement so the squad reacts once.
        // Respects the early-season grace like the sack itself.
        if ultimatum_danger && enough_data && !self.manager_on_final_warning {
            self.manager_on_final_warning = true;
            result.manager_ultimatum_announced = true;
        }

        // Results picked up while on the final warning save the job —
        // the ultimatum lapses quietly (the backing IS the survival).
        // Keyed to the TABLE, not the board's mood: the mood trails
        // results by months, and an ultimatum answered with wins must
        // not become a sack while the boardroom is still sulking. If
        // form slides again, a NEW ultimatum is announced and the
        // squad reacts afresh.
        let form_recovering = !result.underperforming;
        if form_recovering {
            self.manager_on_final_warning = false;
        }

        // The axe: only for a manager already living on the final
        // warning whose situation stayed terminal — total confidence
        // collapse or the full crisis picture, with no visible upturn.
        if enough_data
            && already_on_final_warning
            && (zero_confidence || crisis)
            && !form_recovering
        {
            result.manager_sacked = true;
            result.decisions.push(BoardDecision::SackManager);
            // Reset confidence / relationship so the successor starts neutral.
            self.confidence.level = 50;
            self.poor_mood_months = 0;
            self.relationship.reset();
            self.manager_on_final_warning = false;
        }
    }

    /// Open this season's board promises from the long-term goal / season
    /// targets, the youth brief, and any capex the board just deferred on
    /// affordability. Idempotent per type via `has_active`, so a promise is
    /// never duplicated inside its window (keeps the ledger bounded).
    fn open_season_promises(
        &mut self,
        ctx: &BoardContext,
        today: NaiveDate,
        facility_decisions: &[BoardDecision],
    ) {
        let season_due = today + Duration::days(330);

        // Headline season-outcome promise (survival / continental / title).
        if let Some(kind) = self.season_outcome_promise(ctx) {
            if !self.promises.has_active(kind) {
                self.promises
                    .add(BoardPromise::new(kind, today, season_due));
            }
        }

        // Youth-minutes commitment for development-minded or member-owned
        // boards — they pledge a genuine pathway for academy players.
        let youth_minded = matches!(self.vision.youth_focus, VisionYouthFocus::DevelopYouth)
            || matches!(self.ownership.ownership_type, OwnershipType::MemberOwned);
        if youth_minded && !self.promises.has_active(PromiseType::YouthMinutes) {
            self.promises.add(BoardPromise::new(
                PromiseType::YouthMinutes,
                today,
                season_due,
            ));
        }

        // A requested upgrade the board declined *purely* on affordability
        // becomes a "we'll revisit next season" facility promise. Due a
        // little past the next season start so next year's review has a
        // chance to deliver it before it's judged overdue.
        let deferred_capex = facility_decisions.iter().any(|d| {
            matches!(
                d,
                BoardDecision::RejectFacilityUpgrade {
                    reason: DecisionReason::DebtTooHigh,
                    ..
                }
            )
        });
        if deferred_capex && !self.promises.has_active(PromiseType::FacilityImprovement) {
            let revisit_due = today + Duration::days(400);
            self.promises.add(BoardPromise::new(
                PromiseType::FacilityImprovement,
                today,
                revisit_due,
            ));
        }
    }

    /// The headline season-outcome promise type, derived from the explicit
    /// long-term goal where set, otherwise inferred from where the board
    /// expects to finish. A comfortable mid-table brief makes no headline
    /// promise (returns `None`).
    fn season_outcome_promise(&self, ctx: &BoardContext) -> Option<PromiseType> {
        if let Some(goal) = self.vision.long_term_goal {
            match goal {
                LongTermGoal::WinLeague | LongTermGoal::PromotionToTopFlight => {
                    return Some(PromiseType::TitleChallenge);
                }
                LongTermGoal::WinContinental | LongTermGoal::EstablishTopHalf => {
                    return Some(PromiseType::ContinentalQualification);
                }
                LongTermGoal::Survive => return Some(PromiseType::Survival),
                // A domestic-cup goal isn't a league-table promise.
                LongTermGoal::WinDomesticCup => {}
            }
        }

        let targets = self.season_targets.as_ref()?;
        if ctx.league_size == 0 {
            return None;
        }
        let frac = targets.expected_position as f32 / ctx.league_size as f32; // 0 = top
        if frac <= 0.15 {
            Some(PromiseType::TitleChallenge)
        } else if frac <= 0.35 {
            Some(PromiseType::ContinentalQualification)
        } else if frac >= 0.80 {
            Some(PromiseType::Survival)
        } else {
            None
        }
    }

    /// Resolve outstanding promises against this tick's decisions and league
    /// standing, then reward the manager relationship for any kept. Overdue
    /// breakage is handled separately at season start (`break_overdue`).
    fn resolve_promises(&mut self, ctx: &BoardContext, today: NaiveDate, result: &mut BoardResult) {
        let phase = SeasonPhase::classify(ctx.matches_played, ctx.total_matches);
        let mut reward = 0i32;

        // Decision-driven fulfilment: the board delivered what it pledged.
        let delivered_funds = result
            .decisions
            .iter()
            .any(|d| matches!(d, BoardDecision::IncreaseTransferBudget { .. }));
        if delivered_funds {
            if let Some(r) = self.promises.fulfil(PromiseType::TransferBudget) {
                reward += r as i32;
            }
        }
        let upgraded_facility = result
            .decisions
            .iter()
            .any(|d| matches!(d, BoardDecision::ApproveFacilityUpgrade { .. }));
        if upgraded_facility {
            if let Some(r) = self.promises.fulfil(PromiseType::FacilityImprovement) {
                reward += r as i32;
            }
        }

        // Youth pathway visibly delivering.
        if ctx.academy_graduates_this_season > 0 || ctx.u21_minutes_share >= 0.25 {
            if let Some(r) = self.promises.fulfil(PromiseType::YouthMinutes) {
                reward += r as i32;
            }
        }

        // Season-outcome promises are judged once the table means something.
        if phase.can_judge_table() && ctx.league_position > 0 {
            if ctx.distance_to_relegation > 0 {
                if let Some(r) = self.promises.fulfil(PromiseType::Survival) {
                    reward += r as i32;
                }
            }
            if ctx.distance_to_europe_or_playoff <= 0 {
                if let Some(r) = self.promises.fulfil(PromiseType::ContinentalQualification) {
                    reward += r as i32;
                }
            }
            if ctx.league_position <= 2 {
                if let Some(r) = self.promises.fulfil(PromiseType::TitleChallenge) {
                    reward += r as i32;
                }
            }
        }

        // A public backing carries a budget commitment for the next window.
        // Created after fulfilment so it persists to be delivered later (and
        // breaks, denting trust, if the board never follows through).
        if matches!(result.manager_meeting, Some(BoardManagerMeeting::Backing))
            && !self.promises.has_active(PromiseType::TransferBudget)
        {
            let due = today + Duration::days(210);
            self.promises
                .add(BoardPromise::new(PromiseType::TransferBudget, today, due));
        }

        if reward != 0 {
            self.relationship.adjust_communication(reward);
        }
    }

    /// Refresh the pressure gauges from this month's context: decay, then
    /// re-derive the hard-number gauges and fold in inferable narrative
    /// events (relegation scrap, winless run, promotion push, youth break).
    fn refresh_pressure(&mut self, ctx: &BoardContext) {
        self.pressure.decay();
        self.pressure
            .set_financial(ctx.wage_budget_usage, ctx.debt_ratio, ctx.profit_loss_12m);
        self.pressure.set_regulatory(
            matches!(ctx.ffp_status, FfpStatus::Breach),
            matches!(ctx.ffp_status, FfpStatus::Watchlist),
        );
        self.pressure.set_dressing_room(ctx.key_player_unrest_count);

        if ctx.matches_played >= 5 && ctx.distance_to_relegation <= 0 {
            self.pressure.apply_event(SupporterEvent::InRelegationZone);
        }
        if ctx.matches_played >= 5 && ctx.recent_wins == 0 && ctx.recent_losses >= 3 {
            self.pressure.apply_event(SupporterEvent::LongWinlessRun);
        }
        if ctx.league_position > 0 && ctx.distance_to_europe_or_playoff <= 0 {
            self.pressure.apply_event(SupporterEvent::InPromotionRace);
        }
        if ctx.academy_graduates_this_season > 0 {
            self.pressure
                .apply_event(SupporterEvent::YouthProspectBreakthrough);
        }
        if ctx.supporter_mood < 0.35 {
            self.pressure.supporter_pressure = self.pressure.supporter_pressure.max(40);
        }
    }

    /// Emit this tick's budget / FFP / forced-sale decisions as the single
    /// source of truth for transfer-budget movement.
    ///
    /// The historical bug: `process` cut the budget 25% on Poor mood *and*
    /// `emit_financial_decisions` could emit an FFP `CutTransferBudget`
    /// amount in the very same month, double-punishing the club. Here the
    /// budget moves by **at most one** decision per tick — a cut OR an
    /// increase, never both, and never stacked with a percentage tweak in
    /// `process` (that path has been removed). Player-sale demands are
    /// informational and don't touch the budget, so they're emitted
    /// independently.
    fn emit_budget_decisions(
        &self,
        ctx: &BoardContext,
        targets: &SeasonTargets,
        performance_delta: i32,
        result: &mut BoardResult,
    ) {
        let budget = targets.transfer_budget.max(0) as i64;
        let austere = matches!(
            self.vision.financial_stance,
            FinancialStance::Conservative | FinancialStance::Austerity
        );
        let breach = matches!(ctx.ffp_status, FfpStatus::Breach);

        // ── Player-sale demands (informational; never move the budget). ──
        if breach && (austere || self.ownership.ownership_type.resale_driven()) {
            result.decisions.push(BoardDecision::DemandPlayerSale {
                reason: DecisionReason::FfpPressure,
            });
        } else if !breach && ctx.wage_budget_usage > 1.1 && austere {
            result.decisions.push(BoardDecision::DemandPlayerSale {
                reason: DecisionReason::WageControl,
            });
        }

        // ── The single budget adjustment for the tick. FFP breach is the
        // dominant grievance and pre-empts a mood cut; an unhappy board
        // never simultaneously hands out money. ──
        if breach {
            let cut = (budget / 3).max(0);
            if cut > 0 {
                result.decisions.push(BoardDecision::CutTransferBudget {
                    amount: cut,
                    reason: DecisionReason::FfpPressure,
                });
            }
            result.cut_transfer_budget = true; // UI flag only
            return;
        }

        if matches!(self.mood.state, BoardMoodState::Poor) {
            // Sustained poor mood squeezes the war chest by ~25%.
            let cut = (budget / 4).max(0);
            if cut > 0 {
                result.decisions.push(BoardDecision::CutTransferBudget {
                    amount: cut,
                    reason: DecisionReason::Underperformance,
                });
            }
            result.cut_transfer_budget = true; // UI flag only
            return;
        }

        // Positive side: a wealthy, risk-tolerant owner's injection after a
        // strong run takes precedence over the smaller excellent-mood bonus.
        let strong = self.latest_scores.sporting > 18.0 && self.latest_scores.financial > 0.0;
        if strong && self.ownership.injection_appetite() > 0.6 {
            let inject = (budget / 4).max(2_000_000);
            result
                .decisions
                .push(BoardDecision::IncreaseTransferBudget {
                    amount: inject,
                    reason: DecisionReason::OwnerInjection,
                });
            result.bonus_transfer_funds = true; // UI flag only
        } else if matches!(self.mood.state, BoardMoodState::Excellent) && performance_delta > 3 {
            // Modest reward (~20% of target) for clearly beating expectations.
            let bonus = (budget / 5).max(0);
            if bonus > 0 {
                result
                    .decisions
                    .push(BoardDecision::IncreaseTransferBudget {
                        amount: bonus,
                        reason: DecisionReason::Overperformance,
                    });
                result.bonus_transfer_funds = true; // UI flag only
            }
        }
    }

    /// Derive the ownership archetype + opening vision from durable club
    /// signals. Runs once, on the first simulate tick that has context.
    fn bootstrap_personality(&mut self, ctx: &BoardContext, seed: u32) {
        let owner = OwnershipModel::derive(
            ctx.reputation_score,
            ctx.balance,
            ctx.country_economic_factor,
            seed,
        );

        // Map ownership → legacy chairman knobs so budget / patience logic
        // reflects the derived owner.
        self.chairman.ambition = match owner.ownership_type {
            OwnershipType::StateBacked => ChairmanAmbition::Reckless,
            OwnershipType::Consortium if owner.wealth >= 70 => ChairmanAmbition::Ambitious,
            OwnershipType::FamilyOwned | OwnershipType::MemberOwned => {
                ChairmanAmbition::Conservative
            }
            _ => ChairmanAmbition::Balanced,
        };
        self.chairman.patience = match owner.ownership_type {
            OwnershipType::StateBacked | OwnershipType::PrivateEquity => ChairmanPatience::Low,
            OwnershipType::MemberOwned | OwnershipType::FamilyOwned => ChairmanPatience::High,
            _ => ChairmanPatience::Medium,
        };

        // Opening vision from archetype — gives clubs distinct stories.
        self.vision.preferred_squad_profile = match owner.ownership_type {
            OwnershipType::StateBacked => SquadProfile::Stars,
            OwnershipType::PrivateEquity => SquadProfile::ResaleValue,
            OwnershipType::MemberOwned => SquadProfile::Youth,
            OwnershipType::Consortium => SquadProfile::PrimeAge,
            OwnershipType::LocalBusiness if ctx.reputation_score < 0.4 => SquadProfile::Youth,
            _ => SquadProfile::Balanced,
        };
        self.vision.infrastructure_priority = match owner.ownership_type {
            OwnershipType::MemberOwned => InfrastructurePriority::Youth,
            OwnershipType::FamilyOwned | OwnershipType::StateBacked => {
                InfrastructurePriority::Stadium
            }
            OwnershipType::Consortium => InfrastructurePriority::Commercial,
            OwnershipType::PrivateEquity => InfrastructurePriority::None,
            OwnershipType::LocalBusiness => InfrastructurePriority::Training,
        };
        self.vision.manager_autonomy = if owner.interference >= 60 {
            ManagerAutonomy::Low
        } else if owner.interference >= 35 {
            ManagerAutonomy::Medium
        } else {
            ManagerAutonomy::High
        };
        self.vision.review_frequency = match owner.ownership_type {
            OwnershipType::MemberOwned | OwnershipType::FamilyOwned => ReviewFrequency::Quarterly,
            _ => ReviewFrequency::Monthly,
        };
        self.vision.financial_stance = match owner.ownership_type {
            OwnershipType::StateBacked => FinancialStance::Ambitious,
            OwnershipType::PrivateEquity
            | OwnershipType::MemberOwned
            | OwnershipType::FamilyOwned => FinancialStance::Conservative,
            _ => FinancialStance::Balanced,
        };

        self.ownership = owner;
        self.personality_initialized = true;
    }

    /// Season-start facility review with a per-board cooldown. A funded
    /// upgrade can only be approved once every `FacilityReview::COOLDOWN_SEASONS`
    /// seasons — without this a deep-pocketed owner would rubber-stamp an
    /// upgrade every single year, which no real board does. Records the
    /// approval year so the next season's review is suppressed; rejections /
    /// news-only outcomes don't start the cooldown.
    fn run_facility_review(
        &mut self,
        board_ctx: &BoardContext,
        current_year: i32,
    ) -> Vec<BoardDecision> {
        let cooldown_active = self
            .last_facility_upgrade_year
            .map(|y| current_year - y < FacilityReview::COOLDOWN_SEASONS)
            .unwrap_or(false);
        if cooldown_active {
            return Vec::new();
        }

        let decisions = FacilityReview::run(board_ctx, &self.vision, &self.ownership);
        if decisions
            .iter()
            .any(|d| matches!(d, BoardDecision::ApproveFacilityUpgrade { .. }))
        {
            self.last_facility_upgrade_year = Some(current_year);
        }
        decisions
    }

    /// Monthly takeover watch. Opens / resolves rumours and, on completion,
    /// installs a new owner and resets strategy + relationship.
    ///
    /// The takeover roll is *deterministic*: `GlobalContext` carries no
    /// seeded simulation RNG, so rather than draw from the global
    /// (unreplayable) `IntegerUtils::random`, we derive a stable 0..99 roll
    /// from the club id, the current date, and the months spent in the
    /// current takeover status. Identical club/date/state therefore always
    /// produces the identical decision, so saves and tests replay exactly.
    fn tick_takeover(&mut self, ctx: &BoardContext, today: NaiveDate, result: &mut BoardResult) {
        let roll = takeover_roll(result.club_id, today, self.takeover.months_in_status);
        if let Some(decision) = self.takeover.tick(&self.ownership, ctx, roll) {
            match decision {
                BoardDecision::StartTakeoverRumour => {
                    result.decisions.push(BoardDecision::StartTakeoverRumour);
                }
                BoardDecision::CompleteTakeover => {
                    self.apply_takeover_completion(result.club_id ^ 0x9E37_79B9);
                    result.decisions.push(BoardDecision::CompleteTakeover);
                }
                _ => {}
            }
        }

        // A collapsed takeover leaves instability: morale dip + a short
        // budget freeze. Emit the freeze as an explicit decision (the
        // legacy mood-percentage path in `process` no longer fires).
        if self.takeover.just_failed {
            self.confidence.level = (self.confidence.level - 8).clamp(0, 100);
            let freeze = self
                .season_targets
                .as_ref()
                .map(|t| (t.transfer_budget.max(0) as i64 / 5).max(0))
                .unwrap_or(0);
            if freeze > 0 {
                result.decisions.push(BoardDecision::CutTransferBudget {
                    amount: freeze,
                    reason: DecisionReason::FinancialDiscipline,
                });
            }
            result.cut_transfer_budget = true; // UI flag only
        }
    }

    /// Install a new owner after a successful takeover and reset the club's
    /// strategy + manager relationship to match the fresh mandate.
    ///
    /// The new owner's archetype dictates the strategy rather than a blanket
    /// "buy stars, win the league": a sovereign buyer chases trophies, a
    /// private-equity buyer chases resale and wage discipline, and a
    /// consortium builds a balanced prime-age side aiming for the top half /
    /// continental places.
    fn apply_takeover_completion(&mut self, seed: u32) {
        let owner = TakeoverEngine::post_takeover_owner(seed);
        match owner.ownership_type {
            OwnershipType::StateBacked => {
                // Sovereign wealth: trophies now, money no object.
                self.chairman.ambition = ChairmanAmbition::Reckless;
                self.chairman.patience = ChairmanPatience::Low;
                self.vision.preferred_squad_profile = SquadProfile::Stars;
                self.vision.financial_stance = FinancialStance::Ambitious;
                self.vision.long_term_goal = Some(LongTermGoal::WinLeague);
                self.vision.infrastructure_priority = InfrastructurePriority::Stadium;
                self.vision.manager_autonomy = ManagerAutonomy::Low;
            }
            OwnershipType::PrivateEquity => {
                // Leveraged buyer: trade players for profit, control wages,
                // monetise the brand. Ambitious but financially disciplined.
                self.chairman.ambition = ChairmanAmbition::Ambitious;
                self.chairman.patience = ChairmanPatience::Low;
                self.vision.preferred_squad_profile = SquadProfile::ResaleValue;
                self.vision.financial_stance = FinancialStance::Conservative;
                self.vision.long_term_goal = Some(LongTermGoal::EstablishTopHalf);
                self.vision.infrastructure_priority = InfrastructurePriority::Commercial;
                self.vision.manager_autonomy = ManagerAutonomy::Medium;
            }
            // Consortium (and any future archetype): patient, balanced build
            // around prime-age players, aiming high but living within means.
            _ => {
                self.chairman.ambition = ChairmanAmbition::Ambitious;
                self.chairman.patience = ChairmanPatience::Medium;
                self.vision.preferred_squad_profile = SquadProfile::PrimeAge;
                self.vision.financial_stance = FinancialStance::Balanced;
                self.vision.long_term_goal = Some(LongTermGoal::WinContinental);
                self.vision.infrastructure_priority = InfrastructurePriority::Training;
                self.vision.manager_autonomy = ManagerAutonomy::Medium;
            }
        }
        self.ownership = owner;
        self.relationship.reset();
        self.confidence.level = 60;
    }

    fn is_director_contract_expiring(&self, simulation_ctx: &SimulationContext) -> bool {
        match &self.director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    /// Stand up a fresh director contract — four-year term, salary
    /// indexed to board ambition. This is the board's own administrative
    /// slot, separate from the team's DoF staff member.
    fn run_director_election(&mut self, ctx: &SimulationContext) {
        use crate::{StaffPosition, StaffStatus};
        let base_salary: u32 = match self.chairman.ambition {
            ChairmanAmbition::Reckless | ChairmanAmbition::Ambitious => 200_000,
            ChairmanAmbition::Balanced => 120_000,
            ChairmanAmbition::Conservative => 80_000,
        };
        let expires = ctx
            .date
            .date()
            .with_year(ctx.date.date().year() + 4)
            .unwrap_or(ctx.date.date());
        self.director = Some(StaffClubContract::new(
            base_salary,
            expires,
            StaffPosition::Director,
            StaffStatus::Active,
        ));
    }

    fn is_sport_director_contract_expiring(&self, simulation_ctx: &SimulationContext) -> bool {
        match &self.sport_director {
            Some(d) => d.is_expired(simulation_ctx),
            None => false,
        }
    }

    /// Stand up a sport director contract — three-year term; this is a
    /// more "football-side" role so salary floor is slightly higher.
    fn run_sport_director_election(&mut self, ctx: &SimulationContext) {
        use crate::{StaffPosition, StaffStatus};
        let base_salary: u32 = match self.chairman.ambition {
            ChairmanAmbition::Reckless | ChairmanAmbition::Ambitious => 250_000,
            ChairmanAmbition::Balanced => 150_000,
            ChairmanAmbition::Conservative => 100_000,
        };
        let expires = ctx
            .date
            .date()
            .with_year(ctx.date.date().year() + 3)
            .unwrap_or(ctx.date.date());
        self.sport_director = Some(StaffClubContract::new(
            base_salary,
            expires,
            StaffPosition::DirectorOfFootball,
            StaffStatus::Active,
        ));
    }
}

fn is_board_urgent_reason(reason: &TransferNeedReason) -> bool {
    matches!(
        reason,
        TransferNeedReason::FormationGap
            | TransferNeedReason::QualityUpgrade
            | TransferNeedReason::DepthCover
            | TransferNeedReason::LoanToFillSquad
            | TransferNeedReason::SquadPadding
            | TransferNeedReason::InjuryCoverLoan
            | TransferNeedReason::OpportunisticLoanUpgrade
    )
}

/// Deterministic 0..99 takeover roll. Mixes the club id, the calendar day,
/// and the months-in-status salt with a splitmix64 finalizer so the output
/// is well-distributed yet fully reproducible from `(club, date, state)` —
/// no global RNG, so saves and tests replay identically.
fn takeover_roll(club_id: u32, date: NaiveDate, months_in_status: u8) -> u8 {
    let day = date.num_days_from_ce() as u64;
    let mut x = ((club_id as u64) << 32)
        ^ day.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ ((months_in_status as u64)
            .wrapping_add(1)
            .wrapping_mul(0xD1B5_4A32_D192_ED03));
    // splitmix64 finalizer.
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x % 100) as u8
}

/// How poorly does `tactic` embody `style`? 0 = fine, up to 2 = strong
/// clash. Used as a monthly confidence drag so the board slowly loses
/// patience with a manager whose football doesn't match what they were
/// hired to deliver. `Balanced` never drags.
fn style_mismatch_drag(style: VisionPlayingStyle, tactic: MatchTacticType) -> i32 {
    use MatchTacticType::*;
    use VisionPlayingStyle::*;

    // Bias each formation on two axes: attacking weight (more forwards)
    // and possession weight (tight midfield). Hand-tuned from conventional
    // football wisdom rather than derived from match-engine values.
    let (attacking, possession) = match tactic {
        T343 => (2, 0),
        T4222 => (2, 1),
        T433 => (1, 2),
        T4231 => (1, 2),
        T4312 => (1, 1),
        T442 => (0, 0),
        T442Diamond | T442Narrow | T442DiamondWide => (0, 1),
        T352 => (0, 0),
        T4411 => (-1, 0),
        T4141 => (-1, 1),
        T451 => (-2, 0),
        T1333 => (-2, -1),
    };

    match style {
        Balanced => 0,
        AttackingFootball => (1 - attacking).max(0),
        DefensiveSolid => (1 + attacking).max(0),
        Possession => (1 - possession).max(0),
        DirectPlay => (possession).max(0),
        HighPressing => (1 - possession).max(0) + (0 - attacking).max(0),
        CounterAttack => (attacking - 1).max(0),
    }
}

#[cfg(test)]
mod style_fit_tests {
    use super::*;

    fn transfer_proposal(
        fee: f64,
        allocated_budget: f64,
        priority: TransferNeedPriority,
        reason: TransferNeedReason,
    ) -> BoardTransferProposal {
        BoardTransferProposal {
            fee,
            allocated_budget,
            remaining_transfer_budget: 10_000_000.0,
            priority,
            reason,
            player_age: Some(25),
            player_ability: Some(65),
            squad_avg_ability: 60,
            shortlist_score: 1.0,
            dossier: None,
            economics: None,
        }
    }

    #[test]
    fn balanced_vision_never_drags() {
        for t in MatchTacticType::all() {
            assert_eq!(style_mismatch_drag(VisionPlayingStyle::Balanced, t), 0);
        }
    }

    #[test]
    fn attacking_vision_punishes_defensive_formations() {
        assert!(
            style_mismatch_drag(VisionPlayingStyle::AttackingFootball, MatchTacticType::T451) > 0
        );
        assert!(
            style_mismatch_drag(
                VisionPlayingStyle::AttackingFootball,
                MatchTacticType::T1333
            ) > 0
        );
    }

    #[test]
    fn attacking_vision_accepts_attacking_formations() {
        assert_eq!(
            style_mismatch_drag(VisionPlayingStyle::AttackingFootball, MatchTacticType::T343),
            0
        );
        assert_eq!(
            style_mismatch_drag(
                VisionPlayingStyle::AttackingFootball,
                MatchTacticType::T4222
            ),
            0
        );
    }

    #[test]
    fn defensive_vision_punishes_attacking_formations() {
        assert!(style_mismatch_drag(VisionPlayingStyle::DefensiveSolid, MatchTacticType::T343) > 0);
        assert!(
            style_mismatch_drag(VisionPlayingStyle::DefensiveSolid, MatchTacticType::T4222) > 0
        );
    }

    #[test]
    fn possession_vision_accepts_possession_formations() {
        assert_eq!(
            style_mismatch_drag(VisionPlayingStyle::Possession, MatchTacticType::T433),
            0
        );
        assert_eq!(
            style_mismatch_drag(VisionPlayingStyle::Possession, MatchTacticType::T4231),
            0
        );
    }

    #[test]
    fn counter_attack_vision_prefers_modest_formations() {
        // T442 = balanced → fits counter-attack fine.
        assert_eq!(
            style_mismatch_drag(VisionPlayingStyle::CounterAttack, MatchTacticType::T442),
            0
        );
        // T343 = all-out attack → clashes with counter-attack's defensive base.
        assert!(style_mismatch_drag(VisionPlayingStyle::CounterAttack, MatchTacticType::T343) > 0);
    }

    #[test]
    fn conservative_board_vetoes_excessive_transfer_overrun() {
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Conservative;
        board.chairman.ambition = ChairmanAmbition::Conservative;

        let proposal = transfer_proposal(
            2_000_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );

        assert!(matches!(
            board.review_transfer_proposal(&proposal),
            BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
        ));
    }

    #[test]
    fn ambitious_board_backs_critical_squad_gap_within_cash_limit() {
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Ambitious;
        board.chairman.ambition = ChairmanAmbition::Ambitious;
        board.confidence.level = 80;

        let proposal = transfer_proposal(
            2_250_000.0,
            1_000_000.0,
            TransferNeedPriority::Critical,
            TransferNeedReason::FormationGap,
        );

        assert!(board.review_transfer_proposal(&proposal).is_approved());
    }

    #[test]
    fn strong_dossier_relaxes_board_tolerance() {
        // A proposal that's borderline on budget normally gets flagged
        // financial-discipline. With a strong dossier (consensus + chief
        // scout backing + high confidence) the board approves anyway.
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Balanced;
        let mut proposal = transfer_proposal(
            1_700_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        // Without dossier — borderline.
        let baseline = board.review_transfer_proposal(&proposal);
        // With strong dossier — should approve.
        proposal.dossier = Some(BoardDossierSummary {
            scout_votes: 3,
            chief_scout_support: true,
            avg_confidence: 0.85,
            avg_role_fit: 1.10,
            risk_flag_count: 0,
            consensus_score: 3.0,
            data_support: true,
            matches_watched: 4,
        });
        let with_dossier = board.review_transfer_proposal(&proposal);
        // Dossier-backed should be at least as approved as the baseline.
        // Specifically: a strong dossier should never downgrade an
        // Approved into a Vetoed.
        if matches!(baseline, BoardTransferDecision::Vetoed(_)) {
            assert!(
                with_dossier.is_approved(),
                "strong dossier should rescue a borderline veto, got {:?}",
                with_dossier
            );
        } else {
            assert!(with_dossier.is_approved());
        }
    }

    #[test]
    fn split_vote_dossier_with_risk_flags_vetoes() {
        // Two scouts watching, split decision, multiple risk flags →
        // board sends it back to recruitment instead of approving.
        let board = ClubBoard::new();
        let mut proposal = transfer_proposal(
            900_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        proposal.dossier = Some(BoardDossierSummary {
            scout_votes: 3,
            chief_scout_support: false,
            avg_confidence: 0.55,
            avg_role_fit: 0.95,
            risk_flag_count: 3,
            consensus_score: 0.0,
            data_support: false,
            matches_watched: 1,
        });
        let decision = board.review_transfer_proposal(&proposal);
        assert!(
            matches!(decision, BoardTransferDecision::Vetoed(_)),
            "split-vote risk-heavy dossier must veto, got {:?}",
            decision
        );
    }

    #[test]
    fn dossier_is_optional_legacy_path_unchanged() {
        // Ensure the no-dossier path produces exactly the same result
        // as the pre-recruitment-meeting baseline. The whole point of
        // the optional field is backwards compatibility.
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Conservative;
        let proposal = transfer_proposal(
            2_000_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        let decision = board.review_transfer_proposal(&proposal);
        assert!(matches!(
            decision,
            BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
        ));
    }

    #[test]
    fn youth_vision_marks_old_development_signing_as_conditional() {
        let mut board = ClubBoard::new();
        board.vision.youth_focus = VisionYouthFocus::DevelopYouth;

        let mut proposal = transfer_proposal(
            750_000.0,
            1_000_000.0,
            TransferNeedPriority::Optional,
            TransferNeedReason::DevelopmentSigning,
        );
        proposal.player_age = Some(31);

        assert!(matches!(
            board.review_transfer_proposal(&proposal),
            BoardTransferDecision::Conditional(BoardTransferConcern::ConflictsWithVision)
        ));
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn make_ctx(income: i64, outcome: i64, ffp: FfpStatus) -> BoardContext {
        let mut c = BoardContext::new();
        c.balance = 10_000_000;
        c.total_annual_wages = 50_000_000;
        c.reputation_score = 0.6;
        c.country_economic_factor = 1.0;
        c.country_price_level = 1.0;
        c.trailing_annual_income = income;
        c.trailing_annual_outcome = outcome;
        c.ffp_status = ffp;
        c
    }

    fn calc(ctx: &BoardContext) -> SeasonTargets {
        let mut board = ClubBoard::new();
        board.calculate_season_targets(ctx);
        board.season_targets.expect("should produce targets")
    }

    #[test]
    fn budget_shrinks_under_ffp_breach() {
        let clean = make_ctx(120_000_000, 90_000_000, FfpStatus::Clean);
        let breach = make_ctx(120_000_000, 90_000_000, FfpStatus::Breach);
        let watchlist = make_ctx(120_000_000, 90_000_000, FfpStatus::Watchlist);

        let t_clean = calc(&clean);
        let t_breach = calc(&breach);
        let t_watch = calc(&watchlist);

        assert!(
            t_breach.transfer_budget < t_clean.transfer_budget,
            "breach must cut transfer budget vs clean: {} vs {}",
            t_breach.transfer_budget,
            t_clean.transfer_budget
        );
        assert!(
            t_watch.transfer_budget < t_clean.transfer_budget,
            "watchlist must cut transfer budget vs clean"
        );
        assert!(
            t_breach.transfer_budget <= t_watch.transfer_budget,
            "breach must cut harder than watchlist"
        );
    }

    #[test]
    fn budget_zero_when_outflows_exceed_inflows_and_no_seed_cash() {
        let mut ctx = make_ctx(80_000_000, 95_000_000, FfpStatus::Clean);
        ctx.balance = 0; // no seed cash
        let t = calc(&ctx);
        assert_eq!(t.transfer_budget, 0);
    }

    #[test]
    fn cold_start_with_zero_history_falls_back_to_cash_seed() {
        let mut ctx = make_ctx(0, 0, FfpStatus::Clean);
        ctx.balance = 50_000_000;
        ctx.reputation_score = 0.85; // 0.30 seed pct
        let t = calc(&ctx);
        assert!(t.transfer_budget > 0);
    }

    #[test]
    fn wage_budget_distress_ratio_lower_than_clean() {
        let clean = make_ctx(100_000_000, 60_000_000, FfpStatus::Clean);
        let distress = make_ctx(100_000_000, 60_000_000, FfpStatus::Breach);
        let t_clean = calc(&clean);
        let t_distress = calc(&distress);
        assert!(
            t_distress.wage_budget <= t_clean.wage_budget,
            "distressed wage budget should not exceed clean"
        );
    }

    #[test]
    fn ambitious_owner_with_title_goal_raises_expected_position() {
        let mut ctx = make_ctx(120_000_000, 90_000_000, FfpStatus::Clean);
        ctx.league_size = 20;
        ctx.reputation_score = 0.55;

        let mut ambitious = ClubBoard::new();
        ambitious.chairman.ambition = ChairmanAmbition::Reckless;
        ambitious.vision.long_term_goal = Some(LongTermGoal::WinLeague);
        ambitious.calculate_season_targets(&ctx);
        let amb = ambitious.season_targets.unwrap().expected_position;

        let mut modest = ClubBoard::new();
        modest.calculate_season_targets(&ctx);
        let mid = modest.season_targets.unwrap().expected_position;

        assert!(
            amb < mid,
            "title-chasing owner expects higher: {amb} vs {mid}"
        );
        assert!(
            amb <= 3,
            "a reckless title goal targets the very top: {amb}"
        );
    }

    #[test]
    fn low_rep_survival_side_is_not_expected_to_finish_mid_table() {
        let mut ctx = make_ctx(40_000_000, 38_000_000, FfpStatus::Clean);
        ctx.league_size = 20;
        ctx.reputation_score = 0.18;

        let mut board = ClubBoard::new();
        board.vision.long_term_goal = Some(LongTermGoal::Survive);
        board.chairman.ambition = ChairmanAmbition::Conservative;
        board.calculate_season_targets(&ctx);
        let t = board.season_targets.unwrap();

        assert!(
            t.expected_position >= 14,
            "a survival side expects the lower reaches, not mid-table: {}",
            t.expected_position
        );
        assert!(t.min_acceptable_position >= t.expected_position);
    }

    #[test]
    fn conservative_small_club_is_not_handed_an_impossible_finish() {
        let mut ctx = make_ctx(30_000_000, 29_000_000, FfpStatus::Clean);
        ctx.league_size = 20;
        ctx.reputation_score = 0.30;

        let mut board = ClubBoard::new();
        board.chairman.ambition = ChairmanAmbition::Conservative;
        board.calculate_season_targets(&ctx);
        let t = board.season_targets.unwrap();

        assert!(
            t.expected_position >= 10,
            "a modest club shouldn't be told to finish near the top: {}",
            t.expected_position
        );
    }

    #[test]
    fn lower_division_reputable_club_expects_to_push_for_promotion() {
        let mut top_flight = make_ctx(60_000_000, 55_000_000, FfpStatus::Clean);
        top_flight.league_size = 20;
        top_flight.reputation_score = 0.5;
        top_flight.league_tier = 1;
        let mut second_tier = top_flight.clone();
        second_tier.league_tier = 2;

        let mut a = ClubBoard::new();
        a.calculate_season_targets(&top_flight);
        let mut b = ClubBoard::new();
        b.calculate_season_targets(&second_tier);

        assert!(
            b.season_targets.unwrap().expected_position
                <= a.season_targets.unwrap().expected_position,
            "the same club expects a higher finish in a weaker division"
        );
    }

    #[test]
    fn season_phase_delays_table_judgment_until_enough_matches() {
        assert_eq!(SeasonPhase::classify(4, 38), SeasonPhase::TooEarly);
        assert_eq!(SeasonPhase::classify(8, 38), SeasonPhase::Early);
        assert!(!SeasonPhase::classify(8, 38).can_sack_manager());
        assert!(SeasonPhase::classify(16, 38).can_sack_manager());
        assert_eq!(SeasonPhase::classify(32, 38), SeasonPhase::RunIn);
    }
}

/// Long-term goal → ambition multiplier on the season's transfer budget.
/// Title chasers get the biggest budget; survival sides keep their wallet
/// closed. None defaults to a mid-table rating.
fn ambition_budget_multiplier(goal: Option<LongTermGoal>) -> f64 {
    match goal {
        Some(LongTermGoal::WinLeague)
        | Some(LongTermGoal::WinContinental)
        | Some(LongTermGoal::WinDomesticCup)
        | Some(LongTermGoal::PromotionToTopFlight) => 1.35,
        Some(LongTermGoal::EstablishTopHalf) => 1.15,
        Some(LongTermGoal::Survive) => 0.55,
        None => 0.85,
    }
}

/// Target wage-to-revenue ratio. Healthy clubs target 55–65%; distressed
/// clubs squeeze it to 45–50%; reckless owners at the elite tier are
/// allowed up to 70%.
fn wage_revenue_target(ffp: FfpStatus, ambition: ChairmanAmbition, reputation_score: f32) -> f64 {
    let base: f64 = match ffp {
        FfpStatus::Clean => 0.62,
        FfpStatus::Watchlist => 0.55,
        FfpStatus::Breach => 0.48,
    };
    if matches!(ambition, ChairmanAmbition::Reckless) && reputation_score >= 0.75 {
        return 0.70;
    }
    if matches!(ambition, ChairmanAmbition::Conservative) {
        return (base - 0.05_f64).max(0.35);
    }
    base
}

#[cfg(test)]
mod board_behaviour_tests {
    //! Scenario tests for the expanded board: archetype governance,
    //! season-phase sacking protection, FFP reactions, takeovers, and
    //! manager-relationship renewals. These exercise the integrated
    //! `evaluate_performance` / governance / takeover paths end to end.
    use super::*;
    use crate::club::BoardFacility;
    use crate::club::board::ownership::{OwnershipModel, OwnershipType};
    use crate::club::board::takeover::TakeoverStatus;

    fn targets(expected: u8, min_acceptable: u8) -> SeasonTargets {
        SeasonTargets {
            transfer_budget: 30_000_000,
            wage_budget: 50_000_000,
            max_squad_size: 30,
            min_squad_size: 18,
            expected_position: expected,
            min_acceptable_position: min_acceptable,
        }
    }

    fn poor_ctx(matches_played: u8, total: u8, position: u8, size: u8) -> BoardContext {
        let mut c = BoardContext::new();
        c.total_annual_wages = 12_000_000;
        c.balance = -5_000_000;
        c.league_size = size;
        c.league_position = position;
        c.matches_played = matches_played;
        c.total_matches = total;
        c.points_per_match = 0.5;
        c.recent_wins = 0;
        c.recent_losses = 4;
        c.recent_goal_difference = -8;
        c.goal_difference = -25;
        c.distance_to_relegation = -1;
        c
    }

    fn strong_ctx(position: u8, size: u8) -> BoardContext {
        let mut c = BoardContext::new();
        c.total_annual_wages = 12_000_000;
        c.balance = 30_000_000;
        c.league_size = size;
        c.league_position = position;
        c.matches_played = 19;
        c.total_matches = 38;
        c.points_per_match = 2.2;
        c.recent_wins = 4;
        c.recent_losses = 0;
        c.recent_goal_difference = 8;
        c.goal_difference = 25;
        c.distance_to_relegation = 15;
        c.profit_loss_12m = 5_000_000;
        c
    }

    fn proposal(
        fee: f64,
        age: u8,
        ability: u8,
        priority: TransferNeedPriority,
        reason: TransferNeedReason,
    ) -> BoardTransferProposal {
        BoardTransferProposal {
            fee,
            allocated_budget: 1_000_000.0,
            remaining_transfer_budget: 10_000_000.0,
            priority,
            reason,
            player_age: Some(age),
            player_ability: Some(ability),
            squad_avg_ability: 65,
            shortlist_score: 1.0,
            dossier: None,
            economics: None,
        }
    }

    #[test]
    fn early_season_bad_form_does_not_sack_manager() {
        let mut board = ClubBoard::new();
        board.season_targets = Some(targets(5, 8));
        let ctx = poor_ctx(8, 38, 19, 20); // Early phase
        let mut sacked = false;
        for _ in 0..8 {
            let mut r = BoardResult::new();
            board.evaluate_performance(&ctx, &mut r);
            sacked |= r.manager_sacked;
        }
        assert!(!sacked, "early-season form must not cost a job");
    }

    #[test]
    fn run_in_underperformance_can_trigger_sacking() {
        let mut board = ClubBoard::new();
        board.season_targets = Some(targets(5, 8));
        let ctx = poor_ctx(32, 38, 19, 20); // RunIn phase
        let mut sacked = false;
        for _ in 0..12 {
            let mut r = BoardResult::new();
            board.evaluate_performance(&ctx, &mut r);
            if r.manager_sacked {
                sacked = true;
                break;
            }
        }
        assert!(sacked, "sustained run-in collapse should cost the job");
    }

    #[test]
    fn sack_requires_a_lived_ultimatum_month() {
        // The FIRST evaluation that reaches crisis issues the public
        // ultimatum; the sack may only follow on a LATER evaluation —
        // the squad gets a real month to react.
        let mut board = ClubBoard::new();
        board.season_targets = Some(targets(5, 8));
        let ctx = poor_ctx(32, 38, 19, 20); // RunIn phase
        let mut announced_at: Option<usize> = None;
        let mut sacked_at: Option<usize> = None;
        for month in 0..12 {
            let mut r = BoardResult::new();
            board.evaluate_performance(&ctx, &mut r);
            if r.manager_ultimatum_announced && announced_at.is_none() {
                announced_at = Some(month);
            }
            if r.manager_sacked {
                sacked_at = Some(month);
                break;
            }
        }
        let announced = announced_at.expect("a collapse must produce a public ultimatum");
        let sacked = sacked_at.expect("a sustained collapse still costs the job");
        assert!(
            sacked > announced,
            "the ultimatum (month {}) must precede the sack (month {})",
            announced,
            sacked
        );
    }

    #[test]
    fn results_on_final_warning_save_the_job() {
        let mut board = ClubBoard::new();
        board.season_targets = Some(targets(5, 8));
        let poor = poor_ctx(32, 38, 19, 20);
        let mut announced = false;
        for _ in 0..12 {
            let mut r = BoardResult::new();
            board.evaluate_performance(&poor, &mut r);
            if r.manager_sacked {
                panic!("must not sack before the ultimatum has been lived with");
            }
            if r.manager_ultimatum_announced {
                announced = true;
                break;
            }
        }
        assert!(announced, "the collapse must reach the ultimatum stage");

        // Form turns — the warning lapses instead of becoming a sack.
        let strong = strong_ctx(3, 20);
        let mut sacked = false;
        for _ in 0..4 {
            let mut r = BoardResult::new();
            board.evaluate_performance(&strong, &mut r);
            sacked |= r.manager_sacked;
        }
        assert!(
            !sacked,
            "recovered form on the final warning must save the job"
        );
        assert!(
            !board.manager_on_final_warning,
            "the warning lapses once results recover"
        );
    }

    #[test]
    fn ffp_breach_cuts_budget_and_raises_financial_pressure() {
        let mut board = ClubBoard::new();
        board.season_targets = Some(targets(8, 12));
        let mut ctx = strong_ctx(8, 20);
        ctx.ffp_status = FfpStatus::Breach;
        ctx.wage_budget_usage = 1.2;
        ctx.debt_ratio = 1.2;
        ctx.profit_loss_12m = -10_000_000;

        let mut r = BoardResult::new();
        board.evaluate_performance(&ctx, &mut r);

        assert!(
            r.decisions.iter().any(|d| matches!(
                d,
                BoardDecision::CutTransferBudget {
                    reason: DecisionReason::FfpPressure,
                    ..
                }
            )),
            "FFP breach must emit a budget cut: {:?}",
            r.decisions
        );
        assert!(board.pressure.regulatory_pressure > 0);
        assert!(board.pressure.financial_pressure > 0);
    }

    #[test]
    fn reckless_owner_increases_budget_but_lowers_patience() {
        // Elite club, seed 0 -> StateBacked (reckless) ownership.
        let mut ctx = BoardContext::new();
        ctx.reputation_score = 0.9;
        ctx.balance = 50_000_000;
        ctx.country_economic_factor = 1.2;
        ctx.country_price_level = 1.0;
        ctx.trailing_annual_income = 60_000_000;
        ctx.trailing_annual_outcome = 40_000_000;

        let mut reckless = ClubBoard::new();
        reckless.bootstrap_personality(&ctx, 0);
        assert!(matches!(
            reckless.ownership.ownership_type,
            OwnershipType::StateBacked
        ));
        assert!(matches!(
            reckless.chairman.ambition,
            ChairmanAmbition::Reckless
        ));
        assert!(matches!(reckless.chairman.patience, ChairmanPatience::Low));

        reckless.calculate_season_targets(&ctx);
        let reckless_budget = reckless.season_targets.as_ref().unwrap().transfer_budget;

        let mut neutral = ClubBoard::new();
        neutral.calculate_season_targets(&ctx);
        let neutral_budget = neutral.season_targets.as_ref().unwrap().transfer_budget;

        assert!(
            reckless_budget > neutral_budget,
            "reckless owner should out-spend neutral: {reckless_budget} vs {neutral_budget}"
        );
        assert!(
            reckless.chairman.poor_mood_threshold() < ChairmanProfile::new().poor_mood_threshold(),
            "reckless owner should be quicker to act"
        );
    }

    #[test]
    fn conservative_owner_blocks_wage_heavy_transfer() {
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Conservative;
        let mut p = proposal(
            500_000.0,
            26,
            70,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        p.economics = Some(BoardTransferEconomics {
            wage_impact_annual: 5_000_000.0,
            wage_budget_headroom: 0.0,
            contract_length_years: 4,
            ..Default::default()
        });
        assert!(matches!(
            board.review_transfer_proposal(&p),
            BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
        ));
    }

    #[test]
    fn private_equity_board_flags_poor_resale() {
        let mut board = ClubBoard::new();
        board.ownership.ownership_type = OwnershipType::PrivateEquity;
        // An ageing target whose projected resale is far below the fee.
        let mut p = proposal(
            800_000.0,
            30,
            70,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        p.economics = Some(BoardTransferEconomics {
            resale_projection: p.fee * 0.3, // < 40% of fee
            wage_budget_headroom: 50_000_000.0,
            ..Default::default()
        });
        assert!(
            matches!(
                board.review_transfer_proposal(&p),
                BoardTransferDecision::Conditional(BoardTransferConcern::ConflictsWithVision)
            ),
            "PE owner should flag an ageing, poor-resale signing"
        );
    }

    #[test]
    fn state_backed_board_allows_elite_exception_despite_wage_breach() {
        let mut board = ClubBoard::new();
        board.ownership.ownership_type = OwnershipType::StateBacked;
        board.chairman.ambition = ChairmanAmbition::Reckless;
        // Elite signing (well above squad average) on a critical need, even
        // though wages blow past the headroom.
        let mut p = proposal(
            1_200_000.0,
            25,
            80,
            TransferNeedPriority::Critical,
            TransferNeedReason::QualityUpgrade,
        );
        p.economics = Some(BoardTransferEconomics {
            wage_impact_annual: 5_000_000.0,
            wage_budget_headroom: 0.0, // breach
            ..Default::default()
        });
        assert!(
            board.review_transfer_proposal(&p).is_approved(),
            "state-backed board should grant the elite exception"
        );
    }

    #[test]
    fn member_owned_board_values_homegrown_fit() {
        let mut board = ClubBoard::new();
        board.ownership.ownership_type = OwnershipType::MemberOwned;

        // A borderline-priced signing the import version can't quite justify.
        let make = |homegrown: bool| {
            let mut p = proposal(
                1_900_000.0,
                24,
                70,
                TransferNeedPriority::Optional,
                TransferNeedReason::QualityUpgrade,
            );
            p.economics = Some(BoardTransferEconomics {
                wage_budget_headroom: 50_000_000.0,
                resale_projection: p.fee,
                homegrown_fit: homegrown,
                ..Default::default()
            });
            p
        };

        assert!(
            board.review_transfer_proposal(&make(true)).is_approved(),
            "member-owned board should back the homegrown signing"
        );
        assert!(
            matches!(
                board.review_transfer_proposal(&make(false)),
                BoardTransferDecision::Vetoed(_)
            ),
            "the same deal for an import gets less rope and is vetoed"
        );
    }

    #[test]
    fn youth_board_accepts_weak_young_blocks_old_depth() {
        let mut board = ClubBoard::new();
        board.vision.preferred_squad_profile = SquadProfile::Youth;

        // Weaker-but-young development signing: welcomed.
        let young = proposal(
            300_000.0,
            19,
            50,
            TransferNeedPriority::Optional,
            TransferNeedReason::DevelopmentSigning,
        );
        assert!(
            board.review_transfer_proposal(&young).is_approved(),
            "youth board should accept a promising teenager"
        );

        // Ageing depth signing: blocked.
        let old = proposal(
            400_000.0,
            31,
            66,
            TransferNeedPriority::Important,
            TransferNeedReason::DepthCover,
        );
        assert!(matches!(
            board.review_transfer_proposal(&old),
            BoardTransferDecision::Vetoed(BoardTransferConcern::ConflictsWithVision)
        ));
    }

    #[test]
    fn manager_renewal_merited_after_sustained_high_trust() {
        let mut board = ClubBoard::new();
        board.season_targets = Some(targets(8, 12));
        let ctx = strong_ctx(2, 20); // overachieving
        for _ in 0..14 {
            let mut r = BoardResult::new();
            board.evaluate_performance(&ctx, &mut r);
        }
        assert!(
            board.relationship.merits_renewal(),
            "sustained overperformance should merit a renewal"
        );
        assert!(board.confidence.level >= 70);
    }

    #[test]
    fn poor_and_ffp_breach_apply_exactly_one_budget_cut() {
        // Regression for the double-apply bug: a Poor-mood month that is
        // also an FFP breach must cut the budget once (the FFP cut), never
        // a mood percentage *and* an FFP amount in the same tick.
        let mut board = ClubBoard::new();
        board.season_targets = Some(targets(5, 8));
        let mut ctx = poor_ctx(8, 38, 19, 20); // Early phase → no sacking
        ctx.ffp_status = FfpStatus::Breach;

        let mut last = BoardResult::new();
        for _ in 0..12 {
            last = BoardResult::new();
            board.evaluate_performance(&ctx, &mut last);
        }
        assert!(
            matches!(board.mood.state, BoardMoodState::Poor),
            "test setup should drive the board into Poor mood"
        );

        let cuts = last
            .decisions
            .iter()
            .filter(|d| matches!(d, BoardDecision::CutTransferBudget { .. }))
            .count();
        assert_eq!(
            cuts, 1,
            "exactly one cut in a Poor+breach month, got {:?}",
            last.decisions
        );
        // The single cut is the dominant FFP one — the mood cut is pre-empted.
        assert!(last.decisions.iter().any(|d| matches!(
            d,
            BoardDecision::CutTransferBudget {
                reason: DecisionReason::FfpPressure,
                ..
            }
        )));
        // No increase is emitted while breaching.
        assert!(
            !last
                .decisions
                .iter()
                .any(|d| matches!(d, BoardDecision::IncreaseTransferBudget { .. })),
            "a breaching board must not also boost the budget"
        );
    }

    #[test]
    fn excellent_and_injection_emit_a_single_increase() {
        // A strong run under a wealthy owner injects once — not a fixed
        // injection plus a separate excellent-mood percentage on top.
        let mut ctx = BoardContext::new();
        ctx.reputation_score = 0.9;
        ctx.balance = 50_000_000;
        ctx.country_economic_factor = 1.2;
        ctx.trailing_annual_income = 60_000_000;
        ctx.trailing_annual_outcome = 40_000_000;
        let mut board = ClubBoard::new();
        board.bootstrap_personality(&ctx, 0); // StateBacked → high injection appetite
        board.season_targets = Some(targets(2, 6));

        let strong = strong_ctx(2, 20);
        let mut last = BoardResult::new();
        for _ in 0..4 {
            last = BoardResult::new();
            board.evaluate_performance(&strong, &mut last);
        }

        let increases = last
            .decisions
            .iter()
            .filter(|d| matches!(d, BoardDecision::IncreaseTransferBudget { .. }))
            .count();
        assert_eq!(
            increases, 1,
            "exactly one budget increase, got {:?}",
            last.decisions
        );
        assert!(last.decisions.iter().any(|d| matches!(
            d,
            BoardDecision::IncreaseTransferBudget {
                reason: DecisionReason::OwnerInjection,
                ..
            }
        )));
    }

    #[test]
    fn takeover_roll_is_deterministic_and_in_range() {
        let date = chrono::NaiveDate::from_ymd_opt(2025, 8, 1).unwrap();
        let a = takeover_roll(42, date, 0);
        assert_eq!(a, takeover_roll(42, date, 0), "same inputs → same roll");
        assert!(a < 100, "roll must be a 0..99 percentage");
        // The salt axes actually move the output (so a rumour-start roll and
        // a resolution roll on the same day diverge).
        assert!(
            takeover_roll(43, date, 0) != a
                || takeover_roll(42, date, 1) != a
                || takeover_roll(42, chrono::NaiveDate::from_ymd_opt(2025, 9, 1).unwrap(), 0) != a,
            "roll should vary across club / date / status"
        );
    }

    #[test]
    fn takeover_decision_stream_replays_identically() {
        // Two runs with identical club id, dates and starting state must
        // produce the byte-for-byte same decision stream — no global RNG.
        fn run(club_id: u32) -> Vec<&'static str> {
            let mut board = ClubBoard::new();
            board.ownership = OwnershipModel {
                ownership_type: OwnershipType::PrivateEquity,
                wealth: 45,
                interference: 55,
                risk_tolerance: 65,
                exit_pressure: 80,
            };
            let mut ctx = BoardContext::new();
            ctx.balance = -80_000_000;
            ctx.profit_loss_12m = -20_000_000;
            ctx.ffp_status = FfpStatus::Breach;
            ctx.reputation_score = 0.5;
            board.season_targets = Some(targets(8, 12));

            let mut labels = Vec::new();
            let mut date = chrono::NaiveDate::from_ymd_opt(2025, 7, 1).unwrap();
            for _ in 0..24 {
                let mut r = BoardResult::new();
                r.club_id = club_id;
                board.tick_takeover(&ctx, date, &mut r);
                labels.extend(r.decisions.iter().map(|d| d.label()));
                date += chrono::Duration::days(30);
            }
            labels
        }
        assert_eq!(run(7), run(7), "identical state must replay identically");
        assert_eq!(run(99), run(99));
    }

    /// Build a small / poor member-owned board to take over.
    fn member_owned_board() -> ClubBoard {
        let mut board = ClubBoard::new();
        board.ownership = OwnershipModel {
            ownership_type: OwnershipType::MemberOwned,
            wealth: 25,
            interference: 20,
            risk_tolerance: 25,
            exit_pressure: 60,
        };
        board.vision.financial_stance = FinancialStance::Conservative;
        board.vision.long_term_goal = Some(LongTermGoal::Survive);
        board.relationship.trust_results = 10;
        board
    }

    #[test]
    fn takeover_always_installs_a_wealthier_owner_and_resets_relationship() {
        // Whatever the archetype, the buyer arrives richer and the manager
        // relationship resets to the fresh-appointment baseline.
        for seed in 0..3u32 {
            let mut board = member_owned_board();
            board.apply_takeover_completion(seed);
            assert!(board.ownership.wealth >= 70, "new owner should be wealthy");
            assert_eq!(board.confidence.level, 60);
            // Relationship was reset (trust_results back above the crisis level).
            assert!(board.relationship.trust_results >= 50);
        }
    }

    #[test]
    fn state_backed_takeover_chases_trophies_with_stars() {
        let mut board = member_owned_board();
        board.apply_takeover_completion(0); // seed % 3 == 0 → StateBacked
        assert!(matches!(
            board.ownership.ownership_type,
            OwnershipType::StateBacked
        ));
        assert!(matches!(
            board.vision.preferred_squad_profile,
            SquadProfile::Stars
        ));
        assert!(matches!(
            board.vision.financial_stance,
            FinancialStance::Ambitious
        ));
        assert_eq!(board.vision.long_term_goal, Some(LongTermGoal::WinLeague));
        assert!(matches!(
            board.chairman.ambition,
            ChairmanAmbition::Reckless
        ));
    }

    #[test]
    fn private_equity_takeover_prioritises_resale_and_wage_discipline() {
        let mut board = member_owned_board();
        board.apply_takeover_completion(1); // seed % 3 == 1 → PrivateEquity
        assert!(matches!(
            board.ownership.ownership_type,
            OwnershipType::PrivateEquity
        ));
        assert!(matches!(
            board.vision.preferred_squad_profile,
            SquadProfile::ResaleValue
        ));
        assert!(
            matches!(board.vision.financial_stance, FinancialStance::Conservative),
            "PE owners run a tight wage ship, not a Galáctico policy"
        );
        assert_eq!(
            board.vision.long_term_goal,
            Some(LongTermGoal::EstablishTopHalf)
        );
        assert!(matches!(
            board.vision.infrastructure_priority,
            InfrastructurePriority::Commercial
        ));
    }

    #[test]
    fn consortium_takeover_builds_a_balanced_prime_age_side() {
        let mut board = member_owned_board();
        board.apply_takeover_completion(2); // seed % 3 == 2 → Consortium
        assert!(matches!(
            board.ownership.ownership_type,
            OwnershipType::Consortium
        ));
        assert!(matches!(
            board.vision.preferred_squad_profile,
            SquadProfile::PrimeAge
        ));
        assert!(matches!(
            board.vision.financial_stance,
            FinancialStance::Balanced
        ));
        assert_eq!(
            board.vision.long_term_goal,
            Some(LongTermGoal::WinContinental)
        );
    }

    /// A wealthy board with a clear infrastructure mandate and money to burn.
    fn capex_board() -> (ClubBoard, BoardContext) {
        let mut board = ClubBoard::new();
        board.ownership = OwnershipModel {
            ownership_type: OwnershipType::StateBacked,
            wealth: 90,
            interference: 60,
            risk_tolerance: 80,
            exit_pressure: 5,
        };
        board.vision.infrastructure_priority = InfrastructurePriority::Training;
        let mut ctx = BoardContext::new();
        ctx.balance = 200_000_000;
        ctx.profit_loss_12m = 40_000_000;
        ctx.ffp_status = FfpStatus::Clean;
        (board, ctx)
    }

    #[test]
    fn facility_cooldown_blocks_consecutive_season_upgrades() {
        let (mut board, ctx) = capex_board();

        // Season 1: the upgrade is approved and starts the cooldown.
        let y1 = board.run_facility_review(&ctx, 2025);
        assert!(
            y1.iter()
                .any(|d| matches!(d, BoardDecision::ApproveFacilityUpgrade { .. })),
            "wealthy mandated board should approve in season 1: {y1:?}"
        );
        assert_eq!(board.last_facility_upgrade_year, Some(2025));

        // Season 2: still inside the cooldown → no upgrade at all.
        let y2 = board.run_facility_review(&ctx, 2026);
        assert!(
            y2.is_empty(),
            "cooldown must suppress the very next season: {y2:?}"
        );

        // Season 3: cooldown elapsed → upgrades allowed again.
        let y3 = board.run_facility_review(&ctx, 2027);
        assert!(
            y3.iter()
                .any(|d| matches!(d, BoardDecision::ApproveFacilityUpgrade { .. })),
            "after the cooldown the board may upgrade again: {y3:?}"
        );
    }

    #[test]
    fn ffp_breach_blocks_capex_and_keeps_cooldown_unused() {
        let (mut board, mut ctx) = capex_board();
        ctx.ffp_status = FfpStatus::Breach;

        let d = board.run_facility_review(&ctx, 2025);
        assert!(d.iter().any(|x| matches!(
            x,
            BoardDecision::RejectFacilityUpgrade {
                reason: DecisionReason::FfpPressure,
                ..
            }
        )));
        assert!(
            !d.iter()
                .any(|x| matches!(x, BoardDecision::ApproveFacilityUpgrade { .. })),
            "a breaching club can't fund capex"
        );
        // A rejection doesn't consume the cooldown — once compliant the club
        // is free to upgrade without waiting out a phantom cooldown.
        assert_eq!(board.last_facility_upgrade_year, None);
    }

    fn season_start() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2025, 7, 1).unwrap()
    }

    #[test]
    fn survival_promise_is_created_then_kept_and_builds_trust() {
        let mut board = ClubBoard::new();
        board.vision.long_term_goal = Some(LongTermGoal::Survive);
        board.season_targets = Some(targets(17, 20));
        let today = season_start();

        board.open_season_promises(&strong_ctx(8, 20), today, &[]);
        assert!(
            board.promises.has_active(PromiseType::Survival),
            "a survival-minded board should open a survival promise"
        );

        // Comfortably safe in the run-in → promise kept, trust rises.
        let before = board.relationship.trust_communication;
        let mut ctx = strong_ctx(8, 20);
        ctx.matches_played = 32;
        ctx.total_matches = 38; // RunIn
        ctx.distance_to_relegation = 6;
        let mut r = BoardResult::new();
        board.resolve_promises(&ctx, today + chrono::Duration::days(250), &mut r);

        assert!(
            !board.promises.has_active(PromiseType::Survival),
            "staying clear of the drop should keep the survival promise"
        );
        assert!(board.relationship.trust_communication >= before);
    }

    #[test]
    fn unkept_promise_breaks_and_costs_trust_at_season_end() {
        let mut board = ClubBoard::new();
        board.vision.long_term_goal = Some(LongTermGoal::Survive);
        board.season_targets = Some(targets(17, 20));
        let today = season_start();
        board.open_season_promises(&strong_ctx(8, 20), today, &[]);

        let before = board.relationship.trust_communication;
        // Mirror simulate's season-start reckoning a year later.
        let penalty = board
            .promises
            .break_overdue(today + chrono::Duration::days(366));
        assert!(penalty < 0, "an unmet survival promise must break");
        board.relationship.adjust_communication(penalty);
        assert!(board.relationship.trust_communication < before);
    }

    #[test]
    fn season_promises_do_not_duplicate_within_window() {
        let mut board = ClubBoard::new();
        board.vision.long_term_goal = Some(LongTermGoal::Survive);
        board.vision.youth_focus = VisionYouthFocus::DevelopYouth;
        board.season_targets = Some(targets(17, 20));
        let today = season_start();

        board.open_season_promises(&strong_ctx(8, 20), today, &[]);
        board.open_season_promises(&strong_ctx(8, 20), today, &[]);

        let survival = board
            .promises
            .active()
            .filter(|p| p.promise_type == PromiseType::Survival)
            .count();
        let youth = board
            .promises
            .active()
            .filter(|p| p.promise_type == PromiseType::YouthMinutes)
            .count();
        assert_eq!(survival, 1, "survival promise must not duplicate");
        assert_eq!(youth, 1, "youth promise must not duplicate");
    }

    #[test]
    fn deferred_capex_opens_a_facility_promise() {
        let mut board = ClubBoard::new();
        board.season_targets = Some(targets(10, 14));
        let today = season_start();
        let rejected = [BoardDecision::RejectFacilityUpgrade {
            facility: BoardFacility::Training,
            reason: DecisionReason::DebtTooHigh,
        }];
        board.open_season_promises(&strong_ctx(8, 20), today, &rejected);
        assert!(board.promises.has_active(PromiseType::FacilityImprovement));

        // A later approved upgrade keeps it.
        let mut r = BoardResult::new();
        r.decisions.push(BoardDecision::ApproveFacilityUpgrade {
            facility: BoardFacility::Training,
            cost: 5_000_000,
        });
        board.resolve_promises(
            &strong_ctx(8, 20),
            today + chrono::Duration::days(370),
            &mut r,
        );
        assert!(!board.promises.has_active(PromiseType::FacilityImprovement));
    }

    #[test]
    fn bootstrap_personality_is_deterministic_for_same_club() {
        // No global RNG in derivation: the same durable club signals must
        // always yield the same ownership archetype, so a re-derive (e.g.
        // after a hot-reload) never re-randomises the board.
        let mut ctx = BoardContext::new();
        ctx.reputation_score = 0.62;
        ctx.balance = 8_000_000;
        ctx.country_economic_factor = 1.1;

        let mut a = ClubBoard::new();
        a.bootstrap_personality(&ctx, 1234);
        let mut b = ClubBoard::new();
        b.bootstrap_personality(&ctx, 1234);

        assert_eq!(a.ownership.ownership_type, b.ownership.ownership_type);
        assert_eq!(a.ownership.wealth, b.ownership.wealth);
        assert_eq!(a.ownership.risk_tolerance, b.ownership.risk_tolerance);
        assert!(a.personality_initialized);
    }

    /// Assert every bounded board gauge is in range. Called every simulated
    /// month in the long-progression test.
    fn assert_board_invariants(board: &ClubBoard, result: &BoardResult) {
        assert!(
            (0..=100).contains(&board.confidence.level),
            "confidence out of range: {}",
            board.confidence.level
        );
        for g in [
            board.pressure.supporter_pressure,
            board.pressure.media_pressure,
            board.pressure.dressing_room_pressure,
            board.pressure.financial_pressure,
            board.pressure.regulatory_pressure,
        ] {
            assert!(g <= 100, "pressure gauge out of range: {g}");
        }
        for f in [
            board.relationship.trust_results,
            board.relationship.trust_finances,
            board.relationship.trust_squad_building,
            board.relationship.trust_communication,
            board.relationship.style_alignment,
        ] {
            assert!(f <= 100, "relationship facet out of range: {f}");
        }
        // Budget decisions only ever carry non-negative magnitudes, so
        // `process` can never drive the club's budget negative from them.
        for d in &result.decisions {
            match d {
                BoardDecision::CutTransferBudget { amount, .. }
                | BoardDecision::IncreaseTransferBudget { amount, .. } => {
                    assert!(*amount >= 0, "budget decision amount went negative: {d:?}");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn board_holds_all_invariants_over_three_sustained_poor_seasons() {
        // A relegation-bound mid-table club judged harshly for three
        // seasons. Confidence/pressure/relationship must stay in band, the
        // promise ledger must stay bounded, and the board must not sack the
        // manager in consecutive months (it resets and gives the caretaker
        // a run after each dismissal).
        let mut board = ClubBoard::new();
        board.vision.long_term_goal = Some(LongTermGoal::EstablishTopHalf);

        let total = 38u8;
        let mut today = season_start();
        let mut sack_months: Vec<u32> = Vec::new();
        let mut month_counter = 0u32;
        let mut max_active_promises = 0usize;

        for _season in 0..3 {
            // Season-start reckoning (mirrors `simulate`).
            let mut start_ctx = poor_ctx(0, total, 17, 20);
            start_ctx.league_size = 20;
            board.calculate_season_targets(&start_ctx);
            let penalty = board.promises.break_overdue(today);
            if penalty != 0 {
                board.relationship.adjust_communication(penalty);
            }
            board.promises.prune(today, 800);
            board.open_season_promises(&start_ctx, today, &[]);
            board.confidence.level = 65;
            board.poor_mood_months = 0;
            board.season_month_index = 0;

            for m in 0..11u8 {
                month_counter += 1;
                let matches_played = ((m as u16 * total as u16) / 10).min(total as u16) as u8;
                let mut ctx = poor_ctx(matches_played, total, 17, 20);
                ctx.league_size = 20;

                let mut r = BoardResult::new();
                board.evaluate_performance(&ctx, &mut r);
                board.resolve_promises(&ctx, today, &mut r);

                if r.manager_sacked {
                    sack_months.push(month_counter);
                }

                assert_board_invariants(&board, &r);
                max_active_promises = max_active_promises.max(board.promises.active_count());

                board.season_month_index += 1;
                today += chrono::Duration::days(30);
            }
            today += chrono::Duration::days(65); // skip to next season start
        }

        // Never sacked in back-to-back months: after a dismissal the board
        // resets and the caretaker gets at least a couple of months.
        for w in sack_months.windows(2) {
            assert!(
                w[1] - w[0] >= 2,
                "manager sacked in consecutive months: {sack_months:?}"
            );
        }
        // Ledger stays small across seasons (break + prune + dedupe).
        assert!(
            max_active_promises <= 6,
            "promise ledger grew unbounded: {max_active_promises}"
        );
    }

    #[test]
    fn takeover_rumour_always_resolves_within_its_window() {
        // A distressed, exit-pressured club: a rumour will open at some
        // point and must never sit Rumoured indefinitely — it resolves to
        // Completed or Failed within the simmer window.
        let mut board = ClubBoard::new();
        board.ownership = OwnershipModel {
            ownership_type: OwnershipType::PrivateEquity,
            wealth: 45,
            interference: 55,
            risk_tolerance: 65,
            exit_pressure: 80,
        };
        board.season_targets = Some(targets(10, 18));
        let mut ctx = BoardContext::new();
        ctx.balance = -80_000_000;
        ctx.profit_loss_12m = -20_000_000;
        ctx.ffp_status = FfpStatus::Breach;
        ctx.reputation_score = 0.5;

        let mut date = season_start();
        let mut rumoured_streak = 0u8;
        let mut ever_resolved = false;
        for _ in 0..36 {
            let mut r = BoardResult::new();
            r.club_id = 123;
            board.tick_takeover(&ctx, date, &mut r);
            match board.takeover.status {
                TakeoverStatus::Rumoured => {
                    rumoured_streak += 1;
                    assert!(
                        rumoured_streak <= 3,
                        "rumour stuck unresolved for {rumoured_streak} months"
                    );
                }
                TakeoverStatus::Completed | TakeoverStatus::Failed => {
                    rumoured_streak = 0;
                    ever_resolved = true;
                }
                TakeoverStatus::None => rumoured_streak = 0,
            }
            date += chrono::Duration::days(30);
        }
        assert!(
            ever_resolved,
            "an eligible distressed club should see a takeover resolve over 3 years"
        );
    }
}
