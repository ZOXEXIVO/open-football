use crate::context::GlobalContext;
use crate::league::{League, LeagueResult};
use crate::{Club, Logging};
use rayon::iter::IntoParallelRefMutIterator;
use rayon::iter::ParallelIterator;

pub struct LeagueCollection {
    pub leagues: Vec<League>,
}

impl LeagueCollection {
    pub fn new(leagues: Vec<League>) -> Self {
        LeagueCollection { leagues }
    }

    pub fn simulate(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) -> Vec<LeagueResult> {
        let teams_ids: Vec<(u32, Option<u32>)> = clubs
            .iter()
            .flat_map(|c| &c.teams.teams)
            .map(|c| (c.id, c.league_id))
            .collect();

        self.leagues
            .par_iter_mut()
            .map(|league| {
                let league_team_ids: Vec<u32> = teams_ids
                    .iter()
                    .filter(|(_, league_id)| *league_id == Some(league.id))
                    .map(|(id, _)| *id)
                    .collect();
                {
                    let message = &format!("simulate league: {}", &league.name);

                    let league_slug = String::from(&league.slug);

                    Logging::estimate_result(
                        || {
                            league.simulate(
                                clubs,
                                ctx.with_league(league.id, league_slug, &league_team_ids),
                            )
                        },
                        message,
                    )
                }
            })
            .collect()
    }
}
