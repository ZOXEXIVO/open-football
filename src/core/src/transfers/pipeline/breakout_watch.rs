//! Year-round performance-breakout watch.
//!
//! The demand-driven scout pipeline and the listed-star sweep only run
//! inside a transfer window, so a player whose *form* is outrunning his
//! level — a 22-year-old striker top-scoring a second division — could sit
//! for months with zero interest and zero scout monitoring simply because
//! the window was shut and his club had merely loan-listed him.
//!
//! This pass closes that hole. Weekly, regardless of the window, it surfaces
//! genuine breakouts to plausible buyers as **scout monitoring** — the club
//! puts the player on its books. It never starts a negotiation: those stay
//! window-gated, so the recorded interest simply waits for the window to open
//! and then flows through the normal listed-star / recruitment-meeting path.
//!
//! Realism is identical to the in-window sweep: the same
//! [`evaluate_listed_target`] gates (tier window, affordability, wage
//! headroom, reputation plausibility, squad need / upgrade / resale) plus the
//! staged [`TransferPlausibilityBuilder`] veto run here — only with
//! `form_discovery_mode`, so a not-yet-listed breakout can be discovered on
//! form rather than on a for-sale sign. A flat-track scorer in a weak division
//! is league-rep discounted inside the breakout signal and so never clears the
//! bar; a top club still won't monitor a player who is no upgrade, no resale
//! prospect, and fills no need.
//!
//! Youth squads are covered too. Their matches are friendly-classified, so a
//! youngster's output lives in his `friendly_statistics` and his age-group
//! "league" carries no senior reputation. For a youth team the watch reads the
//! friendly bucket and scores it undiscounted
//! ([`crate::transfers::pipeline::breakout::LeaguePerformanceLookup::breakout_for_youth`]),
//! so an academy standout — an U18 banging in goals — surfaces to plausible
//! clubs as scout monitoring instead of staying invisible behind a
//! zero-reputation age-group league.
//!
//! Per project convention this is a method on [`PipelineProcessor`]; every
//! type is reached through a `use` at the file header.

use std::cmp::Ordering;

use chrono::{Datelike, NaiveDate, Weekday};

use crate::transfers::pipeline::breakout::LeaguePerformanceLookup;
use crate::transfers::pipeline::circulation::BuyerScan;
use crate::transfers::pipeline::plausibility::{
    BuyerPlausibilityContext, TransferPlausibilityBuilder, TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::recommendations::{
    ListedTargetVerdict, ListedTargetView, evaluate_listed_target,
};
use crate::transfers::pipeline::recruitment::{ScoutMonitoringSource, ScoutPlayerMonitoring};
use crate::{Country, Person};

/// One breakout player the watch may surface — his market summary, the few
/// extra signals the listed-target view needs that the summary doesn't carry,
/// and his discovery score.
struct BreakoutCandidate {
    summary: PlayerSummary,
    estimated_potential: u8,
    ambition: f32,
    days_available: i64,
    recent_interest_count: u8,
    failed_scans: u16,
    breakout_score: f32,
}

/// A monitoring row the apply pass will create on a buyer's books.
struct WatchAction {
    club_id: u32,
    recommender_staff_id: u32,
    player_id: u32,
    assessed_ability: u8,
    assessed_potential: u8,
    confidence: f32,
    estimated_value: f64,
}

impl PipelineProcessor {
    /// Per-pass cap on NEW breakout monitors a single club opens, so the
    /// watch builds a club's shortlist gradually rather than in one flood.
    const BREAKOUT_WATCH_PER_PASS: usize = 3;
    /// Soft ceiling on a club's total active monitoring rows before the watch
    /// stops adding more — keeps the books from growing without bound.
    const BREAKOUT_WATCH_MONITOR_CAP: usize = 30;

    /// Weekly, year-round breakout watch. Surfaces high-form players to
    /// plausible buyers as scout monitoring (never a negotiation). See the
    /// module docs for the realism model.
    pub fn scan_breakout_form(country: &mut Country, date: NaiveDate) {
        // Weekly cadence, independent of the transfer window.
        if date.weekday() != Weekday::Mon {
            return;
        }

        let performance_lookup = LeaguePerformanceLookup::build(country);

        // ── Collect breakout candidates (immutable read). ──
        let mut candidates: Vec<BreakoutCandidate> = Vec::new();
        for club in &country.clubs {
            let parent_league_reputation = club
                .teams
                .main()
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);

            for team in &club.teams.teams {
                let is_youth_squad = team.team_type.is_youth();
                for player in &team.players.players {
                    if player.is_on_loan() {
                        continue;
                    }
                    let group = player.position().position_group();
                    let age = player.age(date);
                    // Youth squads play friendly-classified age-group football, so
                    // a youngster's goals and rating live in the FRIENDLY bucket
                    // and his form is judged on the undiscounted signal — a scout
                    // watching the U18s rates the talent on what he sees, not on
                    // the (near-zero) standing of a youth league. Senior squads
                    // keep the official-stats, league-rep-discounted path.
                    let breakout = if is_youth_squad {
                        let appearances = player.friendly_statistics.total_games();
                        let average_rating =
                            player.friendly_statistics.average_rating_realistic(group);
                        performance_lookup.breakout_for_youth(
                            player,
                            appearances,
                            average_rating,
                            age,
                        )
                    } else {
                        let appearances = player.statistics.total_games();
                        let average_rating = player.statistics.average_rating_realistic(group);
                        performance_lookup.breakout_for_player(
                            player,
                            appearances,
                            average_rating,
                            age,
                            parent_league_reputation,
                        )
                    };
                    if !breakout.is_breakout() {
                        continue;
                    }

                    // The candidate walk already holds the (club, player)
                    // pair — build the summary directly instead of
                    // re-finding the player with a country-wide scan.
                    let summary = Self::build_player_summary(country, club, player, date);

                    let skill_ability = Self::position_evaluation_ability(player);
                    let estimated_potential = skill_ability
                        + Self::estimate_growth_potential(
                            age,
                            player.skills.mental.determination,
                            player.skills.mental.work_rate,
                            player.skills.mental.composure,
                            player.skills.mental.anticipation,
                            skill_ability,
                        );

                    candidates.push(BreakoutCandidate {
                        summary,
                        estimated_potential,
                        ambition: player.attributes.ambition,
                        days_available: player.days_available(date),
                        recent_interest_count: player
                            .availability_market_state()
                            .map(|s| s.recent_interest(date))
                            .unwrap_or(0),
                        failed_scans: player
                            .availability_market_state()
                            .map(|s| s.failed_scans)
                            .unwrap_or(0),
                        breakout_score: breakout.score,
                    });
                }
            }
        }

        if candidates.is_empty() {
            return;
        }

        // ── Per-buyer evaluation (immutable read). ──
        let mut actions: Vec<WatchAction> = Vec::new();
        for club in &country.clubs {
            if club.teams.teams.is_empty() {
                continue;
            }
            let plan = &club.transfer_plan;
            if !plan.initialized || plan.scout_monitoring.len() >= Self::BREAKOUT_WATCH_MONITOR_CAP
            {
                continue;
            }
            let Some(scan) = BuyerScan::build(country, club, date) else {
                continue;
            };

            let team = &club.teams.teams[0];
            let resolved = team.staffs.resolve_for_transfers();
            let recommender_id = resolved
                .director_of_football
                .map(|s| s.id)
                .or_else(|| resolved.scouts.first().map(|s| s.id))
                .unwrap_or(team.staffs.head_coach().id);
            let buyer_plaus_ctx = BuyerPlausibilityContext::build(country, club);

            let mut scored: Vec<(&BreakoutCandidate, f32)> = candidates
                .iter()
                .filter_map(|c| {
                    let s = &c.summary;
                    // Identity gates — own player, rival, or one this club is
                    // already tracking.
                    if s.club_id == club.id || club.is_rival(s.club_id) {
                        return None;
                    }
                    if !plan.monitorings_for_player(s.player_id).is_empty() {
                        return None;
                    }
                    // Meeting rejections blocklist the player for 6 months.
                    // The Rejected monitoring row fails is_active_interest,
                    // so the dedup above misses him and the watch used to
                    // re-seed a meeting-ready row the very next Monday —
                    // an agenda churn loop the blocklist exists to stop.
                    if plan.is_rejected(s.player_id, date) {
                        return None;
                    }
                    // Staged plausibility veto — importance / country route /
                    // step-down realism. Unsolicited: we're scouting on form.
                    if matches!(
                        TransferPlausibilityBuilder::evaluate_summary(
                            &buyer_plaus_ctx,
                            s,
                            false,
                            true,
                            date,
                        ),
                        Some(TransferPlausibilityVerdict::HardReject(_))
                    ) {
                        return None;
                    }

                    let view = ListedTargetView {
                        ability: s.skill_ability,
                        estimated_potential: c.estimated_potential,
                        age: s.age,
                        estimated_value: s.estimated_value,
                        position_group: s.position_group,
                        is_listed: s.is_listed,
                        is_transfer_requested: s.seller_ctx.is_transfer_requested,
                        is_unhappy: s.seller_ctx.is_unhappy,
                        is_loan_listed: s.is_loan_listed,
                        breakout_score: c.breakout_score,
                        world_reputation: s.world_reputation,
                        current_reputation: s.current_reputation,
                        ambition: c.ambition,
                        parent_club_score: s.seller_ctx.club_reputation_score,
                        parent_club_in_debt: s.seller_ctx.in_debt,
                        days_available: c.days_available,
                        contract_months_remaining: s.contract_months_remaining,
                        low_usage: s.appearances < 8,
                        recent_interest_count: c.recent_interest_count,
                        failed_scans: c.failed_scans,
                    };
                    // Form-discovery mode: a not-yet-listed breakout is
                    // admitted, but the affordability / tier / reputation /
                    // squad-need gates are unchanged.
                    let ctx = scan.buyer_context(s.position_group, true);
                    match evaluate_listed_target(&view, &ctx) {
                        ListedTargetVerdict::Accept(score) => Some((c, score)),
                        ListedTargetVerdict::Reject(_) => None,
                    }
                })
                .collect();
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

            for (cand, _score) in scored.iter().take(Self::BREAKOUT_WATCH_PER_PASS) {
                let s = &cand.summary;
                actions.push(WatchAction {
                    club_id: club.id,
                    recommender_staff_id: recommender_id,
                    player_id: s.player_id,
                    assessed_ability: s.skill_ability,
                    assessed_potential: cand.estimated_potential,
                    // Confidence scales with breakout strength so a clear
                    // breakout lands meeting-ready — the interest can flow
                    // straight into the shortlist when the window opens.
                    confidence: (0.5 + (cand.breakout_score / 100.0) * 0.35).min(0.85),
                    estimated_value: s.estimated_value,
                });
            }
        }

        if actions.is_empty() {
            return;
        }

        // ── Apply (mutable): open a monitoring row per accepted target. ──
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let plan = &mut club.transfer_plan;
                if plan
                    .find_monitoring_mut(action.recommender_staff_id, action.player_id)
                    .is_some()
                {
                    continue;
                }
                let id = plan.next_monitoring_id();
                let mut row = ScoutPlayerMonitoring::new(
                    id,
                    action.recommender_staff_id,
                    action.player_id,
                    ScoutMonitoringSource::StaffRecommendation,
                    date,
                );
                row.record_observation(
                    action.assessed_ability,
                    action.assessed_potential,
                    action.confidence,
                    1.0,
                    action.estimated_value,
                    Vec::new(),
                    date,
                    false,
                );
                plan.scout_monitoring.push(row);
            }
        }
    }
}
