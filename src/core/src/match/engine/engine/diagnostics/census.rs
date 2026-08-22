//! **The `match-logs` censuses the tick runs.** Sampled from
//! `game_tick_inner`, inert in a normal build, and read by the
//! `dev_match` harness — never delete these.
//!
//! * the defending side's shape, sampled only while it is actually
//!   defending, because an average over a whole match is dominated by
//!   possession;
//! * what the man nearest the carrier in our box actually DOES about
//!   him, bucketed by which gate stopped him;
//! * whether a chaser is running AT a loose ball or merely alongside it.

use crate::PlayerPositionType;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::engine::*;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use nalgebra::Vector3;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Sample the DEFENDING side's shape while it is actually defending.
    ///
    /// Two questions, both of which an aggregate over a whole match
    /// answers wrongly because most of a match is spent attacking:
    ///
    /// * **Is the back line a rigid body?** `depth spread` is the range of
    ///   the back four along the goal-to-goal axis. A real back four
    ///   staggers — the cover defender drops behind the presser, the far
    ///   full-back tucks in — so 25-65u (3-8 m) is normal. Near zero means
    ///   four players sharing one depth target and sliding as one.
    /// * **Does anybody meet the attacker?** For every opponent inside our
    ///   defensive third, how far away is the nearest defender. Real
    ///   marking distances in a settled block are 2-6 m on the ball side.
    ///
    /// Sampled every 25 ticks (quarter-second) and only while an opponent
    /// carries the ball in our half, so the numbers describe defending
    /// rather than an average dominated by possession.
    #[cfg(feature = "match-logs")]
    pub(in crate::r#match::engine::engine) fn sample_defensive_shape(
        field: &MatchField,
        context: &MatchContext,
    ) {
        use crate::mid_run_diag::DefenceDiag;

        const SAMPLE_INTERVAL_TICKS: u64 = 25;
        /// A defender this close is contesting the attacker. 24u = 3 m.
        const MARKED_RADIUS: f32 = 24.0;

        if context.current_tick() % SAMPLE_INTERVAL_TICKS != 0 {
            return;
        }
        let Some(carrier) = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
        else {
            return;
        };
        let attacking_team = carrier.team_id;
        let Some(defending_side) = field
            .players
            .iter()
            .find(|p| p.team_id != attacking_team)
            .and_then(|p| p.side)
        else {
            return;
        };
        let field_width = field.size.width as f32;
        let own_goal_x = match defending_side {
            PlayerSide::Left => 0.0,
            PlayerSide::Right => field_width,
        };
        // Only sample while the ball is in the defending side's half —
        // otherwise "the back line" is a pressing line and the marking
        // question is meaningless.
        if (field.ball.position.x - own_goal_x).abs() > field_width * 0.5 {
            return;
        }

        // ── Back-line shape ──────────────────────────────────────────
        let mut xs = [0.0f32; 8];
        let mut ys = [0.0f32; 8];
        let mut n = 0usize;
        for p in field.players.iter() {
            if p.team_id == attacking_team || n == xs.len() {
                continue;
            }
            let pos = p.tactical_position.current_position;
            // `is_defender()` is the POSITION GROUP, and that group holds
            // `DefensiveMidfielder` (see `position_group`). A DM sits ten
            // to fifteen metres in front of the back four on purpose, so
            // including him put that gap into `max_x - min_x`
            // permanently: this printed 17.5 m of "back-line depth
            // spread" against a real-back-four reference of 3-8 m and
            // could never have reached it, whatever the defenders did.
            // Two rounds of shape work were measured against that number
            // before it was checked. The back LINE is the back four.
            if !pos.is_defender()
                || pos.is_goalkeeper()
                || matches!(pos, PlayerPositionType::DefensiveMidfielder)
            {
                continue;
            }
            // …and the man who has gone to the ball is not part of the
            // shape. Somebody always leaves the line to engage — measured
            // at 25% of the back line at the moment a shot is struck —
            // and with four defenders that is one man permanently 15 m
            // upfield, which puts 15 m into `max_x - min_x` on its own.
            // The number could therefore never approach its own
            // real-football reference no matter how the line behaved, and
            // it did not move across two rounds of shape work for exactly
            // that reason. What the shape constraint governs, and what
            // this should report, is the spread of the defenders actually
            // holding shape.
            if matches!(
                p.state,
                PlayerState::Defender(
                    DefenderState::Pressing
                        | DefenderState::Tackling
                        | DefenderState::TakeBall
                        | DefenderState::Intercepting
                )
            ) {
                continue;
            }
            xs[n] = p.position.x;
            ys[n] = p.position.y;
            n += 1;
        }
        if n >= 3 {
            let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
            for &x in &xs[..n] {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
            // Insertion-sort the lateral positions so "adjacent pair" is
            // meaningful rather than roster order.
            for i in 1..n {
                let v = ys[i];
                let mut j = i;
                while j > 0 && ys[j - 1] > v {
                    ys[j] = ys[j - 1];
                    j -= 1;
                }
                ys[j] = v;
            }
            let mut max_gap = 0.0f32;
            for w in ys[..n].windows(2) {
                max_gap = max_gap.max(w[1] - w[0]);
            }
            DefenceDiag::note_shape(max_x - min_x, max_gap);
        }

        // ── Is anybody meeting the attackers? ────────────────────────
        let third = field_width / 3.0;
        for a in field.players.iter() {
            if a.team_id != attacking_team {
                continue;
            }
            if a.tactical_position.current_position.is_goalkeeper() {
                continue;
            }
            if (a.position.x - own_goal_x).abs() > third {
                continue;
            }
            let mut nearest = f32::MAX;
            for d in field.players.iter() {
                if d.team_id == attacking_team
                    || d.tactical_position.current_position.is_goalkeeper()
                {
                    continue;
                }
                nearest = nearest.min((d.position - a.position).magnitude());
            }
            if nearest.is_finite() {
                DefenceDiag::note_attacker(nearest, nearest > MARKED_RADIUS);
            }
        }

        // ── The marking duel itself ──────────────────────────────────
        //
        // The measure above is every defender against every attacker, and
        // it is dominated by sheer density: the goal-side recovery rule
        // puts bodies near attackers whether or not anybody is marking
        // them, so it cannot tell you whether marking is being BEATEN.
        // This one reads the assigned pairs only — the distance between a
        // marker and the man he was actually given — which is the number
        // the evasion work moves or does not.
        let plan = context.defence_plan_for_team(
            field
                .players
                .iter()
                .find(|p| p.team_id != attacking_team)
                .map(|p| p.team_id)
                .unwrap_or(attacking_team),
        );
        if plan.active {
            for d in field.players.iter() {
                let Some(target) = plan.mark_of(d.id) else {
                    continue;
                };
                if let Some(man) = field.players.iter().find(|p| p.id == target) {
                    let pos = man.tactical_position.current_position;
                    let line = if pos.is_forward() {
                        2
                    } else if pos.is_midfielder() {
                        1
                    } else {
                        0
                    };
                    // Is the assigned marker in a state that actually
                    // HONOURS the assignment? The plan hands out duties
                    // to the whole unit, but only two states read them —
                    // `Marking` for a defender and `Guarding` for a
                    // midfielder — so a marker in any other state is
                    // carrying a duty nothing acts on, and tuning the
                    // marking distance reaches none of those ticks. Same
                    // failure shape as the back line's shape code living
                    // in `HoldingLine`.
                    // 0 marking, 1 playing the ball (legitimate), 2
                    // pressing/covering, 3 running/recovering, 4 idle.
                    // Only bucket 4 — and most of 3 — is a duty nobody
                    // is acting on.
                    let bucket = match d.state {
                        PlayerState::Defender(DefenderState::Marking)
                        | PlayerState::Midfielder(MidfielderState::Guarding) => 0,
                        PlayerState::Defender(
                            DefenderState::Tackling
                            | DefenderState::Intercepting
                            | DefenderState::TakeBall
                            | DefenderState::Clearing
                            | DefenderState::Heading
                            | DefenderState::Passing,
                        )
                        | PlayerState::Midfielder(
                            MidfielderState::Tackling
                            | MidfielderState::Intercepting
                            | MidfielderState::TakeBall
                            | MidfielderState::Heading
                            | MidfielderState::Passing,
                        ) => 1,
                        PlayerState::Defender(
                            DefenderState::Pressing | DefenderState::Covering,
                        )
                        | PlayerState::Midfielder(MidfielderState::Pressing) => 2,
                        PlayerState::Defender(
                            DefenderState::Running
                            | DefenderState::Returning
                            | DefenderState::TrackingBack,
                        )
                        | PlayerState::Midfielder(
                            MidfielderState::Running | MidfielderState::Returning,
                        ) => 3,
                        _ => 4,
                    };
                    DefenceDiag::note_duel((d.position - man.position).magnitude(), line, bucket);
                }
            }
        }
    }

    /// **What happens to the man who has the ball in our box.**
    ///
    /// The shape sampler above measures where defenders STAND. This one
    /// measures what they DO about a carrier who is already among them —
    /// the question behind "he runs around the penalty area surrounded by
    /// defenders and nobody tries to take it off him".
    ///
    /// Every challenge in the engine, from every state and every role,
    /// funnels through the same three gates before an attempt is rolled:
    /// the per-player tackle cooldown (`can_attempt_tackle`), the duel
    /// gate (`TackleEngagement::may_engage_carrier`), and the commitment
    /// roll (`TackleDecision`). A defender stopped by any of them looks
    /// identical from the stands and identical in the aggregate stats —
    /// he is next to the carrier, doing nothing. Bucketing the pairs by
    /// which gate stopped them is the only way to tell which one is
    /// binding.
    ///
    /// Sampled every tick, because a challenge is a moment: a
    /// quarter-second sampler would miss most of the `Tackling` ticks it
    /// is looking for.
    #[cfg(feature = "match-logs")]
    pub(in crate::r#match::engine::engine) fn sample_duel_gates(
        field: &MatchField,
        context: &MatchContext,
    ) {
        use crate::r#match::common_states::TackleEngagement;
        use crate::mid_run_diag::DuelDiag;

        /// Close enough to be in the picture the report describes. 24u = 3 m.
        const SURROUND_RADIUS: f32 = 24.0;

        let Some(carrier) = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
        else {
            return;
        };
        if carrier.tactical_position.current_position.is_goalkeeper() {
            return;
        }
        let attacking_team = carrier.team_id;
        let Some(defending) = field.players.iter().find(|p| p.team_id != attacking_team) else {
            return;
        };
        let defending_team = defending.team_id;
        let Some(defending_side) = defending.side else {
            return;
        };
        let plan = context.defence_plan_for_team(defending_team);
        let presser = plan.presser();
        // The referee's own test — the ball inside the area the defending
        // side is protecting. Same question `PenaltyRisk::applies` asks.
        let in_box = context
            .penalty_area(defending_side == PlayerSide::Left)
            .contains(&field.ball.position);

        let mut bodies = 0u64;
        let mut contested = false;
        // Nearest defender to the carrier, for the closing census below.
        let mut nearest: Option<(f32, &MatchPlayer)> = None;
        for d in field.players.iter() {
            if d.team_id == attacking_team || d.tactical_position.current_position.is_goalkeeper() {
                continue;
            }
            DuelDiag::note_cooldown(!d.can_attempt_tackle());
            let gap = (d.position - carrier.position).magnitude();
            if nearest.is_none_or(|(best, _)| gap < best) {
                nearest = Some((gap, d));
            }
            if gap <= SURROUND_RADIUS {
                bodies += 1;
            }
            let challenging = matches!(
                d.state,
                PlayerState::Defender(DefenderState::Tackling)
                    | PlayerState::Midfielder(MidfielderState::Tackling)
                    | PlayerState::Forward(ForwardState::Tackling)
            );
            if challenging {
                // An attempt is only ever rolled inside `CONTACT`. Where the
                // rest of them are standing is the difference between a
                // defence that declines its challenges and one that never
                // reaches them.
                DuelDiag::note_reach(if gap <= TackleEngagement::CONTACT {
                    0
                } else if gap <= TackleEngagement::COMMIT {
                    1
                } else if gap <= TackleEngagement::DISENGAGE {
                    2
                } else {
                    3
                });
            }
            if challenging && gap <= SURROUND_RADIUS {
                contested = true;
            }
            if gap > TackleEngagement::COMMIT {
                continue;
            }
            // Ordered as the gates are, so the first large bucket is the
            // binding one.
            let bucket = if challenging {
                0
            } else if !d.can_attempt_tackle() {
                1
            } else if presser.is_some_and(|p| p != d.id) {
                2
            } else {
                3
            };
            DuelDiag::note_gate(bucket, in_box);
        }
        if in_box {
            DuelDiag::note_box_carry(bodies, contested);
        }

        // ── IS HE ACTUALLY GETTING ANY CLOSER? ────────────────────────
        //
        // See the note on `mid_run_diag::CLOSE_SAMPLES`. Everything above
        // buckets a defender by whether he is ALLOWED to challenge; this
        // asks whether the man nearest the carrier is converging on him or
        // merely travelling alongside, which is the difference the report
        // is about and which no other counter here can see.
        //
        // Only sampled while the carrier is genuinely moving — a defender
        // holding his ground against a man shielding the ball is
        // jockeying, and counting that as a failure to close would bury
        // the signal under correct defending.
        let carrier_v = carrier.velocity;
        let carrier_speed = carrier_v.magnitude();
        if carrier_speed > 0.05 {
            if let Some((gap, d)) = nearest {
                if gap > 0.5 && gap <= 200.0 {
                    let to_carrier = (carrier.position - d.position) / gap;
                    // Closing rate: how fast the gap shrinks. The carrier's
                    // own motion counts — running at a man who is running
                    // away is not closing on him.
                    let rate = (d.velocity - carrier_v).dot(&to_carrier);
                    let d_speed = d.velocity.magnitude();
                    let align = if d_speed > 0.01 {
                        d.velocity.dot(&carrier_v) / (d_speed * carrier_speed)
                    } else {
                        0.0
                    };
                    let own_goal_x = if defending_side == PlayerSide::Left {
                        0.0
                    } else {
                        context.field_size.width as f32
                    };
                    // ── GOAL-SIDE SHADOWING IS NOT THE DEFECT ─────────
                    //
                    // A defender jockeying a carrier who is running at
                    // goal retreats in front of him: same heading, gap
                    // held, closing rate ~0. That is textbook defending
                    // and the naive "same heading and not closing" test
                    // counts every second of it as a failure — which is
                    // why the first cut of this census read a flat 50%
                    // whatever was changed underneath it.
                    //
                    // What the report describes is the other one: a
                    // defender LEVEL WITH or BEHIND the man, matching his
                    // speed shoulder to shoulder, with the goal open past
                    // him. So the parallel test is restricted to a
                    // defender who is not goal-side — the ball is already
                    // nearer his goal than he is.
                    let goal_side =
                        (d.position.x - own_goal_x).abs() < (carrier.position.x - own_goal_x).abs();
                    let parallel = align > 0.5 && rate < carrier_speed * 0.10 && !goal_side;
                    let gaining = rate > carrier_speed * 0.10;
                    let deep = (carrier.position.x - own_goal_x).abs()
                        < context.field_size.width as f32 / 3.0;
                    let state = match d.state {
                        PlayerState::Defender(DefenderState::Tackling)
                        | PlayerState::Midfielder(MidfielderState::Tackling)
                        | PlayerState::Forward(ForwardState::Tackling) => 0,
                        PlayerState::Defender(DefenderState::Pressing)
                        | PlayerState::Midfielder(MidfielderState::Pressing)
                        | PlayerState::Forward(ForwardState::Pressing) => 1,
                        PlayerState::Defender(DefenderState::Marking)
                        | PlayerState::Midfielder(MidfielderState::Guarding) => 2,
                        PlayerState::Defender(DefenderState::Covering) => 3,
                        PlayerState::Defender(DefenderState::Running)
                        | PlayerState::Midfielder(MidfielderState::Running)
                        | PlayerState::Forward(ForwardState::Running) => 4,
                        PlayerState::Defender(DefenderState::HoldingLine) => 5,
                        PlayerState::Defender(DefenderState::TrackingBack) => 6,
                        _ => 7,
                    };
                    DuelDiag::note_closing(rate, align, gap, deep, parallel, gaining, state);
                }
            }
        }
    }

    /// IS HE RUNNING AT THE BALL, OR JUST ALONGSIDE IT?
    ///
    /// The sibling of the closing census in
    /// [`sample_duel_gates`](Self::sample_duel_gates), for the half of
    /// the game that one cannot see. It samples only while the ball is
    /// LOOSE — which is exactly when it bails out, because a `TakeBall`
    /// state exists only while nobody owns the ball.
    ///
    /// See `mid_run_diag::CHASE_SAMPLES` for what `lead` means and why it
    /// is the quantity that separates an interception from a stern chase.
    #[cfg(feature = "match-logs")]
    pub(in crate::r#match::engine::engine) fn sample_loose_chase(field: &MatchField) {
        use crate::mid_run_diag::ChaseDiag;

        if field.ball.current_owner.is_some() {
            return;
        }
        let ball_v = Vector3::new(field.ball.velocity.x, field.ball.velocity.y, 0.0);
        let ball_speed = ball_v.magnitude();
        // A ball that is barely moving cannot be run alongside, and the
        // lead of a stationary target is undefined. 0.05 u/tick is
        // 6 cm/s — a ball at rest in everything but the last decimal.
        if ball_speed < 0.05 {
            return;
        }
        let ball_dir = ball_v / ball_speed;
        let ball_pos = Vector3::new(field.ball.position.x, field.ball.position.y, 0.0);

        for p in field.players.iter() {
            let line = match p.state {
                PlayerState::Defender(DefenderState::TakeBall) => 0,
                PlayerState::Midfielder(MidfielderState::TakeBall) => 1,
                PlayerState::Forward(ForwardState::TakeBall) => 2,
                PlayerState::Goalkeeper(GoalkeeperState::TakeBall) => 3,
                _ => continue,
            };
            let flat = Vector3::new(p.position.x, p.position.y, 0.0);
            let to_ball = ball_pos - flat;
            let gap = to_ball.magnitude();
            // Inside a stride the geometry is degenerate — the aim point
            // and the ball are the same place whatever the model does.
            if gap < 2.0 {
                continue;
            }
            let to_ball_dir = to_ball / gap;

            let p_v = Vector3::new(p.velocity.x, p.velocity.y, 0.0);
            let p_speed = p_v.magnitude();
            // Standing still is not a chase; it is a different defect and
            // averaging it in would mute this one.
            if p_speed < 0.02 {
                continue;
            }
            let p_dir = p_v / p_speed;

            let rate = (p_v - ball_v).dot(&to_ball_dir);
            let align = p_dir.dot(&ball_dir);
            // Cross-track aim: strip the part of his heading that points
            // AT the ball, and ask how much of what is left runs with the
            // ball's travel. Zero is a man pointed at where the ball is.
            let lead = (p_dir - to_ball_dir * p_dir.dot(&to_ball_dir)).dot(&ball_dir);

            ChaseDiag::note(rate, lead, align, gap, ball_speed, line);
        }
    }
}
