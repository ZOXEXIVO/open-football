mod competitive;
pub(crate) mod helpers;
mod rotation;
pub(crate) mod scoring;

#[cfg(test)]
mod tests;

use crate::club::{ClubPhilosophy, PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::{MatchTacticType, Player, PlayerStatusType, Tactics};
use chrono::NaiveDate;
use log::debug;
use std::borrow::Borrow;

use helpers::*;
use scoring::ScoringEngine;

pub struct SquadSelector;

pub struct PlayerSelectionResult {
    pub main_squad: Vec<MatchPlayer>,
    pub substitutes: Vec<MatchPlayer>,
}

pub struct SelectionContext {
    pub is_friendly: bool,
    pub date: NaiveDate,
    /// Match importance: 0.0 = dead rubber, 1.0 = must-win.
    /// Below 0.4: use rotation selection — reserve/youth players get chances.
    pub match_importance: f32,
    /// Club philosophy — tilts the starting XI selection. Populated by
    /// the match-day caller so a DevelopAndSell side actually puts the
    /// kids on the pitch in league games.
    pub philosophy: Option<ClubPhilosophy>,
    /// Opponent's expected baseline tactic. When present and the coach is
    /// tactically astute, `get_enhanced_match_squad` flips to a counter
    /// formation instead of the pre-selected one.
    pub opponent_tactic: Option<MatchTacticType>,
}

impl Default for SelectionContext {
    fn default() -> Self {
        SelectionContext {
            is_friendly: false,
            date: chrono::Utc::now().date_naive(),
            match_importance: 0.7,
            philosophy: None,
            opponent_tactic: None,
        }
    }
}

impl SquadSelector {
    // ========== PUBLIC API ==========

    pub fn select(team: &crate::Team, staff: &Staff) -> PlayerSelectionResult {
        Self::select_with_reserves(team, staff, &[])
    }

    pub fn select_with_reserves(
        team: &crate::Team,
        staff: &Staff,
        reserve_players: &[&Player],
    ) -> PlayerSelectionResult {
        Self::select_with_context(team, staff, reserve_players, &SelectionContext::default())
    }

    pub fn select_with_context(
        team: &crate::Team,
        staff: &Staff,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let tactics = team.tactics();
        let engine = ScoringEngine::from_staff_with_philosophy(staff, ctx.philosophy.clone());

        let mut available: Vec<&Player> = team
            .players
            .players()
            .iter()
            .filter(|&&p| is_available(p, ctx.is_friendly))
            .copied()
            .collect();

        for &rp in reserve_players {
            if is_available(rp, ctx.is_friendly) && !available.iter().any(|p| p.id == rp.id) {
                available.push(rp);
            }
        }

        let outfield_count = available
            .iter()
            .filter(|p| !is_goalkeeper_player(p))
            .count();
        let gk_count = available.len() - outfield_count;

        if available.len() < DEFAULT_SQUAD_SIZE {
            let all_players = team.players.players();
            let injured_count = all_players.iter().filter(|p| p.player_attributes.is_injured).count();
            let int_count = all_players.iter().filter(|p| p.statuses.get().contains(&PlayerStatusType::Int)).count();
            let low_condition = all_players.iter().filter(|p| !p.player_attributes.is_injured && p.player_attributes.condition_percentage() < HARD_CONDITION_FLOOR).count();
            let banned_count = if !ctx.is_friendly { all_players.iter().filter(|p| p.player_attributes.is_banned).count() } else { 0 };
            let lst_loa_count = if !ctx.is_friendly {
                all_players.iter().filter(|p| {
                    let s = p.statuses.get();
                    s.contains(&PlayerStatusType::Lst) || s.contains(&PlayerStatusType::Loa)
                }).count()
            } else { 0 };

            log::warn!(
                "Squad selection for team {}: only {} available out of {} registered \
                (injured={}, international={}, low_condition={}, banned={}, lst_loa={}, \
                {} outfield, {} GK, {} reserves offered)",
                team.name, available.len(), all_players.len(),
                injured_count, int_count, low_condition, banned_count, lst_loa_count,
                outfield_count, gk_count, reserve_players.len()
            );
        } else if available.len() < DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE {
            debug!(
                "Squad selection for team {}: only {} available (need {} for full squad+bench)",
                team.name, available.len(), DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE
            );
        }

        let main_squad = competitive::select_starting_eleven(
            team.id, &available, staff, tactics.borrow(), &engine, ctx.date, ctx.is_friendly, ctx.match_importance,
        );

        let remaining: Vec<&Player> = available
            .iter()
            .filter(|p| !main_squad.iter().any(|mp| mp.id == p.id))
            .copied()
            .collect();

        let mut substitutes = competitive::select_substitutes(
            team.id, &remaining, staff, tactics.borrow(), &engine, ctx.date, ctx.is_friendly, ctx.match_importance,
        );

        if substitutes.is_empty() && !remaining.is_empty() {
            debug!(
                "Substitute selection produced empty bench with {} remaining players — force-populating",
                remaining.len()
            );
            for player in &remaining {
                if substitutes.len() >= DEFAULT_BENCH_SIZE {
                    break;
                }
                let pos = best_tactical_position(player, tactics.borrow());
                substitutes.push(MatchPlayer::from_player(team.id, player, pos, false));
            }
        }

        debug!("Final squad: {} starters, {} subs", main_squad.len(), substitutes.len());

        PlayerSelectionResult { main_squad, substitutes }
    }

    // ========== ROTATION SELECTION ==========

    pub fn select_for_rotation(team: &crate::Team, staff: &Staff) -> PlayerSelectionResult {
        Self::select_for_rotation_with_reserves(team, staff, &[])
    }

    pub fn select_for_rotation_with_reserves(
        team: &crate::Team,
        staff: &Staff,
        reserve_players: &[&Player],
    ) -> PlayerSelectionResult {
        Self::select_for_rotation_with_context(
            team, staff, reserve_players,
            &SelectionContext { is_friendly: true, ..SelectionContext::default() },
        )
    }

    pub fn select_for_rotation_with_context(
        team: &crate::Team,
        staff: &Staff,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let tactics = team.tactics();

        let mut available: Vec<&Player> = team
            .players
            .players()
            .iter()
            .filter(|&&p| is_available(p, ctx.is_friendly))
            .copied()
            .collect();

        if available.len() < DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE {
            let needed = (DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE) - available.len();
            let mut supplements: Vec<&Player> = reserve_players
                .iter()
                .filter(|&&rp| {
                    is_available(rp, ctx.is_friendly)
                        && !available.iter().any(|p| p.id == rp.id)
                })
                .copied()
                .collect();

            supplements.sort_by(|a, b| {
                b.player_attributes.days_since_last_match
                    .cmp(&a.player_attributes.days_since_last_match)
            });

            for rp in supplements.into_iter().take(needed) {
                available.push(rp);
            }

            if available.len() < DEFAULT_SQUAD_SIZE {
                debug!(
                    "Rotation selection for team {}: only {} available after borrowing ({} reserves offered)",
                    team.name, available.len(), reserve_players.len()
                );
            }
        }

        let main_squad = rotation::select_rotation_starting_eleven(
            team.id, &available, staff, tactics.borrow(),
        );

        let remaining: Vec<&Player> = available
            .iter()
            .filter(|p| !main_squad.iter().any(|mp| mp.id == p.id))
            .copied()
            .collect();

        let mut substitutes = rotation::select_rotation_substitutes(
            team.id, &remaining, staff, tactics.borrow(),
        );

        if substitutes.is_empty() && !remaining.is_empty() {
            debug!(
                "Rotation substitute selection produced empty bench with {} remaining — force-populating",
                remaining.len()
            );
            for player in &remaining {
                if substitutes.len() >= DEFAULT_BENCH_SIZE {
                    break;
                }
                let pos = best_tactical_position(player, tactics.borrow());
                substitutes.push(MatchPlayer::from_player(team.id, player, pos, false));
            }
        }

        PlayerSelectionResult { main_squad, substitutes }
    }

    // ========== LEGACY PUBLIC API ==========

    pub fn calculate_player_rating_for_position(
        player: &Player,
        staff: &Staff,
        position: PlayerPositionType,
        tactics: &Tactics,
    ) -> f32 {
        let group = position.position_group();
        let engine = ScoringEngine::from_staff(staff);
        let date = chrono::Utc::now().date_naive();
        engine.score_player_for_slot(player, position, group, staff, tactics, date, false, &[])
    }

    pub fn select_main_squad(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let engine = ScoringEngine::from_staff(staff);
        let date = chrono::Utc::now().date_naive();
        competitive::select_starting_eleven(team_id, players, staff, tactics, &engine, date, false, 0.7)
    }

    pub fn select_substitutes_legacy(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let engine = ScoringEngine::from_staff(staff);
        let date = chrono::Utc::now().date_naive();
        competitive::select_substitutes(team_id, players, staff, tactics, &engine, date, false, 0.7)
    }
}
