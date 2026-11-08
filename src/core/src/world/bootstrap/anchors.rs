//! Seeding a player's career anchors from the career he already has.
//!
//! Beside the assurance seeding and in the same shape: everybody once at world
//! construction, then whoever has appeared since, within a tick.

use crate::club::player::mind::{CareerDreamReview, CareerSummit, MindClock};
use crate::club::staff::perception::{AbilityEstimator, PotentialEstimator};
use crate::transfers::pipeline::trace::MarketSwitches;
use crate::{Country, Person, Player, SimulatorData};
use chrono::NaiveDate;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

/// Starts in a recorded season at which he was a regular in that division.
const REGULAR_SEASON_STARTS: u16 = 15;

/// Where a player is, for what his anchors are read against.
struct SeedPlace<'a> {
    club_id: u32,
    club_standing: f32,
    division_reputation: u16,
    country_id: u32,
    country_code: &'a str,
}

impl SeedPlace<'static> {
    const NOWHERE: SeedPlace<'static> = SeedPlace {
        club_id: 0,
        club_standing: 0.0,
        division_reputation: 0,
        country_id: 0,
        country_code: "",
    };
}

impl Player {
    /// His summit from his recorded seasons, read through `division_of`.
    fn summit_from_record(
        &self,
        date: NaiveDate,
        division_of: impl Fn(&str) -> Option<u16>,
    ) -> Option<CareerSummit> {
        let own_level = AbilityEstimator::observable_level(self);
        let mut summit: Option<CareerSummit> = None;
        for item in &self.statistics_history.items {
            if item.statistics.played < REGULAR_SEASON_STARTS {
                continue;
            }
            let Some(division) = division_of(&item.league_slug).filter(|d| *d > 0) else {
                continue;
            };
            let year = item.season.start_year as i32;
            let (Some(began), Some(ended)) = (
                NaiveDate::from_ymd_opt(year, 8, 1),
                NaiveDate::from_ymd_opt(year + 1, 6, 1),
            ) else {
                continue;
            };
            let began = MindClock::day(began.min(date));
            let ended = MindClock::day(ended.min(date));
            match summit.as_mut() {
                Some(best) if division < best.league_reputation => {}
                Some(best) if division == best.league_reputation => {
                    best.reached_on = best.reached_on.min(began);
                    best.last_regular_on = best.last_regular_on.max(ended);
                }
                _ => {
                    summit = Some(CareerSummit {
                        league_reputation: division,
                        club_id: 0,
                        club_standing: item.team_reputation as f32 / 10_000.0,
                        reached_on: began,
                        last_regular_on: ended,
                        level_then: own_level,
                    });
                }
            }
        }
        summit
    }

    fn seed_career_anchors(
        &mut self,
        date: NaiveDate,
        place: &SeedPlace<'_>,
        division_of: impl Fn(&str) -> Option<u16>,
    ) {
        let summit = self.summit_from_record(date, division_of);
        let has_senior_record = self
            .statistics_history
            .items
            .iter()
            .any(|item| item.statistics.played + item.statistics.played_subs > 0);
        let mut situation = self.mind_situation(date, place.country_id, place.country_code);
        situation.club_reputation = place.club_standing;
        situation.division_reputation = place.division_reputation;
        situation.own_level = AbilityEstimator::observable_level(self);
        situation.ceiling = PotentialEstimator::observable_ceiling(self, date);
        self.mind.career.seed_anchors(
            &situation,
            place.club_id,
            summit,
            has_senior_record,
            MindClock::day(date),
        );
    }
}

impl Country {
    /// Everyone on this country's rosters who holds no dream yet and is old
    /// enough for one. Skip-fast: the steady state is one read per player.
    fn seed_rosters(&mut self, date: NaiveDate, division_of: impl Fn(&str) -> Option<u16> + Copy) {
        let leagues: FxHashMap<u32, u16> = self
            .leagues
            .leagues
            .iter()
            .map(|league| (league.id, league.reputation))
            .collect();
        let (country_id, country_code) = (self.id, self.code.clone());
        for club in &mut self.clubs {
            let main = club.teams.main();
            let club_standing = main.map_or(0.0, |t| t.reputation.overall_score());
            let division_reputation = main
                .and_then(|t| t.league_id)
                .and_then(|id| leagues.get(&id).copied())
                .unwrap_or(0);
            let place = SeedPlace {
                club_id: club.id,
                club_standing,
                division_reputation,
                country_id,
                country_code: &country_code,
            };
            for player in club.teams.iter_mut().flat_map(|t| &mut t.players.players) {
                if player.mind.career.anchors.dream.is_none()
                    && player.age(date) >= CareerDreamReview::MIN_AGE
                {
                    player.seed_career_anchors(date, &place, division_of);
                }
            }
        }
    }
}

impl SimulatorData {
    /// At world construction, from each man's record. The pool has no club
    /// to read him against, and what he dreams of does not need one.
    pub(in crate::world) fn seed_player_anchors(&mut self) {
        if MarketSwitches::career_anchors_off() {
            return;
        }
        let date = self.date.date();
        let divisions: FxHashMap<String, u16> = self
            .continents
            .iter()
            .flat_map(|continent| &continent.countries)
            .flat_map(|country| &country.leagues.leagues)
            .map(|league| (league.slug.clone(), league.reputation))
            .collect();
        let division_of = |slug: &str| divisions.get(slug).copied();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| country.seed_rosters(date, division_of));
        for player in &mut self.free_agents {
            if player.age(date) >= CareerDreamReview::MIN_AGE {
                player.seed_career_anchors(date, &SeedPlace::NOWHERE, division_of);
            }
        }
    }

    /// Whoever has appeared since: an academy graduate, a regen, a man who
    /// has just turned old enough. None has a record to read yet.
    pub fn seed_missing_player_anchors(&mut self) {
        if MarketSwitches::career_anchors_off() {
            return;
        }
        let date = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| country.seed_rosters(date, |_| None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::mind::DreamKind;
    use crate::club::player::statistics::PlayerStatistics;
    use crate::league::Season;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, PlayerStatisticsHistoryItem,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn player(age: i64) -> Player {
        PlayerBuilder::new()
            .id(7)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(d(2026, 8, 1) - chrono::Duration::days(age * 365 + 30))
            .country_id(1)
            .attributes(PersonAttributes {
                ambition: 10.0,
                loyalty: 10.0,
                adaptability: 10.0,
                ..Default::default()
            })
            .skills(PlayerSkills::default())
            .player_attributes(PlayerAttributes {
                current_ability: 120,
                potential_ability: 125,
                ..Default::default()
            })
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .build()
            .unwrap()
    }

    fn season(year: u16, league: &str, starts: u16) -> PlayerStatisticsHistoryItem {
        PlayerStatisticsHistoryItem {
            season: Season::new(year),
            team_name: "Club".to_string(),
            team_slug: "club".to_string(),
            team_reputation: 6000,
            league_name: league.to_string(),
            league_slug: league.to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics {
                played: starts,
                ..Default::default()
            },
            seq_id: year as u32,
        }
    }

    fn at(club_id: u32) -> SeedPlace<'static> {
        SeedPlace {
            club_id,
            club_standing: 0.5,
            division_reputation: 6000,
            country_id: 1,
            country_code: "",
        }
    }

    fn divisions(slug: &str) -> Option<u16> {
        match slug {
            "top" => Some(9000),
            "lower" => Some(6000),
            _ => None,
        }
    }

    #[test]
    fn a_28_year_old_with_a_history() {
        let today = d(2026, 8, 1);
        let mut p = player(28);
        for year in 2018..=2022 {
            p.statistics_history.items.push(season(year, "top", 30));
        }
        for year in 2023..=2025 {
            p.statistics_history.items.push(season(year, "lower", 30));
        }
        p.seed_career_anchors(today, &at(9), divisions);

        let anchors = p.mind.career.anchors;
        let summit = anchors.summit.unwrap();
        assert_eq!(summit.league_reputation, 9000);
        assert_eq!(summit.last_regular_on, MindClock::day(d(2023, 6, 1)));
        let held = summit.held(MindClock::day(today), 28, summit.level_then);
        assert!(
            held < 9000,
            "three years gone, it has faded by however long ago that was: {held}"
        );
        assert!(anchors.dream.is_some());
    }

    #[test]
    fn a_season_on_the_bench_is_no_summit() {
        let mut p = player(28);
        p.statistics_history.items.push(season(2024, "top", 3));
        p.statistics_history.items.push(season(2025, "lower", 30));
        p.seed_career_anchors(d(2026, 8, 1), &at(9), divisions);
        assert_eq!(
            p.mind.career.anchors.summit.unwrap().league_reputation,
            6000
        );
    }

    #[test]
    fn a_new_academy_graduate() {
        let today = d(2026, 8, 1);
        let mut p = player(17);
        p.seed_career_anchors(today, &at(9), |_| None);
        let anchors = p.mind.career.anchors;
        assert!(anchors.dream.is_some());
        assert!(
            anchors.summit.is_none(),
            "no summit until he has been a regular"
        );
    }

    #[test]
    fn seeding_writes_no_chapter_the_record_does_not_support() {
        let today = d(2026, 8, 1);
        let mut p = player(28);
        p.statistics_history.items.push(season(2020, "top", 30));
        p.seed_career_anchors(today, &at(9), divisions);
        let chapters: Vec<_> = p.mind.career.anchors.chapters.iter().copied().collect();
        assert_eq!(chapters.len(), 1, "only the dream forming: {chapters:?}");
        assert_eq!(chapters[0].on, MindClock::day(today));
        assert_ne!(
            chapters[0].dream,
            DreamKind::ReachTheTop,
            "ambition 10 at 28"
        );
    }
}
