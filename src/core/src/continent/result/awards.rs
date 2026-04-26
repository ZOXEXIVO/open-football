use super::ContinentResult;
use crate::continent::Continent;
use crate::country::CountryResult;
use crate::simulator::SimulatorData;
use crate::utils::DateUtils;
use crate::HappinessEventType;
use log::debug;

impl ContinentResult {
    pub(crate) fn process_continental_awards(&self, data: &mut SimulatorData, _country_results: &[CountryResult]) {
        debug!("Processing continental awards");

        let continent_id = self.get_continent_id();
        let date = data.date.date();

        if let Some(continent) = data.continent(continent_id) {
            let player_of_year = Self::determine_player_of_year(continent, date);
            let _team_of_year = Self::determine_team_of_year(continent);
            let _coach_of_year = Self::determine_coach_of_year(continent);
            let young_player = Self::determine_young_player_award(continent, date);

            if let Some(id) = player_of_year {
                debug!("Player of the Year: {}", id);
            }
            if let Some(id) = young_player {
                debug!("Young Player of the Year: {}", id);
            }
        }

        // Award reputation boost to winners
        if let Some(continent) = data.continent(continent_id) {
            let player_id = Self::determine_player_of_year(continent, date);
            let young_id = Self::determine_young_player_award(continent, date);

            if let Some(id) = player_id {
                if let Some(player) = data.player_mut(id) {
                    // Significant reputation boost for continental award
                    player.player_attributes.current_reputation =
                        (player.player_attributes.current_reputation + 500).min(10000);
                }
            }
            if let Some(id) = young_id {
                if let Some(player) = data.player_mut(id) {
                    player.player_attributes.current_reputation =
                        (player.player_attributes.current_reputation + 300).min(10000);
                }
            }
        }

        // Continental cup trophies / final defeats. Each tier emits to
        // both finalists when the engine resolves a `Final` knockout tie.
        // Today the engine doesn't schedule continental finals, so the
        // accessors return None and this is a no-op — the wiring is here
        // so adding final-stage scheduling later auto-fires the events.
        Self::process_continental_cup_finals(data, continent_id, date);
    }

    /// Emit `TrophyWon` for the cup winner and `CupFinalDefeat` for the
    /// losing finalist of each continental competition. Prestige factors
    /// follow the canonical band (CL > EL > Conference). Cooldown is
    /// 365 days so back-to-back end-of-period ticks don't double-fire.
    fn process_continental_cup_finals(
        data: &mut SimulatorData,
        continent_id: u32,
        date: chrono::NaiveDate,
    ) {
        // Snapshot finals up front so we don't keep an immutable borrow
        // while emitting. (CL 1.5 / EL 1.3 / Conference 1.2 reflect the
        // real-world prestige gap.)
        let finals: Vec<(u32, u32, f32, f32)> = if let Some(continent) = data.continent(continent_id) {
            let comps = &continent.continental_competitions;
            let mut v = Vec::new();
            if let Some((w, l)) = comps.champions_league.final_result() {
                v.push((w, l, 1.5, 1.4));
            }
            if let Some((w, l)) = comps.europa_league.final_result() {
                v.push((w, l, 1.3, 1.2));
            }
            if let Some((w, l)) = comps.conference_league.final_result() {
                v.push((w, l, 1.2, 1.0));
            }
            v
        } else {
            return;
        };

        for (winner_team, loser_team, win_prestige, lose_prestige) in finals {
            // Locate which country owns each team. Continental cups can
            // span the whole continent, so we walk the country list.
            if let Some(continent) = data.continent_mut(continent_id) {
                for country in continent.countries.iter_mut() {
                    CountryResult::apply_team_squad_event(
                        country,
                        winner_team,
                        HappinessEventType::TrophyWon,
                        365,
                        win_prestige,
                        date,
                    );
                    CountryResult::apply_team_squad_event(
                        country,
                        loser_team,
                        HappinessEventType::CupFinalDefeat,
                        365,
                        lose_prestige,
                        date,
                    );
                }
            }
        }
    }

    /// Best player: highest combined goals + assists + avg rating, weighted by league reputation
    fn determine_player_of_year(continent: &Continent, _date: chrono::NaiveDate) -> Option<u32> {
        let mut best_id: Option<u32> = None;
        let mut best_score: f32 = 0.0;

        for country in &continent.countries {
            // League reputation multiplier: top leagues produce better candidates
            let league_rep = country.leagues.leagues.first()
                .map(|l| l.reputation as f32 / 10000.0)
                .unwrap_or(0.1);

            for club in &country.clubs {
                for team in &club.teams.teams {
                    for player in &team.players.players {
                        // Skip players with too few appearances
                        let appearances = player.statistics.played + player.statistics.played_subs;
                        if appearances < 10 {
                            continue;
                        }

                        let goals = player.statistics.goals as f32;
                        let assists = player.statistics.assists as f32;
                        let avg_rating = player.statistics.average_rating;

                        // Score: (goals×2 + assists×1.5 + avg_rating×3) × league_rep
                        let score = (goals * 2.0 + assists * 1.5 + avg_rating * 3.0)
                            * (0.5 + league_rep * 0.5);

                        // Bonus for high current ability
                        let ability_bonus = player.player_attributes.current_ability as f32 / 200.0;
                        let total = score + ability_bonus * 5.0;

                        if total > best_score {
                            best_score = total;
                            best_id = Some(player.id);
                        }
                    }
                }
            }
        }

        best_id
    }

    /// Best team: highest league points total, weighted by league reputation
    fn determine_team_of_year(continent: &Continent) -> Option<u32> {
        let mut best_team_id: Option<u32> = None;
        let mut best_score: f32 = 0.0;

        for country in &continent.countries {
            for league in &country.leagues.leagues {
                if league.friendly {
                    continue;
                }

                let league_rep = league.reputation as f32 / 10000.0;

                for row in &league.table.rows {
                    let points = row.points as f32;
                    let gd = (row.goal_scored - row.goal_concerned) as f32;
                    // Score: (points×3 + gd×0.5) × league_rep
                    let score = (points * 3.0 + gd * 0.5) * (0.5 + league_rep * 0.5);

                    if score > best_score {
                        best_score = score;
                        best_team_id = Some(row.team_id);
                    }
                }
            }
        }

        best_team_id
    }

    /// Best coach: head coach of the team of the year
    fn determine_coach_of_year(continent: &Continent) -> Option<u32> {
        // Find the team with best league performance
        let best_team_id = Self::determine_team_of_year(continent)?;

        for country in &continent.countries {
            for club in &country.clubs {
                for team in &club.teams.teams {
                    if team.id == best_team_id {
                        return Some(team.staffs.head_coach().id);
                    }
                }
            }
        }

        None
    }

    /// Best young player (U23): same scoring as player of year, but age < 23
    fn determine_young_player_award(continent: &Continent, date: chrono::NaiveDate) -> Option<u32> {
        let mut best_id: Option<u32> = None;
        let mut best_score: f32 = 0.0;

        for country in &continent.countries {
            let league_rep = country.leagues.leagues.first()
                .map(|l| l.reputation as f32 / 10000.0)
                .unwrap_or(0.1);

            for club in &country.clubs {
                for team in &club.teams.teams {
                    for player in &team.players.players {
                        let age = DateUtils::age(player.birth_date, date);
                        if age >= 23 {
                            continue;
                        }

                        let appearances = player.statistics.played + player.statistics.played_subs;
                        if appearances < 8 {
                            continue;
                        }

                        let goals = player.statistics.goals as f32;
                        let assists = player.statistics.assists as f32;
                        let avg_rating = player.statistics.average_rating;
                        let potential = player.player_attributes.potential_ability as f32;

                        // Young player score includes potential
                        let score = (goals * 2.0 + assists * 1.5 + avg_rating * 3.0 + potential / 20.0)
                            * (0.5 + league_rep * 0.5);

                        if score > best_score {
                            best_score = score;
                            best_id = Some(player.id);
                        }
                    }
                }
            }
        }

        best_id
    }
}
