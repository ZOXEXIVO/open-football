use chrono::NaiveDate;
use log::debug;

use crate::transfers::pipeline::{
    DetailedScoutingReport, ScoutingAssignment, ScoutingRecommendation, ShortlistCandidate,
    ShortlistCandidateStatus, TransferRequestStatus, TransferShortlist,
};
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::Country;

struct ShortlistResult {
    club_id: u32,
    shortlist: TransferShortlist,
    request_id: u32,
}

impl PipelineProcessor {
    // ============================================================
    // Step 5: Shortlist Building
    // ============================================================

    pub fn build_shortlists(country: &mut Country, date: NaiveDate) {
        let mut results: Vec<ShortlistResult> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;

            let existing_shortlist_request_ids: Vec<u32> = plan
                .shortlists
                .iter()
                .map(|s| s.transfer_request_id)
                .collect();

            // Use assignments that have produced at least one report,
            // not just fully completed ones. This prevents the pipeline
            // from stalling when scouts can only find 1 viable candidate.
            let completed_assignments: Vec<&ScoutingAssignment> = plan
                .scouting_assignments
                .iter()
                .filter(|a| a.completed || a.reports_produced > 0)
                .collect();

            for assignment in completed_assignments {
                if existing_shortlist_request_ids.contains(&assignment.transfer_request_id) {
                    continue;
                }

                let request = plan
                    .transfer_requests
                    .iter()
                    .find(|r| r.id == assignment.transfer_request_id);
                let budget_alloc = request.map(|r| r.budget_allocation).unwrap_or(0.0);

                let reports: Vec<&DetailedScoutingReport> = plan
                    .scouting_reports
                    .iter()
                    .filter(|r| r.assignment_id == assignment.id)
                    .collect();

                if reports.is_empty() {
                    continue;
                }

                let mut candidates: Vec<ShortlistCandidate> = reports
                    .iter()
                    .map(|r| {
                        let ability_score = r.assessed_ability as f32 / 200.0;
                        let potential_score = r.assessed_potential as f32 / 200.0;
                        let value_fit = if budget_alloc > 0.0 {
                            1.0 - (r.estimated_value / budget_alloc).min(1.0) as f32
                        } else {
                            0.5
                        };
                        let confidence_score = r.confidence;

                        let score = ability_score * 0.3
                            + potential_score * 0.2
                            + value_fit * 0.25
                            + confidence_score * 0.1
                            + match r.recommendation {
                                ScoutingRecommendation::StrongBuy => 0.15,
                                ScoutingRecommendation::Buy => 0.10,
                                ScoutingRecommendation::Consider => 0.05,
                                ScoutingRecommendation::Pass => 0.0,
                            };

                        ShortlistCandidate {
                            player_id: r.player_id,
                            score,
                            estimated_fee: r.estimated_value,
                            status: ShortlistCandidateStatus::Available,
                        }
                    })
                    .collect();

                candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
                candidates.truncate(10);

                if !candidates.is_empty() {
                    let mut shortlist =
                        TransferShortlist::new(assignment.transfer_request_id, budget_alloc);
                    shortlist.candidates = candidates;

                    results.push(ShortlistResult {
                        club_id: club.id,
                        shortlist,
                        request_id: assignment.transfer_request_id,
                    });
                }
            }

            // Market shortlist: for any request without a shortlist, also try building
            // one directly from market listings. This keeps transfers flowing while
            // scouts work on deeper evaluations in parallel.
            for request in &plan.transfer_requests {
                if request.status != TransferRequestStatus::Pending
                    && request.status != TransferRequestStatus::ScoutingActive
                {
                    continue;
                }
                if existing_shortlist_request_ids.contains(&request.id) {
                    continue;
                }
                // Skip if we already built a shortlist from scouting reports above
                if results.iter().any(|r| r.club_id == club.id && r.request_id == request.id) {
                    continue;
                }

                let market_candidates: Vec<ShortlistCandidate> = country
                    .transfer_market
                    .get_available_listings()
                    .iter()
                    .filter(|l| l.club_id != club.id && !club.is_rival(l.club_id))
                    .filter_map(|l| {
                        Self::find_player_summary_in_country(country, l.player_id, date).and_then(
                            |p| {
                                if p.position_group == request.position.position_group()
                                    && p.skill_ability >= request.min_ability
                                    && p.estimated_value <= request.budget_allocation * 5.0
                                {
                                    Some(ShortlistCandidate {
                                        player_id: p.player_id,
                                        score: p.skill_ability as f32 / 200.0,
                                        estimated_fee: p.estimated_value,
                                        status: ShortlistCandidateStatus::Available,
                                    })
                                } else {
                                    None
                                }
                            },
                        )
                    })
                    .take(5)
                    .collect();

                if !market_candidates.is_empty() {
                    let mut shortlist =
                        TransferShortlist::new(request.id, request.budget_allocation);
                    shortlist.candidates = market_candidates;

                    results.push(ShortlistResult {
                        club_id: club.id,
                        shortlist,
                        request_id: request.id,
                    });
                }
            }
        }

        if !results.is_empty() {
            debug!("Transfer pipeline: built {} shortlists", results.len());
        }

        for result in results {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == result.club_id) {
                let plan = &mut club.transfer_plan;

                if let Some(req) = plan
                    .transfer_requests
                    .iter_mut()
                    .find(|r| r.id == result.request_id)
                {
                    req.status = TransferRequestStatus::Shortlisted;
                }

                plan.shortlists.push(result.shortlist);
            }
        }
    }
}
