//! Losing your marker — the attacking half of man-marking.
//!
//! # Why this exists
//!
//! Once the defensive rework gave every dangerous attacker a defender
//! whose whole job is to stand on his goal-side shoulder, the attackers
//! had no answer to it. They went to a position and stayed there, so a
//! marker only had to stand still to do his job perfectly. Measured after
//! that change: shots from inside 6 m fell from 44.7% of all shots to
//! 13.8% (the defending is right), but the forwards' share of shots fell
//! with it — 22% against a target of 58% — because a static target is a
//! solved problem for a defender.
//!
//! Real forwards are not static. Almost all of a striker's value off the
//! ball is in three or four specific movements, and none of them existed
//! in the engine:
//!
//! * **The blind side.** A marker has to watch the ball AND his man. Move
//!   to the side of him where he cannot see both and he has to turn his
//!   head, which is the moment you go.
//! * **The seam.** Standing between two defenders means neither of them
//!   is comfortably responsible for you — a metre of hesitation is a
//!   yard of space.
//! * **Double movement.** Show short, then spin off. The marker commits
//!   to the first move and the second one beats him. This is the single
//!   most common way a centre-forward gets free.
//! * **The separation burst.** When the ball is about to be played, you
//!   go — and whether you get away is your acceleration and your timing
//!   against his positioning and his reading of it.
//!
//! # How it is modelled
//!
//! As an OFFSET to whatever position the player's state already wanted,
//! never as a replacement for it. The team plan still owns which patch of
//! the box a forward attacks (see `teamplay::attack`); this decides the
//! angle he attacks it from and when he breaks. Bounded, so a forward
//! cannot evade his way out of his assignment.
//!
//! Every term is a continuous skill contest — the mover's off-the-ball,
//! acceleration and agility against the marker's positioning,
//! anticipation and concentration. A good mover beats a poor marker; a
//! poor mover against a good marker gets nothing, which is the point.

use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchPlayerLite, StateProcessingContext};
use nalgebra::Vector3;

/// What the attacker can see of the man marking him.
#[derive(Clone, Copy)]
pub struct MarkerRead {
    pub marker: MatchPlayerLite,
    /// Unit vector from the attacker toward his marker.
    pub to_marker: Vector3<f32>,
    /// 0..1 — how tightly he is being held. 1 is touch-tight.
    pub tightness: f32,
    /// 0..1 — how much better the attacker is at getting away than the
    /// marker is at staying with him. 0.5 is an even contest.
    pub edge: f32,
}

pub struct MarkerEvasion;

impl MarkerEvasion {
    /// Radius inside which an opponent counts as marking me (~7.5 m).
    const MARK_RADIUS: f32 = 60.0;
    /// Largest offset the evasion may apply (~8 m). A forward moves
    /// around inside his zone; he does not leave it.
    ///
    /// Sized against what it has to beat. At 32u the typical offset
    /// worked out at ~1.5 m (measured: 81% of attacker off-ball ticks
    /// have a marker, mean tightness 0.70 × mean edge 0.53 = 0.37 of the
    /// cap), and a marker steering `Arrive` onto a shoulder tracks a
    /// 1.5 m shift without ever being beaten — attacker separation did
    /// not move at all. Real movement buys 3-5 m at the moment the pass
    /// is played, which is 0.37 of ~64u.
    const MAX_OFFSET: f32 = 64.0;
    /// Period of the check-and-spin, in ticks. ~2.2 s is the real cadence
    /// of a centre-forward's double movement — long enough for the marker
    /// to commit to the first move.
    const DOUBLE_MOVE_PERIOD: f32 = 220.0;

    /// The man marking this attacker, if anybody is.
    ///
    /// Not simply the nearest body: a marker is an opponent who is
    /// GOAL-SIDE (between me and the goal I am attacking) or close enough
    /// to be touch-tight. An opponent standing in front of me on his way
    /// somewhere else is not marking me.
    pub fn read(ctx: &StateProcessingContext) -> Option<MarkerRead> {
        let me = ctx.player.position;
        let goal = ctx.player().opponent_goal_position();
        let to_goal = (goal - me).try_normalize(0.01)?;

        let mut best: Option<(MatchPlayerLite, f32)> = None;
        for opp in ctx.players().opponents().nearby(Self::MARK_RADIUS) {
            if opp.tactical_positions.is_goalkeeper() {
                continue;
            }
            let delta = opp.position - me;
            let gap = delta.magnitude();
            if gap < 0.01 {
                continue;
            }
            // Goal-side, or right on top of me.
            let goal_side = (delta / gap).dot(&to_goal);
            if goal_side < 0.15 && gap > 16.0 {
                continue;
            }
            if best.as_ref().is_none_or(|(_, d)| gap < *d) {
                best = Some((opp, gap));
            }
        }

        let (marker, gap) = best?;
        let to_marker = (marker.position - me).try_normalize(0.01)?;
        let tightness = (1.0 - gap / Self::MARK_RADIUS).clamp(0.0, 1.0);

        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let mover = {
            let s = &ctx.player.skills;
            (sc::n(sc::eff(
                ctx.player,
                sc::EffActionContext::mental(minute),
                |p| p.skills.mental.off_the_ball,
            )) * 0.45
                + (s.physical.acceleration / 20.0).clamp(0.0, 1.0) * 0.32
                + (s.physical.agility / 20.0).clamp(0.0, 1.0) * 0.23)
                .clamp(0.0, 1.0)
        };
        let holder = ctx
            .context
            .players
            .by_id(marker.id)
            .map(|d| {
                let s = &d.skills;
                ((s.mental.positioning / 20.0) * 0.40
                    + (s.mental.anticipation / 20.0) * 0.35
                    + (s.mental.concentration / 20.0) * 0.25)
                    .clamp(0.0, 1.0)
            })
            .unwrap_or(0.5);

        // Even contest sits at 0.5; the spread is deliberately narrow so
        // this shades outcomes rather than deciding them.
        let edge = (0.5 + (mover - holder) * 0.5).clamp(0.05, 0.95);

        Some(MarkerRead {
            marker,
            to_marker,
            tightness,
            edge,
        })
    }

    /// Adjust an off-ball target so the attacker attacks the same zone
    /// from a position his marker cannot cover.
    ///
    /// Returns `base_target` unchanged when nobody is marking him — an
    /// unmarked forward has no reason to move off his spot, and inventing
    /// motion for him would just make him harder to find.
    pub fn evade(ctx: &StateProcessingContext, base_target: Vector3<f32>) -> Vector3<f32> {
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::EvasionDiag::note_call();
        let Some(read) = Self::read(ctx) else {
            return base_target;
        };
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::EvasionDiag::note_marked(read.tightness, read.edge);
        let me = ctx.player.position;
        let ball = ctx.tick_context.positions.ball.position;

        // ── Blind side ───────────────────────────────────────────────
        // The marker has to hold the ball and his man in one picture.
        // Move across him, away from the ball, and he cannot.
        let to_ball = (ball - read.marker.position)
            .try_normalize(0.01)
            .unwrap_or(read.to_marker);
        // Component of "away from the ball" perpendicular to the line
        // between us — sliding around him rather than backing off.
        let across = Vector3::new(-read.to_marker.y, read.to_marker.x, 0.0);
        let side = if across.dot(&to_ball) > 0.0 {
            -1.0
        } else {
            1.0
        };
        let blind_side = across * side;

        // ── The seam ─────────────────────────────────────────────────
        // If there is a second defender near, drift toward the gap
        // between them rather than staying attached to one.
        let seam = ctx
            .players()
            .opponents()
            .nearby(Self::MARK_RADIUS * 2.0)
            .filter(|o| o.id != read.marker.id && !o.tactical_positions.is_goalkeeper())
            .min_by(|a, b| {
                let da = (a.position - me).magnitude();
                let db = (b.position - me).magnitude();
                da.total_cmp(&db)
            })
            .and_then(|second| {
                let mid = (read.marker.position + second.position) * 0.5;
                (mid - me).try_normalize(0.01)
            })
            .unwrap_or_else(Vector3::zeros);

        // ── Double movement ──────────────────────────────────────────
        // Show short toward the ball, then spin away. A slow square-ish
        // oscillation rather than a sine so the two halves read as two
        // decisions — the marker commits to the first and is beaten by
        // the second. Amplitude is the attacker's timing, so a poor mover
        // shuffles and an elite one genuinely goes.
        let phase = (ctx.in_state_time as f32 / Self::DOUBLE_MOVE_PERIOD).fract();
        let checking = phase < 0.35;
        let to_ball_from_me = (ball - me).try_normalize(0.01).unwrap_or(blind_side);
        let double_move = if checking {
            to_ball_from_me
        } else {
            -to_ball_from_me
        };

        // ── Weighting ────────────────────────────────────────────────
        // Only worth moving when somebody is actually holding you, and
        // only as far as you are able to get away from him.
        // The double movement carries the most weight, because it is the
        // only component that beats a marker who simply follows. A
        // defender steering onto a moving shoulder tracks a steady drift
        // indefinitely; what he cannot track is a REVERSAL, because his
        // own momentum carries him past it. Blind side and seam decide
        // where the space is, the check-and-spin is how you get into it.
        let effort = read.tightness * read.edge;
        let offset = (blind_side * 0.32 + seam * 0.20 + double_move * 0.48) * effort;
        let scaled = offset * Self::MAX_OFFSET;

        let field_w = ctx.context.field_size.width as f32;
        let field_h = ctx.context.field_size.height as f32;
        Vector3::new(
            (base_target.x + scaled.x).clamp(10.0, field_w - 10.0),
            (base_target.y + scaled.y).clamp(10.0, field_h - 10.0),
            base_target.z,
        )
    }

    /// Extra pace on a run when the attacker is breaking away from his
    /// marker as the ball is about to be played. 1.0 when he is not
    /// getting away or nobody is on him.
    ///
    /// This is the separation burst: the moment the pass is on, the
    /// forward goes, and whether he gets clear is his acceleration and
    /// timing against his marker's reading of it.
    pub fn burst(ctx: &StateProcessingContext) -> f32 {
        let Some(read) = Self::read(ctx) else {
            return 1.0;
        };
        // Only when the ball is live with a team-mate — bursting away
        // from a marker while the opposition has it is just drifting.
        if !ctx.team().is_control_ball() {
            return 1.0;
        }
        1.0 + read.tightness * read.edge * 0.65
    }
}
