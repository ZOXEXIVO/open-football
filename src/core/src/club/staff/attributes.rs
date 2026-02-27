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
