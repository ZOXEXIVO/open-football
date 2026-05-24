//! Candidate collection, scoring, and balanced squad selection for
//! national-team call-ups. The pipeline shape is:
//!
//! 1. `collect_all_candidates_by_country` walks every continent and
//!    builds a `CallUpCandidate` for each eligible player, grouped by
//!    nationality, then ranks and trims each pool to the scouting cap.
//! 2. Per country, `call_up_squad` builds a `CallUpContext` from the
//!    date, captures incumbent player ids from the previous squad, and
//!    runs `select_balanced_squad` over the trimmed pool — quotas first,
//!    then a role-coverage pass, then reason derivation.
//!
//! The scoring layer is mode-aware: tournament finals reward proven
//! quality and experience, competitive windows favour current form,
//! friendly windows make room for youth and high-potential prospects.
//! Coach archetypes (see [`NationalCoachProfile`]) add a small
//! deterministic personality nudge.
//!
//! Dual nationality is intentionally out of scope here. Future support
//! should layer on top of [`is_eligible_for_country`] and would need:
//!   * a player-side list of secondary nationalities,
//!   * FIFA cap-tie rules (once a senior cap is won, eligibility locks),
//!   * residency / family-line eligibility predicates,
//!   * player refusal (a player can decline a call-up),
//!   * youth-to-senior switching windows.
//! All of those would gate inside `is_eligible_for_country` and the
//! scoring layer, not in the data pipeline above it.

use super::NationalTeam;
use super::types::{
    BREAK_WINDOWS, CallUpCandidate, CallUpContext, CallUpReason, CallUpWindowType,
    MIN_REAL_PLAYERS, NationalCoachProfile, NationalSquadPlayer, TOURNAMENT_SQUAD_SIZE,
};
use crate::{
    Country, Player, PlayerFieldPositionGroup, PlayerPositionType, PlayerStatistics,
    PlayerStatusType, Tactics, TeamType,
};
use chrono::{Datelike, NaiveDate};
use log::{debug, warn};
use std::collections::{HashMap, HashSet};

/// Tactical role grouping used by the role-coverage pass. A national
/// manager keeps an eye on whether they have left/right cover, a
/// central anchor, wide attacking outlets, etc — not just GK/DEF/MID/FWD
/// counts. Variants map onto whichever existing `PlayerPositionType`
/// values fit; missing variants in the enum (e.g. a country might never
/// have used wingbacks in the player generator) just contribute zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum RoleSlot {
    Goalkeeper,
    LeftDefensiveSide,
    RightDefensiveSide,
    CentralDefender,
    CentralOrDefensiveMid,
    WidePlayer,
    CentralForward,
}

impl RoleSlot {
    /// Position types that "cover" this role. A player covers a role
    /// when one of their position entries lists any of these.
    fn positions(self) -> &'static [PlayerPositionType] {
        use PlayerPositionType::*;
        match self {
            RoleSlot::Goalkeeper => &[Goalkeeper],
            RoleSlot::LeftDefensiveSide => &[DefenderLeft, WingbackLeft],
            RoleSlot::RightDefensiveSide => &[DefenderRight, WingbackRight],
            RoleSlot::CentralDefender => &[
                DefenderCenter,
                DefenderCenterLeft,
                DefenderCenterRight,
                Sweeper,
            ],
            RoleSlot::CentralOrDefensiveMid => &[
                DefensiveMidfielder,
                MidfielderCenter,
                MidfielderCenterLeft,
                MidfielderCenterRight,
            ],
            RoleSlot::WidePlayer => &[
                MidfielderLeft,
                MidfielderRight,
                AttackingMidfielderLeft,
                AttackingMidfielderRight,
                ForwardLeft,
                ForwardRight,
            ],
            RoleSlot::CentralForward => &[Striker, ForwardCenter, AttackingMidfielderCenter],
        }
    }
}

impl NationalTeam {
    /// Maximum candidate pool size returned to the squad selection stage.
    /// The coach scouts broadly but narrows down to a shortlist.
    pub(super) const MAX_CANDIDATE_POOL: usize = 60;

    /// True iff the player meets the basic eligibility rules to be
    /// considered for this country's national team. Currently only
    /// single-nationality matching — see module docs for the planned
    /// dual-nationality extension surface.
    pub(super) fn is_eligible_for_country(player: &Player, country_id: u32) -> bool {
        player.country_id == country_id
    }

    /// Collect eligible candidates from clubs across the supplied
    /// countries, grouped by nationality. Generic over the iterator so
    /// the same routine handles both continent-local pools and the
    /// world-wide pool (a Brazilian playing in Spain shows up under
    /// Brazil's bucket without any continent-specific plumbing).
    pub(crate) fn collect_all_candidates_by_country<'a, I>(
        countries: I,
        date: NaiveDate,
    ) -> HashMap<u32, Vec<CallUpCandidate>>
    where
        I: IntoIterator<Item = &'a Country>,
    {
        let mut map: HashMap<u32, Vec<CallUpCandidate>> = HashMap::new();

        for country in countries {
            for club in &country.clubs {
                for team in &club.teams.teams {
                    if team.team_type != TeamType::Main {
                        continue;
                    }

                    let league_reputation = team
                        .league_id
                        .and_then(|lid| {
                            country
                                .leagues
                                .leagues
                                .iter()
                                .find(|l| l.id == lid)
                                .map(|l| l.reputation)
                        })
                        .unwrap_or(0);
                    let club_reputation = team.reputation.world;

                    for player in &team.players.players {
                        if player.player_attributes.is_injured
                            || player.player_attributes.is_banned
                            || player.statuses.get().contains(&PlayerStatusType::Loa)
                            || player.player_attributes.condition < 5000
                        {
                            continue;
                        }

                        // Eligibility check is its own helper so a future
                        // dual-nationality pass can extend it without
                        // touching the candidate-collection skeleton.
                        if !Self::is_eligible_for_country(player, player.country_id) {
                            continue;
                        }

                        if let Some(candidate) = Self::build_candidate(
                            player,
                            club.id,
                            team.id,
                            club_reputation,
                            league_reputation,
                            date,
                        ) {
                            map.entry(player.country_id).or_default().push(candidate);
                        }
                    }
                }
            }
        }

        for candidates in map.values_mut() {
            let trimmed = Self::rank_and_trim_candidates(std::mem::take(candidates));
            *candidates = trimmed;
        }

        map
    }

    /// Build a CallUpCandidate from a player, if the player is worth scouting.
    /// Considers prior-season apps, caps, and ability — early-season call-ups
    /// must not exclude regulars who simply haven't accumulated this-season
    /// games yet.
    pub(super) fn build_candidate(
        player: &Player,
        club_id: u32,
        team_id: u32,
        club_reputation: u16,
        league_reputation: u16,
        date: NaiveDate,
    ) -> Option<CallUpCandidate> {
        let ability = player.player_attributes.current_ability;
        let potential = player.player_attributes.potential_ability;
        let age = date.year() - player.birth_date.year();
        let total_games = player.statistics.played + player.statistics.played_subs;

        let (last_season_apps, last_season_rating, last_season_goals) =
            Self::summarise_last_season(player);

        let international_caps = player.player_attributes.international_apps;

        // Refined activity filter: at least one of these must hold.
        // The threshold mix is wider than "5 games this season" because
        // mid-tournament cycles and post-injury comebacks would
        // otherwise filter out genuine first-teamers.
        let promising_youth = age <= 21 && potential >= 80 && total_games >= 3;
        let active_now = total_games >= 5;
        let veteran_history = last_season_apps >= 10;
        let proven_international = international_caps >= 5;
        let has_track_record = veteran_history || proven_international;

        if !(active_now || veteran_history || proven_international || promising_youth) {
            return None;
        }

        // Ability gate: weak players can sneak in if they're either a
        // promising youth, a recent regular, or already an international.
        if ability < 40 && !promising_youth && !has_track_record {
            return None;
        }

        let condition_pct = (player.player_attributes.condition as f32 / 10000.0) * 100.0;

        let position_levels: Vec<(PlayerPositionType, u8)> = player
            .positions
            .positions
            .iter()
            .map(|pp| (pp.position, pp.level))
            .collect();

        let position_group = player
            .positions
            .positions
            .iter()
            .max_by_key(|p| p.level)
            .map(|p| p.position.position_group())
            .unwrap_or(PlayerFieldPositionGroup::Midfielder);

        Some(CallUpCandidate {
            player_id: player.id,
            club_id,
            team_id,
            current_ability: ability,
            potential_ability: potential,
            age,
            condition_pct,
            match_readiness: player.skills.physical.match_readiness,
            average_rating: player.statistics.average_rating_realistic(position_group),
            played: total_games,
            international_apps: international_caps,
            international_goals: player.player_attributes.international_goals,
            leadership: player.skills.mental.leadership,
            composure: player.skills.mental.composure,
            teamwork: player.skills.mental.teamwork,
            determination: player.skills.mental.determination,
            pressure_handling: player.attributes.pressure,
            world_reputation: player.player_attributes.world_reputation,
            club_reputation,
            league_reputation,
            position_levels,
            position_group,
            goals: player.statistics.goals,
            assists: player.statistics.assists,
            player_of_the_match: player.statistics.player_of_the_match,
            clean_sheets: player.statistics.clean_sheets,
            yellow_cards: player.statistics.yellow_cards,
            red_cards: player.statistics.red_cards,
            last_season_apps,
            last_season_rating,
            last_season_goals,
        })
    }

    /// Summarise a player's most recent prior season into (apps, rating, goals).
    /// Several frozen items can share a season (mid-season transfer/loan) so
    /// games are summed and the per-item ledgers are merged via the same
    /// fallback used by `PlayerStatistics::merge_from` before regression.
    pub(super) fn summarise_last_season(player: &Player) -> (u16, f32, u16) {
        let last_year = match player
            .statistics_history
            .items
            .iter()
            .map(|i| i.season.start_year)
            .max()
        {
            Some(y) => y,
            None => return (0, 0.0, 0),
        };

        let mut apps: u16 = 0;
        let mut goals: u16 = 0;
        let mut combined = PlayerStatistics::default();
        for item in &player.statistics_history.items {
            if item.season.start_year != last_year {
                continue;
            }
            let games = item
                .statistics
                .played
                .saturating_add(item.statistics.played_subs);
            apps = apps.saturating_add(games);
            goals = goals.saturating_add(item.statistics.goals);
            combined.merge_from(&item.statistics);
        }
        let pos = player.position().position_group();
        let rating = combined.average_rating_realistic(pos);
        (apps, rating, goals)
    }

    /// Rank candidates by the realistic scouting score and trim to the
    /// pool cap. Weaker nations still produce a full candidate pool with
    /// their best available players.
    fn rank_and_trim_candidates(mut candidates: Vec<CallUpCandidate>) -> Vec<CallUpCandidate> {
        if candidates.len() <= Self::MAX_CANDIDATE_POOL {
            return candidates;
        }

        candidates.sort_by(|a, b| {
            let score_a = Self::scouting_score(a);
            let score_b = Self::scouting_score(b);
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates.truncate(Self::MAX_CANDIDATE_POOL);
        candidates
    }

    /// Scouting score used to trim the candidate pool. Mirrors the
    /// shape of real-world scouting reports: raw ability, current form,
    /// last season's body of work, league/club/world reputation, an
    /// international track-record signal, a youth-ceiling lift, minus
    /// discipline and age-decline penalties.
    fn scouting_score(c: &CallUpCandidate) -> f32 {
        let current_ability = c.current_ability as f32;

        // Current form: apps-weight and rating-weight, fallback rating
        // 18 for players with no current rating so we don't zero them
        // out (matches "scout had to estimate from prior season").
        let apps_factor = (c.played.min(20) as f32) / 20.0 * 35.0;
        let rating_factor = if c.average_rating > 0.0 {
            c.average_rating / 10.0 * 35.0
        } else {
            18.0
        };
        let current_form_component = apps_factor + rating_factor;

        let last_apps_factor = (c.last_season_apps.min(35) as f32) / 35.0 * 25.0;
        let last_rating_factor = if c.last_season_rating > 0.0 {
            c.last_season_rating / 10.0 * 25.0
        } else {
            0.0
        };
        let last_season_component = last_apps_factor + last_rating_factor;

        let league_reputation_component = (c.league_reputation as f32 / 1000.0 * 100.0).min(100.0);
        let club_reputation_component = (c.club_reputation as f32 / 10000.0 * 100.0).min(100.0);
        let world_reputation_component =
            (c.world_reputation.max(0) as f32 / 8000.0 * 100.0).clamp(0.0, 100.0);

        let int_apps_factor = (c.international_apps.min(60) as f32) / 60.0 * 70.0;
        let int_goals_factor = (c.international_goals.min(25) as f32) / 25.0 * 30.0;
        let international_track_record = int_apps_factor + int_goals_factor;

        let ceiling = (c.potential_ability as i16 - c.current_ability as i16).max(0) as f32;
        let youth_ceiling_bonus = if c.age <= 21 {
            ceiling * 0.45
        } else if c.age <= 24 {
            ceiling * 0.25
        } else {
            0.0
        };

        let discipline_penalty = c.red_cards as f32 * 8.0 + c.yellow_cards as f32 * 1.2;
        let age_decline_risk = match c.age {
            33..=34 => 4.0,
            35..=36 => 8.0,
            _ if c.age >= 37 => 14.0,
            _ => 0.0,
        };

        current_ability * 1.00
            + current_form_component * 0.75
            + last_season_component * 0.35
            + league_reputation_component * 0.25
            + club_reputation_component * 0.08
            + world_reputation_component * 0.20
            + international_track_record * 0.25
            + youth_ceiling_bonus
            - discipline_penalty
            - age_decline_risk
    }

    /// Determine which call-up window a date falls inside. Defaults to
    /// `CompetitiveWindow` so the public callers (which only ever fire
    /// on break/tournament starts) always get a sensible value.
    pub(super) fn window_for_date(date: NaiveDate) -> CallUpWindowType {
        if Self::is_in_tournament_period(date) || Self::is_tournament_start(date) {
            CallUpWindowType::TournamentFinals
        } else {
            // The current data model doesn't distinguish "this break has
            // qualifiers" from "this break is friendly-only". Until that
            // arrives, treat all regular breaks as competitive — the
            // friendly path is exercised in tests and is reachable via
            // direct API for future use.
            CallUpWindowType::CompetitiveWindow
        }
    }

    /// Call up squad using context-aware scoring — considers ability,
    /// tactical fit, form, experience, mentality, age, continuity, role
    /// coverage, and coach archetype. Tournament finals favour proven
    /// quality; friendlies make room for youth.
    ///
    /// The `Int` status of called-up players is applied separately, in
    /// a continent-wide pass after every country has selected (foreign-
    /// based players play at clubs in another country, so we can't
    /// reach them from here without breaking the borrow on `&mut self`).
    pub(crate) fn call_up_squad(
        &mut self,
        candidates: Vec<CallUpCandidate>,
        date: NaiveDate,
        country_id: u32,
        country_ids: &[(u32, String)],
    ) {
        // Capture incumbents BEFORE we clear — continuity scoring needs
        // to know who held a place in the prior cycle.
        let incumbents: HashSet<u32> = self.squad.iter().map(|sp| sp.player_id).collect();

        self.squad.clear();
        self.generated_squad.clear();

        let window_type = Self::window_for_date(date);
        let ctx = CallUpContext::new(date, country_id, window_type);

        let selected = Self::select_balanced_squad(&candidates, &self.tactics, &ctx, &incumbents);

        for (idx, reason, secondaries) in &selected {
            let c = &candidates[*idx];
            self.squad.push(NationalSquadPlayer {
                player_id: c.player_id,
                club_id: c.club_id,
                team_id: c.team_id,
                primary_reason: *reason,
                secondary_reasons: secondaries.clone(),
            });
        }

        let real_count = self.squad.len();
        if real_count < MIN_REAL_PLAYERS {
            warn!(
                "National team {} ({}) generated synthetic squad: real_selected={}, min_required={}",
                self.country_name, country_id, real_count, MIN_REAL_PLAYERS
            );
            self.generate_synthetic_squad(date);
        }

        // Schedule retention rules:
        //   - completed fixtures (with results) are permanent history
        //   - pending fixtures in the past never played → drop (stale)
        //   - pending fixtures in the current break window → drop so we
        //     can re-add fresh friendlies without duplicates
        //   - pending fixtures in a future window stay untouched
        self.schedule.retain(|f| {
            if f.result.is_some() {
                return true;
            }
            if f.date < date {
                return false;
            }
            if Self::dates_in_same_break_window(f.date, date) {
                return false;
            }
            true
        });

        // Friendly fixtures intentionally not auto-scheduled here; see
        // the comment in the previous implementation for the rationale.
        let _ = (window_type, country_ids);

        debug!(
            "National team {} (country {}) called up {} players ({} from clubs, {} synthetic) for window {:?}",
            self.country_name,
            country_id,
            self.squad.len() + self.generated_squad.len(),
            self.squad.len(),
            self.generated_squad.len(),
            window_type
        );
    }

    /// True iff `a` and `b` fall in the same scheduled break window.
    fn dates_in_same_break_window(a: NaiveDate, b: NaiveDate) -> bool {
        BREAK_WINDOWS.iter().any(|(month, start, end)| {
            a.year() == b.year()
                && a.month() == *month
                && b.month() == *month
                && a.day() >= *start
                && a.day() <= *end
                && b.day() >= *start
                && b.day() <= *end
        })
    }

    // ---------- Component score helpers (each returns 0..100) ----------

    fn ability_score(c: &CallUpCandidate) -> f32 {
        (c.current_ability as f32 / 200.0 * 100.0).clamp(0.0, 100.0)
    }

    fn league_score(c: &CallUpCandidate) -> f32 {
        (c.league_reputation as f32 / 1000.0 * 100.0).clamp(0.0, 100.0)
    }

    /// Best matching position level among the tactic's required slots.
    fn tactical_score(c: &CallUpCandidate, tactics: &Tactics) -> f32 {
        let required_positions = tactics.positions();
        let best = required_positions
            .iter()
            .filter_map(|&pos| {
                c.position_levels
                    .iter()
                    .find(|(p, _)| *p == pos)
                    .map(|(_, level)| *level as f32)
            })
            .fold(0.0f32, f32::max);
        (best / 20.0 * 100.0).clamp(0.0, 100.0)
    }

    /// Highest natural-role level across role slots the manager cares
    /// about. Falls back to `tactical_score` if the player has no entry
    /// in any of the role slot lists — keeps the metric defined for
    /// every candidate.
    fn role_fit_score(c: &CallUpCandidate, tactics: &Tactics) -> f32 {
        let mut best: u8 = 0;
        for slot in [
            RoleSlot::Goalkeeper,
            RoleSlot::LeftDefensiveSide,
            RoleSlot::RightDefensiveSide,
            RoleSlot::CentralDefender,
            RoleSlot::CentralOrDefensiveMid,
            RoleSlot::WidePlayer,
            RoleSlot::CentralForward,
        ] {
            for &p in slot.positions() {
                for (pp, lvl) in &c.position_levels {
                    if *pp == p && *lvl > best {
                        best = *lvl;
                    }
                }
            }
        }
        if best == 0 {
            return Self::tactical_score(c, tactics);
        }
        (best as f32 / 20.0 * 100.0).clamp(0.0, 100.0)
    }

    fn form_score(c: &CallUpCandidate) -> f32 {
        let condition_norm = c.condition_pct.clamp(0.0, 100.0);
        let match_readiness_norm = (c.match_readiness / 20.0).clamp(0.0, 1.0) * 100.0;
        let effective_rating = if c.played >= 3 {
            c.average_rating
        } else if c.last_season_rating > 0.0 {
            c.last_season_rating
        } else {
            c.average_rating
        };
        let rating_norm = if effective_rating > 0.0 {
            (effective_rating / 10.0).clamp(0.0, 1.0) * 100.0
        } else {
            30.0
        };
        let blended_games = c.played as f32 + c.last_season_apps as f32 * 0.35;
        let games_norm = (blended_games.min(22.0) / 22.0) * 100.0;

        condition_norm * 0.20 + match_readiness_norm * 0.25 + rating_norm * 0.30 + games_norm * 0.25
    }

    fn experience_score(c: &CallUpCandidate) -> f32 {
        let world_reputation_norm =
            (c.world_reputation.max(0) as f32 / 8000.0 * 100.0).clamp(0.0, 100.0);
        let international_apps_norm = (c.international_apps.min(80) as f32) / 80.0 * 100.0;
        let international_goals_norm = (c.international_goals.min(35) as f32) / 35.0 * 100.0;
        let club_reputation_norm = (c.club_reputation as f32 / 10000.0 * 100.0).clamp(0.0, 100.0);

        world_reputation_norm * 0.35
            + international_apps_norm * 0.40
            + international_goals_norm * 0.15
            + club_reputation_norm * 0.10
    }

    fn mental_score(c: &CallUpCandidate) -> f32 {
        let avg =
            (c.leadership + c.composure + c.teamwork + c.determination + c.pressure_handling) / 5.0;
        (avg / 20.0 * 100.0).clamp(0.0, 100.0)
    }

    fn potential_score(c: &CallUpCandidate) -> f32 {
        (c.potential_ability as f32 / 200.0 * 100.0).clamp(0.0, 100.0)
    }

    fn impact_score(c: &CallUpCandidate) -> f32 {
        let blended_goals = c.goals as f32 + c.last_season_goals as f32 * 0.40;
        let blended_games = (c.played as f32 + c.last_season_apps as f32 * 0.40).max(1.0);
        let blended = c.played >= 5;
        let total_games_f = (c.played as f32).max(1.0);

        let goals_per_game = if blended {
            c.goals as f32 / total_games_f
        } else {
            blended_goals / blended_games
        };
        let assists_per_game = if blended {
            c.assists as f32 / total_games_f
        } else {
            (c.assists as f32) / blended_games
        };
        let clean_sheets_per_game = if blended {
            c.clean_sheets as f32 / total_games_f
        } else {
            (c.clean_sheets as f32) / blended_games
        };

        let pom_norm = (c.player_of_the_match as f32).min(8.0) / 8.0;
        let discipline_penalty = c.red_cards as f32 * 10.0 + c.yellow_cards as f32 * 1.5;

        match c.position_group {
            PlayerFieldPositionGroup::Forward => {
                let g = (goals_per_game * 90.0).min(50.0);
                let a = (assists_per_game * 45.0).min(20.0);
                let p = pom_norm * 20.0;
                (g + a + p - discipline_penalty).clamp(0.0, 100.0)
            }
            PlayerFieldPositionGroup::Midfielder => {
                let a = (assists_per_game * 80.0).min(35.0);
                let g = (goals_per_game * 50.0).min(20.0);
                let p = pom_norm * 20.0;
                (a + g + p - discipline_penalty).clamp(0.0, 100.0)
            }
            PlayerFieldPositionGroup::Defender => {
                let cs = (clean_sheets_per_game * 75.0).min(35.0);
                let g = (goals_per_game * 35.0).min(10.0);
                let p = pom_norm * 20.0;
                (cs + g + p - discipline_penalty).clamp(0.0, 100.0)
            }
            PlayerFieldPositionGroup::Goalkeeper => {
                let cs = (clean_sheets_per_game * 90.0).min(45.0);
                let p = pom_norm * 25.0;
                (cs + p - discipline_penalty).clamp(0.0, 100.0)
            }
        }
    }

    fn age_score(c: &CallUpCandidate, window: CallUpWindowType) -> f32 {
        match window {
            CallUpWindowType::TournamentFinals => match c.age {
                ..=20 => 45.0,
                21..=23 => 65.0,
                24..=29 => 95.0,
                30..=32 => 82.0,
                33..=34 => 62.0,
                35..=36 => 40.0,
                _ => 25.0,
            },
            CallUpWindowType::CompetitiveWindow => match c.age {
                ..=20 => 55.0,
                21..=23 => 75.0,
                24..=29 => 90.0,
                30..=32 => 80.0,
                33..=34 => 58.0,
                35..=36 => 35.0,
                _ => 20.0,
            },
            CallUpWindowType::FriendlyWindow => match c.age {
                ..=20 => 90.0,
                21..=23 => 92.0,
                24..=26 => 78.0,
                27..=29 => 58.0,
                30..=32 => 35.0,
                33..=34 => 18.0,
                _ => 8.0,
            },
        }
    }

    fn continuity_score(
        c: &CallUpCandidate,
        window: CallUpWindowType,
        incumbents: &HashSet<u32>,
    ) -> f32 {
        let in_prior_squad = incumbents.contains(&c.player_id);
        let raw: f32 = if in_prior_squad {
            75.0
        } else if c.international_apps >= 50 {
            70.0
        } else if c.international_apps >= 20 {
            55.0
        } else if c.international_apps >= 5 {
            35.0
        } else if c.international_apps >= 1 {
            18.0
        } else {
            0.0
        };

        // Friendly windows must leave room for fresh blood — cap
        // continuity unless the player is still young enough to remain
        // a long-term project.
        if window == CallUpWindowType::FriendlyWindow && c.age > 24 {
            raw.min(35.0)
        } else {
            raw
        }
    }

    /// Coach-personality nudge. Bounded to roughly +/-6 so it's a
    /// tiebreaker, never a dominant factor.
    fn coach_bias(c: &CallUpCandidate, tactics: &Tactics, country_id: u32) -> f32 {
        let profile = NationalCoachProfile::for_country(country_id);
        let best_tactic_level = {
            let required = tactics.positions();
            required
                .iter()
                .filter_map(|&pos| {
                    c.position_levels
                        .iter()
                        .find(|(p, _)| *p == pos)
                        .map(|(_, level)| *level)
                })
                .fold(0u8, std::cmp::max)
        };

        match profile {
            NationalCoachProfile::Conservative => {
                let mut bias = 0.0;
                if c.international_apps >= 40 {
                    bias += 6.0;
                }
                if (27..=32).contains(&c.age) {
                    bias += 3.0;
                }
                if c.international_apps == 0 {
                    bias -= 3.0;
                }
                bias
            }
            NationalCoachProfile::YouthDeveloper => {
                let mut bias = 0.0;
                if c.age <= 23 && c.potential_ability >= 140 {
                    bias += 6.0;
                }
                if c.age <= 24 && c.international_apps == 0 {
                    bias += 3.0;
                }
                if c.age >= 33 {
                    bias -= 2.0;
                }
                bias
            }
            NationalCoachProfile::StarDriven => {
                let mut bias = 0.0;
                if c.world_reputation >= 5000 {
                    bias += 6.0;
                }
                if c.club_reputation >= 7000 {
                    bias += 3.0;
                }
                bias
            }
            NationalCoachProfile::FormDriven => {
                let mut bias = 0.0;
                if c.average_rating >= 7.4 && c.played >= 5 {
                    bias += 6.0;
                }
                if c.player_of_the_match >= 2 {
                    bias += 3.0;
                }
                if c.average_rating < 6.5 && c.played >= 5 {
                    bias -= 4.0;
                }
                bias
            }
            NationalCoachProfile::TacticalSpecialist => {
                let mut bias = 0.0;
                if best_tactic_level >= 18 {
                    bias += 6.0;
                }
                if best_tactic_level >= 14 {
                    bias += 3.0;
                }
                if best_tactic_level < 12 {
                    bias -= 3.0;
                }
                bias
            }
        }
    }

    /// Friendly-window experimentation lever: nudge uncapped youngsters
    /// up the order, push aging veterans (who aren't elite quality) out.
    fn experimentation_bonus(c: &CallUpCandidate, ability_score: f32) -> f32 {
        let mut bonus = 0.0;
        if c.international_apps == 0 && c.age <= 21 {
            bonus += 8.0;
        } else if c.international_apps == 0 && c.age <= 24 {
            bonus += 6.0;
        } else if c.international_apps <= 3 && c.age <= 24 {
            bonus += 4.0;
        }
        if c.age >= 31 && c.international_apps >= 40 && ability_score < 82.0 {
            bonus -= 5.0;
        }
        bonus
    }

    /// Main scoring entry point — composes the per-axis component
    /// scores into a single number per candidate, with mode-specific
    /// weights and a coach bias term.
    pub(super) fn score_candidate(
        c: &CallUpCandidate,
        tactics: &Tactics,
        ctx: &CallUpContext,
        incumbents: &HashSet<u32>,
    ) -> f32 {
        let ability = Self::ability_score(c);
        let league = Self::league_score(c);
        let tactical = Self::tactical_score(c, tactics);
        let role_fit = Self::role_fit_score(c, tactics);
        let form = Self::form_score(c);
        let experience = Self::experience_score(c);
        let mental = Self::mental_score(c);
        let age_s = Self::age_score(c, ctx.window_type);
        let potential = Self::potential_score(c);
        let continuity = Self::continuity_score(c, ctx.window_type, incumbents);
        let impact = Self::impact_score(c);
        let bias = Self::coach_bias(c, tactics, ctx.country_id);

        let weighted = match ctx.window_type {
            CallUpWindowType::TournamentFinals => {
                ability * 0.22
                    + form * 0.17
                    + experience * 0.16
                    + tactical * 0.11
                    + role_fit * 0.09
                    + impact * 0.10
                    + mental * 0.07
                    + age_s * 0.05
                    + continuity * 0.03
                    + league * 0.00
            }
            CallUpWindowType::CompetitiveWindow => {
                ability * 0.20
                    + form * 0.21
                    + tactical * 0.12
                    + role_fit * 0.10
                    + impact * 0.12
                    + experience * 0.09
                    + mental * 0.06
                    + age_s * 0.04
                    + continuity * 0.06
                    + league * 0.00
            }
            CallUpWindowType::FriendlyWindow => {
                let experiment = Self::experimentation_bonus(c, ability);
                ability * 0.14
                    + form * 0.15
                    + tactical * 0.08
                    + role_fit * 0.08
                    + impact * 0.09
                    + experience * 0.03
                    + mental * 0.04
                    + age_s * 0.12
                    + potential * 0.17
                    + continuity * 0.02
                    + experiment
            }
        };

        // League nudge applied as a small bias regardless of mode — the
        // top-flight signal is too important to fully zero out, even
        // though the explicit weights table doesn't list it.
        let league_nudge = league * 0.04;
        weighted + bias + league_nudge
    }

    /// Compute coverage counts for each `RoleSlot` across a set of
    /// candidate indices. A candidate counts toward a role when any of
    /// their position entries hits a level >= 12 in that role's
    /// position list.
    fn role_coverage_counts(
        candidates: &[CallUpCandidate],
        indices: impl Iterator<Item = usize>,
    ) -> HashMap<RoleSlot, usize> {
        let slots = [
            RoleSlot::Goalkeeper,
            RoleSlot::LeftDefensiveSide,
            RoleSlot::RightDefensiveSide,
            RoleSlot::CentralDefender,
            RoleSlot::CentralOrDefensiveMid,
            RoleSlot::WidePlayer,
            RoleSlot::CentralForward,
        ];

        let mut counts: HashMap<RoleSlot, usize> = slots.iter().copied().map(|s| (s, 0)).collect();

        for idx in indices {
            let c = &candidates[idx];
            for slot in slots {
                let covers = slot.positions().iter().any(|p| {
                    c.position_levels
                        .iter()
                        .any(|(pp, lvl)| pp == p && *lvl >= 12)
                });
                if covers {
                    *counts.entry(slot).or_insert(0) += 1;
                }
            }
        }
        counts
    }

    /// Coverage target per slot, depending on squad size.
    fn coverage_targets(target_squad_size: usize) -> HashMap<RoleSlot, usize> {
        let mut t: HashMap<RoleSlot, usize> = HashMap::new();
        if target_squad_size >= TOURNAMENT_SQUAD_SIZE {
            t.insert(RoleSlot::Goalkeeper, 3);
            t.insert(RoleSlot::LeftDefensiveSide, 2);
            t.insert(RoleSlot::RightDefensiveSide, 2);
            t.insert(RoleSlot::CentralDefender, 3);
            t.insert(RoleSlot::CentralOrDefensiveMid, 4);
            t.insert(RoleSlot::WidePlayer, 3);
            t.insert(RoleSlot::CentralForward, 2);
        } else {
            t.insert(RoleSlot::Goalkeeper, 3);
            t.insert(RoleSlot::LeftDefensiveSide, 1);
            t.insert(RoleSlot::RightDefensiveSide, 1);
            t.insert(RoleSlot::CentralDefender, 3);
            t.insert(RoleSlot::CentralOrDefensiveMid, 3);
            t.insert(RoleSlot::WidePlayer, 2);
            t.insert(RoleSlot::CentralForward, 2);
        }
        t
    }

    /// Best natural level the candidate has in any of a role's
    /// covering positions.
    fn best_level_for_slot(c: &CallUpCandidate, slot: RoleSlot) -> u8 {
        slot.positions()
            .iter()
            .filter_map(|p| {
                c.position_levels
                    .iter()
                    .find(|(pp, _)| pp == p)
                    .map(|(_, lvl)| *lvl)
            })
            .max()
            .unwrap_or(0)
    }

    /// Select a balanced squad respecting positional quotas, then run a
    /// role-coverage pass that swaps in better positional fits where
    /// the manager would in practice.
    ///
    /// Returns `(candidate_index, primary_reason, secondary_reasons)`.
    pub(super) fn select_balanced_squad(
        candidates: &[CallUpCandidate],
        tactics: &Tactics,
        ctx: &CallUpContext,
        incumbents: &HashSet<u32>,
    ) -> Vec<(usize, CallUpReason, Vec<CallUpReason>)> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let scored: Vec<(usize, f32)> = candidates
            .iter()
            .enumerate()
            .map(|(idx, c)| (idx, Self::score_candidate(c, tactics, ctx, incumbents)))
            .collect();

        // Phase 1: broad positional quotas (GK/DEF/MID/FWD).
        let [gk_quota, def_quota, mid_quota, fwd_quota] =
            Self::positional_quotas(tactics, ctx.target_squad_size);

        let desc = |a: &(usize, f32), b: &(usize, f32)| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        };

        let by_group = |group: PlayerFieldPositionGroup| {
            let mut v: Vec<(usize, f32)> = scored
                .iter()
                .filter(|(i, _)| candidates[*i].position_group == group)
                .copied()
                .collect();
            v.sort_by(&desc);
            v
        };

        let gk = by_group(PlayerFieldPositionGroup::Goalkeeper);
        let def = by_group(PlayerFieldPositionGroup::Defender);
        let mid = by_group(PlayerFieldPositionGroup::Midfielder);
        let fwd = by_group(PlayerFieldPositionGroup::Forward);

        // (idx, role-coverage flag, position-need flag, score)
        let mut selected: Vec<(usize, bool, bool, f32)> = Vec::with_capacity(ctx.target_squad_size);
        let mut taken: HashSet<usize> = HashSet::new();

        let take_group = |group: &[(usize, f32)],
                          quota: usize,
                          selected: &mut Vec<(usize, bool, bool, f32)>,
                          taken: &mut HashSet<usize>| {
            for (rank, &(idx, score)) in group.iter().take(quota).enumerate() {
                let c = &candidates[idx];
                let position_need = rank >= quota / 2 && c.current_ability < 130;
                selected.push((idx, false, position_need, score));
                taken.insert(idx);
            }
        };

        take_group(&gk, gk_quota, &mut selected, &mut taken);
        take_group(&def, def_quota, &mut selected, &mut taken);
        take_group(&mid, mid_quota, &mut selected, &mut taken);
        take_group(&fwd, fwd_quota, &mut selected, &mut taken);

        // Top up to target_squad_size from leftover candidates.
        if selected.len() < ctx.target_squad_size {
            let mut leftover: Vec<(usize, f32)> = scored
                .iter()
                .filter(|(i, _)| !taken.contains(i))
                .copied()
                .collect();
            leftover.sort_by(&desc);
            for (idx, score) in leftover {
                if selected.len() >= ctx.target_squad_size {
                    break;
                }
                selected.push((idx, false, false, score));
                taken.insert(idx);
            }
        }
        selected.truncate(ctx.target_squad_size);

        // Phase 2: role coverage. For each slot under target, attempt
        // to swap in the best unselected candidate who fills the gap.
        Self::apply_role_coverage(candidates, &mut selected, &mut taken, &scored, ctx);

        // Phase 3: derive reasons.
        let mut out: Vec<(usize, CallUpReason, Vec<CallUpReason>)> =
            Vec::with_capacity(selected.len());
        for (idx, role_cov, pos_need, _score) in selected {
            let c = &candidates[idx];
            let (primary, secondaries) =
                Self::derive_reasons(c, pos_need, role_cov, ctx, incumbents);
            out.push((idx, primary, secondaries));
        }
        out
    }

    /// Walk the role-coverage deficit list and swap the weakest non-
    /// critical selection for the best unselected candidate who fills
    /// the missing role. Bounded by minimum quality and a tolerance on
    /// the score gap, so a clearly stronger general player isn't
    /// dropped for a marginal role fit.
    fn apply_role_coverage(
        candidates: &[CallUpCandidate],
        selected: &mut Vec<(usize, bool, bool, f32)>,
        taken: &mut HashSet<usize>,
        scored: &[(usize, f32)],
        ctx: &CallUpContext,
    ) {
        let targets = Self::coverage_targets(ctx.target_squad_size);
        let min_quality_level: u8 = 14;
        let mut tolerance = match ctx.window_type {
            CallUpWindowType::TournamentFinals => 6.0,
            CallUpWindowType::CompetitiveWindow => 8.0,
            CallUpWindowType::FriendlyWindow => 8.0,
        };

        // Process each slot independently; each iteration may modify
        // `selected` so recompute coverage on each pass.
        let slots = [
            RoleSlot::Goalkeeper,
            RoleSlot::LeftDefensiveSide,
            RoleSlot::RightDefensiveSide,
            RoleSlot::CentralDefender,
            RoleSlot::CentralOrDefensiveMid,
            RoleSlot::WidePlayer,
            RoleSlot::CentralForward,
        ];

        for slot in slots {
            let target = *targets.get(&slot).unwrap_or(&0);
            loop {
                let coverage =
                    Self::role_coverage_counts(candidates, selected.iter().map(|(i, _, _, _)| *i));
                let have = *coverage.get(&slot).unwrap_or(&0);
                if have >= target {
                    break;
                }

                // Best unselected candidate for this slot, level >= min_quality_level.
                let incoming: Option<(usize, f32, u8)> = scored
                    .iter()
                    .filter(|(i, _)| !taken.contains(i))
                    .filter_map(|(i, score)| {
                        let lvl = Self::best_level_for_slot(&candidates[*i], slot);
                        if lvl >= min_quality_level {
                            Some((*i, *score, lvl))
                        } else {
                            None
                        }
                    })
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

                let (in_idx, in_score, _in_lvl) = match incoming {
                    Some(v) => v,
                    None => break,
                };

                // Friendly U24 uncapped picks get a wider tolerance band.
                let incoming_c = &candidates[in_idx];
                let local_tolerance = if ctx.window_type == CallUpWindowType::FriendlyWindow
                    && incoming_c.age <= 24
                    && incoming_c.international_apps == 0
                {
                    12.0
                } else {
                    tolerance
                };

                // Pick the lowest-scoring outgoing whose removal does
                // not create another deficit.
                let mut best_out: Option<(usize, f32)> = None;
                for (pos_in_selected, (out_idx, _role_cov, _pos_need, out_score)) in
                    selected.iter().enumerate()
                {
                    // Don't evict a candidate that's the *only* cover
                    // for any slot at target.
                    let out_c = &candidates[*out_idx];

                    let mut creates_deficit = false;
                    for other_slot in slots {
                        let other_target = *targets.get(&other_slot).unwrap_or(&0);
                        if other_target == 0 {
                            continue;
                        }
                        let other_have = *coverage.get(&other_slot).unwrap_or(&0);
                        let out_covers = other_slot.positions().iter().any(|p| {
                            out_c
                                .position_levels
                                .iter()
                                .any(|(pp, lvl)| pp == p && *lvl >= 12)
                        });
                        if out_covers && other_have <= other_target {
                            creates_deficit = true;
                            break;
                        }
                    }
                    if creates_deficit {
                        continue;
                    }

                    if in_score < *out_score - local_tolerance {
                        continue;
                    }

                    match best_out {
                        None => best_out = Some((pos_in_selected, *out_score)),
                        Some((_, prev)) if *out_score < prev => {
                            best_out = Some((pos_in_selected, *out_score))
                        }
                        _ => {}
                    }
                }

                let (out_pos, _) = match best_out {
                    Some(v) => v,
                    None => break,
                };

                let evicted_idx = selected[out_pos].0;
                taken.remove(&evicted_idx);
                taken.insert(in_idx);
                selected[out_pos] = (in_idx, true, false, in_score);

                // Tighten tolerance once a swap happens so we don't
                // ping-pong wildly through the squad for fringe gains.
                tolerance = (tolerance - 0.5).max(0.0);
            }
        }
    }

    /// Decide a player's primary call-up reason from their candidate
    /// profile, plus a list of secondary reasons that also apply.
    pub(super) fn derive_reasons(
        c: &CallUpCandidate,
        position_need: bool,
        role_coverage: bool,
        ctx: &CallUpContext,
        incumbents: &HashSet<u32>,
    ) -> (CallUpReason, Vec<CallUpReason>) {
        // Order matters — earlier entries win the primary slot.
        let mut applicable: Vec<CallUpReason> = Vec::new();

        if c.current_ability >= 165 && c.world_reputation >= 5000 {
            applicable.push(CallUpReason::KeyPlayer);
        }
        if ctx.window_type == CallUpWindowType::TournamentFinals && c.international_apps >= 40 {
            applicable.push(CallUpReason::TournamentExperience);
        }
        if c.average_rating >= 7.5 && c.played >= 5 {
            applicable.push(CallUpReason::CurrentForm);
        }
        if c.international_apps >= 30 {
            applicable.push(CallUpReason::InternationalExperience);
        }
        if incumbents.contains(&c.player_id) || c.international_apps >= 10 {
            applicable.push(CallUpReason::Incumbent);
        }
        if c.leadership >= 16.0 && c.age >= 28 {
            applicable.push(CallUpReason::Leadership);
        }
        let blended_apps = c.played as f32 + c.last_season_apps as f32 * 0.6;
        if blended_apps >= 18.0 && c.average_rating.max(c.last_season_rating) >= 6.8 {
            applicable.push(CallUpReason::RegularStarter);
        }
        if c.league_reputation >= 700 {
            applicable.push(CallUpReason::StrongLeague);
        }
        if ctx.window_type == CallUpWindowType::FriendlyWindow
            && c.age <= 24
            && c.international_apps <= 3
        {
            applicable.push(CallUpReason::FriendlyExperiment);
        }
        if c.age <= 22 && c.potential_ability >= 150 {
            applicable.push(CallUpReason::YouthProspect);
        }
        let best_position_level = c
            .position_levels
            .iter()
            .map(|(_, level)| *level)
            .max()
            .unwrap_or(0);
        if best_position_level >= 18 {
            applicable.push(CallUpReason::TacticalFit);
        }

        // Selection-driven reasons take precedence over the profile
        // signals if they were the actual reason this player landed in
        // the squad: PositionNeed (broad quota short), RoleCoverage
        // (role pass swap), then KeyPlayer / TournamentExperience etc.
        if position_need {
            return (CallUpReason::PositionNeed, applicable);
        }
        if role_coverage {
            return (CallUpReason::RoleCoverage, applicable);
        }

        if applicable.is_empty() {
            return (CallUpReason::RegularStarter, Vec::new());
        }

        let primary = applicable[0];
        let secondaries = applicable[1..].to_vec();
        (primary, secondaries)
    }

    /// Positional quotas given tactic shape and squad size. The
    /// returned array is `[GK, DEF, MID, FWD]`. Includes the minimum
    /// floors mentioned in the design spec.
    fn positional_quotas(tactics: &Tactics, target_squad_size: usize) -> [usize; 4] {
        let def_count = tactics.defender_count();
        let (gk, def, mid, fwd) = if target_squad_size >= TOURNAMENT_SQUAD_SIZE {
            if def_count >= 5 {
                (3, 9, 8, 6)
            } else if def_count == 3 {
                (3, 7, 9, 7)
            } else {
                (3, 8, 8, 7)
            }
        } else if def_count >= 5 {
            (3, 8, 7, 5)
        } else if def_count == 3 {
            (3, 6, 8, 6)
        } else {
            (3, 7, 7, 6)
        };

        // Floors (note: GK floor for tournament is 3; for 23 it's 2 —
        // already met by the quotas above).
        [
            gk.max(if target_squad_size >= TOURNAMENT_SQUAD_SIZE {
                3
            } else {
                2
            }),
            def.max(5),
            mid.max(5),
            fwd.max(3),
        ]
    }
}
