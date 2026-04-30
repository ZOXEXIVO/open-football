#[derive(Clone)]
pub struct LeagueContext<'l> {
    pub id: u32,
    pub slug: String,
    pub team_ids: &'l [u32],
    /// League reputation (0-10000) — used to scale player development by competition quality
    pub reputation: u16,
}

impl<'l> LeagueContext<'l> {
    pub fn new(id: u32, slug: String, team_ids: &'l [u32], reputation: u16) -> Self {
        LeagueContext {
            id,
            slug,
            team_ids,
            reputation,
        }
    }
}
