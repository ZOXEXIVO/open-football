use chrono::{Datelike, NaiveDate};
use log::debug;

use crate::transfers::ScoutingRegion;
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::recruitment::{ScoutMonitoringSource, ScoutPlayerMonitoring};
use crate::transfers::pipeline::scouting_config::ScoutingConfig;
use crate::transfers::pipeline::{
    DetailedScoutingReport, PlayerObservation, ReportRiskFlag, ScoutMatchAssignment,
    ScoutingAssignment, ScoutingRecommendation, TransferNeedPriority, TransferRequest,
    TransferRequestStatus,
};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::IntegerUtils;
use crate::{
    ClubPhilosophy, Country, Person, PlayerStatusType, StaffEventType, StaffPosition, TeamType,
};

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
    region: Option<crate::transfers::ScoutingRegion>,
}

/// Apply a monitoring update against a `ClubTransferPlan`. Either
/// upserts an existing row for `(scout_staff_id, player_id)` or
/// creates a fresh one. Pure-state mutation — no side effects beyond
/// the plan.
fn apply_monitoring_update(
    plan: &mut crate::transfers::pipeline::ClubTransferPlan,
    update: MonitoringUpdate,
    date: NaiveDate,
) {
    if let Some(existing) = plan.find_monitoring_mut(update.scout_staff_id, update.player_id) {
        // Refresh linkage if monitoring originated from a different
        // request and now matches an active one — prefer the newer
        // active linkage so meeting agendas stay coherent.
        if update.transfer_request_id.is_some() && existing.transfer_request_id.is_none() {
            existing.transfer_request_id = update.transfer_request_id;
        }
        if update.origin_assignment_id.is_some() && existing.origin_assignment_id.is_none() {
            existing.origin_assignment_id = update.origin_assignment_id;
        }
        if existing.region.is_none() {
            existing.region = update.region;
        }
        existing.record_observation(
            update.assessed_ability,
            update.assessed_potential,
            update.confidence,
            update.role_fit,
            update.estimated_value,
            update.risk_flags,
            date,
            update.is_match,
        );
    } else {
        let id = plan.next_monitoring_id();
        let mut row = ScoutPlayerMonitoring::new(
            id,
            update.scout_staff_id,
            update.player_id,
            update.source,
            date,
        );
        row.transfer_request_id = update.transfer_request_id;
        row.origin_assignment_id = update.origin_assignment_id;
        row.region = update.region;
        row.record_observation(
            update.assessed_ability,
            update.assessed_potential,
            update.confidence,
            update.role_fit,
            update.estimated_value,
            update.risk_flags,
            date,
            update.is_match,
        );
        plan.scout_monitoring.push(row);
    }
}

impl PipelineProcessor {
    pub fn assign_scouts(country: &mut Country, _date: NaiveDate) {
        let mut actions: Vec<ScoutAssignmentAction> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            let assigned_request_ids: Vec<u32> = plan
                .scouting_assignments
                .iter()
                .map(|a| a.transfer_request_id)
                .collect();

            let pending_requests: Vec<&TransferRequest> = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    r.status == TransferRequestStatus::Pending
                        && !assigned_request_ids.contains(&r.id)
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
            let mut team_scores: Vec<(u32, u32, u32, usize)> = Vec::new(); // (team_id, club_id, team_idx_for_ref, score)

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

            // Sort by score descending
            team_scores.sort_by(|a, b| b.3.cmp(&a.3));

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
        let config = ScoutingConfig::default();
        let mut observations: Vec<MatchScoutingObservationResult> = Vec::new();
        let mut reports: Vec<ScoutingReportResult> = Vec::new();
        let mut attended_updates: Vec<(u32, u32, NaiveDate)> = Vec::new(); // (club_id, team_id, date)
        let mut staff_events: Vec<(u32, u32, StaffEventType)> = Vec::new(); // (club_id, staff_id, event)
        let mut monitoring_updates: Vec<MonitoringUpdate> = Vec::new();

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
                    Self::get_scout_skills(club, match_assignment.scout_staff_id);

                // Mark attendance
                attended_updates.push((club.id, match_assignment.target_team_id, current_date));
                staff_events.push((
                    club.id,
                    match_assignment.scout_staff_id,
                    StaffEventType::MatchObserved,
                ));

                // Observe all players on the target team
                for player in &target_team.players.players {
                    let player_pos_group = player.position().position_group();
                    let player_age = player.age(current_date);
                    let match_rating = player.statistics.average_rating;

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
                        None => continue,
                    };

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

                    let growth_potential = Self::estimate_growth_potential(
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
                        let estimated_value =
                            PlayerValuationCalculator::calculate_value_with_price_level(
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
                        let risk_flags = Self::evaluate_risk_flags(
                            player.player_attributes.is_injured,
                            player.skills.mental.determination,
                            player_age,
                            contract_months,
                            player.player_attributes.world_reputation,
                            Self::club_world_reputation(club),
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
            }
        }

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
                apply_monitoring_update(&mut club.transfer_plan, update, current_date);
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

        debug!("process_match_scouting: completed match-day observations");
    }

    // ============================================================
    // Step 4: Scouting Observations
    // ============================================================

    /// Collect player summaries from a country for cross-country scouting.
    pub fn collect_player_pool(country: &Country, date: NaiveDate) -> Vec<PlayerSummary> {
        let price_level = country.settings.pricing.price_level;
        let country_id = country.id;
        let country_reputation = country.reputation;
        let mut players = Vec::new();

        for club in &country.clubs {
            // Seller market context once per club — flat 0/0 used to
            // drag every domestic player to the same baseline regardless
            // of the league/club they actually played for.
            let (seller_league_rep, seller_club_rep) =
                PlayerValuationCalculator::seller_context(country, club);

            for team in &club.teams.teams {
                for player in &team.players.players {
                    if player.is_on_loan() {
                        continue;
                    }
                    let value = PlayerValuationCalculator::calculate_value_with_price_level(
                        player,
                        date,
                        price_level,
                        seller_league_rep,
                        seller_club_rep,
                    );
                    let statuses = player.statuses.get();
                    let (contract_months_remaining, salary) = player
                        .contract
                        .as_ref()
                        .map(|c| {
                            let days = (c.expiration - date).num_days().max(0);
                            ((days / 30).min(i16::MAX as i64) as i16, c.salary)
                        })
                        .unwrap_or((0, 0));
                    players.push(PlayerSummary {
                        player_id: player.id,
                        club_id: club.id,
                        country_id,
                        continent_id: country.continent_id,
                        country_code: country.code.clone(),
                        player_name: player.full_name.to_string(),
                        club_name: club.name.clone(),
                        position: player.position(),
                        position_group: player.position().position_group(),
                        age: player.age(date),
                        estimated_value: value.amount,
                        is_listed: statuses.contains(&PlayerStatusType::Lst),
                        is_loan_listed: statuses.contains(&PlayerStatusType::Loa),
                        skill_ability: player
                            .skills
                            .calculate_ability_for_position(player.position()),
                        average_rating: player.statistics.average_rating,
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
                        is_injured: player.player_attributes.is_injured,
                        contract_months_remaining,
                        salary,
                    });
                }
            }
        }

        players
    }

    pub fn process_scouting(
        country: &mut Country,
        foreign_players: &[PlayerSummary],
        date: NaiveDate,
    ) {
        let country_id = country.id;
        let country_reputation = country.reputation;
        // Single source of truth for observation/error/recommendation/risk-flag tuning.
        let config = ScoutingConfig::default();

        // Reuse collect_player_pool for the domestic pool — the body of this
        // loop used to be a copy-paste of that function, doubling the work
        // that already runs once per country to build the shared foreign
        // pool. Now it runs once.
        let all_players: Vec<PlayerSummary> = Self::collect_player_pool(&*country, date);

        let mut observations: Vec<ScoutingObservationResult> = Vec::new();
        let mut reports: Vec<ScoutingReportResult> = Vec::new();
        let mut staff_events: Vec<(u32, u32, StaffEventType)> = Vec::new();
        let mut familiarity_events: Vec<(u32, u32, ScoutingRegion)> = Vec::new();
        let mut rejected_events: Vec<(u32, u32)> = Vec::new(); // (club_id, player_id)
        let mut wanted_player_ids: Vec<u32> = Vec::new();
        let mut monitoring_updates: Vec<MonitoringUpdate> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;

            for assignment in &plan.scouting_assignments {
                if assignment.completed {
                    continue;
                }

                let (judging_ability, judging_potential) =
                    if let Some(scout_id) = assignment.scout_staff_id {
                        Self::get_scout_skills(club, scout_id)
                    } else {
                        let d = config.observation.default_judging_when_no_scout;
                        (d, d)
                    };

                // Borrow the scout's knowledge struct once — we need both
                // known_regions (slice) and familiarity (per-region lookup).
                let scout_knowledge = assignment
                    .scout_staff_id
                    .and_then(|sid| {
                        club.teams
                            .iter()
                            .flat_map(|t| t.staffs.iter())
                            .find(|s| s.id == sid)
                    })
                    .map(|s| &s.staff_attributes.knowledge);

                const EMPTY_REGIONS: &[ScoutingRegion] = &[];
                let scout_known_regions: &[ScoutingRegion] = scout_knowledge
                    .map(|k| k.known_regions.as_slice())
                    .unwrap_or(EMPTY_REGIONS);

                let observe_chance = config.daily_observation_chance(judging_ability);
                if IntegerUtils::random(0, 100) > observe_chance {
                    continue;
                }

                if let Some(scout_id) = assignment.scout_staff_id {
                    staff_events.push((club.id, scout_id, StaffEventType::PlayerScouted));
                }

                // Find matching players from OTHER clubs (domestic + foreign known regions)
                let target_group = assignment.target_position.position_group();
                let philosophy = &club.philosophy;

                // DevelopAndSell clubs widen the net for young promising players
                let (age_min, age_max, ability_floor) = match philosophy {
                    ClubPhilosophy::DevelopAndSell => {
                        let youth_floor = assignment.min_ability.saturating_sub(20);
                        (
                            assignment.preferred_age_min.min(16),
                            assignment.preferred_age_max,
                            youth_floor,
                        )
                    }
                    ClubPhilosophy::SignToCompete => (
                        assignment.preferred_age_min,
                        assignment.preferred_age_max,
                        assignment.min_ability,
                    ),
                    _ => (
                        assignment.preferred_age_min,
                        assignment.preferred_age_max,
                        assignment.min_ability,
                    ),
                };

                let player_filter = |p: &&PlayerSummary| -> bool {
                    if p.club_id == club.id
                        || p.position_group != target_group
                        || club.is_rival(p.club_id)
                    {
                        return false;
                    }
                    if club.transfer_plan.is_rejected(p.player_id, date) {
                        return false;
                    }
                    let effective_min =
                        if p.age <= 21 && matches!(philosophy, ClubPhilosophy::DevelopAndSell) {
                            ability_floor
                        } else {
                            assignment.min_ability
                        };
                    p.age >= age_min && p.age <= age_max && p.skill_ability >= effective_min
                };

                // Domestic players (always visible)
                let mut matching: Vec<&PlayerSummary> =
                    all_players.iter().filter(player_filter).collect();

                // Foreign players from scout's known regions (region-based matching)
                // Only scout leagues with equal or lower reputation than our own country.
                // e.g. Italian clubs can scout Nigeria, but Nigerian clubs cannot scout Serie A.
                if !scout_known_regions.is_empty() {
                    let foreign_matching: Vec<&PlayerSummary> = foreign_players
                        .iter()
                        .filter(|p| p.country_reputation <= country_reputation)
                        .filter(|p| {
                            let player_region =
                                ScoutingRegion::from_country(p.continent_id, &p.country_code);
                            scout_known_regions.contains(&player_region)
                        })
                        .filter(player_filter)
                        .collect();
                    matching.extend(foreign_matching);
                }

                if matching.is_empty() {
                    continue;
                }

                // Data-first pre-filter: the club's data department narrows the
                // candidate pool from "everyone who matches position/age/ability"
                // to "people the numbers say deserve an eye-test." Higher data
                // skill = tighter pool with less noise; low skill ≈ random.
                // This is what real clubs do — Opta/Wyscout shortlists come
                // first, scouts watch the narrowed list in person.
                let data_skill = Self::club_data_analysis_skill(club);
                if let Some(target_pool) = config.data_prefilter_target(matching.len(), data_skill)
                {
                    let noise = config.data_prefilter_noise(data_skill);
                    let mut scored: Vec<(&PlayerSummary, f32)> = matching
                        .iter()
                        .map(|p| {
                            let score = Self::player_data_score(p);
                            let jitter = IntegerUtils::random(-noise, noise) as f32;
                            (*p, score + jitter)
                        })
                        .collect();
                    scored
                        .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    matching = scored
                        .into_iter()
                        .take(target_pool)
                        .map(|(p, _)| p)
                        .collect();
                }

                let obs_per_day = config.observations_per_day(judging_ability);

                for _obs_round in 0..obs_per_day.min(matching.len()) {
                    // Configurable re-observe vs discover chance: deepen
                    // existing knowledge most of the time, widen the pool
                    // occasionally. Default ~60/40.
                    let re_observe_chance = config.observation.re_observe_chance_pct;
                    let already_observed_ids: Vec<u32> = assignment
                        .observations
                        .iter()
                        .map(|o| o.player_id)
                        .collect();

                    let target = if !already_observed_ids.is_empty()
                        && IntegerUtils::random(0, 100) < re_observe_chance
                    {
                        // Prefer re-observing a known player
                        matching
                            .iter()
                            .find(|p| already_observed_ids.contains(&p.player_id))
                            .or_else(|| matching.first())
                            .unwrap()
                    } else {
                        // Discover new player — reputation-weighted selection
                        // Famous players are more visible to scouts (media coverage, word of mouth)
                        let new_players: Vec<&&PlayerSummary> = matching
                            .iter()
                            .filter(|p| !already_observed_ids.contains(&p.player_id))
                            .collect();
                        if !new_players.is_empty() {
                            Self::pick_reputation_weighted(&new_players)
                        } else {
                            Self::pick_reputation_weighted(&matching.iter().collect::<Vec<_>>())
                        }
                    };

                    let existing_obs = assignment
                        .observations
                        .iter()
                        .find(|o| o.player_id == target.player_id);
                    let obs_count = existing_obs.map(|o| o.observation_count).unwrap_or(0);

                    // Region penalty blends structural knowledge (in known_regions?)
                    // with empirical experience (familiarity, 0-100). A veteran
                    // scout who's been scouting a region for years is sharper than
                    // a brand-new assignee, even to a "known" region.
                    let target_region =
                        ScoutingRegion::from_country(target.continent_id, &target.country_code);
                    let is_domestic = target.country_id == country_id;
                    let is_known_region = scout_known_regions.contains(&target_region);
                    let familiarity = scout_knowledge
                        .map(|k| k.familiarity_for(target_region))
                        .unwrap_or(0);
                    let region_penalty =
                        config.region_penalty(is_domestic, is_known_region, familiarity);

                    let ability_error = config.effective_error(
                        judging_ability,
                        obs_count as u8,
                        region_penalty,
                        false,
                    );
                    let potential_error = config.effective_error(
                        judging_potential,
                        obs_count as u8,
                        region_penalty,
                        false,
                    );

                    // Assess ability from visible skills, boosted by match performance
                    let performance_bonus =
                        config.performance_bonus(target.appearances, target.average_rating);

                    let assessed_ability = (target.skill_ability as i32
                        + performance_bonus
                        + IntegerUtils::random(-ability_error, ability_error))
                    .clamp(1, 200) as u8;

                    // Estimate potential from age, mental attributes, and current skill level
                    // Young players with strong mentals (determination, work rate) suggest higher ceiling
                    let growth_potential = Self::estimate_growth_potential(
                        target.age,
                        target.determination,
                        target.work_rate,
                        target.composure,
                        target.anticipation,
                        target.skill_ability,
                    );
                    let assessed_potential = (target.skill_ability as i32
                        + growth_potential as i32
                        + IntegerUtils::random(-potential_error, potential_error))
                    .clamp(1, 200) as u8;

                    let is_new = !assignment.has_observation_for(target.player_id);

                    // Skip if we already queued an observation for this player this round
                    if observations.iter().any(|o| {
                        o.club_id == club.id
                            && o.assignment_id == assignment.id
                            && o.player_id == target.player_id
                    }) {
                        continue;
                    }

                    observations.push(ScoutingObservationResult {
                        club_id: club.id,
                        assignment_id: assignment.id,
                        player_id: target.player_id,
                        assessed_ability,
                        assessed_potential,
                        is_new,
                    });

                    if let Some(scout_id) = assignment.scout_staff_id {
                        if target.country_id != country_id {
                            familiarity_events.push((club.id, scout_id, target_region));
                        }
                    }

                    if is_new && !wanted_player_ids.contains(&target.player_id) {
                        wanted_player_ids.push(target.player_id);
                    }

                    let final_obs_count = obs_count + 1;
                    let confidence = config.pool_report_confidence(final_obs_count as u8);
                    let youth_bonus =
                        config.youth_bonus(target.age, assessed_ability, assessed_potential);
                    let stats_bonus = config.stats_bonus(target.appearances, target.average_rating);
                    let effective_ability = assessed_ability as i16 + youth_bonus + stats_bonus;
                    let recommendation = config.recommendation_for(
                        effective_ability,
                        assessed_ability,
                        assessed_potential,
                        assignment.min_ability,
                    );

                    let role_fit_now = assignment.role_profile.fit(
                        target.technical_avg,
                        target.mental_avg,
                        target.physical_avg,
                    );
                    let risk_flags_now = Self::evaluate_risk_flags(
                        target.is_injured,
                        target.determination,
                        target.age,
                        target.contract_months_remaining,
                        target.world_reputation,
                        Self::club_world_reputation(club),
                    );

                    // Always update the monitoring row when a real scout
                    // is on this assignment — even if the recommendation
                    // is Pass, the scout has formed an opinion that the
                    // recruitment meeting will see.
                    if let Some(scout_id) = assignment.scout_staff_id {
                        monitoring_updates.push(MonitoringUpdate {
                            club_id: club.id,
                            scout_staff_id: scout_id,
                            player_id: target.player_id,
                            source: ScoutMonitoringSource::TransferRequest,
                            transfer_request_id: Some(assignment.transfer_request_id),
                            origin_assignment_id: Some(assignment.id),
                            assessed_ability,
                            assessed_potential,
                            confidence,
                            role_fit: role_fit_now,
                            estimated_value: target.estimated_value,
                            risk_flags: risk_flags_now.clone(),
                            is_match: false,
                            region: Some(target_region),
                        });
                    }

                    if recommendation == ScoutingRecommendation::Pass {
                        rejected_events.push((club.id, target.player_id));
                    } else {
                        reports.push(ScoutingReportResult {
                            club_id: club.id,
                            report: DetailedScoutingReport {
                                player_id: target.player_id,
                                assignment_id: assignment.id,
                                assessed_ability,
                                assessed_potential,
                                confidence,
                                estimated_value: target.estimated_value,
                                recommendation,
                                role_fit: role_fit_now,
                                risk_flags: risk_flags_now,
                            },
                            assignment_id: assignment.id,
                        });
                    }
                }
            }
        }

        // Pass 2: Apply observations and reports
        for obs in observations {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == obs.club_id) {
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
                    } else if let Some(existing) = assignment.find_observation_mut(obs.player_id) {
                        existing.add_observation(
                            obs.assessed_ability,
                            obs.assessed_potential,
                            date,
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

        // Apply monitoring updates — pool-context scouting.
        for update in monitoring_updates {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == update.club_id) {
                apply_monitoring_update(&mut club.transfer_plan, update, date);
            }
        }

        // Set Wnt status on newly scouted players
        for player_id in &wanted_player_ids {
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) =
                        team.players.players.iter_mut().find(|p| p.id == *player_id)
                    {
                        if !player.statuses.get().contains(&PlayerStatusType::Wnt) {
                            player.statuses.add(date, PlayerStatusType::Wnt);
                        }
                    }
                }
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

        // Accrue per-region familiarity for scouts who observed foreign players
        for (club_id, staff_id, region) in familiarity_events {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                for team in &mut club.teams.teams {
                    if let Some(staff) = team.staffs.find_mut(staff_id) {
                        staff.staff_attributes.knowledge.accrue_region_day(region);
                        break;
                    }
                }
            }
        }

        // Commit rejection memory — a Pass recommendation blocks re-scouting
        // for the configured window, spanning at least the current window
        // and (typically) the next one.
        let rejection_months = config.assignment.rejection_memory_months;
        for (club_id, player_id) in rejected_events {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                club.transfer_plan
                    .reject_player(player_id, date, rejection_months);
            }
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
        if date.weekday() != chrono::Weekday::Mon {
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
        let mut current_ability: std::collections::HashMap<u32, u8> =
            std::collections::HashMap::new();
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
}
