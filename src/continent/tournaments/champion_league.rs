use crate::continent::{Tournament, TournamentContext};
use crate::simulator::context::GlobalContext;

pub struct ChampionLeague {}

impl ChampionLeague {}

impl Tournament for ChampionLeague {
    fn simulate(&mut self, tournament_ctx: &mut TournamentContext, ctx: &mut GlobalContext) {}
}
