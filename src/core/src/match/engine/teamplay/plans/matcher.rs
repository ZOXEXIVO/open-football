//! **The matcher** — how `DutyAssigner` picks WHO gets an assignment
//! once it knows what needs covering.
//!
//! One ranking primitive (`nearest_free`), one danger score
//! (`threat_score`), and the push that records the result. Kept apart
//! from the assignment pass in [`duties`](super::duties) because it is
//! the part that is purely about distance, incumbency and layer — no
//! duty semantics at all.

use crate::r#match::MatchPlayer;
use crate::r#match::engine::teamplay::plans::defence::{DefensiveDuty, DefensivePlan, MAX_UNIT};
use crate::r#match::engine::teamplay::plans::duties::DutyAssigner;
use nalgebra::Vector3;

impl DutyAssigner<'_> {
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
    pub(in crate::r#match::engine::teamplay::plans) fn nearest_free(
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
    pub(in crate::r#match::engine::teamplay::plans) fn threat_score(
        &self,
        opp: &MatchPlayer,
        own_goal: Vector3<f32>,
    ) -> f32 {
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

    pub(in crate::r#match::engine::teamplay::plans) fn push(
        plan: &mut DefensivePlan,
        player_id: u32,
        duty: DefensiveDuty,
    ) {
        if plan.len < MAX_UNIT {
            plan.duties[plan.len] = (player_id, duty);
            plan.len += 1;
        }
    }
}
