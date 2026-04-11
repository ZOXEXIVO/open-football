/// Facility quality levels for clubs.
/// These affect training quality, youth development, and player generation.

#[derive(Debug, Clone, PartialEq)]
pub enum FacilityLevel {
    Best,
    Exceptional,
    Superb,
    Excellent,
    Great,
    Good,
    Adequate,
    Average,
    BelowAverage,
    FairlyBasic,
    Basic,
    Limited,
    Poor,
}

impl FacilityLevel {
    /// Parse from string (e.g. "Superb", "Below Average")
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "best" => FacilityLevel::Best,
            "exceptional" => FacilityLevel::Exceptional,
            "superb" => FacilityLevel::Superb,
            "excellent" => FacilityLevel::Excellent,
            "great" => FacilityLevel::Great,
            "good" => FacilityLevel::Good,
            "adequate" => FacilityLevel::Adequate,
            "average" => FacilityLevel::Average,
            "below average" => FacilityLevel::BelowAverage,
            "fairly basic" => FacilityLevel::FairlyBasic,
            "basic" => FacilityLevel::Basic,
            "limited" => FacilityLevel::Limited,
            "poor" | "unknown" => FacilityLevel::Poor,
            _ => FacilityLevel::Average,
        }
    }

    /// Numeric value 1-20
    pub fn to_rating(&self) -> u8 {
        match self {
            FacilityLevel::Best => 20,
            FacilityLevel::Exceptional => 19,
            FacilityLevel::Superb => 17,
            FacilityLevel::Excellent => 15,
            FacilityLevel::Great => 13,
            FacilityLevel::Good => 11,
            FacilityLevel::Adequate => 9,
            FacilityLevel::Average => 7,
            FacilityLevel::BelowAverage => 5,
            FacilityLevel::FairlyBasic => 4,
            FacilityLevel::Basic => 3,
            FacilityLevel::Limited => 2,
            FacilityLevel::Poor => 1,
        }
    }

    /// Normalized 0.0 - 1.0 multiplier
    pub fn multiplier(&self) -> f32 {
        self.to_rating() as f32 / 20.0
    }
}

impl Default for FacilityLevel {
    fn default() -> Self {
        FacilityLevel::Average
    }
}

/// Club-level facilities that affect training, youth development, and player generation.
#[derive(Debug, Clone)]
pub struct ClubFacilities {
    /// Quality of first-team training facilities
    pub training: FacilityLevel,
    /// Quality of youth team facilities
    pub youth: FacilityLevel,
    /// Quality of the youth academy (coaching, scouting, programs)
    pub academy: FacilityLevel,
    /// Reach and quality of youth recruitment network
    pub recruitment: FacilityLevel,
    /// Average match attendance
    pub average_attendance: u32,
}

impl Default for ClubFacilities {
    fn default() -> Self {
        ClubFacilities {
            training: FacilityLevel::Average,
            youth: FacilityLevel::Average,
            academy: FacilityLevel::Average,
            recruitment: FacilityLevel::Average,
            average_attendance: 0,
        }
    }
}

impl ClubFacilities {
    /// Training quality multiplier (affects player development speed)
    pub fn training_multiplier(&self) -> f32 {
        self.training.multiplier()
    }

    /// Youth development multiplier (affects academy player quality)
    pub fn youth_multiplier(&self) -> f32 {
        (self.youth.multiplier() + self.academy.multiplier()) / 2.0
    }

    /// Youth recruitment multiplier (affects academy intake quality)
    pub fn recruitment_multiplier(&self) -> f32 {
        self.recruitment.multiplier()
    }

    /// Dynamic attendance multiplier: responds to form and league position.
    ///
    /// - `recent_wins_ratio` is the club's win rate over the last ~5 games (0.0–1.0)
    /// - `league_position` is the current league position (1-indexed)
    /// - `total_teams` is the number of teams in the league
    ///
    /// Returns a multiplier around 1.0 — happy fans show up (1.1–1.25),
    /// disillusioned fans stay home (0.7–0.9). Cup runs, top-of-table,
    /// or relegation-battle drama all push attendance higher.
    pub fn dynamic_attendance_multiplier(
        &self,
        recent_wins_ratio: f32,
        league_position: u16,
        total_teams: u16,
    ) -> f32 {
        // Form component: −0.20 at zero wins, +0.20 at all-win streak.
        let form = (recent_wins_ratio - 0.5) * 0.4;

        // Position component — spectators love top-half runs and relegation
        // six-pointers; mid-table apathy drops attendance.
        let tt = total_teams.max(1) as f32;
        let pos = league_position.max(1) as f32;
        let rel_pos = pos / tt; // 0.0 top, 1.0 bottom
        let position = if rel_pos < 0.1 {
            0.20 // Title race
        } else if rel_pos < 0.25 {
            0.10 // European places
        } else if rel_pos > 0.85 {
            0.10 // Relegation drama
        } else if rel_pos > 0.7 {
            0.03
        } else {
            -0.05 // Mid-table apathy
        };

        (1.0 + form + position).clamp(0.65, 1.30)
    }
}
