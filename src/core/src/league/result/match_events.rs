use std::collections::HashMap;
use crate::club::player::contract::ContractBonusType;
use crate::club::player::events::{MatchOutcome, MatchParticipation};
use crate::club::team::reputation::{
    CompetitionType as RepCompetition, MatchOutcome as RepOutcome,
};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::club::team::team_talks::{apply_team_talk, MatchPhase, TeamTalkContext, TeamTalkTone};
use crate::club::StaffPosition;
use crate::continent::competitions::{CHAMPIONS_LEAGUE_ID, EUROPA_LEAGUE_ID, CONFERENCE_LEAGUE_ID};
use crate::r#match::engine::result::MatchResultRaw;
use crate::r#match::MatchResult;
use crate::simulator::SimulatorData;
use super::LeagueResult;

impl LeagueResult {
    pub(super) fn process_match_events(result: &mut MatchResult, data: &mut SimulatorData) {
        let details = match &result.details {
            Some(d) => d,
            None => return,
        };

        let is_cup = result.league_id >= 900_000_000;
        let is_friendly = if is_cup {
            false
        } else {
            data.league(result.league_id)
                .map(|l| l.friendly)
                .unwrap_or(false)
        };

        // Players inside their post-transfer settlement window play at a
        // reduced level. Dampened rating feeds into season averages, POM
        // selection, debriefs, and reputation.
        let now_date = data.date.date();
        let effective_ratings = compute_effective_ratings(details, data, now_date);
        let best_player_id = pick_player_of_the_match(details, &effective_ratings);

        let (league_weight, world_weight) = reputation_weights(result, is_cup, data);

        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();
        let home_team_id = result.score.home_team.team_id;
        let away_team_id = result.score.away_team.team_id;

        // Rivalry detection: walk team → club on both sides and check whether
        // either club lists the other as a rival. Either-direction is a derby.
        let home_club = data.team(home_team_id).map(|t| t.club_id);
        let away_club = data.team(away_team_id).map(|t| t.club_id);
        let is_derby = match (home_club, away_club) {
            (Some(h), Some(a)) => {
                let h_vs_a = data.club(h).map(|c| c.is_rival(a)).unwrap_or(false);
                let a_vs_h = data.club(a).map(|c| c.is_rival(h)).unwrap_or(false);
                h_vs_a || a_vs_h
            }
            _ => false,
        };

        // Per-player match reaction — Player owns all bookkeeping.
        for side in [&details.left_team_players, &details.right_team_players] {
            let conceded = if side.team_id == home_team_id { away_goals } else { home_goals };
            let (team_won, team_lost) = if side.team_id == home_team_id {
                (home_goals > away_goals, home_goals < away_goals)
            } else {
                (away_goals > home_goals, away_goals < home_goals)
            };
            dispatch_match_outcomes(
                side,
                conceded,
                details,
                data,
                &effective_ratings,
                best_player_id,
                is_friendly,
                is_cup,
                league_weight,
                world_weight,
                is_derby,
                team_won,
                team_lost,
            );
        }

        if !is_friendly {
            Self::process_loan_match_fees(details, data);
            Self::process_contract_bonuses(result, details, data);
        }

        Self::apply_full_time_team_talks(result, details, data);
        Self::apply_post_match_physical_effects(details, data, is_friendly);
        Self::apply_post_match_reputation(result, data, is_friendly, is_cup);

        if let Some(details_mut) = &mut result.details {
            details_mut.player_of_the_match_id = best_player_id;
        }
    }

    /// Feed the completed match into both teams' `TeamReputation`. Friendlies
    /// don't drift reputation; cups and league games do, with competition
    /// weighting handled inside `process_weekly_update`.
    fn apply_post_match_reputation(
        result: &MatchResult,
        data: &mut SimulatorData,
        is_friendly: bool,
        is_cup: bool,
    ) {
        if is_friendly {
            return;
        }

        let home_team_id = result.score.home_team.team_id;
        let away_team_id = result.score.away_team.team_id;
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();

        let home_rep = data
            .team(home_team_id)
            .map(|t| overall_score_to_u16(t.reputation.overall_score()))
            .unwrap_or(0);
        let away_rep = data
            .team(away_team_id)
            .map(|t| overall_score_to_u16(t.reputation.overall_score()))
            .unwrap_or(0);

        let (home_outcome, away_outcome) = match home_goals.cmp(&away_goals) {
            std::cmp::Ordering::Greater => (RepOutcome::Win, RepOutcome::Loss),
            std::cmp::Ordering::Less => (RepOutcome::Loss, RepOutcome::Win),
            std::cmp::Ordering::Equal => (RepOutcome::Draw, RepOutcome::Draw),
        };

        let comp = match (result.league_id, is_cup) {
            (CHAMPIONS_LEAGUE_ID | EUROPA_LEAGUE_ID | CONFERENCE_LEAGUE_ID, _) => {
                RepCompetition::ContinentalCup
            }
            (_, true) => RepCompetition::DomesticCup,
            _ => RepCompetition::League,
        };

        let (home_pos, away_pos, total_teams) =
            league_standings(result.league_id, home_team_id, away_team_id, data);
        let date = data.date.date();

        if let Some(team) = data.team_mut(home_team_id) {
            team.on_match_completed(home_outcome, away_rep, comp.clone(), home_pos, total_teams, date);
        }
        if let Some(team) = data.team_mut(away_team_id) {
            team.on_match_completed(away_outcome, home_rep, comp, away_pos, total_teams, date);
        }
    }

    /// Process loan match fees: parent club pays borrowing club per official appearance.
    /// Collects (parent_club_id, borrowing_club_id, fee) for all loan players who appeared,
    /// then applies the financial transactions.
    /// Pay out per-match contract clause triggers:
    ///   AppearanceFee  → flat bonus per player who played
    ///   GoalFee        → flat bonus × goals scored in this match
    ///   CleanSheetFee  → flat bonus for GK on a clean sheet
    ///
    /// Bonuses are charged to the employing club as a player-wage expense.
    fn process_contract_bonuses(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut SimulatorData,
    ) {
        // Count this match's goals per player.
        let mut goals_per_player: HashMap<u32, u8> = HashMap::new();
        for detail in &result.score.details {
            if detail.stat_type == MatchStatisticType::Goal {
                *goals_per_player.entry(detail.player_id).or_insert(0) += 1;
            }
        }

        // Everyone who featured in the match (main + used subs).
        let mut appearance_ids: Vec<u32> = Vec::new();
        appearance_ids.extend(details.left_team_players.main.iter().copied());
        appearance_ids.extend(details.left_team_players.substitutes_used.iter().copied());
        appearance_ids.extend(details.right_team_players.main.iter().copied());
        appearance_ids.extend(details.right_team_players.substitutes_used.iter().copied());

        // Which GKs started on which team + did they keep a clean sheet?
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();
        let home_team_id = result.score.home_team.team_id;
        let left_team_id = details.left_team_players.team_id;
        let left_conceded = if left_team_id == home_team_id { away_goals } else { home_goals };
        let right_conceded = if left_team_id == home_team_id { home_goals } else { away_goals };

        // Pass 1 (read): compute (club_id, total_payout) aggregates without holding
        // a mutable borrow of a club while reading another player.
        let mut club_totals: HashMap<u32, i64> = HashMap::new();
        for pid in &appearance_ids {
            if let Some(player) = data.player(*pid) {
                // Player's current club comes from the player index (updated by
                // the simulator whenever a transfer moves someone).
                let club_id = match data
                    .indexes
                    .as_ref()
                    .and_then(|i| i.get_player_location(*pid))
                {
                    Some((_, _, club_id, _)) => club_id,
                    None => continue,
                };
                let contract = match player.contract.as_ref() {
                    Some(c) => c,
                    None => continue,
                };

                // Determine clean sheet eligibility for a GK
                let is_gk = player.position().is_goalkeeper();
                let on_left = details.left_team_players.main.contains(pid);
                let on_right = details.right_team_players.main.contains(pid);
                let gk_clean_sheet = is_gk && (
                    (on_left && left_conceded == 0) ||
                    (on_right && right_conceded == 0)
                );

                let goals = goals_per_player.get(pid).copied().unwrap_or(0) as i64;

                let mut payout: i64 = 0;
                for bonus in &contract.bonuses {
                    let v = bonus.value as i64;
                    if v <= 0 { continue; }
                    match bonus.bonus_type {
                        ContractBonusType::AppearanceFee => payout += v,
                        ContractBonusType::GoalFee => payout += v * goals,
                        ContractBonusType::CleanSheetFee if gk_clean_sheet => payout += v,
                        _ => {}
                    }
                }
                if payout > 0 {
                    *club_totals.entry(club_id).or_insert(0) += payout;
                }
            }
        }

        // Pass 2 (write): charge each club once.
        for (club_id, amount) in club_totals {
            if let Some(club) = data.club_mut(club_id) {
                club.finance.balance.push_expense_player_wages(amount);
            }
        }
    }

    /// Apply a post-match full-time team talk to both sides. Tone is picked
    /// from the score outcome; the head coach's Man Management / Motivating
    /// attributes drive effectiveness. The actual magnitude-per-player uses
    /// personality (pressure, temperament, important_matches) via
    /// `club::team::team_talks::apply_team_talk`.
    fn apply_full_time_team_talks(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut SimulatorData,
    ) {
        let score = &result.score;
        let home_goals = score.home_team.get() as i8;
        let away_goals = score.away_team.get() as i8;
        let home_team_id = score.home_team.team_id;
        let left_team_id = details.left_team_players.team_id;

        // Map "left" / "right" match slots to score deltas.
        let left_delta: i8 = if left_team_id == home_team_id {
            home_goals - away_goals
        } else {
            away_goals - home_goals
        };
        let right_delta: i8 = -left_delta;

        let tone_for = |delta: i8| -> TeamTalkTone {
            match delta {
                d if d >= 2 => TeamTalkTone::Praise,
                1 => TeamTalkTone::Encourage,
                0 => TeamTalkTone::Encourage,
                -1 => TeamTalkTone::Criticise,
                _ => TeamTalkTone::Passionate,
            }
        };

        // Collect each side's player ids + club id once.
        struct SideTalk {
            club_id: u32,
            player_ids: Vec<u32>,
            delta: i8,
        }

        let mut sides: Vec<SideTalk> = Vec::with_capacity(2);
        for (slot, delta) in [
            (&details.left_team_players, left_delta),
            (&details.right_team_players, right_delta),
        ] {
            let mut pids: Vec<u32> = Vec::new();
            pids.extend(slot.main.iter().copied());
            pids.extend(slot.substitutes_used.iter().copied());
            if pids.is_empty() {
                continue;
            }
            // Resolve club id from the first player's index entry.
            let club_id = data
                .indexes
                .as_ref()
                .and_then(|i| i.get_player_location(*pids.first().unwrap()))
                .map(|(_, _, c, _)| c)
                .unwrap_or(0);
            sides.push(SideTalk { club_id, player_ids: pids, delta });
        }

        for side in sides {
            // Find the head coach (Manager) for this club. Scans each team's
            // staff collection via StaffCollection::find_by_position — the
            // first team that has a Manager on the books wins.
            let manager_ref = data.club(side.club_id).and_then(|club| {
                club.teams
                    .teams
                    .iter()
                    .find_map(|t| t.staffs.find_by_position(StaffPosition::Manager))
            });
            let manager_clone = manager_ref.cloned();

            let tone = tone_for(side.delta);
            let ctx = TeamTalkContext {
                phase: MatchPhase::FullTime,
                score_delta: side.delta,
                big_match: false, // cup/derby detection would slot in here
            };

            // Borrow each player mutably one at a time; apply_team_talk
            // needs &mut Player so we route through player_mut.
            for pid in &side.player_ids {
                if let Some(player) = data.player_mut(*pid) {
                    // apply_team_talk takes an iterator of &mut Player; use a
                    // single-element array for the per-player loop.
                    let single = std::slice::from_mut(player);
                    apply_team_talk(single.iter_mut(), manager_clone.as_ref(), tone, ctx);
                }
            }
        }
    }

    fn process_loan_match_fees(details: &MatchResultRaw, data: &mut SimulatorData) {
        // Collect fee transfers: (parent_club_id, borrowing_club_id, fee)
        let mut fee_transfers: Vec<(u32, u32, u32)> = Vec::new();

        let all_players = details.left_team_players.main.iter()
            .chain(details.left_team_players.substitutes_used.iter())
            .chain(details.right_team_players.main.iter())
            .chain(details.right_team_players.substitutes_used.iter());

        for &player_id in all_players {
            if let Some(player) = data.player(player_id) {
                if let Some(ref loan) = player.contract_loan {
                    if let (Some(fee), Some(parent_id), Some(borrowing_id)) =
                        (loan.loan_match_fee, loan.loan_from_club_id, loan.loan_to_club_id)
                    {
                        if fee > 0 {
                            fee_transfers.push((parent_id, borrowing_id, fee));
                        }
                    }
                }
            }
        }

        // Apply financial transactions
        for (parent_club_id, borrowing_club_id, fee) in fee_transfers {
            let amount = fee as i64;
            if let Some(parent_club) = data.club_mut(parent_club_id) {
                parent_club.finance.balance.push_expense_loan_fees(amount);
            }
            if let Some(borrowing_club) = data.club_mut(borrowing_club_id) {
                borrowing_club.finance.balance.push_income_loan_fees(amount);
            }
        }
    }
}

fn compute_effective_ratings(
    details: &MatchResultRaw,
    data: &SimulatorData,
    now: chrono::NaiveDate,
) -> HashMap<u32, f32> {
    let mut out = HashMap::with_capacity(details.player_stats.len());
    for (player_id, stats) in &details.player_stats {
        let location = data
            .indexes
            .as_ref()
            .and_then(|i| i.get_player_location(*player_id));
        let country_code = location
            .and_then(|(_, country_id, _, _)| data.country_info.get(&country_id))
            .map(|ci| ci.code.clone())
            .unwrap_or_default();
        let club_rep = location
            .and_then(|(_, _, _, team_id)| data.team(team_id))
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.0);
        let mult = data
            .player(*player_id)
            .map(|p| p.settlement_form_multiplier(now, &country_code, club_rep))
            .unwrap_or(1.0);
        out.insert(*player_id, stats.match_rating * mult);
    }
    out
}

fn pick_player_of_the_match(
    details: &MatchResultRaw,
    effective_ratings: &HashMap<u32, f32>,
) -> Option<u32> {
    let mut best_rating = 0.0_f32;
    let mut best = None;
    for (player_id, stats) in &details.player_stats {
        let r = *effective_ratings.get(player_id).unwrap_or(&stats.match_rating);
        if r > best_rating {
            best_rating = r;
            best = Some(*player_id);
        }
    }
    best
}

fn reputation_weights(
    result: &MatchResult,
    is_cup: bool,
    data: &SimulatorData,
) -> (f32, f32) {
    if result.league_id == CHAMPIONS_LEAGUE_ID {
        (1.5, 1.2)
    } else if result.league_id == EUROPA_LEAGUE_ID {
        (1.3, 0.8)
    } else if result.league_id == CONFERENCE_LEAGUE_ID {
        (1.1, 0.5)
    } else if is_cup {
        (1.0, 0.3)
    } else {
        let league_reputation = data
            .league(result.league_id)
            .map(|l| l.reputation)
            .unwrap_or(500) as f32;
        let w = (league_reputation / 1000.0 + 0.5).clamp(0.5, 1.5);
        (w, 0.2)
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_match_outcomes(
    side: &crate::r#match::FieldSquad,
    team_conceded: u8,
    details: &MatchResultRaw,
    data: &mut SimulatorData,
    effective_ratings: &HashMap<u32, f32>,
    best_player_id: Option<u32>,
    is_friendly: bool,
    is_cup: bool,
    league_weight: f32,
    world_weight: f32,
    is_derby: bool,
    team_won: bool,
    team_lost: bool,
) {
    let starter_ids: Vec<u32> = side.main.iter().copied().collect();
    let sub_ids: Vec<u32> = side.substitutes_used.iter().copied().collect();
    let all_ids: Vec<(u32, MatchParticipation)> = starter_ids
        .iter()
        .map(|id| (*id, MatchParticipation::Starter))
        .chain(sub_ids.iter().map(|id| (*id, MatchParticipation::Substitute)))
        .collect();

    for (pid, participation) in all_ids {
        let stats = match details.player_stats.get(&pid) {
            Some(s) => s,
            None => continue,
        };
        let effective = *effective_ratings.get(&pid).unwrap_or(&stats.match_rating);
        let is_motm = best_player_id == Some(pid);
        let team_goals_against = matches!(participation, MatchParticipation::Starter)
            .then_some(team_conceded);
        if let Some(player) = data.player_mut(pid) {
            player.on_match_played(&MatchOutcome {
                stats,
                effective_rating: effective,
                participation,
                is_friendly,
                is_cup,
                is_motm,
                team_goals_against,
                league_weight,
                world_weight,
                is_derby,
                team_won,
                team_lost,
            });
        }
    }

    // Unused substitutes feel the snub as well, even though they didn't
    // feature in player_stats.
    for &pid in &side.substitutes {
        if !side.substitutes_used.contains(&pid) {
            if let Some(player) = data.player_mut(pid) {
                player.on_match_dropped();
            }
        }
    }
}

/// Compact the overall reputation score (0.0–1.0) into the u16 scale the
/// `TeamReputation` drift model expects (same 0–10_000 basis as `home`/
/// `national`/`world`).
fn overall_score_to_u16(score: f32) -> u16 {
    (score * 10_000.0).clamp(0.0, 10_000.0) as u16
}

/// Look up league-table positions for both teams and the total number of
/// teams. Cup and continental competitions fall back to 1/1/1 — the
/// position factor is meaningless there but still needs a valid denominator.
fn league_standings(
    league_id: u32,
    home_team_id: u32,
    away_team_id: u32,
    data: &SimulatorData,
) -> (u8, u8, u8) {
    let league = match data.league(league_id) {
        Some(l) => l,
        None => return (1, 1, 1),
    };
    let rows = &league.table.rows;
    if rows.is_empty() {
        return (1, 1, 1);
    }
    let total = rows.len().min(u8::MAX as usize) as u8;
    let position = |team_id: u32| -> u8 {
        let pts = rows
            .iter()
            .find(|r| r.team_id == team_id)
            .map(|r| r.points)
            .unwrap_or(0);
        let ahead = rows.iter().filter(|r| r.points > pts).count();
        (ahead + 1).min(u8::MAX as usize) as u8
    };
    (position(home_team_id), position(away_team_id), total)
}
