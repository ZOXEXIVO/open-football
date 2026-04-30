//! National-team simulation, split by concern. The `NationalTeam`
//! struct lives here along with the squad/match-squad lifecycle; every
//! larger pass has been moved to a sibling module:
//!
//! | Submodule        | Concern                                                    |
//! |------------------|------------------------------------------------------------|
//! | [`types`]        | Free types and constants (staff/squad records, fixture/result, candidate, break/tournament windows) |
//! | [`callup`]       | Candidate collection, scoring, balanced selection, reason derivation |
//! | [`synthetic`]    | Synthetic-squad and synthetic-player generation             |
//! | [`world_status`] | World-wide `PlayerStatusType::Int` apply/release passes     |
//! | [`calendar`]     | International-break and tournament-window predicates        |

mod calendar;
mod callup;
mod synthetic;
mod types;
mod world_status;

pub use types::*;

use crate::country::PeopleNameGeneratorData;
use crate::r#match::{MatchPlayer, MatchResultRaw, MatchSquad};
use crate::utils::IntegerUtils;
use crate::{Club, MatchTacticType, Player, PlayerPositionType, Tactics};
use chrono::NaiveDate;
use log::debug;

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
                            player.player_attributes.international_goals += stats.goals as u16;
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

    /// Update Elo rating after a match
    pub fn update_elo(&mut self, our_score: u8, opponent_score: u8, opponent_elo: u16) {
        let k: f32 = 20.0;
        let expected =
            1.0 / (1.0 + 10.0_f32.powf((opponent_elo as f32 - self.elo_rating as f32) / 400.0));

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
    pub fn build_synthetic_opponent_squad(
        opponent_country_id: u32,
        opponent_name: &str,
    ) -> MatchSquad {
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
                .max_by_key(|p| {
                    p.positions.get_level(pos) as u16 + p.player_attributes.current_ability as u16
                })
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
}

#[cfg(test)]
mod tests;
