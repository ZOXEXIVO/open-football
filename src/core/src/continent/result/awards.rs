use super::ContinentResult;
use crate::continent::Continent;
use crate::country::CountryResult;
use crate::simulator::SimulatorData;
use crate::{AwardReputationInput, AwardReputationKind, HappinessEventType};
use log::debug;

/// Year-end award outcome staged by the parallel continent pass so the
/// orchestrator can apply the cross-continent player events serially.
/// `top_three` and `winner` are filled from the season ranking; cup
/// finalists are applied inside the parallel pass because they only
/// touch the continent's own countries.
pub(crate) struct ContinentAwardOutcome {
    pub top_three: Vec<u32>,
    pub winner: Option<u32>,
}

impl ContinentResult {
    /// Parallel-pass slice of continental awards: rank players inside
    /// this continent and emit the trophy / final-defeat events to the
    /// finalists' squads. Returns the top-3 / winner ids so the
    /// orchestrator can apply the cross-continent player events
    /// serially (`data.player_mut` walks every continent).
    pub(crate) fn build_continental_award_outcome(
        continent: &mut Continent,
        date: chrono::NaiveDate,
    ) -> ContinentAwardOutcome {
        debug!("Processing continental awards");

        let ranking = Self::rank_continent(continent, date);
        let top_three: Vec<u32> = ranking.iter().take(3).map(|(id, _)| *id).collect();
        let winner = ranking.first().map(|(id, _)| *id);

        // Continental cup trophies / final defeats. Each tier emits to
        // both finalists when the engine resolves a `Final` knockout tie.
        // Today the engine doesn't schedule continental finals, so the
        // accessors return None and this is a no-op — the wiring is here
        // so adding final-stage scheduling later auto-fires the events.
        // Continent-local: only writes to the finalists' own-continent
        // countries, so it stays in the parallel pass.
        Self::apply_continental_cup_finals_local(continent, date);

        ContinentAwardOutcome { top_three, winner }
    }

    /// Apply the cross-continent player events for a year-end award
    /// outcome. Runs serially after the parallel pass because
    /// `data.player_mut` resolves against every continent.
    pub(crate) fn apply_continental_award_outcome(
        data: &mut SimulatorData,
        outcome: ContinentAwardOutcome,
        date: chrono::NaiveDate,
    ) {
        // Nomination events for the top 3 — guard with cooldowns so a
        // back-to-back year-end recompute doesn't double-emit.
        for pid in &outcome.top_three {
            if let Some(player) = data.player_mut(*pid) {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::ContinentalPlayerOfYearNomination,
                    330,
                );
            }
        }

        if let Some(id) = outcome.winner {
            if let Some(player) = data.player_mut(id) {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::ContinentalPlayerOfYear,
                    330,
                );
                player.apply_award_reputation_impact(
                    AwardReputationKind::ContinentalPlayerOfYear,
                    AwardReputationInput::new(),
                    date,
                );
            }
        }
    }

    /// Per-player season-aware ranking across every league of the
    /// continent. Scoring leans on real performance (rating, goals,
    /// assists, motm) with a league-reputation envelope; current
    /// reputation participates only as a tiebreak.
    pub(crate) fn rank_continent(
        continent: &Continent,
        _date: chrono::NaiveDate,
    ) -> Vec<(u32, f32)> {
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
                            // Continental award scoring leans heavily on
                            // `(avg - 6.0) * 18.0`, so an 8.2 raw becomes
                            // +39.6 against a regressed 7.25's +22.5. Use
                            // the regressed value so a 9-app phenom can't
                            // outscore a 30-app proven season.
                            let pos = player.position().position_group();
                            let avg = player.statistics.average_rating_realistic(pos);
                            let motm = player.statistics.player_of_the_match as f32;
                            let raw = (avg - 6.0).max(0.0) * 18.0
                                + goals * 1.6
                                + assists * 1.2
                                + motm * 3.0;
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
    /// Continent-local — only touches the continent's own countries, so
    /// it's safe to run inside the parallel pass.
    fn apply_continental_cup_finals_local(continent: &mut Continent, date: chrono::NaiveDate) {
        // Snapshot finals up front so we don't keep an immutable borrow
        // while emitting. (CL 1.5 / EL 1.3 / Conference 1.2 reflect the
        // real-world prestige gap.)
        let comps = &continent.continental_competitions;
        let mut finals: Vec<(u32, u32, f32, f32)> = Vec::new();
        if let Some((w, l)) = comps.champions_league.final_result() {
            finals.push((w, l, 1.5, 1.4));
        }
        if let Some((w, l)) = comps.europa_league.final_result() {
            finals.push((w, l, 1.3, 1.2));
        }
        if let Some((w, l)) = comps.conference_league.final_result() {
            finals.push((w, l, 1.2, 1.0));
        }

        for (winner_team, loser_team, win_prestige, lose_prestige) in finals {
            // Continental cups can span the whole continent, so we walk
            // the country list.
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
