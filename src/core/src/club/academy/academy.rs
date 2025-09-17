use crate::academy::result::ProduceYouthPlayersResult;
use crate::club::academy::result::ClubAcademyResult;
use crate::club::academy::settings::AcademySettings;
use crate::context::GlobalContext;
use crate::utils::IntegerUtils;
use crate::{PlayerCollection, StaffCollection, PlayerGenerator, PlayerPositionType, Player, Person};
use chrono::Datelike;
use log::debug;

#[derive(Debug)]
pub struct ClubAcademy {
    settings: AcademySettings,
    players: PlayerCollection,
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
        // Simulate existing academy players
        let players_result = self.players.simulate(ctx.with_player(None));

        // Produce new youth players
        let produce_result = self.produce_youth_players(ctx);

        // Add newly produced players to the academy
        for player in produce_result.players {
            debug!("🎓 academy: adding new youth player: {}", player.fullname());
            self.players.add(player);
        }

        ClubAcademyResult::new(players_result)
    }

    fn produce_youth_players(&mut self, ctx: GlobalContext<'_>) -> ProduceYouthPlayersResult {
        let current_year = ctx.simulation.date.year();
        let current_month = ctx.simulation.date.month();

        // Check if we should produce players this year
        if !self.should_produce_players(current_year, current_month) {
            return ProduceYouthPlayersResult::new(Vec::new());
        }

        let club_name = ctx.club.as_ref()
            .map(|c| c.name)
            .unwrap_or("Unknown Club");

        debug!("🎓 academy: {} starting yearly youth intake", club_name);

        // Determine how many players to produce based on academy level and current squad size
        let current_size = self.players.players.len();
        let min_required = self.settings.players_count_range.start as usize;
        let max_allowed = self.settings.players_count_range.end as usize;

        let mut players_to_produce = 0;

        // Always produce some players if below minimum
        if current_size < min_required {
            players_to_produce = min_required - current_size;
            debug!("⚠️ academy: {} below minimum capacity ({}/{}), producing {} players",
                   club_name, current_size, min_required, players_to_produce);
        }

        // Annual intake based on academy level (even if at capacity)
        let annual_intake = self.calculate_annual_intake();

        // Only add annual intake if we won't exceed maximum
        if current_size + players_to_produce + annual_intake <= max_allowed {
            players_to_produce += annual_intake;
            debug!("📊 academy: {} annual intake of {} players (level {})",
                   club_name, annual_intake, self.level);
        } else {
            let space_available = max_allowed.saturating_sub(current_size);
            players_to_produce = space_available.min(players_to_produce + annual_intake);
            debug!("⚠️ academy: {} limited to {} new players due to capacity ({}/{})",
                   club_name, players_to_produce, current_size, max_allowed);
        }

        if players_to_produce == 0 {
            debug!("ℹ️ academy: {} at full capacity, no new players produced", club_name);
            return ProduceYouthPlayersResult::new(Vec::new());
        }

        // Generate the youth players
        let mut generated_players = Vec::with_capacity(players_to_produce);

        // Get country_id from club context if available
        let country_id = ctx.country.as_ref()
            .map(|c| c.id)
            .unwrap_or(1); // Default to 1 if not available

        for i in 0..players_to_produce {
            let position = self.select_position_for_youth_player(i, players_to_produce);

            let generated_player = PlayerGenerator::generate(
                country_id,
                ctx.simulation.date.date(),
                position,
                self.level, // Pass academy level to influence player quality
            );

            debug!("👤 academy: {} generated youth player: {} ({}, age {})",
                   club_name,
                   generated_player.full_name,
                   position.get_short_name(),
                   generated_player.age(ctx.simulation.date.date()));

            generated_players.push(generated_player);
        }

        // Update last production year
        self.last_production_year = Some(current_year);

        debug!("✅ academy: {} completed youth intake with {} new players",
               club_name, generated_players.len());

        ProduceYouthPlayersResult::new(generated_players)
    }

    fn should_produce_players(&self, current_year: i32, current_month: u32) -> bool {
        // Produce players once per year, typically in July (pre-season)
        const YOUTH_INTAKE_MONTH: u32 = 7;

        if current_month != YOUTH_INTAKE_MONTH {
            return false;
        }

        // Check if we've already produced players this year
        match self.last_production_year {
            Some(last_year) if last_year >= current_year => false,
            _ => true,
        }
    }

    fn calculate_annual_intake(&self) -> usize {
        // Academy level determines quality and quantity of youth intake
        // Level 1-3: Poor academy (2-4 players)
        // Level 4-6: Average academy (3-6 players)
        // Level 7-9: Good academy (4-8 players)
        // Level 10: Excellent academy (5-10 players)

        let (min_intake, max_intake) = match self.level {
            1..=3 => (2, 4),
            4..=6 => (3, 6),
            7..=9 => (4, 8),
            10 => (5, 10),
            _ => (2, 4), // Default for unexpected values
        };

        IntegerUtils::random(min_intake, max_intake) as usize
    }

    fn select_position_for_youth_player(&self, index: usize, total_players: usize) -> PlayerPositionType {
        // Distribute positions somewhat realistically
        // Ensure at least 1 GK if producing 4+ players
        // Otherwise random distribution favoring outfield players

        if total_players >= 4 && index == 0 {
            // First player is goalkeeper when producing 4+ players
            PlayerPositionType::Goalkeeper
        } else {
            // Random distribution for other positions
            let position_roll = IntegerUtils::random(0, 100);

            match position_roll {
                0..=5 => PlayerPositionType::Goalkeeper, // 5% chance
                6..=20 => {
                    // Defenders 15% chance
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
                    // Midfielders 30% chance
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
                    // Attacking midfielders/wingers 25% chance
                    match IntegerUtils::random(0, 4) {
                        0 => PlayerPositionType::AttackingMidfielderLeft,
                        1 => PlayerPositionType::AttackingMidfielderRight,
                        2 => PlayerPositionType::AttackingMidfielderCenter,
                        3 => PlayerPositionType::WingbackLeft,
                        _ => PlayerPositionType::WingbackRight,
                    }
                },
                _ => {
                    // Forwards 25% chance
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

    pub fn graduate_player(&mut self, player_id: u32) -> Option<Player> {
        // Remove a player from academy (e.g., promoted to first team)
        self.players.take_player(&player_id)
    }

    pub fn release_player(&mut self, player_id: u32) -> Option<Player> {
        // Release a player from academy
        let player = self.players.take_player(&player_id);

        if let Some(ref p) = player {
            debug!("👋 academy: releasing player {}", p.full_name);
        }

        player
    }

    pub fn get_capacity_status(&self) -> (usize, usize, usize) {
        let current = self.players.players.len();
        let min = self.settings.players_count_range.start as usize;
        let max = self.settings.players_count_range.end as usize;

        (current, min, max)
    }
}