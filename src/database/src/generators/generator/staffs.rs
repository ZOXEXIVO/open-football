use crate::generators::StaffGenerator;
use core::utils::IntegerUtils;
use core::{Staff, StaffPosition, TeamType};

use super::DatabaseGenerator;

impl DatabaseGenerator {
    pub(super) fn generate_staffs(
        staff_generator: &StaffGenerator,
        country_id: u32,
        continent_id: u32,
        country_code: &str,
        team_reputation: u16,
        team_type: &TeamType,
    ) -> Vec<Staff> {
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

            // Scouts get known_regions: home region + foreign regions weighted by transfer corridors
            // Better clubs have scouts with wider knowledge networks
            let mut chief_scout =
                staff_generator.generate(country_id, StaffPosition::ChiefScout, team_reputation);
            Self::assign_scout_regions(
                &mut chief_scout,
                continent_id,
                country_code,
                team_reputation,
            );
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
                Self::assign_scout_regions(&mut scout, continent_id, country_code, team_reputation);
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

        // Specialist coaches appear as the club can afford them.
        if team_reputation >= 3000 {
            staffs.push(hire(StaffPosition::GoalkeeperCoach));
        }
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
    }

    /// Give a scout knowledge of their home region + foreign regions weighted
    /// by real-world transfer corridors (Africa→Europe, SouthAmerica→Europe, etc.).
    ///
    /// A minority of scouts at well-funded clubs are "foreign specialists":
    /// their personal home region is a corridor pick (e.g. Spartak hiring a
    /// Brazilian-born scout for S.America coverage) and they start with
    /// non-trivial familiarity there, modelling a real career hire.
    fn assign_scout_regions(
        staff: &mut Staff,
        continent_id: u32,
        country_code: &str,
        team_reputation: u16,
    ) {
        use core::RegionFamiliarity;
        use core::transfers::ScoutingRegion;

        let club_region = ScoutingRegion::from_country(continent_id, country_code);

        let specialist_chance = if team_reputation >= 7000 {
            40
        } else if team_reputation >= 5000 {
            25
        } else if team_reputation >= 3000 {
            10
        } else {
            0
        };

        let corridors = club_region.transfer_corridors();
        let total_weight: u32 = corridors.iter().map(|(_, w)| *w as u32).sum();

        let is_specialist = specialist_chance > 0
            && IntegerUtils::random(0, 100) < specialist_chance
            && total_weight > 0
            && !corridors.is_empty();

        let (home_region, mut regions, familiarity) = if is_specialist {
            let specialty = Self::pick_weighted_region(corridors, total_weight);
            (
                specialty,
                vec![specialty, club_region],
                vec![RegionFamiliarity {
                    region: specialty,
                    level: 40,
                    days_scouted: 400,
                }],
            )
        } else {
            (club_region, vec![club_region], Vec::new())
        };

        // Number of foreign regions based on club reputation
        let foreign_count = if team_reputation >= 7000 {
            IntegerUtils::random(2, 4) as usize
        } else if team_reputation >= 5000 {
            IntegerUtils::random(1, 3) as usize
        } else if team_reputation >= 3000 {
            IntegerUtils::random(0, 2) as usize
        } else {
            IntegerUtils::random(0, 1) as usize
        };

        if foreign_count == 0 || total_weight == 0 || corridors.is_empty() {
            staff.staff_attributes.knowledge.known_regions = regions;
            staff.staff_attributes.knowledge.region_familiarity = familiarity;
            return;
        }

        // Corridors are rooted in the scout's actual home region — a specialist
        // Brazilian scout brings Brazilian-nearby corridors, not Russian ones.
        let home_corridors = home_region.transfer_corridors();
        let home_total: u32 = home_corridors.iter().map(|(_, w)| *w as u32).sum();
        if home_total == 0 || home_corridors.is_empty() {
            staff.staff_attributes.knowledge.known_regions = regions;
            staff.staff_attributes.knowledge.region_familiarity = familiarity;
            return;
        }

        for _ in 0..foreign_count {
            let region = Self::pick_weighted_region(home_corridors, home_total);
            if !regions.contains(&region) {
                regions.push(region);
            }
        }

        staff.staff_attributes.knowledge.known_regions = regions;
        staff.staff_attributes.knowledge.region_familiarity = familiarity;
    }

    fn pick_weighted_region(
        corridors: &[(core::transfers::ScoutingRegion, u8)],
        total_weight: u32,
    ) -> core::transfers::ScoutingRegion {
        let roll = IntegerUtils::random(0, total_weight as i32) as u32;
        let mut acc = 0u32;
        for (region, weight) in corridors {
            acc += *weight as u32;
            if roll < acc {
                return *region;
            }
        }
        corridors[0].0
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
                DatabaseGenerator::generate_staffs(&generator, 1, 1, "EN", rep, &TeamType::Main);
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
    fn youth_team_gets_no_manager_seat() {
        let generator = make_generator();
        let staffs =
            DatabaseGenerator::generate_staffs(&generator, 1, 1, "EN", 5000, &TeamType::U18);
        assert_eq!(count_position(&staffs, StaffPosition::Manager), 0);
        assert_eq!(count_position(&staffs, StaffPosition::CaretakerManager), 0);
        assert!(!staffs.is_empty(), "youth team still has support staff");
    }

    #[test]
    fn elite_main_team_has_richer_backroom_than_small_club() {
        let generator = make_generator();
        let elite =
            DatabaseGenerator::generate_staffs(&generator, 1, 1, "EN", 8000, &TeamType::Main);
        let small = DatabaseGenerator::generate_staffs(&generator, 1, 1, "EN", 800, &TeamType::Main);
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
