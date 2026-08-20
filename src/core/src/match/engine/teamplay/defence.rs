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

use crate::r#match::{MatchContext, MatchField, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

/// Largest set of players we assign duties to — every outfielder.
///
/// Only the back line and the midfield are eligible to be given a MAN
/// (see `can_mark` in [`DutyAssigner::assign`]); the forwards are here
/// because pressing is not marking. The presser is by definition whoever
/// is nearest the ball, and when the opposition build from the back that
/// is the centre-forward.
const MAX_UNIT: usize = 11;
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
    /// How heavily an assignment that would pull a defender UPFIELD out of
    /// his line counts against him, per unit of displacement. 0.9 nearly
    /// doubles the depth term in the matcher's distance, so a man in front
    /// of the back four goes to whoever is already in front of the back
    /// four. See the note in `nearest_free`.
    const LAYER_BIAS: f32 = 0.9;
    /// The presser has to be able to actually get there (~25 m).
    const PRESS_REACH: f32 = 200.0;
    /// What a candidate for the PRESS duty is charged for being on a
    /// tackle cooldown, in the same units as the distance he is ranked
    /// by. 60u ≈ 7.5 m — comfortably more than the incumbency discount
    /// (32u), so a man who has just gone to ground is displaced by any
    /// team-mate who is genuinely in the picture, and keeps the job when
    /// he is the only one there.
    const NOT_READY_SURCHARGE: f32 = 60.0;
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

    /// …and how near the BALL an opponent has to be to be somebody's
    /// problem wherever on the pitch he is standing.
    ///
    /// # Why depth alone is not enough
    ///
    /// [`Self::THREAT_DEPTH`] answers "is he dangerous to our goal",
    /// which is one half of what a defending team marks. The other half
    /// is the press: when the ball is in midfield the men who matter are
    /// the ones the carrier can find with his next pass, and they are
    /// 30-40 m from our goal, not 10.
    ///
    /// With depth as the only door, a side defending in midfield had
    /// **one presser, one cover and nobody else with a job at all** —
    /// measured, 2.8 of 8 unit members on an individual duty, and every
    /// state-level press entry is additionally gated on being the single
    /// best chaser. The opposition passed and carried through the centre
    /// completely unopposed: `prog_carries` 38 per midfielder per match
    /// against a real 1-2, `succ_dribbles` 9.2 against ~0.5, and 58% of
    /// attackers in our own third with nobody within 3 m. From the stands
    /// that is "nobody presses, they just run at the goal and shoot",
    /// which is exactly how it was reported.
    ///
    /// 200u = 25 m — the radius a pressing unit really covers around the
    /// ball. Deliberately generous: the assignment is exclusive and every
    /// marker still has to be within [`Self::MARK_REACH`] of his man, so
    /// a threat nobody can reach costs a ranking slot and nothing else.
    const BALL_THREAT_RADIUS: f32 = 200.0;

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
        // Toward the opponent goal — the axis an assignment can pull a
        // defender along, and the one `nearest_free` charges for.
        let forward = side.forward_dir_x();

        // The defensive unit: the back line AND the midfield.
        //
        // It was defenders only, and that turned out to be the single
        // biggest distortion in the attacking half of the game. A back
        // four cannot mark five attacking players, so the threat ranking
        // spent its exclusive slots on whoever scored highest — the
        // forwards — and every opposing MIDFIELDER was left completely
        // unmarked. Measured: forwards held at 4.0 m by an assigned
        // marker, midfielders at 10.9 m by nobody, and midfielders
        // consequently took **74% of every shot in the game** against a
        // real 32% while forwards took 23% against a real 58%.
        //
        // Midfield is marked by midfielders. Including them is what lets
        // the ranking cover the whole attacking side instead of running
        // out of bodies after the front line.
        //
        // ── …and the FORWARDS are here for the press only ─────────────
        //
        // `can_mark` is what keeps a striker out of his own box: he can
        // never be given a man, ranked as cover, or told to hold a zone.
        // He can be the presser, because that duty is not a marking
        // assignment at all — it is "you are nearest the ball, go".
        //
        // Leaving the front line out of the pool entirely meant it had no
        // defensive job in the model whatsoever. The only door left into
        // `Forward: Pressing` was `is_best_player_to_chase_ball`, a
        // single-player election run across the whole team, and the state
        // measured **0.3% of all AI ticks — 2.2% of a forward's own
        // match**. From the stands that is a front two watching the
        // opposition play out from the back, which is how it was
        // reported.
        let mut unit = [(0u32, Vector3::<f32>::zeros(), false, true); MAX_UNIT];
        let mut unit_len = 0usize;
        let mut markers = 0usize;
        for p in self.field.players.iter() {
            if p.team_id != self.team_id || unit_len == MAX_UNIT {
                continue;
            }
            let pos = p.tactical_position.current_position;
            if pos.is_goalkeeper() {
                continue;
            }
            let can_mark = pos.is_defender() || pos.is_midfielder();
            if !can_mark && MatchContext::press_off() {
                continue; // A/B control — see `MatchContext::press_off`.
            }
            unit[unit_len] = (p.id, p.position, can_mark, p.can_attempt_tackle());
            unit_len += 1;
            markers += can_mark as usize;
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

        // ── Press, then cover ────────────────────────────────────────
        // Somebody goes to the ball and somebody backs him up. These come
        // first because they are the only duties whose target is fixed;
        // everything else is a choice between men.
        let mut n_press = 0usize;
        let mut n_cover = 0usize;
        let mut n_marks = 0usize;
        let mut n_unreachable = 0usize;
        let mut n_skipped_depth = 0usize;

        // ── Where the next touch is going to happen ──────────────────
        //
        // With an opponent on the ball this is simply the man. Without
        // one it is the man the ball is on its way to — and that case is
        // most of a match: measured over 24 fixtures the plan was live on
        // 80% of refreshes but nominated a presser on only **25%**,
        // because `carrier` is `None` for every pass in flight and every
        // loose ball, and with ~1 700 passes a match at ~1.5 s of flight
        // each nobody owns the ball for well over half the game. For all
        // of that time the plan said press 0.00 / cover 0.00 and handed
        // the whole side marks and zones, which is the model literally
        // not pressing.
        //
        // A ball travelling to an opponent is the moment a press is FOR —
        // you go with the pass, not after it lands.
        //
        // Two cases are deliberately NOT pressed:
        //
        //   * a ball on its way to one of our own players. That is a
        //     reception, and the receiver's own states handle it.
        //   * a ball that has come to REST. Nobody is about to receive
        //     it; it is a race, and the engine already elects exactly one
        //     chaser per side for a race (`should_force_takeball`, with
        //     hysteresis). Nominating a presser on top of that election
        //     puts a second and third body on the same square metre, and
        //     the ownership contest between them is what produces the
        //     standing knot this pass exists to remove.
        //
        // ⚠ THE TARGET IS THE BALL, NOT THE MAN ABOUT TO GET IT. Aiming
        // the press at the receiver instead reads better on paper — you
        // close the man down, somebody else races for the ball — and it
        // measured worse on every axis, over 3×40 fixtures per arm:
        // tackles/team 22.5 → 28.9 (real 18), fouls 19.7 → 21.6 (real
        // 12), yellows 5.92 → 6.80, goals 5.30 → 6.36. Sending the
        // presser THROUGH the receiver puts him touch-tight to a man who
        // has not got the ball yet, and the engagement models
        // (`ContactFoul`, `TackleDecision`) then roll once a second for
        // as long as he stays there. Aiming at the ball makes him arrive
        // WITH it, which is what pressing a pass looks like.
        //
        // Measured, off vs on over 3×40 fixtures: press duties per
        // refresh 0.25 → 0.61, cover 0.21 → 0.53, unit holding a zone
        // 2.9 → 2.3; the carrier's nearest opponent 3.67 → 3.53 m and the
        // opponents within 10 m of him 1.87 → 2.00. Calibration-neutral:
        // goals 5.89 → 5.76, fouls 19.5 → 18.6, yellows 5.85 → 5.86,
        // penalties 0.33 → 0.23, tackles 22.7 → 24.4, stalled-ball time
        // 15.3 → 16.9 s/match.
        let ball = &self.field.ball;
        let our_reception = ball
            .pass_target_player_id
            .and_then(|id| self.field.players.iter().find(|p| p.id == id))
            .is_some_and(|p| p.team_id == self.team_id);
        // 0.2 u/tick ≈ 2.5 m/s — quicker than a ball trundling to a stop
        // and far slower than any pass. (NB the ball's speed is in
        // u/tick, where a hard pass is ~1.2 and the fastest loose ball
        // ever measured is 3.19; `force_claim_if_deadlock`'s own
        // "stopped" threshold of 3.0 is therefore always true and cannot
        // be borrowed for this.)
        let travelling = ball.velocity.norm() > 0.2;
        plan.carrier = carrier.map(|c| c.id);
        let point_of_attack = match carrier {
            Some(c) => Some(c.position),
            // A/B control — see `MatchContext::press_off`.
            None if travelling && !our_reception && !MatchContext::press_off() => {
                Some(ball.position)
            }
            None => None,
        };

        if let Some(point_of_attack) = point_of_attack {
            // No layer bias on either of these — see `nearest_free`. The
            // bias exists to hand a MAN to the defender already standing
            // in his layer; the presser is whoever can reach the ball
            // first, and charging him for being behind it would pick the
            // man facing the wrong way.
            if let Some(i) = self.nearest_free(
                &unit[..unit_len],
                &taken,
                point_of_attack,
                Self::PRESS_REACH,
                previous.presser(),
                forward,
                0.0,
                false,
                true,
            ) {
                taken[i] = true;
                n_press += 1;
                Self::push(plan, unit[i].0, DefensiveDuty::Press);
            }
            // Cover is a back-line job — a forward sitting goal-side of
            // the presser is not what the duty means — so it is markers
            // only.
            //
            // Readiness counts here for the same reason it counts for
            // the press: `Cover` is "the one who deals with it when the
            // presser is beaten", and inside our own area
            // `TackleEngagement::may_engage_carrier` now licenses him to
            // do exactly that. A man who cannot challenge is a poor
            // choice for the job whose definition is the challenge.
            if let Some(i) = self.nearest_free(
                &unit[..unit_len],
                &taken,
                point_of_attack,
                Self::COVER_REACH,
                None,
                forward,
                0.0,
                true,
                true,
            ) {
                taken[i] = true;
                n_cover += 1;
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
            // Dangerous to our goal, OR near the ball — see
            // `BALL_THREAT_RADIUS`. Either makes him somebody's man.
            let near_goal = (p.position.x - own_goal.x).abs() <= depth_limit;
            let near_ball =
                (p.position - self.field.ball.position).magnitude() <= Self::BALL_THREAT_RADIUS;
            if !near_goal && !near_ball {
                n_skipped_depth += 1;
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
                forward,
                Self::LAYER_BIAS,
                true,
                false,
            ) else {
                n_unreachable += 1;
                continue;
            };
            taken[i] = true;
            n_marks += 1;
            Self::push(plan, unit[i].0, DefensiveDuty::Mark(opp_id));
        }

        // ── Everyone else holds the zone ─────────────────────────────
        //
        // Markers only. A forward the press did not need is not "holding
        // a zone" in any defensive sense — he is up the pitch waiting for
        // the ball back, and giving him the duty would only make the
        // zone-holding census lie.
        for (i, (id, _, can_mark, _)) in unit[..unit_len].iter().enumerate() {
            if !taken[i] && *can_mark {
                Self::push(plan, *id, DefensiveDuty::HoldZone);
            }
        }

        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::DefenceDiag::note_plan_shape(
            markers,
            threat_len,
            n_skipped_depth,
            n_unreachable,
            n_press,
            n_cover,
            n_marks,
        );
    }

    /// Closest unassigned unit member to `target` within `reach`.
    ///
    /// `incumbent` gets a distance discount rather than an override — he
    /// keeps the job while he is still a reasonable choice, and loses it
    /// when somebody is clearly better placed. A hard override would
    /// strand a defender chasing a man he can no longer reach.
    ///
    /// `layer_bias` is the surcharge on being dragged upfield (see
    /// below); it belongs to MARKING and is passed as 0.0 for the press.
    /// `markers_only` skips the forwards, who are in the pool for the
    /// press and nothing else.
    ///
    /// `ready_matters` surcharges a candidate who cannot currently
    /// attempt a challenge — see [`Self::NOT_READY_SURCHARGE`]. Passed
    /// for the PRESS nomination only: marking a man and holding a zone
    /// are jobs a defender on a tackle cooldown does perfectly well.
    fn nearest_free(
        &self,
        unit: &[(u32, Vector3<f32>, bool, bool)],
        taken: &[bool; MAX_UNIT],
        target: Vector3<f32>,
        reach: f32,
        incumbent: Option<u32>,
        forward: f32,
        layer_bias: f32,
        markers_only: bool,
        ready_matters: bool,
    ) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (i, (id, pos, can_mark, ready)) in unit.iter().enumerate() {
            if taken[i] || (markers_only && !can_mark) {
                continue;
            }
            let raw = (pos - target).magnitude();
            if raw > reach {
                continue;
            }
            // ── MARK THE MAN IN YOUR LAYER ────────────────────────────
            //
            // Straight-line proximity has no idea which of two defenders a
            // man BELONGS to, and the two answers are not equally good: a
            // centre-half who takes a midfielder loitering eight metres in
            // front of the back four has to leave the line to do it, and
            // the screening midfielder standing next to that man — whose
            // job it is — is left marking nobody. Real defences hand the
            // man in front of the line to the man in front of the line.
            //
            // Measured: 56% of all marking assignments were on
            // MIDFIELDERS, and once markers were allowed to reach their
            // man (see `DefensiveLine::hold_shape_on_man`) the back-four
            // depth spread went 13.2 m → 15.0 m — the back four following
            // midfielders upfield, which is the right bound applied to the
            // wrong assignment.
            //
            // Charging the UPFIELD component twice is the whole rule: a
            // defender already level with the threat pays nothing, one who
            // would have to be dragged out of his line pays for every
            // metre of it. No roles are named and no thresholds are drawn
            // — a screener wins the midfielder because he is already
            // standing there, which is what being a screener means.
            let pulled_upfield = ((target.x - pos.x) * forward).max(0.0);
            let raw = raw + pulled_upfield * layer_bias;
            // ── …AND HE HAS TO BE ABLE TO GO ─────────────────────────
            //
            // A defender who has just challenged carries a tackle
            // cooldown, and `can_attempt_tackle` gates ENTRY into every
            // `Tackling` state. Nominating him as the presser therefore
            // does not merely pick a slightly worse man: because the
            // press duty is exclusive and `may_engage_carrier` refuses
            // everyone else, it leaves the side with **no legal
            // challenger at all** until it runs out. And the incumbency
            // below is a stickiness of four metres, which re-elects him
            // on the next refresh and the one after that.
            //
            // Charged rather than skipped, in the same shape as the
            // layer bias: he keeps the job when nobody else is anywhere
            // near, and loses it the moment a team-mate is within a few
            // metres — which is exactly what a defence does when one of
            // its own has gone to ground.
            let raw = if ready_matters && !*ready {
                raw + Self::NOT_READY_SURCHARGE
            } else {
                raw
            };
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
