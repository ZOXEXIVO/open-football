use crate::club::academy::result::ClubAcademyResult;
use crate::club::academy::settings::AcademySettings;
use crate::context::GlobalContext;
use crate::{PlayerCollection, StaffCollection};

#[derive(Debug, Clone)]
pub struct ClubAcademy {
    pub(super) settings: AcademySettings,
    pub players: PlayerCollection,
    pub staff: StaffCollection,
    pub(super) level: u8,
    pub(super) last_production_year: Option<i32>,
    /// Total players graduated to youth teams over the academy's history.
    pub graduates_produced: u16,
}

impl ClubAcademy {
    pub fn new(level: u8) -> Self {
        ClubAcademy {
            settings: AcademySettings::default(),
            players: PlayerCollection::new(Vec::new()),
            staff: StaffCollection::new(Vec::new()),
            level,
            last_production_year: None,
            graduates_produced: 0,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubAcademyResult {
        let players_result = self.players.simulate(ctx.with_player(None));

        // Weekly academy training — the core development driver
        self.train_academy_players(&ctx);

        let produce_result = self.produce_youth_players(ctx.clone());

        for player in produce_result.players {
            self.players.add(player);
        }

        // Ensure academy always has minimum players from settings
        self.ensure_minimum_players(ctx);

        ClubAcademyResult::new(players_result)
    }
}
