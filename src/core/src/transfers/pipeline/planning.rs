//! L1 — the squad planner and the recruitment brief.
//!
//! The evaluator that this module sits in front of answers one question:
//! *what is missing?* A shirt with nobody in it, a slot below the divisional
//! baseline, a group too thin to survive a suspension. Every one of those is
//! a form of patching, and a squad with no holes produced no request at all —
//! which is why a club sitting on years of income could go a decade without
//! making a call.
//!
//! A real recruitment department plans against an OBJECTIVE, not against a
//! hole. It writes down, for every shirt, the level a side finishing where
//! the board expects it to finish actually fields there; it compares the man
//! in the shirt against that number; and it decides how hard to shop given
//! how much of its own money the improvement would cost. The output is a
//! brief — a short list of slots with a tier, a required improvement, a
//! budget envelope, an age band and a promised role — and everything
//! downstream (scouting, watchlist, shortlist, board, negotiation) recruits
//! against the brief instead of against a deficit.
//!
//! Nothing here is a threshold on a country, a club or a name. The axes are
//! the objective the board set, the money the club has relative to its own
//! income, and how far each slot already sits above what the objective asks
//! for.

use std::collections::HashMap;

use chrono::{Datelike, Duration, NaiveDate, Weekday};

use crate::club::board::ChairmanAmbition;
use crate::club::player::contract::PlayerSquadStatus;
use crate::transfers::pipeline::auction::DeadlineWindow;
use crate::transfers::pipeline::evaluation::{GroupNeed, NeedKind, group_depth_requirement};
use crate::transfers::pipeline::processor::{PipelineProcessor, SquadPlayerInfo};
use crate::transfers::pipeline::{TransferNeedPriority, TransferNeedReason};
use crate::transfers::window::TransferCalendar;
use crate::{Club, ClubPhilosophy, Country, PlayerFieldPositionGroup, PlayerPositionType};

/// How transformative a briefed slot is meant to be, and therefore what
/// share of the window's money it may command.
///
/// The three tiers are the shape of a real window: one signing that changes
/// the side, one or two that raise its floor, and cover bought late and
/// cheap. They are not club sizes — a mid-table club briefs a tier A slot
/// when it has the money for one, and a giant briefs tier C for its third
/// keeper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BriefTier {
    /// Transformative: the club is shopping for a player who visibly
    /// improves its first team, and will spend most of a window on him.
    A,
    /// Upgrade: clearly better than the incumbent, at a sane price.
    B,
    /// Cover: a body for the bench, a loan, a free.
    C,
}

impl BriefTier {
    /// Share of the club's available budget one slot of this tier may
    /// carry. `A + B + C` deliberately sums to 1.0: a club that briefs one
    /// of each has allocated its window exactly once.
    ///
    /// Replaces the flat 15 % marquee allocation, which was itself a wall —
    /// the request could never carry the fee its own named target
    /// commanded.
    pub const A_SHARE: f64 = 0.60;
    pub const B_SHARE: f64 = 0.30;
    pub const C_SHARE: f64 = 0.10;

    /// Smallest improvement over the incumbent, in observable ability
    /// points, that justifies a move at this tier. Tier A is
    /// "transformative" by definition: twice the ordinary upgrade bar.
    const MIN_GAIN_BASE: i16 = 4;

    pub fn envelope_share(self) -> f64 {
        match self {
            BriefTier::A => Self::A_SHARE,
            BriefTier::B => Self::B_SHARE,
            BriefTier::C => Self::C_SHARE,
        }
    }

    /// The improvement bar a candidate has to clear to be worth pursuing
    /// for this slot.
    pub fn min_gain(self) -> i16 {
        match self {
            BriefTier::A => Self::MIN_GAIN_BASE * 2,
            BriefTier::B => Self::MIN_GAIN_BASE,
            // Cover is bought because the group is thin, not because the
            // signing is better than anyone — a fourth centre-back who is
            // merely adequate is still the point.
            BriefTier::C => 0,
        }
    }

    /// The role the club offers a player arriving into this slot. Read by
    /// the personal-terms model: a starter offered rotation says no unless
    /// the stage or the money makes up for it.
    pub fn promised_status(self) -> PlayerSquadStatus {
        match self {
            BriefTier::A => PlayerSquadStatus::KeyPlayer,
            BriefTier::B => PlayerSquadStatus::FirstTeamRegular,
            BriefTier::C => PlayerSquadStatus::FirstTeamSquadRotation,
        }
    }

    /// Whether a slot of this tier is bought or borrowed by default. Tier C
    /// is where the loan and free market lives.
    pub fn prefers_loan(self) -> bool {
        matches!(self, BriefTier::C)
    }

    /// How far below the plan's target for the shirt a candidate may sit and
    /// still be worth watching. Tight for the signings that are meant to
    /// change the side, wide for the bench.
    pub fn level_tolerance(self) -> i16 {
        match self {
            BriefTier::A => 8,
            BriefTier::B => 10,
            BriefTier::C => 15,
        }
    }
}

/// How much each part of the pitch is worth to a recruitment budget.
///
/// A club that can make one signing spends it where the shirt matters most,
/// and the market prices the same way — attacking players cost more than
/// equally good goalkeepers because more clubs want them. This is a weight
/// on the club's *appetite* for a discretionary slot, never a gate: a club
/// short a goalkeeper still briefs a goalkeeper.
pub struct RoleWeight;

impl RoleWeight {
    pub fn of(group: PlayerFieldPositionGroup) -> f32 {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 0.80,
            PlayerFieldPositionGroup::Defender => 1.00,
            PlayerFieldPositionGroup::Midfielder => 1.05,
            PlayerFieldPositionGroup::Forward => 1.15,
        }
    }
}

/// How much of a year's income a club could commit without touching the
/// cash it needs to cover its wage bill.
///
/// This is the term that makes the same €40M a rounding error for one club
/// and four years of business for another. It is the numerator of the
/// planner's appetite and the input to [`super::upgrade_math::MoneyUtility`]
/// on the buying side.
#[derive(Debug, Clone, Copy)]
pub struct MoneySlack {
    /// 0..1. Zero means every dollar the club has is already spoken for.
    pub ratio: f32,
    /// Cash genuinely idle behind the wage cover, in currency units.
    pub idle_cash: f64,
    /// Trailing annual income. Zero when the club has no revenue history.
    pub annual_income: f64,
}

impl MoneySlack {
    /// Years of wage bill a club wants in the bank before any of its cash
    /// reads as idle. Same figure the board's own budget ceiling uses, so
    /// the two agree about what "idle" means.
    pub const WAGE_COVER_YEARS: f64 = 1.5;
    /// Share of that idle cash the planner treats as spendable this window.
    /// Matches the board ceiling's `IDLE_CASH_SHARE`.
    const IDLE_CASH_SHARE: f64 = 0.08;

    /// Read one club's slack. `committed` is what the brief has already
    /// allocated this pass.
    pub fn of(club: &Club, date: NaiveDate, budget: f64, committed: f64) -> Self {
        let annual_income = club.finance.estimated_annual_income(date).max(0) as f64;
        let annual_wages: f64 = club
            .teams
            .iter()
            .map(|t| t.get_annual_salary() as f64)
            .sum();
        let idle_cash =
            (club.finance.balance.balance as f64 - annual_wages * Self::WAGE_COVER_YEARS).max(0.0);

        // No revenue evidence yet — a freshly generated world, or a club
        // whose first month has not closed. Reading a missing denominator
        // as "infinitely rich" would hand every club full appetite for its
        // first month, so fail closed exactly as the board's cold-start
        // budget seed does.
        if annual_income <= 0.0 {
            return MoneySlack {
                ratio: 0.0,
                idle_cash,
                annual_income: 0.0,
            };
        }

        let spendable = budget + idle_cash * Self::IDLE_CASH_SHARE - committed;
        MoneySlack {
            ratio: (spendable / annual_income).clamp(0.0, 1.0) as f32,
            idle_cash,
            annual_income,
        }
    }
}

/// Where the club is trying to finish, and what that does to the level it
/// shops for.
///
/// The divisional starter baseline says what a side of this REPUTATION
/// fields. The objective says what a side chasing THIS FINISH needs to
/// field, which is a different number: a newly-rich club expected to
/// challenge shops above its own standing, a club whose board asks only for
/// survival shops below it. Both sides of the shift are the same continuous
/// function of expected finish over league size.
#[derive(Debug, Clone, Copy)]
pub struct Ambition {
    /// 0 = champions, 1 = bottom of the table. The board's expected finish
    /// as a fraction of the division.
    pub expected_fraction: f32,
    /// 0..1 drive: how hard the club converts money into quality. Reputation
    /// standing × chairman × philosophy, the same ladder the board's own
    /// transfer tolerance uses.
    pub drive: f32,
}

impl Ambition {
    /// Ability points the objective adds at the very top of the table, and
    /// takes off at the very bottom. Fitted to the divisional baseline
    /// curve's own spread: the gap between a title side and a survival side
    /// in the same division is roughly a tier's worth of headroom, and
    /// `tier_starter_ca_score` already spans about 40 points across the
    /// whole reputation range, of which one division is a fraction.
    const OBJECTIVE_SHIFT: f32 = 8.0;
    /// Reputation below which a club's money is entirely spoken for by the
    /// squad it already has. Just under the National/Continental line — the
    /// same floor [`super::evaluation::InvestmentAppetite`] uses.
    const REPUTATION_FLOOR: f32 = 0.45;
    const REPUTATION_SPAN: f32 = 0.45;

    pub fn of(club: &Club, rep_score: f32) -> Self {
        // Expected finish as a fraction of the division. A board that has
        // set no targets yet reads as mid-table, which is the honest
        // neutral: no headroom either way.
        let expected_fraction = club
            .board
            .season_targets
            .as_ref()
            .map(|t| {
                let size = club
                    .teams
                    .main()
                    .and_then(|team| team.league_id)
                    .map(|_| 20.0)
                    .unwrap_or(20.0);
                ((t.expected_position as f32 - 1.0) / (size - 1.0)).clamp(0.0, 1.0)
            })
            .unwrap_or(0.5);

        let standing =
            ((rep_score - Self::REPUTATION_FLOOR) / Self::REPUTATION_SPAN).clamp(0.0, 1.0);
        let chairman = match club.board.chairman.ambition {
            ChairmanAmbition::Reckless => 1.30,
            ChairmanAmbition::Ambitious => 1.12,
            ChairmanAmbition::Balanced => 1.0,
            ChairmanAmbition::Conservative => 0.75,
        };
        let philosophy = match club.philosophy {
            ClubPhilosophy::SignToCompete => 1.20,
            ClubPhilosophy::DevelopAndSell => 1.05,
            ClubPhilosophy::Balanced => 1.0,
            ClubPhilosophy::LoanFocused => 0.55,
        };

        Ambition {
            expected_fraction,
            drive: (standing * chairman * philosophy).clamp(0.0, 1.0),
        }
    }

    /// Offset applied to the divisional starter baseline to get the level
    /// this club's objective actually asks for. `+OBJECTIVE_SHIFT` for a
    /// side expected to win the league, `−OBJECTIVE_SHIFT` for one expected
    /// to finish last, linear between.
    pub fn objective_shift(&self) -> i16 {
        (Self::OBJECTIVE_SHIFT * (1.0 - 2.0 * self.expected_fraction)).round() as i16
    }
}

/// The squad the club is trying to own: a level per shirt, a depth figure
/// per group, an age profile and a wage envelope.
///
/// Built before any need is raised, so the brief below can measure the
/// squad the club HAS against the squad the objective asks for rather than
/// against a divisional average nobody chose.
pub struct SquadPlan {
    /// Target observable level per formation slot, in slot order.
    slot_targets: Vec<(PlayerPositionType, u8)>,
    /// Bodies the plan wants per group, formation slots included.
    depth_targets: HashMap<PlayerFieldPositionGroup, usize>,
    /// Median age band the plan steers toward.
    age_profile_target: (u8, u8),
    /// Annual wage the club can add before it breaches its own mandate.
    wage_envelope: f64,
}

impl SquadPlan {
    /// Median squad age a side competing now steers toward, and the younger
    /// band a develop-and-sell club aims at. Both are the measured shape of
    /// the real thing, not a preference.
    const AGE_PROFILE_COMPETE: (u8, u8) = (25, 27);
    const AGE_PROFILE_DEVELOP: (u8, u8) = (22, 25);

    pub(in crate::transfers::pipeline) fn build(inputs: &PlanInputs<'_>) -> Self {
        let ambition = Ambition::of(inputs.club, inputs.rep_score);
        let shift = ambition.objective_shift();

        let slot_targets = inputs
            .formation_positions
            .iter()
            .map(|pos| {
                let group = pos.position_group();
                let baseline = PipelineProcessor::tier_starter_ca_score(inputs.rep_score, group);
                let target = (baseline as i16 + shift).clamp(20, 200) as u8;
                (*pos, target)
            })
            .collect();

        let mut depth_targets: HashMap<PlayerFieldPositionGroup, usize> = HashMap::new();
        for pos in inputs.formation_positions.iter() {
            let group = pos.position_group();
            depth_targets
                .entry(group)
                .or_insert_with(|| group_depth_requirement(inputs.formation_positions, group));
        }

        let age_profile_target = match inputs.club.philosophy {
            ClubPhilosophy::DevelopAndSell => Self::AGE_PROFILE_DEVELOP,
            _ => Self::AGE_PROFILE_COMPETE,
        };

        // What the club can add to the annual wage bill before the board's
        // own mandate refuses it. A club already over its mandate has none.
        let wage_budget = inputs
            .club
            .finance
            .wage_budget
            .as_ref()
            .map(|w| w.amount)
            .unwrap_or(0.0);
        let current_wages: f64 = inputs
            .club
            .teams
            .iter()
            .map(|t| t.get_annual_salary() as f64)
            .sum();

        SquadPlan {
            slot_targets,
            depth_targets,
            age_profile_target,
            wage_envelope: (wage_budget - current_wages).max(0.0),
        }
    }

    /// Level the plan asks for in one shirt.
    pub fn target_for(&self, position: PlayerPositionType) -> u8 {
        self.slot_targets
            .iter()
            .find(|(pos, _)| *pos == position)
            .map(|(_, level)| *level)
            .unwrap_or_else(|| {
                // A shirt outside the formation (an escalated request naming
                // a role the side does not currently play) takes the group's
                // own target.
                let group = position.position_group();
                self.slot_targets
                    .iter()
                    .filter(|(pos, _)| pos.position_group() == group)
                    .map(|(_, level)| *level)
                    .max()
                    .unwrap_or(0)
            })
    }

    pub fn depth_target(&self, group: PlayerFieldPositionGroup) -> usize {
        self.depth_targets.get(&group).copied().unwrap_or(0)
    }

    pub fn age_profile_target(&self) -> (u8, u8) {
        self.age_profile_target
    }

    pub fn wage_envelope(&self) -> f64 {
        self.wage_envelope
    }
}

/// One shirt the club has decided to shop for, and the terms it is shopping
/// on.
#[derive(Debug, Clone)]
pub struct BriefSlot {
    pub position: PlayerPositionType,
    pub group: PlayerFieldPositionGroup,
    pub tier: BriefTier,
    /// `target_level − incumbent_level`. Positive means a genuine hole; zero
    /// or negative means the slot is adequate and the club is shopping out
    /// of appetite.
    pub gap: i16,
    /// Improvement over the incumbent a candidate must offer.
    pub min_gain: i16,
    /// Money this slot may command.
    pub envelope: f64,
    pub age_band: (u8, u8),
    pub promised_status: PlayerSquadStatus,
    pub target_level: u8,
    pub incumbent_level: u8,
    /// The motive the request carries downstream — the existing enum, so
    /// scouting, the loan market and the UI are unchanged.
    pub reason: TransferNeedReason,
    pub priority: TransferNeedPriority,
}

impl BriefSlot {
    /// Ability floor the request advertises.
    ///
    /// Two floors, and the binding one wins: a candidate must be a real
    /// improvement on the man already in the shirt AND at least somewhere
    /// near the level the plan asks for. An empty shirt has no incumbent, so
    /// only the second speaks — which is what stops a formation gap
    /// admitting anybody with a pulse.
    pub fn min_ability(&self) -> u8 {
        let over_incumbent = self.incumbent_level as i16 + self.min_gain;
        let near_target = self.target_level as i16 - self.tier.level_tolerance();
        over_incumbent.max(near_target).clamp(1, 200) as u8
    }

    /// Ability the club would ideally sign — the plan's target for the
    /// shirt, or the improvement bar if that is higher.
    pub fn ideal_ability(&self) -> u8 {
        self.target_level.max(self.min_ability())
    }
}

/// The club's intent for the window: which shirts it is shopping for, and
/// what it will pay for each.
#[derive(Debug, Clone)]
pub struct RecruitmentBrief {
    pub slots: Vec<BriefSlot>,
    pub planned_on: NaiveDate,
    /// Money slack at plan time — carried so the diagnostics can explain a
    /// thin brief without recomputing it.
    pub money_slack: f32,
    pub ambition_drive: f32,
    /// True when the squad is at or above the board's registered cap. A
    /// full squad must sell before it buys, so the brief carries only what
    /// the club genuinely cannot do without.
    pub roster_full: bool,
    pub age_profile_target: (u8, u8),
}

impl RecruitmentBrief {
    pub fn empty(date: NaiveDate) -> Self {
        RecruitmentBrief {
            slots: Vec::new(),
            planned_on: date,
            money_slack: 0.0,
            ambition_drive: 0.0,
            roster_full: false,
            age_profile_target: SquadPlan::AGE_PROFILE_COMPETE,
        }
    }

    /// The brief slot a live request belongs to, matched on the shirt.
    pub fn slot_for(&self, position: PlayerPositionType) -> Option<&BriefSlot> {
        self.slots
            .iter()
            .find(|s| s.position == position)
            .or_else(|| {
                self.slots
                    .iter()
                    .find(|s| s.group == position.position_group())
            })
    }
}

/// Everything the planner reads. All of it already existed on the club or
/// was already computed by the squad evaluation — the planner adds no new
/// walk of the roster.
pub(in crate::transfers::pipeline) struct PlanInputs<'a> {
    pub club: &'a Club,
    pub squad: &'a [SquadPlayerInfo],
    /// `(shirt, incumbent, effective ability there)` for the eleven
    /// formation slots — exactly what the evaluator already builds.
    pub position_coverage: &'a [(PlayerPositionType, Option<u32>, u8)],
    pub formation_positions: &'a [PlayerPositionType; 11],
    pub rep_score: f32,
    /// The club's spendable transfer budget this window.
    pub available_budget: f64,
    /// The gap-driven needs the evaluator already detected. The planner
    /// takes them as an input rather than re-deriving them: a hole is a
    /// hole whatever the objective says.
    pub group_needs: &'a [GroupNeed],
    pub date: NaiveDate,
    /// Senior squad size, and the cap the board registered for the season.
    pub squad_size: usize,
    pub max_squad_size: usize,
}

/// Writes the brief.
pub struct SquadPlanner;

impl SquadPlanner {
    /// Appetite × role weight above which a slot with no hole in it is
    /// still briefed. Set so a club with a real objective and a third of a
    /// year's income spare shops for its weakest starting shirt, and a
    /// break-even club does not.
    pub const BRIEF_BAR: f32 = 0.30;
    /// Money slack at which a club can promote its top-demand slot to a
    /// transformative (tier A) search.
    const TIER_A_SLACK: f32 = 0.25;
    /// Ability points above the plan's target at which a slot counts as
    /// fully saturated and stops attracting appetite.
    const SATURATION_SPAN: f32 = 12.0;
    /// Most slots one brief carries. A window is a handful of decisions,
    /// not a rebuild — and each extra slot dilutes the envelopes.
    const MAX_SLOTS: usize = 5;

    pub(in crate::transfers::pipeline) fn plan(inputs: &PlanInputs<'_>) -> RecruitmentBrief {
        let plan = SquadPlan::build(inputs);
        let ambition = Ambition::of(inputs.club, inputs.rep_score);
        let slack = MoneySlack::of(inputs.club, inputs.date, inputs.available_budget, 0.0);
        let roster_full = inputs.max_squad_size > 0 && inputs.squad_size >= inputs.max_squad_size;

        // ── Score every formation slot ──────────────────────────────
        //
        // `demand` is what puts a shirt on the brief. It has two terms and
        // they are deliberately not interchangeable: a HOLE is a fact about
        // the squad, and APPETITE is a fact about the club's objective and
        // its bank balance. A hole is briefed whatever the money says; an
        // adequate slot is briefed only when the club both wants to and can.
        let mut candidates: Vec<SlotDemand> = Vec::new();
        for (position, incumbent, quality) in inputs.position_coverage.iter() {
            let group = position.position_group();
            let target = plan.target_for(*position);
            let incumbent_level = if incumbent.is_some() { *quality } else { 0 };
            let gap = target as i16 - incumbent_level as i16;

            // How far the slot already sits above what the objective asks
            // for. A club whose left wing is twelve points better than its
            // own plan demands does not shop for a left winger.
            let saturation = if gap >= 0 {
                0.0
            } else {
                ((-gap) as f32 / Self::SATURATION_SPAN).clamp(0.0, 1.0)
            };
            let appetite = ambition.drive * slack.ratio * (1.0 - saturation);
            let role = RoleWeight::of(group);

            let is_hole = incumbent.is_none() || gap > 0;
            let briefed = is_hole || appetite * role > Self::BRIEF_BAR;
            if !briefed {
                continue;
            }

            // Demand orders the brief and picks which single slot is worth
            // a transformative search. A hole outranks appetite by
            // construction: `gap` is in ability points, appetite is a
            // fraction, so a two-point hole already beats a saturated
            // slot's residual wish.
            let demand = gap.max(0) as f32 + appetite * role;
            candidates.push(SlotDemand {
                position: *position,
                group,
                target,
                incumbent_level,
                gap,
                appetite,
                demand,
                is_hole,
            });
        }

        candidates.sort_by(|a, b| {
            b.demand
                .partial_cmp(&a.demand)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| (a.position as u8).cmp(&(b.position as u8)))
        });

        // One brief slot per position GROUP. A club fixes its centre of
        // defence with one signing, not with three requests that each fund
        // themselves out of the same pot.
        let mut slots: Vec<BriefSlot> = Vec::new();
        let mut committed = 0.0_f64;
        let mut seen_groups: Vec<PlayerFieldPositionGroup> = Vec::new();
        for (rank, cand) in candidates.iter().enumerate() {
            if slots.len() >= Self::MAX_SLOTS {
                break;
            }
            if seen_groups.contains(&cand.group) {
                continue;
            }
            seen_groups.push(cand.group);

            let tier = Self::tier_for(cand, rank, slack.ratio, roster_full);
            let envelope = (inputs.available_budget * tier.envelope_share())
                .min((inputs.available_budget - committed).max(0.0));
            let reason = Self::reason_for(cand, inputs.group_needs);
            let priority = Self::priority_for(cand, tier);

            // The plan's age profile, widened by what the club is buying:
            // cover can be a veteran, a transformative signing has to have
            // resale years left in him.
            let age_band = Self::age_band(plan.age_profile_target(), tier, &reason);

            slots.push(BriefSlot {
                position: cand.position,
                group: cand.group,
                tier,
                gap: cand.gap,
                // The improvement bar is measured against the man in the
                // shirt — so an EMPTY shirt has only the tier's own bar to
                // clear. Folding the gap in there would demand that a
                // formation hole be filled at exactly the plan's target and
                // nothing less, which is not a search, it is a wish.
                min_gain: if cand.incumbent_level == 0 {
                    tier.min_gain()
                } else {
                    tier.min_gain().max(cand.gap.max(0))
                },
                envelope,
                age_band,
                promised_status: tier.promised_status(),
                target_level: cand.target,
                incumbent_level: cand.incumbent_level,
                reason,
                priority,
            });
            committed += envelope;
        }

        // A squad already at the registration cap keeps only the shirts it
        // genuinely cannot field without. Buying past the cap is not a
        // spending decision, it is an unregistrable one — and the club has
        // to move somebody on first (L4's sell list is what answers this).
        if roster_full {
            slots.retain(|s| s.gap > 0);
        }

        // Depth the plan wants and the squad does not have, expressed as
        // the cheapest tier. The evaluator's own DepthCover need covers the
        // same ground for the groups it detects; this catches the case
        // where the XI is fine and the bench is not.
        if !roster_full {
            Self::brief_depth(inputs, &plan, &mut slots, &mut committed);
        }

        RecruitmentBrief {
            slots,
            planned_on: inputs.date,
            money_slack: slack.ratio,
            ambition_drive: ambition.drive,
            roster_full,
            age_profile_target: plan.age_profile_target(),
        }
    }

    /// Which tier a slot is shopped at.
    ///
    /// Exactly one slot per brief can be transformative, and only when the
    /// club has the money for it — two tier-A envelopes would allocate 120 %
    /// of the window. Everything else is an upgrade or cover.
    fn tier_for(cand: &SlotDemand, rank: usize, slack: f32, roster_full: bool) -> BriefTier {
        if !cand.is_hole && cand.appetite <= 0.0 {
            return BriefTier::C;
        }
        // An empty shirt is never cover — somebody has to wear it.
        if cand.incumbent_level == 0 {
            return if slack >= Self::TIER_A_SLACK && rank == 0 {
                BriefTier::A
            } else {
                BriefTier::B
            };
        }
        if roster_full {
            return BriefTier::B;
        }
        if rank == 0 && slack >= Self::TIER_A_SLACK {
            BriefTier::A
        } else if cand.is_hole {
            BriefTier::B
        } else {
            // An adequate slot the club merely wants to improve is worth an
            // upgrade only while it has the money; otherwise it is cover.
            if slack >= Self::TIER_A_SLACK * 0.5 {
                BriefTier::B
            } else {
                BriefTier::C
            }
        }
    }

    /// The motive the request carries. The gap-driven kinds keep their
    /// existing names so scouting, the loan market and the UI read exactly
    /// what they always did; an appetite-driven slot is the discretionary
    /// [`TransferNeedReason::SquadInvestment`].
    fn reason_for(cand: &SlotDemand, group_needs: &[GroupNeed]) -> TransferNeedReason {
        if let Some(need) = group_needs.iter().find(|n| n.group == cand.group) {
            return match need.kind {
                NeedKind::FormationGap => TransferNeedReason::FormationGap,
                NeedKind::QualityUpgrade => TransferNeedReason::QualityUpgrade,
                NeedKind::DepthCover => TransferNeedReason::DepthCover,
            };
        }
        if cand.incumbent_level == 0 {
            TransferNeedReason::FormationGap
        } else if cand.is_hole {
            TransferNeedReason::QualityUpgrade
        } else {
            TransferNeedReason::SquadInvestment
        }
    }

    fn priority_for(cand: &SlotDemand, tier: BriefTier) -> TransferNeedPriority {
        if cand.incumbent_level == 0 {
            return TransferNeedPriority::Critical;
        }
        match tier {
            BriefTier::A | BriefTier::B if cand.is_hole => TransferNeedPriority::Important,
            _ => TransferNeedPriority::Optional,
        }
    }

    /// The age band the brief shops in.
    ///
    /// The plan's own profile is the centre; the tier widens it. A
    /// transformative signing is an ASSET as well as a footballer, so its
    /// band closes at the top — elite clubs do not pay a transformative fee
    /// for a 30-year-old. Cover opens both ends: an experienced free is
    /// exactly what a bench is for.
    fn age_band(profile: (u8, u8), tier: BriefTier, reason: &TransferNeedReason) -> (u8, u8) {
        // A development signing keeps its own band — the brief is not the
        // place to re-decide what a prospect is.
        if matches!(reason, TransferNeedReason::DevelopmentSigning) {
            return (16, 21);
        }
        let (lo, hi) = profile;
        match tier {
            BriefTier::A => (lo.saturating_sub(4).max(18), hi.saturating_add(2)),
            BriefTier::B => (lo.saturating_sub(5).max(18), hi.saturating_add(4)),
            BriefTier::C => (lo.saturating_sub(6).max(18), hi.saturating_add(8)),
        }
    }

    /// Brief the groups whose BENCH is short of the plan, at tier C.
    fn brief_depth(
        inputs: &PlanInputs<'_>,
        plan: &SquadPlan,
        slots: &mut Vec<BriefSlot>,
        committed: &mut f64,
    ) {
        for group in [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            if slots.len() >= Self::MAX_SLOTS {
                return;
            }
            if slots.iter().any(|s| s.group == group) {
                continue;
            }
            let have = inputs
                .squad
                .iter()
                .filter(|p| {
                    p.primary_position.position_group() == group
                        && !(p.is_injured && p.recovery_days > 30)
                })
                .count();
            if have >= plan.depth_target(group) {
                continue;
            }
            // The shirt the gap detector already named for this group when
            // it looked, falling back to the group's first formation slot.
            // `compute_group_needs` picks the specific uncovered slot, so a
            // side with four centre-backs and no left-back briefs the
            // left-back rather than another centre-back.
            let Some(position) = inputs
                .group_needs
                .iter()
                .find(|n| n.group == group)
                .map(|n| n.representative_pos)
                .or_else(|| {
                    inputs
                        .formation_positions
                        .iter()
                        .copied()
                        .find(|p| p.position_group() == group)
                })
            else {
                continue;
            };
            let target = plan.target_for(position);
            let envelope = (inputs.available_budget * BriefTier::C.envelope_share())
                .min((inputs.available_budget - *committed).max(0.0));
            slots.push(BriefSlot {
                position,
                group,
                tier: BriefTier::C,
                gap: 0,
                min_gain: 0,
                envelope,
                age_band: Self::age_band(
                    plan.age_profile_target(),
                    BriefTier::C,
                    &TransferNeedReason::DepthCover,
                ),
                promised_status: BriefTier::C.promised_status(),
                target_level: target,
                // Cover is measured against the divisional floor, not
                // against a starter: a bench player who is merely somewhere
                // near the level is doing his job.
                incumbent_level: target.saturating_sub(15),
                reason: TransferNeedReason::DepthCover,
                priority: TransferNeedPriority::Optional,
            });
            *committed += envelope;
        }
    }
}

/// When the planner runs.
///
/// The old pipeline planned only while a window was open, which is the
/// single largest reason it behaved like a procurement department: a club
/// with a hole in October could not do anything about it until June, and a
/// club with money and no hole never planned at all. Real recruitment runs
/// on its own calendar — a monthly review, a hard look about six weeks before
/// the window, and a re-plan whenever the world changes underneath it.
pub struct PlanningCadence;

impl PlanningCadence {
    /// Days before a window opens at which the club writes the brief it will
    /// actually shop on. Six weeks is the point at which real clubs have
    /// their targets agreed and their scouts already watching them.
    pub const PRE_WINDOW_DAYS: i64 = 45;

    /// True on a day the club should re-plan.
    ///
    /// Three cadences, deliberately overlapping: a monthly review all year,
    /// the pre-window look, and the existing in-window refresh (which the
    /// caller supplies, since it also drives the loan and prospect sweeps).
    pub fn should_plan(country: &Country, date: NaiveDate) -> bool {
        Self::is_monthly_review(date) || Self::is_pre_window_look(country, date)
    }

    /// First Monday of the month. Anchored on a weekday rather than the 1st
    /// so the planner shares the market's weekly rhythm — the watchlist,
    /// the breakout watch and the scouting pass all run on Mondays.
    fn is_monthly_review(date: NaiveDate) -> bool {
        date.weekday() == Weekday::Mon && date.day() <= 7
    }

    /// Exactly [`Self::PRE_WINDOW_DAYS`] out from either window opening.
    fn is_pre_window_look(country: &Country, date: NaiveDate) -> bool {
        let horizon = date + Duration::days(Self::PRE_WINDOW_DAYS);
        let calendar = TransferCalendar::for_country(&country.code, horizon);
        horizon == calendar.summer_window.0 || horizon == calendar.winter_window.0
    }

    /// Where today sits in the country's current window: days left and the
    /// window's own length. `closed` outside one, which is what makes every
    /// deadline term inert year-round.
    pub fn deadline_window(country: &Country, date: NaiveDate) -> DeadlineWindow {
        let calendar = TransferCalendar::for_country(&country.code, date);
        for (start, end) in [calendar.summer_window, calendar.winter_window] {
            if date >= start && date <= end {
                return DeadlineWindow::of((end - date).num_days(), (end - start).num_days());
            }
        }
        DeadlineWindow::closed()
    }
}

/// One scored formation slot, before it becomes a brief entry.
struct SlotDemand {
    position: PlayerPositionType,
    group: PlayerFieldPositionGroup,
    target: u8,
    incumbent_level: u8,
    gap: i16,
    appetite: f32,
    demand: f32,
    is_hole: bool,
}

#[cfg(test)]
mod planning_tests {
    use super::*;
    use crate::club::academy::ClubAcademy;
    use crate::club::board::SeasonTargets;
    use crate::club::team::builder::TeamBuilder;
    use crate::shared::Location;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubFinancialBalance, ClubStatus,
        MatchTacticType, PlayerCollection, StaffCollection, Tactics, TeamCollection,
        TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;

    /// A club is exactly three things to the planner: a board objective, a
    /// balance sheet, and a reputation. The fixture builds those and nothing
    /// else, so each test moves one axis at a time.
    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()
        }

        fn targets(expected_position: u8) -> SeasonTargets {
            SeasonTargets {
                transfer_budget: 0,
                wage_budget: 0,
                max_squad_size: 30,
                min_squad_size: 18,
                expected_position,
                min_acceptable_position: expected_position.saturating_add(5),
            }
        }

        /// A club with a full year of income on the books, a given balance
        /// and a given reputation. Wages are zero (an empty roster), so the
        /// idle cash is exactly the balance.
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
            for month in 1..=12u32 {
                let mut snap = ClubFinancialBalance::new(balance);
                snap.income = annual_income / 12;
                finance
                    .history
                    .add(NaiveDate::from_ymd_opt(2026, month, 1).unwrap(), snap);
            }

            let mut club = Club::new(
                100,
                "Test FC".to_string(),
                Location::new(1),
                finance,
                ClubAcademy::new(10),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main]),
                ClubFacilities::default(),
            );
            club.board.season_targets = Some(Self::targets(10));
            club
        }

        /// A briefed slot with no incumbent — the shape every field is read
        /// from in the floor tests below.
        fn empty_slot() -> BriefSlot {
            BriefSlot {
                position: PlayerPositionType::MidfielderCenter,
                group: PlayerFieldPositionGroup::Midfielder,
                tier: BriefTier::B,
                gap: 140,
                min_gain: BriefTier::B.min_gain(),
                envelope: 0.0,
                age_band: (20, 30),
                promised_status: PlayerSquadStatus::FirstTeamRegular,
                target_level: 140,
                incumbent_level: 0,
                reason: TransferNeedReason::FormationGap,
                priority: TransferNeedPriority::Critical,
            }
        }

        fn demand(incumbent_level: u8, gap: i16, appetite: f32) -> SlotDemand {
            SlotDemand {
                position: PlayerPositionType::MidfielderCenter,
                group: PlayerFieldPositionGroup::Midfielder,
                target: 150,
                incumbent_level,
                gap,
                appetite,
                demand: gap.max(0) as f32 + appetite,
                is_hole: gap > 0 || incumbent_level == 0,
            }
        }
    }

    #[test]
    fn a_title_objective_shops_above_the_divisional_baseline() {
        let mut club = Fx::club(8500, 0, 200_000_000);
        club.board.season_targets = Some(Fx::targets(1));
        let contender = Ambition::of(&club, 0.9);

        club.board.season_targets = Some(Fx::targets(18));
        let survivor = Ambition::of(&club, 0.9);

        assert!(
            contender.objective_shift() > survivor.objective_shift(),
            "a side told to win the league shops above one told to stay up: {} vs {}",
            contender.objective_shift(),
            survivor.objective_shift(),
        );
        assert!(contender.objective_shift() > 0);
        assert!(survivor.objective_shift() < 0);
    }

    #[test]
    fn a_board_with_no_targets_yet_reads_as_mid_table() {
        let mut club = Fx::club(8500, 0, 200_000_000);
        club.board.season_targets = None;
        assert_eq!(
            Ambition::of(&club, 0.9).objective_shift(),
            0,
            "no objective, no headroom either way"
        );
    }

    #[test]
    fn tier_envelopes_allocate_one_window_exactly_once() {
        let total = BriefTier::A.envelope_share()
            + BriefTier::B.envelope_share()
            + BriefTier::C.envelope_share();
        assert!(
            (total - 1.0).abs() < 1e-9,
            "one of each tier must be exactly one window: {total}"
        );
    }

    #[test]
    fn a_transformative_slot_demands_twice_the_ordinary_upgrade() {
        assert_eq!(BriefTier::A.min_gain(), BriefTier::B.min_gain() * 2);
        assert_eq!(BriefTier::C.min_gain(), 0);
    }

    #[test]
    fn the_promised_shirt_rises_with_the_tier() {
        assert_eq!(BriefTier::A.promised_status(), PlayerSquadStatus::KeyPlayer);
        assert_eq!(
            BriefTier::B.promised_status(),
            PlayerSquadStatus::FirstTeamRegular
        );
        assert_eq!(
            BriefTier::C.promised_status(),
            PlayerSquadStatus::FirstTeamSquadRotation
        );
    }

    #[test]
    fn a_club_with_no_revenue_history_has_no_slack() {
        let club = Fx::club(8500, 1_000_000_000, 0);
        assert_eq!(
            MoneySlack::of(&club, Fx::date(), 50_000_000.0, 0.0).ratio,
            0.0,
            "a club that has earned nothing cannot know what it can afford"
        );
    }

    #[test]
    fn slack_rises_with_the_money_and_falls_with_what_is_already_committed() {
        let club = Fx::club(8500, 400_000_000, 200_000_000);
        let free = MoneySlack::of(&club, Fx::date(), 60_000_000.0, 0.0).ratio;
        let committed = MoneySlack::of(&club, Fx::date(), 60_000_000.0, 50_000_000.0).ratio;
        let poor = MoneySlack::of(&club, Fx::date(), 5_000_000.0, 0.0).ratio;
        assert!(free > committed, "{free} vs {committed}");
        assert!(free > poor, "{free} vs {poor}");
    }

    #[test]
    fn an_empty_shirt_is_never_briefed_as_cover() {
        // A formation slot with nobody in it: whatever the money says, the
        // club is not shopping the bargain bin for it.
        let cand = Fx::demand(0, 140, 0.0);
        assert_ne!(SquadPlanner::tier_for(&cand, 3, 0.0, false), BriefTier::C);
    }

    #[test]
    fn only_a_club_with_money_briefs_a_transformative_search() {
        let cand = Fx::demand(145, 5, 0.4);
        assert_eq!(SquadPlanner::tier_for(&cand, 0, 0.9, false), BriefTier::A);
        assert_eq!(SquadPlanner::tier_for(&cand, 0, 0.02, false), BriefTier::B);
        assert_eq!(
            SquadPlanner::tier_for(&cand, 1, 0.9, false),
            BriefTier::B,
            "only the top-demand shirt is worth a whole window"
        );
    }

    #[test]
    fn a_full_squad_never_briefs_a_transformative_search() {
        let cand = Fx::demand(145, 5, 0.4);
        assert_eq!(SquadPlanner::tier_for(&cand, 0, 0.9, true), BriefTier::B);
    }

    #[test]
    fn an_adequate_slot_is_only_shopped_while_there_is_money_for_it() {
        let saturated = Fx::demand(160, -10, 0.4);
        assert_eq!(
            SquadPlanner::tier_for(&saturated, 2, 0.5, false),
            BriefTier::B
        );
        assert_eq!(
            SquadPlanner::tier_for(&saturated, 2, 0.01, false),
            BriefTier::C
        );
    }

    #[test]
    fn the_ability_floor_is_the_binding_one_of_the_two() {
        // An empty shirt has no incumbent to improve on, so the plan's own
        // target is what stops the request admitting anyone with a pulse.
        let empty = Fx::empty_slot();
        assert_eq!(
            empty.min_ability(),
            140 - BriefTier::B.level_tolerance() as u8
        );

        // A slot with a good incumbent is bound by the improvement instead.
        let filled = BriefSlot {
            min_gain: BriefTier::A.min_gain(),
            tier: BriefTier::A,
            incumbent_level: 160,
            target_level: 150,
            gap: -10,
            ..empty
        };
        assert_eq!(filled.min_ability(), 160 + BriefTier::A.min_gain() as u8);
    }

    #[test]
    fn a_transformative_search_will_not_buy_a_veteran() {
        let profile = (25u8, 27u8);
        let (_, a_max) =
            SquadPlanner::age_band(profile, BriefTier::A, &TransferNeedReason::QualityUpgrade);
        let (_, c_max) =
            SquadPlanner::age_band(profile, BriefTier::C, &TransferNeedReason::DepthCover);
        assert!(
            a_max < c_max,
            "an asset has to have resale years left in it: {a_max} vs {c_max}"
        );
    }

    #[test]
    fn a_development_signing_keeps_its_own_band_whatever_the_tier() {
        assert_eq!(
            SquadPlanner::age_band(
                (25, 27),
                BriefTier::A,
                &TransferNeedReason::DevelopmentSigning
            ),
            (16, 21)
        );
    }

    #[test]
    fn a_develop_and_sell_plan_steers_younger() {
        assert!(SquadPlan::AGE_PROFILE_DEVELOP.1 < SquadPlan::AGE_PROFILE_COMPETE.1);
    }

    // ── The planner end to end ───────────────────────────────────────
    //
    // A coverage table in, a brief out. These are the assertions that
    // would break if the wiring in `evaluate_single_club` ever stopped
    // producing a usable brief.

    /// Every formation slot covered at `level`, except `hole`, which is
    /// left empty.
    struct BriefFx;

    impl BriefFx {
        fn formation() -> &'static [PlayerPositionType; 11] {
            &crate::TACTICS_POSITIONS[0].1
        }

        fn coverage(
            level: u8,
            hole: Option<PlayerPositionType>,
        ) -> Vec<(PlayerPositionType, Option<u32>, u8)> {
            Self::formation()
                .iter()
                .enumerate()
                .map(|(i, pos)| {
                    if Some(*pos) == hole {
                        (*pos, None, 0)
                    } else {
                        (*pos, Some(i as u32 + 1), level)
                    }
                })
                .collect()
        }

        fn inputs<'a>(
            club: &'a Club,
            coverage: &'a [(PlayerPositionType, Option<u32>, u8)],
            squad: &'a [SquadPlayerInfo],
            available_budget: f64,
        ) -> PlanInputs<'a> {
            PlanInputs {
                club,
                squad,
                position_coverage: coverage,
                formation_positions: Self::formation(),
                rep_score: Self::REP_SCORE,
                available_budget,
                group_needs: &[],
                date: Fx::date(),
                squad_size: squad.len(),
                max_squad_size: 30,
            }
        }

        /// Reputation the fixtures shop at — a top side, so the objective
        /// shift and the appetite terms are both in play.
        const REP_SCORE: f32 = 0.85;

        /// The first shirt in the formation belonging to `group`. Written
        /// this way rather than naming a position, because which shirts a
        /// formation actually uses is the formation's business — hardcoding
        /// `Striker` silently matched nothing and the test passed a hole it
        /// had never made.
        fn shirt(group: PlayerFieldPositionGroup) -> PlayerPositionType {
            Self::formation()
                .iter()
                .copied()
                .find(|p| p.position_group() == group)
                .expect("every formation fields all four groups")
        }

        /// The level the plan asks for in one shirt — so a fixture can say
        /// "adequate" or "seven points better than it needs to be" without
        /// guessing at the baseline curve.
        fn target(club: &Club, position: PlayerPositionType) -> u8 {
            let coverage = Self::coverage(100, None);
            let squad: Vec<SquadPlayerInfo> = Vec::new();
            SquadPlan::build(&Self::inputs(club, &coverage, &squad, 0.0)).target_for(position)
        }
    }

    #[test]
    fn an_empty_shirt_is_always_briefed_whatever_the_money_says() {
        // A broke club with a hole in its XI still has to fill it — the
        // appetite term is about shopping for improvement, never about
        // whether a side can field eleven players.
        let club = Fx::club(8500, 0, 200_000_000);
        let squad: Vec<SquadPlayerInfo> = Vec::new();
        let empty_shirt = BriefFx::shirt(PlayerFieldPositionGroup::Forward);
        let level = BriefFx::target(&club, empty_shirt);
        let coverage = BriefFx::coverage(level, Some(empty_shirt));
        let brief = SquadPlanner::plan(&BriefFx::inputs(&club, &coverage, &squad, 0.0));

        let forward = brief
            .slots
            .iter()
            .find(|s| s.group == PlayerFieldPositionGroup::Forward)
            .expect("the empty shirt must be briefed");
        assert_eq!(forward.incumbent_level, 0);
        assert_eq!(forward.priority, TransferNeedPriority::Critical);
        assert_ne!(forward.tier, BriefTier::C);
    }

    #[test]
    fn a_squad_above_its_own_plan_with_no_money_briefs_nothing_but_cover() {
        // Every shirt filled far above the objective, and not a dollar
        // spare: this is the club that used to raise no request at all, and
        // it still should not shop for a first-team player.
        let club = Fx::club(8500, 0, 200_000_000);
        let squad: Vec<SquadPlayerInfo> = Vec::new();
        let coverage = BriefFx::coverage(200, None);
        // 200 is the ability ceiling — every shirt is as far above the
        // plan as the model allows.
        let brief = SquadPlanner::plan(&BriefFx::inputs(&club, &coverage, &squad, 0.0));

        assert!(
            brief.slots.iter().all(|s| s.tier == BriefTier::C),
            "nothing but cover: {:?}",
            brief.slots.iter().map(|s| s.tier).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_same_squad_with_money_shops_for_improvement_it_does_not_need() {
        // The whole point of L1. Same objective, same coverage shape — only
        // the balance sheet moves, and now the club briefs its weakest
        // starting shirt.
        let club = Fx::club(8500, 900_000_000, 200_000_000);
        let squad: Vec<SquadPlayerInfo> = Vec::new();
        let weak_shirt = BriefFx::shirt(PlayerFieldPositionGroup::Forward);
        let target = BriefFx::target(&club, weak_shirt);
        // Every shirt comfortably clear of the plan, except one that is
        // merely AT it. Nothing is missing; the club is simply able to do
        // better in one place, which is the case the old evaluator could
        // not express at all.
        let mut coverage = BriefFx::coverage(target + 20, None);
        for slot in coverage.iter_mut() {
            if slot.0 == weak_shirt {
                slot.2 = target;
            }
        }
        let brief = SquadPlanner::plan(&BriefFx::inputs(&club, &coverage, &squad, 80_000_000.0));

        assert!(brief.money_slack > 0.0, "the money is the whole input");
        assert!(
            brief.slots.iter().any(|s| s.tier != BriefTier::C),
            "a cash-rich contender shops above cover: {:?}",
            brief
                .slots
                .iter()
                .map(|s| (s.position, s.tier))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_brief_never_allocates_more_than_the_budget_it_was_given() {
        let club = Fx::club(8500, 900_000_000, 200_000_000);
        let squad: Vec<SquadPlayerInfo> = Vec::new();
        let coverage = BriefFx::coverage(120, None);
        let budget = 80_000_000.0;
        let brief = SquadPlanner::plan(&BriefFx::inputs(&club, &coverage, &squad, budget));

        let committed: f64 = brief.slots.iter().map(|s| s.envelope).sum();
        assert!(
            committed <= budget + 1.0,
            "the window is allocated at most once: {committed} of {budget}"
        );
    }

    #[test]
    fn at_most_one_shirt_per_group_and_one_transformative_search() {
        let club = Fx::club(8500, 900_000_000, 200_000_000);
        let squad: Vec<SquadPlayerInfo> = Vec::new();
        let coverage = BriefFx::coverage(120, None);
        let brief = SquadPlanner::plan(&BriefFx::inputs(&club, &coverage, &squad, 80_000_000.0));

        let mut groups: Vec<usize> = brief.slots.iter().map(|s| s.group.index()).collect();
        let before = groups.len();
        groups.sort_unstable();
        groups.dedup();
        assert_eq!(before, groups.len(), "one shirt per group");
        assert!(
            brief
                .slots
                .iter()
                .filter(|s| s.tier == BriefTier::A)
                .count()
                <= 1,
            "two transformative envelopes would allocate 120% of a window"
        );
    }

    #[test]
    fn a_full_squad_briefs_only_the_shirts_it_cannot_field() {
        let club = Fx::club(8500, 900_000_000, 200_000_000);
        let squad: Vec<SquadPlayerInfo> = Vec::new();
        let coverage = BriefFx::coverage(120, None);
        let mut inputs = BriefFx::inputs(&club, &coverage, &squad, 80_000_000.0);
        inputs.squad_size = 40;
        inputs.max_squad_size = 30;
        let brief = SquadPlanner::plan(&inputs);

        assert!(brief.roster_full);
        assert!(
            brief.slots.iter().all(|s| s.gap > 0),
            "a club that cannot register another player buys only what it must"
        );
    }
}
