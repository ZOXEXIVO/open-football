use crate::generators::StaffGenerator;
use core::utils::IntegerUtils;
use core::{Staff, StaffPosition, TeamType};

use super::DatabaseGenerator;

impl DatabaseGenerator {
    pub(super) fn generate_staffs(staff_generator: &mut StaffGenerator, country_id: u32, continent_id: u32, country_code: &str, team_reputation: u16, team_type: &TeamType) -> Vec<Staff> {
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

            for _ in 0..2 {
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
    fn assign_scout_regions(staff: &mut Staff, continent_id: u32, country_code: &str, team_reputation: u16) {
        use core::transfers::ScoutingRegion;

        let home_region = ScoutingRegion::from_country(continent_id, country_code);
        let mut regions = vec![home_region];

        // Number of foreign regions based on club reputation
        let foreign_count = if team_reputation >= 7000 {
            IntegerUtils::random(2, 4) as usize // Elite: 2-4 foreign regions
        } else if team_reputation >= 5000 {
            IntegerUtils::random(1, 3) as usize // Good: 1-3
        } else if team_reputation >= 3000 {
            IntegerUtils::random(0, 2) as usize // Mid: 0-2
        } else {
            IntegerUtils::random(0, 1) as usize // Small: 0-1
        };

        if foreign_count == 0 {
            staff.staff_attributes.knowledge.known_regions = regions;
            return;
        }

        // Pick foreign regions weighted by transfer corridors
        let corridors = home_region.transfer_corridors();
        let total_weight: u32 = corridors.iter().map(|(_, w)| *w as u32).sum();

        if total_weight == 0 || corridors.is_empty() {
            staff.staff_attributes.knowledge.known_regions = regions;
            return;
        }

        for _ in 0..foreign_count {
            let roll = IntegerUtils::random(0, total_weight as i32) as u32;
            let mut acc = 0u32;
            for (region, weight) in corridors {
                acc += *weight as u32;
                if roll < acc {
                    if !regions.contains(region) {
                        regions.push(*region);
                    }
                    break;
                }
            }
        }

        staff.staff_attributes.knowledge.known_regions = regions;
    }
}
