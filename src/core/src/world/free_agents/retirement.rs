use crate::club::staff::perception::PotentialEstimator;
use crate::country::result::transfers::free::pricing::FreeAgentMarketCalculator;
use crate::utils::IntegerUtils;
use crate::world::SimulatorData;
use crate::{Person, RetirementReason};
use chrono::NaiveDate;
use rayon::prelude::*;

impl SimulatorData {
    /// Monthly retirement pass over the global free-agent pool. Anyone
    /// 12+ months without a club rolls retirement at a probability that
    /// climbs with age, low quality, and time spent unemployed; high
    /// world-rep players resist longer (they're still names, clubs come
    /// looking).
    ///
    /// On top of the probabilistic roll there is a deterministic hard
    /// bound (`deterministic_retirement_months`, 24–48 months by age /
    /// CA / observable ceiling / world rep): a player whose monthly
    /// rolls keep missing still resolves instead of haunting the pool
    /// for half a decade. Young players with visible growth room get
    /// the longest leash; old low-quality journeymen the shortest.
    ///
    /// Gated by the caller on `today.day() == 1`. The internal gate on
    /// `free_since` ≥ 12 months means a fresh database free agent
    /// (seeded `free_since = today - 30d`) is automatically skipped.
    pub fn process_free_agent_retirements(&mut self, date: NaiveDate) {
        // Decision phase is per-player independent — run the prob roll
        // in parallel. Mutation (the `RetirementConsidering` emit plus
        // swap_remove + push into retired_players) stays serial below
        // because it requires `&mut self`. Each outcome carries
        // `(index, will_retire, months_without_club)` so the serial pass
        // can both surface late-career considering moods and retire the
        // ones whose roll came up.
        let outcomes: Vec<(usize, bool, u16)> = self
            .free_agents
            .par_iter()
            .enumerate()
            .filter_map(|(idx, player)| {
                let state = player.free_agent_state()?;
                let days_free = (date - state.free_since).num_days();
                if days_free < 365 {
                    return None;
                }
                let months_without_club = (days_free / 30).max(0) as u16;
                let age = player.age(date);
                let ca = player.player_attributes.current_ability;
                let world_rep = player.player_attributes.world_reputation;
                // Observable ceiling, not hidden biological PA — the
                // retirement judgement reads the same market-visible
                // promise every club decision does.
                let ceiling = PotentialEstimator::observable_ceiling(player, date);
                let hard_bound = FreeAgentMarketCalculator::deterministic_retirement_months(
                    age, ca, ceiling, world_rep,
                );
                if (months_without_club as u32) >= hard_bound {
                    return Some((idx, true, months_without_club));
                }
                let months_after_12 = ((days_free - 365) / 30).max(0) as u32;
                let prob = FreeAgentMarketCalculator::retirement_probability_per_month(
                    months_after_12,
                    age,
                    ca,
                    world_rep,
                );
                if prob <= 0.0 {
                    return None;
                }
                let roll = IntegerUtils::random(1, 1000) as f32 / 1000.0;
                Some((idx, roll < prob, months_without_club))
            })
            .collect();

        // Considering pass first — it only mutates happiness, never the
        // pool's structure, so indices remain valid. Players who are
        // about to retire this tick are skipped (they get the
        // announcement, not the lead-up).
        for &(idx, will_retire, months) in &outcomes {
            if will_retire {
                continue;
            }
            if let Some(player) = self.free_agents.get_mut(idx) {
                player.consider_retirement_as_free_agent(date, months);
            }
        }

        // Retirement pass. Reverse order so swap_remove against earlier
        // indexes doesn't disturb later ones.
        let mut to_retire: Vec<usize> = outcomes
            .iter()
            .filter(|(_, will_retire, _)| *will_retire)
            .map(|(idx, _, _)| *idx)
            .collect();
        to_retire.sort_unstable_by(|a, b| b.cmp(a));
        // Monthly diagnostics flow counter — recorded before the drain so
        // `log_pool_stats` can report how many players left the pool to
        // retirement this month.
        self.free_agent_flow.retired_from_pool = self
            .free_agent_flow
            .retired_from_pool
            .saturating_add(to_retire.len() as u32);
        for idx in to_retire {
            let mut player = self.free_agents.swap_remove(idx);
            // A still-renowned name bows out with a planned farewell;
            // everyone else is retiring because the offers dried up.
            let reason = if player.player_attributes.world_reputation >= 7000 {
                RetirementReason::PlannedFarewell
            } else {
                RetirementReason::LongFreeAgency
            };
            player.announce_retirement(date, reason);
            let country_id = player.country_id;
            if let Some(country) = self.country_mut(country_id) {
                country.retired_players.push(player);
            }
            // Else: nationality country isn't loaded — drop silently.
            // The player is gone from the pool either way.
        }
    }
}

#[cfg(test)]
mod free_agent_retirement_tests {
    //! Deterministic coverage for the free-agent retirement pass: the
    //! hard upper bound must resolve long sits without RNG, and young
    //! players with rep / growth room must not be swept prematurely.

    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::transfer::ReleaseContext;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::LeagueCollection;
    use crate::shared::fullname::FullName;
    use crate::{
        Country, PersonAttributes, Player, PlayerAttributes, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, PlayerSquadStatus,
    };
    use chrono::Duration;

    struct RetirementFixtures;

    impl RetirementFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn pool_player(
            id: u32,
            age_years: i64,
            ca: u8,
            world_reputation: i16,
            free_since: NaiveDate,
            today: NaiveDate,
        ) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = ca;
            attrs.world_reputation = world_reputation;
            let birth = today - Duration::days(age_years * 365 + 30);
            let mut player = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Pool".to_string(), format!("P{id}")))
                .birth_date(birth)
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 16,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap();
            player.enter_free_agent_market(ReleaseContext {
                date: free_since,
                last_club_id: Some(10),
                last_country_id: Some(1),
                last_country_reputation: 3000,
                last_league_reputation: 2500,
                last_club_reputation_score: 0.3,
                last_salary: 40_000,
                last_squad_status: PlayerSquadStatus::FirstTeamSquadRotation,
            });
            player
        }

        fn simulator(today: NaiveDate, free_agents: Vec<Player>) -> SimulatorData {
            let country = Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("England".to_string())
                .continent_id(1)
                .reputation(5000)
                .leagues(LeagueCollection::new(Vec::new()))
                .clubs(Vec::new())
                .build()
                .unwrap();
            let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
            let mut data = SimulatorData::new(
                today.and_hms_opt(12, 0, 0).unwrap(),
                vec![continent],
                GlobalCompetitions::new(Vec::new()),
            );
            data.free_agents = free_agents;
            data
        }
    }

    #[test]
    fn old_low_quality_free_agent_retires_deterministically_after_hard_bound() {
        // 36yo CA-50 journeyman, 40 months without a club: well past
        // the 24-month deterministic bound for the old/low-CA cohort.
        // No RNG involved -- the hard bound fires before any roll.
        let today = RetirementFixtures::d(2026, 6, 1);
        let free_since = today - Duration::days(40 * 30);
        let player = RetirementFixtures::pool_player(900, 36, 50, 0, free_since, today);
        let mut data = RetirementFixtures::simulator(today, vec![player]);

        data.process_free_agent_retirements(today);

        assert!(
            data.free_agents.is_empty(),
            "hard-bound retirement must remove the player from the pool"
        );
        let country = data.country(1).unwrap();
        assert!(
            country.retired_players.iter().any(|p| p.id == 900),
            "retired player must land in his nationality country"
        );
        assert!(country.retired_players[0].retired);
    }

    #[test]
    fn young_free_agent_with_standing_is_not_prematurely_retired() {
        // 21yo, 13 months free, decent world rep: the probabilistic
        // curve nets out to zero (rep offsets the small time base) and
        // every deterministic bound is years away -- the player must
        // still be in the pool. Fully deterministic: prob <= 0 means
        // no roll happens at all.
        let today = RetirementFixtures::d(2026, 6, 1);
        let free_since = today - Duration::days(395);
        let player = RetirementFixtures::pool_player(901, 21, 70, 5000, free_since, today);
        let mut data = RetirementFixtures::simulator(today, vec![player]);

        data.process_free_agent_retirements(today);

        assert_eq!(
            data.free_agents.len(),
            1,
            "young free agent must not be swept at 13 months"
        );
        assert!(!data.free_agents[0].retired);
    }
}
