use chrono::NaiveDate;

use crate::transfers::pipeline::{
    RecommendationSource, RecommendationType, ShortlistCandidate,
    ShortlistCandidateStatus, StaffRecommendation, TransferNeedPriority,
    TransferNeedReason, TransferRequest, TransferRequestStatus,
};
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::staff_resolver::StaffResolver;
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::IntegerUtils;
use crate::{
    Country, Person, PlayerFieldPositionGroup, PlayerPositionType, PlayerStatusType,
    ReputationLevel,
};

impl PipelineProcessor {
    pub fn generate_staff_recommendations(country: &mut Country, date: NaiveDate) {
        // Only runs weekly (same schedule as should_evaluate)
        if !Self::should_evaluate(date) {
            return;
        }

        let is_january = Self::is_january_window(date);
        let price_level = country.settings.pricing.price_level;

        // Pass 1: Build player snapshots across all clubs
        #[allow(dead_code)]
        struct PlayerSnapshot {
            id: u32,
            club_id: u32,
            position: PlayerPositionType,
            position_group: PlayerFieldPositionGroup,
            ability: u8,           // skill-based, not CA
            estimated_potential: u8, // estimated from age + mentals, not PA
            age: u8,
            estimated_value: f64,
            contract_months_remaining: u32,
            club_in_debt: bool,
            parent_club_reputation: ReputationLevel,
            is_loan_listed: bool,
            // Observable performance
            average_rating: f32,
            appearances: u16,
            is_transfer_protected: bool,
        }

        let mut all_snapshots: Vec<PlayerSnapshot> = Vec::new();

        for club in &country.clubs {
            let club_in_debt = club.finance.balance.balance < 0;
            let rep_level = club
                .teams
                .teams
                .first()
                .map(|t| t.reputation.level())
                .unwrap_or(ReputationLevel::Amateur);

            for team in &club.teams.teams {
                for player in &team.players.players {
                    let value = PlayerValuationCalculator::calculate_value_with_price_level(
                        player,
                        date,
                        price_level,
                        0, 0,
                    );
                    let contract_months = player
                        .contract
                        .as_ref()
                        .map(|c| {
                            let days = (c.expiration - date).num_days().max(0) as u32;
                            days / 30
                        })
                        .unwrap_or(0);

                    let statuses = player.statuses.get();

                    let skill_ability = player.skills.calculate_ability_for_position(player.position());
                    let player_age = player.age(date);
                    let estimated_potential = skill_ability + Self::estimate_growth_potential(
                        player_age,
                        player.skills.mental.determination,
                        player.skills.mental.work_rate,
                        player.skills.mental.composure,
                        player.skills.mental.anticipation,
                        skill_ability,
                    );

                    all_snapshots.push(PlayerSnapshot {
                        id: player.id,
                        club_id: club.id,
                        position: player.position(),
                        position_group: player.position().position_group(),
                        ability: skill_ability,
                        estimated_potential,
                        age: player_age,
                        estimated_value: value.amount,
                        contract_months_remaining: contract_months,
                        club_in_debt,
                        parent_club_reputation: rep_level.clone(),
                        is_loan_listed: statuses.contains(&PlayerStatusType::Loa),
                        average_rating: player.statistics.average_rating,
                        appearances: player.statistics.total_games(),
                        is_transfer_protected: player.is_transfer_protected(date),
                    });
                }
            }
        }

        // Collect recommendations per club
        struct RecommendationAction {
            club_id: u32,
            recommendation: StaffRecommendation,
        }

        let mut actions: Vec<RecommendationAction> = Vec::new();

        for club in &country.clubs {
            if club.teams.teams.is_empty() {
                continue;
            }
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            // Cap: 6 recommendations per club per window
            if plan.staff_recommendations.len() >= 6 {
                continue;
            }

            let team = &club.teams.teams[0];
            let resolved = StaffResolver::resolve(&team.staffs);

            let avg_ability: u8 = if !team.players.players.is_empty() {
                let total: u32 = team
                    .players
                    .players
                    .iter()
                    .map(|p| p.player_attributes.current_ability as u32)
                    .sum();
                (total / team.players.players.len() as u32) as u8
            } else {
                50
            };

            let club_rep = team.reputation.level();

            let already_recommended: Vec<u32> = plan
                .staff_recommendations
                .iter()
                .map(|r| r.player_id)
                .collect();

            // Budget cap: scouts should not recommend players the club cannot afford
            let max_recommend_value = plan.total_budget * 2.0;

            // ── Scout network recommendations ──
            for scout in &resolved.scouts {
                let judging = scout.staff_attributes.knowledge.judging_player_ability;
                let judging_pot = scout.staff_attributes.knowledge.judging_player_potential;

                // Discovery chance: 10 + (judging_ability * 3) percent
                let discovery_chance = 10 + (judging as i32 * 3);
                if IntegerUtils::random(0, 100) > discovery_chance {
                    continue;
                }

                // Filter candidates from other clubs
                let candidates: Vec<&PlayerSnapshot> = all_snapshots
                    .iter()
                    .filter(|p| {
                        p.club_id != club.id && !club.is_rival(p.club_id)
                            && !p.is_transfer_protected
                            && p.ability >= avg_ability.saturating_sub(10)
                            && p.ability <= avg_ability + (judging / 2)
                            && (max_recommend_value <= 0.0 || p.estimated_value <= max_recommend_value)
                            && !already_recommended.contains(&p.id)
                            && !actions
                                .iter()
                                .any(|a| a.club_id == club.id && a.recommendation.player_id == p.id)
                    })
                    .collect();

                if candidates.is_empty() {
                    continue;
                }

                // Score candidates
                let mut best_score = 0.0f32;
                let mut best_candidate: Option<&PlayerSnapshot> = None;

                for cand in &candidates {
                    let mut score: f32 = 0.0;

                    // Expiring contract
                    if cand.contract_months_remaining <= 6 {
                        score += 3.0;
                    } else if cand.contract_months_remaining <= 12 {
                        score += 1.5;
                    }

                    // Club in debt
                    if cand.club_in_debt {
                        score += 2.0;
                    }

                    // High potential gap
                    if cand.estimated_potential > cand.ability + 15 {
                        score += 2.5;
                    } else if cand.estimated_potential > cand.ability + 8 {
                        score += 1.5;
                    }

                    // Lower-rep club
                    if Self::rep_level_value(&cand.parent_club_reputation)
                        < Self::rep_level_value(&club_rep)
                    {
                        score += 1.0;
                    }

                    // Loan-listed
                    if cand.is_loan_listed {
                        score += if is_january { 2.0 } else { 1.0 };
                    }

                    // Ability fit
                    if cand.ability >= avg_ability.saturating_sub(5) {
                        score += 1.0;
                    }

                    if score > best_score {
                        best_score = score;
                        best_candidate = Some(cand);
                    }
                }

                if let Some(cand) = best_candidate {
                    // Assess with error based on judging skill
                    let ability_error = (20i16 - judging as i16).max(1) as i32;
                    let potential_error = (20i16 - judging_pot as i16).max(1) as i32;

                    let assessed_ability = (cand.ability as i32
                        + IntegerUtils::random(-ability_error, ability_error))
                    .clamp(1, 200) as u8;
                    let assessed_potential = (cand.estimated_potential as i32
                        + IntegerUtils::random(-potential_error, potential_error))
                    .clamp(1, 200) as u8;

                    let confidence = (0.3 + (judging as f32 * 0.035)).min(0.95);

                    let rec_type = if cand.contract_months_remaining <= 6 {
                        RecommendationType::ExpiringContract
                    } else if cand.club_in_debt {
                        RecommendationType::FinancialDistress
                    } else if cand.estimated_potential > cand.ability + 15 && cand.age <= 22 {
                        RecommendationType::HiddenGem
                    } else if cand.is_loan_listed {
                        RecommendationType::LoanOpportunity
                    } else {
                        RecommendationType::ReadyForStepUp
                    };

                    actions.push(RecommendationAction {
                        club_id: club.id,
                        recommendation: StaffRecommendation {
                            player_id: cand.id,
                            recommender_staff_id: scout.id,
                            source: RecommendationSource::ScoutNetwork,
                            recommendation_type: rec_type,
                            assessed_ability,
                            assessed_potential,
                            confidence,
                            estimated_fee: cand.estimated_value,
                            date_recommended: date,
                        },
                    });
                }
            }

            // ── DoF bargain identification ──
            if let Some(dof) = resolved.director_of_football {
                let judging = dof.staff_attributes.knowledge.judging_player_ability;
                let judging_pot = dof.staff_attributes.knowledge.judging_player_potential;
                let dof_chance = 40 + (judging as i32 * 3);

                if IntegerUtils::random(0, 100) <= dof_chance {
                    // Look for expiring contracts with ability >= avg-5
                    let dof_candidates: Vec<&PlayerSnapshot> = all_snapshots
                        .iter()
                        .filter(|p| {
                            p.club_id != club.id && !club.is_rival(p.club_id)
                                && !p.is_transfer_protected
                                && p.contract_months_remaining <= 6
                                && p.ability >= avg_ability.saturating_sub(5)
                                && !already_recommended.contains(&p.id)
                                && !actions.iter().any(|a| {
                                    a.club_id == club.id && a.recommendation.player_id == p.id
                                })
                        })
                        .collect();

                    if let Some(best) = dof_candidates.iter().max_by_key(|p| p.ability) {
                        let ability_error = (20i16 - judging as i16).max(1) as i32;
                        let potential_error = (20i16 - judging_pot as i16).max(1) as i32;

                        let assessed_ability = (best.ability as i32
                            + IntegerUtils::random(-ability_error, ability_error))
                        .clamp(1, 200) as u8;
                        let assessed_potential = (best.estimated_potential as i32
                            + IntegerUtils::random(-potential_error, potential_error))
                        .clamp(1, 200) as u8;

                        let confidence = (0.4 + (judging as f32 * 0.035)).min(0.95);

                        actions.push(RecommendationAction {
                            club_id: club.id,
                            recommendation: StaffRecommendation {
                                player_id: best.id,
                                recommender_staff_id: dof.id,
                                source: RecommendationSource::DirectorOfFootball,
                                recommendation_type: RecommendationType::ExpiringContract,
                                assessed_ability,
                                assessed_potential,
                                confidence,
                                estimated_fee: best.estimated_value,
                                date_recommended: date,
                            },
                        });
                    }
                }
            }

            // ── Small club staff: aggressive loan/bargain hunting ──
            // Small clubs rely on their staff to find cheap deals, loans,
            // free agents, and surplus players from bigger clubs.
            // Even a head coach at a small club knows what the squad needs.
            let is_small_club = matches!(
                club_rep,
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
            );
            let is_mid_club = club_rep == ReputationLevel::National;

            if is_small_club || is_mid_club {
                let rec_cap = if is_small_club { 10 } else { 8 };
                let current_recs = plan.staff_recommendations.len()
                    + actions.iter().filter(|a| a.club_id == club.id).count();

                if current_recs < rec_cap {
                    let remaining = rec_cap - current_recs;

                    // Coach recommends players available on loan
                    let head_coach = team.staffs.head_coach();
                    let coach_id = head_coach.id;
                    let coach_judging =
                        head_coach.staff_attributes.knowledge.judging_player_ability;
                    let coach_judging_pot =
                        head_coach.staff_attributes.knowledge.judging_player_potential;

                    // ── Cheap loan targets (loan-listed players the club could afford) ──
                    let mut loan_targets: Vec<&PlayerSnapshot> = all_snapshots
                        .iter()
                        .filter(|p| {
                            p.club_id != club.id && !club.is_rival(p.club_id)
                                && !p.is_transfer_protected
                                && p.is_loan_listed
                                && p.ability >= avg_ability.saturating_sub(8)
                                && !already_recommended.contains(&p.id)
                                && !actions.iter().any(|a| {
                                    a.club_id == club.id && a.recommendation.player_id == p.id
                                })
                        })
                        .collect();
                    loan_targets.sort_by(|a, b| b.ability.cmp(&a.ability));

                    for target in loan_targets.iter().take(remaining.min(3)) {
                        let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                        let potential_error = (20i16 - coach_judging_pot as i16).max(1) as i32;

                        let assessed_ability = (target.ability as i32
                            + IntegerUtils::random(-ability_error, ability_error))
                        .clamp(1, 200) as u8;
                        let assessed_potential = (target.estimated_potential as i32
                            + IntegerUtils::random(-potential_error, potential_error))
                        .clamp(1, 200) as u8;

                        let rec_type = if target.ability > avg_ability + 5 {
                            RecommendationType::BigClubSurplus
                        } else if target.age >= 28 {
                            RecommendationType::ExperiencedLoanMentor
                        } else {
                            RecommendationType::CheapLoanAvailable
                        };

                        let confidence = (0.4 + (coach_judging as f32 * 0.03)).min(0.9);

                        actions.push(RecommendationAction {
                            club_id: club.id,
                            recommendation: StaffRecommendation {
                                player_id: target.id,
                                recommender_staff_id: coach_id,
                                source: RecommendationSource::HeadCoach,
                                recommendation_type: rec_type,
                                assessed_ability,
                                assessed_potential,
                                confidence,
                                estimated_fee: target.estimated_value * 0.1, // loan fee
                                date_recommended: date,
                            },
                        });
                    }

                    let current_recs_after_loans = plan.staff_recommendations.len()
                        + actions.iter().filter(|a| a.club_id == club.id).count();
                    let remaining_after_loans = rec_cap.saturating_sub(current_recs_after_loans);

                    // ── Free agent bargains (expiring contracts) ──
                    if remaining_after_loans > 0 {
                        let mut free_targets: Vec<&PlayerSnapshot> = all_snapshots
                            .iter()
                            .filter(|p| {
                                p.club_id != club.id && !club.is_rival(p.club_id)
                                    && !p.is_transfer_protected
                                    && p.contract_months_remaining <= 6
                                    && p.ability >= avg_ability.saturating_sub(10)
                                    && !already_recommended.contains(&p.id)
                                    && !actions.iter().any(|a| {
                                        a.club_id == club.id
                                            && a.recommendation.player_id == p.id
                                    })
                            })
                            .collect();
                        free_targets.sort_by(|a, b| b.ability.cmp(&a.ability));

                        for target in free_targets.iter().take(remaining_after_loans.min(2)) {
                            let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                            let potential_error =
                                (20i16 - coach_judging_pot as i16).max(1) as i32;

                            let assessed_ability = (target.ability as i32
                                + IntegerUtils::random(-ability_error, ability_error))
                            .clamp(1, 200) as u8;
                            let assessed_potential = (target.estimated_potential as i32
                                + IntegerUtils::random(-potential_error, potential_error))
                            .clamp(1, 200) as u8;

                            let confidence = (0.5 + (coach_judging as f32 * 0.03)).min(0.9);

                            actions.push(RecommendationAction {
                                club_id: club.id,
                                recommendation: StaffRecommendation {
                                    player_id: target.id,
                                    recommender_staff_id: coach_id,
                                    source: RecommendationSource::HeadCoach,
                                    recommendation_type: RecommendationType::FreeAgentBargain,
                                    assessed_ability,
                                    assessed_potential,
                                    confidence,
                                    estimated_fee: 0.0, // free agent
                                    date_recommended: date,
                                },
                            });
                        }
                    }

                    let current_recs_after_free = plan.staff_recommendations.len()
                        + actions.iter().filter(|a| a.club_id == club.id).count();
                    let remaining_after_free = rec_cap.saturating_sub(current_recs_after_free);

                    // ── Players wanting game time from bigger clubs ──
                    // Young players at bigger clubs who aren't loan-listed yet but
                    // are below their club's average — they'd benefit from a loan
                    if remaining_after_free > 0 && is_small_club {
                        let mut game_time_seekers: Vec<&PlayerSnapshot> = all_snapshots
                            .iter()
                            .filter(|p| {
                                p.club_id != club.id && !club.is_rival(p.club_id)
                                    && !p.is_transfer_protected
                                    && p.age <= 23
                                    && p.estimated_potential > p.ability + 5
                                    && p.ability >= avg_ability.saturating_sub(5)
                                    && Self::rep_level_value(&p.parent_club_reputation)
                                        > Self::rep_level_value(&club_rep)
                                    && !p.is_loan_listed
                                    && !already_recommended.contains(&p.id)
                                    && !actions.iter().any(|a| {
                                        a.club_id == club.id
                                            && a.recommendation.player_id == p.id
                                    })
                            })
                            .collect();
                        game_time_seekers.sort_by(|a, b| b.estimated_potential.cmp(&a.estimated_potential));

                        for target in game_time_seekers.iter().take(remaining_after_free.min(2))
                        {
                            let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                            let potential_error =
                                (20i16 - coach_judging_pot as i16).max(1) as i32;

                            let assessed_ability = (target.ability as i32
                                + IntegerUtils::random(-ability_error, ability_error))
                            .clamp(1, 200) as u8;
                            let assessed_potential = (target.estimated_potential as i32
                                + IntegerUtils::random(-potential_error, potential_error))
                            .clamp(1, 200) as u8;

                            let confidence = (0.3 + (coach_judging as f32 * 0.025)).min(0.8);

                            actions.push(RecommendationAction {
                                club_id: club.id,
                                recommendation: StaffRecommendation {
                                    player_id: target.id,
                                    recommender_staff_id: coach_id,
                                    source: RecommendationSource::HeadCoach,
                                    recommendation_type: RecommendationType::GameTimeSeeker,
                                    assessed_ability,
                                    assessed_potential,
                                    confidence,
                                    estimated_fee: target.estimated_value * 0.05, // loan fee
                                    date_recommended: date,
                                },
                            });
                        }
                    }
                }
            }
        }

        // Pass 2: Push recommendations into club transfer plans
        // Small clubs get higher cap
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let rep = club
                    .teams
                    .teams
                    .first()
                    .map(|t| t.reputation.level())
                    .unwrap_or(ReputationLevel::Amateur);
                let cap = match rep {
                    ReputationLevel::Regional
                    | ReputationLevel::Local
                    | ReputationLevel::Amateur => 10,
                    ReputationLevel::National => 8,
                    _ => 6,
                };
                if club.transfer_plan.staff_recommendations.len() < cap {
                    club.transfer_plan
                        .staff_recommendations
                        .push(action.recommendation);
                }
            }
        }
    }

    pub fn process_staff_recommendations(country: &mut Country, date: NaiveDate) {
        // Only runs weekly (same schedule as should_evaluate)
        if !Self::should_evaluate(date) {
            return;
        }

        struct RecommendationProcessAction {
            club_id: u32,
            kind: RecommendationProcessKind,
        }

        enum RecommendationProcessKind {
            AddToShortlist {
                shortlist_request_id: u32,
                candidate: ShortlistCandidate,
            },
            CreateRequest {
                request: TransferRequest,
            },
        }

        let mut actions: Vec<RecommendationProcessAction> = Vec::new();
        let seven_days_ago = date - chrono::Duration::days(7);

        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            let recent_recs: Vec<&StaffRecommendation> = plan
                .staff_recommendations
                .iter()
                .filter(|r| r.date_recommended >= seven_days_ago)
                .collect();

            for rec in &recent_recs {
                // Determine player's position group
                let player_pos_group =
                    if let Some(player) = Self::find_player_in_country(country, rec.player_id) {
                        player.position().position_group()
                    } else {
                        continue;
                    };

                // Check if an existing unfulfilled request covers the same position group
                let matching_request = plan.transfer_requests.iter().find(|r| {
                    r.position.position_group() == player_pos_group
                        && r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                });

                if let Some(req) = matching_request {
                    // Find the shortlist for this request
                    let has_shortlist = plan
                        .shortlists
                        .iter()
                        .any(|s| s.transfer_request_id == req.id);

                    if has_shortlist {
                        // Add as candidate to existing shortlist
                        let already_in = plan.shortlists.iter().any(|s| {
                            s.transfer_request_id == req.id
                                && s.candidates.iter().any(|c| c.player_id == rec.player_id)
                        });

                        if !already_in {
                            actions.push(RecommendationProcessAction {
                                club_id: club.id,
                                kind: RecommendationProcessKind::AddToShortlist {
                                    shortlist_request_id: req.id,
                                    candidate: ShortlistCandidate {
                                        player_id: rec.player_id,
                                        score: rec.assessed_ability as f32 / 100.0
                                            + rec.confidence * 0.1,
                                        estimated_fee: rec.estimated_fee,
                                        status: ShortlistCandidateStatus::Available,
                                    },
                                },
                            });
                        }
                    }
                } else if rec.confidence >= 0.6 && rec.assessed_ability >= 50 {
                    // No existing request — create a new one
                    let player_position =
                        if let Some(player) = Self::find_player_in_country(country, rec.player_id)
                        {
                            player.position()
                        } else {
                            continue;
                        };

                    // Check we don't already have too many requests
                    let active_requests = plan
                        .transfer_requests
                        .iter()
                        .filter(|r| {
                            r.status != TransferRequestStatus::Fulfilled
                                && r.status != TransferRequestStatus::Abandoned
                        })
                        .count();

                    if active_requests >= 8 {
                        continue;
                    }

                    // Allocate 15% of available budget
                    let available_budget = plan.available_budget();
                    let alloc = available_budget * 0.15;

                    if alloc <= 0.0 {
                        continue;
                    }

                    let next_id = plan.next_request_id + actions
                        .iter()
                        .filter(|a| {
                            a.club_id == club.id
                                && matches!(a.kind, RecommendationProcessKind::CreateRequest { .. })
                        })
                        .count() as u32;

                    actions.push(RecommendationProcessAction {
                        club_id: club.id,
                        kind: RecommendationProcessKind::CreateRequest {
                            request: TransferRequest::new(
                                next_id,
                                player_position,
                                TransferNeedPriority::Optional,
                                TransferNeedReason::StaffRecommendation,
                                rec.assessed_ability.saturating_sub(5),
                                rec.assessed_ability,
                                alloc,
                            ),
                        },
                    });
                }
            }
        }

        // Pass 2: Apply actions
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let plan = &mut club.transfer_plan;

                match action.kind {
                    RecommendationProcessKind::AddToShortlist {
                        shortlist_request_id,
                        candidate,
                    } => {
                        if let Some(shortlist) = plan
                            .shortlists
                            .iter_mut()
                            .find(|s| s.transfer_request_id == shortlist_request_id)
                        {
                            shortlist.candidates.push(candidate);
                        }
                    }
                    RecommendationProcessKind::CreateRequest { request } => {
                        let req_id = request.id;
                        if req_id >= plan.next_request_id {
                            plan.next_request_id = req_id + 1;
                        }
                        plan.transfer_requests.push(request);
                    }
                }
            }
        }
    }
}
