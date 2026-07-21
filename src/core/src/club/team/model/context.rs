use crate::club::player::position::PlayerPositionType;
use crate::club::team::TeamType;

#[derive(Clone)]
pub struct TeamContext {
    pub id: u32,
    pub reputation: f32,
    /// Snapshot of the team's current formation so per-player processing
    /// (role fit, position coverage checks) can reason about its fit
    /// without reaching back into the team object.
    pub formation: Option<[PlayerPositionType; 11]>,
    /// Which squad tier this team is (Main / B / Reserve / Second /
    /// U18..U23). Lets squad-level passes (team behaviour audits) reason
    /// about "life below the first team" without reaching back into the
    /// team object. `None` when the constructing site didn't know it.
    pub team_type: Option<TeamType>,
    /// Official club captain as of the last monthly review
    /// (`Team::captain_id`). Threaded in so captain-centric behaviour
    /// passes (mediation, morale propagation) act through the appointed
    /// armband holder instead of re-electing an ad-hoc leader that can
    /// disagree with the one the club — and the player — sees. `None`
    /// when the constructing site didn't know it, or the team genuinely
    /// has no captain.
    pub captain_id: Option<u32>,
    /// Official vice-captain (`Team::vice_captain_id`); same contract as
    /// [`Self::captain_id`].
    pub vice_captain_id: Option<u32>,
    /// Reputation (0..10000) of the competition THIS team plays in — the
    /// B team's third division, not the club's top flight. Development
    /// reads this so a reserve squad doesn't train "in La Liga". `None`
    /// when the constructing site didn't know it; consumers fall back to
    /// the club's main-league reputation.
    pub league_reputation: Option<u16>,
    /// Best coaching scores (0-20) on THIS team's own staff, stamped by
    /// `Team::simulate`. The development tick prefers these over the
    /// club-wide bests so a U18 squad trains under its own coaches.
    /// `None` when the constructing site didn't know them.
    pub coaching: Option<TeamCoachingScores>,
}

/// Per-team best coaching attribute scores (0-20 each), computed from the
/// team's own staff list. GK is the average of the three goalkeeping
/// coaching attributes, mirroring the club-wide snapshot's formula.
#[derive(Debug, Clone, Copy)]
pub struct TeamCoachingScores {
    pub technical: u8,
    pub mental: u8,
    pub fitness: u8,
    pub goalkeeping: u8,
}

impl TeamCoachingScores {
    pub fn from_staffs(staffs: &crate::StaffCollection) -> Self {
        let mut best = TeamCoachingScores {
            technical: 0,
            mental: 0,
            fitness: 0,
            goalkeeping: 0,
        };
        for staff in staffs.iter() {
            let coaching = &staff.staff_attributes.coaching;
            best.technical = best.technical.max(coaching.technical);
            best.mental = best.mental.max(coaching.mental);
            best.fitness = best.fitness.max(coaching.fitness);
            let gk = &staff.staff_attributes.goalkeeping;
            let gk_avg =
                ((gk.shot_stopping as u16 + gk.handling as u16 + gk.distribution as u16) / 3) as u8;
            best.goalkeeping = best.goalkeeping.max(gk_avg);
        }
        best
    }
}

impl TeamContext {
    pub fn new(id: u32) -> Self {
        TeamContext {
            id,
            reputation: 0.0,
            formation: None,
            team_type: None,
            captain_id: None,
            vice_captain_id: None,
            league_reputation: None,
            coaching: None,
        }
    }

    pub fn with_reputation(id: u32, reputation: f32) -> Self {
        TeamContext {
            id,
            reputation,
            formation: None,
            team_type: None,
            captain_id: None,
            vice_captain_id: None,
            league_reputation: None,
            coaching: None,
        }
    }

    pub fn with_formation(mut self, formation: [PlayerPositionType; 11]) -> Self {
        self.formation = Some(formation);
        self
    }

    pub fn with_type(mut self, team_type: TeamType) -> Self {
        self.team_type = Some(team_type);
        self
    }

    pub fn with_captaincy(
        mut self,
        captain_id: Option<u32>,
        vice_captain_id: Option<u32>,
    ) -> Self {
        self.captain_id = captain_id;
        self.vice_captain_id = vice_captain_id;
        self
    }
}
