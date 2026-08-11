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

use crate::r#match::{MatchField, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

/// Largest defensive unit we assign duties to.
const MAX_UNIT: usize = 8;
/// Most opponents worth ranking as threats.
const MAX_THREATS: usize = 10;

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
/// [`TeamTacticalState`](super::tactical::TeamTacticalState).
#[derive(Debug, Clone, Copy)]
pub struct DefensivePlan {
    duties: [(u32, DefensiveDuty); MAX_UNIT],
    len: usize,
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

/// One side's duty assignment pass.
struct DutyAssigner<'a> {
    field: &'a MatchField,
    team_id: u32,
}

impl DutyAssigner<'_> {
    /// Furthest a defender will travel to pick a man up (~19 m).
    ///
    /// Must not exceed `MARK_BREAK_DISTANCE`, the distance at which he
    /// actually breaks shape to go. An assignment beyond it is worse than
    /// no assignment: the marker holds the duty and never acts on it,
    /// while the exclusivity that makes this module work stops anybody
    /// else being given the same man. Measured at 175u against a 150u
    /// break: midfielders sat **13 m** from the defender nominally
    /// marking them, unmarked in every sense that matters, and free to
    /// carry and shoot — which is most of why they took 75% of all shots.
    const MARK_REACH: f32 = 150.0;
    /// The presser has to be able to actually get there (~25 m).
    const PRESS_REACH: f32 = 200.0;
    /// Cover sits within this of the carrier (~17 m).
    const COVER_REACH: f32 = 140.0;
    /// How far from our own goal an opponent can be and still be somebody
    /// the BACK LINE takes responsibility for, as a fraction of the pitch
    /// length. 0.34 ≈ 36 m — the defensive third and the approach to it.
    ///
    /// Was 0.62, which is 65 m: two thirds of the pitch. That made every
    /// central midfielder standing in midfield a "threat" a centre-back
    /// was assigned to, and because assignments are exclusive it did so
    /// at the cost of somebody who was actually in the box. It did not
    /// even produce marking — a defender will not follow a midfielder
    /// upfield of the ball (the goal-side guarantee in `DefensiveRecovery`
    /// correctly refuses to), so the assignment resolved as a man
    /// standing **10.9 m** from the player he was nominally marking while
    /// that player carried the ball and shot.
    ///
    /// Midfield is picked up by midfielders. A back four marks the people
    /// in and around its own box, and leaving the deep-lying passer to
    /// somebody else is the correct answer rather than a concession.
    const THREAT_DEPTH: f32 = 0.34;

    fn assign(&self, plan: &mut DefensivePlan, owner_team: Option<u32>) {
        let previous = *plan;
        *plan = DefensivePlan::idle();

        let Some(side) = self
            .field
            .players
            .iter()
            .find(|p| p.team_id == self.team_id)
            .and_then(|p| p.side)
        else {
            return;
        };
        let field_width = self.field.size.width as f32;
        let own_goal = Vector3::new(
            match side {
                PlayerSide::Left => 0.0,
                PlayerSide::Right => field_width,
            },
            self.field.size.height as f32 / 2.0,
            0.0,
        );

        // The defensive unit: the back line plus the holding midfielders
        // that sit in it (`is_defender` covers defensive midfielders — see
        // the depth-count convention).
        let mut unit = [(0u32, Vector3::<f32>::zeros()); MAX_UNIT];
        let mut unit_len = 0usize;
        for p in self.field.players.iter() {
            if p.team_id != self.team_id || unit_len == MAX_UNIT {
                continue;
            }
            let pos = p.tactical_position.current_position;
            if pos.is_goalkeeper() || !pos.is_defender() {
                continue;
            }
            unit[unit_len] = (p.id, p.position);
            unit_len += 1;
        }
        if unit_len == 0 {
            return;
        }
        plan.active = true;

        let mut taken = [false; MAX_UNIT];
        let carrier = owner_team
            .filter(|t| *t != self.team_id)
            .and_then(|_| self.field.ball.current_owner)
            .and_then(|id| self.field.players.iter().find(|p| p.id == id))
            .filter(|p| p.team_id != self.team_id);
        plan.carrier = carrier.map(|c| c.id);

        // ── Press, then cover ────────────────────────────────────────
        // Somebody goes to the ball and somebody backs him up. These come
        // first because they are the only duties whose target is fixed;
        // everything else is a choice between men.
        if let Some(carrier) = carrier {
            if let Some(i) = self.nearest_free(
                &unit[..unit_len],
                &taken,
                carrier.position,
                Self::PRESS_REACH,
                previous.presser(),
            ) {
                taken[i] = true;
                Self::push(plan, unit[i].0, DefensiveDuty::Press);
            }
            if let Some(i) = self.nearest_free(
                &unit[..unit_len],
                &taken,
                carrier.position,
                Self::COVER_REACH,
                None,
            ) {
                taken[i] = true;
                Self::push(plan, unit[i].0, DefensiveDuty::Cover);
            }
        }

        // ── Man-mark the rest, most dangerous first ──────────────────
        // Ranked once, assigned exclusively. This is the whole point: the
        // ranking decides WHO is dangerous, and the assignment guarantees
        // that each of them is somebody's problem and nobody's twice.
        let mut threats = [(0u32, Vector3::<f32>::zeros(), 0.0f32); MAX_THREATS];
        let mut threat_len = 0usize;
        let depth_limit = field_width * Self::THREAT_DEPTH;
        for p in self.field.players.iter() {
            if p.team_id == self.team_id || threat_len == MAX_THREATS {
                continue;
            }
            if p.tactical_position.current_position.is_goalkeeper() {
                continue;
            }
            if Some(p.id) == plan.carrier {
                continue; // the presser has him
            }
            if (p.position.x - own_goal.x).abs() > depth_limit {
                continue; // not a threat yet
            }
            threats[threat_len] = (p.id, p.position, self.threat_score(p, own_goal));
            threat_len += 1;
        }
        // Insertion sort, most dangerous first; ties by id so the
        // assignment is reproducible run to run.
        for i in 1..threat_len {
            let item = threats[i];
            let mut j = i;
            while j > 0
                && (threats[j - 1].2 < item.2
                    || (threats[j - 1].2 == item.2 && threats[j - 1].0 > item.0))
            {
                threats[j] = threats[j - 1];
                j -= 1;
            }
            threats[j] = item;
        }

        for &(opp_id, opp_pos, _) in &threats[..threat_len] {
            // Incumbency: whoever had this man keeps him if he is still
            // in range. A defender who swaps his man every refresh is not
            // marking anybody.
            let incumbent = previous.duties[..previous.len]
                .iter()
                .find(|(_, d)| d.target() == Some(opp_id))
                .map(|(id, _)| *id);
            let Some(i) = self.nearest_free(
                &unit[..unit_len],
                &taken,
                opp_pos,
                Self::MARK_REACH,
                incumbent,
            ) else {
                continue;
            };
            taken[i] = true;
            Self::push(plan, unit[i].0, DefensiveDuty::Mark(opp_id));
        }

        // ── Everyone else holds the zone ─────────────────────────────
        for (i, (id, _)) in unit[..unit_len].iter().enumerate() {
            if !taken[i] {
                Self::push(plan, *id, DefensiveDuty::HoldZone);
            }
        }
    }

    /// Closest unassigned unit member to `target` within `reach`.
    ///
    /// `incumbent` gets a distance discount rather than an override — he
    /// keeps the job while he is still a reasonable choice, and loses it
    /// when somebody is clearly better placed. A hard override would
    /// strand a defender chasing a man he can no longer reach.
    fn nearest_free(
        &self,
        unit: &[(u32, Vector3<f32>)],
        taken: &[bool; MAX_UNIT],
        target: Vector3<f32>,
        reach: f32,
        incumbent: Option<u32>,
    ) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, (id, pos)) in unit.iter().enumerate() {
            if taken[i] {
                continue;
            }
            let raw = (pos - target).magnitude();
            if raw > reach {
                continue;
            }
            // ~4 m of stickiness.
            let effective = if incumbent == Some(*id) {
                raw - 32.0
            } else {
                raw
            };
            let better = match best {
                None => true,
                Some((bi, bd)) => effective < bd || (effective == bd && unit[i].0 < unit[bi].0),
            };
            if better {
                best = Some((i, effective));
            }
        }
        best.map(|(i, _)| i)
    }

    /// How dangerous this opponent is to us right now.
    ///
    /// Continuous and additive rather than banded: proximity to goal is
    /// the spine, a run AT the goal multiplies it, and being a plausible
    /// next receiver matters because marking is about the pass that is
    /// coming, not the one that already happened.
    fn threat_score(&self, opp: &MatchPlayer, own_goal: Vector3<f32>) -> f32 {
        let to_goal = (opp.position - own_goal).magnitude();
        // 0 at 60 m out, 1 on the goal line.
        let mut score = (1.0 - to_goal / 480.0).clamp(0.0, 1.0);

        // A man running at goal is a different problem from a man
        // standing in the same place.
        let v = opp.velocity;
        let speed = v.norm();
        if speed > 0.05 {
            let toward = (own_goal - opp.position)
                .try_normalize(0.01)
                .unwrap_or_default();
            let alignment = (v / speed).dot(&toward);
            if alignment > 0.0 {
                score += alignment * (speed / 0.6).min(1.0) * 0.45;
            }
        }

        // Close to the ball = plausible next receiver.
        let ball_gap = (opp.position - self.field.ball.position).magnitude();
        score += (1.0 - ball_gap / 320.0).clamp(0.0, 1.0) * 0.35;

        // Forwards are marked ahead of full-backs at the same distance.
        if opp.tactical_position.current_position.is_forward() {
            score += 0.18;
        }
        score
    }

    fn push(plan: &mut DefensivePlan, player_id: u32, duty: DefensiveDuty) {
        if plan.len < MAX_UNIT {
            plan.duties[plan.len] = (player_id, duty);
            plan.len += 1;
        }
    }
}
