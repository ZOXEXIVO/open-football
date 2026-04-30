#[derive(Clone)]
pub struct ClubContext<'c> {
    pub id: u32,
    pub name: &'c str,
    pub training_facility_quality: f32,
    pub youth_facility_quality: f32,
    pub academy_quality: f32,
    pub recruitment_quality: f32,
    /// Main team's current league position (1-based, 0 = unknown)
    pub league_position: u8,
    /// Total teams in the league
    pub league_size: u8,
    /// Total matches in a full season (for season progress calculation)
    pub total_league_matches: u8,
    /// League matches the main team has actually played this season.
    pub league_matches_played: u8,
    /// Tier of the main team's primary league (1 = top flight, 2 = second
    /// division, …). Drives the TV revenue ladder.
    pub main_league_tier: u8,
    /// Best physiotherapy score on the club staff (0.0-1.0).
    /// Drives injury recovery speedup and preventive rest.
    pub medical_quality: f32,
    /// Best sports_science score on the club staff (0.0-1.0).
    /// Lowers per-day injury risk and re-injury chance during recovery.
    pub sports_science_quality: f32,
    /// Best working_with_youngsters score on the club staff (0.0-1.0).
    /// Amplifies development gains for players under 23.
    pub youth_coaching_quality: f32,
    /// Best technical coaching score on the club staff (0-20).
    pub coach_best_technical: u8,
    /// Best mental coaching score on the club staff (0-20).
    pub coach_best_mental: u8,
    /// Best fitness coaching score on the club staff (0-20).
    pub coach_best_fitness: u8,
    /// Best goalkeeping (shot stopping) score on the club staff (0-20).
    pub coach_best_goalkeeping: u8,
    /// Main team's blended reputation (0..10000). Used by the academy
    /// generator and other systems that should treat a youth player at Real
    /// Madrid differently from one at a regional minnow with the same
    /// facility ratings.
    pub main_team_reputation: u16,
    /// Main team's world reputation (0..10000) — the international-stage
    /// signal that pulls top youth talent regardless of league context.
    pub main_team_world_reputation: u16,
    /// Reputation of the league the main team plays in (0..10000). Drives
    /// pathway prestige and how much development opportunity youth get from
    /// stepping up.
    pub league_reputation: u16,
    /// Country-level football-ecosystem reputation (0..10000). Same value as
    /// CountryContext.reputation, surfaced here so the academy doesn't have
    /// to walk back up the context tree.
    pub country_reputation: u16,
    /// Internal academy pathway reputation (0..100). Owned by the academy,
    /// mirrored here so the player generator can consult it without taking
    /// a reference to the academy struct itself.
    pub pathway_reputation: u8,
}

impl<'c> ClubContext<'c> {
    pub fn new(id: u32, name: &'c str) -> Self {
        ClubContext {
            id,
            name,
            training_facility_quality: 0.35,
            youth_facility_quality: 0.35,
            academy_quality: 0.35,
            recruitment_quality: 0.35,
            league_position: 0,
            league_size: 0,
            total_league_matches: 0,
            league_matches_played: 0,
            main_league_tier: 1,
            medical_quality: 0.35,
            sports_science_quality: 0.35,
            youth_coaching_quality: 0.35,
            coach_best_technical: 10,
            coach_best_mental: 10,
            coach_best_fitness: 10,
            coach_best_goalkeeping: 10,
            main_team_reputation: 0,
            main_team_world_reputation: 0,
            league_reputation: 0,
            country_reputation: 0,
            pathway_reputation: 50,
        }
    }

    pub fn with_facilities(
        mut self,
        training: f32,
        youth: f32,
        academy: f32,
        recruitment: f32,
    ) -> Self {
        self.training_facility_quality = training;
        self.youth_facility_quality = youth;
        self.academy_quality = academy;
        self.recruitment_quality = recruitment;
        self
    }

    pub fn with_league_position(
        mut self,
        position: u8,
        league_size: u8,
        total_matches: u8,
        matches_played: u8,
    ) -> Self {
        self.league_position = position;
        self.league_size = league_size;
        self.total_league_matches = total_matches;
        self.league_matches_played = matches_played;
        self
    }

    pub fn with_main_league_tier(mut self, tier: u8) -> Self {
        self.main_league_tier = tier;
        self
    }

    pub fn with_staff_quality(
        mut self,
        medical: f32,
        sports_science: f32,
        youth_coaching: f32,
    ) -> Self {
        self.medical_quality = medical;
        self.sports_science_quality = sports_science;
        self.youth_coaching_quality = youth_coaching;
        self
    }

    pub fn with_coach_scores(
        mut self,
        technical: u8,
        mental: u8,
        fitness: u8,
        goalkeeping: u8,
    ) -> Self {
        self.coach_best_technical = technical;
        self.coach_best_mental = mental;
        self.coach_best_fitness = fitness;
        self.coach_best_goalkeeping = goalkeeping;
        self
    }

    pub fn with_reputations(
        mut self,
        main_team_reputation: u16,
        main_team_world_reputation: u16,
        league_reputation: u16,
        country_reputation: u16,
    ) -> Self {
        self.main_team_reputation = main_team_reputation;
        self.main_team_world_reputation = main_team_world_reputation;
        self.league_reputation = league_reputation;
        self.country_reputation = country_reputation;
        self
    }

    pub fn with_pathway_reputation(mut self, pathway_reputation: u8) -> Self {
        self.pathway_reputation = pathway_reputation;
        self
    }
}
