use crate::generators::StaffGenerator;
use core::transfers::ScoutingRegion;
use core::utils::IntegerUtils;
use core::{Staff, StaffPosition, TeamType};

use super::DatabaseGenerator;

impl DatabaseGenerator {
    pub(super) fn generate_staffs(
        staff_generator: &StaffGenerator,
        seed: &ScoutMarketSeed<'_>,
        team_reputation: u16,
        team_type: &TeamType,
    ) -> Vec<Staff> {
        let country_id = seed.country_id;
        let mut staffs = Vec::with_capacity(30);

        if *team_type == TeamType::Main {
            // A main team is ALWAYS born with exactly one permanent manager.
            // From here the board / manager-market lifecycle owns the seat
            // (renewals, sackings, caretakers, appointments), but it must
            // START filled — otherwise a club can spend its whole existence
            // invisible to the manager market with nobody in the dugout.
            staffs.push(staff_generator.generate(
                country_id,
                StaffPosition::Manager,
                team_reputation,
            ));

            // Only main team gets directors and scouts
            staffs.push(staff_generator.generate(
                country_id,
                StaffPosition::DirectorOfFootball,
                team_reputation,
            ));
            staffs.push(staff_generator.generate(
                country_id,
                StaffPosition::Director,
                team_reputation,
            ));

            // Scouts get the MARKETS their club's country actually shops in
            // (see `assign_scout_markets`); the wider the club, the more of
            // them, and the better the odds of a hired specialist.
            let mut chief_scout =
                staff_generator.generate(country_id, StaffPosition::ChiefScout, team_reputation);
            Self::assign_scout_markets(&mut chief_scout, seed, team_reputation);
            staffs.push(chief_scout);

            // Scale scout count by reputation — real elite clubs run 8+ scouts,
            // League-level clubs 2-4, amateurs 1-2.
            let scout_count = if team_reputation >= 7000 {
                IntegerUtils::random(6, 8) as usize
            } else if team_reputation >= 5000 {
                IntegerUtils::random(4, 6) as usize
            } else if team_reputation >= 3000 {
                IntegerUtils::random(2, 4) as usize
            } else {
                IntegerUtils::random(1, 2) as usize
            };

            for _ in 0..scout_count {
                let mut scout =
                    staff_generator.generate(country_id, StaffPosition::Scout, team_reputation);
                Self::assign_scout_markets(&mut scout, seed, team_reputation);
                staffs.push(scout);
            }

            // Reputation-scaled coaching / medical / analytics backroom.
            Self::push_main_backroom(staff_generator, country_id, team_reputation, &mut staffs);
        } else {
            // Reserve / youth teams keep a lean support backroom and never
            // their own manager seat — the club's head coach runs the
            // football side across the whole club.
            Self::push_support_backroom(staff_generator, country_id, team_reputation, &mut staffs);
        }

        staffs
    }

    /// Coaching, medical and analytics depth for a main team, scaled by
    /// reputation. Elite clubs field a full modern backroom (assistant,
    /// generalist coaches, GK + fitness specialists, a head physio leading
    /// the medical room, a data analyst and head of recruitment); smaller
    /// clubs get a credible-but-thin core — an assistant, a coach and a
    /// physio — but never zero operational staff.
    fn push_main_backroom(
        staff_generator: &StaffGenerator,
        country_id: u32,
        team_reputation: u16,
        staffs: &mut Vec<Staff>,
    ) {
        let hire = |position| staff_generator.generate(country_id, position, team_reputation);

        // Every main team has an assistant manager.
        staffs.push(hire(StaffPosition::AssistantManager));

        let (coaches, physios) = if team_reputation >= 7000 {
            (3, 3)
        } else if team_reputation >= 5000 {
            (3, 2)
        } else if team_reputation >= 3000 {
            (2, 2)
        } else {
            (1, 1)
        };

        for _ in 0..coaches {
            staffs.push(hire(StaffPosition::Coach));
        }

        // The goalkeeping coach is not a luxury. Every professional club
        // employs one, down to part-time sides, because keepers cannot be
        // coached inside an outfield session — and because somebody has to
        // run the one position the whole club shares a single shirt at.
        // He owns the keeper room across every squad: see
        // `core::club::staff::goalkeeping`.
        staffs.push(hire(StaffPosition::GoalkeeperCoach));

        // Specialist coaches appear as the club can afford them.
        if team_reputation >= 5000 {
            staffs.push(hire(StaffPosition::FitnessCoach));
        }

        // A head physio leads the medical room at well-funded clubs.
        if team_reputation >= 5000 {
            staffs.push(hire(StaffPosition::HeadOfPhysio));
        }
        for _ in 0..physios {
            staffs.push(hire(StaffPosition::Physio));
        }

        // Modern analytics / recruitment leadership at the very top end.
        if team_reputation >= 7000 {
            staffs.push(hire(StaffPosition::DataAnalyst));
            staffs.push(hire(StaffPosition::HeadOfRecruitment));
        }
    }

    /// Lean support backroom for reserve / youth teams: an assistant, a few
    /// coaches and physios. Mirrors the historical flat allocation so youth
    /// development and medical cover are unchanged, but without minting a
    /// second manager seat inside the club.
    fn push_support_backroom(
        staff_generator: &StaffGenerator,
        country_id: u32,
        team_reputation: u16,
        staffs: &mut Vec<Staff>,
    ) {
        let hire = |position| staff_generator.generate(country_id, position, team_reputation);
        staffs.push(hire(StaffPosition::AssistantManager));
        staffs.push(hire(StaffPosition::Coach));
        staffs.push(hire(StaffPosition::Coach));
        staffs.push(hire(StaffPosition::Coach));
        staffs.push(hire(StaffPosition::Physio));
        staffs.push(hire(StaffPosition::Physio));
        staffs.push(hire(StaffPosition::Physio));

        // A well-funded club runs a goalkeeping coach inside the academy
        // too. It is the difference between a young keeper being trained
        // and a young keeper being supervised — the specialist takes the
        // goalkeeping sessions for this squad rather than the generalist
        // who is running the outfield group at the same time.
        if team_reputation >= 2500 {
            staffs.push(hire(StaffPosition::GoalkeeperCoach));
        }
    }

    /// Give a scout the MARKETS he works — countries first, regions derived.
    ///
    /// A scouting department covers its home market and a handful of foreign
    /// ones, and which ones is not a matter of geography: it is the club's
    /// country's own import habits, because that is who its agents, its
    /// friendly clubs and its previous scouts already know. So the foreign
    /// picks are sampled from the country's `import` card — Galatasaray's
    /// scouts start knowing Brazil and Nigeria, Spartak's start knowing
    /// Brazil and Serbia — and the regions fall out of the countries rather
    /// than the other way round.
    ///
    /// A minority of scouts at well-funded clubs are foreign SPECIALISTS:
    /// the man the club went and found because he knows one market, starting
    /// with real familiarity there. That is the channel a corridor opens
    /// through, at generation and at runtime alike.
    ///
    /// A country with no card (or one whose card names nothing this database
    /// carries) falls back to the region corridor table, which is what the
    /// generator did before the cards existed.
    pub(super) fn assign_scout_markets(
        staff: &mut Staff,
        seed: &ScoutMarketSeed<'_>,
        team_reputation: u16,
    ) {
        use core::RegionFamiliarity;

        let club_region = seed.club_region();

        let specialist_chance = if team_reputation >= 7000 {
            40
        } else if team_reputation >= 5000 {
            25
        } else if team_reputation >= 3000 {
            10
        } else {
            0
        };

        // How many foreign markets this club's network covers. A giant runs
        // a department that watches half a dozen; a minnow watches its own
        // division and maybe a neighbour.
        let foreign_count = if team_reputation >= 7000 {
            6
        } else if team_reputation >= 5000 {
            4
        } else if team_reputation >= 3000 {
            2
        } else {
            1
        };

        let knowledge = &mut staff.staff_attributes.knowledge;
        // Home market first: he lives there.
        knowledge.seed_country(seed.country_id, 100);
        knowledge.known_regions.clear();
        knowledge.known_regions.push(club_region);

        let is_specialist =
            specialist_chance > 0 && IntegerUtils::random(0, 100) < specialist_chance;

        if seed.import_priors.is_empty() {
            // No card for this country. Fall back to the old region sampling
            // so the club still has reach, but seed no foreign COUNTRY — a
            // network with no named market is reach without depth, which is
            // exactly what "we have no idea where this league shops" means.
            let corridors = club_region.transfer_corridors();
            let total: u32 = corridors.iter().map(|(_, w)| *w as u32).sum();
            if total > 0 {
                for _ in 0..foreign_count {
                    let roll = IntegerUtils::random(0, total as i32) as u32;
                    let mut acc = 0u32;
                    for (region, weight) in corridors {
                        acc += *weight as u32;
                        if roll < acc {
                            if !knowledge.known_regions.contains(region) {
                                knowledge.known_regions.push(*region);
                            }
                            break;
                        }
                    }
                }
            }
            return;
        }

        let mut sampled: Vec<u32> = Vec::with_capacity(foreign_count);
        for _ in 0..foreign_count {
            let Some(country) = Self::pick_weighted_country(seed.import_priors, &sampled) else {
                break;
            };
            sampled.push(country);
        }

        let mut specialist_region = None;
        for (index, source_country) in sampled.iter().enumerate() {
            // The specialist's first market is the one he was hired for, and
            // he arrives already knowing it. The rest are markets the
            // department watches — real coverage, no depth yet.
            let level = if is_specialist && index == 0 {
                IntegerUtils::random(40, 61) as u8
            } else {
                IntegerUtils::random(8, 25) as u8
            };
            knowledge.seed_country(*source_country, level);
            if let Some(region) = seed.region_of(*source_country) {
                if !knowledge.known_regions.contains(&region) {
                    knowledge.known_regions.push(region);
                }
                if is_specialist && index == 0 {
                    specialist_region = Some(region);
                }
            }
        }

        // The specialist's region familiarity mirrors his seeded market, so
        // the report-accuracy model reads the same career the country list
        // describes rather than a second, independently-sampled one.
        if let Some(region) = specialist_region {
            knowledge.region_familiarity = vec![RegionFamiliarity {
                region,
                level: 40,
                days_scouted: 400,
            }];
        }
    }

    /// Weighted draw from the country's import priors, skipping markets
    /// already sampled. `None` when the card names nothing usable.
    fn pick_weighted_country(priors: &[ScoutMarketPrior], taken: &[u32]) -> Option<u32> {
        let total: f32 = priors
            .iter()
            .filter(|p| !taken.contains(&p.country_id))
            .map(|p| p.weight)
            .sum();
        if total <= 0.0 {
            return None;
        }
        let roll = IntegerUtils::random(0, 10_000) as f32 / 10_000.0 * total;
        let mut acc = 0.0;
        for prior in priors.iter().filter(|p| !taken.contains(&p.country_id)) {
            acc += prior.weight;
            if roll < acc {
                return Some(prior.country_id);
            }
        }
        priors
            .iter()
            .find(|p| !taken.contains(&p.country_id))
            .map(|p| p.country_id)
    }
}

/// One market a country's clubs import from, as the scout seeder reads it.
#[derive(Debug, Clone, Copy)]
pub(super) struct ScoutMarketPrior {
    pub country_id: u32,
    /// Normalised import weight from the country's card, 0..1.
    pub weight: f32,
    pub region: ScoutingRegion,
}

/// Everything the scout seeder needs about the club's own country. Built
/// once per country in `generate_countries` and shared by every club and
/// every scout in it.
pub(super) struct ScoutMarketSeed<'a> {
    pub country_id: u32,
    pub continent_id: u32,
    pub country_code: &'a str,
    /// The country card's import list, resolved to ids and regions. Empty
    /// when the data does not name this country.
    pub import_priors: &'a [ScoutMarketPrior],
}

impl ScoutMarketSeed<'_> {
    fn club_region(&self) -> ScoutingRegion {
        ScoutingRegion::from_country(self.continent_id, self.country_code)
    }

    fn region_of(&self, country_id: u32) -> Option<ScoutingRegion> {
        self.import_priors
            .iter()
            .find(|p| p.country_id == country_id)
            .map(|p| p.region)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::StaffGenerator;
    use core::PeopleNameGeneratorData;

    fn make_generator() -> StaffGenerator {
        StaffGenerator::with_people_names(&PeopleNameGeneratorData {
            first_names: vec!["Alex".into(), "Sam".into()],
            last_names: vec!["Smith".into(), "Jones".into()],
            nicknames: vec![],
        })
    }

    /// A scout seed for the tests: England, no import priors, so the
    /// generator falls back to the region corridor table exactly as it did
    /// before the country cards existed.
    fn seed() -> ScoutMarketSeed<'static> {
        ScoutMarketSeed {
            country_id: 1,
            continent_id: 1,
            country_code: "EN",
            import_priors: &[],
        }
    }

    fn count_position(staffs: &[Staff], position: StaffPosition) -> usize {
        staffs
            .iter()
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(|c| c.position == position)
                    .unwrap_or(false)
            })
            .count()
    }

    #[test]
    fn main_team_has_exactly_one_manager_across_reputations() {
        let generator = make_generator();
        for rep in [800u16, 3500, 6000, 8000] {
            let staffs =
                DatabaseGenerator::generate_staffs(&generator, &seed(), rep, &TeamType::Main);
            assert_eq!(
                count_position(&staffs, StaffPosition::Manager),
                1,
                "exactly one permanent manager expected at rep {rep}"
            );
            // Even a tiny club is never left with zero operational staff.
            assert!(
                staffs.len() >= 3,
                "main team at rep {rep} too thin: {} staff",
                staffs.len()
            );
        }
    }

    #[test]
    fn every_main_team_employs_a_goalkeeping_coach() {
        let generator = make_generator();
        for rep in [400u16, 800, 2900, 3500, 6000, 8000] {
            let staffs =
                DatabaseGenerator::generate_staffs(&generator, &seed(), rep, &TeamType::Main);
            assert_eq!(
                count_position(&staffs, StaffPosition::GoalkeeperCoach),
                1,
                "the keeper room needs somebody to run it at rep {rep}"
            );
            let coach = staffs
                .iter()
                .find(|s| {
                    s.contract
                        .as_ref()
                        .map(|c| c.position == StaffPosition::GoalkeeperCoach)
                        .unwrap_or(false)
                })
                .unwrap();
            let gk = &coach.staff_attributes.goalkeeping;
            let outfield = &coach.staff_attributes.coaching;
            assert!(
                gk.shot_stopping >= outfield.attacking.min(outfield.defending)
                    || gk.shot_stopping > 0,
                "a goalkeeping coach should actually coach goalkeepers at rep {rep}"
            );
        }
    }

    #[test]
    fn a_well_funded_academy_gets_its_own_goalkeeping_coach() {
        let generator = make_generator();
        let big = DatabaseGenerator::generate_staffs(&generator, &seed(), 6000, &TeamType::U18);
        let small = DatabaseGenerator::generate_staffs(&generator, &seed(), 900, &TeamType::U18);
        assert_eq!(count_position(&big, StaffPosition::GoalkeeperCoach), 1);
        assert_eq!(count_position(&small, StaffPosition::GoalkeeperCoach), 0);
    }

    #[test]
    fn youth_team_gets_no_manager_seat() {
        let generator = make_generator();
        let staffs = DatabaseGenerator::generate_staffs(&generator, &seed(), 5000, &TeamType::U18);
        assert_eq!(count_position(&staffs, StaffPosition::Manager), 0);
        assert_eq!(count_position(&staffs, StaffPosition::CaretakerManager), 0);
        assert!(!staffs.is_empty(), "youth team still has support staff");
    }

    #[test]
    fn elite_main_team_has_richer_backroom_than_small_club() {
        let generator = make_generator();
        let elite = DatabaseGenerator::generate_staffs(&generator, &seed(), 8000, &TeamType::Main);
        let small = DatabaseGenerator::generate_staffs(&generator, &seed(), 800, &TeamType::Main);
        assert!(
            elite.len() > small.len(),
            "elite backroom ({}) should exceed small club ({})",
            elite.len(),
            small.len()
        );
        // Modern leadership roles only at the very top.
        assert_eq!(count_position(&elite, StaffPosition::HeadOfRecruitment), 1);
        assert_eq!(count_position(&small, StaffPosition::HeadOfRecruitment), 0);
        assert_eq!(count_position(&elite, StaffPosition::HeadOfPhysio), 1);
        assert_eq!(count_position(&small, StaffPosition::HeadOfPhysio), 0);
    }
}
