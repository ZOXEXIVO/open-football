#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadershipEventKind {
    CaptaincyAwarded,
    CaptaincyRemoved,
    LeadershipEmergence,
    SeniorPlayerMediates,
    BackedBySeniorPlayers,
    ChallengedTrainingStandards,
    InfluenceInDressingRoomRising,
    InfluenceInDressingRoomFalling,
    MentorshipStarted,
    MentorshipStrained,
    SquadLeadershipQuestioned,
}

impl LeadershipEventKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            LeadershipEventKind::CaptaincyAwarded => "leadership_kind_captaincy_awarded",
            LeadershipEventKind::CaptaincyRemoved => "leadership_kind_captaincy_removed",
            LeadershipEventKind::LeadershipEmergence => "leadership_kind_emergence",
            LeadershipEventKind::SeniorPlayerMediates => "leadership_kind_senior_mediates",
            LeadershipEventKind::BackedBySeniorPlayers => "leadership_kind_backed_seniors",
            LeadershipEventKind::ChallengedTrainingStandards => {
                "leadership_kind_challenged_standards"
            }
            LeadershipEventKind::InfluenceInDressingRoomRising => {
                "leadership_kind_influence_rising"
            }
            LeadershipEventKind::InfluenceInDressingRoomFalling => {
                "leadership_kind_influence_falling"
            }
            LeadershipEventKind::MentorshipStarted => "leadership_kind_mentorship_started",
            LeadershipEventKind::MentorshipStrained => "leadership_kind_mentorship_strained",
            LeadershipEventKind::SquadLeadershipQuestioned => {
                "leadership_kind_squad_leadership_questioned"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct LeadershipEventContext {
    pub kind: LeadershipEventKind,
    pub partner_player_id: Option<u32>,
    pub leadership_attribute: Option<f32>,
    pub influence_change: Option<f32>,
}

impl LeadershipEventContext {
    pub fn new(kind: LeadershipEventKind) -> Self {
        Self {
            kind,
            partner_player_id: None,
            leadership_attribute: None,
            influence_change: None,
        }
    }

    pub fn with_partner(mut self, id: u32) -> Self {
        self.partner_player_id = Some(id);
        self
    }
    pub fn with_leadership_attribute(mut self, attr: f32) -> Self {
        self.leadership_attribute = Some(attr);
        self
    }
    pub fn with_influence_change(mut self, change: f32) -> Self {
        self.influence_change = Some(change);
        self
    }
}
