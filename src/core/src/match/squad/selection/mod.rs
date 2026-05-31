mod competitive;
mod cup_rotation;
pub(crate) mod helpers;
mod omissions;
mod rotation;
pub(crate) mod scoring;

#[cfg(test)]
mod tests;

use crate::club::{ClubPhilosophy, PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::{MatchTacticType, Player, PlayerStatusType, Tactics, Team, TeamType};
use chrono::NaiveDate;
use log::debug;
use std::borrow::Borrow;
use std::collections::HashSet;

use helpers::*;
use scoring::ScoringEngine;

use chrono::Utc;
pub use omissions::OmittedPlayer;

pub struct SquadSelector;

pub struct PlayerSelectionResult {
    pub main_squad: Vec<MatchPlayer>,
    pub substitutes: Vec<MatchPlayer>,
    /// Important omissions — players the selector chose not to start or
    /// not to include in the matchday squad, where the omission deserves
    /// a structured explanation in the player-events feed (KeyPlayers
    /// missed, regulars demoted, force-selected players overlooked, …).
    /// Empty when nothing notable happened.
    pub omissions: Vec<OmittedPlayer>,
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
    /// Which competition the match belongs to. Lets the selector apply a
    /// domestic-cup opportunity bias (rotate hard in early rounds, field a
    /// strong XI in semis/finals) instead of inferring everything from a
    /// `knockout` bool.
    pub competition: SelectionCompetition,
}

/// Competition context for squad selection. Replaces inferring the cup
/// rotation policy from a bare `knockout` flag — a `DomesticCup` tie
/// carries its bracket position and the two sides' reputations so the
/// selector can tell an early-round romp from a final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionCompetition {
    League,
    DomesticCup {
        /// 1-based round index within the bracket.
        round: u8,
        /// Total rounds needed to resolve the bracket (final == total).
        total_rounds: u8,
        /// Selecting side's reputation (market-value score).
        own_reputation: u16,
        /// Opponent's reputation (market-value score).
        opponent_reputation: u16,
    },
    ContinentalCup,
    Friendly,
}

impl SelectionCompetition {
    /// Build the per-side opportunity context for a domestic cup tie.
    /// `None` for league / continental / friendly games, which don't get
    /// the rotation bias.
    pub(crate) fn domestic_cup_context(&self, date: NaiveDate) -> Option<DomesticCupContext> {
        match *self {
            SelectionCompetition::DomesticCup {
                round,
                total_rounds,
                own_reputation,
                opponent_reputation,
            } => Some(DomesticCupContext {
                round,
                total_rounds,
                opponent_ratio: opponent_reputation as f32 / own_reputation.max(1) as f32,
                date,
            }),
            _ => None,
        }
    }
}

/// Knockout stage a domestic cup tie sits at, derived from the round
/// index and bracket size. Drives how hard the selector rotates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CupStage {
    /// Anything earlier than the quarterfinal — the rounds the manager rotates
    /// through most freely.
    Early,
    /// "Last eight" or equivalent third-from-final stage (`round + 2 ==
    /// total_rounds`). Still rotatable: its importance clamp deliberately stays
    /// in the early/mid band so a quarterfinal against a weak opponent can
    /// still be rested, but the opportunity bias is roughly half the early one.
    Quarter,
    /// Semifinal (`round + 1 == total_rounds`).
    Semi,
    /// Final (the last round, or a single-round bracket).
    Final,
}

impl CupStage {
    pub(crate) fn classify(round: u8, total_rounds: u8) -> Self {
        if total_rounds <= 1 || round >= total_rounds {
            CupStage::Final
        } else if round + 1 == total_rounds {
            CupStage::Semi
        } else if round + 2 == total_rounds {
            CupStage::Quarter
        } else {
            CupStage::Early
        }
    }
}

/// Per-side domestic-cup context consumed by the scoring engine's
/// opportunity bias: the bracket position decides the rotation strength
/// (via [`DomesticCupContext::stage`]), the reputation ratio nudges it back
/// toward a strong XI against a stronger opponent. `round`/`total_rounds` are
/// the raw bracket position — the single source of truth for the stage, and
/// readable for tests and debug output.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DomesticCupContext {
    pub round: u8,
    pub total_rounds: u8,
    pub opponent_ratio: f32,
    pub date: NaiveDate,
}

impl DomesticCupContext {
    /// Knockout stage this tie sits at, derived from its bracket position.
    pub(crate) fn stage(&self) -> CupStage {
        CupStage::classify(self.round, self.total_rounds)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionPolicy {
    BestEleven,
    StrongWithRotation,
    ManagedMinutes,
    CupRotation,
    YouthDevelopment,
}

impl SelectionPolicy {
    pub(crate) fn from_context(ctx: &SelectionContext) -> Self {
        if ctx.is_friendly || ctx.match_importance <= 0.20 {
            return SelectionPolicy::YouthDevelopment;
        }
        if ctx.match_importance < 0.40 {
            return SelectionPolicy::CupRotation;
        }
        if ctx.match_importance < 0.60 {
            return SelectionPolicy::ManagedMinutes;
        }
        if ctx.match_importance < 0.82 {
            return SelectionPolicy::StrongWithRotation;
        }
        SelectionPolicy::BestEleven
    }
}

impl Default for SelectionContext {
    fn default() -> Self {
        SelectionContext {
            is_friendly: false,
            date: Utc::now().date_naive(),
            match_importance: 0.7,
            philosophy: None,
            opponent_tactic: None,
            competition: SelectionCompetition::League,
        }
    }
}

impl SquadSelector {
    // ========== PUBLIC API ==========

    pub fn select(team: &Team, staff: &Staff) -> PlayerSelectionResult {
        Self::select_with_reserves(team, staff, &[])
    }

    pub fn select_with_reserves(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
    ) -> PlayerSelectionResult {
        Self::select_with_context(team, staff, reserve_players, &SelectionContext::default())
    }

    pub fn select_with_context(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let tactics = team.tactics();
        Self::select_with_tactics_context(team, staff, reserve_players, tactics.borrow(), ctx)
    }

    pub fn select_with_tactics_context(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
        tactics: &Tactics,
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let is_main_team = team.team_type == TeamType::Main;
        let engine =
            ScoringEngine::from_staff_for_team(staff, ctx.philosophy.clone(), is_main_team);
        let policy = SelectionPolicy::from_context(ctx);
        // Domestic-cup opportunity bias, built once per side. `None` for
        // league / continental / friendly games — those keep the existing
        // quality/readiness/status logic untouched.
        let cup = ctx.competition.domestic_cup_context(ctx.date);

        // Force-selection is a Main-team pin: a flagged player is committed
        // to the senior XI, so non-Main squads must drop them from every
        // entry path — both their own roster (a B-team player flagged for
        // the first team) and the cross-team reserve pool.
        //
        // Track membership in a side `HashSet<u32>` so reserve / keeper-
        // fallback duplicate checks are O(1) instead of O(n) per probe.
        let team_player_count = team.players.players().len();
        let estimated = team_player_count + reserve_players.len();
        let mut available: Vec<&Player> = Vec::with_capacity(estimated);
        let mut available_ids: HashSet<u32> = HashSet::with_capacity(estimated);
        for &p in team.players.players().iter() {
            if PlayerAvailability::is_available(p, ctx.is_friendly)
                && (is_main_team || !p.is_force_match_selection)
                && available_ids.insert(p.id)
            {
                available.push(p);
            }
        }

        for &rp in reserve_players {
            if !is_main_team && rp.is_force_match_selection {
                continue;
            }
            if PlayerAvailability::is_available(rp, ctx.is_friendly) && available_ids.insert(rp.id) {
                available.push(rp);
            }
        }

        // Keeper-fallback recovery. If the normal `is_available` filter
        // excluded every goalkeeper on the roster (all injured or on
        // international duty or low-condition), we'd then press an
        // outfielder into goal — but an outfielder has `Goalkeeping`
        // defaulted to zero, so their save rate is effectively 0% and
        // the team concedes 8-12 goals repeatably. Real football: if
        // all first-team keepers are unavailable, a walking-wounded /
        // low-condition keeper still starts over an outfielder. Add
        // them back here, skipping only keepers who physically can't
        // play (actively banned or absent).
        let has_fit_keeper = available.iter().any(|p| p.positions.is_goalkeeper());
        if !has_fit_keeper {
            for p in team.players.players().iter().copied() {
                // Re-admit only keepers who can physically play. The shared
                // helper skips injured / international-duty keepers and, in a
                // competitive match, banned ones — a low condition is the only
                // reason it relaxes. Force-selected players in non-Main squads
                // stay dropped (the Main-team pin is honoured elsewhere).
                if !KeeperAvailability::is_fallback_available(p, ctx.is_friendly) {
                    continue;
                }
                if !is_main_team && p.is_force_match_selection {
                    continue;
                }
                if !available_ids.insert(p.id) {
                    continue;
                }
                available.push(p);
            }
        }

        let outfield_count = available
            .iter()
            .filter(|p| !p.positions.is_goalkeeper())
            .count();
        let gk_count = available.len() - outfield_count;

        if available.len() < DEFAULT_SQUAD_SIZE {
            let all_players = team.players.players();
            let injured_count = all_players
                .iter()
                .filter(|p| p.player_attributes.is_injured)
                .count();
            let int_count = all_players
                .iter()
                .filter(|p| p.statuses.is_on_international_duty())
                .count();
            let low_condition = all_players
                .iter()
                .filter(|p| {
                    !p.player_attributes.is_injured
                        && p.player_attributes.condition_percentage() < HARD_CONDITION_FLOOR
                })
                .count();
            let banned_count = if !ctx.is_friendly {
                all_players
                    .iter()
                    .filter(|p| p.player_attributes.is_banned)
                    .count()
            } else {
                0
            };
            let lst_loa_count = if !ctx.is_friendly {
                all_players
                    .iter()
                    .filter(|p| {
                        let s = p.statuses.get();
                        s.contains(&PlayerStatusType::Lst) || s.contains(&PlayerStatusType::Loa)
                    })
                    .count()
            } else {
                0
            };

            log::debug!(
                "Squad selection for team {}: only {} available out of {} registered \
                (injured={}, international={}, low_condition={}, banned={}, lst_loa={}, \
                {} outfield, {} GK, {} reserves offered)",
                team.name,
                available.len(),
                all_players.len(),
                injured_count,
                int_count,
                low_condition,
                banned_count,
                lst_loa_count,
                outfield_count,
                gk_count,
                reserve_players.len()
            );
        } else if available.len() < DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE {
            debug!(
                "Squad selection for team {}: only {} available (need {} for full squad+bench)",
                team.name,
                available.len(),
                DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE
            );
        }

        let scx = competitive::SelectionScoringContext {
            staff,
            tactics,
            engine: &engine,
            date: ctx.date,
            is_friendly: ctx.is_friendly,
            match_importance: ctx.match_importance,
            policy,
            cup: cup.as_ref(),
        };

        let main_squad = scx.select_starting_eleven(team.id, &available);

        let main_squad_ids: HashSet<u32> = main_squad.iter().map(|mp| mp.id).collect();
        let remaining: Vec<&Player> = available
            .iter()
            .filter(|p| !main_squad_ids.contains(&p.id))
            .copied()
            .collect();

        let mut substitutes = scx.select_substitutes(team.id, &remaining);

        if substitutes.is_empty() && !remaining.is_empty() {
            debug!(
                "Substitute selection produced empty bench with {} remaining players — force-populating",
                remaining.len()
            );
            for player in &remaining {
                if substitutes.len() >= DEFAULT_BENCH_SIZE {
                    break;
                }
                let pos = best_tactical_position(player, tactics);
                substitutes.push(MatchPlayer::from_player(team.id, player, pos, false));
            }
        }

        debug!(
            "Final squad: {} starters, {} subs",
            main_squad.len(),
            substitutes.len()
        );

        let omissions = omissions::OmissionBuilder {
            available: &available,
            main_squad: &main_squad,
            substitutes: &substitutes,
            staff,
            tactics,
            engine: &engine,
            date: ctx.date,
            is_friendly: ctx.is_friendly,
            match_importance: ctx.match_importance,
            cup: cup.as_ref(),
        }
        .build();

        PlayerSelectionResult {
            main_squad,
            substitutes,
            omissions,
        }
    }

    // ========== ROTATION SELECTION ==========

    pub fn select_for_rotation(team: &Team, staff: &Staff) -> PlayerSelectionResult {
        Self::select_for_rotation_with_reserves(team, staff, &[])
    }

    pub fn select_for_rotation_with_reserves(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
    ) -> PlayerSelectionResult {
        Self::select_for_rotation_with_context(
            team,
            staff,
            reserve_players,
            &SelectionContext {
                is_friendly: true,
                ..SelectionContext::default()
            },
        )
    }

    pub fn select_for_rotation_with_context(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let tactics = team.tactics();
        let is_main_team = team.team_type == TeamType::Main;

        let team_player_count = team.players.players().len();
        let estimated = team_player_count + reserve_players.len();
        let mut available: Vec<&Player> = Vec::with_capacity(estimated);
        let mut available_ids: HashSet<u32> = HashSet::with_capacity(estimated);
        for &p in team.players.players().iter() {
            if PlayerAvailability::is_available(p, ctx.is_friendly)
                && (is_main_team || !p.is_force_match_selection)
                && available_ids.insert(p.id)
            {
                available.push(p);
            }
        }

        if available.len() < DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE {
            let needed = (DEFAULT_SQUAD_SIZE + DEFAULT_BENCH_SIZE) - available.len();
            let mut supplements: Vec<&Player> = reserve_players
                .iter()
                .filter(|&&rp| {
                    if !is_main_team && rp.is_force_match_selection {
                        return false;
                    }
                    PlayerAvailability::is_available(rp, ctx.is_friendly) && !available_ids.contains(&rp.id)
                })
                .copied()
                .collect();

            supplements.sort_by(|a, b| {
                b.player_attributes
                    .days_since_last_match
                    .cmp(&a.player_attributes.days_since_last_match)
            });

            for rp in supplements.into_iter().take(needed) {
                if available_ids.insert(rp.id) {
                    available.push(rp);
                }
            }

            if available.len() < DEFAULT_SQUAD_SIZE {
                debug!(
                    "Rotation selection for team {}: only {} available after borrowing ({} reserves offered)",
                    team.name,
                    available.len(),
                    reserve_players.len()
                );
            }
        }

        let main_squad =
            rotation::select_rotation_starting_eleven(team.id, &available, staff, tactics.borrow());

        let main_squad_ids: HashSet<u32> = main_squad.iter().map(|mp| mp.id).collect();
        let remaining: Vec<&Player> = available
            .iter()
            .filter(|p| !main_squad_ids.contains(&p.id))
            .copied()
            .collect();

        let mut substitutes =
            rotation::select_rotation_substitutes(team.id, &remaining, staff, tactics.borrow());

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

        // Rotation matches (friendlies / dev leagues) don't generate
        // morale-relevant drop events — every player is in line for a
        // run-out and an omission carries no professional sting.
        PlayerSelectionResult {
            main_squad,
            substitutes,
            omissions: Vec::new(),
        }
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
        let date = Utc::now().date_naive();
        engine.score_player_for_slot(player, position, group, staff, tactics, date, false, &[])
    }

    pub fn select_main_squad(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let engine = ScoringEngine::from_staff(staff);
        let scx = Self::legacy_scoring_context(staff, tactics, &engine);
        scx.select_starting_eleven(team_id, players)
    }

    pub fn select_substitutes_legacy(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let engine = ScoringEngine::from_staff(staff);
        let scx = Self::legacy_scoring_context(staff, tactics, &engine);
        scx.select_substitutes(team_id, players)
    }

    /// Scoring context for the legacy public selectors: today's date, a
    /// competitive non-friendly fixture with no cup bias, mid importance.
    fn legacy_scoring_context<'a>(
        staff: &'a Staff,
        tactics: &'a Tactics,
        engine: &'a ScoringEngine,
    ) -> competitive::SelectionScoringContext<'a> {
        competitive::SelectionScoringContext {
            staff,
            tactics,
            engine,
            date: Utc::now().date_naive(),
            is_friendly: false,
            match_importance: 0.7,
            policy: SelectionPolicy::StrongWithRotation,
            cup: None,
        }
    }
}
