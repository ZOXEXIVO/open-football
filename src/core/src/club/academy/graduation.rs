use super::ClubAcademy;
use crate::{Person, Player, PlayerClubContract, PlayerStatusType};
use chrono::{Datelike, NaiveDate};
use log::debug;

impl ClubAcademy {
    /// Graduate the best academy players that exceed the pathway
    /// readiness threshold. Up to `count` players will be returned, but
    /// the function refuses to graduate prospects below the bar — the
    /// resident U18 team is better off with fewer well-prepared players
    /// than a roster stuffed with raw teenagers.
    pub fn graduate_to_youth(&mut self, date: NaiveDate, count: usize) -> Vec<Player> {
        if count == 0 {
            return Vec::new();
        }

        let threshold = self.pathway_policy.readiness_threshold;
        let elite_pa = self.tuning.elite_pa_threshold;

        let mut candidates: Vec<(u32, i16, u8, u8)> = self
            .players
            .players
            .iter()
            .filter_map(|p| {
                let readiness = self.pathway_readiness_score(p, date);
                if readiness < threshold {
                    return None;
                }
                Some((
                    p.id,
                    readiness,
                    p.player_attributes.current_ability,
                    p.player_attributes.potential_ability,
                ))
            })
            .collect();

        // Best pathway fit first.
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
        candidates.truncate(count);

        let elite_overshoot_threshold = threshold + 8;
        let _elite_eligible = candidates
            .iter()
            .filter(|(_, r, _, pa)| *r >= elite_overshoot_threshold && *pa >= elite_pa)
            .count();

        let mut graduated = Vec::new();
        for (player_id, _, _, _) in candidates {
            if let Some(mut player) = self.players.take_player(&player_id) {
                let expiration =
                    NaiveDate::from_ymd_opt(date.year() + 3, date.month(), date.day().min(28))
                        .unwrap_or(date);
                let salary = GraduationSalary::for_ca(player.player_attributes.current_ability);
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
        if !graduated.is_empty() {
            self.last_graduation_year = Some(date.year());
        }
        graduated
    }

    /// Number of additional "elite overshoot" graduates the academy is
    /// willing to push into the U18 even after the normal target has
    /// been filled. Returns 0 if no elite prospect exists.
    pub fn elite_overshoot_count(&self, date: NaiveDate) -> usize {
        let threshold = self.pathway_policy.readiness_threshold + 8;
        let elite_pa = self.tuning.elite_pa_threshold;
        self.players
            .players
            .iter()
            .filter(|p| {
                let readiness = self.pathway_readiness_score(p, date);
                readiness >= threshold && p.player_attributes.potential_ability >= elite_pa
            })
            .count()
            .min(2)
    }

    /// Recommended normal graduation count for the resident U18. Takes
    /// the youth-team head-count and applies the soft-target rules:
    ///   target_youth_size = 24, soft_max_youth_size = 30, max 10.
    pub fn recommended_graduates(&self, youth_count: usize) -> usize {
        let target = 24usize;
        let space = target.saturating_sub(youth_count);
        space.min(10)
    }

    /// Hard ceiling on graduates this round when there are elite
    /// prospects on the books. Caps total team size at the soft max
    /// (30).
    pub fn graduation_ceiling(&self, youth_count: usize, normal_graduates: usize, elite_overshoot: usize) -> usize {
        let soft_max = 30usize;
        let proposed = normal_graduates + elite_overshoot;
        let max_room = soft_max.saturating_sub(youth_count);
        proposed.min(max_room)
    }

    /// Players whose 18th birthday means they're no longer eligible for
    /// the academy. Returns the removed players (already detached from
    /// the academy) so the caller can route them through the free-agent
    /// pipeline. Previously these players were silently dropped — we
    /// now surface them so an aged-out 18-year-old can still find a
    /// senior club instead of disappearing from the simulation.
    pub fn release_aged_out_players(&mut self, date: NaiveDate) -> Vec<Player> {
        let to_release: Vec<u32> = self
            .players
            .players
            .iter()
            .filter(|p| p.age(date) >= 18)
            .map(|p| p.id)
            .collect();

        let mut released = Vec::with_capacity(to_release.len());
        for id in to_release {
            if let Some(mut player) = self.players.take_player(&id) {
                // Stamp release state so the global free-agent pool
                // treats the player like any other contract-cleared,
                // Frt-flagged senior. Without this the player would
                // simply vanish from the world.
                player.contract = None;
                player.statuses.add(date, PlayerStatusType::Frt);
                released.push(player);
            }
        }
        released
    }

    /// Backwards-compatible wrapper that returns the count rather than
    /// the released players. Existing callers that don't yet know what
    /// to do with the released roster keep working; the new
    /// `release_aged_out_players` should be preferred.
    pub fn release_aged_out(&mut self, date: NaiveDate) -> usize {
        self.release_aged_out_players(date).len()
    }

}

/// Salary band for a freshly-graduated academy player. The ladder is a
/// CA → annual-wage lookup, wrapped in a struct so the rest of the
/// graduation pipeline always goes through one named entry point
/// rather than a free helper.
pub struct GraduationSalary;

impl GraduationSalary {
    pub fn for_ca(current_ability: u8) -> u32 {
        match current_ability {
            0..=60 => 2_000,
            61..=80 => 5_000,
            81..=100 => 10_000,
            101..=120 => 20_000,
            121..=150 => 40_000,
            _ => 60_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommended_graduates_does_not_overfill_youth() {
        let academy = ClubAcademy::new(8);
        // Already full → 0 graduates.
        assert_eq!(academy.recommended_graduates(24), 0);
        // Half-empty → up to 10.
        assert_eq!(academy.recommended_graduates(12), 10);
        // One slot open → 1.
        assert_eq!(academy.recommended_graduates(23), 1);
    }

    #[test]
    fn graduation_ceiling_respects_soft_max() {
        let academy = ClubAcademy::new(8);
        // soft_max = 30; current = 28; normal = 8, elite = 2 → caps at 2.
        assert_eq!(academy.graduation_ceiling(28, 8, 2), 2);
        // Plenty of room: clean pass-through.
        assert_eq!(academy.graduation_ceiling(10, 5, 1), 6);
    }

    #[test]
    fn aged_out_release_clears_contract_and_stamps_frt() {
        use crate::{PeopleNameGeneratorData, PlayerGenerator, PlayerPositionType};
        let names = PeopleNameGeneratorData {
            first_names: vec!["Old".into()],
            last_names: vec!["Prospect".into()],
            nicknames: vec![],
        };
        let date = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let mut player = PlayerGenerator::generate(
            1,
            date,
            PlayerPositionType::MidfielderCenter,
            10,
            &names,
        );
        // Force the player to be 18 today so the age filter fires.
        player.birth_date = NaiveDate::from_ymd_opt(2008, 6, 1).unwrap();
        assert!(player.age(date) >= 18, "test setup: player must be 18+");
        // Pre-condition: player has a youth contract from the generator.
        assert!(player.contract.is_some(), "test setup expects a contract");

        let mut academy = ClubAcademy::new(8);
        academy.players.add(player);

        let released = academy.release_aged_out_players(date);
        assert_eq!(released.len(), 1, "aged-out player must be released");
        let p = &released[0];
        assert!(p.contract.is_none(), "released player must have no contract");
        assert!(
            p.statuses.get().contains(&PlayerStatusType::Frt),
            "released player must carry Frt status for free-agent discovery"
        );
        // And they're no longer in the academy.
        assert!(
            academy.players.players.is_empty(),
            "academy must drop the released player"
        );
    }
}
