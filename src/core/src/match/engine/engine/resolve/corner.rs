//! **The corner aerial contest** — one discrete, skill-weighted duel
//! fired the instant a corner delivery is airborne.
//!
//! A lofted corner cannot be settled the way a pass is, so rather than
//! let the box swallow every one, the engine resolves a single contest
//! between the best attacking header, the defending line and the keeper's
//! command of his area, and delivers the ball to whoever won it. The
//! winner's own heading state then strikes it through the normal shot /
//! save pipeline, so goals, shots, xG and saves all credit as usual.

#[cfg(feature = "match-logs")]
use crate::r#match::engine::corner_shape::CornerShape;
use crate::r#match::engine::engine::*;
use crate::r#match::engine::set_pieces::{CORNER_DELIVERY_REFERENCE, CornerRoutine};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::SetPieceDiag;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Discrete corner aerial contest — fires once, the instant the corner
    /// cross is airborne. A played-out lofted corner can't thread the
    /// congested box to the pushed-up centre-back: the cross is always
    /// claimed/cleared mid-flight (`CB header chances` stayed 0 through
    /// every piecemeal GK / defender-duel fix). So we resolve ONE
    /// skill-weighted aerial contest — the best attacking header (a
    /// pushed-up CB or a forward) vs the defending line + GK command of
    /// area — and, if the attacker wins, drop the ball onto their head.
    /// Their existing heading state then strikes it on goal through the
    /// NORMAL shot/save pipeline, so the goal / shot / xG / save stats all
    /// credit correctly (no bespoke scoring path). The win chance is tuned
    /// (~0.30, modulated by the aerial mismatch and the keeper) so that —
    /// carried by a corner header's ~0.10-0.14 xG in the shot pipeline —
    /// only ~3-4% of corners end in a goal (real ≈ 3%), giving defenders
    /// their realistic set-piece share without inflating totals.
    pub(in crate::r#match::engine::engine) fn resolve_corner_contest(
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        use crate::r#match::PassOriginRestart;

        let ball = &field.ball;
        if ball.corner_contest_resolved || ball.pass_origin_restart != PassOriginRestart::Corner {
            return;
        }
        // A short corner and a cutback to the edge are played on the floor:
        // there is no ball into the box to attack, so the discrete aerial
        // contest must not fire and the move simply plays out as open play.
        //
        // Until the routine was wired through, EVERY corner resolved as an
        // aerial contest whatever routine had been chosen — which is why
        // `pick_corner_routine` could be called and its answer thrown away
        // without changing a single outcome.
        if matches!(
            ball.pending_corner_routine,
            Some(CornerRoutine::Short) | Some(CornerRoutine::EdgeCutback)
        ) {
            field.ball.corner_contest_resolved = true;
            return;
        }
        // [diag] reached with an armed Corner origin.
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::CORNER_CONTEST_SEEN.fetch_add(1, Ordering::Relaxed);
        // Only once the cross has actually left the taker and is airborne
        // (not the dead-ball set-up while the taker still holds it, and not
        // a short ground corner played along the floor).
        if ball.current_owner.is_some() {
            return;
        }
        // [diag] cross has left the taker (loose / in flight).
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::CORNER_CONTEST_FIRED.fetch_add(1, Ordering::Relaxed);
        if ball.position.z < 2.0 {
            return;
        }

        let minute = (context.total_match_time / 60_000) as u32;

        // The goal under attack is the one the corner is nearest to.
        let gl = context.goal_positions.left;
        let gr = context.goal_positions.right;
        let ball_pos = ball.position;
        let attacked_goal = if (ball_pos - gl).magnitude() < (ball_pos - gr).magnitude() {
            gl
        } else {
            gr
        };

        // Attacking team = the cross taker's team.
        let taker = ball.previous_owner.or(ball.current_owner);
        let att_team = match taker
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id)
        {
            Some(t) => t,
            None => {
                field.ball.corner_contest_resolved = true;
                return;
            }
        };

        // Best attacking header, best defending header, and GK command of
        // area — among the players inside the box (≈135u of the goal).
        let mut best_att: Option<(usize, f32)> = None;
        let mut best_def_score = 0.40_f32;
        // Who the defending header actually falls to, so the cleared-behind
        // branch can hook it from where he is standing rather than from the
        // corner flag the ball has not left yet.
        let mut best_def: Option<usize> = None;
        let mut gk_command = 0.35_f32;
        for (i, p) in field.players.iter().enumerate() {
            if (p.position - attacked_goal).magnitude() > 135.0 {
                continue;
            }
            let is_gk = p.tactical_position.current_position.is_goalkeeper();
            if p.team_id == att_team {
                if is_gk {
                    continue;
                }
                let s = sc::aerial_outfield_attacker(p, minute);
                if best_att.map_or(true, |(_, bs)| s > bs) {
                    best_att = Some((i, s));
                }
            } else if is_gk {
                gk_command = (p.skills.goalkeeping.command_of_area * 0.6
                    + p.skills.goalkeeping.aerial_reach * 0.4)
                    / 20.0;
            } else {
                let s = sc::aerial_outfield_defender(p, minute);
                if s > best_def_score {
                    best_def_score = s;
                    best_def = Some(i);
                }
            }
        }

        // Box census, taken here because this is the one place that runs
        // exactly once per corner at the instant the delivery is in the
        // air — so it sees both the set-up AND whether the shape survived
        // to the cross. Counted over the real penalty area rather than the
        // contest's 135u radius: "in the box" has to mean the box, or the
        // number cannot be compared with the real one (8-10 defenders).
        #[cfg(feature = "match-logs")]
        {
            let field_height = context.field_size.height as f32;
            let (mut def_in_box, mut att_in_box) = (0u32, 0u32);
            for p in field.players.iter() {
                if p.is_sent_off
                    || p.tactical_position.current_position.is_goalkeeper()
                    || !CornerShape::is_in_penalty_area(p.position, attacked_goal.x, field_height)
                {
                    continue;
                }
                if p.team_id == att_team {
                    att_in_box += 1;
                } else {
                    def_in_box += 1;
                }
            }
            SetPieceDiag::note_corner_box(def_in_box, att_in_box);
        }

        let (att_idx, att_score) = match best_att {
            Some(v) => v,
            None => {
                field.ball.corner_contest_resolved = true;
                return;
            }
        };

        // Base eased 0.36 → 0.31 in the 2026-08 state-repair
        // recalibration. 0.36 was set while the loose-ball override could
        // still yank the winning header off the dropped ball mid-attempt;
        // headers are committed actions now and complete every time, so
        // the same win rate converts to ~35% more corner goals (DEF
        // corner headers on goal 536 → 708 per 200 matches, DEF goal
        // share 14.5% → 18.6% against the real ~10%).
        //
        // Delivery scale: the ball that arrives is the other half of the
        // contest, and it was missing entirely — the two duellists and the
        // keeper decided everything, so a dead-ball specialist's whipped
        // corner and a centre-half's hopeful clip produced identical
        // chances.
        //
        // ⚠ MULTIPLICATIVE, and it has to be. For an evenly-matched box
        // the expression below lands NEGATIVE before the clamp (0.100
        // − gk_command·0.18 with gk_command ≈ 0.6 is −0.008), so the 0.04
        // floor is what most corners actually return. An *additive*
        // delivery term centred on the population mean therefore does not
        // cancel out: the below-average half is swallowed by the floor
        // while the above-average half escapes it, and the contest
        // ratchets upward — measured at +30% attacker wins with a
        // correctly-centred additive term. Scaling instead keeps the sign,
        // so a poor delivery makes an already-floored corner more negative
        // (still floored) and only corners with a real aerial edge move at
        // all, in both directions.
        let delivery_scale =
            (field.ball.pending_corner_delivery / CORNER_DELIVERY_REFERENCE).clamp(0.55, 1.45);
        // Routine: where the ball is put changes how cleanly it can be
        // met. The penalty spot is the classic — most time to attack it
        // and the keeper furthest from it. Near post is a flick, harder to
        // time; far post gives the keeper the whole flight to read it.
        let routine_scale = match field.ball.pending_corner_routine {
            Some(CornerRoutine::NearPost) => 0.95,
            Some(CornerRoutine::FarPost) => 0.92,
            _ => 1.00,
        };
        let att_win = ((0.100 + (att_score - best_def_score) * 0.50 - gk_command * 0.18)
            * delivery_scale
            * routine_scale)
            .clamp(0.04, 0.36);

        if context.rng.bernoulli(att_win) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::CORNER_CONTEST_WON.fetch_add(1, Ordering::Relaxed);
            // Attacker wins: drop the ball just behind them at head height,
            // moving goalward, so it reads as an incoming header to their
            // state (the CB's AttackingCorner, or a forward's run→heading).
            // Loose so they head it; keep the Corner origin so the CB stays
            // in AttackingCorner through the strike.
            //
            // Drop kinematics = apex-of-flick hang time. The previous
            // (z 2.2, vz −1.0, 4.0 u/tick drift) fell through the entire
            // heading band [1.4, 2.5] in ONE tick and drifted out of
            // 6u header reach almost as fast — so only a CB already in
            // AttackingCorner (whose same-tick path runs right after
            // this resolver) ever struck it; a FORWARD winner spent the
            // only valid tick transitioning Running→Heading and found
            // the ball below threshold, and the loose ball was then
            // vacuumed by the intercept gate (z ≤ 2.5). Real contested
            // headers hang ~0.3-0.4 s at the apex: z 2.55 (one tick
            // above the intercept window) with vz −0.35 and a modest
            // 1.8 u/tick goalward drift keeps the ball in the heading
            // band and within reach for ~3 ticks — enough for ANY
            // winner's state machine to strike, which is what the
            // contest already decided should happen.
            //
            // Restated for the metres-per-tick vertical axis: −0.02 m/tick
            // is 2 m/s of descent, which walks the ball down through the
            // [1.4, 2.5] heading band over ~40 ticks, and 0.12 u/tick of
            // goalward drift keeps it inside the 6u header reach for all of
            // them. The old (−0.35, 1.8) pair was written when the vertical
            // axis carried unit-scale speeds and would now fall through the
            // band in three ticks while drifting 60u out of reach.
            //
            // ⚠ **The ball is no longer WRITTEN onto his head.** All of the
            // above still happens; it happens when the delivery gets
            // there. The cross now actually flies the twenty-five metres
            // from the flag — see [`Self::deliver_to_winner`] and
            // [`AerialDelivery`] — which is the corner half of the "ball
            // teleports" report. `CORNER_APEX` is a normal in-swinger:
            // 5 m up, about 1.7 s in the air, comfortably inside
            // `CornerDeadline` so the shape still holds for the
            // whole flight.
            Self::deliver_to_winner(
                field,
                att_idx,
                attacked_goal,
                taker,
                Self::CORNER_DROP_BEHIND,
                Self::CORNER_APEX,
                true,
                false,
            );
        } else if let Some(clearer) = best_def {
            // **The repeat corner.** The defending side wins the header,
            // and the man it falls to — standing in his own six-yard area
            // with the ball already across him — hooks it over his own
            // byline instead of trying to turn it upfield.
            //
            // This is the sibling of the same branch in
            // `resolve_cross_contest`, on the same curve and the same
            // window, and the corner contest was the one that never had
            // it: a delivery the attackers did not win simply flew on to
            // its aim point untouched, so **a corner in this engine could
            // never produce another corner**. Real football does that
            // constantly — it is why sides win three and four in a row —
            // and the corner-source census had the whole "defender puts a
            // delivery behind" family at 4% of supply against a real ~35%.
            //
            // ⚠ This branch was the bigger half of the corner teleport,
            // not the attacking one — `att_win` is clamped at 0.36, so
            // most corners come here. It used to hook the ball behind
            // FROM THE CLEARER'S FEET while the ball was still at the
            // flag, which wrote it the full width of the box in one tick.
            // Now the delivery flies to him and is hooked when it
            // arrives, through the same [`AerialDelivery`] machinery the
            // attacking branch uses.
            let from = field.players[clearer].position;
            if Self::heads_it_behind(from, attacked_goal, field.size.width as f32, context) {
                Self::deliver_to_winner(
                    field,
                    clearer,
                    attacked_goal,
                    taker,
                    Self::CORNER_DROP_BEHIND,
                    Self::CORNER_APEX,
                    false,
                    false,
                );
            }
        }
        // Otherwise the cross plays out — the keeper claims or a defender
        // clears it upfield (the realistic majority outcome).

        // The contest IS the resolution of the delivery — clear the
        // stale cross-target so the original aim point (often the OTHER
        // pushed-up CB) can't auto-claim the dropped ball through the
        // 100u receiver-priority radius. Before this, won headers were
        // routinely converted into a different player's chest-trap →
        // slow foot-shot, and "lost" contests were caught by the
        // attacking CB instead of playing out as GK claims/clearances.
        field.ball.pass_target_player_id = None;
        field.ball.clear_pending_pass_metadata();

        // Persist this corner's routine + estimated xG into the team's
        // history so `pick_corner_routine` can vary future deliveries.
        // The xG used here is a rough estimate (att_win × generic
        // header xG); the precise xG is computed downstream when the
        // header actually fires through the shot pipeline. The history
        // only needs the *flavour* of "did this routine produce a
        // chance" to gate repeats, so an approximate value is fine.
        if let Some(routine) = field.ball.pending_corner_routine.take() {
            let estimated_xg = att_win * 0.12; // ~0.12 header xG ceiling × win prob
            let is_home_attacking = att_team == context.field_home_team_id;
            context
                .set_piece_history
                .record_corner(is_home_attacking, routine, estimated_xg);
        }
        // Back to "an ordinary delivery" so a stale specialist stamp can't
        // leak into the next corner (or into an open-play cross contest
        // that reads the same field).
        field.ball.pending_corner_delivery = CORNER_DELIVERY_REFERENCE;

        field.ball.corner_contest_resolved = true;
    }
}
