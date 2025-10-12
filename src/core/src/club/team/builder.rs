use crate::club::team::behaviour::TeamBehaviour;
use crate::{MatchHistory, PlayerCollection, StaffCollection, Tactics, Team, TeamReputation, TeamType, TrainingSchedule, Transfers};

#[derive(Default)]
pub struct TeamBuilder {
    id: Option<u32>,
    league_id: Option<u32>,
    club_id: Option<u32>,
    name: Option<String>,
    slug: Option<String>,
    team_type: Option<TeamType>,
    tactics: Option<Option<Tactics>>,
    players: Option<PlayerCollection>,
    staffs: Option<StaffCollection>,
    behaviour: Option<TeamBehaviour>,
    reputation: Option<TeamReputation>,
    training_schedule: Option<TrainingSchedule>,
    transfer_list: Option<Transfers>,
    match_history: Option<MatchHistory>,
}

impl TeamBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: u32) -> Self {
        self.id = Some(id);
        self
    }

    pub fn league_id(mut self, league_id: u32) -> Self {
        self.league_id = Some(league_id);
        self
    }

    pub fn club_id(mut self, club_id: u32) -> Self {
        self.club_id = Some(club_id);
        self
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn slug(mut self, slug: String) -> Self {
        self.slug = Some(slug);
        self
    }

    pub fn team_type(mut self, team_type: TeamType) -> Self {
        self.team_type = Some(team_type);
        self
    }

    pub fn tactics(mut self, tactics: Option<Tactics>) -> Self {
        self.tactics = Some(tactics);
        self
    }

    pub fn players(mut self, players: PlayerCollection) -> Self {
        self.players = Some(players);
        self
    }

    pub fn staffs(mut self, staffs: StaffCollection) -> Self {
        self.staffs = Some(staffs);
        self
    }

    pub fn behaviour(mut self, behaviour: TeamBehaviour) -> Self {
        self.behaviour = Some(behaviour);
        self
    }

    pub fn reputation(mut self, reputation: TeamReputation) -> Self {
        self.reputation = Some(reputation);
        self
    }

    pub fn training_schedule(mut self, training_schedule: TrainingSchedule) -> Self {
        self.training_schedule = Some(training_schedule);
        self
    }

    pub fn transfer_list(mut self, transfer_list: Transfers) -> Self {
        self.transfer_list = Some(transfer_list);
        self
    }

    pub fn match_history(mut self, match_history: MatchHistory) -> Self {
        self.match_history = Some(match_history);
        self
    }

    pub fn build(self) -> Result<Team, String> {
        Ok(Team {
            id: self.id.ok_or("id is required")?,
            league_id: self.league_id.ok_or("league_id is required")?,
            club_id: self.club_id.ok_or("club_id is required")?,
            name: self.name.ok_or("name is required")?,
            slug: self.slug.ok_or("slug is required")?,
            team_type: self.team_type.ok_or("team_type is required")?,
            tactics: self.tactics.unwrap_or(None),
            players: self.players.ok_or("players is required")?,
            staffs: self.staffs.ok_or("staffs is required")?,
            behaviour: self.behaviour.unwrap_or_else(TeamBehaviour::new),
            reputation: self.reputation.ok_or("reputation is required")?,
            training_schedule: self.training_schedule.ok_or("training_schedule is required")?,
            transfer_list: self.transfer_list.unwrap_or_else(Transfers::new),
            match_history: self.match_history.unwrap_or_else(MatchHistory::new),
        })
    }
}