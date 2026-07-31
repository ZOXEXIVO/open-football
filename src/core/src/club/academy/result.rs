use crate::league::result::LeagueProcessAccess;
use crate::{Player, PlayerCollectionResult};

pub struct ClubAcademyResult {
    pub players: PlayerCollectionResult,
    /// How many boys the academy took in today, on the one day a year
    /// it takes anybody in.
    ///
    /// Carried out of the academy purely so the club can write intake
    /// day into its own diary: the recruits are added to the academy
    /// roster inside `simulate`, where there is no `&mut Club` to
    /// record an affair on, and by the following Monday the only trace
    /// left is a squad list that is longer than it was.
    pub intake: u16,
    /// The intake the recruiter itself rated exceptional. Judged from
    /// its own candidate scores rather than from the players' hidden
    /// ceilings, which nothing outside the engine is allowed to read.
    pub golden_intake: bool,
}

impl ClubAcademyResult {
    pub fn new(players: PlayerCollectionResult) -> Self {
        ClubAcademyResult {
            players,
            intake: 0,
            golden_intake: false,
        }
    }

    pub fn with_intake(mut self, intake: u16, golden: bool) -> Self {
        self.intake = intake;
        self.golden_intake = golden;
        self
    }

    pub fn process<D: LeagueProcessAccess>(&self, _: &mut D) {}
}

pub struct ProduceYouthPlayersResult {
    pub players: Vec<Player>,
    /// The recruitment staff rated this class exceptional. Their own
    /// verdict, not a reading of anybody's ceiling.
    pub golden: bool,
}

impl ProduceYouthPlayersResult {
    pub fn new(players: Vec<Player>) -> Self {
        ProduceYouthPlayersResult {
            players,
            golden: false,
        }
    }

    pub fn rated(mut self, golden: bool) -> Self {
        self.golden = golden;
        self
    }

    pub fn process<D: LeagueProcessAccess>(&self, _: &mut D) {}
}
