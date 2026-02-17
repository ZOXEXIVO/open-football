use crate::context::GlobalContext;
use crate::continent::{Tournament, TournamentContext};

#[allow(dead_code)]
pub struct LeagueEurope {}

impl LeagueEurope {}

impl Tournament for LeagueEurope {
    fn simulate(&mut self, _: &mut TournamentContext, _: GlobalContext<'_>) {}
}
