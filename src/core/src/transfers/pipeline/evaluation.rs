use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

use crate::club::player::contract::{AffordabilityInput, ContractStalemate, StalemateLevel};
use crate::club::staff::perception::{EstimationContext, PotentialEstimator};
use crate::club::team::squad::{MIN_FIRST_TEAM_SQUAD, SquadAssetContext};
use crate::transfers::TransferWindowManager;
use crate::transfers::pipeline::processor::{PipelineProcessor, SquadPlayerInfo};
use crate::transfers::pipeline::{
    ClubTransferPlan, LoanOutCandidate, LoanOutReason, LoanOutStatus, TransferNeedPriority,
    TransferNeedReason, TransferRequest, TransferRequestStatus,
};
use crate::transfers::squad_needs::{EmergencyGroupSlot, FirstTeamSquadNeeds};
use crate::{
    Club, ClubPhilosophy, Country, MatchTacticType, Person, Player, PlayerFieldPositionGroup,
    PlayerPlanRole, PlayerPositionType, PlayerStatusType, ReputationLevel, TACTICS_POSITIONS,
    TacticsSelector,
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

        // (2) Quality upgrade — best player at this group below tier baseline.
        // Ambition-adjusted tolerance: a club's appetite for an upgrade
        // scales with its standing. The base `quality_tolerance` keeps small
        // clubs patient, but a high-reputation side shops for an upgrade when
        // its best is merely AT the tier standard rather than clearly below
        // it — a title contender strengthens an adequate XI instead of
        // settling for the divisional baseline. Without this, clubs only ever
        // shopped to patch a hole, never to get better, so an established
        // starter at another club was almost never a target. The budget split
        // per need and the seller-side premium still bound how many of these
        // actually complete.
        let baseline = PipelineProcessor::tier_starter_ca_score(rep_score, group);
        let upgrade_tolerance = quality_tolerance - (rep_score * 5.0).round() as i16;
        let best_in_group = squad
            .iter()
            .filter(|p| p.primary_position.position_group() == group)
            .map(|p| p.current_ability)
            .max()
            .unwrap_or(0);
        if (best_in_group as i16) < baseline as i16 - upgrade_tolerance {
            needs.push(GroupNeed {
                group,
                representative_pos,
                kind: NeedKind::QualityUpgrade,
            });
            continue;
        }

        // (3) Depth cover — group thinner than formation footprint
        let group_count = squad
            .iter()
            .filter(|p| p.primary_position.position_group() == group)
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

        // A third loan spell isn't realistic. If he didn't qualify for a
        // sale above, hold — no more loaning this one.
        if s.previous_loans >= 2 {
            return PathwayAction::Hold;
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
        let blocked = s.depth_rank >= Self::BLOCKED_MIN_RANK;

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
    // ============================================================
    // Step 2: Squad Evaluation - Coach-driven, formation-based
    // ============================================================

    pub fn evaluate_squads(country: &mut Country, date: NaiveDate) {
        let is_window_start = Self::is_window_start(date);
        let should_evaluate = is_window_start || Self::should_evaluate(date);
        let window_mgr = TransferWindowManager::for_country(country, date);
        let current_window = window_mgr.current_window_dates(country.id, date);

        // Pass 1: Collect evaluations (immutable reads)
        let mut evaluations: Vec<SquadEvaluation> = Vec::new();

        for club in &country.clubs {
            let needs_eval = should_evaluate
                || !club.transfer_plan.initialized
                || Self::all_shortlists_exhausted(&club.transfer_plan);

            if !needs_eval {
                continue;
            }

            let eval = Self::evaluate_single_club(club, date, current_window);
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

                // Track the highest request ID so re-evaluations don't create duplicate IDs
                if let Some(max_id) = eval.requests.iter().map(|r| r.id).max() {
                    plan.next_request_id = max_id + 1;
                }

                // Only add requests that don't duplicate an existing unfulfilled request
                // for the same position group. Without this, each weekly re-evaluation
                // adds duplicate requests (e.g. "need GK") that all get acted on independently,
                // causing clubs to loan 10+ players for the same position.
                let new_requests: Vec<_> = eval
                    .requests
                    .into_iter()
                    .filter(|new_req| {
                        !plan.transfer_requests.iter().any(|existing| {
                            existing.position.position_group() == new_req.position.position_group()
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
                                if !player.statuses.get().contains(&PlayerStatusType::Lst) {
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
                .filter_map(|p| {
                    // Check if player can play this position (exact or same group)
                    let level = p.position_levels.get(&formation_pos).copied().unwrap_or(0);
                    if level > 0 {
                        return Some((p.player_id, level, p.current_ability));
                    }
                    // Same position group with reduced effectiveness
                    if p.primary_position.position_group() == formation_pos.position_group() {
                        let reduced = (p.current_ability as u16 * 7 / 10) as u8;
                        return Some((p.player_id, 1, reduced));
                    }
                    None
                })
                .max_by_key(|&(_, level, ability)| (level as u16) * 10 + ability as u16);

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

        // Generate one request per group with priority and budget
        // appropriate to the kind of need.
        for need in &group_needs {
            let group = need.group;
            let baseline = Self::tier_starter_ca_score(rep_score, group);
            let (priority, reason, mult, min_ca, ideal_ca) = match need.kind {
                NeedKind::FormationGap => (
                    TransferNeedPriority::Critical,
                    TransferNeedReason::FormationGap,
                    1.5,
                    baseline.saturating_sub(12),
                    baseline,
                ),
                NeedKind::QualityUpgrade => (
                    TransferNeedPriority::Important,
                    TransferNeedReason::QualityUpgrade,
                    1.0,
                    baseline.saturating_sub(8),
                    baseline.saturating_add(5),
                ),
                NeedKind::DepthCover => (
                    TransferNeedPriority::Optional,
                    TransferNeedReason::DepthCover,
                    0.6,
                    baseline.saturating_sub(15),
                    baseline.saturating_sub(5),
                ),
            };

            let alloc = (budget_per_need * mult).min(available_budget - budget_used);
            if alloc <= 0.0 {
                break;
            }

            requests.push(TransferRequest::new(
                next_id,
                need.representative_pos,
                priority,
                reason,
                min_ca,
                ideal_ca,
                alloc,
            ));
            next_id += 1;
            budget_used += alloc;
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
                if player_info.age >= 30
                    && player_info.current_ability >= avg_ability
                    && budget_used < available_budget
                {
                    let alloc = (budget_per_need * 0.4).min(available_budget - budget_used);
                    if alloc <= 0.0 {
                        break;
                    }

                    // Only if we don't already have a request for this position
                    if !requests
                        .iter()
                        .any(|r| r.position == player_info.primary_position)
                    {
                        requests.push(TransferRequest::new(
                            next_id,
                            player_info.primary_position,
                            TransferNeedPriority::Optional,
                            TransferNeedReason::SuccessionPlanning,
                            avg_ability.saturating_sub(10),
                            avg_ability,
                            alloc,
                        ));
                        next_id += 1;
                        budget_used += alloc;
                    }
                }
            }
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
                    let alloc = (budget_per_need * 0.3).min(available_budget - budget_used);
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
            // goalkeepers at all. First-team count (not youth squads) is used
            // on purpose: the club's existing U18 academy keeper is a separate,
            // earlier pipeline stage and must not suppress the first-team
            // succession project.
            let young_keepers = squad
                .iter()
                .filter(|p| {
                    p.primary_position.position_group() == PlayerFieldPositionGroup::Goalkeeper
                        && p.age <= youth_age_max
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
                let alloc = (budget_per_need * 0.3).min(available_budget - budget_used);
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
                    // Padding allocates `budget_per_need * 0.3` so a
                    // small club still leaves headroom for upgrades.
                    // Free-agent fee is zero, but the same request
                    // can also be filled by a cheap paid signing —
                    // when budget is exhausted we still emit the
                    // request with zero allocation so the FA matcher
                    // and emergency pass can both react.
                    let alloc =
                        (budget_per_need * 0.3).min((available_budget - budget_used).max(0.0));
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
                let alloc = (budget_per_need * 0.3).min(available_budget - budget_used);
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
                let alloc = (budget_per_need * 0.3).min(available_budget - budget_used);
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
        );

        SquadEvaluation {
            club_id: club.id,
            requests,
            loan_outs,
            force_transfer_list,
            total_budget: budget,
            max_concurrent,
        }
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
                let statuses = player.statuses.get();
                let already_listed = statuses.contains(&PlayerStatusType::Lst);

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
                && !player.statuses.get().contains(&PlayerStatusType::Lst)
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
    ) {
        let is_january = Self::is_january_window(date);

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
            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Lst)
                || statuses.contains(&PlayerStatusType::Loa)
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

    /// Loan spells that returned with negligible official minutes — the
    /// development the loan was supposed to provide never materialised.
    fn failed_loan_count(player: &Player) -> u8 {
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
        player.statuses.get().iter().any(|s| {
            matches!(
                s,
                PlayerStatusType::Inj
                    | PlayerStatusType::Sus
                    | PlayerStatusType::Int
                    | PlayerStatusType::IntU21
                    | PlayerStatusType::Wp
                    | PlayerStatusType::Unr
                    | PlayerStatusType::Lmp
                    | PlayerStatusType::Unf
            )
        })
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
    /// "too thin to let anyone leave". (Forward intentionally has no +1:
    /// lone-striker shapes already carry the wide forwards counted here.)
    fn group_min_needed(
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
            PlayerFieldPositionGroup::Forward => formation_positions
                .iter()
                .filter(|p| p.is_forward())
                .count(),
        }
    }

    /// Identify loan-out candidates based on club reputation tier.
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
    ) {
        let is_january = Self::is_january_window(date);

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

        let mut players = vec![Fx::player(1, PlayerPositionType::MidfielderCenter, 110, 19, 0)];
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
                players.push(Self::player(10 + i, PlayerPositionType::DefenderCenter, SENIOR, 27));
            }
            players.push(Self::player(20, PlayerPositionType::DefenderCenter, YOUNG, 19));
            players.push(Self::player(21, PlayerPositionType::DefenderCenter, YOUNG, 19));
            // MID: 2 senior + 2 young = 4 (< depth req 5 → DepthCover seed).
            players.push(Self::player(30, PlayerPositionType::MidfielderCenter, SENIOR, 27));
            players.push(Self::player(31, PlayerPositionType::MidfielderCenter, SENIOR, 27));
            players.push(Self::player(32, PlayerPositionType::MidfielderCenter, YOUNG, 19));
            players.push(Self::player(33, PlayerPositionType::MidfielderCenter, YOUNG, 19));
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
    }

    #[test]
    fn youth_minded_club_signs_a_goalkeeper_prospect() {
        let club = GkFx::continental_club();
        // Precondition: a prospect-developing tier (Continental → Balanced).
        assert_eq!(
            club.teams.teams[0].reputation.level(),
            ReputationLevel::Continental
        );

        let eval = PipelineProcessor::evaluate_single_club(&club, GkFx::date(), None);

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

        let eval = PipelineProcessor::evaluate_single_club(&club, GkFx::date(), None);
        assert!(
            GkFx::gk_development_request(&eval).is_none(),
            "a club that already has a young keeper must not sign another keeper prospect"
        );
    }
}
