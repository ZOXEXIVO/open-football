use chrono::NaiveDate;
use log::debug;
use std::collections::{HashMap, HashSet};

use crate::club::board::ChairmanAmbition;
use crate::club::player::contract::{AffordabilityInput, ContractStalemate, StalemateLevel};
use crate::club::staff::perception::{AbilityEstimator, EstimationContext, PotentialEstimator};
use crate::club::team::squad::{MIN_FIRST_TEAM_SQUAD, SquadAssetClass, SquadAssetContext};
use crate::transfers::TransferWindowManager;
use crate::transfers::pipeline::asset_ledger::{
    AssetLedger, AssetRow, LedgerContext, SellListEntry,
};
use crate::transfers::pipeline::planning::{
    BriefTier, PlanInputs, PlanningCadence, RecruitmentBrief, SquadPlanner,
};
use crate::transfers::pipeline::processor::{PipelineProcessor, SquadPlayerInfo};
use crate::transfers::pipeline::trace::TransferTrace;
use crate::transfers::window::PlayerValuationCalculator;

use crate::club::player::adaptation::AdaptationSquadContext;
use crate::transfers::pipeline::loan_home::{SquadHomeContext, UnsettledAbroadScan};
use crate::transfers::pipeline::{
    ClubTransferPlan, LoanDestinationPreference, LoanOutCandidate, LoanOutReason, LoanOutStatus,
    TransferNeedPriority, TransferNeedReason, TransferRequest, TransferRequestStatus,
};
use crate::transfers::squad_needs::{EmergencyGroupSlot, FirstTeamSquadNeeds};
use crate::{
    Club, ClubPhilosophy, Country, LoanSpellVerdict, MatchTacticType, Person, Player,
    PlayerFieldPositionGroup, PlayerPlanRole, PlayerPositionType, PlayerStatusType,
    ReputationLevel, RoleFamiliarity, TACTICS_POSITIONS, TacticsSelector,
};

struct SquadEvaluation {
    club_id: u32,
    requests: Vec<TransferRequest>,
    loan_outs: Vec<LoanOutCandidate>,
    /// Player ids the deterministic position-glut detector wants
    /// hard-listed for transfer this tick. Applied in pass 2 with
    /// mutable Club access. Bypasses the AI listing path because the
    /// signal here is unambiguous: the squad has too many at this
    /// position and the player is too old to loan.
    force_transfer_list: Vec<u32>,
    total_budget: f64,
    max_concurrent: u32,
    /// What the club is shopping for and on what terms — see
    /// [`crate::transfers::pipeline::planning::SquadPlanner`]. The requests
    /// above are emitted from it; the brief itself is stored on the plan so
    /// the year-round watchlist and the shortlist builder can recruit
    /// against it.
    brief: Option<RecruitmentBrief>,
    /// Who the club would sell, and for how much — see
    /// [`crate::transfers::pipeline::asset_ledger::AssetLedger`]. Marketed,
    /// never listed.
    sell_list: Vec<SellListEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::transfers::pipeline) enum NeedKind {
    FormationGap,
    QualityUpgrade,
    DepthCover,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct GroupNeed {
    pub group: PlayerFieldPositionGroup,
    pub representative_pos: PlayerPositionType,
    pub kind: NeedKind,
}

/// Bench depth a group needs in addition to the formation slots —
/// pure function of the formation, used to detect "too thin to cover
/// rotations and injuries." Centralised so evaluation and tests share
/// one source of truth.
pub(in crate::transfers::pipeline) fn group_depth_requirement(
    formation_positions: &[PlayerPositionType; 11],
    group: PlayerFieldPositionGroup,
) -> usize {
    let formation_count = formation_positions
        .iter()
        .filter(|p| p.position_group() == group)
        .count();
    match group {
        PlayerFieldPositionGroup::Goalkeeper => 2,
        PlayerFieldPositionGroup::Defender => formation_count + 2,
        PlayerFieldPositionGroup::Midfielder => formation_count + 1,
        PlayerFieldPositionGroup::Forward => formation_count + 1,
    }
}

/// Detect the recruitment need for each position group, deduped at the
/// group level. Pure function — no side effects, no AI / network.
/// Priority order per group: FormationGap > QualityUpgrade > DepthCover.
/// Each group emits at most one entry, eliminating the slot-level
/// triple-count that distorted budget allocation in the old layout.
pub(in crate::transfers::pipeline) fn compute_group_needs(
    squad: &[SquadPlayerInfo],
    position_coverage: &[(PlayerPositionType, Option<u32>, u8)],
    formation_positions: &[PlayerPositionType; 11],
    rep_score: f32,
    quality_tolerance: i16,
) -> Vec<GroupNeed> {
    let mut needs: Vec<GroupNeed> = Vec::new();
    let mut visited: Vec<PlayerFieldPositionGroup> = Vec::new();

    for (pos, _, _) in position_coverage {
        let group = pos.position_group();
        if visited.contains(&group) {
            continue;
        }
        visited.push(group);

        let representative_pos = formation_positions
            .iter()
            .copied()
            .find(|p| p.position_group() == group)
            .unwrap_or(*pos);

        // (1) Formation gap — any slot in the group has no covering player.
        // Use the SPECIFIC uncovered slot as the representative position, not
        // the group's first formation slot: a side with four centre-backs but
        // no left-back has an empty LB slot, and the request should name the
        // left-back, not a centre-back. Downstream matching is group-based, so
        // this only sharpens the request (and any position-specific logic) —
        // it never narrows who can fill it.
        let gap_pos = position_coverage
            .iter()
            .find(|(p, pid, _)| p.position_group() == group && pid.is_none())
            .map(|(p, _, _)| *p);
        if let Some(gap_pos) = gap_pos {
            needs.push(GroupNeed {
                group,
                representative_pos: gap_pos,
                kind: NeedKind::FormationGap,
            });
            continue;
        }

        // (2) Quality upgrade — the WEAKEST SLOT in this group below tier
        // baseline. Ambition-adjusted tolerance: a club's appetite for an
        // upgrade scales with its standing. The base `quality_tolerance`
        // keeps small clubs patient, but a high-reputation side shops for an
        // upgrade when its best is merely AT the tier standard rather than
        // clearly below it — a title contender strengthens an adequate XI
        // instead of settling for the divisional baseline. Without this,
        // clubs only ever shopped to patch a hole, never to get better, so an
        // established starter at another club was almost never a target. The
        // budget split per need and the seller-side premium still bound how
        // many of these actually complete.
        //
        // The measure is per SLOT, not `max(CA)` over everyone wearing the
        // group's label. A max over the group is answered by the best man in
        // it, so a front line of three excellent wide forwards and no centre
        // forward read as fully stocked — the one shirt nobody could fill was
        // invisible behind the three that were. `position_coverage` has
        // already assigned each slot the best man still available, discounted
        // by how far from his own role he is standing, so the weakest covered
        // slot is the honest answer to "where is this side short?".
        let baseline = PipelineProcessor::tier_starter_ca_score(rep_score, group);
        let upgrade_tolerance = quality_tolerance - (rep_score * 5.0).round() as i16;
        // A long-term-injured player never held a slot in the first place
        // (`position_coverage` skips him), so an ageing incumbent out for
        // months can no longer mask a genuine need.
        let weakest_slot = position_coverage
            .iter()
            .filter(|(p, _, _)| p.position_group() == group)
            .min_by_key(|(_, _, quality)| *quality)
            .map(|(p, _, quality)| (*p, *quality));
        if let Some((slot_pos, _)) = weakest_slot
            .filter(|(_, quality)| (*quality as i16) < baseline as i16 - upgrade_tolerance)
        {
            needs.push(GroupNeed {
                group,
                // Name the shirt that is actually short. Recruitment matches on
                // role capability, so this sharpens the brief without narrowing
                // who can answer it.
                representative_pos: slot_pos,
                kind: NeedKind::QualityUpgrade,
            });
            continue;
        }

        // (3) Depth cover — group thinner than formation footprint. Long-term
        // injuries don't count toward depth: a club whose only cover is out
        // for months is genuinely short right now.
        let group_count = squad
            .iter()
            .filter(|p| {
                p.primary_position.position_group() == group
                    && !(p.is_injured && p.recovery_days > 30)
            })
            .count();
        if group_count < group_depth_requirement(formation_positions, group) {
            needs.push(GroupNeed {
                group,
                representative_pos,
                kind: NeedKind::DepthCover,
            });
        }
    }

    needs
}

/// How much a club wants to convert money into a better player it does not
/// strictly need.
///
/// Every other buying motive the evaluator raises is a form of patching:
/// `FormationGap` when a shirt is empty, `QualityUpgrade` when the weakest
/// slot is below the tier baseline, `DepthCover` when the group is thin.
/// A squad whose starters are all above the baseline therefore produced no
/// request at all — and with no request there is no scout assignment, no
/// shortlist, no board proposal and no bid. That is why clubs sitting on
/// billions never called for the best sub-elite players in the world: they
/// had nothing to patch.
///
/// Real elite recruitment also buys to raise the ceiling and to own assets,
/// and how hard it does so is continuous in four things a club already
/// carries: how big it is, how bold the chairman is, what the club is FOR,
/// and how much money is sitting idle behind the wage bill.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct InvestmentAppetite {
    /// 0..1. Zero means the club does not shop for players it does not need.
    pub score: f32,
}

impl InvestmentAppetite {
    /// Reputation below which discretionary investment is not a thing a club
    /// does — its money is spoken for by the squad it already has. Set just
    /// under the National/Continental line.
    const REPUTATION_FLOOR: f32 = 0.45;
    /// Reputation above the floor at which standing alone fully justifies
    /// shopping for improvement.
    const REPUTATION_SPAN: f32 = 0.45;
    /// Idle-cash ratio — `(balance − annual wages) ÷ annual income` — below
    /// which the club has no spare money at all. Slightly negative: a club
    /// whose cash is a little under its wage bill is normal, not distressed.
    const CASH_FLOOR: f32 = -0.30;
    /// Span from that floor to "sitting on a year of income beyond the wage
    /// bill", where the money term saturates.
    const CASH_SPAN: f32 = 1.0;

    /// Score the appetite for one club.
    pub(in crate::transfers::pipeline) fn of(club: &Club, rep_score: f32, date: NaiveDate) -> Self {
        let standing =
            ((rep_score - Self::REPUTATION_FLOOR) / Self::REPUTATION_SPAN).clamp(0.0, 1.0);
        if standing <= 0.0 {
            return InvestmentAppetite { score: 0.0 };
        }

        // No revenue evidence yet — a freshly generated world, or a club
        // whose first month has not closed. A club that has not earned
        // anything cannot know what it can afford to spend on a player it
        // does not need, and reading a missing denominator as "infinitely
        // cash-rich" would hand every club in the world full appetite for
        // its first month. Fail closed, exactly as the board's own
        // cold-start budget seed does.
        let annual_income = club.finance.estimated_annual_income(date);
        if annual_income <= 0 {
            return InvestmentAppetite { score: 0.0 };
        }
        let annual_wages: i64 = club
            .teams
            .iter()
            .map(|t| t.get_annual_salary() as i64)
            .sum();
        let idle_cash = club.finance.balance.balance - annual_wages;
        let cash_ratio = idle_cash as f32 / annual_income as f32;
        let money = ((cash_ratio - Self::CASH_FLOOR) / Self::CASH_SPAN).clamp(0.0, 1.0);

        // A bold chairman spends to be seen spending; a cautious one wants a
        // reason. Same ladder the board's transfer tolerance uses.
        let chairman = match club.board.chairman.ambition {
            ChairmanAmbition::Reckless => 1.30,
            ChairmanAmbition::Ambitious => 1.12,
            ChairmanAmbition::Balanced => 1.0,
            ChairmanAmbition::Conservative => 0.75,
        };
        // What the club is FOR. A side built to compete now converts money
        // into first-team quality; a develop-and-sell club buys assets too,
        // and a loan-focused one barely buys at all.
        let philosophy = match club.philosophy {
            ClubPhilosophy::SignToCompete => 1.20,
            ClubPhilosophy::DevelopAndSell => 1.05,
            ClubPhilosophy::Balanced => 1.0,
            ClubPhilosophy::LoanFocused => 0.55,
        };

        InvestmentAppetite {
            score: (standing * money * chairman * philosophy).clamp(0.0, 1.0),
        }
    }

    /// True when the club shops for improvement it does not need.
    pub(in crate::transfers::pipeline) fn is_active(&self) -> bool {
        self.score > 0.0
    }
}

/// One name the club's own watch is already carrying that would improve the
/// first team enough to be worth a request of its own.
pub(in crate::transfers::pipeline) struct InvestmentTarget {
    pub group: PlayerFieldPositionGroup,
    pub representative_pos: PlayerPositionType,
    /// The bar the request advertises: what the club's own top-`k` man in
    /// this group is worth. Anything below it is not an investment.
    pub min_ability: u8,
    /// What the recruitment department believes he would cost.
    pub estimated_fee: f64,
    /// How far the believed target sits ABOVE that bar — the size of the
    /// improvement, which is what decides which group a club spends its
    /// one discretionary signing on.
    pub improvement: i16,
}

/// The bridge from "our scouts are watching him" to "raise a request".
///
/// A `SquadInvestment` request is only honest when the club already has a
/// specific answer on its books — otherwise it is a blank brief that the
/// scouting pass would fill with whoever happens to be nearby, which is how
/// a discretionary budget turns into squad churn. So the watch reads the
/// club's OWN institutional memory (monitoring rows plus the known-player
/// record the year-round watch writes) and keeps only names that clear the
/// club's own top-`k` at their position and fit the asset window.
pub(in crate::transfers::pipeline) struct InvestmentWatch {
    targets: Vec<InvestmentTarget>,
}

impl InvestmentWatch {
    /// How stale a scouting memory may be and still count as a live lead.
    /// Roughly one transfer window plus the close season.
    const MEMORY_FRESH_DAYS: i64 = 180;
    /// Investment requests a club raises in one evaluation. More than a
    /// couple at once is a rebuild, not an investment.
    const MAX_TARGETS: usize = 2;

    pub(in crate::transfers::pipeline) fn build(
        club: &Club,
        squad: &[SquadPlayerInfo],
        rep_score: f32,
        date: NaiveDate,
    ) -> Self {
        // The same top-`k` ladder the discovery side files against: a giant
        // wants a top-2 man, a smaller club is improved by a wider band.
        let top_k = (2.0 + (1.0 - rep_score.clamp(0.0, 1.0)) * 5.0)
            .round()
            .max(1.0) as usize;
        let plan = &club.transfer_plan;

        let mut targets: Vec<InvestmentTarget> = Vec::new();
        // Names the department is ACTIVELY watching. The known-player
        // memory alone is not a lead — it holds up to 120 rows, most of
        // them opponents seen once in a match — so the monitoring board is
        // the filter, indexed once rather than rescanned per memory.
        let monitored: HashSet<u32> = plan.scout_monitoring.iter().map(|m| m.player_id).collect();
        for memory in plan.known_players.iter() {
            if (date - memory.last_seen).num_days() > Self::MEMORY_FRESH_DAYS {
                continue;
            }
            if !monitored.contains(&memory.player_id) {
                continue;
            }
            // The band the request itself will carry (see
            // `TransferRequest::new` for `SquadInvestment`). Filtering here
            // as well keeps the club from raising a request its own age
            // band would then reject every candidate for.
            if !(20..=27).contains(&memory.age) {
                continue;
            }
            // Would he be one of our best `k` in this group? Measured
            // against what the club knows about its own players and what
            // its scouts believe about him — never against hidden ability
            // on either side.
            let mut group_abilities: Vec<u8> = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == memory.position_group)
                .map(|p| p.current_ability)
                .collect();
            group_abilities.sort_unstable_by(|a, b| b.cmp(a));
            let kth = group_abilities.get(top_k.saturating_sub(1)).copied();
            let Some(bar) = kth else {
                // Fewer than `k` players in the group at all: that is a
                // depth problem the other needs already own.
                continue;
            };
            if memory.assessed_ability <= bar {
                continue;
            }
            if targets.iter().any(|t| t.group == memory.position_group) {
                continue;
            }
            targets.push(InvestmentTarget {
                group: memory.position_group,
                representative_pos: memory.position,
                min_ability: bar,
                improvement: memory.assessed_ability as i16 - bar as i16,
                estimated_fee: memory.estimated_fee,
            });
        }

        // Best first: the biggest jump over what the club already has. A
        // club with one discretionary signing in it should spend it where
        // it gains most, not where its own squad happens to be strongest.
        targets.sort_by(|a, b| b.improvement.cmp(&a.improvement));
        targets.truncate(Self::MAX_TARGETS);
        InvestmentWatch { targets }
    }

    pub(in crate::transfers::pipeline) fn targets(&self) -> &[InvestmentTarget] {
        &self.targets
    }
}

/// Succession-planning calculus for one aging incumbent. Pure helpers —
/// side-effect free so the heir logic can be unit-tested in isolation.
pub(in crate::transfers::pipeline) struct SuccessionAudit;

/// How close a first-choice player is to the end of his career, and so
/// how hard the club should be looking for his replacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::transfers::pipeline) enum SuccessionUrgency {
    /// Several years left — start grooming someone.
    Watch,
    /// The end is in sight; the heir should be at the club already.
    Pressing,
    /// He could stop playing any season now and there is nobody.
    Critical,
}

impl SuccessionAudit {
    /// Age by which a player of this group has stopped being first-choice
    /// at a serious level. Goalkeeper careers run years longer — which is
    /// exactly why a keeper's succession has to be planned from a horizon
    /// rather than assumed to arrive with the usual decline.
    pub(in crate::transfers::pipeline) fn career_end_age(group: PlayerFieldPositionGroup) -> u8 {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 38,
            PlayerFieldPositionGroup::Defender => 35,
            PlayerFieldPositionGroup::Midfielder => 34,
            PlayerFieldPositionGroup::Forward => 33,
        }
    }

    /// How much of a first-choice career the incumbent has left, and what
    /// the club should be doing about it. `None` means he is nowhere near
    /// the end and there is nothing to plan for yet.
    ///
    /// This replaces a bare "is he past a trigger age" test. The trigger
    /// age alone never explained what to do about a player who sailed
    /// past it and kept playing: a keeper still starting every week at 40
    /// read exactly like one who had just turned 33.
    pub(in crate::transfers::pipeline) fn urgency(
        incumbent: &SquadPlayerInfo,
    ) -> Option<SuccessionUrgency> {
        let group = incumbent.primary_position.position_group();
        let remaining = Self::career_end_age(group).saturating_sub(incumbent.age);
        match remaining {
            0..=1 => Some(SuccessionUrgency::Critical),
            2..=3 => Some(SuccessionUrgency::Pressing),
            4..=5 => Some(SuccessionUrgency::Watch),
            _ => None,
        }
    }

    /// True when the heir is already in the building.
    ///
    /// The bar is deliberately strict, because this test CANCELS the
    /// search: anything it waves through is what the club will rely on
    /// for the next decade. It used to accept any squad-mate four years
    /// younger and within twelve ability points, so two 29-year-old
    /// career deputies counted as the succession plan for a 40-year-old
    /// incumbent — and the club, believing itself covered, never looked
    /// again. A real heir is young enough to inherit a career rather than
    /// a season, and is assessed as actually reaching the incumbent's
    /// level. Reads the scouts' ASSESSED potential, never the hidden raw
    /// PA.
    pub(in crate::transfers::pipeline) fn heir_in_place(
        squad: &[SquadPlayerInfo],
        incumbent: &SquadPlayerInfo,
    ) -> bool {
        /// Oldest a credible heir can be. Past this he is a peer who will
        /// retire alongside the incumbent, not a successor.
        const HEIR_MAX_AGE: u8 = 26;
        /// Minimum age gap — the heir must have a career left when the
        /// incumbent's ends.
        const HEIR_MIN_AGE_GAP: u8 = 5;

        let group = incumbent.primary_position.position_group();
        squad.iter().any(|p| {
            p.player_id != incumbent.player_id
                && p.primary_position.position_group() == group
                && p.age <= HEIR_MAX_AGE
                && p.age + HEIR_MIN_AGE_GAP <= incumbent.age
                && !matches!(p.asset_class, SquadAssetClass::TrueSurplus)
                // Already within touching distance of the level, or
                // credibly assessed to grow into it.
                && (p.current_ability + 12 >= incumbent.current_ability
                    || p.estimated_potential + 4 >= incumbent.current_ability)
        })
    }
}

/// One realistic outcome of the career-pathway ladder for an underused
/// but development-relevant player. Pure data — [`ProspectPathway::decide`]
/// is side-effect free so the ladder can be unit-tested in isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::transfers::pipeline) enum PathwayAction {
    /// No transfer-pipeline action this tick — he's close enough to the
    /// XI that selection bias / internal minutes handle him, or he's too
    /// raw for a senior loan and is better served by reserve football.
    Hold,
    /// Loan out for first-team minutes, tagged with the reason that fits
    /// why he isn't playing.
    LoanOut(LoanOutReason),
    /// The development bet has failed (repeated loans that didn't take, or
    /// the runway has run out): list for permanent sale rather than loan
    /// into the void again.
    Sell,
}

/// Observable signals the pathway ladder reads for one player. Built from
/// squad context with no `Player` borrow so [`ProspectPathway::decide`]
/// stays pure and trivially testable.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct ProspectSignals {
    pub age: u8,
    pub current_ability: u8,
    pub estimated_potential: u8,
    pub potential_confidence: f32,
    pub group_avg_ability: u8,
    /// Official appearances THIS season (league + cups, no friendlies).
    pub official_appearances: u16,
    /// Official games in the most-recent completed season — distinguishes
    /// "stalled" from "was a regular until now".
    pub last_season_official_games: u16,
    /// Teammates at his position with strictly higher CA; 0 = top of the
    /// pecking order (the de-facto starter).
    pub depth_rank: usize,
    /// A clearly better player (>= CLEAR_GAP CA above) sits ahead at his
    /// position — hard evidence he's blocked, not just nominally behind.
    pub clearly_blocked: bool,
    /// His group carries more bodies than the formation minimum, so a loan
    /// won't drop the club below playable depth.
    pub has_loanable_depth: bool,
    /// Completed loan spells already behind him.
    pub previous_loans: u8,
    /// Of those, how many returned with negligible official minutes.
    pub failed_loans: u8,
    /// Most-recent completed seasons ending with ~zero official minutes,
    /// counted consecutively back from the latest.
    pub consecutive_zero_seasons: u8,
    /// Inside the January window — the mid-season "he still hasn't kicked
    /// a competitive ball" checkpoint.
    pub is_january: bool,
    /// Injured / suspended / on international duty / unregistered / unfit —
    /// any current state that means his lack of minutes isn't a free
    /// choice the club is making, so the pathway must not act on it.
    pub unavailable: bool,
    /// Months left on his contract; `None` for a free agent / no contract.
    /// Drives asset-value protection — a valuable prospect on a short deal
    /// must not be loaned into free-transfer risk.
    pub contract_months_remaining: Option<i32>,
    /// Contract-renewal talks are genuinely exhausted (the existing
    /// `ContractStalemate` says renewal is impossible). When true, a
    /// short-contract asset is sold now rather than loaned and lost free.
    pub renewal_exhausted: bool,
}

/// The career-pathway decision ladder. Given how a development-relevant
/// player is ACTUALLY being used, what would a realistic club do? The key
/// principle, and the fix for the "talented player rots with zero official
/// games" case: being BLOCKED and UNUSED is enough to act — the player
/// does NOT have to be far below his squad's average to be moved on.
pub(in crate::transfers::pipeline) struct ProspectPathway;

impl ProspectPathway {
    /// Oldest age still treated as "develop via a senior loan". Past this
    /// the stalled-asset case is sale-only (no endless loaning of a player
    /// who never broke through).
    pub const DEV_RELEVANT_MAX_AGE: u8 = 23;
    /// CA gap that marks a teammate as "clearly better" at the position.
    pub const CLEAR_GAP: u8 = 8;
    /// A returned loan with fewer official games than this counts as a
    /// failed pathway — the development minutes never materialised.
    pub const FAILED_LOAN_GAMES: u16 = 5;
    /// Season official-appearance ceiling that still reads as "no real
    /// senior pathway" — the 0-3 band the design brief calls out.
    pub const STALLED_SEASON_APPS: u16 = 3;
    /// Official appearances (this season OR last) at which the player is
    /// considered an established part of the rotation and is left alone.
    pub const REGULAR_APPS: u16 = 8;
    /// Believed-ceiling gap over current ability before we treat him as a
    /// high-potential asset worth a development loan.
    pub const CEILING_GAP: u8 = 5;
    /// Confidence floor on the coach's potential read before acting on it.
    pub const CONFIDENCE_FLOOR: f32 = 0.30;
    /// How far below his position-group average a player can sit and still
    /// be "good enough for some level of football" (worth developing
    /// rather than simply surplus).
    pub const WITHIN_REACH_GAP: i16 = 20;
    /// Minimum depth rank (players strictly ahead at the position) before a
    /// player counts as genuinely blocked. Rank 1 — narrowly behind a
    /// single teammate, e.g. a #2 keeper — is a credible rotation path, not
    /// a stalled asset, so it is deliberately NOT enough.
    pub const BLOCKED_MIN_RANK: usize = 2;
    /// Contract length (months) at or below which a valuable prospect is
    /// treated as "short deal" — loaning him out risks running the contract
    /// down and losing the asset for free, so renewal must come first.
    pub const SHORT_CONTRACT_MONTHS: i32 = 18;

    pub fn decide(s: &ProspectSignals) -> PathwayAction {
        // Can't fairly assess or move an unavailable player right now.
        if s.unavailable {
            return PathwayAction::Hold;
        }

        // A current regular — this season or last completed season — has a
        // pathway. Never loan/sell him via this sweep.
        let establishing = s.official_appearances >= Self::REGULAR_APPS
            || s.last_season_official_games >= Self::REGULAR_APPS;
        if establishing {
            return PathwayAction::Hold;
        }

        // ── Escalate to SALE — the development bet has failed ─────────
        // Two loans that didn't deliver minutes is enough on its own:
        // endlessly loaning a player who never actually plays isn't
        // realistic. (The `establishing` guard above already spared
        // anyone who has since become a regular.)
        if s.failed_loans >= 2 {
            return PathwayAction::Sell;
        }

        // Out of development runway with a track record of going nowhere:
        // two write-off seasons, or two completed loans and still nothing.
        let runway_gone = s.age >= Self::DEV_RELEVANT_MAX_AGE;
        if runway_gone && (s.consecutive_zero_seasons >= 2 || s.previous_loans >= 2) {
            return PathwayAction::Sell;
        }

        // A third loan spell isn't realistic. What happens instead depends
        // on whether the first two worked.
        //
        // Two spells that delivered minutes mean the pathway is doing its
        // job: he comes back and competes, and holding him is right. But a
        // player whose loans FAILED used to get the same Hold — he dropped
        // out of the only pathway he had and simply stayed, with nothing
        // else ever picking him up, because every other disposal route
        // reads him as a prospect. That is the same player the asset
        // classifier already calls surplus once his loan pathway is
        // exhausted; this is the decision that acts on it.
        if s.previous_loans >= 2 {
            return if s.failed_loans >= 1 {
                PathwayAction::Sell
            } else {
                PathwayAction::Hold
            };
        }

        // ── LOAN path — blocked, unused, worth developing, has runway ─
        // "Good enough for some football" = a believed high ceiling OR
        // already within reach of his group (not a no-hoper to offload).
        let believed_high_ceiling = s.estimated_potential
            > s.current_ability.saturating_add(Self::CEILING_GAP)
            && s.potential_confidence >= Self::CONFIDENCE_FLOOR;
        let within_reach =
            (s.current_ability as i16) >= s.group_avg_ability as i16 - Self::WITHIN_REACH_GAP;
        let worth_developing = believed_high_ceiling || within_reach;

        // Blocked = genuinely behind in the pecking order. A player only
        // narrowly behind ONE teammate (rank 1) still has a credible
        // rotation path — and a #2 keeper is squad depth, not surplus — so
        // require at least two ahead of him before treating him as stuck.
        //
        // Unless the seasons say otherwise. "A credible rotation path" is a
        // prediction, and after two years of no football it is one the
        // record has already refuted: a #2 keeper who has sat behind the
        // same man for two seasons is not rotating into anything. Left as a
        // rank test alone this was a permanent exemption — the one depth
        // position that could never qualify for the pathway at any age, for
        // any number of empty seasons.
        let stalled_behind_one =
            s.depth_rank >= 1 && s.consecutive_zero_seasons >= 2 && s.official_appearances == 0;
        let blocked = s.depth_rank >= Self::BLOCKED_MIN_RANK || stalled_behind_one;

        if !(worth_developing && blocked && s.has_loanable_depth) {
            return PathwayAction::Hold;
        }

        if s.age > Self::DEV_RELEVANT_MAX_AGE {
            return PathwayAction::Hold;
        }

        // Trigger windows for "needs a senior loan now":
        //   * January and still zero official minutes ("0 by January"),
        //   * a prior stalled season AND still nothing this season (the
        //     end-of-season / pre-season pathway decision), or
        //   * a clearly stalled current season (<= 3 official apps) at the
        //     January checkpoint.
        let zero_by_january = s.is_january && s.official_appearances == 0;
        let multi_season_stall = s.consecutive_zero_seasons >= 1 && s.official_appearances == 0;
        let stalled_in_january =
            s.is_january && s.official_appearances <= Self::STALLED_SEASON_APPS;

        if !(zero_by_january || multi_season_stall || stalled_in_january) {
            return PathwayAction::Hold;
        }

        // Asset-value protection: a VALUABLE prospect on a short contract
        // must not be silently loaned into free-transfer risk. Defer to the
        // existing contract-renewal pipeline first (it gets the chance to
        // extend before any move); only once renewal is genuinely exhausted
        // do we cash in — and we SELL rather than loan, so the resale value
        // isn't run down on an expiring deal out on loan.
        let short_contract = s
            .contract_months_remaining
            .is_some_and(|m| m <= Self::SHORT_CONTRACT_MONTHS);
        if believed_high_ceiling && short_contract {
            return if s.renewal_exhausted {
                PathwayAction::Sell
            } else {
                PathwayAction::Hold
            };
        }

        let reason = if s.clearly_blocked {
            LoanOutReason::BlockedByDepth
        } else if believed_high_ceiling && within_reach {
            // Genuinely valuable (high ceiling AND already near his group),
            // and on a safe-length deal — loan to protect and grow value.
            LoanOutReason::AssetValueProtection
        } else {
            LoanOutReason::NeedsFirstTeamMinutes
        };
        PathwayAction::LoanOut(reason)
    }
}

impl PipelineProcessor {
    /// Largest share of a club's available transfer budget one
    /// discretionary [`TransferNeedReason::SquadInvestment`] request may
    /// carry.
    ///
    /// A marquee signing IS most of a window's spending — that is what
    /// makes it marquee — so the old flat 15% share was itself a wall: the
    /// request could never carry the fee its own named target commanded.
    ///
    /// Defined AS the planner's transformative envelope rather than as a
    /// second number that happens to match it: a discretionary signing the
    /// scouts found and one the brief planned for are the same decision
    /// arriving by different doors, and they must not be able to drift
    /// apart. See [`BriefTier::A_SHARE`].
    pub(in crate::transfers::pipeline) const MAX_INVESTMENT_SHARE: f64 = BriefTier::A_SHARE;

    // ============================================================
    // Step 2: Squad Evaluation - Coach-driven, formation-based
    // ============================================================

    pub fn evaluate_squads(country: &mut Country, date: NaiveDate) {
        let is_window_start = Self::is_window_start_for(country, date);
        // Three cadences now reach this pass. The window ones are unchanged
        // (daily for the first week, then Mondays); the year-round planning
        // cadence is what makes a club that has no window open still write
        // down what it is trying to own — the whole point of L1. See
        // [`PlanningCadence`].
        let should_evaluate = is_window_start
            || Self::should_evaluate_for(country, date)
            || PlanningCadence::should_plan(country, date);
        let window_mgr = TransferWindowManager::for_country(country, date);
        let current_window = window_mgr.current_window_dates(country.id, date);
        // The mid-season checkpoint — "he still has no minutes, do
        // something about it" — has to follow the COUNTRY's short window.
        // Keyed on the raw calendar month it fired in January everywhere,
        // which for an MLS (Feb-Apr), Nordic (Feb-Apr), Latam (Jun-Jul) or
        // Asian (Jan 5-Feb 22) calendar is a date when the market is shut
        // and this sweep may not even run — so blocked prospects in those
        // countries got no review at all.
        let mid_season_window = Self::is_mid_season_window_for(country, date);

        // Pass 0 (mut): harvest the searches that died since the last meeting,
        // BEFORE the read pass builds this tick's requests — that ordering is
        // what lets a fresh request be raised in the voice of a club that has
        // tried and failed before.
        for club in &mut country.clubs {
            if club.transfer_plan.initialized {
                club.transfer_plan.harvest_failed_searches(date);
            }
        }

        // Pass 1: Collect evaluations (immutable reads)
        let mut evaluations: Vec<SquadEvaluation> = Vec::new();

        for club in &country.clubs {
            let needs_eval = should_evaluate
                || !club.transfer_plan.initialized
                || Self::all_shortlists_exhausted(&club.transfer_plan);

            if !needs_eval {
                continue;
            }

            let eval = Self::evaluate_single_club(
                club,
                date,
                current_window,
                mid_season_window,
                &SquadHomeContext {
                    country_id: country.id,
                    country_code: &country.code,
                    continent_id: country.continent_id,
                },
            );
            evaluations.push(eval);
        }

        if !evaluations.is_empty() {
            debug!("Transfer pipeline: evaluated {} clubs", evaluations.len());
        }

        // Pass 2: Apply evaluations (mutable writes)
        for eval in evaluations {
            if !eval.requests.is_empty() {
                debug!(
                    "Transfer pipeline: Club {} has {} requests, {} loan-outs, budget={:.0}",
                    eval.club_id,
                    eval.requests.len(),
                    eval.loan_outs.len(),
                    eval.total_budget
                );
            }

            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == eval.club_id) {
                let plan = &mut club.transfer_plan;

                // Only fully reset plans at window start or first initialization
                if !plan.initialized || is_window_start {
                    plan.reset_for_window();
                } else {
                    // On re-evaluation: remove completed assignments and exhausted shortlists
                    // so new requests can be scouted fresh
                    plan.scouting_assignments.retain(|a| !a.completed);
                    plan.shortlists.retain(|s| !s.all_exhausted());
                }

                plan.total_budget = eval.total_budget;
                plan.max_concurrent_negotiations = eval.max_concurrent;

                // The brief and the ledger REPLACE their predecessors rather
                // than accumulating: they are the club's current intent and
                // its current prices, and a stale slot is worse than none.
                // Both survive `reset_for_window` on purpose — a brief is
                // written six weeks ahead of the window and is exactly the
                // preparation the reset must not throw away.
                if let Some(brief) = eval.brief {
                    plan.brief = Some(brief);
                    plan.last_plan_date = Some(date);
                }
                plan.sell_list = eval.sell_list;
                plan.last_ledger_date = Some(date);

                // Track the highest request ID so re-evaluations don't create duplicate IDs
                if let Some(max_id) = eval.requests.iter().map(|r| r.id).max() {
                    plan.next_request_id = max_id + 1;
                }

                // Don't re-raise a need the plan is already working on.
                // Without this, each weekly re-evaluation piled another
                // "need GK" onto the list and every copy was acted on
                // independently, so clubs loaned in ten players for one
                // position.
                //
                // Matched on (group, reason) rather than group alone. A
                // club genuinely can need two different things from the
                // same group — cover for a hole in the XI is not the same
                // brief as a long-term upgrade or a youngster for the
                // future — and collapsing them meant one live request
                // silenced every other need at that position for the whole
                // window: a stalled centre-back pursuit stopped the club
                // ever asking about a full-back. The reason is what the
                // scouts and the loan market actually recruit against, so
                // it is the right unit of duplication.
                let new_requests: Vec<_> = eval
                    .requests
                    .into_iter()
                    .filter(|new_req| {
                        !plan.transfer_requests.iter().any(|existing| {
                            existing.position.position_group() == new_req.position.position_group()
                                && existing.reason == new_req.reason
                                && existing.status != TransferRequestStatus::Fulfilled
                                && existing.status != TransferRequestStatus::Abandoned
                        })
                    })
                    .collect();
                plan.transfer_requests.extend(new_requests);
                // Deduplicate loan-out candidates — don't re-add players already in the list
                for candidate in eval.loan_outs {
                    if !plan
                        .loan_out_candidates
                        .iter()
                        .any(|existing| existing.player_id == candidate.player_id)
                    {
                        plan.loan_out_candidates.push(candidate);
                    }
                }
                plan.last_evaluation_date = Some(date);
                plan.initialized = true;

                // Apply position-glut transfer-list decisions. These
                // write Lst directly. Cheap — usually empty; non-empty
                // only when a club has accumulated a positional surplus
                // (e.g. the Gzira 10-GK case).
                if !eval.force_transfer_list.is_empty() {
                    if let Some(main_team) = club.teams.main_mut() {
                        for player_id in eval.force_transfer_list {
                            if let Some(player) = main_team.players.find_mut(player_id) {
                                if !player.statuses.has(PlayerStatusType::Lst) {
                                    player.statuses.add(date, PlayerStatusType::Lst);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn all_shortlists_exhausted(plan: &ClubTransferPlan) -> bool {
        if plan.shortlists.is_empty() {
            return false;
        }
        plan.shortlists.iter().all(|s| s.all_exhausted())
    }

    /// Core squad evaluation: the head coach analyzes the squad based on their preferred
    /// formation and identifies tactical gaps.
    fn evaluate_single_club(
        club: &Club,
        date: NaiveDate,
        current_window: Option<(NaiveDate, NaiveDate)>,
        // True inside the country's shorter (mid-season) window — the
        // point in the calendar where a club reviews who still isn't
        // playing. Threaded from the caller because the club alone has no
        // country context to resolve its own transfer calendar.
        mid_season_window: bool,
        // Where this squad actually is. A club carries no country of its
        // own, and "is this player a foreigner here?" — the question the
        // unsettled-abroad loan reason turns on — cannot be answered
        // without one.
        home: &SquadHomeContext<'_>,
    ) -> SquadEvaluation {
        let mut requests = Vec::new();
        let mut loan_outs = Vec::new();
        let mut next_id = club.transfer_plan.next_request_id;

        // Calculate budget. Clubs in FFP breach have half their buying
        // power until the losses unwind — a soft equivalent of the
        // real-world transfer ban / spending cap.
        let raw_budget = club
            .finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount)
            .unwrap_or_else(|| (club.finance.balance.balance.max(0) as f64) * 0.3);
        let ffp_breach = club.finance.is_ffp_breach(date);
        let budget = if ffp_breach {
            raw_budget * 0.5
        } else {
            raw_budget
        };

        if club.teams.teams.is_empty() {
            return SquadEvaluation {
                club_id: club.id,
                requests,
                loan_outs,
                force_transfer_list: Vec::new(),
                total_budget: budget,
                max_concurrent: 1,
                brief: None,
                sell_list: Vec::new(),
            };
        }

        let team = &club.teams.teams[0];
        let players = &team.players.players;

        if players.is_empty() {
            // Empty main team: emit group-aware FormationGap requests
            // for every position group so the emergency free-agent pass
            // (in `country::result::transfers::free_agents`) has a
            // signal to react to, and the request-driven matcher has
            // open requests once the squad gets a few bodies. Without
            // these, the pipeline used to return zero requests for a
            // fresh / wiped club and would never bootstrap.
            let needs = FirstTeamSquadNeeds::for_club(club);
            for slot in needs.signing_plan() {
                let representative_pos = EmergencyGroupSlot::representative_position(slot.group);
                requests.push(TransferRequest::new(
                    next_id,
                    representative_pos,
                    TransferNeedPriority::Critical,
                    TransferNeedReason::FormationGap,
                    30, // floor: anything plausibly registerable
                    60, // ideal: journeyman quality
                    // Free-agent fee is zero so a 0 budget allocation
                    // is fine — the matcher uses wage affordability
                    // separately. Non-FA requests would need a real
                    // allocation, but an empty squad implies an
                    // emergency-first signing strategy.
                    0.0,
                ));
                next_id += 1;
            }
            return SquadEvaluation {
                club_id: club.id,
                requests,
                loan_outs,
                force_transfer_list: Vec::new(),
                total_budget: budget,
                max_concurrent: 1,
                brief: None,
                sell_list: Vec::new(),
            };
        }

        // Determine club reputation tier - this drives the entire transfer strategy
        let rep_level = team.reputation.level();

        // Determine max concurrent negotiations by reputation
        let base_max_concurrent = match rep_level {
            ReputationLevel::Elite => 6,
            ReputationLevel::Continental => 5,
            ReputationLevel::National => 3,
            ReputationLevel::Regional => 2,
            _ => 2,
        };
        // FFP breach forces discipline — cap at 1 open negotiation. A real
        // "transfer ban" in everything but name: budget already halved above,
        // and now the club can't spread what it has across multiple targets.
        let max_concurrent = if ffp_breach { 1 } else { base_max_concurrent };

        // Build squad info for analysis. `estimated_potential` is what
        // the **head coach** believes the player's ceiling is — built
        // from visible signals only via `PotentialEstimator`. Players
        // already on the main roster get full-visibility observations
        // (the coach sees them every training day); reserves/youth
        // would feed `is_main_team = false`, but the squad-eval pass
        // only iterates the first team here so all are main-team
        // visibility.
        let head_coach = team.staffs.head_coach();
        // Central squad-asset context, built once per club. Drives the
        // "is this player a protected first-team asset?" gate that every
        // loan-out / surplus sweep below consults — so a key / first-team /
        // inferred-core player (even one whose monthly squad status is still
        // `NotYetSet`, or who is merely short on early-season minutes) is
        // never loaned or listed automatically.
        let asset_ctx = SquadAssetContext::build(club, date);
        let squad: Vec<SquadPlayerInfo> = players
            .iter()
            .map(|p| {
                let all_pos = p.positions();
                let mut levels = HashMap::new();
                for pos in &all_pos {
                    levels.insert(*pos, p.positions.get_level(*pos));
                }
                let ctx = EstimationContext {
                    observation_count: 12,
                    is_main_team: true,
                    salt: 0xA1F0_07BA,
                };
                let estimate = PotentialEstimator::estimate_for_staff(p, head_coach, &ctx, date);
                SquadPlayerInfo {
                    player_id: p.id,
                    primary_position: p.position(),
                    current_ability: p.player_attributes.current_ability,
                    estimated_potential: estimate.estimated_potential,
                    potential_confidence: estimate.confidence,
                    age: p.age(date),
                    position_levels: levels,
                    appearances: p.statistics.played + p.statistics.played_subs,
                    // Official = league + all cups (domestic + continental);
                    // friendly_statistics is intentionally excluded.
                    official_appearances: p.statistics.played
                        + p.statistics.played_subs
                        + p.cup_statistics.played
                        + p.cup_statistics.played_subs,
                    is_injured: p.player_attributes.is_injured,
                    recovery_days: p.player_attributes.recovery_days_remaining,
                    injury_days: p.player_attributes.injury_days_remaining,
                    asset_class: asset_ctx.classify(p, date),
                    contract_months_remaining: p
                        .contract
                        .as_ref()
                        .map(|c| ((c.expiration - date).num_days() / 30) as i32),
                }
            })
            .collect();

        // For Elite/Continental clubs, use top-11 (starter) average to avoid
        // dragging the threshold down with weak youth/reserve players.
        // This prevents top clubs from pursuing mediocre transfer targets.
        let avg_ability: u8 = if !squad.is_empty() {
            let mut abilities: Vec<u8> = squad.iter().map(|p| p.current_ability).collect();
            abilities.sort_unstable_by(|a, b| b.cmp(a));

            let count = match rep_level {
                ReputationLevel::Elite | ReputationLevel::Continental => {
                    abilities.len().min(11) // top-11 starter average
                }
                ReputationLevel::National => {
                    abilities.len().min(16) // top-16 average
                }
                _ => abilities.len(), // full squad average for smaller clubs
            };

            let total: u32 = abilities[..count].iter().map(|&a| a as u32).sum();
            (total / count as u32) as u8
        } else {
            50
        };

        // ──────────────────────────────────────────────────────────
        // Philosophy-driven parameters
        // ──────────────────────────────────────────────────────────

        // Philosophy shapes transfer priorities: what age to target,
        // how much to spend on youth vs proven players, loan appetite.
        let philosophy = &club.philosophy;

        // Age preferences: DevelopAndSell targets young players,
        // SignToCompete targets prime-age proven performers
        let (_preferred_age_min, _preferred_age_max, youth_age_max) = match philosophy {
            ClubPhilosophy::DevelopAndSell => (17u8, 26, 21),
            ClubPhilosophy::SignToCompete => (23, 32, 19),
            ClubPhilosophy::LoanFocused => (19, 28, 22),
            ClubPhilosophy::Balanced => (19, 30, 21),
        };

        // Ability threshold adjustment: youth-focused clubs accept lower CA
        // because they invest in potential; compete-now clubs need immediate quality
        let ability_tolerance: i16 = match philosophy {
            ClubPhilosophy::DevelopAndSell => 25, // accept CA 25 below avg
            ClubPhilosophy::SignToCompete => 5,   // only near or above avg
            ClubPhilosophy::LoanFocused => 15,
            ClubPhilosophy::Balanced => 15,
        };

        // ──────────────────────────────────────────────────────────
        // STEP 1: Coach determines preferred formation
        // ──────────────────────────────────────────────────────────

        // Use the existing tactics if set, otherwise determine what the coach would pick
        let formation = team
            .tactics
            .as_ref()
            .map(|t| t.tactic_type)
            .unwrap_or_else(|| {
                // Determine from coach preference
                let coach = team.staffs.head_coach();
                let available: Vec<&Player> = players.iter().collect();
                if available.len() >= 11 {
                    TacticsSelector::select(team, coach).tactic_type
                } else {
                    MatchTacticType::T442
                }
            });

        // Get the 11 positions required by this formation
        let formation_positions = Self::get_formation_positions(formation);

        // ──────────────────────────────────────────────────────────
        // STEP 2: Map formation positions to squad - find gaps
        // ──────────────────────────────────────────────────────────

        // For each formation position, find the best available player
        // A position has "coverage" if at least one player can play there adequately
        let mut used_player_ids: Vec<u32> = Vec::new();
        let mut position_coverage: Vec<(PlayerPositionType, Option<u32>, u8)> = Vec::new(); // (pos, player_id, quality)

        for &formation_pos in formation_positions {
            // Find best available player for this position (not already assigned)
            let best = squad
                .iter()
                .filter(|p| !used_player_ids.contains(&p.player_id))
                // A long-term-injured player can't cover a slot — leave it
                // uncovered so it registers as a need (all tiers, not just
                // small clubs).
                .filter(|p| !(p.is_injured && p.recovery_days > 30))
                .filter_map(|p| {
                    // Can he play in this part of the pitch at all, and how
                    // good is he there?
                    //
                    // The quality is read off the best role he holds IN THE
                    // SLOT'S GROUP, not off the exact shirt. Which of a
                    // group's shirts a man wears is a shape decision, not a
                    // recruitment one: a centre-midfielder asked to fill in
                    // wide is still the midfielder the club has, and pricing
                    // him as out-of-position there would have every side in
                    // the world reading its own formation as a hole to be
                    // bought out of. What the familiarity DOES price is the
                    // player who only nominally covers the group — a winger
                    // who lists centre-forward at eight is not a
                    // centre-forward, and must not paper over a missing one.
                    let exact = p.position_levels.get(&formation_pos).copied().unwrap_or(0);
                    let group = formation_pos.position_group();
                    let in_group = p
                        .position_levels
                        .iter()
                        .filter(|(pos, _)| pos.position_group() == group)
                        .map(|(_, level)| *level)
                        .max()
                        .unwrap_or(0);
                    if in_group == 0 {
                        return None;
                    }
                    let effective = RoleFamiliarity::effective_ability(p.current_ability, in_group);
                    Some((p.player_id, exact, effective))
                })
                // Effective ability carries the selection; familiarity at the
                // exact shirt only breaks ties, so a natural gets it ahead of
                // an equally good group-mate filling in.
                .max_by_key(|&(_, exact, effective)| (effective, exact));

            match best {
                Some((pid, _level, quality)) => {
                    used_player_ids.push(pid);
                    position_coverage.push((formation_pos, Some(pid), quality));
                }
                None => {
                    position_coverage.push((formation_pos, None, 0));
                }
            }
        }

        // ──────────────────────────────────────────────────────────
        // STEP 3: Generate transfer requests from gaps
        // ──────────────────────────────────────────────────────────

        let available_budget = budget * 0.9; // Keep 10% reserve
        let mut budget_used = 0.0;

        // Continuous reputation score — drives tier baselines without
        // snapping to enum boundaries. A team mid-Continental gets a
        // different threshold from a team top-of-Continental.
        let rep_score = team.reputation.overall_score();
        let quality_tolerance = Self::tier_quality_tolerance_score(rep_score);
        let _ = ability_tolerance; // philosophy-driven tolerance retained
        // elsewhere; tier baselines drive the
        // recruitment thresholds now.

        // ── Build group-level needs in one pass ──────────────────────
        //
        // Each position group can produce AT MOST one need per evaluation,
        // chosen by priority FormationGap > QualityUpgrade > DepthCover.
        // The previous slot-level construction triple-counted groups in
        // `total_needs`, distorting `budget_per_need`: a back-three with
        // two empty slots looked like "two gaps" for budget purposes
        // even though both were filled by one signing in practice.
        //
        // Detection lives in `compute_group_needs` (pure function, unit
        // tested in helpers' test module).
        let group_needs = compute_group_needs(
            &squad,
            &position_coverage,
            formation_positions,
            rep_score,
            quality_tolerance,
        );

        let total_needs = group_needs.len();
        let budget_per_need = if total_needs > 0 {
            available_budget / total_needs as f64
        } else {
            0.0
        };
        // Sizing unit for forward-looking / discretionary buys (succession,
        // youth, experienced head, injury cover). It must NOT collapse to zero
        // when the XI has no formation gap (total_needs == 0 → budget_per_need
        // == 0): a well-built squad still plans succession and farms prospects.
        // When formation needs DO exist it equals `budget_per_need` exactly (no
        // behaviour change); with none it falls back to a slice of the overall
        // budget, widened when the head coach is dissatisfied with the squad
        // (the previously-inert squad-satisfaction signal now has a consumer).
        let squad_satisfaction = club
            .teams
            .head_coach_decision_state()
            .map(|state| state.squad_satisfaction)
            .unwrap_or(0.5);
        let discretionary_unit = if total_needs > 0 {
            budget_per_need
        } else {
            available_budget * (0.20 + (1.0 - squad_satisfaction as f64) * 0.20)
        };

        // ── The brief ────────────────────────────────────────────────
        //
        // The gap-driven needs above are one INPUT to the plan, not the plan
        // itself. They answer "what is missing?"; the planner answers "what
        // is this club trying to own, and what can it pay for it?" — so a
        // side whose XI is entirely above the divisional baseline, which
        // produced no request at all under the old loop, now briefs its
        // weakest starting shirt when it has both an objective and the money.
        //
        // Everything downstream is unchanged in shape: one request per
        // briefed shirt, funded from the same pot, escalated by the same
        // memory of failed searches. What the request now carries in
        // addition is the TIER (how transformative), the improvement bar,
        // and the shirt the club is promising.
        let brief = SquadPlanner::plan(&PlanInputs {
            club,
            squad: &squad,
            position_coverage: &position_coverage,
            formation_positions,
            rep_score,
            available_budget,
            group_needs: &group_needs,
            date,
            squad_size: squad.len(),
            max_squad_size: club
                .board
                .season_targets
                .as_ref()
                .map(|t| t.max_squad_size as usize)
                .unwrap_or(0),
        });

        for slot in brief.slots.iter() {
            // How many times the club has already come up short here. A need
            // raised for the first time escalates by nothing at all; one the
            // club has been carrying since last summer is put to the board
            // as Critical, on a wider brief and with the funding to match.
            let escalation = club
                .transfer_plan
                .escalation_for(slot.group, &slot.reason, date);
            let priority = escalation.priority(slot.priority.clone());
            // The envelope IS the funding decision — the planner already
            // sized it against the tier and what is left of the pot. A dry
            // pot means the club can't pay a FEE; it does not mean the club
            // has stopped needing players, so the request is still recorded
            // at zero allocation and the paths that cost nothing can serve
            // it: the free-agent matcher, the loan market (whose own
            // affordability runs off cash balance, not the transfer budget),
            // and Bosman pre-contracts. The paid negotiation path refuses
            // zero-allocation requests downstream, as before.
            let alloc = slot.envelope.min((available_budget - budget_used).max(0.0));
            requests.push(
                TransferRequest::new(
                    next_id,
                    slot.position,
                    priority,
                    slot.reason.clone(),
                    slot.min_ability(),
                    slot.ideal_ability(),
                    alloc.max(0.0),
                )
                .briefed(slot)
                .escalated(escalation),
            );
            next_id += 1;
            budget_used += alloc.max(0.0);
        }

        // ──────────────────────────────────────────────────────────
        // STEP 3b: Squad investment — buying to get better, not to patch
        // ──────────────────────────────────────────────────────────
        //
        // The needs above all answer "what is missing?". This one answers
        // "who would make us better, and can we afford him?" — and it only
        // asks once the club's own watch has an ANSWER on the books. A
        // request with nobody behind it is a scouting brief; a request
        // raised because the recruitment department is already watching a
        // man who would walk into the side is a decision.
        //
        // Nothing new happens after the request: it feeds the ordinary
        // scout → meeting → shortlist → board → negotiation chain, and the
        // board still has to approve the fee.
        let appetite = InvestmentAppetite::of(club, rep_score, date);
        if let Some(traced) = TransferTrace::target() {
            if club
                .transfer_plan
                .known_players
                .iter()
                .any(|m| m.player_id == traced)
            {
                TransferTrace::line(
                    traced,
                    "need",
                    format!(
                        "club={} ({}) rep={:.2} investment_appetite={:.2} \
                         available_budget={:.0} used={:.0} groups_needed={}",
                        club.name,
                        club.id,
                        rep_score,
                        appetite.score,
                        available_budget,
                        budget_used,
                        group_needs.len(),
                    ),
                );
            }
        }
        if appetite.is_active() && budget_used < available_budget {
            let watch = InvestmentWatch::build(club, &squad, rep_score, date);
            for target in watch.targets() {
                if budget_used >= available_budget {
                    break;
                }
                // One live investment request per group — the (group,
                // reason) dedup in `evaluate_squads` enforces this across
                // ticks, this stops a single tick raising two.
                if requests.iter().any(|r: &TransferRequest| {
                    r.position.position_group() == target.group
                        && r.reason == TransferNeedReason::SquadInvestment
                }) {
                    continue;
                }
                // Fund the actual player, not a flat share of the pot. A
                // marquee purchase sized at 15% of the budget was a wall
                // in its own right: the request could never carry the fee
                // its own named target commanded, so either the board
                // vetoed it on over-allocation or the shortlist admitted
                // nobody who cost what he was worth.
                let alloc = target
                    .estimated_fee
                    .min(available_budget * Self::MAX_INVESTMENT_SHARE)
                    .min(available_budget - budget_used);
                if alloc <= 0.0 {
                    continue;
                }
                requests.push(TransferRequest::new(
                    next_id,
                    target.representative_pos,
                    TransferNeedPriority::Optional,
                    TransferNeedReason::SquadInvestment,
                    target.min_ability,
                    target.min_ability.saturating_add(10),
                    alloc,
                ));
                next_id += 1;
                budget_used += alloc;
            }
        }

        // ──────────────────────────────────────────────────────────
        // STEP 4: Succession planning for aging key players
        // ──────────────────────────────────────────────────────────

        // Succession planning: proactive clubs replace aging stars before decline.
        // SignToCompete and DevelopAndSell clubs always plan; Balanced only at top tiers.
        let does_succession = matches!(
            philosophy,
            ClubPhilosophy::SignToCompete | ClubPhilosophy::DevelopAndSell
        ) || matches!(
            rep_level,
            ReputationLevel::Elite | ReputationLevel::Continental
        );
        if does_succession {
            for player_info in &squad {
                if budget_used >= available_budget {
                    break;
                }
                let group = player_info.primary_position.position_group();
                // How much first-choice career is left in him?
                let Some(urgency) = SuccessionAudit::urgency(player_info) else {
                    continue;
                };
                // Only genuine first-choice players need an heir lined
                // up — an aging backup is depth churn, not succession.
                //
                // The comparison is deliberately WITHIN the position
                // group. It used to be against the squad's top-eleven
                // ability average, which no goalkeeper passes: keepers
                // score below outfield players on the unified ability
                // scale by construction, so every club's number one was
                // silently skipped and keeper succession never once ran.
                let best_in_group = squad
                    .iter()
                    .filter(|p| p.primary_position.position_group() == group)
                    .map(|p| p.current_ability)
                    .max()
                    .unwrap_or(0);
                if player_info.current_ability + 4 < best_in_group {
                    continue;
                }
                // The heir may already be in the building — don't shop
                // for what the academy or an earlier window delivered.
                if SuccessionAudit::heir_in_place(&squad, player_info) {
                    continue;
                }

                let alloc = (discretionary_unit * 0.4).min(available_budget - budget_used);
                if alloc <= 0.0 {
                    break;
                }

                // Shop against the incumbent's own level, not the squad
                // average — the man we are replacing IS the standard, and
                // for a keeper the squad average is an outfield number.
                // The target is a prospect, so the floor sits well below
                // him: the pipeline judges these on assessed potential.
                let incumbent_level = player_info.current_ability;
                let (priority, min_ability) = match urgency {
                    SuccessionUrgency::Watch => (
                        TransferNeedPriority::Optional,
                        incumbent_level.saturating_sub(25),
                    ),
                    SuccessionUrgency::Pressing => (
                        TransferNeedPriority::Important,
                        incumbent_level.saturating_sub(18),
                    ),
                    // Nobody is coming through and he could stop at any
                    // time — the club needs someone who can actually take
                    // the shirt, not a project.
                    SuccessionUrgency::Critical => (
                        TransferNeedPriority::Critical,
                        incumbent_level.saturating_sub(10),
                    ),
                };

                // Only if we don't already have a request for this position
                if !requests
                    .iter()
                    .any(|r| r.position == player_info.primary_position)
                {
                    requests.push(TransferRequest::new(
                        next_id,
                        player_info.primary_position,
                        priority,
                        TransferNeedReason::SuccessionPlanning,
                        min_ability,
                        incumbent_level,
                        alloc,
                    ));
                    next_id += 1;
                    budget_used += alloc;
                }
            }
        }

        // ──────────────────────────────────────────────────────────
        // STEP 4a: Pre-departure replacement for expiring key players
        // ──────────────────────────────────────────────────────────
        // A first-choice player in the last months of his deal is a hole
        // opening next season even though he's full depth/quality right now.
        // If no heir is in the building and we're not already shopping his
        // position, line up a replacement — keyed on the contract clock, not
        // age, so a 26-year-old running his deal down is caught where the
        // age-gated succession pass misses him. Runs for every club, so a
        // Balanced lower-tier side no longer loses its best player for free
        // with nothing lined up.
        const REPLACE_CONTRACT_MONTHS: i32 = 9;
        for player_info in &squad {
            if budget_used >= available_budget {
                break;
            }
            let short_contract = player_info
                .contract_months_remaining
                .is_some_and(|m| m <= REPLACE_CONTRACT_MONTHS);
            if !short_contract {
                continue;
            }
            let group = player_info.primary_position.position_group();
            // Only a genuine first-choice (best or near-best in his group) is
            // worth pre-replacing — an expiring backup simply leaves.
            let best_in_group = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .map(|p| p.current_ability)
                .max()
                .unwrap_or(0);
            if player_info.current_ability + 4 < best_in_group {
                continue;
            }
            if SuccessionAudit::heir_in_place(&squad, player_info) {
                continue;
            }
            if requests
                .iter()
                .any(|r| r.position.position_group() == group)
            {
                continue;
            }
            let alloc = (discretionary_unit * 0.4).min(available_budget - budget_used);
            if alloc <= 0.0 {
                break;
            }
            let baseline = Self::tier_starter_ca_score(rep_score, group);
            requests.push(TransferRequest::new(
                next_id,
                player_info.primary_position,
                TransferNeedPriority::Important,
                TransferNeedReason::QualityUpgrade,
                baseline.saturating_sub(8),
                baseline.saturating_add(5),
                alloc,
            ));
            next_id += 1;
            budget_used += alloc;
        }

        // ──────────────────────────────────────────────────────────
        // STEP 4b: Youth development signings
        // Elite/Continental clubs actively seek young prospects with
        // high potential — even if current ability is well below squad level.
        // Like Juventus loaning a 19yo from Serie B who could become world class.
        // ──────────────────────────────────────────────────────────

        // Youth development signings: philosophy-driven.
        // DevelopAndSell clubs aggressively sign young prospects.
        // LoanFocused clubs borrow young players instead.
        let wants_youth = match philosophy {
            ClubPhilosophy::DevelopAndSell => true,
            ClubPhilosophy::Balanced => matches!(
                rep_level,
                ReputationLevel::Elite | ReputationLevel::Continental | ReputationLevel::National
            ),
            ClubPhilosophy::LoanFocused => false, // they borrow, not buy
            // Compete-now giants with money in the bank run a prospect-
            // ownership desk on the side (Chelsea / Man City model): buy
            // high-upside teenagers, farm them out on loan, promote or
            // sell later. Smaller SignToCompete clubs still buy only
            // ready-made players.
            ClubPhilosophy::SignToCompete => {
                matches!(
                    rep_level,
                    ReputationLevel::Elite | ReputationLevel::Continental
                ) && club.finance.balance.balance >= 0
            }
        };
        if wants_youth {
            let position_groups = [
                PlayerFieldPositionGroup::Defender,
                PlayerFieldPositionGroup::Midfielder,
                PlayerFieldPositionGroup::Forward,
            ];

            let max_youth_requests = match philosophy {
                ClubPhilosophy::DevelopAndSell => 4, // aggressive youth policy
                _ => match rep_level {
                    ReputationLevel::Elite => 3,
                    ReputationLevel::Continental => 2,
                    _ => 1,
                },
            };
            let mut youth_requests = 0u32;

            for group in &position_groups {
                if youth_requests >= max_youth_requests {
                    break;
                }

                // Count young players in this position group
                let young_in_group = squad
                    .iter()
                    .filter(|p| {
                        p.primary_position.position_group() == *group && p.age <= youth_age_max
                    })
                    .count();

                // DevelopAndSell wants more youth pipeline depth
                let min_young = if matches!(philosophy, ClubPhilosophy::DevelopAndSell) {
                    3
                } else {
                    2
                };
                if young_in_group < min_young {
                    let alloc = (discretionary_unit * 0.3).min(available_budget - budget_used);
                    if alloc <= 0.0 {
                        break;
                    }

                    let pos = match group {
                        PlayerFieldPositionGroup::Defender => PlayerPositionType::DefenderCenter,
                        PlayerFieldPositionGroup::Midfielder => {
                            PlayerPositionType::MidfielderCenter
                        }
                        PlayerFieldPositionGroup::Forward => PlayerPositionType::Striker,
                        _ => continue,
                    };

                    // Don't duplicate if we already have a request for this position group
                    if !requests
                        .iter()
                        .any(|r| r.position.position_group() == *group)
                    {
                        requests.push(TransferRequest::new(
                            next_id,
                            pos,
                            TransferNeedPriority::Optional,
                            TransferNeedReason::DevelopmentSigning,
                            // Low current ability floor — we care about potential, not now
                            avg_ability.saturating_sub(40),
                            avg_ability.saturating_sub(15),
                            alloc,
                        ));
                        next_id += 1;
                        budget_used += alloc;
                        youth_requests += 1;
                    }
                }
            }

            // Goalkeeper development signing — handled separately from the
            // outfield youth loop above. A club carries far fewer keepers and
            // treats keeper succession as its own project, so GK is NOT part
            // of the multi-slot outfield prospect budget (folding it in would
            // crowd out outfield prospects and shift that calibration).
            // Instead a youth-minded club with no young keeper in its
            // first-team picture grooms a single high-upside prospect; the
            // post-purchase pathway then typically farms him out on loan for
            // senior minutes (see `DevelopmentLoanPathway::stage_after_purchase`)
            // — the Chelsea / Man City model of buying a teenage keeper years
            // before he's needed. Keepers were previously omitted from the
            // prospect pipeline entirely, so big clubs never scouted young
            // goalkeepers at all. The count spans EVERY club team, not just
            // the first team: only one keeper plays, so a prospect keeper
            // bought last window and rebalanced into the U20 the same month
            // still IS the succession project. Counting the main roster only
            // created a closed re-buy loop (buy → weekly rebalance demotes
            // him → count back to zero → buy again next window) that
            // warehoused dozens of keepers in league-less youth squads.
            // Academy-stage keepers are naturally excluded — they live in
            // `club.academy`, not on any team roster. The band tops out at
            // the DevelopmentSigning request's own age ceiling so a groomed
            // 20-year-old still suppresses the next purchase.
            // Note this counter answers "do we run a keeper youth project
            // at all", which is a different question from "do we have an
            // heir for the man in possession of the shirt". The latter is
            // STEP 4's, and is deliberately NOT gated on this count: a
            // teenage keeper on the books satisfies the youth-project
            // question while doing nothing about a 38-year-old number one.
            const DEVELOPMENT_KEEPER_AGE_MAX: u8 = 21; // DevelopmentSigning band (16, 21)
            let keeper_prospect_age_max = youth_age_max.max(DEVELOPMENT_KEEPER_AGE_MAX);
            let young_keepers = club
                .teams
                .teams
                .iter()
                .flat_map(|t| t.players.players.iter())
                .filter(|p| {
                    !p.is_on_loan()
                        && p.position().position_group() == PlayerFieldPositionGroup::Goalkeeper
                        && p.age(date) <= keeper_prospect_age_max
                })
                .count();
            // Aggressive developers keep a small keeper pool on the books;
            // everyone else grooms one future #1 at a time.
            let want_young_keepers = if matches!(philosophy, ClubPhilosophy::DevelopAndSell) {
                2
            } else {
                1
            };
            // Skip if a GK request already exists (a QualityUpgrade or
            // SuccessionPlanning keeper need from the steps above) — one GK
            // request per window is enough.
            let already_requesting_gk = requests
                .iter()
                .any(|r| r.position.position_group() == PlayerFieldPositionGroup::Goalkeeper);
            if young_keepers < want_young_keepers && !already_requesting_gk {
                let alloc = (discretionary_unit * 0.3).min(available_budget - budget_used);
                if alloc > 0.0 {
                    requests.push(TransferRequest::new(
                        next_id,
                        PlayerPositionType::Goalkeeper,
                        TransferNeedPriority::Optional,
                        TransferNeedReason::DevelopmentSigning,
                        // Low current-ability floor — potential over now,
                        // same profile as the outfield prospect requests.
                        avg_ability.saturating_sub(40),
                        avg_ability.saturating_sub(15),
                        alloc,
                    ));
                    next_id += 1;
                    budget_used += alloc;
                }
            }
        }

        // ──────────────────────────────────────────────────────────
        // STEP 5: Small club squad padding & loan needs
        // ──────────────────────────────────────────────────────────

        let is_small = matches!(
            rep_level,
            ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
        );

        if is_small {
            // Squad below the published first-team minimum? Request
            // group-aware padding rather than a stack of generic
            // midfielders. Iterates the same signing-plan helper the
            // emergency free-agent pass uses, so the two paths can't
            // disagree about which group is actually missing bodies.
            if squad.len() < MIN_FIRST_TEAM_SQUAD {
                let needs = FirstTeamSquadNeeds::for_club(club);
                let mut emitted = 0u8;
                // Keep generated-padding-per-tick at 3 to match the
                // previous behaviour — the goal is gentle catch-up,
                // not an avalanche of low-quality signings every
                // weekly evaluation cycle.
                let max_pad_requests = 3u8;
                for slot in needs.signing_plan() {
                    if emitted >= max_pad_requests {
                        break;
                    }
                    // In-batch dedup: a Critical FormationGap emitted by
                    // step 3 THIS evaluation already covers the group —
                    // stacking an Optional padding request on top gave
                    // one hole two independent pursuit tracks. (The
                    // pass-2 filter only dedups new-vs-existing, never
                    // within the batch.)
                    if requests
                        .iter()
                        .any(|r| r.position.position_group() == slot.group)
                    {
                        continue;
                    }
                    // Padding allocates `budget_per_need * 0.3` so a
                    // small club still leaves headroom for upgrades.
                    // Free-agent fee is zero, but the same request
                    // can also be filled by a cheap paid signing —
                    // when budget is exhausted we still emit the
                    // request with zero allocation so the FA matcher
                    // and emergency pass can both react.
                    let alloc =
                        (discretionary_unit * 0.3).min((available_budget - budget_used).max(0.0));
                    let representative_pos =
                        EmergencyGroupSlot::representative_position(slot.group);
                    requests.push(TransferRequest::new(
                        next_id,
                        representative_pos,
                        TransferNeedPriority::Optional,
                        TransferNeedReason::SquadPadding,
                        avg_ability.saturating_sub(20),
                        avg_ability.saturating_sub(10),
                        alloc.max(0.0),
                    ));
                    next_id += 1;
                    if alloc > 0.0 {
                        budget_used += alloc;
                    }
                    emitted += 1;
                }
            }

            // No experienced players? Request one
            let has_experienced = squad
                .iter()
                .any(|p| p.age >= 28 && p.current_ability >= avg_ability);
            if !has_experienced && budget_used < available_budget {
                let alloc = (discretionary_unit * 0.3).min(available_budget - budget_used);
                if alloc > 0.0 {
                    requests.push(TransferRequest::new(
                        next_id,
                        PlayerPositionType::MidfielderCenter,
                        TransferNeedPriority::Optional,
                        TransferNeedReason::ExperiencedHead,
                        avg_ability.saturating_sub(5),
                        avg_ability,
                        alloc,
                    ));
                    next_id += 1;
                    budget_used += alloc;
                }
            }

            // Long-term injuries? Request cover
            let long_injured: Vec<_> = squad
                .iter()
                .filter(|p| p.is_injured && p.recovery_days > 30)
                .collect();
            for injured in long_injured.iter().take(2) {
                if budget_used >= available_budget {
                    break;
                }
                // In-batch dedup: the injury usually IS why step 3 already
                // emitted a gap/depth request for this group — a second
                // request would give the same hole two pursuit tracks.
                if requests.iter().any(|r| {
                    r.position.position_group() == injured.primary_position.position_group()
                }) {
                    continue;
                }
                let alloc = (discretionary_unit * 0.3).min(available_budget - budget_used);
                if alloc <= 0.0 {
                    break;
                }

                requests.push(TransferRequest::new(
                    next_id,
                    injured.primary_position,
                    TransferNeedPriority::Important,
                    TransferNeedReason::InjuryCoverLoan,
                    avg_ability.saturating_sub(15),
                    avg_ability.saturating_sub(5),
                    alloc,
                ));
                next_id += 1;
                budget_used += alloc;
            }
        }

        // ──────────────────────────────────────────────────────────
        // STEP 6: Identify loan-out candidates
        // ──────────────────────────────────────────────────────────

        Self::identify_loan_outs(
            &squad,
            &rep_level,
            avg_ability,
            date,
            players,
            &mut loan_outs,
            &club.philosophy,
            formation_positions,
            current_window,
            asset_ctx.is_early_season(),
            mid_season_window,
            home,
            rep_score,
        );

        // Position-glut sweep: catches surplus the loan-out branches
        // miss — most importantly the 30+ veterans the loan path
        // explicitly excludes. A club with 8 GKs needs to *eject* the
        // worst, not wait for a deficit signal that never fires when
        // the surplus itself is dragging the average down.
        let mut force_transfer_list =
            Self::identify_position_glut(&squad, date, players, &mut loan_outs);

        // Repeated-loan stagnation sweep: a player already farmed out
        // twice (the loan path refuses a third spell) who still sits
        // below his position-group level is no longer a development
        // asset — sell rather than hold.
        Self::identify_repeated_loan_stagnation(&squad, players, &mut force_transfer_list);

        // Stalled-prospect / blocked-asset pathway sweep. The branches
        // above all loan-list on a position-group-average DEFICIT, so a
        // talented youngster whose ability sits near (or above) his group
        // mean but who never plays falls through every one of them and can
        // rot for seasons. This sweep closes that gap: a development-
        // relevant player who is BLOCKED and UNUSED is loaned out for
        // minutes (or, when the development bet has already failed,
        // listed for sale) regardless of how he compares to the squad
        // average. Runs last and is purely additive — it skips anyone the
        // earlier, calibration-sensitive branches already planned for.
        Self::identify_stalled_prospects(
            &squad,
            date,
            players,
            formation_positions,
            current_window,
            &mut loan_outs,
            &mut force_transfer_list,
            mid_season_window,
        );

        // ── The asset ledger ─────────────────────────────────────────
        //
        // Priced last, because the sell list reads what the brief asked for:
        // a club whose plan wants more money than it has is a more willing
        // seller than the same club with its window already funded. Nothing
        // here lists anybody — an entry is a price and a readiness, and the
        // auto-listing sweeps keep every veto they had.
        let sell_list = Self::price_squad(club, players, &squad, &asset_ctx, &brief, date);

        SquadEvaluation {
            club_id: club.id,
            requests,
            loan_outs,
            force_transfer_list,
            total_budget: budget,
            max_concurrent,
            brief: Some(brief),
            sell_list,
        }
    }

    /// Value every contracted player the way the club would if somebody
    /// called about him, and decide who it would actually sell.
    ///
    /// Split out of the evaluation body because it is a self-contained
    /// seller-side pass: it reads the roster and the brief and writes
    /// nothing.
    fn price_squad(
        club: &Club,
        players: &[Player],
        squad: &[SquadPlayerInfo],
        asset_ctx: &SquadAssetContext,
        brief: &RecruitmentBrief,
        date: NaiveDate,
    ) -> Vec<SellListEntry> {
        let squad_average_level = asset_ctx.squad_avg_level();
        // Depth-chart rank per group, from the same observable levels the
        // asset classifier used, so "third choice" means one thing.
        let mut group_levels: HashMap<PlayerFieldPositionGroup, Vec<u8>> = HashMap::new();
        for player in players.iter().filter(|p| !p.is_on_loan()) {
            group_levels
                .entry(player.position().position_group())
                .or_default()
                .push(AbilityEstimator::observable_level(player));
        }
        for levels in group_levels.values_mut() {
            levels.sort_unstable_by(|a, b| b.cmp(a));
        }

        // A club prices its own players in its own market context — the
        // same blended league + club reputation the seller-advertised asking
        // price already uses, so the ledger and the listing agree.
        let (league_rep, club_rep) = PlayerValuationCalculator::seller_context_from_club(club);

        let annual_wages: f64 = club
            .teams
            .iter()
            .map(|t| t.get_annual_salary() as f64)
            .sum();
        let ctx = LedgerContext {
            philosophy: club.philosophy.clone(),
            annual_wages,
            wage_budget: club
                .finance
                .wage_budget
                .as_ref()
                .map(|w| w.amount)
                .unwrap_or(0.0),
            brief_envelope: brief.slots.iter().map(|s| s.envelope).sum(),
            available_budget: club
                .finance
                .transfer_budget
                .as_ref()
                .map(|b| b.amount)
                .unwrap_or(0.0),
            squad_size: squad.len(),
            max_squad_size: club
                .board
                .season_targets
                .as_ref()
                .map(|t| t.max_squad_size as usize)
                .unwrap_or(0),
            in_debt: club.finance.balance.balance < 0,
        };

        let rows: Vec<AssetRow> = players
            .iter()
            .filter(|p| !p.is_on_loan())
            .filter_map(|player| {
                let info = squad.iter().find(|s| s.player_id == player.id)?;
                let group = player.position().position_group();
                let observable_level = AbilityEstimator::observable_level(player);
                let rank = group_levels
                    .get(&group)
                    .map(|levels| {
                        levels
                            .iter()
                            .filter(|level| **level > observable_level)
                            .count()
                    })
                    .unwrap_or(0);
                Some(AssetRow {
                    player_id: player.id,
                    group,
                    age: info.age,
                    contract_months_remaining: info.contract_months_remaining,
                    estimated_value: PlayerValuationCalculator::calculate_value(
                        player, date, league_rep, club_rep,
                    )
                    .amount,
                    annual_wage: player
                        .contract
                        .as_ref()
                        .map(|c| c.salary as f64 * 12.0)
                        .unwrap_or(0.0),
                    asset_class: info.asset_class,
                    observable_level,
                    squad_average_level,
                    believed_ceiling: info.estimated_potential,
                    group_rank: rank.min(u8::MAX as usize) as u8,
                    is_transfer_requested: player.statuses.has(PlayerStatusType::Req),
                    stage_pull: player.big_stage_inclination,
                    renewal_blocked: Self::renewal_exhausted(player, date),
                    signing_protected: player.signing_protection_active(date),
                    // A player months from fitness cannot be sold now, and
                    // pricing him as though he could would have the club
                    // marketing an empty shirt.
                    unsellable: info.is_injured && info.recovery_days > 60,
                })
            })
            .collect();

        AssetLedger::build(&rows, &ctx, date)
    }

    /// Position-glut detector. Independent of age, deficit, or
    /// philosophy: triggers purely on having too many at one position.
    ///
    /// Per group, picks the bottom `count - keep_threshold` players
    /// (worst CA first, oldest tiebreak). Routes them by age:
    ///   * age >= 30 → **transfer-list** (returned for pass-2 apply,
    ///     since the loan path won't take them).
    ///   * age <  30 → **loan-out candidate** with `Surplus` reason.
    ///
    /// Catches the Gzira pattern: 10 GKs sitting on the main roster
    /// because the loan-out branches can't see them (deficit_vs_group
    /// underflows when surplus dominates the average), and the
    /// transfer-list AI hasn't made up its mind. After this fires,
    /// the worst 4-5 GKs are tagged for departure within one tick.
    fn identify_position_glut(
        squad: &[SquadPlayerInfo],
        date: NaiveDate,
        players: &[Player],
        loan_outs: &mut Vec<LoanOutCandidate>,
    ) -> Vec<u32> {
        // Per-position keep ceiling. Anything beyond this is glut.
        // Conservative — leaves headroom for tactical depth (e.g. 5
        // CBs is fine for a back-3 club; 6 starts to be silly).
        const KEEP_GK: usize = 4;
        const KEEP_DEF: usize = 10;
        const KEEP_MID: usize = 10;
        const KEEP_FWD: usize = 7;

        let keep_for = |group: PlayerFieldPositionGroup| -> usize {
            match group {
                PlayerFieldPositionGroup::Goalkeeper => KEEP_GK,
                PlayerFieldPositionGroup::Defender => KEEP_DEF,
                PlayerFieldPositionGroup::Midfielder => KEEP_MID,
                PlayerFieldPositionGroup::Forward => KEEP_FWD,
            }
        };

        let mut force_list: Vec<u32> = Vec::new();
        let groups = [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ];

        for group in groups {
            // Rank players in this group by CA ascending (worst first),
            // age descending tiebreak (older first — they're the
            // priority to clear because they can't be loaned).
            let mut ranked: Vec<&SquadPlayerInfo> = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .collect();
            ranked.sort_by(|a, b| {
                a.current_ability
                    .cmp(&b.current_ability)
                    .then(b.age.cmp(&a.age))
            });

            let count = ranked.len();
            let keep = keep_for(group);
            if count <= keep {
                continue;
            }
            let excess = count - keep;

            for surplus in ranked.into_iter().take(excess) {
                // Skip players who are already on loan or already
                // marked for transfer — no need to double-tag, and
                // re-listing churn would invalidate prior negotiations.
                let Some(player) = players.iter().find(|p| p.id == surplus.player_id) else {
                    continue;
                };
                if player.is_on_loan() {
                    continue;
                }
                // Manager-pinned players bypass the position-glut path:
                // even at a positional surplus they don't get listed or
                // loaned out — the pin is the whole point. The pin only
                // applies while the player is under contract; a free
                // agent (contract=None) is a leftover pin from the prior
                // contract and must not block the transfer pipeline.
                if player.is_force_match_selection && player.contract.is_some() {
                    continue;
                }
                // Never eject a protected first-team asset on a raw headcount
                // signal. The glut sweep already picks the worst-CA players,
                // so a core player rarely lands here — but a thin or skewed
                // early-season group can mis-rank, and a key player must not
                // be loaned or listed just because his position is crowded.
                if surplus.asset_class.is_first_team_protected() {
                    continue;
                }
                let already_listed = player.statuses.has(PlayerStatusType::Lst);

                if surplus.age >= 30 {
                    // Older surplus → transfer-list path (loan path
                    // explicitly excludes >= 30). Skip if already on
                    // the list to avoid stutter.
                    if !already_listed {
                        debug!(
                            "Position glut: forcing Lst on player {} (age {}, CA {}, group {:?})",
                            surplus.player_id, surplus.age, surplus.current_ability, group
                        );
                        force_list.push(surplus.player_id);
                    }
                } else {
                    // Younger surplus → loan-out path. Use the
                    // standard `Surplus` reason so the existing
                    // listing pipeline picks it up.
                    let already_listed_for_loan =
                        loan_outs.iter().any(|c| c.player_id == surplus.player_id);
                    if !already_listed_for_loan {
                        debug!(
                            "Position glut: adding loan-out candidate {} (age {}, CA {}, group {:?})",
                            surplus.player_id, surplus.age, surplus.current_ability, group
                        );
                        loan_outs.push(LoanOutCandidate {
                            player_id: surplus.player_id,
                            reason: LoanOutReason::Surplus,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                            preferred_destination: LoanDestinationPreference::Any,
                        });
                    }
                }
            }
        }

        let _ = date; // reserved for future age-window tweaks
        force_list
    }

    /// Loan-return re-evaluation, sell branch: players with two loan
    /// spells behind them (the loan path refuses a third) who are 23+
    /// and still clearly below their position-group average get
    /// transfer-listed. Repeatedly loaned and not progressing means the
    /// development bet failed — realistic clubs cash out. Under-23s
    /// keep their runway; 30+ players are the glut sweep's job.
    fn identify_repeated_loan_stagnation(
        squad: &[SquadPlayerInfo],
        players: &[Player],
        force_list: &mut Vec<u32>,
    ) {
        for info in squad {
            if !(23..30).contains(&info.age) {
                continue;
            }
            let Some(player) = players.iter().find(|p| p.id == info.player_id) else {
                continue;
            };
            if player.is_on_loan() {
                continue;
            }
            if player.is_force_match_selection && player.contract.is_some() {
                continue;
            }
            // A core / first-team asset is never sold off via the repeated-
            // loan-stagnation sweep, even if his raw loan history matches.
            if info.asset_class.is_first_team_protected() {
                continue;
            }
            let loan_spells = player
                .statistics_history
                .items
                .iter()
                .filter(|h| h.is_loan)
                .count();
            if loan_spells < 2 {
                continue;
            }
            let group = info.primary_position.position_group();
            let peers: Vec<u32> = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .map(|p| p.current_ability as u32)
                .collect();
            if peers.is_empty() {
                continue;
            }
            let group_avg = (peers.iter().sum::<u32>() / peers.len() as u32) as u8;
            if (info.current_ability as i16) < group_avg as i16 - 10
                && !player.statuses.has(PlayerStatusType::Lst)
                && !force_list.contains(&info.player_id)
            {
                debug!(
                    "Repeated-loan stagnation: listing player {} (age {}, CA {}, {} loan spells)",
                    info.player_id, info.age, info.current_ability, loan_spells
                );
                force_list.push(info.player_id);
            }
        }
    }

    /// Stalled-prospect / blocked-asset pathway sweep — the fix for the
    /// "talented player sits unused for seasons" case. For every
    /// development-relevant player the earlier branches left alone, build
    /// the observable [`ProspectSignals`] and apply the realistic
    /// [`ProspectPathway`] decision: loan him out for first-team minutes,
    /// or (development bet failed) list him for sale.
    ///
    /// Crucially, being BLOCKED and UNUSED is enough here — unlike the
    /// existing branches this does NOT require the player to be far below
    /// his position-group average. Purely additive: every existing guard
    /// (on-loan, manager-pinned, same-window, plan-protected, already
    /// listed / already planned for) is respected so nothing the
    /// calibration-sensitive branches decided is overridden.
    fn identify_stalled_prospects(
        squad: &[SquadPlayerInfo],
        date: NaiveDate,
        players: &[Player],
        formation_positions: &[PlayerPositionType; 11],
        current_window: Option<(NaiveDate, NaiveDate)>,
        loan_outs: &mut Vec<LoanOutCandidate>,
        force_list: &mut Vec<u32>,
        is_january: bool,
    ) {
        for info in squad {
            let Some(player) = players.iter().find(|p| p.id == info.player_id) else {
                continue;
            };

            // ── Guards (mirror the other loan-out paths) ─────────────
            if player.is_on_loan() {
                continue;
            }
            if player.is_force_match_selection && player.contract.is_some() {
                continue;
            }
            // Core / first-team protection: a key or inferred-first-team
            // player is never routed through the stalled-prospect pathway,
            // however thin his current minutes look.
            if info.asset_class.is_first_team_protected() {
                continue;
            }
            // International duty isn't a club decision to bench him — his
            // missing minutes must not read as a stalled pathway.
            if player.statuses.is_on_international_duty() {
                continue;
            }
            // Already handled by an earlier pass — don't double-tag or
            // contradict a plan (e.g. loan-listing a player marked to sell).
            if loan_outs.iter().any(|c| c.player_id == info.player_id)
                || force_list.contains(&info.player_id)
            {
                continue;
            }
            if player.statuses.has(PlayerStatusType::Lst)
                || player.statuses.has(PlayerStatusType::Loa)
            {
                continue;
            }
            // Same-window signing protection.
            if let (Some(transfer_date), Some((window_start, window_end))) =
                (player.last_transfer_date, current_window)
            {
                if transfer_date >= window_start && transfer_date <= window_end {
                    continue;
                }
            }
            // Club signing plan still under evaluation pins the player —
            // except a development plan, where loaning IS the plan.
            if let Some(ref plan) = player.plan {
                if !plan.is_evaluated(date, info.official_appearances)
                    && !plan.is_expired(date)
                    && plan.role != PlayerPlanRole::Development
                {
                    continue;
                }
            }

            // ── Position-group depth / blocking picture ──────────────
            let group = info.primary_position.position_group();
            let group_members: Vec<&SquadPlayerInfo> = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .collect();
            let group_count = group_members.len();
            let min_needed = Self::group_min_needed(group, formation_positions);

            let mut ahead = 0usize;
            let mut clearly_blocked = false;
            for other in &group_members {
                if other.player_id == info.player_id {
                    continue;
                }
                if other.current_ability > info.current_ability {
                    ahead += 1;
                }
                if other.current_ability
                    >= info
                        .current_ability
                        .saturating_add(ProspectPathway::CLEAR_GAP)
                {
                    clearly_blocked = true;
                }
            }
            let group_avg_ability: u8 = if group_count > 0 {
                (group_members
                    .iter()
                    .map(|p| p.current_ability as u32)
                    .sum::<u32>()
                    / group_count as u32) as u8
            } else {
                info.current_ability
            };

            let contract_months_remaining = player
                .contract
                .as_ref()
                .map(|c| ((c.expiration - date).num_days() / 30) as i32);
            // Only a SHORT-contract player's renewal state can change the
            // decision (asset-value protection), so skip the stalemate
            // assessment entirely for everyone else.
            let renewal_exhausted = contract_months_remaining
                .is_some_and(|m| m <= ProspectPathway::SHORT_CONTRACT_MONTHS)
                && Self::renewal_exhausted(player, date);

            let signals = ProspectSignals {
                age: info.age,
                current_ability: info.current_ability,
                estimated_potential: info.estimated_potential,
                potential_confidence: info.potential_confidence,
                group_avg_ability,
                official_appearances: info.official_appearances,
                last_season_official_games: Self::last_completed_season_official_games(player),
                depth_rank: ahead,
                clearly_blocked,
                has_loanable_depth: group_count > min_needed,
                previous_loans: Self::loan_spell_count(player),
                failed_loans: Self::failed_loan_count(player),
                consecutive_zero_seasons: Self::consecutive_zero_official_seasons(player),
                is_january,
                unavailable: Self::pathway_unavailable(player),
                contract_months_remaining,
                renewal_exhausted,
            };

            match ProspectPathway::decide(&signals) {
                PathwayAction::Hold => {}
                PathwayAction::LoanOut(reason) => {
                    debug!(
                        "Stalled prospect: loan-listing player {} (age {}, CA {}, official apps {}, reason {:?})",
                        info.player_id,
                        info.age,
                        info.current_ability,
                        info.official_appearances,
                        reason
                    );
                    loan_outs.push(LoanOutCandidate {
                        player_id: info.player_id,
                        reason,
                        status: LoanOutStatus::Identified,
                        loan_fee: 0.0,
                        preferred_destination: LoanDestinationPreference::Any,
                    });
                }
                PathwayAction::Sell => {
                    debug!(
                        "Stalled prospect: transfer-listing player {} (age {}, CA {}, prev loans {}, failed {})",
                        info.player_id,
                        info.age,
                        info.current_ability,
                        signals.previous_loans,
                        signals.failed_loans
                    );
                    force_list.push(info.player_id);
                }
            }
        }
    }

    /// Completed loan spells in the player's frozen history.
    fn loan_spell_count(player: &Player) -> u8 {
        player
            .statistics_history
            .items
            .iter()
            .filter(|h| h.is_loan)
            .count()
            .min(u8::MAX as usize) as u8
    }

    /// Loan spells that didn't deliver the development they were meant to.
    ///
    /// Read through [`LoanSpellVerdict`] — the same judgement the newsroom
    /// and the player's own mood use, so "the loan didn't work" means one
    /// thing across the game. A bare appearance count could not tell a
    /// season spent in the team apart from one spent on its bench: a
    /// player who came on twenty times as a substitute and never started,
    /// or who started and played badly, scored as a *successful* spell and
    /// so kept his place on the development pathway indefinitely. The
    /// verdict divides the spell length out and reads the football.
    ///
    /// Only judgeable spells count either way; a half-season cut short by
    /// injury is neither a success nor a failure.
    fn failed_loan_count(player: &Player) -> u8 {
        let ledger = &player.statistics_history.season_ledger;
        if !ledger.is_empty() {
            return ledger
                .iter()
                .filter(|e| e.is_loan && e.competition_kind.counts_toward_career_history())
                .filter(|e| {
                    let stats = &e.statistics;
                    let apps = stats.total_games();
                    let verdict = LoanSpellVerdict::classify(
                        stats.played,
                        apps,
                        stats.average_rating_realistic(player.position().position_group()),
                        // Unknown coverage means the row spans its season.
                        e.coverage_days.unwrap_or(300) as i64,
                    );
                    matches!(
                        verdict,
                        LoanSpellVerdict::Peripheral | LoanSpellVerdict::Struggled
                    )
                })
                .count()
                .min(u8::MAX as usize) as u8;
        }
        // Pre-ledger saves keep the original minutes-only read.
        player
            .statistics_history
            .items
            .iter()
            .filter(|h| {
                h.is_loan && h.statistics.total_games() < ProspectPathway::FAILED_LOAN_GAMES
            })
            .count()
            .min(u8::MAX as usize) as u8
    }

    /// Total official games in the player's most-recent completed season
    /// (loan and parent spells in that season summed). Zero when there's
    /// no frozen history yet.
    fn last_completed_season_official_games(player: &Player) -> u16 {
        let latest = player
            .statistics_history
            .items
            .iter()
            .map(|h| h.season.start_year)
            .max();
        match latest {
            Some(year) => player
                .statistics_history
                .items
                .iter()
                .filter(|h| h.season.start_year == year)
                .map(|h| h.statistics.total_games())
                .sum(),
            None => 0,
        }
    }

    /// Most-recent completed seasons that ended with zero official games,
    /// counted consecutively back from the latest. Loan and parent spells
    /// in the same season are summed, so a season counts as "zero" only if
    /// the player featured nowhere official that year.
    fn consecutive_zero_official_seasons(player: &Player) -> u8 {
        let mut by_season: Vec<(u16, u16)> = Vec::new();
        for item in &player.statistics_history.items {
            let year = item.season.start_year;
            let games = item.statistics.total_games();
            if let Some(entry) = by_season.iter_mut().find(|(y, _)| *y == year) {
                entry.1 = entry.1.saturating_add(games);
            } else {
                by_season.push((year, games));
            }
        }
        by_season.sort_by(|a, b| b.0.cmp(&a.0));
        let mut count = 0u8;
        for (_, games) in by_season {
            if games == 0 {
                count = count.saturating_add(1);
            } else {
                break;
            }
        }
        count
    }

    /// Whether the player is in a current state that means his lack of
    /// minutes is NOT a free squad decision — injured, recovering, banned,
    /// suspended, away on international duty, unregistered, or short of
    /// match fitness. The pathway must never loan or sell a player just
    /// because he had no minutes while unavailable.
    fn pathway_unavailable(player: &Player) -> bool {
        let attrs = &player.player_attributes;
        if attrs.is_injured || attrs.is_banned || attrs.is_in_recovery() {
            return true;
        }
        player.statuses.has(PlayerStatusType::Inj)
            || player.statuses.has(PlayerStatusType::Sus)
            || player.statuses.has(PlayerStatusType::Int)
            || player.statuses.has(PlayerStatusType::IntU21)
            || player.statuses.has(PlayerStatusType::Wp)
            || player.statuses.has(PlayerStatusType::Unr)
            || player.statuses.has(PlayerStatusType::Lmp)
            || player.statuses.has(PlayerStatusType::Unf)
    }

    /// True when contract-renewal talks for this player are genuinely
    /// exhausted, per the shared `ContractStalemate` assessment. Wage-budget
    /// headroom isn't known at this layer, so it's left unspecified — the
    /// assessment never treats "unknown" as "unaffordable", so this stays
    /// conservative and only fires on a real rejection track record.
    fn renewal_exhausted(player: &Player, date: NaiveDate) -> bool {
        let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
        let stalemate = ContractStalemate::assess(
            player,
            date,
            AffordabilityInput {
                wage_budget_headroom: None,
                current_salary,
            },
        );
        matches!(stalemate.level, StalemateLevel::Exhausted)
    }

    /// Get formation positions, falling back to T442 for unmapped formations.
    fn get_formation_positions(formation: MatchTacticType) -> &'static [PlayerPositionType; 11] {
        let (_, positions) = TACTICS_POSITIONS
            .iter()
            .find(|(tactic, _)| *tactic == formation)
            .unwrap_or(&TACTICS_POSITIONS[0]);
        positions
    }

    /// Minimum players a position group must retain on the main roster
    /// before any loan-out can fire — the formation footprint plus a thin
    /// rotation cushion. Shared by every loan-out path so they agree on
    /// "too thin to let anyone leave".
    ///
    /// Forward used to be the one group with no cushion, on the reasoning
    /// that "lone-striker shapes already carry the wide forwards counted
    /// here". They do not: AML / AMR group under `Midfielder`, so that
    /// carve-out was counting cover the group never had. In a 4-2-3-1 it put
    /// the floor at one, and a club with four centre-forwards could ship
    /// three of them out and go into a season with a single striker and no
    /// deputy. The cushion is now uniform, and matches
    /// [`group_depth_requirement`], which had always used `+1` here — the
    /// two had quietly disagreed about the same squad.
    pub(in crate::transfers::pipeline) fn group_min_needed(
        group: PlayerFieldPositionGroup,
        formation_positions: &[PlayerPositionType; 11],
    ) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => {
                formation_positions
                    .iter()
                    .filter(|p| p.is_defender())
                    .count()
                    + 1
            }
            PlayerFieldPositionGroup::Midfielder => {
                formation_positions
                    .iter()
                    .filter(|p| p.is_midfielder())
                    .count()
                    + 1
            }
            PlayerFieldPositionGroup::Forward => {
                formation_positions
                    .iter()
                    .filter(|p| p.is_forward())
                    .count()
                    + 1
            }
        }
    }

    /// Identify loan-out candidates based on club reputation tier.
    #[allow(clippy::too_many_arguments)]
    fn identify_loan_outs(
        squad: &[SquadPlayerInfo],
        rep_level: &ReputationLevel,
        avg_ability: u8,
        date: NaiveDate,
        players: &[Player],
        loan_outs: &mut Vec<LoanOutCandidate>,
        philosophy: &ClubPhilosophy,
        formation_positions: &[PlayerPositionType; 11],
        current_window: Option<(NaiveDate, NaiveDate)>,
        early_season: bool,
        is_january: bool,
        home: &SquadHomeContext<'_>,
        club_reputation_score: f32,
    ) {
        // Philosophy-based loan-out aggressiveness
        let (age_threshold, ability_gap, min_appearances_pct) = match philosophy {
            ClubPhilosophy::DevelopAndSell => (21, 5i16, 30u16), // Aggressively loan young players
            ClubPhilosophy::SignToCompete => (19, 10i16, 20u16), // Only loan clearly surplus
            ClubPhilosophy::LoanFocused => (23, 3i16, 40u16),    // Loan to reduce wages
            ClubPhilosophy::Balanced => (21, 8i16, 25u16),       // Standard
        };

        for player_info in squad {
            let player = match players.iter().find(|p| p.id == player_info.player_id) {
                Some(p) => p,
                None => continue,
            };

            // Skip players already on loan
            if player.is_on_loan() {
                continue;
            }

            // Manager-pinned: never propose a loan-out, regardless of
            // philosophy / playing-time / surplus signals. The pin is
            // the manager's decision; the AI must respect it. A free
            // agent (no contract) cannot be loaned anyway, but the pin
            // must not block any future move either.
            if player.is_force_match_selection && player.contract.is_some() {
                continue;
            }

            // Central core-player protection: a key / first-team / inferred-
            // core player is never loaned out automatically (the Litvinov
            // case — a KeyPlayer must not be farmed out for early-season
            // low minutes). RotationUseful / ProspectDevelopment / surplus
            // players fall through to the normal, calibration-sensitive
            // logic below.
            if player_info.asset_class.is_first_team_protected() {
                debug!(
                    "Loan-out skipped: player {} is a protected first-team asset ({})",
                    player_info.player_id,
                    player_info.asset_class.label()
                );
                continue;
            }

            // A player away on international duty isn't being benched by a
            // club choice — his low minutes are an artefact of the call-up,
            // not evidence he is unwanted. Never loan-list on that basis.
            if player.statuses.is_on_international_duty() {
                continue;
            }

            // Players aged 30+ should not be loaned — they should be sold or released.
            // Loaning older players is unrealistic in real football.
            if player_info.age >= 30 {
                continue;
            }

            // Players loaned out 2+ times should be sold, not loaned again.
            // Repeated loans from the same parent club are unrealistic.
            let previous_loan_count = player
                .statistics_history
                .items
                .iter()
                .filter(|h| h.is_loan)
                .count();
            if previous_loan_count >= 2 {
                continue;
            }

            // Players who are regular contributors (15+ appearances) should not
            // be loaned out — they're getting enough game time already.
            if player_info.appearances >= 15 {
                continue;
            }

            // Same-window protection: signed during this open window → can't be loaned out
            if let (Some(transfer_date), Some((window_start, window_end))) =
                (player.last_transfer_date, current_window)
            {
                if transfer_date >= window_start && transfer_date <= window_end {
                    continue;
                }
            }

            // Club has a signing plan for this player — don't loan them out
            // until they've been properly evaluated (enough time + appearances).
            // Development plans are the exception: loaning IS the plan.
            if let Some(ref plan) = player.plan {
                let total_apps = player_info.appearances;
                if !plan.is_evaluated(date, total_apps)
                    && !plan.is_expired(date)
                    && plan.role != PlayerPlanRole::Development
                {
                    continue;
                }
            }

            let group = player_info.primary_position.position_group();

            // Count players in same position group
            let group_count = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .count();

            // Minimum players needed per group from formation
            let min_needed = Self::group_min_needed(group, formation_positions);

            // Don't loan out if we'd drop below minimum
            if group_count <= min_needed {
                continue;
            }

            // Depth-chart position in the player's group. Used later as
            // a graduated resistance — the higher up the pecking order
            // a player sits, the harder it is for any loan-out trigger
            // to fire. No hard cut-off: an utterly surplus #1 can still
            // go, it just needs much stronger signals than the 4th-choice
            // would to get there.
            let mut group_ranks: Vec<(u32, u8)> = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .map(|p| (p.player_id, p.current_ability))
                .collect();
            group_ranks.sort_by(|a, b| b.1.cmp(&a.1));
            let rank = group_ranks
                .iter()
                .position(|(pid, _)| *pid == player_info.player_id)
                .unwrap_or(usize::MAX);
            // Position-group average CA — compares the player to their own
            // role peer group rather than the outfield-dominated starting
            // XI mean (which quietly branded first-choice keepers "below
            // average" and kept shipping them out).
            let group_avg: u8 = if !group_ranks.is_empty() {
                let sum: u32 = group_ranks.iter().map(|(_, ca)| *ca as u32).sum();
                (sum / group_ranks.len() as u32) as u8
            } else {
                avg_ability
            };
            // The best man in his position — what "blocked" is measured
            // against. A group average says nothing about whether the man
            // ahead of him is going to move.
            let group_best: u8 = group_ranks
                .first()
                .map(|(_, ca)| *ca)
                .unwrap_or(player_info.current_ability);
            // Depth cushion: extra CA-below-group-average the player needs
            // to exceed before any "surplus / lack of minutes" branch will
            // fire. Rank 0 (main) needs a massive deficit; rank 3+ needs
            // the normal amount. Scales smoothly; no hard cliff.
            let depth_cushion: i16 = match rank {
                0 => 25,
                1 => 12,
                2 => 5,
                _ => 0,
            };

            // ── The young foreigner who has not settled ─────────────
            //
            // Every branch below is about a CEILING: is he good enough to
            // grow, is he blocked by a better man, is he surplus. None of
            // them is about whether he has settled, so a Brazilian
            // twenty-one-year-old a season into the Premier League with
            // three starts and no language was invisible to all of them —
            // and the single most common loan in world football never
            // happened here.
            //
            // This one fires at EVERY reputation tier and bypasses the
            // potential / age-threshold gates, because it is not a
            // judgement about how good he is. It keeps every protection
            // that matters: the first-team asset guard (which ran above),
            // the same-window rule and the squad-minimum floor.
            //
            // What it must NOT keep is a depth-rank floor. The archetype
            // is the second-best man in his position who never starts —
            // a `rank >= 2` test excluded exactly him, at every tier, and
            // the cushion was applied in the LOOSENING direction where
            // every sibling branch tightens with it. The candidate test is
            // his START SHARE (inside `UnsettledAbroadScan`) plus a
            // ceiling read against the best man in his group, tightened by
            // how high up the chart he sits — the same shape as the
            // branches below.
            if player_info.age <= UnsettledAbroadScan::MAX_AGE
                && (player_info.current_ability as i16)
                    < group_best as i16 - depth_cushion
            {
                let adaptation = Some(
                    player.adaptation_score(
                        date,
                        home.country_code,
                        club_reputation_score,
                        Some(formation_positions),
                        &AdaptationSquadContext {
                            same_language_teammates: player
                                .squad_social_view
                                .as_ref()
                                .map(|v| v.same_language_teammates)
                                .unwrap_or(0),
                            same_nationality_teammates: player
                                .squad_social_view
                                .as_ref()
                                .map(|v| v.same_nationality_teammates)
                                .unwrap_or(0),
                            // Neutral, as the weekly pass reads it. The
                            // struct default is 0.0 — "the dressing room
                            // is at war" — a −7.5 adaptation bias that
                            // inflated the candidate list wherever it was
                            // unknown.
                            squad_chemistry: 50.0,
                            ..AdaptationSquadContext::default()
                        },
                    ),
                );
                if let Some(scan) = UnsettledAbroadScan::read(player, home, adaptation, date) {
                    if scan.is_candidate() {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::UnsettledAbroad,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                            preferred_destination: scan.preference,
                        });
                        continue;
                    }
                }
            }

            match rep_level {
                ReputationLevel::Elite | ReputationLevel::Continental => {
                    // Young players who need game time. Compare to the
                    // position-group average + depth cushion so the main
                    // at any position isn't routed to "dev minutes".
                    // Confidence gate: only act on a clear coach
                    // opinion (≥ 0.4). Borderline reads stay neutral —
                    // a low-judging coach shouldn't ship kids out on a
                    // hunch.
                    if player_info.age <= age_threshold
                        && player_info.estimated_potential > player_info.current_ability + 5
                        && player_info.potential_confidence >= 0.40
                        && (player_info.current_ability as i16)
                            < group_avg as i16 - ability_gap - depth_cushion
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                            preferred_destination: LoanDestinationPreference::Any,
                        });
                        continue;
                    }

                    // Players blocked by better players. Suppressed in the
                    // early-season low-evidence window: a handful of games
                    // into the season, low appearances are sample noise, not
                    // proof a player is blocked and needs to leave.
                    if !early_season
                        && player_info.age <= 25
                        && player_info.current_ability >= avg_ability.saturating_sub(10)
                        && player_info.appearances < min_appearances_pct
                    {
                        // Check if there's a clearly better player in same position
                        let better_exists = squad.iter().any(|other| {
                            other.player_id != player_info.player_id
                                && other.primary_position.position_group() == group
                                && other.current_ability > player_info.current_ability + 10
                        });

                        if better_exists {
                            loan_outs.push(LoanOutCandidate {
                                player_id: player_info.player_id,
                                reason: LoanOutReason::BlockedByBetterPlayer,
                                status: LoanOutStatus::Identified,
                                loan_fee: 0.0,
                                preferred_destination: LoanDestinationPreference::Any,
                            });
                            continue;
                        }
                    }

                    // Post-injury fitness
                    if player_info.age <= 25
                        && player_info.is_injured
                        && player_info.recovery_days <= 14
                        && player_info.injury_days > 60
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::PostInjuryFitness,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                            preferred_destination: LoanDestinationPreference::Any,
                        });
                        continue;
                    }

                    // Lack of playing time (January window)
                    if is_january
                        && player_info.age <= 26
                        && player_info.appearances < 5
                        && player_info.current_ability >= avg_ability.saturating_sub(15)
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::LackOfPlayingTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                            preferred_destination: LoanDestinationPreference::Any,
                        });
                        continue;
                    }
                }
                ReputationLevel::National => {
                    // Young players with high potential gap — group-relative
                    // deficit + depth cushion keeps the starter unscathed.
                    // National-tier staff have weaker judging eyes, so
                    // demand a wider believed gap (10) and reasonable
                    // confidence (≥ 0.35).
                    if player_info.age <= 22
                        && player_info.estimated_potential > player_info.current_ability + 10
                        && player_info.potential_confidence >= 0.35
                        && (player_info.current_ability as i16)
                            < group_avg as i16 - 5 - depth_cushion
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                            preferred_destination: LoanDestinationPreference::Any,
                        });
                        continue;
                    }

                    // Lack of playing time (January)
                    if is_january && player_info.age <= 24 && player_info.appearances < 3 {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::LackOfPlayingTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                            preferred_destination: LoanDestinationPreference::Any,
                        });
                        continue;
                    }
                }
                _ => {
                    // Regional/Local/Amateur: only loan very young players.
                    // Group-relative + depth cushion, same logic as above.
                    // Smaller-club staff are the weakest judges of
                    // potential — require the widest believed gap (15)
                    // and at least baseline confidence (≥ 0.30) before
                    // acting.
                    if player_info.age <= 21
                        && player_info.estimated_potential > player_info.current_ability + 15
                        && player_info.potential_confidence >= 0.30
                        && (player_info.current_ability as i16)
                            < group_avg as i16 - 10 - depth_cushion
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                            preferred_destination: LoanDestinationPreference::Any,
                        });
                        continue;
                    }
                }
            }

            // Surplus detection (all tiers)
            let surplus_threshold = if is_january {
                match group {
                    PlayerFieldPositionGroup::Goalkeeper => 3,
                    PlayerFieldPositionGroup::Defender => 5,
                    PlayerFieldPositionGroup::Midfielder => 5,
                    PlayerFieldPositionGroup::Forward => 3,
                }
            } else {
                match group {
                    PlayerFieldPositionGroup::Goalkeeper => 3,
                    PlayerFieldPositionGroup::Defender => 6,
                    PlayerFieldPositionGroup::Midfielder => 6,
                    PlayerFieldPositionGroup::Forward => 4,
                }
            };

            // Surplus fires on a position-group-relative deficit — a GK
            // sitting below the outfield-dominated squad mean is normal.
            // Depth cushion makes the first-choice extremely hard to flag.
            let deficit_vs_group = group_avg as i16 - player_info.current_ability as i16;
            if group_count >= surplus_threshold && deficit_vs_group >= 5 + depth_cushion {
                loan_outs.push(LoanOutCandidate {
                    player_id: player_info.player_id,
                    reason: LoanOutReason::Surplus,
                    status: LoanOutStatus::Identified,
                    loan_fee: 0.0,
                    preferred_destination: LoanDestinationPreference::Any,
                });
                continue;
            }

            // Financial relief (LoanFocused philosophy). The depth cushion
            // protects starters here too — you don't dump your first-choice
            // for wage relief.
            if *philosophy == ClubPhilosophy::LoanFocused
                && deficit_vs_group >= depth_cushion
                && player_info.appearances < 10
            {
                loan_outs.push(LoanOutCandidate {
                    player_id: player_info.player_id,
                    reason: LoanOutReason::FinancialRelief,
                    status: LoanOutStatus::Identified,
                    loan_fee: 0.0,
                    preferred_destination: LoanDestinationPreference::Any,
                });
            }
        }
    }
}

#[cfg(test)]
mod stalled_prospect_tests {
    use super::*;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::club::team::squad::SquadAssetClass;
    use crate::league::Season;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositions, PlayerSkills,
        PlayerStatistics, PlayerStatisticsHistoryItem,
    };
    use std::collections::HashMap;

    /// Fixtures for the stalled-prospect pathway sweep. Bundled on a unit
    /// struct per the project's no-free-helpers convention.
    struct Fx;

    impl Fx {
        fn date(y: i32, m: u32, d: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, d).unwrap()
        }

        /// The canonical "blocked, unused, high-potential January prospect"
        /// — the Lyan profile. Tests tweak one field at a time.
        fn blocked_prospect() -> ProspectSignals {
            ProspectSignals {
                age: 19,
                current_ability: 110,
                estimated_potential: 140,
                potential_confidence: 0.6,
                group_avg_ability: 118,
                official_appearances: 0,
                last_season_official_games: 0,
                depth_rank: 2,
                clearly_blocked: true,
                has_loanable_depth: true,
                previous_loans: 0,
                failed_loans: 0,
                consecutive_zero_seasons: 0,
                is_january: true,
                unavailable: false,
                contract_months_remaining: Some(36),
                renewal_exhausted: false,
            }
        }

        /// A player with `ca` at `pos`, born so they are ~`age` on the
        /// 2027 reference dates, with `league_apps` league appearances.
        fn player(id: u32, pos: PlayerPositionType, ca: u8, age: i32, league_apps: u16) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = ca.saturating_add(30);
            let mut stats = PlayerStatistics::default();
            stats.played = league_apps;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("P{id}")))
                .birth_date(Self::date(2027 - age, 1, 1))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: pos,
                        level: 16,
                    }],
                })
                .player_attributes(attrs)
                .statistics(stats)
                .build()
                .unwrap()
        }

        /// A frozen-history season row (parent or loan spell) with `games`
        /// official appearances.
        fn season_item(year: u16, is_loan: bool, games: u16) -> PlayerStatisticsHistoryItem {
            let mut stats = PlayerStatistics::default();
            stats.played = games;
            PlayerStatisticsHistoryItem {
                season: Season::new(year),
                team_name: "T".to_string(),
                team_slug: "t".to_string(),
                team_reputation: 0,
                league_name: "L".to_string(),
                league_slug: "l".to_string(),
                is_loan,
                transfer_fee: None,
                statistics: stats,
                seq_id: year as u32,
            }
        }

        fn info(
            player: &Player,
            date: NaiveDate,
            est_pot: u8,
            conf: f32,
            official_apps: u16,
        ) -> SquadPlayerInfo {
            SquadPlayerInfo {
                player_id: player.id,
                primary_position: player.position(),
                current_ability: player.player_attributes.current_ability,
                estimated_potential: est_pot,
                potential_confidence: conf,
                age: player.age(date),
                position_levels: HashMap::new(),
                appearances: official_apps,
                official_appearances: official_apps,
                is_injured: false,
                recovery_days: 0,
                injury_days: 0,
                // Neutral default — the sweeps under test only veto on
                // first-team protection, which `UnknownNeedsEvaluation`
                // does not trigger. Tests that exercise the veto set a
                // protected class explicitly.
                asset_class: SquadAssetClass::UnknownNeedsEvaluation,
                // Long runway by default — the tests here don't exercise the
                // expiring-contract replacement need.
                contract_months_remaining: Some(24),
            }
        }

        fn formation() -> &'static [PlayerPositionType; 11] {
            PipelineProcessor::get_formation_positions(MatchTacticType::T442)
        }

        fn group_min(group: PlayerFieldPositionGroup) -> usize {
            PipelineProcessor::group_min_needed(group, Self::formation())
        }
    }

    // ───────────────────────── decision-ladder unit tests ─────────────

    #[test]
    fn blocked_unused_january_prospect_is_loan_listed() {
        // The headline Lyan fix: blocked + unused is enough to act, even
        // though his ability sits NEAR (not far below) the group average.
        let s = Fx::blocked_prospect();
        assert_eq!(
            ProspectPathway::decide(&s),
            PathwayAction::LoanOut(LoanOutReason::BlockedByDepth)
        );
    }

    #[test]
    fn loan_never_thins_a_minimum_depth_group() {
        // Requirement: do not loan a player out if it leaves the position
        // below minimum depth.
        let mut s = Fx::blocked_prospect();
        s.has_loanable_depth = false;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Hold);
    }

    #[test]
    fn de_facto_starter_is_not_loaned_out() {
        // Top of the pecking order with nobody clearly ahead → not blocked.
        let mut s = Fx::blocked_prospect();
        s.depth_rank = 0;
        s.clearly_blocked = false;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Hold);
    }

    #[test]
    fn established_regular_is_left_alone() {
        let mut s = Fx::blocked_prospect();
        s.official_appearances = 20;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Hold);
        // Even if THIS season hasn't started, a regular last season holds.
        let mut s2 = Fx::blocked_prospect();
        s2.official_appearances = 0;
        s2.last_season_official_games = 25;
        assert_eq!(ProspectPathway::decide(&s2), PathwayAction::Hold);
    }

    #[test]
    fn injured_player_is_not_acted_on() {
        let mut s = Fx::blocked_prospect();
        s.unavailable = true;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Hold);
    }

    #[test]
    fn high_ceiling_asset_near_group_is_value_protected() {
        let mut s = Fx::blocked_prospect();
        s.clearly_blocked = false; // no clear-gap superior...
        s.depth_rank = 2; // ...but genuinely behind two teammates
        s.current_ability = 118;
        s.group_avg_ability = 120; // within reach
        s.estimated_potential = 150; // high believed ceiling
        // On a safe-length deal there's no value-loss risk in loaning him.
        s.contract_months_remaining = Some(36);
        assert_eq!(
            ProspectPathway::decide(&s),
            PathwayAction::LoanOut(LoanOutReason::AssetValueProtection)
        );
    }

    #[test]
    fn plain_blocked_prospect_needs_first_team_minutes() {
        let mut s = Fx::blocked_prospect();
        s.clearly_blocked = false;
        s.depth_rank = 2;
        s.estimated_potential = s.current_ability; // no believed ceiling
        assert_eq!(
            ProspectPathway::decide(&s),
            PathwayAction::LoanOut(LoanOutReason::NeedsFirstTeamMinutes)
        );
    }

    #[test]
    fn two_failed_loans_force_a_sale_not_another_loan() {
        // Requirement: 2 failed loans → transfer-list, not loaned again.
        let mut s = Fx::blocked_prospect();
        s.age = 24;
        s.failed_loans = 2;
        s.previous_loans = 2;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Sell);
    }

    #[test]
    fn two_failed_loans_sell_even_with_runway() {
        // "2 failed loans" is a sale trigger on its own, regardless of age.
        let mut s = Fx::blocked_prospect();
        s.age = 21;
        s.failed_loans = 2;
        s.previous_loans = 2;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Sell);
    }

    #[test]
    fn two_zero_seasons_over_runway_escalate_to_sale() {
        let mut s = Fx::blocked_prospect();
        s.age = 24;
        s.consecutive_zero_seasons = 2;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Sell);
    }

    #[test]
    fn two_zero_seasons_under_runway_escalate_to_loan() {
        // Requirement: 2 seasons of no official games escalate to loan OR
        // sale — under-runway players still get the loan pathway.
        let mut s = Fx::blocked_prospect();
        s.age = 21;
        s.consecutive_zero_seasons = 2;
        s.is_january = false; // driven purely by multi-season history
        assert!(matches!(
            ProspectPathway::decide(&s),
            PathwayAction::LoanOut(_)
        ));
    }

    #[test]
    fn no_third_loan_for_young_player_without_failure_record() {
        let mut s = Fx::blocked_prospect();
        s.age = 20;
        s.previous_loans = 2;
        s.failed_loans = 0; // both loans actually delivered minutes
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Hold);
    }

    #[test]
    fn game_time_reasons_demand_guaranteed_minutes() {
        // The borrower-side minutes gate runs at its strict bar for every
        // game-time-driven loan, so a blocked prospect can't be loaned to
        // a club where he'd sit behind a wall of better players.
        assert!(LoanOutReason::BlockedByDepth.expects_guaranteed_minutes());
        assert!(LoanOutReason::NeedsFirstTeamMinutes.expects_guaranteed_minutes());
        assert!(LoanOutReason::AssetValueProtection.expects_guaranteed_minutes());
        assert!(LoanOutReason::DevelopmentPathway.expects_guaranteed_minutes());
        assert!(LoanOutReason::NeedsGameTime.expects_guaranteed_minutes());
        // Pure surplus / financial loans keep the looser cover bar.
        assert!(!LoanOutReason::Surplus.expects_guaranteed_minutes());
        assert!(!LoanOutReason::FinancialRelief.expects_guaranteed_minutes());
    }

    // ───────────────────────── integration (full sweep) tests ─────────

    #[test]
    fn sweep_loan_lists_blocked_unused_january_prospect() {
        let date = Fx::date(2027, 1, 15);
        let need = Fx::group_min(PlayerFieldPositionGroup::Forward);

        // Prospect (id 1) plus `need + 1` clearly-better, established
        // forwards → group depth comfortably above the minimum.
        let mut players = vec![Fx::player(1, PlayerPositionType::Striker, 110, 19, 0)];
        let mut squad = vec![Fx::info(&players[0], date, 145, 0.6, 0)];
        for i in 0..=need as u32 {
            let p = Fx::player(100 + i, PlayerPositionType::Striker, 130, 27, 25);
            squad.push(Fx::info(&p, date, 130, 0.5, 25));
            players.push(p);
        }

        let mut loan_outs = Vec::new();
        let mut force_list = Vec::new();
        PipelineProcessor::identify_stalled_prospects(
            &squad,
            date,
            &players,
            Fx::formation(),
            None,
            &mut loan_outs,
            &mut force_list,
            PipelineProcessor::is_january_window(date),
        );

        let listed = loan_outs.iter().find(|c| c.player_id == 1);
        assert!(
            listed.is_some(),
            "blocked, unused January prospect must be loan-listed"
        );
        assert_eq!(listed.unwrap().reason, LoanOutReason::BlockedByDepth);
        assert!(
            !force_list.contains(&1),
            "an under-23 prospect with runway is loaned, not sold"
        );
    }

    #[test]
    fn sweep_respects_minimum_position_depth() {
        // A genuinely blocked prospect (several clearly-better players
        // ahead) at a position sitting at EXACTLY its minimum depth must
        // still be held — loaning him would leave the group too thin.
        // Midfield's formation minimum is high enough that the prospect can
        // be buried while the group is at the minimum count.
        let date = Fx::date(2027, 1, 15);
        let need = Fx::group_min(PlayerFieldPositionGroup::Midfielder);

        let mut players = vec![Fx::player(
            1,
            PlayerPositionType::MidfielderCenter,
            110,
            19,
            0,
        )];
        let mut squad = vec![Fx::info(&players[0], date, 145, 0.6, 0)];
        for i in 1..need as u32 {
            let p = Fx::player(100 + i, PlayerPositionType::MidfielderCenter, 130, 27, 25);
            squad.push(Fx::info(&p, date, 130, 0.5, 25));
            players.push(p);
        }
        assert_eq!(squad.len(), need, "group must sit at exactly the minimum");

        let mut loan_outs = Vec::new();
        let mut force_list = Vec::new();
        PipelineProcessor::identify_stalled_prospects(
            &squad,
            date,
            &players,
            Fx::formation(),
            None,
            &mut loan_outs,
            &mut force_list,
            PipelineProcessor::is_january_window(date),
        );

        assert!(
            !loan_outs.iter().any(|c| c.player_id == 1),
            "must not loan out a blocked player when it leaves the group below minimum depth"
        );
    }

    #[test]
    fn sweep_sells_player_with_two_failed_loans() {
        // Requirement: 2 failed loans + age 23+ → transfer-listed, not
        // loaned again. History reads come from the frozen ledger items.
        let date = Fx::date(2027, 1, 15);
        let mut prospect = Fx::player(1, PlayerPositionType::Striker, 110, 24, 0);
        prospect
            .statistics_history
            .items
            .push(Fx::season_item(2024, true, 2)); // failed loan
        prospect
            .statistics_history
            .items
            .push(Fx::season_item(2025, true, 3)); // failed loan

        let squad = vec![Fx::info(&prospect, date, 130, 0.6, 0)];
        let players = vec![prospect];

        let mut loan_outs = Vec::new();
        let mut force_list = Vec::new();
        PipelineProcessor::identify_stalled_prospects(
            &squad,
            date,
            &players,
            Fx::formation(),
            None,
            &mut loan_outs,
            &mut force_list,
            PipelineProcessor::is_january_window(date),
        );

        assert!(
            force_list.contains(&1),
            "two failed loans must escalate to a sale"
        );
        assert!(
            !loan_outs.iter().any(|c| c.player_id == 1),
            "a player with two failed loans must not be loaned a third time"
        );
    }

    #[test]
    fn sweep_loans_blocked_prospect_with_two_zero_seasons_in_summer() {
        // Multi-season detection works without the January checkpoint: two
        // frozen zero-official seasons + still nothing this pre-season →
        // end-of-season pathway loan for an under-runway prospect.
        let date = Fx::date(2027, 6, 10); // summer window, NOT January
        let need = Fx::group_min(PlayerFieldPositionGroup::Forward);

        let mut prospect = Fx::player(1, PlayerPositionType::Striker, 110, 21, 0);
        prospect
            .statistics_history
            .items
            .push(Fx::season_item(2024, false, 0));
        prospect
            .statistics_history
            .items
            .push(Fx::season_item(2025, false, 0));

        let mut squad = vec![Fx::info(&prospect, date, 145, 0.6, 0)];
        let mut players = vec![prospect];
        for i in 0..=need as u32 {
            let p = Fx::player(100 + i, PlayerPositionType::Striker, 130, 27, 25);
            squad.push(Fx::info(&p, date, 130, 0.5, 25));
            players.push(p);
        }

        let mut loan_outs = Vec::new();
        let mut force_list = Vec::new();
        PipelineProcessor::identify_stalled_prospects(
            &squad,
            date,
            &players,
            Fx::formation(),
            None,
            &mut loan_outs,
            &mut force_list,
            PipelineProcessor::is_january_window(date),
        );

        assert!(
            loan_outs.iter().any(|c| c.player_id == 1),
            "two consecutive zero-official seasons must escalate to a loan for an under-23"
        );
    }

    #[test]
    fn sweep_skips_loaned_pinned_and_already_listed_players() {
        let date = Fx::date(2027, 1, 15);
        let need = Fx::group_min(PlayerFieldPositionGroup::Forward);

        // A blocked prospect who is already on the loan list elsewhere must
        // not be double-tagged.
        let mut players = vec![Fx::player(1, PlayerPositionType::Striker, 110, 19, 0)];
        let mut squad = vec![Fx::info(&players[0], date, 145, 0.6, 0)];
        for i in 0..=need as u32 {
            let p = Fx::player(100 + i, PlayerPositionType::Striker, 130, 27, 25);
            squad.push(Fx::info(&p, date, 130, 0.5, 25));
            players.push(p);
        }

        let mut loan_outs = vec![LoanOutCandidate {
            player_id: 1,
            reason: LoanOutReason::Surplus,
            status: LoanOutStatus::Identified,
            loan_fee: 0.0,
            preferred_destination: LoanDestinationPreference::Any,
        }];
        let mut force_list = Vec::new();
        PipelineProcessor::identify_stalled_prospects(
            &squad,
            date,
            &players,
            Fx::formation(),
            None,
            &mut loan_outs,
            &mut force_list,
            PipelineProcessor::is_january_window(date),
        );

        assert_eq!(
            loan_outs.iter().filter(|c| c.player_id == 1).count(),
            1,
            "an already-planned player must not be re-listed by the sweep"
        );
    }

    // ───────────────────────── false-positive guards (task 3) ─────────

    #[test]
    fn second_choice_behind_one_player_is_not_blocked() {
        // Rank 1 — only one teammate ahead (even a clearly better one) — is
        // a credible #2 / rotation path, not a stalled asset. Covers the
        // second-choice goalkeeper and the genuine rotation player.
        let mut s = Fx::blocked_prospect(); // clearly_blocked = true
        s.depth_rank = 1;
        assert_eq!(
            ProspectPathway::decide(&s),
            PathwayAction::Hold,
            "a player narrowly behind one teammate keeps a credible path"
        );
    }

    #[test]
    fn regular_last_season_idle_this_preseason_is_held() {
        // Zero apps in a not-yet-started season but a full regular campaign
        // behind him → fixture timing, not a stalled pathway.
        let mut s = Fx::blocked_prospect();
        s.is_january = false;
        s.official_appearances = 0;
        s.last_season_official_games = 28;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Hold);
    }

    // ───────────────────────── asset-value protection (task 5) ────────

    #[test]
    fn valuable_short_contract_defers_to_renewal_before_loan() {
        // A high-ceiling prospect on a short deal must not be loaned while
        // renewal is still possible — the contract renewal manager gets the
        // first chance to protect the asset.
        let mut s = Fx::blocked_prospect();
        s.contract_months_remaining = Some(10);
        s.renewal_exhausted = false;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Hold);
    }

    #[test]
    fn valuable_short_contract_with_exhausted_renewal_is_sold() {
        // Renewal is impossible and the deal is nearly up — loaning him
        // would run the value down to a free transfer, so sell now instead.
        let mut s = Fx::blocked_prospect();
        s.contract_months_remaining = Some(10);
        s.renewal_exhausted = true;
        assert_eq!(ProspectPathway::decide(&s), PathwayAction::Sell);
    }

    #[test]
    fn valuable_long_contract_is_loaned_safely() {
        // Plenty of contract left → no value-loss risk → loan for minutes.
        let mut s = Fx::blocked_prospect();
        s.contract_months_remaining = Some(40);
        assert!(matches!(
            ProspectPathway::decide(&s),
            PathwayAction::LoanOut(_)
        ));
    }

    // ───────────────────────── history aggregation (task 8) ───────────

    #[test]
    fn failed_loan_count_ignores_loans_with_real_minutes() {
        let mut player = Fx::player(1, PlayerPositionType::Striker, 110, 22, 0);
        player
            .statistics_history
            .items
            .push(Fx::season_item(2024, true, 2)); // failed
        player
            .statistics_history
            .items
            .push(Fx::season_item(2025, true, 18)); // real minutes
        assert_eq!(PipelineProcessor::loan_spell_count(&player), 2);
        assert_eq!(
            PipelineProcessor::failed_loan_count(&player),
            1,
            "a loan with >= 5 official games is not a failed loan"
        );
    }

    #[test]
    fn season_aggregation_sums_parent_and_loan_spells() {
        // A season split between a parent spell (0 games) and a loan spell
        // (4 games) totals 4 — so it is NOT a zero season.
        let mut player = Fx::player(1, PlayerPositionType::Striker, 110, 22, 0);
        player
            .statistics_history
            .items
            .push(Fx::season_item(2025, false, 0));
        player
            .statistics_history
            .items
            .push(Fx::season_item(2025, true, 4));
        assert_eq!(
            PipelineProcessor::consecutive_zero_official_seasons(&player),
            0,
            "parent 0 + loan 4 in one season is 4 games, not a write-off"
        );
        assert_eq!(
            PipelineProcessor::last_completed_season_official_games(&player),
            4
        );
    }

    // ───────────────────────── unavailable guard (task 4) ─────────────

    #[test]
    fn sweep_skips_unavailable_non_injury_player() {
        // A suspended (non-injury) blocked prospect with no minutes must not
        // be pathway-listed — his lack of minutes is forced, not a club
        // choice.
        let date = Fx::date(2027, 1, 15);
        let need = Fx::group_min(PlayerFieldPositionGroup::Forward);

        let mut prospect = Fx::player(1, PlayerPositionType::Striker, 110, 19, 0);
        prospect.statuses.add(date, PlayerStatusType::Sus);

        let mut players = vec![prospect];
        let mut squad = vec![Fx::info(&players[0], date, 145, 0.6, 0)];
        for i in 0..=need as u32 {
            let p = Fx::player(100 + i, PlayerPositionType::Striker, 130, 27, 25);
            squad.push(Fx::info(&p, date, 130, 0.5, 25));
            players.push(p);
        }

        let mut loan_outs = Vec::new();
        let mut force_list = Vec::new();
        PipelineProcessor::identify_stalled_prospects(
            &squad,
            date,
            &players,
            Fx::formation(),
            None,
            &mut loan_outs,
            &mut force_list,
            PipelineProcessor::is_january_window(date),
        );

        assert!(
            !loan_outs.iter().any(|c| c.player_id == 1),
            "a suspended player must not be loan-listed for having no minutes"
        );
        assert!(!force_list.contains(&1));
    }

    // ──────────── core-player loan protection (the Litvinov fix) ───────

    /// Build a forward group where id 1 (CA 100) is a clear positional
    /// surplus behind four CA-120 forwards — the shape that would normally
    /// produce a `Surplus` loan candidate. Returns matching `players` +
    /// `squad` so a test can flip id 1's asset class and re-run.
    fn surplus_forward_scenario(date: NaiveDate) -> (Vec<Player>, Vec<SquadPlayerInfo>) {
        let mut players = vec![Fx::player(1, PlayerPositionType::Striker, 100, 27, 5)];
        let mut squad = vec![Fx::info(&players[0], date, 100, 0.3, 5)];
        for i in 0..4u32 {
            let p = Fx::player(100 + i, PlayerPositionType::Striker, 120, 27, 10);
            squad.push(Fx::info(&p, date, 120, 0.3, 10));
            players.push(p);
        }
        (players, squad)
    }

    fn run_loan_outs(
        squad: &[SquadPlayerInfo],
        players: &[Player],
        date: NaiveDate,
    ) -> Vec<LoanOutCandidate> {
        let mut loan_outs = Vec::new();
        PipelineProcessor::identify_loan_outs(
            squad,
            &ReputationLevel::Regional,
            116,
            date,
            players,
            &mut loan_outs,
            &ClubPhilosophy::Balanced,
            Fx::formation(),
            None,
            false,
            false,
            &SquadHomeContext {
                country_id: 1,
                country_code: "en",
                continent_id: 1,
            },
            0.5,
        );
        loan_outs
    }

    #[test]
    fn surplus_forward_loaned_only_when_not_first_team_protected() {
        let date = Fx::date(2026, 9, 5);

        // Baseline: an unprotected surplus forward IS a loan candidate.
        let (players, squad) = surplus_forward_scenario(date);
        assert!(
            run_loan_outs(&squad, &players, date)
                .iter()
                .any(|c| c.player_id == 1),
            "an unprotected surplus forward should be a loan candidate"
        );

        // The Litvinov case: the very same player, classified as a core
        // player, must never be loan-listed.
        let (players, mut squad) = surplus_forward_scenario(date);
        squad[0].asset_class = SquadAssetClass::CorePlayer;
        assert!(
            !run_loan_outs(&squad, &players, date)
                .iter()
                .any(|c| c.player_id == 1),
            "a core player must never be loan-listed"
        );

        // A first-team-useful player (e.g. a FirstTeamRegular) is equally
        // protected — covering "FirstTeamRegular with low minutes".
        let (players, mut squad) = surplus_forward_scenario(date);
        squad[0].asset_class = SquadAssetClass::FirstTeamUseful;
        assert!(
            !run_loan_outs(&squad, &players, date)
                .iter()
                .any(|c| c.player_id == 1),
            "a first-team-useful player must never be loan-listed"
        );
    }

    #[test]
    fn international_duty_player_is_not_loan_listed() {
        // Even a genuinely surplus-classed player away on international duty
        // must not be loan-listed: his low minutes are a call-up artefact,
        // not a club decision.
        let date = Fx::date(2026, 9, 5);
        let (mut players, mut squad) = surplus_forward_scenario(date);
        players[0].statuses.add(date, PlayerStatusType::Int);
        squad[0].asset_class = SquadAssetClass::TrueSurplus;
        assert!(
            !run_loan_outs(&squad, &players, date)
                .iter()
                .any(|c| c.player_id == 1),
            "a player on international duty must not be loan-listed for low minutes"
        );
    }
}

#[cfg(test)]
mod goalkeeper_prospect_tests {
    //! The keeper half of the youth prospect pipeline. `DevelopmentSigning`
    //! requests used to be emitted for Defender/Midfielder/Forward only, so a
    //! big club never scouted a young goalkeeper. These tests pin the new
    //! dedicated GK-prospect block: a youth-minded club with no young keeper
    //! on its first team grooms one, and a club that already has a young
    //! keeper does not double up.
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, MatchTacticType, PersonAttributes,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositions,
        PlayerSkills, StaffCollection, Tactics, TeamBuilder, TeamCollection, TeamReputation,
        TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    struct GkFx;

    impl GkFx {
        fn date() -> NaiveDate {
            // September — outside the January loan-out branches; pure
            // request-generation context.
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
        }

        fn player(id: u32, pos: PlayerPositionType, ca: u8, age: i32) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = ca.saturating_add(30);
            attrs.condition = 10_000;
            let contract =
                PlayerClubContract::new(20_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("G".into(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2026 - age, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: pos,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        /// A well-stocked Continental main team (→ Balanced philosophy, so
        /// `wants_youth` is true). Two senior keepers, no young keeper. Each
        /// outfield group carries two 19-year-olds so the OUTFIELD youth
        /// pipeline is already satisfied and consumes no budget, and the
        /// midfield sits one body below its depth requirement so a single
        /// cheap `DepthCover` need seeds `budget_per_need`. No GK need is
        /// produced by the earlier steps, so the new GK-prospect block is the
        /// only thing that can emit the keeper request. CA 185 keeps every
        /// group comfortably above the tier baseline (no QualityUpgrade).
        fn continental_club() -> Club {
            const SENIOR: u8 = 185;
            const YOUNG: u8 = 140;
            let mut players = vec![
                // 2 senior keepers, age 28 → not "young" (youth_age_max = 21).
                Self::player(1, PlayerPositionType::Goalkeeper, SENIOR, 28),
                Self::player(2, PlayerPositionType::Goalkeeper, SENIOR, 28),
            ];
            // DEF: 4 senior + 2 young = 6 (== depth req 6 → no need).
            for i in 0..4u32 {
                players.push(Self::player(
                    10 + i,
                    PlayerPositionType::DefenderCenter,
                    SENIOR,
                    27,
                ));
            }
            players.push(Self::player(
                20,
                PlayerPositionType::DefenderCenter,
                YOUNG,
                19,
            ));
            players.push(Self::player(
                21,
                PlayerPositionType::DefenderCenter,
                YOUNG,
                19,
            ));
            // MID: 2 senior + 2 young = 4 (< depth req 5 → DepthCover seed).
            players.push(Self::player(
                30,
                PlayerPositionType::MidfielderCenter,
                SENIOR,
                27,
            ));
            players.push(Self::player(
                31,
                PlayerPositionType::MidfielderCenter,
                SENIOR,
                27,
            ));
            players.push(Self::player(
                32,
                PlayerPositionType::MidfielderCenter,
                YOUNG,
                19,
            ));
            players.push(Self::player(
                33,
                PlayerPositionType::MidfielderCenter,
                YOUNG,
                19,
            ));
            // FWD: 1 senior + 2 young = 3 (== depth req 3 → no need).
            players.push(Self::player(40, PlayerPositionType::Striker, SENIOR, 27));
            players.push(Self::player(41, PlayerPositionType::Striker, YOUNG, 19));
            players.push(Self::player(42, PlayerPositionType::Striker, YOUNG, 19));

            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".into())
                .slug("main".into())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                // home·0.2 + national·0.3 + world·0.5 = 0.70 → Continental.
                .reputation(TeamReputation::new(7000, 7000, 7000))
                .tactics(Some(Tactics::new(MatchTacticType::T442)))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();

            Club::new(
                100,
                "Continental FC".to_string(),
                Location::new(1),
                ClubFinances::new(100_000_000, Vec::new()),
                ClubAcademy::new(10),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main]),
                ClubFacilities::default(),
            )
        }

        fn gk_development_request(eval: &SquadEvaluation) -> Option<&TransferRequest> {
            eval.requests.iter().find(|r| {
                r.reason == TransferNeedReason::DevelopmentSigning
                    && r.position.position_group() == PlayerFieldPositionGroup::Goalkeeper
            })
        }

        fn gk_succession_request(eval: &SquadEvaluation) -> Option<&TransferRequest> {
            eval.requests.iter().find(|r| {
                r.reason == TransferNeedReason::SuccessionPlanning
                    && r.position.position_group() == PlayerFieldPositionGroup::Goalkeeper
            })
        }
    }

    /// The Juventus case: a club whose number one is deep into his
    /// thirties, with nobody behind him but same-age deputies, must be
    /// shopping for a successor.
    ///
    /// Keeper succession never once ran before. The audit required the
    /// incumbent's ability to clear the squad's top-eleven average, and
    /// keepers score below outfield players on the unified ability scale
    /// by construction — so every club's number one was silently skipped
    /// and the search was never even considered.
    #[test]
    fn ageing_number_one_triggers_keeper_succession() {
        let mut club = GkFx::continental_club();
        {
            let team = &mut club.teams.teams[0];
            team.players.players.retain(|p| p.id != 1 && p.id != 2);
            // A 38-year-old first choice and a same-age deputy.
            team.players
                .add(GkFx::player(1, PlayerPositionType::Goalkeeper, 185, 38));
            team.players
                .add(GkFx::player(2, PlayerPositionType::Goalkeeper, 175, 37));
        }

        let eval = PipelineProcessor::evaluate_single_club(
            &club,
            GkFx::date(),
            None,
            false,
            &SquadHomeContext {
                country_id: 1,
                country_code: "en",
                continent_id: 1,
            },
        );
        let succession = GkFx::gk_succession_request(&eval);
        assert!(
            succession.is_some(),
            "a 38-year-old number one with no heir must start a succession search; got: {:?}",
            eval.requests
                .iter()
                .map(|r| (r.position, r.reason.clone()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            succession.unwrap().priority,
            TransferNeedPriority::Critical,
            "a season from the end with nobody behind him is as urgent as succession gets"
        );
    }

    /// …and it stands down once a real heir is in the building.
    #[test]
    fn a_young_heir_stands_the_succession_search_down() {
        let mut club = GkFx::continental_club();
        {
            let team = &mut club.teams.teams[0];
            team.players.players.retain(|p| p.id != 1 && p.id != 2);
            team.players
                .add(GkFx::player(1, PlayerPositionType::Goalkeeper, 185, 38));
            // A 21-year-old the scouts rate: PA is stamped at ca + 30.
            team.players
                .add(GkFx::player(2, PlayerPositionType::Goalkeeper, 180, 21));
        }

        let eval = PipelineProcessor::evaluate_single_club(
            &club,
            GkFx::date(),
            None,
            false,
            &SquadHomeContext {
                country_id: 1,
                country_code: "en",
                continent_id: 1,
            },
        );
        assert!(
            GkFx::gk_succession_request(&eval).is_none(),
            "don't shop for what the academy already delivered"
        );
    }

    #[test]
    fn youth_minded_club_signs_a_goalkeeper_prospect() {
        let club = GkFx::continental_club();
        // Precondition: a prospect-developing tier (Continental → Balanced).
        assert_eq!(
            club.teams.teams[0].reputation.level(),
            ReputationLevel::Continental
        );

        let eval = PipelineProcessor::evaluate_single_club(
            &club,
            GkFx::date(),
            None,
            false,
            &SquadHomeContext {
                country_id: 1,
                country_code: "en",
                continent_id: 1,
            },
        );

        let gk = GkFx::gk_development_request(&eval);
        assert!(
            gk.is_some(),
            "a youth-minded club with no young keeper must now generate a GK \
             DevelopmentSigning request; got: {:?}",
            eval.requests
                .iter()
                .map(|r| (r.position, r.reason.clone()))
                .collect::<Vec<_>>()
        );
        // It must use the raw-prospect age band, not a ready-made keeper.
        assert!(
            gk.unwrap().preferred_age_max <= 21,
            "a GK development signing should target teenagers / early-20s"
        );
    }

    #[test]
    fn club_with_a_young_keeper_does_not_double_up() {
        // Swap a senior keeper for a 19-year-old: the first team already has a
        // young keeper in the pipeline, so no GK prospect should be signed.
        let mut club = GkFx::continental_club();
        {
            let team = &mut club.teams.teams[0];
            team.players.players.retain(|p| p.id != 2);
            team.players
                .add(GkFx::player(2, PlayerPositionType::Goalkeeper, 140, 19));
        }

        let eval = PipelineProcessor::evaluate_single_club(
            &club,
            GkFx::date(),
            None,
            false,
            &SquadHomeContext {
                country_id: 1,
                country_code: "en",
                continent_id: 1,
            },
        );
        assert!(
            GkFx::gk_development_request(&eval).is_none(),
            "a club that already has a young keeper must not sign another keeper prospect"
        );
    }
}

#[cfg(test)]
mod investment_appetite_tests {
    //! Whether a club shops for a player it does not need — the motive that
    //! did not exist. Every buying need the evaluator raised was a form of
    //! patching, so a squad with no hole raised nothing, and a club with no
    //! request never scouted, never shortlisted and never bid.

    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::finance::ClubFinancialBalance;
    use crate::shared::Location;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, MatchTacticType, PlayerCollection,
        StaffCollection, Tactics, TeamBuilder, TeamCollection, TeamReputation, TeamType,
        TrainingSchedule,
    };
    use chrono::NaiveTime;

    struct AppetiteFx;

    impl AppetiteFx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
        }

        /// A club with a full year of income on the books, a given balance
        /// and a given reputation. Wages are zero (an empty roster), so
        /// `idle_cash` is exactly the balance and the cash axis is the only
        /// thing under test.
        fn club(reputation: u16, balance: i64, annual_income: i64) -> Club {
            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".into())
                .slug("main".into())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(Vec::new()))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(reputation, reputation, reputation))
                .tactics(Some(Tactics::new(MatchTacticType::T442)))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();

            let mut finance = ClubFinances::new(balance, Vec::new());
            // Twelve closed months, so `estimated_annual_income` is the
            // plain sum rather than an annualised guess.
            for month in 1..=12u32 {
                let mut snap = ClubFinancialBalance::new(balance);
                snap.income = annual_income / 12;
                finance
                    .history
                    .add(NaiveDate::from_ymd_opt(2026, month, 1).unwrap(), snap);
            }

            Club::new(
                100,
                "Test FC".to_string(),
                Location::new(1),
                finance,
                ClubAcademy::new(10),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main]),
                ClubFacilities::default(),
            )
        }

        fn rep_score(club: &Club) -> f32 {
            club.teams
                .main()
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.0)
        }
    }

    #[test]
    fn a_giant_sitting_on_years_of_income_shops_for_improvement() {
        // Manchester City's shape: ~$1.8bn in the bank against ~$600M of
        // income. This club watched one of the best midfielders in the
        // world play seven seasons in Turkey and never called.
        let club = AppetiteFx::club(9_500, 1_839_000_000, 600_000_000);
        let appetite =
            InvestmentAppetite::of(&club, AppetiteFx::rep_score(&club), AppetiteFx::date());
        assert!(appetite.is_active());
        assert!(appetite.score > 0.5, "score {}", appetite.score);
    }

    #[test]
    fn a_club_whose_cash_is_spoken_for_barely_does() {
        // Same reputation, no reserves: it can still be improved, but the
        // money for a player it does not need is not there.
        let club = AppetiteFx::club(9_500, 0, 600_000_000);
        let appetite =
            InvestmentAppetite::of(&club, AppetiteFx::rep_score(&club), AppetiteFx::date());
        assert!(
            appetite.score < 0.45,
            "a break-even giant must not shop like a cash-rich one: {}",
            appetite.score
        );
    }

    #[test]
    fn a_small_club_never_shops_for_players_it_does_not_need() {
        // Below the reputation floor there is no discretionary market at
        // all, however the balance sheet looks.
        let club = AppetiteFx::club(2_000, 500_000_000, 10_000_000);
        let appetite =
            InvestmentAppetite::of(&club, AppetiteFx::rep_score(&club), AppetiteFx::date());
        assert!(!appetite.is_active(), "score {}", appetite.score);
    }

    #[test]
    fn a_world_with_no_revenue_history_yet_produces_no_appetite() {
        // A freshly generated club has no income on record. A missing
        // denominator must never read as infinite wealth.
        let club = AppetiteFx::club(9_500, 1_000_000_000, 0);
        let appetite =
            InvestmentAppetite::of(&club, AppetiteFx::rep_score(&club), AppetiteFx::date());
        assert!(!appetite.is_active(), "score {}", appetite.score);
    }

    #[test]
    fn a_bolder_chairman_shops_harder_than_a_cautious_one() {
        let mut bold = AppetiteFx::club(9_000, 800_000_000, 400_000_000);
        bold.board.chairman.ambition = ChairmanAmbition::Reckless;
        let mut cautious = AppetiteFx::club(9_000, 800_000_000, 400_000_000);
        cautious.board.chairman.ambition = ChairmanAmbition::Conservative;

        let bold_score =
            InvestmentAppetite::of(&bold, AppetiteFx::rep_score(&bold), AppetiteFx::date()).score;
        let cautious_score = InvestmentAppetite::of(
            &cautious,
            AppetiteFx::rep_score(&cautious),
            AppetiteFx::date(),
        )
        .score;
        assert!(
            bold_score > cautious_score,
            "{bold_score} !> {cautious_score}"
        );
    }
}
