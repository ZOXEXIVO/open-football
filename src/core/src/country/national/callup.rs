//! Candidate collection, scoring, and balanced squad selection for
//! national-team call-ups. The pipeline shape is:
//!
//! 1. `collect_all_candidates_by_country` walks every continent and
//!    builds a `CallUpCandidate` for each eligible player, grouped by
//!    nationality, then ranks and trims each pool to the scouting cap.
//! 2. Per country, `call_up_squad` runs `select_balanced_squad` over
//!    the trimmed pool to fill positional quotas, then `derive_reasons`
//!    annotates every pick with auditable primary / secondary reasons.

use super::types::{
    BREAK_WINDOWS, CallUpCandidate, CallUpReason, MIN_REAL_PLAYERS, NationalSquadPlayer,
    SQUAD_SIZE,
};
use super::NationalTeam;
use crate::{
    Country, Player, PlayerFieldPositionGroup, PlayerPositionType, PlayerStatusType, Tactics,
    TeamType,
};
use chrono::{Datelike, NaiveDate};
use log::debug;
use std::collections::{HashMap, HashSet};

impl NationalTeam {
    /// Collect eligible national team candidates from clubs.
    /// Filters out players from very low divisions and those below minimum ability.
    /// National coaches primarily select from top divisions.
    /// Maximum candidate pool size returned to the squad selection stage.
    /// The coach scouts broadly but narrows down to a shortlist.
    pub(super) const MAX_CANDIDATE_POOL: usize = 60;

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

                    // Real league reputation lives on the League itself,
                    // not on the team. Look it up so the "strong league"
                    // signal isn't just a re-skin of the club's world rep.
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

        // Rank and trim each country's pool independently
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

        // Track record outside the current season — players with caps or
        // a real prior season shouldn't be filtered out just because the
        // new league season barely started.
        let international_caps = player.player_attributes.international_apps;
        let has_track_record = last_season_apps >= 10 || international_caps >= 5;
        let is_promising_youth = age <= 21 && potential >= 80 && total_games >= 3;

        if total_games < 5 && !is_promising_youth && !has_track_record {
            return None;
        }
        if ability < 40 && !is_promising_youth && !has_track_record {
            return None;
        }

        let condition_pct =
            (player.player_attributes.condition as f32 / 10000.0) * 100.0;

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
            average_rating: player.statistics.average_rating,
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
    /// games are summed and rating is weighted by minutes-played.
    pub(super) fn summarise_last_season(player: &Player) -> (u16, f32, u16) {
        let last_year = match player.statistics_history.items.iter().map(|i| i.season.start_year).max() {
            Some(y) => y,
            None => return (0, 0.0, 0),
        };

        let mut apps: u16 = 0;
        let mut goals: u16 = 0;
        let mut weighted_rating: f32 = 0.0;
        let mut rating_weight: u16 = 0;
        for item in &player.statistics_history.items {
            if item.season.start_year != last_year {
                continue;
            }
            let games = item.statistics.played.saturating_add(item.statistics.played_subs);
            apps = apps.saturating_add(games);
            goals = goals.saturating_add(item.statistics.goals);
            if item.statistics.average_rating > 0.0 && games > 0 {
                weighted_rating += item.statistics.average_rating * games as f32;
                rating_weight = rating_weight.saturating_add(games);
            }
        }
        let rating = if rating_weight > 0 {
            weighted_rating / rating_weight as f32
        } else {
            0.0
        };
        (apps, rating, goals)
    }

    /// Rank candidates by a scouting score (ability + reputation + match results)
    /// and trim to MAX_CANDIDATE_POOL. This ensures weaker nations still produce
    /// a full candidate pool with their best available players.
    fn rank_and_trim_candidates(mut candidates: Vec<CallUpCandidate>) -> Vec<CallUpCandidate> {
        if candidates.len() <= Self::MAX_CANDIDATE_POOL {
            return candidates;
        }

        candidates.sort_by(|a, b| {
            let score_a = Self::scouting_score(a);
            let score_b = Self::scouting_score(b);
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates.truncate(Self::MAX_CANDIDATE_POOL);
        candidates
    }

    /// Quick scouting score used to rank candidates before detailed squad selection.
    /// Combines current ability, match performance, and reputation.
    fn scouting_score(c: &CallUpCandidate) -> f32 {
        // Ability: 0-200 scale, primary factor
        let ability = c.current_ability as f32;

        // Match performance — blend current and last season so early-season
        // call-ups don't drop a regular starter for sample-size reasons.
        let effective_games = c.played as f32 + c.last_season_apps as f32 * 0.4;
        let games_factor = (effective_games).min(40.0) / 40.0;
        let primary_rating = if c.played >= 3 {
            c.average_rating
        } else if c.last_season_rating > 0.0 {
            c.last_season_rating
        } else {
            c.average_rating
        };
        let rating_factor = if primary_rating > 0.0 {
            primary_rating / 10.0
        } else {
            0.4
        };
        let performance = games_factor * 40.0 + rating_factor * 30.0;

        // Reputation: world reputation is a strong signal coaches use
        let reputation = (c.world_reputation.max(0) as f32 / 100.0).min(50.0);

        // Goals and assists boost for proven performers
        let goals_bonus = (c.international_goals as f32).min(20.0);

        // League level: playing in a stronger league is a signal
        let league_bonus = (c.league_reputation as f32 / 50.0).min(20.0);

        // Youth potential bonus: young players with high ceiling
        let youth_bonus = if c.age <= 23 {
            (c.potential_ability as f32 - c.current_ability as f32).max(0.0) * 0.3
        } else {
            0.0
        };

        ability + performance + reputation + goals_bonus + league_bonus + youth_bonus
    }

    /// Call up squad using weighted scoring — considers ability, tactical fit,
    /// form, experience, mentality, and age. Friendly breaks allow more
    /// experimentation; tournament periods favour proven performers.
    ///
    /// The Int status of called-up players is applied separately, in a
    /// continent-wide pass after every country has selected (foreign-based
    /// players play at clubs in another country, so we can't reach them
    /// from here without breaking the borrow on `&mut self`).
    pub(crate) fn call_up_squad(
        &mut self,
        candidates: Vec<CallUpCandidate>,
        date: NaiveDate,
        country_id: u32,
        country_ids: &[(u32, String)],
    ) {
        self.squad.clear();
        // Always clear the synthetic pool — without this, a country that
        // once needed fakes (e.g. early-init weak nation) would carry
        // them forever even after enough real players become available.
        self.generated_squad.clear();

        let is_tournament = Self::is_in_tournament_period(date);

        let selected = Self::select_balanced_squad(
            &candidates,
            &self.tactics,
            is_tournament,
            country_id,
        );

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

        if self.squad.len() < MIN_REAL_PLAYERS {
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

        // Friendly fixtures are intentionally NOT scheduled here.
        // Match simulation for friendlies isn't wired up — the previous
        // implementation pushed pending fixtures that would never be
        // played, leaving stale `result: None` entries littering each
        // country's schedule until the next break window swept them.
        // Until friendly simulation lives somewhere that produces a
        // real result, leaving the schedule clean is correct: real
        // matches (qualifiers, tournament games) are populated by the
        // world pipeline as they're played, with results attached. The
        // `is_tournament`, `MIN_REPUTATION_FOR_FRIENDLIES`, and
        // `country_ids` inputs are kept on `call_up_squad` because
        // squad-selection logic still needs them; only the fixture
        // emission was removed.
        let _ = (is_tournament, country_ids);

        debug!(
            "National team {} (country {}) called up {} players ({} from clubs, {} synthetic)",
            self.country_name,
            country_id,
            self.squad.len() + self.generated_squad.len(),
            self.squad.len(),
            self.generated_squad.len()
        );
    }

    /// True iff `a` and `b` fall in the same scheduled break window —
    /// used to decide which pending fixtures should be dropped before
    /// re-scheduling for a new window.
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

    /// Decide a player's primary call-up reason from their candidate profile,
    /// and a list of secondary reasons that also apply. Reasons are picked
    /// from threshold rules so the result is auditable.
    pub(super) fn derive_reasons(
        c: &CallUpCandidate,
        position_need: bool,
    ) -> (CallUpReason, Vec<CallUpReason>) {
        let mut applicable: Vec<CallUpReason> = Vec::new();

        // Order here defines priority for the primary reason when no
        // single signal dominates by magnitude.
        if c.current_ability >= 165 && c.world_reputation >= 5000 {
            applicable.push(CallUpReason::KeyPlayer);
        }
        if c.average_rating >= 7.5 && c.played >= 5 {
            applicable.push(CallUpReason::CurrentForm);
        }
        if c.international_apps >= 30 {
            applicable.push(CallUpReason::InternationalExperience);
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

        // PositionNeed wins when the slot was filled because the position
        // group quota wasn't met by stronger-scoring players.
        if position_need {
            let secondaries = applicable;
            return (CallUpReason::PositionNeed, secondaries);
        }

        if applicable.is_empty() {
            // No threshold tripped — call-up driven purely by raw scoring.
            // RegularStarter is the most generic reason that still says
            // something concrete about the selection.
            return (CallUpReason::RegularStarter, Vec::new());
        }

        let primary = applicable[0];
        let secondaries = applicable[1..].to_vec();
        (primary, secondaries)
    }

    /// Score a candidate player for national team selection.
    ///
    /// National team AI selection logic:
    /// - Ability is the primary factor (you must be good enough)
    /// - Playing at a high level (top division) is strongly favored
    /// - World reputation matters (coaches watch famous players)
    /// - Current form/match rating matters more than raw stats
    /// - International experience gives proven reliability
    /// - Age profile: tournaments prefer prime, friendlies prefer youth
    fn score_candidate(
        candidate: &CallUpCandidate,
        tactics: &Tactics,
        is_tournament: bool,
        country_id: u32,
    ) -> f32 {
        // 1. Ability (0-100) — the core factor
        let ability_score = (candidate.current_ability as f32 / 200.0) * 100.0;

        // 2. League level bonus (0-100) — playing in a top league is a major signal
        // Serie A (rep ~800+) = 80-100, Championship (~500) = 50, Serie C (~200) = 20
        // This is the key factor that prevents Serie C players from being selected
        let league_score = (candidate.league_reputation as f32 / 10.0).clamp(0.0, 100.0);

        // 3. Tactical fit — best match to any required position (0-100)
        let required_positions = tactics.positions();
        let tactical_score = required_positions
            .iter()
            .filter_map(|&pos| {
                candidate
                    .position_levels
                    .iter()
                    .find(|(p, _)| *p == pos)
                    .map(|(_, level)| *level as f32)
            })
            .fold(0.0f32, |acc, x| acc.max(x))
            / 20.0
            * 100.0;

        // 4. Form & match readiness (0-100)
        // Heavily penalize players who aren't match-fit or haven't been playing.
        // Below ~3 current-season apps the rating is noisy, so fall back to
        // the prior-season weighted average — keeps September call-ups sane.
        let condition_norm = candidate.condition_pct.clamp(0.0, 100.0);
        let readiness_norm = (candidate.match_readiness / 20.0).clamp(0.0, 1.0) * 100.0;
        let effective_rating = if candidate.played >= 3 {
            candidate.average_rating
        } else if candidate.last_season_rating > 0.0 {
            candidate.last_season_rating
        } else {
            candidate.average_rating
        };
        let rating_norm = if effective_rating > 0.0 {
            (effective_rating / 10.0).clamp(0.0, 1.0) * 100.0
        } else {
            30.0  // No rating = below average assumption
        };
        // Games: blend current with last season so a player who was a regular
        // last year but has just 1-2 games this season still scores well.
        let blended_games = candidate.played as f32 + candidate.last_season_apps as f32 * 0.4;
        let games_norm = blended_games.min(20.0) / 20.0 * 100.0;
        let form_score =
            condition_norm * 0.25 + readiness_norm * 0.25 + rating_norm * 0.30 + games_norm * 0.20;

        // 5. Reputation & international experience (0-100)
        // World reputation is on 0-10000 scale — top players are 5000+
        // Club rep is a small nudge: at parity, a player at a top club is
        // a marginally safer call-up than one at a relegation candidate.
        let rep_norm = (candidate.world_reputation.max(0) as f32 / 8000.0).clamp(0.0, 1.0) * 55.0;
        let club_rep_bonus = (candidate.club_reputation as f32 / 10000.0).clamp(0.0, 1.0) * 5.0;
        let apps_norm = (candidate.international_apps as f32).min(80.0) / 80.0 * 25.0;
        let goals_bonus = (candidate.international_goals as f32).min(40.0) / 40.0 * 15.0;
        let experience_score = (rep_norm + club_rep_bonus + apps_norm + goals_bonus).min(100.0);

        // 6. Mental & personality (0-100)
        let mental_avg = (candidate.leadership
            + candidate.composure
            + candidate.teamwork
            + candidate.determination
            + candidate.pressure_handling)
            / 5.0;
        let mental_score = (mental_avg / 20.0).clamp(0.0, 1.0) * 100.0;

        // 7. Age profile (0-100)
        let age_score = if is_tournament {
            match candidate.age {
                ..=20 => 40.0,
                21..=23 => 65.0,
                24..=29 => 90.0,
                30..=32 => 75.0,
                33..=35 => 50.0,
                _ => 30.0,
            }
        } else {
            match candidate.age {
                ..=20 => 75.0,
                21..=23 => 85.0,
                24..=29 => 70.0,
                30..=32 => 45.0,
                33..=35 => 30.0,
                _ => 15.0,
            }
        };

        // 8. Season impact — goals, assists, PoM awards, clean sheets (0-100)
        // A striker scoring 15+ goals or a midfielder with 10+ assists stands out.
        // Position-aware: goals matter more for forwards, clean sheets for defenders/GKs.
        // Blend in the previous season so a striker with 15 goals last year
        // and only 1 so far this year is still recognised as a goal-scorer.
        let total_games_f = (candidate.played as f32).max(1.0);
        let blended_goals = candidate.goals as f32 + candidate.last_season_goals as f32 * 0.4;
        let blended_games = (candidate.played as f32 + candidate.last_season_apps as f32 * 0.4).max(1.0);
        let goals_per_game = if candidate.played >= 5 {
            candidate.goals as f32 / total_games_f
        } else {
            blended_goals / blended_games
        };
        let assists_per_game = candidate.assists as f32 / total_games_f;
        let pom_norm = (candidate.player_of_the_match as f32).min(8.0) / 8.0 * 30.0;
        let discipline_penalty = candidate.red_cards as f32 * 10.0
            + candidate.yellow_cards as f32 * 1.5;

        let impact_score = match candidate.position_group {
            PlayerFieldPositionGroup::Forward => {
                let goal_score = (goals_per_game * 80.0).min(40.0);
                let assist_score = (assists_per_game * 60.0).min(15.0);
                (goal_score + assist_score + pom_norm - discipline_penalty).clamp(0.0, 100.0)
            }
            PlayerFieldPositionGroup::Midfielder => {
                let goal_score = (goals_per_game * 60.0).min(20.0);
                let assist_score = (assists_per_game * 80.0).min(30.0);
                (goal_score + assist_score + pom_norm - discipline_penalty).clamp(0.0, 100.0)
            }
            PlayerFieldPositionGroup::Defender => {
                let cs_per_game = candidate.clean_sheets as f32 / total_games_f;
                let cs_score = (cs_per_game * 80.0).min(35.0);
                let goal_score = (goals_per_game * 50.0).min(10.0);
                (cs_score + goal_score + pom_norm - discipline_penalty).clamp(0.0, 100.0)
            }
            PlayerFieldPositionGroup::Goalkeeper => {
                let cs_per_game = candidate.clean_sheets as f32 / total_games_f;
                let cs_score = (cs_per_game * 100.0).min(45.0);
                (cs_score + pom_norm - discipline_penalty).clamp(0.0, 100.0)
            }
        };

        // 9. Potential (only meaningful in friendlies)
        let potential_score = (candidate.potential_ability as f32 / 200.0) * 100.0;

        // 10. Coach bias — deterministic per country
        let coach_bias = match country_id % 4 {
            0 => (candidate.international_apps as f32).min(80.0) / 80.0 * 5.0,
            1 => if candidate.age <= 24 { 5.0 } else { 0.0 },
            2 => (candidate.world_reputation.max(0) as f32 / 5000.0).clamp(0.0, 1.0) * 5.0,
            _ => (candidate.leadership / 20.0).clamp(0.0, 1.0) * 5.0,
        };

        // Apply context-dependent weights
        // Tournament: proven quality + match fitness + season impact + experience
        // Friendly: experiment with youth + potential, but still need fitness
        let weighted = if is_tournament {
            ability_score * 0.22
                + league_score * 0.08
                + tactical_score * 0.10
                + form_score * 0.18
                + impact_score * 0.15
                + experience_score * 0.12
                + mental_score * 0.08
                + age_score * 0.07
        } else {
            let youth_bonus = if candidate.age <= 23 && candidate.international_apps < 10 {
                5.0
            } else {
                0.0
            };
            ability_score * 0.16
                + league_score * 0.06
                + tactical_score * 0.08
                + form_score * 0.18
                + impact_score * 0.12
                + experience_score * 0.05
                + mental_score * 0.06
                + age_score * 0.08
                + potential_score * 0.11
                + youth_bonus
        };

        weighted + coach_bias
    }

    /// Select a balanced squad respecting positional quotas.
    ///
    /// Returns `(candidate_index, primary_reason, secondary_reasons)`.
    /// A pick is flagged `PositionNeed` whenever the squad was below the
    /// quota for that position group at the time the player was added —
    /// otherwise reasons come from the candidate's own profile.
    pub(super) fn select_balanced_squad(
        candidates: &[CallUpCandidate],
        tactics: &Tactics,
        is_tournament: bool,
        country_id: u32,
    ) -> Vec<(usize, CallUpReason, Vec<CallUpReason>)> {
        if candidates.is_empty() {
            return Vec::new();
        }

        let scored: Vec<(usize, f32)> = candidates
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                (
                    idx,
                    Self::score_candidate(c, tactics, is_tournament, country_id),
                )
            })
            .collect();

        let [gk_quota, def_quota, mid_quota, fwd_quota] = Self::positional_quotas(tactics);

        let desc =
            |a: &(usize, f32), b: &(usize, f32)| {
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

        let mut selected: Vec<(usize, CallUpReason, Vec<CallUpReason>)> =
            Vec::with_capacity(SQUAD_SIZE);
        let mut taken: HashSet<usize> = HashSet::new();

        // PositionNeed reflects the realistic situation where a manager
        // *had* to take a player to satisfy the formation, even though
        // they wouldn't have made the squad on raw quality. Strong picks
        // (top of their group) keep their natural reason.
        let take_group = |group: &[(usize, f32)],
                          quota: usize,
                          selected: &mut Vec<(usize, CallUpReason, Vec<CallUpReason>)>,
                          taken: &mut HashSet<usize>| {
            for (rank, &(idx, _)) in group.iter().take(quota).enumerate() {
                let c = &candidates[idx];
                // A player is a position-need pick when they were the
                // only / weakest option to fill the quota — bottom half
                // of the chosen group AND below a quality threshold.
                let weak_pick = rank >= quota / 2 && c.current_ability < 130;
                let (primary, secondaries) = Self::derive_reasons(c, weak_pick);
                selected.push((idx, primary, secondaries));
                taken.insert(idx);
            }
        };

        take_group(&gk, gk_quota, &mut selected, &mut taken);
        take_group(&def, def_quota, &mut selected, &mut taken);
        take_group(&mid, mid_quota, &mut selected, &mut taken);
        take_group(&fwd, fwd_quota, &mut selected, &mut taken);

        if selected.len() < SQUAD_SIZE {
            let mut leftover: Vec<(usize, f32)> = scored
                .iter()
                .filter(|(i, _)| !taken.contains(i))
                .copied()
                .collect();
            leftover.sort_by(&desc);
            for (idx, _) in leftover {
                if selected.len() >= SQUAD_SIZE {
                    break;
                }
                let c = &candidates[idx];
                let (primary, secondaries) = Self::derive_reasons(c, false);
                selected.push((idx, primary, secondaries));
                taken.insert(idx);
            }
        }

        selected.truncate(SQUAD_SIZE);
        selected
    }

    /// Positional quotas for a 23-man squad based on the tactic's shape.
    /// Returns [GK, DEF, MID, FWD].
    fn positional_quotas(tactics: &Tactics) -> [usize; 4] {
        let def_count = tactics.defender_count();
        if def_count >= 5 {
            [3, 8, 7, 5]
        } else if def_count == 3 {
            [3, 6, 8, 6]
        } else {
            [3, 7, 7, 6]
        }
    }
}
