use super::ContinentResult;
use crate::HappinessEventType;
use crate::continent::Continent;
use crate::country::CountryResult;
use crate::simulator::SimulatorData;
use log::debug;

impl ContinentResult {
    pub(crate) fn process_continental_awards(
        &self,
        data: &mut SimulatorData,
        _country_results: &[CountryResult],
    ) {
        debug!("Processing continental awards");

        let continent_id = self.get_continent_id();
        let date = data.date.date();

        // Build a season-score ranking that drives both shortlist and
        // winner. Reputation is a tiebreak, never a dominant axis.
        let ranking: Vec<(u32, f32)> = if let Some(continent) = data.continent(continent_id) {
            Self::rank_continent(continent, date)
        } else {
            return;
        };

        let top_three: Vec<u32> = ranking.iter().take(3).map(|(id, _)| *id).collect();
        let winner = ranking.first().map(|(id, _)| *id);

        // Nomination events for the top 3 — guard with cooldowns so a
        // back-to-back year-end recompute doesn't double-emit.
        for pid in &top_three {
            if let Some(player) = data.player_mut(*pid) {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::ContinentalPlayerOfYearNomination,
                    330,
                );
            }
        }

        if let Some(id) = winner {
            if let Some(player) = data.player_mut(id) {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::ContinentalPlayerOfYear,
                    330,
                );
                let cur = player.player_attributes.current_reputation;
                let home = player.player_attributes.home_reputation;
                let world = player.player_attributes.world_reputation;
                player.player_attributes.update_reputation(
                    ((cur as i32 + 500).min(10000) - cur as i32) as i16,
                    ((home as i32 + 500).min(10000) - home as i32) as i16,
                    ((world as i32 + 250).min(10000) - world as i32) as i16,
                );
            }
        }

        // Continental cup trophies / final defeats. Each tier emits to
        // both finalists when the engine resolves a `Final` knockout tie.
        // Today the engine doesn't schedule continental finals, so the
        // accessors return None and this is a no-op — the wiring is here
        // so adding final-stage scheduling later auto-fires the events.
        Self::process_continental_cup_finals(data, continent_id, date);
    }

    /// Per-player season-aware ranking across every league of the
    /// continent. Scoring leans on real performance (rating, goals,
    /// assists, motm) with a league-reputation envelope; current
    /// reputation participates only as a tiebreak.
    pub(crate) fn rank_continent(continent: &Continent, _date: chrono::NaiveDate) -> Vec<(u32, f32)> {
        let mut out: Vec<(u32, f32)> = Vec::new();
        for country in &continent.countries {
            for league in &country.leagues.leagues {
                if league.friendly {
                    continue;
                }
                let league_rep = league.reputation as f32 / 10000.0;
                let envelope = 0.65 + league_rep * 0.55;
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if team.league_id != Some(league.id) {
                            continue;
                        }
                        for player in &team.players.players {
                            let apps = player.statistics.played + player.statistics.played_subs;
                            if apps < 8 {
                                continue;
                            }
                            let goals = player.statistics.goals as f32;
                            let assists = player.statistics.assists as f32;
                            let avg = player.statistics.average_rating;
                            let motm = player.statistics.player_of_the_match as f32;
                            let raw =
                                (avg - 6.0).max(0.0) * 18.0 + goals * 1.6 + assists * 1.2 + motm * 3.0;
                            // Cup statistics give a small bonus for
                            // continental cup engagement when available.
                            let cup_goals = player.cup_statistics.goals as f32;
                            let cup_assists = player.cup_statistics.assists as f32;
                            let cup_bonus = (cup_goals * 1.4 + cup_assists * 1.0).min(40.0);
                            let rep_tiebreak =
                                player.player_attributes.current_reputation as f32 / 10000.0 * 3.0;
                            let total = raw * envelope + cup_bonus + rep_tiebreak;
                            if total > 0.0 {
                                out.push((player.id, total));
                            }
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        out
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
        let finals: Vec<(u32, u32, f32, f32)> =
            if let Some(continent) = data.continent(continent_id) {
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

}
