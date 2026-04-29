use crate::club::player::load::PlayerLoad;
use crate::club::player::rapport::PlayerRapport;
use crate::country::PeopleNameGeneratorData;
use crate::r#match::{MatchPlayer, MatchResultRaw, MatchSquad};
use crate::shared::FullName;
use crate::utils::IntegerUtils;
use crate::{
    Club, HappinessEventType, MatchTacticType, Mental, PersonAttributes, PersonBehaviour,
    PersonBehaviourState, Physical, Player, PlayerAttributes, PlayerDecisionHistory,
    PlayerFieldPositionGroup, PlayerHappiness, PlayerMailbox, PlayerPosition, PlayerPositionType,
    PlayerPositions, PlayerPreferredFoot, PlayerSkills, PlayerStatistics, PlayerStatisticsHistory,
    PlayerStatus, PlayerStatusType, PlayerTraining, PlayerTrainingHistory, Relations, Tactics,
    TeamType, Technical,
};
use crate::Country;
use chrono::{Datelike, NaiveDate};
use log::debug;
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub struct NationalTeam {
    pub country_id: u32,
    pub country_name: String,
    pub staff: Vec<NationalTeamStaffMember>,
    pub squad: Vec<NationalSquadPlayer>,
    pub generated_squad: Vec<Player>,
    pub tactics: Tactics,
    pub reputation: u16,
    pub elo_rating: u16,
    pub schedule: Vec<NationalTeamFixture>,
}

#[derive(Clone)]
pub struct NationalTeamStaffMember {
    pub first_name: String,
    pub last_name: String,
    pub role: NationalTeamStaffRole,
    pub country_id: u32,
    pub birth_year: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NationalTeamStaffRole {
    Manager,
    AssistantManager,
    Coach,
    GoalkeeperCoach,
    FitnessCoach,
}

impl NationalTeamStaffRole {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            NationalTeamStaffRole::Manager => "staff_manager",
            NationalTeamStaffRole::AssistantManager => "staff_assistant_manager",
            NationalTeamStaffRole::Coach => "staff_coach",
            NationalTeamStaffRole::GoalkeeperCoach => "staff_goalkeeper_coach",
            NationalTeamStaffRole::FitnessCoach => "staff_fitness_coach",
        }
    }
}

#[derive(Clone)]
pub struct NationalSquadPlayer {
    pub player_id: u32,
    pub club_id: u32,
    pub team_id: u32,
    pub primary_reason: CallUpReason,
    pub secondary_reasons: Vec<CallUpReason>,
}

/// Unified view over a national-team squad pick — covers both real
/// players (looked up from a club roster) and synthetic players
/// (generated to fill a thin pool, owned by `generated_squad`). UI
/// code should iterate `NationalTeam::squad_picks()` rather than
/// reaching into `squad` and `generated_squad` separately, so synthetic
/// depth players are visible everywhere a real player would be.
pub enum SquadPick<'a> {
    Real(&'a NationalSquadPlayer),
    Synthetic(&'a Player),
}

/// Why a player was selected for the national squad.
/// Surfaces in the squad UI and in debug logs so call-ups are auditable
/// instead of looking arbitrary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallUpReason {
    /// High ability and world reputation — the manager always picks them.
    KeyPlayer,
    /// Strong recent match ratings — riding a hot form streak.
    CurrentForm,
    /// Plays week-in week-out for a real club; reliable minutes.
    RegularStarter,
    /// Competes in a top-tier league — playing level signal.
    StrongLeague,
    /// Best tactical fit for a position the manager's tactic demands.
    TacticalFit,
    /// Selected primarily to fill a positional shortage.
    PositionNeed,
    /// Veteran with many caps — proven on the international stage.
    InternationalExperience,
    /// Captain material — leadership/composure carry weight in this squad.
    Leadership,
    /// Young player with high potential, blooded for the future.
    YouthProspect,
    /// Synthetic player generated to fill a thin pool (weak nation).
    SyntheticDepth,
}

impl CallUpReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CallUpReason::KeyPlayer => "callup_reason_key_player",
            CallUpReason::CurrentForm => "callup_reason_current_form",
            CallUpReason::RegularStarter => "callup_reason_regular_starter",
            CallUpReason::StrongLeague => "callup_reason_strong_league",
            CallUpReason::TacticalFit => "callup_reason_tactical_fit",
            CallUpReason::PositionNeed => "callup_reason_position_need",
            CallUpReason::InternationalExperience => "callup_reason_international_experience",
            CallUpReason::Leadership => "callup_reason_leadership",
            CallUpReason::YouthProspect => "callup_reason_youth_prospect",
            CallUpReason::SyntheticDepth => "callup_reason_synthetic_depth",
        }
    }
}

#[derive(Clone)]
pub struct NationalTeamFixture {
    pub date: NaiveDate,
    pub opponent_country_id: u32,
    pub opponent_country_name: String,
    pub is_home: bool,
    pub competition_name: String,
    pub match_id: String,
    pub result: Option<NationalTeamMatchResult>,
}

#[derive(Clone)]
pub struct NationalTeamMatchResult {
    pub home_score: u8,
    pub away_score: u8,
    pub date: NaiveDate,
    pub opponent_country_id: u32,
}

/// Break windows matching League::is_international_break:
/// Sep 4-12, Oct 9-17, Nov 13-21, Mar 20-28
const BREAK_WINDOWS: [(u32, u32, u32); 4] = [
    (9, 4, 12),
    (10, 9, 17),
    (11, 13, 21),
    (3, 20, 28),
];

/// Tournament window: June-July for World Cup / Euro finals
const TOURNAMENT_WINDOW: (u32, u32, u32, u32) = (6, 10, 7, 15);

const DEFAULT_STAFF_ROLES: [NationalTeamStaffRole; 5] = [
    NationalTeamStaffRole::Manager,
    NationalTeamStaffRole::AssistantManager,
    NationalTeamStaffRole::Coach,
    NationalTeamStaffRole::GoalkeeperCoach,
    NationalTeamStaffRole::FitnessCoach,
];

/// Minimum number of real club players before generating synthetic ones
const MIN_REAL_PLAYERS: usize = 16;

/// Default squad call-up size
const SQUAD_SIZE: usize = 23;

/// Positions template for generating a balanced synthetic squad
const SYNTHETIC_POSITIONS: [PlayerPositionType; 23] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::MidfielderLeft,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::MidfielderRight,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::AttackingMidfielderCenter,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardCenter,
    PlayerPositionType::ForwardRight,
    PlayerPositionType::Striker,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::ForwardCenter,
    PlayerPositionType::Striker,
];

/// Data collected from a candidate player for call-up scoring.
/// Captures enough information that the selection result is explainable
/// — every field here can be cited as a reason ("regular starter",
/// "strong league", "veteran caps", …) without needing to re-read the
/// underlying Player struct.
pub(crate) struct CallUpCandidate {
    player_id: u32,
    club_id: u32,
    team_id: u32,
    current_ability: u8,
    potential_ability: u8,
    age: i32,
    condition_pct: f32,
    match_readiness: f32,
    average_rating: f32,
    played: u16,
    international_apps: u16,
    international_goals: u16,
    leadership: f32,
    composure: f32,
    teamwork: f32,
    determination: f32,
    pressure_handling: f32,
    world_reputation: i16,
    /// Club reputation where the player plays — was previously misnamed
    /// "league_reputation" while actually holding team.reputation.world.
    club_reputation: u16,
    /// True league reputation (0-1000) looked up via team.league_id —
    /// represents the strength of the division, not the individual club.
    league_reputation: u16,
    position_levels: Vec<(PlayerPositionType, u8)>,
    position_group: PlayerFieldPositionGroup,
    /// Current-season stats
    goals: u16,
    assists: u16,
    player_of_the_match: u8,
    clean_sheets: u16,
    yellow_cards: u8,
    red_cards: u8,
    /// Total apps (league + cup) in the most recent prior season —
    /// keeps early-season call-ups grounded in last year's body of work,
    /// not a 0–4 game sample from the new season.
    last_season_apps: u16,
    /// Weighted average rating across all entries from the most recent
    /// prior season. 0.0 means no prior history.
    last_season_rating: f32,
    /// Goals scored in the most recent prior season.
    last_season_goals: u16,
}

impl NationalTeam {
    pub fn new(country_id: u32, names: &PeopleNameGeneratorData) -> Self {
        let staff = Self::generate_staff(country_id, names);

        NationalTeam {
            country_id,
            country_name: String::new(),
            staff,
            squad: Vec::new(),
            generated_squad: Vec::new(),
            tactics: Tactics::new(MatchTacticType::T442),
            reputation: 0,
            elo_rating: 1500,
            schedule: Vec::new(),
        }
    }

    fn generate_staff(
        country_id: u32,
        names: &PeopleNameGeneratorData,
    ) -> Vec<NationalTeamStaffMember> {
        DEFAULT_STAFF_ROLES
            .iter()
            .map(|&role| {
                let first_name = Self::random_name(&names.first_names);
                let last_name = Self::random_name(&names.last_names);
                let birth_year = IntegerUtils::random(1960, 1990);

                NationalTeamStaffMember {
                    first_name,
                    last_name,
                    role,
                    country_id,
                    birth_year,
                }
            })
            .collect()
    }

    fn random_name(names: &[String]) -> String {
        if names.is_empty() {
            return "Unknown".to_string();
        }
        let idx = IntegerUtils::random(0, names.len() as i32) as usize;
        names
            .get(idx)
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Unified iterator over squad picks: real players first (squad
    /// list), then synthetic depth players (from generated_squad).
    /// UI code should call this so synthetic players appear in the
    /// same surfaces as real call-ups, with reason `SyntheticDepth`.
    pub fn squad_picks(&self) -> Vec<SquadPick<'_>> {
        let mut picks: Vec<SquadPick<'_>> = self.squad.iter().map(SquadPick::Real).collect();
        picks.extend(self.generated_squad.iter().map(SquadPick::Synthetic));
        picks
    }

    /// Returns the fixture index of a pending friendly for today, if any.
    pub fn pending_friendly(&self, date: NaiveDate) -> Option<usize> {
        self.schedule
            .iter()
            .position(|f| f.date == date && f.result.is_none())
    }

    /// Apply the result of a friendly match that was played externally (in parallel).
    pub fn apply_friendly_result(
        &mut self,
        clubs: &mut [Club],
        fixture_idx: usize,
        match_result: &MatchResultRaw,
        date: NaiveDate,
    ) {
        let fixture = &self.schedule[fixture_idx];
        let opponent_id = fixture.opponent_country_id;
        let opponent_name = fixture.opponent_country_name.clone();
        let is_home = fixture.is_home;

        let score = match_result
            .score
            .as_ref()
            .expect("match should have score");
        let home_score = score.home_team.get();
        let away_score = score.away_team.get();

        let result = NationalTeamMatchResult {
            home_score,
            away_score,
            date,
            opponent_country_id: opponent_id,
        };

        // Update player stats
        let squad_player_ids: Vec<u32> = self.squad.iter().map(|s| s.player_id).collect();

        for club in clubs.iter_mut() {
            for team in club.teams.iter_mut() {
                for player in team.players.iter_mut() {
                    if squad_player_ids.contains(&player.id) {
                        player.player_attributes.international_apps += 1;

                        if let Some(stats) = match_result.player_stats.get(&player.id) {
                            player.player_attributes.international_goals +=
                                stats.goals as u16;
                        }
                    }
                }
            }
        }

        // Update Elo rating
        let (our_score, opp_score) = if is_home {
            (home_score, away_score)
        } else {
            (away_score, home_score)
        };
        self.update_elo(our_score, opp_score, 1500);

        self.schedule[fixture_idx].result = Some(result);

        debug!(
            "International friendly: {} vs {} - {}:{}",
            self.country_name, opponent_name, home_score, away_score
        );
    }

    /// Collect eligible national team candidates from clubs.
    /// Filters out players from very low divisions and those below minimum ability.
    /// National coaches primarily select from top divisions.
    /// Maximum candidate pool size returned to the squad selection stage.
    /// The coach scouts broadly but narrows down to a shortlist.
    const MAX_CANDIDATE_POOL: usize = 60;

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
    fn build_candidate(
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
    fn summarise_last_season(player: &Player) -> (u16, f32, u16) {
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
    fn derive_reasons(c: &CallUpCandidate, position_need: bool) -> (CallUpReason, Vec<CallUpReason>) {
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
    fn select_balanced_squad(
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

    /// Generate synthetic players for countries without enough club players.
    /// Ability is derived from country reputation.
    fn generate_synthetic_squad(&mut self, date: NaiveDate) {
        self.generated_squad.clear();

        let slots_needed = SQUAD_SIZE.saturating_sub(self.squad.len());
        if slots_needed == 0 {
            return;
        }

        // Derive ability from reputation (0-1000 reputation -> ~40-180 ability)
        let base_ability = ((self.reputation as f32 / 1000.0) * 140.0 + 40.0) as u8;

        let positions_to_fill = &SYNTHETIC_POSITIONS[..slots_needed.min(SYNTHETIC_POSITIONS.len())];

        for (idx, &position) in positions_to_fill.iter().enumerate() {
            // Vary ability slightly per player
            let ability_variation = IntegerUtils::random(-10, 10) as i16;
            let ability = (base_ability as i16 + ability_variation).clamp(30, 200) as u8;

            let player = Self::generate_synthetic_player(
                self.country_id,
                date,
                position,
                ability,
                idx as u32,
            );
            self.generated_squad.push(player);
        }
    }

    /// Generate a single synthetic player with the given attributes
    fn generate_synthetic_player(
        country_id: u32,
        now: NaiveDate,
        position: PlayerPositionType,
        ability: u8,
        seed_offset: u32,
    ) -> Player {
        let age = IntegerUtils::random(22, 34);
        let year = now.year() - age;
        let month = ((country_id + seed_offset) % 12 + 1) as u32;
        let day = ((country_id + seed_offset * 7) % 28 + 1) as u32;

        // Use deterministic ID based on country + position + offset
        let id = 900_000 + country_id * 100 + seed_offset;

        // Scale skills based on ability (ability 0-200 -> skill factor 0.25-1.0)
        let skill_factor = (ability as f32 / 200.0).clamp(0.25, 1.0);
        let base_skill = skill_factor * 20.0;

        let position_level = (skill_factor * 20.0) as u8;

        Player {
            id,
            full_name: FullName::with_full(
                format!("NT{}", seed_offset),
                format!("Player{}", country_id),
                String::new(),
            ),
            birth_date: NaiveDate::from_ymd_opt(year, month, day)
                .unwrap_or(NaiveDate::from_ymd_opt(year, 1, 1).unwrap()),
            country_id,
            behaviour: PersonBehaviour {
                state: PersonBehaviourState::Normal,
            },
            attributes: PersonAttributes {
                adaptability: base_skill,
                ambition: base_skill,
                controversy: 5.0,
                loyalty: base_skill,
                pressure: base_skill,
                professionalism: base_skill,
                sportsmanship: base_skill,
                temperament: base_skill,
                consistency: base_skill,
                important_matches: base_skill,
                dirtiness: 5.0,
            },
            happiness: PlayerHappiness::new(),
            statuses: PlayerStatus { statuses: vec![] },
            skills: PlayerSkills {
                technical: Technical {
                    corners: base_skill,
                    crossing: base_skill,
                    dribbling: base_skill,
                    finishing: base_skill,
                    first_touch: base_skill,
                    free_kicks: base_skill,
                    heading: base_skill,
                    long_shots: base_skill,
                    long_throws: base_skill,
                    marking: base_skill,
                    passing: base_skill,
                    penalty_taking: base_skill,
                    tackling: base_skill,
                    technique: base_skill,
                },
                mental: Mental {
                    aggression: base_skill,
                    anticipation: base_skill,
                    bravery: base_skill,
                    composure: base_skill,
                    concentration: base_skill,
                    decisions: base_skill,
                    determination: base_skill,
                    flair: base_skill,
                    leadership: base_skill,
                    off_the_ball: base_skill,
                    positioning: base_skill,
                    teamwork: base_skill,
                    vision: base_skill,
                    work_rate: base_skill,
                },
                physical: Physical {
                    acceleration: base_skill,
                    agility: base_skill,
                    balance: base_skill,
                    jumping: base_skill,
                    natural_fitness: base_skill,
                    pace: base_skill,
                    stamina: base_skill,
                    strength: base_skill,
                    match_readiness: 15.0,
                },
                goalkeeping: Default::default(),
            },
            contract: None,
            contract_loan: None,
            positions: PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: position_level,
                }],
            },
            preferred_foot: PlayerPreferredFoot::Right,
            player_attributes: PlayerAttributes {
                is_banned: false,
                is_injured: false,
                condition: 10000,
                fitness: 0,
                jadedness: 0,
                weight: 75,
                height: 180,
                value: 0,
                current_reputation: (ability as i16) * 5,
                home_reputation: 1000,
                world_reputation: (ability as i16) * 3,
                current_ability: ability,
                potential_ability: ability,
                international_apps: 0,
                international_goals: 0,
                under_21_international_apps: 0,
                under_21_international_goals: 0,
                injury_days_remaining: 0,
                injury_type: None,
                injury_proneness: 10,
                recovery_days_remaining: 0,
                last_injury_body_part: 0,
                injury_count: 0,
                days_since_last_match: 0,
            },
            mailbox: PlayerMailbox::new(),
            training: PlayerTraining::new(),
            training_history: PlayerTrainingHistory::new(),
            relations: Relations::new(),
            statistics: PlayerStatistics::default(),
            friendly_statistics: PlayerStatistics::default(),
            cup_statistics: PlayerStatistics::default(),
            statistics_history: PlayerStatisticsHistory::new(),
            decision_history: PlayerDecisionHistory::new(),
            languages: Vec::new(),
            last_transfer_date: None,
            plan: None,
            favorite_clubs: Vec::new(),
            sold_from: None,
            sell_on_obligations: Vec::new(),
            traits: Vec::new(),
            is_force_match_selection: false,
            rapport: PlayerRapport::new(),
            promises: Vec::new(),
            interactions: crate::club::player::interaction::ManagerInteractionLog::new(),
            pending_signing: None,
            generated: true,
            retired: false,
            load: PlayerLoad::new(),
            pending_contract_ask: None,
            last_intl_caps_paid: 0,
        }
    }

    /// Build a MatchSquad from the called-up squad + generated players
    pub fn build_match_squad(&self, clubs: &[Club]) -> MatchSquad {
        let club_refs: Vec<&Club> = clubs.iter().collect();
        self.build_match_squad_from_refs(&club_refs)
    }

    /// Build a MatchSquad searching across all provided clubs (including foreign).
    /// This variant accepts refs so the caller can collect clubs from multiple countries.
    pub fn build_match_squad_from_refs(&self, clubs: &[&Club]) -> MatchSquad {
        let team_id = self.country_id;
        let team_name = self.country_name.clone();

        // Collect real players from clubs (may span multiple countries)
        let mut all_players: Vec<&Player> = Vec::new();

        for squad_player in &self.squad {
            for club in clubs.iter() {
                for team in club.teams.iter() {
                    if let Some(player) = team.players.find(squad_player.player_id) {
                        all_players.push(player);
                    }
                }
            }
        }

        // Add generated synthetic players
        for player in &self.generated_squad {
            all_players.push(player);
        }

        // Select starting 11 and substitutes
        let tactics = &self.tactics;
        let required_positions = tactics.positions();

        let mut main_squad: Vec<MatchPlayer> = Vec::with_capacity(11);
        let mut used_ids: Vec<u32> = Vec::new();

        // Pick goalkeeper. If the squad has NO natural keeper (some
        // smaller national pools end up this way — sim generated only
        // outfielders, or injuries removed the real GKs), fall back to
        // the least-valuable outfielder so we never field an empty
        // goal. Without this fallback the squad played 10-a-side with
        // an open net, which is exactly where the "17-0 / 29-0" CA/EC
        // international scorelines were coming from.
        let natural_gk = all_players
            .iter()
            .filter(|p| {
                p.positions
                    .positions
                    .iter()
                    .any(|pos| pos.position == PlayerPositionType::Goalkeeper)
            })
            .max_by_key(|p| p.player_attributes.current_ability);

        let gk_choice = natural_gk.copied().or_else(|| {
            // No natural keeper — draft the lowest-ability outfielder.
            // Weakest-outfielder-as-GK is realistic: managers sacrifice
            // a fringe player rather than a first-teamer.
            all_players
                .iter()
                .min_by_key(|p| p.player_attributes.current_ability)
                .copied()
        });

        if let Some(gk) = gk_choice {
            main_squad.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        // Fill outfield positions
        for &pos in required_positions.iter() {
            if pos == PlayerPositionType::Goalkeeper {
                continue;
            }
            if main_squad.len() >= 11 {
                break;
            }

            let best = all_players
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| {
                    !p.positions
                        .positions
                        .iter()
                        .any(|pp| pp.position == PlayerPositionType::Goalkeeper)
                })
                .max_by_key(|p| {
                    let pos_fit = p.positions.get_level(pos) as u16;
                    let ability = p.player_attributes.current_ability as u16;
                    pos_fit * 3 + ability
                });

            if let Some(player) = best {
                main_squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // Fill any remaining starting slots
        while main_squad.len() < 11 {
            let best = all_players
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by_key(|p| p.player_attributes.current_ability);

            match best {
                Some(player) => {
                    let pos = player.position();
                    main_squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        // Select substitutes (up to 7)
        let mut substitutes: Vec<MatchPlayer> = Vec::with_capacity(7);
        let remaining: Vec<&&Player> = all_players
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .collect();

        // Backup GK first
        if let Some(gk) = remaining
            .iter()
            .filter(|p| {
                p.positions
                    .positions
                    .iter()
                    .any(|pos| pos.position == PlayerPositionType::Goalkeeper)
            })
            .max_by_key(|p| p.player_attributes.current_ability)
        {
            substitutes.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        // Fill rest of bench
        let mut bench_remaining: Vec<&&Player> = remaining
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .copied()
            .collect();
        bench_remaining.sort_by(|a, b| {
            b.player_attributes
                .current_ability
                .cmp(&a.player_attributes.current_ability)
        });

        for player in bench_remaining.iter().take(6) {
            let pos = player.position();
            substitutes.push(MatchPlayer::from_player(team_id, player, pos, false));
        }

        MatchSquad {
            team_id,
            team_name,
            tactics: self.tactics.clone(),
            main_squad,
            substitutes,
            captain_id: None,
            vice_captain_id: None,
            penalty_taker_id: None,
            free_kick_taker_id: None,
        }
    }


    /// Build a synthetic opponent squad for friendly matches
    pub fn build_synthetic_opponent_squad(opponent_country_id: u32, opponent_name: &str) -> MatchSquad {
        let team_id = opponent_country_id;

        // Generate 18 synthetic players with moderate ability
        let now = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let positions = &SYNTHETIC_POSITIONS[..18];

        let mut players: Vec<Player> = Vec::new();
        for (idx, &pos) in positions.iter().enumerate() {
            let ability = IntegerUtils::random(80, 140) as u8;
            let player = Self::generate_synthetic_player(
                opponent_country_id,
                now,
                pos,
                ability,
                idx as u32 + 50, // offset to avoid ID collision
            );
            players.push(player);
        }

        let tactics = Tactics::new(MatchTacticType::T442);
        let required_positions = tactics.positions();

        let mut main_squad: Vec<MatchPlayer> = Vec::with_capacity(11);
        let mut used_ids: Vec<u32> = Vec::new();

        // GK — fall back to any player if no natural keeper so we never
        // field an empty goal (see `build_match_squad_from_refs` for the
        // same bug when natural pool was exhausted).
        let gk_choice = players
            .iter()
            .find(|p| {
                p.positions
                    .positions
                    .iter()
                    .any(|pos| pos.position == PlayerPositionType::Goalkeeper)
            })
            .or_else(|| players.first());
        if let Some(gk) = gk_choice {
            main_squad.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        // Outfield
        for &pos in required_positions.iter() {
            if pos == PlayerPositionType::Goalkeeper || main_squad.len() >= 11 {
                continue;
            }
            if let Some(player) = players
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by_key(|p| p.positions.get_level(pos) as u16 + p.player_attributes.current_ability as u16)
            {
                main_squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // Subs
        let substitutes: Vec<MatchPlayer> = players
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .take(7)
            .map(|p| {
                let pos = p.position();
                MatchPlayer::from_player(team_id, p, pos, false)
            })
            .collect();

        MatchSquad {
            team_id,
            team_name: opponent_name.to_string(),
            tactics,
            main_squad,
            substitutes,
            captain_id: None,
            vice_captain_id: None,
            penalty_taker_id: None,
            free_kick_taker_id: None,
        }
    }

    /// Apply / release `PlayerStatusType::Int` across every club in
    /// every continent, based on each country's current squad. Squads
    /// are picked from a world-wide candidate pool, so a foreign-based
    /// player (a Brazilian at a Spanish club, …) only gets the right
    /// flag when this pass scans every continent's clubs — not just
    /// the player's nationality continent.
    ///
    /// Fires happiness events on transitions: a fresh call-up is a big
    /// moment for a young pro; being dropped after a run of caps hurts
    /// pride. Keeping events here (not in the per-country call-up)
    /// means each player only sees one event per cycle even if their
    /// nation has already been processed before their club's continent.
    pub(crate) fn apply_callup_statuses_across_world(
        continents: &mut [crate::continent::Continent],
        date: NaiveDate,
    ) {
        let mut called_up: HashSet<u32> = HashSet::new();
        for continent in continents.iter() {
            for country in continent.countries.iter() {
                for sp in &country.national_team.squad {
                    called_up.insert(sp.player_id);
                }
            }
        }

        for continent in continents.iter_mut() {
            for country in continent.countries.iter_mut() {
                for club in country.clubs.iter_mut() {
                    for team in club.teams.iter_mut() {
                        for player in team.players.iter_mut() {
                            let is_called_up = called_up.contains(&player.id);
                            let was_in = player.statuses.get().contains(&PlayerStatusType::Int);

                            if is_called_up {
                                player.statuses.add(date, PlayerStatusType::Int);
                                if !was_in {
                                    let caps = player.player_attributes.international_apps;
                                    let mag = if caps == 0 {
                                        10.0
                                    } else if caps < 10 {
                                        6.0
                                    } else {
                                        3.0
                                    };
                                    player
                                        .happiness
                                        .add_event(HappinessEventType::NationalTeamCallup, mag);
                                }
                            } else if was_in {
                                player.statuses.remove(PlayerStatusType::Int);
                                let caps = player.player_attributes.international_apps;
                                let mag = if caps >= 20 {
                                    -6.0
                                } else if caps >= 5 {
                                    -4.0
                                } else {
                                    -2.0
                                };
                                player
                                    .happiness
                                    .add_event(HappinessEventType::NationalTeamDropped, mag);
                            }
                        }
                    }
                }
            }
        }
    }

    /// World-wide variant of `release_callup_statuses_across_continent`.
    pub(crate) fn release_callup_statuses_across_world(
        continents: &mut [crate::continent::Continent],
    ) {
        let mut released_ids: HashSet<u32> = HashSet::new();
        for continent in continents.iter() {
            for country in continent.countries.iter() {
                for sp in &country.national_team.squad {
                    released_ids.insert(sp.player_id);
                }
            }
        }

        for continent in continents.iter_mut() {
            for country in continent.countries.iter_mut() {
                for club in country.clubs.iter_mut() {
                    for team in club.teams.iter_mut() {
                        for player in team.players.iter_mut() {
                            if released_ids.contains(&player.id) {
                                player.statuses.remove(PlayerStatusType::Int);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Update Elo rating after a match
    pub fn update_elo(&mut self, our_score: u8, opponent_score: u8, opponent_elo: u16) {
        let k: f32 = 20.0;
        let expected = 1.0 / (1.0 + 10.0_f32.powf((opponent_elo as f32 - self.elo_rating as f32) / 400.0));

        let actual = if our_score > opponent_score {
            1.0
        } else if our_score == opponent_score {
            0.5
        } else {
            0.0
        };

        let change = (k * (actual - expected)) as i16;
        self.elo_rating = (self.elo_rating as i16 + change).clamp(500, 2500) as u16;
    }

    pub fn is_break_start(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS
            .iter()
            .any(|(m, start, _)| month == *m && day == *start)
    }

    pub fn is_break_end(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS
            .iter()
            .any(|(m, _, end)| month == *m && day == *end)
    }

    pub fn is_in_break(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS
            .iter()
            .any(|(m, start, end)| month == *m && day >= *start && day <= *end)
    }

    pub fn is_tournament_start(date: NaiveDate) -> bool {
        date.month() == TOURNAMENT_WINDOW.0 && date.day() == TOURNAMENT_WINDOW.1
    }

    pub fn is_tournament_end(date: NaiveDate) -> bool {
        date.month() == TOURNAMENT_WINDOW.2 && date.day() == TOURNAMENT_WINDOW.3
    }

    fn is_in_tournament_period(date: NaiveDate) -> bool {
        let month = date.month();
        (month == TOURNAMENT_WINDOW.0 && date.day() >= TOURNAMENT_WINDOW.1)
            || (month == TOURNAMENT_WINDOW.2 && date.day() <= TOURNAMENT_WINDOW.3)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::statistics::PlayerStatisticsHistoryItem;
    use crate::league::Season;

    fn make_candidate(
        id: u32,
        ability: u8,
        position_group: PlayerFieldPositionGroup,
    ) -> CallUpCandidate {
        let position = match position_group {
            PlayerFieldPositionGroup::Goalkeeper => PlayerPositionType::Goalkeeper,
            PlayerFieldPositionGroup::Defender => PlayerPositionType::DefenderCenter,
            PlayerFieldPositionGroup::Midfielder => PlayerPositionType::MidfielderCenter,
            PlayerFieldPositionGroup::Forward => PlayerPositionType::Striker,
        };
        CallUpCandidate {
            player_id: id,
            club_id: 1,
            team_id: 1,
            current_ability: ability,
            potential_ability: ability + 10,
            age: 27,
            condition_pct: 95.0,
            match_readiness: 18.0,
            average_rating: 7.0,
            played: 10,
            international_apps: 5,
            international_goals: 1,
            leadership: 12.0,
            composure: 12.0,
            teamwork: 12.0,
            determination: 12.0,
            pressure_handling: 12.0,
            world_reputation: 4_000,
            club_reputation: 4_500,
            league_reputation: 600,
            position_levels: vec![(position, 18)],
            position_group,
            goals: 3,
            assists: 2,
            player_of_the_match: 1,
            clean_sheets: 1,
            yellow_cards: 1,
            red_cards: 0,
            last_season_apps: 30,
            last_season_rating: 7.2,
            last_season_goals: 5,
        }
    }

    fn make_player_with_history(
        id: u32,
        current_apps: u16,
        last_season_apps: u16,
        ability: u8,
    ) -> Player {
        let mut current_stats = PlayerStatistics::default();
        current_stats.played = current_apps;
        current_stats.average_rating = if current_apps > 0 { 7.0 } else { 0.0 };

        let last_season = Season::new(2025);
        let mut hist_stats = PlayerStatistics::default();
        hist_stats.played = last_season_apps;
        hist_stats.goals = 8;
        hist_stats.average_rating = 7.4;

        let history = PlayerStatisticsHistory::from_items(vec![
            PlayerStatisticsHistoryItem {
                season: last_season,
                team_name: "Test Club".to_string(),
                team_slug: "test-club".to_string(),
                team_reputation: 5_000,
                league_name: "Test League".to_string(),
                league_slug: "test-league".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: hist_stats,
                seq_id: 0,
            },
        ]);

        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(1996, 5, 1).unwrap())
            .country_id(1)
            .skills(PlayerSkills::default())
            .attributes(PersonAttributes::default())
            .player_attributes(PlayerAttributes {
                current_ability: ability,
                potential_ability: ability + 10,
                condition: 10000,
                world_reputation: (ability as i16) * 30,
                ..Default::default()
            })
            .contract(None)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .statistics(current_stats)
            .statistics_history(history)
            .build()
            .expect("build test player")
    }

    #[test]
    fn derive_reasons_returns_position_need_when_flagged() {
        let c = make_candidate(1, 110, PlayerFieldPositionGroup::Defender);
        let (primary, _secondaries) = NationalTeam::derive_reasons(&c, true);
        assert_eq!(primary, CallUpReason::PositionNeed);
    }

    #[test]
    fn derive_reasons_picks_key_player_for_high_ability_and_world_rep() {
        let mut c = make_candidate(1, 175, PlayerFieldPositionGroup::Midfielder);
        c.world_reputation = 8_000;
        c.average_rating = 6.0; // reduce to avoid CurrentForm winning
        c.played = 0;
        c.international_apps = 5;
        let (primary, secondaries) = NationalTeam::derive_reasons(&c, false);
        assert_eq!(primary, CallUpReason::KeyPlayer);
        assert!(!secondaries.contains(&CallUpReason::PositionNeed));
    }

    #[test]
    fn derive_reasons_picks_youth_prospect_for_high_potential_youngsters() {
        let mut c = make_candidate(1, 130, PlayerFieldPositionGroup::Forward);
        c.age = 20;
        c.potential_ability = 175;
        c.world_reputation = 1_000;
        c.average_rating = 6.5;
        c.played = 4;
        c.international_apps = 0;
        c.last_season_apps = 12;
        c.league_reputation = 400;
        c.position_levels = vec![(PlayerPositionType::Striker, 14)];
        let (primary, _) = NationalTeam::derive_reasons(&c, false);
        assert_eq!(primary, CallUpReason::YouthProspect);
    }

    #[test]
    fn summarise_last_season_aggregates_multiple_items() {
        // Mid-season transfer: same season, two items. Apps and goals
        // should sum across both, rating should be a games-weighted blend.
        let season = Season::new(2025);
        let mut a = PlayerStatistics::default();
        a.played = 10;
        a.goals = 4;
        a.average_rating = 7.0;
        let mut b = PlayerStatistics::default();
        b.played = 20;
        b.goals = 8;
        b.average_rating = 8.0;

        let history = PlayerStatisticsHistory::from_items(vec![
            PlayerStatisticsHistoryItem {
                season: season.clone(),
                team_name: "A".to_string(),
                team_slug: "a".to_string(),
                team_reputation: 3000,
                league_name: "L".to_string(),
                league_slug: "l".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: a,
                seq_id: 0,
            },
            PlayerStatisticsHistoryItem {
                season,
                team_name: "B".to_string(),
                team_slug: "b".to_string(),
                team_reputation: 3000,
                league_name: "L".to_string(),
                league_slug: "l".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: b,
                seq_id: 1,
            },
        ]);

        let player = PlayerBuilder::new()
            .id(99)
            .full_name(FullName::new("T".into(), "P".into()))
            .birth_date(NaiveDate::from_ymd_opt(1995, 1, 1).unwrap())
            .country_id(1)
            .skills(PlayerSkills::default())
            .attributes(PersonAttributes::default())
            .player_attributes(PlayerAttributes {
                current_ability: 130,
                potential_ability: 140,
                condition: 10000,
                ..Default::default()
            })
            .contract(None)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .statistics_history(history)
            .build()
            .expect("build");

        let (apps, rating, goals) = NationalTeam::summarise_last_season(&player);
        assert_eq!(apps, 30);
        assert_eq!(goals, 12);
        // weighted: (10 * 7.0 + 20 * 8.0) / 30 = 7.6666…
        assert!((rating - 7.6667).abs() < 0.01, "got rating {}", rating);
    }

    #[test]
    fn build_candidate_accepts_player_with_low_current_apps_but_strong_history() {
        // September call-up: player has 1 game this season, 32 last season.
        // Without history blending this would be filtered as "unproven".
        let player = make_player_with_history(1, 1, 32, 130);
        let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
        let c = NationalTeam::build_candidate(&player, 1, 1, 5_000, 700, date);
        assert!(c.is_some(), "player with strong prev-season history must qualify");
        let c = c.unwrap();
        assert_eq!(c.last_season_apps, 32);
        assert_eq!(c.played, 1);
    }

    #[test]
    fn build_candidate_rejects_player_with_no_track_record() {
        // No current games, no prior season, no caps — drop them.
        let player = make_player_with_history(1, 1, 0, 80);
        let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
        let c = NationalTeam::build_candidate(&player, 1, 1, 3_000, 400, date);
        assert!(c.is_none(), "player without any minutes must be rejected");
    }

    #[test]
    fn select_balanced_squad_respects_positional_quotas() {
        // Build a healthy candidate pool.
        let mut candidates: Vec<CallUpCandidate> = Vec::new();
        for i in 0..5 {
            candidates.push(make_candidate(100 + i, 140, PlayerFieldPositionGroup::Goalkeeper));
        }
        for i in 0..10 {
            candidates.push(make_candidate(200 + i, 145, PlayerFieldPositionGroup::Defender));
        }
        for i in 0..10 {
            candidates.push(make_candidate(300 + i, 150, PlayerFieldPositionGroup::Midfielder));
        }
        for i in 0..10 {
            candidates.push(make_candidate(400 + i, 150, PlayerFieldPositionGroup::Forward));
        }

        let tactics = Tactics::new(MatchTacticType::T442);
        let selected = NationalTeam::select_balanced_squad(&candidates, &tactics, false, 1);
        assert_eq!(selected.len(), SQUAD_SIZE, "squad must reach full size");

        let count_group = |g: PlayerFieldPositionGroup| -> usize {
            selected
                .iter()
                .filter(|(idx, _, _)| candidates[*idx].position_group == g)
                .count()
        };

        assert!(count_group(PlayerFieldPositionGroup::Goalkeeper) >= 3);
        assert!(count_group(PlayerFieldPositionGroup::Defender) >= 6);
        assert!(count_group(PlayerFieldPositionGroup::Midfielder) >= 6);
        assert!(count_group(PlayerFieldPositionGroup::Forward) >= 5);
    }

    #[test]
    fn select_balanced_squad_assigns_reasons_to_every_pick() {
        let mut candidates: Vec<CallUpCandidate> = Vec::new();
        for i in 0..4 {
            candidates.push(make_candidate(100 + i, 140, PlayerFieldPositionGroup::Goalkeeper));
        }
        for i in 0..10 {
            candidates.push(make_candidate(200 + i, 145, PlayerFieldPositionGroup::Defender));
        }
        for i in 0..10 {
            candidates.push(make_candidate(300 + i, 150, PlayerFieldPositionGroup::Midfielder));
        }
        for i in 0..10 {
            candidates.push(make_candidate(400 + i, 150, PlayerFieldPositionGroup::Forward));
        }

        let tactics = Tactics::new(MatchTacticType::T442);
        let selected = NationalTeam::select_balanced_squad(&candidates, &tactics, false, 1);

        // Every pick must carry a primary reason. RegularStarter is the
        // generic fallback — anything else means a threshold tripped.
        let known_reasons: HashSet<CallUpReason> = [
            CallUpReason::KeyPlayer,
            CallUpReason::CurrentForm,
            CallUpReason::RegularStarter,
            CallUpReason::StrongLeague,
            CallUpReason::TacticalFit,
            CallUpReason::PositionNeed,
            CallUpReason::InternationalExperience,
            CallUpReason::Leadership,
            CallUpReason::YouthProspect,
        ]
        .into_iter()
        .collect();

        for (_, primary, _) in &selected {
            assert!(known_reasons.contains(primary), "primary reason {:?} not in expected set", primary);
        }
    }

    #[test]
    fn call_up_squad_clears_generated_squad_on_subsequent_call() {
        let mut nt = NationalTeam {
            country_id: 1,
            country_name: "TestLand".to_string(),
            staff: Vec::new(),
            squad: Vec::new(),
            generated_squad: Vec::new(),
            tactics: Tactics::new(MatchTacticType::T442),
            reputation: 5_000,
            elo_rating: 1500,
            schedule: Vec::new(),
        };

        // First call-up: no real candidates → entirely synthetic depth.
        let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
        nt.call_up_squad(Vec::new(), date, 1, &[(2, "Other".to_string())]);
        assert!(!nt.generated_squad.is_empty(), "first call-up should have generated synthetic players");
        let initial_synthetic_count = nt.generated_squad.len();

        // Second call-up with enough real candidates — the synthetic
        // pool must be cleared, not accumulated.
        let mut candidates: Vec<CallUpCandidate> = Vec::new();
        for i in 0..3 {
            candidates.push(make_candidate(100 + i, 150, PlayerFieldPositionGroup::Goalkeeper));
        }
        for i in 0..8 {
            candidates.push(make_candidate(200 + i, 150, PlayerFieldPositionGroup::Defender));
        }
        for i in 0..8 {
            candidates.push(make_candidate(300 + i, 150, PlayerFieldPositionGroup::Midfielder));
        }
        for i in 0..6 {
            candidates.push(make_candidate(400 + i, 150, PlayerFieldPositionGroup::Forward));
        }

        let next_break = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        nt.call_up_squad(candidates, next_break, 1, &[(2, "Other".to_string())]);

        assert!(
            nt.generated_squad.is_empty(),
            "generated_squad must be cleared when real players are available; was {} before, {} after",
            initial_synthetic_count,
            nt.generated_squad.len()
        );
        assert_eq!(nt.squad.len(), SQUAD_SIZE);
    }

    #[test]
    fn call_up_squad_preserves_completed_fixtures_when_reselecting() {
        // Older completed fixture must survive a re-call-up. A pending
        // fixture in the new break window is expected to be replaced.
        let mut nt = NationalTeam {
            country_id: 1,
            country_name: "TestLand".to_string(),
            staff: Vec::new(),
            squad: Vec::new(),
            generated_squad: Vec::new(),
            tactics: Tactics::new(MatchTacticType::T442),
            reputation: 5_000,
            elo_rating: 1500,
            schedule: vec![NationalTeamFixture {
                date: NaiveDate::from_ymd_opt(2025, 9, 6).unwrap(),
                opponent_country_id: 2,
                opponent_country_name: "Old Opp".to_string(),
                is_home: true,
                competition_name: "Friendly".to_string(),
                match_id: String::new(),
                result: Some(NationalTeamMatchResult {
                    home_score: 2,
                    away_score: 1,
                    date: NaiveDate::from_ymd_opt(2025, 9, 6).unwrap(),
                    opponent_country_id: 2,
                }),
            }],
        };

        let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
        nt.call_up_squad(Vec::new(), date, 1, &[(2, "Other".to_string())]);

        assert!(
            nt.schedule.iter().any(|f| f.result.is_some() && f.opponent_country_name == "Old Opp"),
            "previous completed fixture must be preserved across a re-call-up"
        );
    }

    #[test]
    fn league_reputation_is_zero_when_no_league_assigned_in_candidate() {
        // Sanity: the candidate captures the league_reputation we passed in,
        // distinct from club_reputation. The bug we fixed was using
        // team.reputation.world for both — this test pins the separation.
        let player = make_player_with_history(1, 10, 25, 130);
        let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
        let c = NationalTeam::build_candidate(&player, 1, 1, 7_000, 250, date)
            .expect("candidate should build");
        assert_eq!(c.club_reputation, 7_000);
        assert_eq!(c.league_reputation, 250);
    }

    #[test]
    fn weak_country_still_gets_squad_but_no_friendlies() {
        // A nation below MIN_REPUTATION_FOR_FRIENDLIES used to be skipped
        // entirely — meaning a real qualifier match would trigger the
        // emergency call-up path. Now they get a normal squad selection;
        // only the friendly fixtures are gated by reputation.
        let mut nt = NationalTeam {
            country_id: 1,
            country_name: "Tiny".to_string(),
            staff: Vec::new(),
            squad: Vec::new(),
            generated_squad: Vec::new(),
            tactics: Tactics::new(MatchTacticType::T442),
            reputation: 1_500, // well below MIN_REPUTATION_FOR_FRIENDLIES (4000)
            elo_rating: 1500,
            schedule: Vec::new(),
        };

        let mut candidates: Vec<CallUpCandidate> = Vec::new();
        for i in 0..3 {
            candidates.push(make_candidate(100 + i, 100, PlayerFieldPositionGroup::Goalkeeper));
        }
        for i in 0..8 {
            candidates.push(make_candidate(200 + i, 110, PlayerFieldPositionGroup::Defender));
        }
        for i in 0..8 {
            candidates.push(make_candidate(300 + i, 110, PlayerFieldPositionGroup::Midfielder));
        }
        for i in 0..6 {
            candidates.push(make_candidate(400 + i, 110, PlayerFieldPositionGroup::Forward));
        }

        let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
        nt.call_up_squad(candidates, date, 1, &[(2, "Other".to_string())]);

        assert_eq!(
            nt.squad.len(),
            SQUAD_SIZE,
            "weak country must still get a full real squad"
        );
        let pending_friendlies = nt
            .schedule
            .iter()
            .filter(|f| f.competition_name == "Friendly" && f.result.is_none())
            .count();
        assert_eq!(
            pending_friendlies, 0,
            "weak country must not get auto-scheduled friendlies"
        );
    }

    #[test]
    fn stale_pending_friendlies_are_dropped_on_recall() {
        // Three classes of pre-existing fixtures must be handled:
        //   completed_past:           kept (history)
        //   pending_past:             dropped (never played, stale)
        //   pending_current_window:   dropped (will be re-scheduled)
        //   pending_future_window:    kept (not ours to touch)
        let completed_past = NationalTeamFixture {
            date: NaiveDate::from_ymd_opt(2025, 9, 6).unwrap(),
            opponent_country_id: 2,
            opponent_country_name: "Hist".to_string(),
            is_home: true,
            competition_name: "Friendly".to_string(),
            match_id: String::new(),
            result: Some(NationalTeamMatchResult {
                home_score: 1,
                away_score: 0,
                date: NaiveDate::from_ymd_opt(2025, 9, 6).unwrap(),
                opponent_country_id: 2,
            }),
        };
        let pending_past = NationalTeamFixture {
            date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
            opponent_country_id: 3,
            opponent_country_name: "Stale".to_string(),
            is_home: false,
            competition_name: "Friendly".to_string(),
            match_id: String::new(),
            result: None,
        };
        let pending_current = NationalTeamFixture {
            date: NaiveDate::from_ymd_opt(2026, 9, 6).unwrap(),
            opponent_country_id: 4,
            opponent_country_name: "OldPending".to_string(),
            is_home: true,
            competition_name: "Friendly".to_string(),
            match_id: String::new(),
            result: None,
        };
        let pending_future = NationalTeamFixture {
            date: NaiveDate::from_ymd_opt(2026, 11, 14).unwrap(),
            opponent_country_id: 5,
            opponent_country_name: "Future".to_string(),
            is_home: false,
            competition_name: "Friendly".to_string(),
            match_id: String::new(),
            result: None,
        };

        let mut nt = NationalTeam {
            country_id: 1,
            country_name: "TestLand".to_string(),
            staff: Vec::new(),
            squad: Vec::new(),
            generated_squad: Vec::new(),
            tactics: Tactics::new(MatchTacticType::T442),
            // Below MIN_REPUTATION_FOR_FRIENDLIES so no fresh friendlies
            // are added — keeps the assertion clean.
            reputation: 1_500,
            elo_rating: 1500,
            schedule: vec![
                completed_past,
                pending_past,
                pending_current,
                pending_future,
            ],
        };

        let date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
        nt.call_up_squad(Vec::new(), date, 1, &[(2, "Other".to_string())]);

        let names: Vec<_> = nt.schedule.iter().map(|f| f.opponent_country_name.as_str()).collect();
        assert!(names.contains(&"Hist"), "completed past fixture must be kept");
        assert!(!names.contains(&"Stale"), "pending past fixture must be dropped");
        assert!(
            !names.contains(&"OldPending"),
            "pending fixture in current break window must be dropped"
        );
        assert!(
            names.contains(&"Future"),
            "pending fixture in future window must be kept"
        );
    }

    #[test]
    fn squad_picks_returns_real_then_synthetic_with_synthetic_depth_reason() {
        let mut nt = NationalTeam {
            country_id: 1,
            country_name: "TestLand".to_string(),
            staff: Vec::new(),
            squad: vec![NationalSquadPlayer {
                player_id: 42,
                club_id: 7,
                team_id: 7,
                primary_reason: CallUpReason::KeyPlayer,
                secondary_reasons: Vec::new(),
            }],
            generated_squad: Vec::new(),
            tactics: Tactics::new(MatchTacticType::T442),
            reputation: 5_000,
            elo_rating: 1500,
            schedule: Vec::new(),
        };

        // Force-generate one synthetic player using the existing helper.
        let synth_date = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();
        nt.generated_squad.push(NationalTeam::generate_synthetic_player(
            1,
            synth_date,
            PlayerPositionType::Goalkeeper,
            120,
            0,
        ));

        let picks = nt.squad_picks();
        assert_eq!(picks.len(), 2);

        match &picks[0] {
            SquadPick::Real(sp) => {
                assert_eq!(sp.player_id, 42);
                assert_eq!(sp.primary_reason, CallUpReason::KeyPlayer);
            }
            _ => panic!("first pick should be Real"),
        }
        match &picks[1] {
            SquadPick::Synthetic(player) => {
                // A synthetic pick is rendered with reason
                // SyntheticDepth at the UI boundary; the enum itself
                // carries the player record, not the reason.
                assert!(
                    player.id >= 900_000,
                    "synthetic player ids start at 900_000+"
                );
            }
            _ => panic!("second pick should be Synthetic"),
        }
    }
}
