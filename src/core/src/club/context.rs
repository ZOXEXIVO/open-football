#[derive(Clone)]
pub struct ClubContext<'c> {
    pub id: u32,
    pub name: &'c str,
    /// Training facility quality (0.0-1.0 normalized from FacilityLevel)
    pub training_facility_quality: f32,
    /// Youth facility quality (0.0-1.0)
    pub youth_facility_quality: f32,
    /// Academy quality (0.0-1.0)
    pub academy_quality: f32,
    /// Youth recruitment quality (0.0-1.0)
    pub recruitment_quality: f32,
}

impl<'c> ClubContext<'c> {
    pub fn new(id: u32, name: &'c str) -> Self {
        ClubContext {
            id,
            name,
            training_facility_quality: 0.35, // Average default
            youth_facility_quality: 0.35,
            academy_quality: 0.35,
            recruitment_quality: 0.35,
        }
    }

    pub fn with_facilities(mut self, training: f32, youth: f32, academy: f32, recruitment: f32) -> Self {
        self.training_facility_quality = training;
        self.youth_facility_quality = youth;
        self.academy_quality = academy;
        self.recruitment_quality = recruitment;
        self
    }
}
