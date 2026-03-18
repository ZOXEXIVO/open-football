use chrono::NaiveDate;
use log::debug;

use crate::transfers::pipeline::{
    DetailedScoutingReport, PlayerObservation, ScoutMatchAssignment, ScoutingAssignment,
    ScoutingRecommendation, TransferNeedPriority, TransferRequest, TransferRequestStatus,
};
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::staff_resolver::StaffResolver;
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::IntegerUtils;
use crate::{
    ClubPhilosophy, Country, Person, PlayerFieldPositionGroup, PlayerStatusType,
    StaffEventType, TeamType,
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
            let resolved = StaffResolver::resolve(&club.teams.teams[0].staffs);

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

            let resolved = StaffResolver::resolve(&club.teams.teams[0].staffs);
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

            let max_assignments = available_scouts.len().min(3);

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
                let linked_ids: Vec<u32> = active_assignments
                    .iter()
                    .map(|a| a.id)
                    .collect();

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
        let mut observations: Vec<MatchScoutingObservationResult> = Vec::new();
        let mut reports: Vec<ScoutingReportResult> = Vec::new();
        let mut attended_updates: Vec<(u32, u32, NaiveDate)> = Vec::new(); // (club_id, team_id, date)
        let mut staff_events: Vec<(u32, u32, StaffEventType)> = Vec::new(); // (club_id, staff_id, event)

        // Pass 1: Immutable reads
        for club in &country.clubs {
            let plan = &club.transfer_plan;

            for match_assignment in &plan.scout_match_assignments {
                // Find the target team and check if it played today
                let target_team = country.clubs.iter()
                    .find(|c| c.id == match_assignment.target_club_id)
                    .and_then(|c| c.teams.teams.iter().find(|t| t.id == match_assignment.target_team_id));

                let target_team = match target_team {
                    Some(t) => t,
                    None => continue,
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
                staff_events.push((club.id, match_assignment.scout_staff_id, StaffEventType::MatchObserved));

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

                    // Calculate assessed ability/potential with 40% less error than pool scanning
                    let existing_obs = assignment.observations.iter()
                        .find(|o| o.player_id == player.id);
                    let obs_count = existing_obs.map(|o| o.observation_count).unwrap_or(0);
                    let sqrt_count = ((obs_count + 1) as f32).sqrt();

                    let base_ability_error = (20i16 - judging_ability as i16).max(1) as f32;
                    let base_potential_error = (20i16 - judging_potential as i16).max(1) as f32;
                    // 40% less error for match-context observations
                    let ability_error = ((base_ability_error * 0.6) / sqrt_count) as i32;
                    let potential_error = ((base_potential_error * 0.6) / sqrt_count) as i32;

                    // Assess from visible skills and match performance, not hidden CA/PA
                    let skill_ability = player.skills.calculate_ability_for_position(player.position());
                    let match_bonus = if match_rating > 7.5 { 5i32 }
                        else if match_rating > 7.0 { 3 }
                        else if match_rating > 6.5 { 1 }
                        else if match_rating < 5.5 { -3 }
                        else { 0 };

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

                    // Generate report at 2+ observations
                    let final_obs_count = obs_count + 1;
                    if final_obs_count >= 2 {
                        let confidence = (1.0 - (0.5 / (final_obs_count as f32 + 1.0))).min(1.0);

                        // Match rating influences recommendation tier
                        let rating_boost = match_rating > 7.0;
                        let rating_penalty = match_rating < 5.5;

                        let recommendation = if rating_penalty {
                            // Low match rating downgrades
                            if assessed_ability >= assignment.min_ability {
                                ScoutingRecommendation::Consider
                            } else {
                                ScoutingRecommendation::Pass
                            }
                        } else if rating_boost
                            && assessed_ability as i16 >= assignment.min_ability as i16 + 5
                            && assessed_potential > assessed_ability
                        {
                            ScoutingRecommendation::StrongBuy
                        } else if assessed_ability as i16 >= assignment.min_ability as i16 + 10
                            && assessed_potential > assessed_ability + 5
                        {
                            ScoutingRecommendation::StrongBuy
                        } else if assessed_ability >= assignment.min_ability
                            && assessed_potential >= assessed_ability
                        {
                            ScoutingRecommendation::Buy
                        } else if assessed_ability >= assignment.min_ability.saturating_sub(5) {
                            ScoutingRecommendation::Consider
                        } else {
                            ScoutingRecommendation::Pass
                        };

                        if recommendation != ScoutingRecommendation::Pass {
                            let estimated_value = PlayerValuationCalculator::calculate_value_with_price_level(
                                player,
                                current_date,
                                country.settings.pricing.price_level,
                                0, 0,
                            );

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
                        let mut new_obs = crate::transfers::pipeline::PlayerObservation::new(
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
                if !club
                    .transfer_plan
                    .scouting_reports
                    .iter()
                    .any(|r| r.player_id == report.report.player_id && r.assignment_id == report.assignment_id)
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

        // Push staff events for scouts
        for (club_id, staff_id, event_type) in staff_events {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                for team in &mut club.teams.teams {
                    if let Some(staff) = team.staffs.staffs.iter_mut().find(|s| s.id == staff_id) {
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
        let mut players = Vec::new();

        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    if player.is_on_loan() {
                        continue;
                    }
                    let value = PlayerValuationCalculator::calculate_value_with_price_level(
                        player, date, price_level, 0, 0,
                    );
                    let statuses = player.statuses.get();
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
                        skill_ability: player.skills.calculate_ability_for_position(player.position()),
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
                    });
                }
            }
        }

        players
    }

    pub fn process_scouting(country: &mut Country, foreign_players: &[PlayerSummary], date: NaiveDate) {
        let price_level = country.settings.pricing.price_level;
        let country_id = country.id;

        // Domestic players
        let mut all_players: Vec<PlayerSummary> = Vec::new();

        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    if player.is_on_loan() {
                        continue;
                    }

                    let value = PlayerValuationCalculator::calculate_value_with_price_level(
                        player,
                        date,
                        price_level,
                        0, 0,
                    );
                    let statuses = player.statuses.get();
                    all_players.push(PlayerSummary {
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
                        skill_ability: player.skills.calculate_ability_for_position(player.position()),
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
                    });
                }
            }
        }

        let mut observations: Vec<ScoutingObservationResult> = Vec::new();
        let mut reports: Vec<ScoutingReportResult> = Vec::new();
        let mut staff_events: Vec<(u32, u32, StaffEventType)> = Vec::new();
        let mut wanted_player_ids: Vec<u32> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;

            for assignment in &plan.scouting_assignments {
                if assignment.completed {
                    continue;
                }

                let (judging_ability, judging_potential) = if let Some(scout_id) = assignment.scout_staff_id {
                    Self::get_scout_skills(club, scout_id)
                } else {
                    (8, 8)
                };

                // Get this scout's known regions for cross-country scouting
                let scout_known_regions: Vec<crate::transfers::ScoutingRegion> = assignment.scout_staff_id
                    .and_then(|sid| {
                        club.teams.teams.iter()
                            .flat_map(|t| &t.staffs.staffs)
                            .find(|s| s.id == sid)
                    })
                    .map(|s| s.staff_attributes.knowledge.known_regions.clone())
                    .unwrap_or_default();

                let observe_chance = 60 + (judging_ability as i32 / 2);
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
                        (assignment.preferred_age_min.min(16), assignment.preferred_age_max, youth_floor)
                    }
                    ClubPhilosophy::SignToCompete => {
                        (assignment.preferred_age_min, assignment.preferred_age_max, assignment.min_ability)
                    }
                    _ => {
                        (assignment.preferred_age_min, assignment.preferred_age_max, assignment.min_ability)
                    }
                };

                let player_filter = |p: &&PlayerSummary| -> bool {
                    if p.club_id == club.id || p.position_group != target_group
                        || club.is_rival(p.club_id) {
                        return false;
                    }
                    let effective_min = if p.age <= 21 && matches!(philosophy, ClubPhilosophy::DevelopAndSell) {
                        ability_floor
                    } else {
                        assignment.min_ability
                    };
                    p.age >= age_min
                        && p.age <= age_max
                        && p.skill_ability >= effective_min
                };

                // Domestic players (always visible)
                let mut matching: Vec<&PlayerSummary> = all_players
                    .iter()
                    .filter(player_filter)
                    .collect();

                // Foreign players from scout's known regions (region-based matching)
                if !scout_known_regions.is_empty() {
                    let foreign_matching: Vec<&PlayerSummary> = foreign_players
                        .iter()
                        .filter(|p| {
                            let player_region = crate::transfers::ScoutingRegion::from_country(
                                p.continent_id, &p.country_code,
                            );
                            scout_known_regions.contains(&player_region)
                        })
                        .filter(player_filter)
                        .collect();
                    matching.extend(foreign_matching);
                }

                if matching.is_empty() {
                    continue;
                }

                // Scouts observe 2-3 players per day (not just 1)
                let obs_per_day = 2 + (judging_ability as usize / 10); // 2-3

                for _obs_round in 0..obs_per_day.min(matching.len()) {
                    // 60% chance to re-observe a previously seen player (deepen knowledge)
                    // 40% chance to discover a new player
                    let already_observed_ids: Vec<u32> = assignment
                        .observations
                        .iter()
                        .map(|o| o.player_id)
                        .collect();

                    let target = if !already_observed_ids.is_empty()
                        && IntegerUtils::random(0, 100) < 60
                    {
                        // Prefer re-observing a known player
                        matching
                            .iter()
                            .find(|p| already_observed_ids.contains(&p.player_id))
                            .or_else(|| matching.first())
                            .unwrap()
                    } else {
                        // Discover new player
                        let new_players: Vec<&&PlayerSummary> = matching
                            .iter()
                            .filter(|p| !already_observed_ids.contains(&p.player_id))
                            .collect();
                        if !new_players.is_empty() {
                            let idx = (IntegerUtils::random(0, new_players.len() as i32) as usize)
                                .min(new_players.len() - 1);
                            new_players[idx]
                        } else {
                            let idx = (IntegerUtils::random(0, matching.len() as i32) as usize)
                                .min(matching.len() - 1);
                            matching[idx]
                        }
                    };

                    let existing_obs = assignment
                        .observations
                        .iter()
                        .find(|o| o.player_id == target.player_id);
                    let obs_count = existing_obs.map(|o| o.observation_count).unwrap_or(0);
                    let sqrt_count = ((obs_count + 1) as f32).sqrt();

                    // Foreign players in unknown regions have +50% assessment error
                    let region_penalty = if target.country_id != country_id {
                        let target_region = crate::transfers::ScoutingRegion::from_country(
                            target.continent_id, &target.country_code,
                        );
                        if scout_known_regions.contains(&target_region) {
                            1.0 // Known region — normal accuracy
                        } else {
                            1.5 // Unknown region — 50% more error
                        }
                    } else {
                        1.0 // Domestic — always accurate
                    };

                    let base_ability_error = (20i16 - judging_ability as i16).max(1) as f32 * region_penalty;
                    let base_potential_error = (20i16 - judging_potential as i16).max(1) as f32 * region_penalty;
                    let ability_error = (base_ability_error / sqrt_count) as i32;
                    let potential_error = (base_potential_error / sqrt_count) as i32;

                    // Assess ability from visible skills, boosted by match performance
                    let performance_bonus = if target.appearances >= 10 && target.average_rating > 7.0 {
                        3i32
                    } else if target.appearances >= 5 && target.average_rating > 6.5 {
                        1
                    } else if target.average_rating > 0.0 && target.average_rating < 5.5 {
                        -2
                    } else {
                        0
                    };

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

                    if is_new && !wanted_player_ids.contains(&target.player_id) {
                        wanted_player_ids.push(target.player_id);
                    }

                    // Generate report after just 1 observation (with lower confidence)
                    let final_obs_count = obs_count + 1;
                    let confidence = if final_obs_count == 1 {
                        0.4
                    } else {
                        1.0 - (1.0 / (final_obs_count as f32 + 1.0))
                    };

                    // Young players with high potential gap get boosted recommendations
                    let youth_bonus: i16 = if target.age <= 21 && assessed_potential > assessed_ability + 15 {
                        10
                    } else if target.age <= 23 && assessed_potential > assessed_ability + 10 {
                        5
                    } else {
                        0
                    };

                    // Strong match stats boost recommendation
                    let stats_bonus: i16 = if target.appearances >= 10 && target.average_rating >= 7.0 {
                        5
                    } else if target.appearances >= 5 && target.average_rating >= 6.5 {
                        2
                    } else {
                        0
                    };

                    let effective_ability = assessed_ability as i16 + youth_bonus + stats_bonus;

                    let recommendation =
                        if effective_ability >= assignment.min_ability as i16 + 10
                            && assessed_potential > assessed_ability + 5
                        {
                            ScoutingRecommendation::StrongBuy
                        } else if effective_ability >= assignment.min_ability as i16
                            && assessed_potential >= assessed_ability
                        {
                            ScoutingRecommendation::Buy
                        } else if effective_ability >= assignment.min_ability as i16 - 5 {
                            ScoutingRecommendation::Consider
                        } else {
                            ScoutingRecommendation::Pass
                        };

                    if recommendation != ScoutingRecommendation::Pass {
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
                        assignment.observations.push(
                            crate::transfers::pipeline::PlayerObservation::new(
                                obs.player_id,
                                obs.assessed_ability,
                                obs.assessed_potential,
                                date,
                            ),
                        );
                    } else if let Some(existing) = assignment.find_observation_mut(obs.player_id) {
                        existing.add_observation(obs.assessed_ability, obs.assessed_potential, date);
                    }
                }
            }
        }

        for report in reports {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == report.club_id) {
                if !club
                    .transfer_plan
                    .scouting_reports
                    .iter()
                    .any(|r| r.player_id == report.report.player_id && r.assignment_id == report.assignment_id)
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
        }

        // Set Wnt status on newly scouted players
        for player_id in &wanted_player_ids {
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) = team.players.players.iter_mut().find(|p| p.id == *player_id) {
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
                    if let Some(staff) = team.staffs.staffs.iter_mut().find(|s| s.id == staff_id) {
                        staff.add_event(event_type);
                        break;
                    }
                }
            }
        }
    }
}
