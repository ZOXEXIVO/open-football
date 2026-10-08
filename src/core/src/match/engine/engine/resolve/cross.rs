//! **The open-play cross contest** — the sibling of
//! [`corner`](super::corner), and for the same reason: a cross is aimed
//! at a patch of the box rather than at a pair of feet, so it needs its
//! own resolution rather than the pass machinery's.
//!
//! Win rates here are deliberately low. Real football completes roughly a
//! quarter of open-play crosses, and only a fraction of those become
//! attempts.

use crate::r#match::PassOriginRestart;
use crate::r#match::engine::ball::ball::AerialReach;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::diagnostics::block_diag::BlockDiag;
use crate::r#match::engine::ball::ball::{Ball, DeliveryIntent, PlayerReach};
use crate::r#match::engine::ball::events::BallEvent;
use crate::r#match::engine::engine::*;
use crate::r#match::engine::events::EventCollection;
use crate::r#match::engine::teamplay::standard::MatchStandard;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::passing::CrossType;
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::CrossDiag;
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

/// **Where the man clearing a cross is**, relative to the ball the contest
/// has just awarded him.
///
/// The defensive outcomes of
/// [`resolve_cross_contest`](FootballEngine::resolve_cross_contest) turn
/// the ball through 180° in a single tick, and until 2026-09-07 they did
/// it at the BALL with no man named at all: the contest loop kept only the
/// best defender's SCORE, and `def_score` falls back to 0.30 when nobody
/// is contesting — so a cross into an empty six-yard box was still
/// cleared, by nobody, off nothing. That is the reported *"the ball
/// bounces off an invisible object"* in its purest form, and
/// `resolve_corner_contest` has kept the clearer's index to avoid exactly
/// it since its own hooked-behind branch existed.
#[derive(Clone, Copy)]
enum CrossClearer {
    /// Close enough to head the ball where it is.
    OnIt,
    /// A stride away: the ball flies to him and the outcome is struck when
    /// it arrives, through the same [`AerialDelivery`] the attacking
    /// branch uses.
    Coming(usize),
    /// Nobody is there. The cross beat the whole defence, and a ball
    /// nobody can reach carries on — it is not cleared by thin air.
    Nobody,
}

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Who actually heads a cross the attackers did not win, and whether
    /// the ball is already on him. See [`CrossClearer`].
    ///
    /// The reach test is `PlayerReach::can_strike(.., aerial = true)` and
    /// nothing of this module's own: `KICKABLE_DISTANCE` across the grass
    /// and the man's own jumping ceiling up it, which is the engine's
    /// single answer to "may he play this ball". A second opinion here is
    /// how the two would drift.
    ///
    /// `OF_CROSS_CLEAR_FLAT` restores the pre-fix arm by answering
    /// [`CrossClearer::OnIt`] to everything — debug infrastructure, do not
    /// remove.
    fn cross_clearer(field: &MatchField, best_def: Option<usize>) -> CrossClearer {
        if MatchContext::cross_clear_flat() {
            return CrossClearer::OnIt;
        }
        match best_def {
            Some(d) if PlayerReach::can_strike(&field.ball, &field.players[d], true) => {
                CrossClearer::OnIt
            }
            Some(d) => CrossClearer::Coming(d),
            None => CrossClearer::Nobody,
        }
    }
    /// A keeper who has come for a delivery claims it on his handling: the
    /// floor for the poorest command of area at this standard, floor plus
    /// span for the best.
    const KEEPER_CLAIM_FLOOR: f32 = 0.55;
    const KEEPER_CLAIM_SPAN: f32 = 0.40;
    /// A keeper who stayed home: the share of his command a ball dropping
    /// beside him is worth.
    const KEEPER_HOME_CLAIM: f32 = 0.55;

    /// Base attacker win rate of the open-play aerial contest — see the
    /// note at the `att_win` computation for its history.
    /// `OF_CROSS_WIN` overrides for titration.
    ///
    /// 0.16 → 0.28 in the same 2026-08-31 campaign, measured: with the
    /// full contest chain finally leak-free (ordering, grants, keeper
    /// double-jeopardy, the always-a-contact strike), 0.16 produced 2.4
    /// wins and 0.87 headed shots a match — the whole header channel
    /// carried ~0.09 goals/match against a real ~0.5-0.7, too thin for
    /// ANY heading-skill swing to reach the scoreline through. 0.28
    /// prices attacking first contact at ~11-13% of contests (real
    /// clean attacking contact on crosses runs higher still), which is
    /// what lifts headed shots toward their real 2.5-4 band.
    fn cross_win_base() -> f32 {
        use std::sync::OnceLock;
        static V: OnceLock<f32> = OnceLock::new();
        *V.get_or_init(|| {
            std::env::var("OF_CROSS_WIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.28)
        })
    }

    /// Base travel of a contested headed clear, in game units (16 m).
    /// The margin-scaled span on top is at the call site — see the note
    /// there for the measured heading-tax history. `OF_CLEAR_RANGE`
    /// overrides for titration.
    fn clear_range_base() -> f32 {
        use std::sync::OnceLock;
        static V: OnceLock<f32> = OnceLock::new();
        *V.get_or_init(|| {
            std::env::var("OF_CLEAR_RANGE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(130.0)
        })
    }

    /// Discrete aerial contest for a lofted delivery, an open-play cross
    /// or a corner alike.
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
        events: &mut EventCollection,
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

        let corner = field.ball.pass_origin_restart == PassOriginRestart::Corner;
        let minute = (context.total_match_time / 60_000) as u32;

        // Only players who can actually get their head on it contest it,
        // and they are measured from where it comes down to head height,
        // not from where it is now. 24u is 3 m — a stride and a jump.
        const CONTEST_RADIUS: f32 = 24.0;
        let (drop, ticks_to_drop) = field
            .ball
            .natural_drop(AerialReach::ATTACKED)
            .unwrap_or((ball_pos, 0));

        let mut best_att: Option<(usize, f32)> = None;
        let mut best_def_score = 0.0_f32;
        // WHO the defending header falls to, not just how good it was.
        // The defensive outcomes below turn the ball in mid-air, and a
        // turn with no man attached to it is the reported "it bounces off
        // an invisible object".
        let mut best_def: Option<usize> = None;
        let mut defenders_contesting = 0u32;
        let mut gk_command = 0.0_f32;
        let mut gk_idx: Option<usize> = None;
        let mut gk_coming = false;

        for (i, p) in field.players.iter().enumerate() {
            // …and by where his run takes him by then, not where he stands:
            // a man already running onto it is priced at his arrival.
            let to_drop = (drop - p.position).xy();
            let closing = to_drop
                .try_normalize(1.0e-4)
                .map_or(0.0, |dir| p.velocity.xy().dot(&dir).max(0.0));
            let gap = (to_drop.norm() - closing * ticks_to_drop as f32).max(0.0);
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
                if best_att.is_none_or(|(_, bs)| s > bs) {
                    best_att = Some((i, s));
                }
            } else if is_gk {
                let raw = MatchStandard::peer(
                    (p.skills.goalkeeping.command_of_area * 0.6
                        + p.skills.goalkeeping.aerial_reach * 0.4)
                        / 20.0,
                    MatchStandard::keeper_shift(context),
                );
                // A keeper who has come for it read the flight and won the
                // race to it, or he would not have come
                // (`KeeperAerialClaim::assess`): it is his on his handling.
                // Priced as a share of a stay-at-home claim instead, he
                // lost it six times in seven and was left off his line for
                // the header. One who stayed
                // home takes what drops near him — a keeper on his line
                // does not command a ball at the back post.
                let coming = matches!(
                    p.state,
                    PlayerState::Goalkeeper(
                        GoalkeeperState::Catching
                            | GoalkeeperState::Jumping
                            | GoalkeeperState::ComingOut
                            | GoalkeeperState::Punching
                    )
                );
                gk_coming = coming;
                gk_command = if coming {
                    Self::KEEPER_CLAIM_FLOOR + raw * Self::KEEPER_CLAIM_SPAN
                } else {
                    raw * (1.0 - gap / 58.0).clamp(0.0, 1.0) * Self::KEEPER_HOME_CLAIM
                };
                gk_idx = Some(i);
            } else {
                defenders_contesting += 1;
                let s = sc::aerial_outfield_defender(p, minute);
                if s > best_def_score {
                    best_def_score = s;
                    best_def = Some(i);
                }
            }
        }

        // Nobody can get to it yet. A whipped ball drops through head height
        // well short of where it is going, and settling the contest there
        // settled it in empty air: the box it was aimed at never got to
        // attack it. It waits, armed, until somebody can reach it or it has
        // dropped out of the band.
        if best_att.is_none() && best_def.is_none() && gk_idx.is_none() {
            return;
        }
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::CROSS_CONTEST_FIRED.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "match-logs")]
        if corner {
            crate::mid_run_diag::CORNER_CONTEST_FIRED.fetch_add(1, Ordering::Relaxed);
            Self::note_corner_box(field, att_team, attacked_goal.x);
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
        // The delivery's shape is what keeps a keeper at home, so it prices
        // only a keeper who stayed there: one who came for it already read
        // its flight in deciding to come.
        let shape = if gk_coming { 1.0 } else { gk_claim_edge };
        let gk_claim = (gk_command * shape * (1.0 + context.conditions.goalkeeper_claim_cross))
            .clamp(0.0, 0.95);
        if gk_idx.is_some() && context.rng.bernoulli(gk_claim) {
            if corner {
                Self::note_corner_routine(field, context, att_team, 0.0);
            }
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::CROSS_CONTEST_GK.fetch_add(1, Ordering::Relaxed);
            // ⚠ **…and only if he can actually get a glove to it.**
            //
            // He is elected from a 58 u (7.25 m) command radius — far
            // wider than an outfielder's — and this branch takes three
            // quarters of the pace off the ball WHERE THE BALL IS.
            // Measured over 200 matches before the gate, 0.44 a match at a
            // mean 1.60 m with the ball 2.77 m up, three quarters of them
            // beyond anything the replay rig can draw a contact on: the
            // ball stops dead in mid-air with the keeper still coming.
            //
            // If he is not on it yet the delivery simply carries on and
            // his own claim model comes out for it — which is what this
            // branch's own comment says it is handing over to.
            // `OF_CROSS_CLEAR_FLAT` restores the old arm.
            let on_it = MatchContext::cross_clear_flat()
                || gk_idx.is_some_and(|k| PlayerReach::can_possess(&field.ball, &field.players[k]));
            // Booked either way, with `deferred` carrying which: the gap is
            // the geometry at the CONTEST, and that is the before/after
            // quantity. See `BlockDiag::CHANNELS`.
            #[cfg(feature = "match-logs")]
            if let Some(k) = gk_idx {
                BlockDiag::note_contact(
                    4,
                    field.ball.position.z,
                    (field.players[k].position - ball_pos).magnitude(),
                    AerialReach::HIGHEST,
                    !on_it,
                );
            }
            if !on_it {
                field.ball.cross_contest_resolved = true;
                return;
            }
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
        //
        // Re-based 0.32 → 0.16 when the contest ordering fix landed
        // (2026-08-31). 0.32 was itself a re-derivation ("0.26 → 0.32,
        // once open-play crossing existed at all") — but it was fitted
        // while the receiver-priority claim was eating 22.9 in-band
        // deliveries a match before this contest could fire, so it was
        // priced against a fraction of the real volume. With the
        // contests running before `play_ball`, every lofted box
        // delivery is genuinely contested (~35-40 a match), and at 0.32
        // that produced an absurd ~12 headed attempts a match against a
        // real ~2.5-4 TOTAL headed shots. 0.16 prices the binary
        // contest honestly: a "win" here is a clean attacking header,
        // not mere first contact (flick-ons and knock-downs live inside
        // the cleared majority), and most crosses ARE headed clear. The
        // `(att_score - def_score) * 0.55` term still decides WHO wins
        // the ones that are won. `OF_CROSS_WIN` overrides the base for
        // titration.
        let att_win =
            (Self::cross_win_base() + (att_score - def_score) * 0.55 + type_edge).clamp(0.04, 0.55);
        if corner {
            Self::note_corner_routine(field, context, att_team, att_win);
        }

        if context.rng.bernoulli(att_win) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::CROSS_CONTEST_WON.fetch_add(1, Ordering::Relaxed);
            #[cfg(feature = "match-logs")]
            if corner {
                crate::mid_run_diag::CORNER_CONTEST_WON.fetch_add(1, Ordering::Relaxed);
            }
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
                DeliveryIntent::Header,
                true,
                2,
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
            // ⚠ The real target is ~10.4 corners a MATCH and ~16 goal
            // kicks, ~40% corners at the byline — not the per-team ~21 an
            // earlier sizing read it as. The engine measured that split
            // only while the touch bookkeeping was wrong: a header out of
            // the air, a claim or a shot booked no touch, so balls a
            // defender put out were given as goal kicks and shots an
            // attacker put wide as corners. With every touch booked it
            // reads ~29%, which is what `behind_at_line` was raised
            // against.
            // How close to his own line the header is decides how much
            // choice he has about where it goes, and the header happens
            // at him: in most of these the ball is still a stride short.
            let header_at = best_def.map_or(ball_pos, |d| field.players[d].position);
            if Self::heads_it_behind(header_at, attacked_goal, field.size.width as f32, context) {
                // ⚠ Hooked behind BY somebody, not off the ball — the
                // clear branch below is the same rule and carries the
                // measurement. See `BlockDiag::CHANNELS`.
                let clearer = Self::cross_clearer(field, best_def);
                #[cfg(feature = "match-logs")]
                BlockDiag::note_contact(
                    3,
                    ball_pos.z,
                    best_def
                        .map(|d| (field.players[d].position - ball_pos).magnitude())
                        .unwrap_or(f32::MAX),
                    AerialReach::HIGHEST,
                    !matches!(clearer, CrossClearer::OnIt),
                );
                match clearer {
                    CrossClearer::OnIt => {
                        if let Some(d) = best_def {
                            events.add_ball_event(BallEvent::HeadedClear(
                                field.players[d].id,
                                ball_pos,
                            ));
                        }
                        Self::hook_it_behind(field, ball_pos, attacked_goal)
                    }
                    CrossClearer::Coming(d) => Self::deliver_to_winner(
                        field,
                        d,
                        attacked_goal,
                        crosser,
                        Self::CROSS_DROP_BEHIND,
                        DeliveryIntent::HookedBehind,
                        false,
                        4,
                    ),
                    // Nobody is there to hook it anywhere. The delivery
                    // carries on, which is what a cross that beat the
                    // whole defence does.
                    CrossClearer::Nobody => {}
                }
                // Out through the shared cleanup below rather than
                // straight back: `hook_it_behind` drops the stale aim
                // itself, but the other two arms do not, and a delivery
                // left armed at its nominal receiver is auto-claimed
                // through the receiver-priority radius.
                field.ball.pass_target_player_id = None;
                field.ball.clear_pending_pass_metadata();
                field.ball.cross_contest_resolved = true;
                return;
            }

            // ── THE CLEAR IS A CONTESTED HEADER, NOT A FREE KICK ─────
            // 210u (26 m) flat was a clean defensive exit on every one
            // of the ~29 headed clears a match, which made the aerial
            // route a possession furnace: the side whose decisions
            // favour crossing (high `heading` targets) burned attacks
            // into guaranteed clean exits, and the 6:18 heading pin
            // measured a WRONG-WAY Δ ≈ +0.28 over n=500 — a style tax
            // on the aerially good. A real defensive header under
            // pressure travels 10-18 m and drops at the edge of the
            // box, where the second ball is genuinely contested — the
            // second-ball phase this branch's own docs name as what it
            // feeds. Range now scales with how comfortably the duel was
            // won: a scrambled, pressured clear squirts to the box edge
            // (~16 m from the resolve point), a dominant free header
            // still buys the old clean exit. `OF_CLEAR_RANGE` overrides
            // the base for titration.
            const CLEAR_APEX_METRES: f32 = 6.0;
            let margin = (def_score - att_score).clamp(0.0, 0.55);
            let clear_range = Self::clear_range_base() + margin * 160.0;

            // ⚠ **The clearance happens at a man, or it does not happen.**
            //
            // This is the majority outcome of every open-play cross and it
            // turns the ball through 180° in one tick. It used to do that
            // at the BALL: the contest loop kept the best defender's SCORE
            // and threw away WHICH defender it was, and `def_score` falls
            // back to 0.30 with nobody contesting at all — so a cross into
            // an empty six-yard box was cleared, by nobody, off nothing.
            //
            // Measured over 200 matches before this: **10.8 a match, the
            // ball 2.81 m up and the nearest defender 2.96 m away, 98.7%
            // of them beyond anything the replay rig can draw a contact
            // on.** That is the reported *"the ball bounces not off the
            // player, but off an invisible object"*, and by volume it was
            // the largest instance of it left in the engine.
            //
            // `OF_CROSS_CLEAR_FLAT` restores the old arm. See
            // [`CrossClearer`] and `BlockDiag::CHANNELS` — `gap` there is
            // the geometry at the CONTEST, which is the before/after
            // quantity, and `deferred` is the share that no longer turn
            // the ball here at all.
            let clearer = Self::cross_clearer(field, best_def);
            #[cfg(feature = "match-logs")]
            BlockDiag::note_contact(
                2,
                ball_pos.z,
                best_def
                    .map(|d| (field.players[d].position - ball_pos).magnitude())
                    .unwrap_or(f32::MAX),
                AerialReach::HIGHEST,
                !matches!(clearer, CrossClearer::OnIt),
            );
            match clearer {
                CrossClearer::OnIt => {
                    // He heads it away, and is credited with having done
                    // it — see [`BallEvent::HeadedClear`].
                    if let Some(d) = best_def {
                        events
                            .add_ball_event(BallEvent::HeadedClear(field.players[d].id, ball_pos));
                    }
                    let b = &mut field.ball;
                    // ⚠ No height write. This used to be `b.position.z =
                    // 2.2`, which is a snap of up to 0.7 m on the one axis
                    // a replay shows most plainly — and it is redundant:
                    // the guard at the top of this function only lets the
                    // contest fire on a ball already inside
                    // `[CONTEST_FLOOR, CONTEST_CEILING]` and already
                    // coming down, so it is at heading height by
                    // construction. He heads it from where it is.
                    b.velocity = Ball::headed_clear_velocity(
                        ball_pos,
                        attacked_goal,
                        clear_range,
                        CLEAR_APEX_METRES,
                    );
                    b.current_owner = None;
                    b.flags.in_flight_state = 1;
                }
                // He is a stride away. The delivery flies to him and the
                // clearance is struck when it gets there, through the same
                // machinery the attacking branch has used since the corner
                // teleport was removed.
                CrossClearer::Coming(d) => Self::deliver_to_winner(
                    field,
                    d,
                    attacked_goal,
                    crosser,
                    Self::CROSS_DROP_BEHIND,
                    DeliveryIntent::Cleared {
                        range: clear_range,
                        apex: CLEAR_APEX_METRES,
                    },
                    false,
                    3,
                ),
                // Nobody to head it. The cross beat the whole defence and
                // carries on — a ball nobody can reach is not cleared by
                // thin air.
                CrossClearer::Nobody => {}
            }
        }

        // The contest IS the resolution of the delivery — drop the stale
        // aim so the nominal target can't auto-claim the dropped ball
        // through the receiver-priority radius, exactly as the corner
        // contest does.
        field.ball.pass_target_player_id = None;
        field.ball.clear_pending_pass_metadata();
        field.ball.cross_contest_resolved = true;
    }

    /// A corner's routine goes into the history that stops a side
    /// repeating one that keeps failing, with the chance it made: a
    /// header's xG ceiling times the chance of winning it.
    fn note_corner_routine(
        field: &mut MatchField,
        context: &mut MatchContext,
        att_team: u32,
        att_win: f32,
    ) {
        if let Some(routine) = field.ball.pending_corner_routine.take() {
            let home = att_team == context.field_home_team_id;
            context
                .set_piece_history
                .record_corner(home, routine, att_win * 0.12);
        }
    }

    /// Who is in the area as a corner is contested — the box census at the
    /// delivery.
    #[cfg(feature = "match-logs")]
    fn note_corner_box(field: &MatchField, att_team: u32, goal_x: f32) {
        use crate::r#match::engine::corner_shape::CornerShape;
        let field_height = field.size.height as f32;
        let (mut defenders, mut attackers) = (0u32, 0u32);
        for p in field.players.iter() {
            if p.off_pitch
                || p.tactical_position.current_position.is_goalkeeper()
                || !CornerShape::is_in_penalty_area(p.position, goal_x, field_height)
            {
                continue;
            }
            if p.team_id == att_team {
                attackers += 1;
            } else {
                defenders += 1;
            }
        }
        crate::mid_run_diag::SetPieceDiag::note_corner_box(defenders, attackers);
    }
}
