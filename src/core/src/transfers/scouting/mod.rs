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

use crate::transfers::scouting::assignment::ClubScan;
use crate::transfers::scouting::judgement::ScoutJudgement;
use crate::transfers::view::club::ClubView;
use crate::transfers::view::player::PlayerView;
use crate::transfers::view::world::{ForeignPool, WorldPool};
use chrono::{Datelike, NaiveDate};
use log::debug;

use crate::club::player::events::transfer_social::TransferInterestSignal;
use crate::club::player::language::Language;
use crate::transfers::MarketMap;
use crate::transfers::ScoutingRegion;
use crate::transfers::gate::build::Suitor;
use crate::transfers::gate::{MoveTerms, PursuitAction, TransferMovePlausibility};
use crate::transfers::loan::interest::InterestDraw;
use crate::transfers::pipeline::processor::PlayerSummary;
use crate::transfers::pipeline::{
    DetailedScoutingReport, FileObservation, FileReading, FileReview, PlayerObservation,
    ScoutMatchAssignment, ScoutingAssignment, ScoutingRecommendation, TransferNeedPriority,
    TransferRequest, TransferRequestStatus,
};
use crate::transfers::scouting::breakout::LeaguePerformanceLookup;
use crate::transfers::scouting::config::ScoutingConfig;
use crate::transfers::value::PlayerValuationCalculator;
use crate::transfers::view::player::{PlayerSummaryBuilder, SummaryClubContext};
use crate::utils::IntegerUtils;
use crate::utils::PerformanceProfiler;
use crate::{
    Club, ClubPhilosophy, Country, Person, PlayerFieldPositionGroup, PlayerStatusType,
    StaffEventType, StaffPosition, TeamType, TransferInterestSource, TransferInterestStage,
};
use crate::{Player, Team};
use chrono::Weekday;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Ordering;

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

/// A viewing to file, staged by the read pass with the gate's reading of
/// the move and committed through the plan's door.
struct MonitoringUpdate {
    club_id: u32,
    reading: FileReading,
    seen: FileObservation,
}

/// A player a scout actually got to look at tonight, and the standing
/// assignment that sent him.
struct PlayerSeen<'a> {
    player: &'a Player,
    assignment: &'a ScoutingAssignment,
    age: u8,
    /// The REGRESSED season average, never the raw one.
    match_rating: f32,
    /// The gate's reading of a move for him.
    reading: FileReading,
}

/// One fixture a scout was sent to, and the side he was sent to watch.
struct MatchWatch<'a> {
    assignment: &'a ScoutMatchAssignment,
    target_club: &'a Club,
    target_team: &'a Team,
    /// How good this scout's eye is — the spread on every read below.
    judging_ability: u8,
    judging_potential: u8,
    /// The scout's club, as every route reads it.
    suitor: &'a Suitor<'a>,
    /// The watched club, as every summary of its players reads it.
    target_ctx: SummaryClubContext,
}

/// What one country's scouts saw at the weekend's fixtures. The read pass walks
/// every club while deciding, so nothing can be written until it is done —
/// which is what the `Pass 1` / `Pass 2` banners inside the old 399-line body
/// were describing.
struct MatchScoutingStaged {
    observations: Vec<MatchScoutingObservationResult>,
    reports: Vec<ScoutingReportResult>,
    attended_updates: Vec<(u32, u32, NaiveDate)>,
    staff_events: Vec<(u32, u32, StaffEventType)>,
    monitoring_updates: Vec<MonitoringUpdate>,
}

/// The scouting tick: assignments, observations, match reports and the shadow board.
pub struct ScoutingPass;

impl ScoutingPass {
    /// A pool filed under every group each player can play, in pool order.
    /// More than half a senior population is multi-position at identical
    /// competence, so bucketing on the primary label alone hid every wide
    /// forward who plays centre-forward from a club looking for one.
    fn by_group<'p>(
        pool: impl Iterator<Item = &'p PlayerSummary>,
    ) -> [Vec<&'p PlayerSummary>; PlayerFieldPositionGroup::COUNT] {
        let mut by_group: [Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT] =
            Default::default();
        for p in pool {
            for group in PlayerFieldPositionGroup::ALL {
                if p.coverage.covers_group(group) {
                    by_group[group.index()].push(p);
                }
            }
        }
        by_group
    }

    pub fn assign_scouts(
        country: &mut Country,
        world: WorldPool<'_>,
        market_map: &MarketMap,
        date: NaiveDate,
    ) {
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
                    request.position,
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
        // A report is read again first: a man who has since moved out of
        // the club's standing seeds nothing.
        for club_id in seeded_clubs {
            let Some(index) = country.clubs.iter().position(|c| c.id == club_id) else {
                continue;
            };
            let keeps: FxHashSet<u32> = {
                let club = &country.clubs[index];
                let suitor = Suitor::of(country, club, date);
                club.transfer_plan
                    .shadow_reports
                    .iter()
                    .map(|s| s.report.player_id)
                    .filter(|player_id| {
                        world.find(*player_id).is_some_and(|summary| {
                            TransferMovePlausibility::read(
                                &suitor,
                                summary,
                                MoveTerms::purchase(true),
                                market_map,
                            )
                            .reaches(PursuitAction::KeepFile.needs())
                        })
                    })
                    .collect()
            };
            country.clubs[index]
                .transfer_plan
                .seed_active_reports_from_shadow(|player_id| keeps.contains(&player_id));
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
            // Read once per club, and only for a club whose scout actually
            // saw a match today.
            let mut suitor: Option<Suitor<'_>> = None;

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

                let suitor: &Suitor<'_> =
                    suitor.get_or_insert_with(|| Suitor::of(country, club, current_date));

                Self::observe_target_team(
                    country,
                    club,
                    &MatchWatch {
                        assignment: match_assignment,
                        target_club,
                        target_team,
                        judging_ability,
                        judging_potential,
                        suitor,
                        target_ctx: SummaryClubContext::of(
                            country,
                            target_club,
                            current_date,
                            false,
                        ),
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

        // A scout at the match sees everyone; the club opens a file only on
        // a man the gate lets it watch. One country, so no geography.
        let summary = PlayerSummaryBuilder::build(
            country,
            target_club,
            &watch.target_ctx,
            target_team,
            player,
            current_date,
        );
        let assessment = TransferMovePlausibility::read(
            watch.suitor,
            &summary,
            MoveTerms::purchase(true),
            &MarketMap::default(),
        );
        if !assessment.reaches(PursuitAction::KeepFile.needs()) {
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
                reading: FileReading::of(&assessment, current_date),
            },
            current_date,
            staged,
        );
    }

    /// What the scout writes down, once the gate has let this player
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
                reading: seen.reading,
                seen: FileObservation {
                    scout_staff_id: match_assignment.scout_staff_id,
                    player_id: player.id,
                    source: ScoutMonitoringSource::MatchStandout,
                    transfer_request_id: Some(assignment.transfer_request_id),
                    origin_assignment_id: Some(assignment.id),
                    region: None,
                    assessed_ability,
                    assessed_potential,
                    confidence,
                    role_fit,
                    estimated_value: estimated_value.amount,
                    risk_flags: risk_flags.clone(),
                    is_match: true,
                },
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
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == obs.club_id)
                && let Some(assignment) = club
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

        for report in reports {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == report.club_id)
                && !club.transfer_plan.scouting_reports.iter().any(|r| {
                    r.player_id == report.report.player_id
                        && r.assignment_id == report.assignment_id
                })
            {
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

        // Update last_attended dates
        for (club_id, team_id, date) in attended_updates {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id)
                && let Some(match_assign) = club
                    .transfer_plan
                    .scout_match_assignments
                    .iter_mut()
                    .find(|a| a.target_team_id == team_id)
            {
                match_assign.last_attended = Some(date);
            }
        }

        // Apply monitoring updates — match-context scouting.
        for update in monitoring_updates {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == update.club_id) {
                club.transfer_plan
                    .keep_file(update.reading, update.seen, current_date);
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
    /// `stage_parent_loan` stages each parent's view of lending him, which
    /// only the foreign loan scan reads off the world pool.
    pub fn collect_player_pool(
        country: &Country,
        date: NaiveDate,
        stage_parent_loan: bool,
    ) -> Vec<PlayerSummary> {
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
                let ctx = SummaryClubContext::of(country, club, date, stage_parent_loan);
                for team in &club.teams.teams {
                    for player in &team.players.players {
                        if player.is_on_loan() {
                            continue;
                        }
                        players.push(PlayerSummaryBuilder::build(
                            country, club, &ctx, team, player, date,
                        ));
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
        foreign_players: ForeignPool<'_>,
        date: NaiveDate,
        market_map: &MarketMap,
    ) {
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
                Self::collect_player_pool(&*country, date, false)
            });

        // Partition both candidate pools by position group ONCE per country.
        // Every assignment scans only its own group's slice — the other
        // three groups were rejected by `player_filter`'s position check
        // anyway, but each assignment used to pay a full-pool walk (clubs ×
        // assignments × world pool) to find that out. Relative order inside
        // a group is preserved, so the filtered candidate sequences — and
        // everything downstream (data-prefilter sort, observation picks) —
        // are identical to the unpartitioned scan.
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
        let domestic_by_group = Self::by_group(all_players.iter());
        // The reputation of the country a player plays in does not decide
        // whether a club may watch him: his level, his wage and the club's
        // market do, as the gate reads them.
        let foreign_by_group = Self::by_group(foreign_players.iter());

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
        let mut seen_targets: FxHashSet<(u32, u32)> = FxHashSet::default();
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
        let mut signals_by_player: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
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

                for update in staged.monitoring_updates {
                    club.transfer_plan
                        .keep_file(update.reading, update.seen, date);
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
    /// Once a week every club re-reads the files it holds on other clubs'
    /// players and closes those whose case has gone. Read in parallel, one
    /// club per worker; each club commits only its own review.
    pub fn review_files(
        country: &mut Country,
        world: WorldPool<'_>,
        market_map: &MarketMap,
        date: NaiveDate,
    ) {
        if date.weekday() != Weekday::Mon {
            return;
        }
        let country_ref: &Country = country;
        let reviews: Vec<FileReview> = country_ref
            .clubs
            .par_iter()
            .map(|club| {
                let suitor = Suitor::of(country_ref, club, date);
                club.transfer_plan.review_files(
                    &suitor,
                    world,
                    market_map,
                    |player_id| {
                        country_ref
                            .transfer_market
                            .has_active_negotiation_for(player_id, club.id)
                    },
                    date,
                )
            })
            .collect();
        country
            .clubs
            .par_iter_mut()
            .zip(reviews.into_par_iter())
            .for_each(|(club, review)| club.transfer_plan.apply_review(review));
    }

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
        let mut current_ability: FxHashMap<u32, u8> = FxHashMap::default();
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
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == u.club_id)
                && let Some(shadow) = club
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
mod match_scouting_gate_tests {
    use super::*;
    use crate::PlayerPositionType;
    use crate::club::team::MatchHistoryItem;
    use crate::r#match::TeamScore;
    use crate::transfers::pipeline::ScoutingAssignment;
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestPlayer};

    struct Fx;

    impl Fx {
        const SCOUT: u32 = 501;
        const AFFORDABLE: u32 = 21;
        const OUT_OF_REACH: u32 = 22;

        fn midfielder(id: u32, salary: u32) -> Player {
            TestPlayer::new(id)
                .position(PlayerPositionType::MidfielderCenter)
                .ability(130)
                .age(25)
                .skills_match_ability()
                .contract_until(salary, TestDate::on(2029, 6, 30))
                .build()
        }

        /// A small club whose scout attends a big club's match today, sent
        /// for a central midfielder.
        fn country() -> Country {
            let today = TestDate::today();
            let mut buyer = TestClub::new(1).reputation(2_000).build();
            let mut assignment = ScoutingAssignment::new(
                7,
                70,
                Some(Self::SCOUT),
                PlayerPositionType::MidfielderCenter,
                60,
                16,
                35,
                1_000_000.0,
            );
            assignment.completed = false;
            buyer.transfer_plan.scouting_assignments.push(assignment);
            buyer
                .transfer_plan
                .scout_match_assignments
                .push(ScoutMatchAssignment {
                    scout_staff_id: Self::SCOUT,
                    target_team_id: 20,
                    target_club_id: 2,
                    linked_assignment_ids: vec![7],
                    last_attended: None,
                });
            let mut seller = TestClub::new(2)
                .reputation(7_000)
                .players(vec![
                    Self::midfielder(Self::AFFORDABLE, 60_000),
                    Self::midfielder(Self::OUT_OF_REACH, 40_000_000),
                ])
                .build();
            seller.teams.teams[0]
                .match_history
                .add(MatchHistoryItem::new(
                    today.and_hms_opt(0, 0, 0).unwrap(),
                    99,
                    (
                        TeamScore::new_with_score(20, 2),
                        TeamScore::new_with_score(99, 0),
                    ),
                ));
            TestCountry::new(1).clubs(vec![buyer, seller]).build()
        }
    }

    /// The scout sees both men; the club opens a file only on the one the
    /// gate lets it watch.
    #[test]
    fn a_standout_the_gate_does_not_let_his_club_watch_gets_no_file() {
        let mut country = Fx::country();
        let today = TestDate::today();
        for _ in 0..3 {
            ScoutingPass::process_match_scouting(&mut country, today);
        }
        let plan = &country.clubs[0].transfer_plan;
        assert!(
            !plan.monitorings_for_player(Fx::AFFORDABLE).is_empty(),
            "the standout the club could pay is filed"
        );
        assert!(
            plan.monitorings_for_player(Fx::OUT_OF_REACH).is_empty(),
            "the one on a wage it could never hold is not"
        );
    }
}

#[cfg(test)]
mod shadow_seed_tests {
    use super::*;
    use crate::PlayerPositionType;
    use crate::transfers::pipeline::{ShadowReport, TransferNeedReason};
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestSummary};
    use crate::transfers::view::world::WorldPoolIndex;

    struct Fx;

    impl Fx {
        fn shadow(player_id: u32) -> ShadowReport {
            ShadowReport {
                report: DetailedScoutingReport {
                    player_id,
                    assignment_id: 1,
                    assessed_ability: 110,
                    assessed_potential: 120,
                    confidence: 0.8,
                    estimated_value: 500_000.0,
                    recommendation: ScoutingRecommendation::Buy,
                    role_fit: 1.0,
                    risk_flags: Vec::new(),
                },
                position_group: PlayerFieldPositionGroup::Midfielder,
                observed_ability: 110,
                recorded_on: TestDate::on(2026, 1, 31),
            }
        }
    }

    /// A new window reads last window's reports again before they seed
    /// anything: the man who has since moved out of the club's standing
    /// seeds no report.
    #[test]
    fn a_report_from_last_window_is_read_again_before_it_is_used() {
        let mut club = TestClub::new(1).reputation(2_000).build();
        club.transfer_plan.initialized = true;
        club.transfer_plan
            .transfer_requests
            .push(TransferRequest::new(
                5,
                PlayerPositionType::MidfielderCenter,
                TransferNeedPriority::Important,
                TransferNeedReason::QualityUpgrade,
                90,
                110,
                1_000_000.0,
            ));
        club.transfer_plan.shadow_reports.push(Fx::shadow(30));
        club.transfer_plan.shadow_reports.push(Fx::shadow(31));
        let mut country = TestCountry::new(1)
            .league_reputation(2_000)
            .clubs(vec![club])
            .build();

        let still_in_reach = TestSummary::new(30)
            .club_id(2)
            .league(2_000, 0.2)
            .salary(80_000)
            .build();
        let moved_on = TestSummary::new(31)
            .club_id(3)
            .league(8_000, 0.85)
            .ability(150)
            .reputation(7_000)
            .salary(9_000_000)
            .build();
        let pool = vec![still_in_reach, moved_on];
        let index = WorldPoolIndex::build(&pool);
        ScoutingPass::assign_scouts(
            &mut country,
            WorldPool::new(&pool, &index),
            &MarketMap::default(),
            TestDate::today(),
        );

        let seeded: Vec<u32> = country.clubs[0]
            .transfer_plan
            .scouting_reports
            .iter()
            .map(|r| r.player_id)
            .collect();
        assert_eq!(seeded, vec![30]);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::PlayerPositionType;
    use crate::shared::{Currency, CurrencyValue};
    use crate::transfers::deal::negotiation::TransferNegotiation;
    use crate::transfers::deal::offer::TransferOffer;
    use crate::transfers::gate::{TransferMoveStage, TransferPlausibilityReason};
    use crate::transfers::pipeline::{
        ShortlistCandidate, ShortlistCandidateStatus, TransferNeedReason,
    };
    use crate::transfers::scouting::recruitment::ScoutMonitoringSource;
    use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestSummary};
    use crate::transfers::view::world::WorldPoolIndex;

    const STILL_FITS: u32 = 30;
    const MOVED_ON: u32 = 31;

    struct Fx;

    impl Fx {
        fn monday() -> NaiveDate {
            TestDate::on(2026, 7, 6)
        }

        fn seen(player_id: u32) -> FileObservation {
            FileObservation {
                scout_staff_id: 1,
                player_id,
                source: ScoutMonitoringSource::StaffRecommendation,
                transfer_request_id: None,
                origin_assignment_id: None,
                region: None,
                assessed_ability: 110,
                assessed_potential: 120,
                confidence: 0.7,
                role_fit: 1.0,
                estimated_value: 500_000.0,
                risk_flags: Vec::new(),
                is_match: false,
            }
        }

        /// A small club holding a file on each man, both read as fitting
        /// when they were filed.
        fn country() -> Country {
            let mut club = TestClub::new(1).reputation(2_000).build();
            let filed = FileReading {
                stage: TransferMoveStage::CanCompleteMove,
                stopped_by: None,
                read_on: TestDate::today(),
            };
            for player_id in [STILL_FITS, MOVED_ON] {
                club.transfer_plan
                    .keep_file(filed, Fx::seen(player_id), TestDate::today());
            }
            TestCountry::new(1)
                .league_reputation(2_000)
                .clubs(vec![club])
                .build()
        }

        /// One man still on a wage the club can carry; the other has since
        /// signed at a big club on a wage it could not hold for one shirt.
        fn pool() -> Vec<PlayerSummary> {
            vec![
                TestSummary::new(STILL_FITS)
                    .club_id(2)
                    .league(2_000, 0.2)
                    .salary(80_000)
                    .build(),
                TestSummary::new(MOVED_ON)
                    .club_id(3)
                    .league(2_000, 0.85)
                    .salary(9_000_000)
                    .build(),
            ]
        }

        fn review(country: &mut Country) {
            let pool = Fx::pool();
            let index = WorldPoolIndex::build(&pool);
            ScoutingPass::review_files(
                country,
                WorldPool::new(&pool, &index),
                &MarketMap::default(),
                Fx::monday(),
            );
        }

        fn negotiate(country: &mut Country, player_id: u32) {
            let offer = TransferOffer::new(
                CurrencyValue::new(5_000_000.0, Currency::Usd),
                1,
                Fx::monday(),
            );
            let negotiation = TransferNegotiation::new(
                7001,
                player_id,
                0,
                3,
                1,
                offer,
                Fx::monday(),
                0.7,
                0.18,
                27,
                10.0,
            );
            country
                .transfer_market
                .negotiations
                .insert(7001, negotiation);
        }
    }

    #[test]
    fn a_man_who_moved_out_of_reach_has_his_file_closed() {
        let mut country = Fx::country();

        Fx::review(&mut country);

        let plan = &country.clubs[0].transfer_plan;
        assert!(plan.monitorings_for_player(MOVED_ON).is_empty());
        let reading = plan.file_reading(MOVED_ON).copied().unwrap();
        assert_eq!(reading.read_on, Fx::monday());
        assert!(!reading.allows(PursuitAction::KeepFile));
        assert_eq!(
            reading.stopped_by,
            Some(TransferPlausibilityReason::UnaffordableWages),
            "the club remembers why"
        );
    }

    #[test]
    fn a_file_that_still_stands_is_kept_as_it_is() {
        let mut country = Fx::country();
        let before = country.clubs[0]
            .transfer_plan
            .monitorings_for_player(STILL_FITS)
            .iter()
            .map(|m| (m.times_watched, m.confidence, m.last_observed))
            .collect::<Vec<_>>();

        Fx::review(&mut country);

        let plan = &country.clubs[0].transfer_plan;
        let after = plan
            .monitorings_for_player(STILL_FITS)
            .iter()
            .map(|m| (m.times_watched, m.confidence, m.last_observed))
            .collect::<Vec<_>>();
        assert_eq!(before.len(), 1);
        assert_eq!(after, before);
        assert!(
            plan.file_reading(STILL_FITS)
                .is_some_and(|r| r.allows(PursuitAction::KeepFile) && r.read_on == Fx::monday())
        );
    }

    /// A shortlist whose next name has moved out of reach: the review
    /// closes his place, the cursor moves on to the man still in reach, and
    /// a list it empties sends the request back to be searched again.
    #[test]
    fn a_closed_shortlist_place_does_not_hold_the_cursor() {
        let mut country = Fx::country();
        let plan = &mut country.clubs[0].transfer_plan;
        plan.transfer_requests.push(TransferRequest::new(
            5,
            PlayerPositionType::MidfielderCenter,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
            90,
            110,
            1_000_000.0,
        ));
        plan.transfer_requests[0].status = TransferRequestStatus::Shortlisted;
        let filed = FileReading {
            stage: TransferMoveStage::CanCompleteMove,
            stopped_by: None,
            read_on: TestDate::today(),
        };
        for player_id in [MOVED_ON, STILL_FITS] {
            plan.shortlist(
                filed,
                5,
                1_000_000.0,
                ShortlistCandidate {
                    player_id,
                    score: 0.5,
                    estimated_fee: 500_000.0,
                    status: ShortlistCandidateStatus::Available,
                },
                None,
            );
        }

        Fx::review(&mut country);

        let plan = &country.clubs[0].transfer_plan;
        let list = &plan.shortlists[0];
        assert_eq!(
            list.candidates[0].status,
            ShortlistCandidateStatus::Unavailable
        );
        assert_eq!(
            list.current_candidate().map(|c| c.player_id),
            Some(STILL_FITS)
        );
        assert_eq!(
            plan.transfer_requests[0].status,
            TransferRequestStatus::Shortlisted
        );
    }

    #[test]
    fn a_shortlist_the_review_empties_reopens_its_request() {
        let mut country = Fx::country();
        let plan = &mut country.clubs[0].transfer_plan;
        plan.transfer_requests.push(TransferRequest::new(
            5,
            PlayerPositionType::MidfielderCenter,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
            90,
            110,
            1_000_000.0,
        ));
        plan.transfer_requests[0].status = TransferRequestStatus::Shortlisted;
        plan.shortlist(
            FileReading {
                stage: TransferMoveStage::CanCompleteMove,
                stopped_by: None,
                read_on: TestDate::today(),
            },
            5,
            1_000_000.0,
            ShortlistCandidate {
                player_id: MOVED_ON,
                score: 0.5,
                estimated_fee: 500_000.0,
                status: ShortlistCandidateStatus::Available,
            },
            None,
        );

        Fx::review(&mut country);

        let plan = &country.clubs[0].transfer_plan;
        assert!(plan.shortlists[0].all_exhausted());
        assert_eq!(
            plan.transfer_requests[0].status,
            TransferRequestStatus::Pending
        );
    }

    #[test]
    fn a_negotiation_holds_the_file_open() {
        let mut country = Fx::country();
        Fx::negotiate(&mut country, MOVED_ON);

        Fx::review(&mut country);

        let plan = &country.clubs[0].transfer_plan;
        assert_eq!(plan.monitorings_for_player(MOVED_ON).len(), 1);
        assert!(
            plan.file_reading(MOVED_ON)
                .is_some_and(|r| !r.allows(PursuitAction::KeepFile))
        );
    }

    /// A board approval marks his rows as negotiating before any talks
    /// open; with none live, the mark holds nothing at the review.
    #[test]
    fn a_negotiating_mark_with_no_talks_does_not_hold_the_file() {
        let mut country = Fx::country();
        country.clubs[0]
            .transfer_plan
            .set_monitoring_status_for_player(MOVED_ON, ScoutMonitoringStatus::Negotiating);

        Fx::review(&mut country);

        assert!(
            country.clubs[0]
                .transfer_plan
                .monitorings_for_player(MOVED_ON)
                .is_empty()
        );
    }

    #[test]
    fn the_review_runs_on_mondays_only() {
        let mut country = Fx::country();
        let pool = Fx::pool();
        let index = WorldPoolIndex::build(&pool);

        ScoutingPass::review_files(
            &mut country,
            WorldPool::new(&pool, &index),
            &MarketMap::default(),
            Fx::monday().succ_opt().unwrap(),
        );

        assert_eq!(
            country.clubs[0]
                .transfer_plan
                .monitorings_for_player(MOVED_ON)
                .len(),
            1
        );
    }
}
