use crate::generators::StaffGenerator;
use core::utils::IntegerUtils;
use core::{Staff, StaffPosition, TeamType};

use super::DatabaseGenerator;

impl DatabaseGenerator {
    pub(super) fn generate_staffs(staff_generator: &StaffGenerator, country_id: u32, continent_id: u32, country_code: &str, team_reputation: u16, team_type: &TeamType) -> Vec<Staff> {
        let mut staffs = Vec::with_capacity(30);

        if *team_type == TeamType::Main {
            // Only main team gets directors and scouts
            staffs.push(staff_generator.generate(country_id, StaffPosition::DirectorOfFootball, team_reputation));
            staffs.push(staff_generator.generate(country_id, StaffPosition::Director, team_reputation));

            // Scouts get known_regions: home region + foreign regions weighted by transfer corridors
            // Better clubs have scouts with wider knowledge networks
            let mut chief_scout = staff_generator.generate(country_id, StaffPosition::ChiefScout, team_reputation);
            Self::assign_scout_regions(&mut chief_scout, continent_id, country_code, team_reputation);
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
                let mut scout = staff_generator.generate(country_id, StaffPosition::Scout, team_reputation);
                Self::assign_scout_regions(&mut scout, continent_id, country_code, team_reputation);
                staffs.push(scout);
            }
        }

        staffs.push(staff_generator.generate(country_id, StaffPosition::AssistantManager, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Coach, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Coach, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Coach, team_reputation));

        staffs.push(staff_generator.generate(country_id, StaffPosition::Physio, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Physio, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Physio, team_reputation));

        staffs
    }

    /// Give a scout knowledge of their home region + foreign regions weighted
    /// by real-world transfer corridors (Africa→Europe, SouthAmerica→Europe, etc.).
    ///
    /// A minority of scouts at well-funded clubs are "foreign specialists":
    /// their personal home region is a corridor pick (e.g. Spartak hiring a
    /// Brazilian-born scout for S.America coverage) and they start with
    /// non-trivial familiarity there, modelling a real career hire.
    fn assign_scout_regions(staff: &mut Staff, continent_id: u32, country_code: &str, team_reputation: u16) {
        use core::transfers::ScoutingRegion;
        use core::RegionFamiliarity;

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

        let is_specialist =
            specialist_chance > 0 && IntegerUtils::random(0, 100) < specialist_chance
                && total_weight > 0 && !corridors.is_empty();

        let (home_region, mut regions, familiarity) = if is_specialist {
            let specialty = Self::pick_weighted_region(corridors, total_weight);
            (
                specialty,
                vec![specialty, club_region],
                vec![RegionFamiliarity { region: specialty, level: 40, days_scouted: 400 }],
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
