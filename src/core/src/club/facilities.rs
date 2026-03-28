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
}
