use crate::Club;
use std::collections::HashMap;

#[derive(Debug)]
pub struct MediaCoverage {
    pub intensity: f32,
    pub trending_stories: Vec<MediaStory>,
    pub pressure_targets: HashMap<u32, f32>, // club_id -> pressure level
}

impl Default for MediaCoverage {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaCoverage {
    pub fn new() -> Self {
        MediaCoverage {
            intensity: 0.5,
            trending_stories: Vec::new(),
            pressure_targets: HashMap::new(),
        }
    }

    pub fn get_pressure_level(&self) -> f32 {
        self.intensity
    }

    pub fn update_from_results(&mut self, _results: &[crate::league::LeagueResult]) {
        // Update media intensity based on exciting results
        self.intensity = (self.intensity * 0.9 + 0.1).min(1.0);
    }

    pub fn generate_weekly_stories(&mut self, clubs: &[Club]) {
        self.trending_stories.clear();

        // Generate stories based on club performance, transfers, etc.
        use crate::utils::IntegerUtils;

        for club in clubs {
            if IntegerUtils::random(0, 100) > 80 {
                self.trending_stories.push(MediaStory {
                    club_id: club.id,
                    story_type: StoryType::TransferRumor,
                    intensity: 0.5,
                });
            }
        }
    }
}

#[derive(Debug)]
pub struct MediaStory {
    pub club_id: u32,
    pub story_type: StoryType,
    pub intensity: f32,
}

#[derive(Debug)]
pub enum StoryType {
    TransferRumor,
    ManagerPressure,
    PlayerControversy,
    SuccessStory,
    CrisisStory,
}
