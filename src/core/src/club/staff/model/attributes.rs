use crate::transfers::ScoutingRegion;

#[derive(Debug, Clone)]
pub struct StaffAttributes {
    pub coaching: StaffCoaching,
    pub goalkeeping: StaffGoalkeeperCoaching,
    pub mental: StaffMental,
    pub knowledge: StaffKnowledge,
    pub data_analysis: StaffDataAnalysis,
    pub medical: StaffMedical,
}

#[derive(Debug, Clone)]
pub struct StaffCoaching {
    pub attacking: u8,
    pub defending: u8,
    pub fitness: u8,
    pub mental: u8,
    pub tactical: u8,
    pub technical: u8,
    pub working_with_youngsters: u8,
}

#[derive(Debug, Clone)]
pub struct StaffGoalkeeperCoaching {
    pub distribution: u8,
    pub handling: u8,
    pub shot_stopping: u8,
}

#[derive(Debug, Clone)]
pub struct StaffMental {
    pub adaptability: u8,
    pub determination: u8,
    pub discipline: u8,
    pub man_management: u8,
    pub motivating: u8,
}

#[derive(Debug, Clone)]
pub struct StaffKnowledge {
    pub judging_player_ability: u8,
    pub judging_player_potential: u8,
    pub tactical_knowledge: u8,
    /// Geographic regions this scout knows well (can scout effectively there).
    /// A scout knowing WestAfrica can evaluate players from Nigeria, Ghana,
    /// Ivory Coast, Cameroon, Senegal, etc. — the entire region.
    /// Scouting in known regions has normal accuracy; unknown regions have
    /// increased error and fewer observations per day.
    pub known_regions: Vec<ScoutingRegion>,
    /// Per-region familiarity score (0-100). Grows over time as the scout
    /// spends assignment days in a region, boosting report accuracy and
    /// expanding the effective player pool they consider.
    pub region_familiarity: Vec<RegionFamiliarity>,
    /// The COUNTRIES this scout actually knows, 0-100 each.
    ///
    /// Regions are too coarse to be the unit of knowledge: knowing Colombia
    /// is not knowing Peru, and no real scouting department covers "South
    /// America". The region list above stays as the coarse reach the older
    /// gates read; this is what decides which foreign players a club can go
    /// and FIND. Seeded at generation from the club's country priors and
    /// grown by assignment days, the same way regions are.
    pub known_countries: Vec<CountryKnowledge>,
}

/// One country a scout knows, and how well.
#[derive(Debug, Clone)]
pub struct CountryKnowledge {
    pub country_id: u32,
    /// 0-100, on the same scale as [`RegionFamiliarity::level`].
    pub level: u8,
    /// Total assignment days spent watching players from this country.
    pub days_scouted: u32,
}

#[derive(Debug, Clone)]
pub struct RegionFamiliarity {
    pub region: ScoutingRegion,
    pub level: u8,
    /// Total days spent scouting in the region over the scout's career.
    pub days_scouted: u32,
}

impl StaffKnowledge {
    /// Advance this scout's familiarity with a given region by one day.
    /// Returns the new familiarity level. Cap at 100.
    pub fn accrue_region_day(&mut self, region: ScoutingRegion) -> u8 {
        if let Some(entry) = self
            .region_familiarity
            .iter_mut()
            .find(|r| r.region == region)
        {
            entry.days_scouted = entry.days_scouted.saturating_add(1);
            // Logarithmic growth: each +100 days yields roughly one extra point
            // at the start, then slows toward the cap.
            let days = entry.days_scouted as f32;
            entry.level = ((days.sqrt() * 2.0) as u8).min(100);
            entry.level
        } else {
            self.region_familiarity.push(RegionFamiliarity {
                region,
                level: 1,
                days_scouted: 1,
            });
            1
        }
    }

    /// Familiarity score (0-100) for a region. Returns 0 if never scouted.
    pub fn familiarity_for(&self, region: ScoutingRegion) -> u8 {
        self.region_familiarity
            .iter()
            .find(|r| r.region == region)
            .map(|r| r.level)
            .unwrap_or(0)
    }

    /// Advance this scout's knowledge of one COUNTRY by a day's watching.
    /// Same curve as the region: `2·sqrt(days)`, capped at 100, so a market
    /// takes about two seasons of real attention to know well and a lifetime
    /// to know perfectly.
    pub fn accrue_country_day(&mut self, country_id: u32) -> u8 {
        if let Some(entry) = self
            .known_countries
            .iter_mut()
            .find(|c| c.country_id == country_id)
        {
            entry.days_scouted = entry.days_scouted.saturating_add(1);
            entry.level = (((entry.days_scouted as f32).sqrt() * 2.0) as u8).min(100);
            entry.level
        } else {
            self.known_countries.push(CountryKnowledge {
                country_id,
                level: 1,
                days_scouted: 1,
            });
            1
        }
    }

    /// How well this scout knows one country, 0-100. `0` for a market he has
    /// never worked.
    pub fn country_level(&self, country_id: u32) -> u8 {
        self.known_countries
            .iter()
            .find(|c| c.country_id == country_id)
            .map(|c| c.level)
            .unwrap_or(0)
    }

    /// Seed a country the scout arrives already knowing — a career hire, or
    /// a specialist the club went out and found. Never lowers an existing
    /// level.
    pub fn seed_country(&mut self, country_id: u32, level: u8) {
        // Invert the growth curve so a seeded level continues to grow from
        // where it was set rather than restarting at day one.
        let days = ((level as f32 / 2.0).powi(2)).round() as u32;
        match self
            .known_countries
            .iter_mut()
            .find(|c| c.country_id == country_id)
        {
            Some(entry) => {
                if level > entry.level {
                    entry.level = level;
                    entry.days_scouted = entry.days_scouted.max(days);
                }
            }
            None => self.known_countries.push(CountryKnowledge {
                country_id,
                level,
                days_scouted: days,
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StaffDataAnalysis {
    pub judging_player_data: u8,
    pub judging_team_data: u8,
    pub presenting_data: u8,
}

#[derive(Debug, Clone)]
pub struct StaffMedical {
    pub physiotherapy: u8,
    pub sports_science: u8,
    pub non_player_tendencies: u8,
}

impl StaffAttributes {}
