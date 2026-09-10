use chrono::NaiveDate;

use crate::club::player::transfer::AvailabilityBlockReason;
use crate::transfers::gate::EffectivePlayerReputation;
use crate::transfers::gate::fit::SquadFitSnapshot;
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::scouting::breakout::BreakoutPerformanceSignal;
use crate::transfers::scouting::exposure::{
    AvailabilityExposure, AvailabilitySignals, ExposureStage,
};
use crate::{Country, PlayerFieldPositionGroup, PositionCoverage, ReputationLevel, WageCalculator};

mod intake;
mod scan;

use intake::RecommendationIntake;
use scan::StaffAdvicePass;

/// Compact view of a listed-target candidate. Decouples the filter
/// from the live `PlayerSnapshot` so tests can construct synthetic
/// candidates without booting the full snapshot pipeline. The
/// player's id isn't needed by the evaluator (it's pure scoring) —
/// callers carry the snapshot reference alongside the view.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers) struct ListedTargetView {
    pub ability: u8,
    /// The target's passport. Read only by the registration gate — a club
    /// at its foreigner quota cannot sign one more.
    pub nationality_country_id: u32,
    pub estimated_potential: u8,
    pub age: u8,
    pub estimated_value: f64,
    pub position_group: PlayerFieldPositionGroup,
    pub is_listed: bool,
    pub is_transfer_requested: bool,
    pub is_unhappy: bool,
    /// Parent club has loan-listed the player. On its own this routes to
    /// the loan market, but a loan-listed player with a strong breakout
    /// score is treated as "available enough" for a permanent approach too
    /// — clubs buy the players smaller clubs only meant to loan out.
    pub is_loan_listed: bool,
    /// Performance-breakout discovery score (0..100) from
    /// [`crate::transfers::scouting::breakout::BreakoutPerformanceSignal`].
    /// A high score lets the player be discovered on *form* — admitting a
    /// loan-listed (or, in form-discovery mode, an unlisted) breakout
    /// player into this path and lifting his ranking — but it never relaxes
    /// the affordability / tier / reputation gates below.
    pub breakout_score: f32,
    pub world_reputation: i16,
    pub current_reputation: i16,
    pub ambition: f32,
    pub parent_club_score: f32,
    pub parent_club_in_debt: bool,
    /// Days since the player first became available. Drives the
    /// market-exposure staleness curve — softening and circulation lift.
    pub days_available: i64,
    /// Months left on contract; <= 6 reads as a bargain pickup.
    pub contract_months_remaining: i16,
    /// Capable player who is barely featuring — the market should want him
    /// even when his own club is ambivalent.
    pub low_usage: bool,
    /// Concrete approaches in the last 30 days (from the player's durable
    /// availability state). High interest damps the circulation lift.
    pub recent_interest_count: u8,
    /// Consecutive weekly circulation scans that found no taker.
    pub failed_scans: u16,
    /// Most recent circulation diagnosis of why the market stalled —
    /// steers which exposure softening arm accelerates.
    pub last_block: Option<AvailabilityBlockReason>,
}

/// The buyer's need picture, one entry per position group.
///
/// Built once per club so the per-candidate filter can ask which of the roles
/// a player covers this particular club is actually shopping for.
#[derive(Debug, Clone, Copy, Default)]
pub(in crate::transfers) struct BuyerNeedPicture {
    pub open_request: [bool; PlayerFieldPositionGroup::COUNT],
    pub aging_starter: [bool; PlayerFieldPositionGroup::COUNT],
    pub best_in_group: [u8; PlayerFieldPositionGroup::COUNT],
}

impl BuyerNeedPicture {
    /// Points a staff tip may fall short of an open request's ability bar
    /// and still count as an answer to it — a scout's read is noisy, a
    /// twenty-point miss is not noise. Without it an open "quality
    /// upgrade" request at a giant was answered by any tip in the group:
    /// the request said 168, the tip said 146, and the club bought the
    /// tip because "it had an open request there".
    pub const STAFF_TIP_ABILITY_TOLERANCE: u8 = 5;

    /// Which of this candidate's roles to judge the move against.
    ///
    /// Judging him by the group his primary label happens to fall in reads a
    /// wide forward against the buyer's MIDFIELD — where a strong side has no
    /// request, no room and a very high best-in-group — so he is rejected as
    /// no upgrade, while the centre-forward shirt he would have filled goes on
    /// unaddressed. The club's own open request comes first, then a starter
    /// running out of career, then simply where he improves the side most.
    /// Ties resolve to his primary label, so a single-group player is judged
    /// exactly as before.
    pub fn role_for(
        &self,
        coverage: PositionCoverage,
        primary: PlayerFieldPositionGroup,
    ) -> PlayerFieldPositionGroup {
        let mut best = primary;
        let mut best_rank = self.rank(primary);
        for group in PlayerFieldPositionGroup::ALL {
            if group == primary || !coverage.covers_group(group) {
                continue;
            }
            let rank = self.rank(group);
            if rank > best_rank {
                best = group;
                best_rank = rank;
            }
        }
        best
    }

    fn rank(&self, group: PlayerFieldPositionGroup) -> (u8, u8, u8) {
        let index = group.index();
        (
            self.open_request[index] as u8,
            self.aging_starter[index] as u8,
            // Weakest area first — that is where a signing does most good.
            u8::MAX - self.best_in_group[index],
        )
    }
}

/// Buyer-side context the filter consults. One struct, one place to
/// describe "who is looking, with what means, against what squad" —
/// keeps the per-target filter pure and trivially testable.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers) struct BuyerContext {
    /// Continuous reputation score (0..1) of the buyer.
    pub buyer_rep_score: f32,
    pub buyer_world_rep: i16,
    pub buyer_league_reputation: u16,
    pub buyer_total_wages: u32,
    pub buyer_wage_budget: u32,
    /// `plan.total_budget` — the transfer-budget cap.
    pub plan_total_budget: f64,
    /// Soft cap from `plan.total_budget * 2.0` — scouts shouldn't tag
    /// players the club cannot afford even with stretch. Pass 0.0 to
    /// disable.
    pub max_recommend_value: f64,
    /// Best CA at the target's position group on the buying squad.
    pub buyer_best_in_group: u8,
    /// Buyer has an open `TransferRequest` matching the target's group.
    pub has_open_request: bool,
    /// Buyer has a 30+ at-tier starter at the target's group — a
    /// succession opportunity that opens up a slot.
    pub has_aging_starter: bool,
    /// Year-round "breakout watch" mode. When set, a strong-breakout player
    /// who is NOT publicly available is still admitted so the club can open
    /// scouting monitoring on him purely on form. Off (the default) for the
    /// in-window listed-star sweep, which only surfaces players who have
    /// advertised availability. Either way the affordability / tier /
    /// reputation / squad-need gates are unchanged.
    pub form_discovery_mode: bool,
    /// Projection of the buyer's own surplus rules at the target's
    /// position group — rejects candidates who would classify as surplus
    /// the day they arrive. [`SquadFitSnapshot::disabled`] opts out.
    pub fit: SquadFitSnapshot,
}

/// Outcome of evaluating a listed target. Either rejected with a
/// specific reason (debug surface, also a clean test API) or accepted
/// with its weighted recruitment score.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::transfers) enum ListedTargetVerdict {
    Reject(ListedRejectReason),
    Accept(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::transfers) enum ListedRejectReason {
    NotListed,
    OutOfTierWindow,
    UnaffordableFee,
    UnaffordableWage,
    ReputationGapTooLarge,
    NoSquadNeed,
    NotAnUpgrade,
    /// The buyer's own surplus maths (squad-average gap / depth cap)
    /// would list this player weeks after he arrived — don't buy him.
    WouldBeSurplus,
    /// The buyer is already at its league's registered-foreigner quota:
    /// signing him would leave a squad it cannot register. Real clubs count
    /// their slots before they bid.
    WouldBeUnregistrable,
}

/// The screen a listed or breakout player is put through before a club
/// will move for him.
///
/// Pure: it reads a [`ListedTargetView`] against a [`BuyerContext`] and
/// returns a verdict. No world access, no RNG, no side effects — which
/// is why the listed sweep, the circulation pass and the breakout watch
/// can all share one answer.
pub(in crate::transfers) struct ListedTargetScreen;

/// What the hard gates worked out on the way through, handed to the scoring
/// so it does not recompute any of it. Every field here was already needed to
/// decide whether the target gets in at all.
/// What the reach gates worked out on the way through: this target is
/// findable, in the buyer's tier window, and affordable. Handed to the need
/// gates so they do not recompute any of it.
struct ReachedTarget {
    strong_breakout: bool,
    baseline: u8,
    exposure: AvailabilityExposure,
    affordability_cap: f64,
}

struct AdmittedTarget {
    exposure: AvailabilityExposure,
    affordability_cap: f64,
    upgrade: i16,
    weak_group: bool,
}

impl ListedTargetScreen {
    /// Pure evaluator for the listed-star / breakout sweep.
    ///
    /// Hard gates (reject if any fail):
    ///   • availability: a public market flag (Lst|Req|Unh), OR loan-listed
    ///     with a strong breakout score, OR — in `form_discovery_mode` — a
    ///     strong breakout alone (the year-round watch discovers on form)
    ///   • CA inside the buyer's tier window
    ///   • estimated fee within `plan_total_budget × 1.4`
    ///   • estimated wage within `wage_headroom × 1.3`
    ///   • world-reputation gap ≤ tier-scaled allowance
    ///   • a reason to act: a squad-need signal (weak group, open request,
    ///     aging starter) OR a market opportunity — a stale available player
    ///     or a genuine breakout who is at least depth-relevant or a resale
    ///     prospect. A breakout never relaxes the fee / wage / tier / rep gates.
    ///   • improvement: ≥ 3 CA above current best, an open request, or a
    ///     qualifying opportunity
    ///
    /// Soft scoring (sum, higher is better — used for top-N selection):
    ///   • improvement margin (capped to +30)
    ///   • prime-age bonus
    ///   • youth potential bonus
    ///   • status urgency (Req > Lst > Unh)
    ///   • seller-debt distress
    ///   • affordability headroom
    ///   • squad-need fit
    ///   • ambition-driven step-up
    ///   • stale-availability circulation lift
    ///   • performance-breakout lift
    pub(in crate::transfers) fn evaluate(
        target: &ListedTargetView,
        ctx: &BuyerContext,
    ) -> ListedTargetVerdict {
        match Self::admit(target, ctx) {
            Ok(admitted) => ListedTargetVerdict::Accept(Self::rank(target, ctx, &admitted)),
            Err(reason) => ListedTargetVerdict::Reject(reason),
        }
    }

    /// The hard gates, in order, returning the first one the target fails.
    /// None of them relax any other — a target that reaches the end is a
    /// realistic signing for this buyer, and only then gets ranked.
    fn admit(
        target: &ListedTargetView,
        ctx: &BuyerContext,
    ) -> Result<AdmittedTarget, ListedRejectReason> {
        let reached = Self::within_reach(target, ctx)?;
        Self::fills_a_need(target, ctx, reached)
    }

    /// Can this buyer get near him at all: is he findable, inside the tier
    /// window, and affordable in both fee and wage. Nothing here asks whether
    /// the buyer actually wants him.
    fn within_reach(
        target: &ListedTargetView,
        ctx: &BuyerContext,
    ) -> Result<ReachedTarget, ListedRejectReason> {
        use ListedRejectReason::*;

        // Availability gate. A player is "available enough" to pursue when he
        // is publicly on the market (Lst/Req/Unh); OR he is loan-listed AND his
        // form is a genuine breakout (clubs buy the players smaller clubs only
        // meant to loan out); OR, in year-round form-discovery mode, his form
        // alone is a strong breakout (scouting monitoring opens on talent, not
        // just on a for-sale sign). None of these relax the realism gates below
        // — they only decide whether the player enters this path at all.
        let strong_breakout =
            target.breakout_score >= BreakoutPerformanceSignal::BREAKOUT_THRESHOLD;
        let publicly_available =
            target.is_listed || target.is_transfer_requested || target.is_unhappy;
        // A clearly bigger club may pursue a smaller club's breakout star even
        // when he isn't listed — the realistic "giant comes for the second-
        // division top scorer". Bounded by a genuine breakout AND a real
        // reputation gap to the parent club; the tier-window, affordability and
        // plausibility gates below still apply, so this never becomes a
        // free-for-all. It converts the year-round breakout MONITORING that
        // `scan_breakout_form` builds into an actual in-window approach instead
        // of a row that just sits on the books waiting for the player to be
        // listed (which his selling club, holding an asset, rarely does).
        let buyer_outranks_parent = ctx.buyer_rep_score >= target.parent_club_score + 0.10;
        let available_enough = publicly_available
            || (target.is_loan_listed && strong_breakout)
            || (ctx.form_discovery_mode && strong_breakout)
            || (strong_breakout && buyer_outranks_parent);
        if !available_enough {
            return Err(NotListed);
        }

        // How far above a buyer's normal aspirational ceiling a genuinely
        // AVAILABLE player may sit and still be a realistic target — roughly one
        // tier. A surplus/listed man at a bigger club is the classic "smaller
        // side signs the fringe player who'd normally be out of reach"; without
        // this lift he was too good for lower tiers (ceiling) yet not an upgrade
        // for his own-tier peers (`upgrade < 3` below), so no club could sign him
        // and he sat listed forever. Affordability and the reputation-gap gate
        // still bound how far above his level a club can actually reach.
        const AVAILABLE_CEILING_RELAX: u8 = 20;

        let baseline =
            PipelineProcessor::tier_starter_ca_score(ctx.buyer_rep_score, target.position_group);
        let base_ceiling = PipelineProcessor::tier_target_ceiling_score(
            ctx.buyer_rep_score,
            target.position_group,
        );
        let ceiling = if publicly_available {
            base_ceiling.saturating_add(AVAILABLE_CEILING_RELAX)
        } else {
            base_ceiling
        };
        let floor = baseline.saturating_sub(20);
        if target.ability < floor || target.ability > ceiling {
            return Err(OutOfTierWindow);
        }

        // Market-exposure verdict — staleness, the seller/player softening
        // curves, and the circulation lift for the soft score. Pure function
        // of the observable signals; never relaxes the tier window above.
        let exposure = AvailabilityExposure::compute(&AvailabilitySignals {
            days_available: target.days_available,
            is_listed: target.is_listed,
            is_transfer_requested: target.is_transfer_requested,
            is_unhappy: target.is_unhappy,
            is_loan_listed: target.is_loan_listed,
            current_ability: target.ability,
            estimated_potential: target.estimated_potential,
            age: target.age,
            estimated_value: target.estimated_value,
            asking_to_value_ratio: if target.parent_club_in_debt { 0.9 } else { 1.1 },
            current_salary: 0,
            world_reputation: target.world_reputation,
            ambition: target.ambition,
            contract_months_remaining: target.contract_months_remaining,
            seller_in_debt: target.parent_club_in_debt,
            squad_surplus: false,
            low_usage_despite_ability: target.low_usage,
            recent_interest_count: target.recent_interest_count,
            failed_scans: target.failed_scans,
            last_block: target.last_block,
        });

        // Affordability — fee. The asking price softens the longer the player
        // sits unsold, so a stale listing becomes reachable for a club that
        // couldn't fund the headline value — bounded, never a giveaway.
        let asking_value = target.estimated_value * (1.0 - exposure.price_softening as f64);
        let affordability_cap = (ctx.plan_total_budget * 1.4).max(0.0);
        if affordability_cap <= 0.0 || asking_value > affordability_cap {
            return Err(UnaffordableFee);
        }
        if ctx.max_recommend_value > 0.0 && asking_value > ctx.max_recommend_value {
            return Err(UnaffordableFee);
        }

        // Affordability — wage proxy
        let estimated_wage = WageCalculator::expected_annual_wage_raw(
            target.ability,
            target.current_reputation,
            matches!(target.position_group, PlayerFieldPositionGroup::Forward),
            matches!(target.position_group, PlayerFieldPositionGroup::Goalkeeper),
            target.age,
            ctx.buyer_rep_score,
            ctx.buyer_league_reputation,
        );
        // The player relaxes his wage expectation over a dry spell, again
        // bounded by the softening curve.
        let softened_wage = (estimated_wage as f64 * (1.0 - exposure.wage_softening as f64)) as u64;
        let wage_headroom = (ctx.buyer_wage_budget as i64 - ctx.buyer_total_wages as i64).max(0);
        let wage_cap = (wage_headroom as f64 * 1.3) as u64;
        if wage_cap > 0 && softened_wage > wage_cap {
            return Err(UnaffordableWage);
        }

        // Reputation plausibility — tier-scaled gap (never softened: an
        // impossible-prestige move stays impossible regardless of staleness).
        // Uses EFFECTIVE reputation, not bare world rep: a player who is a
        // recognised name in his own market (high current/home standing) is
        // gauged on that domestic renown, so a low-world-rep domestic star is
        // not mistaken for a reachable bargain. `max(world, blend)` means a
        // player whose current rep is at/below his world rep is judged exactly
        // as before — the blend only ever raises the bar, never lowers it.
        let effective_rep = EffectivePlayerReputation::compute(
            target.world_reputation,
            target.current_reputation,
            target.current_reputation,
            true,
        )
        .max(target.world_reputation);
        let gap_allowed = (1200.0 + 2400.0 * ctx.buyer_rep_score) as i32;
        if (effective_rep as i32 - ctx.buyer_world_rep as i32) > gap_allowed {
            return Err(ReputationGapTooLarge);
        }
        Ok(ReachedTarget {
            strong_breakout,
            baseline,
            exposure,
            affordability_cap,
        })
    }

    /// Does the buyer want him — and would the squad still hold him a month
    /// later. The last two gates are deliberately immune to every bypass
    /// above: however tempting the deal, a club must not buy a player its own
    /// surplus maths would list weeks after he arrived.
    fn fills_a_need(
        target: &ListedTargetView,
        ctx: &BuyerContext,
        reached: ReachedTarget,
    ) -> Result<AdmittedTarget, ListedRejectReason> {
        use ListedRejectReason::*;
        let ReachedTarget {
            strong_breakout,
            baseline,
            exposure,
            affordability_cap,
        } = reached;

        // Squad need — OR a strong, stale market opportunity. A high-exposure
        // available player who is affordable and would add depth or future
        // resale value can be recommended even to a club without an open
        // positional need. Gated on non-Fresh staleness: a brand-new listing
        // never bypasses the need check, so the opportunity route is reserved
        // for players the market has had time to leave sitting — and it never
        // relaxes the tier / fee / wage / reputation gates above.
        let weak_group = (ctx.buyer_best_in_group as i16) < baseline as i16;
        let has_need = weak_group || ctx.has_open_request || ctx.has_aging_starter;
        let resale_value = target.age <= 23 && target.estimated_potential > target.ability + 5;
        let depth_value = (target.ability as i16) >= baseline as i16 - 10;
        // A stale, untouched available player OR a genuine performance breakout
        // is a market opportunity even without a conventional positional need —
        // but only when he is at least squad-relevant (depth) or a resale
        // prospect, so a club never chases a hot scorer plainly below its level.
        let stale_opportunity = !matches!(exposure.stage, ExposureStage::Fresh)
            && exposure.score >= 45.0
            && (resale_value || depth_value);
        let breakout_opportunity = strong_breakout && (resale_value || depth_value);
        let strong_opportunity = stale_opportunity || breakout_opportunity;
        if !(has_need || strong_opportunity) {
            return Err(NoSquadNeed);
        }

        // Improvement: a meaningful upgrade, coach-requested, a strong
        // market opportunity (depth / resale add), or an heir for a starter
        // running out of career.
        //
        // Succession is a bet on the future, not on this weekend. An heir is
        // by definition below the man he will replace, so measuring him
        // against the incumbent's CURRENT ability rejected exactly the
        // signing succession planning exists to make: a club could see it had
        // an ageing starter, form the need, and then refuse every young
        // player who could actually succeed him for not being better than him
        // today.
        let upgrade = (target.ability as i16) - (ctx.buyer_best_in_group as i16);
        let succession_value = ctx.has_aging_starter
            && target.age <= 26
            && (target.estimated_potential as i16) >= ctx.buyer_best_in_group as i16;
        if !ctx.has_open_request && !strong_opportunity && !succession_value && upgrade < 3 {
            return Err(NotAnUpgrade);
        }

        // Squad-fit projection — the terminal gate, and deliberately immune to
        // every bypass above (open request, bargain staleness, breakout form):
        // however tempting the deal, a club must not buy a player its own
        // surplus maths (squad-average gap, rebalance depth cap) would list
        // weeks after he arrived.
        if ctx
            .fit
            .would_be_surplus(target.ability, target.estimated_potential, target.age)
        {
            return Err(WouldBeSurplus);
        }

        // Registration, on the same terminal footing. A club does not bid for a
        // player it cannot register — and the sim must not either, because the
        // registration pass would omit him and the surplus maths would then list
        // a signing the club had just paid for.
        if ctx
            .fit
            .would_be_unregistrable(target.nationality_country_id)
        {
            return Err(WouldBeUnregistrable);
        }
        Ok(AdmittedTarget {
            exposure,
            affordability_cap,
            upgrade,
            weak_group,
        })
    }

    /// Ranks a survivor of [`Self::admit`]. Ranking only — every hard gate has
    /// already passed, so nothing here can keep a target out, only move it up
    /// or down the slate.
    fn rank(target: &ListedTargetView, ctx: &BuyerContext, admitted: &AdmittedTarget) -> f32 {
        let AdmittedTarget {
            exposure,
            affordability_cap,
            upgrade,
            weak_group,
        } = admitted;
        let affordability_cap = *affordability_cap;
        let upgrade = *upgrade;
        let weak_group = *weak_group;

        let mut score = 0.0_f32;
        score += (upgrade as f32).clamp(0.0, 30.0);

        score += match target.age {
            25..=29 => 5.0,
            22..=24 => 3.0,
            30 => 1.0,
            _ => 0.0,
        };

        if target.age <= 23 && target.estimated_potential > target.ability {
            let gap = (target.estimated_potential - target.ability) as f32;
            score += gap.clamp(0.0, 10.0);
        }

        if target.is_transfer_requested {
            score += 4.0;
        } else if target.is_listed {
            score += 2.0;
        } else if target.is_unhappy {
            score += 1.5;
        }

        if target.parent_club_in_debt {
            score += 2.0;
        }

        let headroom_ratio = if affordability_cap > 0.0 {
            ((affordability_cap - target.estimated_value) / affordability_cap).clamp(0.0, 1.0)
                as f32
        } else {
            0.0
        };
        score += headroom_ratio * 5.0;

        if ctx.has_open_request {
            score += 8.0;
        } else if weak_group {
            score += 4.0;
        } else if ctx.has_aging_starter {
            score += 2.0;
        }

        let tier_delta = ctx.buyer_rep_score - target.parent_club_score;
        score += tier_delta * 4.0 * target.ambition.clamp(0.0, 1.0);

        // Stale, untouched availability lifts the player up the ranking so a
        // genuinely-stuck Req/Unh/listed player doesn't disappear behind a
        // churn of fresher candidates.
        score += exposure.circulation_boost;

        // Breakout form lifts the player up the ranking so a genuinely hot
        // talent is pursued ahead of a merely-available one. Ranking only —
        // the hard gates above already passed.
        score += (target.breakout_score / 100.0) * 12.0;

        score
    }
}

impl PipelineProcessor {
    /// How many standing staff recommendations a club's plan holds before
    /// new ones are dropped. Smaller clubs carry more — their recruitment
    /// runs on tips, while a big club's department filters harder. One
    /// rule for every recommendation source (scout network, listed-star
    /// sweep, breakout watch), so no channel can flood the queue.
    pub(in crate::transfers) fn staff_recommendation_cap(rep: ReputationLevel) -> usize {
        match rep {
            ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur => 10,
            ReputationLevel::National => 8,
            _ => 6,
        }
    }

    /// Headroom the very biggest departments carry on top of
    /// [`Self::staff_recommendation_cap`] at full reputation.
    const RECOMMENDATION_CAP_REACH: f32 = 6.0;

    /// [`Self::staff_recommendation_cap`], widened continuously across the
    /// top tier.
    ///
    /// A flat six was the queue bound that made the standout-abroad channel
    /// self-defeating: the same cap that keeps a National side from
    /// drowning in tips also meant a club with a global scouting network
    /// could hold six standing recommendations in the entire world, and any
    /// seventh name — including a foreign marquee its own watch had just
    /// found — was silently dropped. The small-club tiers are unchanged
    /// (their recruitment genuinely does run on a handful of tips); above
    /// them the queue grows with the department that fills it.
    pub(in crate::transfers) fn staff_recommendation_cap_score(
        rep: ReputationLevel,
        score: f32,
    ) -> usize {
        let base = Self::staff_recommendation_cap(rep);
        match rep {
            ReputationLevel::Regional
            | ReputationLevel::Local
            | ReputationLevel::Amateur
            | ReputationLevel::National => base,
            _ => base + (score.clamp(0.0, 1.0) * Self::RECOMMENDATION_CAP_REACH).round() as usize,
        }
    }

    pub fn generate_staff_recommendations(country: &mut Country, date: NaiveDate) {
        // Only runs weekly (same schedule as should_evaluate)
        if !Self::should_evaluate_for(country, date) {
            return;
        }
        StaffAdvicePass::run(country, date);
    }

    pub fn process_staff_recommendations(country: &mut Country, date: NaiveDate) {
        // Only runs weekly (same schedule as should_evaluate)
        if !Self::should_evaluate_for(country, date) {
            return;
        }
        RecommendationIntake::run(country, date);
    }
}
