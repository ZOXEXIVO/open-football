use crate::continent::CompetitionTier;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ContinentalRankings {
    pub country_rankings: Vec<(u32, f32)>, // country_id, coefficient
    pub club_rankings: Vec<(u32, f32)>,    // club_id, points
    pub qualification_spots: HashMap<u32, QualificationSpots>,
}

impl ContinentalRankings {
    pub fn new() -> Self {
        ContinentalRankings {
            country_rankings: Vec::new(),
            club_rankings: Vec::new(),
            qualification_spots: HashMap::new(),
        }
    }

    pub fn update_country_ranking(&mut self, country_id: u32, coefficient: f32) {
        if let Some(entry) = self
            .country_rankings
            .iter_mut()
            .find(|(id, _)| *id == country_id)
        {
            entry.1 = coefficient;
        } else {
            self.country_rankings.push((country_id, coefficient));
        }

        // Sort by coefficient descending
        self.country_rankings
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    }

    pub fn update_club_ranking(&mut self, club_id: u32, points: f32) {
        if let Some(entry) = self.club_rankings.iter_mut().find(|(id, _)| *id == club_id) {
            entry.1 = points;
        } else {
            self.club_rankings.push((club_id, points));
        }

        self.club_rankings
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    }

    pub fn get_top_country(&self) -> Option<u32> {
        self.country_rankings.first().map(|(id, _)| *id)
    }

    pub fn get_country_rankings(&self) -> &[(u32, f32)] {
        &self.country_rankings
    }

    pub fn get_qualified_clubs(&self) -> HashMap<CompetitionTier, Vec<u32>> {
        // Logic to determine which clubs qualify for each competition
        HashMap::new() // Simplified
    }

    pub fn set_qualification_spots(&mut self, country_id: u32, cl_spots: u8, el_spots: u8) {
        self.qualification_spots.insert(
            country_id,
            QualificationSpots {
                champions_league: cl_spots,
                europa_league: el_spots,
                conference_league: 1, // Default
            },
        );
    }
}

#[derive(Debug, Clone)]
pub struct QualificationSpots {
    pub champions_league: u8,
    pub europa_league: u8,
    pub conference_league: u8,
}
