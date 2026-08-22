//! **The open-play cross contest** — the sibling of
//! [`corner`](super::corner), and for the same reason: a cross is aimed
//! at a patch of the box rather than at a pair of feet, so it needs its
//! own resolution rather than the pass machinery's.
//!
//! Win rates here are deliberately low. Real football completes roughly a
//! quarter of open-play crosses, and only a fraction of those become
//! attempts.

use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::engine::engine::*;
use crate::r#match::player::strategies::passing::CrossType;
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::CrossDiag;
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Discrete OPEN-PLAY cross contest — the sibling of
    /// [`resolve_corner_contest`](Self::resolve_corner_contest), and for
    /// the same reason.
    ///
    /// A lofted cross is aimed at a patch of the box, not at a pair of
    /// feet, so it cannot be settled the way a pass is. Three engine
    /// facts made that impossible before this existed: `try_intercept`
    /// declines any ball above 2.5 m, the receiver claim declines above
    /// 2.8 m, and the in-flight window reserves the delivery for one
    /// named receiver for its entire flight. The result was that an
    /// aerial cross was a private transaction between the crosser and one
    /// teammate that no defender, second attacker or keeper could touch.
    ///
    /// So the engine resolves ONE skill-weighted contest the moment the
    /// delivery is over the box: the best attacking header against the
    /// best defending header, with the keeper's command of his area able
    /// to take the ball off both of them. The winner gets the ball
    /// dropped on their head and strikes it through the NORMAL shot /
    /// save pipeline, so goals, shots, xG and saves all credit through
    /// the paths they already use — no bespoke scoring route.
    ///
    /// Win rates are deliberately low. Real football completes roughly a
    /// quarter of open-play crosses, and only a fraction of those become
    /// attempts, which is why crossing is a low-percentage way to attack
    /// even though every team does it.
    pub(in crate::r#match::engine::engine) fn resolve_cross_contest(
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        let ball = &field.ball;
        if ball.cross_contest_resolved {
            return;
        }
        // Only once the delivery has left the crosser and is genuinely in
        // the air. A cross still at his feet is a set-up, not a contest.
        if ball.current_owner.is_some() {
            return;
        }
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::CROSS_CONTEST_SEEN.fetch_add(1, Ordering::Relaxed);

        // Resolve at the point the ball is actually attackable — head
        // height on the way DOWN. Above that it is still travelling; below
        // it, the ordinary reception path has it.
        //
        // Widening this band to 5.0 m was tried, on the theory that the
        // ordinary receiver claim (which starts at 2.8 m, and resolves
        // EARLIER in the tick than this does) was pre-empting the duel.
        // It moved contests from 3.9 to 4.7 a match — inside run-to-run
        // noise — and was reverted, because the diagnosis was wrong.
        //
        // What actually happens: of ~14 lofted deliveries a match, ~12.6
        // are CORNER kicks, and `resolve_corner_contest` runs first in
        // `game_tick_inner` and ends by calling
        // `clear_pending_pass_metadata`, which disarms this contest —
        // correctly, since a corner is its business. Only 2-3 open-play
        // crosses a match exist for this contest to resolve. The gap is
        // crossing VOLUME, not this window. See `CrossDiag`.
        const CONTEST_CEILING: f32 = 2.9;
        const CONTEST_FLOOR: f32 = 1.5;
        if ball.position.z > CONTEST_CEILING
            || ball.position.z < CONTEST_FLOOR
            || ball.velocity.z > 0.0
        {
            #[cfg(feature = "match-logs")]
            CrossDiag::note_reject(if ball.position.z > CONTEST_CEILING {
                0
            } else if ball.velocity.z > 0.0 {
                2
            } else {
                1
            });
            return;
        }

        let cross_type = ball.pending_cross_type;
        let crosser = ball.previous_owner;
        let Some(att_team) = crosser
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id)
        else {
            field.ball.cross_contest_resolved = true;
            return;
        };

        // The goal being attacked is the one the crossing team shoots at.
        let gl = context.goal_positions.left;
        let gr = context.goal_positions.right;
        let ball_pos = ball.position;
        let attacked_goal = if (ball_pos - gl).magnitude() < (ball_pos - gr).magnitude() {
            gl
        } else {
            gr
        };
        // Not a box delivery — let it play out as an ordinary ball.
        if (ball_pos - attacked_goal).magnitude() > 200.0 {
            #[cfg(feature = "match-logs")]
            CrossDiag::note_reject(3);
            return;
        }

        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::CROSS_CONTEST_FIRED.fetch_add(1, Ordering::Relaxed);

        let minute = (context.total_match_time / 60_000) as u32;

        // Only players who can actually get to the ball contest it. 34u is
        // ~4.3 m — a stride and a jump, which is the real radius of an
        // aerial challenge, not the whole penalty area.
        const CONTEST_RADIUS: f32 = 34.0;

        let mut best_att: Option<(usize, f32)> = None;
        let mut best_def_score = 0.0_f32;
        let mut defenders_contesting = 0u32;
        let mut gk_command = 0.0_f32;
        let mut gk_idx: Option<usize> = None;

        for (i, p) in field.players.iter().enumerate() {
            let gap = (p.position - ball_pos).magnitude();
            let is_gk = p.tactical_position.current_position.is_goalkeeper();
            // The keeper commands a wider zone than an outfielder — that
            // is the whole point of coming for a cross.
            let reach = if is_gk { 58.0 } else { CONTEST_RADIUS };
            if gap > reach {
                continue;
            }
            if p.team_id == att_team {
                if is_gk {
                    continue;
                }
                let s = sc::aerial_outfield_attacker(p, minute);
                if best_att.map_or(true, |(_, bs)| s > bs) {
                    best_att = Some((i, s));
                }
            } else if is_gk {
                let raw = (p.skills.goalkeeping.command_of_area * 0.6
                    + p.skills.goalkeeping.aerial_reach * 0.4)
                    / 20.0;
                // Distance decay — a keeper on his line does not command
                // a ball at the back post.
                gk_command = raw * (1.0 - gap / 58.0).clamp(0.0, 1.0);
                gk_idx = Some(i);
            } else {
                defenders_contesting += 1;
                let s = sc::aerial_outfield_defender(p, minute);
                if s > best_def_score {
                    best_def_score = s;
                }
            }
        }

        // Nobody attacking it — the delivery just runs through, which is
        // what a bad cross does.
        let Some((att_idx, att_score)) = best_att else {
            field.ball.cross_contest_resolved = true;
            return;
        };

        // An unmarked header is rare; an empty box is not a free goal
        // either, because the keeper is still there.
        let def_score = if defenders_contesting == 0 {
            0.30
        } else {
            // Each extra body in the challenge makes it harder to get a
            // clean contact, independent of the best defender's quality.
            best_def_score + (defenders_contesting.saturating_sub(1) as f32) * 0.06
        };

        // A whipped or driven ball is harder for a keeper to claim and
        // easier for an attacker to attack; a floated one hangs long
        // enough for the defence to set. This is the payoff for modelling
        // the delivery mix at all — the numbers live on `CrossType` so the
        // contest and the crosser's own risk estimate read one source.
        let type_edge = cross_type.map(CrossType::contest_edge).unwrap_or(0.0);
        let gk_claim_edge = cross_type.map(CrossType::keeper_claim_scale).unwrap_or(1.0);

        // Keeper first: he either takes it off everyone or he doesn't come.
        let gk_claim = (gk_command * 0.55 * gk_claim_edge).clamp(0.0, 0.45);
        if gk_idx.is_some() && context.rng.bernoulli(gk_claim) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::CROSS_CONTEST_GK.fetch_add(1, Ordering::Relaxed);
            // Leave the ball live and low in front of the keeper — his own
            // claim/catch model in the GK state machine takes it from
            // here, so the save/gather accounting stays on one path.
            //
            // ⚠ **Brought DOWN, not put down.** This used to be
            // `b.position.z = 0.6`, and a cross the keeper comes for is
            // two to three metres up — so the ball fell as much as 2.4 m
            // in a single 10 ms tick with its x/y untouched. On the
            // whole-tick relocation census that was the entire residue of
            // the `cross_contest` row: 1.3 a match, and **every one of
            // them purely vertical**, which is the axis a replay shows
            // most plainly. Height is the one axis `flight_diag` has
            // never measured — its `StageProbe` is `sqrt(dx² + dy²)` — so
            // this had no counter until now.
            //
            // A descent rate instead of a height gets the ball to the
            // same place in an eighth of a second, which is a keeper
            // taking the pace off a cross rather than the ball blinking.
            let b = &mut field.ball;
            /// Ticks the ball takes to come down to the keeper's hands.
            /// 12 (0.12 s) is fast enough that his claim model sees a low
            /// ball on the same approach it always did, and slow enough
            /// that the descent is drawn.
            const SETTLE_TICKS: f32 = 12.0;
            const CLAIM_HEIGHT: f32 = 0.6;
            let drop = ((b.position.z - CLAIM_HEIGHT) / SETTLE_TICKS).max(0.0);
            b.velocity = Vector3::new(b.velocity.x * 0.25, b.velocity.y * 0.25, -drop);
            b.pass_target_player_id = None;
            b.clear_pending_pass_metadata();
            b.cross_contest_resolved = true;
            return;
        }

        // Attacker vs defender. Base is low because most crosses are
        // headed clear — the spread comes from the aerial mismatch.
        let att_win = (0.26 + (att_score - def_score) * 0.55 + type_edge).clamp(0.05, 0.62);

        if context.rng.bernoulli(att_win) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::CROSS_CONTEST_WON.fetch_add(1, Ordering::Relaxed);
            // Drop the ball onto the winner's head, moving goalward, and
            // hold it in the heading band long enough for their state
            // machine to strike it. Same kinematics as the corner contest:
            // z 2.5 sits one tick above the intercept window, -0.02 m/tick
            // walks down through the [1.5, 2.5] band over ~40 ticks, and
            // 0.12 u/tick of drift keeps it inside header reach for all of
            // them — so ANY winner's state machine gets a valid tick, not
            // just one that happened to already be in a heading state.
            // The winner is forced into his heading state — "not all of
            // them carry the entry hook", and leaving the transition to
            // chance is why the contest could be won 307 times and produce
            // zero headers. That is still true; what changed is WHEN. The
            // transition now rides on the delivery and fires when the ball
            // reaches him, because a heading state does not survive the
            // 1.5 s the ball is now in the air. See
            // `AerialDelivery::force_heading`.
            //
            // The cross flies to him rather than being written onto his
            // head — the same change, and the same reasons, as the corner
            // contest above. This one moved the ball a mean of 1.1 m
            // against the corner's 25 m, but it fires on every lofted
            // cross rather than on corners alone, and 80% of its
            // relocations were a VERTICAL snap: the ball dropping to
            // 2.5 m from wherever the delivery had climbed to, which is
            // the most visible axis there is.
            Self::deliver_to_winner(
                field,
                att_idx,
                attacked_goal,
                crosser,
                Self::CROSS_DROP_BEHIND,
                Self::CROSS_APEX,
                true,
                true,
            );
        } else {
            // Headed clear. This is the majority outcome and it is what
            // feeds the second-ball phase — but a defensive header is a
            // full-blooded clearance, not a nudge: it goes 20-30 m and
            // lands OUTSIDE the area. A short one just drops the ball back
            // into the box for a rebound shot, which is a cheap way to
            // manufacture chances that never existed.
            //
            // Solved rather than picked, because the vertical axis is in
            // METRES and a hand-written z reads as a sane number while
            // meaning something absurd: the first draft of this used
            // `0.28`, which is a 40 m apex. Ask for the apex and let the
            // shared ballistics helper produce the launch speed, then size
            // the horizontal component to the range the arc can carry.
            // …but not always UPFIELD. A defender meeting a ball that is
            // already across him, six yards out, cannot turn it round —
            // he puts it behind, and concedes the corner he can defend
            // instead of the chance he cannot.
            //
            // This branch is the majority outcome of every cross in the
            // engine and it could only ever clear away from goal, so
            // **defenders never conceded corners**: before it, the only
            // real supplier was the keeper parrying, at 3.4 a match.
            //
            // ⚠ THE TARGET IT WAS SIZED AGAINST WAS TWICE THE REAL ONE.
            // "corners ran at ~10.8 against a real ~21, and the endline
            // split was 25% corners against ~62% real" — both of those
            // reference figures came from reading the per-MATCH corner
            // average (~10.4) as a per-TEAM one. A real match has ~10.4
            // corners and ~16 goal kicks: ~40% corners, which is what the
            // engine measures today. So this branch was aimed at roughly
            // double the corners football actually produces, and its
            // `BEHIND_AT_LINE` share should be read in that light before
            // anybody raises it further.
            if Self::heads_it_behind(ball_pos, attacked_goal, field.size.width as f32, context) {
                Self::hook_it_behind(field, ball_pos, attacked_goal);
                field.ball.cross_contest_resolved = true;
                return;
            }

            const CLEAR_RANGE_UNITS: f32 = 210.0; // ~26 m
            const CLEAR_APEX_METRES: f32 = 6.0;
            let vz = Ball::launch_speed_for_apex(CLEAR_APEX_METRES);
            let hang = Ball::hang_ticks(vz).max(1.0);
            let speed = CLEAR_RANGE_UNITS / hang;

            let clear_dir = (ball_pos - attacked_goal)
                .try_normalize(0.01)
                .unwrap_or_else(|| Vector3::new(1.0, 0.0, 0.0));
            // Headers are cleared toward the touchline, not straight back
            // down the middle where the attack came from.
            let lateral = if ball_pos.y >= attacked_goal.y {
                1.0
            } else {
                -1.0
            };
            let dir = Vector3::new(
                clear_dir.x + lateral * 0.15,
                clear_dir.y + lateral * 0.55,
                0.0,
            )
            .try_normalize(0.01)
            .unwrap_or(clear_dir);

            let b = &mut field.ball;
            // ⚠ No height write. This used to be `b.position.z = 2.2`,
            // which is a snap of up to 0.7 m on the one axis a replay
            // shows most plainly — and it is redundant: the guard at the
            // top of this function only lets the contest fire on a ball
            // already inside `[CONTEST_FLOOR, CONTEST_CEILING]` and
            // already coming down, so it is at heading height by
            // construction. He heads it from where it is.
            b.velocity = Vector3::new(dir.x * speed, dir.y * speed, vz);
            b.current_owner = None;
            b.flags.in_flight_state = 1;
        }

        // The contest IS the resolution of the delivery — drop the stale
        // aim so the nominal target can't auto-claim the dropped ball
        // through the receiver-priority radius, exactly as the corner
        // contest does.
        field.ball.pass_target_player_id = None;
        field.ball.clear_pending_pass_metadata();
        field.ball.cross_contest_resolved = true;
    }
}
