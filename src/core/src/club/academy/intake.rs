use crate::academy::result::ProduceYouthPlayersResult;
use crate::context::GlobalContext;
use crate::utils::IntegerUtils;
use crate::{PlayerGenerator, PlayerPositionType};
use chrono::Datelike;
use log::debug;
use super::ClubAcademy;

impl ClubAcademy {
    pub(super) fn produce_youth_players(&mut self, ctx: GlobalContext<'_>) -> ProduceYouthPlayersResult {
        let current_year = ctx.simulation.date.year();
        let current_month = ctx.simulation.date.month();

        if !self.should_produce_players(current_year, current_month) {
            return ProduceYouthPlayersResult::new(Vec::new());
        }

        let club_name = ctx.club.as_ref().map(|c| c.name).unwrap_or("Unknown Club");

        // Youth Recruitment affects intake quantity
        let recruitment_quality = ctx.club_recruitment_quality();
        let players_to_produce = self.calculate_annual_intake(recruitment_quality);

        debug!(
            "academy: {} producing {} youth players (level {}, recruitment={:.2})",
            club_name, players_to_produce, self.level, recruitment_quality
        );

        let mut generated_players = Vec::with_capacity(players_to_produce);

        let country_ctx = ctx.country.as_ref();
        let country_id = country_ctx.map(|c| c.id).unwrap_or(1);
        let people_names = match country_ctx.and_then(|c| c.people_names.as_ref()) {
            Some(names) => names,
            None => return ProduceYouthPlayersResult::new(Vec::new()),
        };

        // Youth Facilities affect intake CA, Academy affects PA,
        // Recruitment affects gem chance
        let youth_facility_quality = ctx.club_facilities_youth();
        let academy_quality = ctx.club_academy_quality();

        for i in 0..players_to_produce {
            let position = self.select_position_for_youth_player(i, players_to_produce);

            let generated_player = PlayerGenerator::generate_with_facilities(
                country_id,
                ctx.simulation.date.date(),
                position,
                self.level,
                people_names,
                youth_facility_quality,
                academy_quality,
                recruitment_quality,
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

    fn calculate_annual_intake(&self, recruitment_quality: f32) -> usize {
        // Base: 5-10 players per year, scaled by academy level
        let (min_intake, max_intake) = match self.level {
            1..=3 => (5, 7),
            4..=6 => (5, 8),
            7..=9 => (6, 9),
            10 => (7, 10),
            _ => (5, 7),
        };

        // Better recruitment network finds more prospects
        let recruitment_bonus = ((recruitment_quality - 0.35) * 6.0).round() as i32;
        let min_adj = (min_intake + recruitment_bonus).max(3);
        let max_adj = (max_intake + recruitment_bonus).max(min_adj + 1);

        IntegerUtils::random(min_adj, max_adj) as usize
    }

    pub(super) fn ensure_minimum_players(&mut self, ctx: GlobalContext<'_>) {
        let min_players = self.settings.players_count_range.start as usize;
        let current_count = self.players.players.len();
        if current_count >= min_players {
            return;
        }

        let needed = min_players - current_count;
        let country_ctx = ctx.country.as_ref();
        let country_id = country_ctx.map(|c| c.id).unwrap_or(1);
        let people_names = match country_ctx.and_then(|c| c.people_names.as_ref()) {
            Some(names) => names,
            None => return,
        };
        let date = ctx.simulation.date.date();

        let youth_quality = ctx.club_facilities_youth();
        let academy_quality = ctx.club_academy_quality();
        let recruitment_quality = ctx.club_recruitment_quality();

        for i in 0..needed {
            let position = self.select_position_for_youth_player(i, needed);
            let player = PlayerGenerator::generate_with_facilities(
                country_id,
                date,
                position,
                self.level,
                people_names,
                youth_quality,
                academy_quality,
                recruitment_quality,
            );
            self.players.add(player);
        }
    }

    pub(super) fn select_position_for_youth_player(
        &self,
        index: usize,
        total_players: usize,
    ) -> PlayerPositionType {
        if total_players >= 4 && index == 0 {
            PlayerPositionType::Goalkeeper
        } else {
            let position_roll = IntegerUtils::random(0, 100);

            match position_roll {
                0..=5 => PlayerPositionType::Goalkeeper,
                6..=20 => match IntegerUtils::random(0, 7) {
                    0 => PlayerPositionType::DefenderLeft,
                    1 => PlayerPositionType::DefenderRight,
                    2 | 3 => PlayerPositionType::DefenderCenter,
                    4 => PlayerPositionType::DefenderCenterLeft,
                    5 => PlayerPositionType::DefenderCenterRight,
                    6 => PlayerPositionType::WingbackLeft,
                    _ => PlayerPositionType::WingbackRight,
                },
                21..=50 => match IntegerUtils::random(0, 5) {
                    0 => PlayerPositionType::DefensiveMidfielder,
                    1 => PlayerPositionType::MidfielderLeft,
                    2 => PlayerPositionType::MidfielderRight,
                    3 => PlayerPositionType::MidfielderCenter,
                    4 => PlayerPositionType::MidfielderCenterLeft,
                    _ => PlayerPositionType::MidfielderCenterRight,
                },
                51..=75 => match IntegerUtils::random(0, 2) {
                    0 => PlayerPositionType::AttackingMidfielderLeft,
                    1 => PlayerPositionType::AttackingMidfielderRight,
                    _ => PlayerPositionType::AttackingMidfielderCenter,
                },
                _ => match IntegerUtils::random(0, 3) {
                    0 => PlayerPositionType::Striker,
                    1 => PlayerPositionType::ForwardLeft,
                    2 => PlayerPositionType::ForwardRight,
                    _ => PlayerPositionType::ForwardCenter,
                },
            }
        }
    }
}
