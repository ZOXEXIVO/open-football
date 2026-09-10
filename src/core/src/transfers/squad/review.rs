//! One club's squad review, as the head coach runs it.
//!
//! Six numbered STEPs plus a brief and an asset ledger — the sequence was
//! already written out in comments inside a single 1 042-line
//! `evaluate_single_club`, which is why none of it could be tested or
//! reordered. They are methods here, and every body moved unchanged.
//!
//! Two types carry what the steps were sharing as locals. [`SquadReview`] is
//! the read-only picture: the squad, the formation it is measured against, the
//! money, and where in the world this club plays. [`ReviewLedger`] is the
//! review in progress — the brief the club settled on, the requests raised so
//! far, and what is left of the pot. The steps genuinely interleave, each
//! reading what the ones before it put in the ledger, so this is an
//! accumulator and not six independent producers.

use chrono::NaiveDate;
use std::collections::HashMap;

use crate::club::staff::Staff;
use crate::club::staff::perception::{EstimationContext, PotentialEstimator};
use crate::club::team::squad::{MIN_FIRST_TEAM_SQUAD, SquadAssetContext};
use crate::transfers::loan::home::SquadHomeContext;
use crate::transfers::pipeline::processor::{PipelineProcessor, SquadPlayerInfo};
use crate::transfers::pipeline::trace::TransferTrace;
use crate::transfers::pipeline::{
    LoanOutCandidate, TransferNeedPriority, TransferNeedReason, TransferRequest,
};
use crate::{
    Club, ClubPhilosophy, MatchTacticType, Person, Player, PlayerFieldPositionGroup,
    PlayerPositionType, ReputationLevel, RoleFamiliarity, TacticsSelector, Team,
};

use super::needs::{EmergencyGroupSlot, FirstTeamSquadNeeds};
use super::plan::{PlanInputs, RecruitmentBrief, SquadPlanner};
use super::{
    GroupNeed, GroupNeedScan, InvestmentAppetite, InvestmentWatch, SquadEvaluation,
    SuccessionAudit, SuccessionUrgency,
};

/// Either a review to run, or — for a club with no squad to review — the
/// bootstrap evaluation that stands in for one.
pub(in crate::transfers::squad) enum SquadReviewOpening<'a> {
    Review(SquadReview<'a>),
    Bootstrap(SquadEvaluation),
}

/// Everything the head coach reads, resolved once: the squad, the formation it
/// is measured against, the money, and where in the world this club plays.
pub(in crate::transfers::squad) struct SquadReview<'a> {
    club: &'a Club,
    team: &'a Team,
    players: &'a [Player],
    /// Where this squad actually is. A club carries no country of its own, and
    /// "is this player a foreigner here?" — the question the unsettled-abroad
    /// loan reason turns on — cannot be answered without one.
    home: &'a SquadHomeContext<'a>,
    date: NaiveDate,
    current_window: Option<(NaiveDate, NaiveDate)>,
    /// True inside the country's shorter (mid-season) window — the point in the
    /// calendar where a club reviews who still isn't playing.
    mid_season_window: bool,
    budget: f64,
    max_concurrent: u32,
    rep_level: ReputationLevel,
    /// Continuous reputation score — drives tier baselines without snapping to
    /// enum boundaries. A team mid-Continental gets a different threshold from
    /// a team top-of-Continental.
    rep_score: f32,
    ability_tolerance: i16,
    youth_age_max: u8,
    avg_ability: u8,
    asset_ctx: SquadAssetContext,
    squad: Vec<SquadPlayerInfo>,
    formation_positions: &'static [PlayerPositionType; 11],
    /// One entry per formation slot: (position, who covers it, how well).
    position_coverage: Vec<(PlayerPositionType, Option<u32>, u8)>,
}

/// The review in progress. Every step reads what the steps before it put here:
/// the brief they are all funding from, the requests already raised, and what
/// is left of the pot.
pub(in crate::transfers::squad) struct ReviewLedger {
    brief: RecruitmentBrief,
    group_needs: Vec<GroupNeed>,
    discretionary_unit: f64,
    available_budget: f64,
    budget_used: f64,
    requests: Vec<TransferRequest>,
    next_id: u32,
}

impl<'a> SquadReview<'a> {
    /// Resolve the squad, the money and the formation gaps — STEPs 1 and 2,
    /// which produce no request of their own.
    pub(in crate::transfers::squad) fn open(
        club: &'a Club,
        date: NaiveDate,
        current_window: Option<(NaiveDate, NaiveDate)>,
        mid_season_window: bool,
        home: &'a SquadHomeContext<'a>,
    ) -> SquadReviewOpening<'a> {
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
            return Self::bootstrap(club, budget, false);
        }

        let team = &club.teams.teams[0];
        let players = &team.players.players;

        if players.is_empty() {
            return Self::bootstrap(club, budget, true);
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
        let squad = Self::read_squad(players, head_coach, &asset_ctx, date);

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
        let formation_positions = PipelineProcessor::get_formation_positions(formation);

        let position_coverage = Self::map_formation(&squad, formation_positions);

        let rep_score = team.reputation.overall_score();
        SquadReviewOpening::Review(SquadReview {
            club,
            team,
            players,
            home,
            date,
            current_window,
            mid_season_window,
            budget,
            max_concurrent,
            rep_level,
            rep_score,
            ability_tolerance,
            youth_age_max,
            avg_ability,
            asset_ctx,
            squad,
            formation_positions,
            position_coverage,
        })
    }

    /// A club with no squad to review still has to bootstrap: without an open
    /// request the emergency free-agent pass has nothing to react to, and a
    /// fresh or wiped club would never sign anybody at all.
    fn bootstrap(club: &Club, budget: f64, with_needs: bool) -> SquadReviewOpening<'a> {
        let mut requests = Vec::new();
        let mut next_id = club.transfer_plan.next_request_id;
        if with_needs {
            // Empty main team: emit group-aware FormationGap requests
            // for every position group so the emergency free-agent pass
            // (in `country::result::transfers::free`) has a
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
        }
        SquadReviewOpening::Bootstrap(SquadEvaluation {
            club_id: club.id,
            requests,
            loan_outs: Vec::new(),
            force_transfer_list: Vec::new(),
            total_budget: budget,
            max_concurrent: 1,
            brief: None,
            sell_list: Vec::new(),
        })
    }

    /// The squad as the head coach reads it. `estimated_potential` is what he
    /// *believes* the ceiling is, built from visible signals only — never the
    /// raw PA. Everyone here is on the main roster, so all get full-visibility
    /// observations.
    fn read_squad(
        players: &[Player],
        head_coach: &Staff,
        asset_ctx: &SquadAssetContext,
        date: NaiveDate,
    ) -> Vec<SquadPlayerInfo> {
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
        squad
    }

    /// STEP 2 — map the formation's eleven shirts onto the squad. A shirt has
    /// coverage when at least one unassigned, available player can play there
    /// adequately; anything else is a gap for STEP 3 to price.
    fn map_formation(
        squad: &[SquadPlayerInfo],
        formation_positions: &'static [PlayerPositionType; 11],
    ) -> Vec<(PlayerPositionType, Option<u32>, u8)> {
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
        position_coverage
    }

    /// The steps, in the order the old body ran them.
    pub(in crate::transfers::squad) fn run(self) -> SquadEvaluation {
        let mut ledger = self.open_briefs();
        self.investment_requests(&mut ledger);
        self.succession_requests(&mut ledger);
        self.pre_departure_requests(&mut ledger);
        self.youth_requests(&mut ledger);
        self.padding_and_loan_requests(&mut ledger);
        self.finish(ledger)
    }

    /// STEP 3 — the formation gaps become requests, and the planner turns them
    /// into the brief everything downstream funds from. Returns the ledger the
    /// rest of the review spends.
    fn open_briefs(&self) -> ReviewLedger {
        let club = self.club;
        let date = self.date;
        let team = self.team;
        let budget = self.budget;
        let squad = self.squad.as_slice();
        let ability_tolerance = self.ability_tolerance;
        let formation_positions = self.formation_positions;
        let position_coverage = &self.position_coverage;
        let mut requests: Vec<TransferRequest> = Vec::new();
        let mut next_id = self.club.transfer_plan.next_request_id;

        // STEP 3: Generate transfer requests from gaps
        // ──────────────────────────────────────────────────────────

        let available_budget = budget * 0.9; // Keep 10% reserve
        let mut budget_used = 0.0;

        // Continuous reputation score — drives tier baselines without
        // snapping to enum boundaries. A team mid-Continental gets a
        // different threshold from a team top-of-Continental.
        let rep_score = team.reputation.overall_score();
        let quality_tolerance = PipelineProcessor::tier_quality_tolerance_score(rep_score);
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
        // Detection lives in `GroupNeedScan::needs` (pure function, unit
        // tested in helpers' test module).
        let group_needs = GroupNeedScan::needs(
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

        ReviewLedger {
            brief,
            group_needs,
            discretionary_unit,
            available_budget,
            budget_used,
            requests,
            next_id,
        }
    }

    /// STEP 3b — squad investment: buying to get better, not to patch.
    fn investment_requests(&self, ledger: &mut ReviewLedger) {
        let club = self.club;
        let date = self.date;
        let rep_score = self.rep_score;
        let squad = self.squad.as_slice();
        let available_budget = ledger.available_budget;
        let group_needs = &ledger.group_needs;
        let mut budget_used = ledger.budget_used;
        let mut next_id = ledger.next_id;
        let mut requests = std::mem::take(&mut ledger.requests);

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
                    .min(available_budget * PipelineProcessor::MAX_INVESTMENT_SHARE)
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

        ledger.requests = requests;
        ledger.budget_used = budget_used;
        ledger.next_id = next_id;
    }

    /// STEP 4 — succession planning for aging key players.
    fn succession_requests(&self, ledger: &mut ReviewLedger) {
        let rep_level = self.rep_level.clone();
        let squad = self.squad.as_slice();
        let philosophy = &self.club.philosophy;
        let available_budget = ledger.available_budget;
        let discretionary_unit = ledger.discretionary_unit;
        let mut budget_used = ledger.budget_used;
        let mut next_id = ledger.next_id;
        let mut requests = std::mem::take(&mut ledger.requests);

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
            for player_info in squad {
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

        ledger.requests = requests;
        ledger.budget_used = budget_used;
        ledger.next_id = next_id;
    }

    /// STEP 4a — a replacement lined up before an expiring key player goes.
    fn pre_departure_requests(&self, ledger: &mut ReviewLedger) {
        let rep_score = self.rep_score;
        let squad = self.squad.as_slice();
        let available_budget = ledger.available_budget;
        let discretionary_unit = ledger.discretionary_unit;
        let mut budget_used = ledger.budget_used;
        let mut next_id = ledger.next_id;
        let mut requests = std::mem::take(&mut ledger.requests);

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
        for player_info in squad {
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
            let baseline = PipelineProcessor::tier_starter_ca_score(rep_score, group);
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

        ledger.requests = requests;
        ledger.budget_used = budget_used;
        ledger.next_id = next_id;
    }

    /// STEP 4b — youth development signings. Elite and Continental clubs farm
    /// prospects actively; smaller ones sign the ones they can reach.
    fn youth_requests(&self, ledger: &mut ReviewLedger) {
        let club = self.club;
        let date = self.date;
        let rep_level = self.rep_level.clone();
        let squad = self.squad.as_slice();
        let avg_ability = self.avg_ability;
        let philosophy = &self.club.philosophy;
        let youth_age_max = self.youth_age_max;
        let available_budget = ledger.available_budget;
        let discretionary_unit = ledger.discretionary_unit;
        let mut budget_used = ledger.budget_used;
        let mut next_id = ledger.next_id;
        let mut requests = std::mem::take(&mut ledger.requests);

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

        ledger.requests = requests;
        ledger.budget_used = budget_used;
        ledger.next_id = next_id;
    }

    /// STEP 5 — small-club squad padding and loan needs.
    fn padding_and_loan_requests(&self, ledger: &mut ReviewLedger) {
        let club = self.club;
        let rep_level = self.rep_level.clone();
        let squad = self.squad.as_slice();
        let avg_ability = self.avg_ability;
        let available_budget = ledger.available_budget;
        let discretionary_unit = ledger.discretionary_unit;
        let mut budget_used = ledger.budget_used;
        let mut next_id = ledger.next_id;
        let mut requests = std::mem::take(&mut ledger.requests);

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

        ledger.requests = requests;
        ledger.budget_used = budget_used;
        ledger.next_id = next_id;
    }

    /// STEP 6 — who goes out on loan, and who the club would list outright;
    /// then the asset ledger, priced last because the sell list reads what the
    /// brief asked for. A club whose plan wants more money than it has is a
    /// more willing seller than the same club with its window already funded.
    fn finish(&self, ledger: ReviewLedger) -> SquadEvaluation {
        let club = self.club;
        let date = self.date;
        let current_window = self.current_window;
        let mid_season_window = self.mid_season_window;
        let home = self.home;
        let players = self.players;
        let rep_level = self.rep_level.clone();
        let rep_score = self.rep_score;
        let asset_ctx = &self.asset_ctx;
        let squad = self.squad.as_slice();
        let avg_ability = self.avg_ability;
        let formation_positions = self.formation_positions;
        let budget = self.budget;
        let max_concurrent = self.max_concurrent;
        let ReviewLedger {
            brief, requests, ..
        } = ledger;
        let mut loan_outs: Vec<LoanOutCandidate> = Vec::new();

        // ──────────────────────────────────────────────────────────
        // STEP 6: Identify loan-out candidates
        // ──────────────────────────────────────────────────────────

        PipelineProcessor::identify_loan_outs(
            club,
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
            PipelineProcessor::identify_position_glut(&squad, date, players, &mut loan_outs);

        // Repeated-loan stagnation sweep: a player already farmed out
        // twice (the loan path refuses a third spell) who still sits
        // below his position-group level is no longer a development
        // asset — sell rather than hold.
        PipelineProcessor::identify_repeated_loan_stagnation(
            &squad,
            players,
            &mut force_transfer_list,
        );

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
        PipelineProcessor::identify_stalled_prospects(
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
        let sell_list =
            PipelineProcessor::price_squad(club, players, &squad, &asset_ctx, &brief, date);

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
}
