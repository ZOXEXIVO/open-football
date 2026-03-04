use crate::academy::result::ProduceYouthPlayersResult;
use crate::club::academy::result::ClubAcademyResult;
use crate::club::academy::settings::AcademySettings;
use crate::context::GlobalContext;
use crate::utils::IntegerUtils;
use crate::{Person, Player, PlayerClubContract, PlayerCollection, PlayerGenerator, PlayerPositionType, StaffCollection};
use chrono::{Datelike, NaiveDate};
use log::debug;

#[derive(Debug, Clone)]
pub struct ClubAcademy {
    settings: AcademySettings,
    pub players: PlayerCollection,
    _staff: StaffCollection,
    level: u8,
    last_production_year: Option<i32>,
}

impl ClubAcademy {
    pub fn new(level: u8) -> Self {
        ClubAcademy {
            settings: AcademySettings::default(),
            players: PlayerCollection::new(Vec::new()),
            _staff: StaffCollection::new(Vec::new()),
            level,
            last_production_year: None,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubAcademyResult {
        let players_result = self.players.simulate(ctx.with_player(None));

        let produce_result = self.produce_youth_players(ctx.clone());

        for player in produce_result.players {
            self.players.add(player);
        }

        // Ensure academy always has minimum players from settings
        self.ensure_minimum_players(ctx);

        ClubAcademyResult::new(players_result)
    }

    fn ensure_minimum_players(&mut self, ctx: GlobalContext<'_>) {
        let min_players = self.settings.players_count_range.start as usize;
        let current_count = self.players.players.len();
        if current_count >= min_players {
            return;
        }

        let needed = min_players - current_count;
        let country_ctx = ctx.country.as_ref();
        let country_id = country_ctx.map(|c| c.id).unwrap_or(1);
        let people_names = country_ctx.and_then(|c| c.people_names.as_ref());
        let date = ctx.simulation.date.date();

        for i in 0..needed {
            let position = self.select_position_for_youth_player(i, needed);
            let player = PlayerGenerator::generate(
                country_id,
                date,
                position,
                self.level,
                people_names,
            );
            self.players.add(player);
        }
    }

    /// Graduate the best academy players aged 14+ for promotion to U18 team.
    /// Returns up to `count` players sorted by ability (best first).
    pub fn graduate_to_u18(&mut self, date: NaiveDate, count: usize) -> Vec<Player> {
        if count == 0 {
            return Vec::new();
        }

        let mut candidates: Vec<(u32, u8)> = self.players.players.iter()
            .filter(|p| p.age(date) >= 14)
            .map(|p| (p.id, p.player_attributes.current_ability))
            .collect();

        // Best first
        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        candidates.truncate(count);

        let mut graduated = Vec::new();
        for (player_id, _) in candidates {
            if let Some(mut player) = self.players.take_player(&player_id) {
                // Give a youth contract (3 years) — registered with main team, plays in U18
                let expiration = NaiveDate::from_ymd_opt(
                    date.year() + 3,
                    date.month(),
                    date.day().min(28),
                ).unwrap_or(date);
                let salary = graduation_salary(player.player_attributes.current_ability);
                player.contract = Some(PlayerClubContract::new_youth(salary, expiration));

                debug!("academy graduation -> U18: {} (CA={}, age={})",
                    player.full_name, player.player_attributes.current_ability, player.age(date));
                graduated.push(player);
            }
        }

        graduated
    }

    /// Remove academy players who are too old. They simply disappear.
    pub fn release_aged_out(&mut self, date: NaiveDate) -> usize {
        let to_release: Vec<u32> = self.players.players.iter()
            .filter(|p| p.age(date) >= 16)
            .map(|p| p.id)
            .collect();

        let count = to_release.len();
        for id in to_release {
            self.players.take_player(&id);
        }
        count
    }

    fn produce_youth_players(&mut self, ctx: GlobalContext<'_>) -> ProduceYouthPlayersResult {
        let current_year = ctx.simulation.date.year();
        let current_month = ctx.simulation.date.month();

        if !self.should_produce_players(current_year, current_month) {
            return ProduceYouthPlayersResult::new(Vec::new());
        }

        let club_name = ctx.club.as_ref()
            .map(|c| c.name)
            .unwrap_or("Unknown Club");

        // Produce 5-10 players per year based on academy level
        let players_to_produce = self.calculate_annual_intake();

        debug!("academy: {} producing {} youth players (level {})",
               club_name, players_to_produce, self.level);

        let mut generated_players = Vec::with_capacity(players_to_produce);

        let country_ctx = ctx.country.as_ref();
        let country_id = country_ctx.map(|c| c.id).unwrap_or(1);
        let people_names = country_ctx.and_then(|c| c.people_names.as_ref());

        for i in 0..players_to_produce {
            let position = self.select_position_for_youth_player(i, players_to_produce);

            let generated_player = PlayerGenerator::generate(
                country_id,
                ctx.simulation.date.date(),
                position,
                self.level,
                people_names,
            );

            generated_players.push(generated_player);
        }

        self.last_production_year = Some(current_year);

        ProduceYouthPlayersResult::new(generated_players)
    }

    fn should_produce_players(&self, current_year: i32, current_month: u32) -> bool {
        const YOUTH_INTAKE_MONTH: u32 = 7;

        if current_month != YOUTH_INTAKE_MONTH {
            return false;
        }

        match self.last_production_year {
            Some(last_year) if last_year >= current_year => false,
            _ => true,
        }
    }

    fn calculate_annual_intake(&self) -> usize {
        // 5-10 players per year, scaled by academy level
        let (min_intake, max_intake) = match self.level {
            1..=3 => (5, 7),
            4..=6 => (5, 8),
            7..=9 => (6, 9),
            10 => (7, 10),
            _ => (5, 7),
        };

        IntegerUtils::random(min_intake, max_intake) as usize
    }

    fn select_position_for_youth_player(&self, index: usize, total_players: usize) -> PlayerPositionType {
        if total_players >= 4 && index == 0 {
            PlayerPositionType::Goalkeeper
        } else {
            let position_roll = IntegerUtils::random(0, 100);

            match position_roll {
                0..=5 => PlayerPositionType::Goalkeeper,
                6..=20 => {
                    match IntegerUtils::random(0, 5) {
                        0 => PlayerPositionType::DefenderLeft,
                        1 => PlayerPositionType::DefenderRight,
                        2 => PlayerPositionType::DefenderCenter,
                        3 => PlayerPositionType::DefenderCenterLeft,
                        4 => PlayerPositionType::DefenderCenterRight,
                        _ => PlayerPositionType::DefenderCenter,
                    }
                },
                21..=50 => {
                    match IntegerUtils::random(0, 6) {
                        0 => PlayerPositionType::DefensiveMidfielder,
                        1 => PlayerPositionType::MidfielderLeft,
                        2 => PlayerPositionType::MidfielderRight,
                        3 => PlayerPositionType::MidfielderCenter,
                        4 => PlayerPositionType::MidfielderCenterLeft,
                        5 => PlayerPositionType::MidfielderCenterRight,
                        _ => PlayerPositionType::MidfielderCenter,
                    }
                },
                51..=75 => {
                    match IntegerUtils::random(0, 4) {
                        0 => PlayerPositionType::AttackingMidfielderLeft,
                        1 => PlayerPositionType::AttackingMidfielderRight,
                        2 => PlayerPositionType::AttackingMidfielderCenter,
                        3 => PlayerPositionType::WingbackLeft,
                        _ => PlayerPositionType::WingbackRight,
                    }
                },
                _ => {
                    match IntegerUtils::random(0, 3) {
                        0 => PlayerPositionType::Striker,
                        1 => PlayerPositionType::ForwardLeft,
                        2 => PlayerPositionType::ForwardRight,
                        _ => PlayerPositionType::ForwardCenter,
                    }
                }
            }
        }
    }
}

fn graduation_salary(current_ability: u8) -> u32 {
    match current_ability {
        0..=60 => 500,
        61..=80 => 1000,
        81..=100 => 2000,
        101..=120 => 3000,
        121..=150 => 5000,
        _ => 8000,
    }
}
