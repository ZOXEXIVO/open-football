//! Per-possession DEFENSIVE assignment — who picks up whom, shared by
//! the whole defensive unit. The mirror of [`AttackPlan`](super::attack).
//!
//! # The problem this exists to solve
//!
//! Defensive position in this engine was computed against the BALL and
//! against the KICKOFF FORMATION, and never against an opponent.
//!
//! * `DefensiveRecovery::depth_override` aimed every defender at one
//!   shared `ball_x ± 24u`, at one shared base speed. Four defenders of
//!   similar pace converge on that single depth and then slide together
//!   as the ball moves — a line moving as one body, which is exactly what
//!   it looked like.
//! * `DefenderRunningState::phase_dispatch` — reached by 50% of the back
//!   line at the moment a shot is struck — told everyone except the
//!   single closest defender to go and stand on the line. No opponent
//!   enters that decision at all.
//! * `HoldingLine::calculate_zonal_position` anchors lateral position on
//!   `start_position`, the kickoff slot, so the spacing between defenders
//!   is a formation constant rather than a response to the attack.
//! * `defensive_role_for_ball_carrier` grants `Primary` to exactly one
//!   defender. Everyone else got a positional role.
//!
//! Measured consequences, sampled only while actually defending: **74% of
//! attackers inside our own defensive third had no defender within 3 m**,
//! the average nearest defender was 6.4 m away, the widest gap between
//! adjacent defenders was 21.7 m, and defenders who WERE in a shot's
//! window sat 14.5 m off its line. Blocks landed on 1.2% of shots against
//! a real 18-22%.
//!
//! # The rule
//!
//! Somebody is responsible for every dangerous opponent, and no two
//! defenders are responsible for the same one. Duties are assigned once,
//! at team level, from a single ranking — so a defender's question stops
//! being "where is the line?" and becomes "where is my man?".
//!
//! Assignments are sticky across refreshes (an incumbency bonus), because
//! a defender who swaps his man every quarter-second is not marking
//! anybody.

use crate::r#match::MatchField;
use crate::r#match::engine::teamplay::plans::duties::DutyAssigner;

/// Largest set of players we assign duties to — every outfielder.
///
/// Only the back line and the midfield are eligible to be given a MAN
/// (see `can_mark` in [`DutyAssigner::assign`]); the forwards are here
/// because pressing is not marking. The presser is by definition whoever
/// is nearest the ball, and when the opposition build from the back that
/// is the centre-forward.
pub(in crate::r#match::engine::teamplay::plans) const MAX_UNIT: usize = 11;

/// What one defender is responsible for in the current defensive phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefensiveDuty {
    /// Go to the ball carrier and engage him. Exactly one per side.
    Press,
    /// Second body, goal-side of the presser — the one who deals with it
    /// when the presser is beaten.
    Cover,
    /// Man-mark this specific opponent. Exclusive: no two defenders ever
    /// hold a `Mark` on the same id.
    Mark(u32),
    /// Nobody to pick up — hold the zone and the line.
    HoldZone,
}

impl DefensiveDuty {
    /// Is this duty attached to a specific opponent?
    pub fn target(self) -> Option<u32> {
        match self {
            DefensiveDuty::Mark(id) => Some(id),
            _ => None,
        }
    }

    /// Duties that mean "I am responsible for a man", as opposed to
    /// holding space. Read by the states that decide whether to break
    /// the line.
    pub fn is_individual(self) -> bool {
        !matches!(self, DefensiveDuty::HoldZone)
    }
}

/// Defensive assignments for one side. Cheap to copy, like
/// [`TeamTacticalState`](super::super::tactical::TeamTacticalState).
#[derive(Debug, Clone, Copy)]
pub struct DefensivePlan {
    pub(in crate::r#match::engine::teamplay::plans) duties: [(u32, DefensiveDuty); MAX_UNIT],
    pub(in crate::r#match::engine::teamplay::plans) len: usize,
    /// The opponent carrying the ball, if any.
    pub carrier: Option<u32>,
    /// True while this side is the defending side and the plan is live.
    pub active: bool,
}

impl DefensivePlan {
    pub const fn idle() -> Self {
        DefensivePlan {
            duties: [(0, DefensiveDuty::HoldZone); MAX_UNIT],
            len: 0,
            carrier: None,
            active: false,
        }
    }

    /// This player's duty. `HoldZone` when the plan is inert or he isn't
    /// part of the defensive unit — the safe default, since it is what
    /// every defender did before this module existed.
    pub fn duty_of(&self, player_id: u32) -> DefensiveDuty {
        if !self.active {
            return DefensiveDuty::HoldZone;
        }
        self.duties[..self.len]
            .iter()
            .find(|(id, _)| *id == player_id)
            .map(|(_, d)| *d)
            .unwrap_or(DefensiveDuty::HoldZone)
    }

    /// The opponent this player is man-marking, if any.
    pub fn mark_of(&self, player_id: u32) -> Option<u32> {
        self.duty_of(player_id).target()
    }

    /// The designated presser.
    pub fn presser(&self) -> Option<u32> {
        if !self.active {
            return None;
        }
        self.duties[..self.len]
            .iter()
            .find(|(_, d)| matches!(d, DefensiveDuty::Press))
            .map(|(id, _)| *id)
    }

    /// Is somebody already responsible for this opponent?
    pub fn is_marked(&self, opponent_id: u32) -> bool {
        self.active
            && self.duties[..self.len]
                .iter()
                .any(|(_, d)| d.target() == Some(opponent_id))
    }

    /// How many of the unit hold an individual duty. Diagnostics only.
    pub fn individual_count(&self) -> usize {
        self.duties[..self.len]
            .iter()
            .filter(|(_, d)| d.is_individual())
            .count()
    }

    /// Recompute both sides' plans in place, on the tactical cadence.
    pub fn refresh(home: &mut Self, away: &mut Self, inputs: &DefenceRefreshInputs<'_>) {
        let field = inputs.field;
        let owner_team = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id);

        for (plan, team_id) in [
            (&mut *home, inputs.home_team_id),
            (&mut *away, inputs.away_team_id),
        ] {
            // We are defending when somebody else has the ball, or when
            // it is loose — a loose ball in our half still has to be
            // picked up by somebody, and the shape that decides who is
            // the defensive one.
            let defending = owner_team.is_some_and(|t| t != team_id) || owner_team.is_none();
            if !defending {
                *plan = DefensivePlan::idle();
                #[cfg(feature = "match-logs")]
                crate::mid_run_diag::DefenceDiag::note_plan(false, 0);
                continue;
            }
            DutyAssigner { field, team_id }.assign(plan, owner_team);
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::DefenceDiag::note_plan(plan.active, plan.individual_count());
        }
    }
}

/// Inputs to [`DefensivePlan::refresh`].
pub struct DefenceRefreshInputs<'a> {
    pub field: &'a MatchField,
    pub home_team_id: u32,
    pub away_team_id: u32,
}
