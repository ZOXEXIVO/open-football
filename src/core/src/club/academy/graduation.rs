use super::ClubAcademy;
use crate::{Person, Player, PlayerClubContract, PlayerStatusType};
use chrono::{Datelike, NaiveDate};
use log::debug;

impl ClubAcademy {
    /// Graduate up to `count` academy players into the youth-team
    /// pathway. Selection is over the *eligible* pool only — old enough,
    /// healthy, not exhausted (see [`ClubAcademy::is_graduation_eligible`])
    /// — and readiness merely ranks who goes first when capacity is
    /// limited. There is deliberately no quality cut-off: a fit, age-eligible
    /// prospect graduates even at low current ability, because "ready for
    /// youth football" is about the pathway, not first-team quality.
    pub fn graduate_to_youth(&mut self, date: NaiveDate, count: usize) -> Vec<Player> {
        if count == 0 {
            return Vec::new();
        }

        let mut candidates: Vec<(u32, i16, u8, u8, u8)> = self
            .players
            .players
            .iter()
            .filter(|p| self.is_graduation_eligible(p, date))
            .map(|p| {
                (
                    p.id,
                    self.pathway_readiness_score(p, date),
                    p.age(date),
                    p.player_attributes.potential_ability,
                    p.player_attributes.current_ability,
                )
            })
            .collect();

        // Rank: readiness desc, then age desc, then PA desc, then CA desc.
        // Elite prospects naturally sort to the top (high readiness + PA)
        // without excluding ordinary, ready teenagers below them.
        candidates.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then(b.2.cmp(&a.2))
                .then(b.3.cmp(&a.3))
                .then(b.4.cmp(&a.4))
        });
        candidates.truncate(count);

        let mut graduated = Vec::new();
        for (player_id, _, _, _, _) in candidates {
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
                self.is_graduation_eligible(p, date)
                    && p.player_attributes.potential_ability >= elite_pa
                    && self.pathway_readiness_score(p, date) >= threshold
            })
            .count()
            .min(2)
    }

    /// Recommended normal graduation count for the resident youth team.
    ///
    /// Tuned for *annual throughput*, not just topping the squad up: a
    /// healthy academy should ship a steady stream of graduates each
    /// season rather than stalling once the youth team is nominally full.
    ///   * minimum   5  (graduate all eligible if fewer than 5 exist)
    ///   * preferred 8
    ///   * maximum  12
    /// always capped by the room left under the youth soft-max of 30.
    pub fn recommended_graduates(&self, youth_count: usize, eligible_count: usize) -> usize {
        const MIN: usize = 5;
        const PREFERRED: usize = 8;
        const MAX: usize = 12;
        const SOFT_MAX_YOUTH_SIZE: usize = 30;

        let room = SOFT_MAX_YOUTH_SIZE.saturating_sub(youth_count);
        eligible_count
            .min(PREFERRED)
            .max(eligible_count.min(MIN))
            .min(MAX)
            .min(room)
    }

    /// Hard ceiling on graduates this round when there are elite
    /// prospects on the books. Caps total team size at the soft max
    /// (30).
    pub fn graduation_ceiling(
        &self,
        youth_count: usize,
        normal_graduates: usize,
        elite_overshoot: usize,
    ) -> usize {
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

    /// Build a graduation-test prospect with explicit CA/PA, attitude and
    /// condition so the readiness outcome is deterministic. Mirrors the
    /// `academy_prospect` helper in `academy.rs`'s tests.
    fn prospect(
        age: u8,
        ca: u8,
        pa: u8,
        personality: f32,
        condition: i16,
        today: NaiveDate,
    ) -> Player {
        use crate::{PeopleNameGeneratorData, PlayerGenerator, PlayerPositionType};
        let names = PeopleNameGeneratorData {
            first_names: vec!["Test".into()],
            last_names: vec!["Prospect".into()],
            nicknames: vec![],
        };
        let mut player =
            PlayerGenerator::generate(1, today, PlayerPositionType::MidfielderCenter, 10, &names);
        player.player_attributes.current_ability = ca;
        player.player_attributes.potential_ability = pa;
        player.attributes.professionalism = personality;
        player.attributes.ambition = personality;
        player.skills.mental.determination = personality;
        player.skills.mental.work_rate = personality;
        player.player_attributes.condition = condition;
        player.player_attributes.jadedness = 0;
        player.player_attributes.injury_proneness = 5;
        player.birth_date = NaiveDate::from_ymd_opt(today.year() - age as i32, 6, 15).unwrap();
        player
    }

    #[test]
    fn graduation_produces_minimum_five_when_candidates_exist() {
        let date = NaiveDate::from_ymd_opt(2025, 7, 15).unwrap();
        let mut academy = ClubAcademy::new(8);
        // Seven fit, age-eligible teenagers of mostly modest ability.
        for (age, ca, pa) in [
            (15u8, 48u8, 60u8),
            (15, 52, 70),
            (16, 55, 75),
            (16, 60, 90),
            (17, 50, 58),
            (17, 64, 110),
            (16, 47, 62),
        ] {
            academy.players.add(prospect(age, ca, pa, 9.0, 8200, date));
        }
        let eligible = academy.graduation_candidates(date).len();
        assert_eq!(eligible, 7, "all seven teens are eligible");
        // Youth team nearly empty → ample room; throughput floor is 5.
        let count = academy.recommended_graduates(10, eligible);
        assert!(count >= 5, "seasonal floor is 5, got {count}");
        let graduated = academy.graduate_to_youth(date, count);
        assert!(
            graduated.len() >= 5,
            "at least five academy players graduate: {}",
            graduated.len()
        );
    }

    #[test]
    fn graduation_does_not_require_high_ca() {
        let date = NaiveDate::from_ymd_opt(2025, 7, 15).unwrap();
        let mut academy = ClubAcademy::new(8);
        // Deliberately low CA, but fit and old enough.
        academy.players.add(prospect(16, 45, 55, 8.0, 8000, date));
        academy.players.add(prospect(17, 48, 52, 7.0, 8200, date));
        academy.players.add(prospect(15, 50, 60, 9.0, 8100, date));

        let graduated = academy.graduate_to_youth(date, 5);
        assert_eq!(graduated.len(), 3, "low-CA but eligible teens all graduate");
        assert!(
            graduated
                .iter()
                .all(|p| p.player_attributes.current_ability < 60),
            "graduates are genuinely low-CA"
        );
    }

    #[test]
    fn graduation_respects_youth_soft_max() {
        let academy = ClubAcademy::new(8);
        // 8 eligible candidates but the youth team is nearly full (28/30):
        // only 2 slots of room, so recommended + ceiling both cap at 2.
        let normal = academy.recommended_graduates(28, 8);
        assert_eq!(normal, 2, "room under the soft-max caps the normal count");
        let capped = academy.graduation_ceiling(28, normal, 2);
        assert_eq!(capped, 2, "ceiling never exceeds the youth soft-max room");
        // A full youth team → zero graduates regardless of the pool.
        assert_eq!(academy.recommended_graduates(30, 8), 0);
    }

    #[test]
    fn graduate_to_youth_prioritizes_elite_but_includes_normal_players() {
        let date = NaiveDate::from_ymd_opt(2025, 7, 15).unwrap();
        let mut academy = ClubAcademy::new(8);
        let elite = prospect(17, 88, 170, 17.0, 9000, date);
        let elite_id = elite.id;
        academy.players.add(elite);
        // A handful of ordinary, fit, age-eligible teens.
        let mut normal_ids = Vec::new();
        for (age, ca, pa) in [(16u8, 52u8, 64u8), (15, 48, 58), (16, 55, 72)] {
            let p = prospect(age, ca, pa, 8.0, 8200, date);
            normal_ids.push(p.id);
            academy.players.add(p);
        }

        let graduated = academy.graduate_to_youth(date, 3);
        assert_eq!(graduated.len(), 3);
        assert_eq!(graduated[0].id, elite_id, "elite prospect ranks first");
        assert!(
            graduated.iter().any(|p| normal_ids.contains(&p.id)),
            "ordinary teens are still included, not excluded by the elite"
        );
    }

    #[test]
    fn recommended_graduates_targets_seasonal_throughput() {
        let academy = ClubAcademy::new(8);
        // Plenty eligible, empty youth → preferred 8.
        assert_eq!(academy.recommended_graduates(0, 20), 8);
        // Fewer than the floor eligible → graduate all of them.
        assert_eq!(academy.recommended_graduates(0, 3), 3);
        // Enough to clear the floor → at least the minimum.
        assert_eq!(academy.recommended_graduates(0, 6), 6);
        // Room under the soft-max caps the count.
        assert_eq!(academy.recommended_graduates(27, 20), 3);
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
        let mut player =
            PlayerGenerator::generate(1, date, PlayerPositionType::MidfielderCenter, 10, &names);
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
        assert!(
            p.contract.is_none(),
            "released player must have no contract"
        );
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
