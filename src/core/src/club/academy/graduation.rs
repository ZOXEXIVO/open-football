use crate::{Person, Player, PlayerClubContract};
use chrono::{Datelike, NaiveDate};
use log::debug;
use super::ClubAcademy;

impl ClubAcademy {
    /// Graduate the best academy players aged 14+ for promotion to the lowest youth team.
    /// Returns up to `count` players sorted by ability (best first).
    pub fn graduate_to_youth(&mut self, date: NaiveDate, count: usize) -> Vec<Player> {
        if count == 0 {
            return Vec::new();
        }

        let mut candidates: Vec<(u32, u8)> = self
            .players
            .players
            .iter()
            .filter(|p| p.age(date) >= 14)
            .map(|p| (p.id, p.player_attributes.current_ability))
            .collect();

        // Best first
        candidates.sort_by(|a, b| b.1.cmp(&a.1));
        candidates.truncate(count);

        let mut graduated = Vec::new();
        for (player_id, _) in candidates {
            if let Some(mut player) = self.players.take_player(&player_id) {
                let expiration = NaiveDate::from_ymd_opt(
                    date.year() + 3,
                    date.month(),
                    date.day().min(28),
                )
                .unwrap_or(date);
                let salary = graduation_salary(player.player_attributes.current_ability);
                player.contract = Some(PlayerClubContract::new_youth(salary, expiration));

                debug!(
                    "academy graduation -> U18: {} (CA={}, age={})",
                    player.full_name,
                    player.player_attributes.current_ability,
                    player.age(date)
                );
                graduated.push(player);
            }
        }

        self.graduates_produced += graduated.len() as u16;
        graduated
    }

    /// Remove academy players who are too old. They simply leave the system.
    pub fn release_aged_out(&mut self, date: NaiveDate) -> usize {
        let to_release: Vec<u32> = self
            .players
            .players
            .iter()
            .filter(|p| p.age(date) >= 16)
            .map(|p| p.id)
            .collect();

        let count = to_release.len();
        for id in to_release {
            self.players.take_player(&id);
        }
        count
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
