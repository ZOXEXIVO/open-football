//! Discovery: who the club knows about, and how it came to know.
//!
//! The scout-assignment and report passes live in this file. [`watchlist`]
//! is the standing board of names, [`watch`] and [`breakout`] are the
//! year-round form sweep, [`exposure`] is who can see whom, [`desk`] is
//! how a club hires the scout that covers a market it does not work, and
//! [`recruitment`] is the department that meets and votes.

mod assignment;
pub mod breakout;
pub mod config;
pub mod desk;
pub mod exposure;
pub mod judgement;
pub mod recruitment;
pub mod watch;
pub mod watchlist;

pub use desk::*;
pub use recruitment::*;
pub use watchlist::MarketKnowledge;

use crate::club::team::squad::SquadEvidenceContext;
use crate::transfers::scouting::assignment::ClubScan;
use crate::transfers::scouting::judgement::ScoutJudgement;
use crate::transfers::view::club::ClubView;
use crate::transfers::view::player::PlayerView;
use chrono::{Datelike, NaiveDate};
use log::debug;

use crate::club::player::events::transfer_social::TransferInterestSignal;
use crate::club::player::language::{Language, LanguageProfile};
use crate::club::player::mind::GoalKind;
use crate::club::player::statistics::StuckCareerScan;
use crate::utils::PerformanceProfiler;
use crate::transfers::ScoutingRegion;
use crate::transfers::gate::build::{BuyerPlausibilityContext, TransferPlausibilityBuilder};
use crate::transfers::gate::{SquadEvidenceSource, TransferMoveStage, TransferPlausibilityVerdict};
use crate::transfers::loan::home::HomeLoanGates;
use crate::transfers::loan::interest::InterestDraw;
use crate::transfers::pipeline::processor::{PlayerSummary, SellerPlausibilityContext};
use crate::transfers::pipeline::{
    ClubTransferPlan, DetailedScoutingReport, LoanDestinationPreference, PlayerObservation,
    ReportRiskFlag, ScoutMatchAssignment, ScoutingAssignment, ScoutingRecommendation,
    TransferNeedPriority, TransferRequest, TransferRequestStatus,
};
use crate::transfers::scouting::breakout::LeaguePerformanceLookup;
use crate::transfers::scouting::config::{RealismTarget, ScoutingConfig};
use crate::transfers::squad::standing::CareerRecordSnapshot;
use crate::transfers::value::PlayerValuationCalculator;
use crate::transfers::view::player::ClubGroupRanks;
use crate::transfers::{
    ClubMarketKnowledge, ClubMarketLedger, MarketAffinity, MarketAffinityInputs, MarketMap,
    MoveKind,
};
use crate::utils::IntegerUtils;
use crate::{
    Club, ClubPhilosophy, Country, Person, PlayerFieldPositionGroup, PlayerSquadStatus,
    PlayerStatusType, PositionCoverage, StaffEventType, StaffPosition, TeamType,
    TransferInterestSource, TransferInterestStage,
};
use crate::{Player, Team};
use chrono::Weekday;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

struct ScoutAssignmentAction {
    club_id: u32,
    assignment: ScoutingAssignment,
    request_id: u32,
}

struct ScoutingObservationResult {
    club_id: u32,
    assignment_id: u32,
    player_id: u32,
    assessed_ability: u8,
    assessed_potential: u8,
    is_new: bool,
}

struct ScoutingReportResult {
    club_id: u32,
    report: DetailedScoutingReport,
    assignment_id: u32,
}

struct MatchScoutAssignmentAction {
    club_id: u32,
    assignment: ScoutMatchAssignment,
}

struct MatchScoutingObservationResult {
    club_id: u32,
    assignment_id: u32,
    player_id: u32,
    assessed_ability: u8,
    assessed_potential: u8,
    match_rating: f32,
    is_new: bool,
}

/// Per-club staged output of the parallel scouting scan (pass 1 of
/// `process_scouting`). Merged in club order and applied by pass 2, so
/// the apply order and dedup semantics match the old serial scan.
/// One club's memo of how far its market reaches, per `(passport, league
/// he plays in)`.
///
/// The answer depends on exactly that pair and on nothing else about the
/// player, and a scouting pass scores thousands of candidates against a few
/// hundred pairs — computing it per candidate is the difference between a
/// run that finishes and one that does not (memory
/// `transfer_geography_implementation_2026_09`).
///
/// Interior mutability because the pass reads it from a `Fn` filter closure
/// that is used twice by value: a `&mut` capture would make the closure
/// `FnMut` and non-`Copy`, and the domestic and foreign sweeps both need it.
/// There is no reentrancy — every borrow is taken and dropped inside
/// [`Self::reach`].
struct MarketReachCache {
    entries: std::cell::RefCell<HashMap<(u32, u32), f32>>,
}

impl MarketReachCache {
    /// Floor under the knowledge term. A club may LOOK at a market it does
    /// not work — what it cannot do is see that market as clearly as one it
    /// has people in.
    const KNOWLEDGE_FLOOR: f32 = 0.4;

    fn new() -> Self {
        MarketReachCache {
            entries: std::cell::RefCell::new(HashMap::new()),
        }
    }

    /// How well this club is placed to look at ONE foreign player: the
    /// corridor into his market times what the club knows of it, 0..1.
    ///
    /// Keyed on `(nationality, country he plays in)`. Keying on the country
    /// he plays in ALONE — the first cut — collides two different players: a
    /// Brazilian at Porto and a Portuguese at Porto share one entry, and
    /// whichever was scored first decides the reach for both.
    /// `FreeAgentMarketVisibility::build` already keys on the pair; this is
    /// the same key.
    fn reach(
        &self,
        market_map: &MarketMap,
        buyer_country_id: u32,
        club: &Club,
        player: &PlayerSummary,
        date: NaiveDate,
        best_scout_country_level: &dyn Fn(u32) -> u8,
    ) -> f32 {
        self.reach_for_pair(
            market_map,
            buyer_country_id,
            &club.market_ledger,
            (player.nationality_country_id, player.country_id),
            date,
            best_scout_country_level,
        )
    }

    /// The same read, addressed by the pair directly. Separated so the key
    /// is a thing a test can hold: the defect this cache carried was that
    /// the key was the LEAGUE alone, and no test could see that without
    /// building two whole player summaries.
    fn reach_for_pair(
        &self,
        market_map: &MarketMap,
        buyer_country_id: u32,
        ledger: &ClubMarketLedger,
        key: (u32, u32),
        date: NaiveDate,
        best_scout_country_level: &dyn Fn(u32) -> u8,
    ) -> f32 {
        if market_map.is_silent() {
            return 1.0;
        }
        if let Some(cached) = self.entries.borrow().get(&key) {
            return *cached;
        }
        let affinity = MarketAffinity::affinity(
            market_map,
            MarketAffinityInputs {
                buyer_country_id,
                nationality_country_id: key.0,
                current_country_id: key.1,
                kind: MoveKind::Talent,
                // Discovery is the club's own scouting reach; owner money
                // buys a name, it does not find one.
                benefactor: 0.0,
            },
        );
        let knowledge = ClubMarketKnowledge::knowledge(
            market_map,
            buyer_country_id,
            ledger,
            best_scout_country_level(key.1).max(best_scout_country_level(key.0)),
            key.1,
            date,
        );
        let reach = (affinity * knowledge.max(Self::KNOWLEDGE_FLOOR)).clamp(0.0, 1.0);
        self.entries.borrow_mut().insert(key, reach);
        reach
    }
}

struct ClubScoutingStaged {
    observations: Vec<ScoutingObservationResult>,
    reports: Vec<ScoutingReportResult>,
    staff_events: Vec<(u32, u32, StaffEventType)>,
    /// `(club, scout, region, source country)` — a day spent watching a
    /// foreign player. Both axes accrue: the coarse region the older gates
    /// read, and the COUNTRY, which is the unit knowledge is actually
    /// measured in.
    familiarity_events: Vec<(u32, u32, ScoutingRegion, u32)>,
    rejected_events: Vec<(u32, u32)>,
    /// `(scouting_club_id, player_id)` — the club identity travels with
    /// the target so the apply pass can emit the ScoutWatched beat with
    /// the interested club attached, not just stamp an anonymous `Wnt`.
    wanted_targets: Vec<(u32, u32)>,
    monitoring_updates: Vec<MonitoringUpdate>,
}

/// Rich update payload for an active monitoring row. Built during the
/// immutable read pass of `process_scouting` / `process_match_scouting`
/// and applied during pass 2 against the mutable `ClubTransferPlan`.
struct MonitoringUpdate {
    club_id: u32,
    scout_staff_id: u32,
    player_id: u32,
    source: ScoutMonitoringSource,
    transfer_request_id: Option<u32>,
    origin_assignment_id: Option<u32>,
    assessed_ability: u8,
    assessed_potential: u8,
    confidence: f32,
    role_fit: f32,
    estimated_value: f64,
    risk_flags: Vec<ReportRiskFlag>,
    is_match: bool,
    region: Option<ScoutingRegion>,
}

/// Writes a read-pass monitoring payload back onto a club's plan.
struct MonitoringWriter;

impl MonitoringWriter {
    /// Apply a monitoring update against a `ClubTransferPlan`. Either
    /// upserts an existing row for `(scout_staff_id, player_id)` or creates
    /// a fresh one. Pure-state mutation — no side effects beyond the plan.
    fn apply(plan: &mut ClubTransferPlan, update: MonitoringUpdate, date: NaiveDate) {
        // The upsert itself lives on `ClubTransferPlan` so the match-result
        // showcase path (`LeagueResult::record_domestic_cup_showcase_scouting`)
        // shares the exact same row lifecycle — this wrapper just unpacks the
        // pipeline's read-pass payload.
        plan.upsert_monitoring(
            update.scout_staff_id,
            update.player_id,
            update.source,
            update.transfer_request_id,
            update.origin_assignment_id,
            update.region,
            update.assessed_ability,
            update.assessed_potential,
            update.confidence,
            update.role_fit,
            update.estimated_value,
            update.risk_flags,
            date,
            update.is_match,
        );
    }
}

/// What one country's scouts saw at the weekend's fixtures. The read pass walks
/// every club while deciding, so nothing can be written until it is done —
/// which is what the `Pass 1` / `Pass 2` banners inside the old 399-line body
/// were describing.

/// One fixture a scout was sent to, and the side he was sent to watch.

/// A player a scout actually got to look at tonight, and the standing
/// assignment that sent him.
struct PlayerSeen<'a> {
    player: &'a Player,
    assignment: &'a ScoutingAssignment,
    age: u8,
    /// The REGRESSED season average, never the raw one.
    match_rating: f32,
}

struct MatchWatch<'a> {
    assignment: &'a ScoutMatchAssignment,
    target_club: &'a Club,
    target_team: &'a Team,
    /// How good this scout's eye is — the spread on every read below.
    judging_ability: u8,
    judging_potential: u8,
}

struct MatchScoutingStaged {
    observations: Vec<MatchScoutingObservationResult>,
    reports: Vec<ScoutingReportResult>,
    attended_updates: Vec<(u32, u32, NaiveDate)>,
    staff_events: Vec<(u32, u32, StaffEventType)>,
    monitoring_updates: Vec<MonitoringUpdate>,
}

/// Everything a summary reads that is NOT the player: the country's constants
/// and the club's, resolved once per club so a cross-country buyer can assess
/// any of its players without re-walking the seller's roster.
struct PoolContext<'c> {
    country: &'c Country,
    club: &'c Club,
    date: NaiveDate,
    price_level: f32,
    country_id: u32,
    country_reputation: u16,
    country_region: ScoutingRegion,
    seller_league_rep: u16,
    seller_club_rep: u16,
    club_world_rep: i16,
    seller_club_rep_score: f32,
    seller_league_id: Option<u32>,
    seller_in_debt: bool,
    seller_club_matches: u16,
    group_ranks: ClubGroupRanks,
    home_preferences: Vec<u32>,
}

impl<'c> PoolContext<'c> {
    fn of(
        country: &'c Country,
        club: &'c Club,
        date: NaiveDate,
        price_level: f32,
        country_id: u32,
        country_reputation: u16,
        country_region: ScoutingRegion,
    ) -> Self {
        // Seller market context once per club — flat 0/0 used to
        // drag every domestic player to the same baseline regardless
        // of the league/club they actually played for.
        let (seller_league_rep, seller_club_rep) =
            PlayerValuationCalculator::seller_context(country, club);
        let club_world_rep = ClubView::club_world_reputation(club);
        // Staged-plausibility seller context, resolved once per club so a
        // cross-country buyer can assess this player without re-walking
        // the seller's roster. `overall_score` (not market value) matches
        // the legacy single-country builder's `seller_rep`.
        let main_team = club.teams.main();
        let seller_club_rep_score = main_team
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.3);
        let seller_league_id = main_team.and_then(|t| t.league_id);
        let seller_in_debt = club.finance.balance.balance < 0;
        // Club match count, so a foreign buyer reads the same
        // "is this season readable yet" signal a domestic one does.
        let seller_club_matches =
            SquadEvidenceContext::current_season_sample(date, club).club_matches_proxy();
        // One sorted-group snapshot per club replaces the per-player
        // rank/best re-sorts (same values, O(squad·log) once).
        let group_ranks = ClubGroupRanks::build(club);
        // Loan-out candidates the parent has already decided WHERE it
        // would send — the `UnsettledAbroad` read that names a home
        // country or a home region. That decision is itself a posting
        // (C5), so a candidate identified this tick reaches the world
        // in the same pass rather than a window later.
        let home_preferences: Vec<u32> = club
            .transfer_plan
            .loan_out_candidates
            .iter()
            .filter(|c| c.preferred_destination != LoanDestinationPreference::Any)
            .map(|c| c.player_id)
            .collect();

        Self {
            country,
            club,
            date,
            price_level,
            country_id,
            country_reputation,
            country_region,
            seller_league_rep,
            seller_club_rep,
            club_world_rep,
            seller_club_rep_score,
            seller_league_id,
            seller_in_debt,
            seller_club_matches,
            group_ranks,
            home_preferences,
        }
    }
}

/// The scouting tick: assignments, observations, match reports and the shadow board.
pub struct ScoutingPass;

impl ScoutingPass {
    /// Contract months at or below which a player is publicly on his way
    /// out — clubs abroad watch expiring contracts regardless of which
    /// league he currently plays in.
    const OPENLY_AVAILABLE_EXPIRY_MONTHS: i16 = 12;

    /// The player's own club has advertised him, or he has advertised
    /// himself, or his deal is running out. Any of these is public
    /// knowledge that travels past the normal direction of the market, so
    /// it lifts the country step-down on the foreign candidate pool.
    /// Being merely unhappy does not: that is a mood, not an advert.
    fn is_openly_available(candidate: &PlayerSummary) -> bool {
        candidate.is_listed
            || candidate.is_loan_listed
            || candidate.seller_ctx.is_transfer_requested
            || (candidate.contract_months_remaining > 0
                && candidate.contract_months_remaining <= Self::OPENLY_AVAILABLE_EXPIRY_MONTHS)
    }

    pub fn assign_scouts(country: &mut Country, _date: NaiveDate) {
        let mut actions: Vec<ScoutAssignmentAction> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            // Only assignments still doing work reserve their request. A
            // completed one has filed its report and stopped scouting, so
            // counting it as "covered" is what stranded requests: an
            // assignment closes after its FIRST report, and if that report
            // produced no shortlist the request sat in `ScoutingActive`
            // with a dead assignment attached and no way to earn another —
            // the club had an open need nobody was looking for.
            let assigned_request_ids: Vec<u32> = plan
                .scouting_assignments
                .iter()
                .filter(|a| !a.completed)
                .map(|a| a.transfer_request_id)
                .collect();

            let shortlisted_request_ids: Vec<u32> = plan
                .shortlists
                .iter()
                .map(|s| s.transfer_request_id)
                .collect();

            let pending_requests: Vec<&TransferRequest> = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    // A request whose scouting round produced nothing is
                    // re-scouted; one that already has a shortlist to work
                    // through is not.
                    matches!(
                        r.status,
                        TransferRequestStatus::Pending | TransferRequestStatus::ScoutingActive
                    ) && !assigned_request_ids.contains(&r.id)
                        && !shortlisted_request_ids.contains(&r.id)
                        // Emergency depth requests are free-agent-only:
                        // zero budget, no scouting intent. Assigning a
                        // scout would pull them into the paid pipeline.
                        && !r.is_emergency_free_agent_depth()
                })
                .collect();

            if pending_requests.is_empty() {
                continue;
            }

            if club.teams.teams.is_empty() {
                continue;
            }
            let resolved = club.teams.teams[0].staffs.resolve_for_transfers();

            let mut sorted_requests = pending_requests;
            sorted_requests.sort_by(|a, b| {
                let priority_order = |p: &TransferNeedPriority| match p {
                    TransferNeedPriority::Critical => 0,
                    TransferNeedPriority::Important => 1,
                    TransferNeedPriority::Optional => 2,
                };
                priority_order(&a.priority).cmp(&priority_order(&b.priority))
            });

            let mut scout_idx = 0;
            let next_assign_id = plan.next_assignment_id;

            for (i, request) in sorted_requests.iter().enumerate() {
                let scout_id = if !resolved.scouts.is_empty() {
                    let s = resolved.scouts[scout_idx % resolved.scouts.len()];
                    scout_idx += 1;
                    Some(s.id)
                } else {
                    None
                };

                let assignment = ScoutingAssignment::new(
                    next_assign_id + i as u32,
                    request.id,
                    scout_id,
                    request.position.clone(),
                    request.min_ability,
                    request.preferred_age_min,
                    request.preferred_age_max,
                    request.budget_allocation,
                );

                actions.push(ScoutAssignmentAction {
                    club_id: club.id,
                    assignment,
                    request_id: request.id,
                });
            }
        }

        let mut seeded_clubs: Vec<u32> = Vec::new();
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let plan = &mut club.transfer_plan;

                if let Some(req) = plan
                    .transfer_requests
                    .iter_mut()
                    .find(|r| r.id == action.request_id)
                {
                    req.status = TransferRequestStatus::ScoutingActive;
                }

                plan.next_assignment_id = action.assignment.id + 1;
                plan.scouting_assignments.push(action.assignment);
                if !seeded_clubs.contains(&club.id) {
                    seeded_clubs.push(club.id);
                }
            }
        }

        // After fresh assignments exist, seed any matching shadow reports
        // into the active window — saves clubs from cold-starting each window.
        for club_id in seeded_clubs {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                club.transfer_plan.seed_active_reports_from_shadow();
            }
        }
    }

    // ============================================================
    // Step 3.5: Assign Scouts to Youth/Reserve Matches
    // ============================================================

    pub fn assign_scouts_to_matches(country: &mut Country, current_date: NaiveDate) {
        let mut actions: Vec<MatchScoutAssignmentAction> = Vec::new();

        // Pass 1: Immutable reads - determine which scouts to assign where
        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            // Get active scouting assignments to know what positions/ages we're looking for
            let active_assignments: Vec<&ScoutingAssignment> = plan
                .scouting_assignments
                .iter()
                .filter(|a| !a.completed)
                .collect();

            if active_assignments.is_empty() {
                continue;
            }

            if club.teams.teams.is_empty() {
                continue;
            }

            let resolved = club.teams.teams[0].staffs.resolve_for_transfers();
            if resolved.scouts.is_empty() {
                continue;
            }

            // Check existing match assignments - don't re-assign scouts already watching a team
            let already_assigned_scout_ids: Vec<u32> = plan
                .scout_match_assignments
                .iter()
                .filter(|a| {
                    a.last_attended
                        .map(|d| (current_date - d).num_days() < 7)
                        .unwrap_or(false)
                })
                .map(|a| a.scout_staff_id)
                .collect();

            let available_scouts: Vec<u32> = resolved
                .scouts
                .iter()
                .map(|s| s.id)
                .filter(|id| !already_assigned_scout_ids.contains(id))
                .collect();

            if available_scouts.is_empty() {
                continue;
            }

            let max_assignments = available_scouts.len().min(
                ScoutingConfig::default()
                    .assignment
                    .max_match_assignments_per_club,
            );

            // Score each youth/reserve team from other clubs by how many matching players it has
            let mut team_scores: Vec<(u32, u32, u32, usize)> = Vec::new(); // (team_id, club_id, tiebreak, score)

            for other_club in &country.clubs {
                if other_club.id == club.id {
                    continue;
                }

                for team in &other_club.teams.teams {
                    // Only consider non-Main teams
                    if matches!(team.team_type, TeamType::Main) {
                        continue;
                    }

                    // Skip teams already being watched (within 7 days)
                    let already_watching = plan.scout_match_assignments.iter().any(|a| {
                        a.target_team_id == team.id
                            && a.last_attended
                                .map(|d| (current_date - d).num_days() < 7)
                                .unwrap_or(false)
                    });
                    if already_watching {
                        continue;
                    }

                    // Score: count how many players match any active scouting assignment criteria
                    let mut score = 0usize;
                    for player in &team.players.players {
                        let player_pos_group = player.position().position_group();
                        let player_age = player.age(current_date);

                        for assignment in &active_assignments {
                            let target_group = assignment.target_position.position_group();
                            if player_pos_group == target_group
                                && player_age >= assignment.preferred_age_min
                                && player_age <= assignment.preferred_age_max
                            {
                                score += 1;
                                break;
                            }
                        }
                    }

                    if score > 0 {
                        team_scores.push((team.id, other_club.id, 0, score));
                    }
                }
            }

            // Sort by score descending. The score is a small integer, so ties
            // are the common case — and a stable sort resolved every one of
            // them by club registration order, sending the scouts to the same
            // fixtures week after week. A per-pass tiebreak, drawn once per row
            // so the comparator stays a valid total order, rotates which of the
            // equally-worthwhile matches actually get watched.
            for row in team_scores.iter_mut() {
                row.2 = IntegerUtils::random(0, 10_000) as u32;
            }
            team_scores.sort_by(|a, b| b.3.cmp(&a.3).then(b.2.cmp(&a.2)));

            // Assign scouts to the best-scoring teams
            let assignments_to_make = team_scores.len().min(max_assignments);
            for i in 0..assignments_to_make {
                let (target_team_id, target_club_id, _, _) = team_scores[i];
                let scout_id = available_scouts[i];

                // Link to relevant scouting assignment IDs
                let linked_ids: Vec<u32> = active_assignments.iter().map(|a| a.id).collect();

                actions.push(MatchScoutAssignmentAction {
                    club_id: club.id,
                    assignment: ScoutMatchAssignment {
                        scout_staff_id: scout_id,
                        target_team_id,
                        target_club_id,
                        linked_assignment_ids: linked_ids,
                        last_attended: None,
                    },
                });
            }
        }

        // Pass 2: Apply assignments
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                // Check if we already have an assignment for this team, update it
                if let Some(existing) = club
                    .transfer_plan
                    .scout_match_assignments
                    .iter_mut()
                    .find(|a| a.target_team_id == action.assignment.target_team_id)
                {
                    existing.scout_staff_id = action.assignment.scout_staff_id;
                    existing.linked_assignment_ids = action.assignment.linked_assignment_ids;
                } else {
                    club.transfer_plan
                        .scout_match_assignments
                        .push(action.assignment);
                }
            }
        }

        debug!("assign_scouts_to_matches: completed scout-to-match assignments");
    }

    // ============================================================
    // Step 3.75: Process Match-Day Scouting Observations
    // ============================================================

    pub fn process_match_scouting(country: &mut Country, current_date: NaiveDate) {
        let staged = Self::observe_matches(country, current_date);
        Self::apply_match_scouting(country, current_date, staged);
        debug!("process_match_scouting: completed match-day observations");
    }

    /// Pass 1 — what the scouts actually saw. Read-only.
    fn observe_matches(country: &Country, current_date: NaiveDate) -> MatchScoutingStaged {
        let mut staged = MatchScoutingStaged {
            observations: Vec::new(),
            reports: Vec::new(),
            attended_updates: Vec::new(),
            staff_events: Vec::new(),
            monitoring_updates: Vec::new(),
        };

        // Pass 1: Immutable reads
        for club in &country.clubs {
            let plan = &club.transfer_plan;

            for match_assignment in &plan.scout_match_assignments {
                // Find the target club + team. The selling club's market
                // context drives the estimated_value attached to the
                // scouting report (a player at Real Madrid is worth more
                // than the same skill set at a Maltese club).
                let target_club = country
                    .clubs
                    .iter()
                    .find(|c| c.id == match_assignment.target_club_id);
                let target_team =
                    target_club.and_then(|c| c.teams.find(match_assignment.target_team_id));

                let (target_club, target_team) = match (target_club, target_team) {
                    (Some(c), Some(t)) => (c, t),
                    _ => continue,
                };

                // Check if this team played today
                let played_today = target_team
                    .match_history
                    .items()
                    .last()
                    .map(|m| m.date.date() == current_date)
                    .unwrap_or(false);

                if !played_today {
                    continue;
                }

                // Get scout skills
                let (judging_ability, judging_potential) =
                    ClubView::get_scout_skills(club, match_assignment.scout_staff_id);

                // Mark attendance
                staged.attended_updates.push((
                    club.id,
                    match_assignment.target_team_id,
                    current_date,
                ));
                staged.staff_events.push((
                    club.id,
                    match_assignment.scout_staff_id,
                    StaffEventType::MatchObserved,
                ));

                Self::observe_target_team(
                    country,
                    club,
                    &MatchWatch {
                        assignment: match_assignment,
                        target_club,
                        target_team,
                        judging_ability,
                        judging_potential,
                    },
                    current_date,
                    &mut staged,
                );
            }
        }

        staged
    }

    /// Every player on the team a scout went to watch, as he read them. Rating
    /// is the regressed season average, never the raw one — the raw value let a
    /// one-cap teenager on 8.2 trigger a StrongBuy.
    fn observe_target_team(
        country: &Country,
        club: &Club,
        watch: &MatchWatch<'_>,
        current_date: NaiveDate,
        staged: &mut MatchScoutingStaged,
    ) {
        // Observe all players on the target team
        for player in &watch.target_team.players.players {
            Self::observe_player(country, club, watch, player, current_date, staged);
        }
    }

    /// One player, as this scout read him tonight.
    fn observe_player(
        country: &Country,
        club: &Club,
        watch: &MatchWatch<'_>,
        player: &Player,
        current_date: NaiveDate,
        staged: &mut MatchScoutingStaged,
    ) {
        let config = ScoutingConfig::default();
        let plan = &club.transfer_plan;
        let match_assignment = watch.assignment;
        let target_club = watch.target_club;
        let target_team = watch.target_team;

        let player_pos_group = player.position().position_group();
        let player_age = player.age(current_date);
        // Scout uses the regressed season average to assess
        // the player. The raw value would let a one-cap teen
        // with an 8.2 trigger a StrongBuy recommendation;
        // the regression keeps recommendation tiers anchored
        // to a meaningful sample.
        let match_rating = player.statistics.average_rating_realistic(player_pos_group);

        // Check if this player matches any linked scouting assignment
        let matching_assignment = plan.scouting_assignments.iter().find(|a| {
            !a.completed
                && match_assignment.linked_assignment_ids.contains(&a.id)
                && a.target_position.position_group() == player_pos_group
                && player_age >= a.preferred_age_min
                && player_age <= a.preferred_age_max
        });

        let assignment = match matching_assignment {
            Some(a) => a,
            None => return,
        };

        // Realism gate — the same policy the pool path applies via
        // `is_target_realistic`. A scout at the match still sees
        // everyone, but we don't open persistent monitoring on a
        // player this club could never realistically sign (e.g. a
        // much smaller side tracking a giant's first-choice keeper).
        // Without this the match route bypasses the reputation band
        // entirely and re-surfaces the very monitoring the pool
        // gate blocks.
        let buyer_world_rep = ClubView::club_world_reputation(club);
        let seller_league_rep = target_team
            .league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        let (target_contract_months, target_salary) = player
            .contract
            .as_ref()
            .map(|c| {
                let days = (c.expiration - current_date).num_days().max(0);
                ((days / 30).min(i16::MAX as i64) as i16, c.salary)
            })
            .unwrap_or((0, 0));
        let realism_target = RealismTarget {
            club_world_reputation: ClubView::club_world_reputation(target_club),
            world_reputation: player.player_attributes.world_reputation,
            current_reputation: player.player_attributes.current_reputation,
            home_reputation: player.player_attributes.home_reputation,
            appearances: player.statistics.total_games(),
            age: player_age,
            contract_months_remaining: target_contract_months,
            salary: target_salary,
            estimated_value: player.value(
                current_date,
                seller_league_rep,
                target_team.reputation.market_value_score(),
            ),
            is_listed: player.statuses.has(PlayerStatusType::Lst),
            is_loan_listed: player.statuses.has(PlayerStatusType::Loa),
            squad_status: player
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::NotYetSet),
            days_on_market: player.days_available(current_date).min(i16::MAX as i64) as i16,
        };
        // Real fee headroom (transfer budget × the negotiation
        // fee-gate multiplier), so a funded club can scout up to
        // what it can actually spend, not just its reputation tier.
        let buyer_fee_capacity = club
            .finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount * 1.40)
            .unwrap_or(0.0);
        if !config.is_target_realistic_fields(buyer_world_rep, &realism_target, buyer_fee_capacity)
        {
            return;
        }

        Self::record_observation(
            country,
            club,
            watch,
            &PlayerSeen {
                player,
                assignment,
                age: player_age,
                match_rating,
            },
            current_date,
            staged,
        );
    }

    /// What the scout writes down, once the realism gate has let this player
    /// through. Assessment is from visible skills and match performance —
    /// never hidden potential — with the error narrowed for a live viewing.
    fn record_observation(
        country: &Country,
        club: &Club,
        watch: &MatchWatch<'_>,
        seen: &PlayerSeen<'_>,
        current_date: NaiveDate,
        staged: &mut MatchScoutingStaged,
    ) {
        let config = ScoutingConfig::default();
        let match_assignment = watch.assignment;
        let target_club = watch.target_club;
        let judging_ability = watch.judging_ability;
        let judging_potential = watch.judging_potential;
        let player = seen.player;
        let assignment = seen.assignment;

        let player_age = seen.age;
        let match_rating = seen.match_rating;
        let observations = &mut staged.observations;
        let reports = &mut staged.reports;
        let monitoring_updates = &mut staged.monitoring_updates;

        let existing_obs = assignment
            .observations
            .iter()
            .find(|o| o.player_id == player.id);
        let obs_count = existing_obs.map(|o| o.observation_count).unwrap_or(0);

        // Match-context observations enjoy reduced error (the
        // scout sees the player live for 90 minutes vs a
        // snapshot from a database).
        let ability_error = config.effective_error(
            judging_ability,
            obs_count as u8,
            config.region.domestic_penalty,
            true,
        );
        let potential_error = config.effective_error(
            judging_potential,
            obs_count as u8,
            config.region.domestic_penalty,
            true,
        );

        // Assess from visible skills and match performance, not hidden CA/PA
        let skill_ability = player
            .skills
            .calculate_ability_for_position(player.position());
        let match_bonus = config.match_rating_bonus(match_rating);

        let assessed_ability = (skill_ability as i32
            + match_bonus
            + IntegerUtils::random(-ability_error, ability_error))
        .clamp(1, 200) as u8;

        let growth_potential = ScoutJudgement::estimate_growth_potential(
            player_age,
            player.skills.mental.determination,
            player.skills.mental.work_rate,
            player.skills.mental.composure,
            player.skills.mental.anticipation,
            skill_ability,
        );
        let assessed_potential = (skill_ability as i32
            + growth_potential as i32
            + IntegerUtils::random(-potential_error, potential_error))
        .clamp(1, 200) as u8;

        let is_new = !assignment.has_observation_for(player.id);

        observations.push(MatchScoutingObservationResult {
            club_id: club.id,
            assignment_id: assignment.id,
            player_id: player.id,
            assessed_ability,
            assessed_potential,
            match_rating,
            is_new,
        });

        let final_obs_count = obs_count + 1;
        if final_obs_count >= config.assignment.match_report_threshold as u32 {
            let confidence = config.match_report_confidence(final_obs_count as u8);

            // Match rating influences recommendation tier:
            // a hot match boosts a borderline player into StrongBuy
            // territory, a poor match drops them.
            let rec_cfg = &config.recommendation;
            let rating_boost = match_rating > rec_cfg.match_rating_good;
            let rating_penalty = match_rating < rec_cfg.match_rating_poor_max;

            let recommendation = if rating_penalty {
                if assessed_ability >= assignment.min_ability {
                    ScoutingRecommendation::Consider
                } else {
                    ScoutingRecommendation::Pass
                }
            } else if rating_boost
                && assessed_ability as i16
                    >= assignment.min_ability as i16 + rec_cfg.stats_tier1_bonus
                && assessed_potential > assessed_ability
            {
                ScoutingRecommendation::StrongBuy
            } else {
                // Fall through to the standard recommendation tiers,
                // bypassing the youth/stats bonuses (they would
                // double-count the match-rating influence above).
                config.recommendation_for(
                    assessed_ability as i16,
                    assessed_ability,
                    assessed_potential,
                    assignment.min_ability,
                )
            };

            let (target_league_rep, target_club_rep) =
                PlayerValuationCalculator::seller_context(country, target_club);
            let estimated_value = PlayerValuationCalculator::calculate_value_with_price_level(
                player,
                current_date,
                country.settings.pricing.price_level,
                target_league_rep,
                target_club_rep,
            );

            let player_age = player.age(current_date);
            let (contract_months, _) = player
                .contract
                .as_ref()
                .map(|c| {
                    let days = (c.expiration - current_date).num_days().max(0);
                    ((days / 30).min(i16::MAX as i64) as i16, c.salary)
                })
                .unwrap_or((0, 0));
            let risk_flags = ScoutJudgement::evaluate_risk_flags(
                player.player_attributes.is_injured,
                player.skills.mental.determination,
                player_age,
                contract_months,
                player.player_attributes.world_reputation,
                ClubView::club_world_reputation(club),
            );
            let role_fit = assignment.role_profile.fit(
                player.skills.technical.average(),
                player.skills.mental.average(),
                player.skills.physical.average(),
            );

            // Match-day monitoring update fires regardless
            // of the recommendation tier — the scout has
            // formed an opinion either way.
            monitoring_updates.push(MonitoringUpdate {
                club_id: club.id,
                scout_staff_id: match_assignment.scout_staff_id,
                player_id: player.id,
                source: ScoutMonitoringSource::MatchStandout,
                transfer_request_id: Some(assignment.transfer_request_id),
                origin_assignment_id: Some(assignment.id),
                assessed_ability,
                assessed_potential,
                confidence,
                role_fit,
                estimated_value: estimated_value.amount,
                risk_flags: risk_flags.clone(),
                is_match: true,
                region: None,
            });

            if recommendation != ScoutingRecommendation::Pass {
                reports.push(ScoutingReportResult {
                    club_id: club.id,
                    report: DetailedScoutingReport {
                        player_id: player.id,
                        assignment_id: assignment.id,
                        assessed_ability,
                        assessed_potential,
                        confidence,
                        estimated_value: estimated_value.amount,
                        recommendation,
                        role_fit,
                        risk_flags,
                    },
                    assignment_id: assignment.id,
                });
            }
        }
    }

    /// Pass 2 — write what they saw onto the clubs, the staff and the books.
    fn apply_match_scouting(
        country: &mut Country,
        current_date: NaiveDate,
        staged: MatchScoutingStaged,
    ) {
        let MatchScoutingStaged {
            observations,
            reports,
            attended_updates,
            staff_events,
            monitoring_updates,
        } = staged;
        // Pass 2: Apply observations, reports, and attendance updates
        for obs in observations {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == obs.club_id) {
                if let Some(assignment) = club
                    .transfer_plan
                    .scouting_assignments
                    .iter_mut()
                    .find(|a| a.id == obs.assignment_id)
                {
                    if obs.is_new {
                        let mut new_obs = PlayerObservation::new(
                            obs.player_id,
                            obs.assessed_ability,
                            obs.assessed_potential,
                            current_date,
                        );
                        // Start match observations at higher confidence
                        new_obs.confidence = 0.5;
                        assignment.observations.push(new_obs);
                    } else if let Some(existing) = assignment.find_observation_mut(obs.player_id) {
                        existing.add_match_observation(
                            obs.assessed_ability,
                            obs.assessed_potential,
                            obs.match_rating,
                            current_date,
                        );
                    }
                }
            }
        }

        for report in reports {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == report.club_id) {
                if !club.transfer_plan.scouting_reports.iter().any(|r| {
                    r.player_id == report.report.player_id
                        && r.assignment_id == report.assignment_id
                }) {
                    club.transfer_plan.scouting_reports.push(report.report);

                    if let Some(assignment) = club
                        .transfer_plan
                        .scouting_assignments
                        .iter_mut()
                        .find(|a| a.id == report.assignment_id)
                    {
                        assignment.reports_produced += 1;
                        if assignment.reports_produced >= 1 {
                            assignment.completed = true;
                        }
                    }
                }
            }
        }

        // Update last_attended dates
        for (club_id, team_id, date) in attended_updates {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                if let Some(match_assign) = club
                    .transfer_plan
                    .scout_match_assignments
                    .iter_mut()
                    .find(|a| a.target_team_id == team_id)
                {
                    match_assign.last_attended = Some(date);
                }
            }
        }

        // Apply monitoring updates — match-context scouting.
        for update in monitoring_updates {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == update.club_id) {
                MonitoringWriter::apply(&mut club.transfer_plan, update, current_date);
            }
        }

        // Push staff events for scouts
        for (club_id, staff_id, event_type) in staff_events {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                for team in &mut club.teams.teams {
                    if let Some(staff) = team.staffs.find_mut(staff_id) {
                        staff.add_event(event_type);
                        break;
                    }
                }
            }
        }
    }

    // ============================================================
    // Step 4: Scouting Observations
    // ============================================================

    /// Collect player summaries from a country for cross-country scouting.
    pub fn collect_player_pool(country: &Country, date: NaiveDate) -> Vec<PlayerSummary> {
        let price_level = country.settings.pricing.price_level;
        let country_id = country.id;
        let country_reputation = country.reputation;
        // Constant across every player in this country — derive it once
        // rather than per foreign-scan later.
        let country_region = ScoutingRegion::from_country(country.continent_id, &country.code);

        // Per club, in parallel. Every summary is built from that club's own
        // roster plus country-constant facts, so the clubs are disjoint, and
        // the ordered flatten leaves the pool in exactly the sequence the
        // serial walk produced — the scouting partitions and the data
        // pre-filter's sort both read that order. This is the country's
        // domestic pool AND, once per country, its slice of the world pool.
        country
            .clubs
            .par_iter()
            .map(|club| {
                let mut players = Vec::new();
                let ctx = PoolContext::of(
                    country,
                    club,
                    date,
                    price_level,
                    country_id,
                    country_reputation,
                    country_region,
                );

                for team in &club.teams.teams {
                    for player in &team.players.players {
                        if player.is_on_loan() {
                            continue;
                        }
                        players.push(Self::player_summary(&ctx, team, player));
                    }
                }
                players
            })
            .collect::<Vec<Vec<PlayerSummary>>>()
            .into_iter()
            .flatten()
            .collect()
    }

    /// Regions a club's scouting NETWORK can spot talent in, widening
    /// CONTINUOUSLY with reputation — there is no hard tier cutoff. Every club
    /// knows its own backyard; as a club grows it extends its net outward along
    /// its real transfer corridors (ordered by historical flow weight), and the
    /// world's giants reach across the entire globe. Breadth is
    /// `overall_score`-proportional (0..1 → a fraction of all regions), so a club
    /// that climbs the reputation ladder gains reach smoothly rather than
    /// flipping on at an arbitrary "Elite" line. Returned home-outward, so the
    /// caller can read it as "the regions this club reaches, nearest first".
    pub(in crate::transfers) fn reputation_scout_regions(
        home: ScoutingRegion,
        overall_score: f32,
    ) -> Vec<ScoutingRegion> {
        // Home-outward ordering: own region, then trade corridors (authored
        // highest-weight-first), then any region off the corridor map.
        let mut ordered: Vec<ScoutingRegion> = Vec::with_capacity(ScoutingRegion::all().len());
        ordered.push(home);
        for (region, _weight) in home.transfer_corridors() {
            if !ordered.contains(region) {
                ordered.push(*region);
            }
        }
        for region in ScoutingRegion::all() {
            if !ordered.contains(region) {
                ordered.push(*region);
            }
        }
        // Reputation sets how deep into that ordering the club reaches — and a
        // big club's scouting punches ABOVE its raw reputation share (it invests
        // in a global network), saturating at full world coverage. So the top
        // tiers (Continental and up) reach most or all of the world while smaller
        // clubs stay near home: with the boost, a club is global once its
        // overall_score clears ~0.77 (high-Continental / Elite). A minnow still
        // gets only its backyard.
        const REACH_BOOST: f32 = 1.3;
        let total = ordered.len() as f32;
        let reach_fraction = (overall_score.clamp(0.0, 1.0) * REACH_BOOST).min(1.0);
        let budget = (reach_fraction * total).round() as usize;
        ordered.truncate(budget.max(1)); // always at least the home backyard
        ordered
    }

    pub fn process_scouting(
        country: &mut Country,
        foreign_players: &[&PlayerSummary],
        date: NaiveDate,
        market_map: &MarketMap,
    ) {
        let country_reputation = country.reputation;
        // Single source of truth for observation/error/recommendation/risk-flag tuning.
        let config = ScoutingConfig::default();
        // Scoring-chart + recent-award lookup, built once so the data
        // pre-filter can fold each candidate's breakout score into its rank.
        let performance_lookup = PerformanceProfiler::stage("scout_performance_lookup", 4, || {
            LeaguePerformanceLookup::build(country)
        });

        // Reuse collect_player_pool for the domestic pool — the body of this
        // loop used to be a copy-paste of that function, doubling the work
        // that already runs once per country to build the shared foreign
        // pool. Now it runs once.
        let all_players: Vec<PlayerSummary> =
            PerformanceProfiler::stage("scout_domestic_pool", 4, || {
                Self::collect_player_pool(&*country, date)
            });

        // Partition both candidate pools by position group ONCE per country.
        // Every assignment scans only its own group's slice — the other
        // three groups were rejected by `player_filter`'s position check
        // anyway, but each assignment used to pay a full-pool walk (clubs ×
        // assignments × world pool) to find that out. Relative order inside
        // a group is preserved, so the filtered candidate sequences — and
        // everything downstream (data-prefilter sort, observation picks) —
        // are identical to the unpartitioned scan. The country-reputation
        // step-down on the foreign pool is country-constant, so it is folded
        // into the partition instead of being re-tested per assignment.
        // A player is filed under EVERY group he can play, not just the one
        // his primary label falls in. More than half a senior population is
        // multi-position at identical competence, so that label is an
        // arbitrary pick among equals — bucketing on it alone meant a
        // striker whose record happened to list a wing first sat in the
        // midfield bucket and no club searching for a centre-forward ever
        // walked past him. Same relative order inside each bucket, so the
        // filtered sequences (and the data-prefilter sort that reads them)
        // stay deterministic.
        let partition = PerformanceProfiler::stage_scope("scout_partition_pools", 4);
        let mut domestic_by_group: [Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT] =
            Default::default();
        for p in &all_players {
            for group in PlayerFieldPositionGroup::ALL {
                if p.coverage.covers_group(group) {
                    domestic_by_group[group.index()].push(p);
                }
            }
        }
        let mut foreign_by_group: [Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT] =
            Default::default();
        // The country step-down models the normal direction of the market:
        // clubs recruit from countries at or below their own standing, and
        // a cold approach up the pyramid isn't credible. Applied without
        // exception, though, it made whole countries invisible to each
        // other — a Portuguese club could not SEE an English player who was
        // transfer-listed, had asked to leave, or was months from a free
        // transfer, none of which depends on the buyer outranking the
        // seller's country. Availability is precisely the signal that
        // overrides the normal direction of travel, so it opens the pool;
        // everything downstream (the realism band, the staged plausibility
        // gates, reputation reach and affordability) still has to pass.
        for p in foreign_players
            .iter()
            .copied()
            .filter(|p| p.country_reputation <= country_reputation || Self::is_openly_available(p))
        {
            for group in PlayerFieldPositionGroup::ALL {
                if p.coverage.covers_group(group) {
                    foreign_by_group[group.index()].push(p);
                }
            }
        }

        // Pass 1 (PARALLEL): each club's scan is read-only — over the
        // shared pools, the country, and its own plan — and stages its
        // results on a per-club struct, so the clubs fan out across the
        // rayon pool instead of running head-to-tail inside the
        // country's serial tail. Merging in club order below keeps the
        // applied sequence identical to the old single-threaded scan;
        // the RNG draws come from the executing worker's thread-seeded
        // stream, the same order-of-execution dependence the world tick
        // already has at country granularity.
        drop(partition);

        let scan = PerformanceProfiler::stage_scope("scout_club_scan", 4);
        let country_ref: &Country = country;
        let staged_per_club: Vec<ClubScoutingStaged> = country_ref
            .clubs
            .par_iter()
            .map(|club| {
                Self::scout_club_assignments(
                    country_ref,
                    club,
                    &domestic_by_group,
                    &foreign_by_group,
                    &performance_lookup,
                    &config,
                    date,
                    market_map,
                )
            })
            .collect();

        drop(scan);

        let apply = PerformanceProfiler::stage_scope("scout_apply", 4);
        // The only cross-club effect: the `Wnt` / `Sct` badge lands on the
        // TARGET, who plays for somebody else. Everything else a club
        // staged is about that club, so it stays with it for the parallel
        // apply below. Membership set instead of `Vec::contains` — the
        // dedup was quadratic in the day's target count.
        let mut seen_targets: HashSet<(u32, u32)> = HashSet::new();
        let mut wanted_targets: Vec<(u32, u32)> = Vec::new();
        for staged in &staged_per_club {
            for target in &staged.wanted_targets {
                if seen_targets.insert(*target) {
                    wanted_targets.push(*target);
                }
            }
        }

        Self::apply_scouting_results(country, staged_per_club, wanted_targets, &config, date);
        drop(apply);
    }

    /// One club's scouting scan — pass 1 of [`Self::process_scouting`],
    /// hoisted per club so the scan parallelizes. Strictly read-only:
    /// every mutation is staged on the returned [`ClubScoutingStaged`].
    #[allow(clippy::too_many_arguments)]
    fn scout_club_assignments(
        country: &Country,
        club: &Club,
        domestic_by_group: &[Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT],
        foreign_by_group: &[Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT],
        performance_lookup: &LeaguePerformanceLookup,
        config: &ScoutingConfig,
        date: NaiveDate,
        market_map: &MarketMap,
    ) -> ClubScoutingStaged {
        ClubScan::new(country, club, performance_lookup, config, date, market_map)
            .run(domestic_by_group, foreign_by_group)
    }

    /// Pass 2 of [`Self::process_scouting`] — apply the merged staged
    /// results against the mutable country.
    #[allow(clippy::too_many_arguments)]
    /// Pass 2 of [`Self::process_scouting`] — commit the staged scan.
    ///
    /// Every event a club staged is about that club (its assignments, its
    /// scouts, its rejection memory), so the commit fans back out across
    /// `clubs.par_iter_mut()` zipped with the staged results. It used to be
    /// six flat vectors drained through `country.clubs.iter_mut().find(id)`,
    /// which is a club scan per event — on a deep pyramid that scan, not the
    /// scouting itself, was the pass.
    ///
    /// The `Wnt` / `Sct` badges are the exception: they land on the target,
    /// who plays for another club. They used to walk every club, team and
    /// player in the country PER TARGET (twice — once more inside
    /// `local_interest_signal`); now the beats are resolved in one parallel
    /// read pass and stamped in one parallel walk.
    fn apply_scouting_results(
        country: &mut Country,
        staged_per_club: Vec<ClubScoutingStaged>,
        wanted_targets: Vec<(u32, u32)>,
        config: &ScoutingConfig,
        date: NaiveDate,
    ) {
        // Resolve the interest beats before the mutable pass — each one
        // reads the whole country to find the target and his club.
        let country_ref: &Country = country;
        let wanted_signals: Vec<(u32, Option<TransferInterestSignal>)> = wanted_targets
            .par_iter()
            .map(|(scout_club_id, player_id)| {
                (
                    *player_id,
                    PlayerView::local_interest_signal(
                        country_ref,
                        *scout_club_id,
                        *player_id,
                        TransferInterestStage::ScoutWatched,
                        TransferInterestSource::ScoutAttendance,
                    ),
                )
            })
            .collect();
        // Grouped by target, in the order the clubs staged them, so a
        // player watched by several clubs hears from them in the same
        // sequence the serial loop delivered.
        let mut signals_by_player: HashMap<u32, Vec<usize>> = HashMap::new();
        for (index, (player_id, _)) in wanted_signals.iter().enumerate() {
            signals_by_player.entry(*player_id).or_default().push(index);
        }

        let rejection_months = config.assignment.rejection_memory_months;
        country
            .clubs
            .par_iter_mut()
            .zip(staged_per_club.into_par_iter())
            .for_each(|(club, staged)| {
                debug_assert!(
                    staged.observations.iter().all(|o| o.club_id == club.id),
                    "scouting: staged results must belong to their own club"
                );

                for obs in staged.observations {
                    if let Some(assignment) = club
                        .transfer_plan
                        .scouting_assignments
                        .iter_mut()
                        .find(|a| a.id == obs.assignment_id)
                    {
                        if obs.is_new {
                            assignment.observations.push(PlayerObservation::new(
                                obs.player_id,
                                obs.assessed_ability,
                                obs.assessed_potential,
                                date,
                            ));
                        } else if let Some(existing) =
                            assignment.find_observation_mut(obs.player_id)
                        {
                            existing.add_observation(
                                obs.assessed_ability,
                                obs.assessed_potential,
                                date,
                            );
                        }
                    }
                }

                for report in staged.reports {
                    if club.transfer_plan.scouting_reports.iter().any(|r| {
                        r.player_id == report.report.player_id
                            && r.assignment_id == report.assignment_id
                    }) {
                        continue;
                    }
                    club.transfer_plan.scouting_reports.push(report.report);
                    if let Some(assignment) = club
                        .transfer_plan
                        .scouting_assignments
                        .iter_mut()
                        .find(|a| a.id == report.assignment_id)
                    {
                        assignment.reports_produced += 1;
                        if assignment.reports_produced >= 1 {
                            assignment.completed = true;
                        }
                    }
                }

                // Pool-context scouting.
                for update in staged.monitoring_updates {
                    MonitoringWriter::apply(&mut club.transfer_plan, update, date);
                }

                for (_club_id, staff_id, event_type) in staged.staff_events {
                    for team in &mut club.teams.teams {
                        if let Some(staff) = team.staffs.find_mut(staff_id) {
                            staff.add_event(event_type);
                            break;
                        }
                    }
                }

                // A day spent watching a foreign player accrues on BOTH
                // axes: the region the older gates read, and the country,
                // which is where a scout actually builds knowledge.
                // Watching Colombians for two seasons makes a man who knows
                // Colombia, not South America.
                for (_club_id, staff_id, region, source_country) in staged.familiarity_events {
                    for team in &mut club.teams.teams {
                        if let Some(staff) = team.staffs.find_mut(staff_id) {
                            staff.staff_attributes.knowledge.accrue_region_day(region);
                            staff
                                .staff_attributes
                                .knowledge
                                .accrue_country_day(source_country);
                            break;
                        }
                    }
                }

                // Commit rejection memory — a Pass recommendation blocks
                // re-scouting for the configured window, spanning at least
                // the current window and (typically) the next one.
                for (_club_id, player_id) in staged.rejected_events {
                    club.transfer_plan
                        .reject_player(player_id, date, rejection_months);
                }
            });

        // Newly scouted players: stamp the market badges and let the player
        // hear about it. `Wnt` (wanted) and `Sct` (being watched) were
        // formerly anonymous; the ScoutWatched beat goes through the
        // structured interest funnel, whose surfacing gate keeps everyday
        // scout attendance out of the feed unless the club gap or an
        // emotional link (former / favourite / rival club) makes it news.
        if !signals_by_player.is_empty() {
            country.clubs.par_iter_mut().for_each(|club| {
                for team in &mut club.teams.teams {
                    for player in &mut team.players.players {
                        let Some(indexes) = signals_by_player.get(&player.id) else {
                            continue;
                        };
                        if !player.statuses.has(PlayerStatusType::Wnt) {
                            player.statuses.add(date, PlayerStatusType::Wnt);
                        }
                        if !player.statuses.has(PlayerStatusType::Sct) {
                            player.statuses.add(date, PlayerStatusType::Sct);
                        }
                        for index in indexes {
                            if let Some(signal) = wanted_signals[*index].1.as_ref() {
                                player.on_transfer_interest_signal(signal);
                            }
                        }
                    }
                }
            });
        }
    }

    /// Pick a player from a list with probability weighted by reputation.
    /// Year-round refresh of persisted shadow reports.
    ///
    /// Runs at slow cadence (weekly) regardless of transfer window state —
    /// scouts don't stop working between June and January. For each club
    /// with archived shadow reports, one lookup re-measures a random target's
    /// assessed ability against the player's current state, then dampens the
    /// shift by the scout's judging skill. This keeps tracked players from
    /// drifting out of sync while the window is closed.
    pub fn refresh_shadow_reports(country: &mut Country, date: NaiveDate) {
        if date.weekday() != Weekday::Mon {
            return;
        }
        let config = ScoutingConfig::default();

        struct RefreshUpdate {
            club_id: u32,
            player_id: u32,
            new_ability: u8,
            recorded_on: NaiveDate,
        }
        let mut updates: Vec<RefreshUpdate> = Vec::new();

        // Build a lookup of current player abilities from every club in the
        // country once — shadow targets may have moved clubs since we first
        // observed them, so we search all teams.
        let mut current_ability: HashMap<u32, u8> = HashMap::new();
        for c in &country.clubs {
            for t in &c.teams.teams {
                for p in &t.players.players {
                    current_ability
                        .insert(p.id, p.skills.calculate_ability_for_position(p.position()));
                }
            }
        }

        for club in &country.clubs {
            if club.transfer_plan.shadow_reports.is_empty() {
                continue;
            }
            // Use the Chief Scout's judging_ability if present, else a default.
            let judging = club
                .teams
                .iter()
                .flat_map(|t| t.staffs.iter())
                .filter(|s| {
                    s.contract
                        .as_ref()
                        .map(|c| {
                            matches!(c.position, StaffPosition::Scout | StaffPosition::ChiefScout,)
                        })
                        .unwrap_or(false)
                })
                .map(|s| s.staff_attributes.knowledge.judging_player_ability)
                .max()
                .unwrap_or(config.shadow.refresh_default_judging);

            let refresh_count =
                config.shadow_refresh_count(club.transfer_plan.shadow_reports.len());
            for _ in 0..refresh_count {
                let idx =
                    IntegerUtils::random(0, club.transfer_plan.shadow_reports.len() as i32 - 1)
                        as usize;
                let shadow = &club.transfer_plan.shadow_reports[idx];
                let truth = match current_ability.get(&shadow.report.player_id) {
                    Some(v) => *v,
                    None => continue, // player disappeared — refresh nothing
                };

                // Drift old assessment toward truth, damped by scout skill.
                // High-skill scouts re-measure almost exactly; low-skill drift is noisier.
                let noise =
                    (config.error.max_judging - judging as i16).max(config.error.min_error) as i32;
                let drift = IntegerUtils::random(-noise, noise);
                let blended = ((shadow.observed_ability as i32 + truth as i32) / 2 + drift)
                    .clamp(1, 200) as u8;

                updates.push(RefreshUpdate {
                    club_id: club.id,
                    player_id: shadow.report.player_id,
                    new_ability: blended,
                    recorded_on: date,
                });
            }
        }

        for u in updates {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == u.club_id) {
                if let Some(shadow) = club
                    .transfer_plan
                    .shadow_reports
                    .iter_mut()
                    .find(|s| s.report.player_id == u.player_id)
                {
                    shadow.report.assessed_ability = u.new_ability;
                    shadow.observed_ability = u.new_ability;
                    shadow.recorded_on = u.recorded_on;
                }
            }
        }
    }

    /// Higher reputation = more likely to be discovered (media exposure, word of mouth).
    /// A player with 5000 world_rep is ~6x more likely to be picked than one with 0.
    fn pick_reputation_weighted<'a>(players: &[&'a &PlayerSummary]) -> &'a PlayerSummary {
        if players.len() <= 1 {
            return players.first().unwrap();
        }

        // Weight = base(1.0) + reputation bonus (0.0 to 5.0)
        // Uses max of world and home reputation for visibility
        let weights: Vec<f32> = players
            .iter()
            .map(|p| {
                let rep = p.world_reputation.max(p.home_reputation) as f32;
                1.0 + (rep / 2000.0).min(5.0)
            })
            .collect();

        let total: f32 = weights.iter().sum();
        let roll = IntegerUtils::random(0, (total * 100.0) as i32) as f32 / 100.0;

        let mut cumulative = 0.0;
        for (i, w) in weights.iter().enumerate() {
            cumulative += w;
            if roll < cumulative {
                return players[i];
            }
        }

        players.last().unwrap()
    }

    /// One player's entry in the world pool.
    fn player_summary(ctx: &PoolContext<'_>, team: &Team, player: &Player) -> PlayerSummary {
        let country = ctx.country;
        let club = ctx.club;
        let date = ctx.date;
        let price_level = ctx.price_level;
        let country_id = ctx.country_id;
        let country_reputation = ctx.country_reputation;
        let country_region = ctx.country_region;
        let seller_league_rep = ctx.seller_league_rep;
        let seller_club_rep = ctx.seller_club_rep;
        let club_world_rep = ctx.club_world_rep;
        let seller_club_rep_score = ctx.seller_club_rep_score;
        let seller_league_id = ctx.seller_league_id;
        let seller_in_debt = ctx.seller_in_debt;
        let seller_club_matches = ctx.seller_club_matches;
        let group_ranks = &ctx.group_ranks;
        let home_preferences = &ctx.home_preferences;

        let value = PlayerValuationCalculator::calculate_value_with_price_level(
            player,
            date,
            price_level,
            seller_league_rep,
            seller_club_rep,
        );
        let (contract_months_remaining, salary) = player
            .contract
            .as_ref()
            .map(|c| {
                let days = (c.expiration - date).num_days().max(0);
                ((days / 30).min(i16::MAX as i64) as i16, c.salary)
            })
            .unwrap_or((0, 0));
        // `ClubGroupRanks` only indexes the FIRST-TEAM roster, so
        // everyone below it came back `u8::MAX` and was then read
        // as rank 1 — "the seller's second choice in his
        // position". That inflated every B / reserve / youth
        // player's market importance to near-untouchable, which
        // is the opposite of the truth: he is not in the first
        // team's depth chart at all. Rank him behind it instead.
        let seller_rank = match group_ranks.rank(player.id) {
            u8::MAX => group_ranks
                .group_size(player.position().position_group())
                .saturating_add(1),
            r => r,
        };
        // A B / Second side's own "key player" label is standing
        // in that dressing room, not a first-team promise the
        // market should price against. Read it through the tier
        // that awarded it.
        let squad_status = player
            .contract
            .as_ref()
            .map(|c| c.squad_status.as_first_team_designation(team.team_type))
            .unwrap_or(PlayerSquadStatus::NotYetSet);
        PlayerSummary {
            player_id: player.id,
            club_id: club.id,
            country_id,
            continent_id: country.continent_id,
            region: country_region,
            country_code: country.code.clone(),
            // Where he is FROM, alongside where he plays. The
            // loan market could not see the difference, so a
            // Brazilian prospect's own league had no way to
            // recognise one of its own exports.
            nationality_country_id: player.country_id,
            nationality_continent_id: player.nationality_continent_id,
            nationality_region: player.home_region(),
            starter_share: player.happiness.starter_ratio,
            tenure_days: StuckCareerScan::club_tenure_days(player, date)
                .unwrap_or(i64::from(u16::MAX))
                .clamp(0, i64::from(u16::MAX)) as u16,
            // Read off the weekly cache, not rebuilt here: the
            // mind thinks weekly and `WantsReturnHome` fires on
            // a 60-day cooldown, so a per-player-per-day
            // `MindSituation` build bought nothing (C10).
            return_home_desire: player.home_pull.desire,
            // The parent's own posting: a foreigner the club
            // has put on the loan market — by badge, by its own
            // candidate list, or because the candidate carries
            // a destination preference — whose want to go home
            // has formed. This is the ONE bit that crosses a
            // border (no `&mut` travels, the borrower reads a
            // bool) and it is deliberately scoped to LOANS, so
            // the home-visibility arm can never become a
            // permanent-transfer discovery channel that
            // bypasses the springboard reach model (Part VIII,
            // "the compatriot loophole").
            home_return_wanted: HomeLoanGates::is_posted(
                player.home_pull.wanted,
                home_preferences.contains(&player.id),
            ),
            ambition: player.attributes.ambition as u8,
            loyalty: player.attributes.loyalty as u8,
            adaptability: player.attributes.adaptability as u8,
            leave_pressure: player
                .mind
                .pressure_of(GoalKind::GoOutOnLoan)
                .max(player.mind.pressure_of(GoalKind::LeaveThisClub))
                .max(player.mind.pressure_of(GoalKind::PlayFirstTeamFootball))
                .clamp(0.0, 1.0),
            stay_pressure: player
                .mind
                .pressure_of(GoalKind::StayAtThisClub)
                .max(player.mind.pressure_of(GoalKind::BecomeAClubLegend))
                .clamp(0.0, 1.0),
            player_name: player.full_name.to_string(),
            club_name: club.name.clone(),
            position: player.position(),
            position_group: player.position().position_group(),
            coverage: PositionCoverage::of(&player.positions),
            age: player.age(date),
            estimated_value: value.amount,
            is_listed: player.statuses.has(PlayerStatusType::Lst),
            is_loan_listed: player.statuses.has(PlayerStatusType::Loa),
            skill_ability: player
                .skills
                .calculate_ability_for_position(player.position()),
            // Transfer-market candidate listing: regressed
            // value so the candidate sorter / recommendation
            // engine isn't fooled by a small-sample season.
            average_rating: player
                .statistics
                .average_rating_realistic(player.position().position_group()),
            goals: player.statistics.goals,
            assists: player.statistics.assists,
            appearances: player.statistics.total_games(),
            determination: player.skills.mental.determination,
            work_rate: player.skills.mental.work_rate,
            composure: player.skills.mental.composure,
            anticipation: player.skills.mental.anticipation,
            technical_avg: player.skills.technical.average(),
            mental_avg: player.skills.mental.average(),
            physical_avg: player.skills.physical.average(),
            current_reputation: player.player_attributes.current_reputation,
            home_reputation: player.player_attributes.home_reputation,
            world_reputation: player.player_attributes.world_reputation,
            country_reputation,
            club_world_reputation: club_world_rep,
            club_best_in_group: group_ranks.best(player.position().position_group()),
            is_injured: player.player_attributes.is_injured,
            contract_months_remaining,
            salary,
            language_profile: LanguageProfile::from_languages(&player.languages),
            international_apps: player.player_attributes.international_apps,
            career_record: CareerRecordSnapshot::read(player, player.position().position_group()),
            seller_ctx: SellerPlausibilityContext {
                club_reputation_score: seller_club_rep_score,
                league_reputation: seller_league_rep,
                league_id: seller_league_id,
                position_group_rank: seller_rank,
                squad_status,
                is_transfer_requested: player.statuses.has(PlayerStatusType::Req),
                is_unhappy: player.statuses.has(PlayerStatusType::Unh),
                in_debt: seller_in_debt,
                days_on_market: player.days_available(date).min(i16::MAX as i64) as i16,
                market_resignation: player.market_resignation(date),
                club_matches_played: SquadEvidenceSource::club_matches(
                    team.team_type,
                    team.league_id.is_some(),
                    seller_club_matches,
                ),
                big_stage_inclination: player.big_stage_inclination,
                is_marketed: club.transfer_plan.is_marketed(player.id),
            },
        }
    }
}

#[cfg(test)]
mod scout_reach_tests {
    use super::*;

    /// Scouting reach widens continuously with reputation — no hard tier line.
    #[test]
    fn reach_scales_continuously_with_reputation() {
        let home = ScoutingRegion::WesternEurope;
        let total = ScoutingRegion::all().len();

        // A giant reaches the whole world...
        let giant = ScoutingPass::reputation_scout_regions(home, 1.0);
        assert_eq!(giant.len(), total);

        // ...a minnow only its own backyard...
        let minnow = ScoutingPass::reputation_scout_regions(home, 0.0);
        assert_eq!(minnow, vec![home]);

        // ...and clubs in between land strictly in between, monotonically.
        let small = ScoutingPass::reputation_scout_regions(home, 0.3);
        let big = ScoutingPass::reputation_scout_regions(home, 0.7);
        assert!(minnow.len() < small.len());
        assert!(
            small.len() < big.len(),
            "small {} >= big {}",
            small.len(),
            big.len()
        );
        assert!(big.len() < giant.len());

        // A Continental club (a second-tier giant, ~0.65) reaches MOST of the
        // world — the reach boost lifts the upper tiers toward global.
        let continental = ScoutingPass::reputation_scout_regions(home, 0.65);
        assert!(
            continental.len() >= 12,
            "Continental reach {} should be near-global",
            continental.len()
        );

        // Home is always covered and always first (nearest-out ordering).
        assert_eq!(small[0], home);
        assert_eq!(big[0], home);
    }

    /// A top European club reaches the talent-rich corridors (South America,
    /// West/North Africa) so it can scout a wonderkid there, sign him, and loan
    /// him out — the whole point of global scouting for a giant.
    #[test]
    fn top_european_club_reaches_talent_corridors() {
        let reach = ScoutingPass::reputation_scout_regions(ScoutingRegion::WesternEurope, 0.85);
        assert!(reach.contains(&ScoutingRegion::SouthAmerica));
        assert!(reach.contains(&ScoutingRegion::WestAfrica));
        assert!(reach.contains(&ScoutingRegion::NorthAfrica));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::transfers::market::map::{
        CorridorWeight, CountryTransferProfile, MarketCountryFacts,
    };

    /// A miniature world: Portugal imports Brazilians heavily and Spaniards
    /// hardly at all, and Spain is the club doing the looking.
    struct ReachFixtures;

    impl ReachFixtures {
        const BR: u32 = 1;
        const PT: u32 = 2;
        const ES: u32 = 3;

        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()
        }

        fn facts(id: u32, code: &str, continent: u32, top: u16) -> MarketCountryFacts {
            MarketCountryFacts {
                id,
                code: code.to_string(),
                continent_id: continent,
                region: ScoutingRegion::from_country(continent, code),
                reputation: top,
                top_flight_reputation: top,
                median_top_flight_wage: 500_000,
            }
        }

        fn weight(country_id: u32, weight: f32) -> CorridorWeight {
            CorridorWeight {
                country_id,
                weight,
                money: false,
            }
        }

        fn world() -> MarketMap {
            let mut facts = HashMap::new();
            for f in [
                Self::facts(Self::BR, "br", 3, 7800),
                Self::facts(Self::PT, "pt", 1, 7500),
                Self::facts(Self::ES, "es", 1, 9200),
            ] {
                facts.insert(f.id, f);
            }
            let mut profiles = HashMap::new();
            // Spain's clubs buy Brazilians; they do not import Portuguese
            // nationals at anything like the same rate.
            profiles.insert(
                Self::ES,
                CountryTransferProfile {
                    import: vec![Self::weight(Self::BR, 1.0), Self::weight(Self::PT, 0.05)],
                    ..Default::default()
                },
            );
            profiles.insert(
                Self::BR,
                CountryTransferProfile {
                    export: vec![Self::weight(Self::ES, 1.0), Self::weight(Self::PT, 1.0)],
                    ..Default::default()
                },
            );
            profiles.insert(
                Self::PT,
                CountryTransferProfile {
                    import: vec![Self::weight(Self::BR, 1.0)],
                    export: vec![Self::weight(Self::ES, 0.05)],
                    ..Default::default()
                },
            );
            MarketMap::new(profiles, facts)
        }
    }

    /// The defect the pair key exists for: a Brazilian at Porto and a
    /// Portuguese at Porto are two different market questions, and keying
    /// the memo on the LEAGUE alone let whichever was scored first decide
    /// the reach for both.
    #[test]
    fn two_passports_at_one_club_are_two_different_markets() {
        let map = ReachFixtures::world();
        let ledger = ClubMarketLedger::default();
        let cache = MarketReachCache::new();
        let no_scouts = |_: u32| -> u8 { 0 };

        let brazilian = cache.reach_for_pair(
            &map,
            ReachFixtures::ES,
            &ledger,
            (ReachFixtures::BR, ReachFixtures::PT),
            ReachFixtures::day(),
            &no_scouts,
        );
        let portuguese = cache.reach_for_pair(
            &map,
            ReachFixtures::ES,
            &ledger,
            (ReachFixtures::PT, ReachFixtures::PT),
            ReachFixtures::day(),
            &no_scouts,
        );
        assert!(
            brazilian > portuguese * 1.5,
            "a Brazilian at Porto ({brazilian}) must read differently from a \
             Portuguese at Porto ({portuguese})"
        );

        // And the order they are asked in must not change either answer.
        let fresh = MarketReachCache::new();
        let portuguese_first = fresh.reach_for_pair(
            &map,
            ReachFixtures::ES,
            &ledger,
            (ReachFixtures::PT, ReachFixtures::PT),
            ReachFixtures::day(),
            &no_scouts,
        );
        let brazilian_second = fresh.reach_for_pair(
            &map,
            ReachFixtures::ES,
            &ledger,
            (ReachFixtures::BR, ReachFixtures::PT),
            ReachFixtures::day(),
            &no_scouts,
        );
        assert_eq!(portuguese_first, portuguese);
        assert_eq!(brazilian_second, brazilian);
    }

    #[test]
    fn a_world_with_no_geography_reads_every_market_as_open() {
        let cache = MarketReachCache::new();
        let ledger = ClubMarketLedger::default();
        assert_eq!(
            cache.reach_for_pair(
                &MarketMap::default(),
                ReachFixtures::ES,
                &ledger,
                (ReachFixtures::BR, ReachFixtures::PT),
                ReachFixtures::day(),
                &|_| 0,
            ),
            1.0
        );
    }
}
